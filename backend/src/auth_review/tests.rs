//! Real canonical PostgreSQL coverage for the platform-owned intake service.
//! Each ignored test creates and drops one UUID database on an explicitly named
//! loopback cluster. MFA rows are fixtures, not evidence of human TOTP login.
//! No actor/source function is replaced and no remote synchronization is run.
use super::{policy, service, store};
use crate::{error::AppError, extractors::CurrentUser, state::AppState, util::uuid7};
use anyhow::{anyhow, ensure, Context, Result};
use axum::http::HeaderMap;
use serde_json::{json, Value};
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool,
};
use std::{collections::HashMap, future::Future, str::FromStr, sync::Arc, time::Duration};
use uuid::Uuid;

const ISSUER: &str = "https://auth.saas.test";
const PREFIX: &str = "auth_review_phase19_";
const FINE: &[&str] = &[
    "system:oauth:view",
    "system:oauth:register",
    "system:oauth:review",
    "system:oauth:publish",
    "system:oauth:rotate",
    "system:oauth:disable",
    "system:oauth:scopes",
    "system:oauth:invite",
    "system:oauth:assign",
];

struct Database {
    pool: PgPool,
    admin: PgPool,
    name: String,
}
impl Database {
    async fn new() -> Result<Self> {
        let raw = std::env::var("IDENTITY_PHASE19_TEST_DATABASE_URL")
            .context("explicit IDENTITY_PHASE19_TEST_DATABASE_URL required")?;
        let expected = std::env::var("IDENTITY_PHASE19_TEST_CLUSTER_ID")
            .context("explicit IDENTITY_PHASE19_TEST_CLUSTER_ID required")?;
        ensure!(
            !expected.is_empty() && expected.bytes().all(|b| b.is_ascii_digit()),
            "invalid expected cluster id"
        );
        let options = PgConnectOptions::from_str(&raw)
            .map_err(|_| anyhow!("invalid isolated PostgreSQL configuration"))?;
        ensure!(
            options.get_database() == Some("postgres")
                && matches!(options.get_host(), "127.0.0.1" | "localhost" | "::1"),
            "explicit loopback postgres administration database required"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("isolated cluster unavailable"))?;
        let actual: String =
            sqlx::query_scalar("SELECT system_identifier::text FROM pg_control_system()")
                .fetch_one(&admin)
                .await?;
        ensure!(
            actual == expected,
            "refusing a different PostgreSQL cluster"
        );
        let name = format!("{PREFIX}{}", uuid7::new_uuid().simple());
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await?;
        eprintln!("phase19_owned_database created={name}");
        let pool = match PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.database(&name))
            .await
        {
            Ok(pool) => pool,
            Err(error) => {
                sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
                    .execute(&admin)
                    .await?;
                admin.close().await;
                return Err(error.into());
            }
        };
        let db = Self { pool, admin, name };
        if let Err(error) = sqlx::migrate!("./migrations").run(&db.pool).await {
            db.finish(Ok(())).await?;
            return Err(error.into());
        }
        Ok(db)
    }
    async fn finish(self, result: Result<()>) -> Result<()> {
        self.pool.close().await;
        ensure!(
            self.name.starts_with(PREFIX)
                && self.name.len() == PREFIX.len() + 32
                && self.name[PREFIX.len()..]
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit()),
            "unsafe owned database cleanup name"
        );
        let cleanup = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await;
        let remains: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname=$1)")
                .bind(&self.name)
                .fetch_one(&self.admin)
                .await?;
        self.admin.close().await;
        cleanup.context("owned Phase19 database cleanup failed")?;
        ensure!(!remains, "owned Phase19 database remained after cleanup");
        eprintln!("phase19_owned_database removed={}", self.name);
        result
    }
}

async fn isolated<F, Fut>(case: F) -> Result<()>
where
    F: FnOnce(PgPool) -> Fut,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    let db = Database::new().await?;
    // Joining a separate task preserves the finally/drop path even on assertions
    // or other unexpected panics in a scenario.
    let result = tokio::spawn(case(db.pool.clone()))
        .await
        .map_err(|error| anyhow!("Phase19 scenario task failed: {error}"))
        .and_then(|result| result);
    db.finish(result).await
}

