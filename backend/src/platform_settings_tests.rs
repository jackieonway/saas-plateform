//! Platform branding and tenant identity regressions on an owned PostgreSQL database.
//! Run with `cargo test platform_settings_tests -- --ignored --nocapture`.
//! Test credentials come from TEST_DATABASE_URL or ignored root .env.test, never logs.
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
    async fn create() -> Result<Self> {
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
        let migrator = sqlx::migrate!("./migrations");
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
        "platformTenantName": "星河平台管理租户",
        "primaryColor": "#2563EB"
    })
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn branding_persists_and_renamed_tenant_refreshes_existing_profiles() -> Result<()> {
    let database = Database::create().await?;
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
            "primaryColor",
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
    let database = Database::create().await?;
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
            ("primaryColor", json!("#fff")),
            ("primaryColor", json!("red")),
            ("primaryColor", json!("#123456;--primary:red")),
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
async fn theme_color_is_public_normalized_persisted_and_legacy_updates_preserve_it() -> Result<()> {
    let database = Database::create().await?;
    let result = async {
        let h = Harness::create(&database.pool).await?;
        let initial = h.settings().await?;
        ensure!(
            initial["primaryColor"] == "#008577",
            "default theme differs"
        );
        let mut update = updated_settings();
        update["primaryColor"] = json!("  #b4530a  ");
        let saved = ok(
            &h.app,
            Method::PUT,
            "/api/platform/settings",
            &h.admin_token,
            Some(update),
        )
        .await?;
        ensure!(
            saved["primaryColor"] == "#B4530A",
            "color was not normalized"
        );
        let second = router(&database.pool)?;
        let branding = ok(&second, Method::GET, "/api/platform/branding", "", None).await?;
        ensure!(
            branding["primaryColor"] == "#B4530A",
            "anonymous pages did not receive the committed theme"
        );
        let mut legacy = updated_settings();
        legacy
            .as_object_mut()
            .ok_or_else(|| anyhow!("invalid fixture"))?
            .remove("primaryColor");
        let legacy_saved = ok(
            &h.app,
            Method::PUT,
            "/api/platform/settings",
            &h.admin_token,
            Some(legacy),
        )
        .await?;
        ensure!(
            legacy_saved["primaryColor"] == "#B4530A",
            "legacy update reset the theme"
        );
        let persisted: String =
            sqlx::query_scalar("SELECT primary_color FROM platform_settings WHERE singleton")
                .fetch_one(&database.pool)
                .await?;
        ensure!(persisted == "#B4530A", "theme was not stored in PostgreSQL");
        let unsafe_write =
            sqlx::query("UPDATE platform_settings SET primary_color='red' WHERE singleton")
                .execute(&database.pool)
                .await;
        ensure!(
            unsafe_write.is_err(),
            "database accepted an unsafe color value"
        );
        Ok(())
    }
    .await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn initial_schema_creates_generic_branding_and_preserves_settings_on_restart() -> Result<()> {
    let database = Database::create().await?;
    let result = async {
        let defaults: (String, String, String) = sqlx::query_as(
            "SELECT platform_name,footer_text,primary_color FROM platform_settings WHERE singleton",
        )
        .fetch_one(&database.pool)
        .await?;
        ensure!(
            defaults
                == (
                    "SaaS Platform".into(),
                    "© 2026 SaaS Platform · Powered by Rust & React".into(),
                    "#008577".into()
                ),
            "initial branding defaults differ"
        );
        let duplicate = sqlx::query("INSERT INTO platform_settings(singleton) VALUES(true)")
            .execute(&database.pool)
            .await;
        ensure!(duplicate.is_err(), "branding singleton constraint is missing");
        sqlx::query("UPDATE platform_settings SET platform_name='自定义平台'")
            .execute(&database.pool)
            .await?;
        let baseline_checksum: Vec<u8> = sqlx::query_scalar(
            "SELECT checksum FROM _sqlx_migrations WHERE version=1",
        ).fetch_one(&database.pool).await?;
        let migrator = sqlx::migrate!("./migrations");
        migrator.run(&database.pool).await?;
        // Exercise the startup menu repair with an actual SQL NULL parent, while
        // skipping the unrelated full catalog COPY in this restart regression.
        let system: Uuid = sqlx::query_scalar(
            "INSERT INTO applications(code,name) VALUES('system','System') RETURNING id",
        ).fetch_one(&database.pool).await?;
        sqlx::query("INSERT INTO menus(application_id,parent_id,title,path) VALUES($1,NULL,'Menus','/system/menus')")
            .bind(system).execute(&database.pool).await?;
        sqlx::query("INSERT INTO region_versions(version,label,imported) VALUES('2023','2023版',true)")
            .execute(&database.pool).await?;
        crate::region_seed::initialize(&database.pool).await?;
        let first: (Uuid, Option<Uuid>) = sqlx::query_as(
            "SELECT id,parent_id FROM menus WHERE application_id=$1 AND path='/system/regions'",
        ).bind(system).fetch_one(&database.pool).await?;
        ensure!(first.1.is_none(), "startup changed the root menu's NULL parent");
        crate::region_seed::initialize(&database.pool).await?;
        let after: Vec<(Uuid, Option<Uuid>)> = sqlx::query_as(
            "SELECT id,parent_id FROM menus WHERE application_id=$1 AND path='/system/regions'",
        ).bind(system).fetch_all(&database.pool).await?;
        ensure!(after == vec![first], "restart duplicated or moved the repaired region menu");
        let region_count: i64 = sqlx::query_scalar("SELECT count(*) FROM regions")
            .fetch_one(&database.pool).await?;
        ensure!(region_count == 0, "restart ignored the imported catalog marker");
        let name: String =
            sqlx::query_scalar("SELECT platform_name FROM platform_settings WHERE singleton")
                .fetch_one(&database.pool)
                .await?;
        ensure!(name == "自定义平台", "restart replaced custom branding");
        let versions: Vec<i64> =
            sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
                .fetch_all(&database.pool)
                .await?;
        let expected_versions: Vec<i64> = migrator.iter().map(|migration| migration.version).collect();
        ensure!(expected_versions.first() == Some(&1) && versions == expected_versions,
            "initialization did not apply the unified baseline and exact embedded migrations");
        let after_checksum: Vec<u8> = sqlx::query_scalar(
            "SELECT checksum FROM _sqlx_migrations WHERE version=1",
        ).fetch_one(&database.pool).await?;
        let embedded_baseline = migrator.iter().find(|migration| migration.version == 1)
            .ok_or_else(|| anyhow!("embedded unified baseline is missing"))?;
        ensure!(baseline_checksum == after_checksum && baseline_checksum.as_slice() == embedded_baseline.checksum.as_ref(),
            "restart changed the unified baseline checksum");
        Ok(())
    }
    .await;
    database.finish(result).await
}
