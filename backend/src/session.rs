use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::Serialize;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;
use crate::state::AppState;
use crate::util::{hash, jwt, uuid7::is_uuid_v7};

pub type ActiveUser = (String, String, bool);

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeviceSession {
    id: Uuid,
    user_id: Uuid,
    username: String,
    real_name: String,
    tenant_id: Uuid,
    tenant_name: String,
    organization_id: Uuid,
    organization_name: String,
    issued_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
    ip: String,
    user_agent: String,
    active: bool,
    is_current: bool,
}

/// Owner-filtered personal device listing or the platform administrator's global view.
/// The caller must enforce platform permission before requesting an unfiltered view.
pub(crate) async fn device_sessions(
    pool: &PgPool,
    owner: Option<Uuid>,
    current_user: Uuid,
    current_session: Uuid,
) -> Result<Vec<DeviceSession>, AppError> {
    Ok(sqlx::query_as(
        "SELECT s.id, s.user_id, u.username, COALESCE(u.real_name,'') AS real_name, \
         s.tenant_id, t.name AS tenant_name, s.organization_id, o.name AS organization_name, \
         s.issued_at, s.expires_at, s.revoked_at, s.ip, s.user_agent, \
         platform_session_active(s.id) AS active, \
         (s.id=$2 AND s.user_id=$3) AS is_current \
         FROM platform_sessions s JOIN users u ON u.id=s.user_id \
         JOIN tenants t ON t.id=s.tenant_id \
         JOIN organizations o ON o.id=s.organization_id AND o.tenant_id=s.tenant_id \
         WHERE ($1::uuid IS NULL OR s.user_id=$1) \
         ORDER BY s.issued_at DESC,s.id DESC LIMIT $4",
    )
    .bind(owner)
    .bind(current_session)
    .bind(current_user)
    .bind(if owner.is_some() { 100_i64 } else { 500_i64 })
    .fetch_all(pool)
    .await?)
}

const PLATFORM_REFRESH_PREFIX: &str = "psr_";

pub(crate) fn is_platform_refresh(token: &str) -> bool {
    token.starts_with(PLATFORM_REFRESH_PREFIX)
}

/// Only a hash is stored; prefix separates platform sessions from OAuth client grants.
pub(crate) async fn issue_refresh_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    version: i64,
    expires_at: DateTime<Utc>,
) -> Result<String, AppError> {
    if !is_uuid_v7(session_id) {
        return Err(AppError::unauthorized("会话已失效，请重新登录"));
    }
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| anyhow::anyhow!("refresh token randomness unavailable"))?;
    let token = format!("{PLATFORM_REFRESH_PREFIX}{}", hex::encode(bytes));
    let written = sqlx::query(
        "INSERT INTO platform_session_refresh_tokens \
         (session_id, token_hash, session_version, expires_at, id) \
         SELECT $1,$2,$3,$4,$5 FROM platform_sessions s \
         WHERE s.id=$1 AND s.account_version=$3 AND platform_session_active(s.id)",
    )
    .bind(session_id)
    .bind(hash::sha256_hex(&token))
    .bind(version)
    .bind(expires_at)
    .bind(crate::util::uuid7::new_uuid())
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if written != 1 {
        return Err(AppError::unauthorized("会话已失效，请重新登录"));
    }
    Ok(token)
}

pub(crate) struct RefreshedSession {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    pub refresh_expires_in: i64,
}

#[derive(sqlx::FromRow)]
struct RefreshSession {
    refresh_id: Uuid,
    session_id: Uuid,
    user_id: Uuid,
    tenant_id: Uuid,
    organization_id: Uuid,
    session_version: i64,
    expires_at: DateTime<Utc>,
}

