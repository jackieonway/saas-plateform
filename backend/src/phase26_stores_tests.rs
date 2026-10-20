//! Real platform HTTP, independent stores/work-orders executables and three
//! disposable databases with separate, non-superuser runtime identities.
//! No developer dotenv, identity query by a business service or Node backend.
#[path = "phase30_authorization_tests.rs"]
mod phase30;
#[path = "phase31_old_clients.rs"]
mod phase31;
#[path = "phase32_monitoring_tests.rs"]
mod phase32;
use crate::{
    config::Config,
    state::AppState,
    util::{hash, password},
};
use anyhow::{anyhow, ensure, Context, Result};
use axum::Router;
use reqwest::{Client, Method};
use serde_json::{json, Value};
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool,
};
use std::{
    collections::HashMap, net::SocketAddr, process::Stdio, str::FromStr, sync::Arc, time::Duration,
};
use tokio::{io::AsyncReadExt, net::TcpListener, task::JoinHandle};
use uuid::Uuid;

const SECRET: &str = "phase26-owned-platform-signing-canary";
const PASSWORD: &str = "Phase26-Owned-Member-Password1!";
const PAIRS: [(&str, &str); 9] = [
    ("store", "read"),
    ("store", "create"),
    ("store", "update"),
    ("store-staff", "read"),
    ("store-staff", "assign"),
    ("store-staff", "unassign"),
    ("store-daily-report", "read"),
    ("store-daily-report", "create"),
    ("store-daily-report", "update"),
];
const CHECKS: [&str; 16] = [
    "real-login-and-confidential-introspection",
    "store-staff-daily-crud-and-summary",
    "self-scope-and-owner-not-staff",
    "org-scope",
    "subtree-scope",
    "tenant-and-hard-organization-ceiling",
    "permission-and-grant-independent-denials",
    "cross-app-token-and-client-binding-denials",
    "revocation-next-request",
    "member-directory-and-association",
    "inactive-expired-outside-members-denied",
    "immutable-tenant-organization-owner",
    "daily-unique-concurrency-no-double-summary",
    "optimistic-version-conflicts",
    "decimal-and-shanghai-business-date",
    "platform-fail-closed-and-secret-free-output",
];

fn config(policy: &Value) -> Result<Config> {
    Config::from_values(&HashMap::from([
        ("APPLICATION_ENVIRONMENT".into(), "development".into()),
        (
            "DATABASE_URL".into(),
            "postgresql://unused/phase26-owned".into(),
        ),
        ("JWT_SECRET".into(), SECRET.into()),
        (
            "OAUTH_INTROSPECTION_CLIENT_REQUESTS_PER_MINUTE".into(),
            "20000".into(),
        ),
        (
            "OAUTH_INTROSPECTION_ATTEMPTS_PER_MINUTE".into(),
            "30000".into(),
        ),
        ("DATA_GRANT_POLICY".into(), policy.to_string()),
    ]))
}

