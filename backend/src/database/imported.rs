//! Read-only admission of an authenticated, independently migrated import target.
use std::{error::Error, fmt};

use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool};

const SCHEMA_MATERIAL: &str = include_str!("schema_material.sql");

#[derive(Clone, Copy)]
enum Failure {
    Configuration,
    Identity,
    History,
    Schema,
    Database,
}

/// A sanitized failure while admitting an imported database. Display and Debug
/// contain only fixed categories; driver causes remain in [`Error::source`].
pub struct ImportedDatabaseError {
    failure: Failure,
    source: Option<anyhow::Error>,
}

impl ImportedDatabaseError {
    fn new(failure: Failure) -> Self {
        Self {
            failure,
            source: None,
        }
    }

    fn caused_by(source: impl Into<anyhow::Error>) -> Self {
        Self {
            failure: Failure::Database,
            source: Some(source.into()),
        }
    }
}

impl fmt::Display for ImportedDatabaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.failure {
            Failure::Configuration => "IMPORTED_DATABASE_CONFIGURATION_INVALID",
            Failure::Identity => "IMPORTED_DATABASE_IDENTITY_REJECTED",
            Failure::History => "IMPORTED_DATABASE_MIGRATION_HISTORY_REJECTED",
            Failure::Schema => "IMPORTED_DATABASE_SCHEMA_REJECTED",
            Failure::Database => "IMPORTED_DATABASE_VERIFICATION_FAILED",
        })
    }
}

impl fmt::Debug for ImportedDatabaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Error for ImportedDatabaseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_ref().map(|source| source.as_ref())
    }
}

/// The reviewed identity, import marker and complete structural digest of an
/// already initialized target. This is an admission contract, not permission to
/// import data or evidence that runtime credentials have been rotated.
#[derive(Clone)]
pub struct ImportedDatabase {
    database: String,
    marker: String,
    schema_sha256: String,
}

impl ImportedDatabase {
    /// Validate explicit, trusted import handoff fields without opening a database.
    /// The marker is `saas-uuid-import/1/<canonical UUIDv7>/<64 lowercase hex>`;
    /// the schema digest is the authenticated import's ordered `schema_material`.
    pub fn new(
        database: &str,
        marker: &str,
        schema_sha256: &str,
    ) -> Result<Self, ImportedDatabaseError> {
        if database.is_empty()
            || database.len() > 63
            || database.chars().any(char::is_control)
            || database.contains('/')
            || matches!(database, "postgres" | "template0" | "template1")
            || !valid_marker(marker)
            || !lower_sha256(schema_sha256)
        {
            return Err(ImportedDatabaseError::new(Failure::Configuration));
        }
        Ok(Self {
            database: database.into(),
            marker: marker.into(),
            schema_sha256: schema_sha256.into(),
        })
    }

    /// Require an explicit URL whose decoded database is the reviewed target.
    /// Connection identity overrides and arbitrary startup options are rejected;
    /// transport/TLS parameters use the service's existing explicit URL settings.
    pub fn validate_url(&self, raw: &str) -> Result<(), ImportedDatabaseError> {
        let invalid = || ImportedDatabaseError::new(Failure::Configuration);
        let url = reqwest::Url::parse(raw).map_err(|_| invalid())?;
        if !matches!(url.scheme(), "postgres" | "postgresql")
            || url.host_str().is_none_or(str::is_empty)
            || url.username().is_empty()
            || url.fragment().is_some()
            || url.port() == Some(0)
            || url.path().trim_start_matches('/').contains('/')
        {
            return Err(invalid());
        }
        let decoded = |value: &str| {
            let value = percent_encoding::percent_decode_str(value)
                .decode_utf8()
                .map_err(|_| invalid())?;
            if value.chars().any(char::is_control) {
                return Err(invalid());
            }
            Ok(value.into_owned())
        };
        if decoded(url.path().trim_start_matches('/'))? != self.database {
            return Err(invalid());
        }
        decoded(url.username())?;
        decoded(url.password().unwrap_or(""))?;
        let mut seen = std::collections::BTreeSet::new();
        for (key, value) in url.query_pairs() {
            let category = match key.as_ref() {
                "sslmode" | "ssl-mode" => "sslmode",
                "sslrootcert" | "ssl-root-cert" | "ssl-ca" => "sslrootcert",
                "sslcert" | "ssl-cert" => "sslcert",
                "sslkey" | "ssl-key" => "sslkey",
                "application_name" => "application_name",
                _ => return Err(invalid()),
            };
            if value.is_empty() || value.chars().any(char::is_control) || !seen.insert(category) {
                return Err(invalid());
            }
        }
        Ok(())
    }