/// Rotate once, keeping the same user and context and the original absolute expiry.
pub(crate) async fn refresh(state: &AppState, token: &str) -> Result<RefreshedSession, AppError> {
    let invalid = || AppError::unauthorized("刷新令牌无效或已失效，请重新登录");
    if state.config.bff_enabled && !state.config.bff_legacy_login_enabled {
        return Err(invalid());
    }
    if !is_platform_refresh(token) || token.len() != PLATFORM_REFRESH_PREFIX.len() + 64 {
        return Err(invalid());
    }
    let digest = hash::sha256_hex(token);
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| AppError::Unavailable)?;
    // Match membership/password mutations' lock order (user, then session) to
    // serialize revocation without introducing an inverse lock dependency.
    let locked_user: Option<Uuid> = sqlx::query_scalar(
        "SELECT u.id FROM users u JOIN platform_sessions s ON s.user_id=u.id \
         JOIN platform_session_refresh_tokens r ON r.session_id=s.id \
         WHERE r.token_hash=$1 FOR UPDATE OF u",
    )
    .bind(&digest)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| AppError::Unavailable)?;
    if !locked_user.is_some_and(is_uuid_v7) {
        return Err(invalid());
    }
    lock_source_for_refresh(&mut tx, &digest).await?;
    let row: RefreshSession = sqlx::query_as(
        "SELECT r.id AS refresh_id, s.id AS session_id, s.user_id, s.tenant_id, \
         s.organization_id, r.session_version, LEAST(r.expires_at,s.expires_at, \
         t.expires_at,org_chain_expires_at(o.id)) AS expires_at \
         FROM platform_session_refresh_tokens r \
         JOIN platform_sessions s ON s.id=r.session_id \
         JOIN users u ON u.id=s.user_id \
         JOIN organizations o ON o.id=s.organization_id AND o.tenant_id=s.tenant_id \
         JOIN tenants t ON t.id=s.tenant_id \
         WHERE r.token_hash=$1 AND r.consumed_at IS NULL AND r.expires_at>clock_timestamp() \
         AND s.account_version=r.session_version AND platform_session_active(s.id) \
         FOR UPDATE OF r,s",
    )
    .bind(&digest)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| AppError::Unavailable)?
    .ok_or_else(invalid)?;
    if ![
        row.refresh_id,
        row.session_id,
        row.user_id,
        row.tenant_id,
        row.organization_id,
    ]
    .into_iter()
    .all(is_uuid_v7)
    {
        return Err(invalid());
    }
    let now = Utc::now().timestamp();
    let access_expiry =
        (now + state.config.session_ttl_hours * 3600).min(row.expires_at.timestamp());
    if access_expiry <= now {
        return Err(invalid());
    }
    sqlx::query("UPDATE platform_session_refresh_tokens SET consumed_at=now() WHERE id=$1")
        .bind(row.refresh_id)
        .execute(&mut *tx)
        .await?;
    let refresh_token =
        issue_refresh_in_tx(&mut tx, row.session_id, row.session_version, row.expires_at).await?;
    let access_token = jwt::sign(
        &jwt::Claims {
            sub: row.user_id.to_string(),
            typ: "session".into(),
            tenant: Some(row.tenant_id.to_string()),
            org: Some(row.organization_id.to_string()),
            cid: None,
            scope: None,
            sid: Some(row.session_id.to_string()),
            ver: Some(row.session_version),
            aud: None,
            jti: None,
            exp: access_expiry,
            iat: now,
        },
        &state.config.jwt_secret,
    )?;
    tx.commit().await?;
    Ok(RefreshedSession {
        access_token,
        refresh_token,
        expires_in: access_expiry - now,
        refresh_expires_in: row.expires_at.timestamp() - now,
    })
}

async fn lock_source_for_refresh(
    tx: &mut Transaction<'_, Postgres>,
    digest: &str,
) -> Result<(), AppError> {
    let source: Option<String> = sqlx::query_scalar(
        "SELECT c.client_id FROM oauth_clients c \
         JOIN platform_sessions s ON s.origin_client_ref=c.client_id \
         JOIN platform_session_refresh_tokens r ON r.session_id=s.id \
         WHERE r.token_hash=$1 AND s.origin_issuer='urn:saas:legacy-platform' \
         AND s.bff_session_id IS NULL FOR SHARE OF c",
    )
    .bind(digest)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| AppError::Unavailable)?;
    if source.is_none() {
        return Err(AppError::unauthorized("刷新令牌缺少来源绑定，请重新登录"));
    }
    Ok(())
}

/// Callers already hold the user and optional target application locks. Lock an
/// OIDC source before legacy registrations; return a legacy source for their
/// sorted batch lock. This performs no network request while locks are held.
pub(crate) async fn lock_oidc_source_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    user_id: Uuid,
) -> Result<Option<String>, AppError> {
    let source: Option<(String, String, Option<Uuid>)> = sqlx::query_as(
        "SELECT origin_issuer,origin_client_ref,bff_session_id FROM platform_sessions \
         WHERE id=$1 AND user_id=$2 AND origin_issuer IS NOT NULL AND origin_client_ref IS NOT NULL",
    )
    .bind(session_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| AppError::Unavailable)?;
    let (issuer, client, bff_session) =
        source.ok_or_else(|| AppError::unauthorized("来源会话已失效"))?;
    if issuer == "urn:saas:legacy-platform" && bff_session.is_none() {
        return Ok(Some(client));
    }
    if bff_session.is_none() {
        return Err(AppError::unauthorized("来源会话缺少认证绑定"));
    }
    let locked: bool = sqlx::query_scalar("SELECT identity_lock_oidc_source($1,$2)")
        .bind(&client)
        .bind(issuer)
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| AppError::Unavailable)?;
    if !locked {
        return Err(AppError::unauthorized("OIDC 来源客户端已失效"));
    }
    Ok(None)
}

