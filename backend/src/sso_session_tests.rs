//! Current platform-session regressions over a completed canonical exchange.
//! The shared proof-only fixture does not sign OAuth tokens or expose SSO routes.
//! Identity owns login/password/PKCE/refresh/logout protocol tests; retired
//! platform password, device and second-issuer tests are intentionally removed.
use crate::{
    config::Config,
    state::AppState,
    util::{jwt, uuid7},
};
use anyhow::{anyhow, ensure, Result};
use axum::{
    body::{to_bytes, Body},
    http::{header, Method, Request, StatusCode},
    Router,
};
use serde_json::{json, Value};
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool,
};
use std::{collections::HashMap, str::FromStr, sync::Arc, time::Duration};
use tower::ServiceExt;
use uuid::Uuid;

const SECRET: &str = "phase24-owned-platform-session-regression-signing-key";

struct Database {
    pool: PgPool,
    admin: PgPool,
    name: String,
}

impl Database {
    async fn create() -> Result<Self> {
        let raw = std::env::var("IDENTITY_PHASE24_TEST_DATABASE_URL")
            .map_err(|_| anyhow!("explicit IDENTITY_PHASE24_TEST_DATABASE_URL is required"))?;
        let url = reqwest::Url::parse(&raw)
            .map_err(|_| anyhow!("invalid isolated Phase 24 database configuration"))?;
        let port = url
            .port()
            .ok_or_else(|| anyhow!("Phase 24 administration requires an explicit port"))?;
        let loopback = |host: &str| {
            matches!(
                host.trim_matches(&['[', ']'][..]),
                "127.0.0.1" | "localhost" | "::1"
            )
        };
        ensure!(matches!(url.scheme(),"postgres"|"postgresql") &&
                url.host_str().is_some_and(loopback) && port>0 && !url.username().is_empty() &&
                url.fragment().is_none() &&
                url.query_pairs().all(|(key,value)| matches!(key.as_ref(),"sslmode"|"ssl-mode") &&
                    matches!(value.as_ref(),"disable"|"prefer"|"require")),
                "Phase 24 administration requires an explicit loopback host, port and account without redirection");
        let options = PgConnectOptions::from_str(&raw)
            .map_err(|_| anyhow!("invalid isolated Phase 24 database configuration"))?;
        ensure!(
            options.get_database() == Some("postgres")
                && loopback(options.get_host())
                && options.get_port() == port
                && !options.get_username().is_empty(),
            "Phase 24 administration must explicitly select postgres"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("isolated Phase 24 administration connection failed"))?;
        let expected = std::env::var("IDENTITY_PHASE24_TEST_CLUSTER_ID")
            .map_err(|_| anyhow!("explicit owned Phase 24 cluster identifier required"))?;
        let actual: String =
            sqlx::query_scalar("SELECT system_identifier::text FROM pg_control_system()")
                .fetch_one(&admin)
                .await?;
        ensure!(actual == expected, "refusing another PostgreSQL cluster");
        let name = format!("identity_phase24_sessions_{}", uuid7::new_uuid().simple());
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await?;
        let pool = match PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.database(&name))
            .await
        {
            Ok(pool) => pool,
            Err(_) => {
                let _ = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
                    .execute(&admin)
                    .await;
                admin.close().await;
                return Err(anyhow!("owned Phase 24 database connection failed"));
            }
        };
        let database = Self { pool, admin, name };
        if let Err(error) = sqlx::migrate!("./migrations").run(&database.pool).await {
            let _ = database.finish(Ok(())).await;
            return Err(error.into());
        }
        Ok(database)
    }

    async fn finish(self, assertions: Result<()>) -> Result<()> {
        let suffix = self.name.strip_prefix("identity_phase24_sessions_");
        ensure!(
            suffix.is_some_and(|value| value.len() == 32
                && value.bytes().all(|ch| ch.is_ascii_hexdigit())
                && Uuid::parse_str(value).is_ok_and(uuid7::is_uuid_v7)),
            "refusing cleanup outside an owned Phase 24 database"
        );
        let closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let removed = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await;
        self.admin.close().await;
        eprintln!(
            "identity_phase24_sessions_cleanup database={} assertions_ok={} database_removed={} pool_closed={closed}",
            self.name, assertions.is_ok(), removed.is_ok()
        );
        assertions?;
        removed?;
        ensure!(closed, "owned Phase 24 pool did not close");
        Ok(())
    }
}

