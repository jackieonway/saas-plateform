//! PostgreSQL account storage with caller-owned transaction composition.

use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::error::AppError;
use crate::platform_accounts::{
    AccountCredential, AccountFuture, AccountMutationTx, AccountSnapshot, CreatedAccount,
    CredentialHash, LoginIdentifier, NewAccount, PreparedPassword,
};

#[derive(sqlx::FromRow)]
struct CredentialRow {
    id: Uuid,
    username: String,
    password_hash: String,
    status: String,
    session_version: i64,
    real_name: String,
}

impl From<CredentialRow> for AccountCredential {
    fn from(row: CredentialRow) -> Self {
        Self::new(
            row.id,
            row.username,
            row.real_name,
            row.status,
            row.session_version,
            CredentialHash::new(row.password_hash),
        )
    }
}

pub(super) async fn candidates(
    pool: &PgPool,
    identifier: LoginIdentifier<'_>,
) -> Result<Vec<AccountCredential>, AppError> {
    let rows: Vec<CredentialRow> = sqlx::query_as(
        "SELECT id, username, password_hash, status, session_version, COALESCE(real_name,'') AS real_name \
         FROM users WHERE username=$1 OR ($2<>'' AND regexp_replace(phone, '[[:space:]()\\-]', '', 'g')=$2) \
         OR ($3<>'' AND lower(email)=$3) ORDER BY id LIMIT 65",
    )
    .bind(identifier.identifier)
    .bind(identifier.phone)
    .bind(identifier.email)
    .fetch_all(pool)
    .await
    .map_err(|_| AppError::Unavailable)?;
    Ok(rows.into_iter().map(AccountCredential::from).collect())
}

pub(super) async fn by_username(
    pool: &PgPool,
    username: &str,
) -> Result<Option<AccountCredential>, AppError> {
    let row: Option<CredentialRow> = sqlx::query_as(
        "SELECT id, username, password_hash, status, session_version, COALESCE(real_name,'') AS real_name \
         FROM users WHERE username=$1",
    )
    .bind(username)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(AccountCredential::from))
}

pub(super) async fn by_user_id(pool: &PgPool, id: Uuid) -> Result<AccountCredential, AppError> {
    let row: CredentialRow = sqlx::query_as(
        "SELECT id, username, password_hash, status, session_version, COALESCE(real_name,'') AS real_name \
         FROM users WHERE id=$1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    Ok(row.into())
}

/// Borrows the caller's connection without starting or committing a transaction.
/// The caller supplies its existing transaction connection and retains all
/// platform authorization, audit, notifications and final commit.
pub(crate) struct PostgresAccountTransaction<'a> {
    connection: &'a mut PgConnection,
}

impl<'a> PostgresAccountTransaction<'a> {
    pub(crate) fn new(connection: &'a mut PgConnection) -> Self {
        Self { connection }
    }

    /// Same-database consumer guard for a pending personal password command.
    /// Keep the caller's account lock while it rechecks its own session proof.
    /// This does not write, require a member, or restrict an administrator reset.
    pub(crate) async fn lock_verified_password_snapshot(
        &mut self,
        expected: &AccountCredential,
    ) -> Result<(), AppError> {
        let current = self.lock_account(expected.id).await?;
        if current.id != expected.id
            || current.credential.encoded() != expected.credential().encoded()
        {
            return Err(AppError::conflict("密码已变更，请重新验证"));
        }
        Ok(())
    }
}

