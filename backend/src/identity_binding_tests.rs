//! Phase 2 contracts run only against explicitly selected disposable databases.
//! No .env, preview database, historical credential import or implicit binding repair.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use axum::{middleware, routing::post, Router};
use serde_json::{json, Value};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::config::Config;
use crate::state::AppState;
use crate::util::{hash, jwt, uuid7::new_uuid};

const SECRET: &str = "identity-phase02-owned-fixture-signing-key";
const ISSUER: &str = "urn:saas:legacy-platform";
const REDIRECT: &str = "https://phase02.example.invalid/callback";
const VERIFIER: &str = "phase02-owned-S256-verifier-0123456789-abcdefghijklmnopqrstuvwxyz";
const PASSWORD: &str = "Owned-Phase02-Fixture-Password9!";

struct Database {
    pool: PgPool,
    admin: PgPool,
    options: PgConnectOptions,
    name: String,
}

impl Database {
    async fn create(version: i64) -> Result<Self> {
        let url = std::env::var("IDENTITY_PHASE02_TEST_DATABASE_URL")
            .map_err(|_| anyhow!("explicit IDENTITY_PHASE02_TEST_DATABASE_URL is required"))?;
        let options = PgConnectOptions::from_str(&url)
            .map_err(|_| anyhow!("invalid isolated Phase 2 database configuration"))?;
        ensure!(
            options.get_database() == Some("postgres"),
            "Phase 2 fixture administration must explicitly select postgres"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("isolated Phase 2 administration connection failed"))?;
        let name = format!("identity_phase02_{}", new_uuid().simple());
        ensure!(name.len() <= 63, "owned database name is too long");
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await?;
        let pool = match PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone().database(&name))
            .await
        {
            Ok(pool) => pool,
            Err(_) => {
                let _ = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
                    .execute(&admin)
                    .await;
                return Err(anyhow!("owned Phase 2 database connection failed"));
            }
        };
        let database = Self {
            pool,
            admin,
            options,
            name,
        };
        if let Err(error) = database.migrate(version).await {
            let _ = database.finish(Ok(())).await;
            return Err(error);
        }
        Ok(database)
    }

    async fn migrate(&self, version: i64) -> Result<()> {
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(
                sqlx::migrate!("./migrations")
                    .iter()
                    .filter(|migration| migration.version <= version)
                    .cloned()
                    .collect(),
            ),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&self.pool)
        .await?;
        Ok(())
    }

    async fn independent_pool(&self) -> Result<PgPool> {
        Ok(PgPoolOptions::new()
            .max_connections(2)
            .connect_with(self.options.clone().database(&self.name))
            .await?)
    }

    async fn finish(self, result: Result<()>) -> Result<()> {
        ensure!(
            self.name.starts_with("identity_phase02_")
                && self.name.len() <= 63
                && self
                    .name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_'),
            "refusing cleanup outside an owned Phase 2 database"
        );
        let closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let removed = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await;
        self.admin.close().await;
        eprintln!(
            "identity_phase02_cleanup database={} assertions_ok={} database_removed={} pool_closed={closed}",
            self.name,
            result.is_ok(),
            removed.is_ok()
        );
        result?;
        removed?;
        ensure!(closed, "owned Phase 2 pool did not close");
        Ok(())
    }
}

fn app(pool: &PgPool, probe: Option<Arc<AtomicUsize>>) -> Result<Router> {
    let config = Config::from_values(&HashMap::from([
        ("APP_ENV".into(), "test".into()),
        (
            "DATABASE_URL".into(),
            "postgres://unused/identity-phase02".into(),
        ),
        ("JWT_SECRET".into(), SECRET.into()),
        ("MEMBERSHIP_LIFECYCLE_ENABLED".into(), "true".into()),
        ("CUSTOMER_MEMBERSHIPS_ENABLED".into(), "true".into()),
        ("DUAL_MEMBERSHIPS_ENABLED".into(), "true".into()),
    ]))?;
    let state = AppState::new(pool.clone(), Arc::new(config));
    let mut routes =
        crate::routes::api_router().nest("/oauth", crate::routes::oauth2::oauth_router());
    if let Some(probe) = probe {
        routes = routes.route(
            "/identity-phase02/protected-write",
            post(move |_actor: crate::extractors::CurrentUser| {
                let probe = Arc::clone(&probe);
                async move {
                    probe.fetch_add(1, Ordering::SeqCst);
                    StatusCode::NO_CONTENT
                }
            }),
        );
    }
    Ok(routes
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::observability::observe,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::normalize_client_ip,
        ))
        .with_state(state))
}

struct Reply {
    status: StatusCode,
    body: Value,
}

async fn request(
    app: &Router,
    method: Method,
    path: &str,
    bearer: Option<&str>,
    content_type: &str,
    body: String,
) -> Result<Reply> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, content_type)
        .header("x-request-id", new_uuid().to_string())
        .extension(ConnectInfo("192.0.2.32:30032".parse::<SocketAddr>()?));
    if let Some(token) = bearer {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = tokio::time::timeout(
        Duration::from_secs(25),
        app.clone().oneshot(request.body(Body::from(body))?),
    )
    .await
    .map_err(|_| anyhow!("Phase 2 owned HTTP request timed out: {path}"))??;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok(Reply { status, body })
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
        "application/json",
        body.to_string(),
    )
    .await
}

async fn form(app: &Router, fields: &[(&str, &str)]) -> Result<Reply> {
    let encoded = fields
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                percent_encoding::utf8_percent_encode(key, percent_encoding::NON_ALPHANUMERIC),
                percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC)
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    request(
        app,
        Method::POST,
        "/oauth/token",
        None,
        "application/x-www-form-urlencoded",
        encoded,
    )
    .await
}

fn expect(reply: &Reply, expected: StatusCode, operation: &str) -> Result<()> {
    // Never include successful token-bearing bodies in failures or evidence.
    ensure!(
        reply.status == expected,
        "{operation}: expected {expected}, received {} (error={})",
        reply.status,
        reply
            .body
            .get("error")
            .or_else(|| reply.body.get("code"))
            .unwrap_or(&Value::Null)
    );
    Ok(())
}

async fn user(pool: &PgPool, label: &str, admin: bool) -> Result<Uuid> {
    let id = new_uuid();
    sqlx::query("INSERT INTO users(id,username,password_hash,is_super_admin) VALUES($1,$2,'phase02-unusable-fixture-password',$3)")
        .bind(id).bind(label).bind(admin).execute(pool).await?;
    Ok(id)
}

