//! Atomic management audit regressions on disposable PostgreSQL databases.
//! Requires explicit TEST_DATABASE_URL; never reads .env or production settings.
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use axum::{middleware, Router};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tower::ServiceExt;
use uuid::Uuid;

use crate::config::Config;
use crate::entity_creation_tests::Database;
use crate::routes;
use crate::state::AppState;
use crate::util::{hash, jwt};

const SECRET: &str = "owned-management-audit-regression-only";
const PASSWORD: &str = "Owned-audit-password-123!";
const NEW_PASSWORD: &str = "Replacement-audit-password-456!";
const CLIENT_SECRET: &str = "owned-audit-client-secret";

struct Reply {
    status: StatusCode,
    body: Value,
    request_id: Uuid,
}

type SuccessfulAudit = (
    String,
    Option<Uuid>,
    String,
    Option<Uuid>,
    Option<String>,
    Option<String>,
);

struct Operation {
    method: Method,
    path: String,
    body: Value,
    action: &'static str,
    token: Option<String>,
    name: Option<String>,
    preflight_commits: bool,
    form: bool,
    raw: Option<Vec<u8>>,
    audit_rows: usize,
}

impl Operation {
    fn new(method: Method, path: impl Into<String>, body: Value, action: &'static str) -> Self {
        Self {
            method,
            path: path.into(),
            body,
            action,
            token: None,
            name: None,
            preflight_commits: false,
            form: false,
            raw: None,
            audit_rows: 1,
        }
    }
    fn named(mut self, name: &str) -> Self {
        self.name = Some(name.into());
        self
    }
    fn token(mut self, token: &str) -> Self {
        self.token = Some(token.into());
        self
    }
    fn preflight(mut self) -> Self {
        self.preflight_commits = true;
        self
    }
    fn form(mut self) -> Self {
        self.form = true;
        self
    }
    fn binary(mut self, raw: Vec<u8>) -> Self {
        self.raw = Some(raw);
        self
    }
    fn fanout(mut self) -> Self {
        self.audit_rows = 2;
        self
    }
}

struct Harness {
    pool: PgPool,
    state: AppState,
    app: Router,
    tenant: Uuid,
    root: Uuid,
    child: Uuid,
    actor: Uuid,
    admin: String,
    application: Uuid,
    menu: Uuid,
    secrets: Mutex<Vec<String>>,
}

async fn session(pool: &PgPool, tenant: Uuid, org: Uuid, user: Uuid) -> Result<String> {
    let version: i64 = sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
        .bind(user)
        .fetch_one(pool)
        .await?;
    let sid: Uuid = sqlx::query_scalar(
        "INSERT INTO platform_sessions(user_id,tenant_id,organization_id,expires_at) \
         VALUES($1,$2,$3,now()+interval '1 hour') RETURNING id",
    )
    .bind(user)
    .bind(tenant)
    .bind(org)
    .fetch_one(pool)
    .await?;
    let now = chrono::Utc::now().timestamp();
    jwt::sign(
        &jwt::Claims {
            sub: user.to_string(),
            typ: "session".into(),
            tenant: Some(tenant.to_string()),
            org: Some(org.to_string()),
            cid: None,
            scope: None,
            sid: Some(sid.to_string()),
            ver: Some(version),
            aud: None,
            jti: None,
            iat: now,
            exp: now + 3600,
        },
        SECRET,
    )
}

