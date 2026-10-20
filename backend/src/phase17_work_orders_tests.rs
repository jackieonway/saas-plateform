//! Actual platform HTTP -> independent work-orders Rust process -> owned PG.
//! Loopback HTTP is an explicit development fixture, not production TLS evidence.
use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;
use std::process::Stdio;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use axum::Router;
use reqwest::{Client, Method};
use serde_json::{json, Value};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use tokio::net::TcpListener;
use uuid::Uuid;

use crate::config::Config;
use crate::entity_creation_tests::Database;
use crate::state::AppState;
use crate::util::{hash, jwt};

const SECRET: &str = "owned-phase17-session-signing-only";
const CLIENT_SECRET: &str = "owned-phase17-confidential-client-secret";
const CLIENT_ID: &str = "phase17-work-orders-client";
const CODE: &str = "work-orders";
const CHECKS: [&str; 13] = [
    "real-confidential-introspection",
    "missing-grant-denied",
    "self-scope-list-count-detail-export",
    "org-scope-list-count-detail-export",
    "subtree-scope-list-count-detail-export",
    "nonroot-tenant-platform-downgrade",
    "platform-root-tenant-scope",
    "wrong-audience-denied",
    "grant-removal-next-request",
    "logout-next-request",
    "membership-removal-next-request",
    "platform-unreachable-fail-closed",
    "loopback-http-explicit-development",
];

fn config(policy: &Value) -> Result<Config> {
    Config::from_values(&HashMap::from([
        ("APPLICATION_ENVIRONMENT".into(), "development".into()),
        (
            "DATABASE_URL".into(),
            "postgres://unused/phase17-owned".into(),
        ),
        ("JWT_SECRET".into(), SECRET.into()),
        ("DATA_GRANT_POLICY".into(), policy.to_string()),
    ]))
}

#[derive(Clone)]
struct Member {
    tenant: Uuid,
    org: Uuid,
    user: Uuid,
    membership: Uuid,
    session: String,
}

impl Member {
    fn target(&self, application: Uuid) -> Value {
        json!({"tenantId":self.tenant,"organizationId":self.org,"userId":self.user,
            "membershipId":self.membership,"applicationId":application})
    }
}