impl AccountMutationTx for PostgresAccountTransaction<'_> {
    fn lock_account(&mut self, id: Uuid) -> AccountFuture<'_, AccountSnapshot> {
        Box::pin(async move {
            let (id, account_version, encoded): (Uuid, i64, String) = sqlx::query_as(
                "SELECT id,session_version,password_hash FROM users WHERE id=$1 FOR UPDATE",
            )
            .bind(id)
            .fetch_optional(&mut *self.connection)
            .await?
            .ok_or_else(|| AppError::not_found("用户不存在"))?;
            Ok(AccountSnapshot {
                id,
                account_version,
                credential: CredentialHash::new(encoded),
            })
        })
    }

    fn credential_matches<'a>(
        &'a mut self,
        credential: &'a AccountCredential,
    ) -> AccountFuture<'a, bool> {
        Box::pin(async move {
            // The caller already holds the original user-first issuance lock.
            // Capture the current account version later with membership policy;
            // a kick alone did not previously reject a freshly verified login.
            let current: Option<(String, String)> =
                sqlx::query_as("SELECT password_hash,status FROM users WHERE id=$1")
                    .bind(credential.id)
                    .fetch_optional(&mut *self.connection)
                    .await?;
            Ok(current.is_some_and(|(encoded, status)| {
                encoded == credential.credential().encoded() && status == "ENABLED"
            }))
        })
    }

    fn create<'a>(
        &'a mut self,
        account: NewAccount<'a>,
        password: &'a PreparedPassword,
    ) -> AccountFuture<'a, CreatedAccount> {
        Box::pin(async move {
            let (id, username): (Uuid, String) = sqlx::query_as(
                "INSERT INTO users (username, password_hash, real_name, email, phone, gender, id) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id, username",
            )
            .bind(account.username)
            .bind(password.encoded())
            .bind(account.real_name)
            .bind(account.email)
            .bind(account.phone)
            .bind(account.gender)
            .bind(account.id)
            .fetch_one(&mut *self.connection)
            .await?;
            Ok(CreatedAccount { id, username })
        })
    }

    fn set_status<'a>(&'a mut self, id: Uuid, status: &'a str) -> AccountFuture<'a, bool> {
        Box::pin(async move {
            // Keep the existing status trigger as the account-generation and
            // credential-revocation authority.
            let written =
                sqlx::query("UPDATE users SET status = $2, updated_at = now() WHERE id = $1")
                    .bind(id)
                    .bind(status)
                    .execute(&mut *self.connection)
                    .await?
                    .rows_affected();
            Ok(written > 0)
        })
    }

    fn revoke_all<'a>(
        &'a mut self,
        id: Uuid,
        password: Option<&'a PreparedPassword>,
    ) -> AccountFuture<'a, u64> {
        Box::pin(async move {
            revoke_account(
                &mut *self.connection,
                id,
                password.map(PreparedPassword::encoded),
            )
            .await
        })
    }

    fn account_version(&mut self, id: Uuid) -> AccountFuture<'_, i64> {
        Box::pin(async move {
            sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
                .bind(id)
                .fetch_optional(&mut *self.connection)
                .await?
                .ok_or_else(|| AppError::not_found("用户不存在"))
        })
    }
}

async fn revoke_account(
    connection: &mut PgConnection,
    id: Uuid,
    new_password_hash: Option<&str>,
) -> Result<u64, AppError> {
    let subject: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut *connection)
        .await?;
    if subject.is_none() {
        return Err(AppError::not_found("用户不存在"));
    }
    // Count sessions before the account-version trigger performs its defensive
    // sweep, preserving the existing revocation count in the caller's audit.
    let revoked = sqlx::query(
        "UPDATE platform_sessions SET revoked_at = now() \
         WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(id)
    .execute(&mut *connection)
    .await?
    .rows_affected();
    let updated = sqlx::query(
        "UPDATE users SET session_version = session_version + 1, \
         password_hash = COALESCE($2, password_hash), updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(new_password_hash)
    .execute(&mut *connection)
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(AppError::not_found("用户不存在"));
    }
    sqlx::query("UPDATE oauth_tokens SET revoked=true WHERE user_id=$1 AND revoked=false")
        .bind(id)
        .execute(&mut *connection)
        .await?;
    sqlx::query(
        "UPDATE oauth_authorization_codes SET consumed=true WHERE user_id=$1 AND consumed=false",
    )
    .bind(id)
    .execute(&mut *connection)
    .await?;
    sqlx::query(
        "UPDATE oauth_device_codes SET status='EXPIRED' WHERE user_id=$1 AND status IN ('PENDING','APPROVED')",
    )
    .bind(id)
    .execute(&mut *connection)
    .await?;
    Ok(revoked)
}

/// Test-only bridge for existing session fixtures; production uses the port.
#[cfg(test)]
pub(crate) async fn revoke_account_in_tx(
    connection: &mut PgConnection,
    id: Uuid,
    new_password_hash: Option<&str>,
) -> Result<u64, AppError> {
    revoke_account(connection, id, new_password_hash).await
}
