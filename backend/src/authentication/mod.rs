//! Account authentication and administration adapters.
//!
//! Local mode retains the same-database transaction seam. Independent mode uses
//! explicit durable commands after platform authorization commits; it never
//! silently falls back to this process's password storage on a network failure.

pub(crate) mod commands;
mod postgres;
pub(crate) mod remote;

use std::sync::Arc;

use sqlx::PgPool;
use uuid::Uuid;

use crate::config::Config;
use crate::error::AppError;
use crate::platform_accounts::{
    AccountAdministrationPort, AccountCredential, AccountFuture, AccountMutationTx,
    AuthenticationPort, CreatedAccount, LoginIdentifier, NewAccount, PasswordChange,
    PreparedPassword,
};
use crate::util::password::{self, PasswordService};

#[cfg(test)]
pub(crate) use postgres::revoke_account_in_tx;
pub(crate) use postgres::PostgresAccountTransaction;

struct AccountService {
    pool: PgPool,
    passwords: PasswordService,
    config: Arc<Config>,
}

pub(crate) type AccountPorts = (
    Arc<dyn AuthenticationPort>,
    Arc<dyn AccountAdministrationPort>,
    Option<Arc<remote::RemoteAccounts>>,
);

/// Local capabilities share the process password budget; remote capabilities
/// share one bounded HTTPS adapter and never expose a password hash.
pub(crate) fn new(pool: PgPool, passwords: PasswordService, config: Arc<Config>) -> AccountPorts {
    if let Some(service) = remote::RemoteAccounts::configured(config.clone()) {
        return (service.clone(), service.clone(), Some(service));
    }
    let service = Arc::new(AccountService {
        pool,
        passwords,
        config,
    });
    (service.clone(), service, None)
}

impl AuthenticationPort for AccountService {
    fn candidates<'a>(
        &'a self,
        identifier: LoginIdentifier<'a>,
    ) -> AccountFuture<'a, Vec<AccountCredential>> {
        Box::pin(async move { postgres::candidates(&self.pool, identifier).await })
    }

    fn by_username<'a>(
        &'a self,
        username: &'a str,
    ) -> AccountFuture<'a, Option<AccountCredential>> {
        Box::pin(async move { postgres::by_username(&self.pool, username).await })
    }

    fn by_user_id(&self, id: Uuid) -> AccountFuture<'_, AccountCredential> {
        Box::pin(async move { postgres::by_user_id(&self.pool, id).await })
    }

    fn verify<'a>(
        &'a self,
        credential: &'a AccountCredential,
        password: &'a str,
    ) -> AccountFuture<'a, bool> {
        Box::pin(async move {
            self.passwords
                .verify(credential.credential().encoded(), password)
                .await
        })
    }

    fn credential_matches<'a>(
        &'a self,
        tx: &'a mut dyn AccountMutationTx,
        credential: &'a AccountCredential,
    ) -> AccountFuture<'a, bool> {
        tx.credential_matches(credential)
    }
}

impl AccountAdministrationPort for AccountService {
    fn validate_password(&self, password: &str) -> Result<(), AppError> {
        password::validate_for_config(password, &self.config)
    }

    fn prepare_password<'a>(&'a self, password: &'a str) -> AccountFuture<'a, PreparedPassword> {
        Box::pin(async move {
            self.validate_password(password)?;
            self.passwords
                .hash(password)
                .await
                .map(PreparedPassword::new)
        })
    }

    fn create<'a>(
        &'a self,
        tx: &'a mut dyn AccountMutationTx,
        account: NewAccount<'a>,
        password: &'a PreparedPassword,
    ) -> AccountFuture<'a, CreatedAccount> {
        tx.create(account, password)
    }

    fn replace_password<'a>(
        &'a self,
        tx: &'a mut dyn AccountMutationTx,
        id: Uuid,
        password: &'a PreparedPassword,
        expected: Option<&'a AccountCredential>,
    ) -> AccountFuture<'a, PasswordChange> {
        Box::pin(async move {
            let before = tx.lock_account(id).await?;
            // Personal password changes historically compare the preverified
            // password after the lock. A kick without a password change must
            // not introduce a new account-version conflict here.
            if expected.is_some_and(|expected| {
                expected.id != before.id
                    || expected.credential().encoded() != before.credential.encoded()
            }) {
                return Err(AppError::conflict("密码已变更，请重新验证"));
            }
            let revoked_sessions = tx.revoke_all(id, Some(password)).await?;
            let after_version = tx.account_version(id).await?;
            Ok(PasswordChange {
                before_version: before.account_version,
                after_version,
                revoked_sessions,
            })
        })
    }

    fn set_status<'a>(
        &'a self,
        tx: &'a mut dyn AccountMutationTx,
        id: Uuid,
        status: &'a str,
    ) -> AccountFuture<'a, bool> {
        tx.set_status(id, status)
    }

    fn revoke_all<'a>(
        &'a self,
        tx: &'a mut dyn AccountMutationTx,
        id: Uuid,
    ) -> AccountFuture<'a, u64> {
        tx.revoke_all(id, None)
    }
}
