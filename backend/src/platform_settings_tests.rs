//! Platform branding and tenant identity regressions on an owned PostgreSQL database.
//! Run with `cargo test platform_settings_tests -- --ignored --nocapture`.
//! Test credentials come from TEST_DATABASE_URL or ignored root .env.test, never logs.
use std::borrow::Cow;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
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

const SECRET: &str = "platform-settings-isolated-tests-only";

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

fn database_error(stage: &str, error: &sqlx::Error) -> anyhow::Error {
    match error.as_database_error().and_then(|error| error.code()) {
        Some(code) => anyhow!("{stage} failed (SQLSTATE {code})"),
        None => anyhow!("{stage} failed; check isolated database connectivity and privileges"),
    }
}

struct Database {
    pool: PgPool,
    admin: PgPool,
    name: String,
}

impl Database {
    async fn create(before_settings_migration: bool) -> Result<Self> {
        let options = database_options()?;
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
            .map_err(|error| database_error("administration connection", &error))?;
        let name = format!("platform_settings_test_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await
            .map_err(|error| database_error("creating isolated test database", &error))?;
        let pool = match PgPoolOptions::new()
            .max_connections(6)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.database(&name))
            .await
        {
            Ok(pool) => pool,
            Err(error) => {
                let _ = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
                    .execute(&admin)
                    .await;
                return Err(database_error("isolated test connection", &error));
            }
        };
        let database = Self { pool, admin, name };
        let mut migrator = sqlx::migrate!("./migrations");
        if before_settings_migration {
            migrator.migrations = Cow::Owned(
                migrator
                    .iter()
                    .filter(|m| m.version < 32)
                    .cloned()
                    .collect(),
            );
        }
        if migrator.run(&database.pool).await.is_err() {
            let _ = database
                .finish(Err(anyhow!("isolated test migrations failed")))
                .await;
            return Err(anyhow!("isolated test migrations failed"));
        }
        Ok(database)
    }

    async fn finish(self, assertions: Result<()>) -> Result<()> {
        ensure!(
            self.name.starts_with("platform_settings_test_")
                && self
                    .name
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_'),
            "refusing cleanup outside the owned test database"
        );
        let initially_closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let cleanup = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await
            .map_err(|error| database_error("removing isolated test database", &error));
        let finally_closed = initially_closed
            || tokio::time::timeout(Duration::from_secs(5), self.pool.close())
                .await
                .is_ok();
        let _ = tokio::time::timeout(Duration::from_secs(5), self.admin.close()).await;
        eprintln!(
            "platform settings cleanup assertions_ok={} database_removed={} pool_closed={}",
            assertions.is_ok(),
            cleanup.is_ok(),
            finally_closed
        );
        assertions?;
        cleanup?;
        ensure!(finally_closed, "isolated test pool did not close");
        Ok(())
    }
}

async fn migrate_with_timeout(pool: &PgPool) -> Result<()> {
    let mut connection = pool.acquire().await?;
    let migrator = sqlx::migrate!("./migrations");
    let migration = tokio::time::timeout(
        Duration::from_secs(30),
        migrator.run_direct(&mut *connection),
    )
    .await;
    // SQLx 0.8 retains its session advisory lock when run_direct returns an error.
    // Release it on this same owned connection before returning it to the pool.
    let unlock = tokio::time::timeout(
        Duration::from_secs(5),
        sqlx::query("SELECT pg_advisory_unlock_all()").execute(&mut *connection),
    )
    .await;
    drop(connection);
    unlock.map_err(|_| anyhow!("isolated migration advisory lock cleanup timed out"))??;
    migration.map_err(|_| anyhow!("isolated settings migration timed out"))??;
    Ok(())
}

