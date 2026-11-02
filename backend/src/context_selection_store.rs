//! PostgreSQL adapter for one-time member selection and bounded result recovery.
//!
//! Every mutation takes the caller's transaction: authorization revalidation,
//! old-context revocation, new-context creation and this consumption must commit
//! together. This adapter never authenticates a service or issues human tokens.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::context_selection::{
    Binding, Credential, Id, Member, MemberType, Selection, SelectionError, Version,
};

/// Storage failures preserve database causes without including credentials.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Domain binding, replay or lifetime fence failed.
    #[error(transparent)]
    Selection(#[from] SelectionError),
    /// The database operation failed; HTTP adapters redact this cause.
    #[error("context-selection storage unavailable")]
    Database(#[from] sqlx::Error),
    /// The bounded set of live selections is full.
    #[error("context-selection capacity reached")]
    Capacity,
    /// No credential belongs to this authenticated consumer.
    #[error("context-selection credential unavailable")]
    NotFound,
    /// Stored data does not satisfy the immutable contract.
    #[error("context-selection storage inconsistent")]
    Inconsistent,
}

/// Authenticated consumer identity, derived by the HTTP adapter or local BFF.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Consumer {
    /// Exact registered RP; an unrelated service cannot consume its selections.
    pub client_id: Id,
    /// Independent external service identity; None means the local platform BFF.
    pub service_id: Option<Id>,
}

/// Durable references only, never a plaintext token pair or a generic JSON blob.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum ContextResult {
    /// Existing employee business session created by this transaction.
    Employee {
        /// Employee platform session identity.
        #[serde(rename = "contextId")]
        context_id: Id,
        /// Resulting BFF version.
        version: Version,
    },
    /// Customer-specific resource context, never an employee session.
    Customer {
        /// Customer resource context identity.
        #[serde(rename = "contextId")]
        context_id: Id,
        /// Resulting portal operation generation.
        version: Version,
    },
}

impl ContextResult {
    fn matches(&self, member: &Member) -> bool {
        match (self, member.member_type) {
            (Self::Employee { version, .. }, MemberType::Employee)
            | (Self::Customer { version, .. }, MemberType::Customer) => version.value() >= 1,
            _ => false,
        }
    }
}

#[derive(sqlx::FromRow)]
struct Row {
    selection: String,
    consumed_at: Option<DateTime<Utc>>,
    result: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IssuedSelection {
    /// Original selection and deadline, never reconstructed with a later time.
    pub selection: Selection,
    /// Persisted digest used to check the encrypted recoverable credential.
    pub credential_hash: String,
    /// AEAD ciphertext; only the private platform authority adapter opens it.
    pub credential_ciphertext: String,
}

#[derive(sqlx::FromRow)]
struct IssuedRow {
    selection: String,
    client_id: Uuid,
    consumer_service_id: Option<Uuid>,
    credential_hash: String,
    credential_ciphertext: Option<String>,
    expires_at: DateTime<Utc>,
    consumed_at: Option<DateTime<Utc>>,
}

async fn clock(tx: &mut Transaction<'_, Postgres>) -> Result<DateTime<Utc>, StoreError> {
    Ok(sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut **tx)
        .await?)
}