async fn session(pool: &PgPool, user: Uuid, tenant: Uuid, org: Uuid) -> Result<String> {
    let version = sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
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
            ver: Some(version),
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

async fn member(
    pool: &PgPool,
    name: &str,
    tenant: Uuid,
    org: Uuid,
    menus: &[Uuid],
) -> Result<Member> {
    let user: Uuid = sqlx::query_scalar("INSERT INTO users(username,real_name,password_hash) VALUES($1,$1,'unused-phase17-owned-password') RETURNING id")
        .bind(name).fetch_one(pool).await?;
    let membership: Uuid = sqlx::query_scalar(
        "INSERT INTO memberships(user_id,organization_id) VALUES($1,$2) RETURNING id",
    )
    .bind(user)
    .bind(org)
    .fetch_one(pool)
    .await?;
    let role: Uuid = sqlx::query_scalar(
        "INSERT INTO roles(tenant_id,organization_id,name) VALUES($1,$2,$3) RETURNING id",
    )
    .bind(tenant)
    .bind(org)
    .bind(name)
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
    Ok(Member {
        tenant,
        org,
        user,
        membership,
        session: session(pool, user, tenant, org).await?,
    })
}

struct Fixture {
    app: Router,
    application: Uuid,
    administrator: String,
    ordinary: Member,
    platform_root: Member,
    business_root: Uuid,
    child: Uuid,
    sibling: Uuid,
    other_tenant: Uuid,
    other_org: Uuid,
    platform_child: Uuid,
    other_application: Uuid,
}

impl Fixture {
    async fn create(pool: &PgPool) -> Result<Self> {
        crate::seed::initialize(
            pool,
            &config(&json!({"applications":[],"managementScopes":[]}))?,
        )
        .await?;
        let (platform, root, administrator): (Uuid, Uuid, Uuid) = sqlx::query_as(
            "SELECT t.id,o.id,u.id FROM tenants t JOIN organizations o ON o.tenant_id=t.id AND o.parent_id IS NULL JOIN users u ON u.username='admin' WHERE t.is_platform")
            .fetch_one(pool).await?;
        let tenant: Uuid = sqlx::query_scalar("INSERT INTO tenants(name,code) VALUES('Owned Phase17 business','phase17-business') RETURNING id")
            .fetch_one(pool).await?;
        let other_tenant: Uuid = sqlx::query_scalar("INSERT INTO tenants(name,code) VALUES('Owned Phase17 other tenant','phase17-other') RETURNING id")
            .fetch_one(pool).await?;
        let business_root = org(pool, tenant, None, "Phase17 business root").await?;
        let current = org(pool, tenant, Some(business_root), "Phase17 current").await?;
        let child = org(pool, tenant, Some(current), "Phase17 child").await?;
        let sibling = org(pool, tenant, Some(business_root), "Phase17 sibling").await?;
        let other_org = org(pool, other_tenant, None, "Phase17 other tenant root").await?;
        let platform_child = org(pool, platform, Some(root), "Phase17 platform child").await?;
        let application: Uuid = sqlx::query_scalar("INSERT INTO applications(name,code,base_path,entry_type) VALUES('Owned Phase17 Work Orders',$1,'/micro/work-orders','HTML') RETURNING id")
            .bind(CODE).fetch_one(pool).await?;
        let other_application: Uuid = sqlx::query_scalar("INSERT INTO applications(name,code,base_path,entry_type) VALUES('Owned Phase17 wrong audience','phase17-other-app','/micro/phase17-other','HTML') RETURNING id")
            .fetch_one(pool).await?;
        let mut menus = Vec::new();
        for action in ["read", "export"] {
            let menu: Uuid = sqlx::query_scalar("INSERT INTO menus(application_id,title,type,permission,visible) VALUES($1,$2,'BUTTON',$3,false) RETURNING id")
                .bind(application).bind(format!("Phase17 {action}")).bind(format!("ticket:{action}")).fetch_one(pool).await?;
            menus.push(menu);
        }
        for (t, organizations) in [
            (tenant, vec![business_root, current, child, sibling]),
            (platform, vec![root, platform_child]),
        ] {
            for app in [application, other_application] {
                sqlx::query(
                    "INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)",
                )
                .bind(t)
                .bind(app)
                .execute(pool)
                .await?;
                for organization in &organizations {
                    sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)")
                        .bind(organization).bind(app).execute(pool).await?;
                }
            }
            for menu in &menus {
                sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
                    .bind(t)
                    .bind(menu)
                    .execute(pool)
                    .await?;
                for organization in &organizations {
                    sqlx::query(
                        "INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)",
                    )
                    .bind(organization)
                    .bind(menu)
                    .execute(pool)
                    .await?;
                }
            }
        }
        for (client, app) in [
            (CLIENT_ID, application),
            ("phase17-other-client", other_application),
        ] {
            sqlx::query("INSERT INTO oauth_clients(client_id,client_name,client_secret_hash,application_id,client_type,grant_types,scopes) VALUES($1,'Owned Phase17 confidential client',$2,$3,'CONFIDENTIAL','password,refresh_token','profile')")
                .bind(client).bind(hash::sha256_hex(CLIENT_SECRET)).bind(app).execute(pool).await?;
        }
        let ordinary = member(pool, "phase17-ordinary-member", tenant, current, &menus).await?;
        let platform_root = member(
            pool,
            "phase17-ordinary-platform-root",
            platform,
            root,
            &menus,
        )
        .await?;
        let policy = json!({"applications":[{"applicationId":application,"capabilities":[
            {"resource":"ticket","action":"read","scopes":["SELF","ORG","ORG_SUBTREE","TENANT"]},
            {"resource":"ticket","action":"export","scopes":["SELF","ORG","ORG_SUBTREE","TENANT"]}]}],
            "managementScopes":[
                {"tenantId":tenant,"organizationId":business_root,"includeDescendants":true,"read":true,"write":true},
                {"tenantId":platform,"organizationId":root,"includeDescendants":true,"read":true,"write":true}]});
        let state = AppState::new(pool.clone(), Arc::new(config(&policy)?));
        let app = crate::routes::api_router()
            .nest("/oauth", crate::routes::oauth2::oauth_router())
            .with_state(state);
        Ok(Self {
            app,
            application,
            administrator: session(pool, administrator, platform, root).await?,
            ordinary,
            platform_root,
            business_root,
            child,
            sibling,
            other_tenant,
            other_org,
            platform_child,
            other_application,
        })
    }
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

struct BusinessDatabase {
    pool: PgPool,
    admin: PgPool,
    name: String,
    url: String,
}

impl BusinessDatabase {
    async fn create() -> Result<Self> {
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
        let name = format!("phase17_work_orders_{}", Uuid::new_v4().simple());
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
        })
    }

    async fn finish(self, assertions: Result<()>) -> Result<()> {
        ensure!(
            self.name.starts_with("phase17_work_orders_")
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
            "WORK_ORDERS_LIVE_BUSINESS_CLEANUP assertions_ok={} database_removed={} pool_closed={}",
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
        serde_json::from_slice(&self.bytes)
            .map_err(|_| anyhow!("fixture HTTP response was not JSON"))
    }
    fn require(&self, expected: u16) -> Result<()> {
        ensure!(
            self.status == expected,
            "fixture HTTP response status {} expected {}",
            self.status,
            expected
        );
        Ok(())
    }
}

