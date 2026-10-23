//! Release API regressions use a newly created PostgreSQL database and real HTTP fixtures.
//! Run explicitly with `cargo test release_tests -- --ignored`; TEST_DATABASE_URL may also
//! be read from the repository's .env.test. Its account must be allowed to create databases.
use std::collections::{BTreeMap, HashSet};
use std::io::{Cursor, Write};
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, ensure, Result};
use axum::body::{to_bytes, Body};
use axum::extract::State;
use axum::http::{header, HeaderMap, Method, Request, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use serde_json::{json, Value};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgQueryResult};
use sqlx::PgPool;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tower::ServiceExt;
use uuid::Uuid;

use crate::config::Config;
use crate::routes;
use crate::state::AppState;
use crate::util::hash;
use crate::util::jwt::{self, Claims};

mod database_upgrade;
mod managed_gate;
mod node_fixture;
mod phase31;
mod qiankun_only;
mod url_entries;
mod zip_registration;

const ENVIRONMENT: &str = "development";
const JWT_SECRET: &str = "isolated-release-tests-only";

struct TestDatabase {
    pool: PgPool,
    admin: PgPool,
    name: String,
}

fn test_database_options() -> Result<PgConnectOptions> {
    let url = match std::env::var("TEST_DATABASE_URL") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            let file = Path::new(env!("CARGO_MANIFEST_DIR")).join(".env.test");
            let vars = dotenvy::from_path_iter(file).map_err(|_| {
                anyhow!("set TEST_DATABASE_URL or add it to the root backend/.env.test")
            })?;
            let mut url = None;
            for entry in vars {
                let (key, value) =
                    entry.map_err(|_| anyhow!("root backend/.env.test cannot be parsed"))?;
                if key == "TEST_DATABASE_URL" && !value.trim().is_empty() {
                    url = Some(value);
                }
            }
            url.ok_or_else(|| anyhow!("root backend/.env.test does not define TEST_DATABASE_URL"))?
        }
    };
    PgConnectOptions::from_str(&url)
        .map_err(|_| anyhow!("TEST_DATABASE_URL is not a valid PostgreSQL connection URL"))
}

// Connection errors can include the database account. Keep credentials out of test output.
fn database_error(stage: &str, error: &sqlx::Error) -> anyhow::Error {
    let code = error.as_database_error().and_then(|value| value.code());
    match code {
        Some(code) => anyhow!("{stage} failed (SQLSTATE {code})"),
        None => anyhow!("{stage} failed; check PostgreSQL availability and connection permissions"),
    }
}

impl TestDatabase {
    async fn create() -> Result<Self> {
        Self::create_through(i64::MAX).await
    }

    async fn create_through(version: i64) -> Result<Self> {
        let options = test_database_options()?;
        let admin = PgPoolOptions::new()
            .max_connections(1)
            // Test credentials identify a server/account. The configured test
            // database need not already exist: creation uses the maintenance DB.
            .connect_with(options.clone().database("postgres"))
            .await
            .map_err(|error| database_error("release test administration connection", &error))?;
        let name = format!("release_test_{}", Uuid::new_v4().simple());
        // The identifier is generated here and contains only ASCII letters, digits and '_'.
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await
            .map_err(|error| {
                database_error("creating the isolated release test database", &error)
            })?;
        let pool = match PgPoolOptions::new()
            .max_connections(5)
            .connect_with(options.database(&name))
            .await
        {
            Ok(pool) => pool,
            Err(error) => {
                let failure = database_error("isolated release test database connection", &error);
                let _ = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
                    .execute(&admin)
                    .await;
                admin.close().await;
                return Err(failure);
            }
        };
        let database = Self { pool, admin, name };
        let migrations = sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(
                sqlx::migrate!("./migrations")
                    .iter()
                    .filter(|m| m.version <= version)
                    .cloned()
                    .collect(),
            ),
            ..sqlx::migrate::Migrator::DEFAULT
        };
        if migrations.run(&database.pool).await.is_err() {
            let _ = database.finish(Ok(())).await;
            return Err(anyhow!("isolated release test database migrations failed"));
        }
        Ok(database)
    }

    async fn finish(self, result: Result<()>) -> Result<()> {
        eprintln!(
            "release test cleanup {}: assertions_ok={} pool_size={} idle={}",
            self.name,
            result.is_ok(),
            self.pool.size(),
            self.pool.num_idle()
        );
        // Keep a pending close future alive while the administration connection forcibly
        // drops this unique database. Outstanding protocol cleanup must not hang a test.
        let close = self.pool.close();
        tokio::pin!(close);
        let closed = tokio::time::timeout(std::time::Duration::from_secs(5), &mut close)
            .await
            .is_ok();
        let cleanup = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await
            .map(|_| ())
            .map_err(|error| database_error("dropping the isolated release test database", &error));
        let closed = closed
            || tokio::time::timeout(std::time::Duration::from_secs(10), &mut close)
                .await
                .is_ok();
        eprintln!(
            "release test cleanup {}: database_removed={} pool_closed={closed}",
            self.name,
            cleanup.is_ok()
        );
        let admin_closed =
            tokio::time::timeout(std::time::Duration::from_secs(5), self.admin.close())
                .await
                .is_ok();
        result.and(cleanup).and(if closed && admin_closed {
            Ok(())
        } else {
            Err(anyhow!(
                "isolated database connection pool cleanup timed out"
            ))
        })
    }
}

async fn call(
    app: &Router,
    method: Method,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> Result<(StatusCode, Value)> {
    let mut request = Request::builder().method(method.clone()).uri(path);
    if !token.is_empty() {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let bytes = match body {
        Some(value) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            serde_json::to_vec(&value)?
        }
        None => Vec::new(),
    };
    let trace = std::env::var_os("RELEASE_TEST_TRACE").is_some();
    if trace {
        eprintln!("release API begin {method} {path}");
    }
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(45),
        app.clone().oneshot(request.body(Body::from(bytes))?),
    )
    .await
    .map_err(|_| anyhow!("release API timed out: {method} {path}"))??;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    let body =
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes)));
    if trace {
        eprintln!("release API end {method} {path} {status}");
    }
    Ok((status, body))
}

async fn successful_call(
    app: &Router,
    method: Method,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> Result<Value> {
    let (status, body) = call(app, method, path, token, body).await?;
    ensure!(status == StatusCode::OK, "{path} returned {status}: {body}");
    Ok(body)
}

async fn session(pool: &PgPool, user: Uuid, tenant: Uuid, org: Uuid) -> Result<String> {
    let version: i64 = sqlx::query_scalar("SELECT session_version FROM users WHERE id = $1")
        .bind(user)
        .fetch_one(pool)
        .await?;
    let sid: Uuid = sqlx::query_scalar(
        "INSERT INTO platform_sessions (user_id, tenant_id, organization_id, expires_at) \
         VALUES ($1, $2, $3, now() + interval '1 hour') RETURNING id",
    )
    .bind(user)
    .bind(tenant)
    .bind(org)
    .fetch_one(pool)
    .await?;
    let now = chrono::Utc::now().timestamp();
    jwt::sign(
        &Claims {
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
        JWT_SECRET,
    )
}

async fn tenant(pool: &PgPool, code: &str, platform: bool) -> Result<(Uuid, Uuid)> {
    let tenant: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (name, code, is_platform) VALUES ($1, $1, $2) RETURNING id",
    )
    .bind(code)
    .bind(platform)
    .fetch_one(pool)
    .await?;
    let org: Uuid = sqlx::query_scalar(
        "INSERT INTO organizations (tenant_id, name) VALUES ($1, $2) RETURNING id",
    )
    .bind(tenant)
    .bind(format!("{code} root"))
    .fetch_one(pool)
    .await?;
    Ok((tenant, org))
}

async fn user(pool: &PgPool, username: &str, org: Uuid, super_admin: bool) -> Result<Uuid> {
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, is_super_admin) \
         VALUES ($1, 'test-only-unused-password', $2) RETURNING id",
    )
    .bind(username)
    .bind(super_admin)
    .fetch_one(pool)
    .await?;
    sqlx::query("INSERT INTO memberships (user_id, organization_id) VALUES ($1, $2)")
        .bind(user)
        .bind(org)
        .execute(pool)
        .await?;
    Ok(user)
}

async fn application(pool: &PgPool, code: &str, entry_url: &str) -> Result<Uuid> {
    sqlx::query_scalar(
        "INSERT INTO applications (code, name, base_path, entry_type, runtime_mode, entry_url) \
         VALUES ($1, $1, $2, 'HTML', 'TRUSTED', $3) RETURNING id",
    )
    .bind(code)
    .bind(format!("/micro/{code}"))
    .bind(entry_url)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

async fn grant_application(pool: &PgPool, tenant: Uuid, org: Uuid, app: Uuid) -> Result<()> {
    sqlx::query(
        "INSERT INTO tenant_applications (tenant_id, application_id) VALUES ($1, $2) \
         ON CONFLICT DO NOTHING",
    )
    .bind(tenant)
    .bind(app)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO organization_applications (organization_id, application_id) \
         VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(org)
    .bind(app)
    .execute(pool)
    .await?;
    Ok(())
}

async fn oauth_client(pool: &PgPool, app: Uuid, client_id: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO oauth_clients \
         (client_id, client_secret_hash, client_name, client_type, grant_types, scopes, \
          access_token_validity, refresh_token_validity, application_id) \
         VALUES ($1, $2, $1, 'CONFIDENTIAL', 'client_credentials', 'profile', 7200, 604800, $3)",
    )
    .bind(client_id)
    .bind(hash::sha256_hex("isolated-release-client-secret-only"))
    .bind(app)
    .execute(pool)
    .await?;
    Ok(())
}

