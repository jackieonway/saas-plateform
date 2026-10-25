//! Explicit Phase 6 contraction and environment/binary-bound startup admission.
//!
//! The canonical migration chain never drops the binary membership key. This
//! module is owned by the rollout command; service startup only reads receipts.

use std::{error::Error, ffi::OsString, fmt, path::PathBuf, time::Duration};

use serde::Serialize;
use sqlx::{postgres::PgPoolOptions, PgPool};
use uuid::{Uuid, Variant};

mod evidence;
mod schema;

const CONTRACTION_SQL: &str = include_str!("../../ops/identity/phase-06-contract-memberships.sql");
const RECEIPT_SQL: &str = include_str!("../../ops/identity/phase-06-readiness-receipts.sql");
const LOCK_ID: i64 = 0x4944454e54595436;

#[derive(Clone, Copy)]
enum Failure {
    Arguments,
    Configuration,
    Ambient,
    Connection,
    Evidence,
    Binary,
    History,
    Schema,
    ForeignKey,
    Receipt,
    Target,
    Timeout,
    Close,
}

/// A fixed-category rollout/admission failure. Formatting never includes a
/// database URL, artifact path, credential, manifest contents or driver detail.
pub struct MembershipReadinessError {
    failure: Failure,
    source: Option<anyhow::Error>,
}

fn fail(failure: Failure) -> MembershipReadinessError {
    MembershipReadinessError {
        failure,
        source: None,
    }
}
fn caused(failure: Failure, source: impl Into<anyhow::Error>) -> MembershipReadinessError {
    MembershipReadinessError {
        failure,
        source: Some(source.into()),
    }
}

impl fmt::Display for MembershipReadinessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.failure {
            Failure::Arguments => "MEMBERSHIP_ROLLOUT_ARGUMENTS_INVALID: expected inspect --platform-binary PATH --environment development|production, or apply --manifest PATH --platform-binary PATH",
            Failure::Configuration => "MEMBERSHIP_READINESS_CONFIGURATION_INVALID",
            Failure::Ambient => "MEMBERSHIP_ROLLOUT_AMBIENT_PG_CONFIGURATION_UNSUPPORTED",
            Failure::Connection => "MEMBERSHIP_ROLLOUT_DATABASE_CONNECTION_FAILED",
            Failure::Evidence => "MEMBERSHIP_ROLLOUT_EVIDENCE_INVALID",
            Failure::Binary => "MEMBERSHIP_ROLLOUT_PLATFORM_BINARY_INVALID",
            Failure::History => "MEMBERSHIP_ROLLOUT_MIGRATION_HISTORY_INVALID",
            Failure::Schema => "MEMBERSHIP_ROLLOUT_SCHEMA_INVALID",
            Failure::ForeignKey => "MEMBERSHIP_ROLLOUT_BINARY_KEY_HAS_DEPENDENT_FOREIGN_KEYS",
            Failure::Receipt => "MEMBERSHIP_READINESS_RECEIPT_INVALID",
            Failure::Target => "MEMBERSHIP_ROLLOUT_TARGET_MISMATCH",
            Failure::Timeout => "MEMBERSHIP_ROLLOUT_TIMED_OUT",
            Failure::Close => "MEMBERSHIP_ROLLOUT_DATABASE_CLOSE_TIMED_OUT",
        })
    }
}
impl fmt::Debug for MembershipReadinessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl Error for MembershipReadinessError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_ref().map(|source| source.as_ref())
    }
}

/// Receipt requested by a service environment. Construction only validates the
/// explicit receipt identity; [`Self::verify`] checks the live database before
/// seed, workers or HTTP. Test fixture compatibility is decided by the caller.
#[derive(Clone)]
pub struct MembershipReadiness {
    receipt: Uuid,
    manifest_sha256: String,
    environment: String,
}

impl MembershipReadiness {
    /// Require a canonical RFC UUIDv7, lowercase manifest SHA256 and an exact
    /// deployable environment name. This does not confer database admission.
    pub fn new(
        receipt: &str,
        manifest_sha256: &str,
        environment: &str,
    ) -> Result<Self, MembershipReadinessError> {
        let id =
            Uuid::parse_str(receipt).map_err(|source| caused(Failure::Configuration, source))?;
        if id.get_version_num() != 7
            || id.get_variant() != Variant::RFC4122
            || id.to_string() != receipt
            || !evidence::lower_sha256(manifest_sha256)
            || !matches!(environment, "development" | "production")
        {
            return Err(fail(Failure::Configuration));
        }
        Ok(Self {
            receipt: id,
            manifest_sha256: manifest_sha256.into(),
            environment: environment.into(),
        })
    }