impl Harness {
    async fn create(pool: &PgPool) -> Result<Self> {
        let tenant: Uuid = sqlx::query_scalar("INSERT INTO tenants(name,code,is_platform) VALUES('Audit platform','admin-tenant',true) RETURNING id")
            .fetch_one(pool).await?;
        let root: Uuid = sqlx::query_scalar(
            "INSERT INTO organizations(tenant_id,name) VALUES($1,'Audit root') RETURNING id",
        )
        .bind(tenant)
        .fetch_one(pool)
        .await?;
        let child: Uuid = sqlx::query_scalar("INSERT INTO organizations(tenant_id,parent_id,name) VALUES($1,$2,'Audit child') RETURNING id")
            .bind(tenant).bind(root).fetch_one(pool).await?;
        let settings = Config::from_values(&HashMap::from([
            ("APPLICATION_ENVIRONMENT".into(), "development".into()),
            (
                "DATABASE_URL".into(),
                "postgres://unused/management-audit".into(),
            ),
            ("JWT_SECRET".into(), SECRET.into()),
        ]))?;
        let state = AppState::new(pool.clone(), Arc::new(settings));
        let password_hash = state.passwords.hash(PASSWORD).await?;
        let actor: Uuid = sqlx::query_scalar("INSERT INTO users(username,password_hash,is_super_admin) VALUES('audit-actor',$1,true) RETURNING id")
            .bind(&password_hash).fetch_one(pool).await?;
        sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)")
            .bind(actor)
            .bind(root)
            .execute(pool)
            .await?;
        let application: Uuid = sqlx::query_scalar("INSERT INTO applications(name,code,base_path,entry_type) VALUES('Audit management','system','/micro/system','HTML') RETURNING id")
            .fetch_one(pool).await?;
        let mut menu = None;
        for (index, (permission, path)) in [
            ("system:role", "/system/roles"),
            ("system:user", "/system/users"),
            ("system:tenant", "/system/tenants"),
            ("system:organization", "/system/organizations"),
            ("system:authorization", "/system/authorizations"),
            ("system:application", "/system/applications"),
            ("system:menu", "/system/menus"),
            ("system:oauth", "/system/oauth/clients"),
            ("system:oauth", "/system/oauth/tokens"),
            ("system:settings", "/system/settings"),
            ("system:department", "/system/departments"),
            ("system:region", "/system/regions"),
            ("notification:publish", "/micro/system"),
        ]
        .into_iter()
        .enumerate()
        {
            let id: Uuid = sqlx::query_scalar("INSERT INTO menus(application_id,title,permission,type,path) VALUES($1,$2,$3,'MENU',$4) RETURNING id")
                .bind(application).bind(format!("Audit permission {index}")).bind(permission).bind(path).fetch_one(pool).await?;
            menu = Some(id);
        }
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
            .bind(tenant)
            .bind(application)
            .execute(pool)
            .await?;
        for org in [root, child] {
            sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)")
                .bind(org).bind(application).execute(pool).await?;
            sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) SELECT $1,id FROM menus WHERE application_id=$2")
                .bind(org).bind(application).execute(pool).await?;
        }
        sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) SELECT $1,id FROM menus WHERE application_id=$2")
            .bind(tenant).bind(application).execute(pool).await?;
        sqlx::query("INSERT INTO membership_data_grants(membership_id,application_id,resource_code,action_code,scope_type) SELECT id,$2,'notification','publish','TENANT' FROM memberships WHERE user_id=$1 AND organization_id=$3")
            .bind(actor).bind(application).bind(root).execute(pool).await?;
        sqlx::query("INSERT INTO oauth_clients(client_id,client_name,client_secret_hash,application_id,grant_types,scopes,redirect_uris) VALUES('audit-client','Audit fixture client',$1,$2,'authorization_code,password,refresh_token','profile','https://consumer.example.test/callback')")
            .bind(hash::sha256_hex(CLIENT_SECRET)).bind(application).execute(pool).await?;
        sqlx::raw_sql(
            "CREATE TABLE phase13_audit_fault(action text PRIMARY KEY,mode text NOT NULL,after_first boolean NOT NULL DEFAULT false); \
             CREATE FUNCTION phase13_reject_audit() RETURNS trigger LANGUAGE plpgsql AS $$ \
             DECLARE fault_mode text; after_first boolean; BEGIN \
               SELECT f.mode,f.after_first INTO fault_mode,after_first FROM phase13_audit_fault f WHERE f.action=NEW.action; \
               IF after_first AND NOT EXISTS(SELECT 1 FROM audit_logs WHERE request_id=NEW.request_id AND action=NEW.action) THEN RETURN NEW; END IF; \
               IF fault_mode='raise' THEN RAISE EXCEPTION 'owned audit fault' USING ERRCODE='P0001'; END IF; \
               IF fault_mode='skip' THEN RETURN NULL; END IF; RETURN NEW; END $$; \
             CREATE TRIGGER phase13_audit_fault BEFORE INSERT ON audit_logs \
             FOR EACH ROW EXECUTE FUNCTION phase13_reject_audit();",
        ).execute(pool).await?;
        let app = routes::api_router()
            .nest("/oauth", routes::oauth2::oauth_router())
            .layer(middleware::from_fn_with_state(
                state.clone(),
                crate::observability::observe,
            ))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                crate::normalize_client_ip,
            ))
            .with_state(state.clone());
        let admin = session(pool, tenant, root, actor).await?;
        Ok(Self {
            pool: pool.clone(),
            state,
            app,
            tenant,
            root,
            child,
            actor,
            admin: admin.clone(),
            application,
            menu: menu.ok_or_else(|| anyhow!("fixture menu missing"))?,
            secrets: Mutex::new(vec![
                SECRET.into(),
                PASSWORD.into(),
                NEW_PASSWORD.into(),
                CLIENT_SECRET.into(),
                password_hash,
                admin,
            ]),
        })
    }

    fn remember(&self, value: &Value) {
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    if [
                        "clientSecret",
                        "initialPassword",
                        "temporaryPassword",
                        "accessToken",
                        "refreshToken",
                        "access_token",
                        "refresh_token",
                        "device_code",
                        "user_code",
                    ]
                    .contains(&key.as_str())
                    {
                        if let Some(secret) = value.as_str().filter(|secret| !secret.is_empty()) {
                            let mut secrets = self.secrets.lock().unwrap();
                            secrets.push(secret.into());
                            secrets.push(hash::sha256_hex(secret));
                        }
                    }
                    if key == "redirectTo" {
                        if let Some(url) =
                            value.as_str().and_then(|url| reqwest::Url::parse(url).ok())
                        {
                            for (key, code) in url.query_pairs() {
                                if key == "code" && !code.is_empty() {
                                    self.secrets.lock().unwrap().push(code.into_owned());
                                }
                            }
                        }
                    }
                    self.remember(value);
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.remember(item);
                }
            }
            _ => {}
        }
    }

    async fn call(&self, op: &Operation) -> Result<Reply> {
        let id = Uuid::new_v4();
        let bearer = op.token.as_deref().unwrap_or(&self.admin);
        {
            let mut secrets = self.secrets.lock().unwrap();
            secrets.push(bearer.into());
            secrets.push(hash::sha256_hex(bearer));
        }
        let (content_type, bytes) = if let Some(raw) = &op.raw {
            ("application/octet-stream", raw.clone())
        } else if op.form {
            let fields = op
                .body
                .as_object()
                .ok_or_else(|| anyhow!("form fixture must be an object"))?;
            let encoded = fields
                .iter()
                .map(|(key, value)| {
                    format!(
                        "{}={}",
                        utf8_percent_encode(key, NON_ALPHANUMERIC),
                        utf8_percent_encode(value.as_str().unwrap_or(""), NON_ALPHANUMERIC)
                    )
                })
                .collect::<Vec<_>>()
                .join("&");
            ("application/x-www-form-urlencoded", encoded.into_bytes())
        } else {
            ("application/json", serde_json::to_vec(&op.body)?)
        };
        let request = Request::builder()
            .method(op.method.clone())
            .uri(&op.path)
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", op.token.as_deref().unwrap_or(&self.admin)),
            )
            .header(header::CONTENT_TYPE, content_type)
            .header("x-request-id", id.to_string())
            .header(header::USER_AGENT, "owned-management-audit-test")
            .extension(ConnectInfo("192.0.2.13:31013".parse::<SocketAddr>()?))
            .body(Body::from(bytes))?;
        let response =
            tokio::time::timeout(Duration::from_secs(40), self.app.clone().oneshot(request))
                .await
                .map_err(|_| anyhow!("{} management request timed out", op.action))??;
        ensure!(
            response
                .headers()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok())
                == Some(id.to_string().as_str()),
            "HTTP requestId was not preserved"
        );
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024).await?;
        let body: Value = serde_json::from_slice(&bytes)?;
        self.remember(&body);
        Ok(Reply {
            status,
            body,
            request_id: id,
        })
    }

    async fn snapshot(&self, preflight_commits: bool) -> Result<BTreeMap<String, String>> {
        let tables: Vec<String> = sqlx::query_scalar("SELECT tablename FROM pg_tables WHERE schemaname='public' AND tablename NOT LIKE 'phase13_%' AND tablename<>'_sqlx_migrations' ORDER BY tablename")
            .fetch_all(&self.pool).await?;
        let mut statements = Vec::with_capacity(tables.len());
        for table in tables {
            ensure!(
                table
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
                "unexpected fixture table identifier"
            );
            // Network preflight is a separately committed audited operation.
            // Pointer failure must not undo its diagnostics or audit record.
            if preflight_commits && table == "application_releases" {
                continue;
            }
            let filter = if preflight_commits && table == "audit_logs" {
                " WHERE r.action<>'PREFLIGHT_APPLICATION_RELEASE'"
            } else {
                ""
            };
            statements.push(format!(
                "SELECT '{table}'::text, COALESCE(jsonb_agg(to_jsonb(r) ORDER BY to_jsonb(r)::text),'[]'::jsonb)::text FROM \"{table}\" r{filter}"
            ));
        }
        if statements.is_empty() {
            return Ok(BTreeMap::new());
        }
        // One statement gives every table the same MVCC snapshot and avoids
        // dozens of database round trips for each injected audit failure.
        let rows: Vec<(String, String)> = sqlx::query_as(&statements.join(" UNION ALL "))
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().collect())
    }

    async fn safe_audits(&self) -> Result<()> {
        let rows: Vec<String> = sqlx::query_scalar("SELECT to_jsonb(a)::text FROM audit_logs a")
            .fetch_all(&self.pool)
            .await?;
        let secrets = self.secrets.lock().unwrap().clone();
        for row in rows {
            ensure!(
                !row.contains("$argon2")
                    && !row.contains("passwordHash")
                    && !row.contains("clientSecretHash")
                    && !row.contains("tokenHash")
                    && !row.contains("password_hash")
                    && !row.contains("client_secret_hash")
                    && !row.contains("token_hash"),
                "audit contains password or client credential material"
            );
            for secret in &secrets {
                ensure!(!row.contains(secret), "audit contains credential material");
            }
        }
        Ok(())
    }

    async fn atomic(&self, op: Operation) -> Result<Reply> {
        for mode in ["raise", "skip"] {
            sqlx::query(
                "INSERT INTO phase13_audit_fault(action,mode,after_first) VALUES($1,$2,$3)",
            )
            .bind(op.action)
            .bind(mode)
            .bind(op.audit_rows > 1)
            .execute(&self.pool)
            .await?;
            let before = self.snapshot(op.preflight_commits).await?;
            let failures = crate::observability::audit_write_failures();
            let reply = self.call(&op).await?;
            ensure!(
                reply.status == StatusCode::INTERNAL_SERVER_ERROR,
                "{} audit fault {mode} returned {}, expected 500",
                op.action,
                reply.status
            );
            ensure!(
                self.snapshot(op.preflight_commits).await? == before,
                "{} audit fault {mode} left committed business or audit changes",
                op.action
            );
            ensure!(
                crate::observability::audit_write_failures() == failures.saturating_add(1),
                "{} audit fault did not increment the bounded failure counter once",
                op.action
            );
            sqlx::query("DELETE FROM phase13_audit_fault WHERE action=$1")
                .bind(op.action)
                .execute(&self.pool)
                .await?;
        }
        let failures = crate::observability::audit_write_failures();
        let mut before = self.snapshot(op.preflight_commits).await?;
        before.remove("audit_logs");
        let reply = self.call(&op).await?;
        if !reply.status.is_success() && op.action == "DELETE_ORGANIZATION" {
            self.diagnose_organization_delete(&op).await?;
        }
        ensure!(
            reply.status.is_success(),
            "{} retry returned {}",
            op.action,
            reply.status
        );
        ensure!(
            crate::observability::audit_write_failures() == failures,
            "successful audit incremented its failure counter"
        );
        let mut after = self.snapshot(op.preflight_commits).await?;
        after.remove("audit_logs");
        ensure!(
            after != before,
            "{} recorded an audit without a business-state change",
            op.action
        );
        let rows: Vec<SuccessfulAudit> = sqlx::query_as(
            "SELECT detail,user_id,username,tenant_id,tenant_name,organization_name FROM audit_logs WHERE action=$1 AND request_id=$2 ORDER BY tenant_id",
        ).bind(op.action).bind(reply.request_id).fetch_all(&self.pool).await?;
        ensure!(
            rows.len() == op.audit_rows,
            "{} retry has {} matching audit records, expected {}",
            op.action,
            rows.len(),
            op.audit_rows
        );
        if op.audit_rows > 1 {
            ensure!(
                rows[0].3 != rows[1].3,
                "audit fanout duplicated the same tenant"
            );
        }
        ensure!(
            !rows[0].2.is_empty() && (rows[0].1.is_some() || op.path == "/oauth/revoke"),
            "successful audit lost actor snapshot"
        );
        if let Some(name) = &op.name {
            ensure!(
                rows[0].0.contains(name),
                "{} audit lost object name snapshot",
                op.action
            );
            if op.action == "DELETE_TENANT" {
                ensure!(
                    rows[0].4.as_deref() == Some(name),
                    "deleted tenant lost tenant_name snapshot"
                );
            }
            if op.action == "DELETE_ORGANIZATION" {
                ensure!(
                    rows[0].5.as_deref() == Some(name),
                    "deleted organization lost organization_name snapshot"
                );
            }
        }
        if op.action == "DELETE_TENANT" || op.action == "DELETE_ORGANIZATION" {
            let target: Uuid = op
                .path
                .rsplit('/')
                .next()
                .ok_or_else(|| anyhow!("delete fixture omitted id"))?
                .parse()?;
            let sql = if op.action == "DELETE_TENANT" {
                "SELECT (SELECT count(*) FROM tenants WHERE id=$1)+(SELECT count(*) FROM organizations WHERE tenant_id=$1)+(SELECT count(*) FROM roles WHERE tenant_id=$1)"
            } else {
                "SELECT (SELECT count(*) FROM organizations WHERE id=$1)+(SELECT count(*) FROM memberships WHERE organization_id=$1)+(SELECT count(*) FROM roles WHERE organization_id=$1)"
            };
            let remaining: i64 = sqlx::query_scalar(sql)
                .bind(target)
                .fetch_one(&self.pool)
                .await?;
            ensure!(
                remaining == 0,
                "{} did not remove target and dependent rows",
                op.action
            );
        }
        self.safe_audits().await?;
        eprintln!(
            "management_audit_case action={} faults=2 retry=PASS audits={}",
            op.action, op.audit_rows
        );
        Ok(reply)
    }

    async fn diagnose_organization_delete(&self, op: &Operation) -> Result<()> {
        let target: Uuid = op
            .path
            .rsplit('/')
            .next()
            .ok_or_else(|| anyhow!("delete fixture omitted id"))?
            .parse()?;
        for deferred in [false, true] {
            let mut tx = self.pool.begin().await?;
            if deferred {
                sqlx::query("SET CONSTRAINTS fk_roles_organization_same_tenant DEFERRED")
                    .execute(&mut *tx)
                    .await?;
            }
            match sqlx::query("DELETE FROM organizations WHERE id=$1")
                .bind(target)
                .execute(&mut *tx)
                .await
            {
                Ok(_) => {
                    let checked = sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
                        .execute(&mut *tx)
                        .await;
                    if let Err(error) = checked {
                        if let Some(error) = error.as_database_error() {
                            eprintln!(
                                "owned_org_delete_diagnostic deferred={} code={:?} constraint={:?}",
                                deferred,
                                error.code(),
                                error.constraint()
                            );
                        }
                    } else {
                        eprintln!(
                            "owned_org_delete_diagnostic deferred={} constraints=PASS",
                            deferred
                        );
                    }
                }
                Err(error) => {
                    if let Some(error) = error.as_database_error() {
                        eprintln!(
                            "owned_org_delete_diagnostic deferred={} code={:?} constraint={:?}",
                            deferred,
                            error.code(),
                            error.constraint()
                        );
                    }
                }
            }
            tx.rollback().await?;
        }
        Ok(())
    }

    async fn user(&self, username: &str, org: Uuid) -> Result<Uuid> {
        let password_hash = self.state.passwords.hash(PASSWORD).await?;
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO users(username,password_hash) VALUES($1,$2) RETURNING id",
        )
        .bind(username)
        .bind(password_hash)
        .fetch_one(&self.pool)
        .await?;
        sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)")
            .bind(id)
            .bind(org)
            .execute(&self.pool)
            .await?;
        Ok(id)
    }

    async fn role(&self, name: &str, org: Uuid) -> Result<Uuid> {
        Ok(sqlx::query_scalar("INSERT INTO roles(tenant_id,organization_id,name,code) VALUES($1,$2,$3,$4) RETURNING id")
            .bind(self.tenant).bind(org).bind(name).bind(format!("r-{}", Uuid::new_v4().simple())).fetch_one(&self.pool).await?)
    }
}

