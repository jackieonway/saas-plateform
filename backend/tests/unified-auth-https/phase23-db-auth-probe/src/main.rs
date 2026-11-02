//! A bounded, private-file-only password authentication probe for one owned task.
//! Reports contain only fixed labels, statuses, observations and SQLSTATE classes.
use serde::{Deserialize, Serialize};
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions, PgSslMode},
    ConnectOptions, Row,
};
use std::{io::Write, path::PathBuf, time::Duration};
use tokio::time::{timeout, timeout_at, Instant};

#[cfg(target_os = "linux")]
mod private;

const TASK: &str = "/tmp/rust-saas-unified-auth-20261028";
const OWNER: &[u8] = b"rust-saas-unified-auth-20261028-pg55494\n";
const CONNECT_QUERY_BUDGET: Duration = Duration::from_secs(10);
const CLOSE_BUDGET: Duration = Duration::from_secs(1);
const PROOF_SQL: &str = "SELECT (pg_catalog.inet_client_addr()='127.0.0.2'::inet \
    AND current_user::text=$1::text AND pg_catalog.current_database()::text=$2::text \
    AND pg_catalog.shobj_description((SELECT oid FROM pg_catalog.pg_database \
        WHERE datname=pg_catalog.current_database()),'pg_database')=$3) AS bound";

