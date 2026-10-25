//! OIDC account authentication; tokens never establish platform membership.

mod account_logout;
mod clients;
mod customer_portal;
mod interaction;
mod keys;
mod protocol;
mod remote_admin;
mod store;
mod tokens;

pub(crate) use customer_portal::configured as customer_portal_authority;
pub(crate) use keys::KeySet;
pub(crate) use protocol::routes;
pub(crate) use remote_admin::routes as remote_admin_routes;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::{error::AppError, state::AppState};

type Result<T> = std::result::Result<T, OidcError>;

enum OidcError {
    Protocol(&'static str, &'static str),
    InvalidClient,
    InvalidToken,
    App(AppError),
}

impl From<AppError> for OidcError {
    fn from(error: AppError) -> Self {
        Self::App(error)
    }
}

impl From<sqlx::Error> for OidcError {
    fn from(error: sqlx::Error) -> Self {
        Self::App(error.into())
    }
}

impl IntoResponse for OidcError {
    fn into_response(self) -> Response {
        match self {
            Self::InvalidClient => (StatusCode::UNAUTHORIZED,
                [("www-authenticate", "Basic realm=\"oidc\"")],
                Json(json!({"error":"invalid_client","error_description":"Client authentication failed"}))).into_response(),
            Self::InvalidToken => (StatusCode::UNAUTHORIZED,
                [("www-authenticate", "Bearer error=\"invalid_token\"")],
                Json(json!({"error":"invalid_token","error_description":"The access token is invalid or expired"}))).into_response(),
            Self::Protocol(code, description) => (StatusCode::BAD_REQUEST,
                Json(json!({"error":code,"error_description":description}))).into_response(),
            Self::App(AppError::Unavailable) => (StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error":"temporarily_unavailable","error_description":"Authentication authority unavailable"}))).into_response(),
            Self::App(AppError::RateLimited(retry)) => (StatusCode::TOO_MANY_REQUESTS,
                [("retry-after",retry.to_string())],
                Json(json!({"error":"temporarily_unavailable","error_description":"Request rate limit exceeded"}))).into_response(),
            Self::App(error @ AppError::Server(_)) => {
                // Preserve the existing redacted failure telemetry without
                // changing this protocol endpoint to the business JSON wire.
                let response=error.into_response();
                (response.status(),Json(json!({"error":"server_error","error_description":"Authentication request failed"}))).into_response()
            }
            Self::App(error) => error.into_response(),
        }
    }
}

fn invalid() -> OidcError {
    OidcError::Protocol(
        "invalid_request",
        "Invalid or unsupported request parameters",
    )
}

fn invalid_grant() -> OidcError {
    OidcError::Protocol(
        "invalid_grant",
        "The authorization proof is invalid or expired",
    )
}

fn enabled(state: &AppState) -> std::result::Result<&str, AppError> {
    if !state.config.oidc_enabled {
        return Err(AppError::not_found("OIDC 认证未启用"));
    }
    state
        .config
        .oidc_issuer
        .as_deref()
        .ok_or(AppError::Unavailable)
}

/// Pins the public issuer to this environment's database before serving OIDC.
/// Changing the hosting process must not change existing subjects or issuer.
pub(crate) async fn initialize(state: &AppState) -> std::result::Result<(), AppError> {
    if !state.config.oidc_enabled {
        return Ok(());
    }
    let issuer = enabled(state)?;
    let pinned: Option<String> =
        sqlx::query_scalar("SELECT issuer FROM oidc_provider_settings WHERE singleton")
            .fetch_optional(&state.pool)
            .await
            .map_err(|_| AppError::Unavailable)?;
    let pinned = match pinned {
        Some(pinned) => pinned,
        None => {
            sqlx::query("INSERT INTO oidc_provider_settings(singleton,issuer,created_at) VALUES(true,$1,clock_timestamp()) ON CONFLICT(singleton) DO NOTHING")
                .bind(issuer).execute(&state.pool).await.map_err(|_|AppError::Unavailable)?;
            sqlx::query_scalar::<_, String>(
                "SELECT issuer FROM oidc_provider_settings WHERE singleton",
            )
            .fetch_one(&state.pool)
            .await
            .map_err(|_| AppError::Unavailable)?
        }
    };
    if pinned != issuer {
        return Err(AppError::Unavailable);
    }
    Ok(())
}

async fn authority(state: &AppState) -> std::result::Result<&str, AppError> {
    initialize(state).await?;
    enabled(state)
}

fn secret() -> std::result::Result<String, AppError> {
    crate::sso::opaque_secret()
}
fn digest(value: &str) -> String {
    crate::util::hash::sha256_hex(value)
}

fn equal(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
}

fn canonical_secret(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

async fn deadline(
    connection: &mut sqlx::PgConnection,
    expires: chrono::DateTime<chrono::Utc>,
) -> Result<()> {
    let valid: bool = sqlx::query_scalar("SELECT $1::timestamptz > clock_timestamp()")
        .bind(expires)
        .fetch_one(connection)
        .await
        .map_err(|_| AppError::Unavailable)?;
    if !valid {
        return Err(invalid_grant());
    }
    Ok(())
}
