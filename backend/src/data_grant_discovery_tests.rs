//! Narrow grant-selector regressions on owned PostgreSQL databases.
//! Explicit TEST_DATABASE_URL is required; no private environment file is read.
use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use axum::{middleware, Router};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::config::Config;
use crate::entity_creation_tests::Database;
use crate::util::jwt;

const SECRET: &str = "owned-phase15-discovery-signing-only";
const READ: &str = "system:data-grant:read";
const WRITE: &str = "system:data-grant:write";

#[derive(Clone)]
struct Person {
    user: Uuid,
    membership: Uuid,
    tenant: Uuid,
    organization: Uuid,
    username: String,
}

impl Person {
    fn target(&self, application: Uuid) -> Value {
        json!({"tenantId":self.tenant,"organizationId":self.organization,"userId":self.user,
            "membershipId":self.membership,"applicationId":application})
    }
}

struct Actor {
    person: Person,
    token: String,
    session: Uuid,
    role: Uuid,
}

struct Reply {
    status: StatusCode,
    body: Value,
}

struct Harness {
    pool: PgPool,
    app: Router,
    policy: Value,
    application: Uuid,
    legacy_application: Uuid,
    hidden_application: Uuid,
    unavailable_application: Uuid,
    read_menu: Uuid,
    write_menu: Uuid,
    business: Uuid,
    business_root: Uuid,
    branch: Uuid,
    root_reader: Actor,
    root_writer: Actor,
    broad_actor: Actor,
    org_actor: Actor,
    platform_child_actor: Actor,
    alice: Person,
    bob: Person,
    sibling: Person,
    hidden_people: Vec<Person>,
}

fn config(policy: &Value) -> Result<Config> {
    Config::from_values(&HashMap::from([
        (
            "BOOTSTRAP_ADMIN_USERNAME".into(),
            "owned-fresh-bootstrap".into(),
        ),
        (
            "BOOTSTRAP_ADMIN_PASSWORD".into(),
            "Owned-fresh-bootstrap-password9!".into(),
        ),
        ("APPLICATION_ENVIRONMENT".into(), "development".into()),
        (
            "DATABASE_URL".into(),
            "postgres://unused/owned-discovery".into(),
        ),
        ("JWT_SECRET".into(), SECRET.into()),
        ("DATA_GRANT_POLICY".into(), policy.to_string()),
    ]))
}

async fn database() -> Result<Database> {
    ensure!(
        std::env::var("TEST_DATABASE_URL").is_ok_and(|value| !value.trim().is_empty()),
        "explicit isolated TEST_DATABASE_URL is required"
    );
    Database::create().await
}

async fn tenant(pool: &PgPool, code: &str, platform: bool) -> Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO tenants(name,code,is_platform,id) VALUES($1,$1,$2,$3) RETURNING id",
    )
    .bind(code)
    .bind(platform)
    .bind(crate::util::uuid7::new_uuid())
    .fetch_one(pool)
    .await?)
}

async fn organization(
    pool: &PgPool,
    tenant: Uuid,
    parent: Option<Uuid>,
    name: &str,
) -> Result<Uuid> {
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

async fn person(pool: &PgPool, tenant: Uuid, org: Uuid, username: &str) -> Result<Person> {
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users(username,real_name,password_hash,id) VALUES($1,$1,'unused-owned-discovery-password',$2) RETURNING id",
    )
    .bind(username)
    .bind(crate::util::uuid7::new_uuid()).fetch_one(pool)
    .await?;
    let membership = sqlx::query_scalar(
        "INSERT INTO memberships(user_id,organization_id,id) VALUES($1,$2,$3) RETURNING id",
    )
    .bind(user)
    .bind(org)
    .bind(crate::util::uuid7::new_uuid())
    .fetch_one(pool)
    .await?;
    Ok(Person {
        user,
        membership,
        tenant,
        organization: org,
        username: username.into(),
    })
}

async fn actor(pool: &PgPool, person: Person, menus: &[Uuid]) -> Result<Actor> {
    let role: Uuid = sqlx::query_scalar(
        "INSERT INTO roles(tenant_id,organization_id,name,id) VALUES($1,$2,$3,$4) RETURNING id",
    )
    .bind(person.tenant)
    .bind(person.organization)
    .bind(format!(
        "owned-discovery-{}",
        crate::util::uuid7::new_uuid()
    ))
    .bind(crate::util::uuid7::new_uuid())
    .fetch_one(pool)
    .await?;
    sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3)")
        .bind(person.user)
        .bind(role)
        .bind(person.organization)
        .execute(pool)
        .await?;
    for menu in menus {
        sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)")
            .bind(role)
            .bind(menu)
            .execute(pool)
            .await?;
    }
    let session =
        crate::sso_test_fixture::session(pool, person.user, person.tenant, person.organization)
            .await?;
    let version = sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
        .bind(person.user)
        .fetch_one(pool)
        .await?;
    let now = chrono::Utc::now().timestamp();
    let token = jwt::sign_internal(
        &jwt::Claims {
            sub: person.user.to_string(),
            typ: "platform_session".into(),
            tenant: Some(person.tenant.to_string()),
            org: Some(person.organization.to_string()),
            sid: Some(session.to_string()),
            ver: Some(version),
            cid: None,
            scope: None,
            aud: Some(jwt::PLATFORM_API_AUDIENCE.into()),
            jti: None,
            iat: now,
            exp: now + 3600,
        },
        jwt::InternalUse::PlatformSession,
        SECRET,
    )?;
    Ok(Actor {
        person,
        token,
        session,
        role,
    })
}

