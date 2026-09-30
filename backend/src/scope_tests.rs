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

fn session(user: Uuid, tenant: Uuid, org: Uuid, secret: &str) -> Result<String> {
    let now = chrono::Utc::now().timestamp();
    jwt::sign(
        &Claims {
            sub: user.to_string(),
            typ: "session".into(),
            tenant: Some(tenant.to_string()),
            org: Some(org.to_string()),
            cid: None,
            scope: None,
            iat: now,
            exp: now + 3600,
        },
        secret,
    )
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
    let a = session(admin_a, tenant_a, org_a, secret)?;
    let b = session(admin_b, tenant_b, org_b, secret)?;
    let platform = session(platform_user, platform_tenant, platform_org, secret)?;
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
    result
}