/// An independent statement after source/policy waits and again before commit.
pub(crate) async fn require_source_active_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
) -> Result<(), AppError> {
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM platform_sessions s WHERE s.id=$1 \
         AND platform_source_active(s.origin_issuer,s.origin_client_ref,s.origin_client_policy_version,s.member_type) \
         AND platform_session_active(s.id))",
    )
    .bind(session_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|_| AppError::Unavailable)?;
    if !active {
        return Err(AppError::unauthorized("来源平台上下文已失效"));
    }
    Ok(())
}

/// 每个受保护请求直接查库：会话撤销、成员移除、租户/组织停用或到期立即生效。
pub async fn active_user(
    pool: &PgPool,
    user_id: Uuid,
    tenant_id: Uuid,
    org_id: Uuid,
    session_id: Uuid,
    version: i64,
) -> Result<ActiveUser, AppError> {
    if ![user_id, tenant_id, org_id, session_id]
        .into_iter()
        .all(is_uuid_v7)
    {
        return Err(AppError::unauthorized("会话已失效，请重新登录"));
    }
    let row: Option<ActiveUser> = sqlx::query_as(
        "SELECT u.username, COALESCE(u.real_name,''), u.is_super_admin \
         FROM platform_sessions s \
         JOIN users u ON u.id = s.user_id \
         JOIN organizations o ON o.id = s.organization_id AND o.tenant_id = s.tenant_id \
         JOIN tenants t ON t.id = s.tenant_id \
         WHERE s.id = $1 AND s.user_id = $2 AND s.tenant_id = $3 AND s.organization_id = $4 \
           AND s.account_version = $5 AND s.member_type='EMPLOYEE' \
           AND platform_session_active(s.id)",
    )
    .bind(session_id)
    .bind(user_id)
    .bind(tenant_id)
    .bind(org_id)
    .bind(version)
    .fetch_optional(pool)
    .await
    .map_err(|_| AppError::Unavailable)?;
    row.ok_or_else(|| AppError::unauthorized("会话已失效，请重新登录"))
}

/// Recheck a session after taking the same user lock as password/kick mutations.
/// Credential approval must keep this lock until its derived code is committed.
pub(crate) async fn require_active_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    tenant_id: Uuid,
    org_id: Uuid,
    session_id: Uuid,
) -> Result<(), AppError> {
    if ![user_id, tenant_id, org_id, session_id]
        .into_iter()
        .all(is_uuid_v7)
    {
        return Err(AppError::unauthorized("会话已失效，请重新登录"));
    }
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(user_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| AppError::Unavailable)?;
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM platform_sessions s \
         JOIN users u ON u.id=s.user_id AND u.status='ENABLED' \
         JOIN organizations o ON o.id=s.organization_id AND o.tenant_id=s.tenant_id \
         JOIN tenants t ON t.id=s.tenant_id AND t.status='ENABLED' \
         WHERE s.id=$1 AND s.user_id=$2 AND s.tenant_id=$3 AND s.organization_id=$4 \
         AND s.member_type='EMPLOYEE' AND platform_session_active(s.id))",
    )
    .bind(session_id)
    .bind(user_id)
    .bind(tenant_id)
    .bind(org_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|_| AppError::Unavailable)?;
    if !active {
        return Err(AppError::unauthorized("会话已失效，请重新登录"));
    }
    Ok(())
}

/// 修改密码或管理员踢出时原子提升版本并撤销全部平台与 OAuth 凭据。
/// Production callers compose this operation with their own checks in one transaction.
#[cfg(test)]
pub async fn revoke_all(
    pool: &PgPool,
    user_id: Uuid,
    new_password_hash: Option<&str>,
) -> Result<u64, AppError> {
    let mut tx = pool.begin().await?;
    let revoked = revoke_all_in_tx(&mut tx, user_id, new_password_hash).await?;
    tx.commit().await?;
    Ok(revoked)
}

#[cfg(test)]
pub async fn revoke_all_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    new_password_hash: Option<&str>,
) -> Result<u64, AppError> {
    crate::authentication::revoke_account_in_tx(tx, user_id, new_password_hash).await
}

#[cfg(test)]
mod uuid_tests {
    use super::*;
    use crate::util::uuid7::fixture_uuid;

    #[tokio::test]
    async fn active_user_rejects_legacy_identity_before_database_lookup() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/unused_session_uuid_test")
            .unwrap();
        for invalid in [
            Uuid::nil(),
            Uuid::new_v4(),
            Uuid::parse_str("019a1163-0000-7000-c000-000000000001").unwrap(),
        ] {
            for field in 0..4 {
                let mut ids = [
                    fixture_uuid(1),
                    fixture_uuid(2),
                    fixture_uuid(3),
                    fixture_uuid(4),
                ];
                ids[field] = invalid;
                assert!(matches!(
                    active_user(&pool, ids[0], ids[1], ids[2], ids[3], 0).await,
                    Err(AppError::Unauthorized(_))
                ));
            }
        }
    }
}
