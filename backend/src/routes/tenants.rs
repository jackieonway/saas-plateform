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
        .route("/api/tenants", get(list).post(create))
        .route("/api/tenants/{id}", put(update).delete(delete))
        .route("/api/tenants/{id}/status", put(update_status))
        .route("/api/tenants/{id}/authorizations", get(get_authorizations).put(put_authorizations))
}

const LIST_SELECT: &str = "SELECT t.id, t.name, t.code, COALESCE(t.description,'') AS description, t.status, t.is_platform, t.created_at, t.expires_at, \
 (SELECT count(*) FROM organizations o WHERE o.tenant_id = t.id) AS organization_count, \
 (SELECT count(DISTINCT m.user_id) FROM memberships m JOIN organizations o ON o.id = m.organization_id WHERE o.tenant_id = t.id) AS user_count, \
 (SELECT count(*) FROM tenant_applications ta WHERE ta.tenant_id = t.id) AS authorized_app_count";

#[derive(sqlx::FromRow, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TenantRow {
    id: Uuid,
    name: String,
    code: String,
    description: String,
    status: String,
    is_platform: bool,
    created_at: DateTime<Utc>,
    /// 过期时间: null = 永久
    expires_at: Option<DateTime<Utc>>,
    organization_count: i64,
    user_count: i64,
    authorized_app_count: i64,
}

/// GET /api/tenants — 分页; 非平台租户仅能看到自己的租户 (只读视角)
/// 非平台上下文免权限检查: 只读视野不含他租户任何数据, 供租户内用户查看本租户概览;
/// 平台上下文需 system:tenant (全租户视野), 写操作一律需权限 + 平台上下文双重防线。
async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    let platform = state.is_platform_context(&user).await?;
    if platform {
        RequirePermission("system:tenant").check(&state, &user).await?;
    }
    let (page, size, offset) = paging(&params);
    let kw = params.get("keyword").cloned().unwrap_or_default();
    let st = params.get("status").cloned().unwrap_or_default();
    // 平台租户看全部; 其余租户强制仅看自己 (忽略任何传入过滤, 防跨租户枚举)
    let scope: Option<Uuid> = if platform { None } else { Some(user.tenant_id) };
    let rows: Vec<TenantRow> = sqlx::query_as(&format!(
        "{LIST_SELECT} FROM tenants t \
         WHERE ($1 = '' OR t.name ILIKE '%' || $1 || '%' OR t.code ILIKE '%' || $1 || '%') \
         AND ($4 = '' OR t.status = $4) \
         AND ($5::uuid IS NULL OR t.id = $5) \
         ORDER BY t.created_at LIMIT $2 OFFSET $3"
    ))
    .bind(&kw)
    .bind(size)
    .bind(offset)
    .bind(&st)
    .bind(scope)
    .fetch_all(&state.pool)
    .await?;
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenants t WHERE ($1 = '' OR t.name ILIKE '%' || $1 || '%' OR t.code ILIKE '%' || $1 || '%') \
         AND ($2 = '' OR t.status = $2) AND ($3::uuid IS NULL OR t.id = $3)",
    )
    .bind(&kw)
    .bind(&st)
    .bind(scope)
    .fetch_one(&state.pool)
    .await?;
    let records: Vec<Value> = rows.iter().map(|r| serde_json::to_value(r).unwrap_or_default()).collect();
    Ok(Json(page_json(records, total, page, size)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TenantBody {
    name: String,
    code: String,
    #[serde(default)]
    description: String,
    /// 过期时间: 缺省/null = 永久
    #[serde(default)]
    expires_at: Option<DateTime<Utc>>,
}

/// POST /api/tenants
async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<TenantBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:tenant").check(&state, &user).await?;
    state.require_platform(&user).await?;
    let name = body.name.trim().to_string();
    if name.is_empty() || body.code.trim().is_empty() {
        return Err(AppError::bad_request("租户名称与编码不能为空"));
    }
    let row: TenantRow = sqlx::query_as(&format!(
        "INSERT INTO tenants (name, code, description, expires_at) VALUES ($1, $2, $3, $4) RETURNING id, name, code, \
         description, status, false AS is_platform, created_at, expires_at, \
         0::bigint AS organization_count, 0::bigint AS user_count, 0::bigint AS authorized_app_count"
    ))
    .bind(&name)
    .bind(body.code.trim())
    .bind(&body.description)
    .bind(body.expires_at)
    .fetch_one(&state.pool)
    .await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(row.id), organization_id: Some(user.org_id),
        action: "CREATE_TENANT", detail: &format!("创建租户: {} ({})", row.name, row.code), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(serde_json::to_value(&row).unwrap_or_default()))
}

