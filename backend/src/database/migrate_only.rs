use std::{collections::BTreeSet, error::Error, ffi::OsString, fmt, time::Duration};

use reqwest::Url;
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions, PgSslMode},
    ConnectOptions,
};

#[derive(Clone, Copy, Debug)]
enum Failure {
    Arguments,
    SeedImportConflict,
    SeedUnsupported,
    ServerImportMode,
    ServerMode,
    DatabaseUrl,
    AmbientConfiguration,
    Connection,
    Migration,
    MigrationTimeout,
    CloseTimeout,
}

/// A fixed-category migration command failure. Display and Debug omit connection
/// strings and driver diagnostics; the private source remains available through
/// [`Error::source`] for controlled diagnostics.
pub struct MigrationCommandError {
    failure: Failure,
    source: Option<anyhow::Error>,
}

impl MigrationCommandError {
    fn new(failure: Failure) -> Self {
        Self {
            failure,
            source: None,
        }
    }

    fn caused_by(failure: Failure, source: impl Into<anyhow::Error>) -> Self {
        Self {
            failure,
            source: Some(source.into()),
        }
    }
}

impl fmt::Display for MigrationCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.failure {
            Failure::Arguments => "MIGRATION_ARGUMENTS_INVALID: expected no arguments or --import",
            Failure::SeedImportConflict => "SEED_IMPORT_MUTUALLY_EXCLUSIVE",
            Failure::SeedUnsupported => "MIGRATION_SEED_NOT_SUPPORTED: this command never runs seed",
            Failure::ServerImportMode => "IMPORT_REQUIRES_MIGRATE_ONLY: use platform-migrate --import",
            Failure::ServerMode => "PLATFORM_DATABASE_INIT_MODE must be seed, import or imported",
            Failure::DatabaseUrl => "MIGRATION_DATABASE_URL_INVALID: DATABASE_URL must explicitly identify the target PostgreSQL database",
            Failure::AmbientConfiguration => "MIGRATION_AMBIENT_PG_CONFIGURATION_UNSUPPORTED: supply database options in DATABASE_URL",
            Failure::Connection => "MIGRATION_DATABASE_CONNECTION_FAILED",
            Failure::Migration => "MIGRATION_DATABASE_INITIALIZATION_FAILED",
            Failure::MigrationTimeout => "MIGRATION_DATABASE_INITIALIZATION_TIMED_OUT",
            Failure::CloseTimeout => "MIGRATION_DATABASE_CLOSE_TIMED_OUT",
        })
    }
}

impl fmt::Debug for MigrationCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Error for MigrationCommandError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_ref().map(|source| source.as_ref())
    }
}

/// Check the normal service's initialization mode before connecting or seeding.
/// The absent/default `seed` mode preserves normal startup, including region
/// initialization. `import` always rejects this entry point so no seed, region
/// catalog, grants, workers or listener can run during offline import preparation.
/// `imported` additionally requires an [`super::ImportedDatabase`] contract before
/// the caller may start workers or HTTP; this function only checks the mode name.
pub fn ensure_server_initialization_mode(raw: Option<&str>) -> Result<(), MigrationCommandError> {
    match raw {
        None | Some("seed" | "imported") => Ok(()),
        Some("import") => Err(MigrationCommandError::new(Failure::ServerImportMode)),
        Some(_) => Err(MigrationCommandError::new(Failure::ServerMode)),
    }
}

