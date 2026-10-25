//! Phase 8 legacy self-command authority, using owned databases and real lock waits.
//! Only IDENTITY_PHASE08_TEST_DATABASE_URL is read; no dotenv or existing target is used.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use axum::{middleware, Router};
use serde_json::{json, Value};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{PgConnection, PgPool};
use tower::ServiceExt;
use uuid::Uuid;

use crate::config::Config;
use crate::state::AppState;
use crate::util::{password, uuid7};

const SECRET: &str = "identity-phase08-owned-legacy-command-signing-key";
const PASSWORD: &str = "Owned-Phase08-Legacy-Password9!";
const PENDING_PASSWORD: &str = "Owned-Phase08-Pending-Password8!";
const ADMIN_PASSWORD: &str = "Owned-Phase08-Administrator-Reset7!";
// The actual owner-side lock after password verification and preparation.
const ACCOUNT_LOCK: &str =
    "SELECT id,session_version,password_hash FROM users WHERE id=$1 FOR UPDATE";

struct Database {
    pool: PgPool,
    admin: PgPool,
    name: String,
}

impl Database {
    async fn create() -> Result<Self> {
        let raw = std::env::var("IDENTITY_PHASE08_TEST_DATABASE_URL")
            .map_err(|_| anyhow!("explicit IDENTITY_PHASE08_TEST_DATABASE_URL is required"))?;
        let url = reqwest::Url::parse(&raw)
            .map_err(|_| anyhow!("invalid isolated Phase 8 database configuration"))?;
        let port = url
            .port()
            .ok_or_else(|| anyhow!("Phase 8 administration requires an explicit port"))?;
        let loopback = |host: &str| {
            matches!(
                host.trim_matches(&['[', ']'][..]),
                "127.0.0.1" | "localhost" | "::1"
            )
        };
        ensure!(
            matches!(url.scheme(), "postgres" | "postgresql")
                && url.host_str().is_some_and(loopback)
                && port > 0
                && !url.username().is_empty()
                && url.password().is_some()
                && url.fragment().is_none()
                && url.query_pairs().all(|(key, value)| matches!(
                    key.as_ref(),
                    "sslmode" | "ssl-mode"
                ) && matches!(
                    value.as_ref(),
                    "disable" | "prefer" | "require"
                )),
            "Phase 8 requires an explicit loopback host, port and account without redirection"
        );
        let options = PgConnectOptions::from_str(&raw)
            .map_err(|_| anyhow!("invalid isolated Phase 8 database configuration"))?;
        ensure!(
            options.get_database() == Some("postgres")
                && loopback(options.get_host())
                && options.get_port() == port
                && !options.get_username().is_empty(),
            "Phase 8 administration must explicitly select postgres on loopback"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("isolated Phase 8 administration connection failed"))?;
        let name = format!("identity_phase08_{}", uuid7::new_uuid().simple());
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
                return Err(anyhow!("owned Phase 8 database connection failed"));
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
        ensure!(
            self.name
                .strip_prefix("identity_phase08_")
                .is_some_and(|suffix| suffix.len() == 32
                    && suffix.bytes().all(|ch| ch.is_ascii_hexdigit())
                    && Uuid::parse_str(suffix).is_ok_and(uuid7::is_uuid_v7)),
            "refusing cleanup outside an owned Phase 8 database"
        );
        let closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let removed = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await;
        self.admin.close().await;
        eprintln!("identity_phase08_legacy_cleanup database={} assertions_ok={} database_removed={} pool_closed={closed}",
                  self.name,assertions.is_ok(),removed.is_ok());
        assertions?;
        removed?;
        ensure!(closed, "owned Phase 8 pool did not close");
        Ok(())
    }
}

struct Reply {
    status: StatusCode,
    body: Value,
}

async fn call(
    app: &Router,
    method: Method,
    path: &str,
    bearer: Option<&str>,
    body: Value,
) -> Result<Reply> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-request-id", uuid7::new_uuid().to_string())
        .extension(ConnectInfo("192.0.2.38:30038".parse::<SocketAddr>()?));
    if let Some(token) = bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let reply = tokio::time::timeout(
        Duration::from_secs(30),
        app.clone()
            .oneshot(builder.body(Body::from(body.to_string()))?),
    )
    .await
    .map_err(|_| anyhow!("owned Phase 8 legacy request timed out"))??;
    let status = reply.status();
    let bytes = to_bytes(reply.into_body(), 1024 * 1024).await?;
    Ok(Reply {
        status,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    })
}