    /// Admit a DML-only LOGIN role and verify the import handoff in one read-only,
    /// repeatable-read transaction. This executes no migrations, DDL, seed,
    /// business row scans or credential updates. The caller must finish this before workers
    /// and HTTP are constructed and must independently rotate runtime credentials.
    pub async fn verify(&self, pool: &PgPool) -> Result<(), ImportedDatabaseError> {
        let mut transaction = pool
            .begin()
            .await
            .map_err(ImportedDatabaseError::caused_by)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *transaction)
            .await
            .map_err(ImportedDatabaseError::caused_by)?;
        sqlx::query("SELECT pg_catalog.set_config('search_path','pg_catalog',true), pg_catalog.set_config('timezone','UTC',true), pg_catalog.set_config('row_security','off',true)")
            .execute(&mut *transaction).await.map_err(ImportedDatabaseError::caused_by)?;
        let proof: Admission = sqlx::query_as(ADMISSION_SQL)
            .fetch_one(&mut *transaction)
            .await
            .map_err(ImportedDatabaseError::caused_by)?;
        self.validate_admission(&proof)?;
        verify_history(&mut transaction).await?;
        let material: serde_json::Value = sqlx::query_scalar(SCHEMA_MATERIAL)
            .fetch_one(&mut *transaction)
            .await
            .map_err(ImportedDatabaseError::caused_by)?;
        let bytes = serde_json::to_vec(&material).map_err(ImportedDatabaseError::caused_by)?;
        if !material.is_array() || hex::encode(Sha256::digest(bytes)) != self.schema_sha256 {
            return Err(ImportedDatabaseError::new(Failure::Schema));
        }
        transaction
            .commit()
            .await
            .map_err(ImportedDatabaseError::caused_by)
    }

    fn validate_admission(&self, proof: &Admission) -> Result<(), ImportedDatabaseError> {
        if proof.database != self.database
            || proof.marker.as_deref() != Some(&self.marker)
            || !proof.principal_valid
            || !proof.owner_valid
            || !proof.no_ddl
        {
            return Err(ImportedDatabaseError::new(Failure::Identity));
        }
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct Admission {
    database: String,
    marker: Option<String>,
    principal_valid: bool,
    owner_valid: bool,
    no_ddl: bool,
}

const ADMISSION_SQL: &str = r#"
WITH me AS (
    SELECT oid,rolcanlogin,rolsuper,rolcreatedb,rolcreaterole,rolreplication,rolbypassrls
    FROM pg_catalog.pg_roles WHERE rolname=current_user
), db AS (
    SELECT oid,datname,datdba FROM pg_catalog.pg_database WHERE datname=current_database()
), user_schemas AS (
    SELECT oid,nspowner FROM pg_catalog.pg_namespace
    WHERE nspname<>'information_schema' AND pg_catalog.left(nspname::text,3)<>'pg_'
), protected_roles AS (
    SELECT datdba AS oid FROM db
    UNION SELECT nspowner FROM user_schemas
    UNION SELECT relowner FROM pg_catalog.pg_class WHERE relnamespace IN (SELECT oid FROM user_schemas)
    UNION SELECT proowner FROM pg_catalog.pg_proc WHERE pronamespace IN (SELECT oid FROM user_schemas)
    UNION SELECT typowner FROM pg_catalog.pg_type WHERE typnamespace IN (SELECT oid FROM user_schemas)
)
SELECT db.datname::text AS database,
    pg_catalog.shobj_description(db.oid,'pg_database'::pg_catalog.name) AS marker,
    (current_user=session_user AND r.rolcanlogin AND NOT r.rolsuper AND NOT r.rolcreatedb AND NOT r.rolcreaterole AND NOT r.rolreplication AND NOT r.rolbypassrls) AS principal_valid,
    (NOT o.rolcanlogin AND NOT o.rolsuper AND
        NOT EXISTS(SELECT 1 FROM protected_roles p WHERE pg_catalog.pg_has_role(r.oid,p.oid,'SET'::text) OR pg_catalog.pg_has_role(r.oid,p.oid,'USAGE'::text)) AND
        NOT EXISTS(SELECT 1 FROM pg_catalog.pg_auth_members m WHERE m.admin_option AND (pg_catalog.pg_has_role(r.oid,m.member,'SET'::text) OR pg_catalog.pg_has_role(r.oid,m.member,'USAGE'::text)))) AS owner_valid,
    (NOT EXISTS(SELECT 1 FROM pg_catalog.pg_roles p WHERE (pg_catalog.pg_has_role(r.oid,p.oid,'SET'::text) OR pg_catalog.pg_has_role(r.oid,p.oid,'USAGE'::text)) AND
        (p.rolsuper OR p.rolcreatedb OR p.rolcreaterole OR p.rolreplication OR p.rolbypassrls OR pg_catalog.has_database_privilege(p.oid,db.oid,'CREATE'::text) OR pg_catalog.has_database_privilege(p.oid,db.oid,'TEMPORARY'::text) OR
         EXISTS(SELECT 1 FROM pg_catalog.pg_namespace n WHERE n.nspname::text OPERATOR(pg_catalog.!~) '^pg_(toast_)?temp_'::text AND pg_catalog.has_schema_privilege(p.oid,n.oid,'CREATE'::text))))) AS no_ddl
FROM me r CROSS JOIN db JOIN pg_catalog.pg_roles o ON o.oid=db.datdba
"#;

async fn verify_history(connection: &mut PgConnection) -> Result<(), ImportedDatabaseError> {
    let rows: Vec<(i64, bool, Vec<u8>)> = sqlx::query_as(
        "SELECT version,success,checksum FROM public._sqlx_migrations ORDER BY version",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(ImportedDatabaseError::caused_by)?;
    let applied: Vec<_> = rows
        .iter()
        .map(|(version, _, checksum)| sqlx::migrate::AppliedMigration {
            version: *version,
            checksum: std::borrow::Cow::Owned(checksum.clone()),
        })
        .collect();
    let migrator = super::compatible_migrator(&applied);
    let expected: Vec<_> = migrator
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
        .collect();
    if rows.len() != expected.len()
        || rows
            .iter()
            .zip(expected)
            .any(|((version, success, checksum), migration)| {
                *version != migration.version
                    || !success
                    || checksum.as_slice() != migration.checksum.as_ref()
            })
    {
        return Err(ImportedDatabaseError::new(Failure::History));
    }
    Ok(())
}

fn lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn valid_marker(marker: &str) -> bool {
    let Some(body) = marker.strip_prefix("saas-uuid-import/1/") else {
        return false;
    };
    let Some((batch, digest)) = body.split_once('/') else {
        return false;
    };
    let Ok(uuid) = uuid::Uuid::parse_str(batch) else {
        return false;
    };
    uuid.get_version_num() == 7
        && uuid.get_variant() == uuid::Variant::RFC4122
        && uuid.to_string() == batch
        && lower_sha256(digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    const MARKER: &str = "saas-uuid-import/1/019a1163-0000-7000-8000-000000000001/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const HASH: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn imported_contract_requires_canonical_batch_and_exact_target_url() {
        let contract = ImportedDatabase::new("isolated_target", MARKER, HASH).unwrap();
        assert!(contract
            .validate_url(
                "postgresql://service:private@127.0.0.1:5432/isolated_target?sslmode=disable"
            )
            .is_ok());
        for url in [
            "postgresql://service@127.0.0.1/old_source",
            "postgresql://service@127.0.0.1/isolated_target?dbname=old_source",
            "postgresql://service@127.0.0.1/isolated_target?options[role]=postgres",
            "postgresql://service@127.0.0.1/isolated_target%00options%00-c%20role=postgres",
            "postgresql://service:private%00@127.0.0.1/isolated_target",
        ] {
            assert!(contract.validate_url(url).is_err());
        }
        for marker in [
            MARKER.replace("7000", "4000"),
            MARKER.replace("8000", "0000"),
            MARKER.to_ascii_uppercase(),
            format!("{MARKER}/extra"),
        ] {
            assert!(ImportedDatabase::new("isolated_target", &marker, HASH).is_err());
        }
        assert!(ImportedDatabase::new("postgres", MARKER, HASH).is_err());
        assert!(ImportedDatabase::new("isolated_target", MARKER, &HASH.to_uppercase()).is_err());
    }

    #[test]
    fn an_exact_receipt_and_unprivileged_service_identity_are_all_required() {
        let contract = ImportedDatabase::new("isolated_target", MARKER, HASH).unwrap();
        let proof = || Admission {
            database: "isolated_target".into(),
            marker: Some(MARKER.into()),
            principal_valid: true,
            owner_valid: true,
            no_ddl: true,
        };
        assert!(contract.validate_admission(&proof()).is_ok());
        for index in 0..5 {
            let mut bad = proof();
            match index {
                0 => bad.database = "old_source".into(),
                1 => bad.marker = None,
                2 => bad.principal_valid = false,
                3 => bad.owner_valid = false,
                _ => bad.no_ddl = false,
            }
            assert!(contract.validate_admission(&bad).is_err());
        }
        assert!(ADMISSION_SQL.contains("pg_auth_members m WHERE m.admin_option"));
        assert!(ADMISSION_SQL.contains("pg_has_role(r.oid,m.member,'SET'::text)"));
        assert!(ADMISSION_SQL.contains("pg_has_role(r.oid,m.member,'USAGE'::text)"));
    }

    #[test]
    fn schema_material_is_the_import_contract_without_business_scans() {
        let source =
            include_str!("../../ops/recovery/src/uuid_inventory/catalog.rs").replace("\r\n", "\n");
        let start = source.find("catalog_items AS MATERIALIZED (").unwrap();
        let end = source[start..]
            .find("),\nmigration_rows AS MATERIALIZED (")
            .unwrap()
            + start;
        let user_start = source.find("user_schemas AS MATERIALIZED (").unwrap();
        let user_end = source[user_start..]
            .find("tables AS MATERIALIZED (")
            .unwrap()
            + user_start;
        assert!(SCHEMA_MATERIAL.contains(&source[user_start..user_end]));
        assert!(SCHEMA_MATERIAL.contains(&source[start..end]));
        assert!(SCHEMA_MATERIAL
            .contains("jsonb_agg(item ORDER BY kind, schema_name, object_name, sub_name)"));
        for scan in [
            "query_to_xml",
            "uuid_counts AS",
            "table_counts AS",
            "migration_rows AS",
        ] {
            assert!(!SCHEMA_MATERIAL.contains(scan));
        }
    }
}