async fn menu(pool: &PgPool, app: Uuid, permission: &str) -> Result<Uuid> {
    sqlx::query_scalar(
        "INSERT INTO menus (application_id, title, path, permission, link_target) \
         VALUES ($1, $2, '/micro/release-app', $2, 'MICRO') RETURNING id",
    )
    .bind(app)
    .bind(permission)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

async fn grant_menu(
    pool: &PgPool,
    tenant: Uuid,
    org: Uuid,
    role: Option<Uuid>,
    menu: Uuid,
) -> Result<()> {
    sqlx::query("INSERT INTO tenant_menus (tenant_id, menu_id) VALUES ($1, $2)")
        .bind(tenant)
        .bind(menu)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO organization_menus (organization_id, menu_id) VALUES ($1, $2)")
        .bind(org)
        .bind(menu)
        .execute(pool)
        .await?;
    if let Some(role) = role {
        sqlx::query("INSERT INTO role_menus (role_id, menu_id) VALUES ($1, $2)")
            .bind(role)
            .bind(menu)
            .execute(pool)
            .await?;
    }
    Ok(())
}

struct Harness {
    pool: PgPool,
    app: Router,
    main_app: Uuid,
    other_app: Uuid,
    tenant_only_app: Uuid,
    org_only_app: Uuid,
    no_role_app: Uuid,
    platform: String,
    a: String,
    b: String,
}

impl Harness {
    async fn create(pool: &PgPool, origin: &str) -> Result<Self> {
        Self::create_with_origins(pool, origin, vec![origin.to_string()]).await
    }

    async fn create_with_origins(
        pool: &PgPool,
        origin: &str,
        allowed_origins: Vec<String>,
    ) -> Result<Self> {
        Self::create_with_upload_root(pool, origin, allowed_origins, None).await
    }

    async fn create_with_upload_root(
        pool: &PgPool,
        origin: &str,
        allowed_origins: Vec<String>,
        upload_root: Option<&str>,
    ) -> Result<Self> {
        // Migration SQL defines all functions and constraints; test identities need no seed data.
        let (platform_tenant, platform_org) = tenant(pool, "release-platform", true).await?;
        let (tenant_a, org_a) = tenant(pool, "release-tenant-a", false).await?;
        let (tenant_b, org_b) = tenant(pool, "release-tenant-b", false).await?;
        let platform_user = user(pool, "release-platform", platform_org, true).await?;
        let admin_a = user(pool, "release-admin-a", org_a, false).await?;
        let admin_b = user(pool, "release-admin-b", org_b, false).await?;
        let main_app = application(pool, "release-app", "/legacy/release-app/index.html").await?;
        let other_app = application(pool, "release-other", "/legacy/other/index.html").await?;
        // Native management pages use the same metadata as micro apps, while
        // their platform route has no remote execution entry.
        let system_app: Uuid = sqlx::query_scalar("INSERT INTO applications(code,name,base_path,entry_type,runtime_mode,entry_url) VALUES('release-system','Release management','/system','HTML','TRUSTED','') RETURNING id")
            .fetch_one(pool).await?;
        let tenant_only_app =
            application(pool, "release-tenant-only", "/legacy/tenant/index.html").await?;
        let org_only_app = application(pool, "release-org-only", "/legacy/org/index.html").await?;
        let no_role_app =
            application(pool, "release-no-role", "/legacy/no-role/index.html").await?;
        let management_menu = menu(pool, system_app, "system:application").await?;
        grant_application(pool, platform_tenant, platform_org, system_app).await?;
        grant_menu(pool, platform_tenant, platform_org, None, management_menu).await?;
        let main_menu = menu(pool, main_app, "release:read").await?;
        let no_role_menu = menu(pool, no_role_app, "release:no-role").await?;
        for (tenant, org, user) in [(tenant_a, org_a, admin_a), (tenant_b, org_b, admin_b)] {
            let role: Uuid = sqlx::query_scalar(
                "INSERT INTO roles (tenant_id, organization_id, name, code) \
                 VALUES ($1, $2, 'Release administrator', 'release-admin') RETURNING id",
            )
            .bind(tenant)
            .bind(org)
            .fetch_one(pool)
            .await?;
            sqlx::query(
                "INSERT INTO user_roles (user_id, role_id, organization_id) VALUES ($1, $2, $3)",
            )
            .bind(user)
            .bind(role)
            .bind(org)
            .execute(pool)
            .await?;
            grant_application(pool, tenant, org, system_app).await?;
            grant_application(pool, tenant, org, main_app).await?;
            grant_menu(pool, tenant, org, Some(role), management_menu).await?;
            grant_menu(pool, tenant, org, Some(role), main_menu).await?;
            sqlx::query(
                "INSERT INTO membership_data_grants \
                 (membership_id, application_id, resource_code, action_code, scope_type) \
                 SELECT id, $3, 'release', 'read', 'ORG' FROM memberships \
                 WHERE user_id = $1 AND organization_id = $2",
            )
            .bind(user)
            .bind(org)
            .bind(main_app)
            .execute(pool)
            .await?;
        }
        grant_application(pool, tenant_b, org_b, other_app).await?;
        grant_application(pool, tenant_a, org_a, no_role_app).await?;
        grant_menu(pool, tenant_a, org_a, None, no_role_menu).await?;
        sqlx::query("INSERT INTO tenant_applications (tenant_id, application_id) VALUES ($1, $2)")
            .bind(tenant_a)
            .bind(tenant_only_app)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO organization_applications (organization_id, application_id) VALUES ($1, $2)")
            .bind(org_a)
            .bind(org_only_app)
            .execute(pool)
            .await?;
        let mut config = Config::from_env();
        // AppState uses the supplied pool. Do not keep the administration URL in this fixture.
        config.database_url = "postgres://unused/release-test".into();
        config.auto_create_database = false;
        config.jwt_secret = JWT_SECRET.into();
        config.application_environment = ENVIRONMENT.into();
        config.micro_gateway_origin = origin.into();
        config.micro_allowed_origins = allowed_origins;
        config.micro_apps_dir = upload_root.unwrap_or("").into();
        let app = Router::new()
            .merge(routes::api_router())
            .with_state(AppState::new(pool.clone(), Arc::new(config)));
        Ok(Self {
            pool: pool.clone(),
            app,
            main_app,
            other_app,
            tenant_only_app,
            org_only_app,
            no_role_app,
            platform: session(pool, platform_user, platform_tenant, platform_org).await?,
            a: session(pool, admin_a, tenant_a, org_a).await?,
            b: session(pool, admin_b, tenant_b, org_b).await?,
        })
    }

    async fn register_for(&self, app: Uuid, body: Value) -> Result<Value> {
        successful_call(
            &self.app,
            Method::POST,
            &format!("/api/applications/{app}/releases"),
            &self.platform,
            Some(body),
        )
        .await
    }

    async fn register(&self, body: Value) -> Result<Value> {
        self.register_for(self.main_app, body).await
    }

    async fn preflight(&self, release: Uuid, expected: &str) -> Result<Value> {
        let body = successful_call(
            &self.app,
            Method::POST,
            &format!(
                "/api/applications/{}/releases/{release}/preflight",
                self.main_app
            ),
            &self.platform,
            None,
        )
        .await?;
        ensure!(
            body["id"] == release.to_string(),
            "preflight returned a different release"
        );
        ensure!(
            body["validationStatus"] == expected,
            "preflight should be {expected}: {body}"
        );
        ensure!(
            body["checkedAt"].as_str().is_some(),
            "preflight did not record checkedAt"
        );
        ensure!(
            body["validationResult"].is_object(),
            "preflight has no structured validation result"
        );
        let stored: (String, bool, String) = sqlx::query_as(
            "SELECT validation_status, checked_at IS NOT NULL, validation_result::text \
             FROM application_releases WHERE id = $1",
        )
        .bind(release)
        .fetch_one(&self.pool)
        .await?;
        ensure!(
            stored.0 == expected
                && stored.1
                && serde_json::from_str::<Value>(&stored.2)? == body["validationResult"],
            "preflight did not persist its returned result for release {release}"
        );
        Ok(body)
    }

    async fn activate(&self, release: Uuid) -> Result<Value> {
        successful_call(
            &self.app,
            Method::POST,
            &format!(
                "/api/applications/{}/releases/{release}/activate",
                self.main_app
            ),
            &self.platform,
            None,
        )
        .await
    }

    async fn profile(&self, token: &str) -> Result<Value> {
        successful_call(&self.app, Method::GET, "/api/auth/profile", token, None).await
    }

    async fn app_token(&self, token: &str, code: &str) -> Result<String> {
        let (status, body) = call(
            &self.app,
            Method::POST,
            "/api/oauth/session-token",
            token,
            Some(json!({"appCode":code})),
        )
        .await?;
        ensure!(
            status == StatusCode::CREATED,
            "app token issuance failed: {status} {body}"
        );
        body["accessToken"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("app token response has no accessToken"))
    }
}

fn release_id(row: &Value) -> Result<Uuid> {
    row["id"]
        .as_str()
        .ok_or_else(|| anyhow!("release response has no id"))?
        .parse()
        .map_err(Into::into)
}

fn release_body(version: &str, entry_type: &str) -> Value {
    let file = if entry_type == "HTML" {
        "index.html"
    } else {
        "entry.js"
    };
    json!({
        "environment": ENVIRONMENT,
        "version": version,
        "entryType": entry_type,
        "entryUrl": format!("/micro/{version}/{file}"),
        "healthUrl": format!("/micro/{version}/health"),
        "resources": [],
    })
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn release_archive_upload_registers_files_and_preserves_version_bytes() -> Result<()> {
    let database = TestDatabase::create().await?;
    let root = std::env::temp_dir().join(format!("micro-release-upload-{}", Uuid::new_v4()));
    std::fs::create_dir(&root)?;
    let result: Result<()> = async {
        let h = Harness::create_with_upload_root(&database.pool, "http://127.0.0.1:7777",
            vec!["http://127.0.0.1:7777".into()], root.to_str()).await?;
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer.start_file("index.html", zip::write::SimpleFileOptions::default())?;
        writer.write_all(b"<html><script src='./app.js'></script></html>")?;
        writer.start_file("app.js", zip::write::SimpleFileOptions::default())?;
        writer.write_all(b"console.log('release')")?;
        let archive = writer.finish()?.into_inner();
        let upload = |environment: &str, token: &str, body: Vec<u8>| {
            Request::builder().method(Method::POST).uri(format!(
                "/api/applications/{}/releases/upload?environment={environment}&version=1.0.0&entryType=HTML&format=zip", h.main_app
            )).header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/zip")
                .body(Body::from(body))
        };
        let denied = h.app.clone().oneshot(upload("development", &h.a, archive.clone())?).await?;
        ensure!(denied.status() == StatusCode::FORBIDDEN, "tenant user uploaded a release");
        let first = h.app.clone().oneshot(upload("development", &h.platform, archive.clone())?).await?;
        ensure!(first.status() == StatusCode::OK, "upload returned {}", first.status());
        let first: Value = serde_json::from_slice(&to_bytes(first.into_body(), 1024 * 1024).await?)?;
        ensure!(first["entryUrl"] == "/sub-apps/release-app/versions/1.0.0/index.html", "wrong uploaded entry: {first}");
        ensure!(first["resources"].as_array().is_some_and(|items| items.len() == 1), "resource manifest missing: {first}");
        ensure!(first["validationStatus"] == "FAILED" && first["checkedAt"].is_string(),
            "upload did not persist automatic preflight diagnostics: {first}");
        let file = root.join("release-app/versions/1.0.0/app.js");
        ensure!(std::fs::read(&file)? == b"console.log('release')", "uploaded bytes differ");
        let promoted = h.app.clone().oneshot(upload("staging", &h.platform, archive)?).await?;
        ensure!(promoted.status() == StatusCode::OK, "same bytes could not be registered in staging");
        ensure!(std::fs::read(&file)? == b"console.log('release')", "promotion changed version bytes");
        Ok(())
    }.await;
    let _ = std::fs::remove_dir_all(&root);
    database.finish(result).await
}

type OriginRequestLog = Arc<Mutex<Vec<(String, Option<String>)>>>;

#[derive(Clone)]
struct ResourceState {
    gateway_origin: String,
    denied_origin: String,
    requests: Arc<Mutex<Vec<String>>>,
    request_headers: OriginRequestLog,
    unavailable: Arc<Mutex<HashSet<String>>>,
}

struct HttpFixture {
    origin: String,
    other_origin: String,
    requests: Arc<Mutex<Vec<String>>>,
    request_headers: OriginRequestLog,
    denied_hits: Arc<AtomicUsize>,
    cors_requests: OriginRequestLog,
    unavailable: Arc<Mutex<HashSet<String>>>,
    servers: Vec<JoinHandle<()>>,
    node: Option<node_fixture::NodeHttp>,
}

impl Drop for HttpFixture {
    fn drop(&mut self) {
        for server in &self.servers {
            server.abort();
        }
    }
}

#[derive(Clone)]
struct OtherOriginState {
    hits: Arc<AtomicUsize>,
    requests: OriginRequestLog,
    gateway_origin: String,
}

fn frame_headers(mut response: Response, version: &str, gateway_origin: &str) -> Response {
    let version = version.strip_suffix("-remote").unwrap_or(version);
    match version {
        "xfo-deny" => {
            response
                .headers_mut()
                .insert(header::X_FRAME_OPTIONS, "DENY".parse().unwrap());
        }
        "xfo-sameorigin" => {
            response
                .headers_mut()
                .insert(header::X_FRAME_OPTIONS, "SAMEORIGIN".parse().unwrap());
        }
        "csp-none" => {
            response.headers_mut().insert(
                header::CONTENT_SECURITY_POLICY,
                "frame-ancestors 'none'".parse().unwrap(),
            );
        }
        "csp-wrong" => {
            response.headers_mut().insert(
                header::CONTENT_SECURITY_POLICY,
                "frame-ancestors https://wrong.invalid".parse().unwrap(),
            );
        }
        "csp-self" => {
            response.headers_mut().insert(
                header::CONTENT_SECURITY_POLICY,
                "frame-ancestors 'self'".parse().unwrap(),
            );
        }
        "csp-gateway" => {
            response.headers_mut().insert(
                header::CONTENT_SECURITY_POLICY,
                format!("frame-ancestors {gateway_origin}").parse().unwrap(),
            );
        }
        "csp-star" => {
            response.headers_mut().insert(
                header::CONTENT_SECURITY_POLICY,
                "frame-ancestors *".parse().unwrap(),
            );
        }
        "csp-multiple-none" => {
            response.headers_mut().append(
                header::CONTENT_SECURITY_POLICY,
                "frame-ancestors *".parse().unwrap(),
            );
            response.headers_mut().append(
                header::CONTENT_SECURITY_POLICY,
                "frame-ancestors 'none'".parse().unwrap(),
            );
        }
        "csp-combined-none" => {
            response.headers_mut().insert(
                header::CONTENT_SECURITY_POLICY,
                "frame-ancestors *, frame-ancestors 'none'".parse().unwrap(),
            );
        }
        _ => {}
    }
    response
}

async fn other_origin_resource(
    State(state): State<OtherOriginState>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    state.hits.fetch_add(1, Ordering::SeqCst);
    let path = uri.path();
    if let Ok(mut requests) = state.requests.lock() {
        requests.push((
            path.to_string(),
            headers
                .get(header::ORIGIN)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string),
        ));
    }
    let (mime, body) = if path.ends_with("/main.js") {
        (
            "application/javascript",
            "window.__MICRO_APP__ = { mount: function () {}, unmount: function () {} };",
        )
    } else if path.ends_with(".js") {
        (
            "application/javascript",
            "export async function mount() {} export async function unmount() {}",
        )
    } else if path.ends_with(".css") {
        ("text/css", "body { color: #333; }")
    } else {
        ("text/html", "<!doctype html><html><head><link rel=\"stylesheet\" href=\"./styles.css\"></head><body><script src=\"./main.js\"></script></body></html>")
    };
    let mut response = ([(header::CONTENT_TYPE, mime)], body).into_response();
    let allow_origin = if path.contains("star-cors") {
        Some("*")
    } else if path.contains("null-cors") {
        Some("null")
    } else if path.contains("gateway-cors") {
        Some(state.gateway_origin.as_str())
    } else if path.contains("wrong-cors") {
        Some("https://wrong.invalid")
    } else {
        None
    };
    if let Some(origin) = allow_origin {
        if let Ok(value) = origin.parse() {
            response
                .headers_mut()
                .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
        }
    }
    let version = path.split('/').nth(2).unwrap_or("");
    frame_headers(response, version, &state.gateway_origin)
}

async fn fixture_resource(
    State(state): State<ResourceState>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let path = uri.path();
    if let Ok(mut requests) = state.requests.lock() {
        requests.push(path.to_string());
    }
    if let Ok(mut requests) = state.request_headers.lock() {
        requests.push((
            path.to_string(),
            headers
                .get(header::ORIGIN)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string),
        ));
    }
    if state
        .unavailable
        .lock()
        .is_ok_and(|paths| paths.contains(path))
    {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "fixture temporarily unavailable",
        )
            .into_response();
    }
    let parts: Vec<_> = path.split('/').collect();
    let version = parts.get(2).copied().unwrap_or("");
    let file = parts.last().copied().unwrap_or("");
    if version == "bad404" || file.starts_with("missing.") {
        return (StatusCode::NOT_FOUND, "fixture resource missing").into_response();
    }
    if version == "badredirect" && file == "index.html" {
        return (
            StatusCode::FOUND,
            [(
                header::LOCATION,
                format!("{}/micro/badredirect/index.html", state.denied_origin),
            )],
            "",
        )
            .into_response();
    }
    if path == "/micro/directoryredirect/index.html" {
        return (
            StatusCode::FOUND,
            [(header::LOCATION, "./nested/index.html")],
            "",
        )
            .into_response();
    }
    if version == "loopredirect" && file == "index.html" {
        return (
            StatusCode::TEMPORARY_REDIRECT,
            [(header::LOCATION, path)],
            "",
        )
            .into_response();
    }
    if file == "health" {
        let status = if version == "badhealth" {
            StatusCode::SERVICE_UNAVAILABLE
        } else {
            StatusCode::OK
        };
        return (
            status,
            [(header::CONTENT_TYPE, "application/json")],
            "{\"ok\":true}",
        )
            .into_response();
    }
    if file == "index.html" {
        let mime = if version == "badmime" {
            "application/json"
        } else {
            "text/html; charset=utf-8"
        };
        let script = if version == "badresource404" {
            "missing.js"
        } else {
            "main.js"
        };
        let base = if version == "basechange" {
            "<base href=\"./assets/\">"
        } else {
            ""
        };
        let module_url = if version.contains("-cdn-") {
            format!("{}/micro/{version}/entry.js", state.denied_origin)
        } else {
            "./entry.js".to_string()
        };
        let module = if version == "module-inline-trusted" {
            "<script type=\"module\">export const fixture = true;</script>".to_string()
        } else if version.starts_with("module-") {
            format!("<script type=\"module\" src=\"{module_url}\"></script>")
        } else {
            format!("<script src=\"./{script}\"></script>")
        };
        let preload = if version.starts_with("modulepreload-") {
            format!("<link rel=\"modulepreload\" href=\"{module_url}\">")
        } else {
            String::new()
        };
        let response = ([(header::CONTENT_TYPE, mime)], format!(
            "<!doctype html><html><head>{base}{preload}<link rel=\"stylesheet\" href=\"./styles.css\"></head>\
             <body><div id=\"app\"></div>{module}</body></html>"
        )).into_response();
        return frame_headers(response, version, &state.gateway_origin);
    }
    if file.ends_with(".js") {
        let mime = if version == "badesm" || version == "badresourcemime" {
            "text/html"
        } else {
            "application/javascript; charset=utf-8"
        };
        let body = if file == "main.js" {
            "window.__MICRO_APP__ = { mount: function () {}, unmount: function () {} };"
        } else {
            "export async function mount() {} export async function unmount() {}"
        };
        let mut response = ([(header::CONTENT_TYPE, mime)], body).into_response();
        let allow_origin = if version.ends_with("no-cors") {
            None
        } else if version.ends_with("null-cors") {
            Some("null")
        } else if version.ends_with("gateway-cors") {
            Some(state.gateway_origin.as_str())
        } else if version.ends_with("wrong-cors") {
            Some("https://wrong.invalid")
        } else {
            Some("*")
        };
        if let Some(origin) = allow_origin {
            response
                .headers_mut()
                .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.parse().unwrap());
        }
        return response;
    }
    if file.ends_with(".css") {
        let mime = if version == "badstylemime" {
            "text/html"
        } else {
            "text/css; charset=utf-8"
        };
        return ([(header::CONTENT_TYPE, mime)], "body { color: #222; }").into_response();
    }
    if file == "logo.png" {
        return (
            [(header::CONTENT_TYPE, "image/png")],
            vec![0x89_u8, 0x50, 0x4e, 0x47],
        )
            .into_response();
    }
    (StatusCode::NOT_FOUND, "fixture path missing").into_response()
}

