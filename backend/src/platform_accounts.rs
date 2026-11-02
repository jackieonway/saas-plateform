//! Account capabilities required by the platform's existing consumers.
//!
//! These contracts contain no tenant, organization, membership, role or client
//! policy. The transaction capability is a same-database transition boundary:
//! its caller retains its original locks, audit/notification work and commit.
//! Independent deployment uses the explicit command/receipt protocol in the
//! HTTPS adapter; this transaction, CredentialHash and PreparedPassword are never
//! serialized or represented as a network transaction.

use std::{future::Future, pin::Pin};

use uuid::Uuid;

use crate::error::AppError;

pub(crate) type AccountFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>;

/// Credential material never implements Debug or serialization. Only the
/// authentication implementation and its persistence adapter inspect it.
pub(crate) struct CredentialHash(String);

impl CredentialHash {
    pub(crate) fn new(encoded: String) -> Self {
        Self(encoded)
    }

    pub(crate) fn encoded(&self) -> &str {
        &self.0
    }
}

/// An expensive password preparation completed before acquiring account locks.
pub(crate) struct PreparedPassword(CredentialHash);

impl PreparedPassword {
    pub(crate) fn new(encoded: String) -> Self {
        Self(CredentialHash::new(encoded))
    }

    pub(crate) fn encoded(&self) -> &str {
        self.0.encoded()
    }
}

/// A credential snapshot has no platform authorization claims. Its private
/// material is only handed back to the authentication port for verification.
pub(crate) struct AccountCredential {
    pub(crate) id: Uuid,
    pub(crate) username: String,
    pub(crate) status: String,
    pub(crate) account_version: i64,
    credential: CredentialHash,
}

impl AccountCredential {
    pub(crate) fn new(
        id: Uuid,
        username: String,
        _real_name: String,
        status: String,
        account_version: i64,
        credential: CredentialHash,
    ) -> Self {
        Self {
            id,
            username,
            status,
            account_version,
            credential,
        }
    }

    pub(crate) fn credential(&self) -> &CredentialHash {
        &self.credential
    }
}

pub(crate) struct NewAccount<'a> {
    pub(crate) id: Uuid,
    pub(crate) username: &'a str,
    pub(crate) real_name: &'a str,
    pub(crate) email: &'a str,
    pub(crate) phone: &'a str,
    pub(crate) gender: &'a str,
}

pub(crate) struct CreatedAccount {
    pub(crate) id: Uuid,
    pub(crate) username: String,
}

pub(crate) struct AccountSnapshot {
    pub(crate) id: Uuid,
    pub(crate) account_version: i64,
    pub(crate) credential: CredentialHash,
}

pub(crate) struct PasswordChange {
    pub(crate) before_version: i64,
    pub(crate) after_version: i64,
    pub(crate) revoked_sessions: u64,
}

/// Persistence operations consumed by account use cases, with no SQLx connection
/// or HTTP request/response/extractor types. This same-process seam reuses the
/// application's existing AppError mapping. A fake implements the same capability.
/// Implementations never commit; credential invalidation stays in the caller's tx.
pub(crate) trait AccountMutationTx: Send {
    fn lock_account(&mut self, id: Uuid) -> AccountFuture<'_, AccountSnapshot>;

    /// Compare the preverified snapshot after the caller's original user lock.
    #[cfg(test)]
    fn credential_matches<'a>(
        &'a mut self,
        credential: &'a AccountCredential,
    ) -> AccountFuture<'a, bool>;

    fn create<'a>(
        &'a mut self,
        account: NewAccount<'a>,
        password: &'a PreparedPassword,
    ) -> AccountFuture<'a, CreatedAccount>;

    fn set_status<'a>(&'a mut self, id: Uuid, status: &'a str) -> AccountFuture<'a, bool>;

    /// Elevate the account generation and retire every human credential kind.
    /// A password replacement is composed here with that same atomic operation.
    fn revoke_all<'a>(
        &'a mut self,
        id: Uuid,
        password: Option<&'a PreparedPassword>,
    ) -> AccountFuture<'a, u64>;

    fn account_version(&mut self, id: Uuid) -> AccountFuture<'_, i64>;
}

pub(crate) trait AuthenticationPort: Send + Sync {
    fn by_user_id(&self, id: Uuid) -> AccountFuture<'_, AccountCredential>;

    fn verify<'a>(
        &'a self,
        credential: &'a AccountCredential,
        password: &'a str,
    ) -> AccountFuture<'a, bool>;

    #[cfg(test)]
    fn credential_matches<'a>(
        &'a self,
        tx: &'a mut dyn AccountMutationTx,
        credential: &'a AccountCredential,
    ) -> AccountFuture<'a, bool>;
}

pub(crate) trait AccountAdministrationPort: Send + Sync {
    fn validate_password(&self, password: &str) -> Result<(), AppError>;

    fn prepare_password<'a>(&'a self, password: &'a str) -> AccountFuture<'a, PreparedPassword>;

    fn create<'a>(
        &'a self,
        tx: &'a mut dyn AccountMutationTx,
        account: NewAccount<'a>,
        password: &'a PreparedPassword,
    ) -> AccountFuture<'a, CreatedAccount>;

    fn replace_password<'a>(
        &'a self,
        tx: &'a mut dyn AccountMutationTx,
        id: Uuid,
        password: &'a PreparedPassword,
        expected: Option<&'a AccountCredential>,
    ) -> AccountFuture<'a, PasswordChange>;

    fn set_status<'a>(
        &'a self,
        tx: &'a mut dyn AccountMutationTx,
        id: Uuid,
        status: &'a str,
    ) -> AccountFuture<'a, bool>;
}
