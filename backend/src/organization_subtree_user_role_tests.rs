//! User and role management must stay within the selected organization subtree.
//! Run explicitly with `cargo test organization_subtree_user_role -- --ignored --nocapture`.
use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, ensure, Result};
use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::config::Config;
use crate::entity_creation_tests::Database;
use crate::routes;

const SECRET: &str = "organization-subtree-user-role-tests-only";

async fn org(pool: &PgPool, tenant: Uuid, parent: Option<Uuid>, name: &str) -> Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO organizations(tenant_id,parent_id,name,id) VALUES($1,$2,$3,$4) RETURNING id",
    )
    .bind(tenant)
    .bind(parent)
    .bind(name)
    .bind(crate::util::uuid7::new_uuid())
    .fetch_one(pool)
    .await?)
}

async fn account(pool: &PgPool, name: &str, orgs: &[Uuid], admin: bool) -> Result<Uuid> {
    let id = sqlx::query_scalar(
        "INSERT INTO users(username,password_hash,is_super_admin,id) VALUES($1,'test-only',$2,$3) RETURNING id",
    )
    .bind(name)
    .bind(admin)
    .bind(crate::util::uuid7::new_uuid()).fetch_one(pool)
    .await?;
    for org in orgs {
        sqlx::query("INSERT INTO memberships(user_id,organization_id,id) VALUES($1,$2,$3)")
            .bind(id)
            .bind(org)
            .bind(crate::util::uuid7::new_uuid())
            .execute(pool)
            .await?;
    }
    Ok(id)
}

async fn role(pool: &PgPool, tenant: Uuid, org: Uuid, name: &str, users: &[Uuid]) -> Result<Uuid> {
    let id = sqlx::query_scalar(
        "INSERT INTO roles(tenant_id,organization_id,name,code,id) VALUES($1,$2,$3,$3,$4) RETURNING id",
    )
    .bind(tenant)
    .bind(org)
    .bind(name)
    .bind(crate::util::uuid7::new_uuid()).fetch_one(pool)
    .await?;
    for user in users {
        sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3)")
            .bind(user)
            .bind(id)
            .bind(org)
            .execute(pool)
            .await?;
    }
    Ok(id)
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
            body.map(|value| value.to_string()).unwrap_or_default(),
        ))?;
    let response = app.clone().oneshot(request).await?;
    let status = response.status();
    let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
    ensure!(
        status == expected,
        "{path}: expected {expected}, got {status}: {body}"
    );
    Ok(body)
}

async fn role_assignment_rechecks_membership_after_lock(
    app: &Router,
    pool: &PgPool,
    token: &str,
    member_org: Uuid,
    other_org: Uuid,
    role: Uuid,
) -> Result<()> {
    for role_centric in [false, true] {
        let target = account(
            pool,
            &format!("moving-role-member-{role_centric}"),
            &[member_org, other_org],
            false,
        )
        .await?;
        let before: Vec<Uuid> =
            sqlx::query_scalar("SELECT user_id FROM user_roles WHERE role_id=$1 ORDER BY user_id")
                .bind(role)
                .fetch_all(pool)
                .await?;
        let mut blocker = pool.begin().await?;
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await?;
        sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
            .bind(target)
            .execute(&mut *blocker)
            .await?;
        let (path, body) = if role_centric {
            (
                format!("/api/roles/{role}/users"),
                json!({"userIds":[target]}),
            )
        } else {
            (
                format!("/api/users/{target}/roles"),
                json!({"items":[{"roleId":role}]}),
            )
        };
        let (app, token) = (app.clone(), token.to_owned());
        let pending = tokio::spawn(async move {
            call(
                &app,
                &token,
                Method::PUT,
                &path,
                Some(body),
                StatusCode::BAD_REQUEST,
            )
            .await
        });
        // Observe the actual account-lock wait instead of relying on a delay.
        // The actor keeps the other membership, so the general scope check
        // still succeeds after the specific role eligibility is removed.
        let waiting = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let observed: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
                     AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)) \
                     AND query LIKE '%FROM users%' \
                     AND (query LIKE '%FOR UPDATE%' OR query LIKE '%FOR NO KEY UPDATE%'))",
                )
                .bind(blocker_pid)
                .fetch_one(pool)
                .await?;
                if observed {
                    return Ok::<_, anyhow::Error>(());
                }
                if pending.is_finished() {
                    return Err(anyhow!("role assignment skipped the account-lock barrier"));
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
        if !matches!(waiting, Ok(Ok(()))) {
            blocker.rollback().await?;
            pending.abort();
            let _ = pending.await;
            return Err(anyhow!(
                "role assignment never reached the observed account-lock barrier"
            ));
        }
        sqlx::query("DELETE FROM memberships WHERE user_id=$1 AND organization_id=$2")
            .bind(target)
            .bind(member_org)
            .execute(&mut *blocker)
            .await?;
        blocker.commit().await?;
        tokio::time::timeout(std::time::Duration::from_secs(10), pending)
            .await
            .map_err(|_| anyhow!("role assignment did not finish after the member move"))???;
        let after: Vec<Uuid> =
            sqlx::query_scalar("SELECT user_id FROM user_roles WHERE role_id=$1 ORDER BY user_id")
                .bind(role)
                .fetch_all(pool)
                .await?;
        ensure!(
            before == after,
            "rejected role assignment changed existing bindings"
        );
    }
    Ok(())
}

