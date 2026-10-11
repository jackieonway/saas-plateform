use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
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

const LIST_SELECT: &str = "SELECT r.id, r.name, COALESCE(r.code,'') AS code, COALESCE(r.description,'') AS description, \
 r.organization_id, o.name AS organization_name, r.tenant_id, r.status, r.built_in, r.created_at, \
 (SELECT count(*) FROM role_menus rm WHERE rm.role_id = r.id) AS menu_count, \
 (SELECT count(*) FROM user_roles ur WHERE ur.role_id = r.id) AS user_count";

/// GET /api/roles — 分页 + keyword/tenantId/organizationId; 非平台租户仅看本租户角色
async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:role").check(&state, &user).await?;
    let (page, size, offset) = paging(&params);
    let kw = params.get("keyword").cloned().unwrap_or_default();
    let tenant_filter: Option<Uuid> = if state.is_platform_context(&user).await? {
        params.get("tenantId").and_then(|v| Uuid::parse_str(v).ok())
    } else {
        Some(user.tenant_id)
    };
    let org_filter: Option<Uuid> = params.get("organizationId").and_then(|v| Uuid::parse_str(v).ok());
    let where_clause = "($1 = '' OR r.name ILIKE '%' || $1 || '%' OR r.code ILIKE '%' || $1 || '%') \
     AND ($2::uuid IS NULL OR r.tenant_id = $2) AND ($3::uuid IS NULL OR r.organization_id = $3)";
    let rows: Vec<RoleListRow> = sqlx::query_as(&format!(
        "{LIST_SELECT} FROM roles r JOIN organizations o ON o.id = r.organization_id \
         WHERE {where_clause} ORDER BY r.created_at, r.id LIMIT $4 OFFSET $5"
    ))
    .bind(&kw)
    .bind(tenant_filter)
    .bind(org_filter)
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
    .fetch_one(&state.pool)
    .await?;
    let records: Vec<Value> = rows.iter().map(|r| serde_json::to_value(r).unwrap_or_default()).collect();
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
    let row: Option<(Uuid, Uuid, String)> = sqlx::query_as(
        "SELECT organization_id, tenant_id, name FROM roles WHERE id = $1",
    )
    .bind(role)
    .fetch_optional(pool)
    .await?;
    row.ok_or_else(|| AppError::not_found("角色不存在"))
}

/// 非平台上下文的目标角色隔离校验 (角色自带 tenant_id 列)
async fn ensure_role_in_scope(
    state: &AppState,
    user: &CurrentUser,
    role: (Uuid, Uuid, String),
) -> Result<(), AppError> {
    state.ensure_tenant_scope(user, role.1, "角色").await
}

/// POST /api/roles
async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<RoleBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:role").check(&state, &user).await?;
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::bad_request("角色名称不能为空"));
    }
    let org_id: Uuid = body
        .organization_id
        .parse()
        .map_err(|_| AppError::bad_request("organizationId 格式错误"))?;
    let (tenant_id,): (Uuid,) = sqlx::query_as(
        "SELECT tenant_id FROM organizations WHERE id = $1",
    )
    .bind(org_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found("组织不存在"))?;
    // 非平台租户只能在本租户组织上建角色
    state.ensure_tenant_scope(&user, tenant_id, "组织").await?;
    let row: RoleListRow = sqlx::query_as(
        "INSERT INTO roles (tenant_id, organization_id, name, code, description) VALUES ($1, $2, $3, $4, $5) \
         RETURNING id, name, COALESCE(code,'') AS code, COALESCE(description,'') AS description, \
         organization_id, (SELECT name FROM organizations WHERE id = $2) AS organization_name, tenant_id, \
         status, built_in, created_at, 0::bigint AS menu_count, 0::bigint AS user_count",
    )
    .bind(tenant_id)
    .bind(org_id)
    .bind(&name)
    .bind(&body.code)
    .bind(&body.description)
    .fetch_one(&state.pool)
    .await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(tenant_id), organization_id: Some(org_id),
        action: "CREATE_ROLE", detail: &format!("创建角色: {}", name), ip: &ip, user_agent: &ua,
    }).await;
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
    RequirePermission("system:role").check(&state, &user).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    // 隔离校验需在写库前: 先读角色归属再更新
    let loaded = load_role_org(&state.pool, id).await?;
    ensure_role_in_scope(&state, &user, loaded).await?;
    let managed: bool = sqlx::query_scalar(
        "SELECT built_in AND code IN ('tenant_admin', 'org_admin') FROM roles WHERE id = $1",
    ).bind(id).fetch_one(&state.pool).await?;
    if managed { return Err(AppError::conflict("默认管理员角色由系统维护，不允许修改")); }
    if let Some(name) = &body.name {
        if name.trim().is_empty() {
            return Err(AppError::bad_request("角色名称不能为空"));
        }
    }
    let row: RoleListRow = sqlx::query_as(&format!(
        "UPDATE roles SET name = COALESCE($2, name), code = COALESCE($3, code), \
         description = COALESCE($4, description), updated_at = now() WHERE id = $1 \
         RETURNING id, name, COALESCE(code,'') AS code, COALESCE(description,'') AS description, \
         organization_id, (SELECT name FROM organizations WHERE id = roles.organization_id) AS organization_name, \
         tenant_id, status, built_in, created_at, \
         (SELECT count(*) FROM role_menus rm WHERE rm.role_id = roles.id) AS menu_count, \
         (SELECT count(*) FROM user_roles ur WHERE ur.role_id = roles.id) AS user_count"
    ))
    .bind(id)
    .bind(&body.name)
    .bind(&body.code)
    .bind(&body.description)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found("角色不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(row.tenant_id), organization_id: Some(row.organization_id),
        action: "UPDATE_ROLE", detail: &format!("更新角色: {}", row.name), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(serde_json::to_value(&row).unwrap_or_default()))
}