fn validate_arguments(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<(), MigrationCommandError> {
    let (mut seed, mut import) = (false, false);
    for argument in arguments {
        match argument.to_str() {
            Some("--seed") if !seed => seed = true,
            Some("--import") if !import => import = true,
            _ => return Err(MigrationCommandError::new(Failure::Arguments)),
        }
    }
    if seed && import {
        return Err(MigrationCommandError::new(Failure::SeedImportConflict));
    }
    if seed {
        return Err(MigrationCommandError::new(Failure::SeedUnsupported));
    }
    Ok(())
}

pub(super) fn database_options(raw: &str) -> Result<PgConnectOptions, MigrationCommandError> {
    let invalid = || MigrationCommandError::new(Failure::DatabaseUrl);
    let url = Url::parse(raw).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "postgres" | "postgresql")
        || url.host_str().is_none_or(str::is_empty)
        || url.username().is_empty()
        || url.path().trim_matches('/').is_empty()
        || url.path().trim_start_matches('/').contains('/')
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    let mut seen = BTreeSet::new();
    let mut mode = PgSslMode::Prefer;
    // The runner rejects ambient PG settings before this constructor is called.
    // Build explicitly to avoid FromStr's implicit ~/.pgpass lookup as well.
    let decode = |value: &str| {
        let decoded = percent_encoding::percent_decode_str(value)
            .decode_utf8()
            .map(|value| value.into_owned())
            .map_err(|_| invalid())?;
        // PostgreSQL startup fields are NUL-delimited. A decoded NUL could
        // change the actual database or inject a second startup parameter.
        if decoded.chars().any(char::is_control) {
            return Err(invalid());
        }
        Ok(decoded)
    };
    let mut options = PgConnectOptions::new_without_pgpass()
        .host(url.host_str().ok_or_else(invalid)?)
        .port(url.port().unwrap_or(5432))
        .username(&decode(url.username())?)
        .password(&decode(url.password().unwrap_or(""))?)
        .database(&decode(url.path().trim_start_matches('/'))?);
    // Reject unknown parameters before SQLx can log their supplied values.
    // Only an explicit Unix socket and one simple startup role are allowed.
    for (key, value) in url.query_pairs() {
        let category = match key.as_ref() {
            "sslmode" | "ssl-mode" => {
                mode = value.parse().map_err(|_| invalid())?;
                "sslmode"
            }
            "sslrootcert" | "ssl-root-cert" | "ssl-ca" => {
                options = options.ssl_root_cert(value.as_ref());
                "sslrootcert"
            }
            "sslcert" | "ssl-cert" => {
                options = options.ssl_client_cert(value.as_ref());
                "sslcert"
            }
            "sslkey" | "ssl-key" => {
                options = options.ssl_client_key(value.as_ref());
                "sslkey"
            }
            "application_name" => "application_name",
            "host"
                if value.starts_with('/')
                    && !value
                        .chars()
                        .any(|c| c.is_control() || c.is_whitespace() || c == '\\') =>
            {
                options = options.socket(value.as_ref());
                "host"
            }
            "options[role]" if valid_role(&value) => {
                options = options.options([("role", value.as_ref())]);
                "role"
            }
            _ => return Err(invalid()),
        };
        if value.is_empty() || value.chars().any(char::is_control) || !seen.insert(category) {
            return Err(invalid());
        }
    }
    let options = options
        .ssl_mode(mode)
        .application_name("platform-migrate")
        .disable_statement_logging();
    if matches!(
        options.get_database(),
        Some("postgres" | "template0" | "template1")
    ) {
        return Err(invalid());
    }
    Ok(options)
}

fn valid_role(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
        })
}