    /// Read-only repeatable-read verification of the live contracted schema,
    /// full SQLx history, environment, database identity and current executable.
    /// Receipt and manifest contents are never logged or loaded from dotenv.
    pub async fn verify(&self, pool: &PgPool) -> Result<(), MembershipReadinessError> {
        let path = std::env::current_exe().map_err(|source| caused(Failure::Binary, source))?;
        let binary = tokio::task::spawn_blocking(move || evidence::binary_sha256(&path))
            .await
            .map_err(|source| caused(Failure::Binary, source))??;
        self.verify_binary(pool, &binary).await
    }

    async fn verify_binary(
        &self,
        pool: &PgPool,
        binary: &str,
    ) -> Result<(), MembershipReadinessError> {
        let mut transaction = pool
            .begin()
            .await
            .map_err(|source| caused(Failure::Connection, source))?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *transaction)
            .await
            .map_err(|source| caused(Failure::Receipt, source))?;
        sqlx::query("SELECT pg_catalog.set_config('search_path','public,pg_catalog,pg_temp',true),pg_catalog.set_config('statement_timeout','30000ms',true),pg_catalog.set_config('lock_timeout','5000ms',true)")
            .execute(&mut *transaction).await.map_err(|source| caused(Failure::Receipt, source))?;
        if !schema::receipt_table(&mut transaction).await? {
            return Err(fail(Failure::Receipt));
        }
        let shape = schema::inspect(&mut transaction).await?;
        if shape.binary_key {
            return Err(fail(Failure::Receipt));
        }
        let receipt: Option<Receipt> = sqlx::query_as(
            "SELECT environment,scope,database_name,database_oid,cluster_system_identifier,platform_binary_sha256,manifest_sha256,migration_sha256,schema_sha256 FROM public.identity_membership_readiness_receipts WHERE id=$1",
        ).bind(self.receipt).fetch_optional(&mut *transaction).await.map_err(|source| caused(Failure::Receipt, source))?;
        let receipt = receipt.ok_or_else(|| fail(Failure::Receipt))?;
        let digest = schema::digest(&mut transaction).await?;
        if receipt.environment != self.environment
            || receipt.database_name != shape.database
            || receipt.database_oid != shape.database_oid
            || receipt.platform_binary_sha256 != binary
            || receipt.cluster_system_identifier != shape.cluster_system_identifier
            || receipt.manifest_sha256 != self.manifest_sha256
            || receipt.migration_sha256 != shape.migration_sha256
            || receipt.schema_sha256 != digest
            || !matches!(
                (receipt.environment.as_str(), receipt.scope.as_str()),
                ("development", "TASK_OWNED") | ("production", "PRODUCTION")
            )
        {
            return Err(fail(Failure::Receipt));
        }
        transaction
            .commit()
            .await
            .map_err(|source| caused(Failure::Receipt, source))?;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct Receipt {
    environment: String,
    scope: String,
    database_name: String,
    database_oid: i64,
    cluster_system_identifier: String,
    platform_binary_sha256: String,
    manifest_sha256: String,
    migration_sha256: String,
    schema_sha256: String,
}

enum Action {
    Inspect { environment: String },
    Apply { manifest: PathBuf },
}
struct Arguments {
    action: Action,
    platform_binary: PathBuf,
}

fn arguments(
    values: impl IntoIterator<Item = OsString>,
) -> Result<Arguments, MembershipReadinessError> {
    let mut values = values.into_iter();
    let action = values.next().ok_or_else(|| fail(Failure::Arguments))?;
    let mut binary = None;
    let mut manifest = None;
    let mut environment = None;
    while let Some(option) = values.next() {
        let value = values.next().ok_or_else(|| fail(Failure::Arguments))?;
        if value.is_empty() {
            return Err(fail(Failure::Arguments));
        }
        match option.to_str() {
            Some("--platform-binary") if binary.is_none() => binary = Some(PathBuf::from(value)),
            Some("--manifest") if manifest.is_none() => manifest = Some(PathBuf::from(value)),
            Some("--environment") if environment.is_none() => {
                environment = Some(value.into_string().map_err(|_| fail(Failure::Arguments))?);
            }
            _ => return Err(fail(Failure::Arguments)),
        }
    }
    let action = match (action.to_str(), manifest, environment) {
        (Some("inspect"), None, Some(environment))
            if matches!(environment.as_str(), "development" | "production") =>
        {
            Action::Inspect { environment }
        }
        (Some("apply"), Some(manifest), None) => Action::Apply { manifest },
        _ => return Err(fail(Failure::Arguments)),
    };
    Ok(Arguments {
        action,
        platform_binary: binary.ok_or_else(|| fail(Failure::Arguments))?,
    })
}

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    operation: &'static str,
    environment: String,
    database: String,
    database_oid: i64,
    cluster_system_identifier: String,
    platform_binary_sha256: String,
    binary_key_present: bool,
    migration_sha256: String,
    schema_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    receipt_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    manifest_sha256: Option<String>,
}

