//! 对外 API (第三方服务数据拉取)
//!
//! 面向第三方 OAuth 客户端: 获取 access token 后 (client_credentials 或用户令牌均可),
//! 拉取其「关联应用」(oauth_clients.application_id) 的菜单树与角色授权数据。
//!
//! 鉴权与 /oauth/userinfo 同口径 (Bearer → jwt::verify → oauth_tokens 查库校验),
//! 但允许 client_credentials 令牌 (oauth_tokens.user_id 为 NULL)。

use std::collections::HashMap;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::bearer_token;
use crate::routes::oauth_admin::fetch_client;
use crate::state::AppState;
use crate::util::{hash, jwt, tree};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/external/menus", get(menus))
        .route("/api/external/roles", get(roles))
}

/// 已通过对外 API 鉴权的调用上下文
struct ExternalContext {
    client_id: String,
    application_id: Uuid,
    /// 令牌关联用户名 (client_credentials 令牌为 None → JSON 输出 null)
    username: Option<String>,
    /// true = client_credentials 令牌 (无用户上下文)
    is_client_credentials: bool,
}

/// 对外 API 统一鉴权:
/// 1. Bearer 取 token → 2. jwt::verify → 3. typ 必须 access →
/// 4. oauth_tokens 行校验 (revoked / expires_at; 允许 user_id NULL 即 client_credentials) →
/// 5. claims.cid 查 oauth_clients (状态 DISABLED → 403; 未关联应用 → 404)
async fn authenticate_external(state: &AppState, headers: &HeaderMap) -> Result<ExternalContext, AppError> {
    let token = bearer_token(headers).map_err(|_| AppError::unauthorized("缺少访问令牌"))?;
    let claims = jwt::verify(&token, &state.config.jwt_secret)
        .map_err(|_| AppError::unauthorized("令牌无效或已过期"))?;
    if claims.typ != "access" {
        return Err(AppError::unauthorized("令牌类型错误"));
    }
    // 与 userinfo 同口径查库校验 (token_hash 唯一); user_id 可为 NULL (client_credentials)
    let row = sqlx::query_as::<_, (Option<Uuid>, Option<String>, bool, DateTime<Utc>)>(
        "SELECT t.user_id, u.username, t.revoked, t.expires_at \
         FROM oauth_tokens t LEFT JOIN users u ON u.id = t.user_id \
         WHERE t.token_hash = $1 AND t.token_type = 'access'",
    )
    .bind(hash::sha256_hex(&token))
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::unauthorized("令牌无效或已过期"))?;
    let (user_id, username, revoked, expires_at) = row;
    if revoked || expires_at < Utc::now() {
        return Err(AppError::unauthorized("令牌已被吊销或已过期"));
    }
    let cid = claims
        .cid
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::unauthorized("令牌缺少客户端标识"))?;
    let client = fetch_client(&state.pool, cid)
        .await?
        .ok_or_else(|| AppError::unauthorized("客户端不存在"))?;
    if client.status != "ENABLED" {
        return Err(AppError::forbidden("客户端已禁用"));
    }
    let application_id = client
        .application_id
        .ok_or_else(|| AppError::not_found("该客户端未关联应用, 请在 OAuth 客户端管理中配置"))?;
    Ok(ExternalContext {
        client_id: client.client_id,
        application_id,
        username,
        is_client_credentials: user_id.is_none(),
    })
}

/// 应用标识段 {"id","code","name"} (两端点共用)
async fn application_json(state: &AppState, application_id: Uuid) -> Result<Value, AppError> {
    let (code, name): (String, String) =
        sqlx::query_as("SELECT code, name FROM applications WHERE id = $1")
            .bind(application_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| AppError::not_found("应用不存在"))?;
    Ok(json!({"id": application_id, "code": code, "name": name}))
}

/// 菜单行 (列序与 MENU_SELECT 风格一致: …, permission, sort_order, visible, link_url, link_target)
type MenuRowTuple = (
    Uuid,
    Uuid,
    Option<Uuid>,
    String,
    String,
    String,
    String,
    String,
    String,
    i32,
    bool,
    String,
    String,
);

/// 菜单 JSON (与 /api/menus/tree 的 menu_json 同构, 经 build_tree 组树)
fn menu_json(r: &MenuRowTuple) -> Value {
    let (id, application_id, parent_id, mtype, title, path, icon, component, permission, sort_order, visible, link_url, link_target) =
        r;
    json!({
        "id": id, "applicationId": application_id, "parentId": parent_id, "type": mtype,
        "title": title, "path": path, "icon": icon, "component": component,
        "permission": permission, "sortOrder": sort_order, "visible": visible,
        "linkUrl": link_url, "linkTarget": link_target,
    })
}

