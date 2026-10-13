//! Department organization choices on an owned, migrated PostgreSQL database.
//! Run with `cargo test department_organization_choices -- --ignored --nocapture`.
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::config::Config;
use crate::routes;
use crate::state::AppState;
use crate::util::jwt;

const SECRET: &str = "department-organizations-isolated-tests-only";

fn database_options() -> Result<PgConnectOptions> {
    let url = match std::env::var("TEST_DATABASE_URL") {
        Ok(url) if !url.trim().is_empty() => url,
        _ => {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.env.test");
            let entries = dotenvy::from_path_iter(path)
                .map_err(|_| anyhow!("configure TEST_DATABASE_URL or ignored root .env.test"))?;
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
        let name = format!("department_organizations_test_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await
            .map_err(|_| anyhow!("creating isolated department test database failed"))?;
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
                    "isolated department test database connection failed"
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
                .finish(Err(anyhow!("isolated department test migrations failed")))
                .await?;
            return Err(anyhow!("isolated department test migrations failed"));
        }
        Ok(database)
    }

    async fn finish(self, result: Result<()>) -> Result<()> {
        ensure!(
            self.name.starts_with("department_organizations_test_")
                && self
                    .name
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_'),
            "refusing cleanup outside the owned department test database"
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
            "department organizations cleanup assertions_ok={} database_removed={} pool_closed={}",
            result.is_ok(),
            removed,
            finally_closed
        );
        result?;
        ensure!(
            removed && finally_closed,
            "isolated department test cleanup failed"
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
        "INSERT INTO organizations(tenant_id,parent_id,name,code) VALUES($1,$2,$3,$3) RETURNING id",
    )
    .bind(tenant)
    .bind(parent)
    .bind(name)
    .fetch_one(pool)
    .await?)
}

async fn user(pool: &PgPool, name: &str, org: Uuid, super_admin: bool) -> Result<Uuid> {
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users(username,password_hash,is_super_admin) VALUES($1,'test-only',$2) RETURNING id",
    )
    .bind(name)
    .bind(super_admin)
    .fetch_one(pool)
    .await?;
    sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)")
        .bind(user)
        .bind(org)
        .execute(pool)
        .await?;
    Ok(user)
}

async fn session(pool: &PgPool, user: Uuid, tenant: Uuid, org: Uuid) -> Result<String> {
    let version: i64 = sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
        .bind(user)
        .fetch_one(pool)
        .await?;
    let session: Uuid = sqlx::query_scalar(
        "INSERT INTO platform_sessions(user_id,tenant_id,organization_id,expires_at) \
         VALUES($1,$2,$3,now()+interval '1 hour') RETURNING id",
    )
    .bind(user)
    .bind(tenant)
    .bind(org)
    .fetch_one(pool)
    .await?;
    let now = chrono::Utc::now().timestamp();
    Ok(jwt::sign(
        &jwt::Claims {
            sub: user.to_string(),
            typ: "session".into(),
            tenant: Some(tenant.to_string()),
            org: Some(org.to_string()),
            cid: None,
            scope: None,
            sid: Some(session.to_string()),
            ver: Some(version),
            aud: None,
            jti: None,
            iat: now,
            exp: now + 3600,
        },
        SECRET,
    )?)
}

