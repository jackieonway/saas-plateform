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
use crate::util::{hash, uuid7::new_uuid};

const SECRET: &str = "identity-phase02-owned-fixture-signing-key";

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
    let state = crate::sso_test_fixture::state(pool.clone(), Arc::new(config));
    let mut routes = crate::routes::api_router();
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
    sqlx::query("INSERT INTO users(id,username,password_hash,is_super_admin,session_version) VALUES($1,$2,'phase02-unusable-fixture-password',$3,1)")
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

#[derive(Clone)]
struct Identity {
    user: Uuid,
    tenant: Uuid,
    org: Uuid,
    member: Uuid,
    identity_version: i64,
    kind: &'static str,
}
impl Identity {
    async fn create(
        pool: &PgPool,
        user: Uuid,
        tenant: Uuid,
        org: Uuid,
        kind: &'static str,
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
        })
    }
    async fn active(&self, pool: &PgPool) -> Result<bool> {
        let version: i64 = sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
            .bind(self.user)
            .fetch_one(pool)
            .await?;
        Ok(
            sqlx::query_scalar("SELECT identity_binding_active($1,$2,$3,$4,$5,$6,$7)")
                .bind(self.user)
                .bind(self.tenant)
                .bind(self.org)
                .bind(version)
                .bind(self.member)
                .bind(self.kind)
                .bind(self.identity_version)
                .fetch_one(pool)
                .await?,
        )
    }
}
struct BoundSession {
    sid: Uuid,
    bearer: String,
}
async fn bound_session(pool: &PgPool, identity: &Identity) -> Result<BoundSession> {
    ensure!(
        identity.kind == "EMPLOYEE",
        "platform sessions require an employee identity"
    );
    let bearer =
        crate::sso_test_fixture::token(pool, identity.user, identity.tenant, identity.org, SECRET)
            .await?;
    let claims = crate::util::jwt::verify_internal(
        &bearer,
        SECRET,
        crate::util::jwt::InternalUse::PlatformSession,
        Some(crate::util::jwt::PLATFORM_API_AUDIENCE),
    )?;
    let sid = Uuid::parse_str(
        claims
            .sid
            .as_deref()
            .ok_or_else(|| anyhow!("fixture session id missing"))?,
    )?;
    Ok(BoundSession { sid, bearer })
}
async fn session_active(pool: &PgPool, sid: Uuid) -> Result<bool> {
    Ok(sqlx::query_scalar("SELECT platform_session_active($1)")
        .bind(sid)
        .fetch_one(pool)
        .await?)
}
async fn profile(app: &Router, session: &BoundSession) -> Result<Reply> {
    json_request(
        app,
        Method::GET,
        "/api/auth/profile",
        Some(&session.bearer),
        Value::Null,
    )
    .await
}
async fn target_application(pool: &PgPool, tenant: Uuid, org: Uuid, code: &str) -> Result<Uuid> {
    let id = new_uuid();
    sqlx::query("INSERT INTO applications(id,name,code,allowed_member_types) VALUES($1,$2,$2,ARRAY['EMPLOYEE'])")
        .bind(id).bind(code).execute(pool).await?;
    sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
        .bind(tenant)
        .bind(id)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)",
    )
    .bind(org)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(id)
}

