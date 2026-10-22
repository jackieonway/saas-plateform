//! Ordinary OAuth regressions against disposable PostgreSQL databases.
//! Set TEST_DATABASE_URL explicitly; this suite never loads an environment file.
//! Run `cargo test oauth_issuance_tests -- --ignored --test-threads=1 --nocapture`.
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use chrono::{DateTime, Utc};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::Value;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use tokio::sync::Barrier;
use tokio::task::JoinSet;
use tower::ServiceExt;
use uuid::Uuid;

use crate::config::Config;
use crate::entity_creation_tests::Database;
use crate::routes;
use crate::state::AppState;
use crate::util::{hash, jwt};

const SECRET: &str = "ordinary-oauth-disposable-database-test-secret";
const CLIENT_SECRET: &str = "synthetic-oauth-client-secret";
const REDIRECT: &str = "https://consumer.example.test/callback";

async fn database() -> Result<Database> {
    ensure!(
        std::env::var("TEST_DATABASE_URL").is_ok_and(|url| !url.trim().is_empty()),
        "explicit TEST_DATABASE_URL pointing to a disposable test server is required"
    );
    Database::create().await
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value[name]
        .as_str()
        .ok_or_else(|| anyhow!("OAuth response omitted {name}"))
}

async fn request(
    app: &Router,
    method: Method,
    path: &str,
    bearer: Option<&str>,
    form: &[(&str, &str)],
) -> Result<(StatusCode, Value)> {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(token) = bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let body = form
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                utf8_percent_encode(key, NON_ALPHANUMERIC),
                utf8_percent_encode(value, NON_ALPHANUMERIC)
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    let response = tokio::time::timeout(
        Duration::from_secs(30),
        app.clone().oneshot(
            builder
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))?,
        ),
    )
    .await
    .map_err(|_| anyhow!("ordinary OAuth request timed out"))??;
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((status, serde_json::from_slice(&body)?))
}

struct Harness {
    app: Router,
    client: String,
    user: Uuid,
    tenant: Uuid,
    parent: Uuid,
    org: Uuid,
}