async fn create_application(pool: &PgPool, code: &str) -> Result<Uuid> {
    Ok(
        sqlx::query_scalar("INSERT INTO applications(name,code,id) VALUES($1,$1,$2) RETURNING id")
            .bind(code)
            .bind(crate::util::uuid7::new_uuid())
            .fetch_one(pool)
            .await?,
    )
}

async fn grant_application(pool: &PgPool, tenant: Uuid, orgs: &[Uuid], app: Uuid) -> Result<()> {
    sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(tenant).bind(app).execute(pool).await?;
    for org in orgs {
        sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(org).bind(app).execute(pool).await?;
    }
    Ok(())
}

async fn request(app: &Router, token: &str, path: &str) -> Result<Reply> {
    let request_id = crate::util::uuid7::new_uuid();
    let request = Request::builder()
        .method(Method::GET)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header("x-request-id", request_id.to_string())
        .extension(ConnectInfo("192.0.2.15:31115".parse::<SocketAddr>()?))
        .body(Body::empty())?;
    let response = tokio::time::timeout(Duration::from_secs(40), app.clone().oneshot(request))
        .await
        .map_err(|_| anyhow!("owned discovery request timed out"))??;
    ensure!(
        response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            == Some(request_id.to_string().as_str()),
        "discovery response lost request correlation"
    );
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024).await?;
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok(Reply { status, body })
}

fn records(body: &Value) -> Result<&Vec<Value>> {
    body["records"]
        .as_array()
        .ok_or_else(|| anyhow!("discovery records are not an array"))
}

fn user_ids(body: &Value) -> Result<BTreeSet<Uuid>> {
    records(body)?
        .iter()
        .map(|record| {
            record["target"]["userId"]
                .as_str()
                .ok_or_else(|| anyhow!("discovery target omitted userId"))?
                .parse()
                .map_err(Into::into)
        })
        .collect()
}

fn application_ids(body: &Value) -> Result<BTreeSet<Uuid>> {
    body["applications"]
        .as_array()
        .ok_or_else(|| anyhow!("discovery applications are not an array"))?
        .iter()
        .map(|app| {
            app["id"]
                .as_str()
                .ok_or_else(|| anyhow!("application option omitted id"))?
                .parse()
                .map_err(Into::into)
        })
        .collect()
}

fn encoded(value: &str) -> String {
    let mut result = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            result.push(char::from(byte));
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    result
}

fn snapshot_path(target: &Value) -> Result<String> {
    let mut values = Vec::new();
    for key in [
        "tenantId",
        "organizationId",
        "userId",
        "membershipId",
        "applicationId",
    ] {
        let value = target[key]
            .as_str()
            .ok_or_else(|| anyhow!("incomplete discovery target"))?;
        values.push(format!("{key}={value}"));
    }
    Ok(format!("/api/data-grants?{}", values.join("&")))
}

enum Revocation {
    Read,
    Session,
}

async fn revoke_after_observed_actor_lock(
    h: &Harness,
    subject: &Actor,
    revocation: Revocation,
) -> Result<()> {
    let mut blocker = h.pool.begin().await?;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await?;
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(subject.person.user)
        .execute(&mut *blocker)
        .await?;
    let app = h.app.clone();
    let token = subject.token.clone();
    let pending =
        tokio::spawn(async move { request(&app, &token, "/api/data-grants/options").await });
    let observed = tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)) AND query LIKE '%FROM users WHERE id=$1 FOR NO KEY UPDATE%')")
                .bind(pid).fetch_one(&h.pool).await?;
            if waiting {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap_or_else(|_| Err(anyhow!("discovery did not reach the observed actor-user lock barrier")));
    if let Err(error) = observed {
        blocker.rollback().await?;
        let _ = pending.await;
        return Err(error);
    }
    let expected = match revocation {
        Revocation::Read => {
            sqlx::query("DELETE FROM role_menus WHERE role_id=$1 AND menu_id=$2")
                .bind(subject.role)
                .bind(h.read_menu)
                .execute(&mut *blocker)
                .await?;
            StatusCode::FORBIDDEN
        }
        Revocation::Session => {
            sqlx::query("UPDATE platform_sessions SET revoked_at=clock_timestamp() WHERE id=$1")
                .bind(subject.session)
                .execute(&mut *blocker)
                .await?;
            StatusCode::UNAUTHORIZED
        }
    };
    blocker.commit().await?;
    let response = pending.await??;
    ensure!(
        response.status == expected,
        "discovery reused extractor authority after an observed lock wait"
    );
    ensure!(
        response.body.get("applications").is_none() && response.body.get("records").is_none(),
        "revoked request returned collected discovery names"
    );
    Ok(())
}