async fn run<F, Fut>(suite: F) -> Result<()>
where
    F: FnOnce(Harness) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    ensure!(
        std::env::var("TEST_DATABASE_URL").is_ok_and(|value| !value.trim().is_empty()),
        "explicit TEST_DATABASE_URL is required"
    );
    let database = Database::create().await?;
    let result = tokio::time::timeout(Duration::from_secs(240), async {
        let harness = Harness::create(&database.pool).await?;
        suite(harness).await
    })
    .await
    .map_err(|_| anyhow!("management audit suite deadline exceeded"))
    .and_then(|value| value);
    database.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit TEST_DATABASE_URL and disposable database privileges"]
async fn region_catalog_mutations_rollback_when_audit_is_rejected() -> Result<()> {
    run(|h| async move {
        h.atomic(Operation::new(Method::POST, "/api/regions/versions",
            json!({"version":"audit-regions","label":"Audited catalog"}),
            "CREATE_REGION_VERSION").named("Audited catalog")).await?;
        let mut body = json!({"version":"audit-regions","code":"110000","parentCode":"0",
            "name":"Audited region","namePinyin":"beijing","nameShort":"bj","level":2});
        let created = h.atomic(Operation::new(Method::POST,"/api/regions",body.clone(),
            "CREATE_REGION").named("Audited region")).await?;
        let region = created.body["id"].as_str().ok_or_else(|| anyhow!("region response omitted id"))?;
        let path = format!("/api/regions/{region}");
        body["name"] = json!("Renamed audited region");
        h.atomic(Operation::new(Method::PUT,&path,body.clone(),"UPDATE_REGION")
            .named("Renamed audited region")).await?;
        let before = h.snapshot(false).await?;
        no_event(&h,Operation::new(Method::PUT,&path,body,"UPDATE_REGION")).await?;
        ensure!(h.snapshot(false).await? == before,"region no-op changed persisted state");
        h.atomic(Operation::new(Method::POST,"/api/regions/versions",
            json!({"version":"audit-copy","label":"Copied catalog","sourceVersion":"audit-regions"}),
            "CREATE_REGION_VERSION").named("Copied catalog")).await?;
        let copies: i64 = sqlx::query_scalar("SELECT count(*) FROM regions WHERE version='audit-copy'")
            .fetch_one(&h.pool).await?;
        ensure!(copies == 1,"catalog copy lost its region");
        h.atomic(Operation::new(Method::DELETE,"/api/regions/versions/audit-copy",Value::Null,
            "DELETE_REGION_VERSION").named("Copied catalog")).await?;
        h.atomic(Operation::new(Method::DELETE,format!("{path}?version=audit-regions"),Value::Null,
            "DELETE_REGION").named("Renamed audited region")).await?;
        h.atomic(Operation::new(Method::DELETE,"/api/regions/versions/audit-regions",Value::Null,
            "DELETE_REGION_VERSION").named("Audited catalog")).await?;
        Ok(())
    }).await
}

fn id(reply: &Reply) -> Result<Uuid> {
    reply.body["id"]
        .as_str()
        .ok_or_else(|| anyhow!("response omitted object id"))?
        .parse()
        .map_err(Into::into)
}

#[tokio::test]
#[ignore = "requires explicit TEST_DATABASE_URL and disposable database privileges"]
async fn roles_and_grant_collections_rollback_when_audit_is_rejected() -> Result<()> {
    run(|h| async move {
        let created = h
            .atomic(
                Operation::new(
                    Method::POST,
                    "/api/roles",
                    json!({"name":"Audited role","code":"audited-role","organizationId":h.child}),
                    "CREATE_ROLE",
                )
                .named("Audited role"),
            )
            .await?;
        let role = id(&created)?;
        h.atomic(
            Operation::new(
                Method::PUT,
                format!("/api/roles/{role}"),
                json!({"name":"Renamed audited role"}),
                "UPDATE_ROLE",
            )
            .named("Renamed audited role"),
        )
        .await?;
        h.atomic(Operation::new(
            Method::PUT,
            format!("/api/roles/{role}/menus"),
            json!({"menuIds":[h.menu]}),
            "ROLE_MENUS",
        ))
        .await?;
        let second_menu: Uuid = sqlx::query_scalar(
            "SELECT id FROM menus WHERE application_id=$1 AND id<>$2 ORDER BY id LIMIT 1",
        )
        .bind(h.application)
        .bind(h.menu)
        .fetch_one(&h.pool)
        .await?;
        h.atomic(Operation::new(
            Method::PUT,
            format!("/api/roles/{role}/menus"),
            json!({"menuIds":[second_menu]}),
            "ROLE_MENUS",
        ))
        .await?;
        no_event(
            &h,
            Operation::new(
                Method::PUT,
                format!("/api/roles/{role}/menus"),
                json!({"menuIds":[second_menu]}),
                "ROLE_MENUS",
            ),
        )
        .await?;
        no_event(
            &h,
            Operation::new(
                Method::PUT,
                format!("/api/roles/{role}"),
                json!({"name":"Renamed audited role"}),
                "UPDATE_ROLE",
            ),
        )
        .await?;
        let user = h.user("role-target", h.child).await?;
        h.atomic(Operation::new(
            Method::PUT,
            format!("/api/roles/{role}/users"),
            json!({"userIds":[user]}),
            "UPDATE_ROLE",
        ))
        .await?;
        no_event(
            &h,
            Operation::new(
                Method::PUT,
                format!("/api/roles/{role}/users"),
                json!({"userIds":[user]}),
                "UPDATE_ROLE",
            ),
        )
        .await?;
        h.atomic(
            Operation::new(
                Method::DELETE,
                format!("/api/roles/{role}"),
                Value::Null,
                "DELETE_ROLE",
            )
            .named("Renamed audited role"),
        )
        .await?;
        Ok(())
    })
    .await
}

#[tokio::test]
#[ignore = "requires explicit TEST_DATABASE_URL and disposable database privileges"]
async fn user_security_password_and_platform_settings_mutations_are_audited_atomically(
) -> Result<()> {
    run(|h| async move {
        let created = h.atomic(Operation::new(Method::POST, "/api/users", json!({"username":"audited-user","password":PASSWORD}), "CREATE_USER")).await?;
        let user = id(&created)?;
        sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)").bind(user).bind(h.child).execute(&h.pool).await?;
        for (method, path, body, action) in [
            (Method::PUT, format!("/api/users/{user}"), json!({"realName":"Audited profile"}), "UPDATE_USER"),
            (Method::PUT, format!("/api/users/{user}/status"), json!({"status":"DISABLED"}), "UPDATE_USER"),
            (Method::POST, "/api/users/batch-status".into(), json!({"userIds":[user],"status":"ENABLED"}), "UPDATE_USER"),
            (Method::POST, format!("/api/users/{user}/reset-password"), json!({"newPassword":NEW_PASSWORD}), "RESET_PASSWORD"),
        ] {
            h.atomic(Operation::new(method.clone(),path.clone(),body.clone(),action)).await?;
            if method==Method::PUT { no_event(&h,Operation::new(method,path,body,action)).await?; }
        }
        let target_token = session(&h.pool,h.tenant,h.child,user).await?;
        h.atomic(Operation::new(Method::PUT,"/api/auth/password",json!({"oldPassword":NEW_PASSWORD,"newPassword":PASSWORD}),"UPDATE_PASSWORD").token(&target_token)).await?;
        h.atomic(Operation::new(Method::DELETE,format!("/api/users/{user}"),Value::Null,"DELETE_USER").named("audited-user")).await?;
        let session_user = h.user("audit-session-target",h.root).await?;
        sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)")
            .bind(session_user).bind(h.child).execute(&h.pool).await?;
        let current = session(&h.pool,h.tenant,h.root,session_user).await?;
        let other = session(&h.pool,h.tenant,h.root,session_user).await?;
        let other_sid = jwt::verify(&other,SECRET)?.sid.ok_or_else(||anyhow!("session fixture omitted sid"))?;
        h.atomic(Operation::new(Method::DELETE,format!("/api/auth/sessions/{other_sid}"),Value::Null,"REVOKE_SESSION").token(&current)).await?;
        let switched = h.atomic(Operation::new(Method::POST,"/api/auth/context",json!({"organizationId":h.child}),"CONTEXT_SWITCH").token(&current)).await?;
        let switched_token = switched.body["accessToken"].as_str().ok_or_else(||anyhow!("context switch omitted access token"))?;
        h.atomic(Operation::new(Method::POST,"/api/auth/logout",Value::Null,"LOGOUT").token(switched_token)).await?;
        let settings=json!({
            "platformName":"Audited platform branding","platformDescription":"Audit regression",
            "logoUrl":"/audit-logo.svg","loginTitle":"Audit login","loginDescription":"Owned fixture",
            "footerText":"Audit footer","platformTenantName":"Audited platform name","primaryColor":"#123456",
        });
        h.atomic(Operation::new(Method::PUT,"/api/platform/settings",settings.clone(),"UPDATE_PLATFORM_SETTINGS")).await?;
        no_event(&h,Operation::new(Method::PUT,"/api/platform/settings",settings,"UPDATE_PLATFORM_SETTINGS")).await?;
        Ok(())
    }).await
}

