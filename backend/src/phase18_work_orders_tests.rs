//! Genuine platform authentication and controlled grants -> independent Rust
//! business service -> two owned disposable PostgreSQL databases.
//! Loopback HTTP is an explicit development fixture, not production TLS proof.
use std::{
    collections::HashMap, net::SocketAddr, process::Stdio, str::FromStr, sync::Arc, time::Duration,
};

use anyhow::{anyhow, ensure, Result};
use axum::{
    body::{to_bytes, Body},
    http::Request,
    Router,
};
use reqwest::{Client, Method};
use serde_json::{json, Value};
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool,
};
use tokio::net::TcpListener;
use tower::ServiceExt;
use uuid::Uuid;

use crate::{config::Config, state::AppState};

const SECRET: &str = "owned-phase18-session-signing-only";
const PASSWORD: &str = "Phase18-Owned-Password-1!";
const USERNAME: &str = "phase18-write-user";
const CODE: &str = "work-orders";
const TICKETS: &str = "/work-orders-api/tickets";
const ACTIONS: [&str; 5] = ["create", "update", "delete", "read", "export"];
const CHECKS: [&str; 12] = [
    "real-login-context-confidential-introspection",
    "formal-five-action-grants-preview",
    "create-defaults-normalized-replay",
    "same-key-concurrent-single-row",
    "changed-body-idempotency-conflict",
    "grant-revocation-blocks-replay",
    "cross-organization-idempotency-domain",
    "optimistic-version-single-winner",
    "immutable-fields-and-assignee-fail-closed",
    "state-transition-policy",
    "soft-delete-all-read-paths",
    "create-receipt-transactional-storage",
];

fn config(policy: &Value) -> Result<Config> {
    Config::from_values(&HashMap::from([
        ("APPLICATION_ENVIRONMENT".into(), "development".into()),
        (
            "DATABASE_URL".into(),
            "postgres://unused/phase18-owned".into(),
        ),
        ("JWT_SECRET".into(), SECRET.into()),
        ("DATA_GRANT_POLICY".into(), policy.to_string()),
    ]))
}

fn router(pool: &PgPool, policy: &Value) -> Result<Router> {
    Ok(crate::routes::api_router()
        .nest("/oauth", crate::routes::oauth2::oauth_router())
        .with_state(AppState::new(pool.clone(), Arc::new(config(policy)?))))
}
struct OwnedDatabase {
    pool: PgPool,
    admin: PgPool,
    name: String,
    url: String,
    kind: &'static str,
}

impl OwnedDatabase {
    async fn create(kind: &'static str) -> Result<Self> {
        ensure!(
            matches!(kind, "BUSINESS" | "PLATFORM"),
            "invalid owned database kind"
        );
        // TEST_DATABASE_URL is required before the shared platform fixture is
        // called, so its historical dotenv fallback is never reachable here.
        let source = std::env::var("TEST_DATABASE_URL")
            .map_err(|_| anyhow!("explicit disposable TEST_DATABASE_URL required"))?;
        let options = PgConnectOptions::from_str(&source)
            .map_err(|_| anyhow!("invalid disposable PostgreSQL configuration"))?;
        let mut url = reqwest::Url::parse(&source)
            .map_err(|_| anyhow!("invalid disposable PostgreSQL configuration"))?;
        let owner = std::env::var("CI_TEST_POSTGRES_OWNER")
            .map_err(|_| anyhow!("managed CI_TEST_POSTGRES_OWNER required"))?;
        ensure!(
            owner.starts_with("ci-")
                && (11..=83).contains(&owner.len())
                && owner
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "invalid managed PostgreSQL owner"
        );
        ensure!(
            url.host_str() == Some(format!("{owner}-postgres").as_str())
                && url.path() == "/postgres"
                && url.username() == "ci"
                && url.scheme() == "postgresql"
                && url.port() == Some(5432)
                && url.password().is_some_and(|p| !p.is_empty())
                && url.query().is_none()
                && url.fragment().is_none(),
            "fixture requires its managed PostgreSQL administration endpoint"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("owned business administration connection failed"))?;
        let name = format!(
            "phase18_work_orders_{}_{}",
            kind.to_lowercase(),
            Uuid::new_v4().simple()
        );
        if sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await
            .is_err()
        {
            admin.close().await;
            return Err(anyhow!("owned business database creation failed"));
        }
        // The name becomes cleanup-eligible only after our CREATE succeeded.
        let pool = match PgPoolOptions::new()
            .max_connections(3)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(options.database(&name))
            .await
        {
            Ok(pool) => pool,
            Err(_) => {
                let removed = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
                    .execute(&admin)
                    .await
                    .is_ok();
                admin.close().await;
                ensure!(removed, "owned business connection and cleanup failed");
                return Err(anyhow!("owned business database connection failed"));
            }
        };
        url.set_path(&format!("/{name}"));
        Ok(Self {
            pool,
            admin,
            name,
            url: url.to_string(),
            kind,
        })
    }