impl Harness {
    async fn create(pool: &PgPool) -> Result<Self> {
        let suffix = Uuid::new_v4().simple().to_string();
        let tenant = sqlx::query_scalar(
            "INSERT INTO tenants(name,code) VALUES('OAuth test tenant',$1) RETURNING id",
        )
        .bind(format!("oauth-{suffix}"))
        .fetch_one(pool)
        .await?;
        let parent = sqlx::query_scalar(
            "INSERT INTO organizations(tenant_id,name) VALUES($1,'OAuth ancestor') RETURNING id",
        )
        .bind(tenant)
        .fetch_one(pool)
        .await?;
        let org = sqlx::query_scalar(
            "INSERT INTO organizations(tenant_id,parent_id,name) VALUES($1,$2,'OAuth organization') RETURNING id",
        )
        .bind(tenant)
        .bind(parent)
        .fetch_one(pool)
        .await?;
        let user = sqlx::query_scalar(
            "INSERT INTO users(username,password_hash) VALUES($1,'unused') RETURNING id",
        )
        .bind(format!("oauth-user-{suffix}"))
        .fetch_one(pool)
        .await?;
        sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)")
            .bind(user)
            .bind(org)
            .execute(pool)
            .await?;
        let client = format!("oauth-client-{suffix}");
        sqlx::query(
            "INSERT INTO oauth_clients(client_id,client_secret_hash,client_name,redirect_uris,grant_types,scopes,access_token_validity,refresh_token_validity) \
             VALUES($1,$2,'OAuth test client',$3,'authorization_code,refresh_token,client_credentials,password,urn:ietf:params:oauth:grant-type:device_code','profile,email',3600,86400)",
        )
        .bind(&client)
        .bind(hash::sha256_hex(CLIENT_SECRET))
        .bind(REDIRECT)
        .execute(pool)
        .await?;
        let config = Config::from_values(&HashMap::from([
            ("APPLICATION_ENVIRONMENT".into(), "development".into()),
            (
                "DATABASE_URL".into(),
                "postgres://unused/oauth-tests".into(),
            ),
            ("JWT_SECRET".into(), SECRET.into()),
        ]))?;
        let app = routes::api_router()
            .nest("/oauth", routes::oauth2::oauth_router())
            .with_state(AppState::new(pool.clone(), Arc::new(config)));
        Ok(Self {
            app,
            client,
            user,
            tenant,
            parent,
            org,
        })
    }

    async fn post(&self, path: &str, fields: &[(&str, &str)]) -> Result<(StatusCode, Value)> {
        let mut form = vec![
            ("client_id", self.client.as_str()),
            ("client_secret", CLIENT_SECRET),
        ];
        form.extend_from_slice(fields);
        request(&self.app, Method::POST, path, None, &form).await
    }

    async fn seed_refresh(&self, pool: &PgPool) -> Result<String> {
        let token = hash::random_token(43);
        sqlx::query(
            "INSERT INTO oauth_tokens(token_hash,token_type,user_id,client_id,tenant_id,organization_id,scope,expires_at) \
             VALUES($1,'refresh',$2,$3,$4,$5,'profile email',now()+interval '1 day')",
        )
        .bind(hash::sha256_hex(&token))
        .bind(self.user)
        .bind(&self.client)
        .bind(self.tenant)
        .bind(self.org)
        .execute(pool)
        .await?;
        Ok(token)
    }

    async fn seed_code(&self, pool: &PgPool) -> Result<String> {
        let code = hash::random_token(43);
        sqlx::query(
            "INSERT INTO oauth_authorization_codes(code_hash,client_id,user_id,tenant_id,organization_id,redirect_uri,scope,expires_at) \
             VALUES($1,$2,$3,$4,$5,$6,'profile email',now()+interval '5 minutes')",
        )
        .bind(hash::sha256_hex(&code))
        .bind(&self.client)
        .bind(self.user)
        .bind(self.tenant)
        .bind(self.org)
        .bind(REDIRECT)
        .execute(pool)
        .await?;
        Ok(code)
    }

    async fn seed_device(&self, pool: &PgPool) -> Result<String> {
        let code = hash::random_token(43);
        sqlx::query(
            "INSERT INTO oauth_device_codes(device_code_hash,user_code,client_id,user_id,tenant_id,organization_id,scope,status,interval_seconds,expires_at) \
             VALUES($1,$2,$3,$4,$5,$6,'profile email','APPROVED',0,now()+interval '5 minutes')",
        )
        .bind(hash::sha256_hex(&code))
        .bind(Uuid::new_v4().simple().to_string()[..16].to_owned())
        .bind(&self.client)
        .bind(self.user)
        .bind(self.tenant)
        .bind(self.org)
        .execute(pool)
        .await?;
        Ok(code)
    }

    async fn refresh(&self, token: &str) -> Result<(StatusCode, Value)> {
        self.post(
            "/oauth/token",
            &[("grant_type", "refresh_token"), ("refresh_token", token)],
        )
        .await
    }

    async fn exchange(&self, kind: &str, code: &str) -> Result<(StatusCode, Value)> {
        let fields = match kind {
            "code" => vec![
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", REDIRECT),
            ],
            "device" => vec![
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", code),
            ],
            _ => vec![("grant_type", "refresh_token"), ("refresh_token", code)],
        };
        self.post("/oauth/token", &fields).await
    }

    async fn tokens(&self, pool: &PgPool) -> Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT count(*) FROM oauth_tokens WHERE client_id=$1")
                .bind(&self.client)
                .fetch_one(pool)
                .await?,
        )
    }

    async fn assert_identity(&self, token: &str, active: bool) -> Result<()> {
        let (status, info) =
            request(&self.app, Method::GET, "/oauth/userinfo", Some(token), &[]).await?;
        ensure!(
            status
                == if active {
                    StatusCode::OK
                } else {
                    StatusCode::UNAUTHORIZED
                },
            "userinfo identity validity differs from current state"
        );
        if active {
            ensure!(
                info["sub"] == self.user.to_string(),
                "userinfo changed identity"
            );
        }
        let (status, info) = self.post("/oauth/introspect", &[("token", token)]).await?;
        ensure!(
            (status == StatusCode::OK && info["active"] == true) == active,
            "introspection identity validity differs from current state"
        );
        if !active {
            ensure!(
                (status == StatusCode::OK && info["active"] == false)
                    || (status == StatusCode::UNAUTHORIZED && info["error"] == "invalid_client"),
                "invalid context introspection returned an unexpected protocol error"
            );
        }
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL server"]
async fn legacy_null_client_scopes_issue_and_refresh_an_empty_scope() -> Result<()> {
    let db = database().await?;
    let assertions = async {
        let h = Harness::create(&db.pool).await?;
        sqlx::query("UPDATE oauth_clients SET scopes=NULL WHERE client_id=$1")
            .bind(&h.client)
            .execute(&db.pool)
            .await?;
        let (status, pair) = h
            .post("/oauth/token", &[("grant_type", "client_credentials")])
            .await?;
        ensure!(
            status == StatusCode::OK,
            "legacy NULL scopes prevented issuance"
        );
        ensure!(
            pair["scope"] == "",
            "NULL client scopes were not normalized to empty"
        );
        let access = field(&pair, "access_token")?;
        ensure!(jwt::verify(access, SECRET)?.scope.as_deref() == Some(""));
        let (status, refreshed) = h.refresh(field(&pair, "refresh_token")?).await?;
        ensure!(
            status == StatusCode::OK,
            "legacy NULL scopes prevented rotation"
        );
        ensure!(refreshed["scope"] == "", "rotation widened an empty scope");
        Ok(())
    }
    .await;
    db.finish(assertions).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicitly configured disposable PostgreSQL server"]
async fn twenty_refreshes_have_one_winner_and_replay_is_invalid_grant() -> Result<()> {
    let db = database().await?;
    let assertions = async {
        let h = Harness::create(&db.pool).await?;
        let token = h.seed_refresh(&db.pool).await?;
        let barrier = Arc::new(Barrier::new(21));
        let mut tasks = JoinSet::new();
        for _ in 0..20 {
            let (app, client, token, barrier) = (
                h.app.clone(),
                h.client.clone(),
                token.clone(),
                barrier.clone(),
            );
            tasks.spawn(async move {
                barrier.wait().await;
                request(
                    &app,
                    Method::POST,
                    "/oauth/token",
                    None,
                    &[
                        ("client_id", &client),
                        ("client_secret", CLIENT_SECRET),
                        ("grant_type", "refresh_token"),
                        ("refresh_token", &token),
                    ],
                )
                .await
            });
        }
        barrier.wait().await;
        let mut winners = 0;
        while let Some(result) = tasks.join_next().await {
            let (status, body) = result??;
            if status == StatusCode::OK {
                winners += 1;
                ensure!(
                    field(&body, "access_token").is_ok() && field(&body, "refresh_token").is_ok(),
                    "winner omitted a token from the pair"
                );
            } else {
                ensure!(
                    status == StatusCode::BAD_REQUEST && body["error"] == "invalid_grant",
                    "concurrent refresh loser did not return invalid_grant"
                );
            }
        }
        ensure!(
            winners == 1,
            "ordinary refresh had {winners} successful consumers instead of one"
        );
        let (status, body) = h.refresh(&token).await?;
        ensure!(
            status == StatusCode::BAD_REQUEST && body["error"] == "invalid_grant",
            "refresh replay accepted"
        );
        ensure!(
            h.tokens(&db.pool).await? == 3,
            "concurrent refresh persisted duplicate or partial token pairs"
        );
        Ok(())
    }
    .await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL server"]
async fn authorization_and_approved_device_codes_are_consumed_once() -> Result<()> {
    let db = database().await?;
    let assertions = async {
        for kind in ["code", "device"] {
            let h = Harness::create(&db.pool).await?;
            let code = if kind == "code" { h.seed_code(&db.pool).await? } else { h.seed_device(&db.pool).await? };
            let (left, right) = tokio::join!(h.exchange(kind, &code), h.exchange(kind, &code));
            let results = [left?, right?];
            ensure!(results.iter().filter(|r| r.0 == StatusCode::OK).count() == 1,
                "{kind} was consumed more or less than once");
            let expected = if kind == "code" { "invalid_grant" } else { "expired_token" };
            for (status, body) in results.iter().filter(|r| r.0 != StatusCode::OK) {
                ensure!(*status == StatusCode::BAD_REQUEST && body["error"] == expected,
                    "{kind} concurrent replay error changed");
            }
            let (status, body) = h.exchange(kind, &code).await?;
            ensure!(status == StatusCode::BAD_REQUEST && body["error"] == expected, "{kind} replay accepted");
            ensure!(h.tokens(&db.pool).await? == 2, "{kind} persisted duplicate or partial token pairs");
        }
        let h = Harness::create(&db.pool).await?;
        let expired = h.seed_device(&db.pool).await?;
        sqlx::query("UPDATE oauth_device_codes SET expires_at=now()-interval '1 second' WHERE device_code_hash=$1")
            .bind(hash::sha256_hex(&expired)).execute(&db.pool).await?;
        let (status, body) = h.exchange("device", &expired).await?;
        ensure!(status == StatusCode::BAD_REQUEST && body["error"] == "expired_token",
            "approved device code ignored its absolute expiration");
        ensure!(h.tokens(&db.pool).await? == 0, "expired device code issued tokens");
        Ok(())
    }.await;
    db.finish(assertions).await
}

async fn inject_failure(pool: &PgPool, kind: &str, stage: &str) -> Result<&'static str> {
    let table = match kind {
        "code" => "oauth_authorization_codes",
        "device" => "oauth_device_codes",
        _ => "oauth_tokens",
    };
    let consumption_event = match kind {
        "code" => "UPDATE OF consumed",
        "device" => "UPDATE OF status",
        _ => "UPDATE OF revoked",
    };
    let (target, event, body) = match stage {
        "consume_error" => (table, consumption_event, "RAISE EXCEPTION 'synthetic credential consumption failure';".to_string()),
        "consume_skip" => (table, consumption_event, "RETURN NULL;".to_string()),
        "audit_error" => ("audit_logs", "INSERT", "IF NEW.action='TOKEN_ISSUED' THEN RAISE EXCEPTION 'synthetic audit failure'; END IF;".to_string()),
        "audit_skip" => ("audit_logs", "INSERT", "IF NEW.action='TOKEN_ISSUED' THEN RETURN NULL; END IF;".to_string()),
        "access_skip" => ("oauth_tokens", "INSERT", "IF NEW.token_type='access' THEN RETURN NULL; END IF;".to_string()),
        "refresh_skip" => ("oauth_tokens", "INSERT", "IF NEW.token_type='refresh' THEN RETURN NULL; END IF;".to_string()),
        token_type => ("oauth_tokens", "INSERT", format!(
            "IF NEW.token_type='{token_type}' THEN RAISE EXCEPTION 'synthetic token insertion failure'; END IF;")),
    };
    sqlx::raw_sql(&format!(
        "CREATE OR REPLACE FUNCTION phase11_injected_failure() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN {body} RETURN NEW; END; $$; \
         CREATE TRIGGER phase11_failure BEFORE {event} ON {target} FOR EACH ROW EXECUTE FUNCTION phase11_injected_failure();"
    )).execute(pool).await?;
    Ok(target)
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL server"]
async fn credential_consumption_and_both_token_writes_roll_back_together() -> Result<()> {
    let db = database().await?;
    let assertions = async {
        for kind in ["refresh", "code", "device"] {
            for stage in ["consume_error", "consume_skip", "access", "refresh", "access_skip", "refresh_skip", "audit_error", "audit_skip"] {
                let h = Harness::create(&db.pool).await?;
                let credential = match kind {
                    "refresh" => h.seed_refresh(&db.pool).await?,
                    "code" => h.seed_code(&db.pool).await?,
                    _ => h.seed_device(&db.pool).await?,
                };
                let before = h.tokens(&db.pool).await?;
                let target = inject_failure(&db.pool, kind, stage).await?;
                let (status, body) = h.exchange(kind, &credential).await?;
                sqlx::query(&format!("DROP TRIGGER phase11_failure ON {target}"))
                    .execute(&db.pool).await?;
                let expected = if stage == "consume_skip" { StatusCode::BAD_REQUEST } else { StatusCode::INTERNAL_SERVER_ERROR };
                ensure!(status == expected, "{kind}/{stage} did not propagate injected failure");
                if stage == "consume_skip" {
                    ensure!(body["error"] == if kind == "device" { "expired_token" } else { "invalid_grant" },
                        "{kind} accepted zero affected credential rows");
                }
                ensure!(h.tokens(&db.pool).await? == before, "{kind}/{stage} persisted a partial token pair");
                let digest = hash::sha256_hex(&credential);
                let intact: bool = match kind {
                    "code" => sqlx::query_scalar("SELECT NOT consumed FROM oauth_authorization_codes WHERE code_hash=$1")
                        .bind(&digest).fetch_one(&db.pool).await?,
                    "device" => sqlx::query_scalar("SELECT status='APPROVED' FROM oauth_device_codes WHERE device_code_hash=$1")
                        .bind(&digest).fetch_one(&db.pool).await?,
                    _ => sqlx::query_scalar("SELECT NOT revoked FROM oauth_tokens WHERE token_hash=$1")
                        .bind(&digest).fetch_one(&db.pool).await?,
                };
                ensure!(intact, "{kind}/{stage} burned the original credential");
                ensure!(h.exchange(kind, &credential).await?.0 == StatusCode::OK,
                    "{kind}/{stage} credential cannot be retried after database recovery");
                ensure!(h.tokens(&db.pool).await? == before + 2, "retry did not persist exactly one complete pair");
            }
        }
        Ok(())
    }.await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL server"]
async fn current_user_context_state_is_consistent_across_three_oauth_endpoints() -> Result<()> {
    let db = database().await?;
    let assertions = async {
        let password_hash = crate::util::password::hash_password("synthetic-phase11-password")?;
        for state in [
            "user",
            "tenant_disabled",
            "tenant_expired",
            "org_disabled",
            "org_expired",
            "ancestor_disabled",
            "ancestor_expired",
            "membership",
            "client",
        ] {
            let h = Harness::create(&db.pool).await?;
            let old = h.seed_refresh(&db.pool).await?;
            let (status, pair) = h.refresh(&old).await?;
            ensure!(status == StatusCode::OK, "valid fixture failed issuance");
            let access = field(&pair, "access_token")?;
            h.assert_identity(access, true).await?;
            sqlx::query("UPDATE users SET password_hash=$2 WHERE id=$1")
                .bind(h.user)
                .bind(&password_hash)
                .execute(&db.pool)
                .await?;
            let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id=$1")
                .bind(h.user)
                .fetch_one(&db.pool)
                .await?;
            let password_form = [
                ("grant_type", "password"),
                ("username", username.as_str()),
                ("password", "synthetic-phase11-password"),
            ];
            ensure!(
                h.post("/oauth/token", &password_form).await?.0 == StatusCode::OK,
                "valid password grant fixture failed issuance"
            );
            let baseline = h.tokens(&db.pool).await?;
            match state {
                "user" => {
                    sqlx::query("UPDATE users SET status='DISABLED' WHERE id=$1")
                        .bind(h.user)
                        .execute(&db.pool)
                        .await?;
                }
                "tenant_disabled" => {
                    sqlx::query("UPDATE tenants SET status='DISABLED' WHERE id=$1")
                        .bind(h.tenant)
                        .execute(&db.pool)
                        .await?;
                }
                "tenant_expired" => {
                    sqlx::query(
                        "UPDATE tenants SET expires_at=now()-interval '1 second' WHERE id=$1",
                    )
                    .bind(h.tenant)
                    .execute(&db.pool)
                    .await?;
                }
                "org_disabled" | "ancestor_disabled" => {
                    sqlx::query("UPDATE organizations SET status='DISABLED' WHERE id=$1")
                        .bind(if state == "org_disabled" {
                            h.org
                        } else {
                            h.parent
                        })
                        .execute(&db.pool)
                        .await?;
                }
                "org_expired" | "ancestor_expired" => {
                    sqlx::query(
                        "UPDATE organizations SET expires_at=now()-interval '1 second' WHERE id=$1",
                    )
                    .bind(if state == "org_expired" {
                        h.org
                    } else {
                        h.parent
                    })
                    .execute(&db.pool)
                    .await?;
                }
                "membership" => {
                    sqlx::query("DELETE FROM memberships WHERE user_id=$1 AND organization_id=$2")
                        .bind(h.user)
                        .bind(h.org)
                        .execute(&db.pool)
                        .await?;
                }
                _ => {
                    sqlx::query("UPDATE oauth_clients SET status='DISABLED' WHERE client_id=$1")
                        .bind(&h.client)
                        .execute(&db.pool)
                        .await?;
                }
            }
            h.assert_identity(access, false).await?;
            let (status, body) = h.refresh(field(&pair, "refresh_token")?).await?;
            if state == "client" {
                ensure!(
                    status == StatusCode::UNAUTHORIZED && body["error"] == "invalid_client",
                    "disabled client refreshed"
                );
            } else {
                ensure!(
                    status == StatusCode::BAD_REQUEST && body["error"] == "invalid_grant",
                    "{state} still refreshed"
                );
            }
            for kind in ["code", "device"] {
                let code = if kind == "code" {
                    h.seed_code(&db.pool).await?
                } else {
                    h.seed_device(&db.pool).await?
                };
                let (status, body) = h.exchange(kind, &code).await?;
                ensure!(
                    status
                        == if state == "client" {
                            StatusCode::UNAUTHORIZED
                        } else {
                            StatusCode::BAD_REQUEST
                        }
                        && body["error"]
                            == if state == "client" {
                                "invalid_client"
                            } else {
                                "invalid_grant"
                            },
                    "{state} still issued tokens through {kind}"
                );
            }
            let (status, body) = h.post("/oauth/token", &password_form).await?;
            ensure!(
                status
                    == if state == "client" {
                        StatusCode::UNAUTHORIZED
                    } else {
                        StatusCode::BAD_REQUEST
                    }
                    && body["error"]
                        == if state == "client" {
                            "invalid_client"
                        } else {
                            "invalid_grant"
                        },
                "{state} still issued tokens through password grant"
            );
            ensure!(
                h.tokens(&db.pool).await? == baseline,
                "{state} persisted more tokens after loss of identity"
            );
        }
        Ok(())
    }
    .await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL server"]
async fn client_credentials_survive_unrelated_user_context_invalidation() -> Result<()> {
    let db = database().await?;
    let assertions = async {
        let h = Harness::create(&db.pool).await?;
        sqlx::query("UPDATE users SET status='DISABLED' WHERE id=$1")
            .bind(h.user)
            .execute(&db.pool)
            .await?;
        sqlx::query("UPDATE tenants SET expires_at=now()-interval '1 day' WHERE id=$1")
            .bind(h.tenant)
            .execute(&db.pool)
            .await?;
        sqlx::query("DELETE FROM memberships WHERE user_id=$1")
            .bind(h.user)
            .execute(&db.pool)
            .await?;
        let (status, pair) = h
            .post("/oauth/token", &[("grant_type", "client_credentials")])
            .await?;
        ensure!(
            status == StatusCode::OK,
            "machine grant acquired user or membership requirements"
        );
        let access = field(&pair, "access_token")?;
        let claims = jwt::verify(access, SECRET)?;
        ensure!(
            claims.sub == h.client && claims.tenant.is_none() && claims.org.is_none(),
            "machine grant acquired user context"
        );
        let (status, info) = h.post("/oauth/introspect", &[("token", access)]).await?;
        ensure!(
            status == StatusCode::OK && info["active"] == true,
            "machine token was rejected by user rules"
        );
        ensure!(
            request(&h.app, Method::GET, "/oauth/userinfo", Some(access), &[])
                .await?
                .0
                == StatusCode::UNAUTHORIZED,
            "machine token exposed a user identity"
        );
        if let Some(refresh) = pair["refresh_token"].as_str() {
            ensure!(
                h.refresh(refresh).await?.0 == StatusCode::OK,
                "machine refresh acquired user requirements"
            );
        }
        Ok(())
    }
    .await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL server"]
async fn public_clients_cannot_issue_or_refresh_machine_credentials() -> Result<()> {
    let db = database().await?;
    let assertions = async {
        let h = Harness::create(&db.pool).await?;
        let (status, machine) = h
            .post("/oauth/token", &[("grant_type", "client_credentials")])
            .await?;
        ensure!(status == StatusCode::OK, "confidential fixture grant failed");
        let refresh = field(&machine, "refresh_token")?;
        sqlx::query("UPDATE oauth_clients SET client_type='PUBLIC',client_secret_hash=NULL WHERE client_id=$1")
            .bind(&h.client).execute(&db.pool).await?;
        let original_count = h.tokens(&db.pool).await?;
        for fields in [
            vec![("client_id", h.client.as_str()), ("grant_type", "client_credentials")],
            vec![("client_id", h.client.as_str()), ("grant_type", "refresh_token"), ("refresh_token", refresh)],
        ] {
            let (status, body) = request(&h.app, Method::POST, "/oauth/token", None, &fields).await?;
            ensure!(
                status == StatusCode::BAD_REQUEST && body["error"] == "unauthorized_client",
                "public identifier obtained a machine credential"
            );
            ensure!(h.tokens(&db.pool).await? == original_count, "rejected public grant persisted credentials");
        }
        Ok(())
    }.await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL server"]
async fn password_reset_retires_oauth_tokens_codes_and_devices_but_keeps_machine_grants(
) -> Result<()> {
    let db = database().await?;
    let assertions = async {
        let h = Harness::create(&db.pool).await?;
        let initial_refresh = h.seed_refresh(&db.pool).await?;
        let (status, pair) = h.refresh(&initial_refresh).await?;
        ensure!(status == StatusCode::OK, "user fixture grant failed");
        let access = field(&pair, "access_token")?;
        let refresh = field(&pair, "refresh_token")?;
        let code = h.seed_code(&db.pool).await?;
        let device = h.seed_device(&db.pool).await?;
        let (status, machine) = h
            .post("/oauth/token", &[("grant_type", "client_credentials")])
            .await?;
        ensure!(status == StatusCode::OK, "machine fixture grant failed");
        h.assert_identity(access, true).await?;
        crate::session::revoke_all(&db.pool, h.user, Some("new-password-hash")).await?;
        h.assert_identity(access, false).await?;
        let (status, body) = h.refresh(refresh).await?;
        ensure!(
            status == StatusCode::BAD_REQUEST && body["error"] == "invalid_grant",
            "old user refresh survived password reset"
        );
        ensure!(
            h.exchange("code", &code).await?.0 == StatusCode::BAD_REQUEST,
            "unconsumed authorization code survived password reset"
        );
        ensure!(
            h.exchange("device", &device).await?.0 == StatusCode::BAD_REQUEST,
            "approved device code survived password reset"
        );
        let active: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM oauth_tokens WHERE user_id=$1 AND revoked=false",
        )
        .bind(h.user)
        .fetch_one(&db.pool)
        .await?;
        ensure!(active == 0, "user credentials remained active after reset");
        let (status, info) = h
            .post(
                "/oauth/introspect",
                &[("token", field(&machine, "access_token")?)],
            )
            .await?;
        ensure!(
            status == StatusCode::OK && info["active"] == true,
            "unrelated machine credential was revoked"
        );
        ensure!(
            h.refresh(field(&machine, "refresh_token")?).await?.0 == StatusCode::OK,
            "unrelated machine refresh was revoked"
        );
        Ok(())
    }
    .await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL server"]
async fn credential_approval_waiting_on_password_reset_rechecks_the_platform_session() -> Result<()>
{
    let db = database().await?;
    let assertions = async {
        for kind in ["consent", "device"] {
            let h = Harness::create(&db.pool).await?;
            let session: Uuid = sqlx::query_scalar(
                "INSERT INTO platform_sessions(user_id,tenant_id,organization_id,expires_at) VALUES($1,$2,$3,now()+interval '1 hour') RETURNING id",
            ).bind(h.user).bind(h.tenant).bind(h.org).fetch_one(&db.pool).await?;
            let now = Utc::now().timestamp();
            let access = jwt::sign(&jwt::Claims {
                sub: h.user.to_string(), typ: "session".into(), tenant: Some(h.tenant.to_string()),
                org: Some(h.org.to_string()), cid: None, scope: None,
                sid: Some(session.to_string()), ver: Some(0), aud: None, jti: None, iat: now, exp: now+3600,
            }, SECRET)?;
            let path = if kind == "consent" {
                let id: Uuid = sqlx::query_scalar(
                    "INSERT INTO oauth_authorization_requests(client_id,redirect_uri,scope,expires_at) VALUES($1,$2,'profile',now()+interval '5 minutes') RETURNING id",
                ).bind(&h.client).bind(REDIRECT).fetch_one(&db.pool).await?;
                format!("/api/oauth/consent/{id}/decision")
            } else {
                let user_code = Uuid::new_v4().simple().to_string()[..16].to_string();
                sqlx::query(
                    "INSERT INTO oauth_device_codes(device_code_hash,user_code,client_id,scope,expires_at) VALUES($1,$2,$3,'profile',now()+interval '5 minutes')",
                ).bind(hash::sha256_hex(&hash::random_token(43))).bind(&user_code).bind(&h.client).execute(&db.pool).await?;
                format!("/api/device/{user_code}/decision")
            };
            let mut blocker = db.pool.begin().await?;
            let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *blocker).await?;
            sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
                .bind(h.user).execute(&mut *blocker).await?;
            let app = h.app.clone();
            let pending = tokio::spawn(async move {
                let response = tokio::time::timeout(Duration::from_secs(30), app.oneshot(
                    Request::builder().method(Method::POST).uri(path)
                        .header(header::AUTHORIZATION, format!("Bearer {access}"))
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from("{\"approve\":true}"))?
                )).await.map_err(|_|anyhow!("credential approval timed out"))??;
                Ok::<_, anyhow::Error>(response.status())
            });
            // Observe the lock wait after CurrentUser has accepted the original
            // session, then commit password revocation before approval resumes.
            let waiting = tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    let observed: bool = sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
                         AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)) \
                         AND query LIKE '%FROM users%' AND query LIKE '%FOR UPDATE%')",
                    ).bind(blocker_pid).fetch_one(&db.pool).await?;
                    if observed { return Ok::<_, anyhow::Error>(()); }
                    tokio::task::yield_now().await;
                }
            }).await;
            if !matches!(waiting, Ok(Ok(()))) {
                blocker.rollback().await?;
                let _ = pending.await;
                return Err(anyhow!("{kind} approval never reached its user lock barrier"));
            }
            crate::session::revoke_all_in_tx(&mut blocker,h.user,Some("replacement-password-hash")).await?;
            blocker.commit().await?;
            ensure!(pending.await?? == StatusCode::UNAUTHORIZED, "{kind} approval reused a revoked platform session");
            let codes: i64 = sqlx::query_scalar("SELECT count(*) FROM oauth_authorization_codes WHERE client_id=$1")
                .bind(&h.client).fetch_one(&db.pool).await?;
            let approved: i64 = sqlx::query_scalar("SELECT count(*) FROM oauth_device_codes WHERE client_id=$1 AND status='APPROVED'")
                .bind(&h.client).fetch_one(&db.pool).await?;
            ensure!(codes == 0 && approved == 0, "revoked approval persisted a usable derived credential");
        }
        Ok(())
    }.await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL server"]