#[tokio::test]
#[ignore = "requires explicit TEST_DATABASE_URL and disposable database privileges"]
async fn membership_and_user_role_replacements_rollback_sessions_and_grants_together() -> Result<()>
{
    run(|h| async move {
        let user = h.user("membership-target", h.root).await?;
        let _target_token = session(&h.pool, h.tenant, h.root, user).await?;
        let role = h.role("Membership role", h.root).await?;
        h.atomic(Operation::new(
            Method::PUT,
            format!("/api/users/{user}/roles"),
            json!({"items":[{"roleId":role,"organizationId":h.root}]}),
            "UPDATE_USER",
        ))
        .await?;
        let other_role = h.role("Replacement membership role", h.root).await?;
        h.atomic(Operation::new(
            Method::PUT,
            format!("/api/users/{user}/roles"),
            json!({"items":[{"roleId":other_role,"organizationId":h.root}]}),
            "UPDATE_USER",
        ))
        .await?;
        no_event(
            &h,
            Operation::new(
                Method::PUT,
                format!("/api/users/{user}/roles"),
                json!({"items":[{"roleId":other_role,"organizationId":h.root}]}),
                "UPDATE_USER",
            ),
        )
        .await?;
        h.atomic(Operation::new(
            Method::PUT,
            format!("/api/users/{user}/memberships"),
            json!({"items":[{"organizationId":h.child,"position":"Audited member"}]}),
            "UPDATE_USER",
        ))
        .await?;
        no_event(
            &h,
            Operation::new(
                Method::PUT,
                format!("/api/users/{user}/memberships"),
                json!({"items":[{"organizationId":h.child,"position":"Audited member"}]}),
                "UPDATE_USER",
            ),
        )
        .await?;
        let other_tenant: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(name,code) VALUES('Fanout tenant','audit-fanout') RETURNING id",
        )
        .fetch_one(&h.pool)
        .await?;
        let other_org: Uuid = sqlx::query_scalar(
            "INSERT INTO organizations(tenant_id,name) VALUES($1,'Fanout root') RETURNING id",
        )
        .bind(other_tenant)
        .fetch_one(&h.pool)
        .await?;
        sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)")
            .bind(user)
            .bind(other_org)
            .execute(&h.pool)
            .await?;
        h.atomic(
            Operation::new(
                Method::PUT,
                format!("/api/users/{user}"),
                json!({"realName":"Cross-tenant audited profile"}),
                "UPDATE_USER",
            )
            .fanout(),
        )
        .await?;
        Ok(())
    })
    .await
}

#[tokio::test]
#[ignore = "requires explicit TEST_DATABASE_URL and disposable database privileges"]
async fn tenant_organization_authorizations_initialization_and_cascades_are_atomic() -> Result<()> {
    run(tenant_organization_matrix).await
}

#[tokio::test]
#[ignore = "requires explicit TEST_DATABASE_URL and disposable database privileges"]
async fn oauth_client_secrets_revocations_kicks_and_purge_are_atomic() -> Result<()> {
    run(oauth_matrix).await
}

#[tokio::test]
#[ignore = "requires explicit TEST_DATABASE_URL and disposable database privileges"]
async fn application_and_menu_cascades_keep_snapshots_and_atomic_audit() -> Result<()> {
    run(application_matrix).await
}

#[tokio::test]
#[ignore = "requires explicit TEST_DATABASE_URL and disposable database privileges"]
async fn release_preflight_registration_and_pointer_audits_keep_existing_transaction_boundaries(
) -> Result<()> {
    run(release_matrix).await
}

#[tokio::test]
#[ignore = "requires explicit TEST_DATABASE_URL and disposable database privileges"]
async fn notification_publication_audit_failure_rolls_back_delivery_and_retry_is_idempotent(
) -> Result<()> {
    run(notification_matrix).await
}

async fn tenant_organization_matrix(h: Harness) -> Result<()> {
    let created = h
        .atomic(
            Operation::new(
                Method::POST,
                "/api/tenants",
                json!({"name":"Audited tenant","code":"audited-tenant"}),
                "CREATE_TENANT",
            )
            .named("Audited tenant"),
        )
        .await?;
    let tenant = id(&created)?;
    let root: Uuid =
        sqlx::query_scalar("SELECT id FROM organizations WHERE tenant_id=$1 AND parent_id IS NULL")
            .bind(tenant)
            .fetch_one(&h.pool)
            .await?;
    h.atomic(Operation::new(Method::PUT,format!("/api/tenants/{tenant}"),json!({"name":"Renamed audited tenant","contactName":"Tenant contact","contactEmail":"tenant-contact@example.test"}),"UPDATE_TENANT").named("Renamed audited tenant")).await?;
    no_event(&h,Operation::new(Method::PUT,format!("/api/tenants/{tenant}"),json!({"name":"Renamed audited tenant","contactName":"Tenant contact","contactEmail":"tenant-contact@example.test"}),"UPDATE_TENANT")).await?;
    h.atomic(Operation::new(
        Method::POST,
        format!("/api/tenants/{tenant}/initialize-administrator"),
        Value::Null,
        "INITIALIZE_ADMINISTRATOR",
    ))
    .await?;
    no_event(
        &h,
        Operation::new(
            Method::POST,
            format!("/api/tenants/{tenant}/initialize-administrator"),
            Value::Null,
            "INITIALIZE_ADMINISTRATOR",
        ),
    )
    .await?;
    // Equal-size replacements must compare membership, not only item counts.
    let old_menu: Uuid = sqlx::query_scalar(
        "INSERT INTO menus(application_id,title,permission,type) VALUES($1,'Audit tenant old grant','audit:tenant-old','BUTTON') RETURNING id",
    )
    .bind(h.application)
    .fetch_one(&h.pool)
    .await?;
    let new_menu: Uuid = sqlx::query_scalar(
        "INSERT INTO menus(application_id,title,permission,type) VALUES($1,'Audit tenant new grant','audit:tenant-new','BUTTON') RETURNING id",
    )
    .bind(h.application)
    .fetch_one(&h.pool)
    .await?;
    h.atomic(Operation::new(
        Method::PUT,
        format!("/api/tenants/{tenant}/authorizations"),
        json!({
            "applications":[{"applicationId":h.application,"authorized":true}],
            "menus":[{"menuId":h.menu,"authorized":true},{"menuId":old_menu,"authorized":true}],
        }),
        "TENANT_AUTH",
    ))
    .await?;
    let before_grants: Vec<Uuid> =
        sqlx::query_scalar("SELECT menu_id FROM tenant_menus WHERE tenant_id=$1 ORDER BY menu_id")
            .bind(tenant)
            .fetch_all(&h.pool)
            .await?;
    ensure!(
        before_grants.contains(&old_menu) && !before_grants.contains(&new_menu),
        "equal-size tenant grant fixture was already a no-op"
    );
    // The API applies explicit changes, so revoke the old ID as well as grant
    // its replacement. Omitted grants intentionally remain untouched.
    let tenant_grants = json!({"applications":[{"applicationId":h.application,"authorized":true}],"menus":[{"menuId":old_menu,"authorized":false},{"menuId":new_menu,"authorized":true}]});
    h.atomic(Operation::new(
        Method::PUT,
        format!("/api/tenants/{tenant}/authorizations"),
        tenant_grants.clone(),
        "TENANT_AUTH",
    ))
    .await?;
    let after_grants: Vec<Uuid> =
        sqlx::query_scalar("SELECT menu_id FROM tenant_menus WHERE tenant_id=$1 ORDER BY menu_id")
            .bind(tenant)
            .fetch_all(&h.pool)
            .await?;
    ensure!(
        before_grants.len() == after_grants.len()
            && before_grants != after_grants
            && !after_grants.contains(&old_menu)
            && after_grants.contains(&new_menu),
        "tenant authorization did not replace equal-sized grant sets"
    );
    no_event(
        &h,
        Operation::new(
            Method::PUT,
            format!("/api/tenants/{tenant}/authorizations"),
            tenant_grants,
            "TENANT_AUTH",
        ),
    )
    .await?;
    let organization = h
        .atomic(
            Operation::new(
                Method::POST,
                "/api/organizations",
                json!({"tenantId":tenant,"parentId":root,"name":"Audited organization"}),
                "CREATE_ORGANIZATION",
            )
            .named("Audited organization"),
        )
        .await?;
    let org = id(&organization)?;
    h.atomic(Operation::new(Method::PUT,format!("/api/organizations/{org}"),json!({"name":"Renamed audited organization","contactName":"Org contact","contactEmail":"org-contact@example.test"}),"UPDATE_ORGANIZATION").named("Renamed audited organization")).await?;
    no_event(
        &h,
        Operation::new(
            Method::PUT,
            format!("/api/organizations/{org}"),
            json!({"name":"Renamed audited organization"}),
            "UPDATE_ORGANIZATION",
        ),
    )
    .await?;
    h.atomic(Operation::new(
        Method::POST,
        format!("/api/organizations/{org}/initialize-administrator"),
        Value::Null,
        "INITIALIZE_ADMINISTRATOR",
    ))
    .await?;
    no_event(
        &h,
        Operation::new(
            Method::POST,
            format!("/api/organizations/{org}/initialize-administrator"),
            Value::Null,
            "INITIALIZE_ADMINISTRATOR",
        ),
    )
    .await?;
    h.atomic(Operation::new(Method::PUT,format!("/api/organizations/{org}/authorizations"),json!({
        "applications":[{"applicationId":h.application,"authorized":true}],"menus":[{"menuId":h.menu,"authorized":true}],
    }),"ORG_AUTH")).await?;
    no_event(&h,Operation::new(Method::PUT,format!("/api/organizations/{org}/authorizations"),json!({"applications":[{"applicationId":h.application,"authorized":true}],"menus":[{"menuId":h.menu,"authorized":true}]}),"ORG_AUTH")).await?;
    assert_organization_grant_rechecks_tenant_after_lock_wait(&h, tenant, org).await?;
    let grandchild:Uuid=sqlx::query_scalar("INSERT INTO organizations(tenant_id,parent_id,name) VALUES($1,$2,'Audit cascading grandchild') RETURNING id")
        .bind(tenant).bind(org).fetch_one(&h.pool).await?;
    let cascade_role:Uuid=sqlx::query_scalar("INSERT INTO roles(tenant_id,organization_id,name,code) VALUES($1,$2,'Cascade role','audit-cascade-role') RETURNING id")
        .bind(tenant).bind(grandchild).fetch_one(&h.pool).await?;
    let department:Uuid=sqlx::query_scalar("INSERT INTO departments(organization_id,name) VALUES($1,'Cascade parent department') RETURNING id").bind(grandchild).fetch_one(&h.pool).await?;
    let child_department:Uuid=sqlx::query_scalar("INSERT INTO departments(organization_id,parent_id,name) VALUES($1,$2,'Cascade child department') RETURNING id")
        .bind(grandchild).bind(department).fetch_one(&h.pool).await?;
    let member = h.user("cascading-grandchild-member", grandchild).await?;
    sqlx::query("UPDATE memberships SET department_id=$2 WHERE user_id=$1 AND organization_id=$3")
        .bind(member)
        .bind(child_department)
        .bind(grandchild)
        .execute(&h.pool)
        .await?;
    sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3)")
        .bind(member)
        .bind(cascade_role)
        .bind(grandchild)
        .execute(&h.pool)
        .await?;
    for (path, action) in [
        (
            format!("/api/organizations/{org}/status"),
            "UPDATE_ORGANIZATION",
        ),
        (format!("/api/tenants/{tenant}/status"), "UPDATE_TENANT"),
    ] {
        h.atomic(Operation::new(
            Method::PUT,
            path,
            json!({"status":"DISABLED"}),
            action,
        ))
        .await?;
    }
    h.atomic(
        Operation::new(
            Method::DELETE,
            format!("/api/organizations/{org}"),
            Value::Null,
            "DELETE_ORGANIZATION",
        )
        .named("Renamed audited organization"),
    )
    .await?;
    let cascades:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM organizations WHERE id=$1)+(SELECT count(*) FROM departments WHERE organization_id=$1)+(SELECT count(*) FROM memberships WHERE organization_id=$1)+(SELECT count(*) FROM roles WHERE organization_id=$1)+(SELECT count(*) FROM user_roles WHERE organization_id=$1)")
        .bind(grandchild).fetch_one(&h.pool).await?;
    ensure!(
        cascades == 0,
        "organization subtree deletion retained grandchild grants, departments or members"
    );
    h.atomic(
        Operation::new(
            Method::DELETE,
            format!("/api/tenants/{tenant}"),
            Value::Null,
            "DELETE_TENANT",
        )
        .named("Renamed audited tenant"),
    )
    .await?;
    Ok(())
}

