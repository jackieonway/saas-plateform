use crate::util::uuid7::ParseUuid7;
use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::{ApiJson, CurrentUser, RequirePermission};
use crate::routes::{page_json, paging};
use crate::state::AppState;
use crate::util::audit;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/roles", get(list).post(create))
        .route("/api/roles/{id}", put(update).delete(delete))
        .route("/api/roles/{id}/menus", get(get_menus).put(put_menus))
        .route("/api/roles/{id}/users", get(get_users).put(put_users))
}

#[derive(sqlx::FromRow, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RoleListRow {
    id: Uuid,
    name: String,
    code: String,
    description: String,
    organization_id: Uuid,
    organization_name: String,
    tenant_id: Uuid,
    status: String,
    built_in: bool,
    menu_count: i64,
    user_count: i64,
    created_at: DateTime<Utc>,
}

fn list_select() -> String {
    let employee_support =
        super::users::role_employee_support_sql("ur.user_id", "ur.organization_id", "$4");
    format!(
    "SELECT r.id, r.name, COALESCE(r.code,'') AS code, COALESCE(r.description,'') AS description, \
 r.organization_id, o.name AS organization_name, r.tenant_id, r.status, r.built_in, r.created_at, \
 (SELECT count(*) FROM role_menus rm WHERE rm.role_id = r.id) AS menu_count, \
 (SELECT count(*) FROM user_roles ur WHERE ur.role_id = r.id AND ur.organization_id=r.organization_id \
 AND {employee_support} AND ($4::uuid[] IS NULL OR ur.organization_id = ANY($4))) AS user_count")
}

