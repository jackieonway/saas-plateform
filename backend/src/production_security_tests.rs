//! Phase 12 production initialization and authenticated introspection gates.
//! Each ignored gate uses its own PostgreSQL database; HTTP gates bind a real
//! loopback TCP listener and preserve normal connection information.
//! TEST_DATABASE_URL comes from the environment or ignored root .env.test.
use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use axum::http::StatusCode;
use axum::Router;
use serde_json::{json, Value};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::config::Config;
use crate::routes::{self, oauth2};
use crate::seed;
use crate::util::{hash, jwt, password};

const ADMIN_NAME: &str = "production-gate-admin";
const ADMIN_PASSWORD: &str = "Synthetic-admin-7D!only-for-tests";
const CLIENT_SECRET: &str = "synthetic-isolated-client-secret-only";

fn database_options() -> Result<PgConnectOptions> {
    let url = match std::env::var("TEST_DATABASE_URL") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env.test");
            let entries = dotenvy::from_path_iter(path).map_err(|_| {
                anyhow!("configure TEST_DATABASE_URL or ignored root backend/.env.test")
            })?;
            let mut configured = None;
            for entry in entries {
                let (key, value) = entry.map_err(|_| anyhow!("invalid test environment file"))?;
                if key == "TEST_DATABASE_URL" && !value.trim().is_empty() {
                    configured = Some(value);
                }
            }
            configured.ok_or_else(|| anyhow!("TEST_DATABASE_URL is missing"))?
        }
    };
    PgConnectOptions::from_str(&url)
        .map_err(|_| anyhow!("invalid test database connection configuration"))
}

fn database_error(stage: &str, error: &sqlx::Error) -> anyhow::Error {
    match error.as_database_error().and_then(|error| error.code()) {
        Some(code) => anyhow!("{stage} failed (SQLSTATE {code})"),
        None => anyhow!("{stage} failed; check isolated database connectivity and privileges"),
    }
}

struct Database {
    pool: PgPool,
    admin: PgPool,
    name: String,
    manifest: std::path::PathBuf,
}

impl Database {
    async fn create() -> Result<Self> {
        let options = database_options()?;
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .map_err(|error| database_error("administration connection", &error))?;
        let name = format!(
            "production_security_test_{}",
            crate::util::uuid7::new_uuid().simple()
        );
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/production-security-tests");
        std::fs::create_dir_all(&directory)?;
        let manifest = directory.join(format!("{name}.json"));
        std::fs::write(
            &manifest,
            serde_json::to_vec(&json!({"databaseName":name,"state":"creating"}))?,
        )?;
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await
            .map_err(|error| database_error("creating isolated test database", &error))?;
        let pool = match PgPoolOptions::new()
            .max_connections(6)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.database(&name))
            .await
        {
            Ok(pool) => pool,
            Err(error) => {
                let removed = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
                    .execute(&admin)
                    .await
                    .is_ok();
                let _ = std::fs::write(
                    &manifest,
                    serde_json::to_vec(&json!({
                        "databaseName":name,"state":"connection_failed","databaseRemoved":removed
                    }))?,
                );
                return Err(database_error("isolated test connection", &error));
            }
        };
        let database = Self {
            pool,
            admin,
            name,
            manifest,
        };
        let running_manifest = std::fs::write(
            &database.manifest,
            serde_json::to_vec(&json!({
                "databaseName":database.name,"state":"running"
            }))?,
        );
        if running_manifest.is_err() {
            let _ = database
                .finish(Err(anyhow!(
                    "cannot record isolated test database ownership"
                )))
                .await;
            return Err(anyhow!("cannot record isolated test database ownership"));
        }
        if sqlx::migrate!("./migrations")
            .run(&database.pool)
            .await
            .is_err()
        {
            let _ = database
                .finish(Err(anyhow!("isolated test migrations failed")))
                .await;
            return Err(anyhow!("isolated test migrations failed"));
        }
        Ok(database)
    }

    async fn finish(self, assertions: Result<()>) -> Result<()> {
        let initially_closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let cleanup = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await
            .map_err(|error| database_error("removing isolated test database", &error));
        let finally_closed = initially_closed
            || tokio::time::timeout(Duration::from_secs(5), self.pool.close())
                .await
                .is_ok();
        let _ = tokio::time::timeout(Duration::from_secs(5), self.admin.close()).await;
        let manifest_result = std::fs::write(
            &self.manifest,
            serde_json::to_vec(&json!({
                "databaseName":self.name,"state":"finished","assertionsPassed":assertions.is_ok(),
                "databaseRemoved":cleanup.is_ok(),"poolClosed":finally_closed
            }))?,
        );
        eprintln!(
            "production security cleanup assertions_ok={} database_removed={} pool_closed={}",
            assertions.is_ok(),
            cleanup.is_ok(),
            finally_closed
        );
        assertions?;
        cleanup?;
        manifest_result?;
        ensure!(
            finally_closed,
            "isolated test pool did not close after database removal"
        );
        Ok(())
    }
}

fn production_config() -> Result<Config> {
    production_config_with(ADMIN_NAME, ADMIN_PASSWORD)
}