/// DELETE /api/roles/{id} — built_in 禁删
async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:role").check(&state, &user).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let (org_id, tenant_id, name, built_in): (Uuid, Uuid, String, bool) = sqlx::query_as(
        "SELECT organization_id, tenant_id, name, built_in FROM roles WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found("角色不存在"))?;
    if built_in {
        return Err(AppError::conflict("内置角色不允许删除"));
    }
    ensure_role_in_scope(&state, &user, (org_id, tenant_id, name.clone())).await?;
    sqlx::query("DELETE FROM roles WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    state.invalidate_perm_cache();
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(tenant_id), organization_id: Some(org_id),
        action: "DELETE_ROLE", detail: &format!("删除角色: {}", name), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(json!({"ok": true})))
}

/// GET /api/roles/{id}/menus
async fn get_menus(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:role").check(&state, &user).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
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
    RequirePermission("system:role").check(&state, &user).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let (org_id, tenant_id, role_name) = load_role_org(&state.pool, id).await?;
    ensure_role_in_scope(&state, &user, (org_id, tenant_id, role_name.clone())).await?;
    let managed: bool = sqlx::query_scalar(
        "SELECT built_in AND code IN ('tenant_admin', 'org_admin') FROM roles WHERE id = $1",
    ).bind(id).fetch_one(&state.pool).await?;
    if managed { return Err(AppError::conflict("默认管理员角色的菜单随授权自动同步")); }
    let menu_ids: Vec<Uuid> = body
        .menu_ids
        .iter()
        .map(|m| Uuid::parse_str(m))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AppError::bad_request("menuId 格式错误"))?;
    let mut unique: Vec<Uuid> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for m in menu_ids {
        if seen.insert(m) {
            unique.push(m);
        }
    }
    if !unique.is_empty() {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM menus m WHERE m.id = ANY($1) \
             AND EXISTS(SELECT 1 FROM organization_menus om WHERE om.organization_id = $2 AND om.menu_id = m.id) \
             AND EXISTS(SELECT 1 FROM tenant_menus tm WHERE tm.tenant_id = $3 AND tm.menu_id = m.id)",
        )
        .bind(&unique)
        .bind(org_id)
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await?;
        if n != unique.len() as i64 {
            return Err(AppError::bad_request("存在超出组织授权范围的菜单"));
        }
    }
    let mut tx = state.pool.begin().await?;
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
    tx.commit().await?;
    state.invalidate_perm_cache();
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(tenant_id), organization_id: Some(org_id),
        action: "ROLE_MENUS", detail: &format!("更新角色菜单: {} 共 {} 项", role_name, unique.len()), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(json!({"ok": true})))
}

/// GET /api/roles/{id}/users
async fn get_users(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:role").check(&state, &user).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let loaded = load_role_org(&state.pool, id).await?;
    ensure_role_in_scope(&state, &user, loaded).await?;
    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT u.id, u.username, COALESCE(u.real_name,'') FROM user_roles ur \
         JOIN users u ON u.id = ur.user_id WHERE ur.role_id = $1 ORDER BY u.created_at",
    )
    .bind(id)
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
    RequirePermission("system:role").check(&state, &user).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let (org_id, tenant_id, role_name) = load_role_org(&state.pool, id).await?;
    ensure_role_in_scope(&state, &user, (org_id, tenant_id, role_name.clone())).await?;
    let user_ids: Vec<Uuid> = body
        .user_ids
        .iter()
        .map(|u| Uuid::parse_str(u))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AppError::bad_request("userId 格式错误"))?;
    let mut unique: Vec<Uuid> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for u in user_ids {
        if seen.insert(u) {
            unique.push(u);
        }
    }
    if !unique.is_empty() {
        let platform = state.is_platform_context(&user).await?;
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM users u WHERE u.id = ANY($1) \
             AND ($2::bool OR EXISTS(SELECT 1 FROM memberships m \
                 JOIN organizations o ON o.id = m.organization_id \
                 WHERE m.user_id = u.id AND o.tenant_id = $3))",
        )
        .bind(&unique)
        .bind(platform)
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await?;
        if n != unique.len() as i64 {
            return Err(AppError::not_found("用户不存在"));
        }
    }
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM user_roles WHERE role_id = $1")
        .bind(id)
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
    tx.commit().await?;
    state.invalidate_perm_cache();
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(tenant_id), organization_id: Some(org_id),
        action: "UPDATE_ROLE", detail: &format!("更新角色用户: {} 共 {} 人", role_name, unique.len()), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(json!({"ok": true})))
}
