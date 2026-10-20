//! Genuine platform authentication and controlled grants -> independent Rust
//! business service -> two owned disposable PostgreSQL databases.
//! Loopback HTTP is an explicit development fixture, not production TLS proof.
use std::{
    collections::HashMap,
    net::SocketAddr,
    process::Stdio,
    str::FromStr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use anyhow::{anyhow, ensure, Result};
use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::Request,
    middleware::{self, Next},
    response::Response,
    Router,
};
use reqwest::{Client, Method};
use serde_json::{json, Value};
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool,
};
use tokio::{net::TcpListener, sync::Semaphore};
use tower::ServiceExt;
use uuid::Uuid;

use crate::{config::Config, state::AppState};

const SECRET: &str = "owned-phase19-session-signing-only";
const PASSWORD: &str = "Phase19-Owned-Password-1!";
const USERNAME: &str = "phase19-write-user";
const PEER: &str = "phase19-peer-user";
const CANDIDATE_NAME: &str = "Phase19 eligible %_Alpha";
const PEER_NAME: &str = "Phase19 eligible Beta";
const CODE: &str = "work-orders";
const TICKETS: &str = "/work-orders-api/tickets";
const ACTIONS: [&str; 5] = ["create", "update", "delete", "read", "export"];
const CHECKS: [&str; 10] = [
    "real-login-app-token-registered-members",
    "directory-search-pagination",
    "assigned-create-update-clear",
    "disabled-member-denied",
    "removed-direct-membership-denied",
    "missing-assignment-action-grant-denied",
    "direct-membership-and-organization-ceiling",
    "self-owner-not-assignee",
    "directory-wait-timeout-releases-business-resources",
    "preflight-locked-organization-mismatch",
];
fn config(policy: &Value) -> Result<Config> {
    Config::from_values(&HashMap::from([
        ("APPLICATION_ENVIRONMENT".into(), "development".into()),
        (
            "DATABASE_URL".into(),
            "postgres://unused/phase19-owned".into(),
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
            "phase19_work_orders_{}_{}",
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
            self.name.starts_with("phase19_work_orders_")
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
            "MEMBER_LIVE_{}_CLEANUP assertions_ok={} database_removed={} pool_closed={}",
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
    if path.starts_with("/work-orders-api/") {
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

// The gate only delays the next selected request before the real platform
// handler. It never manufactures a member, authorization, or HTTP response.
#[derive(Default)]
struct DirectoryControl {
    pending: Mutex<Option<Arc<MemberPause>>>,
    calls: AtomicUsize,
}

#[derive(Clone, Default)]
struct DirectoryGate(Arc<DirectoryControl>);

struct MemberPause {
    path: String,
    entered: Semaphore,
    release: Semaphore,
}

struct ArmedPause(Arc<MemberPause>);

impl ArmedPause {
    async fn observed(&self) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(5), self.0.entered.acquire())
            .await
            .map_err(|_| anyhow!("real member request did not reach controlled gate"))?
            .map_err(|_| anyhow!("member gate closed"))?
            .forget();
        Ok(())
    }

    fn release(&self) {
        self.0.release.add_permits(1);
    }
}

impl Drop for ArmedPause {
    fn drop(&mut self) {
        self.release();
    }
}

impl DirectoryGate {
    fn arm(&self, candidate: Uuid) -> Result<ArmedPause> {
        let mut pending = self
            .0
            .pending
            .lock()
            .map_err(|_| anyhow!("member gate poisoned"))?;
        ensure!(pending.is_none(), "member gate already armed");
        let pause = Arc::new(MemberPause {
            path: format!("/api/apps/work-orders/members/{candidate}"),
            entered: Semaphore::new(0),
            release: Semaphore::new(0),
        });
        *pending = Some(pause.clone());
        Ok(ArmedPause(pause))
    }

    fn calls(&self) -> usize {
        self.0.calls.load(Ordering::SeqCst)
    }
}

async fn member_gate(
    State(gate): State<DirectoryGate>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    if request
        .uri()
        .path()
        .starts_with("/api/apps/work-orders/members")
    {
        gate.0.calls.fetch_add(1, Ordering::SeqCst);
    }
    let pause = {
        let mut pending = gate.0.pending.lock().expect("test member gate lock");
        if pending
            .as_ref()
            .is_some_and(|pause| pause.path == request.uri().path())
        {
            pending.take()
        } else {
            None
        }
    };
    if let Some(pause) = pause {
        pause.entered.add_permits(1);
        let _ = pause.release.acquire().await;
    }
    next.run(request).await
}

async fn create_member(
    app: &Router,
    admin: &str,
    username: &str,
    name: &str,
    organizations: &[Uuid],
) -> Result<Uuid> {
    let user = id(&api(
        app,
        "/api/users",
        admin,
        Method::POST,
        Some(json!({"username":username,"password":PASSWORD,"realName":name})),
    )
    .await?)?;
    api(app,&format!("/api/users/{user}/memberships"),admin,Method::PUT,
        Some(json!({"items":organizations.iter().map(|org|json!({"organizationId":org})).collect::<Vec<_>>()}))).await?;
    Ok(user)
}

async fn member_identity(
    app: &Router,
    admin: &str,
    tenant: Uuid,
    org: Uuid,
    user: Uuid,
) -> Result<Member> {
    let memberships = api(
        app,
        &format!("/api/users/{user}/memberships"),
        admin,
        Method::GET,
        None,
    )
    .await?;
    let row = memberships
        .as_array()
        .and_then(|rows| {
            rows.iter()
                .find(|row| row["organizationId"] == org.to_string())
        })
        .ok_or_else(|| anyhow!("formal member identity missing"))?;
    Ok(Member {
        tenant,
        org,
        user,
        membership: text_field(row, "membershipId")?.parse()?,
    })
}

struct Fixture {
    app: Router,
    application: Uuid,
    admin_org: Uuid,
    first: Member,
    second: Member,
    peer: Member,
    candidate: Uuid,
    ancestor_candidate: Uuid,
    outside_candidate: Uuid,
    outside_org: Uuid,
    foreign_candidate: Uuid,
    foreign_org: Uuid,
    client_id: String,
    client_secret: String,
    gate: DirectoryGate,
}

impl Fixture {
    async fn create(pool: &PgPool) -> Result<Self> {
        let empty = json!({"applications":[],"managementScopes":[]});
        crate::seed::initialize(pool, &config(&empty)?).await?;
        let app = router(pool, &empty)?;
        let admin_org: Uuid = sqlx::query_scalar("SELECT o.id FROM organizations o JOIN tenants t ON t.id=o.tenant_id WHERE t.is_platform AND o.parent_id IS NULL")
            .fetch_one(pool).await?;
        let proof = api(
            &app,
            "/api/auth/login",
            "",
            Method::POST,
            Some(json!({"username":"admin","password":"admin123"})),
        )
        .await?;
        let selected = api(
            &app,
            "/api/auth/context",
            &text_field(&proof, "ticket")?,
            Method::POST,
            Some(json!({"organizationId":admin_org})),
        )
        .await?;
        ensure!(
            selected["platformContext"] == true,
            "bootstrap is not actual platform root"
        );
        let admin = text_field(&selected, "accessToken")?;
        let tenant = id(&api(
            &app,
            "/api/tenants",
            &admin,
            Method::POST,
            Some(json!({"name":"Owned Phase19 Members","code":"phase19-members"})),
        )
        .await?)?;
        let root: Uuid = sqlx::query_scalar(
            "SELECT id FROM organizations WHERE tenant_id=$1 AND parent_id IS NULL",
        )
        .bind(tenant)
        .fetch_one(pool)
        .await?;
        let current = id(&api(&app,"/api/organizations",&admin,Method::POST,Some(json!({"tenantId":tenant,"parentId":root,"name":"Phase19 current","code":"phase19-current"}))).await?)?;
        let second_org = id(&api(&app,"/api/organizations",&admin,Method::POST,Some(json!({"tenantId":tenant,"parentId":current,"name":"Phase19 child","code":"phase19-child"}))).await?)?;
        let outside_org = id(&api(&app,"/api/organizations",&admin,Method::POST,Some(json!({"tenantId":tenant,"parentId":root,"name":"Phase19 sibling","code":"phase19-sibling"}))).await?)?;
        let foreign_tenant = id(&api(
            &app,
            "/api/tenants",
            &admin,
            Method::POST,
            Some(json!({"name":"Owned Phase19 Foreign","code":"phase19-foreign"})),
        )
        .await?)?;
        let foreign_org: Uuid = sqlx::query_scalar(
            "SELECT id FROM organizations WHERE tenant_id=$1 AND parent_id IS NULL",
        )
        .bind(foreign_tenant)
        .fetch_one(pool)
        .await?;
        let user = create_member(
            &app,
            &admin,
            USERNAME,
            "Owned Phase19 writer",
            &[current, second_org],
        )
        .await?;
        let peer = create_member(&app, &admin, PEER, PEER_NAME, &[current]).await?;
        let candidate = create_member(
            &app,
            &admin,
            "phase19-candidate",
            CANDIDATE_NAME,
            &[current],
        )
        .await?;
        let ancestor_candidate = create_member(
            &app,
            &admin,
            "phase19-ancestor",
            "Phase19 ancestor only",
            &[root],
        )
        .await?;
        let outside_candidate = create_member(
            &app,
            &admin,
            "phase19-outside",
            "Phase19 sibling only",
            &[outside_org],
        )
        .await?;
        let foreign_candidate = create_member(
            &app,
            &admin,
            "phase19-foreign-user",
            "Phase19 foreign only",
            &[foreign_org],
        )
        .await?;
        let application = id(&api(&app,"/api/applications",&admin,Method::POST,Some(json!({"name":"Owned Phase19 Work Orders","code":CODE,"entryType":"HTML","basePath":"/micro/work-orders"}))).await?)?;
        let mut menus = Vec::new();
        for action in ACTIONS {
            menus.push(id(&api(&app,"/api/menus",&admin,Method::POST,Some(json!({"applicationId":application,"title":format!("Phase19 {action}"),"type":"BUTTON","permission":format!("ticket:{action}"),"visible":false}))).await?)?);
        }
        let authz = json!({"applications":[{"applicationId":application,"authorized":true}],"menus":menus.iter().map(|menu|json!({"menuId":menu,"authorized":true})).collect::<Vec<_>>()});
        for tenant_id in [tenant, foreign_tenant] {
            api(
                &app,
                &format!("/api/tenants/{tenant_id}/authorizations"),
                &admin,
                Method::PUT,
                Some(authz.clone()),
            )
            .await?;
        }
        for organization in [root, current, second_org, outside_org, foreign_org] {
            api(
                &app,
                &format!("/api/organizations/{organization}/authorizations"),
                &admin,
                Method::PUT,
                Some(authz.clone()),
            )
            .await?;
        }
        for organization in [current, second_org] {
            let role = id(&api(&app,"/api/roles",&admin,Method::POST,Some(json!({"name":format!("Phase19 writer {organization}"),"organizationId":organization}))).await?)?;
            api(
                &app,
                &format!("/api/roles/{role}/menus"),
                &admin,
                Method::PUT,
                Some(json!({"menuIds":menus})),
            )
            .await?;
            let users = if organization == current {
                vec![user, peer]
            } else {
                vec![user]
            };
            api(
                &app,
                &format!("/api/roles/{role}/users"),
                &admin,
                Method::PUT,
                Some(json!({"userIds":users})),
            )
            .await?;
        }
        let oauth = api(&app,"/api/oauth/clients",&admin,Method::POST,Some(json!({"clientName":"Owned Phase19 work orders","clientType":"CONFIDENTIAL","grantTypes":["password","refresh_token"],"scopes":["profile"]}))).await?;
        api(
            &app,
            &format!("/api/oauth/clients/{}", id(&oauth)?),
            &admin,
            Method::PUT,
            Some(json!({"applicationId":application})),
        )
        .await?;
        let policy = json!({"applications":[{"applicationId":application,
            "capabilities":ACTIONS.iter().map(|action|json!({"resource":"ticket","action":action,"scopes":["SELF","ORG","ORG_SUBTREE","TENANT"]})).collect::<Vec<_>>(),
            "memberLookups":[{"resource":"ticket","action":"create"},{"resource":"ticket","action":"update"}]}],
            "managementScopes":[{"tenantId":tenant,"organizationId":root,"includeDescendants":true,"read":true,"write":true}]});
        let gate = DirectoryGate::default();
        Ok(Self {
            app: router(pool, &policy)?
                .layer(middleware::from_fn_with_state(gate.clone(), member_gate)),
            application,
            admin_org,
            first: member_identity(&app, &admin, tenant, current, user).await?,
            second: member_identity(&app, &admin, tenant, second_org, user).await?,
            peer: member_identity(&app, &admin, tenant, current, peer).await?,
            candidate,
            ancestor_candidate,
            outside_candidate,
            outside_org,
            foreign_candidate,
            foreign_org,
            client_id: text_field(&oauth, "clientId")?,
            client_secret: text_field(&oauth, "clientSecret")?,
            gate,
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
    println!("MEMBER_LIVE_{name}=PASS");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires managed explicit disposable PostgreSQL and built independent WORK_ORDERS_BINARY"]
async fn real_platform_member_directory_and_assignment() -> Result<()> {
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
        "WORK_ORDERS_MEMBER_LIVE_RESULT {}",
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
        .env("WORK_ORDERS_DB_MAX_CONNECTIONS", "1")
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
        println!("MEMBER_LIVE_PROCESS_CLEANUP stopped={process_stopped}");
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

fn member_path(org: Uuid, action: &str, user: Option<Uuid>) -> String {
    let suffix = user.map(|user| format!("/{user}")).unwrap_or_default();
    format!("/api/apps/work-orders/members{suffix}?resource=ticket&action={action}&organizationId={org}")
}

fn require_code(reply: &Reply, status: u16, code: &str) -> Result<()> {
    reply.require(status)?;
    ensure!(
        reply.json()?["code"] == code,
        "member live error classification mismatch"
    );
    Ok(())
}

async fn create_assigned(
    client: &Client,
    origin: &str,
    token: &str,
    key: &str,
    title: &str,
    assignee: Uuid,
) -> Result<Reply> {
    http(
        client,
        origin,
        TICKETS,
        token,
        Method::POST,
        Some(json!({"title":title,"assigneeUserId":assignee})),
        Some(key),
    )
    .await
}

fn pending_update(
    client: &Client,
    origin: &str,
    path: &str,
    token: &str,
    body: Value,
) -> tokio::task::JoinHandle<Result<Reply>> {
    let client = client.clone();
    let origin = origin.to_owned();
    let path = path.to_owned();
    let token = token.to_owned();
    tokio::spawn(async move {
        http(
            &client,
            &origin,
            &path,
            &token,
            Method::PATCH,
            Some(body),
            None,
        )
        .await
    })
}

async fn invisible(client: &Client, origin: &str, token: &str, ticket: &Value) -> Result<()> {
    let ticket_id = id(ticket)?.to_string();
    http(
        client,
        origin,
        &format!("{TICKETS}/{ticket_id}"),
        token,
        Method::GET,
        None,
        None,
    )
    .await?
    .require(404)?;
    for path in [TICKETS.to_owned(), format!("{TICKETS}/export.csv")] {
        let reply = http(client, origin, &path, token, Method::GET, None, None).await?;
        reply.require(200)?;
        let text = String::from_utf8_lossy(&reply.bytes);
        ensure!(
            !text.contains(&ticket_id) && !text.contains(&text_field(ticket, "number")?),
            "assignment expanded SELF read or export"
        );
    }
    Ok(())
}

async fn fault_identity(
    pool: &PgPool,
    ticket: Uuid,
    organization: Uuid,
    organization_name: &str,
    assignee: Uuid,
    assignee_name: &str,
) -> Result<()> {
    // This owned-database corruption fault is outside the normal API contract.
    // Both ALTERs and the row change are atomic: any error rolls back disabling
    // the immutable guard, and success re-enables it before releasing the gate.
    let mut transaction = pool.begin().await?;
    sqlx::query("ALTER TABLE work_orders DISABLE TRIGGER work_order_identity_immutable")
        .execute(&mut *transaction)
        .await?;
    sqlx::query("UPDATE work_orders SET organization_id=$2,organization_name=$3,assignee_user_id=$4,assignee_display_name=$5 WHERE id=$1")
        .bind(ticket).bind(organization).bind(organization_name).bind(assignee).bind(assignee_name)
        .execute(&mut *transaction).await?;
    sqlx::query("ALTER TABLE work_orders ENABLE TRIGGER work_order_identity_immutable")
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
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
    ensure!(root["platformContext"] == true, "real root context missing");
    let admin = text_field(&root, "accessToken")?;
    let selected = login(client, platform, USERNAME, PASSWORD, f.first.org).await?;
    ensure!(
        selected["platformContext"] == false,
        "ordinary user received root context"
    );
    let token = issue(client, platform, &text_field(&selected, "accessToken")?).await?;
    for member in [&f.first, &f.second, &f.peer] {
        grants(
            client,
            platform,
            &admin,
            member,
            f.application,
            &ACTIONS,
            &[],
        )
        .await?;
    }
    let introspection = client
        .post(format!("{platform}/oauth/introspect"))
        .basic_auth(&f.client_id, Some(&f.client_secret))
        .form(&[("token", token.as_str())])
        .send()
        .await
        .map_err(|_| anyhow!("real member confidential introspection failed"))?;
    ensure!(
        introspection.status().as_u16() == 200,
        "real member introspection status mismatch"
    );
    let intro: Value = serde_json::from_slice(
        &introspection
            .bytes()
            .await
            .map_err(|_| anyhow!("introspection body missing"))?,
    )?;
    ensure!(
        intro["active"] == true && intro["aud"] == CODE && intro["sub"] == f.first.user.to_string(),
        "real member token identity mismatch"
    );
    let member = http(
        client,
        platform,
        &member_path(f.first.org, "create", Some(f.candidate)),
        &token,
        Method::GET,
        None,
        None,
    )
    .await?;
    member.require(200)?;
    ensure!(
        member.json()?["id"] == f.candidate.to_string()
            && member.json()?["displayName"] == CANDIDATE_NAME
            && member.json()?["organizationId"] == f.first.org.to_string(),
        "registered real member response mismatch"
    );
    // delete is a real granted capability, but is not registered for member use.
    http(
        client,
        platform,
        &member_path(f.first.org, "delete", None),
        &token,
        Method::GET,
        None,
        None,
    )
    .await?
    .require(400)?;
    pass(checks, CHECKS[0])?;

    let mut seen = std::collections::BTreeSet::new();
    for page in [1, 2] {
        let reply = http(
            client,
            business,
            &format!(
                "/work-orders-api/assignees?action=create&q=Phase19%20eligible&page={page}&size=1"
            ),
            &token,
            Method::GET,
            None,
            None,
        )
        .await?;
        reply.require(200)?;
        let value = reply.json()?;
        ensure!(
            value["total"] == 2 && value["page"] == page && value["size"] == 1,
            "member pagination metadata changed"
        );
        let records = value["records"]
            .as_array()
            .ok_or_else(|| anyhow!("member page lacks records"))?;
        ensure!(records.len() == 1, "member page cardinality mismatch");
        ensure!(
            records[0]["organizationId"] == f.first.org.to_string()
                && records[0].get("id").is_none(),
            "business member projection changed"
        );
        ensure!(
            seen.insert(text_field(&records[0], "userId")?),
            "member pages repeated a candidate"
        );
    }
    ensure!(
        seen == [f.peer.user.to_string(), f.candidate.to_string()]
            .into_iter()
            .collect(),
        "member page set differs from formal direct memberships"
    );
    for (query, total) in [
        ("q=%25_", 1),
        ("q=NoMatchingPhase19Name", 0),
        ("q=Phase19%20eligible&page=1000000&size=50", 2),
    ] {
        let reply = http(
            client,
            business,
            &format!("/work-orders-api/assignees?action=update&{query}"),
            &token,
            Method::GET,
            None,
            None,
        )
        .await?;
        reply.require(200)?;
        let value = reply.json()?;
        ensure!(
            value["total"] == total,
            "member literal search/total mismatch"
        );
        if query.contains("1000000") || total == 0 {
            ensure!(
                value["records"] == json!([]),
                "member empty page/search changed"
            );
        }
        if total == 1 {
            ensure!(
                value["records"][0]["userId"] == f.candidate.to_string(),
                "wildcard search was not literal"
            );
        }
    }
    for query in [
        "action=read",
        "action=create&size=51",
        "action=create&page=0",
        "action=create&unknown=1",
    ] {
        http(
            client,
            business,
            &format!("/work-orders-api/assignees?{query}"),
            &token,
            Method::GET,
            None,
            None,
        )
        .await?
        .require(400)?;
    }
    pass(checks, CHECKS[1])?;

    let original = create_assigned(
        client,
        business,
        &token,
        "phase19-assigned",
        "Real assigned create",
        f.candidate,
    )
    .await?;
    original.require(201)?;
    let original = original.json()?;
    let ticket = id(&original)?;
    let detail = format!("{TICKETS}/{ticket}");
    ensure!(
        original["assigneeUserId"] == f.candidate.to_string()
            && original["assigneeDisplayName"] == CANDIDATE_NAME
            && original["ownerUserId"] == f.first.user.to_string(),
        "assigned create did not use verified platform identity"
    );
    let replay=http(client,business,TICKETS,&token,Method::POST,Some(json!({"title":"Real assigned create","assigneeUserId":f.candidate.to_string().to_uppercase()})),Some("phase19-assigned")).await?;
    replay.require(201)?;
    ensure!(
        replay.json()? == original,
        "canonical assigned receipt did not replay"
    );
    let changed = http(
        client,
        business,
        &detail,
        &token,
        Method::PATCH,
        Some(json!({"version":1,"assigneeUserId":f.peer.user})),
        None,
    )
    .await?;
    changed.require(200)?;
    ensure!(
        changed.json()?["version"] == 2
            && changed.json()?["assigneeUserId"] == f.peer.user.to_string()
            && changed.json()?["assigneeDisplayName"] == PEER_NAME,
        "assigned update used caller-controlled or stale member identity"
    );
    let calls = f.gate.calls();
    http(
        client,
        business,
        &detail,
        &token,
        Method::PATCH,
        Some(json!({"version":2,"title":"Assignment omitted"})),
        None,
    )
    .await?
    .require(200)?;
    let cleared = http(
        client,
        business,
        &detail,
        &token,
        Method::PATCH,
        Some(json!({"version":3,"assigneeUserId":null})),
        None,
    )
    .await?;
    cleared.require(200)?;
    ensure!(
        cleared.json()?["version"] == 4
            && cleared.json()?["assigneeUserId"].is_null()
            && cleared.json()?["assigneeDisplayName"].is_null(),
        "assignment clear changed contract"
    );
    ensure!(
        f.gate.calls() == calls,
        "omitted/cleared assignment called member directory"
    );
    pass(checks, CHECKS[2])?;

    let history = create_assigned(
        client,
        business,
        &token,
        "phase19-historical",
        "Historical assignment",
        f.candidate,
    )
    .await?;
    history.require(201)?;
    let history_path = format!("{TICKETS}/{}", id(&history.json()?)?);
    http(
        client,
        platform,
        &format!("/api/users/{}/status", f.candidate),
        &admin,
        Method::PUT,
        Some(json!({"status":"DISABLED"})),
        None,
    )
    .await?
    .require(200)?;
    http(
        client,
        platform,
        &member_path(f.first.org, "create", Some(f.candidate)),
        &token,
        Method::GET,
        None,
        None,
    )
    .await?
    .require(404)?;
    let missing = http(
        client,
        business,
        "/work-orders-api/assignees?action=create&q=%25_",
        &token,
        Method::GET,
        None,
        None,
    )
    .await?;
    missing.require(200)?;
    ensure!(
        missing.json()?["total"] == 0,
        "disabled member remained in directory"
    );
    require_code(
        &create_assigned(
            client,
            business,
            &token,
            "phase19-disabled",
            "Disabled target",
            f.candidate,
        )
        .await?,
        404,
        "MEMBER_NOT_FOUND",
    )?;
    require_code(
        &http(
            client,
            business,
            &detail,
            &token,
            Method::PATCH,
            Some(json!({"version":4,"assigneeUserId":f.candidate})),
            None,
        )
        .await?,
        404,
        "MEMBER_NOT_FOUND",
    )?;
    let calls = f.gate.calls();
    let retained=http(client,business,&history_path,&token,Method::PATCH,Some(json!({"version":1,"assigneeUserId":f.candidate,"title":"Disabled historical same-value"})),None).await?;
    retained.require(200)?;
    ensure!(
        retained.json()?["assigneeDisplayName"] == CANDIDATE_NAME
            && retained.json()?["version"] == 2,
        "same-value historical assignment was reinterpreted"
    );
    let recovered = create_assigned(
        client,
        business,
        &token,
        "phase19-assigned",
        "Real assigned create",
        f.candidate,
    )
    .await?;
    recovered.require(201)?;
    ensure!(
        recovered.json()? == original && f.gate.calls() == calls,
        "historical receipt/same-value update revalidated disabled member"
    );
    http(
        client,
        platform,
        &format!("/api/users/{}/status", f.candidate),
        &admin,
        Method::PUT,
        Some(json!({"status":"ENABLED"})),
        None,
    )
    .await?
    .require(200)?;
    pass(checks, CHECKS[3])?;

    http(
        client,
        platform,
        &format!("/api/users/{}/memberships", f.candidate),
        &admin,
        Method::PUT,
        Some(json!({"items":[]})),
        None,
    )
    .await?
    .require(200)?;
    http(
        client,
        platform,
        &member_path(f.first.org, "update", Some(f.candidate)),
        &token,
        Method::GET,
        None,
        None,
    )
    .await?
    .require(404)?;
    require_code(
        &create_assigned(
            client,
            business,
            &token,
            "phase19-removed",
            "Removed member",
            f.candidate,
        )
        .await?,
        404,
        "MEMBER_NOT_FOUND",
    )?;
    require_code(
        &http(
            client,
            business,
            &detail,
            &token,
            Method::PATCH,
            Some(json!({"version":4,"assigneeUserId":f.candidate})),
            None,
        )
        .await?,
        404,
        "MEMBER_NOT_FOUND",
    )?;
    let calls = f.gate.calls();
    let recovered = create_assigned(
        client,
        business,
        &token,
        "phase19-assigned",
        "Real assigned create",
        f.candidate,
    )
    .await?;
    recovered.require(201)?;
    ensure!(
        recovered.json()? == original && f.gate.calls() == calls,
        "receipt recovery required removed direct membership"
    );
    http(
        client,
        platform,
        &format!("/api/users/{}/memberships", f.candidate),
        &admin,
        Method::PUT,
        Some(json!({"items":[{"organizationId":f.first.org}]})),
        None,
    )
    .await?
    .require(200)?;
    pass(checks, CHECKS[4])?;

    for action in ["create", "update"] {
        grants(
            client,
            platform,
            &admin,
            &f.first,
            f.application,
            &[],
            &[action],
        )
        .await?;
        http(
            client,
            platform,
            &member_path(f.first.org, action, None),
            &token,
            Method::GET,
            None,
            None,
        )
        .await?
        .require(403)?;
        http(
            client,
            business,
            &format!("/work-orders-api/assignees?action={action}"),
            &token,
            Method::GET,
            None,
            None,
        )
        .await?
        .require(403)?;
        if action == "create" {
            let denied = create_assigned(
                client,
                business,
                &token,
                "phase19-assigned",
                "Real assigned create",
                f.candidate,
            )
            .await?;
            denied.require(403)?;
            ensure!(
                !String::from_utf8_lossy(&denied.bytes).contains(&ticket.to_string()),
                "revoked create action leaked old receipt"
            );
        } else {
            http(
                client,
                business,
                &detail,
                &token,
                Method::PATCH,
                Some(json!({"version":4,"assigneeUserId":f.candidate})),
                None,
            )
            .await?
            .require(403)?;
        }
        grants(
            client,
            platform,
            &admin,
            &f.first,
            f.application,
            &[action],
            &[],
        )
        .await?;
    }
    pass(checks, CHECKS[5])?;

    for (index, candidate) in [
        f.ancestor_candidate,
        f.outside_candidate,
        f.foreign_candidate,
    ]
    .into_iter()
    .enumerate()
    {
        http(
            client,
            platform,
            &member_path(f.first.org, "create", Some(candidate)),
            &token,
            Method::GET,
            None,
            None,
        )
        .await?
        .require(404)?;
        require_code(
            &create_assigned(
                client,
                business,
                &token,
                &format!("phase19-outside-{index}"),
                "Not a direct member",
                candidate,
            )
            .await?,
            404,
            "MEMBER_NOT_FOUND",
        )?;
    }
    for org in [f.second.org, f.outside_org, f.foreign_org] {
        http(
            client,
            business,
            &format!("/work-orders-api/assignees?action=create&organizationId={org}"),
            &token,
            Method::GET,
            None,
            None,
        )
        .await?
        .require(403)?;
        http(
            client,
            platform,
            &member_path(org, "create", None),
            &token,
            Method::GET,
            None,
            None,
        )
        .await?
        .require(404)?;
    }
    let child_context = login(client, platform, USERNAME, PASSWORD, f.second.org).await?;
    let child_token = issue(
        client,
        platform,
        &text_field(&child_context, "accessToken")?,
    )
    .await?;
    let child = create_assigned(
        client,
        business,
        &child_token,
        "phase19-child",
        "Own child ticket",
        f.first.user,
    )
    .await?;
    child.require(201)?;
    let child_detail = format!("{TICKETS}/{}", id(&child.json()?)?);
    http(
        client,
        business,
        &child_detail,
        &token,
        Method::GET,
        None,
        None,
    )
    .await?
    .require(200)?;
    let calls = f.gate.calls();
    http(client,business,&child_detail,&token,Method::PATCH,Some(json!({"version":1,"assigneeUserId":f.first.user,"title":"Same-value child remains editable"})),None).await?.require(200)?;
    ensure!(
        f.gate.calls() == calls,
        "same-value child assignment widened directory scope or revalidated member"
    );
    http(
        client,
        business,
        &child_detail,
        &token,
        Method::PATCH,
        Some(json!({"version":2,"assigneeUserId":null})),
        None,
    )
    .await?
    .require(403)?;
    pass(checks, CHECKS[6])?;

    let peer_context = login(client, platform, PEER, PASSWORD, f.peer.org).await?;
    let peer_token = issue(client, platform, &text_field(&peer_context, "accessToken")?).await?;
    let owned = create_assigned(
        client,
        business,
        &token,
        "phase19-self-owner",
        "Owner-only assigned ticket",
        f.peer.user,
    )
    .await?;
    owned.require(201)?;
    let owned = owned.json()?;
    invisible(client, business, &peer_token, &owned).await?;
    let peer_cannot_edit = format!("{TICKETS}/{}", id(&owned)?);
    http(
        client,
        business,
        &peer_cannot_edit,
        &peer_token,
        Method::PATCH,
        Some(json!({"version":1,"title":"Assignee cannot edit other's SELF ticket"})),
        None,
    )
    .await?
    .require(404)?;
    http(
        client,
        business,
        &peer_cannot_edit,
        &peer_token,
        Method::DELETE,
        Some(json!({"version":1})),
        None,
    )
    .await?
    .require(404)?;
    let peer_owned = create_assigned(
        client,
        business,
        &peer_token,
        "phase19-peer-owner",
        "Peer-owned assigned back",
        f.first.user,
    )
    .await?;
    peer_owned.require(201)?;
    invisible(client, business, &token, &peer_owned.json()?).await?;
    pass(checks, CHECKS[7])?;

    let pause = f.gate.arm(f.candidate)?;
    let pending = pending_update(
        client,
        business,
        &detail,
        &token,
        json!({"version":4,"assigneeUserId":f.candidate}),
    );
    pause.observed().await?;
    let readable = tokio::time::timeout(
        Duration::from_secs(1),
        http(client, business, &detail, &token, Method::GET, None, None),
    )
    .await
    .map_err(|_| anyhow!("member HTTP held the business service's sole pool connection"))??;
    readable.require(200)?;
    let mut probe = pool.begin().await?;
    sqlx::query("SELECT id FROM work_orders WHERE id=$1 FOR UPDATE NOWAIT")
        .bind(ticket)
        .fetch_one(&mut *probe)
        .await
        .map_err(|_| anyhow!("member HTTP held a work-order row lock"))?;
    probe.rollback().await?;
    pause.release();
    let assigned = pending.await??;
    assigned.require(200)?;
    ensure!(
        assigned.json()?["version"] == 5
            && assigned.json()?["assigneeUserId"] == f.candidate.to_string(),
        "gated real member response did not produce valid assignment"
    );
    drop(pause);
    let pause = f.gate.arm(f.peer.user)?;
    let started = std::time::Instant::now();
    let pending = pending_update(
        client,
        business,
        &detail,
        &token,
        json!({"version":5,"assigneeUserId":f.peer.user,"title":"Must not commit after member timeout"}),
    );
    pause.observed().await?;
    let timed_out = pending.await??;
    require_code(&timed_out, 503, "PLATFORM_UNAVAILABLE")?;
    ensure!(
        started.elapsed() < Duration::from_secs(6),
        "member timeout exceeded its bounded HTTP budget"
    );
    pause.release();
    drop(pause);
    let unchanged = http(client, business, &detail, &token, Method::GET, None, None).await?;
    unchanged.require(200)?;
    ensure!(
        unchanged.json()?["version"] == 5
            && unchanged.json()?["assigneeUserId"] == f.candidate.to_string()
            && unchanged.json()?["title"] == "Assignment omitted",
        "member timeout wrote a late or partial assignment"
    );
    println!(
        "MEMBER_LIVE_BUDGET sole_pool_available=true row_lock_nowait=true member_timeout_ms={}",
        started.elapsed().as_millis()
    );
    pass(checks, CHECKS[8])?;

    // Only the owned business database is faulted. Its immutable trigger is
    // restored atomically before the real member HTTP response is released.
    // This is corruption injection, not a supported organization edit route.
    for fault in ["organization", "assignee"] {
        let pause = f.gate.arm(f.peer.user)?;
        let pending = pending_update(
            client,
            business,
            &detail,
            &token,
            json!({"version":5,"assigneeUserId":f.peer.user}),
        );
        pause.observed().await?;
        fault_identity(
            pool,
            ticket,
            if fault == "organization" {
                f.second.org
            } else {
                f.first.org
            },
            if fault == "organization" {
                "Phase19 injected child"
            } else {
                "Phase19 current"
            },
            if fault == "assignee" {
                f.first.user
            } else {
                f.candidate
            },
            if fault == "assignee" {
                "Phase19 injected identity"
            } else {
                CANDIDATE_NAME
            },
        )
        .await?;
        pause.release();
        let rejected = pending.await??;
        require_code(&rejected, 409, "VERSION_CONFLICT")?;
        drop(pause);
        let (version, assignee, organization): (i32, Option<Uuid>, Uuid) = sqlx::query_as(
            "SELECT version,assignee_user_id,organization_id FROM work_orders WHERE id=$1",
        )
        .bind(ticket)
        .fetch_one(pool)
        .await?;
        ensure!(
            version == 5
                && assignee
                    == Some(if fault == "assignee" {
                        f.first.user
                    } else {
                        f.candidate
                    })
                && organization
                    == if fault == "organization" {
                        f.second.org
                    } else {
                        f.first.org
                    },
            "preflight identity fault was overwritten by a stale member assertion"
        );
        fault_identity(
            pool,
            ticket,
            f.first.org,
            "Phase19 current",
            f.candidate,
            CANDIDATE_NAME,
        )
        .await?;
    }
    pass(checks, CHECKS[9])?;
    Ok(())
}