async fn tenant(pool: &PgPool, label: &str) -> Result<(Uuid, Uuid)> {
    let tenant = new_uuid();
    sqlx::query("INSERT INTO tenants(id,name,code) VALUES($1,$2,$2)")
        .bind(tenant)
        .bind(label)
        .execute(pool)
        .await?;
    let org = organization(pool, tenant, None, label).await?;
    Ok((tenant, org))
}

async fn organization(
    pool: &PgPool,
    tenant: Uuid,
    parent: Option<Uuid>,
    label: &str,
) -> Result<Uuid> {
    let id = new_uuid();
    sqlx::query("INSERT INTO organizations(id,tenant_id,parent_id,name) VALUES($1,$2,$3,$4)")
        .bind(id)
        .bind(tenant)
        .bind(parent)
        .bind(label)
        .execute(pool)
        .await?;
    Ok(id)
}

async fn client(
    pool: &PgPool,
    label: &str,
    kinds: &[&str],
    application: Option<Uuid>,
) -> Result<()> {
    sqlx::query("INSERT INTO oauth_clients(id,client_id,client_name,client_type,redirect_uris,grant_types,scopes,require_pkce,subject_mode,allowed_member_types,application_id) \
        VALUES($1,$2,$2,'PUBLIC',$3,'authorization_code,refresh_token,urn:ietf:params:oauth:grant-type:device_code','profile',true,'USER',$4,$5)")
        .bind(new_uuid()).bind(label).bind(REDIRECT).bind(kinds).bind(application).execute(pool).await?;
    Ok(())
}

#[derive(Clone)]
struct Identity {
    user: Uuid,
    tenant: Uuid,
    org: Uuid,
    member: Uuid,
    identity_version: i64,
    kind: &'static str,
    client: &'static str,
}

impl Identity {
    async fn create(
        pool: &PgPool,
        user: Uuid,
        tenant: Uuid,
        org: Uuid,
        kind: &'static str,
        client: &'static str,
    ) -> Result<Self> {
        let member = new_uuid();
        sqlx::query(
            "INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES($1,$2,$3,$4)",
        )
        .bind(member)
        .bind(user)
        .bind(org)
        .bind(kind)
        .execute(pool)
        .await?;
        Ok(Self {
            user,
            tenant,
            org,
            member,
            identity_version: 1,
            kind,
            client,
        })
    }

    async fn active(&self, pool: &PgPool) -> Result<bool> {
        Ok(
            sqlx::query_scalar("SELECT identity_binding_active($1,$2,$3,0,$4,$5,$6)")
                .bind(self.user)
                .bind(self.tenant)
                .bind(self.org)
                .bind(self.member)
                .bind(self.kind)
                .bind(self.identity_version)
                .fetch_one(pool)
                .await?,
        )
    }
}

struct Credentials {
    sid: Uuid,
    session: String,
    refresh: String,
    access: String,
    oauth_refresh: String,
    code: String,
    device: String,
    access_id: Uuid,
    code_id: Uuid,
    device_id: Uuid,
}

struct BoundSession {
    sid: Uuid,
    session: String,
    refresh: String,
}

fn signed_session(identity: &Identity, sid: Uuid, exp: i64) -> Result<String> {
    let now = chrono::Utc::now().timestamp();
    jwt::sign(
        &jwt::Claims {
            sub: identity.user.to_string(),
            typ: "session".into(),
            tenant: Some(identity.tenant.to_string()),
            org: Some(identity.org.to_string()),
            cid: None,
            scope: None,
            sid: Some(sid.to_string()),
            ver: Some(0),
            aud: None,
            jti: None,
            iat: now,
            exp,
        },
        SECRET,
    )
}

async fn bound_session(pool: &PgPool, identity: &Identity) -> Result<BoundSession> {
    let sid = new_uuid();
    sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version) \
        VALUES($1,$2,$3,$4,now()+interval '1 hour',0,$5,$6,$9,$7,$8,1)")
        .bind(sid).bind(identity.user).bind(identity.tenant).bind(identity.org).bind(identity.member)
        .bind(identity.kind).bind(ISSUER).bind(identity.client).bind(identity.identity_version).execute(pool).await?;
    let mut tx = pool.begin().await?;
    let refresh = crate::session::issue_refresh_in_tx(
        &mut tx,
        sid,
        0,
        chrono::Utc::now() + chrono::Duration::hours(1),
    )
    .await?;
    tx.commit().await?;
    let session = signed_session(identity, sid, chrono::Utc::now().timestamp() + 3600)?;
    Ok(BoundSession {
        sid,
        session,
        refresh,
    })
}

