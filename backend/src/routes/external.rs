//! 兼容旧路径的当前用户视图。完整配置只能使用平台管理 API。

use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::error::AppError;
use crate::extractors::bearer_token;
use crate::routes::app_context;
use crate::state::AppState;
use crate::util::jwt;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/external/menus", get(menus))
        .route("/api/external/roles", get(roles))
}

async fn user_context(state: &AppState, headers: &HeaderMap) -> Result<Value, AppError> {
    let token = bearer_token(headers)?;
    let claims = jwt::verify(&token, &state.config.jwt_secret)
        .map_err(|_| AppError::unauthorized("应用令牌无效或已过期"))?;
    let audience = claims
        .aud
        .as_deref()
        .ok_or_else(|| AppError::forbidden("旧服务令牌不可读取用户授权视图"))?;
    app_context::current_context(state, audience, headers).await
}

/// 旧路径仅返回当前用户的应用菜单，保留原有 application/client/menus 包装形状。
async fn menus(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, AppError> {
    let ctx = user_context(&state, &headers).await?;
    Ok(Json(json!({
        "application": {"id": ctx["application"]["id"], "code": ctx["appCode"], "name": ctx["application"]["name"]},
        "client": {"tokenType": "user", "username": ctx["user"]["username"]},
        "menus": ctx["menus"],
    })))
}

/// 旧路径仅返回当前组织的当前用户角色，无法枚举其他组织的角色配置。
async fn roles(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, AppError> {
    let ctx = user_context(&state, &headers).await?;
    Ok(Json(json!({
        "application": {"id": ctx["application"]["id"], "code": ctx["appCode"], "name": ctx["application"]["name"]},
        "organizations": [{
            "id": ctx["organization"]["id"], "name": ctx["organization"]["name"],
            "tenantId": ctx["tenant"]["id"], "tenantName": ctx["tenant"]["name"],
            "roles": ctx["roles"],
        }],
    })))
}
