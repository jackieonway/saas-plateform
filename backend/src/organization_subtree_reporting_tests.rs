//! Organization subtree reporting on an owned, migrated database.
//! Run with `cargo test organization_subtree_reporting -- --ignored --nocapture`.
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::config::Config;
use crate::routes;

const SECRET: &str = "organization-subtree-reporting-tests-only";

fn database_options() -> Result<PgConnectOptions> {
    let url = match std::env::var("TEST_DATABASE_URL") {
        Ok(url) if !url.trim().is_empty() => url,
        _ => {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env.test");
            let entries = dotenvy::from_path_iter(path).map_err(|_| {
                anyhow!("configure TEST_DATABASE_URL or ignored root backend/.env.test")
            })?;
            let mut url = None;
            for entry in entries {
                let (key, value) = entry.map_err(|_| anyhow!("invalid test environment file"))?;
                if key == "TEST_DATABASE_URL" && !value.trim().is_empty() {
                    url = Some(value);
                }
            }
            url.ok_or_else(|| anyhow!("TEST_DATABASE_URL is missing"))?
        }
    };
    PgConnectOptions::from_str(&url)
        .map_err(|_| anyhow!("invalid test database connection configuration"))
}

struct Database {
    pool: PgPool,
    admin: PgPool,
    name: String,
}

impl Database {
    async fn create() -> Result<Self> {
        let options = database_options()?;
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("test database administration connection failed"))?;
        let name = format!(
            "organization_subtree_reporting_test_{}",
            crate::util::uuid7::new_uuid().simple()
        );
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await
            .map_err(|_| {
                anyhow!("creating isolated organization subtree reporting test database failed")
            })?;
        let pool = match PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.database(&name))
            .await
        {
            Ok(pool) => pool,
            Err(_) => {
                let _ = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
                    .execute(&admin)
                    .await;
                return Err(anyhow!(
                    "isolated organization subtree reporting test database connection failed"
                ));
            }
        };
        let database = Self { pool, admin, name };
        if !matches!(
            tokio::time::timeout(
                Duration::from_secs(30),
                sqlx::migrate!("./migrations").run(&database.pool),
            )
            .await,
            Ok(Ok(()))
        ) {
            database
                .finish(Err(anyhow!(
                    "isolated organization subtree reporting test migrations failed"
                )))
                .await?;
            return Err(anyhow!(
                "isolated organization subtree reporting test migrations failed"
            ));
        }
        Ok(database)
    }

    async fn finish(self, result: Result<()>) -> Result<()> {
        ensure!(
            self.name
                .starts_with("organization_subtree_reporting_test_")
                && self
                    .name
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_'),
            "refusing cleanup outside the owned organization subtree reporting test database"
        );
        let initially_closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let removed = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await
            .is_ok();
        let finally_closed = initially_closed
            || tokio::time::timeout(Duration::from_secs(5), self.pool.close())
                .await
                .is_ok();
        let _ = tokio::time::timeout(Duration::from_secs(5), self.admin.close()).await;
        eprintln!(
            "organization subtree reportings cleanup assertions_ok={} database_removed={} pool_closed={}",
            result.is_ok(),
            removed,
            finally_closed
        );
        result?;
        ensure!(
            removed && finally_closed,
            "isolated organization subtree reporting test cleanup failed"
        );
        Ok(())
    }
}

async fn organization(
    pool: &PgPool,
    tenant: Uuid,
    parent: Option<Uuid>,
    name: &str,
) -> Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO organizations(tenant_id,parent_id,name,code,id) VALUES($1,$2,$3,$3,$4) RETURNING id",
    )
    .bind(tenant)
    .bind(parent)
    .bind(name)
    .bind(crate::util::uuid7::new_uuid()).fetch_one(pool)
    .await?)
}

async fn user(pool: &PgPool, name: &str, org: Uuid, super_admin: bool) -> Result<Uuid> {
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users(username,password_hash,is_super_admin,id) VALUES($1,'test-only',$2,$3) RETURNING id",
    )
    .bind(name)
    .bind(super_admin)
    .bind(crate::util::uuid7::new_uuid()).fetch_one(pool)
    .await?;
    sqlx::query("INSERT INTO memberships(user_id,organization_id,id) VALUES($1,$2,$3)")
        .bind(user)
        .bind(org)
        .bind(crate::util::uuid7::new_uuid())
        .execute(pool)
        .await?;
    Ok(user)
}