    async fn finish(self, assertions: Result<()>) -> Result<()> {
        ensure!(
            self.name.starts_with("phase18_work_orders_")
                && self
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "invalid owned business cleanup target"
        );
        let closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let removed = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await
            .is_ok();
        let closed = closed
            || tokio::time::timeout(Duration::from_secs(5), self.pool.close())
                .await
                .is_ok();
        let admin_closed = tokio::time::timeout(Duration::from_secs(5), self.admin.close())
            .await
            .is_ok();
        println!(
            "WRITE_LIVE_{}_CLEANUP assertions_ok={} database_removed={} pool_closed={}",
            self.kind,
            assertions.is_ok(),
            removed,
            closed && admin_closed
        );
        assertions?;
        ensure!(
            removed && closed && admin_closed,
            "owned business cleanup failed"
        );
        Ok(())
    }
}

struct Reply {
    status: u16,
    bytes: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Result<Value> {
        serde_json::from_slice(&self.bytes).map_err(|_| anyhow!("live response is not JSON"))
    }

    fn require(&self, expected: u16) -> Result<()> {
        ensure!(
            self.status == expected,
            "live response status {} expected {}",
            self.status,
            expected
        );
        Ok(())
    }
}

fn text_field(value: &Value, key: &str) -> Result<String> {
    value[key]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("live response lacks required field {key}"))
}

fn id(value: &Value) -> Result<Uuid> {
    text_field(value, "id")?
        .parse()
        .map_err(|_| anyhow!("live response ID is invalid"))
}

