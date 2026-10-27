//! Phase 11 network-resource contracts against explicitly owned fresh databases.
//! Fixtures create provider records; the adapter still executes real live-token
//! proof on every request. Standard authorization-code flows are covered by OIDC.

use std::{
    collections::{BTreeSet, HashMap},
    str::FromStr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::{anyhow, ensure, Result};
use axum::{
    body::{to_bytes, Body},
    http::{header, Method, Request, StatusCode},
    Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool,
};
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    config::Config,
    customer_portal::authority::{AccountProofPort, ProofFuture},
    error::AppError,
    state::AppState,
    util::{hash, uuid7},
};
mod selection;

const ISSUER: &str = "https://phase11-auth.example.invalid";
const PROFILE_SECRET: &str = "phase11-owned-profile-secret-not-production";
const PREFIX: &str = "/internal/customer-portal";

struct Database {
    pool: PgPool,
    admin: PgPool,
    name: String,
}
impl Database {
    async fn create() -> Result<Self> {
        let url = std::env::var("IDENTITY_PHASE11_TEST_DATABASE_URL")
            .map_err(|_| anyhow!("explicit IDENTITY_PHASE11_TEST_DATABASE_URL is required"))?;
        let options = PgConnectOptions::from_str(&url)
            .map_err(|_| anyhow!("invalid isolated Phase 11 database configuration"))?;
        ensure!(
            options.get_database() == Some("postgres"),
            "Phase 11 administration must explicitly select postgres"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("Phase 11 administration connection failed"))?;
        let name = format!("identity_phase11_{}", uuid7::new_uuid().simple());
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await?;
        let pool = match PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(3))
            .connect_with(options.database(&name))
            .await
        {
            Ok(pool) => pool,
            Err(_) => {
                let _ = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
                    .execute(&admin)
                    .await;
                return Err(anyhow!("Phase 11 owned database connection failed"));
            }
        };
        let db = Self { pool, admin, name };
        if let Err(error) = sqlx::migrate!("./migrations").run(&db.pool).await {
            let _ = db.finish(Ok(())).await;
            return Err(error.into());
        }
        if let Err(error) = db.prepare_dual_identity_fixture().await {
            let _ = db.finish(Ok(())).await;
            return Err(error);
        }
        Ok(db)
    }

    async fn prepare_dual_identity_fixture(&self) -> Result<()> {
        let fresh:bool=sqlx::query_scalar("SELECT current_database()=$1 AND NOT EXISTS(SELECT 1 FROM users) AND NOT EXISTS(SELECT 1 FROM memberships) AND (SELECT count(*)=1 AND bool_and(success AND version=1) FROM _sqlx_migrations) AND NOT EXISTS(SELECT 1 FROM pg_constraint WHERE conrelid='public.memberships'::regclass AND conname='memberships_user_id_organization_id_key') AND EXISTS(SELECT 1 FROM pg_constraint WHERE conrelid='public.memberships'::regclass AND conname='memberships_user_organization_type_key' AND contype='u' AND pg_get_constraintdef(oid)='UNIQUE (user_id, organization_id, member_type)')")
            .bind(&self.name).fetch_one(&self.pool).await?;
        ensure!(
            fresh,
            "customer fixture requires the current empty 0001 baseline"
        );
        Ok(())
    }
    async fn finish(self, result: Result<()>) -> Result<()> {
        ensure!(
            self.name.starts_with("identity_phase11_")
                && self.name.len() <= 63
                && self
                    .name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_'),
            "refusing cleanup outside owned Phase 11 database"
        );
        let closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let removed = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await;
        self.admin.close().await;
        eprintln!("identity_phase11_cleanup database={} assertions_ok={} database_removed={} pool_closed={closed}",self.name,result.is_ok(),removed.is_ok());
        result?;
        removed?;
        ensure!(closed, "owned Phase 11 pool did not close");
        Ok(())
    }
}

