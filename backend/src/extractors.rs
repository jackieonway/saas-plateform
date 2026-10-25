use crate::util::uuid7::ParseUuid7;
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap};
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::error::AppError;
use crate::session;
use crate::state::AppState;
use crate::util::jwt;

/// 当前员工会话用户 (解析 Authorization: Bearer <session JWT>)。
/// 成员及策略版本来自服务器会话记录；JWT 的 user/org 不构成员工资格。
#[derive(Debug, Clone)]
#[allow(dead_code)] // real_name 保留供扩展
pub struct CurrentUser {
    pub user_id: Uuid,
    pub username: String,
    pub real_name: String,
    pub is_super_admin: bool,
    pub tenant_id: Uuid,
    pub org_id: Uuid,
    pub session_id: Uuid,
}

/// 从 Header 取 Bearer Token
pub fn bearer_token(headers: &HeaderMap) -> Result<String, AppError> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::unauthorized("未登录或缺少凭证"))
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(&parts.headers)?;
        let claims = jwt::verify(&token, &state.config.jwt_secret)
            .map_err(|_| AppError::unauthorized("凭证无效或已过期"))?;
        if claims.typ != "session" {
            return Err(AppError::unauthorized("凭证类型错误, 需要会话令牌"));
        }
        let user_id: Uuid = claims
            .sub
            .parse_uuid7()
            .map_err(|_| AppError::unauthorized("凭证无效"))?;
        let tenant_id: Uuid = claims
            .tenant
            .as_deref()
            .and_then(|t| crate::util::uuid7::parse_uuid(t).ok())
            .ok_or_else(|| AppError::unauthorized("凭证缺少租户上下文"))?;
        let org_id: Uuid = claims
            .org
            .as_deref()
            .and_then(|o| crate::util::uuid7::parse_uuid(o).ok())
            .ok_or_else(|| AppError::unauthorized("凭证缺少组织上下文"))?;
        let session_id: Uuid = claims
            .sid
            .as_deref()
            .and_then(|sid| crate::util::uuid7::parse_uuid(sid).ok())
            .ok_or_else(|| AppError::unauthorized("旧版会话已失效，请重新登录"))?;
        let version = claims
            .ver
            .ok_or_else(|| AppError::unauthorized("旧版会话已失效，请重新登录"))?;
        let (username, real_name, is_super_admin) =
            session::active_user(&state.pool, user_id, tenant_id, org_id, session_id, version)
                .await?;
        crate::bff::require_platform_session(state, session_id).await?;
        Ok(Self {
            user_id,
            username,
            real_name,
            is_super_admin,
            tenant_id,
            org_id,
            session_id,
        })
    }
}

/// 权限校验标记: 在处理器内调用 `state.require(&user, perm).check(&state, &user).await`
///
/// 用法: `RequirePermission("system:tenant").check(&state, &user).await?;`
pub struct RequirePermission(pub &'static str);

impl RequirePermission {
    pub async fn check(self, state: &AppState, user: &CurrentUser) -> Result<(), AppError> {
        state.require(user, self.0).await
    }
}

/// JSON 请求体提取器 (解析失败返回统一 JSON 错误)
pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let axum::Json(value) = axum::Json::<T>::from_request(req, state)
            .await
            .map_err(|e| AppError::bad_request(format!("请求体解析失败: {}", e)))?;
        Ok(ApiJson(value))
    }
}