// Bootstrap uses the real platform router, password verifier, and management
// APIs. SQL in this fixture only reads generated IDs; it never forges sessions,
// memberships, menu authorization, role bindings, OAuth credentials, or grants.
async fn api(
    app: &Router,
    path: &str,
    token: &str,
    method: Method,
    body: Option<Value>,
) -> Result<Value> {
    let mut request = Request::builder().method(method.clone()).uri(path);
    if !token.is_empty() {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let body = match body {
        Some(body) => {
            request = request.header("content-type", "application/json");
            serde_json::to_vec(&body)?
        }
        None => Vec::new(),
    };
    let response = tokio::time::timeout(
        Duration::from_secs(20),
        app.clone().oneshot(request.body(Body::from(body))?),
    )
    .await
    .map_err(|_| anyhow!("bootstrap API deadline exceeded"))??;
    ensure!(
        response.status().as_u16() == 200,
        "bootstrap {method} {path} status {}",
        response.status()
    );
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    serde_json::from_slice(&bytes).map_err(|_| anyhow!("bootstrap API response is not JSON"))
}

async fn http(
    client: &Client,
    origin: &str,
    path: &str,
    token: &str,
    method: Method,
    body: Option<Value>,
    key: Option<&str>,
) -> Result<Reply> {
    let request_id = Uuid::new_v4().to_string();
    let mut request = client
        .request(method, format!("{origin}{path}"))
        .header("x-request-id", &request_id);
    if !token.is_empty() {
        request = request.bearer_auth(token);
    }
    if let Some(key) = key {
        request = request.header("idempotency-key", key);
    }
    if let Some(body) = body {
        request = request
            .header("content-type", "application/json")
            .body(serde_json::to_vec(&body)?);
    }
    let response = request
        .send()
        .await
        .map_err(|_| anyhow!("live HTTP request failed"))?;
    if path.starts_with(TICKETS) {
        ensure!(
            response
                .headers()
                .get("x-request-id")
                .and_then(|v| v.to_str().ok())
                == Some(request_id.as_str()),
            "business response lost request ID"
        );
    }
    let status = response.status().as_u16();
    let bytes = response
        .bytes()
        .await
        .map_err(|_| anyhow!("live HTTP response read failed"))?;
    ensure!(bytes.len() <= 1024 * 1024, "live response exceeded budget");
    if path.starts_with("/work-orders-api/") {
        ensure!(
            !bytes.windows(SECRET.len()).any(|v| v == SECRET.as_bytes()),
            "business response exposed credential material"
        );
    }
    Ok(Reply {
        status,
        bytes: bytes.to_vec(),
    })
}

#[derive(Clone)]
struct Member {
    tenant: Uuid,
    org: Uuid,
    user: Uuid,
    membership: Uuid,
}

impl Member {
    fn target(&self, application: Uuid) -> Value {
        json!({"tenantId":self.tenant,"organizationId":self.org,"userId":self.user,
            "membershipId":self.membership,"applicationId":application})
    }
}

struct Fixture {
    app: Router,
    application: Uuid,
    admin_org: Uuid,
    first: Member,
    second: Member,
    client_id: String,
    client_secret: String,
}

impl Fixture {
    async fn create(pool: &PgPool) -> Result<Self> {
        let empty = json!({"applications":[],"managementScopes":[]});
        crate::seed::initialize(pool, &config(&empty)?).await?;
        let app = router(pool, &empty)?;
        let admin_org: Uuid = sqlx::query_scalar("SELECT o.id FROM organizations o JOIN tenants t ON t.id=o.tenant_id WHERE t.is_platform AND o.parent_id IS NULL")
            .fetch_one(pool).await?;
        let login = api(
            &app,
            "/api/auth/login",
            "",
            Method::POST,
            Some(json!({"username":"admin","password":"admin123"})),
        )
        .await?;
        let session = api(
            &app,
            "/api/auth/context",
            &text_field(&login, "ticket")?,
            Method::POST,
            Some(json!({"organizationId":admin_org})),
        )
        .await?;
        ensure!(
            session["platformContext"] == true,
            "bootstrap is not the actual platform root"
        );
        let admin = text_field(&session, "accessToken")?;
        let tenant = id(&api(
            &app,
            "/api/tenants",
            &admin,
            Method::POST,
            Some(json!({"name":"Owned Phase18 Writes","code":"phase18-writes"})),
        )
        .await?)?;
        let root: Uuid = sqlx::query_scalar(
            "SELECT id FROM organizations WHERE tenant_id=$1 AND parent_id IS NULL",
        )
        .bind(tenant)
        .fetch_one(pool)
        .await?;
        let second_org = id(&api(&app, "/api/organizations", &admin, Method::POST, Some(json!({"tenantId":tenant,"parentId":root,"name":"Owned Phase18 second","code":"phase18-second"}))).await?)?;
        let user = id(&api(
            &app,
            "/api/users",
            &admin,
            Method::POST,
            Some(
                json!({"username":USERNAME,"password":PASSWORD,"realName":"Owned Phase18 writer"}),
            ),
        )
        .await?)?;
        api(
            &app,
            &format!("/api/users/{user}/memberships"),
            &admin,
            Method::PUT,
            Some(json!({"items":[{"organizationId":root},{"organizationId":second_org}]})),
        )
        .await?;
        let application = id(&api(&app, "/api/applications", &admin, Method::POST, Some(json!({"name":"Owned Phase18 Work Orders","code":CODE,"entryType":"HTML","basePath":"/micro/work-orders"}))).await?)?;
        let mut menus = Vec::new();
        for action in ACTIONS {
            menus.push(id(&api(&app, "/api/menus", &admin, Method::POST, Some(json!({"applicationId":application,"title":format!("Phase18 {action}"),"type":"BUTTON","permission":format!("ticket:{action}"),"visible":false}))).await?)?);
        }
        let authz = json!({"applications":[{"applicationId":application,"authorized":true}],"menus":menus.iter().map(|m|json!({"menuId":m,"authorized":true})).collect::<Vec<_>>()});
        api(
            &app,
            &format!("/api/tenants/{tenant}/authorizations"),
            &admin,
            Method::PUT,
            Some(authz.clone()),
        )
        .await?;
        for organization in [root, second_org] {
            api(
                &app,
                &format!("/api/organizations/{organization}/authorizations"),
                &admin,
                Method::PUT,
                Some(authz.clone()),
            )
            .await?;
            let role = id(&api(&app, "/api/roles", &admin, Method::POST, Some(json!({"name":format!("Phase18 writer {organization}"),"organizationId":organization}))).await?)?;
            api(
                &app,
                &format!("/api/roles/{role}/menus"),
                &admin,
                Method::PUT,
                Some(json!({"menuIds":menus})),
            )
            .await?;
            api(
                &app,
                &format!("/api/roles/{role}/users"),
                &admin,
                Method::PUT,
                Some(json!({"userIds":[user]})),
            )
            .await?;
        }
        let oauth = api(&app, "/api/oauth/clients", &admin, Method::POST, Some(json!({"clientName":"Owned Phase18 work orders","clientType":"CONFIDENTIAL","grantTypes":["password","refresh_token"],"scopes":["profile"]}))).await?;
        api(
            &app,
            &format!("/api/oauth/clients/{}", id(&oauth)?),
            &admin,
            Method::PUT,
            Some(json!({"applicationId":application})),
        )
        .await?;
        let memberships = api(
            &app,
            &format!("/api/users/{user}/memberships"),
            &admin,
            Method::GET,
            None,
        )
        .await?;
        let member = |org: Uuid| -> Result<Member> {
            let row = memberships
                .as_array()
                .and_then(|rows| {
                    rows.iter()
                        .find(|row| row["organizationId"] == org.to_string())
                })
                .ok_or_else(|| anyhow!("formal membership missing"))?;
            Ok(Member {
                tenant,
                org,
                user,
                membership: text_field(row, "membershipId")?.parse()?,
            })
        };
        let policy = json!({"applications":[{"applicationId":application,"capabilities":ACTIONS.iter().map(|action|json!({"resource":"ticket","action":action,"scopes":["SELF","ORG","ORG_SUBTREE","TENANT"]})).collect::<Vec<_>>()}],
            "managementScopes":[{"tenantId":tenant,"organizationId":root,"includeDescendants":true,"read":true,"write":true}]});
        Ok(Self {
            app: router(pool, &policy)?,
            application,
            admin_org,
            first: member(root)?,
            second: member(second_org)?,
            client_id: text_field(&oauth, "clientId")?,
            client_secret: text_field(&oauth, "clientSecret")?,
        })
    }
}

async fn login(
    client: &Client,
    origin: &str,
    username: &str,
    password: &str,
    org: Uuid,
) -> Result<Value> {
    let proof = http(
        client,
        origin,
        "/api/auth/login",
        "",
        Method::POST,
        Some(json!({"username":username,"password":password})),
        None,
    )
    .await?;
    proof.require(200)?;
    let selected = http(
        client,
        origin,
        "/api/auth/context",
        &text_field(&proof.json()?, "ticket")?,
        Method::POST,
        Some(json!({"organizationId":org})),
        None,
    )
    .await?;
    selected.require(200)?;
    selected.json()
}

async fn issue(client: &Client, origin: &str, session: &str) -> Result<String> {
    let response = http(
        client,
        origin,
        "/api/oauth/session-token",
        session,
        Method::POST,
        Some(json!({"appCode":CODE})),
        None,
    )
    .await?;
    response.require(201)?;
    text_field(&response.json()?, "accessToken")
}

async fn grants(
    client: &Client,
    origin: &str,
    admin: &str,
    member: &Member,
    app: Uuid,
    actions: &[&str],
    remove: &[&str],
) -> Result<()> {
    let target = member.target(app);
    let query = target
        .as_object()
        .ok_or_else(|| anyhow!("grant target invalid"))?
        .iter()
        .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or_default()))
        .collect::<Vec<_>>()
        .join("&");
    let path = format!("/api/data-grants?{query}");
    let before = http(client, origin, &path, admin, Method::GET, None, None).await?;
    before.require(200)?;
    let before = before.json()?;
    let body = json!({"target":target,"expectedVersion":before["version"],
        "upsert":actions.iter().map(|action|json!({"resource":"ticket","action":action,"scope":"SELF"})).collect::<Vec<_>>(),
        "remove":remove.iter().map(|action|json!({"resource":"ticket","action":action})).collect::<Vec<_>>()});
    http(
        client,
        origin,
        "/api/data-grants/preview",
        admin,
        Method::POST,
        Some(body.clone()),
        None,
    )
    .await?
    .require(200)?;
    let after = http(client, origin, &path, admin, Method::GET, None, None).await?;
    after.require(200)?;
    ensure!(
        before == after.json()?,
        "formal grant preview mutated storage"
    );
    http(
        client,
        origin,
        "/api/data-grants",
        admin,
        Method::PATCH,
        Some(body),
        None,
    )
    .await?
    .require(200)?;
    Ok(())
}