#[tokio::test]
#[ignore = "creates and drops identity_phase02_* databases using explicit IDENTITY_PHASE02_TEST_DATABASE_URL"]
async fn identity_binding_canonical_exact_member_revocation() -> Result<()> {
    let db = Database::create(i64::MAX).await?;
    let result=async {
        let (tenant,org)=tenant(&db.pool,"canonical-member").await?;
        let other_org=organization(&db.pool,tenant,Some(org),"canonical-other-org").await?;
        let subject=user(&db.pool,"canonical-dual-user",false).await?;
        let mut employee=Identity::create(&db.pool,subject,tenant,org,"EMPLOYEE").await?;
        let customer=Identity::create(&db.pool,subject,tenant,org,"CUSTOMER").await?;
        let other=Identity::create(&db.pool,subject,tenant,other_org,"EMPLOYEE").await?;
        let first=bound_session(&db.pool,&employee).await?;
        let independent=bound_session(&db.pool,&other).await?;
        let app=app(&db.pool,None)?;
        ensure!(employee.active(&db.pool).await? && customer.active(&db.pool).await? && other.active(&db.pool).await?,"current member tuples must be active");
        expect(&profile(&app,&first).await?,StatusCode::OK,"canonical employee profile")?;
        let version:i64=sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1").bind(subject).fetch_one(&db.pool).await?;
        for (subject_ref,tenant_ref,org_ref,member,kind,account_version,identity_version) in [
            (new_uuid(),tenant,org,employee.member,"EMPLOYEE",version,1_i64),
            (subject,new_uuid(),org,employee.member,"EMPLOYEE",version,1),
            (subject,tenant,other_org,employee.member,"EMPLOYEE",version,1),
            (subject,tenant,org,other.member,"EMPLOYEE",version,1),
            (subject,tenant,org,employee.member,"CUSTOMER",version,1),
            (subject,tenant,org,employee.member,"EMPLOYEE",version+1,1),
            (subject,tenant,org,employee.member,"EMPLOYEE",version,2),
        ] {
            let accepted:bool=sqlx::query_scalar("SELECT identity_binding_active($1,$2,$3,$4,$5,$6,$7)")
                .bind(subject_ref).bind(tenant_ref).bind(org_ref).bind(account_version).bind(member).bind(kind).bind(identity_version).fetch_one(&db.pool).await?;
            ensure!(!accepted,"mismatched identity tuple was accepted");
        }
        // Exercise the canonical member-generation trigger without creating a
        // second authorization server or changing any identity-service grant.
        sqlx::query("UPDATE memberships SET status='DISABLED',identity_version=identity_version+1 WHERE id=$1")
            .bind(employee.member).execute(&db.pool).await?;
        ensure!(!session_active(&db.pool,first.sid).await? && session_active(&db.pool,independent.sid).await? && customer.active(&db.pool).await?,"employee departure crossed member boundary");
        expect(&profile(&app,&first).await?,StatusCode::UNAUTHORIZED,"retired employee profile")?;
        sqlx::query("UPDATE memberships SET status='ENABLED',identity_version=identity_version+1 WHERE id=$1")
            .bind(employee.member).execute(&db.pool).await?;
        employee.identity_version=3;
        ensure!(employee.active(&db.pool).await? && !session_active(&db.pool,first.sid).await?,"reenabling restored a retired session");
        let returned=bound_session(&db.pool,&employee).await?;
        expect(&profile(&app,&returned).await?,StatusCode::OK,"fresh employee generation")?;
        sqlx::query("DELETE FROM memberships WHERE id=$1").bind(customer.member).execute(&db.pool).await?;
        ensure!(!customer.active(&db.pool).await? && session_active(&db.pool,returned.sid).await? && session_active(&db.pool,independent.sid).await?,"customer removal crossed employee boundary");
        let retained:Uuid=sqlx::query_scalar("SELECT membership_id FROM platform_sessions WHERE id=$1").bind(first.sid).fetch_one(&db.pool).await?;
        ensure!(retained==employee.member,"retired context lost its original member binding");
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops identity_phase02_* databases using explicit IDENTITY_PHASE02_TEST_DATABASE_URL"]
async fn identity_binding_canonical_account_and_source_versions() -> Result<()> {
    let db = Database::create(i64::MAX).await?;
    let result=async {
        let (tenant,org)=tenant(&db.pool,"canonical-version").await?;
        let subject=user(&db.pool,"canonical-version-user",false).await?;
        let unrelated=user(&db.pool,"canonical-version-other",false).await?;
        let identity=Identity::create(&db.pool,subject,tenant,org,"EMPLOYEE").await?;
        let independent=Identity::create(&db.pool,unrelated,tenant,org,"EMPLOYEE").await?;
        let old=bound_session(&db.pool,&identity).await?;
        let unaffected=bound_session(&db.pool,&independent).await?;
        let app=app(&db.pool,None)?;
        expect(&profile(&app,&old).await?,StatusCode::OK,"current source before account change")?;
        sqlx::query("UPDATE users SET session_version=session_version+1 WHERE id=$1").bind(subject).execute(&db.pool).await?;
        ensure!(!session_active(&db.pool,old.sid).await? && session_active(&db.pool,unaffected.sid).await?,"account generation crossed another account");
        expect(&profile(&app,&old).await?,StatusCode::UNAUTHORIZED,"old account generation")?;
        let fresh=bound_session(&db.pool,&identity).await?;
        expect(&profile(&app,&fresh).await?,StatusCode::OK,"new account generation")?;
        let client:Uuid=sqlx::query_scalar("SELECT b.oidc_client_id FROM platform_sessions s JOIN bff_sessions b ON b.id=s.bff_session_id WHERE s.id=$1")
            .bind(fresh.sid).fetch_one(&db.pool).await?;
        sqlx::query("UPDATE auth_clients SET policy_version=policy_version+1,version=version+1 WHERE id=$1").bind(client).execute(&db.pool).await?;
        ensure!(!session_active(&db.pool,fresh.sid).await? && !session_active(&db.pool,unaffected.sid).await?,"client policy failed to close its original sources");
        expect(&profile(&app,&fresh).await?,StatusCode::UNAUTHORIZED,"retired canonical client policy")?;
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops identity_phase02_* databases using explicit IDENTITY_PHASE02_TEST_DATABASE_URL"]
async fn identity_binding_canonical_application_capability_isolation() -> Result<()> {
    let db = Database::create(i64::MAX).await?;
    let result=async {
        let (tenant,org)=tenant(&db.pool,"canonical-app").await?;
        let subject=user(&db.pool,"canonical-app-user",false).await?;
        let identity=Identity::create(&db.pool,subject,tenant,org,"EMPLOYEE").await?;
        let session=bound_session(&db.pool,&identity).await?;
        let a=target_application(&db.pool,tenant,org,"canonical-app-a").await?;
        let b=target_application(&db.pool,tenant,org,"canonical-app-b").await?;
        let first=crate::sso_test_fixture::app_capability(&db.pool,subject,tenant,org,a,"canonical-app-a",SECRET).await?;
        let second=crate::sso_test_fixture::app_capability(&db.pool,subject,tenant,org,b,"canonical-app-b",SECRET).await?;
        ensure!(crate::util::jwt::verify_internal(&first,SECRET,crate::util::jwt::InternalUse::AppSession,Some("canonical-app-b")).is_err(),
            "one application capability satisfied a different audience");
        let app=app(&db.pool,None)?;
        expect(&json_request(&app,Method::GET,"/api/auth/profile",Some(&first),Value::Null).await?,StatusCode::UNAUTHORIZED,"app capability cannot become a console session")?;
        let pool=&db.pool;
        let active=|token:String| async move { sqlx::query_scalar::<_,bool>("SELECT platform_app_capability_active($1)").bind(hash::sha256_hex(&token)).fetch_one(pool).await };
        ensure!(active(first.clone()).await? && active(second.clone()).await?,"fresh application capabilities were inactive");
        sqlx::query("UPDATE applications SET allowed_member_types=ARRAY['CUSTOMER'],access_policy_version=access_policy_version+1 WHERE id=$1")
            .bind(a).execute(&db.pool).await?;
        ensure!(!active(first).await? && active(second).await?,"application reduction crossed another application");
        ensure!(session_active(&db.pool,session.sid).await?,"application reduction revoked an account session");
        expect(&profile(&app,&session).await?,StatusCode::OK,"console source survives application reduction")?;
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops identity_phase02_* databases using explicit IDENTITY_PHASE02_TEST_DATABASE_URL"]
async fn identity_binding_canonical_authority_outage_does_not_execute_business() -> Result<()> {
    let db = Database::create(i64::MAX).await?;
    let result = async {
        let (tenant, org) = tenant(&db.pool, "canonical-outage").await?;
        let subject = user(&db.pool, "canonical-outage-user", false).await?;
        let identity = Identity::create(&db.pool, subject, tenant, org, "EMPLOYEE").await?;
        let session = bound_session(&db.pool, &identity).await?;
        let probe = Arc::new(AtomicUsize::new(0));
        let app = app(&db.pool, Some(probe.clone()))?;
        expect(
            &json_request(
                &app,
                Method::POST,
                "/identity-phase02/protected-write",
                Some(&session.bearer),
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
                Some(&session.bearer),
                json!({}),
            )
            .await?,
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable authority",
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
            "authority failure committed a business audit"
        );
        independent.close().await;
        Ok(())
    }
    .await;
    db.finish(result).await
}