async fn http(
    client: &Client,
    origin: &str,
    path: &str,
    token: &str,
    method: Method,
    body: Option<Value>,
) -> Result<Reply> {
    let request_id = Uuid::new_v4().to_string();
    let mut request = client
        .request(method, format!("{origin}{path}"))
        .header("x-request-id", &request_id);
    if !token.is_empty() {
        request = request.bearer_auth(token);
    }
    if let Some(value) = body {
        request = request
            .header("content-type", "application/json")
            .body(serde_json::to_vec(&value)?);
    }
    let response = request
        .send()
        .await
        .map_err(|_| anyhow!("fixture HTTP request failed"))?;
    let status = response.status().as_u16();
    // Business responses must correlate the original request, including errors.
    if path.starts_with("/work-orders-api/tickets") {
        ensure!(
            response
                .headers()
                .get("x-request-id")
                .and_then(|v| v.to_str().ok())
                == Some(request_id.as_str()),
            "business response lost requestId"
        );
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| anyhow!("fixture HTTP body read failed"))?;
    ensure!(
        bytes.len() <= 1024 * 1024,
        "fixture HTTP response exceeded budget"
    );
    if path.starts_with("/work-orders-api/") {
        for secret in [CLIENT_SECRET, SECRET] {
            ensure!(
                !bytes.windows(secret.len()).any(|v| v == secret.as_bytes()),
                "business response exposed credential material"
            );
        }
    }
    Ok(Reply {
        status,
        bytes: bytes.to_vec(),
    })
}

async fn issue(client: &Client, origin: &str, session: &str, code: &str) -> Result<String> {
    let response = http(
        client,
        origin,
        "/api/oauth/session-token",
        session,
        Method::POST,
        Some(json!({"appCode":code})),
    )
    .await?;
    response.require(201)?;
    response.json()?["accessToken"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("platform did not issue an app token"))
}

async fn set_scope(
    client: &Client,
    origin: &str,
    f: &Fixture,
    m: &Member,
    scope: Option<&str>,
) -> Result<Value> {
    let target = m.target(f.application);
    let query = target
        .as_object()
        .ok_or_else(|| anyhow!("invalid owned target"))?
        .iter()
        .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or_default()))
        .collect::<Vec<_>>()
        .join("&");
    let response = http(
        client,
        origin,
        &format!("/api/data-grants?{query}"),
        &f.administrator,
        Method::GET,
        None,
    )
    .await?;
    response.require(200)?;
    let snapshot = response.json()?;
    let (upsert, remove) = match scope {
        Some(scope) => (
            json!([{"resource":"ticket","action":"read","scope":scope},{"resource":"ticket","action":"export","scope":scope}]),
            json!([]),
        ),
        None => (
            json!([]),
            json!([{"resource":"ticket","action":"read"},{"resource":"ticket","action":"export"}]),
        ),
    };
    let change = json!({"target":target,"expectedVersion":snapshot["version"],"upsert":upsert,"remove":remove});
    http(
        client,
        origin,
        "/api/data-grants/preview",
        &f.administrator,
        Method::POST,
        Some(change.clone()),
    )
    .await?
    .require(200)?;
    let after_preview = http(
        client,
        origin,
        &format!("/api/data-grants?{query}"),
        &f.administrator,
        Method::GET,
        None,
    )
    .await?;
    after_preview.require(200)?;
    ensure!(
        after_preview.json()? == snapshot,
        "formal grant preview mutated storage"
    );
    let saved = http(
        client,
        origin,
        "/api/data-grants",
        &f.administrator,
        Method::PATCH,
        Some(change),
    )
    .await?;
    saved.require(200)?;
    saved.json()
}