/// Apply the canonical embedded migrations, then close the pool and return.
///
/// Arguments exclude the executable name. No arguments and `--import` both mean
/// migration-only; this command does not import rows. `--seed`, mixed seed/import,
/// repeated and unknown arguments fail before reading connection configuration.
/// Only process `DATABASE_URL` is loaded: no dotenv, auto-create, API credentials,
/// seed, region initialization, HTTP listener or worker is involved. Ambient
/// libpq configuration is rejected to keep the explicit target unambiguous.
pub async fn run_migration_command(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<(), MigrationCommandError> {
    validate_arguments(arguments)?;
    if std::env::vars_os()
        .any(|(key, _)| key.to_string_lossy().to_ascii_uppercase().starts_with("PG"))
    {
        return Err(MigrationCommandError::new(Failure::AmbientConfiguration));
    }
    let raw = std::env::var("DATABASE_URL")
        .map_err(|_| MigrationCommandError::new(Failure::DatabaseUrl))?;
    let options = database_options(&raw)?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .after_connect(|connection, _| Box::pin(async move {
            sqlx::query("SELECT pg_catalog.set_config('search_path','public,pg_catalog,pg_temp',false), pg_catalog.set_config('statement_timeout','300000ms',false), pg_catalog.set_config('lock_timeout','30000ms',false), pg_catalog.set_config('idle_in_transaction_session_timeout','15000ms',false)")
                .execute(connection).await?;
            Ok(())
        }))
        .connect_with(options)
        .await
        .map_err(|source| MigrationCommandError::caused_by(Failure::Connection, source))?;
    let result =
        match tokio::time::timeout(Duration::from_secs(300), super::run_migrations(&pool)).await {
            Ok(result) => result
                .map_err(|source| MigrationCommandError::caused_by(Failure::Migration, source)),
            Err(source) => Err(MigrationCommandError::caused_by(
                Failure::MigrationTimeout,
                source,
            )),
        };
    let closed = tokio::time::timeout(Duration::from_secs(10), pool.close()).await;
    result?;
    closed.map_err(|source| MigrationCommandError::caused_by(Failure::CloseTimeout, source))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn import_preparation_cannot_select_seed_or_fall_through_to_server() {
        assert!(validate_arguments(args(&[])).is_ok());
        assert!(validate_arguments(args(&["--import"])).is_ok());
        for values in [["--seed", "--import"], ["--import", "--seed"]] {
            assert_eq!(
                validate_arguments(args(&values)).unwrap_err().to_string(),
                "SEED_IMPORT_MUTUALLY_EXCLUSIVE"
            );
        }
        assert!(validate_arguments(args(&["--seed"])).is_err());
        for values in [
            vec!["--import", "--import"],
            vec!["--seed", "--seed"],
            vec!["--unknown"],
            vec!["--import=1"],
        ] {
            assert!(validate_arguments(args(&values)).is_err());
        }
        assert!(ensure_server_initialization_mode(None).is_ok());
        assert!(ensure_server_initialization_mode(Some("seed")).is_ok());
        for mode in ["import", "seed,import", "", "SEED", " import "] {
            assert!(ensure_server_initialization_mode(Some(mode)).is_err());
        }
    }

    #[test]
    fn migration_connection_requires_one_explicit_target_and_keeps_diagnostics_private() {
        let parsed = database_options(
            "postgresql://operator:private%21@127.0.0.1:55437/disposable?sslmode=disable",
        )
        .unwrap();
        assert_eq!(parsed.get_host(), "127.0.0.1");
        assert_eq!(parsed.get_port(), 55437);
        assert_eq!(parsed.get_database(), Some("disposable"));
        assert_eq!(parsed.get_username(), "operator");
        let peer = database_options("postgresql://postgres@localhost:5432/disposable?host=/run/postgresql&options[role]=uuid_u04_platform_owner").unwrap();
        assert_eq!(peer.get_database(), Some("disposable"));
        assert_eq!(peer.get_username(), "postgres");
        assert_eq!(
            peer.get_socket().map(std::path::PathBuf::as_path),
            Some(std::path::Path::new("/run/postgresql"))
        );
        assert_eq!(
            database_options("postgresql://operator@127.0.0.1/disposable")
                .unwrap()
                .get_port(),
            5432
        );
        for url in [
            "postgresql:///disposable",
            "postgresql://127.0.0.1/disposable",
            "postgresql://operator@127.0.0.1/",
            "postgresql://operator@127.0.0.1/postgres",
            "postgresql://operator@127.0.0.1/%74emplate1",
            "postgresql://operator@127.0.0.1/a/b",
            "postgresql://operator:private@127.0.0.1/disposable?host=elsewhere",
            "postgresql://operator:private@127.0.0.1/disposable?host=/run/postgresql&host=/tmp",
            "postgresql://operator:private@127.0.0.1/disposable?options[role]=owner%20-c%20search_path=private",
            "postgresql://operator:private@127.0.0.1/disposable?options[role]=owner&options[role]=other",
            "postgresql://operator:private@127.0.0.1/disposable?options[search_path]=private",
            "postgresql://operator:private@127.0.0.1/disposable?options=-c%20role=private",
            "postgresql://operator:private@127.0.0.1/disposable?secret=private",
            "postgresql://operator:private@127.0.0.1/disposable?sslmode=disable&ssl-mode=require",
            "postgresql://operator:private@127.0.0.1/disposable?sslmode=invalid",
            "postgresql://operator:private@127.0.0.1/disposable#private",
        ] {
            let error = database_options(url).unwrap_err();
            assert!(!format!("{error:?} {error}").contains("private"));
        }
        let error = MigrationCommandError::caused_by(
            Failure::Connection,
            anyhow::anyhow!("private connection details"),
        );
        assert!(!format!("{error:?} {error}").contains("private"));
        assert!(error.source().is_some());
    }

    #[test]
    fn decoded_startup_fields_cannot_redirect_the_database_or_inject_parameters() {
        for url in [
            "postgresql://postgres@localhost:5432/postgres%00options%00-c%20role%3Dpostgres?host=/run/postgresql",
            "postgresql://operator%00options%00-c%20role%3Dpostgres@localhost/disposable",
            "postgresql://operator:private%00value@localhost/disposable",
            "postgresql://operator@localhost/disposable%0A",
            "postgresql://operator@localhost/disposable%7F",
            "postgresql://operator@localhost/disposable?sslrootcert=/private%00certificate",
            "postgresql://operator@localhost/disposable?application_name=private%00options",
        ] {
            let error = database_options(url).unwrap_err();
            assert!(matches!(error.failure, Failure::DatabaseUrl));
            assert!(!format!("{error:?} {error}").contains("private"));
        }
    }
}
