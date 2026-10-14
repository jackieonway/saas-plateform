//! Phase 10 notifications and Phase 11 scoped app-member regressions on isolated PostgreSQL.
//! Run explicitly with `cargo test notification_tests -- --ignored --nocapture`.
//! TEST_DATABASE_URL is read from the environment or ignored root .env.test; never printed.
use std::borrow::Cow;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::Config;
use crate::routes;
use crate::state::AppState;
use crate::util::{hash, jwt};

const SECRET: &str = "notification-isolated-tests-only";
const APP_CODE: &str = "notification-app";
const OTHER_CODE: &str = "notification-other";

struct Database {
    pool: PgPool,
    admin: PgPool,
    name: String,
}

fn database_options() -> Result<PgConnectOptions> {
    let url = match std::env::var("TEST_DATABASE_URL") {
        Ok(url) if !url.trim().is_empty() => url,
        _ => {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.env.test");
            let entries = dotenvy::from_path_iter(path).map_err(|_| {
                anyhow!("set TEST_DATABASE_URL or configure ignored root .env.test")
            })?;
            let mut value = None;
            for entry in entries {
                let (key, item) =
                    entry.map_err(|_| anyhow!("test environment cannot be parsed"))?;
                if key == "TEST_DATABASE_URL" && !item.trim().is_empty() {
                    value = Some(item);
                }
            }
            value.ok_or_else(|| anyhow!("TEST_DATABASE_URL is missing"))?
        }
    };
    PgConnectOptions::from_str(&url)
        .map_err(|_| anyhow!("invalid test database connection configuration"))
}

fn database_error(stage: &str, error: &sqlx::Error) -> anyhow::Error {
    match error.as_database_error().and_then(|error| error.code()) {
        Some(code) => anyhow!("{stage} failed (SQLSTATE {code})"),
        None => anyhow!("{stage} failed; check test database connectivity and privileges"),
    }
}

impl Database {
    async fn create(before_notification_migration: bool) -> Result<Self> {
        let options = database_options()?;
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .map_err(|error| database_error("administration connection", &error))?;
        let name = format!("notification_test_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await
            .map_err(|error| database_error("creating isolated test database", &error))?;
        let pool = match PgPoolOptions::new()
            .max_connections(8)
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
        if before_notification_migration {
            migrator.migrations = Cow::Owned(
                migrator
                    .iter()
                    .filter(|m| m.version < 22)
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
            "notification test cleanup assertions_ok={} database_removed={} pool_closed={}",
            assertions.is_ok(),
            cleanup.is_ok(),
            finally_closed
        );
        assertions?;
        cleanup?;
        ensure!(
            finally_closed,
            "isolated test pool did not close after database removal"
        );
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
    .map_err(|_| anyhow!("notification API timeout: {method} {path}"))??;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

use tower::ServiceExt;

async fn ok(
    app: &Router,
    method: Method,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> Result<Value> {
    let (status, response) = call(app, method.clone(), path, token, body).await?;
    ensure!(
        status == StatusCode::OK,
        "{method} {path} returned {status}"
    );
    Ok(response)
}

async fn rejected(
    app: &Router,
    method: Method,
    path: &str,
    token: &str,
    body: Option<Value>,
    expected: &[StatusCode],
) -> Result<()> {
    let (status, _) = call(app, method.clone(), path, token, body).await?;
    ensure!(
        expected.contains(&status),
        "{method} {path} returned unexpected rejection {status}"
    );
    Ok(())
}

struct Session {
    token: String,
    id: Uuid,
}

async fn session(
    pool: &PgPool,
    user: Uuid,
    tenant: Uuid,
    org: Uuid,
    jwt_ttl: i64,
    database_ttl: i64,
) -> Result<Session> {
    let version: i64 = sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
        .bind(user)
        .fetch_one(pool)
        .await?;
    let now = Utc::now().timestamp();
    let id: Uuid = sqlx::query_scalar("INSERT INTO platform_sessions(user_id,tenant_id,organization_id,expires_at) VALUES($1,$2,$3,to_timestamp($4)) RETURNING id")
        .bind(user).bind(tenant).bind(org).bind((now + database_ttl) as f64).fetch_one(pool).await?;
    let token = jwt::sign(
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
            exp: now + jwt_ttl,
        },
        SECRET,
    )?;
    Ok(Session { token, id })
}

async fn tenant(pool: &PgPool, name: &str, platform: bool) -> Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO tenants(name,code,is_platform) VALUES($1,$1,$2) RETURNING id",
    )
    .bind(name)
    .bind(platform)
    .fetch_one(pool)
    .await?)
}

async fn organization(
    pool: &PgPool,
    tenant: Uuid,
    name: &str,
    parent: Option<Uuid>,
) -> Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO organizations(tenant_id,name,parent_id) VALUES($1,$2,$3) RETURNING id",
    )
    .bind(tenant)
    .bind(name)
    .bind(parent)
    .fetch_one(pool)
    .await?)
}

async fn user(pool: &PgPool, name: &str, super_admin: bool, orgs: &[Uuid]) -> Result<Uuid> {
    let id: Uuid = sqlx::query_scalar("INSERT INTO users(username,password_hash,is_super_admin) VALUES($1,'isolated-unused-password',$2) RETURNING id")
        .bind(name).bind(super_admin).fetch_one(pool).await?;
    for org in orgs {
        sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)")
            .bind(id)
            .bind(org)
            .execute(pool)
            .await?;
    }
    Ok(id)
}

async fn grant_application(pool: &PgPool, tenant: Uuid, org: Uuid, app: Uuid) -> Result<()> {
    sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(tenant).bind(app).execute(pool).await?;
    sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(org).bind(app).execute(pool).await?;
    Ok(())
}

struct Harness {
    pool: PgPool,
    router: Router,
    tenant_a: Uuid,
    tenant_b: Uuid,
    root: Uuid,
    child: Uuid,
    peer: Uuid,
    remote: Uuid,
    no_app_org: Uuid,
    main_app: Uuid,
    other_app: Uuid,
    publish_menu: Uuid,
    root_role: Uuid,
    publisher: Uuid,
    second_publisher: Uuid,
    recipient: Uuid,
    child_user: Uuid,
    peer_user: Uuid,
    remote_user: Uuid,
    disabled_user: Uuid,
    no_app_user: Uuid,
    super_user: Uuid,
    publisher_session: Session,
    child_session: Session,
    remote_session: Session,
    second_session: Session,
    recipient_session: Session,
    recipient_child_session: Session,
    recipient_remote_session: Session,
    platform_session: Session,
    super_session: Session,
}