async fn exercise(platform: bool) -> Result<()> {
    let database = Database::create().await?;
    let pool = &database.pool;
    let result = async {
        let tenant: Uuid = sqlx::query_scalar(
            "INSERT INTO tenants(name,code,is_platform,id) VALUES('Subtree tenant',$1,$2,$3) RETURNING id",
        ).bind(if platform { "admin-tenant" } else { "subtree-tenant" }).bind(platform)
            .bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?;
        let root = org(pool, tenant, None, "Root").await?;
        let child = org(pool, tenant, Some(root), "Child").await?;
        let leaf = org(pool, tenant, Some(child), "Leaf").await?;
        let sibling = org(pool, tenant, Some(root), "Sibling").await?;
        let admin = account(pool, "subtree-admin", &[root, child], true).await?;
        let parent_user = account(pool, "parent-user", &[root], false).await?;
        let child_user = account(pool, "child-user", &[child], false).await?;
        let leaf_user = account(pool, "leaf-user", &[leaf], false).await?;
        let sibling_user = account(pool, "sibling-user", &[sibling], false).await?;
        let shared = account(pool, "shared-user", &[child, sibling], false).await?;
        let parent_role = role(pool, tenant, root, "parent-role", &[parent_user, shared]).await?;
        let child_role = role(pool, tenant, child, "child-role", &[child_user, shared, sibling_user]).await?;
        let leaf_role = role(pool, tenant, leaf, "leaf-role", &[leaf_user]).await?;
        let sibling_role = role(pool, tenant, sibling, "sibling-role", &[sibling_user, shared]).await?;

        let application: Uuid = sqlx::query_scalar(
            "INSERT INTO applications(name,code,id) VALUES('Subtree system','system',$1) RETURNING id",
        ).bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?;
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
            .bind(tenant).bind(application).execute(pool).await?;
        for org in [root, child] {
            sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)")
                .bind(org).bind(application).execute(pool).await?;
        }
        for permission in ["system:user", "system:role", "system:organization"] {
            let menu: Uuid = sqlx::query_scalar(
                "INSERT INTO menus(application_id,title,permission,type,id) VALUES($1,$2,$2,'MENU',$3) RETURNING id",
            ).bind(application).bind(permission).bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?;
            sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
                .bind(tenant).bind(menu).execute(pool).await?;
            for org in [root, child] {
                sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
                    .bind(org).bind(menu).execute(pool).await?;
            }
        }
        let config = Config::from_values(&HashMap::from([
            ("APPLICATION_ENVIRONMENT".into(), "development".into()),
            ("DATABASE_URL".into(), "postgres://unused/subtree-user-role-test".into()),
            ("JWT_SECRET".into(), SECRET.into()),
        ]))?;
        let app = routes::api_router().with_state(crate::sso_test_fixture::state(pool.clone(), Arc::new(config)));
        let token = session(pool, admin, tenant, child).await?;
        let root_token = session(pool, admin, tenant, root).await?;

        let users = call(&app, &token, Method::GET, "/api/users?size=100", None, StatusCode::OK).await?;
        ensure!(users["total"] == 4, "user list count included ancestors/siblings: {users}");
        let rows = users["records"].as_array().unwrap();
        ensure!(!rows.iter().any(|row| row["id"] == json!(parent_user) || row["id"] == json!(sibling_user)),
            "out-of-scope users entered list");
        let shared_row = rows.iter().find(|row| row["id"] == json!(shared)).unwrap();
        ensure!(shared_row["roleCount"] == 1 && shared_row["memberships"].as_array().unwrap().len() == 1
            && shared_row["memberships"][0]["organizationId"] == json!(child), "shared user expanded outside subtree");
        let roles = call(&app, &token, Method::GET, "/api/roles?size=100", None, StatusCode::OK).await?;
        ensure!(roles["total"] == 2 && roles["records"].as_array().unwrap().iter().all(|row|
            row["id"] == json!(child_role) || row["id"] == json!(leaf_role)), "roles included ancestors/siblings");
        let child_row = roles["records"].as_array().unwrap().iter().find(|row| row["id"] == json!(child_role)).unwrap();
        ensure!(child_row["userCount"] == 2, "role user count leaked out-of-scope legacy assignment");
        let assigned = call(&app, &token, Method::GET, &format!("/api/roles/{child_role}/users"), None, StatusCode::OK).await?;
        ensure!(assigned.as_array().unwrap().len() == 2 && !assigned.to_string().contains(&sibling_user.to_string()),
            "role users leaked sibling-only member");
        let memberships = call(&app, &token, Method::GET, &format!("/api/users/{shared}/memberships"), None, StatusCode::OK).await?;
        ensure!(memberships.as_array().unwrap().len() == 1 && memberships[0]["organizationId"] == json!(child),
            "membership detail included sibling");
        let assigned_roles = call(&app, &token, Method::GET, &format!("/api/users/{shared}/roles"), None, StatusCode::OK).await?;
        ensure!(assigned_roles.as_array().unwrap().len() == 1 && assigned_roles[0]["roleId"] == json!(child_role),
            "role detail exposed ancestors/siblings");

        for outside in [root, sibling] {
            for base in ["users", "roles"] {
                call(&app, &token, Method::GET, &format!("/api/{base}?organizationId={outside}"), None, StatusCode::NOT_FOUND).await?;
            }
            call(&app, &token, Method::POST, "/api/roles", Some(json!({"name":"Denied role", "organizationId":outside})), StatusCode::NOT_FOUND).await?;
            call(&app, &token, Method::PUT, &format!("/api/users/{shared}/memberships"),
                Some(json!({"items":[{"organizationId":outside}]})), StatusCode::NOT_FOUND).await?;
        }
        for outside in [parent_user, sibling_user] {
            for suffix in ["memberships", "roles"] {
                call(&app, &token, Method::GET, &format!("/api/users/{outside}/{suffix}"), None, StatusCode::NOT_FOUND).await?;
            }
            call(&app, &token, Method::PUT, &format!("/api/roles/{child_role}/users"),
                Some(json!({"userIds":[outside]})), StatusCode::NOT_FOUND).await?;
        }
        for outside in [parent_role, sibling_role] {
            for suffix in ["menus", "users"] {
                call(&app, &token, Method::GET, &format!("/api/roles/{outside}/{suffix}"), None, StatusCode::NOT_FOUND).await?;
            }
            call(&app, &token, Method::PUT, &format!("/api/roles/{outside}"), Some(json!({"name":"Denied rename"})), StatusCode::NOT_FOUND).await?;
            call(&app, &token, Method::DELETE, &format!("/api/roles/{outside}"), None, StatusCode::NOT_FOUND).await?;
            call(&app, &token, Method::PUT, &format!("/api/users/{shared}/roles"),
                Some(json!({"items":[{"roleId":outside}]})), StatusCode::BAD_REQUEST).await?;
        }
        // The role-oriented endpoint must enforce the same member/ancestor
        // relationship as user-oriented assignment, even for visible users.
        call(&app, &token, Method::PUT, &format!("/api/roles/{leaf_role}/users"),
            Some(json!({"userIds":[child_user]})), StatusCode::BAD_REQUEST).await?;
        call(&app, &root_token, Method::PUT, &format!("/api/roles/{sibling_role}/users"),
            Some(json!({"userIds":[child_user]})), StatusCode::BAD_REQUEST).await?;
        let retained: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM user_roles WHERE role_id=$1")
            .bind(leaf_role).fetch_all(pool).await?;
        ensure!(retained == vec![leaf_user], "invalid role member replacement removed existing bindings");
        call(&app, &token, Method::PUT, &format!("/api/users/{shared}"), Some(json!({"realName":"Denied shared edit"})), StatusCode::FORBIDDEN).await?;
        call(&app, &token, Method::PUT, &format!("/api/users/{shared}/status"), Some(json!({"status":"DISABLED"})), StatusCode::FORBIDDEN).await?;
        call(&app, &token, Method::POST, &format!("/api/users/{shared}/reset-password"), Some(json!({"newPassword":"Regression-Strong-123!"})), StatusCode::FORBIDDEN).await?;
        call(&app, &token, Method::DELETE, &format!("/api/users/{shared}"), None, StatusCode::FORBIDDEN).await?;
        let batch = call(&app, &token, Method::POST, "/api/users/batch-status", Some(json!({"userIds":[shared,sibling_user],"status":"DISABLED"})), StatusCode::OK).await?;
        ensure!(batch["succeeded"] == json!([]) && batch["failed"] == json!([shared,sibling_user]), "batch modified scope-shared identity");
        call(&app, &token, Method::PUT, &format!("/api/users/{child_user}"), Some(json!({"realName":"Allowed local edit"})), StatusCode::OK).await?;

        // A scoped replacement must leave invisible memberships and roles intact.
        call(&app, &token, Method::PUT, &format!("/api/users/{shared}/roles"), Some(json!({"items":[]})), StatusCode::OK).await?;
        let remaining: Vec<Uuid> = sqlx::query_scalar("SELECT role_id FROM user_roles WHERE user_id=$1 ORDER BY role_id")
            .bind(shared).fetch_all(pool).await?;
        ensure!(remaining.len() == 2 && remaining.contains(&parent_role) && remaining.contains(&sibling_role),
            "scoped role replacement removed invisible grants");
        call(&app, &token, Method::PUT, &format!("/api/users/{shared}/memberships"),
            Some(json!({"items":[{"organizationId":leaf,"position":"Preserved scope"}]})), StatusCode::OK).await?;
        let remaining: Vec<Uuid> = sqlx::query_scalar("SELECT organization_id FROM memberships WHERE user_id=$1")
            .bind(shared).fetch_all(pool).await?;
        ensure!(remaining.len() == 2 && remaining.contains(&leaf) && remaining.contains(&sibling),
            "scoped member replacement removed sibling membership");
        call(&app, &token, Method::PUT, &format!("/api/users/{shared}/roles"),
            Some(json!({"items":[{"roleId":child_role}]})), StatusCode::OK).await?;
        call(&app, &token, Method::PUT, &format!("/api/roles/{child_role}/users"),
            Some(json!({"userIds":[leaf_user]})), StatusCode::OK).await?;
        let remaining: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM user_roles WHERE role_id=$1")
            .bind(child_role).fetch_all(pool).await?;
        ensure!(remaining.len() == 2 && remaining.contains(&leaf_user) && remaining.contains(&sibling_user),
            "scoped role-user replacement removed invisible assignment");
        let renamed = call(&app, &token, Method::PUT, &format!("/api/roles/{child_role}"),
            Some(json!({"name":"Allowed role rename"})), StatusCode::OK).await?;
        ensure!(renamed["userCount"] == 1, "role update response leaked invisible user count");
        let root_roles = call(&app, &root_token, Method::GET, "/api/roles?size=100", None, StatusCode::OK).await?;
        ensure!(root_roles["total"] == 4, "root lost visibility over descendants");
        let root_users = call(&app, &root_token, Method::GET, "/api/users?size=100", None, StatusCode::OK).await?;
        ensure!(root_users["total"] == 6, "root lost visibility over users");

        // Contact matching must not reveal or attach ancestor/sibling-only accounts.
        for (outside, name, phone) in [
            (parent_user, "Ancestor contact", "13800138101"),
            (sibling_user, "Sibling contact", "13800138102"),
        ] {
            let email = format!("{phone}@example.com");
            sqlx::query("UPDATE users SET real_name=$2,phone=$3,email=$4 WHERE id=$1")
                .bind(outside).bind(name).bind(phone).bind(&email).execute(pool).await?;
            let created = call(&app, &token, Method::POST, "/api/organizations", Some(json!({
                "tenantId":tenant,"parentId":child,"name":format!("Created for {name}"),
                "contactName":name,"contactPhone":phone,"contactEmail":email,
            })), StatusCode::OK).await?;
            ensure!(created["administrator"]["created"] == true
                && created["administrator"]["userId"] != json!(outside)
                && created["administrator"]["initialPassword"].is_string(),
                "child organization reused an out-of-scope contact account");
            let untouched: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1")
                .bind(outside).fetch_one(pool).await?;
            ensure!(untouched == "test-only", "contact onboarding changed outside password");
        }
        sqlx::query("UPDATE users SET real_name='Outside initialization contact',phone='13800138103',email='outside-init@example.com' WHERE id=$1")
            .bind(parent_user).execute(pool).await?;
        sqlx::query("UPDATE organizations SET contact_name='Outside initialization contact',contact_phone='13800138103',contact_email='outside-init@example.com' WHERE id=$1")
            .bind(leaf).execute(pool).await?;
        let initialized = call(&app, &token, Method::POST,
            &format!("/api/organizations/{leaf}/initialize-administrator"), None, StatusCode::OK).await?;
        ensure!(initialized["administrator"]["created"] == true
            && initialized["administrator"]["userId"] != json!(parent_user),
            "explicit child initialization reused ancestor-only contact account");
        for (candidate, name, phone, actor, parent) in [
            (child_user, "Scoped contact", "13800138104", &token, child),
            (leaf_user, "Tenant root contact", "13800138105", &root_token, root),
        ] {
            let email = format!("{phone}@example.com");
            sqlx::query("UPDATE users SET real_name=$2,phone=$3,email=$4 WHERE id=$1")
                .bind(candidate).bind(name).bind(phone).bind(&email).execute(pool).await?;
            let reused = call(&app, actor, Method::POST, "/api/organizations", Some(json!({
                "tenantId":tenant,"parentId":parent,"name":format!("Reused for {name}"),
                "contactName":name,"contactPhone":phone,"contactEmail":email,
            })), StatusCode::OK).await?;
            ensure!(reused["administrator"]["created"] == false
                && reused["administrator"]["userId"] == json!(candidate)
                && reused["administrator"].get("initialPassword").is_none(),
                "scoped or tenant-root contact matching no longer reused an eligible account");
        }
        role_assignment_rechecks_membership_after_lock(&app, pool, &token, leaf, child, leaf_role).await?;
        Ok(())
    }.await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn organization_subtree_user_role_regular_tenant() -> Result<()> {
    exercise(false).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn organization_subtree_user_role_platform_child() -> Result<()> {
    exercise(true).await
}
