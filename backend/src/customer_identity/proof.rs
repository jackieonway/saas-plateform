use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::bearer_token;
use crate::oauth_issuance::{self, UserBinding};
use crate::state::AppState;
use crate::util::{hash, jwt, uuid7::is_uuid_v7};

#[derive(Clone, Copy, PartialEq)]
struct Registration {
    client_ref: Uuid,
    application_id: Uuid,
}

impl Registration {
    fn from_state(state: &AppState) -> Result<Self, AppError> {
        state
            .config
            .customer_profile_client_ref
            .zip(state.config.customer_profile_application_id)
            .map(|(client_ref, application_id)| Self {
                client_ref,
                application_id,
            })
            .ok_or_else(|| AppError::forbidden("当前环境未登记客户本人资料入口"))
    }
}

#[derive(Clone, PartialEq, sqlx::FromRow)]
pub(super) struct Authority {
    pub(super) token_id: Uuid,
    pub(super) user_id: Uuid,
    pub(super) tenant_id: Uuid,
    pub(super) organization_id: Uuid,
    pub(super) client_id: String,
    pub(super) membership_id: Uuid,
    pub(super) member_type: String,
    pub(super) identity_version: i64,
    pub(super) account_version: i64,
    pub(super) client_policy_version: i64,
    pub(super) application_id: Uuid,
    pub(super) application_policy_version: i64,
    pub(super) platform_session_id: Option<Uuid>,
    scope: String,
    jti: Uuid,
    expires_at: DateTime<Utc>,
}

impl Authority {
    fn binding(&self) -> UserBinding {
        UserBinding {
            account_version: self.account_version,
            membership_id: self.membership_id,
            member_type: self.member_type.clone(),
            identity_version: self.identity_version,
            client_policy_version: self.client_policy_version,
            application_id: Some(self.application_id),
            application_policy_version: Some(self.application_policy_version),
            platform_session_id: self.platform_session_id,
        }
    }

    fn matches_claims(&self, claims: &jwt::Claims) -> bool {
        claims.typ == "access"
            && claims.sub == self.user_id.to_string()
            && claims.tenant.as_deref() == Some(self.tenant_id.to_string().as_str())
            && claims.org.as_deref() == Some(self.organization_id.to_string().as_str())
            && claims.cid.as_deref() == Some(self.client_id.as_str())
            && claims.scope.as_deref() == Some(self.scope.as_str())
            && claims.jti.as_deref() == Some(self.jti.to_string().as_str())
            && claims.exp == self.expires_at.timestamp()
            && claims.aud.is_none()
            && claims.sid.is_none()
            && claims.ver.is_none()
    }
}

pub(super) struct CustomerIdentity {
    pub(super) authority: Authority,
    token_digest: String,
    jwt_expires_at: DateTime<Utc>,
    registration: Registration,
}

fn invalid() -> AppError {
    AppError::unauthorized("客户资料访问凭据无效或准入已失效")
}

async fn load(
    pool: &PgPool,
    digest: &str,
    registration: Registration,
    jwt_expires_at: DateTime<Utc>,
) -> Result<Authority, AppError> {
    let mut connection = pool.acquire().await.map_err(|_| AppError::Unavailable)?;
    // The token hash supplies identity. Registration supplies only the allowed
    // purpose, never a user or membership selected by an untrusted request.
    let authority: Option<Authority> = sqlx::query_as(
        "SELECT tok.id AS token_id,tok.user_id,tok.tenant_id,tok.organization_id,tok.client_id, \
         tok.membership_id,tok.member_type,tok.identity_version,tok.account_version, \
         tok.client_policy_version,tok.application_id,tok.application_policy_version, \
         tok.platform_session_id,COALESCE(tok.scope,'') AS scope,tok.jti,tok.expires_at \
         FROM oauth_tokens tok JOIN oauth_clients c ON c.client_id=tok.client_id \
           AND c.id=$2 AND c.subject_mode='USER' AND c.application_id=$3 \
         WHERE tok.token_hash=$1 AND tok.token_type='access' AND tok.token_use='oauth' \
           AND tok.storage_version=2 AND NOT tok.revoked AND tok.expires_at>clock_timestamp() \
           AND $4::timestamptz>clock_timestamp() AND tok.application_id=$3 \
           AND tok.member_type='CUSTOMER' AND tok.user_id IS NOT NULL \
           AND tok.tenant_id IS NOT NULL AND tok.organization_id IS NOT NULL \
           AND tok.membership_id IS NOT NULL AND tok.identity_version IS NOT NULL \
           AND tok.account_version IS NOT NULL AND tok.client_policy_version IS NOT NULL \
           AND tok.application_policy_version IS NOT NULL AND tok.jti IS NOT NULL \
           AND 'profile'=ANY(regexp_split_to_array(COALESCE(tok.scope,''),'[[:space:]]+')) \
           AND 'profile'=ANY(regexp_split_to_array(trim(COALESCE(c.scopes,'')),'[[:space:]]*,[[:space:]]*'))",
    )
    .bind(digest)
    .bind(registration.client_ref)
    .bind(registration.application_id)
    .bind(jwt_expires_at)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| AppError::Unavailable)?;
    let authority = authority.ok_or_else(invalid)?;
    if !is_uuid_v7(authority.token_id) || !is_uuid_v7(authority.jti) {
        return Err(invalid());
    }
    if oauth_issuance::binding_identity(
        &mut connection,
        authority.user_id,
        authority.tenant_id,
        authority.organization_id,
        &authority.client_id,
        &authority.binding(),
    )
    .await
    .map_err(|_| AppError::Unavailable)?
    .is_none()
    {
        return Err(invalid());
    }
    Ok(authority)
}

impl CustomerIdentity {
    /// Fresh READ COMMITTED statements after data reads, with the original exact
    /// token proof. A current membership cannot repair a revoked generation.
    pub(super) async fn revalidate(&self, state: &AppState) -> Result<(), AppError> {
        if Registration::from_state(state)? != self.registration
            || load(
                &state.pool,
                &self.token_digest,
                self.registration,
                self.jwt_expires_at,
            )
            .await?
                != self.authority
        {
            return Err(invalid());
        }
        Ok(())
    }
}

impl FromRequestParts<AppState> for CustomerIdentity {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        let token = bearer_token(&parts.headers)?;
        let claims = jwt::verify(&token, &state.config.jwt_secret).map_err(|_| invalid())?;
        if claims.typ != "access" {
            return Err(invalid());
        }
        let registration = Registration::from_state(state)?;
        let jwt_expires_at = DateTime::from_timestamp(claims.exp, 0).ok_or_else(invalid)?;
        let token_digest = hash::sha256_hex(&token);
        let authority = load(&state.pool, &token_digest, registration, jwt_expires_at).await?;
        if !authority.matches_claims(&claims) {
            return Err(invalid());
        }
        Ok(Self {
            authority,
            token_digest,
            jwt_expires_at,
            registration,
        })
    }
}
