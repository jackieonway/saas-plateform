//! Password and login regressions against disposable PostgreSQL databases.
//! This suite requires explicit TEST_DATABASE_URL and never reads an env file.
//! Run `cargo test login_protection_tests -- --ignored --test-threads=1 --nocapture`.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, ensure, Result};
use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::middleware;
use axum::Router;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::sync::Barrier;
use tokio::task::JoinSet;
use tower::ServiceExt;
use uuid::Uuid;

use crate::config::Config;
use crate::entity_creation_tests::Database;
use crate::routes;
use crate::state::AppState;
use crate::util::password::PasswordService;
use crate::util::{hash, jwt};

const SECRET: &str = "login-protection-owned-database-only";
const PASSWORD: &str = "Owned-login-password-123!";
const OTHER_PASSWORD: &str = "Other-login-password-456!";
const CLIENT_SECRET: &str = "login-protection-client-secret";
const PEER: &str = "192.0.2.10:40123";

async fn database() -> Result<Database> {
    ensure!(
        std::env::var("TEST_DATABASE_URL").is_ok_and(|url| !url.trim().is_empty()),
        "explicit TEST_DATABASE_URL pointing to a disposable test server is required"
    );
    Database::create().await
}

fn config() -> Result<Config> {
    Config::from_values(&HashMap::from([
        ("APPLICATION_ENVIRONMENT".into(), "development".into()),
        (
            "DATABASE_URL".into(),
            "postgres://unused/login-tests".into(),
        ),
        ("JWT_SECRET".into(), SECRET.into()),
    ]))
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

async fn request(
    app: &Router,
    method: Method,
    path: &str,
    bearer: Option<&str>,
    body: Body,
    content_type: &str,
    forwarded: Option<&str>,
) -> Result<Reply> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, content_type);
    if let Some(token) = bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    if let Some(forwarded) = forwarded {
        builder = builder
            .header("x-forwarded-for", forwarded)
            .header("x-real-ip", forwarded);
    }
    let mut request = builder.body(body)?;
    request
        .extensions_mut()
        .insert(ConnectInfo(PEER.parse::<SocketAddr>()?));
    let response = tokio::time::timeout(Duration::from_secs(20), app.clone().oneshot(request))
        .await
        .map_err(|_| anyhow!("login-protection HTTP request timed out"))??;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok(Reply {
        status,
        headers,
        body: serde_json::from_slice(&bytes)?,
    })
}

async fn json_request(
    app: &Router,
    method: Method,
    path: &str,
    bearer: Option<&str>,
    body: Value,
) -> Result<Reply> {
    request(
        app,
        method,
        path,
        bearer,
        Body::from(serde_json::to_vec(&body)?),
        "application/json",
        None,
    )
    .await
}

async fn login(app: &Router, username: &str, password: &str) -> Result<Reply> {
    json_request(
        app,
        Method::POST,
        "/api/auth/login",
        None,
        json!({"username": username, "password": password}),
    )
    .await
}

async fn oauth_password(app: &Router, username: &str, password: &str) -> Result<Reply> {
    form_request(
        app,
        "/oauth/token",
        &[
            ("grant_type", "password"),
            ("client_id", "login-protection-client"),
            ("client_secret", CLIENT_SECRET),
            ("username", username),
            ("password", password),
        ],
    )
    .await
}