impl Harness {
    async fn new(pool: &PgPool) -> Result<Self> {
        let platform = tenant(pool, "owned-discovery-platform", true).await?;
        let platform_root = organization(pool, platform, None, "Platform root").await?;
        let platform_child =
            organization(pool, platform, Some(platform_root), "Platform child").await?;
        let business = tenant(pool, "owned-discovery-business", false).await?;
        let business_root = organization(pool, business, None, "Business root").await?;
        let branch = organization(pool, business, Some(business_root), "Readable branch").await?;
        let leaf = organization(pool, business, Some(branch), "Readable descendant").await?;
        let sibling_org =
            organization(pool, business, Some(business_root), "Read-only sibling").await?;
        let outside = tenant(pool, "do-not-leak-foreign-tenant", false).await?;
        let outside_root = organization(pool, outside, None, "do-not-leak-foreign-org").await?;
        let expired_tenant = tenant(pool, "do-not-leak-inactive-tenant", false).await?;
        let expired_root = organization(
            pool,
            expired_tenant,
            None,
            "do-not-leak-inactive-tenant-org",
        )
        .await?;
        let inactive = organization(
            pool,
            business,
            Some(branch),
            "do-not-leak-inactive-ancestor",
        )
        .await?;
        let inactive_leaf = organization(
            pool,
            business,
            Some(inactive),
            "do-not-leak-inactive-descendant",
        )
        .await?;
        let expired_org =
            organization(pool, business, Some(branch), "do-not-leak-expired-org").await?;
        let system = create_application(pool, "owned-discovery-system").await?;
        let application = create_application(pool, "owned-discovery-tickets").await?;
        let legacy_application =
            create_application(pool, "owned-discovery-legacy-no-capabilities").await?;
        let hidden_application = create_application(pool, "do-not-leak-unconfigured-app").await?;
        let unavailable_application =
            create_application(pool, "do-not-leak-outside-only-app").await?;
        let mut menus = Vec::new();
        for permission in [
            READ,
            WRITE,
            "system:user",
            "system:application",
            "system:authorization",
        ] {
            let menu = sqlx::query_scalar("INSERT INTO menus(application_id,type,title,permission,visible,id) VALUES($1,'BUTTON',$2,$2,false,$3) RETURNING id")
                .bind(system).bind(permission).bind(crate::util::uuid7::new_uuid()).fetch_one(pool).await?;
            menus.push(menu);
        }
        for permission in ["ticket:read", "ticket:update"] {
            sqlx::query("INSERT INTO menus(application_id,type,title,permission,visible,id) VALUES($1,'BUTTON',$2,$2,false,$3)")
                .bind(application).bind(permission).bind(crate::util::uuid7::new_uuid()).execute(pool).await?;
        }
        for (tenant, orgs) in [
            (platform, vec![platform_root, platform_child]),
            (
                business,
                vec![
                    business_root,
                    branch,
                    leaf,
                    sibling_org,
                    inactive,
                    inactive_leaf,
                    expired_org,
                ],
            ),
            (outside, vec![outside_root]),
            (expired_tenant, vec![expired_root]),
        ] {
            grant_application(pool, tenant, &orgs, system).await?;
            grant_application(pool, tenant, &orgs, application).await?;
            for menu in &menus {
                sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
                    .bind(tenant)
                    .bind(menu)
                    .execute(pool)
                    .await?;
                for org in &orgs {
                    sqlx::query(
                        "INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)",
                    )
                    .bind(org)
                    .bind(menu)
                    .execute(pool)
                    .await?;
                }
            }
        }
        grant_application(pool, business, &[branch, leaf], legacy_application).await?;
        grant_application(
            pool,
            business,
            &[branch, leaf, sibling_org],
            hidden_application,
        )
        .await?;
        grant_application(pool, outside, &[outside_root], unavailable_application).await?;
        let root_reader = actor(
            pool,
            person(pool, platform, platform_root, "actor-root-reader").await?,
            &menus[..1],
        )
        .await?;
        let root_writer = actor(
            pool,
            person(pool, platform, platform_root, "actor-root-writer").await?,
            &menus[..2],
        )
        .await?;
        let broad_actor = actor(
            pool,
            person(pool, platform, platform_root, "actor-broad-management").await?,
            &menus[2..],
        )
        .await?;
        let org_actor = actor(
            pool,
            person(pool, business, branch, "actor-org-reader-writer").await?,
            &menus[..2],
        )
        .await?;
        let platform_child_actor = actor(
            pool,
            person(
                pool,
                platform,
                platform_child,
                "actor-platform-child-reader-writer",
            )
            .await?,
            &menus[..2],
        )
        .await?;
        let alice = person(pool, business, branch, "alpha-15").await?;
        let bob = person(pool, business, leaf, "beta-15").await?;
        let sibling = person(pool, business, sibling_org, "gamma-15").await?;
        let mut hidden_people = Vec::new();
        for (tenant, org, name) in [
            (business, business_root, "do-not-leak-root-user"),
            (outside, outside_root, "do-not-leak-foreign-user"),
            (
                business,
                inactive_leaf,
                "do-not-leak-inactive-ancestor-user",
            ),
            (business, expired_org, "do-not-leak-expired-org-user"),
            (
                expired_tenant,
                expired_root,
                "do-not-leak-inactive-tenant-user",
            ),
            (business, branch, "do-not-leak-disabled-user"),
            (business, branch, "do-not-leak-removed-membership-user"),
        ] {
            hidden_people.push(person(pool, tenant, org, name).await?);
        }
        sqlx::query("UPDATE organizations SET status='DISABLED' WHERE id=$1")
            .bind(inactive)
            .execute(pool)
            .await?;
        sqlx::query(
            "UPDATE organizations SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
        )
        .bind(expired_org)
        .execute(pool)
        .await?;
        sqlx::query(
            "UPDATE tenants SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
        )
        .bind(expired_tenant)
        .execute(pool)
        .await?;
        sqlx::query("UPDATE users SET status='DISABLED' WHERE id=$1")
            .bind(hidden_people[5].user)
            .execute(pool)
            .await?;
        sqlx::query("DELETE FROM memberships WHERE id=$1")
            .bind(hidden_people[6].membership)
            .execute(pool)
            .await?;
        let policy = json!({"applications":[
            {"applicationId":application,"capabilities":[
                {"resource":"ticket","action":"read","scopes":["SELF","ORG"]},
                {"resource":"ticket","action":"update","scopes":["ORG"]},
                {"resource":"ticket","action":"unregistered","scopes":["SELF"]}]},
            {"applicationId":legacy_application,"capabilities":[]},
            {"applicationId":unavailable_application,"capabilities":[
                {"resource":"stock","action":"read","scopes":["ORG"]}]}],
            "managementScopes":[
                {"tenantId":business,"organizationId":branch,"includeDescendants":true,"read":true,"write":true},
                {"tenantId":business,"organizationId":sibling_org,"includeDescendants":false,"read":true,"write":false},
                {"tenantId":platform,"organizationId":platform_child,"includeDescendants":true,"read":true,"write":true},
                {"tenantId":expired_tenant,"organizationId":expired_root,"includeDescendants":true,"read":true,"write":true}]});
        let state = crate::sso_test_fixture::state(pool.clone(), Arc::new(config(&policy)?));
        let app = crate::routes::api_router()
            .layer(middleware::from_fn_with_state(
                state.clone(),
                crate::observability::observe,
            ))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                crate::normalize_client_ip,
            ))
            .with_state(state);
        Ok(Self {
            pool: pool.clone(),
            app,
            policy,
            application,
            legacy_application,
            hidden_application,
            unavailable_application,
            read_menu: menus[0],
            write_menu: menus[1],
            business,
            business_root,
            branch,
            root_reader,
            root_writer,
            broad_actor,
            org_actor,
            platform_child_actor,
            alice,
            bob,
            sibling,
            hidden_people,
        })
    }

    async fn options(&self, token: &str, tail: &str) -> Result<Value> {
        let response =
            request(&self.app, token, &format!("/api/data-grants/options{tail}")).await?;
        ensure!(
            response.status == StatusCode::OK,
            "options returned {}",
            response.status
        );
        Ok(response.body)
    }

    async fn selected(&self, token: &str, person: &Person) -> Result<Value> {
        self.options(
            token,
            &format!("?applicationId={}&userId={}", self.application, person.user),
        )
        .await
    }

    fn readable_users(&self) -> BTreeSet<Uuid> {
        [
            self.alice.user,
            self.bob.user,
            self.sibling.user,
            self.org_actor.person.user,
            self.platform_child_actor.person.user,
        ]
        .into_iter()
        .collect()
    }
}

