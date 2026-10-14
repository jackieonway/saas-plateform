//! Tenant and organization creation regressions on an owned PostgreSQL database.
//! Run with `cargo test entity_creation_tests -- --ignored --nocapture`.
//! Credentials come from TEST_DATABASE_URL or ignored root .env.test, never logs.
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

const SECRET: &str = "entity-creation-isolated-tests-only";
const MANAGEMENT_PERMISSIONS: [&str; 6] = [
    "dashboard:view",
    "system:organization",
    "system:department",
    "system:authorization",
    "system:user",
    "system:role",
];

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

pub(crate) struct Database {
    pub(crate) pool: PgPool,
    admin: PgPool,
    name: String,
}

impl Database {
    pub(crate) async fn create() -> Result<Self> {
        let options = database_options()?;
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
            .map_err(|error| database_error("administration connection", &error))?;
        let name = format!("entity_creation_test_{}", Uuid::new_v4().simple());
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
        if !matches!(
            tokio::time::timeout(Duration::from_secs(30), migrator.run(&database.pool)).await,
            Ok(Ok(()))
        ) {
            return database
                .finish(Err(anyhow!("isolated test migrations failed or timed out")))
                .await
                .and_then(|_| Err(anyhow!("isolated test migrations failed")));
        }
        Ok(database)
    }

    pub(crate) async fn finish(self, assertions: Result<()>) -> Result<()> {
        ensure!(
            self.name.starts_with("entity_creation_test_")
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
            "entity creation cleanup assertions_ok={} database_removed={} pool_closed={}",
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

async fn request(
    app: &Router,
    token: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
    expected: StatusCode,
) -> Result<Value> {
    let mut request = Request::builder()
        .method(method.clone())
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"));
    let bytes = if let Some(body) = body {
        request = request.header(header::CONTENT_TYPE, "application/json");
        serde_json::to_vec(&body)?
    } else {
        Vec::new()
    };
    let response = tokio::time::timeout(
        Duration::from_secs(30),
        app.clone().oneshot(request.body(Body::from(bytes))?),
    )
    .await
    .map_err(|_| anyhow!("entity creation API timeout: {method} {path}"))??;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    let value: Value = serde_json::from_slice(&bytes)?;
    ensure!(
        status == expected,
        "{method} {path} returned {status}; expected {expected}: {value}"
    );
    Ok(value)
}

fn id(value: &Value) -> Result<Uuid> {
    value["id"]
        .as_str()
        .ok_or_else(|| anyhow!("API response lacks id"))?
        .parse()
        .map_err(Into::into)
}

async fn session(pool: &PgPool, tenant: Uuid, org: Uuid, user: Uuid) -> Result<String> {
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

struct Harness {
    app: Router,
    token: String,
    application: Uuid,
    extra_menu: Uuid,
}

impl Harness {
    async fn create(pool: &PgPool) -> Result<Self> {
        let platform: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(name,code,is_platform) VALUES('创建测试平台','admin-tenant',true) RETURNING id")
            .fetch_one(pool).await?;
        let org: Uuid = sqlx::query_scalar(
            "INSERT INTO organizations(tenant_id,name) VALUES($1,'创建测试平台') RETURNING id",
        )
        .bind(platform)
        .fetch_one(pool)
        .await?;
        let admin: Uuid = sqlx::query_scalar(
            "INSERT INTO users(username,password_hash,is_super_admin) \
             VALUES('entity-test-platform-admin','unused-test-hash',true) RETURNING id",
        )
        .fetch_one(pool)
        .await?;
        sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)")
            .bind(admin)
            .bind(org)
            .execute(pool)
            .await?;
        let application: Uuid = sqlx::query_scalar(
            "INSERT INTO applications(name,code) VALUES('系统管理','system') RETURNING id",
        )
        .fetch_one(pool)
        .await?;
        for permission in MANAGEMENT_PERMISSIONS.into_iter().chain(["system:tenant"]) {
            sqlx::query(
                "INSERT INTO menus(application_id,title,permission,type) VALUES($1,$2,$2,'BUTTON')",
            )
            .bind(application)
            .bind(permission)
            .execute(pool)
            .await?;
        }
        let extra_menu: Uuid = sqlx::query_scalar(
            "INSERT INTO menus(application_id,title,permission,type) \
             VALUES($1,'后续授权','entity:later','BUTTON') RETURNING id",
        )
        .bind(application)
        .fetch_one(pool)
        .await?;
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)").bind(platform).bind(application).execute(pool).await?;
        sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)").bind(org).bind(application).execute(pool).await?;
        sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) SELECT $1,id FROM menus WHERE application_id=$2").bind(platform).bind(application).execute(pool).await?;
        sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) SELECT $1,id FROM menus WHERE application_id=$2").bind(org).bind(application).execute(pool).await?;
        let config = Config::from_values(&HashMap::from([
            ("APPLICATION_ENVIRONMENT".into(), "development".into()),
            (
                "DATABASE_URL".into(),
                "postgres://unused/entity-creation-test".into(),
            ),
            ("JWT_SECRET".into(), SECRET.into()),
        ]))?;
        Ok(Self {
            app: routes::api_router().with_state(AppState::new(pool.clone(), Arc::new(config))),
            token: session(pool, platform, org, admin).await?,
            application,
            extra_menu,
        })
    }

    async fn ok(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        request(&self.app, &self.token, method, path, body, StatusCode::OK).await
    }

    async fn invalid(&self, method: Method, path: &str, body: Value) -> Result<()> {
        request(
            &self.app,
            &self.token,
            method,
            path,
            Some(body),
            StatusCode::BAD_REQUEST,
        )
        .await?;
        Ok(())
    }
}

