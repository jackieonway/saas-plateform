//! Real rollout CLI/database/startup contracts. All readiness artifacts here are
//! explicit fixture_only inputs; they do not attest a production safety baseline.
//! Ignored cases require IDENTITY_PHASE06_TEST_DATABASE_URL to explicitly name a
//! loopback `postgres` administration database. Only owned rollout databases are
//! created/dropped; ordinary negative process tests never connect to PostgreSQL.

use std::{
    future::Future,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use chrono::Utc;
use reqwest::Url;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions, PgSslMode},
    PgPool,
};
use tokio::{
    io::AsyncReadExt,
    process::{Child, Command},
    task::JoinHandle,
    time::{timeout, Instant},
};
use uuid::Uuid;

const ROLLOUT: &str = env!("CARGO_BIN_EXE_platform-membership-rollout");
const PLATFORM: &str = env!("CARGO_BIN_EXE_saas-platform-backend");
const PREFIX: &str = "identity_phase06_rollout_";
const ARGUMENTS_ERROR: &str = "MEMBERSHIP_ROLLOUT_ARGUMENTS_INVALID: expected inspect --platform-binary PATH --environment development|production, or apply --manifest PATH --platform-binary PATH";
const GATES: [&str; 10] = [
    "backend",
    "frontend",
    "notification",
    "contracts",
    "phase_01",
    "phase_02",
    "phase_03",
    "phase_04",
    "phase_05",
    "phase_06",
];

fn required<T, E>(result: Result<T, E>, context: &str) -> T {
    match result {
        Ok(value) => value,
        Err(_) => panic!("{context}"),
    }
}

struct Directory(PathBuf);

impl Directory {
    fn create() -> Self {
        let path =
            std::env::temp_dir().join(format!("identity-rollout-fixture-{}", Uuid::now_v7()));
        required(
            std::fs::create_dir(&path),
            "create owned rollout fixture directory",
        );
        Self(path)
    }

    fn command(&self, program: &str) -> Command {
        command(program, &self.0)
    }

    async fn remove(&self) {
        assert!(self.0.is_absolute() && self.0.parent() == Some(std::env::temp_dir().as_path()));
        required(
            required(
                timeout(Duration::from_secs(10), tokio::fs::remove_dir_all(&self.0)).await,
                "fixture directory cleanup deadline",
            ),
            "remove owned fixture artifacts",
        );
        assert!(!self.0.exists());
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        // The fixed generated absolute child is the only recursive cleanup target.
        if self.0.is_absolute()
            && self.0.parent() == Some(std::env::temp_dir().as_path())
            && self.0.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .starts_with("identity-rollout-fixture-")
            })
        {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

fn command(program: &str, directory: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .current_dir(directory)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", root);
    }
    command
}

struct Process {
    child: Child,
    stdout: JoinHandle<Vec<u8>>,
    stderr: JoinHandle<Vec<u8>>,
}

struct Output {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl Process {
    fn start(mut command: Command) -> Self {
        let mut child = required(command.spawn(), "start owned rollout/platform process");
        let mut stdout = child.stdout.take().expect("piped stdout").take(1024 * 1024);
        let mut stderr = child.stderr.take().expect("piped stderr").take(1024 * 1024);
        Self {
            child,
            stdout: tokio::spawn(async move {
                let mut bytes = Vec::new();
                required(
                    stdout.read_to_end(&mut bytes).await,
                    "read bounded process stdout",
                );
                bytes
            }),
            stderr: tokio::spawn(async move {
                let mut bytes = Vec::new();
                required(
                    stderr.read_to_end(&mut bytes).await,
                    "read bounded process stderr",
                );
                bytes
            }),
        }
    }

    async fn finish(mut self) -> Output {
        let waited = timeout(Duration::from_secs(60), self.child.wait()).await;
        let status = match waited {
            Ok(result) => required(result, "wait rollout/platform process"),
            Err(_) => {
                let _ = self.child.kill().await;
                let _ = self.child.wait().await;
                panic!("owned rollout/platform process exceeded bounded exit wait");
            }
        };
        Output {
            success: status.success(),
            stdout: required(
                required(
                    timeout(Duration::from_secs(5), &mut self.stdout).await,
                    "stdout drain deadline",
                ),
                "stdout reader completed",
            ),
            stderr: required(
                required(
                    timeout(Duration::from_secs(5), &mut self.stderr).await,
                    "stderr drain deadline",
                ),
                "stderr reader completed",
            ),
        }
    }