impl Harness {
    async fn create(pool: &PgPool) -> Result<Self> {
        let tenant_a = tenant(pool, "notification-tenant-a", false).await?;
        let tenant_b = tenant(pool, "notification-tenant-b", false).await?;
        let platform = tenant(pool, "notification-platform", true).await?;
        let tenant_root = organization(pool, tenant_a, "tenant root", None).await?;
        let root = organization(pool, tenant_a, "publisher branch", Some(tenant_root)).await?;
        let child = organization(pool, tenant_a, "child", Some(root)).await?;
        let peer = organization(pool, tenant_a, "peer", Some(tenant_root)).await?;
        let no_app_org =
            organization(pool, tenant_a, "without application", Some(tenant_root)).await?;
        let remote = organization(pool, tenant_b, "remote root", None).await?;
        let platform_org = organization(pool, platform, "platform root", None).await?;
        let publisher = user(
            pool,
            "notification-publisher",
            false,
            &[root, child, remote],
        )
        .await?;
        let second_publisher = user(pool, "notification-second", false, &[root]).await?;
        let recipient = user(
            pool,
            "notification-recipient",
            false,
            &[root, child, remote],
        )
        .await?;
        let child_user = user(pool, "notification-child", false, &[child]).await?;
        let peer_user = user(pool, "notification-peer", false, &[peer]).await?;
        let remote_user = user(pool, "notification-remote", false, &[remote]).await?;
        let disabled_user = user(pool, "notification-disabled", false, &[root]).await?;
        let no_app_user = user(pool, "notification-no-app", false, &[no_app_org]).await?;
        let super_user = user(pool, "notification-super", true, &[root]).await?;
        let platform_user =
            user(pool, "notification-platform-admin", true, &[platform_org]).await?;
        sqlx::query("UPDATE users SET status='DISABLED' WHERE id=$1")
            .bind(disabled_user)
            .execute(pool)
            .await?;
        let mut applications = Vec::new();
        let mut menus = Vec::new();
        for code in [APP_CODE, OTHER_CODE] {
            let app: Uuid = sqlx::query_scalar("INSERT INTO applications(code,name,base_path,entry_type,entry_url) VALUES($1,$1,$2,'HTML',$3) RETURNING id")
                .bind(code).bind(format!("/micro/{code}")).bind(format!("/sub-apps/{code}/index.html")).fetch_one(pool).await?;
            let menu: Uuid = sqlx::query_scalar("INSERT INTO menus(application_id,type,title,permission) VALUES($1,'BUTTON','Publish','notification:publish') RETURNING id")
                .bind(app).fetch_one(pool).await?;
            sqlx::query("INSERT INTO oauth_clients(client_id,client_secret_hash,client_name,client_type,grant_types,scopes,access_token_validity,application_id) VALUES($1,$2,$1,'CONFIDENTIAL','client_credentials','profile',300,$3)")
                .bind(code).bind(hash::sha256_hex("notification-test-client-secret-only")).bind(app).execute(pool).await?;
            for (tid, org) in [
                (tenant_a, root),
                (tenant_a, child),
                (tenant_a, peer),
                (tenant_b, remote),
            ] {
                grant_application(pool, tid, org, app).await?;
                sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
                    .bind(tid).bind(menu).execute(pool).await?;
                sqlx::query(
                    "INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)",
                )
                .bind(org)
                .bind(menu)
                .execute(pool)
                .await?;
            }
            applications.push(app);
            menus.push(menu);
        }
        let root_role: Uuid = sqlx::query_scalar("INSERT INTO roles(tenant_id,organization_id,name) VALUES($1,$2,'Notification publisher') RETURNING id")
            .bind(tenant_a).bind(root).fetch_one(pool).await?;
        let remote_role: Uuid = sqlx::query_scalar("INSERT INTO roles(tenant_id,organization_id,name) VALUES($1,$2,'Remote publisher') RETURNING id")
            .bind(tenant_b).bind(remote).fetch_one(pool).await?;
        for (uid, role, org) in [
            (publisher, root_role, root),
            (second_publisher, root_role, root),
            (publisher, remote_role, remote),
        ] {
            sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3)")
                .bind(uid)
                .bind(role)
                .bind(org)
                .execute(pool)
                .await?;
        }
        for role in [root_role, remote_role] {
            for menu in &menus {
                sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)")
                    .bind(role)
                    .bind(menu)
                    .execute(pool)
                    .await?;
            }
        }
        let audit_menu: Uuid = sqlx::query_scalar("INSERT INTO menus(application_id,type,title,permission) VALUES($1,'BUTTON','Audit','system:audit') RETURNING id")
            .bind(applications[0]).fetch_one(pool).await?;
        sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
            .bind(tenant_a)
            .bind(audit_menu)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
            .bind(root)
            .bind(audit_menu)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)")
            .bind(root_role)
            .bind(audit_menu)
            .execute(pool)
            .await?;
        grant_application(pool, platform, platform_org, applications[0]).await?;
        for permission in ["notification:publish", "system:user", "system:role", "system:audit"] {
            let menu = if permission == "notification:publish" {
                menus[0]
            } else if permission == "system:audit" {
                audit_menu
            } else {
                sqlx::query_scalar("INSERT INTO menus(application_id,type,title,permission) VALUES($1,'BUTTON',$2,$2) RETURNING id")
                    .bind(applications[0]).bind(permission).fetch_one(pool).await?
            };
            sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
                .bind(platform).bind(menu).execute(pool).await?;
            sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
                .bind(platform_org).bind(menu).execute(pool).await?;
        }
        let mut config = Config::from_env();
        config.database_url = "postgres://unused/notification-test".into();
        config.jwt_secret = SECRET.into();
        let router = routes::api_router().with_state(AppState::new(pool.clone(), Arc::new(config)));
        let h = Self {
            pool: pool.clone(),
            router,
            tenant_a,
            tenant_b,
            root,
            child,
            peer,
            remote,
            no_app_org,
            main_app: applications[0],
            other_app: applications[1],
            publish_menu: menus[0],
            root_role,
            publisher,
            second_publisher,
            recipient,
            child_user,
            peer_user,
            remote_user,
            disabled_user,
            no_app_user,
            super_user,
            publisher_session: session(pool, publisher, tenant_a, root, 3600, 3600).await?,
            child_session: session(pool, publisher, tenant_a, child, 3600, 3600).await?,
            remote_session: session(pool, publisher, tenant_b, remote, 3600, 3600).await?,
            second_session: session(pool, second_publisher, tenant_a, root, 3600, 3600).await?,
            recipient_session: session(pool, recipient, tenant_a, root, 3600, 3600).await?,
            recipient_child_session: session(pool, recipient, tenant_a, child, 3600, 3600).await?,
            recipient_remote_session: session(pool, recipient, tenant_b, remote, 3600, 3600)
                .await?,
            platform_session: session(pool, platform_user, platform, platform_org, 3600, 3600)
                .await?,
            super_session: session(pool, super_user, tenant_a, root, 3600, 3600).await?,
        };
        h.grant(publisher, root, h.main_app, "TENANT").await?;
        Ok(h)
    }

    async fn grant(&self, user: Uuid, org: Uuid, app: Uuid, scope: &str) -> Result<()> {
        let affected = sqlx::query("INSERT INTO membership_data_grants(membership_id,application_id,resource_code,action_code,scope_type) SELECT id,$3,'notification','publish',$4 FROM memberships WHERE user_id=$1 AND organization_id=$2 ON CONFLICT(membership_id,application_id,resource_code,action_code) DO UPDATE SET scope_type=EXCLUDED.scope_type")
            .bind(user).bind(org).bind(app).bind(scope).execute(&self.pool).await?.rows_affected();
        ensure!(affected == 1, "grant fixture has no direct membership");
        Ok(())
    }

    async fn app_token(&self, session: &str, code: &str) -> Result<String> {
        let (status, response) = call(
            &self.router,
            Method::POST,
            "/api/oauth/session-token",
            session,
            Some(json!({"appCode":code})),
        )
        .await?;
        ensure!(
            status == StatusCode::CREATED,
            "application token fixture issuance failed: {status}"
        );
        response["accessToken"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("application token response is incomplete"))
    }

    async fn publish(&self, token: &str, body: Value) -> Result<Value> {
        ok(
            &self.router,
            Method::POST,
            &format!("/api/apps/{APP_CODE}/notifications"),
            token,
            Some(body),
        )
        .await
    }

    async fn counts(&self, key: &str) -> Result<(i64, i64)> {
        let pubs: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM notification_publications WHERE idempotency_key=$1",
        )
        .bind(key)
        .fetch_one(&self.pool)
        .await?;
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications n JOIN notification_publications p ON p.id=n.publication_id WHERE p.idempotency_key=$1").bind(key).fetch_one(&self.pool).await?;
        Ok((pubs, rows))
    }

    async fn deliveries(&self, publication: &Value) -> Result<Vec<(Uuid, Uuid, Uuid)>> {
        let id = id_of(publication, "publicationId")?;
        Ok(sqlx::query_as("SELECT tenant_id,organization_id,user_id FROM notifications WHERE publication_id=$1 ORDER BY organization_id,user_id")
            .bind(id).fetch_all(&self.pool).await?)
    }
}