async fn assert_organization_grant_rechecks_tenant_after_lock_wait(
    h: &Harness,
    tenant: Uuid,
    org: Uuid,
) -> Result<()> {
    let mut blocker = h.pool.begin().await?;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await?;
    sqlx::query("SELECT id FROM tenants WHERE id=$1 FOR NO KEY UPDATE")
        .bind(tenant)
        .execute(&mut *blocker)
        .await?;
    let op = Operation::new(
        Method::PUT,
        format!("/api/organizations/{org}/authorizations"),
        json!({
            "applications":[{"applicationId":h.application,"authorized":true}],"menus":[{"menuId":h.menu,"authorized":true}],
        }),
        "ORG_AUTH",
    );
    let control = async {
        if let Err(error) = wait_for_blocker(&h.pool, pid, "tenants").await {
            blocker.rollback().await?;
            return Err(error);
        }
        sqlx::query("DELETE FROM organization_menus WHERE organization_id=$1")
            .bind(org)
            .execute(&mut *blocker)
            .await?;
        sqlx::query(
            "DELETE FROM organization_applications WHERE organization_id=$1 AND application_id=$2",
        )
        .bind(org)
        .bind(h.application)
        .execute(&mut *blocker)
        .await?;
        sqlx::query("DELETE FROM tenant_menus WHERE tenant_id=$1 AND menu_id IN(SELECT id FROM menus WHERE application_id=$2)").bind(tenant).bind(h.application).execute(&mut *blocker).await?;
        sqlx::query("DELETE FROM tenant_applications WHERE tenant_id=$1 AND application_id=$2")
            .bind(tenant)
            .bind(h.application)
            .execute(&mut *blocker)
            .await?;
        blocker.commit().await?;
        Ok(())
    };
    let (reply, controlled) = tokio::join!(h.call(&op), control);
    controlled?;
    let reply = reply?;
    ensure!(
        [StatusCode::BAD_REQUEST, StatusCode::FORBIDDEN].contains(&reply.status),
        "org authorization did not reject the tenant grant withdrawn during lock wait"
    );
    let restored: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM organization_applications WHERE organization_id=$1 AND application_id=$2)")
        .bind(org).bind(h.application).fetch_one(&h.pool).await?;
    ensure!(
        !restored,
        "org authorization resurrected a withdrawn tenant grant"
    );
    let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE request_id=$1")
        .bind(reply.request_id)
        .fetch_one(&h.pool)
        .await?;
    ensure!(
        audits == 0,
        "rejected stale org authorization wrote success audit"
    );
    eprintln!("management_audit_special tenant_parent_lock_revalidation=PASS");
    Ok(())
}
async fn seed_token(h: &Harness, user: Uuid, expired: bool) -> Result<(Uuid, String)> {
    let token = format!("owned-audit-{}", Uuid::new_v4().simple());
    h.secrets.lock().unwrap().push(token.clone());
    let expiry = if expired {
        "now()-interval '10 days'"
    } else {
        "now()+interval '1 hour'"
    };
    let id = sqlx::query_scalar(&format!(
        "INSERT INTO oauth_tokens(token_hash,token_type,user_id,client_id,tenant_id,organization_id,scope,expires_at) \
         VALUES($1,'access',$2,'audit-client',$3,$4,'profile',{expiry}) RETURNING id",
    )).bind(hash::sha256_hex(&token)).bind(user).bind(h.tenant).bind(h.root).fetch_one(&h.pool).await?;
    Ok((id, token))
}

async fn seed_device(h: &Harness, user_code: &str) -> Result<Uuid> {
    h.secrets.lock().unwrap().push(user_code.into());
    Ok(sqlx::query_scalar("INSERT INTO oauth_device_codes(device_code_hash,user_code,client_id,scope,expires_at) VALUES($1,$2,'audit-client','profile',now()+interval '1 hour') RETURNING id")
        .bind(hash::sha256_hex(&Uuid::new_v4().to_string())).bind(user_code).fetch_one(&h.pool).await?)
}

#[tokio::test]
#[ignore = "requires explicit TEST_DATABASE_URL and disposable database privileges"]
async fn oauth_pkce_management_defaults_explicit_overrides_and_audit_rollback() -> Result<()> {
    run(|h| async move {
        let body = |name: &str, kind: &str| json!({
            "clientName":name,"clientType":kind,"grantTypes":["authorization_code","refresh_token"],
            "scopes":["profile"],"redirectUris":["https://pkce.example.test/callback"]
        });
        let public = h.atomic(Operation::new(Method::POST,"/api/oauth/clients",
            body("New public PKCE client","PUBLIC"),"CREATE_CLIENT")).await?;
        ensure!(public.body["requirePkce"] == true,"new PUBLIC client did not default to required PKCE");
        let confidential = h.atomic(Operation::new(Method::POST,"/api/oauth/clients",
            body("New confidential PKCE client","CONFIDENTIAL"),"CREATE_CLIENT")).await?;
        ensure!(confidential.body["requirePkce"] == false,"new CONFIDENTIAL client changed its compatibility default");
        for (kind, required) in [("PUBLIC",false),("CONFIDENTIAL",true)] {
            let mut explicit = body("Explicit PKCE client",kind);
            explicit["requirePkce"] = json!(required);
            let reply = h.call(&Operation::new(Method::POST,"/api/oauth/clients",explicit,"CREATE_CLIENT")).await?;
            ensure!(reply.status.is_success() && reply.body["requirePkce"] == required,
                "client creation ignored an explicit PKCE policy");
        }

        // Direct insertion models an existing client after the additive column
        // migration; managed creation intentionally has different defaults.
        let legacy: Uuid = sqlx::query_scalar(
            "INSERT INTO oauth_clients(client_id,client_name,client_type,grant_types,scopes) \
             VALUES('legacy-pkce-management','Legacy public PKCE client','PUBLIC','authorization_code','profile') RETURNING id",
        ).fetch_one(&h.pool).await?;
        let listed = h.call(&Operation::new(Method::GET,"/api/oauth/clients",Value::Null,"LIST_CLIENTS")).await?;
        ensure!(listed.status == StatusCode::OK,"client listing failed");
        let legacy_row = listed.body.as_array().and_then(|rows| rows.iter().find(|row| row["id"] == legacy.to_string()))
            .ok_or_else(|| anyhow!("client listing omitted historical PUBLIC client"))?;
        ensure!(legacy_row["requirePkce"] == false,"client listing hid historical PKCE compatibility mode");
        let preserved = h.atomic(Operation::new(Method::PUT,format!("/api/oauth/clients/{legacy}"),
            json!({"clientName":"Renamed legacy PKCE client"}),"UPDATE_CLIENT")).await?;
        ensure!(preserved.body["requirePkce"] == false,"unrelated edit tightened historical PUBLIC policy implicitly");

        let client = id(&confidential)?;
        let path = format!("/api/oauth/clients/{client}");
        let switched = h.atomic(Operation::new(Method::PUT,&path,json!({"clientType":"PUBLIC"}),"UPDATE_CLIENT")).await?;
        ensure!(switched.body["requirePkce"] == true,"switching to PUBLIC did not default to required PKCE");
        let detail: Value = sqlx::query_scalar("SELECT detail::jsonb FROM audit_logs WHERE request_id=$1 AND action='UPDATE_CLIENT'")
            .bind(switched.request_id).fetch_one(&h.pool).await?;
        ensure!(detail["before"]["requirePkce"] == false && detail["after"]["requirePkce"] == true,
            "PKCE policy change was omitted from its management audit");

        // atomic() rejects both throwing and silently skipped audit writes and
        // compares every business table before retrying, including this policy.
        let disabled = h.atomic(Operation::new(Method::PUT,&path,json!({"requirePkce":false}),"UPDATE_CLIENT")).await?;
        ensure!(disabled.body["requirePkce"] == false,"explicit compatibility override was ignored");
        let back = h.atomic(Operation::new(Method::PUT,&path,json!({"clientType":"CONFIDENTIAL"}),"UPDATE_CLIENT")).await?;
        ensure!(back.body["requirePkce"] == false,"type edit silently changed an existing policy");
        let overridden = h.atomic(Operation::new(Method::PUT,&path,
            json!({"clientType":"PUBLIC","requirePkce":false}),"UPDATE_CLIENT")).await?;
        ensure!(overridden.body["requirePkce"] == false,"explicit false did not override the PUBLIC transition default");
        let stored: bool = sqlx::query_scalar("SELECT require_pkce FROM oauth_clients WHERE id=$1")
            .bind(client).fetch_one(&h.pool).await?;
        ensure!(!stored,"client response and persisted PKCE policy disagree");
        Ok(())
    }).await
}

