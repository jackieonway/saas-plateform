//! Selected tenant/organization authorization isolation on an owned, migrated database.
//! Run with `cargo test authorization_isolation -- --ignored --nocapture`.
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
use crate::state::AppState;
use crate::util::jwt;

const SECRET: &str = "authorization-isolation-tests-only";

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
        let name = format!("authorization_isolation_test_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await
            .map_err(|_| {
                anyhow!("creating isolated authorization isolation test database failed")
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
                    "isolated authorization isolation test database connection failed"
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
                    "isolated authorization isolation test migrations failed"
                )))
                .await?;
            return Err(anyhow!(
                "isolated authorization isolation test migrations failed"
            ));
        }
        Ok(database)
    }

    async fn finish(self, result: Result<()>) -> Result<()> {
        ensure!(
            self.name.starts_with("authorization_isolation_test_")
                && self
                    .name
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_'),
            "refusing cleanup outside the owned authorization isolation test database"
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
            "authorization isolations cleanup assertions_ok={} database_removed={} pool_closed={}",
            result.is_ok(),
            removed,
            finally_closed
        );
        result?;
        ensure!(
            removed && finally_closed,
            "isolated authorization isolation test cleanup failed"
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
        .map_err(|_| anyhow!("authorization isolation request timed out"))??;
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

async fn grant_menu(pool: &PgPool, tenant: Uuid, org: Uuid, menu: Uuid) -> Result<()> {
    sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(tenant)
        .bind(menu)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(org).bind(menu).execute(pool).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn authorization_isolation_respects_selected_tenant_and_revocation() -> Result<()> {
    let database = Database::create().await?;
    let pool = &database.pool;
    let result = async {
        let platform: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(name,code,is_platform) VALUES('Isolation platform','admin-tenant',true) RETURNING id",
        ).fetch_one(pool).await?;
        let tenant: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(name,code) VALUES('Isolation tenant','isolation-a') RETURNING id",
        ).fetch_one(pool).await?;
        let foreign_tenant: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(name,code) VALUES('Isolation foreign','isolation-b') RETURNING id",
        ).fetch_one(pool).await?;
        let platform_org = organization(pool, platform, None, "Isolation platform org").await?;
        let org = organization(pool, tenant, None, "Isolation org").await?;
        let foreign_org = organization(pool, foreign_tenant, None, "Isolation foreign org").await?;
        let admin = user(pool, "isolation-global-admin", platform_org, true).await?;
        sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)")
            .bind(admin).bind(org).execute(pool).await?;
        let scoped = user(pool, "isolation-scoped-admin", org, false).await?;
        let foreign_user = user(pool, "isolation-foreign-user", foreign_org, false).await?;
        let app_id: Uuid = sqlx::query_scalar(
            "INSERT INTO applications(name,code) VALUES('Isolation system','system') RETURNING id",
        ).fetch_one(pool).await?;
        let denied_app: Uuid = sqlx::query_scalar(
            "INSERT INTO applications(name,code) VALUES('Isolation denied app','isolation-denied') RETURNING id",
        ).fetch_one(pool).await?;
        let mut menus = HashMap::new();
        for permission in ["system:tenant", "system:organization", "system:user", "system:role", "system:application", "system:menu", "notification:publish"] {
            let menu: Uuid = sqlx::query_scalar(
                "INSERT INTO menus(application_id,title,permission,type) VALUES($1,$2,$2,'MENU') RETURNING id",
            ).bind(app_id).bind(permission).fetch_one(pool).await?;
            menus.insert(permission, menu);
        }
        let denied_menu: Uuid = sqlx::query_scalar(
            "INSERT INTO menus(application_id,title,permission,type) VALUES($1,'Denied data','denied:data','MENU') RETURNING id",
        ).bind(denied_app).fetch_one(pool).await?;
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
            .bind(tenant).bind(app_id).execute(pool).await?;
        sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)")
            .bind(org).bind(app_id).execute(pool).await?;
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
            .bind(platform).bind(app_id).execute(pool).await?;
        sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)")
            .bind(platform_org).bind(app_id).execute(pool).await?;
        for menu in menus.values() {
            grant_menu(pool, platform, platform_org, *menu).await?;
        }
        for permission in ["system:organization", "system:user", "system:role"] {
            grant_menu(pool, tenant, org, menus[permission]).await?;
        }
        // Menu grants alone cannot bypass an absent application grant.
        grant_menu(pool, tenant, org, denied_menu).await?;
        let role: Uuid = sqlx::query_scalar(
            "INSERT INTO roles(tenant_id,organization_id,name,code) VALUES($1,$2,'超级管理员','tenant_admin') RETURNING id",
        ).bind(tenant).bind(org).fetch_one(pool).await?;
        sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3)")
            .bind(scoped).bind(role).bind(org).execute(pool).await?;
        sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2),($1,$3)")
            .bind(role).bind(menus["system:user"]).bind(menus["system:tenant"]).execute(pool).await?;
        let config = Config::from_values(&HashMap::from([
            ("APPLICATION_ENVIRONMENT".into(), "development".into()),
            ("DATABASE_URL".into(), "postgres://unused/authorization-isolation-test".into()),
            ("JWT_SECRET".into(), SECRET.into()),
        ]))?;
        let app = routes::api_router().with_state(AppState::new(pool.clone(), Arc::new(config)));
        let selected = session(pool, admin, tenant, org).await?;
        let platform_session = session(pool, admin, platform, platform_org).await?;
        let scoped_session = session(pool, scoped, tenant, org).await?;
        let menu_path = format!("/api/auth/menus?applicationId={app_id}");
        let permissions = call(&app, &selected, Method::GET, "/api/auth/permissions", None, StatusCode::OK).await?;
        ensure!(permissions["permissions"] == json!(["system:organization", "system:role", "system:user"]),
            "ordinary tenant global administrator exceeded current application/menu grants: {permissions}");
        let menu_result = call(&app, &selected, Method::GET, &menu_path, None, StatusCode::OK).await?;
        ensure!(menu_result["menus"].as_array().is_some_and(|items| items.len() == 3),
            "ordinary tenant menu tree was not bounded");
        let denied_menus = call(&app, &selected, Method::GET,
            &format!("/api/auth/menus?applicationId={denied_app}"), None, StatusCode::OK).await?;
        ensure!(denied_menus["menus"] == json!([]), "unauthorized app exposed menu data");
        let profile = call(&app, &selected, Method::GET, "/api/auth/profile", None, StatusCode::OK).await?;
        ensure!(profile["applications"].as_array().is_some_and(|apps| apps.len() == 1)
            && profile["applications"][0]["id"] == json!(app_id), "unauthorized app entered usable profile apps");
        for path in ["/api/tenants", "/api/applications", "/api/monitor/overview", "/api/oauth/clients"] {
            call(&app, &selected, Method::GET, path, None, StatusCode::FORBIDDEN).await?;
        }
        let choices = call(&app, &selected, Method::GET, "/api/tenants/options", None, StatusCode::OK).await?;
        ensure!(choices == json!([{"id":tenant,"name":"Isolation tenant","code":"isolation-a","isPlatform":false}]),
            "tenant choices leaked another tenant or detailed management data");
        call(&app, &selected, Method::GET, &format!("/api/menus/tree?applicationId={app_id}"), None, StatusCode::FORBIDDEN).await?;
        call(&app, &selected, Method::POST, "/api/oauth/session-token", Some(json!({"appCode":"isolation-denied"})), StatusCode::NOT_FOUND).await?;
        let users = call(&app, &selected, Method::GET, &format!("/api/users?tenantId={foreign_tenant}&size=100"), None, StatusCode::OK).await?;
        ensure!(!users.to_string().contains(&foreign_user.to_string())
            && !users.to_string().contains(&foreign_org.to_string()), "ordinary context leaked foreign user or memberships");
        call(&app, &selected, Method::GET, &format!("/api/users/{foreign_user}/memberships"), None, StatusCode::NOT_FOUND).await?;
        call(&app, &selected, Method::GET, &format!("/api/organizations/tree?tenantId={foreign_tenant}"), None, StatusCode::NOT_FOUND).await?;
        let search = call(&app, &selected, Method::GET, "/api/search?q=Isolation", None, StatusCode::OK).await?;
        ensure!(search["tenants"] == json!([]) && !search.to_string().contains(&foreign_tenant.to_string())
            && !search.to_string().contains(&foreign_org.to_string()), "search bypassed grants or current tenant scope");
        let scoped_perms = call(&app, &scoped_session, Method::GET, "/api/auth/permissions", None, StatusCode::OK).await?;
        ensure!(scoped_perms["permissions"] == json!(["system:user"]), "scoped administrator role bypassed tenant grant ceiling");
        call(&app, &scoped_session, Method::GET, "/api/tenants", None, StatusCode::FORBIDDEN).await?;
        let platform_tenants = call(&app, &platform_session, Method::GET, "/api/tenants", None, StatusCode::OK).await?;
        ensure!(platform_tenants["total"] == 3, "platform context lost legitimate cross-tenant visibility");
        let platform_choices = call(&app, &platform_session, Method::GET, "/api/tenants/options", None, StatusCode::OK).await?;
        ensure!(platform_choices.as_array().is_some_and(|choices| choices.len() == 3),
            "platform context lost legitimate tenant choices");
        call(&app, &platform_session, Method::GET, "/api/applications", None, StatusCode::OK).await?;
        let platform_perms = call(&app, &platform_session, Method::GET, "/api/auth/permissions", None, StatusCode::OK).await?;
        ensure!(platform_perms["permissions"].as_array().is_some_and(|perms| perms.len() == menus.len()
            && perms.contains(&json!("system:tenant")) && !perms.contains(&json!("denied:data"))),
            "platform super administrator did not respect its current organization grants");
        grant_menu(pool, tenant, org, menus["system:tenant"]).await?;
        let granted_tenants = call(&app, &selected, Method::GET, "/api/tenants", None, StatusCode::OK).await?;
        ensure!(granted_tenants["total"] == 1 && granted_tenants["records"][0]["id"] == json!(tenant),
            "authorized ordinary context still listed other tenants");
        call(&app, &selected, Method::GET, &format!("/api/tenants/{foreign_tenant}/authorizations"), None, StatusCode::FORBIDDEN).await?;
        sqlx::query("DELETE FROM organization_menus WHERE organization_id=$1 AND menu_id=$2")
            .bind(org).bind(menus["system:tenant"]).execute(pool).await?;
        call(&app, &selected, Method::GET, "/api/tenants", None, StatusCode::FORBIDDEN).await?;
        sqlx::query("DELETE FROM tenant_menus WHERE tenant_id=$1 AND menu_id=$2")
            .bind(tenant).bind(menus["system:user"]).execute(pool).await?;
        call(&app, &selected, Method::GET, "/api/users", None, StatusCode::FORBIDDEN).await?;
        call(&app, &scoped_session, Method::GET, "/api/users", None, StatusCode::FORBIDDEN).await?;

        // App-context permissions must observe the same cap when the user has a global admin flag.
        sqlx::query("INSERT INTO oauth_clients(client_id,client_name,client_type,grant_types,scopes,application_id) \
                     VALUES('isolation-client','Isolation client','CONFIDENTIAL','client_credentials','profile',$1)")
            .bind(app_id).execute(pool).await?;
        let issue = call(&app, &selected, Method::POST, "/api/oauth/session-token",
            Some(json!({"appCode":"system"})), StatusCode::CREATED).await?;
        let app_token = issue["accessToken"].as_str().ok_or_else(|| anyhow!("missing app token"))?;
        let context = call(&app, app_token, Method::GET, "/api/apps/system/context", None, StatusCode::OK).await?;
        ensure!(context["permissions"] == json!(["system:organization", "system:role"]),
            "global administrator app context exceeded two-level menu grants");
        sqlx::query("INSERT INTO membership_data_grants(membership_id,application_id,resource_code,action_code,scope_type) \
                     SELECT id,$3,'notification','publish','SELF' FROM memberships WHERE user_id=$1 AND organization_id=$2")
            .bind(admin).bind(org).bind(app_id).execute(pool).await?;
        let publish = json!({"idempotencyKey":"isolation-ungranted-publish","target":{"kind":"USER","userId":admin,"organizationId":org},
            "type":"INFO","title":"Isolation publication","content":"","link":""});
        call(&app, app_token, Method::POST, "/api/apps/system/notifications", Some(publish.clone()), StatusCode::FORBIDDEN).await?;
        grant_menu(pool, tenant, org, menus["notification:publish"]).await?;
        call(&app, app_token, Method::POST, "/api/apps/system/notifications", Some(publish.clone()), StatusCode::OK).await?;
        sqlx::query("DELETE FROM tenant_menus WHERE tenant_id=$1 AND menu_id=$2")
            .bind(tenant).bind(menus["notification:publish"]).execute(pool).await?;
        call(&app, app_token, Method::POST, "/api/apps/system/notifications", Some(publish), StatusCode::FORBIDDEN).await?;
        sqlx::query("DELETE FROM organization_applications WHERE organization_id=$1 AND application_id=$2")
            .bind(org).bind(app_id).execute(pool).await?;
        let revoked = call(&app, &selected, Method::GET, "/api/auth/permissions", None, StatusCode::OK).await?;
        ensure!(revoked["permissions"] == json!([]), "organization app revocation did not cap the next request");
        call(&app, &selected, Method::GET, "/api/tenants/options", None, StatusCode::FORBIDDEN).await?;
        let revoked_menus = call(&app, &selected, Method::GET, &menu_path, None, StatusCode::OK).await?;
        ensure!(revoked_menus["menus"] == json!([]), "organization app revocation left menu data visible");
        call(&app, &selected, Method::GET, &format!("/api/organizations/tree?tenantId={tenant}"), None, StatusCode::FORBIDDEN).await?;
        call(&app, app_token, Method::GET, "/api/apps/system/context", None, StatusCode::UNAUTHORIZED).await?;
        call(&app, &selected, Method::POST, "/api/oauth/session-token", Some(json!({"appCode":"system"})), StatusCode::NOT_FOUND).await?;
        sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)")
            .bind(org).bind(app_id).execute(pool).await?;
        sqlx::query("DELETE FROM tenant_applications WHERE tenant_id=$1 AND application_id=$2")
            .bind(tenant).bind(app_id).execute(pool).await?;
        let revoked = call(&app, &selected, Method::GET, "/api/auth/permissions", None, StatusCode::OK).await?;
        ensure!(revoked["permissions"] == json!([]), "tenant app revocation did not cap the next request");
        call(&app, &selected, Method::GET, "/api/roles", None, StatusCode::FORBIDDEN).await?;
        Ok(())
    }.await;
    database.finish(result).await
}