async fn new_tokens_store_digests_have_random_jti_and_revoke_independently() -> Result<()> {
    let db = database().await?;
    let assertions = async {
        let h = Harness::create(&db.pool).await?;
        // Concurrent identical grants exercise the former second-resolution JWT collision.
        let (left, right) = tokio::join!(
            h.post("/oauth/token", &[("grant_type", "client_credentials")]),
            h.post("/oauth/token", &[("grant_type", "client_credentials")])
        );
        let (left, right) = (left?, right?);
        ensure!(
            left.0 == StatusCode::OK && right.0 == StatusCode::OK,
            "identical concurrent grants failed"
        );
        let (first, second) = (
            field(&left.1, "access_token")?,
            field(&right.1, "access_token")?,
        );
        let (a, b) = (jwt::verify(first, SECRET)?, jwt::verify(second, SECRET)?);
        let (a_jti, b_jti) = (
            a.jti.ok_or_else(|| anyhow!("first jti missing"))?,
            b.jti.ok_or_else(|| anyhow!("second jti missing"))?,
        );
        ensure!(
            first != second
                && a_jti != b_jti
                && Uuid::parse_str(&a_jti)?.get_version_num() == 4
                && Uuid::parse_str(&b_jti)?.get_version_num() == 4,
            "access JWT identity is not independently randomized"
        );
        let rows: Vec<(String, String, Option<String>, Option<Uuid>)> = sqlx::query_as(
            "SELECT token_hash,token_type,jwt,jti FROM oauth_tokens WHERE client_id=$1",
        )
        .bind(&h.client)
        .fetch_all(&db.pool)
        .await?;
        ensure!(
            rows.len() == 4 && rows.iter().all(|r| r.2.is_none() && r.0.len() == 64),
            "new token rows contain JWT plaintext or incomplete token pairs"
        );
        for token in [
            first,
            second,
            field(&left.1, "refresh_token")?,
            field(&right.1, "refresh_token")?,
        ] {
            ensure!(
                rows.iter().any(|r| r.0 == hash::sha256_hex(token)),
                "issued credential lacks digest record"
            );
        }
        ensure!(
            rows.iter()
                .filter(|r| r.1 == "access")
                .all(|r| r.3.is_some()),
            "access metadata omitted jti"
        );
        ensure!(
            h.post("/oauth/revoke", &[("token", first)]).await?.0 == StatusCode::OK,
            "revocation failed"
        );
        ensure!(
            h.post("/oauth/introspect", &[("token", first)]).await?.1["active"] == false,
            "revoked access remains active"
        );
        ensure!(
            h.post("/oauth/introspect", &[("token", second)]).await?.1["active"] == true,
            "independent token was revoked with its sibling"
        );
        Ok(())
    }
    .await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL server"]
async fn legacy_access_without_jti_remains_usable_and_refresh_scope_cannot_expand() -> Result<()> {
    let db = database().await?;
    let assertions = async {
        let h = Harness::create(&db.pool).await?;
        let now = Utc::now().timestamp();
        let legacy = jwt::sign(&jwt::Claims {
            sub: h.user.to_string(), typ: "access".into(), tenant: Some(h.tenant.to_string()),
            org: Some(h.org.to_string()), cid: Some(h.client.clone()), scope: Some("profile email".into()),
            sid: None, ver: None, aud: None, jti: None, iat: now, exp: now+3600,
        }, SECRET)?;
        sqlx::query("INSERT INTO oauth_tokens(token_hash,token_type,jwt,user_id,client_id,tenant_id,organization_id,scope,expires_at,storage_version) VALUES($1,'access',$2,$3,$4,$5,$6,'profile email',now()+interval '1 hour',1)")
            .bind(hash::sha256_hex(&legacy)).bind(&legacy).bind(h.user).bind(&h.client).bind(h.tenant).bind(h.org).execute(&db.pool).await?;
        h.assert_identity(&legacy, true).await?;
        let refresh = h.seed_refresh(&db.pool).await?;
        let (status, narrow) = h.post("/oauth/token", &[("grant_type","refresh_token"),("refresh_token",&refresh),("scope","profile")]).await?;
        ensure!(status == StatusCode::OK && narrow["scope"] == "profile", "refresh scope could not narrow");
        let next = field(&narrow,"refresh_token")?;
        let (status, body) = h.post("/oauth/token", &[("grant_type","refresh_token"),("refresh_token",next),("scope","email")]).await?;
        ensure!(status == StatusCode::BAD_REQUEST && body["error"] == "invalid_scope", "refresh enlarged original scope");
        sqlx::query("UPDATE oauth_clients SET scopes='email' WHERE client_id=$1").bind(&h.client).execute(&db.pool).await?;
        let (status, body) = h.refresh(next).await?;
        ensure!(status == StatusCode::BAD_REQUEST && body["error"] == "invalid_scope", "omitted refresh scope bypassed current client scopes");
        // The compatibility window preserves historical rows; only newly issued rows omit plaintext.
        let stored: Option<String> = sqlx::query_scalar("SELECT jwt FROM oauth_tokens WHERE token_hash=$1")
            .bind(hash::sha256_hex(&legacy)).fetch_one(&db.pool).await?;
        ensure!(stored.as_deref() == Some(legacy.as_str()), "issuance destructively cleaned historical JWT storage");
        let (expiry, revoked): (DateTime<Utc>, bool) = sqlx::query_as("SELECT expires_at,revoked FROM oauth_tokens WHERE token_hash=$1")
            .bind(hash::sha256_hex(next)).fetch_one(&db.pool).await?;
        ensure!(expiry > Utc::now() && !revoked, "scope failure burned a valid refresh credential");
        Ok(())
    }.await;
    db.finish(assertions).await
}

async fn approve_consent(h: &Harness, token: &str, id: Uuid) -> Result<StatusCode> {
    let response = tokio::time::timeout(
        Duration::from_secs(30),
        h.app.clone().oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/oauth/consent/{id}/decision"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"approve":true}"#))?,
        ),
    )
    .await
    .map_err(|_| anyhow!("consent decision timed out"))??;
    Ok(response.status())
}

