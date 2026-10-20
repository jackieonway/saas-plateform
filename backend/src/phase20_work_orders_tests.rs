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

const SECRET: &str = "owned-phase20-session-signing-only";
const PASSWORD: &str = "Phase20-Owned-Password-1!";
const USERNAME: &str = "phase20-write-user";
const PEER: &str = "phase20-peer-user";
const CANDIDATE_NAME: &str = "Phase20 reminder recipient";
const PEER_NAME: &str = "Phase20 eligible Beta";
const CODE: &str = "work-orders";
const TICKETS: &str = "/work-orders-api/tickets";
const ACTIONS: [&str; 5] = ["create", "update", "delete", "read", "export"];
const CHECKS: [&str; 12] = [
    "real-login-app-token-official-grants",
    "persisted-assignee-reminder-no-ticket-write",
    "same-event-idempotent-receipt",
    "owner-fallback-and-self-scope",
    "missing-ticket-read-permission-denied",
    "missing-ticket-read-grant-denied",
    "missing-notification-publish-permission-denied",
    "missing-notification-publish-grant-denied",
    "recipient-enabled-direct-membership-and-scope",
    "precommit-unknown-intent-recipient-conflict",
    "committed-response-loss-same-key-recovery",
    "realtime-absent-persistence-still-success",
];
fn config(policy: &Value) -> Result<Config> {
    Config::from_values(&HashMap::from([
        ("APPLICATION_ENVIRONMENT".into(), "development".into()),
        (
            "DATABASE_URL".into(),
            "postgres://unused/phase20-owned".into(),
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
            "phase20_work_orders_{}_{}",
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
            self.name.starts_with("phase20_work_orders_")
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
            "REMINDER_LIVE_{}_CLEANUP assertions_ok={} database_removed={} pool_closed={}",
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

// Faults only pause a real handler or its already-produced response. They never
// manufacture a successful platform result or bypass platform authorization.
#[derive(Clone, Copy)]
enum PauseAt {
    Before,
    After,
}

#[derive(Default)]
struct PublicationControl {
    pending: Mutex<Option<Arc<PublicationPause>>>,
    calls: AtomicUsize,
}

#[derive(Clone, Default)]
struct PublicationGate(Arc<PublicationControl>);

struct PublicationPause {
    at: PauseAt,
    entered: Semaphore,
    release: Semaphore,
    completed: Semaphore,
    status: AtomicUsize,
}

struct ArmedPause(Arc<PublicationPause>);

impl ArmedPause {
    async fn observed(&self) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(5), self.0.entered.acquire())
            .await
            .map_err(|_| anyhow!("real publication did not reach controlled gate"))?
            .map_err(|_| anyhow!("publication gate closed"))?
            .forget();
        Ok(())
    }

    fn release(&self) {
        self.0.release.add_permits(1);
    }

    async fn completed(&self, status: u16) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(5), self.0.completed.acquire())
            .await
            .map_err(|_| anyhow!("real publication handler did not finish"))?
            .map_err(|_| anyhow!("publication completion gate closed"))?
            .forget();
        ensure!(
            self.0.status.load(Ordering::SeqCst) == usize::from(status),
            "real publication handler status mismatch"
        );
        Ok(())
    }
}

impl Drop for ArmedPause {
    fn drop(&mut self) {
        self.release();
    }
}

impl PublicationGate {
    fn arm(&self, at: PauseAt) -> Result<ArmedPause> {
        let mut pending = self
            .0
            .pending
            .lock()
            .map_err(|_| anyhow!("publication gate poisoned"))?;
        ensure!(pending.is_none(), "publication gate already armed");
        let pause = Arc::new(PublicationPause {
            at,
            entered: Semaphore::new(0),
            release: Semaphore::new(0),
            completed: Semaphore::new(0),
            status: AtomicUsize::new(0),
        });
        *pending = Some(pause.clone());
        Ok(ArmedPause(pause))
    }

    fn calls(&self) -> usize {
        self.0.calls.load(Ordering::SeqCst)
    }
}

async fn publication_gate(
    State(gate): State<PublicationGate>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let publication = request.method() == Method::POST
        && request.uri().path() == "/api/apps/work-orders/notifications";
    let pause = if publication {
        gate.0.calls.fetch_add(1, Ordering::SeqCst);
        gate.0
            .pending
            .lock()
            .expect("test publication gate lock")
            .take()
    } else {
        None
    };
    let Some(pause) = pause else {
        return next.run(request).await;
    };
    // Keep ownership of the genuine handler when the business client times out.
    // Gate release then has a definite outcome even if Hyper drops its caller.
    tokio::spawn(async move {
        if matches!(pause.at, PauseAt::Before) {
            pause.entered.add_permits(1);
            let _ = pause.release.acquire().await;
        }
        let response = next.run(request).await;
        pause
            .status
            .store(usize::from(response.status().as_u16()), Ordering::SeqCst);
        if matches!(pause.at, PauseAt::After) {
            pause.entered.add_permits(1);
            let _ = pause.release.acquire().await;
        }
        pause.completed.add_permits(1);
        response
    })
    .await
    .expect("owned real publication handler task")
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
    api(app, &format!("/api/users/{user}/memberships"), admin, Method::PUT,
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
    role: Uuid,
    menus: HashMap<String, Uuid>,
    client_id: String,
    client_secret: String,
    gate: PublicationGate,
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
            Some(json!({"name":"Owned Phase20 Reminders","code":"phase20-reminders"})),
        )
        .await?)?;
        let root: Uuid = sqlx::query_scalar(
            "SELECT id FROM organizations WHERE tenant_id=$1 AND parent_id IS NULL",
        )
        .bind(tenant)
        .fetch_one(pool)
        .await?;
        let current = id(&api(&app,"/api/organizations",&admin,Method::POST,
            Some(json!({"tenantId":tenant,"parentId":root,"name":"Phase20 current","code":"phase20-current"}))).await?)?;
        let child = id(&api(&app,"/api/organizations",&admin,Method::POST,
            Some(json!({"tenantId":tenant,"parentId":current,"name":"Phase20 child","code":"phase20-child"}))).await?)?;
        let user = create_member(
            &app,
            &admin,
            USERNAME,
            "Owned Phase20 publisher",
            &[current, child],
        )
        .await?;
        let peer = create_member(&app, &admin, PEER, PEER_NAME, &[current]).await?;
        let candidate = create_member(
            &app,
            &admin,
            "phase20-candidate",
            CANDIDATE_NAME,
            &[current],
        )
        .await?;
        let application = id(&api(&app,"/api/applications",&admin,Method::POST,
            Some(json!({"name":"Owned Phase20 Work Orders","code":CODE,"entryType":"HTML","basePath":"/micro/work-orders"}))).await?)?;
        let mut menus = HashMap::new();
        for permission in ACTIONS
            .iter()
            .map(|action| format!("ticket:{action}"))
            .chain(std::iter::once("notification:publish".to_owned()))
        {
            let menu = id(&api(&app,"/api/menus",&admin,Method::POST,
                Some(json!({"applicationId":application,"title":format!("Phase20 {permission}"),"type":"BUTTON","permission":permission,"visible":false}))).await?)?;
            menus.insert(permission, menu);
        }
        let menu_ids = menus.values().copied().collect::<Vec<_>>();
        let authz = json!({"applications":[{"applicationId":application,"authorized":true}],
            "menus":menu_ids.iter().map(|menu|json!({"menuId":menu,"authorized":true})).collect::<Vec<_>>()});
        api(
            &app,
            &format!("/api/tenants/{tenant}/authorizations"),
            &admin,
            Method::PUT,
            Some(authz.clone()),
        )
        .await?;
        for org in [root, current, child] {
            api(
                &app,
                &format!("/api/organizations/{org}/authorizations"),
                &admin,
                Method::PUT,
                Some(authz.clone()),
            )
            .await?;
        }
        let mut current_role = None;
        for org in [current, child] {
            let role = id(&api(
                &app,
                "/api/roles",
                &admin,
                Method::POST,
                Some(json!({"name":format!("Phase20 publisher {org}"),"organizationId":org})),
            )
            .await?)?;
            api(
                &app,
                &format!("/api/roles/{role}/menus"),
                &admin,
                Method::PUT,
                Some(json!({"menuIds":menu_ids})),
            )
            .await?;
            let users = if org == current {
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
            if org == current {
                current_role = Some(role);
            }
        }
        let oauth = api(&app,"/api/oauth/clients",&admin,Method::POST,
            Some(json!({"clientName":"Owned Phase20 work orders","clientType":"CONFIDENTIAL","grantTypes":["password","refresh_token"],"scopes":["profile"]}))).await?;
        api(
            &app,
            &format!("/api/oauth/clients/{}", id(&oauth)?),
            &admin,
            Method::PUT,
            Some(json!({"applicationId":application})),
        )
        .await?;
        let mut capabilities = ACTIONS.iter().map(|action|json!({"resource":"ticket","action":action,"scopes":["SELF","ORG","ORG_SUBTREE","TENANT"]})).collect::<Vec<_>>();
        capabilities.push(json!({"resource":"notification","action":"publish","scopes":["SELF","ORG","ORG_SUBTREE","TENANT"]}));
        let policy = json!({"applications":[{"applicationId":application,"capabilities":capabilities,
            "memberLookups":[{"resource":"ticket","action":"create"},{"resource":"ticket","action":"update"}]}],
            "managementScopes":[{"tenantId":tenant,"organizationId":root,"includeDescendants":true,"read":true,"write":true}]});
        let gate = PublicationGate::default();
        Ok(Self {
            app: router(pool, &policy)?.layer(middleware::from_fn_with_state(
                gate.clone(),
                publication_gate,
            )),
            application,
            admin_org,
            first: member_identity(&app, &admin, tenant, current, user).await?,
            second: member_identity(&app, &admin, tenant, child, user).await?,
            peer: member_identity(&app, &admin, tenant, current, peer).await?,
            candidate,
            role: current_role.ok_or_else(|| anyhow!("current publisher role missing"))?,
            menus,
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
    upsert: &[(&str, &str, &str)],
    remove: &[(&str, &str)],
) -> Result<()> {
    let target = member.target(app);
    let query = target
        .as_object()
        .ok_or_else(|| anyhow!("grant target invalid"))?
        .iter()
        .map(|(key, value)| format!("{key}={}", value.as_str().unwrap_or_default()))
        .collect::<Vec<_>>()
        .join("&");
    let path = format!("/api/data-grants?{query}");
    let before = http(client, origin, &path, admin, Method::GET, None, None).await?;
    before.require(200)?;
    let before = before.json()?;
    let body = json!({"target":target,"expectedVersion":before["version"],
        "upsert":upsert.iter().map(|(resource,action,scope)|json!({"resource":resource,"action":action,"scope":scope})).collect::<Vec<_>>(),
        "remove":remove.iter().map(|(resource,action)|json!({"resource":resource,"action":action})).collect::<Vec<_>>()});
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

async fn permissions(
    client: &Client,
    platform: &str,
    admin: &str,
    f: &Fixture,
    omitted: Option<&str>,
) -> Result<()> {
    let menus = f
        .menus
        .iter()
        .filter(|(permission, _)| Some(permission.as_str()) != omitted)
        .map(|(_, menu)| *menu)
        .collect::<Vec<_>>();
    http(
        client,
        platform,
        &format!("/api/roles/{}/menus", f.role),
        admin,
        Method::PUT,
        Some(json!({"menuIds":menus})),
        None,
    )
    .await?
    .require(200)
}

async fn candidate_status(
    client: &Client,
    platform: &str,
    admin: &str,
    f: &Fixture,
    status: &str,
) -> Result<()> {
    http(
        client,
        platform,
        &format!("/api/users/{}/status", f.candidate),
        admin,
        Method::PUT,
        Some(json!({"status":status})),
        None,
    )
    .await?
    .require(200)
}

async fn candidate_membership(
    client: &Client,
    platform: &str,
    admin: &str,
    f: &Fixture,
    included: bool,
) -> Result<()> {
    let items = if included {
        vec![json!({"organizationId":f.first.org})]
    } else {
        Vec::new()
    };
    http(
        client,
        platform,
        &format!("/api/users/{}/memberships", f.candidate),
        admin,
        Method::PUT,
        Some(json!({"items":items})),
        None,
    )
    .await?
    .require(200)
}

fn require_code(reply: &Reply, status: u16, code: &str) -> Result<()> {
    reply.require(status)?;
    ensure!(
        reply.json()?["code"] == code,
        "reminder error classification mismatch"
    );
    Ok(())
}

fn pass(checks: &mut Vec<Value>, name: &str) -> Result<()> {
    ensure!(
        CHECKS.contains(&name) && !checks.iter().any(|value| value["name"] == name),
        "unknown or duplicate reminder live check"
    );
    checks.push(json!({"name":name,"status":"PASS"}));
    println!("REMINDER_LIVE_{name}=PASS");
    Ok(())
}

async fn create_ticket(
    client: &Client,
    business: &str,
    token: &str,
    key: &str,
    assignee: Option<Uuid>,
) -> Result<Value> {
    let mut body = json!({"title":format!("Phase20 reminder {key}")});
    if let Some(assignee) = assignee {
        body["assigneeUserId"] = json!(assignee);
    }
    let response = http(
        client,
        business,
        TICKETS,
        token,
        Method::POST,
        Some(body),
        Some(key),
    )
    .await?;
    response.require(201)?;
    response.json()
}

async fn remind(
    client: &Client,
    business: &str,
    token: &str,
    ticket: Uuid,
    key: &str,
) -> Result<Reply> {
    http(
        client,
        business,
        &format!("{TICKETS}/{ticket}/remind"),
        token,
        Method::POST,
        Some(json!({"idempotencyKey":key})),
        None,
    )
    .await
}

fn pending_remind(
    client: &Client,
    business: &str,
    token: &str,
    ticket: Uuid,
    key: &str,
) -> tokio::task::JoinHandle<Result<Reply>> {
    let client = client.clone();
    let business = business.to_owned();
    let token = token.to_owned();
    let key = key.to_owned();
    tokio::spawn(async move { remind(&client, &business, &token, ticket, &key).await })
}

async fn update_assignee(
    client: &Client,
    business: &str,
    token: &str,
    ticket: &Value,
    assignee: Uuid,
) -> Result<Value> {
    let response = http(
        client,
        business,
        &format!("{TICKETS}/{}", id(ticket)?),
        token,
        Method::PATCH,
        Some(json!({"version":ticket["version"],"assigneeUserId":assignee})),
        None,
    )
    .await?;
    response.require(200)?;
    response.json()
}

async fn ticket_snapshot(pool: &PgPool, ticket: Uuid) -> Result<Value> {
    let snapshot: sqlx::types::Json<Value> =
        sqlx::query_scalar("SELECT to_jsonb(w) FROM work_orders w WHERE id=$1")
            .bind(ticket)
            .fetch_one(pool)
            .await?;
    Ok(snapshot.0)
}

async fn counts(pool: &PgPool, key: &str) -> Result<(i64, i64)> {
    let publications: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notification_publications WHERE idempotency_key=$1",
    )
    .bind(key)
    .fetch_one(pool)
    .await?;
    let notifications: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications n JOIN notification_publications p ON p.id=n.publication_id WHERE p.idempotency_key=$1")
        .bind(key).fetch_one(pool).await?;
    Ok((publications, notifications))
}

type PersistedDeliveryRow = (
    Uuid,
    Uuid,
    Uuid,
    Uuid,
    String,
    String,
    String,
    String,
    String,
);

async fn persisted(
    pool: &PgPool,
    f: &Fixture,
    ticket: &Value,
    key: &str,
    user: Uuid,
    response: &Value,
) -> Result<Uuid> {
    let publication: Uuid = text_field(response, "publicationId")?.parse()?;
    ensure!(
        response["recipientCount"] == 1
            && response["deliveryCount"] == 1
            && response["duplicate"].is_boolean()
            && chrono::DateTime::parse_from_rfc3339(&text_field(response, "createdAt")?).is_ok(),
        "invalid actual publication receipt"
    );
    let expected_title = format!("工单提醒 {}", text_field(ticket, "number")?);
    let expected_content = format!(
        "请查看工单 {} 的详情并处理。",
        text_field(ticket, "number")?
    );
    let expected_link = format!("/micro/work-orders/tickets/{}", id(ticket)?);
    let rows: Vec<PersistedDeliveryRow> = sqlx::query_as(
        "SELECT n.id,n.user_id,n.tenant_id,n.organization_id,n.source_kind,n.type,n.title,n.content,n.link FROM notifications n \
         JOIN notification_publications p ON p.id=n.publication_id WHERE p.id=$1 AND p.idempotency_key=$2 \
         AND p.application_id=$3 AND p.publisher_user_id=$4 AND p.publisher_organization_id=$5")
        .bind(publication).bind(key).bind(f.application).bind(f.first.user).bind(f.first.org).fetch_all(pool).await?;
    ensure!(
        rows.len() == 1,
        "publication did not persist exactly one recipient notification"
    );
    let row = &rows[0];
    ensure!(
        row.1 == user
            && row.2 == f.first.tenant
            && row.3.to_string() == text_field(ticket, "organizationId")?
            && row.4 == "APP"
            && row.5 == "INFO"
            && row.6 == expected_title
            && row.7 == expected_content
            && row.8 == expected_link,
        "persisted notification context/body differs from trusted work order"
    );
    ensure!(
        counts(pool, key).await? == (1, 1),
        "event generated more than one durable notification"
    );
    Ok(row.0)
}

async fn inbox_contains(
    client: &Client,
    platform: &str,
    session: &str,
    notification: Uuid,
    present: bool,
) -> Result<()> {
    let response = http(
        client,
        platform,
        "/api/notifications?page=1&size=100",
        session,
        Method::GET,
        None,
        None,
    )
    .await?;
    response.require(200)?;
    let body = response.json()?;
    let found = body["records"]
        .as_array()
        .ok_or_else(|| anyhow!("real inbox records missing"))?
        .iter()
        .any(|row| row["id"] == notification.to_string());
    ensure!(
        found == present,
        "actual contextual inbox did not match persistent delivery"
    );
    Ok(())
}

async fn ready(client: &Client, origin: &str, child: &mut tokio::process::Child) -> Result<()> {
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
                .is_ok_and(|response| response.status().is_success())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .map_err(|_| anyhow!("independent work-orders readiness deadline exceeded"))?
}

async fn stop(child: &mut tokio::process::Child) -> bool {
    match child.try_wait() {
        Ok(Some(_)) => true,
        _ => {
            let _ = child.start_kill();
            tokio::time::timeout(Duration::from_secs(5), child.wait())
                .await
                .is_ok_and(|result| result.is_ok())
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires managed explicit disposable PostgreSQL and built independent WORK_ORDERS_BINARY"]
async fn real_platform_reminder_publication_and_recovery() -> Result<()> {
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
        "reminder live verification missed mandatory checks"
    );
    println!(
        "WORK_ORDERS_REMINDER_LIVE_RESULT {}",
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
            ready(&client, &origin, &mut child).await?;
            assertions(
                &client,
                &platform_origin,
                &origin,
                &fixture,
                DatabasePools {
                    platform: pool,
                    business: &business.pool,
                },
                BusinessRuntime {
                    command: &mut command,
                    child: &mut child,
                },
                checks,
            )
            .await
        }
        .await;
        let process_stopped = stop(&mut child).await;
        println!("REMINDER_LIVE_PROCESS_CLEANUP stopped={process_stopped}");
        result?;
        ensure!(process_stopped, "owned business process cleanup failed");
        Ok(())
    }
    .await;
    let _ = stop_tx.send(());
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

struct DatabasePools<'a> {
    platform: &'a PgPool,
    business: &'a PgPool,
}

struct BusinessRuntime<'a> {
    command: &'a mut tokio::process::Command,
    child: &'a mut tokio::process::Child,
}

async fn assertions(
    client: &Client,
    platform: &str,
    business: &str,
    f: &Fixture,
    pools: DatabasePools<'_>,
    runtime: BusinessRuntime<'_>,
    checks: &mut Vec<Value>,
) -> Result<()> {
    let root = login(client, platform, "admin", "admin123", f.admin_org).await?;
    ensure!(
        root["platformContext"] == true,
        "real platform root context missing"
    );
    let admin = text_field(&root, "accessToken")?;
    let selected = login(client, platform, USERNAME, PASSWORD, f.first.org).await?;
    ensure!(
        selected["platformContext"] == false,
        "ordinary publisher received root context"
    );
    let owner_session = text_field(&selected, "accessToken")?;
    let token = issue(client, platform, &owner_session).await?;
    let selected_child = login(client, platform, USERNAME, PASSWORD, f.second.org).await?;
    let child_token = issue(
        client,
        platform,
        &text_field(&selected_child, "accessToken")?,
    )
    .await?;
    let upsert = ACTIONS
        .iter()
        .map(|action| ("ticket", *action, "SELF"))
        .chain(std::iter::once(("notification", "publish", "ORG")))
        .collect::<Vec<_>>();
    for member in [&f.first, &f.second] {
        grants(
            client,
            platform,
            &admin,
            member,
            f.application,
            &upsert,
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
        .map_err(|_| anyhow!("real reminder confidential introspection failed"))?;
    ensure!(
        introspection.status().as_u16() == 200,
        "real reminder introspection status mismatch"
    );
    let intro: Value = serde_json::from_slice(
        &introspection
            .bytes()
            .await
            .map_err(|_| anyhow!("introspection body missing"))?,
    )?;
    ensure!(
        intro["active"] == true && intro["aud"] == CODE && intro["sub"] == f.first.user.to_string(),
        "real reminder token identity mismatch"
    );
    pass(checks, CHECKS[0])?;

    let ticket = create_ticket(
        client,
        business,
        &token,
        "phase20-assigned",
        Some(f.candidate),
    )
    .await?;
    let ticket_id = id(&ticket)?;
    let before = ticket_snapshot(pools.business, ticket_id).await?;
    let original = remind(client, business, &token, ticket_id, "phase20-event").await?;
    original.require(200)?;
    let original = original.json()?;
    ensure!(
        original["duplicate"] == false,
        "first publication was incorrectly duplicate"
    );
    let notification = persisted(
        pools.platform,
        f,
        &ticket,
        "phase20-event",
        f.candidate,
        &original,
    )
    .await?;
    let candidate_session = text_field(
        &login(client, platform, "phase20-candidate", PASSWORD, f.first.org).await?,
        "accessToken",
    )?;
    inbox_contains(client, platform, &candidate_session, notification, true).await?;
    inbox_contains(client, platform, &owner_session, notification, false).await?;
    ensure!(
        before == ticket_snapshot(pools.business, ticket_id).await?,
        "reminder changed work-order version, timestamps or body"
    );
    pass(checks, CHECKS[1])?;

    let (same_a, same_b) = tokio::join!(
        remind(client, business, &token, ticket_id, "phase20-event"),
        remind(client, business, &token, ticket_id, "phase20-event")
    );
    for response in [same_a?, same_b?] {
        response.require(200)?;
        let response = response.json()?;
        ensure!(
            response["duplicate"] == true
                && response["publicationId"] == original["publicationId"]
                && response["recipientCount"] == original["recipientCount"]
                && response["deliveryCount"] == original["deliveryCount"]
                && response["createdAt"] == original["createdAt"],
            "same event did not recover immutable actual receipt"
        );
    }
    ensure!(
        counts(pools.platform, "phase20-event").await? == (1, 1)
            && before == ticket_snapshot(pools.business, ticket_id).await?,
        "concurrent same-event retry duplicated notification or changed ticket"
    );
    let other = create_ticket(client, business, &token, "phase20-other-ticket", None).await?;
    let calls = f.gate.calls();
    require_code(
        &remind(client, business, &token, id(&other)?, "phase20-event").await?,
        409,
        "IDEMPOTENCY_CONFLICT",
    )?;
    ensure!(
        f.gate.calls() == calls,
        "cross-ticket reused key reached platform"
    );
    pass(checks, CHECKS[2])?;

    grants(
        client,
        platform,
        &admin,
        &f.first,
        f.application,
        &[("notification", "publish", "SELF")],
        &[],
    )
    .await?;
    let fallback = remind(
        client,
        business,
        &token,
        id(&other)?,
        "phase20-owner-fallback",
    )
    .await?;
    fallback.require(200)?;
    let fallback_notification = persisted(
        pools.platform,
        f,
        &other,
        "phase20-owner-fallback",
        f.first.user,
        &fallback.json()?,
    )
    .await?;
    inbox_contains(
        client,
        platform,
        &owner_session,
        fallback_notification,
        true,
    )
    .await?;
    let calls = f.gate.calls();
    require_code(
        &remind(
            client,
            business,
            &token,
            ticket_id,
            "phase20-self-other-deny",
        )
        .await?,
        403,
        "DATA_DENIED",
    )?;
    ensure!(
        f.gate.calls() == calls
            && counts(pools.platform, "phase20-self-other-deny").await? == (0, 0),
        "SELF publication expanded to another recipient"
    );
    grants(
        client,
        platform,
        &admin,
        &f.first,
        f.application,
        &[("notification", "publish", "ORG")],
        &[],
    )
    .await?;
    pass(checks, CHECKS[3])?;

    for (permission, check, key) in [
        ("ticket:read", CHECKS[4], "phase20-no-read-operation"),
        (
            "notification:publish",
            CHECKS[6],
            "phase20-no-publish-operation",
        ),
    ] {
        permissions(client, platform, &admin, f, Some(permission)).await?;
        let calls = f.gate.calls();
        require_code(
            &remind(client, business, &token, ticket_id, key).await?,
            403,
            "OPERATION_DENIED",
        )?;
        // A historical receipt cannot bypass current operation permission.
        require_code(
            &remind(client, business, &token, ticket_id, "phase20-event").await?,
            403,
            "OPERATION_DENIED",
        )?;
        ensure!(
            f.gate.calls() == calls && counts(pools.platform, key).await? == (0, 0),
            "missing operation reached platform or persisted a publication"
        );
        permissions(client, platform, &admin, f, None).await?;
        pass(checks, check)?;
    }
    for (resource, action, check, key, scope) in [
        ("ticket", "read", CHECKS[5], "phase20-no-read-grant", "SELF"),
        (
            "notification",
            "publish",
            CHECKS[7],
            "phase20-no-publish-grant",
            "ORG",
        ),
    ] {
        grants(
            client,
            platform,
            &admin,
            &f.first,
            f.application,
            &[],
            &[(resource, action)],
        )
        .await?;
        let calls = f.gate.calls();
        require_code(
            &remind(client, business, &token, ticket_id, key).await?,
            403,
            "DATA_DENIED",
        )?;
        require_code(
            &remind(client, business, &token, ticket_id, "phase20-event").await?,
            403,
            "DATA_DENIED",
        )?;
        ensure!(
            f.gate.calls() == calls && counts(pools.platform, key).await? == (0, 0),
            "missing direct grant reached platform or persisted a publication"
        );
        grants(
            client,
            platform,
            &admin,
            &f.first,
            f.application,
            &[(resource, action, scope)],
            &[],
        )
        .await?;
        pass(checks, check)?;
    }

    // The selected org may read its own descendant record, but an ORG publish
    // grant cannot be expanded merely because read access was sufficient.
    let child_ticket =
        create_ticket(client, business, &child_token, "phase20-child-ticket", None).await?;
    grants(
        client,
        platform,
        &admin,
        &f.first,
        f.application,
        &[("ticket", "read", "TENANT")],
        &[],
    )
    .await?;
    let calls = f.gate.calls();
    http(
        client,
        business,
        &format!("{TICKETS}/{}", id(&child_ticket)?),
        &token,
        Method::GET,
        None,
        None,
    )
    .await?
    .require(200)?;
    require_code(
        &remind(
            client,
            business,
            &token,
            id(&child_ticket)?,
            "phase20-outside-publish-org",
        )
        .await?,
        403,
        "DATA_DENIED",
    )?;
    ensure!(
        f.gate.calls() == calls
            && counts(pools.platform, "phase20-outside-publish-org").await? == (0, 0),
        "read grant expanded notification organization scope"
    );
    grants(
        client,
        platform,
        &admin,
        &f.first,
        f.application,
        &[("ticket", "read", "SELF")],
        &[],
    )
    .await?;
    candidate_status(client, platform, &admin, f, "DISABLED").await?;
    require_code(
        &remind(
            client,
            business,
            &token,
            ticket_id,
            "phase20-disabled-recipient",
        )
        .await?,
        404,
        "RECIPIENT_NOT_FOUND",
    )?;
    ensure!(
        counts(pools.platform, "phase20-disabled-recipient").await? == (0, 0),
        "disabled recipient received new notification"
    );
    // History recovery does not reinterpret an earlier valid recipient.
    let retained = remind(client, business, &token, ticket_id, "phase20-event").await?;
    retained.require(200)?;
    ensure!(
        retained.json()?["publicationId"] == original["publicationId"]
            && retained.json()?["duplicate"] == true,
        "disabled recipient blocked historical receipt recovery"
    );
    candidate_status(client, platform, &admin, f, "ENABLED").await?;
    candidate_membership(client, platform, &admin, f, false).await?;
    require_code(
        &remind(
            client,
            business,
            &token,
            ticket_id,
            "phase20-removed-recipient",
        )
        .await?,
        404,
        "RECIPIENT_NOT_FOUND",
    )?;
    let retained = remind(client, business, &token, ticket_id, "phase20-event").await?;
    retained.require(200)?;
    ensure!(
        retained.json()?["publicationId"] == original["publicationId"]
            && retained.json()?["duplicate"] == true
            && counts(pools.platform, "phase20-removed-recipient").await? == (0, 0),
        "removed membership rewrote history or allowed a new delivery"
    );
    candidate_membership(client, platform, &admin, f, true).await?;
    ensure!(
        before == ticket_snapshot(pools.business, ticket_id).await?,
        "recipient rejection or historical recovery wrote ticket state"
    );
    pass(checks, CHECKS[8])?;

    // The original upstream handler has not executed when the business request
    // becomes unknown. A durable pre-send event must survive the process exit.
    let pending_ticket = create_ticket(
        client,
        business,
        &token,
        "phase20-pending-ticket",
        Some(f.candidate),
    )
    .await?;
    let pending_id = id(&pending_ticket)?;
    let pending_before = ticket_snapshot(pools.business, pending_id).await?;
    let pause = f.gate.arm(PauseAt::Before)?;
    let pending = pending_remind(
        client,
        business,
        &token,
        pending_id,
        "phase20-precommit-unknown",
    );
    pause.observed().await?;
    require_code(&pending.await??, 503, "REMINDER_RESULT_UNKNOWN")?;
    ensure!(
        counts(pools.platform, "phase20-precommit-unknown").await? == (0, 0)
            && pending_before == ticket_snapshot(pools.business, pending_id).await?,
        "precommit unknown unexpectedly committed platform message or wrote ticket"
    );
    // Once released, the genuine handler sees a formally disabled recipient and
    // returns 404; this leaves no latent successful original publication.
    candidate_status(client, platform, &admin, f, "DISABLED").await?;
    pause.release();
    pause.completed(404).await?;
    candidate_status(client, platform, &admin, f, "ENABLED").await?;
    ensure!(
        counts(pools.platform, "phase20-precommit-unknown").await? == (0, 0),
        "delayed rejected handler created a publication"
    );
    let stopped = stop(runtime.child).await;
    println!("REMINDER_LIVE_PROCESS_RESTART stopped={stopped}");
    ensure!(
        stopped,
        "owned business process did not stop for durable-intent restart"
    );
    *runtime.child = runtime
        .command
        .spawn()
        .map_err(|_| anyhow!("independent business process restart failed"))?;
    ready(client, business, runtime.child).await?;
    let changed = update_assignee(client, business, &token, &pending_ticket, f.peer.user).await?;
    let changed_before = ticket_snapshot(pools.business, pending_id).await?;
    let calls = f.gate.calls();
    require_code(
        &remind(
            client,
            business,
            &token,
            pending_id,
            "phase20-precommit-unknown",
        )
        .await?,
        409,
        "IDEMPOTENCY_CONFLICT",
    )?;
    ensure!(
        f.gate.calls() == calls
            && counts(pools.platform, "phase20-precommit-unknown").await? == (0, 0),
        "restart lost intent and silently retargeted original event"
    );
    let new_event = remind(
        client,
        business,
        &token,
        pending_id,
        "phase20-explicit-new-event",
    )
    .await?;
    new_event.require(200)?;
    let changed_notification = persisted(
        pools.platform,
        f,
        &changed,
        "phase20-explicit-new-event",
        f.peer.user,
        &new_event.json()?,
    )
    .await?;
    let peer_session = text_field(
        &login(client, platform, PEER, PASSWORD, f.first.org).await?,
        "accessToken",
    )?;
    inbox_contains(client, platform, &peer_session, changed_notification, true).await?;
    inbox_contains(
        client,
        platform,
        &owner_session,
        changed_notification,
        false,
    )
    .await?;
    ensure!(
        changed_before == ticket_snapshot(pools.business, pending_id).await?,
        "new reminder changed explicitly updated ticket version"
    );
    // A successful event also cannot be reused after its recipient changes.
    let changed_original = update_assignee(client, business, &token, &ticket, f.peer.user).await?;
    require_code(
        &remind(client, business, &token, ticket_id, "phase20-event").await?,
        409,
        "IDEMPOTENCY_CONFLICT",
    )?;
    let explicit = remind(
        client,
        business,
        &token,
        ticket_id,
        "phase20-changed-success-new-event",
    )
    .await?;
    explicit.require(200)?;
    persisted(
        pools.platform,
        f,
        &changed_original,
        "phase20-changed-success-new-event",
        f.peer.user,
        &explicit.json()?,
    )
    .await?;
    pass(checks, CHECKS[9])?;

    let lost_ticket = create_ticket(
        client,
        business,
        &token,
        "phase20-lost-response-ticket",
        Some(f.candidate),
    )
    .await?;
    let lost_id = id(&lost_ticket)?;
    let lost_before = ticket_snapshot(pools.business, lost_id).await?;
    let pause = f.gate.arm(PauseAt::After)?;
    let pending = pending_remind(
        client,
        business,
        &token,
        lost_id,
        "phase20-committed-lost-response",
    );
    pause.observed().await?;
    ensure!(
        pause.0.status.load(Ordering::SeqCst) == 200
            && counts(pools.platform, "phase20-committed-lost-response").await? == (1, 1),
        "response-loss fault did not follow actual durable platform success"
    );
    require_code(&pending.await??, 503, "REMINDER_RESULT_UNKNOWN")?;
    ensure!(
        counts(pools.platform, "phase20-committed-lost-response").await? == (1, 1),
        "unknown response automatically retried with a new event key"
    );
    pause.release();
    pause.completed(200).await?;
    let recovered = remind(
        client,
        business,
        &token,
        lost_id,
        "phase20-committed-lost-response",
    )
    .await?;
    recovered.require(200)?;
    let recovered = recovered.json()?;
    ensure!(
        recovered["duplicate"] == true,
        "lost committed response was not recovered as duplicate"
    );
    let lost_notification = persisted(
        pools.platform,
        f,
        &lost_ticket,
        "phase20-committed-lost-response",
        f.candidate,
        &recovered,
    )
    .await?;
    ensure!(
        lost_before == ticket_snapshot(pools.business, lost_id).await?,
        "committed unknown/recovery wrote ticket state"
    );
    pass(checks, CHECKS[10])?;

    // This harness never starts a realtime process, socket, or dispatcher. The
    // actual platform inbox still contains the committed APP notification.
    let candidate_session = text_field(
        &login(client, platform, "phase20-candidate", PASSWORD, f.first.org).await?,
        "accessToken",
    )?;
    inbox_contains(
        client,
        platform,
        &candidate_session,
        lost_notification,
        true,
    )
    .await?;
    http(
        client,
        platform,
        &format!("/api/notifications/{lost_notification}"),
        &candidate_session,
        Method::DELETE,
        None,
        None,
    )
    .await?
    .require(200)?;
    inbox_contains(
        client,
        platform,
        &candidate_session,
        lost_notification,
        false,
    )
    .await?;
    let retained = remind(
        client,
        business,
        &token,
        lost_id,
        "phase20-committed-lost-response",
    )
    .await?;
    retained.require(200)?;
    ensure!(
        retained.json()?["publicationId"] == recovered["publicationId"]
            && retained.json()?["duplicate"] == true
            && counts(pools.platform, "phase20-committed-lost-response").await? == (1, 0),
        "deleted inbox message was recreated to compensate for absent realtime"
    );
    ensure!(
        lost_before == ticket_snapshot(pools.business, lost_id).await?,
        "inbox deletion or receipt recovery wrote ticket version"
    );
    println!("REMINDER_LIVE_REALTIME service_started=false durable_inbox_verified=true deleted_delivery_not_recreated=true");
    pass(checks, CHECKS[11])?;
    Ok(())
}