struct Fixture {
    state: AppState,
    user: Uuid,
    tenant: Uuid,
    org: Uuid,
    customer: Uuid,
    employee: Uuid,
    app: Uuid,
    profile_client: Uuid,
    profile_public: String,
    rp: Uuid,
    sid: Uuid,
    grant: Uuid,
    access: String,
    refresh: String,
}

async fn source(pool: &PgPool, user: Uuid, rp: Uuid, access: &str) -> Result<(Uuid, Uuid)> {
    let sid = uuid7::new_uuid();
    let grant = uuid7::new_uuid();
    sqlx::query("INSERT INTO sso_sessions(id,user_id,token_hash,csrf_hash,account_version,authenticated_at,expires_at) VALUES($1,$2,$3,$4,3,clock_timestamp(),clock_timestamp()+interval '1 hour')")
        .bind(sid).bind(user).bind(hash::sha256_hex(&sid.to_string())).bind(hash::sha256_hex(&format!("csrf-{sid}"))).execute(pool).await?;
    sqlx::query("INSERT INTO oidc_grants(id,client_id,client_policy_version,session_id,user_id,account_version,scope,nonce,auth_time,created_at,expires_at) VALUES($1,$2,1,$3,$4,3,ARRAY['openid','profile','offline_access'],'owned-nonce',clock_timestamp(),clock_timestamp(),clock_timestamp()+interval '30 minutes')")
        .bind(grant).bind(rp).bind(sid).bind(user).execute(pool).await?;
    sqlx::query("INSERT INTO oidc_access_tokens(id,grant_id,token_hash,created_at,expires_at) VALUES($1,$2,$3,clock_timestamp(),clock_timestamp()+interval '5 minutes')")
        .bind(uuid7::new_uuid()).bind(grant).bind(hash::sha256_hex(access)).execute(pool).await?;
    Ok((sid, grant))
}

async fn oidc_client(pool: &PgPool, purpose: &str, kinds: &[&str]) -> Result<Uuid> {
    let id = uuid7::new_uuid();
    sqlx::query("INSERT INTO oidc_clients(id,name,purpose,secret_hash,allowed_member_types,redirect_uris,status,access_policy_version,created_at,updated_at) VALUES($1,'Owned Phase11 RP',$2,$3,$4,ARRAY['https://phase11-customer.example.invalid/callback'],'ENABLED',1,clock_timestamp(),clock_timestamp())")
        .bind(id).bind(purpose).bind(hash::sha256_hex(&"9".repeat(64))).bind(kinds).execute(pool).await?;
    Ok(id)
}