async fn assert_no_management_data(app: &Router, token: &str, app_id: Uuid) -> Result<()> {
    let permissions = call(
        app,
        token,
        Method::GET,
        "/api/auth/permissions",
        None,
        StatusCode::OK,
    )
    .await?;
    ensure!(
        permissions["permissions"] == json!([]),
        "ungranted organization exposed permissions"
    );
    let menus = call(
        app,
        token,
        Method::GET,
        &format!("/api/auth/menus?applicationId={app_id}"),
        None,
        StatusCode::OK,
    )
    .await?;
    ensure!(
        menus["menus"] == json!([]),
        "ungranted organization exposed menus"
    );
    for path in [
        "/api/users",
        "/api/tenants",
        "/api/tenants/options",
        "/api/applications",
    ] {
        call(app, token, Method::GET, path, None, StatusCode::FORBIDDEN).await?;
    }
    let search = call(
        app,
        token,
        Method::GET,
        "/api/search?q=Organization",
        None,
        StatusCode::OK,
    )
    .await?;
    ensure!(
        search == json!({"users":[],"tenants":[],"organizations":[],"roles":[]}),
        "ungranted organization search exposed management data: {search}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn authorization_isolation_platform_root_and_child_require_direct_org_grants() -> Result<()> {
    let database = Database::create().await?;
    let pool = &database.pool;
    let result = async {
        let tenant: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(name,code,is_platform) VALUES('Organization platform','admin-tenant',true) RETURNING id",
        ).fetch_one(pool).await?;
        let root = organization(pool, tenant, None, "Organization root").await?;
        let child = organization(pool, tenant, Some(root), "Organization child").await?;
        let admin = user(pool, "organization-global-admin", root, true).await?;
        sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)")
            .bind(admin).bind(child).execute(pool).await?;
        let application: Uuid = sqlx::query_scalar(
            "INSERT INTO applications(name,code) VALUES('Organization system','system') RETURNING id",
        ).fetch_one(pool).await?;
        let mut menus = HashMap::new();
        for permission in ["system:tenant", "system:user", "notification:publish"] {
            let menu: Uuid = sqlx::query_scalar(
                "INSERT INTO menus(application_id,title,permission,type) VALUES($1,$2,$2,'MENU') RETURNING id",
            ).bind(application).bind(permission).fetch_one(pool).await?;
            menus.insert(permission, menu);
        }
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
            .bind(tenant).bind(application).execute(pool).await?;
        for org in [root, child] {
            for menu in menus.values() {
                grant_menu(pool, tenant, org, *menu).await?;
            }
            sqlx::query("INSERT INTO membership_data_grants(membership_id,application_id,resource_code,action_code,scope_type) \
                         SELECT id,$3,'notification','publish','SELF' FROM memberships WHERE user_id=$1 AND organization_id=$2")
                .bind(admin).bind(org).bind(application).execute(pool).await?;
        }
        sqlx::query("INSERT INTO oauth_clients(client_id,client_name,client_type,grant_types,scopes,application_id) \
                     VALUES('organization-client','Organization client','CONFIDENTIAL','client_credentials','profile',$1)")
            .bind(application).execute(pool).await?;
        let config = Config::from_values(&HashMap::from([
            ("APPLICATION_ENVIRONMENT".into(), "development".into()),
            ("DATABASE_URL".into(), "postgres://unused/organization-isolation-test".into()),
            ("JWT_SECRET".into(), SECRET.into()),
        ]))?;
        let app = routes::api_router().with_state(AppState::new(pool.clone(), Arc::new(config)));
        let root_session = session(pool, admin, tenant, root).await?;
        let child_session = session(pool, admin, tenant, child).await?;
        let contexts = [(root, root_session), (child, child_session)];

        // Tenant application/menu and organization menu grants are insufficient
        // without the current organization's own application grant, even for a
        // global administrator in the actual platform tenant.
        for (_, token) in &contexts {
            assert_no_management_data(&app, token, application).await?;
            let profile = call(&app, token, Method::GET, "/api/auth/profile", None, StatusCode::OK).await?;
            ensure!(profile["applications"] == json!([]), "organization without app grant exposed an app");
            call(&app, token, Method::POST, "/api/oauth/session-token",
                Some(json!({"appCode":"system"})), StatusCode::NOT_FOUND).await?;
        }

        // An application grant alone does not confer any menu or operation.
        let mut app_tokens = HashMap::new();
        for (org, token) in &contexts {
            sqlx::query("DELETE FROM organization_menus WHERE organization_id=$1")
                .bind(org).execute(pool).await?;
            sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)")
                .bind(org).bind(application).execute(pool).await?;
            assert_no_management_data(&app, token, application).await?;
            let issue = call(&app, token, Method::POST, "/api/oauth/session-token",
                Some(json!({"appCode":"system"})), StatusCode::CREATED).await?;
            let app_token = issue["accessToken"].as_str().ok_or_else(|| anyhow!("missing app token"))?.to_owned();
            let context = call(&app, &app_token, Method::GET, "/api/apps/system/context", None, StatusCode::OK).await?;
            ensure!(context["permissions"] == json!([]) && context["menus"] == json!([]),
                "platform app context bypassed current organization menu grants");
            let publish = json!({"idempotencyKey":format!("organization-ungranted-{org}"),
                "target":{"kind":"USER","userId":admin,"organizationId":org},
                "type":"INFO","title":"Organization publication","content":"","link":""});
            call(&app, &app_token, Method::POST, "/api/apps/system/notifications",
                Some(publish), StatusCode::FORBIDDEN).await?;
            app_tokens.insert(*org, app_token);
        }

        // Parent grants must not grant child menus implicitly. Grant only the
        // root user menu, confirm the child is denied, then grant the child.
        grant_menu(pool, tenant, root, menus["system:user"]).await?;
        call(&app, &contexts[0].1, Method::GET, "/api/users", None, StatusCode::OK).await?;
        assert_no_management_data(&app, &contexts[1].1, application).await?;
        grant_menu(pool, tenant, child, menus["system:user"]).await?;
        for (org, token) in &contexts {
            call(&app, token, Method::GET, "/api/users", None, StatusCode::OK).await?;
            call(&app, token, Method::GET, "/api/tenants", None, StatusCode::FORBIDDEN).await?;
            let app_token = &app_tokens[org];
            let context = call(&app, app_token, Method::GET, "/api/apps/system/context", None, StatusCode::OK).await?;
            ensure!(context["permissions"] == json!(["system:user"]),
                "app context did not restrict global administrator to granted menu");
            grant_menu(pool, tenant, *org, menus["notification:publish"]).await?;
            let publish = json!({"idempotencyKey":format!("organization-granted-{org}"),
                "target":{"kind":"USER","userId":admin,"organizationId":org},
                "type":"INFO","title":"Organization publication","content":"","link":""});
            call(&app, app_token, Method::POST, "/api/apps/system/notifications",
                Some(publish.clone()), StatusCode::OK).await?;
            sqlx::query("DELETE FROM organization_menus WHERE organization_id=$1 AND menu_id=$2")
                .bind(org).bind(menus["notification:publish"]).execute(pool).await?;
            call(&app, app_token, Method::POST, "/api/apps/system/notifications",
                Some(publish), StatusCode::FORBIDDEN).await?;
            sqlx::query("DELETE FROM organization_applications WHERE organization_id=$1 AND application_id=$2")
                .bind(org).bind(application).execute(pool).await?;
            assert_no_management_data(&app, token, application).await?;
            call(&app, app_token, Method::GET, "/api/apps/system/context", None, StatusCode::UNAUTHORIZED).await?;
            call(&app, token, Method::POST, "/api/oauth/session-token",
                Some(json!({"appCode":"system"})), StatusCode::NOT_FOUND).await?;
        }
        Ok(())
    }.await;
    database.finish(result).await
}