    async fn stop(mut self) {
        let _ = self.child.kill().await;
        required(
            timeout(Duration::from_secs(10), self.child.wait()).await,
            "platform stop deadline",
        )
        .expect("platform stopped");
        let _ = timeout(Duration::from_secs(5), &mut self.stdout).await;
        let _ = timeout(Duration::from_secs(5), &mut self.stderr).await;
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        self.stdout.abort();
        self.stderr.abort();
    }
}

impl Output {
    fn refusal(&self, code: &str) {
        assert!(!self.success, "refused process must exit unsuccessfully");
        assert!(
            self.stdout.is_empty(),
            "rollout refusal has no success report"
        );
        assert_eq!(String::from_utf8_lossy(&self.stderr).trim(), code);
        assert!(!String::from_utf8_lossy(&self.stderr).contains("private-canary"));
    }

    fn report(&self) -> Value {
        assert!(
            self.success,
            "rollout failed with category: {}",
            String::from_utf8_lossy(&self.stderr)
        );
        assert!(
            self.stderr.is_empty(),
            "successful rollout has no diagnostic stderr"
        );
        required(
            serde_json::from_slice(&self.stdout),
            "parse rollout JSON report",
        )
    }
}

struct Harness {
    admin: PgPool,
    pool: PgPool,
    name: String,
    url: String,
    directory: Directory,
}

#[derive(Clone)]
struct Target {
    pool: PgPool,
    name: String,
    url: String,
    directory: PathBuf,
}

impl Harness {
    async fn create() -> Self {
        let raw = required(
            std::env::var("IDENTITY_PHASE06_TEST_DATABASE_URL"),
            "set explicit IDENTITY_PHASE06_TEST_DATABASE_URL",
        );
        let mut url = required(
            Url::parse(&raw),
            "parse explicit phase06 administration URL",
        );
        assert!(matches!(url.scheme(), "postgres" | "postgresql"));
        assert!(
            matches!(
                url.host_str(),
                Some("localhost" | "127.0.0.1" | "::1" | "[::1]")
            ),
            "phase06 database must be loopback"
        );
        assert_eq!(
            url.path(),
            "/postgres",
            "explicit administration database must be postgres"
        );
        assert!(
            !url.username().is_empty() && url.query().is_none() && url.fragment().is_none(),
            "administration URL must not override connection identity"
        );
        let directory = Directory::create();
        let admin = required(
            PgPoolOptions::new()
                .max_connections(2)
                .connect_with(explicit_options(&url, "postgres"))
                .await,
            "connect explicit loopback administration database",
        );
        let name = format!("{PREFIX}{}", Uuid::now_v7().simple());
        required(
            sqlx::query(&format!("CREATE DATABASE \"{name}\""))
                .execute(&admin)
                .await,
            "create owned phase06 rollout database",
        );
        url.set_path(&format!("/{name}"));
        let target_url = url.to_string();
        let pool = match PgPoolOptions::new()
            .max_connections(6)
            .connect_with(explicit_options(&url, &name))
            .await
        {
            Ok(pool) => pool,
            Err(_) => {
                let _ = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
                    .execute(&admin)
                    .await;
                admin.close().await;
                panic!("connect owned phase06 rollout database");
            }
        };
        let harness = Self {
            admin,
            pool,
            name,
            url: target_url,
            directory,
        };
        if saas_platform_backend::database::run_migrations(&harness.pool)
            .await
            .is_err()
        {
            harness.cleanup().await;
            panic!("apply real platform expansion migration chain");
        }
        harness
    }

    fn target(&self) -> Target {
        Target {
            pool: self.pool.clone(),
            name: self.name.clone(),
            url: self.url.clone(),
            directory: self.directory.0.clone(),
        }
    }

    async fn cleanup(self) {
        assert!(
            self.name.starts_with(PREFIX)
                && self.name.len() < 64
                && self
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        );
        required(
            timeout(Duration::from_secs(10), self.pool.close()).await,
            "close owned target pool",
        );
        required(
            sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
                .execute(&self.admin)
                .await,
            "drop only owned phase06 rollout database",
        );
        let remains: bool = required(
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname=$1)")
                .bind(&self.name)
                .fetch_one(&self.admin)
                .await,
            "verify owned database removal",
        );
        assert!(!remains);
        self.admin.close().await;
        self.directory.remove().await;
        println!("phase06 rollout owned database cleanup PASS");
    }
}