async fn session(pool: &PgPool, user: Uuid, tenant: Uuid, org: Uuid) -> Result<String> {
    crate::sso_test_fixture::token(pool, user, tenant, org, SECRET).await
}

async fn call(
    app: &Router,
    token: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
    expected: StatusCode,
) -> Result<Value> {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            body.map(|body| body.to_string()).unwrap_or_default(),
        ))?;
    let response = tokio::time::timeout(Duration::from_secs(30), app.clone().oneshot(request))
        .await
        .map_err(|_| anyhow!("organization subtree reporting request timed out"))??;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    let body: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)?
    };
    ensure!(
        status == expected,
        "{path} returned {status}, expected {expected}: {body}"
    );
    Ok(body)
}

async fn csv(app: &Router, token: &str, path: &str) -> Result<String> {
    let request = Request::builder()
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())?;
    let response = app.clone().oneshot(request).await?;
    ensure!(response.status() == StatusCode::OK, "{path} failed");
    Ok(String::from_utf8(
        to_bytes(response.into_body(), 1024 * 1024).await?.to_vec(),
    )?)
}

async fn reporting_scope(platform_tenant: bool) -> Result<()> {
    let database = Database::create().await?;
    let pool = &database.pool;
    let result = async {
        let tenant: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(name,code,is_platform,id) VALUES('report tenant',$1,$2,$3) RETURNING id",
        ).bind(if platform_tenant { "admin-tenant" } else { "report-tenant" })
            .bind(platform_tenant).bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?;
        let foreign_tenant: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(name,code,id) VALUES('report foreign tenant','report-foreign',$1) RETURNING id",
        ).bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?;
        let ancestor = organization(pool, tenant, None, "report ancestor").await?;
        let current = organization(pool, tenant, Some(ancestor), "report current").await?;
        let child = organization(pool, tenant, Some(current), "report descendant").await?;
        let sibling = organization(pool, tenant, Some(ancestor), "report sibling").await?;
        let foreign = organization(pool, foreign_tenant, None, "report foreign").await?;
        let manager = user(pool, "report manager", current, false).await?;
        let global_admin = user(pool, "report global admin", current, true).await?;
        for account in [manager, global_admin] {
            for org in [ancestor, sibling] {
                sqlx::query("INSERT INTO memberships(user_id,organization_id,id) VALUES($1,$2,$3)")
                    .bind(account).bind(org).bind(crate::util::uuid7::new_uuid()).execute(pool).await?;
            }
        }
        let app_id: Uuid = sqlx::query_scalar(
            "INSERT INTO applications(name,code,id) VALUES('report system','system',$1) RETURNING id",
        ).bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?;
        let ancestor_only_app: Uuid = sqlx::query_scalar(
            "INSERT INTO applications(name,code,id) VALUES('report ancestor app','report-ancestor-app',$1) RETURNING id",
        ).bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?;
        for app in [app_id, ancestor_only_app] {
            sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
                .bind(tenant).bind(app).execute(pool).await?;
        }
        for org in [ancestor, current, child, sibling] {
            sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)")
                .bind(org).bind(app_id).execute(pool).await?;
        }
        sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)")
            .bind(ancestor).bind(ancestor_only_app).execute(pool).await?;
        let mut menus = Vec::new();
        for permission in ["system:tenant", "system:user", "system:role", "system:organization", "system:audit", "system:monitor"] {
            let menu: Uuid = sqlx::query_scalar(
                "INSERT INTO menus(application_id,title,permission,type,id) VALUES($1,$2,$2,'MENU',$3) RETURNING id",
            ).bind(app_id).bind(permission).bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?;
            menus.push(menu);
            sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
                .bind(tenant).bind(menu).execute(pool).await?;
            for org in [ancestor, current, child, sibling] {
                sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
                    .bind(org).bind(menu).execute(pool).await?;
            }
        }
        let mut ancestor_role = Uuid::nil();
        for (org, name) in [(ancestor,"report ancestor"),(current,"report current"),(child,"report descendant"),(sibling,"report sibling")] {
            let report_user = user(pool, &format!("{name} user"), org, false).await?;
            crate::sso_test_fixture::session(pool, report_user, tenant, org).await?;
            let role: Uuid = sqlx::query_scalar(
                "INSERT INTO roles(tenant_id,organization_id,name,code,id) VALUES($1,$2,$3,$3,$4) RETURNING id",
            ).bind(tenant).bind(org).bind(format!("{name} role")).bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?;
            if org == ancestor { ancestor_role = role; }
            sqlx::query("INSERT INTO audit_logs(tenant_id,organization_id,username,organization_name,action,detail) \
                         VALUES($1,$2,$3,$3,'LOGIN',$3)")
                .bind(tenant).bind(org).bind(name).execute(pool).await?;
        }
        user(pool, "report foreign user", foreign, false).await?;
        sqlx::query("INSERT INTO audit_logs(tenant_id,organization_id,username,organization_name,action,detail) \
                     VALUES($1,$2,'report foreign','report foreign','LOGIN','report foreign')")
            .bind(foreign_tenant).bind(foreign).execute(pool).await?;
        sqlx::query("INSERT INTO audit_logs(tenant_id,username,action,detail) \
                     VALUES($1,'report unassigned','LOGIN','report unassigned')")
            .bind(tenant).execute(pool).await?;
        // This disposable target-schema fixture exercises two concrete member
        // relations without changing the deployment migration/creation gates.
        sqlx::query("ALTER TABLE memberships DROP CONSTRAINT IF EXISTS memberships_user_id_organization_id_key")
            .execute(pool).await?;
        let customer = crate::util::uuid7::new_uuid();
        sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,status) VALUES($1,$2,$3,'CUSTOMER','ENABLED')")
            .bind(customer).bind(manager).bind(current).execute(pool).await?;
        let current_counts: (i64, i64) = sqlx::query_as(
            "SELECT count(*),count(DISTINCT user_id) FROM memberships WHERE organization_id=$1",
        ).bind(current).fetch_one(pool).await?;
        ensure!(current_counts == (4, 3), "dual fixture must preserve four relations for three accounts");
        sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3)")
            .bind(manager).bind(ancestor_role).bind(ancestor).execute(pool).await?;
        for menu in menus {
            sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)")
                .bind(ancestor_role).bind(menu).execute(pool).await?;
        }
        let config = Config::from_values(&HashMap::from([
            ("APPLICATION_ENVIRONMENT".into(), "development".into()),
            ("DATABASE_URL".into(), "postgres://unused/reporting-test".into()),
            ("JWT_SECRET".into(), SECRET.into()),
        ]))?;
        let app = routes::api_router().with_state(crate::sso_test_fixture::state(pool.clone(), Arc::new(config)));

        let report_tokens = [
            session(pool, manager, tenant, current).await?,
            session(pool, global_admin, tenant, current).await?,
        ];
        for token in &report_tokens {
            let organizations = call(&app, token, Method::GET,
                &format!("/api/organizations/tree?tenantId={tenant}"), None, StatusCode::OK).await?;
            let current_row = organizations.as_array().and_then(|rows| rows.iter().find(|row| row["id"] == json!(current)))
                .ok_or_else(|| anyhow!("current organization must remain the scoped tree root"))?;
            ensure!(current_row["userCount"] == 3,
                "same-account employee/customer rows were counted as two people: {organizations}");
            call(&app, token, Method::GET, "/api/search?q=%00", None, StatusCode::BAD_REQUEST).await?;
            for query in ["%25", "_", "%5C"] {
                let search = call(&app, token, Method::GET, &format!("/api/search?q={query}&limit=10"), None, StatusCode::OK).await?;
                ensure!(search == json!({"users":[],"tenants":[],"organizations":[],"roles":[]}), "search interpreted a literal keyword as a SQL pattern: {search}");
            }
            let search = call(&app, token, Method::GET, "/api/search?q=report&limit=10", None, StatusCode::OK).await?;
            for key in ["organizations", "roles"] {
                ensure!(search[key].as_array().is_some_and(|rows| rows.len() == 2), "unscoped {key}: {search}");
            }
            ensure!(search["users"].as_array().is_some_and(|rows| rows.len() == 4), "unscoped users: {search}");
            ensure!(!search.to_string().contains("report ancestor") && !search.to_string().contains("report sibling")
                && !search.to_string().contains("report foreign"), "search leaked outside subtree: {search}");
            ensure!(search["organizations"].as_array().unwrap().iter().all(|row|
                row["path"].as_str().is_some_and(|path| path.starts_with("report current"))), "relative path leaked ancestor");
            // Deliberate out-of-scope tenant filter cannot expand the scoped audit data.
            let audit = call(&app, token, Method::GET, "/api/audit-logs?size=100", None, StatusCode::OK).await?;
            ensure!(audit["total"] == 2 && !audit.to_string().contains("report ancestor")
                && !audit.to_string().contains("report sibling") && !audit.to_string().contains("report unassigned"), "unscoped audit: {audit}");
            let foreign_audit = call(&app, token, Method::GET,
                &format!("/api/audit-logs?tenant_id={foreign_tenant}"), None, StatusCode::OK).await?;
            ensure!(foreign_audit["total"] == 0, "foreign filter expanded audit scope");
            let audit_csv = csv(&app, token, "/api/audit-logs/export").await?;
            ensure!(audit_csv.contains("report current") && audit_csv.contains("report descendant")
                && !audit_csv.contains("report ancestor") && !audit_csv.contains("report sibling")
                && !audit_csv.contains("report unassigned") && !audit_csv.contains("report foreign"), "audit export leaked subtree");
            let tenants = call(&app, token, Method::GET, "/api/tenants", None, StatusCode::OK).await?;
            ensure!(tenants["total"] == 1 && tenants["records"][0]["organizationCount"] == 2
                && tenants["records"][0]["userCount"] == 4 && tenants["records"][0]["authorizedAppCount"] == 1,
                "tenant summary exposed upstream totals: {tenants}");
            let choices = call(&app, token, Method::GET, "/api/tenants/options", None, StatusCode::OK).await?;
            ensure!(choices.as_array().is_some_and(|rows| rows.len()==1) && choices[0]["id"]==json!(tenant), "unscoped tenant choice");
            let insights = call(&app, token, Method::GET, "/api/monitor/tenant-insights", None, StatusCode::OK).await?;
            let row = &insights["insights"][0];
            ensure!(insights["insights"].as_array().is_some_and(|rows| rows.len() == 1)
                && row["users"] == 4 && row["organizations"] == 2 && row["roles"] == 2
                && row["logins7d"] == 2 && row["activePlatformSessions"] == 4, "unscoped insights: {insights}");
            ensure!(row["loginsTrend"].as_array().unwrap().iter().filter_map(Value::as_i64).sum::<i64>() == 2,
                "login trend included outside subtree");
            let insight_csv = csv(&app, token, "/api/monitor/tenant-insights/export").await?;
            ensure!(insight_csv.contains(",4,2,2,2,4,") && !insight_csv.contains("report foreign"), "insights export totals not scoped");
            for path in ["/api/monitor/overview", "/api/monitor/system"] {
                call(&app, token, Method::GET, path, None, StatusCode::FORBIDDEN).await?;
            }
        }

        let root_token = session(pool, global_admin, tenant, ancestor).await?;
        let root_audit = call(&app, &root_token, Method::GET, "/api/audit-logs?size=100", None, StatusCode::OK).await?;
        ensure!(root_audit["total"] == if platform_tenant { 6 } else { 5 }, "tenant root lost legal audit visibility: {root_audit}");
        let root_tenants = call(&app, &root_token, Method::GET, "/api/tenants", None, StatusCode::OK).await?;
        ensure!(root_tenants["total"] == if platform_tenant { 2 } else { 1 }, "tenant root scope regressed");
        let current_summary = root_tenants["records"].as_array().unwrap().iter()
            .find(|row| row["id"] == json!(tenant)).unwrap();
        ensure!(current_summary["organizationCount"] == 4 && current_summary["userCount"] == 6,
            "root totals did not include complete tree");
        call(&app, &root_token, Method::GET, "/api/monitor/overview", None,
            if platform_tenant { StatusCode::OK } else { StatusCode::FORBIDDEN }).await?;
        let retained: (i64, bool) = sqlx::query_as(
            "SELECT count(*),bool_or(id=$3 AND member_type='CUSTOMER' AND status='ENABLED') \
             FROM memberships WHERE user_id=$1 AND organization_id=$2",
        ).bind(manager).bind(current).bind(customer).fetch_one(pool).await?;
        ensure!(retained == (2, true), "account counting must not delete or merge either concrete identity");
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn organization_subtree_reporting_in_ordinary_tenant() -> Result<()> {
    reporting_scope(false).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn organization_subtree_reporting_in_platform_tenant() -> Result<()> {
    reporting_scope(true).await
}
