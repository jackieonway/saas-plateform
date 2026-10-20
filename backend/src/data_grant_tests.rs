//! Controlled data-grant HTTP regressions on disposable PostgreSQL databases.
//! An explicit TEST_DATABASE_URL is mandatory; fixtures never read .env.
use std::collections::{BTreeMap, HashMap};
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
use crate::routes;
use crate::state::AppState;
use crate::util::jwt;

const SECRET: &str = "isolated-phase14-data-grant-regression-only";
const CLIENT_SECRET: &str = "owned-phase14-client-secret";
const ACTION: &str = "DATA_GRANT_CHANGE";
const CODE: &str = "phase14-tickets";

#[derive(Clone, Copy)]
struct Target {
    tenant: Uuid,
    org: Uuid,
    user: Uuid,
    membership: Uuid,
    application: Uuid,
}

impl Target {
    fn json(self) -> Value {
        json!({"tenantId":self.tenant,"organizationId":self.org,"userId":self.user,
            "membershipId":self.membership,"applicationId":self.application})
    }
    fn path(self) -> String {
        format!("/api/data-grants?tenantId={}&organizationId={}&userId={}&membershipId={}&applicationId={}",
            self.tenant, self.org, self.user, self.membership, self.application)
    }
    fn edit(self, version: &Value, upsert: Value, remove: Value) -> Value {
        json!({"target":self.json(),"expectedVersion":version,"upsert":upsert,"remove":remove})
    }
}

struct Reply {
    status: StatusCode,
    body: Value,
    request_id: Uuid,
}

#[derive(sqlx::FromRow)]
struct AuditRow {
    user_id: Uuid,
    tenant_id: Option<Uuid>,
    organization_id: Option<Uuid>,
    application_id: Option<Uuid>,
    detail: String,
}

struct Harness {
    pool: PgPool,
    app: Router,
    policy: Value,
    platform: Uuid,
    root: Uuid,
    child: Uuid,
    actor: Uuid,
    admin: String,
    plain: String,
    org_admin: String,
    reader: String,
    writer: String,
    business_session: String,
    read_menu: Uuid,
    write_menu: Uuid,
    create_menu: Uuid,
    update_menu: Uuid,
    writer_role: Uuid,
    target: Target,
    platform_target: Target,
    platform_root_target: Target,
    outside: Target,
    sibling: Uuid,
    descendant: Uuid,
    other_application: Uuid,
}

fn config(policy: &Value) -> Result<Config> {
    Config::from_values(&HashMap::from([
        ("APPLICATION_ENVIRONMENT".into(), "development".into()),
        (
            "DATABASE_URL".into(),
            "postgres://unused/isolated-data-grants".into(),
        ),
        ("JWT_SECRET".into(), SECRET.into()),
        ("DATA_GRANT_POLICY".into(), policy.to_string()),
    ]))
}

fn router(pool: &PgPool, policy: &Value) -> Result<Router> {
    let state = AppState::new(pool.clone(), Arc::new(config(policy)?));
    Ok(routes::api_router()
        .nest("/oauth", routes::oauth2::oauth_router())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::observability::observe,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::normalize_client_ip,
        ))
        .with_state(state))
}

async fn database() -> Result<Database> {
    ensure!(
        std::env::var("TEST_DATABASE_URL").is_ok_and(|v| !v.trim().is_empty()),
        "explicit isolated TEST_DATABASE_URL is required"
    );
    Database::create().await
}

async fn tenant(pool: &PgPool, code: &str, platform: bool) -> Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO tenants(name,code,is_platform) VALUES($1,$1,$2) RETURNING id",
    )
    .bind(code)
    .bind(platform)
    .fetch_one(pool)
    .await?)
}

async fn org(pool: &PgPool, tenant: Uuid, parent: Option<Uuid>, name: &str) -> Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO organizations(tenant_id,parent_id,name) VALUES($1,$2,$3) RETURNING id",
    )
    .bind(tenant)
    .bind(parent)
    .bind(name)
    .fetch_one(pool)
    .await?)
}

async fn user(pool: &PgPool, name: &str, super_admin: bool, org: Uuid) -> Result<(Uuid, Uuid)> {
    let user: Uuid = sqlx::query_scalar("INSERT INTO users(username,password_hash,is_super_admin,real_name) VALUES($1,'unused-fixture-password',$2,$1) RETURNING id")
        .bind(name).bind(super_admin).fetch_one(pool).await?;
    let membership = sqlx::query_scalar(
        "INSERT INTO memberships(user_id,organization_id) VALUES($1,$2) RETURNING id",
    )
    .bind(user)
    .bind(org)
    .fetch_one(pool)
    .await?;
    Ok((user, membership))
}

async fn session(pool: &PgPool, user: Uuid, tenant: Uuid, org: Uuid) -> Result<String> {
    let ver = sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
        .bind(user)
        .fetch_one(pool)
        .await?;
    let sid: Uuid = sqlx::query_scalar("INSERT INTO platform_sessions(user_id,tenant_id,organization_id,expires_at) VALUES($1,$2,$3,now()+interval '1 hour') RETURNING id")
        .bind(user).bind(tenant).bind(org).fetch_one(pool).await?;
    let now = chrono::Utc::now().timestamp();
    jwt::sign(
        &jwt::Claims {
            sub: user.to_string(),
            typ: "session".into(),
            tenant: Some(tenant.to_string()),
            org: Some(org.to_string()),
            sid: Some(sid.to_string()),
            ver: Some(ver),
            cid: None,
            scope: None,
            aud: None,
            jti: None,
            iat: now,
            exp: now + 3600,
        },
        SECRET,
    )
}

async fn role(pool: &PgPool, tenant: Uuid, org: Uuid, user: Uuid, menus: &[Uuid]) -> Result<Uuid> {
    let role: Uuid = sqlx::query_scalar(
        "INSERT INTO roles(tenant_id,organization_id,name) VALUES($1,$2,$3) RETURNING id",
    )
    .bind(tenant)
    .bind(org)
    .bind(format!("fixture-role-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await?;
    sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3)")
        .bind(user)
        .bind(role)
        .bind(org)
        .execute(pool)
        .await?;
    for menu in menus {
        sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)")
            .bind(role)
            .bind(menu)
            .execute(pool)
            .await?;
    }
    Ok(role)
}