#[tokio::test]
#[ignore = "requires explicit TEST_DATABASE_URL; creates and removes owned PostgreSQL databases"]
async fn options_require_independent_read_and_never_leak_outside_or_inactive_targets() -> Result<()>
{
    let db = database().await?;
    let result = async {
        let h = Harness::new(&db.pool).await?;
        let permissions = request(&h.app, &h.root_reader.token, "/api/auth/permissions").await?;
        ensure!(
            permissions.status == StatusCode::OK
                && permissions.body["permissions"] == json!([READ]),
            "fixture reader acquired broad management rights"
        );
        for path in ["/api/users", "/api/applications"] {
            ensure!(
                request(&h.app, &h.root_reader.token, path).await?.status == StatusCode::FORBIDDEN,
                "grant read implicitly enabled broad discovery"
            );
        }
        let applications = h.options(&h.root_reader.token, "").await?;
        ensure!(
            application_ids(&applications)?
                == [h.application, h.legacy_application].into_iter().collect(),
            "application options leaked an unconfigured or outside-only application"
        );
        ensure!(
            records(&applications)?.is_empty() && applications["total"] == 0,
            "unselected application returned member records"
        );
        let selected = h
            .options(
                &h.root_reader.token,
                &format!("?applicationId={}&size=50", h.application),
            )
            .await?;
        ensure!(
            selected["total"] == 5 && user_ids(&selected)? == h.readable_users(),
            "root discovery lost a legitimate business target or included an unauthorized target"
        );
        let serialized = selected.to_string();
        for hidden in &h.hidden_people {
            ensure!(
                !serialized.contains(&hidden.username)
                    && !serialized.contains(&hidden.user.to_string())
                    && !serialized.contains(&hidden.membership.to_string()),
                "hidden member identity leaked through discovery"
            );
        }
        for id in [h.hidden_application, h.unavailable_application] {
            ensure!(
                !serialized.contains(&id.to_string()),
                "hidden application identifier leaked through discovery"
            );
        }
        ensure!(
            !serialized.contains("do-not-leak-"),
            "outside display labels leaked through discovery"
        );
        for record in records(&selected)? {
            let snapshot = request(
                &h.app,
                &h.root_reader.token,
                &snapshot_path(&record["target"])?,
            )
            .await?;
            ensure!(
                snapshot.status == StatusCode::OK && snapshot.body["target"] == record["target"],
                "discovered target was not accepted by the same Phase14 read boundary"
            );
            ensure!(
                snapshot.body["capabilities"]
                    .as_array()
                    .is_some_and(|caps| caps.len() == 2
                        && caps.iter().all(|cap| cap["action"] != "unregistered")),
                "snapshot capabilities exceeded actual app registration"
            );
            ensure!(
                record.get("passwordHash").is_none()
                    && record.get("email").is_none()
                    && record.get("phone").is_none()
                    && record.get("roles").is_none(),
                "selector disclosed unrelated user administration data"
            );
        }
        let scoped = h
            .options(
                &h.org_actor.token,
                &format!("?applicationId={}&size=50", h.application),
            )
            .await?;
        ensure!(
            scoped["total"] == 3
                && user_ids(&scoped)?
                    == [h.alice.user, h.bob.user, h.org_actor.person.user]
                        .into_iter()
                        .collect(),
            "organization read scope did not intersect the actual actor subtree"
        );
        let outside_user = h.selected(&h.org_actor.token, &h.sibling).await?;
        ensure!(
            outside_user["total"] == 0
                && records(&outside_user)?.is_empty()
                && application_ids(&outside_user)?.is_empty(),
            "userId filter disclosed a sibling outside actual read context"
        );
        sqlx::query(
            "UPDATE organizations SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
        )
        .bind(h.business_root)
        .execute(&h.pool)
        .await?;
        let expired_ancestor = h
            .options(
                &h.root_reader.token,
                &format!("?applicationId={}&size=50", h.application),
            )
            .await?;
        ensure!(
            expired_ancestor["total"] == 1
                && user_ids(&expired_ancestor)?
                    == [h.platform_child_actor.person.user].into_iter().collect(),
            "expired target ancestor remained discoverable"
        );
        ensure!(
            application_ids(&expired_ancestor)? == [h.application].into_iter().collect(),
            "application with no remaining valid member leaked into options"
        );
        sqlx::query("UPDATE organizations SET expires_at=NULL WHERE id=$1")
            .bind(h.business_root)
            .execute(&h.pool)
            .await?;
        sqlx::query("UPDATE tenants SET status='DISABLED' WHERE id=$1")
            .bind(h.business)
            .execute(&h.pool)
            .await?;
        let disabled_tenant = h
            .options(
                &h.root_reader.token,
                &format!("?applicationId={}", h.application),
            )
            .await?;
        ensure!(
            disabled_tenant["total"] == 1,
            "disabled tenant contributed selector records or totals"
        );
        sqlx::query("UPDATE tenants SET status='ENABLED' WHERE id=$1")
            .bind(h.business)
            .execute(&h.pool)
            .await?;
        sqlx::query("DELETE FROM memberships WHERE id=$1")
            .bind(h.alice.membership)
            .execute(&h.pool)
            .await?;
        let removed_member = h.selected(&h.root_reader.token, &h.alice).await?;
        ensure!(
            removed_member["total"] == 0 && application_ids(&removed_member)?.is_empty(),
            "removed direct member remained selectable"
        );
        sqlx::query("UPDATE applications SET status='DISABLED' WHERE id=$1")
            .bind(h.application)
            .execute(&h.pool)
            .await?;
        let disabled_app = h
            .options(
                &h.root_reader.token,
                &format!("?applicationId={}", h.application),
            )
            .await?;
        ensure!(
            disabled_app["total"] == 0
                && records(&disabled_app)?.is_empty()
                && application_ids(&disabled_app)? == [h.legacy_application].into_iter().collect(),
            "disabled application remained selectable"
        );
        eprintln!(
            "grant_discovery independent_read_policy_context_active_direct_members_no_leaks=PASS"
        );
        Ok(())
    }
    .await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit TEST_DATABASE_URL; creates and removes an owned PostgreSQL database"]
async fn options_strict_queries_literal_search_and_authorized_sql_pagination_are_truthful(
) -> Result<()> {
    let db = database().await?;
    let result = async {
        let h = Harness::new(&db.pool).await?;
        let base = format!("?applicationId={}", h.application);
        let defaults = h.options(&h.root_reader.token, &base).await?;
        ensure!(
            defaults["page"] == 1 && defaults["size"] == 20 && defaults["total"] == 5,
            "default pagination contract changed"
        );
        let all = records(&defaults)?.clone();
        let mut paged = Vec::new();
        for page in 1..=3 {
            let current = h
                .options(&h.root_reader.token, &format!("{base}&page={page}&size=2"))
                .await?;
            ensure!(
                current["total"] == 5 && current["page"] == page && current["size"] == 2,
                "page metadata was not canonical or authorized"
            );
            paged.extend(records(&current)?.iter().cloned());
        }
        ensure!(
            paged == all,
            "stable pagination duplicated, reordered, or lost a target"
        );
        let beyond = h
            .options(&h.root_reader.token, &format!("{base}&page=4&size=2"))
            .await?;
        ensure!(
            beyond["total"] == 5 && records(&beyond)?.is_empty(),
            "empty page falsified the authorized total"
        );
        let mut added = Vec::new();
        for name in [
            "literal%mark",
            "literalXmark",
            "literal_mark",
            "literalZmark",
            "literal\\mark",
            "literalNmark",
            "MixedCase",
        ] {
            added.push(person(&h.pool, h.business, h.branch, name).await?);
        }
        for (keyword, expected) in [
            ("%", added[0].user),
            ("_", added[2].user),
            ("\\", added[4].user),
            ("mIxEdCaSe", added[6].user),
            ("  alpha-15  ", h.alice.user),
        ] {
            let matched = h
                .options(
                    &h.root_reader.token,
                    &format!("{base}&keyword={}", encoded(keyword)),
                )
                .await?;
            ensure!(
                matched["total"] == 1 && user_ids(&matched)? == [expected].into_iter().collect(),
                "keyword was not a literal case-insensitive trimmed substring"
            );
        }
        sqlx::query("UPDATE users SET real_name='real-only search label' WHERE id=$1")
            .bind(h.bob.user)
            .execute(&h.pool)
            .await?;
        let display = h
            .options(&h.root_reader.token, &format!("{base}&keyword=REAL-only"))
            .await?;
        ensure!(
            display["total"] == 1 && user_ids(&display)? == [h.bob.user].into_iter().collect(),
            "displayName was not searchable"
        );
        let unmatched = h
            .options(
                &h.root_reader.token,
                &format!("{base}&keyword=no-such-member"),
            )
            .await?;
        ensure!(
            unmatched["total"] == 0
                && application_ids(&unmatched)?
                    == [h.application, h.legacy_application].into_iter().collect(),
            "keyword unexpectedly filtered the independent application selector"
        );
        let canonical = h
            .options(
                &h.root_reader.token,
                &format!("{base}&userId={}", h.alice.user.to_string().to_uppercase()),
            )
            .await?;
        ensure!(
            canonical["total"] == 1
                && records(&canonical)?[0]["target"] == h.alice.target(h.application),
            "userId filtering did not preserve the exact canonical five-UUID target"
        );
        let absent = h
            .options(
                &h.root_reader.token,
                &format!("{base}&userId={}", crate::util::uuid7::new_uuid()),
            )
            .await?;
        ensure!(
            absent["total"] == 0 && application_ids(&absent)?.is_empty(),
            "unknown user leaked application options or totals"
        );
        let nil = Uuid::nil();
        for tail in [
            format!("{base}&unknown=true"),
            format!("{base}&page=0"),
            format!("{base}&page=-1"),
            format!("{base}&page=1.5"),
            format!("{base}&page=1000001"),
            format!("{base}&size=0"),
            format!("{base}&size=51"),
            format!("{base}&size=-1"),
            format!("{base}&size=1&size=2"),
            format!("{base}&page=1&page=2"),
            format!("{base}&keyword=a&keyword=b"),
            format!("{base}&applicationId={}", h.application),
            format!("{base}&userId={}&userId={}", h.alice.user, h.bob.user),
            format!("?applicationId={nil}"),
            format!("{base}&userId={nil}"),
            "?applicationId=bad".into(),
            format!("{base}&userId="),
            format!("{base}&keyword={}", "a".repeat(101)),
            format!("{base}&keyword={}", encoded(&"😀".repeat(101))),
        ] {
            let response = request(
                &h.app,
                &h.root_reader.token,
                &format!("/api/data-grants/options{tail}"),
            )
            .await?;
            ensure!(
                response.status == StatusCode::BAD_REQUEST,
                "invalid discovery query was not rejected with 400"
            );
        }
        let maximum = h
            .options(
                &h.root_reader.token,
                &format!(
                    "{base}&page=1000000&size=50&keyword={}",
                    encoded(&"😀".repeat(100))
                ),
            )
            .await?;
        ensure!(
            maximum["page"] == 1000000 && maximum["size"] == 50 && maximum["total"] == 0,
            "valid query bounds were rejected or silently clamped"
        );
        // Sorting outside users first catches LIMIT-before-authorization and
        // bounded in-process scans which silently hide later allowed members.
        for index in 0..60 {
            person(
                &h.pool,
                h.business,
                h.business_root,
                &format!("aaa-outside-{index:03}"),
            )
            .await?;
        }
        let mut expected = h.readable_users();
        expected.extend(added.iter().map(|entry| entry.user));
        for index in 0..55 {
            expected.insert(
                person(
                    &h.pool,
                    h.business,
                    h.branch,
                    &format!("zzz-eligible-{index:03}"),
                )
                .await?
                .user,
            );
        }
        let mut actual = BTreeSet::new();
        for page in 1..=2 {
            let current = h
                .options(&h.root_reader.token, &format!("{base}&page={page}&size=50"))
                .await?;
            ensure!(
                current["total"].as_u64() == Some(expected.len() as u64),
                "discovery counted outside rows or truncated the authorized population"
            );
            actual.extend(user_ids(&current)?);
        }
        ensure!(
            actual == expected,
            "authorized SQL pagination lost valid members behind outside rows"
        );
        let outside_search = h
            .options(&h.root_reader.token, &format!("{base}&keyword=aaa-outside"))
            .await?;
        ensure!(
            outside_search["total"] == 0 && records(&outside_search)?.is_empty(),
            "search or total leaked outside-only users"
        );
        eprintln!(
            "grant_discovery strict_queries_literal_wildcards_exact_user_filter_and_sql_pages=PASS"
        );
        Ok(())
    }
    .await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit TEST_DATABASE_URL; creates and removes owned PostgreSQL databases"]
async fn options_write_hints_and_live_revocations_preserve_independent_permissions_and_seed_menu(
) -> Result<()> {
    let db = database().await?;
    let result = async {
        let h = Harness::new(&db.pool).await?;
        let reader = h.selected(&h.root_reader.token, &h.alice).await?;
        let writer = h.selected(&h.root_writer.token, &h.alice).await?;
        ensure!(records(&reader)?.len() == 1 && records(&reader)?[0]["canWrite"] == false, "independent read implied write");
        ensure!(records(&writer)?.len() == 1 && records(&writer)?[0]["canWrite"] == true, "actual root with explicit write boundary was not editable");
        let read_only_target = h.selected(&h.root_writer.token, &h.sibling).await?;
        ensure!(records(&read_only_target)?.len() == 1 && records(&read_only_target)?[0]["canWrite"] == false, "read-only policy boundary advertised write authority");
        let org = h.selected(&h.org_actor.token, &h.alice).await?;
        ensure!(records(&org)?.len() == 1 && records(&org)?[0]["canWrite"] == false, "non-platform organization admin advertised write authority despite the actual-root requirement");
        let child = h.selected(&h.platform_child_actor.token, &h.platform_child_actor.person).await?;
        ensure!(records(&child)?.len() == 1 && records(&child)?[0]["canWrite"] == false, "platform tenant child was mistaken for the actual platform root");
        ensure!(request(&h.app, &h.broad_actor.token, "/api/data-grants/options").await?.status == StatusCode::FORBIDDEN, "broad authorization/user/application permissions substituted for grant read");
        let legacy = h.options(&h.root_reader.token, &format!("?applicationId={}&userId={}", h.legacy_application, h.alice.user)).await?;
        ensure!(records(&legacy)?.len() == 1, "an explicitly readable app with no current capabilities became undiscoverable");
        let legacy_read = request(&h.app, &h.root_reader.token, &snapshot_path(&records(&legacy)?[0]["target"])?).await?;
        ensure!(legacy_read.status == StatusCode::OK && legacy_read.body["capabilities"] == json!([]), "legacy readable snapshot incorrectly required mutable capabilities");
        sqlx::query("DELETE FROM role_menus WHERE role_id=$1 AND menu_id=$2").bind(h.root_writer.role).bind(h.write_menu).execute(&h.pool).await?;
        let revoked_write = h.selected(&h.root_writer.token, &h.alice).await?;
        ensure!(records(&revoked_write)?[0]["canWrite"] == false, "write hint reused revoked role permission");
        revoke_after_observed_actor_lock(&h, &h.root_reader, Revocation::Read).await?;
        ensure!(request(&h.app, &h.root_reader.token, "/api/data-grants/options").await?.status == StatusCode::FORBIDDEN, "discovery reused revoked independent read permission");
        sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)").bind(h.root_reader.role).bind(h.read_menu).execute(&h.pool).await?;
        sqlx::query("DELETE FROM organization_applications WHERE organization_id=$1 AND application_id=$2").bind(h.branch).bind(h.application).execute(&h.pool).await?;
        let removed_app_grant = h.selected(&h.root_reader.token, &h.alice).await?;
        ensure!(removed_app_grant["total"] == 0 && records(&removed_app_grant)?.is_empty() && !application_ids(&removed_app_grant)?.contains(&h.application), "revoked target organization app grant remained in selectors");
        sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)").bind(h.branch).bind(h.application).execute(&h.pool).await?;
        revoke_after_observed_actor_lock(&h, &h.root_writer, Revocation::Session).await?;
        ensure!(request(&h.app, &h.root_writer.token, "/api/data-grants/options").await?.status == StatusCode::UNAUTHORIZED, "discovery reused a revoked platform session");
        sqlx::query("UPDATE users SET session_version=session_version+1 WHERE id=$1").bind(h.root_reader.person.user).execute(&h.pool).await?;
        ensure!(request(&h.app, &h.root_reader.token, "/api/data-grants/options").await?.status == StatusCode::UNAUTHORIZED, "discovery accepted an old session version");
        crate::seed::initialize(&h.pool, &config(&h.policy)?).await?;
        let backfilled: i64 = sqlx::query_scalar("SELECT count(*) FROM menus WHERE path='/system/data-grants'").fetch_one(&h.pool).await?;
        ensure!(backfilled == 0, "existing installation was implicitly granted a new page");
        eprintln!("grant_discovery actual_write_hint_role_and_session_revocations=PASS");
        Ok(())
    }.await;
    db.finish(result).await?;
    assert_fresh_seed_menu().await
}