async fn consent_session(h: &Harness, pool: &PgPool) -> Result<String> {
    let session: Uuid = sqlx::query_scalar(
        "INSERT INTO platform_sessions(user_id,tenant_id,organization_id,expires_at) VALUES($1,$2,$3,now()+interval '1 hour') RETURNING id",
    ).bind(h.user).bind(h.tenant).bind(h.org).fetch_one(pool).await?;
    let now = Utc::now().timestamp();
    jwt::sign(
        &jwt::Claims {
            sub: h.user.to_string(),
            typ: "session".into(),
            tenant: Some(h.tenant.to_string()),
            org: Some(h.org.to_string()),
            cid: None,
            scope: None,
            sid: Some(session.to_string()),
            ver: Some(0),
            aud: None,
            jti: None,
            iat: now,
            exp: now + 3600,
        },
        SECRET,
    )
}

async fn pkce_authorize(h: &Harness, pkce: Option<(&str, &str)>) -> Result<(StatusCode, Value)> {
    let mut fields = vec![
        ("client_id", h.client.as_str()),
        ("redirect_uri", REDIRECT),
        ("response_type", "code"),
        ("scope", "profile"),
        ("state", "owned-pkce-state"),
    ];
    if let Some((challenge, method)) = pkce {
        fields.push(("code_challenge", challenge));
        fields.push(("code_challenge_method", method));
    }
    let query = fields
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                utf8_percent_encode(key, NON_ALPHANUMERIC),
                utf8_percent_encode(value, NON_ALPHANUMERIC)
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    let response = tokio::time::timeout(
        Duration::from_secs(30),
        h.app.clone().oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(format!("/oauth/authorize?{query}"))
                .body(Body::empty())?,
        ),
    )
    .await
    .map_err(|_| anyhow!("PKCE authorization timed out"))??;
    let status = response.status();
    if status == StatusCode::FOUND {
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| anyhow!("authorization omitted consent location"))?;
        let location = reqwest::Url::parse("http://platform.example.test")?.join(location)?;
        let request_id = location
            .query_pairs()
            .find(|(key, _)| key == "request_id")
            .map(|(_, value)| value.into_owned())
            .ok_or_else(|| anyhow!("authorization omitted request_id"))?;
        Uuid::parse_str(&request_id)?;
        Ok((status, serde_json::json!({"requestId":request_id})))
    } else {
        let body = to_bytes(response.into_body(), 1024 * 1024).await?;
        Ok((status, serde_json::from_slice(&body)?))
    }
}