async fn call(app: &Router, token: &str, method: Method, path: &str, body: Value) -> Result<Reply> {
    let request_id = Uuid::new_v4();
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-request-id", request_id.to_string())
        .extension(ConnectInfo("192.0.2.14:31014".parse::<SocketAddr>()?))
        .body(Body::from(serde_json::to_vec(&body)?))?;
    let response = tokio::time::timeout(Duration::from_secs(40), app.clone().oneshot(request))
        .await
        .map_err(|_| anyhow!("data grant HTTP request timed out"))??;
    ensure!(
        response
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            == Some(request_id.to_string().as_str()),
        "data grant response lost requestId"
    );
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024).await?;
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok(Reply {
        status,
        body,
        request_id,
    })
}

impl Harness {
    async fn new(pool: &PgPool) -> Result<Self> {
        let platform = tenant(pool, "phase14-platform", true).await?;
        let root = org(pool, platform, None, "Platform root").await?;
        let child = org(pool, platform, Some(root), "Platform child").await?;
        let business = tenant(pool, "phase14-business", false).await?;
        let business_root = org(pool, business, None, "Business root").await?;
        let business_child = org(pool, business, Some(business_root), "Business child").await?;
        let descendant = org(pool, business, Some(business_child), "Business descendant").await?;
        let sibling = org(pool, business, Some(business_root), "Business sibling").await?;
        let foreign = tenant(pool, "phase14-outside", false).await?;
        let foreign_root = org(pool, foreign, None, "Outside root").await?;
        let system: Uuid = sqlx::query_scalar("INSERT INTO applications(name,code,base_path) VALUES('Grant management','system','/system') RETURNING id")
            .fetch_one(pool).await?;
        let application: Uuid = sqlx::query_scalar("INSERT INTO applications(name,code,base_path,entry_type) VALUES('Grant tickets',$1,'/micro/phase14','HTML') RETURNING id")
            .bind(CODE).fetch_one(pool).await?;
        let other_application: Uuid = sqlx::query_scalar(
            "INSERT INTO applications(name,code) VALUES('Other app','phase14-other') RETURNING id",
        )
        .fetch_one(pool)
        .await?;
        let mut menus = Vec::new();
        for (app, permission) in [
            (system, "system:data-grant:read"),
            (system, "system:data-grant:write"),
            (system, "system:user"),
            (system, "system:role"),
            (system, "system:menu"),
            (application, "ticket:create"),
            (application, "ticket:update"),
            (other_application, "ticket:create"),
        ] {
            let id: Uuid = sqlx::query_scalar("INSERT INTO menus(application_id,type,title,permission,visible) VALUES($1,'BUTTON',$2,$2,false) RETURNING id")
                .bind(app).bind(permission).fetch_one(pool).await?;
            menus.push(id);
        }
        for (tenant, orgs) in [
            (platform, vec![root, child]),
            (
                business,
                vec![business_root, business_child, descendant, sibling],
            ),
            (foreign, vec![foreign_root]),
        ] {
            for app in [system, application, other_application] {
                sqlx::query(
                    "INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)",
                )
                .bind(tenant)
                .bind(app)
                .execute(pool)
                .await?;
                for org in &orgs {
                    sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)")
                        .bind(org).bind(app).execute(pool).await?;
                }
            }
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
        let (actor, _) = user(pool, "phase14-admin", true, root).await?;
        let (plain, _) = user(pool, "phase14-broad-admin", false, root).await?;
        role(pool, platform, root, plain, &menus[2..5]).await?;
        // Business scope never supplies delegation authority or limits it.
        for (subject, scope) in [(actor, "SELF"), (plain, "TENANT")] {
            sqlx::query("INSERT INTO membership_data_grants(membership_id,application_id,resource_code,action_code,scope_type) SELECT id,$2,'ticket','create',$3 FROM memberships WHERE user_id=$1 AND organization_id=$4")
                .bind(subject).bind(application).bind(scope).bind(root).execute(pool).await?;
        }
        sqlx::query("INSERT INTO oauth_clients(client_id,client_name,client_secret_hash,application_id,client_type,grant_types,scopes) VALUES('phase14-business-client','Owned data grant client',$1,$2,'CONFIDENTIAL','password,refresh_token','profile')")
            .bind(crate::util::hash::sha256_hex(CLIENT_SECRET))
            .bind(application).execute(pool).await?;
        let (org_admin, _) = user(pool, "phase14-org-admin", false, child).await?;
        role(pool, platform, child, org_admin, &menus[..5]).await?;
        let (reader, _) = user(pool, "phase14-reader", false, child).await?;
        role(pool, platform, child, reader, &menus[..1]).await?;
        let (writer, _) = user(pool, "phase14-writer", false, root).await?;
        let writer_role = role(pool, platform, root, writer, &menus[1..2]).await?;
        let (business_user, membership) =
            user(pool, "phase14-business-user", false, business_child).await?;
        role(pool, business, business_child, business_user, &menus[5..7]).await?;
        user(pool, "phase14-assignee", false, business_child).await?;
        user(pool, "phase14-descendant-assignee", false, descendant).await?;
        let (platform_user, platform_member) =
            user(pool, "phase14-platform-member", false, child).await?;
        let (root_user, root_member) =
            user(pool, "phase14-platform-root-member", false, root).await?;
        let (outside_user, outside_member) =
            user(pool, "phase14-outside-user", false, foreign_root).await?;
        let policy = json!({"applications":[{"applicationId":application,"capabilities":[
            {"resource":"ticket","action":"create","scopes":["SELF","ORG","ORG_SUBTREE","TENANT"]},
            {"resource":"ticket","action":"update","scopes":["ORG"]}],
            "memberLookups":[{"resource":"ticket","action":"create"},{"resource":"ticket","action":"update"}]}],
            "managementScopes":[
                {"tenantId":business,"organizationId":business_child,"includeDescendants":true,"read":true,"write":true},
                {"tenantId":platform,"organizationId":root,"includeDescendants":true,"read":true,"write":true}]});
        let app = router(pool, &policy)?;
        let admin = session(pool, actor, platform, root).await?;
        let plain = session(pool, plain, platform, root).await?;
        let org_admin = session(pool, org_admin, platform, child).await?;
        let reader = session(pool, reader, platform, child).await?;
        let writer = session(pool, writer, platform, root).await?;
        let business_session = session(pool, business_user, business, business_child).await?;
        sqlx::raw_sql("CREATE TABLE phase14_audit_fault(mode text NOT NULL); \
            CREATE FUNCTION phase14_reject_audit() RETURNS trigger LANGUAGE plpgsql AS $$ \
            DECLARE fault_mode text; BEGIN \
              IF NEW.action<>'DATA_GRANT_CHANGE' THEN RETURN NEW; END IF; \
              SELECT mode INTO fault_mode FROM phase14_audit_fault LIMIT 1; \
              IF fault_mode='raise' THEN RAISE EXCEPTION 'owned data grant audit fault' USING ERRCODE='P0001'; END IF; \
              IF fault_mode='skip' THEN RETURN NULL; END IF; RETURN NEW; END $$; \
            CREATE TRIGGER phase14_audit_fault BEFORE INSERT ON audit_logs FOR EACH ROW EXECUTE FUNCTION phase14_reject_audit();")
            .execute(pool).await?;
        Ok(Self {
            pool: pool.clone(),
            app,
            policy,
            platform,
            root,
            child,
            actor,
            admin,
            plain,
            org_admin,
            reader,
            writer,
            business_session,
            read_menu: menus[0],
            write_menu: menus[1],
            create_menu: menus[5],
            update_menu: menus[6],
            writer_role,
            target: Target {
                tenant: business,
                org: business_child,
                user: business_user,
                membership,
                application,
            },
            platform_target: Target {
                tenant: platform,
                org: child,
                user: platform_user,
                membership: platform_member,
                application,
            },
            platform_root_target: Target {
                tenant: platform,
                org: root,
                user: root_user,
                membership: root_member,
                application,
            },
            outside: Target {
                tenant: foreign,
                org: foreign_root,
                user: outside_user,
                membership: outside_member,
                application,
            },
            sibling,
            descendant,
            other_application,
        })
    }

