use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::{ApiJson, CurrentUser, RequirePermission};
use crate::state::AppState;
use crate::util::{audit, tree};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/organizations/tree", get(tree_handler))
        .route("/api/organizations", post(create))
        .route("/api/organizations/{id}", put(update).delete(delete))
        .route("/api/organizations/{id}/status", put(update_status))
        .route("/api/organizations/{id}/authorizations", get(get_authorizations).put(put_authorizations))
}

type OrgRowTuple = (Uuid, Uuid, Option<Uuid>, String, String, i32, String, String, Option<DateTime<Utc>>);

const ORG_SELECT: &str = "SELECT o.id, o.tenant_id, o.parent_id, o.name, COALESCE(o.code,'') AS code, \
 o.sort_order, COALESCE(o.description,'') AS description, o.status, o.expires_at";

fn org_json(r: &OrgRowTuple) -> Value {
    let (id, tenant_id, parent_id, name, code, sort_order, description, status, expires_at) = r;
    json!({
        "id": id, "tenantId": tenant_id, "parentId": parent_id, "name": name, "code": code,
        "sortOrder": sort_order, "description": description, "status": status,
        "expiresAt": expires_at,
    })
}

/// 三态可空字段反序列化: 键缺省 → None(不更新); 显式 null → Some(None)(清除置空); 有值 → Some(Some(v))
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Deserialize::deserialize(de).map(Some)
}

/// DB 层兜底: 根组织唯一索引 uq_org_root_per_tenant 冲突 → 友好 409 (并发窗口下 check-then-insert 竞态由唯一索引封死)
fn map_root_unique_violation(e: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(ref db) = e {
        if db.code().as_deref() == Some("23505") && db.constraint() == Some("uq_org_root_per_tenant") {
            return AppError::conflict("该租户已存在根组织, 每个租户只能有一个根组织");
        }
    }
    e.into()
}

/// GET /api/organizations/tree?tenantId=
/// 节点额外携带 departmentCount / userCount 统计 (架构图视图使用)
async fn tree_handler(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:organization").check(&state, &user).await?;
    let tenant_id: Uuid = if state.is_platform_context(&user).await? {
        params
            .get("tenantId")
            .and_then(|v| Uuid::parse_str(v).ok())
            .ok_or_else(|| AppError::bad_request("缺少 tenantId 参数"))?
    } else {
        // 非平台租户: 忽略传入 tenantId, 强制只能查看本租户组织树
        user.tenant_id
    };
    let rows: Vec<OrgRowTuple> = sqlx::query_as(&format!(
        "{ORG_SELECT} FROM organizations o WHERE o.tenant_id = $1 ORDER BY o.sort_order, o.created_at"
    ))
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    // 统计每个组织的部门数与直属成员数
    let org_ids: Vec<Uuid> = rows.iter().map(|r| r.0).collect();
    let mut dept_counts: HashMap<Uuid, i64> = HashMap::new();
    let mut member_counts: HashMap<Uuid, i64> = HashMap::new();
    if !org_ids.is_empty() {
        let dept_rows: Vec<(Uuid, i64)> =
            sqlx::query_as("SELECT organization_id, COUNT(*) FROM departments WHERE organization_id = ANY($1) GROUP BY organization_id")
                .bind(&org_ids)
                .fetch_all(&state.pool)
                .await?;
        dept_counts = dept_rows.into_iter().collect();
        let member_rows: Vec<(Uuid, i64)> =
            sqlx::query_as("SELECT organization_id, COUNT(*) FROM memberships WHERE organization_id = ANY($1) GROUP BY organization_id")
                .bind(&org_ids)
                .fetch_all(&state.pool)
                .await?;
        member_counts = member_rows.into_iter().collect();
    }

    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            let mut v = org_json(r);
            v["departmentCount"] = json!(dept_counts.get(&r.0).copied().unwrap_or(0));
            v["userCount"] = json!(member_counts.get(&r.0).copied().unwrap_or(0));
            v
        })
        .collect();
    Ok(Json(Value::Array(tree::build_tree(items))))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrgBody {
    tenant_id: String,
    #[serde(default)]
    parent_id: Option<String>,
    name: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    sort_order: i32,
    #[serde(default)]
    description: String,
    /// 过期时间: 缺省/null = 永久
    #[serde(default)]
    expires_at: Option<DateTime<Utc>>,
}

