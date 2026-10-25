//! Setup uses a disposable database and the production router, never source-text assertions.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::config::Config;
use crate::entity_creation_tests::Database;
use crate::state::AppState;
use crate::util::jwt;

const SECRET: &str = "application-setup-disposable-test-only";

struct Fixture {
    router: Router,
    admin: String,
    limited: String,
    application: Uuid,
    tenant: Uuid,
    organization: Uuid,
    role: Uuid,
    entry_menu: Uuid,
    button_menu: Uuid,
    foreign_tenant: Uuid,
    foreign_organization: Uuid,
    foreign_role: Uuid,
    foreign_menu: Uuid,
    managed_role: Uuid,
}

async fn create_tenant(pool: &PgPool, code: &str, platform: bool) -> Result<(Uuid, Uuid)> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants(name,code,is_platform,id) VALUES($1,$1,$2,$3) RETURNING id",
    )
    .bind(code)
    .bind(platform)
    .bind(crate::util::uuid7::new_uuid())
    .fetch_one(pool)
    .await?;
    let organization = sqlx::query_scalar(
        "INSERT INTO organizations(tenant_id,name,id) VALUES($1,$2,$3) RETURNING id",
    )
    .bind(id)
    .bind(code)
    .bind(crate::util::uuid7::new_uuid())
    .fetch_one(pool)
    .await?;
    Ok((id, organization))
}

async fn create_role(pool: &PgPool, tenant: Uuid, organization: Uuid, code: &str) -> Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO roles(tenant_id,organization_id,name,code,id) VALUES($1,$2,$3,$3,$4) RETURNING id",
    )
    .bind(tenant)
    .bind(organization)
    .bind(code)
    .bind(crate::util::uuid7::new_uuid()).fetch_one(pool)
    .await?)
}

async fn user_session(
    pool: &PgPool,
    tenant: Uuid,
    organization: Uuid,
    role: Uuid,
    name: &str,
) -> Result<String> {
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users(username,password_hash,is_super_admin,id) VALUES($1,'test-only-unused-hash',false,$2) RETURNING id",
    )
    .bind(name)
    .bind(crate::util::uuid7::new_uuid()).fetch_one(pool)
    .await?;
    sqlx::query("INSERT INTO memberships(user_id,organization_id,id) VALUES($1,$2,$3)")
        .bind(user)
        .bind(organization)
        .bind(crate::util::uuid7::new_uuid())
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3)")
        .bind(user)
        .bind(role)
        .bind(organization)
        .execute(pool)
        .await?;
    let sid: Uuid = sqlx::query_scalar(
        "INSERT INTO platform_sessions(user_id,tenant_id,organization_id,expires_at,id,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version) \
         SELECT $1,$2,$3,now()+interval '1 hour',$4,u.session_version,m.id,m.member_type,m.identity_version,'urn:saas:legacy-platform','platform-console-legacy',c.access_policy_version \
         FROM users u JOIN memberships m ON m.user_id=u.id AND m.organization_id=$3 AND m.member_type='EMPLOYEE' \
         JOIN oauth_clients c ON c.client_id='platform-console-legacy' WHERE u.id=$1 RETURNING id",
    )
    .bind(user)
    .bind(tenant)
    .bind(organization)
    .bind(crate::util::uuid7::new_uuid()).fetch_one(pool)
    .await?;
    let now = chrono::Utc::now().timestamp();
    jwt::sign(
        &jwt::Claims {
            sub: user.to_string(),
            typ: "session".into(),
            tenant: Some(tenant.to_string()),
            org: Some(organization.to_string()),
            cid: None,
            scope: None,
            sid: Some(sid.to_string()),
            ver: Some(0),
            aud: None,
            jti: None,
            iat: now,
            exp: now + 3600,
        },
        SECRET,
    )
}