fn id_of(body: &Value, field: &str) -> Result<Uuid> {
    body[field]
        .as_str()
        .ok_or_else(|| anyhow!("missing {field}"))?
        .parse()
        .map_err(Into::into)
}
fn user_target(user: Uuid, org: Uuid) -> Value {
    json!({"kind":"USER","userId":user,"organizationId":org})
}
fn org_target(org: Uuid) -> Value {
    json!({"kind":"ORGANIZATION","organizationId":org})
}
fn body(key: &str, target: Value) -> Value {
    json!({"idempotencyKey":key,"target":target,"type":"INFO","title":"通知测试","content":"Persistent message","link":""})
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn notification_application_authorization_and_four_scopes() -> Result<()> {
    let db = Database::create(false).await?;
    let result: Result<()> = async {
        let h = Harness::create(&db.pool).await?;
        let token = h.app_token(&h.publisher_session.token,APP_CODE).await?;
        let path = format!("/api/apps/{APP_CODE}/notifications");
        let reject = &[StatusCode::FORBIDDEN,StatusCode::NOT_FOUND];
        for scope in ["SELF","ORG","ORG_SUBTREE","TENANT"] {
            h.grant(h.publisher,h.root,h.main_app,scope).await?;
            h.publish(&token,body(&format!("{scope}:self"),user_target(h.publisher,h.root))).await?;
            if scope == "SELF" {
                rejected(&h.router,Method::POST,&path,&token,Some(body("self-other",user_target(h.recipient,h.root))),reject).await?;
                rejected(&h.router,Method::POST,&path,&token,Some(body("self-broadcast",org_target(h.root))),reject).await?;
            } else { h.publish(&token,body(&format!("{scope}:other"),user_target(h.recipient,h.root))).await?; }
            if ["ORG_SUBTREE","TENANT"].contains(&scope) {
                h.publish(&token,body(&format!("{scope}:child"),user_target(h.child_user,h.child))).await?;
                let child = h.publish(&token,body(&format!("{scope}:child-broadcast"),org_target(h.child))).await?;
                ensure!(h.deliveries(&child).await?.iter().all(|(_,org,_)|*org==h.child), "organization broadcast expanded its target");
            } else {
                rejected(&h.router,Method::POST,&path,&token,Some(body(&format!("{scope}:child-deny"),user_target(h.child_user,h.child))),reject).await?;
            }
            rejected(&h.router,Method::POST,&path,&token,Some(body(&format!("{scope}:peer-deny"),user_target(h.peer_user,h.peer))),reject).await?;
            rejected(&h.router,Method::POST,&path,&token,Some(body(&format!("{scope}:tenant-deny"),json!({"kind":"TENANT"}))),reject).await?;
        }
        let direct = h.publish(&token,body("direct-root",org_target(h.root))).await?;
        let rows = h.deliveries(&direct).await?;
        ensure!(rows.iter().all(|(_,org,uid)|*org==h.root && *uid!=h.child_user && *uid!=h.disabled_user), "direct broadcast included subtree/disabled user");
        ensure!(direct["deliveryCount"].as_u64()==Some(rows.len() as u64) && direct["recipientCount"].as_u64()==Some(rows.iter().map(|(_,_,uid)|*uid).collect::<std::collections::HashSet<_>>().len() as u64), "publication counts differ from actual recipients");
        for (uid,org,key) in [(h.remote_user,h.remote,"cross-tenant"),(h.no_app_user,h.no_app_org,"missing-app"),(h.disabled_user,h.root,"disabled-user")] {
            rejected(&h.router,Method::POST,&path,&token,Some(body(key,user_target(uid,org))),reject).await?;
            ensure!(h.counts(key).await?==(0,0), "rejected target created a publication");
        }
        sqlx::query("DELETE FROM organization_applications WHERE organization_id=$1 AND application_id=$2").bind(h.child).bind(h.main_app).execute(&db.pool).await?;
        rejected(&h.router,Method::POST,&path,&token,Some(body("receiver-app-revoked",user_target(h.child_user,h.child))),reject).await?;
        grant_application(&db.pool,h.tenant_a,h.child,h.main_app).await?;
        sqlx::query("UPDATE organizations SET status='DISABLED' WHERE id=$1").bind(h.child).execute(&db.pool).await?;
        rejected(&h.router,Method::POST,&path,&token,Some(body("receiver-org-disabled",user_target(h.child_user,h.child))),reject).await?;
        sqlx::query("UPDATE organizations SET status='ENABLED',expires_at=now()-interval '1 hour' WHERE id=$1").bind(h.child).execute(&db.pool).await?;
        rejected(&h.router,Method::POST,&path,&token,Some(body("receiver-org-expired",user_target(h.child_user,h.child))),reject).await?;
        sqlx::query("UPDATE organizations SET expires_at=NULL WHERE id=$1").bind(h.child).execute(&db.pool).await?;
        // Disabling a recipient organization intentionally revokes its sessions.
        // The next operation needs a newly established child context.
        let renewed_child_session = session(&db.pool,h.publisher,h.tenant_a,h.child,3600,3600).await?;
        let child_token = h.app_token(&renewed_child_session.token,APP_CODE).await?;
        let context = ok(&h.router,Method::GET,&format!("/api/apps/{APP_CODE}/context"),&child_token,None).await?;
        ensure!(context["permissions"].as_array().is_some_and(|v|v.contains(&json!("notification:publish"))), "ancestor role fixture did not provide the operation");
        rejected(&h.router,Method::POST,&path,&child_token,Some(body("ancestor-no-grant",user_target(h.publisher,h.child))),&[StatusCode::FORBIDDEN]).await?;
        let super_token = h.app_token(&h.super_session.token,APP_CODE).await?;
        rejected(&h.router,Method::POST,&path,&super_token,Some(body("super-no-grant",user_target(h.super_user,h.root))),&[StatusCode::FORBIDDEN]).await?;
        sqlx::query("DELETE FROM role_menus WHERE role_id=$1 AND menu_id=$2").bind(h.root_role).bind(h.publish_menu).execute(&db.pool).await?;
        rejected(&h.router,Method::POST,&path,&token,Some(body("no-operation",user_target(h.publisher,h.root))),&[StatusCode::FORBIDDEN]).await?;
        sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)").bind(h.root_role).bind(h.publish_menu).execute(&db.pool).await?;
        rejected(&h.router,Method::POST,&format!("/api/apps/{OTHER_CODE}/notifications"),&token,Some(body("wrong-audience",user_target(h.publisher,h.root))),&[StatusCode::FORBIDDEN,StatusCode::UNAUTHORIZED]).await?;
        rejected(&h.router,Method::POST,&path,&h.publisher_session.token,Some(body("session-not-app",user_target(h.publisher,h.root))),&[StatusCode::UNAUTHORIZED]).await?;
        for typ in ["credential","refresh","access"] {
            let now = Utc::now().timestamp();
            let forged = jwt::sign(&jwt::Claims { sub:h.publisher.to_string(),typ:typ.into(),tenant:Some(h.tenant_a.to_string()),org:Some(h.root.to_string()),cid:None,scope:None,sid:None,ver:None,aud:None,jti:None,iat:now,exp:now+3600 },SECRET)?;
            rejected(&h.router,Method::POST,&path,&forged,Some(body(&format!("wrong-type:{typ}"),user_target(h.publisher,h.root))),&[StatusCode::UNAUTHORIZED]).await?;
        }
        let mut expired=jwt::verify(&token,SECRET)?;expired.exp=Utc::now().timestamp()-60;
        rejected(&h.router,Method::POST,&path,&jwt::sign(&expired,SECRET)?,Some(body("expired-token",user_target(h.publisher,h.root))),&[StatusCode::UNAUTHORIZED]).await?;
        let mut admin_body = body("admin-adapter",user_target(h.recipient,h.root)); admin_body["applicationId"]=json!(h.main_app);
        let admin = ok(&h.router,Method::POST,"/api/notifications",&h.publisher_session.token,Some(admin_body)).await?;
        ensure!(admin["sent"]==admin["recipientCount"] && admin["sent"]==1, "session adapter returned incorrect sent count");
        sqlx::query("UPDATE oauth_tokens SET revoked=true WHERE application_id=$1 AND user_id=$2").bind(h.main_app).bind(h.publisher).execute(&db.pool).await?;
        rejected(&h.router,Method::POST,&path,&token,Some(body("revoked-token",user_target(h.recipient,h.root))),&[StatusCode::UNAUTHORIZED]).await?;
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn notification_tenant_grants_obey_selected_organization_subtree() -> Result<()> {
    let db = Database::create(false).await?;
    let result: Result<()> = async {
        let h = Harness::create(&db.pool).await?;
        let ancestor: Uuid = sqlx::query_scalar("SELECT parent_id FROM organizations WHERE id=$1")
            .bind(h.root).fetch_one(&db.pool).await?;
        let grandchild = organization(&db.pool, h.tenant_a, "nested child", Some(h.child)).await?;
        let grandchild_user = user(&db.pool, "notification-nested-child", false, &[grandchild]).await?;
        grant_application(&db.pool, h.tenant_a, grandchild, h.main_app).await?;

        let (platform_user, platform_tenant, platform_root): (Uuid, Uuid, Uuid) = sqlx::query_as(
            "SELECT user_id,tenant_id,organization_id FROM platform_sessions WHERE id=$1",
        ).bind(h.platform_session.id).fetch_one(&db.pool).await?;
        let platform_child = organization(&db.pool, platform_tenant, "platform child", Some(platform_root)).await?;
        let platform_leaf = organization(&db.pool, platform_tenant, "platform leaf", Some(platform_child)).await?;
        let platform_peer = organization(&db.pool, platform_tenant, "platform peer", Some(platform_root)).await?;
        for org in [platform_child, platform_leaf, platform_peer] {
            sqlx::query("INSERT INTO memberships(user_id,organization_id) VALUES($1,$2)")
                .bind(platform_user).bind(org).execute(&db.pool).await?;
            grant_application(&db.pool, platform_tenant, org, h.main_app).await?;
            sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
                .bind(org).bind(h.publish_menu).execute(&db.pool).await?;
        }
        let platform_child_session = session(&db.pool, platform_user, platform_tenant, platform_child, 3600, 3600).await?;
        h.grant(h.publisher, h.child, h.main_app, "TENANT").await?;
        h.grant(platform_user, platform_child, h.main_app, "TENANT").await?;
        h.grant(platform_user, platform_root, h.main_app, "TENANT").await?;
        let path = format!("/api/apps/{APP_CODE}/notifications");
        for (label, session_token, current, descendant, recipient, outside) in [
            ("ordinary-branch", h.publisher_session.token.as_str(), h.root, h.child, h.child_user, vec![(ancestor, h.publisher), (h.peer, h.peer_user)]),
            ("ordinary-child", h.child_session.token.as_str(), h.child, grandchild, grandchild_user, vec![(h.root, h.publisher), (h.peer, h.peer_user)]),
            ("platform-child", platform_child_session.token.as_str(), platform_child, platform_leaf, platform_user, vec![(platform_root, platform_user), (platform_peer, platform_user)]),
        ] {
            let token = h.app_token(session_token, APP_CODE).await?;
            let context = ok(&h.router, Method::GET, &format!("/api/apps/{APP_CODE}/context"), &token, None).await?;
            let grant = context["dataGrants"].as_array().and_then(|grants| grants.iter().find(|grant| {
                grant["resource"] == "notification" && grant["action"] == "publish"
            })).ok_or_else(|| anyhow!("missing notification grant for {label}"))?;
            let ids = grant["organizationIds"].as_array().ok_or_else(|| anyhow!("missing subtree IDs for {label}"))?;
            ensure!(grant["scope"] == "ORG_SUBTREE" && ids.contains(&json!(current)) && ids.contains(&json!(descendant)),
                "TENANT grant was not capped to selected subtree for {label}");
            let published = h.publish(&token, body(&format!("{label}:descendant"), user_target(recipient, descendant))).await?;
            ensure!(h.deliveries(&published).await? == vec![(if label == "platform-child" { platform_tenant } else { h.tenant_a }, descendant, recipient)],
                "descendant publication delivered outside its target for {label}");
            for (org, uid) in outside {
                ensure!(!ids.contains(&json!(org)), "ancestor/sibling leaked into app context for {label}");
                for (kind, target) in [("user", user_target(uid, org)), ("organization", org_target(org))] {
                    let key = format!("{label}:{kind}:{org}");
                    rejected(&h.router, Method::POST, &path, &token, Some(body(&key, target)), &[StatusCode::NOT_FOUND]).await?;
                    ensure!(h.counts(&key).await? == (0, 0), "rejected out-of-subtree target created records for {label}");
                }
            }
            let key = format!("{label}:tenant");
            rejected(&h.router, Method::POST, &path, &token, Some(body(&key, json!({"kind":"TENANT"}))), &[StatusCode::FORBIDDEN]).await?;
            ensure!(h.counts(&key).await? == (0, 0), "child context broadcast to entire tenant for {label}");
        }

        let token = h.app_token(&h.platform_session.token, APP_CODE).await?;
        let publication = h.publish(&token, body("platform-root:tenant", json!({"kind":"TENANT"}))).await?;
        let rows = h.deliveries(&publication).await?;
        ensure!(rows.len() == 4 && rows.iter().all(|(tenant, _, uid)| *tenant == platform_tenant && *uid == platform_user),
            "platform-root tenant broadcast did not preserve tenant/recipient isolation");
        ensure!(publication["recipientCount"] == 1 && publication["deliveryCount"] == 4,
            "multi-organization recipient publication counts changed");
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn notification_idempotency_namespaces_concurrency_and_transaction_failures() -> Result<()> {
    let db = Database::create(false).await?;
    let result: Result<()> = async {
        let h = Harness::create(&db.pool).await?;
        let token = h.app_token(&h.publisher_session.token,APP_CODE).await?;
        let payload = body("concurrent-key",user_target(h.recipient,h.root));
        let (a,b) = tokio::join!(h.publish(&token,payload.clone()),h.publish(&token,payload.clone()));
        let a=a?; let b=b?;
        ensure!(a["publicationId"]==b["publicationId"] && a["duplicate"]!=b["duplicate"], "concurrent requests did not share one original publication");
        ensure!(h.counts("concurrent-key").await?==(1,1), "concurrent publication duplicated records");
        let mut normalized=body("normalized-key",user_target(h.recipient,h.root));normalized["content"]=json!("");
        let first_normalized=h.publish(&token,normalized.clone()).await?;
        normalized.as_object_mut().unwrap().remove("content");normalized.as_object_mut().unwrap().remove("link");normalized["title"]=json!("  通知测试  ");
        let renewed=h.app_token(&h.publisher_session.token,APP_CODE).await?;
        let normalized_retry=h.publish(&renewed,normalized).await?;
        ensure!(normalized_retry["duplicate"]==true && normalized_retry["publicationId"]==first_normalized["publicationId"] && h.counts("normalized-key").await?==(1,1), "normalization or token renewal changed idempotency");
        for field in ["title","content","type","link","target"] {
            let mut changed=payload.clone(); changed[field]=match field {
                "type"=>json!("WARNING"), "link"=>json!(format!("/micro/{APP_CODE}/changed")),
                "target"=>user_target(h.publisher,h.root), _=>json!("changed payload"),
            };
            rejected(&h.router,Method::POST,&format!("/api/apps/{APP_CODE}/notifications"),&token,Some(changed),&[StatusCode::CONFLICT]).await?;
        }
        let notification: Uuid=sqlx::query_scalar("SELECT id FROM notifications WHERE publication_id=$1").bind(id_of(&a,"publicationId")?).fetch_one(&db.pool).await?;
        ok(&h.router,Method::DELETE,&format!("/api/notifications/{notification}"),&h.recipient_session.token,None).await?;
        let again=h.publish(&token,payload.clone()).await?;
        ensure!(again["duplicate"]==true && again["publicationId"]==a["publicationId"] && again["recipientCount"]==a["recipientCount"] && again["deliveryCount"]==a["deliveryCount"] && again["createdAt"]==a["createdAt"] && h.counts("concurrent-key").await?==(1,0), "retry recreated a deleted recipient notification");
        sqlx::query("DELETE FROM membership_data_grants WHERE application_id=$1 AND membership_id IN(SELECT id FROM memberships WHERE user_id=$2 AND organization_id=$3)").bind(h.main_app).bind(h.publisher).bind(h.root).execute(&db.pool).await?;
        rejected(&h.router,Method::POST,&format!("/api/apps/{APP_CODE}/notifications"),&token,Some(payload),&[StatusCode::FORBIDDEN]).await?;
        h.grant(h.publisher,h.root,h.main_app,"TENANT").await?;
        h.grant(h.publisher,h.child,h.main_app,"SELF").await?;
        h.grant(h.publisher,h.remote,h.main_app,"SELF").await?;
        h.grant(h.publisher,h.root,h.other_app,"SELF").await?;
        h.grant(h.second_publisher,h.root,h.main_app,"SELF").await?;
        let t_child=h.app_token(&h.child_session.token,APP_CODE).await?;
        let t_remote=h.app_token(&h.remote_session.token,APP_CODE).await?;
        let t_other=h.app_token(&h.publisher_session.token,OTHER_CODE).await?;
        let t_second=h.app_token(&h.second_session.token,APP_CODE).await?;
        for (code,tok,uid,org) in [(APP_CODE,token.as_str(),h.publisher,h.root),(APP_CODE,t_child.as_str(),h.publisher,h.child),(APP_CODE,t_remote.as_str(),h.publisher,h.remote),(OTHER_CODE,t_other.as_str(),h.publisher,h.root),(APP_CODE,t_second.as_str(),h.second_publisher,h.root)] {
            let response=ok(&h.router,Method::POST,&format!("/api/apps/{code}/notifications"),tok,Some(body("namespace-key",user_target(uid,org)))).await?;
            ensure!(response["duplicate"]==false, "five-part namespace incorrectly deduplicated separate identities");
        }
        ensure!(h.counts("namespace-key").await?==(5,5), "five namespaces did not remain independent");
        for (table,key) in [("notifications","insert-failure"),("audit_logs","audit-failure")] {
            sqlx::query("CREATE FUNCTION notification_test_fail_insert() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected notification test failure'; END $$").execute(&db.pool).await?;
            sqlx::query(&format!("CREATE TRIGGER notification_test_failure BEFORE INSERT ON {table} FOR EACH ROW EXECUTE FUNCTION notification_test_fail_insert()"))
                .execute(&db.pool).await?;
            let request=body(key,user_target(h.recipient,h.root));
            rejected(&h.router,Method::POST,&format!("/api/apps/{APP_CODE}/notifications"),&token,Some(request.clone()),&[StatusCode::INTERNAL_SERVER_ERROR]).await?;
            ensure!(h.counts(key).await?==(0,0), "failed transaction left publication or recipient rows");
            sqlx::query(&format!("DROP TRIGGER notification_test_failure ON {table}")).execute(&db.pool).await?;
            sqlx::query("DROP FUNCTION notification_test_fail_insert()").execute(&db.pool).await?;
            let retried=h.publish(&token,request).await?;
            ensure!(retried["duplicate"]==false && h.counts(key).await?==(1,1), "rolled-back key could not be retried atomically");
        }
        let original: String=sqlx::query_scalar("SELECT request_hash FROM notification_publications WHERE id=$1").bind(id_of(&a,"publicationId")?).fetch_one(&db.pool).await?;
        let mutation=sqlx::query("UPDATE notification_publications SET request_hash=repeat('0',64) WHERE id=$1").bind(id_of(&a,"publicationId")?).execute(&db.pool).await;
        ensure!(mutation.as_ref().err().and_then(|e|e.as_database_error()).and_then(|e|e.code()).as_deref()==Some("23514"), "publication definition was mutable or failed for an unrelated reason");
        let unchanged: String=sqlx::query_scalar("SELECT request_hash FROM notification_publications WHERE id=$1").bind(id_of(&a,"publicationId")?).fetch_one(&db.pool).await?;
        ensure!(unchanged==original, "rejected definition mutation changed publication");
        for case in ["queued-revocation","queued-app-expiry","queued-session-expiry"] {
            let mut request=body(case,user_target(h.recipient,h.root));
            let (route,queued_token,expiry)=if case=="queued-session-expiry" {
                let short=session(&db.pool,h.publisher,h.tenant_a,h.root,3,3600).await?;
                let expiry=jwt::verify(&short.token,SECRET)?.exp;
                request["applicationId"]=json!(h.main_app);
                ("/api/notifications".to_string(),short.token,Some(expiry))
            } else {
                let fresh=h.app_token(&h.publisher_session.token,APP_CODE).await?;
                let expiry=if case=="queued-app-expiry" {
                    let expiry=Utc::now().timestamp()+3;
                    sqlx::query("UPDATE oauth_tokens SET expires_at=to_timestamp($2) WHERE token_hash=$1")
                        .bind(hash::sha256_hex(&fresh)).bind(expiry as f64).execute(&db.pool).await?;
                    Some(expiry)
                } else { None };
                (format!("/api/apps/{APP_CODE}/notifications"),fresh,expiry)
            };
            let namespace=json!([h.tenant_a,h.root,h.main_app,h.publisher,case]).to_string();
            let mut blocker=db.pool.begin().await?;
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,10))").bind(namespace).execute(&mut *blocker).await?;
            let queued_router=h.router.clone();let queued_bearer=queued_token.clone();
            let queued=tokio::spawn(async move { call(&queued_router,Method::POST,&route,&queued_bearer,Some(request)).await });
            let deadline=tokio::time::Instant::now()+Duration::from_secs(10);
            loop {
                let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE 'SELECT pg_advisory_xact_lock%')").fetch_one(&db.pool).await?;
                if waiting { break; }
                ensure!(!queued.is_finished() && tokio::time::Instant::now()<deadline,"publication did not reach the controlled namespace lock for {case}");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            if let Some(expiry)=expiry {
                while Utc::now().timestamp()<=expiry {tokio::time::sleep(Duration::from_millis(50)).await;}
            } else {
                sqlx::query("UPDATE oauth_tokens SET revoked=true WHERE token_hash=$1").bind(hash::sha256_hex(&queued_token)).execute(&db.pool).await?;
            }
            blocker.rollback().await?;
            let (status,_)=queued.await??;
            ensure!(status==StatusCode::UNAUTHORIZED && h.counts(case).await?==(0,0),"queued {case} passed stale credentials after the lock was released");
        }
        Ok(())
    }.await;
    db.finish(result).await
}

async fn notification_id(
    pool: &PgPool,
    publication: &Value,
    org: Uuid,
    user: Uuid,
) -> Result<Uuid> {
    Ok(sqlx::query_scalar("SELECT id FROM notifications WHERE publication_id=$1 AND organization_id=$2 AND user_id=$3")
        .bind(id_of(publication,"publicationId")?).bind(org).bind(user).fetch_one(pool).await?)
}
async fn is_read(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT is_read FROM notifications WHERE id=$1")
            .bind(id)
            .fetch_one(pool)
            .await?,
    )
}
async fn inbox(h: &Harness, token: &str) -> Result<Value> {
    ok(
        &h.router,
        Method::GET,
        "/api/notifications?page=1&size=100",
        token,
        None,
    )
    .await
}
async fn unread(h: &Harness, token: &str) -> Result<i64> {
    let row = ok(
        &h.router,
        Method::GET,
        "/api/notifications/unread-count",
        token,
        None,
    )
    .await?;
    row["count"]
        .as_i64()
        .ok_or_else(|| anyhow!("unread count missing"))
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn notification_legacy_migration_and_context_inbox_boundaries() -> Result<()> {
    let db = Database::create(true).await?;
    let result: Result<()>=async {
        let h=Harness::create(&db.pool).await?;
        let no_membership=user(&db.pool,"notification-legacy-unassigned",false,&[]).await?;
        let mut legacy_ids=Vec::new();
        for (uid,read) in [(no_membership,false),(h.second_publisher,true),(h.recipient,false)] {
            let id:Uuid=sqlx::query_scalar("INSERT INTO notifications(user_id,type,title,content,link,is_read,read_at,created_at) VALUES($1,'SECURITY','Legacy retained','旧通知原文','/profile',$2,CASE WHEN $2 THEN TIMESTAMPTZ '2024-01-02T00:00:00Z' ELSE NULL END,TIMESTAMPTZ '2024-01-01T00:00:00Z') RETURNING id")
                .bind(uid).bind(read).fetch_one(&db.pool).await?; legacy_ids.push(id);
        }
        let snapshot_sql="SELECT jsonb_agg(jsonb_build_object('id',id,'user',user_id,'title',title,'content',content,'link',link,'type',type,'read',is_read,'readAt',read_at,'createdAt',created_at) ORDER BY id) FROM notifications WHERE id=ANY($1)";
        let before:Value=sqlx::query_scalar(snapshot_sql).bind(&legacy_ids).fetch_one(&db.pool).await?;
        sqlx::migrate!("./migrations").run(&db.pool).await?;
        let after:Value=sqlx::query_scalar(snapshot_sql).bind(&legacy_ids).fetch_one(&db.pool).await?;
        ensure!(before==after, "legacy migration altered message content or read timestamps");
        let preserved:i64=sqlx::query_scalar("SELECT count(*) FROM notifications WHERE id=ANY($1) AND source_kind='LEGACY' AND tenant_id IS NULL AND organization_id IS NULL AND application_id IS NULL AND publication_id IS NULL")
            .bind(&legacy_ids).fetch_one(&db.pool).await?;
        ensure!(preserved==3, "migration inferred or copied legacy ownership");
        ensure!(inbox(&h,&h.second_session.token).await?["total"]==0 && unread(&h,&h.recipient_session.token).await?==0, "legacy data entered a contextual inbox");
        let legacy=ok(&h.router,Method::GET,"/api/notifications/legacy?page=1&size=100",&h.platform_session.token,None).await?;
        ensure!(legacy["total"]==3, "platform legacy audit did not preserve the old rows");
        rejected(&h.router,Method::GET,"/api/notifications/legacy",&h.publisher_session.token,None,&[StatusCode::FORBIDDEN]).await?;
        let token=h.app_token(&h.publisher_session.token,APP_CODE).await?;
        rejected(&h.router,Method::GET,"/api/notifications/legacy",&token,None,&[StatusCode::UNAUTHORIZED]).await?;
        let root=h.publish(&token,body("inbox-root",user_target(h.recipient,h.root))).await?;
        let child=h.publish(&token,body("inbox-child",user_target(h.recipient,h.child))).await?;
        let root_id=notification_id(&db.pool,&root,h.root,h.recipient).await?;
        let child_id=notification_id(&db.pool,&child,h.child,h.recipient).await?;
        let system:Uuid=sqlx::query_scalar("INSERT INTO notifications(user_id,type,title,content,link,source_kind,tenant_id,organization_id) VALUES($1,'INFO','Context system','','','SYSTEM',$2,$3) RETURNING id")
            .bind(h.recipient).bind(h.tenant_a).bind(h.root).fetch_one(&db.pool).await?;
        ensure!(inbox(&h,&h.recipient_session.token).await?["total"]==2 && unread(&h,&h.recipient_session.token).await?==2, "root inbox count/list differ");
        ensure!(inbox(&h,&h.recipient_child_session.token).await?["total"]==1 && unread(&h,&h.recipient_child_session.token).await?==1 && unread(&h,&h.recipient_remote_session.token).await?==0, "same user saw another organization or tenant's messages");
        let (status,_)=call(&h.router,Method::PUT,&format!("/api/notifications/{root_id}/read"),&h.recipient_child_session.token,None).await?;
        ensure!([StatusCode::OK,StatusCode::NOT_FOUND].contains(&status) && !is_read(&db.pool,root_id).await?, "cross-organization read changed a message");
        ok(&h.router,Method::PUT,"/api/notifications/read-batch",&h.recipient_session.token,Some(json!({"ids":[root_id,child_id,legacy_ids[2]]}))).await?;
        ensure!(is_read(&db.pool,root_id).await? && !is_read(&db.pool,child_id).await? && !is_read(&db.pool,legacy_ids[2]).await?, "batch read crossed notification ownership");
        rejected(&h.router,Method::DELETE,&format!("/api/notifications/{child_id}"),&h.recipient_session.token,None,&[StatusCode::NOT_FOUND]).await?;
        ok(&h.router,Method::DELETE,"/api/notifications",&h.recipient_session.token,Some(json!({"ids":[root_id,child_id,legacy_ids[2]]}))).await?;
        let remaining:i64=sqlx::query_scalar("SELECT count(*) FROM notifications WHERE id=ANY($1)").bind(vec![root_id,child_id,legacy_ids[2]]).fetch_one(&db.pool).await?;
        ensure!(remaining==2, "batch delete removed another context or legacy record");
        ok(&h.router,Method::PUT,"/api/notifications/read-all",&h.recipient_session.token,None).await?;
        ensure!(is_read(&db.pool,system).await? && !is_read(&db.pool,child_id).await? && unread(&h,&h.recipient_session.token).await?==0, "read-all exceeded current context");
        sqlx::query("DELETE FROM organization_applications WHERE organization_id=$1 AND application_id=$2").bind(h.child).bind(h.main_app).execute(&db.pool).await?;
        ensure!(inbox(&h,&h.recipient_child_session.token).await?["total"]==0 && unread(&h,&h.recipient_child_session.token).await?==0, "revoked app's historical notifications stayed visible");
        let retained:i64=sqlx::query_scalar("SELECT count(*) FROM notifications WHERE id=$1").bind(child_id).fetch_one(&db.pool).await?;
        ensure!(retained==1, "app access revocation deleted historical notification data");
        let preference=json!({"dndEnabled":true,"dndStart":"08:00","dndEnd":"09:00","mutedTypes":["INFO"]});
        ok(&h.router,Method::PUT,"/api/notifications/prefs",&h.publisher_session.token,Some(preference)).await?;
        let theirs=ok(&h.router,Method::GET,"/api/notifications/prefs",&h.recipient_session.token,None).await?;
        ensure!(theirs["dndEnabled"]==false, "one user's preference changed another user");
        let own=ok(&h.router,Method::GET,"/api/notifications/prefs",&h.child_session.token,None).await?;
        ensure!(own["dndEnabled"]==true, "personal preferences were accidentally organization-specific");
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn notification_socket_context_validates_live_session_and_delivery_visibility() -> Result<()>
{
    let db = Database::create(false).await?;
    let result:Result<()>=async {
        let h=Harness::create(&db.pool).await?;
        let path="/api/notifications/socket-context";
        let token=h.app_token(&h.publisher_session.token,APP_CODE).await?;
        rejected(&h.router,Method::GET,path,&token,None,&[StatusCode::UNAUTHORIZED]).await?;
        let mut missing_sid=jwt::verify(&h.publisher_session.token,SECRET)?; missing_sid.sid=None;
        rejected(&h.router,Method::GET,path,&jwt::sign(&missing_sid,SECRET)?,None,&[StatusCode::UNAUTHORIZED]).await?;
        let mut wrong_binding=jwt::verify(&h.publisher_session.token,SECRET)?;wrong_binding.org=Some(h.child.to_string());
        rejected(&h.router,Method::GET,path,&jwt::sign(&wrong_binding,SECRET)?,None,&[StatusCode::UNAUTHORIZED]).await?;
        let expired=session(&db.pool,h.recipient,h.tenant_a,h.root,-120,3600).await?;
        rejected(&h.router,Method::GET,path,&expired.token,None,&[StatusCode::UNAUTHORIZED]).await?;
        let mut just_expired=jwt::verify(&h.publisher_session.token,SECRET)?;just_expired.exp=Utc::now().timestamp()-1;
        rejected(&h.router,Method::GET,path,&jwt::sign(&just_expired,SECRET)?,None,&[StatusCode::UNAUTHORIZED]).await?;
        let password_session=session(&db.pool,h.second_publisher,h.tenant_a,h.root,3600,3600).await?;
        crate::session::revoke_all(&db.pool,h.second_publisher,None).await?;
        rejected(&h.router,Method::GET,path,&password_session.token,None,&[StatusCode::UNAUTHORIZED]).await?;
        for (jwt_ttl,db_ttl) in [(900,1800),(1800,900)] {
            let fresh=session(&db.pool,h.recipient,h.tenant_a,h.root,jwt_ttl,db_ttl).await?;
            let claims=jwt::verify(&fresh.token,SECRET)?;
            let expires:DateTime<Utc>=sqlx::query_scalar("SELECT expires_at FROM platform_sessions WHERE id=$1").bind(fresh.id).fetch_one(&db.pool).await?;
            let context=ok(&h.router,Method::GET,path,&fresh.token,None).await?;
            ensure!(context["contractVersion"]=="1.0" && context["userId"]==h.recipient.to_string() && context["tenantId"]==h.tenant_a.to_string() && context["organizationId"]==h.root.to_string() && context["sessionId"]==fresh.id.to_string(), "socket context identity is not authoritative");
            let returned=DateTime::parse_from_rfc3339(context["expiresAt"].as_str().ok_or_else(||anyhow!("socket expiration missing"))?)?.timestamp();
            ensure!(returned==claims.exp.min(expires.timestamp()), "socket context did not use earlier JWT/database expiry");
        }
        let published=h.publish(&token,body("socket-owned",user_target(h.recipient,h.root))).await?;
        let notification=notification_id(&db.pool,&published,h.root,h.recipient).await?;
        let delivery_path=format!("{path}?notificationId={notification}");
        let system:Uuid=sqlx::query_scalar("INSERT INTO notifications(user_id,title,source_kind,tenant_id,organization_id) VALUES($1,'Socket system','SYSTEM',$2,$3) RETURNING id")
            .bind(h.recipient).bind(h.tenant_a).bind(h.root).fetch_one(&db.pool).await?;
        let system_path=format!("{path}?notificationId={system}");
        ok(&h.router,Method::GET,&delivery_path,&h.recipient_session.token,None).await?;
        rejected(&h.router,Method::GET,&delivery_path,&h.recipient_child_session.token,None,&[StatusCode::NOT_FOUND]).await?;
        rejected(&h.router,Method::GET,&delivery_path,&h.recipient_remote_session.token,None,&[StatusCode::NOT_FOUND]).await?;
        let legacy:Uuid=sqlx::query_scalar("INSERT INTO notifications(user_id,title) VALUES($1,'isolated legacy') RETURNING id").bind(h.recipient).fetch_one(&db.pool).await?;
        rejected(&h.router,Method::GET,&format!("{path}?notificationId={legacy}"),&h.recipient_session.token,None,&[StatusCode::NOT_FOUND]).await?;
        sqlx::query("DELETE FROM organization_applications WHERE organization_id=$1 AND application_id=$2").bind(h.root).bind(h.main_app).execute(&db.pool).await?;
        rejected(&h.router,Method::GET,&delivery_path,&h.recipient_session.token,None,&[StatusCode::FORBIDDEN,StatusCode::NOT_FOUND]).await?;
        ok(&h.router,Method::GET,&system_path,&h.recipient_session.token,None).await?;
        grant_application(&db.pool,h.tenant_a,h.root,h.main_app).await?;
        ok(&h.router,Method::GET,&delivery_path,&h.recipient_session.token,None).await?;
        sqlx::query("UPDATE applications SET status='DISABLED' WHERE id=$1").bind(h.main_app).execute(&db.pool).await?;
        rejected(&h.router,Method::GET,&delivery_path,&h.recipient_session.token,None,&[StatusCode::FORBIDDEN,StatusCode::NOT_FOUND]).await?;
        sqlx::query("UPDATE applications SET status='ENABLED' WHERE id=$1").bind(h.main_app).execute(&db.pool).await?;
        sqlx::query("UPDATE platform_sessions SET revoked_at=now() WHERE id=$1").bind(h.recipient_session.id).execute(&db.pool).await?;
        rejected(&h.router,Method::GET,path,&h.recipient_session.token,None,&[StatusCode::UNAUTHORIZED]).await?;
        ok(&h.router,Method::GET,path,&h.recipient_child_session.token,None).await?;
        sqlx::query("DELETE FROM memberships WHERE user_id=$1 AND organization_id=$2").bind(h.recipient).bind(h.child).execute(&db.pool).await?;
        rejected(&h.router,Method::GET,path,&h.recipient_child_session.token,None,&[StatusCode::UNAUTHORIZED]).await?;
        sqlx::query("UPDATE tenants SET status='DISABLED' WHERE id=$1").bind(h.tenant_b).execute(&db.pool).await?;
        rejected(&h.router,Method::GET,path,&h.recipient_remote_session.token,None,&[StatusCode::UNAUTHORIZED]).await?;
        sqlx::query("UPDATE organizations SET status='DISABLED' WHERE id=$1").bind(h.root).execute(&db.pool).await?;
        rejected(&h.router,Method::GET,path,&h.publisher_session.token,None,&[StatusCode::UNAUTHORIZED]).await?;
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn notification_input_limits_and_context_constraints_fail_before_partial_publish(
) -> Result<()> {
    let db = Database::create(false).await?;
    let result:Result<()>=async {
        let h=Harness::create(&db.pool).await?;
        let token=h.app_token(&h.publisher_session.token,APP_CODE).await?;
        let path=format!("/api/apps/{APP_CODE}/notifications");
        let valid=body("bad-input",user_target(h.recipient,h.root));
        let mut cases=Vec::new();
        for (field,value) in [("title",json!("")),("title",json!("字".repeat(201))),("type",json!("UNKNOWN")),("content",json!("x".repeat(8193))),
            ("idempotencyKey",json!("")),("idempotencyKey",json!("bad key")),("idempotencyKey",json!("k".repeat(129))),
            ("tenantId",json!(h.tenant_b)),("sourceKind",json!("SYSTEM")),("link",json!("https://external.invalid")),
            ("link",json!(format!("/micro/{OTHER_CODE}/secret"))),("link",json!(format!("/micro/{APP_CODE}/../{OTHER_CODE}/secret"))),
            ("link",json!("//external.invalid/path")),("link",json!(format!("/micro/{APP_CODE}-other/secret"))),
            ("link",json!(format!("/micro/{APP_CODE}/%2e%2e%2f{OTHER_CODE}/secret"))),
            ("link",json!(format!("/micro/{APP_CODE}/%252e%252e/{OTHER_CODE}/secret"))),
            ("link",json!(format!("/micro/{APP_CODE}/bad\\path"))),("link",json!(format!("/micro/{APP_CODE}/bad\npath")))] {
            let mut request=valid.clone();request[field]=value;cases.push(request);
        }
        let mut mixed=valid.clone();mixed["target"]["tenantId"]=json!(h.tenant_b);cases.push(mixed);
        for field in ["userId","organizationId","tenantId"] {
            let mut mixed=body("mixed-tenant",json!({"kind":"TENANT"}));mixed["target"][field]=json!(h.recipient);cases.push(mixed);
        }
        for request in cases { rejected(&h.router,Method::POST,&path,&token,Some(request),&[StatusCode::BAD_REQUEST]).await?; }
        ensure!(h.counts("bad-input").await?==(0,0), "invalid input left partial publication data");
        let mut oversized=valid.clone(); oversized["content"]=json!("x".repeat(70*1024));
        rejected(&h.router,Method::POST,&path,&token,Some(oversized),&[StatusCode::BAD_REQUEST,StatusCode::PAYLOAD_TOO_LARGE]).await?;
        let mut padded=vec![b' ';70*1024];padded.extend(serde_json::to_vec(&body("raw-body-limit",user_target(h.recipient,h.root)))?);
        let response=h.router.clone().oneshot(Request::builder().method(Method::POST).uri(&path)
            .header(header::AUTHORIZATION,format!("Bearer {token}")).header(header::CONTENT_TYPE,"application/json")
            .body(Body::from(padded))?).await?;
        ensure!([StatusCode::BAD_REQUEST,StatusCode::PAYLOAD_TOO_LARGE].contains(&response.status()) && h.counts("raw-body-limit").await?==(0,0),"raw 64 KiB body limit was bypassed with otherwise valid JSON");
        let unicode_link=format!("/micro/{APP_CODE}/{}","字".repeat(100));
        let mut unicode=body("unicode-boundaries",user_target(h.recipient,h.root));
        unicode["title"]=json!("字".repeat(200));unicode["content"]=json!("字".repeat(8192));unicode["link"]=json!(unicode_link);
        let publication=h.publish(&token,unicode).await?;
        let saved:(String,String,String)=sqlx::query_as("SELECT title,content,link FROM notifications WHERE publication_id=$1")
            .bind(id_of(&publication,"publicationId")?).fetch_one(&db.pool).await?;
        ensure!(saved.0.chars().count()==200 && saved.1.chars().count()==8192 && saved.2==unicode_link,
            "valid Unicode character boundaries or application link were rejected or altered");
        let bad_owner=sqlx::query("INSERT INTO notifications(user_id,title,source_kind,tenant_id,organization_id) VALUES($1,'wrong owner','SYSTEM',$2,$3)")
            .bind(h.recipient).bind(h.tenant_a).bind(h.remote).execute(&db.pool).await;
        ensure!(bad_owner.as_ref().err().and_then(|e|e.as_database_error()).and_then(|e|e.code()).as_deref()==Some("23503"), "notification organization/tenant FK accepted cross-tenant ownership");
        let partial=sqlx::query("INSERT INTO notifications(user_id,title,source_kind,tenant_id,organization_id,application_id) VALUES($1,'partial app','APP',$2,$3,$4)")
            .bind(h.recipient).bind(h.tenant_a).bind(h.root).bind(h.main_app).execute(&db.pool).await;
        ensure!(partial.as_ref().err().and_then(|e|e.as_database_error()).and_then(|e|e.code()).as_deref()==Some("23514"), "APP notification with missing publication was accepted or failed for an unrelated reason");
        sqlx::query("INSERT INTO users(username,password_hash) SELECT 'notification-cap-'||g,'unused' FROM generate_series(1,1001) AS g").execute(&db.pool).await?;
        sqlx::query("INSERT INTO memberships(user_id,organization_id) SELECT id,$1 FROM users WHERE username LIKE 'notification-cap-%'").bind(h.root).execute(&db.pool).await?;
        rejected(&h.router,Method::POST,&path,&token,Some(body("over-cap",org_target(h.root))),&[StatusCode::BAD_REQUEST]).await?;
        ensure!(h.counts("over-cap").await?==(0,0), "fanout cap truncated or partially published a notification");
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn notification_system_call_sites_preserve_context_and_domain_transactions() -> Result<()> {
    let db = Database::create(false).await?;
    let result:Result<()>=async {
        let h=Harness::create(&db.pool).await?;
        let menu:Uuid=sqlx::query_scalar("INSERT INTO menus(application_id,type,title,permission) VALUES($1,'BUTTON','Manage users','system:user') RETURNING id").bind(h.main_app).fetch_one(&db.pool).await?;
        sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)").bind(h.tenant_a).bind(menu).execute(&db.pool).await?;
        sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)").bind(h.root).bind(menu).execute(&db.pool).await?;
        sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)").bind(h.root_role).bind(menu).execute(&db.pool).await?;
        let created=ok(&h.router,Method::POST,"/api/users",&h.publisher_session.token,Some(json!({"username":"notification-created","password":"initial-test-password"}))).await?;
        let uid=id_of(&created,"id")?;
        let welcome:Vec<(Uuid,Uuid,String)>=sqlx::query_as("SELECT tenant_id,organization_id,source_kind FROM notifications WHERE user_id=$1").bind(uid).fetch_all(&db.pool).await?;
        ensure!(welcome==vec![(h.tenant_a,h.root,"SYSTEM".to_string())],"tenant creation did not produce exactly one contextual SYSTEM welcome");
        let global=ok(&h.router,Method::POST,"/api/users",&h.platform_session.token,Some(json!({"username":"notification-global-created","password":"initial-global-password"}))).await?;
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM notifications WHERE user_id=$1").bind(id_of(&global,"id")?).fetch_one(&db.pool).await?;
        ensure!(count==0,"global creation guessed ownership for a user without membership");
        let reset_session=session(&db.pool,uid,h.tenant_a,h.root,3600,3600).await?;
        let before:(String,i64)=sqlx::query_as("SELECT password_hash,session_version FROM users WHERE id=$1").bind(uid).fetch_one(&db.pool).await?;
        let reset_path=format!("/api/users/{uid}/reset-password");
        let roles_path=format!("/api/users/{}/roles",h.recipient);
        let role_body=json!({"items":[{"roleId":h.root_role,"organizationId":h.root}]});
        let device:Uuid=sqlx::query_scalar("INSERT INTO oauth_device_codes(device_code_hash,user_code,client_id,scope,expires_at) VALUES($1,'NOTIFY-DEVICE',$2,'profile',now()+interval '10 minutes') RETURNING id")
            .bind(hash::sha256_hex("notification-device-fixture")).bind(APP_CODE).fetch_one(&db.pool).await?;
        sqlx::query("CREATE FUNCTION notification_system_test_fail() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected SYSTEM failure'; END $$").execute(&db.pool).await?;
        sqlx::query("CREATE TRIGGER notification_system_test_failure BEFORE INSERT ON notifications FOR EACH ROW WHEN(NEW.source_kind='SYSTEM') EXECUTE FUNCTION notification_system_test_fail()").execute(&db.pool).await?;
        rejected(&h.router,Method::POST,"/api/users",&h.publisher_session.token,Some(json!({"username":"notification-rollback-created","password":"initial-test-password"})),&[StatusCode::INTERNAL_SERVER_ERROR]).await?;
        let partial_user:i64=sqlx::query_scalar("SELECT count(*) FROM users WHERE username='notification-rollback-created'").fetch_one(&db.pool).await?;
        ensure!(partial_user==0,"SYSTEM welcome failure committed account creation");
        rejected(&h.router,Method::POST,&reset_path,&h.publisher_session.token,Some(json!({"newPassword":"changed-test-password"})),&[StatusCode::INTERNAL_SERVER_ERROR]).await?;
        let unchanged:(String,i64)=sqlx::query_as("SELECT password_hash,session_version FROM users WHERE id=$1").bind(uid).fetch_one(&db.pool).await?;
        let live:bool=sqlx::query_scalar("SELECT revoked_at IS NULL FROM platform_sessions WHERE id=$1").bind(reset_session.id).fetch_one(&db.pool).await?;
        ensure!(before==unchanged && live,"SYSTEM failure committed reset/version/session invalidation");
        rejected(&h.router,Method::PUT,&roles_path,&h.platform_session.token,Some(role_body.clone()),&[StatusCode::INTERNAL_SERVER_ERROR]).await?;
        let partial_role:i64=sqlx::query_scalar("SELECT count(*) FROM user_roles WHERE user_id=$1").bind(h.recipient).fetch_one(&db.pool).await?;
        ensure!(partial_role==0,"SYSTEM failure committed role mutation");
        rejected(&h.router,Method::POST,"/api/device/NOTIFY-DEVICE/decision",&h.publisher_session.token,Some(json!({"approve":true})),&[StatusCode::INTERNAL_SERVER_ERROR]).await?;
        let pending:(String,Option<Uuid>)=sqlx::query_as("SELECT status,user_id FROM oauth_device_codes WHERE id=$1").bind(device).fetch_one(&db.pool).await?;
        ensure!(pending==("PENDING".to_string(),None),"SYSTEM failure committed device approval");
        sqlx::query("DROP TRIGGER notification_system_test_failure ON notifications").execute(&db.pool).await?;
        sqlx::query("DROP FUNCTION notification_system_test_fail()").execute(&db.pool).await?;
        ok(&h.router,Method::POST,&reset_path,&h.publisher_session.token,Some(json!({"newPassword":"changed-test-password"}))).await?;
        let changed:(String,i64)=sqlx::query_as("SELECT password_hash,session_version FROM users WHERE id=$1").bind(uid).fetch_one(&db.pool).await?;
        let revoked:bool=sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM platform_sessions WHERE id=$1").bind(reset_session.id).fetch_one(&db.pool).await?;
        ensure!(changed.0!=before.0 && changed.1==before.1+1 && revoked,"successful reset retry was not atomic");
        ok(&h.router,Method::PUT,&roles_path,&h.platform_session.token,Some(role_body)).await?;
        ok(&h.router,Method::POST,"/api/device/NOTIFY-DEVICE/decision",&h.publisher_session.token,Some(json!({"approve":true}))).await?;
        let approved:String=sqlx::query_scalar("SELECT status FROM oauth_device_codes WHERE id=$1").bind(device).fetch_one(&db.pool).await?;
        ensure!(approved=="APPROVED","device retry did not commit");
        sqlx::query("UPDATE organizations SET expires_at=now()-interval '1 hour' WHERE id=$1").bind(h.child).execute(&db.pool).await?;
        sqlx::query("UPDATE tenants SET status='DISABLED' WHERE id=$1").bind(h.tenant_b).execute(&db.pool).await?;
        ok(&h.router,Method::POST,&format!("/api/users/{}/reset-password",h.recipient),&h.platform_session.token,Some(json!({"newPassword":"global-changed-password"}))).await?;
        let recipients:Vec<(Uuid,Uuid,String)>=sqlx::query_as("SELECT tenant_id,organization_id,source_kind FROM notifications WHERE user_id=$1 AND title='密码已被管理员重置'").bind(h.recipient).fetch_all(&db.pool).await?;
        ensure!(recipients==vec![(h.tenant_a,h.root,"SYSTEM".to_string())],"global reset sent into inactive or expired recipient contexts");
        let legacy:i64=sqlx::query_scalar("SELECT count(*) FROM notifications WHERE source_kind='LEGACY'").fetch_one(&db.pool).await?;
        ensure!(legacy==0,"new SYSTEM call sites created LEGACY rows");
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn notification_seed_does_not_create_new_unowned_messages() -> Result<()> {
    let db = Database::create(false).await?;
    let result: Result<()> = async {
        crate::seed::run(&db.pool).await?;
        let before: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications")
            .fetch_one(&db.pool)
            .await?;
        let legacy: i64 =
            sqlx::query_scalar("SELECT count(*) FROM notifications WHERE source_kind='LEGACY'")
                .fetch_one(&db.pool)
                .await?;
        ensure!(legacy == 0, "seed created new unowned LEGACY notifications");
        crate::seed::run(&db.pool).await?;
        let after: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications")
            .fetch_one(&db.pool)
            .await?;
        ensure!(before == after, "repeated seed duplicated notifications");
        Ok(())
    }
    .await;
    db.finish(result).await
}

// Phase 11 reuses the isolated context/app-token fixtures above; no platform directory
// data can be fetched solely because a user holds a role or enters an application.
async fn app_members_setup(h: &Harness) -> Result<Vec<Uuid>> {
    let mut menus = Vec::new();
    for action in ["create", "update"] {
        let menu: Uuid = sqlx::query_scalar("INSERT INTO menus(application_id,type,title,permission) VALUES($1,'BUTTON',$2,$3) RETURNING id")
            .bind(h.main_app).bind(format!("工单 {action}")).bind(format!("ticket:{action}")).fetch_one(&h.pool).await?;
        sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
            .bind(h.tenant_a)
            .bind(menu)
            .execute(&h.pool)
            .await?;
        for org in [h.root, h.child, h.peer] {
            sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
                .bind(org)
                .bind(menu)
                .execute(&h.pool)
                .await?;
        }
        sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)")
            .bind(h.root_role)
            .bind(menu)
            .execute(&h.pool)
            .await?;
        menus.push(menu);
    }
    Ok(menus)
}

async fn app_members_grant(h: &Harness, org: Uuid, action: &str, scope: &str) -> Result<()> {
    let changed = sqlx::query("INSERT INTO membership_data_grants(membership_id,application_id,resource_code,action_code,scope_type) SELECT id,$3,'ticket',$4,$5 FROM memberships WHERE user_id=$1 AND organization_id=$2 ON CONFLICT(membership_id,application_id,resource_code,action_code) DO UPDATE SET scope_type=EXCLUDED.scope_type")
        .bind(h.publisher).bind(org).bind(h.main_app).bind(action).bind(scope).execute(&h.pool).await?.rows_affected();
    ensure!(changed == 1, "ticket grant needs direct membership");
    Ok(())
}

fn app_members_path(org: Uuid, action: &str, user: Option<Uuid>) -> String {
    let suffix = user.map(|id| format!("/{id}")).unwrap_or_default();
    format!(
        "/api/apps/{APP_CODE}/members{suffix}?resource=ticket&action={action}&organizationId={org}"
    )
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn app_members_scopes_minimal_dto_and_recipient_validity() -> Result<()> {
    let db = Database::create(false).await?;
    let result:Result<()>=async {
        let h=Harness::create(&db.pool).await?;
        app_members_setup(&h).await?;
        let token=h.app_token(&h.publisher_session.token,APP_CODE).await?;
        let root_path=app_members_path(h.root,"create",None);
        rejected(&h.router,Method::GET,&root_path,&token,None,&[StatusCode::FORBIDDEN]).await?;
        for scope in ["SELF","ORG","ORG_SUBTREE","TENANT"] {
            app_members_grant(&h,h.root,"create",scope).await?;
            let list=ok(&h.router,Method::GET,&root_path,&token,None).await?;
            ensure!(list["total"]==4,"candidate list did not include exactly enabled direct members");
            for item in list["records"].as_array().ok_or_else(||anyhow!("missing members"))? {
                let keys:std::collections::BTreeSet<_>=item.as_object().ok_or_else(||anyhow!("invalid member"))?.keys().map(String::as_str).collect();
                ensure!(keys==["displayName","id","organizationId"].into_iter().collect(),"member helper leaked private profile fields");
                ensure!(id_of(item,"organizationId")?==h.root,"candidate row has wrong organization");
            }
            let member=ok(&h.router,Method::GET,&app_members_path(h.root,"create",Some(h.recipient)),&token,None).await?;
            ensure!(id_of(&member,"id")?==h.recipient,"minimal single-member validation returned another identity");
            rejected(&h.router,Method::GET,&app_members_path(h.root,"create",Some(h.disabled_user)),&token,None,&[StatusCode::NOT_FOUND]).await?;
            rejected(&h.router,Method::GET,&app_members_path(h.root,"create",Some(h.child_user)),&token,None,&[StatusCode::NOT_FOUND]).await?;
            let child_path=app_members_path(h.child,"create",None);
            if ["SELF","ORG"].contains(&scope) {
                rejected(&h.router,Method::GET,&child_path,&token,None,&[StatusCode::NOT_FOUND]).await?;
            } else { ok(&h.router,Method::GET,&child_path,&token,None).await?; }
            let peer_path=app_members_path(h.peer,"create",None);
            rejected(&h.router,Method::GET,&peer_path,&token,None,&[StatusCode::NOT_FOUND]).await?;
            rejected(&h.router,Method::GET,&app_members_path(h.remote,"create",None),&token,None,&[StatusCode::NOT_FOUND]).await?;
            rejected(&h.router,Method::GET,&app_members_path(h.no_app_org,"create",None),&token,None,&[StatusCode::NOT_FOUND]).await?;
        }
        // Inherited role permissions never supply the child direct membership's grant.
        let child_token=h.app_token(&h.child_session.token,APP_CODE).await?;
        rejected(&h.router,Method::GET,&app_members_path(h.child,"create",None),&child_token,None,&[StatusCode::FORBIDDEN]).await?;
        app_members_grant(&h,h.child,"create","SELF").await?;
        ok(&h.router,Method::GET,&app_members_path(h.child,"create",None),&child_token,None).await?;
        rejected(&h.router,Method::GET,&app_members_path(h.root,"create",None),&child_token,None,&[StatusCode::NOT_FOUND]).await?;
        rejected(&h.router,Method::GET,&app_members_path(h.root,"update",None),&token,None,&[StatusCode::FORBIDDEN]).await?;
        app_members_grant(&h,h.root,"update","ORG").await?;
        ok(&h.router,Method::GET,&app_members_path(h.root,"update",None),&token,None).await?;
        sqlx::query("UPDATE users SET real_name='处理人 % _ 字面量' WHERE id=$1").bind(h.recipient).execute(&db.pool).await?;
        let filtered=ok(&h.router,Method::GET,&format!("{root_path}&q=%25"),&token,None).await?;
        ensure!(filtered["total"]==1 && filtered["records"][0]["displayName"]=="处理人 % _ 字面量","member search treated wildcard as pattern");
        let page=ok(&h.router,Method::GET,&format!("{root_path}&page=2&size=2"),&token,None).await?;
        ensure!(page["total"]==4 && page["page"]==2 && page["size"]==2 && page["records"].as_array().unwrap().len()==2,"member pagination changed");
        sqlx::query("UPDATE organizations SET status='DISABLED' WHERE id=$1").bind(h.child).execute(&db.pool).await?;
        rejected(&h.router,Method::GET,&app_members_path(h.child,"create",None),&token,None,&[StatusCode::NOT_FOUND]).await?;
        sqlx::query("UPDATE organizations SET status='ENABLED',expires_at=now()-interval '1 second' WHERE id=$1").bind(h.child).execute(&db.pool).await?;
        rejected(&h.router,Method::GET,&app_members_path(h.child,"create",None),&token,None,&[StatusCode::NOT_FOUND]).await?;
        sqlx::query("UPDATE organizations SET expires_at=NULL WHERE id=$1").bind(h.child).execute(&db.pool).await?;
        sqlx::query("DELETE FROM organization_applications WHERE organization_id=$1 AND application_id=$2").bind(h.child).bind(h.main_app).execute(&db.pool).await?;
        rejected(&h.router,Method::GET,&app_members_path(h.child,"create",None),&token,None,&[StatusCode::NOT_FOUND]).await?;
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops an isolated PostgreSQL database using TEST_DATABASE_URL"]
async fn app_members_authorization_and_next_request_revocation() -> Result<()> {
    let db = Database::create(false).await?;
    let result:Result<()>=async {
        let h=Harness::create(&db.pool).await?;
        let menus=app_members_setup(&h).await?;
        app_members_grant(&h,h.root,"create","TENANT").await?;
        let token=h.app_token(&h.publisher_session.token,APP_CODE).await?;
        let path=app_members_path(h.root,"create",None);
        ok(&h.router,Method::GET,&path,&token,None).await?;
        rejected(&h.router,Method::GET,&path,&h.publisher_session.token,None,&[StatusCode::UNAUTHORIZED]).await?;
        let wrong=h.app_token(&h.publisher_session.token,OTHER_CODE).await?;
        rejected(&h.router,Method::GET,&path,&wrong,None,&[StatusCode::FORBIDDEN]).await?;
        for extra in ["&size=51","&page=0","&tenantId=forged","&userId=forged"] {
            rejected(&h.router,Method::GET,&format!("{path}{extra}"),&token,None,&[StatusCode::BAD_REQUEST]).await?;
        }
        for operation in ["read","export","publish"] {
            rejected(&h.router,Method::GET,&app_members_path(h.root,operation,None),&token,None,&[StatusCode::BAD_REQUEST]).await?;
        }
        rejected(&h.router,Method::GET,&path.replace("resource=ticket","resource=user"),&token,None,&[StatusCode::BAD_REQUEST]).await?;
        let super_token=h.app_token(&h.super_session.token,APP_CODE).await?;
        rejected(&h.router,Method::GET,&path,&super_token,None,&[StatusCode::FORBIDDEN]).await?;
        sqlx::query("DELETE FROM role_menus WHERE role_id=$1 AND menu_id=$2").bind(h.root_role).bind(menus[0]).execute(&db.pool).await?;
        rejected(&h.router,Method::GET,&path,&token,None,&[StatusCode::FORBIDDEN]).await?;
        sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)").bind(h.root_role).bind(menus[0]).execute(&db.pool).await?;
        sqlx::query("DELETE FROM membership_data_grants WHERE membership_id IN(SELECT id FROM memberships WHERE user_id=$1 AND organization_id=$2) AND application_id=$3 AND resource_code='ticket' AND action_code='create'").bind(h.publisher).bind(h.root).bind(h.main_app).execute(&db.pool).await?;
        rejected(&h.router,Method::GET,&path,&token,None,&[StatusCode::FORBIDDEN]).await?;
        app_members_grant(&h,h.root,"create","ORG").await?;
        ok(&h.router,Method::GET,&path,&token,None).await?;
        sqlx::query("UPDATE roles SET status='DISABLED' WHERE id=$1").bind(h.root_role).execute(&db.pool).await?;
        rejected(&h.router,Method::GET,&path,&token,None,&[StatusCode::FORBIDDEN]).await?;
        sqlx::query("UPDATE roles SET status='ENABLED' WHERE id=$1").bind(h.root_role).execute(&db.pool).await?;
        ok(&h.router,Method::GET,&path,&token,None).await?;
        sqlx::query("DELETE FROM memberships WHERE user_id=$1 AND organization_id=$2").bind(h.publisher).bind(h.root).execute(&db.pool).await?;
        rejected(&h.router,Method::GET,&path,&token,None,&[StatusCode::UNAUTHORIZED]).await?;
        Ok(())
    }.await;
    db.finish(result).await
}
