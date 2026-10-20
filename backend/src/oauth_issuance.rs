//! Ordinary OAuth issuance and its shared live identity contract.
//!
//! The caller owns the transaction, client authentication, credential consumption
//! and audit. Lock the user first, then the client, then the one-time credential;
//! this module writes the complete token pair before that transaction commits.

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::{PgConnection, Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;
use crate::routes::oauth_admin::ClientFull;
use crate::state::AppState;
use crate::util::{hash, jwt};

#[derive(Clone, Copy)]
pub(crate) struct IssuanceIdentity {
    pub user_id: Option<Uuid>,
    pub tenant_id: Option<Uuid>,
    pub organization_id: Option<Uuid>,
}

/// Match account, membership and platform-session revocations' user-first order.
/// A missing account is rejected by the subsequent identity check.
pub(crate) async fn lock_user(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    if let Some(user_id) = user_id {
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE id = $1 FOR UPDATE")
            .bind(user_id)
            .fetch_optional(&mut **tx)
            .await?;
    }
    Ok(())
}

/// Resolve a currently valid ordinary OAuth identity to its username.
///
/// Machine grants have no user or context and return an empty username. User
/// grants require the exact membership and a matching, active tenant and entire
/// organization ancestry. Wall-clock expiry remains strict after lock waits.
pub(crate) async fn active_identity(
    connection: &mut PgConnection,
    user_id: Option<Uuid>,
    tenant_id: Option<Uuid>,
    org_id: Option<Uuid>,
    client_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    match (user_id, tenant_id, org_id) {
        (None, None, None) => {
            sqlx::query_scalar(
                "SELECT ''::text FROM oauth_clients WHERE client_id = $1 AND status = 'ENABLED'",
            )
            .bind(client_id)
            .fetch_optional(connection)
            .await
        }
        (Some(user_id), Some(tenant_id), Some(org_id)) => {
            sqlx::query_scalar(
                "WITH RECURSIVE chain AS ( \
                   SELECT id, parent_id, tenant_id, status, expires_at FROM organizations WHERE id = $3 \
                   UNION \
                   SELECT parent.id, parent.parent_id, parent.tenant_id, parent.status, parent.expires_at \
                   FROM chain child JOIN organizations parent ON parent.id = child.parent_id \
                 ) \
                 SELECT u.username FROM users u \
                 JOIN memberships m ON m.user_id = u.id AND m.organization_id = $3 \
                 JOIN organizations o ON o.id = m.organization_id AND o.tenant_id = $2 \
                 JOIN tenants t ON t.id = o.tenant_id \
                 JOIN oauth_clients c ON c.client_id = $4 AND c.status = 'ENABLED' \
                 WHERE u.id = $1 AND u.status = 'ENABLED' AND t.status = 'ENABLED' \
                   AND (t.expires_at IS NULL OR t.expires_at > clock_timestamp()) \
                   AND EXISTS (SELECT 1 FROM chain WHERE parent_id IS NULL) \
                   AND NOT EXISTS (SELECT 1 FROM chain \
                       WHERE tenant_id <> $2 OR status <> 'ENABLED' \
                         OR (expires_at IS NOT NULL AND expires_at <= clock_timestamp()))",
            )
            .bind(user_id)
            .bind(tenant_id)
            .bind(org_id)
            .bind(client_id)
            .fetch_optional(connection)
            .await
        }
        _ => Ok(None),
    }
}

fn expires_at(now: DateTime<Utc>, ttl: i64) -> Result<DateTime<Utc>, AppError> {
    let seconds = (ttl > 0)
        .then(|| now.timestamp().checked_add(ttl))
        .flatten()
        .ok_or_else(|| anyhow::anyhow!("invalid OAuth token lifetime"))?;
    DateTime::from_timestamp(seconds, 0)
        .ok_or_else(|| anyhow::anyhow!("OAuth token expiry is out of range").into())
}

/// Write a randomized access token and, if enabled, its replacement refresh.
///
/// No token string is persisted. The caller must propagate failures and roll
/// back its transaction, including any preceding one-time credential update.
pub(crate) async fn issue_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    state: &AppState,
    client: &ClientFull,
    identity: IssuanceIdentity,
    scope: &str,
) -> Result<Value, AppError> {
    if active_identity(
        tx.as_mut(),
        identity.user_id,
        identity.tenant_id,
        identity.organization_id,
        &client.client_id,
    )
    .await?
    .is_none()
    {
        return Err(AppError::unauthorized("OAuth 身份或客户端已失效"));
    }
    // An issued token cannot outlive a known authority deadline. Live reads
    // still reject subsequent disablement, membership deletion or shortening.
    let authority_expiry: Option<DateTime<Utc>> = match identity.organization_id {
        Some(org_id) => {
            sqlx::query_scalar(
                "WITH RECURSIVE chain AS ( \
                   SELECT id, parent_id, expires_at FROM organizations WHERE id = $1 \
                   UNION \
                   SELECT parent.id, parent.parent_id, parent.expires_at \
                   FROM chain child JOIN organizations parent ON parent.id = child.parent_id \
                 ) SELECT LEAST(t.expires_at, (SELECT min(expires_at) FROM chain)) \
                   FROM tenants t WHERE t.id = $2",
            )
            .bind(org_id)
            .bind(identity.tenant_id)
            .fetch_one(&mut **tx)
            .await?
        }
        None => None,
    };
    let now = Utc::now();
    let access_expiry = expires_at(now, client.access_token_validity)?;
    let access_expiry = authority_expiry.map_or(access_expiry, |end| end.min(access_expiry));
    let expires_in = access_expiry.timestamp() - now.timestamp();
    if expires_in <= 0 {
        return Err(AppError::unauthorized("OAuth 身份已到期"));
    }
    let jti = Uuid::new_v4();
    let access = jwt::sign(
        &jwt::Claims {
            sub: identity
                .user_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| client.client_id.clone()),
            typ: "access".into(),
            tenant: identity.tenant_id.map(|id| id.to_string()),
            org: identity.organization_id.map(|id| id.to_string()),
            cid: Some(client.client_id.clone()),
            scope: Some(scope.to_string()),
            sid: None,
            ver: None,
            aud: None,
            jti: Some(jti.to_string()),
            exp: access_expiry.timestamp(),
            iat: now.timestamp(),
        },
        &state.config.jwt_secret,
    )?;
    let access_written = sqlx::query(
        "INSERT INTO oauth_tokens \
         (token_hash, token_type, jwt, user_id, client_id, tenant_id, organization_id, \
          scope, expires_at, token_use, jti, storage_version) \
         VALUES ($1, 'access', NULL, $2, $3, $4, $5, $6, $7, 'oauth', $8, 2)",
    )
    .bind(hash::sha256_hex(&access))
    .bind(identity.user_id)
    .bind(&client.client_id)
    .bind(identity.tenant_id)
    .bind(identity.organization_id)
    .bind(scope)
    .bind(access_expiry)
    .bind(jti)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if access_written != 1 {
        return Err(anyhow::anyhow!("OAuth access token insert did not write one row").into());
    }
    let mut response = json!({
        "access_token": access,
        "token_type": "Bearer",
        "expires_in": expires_in,
        "scope": scope,
    });
    if client.has_grant("refresh_token") {
        let refresh_expiry = expires_at(now, client.refresh_token_validity)?;
        let refresh_expiry = authority_expiry.map_or(refresh_expiry, |end| end.min(refresh_expiry));
        if refresh_expiry.timestamp() <= now.timestamp() {
            return Err(AppError::unauthorized("OAuth 身份已到期"));
        }
        let refresh = hash::random_token(43);
        let refresh_written = sqlx::query(
            "INSERT INTO oauth_tokens \
             (token_hash, token_type, jwt, user_id, client_id, tenant_id, organization_id, \
              scope, expires_at, token_use, storage_version) \
             VALUES ($1, 'refresh', NULL, $2, $3, $4, $5, $6, $7, 'oauth', 2)",
        )
        .bind(hash::sha256_hex(&refresh))
        .bind(identity.user_id)
        .bind(&client.client_id)
        .bind(identity.tenant_id)
        .bind(identity.organization_id)
        .bind(scope)
        .bind(refresh_expiry)
        .execute(&mut **tx)
        .await?
        .rows_affected();
        if refresh_written != 1 {
            return Err(anyhow::anyhow!("OAuth refresh token insert did not write one row").into());
        }
        response["refresh_token"] = json!(refresh);
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_expiry_rejects_invalid_or_overflowing_configuration() {
        let now = Utc::now();
        for ttl in [0, -1, i64::MAX] {
            assert!(expires_at(now, ttl).is_err());
        }
        let expiry = expires_at(now, 300).unwrap();
        assert_eq!(expiry.timestamp(), now.timestamp() + 300);
    }
}
