//! Minimal confidential-client credential adapter, shared with the OAuth parser.
use crate::{
    error::AppError,
    state::AppState,
    util::{audit, hash},
};
use axum::http::{header, HeaderMap};
use uuid::Uuid;
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < b.len() => match (hex_val(b[i + 1]), hex_val(b[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(h * 16 + l);
                    i += 3;
                }
                _ => {
                    out.push(b[i]);
                    i += 1;
                }
            },
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn parse_basic_auth(headers: &HeaderMap) -> Option<(String, String)> {
    use base64::Engine;
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let b64 = v.strip_prefix("Basic ")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .ok()?;
    let s = String::from_utf8(decoded).ok()?;
    let (u, p) = s.split_once(':')?;
    Some((percent_decode(u), percent_decode(p)))
}

#[derive(PartialEq, Eq)]
pub(crate) struct Caller {
    pub(crate) application_id: Uuid,
    pub(crate) client_id: String,
}

pub(crate) async fn recheck(state: &AppState, headers: &HeaderMap) -> Result<Caller, AppError> {
    let (id, secret) =
        parse_basic_auth(headers).ok_or_else(|| AppError::unauthorized("机密客户端凭据无效"))?;
    if id.is_empty() || id.len() > 100 || secret.is_empty() || secret.len() > 1024 {
        return Err(AppError::unauthorized("机密客户端凭据无效"));
    }
    let row:Option<(String,String,String,Option<Uuid>)>=sqlx::query_as("SELECT COALESCE(client_secret_hash,''),client_type,status,application_id FROM oauth_clients WHERE client_id=$1").bind(&id).fetch_optional(&state.pool).await?;
    let Some((expected, kind, status, Some(application_id))) = row else {
        return Err(AppError::unauthorized("机密客户端凭据无效"));
    };
    if kind != "CONFIDENTIAL" || status != "ENABLED" || hash::sha256_hex(&secret) != expected {
        return Err(AppError::unauthorized("机密客户端凭据无效"));
    }
    Ok(Caller {
        application_id,
        client_id: id,
    })
}

pub(crate) async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Caller, AppError> {
    let ip = audit::client_ip(headers);
    if let Some(retry) = state.record_introspection_attempt(&ip) {
        return Err(AppError::RateLimited(retry as u64));
    }
    let permit = state
        .introspection_auth_slots
        .try_acquire()
        .map_err(|_| AppError::RateLimited(1))?;
    let result = recheck(state, headers).await;
    drop(permit);
    match result {
        Ok(caller) => {
            if let Some(retry) = state.record_introspection_client(&caller.client_id) {
                return Err(AppError::RateLimited(retry as u64));
            }
            Ok(caller)
        }
        Err(error) => {
            if matches!(error, AppError::Unauthorized(_)) {
                crate::observability::record_auth_failure("invalid_client");
                if let Some(retry) = state.record_introspection_failure(&ip) {
                    return Err(AppError::RateLimited(retry as u64));
                }
            }
            Err(error)
        }
    }
}