async fn operate(pool: &PgPool, args: Arguments) -> Result<String, MembershipReadinessError> {
    let path = args.platform_binary;
    let binary = tokio::task::spawn_blocking(move || evidence::binary_sha256(&path))
        .await
        .map_err(|source| caused(Failure::Binary, source))??;
    let evidence = match &args.action {
        Action::Apply { manifest } => {
            let manifest = manifest.clone();
            Some(
                tokio::task::spawn_blocking(move || evidence::load(&manifest))
                    .await
                    .map_err(|source| caused(Failure::Evidence, source))??,
            )
        }
        _ => None,
    };
    let mut transaction = pool
        .begin()
        .await
        .map_err(|source| caused(Failure::Connection, source))?;
    match args.action {
        Action::Inspect { .. } => {
            sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
                .execute(&mut *transaction)
                .await
                .map_err(|source| caused(Failure::Schema, source))?;
        }
        Action::Apply { .. } => {
            sqlx::query("SELECT pg_catalog.pg_advisory_xact_lock($1)")
                .bind(LOCK_ID)
                .execute(&mut *transaction)
                .await
                .map_err(|source| caused(Failure::Schema, source))?;
            sqlx::query("LOCK TABLE public.memberships IN ACCESS EXCLUSIVE MODE")
                .execute(&mut *transaction)
                .await
                .map_err(|source| caused(Failure::Schema, source))?;
        }
    }
    let shape = schema::inspect(&mut transaction).await?;
    let table_exists = schema::receipt_table(&mut transaction).await?;
    let mut report = Report {
        schema_version: 1,
        operation: "inspect",
        environment: match &args.action {
            Action::Inspect { environment } => environment.clone(),
            Action::Apply { .. } => evidence
                .as_ref()
                .ok_or_else(|| fail(Failure::Evidence))?
                .environment
                .clone(),
        },
        database: shape.database.clone(),
        database_oid: shape.database_oid,
        cluster_system_identifier: shape.cluster_system_identifier.clone(),
        platform_binary_sha256: binary.clone(),
        binary_key_present: shape.binary_key,
        migration_sha256: shape.migration_sha256.clone(),
        schema_sha256: String::new(),
        receipt_id: None,
        manifest_sha256: None,
    };
    if let Some(evidence) = evidence {
        if evidence.target_database != shape.database || evidence.platform_binary_sha256 != binary {
            return Err(fail(Failure::Target));
        }
        if evidence.target_database_oid != shape.database_oid
            || evidence.target_cluster_system_identifier != shape.cluster_system_identifier
        {
            return Err(fail(Failure::Target));
        }
        // First apply must find the known expansion schema. Renewal is explicit
        // for a rebuilt executable; an unreceipted hand-edited contraction fails.
        if !shape.binary_key {
            if !table_exists {
                return Err(fail(Failure::Receipt));
            }
            let prior: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.identity_membership_readiness_receipts WHERE database_name=$1 AND database_oid=$2 AND environment=$3)")
                .bind(&shape.database).bind(shape.database_oid).bind(&evidence.environment)
                .fetch_one(&mut *transaction).await.map_err(|source| caused(Failure::Receipt, source))?;
            if !prior {
                return Err(fail(Failure::Receipt));
            }
        } else {
            if table_exists {
                return Err(fail(Failure::Receipt));
            }
            sqlx::raw_sql(CONTRACTION_SQL)
                .execute(&mut *transaction)
                .await
                .map_err(|source| caused(Failure::Schema, source))?;
        }
        if !table_exists {
            sqlx::raw_sql(RECEIPT_SQL)
                .execute(&mut *transaction)
                .await
                .map_err(|source| caused(Failure::Schema, source))?;
        }
        if !schema::receipt_table(&mut transaction).await? {
            return Err(fail(Failure::Schema));
        }
        let contracted = schema::inspect(&mut transaction).await?;
        if contracted.binary_key {
            return Err(fail(Failure::Schema));
        }
        report.schema_sha256 = schema::digest(&mut transaction).await?;
        let id = Uuid::now_v7();
        sqlx::query("INSERT INTO public.identity_membership_readiness_receipts(id,environment,scope,database_name,database_oid,cluster_system_identifier,platform_binary_sha256,manifest_sha256,migration_sha256,schema_sha256,inventory,gates) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)")
            .bind(id).bind(&evidence.environment).bind(&evidence.scope).bind(&shape.database).bind(shape.database_oid)
            .bind(&shape.cluster_system_identifier)
            .bind(&binary).bind(&evidence.manifest_sha256).bind(&shape.migration_sha256).bind(&report.schema_sha256)
            .bind(&evidence.inventory).bind(&evidence.gates)
            .execute(&mut *transaction).await.map_err(|source| caused(Failure::Receipt, source))?;
        report.operation = "apply";
        report.binary_key_present = false;
        report.receipt_id = Some(id);
        report.manifest_sha256 = Some(evidence.manifest_sha256);
    } else {
        report.schema_sha256 = schema::digest(&mut transaction).await?;
    }
    transaction
        .commit()
        .await
        .map_err(|source| caused(Failure::Receipt, source))?;
    serde_json::to_string(&report).map_err(|source| caused(Failure::Receipt, source))
}