async fn oauth_matrix(h: Harness) -> Result<()> {
    let created = h.atomic(Operation::new(Method::POST,"/api/oauth/clients",json!({"clientName":"Audited client","grantTypes":["authorization_code","refresh_token"],"scopes":["profile"],"redirectUris":["https://consumer.example.test/callback"]}),"CREATE_CLIENT").named("Audited client")).await?;
    let client = id(&created)?;
    h.atomic(
        Operation::new(
            Method::PUT,
            format!("/api/oauth/clients/{client}"),
            json!({"clientName":"Renamed audited client"}),
            "UPDATE_CLIENT",
        )
        .named("Renamed audited client"),
    )
    .await?;
    no_event(
        &h,
        Operation::new(
            Method::PUT,
            format!("/api/oauth/clients/{client}"),
            json!({"clientName":"Renamed audited client"}),
            "UPDATE_CLIENT",
        ),
    )
    .await?;
    h.atomic(Operation::new(
        Method::POST,
        format!("/api/oauth/clients/{client}/regenerate-secret"),
        Value::Null,
        "UPDATE_CLIENT",
    ))
    .await?;
    let target = h.user("oauth-security-target", h.root).await?;
    let (revoked_id, _) = seed_token(&h, target, false).await?;
    h.atomic(Operation::new(
        Method::DELETE,
        format!("/api/oauth/tokens/{revoked_id}"),
        Value::Null,
        "TOKEN_REVOKED",
    ))
    .await?;
    let _session = session(&h.pool, h.tenant, h.root, target).await?;
    seed_token(&h, target, false).await?;
    h.atomic(Operation::new(
        Method::POST,
        "/api/oauth/tokens/kick",
        json!({"userId":target}),
        "KICK_SESSIONS",
    ))
    .await?;
    let device = seed_device(&h, "AUDIT-ADMIN-CODE").await?;
    h.atomic(Operation::new(
        Method::PUT,
        format!("/api/oauth/device-codes/{device}/revoke"),
        Value::Null,
        "DEVICE_DENY",
    ))
    .await?;
    seed_token(&h, target, true).await?;
    h.atomic(Operation::new(
        Method::POST,
        "/api/oauth/tokens/purge",
        Value::Null,
        "OAUTH_PURGE",
    ))
    .await?;
    no_event(
        &h,
        Operation::new(
            Method::POST,
            "/api/oauth/tokens/purge",
            Value::Null,
            "OAUTH_PURGE",
        ),
    )
    .await?;
    h.atomic(
        Operation::new(
            Method::DELETE,
            format!("/api/oauth/clients/{client}"),
            Value::Null,
            "DELETE_CLIENT",
        )
        .named("Renamed audited client"),
    )
    .await?;
    h.atomic(Operation::new(
        Method::POST,
        "/api/oauth/session-token",
        json!({"appCode":"system"}),
        "TOKEN_ISSUED",
    ))
    .await?;
    for approve in [true, false] {
        let request: Uuid = sqlx::query_scalar("INSERT INTO oauth_authorization_requests(client_id,redirect_uri,scope,expires_at) VALUES('audit-client','https://consumer.example.test/callback','profile',now()+interval '5 minutes') RETURNING id")
            .fetch_one(&h.pool).await?;
        let action = if approve {
            "CONSENT_APPROVE"
        } else {
            "CONSENT_DENY"
        };
        h.atomic(Operation::new(
            Method::POST,
            format!("/api/oauth/consent/{request}/decision"),
            json!({"approve":approve}),
            action,
        ))
        .await?;
        let code = if approve {
            "AUDIT-APPROVE-CODE"
        } else {
            "AUDIT-DENY-CODE"
        };
        seed_device(&h, code).await?;
        let action = if approve {
            "DEVICE_APPROVE"
        } else {
            "DEVICE_DENY"
        };
        h.atomic(Operation::new(
            Method::POST,
            format!("/api/device/{code}/decision"),
            json!({"approve":approve}),
            action,
        ))
        .await?;
    }
    let (_, raw) = seed_token(&h, h.actor, false).await?;
    let revoke = Operation::new(
        Method::POST,
        "/oauth/revoke",
        json!({"client_id":"audit-client","client_secret":CLIENT_SECRET,"token":raw}),
        "TOKEN_REVOKED",
    )
    .form();
    h.atomic(revoke).await?;
    sqlx::query("INSERT INTO oauth_clients(client_id,client_name,client_secret_hash) VALUES('audit-foreign-client','Foreign fixture client',$1)").bind(hash::sha256_hex(CLIENT_SECRET)).execute(&h.pool).await?;
    let (_, foreign_token) = seed_token(&h, h.actor, false).await?;
    let before = h.snapshot(false).await?;
    for (client, raw) in [
        ("audit-client", raw.as_str()),
        ("audit-client", "unknown-owned-token"),
        ("audit-foreign-client", foreign_token.as_str()),
    ] {
        let reply = h
            .call(
                &Operation::new(
                    Method::POST,
                    "/oauth/revoke",
                    json!({"client_id":client,"client_secret":CLIENT_SECRET,"token":raw}),
                    "TOKEN_REVOKED",
                )
                .form(),
            )
            .await?;
        ensure!(
            reply.status == StatusCode::OK,
            "RFC7009 repeat/unknown did not preserve 200"
        );
    }
    ensure!(
        h.snapshot(false).await? == before,
        "RFC7009 repeat/unknown changed durable state or audit"
    );
    assert_kick_lock_wait_counts_current_tokens(&h, target).await?;
    assert_client_update_waits_for_application_before_client(&h).await?;
    eprintln!("management_audit_special RFC7009_repeat_unknown_foreign=PASS kick_fresh_snapshot=PASS client_application_lock_order=PASS");
    Ok(())
}

async fn wait_for_blocker(pool: &PgPool, blocker: i32, table: &str) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
                 AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)) AND query LIKE $2)",
            )
            .bind(blocker)
            .bind(format!("%{table}%"))
            .fetch_one(pool)
            .await?;
            if waiting {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(|_| anyhow!("request did not reach the observed {table} lock barrier"))?
}

async fn assert_kick_lock_wait_counts_current_tokens(h: &Harness, user: Uuid) -> Result<()> {
    let mut blocker = h.pool.begin().await?;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await?;
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(user)
        .execute(&mut *blocker)
        .await?;
    let op = Operation::new(
        Method::POST,
        "/api/oauth/tokens/kick",
        json!({"userId":user}),
        "KICK_SESSIONS",
    );
    let control = async {
        let observed = wait_for_blocker(&h.pool, pid, "users").await;
        if let Err(error) = observed {
            blocker.rollback().await?;
            return Err(error);
        }
        sqlx::query("INSERT INTO oauth_tokens(token_hash,token_type,user_id,client_id,tenant_id,organization_id,expires_at) VALUES($1,'access',$2,'audit-client',$3,$4,now()+interval '1 hour')")
            .bind(hash::sha256_hex(&Uuid::new_v4().to_string())).bind(user).bind(h.tenant).bind(h.root).execute(&mut *blocker).await?;
        blocker.commit().await?;
        Ok(())
    };
    let (reply, controlled) = tokio::join!(h.call(&op), control);
    controlled?;
    let reply = reply?;
    ensure!(
        reply.status == StatusCode::OK && reply.body["revoked"] == 1,
        "kick missed a token committed while waiting for its user lock"
    );
    let detail: String = sqlx::query_scalar(
        "SELECT detail FROM audit_logs WHERE action='KICK_SESSIONS' AND request_id=$1",
    )
    .bind(reply.request_id)
    .fetch_one(&h.pool)
    .await?;
    let detail: Value = serde_json::from_str(&detail)?;
    ensure!(
        detail["before"]["unrevokedCredentialCount"] == 1
            && detail["after"]["unrevokedCredentialCount"] == 0,
        "kick audit used a stale count from before the lock wait"
    );
    h.safe_audits().await
}
async fn application_matrix(h: Harness) -> Result<()> {
    let department = h
        .atomic(
            Operation::new(
                Method::POST,
                "/api/departments",
                json!({"organizationId":h.child,"name":"Audited department"}),
                "CREATE_DEPARTMENT",
            )
            .named("Audited department"),
        )
        .await?;
    let department = id(&department)?;
    h.atomic(
        Operation::new(
            Method::PUT,
            format!("/api/departments/{department}"),
            json!({"name":"Renamed audited department"}),
            "UPDATE_DEPARTMENT",
        )
        .named("Renamed audited department"),
    )
    .await?;
    h.atomic(
        Operation::new(
            Method::DELETE,
            format!("/api/departments/{department}"),
            Value::Null,
            "DELETE_DEPARTMENT",
        )
        .named("Renamed audited department"),
    )
    .await?;
    let created = h.atomic(Operation::new(Method::POST,"/api/applications",json!({"code":"audited-application","name":"Audited application","entryType":"HTML","basePath":"/micro/audited-application","entryUrl":"https://example.test/audited-application/v1/index.html","autoCreateMenu":true}),"CREATE_APPLICATION").named("Audited application")).await?;
    let app = id(&created)?;
    h.atomic(
        Operation::new(
            Method::PUT,
            format!("/api/applications/{app}"),
            json!({"name":"Renamed audited application"}),
            "UPDATE_APPLICATION",
        )
        .named("Renamed audited application"),
    )
    .await?;
    no_event(
        &h,
        Operation::new(
            Method::PUT,
            format!("/api/applications/{app}"),
            json!({"name":"Renamed audited application"}),
            "UPDATE_APPLICATION",
        ),
    )
    .await?;
    let menu = h.atomic(Operation::new(Method::POST,"/api/menus",json!({"applicationId":app,"title":"Audited parent menu","type":"MENU","path":"/micro/audited-application/parent"}),"CREATE_MENU").named("Audited parent menu")).await?;
    let menu_id = id(&menu)?;
    h.atomic(
        Operation::new(
            Method::PUT,
            format!("/api/menus/{menu_id}"),
            json!({"title":"Renamed audited parent menu"}),
            "UPDATE_MENU",
        )
        .named("Renamed audited parent menu"),
    )
    .await?;
    no_event(
        &h,
        Operation::new(
            Method::PUT,
            format!("/api/menus/{menu_id}"),
            json!({"title":"Renamed audited parent menu"}),
            "UPDATE_MENU",
        ),
    )
    .await?;
    sqlx::query("INSERT INTO menus(application_id,parent_id,title,type,path) VALUES($1,$2,'Audit cascading child','MENU','/micro/audited-application/child')")
        .bind(app).bind(menu_id).execute(&h.pool).await?;
    h.atomic(
        Operation::new(
            Method::DELETE,
            format!("/api/menus/{menu_id}"),
            Value::Null,
            "DELETE_MENU",
        )
        .named("Renamed audited parent menu"),
    )
    .await?;
    h.atomic(
        Operation::new(
            Method::DELETE,
            format!("/api/applications/{app}"),
            Value::Null,
            "DELETE_APPLICATION",
        )
        .named("Renamed audited application"),
    )
    .await?;
    Ok(())
}
const RELEASE_AUDIT_HTML: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>Owned audit release</title></head><body>Owned immutable audit fixture</body></html>";