async fn credentials(
    pool: &PgPool,
    identity: &Identity,
    application: Option<Uuid>,
) -> Result<Credentials> {
    let BoundSession {
        sid,
        session,
        refresh,
    } = bound_session(pool, identity).await?;
    let access_id = new_uuid();
    let jti = new_uuid();
    let now = chrono::Utc::now().timestamp();
    let access = jwt::sign(
        &jwt::Claims {
            sub: identity.user.to_string(),
            typ: "access".into(),
            tenant: Some(identity.tenant.to_string()),
            org: Some(identity.org.to_string()),
            cid: Some(identity.client.into()),
            scope: Some("profile".into()),
            sid: None,
            ver: Some(0),
            aud: None,
            jti: Some(jti.to_string()),
            iat: now,
            exp: now + 3600,
        },
        SECRET,
    )?;
    let oauth_refresh = hash::random_token(43);
    for (id, raw, token_type, jti) in [
        (access_id, access.as_str(), "access", Some(jti)),
        (new_uuid(), oauth_refresh.as_str(), "refresh", None),
    ] {
        sqlx::query("INSERT INTO oauth_tokens(id,token_hash,token_type,user_id,client_id,tenant_id,organization_id,scope,expires_at,token_use,jti,storage_version,account_version,membership_id,member_type,identity_version,client_policy_version,application_id,application_policy_version,origin_issuer,origin_client_ref,origin_client_policy_version) \
            VALUES($1,$2,$3,$4,$5,$6,$7,'profile',now()+interval '1 hour','oauth',$8,2,0,$9,$10,$14,1,$11,$12,$13,$5,1)")
            .bind(id).bind(hash::sha256_hex(raw)).bind(token_type).bind(identity.user).bind(identity.client)
            .bind(identity.tenant).bind(identity.org).bind(jti).bind(identity.member).bind(identity.kind)
            .bind(application).bind(application.map(|_| 1_i64)).bind(ISSUER).bind(identity.identity_version).execute(pool).await?;
    }
    let code = hash::random_token(48);
    let code_id = new_uuid();
    sqlx::query("INSERT INTO oauth_authorization_codes(id,code_hash,client_id,user_id,tenant_id,organization_id,redirect_uri,scope,code_challenge,code_challenge_method,expires_at,account_version,membership_id,member_type,identity_version,client_policy_version,application_id,application_policy_version) \
        VALUES($1,$2,$3,$4,$5,$6,$7,'profile',$8,'S256',now()+interval '5 minutes',0,$9,$10,$13,1,$11,$12)")
        .bind(code_id).bind(hash::sha256_hex(&code)).bind(identity.client).bind(identity.user)
        .bind(identity.tenant).bind(identity.org).bind(REDIRECT).bind(hash::b64url_sha256(VERIFIER))
        .bind(identity.member).bind(identity.kind).bind(application).bind(application.map(|_| 1_i64)).bind(identity.identity_version).execute(pool).await?;
    let device = hash::random_token(48);
    let device_id = new_uuid();
    sqlx::query("INSERT INTO oauth_device_codes(id,device_code_hash,user_code,client_id,scope,status,user_id,tenant_id,organization_id,expires_at,account_version,membership_id,member_type,identity_version,client_policy_version,application_id,application_policy_version) \
        VALUES($1,$2,$3,$4,'profile','APPROVED',$5,$6,$7,now()+interval '5 minutes',0,$8,$9,$12,1,$10,$11)")
        .bind(device_id).bind(hash::sha256_hex(&device)).bind(hash::random_user_code()).bind(identity.client)
        .bind(identity.user).bind(identity.tenant).bind(identity.org).bind(identity.member).bind(identity.kind)
        .bind(application).bind(application.map(|_| 1_i64)).bind(identity.identity_version).execute(pool).await?;
    Ok(Credentials {
        sid,
        session,
        refresh,
        access,
        oauth_refresh,
        code,
        device,
        access_id,
        code_id,
        device_id,
    })
}

async fn session_active(pool: &PgPool, sid: Uuid) -> Result<bool> {
    Ok(sqlx::query_scalar("SELECT platform_session_active($1)")
        .bind(sid)
        .fetch_one(pool)
        .await?)
}

async fn userinfo(app: &Router, access: &str) -> Result<Reply> {
    json_request(
        app,
        Method::GET,
        "/oauth/userinfo",
        Some(access),
        Value::Null,
    )
    .await
}

async fn exchange_code(
    app: &Router,
    identity: &Identity,
    credentials: &Credentials,
) -> Result<Reply> {
    form(
        app,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", identity.client),
            ("code", &credentials.code),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
        ],
    )
    .await
}

async fn exchange_device(
    app: &Router,
    identity: &Identity,
    credentials: &Credentials,
) -> Result<Reply> {
    form(
        app,
        &[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", identity.client),
            ("device_code", &credentials.device),
        ],
    )
    .await
}

async fn admin_identity(
    pool: &PgPool,
    tenant: Uuid,
    org: Uuid,
) -> Result<(Identity, BoundSession)> {
    let actor = user(pool, "phase02-manager", true).await?;
    let actor = Identity::create(
        pool,
        actor,
        tenant,
        org,
        "EMPLOYEE",
        "platform-console-legacy",
    )
    .await?;
    // Administrative permission is explicitly registered even for a super admin.
    let application = new_uuid();
    let menu = new_uuid();
    sqlx::query("INSERT INTO applications(id,name,code,base_path) VALUES($1,'Phase 2 management','system','/micro/system')")
        .bind(application).execute(pool).await?;
    sqlx::query("INSERT INTO menus(id,application_id,title,permission,path) VALUES($1,$2,'Phase 2 identity','system:user','/system/users')")
        .bind(menu).bind(application).execute(pool).await?;
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
    // Only the actor session is required; avoid issuing unsupported grants to the
    // reserved console registration when exercising unrelated lifecycle writes.
    let credentials = bound_session(pool, &actor).await?;
    Ok((actor, credentials))
}