/// Inspect or explicitly contract one existing database. Only process
/// `MEMBERSHIP_ROLLOUT_DATABASE_URL` is read; ambient `PG*`, dotenv, seed, workers, HTTP and automatic
/// migration execution are excluded. Apply requires an artifact-backed manifest,
/// takes advisory/table locks, checks FK dependencies, then records its receipt
/// atomically with the separate contraction SQL. Inspect is read-only.
///
/// The JSON report contains the receipt ID, hashes and target identity but no
/// credentials or evidence contents. A receipt proves only the declared scope;
/// task-owned validation must never be presented as production inventory proof.
pub async fn run_membership_rollout_command(
    values: impl IntoIterator<Item = OsString>,
) -> Result<String, MembershipReadinessError> {
    let args = arguments(values)?;
    if std::env::vars_os()
        .any(|(key, _)| key.to_string_lossy().to_ascii_uppercase().starts_with("PG"))
    {
        return Err(fail(Failure::Ambient));
    }
    let raw = std::env::var("MEMBERSHIP_ROLLOUT_DATABASE_URL")
        .map_err(|source| caused(Failure::Configuration, source))?;
    let options = super::migrate_only::database_options(&raw)
        .map_err(|source| caused(Failure::Configuration, source))?;
    let pool=PgPoolOptions::new().max_connections(1).acquire_timeout(Duration::from_secs(10))
        .after_connect(|connection,_| Box::pin(async move {
            sqlx::query("SELECT pg_catalog.set_config('search_path','public,pg_catalog,pg_temp',false),pg_catalog.set_config('statement_timeout','300000ms',false),pg_catalog.set_config('lock_timeout','30000ms',false),pg_catalog.set_config('idle_in_transaction_session_timeout','15000ms',false)")
                .execute(connection).await?;
            Ok(())
        })).connect_with(options).await.map_err(|source| caused(Failure::Connection, source))?;
    let result = tokio::time::timeout(Duration::from_secs(300), operate(&pool, args))
        .await
        .map_err(|source| caused(Failure::Timeout, source))
        .and_then(|result| result);
    let closed = tokio::time::timeout(Duration::from_secs(10), pool.close()).await;
    let report = result?;
    closed.map_err(|source| caused(Failure::Close, source))?;
    Ok(report)
}

#[cfg(test)]
mod tests;
