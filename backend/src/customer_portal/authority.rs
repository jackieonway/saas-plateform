//! Consumer-owned account proof. Implementations never select a membership.

use std::{future::Future, pin::Pin};

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::error::AppError;
use crate::util::uuid7;

#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Account {
    pub id: Uuid,
    pub username: String,
    pub real_name: String,
    pub avatar: String,
}

#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Source {
    pub issuer: String,
    pub client_id: Uuid,
    pub grant_id: Uuid,
    pub global_session_id: Uuid,
    pub user_id: Uuid,
    #[serde(serialize_with = "decimal")]
    pub account_version: i64,
    #[serde(serialize_with = "decimal")]
    pub client_policy_version: i64,
    pub auth_time: i64,
    pub expires_at: DateTime<Utc>,
}

fn decimal<S: serde::Serializer>(value: &i64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&value.to_string())
}

#[derive(Clone, PartialEq)]
pub(crate) struct AccountProof {
    pub source: Source,
    pub account: Account,
}

impl AccountProof {
    /// Shared semantic fence for local and remote authority implementations.
    /// The caller retains its original proof across business reads; a current
    /// account or a different RP can never repair a retired source tuple.
    pub(crate) fn validate(&self, issuer: &str, expected_client: Uuid) -> Result<(), AppError> {
        let source = &self.source;
        if source.issuer != issuer
            || source.client_id != expected_client
            || self.account.id != source.user_id
            || ![
                source.client_id,
                source.grant_id,
                source.global_session_id,
                source.user_id,
            ]
            .into_iter()
            .all(uuid7::is_uuid_v7)
            || source.account_version < 1
            || source.client_policy_version < 1
            || source.auth_time <= 0
            || source.auth_time > Utc::now().timestamp() + 60
            || source.expires_at <= Utc::now()
        {
            return Err(AppError::unauthorized("客户门户账号认证来源无效或已失效"));
        }
        Ok(())
    }
}

pub(crate) type ProofFuture<'a> =
    Pin<Box<dyn Future<Output = Result<AccountProof, AppError>> + Send + 'a>>;

/// Proves a current access token for one exact CUSTOMER_PORTAL relying party.
/// Invalid or retired source is 401; unavailable authority is 503. No permit
/// cache, refresh-token acceptance, membership or organization is part of it.
pub(crate) trait AccountProofPort: Send + Sync {
    fn verify_access<'a>(&'a self, access: &'a str, expected_client: Uuid) -> ProofFuture<'a>;
}

/// Missing identity configuration never enables a same-process provider.
pub(crate) struct UnavailableAuthority;
impl AccountProofPort for UnavailableAuthority {
    fn verify_access<'a>(&'a self, _access: &'a str, _expected_client: Uuid) -> ProofFuture<'a> {
        Box::pin(async { Err(AppError::Unavailable) })
    }
}