fn production_config_with(admin_name: &str, admin_password: &str) -> Result<Config> {
    let values = HashMap::from([
        ("APPLICATION_ENVIRONMENT".into(), "production".into()),
        (
            "DATABASE_URL".into(),
            "postgres://unused/production-test".into(),
        ),
        (
            "JWT_SECRET".into(),
            "Production-tests-strong-JWT-47383fcd98404e4bad316ecd89075279".into(),
        ),
        (
            "METRICS_TOKEN".into(),
            "Production-tests-metrics-92043d437cf046869b91df3d6f9773d1".into(),
        ),
        (
            "PUBLIC_APP_URL".into(),
            "https://production.example.test".into(),
        ),
        (
            "MICRO_GATEWAY_ORIGIN".into(),
            "https://production.example.test".into(),
        ),
        (
            "MICRO_ALLOWED_ORIGINS".into(),
            "https://production.example.test".into(),
        ),
        (
            "CORS_ALLOWED_ORIGINS".into(),
            "https://production.example.test".into(),
        ),
        ("BOOTSTRAP_ADMIN_USERNAME".into(), admin_name.into()),
        ("BOOTSTRAP_ADMIN_PASSWORD".into(), admin_password.into()),
    ]);
    Config::from_values(&values)
}

/// Hash row contents instead of displaying secrets when a non-mutation check fails.
async fn snapshot(pool: &PgPool) -> Result<BTreeMap<String, String>> {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT tablename FROM pg_tables WHERE schemaname='public' ORDER BY tablename",
    )
    .fetch_all(pool)
    .await?;
    let mut result = BTreeMap::new();
    for table in tables {
        let escaped = table.replace('"', "\"\"");
        let text: String = sqlx::query_scalar(&format!(
            "SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text), '[]'::jsonb)::text FROM \"{escaped}\" t"
        )).fetch_one(pool).await?;
        result.insert(table, hash::sha256_hex(&text));
    }
    Ok(result)
}

struct HttpServer {
    url: String,
    client: reqwest::Client,
    stop: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl HttpServer {
    async fn start(pool: &PgPool, config: Config) -> Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let state = crate::sso_test_fixture::state(pool.clone(), Arc::new(config));
        let router = Router::new()
            .nest("/oauth", oauth2::oauth_router())
            .merge(routes::api_router())
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::normalize_client_ip,
            ))
            .with_state(state);
        let (stop, stopped) = oneshot::channel();
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
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self {
            url,
            client,
            stop: Some(stop),
            task,
        })
    }

    async fn introspect(
        &self,
        client_id: &str,
        secret: &str,
        token: &str,
        forged_ip: Option<&str>,
    ) -> Result<(StatusCode, Value, Option<u64>)> {
        // Synthetic fixture strings are unreserved characters, so this deliberately
        // uses the same standard form body sent by an external business backend.
        let mut request = self
            .client
            .post(format!("{}/oauth/introspect", self.url))
            .basic_auth(client_id, Some(secret))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(format!("token={token}"));
        if let Some(ip) = forged_ip {
            request = request
                .header("x-forwarded-for", ip)
                .header("x-real-ip", ip)
                .header("forwarded", format!("for={ip}"));
        }
        let response = request.send().await?;
        let status = response.status();
        let retry = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok());
        let body = serde_json::from_slice(&response.bytes().await?)?;
        Ok((status, body, retry))
    }

    async fn api(
        &self,
        method: reqwest::Method,
        path: &str,
        bearer: &str,
    ) -> Result<(StatusCode, Value)> {
        let response = self
            .client
            .request(method, format!("{}{path}", self.url))
            .bearer_auth(bearer)
            .send()
            .await?;
        let status = response.status();
        let body = serde_json::from_slice(&response.bytes().await?)?;
        Ok((status, body))
    }

    async fn api_json(
        &self,
        method: reqwest::Method,
        path: &str,
        bearer: &str,
        body: Value,
    ) -> Result<(StatusCode, Value)> {
        let response = self
            .client
            .request(method, format!("{}{path}", self.url))
            .bearer_auth(bearer)
            .header("content-type", "application/json")
            .body(serde_json::to_vec(&body)?)
            .send()
            .await?;
        let status = response.status();
        let body = serde_json::from_slice(&response.bytes().await?)?;
        Ok((status, body))
    }

    async fn public_introspect(&self, client_id: &str) -> Result<(StatusCode, Value, Option<u64>)> {
        let response = self
            .client
            .post(format!("{}/oauth/introspect", self.url))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(format!("client_id={client_id}&token=missing-token"))
            .send()
            .await?;
        let status = response.status();
        let retry = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok());
        let body = serde_json::from_slice(&response.bytes().await?)?;
        Ok((status, body, retry))
    }

    async fn finish(mut self) -> Result<()> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        match tokio::time::timeout(Duration::from_secs(5), &mut self.task).await {
            Ok(result) => {
                result??;
                Ok(())
            }
            Err(_) => {
                self.task.abort();
                let _ = (&mut self.task).await;
                Err(anyhow!(
                    "production HTTP gate server did not stop gracefully"
                ))
            }
        }
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn fixture_client(pool: &PgPool, client_id: &str) -> Result<()> {
    sqlx::query("INSERT INTO oauth_clients(client_id,client_secret_hash,client_name,client_type,subject_mode,allowed_member_types,grant_types,scopes,access_token_validity,id) VALUES($1,$2,$1,'CONFIDENTIAL','MACHINE',ARRAY[]::text[],'client_credentials','profile',300,$3)")
        .bind(client_id).bind(hash::sha256_hex(CLIENT_SECRET)).bind(crate::util::uuid7::new_uuid()).execute(pool).await?;
    Ok(())
}

