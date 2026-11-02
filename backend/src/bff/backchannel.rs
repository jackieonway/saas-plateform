//! A signed logout is its own authority; an already revoked access token is
//! never required to prove the notification that revoked it.
use super::{protocol, store, transport::Chain};
use crate::{
    error::AppError,
    state::AppState,
    util::{audit, uuid7},
};
use axum::{
    extract::{rejection::FormRejection, Form, RawQuery, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LogoutForm {
    logout_token: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    alg: String,
    kid: String,
    typ: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Events {
    #[serde(rename = "http://schemas.openid.net/event/backchannel-logout")]
    _logout: Empty,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Claims {
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    jti: String,
    sid: String,
    sub: String,
    events: Events,
}

fn bad() -> AppError {
    AppError::bad_request("登出通知无效")
}

fn strict_header(token: &str) -> Result<(), AppError> {
    if token.is_empty() || token.len() > 8192 {
        return Err(bad());
    }
    let mut parts = token.split('.');
    let encoded = parts.next().ok_or_else(bad)?;
    if parts.next().is_none() || parts.next().is_none() || parts.next().is_some() {
        return Err(bad());
    }
    let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| bad())?;
    let header: Header = serde_json::from_slice(&bytes).map_err(|_| bad())?;
    if header.alg != "RS256"
        || header.typ != "logout+jwt"
        || header.kid.is_empty()
        || header.kid.len() > 80
        || !header
            .kid
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(bad());
    }
    Ok(())
}

fn validate(
    token: &str,
    key: &DecodingKey,
    issuer: &str,
    client: &str,
    now: i64,
) -> Result<Claims, AppError> {
    strict_header(token)?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.leeway = 0;
    validation.set_issuer(&[issuer]);
    validation.set_audience(&[client]);
    validation.set_required_spec_claims(&["iss", "aud", "iat", "exp", "jti", "sid", "sub"]);
    // Typed decoding rejects duplicate and unknown claims before any SQL.
    let claims = jsonwebtoken::decode::<Claims>(token, key, &validation)
        .map_err(|_| bad())?
        .claims;
    if claims.iss != issuer
        || claims.aud != client
        || !super::canonical_secret(&claims.sid)
        || claims.sub.is_empty()
        || claims.sub.len() > 256
        || !claims.sub.bytes().all(|byte| byte.is_ascii_graphic())
        || !uuid7::parse_uuid(&claims.jti).is_ok_and(|id| id.to_string() == claims.jti)
        || claims.iat <= 0
        || claims.iat > now.saturating_add(5)
        || claims.iat < now.saturating_sub(120)
        || claims.exp != claims.iat.saturating_add(120)
        || claims.exp <= now
    {
        return Err(bad());
    }
    let _ = &claims.events;
    Ok(claims)
}

pub(crate) async fn receive(state: &AppState, token: &str) -> Result<bool, AppError> {
    strict_header(token)?;
    let chain = Chain::new(state)?;
    let key = protocol::verification_key(&chain, token)
        .await
        .map_err(|error| {
            if matches!(error, AppError::Unavailable) {
                error
            } else {
                bad()
            }
        })?;
    let claims = validate(
        token,
        &key,
        store::issuer(state)?,
        &store::client(state)?.to_string(),
        Utc::now().timestamp(),
    )?;
    apply(state, &claims).await
}

async fn lock_source(
    connection: &mut sqlx::PgConnection,
    issuer: &str,
    client: Uuid,
    sid: &str,
) -> Result<(), AppError> {
    let key = serde_json::to_string(&["oidc-rp-source/1", issuer, &client.to_string(), sid])
        .map_err(|_| AppError::Unavailable)?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(key)
        .execute(connection)
        .await?;
    Ok(())
}

/// The lock must precede account/browser/session locks and remain held until
/// commit. It serializes even an absent session with an earlier notification.
pub(super) async fn reject_revoked(
    connection: &mut sqlx::PgConnection,
    issuer: &str,
    client: Uuid,
    sid: &str,
) -> Result<(), AppError> {
    lock_source(connection, issuer, client, sid).await?;
    let revoked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM platform_backchannel_logout_sources WHERE issuer=$1 AND client_id=$2 AND sid_digest=$3)")
        .bind(issuer).bind(client).bind(super::digest(sid)).fetch_one(connection).await?;
    if revoked {
        return Err(AppError::unauthorized("原认证会话已收到登出通知"));
    }
    Ok(())
}

async fn notification_deadline(
    connection: &mut sqlx::PgConnection,
    expires: DateTime<Utc>,
) -> Result<(), AppError> {
    let valid: bool = sqlx::query_scalar("SELECT $1::timestamptz>clock_timestamp()")
        .bind(expires)
        .fetch_one(connection)
        .await?;
    if !valid {
        return Err(bad());
    }
    Ok(())
}

async fn apply(state: &AppState, claims: &Claims) -> Result<bool, AppError> {
    let client = store::client(state)?;
    let issued = DateTime::from_timestamp(claims.iat, 0).ok_or_else(bad)?;
    let expires = DateTime::from_timestamp(claims.exp, 0).ok_or_else(bad)?;
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| AppError::Unavailable)?;
    notification_deadline(&mut tx, expires).await?;
    lock_source(&mut tx, &claims.iss, client, &claims.sid).await?;
    notification_deadline(&mut tx, expires).await?;
    let wrong_subject: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM bff_sessions WHERE issuer=$1 AND oidc_client_id=$2 AND oidc_public_sid=$3 AND oidc_public_sub<>$4)")
        .bind(&claims.iss).bind(client).bind(&claims.sid).bind(&claims.sub).fetch_one(&mut *tx).await?;
    if wrong_subject {
        return Err(bad());
    }
    let sid_digest = super::digest(&claims.sid);
    let sub_digest = super::digest(&claims.sub);
    sqlx::query("INSERT INTO platform_backchannel_logout_sources(id,issuer,client_id,sid_digest,sub_digest) VALUES($1,$2,$3,$4,$5) ON CONFLICT(issuer,client_id,sid_digest) DO NOTHING")
        .bind(uuid7::new_uuid()).bind(&claims.iss).bind(client).bind(&sid_digest).bind(&sub_digest)
        .execute(&mut *tx).await?;
    let original_sub: String = sqlx::query_scalar("SELECT sub_digest FROM platform_backchannel_logout_sources WHERE issuer=$1 AND client_id=$2 AND sid_digest=$3")
        .bind(&claims.iss).bind(client).bind(&sid_digest).fetch_one(&mut *tx).await?;
    if original_sub != sub_digest {
        return Err(bad());
    }
    let inserted: Option<Uuid> = sqlx::query_scalar("INSERT INTO platform_backchannel_logout_receipts(id,issuer,client_id,jti_digest,issued_at,expires_at) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(issuer,client_id,jti_digest) DO NOTHING RETURNING id")
        .bind(uuid7::new_uuid()).bind(&claims.iss).bind(client).bind(super::digest(&claims.jti))
        .bind(issued).bind(expires).fetch_optional(&mut *tx).await?;
    if inserted.is_none() {
        return Ok(false);
    }

    // Normal callback/logout takes account -> browser -> session locks. Match
    // that order, without relying on the now-invalid canonical access proof.
    let users: Vec<Uuid> = sqlx::query_scalar("SELECT DISTINCT user_id FROM bff_sessions WHERE issuer=$1 AND oidc_client_id=$2 AND oidc_public_sid=$3 AND oidc_public_sub=$4 ORDER BY user_id")
        .bind(&claims.iss).bind(client).bind(&claims.sid).bind(&claims.sub).fetch_all(&mut *tx).await?;
    for user in users {
        store::lock_user(&mut tx, user).await?;
    }
    sqlx::query("SELECT b.id FROM console_browsers b WHERE EXISTS(SELECT 1 FROM bff_sessions s WHERE s.browser_id=b.id AND s.issuer=$1 AND s.oidc_client_id=$2 AND s.oidc_public_sid=$3 AND s.oidc_public_sub=$4) ORDER BY b.id FOR UPDATE")
        .bind(&claims.iss).bind(client).bind(&claims.sid).bind(&claims.sub).fetch_all(&mut *tx).await?;
    let sessions: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM bff_sessions WHERE issuer=$1 AND oidc_client_id=$2 AND oidc_public_sid=$3 AND oidc_public_sub=$4 ORDER BY id FOR UPDATE")
        .bind(&claims.iss).bind(client).bind(&claims.sid).bind(&claims.sub).fetch_all(&mut *tx).await?;
    for session in &sessions {
        store::revoke(&mut tx, *session).await?;
    }
    // A delayed old notification cannot clear a newer source's browser pointer.
    sqlx::query("UPDATE console_browsers b SET generation=generation+1,current_session_id=NULL WHERE EXISTS(SELECT 1 FROM bff_sessions s WHERE s.id=b.current_session_id AND s.issuer=$1 AND s.oidc_client_id=$2 AND s.oidc_public_sid=$3 AND s.oidc_public_sub=$4)")
        .bind(&claims.iss).bind(client).bind(&claims.sid).bind(&claims.sub).execute(&mut *tx).await?;
    audit::write_in_tx(
        &mut tx,
        audit::AuditInfo {
            user_id: None,
            username: "identity",
            tenant_id: None,
            organization_id: None,
            action: "BFF_BACKCHANNEL_LOGOUT",
            detail:
                &json!({"jtiDigest":super::digest(&claims.jti),"matchedSessions":sessions.len()})
                    .to_string(),
            ip: "",
            user_agent: "",
        },
    )
    .await?;
    notification_deadline(&mut tx, expires).await?;
    tx.commit().await.map_err(|_| AppError::Unavailable)?;
    Ok(true)
}

pub(super) async fn endpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    form: Result<Form<LogoutForm>, FormRejection>,
) -> Result<Response, AppError> {
    if headers.contains_key(header::AUTHORIZATION)
        || !super::unique(&headers, "host")?.is_some_and(|host| {
            state
                .config
                .public_app_url
                .strip_prefix("https://")
                .is_some_and(|expected| expected.eq_ignore_ascii_case(host))
        })
        || query.is_some()
    {
        return Err(bad());
    }
    let Form(form) = form.map_err(|_| bad())?;
    if receive(&state, &form.logout_token).await? {
        Ok(StatusCode::OK.into_response())
    } else {
        Ok((
            StatusCode::CONFLICT,
            Json(json!({"code":"BACKCHANNEL_REPLAY","message":"登出通知已经处理"})),
        )
            .into_response())
    }
}

#[cfg(test)]
pub(crate) fn valid_fixture(
    token: &str,
    key: &DecodingKey,
    issuer: &str,
    client: &str,
    now: i64,
) -> bool {
    validate(token, key, issuer, client, now).is_ok()
}