/// GET /api/roles — 分页 + keyword/tenantId/organizationId; 非平台根组织仅看当前组织及下级组织角色。
async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:role")
        .check(&state, &user)
        .await?;
    let (page, size, offset) = paging(&params);
    let kw = params.get("keyword").cloned().unwrap_or_default();
    let scoped_orgs = state.scoped_org_ids(&user).await?;
    let requested_tenant = crate::util::uuid7::optional_query(params.get("tenantId"))?;
    let tenant_filter: Option<Uuid> = if scoped_orgs.is_none() {
        requested_tenant
    } else {
        Some(user.tenant_id)
    };
    let org_filter = crate::util::uuid7::optional_query(params.get("organizationId"))?;
    if let Some(org) = org_filter {
        state.ensure_org_scope(&user, org, "组织").await?;
    }
    let where_clause =
        "($1 = '' OR r.name ILIKE '%' || $1 || '%' OR r.code ILIKE '%' || $1 || '%') \
     AND ($2::uuid IS NULL OR r.tenant_id = $2) AND ($3::uuid IS NULL OR r.organization_id = $3) \
     AND ($4::uuid[] IS NULL OR r.organization_id = ANY($4))";
    let list_select = list_select();
    let rows: Vec<RoleListRow> = sqlx::query_as(&format!(
        "{list_select} FROM roles r JOIN organizations o ON o.id = r.organization_id \
         WHERE {where_clause} ORDER BY r.created_at, r.id LIMIT $5 OFFSET $6"
    ))
    .bind(&kw)
    .bind(tenant_filter)
    .bind(org_filter)
    .bind(&scoped_orgs)
    .bind(size)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;
    let total: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM roles r WHERE {where_clause}"
    ))
    .bind(&kw)
    .bind(tenant_filter)
    .bind(org_filter)
    .bind(&scoped_orgs)
    .fetch_one(&state.pool)
    .await?;
    let records: Vec<Value> = rows
        .iter()
        .map(|r| serde_json::to_value(r).unwrap_or_default())
        .collect();
    Ok(Json(page_json(records, total, page, size)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoleBody {
    name: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    description: String,
    organization_id: String,
}

async fn load_role_org(pool: &sqlx::PgPool, role: Uuid) -> Result<(Uuid, Uuid, String), AppError> {
    let row: Option<(Uuid, Uuid, String)> =
        sqlx::query_as("SELECT organization_id, tenant_id, name FROM roles WHERE id = $1")
            .bind(role)
            .fetch_optional(pool)
            .await?;
    row.ok_or_else(|| AppError::not_found("角色不存在"))
}

#[derive(sqlx::FromRow)]
struct RoleSnapshot {
    id: Uuid,
    name: String,
    code: String,
    status: String,
    built_in: bool,
    description: String,
}

impl RoleSnapshot {
    fn summary(&self) -> Value {
        json!({"id": self.id, "name": self.name, "code": self.code,
               "status": self.status, "builtIn": self.built_in})
    }

    fn managed(&self) -> bool {
        self.built_in && matches!(self.code.as_str(), "tenant_admin" | "org_admin")
    }
}

async fn lock_role(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
) -> Result<RoleSnapshot, AppError> {
    // Serialize replacements on the same role without conflicting with the
    // KEY SHARE locks acquired by user_roles foreign-key inserts.
    sqlx::query_as(
        "SELECT id, name, COALESCE(code,'') AS code, status, built_in, \
         COALESCE(description,'') AS description \
         FROM roles WHERE id=$1 FOR NO KEY UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| AppError::not_found("角色不存在"))
}

fn role_summary(row: &RoleListRow) -> Value {
    json!({"id": row.id, "name": row.name, "code": row.code,
           "status": row.status, "builtIn": row.built_in})
}

fn ids_digest(ids: impl IntoIterator<Item = Uuid>) -> String {
    let mut digest = Sha256::new();
    for id in ids {
        digest.update(id.as_bytes());
    }
    hex::encode(digest.finalize())
}

/// 非平台上下文的目标角色隔离校验 (角色自带 tenant_id 列)
async fn ensure_role_in_scope(
    state: &AppState,
    user: &CurrentUser,
    role: (Uuid, Uuid, String),
) -> Result<(), AppError> {
    state.ensure_org_scope(user, role.0, "角色").await
}

async fn lock_actor(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: &CurrentUser,
) -> Result<(), AppError> {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE id=$1 FOR NO KEY UPDATE")
        .bind(actor.user_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| AppError::unauthorized("当前会话用户已失效"))?;
    super::users::require_employee_permission_in_tx(tx, actor, "system:role").await
}

async fn revalidate_actor(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: &CurrentUser,
    scope: Option<&[Uuid]>,
) -> Result<(), AppError> {
    super::users::require_scope_unchanged_in_tx(tx, actor, scope).await?;
    super::users::require_employee_permission_in_tx(tx, actor, "system:role").await
}

/// POST /api/roles
async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<RoleBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:role")
        .check(&state, &user)
        .await?;
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::bad_request("角色名称不能为空"));
    }
    let org_id: Uuid = body
        .organization_id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("organizationId 格式错误"))?;
    let (tenant_id,): (Uuid,) = sqlx::query_as("SELECT tenant_id FROM organizations WHERE id = $1")
        .bind(org_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found("组织不存在"))?;
    state.ensure_org_scope(&user, org_id, "组织").await?;
    let scoped_orgs = state.scoped_org_ids(&user).await?;
    let mut tx = state.pool.begin().await?;
    lock_actor(&mut tx, &user).await?;
    revalidate_actor(&mut tx, &user, scoped_orgs.as_deref()).await?;
    let row: RoleListRow = sqlx::query_as(
        "INSERT INTO roles (tenant_id, organization_id, name, code, description, id) VALUES ($1, $2, $3, $4, $5, $6) \
         RETURNING id, name, COALESCE(code,'') AS code, COALESCE(description,'') AS description, \
         organization_id, (SELECT name FROM organizations WHERE id = $2) AS organization_name, tenant_id, \
         status, built_in, created_at, 0::bigint AS menu_count, 0::bigint AS user_count",
    )
    .bind(tenant_id)
    .bind(org_id)
    .bind(&name)
    .bind(&body.code)
    .bind(&body.description)
    .bind(crate::util::uuid7::new_uuid()).fetch_one(&mut *tx)
    .await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let detail = audit::change_detail("role", row.id, Value::Null, role_summary(&row));
    audit::write_in_tx(
        &mut tx,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(tenant_id),
            organization_id: Some(org_id),
            action: "CREATE_ROLE",
            detail: &detail,
            ip: &ip,
            user_agent: &ua,
        },
    )
    .await?;
    revalidate_actor(&mut tx, &user, scoped_orgs.as_deref()).await?;
    tx.commit().await?;
    Ok(Json(serde_json::to_value(&row).unwrap_or_default()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoleUpdate {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

/// PUT /api/roles/{id}
async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<RoleUpdate>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:role")
        .check(&state, &user)
        .await?;
    let id: Uuid = id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    // 隔离校验需在写库前: 先读角色归属再更新
    let loaded = load_role_org(&state.pool, id).await?;
    ensure_role_in_scope(&state, &user, loaded).await?;
    if let Some(name) = &body.name {
        if name.trim().is_empty() {
            return Err(AppError::bad_request("角色名称不能为空"));
        }
    }
    let scoped_orgs = state.scoped_org_ids(&user).await?;
    let mut tx = state.pool.begin().await?;
    lock_actor(&mut tx, &user).await?;
    let before = lock_role(&mut tx, id).await?;
    revalidate_actor(&mut tx, &user, scoped_orgs.as_deref()).await?;
    if before.managed() {
        return Err(AppError::conflict("默认管理员角色由系统维护，不允许修改"));
    }
    let employee_support =
        super::users::role_employee_support_sql("ur.user_id", "ur.organization_id", "$5");
    let row: RoleListRow = sqlx::query_as(&format!(
        "UPDATE roles SET name = COALESCE($2, name), code = COALESCE($3, code), \
         description = COALESCE($4, description), updated_at = now() WHERE id = $1 \
         RETURNING id, name, COALESCE(code,'') AS code, COALESCE(description,'') AS description, \
         organization_id, (SELECT name FROM organizations WHERE id = roles.organization_id) AS organization_name, \
         tenant_id, status, built_in, created_at, \
         (SELECT count(*) FROM role_menus rm WHERE rm.role_id = roles.id) AS menu_count, \
         (SELECT count(*) FROM user_roles ur WHERE ur.role_id = roles.id AND ur.organization_id=roles.organization_id \
         AND {employee_support} AND ($5::uuid[] IS NULL OR ur.organization_id=ANY($5))) AS user_count"
    ))
    .bind(id)
    .bind(&body.name)
    .bind(&body.code)
    .bind(&body.description)
    .bind(&scoped_orgs)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("角色不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let changed_fields: Vec<&str> = [
        ("name", before.name != row.name),
        ("code", before.code != row.code),
        ("description", before.description != row.description),
    ]
    .into_iter()
    .filter_map(|(field, changed)| changed.then_some(field))
    .collect();
    if !changed_fields.is_empty() {
        let mut after = role_summary(&row);
        after["changedFields"] = json!(changed_fields);
        let detail = audit::change_detail("role", id, before.summary(), after);
        audit::write_in_tx(
            &mut tx,
            audit::AuditInfo {
                user_id: Some(user.user_id),
                username: &user.username,
                tenant_id: Some(row.tenant_id),
                organization_id: Some(row.organization_id),
                action: "UPDATE_ROLE",
                detail: &detail,
                ip: &ip,
                user_agent: &ua,
            },
        )
        .await?;
    }
    revalidate_actor(&mut tx, &user, scoped_orgs.as_deref()).await?;
    tx.commit().await?;
    Ok(Json(serde_json::to_value(&row).unwrap_or_default()))
}

/// DELETE /api/roles/{id} — built_in 禁删
async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:role")
        .check(&state, &user)
        .await?;
    let id: Uuid = id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let (org_id, tenant_id, name) = load_role_org(&state.pool, id).await?;
    ensure_role_in_scope(&state, &user, (org_id, tenant_id, name)).await?;
    let scoped_orgs = state.scoped_org_ids(&user).await?;
    let mut tx = state.pool.begin().await?;
    lock_actor(&mut tx, &user).await?;
    let before = lock_role(&mut tx, id).await?;
    revalidate_actor(&mut tx, &user, scoped_orgs.as_deref()).await?;
    if before.built_in {
        return Err(AppError::conflict("内置角色不允许删除"));
    }
    // Upgrade the parent lock before the deletion snapshot and its cascades.
    sqlx::query("SELECT id FROM roles WHERE id=$1 FOR UPDATE")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let (menu_count, user_count): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM role_menus WHERE role_id=$1), \
         (SELECT count(*) FROM user_roles WHERE role_id=$1)",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    let mut summary = before.summary();
    summary["menuCount"] = json!(menu_count);
    summary["userCount"] = json!(user_count);
    sqlx::query("DELETE FROM roles WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let detail = audit::change_detail("role", id, summary, Value::Null);
    audit::write_in_tx(
        &mut tx,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(tenant_id),
            organization_id: Some(org_id),
            action: "DELETE_ROLE",
            detail: &detail,
            ip: &ip,
            user_agent: &ua,
        },
    )
    .await?;
    revalidate_actor(&mut tx, &user, scoped_orgs.as_deref()).await?;
    tx.commit().await?;
    state.invalidate_perm_cache();
    Ok(Json(json!({"ok": true})))
}