struct ReleaseAuditTempRoot {
    path: std::path::PathBuf,
    parent: std::path::PathBuf,
    owner: String,
}

impl ReleaseAuditTempRoot {
    fn create() -> Result<Self> {
        let parent = std::fs::canonicalize(std::env::temp_dir())?;
        let owner = format!("phase13-release-audit-{}", Uuid::new_v4().simple());
        let path = parent.join(&owner);
        std::fs::create_dir(&path)?;
        let root = Self {
            path,
            parent,
            owner,
        };
        std::fs::write(root.path.join(".phase13-owner"), root.owner.as_bytes())?;
        Ok(root)
    }

    fn cleanup(&self) -> Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        let actual = std::fs::canonicalize(&self.path)?;
        // Recursive deletion is limited to this freshly-created UUID directory,
        // directly inside the resolved OS temp root, with its exact owner marker.
        ensure!(
            actual == self.path
                && actual.parent() == Some(self.parent.as_path())
                && actual.file_name() == Some(std::ffi::OsStr::new(&self.owner)),
            "release audit temporary directory ownership changed"
        );
        ensure!(
            std::fs::read(actual.join(".phase13-owner"))? == self.owner.as_bytes(),
            "release audit temporary directory owner marker changed"
        );
        std::fs::remove_dir_all(&actual)?;
        Ok(())
    }
}

impl Drop for ReleaseAuditTempRoot {
    fn drop(&mut self) {
        if self.cleanup().is_err() {
            eprintln!("owned release audit temporary directory cleanup failed");
        }
    }
}

struct ReleaseAuditServer {
    origin: String,
    task: JoinHandle<()>,
}

impl ReleaseAuditServer {
    async fn start(root: &std::path::Path) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let app = Router::new()
            .route(
                "/micro/audit-release/v1/index.html",
                axum::routing::get(|| async { axum::response::Html(RELEASE_AUDIT_HTML) }),
            )
            .route(
                "/micro/audit-release/v2/index.html",
                axum::routing::get(|| async { axum::response::Html(RELEASE_AUDIT_HTML) }),
            )
            .nest_service("/sub-apps", tower_http::services::ServeDir::new(root));
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self { origin, task })
    }

    async fn stop(&mut self) -> Result<()> {
        self.task.abort();
        match (&mut self.task).await {
            Err(error) if error.is_cancelled() => Ok(()),
            Ok(()) => Ok(()),
            Err(_) => Err(anyhow!("owned release audit HTTP fixture failed")),
        }
    }
}

impl Drop for ReleaseAuditServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn release_audit_zip() -> Result<Vec<u8>> {
    use std::io::{Cursor, Write};
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    archive.start_file("index.html", zip::write::SimpleFileOptions::default())?;
    archive.write_all(RELEASE_AUDIT_HTML.as_bytes())?;
    Ok(archive.finish()?.into_inner())
}

async fn release_audit_pointer(h: &Harness, app: Uuid) -> Result<(Uuid, Option<Uuid>)> {
    Ok(sqlx::query_as(
        "SELECT active_release_id,previous_release_id FROM application_deployments \
         WHERE application_id=$1 AND environment='development'",
    )
    .bind(app)
    .fetch_one(&h.pool)
    .await?)
}

async fn release_matrix(mut h: Harness) -> Result<()> {
    let root = ReleaseAuditTempRoot::create()?;
    let mut server = ReleaseAuditServer::start(&root.path).await?;
    let mut config = h.state.config.as_ref().clone();
    config.application_environment = "development".into();
    config.micro_gateway_origin = server.origin.clone();
    config.micro_allowed_origins = vec![server.origin.clone()];
    config.micro_apps_dir = root
        .path
        .to_str()
        .ok_or_else(|| anyhow!("owned release audit temporary path is not UTF-8"))?
        .into();
    h.state = AppState::new(h.pool.clone(), Arc::new(config));
    h.app = routes::api_router()
        .nest("/oauth", routes::oauth2::oauth_router())
        .layer(middleware::from_fn_with_state(
            h.state.clone(),
            crate::observability::observe,
        ))
        .layer(middleware::from_fn_with_state(
            h.state.clone(),
            crate::normalize_client_ip,
        ))
        .with_state(h.state.clone());
    let application: Uuid = sqlx::query_scalar(
        "INSERT INTO applications(code,name,base_path,entry_type,runtime_mode,entry_url) \
         VALUES('audit-release','Owned audit release','/micro/audit-release','HTML','IFRAME', \
                '/micro/audit-release/v1/index.html') RETURNING id",
    )
    .fetch_one(&h.pool)
    .await?;

    let mut versions = Vec::new();
    for version in ["v1", "v2"] {
        let registered = h.atomic(Operation::new(Method::POST,
            format!("/api/applications/{application}/releases"), json!({
                "environment":"development", "version":version, "entryType":"HTML",
                "entryUrl":format!("{}/micro/audit-release/{version}/index.html", server.origin),
                "resources":[],
            }), "REGISTER_APPLICATION_RELEASE").named(version)).await?;
        let release = id(&registered)?;
        let checked = h
            .atomic(
                Operation::new(
                    Method::POST,
                    format!("/api/applications/{application}/releases/{release}/preflight"),
                    Value::Null,
                    "PREFLIGHT_APPLICATION_RELEASE",
                )
                .named(version),
            )
            .await?;
        ensure!(
            checked.body["validationStatus"] == "PASSED",
            "owned release {version} HTTP preflight did not pass: {}",
            checked.body
        );
        ensure!(
            checked.body["validationResult"]["passed"] == true,
            "owned release {version} lacks passing real HTTP diagnostics"
        );
        h.atomic(
            Operation::new(
                Method::POST,
                format!("/api/applications/{application}/releases/{release}/activate"),
                Value::Null,
                "ACTIVATE_APPLICATION_RELEASE",
            )
            .preflight()
            .named(version),
        )
        .await?;
        let pointer = release_audit_pointer(&h, application).await?;
        ensure!(
            pointer.0 == release && pointer.1 == versions.last().copied(),
            "owned release {version} activation did not preserve expected pointers"
        );
        versions.push(release);
    }
    h.atomic(
        Operation::new(
            Method::POST,
            format!("/api/applications/{application}/deployments/development/rollback"),
            Value::Null,
            "ROLLBACK_APPLICATION_RELEASE",
        )
        .preflight()
        .named("v1"),
    )
    .await?;
    ensure!(
        release_audit_pointer(&h, application).await? == (versions[0], Some(versions[1])),
        "owned release rollback did not atomically exchange active/previous pointers"
    );

    // Upload installs immutable files before the registration transaction. Audit
    // rejection must roll back all PG rows while retaining those reusable bytes.
    let upload_path = format!("/api/applications/{application}/releases/upload?environment=development&version=zip-v1&entryType=HTML&format=zip");
    let archive = release_audit_zip()?;
    let uploaded = h
        .atomic(
            Operation::new(
                Method::POST,
                &upload_path,
                Value::Null,
                "REGISTER_APPLICATION_RELEASE",
            )
            .binary(archive.clone())
            .named("zip-v1"),
        )
        .await?;
    let uploaded_id = id(&uploaded)?;
    ensure!(
        uploaded.body["validationStatus"] == "PASSED",
        "owned uploaded release HTTP preflight did not pass: {}",
        uploaded.body
    );
    let installed = root.path.join("audit-release/versions/zip-v1/index.html");
    ensure!(
        std::fs::read(&installed)? == RELEASE_AUDIT_HTML.as_bytes(),
        "owned upload retry changed its immutable HTML bytes"
    );
    let prior_register_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE action='REGISTER_APPLICATION_RELEASE' AND application_id=$1",
    ).bind(application).fetch_one(&h.pool).await?;
    let prior_release_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM application_releases WHERE application_id=$1")
            .bind(application)
            .fetch_one(&h.pool)
            .await?;
    let duplicate = h
        .call(
            &Operation::new(
                Method::POST,
                &upload_path,
                Value::Null,
                "REGISTER_APPLICATION_RELEASE",
            )
            .binary(archive),
        )
        .await?;
    ensure!(
        duplicate.status.is_success() && id(&duplicate)? == uploaded_id,
        "identical upload retry did not return its existing registered release"
    );
    let (register_count, register_for_request, preflight_for_request): (i64, i64, i64) =
        sqlx::query_as(
            "SELECT count(*) FILTER (WHERE action='REGISTER_APPLICATION_RELEASE'), \
         count(*) FILTER (WHERE action='REGISTER_APPLICATION_RELEASE' AND request_id=$2), \
         count(*) FILTER (WHERE action='PREFLIGHT_APPLICATION_RELEASE' AND request_id=$2) \
         FROM audit_logs WHERE application_id=$1",
        )
        .bind(application)
        .bind(duplicate.request_id)
        .fetch_one(&h.pool)
        .await?;
    let release_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM application_releases WHERE application_id=$1")
            .bind(application)
            .fetch_one(&h.pool)
            .await?;
    ensure!(
        register_count == prior_register_count
            && register_for_request == 0
            && preflight_for_request == 1
            && release_count == prior_release_count,
        "identical upload retry duplicated registration or lost its independent preflight audit"
    );
    ensure!(
        std::fs::read(&installed)? == RELEASE_AUDIT_HTML.as_bytes(),
        "identical upload retry modified installed immutable bytes"
    );
    h.safe_audits().await?;
    eprintln!("management_audit_special release_same_artifact_retry=PASS");
    server.stop().await?;
    root.cleanup()?;
    ensure!(
        !root.path.exists(),
        "owned release audit temporary files remain"
    );
    Ok(())
}