struct Ticket {
    id: Uuid,
    number: String,
    tenant: Uuid,
    org: Uuid,
    owner: Uuid,
    deleted: bool,
}

async fn tickets(pool: &PgPool, f: &Fixture) -> Result<Vec<Ticket>> {
    let ordinary = &f.ordinary;
    let peer = Uuid::new_v4();
    let rows = [
        (ordinary.tenant, ordinary.org, ordinary.user, false),
        (ordinary.tenant, ordinary.org, peer, false),
        (ordinary.tenant, f.child, ordinary.user, false),
        (ordinary.tenant, f.child, peer, false),
        (ordinary.tenant, f.business_root, ordinary.user, false),
        (ordinary.tenant, f.sibling, ordinary.user, false),
        (f.other_tenant, f.other_org, ordinary.user, false),
        (ordinary.tenant, ordinary.org, ordinary.user, true),
        (
            f.platform_root.tenant,
            f.platform_root.org,
            f.platform_root.user,
            false,
        ),
        (
            f.platform_root.tenant,
            f.platform_child,
            f.platform_root.user,
            false,
        ),
    ];
    let mut result = Vec::new();
    for (index, (tenant, org, owner, deleted)) in rows.into_iter().enumerate() {
        let ticket = Ticket {
            id: Uuid::new_v4(),
            number: format!("P17-{index:03}"),
            tenant,
            org,
            owner,
            deleted,
        };
        sqlx::query("INSERT INTO work_orders(id,number,tenant_id,organization_id,organization_name,owner_user_id,owner_display_name,title,description,deleted_at) VALUES($1,$2,$3,$4,'阶段17组织',$5,'阶段17成员',$6,'真实平台与独立业务库',CASE WHEN $7 THEN now() ELSE NULL END)")
            .bind(ticket.id).bind(&ticket.number).bind(tenant).bind(org).bind(owner)
            .bind(if index==0 {"=阶段17公式"} else {"阶段17工单"}).bind(deleted).execute(pool).await?;
        result.push(ticket);
    }
    Ok(result)
}