async fn consent_code(h: &Harness, token: &str, id: Uuid) -> Result<String> {
    let response = tokio::time::timeout(
        Duration::from_secs(30),
        h.app.clone().oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/oauth/consent/{id}/decision"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"approve":true}"#))?,
        ),
    )
    .await
    .map_err(|_| anyhow!("PKCE consent timed out"))??;
    ensure!(
        response.status() == StatusCode::OK,
        "valid PKCE consent was rejected"
    );
    let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
    let redirect = reqwest::Url::parse(field(&body, "redirectTo")?)?;
    ensure!(
        redirect
            .query_pairs()
            .any(|(key, value)| key == "state" && value == "owned-pkce-state"),
        "consent lost OAuth state"
    );
    redirect
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .ok_or_else(|| anyhow!("consent omitted authorization code"))
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL server"]
async fn pkce_legacy_clients_keep_optional_and_plain_authorization() -> Result<()> {
    let db = database().await?;
    let assertions = async {
        let h = Harness::create(&db.pool).await?;
        let required: bool =
            sqlx::query_scalar("SELECT require_pkce FROM oauth_clients WHERE client_id=$1")
                .bind(&h.client)
                .fetch_one(&db.pool)
                .await?;
        ensure!(!required, "PKCE migration changed legacy client defaults");
        let session = consent_session(&h, &db.pool).await?;
        let verifier = "legacy-verifier-0123456789012345678901234567890123456789";
        for challenge in [None, Some((verifier, "plain"))] {
            let (status, body) = pkce_authorize(&h, challenge).await?;
            ensure!(status == StatusCode::FOUND, "legacy PKCE mode was rejected");
            let id = Uuid::parse_str(field(&body, "requestId")?)?;
            let code = consent_code(&h, &session, id).await?;
            let mut form = vec![
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("redirect_uri", REDIRECT),
            ];
            if challenge.is_some() {
                form.push(("code_verifier", verifier));
            }
            let (status, pair) = h.post("/oauth/token", &form).await?;
            ensure!(
                status == StatusCode::OK && pair["scope"] == "profile",
                "legacy PKCE mode no longer exchanges"
            );
            h.assert_identity(field(&pair, "access_token")?, true)
                .await?;
        }
        Ok(())
    }
    .await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL server"]
async fn pkce_required_s256_rejects_downgrades_and_bad_verifiers_without_consuming_code(
) -> Result<()> {
    let db = database().await?;
    let assertions = async {
        let h = Harness::create(&db.pool).await?;
        sqlx::query("UPDATE oauth_clients SET require_pkce=true,client_type='PUBLIC',grant_types='authorization_code,refresh_token' WHERE client_id=$1")
            .bind(&h.client).execute(&db.pool).await?;
        let session = consent_session(&h, &db.pool).await?;
        let verifier = "required-verifier-0123456789012345678901234567890123456789";
        let challenge = hash::b64url_sha256(verifier);
        let noncanonical = format!("{}B", "A".repeat(42));
        for pkce in [None, Some((verifier, "plain")), Some(("short", "S256")), Some(("!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!", "S256")), Some((noncanonical.as_str(), "S256"))] {
            let (status, body) = pkce_authorize(&h, pkce).await?;
            ensure!(status == StatusCode::BAD_REQUEST && body["error"] == "invalid_request",
                "required PKCE accepted a missing, downgraded or malformed challenge");
        }
        let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM oauth_authorization_requests WHERE client_id=$1")
            .bind(&h.client).fetch_one(&db.pool).await?;
        ensure!(pending == 0, "rejected PKCE challenges persisted pending authorization requests");
        let (status, body) = pkce_authorize(&h, Some((&challenge, "S256"))).await?;
        ensure!(status == StatusCode::FOUND, "required S256 request was rejected");
        let request_id = Uuid::parse_str(field(&body, "requestId")?)?;
        ensure!(request(&h.app, Method::GET, &format!("/api/oauth/consent/{request_id}"), Some(&session), &[]).await?.0 == StatusCode::OK,
            "required S256 consent cannot be displayed");
        let code = consent_code(&h, &session, request_id).await?;
        // Public clients authenticate the exchange using only client_id and PKCE.
        let base = [("client_id", h.client.as_str()), ("grant_type", "authorization_code"), ("code", code.as_str()), ("redirect_uri", REDIRECT)];
        for (supplied, expected_error) in [(None, "invalid_request"), (Some("short"), "invalid_grant"), (Some("wrong-verifier-0123456789012345678901234567890123456789"), "invalid_grant")] {
            let mut form = base.to_vec();
            if let Some(value) = supplied { form.push(("code_verifier", value)); }
            let (status, body) = request(&h.app, Method::POST, "/oauth/token", None, &form).await?;
            ensure!(status == StatusCode::BAD_REQUEST && body["error"] == expected_error,
                "required S256 exchange accepted a missing or wrong verifier");
            let consumed: bool = sqlx::query_scalar("SELECT consumed FROM oauth_authorization_codes WHERE code_hash=$1")
                .bind(hash::sha256_hex(&code)).fetch_one(&db.pool).await?;
            ensure!(!consumed && h.tokens(&db.pool).await? == 0, "failed PKCE exchange consumed code or persisted tokens");
        }
        let mut form = base.to_vec();
        form.push(("code_verifier", verifier));
        let (status, pair) = request(&h.app, Method::POST, "/oauth/token", None, &form).await?;
        ensure!(status == StatusCode::OK && pair["scope"] == "profile", "correct verifier could not recover the same authorization code");
        let claims = jwt::verify(field(&pair, "access_token")?, SECRET)?;
        ensure!(claims.sub == h.user.to_string() && claims.org.as_deref() == Some(h.org.to_string().as_str()),
            "PKCE exchange changed authorized identity or context");
        ensure!(request(&h.app, Method::POST, "/oauth/token", None, &form).await?.0 == StatusCode::BAD_REQUEST,
            "successful PKCE code was consumed more than once");
        Ok(())
    }.await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL server"]
async fn pkce_policy_tightening_invalidates_legacy_pending_consent_and_codes() -> Result<()> {
    let db = database().await?;
    let assertions = async {
        let verifier = "legacy-verifier-0123456789012345678901234567890123456789";
        for pkce in [None, Some((verifier, "plain"))] {
            let h = Harness::create(&db.pool).await?;
            let session = consent_session(&h, &db.pool).await?;
            let (status, pending) = pkce_authorize(&h, pkce).await?;
            ensure!(
                status == StatusCode::FOUND,
                "legacy pending authorization failed before migration"
            );
            let pending_id = Uuid::parse_str(field(&pending, "requestId")?)?;
            let (status, approved) = pkce_authorize(&h, pkce).await?;
            ensure!(
                status == StatusCode::FOUND,
                "legacy authorization failed before migration"
            );
            let approved_id = Uuid::parse_str(field(&approved, "requestId")?)?;
            let code = consent_code(&h, &session, approved_id).await?;
            sqlx::query("UPDATE oauth_clients SET require_pkce=true WHERE client_id=$1")
                .bind(&h.client)
                .execute(&db.pool)
                .await?;
            ensure!(
                request(
                    &h.app,
                    Method::GET,
                    &format!("/api/oauth/consent/{pending_id}"),
                    Some(&session),
                    &[]
                )
                .await?
                .0 == StatusCode::NOT_FOUND,
                "legacy pending consent remained visible after S256 became required"
            );
            ensure!(
                approve_consent(&h, &session, pending_id).await? == StatusCode::BAD_REQUEST,
                "legacy pending consent was approved after S256 became required"
            );
            let mut form = vec![
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("redirect_uri", REDIRECT),
            ];
            if pkce.is_some() {
                form.push(("code_verifier", verifier));
            }
            let (status, body) = h.post("/oauth/token", &form).await?;
            ensure!(
                status == StatusCode::BAD_REQUEST && body["error"] == "invalid_grant",
                "legacy code bypassed tightened PKCE policy"
            );
            let consumed: bool = sqlx::query_scalar(
                "SELECT consumed FROM oauth_authorization_codes WHERE code_hash=$1",
            )
            .bind(hash::sha256_hex(&code))
            .fetch_one(&db.pool)
            .await?;
            let pending_consumed: bool =
                sqlx::query_scalar("SELECT consumed FROM oauth_authorization_requests WHERE id=$1")
                    .bind(pending_id)
                    .fetch_one(&db.pool)
                    .await?;
            ensure!(
                !consumed && !pending_consumed && h.tokens(&db.pool).await? == 0,
                "tightened-policy rejection consumed credentials or persisted tokens"
            );
        }
        Ok(())
    }
    .await;
    db.finish(assertions).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicitly configured disposable PostgreSQL server"]
async fn pkce_policy_tightening_during_client_lock_wait_rejects_legacy_code() -> Result<()> {
    let db = database().await?;
    let assertions = async {
        let h = Harness::create(&db.pool).await?;
        let code = h.seed_code(&db.pool).await?;
        let mut blocker = db.pool.begin().await?;
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await?;
        sqlx::query("SELECT id FROM oauth_clients WHERE client_id=$1 FOR UPDATE")
            .bind(&h.client)
            .execute(&mut *blocker)
            .await?;
        let (app, client, pending_code) = (h.app.clone(), h.client.clone(), code.clone());
        let pending = tokio::spawn(async move {
            request(
                &app,
                Method::POST,
                "/oauth/token",
                None,
                &[
                    ("client_id", client.as_str()),
                    ("client_secret", CLIENT_SECRET),
                    ("grant_type", "authorization_code"),
                    ("code", pending_code.as_str()),
                    ("redirect_uri", REDIRECT),
                ],
            )
            .await
        });
        let waiting = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let observed: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
                     AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)) \
                     AND query LIKE '%FROM oauth_clients%' AND query LIKE '%FOR UPDATE%')",
                ).bind(blocker_pid).fetch_one(&db.pool).await?;
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
                "PKCE exchange never reached its client lock barrier"
            ));
        }
        sqlx::query("UPDATE oauth_clients SET require_pkce=true WHERE client_id=$1")
            .bind(&h.client)
            .execute(&mut *blocker)
            .await?;
        blocker.commit().await?;
        let (status, body) = pending.await??;
        ensure!(
            status == StatusCode::UNAUTHORIZED && body["error"] == "invalid_client",
            "client lock wait bypassed the newly required PKCE policy"
        );
        let consumed: bool =
            sqlx::query_scalar("SELECT consumed FROM oauth_authorization_codes WHERE code_hash=$1")
                .bind(hash::sha256_hex(&code))
                .fetch_one(&db.pool)
                .await?;
        ensure!(
            !consumed && h.tokens(&db.pool).await? == 0,
            "PKCE policy race consumed a legacy code or persisted tokens"
        );
        Ok(())
    }
    .await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL server"]