impl Fixture {
    async fn create(pool: &PgPool) -> Result<Self> {
        let (platform, root) = create_tenant(pool, "admin-tenant", true).await?;
        let management: Uuid = sqlx::query_scalar(
            "INSERT INTO applications(name,code,base_path,id) VALUES('Management','system','/system',$1) RETURNING id",
        )
        .bind(crate::util::uuid7::new_uuid()).fetch_one(pool)
        .await?;
        let admin_role = create_role(pool, platform, root, "setup-admin").await?;
        let limited_role = create_role(pool, platform, root, "setup-reader").await?;
        for permission in [
            "system:application",
            "system:tenant",
            "system:authorization",
            "system:role",
        ] {
            let menu: Uuid = sqlx::query_scalar(
                "INSERT INTO menus(application_id,title,type,permission,id) VALUES($1,$2,'BUTTON',$2,$3) RETURNING id",
            )
            .bind(management)
            .bind(permission)
            .bind(crate::util::uuid7::new_uuid()).fetch_one(pool)
            .await?;
            sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
                .bind(platform)
                .bind(menu)
                .execute(pool)
                .await?;
            sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
                .bind(root)
                .bind(menu)
                .execute(pool)
                .await?;
            sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)")
                .bind(admin_role)
                .bind(menu)
                .execute(pool)
                .await?;
            if permission == "system:application" {
                sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)")
                    .bind(limited_role)
                    .bind(menu)
                    .execute(pool)
                    .await?;
            }
        }
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
            .bind(platform)
            .bind(management)
            .execute(pool)
            .await?;
        sqlx::query(
            "INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)",
        )
        .bind(root)
        .bind(management)
        .execute(pool)
        .await?;
        let admin = user_session(pool, platform, root, admin_role, "setup-admin").await?;
        let limited = user_session(pool, platform, root, limited_role, "setup-reader").await?;
        let (tenant, organization) = create_tenant(pool, "setup-business", false).await?;
        let role = create_role(pool, tenant, organization, "setup-business-role").await?;
        let _ = user_session(pool, tenant, organization, role, "setup-business-user").await?;
        let managed_role: Uuid = sqlx::query_scalar(
            "INSERT INTO roles(tenant_id,organization_id,name,code,built_in,id) VALUES($1,$2,'Managed administrator','tenant_admin',true,$3) RETURNING id",
        ).bind(tenant).bind(organization).bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?;
        let (foreign_tenant, foreign_organization) =
            create_tenant(pool, "setup-foreign", false).await?;
        let foreign_role = create_role(
            pool,
            foreign_tenant,
            foreign_organization,
            "setup-foreign-role",
        )
        .await?;
        let application: Uuid = sqlx::query_scalar(
            "INSERT INTO applications(name,code,base_path,entry_type,runtime_mode,entry_url,id) VALUES('Setup demo','setup-demo','/micro/setup-demo','HTML','TRUSTED','/sub-apps/setup-demo/unreleased/index.html',$1) RETURNING id",
        ).bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?;
        let entry_menu: Uuid = sqlx::query_scalar(
            "INSERT INTO menus(application_id,title,type,path,id) VALUES($1,'Home','MENU','/micro/setup-demo',$2) RETURNING id",
        ).bind(application).bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?;
        let button_menu: Uuid = sqlx::query_scalar(
            "INSERT INTO menus(application_id,title,type,permission,id) VALUES($1,'Edit','BUTTON','setup:edit',$2) RETURNING id",
        ).bind(application).bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?;
        let foreign_application: Uuid = sqlx::query_scalar(
            "INSERT INTO applications(name,code,base_path,id) VALUES('Unrelated','setup-unrelated','/unrelated',$1) RETURNING id",
        ).bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?;
        let foreign_menu: Uuid = sqlx::query_scalar(
            "INSERT INTO menus(application_id,title,type,path,id) VALUES($1,'Unrelated','MENU','/unrelated',$2) RETURNING id",
        ).bind(foreign_application).bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?;
        // This pre-existing chain must survive setup of a different application.
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
            .bind(tenant)
            .bind(foreign_application)
            .execute(pool)
            .await?;
        sqlx::query(
            "INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)",
        )
        .bind(organization)
        .bind(foreign_application)
        .execute(pool)
        .await?;
        sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
            .bind(tenant)
            .bind(foreign_menu)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
            .bind(organization)
            .bind(foreign_menu)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)")
            .bind(role)
            .bind(foreign_menu)
            .execute(pool)
            .await?;
        let config = Config::from_values(&HashMap::from([
            ("APPLICATION_ENVIRONMENT".into(), "development".into()),
            (
                "DATABASE_URL".into(),
                "postgres://unused/application-setup-tests".into(),
            ),
            ("JWT_SECRET".into(), SECRET.into()),
        ]))?;
        Ok(Self {
            router: crate::routes::api_router()
                .with_state(AppState::new(pool.clone(), Arc::new(config))),
            admin,
            limited,
            application,
            tenant,
            organization,
            role,
            entry_menu,
            button_menu,
            foreign_tenant,
            foreign_organization,
            foreign_role,
            foreign_menu,
            managed_role,
        })
    }

    fn setup_path(&self, target: bool) -> String {
        let mut path = format!(
            "/api/applications/{}/setup?environment=development",
            self.application
        );
        if target {
            path.push_str(&format!(
                "&tenantId={}&organizationId={}&roleId={}",
                self.tenant, self.organization, self.role
            ));
        }
        path
    }

    fn body(&self) -> Value {
        json!({"tenantId": self.tenant, "organizationId": self.organization, "roleId": self.role,
            "menuIds": [self.entry_menu, self.button_menu]})
    }

    async fn call(
        &self,
        token: &str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<(StatusCode, Value)> {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {token}"));
        let bytes = match body {
            Some(body) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                serde_json::to_vec(&body)?
            }
            None => Vec::new(),
        };
        let response = tokio::time::timeout(
            Duration::from_secs(30),
            self.router
                .clone()
                .oneshot(request.body(Body::from(bytes))?),
        )
        .await
        .map_err(|_| anyhow!("setup API request timed out"))??;
        let status = response.status();
        let body = serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
        Ok((status, body))
    }

    async fn grant(&self, token: &str, body: Value) -> Result<(StatusCode, Value)> {
        self.call(
            token,
            Method::POST,
            &format!("/api/applications/{}/setup/access", self.application),
            Some(body),
        )
        .await
    }

    async fn snapshot(&self, pool: &PgPool) -> Result<Vec<i64>> {
        let counts: (i64, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM tenant_applications WHERE tenant_id=$1), \
             (SELECT count(*) FROM organization_applications WHERE organization_id=$2), \
             (SELECT count(*) FROM tenant_menus WHERE tenant_id=$1), \
             (SELECT count(*) FROM organization_menus WHERE organization_id=$2), \
             (SELECT count(*) FROM role_menus WHERE role_id=$3)",
        )
        .bind(self.tenant)
        .bind(self.organization)
        .bind(self.role)
        .fetch_one(pool)
        .await?;
        Ok(vec![counts.0, counts.1, counts.2, counts.3, counts.4])
    }
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn application_setup_preserves_grants_and_reports_real_environment_and_access() -> Result<()>
{
    let database = Database::create().await?;
    let result = async {
        let h = Fixture::create(&database.pool).await?;
        let (status, setup) = h.call(&h.admin, Method::GET, &h.setup_path(false), None).await?;
        ensure!(status == StatusCode::OK && setup["releaseRequired"] == true && setup["activeVersion"].is_null()
            && setup["releaseValidated"] == false && setup["entryMenuCount"] == 1 && setup["access"].is_null(), "unpublished application reported ready: {setup}");
        ensure!(setup["canConfigureAccess"] == true, "ordinary authorized platform administrator was rejected");
        let before = h.snapshot(&database.pool).await?;
        let mut duplicate_selection = h.body();
        duplicate_selection["menuIds"] = json!([h.entry_menu, h.entry_menu, h.button_menu]);
        let (status, body) = h.grant(&h.admin, duplicate_selection).await?;
        ensure!(status == StatusCode::OK && body["ok"] == true, "additive setup failed: {status} {body}");
        let after = h.snapshot(&database.pool).await?;
        ensure!(after == before.iter().zip([1, 1, 2, 2, 2]).map(|(value, added)| value + added).collect::<Vec<_>>(), "setup replaced unrelated grants");
        let (status, body) = h.grant(&h.admin, h.body()).await?;
        ensure!(status == StatusCode::OK && body["ok"] == true && h.snapshot(&database.pool).await? == after, "repeated setup was not idempotent");
        let (status, setup) = h.call(&h.admin, Method::GET, &h.setup_path(true), None).await?;
        ensure!(status == StatusCode::OK && setup["access"]["tenantAuthorized"] == true
            && setup["access"]["organizationAuthorized"] == true && setup["access"]["roleAuthorized"] == true
            && setup["access"]["assignedUserCount"] == 1, "target readiness is incorrect: {setup}");
        // Ancestor role bindings remain valid with only a descendant employee.
        // Keep the role row while changing this owned fixture's memberships so
        // the read API must distinguish real support from stale configuration.
        let business_user: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE username='setup-business-user'")
            .fetch_one(&database.pool).await?;
        let child = option_organization(&database.pool,h.tenant,h.organization,"Setup employee child",false,false).await?;
        let child_employee = crate::util::uuid7::new_uuid();
        let mut moved = database.pool.begin().await?;
        sqlx::query("DELETE FROM memberships WHERE user_id=$1 AND organization_id=$2 AND member_type='EMPLOYEE'")
            .bind(business_user).bind(h.organization).execute(&mut *moved).await?;
        sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES($1,$2,$3,'EMPLOYEE')")
            .bind(child_employee).bind(business_user).bind(child).execute(&mut *moved).await?;
        moved.commit().await?;
        let (status, setup) = h.call(&h.admin,Method::GET,&h.setup_path(true),None).await?;
        ensure!(status==StatusCode::OK && setup["access"]["assignedUserCount"]==1,
            "descendant employee no longer supported its ancestor role: {status} {setup}");
        sqlx::query("UPDATE memberships SET status='DISABLED',identity_version=identity_version+1 WHERE id=$1")
            .bind(child_employee).execute(&database.pool).await?;
        let (status, setup) = h.call(&h.admin,Method::GET,&h.setup_path(true),None).await?;
        ensure!(status==StatusCode::OK && setup["access"]["assignedUserCount"]==0,
            "disabled employee still counted as assigned role support: {status} {setup}");
        sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES($1,$2,$3,'CUSTOMER')")
            .bind(crate::util::uuid7::new_uuid()).bind(business_user).bind(h.organization).execute(&database.pool).await?;
        let (status, setup) = h.call(&h.admin,Method::GET,&h.setup_path(true),None).await?;
        ensure!(status==StatusCode::OK && setup["access"]["assignedUserCount"]==0,
            "customer substituted for a disabled employee supporting the ancestor role: {status} {setup}");
        sqlx::query("DELETE FROM memberships WHERE id=$1").bind(child_employee).execute(&database.pool).await?;
        let (status, setup) = h.call(&h.admin,Method::GET,&h.setup_path(true),None).await?;
        ensure!(status==StatusCode::OK && setup["access"]["assignedUserCount"]==0,
            "customer-only account counted as assigned employee support: {status} {setup}");
        sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES($1,$2,$3,'EMPLOYEE')")
            .bind(crate::util::uuid7::new_uuid()).bind(business_user).bind(child).execute(&database.pool).await?;
        let (status, setup) = h.call(&h.admin,Method::GET,&h.setup_path(true),None).await?;
        ensure!(status==StatusCode::OK && setup["access"]["assignedUserCount"]==1,
            "customer membership prevented a valid descendant employee from supporting its role: {status} {setup}");
        let release: Uuid = sqlx::query_scalar(
            "INSERT INTO application_releases(application_id,environment,version,entry_type,entry_url,validation_status,checked_at,id) VALUES($1,'development','setup-v1','HTML','/sub-apps/setup-demo/versions/setup-v1/index.html','PASSED',now(),$2) RETURNING id",
        ).bind(h.application).bind(crate::util::uuid7::new_uuid()).fetch_one(&database.pool).await?;
        sqlx::query("INSERT INTO application_deployments(application_id,environment,active_release_id) VALUES($1,'development',$2)")
            .bind(h.application).bind(release).execute(&database.pool).await?;
        let (status, setup) = h.call(&h.admin, Method::GET, &h.setup_path(false), None).await?;
        ensure!(status == StatusCode::OK && setup["activeVersion"] == "setup-v1" && setup["releaseValidated"] == true, "active release not reflected: {setup}");
        let staging_path = h.setup_path(false).replace("environment=development", "environment=staging");
        let (status, setup) = h.call(&h.admin, Method::GET, &staging_path, None).await?;
        ensure!(status == StatusCode::OK && setup["activeVersion"].is_null() && setup["releaseValidated"] == false, "development release leaked into staging");
        sqlx::query("DELETE FROM organization_menus WHERE organization_id=$1 AND menu_id=$2")
            .bind(h.organization).bind(h.entry_menu).execute(&database.pool).await?;
        let (_, setup) = h.call(&h.admin, Method::GET, &h.setup_path(true), None).await?;
        ensure!(setup["access"]["organizationAuthorized"] == false && setup["access"]["roleAuthorized"] == false,
            "button-only or disconnected grant was reported as a usable entry: {setup}");
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn application_setup_rejects_cross_scope_disabled_targets_and_missing_permissions(
) -> Result<()> {
    let database = Database::create().await?;
    let result = async {
        let h = Fixture::create(&database.pool).await?;
        let before = h.snapshot(&database.pool).await?;
        for (key, value) in [
            ("tenantId", json!(h.foreign_tenant)),
            ("organizationId", json!(h.foreign_organization)),
            ("roleId", json!(h.foreign_role)),
            ("menuIds", json!([h.foreign_menu])),
            ("menuIds", json!([])),
        ] {
            let mut body = h.body();
            body[key] = value;
            let (status, _) = h.grant(&h.admin, body).await?;
            ensure!(
                matches!(status, StatusCode::BAD_REQUEST | StatusCode::FORBIDDEN),
                "cross-scope {key} accepted or returned server failure: {status}"
            );
            ensure!(
                h.snapshot(&database.pool).await? == before,
                "rejected {key} left partial grants"
            );
        }
        let mut managed = h.body();
        managed["roleId"] = json!(h.managed_role);
        ensure!(
            h.grant(&h.admin, managed).await?.0 == StatusCode::CONFLICT,
            "setup modified a managed administrator role"
        );
        ensure!(
            h.grant(&h.limited, h.body()).await?.0 == StatusCode::FORBIDDEN,
            "application-only permission authorized access writes"
        );
        let (status, setup) = h
            .call(&h.limited, Method::GET, &h.setup_path(false), None)
            .await?;
        ensure!(
            status == StatusCode::OK
                && setup["canConfigureAccess"] == false
                && setup["access"].is_null(),
            "limited reader received access data"
        );
        ensure!(
            h.call(&h.limited, Method::GET, &h.setup_path(true), None)
                .await?
                .0
                == StatusCode::FORBIDDEN,
            "limited reader queried tenant/role access"
        );
        let partial = format!("{}&tenantId={}", h.setup_path(false), h.tenant);
        ensure!(
            h.call(&h.admin, Method::GET, &partial, None).await?.0 == StatusCode::BAD_REQUEST,
            "partial target accepted"
        );
        let invalid_environment = h.setup_path(false).replace("development", "unrecognized");
        ensure!(
            h.call(&h.admin, Method::GET, &invalid_environment, None)
                .await?
                .0
                == StatusCode::BAD_REQUEST,
            "unknown environment accepted"
        );
        for (table, id) in [
            ("tenants", h.tenant),
            ("organizations", h.organization),
            ("roles", h.role),
            ("applications", h.application),
        ] {
            sqlx::query(&format!("UPDATE {table} SET status='DISABLED' WHERE id=$1"))
                .bind(id)
                .execute(&database.pool)
                .await?;
            let (status, _) = h.grant(&h.admin, h.body()).await?;
            sqlx::query(&format!("UPDATE {table} SET status='ENABLED' WHERE id=$1"))
                .bind(id)
                .execute(&database.pool)
                .await?;
            ensure!(
                matches!(status, StatusCode::BAD_REQUEST | StatusCode::FORBIDDEN),
                "disabled {table} accepted or returned server failure: {status}"
            );
            ensure!(
                h.snapshot(&database.pool).await? == before,
                "disabled {table} left partial grants"
            );
        }
        Ok(())
    }
    .await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn application_setup_audit_failure_rolls_back_every_authorization_layer() -> Result<()> {
    let database = Database::create().await?;
    let result = async {
        let h = Fixture::create(&database.pool).await?;
        let before = h.snapshot(&database.pool).await?;
        for mode in ["raise", "skip"] {
            let fault = if mode == "raise" { "RAISE EXCEPTION 'owned setup audit failure';" } else { "RETURN NULL;" };
            sqlx::raw_sql(&format!(
                "CREATE OR REPLACE FUNCTION reject_setup_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN \
                 IF NEW.action='APPLICATION_SETUP_ACCESS' THEN {fault} END IF; RETURN NEW; END $$; \
                 CREATE TRIGGER reject_setup_audit BEFORE INSERT ON audit_logs FOR EACH ROW EXECUTE FUNCTION reject_setup_audit();",
            )).execute(&database.pool).await?;
            let (status, _) = h.grant(&h.admin, h.body()).await?;
            sqlx::query("DROP TRIGGER reject_setup_audit ON audit_logs").execute(&database.pool).await?;
            ensure!(status == StatusCode::INTERNAL_SERVER_ERROR, "audit {mode} failure did not abort setup: {status}");
            ensure!(h.snapshot(&database.pool).await? == before, "audit {mode} persisted partial grants");
            let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='APPLICATION_SETUP_ACCESS'")
                .fetch_one(&database.pool).await?;
            ensure!(audits == 0, "audit {mode} left a success record");
        }
        ensure!(h.grant(&h.admin, h.body()).await?.0 == StatusCode::OK, "setup could not recover after audit fault removal");
        Ok(())
    }.await;
    database.finish(result).await
}

async fn wait_for_user_lock(pool: &PgPool, blocker: i32) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
                 AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)) \
                 AND query LIKE '%FROM users%' AND query LIKE '%FOR UPDATE%')",
            )
            .bind(blocker)
            .fetch_one(pool)
            .await?;
            if waiting {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(|_| anyhow!("setup request did not reach the observed actor-user lock barrier"))?
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn application_setup_rechecks_permission_and_session_after_user_lock_wait() -> Result<()> {
    let database = Database::create().await?;
    let result = async {
        let h = Arc::new(Fixture::create(&database.pool).await?);
        let before = h.snapshot(&database.pool).await?;
        let actor: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE username='setup-admin'")
            .fetch_one(&database.pool)
            .await?;
        let actor_role: Uuid =
            sqlx::query_scalar("SELECT role_id FROM user_roles WHERE user_id=$1")
                .bind(actor)
                .fetch_one(&database.pool)
                .await?;
        let permission_menu: Uuid =
            sqlx::query_scalar("SELECT id FROM menus WHERE permission='system:role'")
                .fetch_one(&database.pool)
                .await?;
        for mode in ["permission", "session"] {
            let mut blocker = database.pool.begin().await?;
            let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *blocker)
                .await?;
            sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
                .bind(actor)
                .execute(&mut *blocker)
                .await?;
            let pending_h = h.clone();
            let pending =
                tokio::spawn(
                    async move { pending_h.grant(&pending_h.admin, pending_h.body()).await },
                );
            if let Err(error) = wait_for_user_lock(&database.pool, blocker_pid).await {
                blocker.rollback().await?;
                let _ = pending.await;
                return Err(error);
            }
            if mode == "permission" {
                sqlx::query("DELETE FROM role_menus WHERE role_id=$1 AND menu_id=$2")
                    .bind(actor_role)
                    .bind(permission_menu)
                    .execute(&mut *blocker)
                    .await?;
            } else {
                sqlx::query("UPDATE platform_sessions SET revoked_at=now() WHERE user_id=$1")
                    .bind(actor)
                    .execute(&mut *blocker)
                    .await?;
            }
            blocker.commit().await?;
            let (status, _) = pending.await??;
            let expected = if mode == "permission" {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::UNAUTHORIZED
            };
            ensure!(
                status == expected,
                "setup {mode} lock wait returned {status}, expected {expected}"
            );
            ensure!(
                h.snapshot(&database.pool).await? == before,
                "setup {mode} lock wait left partial grants"
            );
            if mode == "permission" {
                sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)")
                    .bind(actor_role)
                    .bind(permission_menu)
                    .execute(&database.pool)
                    .await?;
            }
        }
        let audit_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_logs WHERE action='APPLICATION_SETUP_ACCESS'",
        )
        .fetch_one(&database.pool)
        .await?;
        ensure!(
            audit_count == 0,
            "revoked setup request recorded a successful access change"
        );
        Ok(())
    }
    .await;
    database.finish(result).await
}