fn config() -> Result<Arc<Config>> {
    Ok(Arc::new(Config::from_values(&HashMap::from([
        ("APP_ENV".into(), "test".into()),
        (
            "DATABASE_URL".into(),
            "postgres://unused/owned-phase24".into(),
        ),
        ("JWT_SECRET".into(), SECRET.into()),
        ("MEMBERSHIP_LIFECYCLE_ENABLED".into(), "true".into()),
        ("CUSTOMER_MEMBERSHIPS_ENABLED".into(), "true".into()),
        ("DUAL_MEMBERSHIPS_ENABLED".into(), "true".into()),
    ]))?))
}
fn router(state: AppState) -> Router {
    crate::routes::api_router().with_state(state)
}
struct Reply {
    status: StatusCode,
    body: Value,
    sets_cookie: bool,
}
async fn call(app: &Router, method: Method, path: &str, token: Option<&str>) -> Result<Reply> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = tokio::time::timeout(
        Duration::from_secs(10),
        app.clone().oneshot(request.body(Body::empty())?),
    )
    .await??;
    let status = response.status();
    let sets_cookie = response.headers().contains_key(header::SET_COOKIE);
    let raw = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok(Reply {
        status,
        body: serde_json::from_slice(&raw).unwrap_or(Value::Null),
        sets_cookie,
    })
}
fn session_id(token: &str) -> Result<Uuid> {
    let proof = jwt::verify_internal(
        token,
        SECRET,
        jwt::InternalUse::PlatformSession,
        Some(jwt::PLATFORM_API_AUDIENCE),
    )?;
    Ok(Uuid::parse_str(
        proof
            .sid
            .as_deref()
            .ok_or_else(|| anyhow!("owned session has no source"))?,
    )?)
}
struct Fixture {
    state: AppState,
    user: Uuid,
    other: Uuid,
    member: Uuid,
    customer: Uuid,
    first: String,
    second: String,
    unrelated: String,
}
impl Fixture {
    async fn create(pool: &PgPool) -> Result<Self> {
        let tenant = uuid7::new_uuid();
        let root = uuid7::new_uuid();
        let second_org = uuid7::new_uuid();
        sqlx::query("INSERT INTO tenants(id,name,code,is_platform) VALUES($1,'Owned current sessions',$2,true)")
            .bind(tenant).bind(format!("p24-{}",tenant.simple())).execute(pool).await?;
        sqlx::query("INSERT INTO organizations(id,tenant_id,name) VALUES($1,$2,'Owned root')")
            .bind(root)
            .bind(tenant)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO organizations(id,tenant_id,parent_id,name) VALUES($1,$2,$3,'Owned second context')")
            .bind(second_org).bind(tenant).bind(root).execute(pool).await?;
        let user = uuid7::new_uuid();
        let other = uuid7::new_uuid();
        for id in [user, other] {
            sqlx::query("INSERT INTO users(id,username,password_hash,session_version) VALUES($1,$2,'unused-owned-canonical-fixture',1)")
                .bind(id).bind(format!("p24-{}",id.simple())).execute(pool).await?;
        }
        let member = uuid7::new_uuid();
        let customer = uuid7::new_uuid();
        for (id, account, org, kind) in [
            (member, user, root, "EMPLOYEE"),
            (uuid7::new_uuid(), user, second_org, "EMPLOYEE"),
            (customer, user, root, "CUSTOMER"),
            (uuid7::new_uuid(), other, root, "EMPLOYEE"),
        ] {
            sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,status,identity_version) VALUES($1,$2,$3,$4,'ENABLED',1)")
                .bind(id).bind(account).bind(org).bind(kind).execute(pool).await?;
        }
        let first = crate::sso_test_fixture::token(pool, user, tenant, root, SECRET).await?;
        let second = crate::sso_test_fixture::token(pool, user, tenant, second_org, SECRET).await?;
        let unrelated = crate::sso_test_fixture::token(pool, other, tenant, root, SECRET).await?;
        let state = crate::sso_test_fixture::state(pool.clone(), config()?);
        Ok(Self {
            state,
            user,
            other,
            member,
            customer,
            first,
            second,
            unrelated,
        })
    }
    fn app(&self) -> Router {
        router(self.state.clone())
    }
}
async fn expect_profile(app: &Router, token: &str, status: StatusCode) -> Result<()> {
    let reply = call(app, Method::GET, "/api/auth/profile", Some(token)).await?;
    ensure!(
        reply.status == status,
        "current platform profile returned unexpected status {}",
        reply.status
    );
    ensure!(
        !reply.sets_cookie,
        "platform bearer profile changed a browser authentication cookie"
    );
    Ok(())
}