async fn fixture_token(pool: &PgPool, client_id: &str, token: &str) -> Result<Uuid> {
    Ok(sqlx::query_scalar("INSERT INTO oauth_tokens(client_id,token_hash,token_type,token_use,scope,expires_at,id,client_policy_version,application_id,application_policy_version) \
        SELECT $1,$2,'access','oauth','profile',now()+interval '5 minutes',$3,c.access_policy_version,c.application_id,a.access_policy_version \
        FROM oauth_clients c LEFT JOIN applications a ON a.id=c.application_id WHERE c.client_id=$1 RETURNING id")
        .bind(client_id).bind(hash::sha256_hex(token)).bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?)
}

async fn fixture_session(
    pool: &PgPool,
    config: &Config,
    user: Uuid,
    tenant: Uuid,
    org: Uuid,
) -> Result<String> {
    let version: i64 = sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
        .bind(user)
        .fetch_one(pool)
        .await?;
    let now = chrono::Utc::now().timestamp();
    let sid = crate::sso_test_fixture::session(pool, user, tenant, org).await?;
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
        &config.jwt_secret,
    )
}

async fn fixture_actor(
    pool: &PgPool,
    tenant: Uuid,
    org: Uuid,
    name: &str,
    application: Uuid,
    menu: Uuid,
    grant: bool,
) -> Result<Uuid> {
    let user: Uuid = sqlx::query_scalar("INSERT INTO users(username,password_hash,is_super_admin,id) VALUES($1,'synthetic-unusable-password',false,$2) RETURNING id")
        .bind(name).bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?;
    sqlx::query("INSERT INTO memberships(user_id,organization_id,id) VALUES($1,$2,$3)")
        .bind(user)
        .bind(org)
        .bind(crate::util::uuid7::new_uuid())
        .execute(pool)
        .await?;
    if grant {
        let role: Uuid = sqlx::query_scalar(
            "INSERT INTO roles(tenant_id,organization_id,name,id) VALUES($1,$2,$3,$4) RETURNING id",
        )
        .bind(tenant)
        .bind(org)
        .bind(name)
        .bind(crate::util::uuid7::new_uuid())
        .fetch_one(pool)
        .await?;
        sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3)")
            .bind(user)
            .bind(role)
            .bind(org)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)")
            .bind(role)
            .bind(menu)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(tenant).bind(application).execute(pool).await?;
        sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(org).bind(application).execute(pool).await?;
        sqlx::query(
            "INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2) ON CONFLICT DO NOTHING",
        )
        .bind(tenant)
        .bind(menu)
        .execute(pool)
        .await?;
        sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(org).bind(menu).execute(pool).await?;
    }
    Ok(user)
}

fn successful_introspection(
    response: &(StatusCode, Value, Option<u64>),
    active: bool,
) -> Result<()> {
    ensure!(
        response.0 == StatusCode::OK,
        "authenticated introspection unexpectedly rejected"
    );
    ensure!(
        response.1.get("active") == Some(&Value::Bool(active)),
        "unexpected token activity"
    );
    ensure!(
        response.2.is_none(),
        "successful response advertised a rate limit retry"
    );
    Ok(())
}