async fn form_request(app: &Router, path: &str, form: &[(&str, &str)]) -> Result<Reply> {
    let form = form
        .iter()
        .map(|(name, value)| {
            format!(
                "{}={}",
                utf8_percent_encode(name, NON_ALPHANUMERIC),
                utf8_percent_encode(value, NON_ALPHANUMERIC)
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    request(
        app,
        Method::POST,
        path,
        None,
        Body::from(form),
        "application/x-www-form-urlencoded",
        None,
    )
    .await
}

fn expect(reply: &Reply, status: StatusCode) -> Result<()> {
    ensure!(
        reply.status == status,
        "HTTP status was {}, expected {} (code={})",
        reply.status,
        status,
        reply.body["code"].as_str().unwrap_or("OAuth")
    );
    Ok(())
}

fn limited(reply: &Reply) -> Result<()> {
    expect(reply, StatusCode::TOO_MANY_REQUESTS)?;
    ensure!(
        reply
            .headers
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|seconds| seconds > 0),
        "429 must give a positive Retry-After"
    );
    ensure!(reply.body.get("ticket").is_none(), "429 returned a ticket");
    ensure!(
        reply.body.get("access_token").is_none(),
        "429 returned an access token"
    );
    Ok(())
}

fn field<'a>(reply: &'a Reply, name: &str) -> Result<&'a str> {
    reply.body[name]
        .as_str()
        .ok_or_else(|| anyhow!("HTTP response omitted {name}"))
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

struct Harness {
    app: Router,
    state: AppState,
    tenant: Uuid,
    org: Uuid,
    admin: String,
}

impl Harness {
    async fn create(pool: &PgPool, config: Config) -> Result<Self> {
        let tenant: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(name,code,is_platform) \
             VALUES('Login test platform','admin-tenant',true) RETURNING id",
        )
        .fetch_one(pool)
        .await?;
        let org: Uuid = sqlx::query_scalar(
            "INSERT INTO organizations(tenant_id,name) VALUES($1,'Login test root') RETURNING id",
        )
        .bind(tenant)
        .fetch_one(pool)
        .await?;
        let state = AppState::new(pool.clone(), Arc::new(config));
        let password_hash = state.passwords.hash(PASSWORD).await?;
        let admin: Uuid = sqlx::query_scalar(
            "INSERT INTO users(username,password_hash,is_super_admin) \
             VALUES('login-test-admin',$1,true) RETURNING id",
        )
        .bind(password_hash)
        .fetch_one(pool)
        .await?;
        sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)")
            .bind(admin)
            .bind(org)
            .execute(pool)
            .await?;
        let application: Uuid = sqlx::query_scalar(
            "INSERT INTO applications(name,code) VALUES('Login test management','system') RETURNING id",
        )
        .fetch_one(pool)
        .await?;
        let menu: Uuid = sqlx::query_scalar(
            "INSERT INTO menus(application_id,title,permission,type) \
             VALUES($1,'Manage users','system:user','BUTTON') RETURNING id",
        )
        .bind(application)
        .fetch_one(pool)
        .await?;
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
        sqlx::query(
            "INSERT INTO oauth_clients(client_id,client_secret_hash,client_name,grant_types,scopes,application_id) \
             VALUES('login-protection-client',$1,'Owned login test','password,refresh_token','profile',$2)",
        )
        .bind(hash::sha256_hex(CLIENT_SECRET))
        .bind(application)
        .execute(pool)
        .await?;
        let app = routes::api_router()
            .nest("/oauth", routes::oauth2::oauth_router())
            .layer(middleware::from_fn_with_state(
                state.clone(),
                crate::normalize_client_ip,
            ))
            .with_state(state.clone());
        Ok(Self {
            app,
            state,
            tenant,
            org,
            admin: session(pool, tenant, org, admin).await?,
        })
    }

    async fn user(&self, username: &str, password: &str, email: &str, phone: &str) -> Result<Uuid> {
        let password_hash = self.state.passwords.hash(password).await?;
        self.user_with_hash(username, &password_hash, email, phone)
            .await
    }

    async fn user_with_hash(
        &self,
        username: &str,
        password_hash: &str,
        email: &str,
        phone: &str,
    ) -> Result<Uuid> {
        let user: Uuid = sqlx::query_scalar(
            "INSERT INTO users(username,password_hash,email,phone) VALUES($1,$2,$3,$4) RETURNING id",
        )
        .bind(username)
        .bind(password_hash)
        .bind(email)
        .bind(phone)
        .fetch_one(&self.state.pool)
        .await?;
        sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)")
            .bind(user)
            .bind(self.org)
            .execute(&self.state.pool)
            .await?;
        Ok(user)
    }
}

async fn drain_passwords(state: &AppState) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(15), async {
        while state.passwords.snapshot().admitted != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(|_| anyhow!("password workers did not release their owned permits"))
}