impl Fixture {
    async fn create(pool: &PgPool) -> Result<Self> {
        let user = uuid7::new_uuid();
        let tenant = uuid7::new_uuid();
        let org = uuid7::new_uuid();
        let customer = uuid7::new_uuid();
        let employee = uuid7::new_uuid();
        let app = uuid7::new_uuid();
        let profile_client = uuid7::new_uuid();
        let profile_public = format!("phase11-{}", profile_client.simple());
        sqlx::query("INSERT INTO tenants(id,name,code) VALUES($1,'Owned customer tenant',$2)")
            .bind(tenant)
            .bind(format!("phase11-{}", tenant.simple()))
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO organizations(id,tenant_id,name) VALUES($1,$2,'Owned customer organization')").bind(org).bind(tenant).execute(pool).await?;
        sqlx::query("INSERT INTO users(id,username,password_hash,session_version,is_super_admin,real_name,avatar,email,phone) VALUES($1,$2,'unused-fixture-hash',3,true,'Owned customer','/owned-avatar.png','hidden@example.invalid','hidden-phone')")
            .bind(user).bind(format!("phase11-{}",user.simple())).execute(pool).await?;
        for (id, kind) in [(customer, "CUSTOMER"), (employee, "EMPLOYEE")] {
            sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,identity_version) VALUES($1,$2,$3,$4,5)").bind(id).bind(user).bind(org).bind(kind).execute(pool).await?;
        }
        sqlx::query("INSERT INTO applications(id,name,code,base_path,status,allowed_member_types) VALUES($1,'Owned profile',$2,'/owned/profile','ENABLED',ARRAY['CUSTOMER'])")
            .bind(app).bind(format!("phase11-{}",app.simple())).execute(pool).await?;
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
            .bind(tenant)
            .bind(app)
            .execute(pool)
            .await?;
        sqlx::query(
            "INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)",
        )
        .bind(org)
        .bind(app)
        .execute(pool)
        .await?;
        sqlx::query("INSERT INTO oauth_clients(id,client_id,client_name,client_type,client_secret_hash,grant_types,scopes,subject_mode,allowed_member_types,application_id,status) VALUES($1,$2,'Owned profile port','CONFIDENTIAL',$3,'authorization_code','profile','USER',ARRAY['CUSTOMER'],$4,'ENABLED')")
            .bind(profile_client).bind(&profile_public).bind(hash::sha256_hex(PROFILE_SECRET)).bind(app).execute(pool).await?;
        sqlx::query("INSERT INTO oidc_provider_settings(singleton,issuer,created_at) VALUES(true,$1,clock_timestamp())").bind(ISSUER).execute(pool).await?;
        let rp = oidc_client(pool, "CUSTOMER_PORTAL", &["CUSTOMER"]).await?;
        let access = "a".repeat(64);
        let refresh = "b".repeat(64);
        let (sid, grant) = source(pool, user, rp, &access).await?;
        sqlx::query("INSERT INTO oidc_refresh_tokens(id,grant_id,token_hash,created_at,expires_at) VALUES($1,$2,$3,clock_timestamp(),clock_timestamp()+interval '20 minutes')")
            .bind(uuid7::new_uuid()).bind(grant).bind(hash::sha256_hex(&refresh)).execute(pool).await?;
        let values = HashMap::from([
            ("APP_ENV".into(), "test".into()),
            (
                "JWT_SECRET".into(),
                "phase11-test-signing-secret-not-production".into(),
            ),
        ]);
        let mut config = Config::from_values(&values)?;
        config.customer_profile_client_ref = Some(profile_client);
        config.customer_profile_application_id = Some(app);
        config.customer_portal_port_enabled = true;
        config.customer_portal_oidc_client_id = Some(rp);
        config.oidc_enabled = true;
        config.oidc_issuer = Some(ISSUER.into());
        let state = AppState::new(pool.clone(), Arc::new(config));
        Ok(Self {
            state,
            user,
            tenant,
            org,
            customer,
            employee,
            app,
            profile_client,
            profile_public,
            rp,
            sid,
            grant,
            access,
            refresh,
        })
    }
    fn router(&self) -> Router {
        crate::customer_portal::routes().with_state(self.state.clone())
    }
    fn basic(&self) -> String {
        format!(
            "Basic {}",
            STANDARD.encode(format!("{}:{PROFILE_SECRET}", self.profile_public))
        )
    }
    async fn call(&self, path: &str, body: Value) -> Result<Reply> {
        request(&self.router(), path, Some(self.basic()), body).await
    }
    async fn select(&self) -> Result<String> {
        let reply=self.call("context",json!({"oidcAccessToken":self.access,"membershipId":self.customer,"organizationId":self.org,"expectedIdentityVersion":"5"})).await?;
        expect(&reply, StatusCode::OK, "customer selection")?;
        reply.body["customerContext"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("missing derived customer context"))
    }
    async fn profile(&self, token: &str) -> Result<Reply> {
        self.call(
            "profile",
            json!({"oidcAccessToken":self.access,"customerContext":token}),
        )
        .await
    }
}

