//! Spawn the standalone administrator CLI against real Axum and owned PostgreSQL.
//! No CLI library dependency, dotenv, ZIP installation, or shared image volume is used.
use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use axum::body::{to_bytes, Body};
use axum::extract::{Request, State};
use axum::http::{header, Method, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::{routing::get, Router};
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::config::Config;
use crate::entity_creation_tests::Database;
use crate::state::AppState;
use crate::util::{hash, jwt};

const SECRET: &str = "owned-phase25-platform-session-signing-only";
const CLIENT_SECRET: &str = "owned-phase25-confidential-client-secret";
const CHECKS: [&str; 13] = [
    "plan-and-no-apply-no-database-writes",
    "bootstrap-repeat-preserves-ids-and-counts",
    "actual-client-id-associated-with-own-application",
    "foreign-client-and-conflicting-menu-rejected",
    "register-only-existing-allowed-fields",
    "existing-release-version-rejected-and-queryable",
    "url-preflight-two-activations-deployments-rollback",
    "grant-plan-and-no-apply-offline-readonly",
    "grant-apply-formal-api-and-exact-direct-membership",
    "grant-independent-read-write-and-root-context",
    "grant-no-permission-organization-and-cross-app-denied",
    "same-token-next-context-and-member-scope",
    "grant-request-bound-audit-and-secret-free-output",
];

#[derive(Clone)]
struct RecordedRequest {
    method: Method,
    path: String,
    body: Value,
    status: StatusCode,
    request_id: Option<Uuid>,
}

type Requests = Arc<Mutex<Vec<RecordedRequest>>>;

async fn record(State(log): State<Requests>, request: Request, next: Next) -> Response {
    let (parts, body) = request.into_parts();
    let Ok(bytes) = to_bytes(body, 2 * 1024 * 1024).await else {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    };
    let method = parts.method.clone();
    let path = parts.uri.path().to_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let response = next
        .run(Request::from_parts(parts, Body::from(bytes)))
        .await;
    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    log.lock()
        .expect("owned request recorder lock")
        .push(RecordedRequest {
            method,
            path,
            body,
            status: response.status(),
            request_id,
        });
    response
}

struct Server {
    origin: String,
    task: JoinHandle<std::io::Result<()>>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Server {
    async fn start(router: Router) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
        });
        Ok(Self {
            origin,
            task,
            stop: Some(stop),
        })
    }

    async fn close(mut self) -> Result<()> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        tokio::time::timeout(Duration::from_secs(5), &mut self.task)
            .await
            .map_err(|_| anyhow!("owned Phase25 HTTP server did not stop"))???;
        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Files(PathBuf);

impl Files {
    fn create() -> Result<Self> {
        let path = std::env::temp_dir().join(format!("phase25-cli-{}", Uuid::new_v4().simple()));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn json(&self, name: &str, value: &Value) -> Result<String> {
        ensure!(
            name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
            "invalid owned fixture name"
        );
        let path = self.0.join(format!("{name}.json"));
        std::fs::write(&path, serde_json::to_vec(value)?)?;
        path.to_str()
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("owned fixture path is not UTF-8"))
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        if self.0.parent() == Some(std::env::temp_dir().as_path())
            && self
                .0
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("phase25-cli-"))
        {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

struct Cli<'a> {
    binary: &'a Path,
    files: &'a Files,
    origin: &'a str,
    bearer: &'a str,
    client_id: &'a str,
}

impl Cli<'_> {
    async fn call(&self, args: &[&str], expected_error: Option<&str>) -> Result<Value> {
        let mut command = tokio::process::Command::new(self.binary);
        command
            .current_dir(&self.files.0)
            .env_clear()
            .args(args)
            .args(["--environment", "development", "--allow-loopback-http"])
            .env("PLATFORM_API_ORIGIN", self.origin)
            .env("PLATFORM_ADMIN_BEARER", self.bearer)
            .env("PLATFORM_CLIENT_ID", self.client_id)
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for name in [
            "PATH",
            "Path",
            "SystemRoot",
            "SYSTEMROOT",
            "WINDIR",
            "TEMP",
            "TMP",
            "TMPDIR",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        let child = command
            .spawn()
            .map_err(|_| anyhow!("unable to spawn explicit Phase25 CLI binary"))?;
        let output = tokio::time::timeout(Duration::from_secs(50), child.wait_with_output())
            .await
            .map_err(|_| anyhow!("Phase25 CLI deadline exceeded"))??;
        ensure!(
            output.stdout.len() + output.stderr.len() <= 1024 * 1024,
            "Phase25 CLI output exceeded budget"
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        for secret in [SECRET, CLIENT_SECRET, self.bearer] {
            ensure!(
                !stdout.contains(secret) && !stderr.contains(secret),
                "Phase25 CLI exposed fixture credential material"
            );
        }
        ensure!(
            !stdout.contains("postgres://")
                && !stderr.contains("postgres://")
                && !stdout.contains("postgresql://")
                && !stderr.contains("postgresql://"),
            "Phase25 CLI exposed database configuration"
        );
        if let Some(category) = expected_error {
            ensure!(
                !output.status.success() && stdout.trim().is_empty(),
                "{} should fail without stdout",
                args[0]
            );
            let error: Value = serde_json::from_str(stderr.trim())
                .map_err(|_| anyhow!("CLI error was not safe JSON"))?;
            ensure!(
                error["category"] == category,
                "{} error category mismatch: observed {:?}, HTTP status {:?}",
                args[0],
                error["category"].as_str(),
                error["status"].as_u64()
            );
            Ok(error)
        } else {
            let error = serde_json::from_str::<Value>(stderr.trim()).unwrap_or(Value::Null);
            ensure!(
                output.status.success(),
                "{} CLI failed with category {:?}, HTTP status {:?}",
                args[0],
                error["category"].as_str(),
                error["status"].as_u64()
            );
            ensure!(stderr.trim().is_empty(), "successful CLI wrote stderr");
            serde_json::from_str(stdout.trim())
                .map_err(|_| anyhow!("successful CLI output was not JSON"))
        }
    }
}

async fn tenant(pool: &PgPool, code: &str, platform: bool) -> Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO tenants(name,code,is_platform) VALUES($1,$1,$2) RETURNING id",
    )
    .bind(code)
    .bind(platform)
    .fetch_one(pool)
    .await?)
}

