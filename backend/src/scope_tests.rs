//! 隔离 PostgreSQL 上的管理 API 回归；显式运行 ignored 测试。
use std::sync::Arc;

use anyhow::{ensure, Result};
use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::config::Config;
use crate::routes;
use crate::state::AppState;
use crate::util::jwt::{self, Claims};
use crate::util::password;
use crate::util::hash;

async fn call(
    app: &Router,
    method: Method,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> Result<(StatusCode, Value)> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
    let bytes = if let Some(value) = body {
        builder = builder.header("content-type", "application/json");
        serde_json::to_vec(&value)?
    } else {
        Vec::new()
    };
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(bytes))?)
        .await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    let value =
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes)));
    Ok((status, value))
}

async fn call_form(app: &Router, path: &str, client_id: &str, secret: &str, extra: &str) -> Result<(StatusCode, Value)> {
    let body = format!("client_id={client_id}&client_secret={secret}&{extra}");
    let request = Request::builder().method(Method::POST).uri(path)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body))?;
    let response = app.clone().oneshot(request).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((status, serde_json::from_slice(&bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes)))))
}

async fn session(
    pool: &PgPool,
    user: Uuid,
    tenant: Uuid,
    org: Uuid,
    secret: &str,
) -> Result<(String, Uuid)> {
    let now = chrono::Utc::now().timestamp();
    let version: i64 = sqlx::query_scalar("SELECT session_version FROM users WHERE id = $1")
        .bind(user)
        .fetch_one(pool)
        .await?;
    let session_id: Uuid = sqlx::query_scalar(
        "INSERT INTO platform_sessions (user_id, tenant_id, organization_id, expires_at) \
         VALUES ($1, $2, $3, now() + interval '1 hour') RETURNING id",
    )
    .bind(user)
    .bind(tenant)
    .bind(org)
    .fetch_one(pool)
    .await?;
    let token = jwt::sign(
        &Claims {
            sub: user.to_string(),
            typ: "session".into(),
            tenant: Some(tenant.to_string()),
            org: Some(org.to_string()),
            cid: None,
            scope: None,
            sid: Some(session_id.to_string()),
            ver: Some(version),
            aud: None,
            jti: None,
            iat: now,
            exp: now + 3600,
        },
        secret,
    )?;
    Ok((token, session_id))
}

async fn tenant(pool: &PgPool, suffix: &str, label: &str) -> Result<(Uuid, Uuid)> {
    let id: Uuid =
        sqlx::query_scalar("INSERT INTO tenants (name, code) VALUES ($1, $2) RETURNING id")
            .bind(label)
            .bind(format!("scope-{suffix}"))
            .fetch_one(pool)
            .await?;
    let org: Uuid = sqlx::query_scalar(
        "INSERT INTO organizations (tenant_id, name) VALUES ($1, $2) RETURNING id",
    )
    .bind(id)
    .bind(format!("{label} Root"))
    .fetch_one(pool)
    .await?;
    Ok((id, org))
}

async fn test_application(pool: &PgPool, suffix: &str) -> Result<(Uuid, String, String, String)> {
    let code = format!("scope-app-{suffix}");
    let app_id: Uuid = sqlx::query_scalar("INSERT INTO applications (code, name) VALUES ($1, $2) RETURNING id")
        .bind(&code).bind(&code).fetch_one(pool).await?;
    let client_id = format!("scope-client-{suffix}");
    let secret = format!("s{}", Uuid::new_v4().simple());
    sqlx::query("INSERT INTO oauth_clients \
        (client_id, client_secret_hash, client_name, client_type, grant_types, scopes, \
         access_token_validity, refresh_token_validity, application_id) \
         VALUES ($1, $2, $3, 'CONFIDENTIAL', 'client_credentials', 'profile', 7200, 604800, $4)")
        .bind(&client_id).bind(hash::sha256_hex(&secret)).bind(&client_id).bind(app_id)
        .execute(pool).await?;
    Ok((app_id, code, client_id, secret))
}

async fn test_user(pool: &PgPool, suffix: &str, orgs: &[Uuid]) -> Result<Uuid> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash) VALUES ($1, 'test-only') RETURNING id",
    )
    .bind(format!("scope-{suffix}"))
    .fetch_one(pool)
    .await?;
    for org in orgs {
        sqlx::query("INSERT INTO memberships (user_id, organization_id) VALUES ($1, $2)")
            .bind(id)
            .bind(org)
            .execute(pool)
            .await?;
    }
    Ok(id)
}