async fn call(
    app: &Router,
    method: Method,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> Result<(StatusCode, Value)> {
    let mut request = Request::builder().method(method.clone()).uri(path);
    if !token.is_empty() {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let bytes = match body {
        Some(body) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            serde_json::to_vec(&body)?
        }
        None => Vec::new(),
    };
    let response = tokio::time::timeout(
        Duration::from_secs(30),
        app.clone().oneshot(request.body(Body::from(bytes))?),
    )
    .await
    .map_err(|_| anyhow!("platform settings API timeout: {method} {path}"))??;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

async fn ok(
    app: &Router,
    method: Method,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> Result<Value> {
    let (status, value) = call(app, method.clone(), path, token, body).await?;
    ensure!(
        status == StatusCode::OK,
        "{method} {path} returned {status}"
    );
    Ok(value)
}

fn router(pool: &PgPool) -> Result<Router> {
    let values = HashMap::from([
        ("APPLICATION_ENVIRONMENT".into(), "development".into()),
        (
            "DATABASE_URL".into(),
            "postgres://unused/platform-settings-test".into(),
        ),
        ("JWT_SECRET".into(), SECRET.into()),
    ]);
    let config = Config::from_values(&values)?;
    Ok(routes::api_router().with_state(AppState::new(pool.clone(), Arc::new(config))))
}

async fn identity(pool: &PgPool, tenant: Uuid, name: &str) -> Result<(Uuid, Uuid, Uuid)> {
    let root: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM organizations WHERE tenant_id=$1 AND parent_id IS NULL")
            .bind(tenant)
            .fetch_optional(pool)
            .await?;
    let org: Uuid = sqlx::query_scalar(
        "INSERT INTO organizations(tenant_id,name,parent_id) VALUES($1,$2,$3) RETURNING id",
    )
    .bind(tenant)
    .bind(format!("{name} 组织"))
    .bind(root)
    .fetch_one(pool)
    .await?;
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users(username,password_hash) VALUES($1,'isolated-unused-password') RETURNING id",
    ).bind(name).fetch_one(pool).await?;
    let membership: Uuid = sqlx::query_scalar(
        "INSERT INTO memberships(user_id,organization_id) VALUES($1,$2) RETURNING id",
    )
    .bind(user)
    .bind(org)
    .fetch_one(pool)
    .await?;
    Ok((org, user, membership))
}

async fn session(pool: &PgPool, tenant: Uuid, org: Uuid, user: Uuid) -> Result<String> {
    let version: i64 = sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
        .bind(user)
        .fetch_one(pool)
        .await?;
    let id: Uuid = sqlx::query_scalar(
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
            sid: Some(id.to_string()),
            ver: Some(version),
            aud: None,
            jti: None,
            iat: now,
            exp: now + 3600,
        },
        SECRET,
    )?)
}

async fn grant(
    pool: &PgPool,
    tenant: Uuid,
    org: Uuid,
    user: Uuid,
    app: Uuid,
    menus: &[Uuid],
) -> Result<()> {
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
    let role: Uuid = sqlx::query_scalar(
        "INSERT INTO roles(tenant_id,organization_id,name) VALUES($1,$2,'settings-test-role') RETURNING id",
    ).bind(tenant).bind(org).fetch_one(pool).await?;
    sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3)")
        .bind(user)
        .bind(role)
        .bind(org)
        .execute(pool)
        .await?;
    for menu in menus {
        sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
            .bind(tenant)
            .bind(menu)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
            .bind(org)
            .bind(menu)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)")
            .bind(role)
            .bind(menu)
            .execute(pool)
            .await?;
    }
    Ok(())
}

struct Harness {
    app: Router,
    platform: Uuid,
    organization: Uuid,
    membership: Uuid,
    other_tenant: Uuid,
    admin_token: String,
    viewer_token: String,
    other_token: String,
}

impl Harness {
    async fn create(pool: &PgPool) -> Result<Self> {
        let platform: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(name,code,is_platform) VALUES('初始平台租户','admin-tenant',true) RETURNING id",
        ).fetch_one(pool).await?;
        let other_tenant: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(name,code) VALUES('普通租户','settings-other-tenant') RETURNING id",
        ).fetch_one(pool).await?;
        let (organization, user, membership) = identity(pool, platform, "settings-admin").await?;
        let (viewer_org, viewer, _) = identity(pool, platform, "settings-viewer").await?;
        let (other_org, other, _) = identity(pool, other_tenant, "settings-other").await?;
        let application: Uuid = sqlx::query_scalar(
            "INSERT INTO applications(name,code) VALUES('设置测试','settings-test-app') RETURNING id",
        ).fetch_one(pool).await?;
        let mut menus = Vec::new();
        for permission in [
            "system:settings",
            "system:tenant",
            "system:monitor",
            "system:user",
        ] {
            menus.push(sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO menus(application_id,title,permission,type) VALUES($1,$2,$2,'BUTTON') RETURNING id",
            ).bind(application).bind(permission).fetch_one(pool).await?);
        }
        grant(pool, platform, organization, user, application, &menus).await?;
        grant(pool, other_tenant, other_org, other, application, &menus).await?;
        Ok(Self {
            app: router(pool)?,
            platform,
            organization,
            membership,
            other_tenant,
            admin_token: session(pool, platform, organization, user).await?,
            viewer_token: session(pool, platform, viewer_org, viewer).await?,
            other_token: session(pool, other_tenant, other_org, other).await?,
        })
    }

    async fn settings(&self) -> Result<Value> {
        ok(
            &self.app,
            Method::GET,
            "/api/platform/settings",
            &self.admin_token,
            None,
        )
        .await
    }
}

