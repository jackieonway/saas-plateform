use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// 统一业务错误: {"code":"UNAUTHORIZED|FORBIDDEN|NOT_FOUND|BAD_REQUEST|CONFLICT|TOO_MANY_REQUESTS|SERVER_ERROR","message":"中文消息"}
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Conflict(String),
    /// 429 限流, 携带 Retry-After 秒数
    #[error("请求过于频繁，请稍后再试")]
    RateLimited(u64),
    #[error(transparent)]
    Server(#[from] anyhow::Error),
}

impl AppError {
    pub fn bad_request(msg: impl Into<String>) -> Self {
        AppError::BadRequest(msg.into())
    }
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        AppError::Unauthorized(msg.into())
    }
    pub fn forbidden(msg: impl Into<String>) -> Self {
        AppError::Forbidden(msg.into())
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        AppError::NotFound(msg.into())
    }
    pub fn conflict(msg: impl Into<String>) -> Self {
        AppError::Conflict(msg.into())
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        if let sqlx::Error::Database(ref db) = e {
            if db.code().as_deref() == Some("23505") {
                return AppError::Conflict("数据已存在, 请检查编码/名称等唯一性字段".into());
            }
            if db.code().as_deref() == Some("23503") {
                return AppError::BadRequest("存在关联数据, 无法完成该操作".into());
            }
        }
        AppError::Server(anyhow::Error::new(e))
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        // 429 限流需附带 Retry-After 头, 单独构造
        if let AppError::RateLimited(retry_after) = &self {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [(header::RETRY_AFTER, retry_after.to_string())],
                Json(json!({"code": "TOO_MANY_REQUESTS", "message": "请求过于频繁，请稍后再试"})),
            )
                .into_response();
        }
        let (status, code) = match &self {
            AppError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED"),
            AppError::Forbidden(_) => (StatusCode::FORBIDDEN, "FORBIDDEN"),
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, "NOT_FOUND"),
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, "BAD_REQUEST"),
            AppError::Conflict(_) => (StatusCode::CONFLICT, "CONFLICT"),
            AppError::RateLimited(_) => (StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS"),
            AppError::Server(_) => (StatusCode::INTERNAL_SERVER_ERROR, "SERVER_ERROR"),
        };
        let message = match &self {
            AppError::Server(_) => {
                tracing::error!(event = "server_error", code = "SERVER_ERROR",
                    request_id = ?crate::observability::request_id());
                "服务器内部错误，请凭请求编号联系管理员".to_string()
            }
            other => other.to_string(),
        };
        (status, Json(json!({"code": code, "message": message}))).into_response()
    }
}

#[allow(dead_code)]
pub type ApiResult<T> = Result<T, AppError>;