fn explicit_options(url: &Url, database: &str) -> PgConnectOptions {
    let decode = |value: &str| {
        let value = required(
            percent_encoding::percent_decode_str(value).decode_utf8(),
            "decode explicit database credential",
        );
        assert!(
            !value.chars().any(char::is_control),
            "database startup fields cannot contain controls"
        );
        value.into_owned()
    };
    PgConnectOptions::new_without_pgpass()
        .host(
            url.host_str()
                .unwrap()
                .trim_start_matches('[')
                .trim_end_matches(']'),
        )
        .port(url.port().unwrap_or(5432))
        .username(&decode(url.username()))
        .password(&decode(url.password().unwrap_or("")))
        .database(database)
        .ssl_mode(PgSslMode::Prefer)
}

async fn isolated<F, Fut>(run: F)
where
    F: FnOnce(Target) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let harness = Harness::create().await;
    let outcome = tokio::spawn(run(harness.target())).await;
    harness.cleanup().await;
    if let Err(error) = outcome {
        if error.is_panic() {
            std::panic::resume_unwind(error.into_panic())
        }
        panic!("phase06 test task cancelled");
    }
}

impl Target {
    fn rollout(&self) -> Command {
        let mut command = command(ROLLOUT, &self.directory);
        command.env("MEMBERSHIP_ROLLOUT_DATABASE_URL", &self.url);
        command
    }

    async fn inspect(&self) -> Value {
        let mut command = self.rollout();
        command.args([
            "inspect",
            "--platform-binary",
            PLATFORM,
            "--environment",
            "development",
        ]);
        Process::start(command).finish().await.report()
    }

    async fn apply(&self, manifest: &Value) -> Output {
        let path = self.directory.join("manifest.json");
        required(
            tokio::fs::write(&path, serde_json::to_vec(manifest).unwrap()).await,
            "write fixture_only manifest",
        );
        let mut command = self.rollout();
        command
            .arg("apply")
            .arg("--manifest")
            .arg(&path)
            .arg("--platform-binary")
            .arg(PLATFORM);
        Process::start(command).finish().await
    }

    async fn fixture_manifest(&self, inspected: &Value) -> Value {
        let declaration = serde_json::to_vec(&json!({
            "fixture_only": true, "scope": "TASK_OWNED", "database": self.name,
            "purpose": "Synthetic CLI validation input; this is not evidence that production gates or deployment inventory passed",
            "absent_writers": ["CLI", "IMPORTER", "BATCH", "OFFLINE_JOB"]
        })).unwrap();
        required(
            tokio::fs::write(
                self.directory.join("fixture-declaration.json"),
                &declaration,
            )
            .await,
            "write actual nonempty fixture declaration artifact",
        );
        let artifact = json!({"path":"fixture-declaration.json", "sha256":hex::encode(Sha256::digest(&declaration))});
        let inventory: Vec<Value> = ["PLATFORM_INSTANCE", "CLI", "IMPORTER", "BATCH", "OFFLINE_JOB"].iter().map(|kind| json!({
            "id": format!("fixture-only-{kind}"), "kind":kind,
            "status":if *kind=="PLATFORM_INSTANCE" {"READY"} else {"NOT_APPLICABLE"},
            "binary_sha256":if *kind=="PLATFORM_INSTANCE" {inspected["platform_binary_sha256"].clone()} else {Value::Null},
            "evidence":artifact
        })).collect();
        let gates: Vec<Value> = GATES
            .iter()
            .map(|gate| json!({"id":gate,"status":"PASS","evidence":artifact}))
            .collect();
        json!({
            "schema_version":1,"environment":"development","scope":"TASK_OWNED",
            "target_database":inspected["database"],"target_database_oid":inspected["database_oid"],
            "target_cluster_system_identifier":inspected["cluster_system_identifier"],
            "platform_binary_sha256":inspected["platform_binary_sha256"],"generated_at":Utc::now(),
            "inventory":inventory,"gates":gates
        })
    }