async fn check_retired(pool: &PgPool, credentials: &Credentials, retired: bool) -> Result<()> {
    let state: (bool, bool, bool, bool, bool) = sqlx::query_as(
        "SELECT s.revoked_at IS NOT NULL, r.consumed_at IS NOT NULL, tok.revoked, code.consumed, device.status='EXPIRED' \
         FROM platform_sessions s JOIN platform_session_refresh_tokens r ON r.session_id=s.id \
         JOIN oauth_tokens tok ON tok.id=$2 JOIN oauth_authorization_codes code ON code.id=$3 \
         JOIN oauth_device_codes device ON device.id=$4 WHERE s.id=$1")
        .bind(credentials.sid).bind(credentials.access_id).bind(credentials.code_id).bind(credentials.device_id)
        .fetch_one(pool).await?;
    ensure!(
        state == (retired, retired, retired, retired, retired),
        "credential retirement crossed a membership/client/application boundary: {state:?}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "creates and drops identity_phase02_* databases using explicit IDENTITY_PHASE02_TEST_DATABASE_URL"]
async fn identity_binding_phase02_legacy_rows_require_reauthentication() -> Result<()> {
    let db = Database::create(13).await?;
    let result = async {
        let (tenant, org) = tenant(&db.pool, "phase02-upgrade").await?;
        client(&db.pool, "phase02-upgrade-client", &["EMPLOYEE"], None).await?;
        let subject = user(&db.pool, "phase02-upgrade-user", false).await?;
        let identity = Identity::create(&db.pool, subject, tenant, org, "EMPLOYEE", "phase02-upgrade-client").await?;
        let sid = new_uuid();
        sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at) VALUES($1,$2,$3,$4,now()+interval '1 hour')")
            .bind(sid).bind(subject).bind(tenant).bind(org).execute(&db.pool).await?;
        let raw = hash::random_token(43);
        sqlx::query("INSERT INTO oauth_tokens(id,token_hash,token_type,user_id,client_id,tenant_id,organization_id,expires_at,storage_version) VALUES($1,$2,'refresh',$3,$4,$5,$6,now()+interval '1 hour',2)")
            .bind(new_uuid()).bind(hash::sha256_hex(&raw)).bind(subject).bind(identity.client).bind(tenant).bind(org).execute(&db.pool).await?;
        let now=chrono::Utc::now().timestamp();
        let old_access=jwt::sign(&jwt::Claims {
            sub:subject.to_string(),typ:"access".into(),tenant:Some(tenant.to_string()),org:Some(org.to_string()),
            cid:Some(identity.client.into()),scope:Some("profile".into()),sid:None,ver:Some(0),aud:None,
            jti:Some(new_uuid().to_string()),iat:now,exp:now+3600,
        },SECRET)?;
        sqlx::query("INSERT INTO oauth_tokens(id,token_hash,token_type,user_id,client_id,tenant_id,organization_id,expires_at,storage_version) VALUES($1,$2,'access',$3,$4,$5,$6,now()+interval '1 hour',2)")
            .bind(new_uuid()).bind(hash::sha256_hex(&old_access)).bind(subject).bind(identity.client).bind(tenant).bind(org).execute(&db.pool).await?;
        let code = hash::random_token(48);
        let code_id = new_uuid();
        sqlx::query("INSERT INTO oauth_authorization_codes(id,code_hash,client_id,user_id,tenant_id,organization_id,redirect_uri,scope,code_challenge,code_challenge_method,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,'profile',$8,'S256',now()+interval '5 minutes')")
            .bind(code_id).bind(hash::sha256_hex(&code)).bind(identity.client).bind(subject).bind(tenant).bind(org)
            .bind(REDIRECT).bind(hash::b64url_sha256(VERIFIER)).execute(&db.pool).await?;
        let device = hash::random_token(48);
        let device_id = new_uuid();
        sqlx::query("INSERT INTO oauth_device_codes(id,device_code_hash,user_code,client_id,scope,status,user_id,tenant_id,organization_id,expires_at) VALUES($1,$2,$3,$4,'profile','APPROVED',$5,$6,$7,now()+interval '5 minutes')")
            .bind(device_id).bind(hash::sha256_hex(&device)).bind(hash::random_user_code()).bind(identity.client)
            .bind(subject).bind(tenant).bind(org).execute(&db.pool).await?;
        db.migrate(14).await?;
        ensure!(!session_active(&db.pool, sid).await?, "unbound upgrade session became active");
        let retained: (bool, bool, bool) = sqlx::query_as("SELECT membership_id IS NULL,account_version IS NULL,origin_client_ref IS NULL FROM platform_sessions WHERE id=$1")
            .bind(sid).fetch_one(&db.pool).await?;
        ensure!(retained == (true,true,true), "migration fabricated old session binding");
        let unbound: bool = sqlx::query_scalar("SELECT membership_id IS NULL AND account_version IS NULL FROM oauth_tokens WHERE token_hash=$1")
            .bind(hash::sha256_hex(&raw)).fetch_one(&db.pool).await?;
        ensure!(unbound, "migration fabricated an old OAuth binding");
        // Independently prove reader rejection of missing proof. These owned
        // malformed-row fixtures clear only terminal flags, never fill binding.
        sqlx::query("UPDATE platform_sessions SET revoked_at=NULL WHERE id=$1").bind(sid).execute(&db.pool).await?;
        sqlx::query("UPDATE oauth_tokens SET revoked=false").execute(&db.pool).await?;
        sqlx::query("UPDATE oauth_authorization_codes SET consumed=false WHERE id=$1").bind(code_id).execute(&db.pool).await?;
        sqlx::query("UPDATE oauth_device_codes SET status='APPROVED' WHERE id=$1").bind(device_id).execute(&db.pool).await?;
        ensure!(!session_active(&db.pool,sid).await?,"old session was accepted after terminal flag reset");
        let app = app(&db.pool, None)?;
        let token = signed_session(&identity, sid, chrono::Utc::now().timestamp()+3600)?;
        expect(&json_request(&app, Method::GET, "/api/auth/profile", Some(&token), Value::Null).await?, StatusCode::UNAUTHORIZED, "old session after upgrade")?;
        expect(&userinfo(&app,&old_access).await?,StatusCode::UNAUTHORIZED,"unbound legacy OAuth access")?;
        let reply = form(&app, &[("grant_type","refresh_token"),("client_id",identity.client),("refresh_token",&raw)]).await?;
        expect(&reply, StatusCode::BAD_REQUEST, "unbound OAuth refresh after upgrade")?;
        ensure!(reply.body["error"] == "invalid_grant", "old refresh did not request reauthentication");
        expect(&form(&app,&[("grant_type","authorization_code"),("client_id",identity.client),("code",&code),("redirect_uri",REDIRECT),("code_verifier",VERIFIER)]).await?,StatusCode::BAD_REQUEST,"unbound legacy authorization code")?;
        expect(&form(&app,&[("grant_type","urn:ietf:params:oauth:grant-type:device_code"),("client_id",identity.client),("device_code",&device)]).await?,StatusCode::BAD_REQUEST,"unbound legacy approved device")?;
        let unchanged:bool=sqlx::query_scalar("SELECT membership_id IS NULL AND account_version IS NULL FROM oauth_authorization_codes WHERE id=$1")
            .bind(code_id).fetch_one(&db.pool).await?;
        ensure!(unchanged,"rejected authorization code was rebound from current state");
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops identity_phase02_* databases using explicit IDENTITY_PHASE02_TEST_DATABASE_URL"]
async fn identity_binding_phase02_exact_member_revocation() -> Result<()> {
    let db = Database::create(i64::MAX).await?;
    let result = async {
        // Target-schema fixture only: the independently published contraction
        // migration remains a Phase 6 delivery.
        sqlx::query("ALTER TABLE memberships DROP CONSTRAINT memberships_user_id_organization_id_key").execute(&db.pool).await?;
        let (tenant, org) = tenant(&db.pool, "phase02-local-revoke").await?;
        let other_org = organization(&db.pool, tenant, Some(org), "phase02-other-org").await?;
        client(&db.pool, "phase02-employee", &["EMPLOYEE"], None).await?;
        client(&db.pool, "phase02-customer", &["CUSTOMER"], None).await?;
        let subject = user(&db.pool, "phase02-dual-user", false).await?;
        let employee = Identity::create(&db.pool,subject,tenant,org,"EMPLOYEE","phase02-employee").await?;
        let customer = Identity::create(&db.pool,subject,tenant,org,"CUSTOMER","phase02-customer").await?;
        let other = Identity::create(&db.pool,subject,tenant,other_org,"EMPLOYEE","phase02-employee").await?;
        let employee_tokens = credentials(&db.pool,&employee,None).await?;
        let customer_tokens = credentials(&db.pool,&customer,None).await?;
        let other_tokens = credentials(&db.pool,&other,None).await?;
        let (_, actor) = admin_identity(&db.pool,tenant,org).await?;
        let app = app(&db.pool,None)?;
        for (identity,tokens) in [(&employee,&employee_tokens),(&customer,&customer_tokens),(&other,&other_tokens)] {
            ensure!(identity.active(&db.pool).await? && session_active(&db.pool,tokens.sid).await?, "fresh exact binding inactive");
            let reply=userinfo(&app,&tokens.access).await?;
            expect(&reply,StatusCode::OK,"fully bound OAuth userinfo")?;
            if identity.kind=="CUSTOMER" {
                ensure!(reply.body["roles"].as_array().is_none_or(Vec::is_empty),"customer userinfo exposed employee roles");
            }
        }
        expect(&json_request(&app,Method::GET,"/api/auth/profile",Some(&employee_tokens.session),Value::Null).await?,StatusCode::OK,"fully bound employee HTTP session")?;
        for (user,tenant_ref,org_ref,member,kind,account_version,identity_version) in [
            (new_uuid(),tenant,org,employee.member,"EMPLOYEE",0_i64,1_i64),
            (subject,new_uuid(),org,employee.member,"EMPLOYEE",0,1),
            (subject,tenant,other_org,employee.member,"EMPLOYEE",0,1),
            (subject,tenant,org,other.member,"EMPLOYEE",0,1),
            (subject,tenant,org,employee.member,"CUSTOMER",0,1),
            (subject,tenant,org,employee.member,"EMPLOYEE",1,1),
            (subject,tenant,org,employee.member,"EMPLOYEE",0,2),
        ] {
            let active:bool=sqlx::query_scalar("SELECT identity_binding_active($1,$2,$3,$4,$5,$6,$7)")
                .bind(user).bind(tenant_ref).bind(org_ref).bind(account_version).bind(member).bind(kind).bind(identity_version)
                .fetch_one(&db.pool).await?;
            ensure!(!active,"mismatched identity tuple was accepted");
        }
        let reply = json_request(&app,Method::PUT,&format!("/api/memberships/{}/status",employee.member),Some(&actor.session),
            json!({"expectedIdentityVersion":1,"status":"DISABLED","reason":"Owned Phase 2 employee departure fixture"})).await?;
        expect(&reply,StatusCode::OK,"disable only employee membership")?;
        check_retired(&db.pool,&employee_tokens,true).await?;
        check_retired(&db.pool,&customer_tokens,false).await?;
        check_retired(&db.pool,&other_tokens,false).await?;
        ensure!(!session_active(&db.pool,employee_tokens.sid).await? && session_active(&db.pool,customer_tokens.sid).await? && session_active(&db.pool,other_tokens.sid).await?,"local revoke crossed identity boundary");
        expect(&userinfo(&app,&employee_tokens.access).await?,StatusCode::UNAUTHORIZED,"retired employee access")?;
        expect(&json_request(&app,Method::GET,"/api/auth/profile",Some(&employee_tokens.session),Value::Null).await?,StatusCode::UNAUTHORIZED,"retired employee HTTP session")?;
        expect(&exchange_code(&app,&employee,&employee_tokens).await?,StatusCode::BAD_REQUEST,"retired employee code")?;
        expect(&exchange_device(&app,&employee,&employee_tokens).await?,StatusCode::BAD_REQUEST,"retired employee device")?;
        expect(&userinfo(&app,&customer_tokens.access).await?,StatusCode::OK,"customer retained after employee departure")?;
        expect(&exchange_code(&app,&customer,&customer_tokens).await?,StatusCode::OK,"customer code retained")?;
        expect(&exchange_device(&app,&other,&other_tokens).await?,StatusCode::OK,"other organization device retained")?;
        let enabled=json_request(&app,Method::PUT,&format!("/api/memberships/{}/status",employee.member),Some(&actor.session),
            json!({"expectedIdentityVersion":2,"status":"ENABLED","reason":"Owned Phase 2 explicit employee return requires reauthentication"})).await?;
        expect(&enabled,StatusCode::OK,"explicitly enable a new employee generation")?;
        let enabled_version:i64=sqlx::query_scalar("SELECT identity_version FROM memberships WHERE id=$1 AND status='ENABLED'")
            .bind(employee.member).fetch_one(&db.pool).await?;
        ensure!(enabled_version==3,"reenabling did not advance the member generation");
        let new_generation_active:bool=sqlx::query_scalar("SELECT identity_binding_active($1,$2,$3,0,$4,'EMPLOYEE',$5)")
            .bind(subject).bind(tenant).bind(org).bind(employee.member).bind(enabled_version).fetch_one(&db.pool).await?;
        ensure!(new_generation_active,"explicit reenable left the current member unusable");
        check_retired(&db.pool,&employee_tokens,true).await?;
        ensure!(!session_active(&db.pool,employee_tokens.sid).await?,"reenabling resurrected an old employee context");
        expect(&userinfo(&app,&employee_tokens.access).await?,StatusCode::UNAUTHORIZED,"reenable cannot restore old access")?;
        expect(&exchange_code(&app,&employee,&employee_tokens).await?,StatusCode::BAD_REQUEST,"reenable cannot restore old code")?;
        expect(&exchange_device(&app,&employee,&employee_tokens).await?,StatusCode::BAD_REQUEST,"reenable cannot restore old device")?;
        expect(&form(&app,&[("grant_type","refresh_token"),("client_id",employee.client),("refresh_token",&employee_tokens.oauth_refresh)]).await?,StatusCode::BAD_REQUEST,"reenable cannot restore old OAuth refresh")?;
        expect(&form(&app,&[("grant_type","refresh_token"),("refresh_token",&employee_tokens.refresh)]).await?,StatusCode::BAD_REQUEST,"reenable cannot restore old platform refresh")?;
        ensure!(session_active(&db.pool,customer_tokens.sid).await? && session_active(&db.pool,other_tokens.sid).await?,"reenabling employee disturbed another valid identity");
        // These are newly issued fixtures carrying the explicitly read v3
        // generation; no retired row is rebound or repaired after reenable.
        let mut returned_employee=employee.clone();
        returned_employee.identity_version=enabled_version;
        let returned_tokens=credentials(&db.pool,&returned_employee,None).await?;
        ensure!(session_active(&db.pool,returned_tokens.sid).await?,"new employee generation did not receive an active context");
        expect(&userinfo(&app,&returned_tokens.access).await?,StatusCode::OK,"new employee access before customer removal")?;
        let reply = json_request(&app,Method::DELETE,&format!("/api/memberships/{}",customer.member),Some(&actor.session),
            json!({"expectedIdentityVersion":1,"reason":"Owned Phase 2 customer relationship removal"})).await?;
        expect(&reply,StatusCode::OK,"delete customer relationship")?;
        ensure!(!session_active(&db.pool,customer_tokens.sid).await? && session_active(&db.pool,other_tokens.sid).await?,"customer removal revoked another organization");
        expect(&userinfo(&app,&customer_tokens.access).await?,StatusCode::UNAUTHORIZED,"deleted customer access is retired")?;
        check_retired(&db.pool,&returned_tokens,false).await?;
        ensure!(session_active(&db.pool,returned_tokens.sid).await?,"customer removal revoked same-organization current employee session");
        expect(&json_request(&app,Method::GET,"/api/auth/profile",Some(&returned_tokens.session),Value::Null).await?,StatusCode::OK,"current employee HTTP session after customer removal")?;
        expect(&userinfo(&app,&returned_tokens.access).await?,StatusCode::OK,"current employee access after customer removal")?;
        expect(&form(&app,&[("grant_type","refresh_token"),("client_id",returned_employee.client),("refresh_token",&returned_tokens.oauth_refresh)]).await?,StatusCode::OK,"current employee OAuth refresh after customer removal")?;
        expect(&form(&app,&[("grant_type","refresh_token"),("refresh_token",&returned_tokens.refresh)]).await?,StatusCode::OK,"current employee platform refresh after customer removal")?;
        expect(&userinfo(&app,&other_tokens.access).await?,StatusCode::OK,"other organization access after customer removal")?;
        expect(&userinfo(&app,&employee_tokens.access).await?,StatusCode::UNAUTHORIZED,"old employee generation remains rejected after customer removal")?;
        ensure!(!session_active(&db.pool,employee_tokens.sid).await?,"customer removal resurrected retired employee generation");
        let retained: Uuid = sqlx::query_scalar("SELECT membership_id FROM platform_sessions WHERE id=$1").bind(customer_tokens.sid).fetch_one(&db.pool).await?;
        ensure!(retained==customer.member,"deletion erased the credential tombstone binding");
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops identity_phase02_* databases using explicit IDENTITY_PHASE02_TEST_DATABASE_URL"]
async fn identity_binding_phase02_account_and_policy_versions() -> Result<()> {
    let db = Database::create(i64::MAX).await?;
    let result = async {
        let (tenant,org)=tenant(&db.pool,"phase02-policy").await?;
        let subject=user(&db.pool,"phase02-policy-user",false).await?;
        client(&db.pool,"phase02-policy-source",&["EMPLOYEE","CUSTOMER"],None).await?;
        client(&db.pool,"phase02-policy-other",&["EMPLOYEE"],None).await?;
        let identity=Identity::create(&db.pool,subject,tenant,org,"EMPLOYEE","phase02-policy-source").await?;
        let mut other=identity.clone();
        other.client="phase02-policy-other";
        let first=credentials(&db.pool,&identity,None).await?;
        let second=credentials(&db.pool,&other,None).await?;
        sqlx::query("UPDATE oauth_clients SET allowed_member_types=ARRAY['CUSTOMER']::text[],access_policy_version=access_policy_version+1 WHERE client_id=$1")
            .bind(identity.client).execute(&db.pool).await?;
        check_retired(&db.pool,&first,true).await?;
        check_retired(&db.pool,&second,false).await?;
        ensure!(!session_active(&db.pool,first.sid).await? && session_active(&db.pool,second.sid).await?,"source policy invalidated a different client");
        let app=app(&db.pool,None)?;
        expect(&userinfo(&app,&first.access).await?,StatusCode::UNAUTHORIZED,"old source policy access")?;
        expect(&exchange_code(&app,&identity,&first).await?,StatusCode::BAD_REQUEST,"old source policy code")?;
        expect(&exchange_device(&app,&identity,&first).await?,StatusCode::BAD_REQUEST,"old source policy device")?;
        expect(&userinfo(&app,&second.access).await?,StatusCode::OK,"other client unaffected")?;
        // No physical cleanup: the account owner version must alone reject all
        // retained protocol credentials before any outbox consumer can run.
        // Owned fault injection: deliberately delay only physical cleanup. The
        // authoritative generation still commits normally and must be enough.
        let mut delayed_cleanup=db.pool.begin().await?;
        sqlx::query("ALTER TABLE users DISABLE TRIGGER trg_account_credential_generation").execute(&mut *delayed_cleanup).await?;
        sqlx::query("UPDATE users SET session_version=session_version+1 WHERE id=$1").bind(subject).execute(&mut *delayed_cleanup).await?;
        sqlx::query("ALTER TABLE users ENABLE TRIGGER trg_account_credential_generation").execute(&mut *delayed_cleanup).await?;
        delayed_cleanup.commit().await?;
        ensure!(!identity.active(&db.pool).await? && !session_active(&db.pool,second.sid).await?,"account version was not authoritative");
        let untouched:bool=sqlx::query_scalar("SELECT NOT revoked FROM oauth_tokens WHERE id=$1").bind(second.access_id).fetch_one(&db.pool).await?;
        ensure!(untouched,"fixture unexpectedly depended on physical account token cleanup");
        expect(&userinfo(&app,&second.access).await?,StatusCode::UNAUTHORIZED,"account version invalidates legacy access")?;
        expect(&exchange_code(&app,&other,&second).await?,StatusCode::BAD_REQUEST,"account version invalidates pending code")?;
        expect(&exchange_device(&app,&other,&second).await?,StatusCode::BAD_REQUEST,"account version invalidates approved device")?;
        expect(&form(&app,&[("grant_type","refresh_token"),("client_id",other.client),("refresh_token",&second.oauth_refresh)]).await?,StatusCode::BAD_REQUEST,"account version invalidates OAuth refresh")?;
        expect(&form(&app,&[("grant_type","refresh_token"),("refresh_token",&second.refresh)]).await?,StatusCode::BAD_REQUEST,"account version invalidates platform refresh")?;
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops identity_phase02_* databases using explicit IDENTITY_PHASE02_TEST_DATABASE_URL"]
async fn identity_binding_phase02_application_grant_isolation() -> Result<()> {
    let db = Database::create(14).await?;
    let result = async {
        let (tenant,org)=tenant(&db.pool,"phase02-app-policy").await?;
        let subject=user(&db.pool,"phase02-app-policy-user",false).await?;
        let mut apps=Vec::new();
        for code in ["phase02-app-a","phase02-app-b"] {
            let id=new_uuid();
            sqlx::query("INSERT INTO applications(id,name,code,base_path) VALUES($1,$2,$2,$3)")
                .bind(id).bind(code).bind(format!("/micro/{code}")).execute(&db.pool).await?;
            sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
                .bind(tenant).bind(id).execute(&db.pool).await?;
            sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)")
                .bind(org).bind(id).execute(&db.pool).await?;
            apps.push(id);
        }
        client(&db.pool,"phase02-app-client-a",&["EMPLOYEE"],Some(apps[0])).await?;
        client(&db.pool,"phase02-app-client-b",&["EMPLOYEE"],Some(apps[1])).await?;
        let first_identity=Identity::create(&db.pool,subject,tenant,org,"EMPLOYEE","phase02-app-client-a").await?;
        let mut second_identity=first_identity.clone();
        second_identity.client="phase02-app-client-b";
        let first=credentials(&db.pool,&first_identity,Some(apps[0])).await?;
        let second=credentials(&db.pool,&second_identity,Some(apps[1])).await?;
        let app=app(&db.pool,None)?;
        expect(&userinfo(&app,&first.access).await?,StatusCode::OK,"first app initial grant")?;
        expect(&userinfo(&app,&second.access).await?,StatusCode::OK,"second app initial grant")?;
        let pending=new_uuid();
        sqlx::query("INSERT INTO oauth_authorization_requests(id,client_id,redirect_uri,scope,code_challenge,code_challenge_method,expires_at,client_policy_version,application_id,application_policy_version) \
            VALUES($1,$2,$3,'profile',$4,'S256',now()+interval '5 minutes',1,$5,1)")
            .bind(pending).bind(first_identity.client).bind(REDIRECT).bind(hash::b64url_sha256(VERIFIER)).bind(apps[0])
            .execute(&db.pool).await?;
        sqlx::query("UPDATE applications SET allowed_member_types=ARRAY['CUSTOMER']::text[],access_policy_version=access_policy_version+1 WHERE id=$1")
            .bind(apps[0]).execute(&db.pool).await?;
        let retired:(bool,bool,bool,bool)=sqlx::query_as(
            "SELECT tok.revoked,code.consumed,device.status='EXPIRED',request.consumed \
             FROM oauth_tokens tok CROSS JOIN oauth_authorization_codes code CROSS JOIN oauth_device_codes device \
             CROSS JOIN oauth_authorization_requests request WHERE tok.id=$1 AND code.id=$2 AND device.id=$3 AND request.id=$4")
            .bind(first.access_id).bind(first.code_id).bind(first.device_id).bind(pending).fetch_one(&db.pool).await?;
        ensure!(retired==(true,true,true,true),"app policy failed to retire token/code/device/pending consent");
        ensure!(session_active(&db.pool,first.sid).await? && session_active(&db.pool,second.sid).await?,"app policy revoked an account authentication session");
        check_retired(&db.pool,&second,false).await?;
        expect(&userinfo(&app,&first.access).await?,StatusCode::UNAUTHORIZED,"first app retired access")?;
        expect(&exchange_code(&app,&first_identity,&first).await?,StatusCode::BAD_REQUEST,"first app retired code")?;
        expect(&exchange_device(&app,&first_identity,&first).await?,StatusCode::BAD_REQUEST,"first app retired device")?;
        expect(&userinfo(&app,&second.access).await?,StatusCode::OK,"other app access retained")?;
        expect(&exchange_code(&app,&second_identity,&second).await?,StatusCode::OK,"other app code retained")?;
        expect(&exchange_code(&app,&second_identity,&second).await?,StatusCode::BAD_REQUEST,"one-time authorization code replay")?;
        expect(&exchange_device(&app,&second_identity,&second).await?,StatusCode::OK,"other app approved device retained")?;
        expect(&exchange_device(&app,&second_identity,&second).await?,StatusCode::BAD_REQUEST,"one-time device replay")?;
        let expired=credentials(&db.pool,&second_identity,Some(apps[1])).await?;
        sqlx::query("UPDATE oauth_tokens SET expires_at=now()-interval '1 second' WHERE id=$1 OR token_hash=$2")
            .bind(expired.access_id).bind(hash::sha256_hex(&expired.oauth_refresh)).execute(&db.pool).await?;
        sqlx::query("UPDATE oauth_authorization_codes SET expires_at=now()-interval '1 second' WHERE id=$1")
            .bind(expired.code_id).execute(&db.pool).await?;
        sqlx::query("UPDATE oauth_device_codes SET expires_at=now()-interval '1 second' WHERE id=$1")
            .bind(expired.device_id).execute(&db.pool).await?;
        expect(&userinfo(&app,&expired.access).await?,StatusCode::UNAUTHORIZED,"expired stored OAuth access")?;
        expect(&exchange_code(&app,&second_identity,&expired).await?,StatusCode::BAD_REQUEST,"expired stored authorization code")?;
        expect(&exchange_device(&app,&second_identity,&expired).await?,StatusCode::BAD_REQUEST,"expired stored approved device")?;
        expect(&form(&app,&[("grant_type","refresh_token"),("client_id",second_identity.client),("refresh_token",&expired.oauth_refresh)]).await?,StatusCode::BAD_REQUEST,"expired stored OAuth refresh")?;
        let unconsumed:bool=sqlx::query_scalar("SELECT NOT consumed FROM oauth_authorization_codes WHERE id=$1")
            .bind(expired.code_id).fetch_one(&db.pool).await?;
        ensure!(unconsumed,"expired code was consumed or upgraded during rejection");
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops identity_phase02_* databases using explicit IDENTITY_PHASE02_TEST_DATABASE_URL"]
async fn identity_binding_phase02_real_http_issuance_and_application_policy() -> Result<()> {
    let db = Database::create(i64::MAX).await?;
    let result=async {
        let (tenant,org)=tenant(&db.pool,"phase02-http").await?;
        let subject=user(&db.pool,"phase02-http-user",false).await?;
        let identity=Identity::create(&db.pool,subject,tenant,org,"EMPLOYEE","platform-console-legacy").await?;
        let password_hash=tokio::task::spawn_blocking(||crate::util::password::hash_password(PASSWORD)).await??;
        sqlx::query("UPDATE users SET password_hash=$2 WHERE id=$1").bind(subject).bind(password_hash).execute(&db.pool).await?;
        let application=new_uuid();
        sqlx::query("INSERT INTO applications(id,name,code,base_path) VALUES($1,'Phase 2 HTTP app','phase02-http-app','/micro/phase02')")
            .bind(application).execute(&db.pool).await?;
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)").bind(tenant).bind(application).execute(&db.pool).await?;
        sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)").bind(org).bind(application).execute(&db.pool).await?;
        sqlx::query("INSERT INTO oauth_clients(id,client_id,client_name,client_type,client_secret_hash,grant_types,subject_mode,allowed_member_types,application_id) \
            VALUES($1,'phase02-http-service','Phase 2 app service','CONFIDENTIAL',$2,'client_credentials','MACHINE',ARRAY[]::text[],$3)")
            .bind(new_uuid()).bind(hash::sha256_hex("owned-phase02-service-secret")).bind(application).execute(&db.pool).await?;
        let app=app(&db.pool,None)?;
        let login=json_request(&app,Method::POST,"/api/auth/login",None,json!({"username":"phase02-http-user","password":PASSWORD})).await?;
        expect(&login,StatusCode::OK,"real password login")?;
        let ticket=login.body["ticket"].as_str().ok_or_else(||anyhow!("login response lacks selection ticket"))?;
        let context=json_request(&app,Method::POST,"/api/auth/context",Some(ticket),json!({"organizationId":org,"membershipId":identity.member})).await?;
        expect(&context,StatusCode::OK,"real membership context issuance")?;
        let bearer=context.body["accessToken"].as_str().ok_or_else(||anyhow!("context response lacks token"))?;
        let claims=jwt::verify(bearer,SECRET)?;
        let sid:Uuid=claims.sid.ok_or_else(||anyhow!("session JWT lacks sid"))?.parse()?;
        let proof:(Uuid,String,i64,i64,String,String,i64)=sqlx::query_as("SELECT membership_id,member_type,identity_version,account_version,origin_issuer,origin_client_ref,origin_client_policy_version FROM platform_sessions WHERE id=$1")
            .bind(sid).fetch_one(&db.pool).await?;
        ensure!(proof==(identity.member,"EMPLOYEE".into(),1,0,ISSUER.into(),"platform-console-legacy".into(),1),"context did not persist exact original authority");
        let issued=json_request(&app,Method::POST,"/api/oauth/session-token",Some(bearer),json!({"appCode":"phase02-http-app"})).await?;
        expect(&issued,StatusCode::CREATED,"application token from real session")?;
        let token=issued.body["accessToken"].as_str().ok_or_else(||anyhow!("app response lacks token"))?;
        let token_claims=jwt::verify(token,SECRET)?;
        ensure!(crate::app_token::inspect(&db.pool,token,&token_claims).await?.is_some(),"fresh application token not accepted");
        let provenance:(Uuid,String,String,i64,String,i64)=sqlx::query_as("SELECT membership_id,origin_client_ref,application_service_client_ref,origin_client_policy_version,client_id,application_policy_version FROM oauth_tokens WHERE token_hash=$1")
            .bind(hash::sha256_hex(token)).fetch_one(&db.pool).await?;
        ensure!(provenance==(identity.member,"platform-console-legacy".into(),"phase02-http-service".into(),1,"phase02-http-service".into(),1),"application service cid overwrote source login client");
        let refresh=context.body["refreshToken"].as_str().ok_or_else(||anyhow!("context response lacks platform refresh"))?;
        let rotated=form(&app,&[("grant_type","refresh_token"),("refresh_token",refresh)]).await?;
        expect(&rotated,StatusCode::OK,"real bound platform refresh")?;
        expect(&form(&app,&[("grant_type","refresh_token"),("refresh_token",refresh)]).await?,StatusCode::BAD_REQUEST,"platform refresh replay")?;
        sqlx::query("UPDATE applications SET allowed_member_types=ARRAY['CUSTOMER']::text[],access_policy_version=access_policy_version+1 WHERE id=$1")
            .bind(application).execute(&db.pool).await?;
        ensure!(crate::app_token::inspect(&db.pool,token,&token_claims).await?.is_none(),"application policy failed to reject old token");
        ensure!(session_active(&db.pool,sid).await?,"application admission change revoked unrelated platform login");
        let rejected=json_request(&app,Method::POST,"/api/oauth/session-token",Some(bearer),json!({"appCode":"phase02-http-app"})).await?;
        ensure!(matches!(rejected.status,StatusCode::FORBIDDEN|StatusCode::NOT_FOUND),"employee received token outside application member set");
        sqlx::query("UPDATE platform_sessions SET issued_at=now()-interval '1 hour',expires_at=now()-interval '1 second' WHERE id=$1").bind(sid).execute(&db.pool).await?;
        expect(&json_request(&app,Method::GET,"/api/auth/profile",Some(bearer),Value::Null).await?,StatusCode::UNAUTHORIZED,"expired stored platform session")?;
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops identity_phase02_* databases using explicit IDENTITY_PHASE02_TEST_DATABASE_URL"]
async fn identity_binding_phase02_authority_outage_does_not_execute_business() -> Result<()> {
    let db = Database::create(i64::MAX).await?;
    let result = async {
        let (tenant, org) = tenant(&db.pool, "phase02-outage").await?;
        let (_, actor) = admin_identity(&db.pool, tenant, org).await?;
        let probe = Arc::new(AtomicUsize::new(0));
        let app = app(&db.pool, Some(Arc::clone(&probe)))?;
        expect(
            &json_request(
                &app,
                Method::POST,
                "/identity-phase02/protected-write",
                Some(&actor.session),
                json!({}),
            )
            .await?,
            StatusCode::NO_CONTENT,
            "healthy protected business",
        )?;
        let independent = db.independent_pool().await?;
        let before: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs")
            .fetch_one(&independent)
            .await?;
        db.pool.close().await;
        expect(
            &json_request(
                &app,
                Method::POST,
                "/identity-phase02/protected-write",
                Some(&actor.session),
                json!({}),
            )
            .await?,
            StatusCode::SERVICE_UNAVAILABLE,
            "closed authoritative pool",
        )?;
        ensure!(
            probe.load(Ordering::SeqCst) == 1,
            "business handler executed after authority failed"
        );
        let after: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs")
            .fetch_one(&independent)
            .await?;
        ensure!(
            before == after,
            "authority failure produced business audit writes"
        );
        independent.close().await;
        Ok(())
    }
    .await;
    db.finish(result).await
}
