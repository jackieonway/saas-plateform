use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::Serialize;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;
use crate::util::{hash, uuid7::is_uuid_v7};

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

pub(crate) async fn lock_oidc_source_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    user_id: Uuid,
) -> Result<(), AppError> {
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
    Ok(())
}

/// An independent statement after source/policy waits and again before commit.
pub(crate) async fn require_source_active_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
) -> Result<(), AppError> {
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM platform_sessions s WHERE s.id=$1 AND s.bff_session_id IS NOT NULL \
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
           AND s.account_version = $5 AND s.member_type='EMPLOYEE' AND s.bff_session_id IS NOT NULL \
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
         AND s.member_type='EMPLOYEE' AND s.bff_session_id IS NOT NULL AND platform_session_active(s.id))",
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
