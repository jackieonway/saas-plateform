//! Independent private resource-service authentication, never an OAuth registry.
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
    if headers.contains_key(header::COOKIE)
        || headers.get_all(header::AUTHORIZATION).iter().count() != 1
    {
        return None;
    }
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
    pub(crate) id: Uuid,
    pub(crate) application_id: Uuid,
    pub(crate) client_id: String,
    pub(crate) credential_version: i64,
    pub(crate) purpose: String,
    pub(crate) audience: String,
    pub(crate) allowed_member_types: Vec<String>,
    pub(crate) allowed_operations: Vec<String>,
    secret_hash: String,
}

pub(crate) async fn recheck(state: &AppState, headers: &HeaderMap) -> Result<Caller, AppError> {
    recheck_for(state, headers, "APP_RESOURCE").await
}

pub(crate) async fn recheck_for(
    state: &AppState,
    headers: &HeaderMap,
    purpose: &str,
) -> Result<Caller, AppError> {
    let (id, secret) =
        parse_basic_auth(headers).ok_or_else(|| AppError::unauthorized("机密客户端凭据无效"))?;
    if id.is_empty() || id.len() > 100 || secret.is_empty() || secret.len() > 1024 {
        return Err(AppError::unauthorized("机密客户端凭据无效"));
    }
    #[derive(sqlx::FromRow)]
    struct Service {
        id: Uuid,
        application_id: Uuid,
        secret_hash: String,
        credential_version: i64,
        purpose: String,
        audience: String,
        allowed_member_types: Vec<String>,
        allowed_operations: Vec<String>,
    }
    let row:Option<Service>=sqlx::query_as("SELECT s.id,s.application_id,s.secret_hash,s.credential_version,s.purpose,s.audience,s.allowed_member_types,s.allowed_operations FROM platform_resource_services s JOIN applications a ON a.id=s.application_id AND a.status='ENABLED' AND s.audience=a.code WHERE s.service_id=$1 AND s.status='ENABLED' AND s.purpose=$2 AND NOT identity_credential_digest_in_use(s.secret_hash) AND NOT EXISTS(SELECT 1 FROM context_selection_services context_service WHERE context_service.secret_hash=s.secret_hash)")
        .bind(&id).bind(purpose).fetch_optional(&state.pool).await.map_err(|_|AppError::Unavailable)?;
    let Some(row) = row else {
        return Err(AppError::unauthorized("机密客户端凭据无效"));
    };
    let supplied = hash::sha256_hex(&secret);
    if supplied.len() != row.secret_hash.len()
        || supplied
            .bytes()
            .zip(row.secret_hash.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            != 0
    {
        return Err(AppError::unauthorized("机密客户端凭据无效"));
    }
    Ok(Caller {
        id: row.id,
        application_id: row.application_id,
        client_id: id,
        credential_version: row.credential_version,
        purpose: row.purpose,
        audience: row.audience,
        allowed_member_types: row.allowed_member_types,
        allowed_operations: row.allowed_operations,
        secret_hash: row.secret_hash,
    })
}

pub(crate) async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Caller, AppError> {
    authenticate_for(state, headers, "APP_RESOURCE").await
}

pub(crate) async fn authenticate_for(
    state: &AppState,
    headers: &HeaderMap,
    purpose: &str,
) -> Result<Caller, AppError> {
    let ip = audit::client_ip(headers);
    if let Some(retry) = state.record_introspection_attempt(&ip) {
        return Err(AppError::RateLimited(retry as u64));
    }
    let permit = state
        .introspection_auth_slots
        .try_acquire()
        .map_err(|_| AppError::RateLimited(1))?;
    let result = recheck_for(state, headers, purpose).await;
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
