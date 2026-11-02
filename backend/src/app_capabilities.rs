//! Internal micro-application capabilities; this is not an OAuth token endpoint.
use crate::{
    error::AppError,
    extractors::{bearer_token, ApiJson, CurrentUser},
    state::AppState,
    util::{audit, hash, jwt},
};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Request {
    app_code: String,
}

#[derive(sqlx::FromRow)]
struct Binding {
    application_id: Uuid,
    application_policy_version: i64,
    membership_id: Uuid,
    member_type: String,
    identity_version: i64,
    account_version: i64,
    origin_issuer: String,
    origin_client_ref: String,
    origin_client_policy_version: i64,
    expires_at: DateTime<Utc>,
}

pub(crate) async fn issue(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(input): ApiJson<Request>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    let source = jwt::verify_internal(
        &bearer_token(&headers)?,
        &state.config.jwt_secret,
        jwt::InternalUse::PlatformSession,
        Some(jwt::PLATFORM_API_AUDIENCE),
    )
    .map_err(|_| AppError::unauthorized("来源平台会话无效"))?;
    if input.app_code.is_empty()
        || input.app_code.len() > 100
        || input.app_code.trim() != input.app_code
    {
        return Err(AppError::bad_request("appCode 无效"));
    }
    let source_end = DateTime::from_timestamp(source.exp, 0)
        .ok_or_else(|| AppError::unauthorized("来源期限无效"))?;
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| AppError::Unavailable)?;
    crate::session::require_active_in_tx(
        &mut tx,
        user.user_id,
        user.tenant_id,
        user.org_id,
        user.session_id,
    )
    .await?;
    crate::session::lock_oidc_source_in_tx(&mut tx, user.session_id, user.user_id).await?;
    // Application policy writes serialize with issuance. Grant deletion is
    // checked again in an independent final statement below.
    sqlx::query("SELECT id FROM applications WHERE code=$1 FOR SHARE")
        .bind(&input.app_code)
        .execute(&mut *tx)
        .await
        .map_err(|_| AppError::Unavailable)?;
    let binding: Binding=sqlx::query_as(
        "SELECT a.id AS application_id,a.access_policy_version AS application_policy_version, \
         s.membership_id,s.member_type,s.identity_version,s.account_version, \
         s.origin_issuer,s.origin_client_ref,s.origin_client_policy_version, \
         LEAST(s.expires_at,org_chain_expires_at(o.id),t.expires_at) AS expires_at \
         FROM platform_sessions s JOIN organizations o ON o.id=s.organization_id \
         JOIN tenants t ON t.id=s.tenant_id JOIN applications a ON a.code=$5 \
         JOIN tenant_applications ta ON ta.tenant_id=s.tenant_id AND ta.application_id=a.id \
         JOIN organization_applications oa ON oa.organization_id=s.organization_id AND oa.application_id=a.id \
         WHERE s.id=$1 AND s.user_id=$2 AND s.tenant_id=$3 AND s.organization_id=$4 \
         AND s.member_type='EMPLOYEE' AND a.status='ENABLED' AND s.member_type=ANY(a.allowed_member_types) \
         AND platform_session_active(s.id)")
        .bind(user.session_id).bind(user.user_id).bind(user.tenant_id).bind(user.org_id).bind(&input.app_code)
        .fetch_optional(&mut *tx).await.map_err(|_|AppError::Unavailable)?
        .ok_or_else(||AppError::forbidden("当前成员未获应用授权"))?;
    let now = Utc::now();
    let end = (now + chrono::Duration::seconds(300))
        .min(binding.expires_at)
        .min(source_end);
    let end = DateTime::from_timestamp(end.timestamp(), 0).ok_or(AppError::Unavailable)?;
    if end <= now {
        return Err(AppError::unauthorized("来源平台会话已过期"));
    }
    let id = crate::util::uuid7::new_uuid();
    let claims = jwt::Claims {
        sub: user.user_id.to_string(),
        typ: "app_session".into(),
        tenant: Some(user.tenant_id.to_string()),
        org: Some(user.org_id.to_string()),
        cid: Some(input.app_code.clone()),
        scope: Some("app:access".into()),
        sid: Some(user.session_id.to_string()),
        ver: Some(binding.account_version),
        aud: Some(input.app_code.clone()),
        jti: Some(id.to_string()),
        exp: end.timestamp(),
        iat: now.timestamp(),
    };
    let token = jwt::sign_internal(
        &claims,
        jwt::InternalUse::AppSession,
        &state.config.jwt_secret,
    )?;
    sqlx::query("INSERT INTO platform_app_capabilities \
        (id,jti,token_hash,user_id,tenant_id,organization_id,application_id,audience,scope,platform_session_id, \
         membership_id,member_type,identity_version,account_version,application_policy_version,origin_issuer, \
         origin_client_ref,origin_client_policy_version,issued_at,expires_at) \
        VALUES($1,$1,$2,$3,$4,$5,$6,$7,'app:access',$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)")
        .bind(id).bind(hash::sha256_hex(&token)).bind(user.user_id).bind(user.tenant_id).bind(user.org_id)
        .bind(binding.application_id).bind(&input.app_code).bind(user.session_id).bind(binding.membership_id)
        .bind(&binding.member_type).bind(binding.identity_version).bind(binding.account_version)
        .bind(binding.application_policy_version).bind(&binding.origin_issuer).bind(&binding.origin_client_ref)
        .bind(binding.origin_client_policy_version).bind(DateTime::from_timestamp(now.timestamp(),0)).bind(end)
        .execute(&mut *tx).await.map_err(|_|AppError::Unavailable)?;
    audit::write_in_tx_for_app(&mut tx,audit::AuditInfo {
        user_id:Some(user.user_id),username:&user.username,tenant_id:Some(user.tenant_id),organization_id:Some(user.org_id),
        action:"TOKEN_ISSUED",detail:&json!({"kind":"app_session","capabilityId":id,"audience":input.app_code,"expiresAt":end}).to_string(),
        ip:&audit::client_ip(&headers),user_agent:&audit::ua_of(&headers),
    },binding.application_id).await?;
    let active: bool = sqlx::query_scalar(
        "SELECT platform_app_capability_active($1) AND $2::timestamptz>clock_timestamp()",
    )
    .bind(hash::sha256_hex(&token))
    .bind(source_end)
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| AppError::Unavailable)?;
    if !active {
        return Err(AppError::unauthorized("应用能力来源已失效"));
    }
    tx.commit().await.map_err(|_| AppError::Unavailable)?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({"accessToken":token,"expiresIn":(end-now).num_seconds(),"tokenType":"Bearer","audience":input.app_code}),
        ),
    ))
}