#[tokio::test]
async fn platform_has_no_password_sso_or_second_oauth_authority_even_without_a_database(
) -> Result<()> {
    let pool = PgPoolOptions::new().connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")?;
    let app = router(AppState::new(pool, config()?));
    for (method, path) in [
        (Method::POST, "/api/auth/login"),
        (Method::POST, "/api/auth/context"),
        (Method::GET, "/api/sso/preauth"),
        (Method::POST, "/api/sso/login"),
        (Method::GET, "/api/sso/session"),
        (Method::POST, "/api/sso/logout"),
        (Method::PUT, "/api/sso/password"),
        (Method::GET, "/oauth/authorize"),
        (Method::POST, "/oauth/token"),
        (Method::GET, "/oauth/userinfo"),
        (Method::POST, "/oauth/introspect"),
        (Method::POST, "/oauth/revoke"),
        (Method::POST, "/oauth/device/authorization"),
    ] {
        let reply = call(&app, method, path, None).await?;
        ensure!(
            reply.status == StatusCode::NOT_FOUND,
            "retired platform authentication route remained mounted: {path}"
        );
        ensure!(
            !reply.sets_cookie,
            "retired platform route emitted an authentication cookie"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit owned IDENTITY_PHASE24_TEST_DATABASE_URL/CLUSTER_ID"]
async fn canonical_account_generation_invalidates_all_own_contexts_but_preserves_other_accounts(
) -> Result<()> {
    let db = Database::create().await?;
    let result = async {
        let f = Fixture::create(&db.pool).await?;
        let app = f.app();
        for token in [&f.first, &f.second, &f.unrelated] {
            expect_profile(&app, token, StatusCode::OK).await?;
        }
        let memberships: Value = sqlx::query_scalar(
            "SELECT jsonb_agg(to_jsonb(m) ORDER BY id) FROM memberships m WHERE user_id=$1",
        )
        .bind(f.user)
        .fetch_one(&db.pool)
        .await?;
        // This owner-side mutation is an explicit source fault; private identity
        // command atomicity is exercised in the identity repository.
        sqlx::query("UPDATE users SET session_version=session_version+1 WHERE id=$1")
            .bind(f.user)
            .execute(&db.pool)
            .await?;
        for token in [&f.first, &f.second] {
            expect_profile(&app, token, StatusCode::UNAUTHORIZED).await?;
        }
        expect_profile(&app, &f.unrelated, StatusCode::OK).await?;
        let unchanged: Value = sqlx::query_scalar(
            "SELECT jsonb_agg(to_jsonb(m) ORDER BY id) FROM memberships m WHERE user_id=$1",
        )
        .bind(f.user)
        .fetch_one(&db.pool)
        .await?;
        let other_version: i64 =
            sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
                .bind(f.other)
                .fetch_one(&db.pool)
                .await?;
        ensure!(
            unchanged == memberships && other_version == 1,
            "account revocation crossed membership/account ownership"
        );
        Ok(())
    }
    .await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit owned IDENTITY_PHASE24_TEST_DATABASE_URL/CLUSTER_ID"]
async fn canonical_member_departure_preserves_sibling_context_customer_identity_and_global_sources(
) -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let f=Fixture::create(&db.pool).await?;let app=f.app();
        for token in [&f.first,&f.second,&f.unrelated] {expect_profile(&app,token,StatusCode::OK).await?;}
        let global:i64=sqlx::query_scalar("SELECT count(*) FROM sso_sessions WHERE user_id=$1 AND revoked_at IS NULL").bind(f.user).fetch_one(&db.pool).await?;
        sqlx::query("UPDATE memberships SET status='DISABLED',identity_version=identity_version+1 WHERE id=$1").bind(f.member).execute(&db.pool).await?;
        expect_profile(&app,&f.first,StatusCode::UNAUTHORIZED).await?;
        for token in [&f.second,&f.unrelated] {expect_profile(&app,token,StatusCode::OK).await?;}
        let preserved:bool=sqlx::query_scalar("SELECT (SELECT session_version=1 FROM users WHERE id=$1) AND (SELECT status='ENABLED' AND identity_version=1 FROM memberships WHERE id=$2) AND (SELECT count(*)=$3 FROM sso_sessions WHERE user_id=$1 AND revoked_at IS NULL)")
            .bind(f.user).bind(f.customer).bind(global).fetch_one(&db.pool).await?;
        ensure!(preserved,"member-local departure revoked account, customer or unrelated global sources");
        sqlx::query("UPDATE memberships SET status='ENABLED',identity_version=identity_version+1 WHERE id=$1").bind(f.member).execute(&db.pool).await?;
        expect_profile(&app,&f.first,StatusCode::UNAUTHORIZED).await?;
        expect_profile(&app,&f.second,StatusCode::OK).await?;
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit owned IDENTITY_PHASE24_TEST_DATABASE_URL/CLUSTER_ID"]
async fn canonical_personal_session_listing_and_revocation_are_exactly_owner_scoped() -> Result<()>
{
    let db = Database::create().await?;
    let result=async {
        let f=Fixture::create(&db.pool).await?;let app=f.app();
        let first=session_id(&f.first)?;let second=session_id(&f.second)?;let unrelated=session_id(&f.unrelated)?;
        let list=call(&app,Method::GET,"/api/auth/my-sessions",Some(&f.second)).await?;
        ensure!(list.status==StatusCode::OK,"current personal-session list failed");
        let entries=list.body["platformSessions"].as_array().ok_or_else(||anyhow!("current personal-session list omitted rows"))?;
        ensure!(entries.len()==2&&entries.iter().all(|entry|entry["userId"]==json!(f.user))&&entries.iter().any(|entry|entry["id"]==json!(second)&&entry["isCurrent"]==true),"personal session listing crossed account ownership or lost current context");
        let denied=call(&app,Method::DELETE,&format!("/api/auth/sessions/{unrelated}"),Some(&f.second)).await?;
        ensure!(denied.status==StatusCode::NOT_FOUND,"personal revocation exposed or changed another account session");
        let revoked=call(&app,Method::DELETE,&format!("/api/auth/sessions/{first}"),Some(&f.second)).await?;
        ensure!(revoked.status==StatusCode::OK,"current owner could not revoke its exact device session");
        expect_profile(&app,&f.first,StatusCode::UNAUTHORIZED).await?;
        for token in [&f.second,&f.unrelated] {expect_profile(&app,token,StatusCode::OK).await?;}
        let counts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM audit_logs WHERE user_id=$1 AND action='REVOKE_SESSION'),(SELECT count(*) FROM platform_sessions WHERE id=$2 AND revoked_at IS NULL)")
            .bind(f.user).bind(unrelated).fetch_one(&db.pool).await?;
        ensure!(counts==(1,1),"session command omitted its audit or revoked another account");
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit owned IDENTITY_PHASE24_TEST_DATABASE_URL/CLUSTER_ID"]
async fn canonical_unavailable_source_is_503_without_changing_local_session_or_cookie() -> Result<()>
{
    let db = Database::create().await?;
    let result = async {
        let f = Fixture::create(&db.pool).await?;
        let app = f.app();
        expect_profile(&app, &f.first, StatusCode::OK).await?;
        f.state.pool.close().await;
        let reply = call(&app, Method::GET, "/api/auth/profile", Some(&f.first)).await?;
        ensure!(
            reply.status == StatusCode::SERVICE_UNAVAILABLE && !reply.sets_cookie,
            "source outage changed credentials or was reported as bad authentication"
        );
        // Credential preservation on remote transport failures is covered by
        // current BFF tests; this closed-pool case proves fail-closed dispatch.
        Ok(())
    }
    .await;
    db.finish(result).await
}