/// GET /api/external/menus — 当前客户端关联应用的完整菜单树 (仅 status='ENABLED', 不暴露禁用项)
async fn menus(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, AppError> {
    let ctx = authenticate_external(&state, &headers).await?;
    let app = application_json(&state, ctx.application_id).await?;
    let rows: Vec<MenuRowTuple> = sqlx::query_as(
        "SELECT m.id, m.application_id, m.parent_id, m.type, m.title, COALESCE(m.path,'') AS path, \
         COALESCE(m.icon,'') AS icon, COALESCE(m.component,'') AS component, COALESCE(m.permission,'') AS permission, \
         m.sort_order, m.visible, m.link_url, m.link_target \
         FROM menus m WHERE m.application_id = $1 AND m.status = 'ENABLED' \
         ORDER BY m.sort_order, m.created_at",
    )
    .bind(ctx.application_id)
    .fetch_all(&state.pool)
    .await?;
    let items: Vec<Value> = rows.iter().map(menu_json).collect();
    Ok(Json(json!({
        "application": app,
        "client": {
            "clientId": ctx.client_id,
            "tokenType": if ctx.is_client_credentials { "client_credentials" } else { "user" },
            "username": ctx.username,
        },
        "menus": tree::build_tree(items),
    })))
}

/// GET /api/external/roles — 本应用被授权组织 (organization_applications) 的角色清单与菜单授权概览
///
/// 组织过期过滤: organizations.expires_at / tenants.expires_at 任一到期即排除 (Task 18 语义);
/// 角色仅取 status='ENABLED'; menuCount / menuIds 为该角色在本应用菜单 (menus.status='ENABLED')
/// 上的授权 (role_menus JOIN menus ON menus.application_id = 本应用); 无授权数据返回空数组。
async fn roles(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, AppError> {
    let ctx = authenticate_external(&state, &headers).await?;
    let app = application_json(&state, ctx.application_id).await?;
    // 授权了本应用的组织 (JOIN organizations / tenants, 过滤到期组织与到期租户)
    let orgs: Vec<(Uuid, String, Uuid, String)> = sqlx::query_as(
        "SELECT o.id, o.name, o.tenant_id, t.name AS tenant_name \
         FROM organization_applications oa \
         JOIN organizations o ON o.id = oa.organization_id \
         JOIN tenants t ON t.id = o.tenant_id \
         WHERE oa.application_id = $1 \
           AND (o.expires_at IS NULL OR o.expires_at > now()) \
           AND (t.expires_at IS NULL OR t.expires_at > now()) \
         ORDER BY t.name, o.sort_order, o.name",
    )
    .bind(ctx.application_id)
    .fetch_all(&state.pool)
    .await?;
    if orgs.is_empty() {
        return Ok(Json(json!({"application": app, "organizations": []})));
    }
    let org_ids: Vec<Uuid> = orgs.iter().map(|(id, _, _, _)| *id).collect();
    // 各组织角色 (id, organization_id, name, code, built_in)
    let role_rows: Vec<(Uuid, Uuid, String, String, bool)> = sqlx::query_as(
        "SELECT r.id, r.organization_id, r.name, COALESCE(r.code,'') AS code, r.built_in \
         FROM roles r WHERE r.organization_id = ANY($1) AND r.status = 'ENABLED' \
         ORDER BY r.created_at",
    )
    .bind(&org_ids)
    .fetch_all(&state.pool)
    .await?;
    // 角色 → 本应用菜单授权 (role_id, menu_id), 菜单顺序按 sort_order
    let role_ids: Vec<Uuid> = role_rows.iter().map(|(id, _, _, _, _)| *id).collect();
    let menu_grants: Vec<(Uuid, Uuid)> = if role_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as(
            "SELECT rm.role_id, rm.menu_id \
             FROM role_menus rm JOIN menus m ON m.id = rm.menu_id \
             WHERE m.application_id = $1 AND m.status = 'ENABLED' AND rm.role_id = ANY($2) \
             ORDER BY rm.role_id, m.sort_order, m.created_at",
        )
        .bind(ctx.application_id)
        .bind(&role_ids)
        .fetch_all(&state.pool)
        .await?
    };
    let mut menus_of_role: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for (role_id, menu_id) in &menu_grants {
        menus_of_role.entry(*role_id).or_default().push(*menu_id);
    }
    let mut roles_of_org: HashMap<Uuid, Vec<Value>> = HashMap::new();
    for (role_id, org_id, name, code, built_in) in &role_rows {
        let menu_ids = menus_of_role.get(role_id).cloned().unwrap_or_default();
        let menu_count = menu_ids.len();
        roles_of_org.entry(*org_id).or_default().push(json!({
            "id": role_id, "name": name, "code": code, "builtIn": built_in,
            "menuCount": menu_count, "menuIds": menu_ids,
        }));
    }
    let organizations: Vec<Value> = orgs
        .into_iter()
        .map(|(id, name, tenant_id, tenant_name)| {
            let roles = roles_of_org.remove(&id).unwrap_or_default();
            json!({
                "id": id, "name": name, "tenantId": tenant_id, "tenantName": tenant_name,
                "roles": roles,
            })
        })
        .collect();
    Ok(Json(json!({
        "application": app,
        "organizations": organizations,
    })))
}