impl HttpFixture {
    async fn start() -> Result<Self> {
        if std::env::var_os("MICRO_PROXY_TEST_NODE").is_some() {
            return node_fixture::start().await;
        }
        Self::start_at("[::1]:0").await
    }

    async fn start_at(bind: &str) -> Result<Self> {
        if std::env::var_os("MICRO_PROXY_TEST_NODE").is_some() {
            return node_fixture::start().await;
        }
        // IPv6 avoids inherited IPv4 proxy bypass assumptions. The release client must no_proxy().
        let denied_listener = TcpListener::bind(bind).await?;
        let denied_origin = format!("http://{}", denied_listener.local_addr()?);
        let listener = TcpListener::bind(bind).await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let denied_hits = Arc::new(AtomicUsize::new(0));
        let cors_requests = Arc::new(Mutex::new(Vec::new()));
        let denied_app =
            Router::new()
                .fallback(other_origin_resource)
                .with_state(OtherOriginState {
                    hits: denied_hits.clone(),
                    requests: cors_requests.clone(),
                    gateway_origin: origin.clone(),
                });
        let denied_server = tokio::spawn(async move {
            let _ = axum::serve(denied_listener, denied_app).await;
        });
        let requests = Arc::new(Mutex::new(Vec::new()));
        let request_headers = Arc::new(Mutex::new(Vec::new()));
        let unavailable = Arc::new(Mutex::new(HashSet::new()));
        let fixture = Router::new()
            .fallback(fixture_resource)
            .with_state(ResourceState {
                gateway_origin: origin.clone(),
                denied_origin: denied_origin.clone(),
                requests: requests.clone(),
                request_headers: request_headers.clone(),
                unavailable: unavailable.clone(),
            });
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, fixture).await;
        });
        Ok(Self {
            origin,
            other_origin: denied_origin,
            requests,
            request_headers,
            denied_hits,
            cors_requests,
            unavailable,
            servers: vec![server, denied_server],
            node: None,
        })
    }

    fn requested(&self, path: &str) -> Result<bool> {
        Ok(self
            .requests
            .lock()
            .map_err(|_| anyhow!("HTTP fixture request log poisoned"))?
            .iter()
            .any(|request| request == path))
    }

    fn set_unavailable(&self, path: &str, unavailable: bool) -> Result<()> {
        if let Some(node) = &self.node {
            node.set_unavailable(path, unavailable)?;
        }
        let mut paths = self
            .unavailable
            .lock()
            .map_err(|_| anyhow!("HTTP fixture availability lock poisoned"))?;
        if unavailable {
            paths.insert(path.to_string());
        } else {
            paths.remove(path);
        }
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires the test process to connect to its local IPv6 HTTP fixture"]
async fn release_fixture_real_ipv6_http() -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let http = HttpFixture::start().await?;
    let address = http
        .origin
        .trim_start_matches("http://")
        .parse::<std::net::SocketAddr>()?;
    let mut stream = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::net::TcpStream::connect(address),
    )
    .await
    .map_err(|_| anyhow!("raw IPv6 TCP connection timed out"))??;
    stream
        .write_all(
            format!(
                "GET /micro/v1/health HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await?;
    let mut bytes = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        stream.read_to_end(&mut bytes),
    )
    .await
    .map_err(|_| {
        anyhow!(
            "raw IPv6 TCP response timed out; handler hit={}",
            http.requested("/micro/v1/health").unwrap_or(false)
        )
    })??;
    let first_line = String::from_utf8_lossy(&bytes)
        .lines()
        .next()
        .unwrap_or("")
        .to_string();
    eprintln!(
        "raw IPv6 TCP fixture: {first_line}; handler hit={}",
        http.requested("/micro/v1/health")?
    );
    ensure!(
        first_line.contains("200"),
        "raw TCP fixture response failed"
    );
    let response = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(4))
        .build()?
        .get(format!("{}/micro/v1/health", http.origin))
        .send()
        .await?;
    eprintln!("reqwest no_proxy IPv6 fixture: {}", response.status());
    ensure!(
        response.status() == reqwest::StatusCode::OK,
        "reqwest fixture response failed"
    );
    Ok(())
}