struct Actor {
    user: CurrentUser,
    role: Uuid,
    sid: Uuid,
    grant: Uuid,
}
struct Fixture {
    state: AppState,
    tenant: Uuid,
    org: Uuid,
    menu: Uuid,
    fine: HashMap<&'static str, Uuid>,
}
impl Fixture {
    async fn new(pool: PgPool) -> Result<Self> {
        let config = crate::config::Config::from_values(&HashMap::from([
            ("APP_ENV".into(), "test".into()),
            (
                "DATABASE_URL".into(),
                "postgres://unused/owned-phase19".into(),
            ),
            (
                "JWT_SECRET".into(),
                "owned-phase19-tests-only-not-production".into(),
            ),
        ]))?;
        let mut state = AppState::new(pool, Arc::new(config));
        Arc::make_mut(&mut state.config).bff_oidc_issuer = Some(ISSUER.into());
        let (tenant, org, app, menu) = (
            uuid7::new_uuid(),
            uuid7::new_uuid(),
            uuid7::new_uuid(),
            uuid7::new_uuid(),
        );
        let pool = &state.pool;
        sqlx::query("INSERT INTO tenants(id,name,code,is_platform) VALUES($1,'Owned review tenant','owned-phase19',true)").bind(tenant).execute(pool).await?;
        sqlx::query(
            "INSERT INTO organizations(id,tenant_id,name) VALUES($1,$2,'Owned review root')",
        )
        .bind(org)
        .bind(tenant)
        .execute(pool)
        .await?;
        sqlx::query("INSERT INTO applications(id,name,code,allowed_member_types) VALUES($1,'Owned intake administration','owned-phase19',ARRAY['EMPLOYEE'])").bind(app).execute(pool).await?;
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
        sqlx::query("INSERT INTO menus(id,application_id,type,title,permission,path) VALUES($1,$2,'MENU','Owned intake','system:oauth','/system/oauth/clients')").bind(menu).bind(app).execute(pool).await?;
        let mut fine = HashMap::new();
        for permission in FINE {
            let id = uuid7::new_uuid();
            sqlx::query("INSERT INTO menus(id,application_id,parent_id,type,title,permission) VALUES($1,$2,$3,'BUTTON',$4,$4)").bind(id).bind(app).bind(menu).bind(permission).execute(pool).await?;
            fine.insert(*permission, id);
        }
        for id in std::iter::once(menu).chain(fine.values().copied()) {
            sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
                .bind(tenant)
                .bind(id)
                .execute(pool)
                .await?;
            sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
                .bind(org)
                .bind(id)
                .execute(pool)
                .await?;
        }
        sqlx::query("INSERT INTO auth_provider_settings(singleton,issuer,created_at) VALUES(true,$1,clock_timestamp())").bind(ISSUER).execute(pool).await?;
        Ok(Self {
            state,
            tenant,
            org,
            menu,
            fine,
        })
    }
    async fn actor(&self, name: &str, permissions: &[&str]) -> Result<Actor> {
        let pool = &self.state.pool;
        let (subject, role, member, selected) = (
            uuid7::new_uuid(),
            uuid7::new_uuid(),
            uuid7::new_uuid(),
            uuid7::new_uuid(),
        );
        sqlx::query("INSERT INTO users(id,username,password_hash,real_name,session_version) VALUES($1,$2,'fixture-not-a-login-credential','Owned review fixture',3)").bind(subject).bind(name).execute(pool).await?;
        sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,status,identity_version) VALUES($1,$2,$3,'EMPLOYEE','ENABLED',1)").bind(member).bind(subject).bind(self.org).execute(pool).await?;
        sqlx::query(
            "INSERT INTO roles(id,tenant_id,organization_id,name,code) VALUES($1,$2,$3,$4,$4)",
        )
        .bind(role)
        .bind(self.tenant)
        .bind(self.org)
        .bind(name)
        .execute(pool)
        .await?;
        sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3)")
            .bind(subject)
            .bind(role)
            .bind(self.org)
            .execute(pool)
            .await?;
        for id in std::iter::once(self.menu)
            .chain(permissions.iter().map(|permission| self.fine[permission]))
        {
            sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)")
                .bind(role)
                .bind(id)
                .execute(pool)
                .await?;
        }
        let (client, grant, sid, auth_time, expires) =
            crate::identity_deployment_tests::canonical_employee_source(pool, subject).await?;
        // Fixture-only assurance data. It exercises canonical joins and policy,
        // not authenticator enrollment, TOTP verification or real HTTPS login.
        sqlx::query("INSERT INTO auth_totp_factors(user_id,secret_ciphertext) VALUES($1,decode(repeat('ab',32),'hex'))").bind(subject).execute(pool).await?;
        sqlx::query("UPDATE sso_sessions SET mfa_method='totp',mfa_verified_at=clock_timestamp() WHERE id=$1").bind(sid).execute(pool).await?;
        let (browser, bff) = (uuid7::new_uuid(), uuid7::new_uuid());
        let digest = crate::util::hash::sha256_hex(&bff.to_string());
        sqlx::query("INSERT INTO console_browsers(id,token_hash,generation,current_session_id,created_at,expires_at) VALUES($1,$2,1,$3,clock_timestamp(),$4)").bind(browser).bind(&digest).bind(bff).bind(expires).execute(pool).await?;
        sqlx::query("INSERT INTO bff_sessions(id,browser_id,token_hash,csrf_hash,user_id,account_version,global_session_id,oidc_client_id,oidc_grant_id,oidc_client_policy_version,issuer,auth_time,nonce_hash,access_ciphertext,refresh_ciphertext,access_expires_at,version,selected_session_id,created_at,expires_at,oidc_public_sid,oidc_public_sub) VALUES($1,$2,$3,$3,$4,3,$5,$6,$7,1,$8,$9,$3,'owned-review-fixture','owned-review-fixture',$10,1,$11,clock_timestamp(),$10,$3,'owned-pairwise-'||$4::text)").bind(bff).bind(browser).bind(&digest).bind(subject).bind(sid).bind(client).bind(grant).bind(ISSUER).bind(auth_time).bind(expires).bind(selected).execute(pool).await?;
        sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version,bff_session_id,bff_context_version) VALUES($1,$2,$3,$4,$5,3,$6,'EMPLOYEE',1,$7,$8,1,$9,1)").bind(selected).bind(subject).bind(self.tenant).bind(self.org).bind(expires).bind(member).bind(ISSUER).bind(client.to_string()).bind(bff).execute(pool).await?;
        let user = CurrentUser {
            user_id: subject,
            username: name.into(),
            real_name: "Owned fixture".into(),
            is_super_admin: false,
            tenant_id: self.tenant,
            org_id: self.org,
            session_id: selected,
        };
        let active: bool = sqlx::query_scalar(
            "SELECT platform_identity_management_actor_active($1,$2,$3,true,false)",
        )
        .bind(subject)
        .bind(selected)
        .bind(permissions.first().copied().unwrap_or(policy::VIEW))
        .fetch_one(pool)
        .await?;
        ensure!(
            active,
            "canonical employee fixture is not actually authorized"
        );
        Ok(Actor {
            user,
            role,
            sid,
            grant,
        })
    }
    async fn application(&self, actor: &Actor, name: &str) -> Result<Uuid> {
        let partner =
            service::partner_create(&self.state, &actor.user, &headers(), partner(name)).await?;
        let app = service::application_create(
            &self.state,
            &actor.user,
            &headers(),
            json!({"partnerId":partner["id"],"displayName":name}),
        )
        .await?;
        identifier(&app, "applicationId")
    }
}
fn headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-identity-operation",
        uuid7::new_uuid().to_string().parse().expect("UUID header"),
    );
    headers
}
fn identifier(value: &Value, name: &str) -> Result<Uuid> {
    Uuid::parse_str(value[name].as_str().context("missing UUID result")?).map_err(Into::into)
}
fn partner(name: &str) -> Value {
    json!({"name":name,"contactName":"Owned fixture contact","contactChannel":"verified-channel:owned-fixture","subjectReference":uuid7::new_uuid()})
}
fn manifest() -> Value {
    json!({"displayName":"Owned partner login","purpose":"Controlled fixture account login","privacyUrl":"https://partner.example.invalid/privacy","domains":["partner.example.invalid"],"redirectUris":["https://partner.example.invalid/callback"],"postLogoutRedirectUris":[],"scopes":["openid","profile"],"contactReference":uuid7::new_uuid(),"domainEvidenceReference":uuid7::new_uuid(),"logoutMode":"SHORT_SESSION_ONLINE_CHECK"})
}
fn submission(manifest: Value, environment: &str, version: Option<i64>) -> Value {
    json!({"manifest":manifest,"environment":environment,"expectedVersion":version})
}
fn decision(manifest: &Value, version: i64, status: &str) -> Value {
    json!({"expectedVersion":version,"status":status,"reason":"Owned fixture manual evidence review","verifiedContactReference":manifest["contactReference"],"verifiedDomainReference":manifest["domainEvidenceReference"]})
}
fn denied<T>(result: std::result::Result<T, AppError>, code: &str) -> Result<()> {
    match result {
        Err(error)
            if matches!(
                (&error, code),
                (AppError::Forbidden(_), "FORBIDDEN")
                    | (AppError::BadRequest(_), "BAD_REQUEST")
                    | (AppError::Conflict(_), "CONFLICT")
                    | (AppError::Unauthorized(_), "UNAUTHORIZED")
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(anyhow!("expected {code}, received {error}")),
        Ok(_) => Err(anyhow!("expected {code}, operation unexpectedly succeeded")),
    }
}
async fn unchanged(pool: &PgPool, request: Uuid, version: i64, status: &str) -> Result<()> {
    let actual: (i64, String) =
        sqlx::query_as("SELECT version,status FROM platform_auth_application_requests WHERE id=$1")
            .bind(request)
            .fetch_one(pool)
            .await?;
    ensure!(
        actual == (version, status.into()),
        "denied command changed the request"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE19_TEST_DATABASE_URL and cluster id"]
async fn phase19_canonical_submit_review_self_denial_and_durable_operations() -> Result<()> {
    isolated(|pool| async move {
        let f = Fixture::new(pool).await?;
        let registrar = f.actor("registrar", FINE).await?;
        let applicant = f.actor("applicant", FINE).await?;
        let submitter = f.actor("submitter", FINE).await?;
        let reviewer = f.actor("reviewer", FINE).await?;
        let app = f.application(&registrar, "Owned application").await?;
        let input = manifest();
        let request = service::submit(&f.state, &applicant.user, &headers(), app, submission(input.clone(), "test", None)).await?;
        let id = identifier(&request, "id")?;
        ensure!(request["version"] == 1 && request["status"] == "SUBMITTED", "initial request state mismatch");
        let returned = service::decide(&f.state, &reviewer.user, &headers(), id, decision(&input, 1, "RETURNED")).await?;
        ensure!(!returned.1 && returned.0["version"] == 2, "return incorrectly required identity mutation");
        let revised = service::submit(&f.state, &submitter.user, &headers(), app, submission(input.clone(), "test", Some(2))).await?;
        ensure!(revised["requestedBy"] == applicant.user.user_id.to_string() && revised["submittedBy"] == submitter.user.user_id.to_string() && revised["version"] == 3, "revision lost actor history");
        for actor in [&registrar, &applicant, &submitter] {
            denied(service::decide(&f.state, &actor.user, &headers(), id, decision(&input, 3, "TEST_APPROVED")).await, "FORBIDDEN")?;
            unchanged(&f.state.pool, id, 3, "SUBMITTED").await?;
        }
        denied(service::decide(&f.state, &reviewer.user, &headers(), id, decision(&input, 2, "TEST_APPROVED")).await, "CONFLICT")?;
        let mut wrong = decision(&input, 3, "TEST_APPROVED");
        wrong["verifiedDomainReference"] = json!(uuid7::new_uuid());
        denied(service::decide(&f.state, &reviewer.user, &headers(), id, wrong).await, "CONFLICT")?;
        let operation = headers();
        let body = decision(&input, 3, "TEST_APPROVED");
        let (approved, remote) = service::decide(&f.state, &reviewer.user, &operation, id, body.clone()).await?;
        ensure!(remote && approved["status"] == "TEST_APPROVED" && approved["version"] == 4, "independent approval was not durable");
        let (again, remote_again) = service::decide(&f.state, &reviewer.user, &operation, id, body).await?;
        ensure!(again == approved && !remote_again, "same operation did not replay its original result");
        let counts: (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM platform_auth_review_operations),(SELECT count(*) FROM audit_logs WHERE action='AUTH_INTAKE_COMMAND')").fetch_one(&f.state.pool).await?;
        ensure!(counts == (6, 6), "denied or replayed commands added durable operations/audits");
        let pending: bool = sqlx::query_scalar("SELECT remote_required AND remote_result IS NULL FROM platform_auth_review_operations WHERE operation_id=$1").bind(Uuid::parse_str(operation["x-identity-operation"].to_str()?)?).fetch_one(&f.state.pool).await?;
        ensure!(pending, "local approval incorrectly claimed remote synchronization");
        Ok(())
    }).await
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE19_TEST_DATABASE_URL and cluster id"]
async fn phase19_canonical_manifest_rejections_leave_no_request_or_receipt() -> Result<()> {
    isolated(|pool| async move {
        let f = Fixture::new(pool).await?;
        let actor = f.actor("manifest-reviewer", FINE).await?;
        let app = f.application(&actor, "Owned manifest application").await?;
        let mut invalid = Vec::new();
        for scope in ["email", "phone", "offline_access", "stores.read"] {
            let mut value = manifest(); value["scopes"] = json!(["openid",scope]); invalid.push(value);
        }
        for uri in ["http://partner.example.invalid/callback", "https://foreign.example.invalid/callback", "https://partner.example.invalid/callback?code=stale", "https://*.example.invalid/callback", "https://partner.example.invalid/callback#token"] {
            let mut value = manifest(); value["redirectUris"] = json!([uri]); invalid.push(value);
        }
        for (key,value) in [("domains",json!(["partner.example.invalid","partner.example.invalid"])), ("scopes",json!(["profile"])), ("purpose",json!("PLATFORM_CONSOLE")), ("approvedBy",json!(uuid7::new_uuid())), ("contactReference",json!("not-a-proof")), ("logoutMode",json!("NONE"))] {
            let mut input = manifest(); input[key] = value; invalid.push(input);
        }
        for value in invalid {
            denied(service::submit(&f.state, &actor.user, &headers(), app, submission(value, "test", None)).await, "BAD_REQUEST")?;
        }
        let mut extra = submission(manifest(), "test", None); extra["approvedBy"] = json!(actor.user.user_id);
        denied(service::submit(&f.state, &actor.user, &headers(), app, extra).await, "BAD_REQUEST")?;
        let counts: (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM platform_auth_application_requests),(SELECT count(*) FROM platform_auth_review_operations)").fetch_one(&f.state.pool).await?;
        ensure!(counts == (0, 2), "invalid manifests changed durable request state");
        Ok(())
    }).await
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE19_TEST_DATABASE_URL and cluster id"]
async fn phase19_canonical_production_default_and_missing_binding_remain_closed() -> Result<()> {
    isolated(|pool| async move {
        let f = Fixture::new(pool).await?;
        let actor = f.actor("production-applicant", FINE).await?;
        let reviewer = f.actor("production-reviewer", FINE).await?;
        let app = f
            .application(&actor, "Owned production application")
            .await?;
        let input = manifest();
        let row = service::submit(
            &f.state,
            &actor.user,
            &headers(),
            app,
            submission(input.clone(), "production", None),
        )
        .await?;
        let id = identifier(&row, "id")?;
        for delete_default in [false, true] {
            if delete_default {
                sqlx::query("DELETE FROM platform_auth_release_bindings")
                    .execute(&f.state.pool)
                    .await?;
            }
            let security = service::security(&f.state, &reviewer.user).await?;
            ensure!(
                security["productionReady"] == false,
                "default or missing responsibility bindings opened production"
            );
            ensure!(
                security["reauthenticateUrl"] == "/auth/step-up?return=/system/oauth/clients",
                "controlled reauthentication entry changed"
            );
            denied(
                service::decide(
                    &f.state,
                    &reviewer.user,
                    &headers(),
                    id,
                    decision(&input, 1, "PRODUCTION_APPROVED"),
                )
                .await,
                "FORBIDDEN",
            )?;
            unchanged(&f.state.pool, id, 1, "SUBMITTED").await?;
        }
        Ok(())
    })
    .await
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE19_TEST_DATABASE_URL and cluster id"]
async fn phase19_canonical_missing_and_stale_mfa_facts_block_writes_but_allow_bootstrap(
) -> Result<()> {
    isolated(|pool| async move {
        let mut f = Fixture::new(pool).await?;
        let actor = f.actor("assurance-reviewer", FINE).await?;
        Arc::make_mut(&mut f.state.config).app_env = "production".into();
        sqlx::query("DELETE FROM auth_totp_factors WHERE user_id=$1").bind(actor.user.user_id).execute(&f.state.pool).await?;
        let security = service::security(&f.state, &actor.user).await?;
        ensure!(security["totpEnabled"] == false && security["recent"] == false, "missing factor falsely reported recent");
        denied(service::partner_create(&f.state, &actor.user, &headers(), partner("No factor")).await, "FORBIDDEN")?;
        sqlx::query("INSERT INTO auth_totp_factors(user_id,secret_ciphertext) VALUES($1,decode(repeat('ab',32),'hex'))").bind(actor.user.user_id).execute(&f.state.pool).await?;
        for (method,offset) in [(None,None), (Some("totp"),Some(-360)), (Some("totp"),Some(360)), (Some("recovery_code"),Some(0))] {
            sqlx::query("UPDATE sso_sessions SET mfa_method=$2,mfa_verified_at=CASE WHEN $3::bigint IS NULL THEN NULL ELSE clock_timestamp()+make_interval(secs=>$3::double precision) END WHERE id=$1").bind(actor.sid).bind(method).bind(offset).execute(&f.state.pool).await?;
            let security = service::security(&f.state, &actor.user).await?;
            ensure!(security["totpEnabled"] == true && security["recent"] == false, "ineligible assurance reported recent");
            denied(service::partner_create(&f.state, &actor.user, &headers(), partner("Ineligible MFA")).await, "FORBIDDEN")?;
        }
        sqlx::query("UPDATE sso_sessions SET mfa_method='totp',mfa_verified_at=clock_timestamp() WHERE id=$1").bind(actor.sid).execute(&f.state.pool).await?;
        service::partner_create(&f.state, &actor.user, &headers(), partner("Fresh fixture assurance")).await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM platform_auth_partners").fetch_one(&f.state.pool).await?;
        ensure!(count == 1, "blocked assurance writes produced partners");
        Ok(())
    }).await
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE19_TEST_DATABASE_URL and cluster id"]
async fn phase19_canonical_permission_revocation_blocks_new_writes_and_replays() -> Result<()> {
    isolated(|pool| async move {
        let f = Fixture::new(pool).await?;
        let actor = f.actor("revoked-reviewer", FINE).await?;
        let reviewer = f.actor("independent-revoked-reviewer", FINE).await?;
        let operation = headers(); let body = partner("Owned replay partner");
        let created_partner = service::partner_create(&f.state, &actor.user, &operation, body.clone()).await?;
        let application = service::application_create(&f.state, &actor.user, &headers(), json!({"partnerId":created_partner["id"],"displayName":"Permission revocation application"})).await?;
        let app = identifier(&application, "applicationId")?;
        let input = manifest();
        let request = service::submit(&f.state, &actor.user, &headers(), app, submission(input.clone(), "test", None)).await?;
        let request = identifier(&request, "id")?;
        sqlx::query("DELETE FROM role_menus WHERE role_id=$1 AND menu_id=$2").bind(reviewer.role).bind(f.fine[policy::REVIEW]).execute(&f.state.pool).await?;
        denied(service::decide(&f.state, &reviewer.user, &headers(), request, decision(&input, 1, "TEST_APPROVED")).await, "FORBIDDEN")?;
        unchanged(&f.state.pool, request, 1, "SUBMITTED").await?;
        sqlx::query("DELETE FROM role_menus WHERE role_id=$1 AND menu_id=$2").bind(actor.role).bind(f.fine[policy::REGISTER]).execute(&f.state.pool).await?;
        denied(service::partner_create(&f.state, &actor.user, &operation, body).await, "FORBIDDEN")?;
        denied(service::partner_create(&f.state, &actor.user, &headers(), partner("Revoked new command")).await, "FORBIDDEN")?;
        // A real database superadmin flag bypasses roles, never the fine grant
        // at tenant/application/organization level.
        sqlx::query("UPDATE users SET is_super_admin=true WHERE id=$1").bind(actor.user.user_id).execute(&f.state.pool).await?;
        sqlx::query("DELETE FROM organization_menus WHERE organization_id=$1 AND menu_id=$2").bind(f.org).bind(f.fine[policy::REGISTER]).execute(&f.state.pool).await?;
        let mut stale_actor = actor.user.clone(); stale_actor.is_super_admin = true;
        denied(service::partner_create(&f.state, &stale_actor, &headers(), partner("Superadmin missing fine grant")).await, "FORBIDDEN")?;
        sqlx::query("DELETE FROM organization_menus WHERE organization_id=$1 AND menu_id=$2").bind(f.org).bind(f.fine[policy::VIEW]).execute(&f.state.pool).await?;
        denied(service::security(&f.state, &stale_actor).await, "FORBIDDEN")?;
        let counts: (i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM platform_auth_partners),(SELECT count(*) FROM platform_auth_review_operations)").fetch_one(&f.state.pool).await?;
        ensure!(counts == (1,3), "permission revocation allowed another durable command");
        Ok(())
    }).await
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE19_TEST_DATABASE_URL and cluster id"]
async fn phase19_canonical_assignment_is_explicit_and_original_source_revocation_fences_writes(
) -> Result<()> {
    isolated(|pool| async move {
        let f = Fixture::new(pool).await?;
        let registrar = f.actor("assignment-manager", FINE).await?;
        let operator = f
            .actor("assigned-registrar", &[policy::VIEW, policy::REGISTER])
            .await?;
        let app = f
            .application(&registrar, "Explicit assigned application")
            .await?;
        let other = f.application(&registrar, "Unassigned application").await?;
        denied(
            service::partner_create(
                &f.state,
                &operator.user,
                &headers(),
                partner("Ordinary directory access"),
            )
            .await,
            "FORBIDDEN",
        )?;
        denied(
            service::submit(
                &f.state,
                &operator.user,
                &headers(),
                app,
                submission(manifest(), "test", None),
            )
            .await,
            "FORBIDDEN",
        )?;
        service::assign(
            &f.state,
            &registrar.user,
            &headers(),
            app,
            json!({"userId":operator.user.user_id}),
        )
        .await?;
        service::submit(
            &f.state,
            &operator.user,
            &headers(),
            app,
            submission(manifest(), "test", None),
        )
        .await?;
        denied(
            service::submit(
                &f.state,
                &operator.user,
                &headers(),
                other,
                submission(manifest(), "test", None),
            )
            .await,
            "FORBIDDEN",
        )?;
        sqlx::query("UPDATE auth_grants SET revoked_at=clock_timestamp() WHERE id=$1")
            .bind(operator.grant)
            .execute(&f.state.pool)
            .await?;
        denied(
            service::submit(
                &f.state,
                &operator.user,
                &headers(),
                app,
                submission(manifest(), "production", None),
            )
            .await,
            "UNAUTHORIZED",
        )?;
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM platform_auth_application_requests")
                .fetch_one(&f.state.pool)
                .await?;
        ensure!(count == 1, "unassigned or revoked source produced requests");
        Ok(())
    })
    .await
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE19_TEST_DATABASE_URL and cluster id"]
async fn phase19_canonical_sync_receipt_preserves_pending_and_original_snapshot() -> Result<()> {
    isolated(|pool| async move {
        let f = Fixture::new(pool).await?;
        let applicant = f.actor("sync-applicant", FINE).await?;
        let reviewer = f.actor("sync-reviewer", FINE).await?;
        let app = f.application(&applicant, "Owned sync receipt application").await?;
        let input = manifest();
        let submitted = service::submit(&f.state, &applicant.user, &headers(), app, submission(input.clone(), "test", None)).await?;
        let request = identifier(&submitted, "id")?;
        let original_headers = headers();
        let operation = Uuid::parse_str(original_headers["x-identity-operation"].to_str()?)?;
        let original_body = decision(&input, 1, "TEST_APPROVED");
        let (approved, remote) = service::decide(&f.state, &reviewer.user, &original_headers, request, original_body.clone()).await?;
        ensure!(remote && approved["syncState"] == "PENDING", "local approval falsely claimed synchronization");
        let metadata = json!({"projectionVersion":null,"admissionVersion":null});
        let plan = service::prepare_sync(&f.state, &reviewer.user, &original_headers, request, None, &metadata).await?;
        ensure!(plan.operation == operation && plan.result == approved, "sync plan lost its original authorized snapshot");
        let payload = plan.payload.context("approved request did not produce a sync payload")?;
        ensure!(payload["requestId"] == request.to_string() && payload["requestVersion"] == 2
            && payload["expectedProjectionVersion"].is_null() && payload["expectedAdmissionVersion"].is_null(), "sync payload lost version preconditions");
        ensure!(store::local_operation(&f.state, &reviewer.user, operation).await?.is_none(), "pending identity receipt was presented as completed");
        let (pending_replay, remote_replay) = service::decide(&f.state, &reviewer.user, &original_headers, request, original_body).await?;
        ensure!(!remote_replay && pending_replay["syncState"] == "PENDING" && pending_replay["syncedVersion"] == 0, "pending replay falsely reported remote success");
        denied(service::prepare_sync(&f.state, &reviewer.user, &headers(), request, Some(json!({"expectedVersion":2})), &metadata).await, "CONFLICT")?;
        let safe_result = json!({"requestId":request,"requestVersion":2,"projectionVersion":7,"admissionVersion":9});
        let mut tx = f.state.pool.begin().await?;
        store::complete_remote(&mut tx, operation, &safe_result).await?;
        tx.commit().await?;
        let completed = store::local_operation(&f.state, &reviewer.user, operation).await?.context("completed receipt missing")?;
        ensure!(completed["state"] == "COMPLETED" && completed["result"]["syncState"] == "COMPLETED"
            && completed["result"]["version"] == 2 && completed["result"]["syncedVersion"] == 2
            && completed["result"]["identityProjectionVersion"] == 7 && completed["result"]["identityAdmissionVersion"] == 9,
            "completed receipt lost the exact synchronized version");
        let synced: (i64, i64) = sqlx::query_as("SELECT version,synced_version FROM platform_auth_application_requests WHERE id=$1").bind(request).fetch_one(&f.state.pool).await?;
        ensure!(synced == (2,2), "completed receipt did not mark the current approved proposal synchronized");

        let mut revision = input.clone(); revision["purpose"] = json!("New fixture proposal awaiting independent approval");
        service::submit(&f.state, &applicant.user, &headers(), app, submission(revision.clone(), "test", Some(2))).await?;
        let prior = store::local_operation(&f.state, &reviewer.user, operation).await?.context("original receipt disappeared after revision")?;
        ensure!(prior == completed && prior["result"]["manifest"] == approved["manifest"], "new proposal overwrote the old synchronized receipt");

        // A second in-flight synchronization is completed only after a newer
        // proposal exists. Its durable receipt must never update that proposal.
        let next_headers = headers();
        let next_operation = Uuid::parse_str(next_headers["x-identity-operation"].to_str()?)?;
        service::decide(&f.state, &reviewer.user, &next_headers, request, decision(&revision, 3, "TEST_APPROVED")).await?;
        let next_plan = service::prepare_sync(&f.state, &reviewer.user, &next_headers, request, None, &safe_result).await?;
        ensure!(next_plan.payload.is_some(), "second approval did not produce a sync payload");
        let mut premature_acceptance = decision(&revision, 4, "TEST_ACCEPTED");
        premature_acceptance["testEvidenceReference"] = json!(uuid7::new_uuid());
        denied(service::decide(&f.state, &reviewer.user, &headers(), request, premature_acceptance).await, "CONFLICT")?;
        unchanged(&f.state.pool, request, 4, "TEST_APPROVED").await?;
        let mut newer = revision.clone(); newer["purpose"] = json!("Newest fixture proposal must not be silently approved");
        service::submit(&f.state, &applicant.user, &headers(), app, submission(newer.clone(), "test", Some(4))).await?;
        let late_result = json!({"requestId":request,"requestVersion":4,"projectionVersion":8,"admissionVersion":10});
        let mut tx = f.state.pool.begin().await?;
        store::complete_remote(&mut tx, next_operation, &late_result).await?;
        tx.commit().await?;
        let late_receipt = store::local_operation(&f.state, &reviewer.user, next_operation).await?.context("late completed receipt missing")?;
        ensure!(late_receipt["result"]["version"] == 4 && late_receipt["result"]["syncedVersion"] == 4
            && late_receipt["result"]["manifest"] == revision && late_receipt["result"]["status"] == "TEST_APPROVED", "late receipt lost its original approved proposal");
        let current: (i64,i64,String,Value,Option<Uuid>) = sqlx::query_as("SELECT version,synced_version,status,manifest,reviewed_by FROM platform_auth_application_requests WHERE id=$1").bind(request).fetch_one(&f.state.pool).await?;
        ensure!(current == (5,2,"SUBMITTED".into(),newer,None), "late identity result overwrote the newest unapproved proposal");

        let not_applied_headers = headers();
        let not_applied_operation = Uuid::parse_str(not_applied_headers["x-identity-operation"].to_str()?)?;
        let not_applied_body = decision(&revision, 5, "TEST_APPROVED");
        service::decide(&f.state, &reviewer.user, &not_applied_headers, request, not_applied_body.clone()).await?;
        let mut tx = f.state.pool.begin().await?;
        store::complete_remote(&mut tx, not_applied_operation, &json!({"state":"NOT_APPLIED"})).await?;
        tx.commit().await?;
        let stored: Value = sqlx::query_scalar("SELECT result_json FROM platform_auth_review_operations WHERE operation_id=$1").bind(not_applied_operation).fetch_one(&f.state.pool).await?;
        ensure!(stored["syncState"] == "NOT_APPLIED", "NOT_APPLIED was not persisted in the local result snapshot");
        let replayed = service::decide(&f.state, &reviewer.user, &not_applied_headers, request, not_applied_body).await?;
        ensure!(!replayed.1 && replayed.0["syncState"] == "NOT_APPLIED", "NOT_APPLIED replay reverted to PENDING");
        let terminal = store::local_operation(&f.state, &reviewer.user, not_applied_operation).await?.context("NOT_APPLIED terminal receipt missing")?;
        ensure!(terminal["state"] == "COMPLETED" && terminal["result"]["syncState"] == "NOT_APPLIED", "NOT_APPLIED receipt remained unresolved");
        Ok(())
    }).await
}

#[tokio::test]
#[ignore = "requires explicit isolated IDENTITY_PHASE19_TEST_DATABASE_URL and cluster id"]
async fn phase19_canonical_configured_production_requires_distinct_bound_publishers() -> Result<()>
{
    isolated(|pool| async move {
        let f = Fixture::new(pool).await?;
        let applicant = f.actor("production-fixture-applicant", FINE).await?;
        let reviewer = f.actor("production-fixture-reviewer", &[policy::VIEW,policy::REVIEW]).await?;
        let publisher = f.actor("production-fixture-publisher", &[policy::VIEW,policy::PUBLISH]).await?;
        let unbound = f.actor("production-fixture-unbound", &[policy::VIEW,policy::PUBLISH]).await?;
        let app = f.application(&applicant, "Fixture controlled production application").await?;
        for target in [&publisher, &unbound] {
            service::assign(&f.state, &applicant.user, &headers(), app, json!({"userId":target.user.user_id})).await?;
        }
        let input = manifest();
        let test = service::submit(&f.state, &applicant.user, &headers(), app, submission(input.clone(), "test", None)).await?;
        let test_request = identifier(&test, "id")?;
        let test_headers = headers();
        let test_operation = Uuid::parse_str(test_headers["x-identity-operation"].to_str()?)?;
        let approved = service::decide(&f.state, &reviewer.user, &test_headers, test_request, decision(&input, 1, "TEST_APPROVED")).await?;
        ensure!(approved.1, "test approval lost its synchronization requirement");
        // Known remote results below are test input to the durable receipt store,
        // not a claim that a real Identity HTTPS request was executed.
        let mut tx = f.state.pool.begin().await?;
        store::complete_remote(&mut tx, test_operation, &json!({"requestId":test_request,"requestVersion":2,"projectionVersion":1,"admissionVersion":1})).await?;
        tx.commit().await?;
        let evidence = uuid7::new_uuid();
        let mut acceptance = decision(&input, 2, "TEST_ACCEPTED");
        acceptance["testEvidenceReference"] = json!(evidence);
        let accepted = service::decide(&f.state, &reviewer.user, &headers(), test_request, acceptance).await?;
        ensure!(!accepted.1 && accepted.0["version"] == 3 && accepted.0["syncedVersion"] == 2
            && accepted.0["testEvidenceReference"] == evidence.to_string(), "test acceptance lost the known synchronized proposal or evidence");

        // Only the isolated database administration fixture sets responsibility.
        // Runtime service calls cannot manufacture this deployment-owned row.
        sqlx::query("UPDATE platform_auth_release_bindings SET enabled=true,reviewer_user_ids=$1,publisher_user_ids=$2,policy_reference='fixture-policy:owned-phase19',operations_reference='fixture-operations:owned-phase19',verified_at=clock_timestamp() WHERE singleton")
            .bind(vec![reviewer.user.user_id]).bind(vec![publisher.user.user_id]).execute(&f.state.pool).await?;
        let security = service::security(&f.state, &reviewer.user).await?;
        ensure!(security["productionReady"] == true, "explicit fixture responsibility binding remained closed");
        let production = service::submit(&f.state, &applicant.user, &headers(), app, submission(input.clone(), "production", None)).await?;
        let production_request = identifier(&production, "id")?;
        denied(service::decide(&f.state, &applicant.user, &headers(), production_request, decision(&input, 1, "PRODUCTION_APPROVED")).await, "FORBIDDEN")?;
        let approved = service::decide(&f.state, &reviewer.user, &headers(), production_request, decision(&input, 1, "PRODUCTION_APPROVED")).await?;
        ensure!(!approved.1 && approved.0["version"] == 2 && approved.0["reviewedBy"] == reviewer.user.user_id.to_string(), "production approval skipped its independent reviewer");
        denied(service::decide(&f.state, &applicant.user, &headers(), production_request, decision(&input, 2, "ENABLED")).await, "FORBIDDEN")?;
        // This account has a current canonical source, fine publish permission
        // and explicit application assignment, but is absent from release binding.
        denied(service::decide(&f.state, &unbound.user, &headers(), production_request, decision(&input, 2, "ENABLED")).await, "FORBIDDEN")?;
        unchanged(&f.state.pool, production_request, 2, "PRODUCTION_APPROVED").await?;
        let publish_headers = headers();
        let publish_operation = Uuid::parse_str(publish_headers["x-identity-operation"].to_str()?)?;
        let enabled = service::decide(&f.state, &publisher.user, &publish_headers, production_request, decision(&input, 2, "ENABLED")).await?;
        ensure!(enabled.1 && enabled.0["version"] == 3 && enabled.0["publishedBy"] == publisher.user.user_id.to_string()
            && enabled.0["reviewedBy"] == reviewer.user.user_id.to_string() && enabled.0["syncState"] == "PENDING", "production enablement lost its distinct publisher or pending receipt");
        let mut tx = f.state.pool.begin().await?;
        store::complete_remote(&mut tx, publish_operation, &json!({"requestId":production_request,"requestVersion":3,"projectionVersion":2,"admissionVersion":1})).await?;
        tx.commit().await?;
        let receipt = store::local_operation(&f.state, &publisher.user, publish_operation).await?.context("production fixture receipt missing")?;
        ensure!(receipt["result"]["syncState"] == "COMPLETED" && receipt["result"]["syncedVersion"] == 3
            && receipt["result"]["publishedBy"] == publisher.user.user_id.to_string(), "known production fixture receipt lost its publisher/version");
        let persisted: (i64,i64,Uuid,Uuid) = sqlx::query_as("SELECT version,synced_version,reviewed_by,published_by FROM platform_auth_application_requests WHERE id=$1").bind(production_request).fetch_one(&f.state.pool).await?;
        ensure!(persisted == (3,3,reviewer.user.user_id,publisher.user.user_id), "production fixture persisted different actors or versions");
        let counts: (i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM platform_auth_review_operations),(SELECT count(*) FROM audit_logs WHERE action='AUTH_INTAKE_COMMAND')").fetch_one(&f.state.pool).await?;
        ensure!(counts == (10,10), "denied production actions changed operation/audit counts");
        let actor_facts: (Uuid,Uuid,String) = sqlx::query_as("SELECT actor_user_id,actor_session_id,management_permission FROM platform_auth_review_operations WHERE operation_id=$1").bind(publish_operation).fetch_one(&f.state.pool).await?;
        ensure!(actor_facts == (publisher.user.user_id,publisher.user.session_id,policy::PUBLISH.into()), "publish receipt lost its canonical actor/fine permission");
        Ok(())
    }).await
}