async fn pending_consent_rechecks_current_redirect_grant_and_scopes() -> Result<()> {
    let db = database().await?;
    let assertions = async {
        for change in ["redirect", "grant", "scope"] {
            let h = Harness::create(&db.pool).await?;
            let session: Uuid = sqlx::query_scalar(
                "INSERT INTO platform_sessions(user_id,tenant_id,organization_id,expires_at) VALUES($1,$2,$3,now()+interval '1 hour') RETURNING id",
            ).bind(h.user).bind(h.tenant).bind(h.org).fetch_one(&db.pool).await?;
            let now = Utc::now().timestamp();
            let access = jwt::sign(&jwt::Claims {
                sub: h.user.to_string(), typ: "session".into(), tenant: Some(h.tenant.to_string()),
                org: Some(h.org.to_string()), cid: None, scope: None,
                sid: Some(session.to_string()), ver: Some(0), aud: None, jti: None, iat: now, exp: now+3600,
            }, SECRET)?;
            let request_id: Uuid = sqlx::query_scalar(
                "INSERT INTO oauth_authorization_requests(client_id,redirect_uri,scope,expires_at) VALUES($1,$2,'profile',now()+interval '5 minutes') RETURNING id",
            ).bind(&h.client).bind(REDIRECT).fetch_one(&db.pool).await?;
            let path = format!("/api/oauth/consent/{request_id}");
            ensure!(request(&h.app, Method::GET, &path, Some(&access), &[]).await?.0 == StatusCode::OK,
                "valid pending consent is unavailable");
            let statement = match change {
                "redirect" => "UPDATE oauth_clients SET redirect_uris='https://replacement.example.test/callback' WHERE client_id=$1",
                "grant" => "UPDATE oauth_clients SET grant_types='refresh_token' WHERE client_id=$1",
                _ => "UPDATE oauth_clients SET scopes='email' WHERE client_id=$1",
            };
            sqlx::query(statement).bind(&h.client).execute(&db.pool).await?;
            ensure!(request(&h.app, Method::GET, &path, Some(&access), &[]).await?.0 == StatusCode::NOT_FOUND,
                "pending consent remained visible after {change} was withdrawn");
            ensure!(approve_consent(&h, &access, request_id).await? == StatusCode::BAD_REQUEST,
                "consent approved after {change} was withdrawn");
            let consumed: bool = sqlx::query_scalar("SELECT consumed FROM oauth_authorization_requests WHERE id=$1")
                .bind(request_id).fetch_one(&db.pool).await?;
            let codes: i64 = sqlx::query_scalar("SELECT count(*) FROM oauth_authorization_codes WHERE client_id=$1")
                .bind(&h.client).fetch_one(&db.pool).await?;
            ensure!(!consumed && codes == 0, "invalid consent consumed its request or issued a code");
        }
        Ok(())
    }.await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL server"]
async fn authorization_code_cannot_exchange_after_its_redirect_is_removed() -> Result<()> {
    let db = database().await?;
    let assertions = async {
        let h = Harness::create(&db.pool).await?;
        let code = h.seed_code(&db.pool).await?;
        sqlx::query("UPDATE oauth_clients SET redirect_uris='https://replacement.example.test/callback' WHERE client_id=$1")
            .bind(&h.client).execute(&db.pool).await?;
        let (status, body) = h.exchange("code", &code).await?;
        ensure!(status == StatusCode::BAD_REQUEST && body["error"] == "invalid_grant",
            "authorization code accepted a removed redirect registration");
        let consumed: bool = sqlx::query_scalar("SELECT consumed FROM oauth_authorization_codes WHERE code_hash=$1")
            .bind(hash::sha256_hex(&code)).fetch_one(&db.pool).await?;
        ensure!(!consumed && h.tokens(&db.pool).await? == 0,
            "redirect rejection consumed the code or persisted tokens");
        Ok(())
    }.await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL server"]
async fn consent_request_and_authorization_code_insert_commit_together() -> Result<()> {
    let db = database().await?;
    let assertions = async {
        for stage in ["error", "skip"] {
            let h = Harness::create(&db.pool).await?;
            let session: Uuid = sqlx::query_scalar(
                "INSERT INTO platform_sessions(user_id,tenant_id,organization_id,expires_at) VALUES($1,$2,$3,now()+interval '1 hour') RETURNING id",
            ).bind(h.user).bind(h.tenant).bind(h.org).fetch_one(&db.pool).await?;
            let now = Utc::now().timestamp();
            let access = jwt::sign(&jwt::Claims {
                sub: h.user.to_string(), typ: "session".into(), tenant: Some(h.tenant.to_string()),
                org: Some(h.org.to_string()), cid: None, scope: None,
                sid: Some(session.to_string()), ver: Some(0), aud: None, jti: None, iat: now, exp: now+3600,
            }, SECRET)?;
            let request_id: Uuid = sqlx::query_scalar(
                "INSERT INTO oauth_authorization_requests(client_id,redirect_uri,scope,expires_at) VALUES($1,$2,'profile',now()+interval '5 minutes') RETURNING id",
            ).bind(&h.client).bind(REDIRECT).fetch_one(&db.pool).await?;
            let body = if stage == "error" {
                "RAISE EXCEPTION 'synthetic authorization code insert failure';"
            } else {
                "RETURN NULL;"
            };
            sqlx::raw_sql(&format!(
                "CREATE OR REPLACE FUNCTION phase11_consent_failure() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN {body} RETURN NEW; END; $$; \
                 CREATE TRIGGER phase11_consent_failure BEFORE INSERT ON oauth_authorization_codes FOR EACH ROW EXECUTE FUNCTION phase11_consent_failure();"
            )).execute(&db.pool).await?;
            let status = approve_consent(&h, &access, request_id).await?;
            sqlx::query("DROP TRIGGER phase11_consent_failure ON oauth_authorization_codes")
                .execute(&db.pool).await?;
            ensure!(status == StatusCode::INTERNAL_SERVER_ERROR, "consent {stage} did not propagate code insert failure");
            let consumed: bool = sqlx::query_scalar("SELECT consumed FROM oauth_authorization_requests WHERE id=$1")
                .bind(request_id).fetch_one(&db.pool).await?;
            let codes: i64 = sqlx::query_scalar("SELECT count(*) FROM oauth_authorization_codes WHERE client_id=$1")
                .bind(&h.client).fetch_one(&db.pool).await?;
            ensure!(!consumed && codes == 0, "consent {stage} burned request or persisted an orphan code");
            ensure!(approve_consent(&h, &access, request_id).await? == StatusCode::OK,
                "consent request cannot be retried after code insert failure");
            ensure!(approve_consent(&h, &access, request_id).await? == StatusCode::NOT_FOUND,
                "consent request was consumed twice");
            let codes: i64 = sqlx::query_scalar("SELECT count(*) FROM oauth_authorization_codes WHERE client_id=$1")
                .bind(&h.client).fetch_one(&db.pool).await?;
            ensure!(codes == 1, "consent recovery did not persist exactly one code");
        }
        Ok(())
    }.await;
    db.finish(assertions).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicitly configured disposable PostgreSQL server"]
async fn password_change_during_user_lock_wait_rejects_preverified_old_password() -> Result<()> {
    let db = database().await?;
    let assertions = async {
        let h = Harness::create(&db.pool).await?;
        let old_hash = crate::util::password::hash_password("phase11-old-password")?;
        let new_hash = crate::util::password::hash_password("phase11-new-password")?;
        sqlx::query("UPDATE users SET password_hash=$2 WHERE id=$1")
            .bind(h.user)
            .bind(old_hash)
            .execute(&db.pool)
            .await?;
        let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id=$1")
            .bind(h.user)
            .fetch_one(&db.pool)
            .await?;
        let mut blocker = db.pool.begin().await?;
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await?;
        sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
            .bind(h.user)
            .execute(&mut *blocker)
            .await?;
        let (app, client) = (h.app.clone(), h.client.clone());
        let pending = tokio::spawn(async move {
            request(
                &app,
                Method::POST,
                "/oauth/token",
                None,
                &[
                    ("client_id", &client),
                    ("client_secret", CLIENT_SECRET),
                    ("grant_type", "password"),
                    ("username", &username),
                    ("password", "phase11-old-password"),
                ],
            )
            .await
        });
        // Observe the real blocked SQL statement: verification has finished, but
        // issuance cannot acquire its user-first lock until the password commits.
        let waiting = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let observed: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
                     AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)) \
                     AND query LIKE '%FROM users%' AND query LIKE '%FOR UPDATE%')",
                ).bind(blocker_pid).fetch_one(&db.pool).await?;
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
                "password request never reached the observed user-lock barrier"
            ));
        }
        sqlx::query(
            "UPDATE users SET password_hash=$2,session_version=session_version+1 WHERE id=$1",
        )
        .bind(h.user)
        .bind(new_hash)
        .execute(&mut *blocker)
        .await?;
        blocker.commit().await?;
        let (status, body) = pending.await??;
        ensure!(
            status == StatusCode::BAD_REQUEST && body["error"] == "invalid_grant",
            "preverified old password issued a token after the locked password changed"
        );
        ensure!(
            h.tokens(&db.pool).await? == 0,
            "password race persisted tokens"
        );
        Ok(())
    }
    .await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL server"]