    async fn get(&self, target: Target) -> Result<Value> {
        let response = call(
            &self.app,
            &self.admin,
            Method::GET,
            &target.path(),
            Value::Null,
        )
        .await?;
        ensure!(
            response.status == StatusCode::OK,
            "grant GET returned {}",
            response.status
        );
        ensure!(
            response.body["target"] == target.json() && response.body["version"].is_string(),
            "grant snapshot omitted target or version"
        );
        Ok(response.body)
    }

    async fn patch(
        &self,
        target: Target,
        version: &Value,
        upsert: Value,
        remove: Value,
    ) -> Result<Reply> {
        call(
            &self.app,
            &self.admin,
            Method::PATCH,
            "/api/data-grants",
            target.edit(version, upsert, remove),
        )
        .await
    }

    async fn snapshot(&self) -> Result<BTreeMap<String, String>> {
        let tables: Vec<String> = sqlx::query_scalar("SELECT tablename FROM pg_tables WHERE schemaname='public' AND tablename NOT LIKE 'phase14_%' AND tablename<>'_sqlx_migrations' ORDER BY tablename")
            .fetch_all(&self.pool).await?;
        let mut snapshot = BTreeMap::new();
        for table in tables {
            ensure!(
                table
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
                "unexpected table identifier"
            );
            let rows=sqlx::query_scalar(&format!("SELECT COALESCE(jsonb_agg(to_jsonb(r) ORDER BY to_jsonb(r)::text),'[]'::jsonb)::text FROM \"{table}\" r"))
                .fetch_one(&self.pool).await?;
            snapshot.insert(table, rows);
        }
        Ok(snapshot)
    }

    async fn assert_denied(
        &self,
        token: &str,
        target: Target,
        expected: &[StatusCode],
    ) -> Result<()> {
        let before = self.snapshot().await?;
        let response = call(&self.app, token, Method::GET, &target.path(), Value::Null).await?;
        ensure!(
            expected.contains(&response.status),
            "denied GET returned {}",
            response.status
        );
        let body = target.edit(
            &json!("0".repeat(64)),
            json!([{"resource":"ticket","action":"create","scope":"ORG"}]),
            json!([]),
        );
        for method in [Method::POST, Method::PATCH] {
            let path = if method == Method::POST {
                "/api/data-grants/preview"
            } else {
                "/api/data-grants"
            };
            let response = call(&self.app, token, method, path, body.clone()).await?;
            ensure!(
                expected.contains(&response.status),
                "denied mutation/preview returned {}",
                response.status
            );
        }
        ensure!(
            self.snapshot().await? == before,
            "denied request changed database state"
        );
        Ok(())
    }

    async fn audit(&self, response: &Reply, target: Target) -> Result<()> {
        let rows: Vec<AuditRow> = sqlx::query_as(
            "SELECT user_id,tenant_id,organization_id,application_id,detail FROM audit_logs WHERE action=$1 AND request_id=$2")
            .bind(ACTION).bind(response.request_id).fetch_all(&self.pool).await?;
        ensure!(
            rows.len() == 1,
            "data grant mutation did not write exactly one request-bound success audit"
        );
        let row = &rows[0];
        ensure!(
            row.user_id == self.actor
                && row.tenant_id == Some(target.tenant)
                && row.organization_id == Some(target.org)
                && row.application_id == Some(target.application),
            "data grant audit lost actor/target/requestId binding"
        );
        ensure!(
            !row.detail.contains(SECRET)
                && !row.detail.contains(&self.admin)
                && !row.detail.contains("password")
                && !row.detail.contains(CLIENT_SECRET)
                && !row
                    .detail
                    .contains(&crate::util::hash::sha256_hex(CLIENT_SECRET)),
            "data grant audit leaked credentials"
        );
        let detail: Value = serde_json::from_str(&row.detail)?;
        ensure!(
            detail["before"]["target"] == target.json()
                && detail["after"]["target"] == target.json(),
            "data grant audit omitted exact member/user/application target snapshots"
        );
        Ok(())
    }
}

fn grant<'a>(snapshot: &'a Value, action: &str) -> Result<&'a Value> {
    snapshot["grants"]
        .as_array()
        .and_then(|rows| {
            rows.iter()
                .find(|row| row["resource"] == "ticket" && row["action"] == action)
        })
        .ok_or_else(|| anyhow!("expected ticket grant missing"))
}

fn upsert(action: &str, scope: &str) -> Value {
    json!([{"resource":"ticket","action":action,"scope":scope}])
}
fn remove(action: &str) -> Value {
    json!([{"resource":"ticket","action":action}])
}

