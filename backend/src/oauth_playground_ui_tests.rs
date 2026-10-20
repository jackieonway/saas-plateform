//! Built OAuth Vite page -> Rust playground -> real platform -> owned PostgreSQL.
//! This fixture is test-only and requires an explicit disposable TEST_DATABASE_URL.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use axum::Router;
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};
use uuid::Uuid;

use crate::config::Config;
use crate::entity_creation_tests::Database;
use crate::state::AppState;
use crate::util::{hash, jwt, password};

const SECRET: &str = "owned-phase24-platform-session-signing-only";
const CLIENT_SECRET: &str = "owned-phase24-confidential-client-secret";
const PASSWORD: &str = "Owned-Phase24-password-9!";
const USERNAME: &str = "phase24-playground-user";
const APPLICATION: &str = "phase24-playground";

struct Fixture {
    session: String,
    clients: [String; 4],
}

impl Fixture {
    async fn create(pool: &PgPool, callback: &str) -> Result<Self> {
        let tenant: Uuid = sqlx::query_scalar("INSERT INTO tenants(name,code) VALUES('Phase24 owned tenant','phase24-owned') RETURNING id")
            .fetch_one(pool).await?;
        let org: Uuid = sqlx::query_scalar("INSERT INTO organizations(tenant_id,name) VALUES($1,'Phase24 owned organization') RETURNING id")
            .bind(tenant).fetch_one(pool).await?;
        let user: Uuid = sqlx::query_scalar("INSERT INTO users(username,real_name,password_hash) VALUES($1,'Phase24 User',$2) RETURNING id")
            .bind(USERNAME).bind(password::hash_password(PASSWORD)?).fetch_one(pool).await?;
        sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)")
            .bind(user)
            .bind(org)
            .execute(pool)
            .await?;
        let application: Uuid = sqlx::query_scalar("INSERT INTO applications(name,code,base_path,entry_type) VALUES('Phase24 OAuth App',$1,'/micro/phase24','HTML') RETURNING id")
            .bind(APPLICATION).fetch_one(pool).await?;
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
            .bind(tenant)
            .bind(application)
            .execute(pool)
            .await?;
        sqlx::query(
            "INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)",
        )
        .bind(org)
        .bind(application)
        .execute(pool)
        .await?;
        let menu: Uuid = sqlx::query_scalar("INSERT INTO menus(application_id,title,type,path,permission) VALUES($1,'Phase24 resource','MENU','/phase24','phase24:read') RETURNING id")
            .bind(application).fetch_one(pool).await?;
        sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
            .bind(tenant)
            .bind(menu)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
            .bind(org)
            .bind(menu)
            .execute(pool)
            .await?;
        let role: Uuid = sqlx::query_scalar("INSERT INTO roles(tenant_id,organization_id,name,code) VALUES($1,$2,'Phase24 reader','phase24-reader') RETURNING id")
            .bind(tenant).bind(org).fetch_one(pool).await?;
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