async fn assert_password_change_rechecks_after_waiting_for_user_lock(
    harness: &Harness,
) -> Result<()> {
    let user = harness.user("password-lock-race", PASSWORD, "", "").await?;
    let token = session(&harness.state.pool, harness.tenant, harness.org, user).await?;
    let replacement_hash = harness.state.passwords.hash(OTHER_PASSWORD).await?;
    let original_version: i64 = sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
        .bind(user)
        .fetch_one(&harness.state.pool)
        .await?;
    let mut blocker = harness.state.pool.begin().await?;
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await?;
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(user)
        .execute(&mut *blocker)
        .await?;
    let app = harness.app.clone();
    let pending = tokio::spawn(async move {
        json_request(
            &app,
            Method::PUT,
            "/api/auth/password",
            Some(&token),
            json!({
                "oldPassword": PASSWORD, "newPassword": "Concurrent-user-password-789!",
            }),
        )
        .await
    });
    // Observe the actual blocked statement after both real Argon2 jobs finish.
    // The held user row then commits an administrator's password replacement.
    let waiting =
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let observed: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
                 AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)) \
                 AND query LIKE '%FROM users%' AND query LIKE '%FOR UPDATE%')",
            ).bind(blocker_pid).fetch_one(&harness.state.pool).await?;
                if observed {
                    return Ok::<_, anyhow::Error>(());
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
    if !matches!(waiting, Ok(Ok(()))) {
        blocker.rollback().await?;
        let _ = pending.await;
        return Err(anyhow!(
            "password change never reached the observed user-lock barrier"
        ));
    }
    crate::session::revoke_all_in_tx(&mut blocker, user, Some(&replacement_hash)).await?;
    blocker.commit().await?;
    let reply = pending.await??;
    expect(&reply, StatusCode::CONFLICT)?;
    let (current_hash, current_version): (String, i64) =
        sqlx::query_as("SELECT password_hash,session_version FROM users WHERE id=$1")
            .bind(user)
            .fetch_one(&harness.state.pool)
            .await?;
    ensure!(
        current_hash == replacement_hash,
        "preverified password overwrote a concurrent administrator reset"
    );
    ensure!(
        current_version == original_version + 1,
        "rejected password change performed a second mutation"
    );
    let changed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE user_id=$1 AND action='UPDATE_PASSWORD'",
    )
    .bind(user)
    .fetch_one(&harness.state.pool)
    .await?;
    ensure!(
        changed == 0,
        "rejected stale password change reported a successful update"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit TEST_DATABASE_URL and disposable database privileges"]
async fn random_unknown_logins_hit_capacity_and_untrusted_forwarding_cannot_reset_source(
) -> Result<()> {
    let database = database().await?;
    let assertions = tokio::time::timeout(Duration::from_secs(90), async {
        let mut settings = config()?;
        settings.login_max_entries = 16;
        let harness = Harness::create(&database.pool, settings).await?;
        expect(
            &login(&harness.app, &"a".repeat(321), PASSWORD).await?,
            StatusCode::BAD_REQUEST,
        )?;
        expect(
            &login(&harness.app, "oversized-password", &"a".repeat(1025)).await?,
            StatusCode::BAD_REQUEST,
        )?;
        expect(
            &oauth_password(&harness.app, &"a".repeat(321), PASSWORD).await?,
            StatusCode::BAD_REQUEST,
        )?;
        for number in 0..15 {
            expect(
                &login(&harness.app, &format!("unknown-{number}"), PASSWORD).await?,
                StatusCode::UNAUTHORIZED,
            )?;
        }
        for number in 15..1015 {
            let reply = request(
                &harness.app,
                Method::POST,
                "/api/auth/login",
                None,
                Body::from(serde_json::to_vec(&json!({
                    "username": format!("unknown-{number}"), "password": PASSWORD,
                }))?),
                "application/json",
                Some(&format!("198.51.100.{}", number % 250 + 1)),
            )
            .await?;
            limited(&reply)?;
        }
        let attempts: i64 =
            sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='LOGIN_FAILED'")
                .fetch_one(&database.pool)
                .await?;
        ensure!(
            attempts == 15,
            "capacity rejection reached database audit work"
        );

        // A fresh process-local state demonstrates source policy independently
        // from the full store. No fixture raises a production budget.
        let mut settings = config()?;
        settings.login_source_requests = 3;
        let state = AppState::new(database.pool.clone(), Arc::new(settings));
        let app = routes::api_router()
            .nest("/oauth", routes::oauth2::oauth_router())
            .layer(middleware::from_fn_with_state(
                state.clone(),
                crate::normalize_client_ip,
            ))
            .with_state(state);
        for number in 0..4 {
            let reply = request(
                &app,
                Method::POST,
                "/api/auth/login",
                None,
                Body::from(serde_json::to_vec(&json!({
                    "username": format!("source-unknown-{number}"), "password": PASSWORD,
                }))?),
                "application/json",
                Some(&format!("203.0.113.{}", number + 1)),
            )
            .await?;
            if number < 3 {
                expect(&reply, StatusCode::UNAUTHORIZED)?;
            } else {
                limited(&reply)?;
            }
        }
        limited(&oauth_password(&app, "other-source-unknown", PASSWORD).await?)?;
        let audit_ips: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT ip FROM audit_logs WHERE username LIKE 'source-unknown-%'",
        )
        .fetch_all(&database.pool)
        .await?;
        ensure!(
            audit_ips == ["192.0.2.10"],
            "untrusted headers selected audit or limiter identity"
        );
        // Exercise the supported maximum setting separately from the default
        // input policy and full-store tests above, including lowercase expansion.
        let mut settings = config()?;
        settings.login_identifier_max_bytes = 1024;
        let state = AppState::new(database.pool.clone(), Arc::new(settings));
        let app = routes::api_router()
            .layer(middleware::from_fn_with_state(
                state.clone(),
                crate::normalize_client_ip,
            ))
            .with_state(state);
        for identifier in [
            "a".repeat(1024),
            format!("{}@example.test", "İ".repeat(500)),
        ] {
            ensure!(
                identifier.len() <= 1024,
                "boundary fixture exceeds configured input maximum"
            );
            expect(
                &login(&app, &identifier, PASSWORD).await?,
                StatusCode::UNAUTHORIZED,
            )?;
        }
        Ok(())
    })
    .await
    .map_err(|_| anyhow!("random-login regression deadline exceeded"))
    .and_then(|result| result);
    database.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit TEST_DATABASE_URL and disposable database privileges"]
async fn shared_email_and_phone_success_does_not_penalize_other_matching_accounts() -> Result<()> {
    let database = database().await?;
    let assertions = tokio::time::timeout(Duration::from_secs(90), async {
        let harness = Harness::create(&database.pool, config()?).await?;
        let first = harness
            .user(
                "alias-first",
                PASSWORD,
                "shared@example.test",
                "+8613800001234",
            )
            .await?;
        let second = harness
            .user(
                "alias-second",
                OTHER_PASSWORD,
                "shared@example.test",
                "+86 138-0000-1234",
            )
            .await?;
        for attempt in 0..6 {
            let identifier = if attempt % 2 == 0 {
                "SHARED@EXAMPLE.TEST"
            } else {
                "+86 (138) 0000-1234"
            };
            let reply = login(&harness.app, identifier, PASSWORD).await?;
            expect(&reply, StatusCode::OK)?;
            ensure!(
                reply.body["accountCount"] == 1,
                "wrong-password alias account was authenticated"
            );
            let claims = jwt::verify(field(&reply, "ticket")?, SECRET)?;
            ensure!(
                claims.sub == first.to_string(),
                "alias ticket selected unverified identity"
            );
        }
        for identifier in ["alias-second", "shared@example.test", "+8613800001234"] {
            let reply = login(&harness.app, identifier, OTHER_PASSWORD).await?;
            expect(&reply, StatusCode::OK)?;
            let claims = jwt::verify(field(&reply, "ticket")?, SECRET)?;
            ensure!(
                claims.sub == second.to_string(),
                "successful alias changed account identity"
            );
        }
        let failures: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_logs WHERE action='LOGIN_FAILED' AND user_id=$1",
        )
        .bind(second)
        .fetch_one(&database.pool)
        .await?;
        ensure!(
            failures == 0,
            "successful shared alias penalized its other account"
        );
        // Failure protection still applies when no matching proof succeeds.
        for _ in 0..5 {
            expect(
                &login(&harness.app, "alias-second", "Incorrect-password-999!").await?,
                StatusCode::UNAUTHORIZED,
            )?;
        }
        limited(&login(&harness.app, "alias-second", OTHER_PASSWORD).await?)?;
        drain_passwords(&harness.state).await
    })
    .await
    .map_err(|_| anyhow!("shared-alias regression deadline exceeded"))
    .and_then(|result| result);
    database.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit TEST_DATABASE_URL and disposable database privileges"]
async fn shared_alias_aggregate_verify_deadline_never_returns_partial_credentials() -> Result<()> {
    let database = database().await?;
    let assertions = tokio::time::timeout(Duration::from_secs(90), async {
        let mut settings = config()?;
        // Allow several real proofs at this machine's existing Argon2 cost.
        // Resetting the deadline per candidate would authenticate all 64;
        // a shared deadline must stop between proofs and return no ticket.
        // Calibration never raises the production default aggregate budget.
        let passwords = PasswordService::new(
            settings.password_concurrency,
            settings.password_queue_capacity,
            Duration::from_millis(settings.password_queue_timeout_ms),
            Duration::from_millis(settings.password_execution_timeout_ms),
        );
        let password_hash = passwords.hash(PASSWORD).await?;
        ensure!(
            passwords.verify(&password_hash, PASSWORD).await?,
            "calibration proof failed"
        );
        let began = Instant::now();
        for _ in 0..3 {
            ensure!(
                passwords.verify(&password_hash, PASSWORD).await?,
                "calibration proof failed"
            );
        }
        settings.login_verify_budget_ms = (began.elapsed().as_millis() as u64)
            .saturating_mul(3)
            .max(20)
            .min(settings.login_verify_budget_ms);
        let harness = Harness::create(&database.pool, settings).await?;
        for number in 0..64 {
            harness
                .user_with_hash(
                    &format!("budget-user-{number}"),
                    &password_hash,
                    "budget@example.test",
                    "",
                )
                .await?;
        }
        let reply = login(&harness.app, "budget@example.test", PASSWORD).await?;
        limited(&reply)?;
        ensure!(
            reply.body.get("memberships").is_none(),
            "expired aggregate proof returned partial memberships"
        );
        let completed_proofs: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM users WHERE email='budget@example.test' AND last_login_at IS NOT NULL",
        )
        .fetch_one(&database.pool)
        .await?;
        ensure!(
            (1..64).contains(&completed_proofs),
            "aggregate deadline did not stop between real candidate proofs"
        );
        let tokens: i64 = sqlx::query_scalar("SELECT count(*) FROM oauth_tokens")
            .fetch_one(&database.pool)
            .await?;
        ensure!(
            tokens == 0,
            "expired alias proof issued persistent credentials"
        );
        drain_passwords(&harness.state).await
    })
    .await
    .map_err(|_| anyhow!("aggregate verification regression deadline exceeded"))
    .and_then(|result| result);
    database.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit TEST_DATABASE_URL and disposable database privileges"]
async fn password_http_lifecycle_preserves_hashes_login_oauth_reset_and_revocation() -> Result<()> {
    let database = database().await?;
    let assertions = tokio::time::timeout(Duration::from_secs(90), async {
        let harness = Harness::create(&database.pool, config()?).await?;
        let created = json_request(
            &harness.app,
            Method::POST,
            "/api/users",
            Some(&harness.admin),
            json!({
                "username": "created-password-user", "password": PASSWORD,
            }),
        )
        .await?;
        expect(&created, StatusCode::OK)?;
        let user: Uuid = field(&created, "id")?.parse()?;
        sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)")
            .bind(user)
            .bind(harness.org)
            .execute(&database.pool)
            .await?;
        let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1")
            .bind(user)
            .fetch_one(&database.pool)
            .await?;
        ensure!(
            hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
            "create changed established Argon2 policy"
        );
        ensure!(
            harness.state.passwords.verify(&hash, PASSWORD).await?,
            "create hash is incompatible"
        );
        expect(
            &login(&harness.app, "created-password-user", PASSWORD).await?,
            StatusCode::OK,
        )?;
        let token = session(&database.pool, harness.tenant, harness.org, user).await?;
        let changed = json_request(
            &harness.app,
            Method::PUT,
            "/api/auth/password",
            Some(&token),
            json!({
                "oldPassword": PASSWORD, "newPassword": OTHER_PASSWORD,
            }),
        )
        .await?;
        expect(&changed, StatusCode::OK)?;
        let old_profile = json_request(
            &harness.app,
            Method::GET,
            "/api/auth/profile",
            Some(&token),
            Value::Null,
        )
        .await?;
        expect(&old_profile, StatusCode::UNAUTHORIZED)?;
        expect(
            &login(&harness.app, "created-password-user", PASSWORD).await?,
            StatusCode::UNAUTHORIZED,
        )?;
        let login_reply = login(&harness.app, "created-password-user", OTHER_PASSWORD).await?;
        expect(&login_reply, StatusCode::OK)?;
        let context = json_request(
            &harness.app,
            Method::POST,
            "/api/auth/context",
            Some(field(&login_reply, "ticket")?),
            json!({"organizationId": harness.org}),
        )
        .await?;
        expect(&context, StatusCode::OK)?;
        let platform_access = field(&context, "accessToken")?.to_owned();
        let platform_refresh = field(&context, "refreshToken")?.to_owned();
        let application = json_request(
            &harness.app,
            Method::POST,
            "/api/oauth/session-token",
            Some(&platform_access),
            json!({"appCode": "system"}),
        )
        .await?;
        expect(&application, StatusCode::CREATED)?;
        let app_access = field(&application, "accessToken")?.to_owned();
        expect(
            &json_request(
                &harness.app,
                Method::GET,
                "/oauth/userinfo",
                Some(&app_access),
                Value::Null,
            )
            .await?,
            StatusCode::OK,
        )?;
        let oauth = oauth_password(&harness.app, "created-password-user", OTHER_PASSWORD).await?;
        expect(&oauth, StatusCode::OK)?;
        let old_access = field(&oauth, "access_token")?.to_owned();
        let reset = json_request(
            &harness.app,
            Method::POST,
            &format!("/api/users/{user}/reset-password"),
            Some(&harness.admin),
            json!({
                "newPassword": PASSWORD,
            }),
        )
        .await?;
        expect(&reset, StatusCode::OK)?;
        let revoked: bool =
            sqlx::query_scalar("SELECT revoked FROM oauth_tokens WHERE token_hash=$1")
                .bind(hash::sha256_hex(&old_access))
                .fetch_one(&database.pool)
                .await?;
        ensure!(
            !revoked,
            "reset silently changed the existing ordinary OAuth revocation policy"
        );
        expect(
            &json_request(
                &harness.app,
                Method::GET,
                "/api/auth/profile",
                Some(&platform_access),
                Value::Null,
            )
            .await?,
            StatusCode::UNAUTHORIZED,
        )?;
        let refresh = form_request(
            &harness.app,
            "/oauth/token",
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", &platform_refresh),
            ],
        )
        .await?;
        expect(&refresh, StatusCode::BAD_REQUEST)?;
        ensure!(
            refresh.body["error"] == "invalid_grant",
            "reset platform refresh returned an unrelated error"
        );
        expect(
            &json_request(
                &harness.app,
                Method::GET,
                "/oauth/userinfo",
                Some(&app_access),
                Value::Null,
            )
            .await?,
            StatusCode::UNAUTHORIZED,
        )?;
        // Ordinary OAuth has independent explicit revocation. A password reset
        // preserves that established policy while rejecting new old-password grants.
        expect(
            &json_request(
                &harness.app,
                Method::GET,
                "/oauth/userinfo",
                Some(&old_access),
                Value::Null,
            )
            .await?,
            StatusCode::OK,
        )?;
        expect(
            &form_request(
                &harness.app,
                "/oauth/revoke",
                &[
                    ("client_id", "login-protection-client"),
                    ("client_secret", CLIENT_SECRET),
                    ("token", &old_access),
                ],
            )
            .await?,
            StatusCode::OK,
        )?;
        expect(
            &json_request(
                &harness.app,
                Method::GET,
                "/oauth/userinfo",
                Some(&old_access),
                Value::Null,
            )
            .await?,
            StatusCode::UNAUTHORIZED,
        )?;
        expect(
            &oauth_password(&harness.app, "created-password-user", OTHER_PASSWORD).await?,
            StatusCode::BAD_REQUEST,
        )?;
        let oauth = oauth_password(&harness.app, "created-password-user", PASSWORD).await?;
        expect(&oauth, StatusCode::OK)?;
        ensure!(
            !field(&oauth, "access_token")?.is_empty(),
            "new password did not issue OAuth token"
        );
        assert_password_change_rechecks_after_waiting_for_user_lock(&harness).await?;
        drain_passwords(&harness.state).await
    })
    .await
    .map_err(|_| anyhow!("password lifecycle regression deadline exceeded"))
    .and_then(|result| result);
    database.finish(assertions).await
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires explicit TEST_DATABASE_URL and disposable database privileges"]
async fn mixed_argon2_http_pressure_keeps_health_and_authenticated_reads_responsive() -> Result<()>
{
    let database = database().await?;
    let assertions =
        tokio::time::timeout(Duration::from_secs(120), async {
            let harness = Harness::create(&database.pool, config()?).await?;
            let user = harness.user("pressure-user", PASSWORD, "", "").await?;
            let user_token = session(&database.pool, harness.tenant, harness.org, user).await?;
            let barrier = Arc::new(Barrier::new(25));
            let mut requests = JoinSet::new();
            for number in 0..24 {
                let app = harness.app.clone();
                let barrier = barrier.clone();
                let token = if number < 18 {
                    user_token.clone()
                } else {
                    harness.admin.clone()
                };
                requests.spawn(async move {
                    barrier.wait().await;
                    let reply = match number / 6 {
                    0 => login(&app, "pressure-user", "Incorrect-password-999!").await?,
                    1 => oauth_password(&app, "pressure-user", "Incorrect-password-999!").await?,
                    2 => json_request(&app, Method::PUT, "/api/auth/password", Some(&token), json!({
                        "oldPassword": "Incorrect-password-999!", "newPassword": OTHER_PASSWORD,
                    })).await?,
                    _ => json_request(&app, Method::POST, "/api/users", Some(&token), json!({
                        "username": format!("pressure-created-{number}"), "password": PASSWORD,
                    })).await?,
                };
                    let allowed = match number / 6 {
                        0 => [StatusCode::UNAUTHORIZED, StatusCode::TOO_MANY_REQUESTS],
                        1 | 2 => [StatusCode::BAD_REQUEST, StatusCode::TOO_MANY_REQUESTS],
                        _ => [StatusCode::OK, StatusCode::TOO_MANY_REQUESTS],
                    };
                    ensure!(
                        allowed.contains(&reply.status),
                        "pressure route {number} returned {}",
                        reply.status
                    );
                    if reply.status == StatusCode::TOO_MANY_REQUESTS {
                        limited(&reply)?;
                    }
                    Ok::<_, anyhow::Error>(reply.status)
                });
            }
            barrier.wait().await;
            let active = tokio::time::timeout(
                Duration::from_secs(5),
                harness.state.passwords.wait_for_active(),
            )
            .await
            .map_err(|_| anyhow!("HTTP pressure never entered the shared Argon2 service"))?;
            ensure!(
                active.active > 0,
                "workload barrier did not observe real Argon2 work"
            );
            ensure!(
                active.active <= 2 && active.admitted <= 10,
                "HTTP workload exceeded configured password budgets"
            );
            for path in ["/api/health", "/api/auth/profile", "/api/health"] {
                let began = Instant::now();
                let reply = tokio::time::timeout(
                    Duration::from_secs(2),
                    json_request(
                        &harness.app,
                        Method::GET,
                        path,
                        Some(&harness.admin),
                        Value::Null,
                    ),
                )
                .await
                .map_err(|_| anyhow!("ordinary HTTP read blocked during Argon2 load"))??;
                ensure!(
                    began.elapsed() < Duration::from_secs(2),
                    "synchronous password work stalled Tokio beyond read deadline"
                );
                expect(&reply, StatusCode::OK)?;
            }
            let mut rejected = 0;
            while let Some(result) = requests.join_next().await {
                if result?? == StatusCode::TOO_MANY_REQUESTS {
                    rejected += 1;
                }
            }
            ensure!(
                rejected > 0,
                "mixed password flood did not exercise explicit overload response"
            );
            drain_passwords(&harness.state).await
        })
        .await
        .map_err(|_| anyhow!("mixed password-pressure regression deadline exceeded"))
        .and_then(|result| result);
    database.finish(assertions).await
}