async fn authorization_snapshot(pool: &PgPool) -> Result<BTreeMap<&'static str, Value>> {
    let mut result = BTreeMap::new();
    for table in [
        "tenant_applications",
        "organization_applications",
        "tenant_menus",
        "organization_menus",
        "role_menus",
        "user_roles",
        "membership_data_grants",
        "applications",
        "menus",
        "roles",
        "memberships",
        "users",
        "tenants",
        "organizations",
    ] {
        let rows: String = sqlx::query_scalar(&format!(
            "SELECT COALESCE(jsonb_agg(to_jsonb(source_row) ORDER BY to_jsonb(source_row)::text), '[]'::jsonb)::text \
             FROM {table} source_row"
        )).fetch_one(pool).await?;
        result.insert(table, serde_json::from_str(&rows)?);
    }
    Ok(result)
}

fn profile_application(profile: &Value, app: Uuid) -> Result<&Value> {
    profile["applications"]
        .as_array()
        .and_then(|apps| apps.iter().find(|row| row["id"] == app.to_string()))
        .ok_or_else(|| anyhow!("authorized application missing from profile"))
}

fn profile_contains(profile: &Value, app: Uuid) -> bool {
    profile["applications"]
        .as_array()
        .is_some_and(|apps| apps.iter().any(|row| row["id"] == app.to_string()))
}

struct ExpectedReleaseEntry<'a> {
    version: &'a str,
    entry_type: &'a str,
    entry_url: &'a str,
}

fn check_deployment(
    row: &Value,
    app: Uuid,
    environment: &str,
    active: Uuid,
    previous: Option<Uuid>,
    entry: ExpectedReleaseEntry<'_>,
) -> Result<()> {
    ensure!(
        row["applicationId"] == app.to_string() && row["environment"] == environment,
        "deployment has a different application or environment: {row}"
    );
    ensure!(
        row["activeReleaseId"] == active.to_string(),
        "deployment has a different active release: {row}"
    );
    ensure!(
        row["previousReleaseId"] == json!(previous),
        "deployment previous release is incorrect: {row}"
    );
    ensure!(
        row["version"] == entry.version
            && row["entryType"] == entry.entry_type
            && row["entryUrl"] == entry.entry_url,
        "deployment effective entry is incorrect: {row}"
    );
    ensure!(
        row["updatedAt"].as_str().is_some(),
        "deployment has no updatedAt"
    );
    Ok(())
}

fn check_profile_entry(
    profile: &Value,
    app: Uuid,
    release: Uuid,
    version: &str,
    entry_type: &str,
    entry_url: &str,
) -> Result<()> {
    let row = profile_application(profile, app)?;
    ensure!(
        row["entryVersionId"] == release.to_string() && row["deploymentVersion"] == version,
        "profile release metadata differs from the deployment: {row}"
    );
    ensure!(
        row["entryType"] == entry_type && row["entryUrl"] == entry_url,
        "profile did not use the active release entry: {row}"
    );
    Ok(())
}

fn check_context_application(
    context: &Value,
    app: Uuid,
    mode: &str,
    release: Option<Uuid>,
    version: Option<&str>,
    entry_type: &str,
    entry_url: &str,
) -> Result<()> {
    let row = &context["application"];
    ensure!(
        row["id"] == app.to_string() && row["runtimeMode"] == mode,
        "application context did not expose the current identity/runtime mode: {row}"
    );
    ensure!(
        row["entryType"] == entry_type && row["entryUrl"] == entry_url,
        "application context did not expose the current effective entry: {row}"
    );
    ensure!(
        row.get("entryVersionId").is_some()
            && row.get("deploymentVersion").is_some()
            && row["entryVersionId"] == json!(release)
            && row["deploymentVersion"] == json!(version),
        "application context did not expose the current deployment metadata: {row}"
    );
    Ok(())
}