async fn parse_parent(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    parent: &Option<String>,
) -> Result<Option<Uuid>, AppError> {
    match parent {
        None => Ok(None),
        Some(p) if p.trim().is_empty() => Ok(None),
        Some(p) => {
            let pid: Uuid = p.parse().map_err(|_| AppError::bad_request("parentId 格式错误"))?;
            let (parent_tenant,): (Uuid,) =
                sqlx::query_as("SELECT tenant_id FROM organizations WHERE id = $1")
                    .bind(pid)
                    .fetch_optional(pool)
                    .await?
                    .ok_or_else(|| AppError::bad_request("父组织不存在"))?;
            if parent_tenant != tenant_id {
                return Err(AppError::bad_request("父组织不属于该租户"));
            }
            Ok(Some(pid))
        }
    }
}

/// POST /api/organizations
async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<OrgBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:organization").check(&state, &user).await?;
    let tenant_id: Uuid = body
        .tenant_id
        .parse()
        .map_err(|_| AppError::bad_request("tenantId 格式错误"))?;
    // 非平台租户只能在本租户下建组织
    if tenant_id != user.tenant_id {
        state.require_platform(&user).await?;
    }
    let tenant_exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tenants WHERE id = $1)")
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await?;
    if !tenant_exists {
        return Err(AppError::bad_request("所属租户不存在"));
    }
    let parent_id = parse_parent(&state.pool, tenant_id, &body.parent_id).await?;
    // 根组织唯一: 每个租户仅允许一个 parent_id IS NULL 的组织 (应用层前置校验, DB 唯一索引兜底)
    if parent_id.is_none() {
        let root_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM organizations WHERE tenant_id = $1 AND parent_id IS NULL)",
        )
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await?;
        if root_exists {
            return Err(AppError::conflict("该租户已存在根组织, 每个租户只能有一个根组织"));
        }
    }
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::bad_request("组织名称不能为空"));
    }
    let row: OrgRowTuple = sqlx::query_as(
        "INSERT INTO organizations (tenant_id, parent_id, name, code, sort_order, description, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) \
         RETURNING id, tenant_id, parent_id, name, COALESCE(code,'') AS code, sort_order, \
         COALESCE(description,'') AS description, status, expires_at",
    )
    .bind(tenant_id)
    .bind(parent_id)
    .bind(&name)
    .bind(&body.code)
    .bind(body.sort_order)
    .bind(&body.description)
    .bind(body.expires_at)
    .fetch_one(&state.pool)
    .await
    .map_err(map_root_unique_violation)?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(tenant_id), organization_id: Some(user.org_id),
        action: "CREATE_ORGANIZATION", detail: &format!("创建组织: {}", name), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(org_json(&row)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrgUpdate {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    sort_order: Option<i32>,
    #[serde(default)]
    description: Option<String>,
    /// 过期时间三态: 缺省不更新 / 显式 null 清除(永久) / 有值则设置
    /// (注: 本端点不支持修改 parent_id, 根组织归属仅由创建时决定)
    #[serde(default, deserialize_with = "double_option")]
    expires_at: Option<Option<DateTime<Utc>>>,
}

/// PUT /api/organizations/{id}
async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<OrgUpdate>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:organization").check(&state, &user).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_org_in_scope(&state, &user, id).await?;
    if let Some(name) = &body.name {
        if name.trim().is_empty() {
            return Err(AppError::bad_request("组织名称不能为空"));
        }
    }
    let row: OrgRowTuple = sqlx::query_as(
        "UPDATE organizations SET name = COALESCE($2, name), code = COALESCE($3, code), \
         sort_order = COALESCE($4, sort_order), description = COALESCE($5, description), \
         expires_at = CASE WHEN $6 THEN $7::timestamptz ELSE expires_at END, updated_at = now() \
         WHERE id = $1 \
         RETURNING id, tenant_id, parent_id, name, COALESCE(code,'') AS code, sort_order, \
         COALESCE(description,'') AS description, status, expires_at",
    )
    .bind(id)
    .bind(&body.name)
    .bind(&body.code)
    .bind(body.sort_order)
    .bind(&body.description)
    .bind(body.expires_at.is_some())
    .bind(body.expires_at.flatten())
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found("组织不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(row.1), organization_id: Some(id),
        action: "UPDATE_ORGANIZATION", detail: &format!("更新组织: {}", row.3), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(org_json(&row)))
}

