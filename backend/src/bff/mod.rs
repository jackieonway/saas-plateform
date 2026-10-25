//! Platform OIDC relying party. Browser cookies and upstream tokens never
//! replace the exact platform membership proof required by business APIs.

mod context;
mod crypto;
mod protocol;
mod routes;
mod store;
mod transport;

pub(crate) use crypto::CipherKeys;
pub(crate) use routes::routes;
pub(crate) use transport::{configured as configured_transport, OidcTransport};
#[cfg(test)]
pub(crate) use transport::{Endpoint, OidcFuture, OidcRequest, OidcResponse};

use crate::{error::AppError, state::AppState};
use axum::http::{header, HeaderMap};
use uuid::Uuid;

const LOGIN_COOKIE: &str = "__Host-saas_console_login";
const SESSION_COOKIE: &str = "__Host-saas_console";

fn enabled(state: &AppState) -> Result<(), AppError> {
    if !state.config.bff_enabled {
        return Err(AppError::not_found("平台 BFF 未启用"));
    }
    Ok(())
}
fn secret() -> Result<String, AppError> {
    crate::sso::opaque_secret()
}
fn digest(value: &str) -> String {
    crate::util::hash::sha256_hex(value)
}
fn csrf(cookie: &str) -> String {
    digest(&format!("platform-bff-csrf/1:{cookie}"))
}
fn equal(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0_u8, |difference, (l, r)| difference | (l ^ r))
            == 0
}
fn canonical_secret(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn unique<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>, AppError> {
    let mut values = headers.get_all(name).iter();
    let value = values.next();
    if values.next().is_some() {
        return Err(AppError::forbidden("平台认证请求头重复"));
    }
    value
        .map(|value| {
            value
                .to_str()
                .map_err(|_| AppError::forbidden("平台认证请求头无效"))
        })
        .transpose()
}
fn origin(state: &AppState, headers: &HeaderMap, read: bool) -> Result<(), AppError> {
    enabled(state)?;
    if headers.contains_key(header::AUTHORIZATION) {
        return Err(AppError::unauthorized("此入口只接受平台 BFF Cookie"));
    }
    let expected = &state.config.public_app_url;
    match unique(headers, "origin")? {
        Some(origin) if origin == expected => Ok(()),
        None if read
            && unique(headers, "sec-fetch-site")? == Some("same-origin")
            && unique(headers, "host")?.is_some_and(|host| {
                host.eq_ignore_ascii_case(expected.strip_prefix("https://").unwrap_or(""))
            }) =>
        {
            Ok(())
        }
        _ => Err(AppError::forbidden("平台认证请求来源无效")),
    }
}
fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>, AppError> {
    let mut found = None;
    for header in headers.get_all(header::COOKIE).iter() {
        let header = header
            .to_str()
            .map_err(|_| AppError::unauthorized("平台认证 Cookie 无效"))?;
        for pair in header.split(';') {
            let (key, value) = pair
                .trim()
                .split_once('=')
                .ok_or_else(|| AppError::unauthorized("平台认证 Cookie 无效"))?;
            if key == name {
                if found.is_some() || !canonical_secret(value) {
                    return Err(AppError::unauthorized("平台认证 Cookie 无效"));
                }
                found = Some(value);
            }
        }
    }
    Ok(found)
}
fn cookie(name: &str, value: &str, ttl: i64) -> String {
    format!(
        "{name}={value}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={}",
        ttl.max(0)
    )
}

/// Used only before caller-owned database transactions. Existing legacy
/// sessions incur no network request; BFF-derived business credentials require
/// a fresh, exact upstream account proof and never refresh on this path.
pub(crate) async fn require_platform_session(state: &AppState, sid: Uuid) -> Result<(), AppError> {
    let bff: Option<Option<Uuid>> =
        sqlx::query_scalar("SELECT bff_session_id FROM platform_sessions WHERE id=$1")
            .bind(sid)
            .fetch_optional(&state.pool)
            .await
            .map_err(|_| AppError::Unavailable)?;
    let Some(Some(id)) = bff else {
        return Ok(());
    };
    if !state.config.bff_enabled {
        return Err(AppError::Unavailable);
    }
    let session = store::session_id(&state.pool, id, false)
        .await?
        .ok_or_else(|| AppError::unauthorized("平台认证已失效"))?;
    let chain = transport::Chain::new(state)?;
    let checked = store::online(&chain, session, false).await?;
    if checked.selected_session_id != Some(sid) {
        return Err(AppError::unauthorized("平台组织上下文已切换"));
    }
    let current:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM platform_sessions s JOIN bff_sessions b ON b.id=s.bff_session_id WHERE s.id=$1 AND b.id=$2 AND b.version=$3 AND b.selected_session_id=s.id AND s.bff_context_version=$3 AND platform_session_active(s.id))")
        .bind(sid).bind(checked.id).bind(checked.version).fetch_one(&state.pool).await.map_err(|_|AppError::Unavailable)?;
    if !current {
        return Err(AppError::unauthorized("平台组织上下文已在权威校验期间变化"));
    }
    Ok(())
}