/// 三态可空字段反序列化: 键缺省 → None(不更新); 显式 null → Some(None)(清除置空); 有值 → Some(Some(v))
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Deserialize::deserialize(de).map(Some)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TenantUpdate {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    description: Option<String>,
    /// 过期时间三态: 缺省不更新 / 显式 null 清除(永久) / 有值则设置
    #[serde(default, deserialize_with = "double_option")]
    expires_at: Option<Option<DateTime<Utc>>>,
}

/// PUT /api/tenants/{id}
async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<TenantUpdate>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:tenant").check(&state, &user).await?;
    state.require_platform(&user).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    if let Some(name) = &body.name {
        if name.trim().is_empty() {
            return Err(AppError::bad_request("租户名称不能为空"));
        }
    }
    let row = sqlx::query_as::<_, TenantRow>(&format!(
        // 注: 目标表别名 t 供 RETURNING 子查询引用 (无别名时 o.tenant_id = t.id 报 missing FROM-clause,
        // 此前该端点一直 500, 本任务修复: UPDATE tenants AS t ... WHERE t.id = $1)
        "UPDATE tenants AS t SET name = COALESCE($2, name), code = COALESCE($3, code), \
         description = COALESCE($4, description), \
         expires_at = CASE WHEN $5 THEN $6::timestamptz ELSE expires_at END, updated_at = now() \
         WHERE t.id = $1 RETURNING id, name, code, description, status, is_platform, created_at, expires_at, \
         (SELECT count(*) FROM organizations o WHERE o.tenant_id = t.id) AS organization_count, \
         (SELECT count(DISTINCT m.user_id) FROM memberships m JOIN organizations o ON o.id = m.organization_id WHERE o.tenant_id = t.id) AS user_count, \
         (SELECT count(*) FROM tenant_applications ta WHERE ta.tenant_id = t.id) AS authorized_app_count"
    ))
    .bind(id)
    .bind(&body.name)
    .bind(&body.code)
    .bind(&body.description)
    .bind(body.expires_at.is_some())
    .bind(body.expires_at.flatten())
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found("租户不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(id), organization_id: Some(user.org_id),
        action: "UPDATE_TENANT", detail: &format!("更新租户: {}", row.name), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(serde_json::to_value(&row).unwrap_or_default()))
}

/// DELETE /api/tenants/{id} — 级联删除组织/授权/角色 (FK CASCADE); 平台租户禁删
async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:tenant").check(&state, &user).await?;
    state.require_platform(&user).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let is_platform: bool = sqlx::query_scalar("SELECT is_platform FROM tenants WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found("租户不存在"))?;
    if is_platform {
        return Err(AppError::bad_request("平台租户不允许删除"));
    }
    let (name,): (String,) = sqlx::query_as("DELETE FROM tenants WHERE id = $1 RETURNING name")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found("租户不存在"))?;
    state.invalidate_perm_cache();
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: None, organization_id: Some(user.org_id),
        action: "DELETE_TENANT", detail: &format!("删除租户(级联): {}", name), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct StatusBody {
    status: String,
}

/// PUT /api/tenants/{id}/status
async fn update_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<StatusBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:tenant").check(&state, &user).await?;
    state.require_platform(&user).await?;
    if !["ENABLED", "DISABLED"].contains(&body.status.as_str()) {
        return Err(AppError::bad_request("状态仅支持 ENABLED / DISABLED"));
    }
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let is_platform: bool = sqlx::query_scalar("SELECT is_platform FROM tenants WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found("租户不存在"))?;
    if is_platform {
        return Err(AppError::bad_request("平台租户不允许停用"));
    }
    let (name,): (String,) = sqlx::query_as(
        "UPDATE tenants SET status = $2, updated_at = now() WHERE id = $1 RETURNING name",
    )
    .bind(id)
    .bind(&body.status)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found("租户不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(id), organization_id: Some(user.org_id),
        action: "UPDATE_TENANT", detail: &format!("租户状态变更为 {}: {}", body.status, name), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(json!({"ok": true})))
}

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

fn menu_flat_json(m: &MenuFlatRow) -> Value {
    json!({
        "id": m.id, "parentId": m.parent_id, "type": m.r#type, "title": m.title,
        "path": m.path, "icon": m.icon, "permission": m.permission,
        "sortOrder": m.sort_order, "visible": m.visible, "authorized": m.authorized,
        "linkUrl": m.link_url, "linkTarget": m.link_target,
    })
}