        let clients = [
            "phase24-auth",
            "phase24-device",
            "phase24-service",
            "phase24-legacy",
        ]
        .map(str::to_owned);
        let grants = [
            "authorization_code,refresh_token",
            "urn:ietf:params:oauth:grant-type:device_code,refresh_token",
            "client_credentials",
            "password,refresh_token",
        ];
        for (index, client) in clients.iter().enumerate() {
            sqlx::query("INSERT INTO oauth_clients(client_id,client_secret_hash,client_name,client_type,redirect_uris,grant_types,scopes,access_token_validity,refresh_token_validity,application_id) VALUES($1,$2,$3,$4,$5,$6,'profile,email',3600,86400,$7)")
                .bind(client).bind(if index == 1 { None } else { Some(hash::sha256_hex(CLIENT_SECRET)) })
                .bind(format!("Phase24 flow {index}"))
                .bind(if index == 1 { "PUBLIC" } else { "CONFIDENTIAL" })
                .bind(if index == 0 { callback } else { "" })
                .bind(grants[index]).bind(if index == 0 { Some(application) } else { None })
                .execute(pool).await?;
        }
        let sid: Uuid = sqlx::query_scalar("INSERT INTO platform_sessions(user_id,tenant_id,organization_id,expires_at) VALUES($1,$2,$3,now()+interval '1 hour') RETURNING id")
            .bind(user).bind(tenant).bind(org).fetch_one(pool).await?;
        let version = sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
            .bind(user)
            .fetch_one(pool)
            .await?;
        let now = chrono::Utc::now().timestamp();
        let session = jwt::sign(
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
        )?;
        Ok(Self { session, clients })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicit disposable PostgreSQL, built OAuth frontend, and installed test browser"]
async fn real_platform_four_flows_and_callback_browser() -> Result<()> {
    ensure!(
        std::env::var("TEST_DATABASE_URL").is_ok_and(|v| !v.trim().is_empty()),
        "explicit disposable TEST_DATABASE_URL required; no dotenv fallback"
    );
    let database = Database::create().await?;
    let result = run_live(&database.pool).await;
    database.finish(result).await
}

async fn run_live(pool: &PgPool) -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| anyhow!("workspace root missing"))?;
    let dist = root.join("mini-services/oauth-playground/frontend/dist");
    ensure!(
        dist.join("index.html").is_file(),
        "build the independent OAuth Vite frontend before this browser test"
    );
    let platform_listener = TcpListener::bind("127.0.0.1:0").await?;
    let playground_listener = TcpListener::bind("127.0.0.1:0").await?;
    let platform_origin = format!("http://{}", platform_listener.local_addr()?);
    let public_origin = format!("http://{}", playground_listener.local_addr()?);
    let fixture = Fixture::create(pool, &format!("{public_origin}/callback")).await?;
    let platform_config = Config::from_values(&HashMap::from([
        ("APPLICATION_ENVIRONMENT".into(), "development".into()),
        (
            "DATABASE_URL".into(),
            "postgres://unused/owned-phase24-browser".into(),
        ),
        ("JWT_SECRET".into(), SECRET.into()),
        ("PUBLIC_APP_URL".into(), platform_origin.clone()),
    ]))?;
    let platform = crate::routes::api_router()
        .nest("/oauth", crate::routes::oauth2::oauth_router())
        .with_state(AppState::new(pool.clone(), Arc::new(platform_config)));
    let playground_config = oauth_playground::Config::from_values(HashMap::from([
        ("OAUTH_PUBLIC_ORIGIN".into(), public_origin.clone()),
        ("OAUTH_PLATFORM_ORIGIN".into(), platform_origin.clone()),
        ("OAUTH_ALLOW_INSECURE_LOOPBACK".into(), "true".into()),
        ("OAUTH_AUTH_CLIENT_ID".into(), fixture.clients[0].clone()),
        ("OAUTH_AUTH_CLIENT_SECRET".into(), CLIENT_SECRET.into()),
        ("OAUTH_DEVICE_CLIENT_ID".into(), fixture.clients[1].clone()),
        ("OAUTH_CC_CLIENT_ID".into(), fixture.clients[2].clone()),
        ("OAUTH_CC_CLIENT_SECRET".into(), CLIENT_SECRET.into()),
        ("OAUTH_PASSWORD_ENABLED".into(), "true".into()),
        (
            "OAUTH_PASSWORD_CLIENT_ID".into(),
            fixture.clients[3].clone(),
        ),
        ("OAUTH_PASSWORD_CLIENT_SECRET".into(), CLIENT_SECRET.into()),
        ("OAUTH_SCOPE".into(), "profile email".into()),
    ]))
    .map_err(|_| anyhow!("owned playground configuration rejected"))?;
    let playground = oauth_playground::app(playground_config)
        .map_err(|_| anyhow!("owned playground initialization failed"))?
        .route_service("/callback", ServeFile::new(dist.join("callback.html")))
        .fallback_service(ServeDir::new(&dist));
    let (platform_stop, platform_server) = serve(platform_listener, platform);
    let (playground_stop, playground_server) = serve(playground_listener, playground);
    let result = async {
        let product = std::env::var("SAAS_TEST_BROWSER").unwrap_or_else(|_| "chromium".into());
        let mut command = tokio::process::Command::new("node");
        command.current_dir(root).arg("tests/phase24-oauth-browser.mjs").env_clear()
            .env("SAAS_TEST_BROWSER", &product).kill_on_drop(true)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        for name in ["PATH", "Path", "SystemRoot", "SYSTEMROOT", "WINDIR", "COMSPEC", "TEMP", "TMP", "TMPDIR", "HOME", "USERPROFILE", "LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)", "DISPLAY", "XDG_RUNTIME_DIR", "PLAYWRIGHT_BROWSERS_PATH", "PHASE24_SCREENSHOT_PATH"] {
            if let Some(value) = std::env::var_os(name) { command.env(name, value); }
        }
        let mut child = command.spawn().map_err(|_| anyhow!("unable to start owned browser driver"))?;
        let mut input = child.stdin.take().ok_or_else(|| anyhow!("owned browser driver stdin missing"))?;
        input.write_all(&serde_json::to_vec(&json!({"schemaVersion":1,"origin":public_origin,
            "platformOrigin":platform_origin,"session":fixture.session,"username":USERNAME,"password":PASSWORD,
            "application":APPLICATION,"secrets":[CLIENT_SECRET,SECRET]}))?).await?;
        input.shutdown().await?;
        drop(input);
        let output = tokio::time::timeout(Duration::from_secs(180), child.wait_with_output()).await
            .map_err(|_| anyhow!("owned OAuth browser deadline exceeded"))??;
        let combined = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        ensure!(combined.len() <= 1024 * 1024, "owned OAuth browser diagnostics exceeded budget");
        for secret in [SECRET, CLIENT_SECRET, PASSWORD, fixture.session.as_str()] {
            ensure!(!combined.contains(secret), "browser output exposed fixture credential material");
        }
        ensure!(!combined.contains("postgres://") && !combined.contains("postgresql://"), "browser exposed database configuration");
        print!("{combined}");
        ensure!(output.status.success(), "real OAuth browser driver failed");
        let reports = String::from_utf8_lossy(&output.stdout).lines()
            .filter_map(|line| line.strip_prefix("PHASE24_LIVE_RESULT "))
            .map(serde_json::from_str::<Value>).collect::<std::result::Result<Vec<_>, _>>()?;
        ensure!(reports.len() == 1 && reports[0]["schemaVersion"] == 1 && reports[0]["product"] == product
            && reports[0]["failed"] == 0 && reports[0]["skipped"] == 0, "OAuth browser structured result invalid");
        let consent: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='CONSENT_APPROVE'").fetch_one(pool).await?;
        let device: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='DEVICE_APPROVE'").fetch_one(pool).await?;
        let issued: i64 = sqlx::query_scalar("SELECT count(DISTINCT client_id) FROM oauth_tokens WHERE token_use='oauth'").fetch_one(pool).await?;
        let raw_tokens: i64 = sqlx::query_scalar("SELECT count(*) FROM oauth_tokens WHERE jwt IS NOT NULL").fetch_one(pool).await?;
        let auth_revoked: bool = sqlx::query_scalar("SELECT bool_and(revoked) FROM (SELECT revoked FROM oauth_tokens WHERE client_id=$1 AND token_use='oauth' ORDER BY created_at DESC LIMIT 2) recent")
            .bind(&fixture.clients[0]).fetch_one(pool).await?;
        let legacy_revoked: bool = sqlx::query_scalar("SELECT bool_and(revoked) FROM (SELECT revoked FROM oauth_tokens WHERE client_id=$1 AND token_use='oauth' ORDER BY created_at DESC LIMIT 2) recent")
            .bind(&fixture.clients[3]).fetch_one(pool).await?;
        ensure!(consent >= 1 && device == 1 && issued == 4 && raw_tokens == 0, "browser flows not backed by real persisted platform OAuth state");
        ensure!(auth_revoked && legacy_revoked, "real platform did not revoke the browser's retained authorization and password pairs");
        println!("PHASE24_LIVE_DATABASE {}", json!({"consentApprovals":consent,"deviceApprovals":device,"issuedClientCount":issued,"rawTokensPersisted":raw_tokens,"retainedPairsRevoked":true}));
        Ok(())
    }.await;
    let _ = platform_stop.send(());
    let _ = playground_stop.send(());
    let (platform_shutdown, playground_shutdown) =
        tokio::join!(stop(platform_server), stop(playground_server));
    result?;
    platform_shutdown?;
    playground_shutdown?;
    Ok(())
}

fn serve(
    listener: TcpListener,
    app: Router,
) -> (
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<std::io::Result<()>>,
) {
    let (send, receive) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = receive.await;
        })
        .await
    });
    (send, server)
}

async fn stop(mut server: tokio::task::JoinHandle<std::io::Result<()>>) -> Result<()> {
    let result = tokio::time::timeout(Duration::from_secs(10), &mut server).await;
    if result.is_err() {
        server.abort();
        let _ = server.await;
    }
    ensure!(
        matches!(result, Ok(Ok(Ok(())))),
        "owned OAuth test listener did not stop cleanly"
    );
    Ok(())
}
