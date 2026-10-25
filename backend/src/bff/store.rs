//! Platform-owned BFF persistence and the explicit authentication read projection.

use super::{protocol, transport::Chain};
use crate::{error::AppError, state::AppState};
use chrono::{DateTime, Utc};
use sqlx::{Executor, Postgres, Transaction};
use uuid::Uuid;

#[derive(Clone, sqlx::FromRow)]
pub(super) struct Session {
    pub id: Uuid,
    pub browser_id: Uuid,
    pub token_hash: String,
    pub csrf_hash: String,
    pub user_id: Uuid,
    pub account_version: i64,
    pub global_session_id: Uuid,
    pub oidc_client_id: Uuid,
    pub oidc_grant_id: Uuid,
    pub oidc_client_policy_version: i64,
    pub issuer: String,
    pub auth_time: i64,
    pub nonce_hash: String,
    pub access_ciphertext: String,
    pub refresh_ciphertext: String,
    pub access_expires_at: DateTime<Utc>,
    pub version: i64,
    pub selected_session_id: Option<Uuid>,
    pub platform_refresh_ciphertext: Option<String>,
    pub refresh_lease_id: Option<Uuid>,
    pub refresh_started_at: Option<DateTime<Utc>>,
    pub refresh_uncertain: bool,
    pub logout_request_id: Option<Uuid>,
    pub logout_scope: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
pub(super) struct Browser {
    pub id: Uuid,
    pub generation: i64,
    pub current_session_id: Option<Uuid>,
}

#[derive(sqlx::FromRow)]
pub(super) struct Flow {
    pub id: Uuid,
    pub browser_id: Uuid,
    pub browser_generation: i64,
    pub nonce_hash: String,
    pub verifier_ciphertext: String,
    pub return_path: String,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
}

pub(super) fn keys(state: &AppState) -> Result<&super::CipherKeys, AppError> {
    state
        .config
        .bff_token_keys
        .as_deref()
        .ok_or(AppError::Unavailable)
}
pub(super) fn client(state: &AppState) -> Result<Uuid, AppError> {
    state.config.bff_oidc_client_id.ok_or(AppError::Unavailable)
}
pub(super) fn issuer(state: &AppState) -> Result<&str, AppError> {
    state
        .config
        .bff_oidc_issuer
        .as_deref()
        .ok_or(AppError::Unavailable)
}

pub(super) async fn session_id<'e, E: Executor<'e, Database = Postgres>>(
    executor: E,
    id: Uuid,
    lock: bool,
) -> Result<Option<Session>, AppError> {
    let sql = if lock {
        "SELECT * FROM bff_sessions WHERE id=$1 FOR UPDATE"
    } else {
        "SELECT * FROM bff_sessions WHERE id=$1"
    };
    sqlx::query_as(sql)
        .bind(id)
        .fetch_optional(executor)
        .await
        .map_err(|_| AppError::Unavailable)
}
pub(super) async fn session_cookie(state: &AppState, cookie: &str) -> Result<Session, AppError> {
    sqlx::query_as("SELECT * FROM bff_sessions WHERE token_hash=$1 AND revoked_at IS NULL AND expires_at>clock_timestamp()")
        .bind(super::digest(cookie)).fetch_optional(&state.pool).await.map_err(|_|AppError::Unavailable)?
        .ok_or_else(||AppError::unauthorized("平台认证已失效，请重新登录"))
}
pub(super) async fn browser<'e, E: Executor<'e, Database = Postgres>>(
    executor: E,
    hash: &str,
    lock: bool,
) -> Result<Option<Browser>, AppError> {
    let sql = if lock {
        "SELECT id,generation,current_session_id FROM console_browsers WHERE token_hash=$1 AND expires_at>clock_timestamp() FOR UPDATE"
    } else {
        "SELECT id,generation,current_session_id FROM console_browsers WHERE token_hash=$1 AND expires_at>clock_timestamp()"
    };
    sqlx::query_as(sql)
        .bind(hash)
        .fetch_optional(executor)
        .await
        .map_err(|_| AppError::Unavailable)
}
pub(super) async fn lock_user(
    connection: &mut sqlx::PgConnection,
    id: Uuid,
) -> Result<(), AppError> {
    let found: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(connection)
        .await
        .map_err(|_| AppError::Unavailable)?;
    if found.is_none() {
        return Err(AppError::unauthorized("账号认证已失效"));
    }
    Ok(())
}
pub(super) async fn lock_client(
    connection: &mut sqlx::PgConnection,
    id: Uuid,
    issuer: &str,
) -> Result<(), AppError> {
    let found: bool = sqlx::query_scalar("SELECT identity_lock_oidc_source($1,$2)")
        .bind(id.to_string())
        .bind(issuer)
        .fetch_one(connection)
        .await
        .map_err(|_| AppError::Unavailable)?;
    if !found {
        return Err(AppError::unauthorized("平台登录来源已失效"));
    }
    Ok(())
}

pub(super) async fn deadline(
    connection: &mut sqlx::PgConnection,
    expires: DateTime<Utc>,
) -> Result<(), AppError> {
    let current: bool = sqlx::query_scalar("SELECT $1::timestamptz>clock_timestamp()")
        .bind(expires)
        .fetch_one(connection)
        .await
        .map_err(|_| AppError::Unavailable)?;
    if !current {
        return Err(AppError::unauthorized("原认证证明已到期"));
    }
    Ok(())
}

pub(super) async fn proof(
    connection: &mut sqlx::PgConnection,
    proof: &protocol::AccountProof,
    issuer: &str,
) -> Result<(), AppError> {
    let valid: bool = sqlx::query_scalar("SELECT bff_oidc_proof_active($1,$2,$3,$4,$5,$6,$7)")
        .bind(proof.user_id)
        .bind(proof.global_session_id)
        .bind(proof.grant_id)
        .bind(proof.account_version)
        .bind(proof.client_id)
        .bind(proof.client_policy_version)
        .bind(issuer)
        .fetch_one(connection)
        .await
        .map_err(|_| AppError::Unavailable)?;
    if !valid {
        return Err(AppError::unauthorized(
            "原 SSO 会话、账号或平台客户端策略已失效",
        ));
    }
    Ok(())
}

/// This read projection can only reject a recorded authority binding. A true
/// result still requires the complete back-channel and final commit checks.
async fn reject_inactive_source(state: &AppState, original: &Session) -> Result<(), AppError> {
    let active: bool = sqlx::query_scalar("SELECT bff_oidc_proof_active($1,$2,$3,$4,$5,$6,$7)")
        .bind(original.user_id)
        .bind(original.global_session_id)
        .bind(original.oidc_grant_id)
        .bind(original.account_version)
        .bind(original.oidc_client_id)
        .bind(original.oidc_client_policy_version)
        .bind(&original.issuer)
        .fetch_one(&state.pool)
        .await
        .map_err(|_| AppError::Unavailable)?;
    if !active {
        return Err(AppError::unauthorized(
            "原 SSO 会话、账号或平台客户端策略已失效",
        ));
    }
    Ok(())
}

pub(super) async fn session_proof(
    connection: &mut sqlx::PgConnection,
    session: &Session,
) -> Result<(), AppError> {
    let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM bff_sessions b WHERE b.id=$1 AND b.version=$2 AND b.selected_session_id IS NOT DISTINCT FROM $3::uuid AND bff_session_active(b.id))")
        .bind(session.id).bind(session.version).bind(session.selected_session_id)
        .fetch_one(&mut *connection).await.map_err(|_|AppError::Unavailable)?;
    if !valid {
        return Err(AppError::unauthorized("平台认证已失效"));
    }
    deadline(connection, session.expires_at).await
}