/// GET /api/roles/{id}/menus
async fn get_menus(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:role")
        .check(&state, &user)
        .await?;
    let id: Uuid = id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let loaded = load_role_org(&state.pool, id).await?;
    ensure_role_in_scope(&state, &user, loaded).await?;
    let rows: Vec<(Uuid,)> = sqlx::query_as("SELECT menu_id FROM role_menus WHERE role_id = $1")
        .bind(id)
        .fetch_all(&state.pool)
        .await?;
    let menu_ids: Vec<Uuid> = rows.into_iter().map(|(m,)| m).collect();
    Ok(Json(json!({"menuIds": menu_ids})))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoleMenusBody {
    #[serde(default)]
    menu_ids: Vec<String>,
}

/// PUT /api/roles/{id}/menus — 只能分配角色所属组织已授权(且租户已授权)的菜单
async fn put_menus(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<RoleMenusBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:role")
        .check(&state, &user)
        .await?;
    let id: Uuid = id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let (org_id, tenant_id, role_name) = load_role_org(&state.pool, id).await?;
    ensure_role_in_scope(&state, &user, (org_id, tenant_id, role_name.clone())).await?;
    let menu_ids: Vec<Uuid> = body
        .menu_ids
        .iter()
        .map(|m| crate::util::uuid7::parse_uuid(m))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AppError::bad_request("menuId 格式错误"))?;
    let mut unique: Vec<Uuid> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for m in menu_ids {
        if seen.insert(m) {
            unique.push(m);
        }
    }
    let scoped_orgs = state.scoped_org_ids(&user).await?;
    let mut tx = state.pool.begin().await?;
    lock_actor(&mut tx, &user).await?;
    let role = lock_role(&mut tx, id).await?;
    revalidate_actor(&mut tx, &user, scoped_orgs.as_deref()).await?;
    if role.managed() {
        return Err(AppError::conflict("默认管理员角色的菜单随授权自动同步"));
    }
    let before_ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT menu_id FROM role_menus WHERE role_id=$1 ORDER BY menu_id")
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
    if !unique.is_empty() {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM menus m WHERE m.id = ANY($1) \
             AND EXISTS(SELECT 1 FROM organization_menus om WHERE om.organization_id = $2 AND om.menu_id = m.id) \
             AND EXISTS(SELECT 1 FROM tenant_menus tm WHERE tm.tenant_id = $3 AND tm.menu_id = m.id)",
        )
        .bind(&unique)
        .bind(org_id)
        .bind(tenant_id)
        .fetch_one(&mut *tx)
        .await?;
        if n != unique.len() as i64 {
            return Err(AppError::bad_request("存在超出组织授权范围的菜单"));
        }
    }
    sqlx::query("DELETE FROM role_menus WHERE role_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    for menu in &unique {
        sqlx::query("INSERT INTO role_menus (role_id, menu_id) VALUES ($1, $2)")
            .bind(id)
            .bind(menu)
            .execute(&mut *tx)
            .await?;
    }
    let after_ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT menu_id FROM role_menus WHERE role_id=$1 ORDER BY menu_id")
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    if before_ids != after_ids {
        let detail = audit::change_detail(
            "role_menus",
            id,
            json!({"id": id, "name": role.name, "menuCount": before_ids.len(),
                   "menuIdsSha256": ids_digest(before_ids.iter().copied())}),
            json!({"id": id, "name": role.name, "menuCount": after_ids.len(),
                   "menuIdsSha256": ids_digest(after_ids.iter().copied())}),
        );
        audit::write_in_tx(
            &mut tx,
            audit::AuditInfo {
                user_id: Some(user.user_id),
                username: &user.username,
                tenant_id: Some(tenant_id),
                organization_id: Some(org_id),
                action: "ROLE_MENUS",
                detail: &detail,
                ip: &ip,
                user_agent: &ua,
            },
        )
        .await?;
    }
    revalidate_actor(&mut tx, &user, scoped_orgs.as_deref()).await?;
    tx.commit().await?;
    state.invalidate_perm_cache();
    Ok(Json(json!({"ok": true})))
}