async fn managed_role(pool: &PgPool, tenant: Uuid, org: Uuid, code: &str) -> Result<Uuid> {
    let roles: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM roles WHERE tenant_id=$1 AND organization_id=$2 AND code=$3 AND built_in",
    )
    .bind(tenant)
    .bind(org)
    .bind(code)
    .fetch_all(pool)
    .await?;
    ensure!(roles.len() == 1, "expected exactly one managed {code} role");
    Ok(roles[0])
}

async fn verify_unassigned_grants(
    pool: &PgPool,
    tenant: Uuid,
    org: Uuid,
    role: Uuid,
) -> Result<()> {
    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM memberships WHERE organization_id=$1), \
         (SELECT count(*) FROM user_roles WHERE role_id=$2), \
         (SELECT count(*) FROM organization_applications WHERE organization_id=$1), \
         (SELECT count(*) FROM role_menus WHERE role_id=$2)",
    )
    .bind(org)
    .bind(role)
    .fetch_one(pool)
    .await?;
    ensure!(
        counts == (0, 0, 1, 6),
        "administrator was assigned or management grants differ: {counts:?}"
    );
    let permissions: Vec<String> = sqlx::query_scalar(
        "SELECT m.permission FROM role_menus rm JOIN menus m ON m.id=rm.menu_id \
         JOIN organization_menus om ON om.menu_id=m.id AND om.organization_id=$1 \
         JOIN tenant_menus tm ON tm.menu_id=m.id AND tm.tenant_id=$2 WHERE rm.role_id=$3",
    )
    .bind(org)
    .bind(tenant)
    .bind(role)
    .fetch_all(pool)
    .await?;
    ensure!(
        permissions.len() == 6
            && MANAGEMENT_PERMISSIONS
                .iter()
                .all(|permission| permissions.iter().any(|actual| actual == permission)),
        "managed role lacks effective management menus"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn creation_without_administrator_preserves_grants_and_supports_later_assignment(
) -> Result<()> {
    let database = Database::create().await?;
    let result = async {
        let pool = &database.pool;
        let h = Harness::create(pool).await?;
        let tenant = h.ok(Method::POST, "/api/tenants", Some(json!({
            "name": "  独立维护租户  ", "code": "deferred-admin", "tenantType": "COMPANY",
            "province": "浙江省", "city": "杭州市", "district": "西湖区",
            "township": "转塘街道", "village": "龙王沙社区", "address": "测试地址",
            "longitude": 120.1, "latitude": 30.1,
        }))).await?;
        let tenant_id = id(&tenant)?;
        ensure!(tenant["organizationCount"] == 1 && tenant["userCount"] == 0
            && tenant["authorizedAppCount"] == 1 && tenant["tenantType"] == "COMPANY",
            "tenant response counts or type differ");
        let tree = h.ok(Method::GET, &format!("/api/organizations/tree?tenantId={tenant_id}"), None).await?;
        let root = &tree[0];
        let root_id = id(root)?;
        ensure!(tree.as_array().is_some_and(|roots| roots.len() == 1)
            && root["name"] == "独立维护租户" && root["organizationType"] == "COMPANY"
            && root["parentId"].is_null(), "root name, type, or uniqueness differs");
        for field in ["province", "city", "district", "township", "village", "address", "longitude", "latitude"] {
            ensure!(root[field] == tenant[field], "root location {field} does not inherit tenant");
        }
        let tenant_role = managed_role(pool, tenant_id, root_id, "tenant_admin").await?;
        verify_unassigned_grants(pool, tenant_id, root_id, tenant_role).await?;
        let child = h.ok(Method::POST, "/api/organizations", Some(json!({
            "tenantId": tenant_id, "parentId": root_id, "name": "分公司", "organizationType": "BRANCH",
        }))).await?;
        let child_id = id(&child)?;
        ensure!(child["organizationType"] == "BRANCH", "child type was not persisted");
        let child_role = managed_role(pool, tenant_id, child_id, "org_admin").await?;
        verify_unassigned_grants(pool, tenant_id, child_id, child_role).await?;
        let user_count: i64 = sqlx::query_scalar("SELECT count(*) FROM users").fetch_one(pool).await?;
        ensure!(user_count == 1, "creation unexpectedly provisioned an administrator account");

        // Authorizations still synchronize managed roles before any administrator is assigned.
        h.ok(Method::PUT, &format!("/api/tenants/{tenant_id}/authorizations"), Some(json!({
            "applications": [{"applicationId": h.application, "authorized": true}],
            "menus": [{"menuId": h.extra_menu, "authorized": true}],
        }))).await?;
        h.ok(Method::PUT, &format!("/api/organizations/{child_id}/authorizations"), Some(json!({
            "applications": [{"applicationId": h.application, "authorized": true}],
            "menus": [{"menuId": h.extra_menu, "authorized": true}],
        }))).await?;
        let synchronized: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM role_menus WHERE role_id=ANY($1) AND menu_id=$2")
            .bind([tenant_role, child_role].as_slice()).bind(h.extra_menu).fetch_one(pool).await?;
        ensure!(synchronized == 2, "later authorization did not synchronize both unassigned admin roles");

        let account = h.ok(Method::POST, "/api/users", Some(json!({
            "username": "deferred-administrator", "password": "Isolated-Test-Account-123!",
        }))).await?;
        let account_id = id(&account)?;
        h.invalid(Method::PUT, &format!("/api/users/{account_id}/roles"), json!({
            "items": [{"roleId": tenant_role, "organizationId": root_id}],
        })).await?;
        h.ok(Method::PUT, &format!("/api/users/{account_id}/memberships"), Some(json!({
            "items": [{"organizationId": root_id}, {"organizationId": child_id}],
        }))).await?;
        h.ok(Method::PUT, &format!("/api/roles/{tenant_role}/users"), Some(json!({
            "userIds": [account_id],
        }))).await?;
        h.ok(Method::PUT, &format!("/api/users/{account_id}/roles"), Some(json!({
            "items": [{"roleId": tenant_role, "organizationId": root_id},
                {"roleId": child_role, "organizationId": child_id}],
        }))).await?;
        let root_token = session(pool, tenant_id, root_id, account_id).await?;
        let child_token = session(pool, tenant_id, child_id, account_id).await?;
        for token in [&root_token, &child_token] {
            for path in [format!("/api/organizations/tree?tenantId={tenant_id}"),
                format!("/api/roles?tenantId={tenant_id}"), "/api/users".into()] {
                request(&h.app, token, Method::GET, &path, None, StatusCode::OK).await?;
            }
        }
        request(&h.app, &root_token, Method::POST, "/api/tenants", Some(json!({
            "name": "无权限新租户", "code": "forbidden-create",
        })), StatusCode::FORBIDDEN).await?;
        let listed = h.ok(Method::GET, "/api/tenants?keyword=deferred-admin", None).await?;
        ensure!(listed["records"][0]["userCount"] == 1,
            "later member assignment is not reflected in tenant user count");
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn entity_types_persist_update_and_reject_invalid_values_without_provisioning_users(
) -> Result<()> {
    let database = Database::create().await?;
    let result = async {
        let pool = &database.pool;
        let h = Harness::create(pool).await?;
        let mut last = None;
        for (tenant_type, root_type) in [("PERSONAL", "OTHER"), ("COMPANY", "COMPANY"),
            ("PUBLIC_INSTITUTION", "PUBLIC_INSTITUTION"), ("GOVERNMENT", "GOVERNMENT"), ("OTHER", "OTHER")] {
            let tenant = h.ok(Method::POST, "/api/tenants", Some(json!({
                "name": format!("类型测试 {tenant_type}"), "code": format!("type-{tenant_type}"),
                "tenantType": tenant_type, "rootOrganizationName": "忽略的根组织名称",
                "administrator": {"username": "ignored-legacy-admin", "password": "Ignored-Password-123!"},
            }))).await?;
            let tenant_id = id(&tenant)?;
            let persisted: String = sqlx::query_scalar("SELECT tenant_type FROM tenants WHERE id=$1")
                .bind(tenant_id).fetch_one(pool).await?;
            let tree = h.ok(Method::GET, &format!("/api/organizations/tree?tenantId={tenant_id}"), None).await?;
            ensure!(tenant["tenantType"] == tenant_type && persisted == tenant_type
                && tree[0]["organizationType"] == root_type && tree[0]["name"] == tenant["name"],
                "tenant type or automatic root mapping differs for {tenant_type}");
            last = Some((tenant_id, id(&tree[0])?));
        }
        let default = h.ok(Method::POST, "/api/tenants", Some(json!({
            "name": "缺省类型租户", "code": "default-type",
        }))).await?;
        ensure!(default["tenantType"] == "COMPANY", "omitted tenant type must default to COMPANY");
        let (tenant_id, root_id) = last.ok_or_else(|| anyhow!("type fixtures are missing"))?;
        for organization_type in ["COMPANY", "SUBSIDIARY", "BRANCH", "PUBLIC_INSTITUTION", "GOVERNMENT", "OTHER"] {
            let org = h.ok(Method::POST, "/api/organizations", Some(json!({
                "tenantId": tenant_id, "parentId": root_id, "name": format!("类型组织 {organization_type}"),
                "organizationType": organization_type,
                "administrator": {"username": "ignored-org-admin", "password": "Ignored-Password-123!"},
            }))).await?;
            let persisted: String = sqlx::query_scalar("SELECT organization_type FROM organizations WHERE id=$1")
                .bind(id(&org)?).fetch_one(pool).await?;
            ensure!(org["organizationType"] == organization_type && persisted == organization_type,
                "organization type did not persist for {organization_type}");
        }
        let default_org = h.ok(Method::POST, "/api/organizations", Some(json!({
            "tenantId": tenant_id, "parentId": root_id, "name": "缺省类型组织",
        }))).await?;
        ensure!(default_org["organizationType"] == "COMPANY", "omitted organization type must default to COMPANY");
        let org_id = id(&default_org)?;
        let updated = h.ok(Method::PUT, &format!("/api/tenants/{tenant_id}"), Some(json!({
            "tenantType": "GOVERNMENT",
        }))).await?;
        let updated_org = h.ok(Method::PUT, &format!("/api/organizations/{org_id}"), Some(json!({
            "organizationType": "SUBSIDIARY",
        }))).await?;
        ensure!(updated["tenantType"] == "GOVERNMENT" && updated_org["organizationType"] == "SUBSIDIARY",
            "entity type update response differs");
        let unchanged = h.ok(Method::PUT, &format!("/api/tenants/{tenant_id}"), Some(json!({
            "description": "保留类型",
        }))).await?;
        let unchanged_org = h.ok(Method::PUT, &format!("/api/organizations/{org_id}"), Some(json!({
            "description": "保留类型",
        }))).await?;
        ensure!(unchanged["tenantType"] == "GOVERNMENT" && unchanged_org["organizationType"] == "SUBSIDIARY",
            "omitted type update reset an existing value");
        for invalid in ["", "HEADQUARTERS", "INVALID"] {
            h.invalid(Method::POST, "/api/tenants", json!({
                "name": "错误类型", "code": "invalid-type", "tenantType": invalid,
            })).await?;
            h.invalid(Method::PUT, &format!("/api/tenants/{tenant_id}"), json!({"tenantType": invalid})).await?;
            h.invalid(Method::POST, "/api/organizations", json!({
                "tenantId": tenant_id, "parentId": root_id, "name": "错误类型", "organizationType": invalid,
            })).await?;
            h.invalid(Method::PUT, &format!("/api/organizations/{org_id}"), json!({"organizationType": invalid})).await?;
        }
        let counts: (i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM users), \
             (SELECT count(*) FROM memberships), (SELECT count(*) FROM user_roles)")
            .fetch_one(pool).await?;
        ensure!(counts == (1, 1, 0), "legacy administrator payload provisioned account/member/role assignments: {counts:?}");
        let invalid_rows: i64 = sqlx::query_scalar(
            "SELECT (SELECT count(*) FROM tenants WHERE code='invalid-type') + \
             (SELECT count(*) FROM organizations WHERE name='错误类型')")
            .fetch_one(pool).await?;
        ensure!(invalid_rows == 0, "invalid type request persisted partial data");
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn initial_schema_entity_types_default_and_enforce_database_constraints() -> Result<()> {
    let database = Database::create().await?;
    let result = async {
        let pool = &database.pool;
        let tenant: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(name,code) VALUES('历史租户','legacy-types') RETURNING id")
            .fetch_one(pool).await?;
        let org: Uuid = sqlx::query_scalar(
            "INSERT INTO organizations(tenant_id,name) VALUES($1,'历史根组织') RETURNING id")
            .bind(tenant).fetch_one(pool).await?;
        let defaults: (String, String) = sqlx::query_as(
            "SELECT t.tenant_type,o.organization_type FROM tenants t \
             JOIN organizations o ON o.tenant_id=t.id WHERE t.id=$1 AND o.id=$2")
            .bind(tenant).bind(org).fetch_one(pool).await?;
        ensure!(defaults == ("COMPANY".into(), "COMPANY".into()),
            "existing tenant and organization were not classified as COMPANY");
        for statement in ["UPDATE tenants SET tenant_type='BRANCH' WHERE id=$1",
            "UPDATE organizations SET organization_type='PERSONAL' WHERE id=$1"] {
            let result = sqlx::query(statement).bind(if statement.starts_with("UPDATE tenants") { tenant } else { org })
                .execute(pool).await;
            ensure!(matches!(&result, Err(error) if error.as_database_error()
                .and_then(|error| error.code()).as_deref() == Some("23514")),
                "database accepted an invalid entity type");
        }
        let new_tenant: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(name,code) VALUES('历史客户端新租户','legacy-new-types') RETURNING id")
            .fetch_one(pool).await?;
        let new_defaults: (String, String) = sqlx::query_as(
            "INSERT INTO organizations(tenant_id,name) VALUES($1,'历史客户端新组织') \
             RETURNING (SELECT tenant_type FROM tenants WHERE id=$1), organization_type")
            .bind(new_tenant).fetch_one(pool).await?;
        ensure!(new_defaults == ("COMPANY".into(), "COMPANY".into()),
            "inserts omitting entity types do not retain database defaults");
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn contacts_initialize_administrators_and_support_telephone_and_email_login() -> Result<()> {
    let database = Database::create().await?;
    let result = async {
        let pool = &database.pool;
        let h = Harness::create(pool).await?;
        let tenant = h.ok(Method::POST, "/api/tenants", Some(json!({
            "name": "联系人租户", "code": "contact-tenant", "contactName": " 联系人甲 ",
            "contactPhone": "13800138001", "contactEmail": " Contact@Example.com ",
        }))).await?;
        let tenant_id = id(&tenant)?;
        ensure!(tenant["contactName"] == "联系人甲" && tenant["contactEmail"] == "contact@example.com"
            && tenant["userCount"] == 1, "contact information or first user count differs");
        let administrator = &tenant["administrator"];
        ensure!(administrator["roleName"] == "超级管理员", "tenant administrator label differs");
        let user_id: Uuid = administrator["userId"].as_str().ok_or_else(|| anyhow!("administrator id missing"))?.parse()?;
        let secret = administrator["initialPassword"].as_str().ok_or_else(|| anyhow!("initial credential missing"))?;
        ensure!(administrator["username"] == "13800138001" && administrator["created"] == true
            && secret.len() >= 12, "telephone priority or initial credential differs");
        let user: (String, String, String, String) = sqlx::query_as(
            "SELECT real_name,phone,email,password_hash FROM users WHERE id=$1")
            .bind(user_id).fetch_one(pool).await?;
        ensure!(user.0 == "联系人甲" && user.1 == "13800138001" && user.2 == "contact@example.com"
            && user.3 != secret && crate::util::password::verify_password(&user.3, secret), "initial user or password hash differs");
        let tree = h.ok(Method::GET, &format!("/api/organizations/tree?tenantId={tenant_id}"), None).await?;
        let root_id = id(&tree[0])?;
        ensure!(tree[0]["contactName"] == tenant["contactName"] && tree[0]["contactPhone"] == tenant["contactPhone"]
            && tree[0]["contactEmail"] == tenant["contactEmail"], "root did not inherit contact information");
        let role = managed_role(pool, tenant_id, root_id, "tenant_admin").await?;
        let assigned: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM user_roles WHERE user_id=$1 AND role_id=$2 AND organization_id=$3)")
            .bind(user_id).bind(role).bind(root_id).fetch_one(pool).await?;
        ensure!(assigned, "first user lacks tenant administrator assignment");
        for identifier in ["13800138001", "CONTACT@EXAMPLE.COM"] {
            let login = h.ok(Method::POST, "/api/auth/login", Some(json!({"username": identifier, "password": secret}))).await?;
            let ticket = login["ticket"].as_str().ok_or_else(|| anyhow!("login credential ticket missing"))?;
            let context = request(&h.app, ticket, Method::POST, "/api/auth/context", Some(json!({"organizationId": root_id})), StatusCode::OK).await?;
            let token = context["accessToken"].as_str().ok_or_else(|| anyhow!("session token missing"))?;
            let permissions = request(&h.app, token, Method::GET, "/api/auth/permissions", None, StatusCode::OK).await?;
            ensure!(permissions["permissions"].as_array().is_some_and(|items| items.contains(&json!("system:organization"))),
                "contact login has no effective administrator permissions");
        }
        let child = h.ok(Method::POST, "/api/organizations", Some(json!({
            "tenantId": tenant_id, "parentId": root_id, "name": "联系人子组织",
            "contactName": "联系人甲", "contactPhone": "13800138001", "contactEmail": "contact@example.com",
        }))).await?;
        ensure!(child["administrator"]["roleName"] == "超级管理员", "organization administrator label differs");
        ensure!(child["administrator"]["userId"] == administrator["userId"]
            && child["administrator"]["created"] == false && child["administrator"].get("initialPassword").is_none(),
            "existing contact account was duplicated or given a new password");
        let child_role = managed_role(pool, tenant_id, id(&child)?, "org_admin").await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM user_roles WHERE user_id=$1 AND role_id=$2")
            .bind(user_id).bind(child_role).fetch_one(pool).await?;
        ensure!(count == 1, "contact does not have organization administrator role");
        let email_only = format!("{}@example.com", "contact".repeat(8));
        let org = h.ok(Method::POST, "/api/organizations", Some(json!({
            "tenantId": tenant_id, "parentId": root_id, "name": "邮箱管理员组织",
            "contactName": "联系人乙", "contactEmail": email_only,
        }))).await?;
        ensure!(org["administrator"]["username"] == email_only && email_only.len() > 50,
            "email-only account does not support the full email length");
        h.ok(Method::POST, "/api/auth/login", Some(json!({"username": email_only,
            "password": org["administrator"]["initialPassword"]}))).await?;
        let email_login_audit: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE username=$1 AND action='LOGIN'")
            .bind(&email_only).fetch_one(pool).await?;
        ensure!(email_login_audit == 1, "long email username login lost its audit record");
        let updated = h.ok(Method::PUT, &format!("/api/tenants/{tenant_id}"), Some(json!({
            "contactName": "后续业务联系人", "contactPhone": "", "contactEmail": "later@example.com",
        }))).await?;
        ensure!(updated["contactName"] == "后续业务联系人" && updated["contactPhone"] == ""
            && updated.get("administrator").is_none(), "contact maintenance has provisioning side effects");
        let hash_after: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1")
            .bind(user_id).fetch_one(pool).await?;
        ensure!(hash_after == user.3, "contact maintenance reset the account password");
        let listed = h.ok(Method::GET, "/api/tenants?keyword=contact-tenant", None).await?;
        ensure!(listed["records"][0]["contactEmail"] == "later@example.com"
            && !listed.to_string().contains(secret), "read API lost contacts or disclosed initial credentials");
        let audit_leaks: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE detail LIKE '%' || $1 || '%'")
            .bind(secret).fetch_one(pool).await?;
        ensure!(audit_leaks == 0, "initial credential appeared in audit details");
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn shared_contact_identifiers_select_only_password_verified_accounts_and_contexts(
) -> Result<()> {
    let database = Database::create().await?;
    let result = async {
        let pool = &database.pool;
        let h = Harness::create(pool).await?;
        let first = h.ok(Method::POST, "/api/tenants", Some(json!({
            "name": "同电话租户甲", "code": "contact-first", "contactName": "联系人甲",
            "contactPhone": "13800138002", "contactEmail": "shared@example.com",
        }))).await?;
        let second = h.ok(Method::POST, "/api/tenants", Some(json!({
            "name": "同电话租户乙", "code": "contact-second", "contactName": "联系人乙",
            "contactPhone": "13800138002", "contactEmail": "second@example.com",
        }))).await?;
        let third = h.ok(Method::POST, "/api/tenants", Some(json!({
            "name": "同邮箱租户丙", "code": "contact-third", "contactName": "联系人丙",
            "contactPhone": "13800138003", "contactEmail": "shared@example.com",
        }))).await?;
        let first_id = id(&first)?;
        let second_id = id(&second)?;
        let third_id = id(&third)?;
        let first_user = first["administrator"]["userId"].as_str().ok_or_else(|| anyhow!("first contact user missing"))?;
        let second_user = second["administrator"]["userId"].as_str().ok_or_else(|| anyhow!("second contact user missing"))?;
        let first_secret = first["administrator"]["initialPassword"].as_str().ok_or_else(|| anyhow!("first credential missing"))?;
        let second_secret = second["administrator"]["initialPassword"].as_str().ok_or_else(|| anyhow!("second credential missing"))?;
        let third_secret = third["administrator"]["initialPassword"].as_str().ok_or_else(|| anyhow!("third credential missing"))?;
        ensure!(first_user != second_user && first_secret != second_secret,
            "shared telephone merged distinct people or reused a credential");
        for (identifier, secret, tenant_id) in [
            ("13800138002", first_secret, first_id), ("13800138002", second_secret, second_id),
            ("shared@example.com", first_secret, first_id), ("shared@example.com", third_secret, third_id),
            ("second@example.com", second_secret, second_id),
        ] {
            let login = h.ok(Method::POST, "/api/auth/login", Some(json!({"username": identifier, "password": secret}))).await?;
            let memberships = login["memberships"].as_array().ok_or_else(|| anyhow!("login choices missing"))?;
            ensure!(login["accountCount"] == 1 && memberships.iter().all(|item| item["tenantId"] == json!(tenant_id)),
                "shared alias exposed an account whose password was not verified");
        }
        // Repeated successful logins for one person must not lock another person
        // sharing the phone number by counting mismatched candidate hashes.
        for _ in 0..6 {
            h.ok(Method::POST, "/api/auth/login", Some(json!({"username": "13800138002", "password": first_secret}))).await?;
        }
        h.ok(Method::POST, "/api/auth/login", Some(json!({"username": "13800138002", "password": second_secret}))).await?;
        let same_person = h.ok(Method::POST, "/api/tenants", Some(json!({
            "name": "同一联系人另一个租户", "code": "contact-same-person", "contactName": "联系人甲",
            "contactPhone": "13800138002", "contactEmail": "shared@example.com",
        }))).await?;
        ensure!(same_person["administrator"]["userId"] == first["administrator"]["userId"]
            && same_person["administrator"]["created"] == false
            && same_person["administrator"].get("initialPassword").is_none(), "same contact did not reuse its proven identity");
        let reused_login = h.ok(Method::POST, "/api/auth/login", Some(json!({"username": "shared@example.com", "password": first_secret}))).await?;
        ensure!(reused_login["accountCount"] == 1 && reused_login["memberships"].as_array().is_some_and(|items| items.len() == 2),
            "one person managing multiple tenants lost membership choices");
        let shared_hash = crate::util::password::hash_password(first_secret)?;
        let second_uid: Uuid = second_user.parse()?;
        sqlx::query("UPDATE users SET password_hash=$2 WHERE id=$1")
            .bind(second_uid).bind(shared_hash).execute(pool).await?;
        let multi = h.ok(Method::POST, "/api/auth/login", Some(json!({"username": "13800138002", "password": first_secret}))).await?;
        ensure!(multi["accountCount"] == 2 && multi["memberships"].as_array().is_some_and(|items| items.len() == 3),
            "same alias/password cannot select among separately verified tenant accounts");
        let ticket = multi["ticket"].as_str().ok_or_else(|| anyhow!("multi-account ticket missing"))?;
        let roots_first = h.ok(Method::GET, &format!("/api/organizations/tree?tenantId={first_id}"), None).await?;
        let roots_second = h.ok(Method::GET, &format!("/api/organizations/tree?tenantId={second_id}"), None).await?;
        let first_root = id(&roots_first[0])?;
        let second_root = id(&roots_second[0])?;
        let selected = request(&h.app, ticket, Method::POST, "/api/auth/context", Some(json!({
            "organizationId": second_root, "userId": second_uid,
        })), StatusCode::OK).await?;
        ensure!(selected["user"]["id"] == json!(second_uid) && selected["tenant"]["id"] == json!(second_id),
            "selected tenant reused another account identity");
        let token = selected["accessToken"].as_str().ok_or_else(|| anyhow!("selected session missing"))?;
        request(&h.app, token, Method::GET, "/api/tenants", None, StatusCode::FORBIDDEN).await?;
        let scoped = request(&h.app, token, Method::GET, "/api/tenants/options", None, StatusCode::OK).await?;
        ensure!(scoped.as_array().is_some_and(|items| items.len() == 1 && items[0]["id"] == json!(second_id)),
            "selected account session exposes another tenant");
        request(&h.app, ticket, Method::POST, "/api/auth/context", Some(json!({
            "organizationId": second_root, "userId": first_user,
        })), StatusCode::FORBIDDEN).await?;
        request(&h.app, ticket, Method::POST, "/api/auth/context", Some(json!({
            "organizationId": second_root, "userId": Uuid::new_v4(),
        })), StatusCode::FORBIDDEN).await?;
        request(&h.app, ticket, Method::GET, "/api/auth/profile", None, StatusCode::UNAUTHORIZED).await?;
        sqlx::query("UPDATE users SET session_version=session_version+1 WHERE id=$1")
            .bind(second_uid).execute(pool).await?;
        request(&h.app, ticket, Method::POST, "/api/auth/context", Some(json!({
            "organizationId": second_root, "userId": second_uid,
        })), StatusCode::UNAUTHORIZED).await?;
        request(&h.app, ticket, Method::POST, "/api/auth/context", Some(json!({
            "organizationId": first_root, "userId": first_user,
        })), StatusCode::OK).await?;
        for fields in [json!({"contactName": "无电话邮箱"}), json!({"contactPhone": "13800138004"}),
            json!({"contactName": "格式错误", "contactPhone": "letters"}),
            json!({"contactName": "格式错误", "contactEmail": "not-an-email"})] {
            let mut body = fields;
            body["name"] = json!("非法联系人"); body["code"] = json!("invalid-contact");
            h.invalid(Method::POST, "/api/tenants", body).await?;
        }
        let partial: i64 = sqlx::query_scalar("SELECT count(*) FROM tenants WHERE code='invalid-contact'")
            .fetch_one(pool).await?;
        ensure!(partial == 0, "invalid contact left partial entities behind");
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn explicit_administrator_initialization_is_idempotent_and_preserves_accounts() -> Result<()>
{
    let database = Database::create().await?;
    let result = async {
        let pool = &database.pool;
        let h = Harness::create(pool).await?;
        let tenant = h.ok(Method::POST, "/api/tenants", Some(json!({"name":"旧租户","code":"existing-init"}))).await?;
        let tid = id(&tenant)?;
        let path = format!("/api/tenants/{tid}/initialize-administrator");
        request(&h.app, &h.token, Method::POST, &path, None, StatusCode::BAD_REQUEST).await?;
        h.ok(Method::PUT, &format!("/api/tenants/{tid}"), Some(json!({"contactName":"首位联系人","contactPhone":"13800138991","contactEmail":"init@example.com"}))).await?;
        let tree = h.ok(Method::GET, &format!("/api/organizations/tree?tenantId={tid}"), None).await?;
        let root = id(&tree[0])?;
        // Older entities can lack the managed role altogether.
        sqlx::query("DELETE FROM roles WHERE organization_id=$1").bind(root).execute(pool).await?;
        let (first, second) = tokio::try_join!(h.ok(Method::POST, &path, None), h.ok(Method::POST, &path, None))?;
        ensure!(first["administrator"]["userId"] == second["administrator"]["userId"], "concurrent initialization duplicated users");
        let created = if first["administrator"]["created"] == true { &first } else { &second };
        let repeated = if first["administrator"]["created"] == true { &second } else { &first };
        ensure!(created["administrator"]["initialPassword"].is_string() && repeated["administrator"]["alreadyInitialized"] == true
            && repeated["administrator"].get("initialPassword").is_none(), "initial credential/idempotency contract differs");
        let uid: Uuid = created["administrator"]["userId"].as_str().unwrap().parse()?;
        let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1").bind(uid).fetch_one(pool).await?;
        h.ok(Method::PUT, &format!("/api/tenants/{tid}"), Some(json!({"contactName":"替换联系人","contactPhone":"13800138992"}))).await?;
        let again = h.ok(Method::POST, &path, None).await?;
        ensure!(again["administrator"]["userId"] == created["administrator"]["userId"], "changed contact replaced existing admin");
        let root_again = h.ok(Method::POST, &format!("/api/organizations/{root}/initialize-administrator"), None).await?;
        ensure!(root_again["administrator"]["userId"] == created["administrator"]["userId"], "tenant and root initialization differ");
        let child = h.ok(Method::POST, "/api/organizations", Some(json!({"tenantId":tid,"parentId":root,"name":"旧子组织"}))).await?;
        let oid = id(&child)?;
        h.ok(Method::PUT, &format!("/api/organizations/{oid}"), Some(json!({"contactName":"首位联系人","contactPhone":"13800138991","contactEmail":"init@example.com"}))).await?;
        let child_init = h.ok(Method::POST, &format!("/api/organizations/{oid}/initialize-administrator"), None).await?;
        ensure!(child_init["administrator"]["userId"] == created["administrator"]["userId"] && child_init["administrator"]["created"] == false
            && child_init["administrator"].get("initialPassword").is_none(), "exact existing contact was not reused");
        let after: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1").bind(uid).fetch_one(pool).await?;
        ensure!(hash == after, "existing password changed");
        let users: i64 = sqlx::query_scalar("SELECT count(*) FROM memberships WHERE organization_id=$1").bind(root).fetch_one(pool).await?;
        ensure!(users == 1, "repeated initialization created personnel");
        let token = session(pool, tid, root, uid).await?;
        request(&h.app, &token, Method::POST, &path, None, StatusCode::FORBIDDEN).await?;
        let other = h.ok(Method::POST, "/api/tenants", Some(json!({"name":"其他租户","code":"other-init"}))).await?;
        let other_tree = h.ok(Method::GET, &format!("/api/organizations/tree?tenantId={}",id(&other)?), None).await?;
        request(&h.app, &token, Method::POST, &format!("/api/organizations/{}/initialize-administrator",id(&other_tree[0])?), None, StatusCode::NOT_FOUND).await?;
        let reader: Uuid = sqlx::query_scalar("INSERT INTO users(username,password_hash) VALUES('init-reader','unused') RETURNING id").fetch_one(pool).await?;
        sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)").bind(reader).bind(root).execute(pool).await?;
        let reader_token = session(pool, tid, root, reader).await?;
        request(&h.app, &reader_token, Method::POST, &format!("/api/organizations/{oid}/initialize-administrator"), None, StatusCode::FORBIDDEN).await?;
        // Preserve a manually maintained legacy administrator instead of granting another.
        sqlx::query("UPDATE roles SET built_in=false,code='legacy_admin' WHERE organization_id=$1 AND code='org_admin'").bind(oid).execute(pool).await?;
        request(&h.app, &h.token, Method::POST, &format!("/api/organizations/{oid}/initialize-administrator"), None, StatusCode::BAD_REQUEST).await?;
        let audit: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM audit_logs WHERE action='INITIALIZE_ADMINISTRATOR' AND detail LIKE '%' || $1 || '%')")
            .bind(created["administrator"]["initialPassword"].as_str().unwrap()).fetch_one(pool).await?;
        ensure!(!audit, "initial password leaked into audit");
        Ok(())
    }.await;
    database.finish(result).await
}