fn updated_settings() -> Value {
    json!({
        "platformName": "星河管理平台",
        "platformDescription": "租户与应用统一管理",
        "logoUrl": "/branding/platform-logo.svg",
        "loginTitle": "欢迎登录星河",
        "loginDescription": "使用你的企业账号登录",
        "footerText": "© 2026 星河平台",
        "platformTenantName": "星河平台管理租户"
    })
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn branding_persists_and_renamed_tenant_refreshes_existing_profiles() -> Result<()> {
    let database = Database::create(false).await?;
    let result = async {
        let h = Harness::create(&database.pool).await?;
        let initial = h.settings().await?;
        ensure!(
            initial["platformTenantCode"] == "admin-tenant",
            "platform code differs"
        );
        ensure!(
            initial["platformTenantName"] == "初始平台租户",
            "tenant name is not live data"
        );
        let expected = updated_settings();
        let saved = ok(
            &h.app,
            Method::PUT,
            "/api/platform/settings",
            &h.admin_token,
            Some(expected.clone()),
        )
        .await?;
        for (key, value) in expected
            .as_object()
            .ok_or_else(|| anyhow!("invalid fixture"))?
        {
            ensure!(saved[key] == *value, "saved setting differs: {key}");
        }
        ensure!(
            saved["platformTenantCode"] == "admin-tenant",
            "save changed platform code"
        );

        // A separately constructed application instance must immediately see the committed configuration.
        let second = router(&database.pool)?;
        let persisted = ok(
            &second,
            Method::GET,
            "/api/platform/settings",
            &h.admin_token,
            None,
        )
        .await?;
        ensure!(persisted == saved, "settings were cached or not committed");
        let branding = ok(&second, Method::GET, "/api/platform/branding", "", None).await?;
        let fields = [
            "platformName",
            "platformDescription",
            "logoUrl",
            "loginTitle",
            "loginDescription",
            "footerText",
        ];
        ensure!(
            branding
                .as_object()
                .is_some_and(|v| v.len() == fields.len()),
            "anonymous branding contains extra data"
        );
        for key in fields {
            ensure!(
                branding[key] == saved[key],
                "public branding differs: {key}"
            );
        }
        let monitor = ok(
            &second,
            Method::GET,
            "/api/monitor/system",
            &h.admin_token,
            None,
        )
        .await?;
        ensure!(
            monitor["service"]["name"] == expected["platformName"],
            "monitor did not use the configured platform name"
        );
        let new_user = ok(
            &h.app,
            Method::POST,
            "/api/users",
            &h.other_token,
            Some(json!({
                "username": "settings-welcome-user",
                "password": "Synthetic-welcome-password-7D!"
            })),
        )
        .await?;
        let new_user_id = new_user["id"]
            .as_str()
            .ok_or_else(|| anyhow!("created user ID is missing"))?
            .parse::<Uuid>()?;
        let welcome: String = sqlx::query_scalar(
            "SELECT title FROM notifications WHERE user_id=$1 AND tenant_id=$2 ORDER BY created_at LIMIT 1",
        )
        .bind(new_user_id)
        .bind(h.other_tenant)
        .fetch_one(&database.pool)
        .await?;
        ensure!(
            welcome == "欢迎加入 星河管理平台",
            "new user welcome did not use the configured platform name"
        );
        let profile = ok(
            &second,
            Method::GET,
            "/api/auth/profile",
            &h.admin_token,
            None,
        )
        .await?;
        ensure!(
            profile["tenant"]["id"] == json!(h.platform),
            "rename replaced tenant identity"
        );
        ensure!(
            profile["tenant"]["name"] == expected["platformTenantName"],
            "existing session has stale tenant name"
        );
        ensure!(
            profile["tenant"]["code"] == "admin-tenant",
            "profile platform code differs"
        );
        ensure!(
            profile["organization"]["id"] == json!(h.organization),
            "rename replaced organization identity"
        );
        ensure!(
            profile["memberships"][0]["tenantName"] == expected["platformTenantName"],
            "membership name was not refreshed"
        );
        let membership: (Uuid, Uuid) =
            sqlx::query_as("SELECT id,organization_id FROM memberships WHERE id=$1")
                .bind(h.membership)
                .fetch_one(&database.pool)
                .await?;
        ensure!(
            membership == (h.membership, h.organization),
            "rename replaced membership identity"
        );

        ok(
            &h.app,
            Method::PUT,
            &format!("/api/tenants/{}", h.platform),
            &h.admin_token,
            Some(json!({"name":"租户管理页修改的名称","code":"admin-tenant"})),
        )
        .await?;
        ensure!(
            h.settings().await?["platformTenantName"] == "租户管理页修改的名称",
            "settings and tenant management disagree"
        );
        Ok(())
    }
    .await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn settings_require_platform_permission_and_reject_partial_invalid_updates() -> Result<()> {
    let database = Database::create(false).await?;
    let result = async {
        let h = Harness::create(&database.pool).await?;
        let before = h.settings().await?;
        for (token, expected_status) in [
            ("", StatusCode::UNAUTHORIZED),
            (h.viewer_token.as_str(), StatusCode::FORBIDDEN),
            (h.other_token.as_str(), StatusCode::FORBIDDEN),
        ] {
            for method in [Method::GET, Method::PUT] {
                let body = (method == Method::PUT).then(updated_settings);
                let (status, _) = call(
                    &h.app,
                    method.clone(),
                    "/api/platform/settings",
                    token,
                    body,
                )
                .await?;
                ensure!(
                    status == expected_status,
                    "settings {method} unauthorized status: {status}"
                );
            }
        }

        let mut invalid = Vec::new();
        for (field, value) in [
            ("platformName", json!("  ")),
            ("platformTenantName", json!("")),
            ("loginTitle", json!("名".repeat(101))),
            ("platformDescription", json!("含有\n换行")),
            ("logoUrl", json!("javascript:alert(1)")),
            ("logoUrl", json!("data:image/svg+xml,<svg></svg>")),
            ("logoUrl", json!("//external.example/logo.svg")),
            (
                "logoUrl",
                json!("https://user:secret@example.test/logo.svg"),
            ),
            ("logoUrl", json!("/branding/%2e%2e/logo.svg")),
            ("platformTenantCode", json!("changed-platform-code")),
        ] {
            let mut body = updated_settings();
            body[field] = value;
            invalid.push((field, body));
        }
        let mut missing = updated_settings();
        missing
            .as_object_mut()
            .ok_or_else(|| anyhow!("invalid fixture"))?
            .remove("platformTenantName");
        invalid.push(("missing platformTenantName", missing));
        for (field, body) in invalid {
            let (status, _) = call(
                &h.app,
                Method::PUT,
                "/api/platform/settings",
                &h.admin_token,
                Some(body),
            )
            .await?;
            ensure!(
                status == StatusCode::BAD_REQUEST,
                "invalid {field} returned {status}"
            );
            ensure!(
                h.settings().await? == before,
                "invalid {field} partially changed configuration or tenant name"
            );
        }
        for tenant in [h.platform, h.other_tenant] {
            let attempted_code = if tenant == h.platform {
                "replace-admin-code"
            } else {
                "admin-tenant"
            };
            let (status, _) = call(
                &h.app,
                Method::PUT,
                &format!("/api/tenants/{tenant}"),
                &h.admin_token,
                Some(json!({"name":"错误请求不应修改名称","code":attempted_code})),
            )
            .await?;
            ensure!(
                status == StatusCode::BAD_REQUEST,
                "tenant endpoint accepted reserved platform code change: {status}"
            );
        }
        ensure!(
            h.settings().await? == before,
            "tenant endpoint partially changed platform data"
        );
        let other: (String, String) = sqlx::query_as("SELECT name,code FROM tenants WHERE id=$1")
            .bind(h.other_tenant)
            .fetch_one(&database.pool)
            .await?;
        ensure!(
            other == ("普通租户".into(), "settings-other-tenant".into()),
            "reserved code rejection partially changed ordinary tenant"
        );
        Ok(())
    }
    .await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn settings_migration_keeps_legacy_platform_name_and_relationships() -> Result<()> {
    let database = Database::create(true).await?;
    let result = async {
        let platform: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(name,code,is_platform) VALUES('自定义旧平台名称','dante',true) RETURNING id",
        ).fetch_one(&database.pool).await?;
        let (org, user, membership) = identity(&database.pool, platform, "settings-migration-user").await?;
        sqlx::migrate!("./migrations").run(&database.pool).await?;
        let tenant: (Uuid, String, String, bool) = sqlx::query_as("SELECT id,name,code,is_platform FROM tenants WHERE id=$1")
            .bind(platform).fetch_one(&database.pool).await?;
        ensure!(tenant == (platform, "自定义旧平台名称".into(), "admin-tenant".into(), true), "migration replaced tenant or custom name");
        let linked: (Uuid, Uuid, Uuid, Uuid) = sqlx::query_as(
            "SELECT m.id,m.user_id,m.organization_id,o.tenant_id FROM memberships m JOIN organizations o ON o.id=m.organization_id WHERE m.id=$1",
        ).bind(membership).fetch_one(&database.pool).await?;
        ensure!(linked == (membership, user, org, platform), "migration changed tenant relationships");
        let app = router(&database.pool)?;
        let branding = ok(&app, Method::GET, "/api/platform/branding", "", None).await?;
        ensure!(branding["platformName"].as_str().is_some_and(|name| !name.is_empty()), "migration did not initialize branding");
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn settings_migration_blocks_code_collisions_and_converts_only_the_default_name() -> Result<()>
{
    let database = Database::create(true).await?;
    let result = async {
        let platform: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(name,code,is_platform) VALUES('Dante 云平台','dante',true) RETURNING id",
        )
        .fetch_one(&database.pool)
        .await?;
        let conflicting: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(name,code) VALUES('已有编码的普通租户','admin-tenant') RETURNING id",
        )
        .fetch_one(&database.pool)
        .await?;
        let (org, user, membership) =
            identity(&database.pool, platform, "settings-collision-user").await?;
        let migration_error = migrate_with_timeout(&database.pool)
            .await
            .err()
            .ok_or_else(|| anyhow!("migration accepted an existing reserved-code collision"))?;
        ensure!(
            migration_error.to_string().contains("PLATFORM_TENANT_CODE_CONFLICT"),
            "migration did not report the explicit platform code conflict"
        );
        let old_platform: (String, String) =
            sqlx::query_as("SELECT name,code FROM tenants WHERE id=$1")
                .bind(platform)
                .fetch_one(&database.pool)
                .await?;
        ensure!(
            old_platform == ("Dante 云平台".into(), "dante".into()),
            "failed migration partially modified the platform tenant"
        );
        let existing_code: String = sqlx::query_scalar("SELECT code FROM tenants WHERE id=$1")
            .bind(conflicting)
            .fetch_one(&database.pool)
            .await?;
        ensure!(
            existing_code == "admin-tenant",
            "failed migration reassigned the ordinary tenant code"
        );
        let settings_table: Option<String> =
            sqlx::query_scalar("SELECT to_regclass('public.platform_settings')::text")
                .fetch_one(&database.pool)
                .await?;
        ensure!(settings_table.is_none(), "failed migration partially installed settings");

        // Resolve only the synthetic conflicting tenant, then retry the same migration.
        sqlx::query("UPDATE tenants SET code='settings-resolved-tenant' WHERE id=$1")
            .bind(conflicting)
            .execute(&database.pool)
            .await?;
        migrate_with_timeout(&database.pool).await?;
        let new_platform: (Uuid, String, String) =
            sqlx::query_as("SELECT id,name,code FROM tenants WHERE id=$1")
                .bind(platform)
                .fetch_one(&database.pool)
                .await?;
        ensure!(
            new_platform == (platform, "平台管理租户".into(), "admin-tenant".into()),
            "successful migration did not convert the old default name in place"
        );
        let linked: (Uuid, Uuid, Uuid, Uuid) = sqlx::query_as(
            "SELECT m.id,m.user_id,m.organization_id,o.tenant_id FROM memberships m JOIN organizations o ON o.id=m.organization_id WHERE m.id=$1",
        )
        .bind(membership)
        .fetch_one(&database.pool)
        .await?;
        ensure!(
            linked == (membership, user, org, platform),
            "migration retry changed tenant relationships"
        );
        Ok(())
    }
    .await;
    database.finish(result).await
}
