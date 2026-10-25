//! Same-process adapter for the customer's account-proof capability.

use std::sync::Arc;

use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    config::Config,
    customer_portal::authority::{Account, AccountProof, AccountProofPort, ProofFuture, Source},
    error::AppError,
};

struct LocalAuthority {
    pool: PgPool,
    config: Arc<Config>,
}

pub(crate) fn configured(pool: PgPool, config: Arc<Config>) -> Arc<dyn AccountProofPort> {
    Arc::new(LocalAuthority { pool, config })
}

impl AccountProofPort for LocalAuthority {
    fn verify_access<'a>(&'a self, access: &'a str, expected_client: Uuid) -> ProofFuture<'a> {
        Box::pin(async move {
            let invalid = || AppError::unauthorized("客户门户账号认证来源无效或已失效");
            if !self.config.oidc_enabled || !super::canonical_secret(access) {
                return Err(invalid());
            }
            let issuer = self
                .config
                .oidc_issuer
                .as_deref()
                .ok_or(AppError::Unavailable)?;
            let pinned: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM oidc_provider_settings WHERE singleton AND issuer=$1)",
            )
            .bind(issuer)
            .fetch_one(&self.pool)
            .await
            .map_err(|_| AppError::Unavailable)?;
            if !pinned {
                return Err(AppError::Unavailable);
            }
            // Access only. A refresh token or another RP's valid token never
            // becomes a customer business credential through this adapter.
            let proof = super::tokens::live(&self.pool, &super::digest(access), false)
                .await
                .map_err(|_| AppError::Unavailable)?
                .filter(|proof| {
                    proof.client_id == expected_client
                        && proof.client_purpose == "CUSTOMER_PORTAL"
                        && proof.allowed_member_types == ["CUSTOMER"]
                        && ["openid", "profile"].iter().all(|scope| {
                            proof.scope.iter().any(|present| present.as_str() == *scope)
                        })
                })
                .ok_or_else(invalid)?;
            let result = AccountProof {
                source: Source {
                    issuer: issuer.to_owned(),
                    client_id: proof.client_id,
                    grant_id: proof.grant_id,
                    global_session_id: proof.session_id,
                    user_id: proof.user_id,
                    account_version: proof.account_version,
                    client_policy_version: proof.client_policy_version,
                    auth_time: proof.auth_time.timestamp(),
                    expires_at: proof.expires_at,
                },
                account: Account {
                    id: proof.user_id,
                    username: proof.username,
                    real_name: proof.real_name,
                    avatar: proof.avatar,
                },
            };
            result.validate(issuer, expected_client)?;
            Ok(result)
        })
    }
}