fn expect(reply: &Reply, status: StatusCode, label: &str) -> Result<()> {
    ensure!(
        reply.status == status,
        "{label}: expected {status}, received {}",
        reply.status
    );
    Ok(())
}

fn text<'a>(body: &'a Value, name: &str) -> Result<&'a str> {
    body[name]
        .as_str()
        .ok_or_else(|| anyhow!("missing public response field {name}"))
}

struct Fixture {
    state: AppState,
    app: Router,
    user: Uuid,
    member: Uuid,
    other_member: Uuid,
    admin_access: String,
    access: String,
    other_access: String,
    unrelated_access: String,
}

async fn create_user(pool: &PgPool, name: &str, encoded: &str, admin: bool) -> Result<Uuid> {
    let id = uuid7::new_uuid();
    sqlx::query("INSERT INTO users(id,username,password_hash,is_super_admin) VALUES($1,$2,$3,$4)")
        .bind(id)
        .bind(name)
        .bind(encoded)
        .bind(admin)
        .execute(pool)
        .await?;
    Ok(id)
}

async fn member(pool: &PgPool, user: Uuid, org: Uuid) -> Result<Uuid> {
    let id = uuid7::new_uuid();
    sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,status) VALUES($1,$2,$3,'EMPLOYEE','ENABLED')")
        .bind(id).bind(user).bind(org).execute(pool).await?;
    Ok(id)
}

async fn context(
    app: &Router,
    name: &str,
    member: Uuid,
    org: Uuid,
    supplied: &str,
) -> Result<String> {
    let login = call(
        app,
        Method::POST,
        "/api/auth/login",
        None,
        json!({"username":name,"password":supplied}),
    )
    .await?;
    expect(
        &login,
        StatusCode::OK,
        "real legacy password authentication",
    )?;
    let context = call(
        app,
        Method::POST,
        "/api/auth/context",
        Some(text(&login.body, "ticket")?),
        json!({"membershipId":member,"organizationId":org}),
    )
    .await?;
    expect(&context, StatusCode::OK, "real concrete employee context")?;
    ensure!(
        context.body["membershipId"] == json!(member) && context.body["memberType"] == "EMPLOYEE",
        "legacy context changed member identity"
    );
    Ok(text(&context.body, "accessToken")?.to_owned())
}