fn check_context_authorization(before: &Value, after: &Value) -> Result<()> {
    for field in [
        "contextVersion",
        "contractVersion",
        "appCode",
        "user",
        "tenant",
        "organization",
        "roles",
        "permissions",
        "dataGrants",
        "menus",
    ] {
        ensure!(
            before[field] == after[field],
            "deployment or runtime metadata changed context authorization field {field}"
        );
    }
    ensure!(
        before["application"]["id"] == after["application"]["id"]
            && before["application"]["name"] == after["application"]["name"],
        "deployment or runtime metadata changed application context identity"
    );
    Ok(())
}

fn expect_sqlstate(
    result: std::result::Result<PgQueryResult, sqlx::Error>,
    code: &str,
    label: &str,
) -> Result<()> {
    match result {
        Err(error)
            if error
                .as_database_error()
                .and_then(|error| error.code())
                .as_deref()
                == Some(code) =>
        {
            Ok(())
        }
        Err(_) => Err(anyhow!("{label} failed with a different SQLSTATE")),
        Ok(_) => Err(anyhow!("database accepted {label}")),
    }
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn release_registration_and_platform_scope() -> Result<()> {
    let database = TestDatabase::create().await?;
    let result: Result<()> = async {
        let http = HttpFixture::start().await?;
        let h = Harness::create(&database.pool, &http.origin).await?;
        let snapshot = authorization_snapshot(&database.pool).await?;
        let draft = h.register(release_body("v1", "HTML")).await?;
        let draft_id = release_id(&draft)?;
        ensure!(
            draft["validationStatus"] == "DRAFT" && draft["checkedAt"].is_null(),
            "new release is not an unchecked draft: {draft}"
        );
        // Both tenant administrators have a real system:application permission. The platform gate
        // must still protect every release/deployment operation, including read operations.
        for token in [&h.a, &h.b] {
            let permissions =
                successful_call(&h.app, Method::GET, "/api/auth/permissions", token, None).await?;
            ensure!(
                permissions["permissions"]
                    .as_array()
                    .is_some_and(|items| items.contains(&json!("system:application"))),
                "tenant fixture lacks management permission"
            );
            for (method, path, body) in [
                (
                    Method::GET,
                    format!("/api/applications/{}/releases", h.main_app),
                    None,
                ),
                (
                    Method::POST,
                    format!("/api/applications/{}/releases", h.main_app),
                    Some(release_body("v2", "HTML")),
                ),
                (
                    Method::POST,
                    format!(
                        "/api/applications/{}/releases/{draft_id}/preflight",
                        h.main_app
                    ),
                    None,
                ),
                (
                    Method::POST,
                    format!(
                        "/api/applications/{}/releases/{draft_id}/activate",
                        h.main_app
                    ),
                    None,
                ),
                (
                    Method::GET,
                    format!("/api/applications/{}/deployments", h.main_app),
                    None,
                ),
                (
                    Method::POST,
                    format!(
                        "/api/applications/{}/deployments/{ENVIRONMENT}/rollback",
                        h.main_app
                    ),
                    None,
                ),
            ] {
                let (status, body) = call(&h.app, method, &path, token, body).await?;
                ensure!(
                    status == StatusCode::FORBIDDEN,
                    "tenant accessed {path}: {status} {body}"
                );
            }
        }
        let (status, _) = call(
            &h.app,
            Method::GET,
            &format!("/api/applications/{}/releases", h.main_app),
            "",
            None,
        )
        .await?;
        ensure!(
            status == StatusCode::UNAUTHORIZED,
            "anonymous user listed releases"
        );
        let (status, _) = call(
            &h.app,
            Method::POST,
            &format!(
                "/api/applications/{}/releases/{draft_id}/activate",
                h.main_app
            ),
            &h.platform,
            None,
        )
        .await?;
        ensure!(
            status == StatusCode::CONFLICT,
            "an unchecked draft was activated"
        );
        let (status, _) = call(
            &h.app,
            Method::POST,
            &format!(
                "/api/applications/{}/deployments/{ENVIRONMENT}/rollback",
                h.main_app
            ),
            &h.platform,
            None,
        )
        .await?;
        ensure!(
            status == StatusCode::NOT_FOUND,
            "rollback of an undeployed environment was accepted"
        );

        let baseline = release_body("v1", "HTML");
        let mut invalid = Vec::new();
        for (field, value) in [
            ("entryType", json!("SCRIPT")),
            ("entryUrl", json!("")),
            ("entryUrl", json!("javascript:alert(1)")),
            ("entryUrl", json!("//blocked.invalid/micro/v1/index.html")),
            (
                "entryUrl",
                json!("https://blocked.invalid/micro/v1/index.html"),
            ),
            (
                "entryUrl",
                json!(format!("{}/micro/latest/index.html", http.origin)),
            ),
            (
                "entryUrl",
                json!(format!("{}/micro/v1-extra/index.html", http.origin)),
            ),
            (
                "entryUrl",
                json!(format!("{}/micro/v1/../latest/index.html", http.origin)),
            ),
            (
                "entryUrl",
                json!(format!("{}/micro/v1%2f..%2flatest/index.html", http.origin)),
            ),
            ("version", json!("..")),
            ("version", json!("v/1")),
            ("version", json!("v 1")),
            ("version", json!("x".repeat(65))),
            ("environment", json!("Production")),
            ("environment", json!("../production")),
            ("environment", json!("")),
            ("environment", json!("x".repeat(33))),
            (
                "healthUrl",
                json!("https://blocked.invalid/micro/v1/health"),
            ),
            (
                "resources",
                json!([{"url":"/micro/v1/main.js","kind":"MODULE"}]),
            ),
            (
                "resources",
                json!([{"url":"https://blocked.invalid/micro/v1/main.js","kind":"SCRIPT"}]),
            ),
        ] {
            let mut body = baseline.clone();
            body[field] = value;
            invalid.push((field, body));
        }
        let mut production_http = baseline.clone();
        production_http["environment"] = json!("production");
        production_http["entryUrl"] = json!(format!("{}/micro/v1/index.html", http.origin));
        invalid.push(("production HTTP entry", production_http));
        for (label, body) in invalid {
            let (status, response) = call(
                &h.app,
                Method::POST,
                &format!("/api/applications/{}/releases", h.main_app),
                &h.platform,
                Some(body),
            )
            .await?;
            ensure!(
                status == StatusCode::BAD_REQUEST,
                "invalid {label} was not rejected: {status} {response}"
            );
        }
        let (status, _) = call(
            &h.app,
            Method::POST,
            &format!("/api/applications/{}/releases", h.main_app),
            &h.platform,
            Some(baseline),
        )
        .await?;
        ensure!(
            status == StatusCode::CONFLICT,
            "duplicate environment/version was registered"
        );
        let releases = successful_call(
            &h.app,
            Method::GET,
            &format!("/api/applications/{}/releases", h.main_app),
            &h.platform,
            None,
        )
        .await?;
        ensure!(
            releases
                .as_array()
                .is_some_and(|rows| rows.len() == 1 && rows[0]["id"] == draft_id.to_string()),
            "rejected registrations changed the release catalog: {releases}"
        );
        ensure!(
            http.requests
                .lock()
                .map_err(|_| anyhow!("HTTP fixture log poisoned"))?
                .is_empty(),
            "registration or a forbidden operation fetched a remote resource"
        );
        ensure!(
            snapshot == authorization_snapshot(&database.pool).await?,
            "registration or scope rejection changed existing application authorization"
        );
        Ok(())
    }
    .await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn release_preflight_checks_entry_resources_health_and_redirects() -> Result<()> {
    let database = TestDatabase::create().await?;
    let result: Result<()> = async {
        let http = HttpFixture::start().await?;
        let h = Harness::create(&database.pool, &http.origin).await?;
        let mut html = release_body("v1", "HTML");
        // Absolute entries and gateway-relative resources share the same allowlist.
        html["entryUrl"] = json!(format!("{}/micro/v1/index.html", http.origin));
        html["resources"] = json!([{"url":"/micro/v1/logo.png","kind":"ASSET"}]);
        let html_id = release_id(&h.register(html).await?)?;
        h.preflight(html_id, "PASSED").await?;
        for path in ["/micro/v1/index.html", "/micro/v1/main.js", "/micro/v1/styles.css",
            "/micro/v1/logo.png", "/micro/v1/health"] {
            ensure!(http.requested(path)?, "HTML preflight did not request {path}");
        }
        let mut failures = vec![
            release_body("bad404", "HTML"), release_body("badmime", "HTML"),
            release_body("badresource404", "HTML"), release_body("badresourcemime", "HTML"),
            release_body("badstylemime", "HTML"), release_body("badhealth", "HTML"),
            release_body("badredirect", "HTML"), release_body("loopredirect", "HTML"),
            release_body("basechange", "HTML"), release_body("directoryredirect", "HTML"),
        ];
        let mut missing_asset = release_body("badasset404", "HTML");
        missing_asset["resources"] = json!([{"url":"/micro/badasset404/missing.bin","kind":"ASSET"}]);
        failures.push(missing_asset);
        let mut outside_version = release_body("badresourcepath", "HTML");
        outside_version["resources"] = json!([{"url":"/micro/latest/main.js","kind":"SCRIPT"}]);
        failures.push(outside_version);
        for body in failures {
            let version = body["version"].as_str().ok_or_else(|| anyhow!("fixture version missing"))?.to_string();
            let id = release_id(&h.register(body).await?)?;
            let checked = h.preflight(id, "FAILED").await?;
            ensure!(!checked["validationResult"].as_object().is_some_and(|value| value.is_empty()),
                "failed {version} preflight has no diagnostic result");
            let stored: (String, bool) = sqlx::query_as(
                "SELECT validation_status, checked_at IS NOT NULL FROM application_releases WHERE id = $1",
            ).bind(id).fetch_one(&database.pool).await?;
            ensure!(stored == ("FAILED".into(), true), "failed preflight was not persisted for {version}");
            let (status, response) = call(&h.app, Method::POST,
                &format!("/api/applications/{}/releases/{id}/activate", h.main_app), &h.platform, None).await?;
            ensure!(status == StatusCode::CONFLICT, "failed {version} release was activated: {status} {response}");
        }
        ensure!(http.denied_hits.load(Ordering::SeqCst) == 0,
            "preflight followed a redirect to an origin outside the allowlist");
        let deployments = successful_call(&h.app, Method::GET,
            &format!("/api/applications/{}/deployments", h.main_app), &h.platform, None).await?;
        ensure!(deployments.as_array().is_some_and(Vec::is_empty),
            "preflight or rejected activation created a deployment");
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn release_trusted_cross_origin_preflight_requires_gateway_cors() -> Result<()> {
    let database = TestDatabase::create().await?;
    let result: Result<()> = async {
        let http = HttpFixture::start().await?;
        let h = Harness::create_with_origins(
            &database.pool,
            &http.origin,
            vec![http.origin.clone(), http.other_origin.clone()],
        )
        .await?;
        for version in ["trusted-no-cors", "trusted-wrong-cors", "trusted-null-cors"] {
            let mut body = release_body(version, "HTML");
            body["entryUrl"] = json!(format!("{}/micro/{version}/index.html", http.other_origin));
            let id = release_id(&h.register(body).await?)?;
            h.preflight(id, "FAILED").await?;
            let (status, _) = call(
                &h.app,
                Method::POST,
                &format!("/api/applications/{}/releases/{id}/activate", h.main_app),
                &h.platform,
                None,
            )
            .await?;
            ensure!(
                status == StatusCode::CONFLICT,
                "an HTML entry with missing/wrong gateway CORS was activated for {version}"
            );
        }
        let mut cross_script = release_body("trusted-resource-no-cors", "HTML");
        cross_script["resources"] = json!([{
            "url":format!("{}/micro/trusted-resource-no-cors/dep.js", http.other_origin),
            "kind":"SCRIPT",
        }]);
        let cross_script_id = release_id(&h.register(cross_script).await?)?;
        h.preflight(cross_script_id, "FAILED").await?;
        let mut star = release_body("trusted-star-cors", "HTML");
        let star_url = format!("{}/micro/trusted-star-cors/index.html", http.other_origin);
        star["entryUrl"] = json!(star_url);
        let star_id = release_id(&h.register(star).await?)?;
        h.preflight(star_id, "PASSED").await?;
        check_deployment(
            &h.activate(star_id).await?,
            h.main_app,
            ENVIRONMENT,
            star_id,
            None,
            ExpectedReleaseEntry {
                version: "trusted-star-cors",
                entry_type: "HTML",
                entry_url: &star_url,
            },
        )?;
        let mut gateway = release_body("trusted-gateway-cors", "HTML");
        let gateway_url = format!(
            "{}/micro/trusted-gateway-cors/index.html",
            http.other_origin
        );
        gateway["entryUrl"] = json!(gateway_url);
        let gateway_id = release_id(&h.register(gateway).await?)?;
        h.preflight(gateway_id, "PASSED").await?;
        check_deployment(
            &h.activate(gateway_id).await?,
            h.main_app,
            ENVIRONMENT,
            gateway_id,
            Some(star_id),
            ExpectedReleaseEntry {
                version: "trusted-gateway-cors",
                entry_type: "HTML",
                entry_url: &gateway_url,
            },
        )?;
        check_profile_entry(
            &h.profile(&h.a).await?,
            h.main_app,
            gateway_id,
            "trusted-gateway-cors",
            "HTML",
            &gateway_url,
        )?;
        let requests = http
            .cors_requests
            .lock()
            .map_err(|_| anyhow!("CORS fixture request log poisoned"))?;
        ensure!(
            requests
                .iter()
                .any(|(path, _)| path.ends_with("/trusted-gateway-cors/main.js"))
                && requests
                    .iter()
                    .any(|(path, _)| path.ends_with("/trusted-gateway-cors/styles.css")),
            "cross-origin HTML scripts and styles were not checked"
        );
        ensure!(
            requests
                .iter()
                .filter(|(path, _)| path.contains("trusted-"))
                .all(|(_, origin)| origin.as_deref() == Some(http.origin.as_str())),
            "TRUSTED preflight did not send the gateway Origin to the external resource server"
        );
        Ok(())
    }
    .await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn release_html_runtime_checks_native_modules_and_frame_headers() -> Result<()> {
    let database = TestDatabase::create().await?;
    let result: Result<()> = async {
        let http = HttpFixture::start().await?;
        let h = Harness::create_with_origins(
            &database.pool,
            &http.origin,
            vec![http.origin.clone(), http.other_origin.clone()],
        )
        .await?;
        let classic = release_id(&h.register(release_body("classic-no-cors", "HTML")).await?)?;
        h.preflight(classic, "PASSED").await?;
        // qiankun consumes classic/UMD scripts, so CORS cannot make native modules compatible.
        let native_cases = [
            "module-star-cors",
            "module-null-cors",
            "module-gateway-cors",
            "module-inline-trusted",
            "modulepreload-star-cors",
            "modulepreload-gateway-cors",
            "module-cdn-star-cors",
            "modulepreload-cdn-star-cors",
        ];
        for version in native_cases {
            let mut body = release_body(version, "HTML");
            let script = if version.contains("-cdn-") {
                format!("{}/micro/{version}/entry.js", http.other_origin)
            } else {
                format!("/micro/{version}/entry.js")
            };
            // The ordinary manifest row must not hide the stronger module/modulepreload check.
            body["resources"] = json!([{"url":script,"kind":"SCRIPT"}]);
            let id = release_id(&h.register(body).await?)?;
            let checked = h.preflight(id, "FAILED").await?;
            ensure!(
                checked["validationResult"]["checks"].as_array().is_some_and(|checks| {
                    checks.iter().any(|check| check["kind"] == "HTML_MODULE"
                        && check["passed"] == false
                        && check["message"].as_str().is_some_and(|message| message.contains("qiankun")))
                }),
                "native-module rejection did not explain the qiankun incompatibility for {version}: {checked}"
            );
            let (status, _) = call(
                &h.app,
                Method::POST,
                &format!("/api/applications/{}/releases/{id}/activate", h.main_app),
                &h.platform,
                None,
            )
            .await?;
            ensure!(
                status == StatusCode::CONFLICT,
                "qiankun HTML containing native module code was activated for {version}"
            );
        }
        let local = http
            .request_headers
            .lock()
            .map_err(|_| anyhow!("local HTTP request log poisoned"))?
            .clone();
        let remote = http
            .cors_requests
            .lock()
            .map_err(|_| anyhow!("external HTTP request log poisoned"))?
            .clone();
        for version in native_cases {
            let suffix = format!("/micro/{version}/entry.js");
            let requests: Vec<_> = local
                .iter()
                .chain(remote.iter())
                .filter(|(path, _)| path == &suffix)
                .collect();
            ensure!(
                !requests.is_empty()
                    && requests
                        .iter()
                        .all(|(_, origin)| origin.as_deref() == Some(http.origin.as_str())),
                "qiankun resource checks did not use the gateway Origin for {version}"
            );
        }
        // qiankun fetches the HTML; frame navigation restrictions do not block that fetch.
        for version in ["xfo-deny", "csp-none", "csp-multiple-none", "csp-combined-none"] {
            let id = release_id(&h.register(release_body(version, "HTML")).await?)?;
            h.preflight(id, "PASSED").await?;
        }
        Ok(())
    }
    .await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn release_activation_and_rollback_preserve_identity_and_authorization() -> Result<()> {
    let database = TestDatabase::create().await?;
    let result: Result<()> = async {
        let http = HttpFixture::start().await?;
        let h = Harness::create(&database.pool, &http.origin).await?;
        let initial_a = h.profile(&h.a).await?;
        let initial_b = h.profile(&h.b).await?;
        ensure!(profile_contains(&initial_a, h.main_app) && profile_contains(&initial_a, h.no_role_app),
            "profile does not honor tenant and organization application grants");
        ensure!(!profile_contains(&initial_a, h.tenant_only_app) && !profile_contains(&initial_a, h.org_only_app)
            && !profile_contains(&initial_a, h.other_app), "profile bypassed a tenant/organization application grant");
        ensure!(profile_contains(&initial_b, h.main_app) && profile_contains(&initial_b, h.other_app)
            && !profile_contains(&initial_b, h.no_role_app), "B profile leaked an A-only application");
        let legacy = profile_application(&initial_a, h.main_app)?;
        ensure!(legacy["entryType"] == "HTML" && legacy["entryUrl"] == "/legacy/release-app/index.html"
            && legacy.get("entryVersionId").is_some_and(Value::is_null)
            && legacy.get("deploymentVersion").is_some_and(Value::is_null)
            && legacy["runtimeMode"] == "TRUSTED",
            "undeployed application lost its legacy entry or null deployment metadata");
        let menus_path = format!("/api/auth/menus?applicationId={}", h.main_app);
        let initial_menus_a = successful_call(&h.app, Method::GET, &menus_path, &h.a, None).await?;
        let initial_menus_b = successful_call(&h.app, Method::GET, &menus_path, &h.b, None).await?;
        let initial_permissions_a = successful_call(&h.app, Method::GET, "/api/auth/permissions", &h.a, None).await?;
        let initial_permissions_b = successful_call(&h.app, Method::GET, "/api/auth/permissions", &h.b, None).await?;
        let no_role_menus = successful_call(&h.app, Method::GET,
            &format!("/api/auth/menus?applicationId={}", h.no_role_app), &h.a, None).await?;
        ensure!(no_role_menus["menus"].as_array().is_some_and(Vec::is_empty),
            "the profile fixture unexpectedly has role authorization for the no-role application");
        oauth_client(&database.pool, h.main_app, "release-lifecycle-client").await?;
        let app_token = h.app_token(&h.a, "release-app").await?;
        let initial_context = successful_call(&h.app, Method::GET, "/api/apps/release-app/context", &app_token, None).await?;
        check_context_application(&initial_context, h.main_app, "TRUSTED", None, None,
            "HTML", "/legacy/release-app/index.html")?;
        let snapshot = authorization_snapshot(&database.pool).await?;

        let v1 = release_id(&h.register(release_body("v1", "HTML")).await?)?;
        h.preflight(v1, "PASSED").await?;
        let deployed_v1 = h.activate(v1).await?;
        check_deployment(&deployed_v1, h.main_app, ENVIRONMENT, v1, None,
            ExpectedReleaseEntry { version: "v1", entry_type: "HTML", entry_url: "/micro/v1/index.html" })?;
        let (status, _) = call(&h.app, Method::POST,
            &format!("/api/applications/{}/deployments/{ENVIRONMENT}/rollback", h.main_app), &h.platform, None).await?;
        ensure!(status == StatusCode::CONFLICT, "rollback without a previous release was accepted");
        let profile_v1_a = h.profile(&h.a).await?;
        let profile_v1_b = h.profile(&h.b).await?;
        for profile in [&profile_v1_a, &profile_v1_b] {
            check_profile_entry(profile, h.main_app, v1, "v1", "HTML", "/micro/v1/index.html")?;
        }
        let context_v1 = successful_call(&h.app, Method::GET, "/api/apps/release-app/context", &app_token, None).await?;
        check_context_application(&context_v1, h.main_app, "TRUSTED", Some(v1), Some("v1"),
            "HTML", "/micro/v1/index.html")?;
        check_context_authorization(&initial_context, &context_v1)?;
        ensure!(snapshot == authorization_snapshot(&database.pool).await?, "v1 activation changed identity or grants");

        let v2 = release_id(&h.register(release_body("v2", "HTML")).await?)?;
        h.preflight(v2, "PASSED").await?;
        // Availability can change after preflight; activation must check again before moving pointers.
        http.set_unavailable("/micro/v2/health", true)?;
        let (status, _) = call(&h.app, Method::POST,
            &format!("/api/applications/{}/releases/{v2}/activate", h.main_app), &h.platform, None).await?;
        ensure!(status == StatusCode::CONFLICT, "activation ignored an unavailable health check after preflight");
        check_profile_entry(&h.profile(&h.a).await?, h.main_app, v1, "v1", "HTML", "/micro/v1/index.html")?;
        let failed_status: String = sqlx::query_scalar("SELECT validation_status FROM application_releases WHERE id = $1")
            .bind(v2).fetch_one(&database.pool).await?;
        ensure!(failed_status == "FAILED", "activation's failed fresh preflight was not persisted");
        http.set_unavailable("/micro/v2/health", false)?;
        h.preflight(v2, "PASSED").await?;
        let deployed_v2 = h.activate(v2).await?;
        check_deployment(&deployed_v2, h.main_app, ENVIRONMENT, v2, Some(v1),
            ExpectedReleaseEntry { version: "v2", entry_type: "HTML", entry_url: "/micro/v2/index.html" })?;
        for token in [&h.a, &h.b] {
            let profile = h.profile(token).await?;
            check_profile_entry(&profile, h.main_app, v2, "v2", "HTML", "/micro/v2/index.html")?;
            let row = profile_application(&profile, h.main_app)?;
            for field in ["id", "code", "name", "icon", "basePath", "runtimeMode"] {
                ensure!(row[field] == legacy[field], "v2 changed the application {field}");
            }
        }
        ensure!(snapshot == authorization_snapshot(&database.pool).await?, "v2 activation changed identity or grants");
        let context_v2 = successful_call(&h.app, Method::GET, "/api/apps/release-app/context", &app_token, None).await?;
        check_context_application(&context_v2, h.main_app, "TRUSTED", Some(v2), Some("v2"),
            "HTML", "/micro/v2/index.html")?;
        check_context_authorization(&initial_context, &context_v2)?;

        let mut staging = release_body("staging-v1", "HTML");
        staging["environment"] = json!("staging");
        let staging_id = release_id(&h.register(staging).await?)?;
        h.preflight(staging_id, "PASSED").await?;
        let staging_deployment = h.activate(staging_id).await?;
        check_deployment(&staging_deployment, h.main_app, "staging", staging_id, None,
            ExpectedReleaseEntry { version: "staging-v1", entry_type: "HTML", entry_url: "/micro/staging-v1/index.html" })?;
        check_profile_entry(&h.profile(&h.a).await?, h.main_app, v2, "v2", "HTML", "/micro/v2/index.html")?;
        let listed = successful_call(&h.app, Method::GET,
            &format!("/api/applications/{}/deployments", h.main_app), &h.platform, None).await?;
        let deployments = listed.as_array().ok_or_else(|| anyhow!("deployments response is not an array"))?;
        ensure!(deployments.len() == 2, "deployment listing lost an environment");
        let dev = deployments.iter().find(|row| row["environment"] == ENVIRONMENT)
            .ok_or_else(|| anyhow!("development deployment missing"))?;
        check_deployment(dev, h.main_app, ENVIRONMENT, v2, Some(v1),
            ExpectedReleaseEntry { version: "v2", entry_type: "HTML", entry_url: "/micro/v2/index.html" })?;

        http.set_unavailable("/micro/v1/main.js", true)?;
        let (status, _) = call(&h.app, Method::POST,
            &format!("/api/applications/{}/deployments/{ENVIRONMENT}/rollback", h.main_app), &h.platform, None).await?;
        ensure!(status == StatusCode::CONFLICT, "rollback ignored unavailable previous-version resources");
        check_profile_entry(&h.profile(&h.a).await?, h.main_app, v2, "v2", "HTML", "/micro/v2/index.html")?;
        let failed_previous: String = sqlx::query_scalar("SELECT validation_status FROM application_releases WHERE id = $1")
            .bind(v1).fetch_one(&database.pool).await?;
        ensure!(failed_previous == "FAILED", "rollback's failed fresh preflight was not persisted");
        http.set_unavailable("/micro/v1/main.js", false)?;
        let rollback = successful_call(&h.app, Method::POST,
            &format!("/api/applications/{}/deployments/{ENVIRONMENT}/rollback", h.main_app),
            &h.platform, None).await?;
        check_deployment(&rollback, h.main_app, ENVIRONMENT, v1, Some(v2),
            ExpectedReleaseEntry { version: "v1", entry_type: "HTML", entry_url: "/micro/v1/index.html" })?;
        ensure!(h.profile(&h.a).await? == profile_v1_a && h.profile(&h.b).await? == profile_v1_b,
            "rollback did not restore the exact v1 profile for both tenants");
        let context_rollback = successful_call(&h.app, Method::GET, "/api/apps/release-app/context", &app_token, None).await?;
        check_context_application(&context_rollback, h.main_app, "TRUSTED", Some(v1), Some("v1"),
            "HTML", "/micro/v1/index.html")?;
        check_context_authorization(&initial_context, &context_rollback)?;
        ensure!(successful_call(&h.app, Method::GET, &menus_path, &h.a, None).await? == initial_menus_a
            && successful_call(&h.app, Method::GET, &menus_path, &h.b, None).await? == initial_menus_b,
            "release switch or rollback changed effective menus");
        ensure!(successful_call(&h.app, Method::GET, "/api/auth/permissions", &h.a, None).await? == initial_permissions_a
            && successful_call(&h.app, Method::GET, "/api/auth/permissions", &h.b, None).await? == initial_permissions_b,
            "release switch or rollback changed roles or permissions");
        ensure!(snapshot == authorization_snapshot(&database.pool).await?,
            "release operations changed existing tenant, organization, role, data grant or application rows");
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn release_qiankun_defaults_and_reconfiguration_preserve_authorization() -> Result<()> {
    let database = TestDatabase::create().await?;
    let result: Result<()> = async {
        let http = HttpFixture::start().await?;
        let h = Harness::create(&database.pool, &http.origin).await?;
        let created = successful_call(
            &h.app,
            Method::POST,
            "/api/applications",
            &h.platform,
            Some(json!({
                "code":"release-api-created", "name":"Release API created",
                "entryUrl":"/legacy/api-created/index.html",
            })),
        )
        .await?;
        ensure!(
            created["entryType"] == "HTML"
                && created["runtimeMode"] == "TRUSTED"
                && created["basePath"] == "/micro/release-api-created",
            "new applications did not default to qiankun"
        );
        oauth_client(&database.pool, h.main_app, "release-trust-client").await?;
        oauth_client(&database.pool, h.other_app, "release-other-trust-client").await?;
        let original_token = h.app_token(&h.a, "release-app").await?;
        let unrelated_token = h.app_token(&h.b, "release-other").await?;
        let initial_context = successful_call(
            &h.app,
            Method::GET,
            "/api/apps/release-app/context",
            &original_token,
            None,
        )
        .await?;
        check_context_application(
            &initial_context,
            h.main_app,
            "TRUSTED",
            None,
            None,
            "HTML",
            "/legacy/release-app/index.html",
        )?;
        let before_rejections = authorization_snapshot(&database.pool).await?;
        let app_path = format!("/api/applications/{}", h.main_app);
        for token in [&h.a, &h.b] {
            ensure!(
                profile_application(&h.profile(token).await?, h.main_app)?["runtimeMode"]
                    == "TRUSTED",
                "profile did not expose the default qiankun mode"
            );
            let (status, _) = call(
                &h.app,
                Method::PUT,
                &app_path,
                token,
                Some(json!({"runtimeMode":"TRUSTED"})),
            )
            .await?;
            ensure!(
                status == StatusCode::FORBIDDEN,
                "a tenant administrator changed platform execution configuration"
            );
        }
        for patch in [
            json!({"runtimeMode":"UNRESTRICTED"}),
            json!({"runtimeMode":"IFRAME"}),
            json!({"entryType":"ESM","runtimeMode":"TRUSTED"}),
        ] {
            let (status, _) =
                call(&h.app, Method::PUT, &app_path, &h.platform, Some(patch)).await?;
            ensure!(
                status == StatusCode::BAD_REQUEST,
                "an unsupported qiankun runtime or entry type was accepted"
            );
        }
        ensure!(
            before_rejections == authorization_snapshot(&database.pool).await?,
            "rejected loader changes modified application metadata or authorization"
        );
        let unchanged_context = successful_call(
            &h.app,
            Method::GET,
            "/api/apps/release-app/context",
            &original_token,
            None,
        )
        .await?;
        check_context_authorization(&initial_context, &unchanged_context)?;
        let mut before = before_rejections;
        before.remove("applications");

        // Reconstruct a disabled pre-upgrade configuration. It must remain inert until
        // a platform administrator explicitly supplies a complete qiankun configuration.
        sqlx::query(
            "UPDATE applications SET entry_type='ESM',runtime_mode='IFRAME',status='DISABLED', \
             entry_url='/legacy/release-app/entry.js' WHERE id=$1",
        )
        .bind(h.main_app)
        .execute(&database.pool)
        .await?;
        for token in [&h.a, &h.b] {
            ensure!(
                !profile_contains(&h.profile(token).await?, h.main_app),
                "a disabled historical application remained in the profile"
            );
        }
        for patch in [
            json!({"status":"ENABLED"}),
            json!({"entryType":"HTML"}),
            json!({"runtimeMode":"TRUSTED"}),
        ] {
            let (status, _) =
                call(&h.app, Method::PUT, &app_path, &h.platform, Some(patch)).await?;
            ensure!(
                status == StatusCode::BAD_REQUEST,
                "an incomplete legacy reconfiguration was accepted"
            );
        }
        let repaired = successful_call(
            &h.app,
            Method::PUT,
            &app_path,
            &h.platform,
            Some(json!({
                "entryType":"HTML", "runtimeMode":"TRUSTED",
                "entryUrl":"/legacy/release-app/index.html", "status":"DISABLED",
            })),
        )
        .await?;
        ensure!(
            repaired["entryType"] == "HTML"
                && repaired["runtimeMode"] == "TRUSTED"
                && repaired["status"] == "DISABLED",
            "explicit qiankun reconfiguration implicitly enabled the application"
        );
        ensure!(
            !profile_contains(&h.profile(&h.a).await?, h.main_app),
            "reconfigured application appeared before explicit enablement"
        );
        successful_call(
            &h.app,
            Method::PUT,
            &app_path,
            &h.platform,
            Some(json!({"status":"ENABLED"})),
        )
        .await?;
        let fresh_token = h.app_token(&h.a, "release-app").await?;
        let restored_context = successful_call(
            &h.app,
            Method::GET,
            "/api/apps/release-app/context",
            &fresh_token,
            None,
        )
        .await?;
        check_context_application(
            &restored_context,
            h.main_app,
            "TRUSTED",
            None,
            None,
            "HTML",
            "/legacy/release-app/index.html",
        )?;
        check_context_authorization(&initial_context, &restored_context)?;
        let (status, _) = call(
            &h.app,
            Method::GET,
            "/api/apps/release-app/context",
            &original_token,
            None,
        )
        .await?;
        ensure!(
            status == StatusCode::UNAUTHORIZED,
            "reconfiguration revived the token revoked by historical application disablement"
        );
        // Idempotent configuration updates preserve the fresh token; revocation stays
        // scoped to the changed application and preserves unrelated platform sessions.
        successful_call(
            &h.app,
            Method::PUT,
            &app_path,
            &h.platform,
            Some(json!({"runtimeMode":"TRUSTED"})),
        )
        .await?;
        successful_call(
            &h.app,
            Method::GET,
            "/api/apps/release-app/context",
            &fresh_token,
            None,
        )
        .await?;
        successful_call(
            &h.app,
            Method::GET,
            "/api/apps/release-other/context",
            &unrelated_token,
            None,
        )
        .await?;
        h.profile(&h.platform).await?;
        for token in [&h.a, &h.b] {
            let profile = h.profile(token).await?;
            let application = profile_application(&profile, h.main_app)?;
            ensure!(
                application["entryType"] == "HTML" && application["runtimeMode"] == "TRUSTED",
                "restored profile did not expose qiankun configuration"
            );
        }
        let mut after = authorization_snapshot(&database.pool).await?;
        after.remove("applications");
        ensure!(
            before == after,
            "qiankun reconfiguration changed existing identity or authorization"
        );
        Ok(())
    }
    .await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn release_database_enforces_context_and_immutable_definition() -> Result<()> {
    let database = TestDatabase::create().await?;
    let result: Result<()> = async {
        let http = HttpFixture::start().await?;
        let h = Harness::create(&database.pool, &http.origin).await?;
        let a = release_id(&h.register(release_body("v1", "HTML")).await?)?;
        let b = release_id(&h.register_for(h.other_app, release_body("v1", "HTML")).await?)?;
        let mut staging = release_body("v1", "HTML");
        staging["environment"] = json!("staging");
        let staging = release_id(&h.register(staging).await?)?;
        for (active, previous, label) in [
            (b, None, "active release from another application"),
            (staging, None, "active release from another environment"),
            (a, Some(b), "previous release from another application"),
            (a, Some(staging), "previous release from another environment"),
        ] {
            expect_sqlstate(sqlx::query(
                "INSERT INTO application_deployments \
                 (application_id, environment, active_release_id, previous_release_id) VALUES ($1, $2, $3, $4)",
            ).bind(h.main_app).bind(ENVIRONMENT).bind(active).bind(previous)
                .execute(&database.pool).await, "23503", label)?;
        }
        expect_sqlstate(sqlx::query(
            "INSERT INTO application_deployments \
             (application_id, environment, active_release_id, previous_release_id) VALUES ($1, $2, $3, $3)",
        ).bind(h.main_app).bind(ENVIRONMENT).bind(a).execute(&database.pool).await,
            "23514", "active release repeated as previous release")?;
        let before: String = sqlx::query_scalar("SELECT to_jsonb(row)::text FROM application_releases row WHERE id = $1")
            .bind(a).fetch_one(&database.pool).await?;
        // Every field in the immutable definition is checked; validation fields remain writable.
        for (change, label) in [
            ("id = gen_random_uuid()", "release id mutation"),
            ("version = 'v2'", "version mutation"),
            ("entry_type = 'ESM'", "entry type mutation"),
            ("entry_url = '/micro/v1/other.html'", "entry URL mutation"),
            ("health_url = '/micro/v1/other-health'", "health URL mutation"),
            ("resources = '[{\"url\":\"/micro/v1/other.js\",\"kind\":\"SCRIPT\"}]'::jsonb", "resources mutation"),
            ("environment = 'production'", "environment mutation"),
            ("created_at = created_at + interval '1 second'", "creation timestamp mutation"),
            ("created_by = gen_random_uuid()", "creator mutation"),
        ] {
            expect_sqlstate(sqlx::query(&format!("UPDATE application_releases SET {change} WHERE id = $1"))
                .bind(a).execute(&database.pool).await, "23514", label)?;
        }
        expect_sqlstate(sqlx::query("UPDATE application_releases SET application_id = $2 WHERE id = $1")
            .bind(a).bind(h.other_app).execute(&database.pool).await, "23514", "application identity mutation")?;
        let after: String = sqlx::query_scalar("SELECT to_jsonb(row)::text FROM application_releases row WHERE id = $1")
            .bind(a).fetch_one(&database.pool).await?;
        ensure!(before == after, "a rejected mutation changed the stored release");
        expect_sqlstate(sqlx::query("UPDATE application_releases SET validation_status = 'PASSED' WHERE id = $1")
            .bind(a).execute(&database.pool).await, "23514", "PASSED without checked_at")?;
        expect_sqlstate(sqlx::query("UPDATE application_releases SET checked_at = now() WHERE id = $1")
            .bind(a).execute(&database.pool).await, "23514", "DRAFT with checked_at")?;
        sqlx::query(
            "UPDATE application_releases SET validation_status = 'PASSED', checked_at = now(), \
             validation_result = '{\"checks\":[]}'::jsonb WHERE id = $1",
        ).bind(a).execute(&database.pool).await?;
        sqlx::query(
            "INSERT INTO application_deployments (application_id, environment, active_release_id) VALUES ($1, $2, $3)",
        ).bind(h.main_app).bind(ENVIRONMENT).bind(a).execute(&database.pool).await?;
        let (status, _) = call(&h.app, Method::POST,
            &format!("/api/applications/{}/releases/{a}/activate", h.other_app), &h.platform, None).await?;
        ensure!(status == StatusCode::NOT_FOUND, "another application's route activated a release");
        Ok(())
    }.await;
    database.finish(result).await
}
