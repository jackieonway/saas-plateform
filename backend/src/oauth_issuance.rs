//! Ordinary OAuth issuance and its shared live identity contract.
//!
//! The caller owns the transaction, client authentication, credential consumption
//! and audit. Lock the user first, then the application, client and one-time credential;
//! this module writes the complete token pair before that transaction commits.

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::{PgConnection, Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;
use crate::routes::oauth_admin::ClientFull;
use crate::state::AppState;
use crate::util::{hash, jwt, uuid7::is_uuid_v7};

#[derive(Clone)]
pub(crate) struct IssuanceIdentity {
    pub user_id: Option<Uuid>,
    pub tenant_id: Option<Uuid>,
    pub organization_id: Option<Uuid>,
    pub binding: StoredBinding,
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

/// Persisted proof is nullable so pre-binding credentials fail closed.
#[derive(Clone, Debug, Default, sqlx::FromRow)]
pub(crate) struct StoredBinding {
    pub account_version: Option<i64>,
    pub membership_id: Option<Uuid>,
    pub member_type: Option<String>,
    pub identity_version: Option<i64>,
    pub client_policy_version: Option<i64>,
    pub application_id: Option<Uuid>,
    pub application_policy_version: Option<i64>,
    pub platform_session_id: Option<Uuid>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub(crate) struct UserBinding {
    pub account_version: i64,
    pub membership_id: Uuid,
    pub member_type: String,
    pub identity_version: i64,
    pub client_policy_version: i64,
    pub application_id: Option<Uuid>,
    pub application_policy_version: Option<i64>,
    pub platform_session_id: Option<Uuid>,
}

impl StoredBinding {
    pub(crate) fn user(&self) -> Option<UserBinding> {
        Some(UserBinding {
            account_version: self.account_version?,
            membership_id: self.membership_id?,
            member_type: self.member_type.clone()?,
            identity_version: self.identity_version?,
            client_policy_version: self.client_policy_version?,
            application_id: self.application_id,
            application_policy_version: self.application_policy_version,
            platform_session_id: self.platform_session_id,
        })
    }
}

impl From<UserBinding> for StoredBinding {
    fn from(value: UserBinding) -> Self {
        Self {
            account_version: Some(value.account_version),
            membership_id: Some(value.membership_id),
            member_type: Some(value.member_type),
            identity_version: Some(value.identity_version),
            client_policy_version: Some(value.client_policy_version),
            application_id: value.application_id,
            application_policy_version: value.application_policy_version,
            platform_session_id: value.platform_session_id,
        }
    }
}

/// Validate exactly the proof stored at issuance; current state cannot repair it.
pub(crate) async fn binding_identity(
    connection: &mut PgConnection,
    user_id: Uuid,
    tenant_id: Uuid,
    org_id: Uuid,
    client_id: &str,
    binding: &UserBinding,
) -> Result<Option<String>, sqlx::Error> {
    if ![user_id, tenant_id, org_id, binding.membership_id]
        .into_iter()
        .all(is_uuid_v7)
        || binding.application_id.is_some_and(|id| !is_uuid_v7(id))
        || binding
            .platform_session_id
            .is_some_and(|id| !is_uuid_v7(id))
    {
        return Ok(None);
    }
    sqlx::query_scalar(
        "SELECT u.username FROM users u \
         JOIN oauth_clients c ON c.client_id=$4 AND c.status='ENABLED' \
           AND c.subject_mode IN ('USER','LEGACY_MIXED') \
           AND c.access_policy_version=$9 AND $7=ANY(c.allowed_member_types) \
           AND c.application_id IS NOT DISTINCT FROM $10 \
         LEFT JOIN applications a ON a.id=c.application_id \
         WHERE u.id=$1 AND identity_binding_active($1,$2,$3,$5,$6,$7,$8) \
           AND ($12::uuid IS NULL OR EXISTS (SELECT 1 FROM platform_sessions s WHERE s.id=$12 \
             AND s.user_id=$1 AND s.tenant_id=$2 AND s.organization_id=$3 \
             AND s.account_version=$5 AND s.membership_id=$6 AND s.member_type=$7 AND s.identity_version=$8 \
             AND platform_session_active(s.id))) \
           AND ((c.application_id IS NULL AND $11::bigint IS NULL) OR \
             (a.status='ENABLED' AND a.access_policy_version=$11 \
              AND $7=ANY(a.allowed_member_types) \
              AND EXISTS(SELECT 1 FROM tenant_applications WHERE tenant_id=$2 AND application_id=a.id) \
              AND EXISTS(SELECT 1 FROM organization_applications WHERE organization_id=$3 AND application_id=a.id)))",
    ).bind(user_id).bind(tenant_id).bind(org_id).bind(client_id)
    .bind(binding.account_version).bind(binding.membership_id).bind(&binding.member_type)
    .bind(binding.identity_version).bind(binding.client_policy_version)
    .bind(binding.application_id).bind(binding.application_policy_version).bind(binding.platform_session_id)
    .fetch_optional(connection).await
}

/// Capture registration when starting a pending request before any user exists.
pub(crate) async fn client_registration(
    connection: &mut PgConnection,
    client_id: &str,
) -> Result<Option<(i64, Option<Uuid>, Option<i64>)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT c.access_policy_version,c.application_id,a.access_policy_version \
         FROM oauth_clients c LEFT JOIN applications a ON a.id=c.application_id \
         WHERE c.client_id=$1 AND c.status='ENABLED' AND c.subject_mode IN ('USER','LEGACY_MIXED') \
         AND (c.application_id IS NULL OR a.status='ENABLED')",
    )
    .bind(client_id)
    .fetch_optional(connection)
    .await
}

pub(crate) async fn active_identity(
    connection: &mut PgConnection,
    user_id: Option<Uuid>,
    tenant_id: Option<Uuid>,
    org_id: Option<Uuid>,
    client_id: &str,
    binding: &StoredBinding,
) -> Result<Option<String>, sqlx::Error> {
    match (user_id,tenant_id,org_id) {
        (Some(user),Some(tenant),Some(org)) => match binding.user() {
            Some(proof) => binding_identity(connection,user,tenant,org,client_id,&proof).await,
            None => Ok(None),
        },
        (None,None,None) if binding.account_version.is_none() && binding.membership_id.is_none()
            && binding.member_type.is_none() && binding.identity_version.is_none() && binding.platform_session_id.is_none() => {
            sqlx::query_scalar(
                "SELECT ''::text FROM oauth_clients c LEFT JOIN applications a ON a.id=c.application_id \
                 WHERE c.client_id=$1 AND c.status='ENABLED' AND c.subject_mode IN ('MACHINE','LEGACY_MIXED') \
                   AND c.access_policy_version=$2 AND c.application_id IS NOT DISTINCT FROM $3 \
                   AND ((c.application_id IS NULL AND $4::bigint IS NULL) OR \
                     (a.status='ENABLED' AND a.access_policy_version=$4))",
            ).bind(client_id).bind(binding.client_policy_version).bind(binding.application_id)
                .bind(binding.application_policy_version).fetch_optional(connection).await
        },
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
        &identity.binding,
    )
    .await
    .map_err(|_| AppError::Unavailable)?
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
    let authority_expiry = if let Some(sid) = identity.binding.platform_session_id {
        let parent: DateTime<Utc> = sqlx::query_scalar(
            "SELECT expires_at FROM platform_sessions WHERE id=$1 AND platform_session_active(id)",
        )
        .bind(sid)
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| AppError::Unavailable)?;
        Some(authority_expiry.map_or(parent, |end| end.min(parent)))
    } else {
        authority_expiry
    };
    let now = Utc::now();
    let access_expiry = expires_at(now, client.access_token_validity)?;
    let access_expiry = authority_expiry.map_or(access_expiry, |end| end.min(access_expiry));
    let expires_in = access_expiry.timestamp() - now.timestamp();
    if expires_in <= 0 {
        return Err(AppError::unauthorized("OAuth 身份已到期"));
    }
    let jti = crate::util::uuid7::new_uuid();
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
          scope, expires_at, token_use, jti, storage_version, id, \
          account_version,membership_id,member_type,identity_version,client_policy_version,application_id,application_policy_version,platform_session_id) \
         VALUES ($1, 'access', NULL, $2, $3, $4, $5, $6, $7, 'oauth', $8, 2, $9,$10,$11,$12,$13,$14,$15,$16,$17)",
    )
    .bind(hash::sha256_hex(&access))
    .bind(identity.user_id)
    .bind(&client.client_id)
    .bind(identity.tenant_id)
    .bind(identity.organization_id)
    .bind(scope)
    .bind(access_expiry)
    .bind(jti)
    .bind(crate::util::uuid7::new_uuid())
    .bind(identity.binding.account_version).bind(identity.binding.membership_id)
    .bind(&identity.binding.member_type).bind(identity.binding.identity_version)
    .bind(identity.binding.client_policy_version).bind(identity.binding.application_id)
    .bind(identity.binding.application_policy_version).bind(identity.binding.platform_session_id)
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
              scope, expires_at, token_use, storage_version, id, \
              account_version,membership_id,member_type,identity_version,client_policy_version,application_id,application_policy_version,platform_session_id) \
             VALUES ($1, 'refresh', NULL, $2, $3, $4, $5, $6, $7, 'oauth', 2, $8,$9,$10,$11,$12,$13,$14,$15,$16)",
        )
        .bind(hash::sha256_hex(&refresh))
        .bind(identity.user_id)
        .bind(&client.client_id)
        .bind(identity.tenant_id)
        .bind(identity.organization_id)
        .bind(scope)
        .bind(refresh_expiry)
        .bind(crate::util::uuid7::new_uuid())
        .bind(identity.binding.account_version).bind(identity.binding.membership_id)
        .bind(&identity.binding.member_type).bind(identity.binding.identity_version)
        .bind(identity.binding.client_policy_version).bind(identity.binding.application_id)
        .bind(identity.binding.application_policy_version).bind(identity.binding.platform_session_id)
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
    fn old_or_partial_credentials_cannot_acquire_a_current_user_binding() {
        let full = StoredBinding {
            account_version: Some(4),
            membership_id: Some(crate::util::uuid7::new_uuid()),
            member_type: Some("CUSTOMER".into()),
            identity_version: Some(8),
            client_policy_version: Some(3),
            ..Default::default()
        };
        assert!(full.user().is_some());
        assert!(StoredBinding::default().user().is_none());
        for missing in 0..5 {
            let mut proof = full.clone();
            match missing {
                0 => proof.account_version = None,
                1 => proof.membership_id = None,
                2 => proof.member_type = None,
                3 => proof.identity_version = None,
                _ => proof.client_policy_version = None,
            }
            assert!(proof.user().is_none());
        }
    }

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