/// DELETE /api/organizations/{id} — 级联子组织/部门/成员/授权/角色
async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:organization").check(&state, &user).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_org_in_scope(&state, &user, id).await?;
    let (name,): (String,) = sqlx::query_as("DELETE FROM organizations WHERE id = $1 RETURNING name")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found("组织不存在"))?;
    state.invalidate_perm_cache();
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: None,
        action: "DELETE_ORGANIZATION", detail: &format!("删除组织(级联): {}", name), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct StatusBody {
    status: String,
}

/// PUT /api/organizations/{id}/status
async fn update_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<StatusBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:organization").check(&state, &user).await?;
    if !["ENABLED", "DISABLED"].contains(&body.status.as_str()) {
        return Err(AppError::bad_request("状态仅支持 ENABLED / DISABLED"));
    }
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_org_in_scope(&state, &user, id).await?;
    let (name,): (String,) = sqlx::query_as(
        "UPDATE organizations SET status = $2, updated_at = now() WHERE id = $1 RETURNING name",
    )
    .bind(id)
    .bind(&body.status)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found("组织不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(id),
        action: "UPDATE_ORGANIZATION", detail: &format!("组织状态变更为 {}: {}", body.status, name), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(json!({"ok": true})))
}

// ===== 组织授权 =====

#[derive(sqlx::FromRow)]
#[allow(dead_code)]
struct AppAuthRow {
    id: Uuid,
    code: String,
    name: String,
    icon: String,
    authorized: bool,
    menu_count: i64,
    authorized_menu_count: i64,
}

#[derive(sqlx::FromRow)]
struct MenuFlatRow {
    id: Uuid,
    parent_id: Option<Uuid>,
    r#type: String,
    title: String,
    path: String,
    icon: String,
    permission: String,
    link_url: String,
    link_target: String,
    sort_order: i32,
    visible: bool,
    authorized: bool,
}

async fn load_org_tenant(pool: &sqlx::PgPool, org_id: Uuid) -> Result<Uuid, AppError> {
    let row: Option<(Uuid,)> = sqlx::query_as("SELECT tenant_id FROM organizations WHERE id = $1")
        .bind(org_id)
        .fetch_optional(pool)
        .await?;
    row.map(|(t,)| t).ok_or_else(|| AppError::not_found("组织不存在"))
}

/// 非平台上下文的目标组织隔离校验 (不存在交由后续 not_found)
async fn ensure_org_in_scope(state: &AppState, user: &CurrentUser, org_id: Uuid) -> Result<(), AppError> {
    if state.is_platform_context(user).await? {
        return Ok(());
    }
    let tenant: Option<Uuid> = sqlx::query_scalar("SELECT tenant_id FROM organizations WHERE id = $1")
        .bind(org_id)
        .fetch_optional(&state.pool)
        .await?;
    match tenant {
        None => Ok(()),
        Some(t) if t == user.tenant_id => Ok(()),
        Some(_) => Err(AppError::forbidden("目标组织不属于当前租户")),
    }
}

