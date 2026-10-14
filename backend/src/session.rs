use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::Serialize;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;
use crate::state::AppState;
use crate::util::{hash, jwt};

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
         (s.revoked_at IS NULL AND s.expires_at > now() AND u.status='ENABLED' \
          AND t.status='ENABLED' AND org_chain_active(o.id) \
          AND (t.expires_at IS NULL OR t.expires_at>now()) \
          AND EXISTS(SELECT 1 FROM memberships m WHERE m.user_id=s.user_id \
                     AND m.organization_id=s.organization_id)) AS active, \
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
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| anyhow::anyhow!("refresh token randomness unavailable"))?;
    let token = format!("{PLATFORM_REFRESH_PREFIX}{}", hex::encode(bytes));
    sqlx::query(
        "INSERT INTO platform_session_refresh_tokens \
         (session_id, token_hash, session_version, expires_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(session_id)
    .bind(hash::sha256_hex(&token))
    .bind(version)
    .bind(expires_at)
    .execute(&mut **tx)
    .await?;
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
    if !is_platform_refresh(token) || token.len() != PLATFORM_REFRESH_PREFIX.len() + 64 {
        return Err(invalid());
    }
    let digest = hash::sha256_hex(token);
    let mut tx = state.pool.begin().await?;
    // Match membership/password mutations' lock order (user, then session) to
    // serialize revocation without introducing an inverse lock dependency.
    let locked_user: Option<Uuid> = sqlx::query_scalar(
        "SELECT u.id FROM users u JOIN platform_sessions s ON s.user_id=u.id \
         JOIN platform_session_refresh_tokens r ON r.session_id=s.id \
         WHERE r.token_hash=$1 FOR UPDATE OF u",
    )
    .bind(&digest)
    .fetch_optional(&mut *tx)
    .await?;
    if locked_user.is_none() {
        return Err(invalid());
    }
    let row: RefreshSession = sqlx::query_as(
        "SELECT r.id AS refresh_id, s.id AS session_id, s.user_id, s.tenant_id, \
         s.organization_id, r.session_version, LEAST(r.expires_at,s.expires_at, \
         t.expires_at,org_chain_expires_at(o.id)) AS expires_at \
         FROM platform_session_refresh_tokens r \
         JOIN platform_sessions s ON s.id=r.session_id \
         JOIN users u ON u.id=s.user_id \
         JOIN organizations o ON o.id=s.organization_id AND o.tenant_id=s.tenant_id \
         JOIN tenants t ON t.id=s.tenant_id \
         JOIN memberships m ON m.user_id=s.user_id AND m.organization_id=s.organization_id \
         WHERE r.token_hash=$1 AND r.consumed_at IS NULL AND r.expires_at>now() \
         AND s.revoked_at IS NULL AND s.expires_at>now() \
         AND u.status='ENABLED' AND u.session_version=r.session_version \
         AND t.status='ENABLED' AND (t.expires_at IS NULL OR t.expires_at>now()) \
         AND org_chain_active(o.id) FOR UPDATE OF r,s",
    )
    .bind(&digest)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(invalid)?;
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

/// 每个受保护请求直接查库：会话撤销、成员移除、租户/组织停用或到期立即生效。
pub async fn active_user(
    pool: &PgPool,
    user_id: Uuid,
    tenant_id: Uuid,
    org_id: Uuid,
    session_id: Uuid,
    version: i64,
) -> Result<ActiveUser, AppError> {
    let row: Option<ActiveUser> = sqlx::query_as(
        "SELECT u.username, COALESCE(u.real_name,''), u.is_super_admin \
         FROM platform_sessions s \
         JOIN users u ON u.id = s.user_id \
         JOIN organizations o ON o.id = s.organization_id AND o.tenant_id = s.tenant_id \
         JOIN tenants t ON t.id = s.tenant_id \
         JOIN memberships m ON m.user_id = s.user_id AND m.organization_id = s.organization_id \
         WHERE s.id = $1 AND s.user_id = $2 AND s.tenant_id = $3 AND s.organization_id = $4 \
           AND u.session_version = $5 AND u.status = 'ENABLED' \
           AND t.status = 'ENABLED' AND org_chain_active(o.id) \
           AND (t.expires_at IS NULL OR t.expires_at > now()) \
           AND s.revoked_at IS NULL AND s.expires_at > now()",
    )
    .bind(session_id)
    .bind(user_id)
    .bind(tenant_id)
    .bind(org_id)
    .bind(version)
    .fetch_optional(pool)
    .await?;
    row.ok_or_else(|| AppError::unauthorized("会话已失效，请重新登录"))
}

/// 修改密码或管理员踢出时原子提升版本并撤销全部平台 session。
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

pub async fn revoke_all_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    new_password_hash: Option<&str>,
) -> Result<u64, AppError> {
    let updated = sqlx::query(
        "UPDATE users SET session_version = session_version + 1, \
         password_hash = COALESCE($2, password_hash), updated_at = now() WHERE id = $1",
    )
    .bind(user_id)
    .bind(new_password_hash)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(AppError::not_found("用户不存在"));
    }
    let revoked = sqlx::query(
        "UPDATE platform_sessions SET revoked_at = now() \
         WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok(revoked)
}

pub async fn revoke_one(pool: &PgPool, user_id: Uuid, session_id: Uuid) -> Result<(), AppError> {
    let changed = sqlx::query(
        "UPDATE platform_sessions SET revoked_at = now() \
         WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
    )
    .bind(session_id)
    .bind(user_id)
    .execute(pool)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::unauthorized("会话已失效，请重新登录"));
    }
    Ok(())
}