fn app_auth_json(a: &AppAuthRow) -> Value {
    json!({
        "id": a.id, "code": a.code, "name": a.name, "icon": a.icon, "authorized": a.authorized,
        "menuCount": a.menu_count, "authorizedMenuCount": a.authorized_menu_count,
    })
}

/// GET /api/tenants/{id}/authorizations — 全部应用 + 各应用全部菜单平铺 (authorized 标记)
/// 租户级授权是平台职能: 仅平台租户可操作
async fn get_authorizations(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:tenant").check(&state, &user).await?;
    state.require_platform(&user).await?;
    let tenant_id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tenants WHERE id = $1)")
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await?;
    if !exists {
        return Err(AppError::not_found("租户不存在"));
    }
    let apps: Vec<AppAuthRow> = sqlx::query_as(
        "SELECT a.id, a.code, a.name, COALESCE(a.icon,'') AS icon, \
         EXISTS(SELECT 1 FROM tenant_applications ta WHERE ta.tenant_id = $1 AND ta.application_id = a.id) AS authorized, \
         (SELECT count(*) FROM menus m WHERE m.application_id = a.id) AS menu_count, \
         (SELECT count(*) FROM tenant_menus tm JOIN menus m ON m.id = tm.menu_id WHERE tm.tenant_id = $1 AND m.application_id = a.id) AS authorized_menu_count \
         FROM applications a ORDER BY a.sort_order, a.created_at",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let mut menus_map = serde_json::Map::new();
    for app in &apps {
        let rows: Vec<MenuFlatRow> = sqlx::query_as(
            "SELECT m.id, m.parent_id, m.type, m.title, COALESCE(m.path,'') AS path, COALESCE(m.icon,'') AS icon, \
             COALESCE(m.permission,'') AS permission, m.link_url, m.link_target, m.sort_order, m.visible, \
             EXISTS(SELECT 1 FROM tenant_menus tm WHERE tm.tenant_id = $1 AND tm.menu_id = m.id) AS authorized \
             FROM menus m WHERE m.application_id = $2 ORDER BY m.sort_order, m.created_at",
        )
        .bind(tenant_id)
        .bind(app.id)
        .fetch_all(&state.pool)
        .await?;
        menus_map.insert(
            app.id.to_string(),
            Value::Array(rows.iter().map(menu_flat_json).collect()),
        );
    }
    Ok(Json(json!({
        "applications": apps.iter().map(app_auth_json).collect::<Vec<_>>(),
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

/// PUT /api/tenants/{id}/authorizations — 全量对账 + 授权级联清理 (事务)
/// 租户级授权是平台职能: 仅平台租户可操作
async fn put_authorizations(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<AuthzBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:tenant").check(&state, &user).await?;
    state.require_platform(&user).await?;
    let tenant_id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tenants WHERE id = $1)")
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await?;
    if !exists {
        return Err(AppError::not_found("租户不存在"));
    }

    let mut app_ids: Vec<Uuid> = Vec::new();
    let mut app_authed: Vec<bool> = Vec::new();
    for item in &body.applications {
        let app: Uuid = item
            .application_id
            .parse()
            .map_err(|_| AppError::bad_request("applicationId 格式错误"))?;
        app_ids.push(app);
        app_authed.push(item.authorized);
    }
    let mut menu_ids: Vec<Uuid> = Vec::new();
    let mut menu_authed: Vec<bool> = Vec::new();
    for item in &body.menus {
        let menu: Uuid = item
            .menu_id
            .parse()
            .map_err(|_| AppError::bad_request("menuId 格式错误"))?;
        menu_ids.push(menu);
        menu_authed.push(item.authorized);
    }

    // 校验应用/菜单存在
    if !app_ids.is_empty() {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM applications WHERE id = ANY($1)")
            .bind(&app_ids)
            .fetch_one(&state.pool)
            .await?;
        if n != app_ids.len() as i64 {
            return Err(AppError::bad_request("存在无效的应用 ID"));
        }
    }
    if !menu_ids.is_empty() {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM menus WHERE id = ANY($1)")
            .bind(&menu_ids)
            .fetch_one(&state.pool)
            .await?;
        if n != menu_ids.len() as i64 {
            return Err(AppError::bad_request("存在无效的菜单 ID"));
        }
    }

    let mut tx = state.pool.begin().await?;
    for (app, authorized) in app_ids.iter().zip(app_authed.iter()) {
        if *authorized {
            sqlx::query("INSERT INTO tenant_applications (tenant_id, application_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(tenant_id)
                .bind(app)
                .execute(&mut *tx)
                .await?;
        } else {
            // 取消应用: 级联清理该租户所有组织的组织应用/组织菜单 + 该租户全部角色的角色菜单 + 租户菜单
            sqlx::query(
                "DELETE FROM organization_applications oa USING organizations o \
                 WHERE oa.organization_id = o.id AND o.tenant_id = $1 AND oa.application_id = $2",
            )
            .bind(tenant_id)
            .bind(app)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "DELETE FROM organization_menus om USING organizations o, menus m \
                 WHERE om.organization_id = o.id AND o.tenant_id = $1 AND om.menu_id = m.id AND m.application_id = $2",
            )
            .bind(tenant_id)
            .bind(app)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "DELETE FROM role_menus rm USING roles r, menus m \
                 WHERE rm.role_id = r.id AND r.tenant_id = $1 AND rm.menu_id = m.id AND m.application_id = $2",
            )
            .bind(tenant_id)
            .bind(app)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "DELETE FROM tenant_menus WHERE tenant_id = $1 AND menu_id IN (SELECT id FROM menus WHERE application_id = $2)",
            )
            .bind(tenant_id)
            .bind(app)
            .execute(&mut *tx)
            .await?;
            sqlx::query("DELETE FROM tenant_applications WHERE tenant_id = $1 AND application_id = $2")
                .bind(tenant_id)
                .bind(app)
                .execute(&mut *tx)
                .await?;
        }
    }
    for (menu, authorized) in menu_ids.iter().zip(menu_authed.iter()) {
        if *authorized {
            sqlx::query("INSERT INTO tenant_menus (tenant_id, menu_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(tenant_id)
                .bind(menu)
                .execute(&mut *tx)
                .await?;
        } else {
            // 取消菜单: 级联清理该租户组织的组织菜单 + 该租户角色的角色菜单 + 租户菜单
            sqlx::query(
                "DELETE FROM organization_menus WHERE menu_id = $2 AND organization_id IN (SELECT id FROM organizations WHERE tenant_id = $1)",
            )
            .bind(tenant_id)
            .bind(menu)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "DELETE FROM role_menus rm USING roles r WHERE rm.role_id = r.id AND r.tenant_id = $1 AND rm.menu_id = $2",
            )
            .bind(tenant_id)
            .bind(menu)
            .execute(&mut *tx)
            .await?;
            sqlx::query("DELETE FROM tenant_menus WHERE tenant_id = $1 AND menu_id = $2")
                .bind(tenant_id)
                .bind(menu)
                .execute(&mut *tx)
                .await?;
        }
    }
    // 兜底对账: 保证 租户菜单⊆租户应用、组织授权⊆租户授权、角色菜单⊆组织授权
    sqlx::query(
        "DELETE FROM tenant_menus tm WHERE tm.tenant_id = $1 AND NOT EXISTS ( \
         SELECT 1 FROM tenant_applications ta JOIN menus m ON m.id = tm.menu_id \
         WHERE ta.tenant_id = $1 AND ta.application_id = m.application_id)",
    )
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM organization_applications oa WHERE oa.organization_id IN (SELECT id FROM organizations WHERE tenant_id = $1) \
         AND NOT EXISTS (SELECT 1 FROM tenant_applications ta WHERE ta.tenant_id = $1 AND ta.application_id = oa.application_id)",
    )
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM organization_menus om WHERE om.organization_id IN (SELECT id FROM organizations WHERE tenant_id = $1) \
         AND NOT EXISTS (SELECT 1 FROM tenant_menus tm WHERE tm.tenant_id = $1 AND tm.menu_id = om.menu_id)",
    )
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM role_menus rm USING roles r WHERE rm.role_id = r.id AND r.tenant_id = $1 \
         AND NOT EXISTS (SELECT 1 FROM organization_menus om WHERE om.organization_id = r.organization_id AND om.menu_id = rm.menu_id)",
    )
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    state.invalidate_perm_cache();

    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(tenant_id), organization_id: Some(user.org_id),
        action: "TENANT_AUTH",
        detail: &format!("租户授权变更: 应用 {} 项, 菜单 {} 项 (含级联清理)", body.applications.len(), body.menus.len()),
        ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(json!({"ok": true})))
}