async fn assert_oauth_clients_denied(app: &Router, token: &str) -> Result<()> {
    let client = Uuid::new_v4();
    for (method, path, body) in [
        (Method::GET, "/api/oauth/clients".to_owned(), None),
        (
            Method::POST,
            "/api/oauth/clients".to_owned(),
            Some(json!({
                "clientName":"Denied client","grantTypes":["client_credentials"],"scopes":["profile"],
            })),
        ),
        (
            Method::PUT,
            format!("/api/oauth/clients/{client}"),
            Some(json!({"clientName":"Denied update"})),
        ),
        (Method::DELETE, format!("/api/oauth/clients/{client}"), None),
        (
            Method::POST,
            format!("/api/oauth/clients/{client}/regenerate-secret"),
            None,
        ),
    ] {
        call(app, token, method, &path, body, StatusCode::FORBIDDEN).await?;
    }
    Ok(())
}

async fn assert_oauth_tokens_denied(app: &Router, token: &str, target: Uuid) -> Result<()> {
    let missing = Uuid::new_v4();
    for (method, path, body) in [
        (Method::GET, "/api/oauth/tokens".to_owned(), None),
        (Method::GET, "/api/oauth/tokens/sessions".to_owned(), None),
        (Method::GET, "/api/oauth/device-codes".to_owned(), None),
        (Method::DELETE, format!("/api/oauth/tokens/{missing}"), None),
        (Method::POST, "/api/oauth/tokens/purge".to_owned(), None),
        (
            Method::POST,
            "/api/oauth/tokens/kick".to_owned(),
            Some(json!({"userId":target})),
        ),
        (
            Method::PUT,
            format!("/api/oauth/device-codes/{missing}/revoke"),
            None,
        ),
    ] {
        call(app, token, method, &path, body, StatusCode::FORBIDDEN).await?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn authorization_isolation_shared_oauth_code_requires_the_specific_menu() -> Result<()> {
    let database = Database::create().await?;
    let pool = &database.pool;
    let result = async {
        let tenant: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(name,code,is_platform) VALUES('Shared code platform','admin-tenant',true) RETURNING id",
        ).fetch_one(pool).await?;
        let org = organization(pool, tenant, None, "Shared code root").await?;
        let admin = user(pool, "shared-code-global-admin", org, true).await?;
        let scoped = user(pool, "shared-code-scoped-admin", org, false).await?;
        let target = user(pool, "shared-code-session-target", org, false).await?;
        let application: Uuid = sqlx::query_scalar(
            "INSERT INTO applications(name,code) VALUES('Shared code system','system') RETURNING id",
        ).fetch_one(pool).await?;
        let client_menu: Uuid = sqlx::query_scalar(
            "INSERT INTO menus(application_id,title,permission,type,path) \
             VALUES($1,'OAuth clients','system:oauth','MENU','/system/oauth/clients') RETURNING id",
        ).bind(application).fetch_one(pool).await?;
        let token_menu: Uuid = sqlx::query_scalar(
            "INSERT INTO menus(application_id,title,permission,type,path) \
             VALUES($1,'OAuth devices and sessions','system:oauth','MENU','/system/oauth/tokens') RETURNING id",
        ).bind(application).fetch_one(pool).await?;
        let misleading_button: Uuid = sqlx::query_scalar(
            "INSERT INTO menus(application_id,title,permission,type,path) \
             VALUES($1,'Shared code operation','system:oauth','BUTTON','/system/oauth/tokens') RETURNING id",
        ).bind(application).fetch_one(pool).await?;
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
            .bind(tenant).bind(application).execute(pool).await?;
        sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)")
            .bind(org).bind(application).execute(pool).await?;
        for menu in [client_menu, token_menu, misleading_button] {
            sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
                .bind(tenant).bind(menu).execute(pool).await?;
        }
        for menu in [client_menu, misleading_button] {
            sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
                .bind(org).bind(menu).execute(pool).await?;
        }
        let role: Uuid = sqlx::query_scalar(
            "INSERT INTO roles(tenant_id,organization_id,name,code) VALUES($1,$2,'OAuth operator','oauth-operator') RETURNING id",
        ).bind(tenant).bind(org).fetch_one(pool).await?;
        sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3)")
            .bind(scoped).bind(role).bind(org).execute(pool).await?;
        sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2),($1,$3)")
            .bind(role).bind(client_menu).bind(misleading_button).execute(pool).await?;
        let config = Config::from_values(&HashMap::from([
            ("APPLICATION_ENVIRONMENT".into(), "development".into()),
            ("DATABASE_URL".into(), "postgres://unused/shared-code-isolation-test".into()),
            ("JWT_SECRET".into(), SECRET.into()),
        ]))?;
        let app = routes::api_router().with_state(AppState::new(pool.clone(), Arc::new(config)));
        let admin_session = session(pool, admin, tenant, org).await?;
        let scoped_session = session(pool, scoped, tenant, org).await?;
        let target_session = session(pool, target, tenant, org).await?;

        // A granted OAuth client menu and even a BUTTON with the token page's
        // path cannot unlock device/session data sharing the same permission code.
        for token in [&admin_session, &scoped_session] {
            let permissions = call(&app, token, Method::GET, "/api/auth/permissions", None, StatusCode::OK).await?;
            ensure!(permissions["permissions"] == json!(["system:oauth"]), "fixture lacks the shared permission code");
            call(&app, token, Method::GET, "/api/oauth/clients", None, StatusCode::OK).await?;
            assert_oauth_tokens_denied(&app, token, target).await?;
        }
        call(&app, &target_session, Method::GET, "/api/auth/profile", None, StatusCode::OK).await?;
        let created = call(&app, &scoped_session, Method::POST, "/api/oauth/clients", Some(json!({
            "clientName":"Granted client","grantTypes":["client_credentials"],"scopes":["profile"],
        })), StatusCode::OK).await?;
        let client_id = created["id"].as_str().ok_or_else(|| anyhow!("missing client id"))?;
        call(&app, &scoped_session, Method::PUT, &format!("/api/oauth/clients/{client_id}"),
            Some(json!({"clientName":"Updated client"})), StatusCode::OK).await?;
        call(&app, &scoped_session, Method::POST, &format!("/api/oauth/clients/{client_id}/regenerate-secret"),
            None, StatusCode::OK).await?;
        call(&app, &scoped_session, Method::DELETE, &format!("/api/oauth/clients/{client_id}"),
            None, StatusCode::OK).await?;

        // Global administrators bypass only the user-role layer. Normal users
        // still need the specific token menu in their effective role grants.
        grant_menu(pool, tenant, org, token_menu).await?;
        call(&app, &admin_session, Method::GET, "/api/oauth/tokens/sessions", None, StatusCode::OK).await?;
        assert_oauth_tokens_denied(&app, &scoped_session, target).await?;
        sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)")
            .bind(role).bind(token_menu).execute(pool).await?;
        for token in [&admin_session, &scoped_session] {
            for path in ["/api/oauth/tokens", "/api/oauth/tokens/sessions", "/api/oauth/device-codes"] {
                call(&app, token, Method::GET, path, None, StatusCode::OK).await?;
            }
        }
        sqlx::query("DELETE FROM organization_menus WHERE organization_id=$1 AND menu_id=$2")
            .bind(org).bind(client_menu).execute(pool).await?;
        for token in [&admin_session, &scoped_session] {
            assert_oauth_clients_denied(&app, token).await?;
            call(&app, token, Method::GET, "/api/oauth/tokens/sessions", None, StatusCode::OK).await?;
        }
        call(&app, &scoped_session, Method::POST, "/api/oauth/tokens/purge", None, StatusCode::OK).await?;
        let missing = Uuid::new_v4();
        call(&app, &scoped_session, Method::DELETE, &format!("/api/oauth/tokens/{missing}"), None, StatusCode::NOT_FOUND).await?;
        call(&app, &scoped_session, Method::PUT, &format!("/api/oauth/device-codes/{missing}/revoke"), None, StatusCode::NOT_FOUND).await?;
        call(&app, &scoped_session, Method::POST, "/api/oauth/tokens/kick", Some(json!({"userId":target})), StatusCode::OK).await?;
        call(&app, &target_session, Method::GET, "/api/auth/profile", None, StatusCode::UNAUTHORIZED).await?;

        // Retaining a shared-code button does not preserve access after the
        // tenant menu cap or the current organization's app cap is revoked.
        sqlx::query("DELETE FROM tenant_menus WHERE tenant_id=$1 AND menu_id=$2")
            .bind(tenant).bind(token_menu).execute(pool).await?;
        for token in [&admin_session, &scoped_session] {
            assert_oauth_tokens_denied(&app, token, target).await?;
        }
        grant_menu(pool, tenant, org, token_menu).await?;
        sqlx::query("DELETE FROM organization_applications WHERE organization_id=$1 AND application_id=$2")
            .bind(org).bind(application).execute(pool).await?;
        for token in [&admin_session, &scoped_session] {
            assert_oauth_clients_denied(&app, token).await?;
            assert_oauth_tokens_denied(&app, token, target).await?;
        }
        Ok(())
    }.await;
    database.finish(result).await
}