struct Reply {
    status: StatusCode,
    body: Value,
}
async fn request(
    router: &Router,
    path: &str,
    authorization: Option<String>,
    body: Value,
) -> Result<Reply> {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri(format!("{PREFIX}/{path}"))
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(value) = authorization {
        request = request.header(header::AUTHORIZATION, value);
    }
    let response = tokio::time::timeout(
        Duration::from_secs(10),
        router
            .clone()
            .oneshot(request.body(Body::from(body.to_string()))?),
    )
    .await
    .map_err(|_| anyhow!("Phase11 request timed out"))??;
    let status = response.status();
    ensure!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            == Some("no-store"),
        "private resource response omitted no-store"
    );
    ensure!(
        response
            .headers()
            .get("referrer-policy")
            .and_then(|value| value.to_str().ok())
            == Some("no-referrer"),
        "private resource response omitted no-referrer"
    );
    let bytes = to_bytes(response.into_body(), 65536).await?;
    Ok(Reply {
        status,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    })
}
fn expect(reply: &Reply, expected: StatusCode, label: &str) -> Result<()> {
    ensure!(
        reply.status == expected,
        "{label}: expected {expected}, received {}",
        reply.status
    );
    Ok(())
}
fn keys(value: &Value, expected: &[&str]) -> Result<()> {
    let actual = value
        .as_object()
        .ok_or_else(|| anyhow!("expected exact DTO object"))?
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    ensure!(
        actual == expected.iter().copied().collect(),
        "customer DTO changed its exact fields"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE11_TEST_DATABASE_URL"]
async fn phase11_customer_portal_selects_exact_customer_and_returns_only_self_profile() -> Result<()>
{
    let db = Database::create().await?;
    let result=async {
        let f=Fixture::create(&db.pool).await?;
        let reply=f.call("memberships",json!({"oidcAccessToken":f.access})).await?;expect(&reply,StatusCode::OK,"memberships")?;
        keys(&reply.body,&["contractVersion","account","memberships","source"])?;
        ensure!(reply.body["memberships"].as_array().is_some_and(|members|members.len()==1),"employee was merged into customer choices");
        let choice=&reply.body["memberships"][0];keys(choice,&["membershipId","userId","memberType","identityVersion","organizationId","organizationName","tenantId","tenantName"])?;
        ensure!(choice["membershipId"]==f.customer.to_string() && choice["memberType"]=="CUSTOMER" && choice["identityVersion"]=="5","wrong exact customer identity");
        keys(&reply.body["source"],&["issuer","clientId","grantId","globalSessionId","userId","accountVersion","clientPolicyVersion","authTime","expiresAt"])?;
        let token=f.select().await?;ensure!(token.starts_with("cpc_")&&token.len()==68,"derived context encoding changed");
        let reply=f.profile(&token).await?;expect(&reply,StatusCode::OK,"self profile")?;
        keys(&reply.body,&["context","account"])?;keys(&reply.body["account"],&["id","username","realName","avatar"])?;
        keys(&reply.body["context"],&["contractVersion","purpose","userId","membershipId","memberType","identityVersion","accountVersion","tenant","organization","application"])?;
        ensure!(reply.body["account"]["id"]==f.user.to_string() && reply.body["context"]["membershipId"]==f.customer.to_string() && reply.body["context"]["tenant"]["id"]==f.tenant.to_string(),"profile changed subject or exact relationship");
        let stored:bool=sqlx::query_scalar("SELECT token_hash=$1 AND expires_at<=source_expires_at FROM customer_portal_contexts WHERE membership_id=$2").bind(hash::sha256_hex(&token)).bind(f.customer).fetch_one(&db.pool).await?;
        ensure!(stored,"context did not preserve hash/deadline boundary");
        for path in ["revoke","revoke"] {let reply=f.call(path,json!({"oidcAccessToken":f.access,"customerContext":token})).await?;expect(&reply,StatusCode::OK,"idempotent exact context revoke")?;}
        expect(&f.profile(&token).await?,StatusCode::UNAUTHORIZED,"revoked context")?;
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE11_TEST_DATABASE_URL"]
async fn phase11_customer_portal_rejects_wrong_member_org_generation_source_and_token_kind(
) -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let f=Fixture::create(&db.pool).await?;
        let foreign_user=uuid7::new_uuid();let foreign_member=uuid7::new_uuid();
        sqlx::query("INSERT INTO users(id,username,password_hash,session_version) VALUES($1,$2,'unused-hash',3)").bind(foreign_user).bind(format!("foreign-{}",foreign_user.simple())).execute(&db.pool).await?;
        sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,identity_version) VALUES($1,$2,$3,'CUSTOMER',5)").bind(foreign_member).bind(foreign_user).bind(f.org).execute(&db.pool).await?;
        for (member,org,version) in [(f.employee,f.org,"5"),(foreign_member,f.org,"5"),(uuid7::new_uuid(),f.org,"5"),(f.customer,uuid7::new_uuid(),"5"),(f.customer,f.org,"4")] {
            expect(&f.call("context",json!({"oidcAccessToken":f.access,"membershipId":member,"organizationId":org,"expectedIdentityVersion":version})).await?,StatusCode::FORBIDDEN,"invalid exact selection")?;
        }
        for value in [&f.refresh,"ID.TOKEN.VALUE","Bearer token"] {expect(&f.call("memberships",json!({"oidcAccessToken":value})).await?,StatusCode::UNAUTHORIZED,"wrong token kind")?;}
        let other_rp=oidc_client(&db.pool,"CUSTOMER_PORTAL",&["CUSTOMER"]).await?;let other_access="c".repeat(64);source(&db.pool,f.user,other_rp,&other_access).await?;
        expect(&f.call("memberships",json!({"oidcAccessToken":other_access})).await?,StatusCode::UNAUTHORIZED,"another real customer RP")?;
        let token=f.select().await?;let same_rp_other_grant="d".repeat(64);source(&db.pool,f.user,f.rp,&same_rp_other_grant).await?;
        expect(&f.call("profile",json!({"oidcAccessToken":same_rp_other_grant,"customerContext":token})).await?,StatusCode::UNAUTHORIZED,"another grant of same user RP")?;
        expect(&f.call("revoke",json!({"oidcAccessToken":same_rp_other_grant,"customerContext":token})).await?,StatusCode::UNAUTHORIZED,"wrong source cannot revoke context")?;
        expect(&f.profile(&token).await?,StatusCode::OK,"failed foreign revoke preserved original")?;
        for path in ["memberships?userId=forged","profile?membershipId=forged"] {expect(&f.call(path,json!({"oidcAccessToken":f.access,"customerContext":token})).await?,StatusCode::BAD_REQUEST,"query subject override")?;}
        expect(&request(&f.router(),"memberships",Some(format!("Bearer {}",f.access)),json!({"oidcAccessToken":f.access})).await?,StatusCode::UNAUTHORIZED,"business Bearer cannot authenticate internal caller")?;
        expect(&request(&f.router(),"memberships",Some(format!("Basic {}",STANDARD.encode(format!("{}:wrong",f.profile_public)))),json!({"oidcAccessToken":f.access})).await?,StatusCode::UNAUTHORIZED,"wrong profile client secret")?;
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE11_TEST_DATABASE_URL"]
async fn phase11_customer_portal_member_revocation_is_exact_and_global_source_revocation_is_shared(
) -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let f=Fixture::create(&db.pool).await?;let token=f.select().await?;
        sqlx::query("UPDATE memberships SET status='DISABLED',identity_version=identity_version+1 WHERE id=$1").bind(f.employee).execute(&db.pool).await?;
        expect(&f.profile(&token).await?,StatusCode::OK,"employee-only disable leaves customer valid")?;
        let employee_active:bool=sqlx::query_scalar("SELECT identity_binding_active($1,$2,$3,3,$4,'EMPLOYEE',identity_version) FROM memberships WHERE id=$4")
            .bind(f.user).bind(f.tenant).bind(f.org).bind(f.employee).fetch_one(&db.pool).await?;ensure!(!employee_active,"employee disable did not retire only employee proof");
        sqlx::query("UPDATE memberships SET status='ENABLED',identity_version=identity_version+1 WHERE id=$1").bind(f.employee).execute(&db.pool).await?;
        sqlx::query("UPDATE memberships SET status='DISABLED',identity_version=identity_version+1 WHERE id=$1").bind(f.customer).execute(&db.pool).await?;
        expect(&f.profile(&token).await?,StatusCode::UNAUTHORIZED,"customer-only disable")?;
        let employee_active:bool=sqlx::query_scalar("SELECT identity_binding_active($1,$2,$3,3,$4,'EMPLOYEE',identity_version) FROM memberships WHERE id=$4")
            .bind(f.user).bind(f.tenant).bind(f.org).bind(f.employee).fetch_one(&db.pool).await?;ensure!(employee_active,"customer revocation broke valid same-org employee");
        let choices=f.call("memberships",json!({"oidcAccessToken":f.access})).await?;expect(&choices,StatusCode::OK,"employee-only account remains authenticated")?;
        ensure!(choices.body["memberships"].as_array().is_some_and(Vec::is_empty),"disabled customer was auto-created or employee was substituted");
        sqlx::query("UPDATE users SET session_version=session_version+1 WHERE id=$1").bind(f.user).execute(&db.pool).await?;
        expect(&f.call("memberships",json!({"oidcAccessToken":f.access})).await?,StatusCode::UNAUTHORIZED,"global account generation revokes source")?;
        let revoked:bool=sqlx::query_scalar("SELECT s.revoked_at IS NOT NULL AND g.revoked_at IS NOT NULL FROM sso_sessions s JOIN oidc_grants g ON g.session_id=s.id WHERE s.id=$1 AND g.id=$2").bind(f.sid).bind(f.grant).fetch_one(&db.pool).await?;
        ensure!(revoked,"account-wide logout did not revoke SSO and grant");
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE11_TEST_DATABASE_URL"]
async fn phase11_customer_portal_rechecks_client_app_scope_and_source_policy() -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let f=Fixture::create(&db.pool).await?;let token=f.select().await?;
        sqlx::query("DELETE FROM organization_applications WHERE organization_id=$1 AND application_id=$2").bind(f.org).bind(f.app).execute(&db.pool).await?;
        expect(&f.profile(&token).await?,StatusCode::UNAUTHORIZED,"organization application cap removed")?;
        sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)").bind(f.org).bind(f.app).execute(&db.pool).await?;
        expect(&f.profile(&token).await?,StatusCode::OK,"unchanged generation restored application grant")?;
        sqlx::query("UPDATE applications SET allowed_member_types=ARRAY['EMPLOYEE'] WHERE id=$1").bind(f.app).execute(&db.pool).await?;
        expect(&f.profile(&token).await?,StatusCode::UNAUTHORIZED,"employee-only app")?;
        sqlx::query("UPDATE applications SET allowed_member_types=ARRAY['CUSTOMER'] WHERE id=$1").bind(f.app).execute(&db.pool).await?;
        expect(&f.profile(&token).await?,StatusCode::UNAUTHORIZED,"restoring app cannot repair old policy version")?;
        let token=f.select().await?;
        let mut wrong_registration=f.state.clone();
        let mut wrong_config=(*wrong_registration.config).clone();
        wrong_config.customer_profile_application_id=Some(uuid7::new_uuid());
        wrong_registration.config=Arc::new(wrong_config);
        expect(&request(&crate::customer_portal::routes().with_state(wrong_registration),"memberships",Some(f.basic()),json!({"oidcAccessToken":f.access})).await?,StatusCode::UNAUTHORIZED,"wrong fixed server application registration")?;
        sqlx::query("UPDATE oauth_clients SET client_type='PUBLIC' WHERE id=$1").bind(f.profile_client).execute(&db.pool).await?;
        expect(&f.profile(&token).await?,StatusCode::UNAUTHORIZED,"public client cannot authenticate portal backend")?;
        sqlx::query("UPDATE oauth_clients SET client_type='CONFIDENTIAL',allowed_member_types=ARRAY['EMPLOYEE'] WHERE id=$1").bind(f.profile_client).execute(&db.pool).await?;
        expect(&f.profile(&token).await?,StatusCode::UNAUTHORIZED,"employee-only profile client")?;
        sqlx::query("UPDATE oauth_clients SET allowed_member_types=ARRAY['CUSTOMER'] WHERE id=$1").bind(f.profile_client).execute(&db.pool).await?;
        expect(&f.profile(&token).await?,StatusCode::UNAUTHORIZED,"restoring profile client cannot repair original policy")?;
        let token=f.select().await?;
        sqlx::query("UPDATE oauth_clients SET scopes='other' WHERE id=$1").bind(f.profile_client).execute(&db.pool).await?;
        expect(&f.profile(&token).await?,StatusCode::UNAUTHORIZED,"profile scope removed")?;
        sqlx::query("UPDATE oauth_clients SET scopes='profile' WHERE id=$1").bind(f.profile_client).execute(&db.pool).await?;
        sqlx::query("UPDATE oidc_clients SET status='DISABLED',access_policy_version=access_policy_version+1 WHERE id=$1").bind(f.rp).execute(&db.pool).await?;
        expect(&f.call("memberships",json!({"oidcAccessToken":f.access})).await?,StatusCode::UNAUTHORIZED,"source RP policy changed")?;
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE11_TEST_DATABASE_URL"]
async fn phase11_customer_portal_expiry_and_grant_revocation_never_repair_old_source() -> Result<()>
{
    let db = Database::create().await?;
    let result=async {
        let f=Fixture::create(&db.pool).await?;let token=f.select().await?;
        sqlx::query("UPDATE oidc_access_tokens SET created_at=clock_timestamp()-interval '2 minutes',expires_at=clock_timestamp()-interval '1 second' WHERE token_hash=$1")
            .bind(hash::sha256_hex(&f.access)).execute(&db.pool).await?;
        expect(&f.profile(&token).await?,StatusCode::UNAUTHORIZED,"expired upstream access")?;
        sqlx::query("UPDATE oidc_access_tokens SET expires_at=clock_timestamp()+interval '5 minutes' WHERE token_hash=$1")
            .bind(hash::sha256_hex(&f.access)).execute(&db.pool).await?;
        expect(&f.profile(&token).await?,StatusCode::UNAUTHORIZED,"changed source deadline cannot repair old context")?;
        let token=f.select().await?;
        sqlx::query("UPDATE oidc_grants SET revoked_at=clock_timestamp() WHERE id=$1").bind(f.grant).execute(&db.pool).await?;
        expect(&f.profile(&token).await?,StatusCode::UNAUTHORIZED,"explicit RP grant revocation")?;
        Ok(())
    }.await;
    db.finish(result).await
}

struct UnavailableAuthority;
impl AccountProofPort for UnavailableAuthority {
    fn verify_access<'a>(&'a self, _access: &'a str, _client: Uuid) -> ProofFuture<'a> {
        Box::pin(async { Err(AppError::Unavailable) })
    }
}
struct PendingAuthority;
impl AccountProofPort for PendingAuthority {
    fn verify_access<'a>(&'a self, _access: &'a str, _client: Uuid) -> ProofFuture<'a> {
        Box::pin(std::future::pending())
    }
}
struct RevokeAfterProof {
    inner: Arc<dyn AccountProofPort>,
    pool: PgPool,
    member: Uuid,
    calls: AtomicUsize,
}

struct RevokeSourceAfterProof {
    inner: Arc<dyn AccountProofPort>,
    pool: PgPool,
    grant: Uuid,
    calls: AtomicUsize,
}
impl AccountProofPort for RevokeSourceAfterProof {
    fn verify_access<'a>(&'a self, access: &'a str, client: Uuid) -> ProofFuture<'a> {
        Box::pin(async move {
            let proof = self.inner.verify_access(access, client).await?;
            // Revoke immediately after the second live proof read. The final
            // projection must reject that once-valid response before publish.
            if self.calls.fetch_add(1, Ordering::SeqCst) == 1 {
                sqlx::query("UPDATE oidc_grants SET revoked_at=clock_timestamp() WHERE id=$1")
                    .bind(self.grant)
                    .execute(&self.pool)
                    .await
                    .map_err(|_| AppError::Unavailable)?;
            }
            Ok(proof)
        })
    }
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE11_TEST_DATABASE_URL"]
async fn phase11_customer_portal_final_projection_rejects_source_revoked_after_online_proof(
) -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let mut f=Fixture::create(&db.pool).await?;
        let token=f.select().await?;
        let original=f.state.customer_portal_authority.clone();
        for operation in ["profile","context","memberships"] {
            // Explicit test-owner restoration provides a fresh identical
            // fixture for each boundary; production has no such operation.
            sqlx::query("UPDATE oidc_grants SET revoked_at=NULL WHERE id=$1").bind(f.grant).execute(&db.pool).await?;
            f.state.customer_portal_authority=Arc::new(RevokeSourceAfterProof{inner:original.clone(),pool:db.pool.clone(),grant:f.grant,calls:AtomicUsize::new(0)});
            let body=match operation {
                "profile"=>json!({"oidcAccessToken":f.access,"customerContext":token}),
                "context"=>json!({"oidcAccessToken":f.access,"membershipId":f.customer,"organizationId":f.org,"expectedIdentityVersion":"5"}),
                _=>json!({"oidcAccessToken":f.access}),
            };
            expect(&f.call(operation,body).await?,StatusCode::UNAUTHORIZED,"final source projection after second online proof")?;
        }
        Ok(())
    }.await;
    db.finish(result).await
}
impl AccountProofPort for RevokeAfterProof {
    fn verify_access<'a>(&'a self, access: &'a str, client: Uuid) -> ProofFuture<'a> {
        Box::pin(async move {
            let proof = self.inner.verify_access(access, client).await?;
            if self.calls.fetch_add(1, Ordering::SeqCst) == 1 {
                sqlx::query("UPDATE memberships SET status='DISABLED',identity_version=identity_version+1 WHERE id=$1")
                    .bind(self.member)
                    .execute(&self.pool)
                    .await
                    .map_err(|_| AppError::Unavailable)?;
            }
            Ok(proof)
        })
    }
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE11_TEST_DATABASE_URL"]
async fn phase11_customer_portal_fails_closed_on_authority_database_and_post_read_revocation(
) -> Result<()> {
    let db = Database::create().await?;
    let result = async {
        let mut f = Fixture::create(&db.pool).await?;
        let token = f.select().await?;
        let original = f.state.customer_portal_authority.clone();
        f.state.customer_portal_authority = Arc::new(UnavailableAuthority);
        expect(
            &f.profile(&token).await?,
            StatusCode::SERVICE_UNAVAILABLE,
            "authority unavailable cannot reuse context",
        )?;
        f.state.customer_portal_authority = Arc::new(PendingAuthority);
        let started = std::time::Instant::now();
        expect(
            &f.profile(&token).await?,
            StatusCode::SERVICE_UNAVAILABLE,
            "shared request budget includes unresponsive authority",
        )?;
        ensure!(
            started.elapsed() < Duration::from_secs(5),
            "shared request budget was not bounded"
        );
        f.state.customer_portal_authority = Arc::new(RevokeAfterProof {
            inner: original.clone(),
            pool: db.pool.clone(),
            member: f.customer,
            calls: AtomicUsize::new(0),
        });
        expect(
            &f.profile(&token).await?,
            StatusCode::UNAUTHORIZED,
            "revocation between profile read and final authority proof",
        )?;
        f.state.customer_portal_authority = original;
        sqlx::query(
            "ALTER TABLE customer_portal_contexts RENAME TO customer_portal_contexts_unavailable",
        )
        .execute(&db.pool)
        .await?;
        let reply = f.profile(&token).await?;
        sqlx::query(
            "ALTER TABLE customer_portal_contexts_unavailable RENAME TO customer_portal_contexts",
        )
        .execute(&db.pool)
        .await?;
        expect(
            &reply,
            StatusCode::SERVICE_UNAVAILABLE,
            "database read failure",
        )?;
        let slots = f
            .state
            .customer_portal_slots
            .clone()
            .acquire_many_owned(16)
            .await?;
        expect(
            &f.call("memberships", json!({"oidcAccessToken":f.access}))
                .await?,
            StatusCode::SERVICE_UNAVAILABLE,
            "bounded authority budget",
        )?;
        drop(slots);
        Ok(())
    }
    .await;
    db.finish(result).await
}