#[tokio::test]
#[ignore = "requires explicit isolated TEST_DATABASE_URL and owned database privileges"]
async fn independent_permissions_platform_root_and_read_scope_are_enforced() -> Result<()> {
    let db = database().await?;
    let result=async {
        let h=Harness::new(&db.pool).await?;
        h.assert_denied(&h.plain,h.target,&[StatusCode::FORBIDDEN]).await?;
        let discover=format!("/api/users/{}/memberships",h.target.user);
        let discovered=call(&h.app,&h.admin,Method::GET,&discover,Value::Null).await?;
        ensure!(discovered.status==StatusCode::OK&&discovered.body.as_array().is_some_and(|rows|rows.iter().any(|row|row["membershipId"]==h.target.membership.to_string()&&row["organizationId"]==h.target.org.to_string())),"formal membership discovery omitted the direct membership UUID");
        let forbidden=call(&h.app,&h.writer,Method::GET,&discover,Value::Null).await?;
        ensure!(forbidden.status==StatusCode::FORBIDDEN,"grant permission implicitly granted user management/discovery");
        let initial=h.get(h.target).await?;
        let body=h.target.edit(&initial["version"],upsert("create","ORG"),json!([]));
        for token in [&h.org_admin,&h.reader] {
            for (method,path) in [(Method::PATCH,"/api/data-grants"),(Method::POST,"/api/data-grants/preview")] {
                let response=call(&h.app,token,method,path,body.clone()).await?;
                ensure!(response.status==StatusCode::FORBIDDEN,"nonroot actor forged data grant mutation: {}",response.status);
            }
        }
        let read=call(&h.app,&h.reader,Method::GET,&h.platform_target.path(),Value::Null).await?;
        ensure!(read.status==StatusCode::OK,"read-only actor cannot read permitted actual subtree");
        for target in [h.platform_root_target,h.target,h.outside] {
            let reply=call(&h.app,&h.reader,Method::GET,&target.path(),Value::Null).await?;
            ensure!([StatusCode::FORBIDDEN,StatusCode::NOT_FOUND].contains(&reply.status),"reader escaped actual session subtree");
        }
        let write_get=call(&h.app,&h.writer,Method::GET,&h.target.path(),Value::Null).await?;
        let preview=call(&h.app,&h.writer,Method::POST,"/api/data-grants/preview",body.clone()).await?;
        ensure!(write_get.status==StatusCode::FORBIDDEN&&preview.status==StatusCode::FORBIDDEN,"write permission implied read");
        let write=call(&h.app,&h.writer,Method::PATCH,"/api/data-grants",body).await?;
        ensure!(write.status==StatusCode::OK,"independent write permission without read was rejected");
        // Even global administrators need the selected organization's explicit menu ceiling.
        sqlx::query("DELETE FROM organization_menus WHERE organization_id=$1 AND menu_id=ANY($2)")
            .bind(h.root).bind(vec![h.read_menu,h.write_menu]).execute(&h.pool).await?;
        h.assert_denied(&h.admin,h.target,&[StatusCode::FORBIDDEN]).await?;
        // Existing installations are never silently backfilled by development startup.
        let before=h.snapshot().await?;
        crate::seed::initialize(&h.pool,&config(&h.policy)?).await?;
        ensure!(h.snapshot().await?==before,"nonempty startup backfilled grant-management permissions");
        eprintln!("data_grant_permissions independent=PASS root_ceiling=PASS readonly_subtree=PASS no_backfill=PASS");
        Ok(())
    }.await;
    db.finish(result).await?;
    assert_fresh_seed_permissions().await
}