async fn assert_scope(
    client: &Client,
    origin: &str,
    token: &str,
    all: &[Ticket],
    expected: &BTreeSet<Uuid>,
) -> Result<()> {
    let response = http(
        client,
        origin,
        "/work-orders-api/tickets?size=100",
        token,
        Method::GET,
        None,
    )
    .await?;
    response.require(200)?;
    let list = response.json()?;
    let records = list["records"]
        .as_array()
        .ok_or_else(|| anyhow!("business records missing"))?;
    let ids = records
        .iter()
        .map(|v| {
            v["id"]
                .as_str()
                .ok_or_else(|| anyhow!("business record id missing"))?
                .parse::<Uuid>()
                .map_err(|_| anyhow!("business record id invalid"))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    ensure!(
        &ids == expected && list["total"].as_u64() == Some(expected.len() as u64),
        "list/count escaped the expected data scope"
    );
    let mut paged = BTreeSet::new();
    for page in 1..=expected.len() + 1 {
        let reply = http(
            client,
            origin,
            &format!("/work-orders-api/tickets?page={page}&size=1"),
            token,
            Method::GET,
            None,
        )
        .await?;
        reply.require(200)?;
        let value = reply.json()?;
        ensure!(
            value["total"].as_u64() == Some(expected.len() as u64),
            "pagination changed scope count"
        );
        for row in value["records"]
            .as_array()
            .ok_or_else(|| anyhow!("paged records missing"))?
        {
            let id = row["id"]
                .as_str()
                .ok_or_else(|| anyhow!("paged id missing"))?
                .parse::<Uuid>()?;
            ensure!(paged.insert(id), "pagination duplicated a row");
        }
    }
    ensure!(&paged == expected, "pagination did not preserve scope");
    for ticket in all {
        let detail = http(
            client,
            origin,
            &format!("/work-orders-api/tickets/{}", ticket.id),
            token,
            Method::GET,
            None,
        )
        .await?;
        detail.require(if expected.contains(&ticket.id) {
            200
        } else {
            404
        })?;
        let filtered = http(
            client,
            origin,
            &format!("/work-orders-api/tickets?q={}", ticket.number),
            token,
            Method::GET,
            None,
        )
        .await?;
        filtered.require(200)?;
        ensure!(
            filtered.json()?["total"].as_u64() == Some(u64::from(expected.contains(&ticket.id))),
            "parameterized filter escaped data scope"
        );
    }
    let export = http(
        client,
        origin,
        "/work-orders-api/tickets/export.csv",
        token,
        Method::GET,
        None,
    )
    .await?;
    export.require(200)?;
    ensure!(
        export.bytes.starts_with(&[0xef, 0xbb, 0xbf]),
        "CSV omitted UTF-8 BOM"
    );
    let csv = std::str::from_utf8(&export.bytes).map_err(|_| anyhow!("CSV is not UTF-8"))?;
    for ticket in all {
        ensure!(
            csv.contains(&format!("\"{}\"", ticket.number)) == expected.contains(&ticket.id),
            "CSV/list/detail scope mismatch"
        );
    }
    ensure!(
        csv.lines().count() == expected.len() + 1,
        "CSV contained extra rows"
    );
    if expected.contains(&all[0].id) {
        ensure!(
            csv.contains("\"'=阶段17公式\""),
            "CSV formula protection was lost"
        );
    }
    Ok(())
}

fn pass(checks: &mut Vec<Value>, id: &str) -> Result<()> {
    ensure!(
        CHECKS.contains(&id) && !checks.iter().any(|v| v["id"] == id),
        "unknown or duplicate joint verification check"
    );
    checks.push(json!({"id":id,"status":"PASS"}));
    println!("LIVE_WORK_ORDERS_{id}=PASS");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires managed explicit disposable PostgreSQL and built independent WORK_ORDERS_BINARY"]
async fn real_platform_authorization_and_business_scope() -> Result<()> {
    ensure!(
        std::env::var("TEST_DATABASE_URL").is_ok_and(|v| !v.trim().is_empty()),
        "explicit disposable TEST_DATABASE_URL required; dotenv is not read"
    );
    // Validate the managed endpoint before the historical platform helper can
    // create anything. No business schema is installed in the platform database.
    let business = BusinessDatabase::create().await?;
    let mut checks = Vec::new();
    let platform = match Database::create().await {
        Ok(database) => database,
        Err(error) => return business.finish(Err(error)).await,
    };
    let result = joint(&platform.pool, &business, &mut checks).await;
    let result = business.finish(result).await;
    let result = platform.finish(result).await;
    println!(
        "WORK_ORDERS_LIVE_PLATFORM_CLEANUP assertions_ok={} database_removed={} pool_closed={}",
        result.is_ok(),
        result.is_ok(),
        result.is_ok()
    );
    result?;
    ensure!(
        checks.len() == CHECKS.len(),
        "joint verification did not execute all mandatory checks"
    );
    println!(
        "WORK_ORDERS_LIVE_RESULT {}",
        json!({"schemaVersion":1,"transport":"explicit-development-loopback-http","passed":checks.len(),"failed":0,"skipped":0,"checks":checks})
    );
    Ok(())
}

async fn joint(pool: &PgPool, business: &BusinessDatabase, checks: &mut Vec<Value>) -> Result<()> {
    let f = Fixture::create(pool).await?;
    let platform_listener = TcpListener::bind("127.0.0.1:0").await?;
    let platform_origin = format!("http://{}", platform_listener.local_addr()?);
    // Reserve and release one loopback port immediately before the child bind.
    // Startup checks distinguish an early process exit from failed readiness.
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
        .env("PLATFORM_CLIENT_ID", CLIENT_ID)
        .env("PLATFORM_CLIENT_SECRET", CLIENT_SECRET)
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
    let platform_app = f.app.clone();
    let mut server = tokio::spawn(async move {
        axum::serve(
            platform_listener,
            platform_app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = stop_rx.await;
        })
        .await
    });
    let spawned = command
        .spawn()
        .map_err(|_| anyhow!("independent Rust work-orders process could not start"));
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            if let Some(stop) = stop_tx.take() {
                let _ = stop.send(());
            }
            let _ = tokio::time::timeout(Duration::from_secs(5), &mut server).await;
            server.abort();
            return Err(error);
        }
    };
    let result = async {
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()?;
        tokio::time::timeout(Duration::from_secs(35), async {
            loop {
                ensure!(
                    child.try_wait()?.is_none(),
                    "independent Rust work-orders process exited before ready"
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
        .map_err(|_| anyhow!("independent Rust work-orders readiness deadline exceeded"))??;
        let all = tickets(&business.pool, &f).await?;
        let token = issue(&client, &platform_origin, &f.ordinary.session, CODE).await?;
        // Verify the real introspection caller is confidential and bound to the
        // same registered application. No platform response is mocked here.
        let intro = client
            .post(format!("{platform_origin}/oauth/introspect"))
            .basic_auth(CLIENT_ID, Some(CLIENT_SECRET))
            .form(&[("token", token.as_str())])
            .send()
            .await?;
        ensure!(
            intro.status().as_u16() == 200,
            "real confidential introspection failed"
        );
        let intro: Value = serde_json::from_slice(&intro.bytes().await?)?;
        ensure!(
            intro["active"] == true
                && intro["aud"] == CODE
                && intro["sub"] == f.ordinary.user.to_string(),
            "real app token identity mismatch"
        );
        let rejected = client
            .post(format!("{platform_origin}/oauth/introspect"))
            .basic_auth(CLIENT_ID, Some("incorrect-owned-fixture-secret"))
            .form(&[("token", token.as_str())])
            .send()
            .await?;
        ensure!(
            rejected.status().as_u16() == 401,
            "introspection did not authenticate the confidential client"
        );
        pass(checks, "real-confidential-introspection")?;
        for path in [
            "/work-orders-api/tickets",
            "/work-orders-api/tickets/export.csv",
        ] {
            http(&client, &origin, path, &token, Method::GET, None)
                .await?
                .require(403)?;
        }
        pass(checks, "missing-grant-denied")?;
        for (scope, check) in [
            ("SELF", CHECKS[2]),
            ("ORG", CHECKS[3]),
            ("ORG_SUBTREE", CHECKS[4]),
            ("TENANT", CHECKS[5]),
        ] {
            let saved = set_scope(&client, &platform_origin, &f, &f.ordinary, Some(scope)).await?;
            let expected = all
                .iter()
                .filter(|t| {
                    !t.deleted
                        && t.tenant == f.ordinary.tenant
                        && match scope {
                            "SELF" => {
                                t.owner == f.ordinary.user
                                    && [f.ordinary.org, f.child].contains(&t.org)
                            }
                            "ORG" => t.org == f.ordinary.org,
                            _ => [f.ordinary.org, f.child].contains(&t.org),
                        }
                })
                .map(|t| t.id)
                .collect();
            if scope == "TENANT" {
                ensure!(
                    saved["grants"].as_array().is_some_and(|rows| rows
                        .iter()
                        .all(|g| g["scope"] == "TENANT" && g["effectiveScope"] == "ORG_SUBTREE")),
                    "formal grant API did not downgrade nonroot TENANT"
                );
                let context = http(
                    &client,
                    &platform_origin,
                    "/api/apps/work-orders/context",
                    &token,
                    Method::GET,
                    None,
                )
                .await?;
                context.require(200)?;
                ensure!(
                    context.json()?["dataGrants"]
                        .as_array()
                        .is_some_and(|g| g.iter().all(|g| g["scope"] == "ORG_SUBTREE")),
                    "real platform context did not enforce organization ceiling"
                );
            }
            assert_scope(&client, &origin, &token, &all, &expected).await?;
            pass(checks, check)?;
        }
        set_scope(
            &client,
            &platform_origin,
            &f,
            &f.platform_root,
            Some("TENANT"),
        )
        .await?;
        let root_token = issue(&client, &platform_origin, &f.platform_root.session, CODE).await?;
        let context = http(
            &client,
            &platform_origin,
            "/api/apps/work-orders/context",
            &root_token,
            Method::GET,
            None,
        )
        .await?;
        context.require(200)?;
        ensure!(
            context.json()?["dataGrants"]
                .as_array()
                .is_some_and(|g| g.iter().all(|g| g["scope"] == "TENANT")),
            "platform root TENANT was unexpectedly narrowed"
        );
        let expected = all
            .iter()
            .filter(|t| !t.deleted && t.tenant == f.platform_root.tenant)
            .map(|t| t.id)
            .collect();
        assert_scope(&client, &origin, &root_token, &all, &expected).await?;
        pass(checks, "platform-root-tenant-scope")?;
        let other = issue(
            &client,
            &platform_origin,
            &f.ordinary.session,
            "phase17-other-app",
        )
        .await?;
        http(
            &client,
            &origin,
            "/work-orders-api/tickets",
            &other,
            Method::GET,
            None,
        )
        .await?
        .require(401)?;
        // Exercise the true platform audience endpoint too, proving this token
        // is valid for a different registered application rather than garbage.
        let other_context = http(
            &client,
            &platform_origin,
            "/api/apps/phase17-other-app/context",
            &other,
            Method::GET,
            None,
        )
        .await?;
        other_context.require(200)?;
        ensure!(
            other_context.json()?["application"]["id"] == f.other_application.to_string(),
            "other audience fixture is invalid"
        );
        pass(checks, "wrong-audience-denied")?;
        set_scope(&client, &platform_origin, &f, &f.ordinary, None).await?;
        http(
            &client,
            &origin,
            "/work-orders-api/tickets",
            &token,
            Method::GET,
            None,
        )
        .await?
        .require(403)?;
        http(
            &client,
            &origin,
            "/work-orders-api/tickets/export.csv",
            &token,
            Method::GET,
            None,
        )
        .await?
        .require(403)?;
        pass(checks, "grant-removal-next-request")?;
        set_scope(
            &client,
            &platform_origin,
            &f,
            &f.ordinary,
            Some("ORG_SUBTREE"),
        )
        .await?;
        http(
            &client,
            &origin,
            "/work-orders-api/tickets",
            &token,
            Method::GET,
            None,
        )
        .await?
        .require(200)?;
        http(
            &client,
            &platform_origin,
            "/api/auth/logout",
            &f.ordinary.session,
            Method::POST,
            Some(json!({})),
        )
        .await?
        .require(200)?;
        http(
            &client,
            &origin,
            "/work-orders-api/tickets",
            &token,
            Method::GET,
            None,
        )
        .await?
        .require(401)?;
        pass(checks, "logout-next-request")?;
        // Membership disappearance is an initial-platform-fixture mutation,
        // independent of the controlled grant-change flow verified above.
        sqlx::query("DELETE FROM memberships WHERE id=$1")
            .bind(f.platform_root.membership)
            .execute(pool)
            .await?;
        http(
            &client,
            &origin,
            "/work-orders-api/tickets",
            &root_token,
            Method::GET,
            None,
        )
        .await?
        .require(401)?;
        pass(checks, "membership-removal-next-request")?;
        if let Some(stop) = stop_tx.take() {
            let _ = stop.send(());
        }
        tokio::time::timeout(Duration::from_secs(5), &mut server)
            .await
            .map_err(|_| anyhow!("owned platform HTTP shutdown timed out"))???;
        http(
            &client,
            &origin,
            "/work-orders-api/tickets",
            &token,
            Method::GET,
            None,
        )
        .await?
        .require(503)?;
        pass(checks, "platform-unreachable-fail-closed")?;
        pass(checks, "loopback-http-explicit-development")?;
        Ok(())
    }
    .await;
    if let Some(stop) = stop_tx.take() {
        let _ = stop.send(());
    }
    if !server.is_finished()
        && tokio::time::timeout(Duration::from_secs(5), &mut server)
            .await
            .is_err()
    {
        server.abort();
        let _ = server.await;
    }
    let process_stopped = match child.try_wait() {
        Ok(Some(_)) => true,
        _ => {
            let _ = child.start_kill();
            tokio::time::timeout(Duration::from_secs(5), child.wait())
                .await
                .is_ok_and(|r| r.is_ok())
        }
    };
    println!("WORK_ORDERS_LIVE_PROCESS_CLEANUP stopped={process_stopped}");
    result?;
    ensure!(
        process_stopped,
        "owned independent Rust process cleanup failed"
    );
    Ok(())
}