async fn assert_fresh_seed_menu() -> Result<()> {
    let db = database().await?;
    let result = async {
        crate::seed::initialize(&db.pool, &config(&json!({"applications":[],"managementScopes":[]}))?).await?;
        let pages: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('id',id,'parentId',parent_id,'type',type,'path',path,'permission',permission,'visible',visible) FROM menus WHERE path='/system/data-grants'")
            .fetch_all(&db.pool).await?;
        ensure!(pages.len() == 1 && pages[0]["parentId"].is_null() && pages[0]["type"] == "MENU" && pages[0]["permission"] == READ && pages[0]["visible"] == true, "fresh grant page is not a distinct visible root MENU guarded by independent read");
        let page: Uuid = pages[0]["id"].as_str().ok_or_else(|| anyhow!("fresh page omitted id"))?.parse()?;
        let layers: Value = sqlx::query_scalar("SELECT jsonb_build_object('role',(SELECT count(*) FROM role_menus WHERE menu_id=$1),'tenant',(SELECT count(*) FROM tenant_menus WHERE menu_id=$1),'organization',(SELECT count(*) FROM organization_menus WHERE menu_id=$1),'outside',(SELECT count(*) FROM organization_menus om JOIN organizations o ON o.id=om.organization_id JOIN tenants t ON t.id=o.tenant_id WHERE om.menu_id=$1 AND (NOT t.is_platform OR o.parent_id IS NOT NULL)))")
            .bind(page).fetch_one(&db.pool).await?;
        ensure!(layers == json!({"role":1,"tenant":1,"organization":1,"outside":0}), "fresh page authorization escaped the explicit bootstrap three layers");
        let buttons: i64 = sqlx::query_scalar("SELECT count(*) FROM menus WHERE type='BUTTON' AND permission=ANY($1) AND visible=false")
            .bind([READ, WRITE]).fetch_one(&db.pool).await?;
        ensure!(buttons == 2, "visible page replaced the independent hidden read/write buttons");
        eprintln!("grant_discovery fresh_visible_independent_menu_three_layers_no_backfill=PASS");
        Ok(())
    }.await;
    db.finish(result).await
}