async fn notification_matrix(h: Harness) -> Result<()> {
    let recipient = h.user("notification-audit-target", h.root).await?;
    let issued = h
        .call(&Operation::new(
            Method::POST,
            "/api/oauth/session-token",
            json!({"appCode": "system"}),
            "TOKEN_ISSUED",
        ))
        .await?;
    ensure!(
        issued.status == StatusCode::CREATED,
        "notification app-token setup returned {}",
        issued.status
    );
    let app_token = issued.body["accessToken"]
        .as_str()
        .ok_or_else(|| anyhow!("notification app-token setup omitted accessToken"))?
        .to_string();

    for (label, path, token) in [
        ("session", "/api/notifications", h.admin.as_str()),
        (
            "application",
            "/api/apps/system/notifications",
            app_token.as_str(),
        ),
    ] {
        let title = format!("Sensitive notification title {label} {}", Uuid::new_v4());
        let content = format!("Sensitive notification body {label} {}", Uuid::new_v4());
        {
            let mut secrets = h.secrets.lock().unwrap();
            secrets.push(title.clone());
            secrets.push(content.clone());
        }
        let key = format!("audit-notification-{label}-{}", Uuid::new_v4());
        let mut body = json!({
            "idempotencyKey": key,
            "target": {"kind": "USER", "userId": recipient, "organizationId": h.root},
            "type": "SECURITY", "title": title, "content": content,
            "link": "/micro/system/audit-notifications",
        });
        if label == "session" {
            body["applicationId"] = json!(h.application);
        }
        let success = h
            .atomic(
                Operation::new(Method::POST, path, body.clone(), "PUBLISH_NOTIFICATION")
                    .token(token),
            )
            .await?;
        let publication = success.body["publicationId"]
            .as_str()
            .ok_or_else(|| anyhow!("notification publication omitted id"))?
            .parse::<Uuid>()?;
        ensure!(
            success.body["duplicate"] == false
                && success.body["recipientCount"] == 1
                && success.body["deliveryCount"] == 1,
            "{label} publication did not commit exactly one recipient and delivery"
        );
        let delivered: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM notifications WHERE publication_id=$1 AND user_id=$2 \
             AND tenant_id=$3 AND organization_id=$4 AND application_id=$5 AND title=$6 AND content=$7",
        ).bind(publication).bind(recipient).bind(h.tenant).bind(h.root)
            .bind(h.application).bind(&title).bind(&content).fetch_one(&h.pool).await?;
        ensure!(
            delivered == 1,
            "{label} publication lost its actual delivery content or scope"
        );
        let audit_application: Option<Uuid> = sqlx::query_scalar(
            "SELECT application_id FROM audit_logs WHERE action='PUBLISH_NOTIFICATION' AND request_id=$1",
        ).bind(success.request_id).fetch_one(&h.pool).await?;
        ensure!(
            audit_application == Some(h.application),
            "{label} publication audit lost explicit application attribution"
        );

        let snapshot = h.snapshot(false).await?;
        for _ in 0..2 {
            let duplicate = h
                .call(
                    &Operation::new(Method::POST, path, body.clone(), "PUBLISH_NOTIFICATION")
                        .token(token),
                )
                .await?;
            ensure!(
                duplicate.status == StatusCode::OK && duplicate.body["duplicate"] == true,
                "{label} publication retry did not return the existing result"
            );
            ensure!(
                duplicate.body["publicationId"] == success.body["publicationId"]
                    && duplicate.body["recipientCount"] == success.body["recipientCount"]
                    && duplicate.body["deliveryCount"] == success.body["deliveryCount"],
                "{label} publication retry changed the existing publication result"
            );
            let duplicate_audits: i64 =
                sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE request_id=$1")
                    .bind(duplicate.request_id)
                    .fetch_one(&h.pool)
                    .await?;
            ensure!(
                duplicate_audits == 0,
                "{label} publication retry wrote a duplicate audit"
            );
            ensure!(h.snapshot(false).await? == snapshot,
                "{label} publication retry changed publication, delivery, audit or other business tables");
        }
        h.safe_audits().await?;
        eprintln!(
            "management_audit_special notification_idempotency={} result=PASS",
            label
        );
    }
    Ok(())
}

// Explicit no-op and controlled lock-order regressions.
async fn no_event(h: &Harness, op: Operation) -> Result<()> {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs")
        .fetch_one(&h.pool)
        .await?;
    let failures = crate::observability::audit_write_failures();
    let reply = h.call(&op).await?;
    ensure!(
        reply.status.is_success(),
        "explicit no-op returned {}",
        reply.status
    );
    let now: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs")
        .fetch_one(&h.pool)
        .await?;
    let per_request: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE request_id=$1")
            .bind(reply.request_id)
            .fetch_one(&h.pool)
            .await?;
    ensure!(
        now == count && per_request == 0,
        "explicit no-op wrote a success audit"
    );
    ensure!(
        crate::observability::audit_write_failures() == failures,
        "explicit no-op changed the audit failure counter"
    );
    eprintln!("management_audit_noop action={} result=PASS", op.action);
    Ok(())
}

async fn assert_client_update_waits_for_application_before_client(h: &Harness) -> Result<()> {
    let old: Uuid = sqlx::query_scalar("INSERT INTO applications(name,code) VALUES('Concurrent old application','audit-old-binding') RETURNING id")
        .fetch_one(&h.pool).await?;
    let new: Uuid = sqlx::query_scalar("INSERT INTO applications(name,code) VALUES('Concurrent new application','audit-new-binding') RETURNING id")
        .fetch_one(&h.pool).await?;
    let client: Uuid = sqlx::query_scalar("INSERT INTO oauth_clients(client_id,client_name,application_id) VALUES('audit-lock-client','Concurrent binding client',$1) RETURNING id")
        .bind(old).fetch_one(&h.pool).await?;
    let mut blocker = h.pool.begin().await?;
    let external: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await?;
    sqlx::query("SELECT id FROM oauth_clients WHERE id=$1 FOR UPDATE")
        .bind(client)
        .execute(&mut *blocker)
        .await?;
    let delete = Operation::new(
        Method::DELETE,
        format!("/api/applications/{old}"),
        Value::Null,
        "DELETE_APPLICATION",
    );
    let update = Operation::new(
        Method::PUT,
        format!("/api/oauth/clients/{client}"),
        json!({"applicationId":new}),
        "UPDATE_CLIENT",
    );
    let control = async {
        let deletion=tokio::time::timeout(Duration::from_secs(15),async {
            loop {
                let pid: Option<i32>=sqlx::query_scalar(
                    "SELECT pid FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' \
                     AND $1=ANY(pg_blocking_pids(pid)) AND query LIKE '%DELETE FROM applications%' LIMIT 1",
                ).bind(external).fetch_optional(&h.pool).await?;
                if let Some(pid)=pid { return Ok::<_,anyhow::Error>(pid); }
                tokio::task::yield_now().await;
            }
        }).await;
        let deletion = match deletion {
            Ok(Ok(pid)) => pid,
            _ => {
                blocker.rollback().await?;
                return Err(anyhow!(
                    "application delete did not reach the observed client-cascade lock barrier"
                ));
            }
        };
        let release = async {
            let observed = wait_for_blocker(&h.pool, deletion, "applications").await;
            blocker.rollback().await?;
            observed
        };
        let (reply, observed) = tokio::join!(h.call(&update), release);
        observed?;
        let reply = reply?;
        ensure!(
            [StatusCode::NOT_FOUND, StatusCode::CONFLICT].contains(&reply.status),
            "client update after concurrent app deletion returned {}",
            reply.status
        );
        let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE request_id=$1")
            .bind(reply.request_id)
            .fetch_one(&h.pool)
            .await?;
        ensure!(
            audits == 0,
            "rejected concurrent binding wrote success audit"
        );
        Ok(())
    };
    let (deleted, controlled) = tokio::join!(h.call(&delete), control);
    controlled?;
    let deleted = deleted?;
    ensure!(
        deleted.status == StatusCode::OK,
        "concurrent application delete failed or deadlocked"
    );
    let remaining:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM applications WHERE id=$1)+(SELECT count(*) FROM oauth_clients WHERE id=$2 AND application_id IS NOT NULL)")
        .bind(old).bind(client).fetch_one(&h.pool).await?;
    ensure!(
        remaining == 0,
        "concurrent app deletion did not clear its client binding"
    );
    h.safe_audits().await
}