struct Database {
    kind: &'static str,
    name: String,
    role: String,
    url: String,
    pool: PgPool,
    admin: PgPool,
}
impl Database {
    async fn create(kind: &'static str) -> Result<Self> {
        ensure!(
            ["PLATFORM", "STORES", "WORK_ORDERS"].contains(&kind),
            "invalid owned database kind"
        );
        let source = std::env::var("TEST_DATABASE_URL")
            .context("explicit owned TEST_DATABASE_URL required")?;
        let owner =
            std::env::var("CI_TEST_POSTGRES_OWNER").context("managed PostgreSQL owner required")?;
        let mut url =
            reqwest::Url::parse(&source).map_err(|_| anyhow!("invalid owned database URL"))?;
        ensure!(
            owner.starts_with("ci-")
                && (11..=83).contains(&owner.len())
                && owner
                    .bytes()
                    .all(|v| v.is_ascii_lowercase() || v.is_ascii_digit() || v == b'-'),
            "invalid owned database owner"
        );
        ensure!(
            url.scheme() == "postgresql"
                && url.host_str() == Some(format!("{owner}-postgres").as_str())
                && url.port() == Some(5432)
                && url.username() == "ci"
                && url.password().is_some_and(|v| !v.is_empty())
                && url.path() == "/postgres"
                && url.query().is_none()
                && url.fragment().is_none(),
            "owned managed PostgreSQL administration endpoint required"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(
                PgConnectOptions::from_str(&source)
                    .map_err(|_| anyhow!("invalid owned database configuration"))?,
            )
            .await
            .map_err(|_| anyhow!("owned database administration failed"))?;
        let suffix = Uuid::new_v4().simple().to_string();
        let name = format!("phase26_{}_{}", kind.to_lowercase(), suffix);
        let role = format!("phase26_role_{suffix}");
        let credential = Uuid::new_v4().simple().to_string();
        // Both identifiers and the password are generated hexadecimal values,
        // never user input. Keep DDL credentials out of error/log output.
        let created = async {
            sqlx::query(&format!("CREATE ROLE \"{role}\" LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD '{credential}'")).execute(&admin).await.map_err(|_| anyhow!("owned runtime role creation failed"))?;
            sqlx::query(&format!("CREATE DATABASE \"{name}\" OWNER \"{role}\" TEMPLATE template0")).execute(&admin).await.map_err(|_| anyhow!("owned database creation failed"))?;
            sqlx::query(&format!("REVOKE CONNECT ON DATABASE \"{name}\" FROM PUBLIC")).execute(&admin).await?;
            sqlx::query(&format!("GRANT CONNECT ON DATABASE \"{name}\" TO \"{role}\"")).execute(&admin).await?;
            Ok::<_, anyhow::Error>(())
        }.await;
        if created.is_err() {
            let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"))
                .execute(&admin)
                .await;
            let _ = sqlx::query(&format!("DROP ROLE IF EXISTS \"{role}\""))
                .execute(&admin)
                .await;
            admin.close().await;
            return Err(anyhow!("owned runtime database setup failed"));
        }
        url.set_path(&format!("/{name}"));
        url.set_username(&role)
            .map_err(|_| anyhow!("owned role URL failed"))?;
        url.set_password(Some(&credential))
            .map_err(|_| anyhow!("owned credential URL failed"))?;
        let pool = match PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(5))
            .connect(url.as_str())
            .await
        {
            Ok(pool) => pool,
            Err(_) => {
                let _ = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
                    .execute(&admin)
                    .await;
                let _ = sqlx::query(&format!("DROP ROLE \"{role}\""))
                    .execute(&admin)
                    .await;
                admin.close().await;
                return Err(anyhow!("owned runtime connection failed"));
            }
        };
        Ok(Self {
            kind,
            name,
            role,
            url: url.to_string(),
            pool,
            admin,
        })
    }
    async fn cleanup(self, assertions_ok: bool) -> Result<()> {
        ensure!(
            self.name.starts_with("phase26_")
                && self.role.starts_with("phase26_role_")
                && [&self.name, &self.role]
                    .iter()
                    .all(|v| v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')),
            "invalid owned cleanup target"
        );
        let mut pool_closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let database_removed =
            sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
                .execute(&self.admin)
                .await
                .is_ok();
        pool_closed |= tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let role_removed = sqlx::query(&format!("DROP ROLE \"{}\"", self.role))
            .execute(&self.admin)
            .await
            .is_ok();
        pool_closed &= tokio::time::timeout(Duration::from_secs(5), self.admin.close())
            .await
            .is_ok();
        println!(
            "STORES_LIVE_{}_CLEANUP {}",
            self.kind,
            json!({"assertionsOk":assertions_ok,"databaseRemoved":database_removed,"roleRemoved":role_removed,"poolClosed":pool_closed})
        );
        ensure!(
            database_removed && role_removed && pool_closed,
            "owned database/role cleanup failed"
        );
        Ok(())
    }
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
    async fn stop(&mut self) -> bool {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        match tokio::time::timeout(Duration::from_secs(5), &mut self.task).await {
            Ok(Ok(Ok(()))) => true,
            _ => {
                self.task.abort();
                let _ = (&mut self.task).await;
                false
            }
        }
    }
}

struct Process {
    child: tokio::process::Child,
    logs: Vec<JoinHandle<Vec<u8>>>,
    origin: String,
}
impl Process {
    async fn start(
        kind: &str,
        database: &Database,
        platform: &str,
        client_id: &str,
        client_secret: &str,
    ) -> Result<Self> {
        let binary = std::env::var_os(if kind == "STORES" {
            "STORES_BINARY"
        } else {
            "WORK_ORDERS_BINARY"
        })
        .ok_or_else(|| anyhow!("explicit built business binary required"))?;
        ensure!(
            std::path::Path::new(&binary).is_absolute() && std::path::Path::new(&binary).is_file(),
            "business binary must be an existing absolute file"
        );
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        drop(listener);
        let mut command = tokio::process::Command::new(binary);
        command
            .env_clear()
            .env("NODE_ENV", "test")
            .env(format!("{kind}_DATABASE_URL"), &database.url)
            .env(format!("{kind}_HOST"), "127.0.0.1")
            .env(format!("{kind}_PORT"), port.to_string())
            .env(
                format!("{kind}_DB_MAX_CONNECTIONS"),
                std::env::var("PHASE32_TEST_DB_MAX_CONNECTIONS").unwrap_or_else(|_| "2".into()),
            )
            .env(
                format!("{kind}_METRICS_TOKEN"),
                std::env::var("PHASE32_TEST_METRICS_TOKEN").unwrap_or_default(),
            )
            .env(format!("{kind}_ALLOWED_ORIGINS"), "null")
            .env("PLATFORM_API_ORIGIN", platform)
            .env("PLATFORM_CLIENT_ID", client_id)
            .env("PLATFORM_CLIENT_SECRET", client_secret)
            .env(
                "PLATFORM_AUTHORIZATION_MODE",
                if std::env::var_os("PHASE32_TEST_DEFAULT_MODES").is_some() {
                    if kind == "STORES" {
                        "legacy".into()
                    } else {
                        "combined".into()
                    }
                } else {
                    std::env::var("PHASE30_TEST_AUTHORIZATION_MODE")
                        .unwrap_or_else(|_| "combined".into())
                },
            )
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
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
        let mut child = command
            .spawn()
            .map_err(|_| anyhow!("independent business process failed to start"))?;
        let mut logs = Vec::new();
        if let Some(stream) = child.stdout.take() {
            logs.push(tokio::spawn(async move {
                let mut bytes = Vec::new();
                let _ = stream.take(1024 * 1024 + 1).read_to_end(&mut bytes).await;
                bytes
            }));
        }
        if let Some(stream) = child.stderr.take() {
            logs.push(tokio::spawn(async move {
                let mut bytes = Vec::new();
                let _ = stream.take(1024 * 1024 + 1).read_to_end(&mut bytes).await;
                bytes
            }));
        }
        Ok(Self {
            child,
            logs,
            origin: format!("http://127.0.0.1:{port}"),
        })
    }
    async fn ready(&mut self, client: &Client, prefix: &str) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(40), async {
            loop {
                ensure!(
                    self.child.try_wait()?.is_none(),
                    "business process exited before readiness"
                );
                if client
                    .get(format!("{}{prefix}/health/ready", self.origin))
                    .send()
                    .await
                    .is_ok_and(|v| v.status().is_success())
                {
                    return Ok::<_, anyhow::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .map_err(|_| anyhow!("business readiness deadline exceeded"))?
    }
    async fn stop(&mut self, secrets: &[String]) -> Result<bool> {
        let stopped = if self.child.try_wait()?.is_some() {
            true
        } else {
            let _ = self.child.start_kill();
            tokio::time::timeout(Duration::from_secs(5), self.child.wait())
                .await
                .is_ok_and(|v| v.is_ok())
        };
        let mut safe = true;
        for task in self.logs.drain(..) {
            match tokio::time::timeout(Duration::from_secs(5), task).await {
                Ok(Ok(bytes)) => {
                    safe &= bytes.len() <= 1024 * 1024;
                    for secret in secrets {
                        if !secret.is_empty()
                            && bytes.windows(secret.len()).any(|v| v == secret.as_bytes())
                        {
                            safe = false;
                        }
                    }
                }
                _ => safe = false,
            }
        }
        ensure!(
            safe,
            "business process output exposed credentials or could not be verified"
        );
        Ok(stopped)
    }
}

struct Reply {
    path: String,
    request_id: String,
    status: u16,
    data: Value,
    text: String,
}
impl Reply {
    fn require(&self, status: u16) -> Result<&Value> {
        ensure!(
            self.status == status,
            "HTTP status {} expected {} at {} (safe code {})",
            self.status,
            status,
            self.path,
            self.data
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("none")
        );
        Ok(&self.data)
    }
    fn denied(&self) -> Result<()> {
        ensure!(
            [401, 403].contains(&self.status),
            "authorization was not denied: {}",
            self.status
        );
        Ok(())
    }
}
async fn request(
    client: &Client,
    origin: &str,
    path: &str,
    token: &str,
    method: Method,
    body: Option<Value>,
) -> Result<Reply> {
    let request_id = Uuid::new_v4().to_string();
    let mut req = client
        .request(method, format!("{origin}{path}"))
        .header("x-request-id", &request_id);
    if !token.is_empty() {
        req = req.bearer_auth(token);
    }
    if let Some(body) = body {
        req = req
            .header("content-type", "application/json")
            .body(serde_json::to_vec(&body)?);
    }
    let response = req
        .send()
        .await
        .map_err(|_| anyhow!("owned HTTP request failed"))?;
    if path.starts_with("/stores-api/") {
        ensure!(
            response
                .headers()
                .get("x-request-id")
                .and_then(|v| v.to_str().ok())
                == Some(request_id.as_str()),
            "stores response lost request identity"
        );
    }
    let status = response.status().as_u16();
    let bytes = response
        .bytes()
        .await
        .map_err(|_| anyhow!("owned HTTP response failed"))?;
    ensure!(
        bytes.len() <= 1024 * 1024 && !bytes.windows(SECRET.len()).any(|v| v == SECRET.as_bytes()),
        "unsafe response payload"
    );
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let data = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok(Reply {
        path: path.into(),
        request_id,
        status,
        data,
        text,
    })
}
async fn introspect(
    client: &Client,
    platform: &str,
    id: &str,
    secret: &str,
    token: &str,
) -> Result<bool> {
    let response = client
        .post(format!("{platform}/oauth/introspect"))
        .basic_auth(id, Some(secret))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(format!("token={token}"))
        .send()
        .await?;
    let success = response.status().is_success();
    let bytes = response.bytes().await?;
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok(success && value["active"] == true)
}
fn value_id(value: &Value) -> Result<Uuid> {
    field(value, "id")?
        .parse()
        .map_err(|_| anyhow!("invalid response identity"))
}
fn field(value: &Value, name: &str) -> Result<String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("missing response field {name}"))
}
fn pass(checks: &mut Vec<Value>, id: &str) -> Result<()> {
    ensure!(
        CHECKS.contains(&id) && !checks.iter().any(|v| v["id"] == id),
        "unknown/duplicate live check"
    );
    checks.push(json!({"id":id,"status":"PASS"}));
    println!("STORES_LIVE_CHECK {id}=PASS");
    Ok(())
}

#[derive(Clone)]
struct Member {
    tenant: Uuid,
    org: Uuid,
    user: Uuid,
    membership: Uuid,
    role: Uuid,
    username: String,
}
impl Member {
    fn target(&self, app: Uuid) -> Value {
        json!({"tenantId":self.tenant,"organizationId":self.org,"userId":self.user,"membershipId":self.membership,"applicationId":app})
    }
}
struct Fixture {
    policy: Value,
    app: Uuid,
    work_app: Uuid,
    menus: Vec<Uuid>,
    work_menus: Vec<Uuid>,
    tenant: Uuid,
    actor: Member,
    child: Member,
    peer: Member,
    sibling: Member,
    foreign: Member,
    platform_root: Member,
    platform_child: Member,
    candidate: Member,
    ancestor: Member,
    client_id: String,
    client_secret: String,
    work_client_id: String,
    work_client_secret: String,
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
async fn member(
    pool: &PgPool,
    tenant: Uuid,
    org: Uuid,
    username: &str,
    existing: Option<Uuid>,
    menus: &[Uuid],
    encoded: &str,
) -> Result<Member> {
    let user =
        match existing {
            Some(id) => id,
            None => sqlx::query_scalar(
                "INSERT INTO users(username,real_name,password_hash) VALUES($1,$1,$2) RETURNING id",
            )
            .bind(username)
            .bind(encoded)
            .fetch_one(pool)
            .await?,
        };
    let membership = sqlx::query_scalar(
        "INSERT INTO memberships(user_id,organization_id) VALUES($1,$2) RETURNING id",
    )
    .bind(user)
    .bind(org)
    .fetch_one(pool)
    .await?;
    let role = sqlx::query_scalar(
        "INSERT INTO roles(tenant_id,organization_id,name) VALUES($1,$2,$3) RETURNING id",
    )
    .bind(tenant)
    .bind(org)
    .bind(format!("{username}-{org}"))
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
        role,
        username: username.into(),
    })
}
impl Fixture {
    async fn prepare(pool: &PgPool) -> Result<Self> {
        crate::seed::initialize(
            pool,
            &config(&json!({"applications":[],"managementScopes":[]}))?,
        )
        .await?;
        let (platform, platform_root_org): (Uuid, Uuid) = sqlx::query_as("SELECT t.id,o.id FROM tenants t JOIN organizations o ON o.tenant_id=t.id AND o.parent_id IS NULL WHERE t.is_platform").fetch_one(pool).await?;
        let tenant = sqlx::query_scalar("INSERT INTO tenants(name,code) VALUES('Phase26 owned business','phase26-business') RETURNING id").fetch_one(pool).await?;
        let other = sqlx::query_scalar("INSERT INTO tenants(name,code) VALUES('Phase26 owned foreign','phase26-foreign') RETURNING id").fetch_one(pool).await?;
        let root = org(pool, tenant, None, "Phase26 root").await?;
        let current = org(pool, tenant, Some(root), "Phase26 current").await?;
        let child_org = org(pool, tenant, Some(current), "Phase26 child").await?;
        let sibling_org = org(pool, tenant, Some(root), "Phase26 sibling").await?;
        let foreign_org = org(pool, other, None, "Phase26 foreign root").await?;
        let platform_child_org = org(
            pool,
            platform,
            Some(platform_root_org),
            "Phase26 platform child",
        )
        .await?;
        let app: Uuid = sqlx::query_scalar("INSERT INTO applications(name,code,base_path,entry_type) VALUES('Phase26 stores','stores','/micro/stores','HTML') RETURNING id").fetch_one(pool).await?;
        let work_app: Uuid = sqlx::query_scalar("INSERT INTO applications(name,code,base_path,entry_type) VALUES('Phase26 work orders','work-orders','/micro/work-orders','HTML') RETURNING id").fetch_one(pool).await?;
        let mut menus = Vec::new();
        let mut work_menus = Vec::new();
        for (application, pairs, output) in [
            (app, PAIRS.to_vec(), &mut menus),
            (
                work_app,
                vec![
                    ("ticket", "read"),
                    ("ticket", "create"),
                    ("ticket", "update"),
                ],
                &mut work_menus,
            ),
        ] {
            for (resource, action) in pairs {
                output.push(sqlx::query_scalar("INSERT INTO menus(application_id,title,type,permission,visible) VALUES($1,$2,'BUTTON',$2,false) RETURNING id").bind(application).bind(format!("{resource}:{action}")).fetch_one(pool).await?);
            }
        }
        for (t, organizations) in [
            (tenant, vec![root, current, child_org, sibling_org]),
            (other, vec![foreign_org]),
            (platform, vec![platform_root_org, platform_child_org]),
        ] {
            for application in [app, work_app] {
                sqlx::query(
                    "INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)",
                )
                .bind(t)
                .bind(application)
                .execute(pool)
                .await?;
                for organization in &organizations {
                    sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)").bind(organization).bind(application).execute(pool).await?;
                }
            }
            for menu in menus.iter().chain(&work_menus) {
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
        let encoded = tokio::task::spawn_blocking(|| password::hash_password(PASSWORD)).await??;
        let all_menus = menus.iter().chain(&work_menus).copied().collect::<Vec<_>>();
        let actor = member(
            pool,
            tenant,
            current,
            "phase26-actor",
            None,
            &all_menus,
            &encoded,
        )
        .await?;
        let child = member(
            pool,
            tenant,
            child_org,
            "phase26-actor",
            Some(actor.user),
            &all_menus,
            &encoded,
        )
        .await?;
        let peer = member(
            pool,
            tenant,
            current,
            "phase26-peer",
            None,
            &all_menus,
            &encoded,
        )
        .await?;
        let sibling = member(
            pool,
            tenant,
            sibling_org,
            "phase26-sibling",
            None,
            &all_menus,
            &encoded,
        )
        .await?;
        let foreign = member(
            pool,
            other,
            foreign_org,
            "phase26-foreign",
            None,
            &all_menus,
            &encoded,
        )
        .await?;
        let platform_root = member(
            pool,
            platform,
            platform_root_org,
            "phase26-platform",
            None,
            &all_menus,
            &encoded,
        )
        .await?;
        let platform_child = member(
            pool,
            platform,
            platform_child_org,
            "phase26-platform",
            Some(platform_root.user),
            &all_menus,
            &encoded,
        )
        .await?;
        let candidate = member(
            pool,
            tenant,
            current,
            "phase26-candidate",
            None,
            &[],
            &encoded,
        )
        .await?;
        let ancestor = member(pool, tenant, root, "phase26-ancestor", None, &[], &encoded).await?;
        let client_id = Uuid::new_v4().to_string();
        let work_client_id = Uuid::new_v4().to_string();
        let client_secret = Uuid::new_v4().simple().to_string();
        let work_client_secret = Uuid::new_v4().simple().to_string();
        for (id, secret, application) in [
            (&client_id, &client_secret, app),
            (&work_client_id, &work_client_secret, work_app),
        ] {
            sqlx::query("INSERT INTO oauth_clients(client_id,client_name,client_secret_hash,application_id,client_type,grant_types,scopes) VALUES($1,'Phase26 owned client',$2,$3,'CONFIDENTIAL','client_credentials','openid')").bind(id).bind(hash::sha256_hex(secret)).bind(application).execute(pool).await?;
        }
        let capability = |pairs: &[(&str, &str)]| {
            pairs.iter().map(|(resource,action)|json!({"resource":resource,"action":action,"scopes":["SELF","ORG","ORG_SUBTREE","TENANT"]})).collect::<Vec<_>>()
        };
        let policy = json!({"applications":[
            {"applicationId":app,"capabilities":capability(&PAIRS),"memberLookups":[{"resource":"store","action":"create"},{"resource":"store","action":"update"},{"resource":"store-staff","action":"assign"}]},
            {"applicationId":work_app,"capabilities":capability(&[("ticket","read"),("ticket","create"),("ticket","update")]),"memberLookups":[{"resource":"ticket","action":"create"},{"resource":"ticket","action":"update"}]}],
            "managementScopes":[{"tenantId":tenant,"organizationId":root,"includeDescendants":true,"read":true,"write":true},{"tenantId":other,"organizationId":foreign_org,"includeDescendants":true,"read":true,"write":true},{"tenantId":platform,"organizationId":platform_root_org,"includeDescendants":true,"read":true,"write":true}]});
        Ok(Self {
            policy,
            app,
            work_app,
            menus,
            work_menus,
            tenant,
            actor,
            child,
            peer,
            sibling,
            foreign,
            platform_root,
            platform_child,
            candidate,
            ancestor,
            client_id,
            client_secret,
            work_client_id,
            work_client_secret,
        })
    }
}

async fn login(
    client: &Client,
    platform: &str,
    username: &str,
    password: &str,
    organization: Uuid,
) -> Result<String> {
    let reply = request(
        client,
        platform,
        "/api/auth/login",
        "",
        Method::POST,
        Some(json!({"username":username,"password":password})),
    )
    .await?;
    let ticket = field(reply.require(200)?, "ticket")?;
    let reply = request(
        client,
        platform,
        "/api/auth/context",
        &ticket,
        Method::POST,
        Some(json!({"organizationId":organization})),
    )
    .await?;
    field(reply.require(200)?, "accessToken")
}
async fn issue(client: &Client, platform: &str, session: &str, code: &str) -> Result<String> {
    let reply = request(
        client,
        platform,
        "/api/oauth/session-token",
        session,
        Method::POST,
        Some(json!({"appCode":code})),
    )
    .await?;
    field(reply.require(201)?, "accessToken")
}
async fn token(client: &Client, platform: &str, member: &Member, code: &str) -> Result<String> {
    issue(
        client,
        platform,
        &login(client, platform, &member.username, PASSWORD, member.org).await?,
        code,
    )
    .await
}
async fn grants(
    client: &Client,
    platform: &str,
    admin: &str,
    member: &Member,
    app: Uuid,
    pairs: &[(&str, &str)],
    scope: Option<&str>,
) -> Result<()> {
    let target = member.target(app);
    let query = target
        .as_object()
        .ok_or_else(|| anyhow!("invalid owned target"))?
        .iter()
        .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or_default()))
        .collect::<Vec<_>>()
        .join("&");
    let before = request(
        client,
        platform,
        &format!("/api/data-grants?{query}"),
        admin,
        Method::GET,
        None,
    )
    .await?;
    let snapshot = before.require(200)?;
    let upsert = scope
        .map(|scope| {
            pairs
                .iter()
                .map(|(r, a)| json!({"resource":r,"action":a,"scope":scope}))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let remove = if scope.is_none() {
        pairs
            .iter()
            .map(|(r, a)| json!({"resource":r,"action":a}))
            .collect::<Vec<_>>()
    } else {
        vec![]
    };
    let body = json!({"target":target,"expectedVersion":snapshot["version"],"upsert":upsert,"remove":remove});
    request(
        client,
        platform,
        "/api/data-grants/preview",
        admin,
        Method::POST,
        Some(body.clone()),
    )
    .await?
    .require(200)?;
    request(
        client,
        platform,
        "/api/data-grants",
        admin,
        Method::PATCH,
        Some(body),
    )
    .await?
    .require(200)?;
    Ok(())
}
async fn set_permissions(
    client: &Client,
    platform: &str,
    admin: &str,
    member: &Member,
    menus: &[Uuid],
) -> Result<()> {
    request(
        client,
        platform,
        &format!("/api/roles/{}/menus", member.role),
        admin,
        Method::PUT,
        Some(json!({"menuIds":menus})),
    )
    .await?
    .require(200)?;
    Ok(())
}
async fn store(client: &Client, origin: &str, token: &str, name: &str) -> Result<Uuid> {
    let reply = request(
        client,
        origin,
        "/stores-api/stores",
        token,
        Method::POST,
        Some(
            json!({"name":name,"businessStatus":"OPEN","businessContact":"Phase26 owned contact"}),
        ),
    )
    .await?;
    value_id(reply.require(201)?)
}
async fn report(
    client: &Client,
    origin: &str,
    token: &str,
    store: Uuid,
    day: &str,
    amount: &str,
) -> Result<Uuid> {
    let reply = request(
        client,
        origin,
        &format!("/stores-api/stores/{store}/daily-reports"),
        token,
        Method::POST,
        Some(json!({"businessDate":day,"amount":amount,"note":"Phase26 owned daily"})),
    )
    .await?;
    value_id(reply.require(201)?)
}
async fn list_count(
    client: &Client,
    origin: &str,
    token: &str,
    path: &str,
    total: u64,
) -> Result<Value> {
    let reply = request(client, origin, path, token, Method::GET, None).await?;
    let value = reply.require(200)?;
    ensure!(
        value["total"] == total
            && value["records"]
                .as_array()
                .is_some_and(|v| v.len() as u64 == total),
        "SQL list/count boundary mismatch at {path}"
    );
    Ok(value.clone())
}
async fn summary(
    client: &Client,
    origin: &str,
    token: &str,
    count: u64,
    amount: &str,
) -> Result<()> {
    let reply = request(
        client,
        origin,
        "/stores-api/daily-reports/summary?dateFrom=2026-10-01&dateTo=2026-10-31",
        token,
        Method::GET,
        None,
    )
    .await?;
    let value = reply.require(200)?;
    ensure!(
        value["count"] == count && value["totalAmount"] == amount,
        "SQL summary does not match authorized rows"
    );
    Ok(())
}

/// A real HTTP mutation must roll back when a BEFORE INSERT audit trigger
/// suppresses the audit row without raising an SQL error. Remove our exact
/// fixture trigger/function even if either assertion fails; the lifecycle owner
/// still drops the whole owned database and runtime role on every exit.
async fn audit_zero_rows(
    client: &Client,
    origin: &str,
    token: &str,
    pool: &PgPool,
    store: Uuid,
    existing_report: Uuid,
    actor: Uuid,
) -> Result<()> {
    let before: (String, i32, String, String) = sqlx::query_as(
        "SELECT amount::text,version,COALESCE(note,''),updated_at::text FROM store_daily_reports WHERE id=$1",
    )
    .bind(existing_report)
    .fetch_one(pool)
    .await?;
    let before_count: i64 = sqlx::query_scalar("SELECT count(*) FROM store_daily_reports")
        .fetch_one(pool)
        .await?;
    let before_audits: i64 = sqlx::query_scalar("SELECT count(*) FROM store_audit_events")
        .fetch_one(pool)
        .await?;
    let create_path = format!("/stores-api/stores/{store}/daily-reports");
    let update_path = format!("/stores-api/daily-reports/{existing_report}");
    let create_body =
        json!({"businessDate":"2025-12-30","amount":"0.11","note":"phase26-audit-zero-create"});
    let update_body =
        json!({"version":before.1,"amount":"0.16","note":"phase26-audit-zero-update"});
    let negative = async {
        sqlx::query("CREATE FUNCTION public.phase26_daily_zero_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.resource_code='store-daily-report' THEN RETURN NULL; END IF; RETURN NEW; END $$")
            .execute(pool).await?;
        sqlx::query("CREATE TRIGGER phase26_daily_zero_audit BEFORE INSERT ON public.store_audit_events FOR EACH ROW EXECUTE FUNCTION public.phase26_daily_zero_audit()")
            .execute(pool).await?;
        let create = request(client, origin, &create_path, token, Method::POST, Some(create_body.clone())).await?;
        create.require(503)?;
        let create_request: Uuid = create.request_id.parse()?;
        let leaked_create: i64 = sqlx::query_scalar("SELECT count(*) FROM store_daily_reports WHERE store_id=$1 AND business_date='2025-12-30'")
            .bind(store).fetch_one(pool).await?;
        let create_audits: i64 = sqlx::query_scalar("SELECT count(*) FROM store_audit_events WHERE request_id=$1")
            .bind(create_request).fetch_one(pool).await?;
        ensure!(leaked_create==0 && create_audits==0, "zero-row create audit committed a daily row or audit");
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM store_daily_reports").fetch_one(pool).await?;
        ensure!(count==before_count, "zero-row audit create changed the global daily count");

        let update = request(client, origin, &update_path, token, Method::PATCH, Some(update_body.clone())).await?;
        update.require(503)?;
        let update_request: Uuid = update.request_id.parse()?;
        let after: (String,i32,String,String) = sqlx::query_as("SELECT amount::text,version,COALESCE(note,''),updated_at::text FROM store_daily_reports WHERE id=$1")
            .bind(existing_report).fetch_one(pool).await?;
        let update_audits: i64 = sqlx::query_scalar("SELECT count(*) FROM store_audit_events WHERE request_id=$1")
            .bind(update_request).fetch_one(pool).await?;
        let audit_count: i64 = sqlx::query_scalar("SELECT count(*) FROM store_audit_events").fetch_one(pool).await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM store_daily_reports").fetch_one(pool).await?;
        ensure!(after==before && update_audits==0 && audit_count==before_audits && count==before_count,
            "zero-row audit update changed amount, note, version, timestamp, count or audit");
        Ok::<_,anyhow::Error>((create_request,update_request))
    }.await;
    let trigger_removed =
        sqlx::query("DROP TRIGGER IF EXISTS phase26_daily_zero_audit ON public.store_audit_events")
            .execute(pool)
            .await
            .is_ok();
    let function_removed = sqlx::query("DROP FUNCTION IF EXISTS public.phase26_daily_zero_audit()")
        .execute(pool)
        .await
        .is_ok();
    ensure!(
        trigger_removed && function_removed,
        "owned zero-row audit fault cleanup failed"
    );
    let (create_request, update_request) = negative?;

    let created = request(
        client,
        origin,
        &create_path,
        token,
        Method::POST,
        Some(create_body),
    )
    .await?;
    let created_id = value_id(created.require(201)?)?;
    let create_retry: Uuid = created.request_id.parse()?;
    let create_audits:i64=sqlx::query_scalar("SELECT count(*) FROM store_audit_events WHERE request_id=$1 AND resource_code='store-daily-report' AND action_code='create' AND entity_id=$2 AND actor_user_id=$3")
        .bind(create_retry).bind(created_id).bind(actor).fetch_one(pool).await?;
    ensure!(
        create_audits == 1,
        "successful create retry did not commit exactly its request-bound audit"
    );
    let updated = request(
        client,
        origin,
        &update_path,
        token,
        Method::PATCH,
        Some(update_body),
    )
    .await?;
    let updated_value = updated.require(200)?;
    ensure!(
        updated_value["amount"] == "0.16" && updated_value["version"] == before.1 + 1,
        "successful update retry did not commit exactly once"
    );
    let update_retry: Uuid = updated.request_id.parse()?;
    let update_audits:i64=sqlx::query_scalar("SELECT count(*) FROM store_audit_events WHERE request_id=$1 AND resource_code='store-daily-report' AND action_code='update' AND entity_id=$2 AND actor_user_id=$3")
        .bind(update_retry).bind(existing_report).bind(actor).fetch_one(pool).await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM store_daily_reports")
        .fetch_one(pool)
        .await?;
    let audit_count: i64 = sqlx::query_scalar("SELECT count(*) FROM store_audit_events")
        .fetch_one(pool)
        .await?;
    ensure!(
        update_audits == 1 && count == before_count + 1 && audit_count == before_audits + 2,
        "audit fault recovery created duplicate records or audit events"
    );
    println!(
        "STORES_LIVE_AUDIT_ZERO_PROOF {}",
        json!({"createRequestId":create_request,"updateRequestId":update_request,"createRolledBack":true,"updateRolledBack":true,"createRetryAuditCount":create_audits,"updateRetryAuditCount":update_audits,"triggerRemoved":true})
    );
    Ok(())
}

async fn assertions(
    client: &Client,
    platform: &mut Server,
    stores: &mut Process,
    work: &mut Process,
    databases: &[Database],
    f: &Fixture,
    checks: &mut Vec<Value>,
    secrets: &mut Vec<String>,
) -> Result<()> {
    let platform_origin = platform.origin.clone();
    let origin = stores.origin.clone();
    let pool = &databases[0].pool;
    let business_pool = &databases[1].pool;
    let admin_org: Uuid = sqlx::query_scalar("SELECT o.id FROM organizations o JOIN tenants t ON t.id=o.tenant_id WHERE t.is_platform AND o.parent_id IS NULL").fetch_one(pool).await?;
    let admin = login(client, &platform_origin, "admin", "admin123", admin_org).await?;
    secrets.push(admin.clone());
    let actor = token(client, &platform_origin, &f.actor, "stores").await?;
    let context = request(
        client,
        &platform_origin,
        "/api/apps/stores/context",
        &actor,
        Method::GET,
        None,
    )
    .await?;
    ensure!(
        !context.require(200)?["permissions"]
            .as_array()
            .ok_or_else(|| anyhow!("missing current permissions"))?
            .iter()
            .any(|v| v
                .as_str()
                .is_some_and(|v| v.starts_with("system:data-grant"))),
        "ordinary business actor has grant management permission"
    );
    request(
        client,
        &origin,
        "/stores-api/stores",
        &actor,
        Method::GET,
        None,
    )
    .await?
    .require(403)?;
    for member in [
        &f.actor,
        &f.child,
        &f.peer,
        &f.sibling,
        &f.foreign,
        &f.platform_root,
        &f.platform_child,
    ] {
        grants(
            client,
            &platform_origin,
            &admin,
            member,
            f.app,
            &PAIRS,
            Some("ORG_SUBTREE"),
        )
        .await?;
    }
    grants(
        client,
        &platform_origin,
        &admin,
        &f.actor,
        f.work_app,
        &[
            ("ticket", "read"),
            ("ticket", "create"),
            ("ticket", "update"),
        ],
        Some("ORG"),
    )
    .await?;
    let peer = token(client, &platform_origin, &f.peer, "stores").await?;
    let child = token(client, &platform_origin, &f.child, "stores").await?;
    let sibling = token(client, &platform_origin, &f.sibling, "stores").await?;
    let foreign = token(client, &platform_origin, &f.foreign, "stores").await?;
    let platform_token = token(client, &platform_origin, &f.platform_root, "stores").await?;
    let platform_child = token(client, &platform_origin, &f.platform_child, "stores").await?;
    let work_token = token(client, &platform_origin, &f.actor, "work-orders").await?;
    secrets.extend([
        actor.clone(),
        peer.clone(),
        child.clone(),
        sibling.clone(),
        foreign.clone(),
        platform_token.clone(),
        platform_child.clone(),
        work_token.clone(),
    ]);
    ensure!(
        introspect(
            client,
            &platform_origin,
            &f.client_id,
            &f.client_secret,
            &actor
        )
        .await?,
        "real confidential introspection was not active"
    );
    pass(checks, CHECKS[0])?;

    // All business records are created through the independent service HTTP.
    // One store per organization precludes two owners in a single organization;
    // the primary actor has direct membership in current and child organizations.
    let current_store = store(client, &origin, &peer, "Phase26 peer-owned current store").await?;
    let child_store = store(client, &origin, &child, "Phase26 actor-owned child store").await?;
    let sibling_store = store(client, &origin, &sibling, "Phase26 sibling store").await?;
    let foreign_store = store(client, &origin, &foreign, "Phase26 foreign store").await?;
    let root_store = store(
        client,
        &origin,
        &platform_token,
        "Phase26 platform root store",
    )
    .await?;
    let platform_child_store = store(
        client,
        &origin,
        &platform_child,
        "Phase26 platform child store",
    )
    .await?;
    let current_report =
        report(client, &origin, &actor, current_store, "2026-10-01", "0.10").await?;
    let peer_report = report(client, &origin, &peer, current_store, "2026-10-02", "0.20").await?;
    let child_report = report(client, &origin, &child, child_store, "2026-10-03", "0.30").await?;
    let sibling_report = report(
        client,
        &origin,
        &sibling,
        sibling_store,
        "2026-10-04",
        "4.00",
    )
    .await?;
    let foreign_report = report(
        client,
        &origin,
        &foreign,
        foreign_store,
        "2026-10-05",
        "5.00",
    )
    .await?;
    report(
        client,
        &origin,
        &platform_token,
        root_store,
        "2026-10-06",
        "1.00",
    )
    .await?;
    report(
        client,
        &origin,
        &platform_child,
        platform_child_store,
        "2026-10-07",
        "2.00",
    )
    .await?;
    let staff_path = format!("/stores-api/stores/{current_store}/staff");
    let link = request(
        client,
        &origin,
        &staff_path,
        &peer,
        Method::POST,
        Some(json!({"userId":f.actor.user})),
    )
    .await?;
    let actor_link = value_id(link.require(201)?)?;
    let link = request(
        client,
        &origin,
        &staff_path,
        &actor,
        Method::POST,
        Some(json!({"userId":f.peer.user})),
    )
    .await?;
    let peer_link = value_id(link.require(201)?)?;
    request(
        client,
        &origin,
        &format!("/stores-api/stores/{child_store}/staff"),
        &child,
        Method::POST,
        Some(json!({"userId":f.child.user})),
    )
    .await?
    .require(201)?;

    grants(
        client,
        &platform_origin,
        &admin,
        &f.actor,
        f.app,
        &PAIRS,
        Some("SELF"),
    )
    .await?;
    let own = list_count(client, &origin, &actor, "/stores-api/stores", 1).await?;
    ensure!(
        own["records"][0]["id"] == child_store.to_string(),
        "store SELF followed manager/staff instead of creator"
    );
    request(
        client,
        &origin,
        &format!("/stores-api/stores/{current_store}"),
        &actor,
        Method::GET,
        None,
    )
    .await?
    .require(404)?;
    let staff = list_count(client, &origin, &actor, &staff_path, 1).await?;
    ensure!(
        staff["records"][0]["id"] == actor_link.to_string()
            && staff["records"][0]["userId"] == f.actor.user.to_string(),
        "staff SELF must follow target user, never assigning actor"
    );
    ensure!(
        staff["records"][0]["id"] != peer_link.to_string(),
        "staff SELF expanded through createdByUserId"
    );
    list_count(
        client,
        &origin,
        &actor,
        &format!("/stores-api/stores/{current_store}/daily-reports"),
        1,
    )
    .await?;
    request(
        client,
        &origin,
        &format!("/stores-api/daily-reports/{peer_report}"),
        &actor,
        Method::GET,
        None,
    )
    .await?
    .require(404)?;
    request(
        client,
        &origin,
        &format!("/stores-api/daily-reports/{child_report}"),
        &actor,
        Method::GET,
        None,
    )
    .await?
    .require(200)?;
    summary(client, &origin, &actor, 2, "0.40").await?;
    pass(checks, CHECKS[2])?;

    grants(
        client,
        &platform_origin,
        &admin,
        &f.actor,
        f.app,
        &PAIRS,
        Some("ORG"),
    )
    .await?;
    list_count(client, &origin, &actor, "/stores-api/stores", 1).await?;
    list_count(client, &origin, &actor, &staff_path, 2).await?;
    list_count(
        client,
        &origin,
        &actor,
        &format!("/stores-api/stores/{current_store}/daily-reports"),
        2,
    )
    .await?;
    request(
        client,
        &origin,
        &format!("/stores-api/stores/{child_store}"),
        &actor,
        Method::GET,
        None,
    )
    .await?
    .require(404)?;
    summary(client, &origin, &actor, 2, "0.30").await?;
    pass(checks, CHECKS[3])?;
    grants(
        client,
        &platform_origin,
        &admin,
        &f.actor,
        f.app,
        &PAIRS,
        Some("ORG_SUBTREE"),
    )
    .await?;
    list_count(client, &origin, &actor, "/stores-api/stores", 2).await?;
    request(
        client,
        &origin,
        &format!("/stores-api/stores/{child_store}"),
        &actor,
        Method::GET,
        None,
    )
    .await?
    .require(200)?;
    for hidden in [sibling_store, foreign_store] {
        request(
            client,
            &origin,
            &format!("/stores-api/stores/{hidden}"),
            &actor,
            Method::GET,
            None,
        )
        .await?
        .require(404)?;
        request(
            client,
            &origin,
            &format!("/stores-api/stores/{hidden}"),
            &actor,
            Method::PATCH,
            Some(json!({"version":1,"name":"forbidden"})),
        )
        .await?
        .require(404)?;
        request(
            client,
            &origin,
            &format!("/stores-api/stores/{hidden}/staff"),
            &actor,
            Method::GET,
            None,
        )
        .await?
        .require(404)?;
        list_count(
            client,
            &origin,
            &actor,
            &format!("/stores-api/stores/{hidden}/daily-reports"),
            0,
        )
        .await?;
    }
    for hidden in [sibling_report, foreign_report] {
        request(
            client,
            &origin,
            &format!("/stores-api/daily-reports/{hidden}"),
            &actor,
            Method::GET,
            None,
        )
        .await?
        .require(404)?;
    }
    summary(client, &origin, &actor, 3, "0.60").await?;
    pass(checks, CHECKS[4])?;
    grants(
        client,
        &platform_origin,
        &admin,
        &f.actor,
        f.app,
        &PAIRS,
        Some("TENANT"),
    )
    .await?;
    list_count(client, &origin, &actor, "/stores-api/stores", 2).await?;
    summary(client, &origin, &actor, 3, "0.60").await?;
    grants(
        client,
        &platform_origin,
        &admin,
        &f.platform_root,
        f.app,
        &PAIRS,
        Some("TENANT"),
    )
    .await?;
    list_count(client, &origin, &platform_token, "/stores-api/stores", 2).await?;
    summary(client, &origin, &platform_token, 2, "3.00").await?;
    request(
        client,
        &origin,
        &format!("/stores-api/stores/{current_store}"),
        &platform_token,
        Method::GET,
        None,
    )
    .await?
    .require(404)?;
    pass(checks, CHECKS[5])?;

    // Remove every functional permission while grants remain, then restore the
    // exact role menus and remove grants. Same bearer, fresh platform context.
    set_permissions(client, &platform_origin, &admin, &f.actor, &[]).await?;
    for path in [
        "/stores-api/stores".to_string(),
        staff_path.clone(),
        "/stores-api/daily-reports/summary?dateFrom=2026-10-01&dateTo=2026-10-31".to_string(),
    ] {
        request(client, &origin, &path, &actor, Method::GET, None)
            .await?
            .require(403)?;
    }
    request(
        client,
        &origin,
        "/stores-api/stores",
        &actor,
        Method::POST,
        Some(json!({"name":"no permission","businessStatus":"OPEN","businessContact":""})),
    )
    .await?
    .require(403)?;
    let all_menus = f
        .menus
        .iter()
        .chain(&f.work_menus)
        .copied()
        .collect::<Vec<_>>();
    set_permissions(client, &platform_origin, &admin, &f.actor, &all_menus).await?;
    grants(
        client,
        &platform_origin,
        &admin,
        &f.actor,
        f.app,
        &PAIRS,
        None,
    )
    .await?;
    for path in [
        "/stores-api/stores".to_string(),
        staff_path.clone(),
        "/stores-api/daily-reports/summary?dateFrom=2026-10-01&dateTo=2026-10-31".to_string(),
    ] {
        request(client, &origin, &path, &actor, Method::GET, None)
            .await?
            .require(403)?;
    }
    grants(
        client,
        &platform_origin,
        &admin,
        &f.actor,
        f.app,
        &PAIRS,
        Some("ORG_SUBTREE"),
    )
    .await?;
    pass(checks, CHECKS[6])?;

    // The actual services reject one another's signed application tokens.
    request(
        client,
        &origin,
        "/stores-api/stores",
        &work_token,
        Method::GET,
        None,
    )
    .await?
    .denied()?;
    request(
        client,
        &work.origin,
        "/work-orders-api/tickets",
        &actor,
        Method::GET,
        None,
    )
    .await?
    .denied()?;
    request(
        client,
        &work.origin,
        "/work-orders-api/tickets",
        &work_token,
        Method::GET,
        None,
    )
    .await?
    .require(200)?;
    ensure!(
        !introspect(
            client,
            &platform_origin,
            &f.work_client_id,
            &f.work_client_secret,
            &actor
        )
        .await?,
        "foreign application client introspected stores token"
    );
    ensure!(
        !introspect(
            client,
            &platform_origin,
            &f.client_id,
            "phase26-invalid-client-secret",
            &actor
        )
        .await?,
        "incorrect client secret introspected token"
    );
    let mut misbound = Process::start(
        "STORES",
        &databases[1],
        &platform_origin,
        &f.work_client_id,
        &f.work_client_secret,
    )
    .await?;
    let wrong_result = async {
        misbound.ready(client, "/stores-api").await?;
        request(
            client,
            &misbound.origin,
            "/stores-api/stores",
            &actor,
            Method::GET,
            None,
        )
        .await?
        .denied()
    }
    .await;
    let misbound_stopped = misbound.stop(secrets).await?;
    ensure!(misbound_stopped, "misbound client process cleanup failed");
    wrong_result?;
    pass(checks, CHECKS[7])?;

    let members_path = format!("/stores-api/members?resource=store-staff&action=assign&organizationId={}&q=phase26-candidate&page=1&size=1",f.actor.org);
    let directory = request(client, &origin, &members_path, &actor, Method::GET, None).await?;
    let directory = directory.require(200)?;
    ensure!(
        directory["total"] == 1 && directory["records"][0]["id"] == f.candidate.user.to_string(),
        "real controlled members search/association mismatch"
    );
    request(
        client,
        &origin,
        &format!(
            "/stores-api/members?resource=store&action=read&organizationId={}",
            f.actor.org
        ),
        &actor,
        Method::GET,
        None,
    )
    .await?
    .require(400)?;
    let update = request(client,&origin,&format!("/stores-api/stores/{current_store}"),&actor,Method::PATCH,Some(json!({"version":1,"managerUserId":f.candidate.user,"businessContact":"verified manager"}))).await?;
    ensure!(
        update.require(200)?["managerUserId"] == f.candidate.user.to_string(),
        "manager was not verified/associated"
    );
    let assigned = request(
        client,
        &origin,
        &staff_path,
        &actor,
        Method::POST,
        Some(json!({"userId":f.candidate.user})),
    )
    .await?;
    let candidate_link = value_id(assigned.require(201)?)?;
    request(
        client,
        &origin,
        &format!("{staff_path}/{candidate_link}"),
        &actor,
        Method::DELETE,
        Some(json!({"version":1})),
    )
    .await?
    .require(200)?;
    pass(checks, CHECKS[9])?;

    request(
        client,
        &platform_origin,
        &format!("/api/users/{}/status", f.candidate.user),
        &admin,
        Method::PUT,
        Some(json!({"status":"DISABLED"})),
    )
    .await?
    .require(200)?;
    request(
        client,
        &origin,
        &staff_path,
        &actor,
        Method::POST,
        Some(json!({"userId":f.candidate.user})),
    )
    .await?
    .require(404)?;
    request(
        client,
        &origin,
        &format!("/stores-api/stores/{current_store}"),
        &actor,
        Method::PATCH,
        Some(json!({"version":2,"businessContact":"unchanged disabled historical manager"})),
    )
    .await?
    .require(200)?;
    request(
        client,
        &platform_origin,
        &format!("/api/users/{}/status", f.candidate.user),
        &admin,
        Method::PUT,
        Some(json!({"status":"ENABLED"})),
    )
    .await?
    .require(200)?;
    request(
        client,
        &platform_origin,
        &format!("/api/users/{}/memberships", f.candidate.user),
        &admin,
        Method::PUT,
        Some(json!({"items":[]})),
    )
    .await?
    .require(200)?;
    request(
        client,
        &origin,
        &staff_path,
        &actor,
        Method::POST,
        Some(json!({"userId":f.candidate.user})),
    )
    .await?
    .require(404)?;
    for outside in [f.ancestor.user, f.sibling.user, f.foreign.user] {
        request(
            client,
            &origin,
            &staff_path,
            &actor,
            Method::POST,
            Some(json!({"userId":outside})),
        )
        .await?
        .require(404)?;
    }
    // Controlled fixture fault: expiration is a DB-level boundary on platform
    // identity data only, never an authorization write by the business process.
    sqlx::query("UPDATE organizations SET expires_at=now()-interval '1 second' WHERE id=$1")
        .bind(f.child.org)
        .execute(pool)
        .await?;
    request(
        client,
        &origin,
        &format!("/stores-api/stores/{child_store}/staff"),
        &actor,
        Method::POST,
        Some(json!({"userId":f.child.user})),
    )
    .await?
    .require(404)?;
    sqlx::query("UPDATE organizations SET expires_at=NULL WHERE id=$1")
        .bind(f.child.org)
        .execute(pool)
        .await?;
    pass(checks, CHECKS[10])?;

    for property in ["tenantId", "organizationId", "ownerUserId"] {
        let mut body = json!({"version":3,"name":"must reject immutable identity"});
        body[property] = json!(Uuid::new_v4());
        request(
            client,
            &origin,
            &format!("/stores-api/stores/{current_store}"),
            &actor,
            Method::PATCH,
            Some(body),
        )
        .await?
        .require(400)?;
    }
    for property in [
        "tenantId",
        "organizationId",
        "ownerUserId",
        "storeId",
        "businessDate",
    ] {
        let mut body = json!({"version":1,"amount":"0.11"});
        body[property] = json!(if property == "businessDate" {
            "2026-10-08".to_string()
        } else {
            Uuid::new_v4().to_string()
        });
        request(
            client,
            &origin,
            &format!("/stores-api/daily-reports/{current_report}"),
            &actor,
            Method::PATCH,
            Some(body),
        )
        .await?
        .require(400)?;
    }
    let identity: (Uuid, Uuid, Uuid) =
        sqlx::query_as("SELECT tenant_id,organization_id,owner_user_id FROM stores WHERE id=$1")
            .bind(current_store)
            .fetch_one(business_pool)
            .await?;
    ensure!(
        identity == (f.tenant, f.actor.org, f.peer.user),
        "trusted store identity changed"
    );
    pass(checks, CHECKS[11])?;

    let race_path = format!("/stores-api/stores/{current_store}/daily-reports");
    let race_body = json!({"businessDate":"2026-10-20","amount":"0.70"});
    let (a, b) = tokio::join!(
        request(
            client,
            &origin,
            &race_path,
            &actor,
            Method::POST,
            Some(race_body.clone())
        ),
        request(
            client,
            &origin,
            &race_path,
            &actor,
            Method::POST,
            Some(race_body.clone())
        )
    );
    let (a, b) = (a?, b?);
    let mut statuses = [a.status, b.status];
    statuses.sort();
    ensure!(
        statuses == [201, 409],
        "same-day concurrent requests must create exactly once"
    );
    request(
        client,
        &origin,
        &race_path,
        &actor,
        Method::POST,
        Some(race_body),
    )
    .await?
    .require(409)?;
    request(
        client,
        &origin,
        &race_path,
        &actor,
        Method::POST,
        Some(json!({"businessDate":"2026-10-20","amount":"999.00"})),
    )
    .await?
    .require(409)?;
    let (rows, amount):(i64,String) = sqlx::query_as("SELECT count(*),COALESCE(sum(amount),0)::text FROM store_daily_reports WHERE store_id=$1 AND business_date='2026-10-20'").bind(current_store).fetch_one(business_pool).await?;
    ensure!(
        rows == 1 && amount == "0.70",
        "duplicate daily request changed SQL count/total"
    );
    summary(client, &origin, &actor, 4, "1.30").await?;
    pass(checks, CHECKS[12])?;
    let update_path = format!("/stores-api/daily-reports/{current_report}");
    let (a, b) = tokio::join!(
        request(
            client,
            &origin,
            &update_path,
            &actor,
            Method::PATCH,
            Some(json!({"version":1,"amount":"0.15"}))
        ),
        request(
            client,
            &origin,
            &update_path,
            &actor,
            Method::PATCH,
            Some(json!({"version":1,"amount":"0.15"}))
        )
    );
    let (a, b) = (a?, b?);
    let mut statuses = [a.status, b.status];
    statuses.sort();
    ensure!(
        statuses == [200, 409],
        "optimistic update did not have a single winner"
    );
    request(
        client,
        &origin,
        &format!("/stores-api/stores/{current_store}"),
        &actor,
        Method::PATCH,
        Some(json!({"version":1,"name":"stale"})),
    )
    .await?
    .require(409)?;
    summary(client, &origin, &actor, 4, "1.35").await?;
    pass(checks, CHECKS[13])?;

    for amount in [
        json!(0.1),
        json!("-1.00"),
        json!("1e2"),
        json!("0.001"),
        json!("10000000000000000.00"),
    ] {
        request(
            client,
            &origin,
            &race_path,
            &actor,
            Method::POST,
            Some(json!({"businessDate":"2026-10-21","amount":amount})),
        )
        .await?
        .require(400)?;
    }
    for day in [
        "2026-02-29",
        "2026-1-01",
        "2026-10-01T00:00:00+08:00",
        "0000-01-01",
        "10000-01-01",
    ] {
        request(
            client,
            &origin,
            &race_path,
            &actor,
            Method::POST,
            Some(json!({"businessDate":day,"amount":"0.10"})),
        )
        .await?
        .require(400)?;
    }
    let leap = report(
        client,
        &origin,
        &actor,
        current_store,
        "2024-02-29",
        "9999999999999999.99",
    )
    .await?;
    let leap_reply = request(
        client,
        &origin,
        &format!("/stores-api/daily-reports/{leap}"),
        &actor,
        Method::GET,
        None,
    )
    .await?;
    ensure!(
        leap_reply.require(200)?["amount"] == "9999999999999999.99",
        "NUMERIC exact maximum lost precision"
    );
    let before_day = (chrono::Utc::now() + chrono::Duration::hours(8))
        .format("%Y-%m-%d")
        .to_string();
    let default_day = request(
        client,
        &origin,
        &race_path,
        &actor,
        Method::POST,
        Some(json!({"amount":"0.00"})),
    )
    .await?;
    let after_day = (chrono::Utc::now() + chrono::Duration::hours(8))
        .format("%Y-%m-%d")
        .to_string();
    if default_day.status == 201 {
        let day = field(default_day.require(201)?, "businessDate")?;
        ensure!(
            day == before_day || day == after_day,
            "default business date is not Asia/Shanghai civil day"
        );
    } else {
        ensure!(
            default_day.status == 409
                && [before_day.as_str(), after_day.as_str()].contains(&"2026-10-20"),
            "unexpected default business day conflict"
        );
    }
    pass(checks, CHECKS[14])?;
    audit_zero_rows(
        client,
        &origin,
        &actor,
        business_pool,
        current_store,
        current_report,
        f.actor.user,
    )
    .await?;
    pass(checks, CHECKS[1])?;

    // The actor's unchanged token must immediately lose its context after
    // removing the exact direct membership, while child membership is retained.
    request(
        client,
        &platform_origin,
        &format!("/api/users/{}/memberships", f.actor.user),
        &admin,
        Method::PUT,
        Some(json!({"items":[{"organizationId":f.child.org}]})),
    )
    .await?
    .require(200)?;
    request(
        client,
        &origin,
        "/stores-api/stores",
        &actor,
        Method::GET,
        None,
    )
    .await?
    .denied()?;
    request(client, &origin, &staff_path, &actor, Method::GET, None)
        .await?
        .denied()?;
    request(
        client,
        &origin,
        "/stores-api/daily-reports/summary?dateFrom=2026-10-01&dateTo=2026-10-31",
        &actor,
        Method::GET,
        None,
    )
    .await?
    .denied()?;
    pass(checks, CHECKS[8])?;

    let (stores_rows,staff_rows,daily_rows):(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM stores),(SELECT count(*) FROM store_staff),(SELECT count(*) FROM store_daily_reports)").fetch_one(business_pool).await?;
    let audits: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='DATA_GRANT_CHANGE'")
            .fetch_one(pool)
            .await?;
    let grants_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM membership_data_grants WHERE application_id=$1")
            .bind(f.app)
            .fetch_one(pool)
            .await?;
    ensure!(
        stores_rows == 6 && staff_rows >= 4 && daily_rows >= 9 && grants_count > 0 && audits > 0,
        "actual business/grant SQL evidence is incomplete"
    );
    println!(
        "STORES_LIVE_DATABASE {}",
        json!({"distinctAccounts":true,"nonSuperuser":true,"crossDatabaseConnectDenied":true,"storesRows":stores_rows,"staffRows":staff_rows,"dailyRows":daily_rows,"grantRows":grants_count,"grantAudits":audits,"grantWritesOnlyViaHttp":true})
    );
    let current_count: i64 = sqlx::query_scalar("SELECT count(*) FROM stores")
        .fetch_one(business_pool)
        .await?;
    ensure!(
        platform.stop().await,
        "platform HTTP could not stop for fail-closed test"
    );
    let failed = request(
        client,
        &origin,
        "/stores-api/stores",
        &peer,
        Method::POST,
        Some(json!({"name":"must not write","businessStatus":"OPEN","businessContact":""})),
    )
    .await?;
    ensure!(
        [401, 403, 503].contains(&failed.status),
        "platform outage did not stop business authorization"
    );
    let after_count: i64 = sqlx::query_scalar("SELECT count(*) FROM stores")
        .fetch_one(business_pool)
        .await?;
    ensure!(
        current_count == after_count,
        "platform outage permitted a business write"
    );
    for secret in secrets.iter() {
        ensure!(
            secret.is_empty() || !failed.text.contains(secret),
            "failure output exposed credential"
        );
    }
    // The final check is emitted after both actual business stdout/stderr streams
    // have been collected and checked by the outer lifecycle owner.
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires managed explicit disposable PostgreSQL and prebuilt STORES_BINARY/WORK_ORDERS_BINARY"]
async fn real_platform_stores_and_cross_app_isolation() -> Result<()> {
    let mut databases = Vec::new();
    for kind in ["PLATFORM", "STORES", "WORK_ORDERS"] {
        match Database::create(kind).await {
            Ok(database) => databases.push(database),
            Err(error) => {
                for database in databases.into_iter().rev() {
                    let _ = database.cleanup(false).await;
                }
                return Err(error);
            }
        }
    }
    let mut checks = Vec::new();
    let result=async {
        // Prove the actual runtime identities are distinct, non-superuser and
        // unable to connect to either other owned database, not just different DSNs.
        for database in &databases {
            let (user,superuser,createdb,createrole):(String,bool,bool,bool)=sqlx::query_as("SELECT current_user,rolsuper,rolcreatedb,rolcreaterole FROM pg_roles WHERE rolname=current_user").fetch_one(&database.pool).await?;
            ensure!(user==database.role && !superuser && !createdb && !createrole,"runtime role is not isolated");
            for other in &databases {
                if other.name != database.name {
                    let mut url = reqwest::Url::parse(&database.url)?;
                    url.set_path(&format!("/{}", other.name));
                    match PgPoolOptions::new().max_connections(1).acquire_timeout(Duration::from_secs(3)).connect(url.as_str()).await {
                        Ok(pool) => { pool.close().await; return Err(anyhow!("runtime role connected to another application database")); }
                        Err(error) => ensure!(error.as_database_error().and_then(|error|error.code()).as_deref() == Some("42501"), "cross-database connection failed without a PostgreSQL permission denial"),
                    }
                }
            }
        }
        tokio::time::timeout(Duration::from_secs(60),sqlx::migrate!("./migrations").run(&databases[0].pool)).await.map_err(|_|anyhow!("platform migrations timed out"))?.map_err(|_|anyhow!("owned platform migrations failed"))?;
        let fixture=Fixture::prepare(&databases[0].pool).await?;
        let state=AppState::new(databases[0].pool.clone(),Arc::new(config(&fixture.policy)?));
        let mut server=Server::start(crate::routes::api_router().nest("/oauth",crate::routes::oauth2::oauth_router()).with_state(state)).await?;
        let mut secrets=vec![SECRET.to_string(),PASSWORD.to_string(),fixture.client_secret.clone(),fixture.work_client_secret.clone()];
        for database in &databases { secrets.push(database.url.clone()); if let Some(password)=reqwest::Url::parse(&database.url)?.password(){ secrets.push(password.into()); } }
        let mut stores=match Process::start("STORES",&databases[1],&server.origin,&fixture.client_id,&fixture.client_secret).await { Ok(process)=>process,Err(error)=>{let _=server.stop().await; return Err(error);} };
        let mut work=match Process::start("WORK_ORDERS",&databases[2],&server.origin,&fixture.work_client_id,&fixture.work_client_secret).await { Ok(process)=>process,Err(error)=>{let _=stores.stop(&secrets).await; let _=server.stop().await; return Err(error);} };
        let client=Client::builder().timeout(Duration::from_secs(15)).redirect(reqwest::redirect::Policy::none()).no_proxy().build()?;
        let assertions=async { stores.ready(&client,"/stores-api").await?; work.ready(&client,"/work-orders-api").await?; assertions(&client,&mut server,&mut stores,&mut work,&databases,&fixture,&mut checks,&mut secrets).await }.await;
        let stores_stop=stores.stop(&secrets).await; let work_stop=work.stop(&secrets).await;
        let platform_stopped=if server.stop.is_none(){server.task.is_finished()}else{server.stop().await};
        let stores_stopped=stores_stop.as_ref().is_ok_and(|v|*v); let work_stopped=work_stop.as_ref().is_ok_and(|v|*v);
        println!("STORES_LIVE_PROCESS_CLEANUP {}",json!({"storesStopped":stores_stopped,"workOrdersStopped":work_stopped,"platformStopped":platform_stopped}));
        assertions?; stores_stop?; work_stop?;
        ensure!(stores_stopped && work_stopped && platform_stopped,"owned service shutdown failed");
        pass(&mut checks,CHECKS[15])?;
        ensure!(checks.len()==CHECKS.len(),"mandatory named stores checks missing");
        Ok::<_,anyhow::Error>(())
    }.await;
    let assertions_ok = result.is_ok();
    let mut cleanup_ok = true;
    for database in databases.into_iter().rev() {
        if database.cleanup(assertions_ok).await.is_err() {
            cleanup_ok = false;
        }
    }
    result?;
    ensure!(cleanup_ok, "owned database/role cleanup incomplete");
    println!(
        "STORES_LIVE_RESULT {}",
        json!({"schemaVersion":1,"transport":"explicit-development-loopback-http","passed":checks.len(),"failed":0,"skipped":0,"checks":checks})
    );
    Ok(())
}