impl Fixture {
    async fn create(pool: &PgPool) -> Result<Self> {
        let tenant = uuid7::new_uuid();
        let org = uuid7::new_uuid();
        let other_org = uuid7::new_uuid();
        sqlx::query("INSERT INTO tenants(id,name,code,is_platform) VALUES($1,'Owned Phase 8 tenant','admin-tenant',true)")
            .bind(tenant).execute(pool).await?;
        sqlx::query(
            "INSERT INTO organizations(id,tenant_id,name) VALUES($1,$2,'Owned Phase 8 root')",
        )
        .bind(org)
        .bind(tenant)
        .execute(pool)
        .await?;
        sqlx::query("INSERT INTO organizations(id,tenant_id,parent_id,name) VALUES($1,$2,$3,'Owned Phase 8 branch')")
            .bind(other_org).bind(tenant).bind(org).execute(pool).await?;
        let encoded = password::hash_password(PASSWORD)?;
        let admin = create_user(pool, "phase08-legacy-admin", &encoded, true).await?;
        let user = create_user(pool, "phase08-legacy-subject", &encoded, false).await?;
        let unrelated = create_user(pool, "phase08-legacy-unrelated", &encoded, false).await?;
        let admin_member = member(pool, admin, org).await?;
        let membership = member(pool, user, org).await?;
        let other_member = member(pool, user, other_org).await?;
        let unrelated_member = member(pool, unrelated, org).await?;
        let application = uuid7::new_uuid();
        let menu = uuid7::new_uuid();
        sqlx::query("INSERT INTO applications(id,name,code,allowed_member_types) VALUES($1,'Owned Phase 8 administration','phase08-legacy-admin',ARRAY['EMPLOYEE']::text[])")
            .bind(application).execute(pool).await?;
        sqlx::query("INSERT INTO menus(id,application_id,type,title,permission,path) VALUES($1,$2,'MENU','Owned Phase 8 users','system:user','/system/users')")
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
        let config = Config::from_values(&HashMap::from([
            ("APP_ENV".into(), "test".into()),
            (
                "DATABASE_URL".into(),
                "postgres://unused/phase08-legacy".into(),
            ),
            ("JWT_SECRET".into(), SECRET.into()),
            ("LOGIN_SOURCE_REQUESTS".into(), "1000".into()),
        ]))?;
        let state = AppState::new(pool.clone(), Arc::new(config));
        let app = crate::routes::api_router()
            .nest("/oauth", crate::routes::oauth2::oauth_router())
            .layer(middleware::from_fn_with_state(
                state.clone(),
                crate::observability::observe,
            ))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                crate::normalize_client_ip,
            ))
            .with_state(state.clone());
        let admin_access =
            context(&app, "phase08-legacy-admin", admin_member, org, PASSWORD).await?;
        let access = context(&app, "phase08-legacy-subject", membership, org, PASSWORD).await?;
        let other_access = context(
            &app,
            "phase08-legacy-subject",
            other_member,
            other_org,
            PASSWORD,
        )
        .await?;
        let unrelated_access = context(
            &app,
            "phase08-legacy-unrelated",
            unrelated_member,
            org,
            PASSWORD,
        )
        .await?;
        Ok(Self {
            state,
            app,
            user,
            member: membership,
            other_member,
            admin_access,
            access,
            other_access,
            unrelated_access,
        })
    }
}

async fn snapshot(connection: &mut PgConnection, user: Uuid) -> Result<Value> {
    // Only compared in memory, never included in diagnostic output.
    Ok(sqlx::query_scalar("SELECT jsonb_build_object( \
        'account',to_jsonb(u), \
        'memberships',(SELECT COALESCE(jsonb_agg(to_jsonb(m) ORDER BY m.id),'[]'::jsonb) FROM memberships m WHERE m.user_id=u.id), \
        'sessions',(SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY s.id),'[]'::jsonb) FROM platform_sessions s WHERE s.user_id=u.id), \
        'refresh',(SELECT COALESCE(jsonb_agg(to_jsonb(r) ORDER BY r.id),'[]'::jsonb) FROM platform_session_refresh_tokens r JOIN platform_sessions s ON s.id=r.session_id WHERE s.user_id=u.id), \
        'oauth',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY t.id),'[]'::jsonb) FROM oauth_tokens t WHERE t.user_id=u.id), \
        'audit',(SELECT COALESCE(jsonb_agg(to_jsonb(a) ORDER BY a.id),'[]'::jsonb) FROM audit_logs a WHERE a.user_id=u.id), \
        'notifications',(SELECT COALESCE(jsonb_agg(to_jsonb(n) ORDER BY n.id),'[]'::jsonb) FROM notifications n WHERE n.user_id=u.id)) \
        FROM users u WHERE u.id=$1").bind(user).fetch_one(connection).await?)
}

async fn wait_for_account_lock(pool: &PgPool, blocker_pid: i32) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(20),async {
        loop {
            let observed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity a \
                WHERE a.datname=current_database() AND a.pid<>pg_backend_pid() AND a.state='active' \
                  AND a.wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(a.pid)) AND a.query=$2)")
                .bind(blocker_pid).bind(ACCOUNT_LOCK).fetch_one(pool).await?;
            if observed { return Ok::<_,anyhow::Error>(()); }
            tokio::task::yield_now().await;
        }
    }).await.map_err(|_|anyhow!("pending legacy command never reached the exact observed account-lock barrier"))?
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mutation {
    Ban,
    Kick,
    CurrentMember,
    OtherMember,
    Reset,
}