fn rate_limited(response: &(StatusCode, Value, Option<u64>)) -> Result<()> {
    ensure!(
        response.0 == StatusCode::TOO_MANY_REQUESTS,
        "quota boundary was not enforced"
    );
    ensure!(
        response
            .2
            .is_some_and(|seconds| (1..=60).contains(&seconds)),
        "429 must include bounded Retry-After"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL CREATE DATABASE privileges"]
async fn production_bootstrap_is_minimal_explicit_and_idempotent() -> Result<()> {
    let database = Database::create().await?;
    let result = async {
        let config = production_config()?;
        let mut missing = config.clone();
        missing.bootstrap_admin_username = None;
        missing.bootstrap_admin_password = None;
        let empty = snapshot(&database.pool).await?;
        ensure!(seed::initialize(&database.pool,&missing).await.is_err(),"empty production database accepted missing administrator credentials");
        ensure!(empty==snapshot(&database.pool).await?,"failed administrator bootstrap changed migration records");
        seed::initialize(&database.pool,&config).await?;
        let users: Vec<(String,String,bool,String)> = sqlx::query_as(
            "SELECT username,password_hash,is_super_admin,status FROM users ORDER BY username",
        ).fetch_all(&database.pool).await?;
        ensure!(users.len()==1,"fresh production bootstrap must create exactly one administrator");
        ensure!(users[0].0==ADMIN_NAME && users[0].2 && users[0].3=="ENABLED","explicit initial administrator missing");
        ensure!(password::verify_password(&users[0].1,ADMIN_PASSWORD),"administrator password differs from explicit configuration");
        let context: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM tenants WHERE is_platform=true),(SELECT count(*) FROM organizations),(SELECT count(*) FROM memberships m JOIN users u ON u.id=m.user_id JOIN organizations o ON o.id=m.organization_id JOIN tenants t ON t.id=o.tenant_id WHERE u.username=$1 AND t.is_platform=true AND o.parent_id IS NULL)")
            .bind(ADMIN_NAME).fetch_one(&database.pool).await?;
        ensure!(context==(1,1,1),"production bootstrap must create one platform root and direct administrator membership");
        let enabled_examples: i64 = sqlx::query_scalar("SELECT count(*) FROM applications WHERE status='ENABLED' AND code IN ('athena','vue-workshop','legacy-jquery','report-center')")
            .fetch_one(&database.pool).await?;
        ensure!(enabled_examples==0,"fresh production enabled demonstration applications");
        let enabled_clients: i64 = sqlx::query_scalar("SELECT count(*) FROM oauth_clients WHERE status='ENABLED'")
            .fetch_one(&database.pool).await?;
        ensure!(enabled_clients==0,"fresh production enabled a demonstration OAuth client");
        let registered_rps: i64 = sqlx::query_scalar("SELECT count(*) FROM oidc_clients")
            .fetch_one(&database.pool).await?;
        ensure!(registered_rps == 0,"fresh bootstrap must leave SSO RP registration to explicit provisioning");
        let system_menus: i64 = sqlx::query_scalar("SELECT count(DISTINCT m.permission) FROM menus m JOIN applications a ON a.id=m.application_id WHERE a.code='system' AND m.permission IN ('system:tenant','system:organization','system:user','system:role','system:application','system:menu','system:oauth','system:audit','system:monitor')")
            .fetch_one(&database.pool).await?;
        ensure!(system_menus==9,"production bootstrap lacks required platform management menus");
        let directories: i64 = sqlx::query_scalar("SELECT count(*) FROM menus m JOIN applications a ON a.id=m.application_id WHERE a.code='system' AND m.type='DIR' AND m.parent_id IS NULL AND m.title IN ('组织机构','用户与角色','平台配置','开放平台')")
            .fetch_one(&database.pool).await?;
        ensure!(directories == 4, "fresh bootstrap lacks the original collapsible directories");
        let parents: Vec<(String,String)> = sqlx::query_as("SELECT m.path,p.title FROM menus m JOIN menus p ON p.id=m.parent_id WHERE m.path IN ('/system/customers','/system/tenants','/system/users','/system/menus','/system/oauth/clients') ORDER BY m.path")
            .fetch_all(&database.pool).await?;
        ensure!(parents == vec![
            ("/system/customers".into(), "用户与角色".into()),
            ("/system/menus".into(), "平台配置".into()),
            ("/system/oauth/clients".into(), "开放平台".into()),
            ("/system/tenants".into(), "组织机构".into()),
            ("/system/users".into(), "用户与角色".into()),
        ], "fresh system menus lost their original directory parents");
        seed::check_production_credentials(&database.pool).await?;
        let before = snapshot(&database.pool).await?;
        seed::initialize(&database.pool,&config).await?;
        ensure!(before==snapshot(&database.pool).await?,"repeated production initialization changed persisted rows");
        seed::initialize(&database.pool,&missing).await?;
        ensure!(before==snapshot(&database.pool).await?,"occupied production database required bootstrap credentials or changed persisted rows");
        let replaced = production_config_with("never-create-a-second-admin","Synthetic-unused-bootstrap-password-92!")?;
        seed::initialize(&database.pool,&replaced).await?;
        ensure!(before==snapshot(&database.pool).await?,"changed bootstrap configuration reset an existing account or created a second administrator");
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL CREATE DATABASE privileges"]
async fn existing_demonstration_database_is_refused_without_rewriting_accounts_or_grants(
) -> Result<()> {
    let database = Database::create().await?;
    let result = async {
        seed::run(&database.pool).await?;
        let before = snapshot(&database.pool).await?;
        // Exercise the startup initialization path, rather than only the read-only
        // check, so the fresh-database policy cannot silently alter an occupied DB.
        let startup = async {
            seed::initialize(&database.pool, &production_config()?).await?;
            seed::check_production_credentials(&database.pool).await
        }.await;
        ensure!(startup.is_err(), "production startup accepted enabled demonstration credentials");
        ensure!(before == snapshot(&database.pool).await?, "production startup changed existing accounts, clients, or authorization grants before refusing them");
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL CREATE DATABASE privileges"]
async fn production_stock_user_credentials_are_rejected_without_mutation() -> Result<()> {
    let database = Database::create().await?;
    let result = async {
        seed::initialize(&database.pool, &production_config()?).await?;
        let mut disabled_users = Vec::new();
        for (username, plain) in [
            ("admin", "admin123"),
            ("zhangsan", "Zs@12345"),
            ("lisi", "Ls@12345"),
            ("wangwu", "Ww@12345"),
            ("tianqi", "Tq@12345"),
            ("chenjiu", "Cj@12345"),
            ("renamed-installed-administrator", "admin123"),
        ] {
            let id: Uuid = sqlx::query_scalar(
                "INSERT INTO users(username,password_hash,id) VALUES($1,$2,$3) RETURNING id",
            )
            .bind(username)
            .bind(password::hash_password(plain)?)
            .bind(crate::util::uuid7::new_uuid())
            .fetch_one(&database.pool)
            .await?;
            let before = snapshot(&database.pool).await?;
            ensure!(
                seed::check_production_credentials(&database.pool)
                    .await
                    .is_err(),
                "enabled known demonstration password was accepted"
            );
            ensure!(
                before == snapshot(&database.pool).await?,
                "stock-credential startup refusal mutated persisted rows"
            );
            sqlx::query("UPDATE users SET status='DISABLED' WHERE id=$1")
                .bind(id)
                .execute(&database.pool)
                .await?;
            seed::check_production_credentials(&database.pool).await?;
            disabled_users.push(id);
        }
        for id in disabled_users {
            sqlx::query("UPDATE users SET status='ENABLED',password_hash=$2 WHERE id=$1")
                .bind(id)
                .bind(password::hash_password(
                    "Synthetic-rotated-production-credential-8!",
                )?)
                .execute(&database.pool)
                .await?;
        }
        seed::check_production_credentials(&database.pool).await?;
        Ok(())
    }
    .await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL CREATE DATABASE privileges"]
async fn production_stock_client_secrets_are_rejected_without_mutation() -> Result<()> {
    let database = Database::create().await?;
    let result = async {
        seed::initialize(&database.pool,&production_config()?).await?;
        // The user-password audit has its own gate; this fixture isolates client
        // secret hashes without repeating an unrelated Argon audit for every case.
        sqlx::query("UPDATE users SET status='DISABLED' WHERE username=$1")
            .bind(ADMIN_NAME).execute(&database.pool).await?;
        for (client_id,plain) in [("platform-web","platform-web-secret"),("api-service","api-service-secret"),("resource-portal","resource-portal-secret"),("micro-vue-shell","micro-vue-shell-secret"),("micro-jquery-shell","micro-jquery-shell-secret"),("micro-report-shell","micro-report-shell-secret"),("renamed-installed-micro-client","micro-vue-shell-secret")] {
            sqlx::query("INSERT INTO oauth_clients(client_id,client_secret_hash,client_name,client_type,id) VALUES($1,$2,$1,'CONFIDENTIAL',$3) ON CONFLICT(client_id) DO UPDATE SET client_secret_hash=EXCLUDED.client_secret_hash,status='ENABLED'")
                .bind(client_id).bind(hash::sha256_hex(plain)).bind(crate::util::uuid7::new_uuid()).execute(&database.pool).await?;
            let before = snapshot(&database.pool).await?;
            ensure!(seed::check_production_credentials(&database.pool).await.is_err(),"enabled known demonstration client secret was accepted");
            ensure!(before==snapshot(&database.pool).await?,"stock-client startup refusal mutated persisted rows");
            sqlx::query("UPDATE oauth_clients SET status='DISABLED' WHERE client_id=$1")
                .bind(client_id).execute(&database.pool).await?;
            seed::check_production_credentials(&database.pool).await?;
            sqlx::query("UPDATE oauth_clients SET status='ENABLED',client_secret_hash=$2 WHERE client_id=$1")
                .bind(client_id).bind(hash::sha256_hex(CLIENT_SECRET)).execute(&database.pool).await?;
            seed::check_production_credentials(&database.pool).await?;
        }
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL and actual loopback HTTP"]
async fn authenticated_introspection_quotas_isolate_clients_and_failed_authentication() -> Result<()>
{
    let database = Database::create().await?;
    let result = async {
        let mut config = production_config()?;
        config.oauth_introspection_client_requests_per_minute = 2;
        config.oauth_auth_failure_requests_per_minute = 3;
        fixture_client(&database.pool, "quota-a").await?;
        fixture_client(&database.pool, "quota-b").await?;
        fixture_client(&database.pool, "quota-public").await?;
        sqlx::query("UPDATE oauth_clients SET subject_mode='USER',allowed_member_types=ARRAY['EMPLOYEE']::text[],grant_types='authorization_code',client_type='PUBLIC',client_secret_hash=NULL WHERE client_id='quota-public'")
            .execute(&database.pool).await?;
        let server = HttpServer::start(&database.pool, config).await?;
        let assertions = async {
            for index in 0..3 {
                let forged = format!("198.51.100.{}", index + 10);
                let response = server
                    .introspect("quota-a", "incorrect", "missing-token", Some(&forged))
                    .await?;
                ensure!(
                    response.0 == StatusCode::UNAUTHORIZED,
                    "failed authentication boundary changed"
                );
                ensure!(
                    response.1.get("error") == Some(&Value::String("invalid_client".into())),
                    "bad client credentials must produce invalid_client"
                );
            }
            rate_limited(
                &server
                    .introspect(
                        "quota-a",
                        "incorrect",
                        "missing-token",
                        Some("203.0.113.91"),
                    )
                    .await?,
            )?;
            // Real direct peer remains the same despite changing every forwarded header.
            rate_limited(
                &server
                    .introspect(
                        "unknown-client",
                        "incorrect",
                        "missing-token",
                        Some("203.0.113.92"),
                    )
                    .await?,
            )?;
            rate_limited(&server.public_introspect("quota-public").await?)?;
            successful_introspection(
                &server
                    .introspect("quota-a", CLIENT_SECRET, "missing-token", None)
                    .await?,
                false,
            )?;
            successful_introspection(
                &server
                    .introspect("quota-a", CLIENT_SECRET, "missing-token", None)
                    .await?,
                false,
            )?;
            rate_limited(
                &server
                    .introspect(
                        "quota-a",
                        CLIENT_SECRET,
                        "missing-token",
                        Some("203.0.113.93"),
                    )
                    .await?,
            )?;
            successful_introspection(
                &server
                    .introspect("quota-b", CLIENT_SECRET, "missing-token", None)
                    .await?,
                false,
            )?;
            successful_introspection(
                &server
                    .introspect("quota-b", CLIENT_SECRET, "missing-token", None)
                    .await?,
                false,
            )?;
            rate_limited(
                &server
                    .introspect("quota-b", CLIENT_SECRET, "missing-token", None)
                    .await?,
            )?;
            // Successful authentication does not erase previous failure protection.
            rate_limited(
                &server
                    .introspect(
                        "quota-a",
                        "incorrect",
                        "missing-token",
                        Some("203.0.113.94"),
                    )
                    .await?,
            )?;
            Ok(())
        }
        .await;
        let stopped = server.finish().await;
        assertions?;
        stopped?;
        Ok(())
    }
    .await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL and actual loopback HTTP"]
async fn default_introspection_capacity_preserves_immediate_revocation_and_client_disable(
) -> Result<()> {
    let database = Database::create().await?;
    let result = async {
        let config = production_config()?;
        ensure!(
            config.oauth_introspection_client_requests_per_minute == 600,
            "production introspection default differs from reviewed capacity"
        );
        fixture_client(&database.pool, "capacity-a").await?;
        fixture_client(&database.pool, "capacity-b").await?;
        let token_id =
            fixture_token(&database.pool, "capacity-a", "synthetic-capacity-token").await?;
        let server = HttpServer::start(&database.pool, config).await?;
        let assertions = async {
            for _ in 0..31 {
                successful_introspection(
                    &server
                        .introspect(
                            "capacity-a",
                            CLIENT_SECRET,
                            "synthetic-capacity-token",
                            None,
                        )
                        .await?,
                    true,
                )?;
            }
            successful_introspection(
                &server
                    .introspect(
                        "capacity-b",
                        CLIENT_SECRET,
                        "synthetic-capacity-token",
                        None,
                    )
                    .await?,
                false,
            )?;
            sqlx::query("UPDATE oauth_tokens SET revoked=true WHERE id=$1")
                .bind(token_id)
                .execute(&database.pool)
                .await?;
            successful_introspection(
                &server
                    .introspect(
                        "capacity-a",
                        CLIENT_SECRET,
                        "synthetic-capacity-token",
                        None,
                    )
                    .await?,
                false,
            )?;
            sqlx::query("UPDATE oauth_clients SET status='DISABLED' WHERE client_id='capacity-a'")
                .execute(&database.pool)
                .await?;
            let response = server
                .introspect(
                    "capacity-a",
                    CLIENT_SECRET,
                    "synthetic-capacity-token",
                    None,
                )
                .await?;
            ensure!(
                response.0 == StatusCode::UNAUTHORIZED,
                "disabled authenticated client was accepted on its next request"
            );
            successful_introspection(
                &server
                    .introspect("capacity-b", CLIENT_SECRET, "missing-token", None)
                    .await?,
                false,
            )?;
            Ok(())
        }
        .await;
        let stopped = server.finish().await;
        assertions?;
        stopped?;
        Ok(())
    }
    .await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL and actual loopback HTTP"]
async fn global_oauth_administration_requires_platform_context_and_operation_permission(
) -> Result<()> {
    let database = Database::create().await?;
    let result = async {
        let config = production_config()?;
        let application: Uuid = sqlx::query_scalar("INSERT INTO applications(code,name,base_path,id) VALUES('production-oauth-admin-gate','OAuth security gate','/gate',$1) RETURNING id")
            .bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
        let menu: Uuid = sqlx::query_scalar("INSERT INTO menus(application_id,type,title,path,permission,id) VALUES($1,'MENU','OAuth clients','/system/oauth/clients','system:oauth',$2) RETURNING id")
            .bind(application).bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
        let tokens_menu: Uuid = sqlx::query_scalar("INSERT INTO menus(application_id,type,title,path,permission,id) VALUES($1,'MENU','OAuth tokens','/system/oauth/tokens','system:oauth',$2) RETURNING id")
            .bind(application).bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
        let platform: Uuid = sqlx::query_scalar("INSERT INTO tenants(code,name,is_platform,id) VALUES('oauth-admin-platform','Platform',true,$1) RETURNING id")
            .bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
        let ordinary: Uuid = sqlx::query_scalar("INSERT INTO tenants(code,name,is_platform,id) VALUES('oauth-admin-ordinary','Ordinary',false,$1) RETURNING id")
            .bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
        let mut organizations = Vec::new();
        for tenant in [platform, ordinary] {
            let org: Uuid = sqlx::query_scalar("INSERT INTO organizations(tenant_id,name,id) VALUES($1,'Root',$2) RETURNING id")
                .bind(tenant).bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
            organizations.push(org);
        }
        let platform_user = fixture_actor(&database.pool,platform,organizations[0],"oauth-platform-operator",application,menu,true).await?;
        let ordinary_user = fixture_actor(&database.pool,ordinary,organizations[1],"oauth-ordinary-operator",application,menu,true).await?;
        let unprivileged = fixture_actor(&database.pool,platform,organizations[0],"oauth-platform-without-permission",application,menu,false).await?;
        for (user, tenant, org) in [(platform_user, platform, organizations[0]), (ordinary_user, ordinary, organizations[1])] {
            sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
                .bind(tenant).bind(tokens_menu).execute(&database.pool).await?;
            sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
                .bind(org).bind(tokens_menu).execute(&database.pool).await?;
            sqlx::query("INSERT INTO role_menus(role_id,menu_id) SELECT role_id,$2 FROM user_roles WHERE user_id=$1 ON CONFLICT DO NOTHING")
                .bind(user).bind(tokens_menu).execute(&database.pool).await?;
        }
        let platform_token = fixture_session(&database.pool,&config,platform_user,platform,organizations[0]).await?;
        let ordinary_token = fixture_session(&database.pool,&config,ordinary_user,ordinary,organizations[1]).await?;
        let unprivileged_token = fixture_session(&database.pool,&config,unprivileged,platform,organizations[0]).await?;
        let state = crate::sso_test_fixture::state(database.pool.clone(),Arc::new(config.clone()));
        ensure!(state.permissions_of(ordinary_user,organizations[1]).await?.contains("system:oauth"),"ordinary actor fixture lacks the explicit OAuth operation grant being tested");
        ensure!(state.permissions_of(platform_user,organizations[0]).await?.contains("system:oauth"),"platform actor fixture lacks an explicit OAuth operation grant");
        ensure!(!state.permissions_of(unprivileged,organizations[0]).await?.contains("system:oauth"),"unprivileged platform actor inherited unexpected OAuth operation permission");
        fixture_client(&database.pool,"oauth-admin-target").await?;
        let client: Uuid = sqlx::query_scalar("SELECT id FROM oauth_clients WHERE client_id='oauth-admin-target'")
            .fetch_one(&database.pool).await?;
        let expired = fixture_token(&database.pool,"oauth-admin-target","synthetic-expired-admin-token").await?;
        sqlx::query("UPDATE oauth_tokens SET expires_at=now()-interval '14 days' WHERE id=$1")
            .bind(expired).execute(&database.pool).await?;
        let regenerate = format!("/api/oauth/clients/{client}/regenerate-secret");
        let server = HttpServer::start(&database.pool,config).await?;
        let assertions = async {
            let before = snapshot(&database.pool).await?;
            for token in [&ordinary_token,&unprivileged_token] {
                for (method,path) in [(reqwest::Method::GET,"/api/oauth/clients"),(reqwest::Method::GET,"/api/oauth/tokens"),
                    (reqwest::Method::POST,"/api/oauth/tokens/purge"),(reqwest::Method::POST,regenerate.as_str())] {
                    let response = server.api(method,path,token).await?;
                    ensure!(response.0==StatusCode::FORBIDDEN,"global OAuth operation accepted a user without both platform context and operation permission");
                    ensure!(before==snapshot(&database.pool).await?,"rejected global OAuth operation mutated persisted rows");
                }
            }
            let clients = server.api(reqwest::Method::GET,"/api/oauth/clients",&platform_token).await?;
            ensure!(clients.0==StatusCode::OK && clients.1.is_array(),"authorized non-super platform user cannot list OAuth clients");
            ensure!(clients.1.as_array().is_some_and(|rows|rows.iter().any(|row|row.get("id")==Some(&Value::String(client.to_string())))),"platform global OAuth list did not expose its actual target fixture");
            ensure!(clients.1.as_array().is_some_and(|rows|rows.iter().all(|row|row.get("clientSecret").is_none() && row.get("clientSecretHash").is_none())),"global client list disclosed a client credential");
            let tokens = server.api(reqwest::Method::GET,"/api/oauth/tokens",&platform_token).await?;
            ensure!(tokens.0==StatusCode::OK && tokens.1.is_array(),"authorized platform user cannot list OAuth tokens");
            let purge = server.api(reqwest::Method::POST,"/api/oauth/tokens/purge",&platform_token).await?;
            ensure!(purge.0==StatusCode::OK && purge.1.get("tokensDeleted").and_then(Value::as_u64)==Some(1),"authorized explicit purge did not remove the expired fixture token");
            let remains: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM oauth_tokens WHERE id=$1)")
                .bind(expired).fetch_one(&database.pool).await?;
            ensure!(!remains,"authorized purge reported deletion while retaining target token");
            let rotated = server.api(reqwest::Method::POST,&regenerate,&platform_token).await?;
            ensure!(rotated.0==StatusCode::OK,"authorized platform secret rotation failed");
            let secret = rotated.1.get("clientSecret").and_then(Value::as_str)
                .ok_or_else(||anyhow!("authorized rotation omitted its one-time credential"))?;
            let persisted: String = sqlx::query_scalar("SELECT client_secret_hash FROM oauth_clients WHERE id=$1")
                .bind(client).fetch_one(&database.pool).await?;
            ensure!(secret != CLIENT_SECRET && persisted==hash::sha256_hex(secret),"authorized secret rotation failed to persist the new credential hash");
            Ok(())
        }.await;
        let stopped = server.finish().await;
        assertions?;
        stopped?;
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL and actual loopback HTTP"]
async fn production_password_writes_reject_unsafe_values_without_mutation_and_revoke_sessions_on_success(
) -> Result<()> {
    const INITIAL: &str = "Synthetic-personal-user-start-73!";
    const RESET: &str = "Synthetic-personal-user-reset-94!";
    const CHANGED: &str = "Synthetic-personal-user-changed-26!";
    let database = Database::create().await?;
    let result = async {
        let config = production_config()?;
        seed::initialize(&database.pool,&config).await?;
        let (administrator,tenant,organization): (Uuid,Uuid,Uuid) = sqlx::query_as("SELECT u.id,o.tenant_id,o.id FROM users u JOIN memberships m ON m.user_id=u.id JOIN organizations o ON o.id=m.organization_id WHERE u.username=$1")
            .bind(ADMIN_NAME).fetch_one(&database.pool).await?;
        let administrator_token = fixture_session(&database.pool,&config,administrator,tenant,organization).await?;
        let server = HttpServer::start(&database.pool,config.clone()).await?;
        let assertions = async {
            for (suffix,weak) in [("known","admin123"),("short","sixsix")] {
                let before = snapshot(&database.pool).await?;
                let response = server.api_json(reqwest::Method::POST,"/api/users",&administrator_token,
                    json!({"username":format!("production-rejected-password-{suffix}"),"password":weak})).await?;
                ensure!(response.0==StatusCode::BAD_REQUEST,"production user creation accepted an unsafe password");
                ensure!(before==snapshot(&database.pool).await?,"rejected production user creation changed users, notifications, sessions, or audit rows");
            }
            let created = server.api_json(reqwest::Method::POST,"/api/users",&administrator_token,
                json!({"username":"production-password-writes-user","password":INITIAL})).await?;
            ensure!(created.0==StatusCode::OK,"production user creation rejected a compliant password");
            let user: Uuid = created.1.get("id").and_then(Value::as_str)
                .and_then(|value|Uuid::parse_str(value).ok())
                .ok_or_else(||anyhow!("created production user has no valid identifier"))?;
            let (initial_hash,initial_version): (String,i64) = sqlx::query_as("SELECT password_hash,session_version FROM users WHERE id=$1")
                .bind(user).fetch_one(&database.pool).await?;
            ensure!(password::verify_password(&initial_hash,INITIAL),"production user creation persisted an incorrect password hash");
            // Platform-created users are global accounts until an administrator
            // assigns membership; this explicit fixture enables actual sessions.
            sqlx::query("INSERT INTO memberships(user_id,organization_id,id) VALUES($1,$2,$3)")
                .bind(user).bind(organization).bind(crate::util::uuid7::new_uuid()).execute(&database.pool).await?;
            let user_token = fixture_session(&database.pool,&config,user,tenant,organization).await?;
            let second_token = fixture_session(&database.pool,&config,user,tenant,organization).await?;
            for token in [&user_token,&second_token] {
                ensure!(server.api(reqwest::Method::GET,"/api/auth/profile",token).await?.0==StatusCode::OK,"password-write fixture session is not active");
            }
            let reset_path = format!("/api/users/{user}/reset-password");
            for weak in ["admin123","sixsix"] {
                let before = snapshot(&database.pool).await?;
                let rejected_reset = server.api_json(reqwest::Method::POST,&reset_path,&administrator_token,
                    json!({"newPassword":weak})).await?;
                ensure!(rejected_reset.0==StatusCode::BAD_REQUEST,"production administrator reset accepted an unsafe password");
                ensure!(before==snapshot(&database.pool).await?,"rejected administrator reset changed a password hash, revoked sessions, or created notifications/audit rows");
                let rejected_change = server.api_json(reqwest::Method::PUT,"/api/auth/password",&user_token,
                    json!({"oldPassword":INITIAL,"newPassword":weak})).await?;
                ensure!(rejected_change.0==StatusCode::BAD_REQUEST,"production personal password change accepted an unsafe password");
                ensure!(before==snapshot(&database.pool).await?,"rejected personal password change changed persisted rows");
                for token in [&user_token,&second_token] {
                    ensure!(server.api(reqwest::Method::GET,"/api/auth/profile",token).await?.0==StatusCode::OK,"rejected unsafe password write revoked a valid session");
                }
            }
            let reset = server.api_json(reqwest::Method::POST,&reset_path,&administrator_token,
                json!({"newPassword":RESET})).await?;
            ensure!(reset.0==StatusCode::OK,"production administrator reset rejected a compliant password");
            let (reset_hash,reset_version): (String,i64) = sqlx::query_as("SELECT password_hash,session_version FROM users WHERE id=$1")
                .bind(user).fetch_one(&database.pool).await?;
            ensure!(reset_version==initial_version+1 && reset_hash!=initial_hash && password::verify_password(&reset_hash,RESET),"successful administrator reset failed to persist a new hash and session version");
            let active_after_reset: i64 = sqlx::query_scalar("SELECT count(*) FROM platform_sessions WHERE user_id=$1 AND revoked_at IS NULL")
                .bind(user).fetch_one(&database.pool).await?;
            ensure!(active_after_reset==0,"successful administrator reset left a target device session active");
            for token in [&user_token,&second_token] {
                ensure!(server.api(reqwest::Method::GET,"/api/auth/profile",token).await?.0==StatusCode::UNAUTHORIZED,"administrator reset did not reject the old session on its next request");
            }
            ensure!(server.api(reqwest::Method::GET,"/api/auth/profile",&administrator_token).await?.0==StatusCode::OK,"resetting another user revoked the administrator session");
            let fresh_token = fixture_session(&database.pool,&config,user,tenant,organization).await?;
            let fresh_second = fixture_session(&database.pool,&config,user,tenant,organization).await?;
            let changed = server.api_json(reqwest::Method::PUT,"/api/auth/password",&fresh_token,
                json!({"oldPassword":RESET,"newPassword":CHANGED})).await?;
            ensure!(changed.0==StatusCode::OK,"production personal password change rejected a compliant password");
            let (changed_hash,changed_version): (String,i64) = sqlx::query_as("SELECT password_hash,session_version FROM users WHERE id=$1")
                .bind(user).fetch_one(&database.pool).await?;
            ensure!(changed_version==reset_version+1 && changed_hash!=reset_hash && password::verify_password(&changed_hash,CHANGED),"successful personal password change failed to persist a new hash and session version");
            let active_after_change: i64 = sqlx::query_scalar("SELECT count(*) FROM platform_sessions WHERE user_id=$1 AND revoked_at IS NULL")
                .bind(user).fetch_one(&database.pool).await?;
            ensure!(active_after_change==0,"successful personal password change left a device session active");
            for token in [&fresh_token,&fresh_second] {
                ensure!(server.api(reqwest::Method::GET,"/api/auth/profile",token).await?.0==StatusCode::UNAUTHORIZED,"personal password change did not immediately invalidate its device sessions");
            }
            ensure!(server.api(reqwest::Method::GET,"/api/auth/profile",&administrator_token).await?.0==StatusCode::OK,"another user's personal password change revoked the administrator session");
            Ok(())
        }.await;
        let stopped = server.finish().await;
        assertions?;
        stopped?;
        Ok(())
    }.await;
    database.finish(result).await
}