type LocalResult<T> = Result<T, &'static str>;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Mode {
    Source,
    Restore,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Binding {
    mode: Mode,
    shared_database: String,
    portal_database: Option<String>,
    role_prefix: String,
    deployment_id: String,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Label {
    IdentityCorrect,
    IdentityWrong,
    IdentityOld,
    IdentityNew,
    PlatformCorrect,
    PlatformWrong,
    PlatformOld,
    PlatformNew,
    PortalCorrect,
    PortalWrong,
    PortalOld,
    PortalNew,
}

impl Label {
    fn role_suffix(self) -> &'static str {
        match self {
            Self::IdentityCorrect | Self::IdentityWrong | Self::IdentityOld | Self::IdentityNew => {
                "_identity_runtime"
            }
            Self::PlatformCorrect | Self::PlatformWrong | Self::PlatformOld | Self::PlatformNew => {
                "_platform_runtime"
            }
            Self::PortalCorrect | Self::PortalWrong | Self::PortalOld | Self::PortalNew => {
                "_portal_runtime"
            }
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Expected {
    Accept,
    #[serde(rename = "REJECT_28P01")]
    Reject28p01,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Case {
    label: Label,
    database: String,
    role: String,
    password_file: PathBuf,
    expected: Expected,
    expected_marker: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    contract_version: String,
    binding: Binding,
    cases: Vec<Case>,
}

fn canonical_v7(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
        && value.as_bytes()[14] == b'7'
        && matches!(value.as_bytes()[19], b'8' | b'9' | b'a' | b'b')
}

fn validate(input: &Input) -> LocalResult<()> {
    if input.contract_version != "identity-db-password-probe-input/1"
        || !(1..=16).contains(&input.cases.len())
        || !canonical_v7(&input.binding.deployment_id)
    {
        return Err("DB_AUTH_PROBE_INPUT_REJECTED");
    }
    let binding = &input.binding;
    let suffix = binding
        .shared_database
        .strip_prefix("unified_auth_https_")
        .filter(|tail| {
            tail.len() == 32
                && tail
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .ok_or("DB_AUTH_PROBE_BINDING_REJECTED")?;
    if binding.role_prefix != format!("uah_{}", &suffix[20..])
        || binding
            .portal_database
            .as_deref()
            .is_some_and(|name| name != format!("unified_auth_portal_{suffix}"))
    {
        return Err("DB_AUTH_PROBE_BINDING_REJECTED");
    }
    let shared = match binding.mode {
        Mode::Source => binding.shared_database.clone(),
        Mode::Restore => format!("unified_auth_restore_{suffix}"),
    };
    let portal = binding
        .portal_database
        .as_ref()
        .map(|name| match binding.mode {
            Mode::Source => name.clone(),
            Mode::Restore => format!("unified_auth_restore_portal_{suffix}"),
        });
    let (shared_marker, portal_marker) = match binding.mode {
        Mode::Source => (
            format!("identity-deployment/1/{}", binding.deployment_id),
            format!("unified-auth-https-portal/1/{}", binding.deployment_id),
        ),
        Mode::Restore => {
            let marker = format!("phase23-recovery/2/{}", binding.deployment_id);
            (marker.clone(), marker)
        }
    };
    let mut labels = std::collections::BTreeSet::new();
    for case in &input.cases {
        // Serialization produces a fixed enum label, never arbitrary caller text.
        let label =
            serde_json::to_string(&case.label).map_err(|_| "DB_AUTH_PROBE_INPUT_REJECTED")?;
        if !labels.insert(label)
            || case.role != format!("{}{}", binding.role_prefix, case.label.role_suffix())
        {
            return Err("DB_AUTH_PROBE_CASE_BINDING_REJECTED");
        }
        let portal_case = case.label.role_suffix() == "_portal_runtime";
        let expected_database = if portal_case {
            portal
                .as_deref()
                .ok_or("DB_AUTH_PROBE_CASE_BINDING_REJECTED")?
        } else {
            &shared
        };
        let marker = if portal_case {
            &portal_marker
        } else {
            &shared_marker
        };
        if case.database != expected_database || &case.expected_marker != marker {
            return Err("DB_AUTH_PROBE_CASE_BINDING_REJECTED");
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum SqlState {
    #[serde(rename = "28P01")]
    Password28p01,
    #[serde(rename = "28000")]
    Authorization28000,
    #[serde(rename = "3D000")]
    Database3d000,
    ConnectionClass08,
    ResourceClass53,
    Other,
}

fn sqlstate(code: &str) -> SqlState {
    match code {
        "28P01" => SqlState::Password28p01,
        "28000" => SqlState::Authorization28000,
        "3D000" => SqlState::Database3d000,
        code if code.starts_with("08") => SqlState::ConnectionClass08,
        code if code.starts_with("53") => SqlState::ResourceClass53,
        _ => SqlState::Other,
    }
}

#[derive(Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Observation {
    Accepted,
    PasswordRejected,
    OtherDatabaseError,
    Timeout,
    Unavailable,
    ProofMismatch,
    CloseFailed,
}

fn failure(error: &sqlx::Error) -> (Observation, Option<SqlState>) {
    if let sqlx::Error::Database(database) = error {
        let state = database
            .code()
            .map(|code| sqlstate(&code))
            .unwrap_or(SqlState::Other);
        let observed = if state == SqlState::Password28p01 {
            Observation::PasswordRejected
        } else {
            Observation::OtherDatabaseError
        };
        return (observed, Some(state));
    }
    (
        if matches!(error, sqlx::Error::PoolTimedOut) {
            Observation::Timeout
        } else {
            Observation::Unavailable
        },
        None,
    )
}

#[derive(Serialize, PartialEq, Eq)]
enum Status {
    #[serde(rename = "PASS")]
    Pass,
    #[serde(rename = "FAIL")]
    Fail,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CaseReport {
    label: Label,
    status: Status,
    observation: Observation,
    sql_state: Option<SqlState>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    contract_version: &'static str,
    result: Status,
    cases: Vec<CaseReport>,
}

fn result(
    case: &Case,
    observation: Observation,
    state: Option<SqlState>,
    during_connect: bool,
) -> CaseReport {
    let pass = match case.expected {
        Expected::Accept => observation == Observation::Accepted,
        Expected::Reject28p01 => {
            during_connect
                && observation == Observation::PasswordRejected
                && state == Some(SqlState::Password28p01)
        }
    };
    CaseReport {
        label: case.label,
        status: if pass { Status::Pass } else { Status::Fail },
        observation,
        sql_state: state,
    }
}

async fn probe(case: &Case, password: &str) -> CaseReport {
    let deadline = Instant::now() + CONNECT_QUERY_BUDGET;
    let options = PgConnectOptions::new()
        .host("127.0.0.1")
        .port(25495)
        .username(&case.role)
        .password(password)
        .database(&case.database)
        .ssl_mode(PgSslMode::Prefer)
        .application_name("phase23-db-auth-probe")
        .disable_statement_logging();
    let connected = timeout_at(
        deadline,
        PgPoolOptions::new()
            .max_connections(1)
            .min_connections(0)
            .acquire_timeout(CONNECT_QUERY_BUDGET)
            .connect_with(options),
    )
    .await;
    let pool = match connected {
        Err(_) => return result(case, Observation::Timeout, None, true),
        Ok(Err(error)) => {
            let (observation, state) = failure(&error);
            return result(case, observation, state, true);
        }
        Ok(Ok(pool)) => pool,
    };
    let proof = timeout_at(
        deadline,
        sqlx::query(PROOF_SQL)
            .bind(&case.role)
            .bind(&case.database)
            .bind(&case.expected_marker)
            .fetch_one(&pool),
    )
    .await;
    let mut report = match proof {
        Err(_) => result(case, Observation::Timeout, None, false),
        Ok(Err(error)) => {
            let (observation, state) = failure(&error);
            result(case, observation, state, false)
        }
        Ok(Ok(row)) => result(
            case,
            if row.try_get::<bool, _>("bound").unwrap_or(false) {
                Observation::Accepted
            } else {
                Observation::ProofMismatch
            },
            None,
            false,
        ),
    };
    // A successful socket is always closed, including unexpected acceptance or
    // a cancelled proof. Cleanup cannot turn a failure into a password rejection.
    if timeout(CLOSE_BUDGET, pool.close()).await.is_err() {
        report = result(case, Observation::CloseFailed, None, false);
    }
    report
}

struct Cli {
    input: PathBuf,
    output: PathBuf,
}

fn arguments(args: impl IntoIterator<Item = std::ffi::OsString>) -> LocalResult<Cli> {
    let mut args = args.into_iter();
    let mut input = None;
    let mut output = None;
    while let Some(flag) = args.next() {
        let target = match flag.to_str() {
            Some("--input") if input.is_none() => &mut input,
            Some("--output") if output.is_none() => &mut output,
            _ => return Err("DB_AUTH_PROBE_ARGUMENTS_REJECTED"),
        };
        *target = Some(PathBuf::from(
            args.next().ok_or("DB_AUTH_PROBE_ARGUMENTS_REJECTED")?,
        ));
    }
    Ok(Cli {
        input: input.ok_or("DB_AUTH_PROBE_ARGUMENTS_REJECTED")?,
        output: output.ok_or("DB_AUTH_PROBE_ARGUMENTS_REJECTED")?,
    })
}

#[cfg(target_os = "linux")]
async fn run(cli: Cli) -> LocalResult<Report> {
    let task = private::OwnedTask::open(std::path::Path::new(TASK), OWNER)?;
    let input: Input = serde_json::from_slice(&task.read(&cli.input, 65_536)?)
        .map_err(|_| "DB_AUTH_PROBE_INPUT_REJECTED")?;
    validate(&input)?;
    let mut passwords = Vec::with_capacity(input.cases.len());
    for case in &input.cases {
        let bytes = task.read(&case.password_file, 4096)?;
        let value =
            std::str::from_utf8(&bytes).map_err(|_| "DB_AUTH_PROBE_PASSWORD_FILE_REJECTED")?;
        let value = value.strip_suffix('\n').unwrap_or(value);
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err("DB_AUTH_PROBE_PASSWORD_FILE_REJECTED");
        }
        passwords.push(value.to_owned());
    }
    // Reserve the exclusive output before opening any database socket.
    let mut output = task.create(&cli.output)?;
    let mut cases = Vec::with_capacity(input.cases.len());
    for (case, password) in input.cases.iter().zip(&passwords) {
        cases.push(probe(case, password).await);
    }
    let report = Report {
        contract_version: "identity-db-password-probe-report/1",
        result: if cases.iter().all(|case| case.status == Status::Pass) {
            Status::Pass
        } else {
            Status::Fail
        },
        cases,
    };
    let encoded = serde_json::to_vec_pretty(&report).map_err(|_| "DB_AUTH_PROBE_REPORT_FAILED")?;
    output
        .write_all(&encoded)
        .and_then(|()| output.write_all(b"\n"))
        .and_then(|()| output.sync_all())
        .map_err(|_| "DB_AUTH_PROBE_REPORT_FAILED")?;
    Ok(report)
}

#[cfg(not(target_os = "linux"))]
async fn run(_cli: Cli) -> LocalResult<Report> {
    Err("DB_AUTH_PROBE_LINUX_REQUIRED")
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::process::ExitCode {
    std::panic::set_hook(Box::new(|_| eprintln!("DB_AUTH_PROBE_INTERNAL_FAILURE")));
    let outcome = match arguments(std::env::args_os().skip(1)) {
        Ok(cli) => run(cli).await,
        Err(error) => Err(error),
    };
    match outcome {
        Ok(report) if report.result == Status::Pass => {
            println!("DB_AUTH_PROBE_PASS");
            std::process::ExitCode::SUCCESS
        }
        Ok(_) => {
            println!("DB_AUTH_PROBE_FAIL");
            std::process::ExitCode::from(1)
        }
        Err(code) => {
            eprintln!("{code}");
            std::process::ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input() -> Input {
        serde_json::from_value(json!({"contractVersion":"identity-db-password-probe-input/1",
            "binding":{"mode":"SOURCE","sharedDatabase":"unified_auth_https_019a1234567871238123456789abcdef","portalDatabase":"unified_auth_portal_019a1234567871238123456789abcdef","rolePrefix":"uah_456789abcdef","deploymentId":"019a1234-5678-7123-8123-456789abcdef"},
            "cases":[{"label":"IDENTITY_CORRECT","database":"unified_auth_https_019a1234567871238123456789abcdef","role":"uah_456789abcdef_identity_runtime","passwordFile":"/tmp/rust-saas-unified-auth-20261028/phase23-password","expected":"ACCEPT","expectedMarker":"identity-deployment/1/019a1234-5678-7123-8123-456789abcdef"}]})).unwrap()
    }

    #[test]
    fn source_and_restore_names_markers_roles_are_exact_derivations() {
        let mut value = input();
        assert!(validate(&value).is_ok());
        value.binding.mode = Mode::Restore;
        assert!(validate(&value).is_err());
        value.cases[0].database = value.cases[0]
            .database
            .replace("unified_auth_https_", "unified_auth_restore_");
        value.cases[0].expected_marker =
            "phase23-recovery/2/019a1234-5678-7123-8123-456789abcdef".into();
        assert!(validate(&value).is_ok());
        value.cases[0].role = "uah_000000000000_identity_runtime".into();
        assert!(validate(&value).is_err());
    }

    #[test]
    fn unknown_fields_labels_duplicates_and_wrong_portal_bindings_reject() {
        let mut value = input();
        value.cases[0].label = Label::PortalCorrect;
        value.cases[0].role = "uah_456789abcdef_portal_runtime".into();
        assert!(validate(&value).is_err());
        value.cases[0].database = value.binding.portal_database.clone().unwrap();
        value.cases[0].expected_marker =
            "unified-auth-https-portal/1/019a1234-5678-7123-8123-456789abcdef".into();
        assert!(validate(&value).is_ok());
        value.binding.mode = Mode::Restore;
        value.cases[0].database = value.cases[0]
            .database
            .replace("unified_auth_portal_", "unified_auth_restore_portal_");
        value.cases[0].expected_marker =
            "phase23-recovery/2/019a1234-5678-7123-8123-456789abcdef".into();
        assert!(validate(&value).is_ok());
        value.cases.push(value.cases[0].clone());
        assert!(validate(&value).is_err());
        let encoded = serde_json::json!({"contractVersion":"identity-db-password-probe-input/1","binding":{},"cases":[],"host":"127.0.0.1"});
        assert!(serde_json::from_value::<Input>(encoded).is_err());
        assert!(serde_json::from_str::<Expected>("\"REJECT_28P01\"").is_ok());
        assert!(serde_json::from_str::<Label>("\"FREE_TEXT\"").is_err());
    }

    #[test]
    fn only_real_connect_database_28p01_is_a_successful_password_rejection() {
        let mut value = input();
        value.cases[0].expected = Expected::Reject28p01;
        let case = &value.cases[0];
        assert!(
            result(
                case,
                Observation::PasswordRejected,
                Some(SqlState::Password28p01),
                true
            )
            .status
                == Status::Pass
        );
        for (observation, state, connect) in [
            (
                Observation::PasswordRejected,
                Some(SqlState::Password28p01),
                false,
            ),
            (
                Observation::OtherDatabaseError,
                Some(SqlState::Database3d000),
                true,
            ),
            (Observation::Timeout, None, true),
            (Observation::Unavailable, None, true),
            (Observation::Accepted, None, false),
            (Observation::ProofMismatch, None, false),
        ] {
            assert!(result(case, observation, state, connect).status == Status::Fail);
        }
        assert!(
            failure(&sqlx::Error::Protocol(
                "server text includes 28P01 and private material".into()
            ))
            .0 == Observation::Unavailable
        );
        assert!(sqlstate("3D000") == SqlState::Database3d000);
        assert!(sqlstate("08006") == SqlState::ConnectionClass08);
    }

    #[test]
    fn report_never_serializes_input_bindings_or_passwords() {
        let value = input();
        let report = Report {
            contract_version: "identity-db-password-probe-report/1",
            result: Status::Pass,
            cases: vec![result(&value.cases[0], Observation::Accepted, None, false)],
        };
        let text = serde_json::to_string(&report).unwrap();
        for forbidden in [
            &value.binding.shared_database,
            &value.cases[0].role,
            &value.cases[0].expected_marker,
        ] {
            assert!(!text.contains(forbidden));
        }
        assert!(text.contains("IDENTITY_CORRECT") && text.contains("PASS"));
    }

    #[test]
    fn cli_rejects_duplicate_flags_and_missing_or_unknown_arguments() {
        let args = |values: &[&str]| {
            values
                .iter()
                .map(std::ffi::OsString::from)
                .collect::<Vec<_>>()
        };
        assert!(arguments(args(&["--input", "/a", "--output", "/b"])).is_ok());
        assert!(arguments(args(&["--input", "/a", "--input", "/c", "--output", "/b"])).is_err());
        assert!(arguments(args(&["--input", "/a", "--password", "do-not-log"])).is_err());
        assert!(arguments(args(&["--input"])).is_err());
    }
}