async fn assert_fresh_seed_permissions() -> Result<()> {
    let db = database().await?;
    let result=async {
        crate::seed::initialize(&db.pool,&config(&json!({"applications":[],"managementScopes":[]}))?).await?;
        let rows: Vec<(Uuid,String,String,bool,String)> = sqlx::query_as("SELECT m.id,m.permission,m.type,m.visible,p.permission FROM menus m JOIN menus p ON p.id=m.parent_id JOIN applications a ON a.id=m.application_id WHERE a.code='system' AND m.type='BUTTON' AND m.permission IN('system:data-grant:read','system:data-grant:write') ORDER BY m.permission")
            .fetch_all(&db.pool).await?;
        ensure!(rows.len()==2 && rows[0].0!=rows[1].0 && rows.iter().all(|row|row.2=="BUTTON"&&!row.3&&row.4=="system:authorization"),"fresh seed grant permissions are not distinct hidden authorization buttons");
        for row in rows {
            let layers: (i64,i64,i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM role_menus rm JOIN roles r ON r.id=rm.role_id WHERE rm.menu_id=$1 AND r.code='super_admin'),(SELECT count(*) FROM tenant_menus WHERE menu_id=$1),(SELECT count(*) FROM organization_menus WHERE menu_id=$1),(SELECT count(*) FROM role_menus rm JOIN roles r ON r.id=rm.role_id WHERE rm.menu_id=$1 AND r.code IN('org_admin','common_user','star_admin'))")
                .bind(row.0).fetch_one(&db.pool).await?;
            ensure!(layers==(1,1,1,0),"fresh seed granted management permission beyond platform bootstrap role/root");
        }
        eprintln!("data_grant_permissions fresh_seed_independent_buttons_and_three_grant_layers=PASS");
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit isolated TEST_DATABASE_URL and owned database privileges"]
async fn target_membership_application_capability_and_policy_boundaries_are_checked() -> Result<()>
{
    let db = database().await?;
    let result=async {
        let h=Harness::new(&db.pool).await?;
        let denied=[StatusCode::FORBIDDEN,StatusCode::NOT_FOUND,StatusCode::BAD_REQUEST];
        for target in [h.outside,Target{tenant:h.platform,..h.target},Target{org:h.child,..h.target},
            Target{user:h.actor,..h.target},Target{membership:Uuid::new_v4(),..h.target},
            Target{application:h.other_application,..h.target}] {
            h.assert_denied(&h.admin,target,&denied).await?;
        }
        let initial=h.get(h.target).await?;
        for (resource,action,scope) in [("unknown","create","ORG"),("ticket","delete","ORG"),("ticket","update","SELF"),
            ("ticket","update","ORG_SUBTREE"),("ticket","update","TENANT")] {
            let before=h.snapshot().await?;
            let response=h.patch(h.target,&initial["version"],json!([{"resource":resource,"action":action,"scope":scope}]),json!([])).await?;
            ensure!(denied.contains(&response.status),"unregistered or out-of-policy capability returned {}",response.status);
            ensure!(h.snapshot().await?==before,"capability denial changed data");
        }
        for (table,id) in [("users",h.target.user),("organizations",h.target.org),("tenants",h.target.tenant),("applications",h.target.application)] {
            sqlx::query(&format!("UPDATE {table} SET status='DISABLED' WHERE id=$1")).bind(id).execute(&h.pool).await?;
            h.assert_denied(&h.admin,h.target,&denied).await?;
            sqlx::query(&format!("UPDATE {table} SET status='ENABLED' WHERE id=$1")).bind(id).execute(&h.pool).await?;
        }
        let ancestor:Uuid=sqlx::query_scalar("SELECT parent_id FROM organizations WHERE id=$1")
            .bind(h.target.org).fetch_one(&h.pool).await?;
        for (table,id) in [("tenants",h.target.tenant),("organizations",h.target.org),("organizations",ancestor)] {
            sqlx::query(&format!("UPDATE {table} SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1"))
                .bind(id).execute(&h.pool).await?;
            h.assert_denied(&h.admin,h.target,&denied).await?;
            sqlx::query(&format!("UPDATE {table} SET expires_at=NULL WHERE id=$1"))
                .bind(id).execute(&h.pool).await?;
        }
        sqlx::query("UPDATE organizations SET status='DISABLED' WHERE id=$1").bind(ancestor).execute(&h.pool).await?;
        h.assert_denied(&h.admin,h.target,&denied).await?;
        sqlx::query("UPDATE organizations SET status='ENABLED' WHERE id=$1").bind(ancestor).execute(&h.pool).await?;
        for (table,column,id) in [("tenant_applications","tenant_id",h.target.tenant),("organization_applications","organization_id",h.target.org)] {
            sqlx::query(&format!("DELETE FROM {table} WHERE {column}=$1 AND application_id=$2"))
                .bind(id).bind(h.target.application).execute(&h.pool).await?;
            h.assert_denied(&h.admin,h.target,&denied).await?;
            sqlx::query(&format!("INSERT INTO {table}({column},application_id) VALUES($1,$2)"))
                .bind(id).bind(h.target.application).execute(&h.pool).await?;
        }
        for (menu,action) in [(h.create_menu,"create"),(h.update_menu,"update")] {
            sqlx::query("UPDATE menus SET status='DISABLED' WHERE id=$1").bind(menu).execute(&h.pool).await?;
            let filtered=h.get(h.target).await?;
            ensure!(filtered["capabilities"].as_array().is_some_and(|caps|caps.iter().all(|cap|cap["action"]!=action)),"disabled exact app menu remained a manageable capability");
            let before=h.snapshot().await?;
            let response=h.patch(h.target,&filtered["version"],upsert(action,"ORG"),json!([])).await?;
            ensure!(response.status==StatusCode::BAD_REQUEST&&h.snapshot().await?==before,"disabled capability could be granted");
            sqlx::query("UPDATE menus SET status='ENABLED' WHERE id=$1").bind(menu).execute(&h.pool).await?;
        }
        let empty=router(&h.pool,&json!({"applications":[],"managementScopes":[]}))?;
        let response=call(&empty,&h.admin,Method::GET,&h.target.path(),Value::Null).await?;
        ensure!(denied.contains(&response.status),"empty default policy did not deny");
        let mut scoped=h.policy.clone();
        scoped["managementScopes"][0]["includeDescendants"]=json!(false);
        let (user,membership)=user(&h.pool,"phase14-descendant-target",false,h.descendant).await?;
        let target=Target{org:h.descendant,user,membership,..h.target};
        let response=call(&router(&h.pool,&scoped)?,&h.admin,Method::GET,&target.path(),Value::Null).await?;
        ensure!(denied.contains(&response.status),"exact policy root implicitly included descendants");
        // A tenant-wide stored label remains capped to this business membership's actual subtree.
        let valid=h.get(h.target).await?;
        let response=h.patch(h.target,&valid["version"],upsert("create","TENANT"),json!([])).await?;
        ensure!(response.status==StatusCode::OK,"explicit valid business tenant write was rejected");
        let view=grant(&response.body,"create")?;
        ensure!(view["scope"]=="TENANT"&&view["effectiveScope"]=="ORG_SUBTREE","stored/effective grant scope was conflated");
        let ids=view["organizationIds"].as_array().ok_or_else(||anyhow!("missing grant hard boundary"))?;
        ensure!(ids.len()==2&&ids.contains(&json!(h.target.org))&&ids.contains(&json!(h.descendant))&&!ids.contains(&json!(h.sibling)),"TENANT grant escaped membership subtree");
        sqlx::query("DELETE FROM memberships WHERE id=$1").bind(h.target.membership).execute(&h.pool).await?;
        h.assert_denied(&h.admin,h.target,&denied).await?;
        eprintln!("data_grant_boundaries membership=PASS tenant_org=PASS catalog=PASS explicit_scope=PASS disabled=PASS hard_boundary=PASS");
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit isolated TEST_DATABASE_URL and owned database privileges"]
async fn malformed_differentials_and_stale_sessions_are_rejected_without_effects() -> Result<()> {
    let db = database().await?;
    let result=async {
        let h=Harness::new(&db.pool).await?;
        let initial=h.get(h.target).await?;
        let valid=h.target.edit(&initial["version"],upsert("create","ORG"),json!([]));
        let mut cases=Vec::new();
        for key in ["target","expectedVersion","upsert","remove"] {
            let mut body=valid.clone(); body.as_object_mut().unwrap().remove(key); cases.push(body);
        }
        let mut body=valid.clone(); body["unexpected"]=json!(true); cases.push(body);
        let mut body=valid.clone(); body["target"]["unexpected"]=json!(true); cases.push(body);
        let mut body=valid.clone(); body["upsert"][0]["unexpected"]=json!(true); cases.push(body);
        let mut body=valid.clone(); body["target"]["membershipId"]=json!("invalid-uuid"); cases.push(body);
        let mut body=valid.clone(); body["upsert"][0]["scope"]=json!("UNKNOWN"); cases.push(body);
        cases.push(h.target.edit(&initial["version"],json!([]),json!([])));
        cases.push(h.target.edit(&initial["version"],json!([{"resource":"ticket","action":"create","scope":"ORG"},{"resource":"ticket","action":"create","scope":"SELF"}]),json!([])));
        cases.push(h.target.edit(&initial["version"],upsert("create","ORG"),remove("create")));
        cases.push(h.target.edit(&initial["version"],json!([]),json!([{"resource":"ticket","action":"create"},{"resource":"ticket","action":"create"}])));
        let oversized:Vec<Value>=(0..129).map(|i|json!({"resource":"ticket","action":format!("action-{i}"),"scope":"ORG"})).collect();
        cases.push(h.target.edit(&initial["version"],json!(oversized),json!([])));
        for body in cases {
            for (method,path) in [(Method::PATCH,"/api/data-grants"),(Method::POST,"/api/data-grants/preview")] {
                let before=h.snapshot().await?;
                let reply=call(&h.app,&h.admin,method,path,body.clone()).await?;
                ensure!(reply.status==StatusCode::BAD_REQUEST,"invalid differential returned {}",reply.status);
                ensure!(h.snapshot().await?==before,"invalid differential changed database");
            }
        }
        for query in ["/api/data-grants", "/api/data-grants?tenantId=invalid", &format!("{}&unexpected=1",h.target.path())] {
            let before=h.snapshot().await?;
            let reply=call(&h.app,&h.admin,Method::GET,query,Value::Null).await?;
            ensure!(reply.status.is_client_error(),"invalid query returned {}",reply.status);
            ensure!(h.snapshot().await?==before,"invalid query changed database");
        }
        sqlx::query("UPDATE users SET session_version=session_version+1 WHERE id=$1").bind(h.actor).execute(&h.pool).await?;
        h.assert_denied(&h.admin,h.target,&[StatusCode::UNAUTHORIZED]).await?;
        eprintln!("data_grant_contract strict_json=PASS missing=PASS duplicate=PASS oversized=PASS stale_session=PASS");
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit isolated TEST_DATABASE_URL and owned database privileges"]
async fn preview_differential_uuid_preservation_noop_and_conflict_are_observable() -> Result<()> {
    let db = database().await?;
    let result = async {
        let h = Harness::new(&db.pool).await?;
        let initial = h.get(h.target).await?;
        ensure!(
            initial["grants"] == json!([]),
            "new direct membership received implicit data grants"
        );
        let original_membership: Uuid = sqlx::query_scalar(
            "SELECT id FROM memberships WHERE user_id=$1 AND organization_id=$2",
        )
        .bind(h.target.user)
        .bind(h.target.org)
        .fetch_one(&h.pool)
        .await?;
        let body = h.target.edit(
            &initial["version"],
            json!([
            {"resource":"ticket","action":"create","scope":"ORG"},
            {"resource":"ticket","action":"update","scope":"ORG"}]),
            json!([]),
        );
        let before = h.snapshot().await?;
        let preview = call(
            &h.app,
            &h.admin,
            Method::POST,
            "/api/data-grants/preview",
            body.clone(),
        )
        .await?;
        ensure!(
            preview.status == StatusCode::OK
                && preview.body["before"] == json!([])
                && preview.body["after"]
                    .as_array()
                    .is_some_and(|v| v.len() == 2)
                && preview.body["changes"]["added"]
                    .as_array()
                    .is_some_and(|v| v.len() == 2),
            "preview omitted differential results"
        );
        ensure!(
            h.snapshot().await? == before,
            "preview wrote business state or audit"
        );
        let created = call(&h.app, &h.admin, Method::PATCH, "/api/data-grants", body).await?;
        ensure!(
            created.status == StatusCode::OK,
            "valid differential returned {}",
            created.status
        );
        h.audit(&created, h.target).await?;
        let create_id = grant(&created.body, "create")?["id"].clone();
        let update_id = grant(&created.body, "update")?["id"].clone();
        ensure!(
            create_id
                .as_str()
                .is_some_and(|id| Uuid::parse_str(id).is_ok())
                && create_id != update_id,
            "created grant IDs are invalid"
        );
        let change = h
            .patch(
                h.target,
                &created.body["version"],
                upsert("create", "ORG_SUBTREE"),
                json!([]),
            )
            .await?;
        ensure!(
            change.status == StatusCode::OK && change.body["version"] != created.body["version"],
            "scope update did not advance version"
        );
        ensure!(
            grant(&change.body, "create")?["id"] == create_id
                && grant(&change.body, "update")?["id"] == update_id,
            "scope update or unchanged grant replaced its UUID"
        );
        h.audit(&change, h.target).await?;
        let before = h.snapshot().await?;
        let noop = h
            .patch(
                h.target,
                &change.body["version"],
                upsert("create", "ORG_SUBTREE"),
                json!([]),
            )
            .await?;
        ensure!(
            noop.status == StatusCode::OK
                && noop.body["version"] == change.body["version"]
                && h.snapshot().await? == before,
            "no-op replaced state, version, timestamps or audit"
        );
        let stale = h
            .patch(
                h.target,
                &created.body["version"],
                upsert("create", "SELF"),
                json!([]),
            )
            .await?;
        ensure!(
            stale.status == StatusCode::CONFLICT && h.snapshot().await? == before,
            "stale differential did not conflict without effects"
        );
        let preview = call(
            &h.app,
            &h.admin,
            Method::POST,
            "/api/data-grants/preview",
            h.target.edit(
                &change.body["version"],
                upsert("create", "ORG"),
                remove("update"),
            ),
        )
        .await?;
        ensure!(
            preview.status == StatusCode::OK
                && preview.body["changes"]["updated"]
                    .as_array()
                    .is_some_and(|v| v.len() == 1)
                && preview.body["changes"]["removed"]
                    .as_array()
                    .is_some_and(|v| v.len() == 1)
                && h.snapshot().await? == before,
            "update/remove preview was not accurate and read-only"
        );
        let removed = h
            .patch(
                h.target,
                &change.body["version"],
                json!([]),
                remove("update"),
            )
            .await?;
        ensure!(
            removed.status == StatusCode::OK
                && removed.body["grants"]
                    .as_array()
                    .is_some_and(|v| v.len() == 1)
                && grant(&removed.body, "create")?["id"] == create_id,
            "differential delete disturbed surviving UUID"
        );
        let current_membership: Uuid = sqlx::query_scalar(
            "SELECT id FROM memberships WHERE user_id=$1 AND organization_id=$2",
        )
        .bind(h.target.user)
        .bind(h.target.org)
        .fetch_one(&h.pool)
        .await?;
        ensure!(
            current_membership == original_membership,
            "grant management replaced direct membership UUID"
        );
        eprintln!(
            "data_grant_differential preview_readonly=PASS uuid=PASS no_op=PASS conflict=PASS"
        );
        Ok(())
    }
    .await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit isolated TEST_DATABASE_URL and owned database privileges"]
async fn next_business_context_and_member_request_see_http_grant_changes() -> Result<()> {
    let db = database().await?;
    let result=async {
        let h=Harness::new(&db.pool).await?;
        let issue=call(&h.app,&h.business_session,Method::POST,"/api/oauth/session-token",json!({"appCode":CODE})).await?;
        ensure!(issue.status==StatusCode::CREATED,"business app token could not be issued");
        let token=issue.body["accessToken"].as_str().ok_or_else(||anyhow!("missing real business app token"))?;
        let context_path=format!("/api/apps/{CODE}/context");
        let members_path=format!("/api/apps/{CODE}/members?resource=ticket&action=create&organizationId={}",h.target.org);
        let before=call(&h.app,token,Method::GET,&context_path,Value::Null).await?;
        ensure!(before.status==StatusCode::OK&&before.body["dataGrants"]==json!([]),"business fixture contains implicit grant");
        ensure!(before.body["permissions"]==json!(["ticket:create","ticket:update"]),"business user has unexpected management permissions");
        ensure!(call(&h.app,token,Method::GET,&members_path,Value::Null).await?.status==StatusCode::FORBIDDEN,"business member endpoint accepted absent grant");
        let initial=h.get(h.target).await?;
        let granted=h.patch(h.target,&initial["version"],upsert("create","ORG_SUBTREE"),json!([])).await?;
        ensure!(granted.status==StatusCode::OK,"business grant was rejected");
        let after=call(&h.app,token,Method::GET,&context_path,Value::Null).await?;
        ensure!(after.status==StatusCode::OK&&after.body["contextVersion"]!=before.body["contextVersion"]
            &&after.body["dataGrants"][0]["scope"]=="ORG_SUBTREE","same app token retained stale grant context");
        ensure!(call(&h.app,token,Method::GET,&members_path,Value::Null).await?.status==StatusCode::OK,"ordinary business member query required management permission");
        let descendant=format!("/api/apps/{CODE}/members?resource=ticket&action=create&organizationId={}",h.descendant);
        ensure!(call(&h.app,token,Method::GET,&descendant,Value::Null).await?.status==StatusCode::OK,"business grant could not reach explicit descendant");
        let sibling=format!("/api/apps/{CODE}/members?resource=ticket&action=create&organizationId={}",h.sibling);
        ensure!(call(&h.app,token,Method::GET,&sibling,Value::Null).await?.status==StatusCode::NOT_FOUND,"business grant escaped sibling boundary");
        let one=format!("/api/apps/{CODE}/members/{}?resource=ticket&action=create&organizationId={}",h.target.user,h.target.org);
        ensure!(call(&h.app,token,Method::GET,&one,Value::Null).await?.status==StatusCode::OK,"ordinary business single-member verification failed");
        let management=call(&h.app,&h.business_session,Method::GET,&h.target.path(),Value::Null).await?;
        ensure!(management.status==StatusCode::FORBIDDEN,"business grant implied grant-management authority");
        let removed=h.patch(h.target,&granted.body["version"],json!([]),remove("create")).await?;
        ensure!(removed.status==StatusCode::OK,"grant removal failed");
        let revoked=call(&h.app,token,Method::GET,&context_path,Value::Null).await?;
        ensure!(revoked.status==StatusCode::OK&&revoked.body["dataGrants"]==json!([])&&revoked.body["contextVersion"]!=after.body["contextVersion"],"grant deletion was not visible next context");
        ensure!(call(&h.app,token,Method::GET,&members_path,Value::Null).await?.status==StatusCode::FORBIDDEN,"next member request retained removed grant");
        eprintln!("data_grant_business same_token_next_context=PASS next_member_authorization=PASS no_management_permission=PASS");
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit isolated TEST_DATABASE_URL and owned database privileges"]
async fn grant_audit_raise_and_silent_skip_roll_back_complete_differential() -> Result<()> {
    let db = database().await?;
    let result=async {
        let h=Harness::new(&db.pool).await?;
        let initial=h.get(h.target).await?;
        let created=h.patch(h.target,&initial["version"],upsert("update","ORG"),json!([])).await?;
        ensure!(created.status==StatusCode::OK,"audit fixture setup through HTTP failed");
        let body=h.target.edit(&created.body["version"],upsert("create","ORG_SUBTREE"),remove("update"));
        for mode in ["raise","skip"] {
            sqlx::query("INSERT INTO phase14_audit_fault(mode) VALUES($1)").bind(mode).execute(&h.pool).await?;
            let before=h.snapshot().await?;
            let failures=crate::observability::audit_write_failures();
            let reply=call(&h.app,&h.admin,Method::PATCH,"/api/data-grants",body.clone()).await?;
            ensure!(reply.status==StatusCode::INTERNAL_SERVER_ERROR,"audit {mode} returned {}",reply.status);
            ensure!(h.snapshot().await?==before,"audit {mode} committed partial grant/delete/audit changes");
            ensure!(crate::observability::audit_write_failures()==failures+1,"audit {mode} failure counter did not increment once");
            sqlx::query("DELETE FROM phase14_audit_fault").execute(&h.pool).await?;
        }
        let retry=call(&h.app,&h.admin,Method::PATCH,"/api/data-grants",body).await?;
        ensure!(retry.status==StatusCode::OK,"same-version retry after rollback failed");
        h.audit(&retry,h.target).await?;
        ensure!(retry.body["grants"].as_array().is_some_and(|rows|rows.len()==1)&&grant(&retry.body,"create")?["scope"]=="ORG_SUBTREE","successful retry did not apply complete differential");
        eprintln!("data_grant_audit raise_rollback=PASS silent_skip_rollback=PASS successful_retry_request_id=PASS");
        Ok(())
    }.await;
    db.finish(result).await
}

async fn wait_for_lock(pool: &PgPool, blocker: i32, table: &str) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(12),async {
        loop {
            let waiting: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)) AND query LIKE $2)")
                .bind(blocker).bind(format!("%{table}%")).fetch_one(pool).await?;
            if waiting {return Ok(());}
            tokio::task::yield_now().await;
        }
    }).await.map_err(|_|anyhow!("data grant request did not reach observed {table} lock barrier"))?
}

#[tokio::test]
#[ignore = "requires explicit isolated TEST_DATABASE_URL and owned database privileges"]
async fn competing_versions_serialize_and_lock_wait_rechecks_current_authority() -> Result<()> {
    let db = database().await?;
    let result=async {
        let h=Harness::new(&db.pool).await?;
        let initial=h.get(h.target).await?;
        let a=h.target.edit(&initial["version"],upsert("create","ORG"),json!([]));
        let b=h.target.edit(&initial["version"],upsert("create","SELF"),json!([]));
        let (a,b)=tokio::join!(call(&h.app,&h.admin,Method::PATCH,"/api/data-grants",a),call(&h.app,&h.admin,Method::PATCH,"/api/data-grants",b));
        let a=a?; let b=b?;
        ensure!((a.status==StatusCode::OK&&b.status==StatusCode::CONFLICT)||(b.status==StatusCode::OK&&a.status==StatusCode::CONFLICT),"competing base versions did not produce one winner and one conflict");
        let audits:i64=sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action=$1").bind(ACTION).fetch_one(&h.pool).await?;
        ensure!(audits==1,"concurrent conflict wrote duplicate success audit");
        assert_permission_rechecked_after_lock(&h).await?;
        assert_target_rechecked_after_lock(&h).await?;
        assert_membership_rechecked_after_lock(&h).await?;
        eprintln!("data_grant_concurrency one_winner=PASS permission_revalidation=PASS target_revalidation=PASS membership_revalidation=PASS");
        Ok(())
    }.await;
    db.finish(result).await
}

async fn assert_permission_rechecked_after_lock(h: &Harness) -> Result<()> {
    let current = h.get(h.target).await?;
    let protected = grant_and_audit_state(&h.pool, None).await?;
    let mut blocker = h.pool.begin().await?;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await?;
    sqlx::query("SELECT id FROM memberships WHERE id=$1 FOR UPDATE")
        .bind(h.target.membership)
        .execute(&mut *blocker)
        .await?;
    let app = h.app.clone();
    let token = h.writer.clone();
    let body = h.target.edit(
        &current["version"],
        upsert("create", "ORG_SUBTREE"),
        json!([]),
    );
    let request =
        tokio::spawn(
            async move { call(&app, &token, Method::PATCH, "/api/data-grants", body).await },
        );
    if let Err(error) = wait_for_lock(&h.pool, pid, "memberships").await {
        blocker.rollback().await?;
        let _ = request.await;
        return Err(error);
    }
    sqlx::query("DELETE FROM role_menus WHERE role_id=$1 AND menu_id=$2")
        .bind(h.writer_role)
        .bind(h.write_menu)
        .execute(&mut *blocker)
        .await?;
    blocker.commit().await?;
    let before = h.snapshot().await?;
    let response = request.await??;
    ensure!(
        response.status == StatusCode::FORBIDDEN,
        "writer permission removed during lock wait was accepted: {}",
        response.status
    );
    ensure!(
        h.snapshot().await? == before,
        "permission loss during lock wait committed grant/audit"
    );
    ensure!(
        grant_and_audit_state(&h.pool, None).await? == protected,
        "permission revocation changed protected grant/audit state after unlock"
    );
    Ok(())
}

async fn assert_target_rechecked_after_lock(h: &Harness) -> Result<()> {
    let current = h.get(h.target).await?;
    let protected = grant_and_audit_state(&h.pool, None).await?;
    let mut blocker = h.pool.begin().await?;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await?;
    sqlx::query("SELECT id FROM memberships WHERE id=$1 FOR UPDATE")
        .bind(h.target.membership)
        .execute(&mut *blocker)
        .await?;
    let app = h.app.clone();
    let token = h.admin.clone();
    let body = h
        .target
        .edit(&current["version"], upsert("create", "TENANT"), json!([]));
    let request =
        tokio::spawn(
            async move { call(&app, &token, Method::PATCH, "/api/data-grants", body).await },
        );
    if let Err(error) = wait_for_lock(&h.pool, pid, "memberships").await {
        blocker.rollback().await?;
        let _ = request.await;
        return Err(error);
    }
    sqlx::query(
        "DELETE FROM organization_applications WHERE organization_id=$1 AND application_id=$2",
    )
    .bind(h.target.org)
    .bind(h.target.application)
    .execute(&mut *blocker)
    .await?;
    blocker.commit().await?;
    let before = h.snapshot().await?;
    let response = request.await??;
    ensure!(
        [StatusCode::FORBIDDEN, StatusCode::NOT_FOUND].contains(&response.status),
        "target app revoked during lock wait was accepted: {}",
        response.status
    );
    ensure!(
        h.snapshot().await? == before,
        "target app loss during lock wait committed grant/audit"
    );
    ensure!(
        grant_and_audit_state(&h.pool, None).await? == protected,
        "application revocation changed protected grant/audit state after unlock"
    );
    sqlx::query(
        "INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)",
    )
    .bind(h.target.org)
    .bind(h.target.application)
    .execute(&h.pool)
    .await?;
    Ok(())
}

async fn assert_membership_rechecked_after_lock(h: &Harness) -> Result<()> {
    let current = h.get(h.target).await?;
    // Membership removal legitimately cascades its grants; preserve all other
    // grants and every audit from before the waiting request can resume.
    let protected = grant_and_audit_state(&h.pool, Some(h.target.membership)).await?;
    let mut blocker = h.pool.begin().await?;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await?;
    sqlx::query("SELECT id FROM memberships WHERE id=$1 FOR UPDATE")
        .bind(h.target.membership)
        .execute(&mut *blocker)
        .await?;
    let app = h.app.clone();
    let token = h.admin.clone();
    let body = h
        .target
        .edit(&current["version"], upsert("create", "TENANT"), json!([]));
    let request =
        tokio::spawn(
            async move { call(&app, &token, Method::PATCH, "/api/data-grants", body).await },
        );
    if let Err(error) = wait_for_lock(&h.pool, pid, "memberships").await {
        blocker.rollback().await?;
        let _ = request.await;
        return Err(error);
    }
    sqlx::query("DELETE FROM memberships WHERE id=$1")
        .bind(h.target.membership)
        .execute(&mut *blocker)
        .await?;
    blocker.commit().await?;
    let before = h.snapshot().await?;
    let response = request.await??;
    ensure!(
        response.status == StatusCode::NOT_FOUND,
        "membership removed during lock wait was accepted: {}",
        response.status
    );
    ensure!(
        h.snapshot().await? == before,
        "membership removal during lock wait committed grant/audit"
    );
    ensure!(
        grant_and_audit_state(&h.pool, Some(h.target.membership)).await? == protected,
        "membership revocation changed unrelated grants or success audits"
    );
    Ok(())
}

async fn grant_and_audit_state(
    pool: &PgPool,
    excluded_member: Option<Uuid>,
) -> Result<(String, String)> {
    let grants = sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(g) ORDER BY g.id),'[]'::jsonb)::text FROM membership_data_grants g WHERE $1::uuid IS NULL OR g.membership_id<>$1")
        .bind(excluded_member).fetch_one(pool).await?;
    let audits = sqlx::query_scalar(
        "SELECT COALESCE(jsonb_agg(to_jsonb(a) ORDER BY a.id),'[]'::jsonb)::text FROM audit_logs a",
    )
    .fetch_one(pool)
    .await?;
    Ok((grants, audits))
}
