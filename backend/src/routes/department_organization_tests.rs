//! Department organization choices on an owned, migrated PostgreSQL database.
//! Run with `cargo test department_organization_choices -- --ignored --nocapture`.
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

const SECRET: &str = "department-organizations-isolated-tests-only";

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
    jwt::sign(
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
    )
}

async fn get(app: &Router, token: &str, path: &str, expected: StatusCode) -> Result<Value> {
    call(app, token, Method::GET, path, None, expected).await
}

async fn call(
    app: &Router,
    token: &str,
    method: Method,
    path: &str,
    payload: Option<Value>,
    expected: StatusCode,
) -> Result<Value> {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(payload.map_or_else(Body::empty, |value| Body::from(value.to_string())))?;
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
        let org_menu: Uuid = sqlx::query_scalar(
            "INSERT INTO menus(application_id,title,permission,type) \
             VALUES($1,'组织管理','system:organization','BUTTON') RETURNING id",
        ).bind(app_id).fetch_one(pool).await?;
        let role: Uuid = sqlx::query_scalar(
            "INSERT INTO roles(tenant_id,organization_id,name) VALUES($1,$2,'部门管理员') RETURNING id",
        ).bind(tenant).bind(current).fetch_one(pool).await?;
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
            .bind(tenant).bind(app_id).execute(pool).await?;
        sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
            .bind(tenant).bind(menu).execute(pool).await?;
        sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
            .bind(tenant).bind(org_menu).execute(pool).await?;
        for org in [current, leaf] {
            sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)")
                .bind(org).bind(app_id).execute(pool).await?;
            sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
                .bind(org).bind(menu).execute(pool).await?;
            sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
                .bind(org).bind(org_menu).execute(pool).await?;
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
        let state = AppState::new(pool.clone(), Arc::new(config));
        let app = routes::api_router().with_state(state.clone());
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
        let current_user = crate::extractors::CurrentUser {
            user_id: super_admin, username: "super-admin".into(), real_name: String::new(),
            is_super_admin: true, tenant_id: tenant, org_id: current, session_id: Uuid::nil(),
        };
        let scoped_ids = state.scoped_org_ids(&current_user).await?.ok_or_else(|| anyhow!("ordinary context was unbounded"))?;
        ensure!(scoped_ids.len()==3 && [current,child,leaf].iter().all(|org|scoped_ids.contains(org)), "organization helper included ancestors or siblings");
        let organization_tree = get(&app, &super_token, &format!("/api/organizations/tree?tenantId={tenant}"), StatusCode::OK).await?;
        ensure!(organization_tree[0]["id"]==json!(current) && organization_tree[0]["parentId"].is_null()
            && organization_tree[0]["children"][0]["children"][0]["id"]==json!(leaf), "organization tree did not root the selected subtree");
        for excluded in [root,sibling,foreign] {
            ensure!(!organization_tree.to_string().contains(&excluded.to_string()), "organization tree leaked {excluded}");
            get(&app, &super_token, &format!("/api/departments/tree?organizationId={excluded}"), StatusCode::NOT_FOUND).await?;
            call(&app, &super_token, Method::PUT, &format!("/api/organizations/{excluded}"), Some(json!({"name":"forbidden"})), StatusCode::NOT_FOUND).await?;
            call(&app, &super_token, Method::DELETE, &format!("/api/organizations/{excluded}"), None, StatusCode::NOT_FOUND).await?;
            call(&app, &super_token, Method::POST, "/api/organizations", Some(json!({"tenantId":tenant,"parentId":excluded,"name":"forbidden child"})), StatusCode::NOT_FOUND).await?;
            call(&app, &super_token, Method::POST, "/api/departments", Some(json!({"organizationId":excluded,"name":"forbidden department"})), StatusCode::NOT_FOUND).await?;
            let department: Uuid = sqlx::query_scalar("INSERT INTO departments(organization_id,name) VALUES($1,'excluded department') RETURNING id")
                .bind(excluded).fetch_one(pool).await?;
            call(&app, &super_token, Method::PUT, &format!("/api/departments/{department}"), Some(json!({"name":"forbidden"})), StatusCode::NOT_FOUND).await?;
            call(&app, &super_token, Method::DELETE, &format!("/api/departments/{department}"), None, StatusCode::NOT_FOUND).await?;
        }
        let allowed_department = call(&app, &super_token, Method::POST, "/api/departments", Some(json!({"organizationId":leaf,"name":"leaf department"})), StatusCode::OK).await?;
        let department = allowed_department["id"].as_str().ok_or_else(||anyhow!("department ID missing"))?;
        get(&app, &super_token, &format!("/api/departments/tree?organizationId={leaf}"), StatusCode::OK).await?;
        call(&app, &super_token, Method::PUT, &format!("/api/departments/{department}"), Some(json!({"name":"updated leaf department"})), StatusCode::OK).await?;
        call(&app, &super_token, Method::DELETE, &format!("/api/departments/{department}"), None, StatusCode::OK).await?;
        call(&app, &super_token, Method::PUT, &format!("/api/organizations/{leaf}"), Some(json!({"description":"in-scope leaf"})), StatusCode::OK).await?;
        get(&app, &super_token, &format!("/api/organizations/tree?tenantId={other_tenant}"), StatusCode::NOT_FOUND).await?;
        sqlx::query("UPDATE tenants SET is_platform=true WHERE id=$1")
            .bind(tenant).execute(pool).await?;
        ensure!(!state.is_platform_context(&current_user).await?, "platform child inherited global resource scope");
        let child_profile = get(&app, &super_token, "/api/auth/profile", StatusCode::OK).await?;
        ensure!(child_profile["tenant"]["isPlatform"]==true && child_profile["platformContext"]==false,
            "platform child profile advertised global management context");
        for excluded in [root,sibling,foreign] {
            ensure!(state.ensure_org_scope(&current_user, excluded, "组织").await.is_err(), "platform child helper allowed {excluded}");
            get(&app, &super_token, &format!("/api/departments/tree?organizationId={excluded}"), StatusCode::NOT_FOUND).await?;
        }
        let root_user = crate::extractors::CurrentUser { org_id: root, ..current_user.clone() };
        ensure!(state.is_platform_context(&root_user).await? && state.scoped_org_ids(&root_user).await?.is_none(), "actual platform root lost global management scope");
        state.ensure_org_scope(&root_user, foreign, "组织").await?;
        sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)")
            .bind(super_admin).bind(root).execute(pool).await?;
        sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)")
            .bind(root).bind(app_id).execute(pool).await?;
        for granted in [menu,org_menu] {
            sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
                .bind(root).bind(granted).execute(pool).await?;
        }
        let root_token = session(pool, super_admin, tenant, root).await?;
        let root_profile = get(&app, &root_token, "/api/auth/profile", StatusCode::OK).await?;
        ensure!(root_profile["tenant"]["isPlatform"]==true && root_profile["platformContext"]==true,
            "platform root profile omitted global management context");
        get(&app, &root_token, &format!("/api/organizations/tree?tenantId={other_tenant}"), StatusCode::OK).await?;
        get(&app, &root_token, &format!("/api/departments/tree?organizationId={foreign}"), StatusCode::OK).await?;
        sqlx::query("INSERT INTO oauth_clients(client_id,client_name,client_type,application_id,scopes) VALUES('subtree-context-client','Subtree context client','CONFIDENTIAL',$1,'profile')")
            .bind(app_id).execute(pool).await?;
        for organization_id in [root,current] {
            sqlx::query("INSERT INTO membership_data_grants(membership_id,application_id,resource_code,action_code,scope_type) \
                         SELECT id,$3,'records','read','TENANT' FROM memberships WHERE user_id=$1 AND organization_id=$2")
                .bind(super_admin).bind(organization_id).bind(app_id).execute(pool).await?;
        }
        let issued_child = call(&app, &super_token, Method::POST, "/api/oauth/session-token", Some(json!({"appCode":"system"})), StatusCode::CREATED).await?;
        let issued_root = call(&app, &root_token, Method::POST, "/api/oauth/session-token", Some(json!({"appCode":"system"})), StatusCode::CREATED).await?;
        let child_app_token = issued_child["accessToken"].as_str().ok_or_else(||anyhow!("child app token missing"))?;
        let root_app_token = issued_root["accessToken"].as_str().ok_or_else(||anyhow!("root app token missing"))?;
        let child_app_context = get(&app, child_app_token, "/api/apps/system/context", StatusCode::OK).await?;
        let child_hard_scope = child_app_context["organizationScopeIds"].as_array().ok_or_else(||anyhow!("child app hard scope missing"))?;
        ensure!(child_hard_scope.len()==3 && [current,child,leaf].iter().all(|org|child_hard_scope.contains(&json!(org))),
            "child app hard organization cap expanded outside the selected subtree");
        ensure!(child_app_context["dataGrants"][0]["scope"]=="ORG_SUBTREE"
            && child_app_context["dataGrants"][0]["organizationIds"]==child_app_context["organizationScopeIds"],
            "TENANT app grant bypassed the selected platform child's cap");
        let root_app_context = get(&app, root_app_token, "/api/apps/system/context", StatusCode::OK).await?;
        ensure!(root_app_context["organizationScopeIds"].is_null() && root_app_context["dataGrants"][0]["scope"]=="TENANT",
            "actual platform root app context lost explicit TENANT grant semantics");
        sqlx::query("UPDATE membership_data_grants SET scope_type='SELF' WHERE membership_id IN (SELECT id FROM memberships WHERE user_id=$1 AND organization_id=$2)")
            .bind(super_admin).bind(current).execute(pool).await?;
        let self_context = get(&app, child_app_token, "/api/apps/system/context", StatusCode::OK).await?;
        ensure!(self_context["dataGrants"][0]["scope"]=="SELF"
            && self_context["dataGrants"][0]["organizationIds"]==self_context["organizationScopeIds"]
            && self_context["contextVersion"]!=child_app_context["contextVersion"],
            "SELF app grant omitted the hard organization cap or stale context version remained");
        sqlx::query("UPDATE tenants SET is_platform=false WHERE id=$1").bind(tenant).execute(pool).await?;
        let ordinary_root_app_context = get(&app, root_app_token, "/api/apps/system/context", StatusCode::OK).await?;
        ensure!(ordinary_root_app_context["dataGrants"][0]["scope"]=="ORG_SUBTREE"
            && ordinary_root_app_context["organizationScopeIds"].as_array().is_some_and(|ids|ids.len()==5
                && [root,current,child,leaf,sibling].iter().all(|org|ids.contains(&json!(org))))
            && ordinary_root_app_context["contextVersion"]!=root_app_context["contextVersion"],
            "ordinary root app scope was unbounded or context version did not change");
        sqlx::query("UPDATE tenants SET is_platform=true WHERE id=$1").bind(tenant).execute(pool).await?;
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
        let switch_source = session(pool, super_admin, tenant, current).await?;
        let switched_root = call(&app, &switch_source, Method::POST, "/api/auth/context", Some(json!({"organizationId":root})), StatusCode::OK).await?;
        ensure!(switched_root["platformContext"]==true && switched_root["tenant"]["isPlatform"]==true,
            "switching to platform root did not return its management context");
        let switched_root_token = switched_root["accessToken"].as_str().ok_or_else(||anyhow!("switched root access token missing"))?;
        let switched_child = call(&app, switched_root_token, Method::POST, "/api/auth/context", Some(json!({"organizationId":current})), StatusCode::OK).await?;
        ensure!(switched_child["platformContext"]==false && switched_child["tenant"]["isPlatform"]==true,
            "switching to platform child retained root management context");
        let switched_child_token = switched_child["accessToken"].as_str().ok_or_else(||anyhow!("switched child access token missing"))?;
        let switched_child_profile = get(&app, switched_child_token, "/api/auth/profile", StatusCode::OK).await?;
        ensure!(switched_child_profile["platformContext"]==false, "profile and newly selected child context disagree");
        Ok(())
    }.await;
    database.finish(result).await
}