async fn option_organization(
    pool: &PgPool,
    tenant: Uuid,
    parent: Uuid,
    name: &str,
    disabled: bool,
    expired: bool,
) -> Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO organizations(tenant_id,parent_id,name,status,expires_at,contact_name,contact_phone,address,id) \
         VALUES($1,$2,$3,CASE WHEN $4 THEN 'DISABLED' ELSE 'ENABLED' END, \
         CASE WHEN $5 THEN now()-interval '1 day' ELSE NULL END,'private contact','13800000000','private address',$6) RETURNING id",
    )
    .bind(tenant)
    .bind(parent)
    .bind(name)
    .bind(disabled)
    .bind(expired)
    .bind(crate::util::uuid7::new_uuid()).fetch_one(pool)
    .await?)
}

fn option_ids_and_validate_fields(
    nodes: &[Value],
    tenant: Uuid,
    ids: &mut Vec<Uuid>,
) -> Result<()> {
    for node in nodes {
        let object = node
            .as_object()
            .ok_or_else(|| anyhow!("organization option is not an object"))?;
        let expected = [
            "id",
            "name",
            "code",
            "status",
            "tenantId",
            "parentId",
            "expiresAt",
            "children",
        ];
        ensure!(
            object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key)),
            "organization options disclose extra fields or omit contract fields: {node}"
        );
        ensure!(
            node["tenantId"] == json!(tenant) && node["status"] == "ENABLED",
            "organization options escaped tenant or active state"
        );
        ids.push(
            node["id"]
                .as_str()
                .ok_or_else(|| anyhow!("option id missing"))?
                .parse()?,
        );
        let children = node["children"]
            .as_array()
            .ok_or_else(|| anyhow!("option children missing"))?;
        option_ids_and_validate_fields(children, tenant, ids)?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn application_setup_organization_options_are_minimal_and_filter_inactive_subtrees(
) -> Result<()> {
    let database = Database::create().await?;
    let result = async {
        let h = Fixture::create(&database.pool).await?;
        let child = option_organization(&database.pool, h.tenant, h.organization, "active child", false, false).await?;
        let disabled = option_organization(&database.pool, h.tenant, h.organization, "disabled parent", true, false).await?;
        option_organization(&database.pool, h.tenant, disabled, "enabled under disabled", false, false).await?;
        let expired = option_organization(&database.pool, h.tenant, h.organization, "expired parent", false, true).await?;
        option_organization(&database.pool, h.tenant, expired, "enabled under expired", false, false).await?;
        let base = format!("/api/applications/{}/setup/organizations", h.application);
        let path = format!("{base}?tenantId={}", h.tenant);
        let (status, options) = h.call(&h.admin, Method::GET, &path, None).await?;
        ensure!(status == StatusCode::OK, "setup administrator without system:organization could not load options: {status} {options}");
        let nodes = options.as_array().ok_or_else(|| anyhow!("organization options are not a tree array"))?;
        let mut ids = Vec::new();
        option_ids_and_validate_fields(nodes, h.tenant, &mut ids)?;
        ids.sort_unstable();
        let mut expected = vec![h.organization, child];
        expected.sort_unstable();
        ensure!(ids == expected, "organization options exposed inactive descendants or other tenants");
        ensure!(h.call(&h.limited, Method::GET, &path, None).await?.0 == StatusCode::FORBIDDEN, "application-only reader received access options");
        ensure!(h.call(&h.admin, Method::GET, &base, None).await?.0 == StatusCode::BAD_REQUEST, "organization options accepted absent tenant");
        ensure!(h.call(&h.admin, Method::GET, &format!("{base}?tenantId={}", Uuid::nil()), None).await?.0 == StatusCode::BAD_REQUEST, "organization options accepted nil tenant");
        sqlx::query("UPDATE tenants SET status='DISABLED' WHERE id=$1").bind(h.tenant).execute(&database.pool).await?;
        ensure!(h.call(&h.admin, Method::GET, &path, None).await?.0 == StatusCode::FORBIDDEN, "organization options accepted disabled tenant");
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn application_setup_locks_actor_before_target_role() -> Result<()> {
    let database = Database::create().await?;
    let result = async {
        let h = Arc::new(Fixture::create(&database.pool).await?);
        let actor: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE username='setup-admin'")
            .fetch_one(&database.pool).await?;
        let mut blocker = database.pool.begin().await?;
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *blocker).await?;
        sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
            .bind(actor).execute(&mut *blocker).await?;
        let pending_h = h.clone();
        let pending = tokio::spawn(async move { pending_h.grant(&pending_h.admin, pending_h.body()).await });
        if let Err(error) = wait_for_user_lock(&database.pool, blocker_pid).await {
            blocker.rollback().await?;
            let _ = pending.await;
            return Err(error);
        }
        // Existing tenant/organization edits take parents before existing users.
        // At the user barrier, those parent locks must already be held by setup.
        for (table, id) in [("tenants", h.tenant), ("organizations", h.organization)] {
            let mut probe = database.pool.begin().await?;
            let available = sqlx::query_scalar::<_, Uuid>(&format!(
                "SELECT id FROM {table} WHERE id=$1 FOR NO KEY UPDATE NOWAIT",
            ))
            .bind(id)
            .fetch_one(&mut *probe)
            .await;
            let parent_locked = matches!(available, Err(ref error)
                if error.as_database_error().and_then(|error| error.code()).as_deref() == Some("55P03"));
            probe.rollback().await?;
            if !parent_locked {
                blocker.rollback().await?;
                let _ = pending.await;
                return Err(anyhow!(
                    "setup waited for actor user before locking target {table}, reversing parent/user lock order"
                ));
            }
        }
        // A role-membership edit locks user U then role R. Setup must not hold R
        // while waiting for U; prove R remains available at that exact barrier.
        sqlx::query("SET LOCAL lock_timeout='1s'").execute(&mut *blocker).await?;
        let available = tokio::time::timeout(
            Duration::from_secs(2),
            sqlx::query_scalar::<_, Uuid>("SELECT id FROM roles WHERE id=$1 FOR NO KEY UPDATE")
                .bind(h.role).fetch_one(&mut *blocker),
        ).await;
        if !matches!(available, Ok(Ok(id)) if id == h.role) {
            blocker.rollback().await?;
            let _ = pending.await;
            return Err(anyhow!("setup held target role while waiting for actor user, reversing membership lock order"));
        }
        blocker.commit().await?;
        let (status, body) = pending.await??;
        ensure!(status == StatusCode::OK && body["ok"] == true, "setup did not complete after releasing actor lock: {status} {body}");
        Ok(())
    }.await;
    database.finish(result).await
}