/// GET /api/organizations/{id}/authorizations — 仅展示租户已授权范围
async fn get_authorizations(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:authorization").check(&state, &user).await?;
    let org_id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_org_in_scope(&state, &user, org_id).await?;
    let tenant_id = load_org_tenant(&state.pool, org_id).await?;

    let apps: Vec<AppAuthRow> = sqlx::query_as(
        "SELECT a.id, a.code, a.name, COALESCE(a.icon,'') AS icon, \
         EXISTS(SELECT 1 FROM organization_applications oa WHERE oa.organization_id = $1 AND oa.application_id = a.id) AS authorized, \
         (SELECT count(*) FROM menus m JOIN tenant_menus tm ON tm.menu_id = m.id WHERE tm.tenant_id = $2 AND m.application_id = a.id) AS menu_count, \
         (SELECT count(*) FROM organization_menus om JOIN tenant_menus tm ON tm.menu_id = om.menu_id \
          WHERE om.organization_id = $1 AND tm.tenant_id = $2 AND om.menu_id IN (SELECT id FROM menus WHERE application_id = a.id)) AS authorized_menu_count \
         FROM applications a \
         JOIN tenant_applications ta ON ta.application_id = a.id AND ta.tenant_id = $2 \
         ORDER BY a.sort_order, a.created_at",
    )
    .bind(org_id)
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let mut menus_map = serde_json::Map::new();
    for app in &apps {
        let rows: Vec<MenuFlatRow> = sqlx::query_as(
            "SELECT m.id, m.parent_id, m.type, m.title, COALESCE(m.path,'') AS path, COALESCE(m.icon,'') AS icon, \
             COALESCE(m.permission,'') AS permission, m.link_url, m.link_target, m.sort_order, m.visible, \
             EXISTS(SELECT 1 FROM organization_menus om WHERE om.organization_id = $1 AND om.menu_id = m.id) AS authorized \
             FROM menus m \
             JOIN tenant_menus tm ON tm.menu_id = m.id AND tm.tenant_id = $2 \
             WHERE m.application_id = $3 ORDER BY m.sort_order, m.created_at",
        )
        .bind(org_id)
        .bind(tenant_id)
        .bind(app.id)
        .fetch_all(&state.pool)
        .await?;
        menus_map.insert(
            app.id.to_string(),
            Value::Array(
                rows.iter()
                    .map(|m| {
                        json!({
                            "id": m.id, "parentId": m.parent_id, "type": m.r#type, "title": m.title,
                            "path": m.path, "icon": m.icon, "permission": m.permission,
                            "sortOrder": m.sort_order, "visible": m.visible, "authorized": m.authorized,
                            "linkUrl": m.link_url, "linkTarget": m.link_target,
                        })
                    })
                    .collect(),
            ),
        );
    }
    let apps_json: Vec<Value> = apps
        .iter()
        .map(|a| {
            json!({
                "id": a.id, "code": a.code, "name": a.name, "icon": a.icon, "authorized": a.authorized,
                "menuCount": a.menu_count, "authorizedMenuCount": a.authorized_menu_count,
            })
        })
        .collect();
    Ok(Json(json!({
        "applications": apps_json,
        "menus": Value::Object(menus_map),
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthzBody {
    #[serde(default)]
    applications: Vec<AuthzItemApp>,
    #[serde(default)]
    menus: Vec<AuthzItemMenu>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthzItemApp {
    application_id: String,
    authorized: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthzItemMenu {
    menu_id: String,
    authorized: bool,
}

/// PUT /api/organizations/{id}/authorizations — 写入校验 ⊆ 租户授权; 取消时级联清理本组织角色菜单
async fn put_authorizations(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<AuthzBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:authorization").check(&state, &user).await?;
    let org_id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_org_in_scope(&state, &user, org_id).await?;
    let tenant_id = load_org_tenant(&state.pool, org_id).await?;

    let app_ids: Vec<Uuid> = body
        .applications
        .iter()
        .map(|i| i.application_id.parse::<Uuid>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AppError::bad_request("applicationId 格式错误"))?;
    let menu_ids: Vec<Uuid> = body
        .menus
        .iter()
        .map(|i| i.menu_id.parse::<Uuid>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AppError::bad_request("menuId 格式错误"))?;

    // 校验: 应用/菜单必须已在租户授权范围内
    if !app_ids.is_empty() {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM tenant_applications WHERE tenant_id = $1 AND application_id = ANY($2)",
        )
        .bind(tenant_id)
        .bind(&app_ids)
        .fetch_one(&state.pool)
        .await?;
        if n != app_ids.len() as i64 {
            return Err(AppError::bad_request("应用超出租户授权范围"));
        }
    }
    if !menu_ids.is_empty() {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM tenant_menus WHERE tenant_id = $1 AND menu_id = ANY($2)",
        )
        .bind(tenant_id)
        .bind(&menu_ids)
        .fetch_one(&state.pool)
        .await?;
        if n != menu_ids.len() as i64 {
            return Err(AppError::bad_request("菜单超出租户授权范围"));
        }
    }

    let mut tx = state.pool.begin().await?;
    for (item, app) in body.applications.iter().zip(app_ids.iter()) {
        if item.authorized {
            sqlx::query("INSERT INTO organization_applications (organization_id, application_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(org_id)
                .bind(app)
                .execute(&mut *tx)
                .await?;
        } else {
            // 取消应用: 清理本组织菜单授权 + 本组织角色的该应用菜单
            sqlx::query(
                "DELETE FROM organization_menus om USING menus m \
                 WHERE om.organization_id = $1 AND om.menu_id = m.id AND m.application_id = $2",
            )
            .bind(org_id)
            .bind(app)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "DELETE FROM role_menus rm USING roles r, menus m \
                 WHERE rm.role_id = r.id AND r.organization_id = $1 AND rm.menu_id = m.id AND m.application_id = $2",
            )
            .bind(org_id)
            .bind(app)
            .execute(&mut *tx)
            .await?;
            sqlx::query("DELETE FROM organization_applications WHERE organization_id = $1 AND application_id = $2")
                .bind(org_id)
                .bind(app)
                .execute(&mut *tx)
                .await?;
        }
    }
    for (item, menu) in body.menus.iter().zip(menu_ids.iter()) {
        if item.authorized {
            sqlx::query("INSERT INTO organization_menus (organization_id, menu_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(org_id)
                .bind(menu)
                .execute(&mut *tx)
                .await?;
        } else {
            // 取消菜单: 清理本组织角色的该菜单
            sqlx::query(
                "DELETE FROM role_menus rm USING roles r WHERE rm.role_id = r.id AND r.organization_id = $1 AND rm.menu_id = $2",
            )
            .bind(org_id)
            .bind(menu)
            .execute(&mut *tx)
            .await?;
            sqlx::query("DELETE FROM organization_menus WHERE organization_id = $1 AND menu_id = $2")
                .bind(org_id)
                .bind(menu)
                .execute(&mut *tx)
                .await?;
        }
    }
    // 兜底对账: 组织菜单 ⊆ 组织应用 ∩ 租户菜单; 角色菜单 ⊆ 组织菜单 (仅本组织角色)
    sqlx::query(
        "DELETE FROM organization_menus om USING menus m \
         WHERE om.organization_id = $1 AND om.menu_id = m.id \
         AND NOT EXISTS (SELECT 1 FROM organization_applications oa WHERE oa.organization_id = $1 AND oa.application_id = m.application_id)",
    )
    .bind(org_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM role_menus rm USING roles r WHERE rm.role_id = r.id AND r.organization_id = $1 \
         AND NOT EXISTS (SELECT 1 FROM organization_menus om WHERE om.organization_id = $1 AND om.menu_id = rm.menu_id)",
    )
    .bind(org_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    state.invalidate_perm_cache();

    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(tenant_id), organization_id: Some(org_id),
        action: "ORG_AUTH",
        detail: &format!("组织授权变更: 应用 {} 项, 菜单 {} 项 (含级联清理)", body.applications.len(), body.menus.len()),
        ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(json!({"ok": true})))
}