async fn org(pool: &PgPool, tenant: Uuid, parent: Option<Uuid>, name: &str) -> Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO organizations(tenant_id,parent_id,name) VALUES($1,$2,$3) RETURNING id",
    )
    .bind(tenant)
    .bind(parent)
    .bind(name)
    .fetch_one(pool)
    .await?)
}

async fn user(
    pool: &PgPool,
    name: &str,
    super_admin: bool,
    organization: Uuid,
) -> Result<(Uuid, Uuid)> {
    let id = sqlx::query_scalar("INSERT INTO users(username,real_name,password_hash,is_super_admin) VALUES($1,$1,'unused-owned-fixture-password',$2) RETURNING id")
        .bind(name).bind(super_admin).fetch_one(pool).await?;
    let membership = sqlx::query_scalar(
        "INSERT INTO memberships(user_id,organization_id) VALUES($1,$2) RETURNING id",
    )
    .bind(id)
    .bind(organization)
    .fetch_one(pool)
    .await?;
    Ok((id, membership))
}

async fn role(pool: &PgPool, tenant: Uuid, org: Uuid, user: Uuid, menus: &[Uuid]) -> Result<Uuid> {
    let role = sqlx::query_scalar(
        "INSERT INTO roles(tenant_id,organization_id,name,code) VALUES($1,$2,$3,$3) RETURNING id",
    )
    .bind(tenant)
    .bind(org)
    .bind(format!("phase25-role-{}", Uuid::new_v4().simple()))
    .fetch_one(pool)
    .await?;
    sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3)")
        .bind(user)
        .bind(role)
        .bind(org)
        .execute(pool)
        .await?;
    for menu in menus {
        sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)")
            .bind(role)
            .bind(menu)
            .execute(pool)
            .await?;
    }
    Ok(role)
}

async fn session(pool: &PgPool, user: Uuid, tenant: Uuid, org: Uuid) -> Result<String> {
    let ver = sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
        .bind(user)
        .fetch_one(pool)
        .await?;
    let sid: Uuid = sqlx::query_scalar("INSERT INTO platform_sessions(user_id,tenant_id,organization_id,expires_at) VALUES($1,$2,$3,now()+interval '1 hour') RETURNING id")
        .bind(user).bind(tenant).bind(org).fetch_one(pool).await?;
    let now = chrono::Utc::now().timestamp();
    jwt::sign(
        &jwt::Claims {
            sub: user.to_string(),
            typ: "session".into(),
            tenant: Some(tenant.to_string()),
            org: Some(org.to_string()),
            sid: Some(sid.to_string()),
            ver: Some(ver),
            cid: None,
            scope: None,
            aud: None,
            jti: None,
            iat: now,
            exp: now + 3600,
        },
        SECRET,
    )
}

async fn authorize(
    pool: &PgPool,
    tenant: Uuid,
    orgs: &[Uuid],
    app: Uuid,
    menus: &[Uuid],
) -> Result<()> {
    sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
        .bind(tenant)
        .bind(app)
        .execute(pool)
        .await?;
    for org in orgs {
        sqlx::query(
            "INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)",
        )
        .bind(org)
        .bind(app)
        .execute(pool)
        .await?;
    }
    for menu in menus {
        sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
            .bind(tenant)
            .bind(menu)
            .execute(pool)
            .await?;
        for org in orgs {
            sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
                .bind(org)
                .bind(menu)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

async fn snapshot(pool: &PgPool) -> Result<BTreeMap<String, String>> {
    let tables: Vec<String> = sqlx::query_scalar("SELECT tablename FROM pg_tables WHERE schemaname='public' AND tablename<>'_sqlx_migrations' ORDER BY tablename").fetch_all(pool).await?;
    let mut state = BTreeMap::new();
    for table in tables {
        ensure!(
            table
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "unexpected owned table identifier"
        );
        let rows = sqlx::query_scalar(&format!("SELECT COALESCE(jsonb_agg(to_jsonb(r) ORDER BY to_jsonb(r)::text),'[]'::jsonb)::text FROM \"{table}\" r")).fetch_one(pool).await?;
        state.insert(table, rows);
    }
    Ok(state)
}

fn uuid(value: &Value, field: &str) -> Result<Uuid> {
    value[field]
        .as_str()
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| anyhow!("CLI output omitted valid {field}"))
}

fn config(artifact_origin: &str, policy: &Value) -> Result<Config> {
    Config::from_values(&HashMap::from([
        ("APPLICATION_ENVIRONMENT".into(), "development".into()),
        (
            "DATABASE_URL".into(),
            "postgres://unused/owned-phase25-cli".into(),
        ),
        ("JWT_SECRET".into(), SECRET.into()),
        ("MICRO_GATEWAY_ORIGIN".into(), artifact_origin.into()),
        ("MICRO_ALLOWED_ORIGINS".into(), artifact_origin.into()),
        ("DATA_GRANT_POLICY".into(), policy.to_string()),
    ]))
}

async fn platform(
    pool: &PgPool,
    artifact_origin: &str,
    policy: &Value,
    log: &Requests,
) -> Result<Server> {
    let state = AppState::new(pool.clone(), Arc::new(config(artifact_origin, policy)?));
    let router = crate::routes::api_router()
        .nest("/oauth", crate::routes::oauth2::oauth_router())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::observability::observe,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::normalize_client_ip,
        ))
        .layer(middleware::from_fn_with_state(log.clone(), record))
        .with_state(state);
    Server::start(router).await
}

