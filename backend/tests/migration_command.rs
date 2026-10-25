//! Process-level negative gates never connect to a database.
use std::{path::PathBuf, process::Command};

struct Directory(PathBuf);

impl Directory {
    fn create() -> Self {
        let path =
            std::env::temp_dir().join(format!("platform-migrate-test-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command.env_clear().current_dir(&self.0);
        if let Some(root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", root);
        }
        command
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0.join(".env"));
        let _ = std::fs::remove_dir(&self.0);
    }
}

fn assert_failure(mut command: Command, message: &str) {
    let output = command.output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stderr.trim(), message);
    assert!(!stderr.contains("private-canary"));
}

#[test]
fn migration_seed_import_conflict_and_bad_arguments_fail_before_configuration() {
    let directory = Directory::create();
    for arguments in [["--seed", "--import"], ["--import", "--seed"]] {
        let mut command = directory.command(env!("CARGO_BIN_EXE_platform-migrate"));
        command.args(arguments).env("PGPASSWORD", "private-canary");
        assert_failure(command, "SEED_IMPORT_MUTUALLY_EXCLUSIVE");
    }
    let mut command = directory.command(env!("CARGO_BIN_EXE_platform-migrate"));
    command.arg("--seed");
    assert_failure(
        command,
        "MIGRATION_SEED_NOT_SUPPORTED: this command never runs seed",
    );
    for arguments in [
        vec!["--import", "--import"],
        vec!["--seed", "--seed"],
        vec!["--unknown-private-canary"],
    ] {
        let mut command = directory.command(env!("CARGO_BIN_EXE_platform-migrate"));
        command.args(arguments);
        assert_failure(
            command,
            "MIGRATION_ARGUMENTS_INVALID: expected no arguments or --import",
        );
    }
}

#[test]
fn migration_never_loads_dotenv_or_app_env_file() {
    let directory = Directory::create();
    std::fs::write(
        directory.0.join(".env"),
        "DATABASE_URL=postgresql://private-canary@127.0.0.1:1/disposable\n",
    )
    .unwrap();
    for arguments in [vec![], vec!["--import"]] {
        let mut command = directory.command(env!("CARGO_BIN_EXE_platform-migrate"));
        command.args(arguments).env(
            "APP_ENV_FILE",
            directory.0.join("missing-private-canary.env"),
        );
        assert_failure(command, "MIGRATION_DATABASE_URL_INVALID: DATABASE_URL must explicitly identify the target PostgreSQL database");
    }
}

#[test]
fn migration_rejects_ambient_libpq_settings_without_revealing_values() {
    let directory = Directory::create();
    for name in ["PGPORT", "PGPASSWORD", "PGOPTIONS"] {
        let mut command = directory.command(env!("CARGO_BIN_EXE_platform-migrate"));
        command.env(name, "private-canary");
        assert_failure(command, "MIGRATION_AMBIENT_PG_CONFIGURATION_UNSUPPORTED: supply database options in DATABASE_URL");
    }
}

#[test]
fn decoded_nul_cannot_bypass_the_explicit_database_boundary() {
    let directory = Directory::create();
    let mut command = directory.command(env!("CARGO_BIN_EXE_platform-migrate"));
    command.env("DATABASE_URL", "postgresql://postgres@localhost:5432/postgres%00options%00-c%20role%3Dpostgres?host=/run/postgresql");
    assert_failure(command, "MIGRATION_DATABASE_URL_INVALID: DATABASE_URL must explicitly identify the target PostgreSQL database");
}

#[test]
fn normal_server_rejects_import_before_runtime_configuration_or_database_access() {
    let directory = Directory::create();
    let mut command = directory.command(env!("CARGO_BIN_EXE_saas-platform-backend"));
    command.env("PLATFORM_DATABASE_INIT_MODE", "import");
    assert_failure(
        command,
        "Error: IMPORT_REQUIRES_MIGRATE_ONLY: use platform-migrate --import",
    );
    for mode in ["seed,import", "", "SEED"] {
        let mut command = directory.command(env!("CARGO_BIN_EXE_saas-platform-backend"));
        command.env("PLATFORM_DATABASE_INIT_MODE", mode);
        assert_failure(
            command,
            "Error: PLATFORM_DATABASE_INIT_MODE must be seed, import or imported",
        );
    }
}

#[test]
fn imported_startup_requires_the_complete_explicit_handoff_before_connecting() {
    let directory = Directory::create();
    let mut command = directory.command(env!("CARGO_BIN_EXE_saas-platform-backend"));
    command.env("PLATFORM_DATABASE_INIT_MODE", "imported");
    assert_failure(command, "Error: IMPORTED_DATABASE_CONFIGURATION_INVALID");
    let mut command = directory.command(env!("CARGO_BIN_EXE_saas-platform-backend"));
    command.env("PLATFORM_IMPORT_MARKER", "private-canary");
    assert_failure(
        command,
        "Error: IMPORTED_DATABASE_FIELDS_REQUIRE_IMPORTED_MODE",
    );
}

#[cfg(unix)]
#[test]
fn imported_non_unicode_handoff_fails_without_panicking_or_using_defaults() {
    use std::os::unix::ffi::OsStringExt;
    let directory = Directory::create();
    let mut command = directory.command(env!("CARGO_BIN_EXE_saas-platform-backend"));
    command.env("PLATFORM_DATABASE_INIT_MODE", "imported");
    command.env(
        "PLATFORM_EXPECTED_DATABASE",
        std::ffi::OsString::from_vec(vec![b't', 0xff]),
    );
    assert_failure(command, "Error: CONFIGURATION_VALUES_MUST_BE_UTF8");
}