pub(super) async fn revoke(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE bff_sessions SET revoked_at=COALESCE(revoked_at,clock_timestamp()) WHERE id=$1",
    )
    .bind(id)
    .execute(&mut **tx)
    .await?;
    sqlx::query("UPDATE platform_sessions SET revoked_at=COALESCE(revoked_at,clock_timestamp()) WHERE bff_session_id=$1").bind(id).execute(&mut **tx).await?;
    sqlx::query("UPDATE platform_session_refresh_tokens r SET consumed_at=COALESCE(consumed_at,clock_timestamp()) FROM platform_sessions s WHERE r.session_id=s.id AND s.bff_session_id=$1")
        .bind(id).execute(&mut **tx).await?;
    sqlx::query("UPDATE oauth_tokens t SET revoked=true FROM platform_sessions s WHERE t.platform_session_id=s.id AND s.bff_session_id=$1 AND NOT t.revoked")
        .bind(id).execute(&mut **tx).await?;
    Ok(())
}

pub(super) async fn online(
    chain: &Chain<'_>,
    original: Session,
    allow_refresh: bool,
) -> Result<Session, AppError> {
    let state = chain.state;
    if original.revoked_at.is_some()
        || original.issuer != issuer(state)?
        || original.oidc_client_id != client(state)?
    {
        return Err(AppError::unauthorized("平台认证来源已失效"));
    }
    if original.refresh_uncertain || original.logout_request_id.is_some() {
        return Err(AppError::Unavailable);
    }
    if allow_refresh && original.refresh_lease_id.is_some() {
        // An expired lease is an unknown external outcome, never permission to
        // replay the old refresh. Late responses cannot reverse this state.
        sqlx::query("UPDATE bff_sessions SET refresh_uncertain=true WHERE id=$1 AND refresh_lease_id IS NOT NULL AND refresh_started_at<=clock_timestamp()-$2*interval '1 millisecond'")
            .bind(original.id).bind(state.config.bff_authority_timeout_ms as f64).execute(&state.pool).await?;
        return Err(AppError::Unavailable);
    }
    reject_inactive_source(state, &original).await?;
    if allow_refresh && original.access_expires_at <= Utc::now() + chrono::Duration::seconds(30) {
        let lease = crate::util::uuid7::new_uuid();
        let leased=sqlx::query("UPDATE bff_sessions SET refresh_lease_id=$2,refresh_started_at=clock_timestamp() WHERE id=$1 AND revoked_at IS NULL AND expires_at>clock_timestamp() AND NOT refresh_uncertain AND logout_request_id IS NULL AND refresh_lease_id IS NULL AND access_ciphertext=$3 AND version=$4")
            .bind(original.id).bind(lease).bind(&original.access_ciphertext).bind(original.version).execute(&state.pool).await?.rows_affected();
        if leased != 1 {
            return Err(AppError::Unavailable);
        }
        let old = keys(state)?.open(
            original.id,
            &original.issuer,
            original.oidc_client_id,
            "oidc-refresh",
            &original.refresh_ciphertext,
        )?;
        let result = protocol::refresh(chain, &original, &old).await;
        let verified = match result {
            Ok(result) => result,
            Err(error) => {
                sqlx::query("UPDATE bff_sessions SET refresh_uncertain=true WHERE id=$1 AND refresh_lease_id=$2")
                    .bind(original.id).bind(lease).execute(&state.pool).await?;
                return Err(error);
            }
        };
        let mut tx = state
            .pool
            .begin()
            .await
            .map_err(|_| AppError::Unavailable)?;
        lock_user(&mut tx, original.user_id).await?;
        lock_client(&mut tx, original.oidc_client_id, &original.issuer).await?;
        let current = session_id(&mut *tx, original.id, true)
            .await?
            .ok_or_else(|| AppError::unauthorized("平台认证已失效"))?;
        if current.refresh_lease_id != Some(lease)
            || current.refresh_uncertain
            || current.logout_request_id.is_some()
            || current.revoked_at.is_some()
            || current.version != original.version
        {
            return Err(AppError::unauthorized("平台认证已在刷新期间变化"));
        }
        let lease_deadline = current.refresh_started_at.ok_or(AppError::Unavailable)?
            + chrono::Duration::milliseconds(state.config.bff_authority_timeout_ms as i64);
        deadline(&mut tx, lease_deadline).await?;
        proof(&mut tx, &verified.proof, &original.issuer).await?;
        deadline(
            &mut tx,
            original
                .expires_at
                .min(verified.proof.expires_at)
                .min(verified.id_expires_at),
        )
        .await?;
        deadline(&mut tx, lease_deadline).await?;
        let encrypted_access = keys(state)?.seal(
            original.id,
            &original.issuer,
            original.oidc_client_id,
            "oidc-access",
            &verified.access,
        )?;
        let encrypted_refresh = keys(state)?.seal(
            original.id,
            &original.issuer,
            original.oidc_client_id,
            "oidc-refresh",
            &verified.refresh,
        )?;
        sqlx::query("UPDATE bff_sessions SET access_ciphertext=$2,refresh_ciphertext=$3,access_expires_at=$4,refresh_lease_id=NULL,refresh_started_at=NULL WHERE id=$1")
            .bind(original.id).bind(encrypted_access).bind(encrypted_refresh).bind(verified.proof.expires_at).execute(&mut *tx).await?;
        let current = session_id(&mut *tx, original.id, false)
            .await?
            .ok_or(AppError::Unavailable)?;
        deadline(
            &mut tx,
            original
                .expires_at
                .min(verified.proof.expires_at)
                .min(verified.id_expires_at),
        )
        .await?;
        tx.commit().await?;
        return Ok(current);
    }
    let access = keys(state)?.open(
        original.id,
        &original.issuer,
        original.oidc_client_id,
        "oidc-access",
        &original.access_ciphertext,
    )?;
    let live = protocol::existing(chain, &original, &access).await?;
    let mut connection = state
        .pool
        .acquire()
        .await
        .map_err(|_| AppError::Unavailable)?;
    session_proof(&mut connection, &original).await?;
    let mut checked = original;
    checked.access_expires_at = checked.access_expires_at.min(live.expires_at);
    deadline(
        &mut connection,
        checked.access_expires_at.min(checked.expires_at),
    )
    .await?;
    Ok(checked)
}