async fn lock_wait_race(pool: &PgPool, mutation: Mutation) -> Result<()> {
    let fixture = Fixture::create(pool).await?;
    // Prepare the administrator's replacement before taking the test blocker.
    let admin_password = fixture
        .state
        .accounts
        .prepare_password(ADMIN_PASSWORD)
        .await?;
    let before: (String, i64) =
        sqlx::query_as("SELECT password_hash,session_version FROM users WHERE id=$1")
            .bind(fixture.user)
            .fetch_one(pool)
            .await?;
    let mut blocker = pool.begin().await?;
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await?;
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(fixture.user)
        .execute(&mut *blocker)
        .await?;
    let app = fixture.app.clone();
    let bearer = fixture.access.clone();
    let mut pending = tokio::spawn(async move {
        call(
            &app,
            Method::PUT,
            "/api/auth/password",
            Some(&bearer),
            json!({"oldPassword":PASSWORD,"newPassword":PENDING_PASSWORD}),
        )
        .await
    });
    let outcome=async {
        // The observed statement is after the real extractor and both password jobs.
        wait_for_account_lock(pool,blocker_pid).await?;
        match mutation {
            Mutation::Ban=>{
                let mut accounts=crate::authentication::PostgresAccountTransaction::new(&mut blocker);
                fixture.state.accounts.set_status(&mut accounts,fixture.user,"DISABLED").await?;
            }
            Mutation::Kick=>{
                let mut accounts=crate::authentication::PostgresAccountTransaction::new(&mut blocker);
                fixture.state.accounts.revoke_all(&mut accounts,fixture.user).await?;
            }
            Mutation::Reset=>{
                let mut accounts=crate::authentication::PostgresAccountTransaction::new(&mut blocker);
                fixture.state.accounts.replace_password(&mut accounts,fixture.user,&admin_password,None).await?;
            }
            Mutation::CurrentMember|Mutation::OtherMember=>{
                let id=if mutation==Mutation::CurrentMember { fixture.member } else { fixture.other_member };
                let affected=sqlx::query("UPDATE memberships SET status='DISABLED',identity_version=identity_version+1,updated_at=clock_timestamp() WHERE id=$1 AND user_id=$2")
                    .bind(id).bind(fixture.user).execute(&mut *blocker).await?.rows_affected();
                ensure!(affected==1,"test writer did not revoke its precise member");
            }
        }
        let committed=snapshot(&mut blocker,fixture.user).await?;
        blocker.commit().await?;
        let reply=tokio::time::timeout(Duration::from_secs(30),&mut pending).await
            .map_err(|_|anyhow!("pending legacy password command did not finish after the blocker commit"))???;
        let status=match mutation { Mutation::Reset=>StatusCode::CONFLICT,
            Mutation::OtherMember=>StatusCode::OK,_=>StatusCode::UNAUTHORIZED };
        expect(&reply,status,"legacy command authority after a committed concurrent writer")?;
        let after={
            let mut connection=pool.acquire().await?;
            snapshot(&mut connection,fixture.user).await?
        };
        if mutation!=Mutation::OtherMember {
            ensure!(after==committed,"rejected pending command partially changed account, credentials, audit or notifications");
        }
        let (encoded,version,status):(String,i64,String)=sqlx::query_as("SELECT password_hash,session_version,status FROM users WHERE id=$1")
            .bind(fixture.user).fetch_one(pool).await?;
        let success_audits:i64=sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE user_id=$1 AND action='UPDATE_PASSWORD'")
            .bind(fixture.user).fetch_one(pool).await?;
        ensure!(success_audits==i64::from(mutation==Mutation::OtherMember),"pending legacy command reported an incorrect successful password audit");
        match mutation {
            Mutation::OtherMember=>{
                ensure!(password::verify_password(&encoded,PENDING_PASSWORD) && version==before.1+1,
                        "another member's revocation incorrectly blocked a valid current context");
                let member_state:(String,i64)=sqlx::query_as("SELECT status,identity_version FROM memberships WHERE id=$1")
                    .bind(fixture.member).fetch_one(pool).await?;
                ensure!(member_state==("ENABLED".into(),1),"personal password change rewrote current member identity");
            }
            Mutation::Reset=>{
                ensure!(password::verify_password(&encoded,ADMIN_PASSWORD) && version==before.1+1,
                        "stale command overwrote an administrator reset or added another generation");
            }
            Mutation::CurrentMember=>{
                ensure!(encoded==before.0 && version==before.1,"member-local rejection mutated account credentials");
                expect(&call(&fixture.app,Method::GET,"/api/auth/profile",Some(&fixture.other_access),Value::Null).await?,
                       StatusCode::OK,"other organization context remains independently effective")?;
            }
            Mutation::Ban|Mutation::Kick=>{
                ensure!(encoded==before.0 && version==before.1+1,"rejected stale context performed another credential mutation");
                ensure!(status==if mutation==Mutation::Ban { "DISABLED" } else { "ENABLED" },
                        "pending command changed the committed account status");
                expect(&call(&fixture.app,Method::GET,"/api/auth/profile",Some(&fixture.other_access),Value::Null).await?,
                       StatusCode::UNAUTHORIZED,"global account revocation covers the other organization")?;
            }
        }
        expect(&call(&fixture.app,Method::GET,"/api/auth/profile",Some(&fixture.unrelated_access),Value::Null).await?,
               StatusCode::OK,"unrelated account remains authenticated")?;
        Ok(())
    }.await;
    if !pending.is_finished() {
        pending.abort();
        let _ = pending.await;
    }
    outcome
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE08_TEST_DATABASE_URL and owned database creation"]
async fn phase08_legacy_password_wait_rejects_committed_global_ban() -> Result<()> {
    let db = Database::create().await?;
    let assertions = lock_wait_race(&db.pool, Mutation::Ban).await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE08_TEST_DATABASE_URL and owned database creation"]
async fn phase08_legacy_password_wait_rejects_committed_global_kick() -> Result<()> {
    let db = Database::create().await?;
    let assertions = lock_wait_race(&db.pool, Mutation::Kick).await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE08_TEST_DATABASE_URL and owned database creation"]
async fn phase08_legacy_password_wait_rejects_only_the_revoked_current_member() -> Result<()> {
    let db = Database::create().await?;
    let assertions = lock_wait_race(&db.pool, Mutation::CurrentMember).await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE08_TEST_DATABASE_URL and owned database creation"]
async fn phase08_legacy_password_wait_preserves_administrator_reset_conflict() -> Result<()> {
    let db = Database::create().await?;
    let assertions = lock_wait_race(&db.pool, Mutation::Reset).await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE08_TEST_DATABASE_URL and owned database creation"]
async fn phase08_legacy_password_wait_allows_the_unrevoked_current_context() -> Result<()> {
    let db = Database::create().await?;
    let assertions = lock_wait_race(&db.pool, Mutation::OtherMember).await;
    db.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE08_TEST_DATABASE_URL and owned database creation"]
async fn phase08_generic_administrator_reset_still_supports_a_disabled_account() -> Result<()> {
    let db = Database::create().await?;
    let assertions=async {
        let fixture=Fixture::create(&db.pool).await?;
        expect(&call(&fixture.app,Method::PUT,&format!("/api/users/{}/status",fixture.user),
                     Some(&fixture.admin_access),json!({"status":"DISABLED"})).await?,
               StatusCode::OK,"authorized account disable")?;
        expect(&call(&fixture.app,Method::POST,&format!("/api/users/{}/reset-password",fixture.user),
                     Some(&fixture.admin_access),json!({"newPassword":ADMIN_PASSWORD})).await?,
               StatusCode::OK,"generic administrator reset does not require the target member or session")?;
        let (encoded,version,status):(String,i64,String)=sqlx::query_as("SELECT password_hash,session_version,status FROM users WHERE id=$1")
            .bind(fixture.user).fetch_one(&db.pool).await?;
        ensure!(password::verify_password(&encoded,ADMIN_PASSWORD) && version==2 && status=="DISABLED",
                "authorized disabled-account reset changed status or lost credential ownership");
        let audits:(i64,i64)=sqlx::query_as("SELECT count(*) FILTER(WHERE action='RESET_PASSWORD'),count(*) FILTER(WHERE action='UPDATE_PASSWORD') FROM audit_logs WHERE detail LIKE $1")
            .bind(format!("%{}%",fixture.user)).fetch_one(&db.pool).await?;
        ensure!(audits==(1,0),"administrator reset was confused with the legacy personal command");
        Ok(())
    }.await;
    db.finish(assertions).await
}