async fn grant_admin(
    pool: &PgPool,
    tenant: Uuid,
    org: Uuid,
    admin: Uuid,
    suffix: &str,
) -> Result<Uuid> {
    let role: Uuid = sqlx::query_scalar(
        "INSERT INTO roles (tenant_id, organization_id, name) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(tenant)
    .bind(org)
    .bind(format!("scope-admin-{suffix}"))
    .fetch_one(pool)
    .await?;
    let menus: Vec<(Uuid, Uuid)> =
        sqlx::query_as("SELECT id, application_id FROM menus WHERE permission = ANY($1)")
            .bind(&[
                "system:user",
                "system:role",
                "system:monitor",
                "system:organization",
                "system:department",
                "system:audit",
                "system:application",
                "system:menu",
                "system:tenant",
            ])
            .fetch_all(pool)
            .await?;
    ensure!(menus.len() == 9, "seed management menus missing");
    for (menu, app) in menus {
        sqlx::query("INSERT INTO tenant_applications (tenant_id, application_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(tenant).bind(app).execute(pool).await?;
        sqlx::query("INSERT INTO organization_applications (organization_id, application_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(org).bind(app).execute(pool).await?;
        sqlx::query(
            "INSERT INTO tenant_menus (tenant_id, menu_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(tenant)
        .bind(menu)
        .execute(pool)
        .await?;
        sqlx::query("INSERT INTO organization_menus (organization_id, menu_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(org).bind(menu).execute(pool).await?;
        sqlx::query(
            "INSERT INTO role_menus (role_id, menu_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(role)
        .bind(menu)
        .execute(pool)
        .await?;
    }
    sqlx::query("INSERT INTO user_roles (user_id, role_id, organization_id) VALUES ($1, $2, $3)")
        .bind(admin)
        .bind(role)
        .bind(org)
        .execute(pool)
        .await?;
    Ok(role)
}

async fn login_and_select(
    app: &Router,
    username: &str,
    password: &str,
    org: Uuid,
) -> Result<(String, String)> {
    let (status, login) = call(
        app,
        Method::POST,
        "/api/auth/login",
        "",
        Some(json!({"username":username,"password":password})),
    )
    .await?;
    ensure!(status == StatusCode::OK, "login failed: {status} {login}");
    let ticket = login["ticket"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing ticket"))?
        .to_string();
    let (status, context) = call(
        app,
        Method::POST,
        "/api/auth/context",
        &ticket,
        Some(json!({"organizationId":org.to_string()})),
    )
    .await?;
    ensure!(
        status == StatusCode::OK,
        "context failed: {status} {context}"
    );
    let token = context["accessToken"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing access token"))?
        .to_string();
    Ok((ticket, token))
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing at an isolated PostgreSQL test database"]
async fn tenant_management_scope_matrix() -> Result<()> {
    let url = std::env::var("TEST_DATABASE_URL")?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await?;
    let suffix = Uuid::new_v4().simple().to_string();
    let (tenant_a, org_a) = tenant(&pool, &format!("a-{suffix}"), "Scope A").await?;
    let (tenant_b, org_b) = tenant(&pool, &format!("b-{suffix}"), "Scope B").await?;
    let admin_a = test_user(&pool, &format!("admin-a-{suffix}"), &[org_a]).await?;
    let admin_b = test_user(&pool, &format!("admin-b-{suffix}"), &[org_b]).await?;
    let shared = test_user(&pool, &format!("shared-{suffix}"), &[org_a, org_b]).await?;
    let role_a = grant_admin(&pool, tenant_a, org_a, admin_a, &suffix).await?;
    let role_b = grant_admin(&pool, tenant_b, org_b, admin_b, &format!("b-{suffix}")).await?;
    let dept_b: Uuid = sqlx::query_scalar(
        "INSERT INTO departments (organization_id, name) VALUES ($1, 'B Dept') RETURNING id",
    )
    .bind(org_b)
    .fetch_one(&pool)
    .await?;
    for (role, org) in [(role_a, org_a), (role_b, org_b)] {
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_id, organization_id) VALUES ($1, $2, $3)",
        )
        .bind(shared)
        .bind(role)
        .bind(org)
        .execute(&pool)
        .await?;
    }
    let (platform_user, platform_tenant, platform_org): (Uuid, Uuid, Uuid) = sqlx::query_as(
        "SELECT u.id, t.id, o.id FROM users u, tenants t, organizations o \
         WHERE u.is_super_admin = true AND t.is_platform = true AND o.tenant_id = t.id \
         AND o.parent_id IS NULL LIMIT 1",
    )
    .fetch_one(&pool)
    .await?;

    let secret = "isolated-scope-test-only";
    let mut config = Config::from_env();
    config.database_url = url;
    config.jwt_secret = secret.into();
    let app = Router::new()
        .merge(routes::api_router())
        .with_state(AppState::new(pool.clone(), Arc::new(config)));
    let (a, _) = session(&pool, admin_a, tenant_a, org_a, secret).await?;
    let (b, _) = session(&pool, admin_b, tenant_b, org_b, secret).await?;
    let (platform, platform_session_id) =
        session(&pool, platform_user, platform_tenant, platform_org, secret).await?;
    let created_username = format!("scope-created-{suffix}");

    let result: Result<()> = async {
        let (status, body) = call(&app, Method::GET, "/api/tenants?size=200", &a, None).await?;
        ensure!(status == StatusCode::OK && !body.to_string().contains(&tenant_b.to_string()), "A tenant list leaked B");
        let (status, body) = call(&app, Method::GET, "/api/tenants?size=200", &b, None).await?;
        ensure!(status == StatusCode::OK && !body.to_string().contains(&tenant_a.to_string()), "B tenant list leaked A");
        let (status, body) = call(&app, Method::GET, "/api/users?size=200", &a, None).await?;
        ensure!(status == StatusCode::OK, "A user list: {status} {body}");
        let serialized = body.to_string();
        ensure!(!serialized.contains(&tenant_b.to_string()) && !serialized.contains(&org_b.to_string())
            && !serialized.contains(&admin_b.to_string()), "A user list leaked B data");
        let (status, body) = call(&app, Method::GET, "/api/users?size=200", &b, None).await?;
        ensure!(status == StatusCode::OK && !body.to_string().contains(&org_a.to_string())
            && !body.to_string().contains(&admin_a.to_string()), "B user list leaked A data");
        let (status, body) = call(&app, Method::GET, &format!("/api/search?q=scope-shared-{suffix}"), &b, None).await?;
        ensure!(status == StatusCode::OK && !body.to_string().contains("Scope A Root"), "B search leaked A organization name");
        let (status, body) = call(&app, Method::GET, &format!("/api/users/{shared}/memberships"), &a, None).await?;
        ensure!(status == StatusCode::OK && !body.to_string().contains(&org_b.to_string()), "A membership detail leaked B data");
        let (status, body) = call(&app, Method::GET, &format!("/api/users/{shared}/roles"), &a, None).await?;
        ensure!(status == StatusCode::OK && !body.to_string().contains(&role_b.to_string()), "A role detail leaked B data");
        let (status, body) = call(&app, Method::GET, "/api/roles?size=200", &a, None).await?;
        ensure!(status == StatusCode::OK && !body.to_string().contains(&role_b.to_string()), "A role list leaked B");
        let (status, body) = call(&app, Method::GET, "/api/roles?size=200", &b, None).await?;
        ensure!(status == StatusCode::OK && !body.to_string().contains(&role_a.to_string()), "B role list leaked A");
        let (status, _) = call(&app, Method::GET, &format!("/api/organizations/tree?tenantId={tenant_a}"), &b, None).await?;
        ensure!(status == StatusCode::NOT_FOUND, "B saw A organization tree");
        let (status, _) = call(&app, Method::PUT, &format!("/api/users/{admin_a}"), &b,
            Some(json!({"realName":"Denied"}))).await?;
        ensure!(status == StatusCode::NOT_FOUND, "B changed A user");
        let (status, _) = call(&app, Method::PUT, &format!("/api/tenants/{tenant_b}/status"), &a,
            Some(json!({"status":"DISABLED"}))).await?;
        ensure!(status == StatusCode::FORBIDDEN, "A changed B tenant status");
        for (method, path, body) in [
            (Method::GET, format!("/api/organizations/tree?tenantId={tenant_b}"), None),
            (Method::PUT, format!("/api/organizations/{org_b}"), Some(json!({"name":"Denied"}))),
            (Method::PUT, format!("/api/organizations/{org_b}/status"), Some(json!({"status":"DISABLED"}))),
            (Method::DELETE, format!("/api/organizations/{org_b}"), None),
            (Method::GET, format!("/api/departments/tree?organizationId={org_b}"), None),
            (Method::PUT, format!("/api/departments/{dept_b}"), Some(json!({"name":"Denied"}))),
            (Method::DELETE, format!("/api/departments/{dept_b}"), None),
            (Method::GET, format!("/api/roles/{role_b}/menus"), None),
            (Method::DELETE, format!("/api/roles/{role_b}"), None),
            (Method::GET, format!("/api/users/{admin_b}/roles"), None),
            (Method::PUT, format!("/api/users/{admin_b}"), Some(json!({"realName":"Denied"}))),
            (Method::DELETE, format!("/api/users/{admin_b}"), None),
        ] {
            let (status, _) = call(&app, method, &path, &a, body).await?;
            ensure!(status == StatusCode::NOT_FOUND, "cross-tenant path {path} returned {status}");
        }
        for (path, body) in [
            ("/api/organizations", json!({"tenantId":tenant_b.to_string(),"parentId":org_b.to_string(),"name":"Denied"})),
            ("/api/departments", json!({"organizationId":org_b.to_string(),"name":"Denied"})),
            ("/api/roles", json!({"organizationId":org_b.to_string(),"name":"Denied"})),
        ] {
            let (status, _) = call(&app, Method::POST, path, &a, Some(body)).await?;
            ensure!(status == StatusCode::NOT_FOUND, "cross-tenant create {path} returned {status}");
        }
        let (status, _) = call(&app, Method::PUT, &format!("/api/roles/{role_a}/users"), &a,
            Some(json!({"userIds":[admin_b]}))).await?;
        ensure!(status == StatusCode::NOT_FOUND, "A assigned B user to A role");
        let (status, _) = call(&app, Method::PUT, &format!("/api/users/{shared}/memberships"), &a,
            Some(json!({"items":[{"organizationId":org_b.to_string()}]}))).await?;
        ensure!(status == StatusCode::NOT_FOUND, "A assigned B membership");
        let (status, _) = call(&app, Method::PUT, &format!("/api/users/{shared}/status"), &a, Some(json!({"status":"DISABLED"}))).await?;
        ensure!(status == StatusCode::FORBIDDEN, "A changed a shared global account");
        let (status, body) = call(&app, Method::POST, "/api/users/batch-status", &a,
            Some(json!({"userIds":[shared,admin_b],"status":"DISABLED"}))).await?;
        ensure!(status == StatusCode::OK && body["failed"].as_array().map_or(false, |a| a.len() == 2), "A batch changed cross-tenant/shared account");
        let (status, _) = call(&app, Method::PUT, &format!("/api/roles/{role_b}"), &a, Some(json!({"name":"Denied"}))).await?;
        ensure!(status == StatusCode::NOT_FOUND, "A changed B role");
        let (status, _) = call(&app, Method::GET, "/api/monitor/overview", &a, None).await?;
        ensure!(status == StatusCode::FORBIDDEN, "A accessed global monitor");
        let (status, _) = call(&app, Method::GET, "/api/applications", &a, None).await?;
        ensure!(status == StatusCode::FORBIDDEN, "A accessed global application catalog");
        let (status, _) = call(&app, Method::GET, "/api/menus/tree?applicationId=00000000-0000-0000-0000-000000000000", &a, None).await?;
        ensure!(status == StatusCode::FORBIDDEN, "A accessed global menu catalog");
        let (status, body) = call(&app, Method::GET, "/api/monitor/tenant-insights", &a, None).await?;
        ensure!(status == StatusCode::OK && !body.to_string().contains(&tenant_b.to_string()), "A monitor leaked B data");
        let (status, body) = call(&app, Method::GET, "/api/monitor/tenant-insights/export", &a, None).await?;
        ensure!(status == StatusCode::OK && !body.to_string().contains("Scope B"), "A monitor CSV leaked B data");
        let (status, _) = call(&app, Method::PUT, &format!("/api/users/{admin_a}"), &a, Some(json!({"realName":"Allowed A"}))).await?;
        ensure!(status == StatusCode::OK, "A same-tenant update failed");
        let (status, body) = call(&app, Method::POST, "/api/users", &a,
            Some(json!({"username":created_username,"password":"test-only-password"}))).await?;
        ensure!(status == StatusCode::OK, "A same-tenant user create failed: {status} {body}");
        let created_id: Uuid = body["id"].as_str().ok_or_else(|| anyhow::anyhow!("missing created user id"))?.parse()?;
        let created_org: Uuid = sqlx::query_scalar("SELECT organization_id FROM memberships WHERE user_id = $1")
            .bind(created_id).fetch_one(&pool).await?;
        ensure!(created_org == org_a, "new tenant user did not join the current organization");
        let (status, body) = call(&app, Method::POST, "/api/organizations", &a,
            Some(json!({"tenantId":tenant_a.to_string(),"parentId":org_a.to_string(),"name":"Allowed A Child"}))).await?;
        ensure!(status == StatusCode::OK, "A same-tenant organization create failed: {status} {body}");
        let child_a = body["id"].as_str().ok_or_else(|| anyhow::anyhow!("missing child organization id"))?;
        let (status, _) = call(&app, Method::POST, "/api/departments", &a,
            Some(json!({"organizationId":child_a,"name":"Allowed A Dept"}))).await?;
        ensure!(status == StatusCode::OK, "A same-tenant department create failed");
        let (status, _) = call(&app, Method::POST, "/api/roles", &a,
            Some(json!({"organizationId":child_a,"name":"Allowed A Role"}))).await?;
        ensure!(status == StatusCode::OK, "A same-tenant role create failed");
        let (status, _) = call(&app, Method::PUT, &format!("/api/roles/{role_b}"), &platform, Some(json!({"name":"Platform Updated B"}))).await?;
        ensure!(status == StatusCode::OK, "platform cross-tenant update failed");
        let (status, _) = call(&app, Method::GET, "/api/monitor/overview", &platform, None).await?;
        ensure!(status == StatusCode::OK, "platform global monitor unavailable");
        let (status, _) = call(&app, Method::GET, "/api/applications", &platform, None).await?;
        ensure!(status == StatusCode::OK, "platform application catalog unavailable");
        let (status, _) = call(&app, Method::PUT, &format!("/api/users/{shared}/status"), &platform,
            Some(json!({"status":"DISABLED"}))).await?;
        ensure!(status == StatusCode::OK, "platform shared-user update failed");
        let logged_tenants: i64 = sqlx::query_scalar("SELECT count(DISTINCT tenant_id) FROM audit_logs \
            WHERE user_id = $1 AND action = 'UPDATE_USER' AND tenant_id = ANY($2)")
            .bind(platform_user).bind(&[tenant_a, tenant_b]).fetch_one(&pool).await?;
        ensure!(logged_tenants == 2, "platform shared-user audit omitted a target tenant");
        let (status, _) = call(&app, Method::PUT, &format!("/api/organizations/{org_b}"), &platform,
            Some(json!({"name":"Platform B"}))).await?;
        ensure!(status == StatusCode::OK, "platform cross-tenant organization update failed");
        let (status, _) = call(&app, Method::PUT, &format!("/api/departments/{dept_b}"), &platform,
            Some(json!({"name":"Platform B Dept"}))).await?;
        ensure!(status == StatusCode::OK, "platform cross-tenant department update failed");
        let logged: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM audit_logs WHERE user_id = $1 AND tenant_id = $2 AND action = 'UPDATE_ROLE')")
            .bind(platform_user).bind(tenant_b).fetch_one(&pool).await?;
        ensure!(logged, "platform cross-tenant audit missing target tenant");
        let other_logs: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE user_id = $1 AND tenant_id = $2 \
            AND action IN ('UPDATE_ORGANIZATION','UPDATE_DEPARTMENT')")
            .bind(platform_user).bind(tenant_b).fetch_one(&pool).await?;
        ensure!(other_logs >= 2, "platform organization/department audit missing target tenant");
        let (status, body) = call(&app, Method::GET, "/api/audit-logs?size=200", &a, None).await?;
        ensure!(status == StatusCode::OK && !body.to_string().contains("Platform Updated B"), "A audit list leaked B data");
        let (status, body) = call(&app, Method::GET, "/api/audit-logs/export", &a, None).await?;
        ensure!(status == StatusCode::OK && !body.to_string().contains("Platform Updated B"), "A audit CSV leaked B data");
        let (status, _) = call(&app, Method::PUT, &format!("/api/users/{shared}/roles"), &a, Some(json!({"items":[]}))).await?;
        ensure!(status == StatusCode::OK, "A scoped role replacement failed");
        let b_role_count: i64 = sqlx::query_scalar("SELECT count(*) FROM user_roles WHERE user_id = $1 AND role_id = $2")
            .bind(shared).bind(role_b).fetch_one(&pool).await?;
        ensure!(b_role_count == 1, "A role replacement deleted B role");
        let (status, _) = call(&app, Method::PUT, &format!("/api/users/{shared}/memberships"), &a, Some(json!({"items":[]}))).await?;
        ensure!(status == StatusCode::OK, "A scoped membership replacement failed");
        let b_member_count: i64 = sqlx::query_scalar("SELECT count(*) FROM memberships WHERE user_id = $1 AND organization_id = $2")
            .bind(shared).bind(org_b).fetch_one(&pool).await?;
        ensure!(b_member_count == 1, "A membership replacement deleted B membership");
        let (status, _) = call(&app, Method::PUT, &format!("/api/tenants/{tenant_b}/status"), &platform,
            Some(json!({"status":"DISABLED"}))).await?;
        ensure!(status == StatusCode::OK, "platform tenant status update failed");
        let (status, _) = call(&app, Method::DELETE, &format!("/api/tenants/{tenant_b}"), &platform, None).await?;
        ensure!(status == StatusCode::OK, "platform tenant delete failed");
        let target_audit_count: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE user_id = $1 AND tenant_id = $2 \
            AND action IN ('UPDATE_TENANT','DELETE_TENANT')")
            .bind(platform_user).bind(tenant_b).fetch_one(&pool).await?;
        ensure!(target_audit_count >= 2, "platform tenant status/delete audit omitted target tenant");
        Ok(())
    }.await;

    sqlx::query("DELETE FROM audit_logs WHERE tenant_id = ANY($1)")
        .bind(&[tenant_a, tenant_b])
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM tenants WHERE id = ANY($1)")
        .bind(&[tenant_a, tenant_b])
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM users WHERE username = $1")
        .bind(&created_username)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(&[admin_a, admin_b, shared])
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM platform_sessions WHERE id = $1")
        .bind(platform_session_id)
        .execute(&pool)
        .await?;
    result
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing at an isolated PostgreSQL test database"]
async fn platform_session_lifecycle() -> Result<()> {
    let url = std::env::var("TEST_DATABASE_URL")?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await?;
    let suffix = Uuid::new_v4().simple().to_string();
    let (tenant_id, root) = tenant(&pool, &format!("session-{suffix}"), "Session test").await?;
    let child: Uuid = sqlx::query_scalar("INSERT INTO organizations (tenant_id, parent_id, name) VALUES ($1, $2, 'Child') RETURNING id")
        .bind(tenant_id).bind(root).fetch_one(&pool).await?;
    let unrelated: Uuid = sqlx::query_scalar("INSERT INTO organizations (tenant_id, parent_id, name) VALUES ($1, $2, 'Unrelated') RETURNING id")
        .bind(tenant_id).bind(root).fetch_one(&pool).await?;
    let username = format!("session-user-{suffix}");
    let initial_hash = password::hash_password("StartPass123")?;
    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash) VALUES ($1, $2) RETURNING id",
    )
    .bind(&username)
    .bind(initial_hash)
    .fetch_one(&pool)
    .await?;
    for org in [root, child] {
        sqlx::query("INSERT INTO memberships (user_id, organization_id) VALUES ($1, $2)")
            .bind(user_id)
            .bind(org)
            .execute(&pool)
            .await?;
    }
    let (admin_id, admin_tenant, admin_org): (Uuid, Uuid, Uuid) = sqlx::query_as(
        "SELECT u.id, t.id, o.id FROM users u, tenants t, organizations o \
         WHERE u.is_super_admin = true AND t.is_platform = true AND o.tenant_id = t.id \
         AND o.parent_id IS NULL LIMIT 1",
    )
    .fetch_one(&pool)
    .await?;
    let secret = "isolated-session-test-only";
    let mut config = Config::from_env();
    config.database_url = url;
    config.jwt_secret = secret.into();
    let app = Router::new()
        .merge(routes::api_router())
        .with_state(AppState::new(pool.clone(), Arc::new(config)));
    let (admin_token, admin_session_id) =
        session(&pool, admin_id, admin_tenant, admin_org, secret).await?;

    let result: Result<()> = async {
        let (old_ticket, first) = login_and_select(&app, &username, "StartPass123", root).await?;
        let (_, second) = login_and_select(&app, &username, "StartPass123", root).await?;
        let (status, listing) =
            call(&app, Method::GET, "/api/auth/my-sessions", &first, None).await?;
        ensure!(status == StatusCode::OK, "session list failed");
        ensure!(
            listing["platformSessions"]
                .as_array()
                .map_or(false, |items| items
                    .iter()
                    .filter(|item| item["active"] == true)
                    .count()
                    >= 2),
            "two independent device sessions were not listed"
        );

        let (status, _) = call(&app, Method::POST, "/api/auth/logout", &first, None).await?;
        ensure!(status == StatusCode::OK, "logout failed");
        ensure!(
            call(&app, Method::GET, "/api/auth/profile", &first, None)
                .await?
                .0
                == StatusCode::UNAUTHORIZED,
            "logged-out session remained active"
        );
        ensure!(
            call(&app, Method::GET, "/api/auth/profile", &second, None)
                .await?
                .0
                == StatusCode::OK,
            "logout revoked another device"
        );

        let (status, switched) = call(
            &app,
            Method::POST,
            "/api/auth/context",
            &second,
            Some(json!({"organizationId":child.to_string()})),
        )
        .await?;
        ensure!(
            status == StatusCode::OK,
            "valid context switch failed: {switched}"
        );
        let switched_token = switched["accessToken"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing switched token"))?;
        ensure!(
            call(&app, Method::GET, "/api/auth/profile", &second, None)
                .await?
                .0
                == StatusCode::UNAUTHORIZED,
            "old context session remained active"
        );
        ensure!(
            call(&app, Method::GET, "/api/auth/profile", switched_token, None)
                .await?
                .0
                == StatusCode::OK,
            "new context session was not active"
        );
        ensure!(
            call(
                &app,
                Method::POST,
                "/api/auth/context",
                switched_token,
                Some(json!({"organizationId":unrelated.to_string()}))
            )
            .await?
            .0 == StatusCode::FORBIDDEN,
            "user switched into organization without direct membership"
        );
        ensure!(
            call(
                &app,
                Method::POST,
                "/api/auth/context",
                &second,
                Some(json!({"organizationId":root.to_string()}))
            )
            .await?
            .0 == StatusCode::UNAUTHORIZED,
            "concurrent reuse of old context token was accepted"
        );

        let (third, third_id) = session(&pool, user_id, tenant_id, root, secret).await?;
        ensure!(
            call(
                &app,
                Method::DELETE,
                &format!("/api/auth/sessions/{third_id}"),
                switched_token,
                None
            )
            .await?
            .0 == StatusCode::OK,
            "self-service device revocation failed"
        );
        ensure!(
            call(&app, Method::GET, "/api/auth/profile", &third, None)
                .await?
                .0
                == StatusCode::UNAUTHORIZED,
            "revoked device remained active"
        );
        ensure!(
            call(&app, Method::GET, "/api/auth/profile", switched_token, None)
                .await?
                .0
                == StatusCode::OK,
            "device revocation affected current device"
        );

        ensure!(
            call(
                &app,
                Method::PUT,
                "/api/auth/password",
                switched_token,
                Some(json!({"oldPassword":"StartPass123","newPassword":"NextPass123"}))
            )
            .await?
            .0 == StatusCode::OK,
            "personal password change failed"
        );
        ensure!(
            call(&app, Method::GET, "/api/auth/profile", switched_token, None)
                .await?
                .0
                == StatusCode::UNAUTHORIZED,
            "password change did not revoke current session"
        );
        ensure!(
            call(
                &app,
                Method::POST,
                "/api/auth/context",
                &old_ticket,
                Some(json!({"organizationId":root.to_string()}))
            )
            .await?
            .0 == StatusCode::UNAUTHORIZED,
            "old credential ticket survived password change"
        );
        ensure!(
            call(
                &app,
                Method::POST,
                "/api/auth/login",
                "",
                Some(json!({"username":username,"password":"StartPass123"}))
            )
            .await?
            .0 == StatusCode::UNAUTHORIZED,
            "old password still worked"
        );
        let (ticket_after_change, after_change) =
            login_and_select(&app, &username, "NextPass123", root).await?;

        let (status, kicked) = call(
            &app,
            Method::POST,
            "/api/oauth/tokens/kick",
            &admin_token,
            Some(json!({"userId":user_id.to_string()})),
        )
        .await?;
        ensure!(
            status == StatusCode::OK && kicked["platformSessionsRevoked"].as_u64().unwrap_or(0) > 0,
            "administrator kick did not revoke platform sessions"
        );
        ensure!(
            call(&app, Method::GET, "/api/auth/profile", &after_change, None)
                .await?
                .0
                == StatusCode::UNAUTHORIZED,
            "kicked session remained active"
        );
        ensure!(
            call(
                &app,
                Method::POST,
                "/api/auth/context",
                &ticket_after_change,
                Some(json!({"organizationId":root.to_string()}))
            )
            .await?
            .0 == StatusCode::UNAUTHORIZED,
            "credential ticket survived administrator kick"
        );

        ensure!(
            call(
                &app,
                Method::POST,
                &format!("/api/users/{user_id}/reset-password"),
                &admin_token,
                Some(json!({"newPassword":"ResetPass123"}))
            )
            .await?
            .0 == StatusCode::OK,
            "administrator password reset failed"
        );
        ensure!(
            call(
                &app,
                Method::POST,
                "/api/auth/login",
                "",
                Some(json!({"username":username,"password":"NextPass123"}))
            )
            .await?
            .0 == StatusCode::UNAUTHORIZED,
            "old password survived administrator reset"
        );
        let (_, after_reset) = login_and_select(&app, &username, "ResetPass123", root).await?;
        sqlx::query("DELETE FROM memberships WHERE user_id = $1 AND organization_id = $2")
            .bind(user_id)
            .bind(root)
            .execute(&pool)
            .await?;
        ensure!(
            call(&app, Method::GET, "/api/auth/profile", &after_reset, None)
                .await?
                .0
                == StatusCode::UNAUTHORIZED,
            "member removal did not invalidate session"
        );
        sqlx::query("INSERT INTO memberships (user_id, organization_id) VALUES ($1, $2)")
            .bind(user_id)
            .bind(root)
            .execute(&pool)
            .await?;
        ensure!(
            call(&app, Method::GET, "/api/auth/profile", &after_reset, None)
                .await?
                .0
                == StatusCode::UNAUTHORIZED,
            "re-adding member revived old session"
        );

        let (org_token, _) = session(&pool, user_id, tenant_id, child, secret).await?;
        sqlx::query("UPDATE organizations SET status = 'DISABLED' WHERE id = $1")
            .bind(root)
            .execute(&pool)
            .await?;
        ensure!(
            call(&app, Method::GET, "/api/auth/profile", &org_token, None)
                .await?
                .0
                == StatusCode::UNAUTHORIZED,
            "disabled ancestor organization did not invalidate child session"
        );
        sqlx::query("UPDATE organizations SET status = 'ENABLED' WHERE id = $1")
            .bind(root)
            .execute(&pool)
            .await?;
        ensure!(
            call(&app, Method::GET, "/api/auth/profile", &org_token, None)
                .await?
                .0
                == StatusCode::UNAUTHORIZED,
            "re-enabled organization revived old session"
        );

        let (org_expiry_token, _) = session(&pool, user_id, tenant_id, root, secret).await?;
        sqlx::query(
            "UPDATE organizations SET expires_at = now() - interval '1 second' WHERE id = $1",
        )
        .bind(root)
        .execute(&pool)
        .await?;
        ensure!(
            call(
                &app,
                Method::GET,
                "/api/auth/profile",
                &org_expiry_token,
                None
            )
            .await?
            .0 == StatusCode::UNAUTHORIZED,
            "expired organization did not invalidate session"
        );
        sqlx::query("UPDATE organizations SET expires_at = NULL WHERE id = $1")
            .bind(root)
            .execute(&pool)
            .await?;
        ensure!(
            call(
                &app,
                Method::GET,
                "/api/auth/profile",
                &org_expiry_token,
                None
            )
            .await?
            .0 == StatusCode::UNAUTHORIZED,
            "extending organization revived old session"
        );

        let (tenant_token, _) = session(&pool, user_id, tenant_id, root, secret).await?;
        sqlx::query("UPDATE tenants SET status = 'DISABLED' WHERE id = $1")
            .bind(tenant_id)
            .execute(&pool)
            .await?;
        ensure!(
            call(&app, Method::GET, "/api/auth/profile", &tenant_token, None)
                .await?
                .0
                == StatusCode::UNAUTHORIZED,
            "disabled tenant did not invalidate session"
        );
        sqlx::query("UPDATE tenants SET status = 'ENABLED' WHERE id = $1")
            .bind(tenant_id)
            .execute(&pool)
            .await?;
        ensure!(
            call(&app, Method::GET, "/api/auth/profile", &tenant_token, None)
                .await?
                .0
                == StatusCode::UNAUTHORIZED,
            "re-enabled tenant revived old session"
        );

        let (tenant_expiry_token, _) = session(&pool, user_id, tenant_id, root, secret).await?;
        sqlx::query("UPDATE tenants SET expires_at = now() - interval '1 second' WHERE id = $1")
            .bind(tenant_id)
            .execute(&pool)
            .await?;
        ensure!(
            call(
                &app,
                Method::GET,
                "/api/auth/profile",
                &tenant_expiry_token,
                None
            )
            .await?
            .0 == StatusCode::UNAUTHORIZED,
            "expired tenant did not invalidate session"
        );
        sqlx::query("UPDATE tenants SET expires_at = NULL WHERE id = $1")
            .bind(tenant_id)
            .execute(&pool)
            .await?;
        ensure!(
            call(
                &app,
                Method::GET,
                "/api/auth/profile",
                &tenant_expiry_token,
                None
            )
            .await?
            .0 == StatusCode::UNAUTHORIZED,
            "extending tenant revived old session"
        );

        let (user_status_token, _) = session(&pool, user_id, tenant_id, root, secret).await?;
        sqlx::query("UPDATE users SET status = 'DISABLED' WHERE id = $1")
            .bind(user_id)
            .execute(&pool)
            .await?;
        ensure!(
            call(
                &app,
                Method::GET,
                "/api/auth/profile",
                &user_status_token,
                None
            )
            .await?
            .0 == StatusCode::UNAUTHORIZED,
            "disabled user did not invalidate session"
        );
        sqlx::query("UPDATE users SET status = 'ENABLED' WHERE id = $1")
            .bind(user_id)
            .execute(&pool)
            .await?;
        ensure!(
            call(
                &app,
                Method::GET,
                "/api/auth/profile",
                &user_status_token,
                None
            )
            .await?
            .0 == StatusCode::UNAUTHORIZED,
            "re-enabled user revived old session"
        );

        let legacy = jwt::sign(
            &Claims {
                sub: user_id.to_string(),
                typ: "session".into(),
                tenant: Some(tenant_id.to_string()),
                org: Some(root.to_string()),
                cid: None,
                scope: None,
                sid: None,
                ver: None,
                aud: None,
                jti: None,
                iat: chrono::Utc::now().timestamp(),
                exp: chrono::Utc::now().timestamp() + 3600,
            },
            secret,
        )?;
        ensure!(
            call(&app, Method::GET, "/api/auth/profile", &legacy, None)
                .await?
                .0
                == StatusCode::UNAUTHORIZED,
            "legacy session without sid/ver was accepted"
        );
        Ok(())
    }
    .await;

    sqlx::query("DELETE FROM audit_logs WHERE user_id = $1 OR tenant_id = $2")
        .bind(user_id)
        .bind(tenant_id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM platform_sessions WHERE id = $1")
        .bind(admin_session_id)
        .execute(&pool)
        .await?;
    result
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing at an isolated PostgreSQL test database"]
async fn application_token_audience_and_revocation() -> Result<()> {
    let url = std::env::var("TEST_DATABASE_URL")?;
    let pool = PgPoolOptions::new().max_connections(5).connect(&url).await?;
    let suffix = Uuid::new_v4().simple().to_string();
    let (tenant_a, org_a) = tenant(&pool, &format!("app-a-{suffix}"), "App tenant A").await?;
    let (tenant_b, org_b) = tenant(&pool, &format!("app-b-{suffix}"), "App tenant B").await?;
    let user_a = test_user(&pool, &format!("app-a-{suffix}"), &[org_a]).await?;
    let user_b = test_user(&pool, &format!("app-b-{suffix}"), &[org_b]).await?;
    let (app_a, code_a, client_a, secret_a) = test_application(&pool, &format!("a-{suffix}")).await?;
    let (app_b, code_b, client_b, secret_b) = test_application(&pool, &format!("b-{suffix}")).await?;
    for (tenant, org, app_id) in [(tenant_a, org_a, app_a), (tenant_b, org_b, app_b)] {
        sqlx::query("INSERT INTO tenant_applications (tenant_id, application_id) VALUES ($1, $2)")
            .bind(tenant).bind(app_id).execute(&pool).await?;
        sqlx::query("INSERT INTO organization_applications (organization_id, application_id) VALUES ($1, $2)")
            .bind(org).bind(app_id).execute(&pool).await?;
    }
    let secret = "isolated-app-token-test-only";
    let mut config = Config::from_env();
    config.database_url = url;
    config.jwt_secret = secret.into();
    let config = Arc::new(config);
    let app = Router::new().merge(routes::api_router())
        .nest("/oauth", routes::oauth2::oauth_router())
        .with_state(AppState::new(pool.clone(), config.clone()));
    let peer = Router::new().merge(routes::api_router())
        .nest("/oauth", routes::oauth2::oauth_router())
        .with_state(AppState::new(pool.clone(), config));
    let (session_a, _) = session(&pool, user_a, tenant_a, org_a, secret).await?;
    let (session_b, _) = session(&pool, user_b, tenant_b, org_b, secret).await?;

    let result: Result<()> = async {
        let (status, issued) = call(&app, Method::POST, "/api/oauth/session-token", &session_a,
            Some(json!({"appCode":code_a}))).await?;
        ensure!(status == StatusCode::CREATED, "authorized app token issue failed: {status} {issued}");
        ensure!(issued["expiresIn"].as_i64().is_some_and(|ttl| ttl > 0 && ttl <= 300),
            "app token TTL exceeded five minutes");
        ensure!(issued["audience"] == code_a, "app audience missing from response");
        ensure!(!issued.to_string().contains("clientSecret") && !issued.to_string().contains(&secret_a),
            "token response disclosed client secret");
        let token_a = issued["accessToken"].as_str().ok_or_else(|| anyhow::anyhow!("missing app token"))?.to_string();
        let claims = jwt::verify(&token_a, secret)?;
        ensure!(claims.sub == user_a.to_string() && claims.tenant.as_deref() == Some(tenant_a.to_string().as_str())
            && claims.org.as_deref() == Some(org_a.to_string().as_str())
            && claims.aud.as_deref() == Some(code_a.as_str()) && claims.jti.is_some(),
            "app JWT subject/context/audience/jti mismatch");
        let stored: (String, Option<String>, Option<Uuid>, Option<Uuid>, Option<Uuid>, Option<String>) = sqlx::query_as(
            "SELECT token_use, audience, jti, application_id, platform_session_id, jwt \
             FROM oauth_tokens WHERE token_hash = $1",
        ).bind(hash::sha256_hex(&token_a)).fetch_one(&pool).await?;
        ensure!(stored.0 == "app" && stored.1.as_deref() == Some(code_a.as_str())
            && stored.2.map(|j| j.to_string()) == claims.jti && stored.3 == Some(app_a)
            && stored.4.is_some() && stored.5.is_none(), "app token database binding incomplete");
        let (status, second) = call(&app, Method::POST, "/api/oauth/session-token", &session_a,
            Some(json!({"appCode":code_a}))).await?;
        ensure!(status == StatusCode::CREATED && second["accessToken"] != token_a,
            "same-second app token issuance reused JWT");
        let token_a2 = second["accessToken"].as_str().ok_or_else(|| anyhow::anyhow!("missing second token"))?.to_string();

        ensure!(call(&app, Method::POST, "/api/oauth/session-token", &session_a,
            Some(json!({"appCode":code_b}))).await?.0 == StatusCode::NOT_FOUND,
            "A obtained B application token");
        ensure!(call(&app, Method::POST, "/api/oauth/session-token", &session_b,
            Some(json!({"appCode":code_a}))).await?.0 == StatusCode::NOT_FOUND,
            "B obtained A application token");
        let (status, issued_b) = call(&app, Method::POST, "/api/oauth/session-token", &session_b,
            Some(json!({"appCode":code_b}))).await?;
        ensure!(status == StatusCode::CREATED, "B legitimate application token failed");
        let token_b = issued_b["accessToken"].as_str()
            .ok_or_else(|| anyhow::anyhow!("missing B app token"))?.to_string();

        let (status, active_a) = call_form(&app, "/oauth/introspect", &client_a, &secret_a,
            &format!("token={token_a}")).await?;
        ensure!(status == StatusCode::OK && active_a["active"] == true && active_a["aud"] == code_a
            && active_a["jti"] == claims.jti.as_deref().unwrap_or_default(),
            "A backend could not introspect A token");
        let (_, wrong_audience) = call_form(&app, "/oauth/introspect", &client_b, &secret_b,
            &format!("token={token_a}")).await?;
        ensure!(wrong_audience["active"] == false, "B backend accepted A token");
        ensure!(call(&app, Method::GET, "/oauth/userinfo", &token_a, None).await?.0 == StatusCode::OK,
            "authorized app token broke userinfo compatibility");
        let (status, external) = call(&app, Method::GET, "/api/external/menus", &token_a, None).await?;
        ensure!(status == StatusCode::OK && external["application"]["id"] == app_a.to_string(),
            "authorized app token broke external menu compatibility");

        let context_path = format!("/api/apps/{code_a}/context");
        let (status, empty_context) = call(&app, Method::GET, &context_path, &token_a, None).await?;
        ensure!(status == StatusCode::OK && empty_context["contractVersion"] == "1.0"
            && empty_context["dataGrants"].as_array().is_some_and(Vec::is_empty),
            "app context did not default deny data access: {status} {empty_context}");
        ensure!(call(&app, Method::GET, &format!("/api/apps/{code_b}/context"), &token_a, None)
            .await?.0 == StatusCode::FORBIDDEN, "A token accessed B context");
        let (_, context_b) = call(&app, Method::GET, &format!("/api/apps/{code_b}/context"),
            &token_b, None).await?;
        ensure!(context_b["tenant"]["id"] == tenant_b.to_string()
            && context_b["organization"]["id"] == org_b.to_string()
            && context_b["dataGrants"].as_array().is_some_and(Vec::is_empty),
            "B context leaked A tenant or data grants");
        ensure!(call(&app, Method::GET, &context_path, &token_b, None).await?.0 == StatusCode::FORBIDDEN,
            "B token accessed A context");
        let role_id: Uuid = sqlx::query_scalar(
            "INSERT INTO roles (tenant_id, organization_id, name, code) \
             VALUES ($1, $2, $3, 'APP_TEST') RETURNING id",
        ).bind(tenant_a).bind(org_a).bind(format!("app-test-{suffix}"))
         .fetch_one(&pool).await?;
        let menu_id: Uuid = sqlx::query_scalar(
            "INSERT INTO menus (application_id, type, title, permission, path) \
             VALUES ($1, 'BUTTON', 'Read orders', 'order:read', '/orders') RETURNING id",
        ).bind(app_a).fetch_one(&pool).await?;
        sqlx::query(
            "INSERT INTO menus (application_id, type, title, permission, path) \
             VALUES ($1, 'BUTTON', 'Hidden admin action', 'order:admin', '/orders/admin')",
        ).bind(app_a).execute(&pool).await?;
        sqlx::query("INSERT INTO user_roles (user_id, role_id, organization_id) VALUES ($1, $2, $3)")
            .bind(user_a).bind(role_id).bind(org_a).execute(&pool).await?;
        sqlx::query("INSERT INTO role_menus (role_id, menu_id) VALUES ($1, $2)")
            .bind(role_id).bind(menu_id).execute(&pool).await?;
        sqlx::query("INSERT INTO tenant_menus (tenant_id, menu_id) VALUES ($1, $2)")
            .bind(tenant_a).bind(menu_id).execute(&pool).await?;
        sqlx::query("INSERT INTO organization_menus (organization_id, menu_id) VALUES ($1, $2)")
            .bind(org_a).bind(menu_id).execute(&pool).await?;
        let member_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM memberships WHERE user_id = $1 AND organization_id = $2",
        ).bind(user_a).bind(org_a).fetch_one(&pool).await?;
        sqlx::query(
            "INSERT INTO membership_data_grants \
             (membership_id, application_id, resource_code, action_code, scope_type) \
             VALUES ($1, $2, 'order', 'read', 'ORG')",
        ).bind(member_id).bind(app_a).execute(&pool).await?;
        let (_, context_with_grant) = call(&app, Method::GET, &context_path, &token_a, None).await?;
        ensure!(context_with_grant["permissions"] == json!(["order:read"])
            && context_with_grant["dataGrants"][0]["scope"] == "ORG"
            && context_with_grant["contextVersion"] != empty_context["contextVersion"],
            "independent menu and data grant missing from context");
        let (_, host_menus) = call(&app, Method::GET,
            &format!("/api/auth/menus?applicationId={app_a}"), &session_a, None).await?;
        ensure!(host_menus["menus"] == context_with_grant["menus"],
            "host menus and app context diverged");
        let (_, external_user_menus) = call(&app, Method::GET, "/api/external/menus", &token_a, None).await?;
        ensure!(external_user_menus["menus"] == context_with_grant["menus"],
            "external menu path exposed full application configuration");
        ensure!(external_user_menus["menus"].as_array().is_some_and(|v| v.len() == 1),
            "external menu path included an ungranted application menu");
        let (_, external_user_roles) = call(&app, Method::GET, "/api/external/roles", &token_a, None).await?;
        ensure!(external_user_roles["organizations"].as_array().is_some_and(|v| v.len() == 1)
            && external_user_roles["organizations"][0]["id"] == org_a.to_string()
            && external_user_roles["organizations"][0]["roles"][0]["id"] == role_id.to_string(),
            "external roles path exposed other organizations or missed current role");
        sqlx::query("DELETE FROM role_menus WHERE role_id = $1 AND menu_id = $2")
            .bind(role_id).bind(menu_id).execute(&pool).await?;
        let (_, context_without_role) = call(&app, Method::GET, &context_path, &token_a, None).await?;
        ensure!(context_without_role["permissions"].as_array().is_some_and(Vec::is_empty)
            && context_without_role["dataGrants"][0]["scope"] == "ORG"
            && context_without_role["contextVersion"] != context_with_grant["contextVersion"],
            "role change altered independent data grant or failed to invalidate context");
        let (_, peer_context) = call(&peer, Method::GET, &context_path, &token_a, None).await?;
        ensure!(peer_context["permissions"] == context_without_role["permissions"]
            && peer_context["contextVersion"] == context_without_role["contextVersion"],
            "second backend instance retained stale permissions");
        sqlx::query("DELETE FROM membership_data_grants WHERE membership_id = $1 AND application_id = $2")
            .bind(member_id).bind(app_a).execute(&pool).await?;
        let (_, context_without_grant) = call(&app, Method::GET, &context_path, &token_a, None).await?;
        ensure!(context_without_grant["dataGrants"].as_array().is_some_and(Vec::is_empty),
            "data grant deletion was not visible on next request");

        let child_org: Uuid = sqlx::query_scalar(
            "INSERT INTO organizations (tenant_id, parent_id, name) \
             VALUES ($1, $2, $3) RETURNING id",
        ).bind(tenant_a).bind(org_a).bind(format!("child-{suffix}"))
         .fetch_one(&pool).await?;
        let descendant: Uuid = sqlx::query_scalar(
            "INSERT INTO organizations (tenant_id, parent_id, name) \
             VALUES ($1, $2, $3) RETURNING id",
        ).bind(tenant_a).bind(child_org).bind(format!("descendant-{suffix}"))
         .fetch_one(&pool).await?;
        let child_member: Uuid = sqlx::query_scalar(
            "INSERT INTO memberships (user_id, organization_id) VALUES ($1, $2) RETURNING id",
        ).bind(user_a).bind(child_org).fetch_one(&pool).await?;
        sqlx::query("INSERT INTO organization_applications (organization_id, application_id) VALUES ($1, $2)")
            .bind(child_org).bind(app_a).execute(&pool).await?;
        sqlx::query("INSERT INTO organization_menus (organization_id, menu_id) VALUES ($1, $2)")
            .bind(child_org).bind(menu_id).execute(&pool).await?;
        sqlx::query("INSERT INTO role_menus (role_id, menu_id) VALUES ($1, $2)")
            .bind(role_id).bind(menu_id).execute(&pool).await?;
        let (child_session, _) = session(&pool, user_a, tenant_a, child_org, secret).await?;
        let (_, child_issue) = call(&app, Method::POST, "/api/oauth/session-token", &child_session,
            Some(json!({"appCode":code_a}))).await?;
        let child_token = child_issue["accessToken"].as_str()
            .ok_or_else(|| anyhow::anyhow!("missing child app token"))?.to_string();
        let (_, inherited) = call(&app, Method::GET, &context_path, &child_token, None).await?;
        ensure!(inherited["permissions"] == json!(["order:read"])
            && inherited["dataGrants"].as_array().is_some_and(Vec::is_empty),
            "ancestor role was not inherited or implicitly granted data access");
        sqlx::query(
            "INSERT INTO membership_data_grants \
             (membership_id, application_id, resource_code, action_code, scope_type) \
             VALUES ($1, $2, 'order', 'read', 'ORG_SUBTREE')",
        ).bind(child_member).bind(app_a).execute(&pool).await?;
        let (_, child_context) = call(&app, Method::GET, &context_path, &child_token, None).await?;
        let allowed_orgs = child_context["dataGrants"][0]["organizationIds"].as_array()
            .ok_or_else(|| anyhow::anyhow!("missing subtree organization IDs"))?;
        ensure!(allowed_orgs.contains(&json!(child_org)) && allowed_orgs.contains(&json!(descendant))
            && !allowed_orgs.contains(&json!(org_a)) && !allowed_orgs.contains(&json!(org_b)),
            "subtree grant leaked parent or other tenant organization");
        let (_, child_external_roles) = call(&app, Method::GET, "/api/external/roles", &child_token, None).await?;
        ensure!(child_external_roles["organizations"].as_array().is_some_and(|v| v.len() == 1)
            && child_external_roles["organizations"][0]["id"] == child_org.to_string(),
            "child external roles enumerated ancestor or unrelated organizations");

        let (status, legacy) = call_form(&app, "/oauth/token", &client_a, &secret_a,
            "grant_type=client_credentials").await?;
        ensure!(status == StatusCode::OK, "legacy client_credentials flow failed: {status} {legacy}");
        let legacy_token = legacy["access_token"].as_str().ok_or_else(|| anyhow::anyhow!("missing legacy token"))?;
        let (_, legacy_active) = call_form(&app, "/oauth/introspect", &client_a, &secret_a,
            &format!("token={legacy_token}")).await?;
        ensure!(legacy_active["active"] == true, "legacy OAuth token lost introspection compatibility");
        ensure!(call(&app, Method::GET, "/api/external/menus", legacy_token, None).await?.0
            == StatusCode::FORBIDDEN,
            "client_credentials token accessed user menu view");
        ensure!(call_form(&app, "/oauth/introspect", &client_b, &secret_b,
            &format!("token={legacy_token}")).await?.1["active"] == false,
            "B backend introspected A legacy OAuth token");
        let refresh_probe = format!("refreshprobe{}", Uuid::new_v4().simple());
        sqlx::query("INSERT INTO oauth_tokens (token_hash, token_type, user_id, client_id, expires_at) \
                     VALUES ($1, 'refresh', $2, $3, now() + interval '1 hour')")
            .bind(hash::sha256_hex(&refresh_probe)).bind(user_a).bind(&client_a)
            .execute(&pool).await?;
        ensure!(call_form(&app, "/oauth/introspect", &client_a, &secret_a,
            &format!("token={refresh_probe}")).await?.1["active"] == false,
            "refresh token was accepted as resource access token");

        call_form(&app, "/oauth/revoke", &client_a, &secret_a, &format!("token={token_a}")).await?;
        let (_, revoked) = call_form(&app, "/oauth/introspect", &client_a, &secret_a,
            &format!("token={token_a}")).await?;
        ensure!(revoked["active"] == false, "revoked app token remained active");
        ensure!(call(&app, Method::GET, "/api/external/menus", &token_a, None).await?.0 == StatusCode::UNAUTHORIZED,
            "external API accepted revoked app token");
        ensure!(call(&app, Method::GET, &context_path, &token_a, None).await?.0 == StatusCode::UNAUTHORIZED,
            "context API accepted revoked app token");

        sqlx::query("DELETE FROM organization_applications WHERE organization_id = $1 AND application_id = $2")
            .bind(org_a).bind(app_a).execute(&pool).await?;
        let (_, after_org_revoke) = call_form(&app, "/oauth/introspect", &client_a, &secret_a,
            &format!("token={token_a2}")).await?;
        ensure!(after_org_revoke["active"] == false, "organization grant removal did not revoke app token");
        ensure!(call(&app, Method::POST, "/api/oauth/session-token", &session_a,
            Some(json!({"appCode":code_a}))).await?.0 == StatusCode::NOT_FOUND,
            "app token issued without organization grant");
        sqlx::query("INSERT INTO organization_applications (organization_id, application_id) VALUES ($1, $2)")
            .bind(org_a).bind(app_a).execute(&pool).await?;
        ensure!(call_form(&app, "/oauth/introspect", &client_a, &secret_a,
            &format!("token={token_a2}")).await?.1["active"] == false,
            "regrant revived revoked app token");

        let (_, issued3) = call(&app, Method::POST, "/api/oauth/session-token", &session_a,
            Some(json!({"appCode":code_a}))).await?;
        let token_a3 = issued3["accessToken"].as_str().ok_or_else(|| anyhow::anyhow!("missing third token"))?.to_string();
        sqlx::query("UPDATE oauth_clients SET status = 'DISABLED' WHERE client_id = $1")
            .bind(&client_a).execute(&pool).await?;
        ensure!(call_form(&app, "/oauth/introspect", &client_b, &secret_b,
            &format!("token={token_a3}")).await?.1["active"] == false,
            "disabled client token remained active");
        ensure!(call(&app, Method::POST, "/api/oauth/session-token", &session_a,
            Some(json!({"appCode":code_a}))).await?.0 == StatusCode::NOT_FOUND,
            "disabled client issued app token");
        sqlx::query("UPDATE oauth_clients SET status = 'ENABLED' WHERE client_id = $1")
            .bind(&client_a).execute(&pool).await?;
        ensure!(call_form(&app, "/oauth/introspect", &client_a, &secret_a,
            &format!("token={token_a3}")).await?.1["active"] == false,
            "client reenable revived token");

        let (_, issued4) = call(&app, Method::POST, "/api/oauth/session-token", &session_a,
            Some(json!({"appCode":code_a}))).await?;
        let token_a4 = issued4["accessToken"].as_str().ok_or_else(|| anyhow::anyhow!("missing fourth token"))?.to_string();
        sqlx::query("UPDATE applications SET status = 'DISABLED' WHERE id = $1")
            .bind(app_a).execute(&pool).await?;
        ensure!(call_form(&app, "/oauth/introspect", &client_a, &secret_a,
            &format!("token={token_a4}")).await?.1["active"] == false,
            "disabled application token remained active");
        sqlx::query("UPDATE applications SET status = 'ENABLED' WHERE id = $1")
            .bind(app_a).execute(&pool).await?;
        ensure!(call_form(&app, "/oauth/introspect", &client_a, &secret_a,
            &format!("token={token_a4}")).await?.1["active"] == false,
            "application reenable revived token");

        let (_, issued5) = call(&app, Method::POST, "/api/oauth/session-token", &session_a,
            Some(json!({"appCode":code_a}))).await?;
        let token_a5 = issued5["accessToken"].as_str().ok_or_else(|| anyhow::anyhow!("missing fifth token"))?.to_string();
        sqlx::query("DELETE FROM tenant_applications WHERE tenant_id = $1 AND application_id = $2")
            .bind(tenant_a).bind(app_a).execute(&pool).await?;
        ensure!(call_form(&app, "/oauth/introspect", &client_a, &secret_a,
            &format!("token={token_a5}")).await?.1["active"] == false,
            "tenant grant removal did not revoke token");
        sqlx::query("INSERT INTO tenant_applications (tenant_id, application_id) VALUES ($1, $2)")
            .bind(tenant_a).bind(app_a).execute(&pool).await?;

        let (_, code_issue) = call(&app, Method::POST, "/api/oauth/session-token", &session_a,
            Some(json!({"appCode":code_a}))).await?;
        let code_token = code_issue["accessToken"].as_str().ok_or_else(|| anyhow::anyhow!("missing code-change token"))?.to_string();
        let changed_code = format!("x{}", &code_a[1..]);
        sqlx::query("UPDATE applications SET code = $1 WHERE id = $2")
            .bind(&changed_code).bind(app_a).execute(&pool).await?;
        sqlx::query("UPDATE applications SET code = $1 WHERE id = $2")
            .bind(&code_a).bind(app_a).execute(&pool).await?;
        ensure!(call_form(&app, "/oauth/introspect", &client_a, &secret_a,
            &format!("token={code_token}")).await?.1["active"] == false,
            "restoring application code revived token");

        let (_, config_issue) = call(&app, Method::POST, "/api/oauth/session-token", &session_a,
            Some(json!({"appCode":code_a}))).await?;
        let config_token = config_issue["accessToken"].as_str().ok_or_else(|| anyhow::anyhow!("missing config-change token"))?.to_string();
        sqlx::query("UPDATE oauth_clients SET access_token_validity = access_token_validity + 1 WHERE client_id = $1")
            .bind(&client_a).execute(&pool).await?;
        sqlx::query("UPDATE oauth_clients SET access_token_validity = access_token_validity - 1 WHERE client_id = $1")
            .bind(&client_a).execute(&pool).await?;
        ensure!(call_form(&app, "/oauth/introspect", &client_a, &secret_a,
            &format!("token={config_token}")).await?.1["active"] == false,
            "restoring client token configuration revived token");

        let (_, issued6) = call(&app, Method::POST, "/api/oauth/session-token", &session_a,
            Some(json!({"appCode":code_a}))).await?;
        let token_a6 = issued6["accessToken"].as_str().ok_or_else(|| anyhow::anyhow!("missing sixth token"))?.to_string();
        sqlx::query("UPDATE oauth_tokens SET expires_at = now() - interval '1 second' WHERE token_hash = $1")
            .bind(hash::sha256_hex(&token_a6)).execute(&pool).await?;
        ensure!(call_form(&app, "/oauth/introspect", &client_a, &secret_a,
            &format!("token={token_a6}")).await?.1["active"] == false,
            "expired app token remained active");

        let (_, issued7) = call(&app, Method::POST, "/api/oauth/session-token", &session_a,
            Some(json!({"appCode":code_a}))).await?;
        let token_a7 = issued7["accessToken"].as_str().ok_or_else(|| anyhow::anyhow!("missing seventh token"))?.to_string();
        ensure!(call(&app, Method::POST, "/api/auth/logout", &session_a, None).await?.0 == StatusCode::OK,
            "session logout failed");
        ensure!(call_form(&app, "/oauth/introspect", &client_a, &secret_a,
            &format!("token={token_a7}")).await?.1["active"] == false,
            "app token survived parent session logout");

        let (fresh_session, _) = session(&pool, user_a, tenant_a, org_a, secret).await?;
        let (_, issued8) = call(&app, Method::POST, "/api/oauth/session-token", &fresh_session,
            Some(json!({"appCode":code_a}))).await?;
        let token_a8 = issued8["accessToken"].as_str().ok_or_else(|| anyhow::anyhow!("missing eighth token"))?.to_string();
        sqlx::query("DELETE FROM memberships WHERE user_id = $1 AND organization_id = $2")
            .bind(user_a).bind(org_a).execute(&pool).await?;
        ensure!(call_form(&app, "/oauth/introspect", &client_a, &secret_a,
            &format!("token={token_a8}")).await?.1["active"] == false,
            "app token survived member removal");
        ensure!(call(&app, Method::GET, &context_path, &token_a8, None).await?.0 == StatusCode::UNAUTHORIZED,
            "context API accepted removed member");
        ensure!(call(&app, Method::POST, "/api/oauth/session-token", &fresh_session,
            Some(json!({"appCode":code_a}))).await?.0 == StatusCode::UNAUTHORIZED,
            "user without membership obtained app token");
        sqlx::query("INSERT INTO memberships (user_id, organization_id) VALUES ($1, $2)")
            .bind(user_a).bind(org_a).execute(&pool).await?;
        ensure!(call_form(&app, "/oauth/introspect", &client_a, &secret_a,
            &format!("token={token_a8}")).await?.1["active"] == false,
            "re-adding member revived app token");

        let audit_details: Vec<String> = sqlx::query_scalar(
            "SELECT detail FROM audit_logs WHERE user_id = $1 AND action = 'TOKEN_ISSUED'",
        ).bind(user_a).fetch_all(&pool).await?;
        ensure!(audit_details.iter().all(|line| !line.contains(&secret_a) && !line.contains(&secret_b)),
            "audit log disclosed client secret");
        Ok(())
    }.await;

    sqlx::query("DELETE FROM oauth_tokens WHERE client_id = ANY($1)")
        .bind(&[&client_a, &client_b]).execute(&pool).await?;
    sqlx::query("DELETE FROM audit_logs WHERE tenant_id = ANY($1) OR user_id = ANY($2) \
                 OR username = ANY($3)")
        .bind(&[tenant_a, tenant_b]).bind(&[user_a, user_b])
        .bind(&[&client_a, &client_b]).execute(&pool).await?;
    sqlx::query("DELETE FROM oauth_clients WHERE client_id = ANY($1)")
        .bind(&[&client_a, &client_b]).execute(&pool).await?;
    sqlx::query("DELETE FROM applications WHERE id = ANY($1)")
        .bind(&[app_a, app_b]).execute(&pool).await?;
    sqlx::query("DELETE FROM tenants WHERE id = ANY($1)")
        .bind(&[tenant_a, tenant_b]).execute(&pool).await?;
    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(&[user_a, user_b]).execute(&pool).await?;
    result
}