async fn get(app: &Router, token: &str, path: &str, expected: StatusCode) -> Result<Value> {
    let request = Request::builder()
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())?;
    let response = tokio::time::timeout(Duration::from_secs(30), app.clone().oneshot(request))
        .await
        .map_err(|_| anyhow!("department organization request timed out"))??;
    let status = response.status();
    let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
    ensure!(
        status == expected,
        "{path} returned {status}, expected {expected}: {body}"
    );
    Ok(body)
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn department_organization_choices() -> Result<()> {
    let database = Database::create().await?;
    let pool = &database.pool;
    let result = async {
        let tenant: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(name,code) VALUES('部门组织测试','department-tenant') RETURNING id",
        ).fetch_one(pool).await?;
        let other_tenant: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(name,code) VALUES('其他租户','other-tenant') RETURNING id",
        ).fetch_one(pool).await?;
        let root = organization(pool, tenant, None, "ancestor").await?;
        let current = organization(pool, tenant, Some(root), "current").await?;
        let child = organization(pool, tenant, Some(current), "child").await?;
        let leaf = organization(pool, tenant, Some(child), "leaf").await?;
        let sibling = organization(pool, tenant, Some(root), "sibling").await?;
        let foreign = organization(pool, other_tenant, None, "foreign").await?;
        let manager = user(pool, "department-only-manager", current, false).await?;
        let denied = user(pool, "no-department-permission", current, false).await?;
        let super_admin = user(pool, "super-admin", current, true).await?;
        sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)")
            .bind(manager).bind(leaf).execute(pool).await?;
        let app_id: Uuid = sqlx::query_scalar(
            "INSERT INTO applications(name,code) VALUES('系统管理','system') RETURNING id",
        ).fetch_one(pool).await?;
        let menu: Uuid = sqlx::query_scalar(
            "INSERT INTO menus(application_id,title,permission,type) \
             VALUES($1,'部门管理','system:department','BUTTON') RETURNING id",
        ).bind(app_id).fetch_one(pool).await?;
        let role: Uuid = sqlx::query_scalar(
            "INSERT INTO roles(tenant_id,organization_id,name) VALUES($1,$2,'部门管理员') RETURNING id",
        ).bind(tenant).bind(current).fetch_one(pool).await?;
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
            .bind(tenant).bind(app_id).execute(pool).await?;
        sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
            .bind(tenant).bind(menu).execute(pool).await?;
        for org in [current, leaf] {
            sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)")
                .bind(org).bind(app_id).execute(pool).await?;
            sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
                .bind(org).bind(menu).execute(pool).await?;
        }
        sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)")
            .bind(role).bind(menu).execute(pool).await?;
        sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3)")
            .bind(manager).bind(role).bind(current).execute(pool).await?;
        let config = Config::from_values(&HashMap::from([
            ("APPLICATION_ENVIRONMENT".into(), "development".into()),
            ("DATABASE_URL".into(), "postgres://unused/department-organization-test".into()),
            ("JWT_SECRET".into(), SECRET.into()),
        ]))?;
        let app = routes::api_router().with_state(AppState::new(pool.clone(), Arc::new(config)));
        let manager_token = session(pool, manager, tenant, current).await?;
        let denied_token = session(pool, denied, tenant, current).await?;
        let super_token = session(pool, super_admin, tenant, current).await?;

        get(&app, "", "/api/departments/organizations", StatusCode::UNAUTHORIZED).await?;
        get(&app, &denied_token, "/api/departments/organizations", StatusCode::FORBIDDEN).await?;
        get(&app, &manager_token, &format!("/api/organizations/tree?tenantId={tenant}"), StatusCode::FORBIDDEN).await?;
        let choices = get(&app, &manager_token, "/api/departments/organizations", StatusCode::OK).await?;
        ensure!(choices.as_array().is_some_and(|nodes| nodes.len() == 1), "expected current organization as the only root");
        ensure!(choices[0]["id"] == json!(current) && choices[0]["parentId"].is_null(), "nested current organization was not rooted");
        ensure!(choices[0]["name"] == "current" && choices[0]["code"] == "current"
            && choices[0]["status"] == "ENABLED" && choices[0]["tenantId"] == json!(tenant), "organization option fields differ");
        ensure!(choices[0]["children"][0]["id"] == json!(child)
            && choices[0]["children"][0]["parentId"] == json!(current)
            && choices[0]["children"][0]["children"][0]["id"] == json!(leaf), "descendant hierarchy was not retained");
        let encoded = choices.to_string();
        for excluded in [root, sibling, foreign, other_tenant] {
            ensure!(!encoded.contains(&excluded.to_string()), "organization selection leaked excluded scope {excluded}");
        }
        ensure!(choices[0].get("userCount").is_none() && choices[0].get("address").is_none(), "selector exposed organization management details");
        let super_choices = get(&app, &super_token,
            &format!("/api/departments/organizations?tenantId={other_tenant}&organizationId={foreign}"), StatusCode::OK).await?;
        ensure!(super_choices == choices, "super administrator or client parameters expanded the selector scope");
        sqlx::query("UPDATE tenants SET is_platform=true WHERE id=$1")
            .bind(tenant).execute(pool).await?;
        let platform_choices = get(&app, &manager_token, "/api/departments/organizations", StatusCode::OK).await?;
        ensure!(platform_choices == choices, "platform context expanded the selector scope");
        let leaf_token = session(pool, manager, tenant, leaf).await?;
        let leaf_choices = get(&app, &leaf_token, "/api/departments/organizations", StatusCode::OK).await?;
        ensure!(leaf_choices.as_array().is_some_and(|nodes| nodes.len() == 1)
            && leaf_choices[0]["id"] == json!(leaf)
            && leaf_choices[0]["parentId"].is_null()
            && leaf_choices[0]["children"] == json!([]), "leaf context did not return its own selectable root");
        sqlx::query("DELETE FROM role_menus WHERE role_id=$1")
            .bind(role).execute(pool).await?;
        get(&app, &manager_token, "/api/departments/organizations", StatusCode::FORBIDDEN).await?;
        Ok(())
    }.await;
    database.finish(result).await
}