async fn artifact(uri: Uri) -> Response {
    let path = uri.path();
    if !["v1", "v2"]
        .iter()
        .any(|v| path.starts_with(&format!("/sub-apps/work-orders/versions/{v}/")))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    match path.rsplit('/').next() {
        Some("index.html") => ([(header::CONTENT_TYPE, "text/html")], "<!doctype html><html><body>Owned Phase25 URL release<script src=\"./app.js\"></script></body></html>").into_response(),
        Some("app.js") => ([(header::CONTENT_TYPE, "text/javascript"), (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")], "document.body.dataset.phase25 = 'ready';").into_response(),
        Some("health") => ([(header::CONTENT_TYPE, "text/plain")], "ready").into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

struct Reply {
    status: StatusCode,
    body: Value,
    request_id: Uuid,
}

#[derive(sqlx::FromRow)]
struct GrantAudit {
    user_id: Uuid,
    tenant_id: Option<Uuid>,
    organization_id: Option<Uuid>,
    application_id: Option<Uuid>,
    detail: String,
}

async fn http(origin: &str, token: &str, method: Method, path: &str, body: Value) -> Result<Reply> {
    let request_id = Uuid::new_v4();
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()?;
    let mut request = client
        .request(method, format!("{origin}{path}"))
        .bearer_auth(token)
        .header("x-request-id", request_id.to_string());
    if !body.is_null() {
        request = request
            .header(header::CONTENT_TYPE, "application/json")
            .body(serde_json::to_vec(&body)?);
    }
    let response = request
        .send()
        .await
        .map_err(|_| anyhow!("owned Phase25 HTTP request failed"))?;
    ensure!(
        response
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            == Some(request_id.to_string().as_str()),
        "real platform lost request ID"
    );
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|_| anyhow!("owned Phase25 HTTP response incomplete"))?;
    ensure!(
        bytes.len() <= 2 * 1024 * 1024,
        "owned HTTP response exceeded budget"
    );
    Ok(Reply {
        status,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        request_id,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicit owned TEST_DATABASE_URL and prebuilt absolute PHASE25_CLI_BIN"]
async fn real_cli_platform_round_trip() -> Result<()> {
    ensure!(
        std::env::var("TEST_DATABASE_URL").is_ok_and(|v| !v.trim().is_empty()),
        "explicit owned TEST_DATABASE_URL required; no dotenv fallback"
    );
    let binary = std::env::var_os("PHASE25_CLI_BIN")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("explicit prebuilt PHASE25_CLI_BIN required"))?;
    ensure!(
        binary.is_absolute() && binary.is_file(),
        "PHASE25_CLI_BIN must identify an existing absolute binary"
    );
    let database = Database::create().await?;
    let result = run_live(&database.pool, &binary).await;
    database.finish(result).await
}

async fn run_live(pool: &PgPool, binary: &Path) -> Result<()> {
    let files = Files::create()?;
    let log = Requests::default();
    let artifacts = Server::start(Router::new().route("/{*path}", get(artifact))).await?;
    let server = platform(
        pool,
        &artifacts.origin,
        &json!({"applications":[],"managementScopes":[]}),
        &log,
    )
    .await?;
    let result = bootstrap_and_releases(
        pool,
        binary,
        &files,
        &server.origin,
        &artifacts.origin,
        &log,
    )
    .await;
    let shutdown = server.close().await;
    let artifact_shutdown = artifacts.close().await;
    if result.is_err() {
        // Only method, normalized platform route, and numeric status are emitted.
        // Headers, bodies, queries, credentials and arbitrary error text stay private.
        let records = log.lock().expect("owned recorder");
        for request in records.iter().skip(records.len().saturating_sub(12)) {
            let route = request
                .path
                .split('/')
                .map(|segment| {
                    if Uuid::parse_str(segment).is_ok() {
                        "{id}"
                    } else {
                        segment
                    }
                })
                .collect::<Vec<_>>()
                .join("/");
            eprintln!(
                "PHASE25_HTTP_TRACE {}",
                json!({"method":request.method.as_str(),"route":route,"status":request.status.as_u16()})
            );
        }
    }
    result?;
    shutdown?;
    artifact_shutdown
}

async fn bootstrap_and_releases(
    pool: &PgPool,
    binary: &Path,
    files: &Files,
    origin: &str,
    artifact_origin: &str,
    log: &Requests,
) -> Result<()> {
    let platform_tenant = tenant(pool, "phase25-platform", true).await?;
    let root = org(pool, platform_tenant, None, "Phase25 platform root").await?;
    let child = org(pool, platform_tenant, Some(root), "Phase25 platform child").await?;
    let system: Uuid = sqlx::query_scalar(
        "INSERT INTO applications(name,code) VALUES('Phase25 management','system') RETURNING id",
    )
    .fetch_one(pool)
    .await?;
    let mut management = Vec::new();
    for permission in [
        "system:application",
        "system:menu",
        "system:oauth",
        "system:tenant",
        "system:organization",
        "system:user",
        "system:role",
        "system:authorization",
        "system:data-grant:read",
        "system:data-grant:write",
    ] {
        // OAuth administration requires this exact MENU page as well as its
        // permission code; a generic BUTTON does not authorize the client page.
        let (kind, path, visible) = if permission == "system:oauth" {
            ("MENU", "/system/oauth/clients", true)
        } else {
            ("BUTTON", "", false)
        };
        let menu = sqlx::query_scalar("INSERT INTO menus(application_id,type,title,permission,visible,path) VALUES($1,$2,$3,$3,$4,$5) RETURNING id")
            .bind(system).bind(kind).bind(permission).bind(visible).bind(path).fetch_one(pool).await?;
        management.push(menu);
    }
    authorize(pool, platform_tenant, &[root, child], system, &management).await?;
    let (actor, _) = user(pool, "phase25-admin", true, root).await?;
    let admin = session(pool, actor, platform_tenant, root).await?;
    let client_id = format!("phase25-generated-{}", Uuid::new_v4().simple());
    let client: Uuid = sqlx::query_scalar("INSERT INTO oauth_clients(client_id,client_name,client_secret_hash,client_type,grant_types,scopes,access_token_validity,refresh_token_validity) VALUES($1,'work-orders-client',$2,'CONFIDENTIAL','client_credentials','openid',300,300) RETURNING id")
        .bind(&client_id).bind(hash::sha256_hex(CLIENT_SECRET)).fetch_one(pool).await?;
    let cli = Cli {
        binary,
        files,
        origin,
        bearer: &admin,
        client_id: &client_id,
    };
    let release = |version: &str| {
        json!({"environment":"development","version":version,"entryType":"HTML",
        "entryUrl":format!("{artifact_origin}/sub-apps/work-orders/versions/{version}/index.html"),
        "healthUrl":format!("{artifact_origin}/sub-apps/work-orders/versions/{version}/health"),
        "resources":[{"url":format!("{artifact_origin}/sub-apps/work-orders/versions/{version}/app.js"),"kind":"SCRIPT"}]})
    };
    let release1 = files.json("release-v1", &release("v1"))?;
    let before = snapshot(pool).await?;
    for args in [
        vec!["plan"],
        vec!["bootstrap"],
        vec!["register", "--release-file", &release1],
        vec![
            "preflight",
            "--release-id",
            "00000000-0000-4000-8000-000000000001",
        ],
        vec![
            "activate",
            "--release-id",
            "00000000-0000-4000-8000-000000000001",
        ],
        vec!["rollback"],
    ] {
        let result = cli.call(&args, None).await?;
        ensure!(
            result["mode"] == "PLAN" && result["externalWrites"] == false,
            "write command without apply did not plan"
        );
    }
    ensure!(
        snapshot(pool).await? == before && log.lock().expect("owned recorder").is_empty(),
        "plan/no-apply contacted platform or changed database"
    );
    let first = cli.call(&["bootstrap", "--apply"], None).await?;
    let app = uuid(&first, "applicationId")?;
    ensure!(
        uuid(&first, "clientRecordId")? == client
            && first["created"]
                .as_array()
                .is_some_and(|v| v.contains(&json!("application"))),
        "bootstrap did not create application using actual client record"
    );
    let associated: (String, Option<Uuid>) =
        sqlx::query_as("SELECT client_id,application_id FROM oauth_clients WHERE id=$1")
            .bind(client)
            .fetch_one(pool)
            .await?;
    ensure!(
        associated == (client_id.clone(), Some(app)) && client_id != "work-orders-client",
        "bootstrap did not preserve real generated client ID"
    );
    let stable = snapshot(pool).await?;
    let second = cli.call(&["bootstrap", "--apply"], None).await?;
    ensure!(
        second["applicationId"] == first["applicationId"]
            && second["clientRecordId"] == first["clientRecordId"]
            && second["menuIds"] == first["menuIds"]
            && second["created"] == json!([])
            && snapshot(pool).await? == stable,
        "repeat bootstrap replaced IDs, rows, timestamps or audit"
    );
    let inspection = cli.call(&["inspect"], None).await?;
    ensure!(
        inspection["clientAssociated"] == true
            && inspection["missingMenus"] == json!([])
            && inspection["menuIds"] == first["menuIds"],
        "inspect failed to reconcile real bootstrap"
    );
    let other: Uuid = sqlx::query_scalar("INSERT INTO applications(name,code) VALUES('Other owned app','phase25-other') RETURNING id").fetch_one(pool).await?;
    sqlx::query("UPDATE oauth_clients SET application_id=$1 WHERE id=$2")
        .bind(other)
        .bind(client)
        .execute(pool)
        .await?;
    let conflict = snapshot(pool).await?;
    cli.call(&["bootstrap", "--apply"], Some("CLIENT_OWNERSHIP_CONFLICT"))
        .await?;
    ensure!(
        snapshot(pool).await? == conflict,
        "foreign client conflict changed state"
    );
    sqlx::query("UPDATE oauth_clients SET application_id=$1 WHERE id=$2")
        .bind(app)
        .bind(client)
        .execute(pool)
        .await?;
    let read = uuid(&first["menuIds"], "ticket:read")?;
    sqlx::query("UPDATE menus SET path='/phase25-conflicting-menu' WHERE id=$1")
        .bind(read)
        .execute(pool)
        .await?;
    let conflict = snapshot(pool).await?;
    cli.call(&["bootstrap", "--apply"], Some("MENU_CONFLICT"))
        .await?;
    ensure!(
        snapshot(pool).await? == conflict,
        "conflicting menu was adopted or changed"
    );
    sqlx::query("UPDATE menus SET path='/micro/work-orders/tickets/:id' WHERE id=$1")
        .bind(read)
        .execute(pool)
        .await?;
    let one = cli
        .call(&["register", "--release-file", &release1, "--apply"], None)
        .await?;
    let one_id = uuid(&one["release"], "id")?.to_string();
    ensure!(
        one["release"]["validationStatus"] == "DRAFT" && one["releaseActivated"] == false,
        "register activated or prevalidated release"
    );
    let recorded = log
        .lock()
        .expect("owned recorder")
        .iter()
        .find(|r| r.method == Method::POST && r.path == format!("/api/applications/{app}/releases"))
        .cloned()
        .ok_or_else(|| anyhow!("real register HTTP request missing"))?;
    let mut keys: Vec<_> = recorded
        .body
        .as_object()
        .ok_or_else(|| anyhow!("register request was not JSON object"))?
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    ensure!(
        keys == [
            "entryType",
            "entryUrl",
            "environment",
            "healthUrl",
            "resources",
            "version"
        ] && recorded.body == release("v1"),
        "register sent unsupported or transformed fields"
    );
    let conflict = snapshot(pool).await?;
    cli.call(
        &["register", "--release-file", &release1, "--apply"],
        Some("VERSION_EXISTS"),
    )
    .await?;
    ensure!(
        snapshot(pool).await? == conflict,
        "existing release version was overwritten"
    );
    let rows = cli.call(&["releases"], None).await?;
    ensure!(
        rows["releases"]
            .as_array()
            .is_some_and(|rows| rows.len() == 1 && rows[0]["id"] == one_id),
        "release query did not reconcile registered version"
    );
    cli.call(
        &["activate", "--release-id", &one_id, "--apply"],
        Some("PREFLIGHT_REQUIRED"),
    )
    .await?;
    cli.call(&["preflight", "--release-id", &one_id, "--apply"], None)
        .await?;
    cli.call(&["activate", "--release-id", &one_id, "--apply"], None)
        .await?;
    let release2 = files.json("release-v2", &release("v2"))?;
    let two = cli
        .call(&["register", "--release-file", &release2, "--apply"], None)
        .await?;
    let two_id = uuid(&two["release"], "id")?.to_string();
    cli.call(&["preflight", "--release-id", &two_id, "--apply"], None)
        .await?;
    cli.call(&["activate", "--release-id", &two_id, "--apply"], None)
        .await?;
    let deployed = cli.call(&["deployments"], None).await?;
    ensure!(
        deployed["deployments"][0]["activeReleaseId"] == two_id
            && deployed["deployments"][0]["previousReleaseId"] == one_id,
        "second activation lost deployment history"
    );
    let rolled = cli.call(&["rollback", "--apply"], None).await?;
    ensure!(
        rolled["deployment"]["activeReleaseId"] == one_id
            && rolled["deployment"]["previousReleaseId"] == two_id,
        "rollback did not restore earlier URL release"
    );
    let release_state: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM application_releases WHERE application_id=$1 AND validation_status='PASSED'),(SELECT count(*) FROM application_releases WHERE application_id=$1 AND artifact_digest IS NOT NULL),(SELECT count(*) FROM application_release_artifacts WHERE application_id=$1)").bind(app).fetch_one(pool).await?;
    ensure!(
        release_state == (2, 0, 0),
        "URL workflow installed ZIP artifacts or did not preflight both versions"
    );
    grant_round_trip(
        pool,
        &cli,
        GrantSetup {
            app,
            menus: &first["menuIds"],
            other,
            platform_tenant,
            root,
            child,
            actor,
            management: &management,
        },
        artifact_origin,
        log,
    )
    .await?;
    let counts: (i64, i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM applications WHERE code='work-orders'),(SELECT count(*) FROM menus WHERE application_id=$1),(SELECT count(*) FROM application_releases WHERE application_id=$1),(SELECT count(*) FROM audit_logs WHERE action='DATA_GRANT_CHANGE')").bind(app).fetch_one(pool).await?;
    eprintln!(
        "PHASE25_LIVE_DATABASE {}",
        json!({"bootstrapApplications":counts.0,"bootstrapMenus":counts.1,"urlReleases":counts.2,"zipArtifacts":0,"grantAudits":counts.3,"actualClientIdAssociated":true,"grantWritesOnlyViaHttp":true})
    );
    eprintln!(
        "PHASE25_LIVE_RESULT {}",
        json!({"schemaVersion":1,"passed":CHECKS.len(),"failed":0,"skipped":0,"checks":CHECKS.map(|id|json!({"id":id,"status":"PASS"}))})
    );
    Ok(())
}

struct GrantSetup<'a> {
    app: Uuid,
    menus: &'a Value,
    other: Uuid,
    platform_tenant: Uuid,
    root: Uuid,
    child: Uuid,
    actor: Uuid,
    management: &'a [Uuid],
}

fn grant_path(target: &Value) -> Result<String> {
    Ok(format!(
        "/api/data-grants?tenantId={}&organizationId={}&userId={}&membershipId={}&applicationId={}",
        uuid(target, "tenantId")?,
        uuid(target, "organizationId")?,
        uuid(target, "userId")?,
        uuid(target, "membershipId")?,
        uuid(target, "applicationId")?
    ))
}

async fn grant_round_trip(
    pool: &PgPool,
    cli: &Cli<'_>,
    setup: GrantSetup<'_>,
    artifact_origin: &str,
    log: &Requests,
) -> Result<()> {
    let GrantSetup {
        app,
        menus,
        other,
        platform_tenant,
        root,
        child,
        actor,
        management,
    } = setup;
    let business = tenant(pool, "phase25-business", false).await?;
    let business_root = org(pool, business, None, "Phase25 business root").await?;
    let branch = org(
        pool,
        business,
        Some(business_root),
        "Phase25 business branch",
    )
    .await?;
    let descendant = org(pool, business, Some(branch), "Phase25 business descendant").await?;
    let sibling = org(
        pool,
        business,
        Some(business_root),
        "Phase25 business sibling",
    )
    .await?;
    let (business_user, membership) = user(pool, "phase25-business-user", false, branch).await?;
    user(pool, "phase25-assignee", false, branch).await?;
    let (descendant_user, _) = user(pool, "phase25-descendant-assignee", false, descendant).await?;
    user(pool, "phase25-sibling-assignee", false, sibling).await?;
    let all_menus: Vec<Uuid> = menus
        .as_object()
        .ok_or_else(|| anyhow!("bootstrap menu IDs absent"))?
        .values()
        .map(|value| {
            value
                .as_str()
                .and_then(|id| id.parse().ok())
                .ok_or_else(|| anyhow!("invalid bootstrap menu ID"))
        })
        .collect::<Result<_>>()?;
    authorize(
        pool,
        business,
        &[business_root, branch, descendant, sibling],
        app,
        &all_menus,
    )
    .await?;
    let business_role = role(
        pool,
        business,
        branch,
        business_user,
        &[
            uuid(menus, "root")?,
            uuid(menus, "ticket:create")?,
            uuid(menus, "ticket:update")?,
        ],
    )
    .await?;
    let mut identities = Vec::new();
    let mut identity_users = Vec::new();
    for (name, selected) in [
        ("reader", vec![management[8]]),
        ("writer", vec![management[9]]),
        ("plain", vec![]),
    ] {
        let (id, _) = user(pool, &format!("phase25-{name}"), false, root).await?;
        let mut allowed = management[..8].to_vec();
        allowed.extend(selected);
        role(pool, platform_tenant, root, id, &allowed).await?;
        identity_users.push(id);
        identities.push(session(pool, id, platform_tenant, root).await?);
    }
    sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)")
        .bind(actor)
        .bind(child)
        .execute(pool)
        .await?;
    let child_admin = session(pool, actor, platform_tenant, child).await?;
    let policy = json!({"applications":[{"applicationId":app,"capabilities":[
        {"resource":"ticket","action":"create","scopes":["SELF","ORG","ORG_SUBTREE"]},
        {"resource":"ticket","action":"update","scopes":["ORG"]}],
        "memberLookups":[{"resource":"ticket","action":"create"},{"resource":"ticket","action":"update"}]}],
        "managementScopes":[{"tenantId":business,"organizationId":branch,"includeDescendants":true,"read":true,"write":true}]});
    let server = platform(pool, artifact_origin, &policy, log).await?;
    let live_cli = Cli {
        origin: &server.origin,
        ..*cli
    };
    let target = json!({"tenantId":business,"organizationId":branch,"userId":business_user,"membershipId":membership,"applicationId":app});
    let path = grant_path(&target)?;
    let manifest = json!({"schemaVersion":"1.0","applicationCode":"work-orders","applicationId":app,"menuIds":menus,
        "targets":[{"tenantId":business,"organizationId":branch,"roleId":business_role,"userId":business_user,"membershipId":membership,
            "permissions":["ticket:create","ticket:update"],"dataGrants":[{"resource":"ticket","action":"create","scope":"ORG_SUBTREE"}]}]});
    let manifest_file = cli.files.json("grants", &manifest)?;
    let result = async {
        let before = snapshot(pool).await?;
        let requests = log.lock().expect("owned recorder").len();
        for command in ["grants-plan", "grants-apply"] {
            let planned = live_cli.call(&[command, "--manifest", &manifest_file], None).await?;
            ensure!(planned["writes"] == false && planned["offline"] == true, "grant plan omitted explicit offline/no-write result");
        }
        ensure!(snapshot(pool).await? == before && log.lock().expect("owned recorder").len() == requests, "grant plan/no-apply made HTTP calls or changed state");
        let initial = http(&server.origin, cli.bearer, Method::GET, &path, Value::Null).await?;
        ensure!(initial.status == StatusCode::OK && initial.body["grants"] == json!([]), "fixture data grants were not initially empty");
        let edit = |version: &Value, action: &str, scope: &str| json!({"target":target,"expectedVersion":version,
            "upsert":[{"resource":"ticket","action":action,"scope":scope}],"remove":[]});
        // Exercise the actual API's distinct permissions before involving CLI preconditions.
        let read = http(&server.origin, &identities[0], Method::GET, &path, Value::Null).await?;
        ensure!(read.status == StatusCode::OK, "independent read permission could not read real target");
        for token in [&identities[0], &identities[2], &child_admin] {
            for (method, endpoint) in [(Method::POST, "/api/data-grants/preview"), (Method::PATCH, "/api/data-grants")] {
                let denied = http(&server.origin, token, method, endpoint, edit(&initial.body["version"], "create", "ORG_SUBTREE")).await?;
                ensure!(denied.status == StatusCode::FORBIDDEN, "reader/unprivileged/child administrator bypassed real grant API");
            }
        }
        ensure!(http(&server.origin, &identities[1], Method::GET, &path, Value::Null).await?.status == StatusCode::FORBIDDEN, "write permission implied read");
        ensure!(http(&server.origin, &identities[1], Method::POST, "/api/data-grants/preview", edit(&initial.body["version"], "create", "ORG_SUBTREE")).await?.status == StatusCode::FORBIDDEN, "write-only permission bypassed preview's independent read requirement");
        for token in [&identities[0], &identities[1], &identities[2], &child_admin] {
            let denied_cli = Cli { bearer: token, ..live_cli };
            let denied = denied_cli.call(&["grants-apply", "--manifest", &manifest_file, "--apply"], Some("HTTP_ERROR")).await?;
            ensure!(denied["status"] == 403, "denied grant CLI did not retain the actual platform 403");
        }
        ensure!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM membership_data_grants").fetch_one(pool).await? == 0, "denied CLI granted data scope");
        let foreign_menu: Uuid = sqlx::query_scalar("INSERT INTO menus(application_id,title,type,permission) VALUES($1,'Foreign ticket read','BUTTON','ticket:read') RETURNING id").bind(other).fetch_one(pool).await?;
        let mut bad = manifest.clone();
        bad["menuIds"]["ticket:read"] = json!(foreign_menu);
        let cross_app = cli.files.json("grants-cross-app", &bad)?;
        live_cli.call(&["grants-apply", "--manifest", &cross_app, "--apply"], Some("GRANT_PRECONDITION_FAILED")).await?;
        let mut wrong_member = manifest.clone();
        wrong_member["targets"][0]["userId"] = json!(descendant_user);
        let wrong_member = cli.files.json("grants-wrong-membership", &wrong_member)?;
        live_cli.call(&["grants-apply", "--manifest", &wrong_member, "--apply"], Some("GRANT_PRECONDITION_FAILED")).await?;
        let business_session = session(pool, business_user, business, branch).await?;
        let issued = http(&server.origin, &business_session, Method::POST, "/api/oauth/session-token", json!({"appCode":"work-orders"})).await?;
        ensure!(issued.status == StatusCode::CREATED, "could not issue real non-super application token");
        let token = issued.body["accessToken"].as_str().ok_or_else(|| anyhow!("real app token missing"))?;
        let context_path = "/api/apps/work-orders/context";
        let member_path = |organization: Uuid| format!("/api/apps/work-orders/members?resource=ticket&action=create&organizationId={organization}");
        let before_context = http(&server.origin, token, Method::GET, context_path, Value::Null).await?;
        ensure!(before_context.status == StatusCode::OK && before_context.body["dataGrants"] == json!([]), "business app had an implicit grant");
        ensure!(http(&server.origin, token, Method::GET, &member_path(branch), Value::Null).await?.status == StatusCode::FORBIDDEN, "business member API accepted absent data grant");
        let applied = live_cli.call(&["grants-apply", "--manifest", &manifest_file, "--apply"], None).await?;
        ensure!(applied["mode"] == "APPLIED", "grant apply was not explicit");
        functional_authorization_round_trip(pool, &live_cli, app, menus, log).await?;
        let granted = http(&server.origin, cli.bearer, Method::GET, &path, Value::Null).await?;
        ensure!(granted.status == StatusCode::OK && granted.body["target"] == target && granted.body["grants"].as_array().is_some_and(|v| v.len() == 1 && v[0]["scope"] == "ORG_SUBTREE"), "CLI did not apply exact direct-membership scope");
        let grant_id = granted.body["grants"][0]["id"].clone();
        let identity: (Uuid, Uuid, Uuid, Uuid, String) = sqlx::query_as("SELECT g.membership_id,m.user_id,m.organization_id,g.application_id,g.scope_type FROM membership_data_grants g JOIN memberships m ON m.id=g.membership_id WHERE g.membership_id=$1")
            .bind(membership).fetch_one(pool).await?;
        ensure!(identity == (membership, business_user, branch, app, "ORG_SUBTREE".into()), "actual SQL grant identity diverged from exact target");
        let same = live_cli.call(&["grants-apply", "--manifest", &manifest_file, "--apply"], None).await?;
        ensure!(same["mode"] == "APPLIED" && sqlx::query_scalar::<_, i64>("SELECT count(*) FROM audit_logs WHERE action='DATA_GRANT_CHANGE'").fetch_one(pool).await? == 1, "same grant apply rewrote data grants or audit");
        let mut conflicting_scope = manifest.clone();
        conflicting_scope["targets"][0]["dataGrants"][0]["scope"] = json!("SELF");
        let scope_file = cli.files.json("grants-scope-conflict", &conflicting_scope)?;
        live_cli.call(&["grants-apply", "--manifest", &scope_file, "--apply"], Some("GRANT_SCOPE_CONFLICT")).await?;
        let unchanged = http(&server.origin, cli.bearer, Method::GET, &path, Value::Null).await?;
        ensure!(unchanged.body["version"] == granted.body["version"] && unchanged.body["grants"][0]["id"] == grant_id, "add-only grant scope conflict changed state");
        let context = http(&server.origin, token, Method::GET, context_path, Value::Null).await?;
        ensure!(context.status == StatusCode::OK && context.body["contextVersion"] != before_context.body["contextVersion"]
            && context.body["dataGrants"].as_array().is_some_and(|v| v.iter().any(|g| g["scope"] == "ORG_SUBTREE" && g["subjectUserId"] == business_user.to_string() && g["tenantId"] == business.to_string())), "same app token retained stale or wrong-subject context");
        ensure!(http(&server.origin, token, Method::GET, &member_path(branch), Value::Null).await?.status == StatusCode::OK
            && http(&server.origin, token, Method::GET, &member_path(descendant), Value::Null).await?.status == StatusCode::OK
            && http(&server.origin, token, Method::GET, &member_path(sibling), Value::Null).await?.status == StatusCode::NOT_FOUND, "actual business member requests escaped or lost organization subtree scope");
        ensure!(http(&server.origin, &business_session, Method::GET, &path, Value::Null).await?.status == StatusCode::FORBIDDEN, "business grant implied management permission");
        let writer = http(&server.origin, &identities[1], Method::PATCH, "/api/data-grants", edit(&granted.body["version"], "update", "ORG")).await?;
        ensure!(writer.status == StatusCode::OK && writer.body["grants"].as_array().is_some_and(|v| v.len() == 2), "independent write-only permission failed formal PATCH or replaced omitted action");
        let deleted = http(&server.origin, cli.bearer, Method::PATCH, "/api/data-grants",
            json!({"target":target,"expectedVersion":writer.body["version"],"upsert":[],"remove":[{"resource":"ticket","action":"create"}]})).await?;
        ensure!(deleted.status == StatusCode::OK, "formal grant removal failed");
        let revoked = http(&server.origin, token, Method::GET, context_path, Value::Null).await?;
        ensure!(revoked.body["contextVersion"] != context.body["contextVersion"]
            && http(&server.origin, token, Method::GET, &member_path(branch), Value::Null).await?.status == StatusCode::FORBIDDEN, "next business request retained revoked grant");
        let records = log.lock().expect("owned recorder").clone();
        let writes: Vec<_> = records.iter().filter(|r| r.method == Method::PATCH && r.path == "/api/data-grants" && r.status == StatusCode::OK).collect();
        ensure!(writes.len() == 3 && records.iter().any(|r| r.method == Method::POST && r.path == "/api/data-grants/preview" && r.status == StatusCode::OK), "CLI grant flow omitted formal preview/PATCH or rewrote no-op");
        for request in writes {
            let request_id = request.request_id.ok_or_else(|| anyhow!("grant HTTP mutation lost request ID"))?;
            let rows: Vec<GrantAudit> = sqlx::query_as("SELECT user_id,tenant_id,organization_id,application_id,detail FROM audit_logs WHERE action='DATA_GRANT_CHANGE' AND request_id=$1")
                .bind(request_id).fetch_all(pool).await?;
            let expected_actor = if request_id == writer.request_id { identity_users[1] } else { actor };
            ensure!(rows.len() == 1 && rows[0].user_id == expected_actor && rows[0].tenant_id == Some(business) && rows[0].organization_id == Some(branch) && rows[0].application_id == Some(app), "real grant audit missing exact actor/target/request binding");
            let detail: Value = serde_json::from_str(&rows[0].detail)?;
            ensure!(detail["before"]["target"] == target && detail["after"]["target"] == target, "grant audit lost direct membership snapshot");
            for secret in [SECRET, CLIENT_SECRET, cli.bearer, &business_session, token] {
                ensure!(!rows[0].detail.contains(secret), "grant audit exposed fixture secret");
            }
        }
        let writer_audit: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='DATA_GRANT_CHANGE' AND request_id=$1").bind(writer.request_id).fetch_one(pool).await?;
        let removal_audit: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='DATA_GRANT_CHANGE' AND request_id=$1 AND user_id=$2").bind(deleted.request_id).bind(actor).fetch_one(pool).await?;
        ensure!(writer_audit == 1 && removal_audit == 1, "read/write-specific HTTP audit did not persist");
        let membership_after: Uuid = sqlx::query_scalar("SELECT id FROM memberships WHERE user_id=$1 AND organization_id=$2").bind(business_user).bind(branch).fetch_one(pool).await?;
        ensure!(membership_after == membership, "grant apply replaced actual direct membership UUID");
        Ok(())
    }.await;
    let shutdown = server.close().await;
    result?;
    shutdown
}

async fn functional_authorization_round_trip(
    pool: &PgPool,
    cli: &Cli<'_>,
    app: Uuid,
    menus: &Value,
    log: &Requests,
) -> Result<()> {
    // A separate unprovisioned target exercises all four functional PUT paths.
    // Main negative grant fixtures stay fully provisioned so rejected calls cannot
    // be mistaken for data-grant writes after unrelated partial provisioning.
    let tenant = tenant(pool, "phase25-new-customer", false).await?;
    let organization = org(pool, tenant, None, "Phase25 new customer root").await?;
    let (user, membership) = user(pool, "phase25-new-customer-user", false, organization).await?;
    let role: Uuid = sqlx::query_scalar("INSERT INTO roles(tenant_id,organization_id,name,code) VALUES($1,$2,'Phase25 unprovisioned role','phase25-unprovisioned') RETURNING id")
        .bind(tenant).bind(organization).fetch_one(pool).await?;
    let manifest = json!({"schemaVersion":"1.0","applicationCode":"work-orders","applicationId":app,"menuIds":menus,
        "targets":[{"tenantId":tenant,"organizationId":organization,"roleId":role,"userId":user,"membershipId":membership,
            "permissions":["ticket:create"],"dataGrants":[]}]});
    let path = cli.files.json("functional-grants", &manifest)?;
    let applied = cli
        .call(&["grants-apply", "--manifest", &path, "--apply"], None)
        .await?;
    let completed = applied["completed"]
        .as_array()
        .ok_or_else(|| anyhow!("functional grant completed operations absent"))?;
    for kind in [
        "TENANT_AUTHORIZATION",
        "ORGANIZATION_AUTHORIZATION",
        "ROLE_MENUS",
        "ROLE_USERS",
    ] {
        ensure!(
            completed.iter().filter(|op| op["kind"] == kind).count() == 1,
            "functional grant did not execute {kind} exactly once"
        );
    }
    ensure!(
        completed.len() == 4,
        "functional-only provisioning executed an unexpected operation"
    );
    let expected_paths = [
        format!("/api/tenants/{tenant}/authorizations"),
        format!("/api/organizations/{organization}/authorizations"),
        format!("/api/roles/{role}/menus"),
        format!("/api/roles/{role}/users"),
    ];
    for path in expected_paths {
        ensure!(
            log.lock()
                .expect("owned recorder")
                .iter()
                .filter(|request| request.method == Method::PUT
                    && request.path == path
                    && request.status == StatusCode::OK)
                .count()
                == 1,
            "functional grant did not use the actual successful management PUT"
        );
    }
    let state: (i64, i64, i64, i64, i64, Uuid) = sqlx::query_as("SELECT (SELECT count(*) FROM tenant_applications WHERE tenant_id=$1 AND application_id=$2),(SELECT count(*) FROM organization_applications WHERE organization_id=$3 AND application_id=$2),(SELECT count(*) FROM tenant_menus WHERE tenant_id=$1),(SELECT count(*) FROM organization_menus WHERE organization_id=$3),(SELECT count(*) FROM role_menus WHERE role_id=$4),(SELECT id FROM memberships WHERE user_id=$5 AND organization_id=$3)")
        .bind(tenant).bind(app).bind(organization).bind(role).bind(user).fetch_one(pool).await?;
    ensure!(
        state == (1, 1, 2, 2, 2, membership),
        "functional management API SQL results or direct membership diverged"
    );
    ensure!(sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM user_roles WHERE user_id=$1 AND role_id=$2 AND organization_id=$3)").bind(user).bind(role).bind(organization).fetch_one(pool).await?, "functional grant did not assign the actual role user");
    let before = snapshot(pool).await?;
    let again = cli
        .call(&["grants-apply", "--manifest", &path, "--apply"], None)
        .await?;
    ensure!(
        again["completedCount"] == 0 && snapshot(pool).await? == before,
        "repeated functional grant rewrote current authorization sets"
    );
    Ok(())
}