fn pass(checks: &mut Vec<Value>, name: &str) -> Result<()> {
    ensure!(
        CHECKS.contains(&name) && !checks.iter().any(|v| v["name"] == name),
        "unknown or duplicate live check"
    );
    checks.push(json!({"name":name,"status":"PASS"}));
    println!("WRITE_LIVE_{name}=PASS");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires managed explicit disposable PostgreSQL and built independent WORK_ORDERS_BINARY"]
async fn real_platform_writes_and_idempotency() -> Result<()> {
    // Validate the explicit managed endpoint before creating either database.
    // No dotenv fallback or developer database can enter this harness.
    let business = OwnedDatabase::create("BUSINESS").await?;
    let platform = match OwnedDatabase::create("PLATFORM").await {
        Ok(database) => database,
        Err(error) => return business.finish(Err(error)).await,
    };
    let mut checks = Vec::new();
    let result = async {
        tokio::time::timeout(
            Duration::from_secs(45),
            sqlx::migrate!("./migrations").run(&platform.pool),
        )
        .await
        .map_err(|_| anyhow!("owned platform migrations timed out"))?
        .map_err(|_| anyhow!("owned platform migrations failed"))?;
        joint(&platform.pool, &business, &mut checks).await
    }
    .await;
    let result = business.finish(result).await;
    let result = platform.finish(result).await;
    result?;
    ensure!(
        checks.len() == CHECKS.len(),
        "write live verification missed mandatory checks"
    );
    println!(
        "WORK_ORDERS_WRITE_LIVE_RESULT {}",
        json!({"schemaVersion":1,"transport":"explicit-development-loopback-http","passed":checks.len(),"failed":0,"skipped":0,"checks":checks})
    );
    Ok(())
}

async fn joint(pool: &PgPool, business: &OwnedDatabase, checks: &mut Vec<Value>) -> Result<()> {
    let fixture = Fixture::create(pool).await?;
    let platform_listener = TcpListener::bind("127.0.0.1:0").await?;
    let platform_origin = format!("http://{}", platform_listener.local_addr()?);
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    drop(listener);
    let origin = format!("http://127.0.0.1:{port}");
    let binary = std::env::var_os("WORK_ORDERS_BINARY")
        .ok_or_else(|| anyhow!("built independent WORK_ORDERS_BINARY required"))?;
    let mut command = tokio::process::Command::new(binary);
    command
        .env_clear()
        .env("NODE_ENV", "test")
        .env("WORK_ORDERS_DATABASE_URL", &business.url)
        .env("WORK_ORDERS_HOST", "127.0.0.1")
        .env("WORK_ORDERS_PORT", port.to_string())
        .env("PLATFORM_API_ORIGIN", &platform_origin)
        .env("PLATFORM_CLIENT_ID", &fixture.client_id)
        .env("PLATFORM_CLIENT_SECRET", &fixture.client_secret)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
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
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let mut stop_tx = Some(stop_tx);
    let app = fixture.app.clone();
    let mut server = tokio::spawn(async move {
        axum::serve(
            platform_listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = stop_rx.await;
        })
        .await
    });
    let result = async {
        let mut child = command
            .spawn()
            .map_err(|_| anyhow!("independent Rust work-orders process could not start"))?;
        let result = async {
            let client = Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .build()?;
            tokio::time::timeout(Duration::from_secs(35), async {
                loop {
                    ensure!(
                        child.try_wait()?.is_none(),
                        "independent work-orders process exited before readiness"
                    );
                    if client
                        .get(format!("{origin}/work-orders-api/health/ready"))
                        .send()
                        .await
                        .is_ok_and(|r| r.status().is_success())
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Ok::<_, anyhow::Error>(())
            })
            .await
            .map_err(|_| anyhow!("independent work-orders readiness deadline exceeded"))??;
            assertions(
                &client,
                &platform_origin,
                &origin,
                &fixture,
                &business.pool,
                checks,
            )
            .await
        }
        .await;
        let process_stopped = match child.try_wait() {
            Ok(Some(_)) => true,
            _ => {
                let _ = child.start_kill();
                tokio::time::timeout(Duration::from_secs(5), child.wait())
                    .await
                    .is_ok_and(|r| r.is_ok())
            }
        };
        println!("WRITE_LIVE_PROCESS_CLEANUP stopped={process_stopped}");
        result?;
        ensure!(process_stopped, "owned business process cleanup failed");
        Ok(())
    }
    .await;
    if let Some(stop) = stop_tx.take() {
        let _ = stop.send(());
    }
    let platform_stopped = match tokio::time::timeout(Duration::from_secs(5), &mut server).await {
        Ok(Ok(Ok(()))) => true,
        Ok(_) => false,
        Err(_) => {
            server.abort();
            let _ = server.await;
            false
        }
    };
    result?;
    ensure!(platform_stopped, "owned platform HTTP shutdown failed");
    Ok(())
}

async fn assertions(
    client: &Client,
    platform: &str,
    business: &str,
    f: &Fixture,
    pool: &PgPool,
    checks: &mut Vec<Value>,
) -> Result<()> {
    let root = login(client, platform, "admin", "admin123", f.admin_org).await?;
    ensure!(
        root["platformContext"] == true,
        "real root login lost platform context"
    );
    let admin = text_field(&root, "accessToken")?;
    let first = login(client, platform, USERNAME, PASSWORD, f.first.org).await?;
    ensure!(
        first["platformContext"] == false,
        "ordinary login became platform root"
    );
    let session = text_field(&first, "accessToken")?;
    let token = issue(client, platform, &session).await?;
    let intro = client
        .post(format!("{platform}/oauth/introspect"))
        .basic_auth(&f.client_id, Some(&f.client_secret))
        .form(&[("token", token.as_str())])
        .send()
        .await
        .map_err(|_| anyhow!("real confidential introspection failed"))?;
    ensure!(
        intro.status().as_u16() == 200,
        "real introspection status mismatch"
    );
    let intro: Value = serde_json::from_slice(
        &intro
            .bytes()
            .await
            .map_err(|_| anyhow!("introspection body unavailable"))?,
    )?;
    ensure!(
        intro["active"] == true && intro["aud"] == CODE && intro["sub"] == f.first.user.to_string(),
        "real introspection identity mismatch"
    );
    let rejected = client
        .post(format!("{platform}/oauth/introspect"))
        .basic_auth(&f.client_id, Some("incorrect-owned-fixture-secret"))
        .form(&[("token", token.as_str())])
        .send()
        .await
        .map_err(|_| anyhow!("confidential negative control failed"))?;
    ensure!(
        rejected.status().as_u16() == 401,
        "introspection accepted wrong confidential secret"
    );
    pass(checks, CHECKS[0])?;

    http(
        client,
        business,
        TICKETS,
        &token,
        Method::POST,
        Some(json!({"title":"initially denied"})),
        Some("phase18-denied"),
    )
    .await?
    .require(403)?;
    grants(
        client,
        platform,
        &admin,
        &f.first,
        f.application,
        &ACTIONS,
        &[],
    )
    .await?;
    grants(
        client,
        platform,
        &admin,
        &f.second,
        f.application,
        &ACTIONS,
        &[],
    )
    .await?;
    let context = http(
        client,
        platform,
        "/api/apps/work-orders/context",
        &token,
        Method::GET,
        None,
        None,
    )
    .await?;
    context.require(200)?;
    let context = context.json()?;
    for action in ACTIONS {
        ensure!(
            context["permissions"].as_array().is_some_and(|permissions| permissions.contains(&json!(format!("ticket:{action}")))),
            "formal action permission absent"
        );
        ensure!(
            context["dataGrants"]
                .as_array()
                .is_some_and(
                    |grants| grants.iter().any(|grant| grant["resource"] == "ticket"
                        && grant["action"] == action
                        && grant["scope"] == "SELF")
                ),
            "formal data grant absent"
        );
    }
    pass(checks, CHECKS[1])?;

    let key = "phase18-normalized";
    let canonical = json!({"title":"Normalized write","description":"","priority":"NORMAL","assigneeUserId":null});
    let created = http(
        client,
        business,
        TICKETS,
        &token,
        Method::POST,
        Some(json!({"title":"  Normalized write  "})),
        Some(key),
    )
    .await?;
    created.require(201)?;
    let created = created.json()?;
    let ticket = id(&created)?;
    ensure!(
        created["title"] == "Normalized write"
            && created["description"] == ""
            && created["priority"] == "NORMAL"
            && created["assigneeUserId"].is_null()
            && created["version"] == 1,
        "legacy create defaults changed"
    );
    ensure!(
        created["tenantId"] == f.first.tenant.to_string()
            && created["organizationId"] == f.first.org.to_string()
            && created["ownerUserId"] == f.first.user.to_string(),
        "business identity was not server-owned"
    );
    let replay = http(
        client,
        business,
        TICKETS,
        &token,
        Method::POST,
        Some(canonical.clone()),
        Some(key),
    )
    .await?;
    replay.require(201)?;
    ensure!(
        replay.json()? == created,
        "canonical equivalent request did not replay original receipt"
    );
    // A committed response is deliberately abandoned without reading its body.
    // Retrying the original key must recover the same record from durable state.
    let lost = client
        .post(format!("{business}{TICKETS}"))
        .bearer_auth(&token)
        .header("idempotency-key", "phase18-lost-response")
        .header("content-type", "application/json")
        .body(serde_json::to_vec(
            &json!({"title":"Unread committed response"}),
        )?)
        .send()
        .await
        .map_err(|_| anyhow!("commit-response fixture failed"))?;
    ensure!(
        lost.status().as_u16() == 201,
        "abandoned response was not committed"
    );
    drop(lost);
    let recovered = http(
        client,
        business,
        TICKETS,
        &token,
        Method::POST,
        Some(json!({"title":"Unread committed response"})),
        Some("phase18-lost-response"),
    )
    .await?;
    recovered.require(201)?;
    let recovered = id(&recovered.json()?)?;
    let durable: Uuid = sqlx::query_scalar("SELECT ticket_id FROM work_order_create_idempotency WHERE idempotency_key='phase18-lost-response'").fetch_one(pool).await?;
    ensure!(
        recovered == durable,
        "original key did not recover unread committed response"
    );
    pass(checks, CHECKS[2])?;

    let mut jobs = Vec::new();
    for _ in 0..8 {
        let client = client.clone();
        let business = business.to_owned();
        let token = token.clone();
        jobs.push(tokio::spawn(async move {
            http(
                &client,
                &business,
                TICKETS,
                &token,
                Method::POST,
                Some(json!({"title":"Concurrent same-key write"})),
                Some("phase18-concurrent"),
            )
            .await
        }));
    }
    let mut winner = None;
    for job in jobs {
        let reply = job.await??;
        reply.require(201)?;
        let returned = id(&reply.json()?)?;
        ensure!(
            winner.is_none_or(|previous| previous == returned),
            "same-key concurrent requests produced multiple IDs"
        );
        winner = Some(returned);
    }
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM work_orders WHERE title='Concurrent same-key write'",
    )
    .fetch_one(pool)
    .await?;
    ensure!(
        rows == 1,
        "same-key concurrency created multiple durable records"
    );
    pass(checks, CHECKS[3])?;
    let changed = http(
        client,
        business,
        TICKETS,
        &token,
        Method::POST,
        Some(json!({"title":"Changed body"})),
        Some(key),
    )
    .await?;
    changed.require(409)?;
    ensure!(
        changed.json()?["error"] == "IDEMPOTENCY_CONFLICT"
            || changed.json()?["code"] == "IDEMPOTENCY_CONFLICT",
        "changed-body conflict lost error classification"
    );
    pass(checks, CHECKS[4])?;

    grants(
        client,
        platform,
        &admin,
        &f.first,
        f.application,
        &[],
        &["create"],
    )
    .await?;
    let denied = http(
        client,
        business,
        TICKETS,
        &token,
        Method::POST,
        Some(canonical.clone()),
        Some(key),
    )
    .await?;
    denied.require(403)?;
    ensure!(
        !String::from_utf8_lossy(&denied.bytes).contains(&ticket.to_string()),
        "revoked retry leaked old receipt"
    );
    grants(
        client,
        platform,
        &admin,
        &f.first,
        f.application,
        &["create"],
        &[],
    )
    .await?;
    pass(checks, CHECKS[5])?;

    let second = login(client, platform, USERNAME, PASSWORD, f.second.org).await?;
    let other_token = issue(client, platform, &text_field(&second, "accessToken")?).await?;
    let other = http(
        client,
        business,
        TICKETS,
        &other_token,
        Method::POST,
        Some(canonical),
        Some(key),
    )
    .await?;
    other.require(201)?;
    let other = other.json()?;
    ensure!(
        id(&other)? != ticket && other["organizationId"] == f.second.org.to_string(),
        "idempotency key crossed organization domain"
    );
    http(
        client,
        business,
        &format!("{TICKETS}/{ticket}"),
        &other_token,
        Method::GET,
        None,
        None,
    )
    .await?
    .require(404)?;
    let receipts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM work_order_create_idempotency WHERE idempotency_key=$1",
    )
    .bind(key)
    .fetch_one(pool)
    .await?;
    ensure!(
        receipts == 2,
        "distinct organization receipts were not isolated"
    );
    pass(checks, CHECKS[6])?;

    let detail = format!("{TICKETS}/{ticket}");
    let (left, right) = tokio::join!(
        http(
            client,
            business,
            &detail,
            &token,
            Method::PATCH,
            Some(json!({"version":1,"title":"Version winner A"})),
            None
        ),
        http(
            client,
            business,
            &detail,
            &token,
            Method::PATCH,
            Some(json!({"version":1,"title":"Version winner B"})),
            None
        )
    );
    let left = left?;
    let right = right?;
    ensure!(
        [left.status, right.status]
            .iter()
            .filter(|&&s| s == 200)
            .count()
            == 1
            && [left.status, right.status]
                .iter()
                .filter(|&&s| s == 409)
                .count()
                == 1,
        "optimistic race did not have exactly one winner"
    );
    let winner = if left.status == 200 {
        left.json()?
    } else {
        right.json()?
    };
    ensure!(
        winner["version"] == 2,
        "optimistic race advanced version incorrectly"
    );
    pass(checks, CHECKS[7])?;

    for field in [
        "tenantId",
        "organizationId",
        "ownerUserId",
        "number",
        "organizationName",
        "ownerDisplayName",
        "createdAt",
        "updatedAt",
    ] {
        let mut body = json!({"version":2});
        body[field] = json!("caller-controlled");
        http(
            client,
            business,
            &detail,
            &token,
            Method::PATCH,
            Some(body),
            None,
        )
        .await?
        .require(400)?;
    }
    http(
        client,
        business,
        TICKETS,
        &token,
        Method::POST,
        Some(json!({"title":"No membership stub","assigneeUserId":"not-a-uuid"})),
        Some("phase18-assignee"),
    )
    .await?
    .require(400)?;
    http(
        client,
        business,
        &detail,
        &token,
        Method::PATCH,
        Some(json!({"version":2,"assigneeUserId":"not-a-uuid"})),
        None,
    )
    .await?
    .require(400)?;
    http(
        client,
        business,
        TICKETS,
        &token,
        Method::POST,
        Some(json!({"title":"Immutable create","ownerUserId":f.first.user})),
        Some("phase18-immutable"),
    )
    .await?
    .require(400)?;
    pass(checks, CHECKS[8])?;

    http(
        client,
        business,
        &detail,
        &token,
        Method::PATCH,
        Some(json!({"version":2,"status":"RESOLVED"})),
        None,
    )
    .await?
    .require(400)?;
    for (version, status) in [(2, "IN_PROGRESS"), (3, "RESOLVED")] {
        let changed = http(
            client,
            business,
            &detail,
            &token,
            Method::PATCH,
            Some(json!({"version":version,"status":status})),
            None,
        )
        .await?;
        changed.require(200)?;
        ensure!(
            changed.json()?["status"] == status && changed.json()?["version"] == version + 1,
            "state transition version mismatch"
        );
    }
    http(
        client,
        business,
        &detail,
        &token,
        Method::PATCH,
        Some(json!({"version":4,"status":"OPEN"})),
        None,
    )
    .await?
    .require(400)?;
    http(
        client,
        business,
        &detail,
        &token,
        Method::PATCH,
        Some(json!({"version":4,"status":"IN_PROGRESS"})),
        None,
    )
    .await?
    .require(200)?;
    pass(checks, CHECKS[9])?;

    http(
        client,
        business,
        &detail,
        &token,
        Method::DELETE,
        Some(json!({"version":5})),
        None,
    )
    .await?
    .require(200)?;
    http(client, business, &detail, &token, Method::GET, None, None)
        .await?
        .require(404)?;
    let list = http(client, business, TICKETS, &token, Method::GET, None, None).await?;
    list.require(200)?;
    ensure!(
        !String::from_utf8_lossy(&list.bytes).contains(&ticket.to_string()),
        "soft-deleted record remained in list"
    );
    let csv = http(
        client,
        business,
        &format!("{TICKETS}/export.csv"),
        &token,
        Method::GET,
        None,
        None,
    )
    .await?;
    csv.require(200)?;
    ensure!(
        !String::from_utf8_lossy(&csv.bytes).contains(&text_field(&created, "number")?),
        "soft-deleted record remained in export"
    );
    let deleted: bool =
        sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM work_orders WHERE id=$1")
            .bind(ticket)
            .fetch_one(pool)
            .await?;
    ensure!(deleted, "delete physically removed the record");
    pass(checks, CHECKS[10])?;

    // Business-only fault injection makes receipt persistence fail after the
    // ticket insert. Both records must roll back and the original key be reusable.
    sqlx::query("CREATE FUNCTION phase18_fail_receipt() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.idempotency_key='phase18-rollback' AND NEW.ticket_id IS NOT NULL THEN RAISE EXCEPTION 'owned receipt failure'; END IF; RETURN NEW; END $$").execute(pool).await?;
    sqlx::query("CREATE TRIGGER phase18_fail_receipt BEFORE UPDATE ON work_order_create_idempotency FOR EACH ROW EXECUTE FUNCTION phase18_fail_receipt()").execute(pool).await?;
    let failed = http(
        client,
        business,
        TICKETS,
        &token,
        Method::POST,
        Some(json!({"title":"Atomic receipt rollback"})),
        Some("phase18-rollback"),
    )
    .await?;
    failed.require(503)?;
    let (tickets,receipts): (i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM work_orders WHERE title='Atomic receipt rollback'),(SELECT count(*) FROM work_order_create_idempotency WHERE idempotency_key='phase18-rollback')").fetch_one(pool).await?;
    ensure!(
        tickets == 0 && receipts == 0,
        "receipt failure left a half-committed create"
    );
    sqlx::query("DROP TRIGGER phase18_fail_receipt ON work_order_create_idempotency")
        .execute(pool)
        .await?;
    sqlx::query("DROP FUNCTION phase18_fail_receipt()")
        .execute(pool)
        .await?;
    http(
        client,
        business,
        TICKETS,
        &token,
        Method::POST,
        Some(json!({"title":"Atomic receipt rollback"})),
        Some("phase18-rollback"),
    )
    .await?
    .require(201)?;
    let incomplete: i64 = sqlx::query_scalar("SELECT count(*) FROM work_order_create_idempotency r LEFT JOIN work_orders w ON w.id=r.ticket_id WHERE r.ticket_id IS NULL OR r.response_json IS NULL OR w.id IS NULL OR w.tenant_id<>r.tenant_id OR w.organization_id<>r.organization_id OR w.owner_user_id<>r.user_id").fetch_one(pool).await?;
    ensure!(
        incomplete == 0,
        "committed receipt domain or completeness mismatch"
    );
    pass(checks, CHECKS[11])?;
    Ok(())
}