/// Serialize selection capacity decisions and same-transaction recovery.
pub async fn lock_issue_capacity(tx: &mut Transaction<'_, Postgres>) -> Result<(), StoreError> {
    sqlx::query(
        "SELECT pg_advisory_xact_lock(hashtextextended('context-selection-capacity-v1',0))",
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Recover an unconsumed credential only for the same original binding,
/// selected membership, RP and service while every original deadline is live.
pub async fn recover_issued(
    tx: &mut Transaction<'_, Postgres>,
    transaction_id: Id,
    binding: &Binding,
    member: &Member,
    consumer: Consumer,
    current_deadline: DateTime<Utc>,
) -> Result<Option<IssuedSelection>, StoreError> {
    let existing: Option<IssuedRow> = sqlx::query_as(
        "SELECT selection::text,client_id,consumer_service_id,credential_hash, \
         credential_ciphertext,expires_at,consumed_at FROM context_selections \
         WHERE transaction_id=$1 FOR UPDATE",
    )
    .bind(transaction_id.uuid())
    .fetch_optional(&mut **tx)
    .await?;
    let Some(IssuedRow {
        selection: raw,
        client_id,
        consumer_service_id: service_id,
        credential_hash,
        credential_ciphertext,
        expires_at,
        consumed_at,
    }) = existing
    else {
        return Ok(None);
    };
    let selection: Selection = serde_json::from_str(&raw).map_err(|_| StoreError::Inconsistent)?;
    if client_id != consumer.client_id.uuid()
        || service_id != consumer.service_id.map(Id::uuid)
        || selection.expires_at != expires_at
        || expires_at > current_deadline
    {
        return Err(SelectionError::Changed.into());
    }
    selection.check_consumption(binding, member, clock(tx).await?, consumed_at.is_some())?;
    let credential_ciphertext = credential_ciphertext.ok_or(StoreError::Inconsistent)?;
    Ok(Some(IssuedSelection {
        selection,
        credential_hash,
        credential_ciphertext,
    }))
}

/// Save a newly confirmed selection under a bounded global capacity lock.
/// `selection` must have been assembled from fresh platform and identity proofs.
/// Caller supplies the newly random credential and a consumer resolved from the
/// server registration, not from confirmation form fields.
pub async fn issue(
    tx: &mut Transaction<'_, Postgres>,
    selection: &Selection,
    credential: &Credential,
    credential_ciphertext: &str,
    consumer: Consumer,
    capacity: u32,
) -> Result<Selection, StoreError> {
    let now = clock(tx).await?;
    if capacity == 0 || consumer.client_id != selection.binding.registration.client_id {
        return Err(SelectionError::Invalid.into());
    }
    selection
        .binding
        .validate(&selection.binding.source.issuer, now)?;
    selection.check_consumption(&selection.binding, &selection.member, now, false)?;
    if selection.expires_at > selection.created_at + chrono::Duration::seconds(120) {
        return Err(SelectionError::Invalid.into());
    }
    lock_issue_capacity(tx).await?;
    // A consumed result is a durable source/member fact and switch fence, even
    // after the one-time credential's recovery deadline. Only abandoned flows
    // are removed here; active-capacity accounting remains deadline bounded.
    sqlx::query("DELETE FROM context_selections WHERE transaction_id IN (SELECT transaction_id FROM context_selections WHERE consumed_at IS NULL AND expires_at<=clock_timestamp() ORDER BY expires_at LIMIT 256)")
        .execute(&mut **tx).await?;
    if let Some(existing) = recover_issued(
        tx,
        selection.binding.transaction_id,
        &selection.binding,
        &selection.member,
        consumer,
        selection.expires_at,
    )
    .await?
    {
        if existing.credential_hash == credential.digest()
            && existing.credential_ciphertext == credential_ciphertext
        {
            return Ok(existing.selection);
        }
        return Err(SelectionError::Changed.into());
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM context_selections")
        .fetch_one(&mut **tx)
        .await?;
    if count >= i64::from(capacity) {
        return Err(StoreError::Capacity);
    }
    let encoded = serde_json::to_string(selection).map_err(|_| StoreError::Inconsistent)?;
    sqlx::query("INSERT INTO context_selections(transaction_id,interaction_id,client_id,consumer_service_id,credential_hash,credential_ciphertext,selection,created_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7::jsonb,$8,$9)")
        .bind(selection.binding.transaction_id.uuid())
        .bind(selection.binding.source.interaction_id.uuid())
        .bind(consumer.client_id.uuid())
        .bind(consumer.service_id.map(Id::uuid))
        .bind(credential.digest())
        .bind(credential_ciphertext)
        .bind(encoded)
        .bind(selection.created_at)
        .bind(selection.expires_at)
        .execute(&mut **tx).await?;
    Ok(selection.clone())
}

async fn load(
    tx: &mut Transaction<'_, Postgres>,
    credential: &Credential,
    consumer: Consumer,
) -> Result<(Selection, Row), StoreError> {
    let row: Row = sqlx::query_as("SELECT selection::text,consumed_at,result::text FROM context_selections WHERE credential_hash=$1 AND client_id=$2 AND consumer_service_id IS NOT DISTINCT FROM $3::uuid FOR UPDATE")
        .bind(credential.digest()).bind(consumer.client_id.uuid()).bind(consumer.service_id.map(Id::uuid))
        .fetch_optional(&mut **tx).await?.ok_or(StoreError::NotFound)?;
    let selection: Selection =
        serde_json::from_str(&row.selection).map_err(|_| StoreError::Inconsistent)?;
    if selection.binding.registration.client_id != consumer.client_id {
        return Err(StoreError::Inconsistent);
    }
    Ok((selection, row))
}

/// Commit a reference to the context established in this same transaction.
/// The caller must revalidate `binding` and `member` against online authority
/// with the existing lock order before calling this function. Any error requires
/// rolling back the entire caller transaction, including context/token writes.
pub async fn consume(
    tx: &mut Transaction<'_, Postgres>,
    credential: &Credential,
    consumer: Consumer,
    binding: &Binding,
    member: &Member,
    result: &ContextResult,
    result_ciphertext: Option<&str>,
) -> Result<(), StoreError> {
    let (selection, row) = load(tx, credential, consumer).await?;
    let now = clock(tx).await?;
    selection.check_consumption(binding, member, now, row.consumed_at.is_some())?;
    if !result.matches(member) || row.result.is_some() {
        return Err(SelectionError::Changed.into());
    }
    let result = serde_json::to_string(result).map_err(|_| StoreError::Inconsistent)?;
    let changed = sqlx::query("UPDATE context_selections SET consumed_at=clock_timestamp(),result=$2::jsonb,result_ciphertext=$3,credential_ciphertext=NULL WHERE credential_hash=$1 AND consumed_at IS NULL AND expires_at>clock_timestamp()")
        .bind(credential.digest()).bind(result).bind(result_ciphertext).execute(&mut **tx).await?.rows_affected();
    if changed != 1 {
        return Err(SelectionError::Expired.into());
    }
    Ok(())
}

/// Read the existing result without minting a new context or extending deadlines.
/// The caller must authenticate the original consumer, freshly verify the source
/// and member, then revalidate the returned resulting context before responding.
pub async fn recover(
    tx: &mut Transaction<'_, Postgres>,
    credential: &Credential,
    consumer: Consumer,
    binding: &Binding,
    member: &Member,
) -> Result<ContextResult, StoreError> {
    let (selection, row) = load(tx, credential, consumer).await?;
    selection.check_recovery(binding, member, clock(tx).await?, row.consumed_at.is_some())?;
    let result: ContextResult =
        serde_json::from_str(row.result.as_deref().ok_or(StoreError::Inconsistent)?)
            .map_err(|_| StoreError::Inconsistent)?;
    if !result.matches(member) {
        return Err(StoreError::Inconsistent);
    }
    Ok(result)
}