async fn password_grant_selects_an_active_membership_after_an_inactive_one() -> Result<()> {
    let db = database().await?;
    let assertions = async {
        let password = "Synthetic-membership-password-8H!";
        let password_hash = crate::util::password::hash_password(password)?;
        for state in ["organization_disabled", "ancestor_expired", "tenant_disabled", "tenant_expired"] {
            let h = Harness::create(&db.pool).await?;
            let username: String = sqlx::query_scalar("UPDATE users SET password_hash=$2 WHERE id=$1 RETURNING username")
                .bind(h.user).bind(&password_hash).fetch_one(&db.pool).await?;
            sqlx::query("UPDATE memberships SET created_at=now()-interval '1 day' WHERE user_id=$1")
                .bind(h.user).execute(&db.pool).await?;
            let fallback_tenant: Uuid = sqlx::query_scalar(
                "INSERT INTO tenants(name,code) VALUES('Active password context',$1) RETURNING id",
            ).bind(format!("active-{}", Uuid::new_v4().simple())).fetch_one(&db.pool).await?;
            let fallback_org: Uuid = sqlx::query_scalar(
                "INSERT INTO organizations(tenant_id,name) VALUES($1,'Active password organization') RETURNING id",
            ).bind(fallback_tenant).fetch_one(&db.pool).await?;
            sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)")
                .bind(h.user).bind(fallback_org).execute(&db.pool).await?;
            let (statement, id) = match state {
                "organization_disabled" => ("UPDATE organizations SET status='DISABLED' WHERE id=$1", h.org),
                "ancestor_expired" => ("UPDATE organizations SET expires_at=now()-interval '1 hour' WHERE id=$1", h.parent),
                "tenant_disabled" => ("UPDATE tenants SET status='DISABLED' WHERE id=$1", h.tenant),
                _ => ("UPDATE tenants SET expires_at=now()-interval '1 hour' WHERE id=$1", h.tenant),
            };
            sqlx::query(statement).bind(id).execute(&db.pool).await?;
            let fields = [("grant_type", "password"), ("username", username.as_str()), ("password", password)];
            let (status, pair) = h.post("/oauth/token", &fields).await?;
            ensure!(status == StatusCode::OK, "inactive first membership blocked valid password context: {state}");
            let claims = jwt::verify(field(&pair, "access_token")?, SECRET)?;
            ensure!(claims.tenant.as_deref() == Some(fallback_tenant.to_string().as_str())
                && claims.org.as_deref() == Some(fallback_org.to_string().as_str()),
                "password grant selected an inactive context: {state}");
            sqlx::query("UPDATE organizations SET status='DISABLED' WHERE id=$1")
                .bind(fallback_org).execute(&db.pool).await?;
            let (status, body) = h.post("/oauth/token", &fields).await?;
            ensure!(status == StatusCode::BAD_REQUEST && body["error"] == "invalid_grant",
                "password grant accepted an account without active memberships");
        }
        Ok(())
    }.await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL server"]