    async fn snapshot(&self) -> Value {
        required(sqlx::query_scalar(r#"
            SELECT jsonb_build_object(
              'members',(SELECT COALESCE(jsonb_agg(to_jsonb(m) ORDER BY id),'[]'::jsonb) FROM public.memberships m),
              'users',(SELECT COALESCE(jsonb_agg(to_jsonb(u) ORDER BY id),'[]'::jsonb) FROM public.users u),
              'history',(SELECT jsonb_agg(to_jsonb(m) ORDER BY version) FROM public._sqlx_migrations m),
              'constraints',(SELECT jsonb_agg(jsonb_build_array(c.conrelid::regclass::text,c.conname,pg_get_constraintdef(c.oid)) ORDER BY c.conrelid::regclass::text,c.conname) FROM pg_constraint c WHERE c.connamespace='public'::regnamespace),
              'indexes',(SELECT jsonb_agg(indexdef ORDER BY indexname) FROM pg_indexes WHERE schemaname='public'),
              'tables',(SELECT jsonb_agg(tablename ORDER BY tablename) FROM pg_tables WHERE schemaname='public'))
        "#).fetch_one(&self.pool).await, "snapshot owned target before refusal")
    }

    async fn assert_expansion(&self) {
        let (binary, triple, receipt): (bool, bool, bool) = required(sqlx::query_as("SELECT EXISTS(SELECT 1 FROM pg_constraint WHERE conrelid='public.memberships'::regclass AND conname='memberships_user_id_organization_id_key'),EXISTS(SELECT 1 FROM pg_constraint WHERE conrelid='public.memberships'::regclass AND conname='memberships_user_organization_type_key'),to_regclass('public.identity_membership_readiness_receipts') IS NOT NULL").fetch_one(&self.pool).await, "read expansion constraints");
        assert!(
            binary && triple && !receipt,
            "refusal must retain binary/triple constraints without a receipt table"
        );
    }
}

#[derive(Clone, Copy)]
struct Employee {
    user: Uuid,
    organization: Uuid,
    membership: Uuid,
    role: Uuid,
    grant: Uuid,
}

async fn seed_employee(target: &Target) -> Employee {
    let tenant = Uuid::now_v7();
    let employee = Employee {
        user: Uuid::now_v7(),
        organization: Uuid::now_v7(),
        membership: Uuid::now_v7(),
        role: Uuid::now_v7(),
        grant: Uuid::now_v7(),
    };
    let application = Uuid::now_v7();
    required(
        sqlx::query(
            "INSERT INTO tenants(id,name,code) VALUES($1,'Fixture only','phase06-rollout')",
        )
        .bind(tenant)
        .execute(&target.pool)
        .await,
        "seed owned tenant",
    );
    required(sqlx::query("INSERT INTO organizations(id,tenant_id,name,code) VALUES($1,$2,'Fixture only','phase06-rollout')").bind(employee.organization).bind(tenant).execute(&target.pool).await, "seed owned organization");
    required(sqlx::query("INSERT INTO users(id,username,password_hash) VALUES($1,'phase06-rollout','fixture-only-unusable-password-hash')").bind(employee.user).execute(&target.pool).await, "seed owned account");
    required(sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,position) VALUES($1,$2,$3,'EMPLOYEE','before rollout')").bind(employee.membership).bind(employee.user).bind(employee.organization).execute(&target.pool).await, "seed existing employee identity");
    required(sqlx::query("INSERT INTO roles(id,tenant_id,organization_id,name,code) VALUES($1,$2,$3,'Fixture role','fixture-role')").bind(employee.role).bind(tenant).bind(employee.organization).execute(&target.pool).await, "seed existing employee role");
    required(
        sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3)")
            .bind(employee.user)
            .bind(employee.role)
            .bind(employee.organization)
            .execute(&target.pool)
            .await,
        "seed role assignment",
    );
    required(
        sqlx::query(
            "INSERT INTO applications(id,code,name) VALUES($1,'phase06-rollout','Fixture app')",
        )
        .bind(application)
        .execute(&target.pool)
        .await,
        "seed grant application",
    );
    required(sqlx::query("INSERT INTO membership_data_grants(id,membership_id,application_id,resource_code,action_code,scope_type) VALUES($1,$2,$3,'fixture','read','ORG')").bind(employee.grant).bind(employee.membership).bind(application).execute(&target.pool).await, "seed existing employee grant");
    employee
}

fn unique_violation(error: &sqlx::Error) {
    assert_eq!(
        error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23505"),
        "same-type duplicates retain the unique-violation category mapped to HTTP 409"
    );
}

#[tokio::test]
async fn rollout_cli_arguments_dotenv_and_ambient_pg_are_sanitized_before_connection() {
    let directory = Directory::create();
    required(
        std::fs::write(
            directory.0.join(".env"),
            "MEMBERSHIP_ROLLOUT_DATABASE_URL=postgresql://private-canary@127.0.0.1:1/disposable\n",
        ),
        "write dotenv canary",
    );
    for values in [
        vec![],
        vec!["--unknown-private-canary"],
        vec!["apply"],
        vec![
            "inspect",
            "--platform-binary",
            PLATFORM,
            "--environment",
            "test",
        ],
    ] {
        let mut command = directory.command(ROLLOUT);
        command.args(values).env("PGPASSWORD", "private-canary");
        Process::start(command)
            .finish()
            .await
            .refusal(ARGUMENTS_ERROR);
    }
    let mut missing = directory.command(ROLLOUT);
    missing
        .args([
            "inspect",
            "--platform-binary",
            PLATFORM,
            "--environment",
            "development",
        ])
        .env("APP_ENV_FILE", directory.0.join("private-canary.env"));
    Process::start(missing)
        .finish()
        .await
        .refusal("MEMBERSHIP_READINESS_CONFIGURATION_INVALID");
    let mut ambient = directory.command(ROLLOUT);
    ambient
        .args([
            "inspect",
            "--platform-binary",
            PLATFORM,
            "--environment",
            "development",
        ])
        .env("PGPASSWORD", "private-canary");
    Process::start(ambient)
        .finish()
        .await
        .refusal("MEMBERSHIP_ROLLOUT_AMBIENT_PG_CONFIGURATION_UNSUPPORTED");
}

#[tokio::test]
#[ignore = "explicit loopback IDENTITY_PHASE06_TEST_DATABASE_URL; creates and drops owned rollout database"]
async fn membership_rollout_phase06_inspect_apply_and_identity_contract() {
    isolated(|target| async move {
        let employee = seed_employee(&target).await;
        let before = target.snapshot().await;
        let inspected = target.inspect().await;
        assert_eq!(inspected["operation"], "inspect");
        assert_eq!(inspected["database"], target.name);
        assert_eq!(inspected["binary_key_present"], true);
        assert_eq!(before, target.snapshot().await, "inspect is catalog/data read only");
        target.assert_expansion().await;
        let manifest = target.fixture_manifest(&inspected).await;
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        let applied = target.apply(&manifest).await.report();
        assert_eq!(applied["operation"], "apply");
        assert_eq!(applied["binary_key_present"], false);
        assert_eq!(applied["migration_sha256"], inspected["migration_sha256"]);
        assert_eq!(applied["manifest_sha256"], hex::encode(Sha256::digest(manifest_bytes)));
        let receipt_id: Uuid = serde_json::from_value(applied["receipt_id"].clone()).unwrap();
        assert_eq!(receipt_id.get_version_num(), 7);
        let stored: Value = required(sqlx::query_scalar("SELECT to_jsonb(r) FROM identity_membership_readiness_receipts r WHERE id=$1").bind(receipt_id).fetch_one(&target.pool).await, "read actual rollout receipt");
        for key in ["database_oid", "cluster_system_identifier", "platform_binary_sha256", "manifest_sha256", "migration_sha256", "schema_sha256"] { assert_eq!(stored[key], applied[key]); }
        assert_eq!(stored["database_name"], target.name);
        assert_eq!(stored["environment"], "development");
        assert_eq!(stored["scope"], "TASK_OWNED");
        let mut invalid_variant = *Uuid::now_v7().as_bytes();
        invalid_variant[8] = (invalid_variant[8] & 0x3f) | 0xc0;
        for invalid in [Uuid::new_v4(), Uuid::from_bytes(invalid_variant)] {
            let result = sqlx::query("INSERT INTO identity_membership_readiness_receipts SELECT $1,environment,scope,database_name,database_oid,cluster_system_identifier,platform_binary_sha256,manifest_sha256,migration_sha256,schema_sha256,applied_at,inventory,gates FROM identity_membership_readiness_receipts WHERE id=$2")
                .bind(invalid).bind(receipt_id).execute(&target.pool).await;
            let error = result.expect_err("receipt UUID rule must reject v4 and non-RFC variant");
            let database = error.as_database_error().expect("database check error");
            assert_eq!(database.code().as_deref(), Some("23514"));
            assert_eq!(database.constraint(), Some("uuid_v7_id"));
        }
        let second = target.apply(&manifest).await.report();
        assert_ne!(applied["receipt_id"], second["receipt_id"], "explicit reattestation issues a distinct receipt");
        assert_eq!(applied["schema_sha256"], second["schema_sha256"]);
        let receipts: i64 = required(sqlx::query_scalar("SELECT count(*) FROM identity_membership_readiness_receipts").fetch_one(&target.pool).await, "count reattestations");
        assert_eq!(receipts, 2);
        let after_apply = target.snapshot().await;
        assert_eq!(target.inspect().await["binary_key_present"], false);
        assert_eq!(after_apply, target.snapshot().await);
        let customer = Uuid::now_v7();
        required(sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES($1,$2,$3,'CUSTOMER')").bind(customer).bind(employee.user).bind(employee.organization).execute(&target.pool).await, "contracted schema permits same-account same-org second identity");
        let result = sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES($1,$2,$3,'EMPLOYEE')").bind(Uuid::now_v7()).bind(employee.user).bind(employee.organization).execute(&target.pool).await;
        unique_violation(&result.expect_err("same-type duplicate must be refused"));
        let retained: Uuid = required(sqlx::query_scalar("INSERT INTO memberships(id,user_id,organization_id,member_type,position) VALUES($1,$2,$3,'EMPLOYEE','after rollout') ON CONFLICT(user_id,organization_id,member_type) DO UPDATE SET position=EXCLUDED.position RETURNING id").bind(Uuid::now_v7()).bind(employee.user).bind(employee.organization).fetch_one(&target.pool).await, "real ternary employee UPSERT");
        assert_eq!(retained, employee.membership);
        let role: Uuid = required(sqlx::query_scalar("SELECT role_id FROM user_roles WHERE user_id=$1").bind(employee.user).fetch_one(&target.pool).await, "existing employee role retained");
        let grant: Uuid = required(sqlx::query_scalar("SELECT id FROM membership_data_grants WHERE membership_id=$1").bind(employee.membership).fetch_one(&target.pool).await, "existing employee grant retained");
        assert_eq!(role, employee.role); assert_eq!(grant, employee.grant);
        let identities: Vec<(Uuid, String)> = required(sqlx::query_as("SELECT id,member_type FROM memberships WHERE user_id=$1 AND organization_id=$2 ORDER BY member_type").bind(employee.user).bind(employee.organization).fetch_all(&target.pool).await, "read exact dual identities");
        assert_eq!(identities, vec![(customer, "CUSTOMER".into()), (employee.membership, "EMPLOYEE".into())]);
        let race_user = Uuid::now_v7();
        required(sqlx::query("INSERT INTO users(id,username,password_hash) VALUES($1,'phase06-rollout-race','fixture-only-unusable-password-hash')").bind(race_user).execute(&target.pool).await, "seed concurrent duplicate account");
        let left = sqlx::query_scalar::<_, Uuid>("INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES($1,$2,$3,'EMPLOYEE') RETURNING id")
            .bind(Uuid::now_v7()).bind(race_user).bind(employee.organization);
        let right = sqlx::query_scalar::<_, Uuid>("INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES($1,$2,$3,'EMPLOYEE') RETURNING id")
            .bind(Uuid::now_v7()).bind(race_user).bind(employee.organization);
        let (left, right) = tokio::join!(left.fetch_one(&target.pool), right.fetch_one(&target.pool));
        let outcomes = [left, right];
        assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
        for error in outcomes.iter().filter_map(|result| result.as_ref().err()) { unique_violation(error); }
    }).await;
}

#[tokio::test]
#[ignore = "explicit loopback IDENTITY_PHASE06_TEST_DATABASE_URL; creates and drops owned rollout database"]
async fn membership_rollout_phase06_refusals_are_atomic() {
    isolated(|target| async move {
        seed_employee(&target).await;
        let inspected = target.inspect().await;
        let valid = target.fixture_manifest(&inspected).await;
        for fault in 0..7 {
            let mut invalid = valid.clone();
            let code = match fault {
                0 => { invalid["target_database"] = format!("{PREFIX}wrong_target").into(); "MEMBERSHIP_ROLLOUT_TARGET_MISMATCH" }
                1 => { invalid["target_database_oid"] = (valid["target_database_oid"].as_i64().unwrap() + 1).into(); "MEMBERSHIP_ROLLOUT_TARGET_MISMATCH" }
                2 => { invalid["gates"][0]["evidence"]["sha256"] = "0".repeat(64).into(); "MEMBERSHIP_ROLLOUT_EVIDENCE_INVALID" }
                3 => { let _ = invalid["gates"].as_array_mut().unwrap().pop(); "MEMBERSHIP_ROLLOUT_EVIDENCE_INVALID" }
                4 => { invalid["unknown_fixture_field"] = true.into(); "MEMBERSHIP_ROLLOUT_EVIDENCE_INVALID" }
                5 => { invalid["platform_binary_sha256"] = "0".repeat(64).into(); invalid["inventory"][0]["binary_sha256"] = "0".repeat(64).into(); "MEMBERSHIP_ROLLOUT_TARGET_MISMATCH" }
                _ => { invalid["target_cluster_system_identifier"] = "1".into(); "MEMBERSHIP_ROLLOUT_TARGET_MISMATCH" }
            };
            let before = target.snapshot().await;
            target.apply(&invalid).await.refusal(code);
            assert_eq!(before, target.snapshot().await, "manifest refusal {fault} is atomic");
            target.assert_expansion().await;
        }
        required(sqlx::query("CREATE TABLE public.fixture_binary_dependency(user_id uuid,organization_id uuid,FOREIGN KEY(user_id,organization_id) REFERENCES public.memberships(user_id,organization_id))").execute(&target.pool).await, "create real binary-key foreign-key dependency");
        let before = target.snapshot().await;
        target.apply(&valid).await.refusal("MEMBERSHIP_ROLLOUT_BINARY_KEY_HAS_DEPENDENT_FOREIGN_KEYS");
        assert_eq!(before, target.snapshot().await);
        target.assert_expansion().await;
        required(sqlx::query("DROP TABLE public.fixture_binary_dependency").execute(&target.pool).await, "retire owned dependency fixture");
        required(sqlx::query("CREATE UNIQUE INDEX fixture_alternate_binary_key ON public.memberships(organization_id,user_id)").execute(&target.pool).await, "create alternate binary unique index");
        let before = target.snapshot().await;
        target.apply(&valid).await.refusal("MEMBERSHIP_ROLLOUT_SCHEMA_INVALID");
        assert_eq!(before, target.snapshot().await);
        target.assert_expansion().await;
        required(sqlx::query("DROP INDEX public.fixture_alternate_binary_key").execute(&target.pool).await, "retire owned alternate key fixture");
        assert_eq!(target.apply(&valid).await.report()["operation"], "apply");
        for (alter, restore) in [
            ("ALTER TABLE public.identity_membership_readiness_receipts DROP CONSTRAINT uuid_v7_id", Some("ALTER TABLE public.identity_membership_readiness_receipts ADD CONSTRAINT uuid_v7_id CHECK(public.is_uuid_v7(id))")),
            ("ALTER TABLE public.identity_membership_readiness_receipts DROP CONSTRAINT identity_membership_readiness_receipts_pkey", None),
        ] {
            required(sqlx::query(alter).execute(&target.pool).await, "introduce owned receipt schema drift");
            let before = target.snapshot().await;
            let mut command = target.rollout();
            command.args(["inspect", "--platform-binary", PLATFORM, "--environment", "development"]);
            Process::start(command).finish().await.refusal("MEMBERSHIP_ROLLOUT_SCHEMA_INVALID");
            assert_eq!(before, target.snapshot().await, "receipt catalog drift is refused read only");
            if let Some(restore) = restore { required(sqlx::query(restore).execute(&target.pool).await, "restore v7 fixture check before independent PK drift"); }
        }
    }).await;
}

fn platform_command(
    target: &Target,
    address: std::net::SocketAddr,
    receipt: Option<(&str, &str)>,
) -> Command {
    let mut command = command(PLATFORM, &target.directory);
    command
        .env("DATABASE_URL", &target.url)
        .env("APP_ENV", "development")
        .env("APPLICATION_ENVIRONMENT", "development")
        .env("BIND_ADDR", address.to_string())
        .env(
            "JWT_SECRET",
            "phase06-fixture-only-strong-independent-secret-72",
        )
        .env("AUTO_CREATE_DATABASE", "false")
        .env("DB_MAX_CONNECTIONS", "4")
        .env("MEMBERSHIP_LIFECYCLE_ENABLED", "true")
        .env("CUSTOMER_MEMBERSHIPS_ENABLED", "true")
        .env("DUAL_MEMBERSHIPS_ENABLED", "true")
        .env("RUST_LOG", "warn");
    if let Some((receipt, manifest)) = receipt {
        command
            .env("MEMBERSHIP_READINESS_RECEIPT", receipt)
            .env("MEMBERSHIP_READINESS_MANIFEST_SHA256", manifest);
    }
    command
}

async fn loopback_address() -> std::net::SocketAddr {
    let listener = required(
        tokio::net::TcpListener::bind("127.0.0.1:0").await,
        "reserve random loopback test port",
    );
    required(listener.local_addr(), "read loopback test address")
}

#[tokio::test]
#[ignore = "explicit loopback IDENTITY_PHASE06_TEST_DATABASE_URL; real platform binary startup with fixture-only receipt"]
async fn membership_rollout_phase06_development_startup_requires_live_receipt() {
    isolated(|target| async move {
        // Existing fixture data avoids unrelated demonstration credential creation
        // and the 665923-row region import during this focused startup contract.
        seed_employee(&target).await;
        required(sqlx::query("INSERT INTO region_versions(version,label,imported) VALUES('2023','fixture-only startup preparation',true)").execute(&target.pool).await, "prepare explicit existing region fixture marker");
        let inspected = target.inspect().await;
        let manifest = target.fixture_manifest(&inspected).await;
        let receipt = target.apply(&manifest).await.report();
        let receipt_id = receipt["receipt_id"].as_str().unwrap();
        let manifest_sha = receipt["manifest_sha256"].as_str().unwrap();
        let before = target.snapshot().await;
        for (proof, expected) in [
            (None, "Error: MEMBERSHIP_READINESS_REQUIRED: identity writes require an environment-bound Phase 6 receipt"),
            (Some((receipt_id, "0000000000000000000000000000000000000000000000000000000000000000")), "Error: MEMBERSHIP_READINESS_VERIFICATION_FAILED"),
        ] {
            let output = Process::start(platform_command(&target, loopback_address().await, proof)).finish().await;
            assert!(!output.success);
            assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), expected);
            assert_eq!(before, target.snapshot().await, "startup refusal precedes seed and workers");
        }
        let address = loopback_address().await;
        let mut server = Process::start(platform_command(&target, address, Some((receipt_id, manifest_sha))));
        let client = required(reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(2)).build(), "build loopback health client");
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            assert!(required(server.child.try_wait(), "inspect platform child status").is_none(), "valid receipt platform exited before health");
            if let Ok(response) = client.get(format!("http://{address}/api/health")).send().await {
                if response.status().is_success() {
                    let health: Value = required(response.json().await, "parse actual health route response");
                    assert_eq!(health["status"], "UP"); assert_eq!(health["database"], "UP");
                    break;
                }
            }
            assert!(Instant::now() < deadline, "platform health did not become ready within bounded wait");
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        server.stop().await;
        required(sqlx::query("ALTER TABLE public.identity_membership_readiness_receipts DROP CONSTRAINT uuid_v7_id").execute(&target.pool).await, "introduce receipt check drift after actual healthy startup");
        let after_drift = target.snapshot().await;
        let output = Process::start(platform_command(&target, loopback_address().await, Some((receipt_id, manifest_sha)))).finish().await;
        assert!(!output.success);
        assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), "Error: MEMBERSHIP_READINESS_VERIFICATION_FAILED");
        assert_eq!(after_drift, target.snapshot().await);
    }).await;
}