/// GET /api/roles/{id}/users
async fn get_users(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:role")
        .check(&state, &user)
        .await?;
    let id: Uuid = id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let loaded = load_role_org(&state.pool, id).await?;
    let org_id = loaded.0;
    ensure_role_in_scope(&state, &user, loaded).await?;
    let employee_support =
        super::users::role_employee_support_sql("ur.user_id", "ur.organization_id", "$2");
    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(&format!(
        "SELECT u.id, u.username, COALESCE(u.real_name,'') FROM user_roles ur \
         JOIN users u ON u.id = ur.user_id WHERE ur.role_id = $1 AND ur.organization_id=$3 \
         AND {employee_support} AND ($2::uuid[] IS NULL OR ur.organization_id = ANY($2)) ORDER BY u.created_at",
    ))
    .bind(id)
    .bind(state.scoped_org_ids(&user).await?)
    .bind(org_id)
    .fetch_all(&state.pool)
    .await?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(user_id, username, real_name)| {
            json!({"userId": user_id, "username": username, "realName": real_name})
        })
        .collect();
    Ok(Json(Value::Array(items)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoleUsersBody {
    #[serde(default)]
    user_ids: Vec<String>,
}

/// PUT /api/roles/{id}/users
async fn put_users(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<RoleUsersBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:role")
        .check(&state, &user)
        .await?;
    let id: Uuid = id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let (org_id, tenant_id, role_name) = load_role_org(&state.pool, id).await?;
    ensure_role_in_scope(&state, &user, (org_id, tenant_id, role_name.clone())).await?;
    let user_ids: Vec<Uuid> = body
        .user_ids
        .iter()
        .map(|u| crate::util::uuid7::parse_uuid(u))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AppError::bad_request("userId 格式错误"))?;
    let mut unique: Vec<Uuid> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for u in user_ids {
        if seen.insert(u) {
            unique.push(u);
        }
    }
    let scoped_orgs = state.scoped_org_ids(&user).await?;
    let mut tx = state.pool.begin().await?;
    let mut account_locks = unique.clone();
    account_locks.push(user.user_id);
    account_locks.sort_unstable();
    account_locks.dedup();
    sqlx::query("SELECT id FROM users WHERE id = ANY($1) ORDER BY id FOR NO KEY UPDATE")
        .bind(&account_locks)
        .execute(&mut *tx)
        .await?;
    revalidate_actor(&mut tx, &user, scoped_orgs.as_deref()).await?;
    if !unique.is_empty() {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM users u WHERE u.id = ANY($1) AND u.status='ENABLED' \
             AND EXISTS(SELECT 1 FROM memberships m JOIN organizations o ON o.id=m.organization_id \
                 JOIN tenants t ON t.id=o.tenant_id WHERE m.user_id=u.id \
                 AND m.member_type='EMPLOYEE' AND m.status='ENABLED' \
                 AND t.status='ENABLED' AND (t.expires_at IS NULL OR t.expires_at>clock_timestamp()) \
                 AND org_chain_active(o.id) \
                 AND (org_chain_expires_at(o.id) IS NULL OR org_chain_expires_at(o.id)>clock_timestamp()) \
                 AND ($2::uuid[] IS NULL OR m.organization_id=ANY($2)))",
        )
        .bind(&unique)
        .bind(&scoped_orgs)
        .fetch_one(&mut *tx)
        .await?;
        if n != unique.len() as i64 {
            return Err(AppError::not_found("用户不存在"));
        }
    }
    let role = lock_role(&mut tx, id).await?;
    revalidate_actor(&mut tx, &user, scoped_orgs.as_deref()).await?;
    for uid in &unique {
        super::users::ensure_role_membership_in_tx(&mut tx, *uid, org_id).await?;
    }
    let employee_support =
        super::users::role_employee_support_sql("ur.user_id", "ur.organization_id", "$2");
    let before_bindings: Vec<(Uuid, Uuid)> = sqlx::query_as(&format!(
        "SELECT ur.user_id, ur.organization_id FROM user_roles ur WHERE ur.role_id=$1 AND ur.organization_id=$3 \
         AND {employee_support} AND ($2::uuid[] IS NULL OR ur.organization_id = ANY($2)) \
         ORDER BY ur.user_id, ur.organization_id",
    ))
    .bind(id)
    .bind(&scoped_orgs)
    .bind(org_id)
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query(&format!(
        "DELETE FROM user_roles ur WHERE ur.role_id = $1 AND ur.organization_id=$3 \
        AND {employee_support} AND ($2::uuid[] IS NULL OR ur.organization_id = ANY($2))",
    ))
    .bind(id)
    .bind(&scoped_orgs)
    .bind(org_id)
    .execute(&mut *tx)
    .await?;
    for uid in &unique {
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_id, organization_id) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(uid)
        .bind(id)
        .bind(org_id)
        .execute(&mut *tx)
        .await?;
    }
    let after_bindings: Vec<(Uuid, Uuid)> = sqlx::query_as(&format!(
        "SELECT ur.user_id, ur.organization_id FROM user_roles ur WHERE ur.role_id=$1 AND ur.organization_id=$3 \
         AND {employee_support} AND ($2::uuid[] IS NULL OR ur.organization_id = ANY($2)) \
         ORDER BY ur.user_id, ur.organization_id",
    ))
    .bind(id)
    .bind(&scoped_orgs)
    .bind(org_id)
    .fetch_all(&mut *tx)
    .await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    if before_bindings != after_bindings {
        let detail = audit::change_detail(
            "role_users",
            id,
            json!({"id": id, "name": role.name, "userCount": before_bindings.len(),
                   "userBindingsSha256": ids_digest(before_bindings.iter().flat_map(|(uid, org)| [*uid, *org]))}),
            json!({"id": id, "name": role.name, "userCount": after_bindings.len(),
                   "userBindingsSha256": ids_digest(after_bindings.iter().flat_map(|(uid, org)| [*uid, *org]))}),
        );
        audit::write_in_tx(
            &mut tx,
            audit::AuditInfo {
                user_id: Some(user.user_id),
                username: &user.username,
                tenant_id: Some(tenant_id),
                organization_id: Some(org_id),
                action: "UPDATE_ROLE",
                detail: &detail,
                ip: &ip,
                user_agent: &ua,
            },
        )
        .await?;
    }
    for uid in &unique {
        super::users::ensure_role_membership_in_tx(&mut tx, *uid, org_id).await?;
    }
    revalidate_actor(&mut tx, &user, scoped_orgs.as_deref()).await?;
    tx.commit().await?;
    state.invalidate_perm_cache();
    Ok(Json(json!({"ok": true})))
}