async fn pre_migration_plaintext_and_no_jti_tokens_upgrade_without_revocation() -> Result<()> {
    let db = database().await?;
    let assertions = async {
        let name = format!("oauth_legacy_upgrade_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&db.pool).await?;
        let options = PgConnectOptions::from_str(&std::env::var("TEST_DATABASE_URL")?)
            .map_err(|_| anyhow!("invalid isolated PostgreSQL test configuration"))?
            .database(&name);
        let pool = PgPoolOptions::new().max_connections(3).connect_with(options).await?;
        let upgrade: Result<()> = async {
            // Build the actual pre-0003 schema, then insert old credentials before
            // running the unchanged migration under test.
            sqlx::raw_sql(include_str!("../migrations/0001_init.sql")).execute(&pool).await?;
            sqlx::raw_sql(include_str!("../migrations/0002_release_artifacts.sql")).execute(&pool).await?;
            let h = Harness::create(&pool).await?;
            // A historical PUBLIC client must keep compatibility after the
            // opt-in PKCE migration, independently of managed creation defaults.
            let public_client = format!("legacy-public-{}", Uuid::new_v4().simple());
            sqlx::query("INSERT INTO oauth_clients(client_id,client_name,client_type,grant_types,scopes,redirect_uris) VALUES($1,'Historical public client','PUBLIC','authorization_code,refresh_token','profile',$2)")
                .bind(&public_client).bind(REDIRECT).execute(&pool).await?;
            let now = Utc::now().timestamp();
            let legacy = jwt::sign(&jwt::Claims {
                sub: h.user.to_string(), typ: "access".into(), tenant: Some(h.tenant.to_string()),
                org: Some(h.org.to_string()), cid: Some(h.client.clone()), scope: Some("profile email".into()),
                sid: None, ver: None, aud: None, jti: None, iat: now, exp: now+3600,
            }, SECRET)?;
            sqlx::query("INSERT INTO oauth_tokens(token_hash,token_type,jwt,user_id,client_id,tenant_id,organization_id,scope,expires_at) VALUES($1,'access',$2,$3,$4,$5,$6,'profile email',now()+interval '1 hour')")
                .bind(hash::sha256_hex(&legacy)).bind(&legacy).bind(h.user).bind(&h.client).bind(h.tenant).bind(h.org).execute(&pool).await?;
            let refresh = h.seed_refresh(&pool).await?;
            sqlx::raw_sql(include_str!("../migrations/0003_oauth_issuance.sql")).execute(&pool).await?;
            // The current route adapter reads the additive client policy column;
            // this does not alter the 0003 token upgrade under test.
            sqlx::raw_sql(include_str!("../migrations/0007_oauth_pkce_policy.sql")).execute(&pool).await?;
            let public_requires_pkce: bool = sqlx::query_scalar("SELECT require_pkce FROM oauth_clients WHERE client_id=$1")
                .bind(&public_client).fetch_one(&pool).await?;
            ensure!(!public_requires_pkce, "PKCE migration forced a historical PUBLIC client into strict mode");
            let versions: Vec<i16> = sqlx::query_scalar("SELECT storage_version FROM oauth_tokens")
                .fetch_all(&pool).await?;
            ensure!(versions == vec![1,1], "migration did not mark both existing credentials as legacy");
            h.assert_identity(&legacy, true).await?;
            let (status, pair) = h.refresh(&refresh).await?;
            ensure!(status == StatusCode::OK, "legacy refresh cannot rotate after migration");
            let new_version: i16 = sqlx::query_scalar("SELECT storage_version FROM oauth_tokens WHERE token_hash=$1")
                .bind(hash::sha256_hex(field(&pair,"access_token")?)).fetch_one(&pool).await?;
            ensure!(new_version == 2, "new issuance retained legacy storage semantics");
            ensure!(h.post("/oauth/revoke", &[("token",&legacy)]).await?.0 == StatusCode::OK,
                "migration check constraint prevented legacy plaintext row revocation");
            h.assert_identity(&legacy, false).await?;
            let retained: (Option<String>, bool, i16) = sqlx::query_as("SELECT jwt,revoked,storage_version FROM oauth_tokens WHERE token_hash=$1")
                .bind(hash::sha256_hex(&legacy)).fetch_one(&pool).await?;
            ensure!(retained == (Some(legacy),true,1), "migration deleted or rewrote historical token material");
            let invalid = sqlx::query("INSERT INTO oauth_tokens(token_hash,token_type,jwt,client_id,expires_at) VALUES($1,'access','new plaintext forbidden',$2,now()+interval '1 hour')")
                .bind(hash::sha256_hex("synthetic-invalid-new-token")).bind(&h.client).execute(&pool).await;
            ensure!(matches!(invalid, Err(ref error) if error.as_database_error().and_then(|e| e.code()).as_deref()==Some("23514")),
                "new ordinary OAuth JWT plaintext was accepted by the database");
            Ok(())
        }.await;
        pool.close().await;
        // name is generated locally from a fixed prefix and UUID, never an input.
        let cleanup = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
            .execute(&db.pool).await;
        upgrade?;
        cleanup?;
        Ok(())
    }.await;
    db.finish(assertions).await
}
