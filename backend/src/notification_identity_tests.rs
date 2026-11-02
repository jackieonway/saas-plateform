//! Phase 4 receipt identities on owned PostgreSQL databases and real Router HTTP.
//! Only IDENTITY_PHASE04_TEST_DATABASE_URL is read. No production customer login is needed.
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::process::{Child, Command, Stdio};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, ensure, Result};
use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use axum::{middleware, Router};
use serde_json::{json, Value};
use sqlx::postgres::{PgConnectOptions, PgListener, PgPoolOptions};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::config::Config;
use crate::util::{jwt, uuid7::new_uuid};

const SECRET: &str = "owned-phase04-notification-identity-only";
const CODE: &str = "phase04-notifications";
const REASON: &str = "Owned Phase 4 identity notification fixture";
const PROOF_PATH: &str = "/api/internal/notifications/context-proof";

struct Database {
    pool: PgPool,
    admin: PgPool,
    options: PgConnectOptions,
    name: String,
}

impl Database {
    async fn create() -> Result<Self> {
        Self::create_through(i64::MAX, true).await
    }

    async fn create_through(version: i64, target_schema: bool) -> Result<Self> {
        let url = std::env::var("IDENTITY_PHASE04_TEST_DATABASE_URL")
            .map_err(|_| anyhow!("explicit IDENTITY_PHASE04_TEST_DATABASE_URL is required"))?;
        let options = PgConnectOptions::from_str(&url)
            .map_err(|_| anyhow!("invalid owned Phase 4 database configuration"))?;
        ensure!(
            options.get_database() == Some("postgres"),
            "Phase 4 administration must explicitly select postgres"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("owned Phase 4 administration connection failed"))?;
        let name = format!("identity_phase04_{}", new_uuid().simple());
        ensure!(
            name.len() <= 63,
            "owned Phase 4 database name exceeds limit"
        );
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await?;
        let options = options.database(&name);
        let pool = match PgPoolOptions::new()
            .max_connections(12)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
        {
            Ok(pool) => pool,
            Err(_) => {
                let _ = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
                    .execute(&admin)
                    .await;
                return Err(anyhow!("owned Phase 4 fixture connection failed"));
            }
        };
        let db = Self {
            pool,
            admin,
            options,
            name,
        };
        if let Err(error) = db.migrate(version).await {
            let _ = db.finish(Ok(())).await;
            return Err(error);
        }
        // Owned target-schema preparation only. Published 0015 retains the
        // old binary key; the production contraction belongs to Phase 6.
        if target_schema {
            if let Err(error) = sqlx::query(
                "ALTER TABLE memberships DROP CONSTRAINT IF EXISTS memberships_user_id_organization_id_key",
            )
            .execute(&db.pool)
            .await
            {
                let _ = db.finish(Ok(())).await;
                return Err(error.into());
            }
        }
        Ok(db)
    }

    async fn migrate(&self, version: i64) -> Result<()> {
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(
                sqlx::migrate!("./migrations")
                    .iter()
                    .filter(|migration| migration.version <= version)
                    .cloned()
                    .collect(),
            ),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&self.pool)
        .await?;
        Ok(())
    }

    async fn independent_pool(&self) -> Result<PgPool> {
        PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(self.options.clone())
            .await
            .map_err(|_| anyhow!("owned Phase 4 inspection connection failed"))
    }

    async fn finish(self, assertions: Result<()>) -> Result<()> {
        ensure!(
            self.name.starts_with("identity_phase04_")
                && self.name.len() <= 63
                && self
                    .name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_'),
            "refusing cleanup outside owned Phase 4 fixture"
        );
        let closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let removed = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await;
        self.admin.close().await;
        eprintln!("identity_phase04_cleanup database={} assertions_ok={} database_removed={} pool_closed={closed}",self.name,assertions.is_ok(),removed.is_ok());
        assertions?;
        removed?;
        ensure!(closed, "owned Phase 4 pool did not close");
        Ok(())
    }
}

struct Reply {
    status: StatusCode,
    body: Value,
}

async fn call(app: &Router, token: &str, method: Method, path: &str, body: Value) -> Result<Reply> {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-request-id", new_uuid().to_string())
        .extension(ConnectInfo("192.0.2.44:30044".parse::<SocketAddr>()?))
        .body(Body::from(serde_json::to_vec(&body)?))?;
    let response = tokio::time::timeout(Duration::from_secs(25), app.clone().oneshot(request))
        .await
        .map_err(|_| anyhow!("owned Phase 4 HTTP timeout: {path}"))??;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok(Reply {
        status,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    })
}

fn expect(reply: &Reply, status: StatusCode, operation: &str) -> Result<()> {
    ensure!(
        reply.status == status,
        "{operation}: expected {status}, received {} (error={})",
        reply.status,
        reply
            .body
            .get("code")
            .or_else(|| reply.body.get("error"))
            .unwrap_or(&Value::Null)
    );
    Ok(())
}

fn keys(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .map(|v| v.keys().map(String::as_str).collect())
        .unwrap_or_default()
}

#[derive(Clone)]
struct Member {
    user: Uuid,
    tenant: Uuid,
    org: Uuid,
    id: Uuid,
    kind: &'static str,
    version: i64,
}
struct Credentials {
    sid: Uuid,
    session: String,
    app: String,
    account_version: i64,
}

async fn organization(
    pool: &PgPool,
    tenant: Uuid,
    parent: Option<Uuid>,
    label: &str,
) -> Result<Uuid> {
    let id = new_uuid();
    sqlx::query("INSERT INTO organizations(id,tenant_id,parent_id,name) VALUES($1,$2,$3,$4)")
        .bind(id)
        .bind(tenant)
        .bind(parent)
        .bind(label)
        .execute(pool)
        .await?;
    Ok(id)
}
async fn account(pool: &PgPool, label: &str, admin: bool) -> Result<Uuid> {
    let id = new_uuid();
    sqlx::query("INSERT INTO users(id,username,password_hash,is_super_admin) VALUES($1,$2,'owned-unusable-phase04-password',$3)")
        .bind(id).bind(label).bind(admin).execute(pool).await?;
    Ok(id)
}
async fn member(
    pool: &PgPool,
    user: Uuid,
    tenant: Uuid,
    org: Uuid,
    kind: &'static str,
    status: &str,
) -> Result<Member> {
    let id = new_uuid();
    sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,status) VALUES($1,$2,$3,$4,$5)")
        .bind(id).bind(user).bind(org).bind(kind).bind(status).execute(pool).await?;
    Ok(Member {
        user,
        tenant,
        org,
        id,
        kind,
        version: 1,
    })
}
async fn credentials(pool: &PgPool, member: &Member, application: Uuid) -> Result<Credentials> {
    let account_version: i64 = sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
        .bind(member.user)
        .fetch_one(pool)
        .await?;
    let now = chrono::Utc::now().timestamp();
    let app = if member.kind == "EMPLOYEE" {
        crate::sso_test_fixture::app_capability(
            pool,
            member.user,
            member.tenant,
            member.org,
            application,
            CODE,
            SECRET,
        )
        .await?
    } else {
        // A customer-shaped internal token is an explicit negative input. It has
        // no admitted employee source or stored application capability.
        let sid = crate::sso_test_fixture::session_for_member(
            pool,
            member.user,
            member.tenant,
            member.org,
            member.kind,
        )
        .await?;
        jwt::sign_internal(
            &jwt::Claims {
                sub: member.user.to_string(),
                typ: "app_session".into(),
                tenant: Some(member.tenant.to_string()),
                org: Some(member.org.to_string()),
                sid: Some(sid.to_string()),
                ver: Some(account_version),
                cid: Some(CODE.into()),
                scope: Some("app:access".into()),
                aud: Some(CODE.into()),
                jti: Some(new_uuid().to_string()),
                iat: now,
                exp: now + 300,
            },
            jwt::InternalUse::AppSession,
            SECRET,
        )?
    };
    let app_claims = jwt::verify_internal(&app, SECRET, jwt::InternalUse::AppSession, Some(CODE))?;
    let sid: Uuid = app_claims
        .sid
        .as_deref()
        .ok_or_else(|| anyhow!("fixture capability source missing"))?
        .parse()?;
    let session = jwt::sign_internal(
        &jwt::Claims {
            sub: member.user.to_string(),
            typ: "platform_session".into(),
            tenant: Some(member.tenant.to_string()),
            org: Some(member.org.to_string()),
            sid: Some(sid.to_string()),
            ver: Some(account_version),
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
    Ok(Credentials {
        sid,
        session,
        app,
        account_version,
    })
}

struct Harness {
    pool: PgPool,
    app: Router,
    tenant: Uuid,
    child: Uuid,
    sibling: Uuid,
    application: Uuid,
    admin: Credentials,
}
impl Harness {
    async fn create(pool: &PgPool) -> Result<Self> {
        let tenant = new_uuid();
        sqlx::query("INSERT INTO tenants(id,name,code,is_platform) VALUES($1,'Owned Phase 4','owned-phase04',true)").bind(tenant).execute(pool).await?;
        let root = organization(pool, tenant, None, "Phase 4 root").await?;
        let child = organization(pool, tenant, Some(root), "Phase 4 child").await?;
        let sibling = organization(pool, tenant, Some(root), "Phase 4 sibling").await?;
        let application = new_uuid();
        let system = new_uuid();
        sqlx::query("INSERT INTO applications(id,name,code,base_path,allowed_member_types) VALUES($1,'Owned Phase 4 business',$2,$3,ARRAY['EMPLOYEE','CUSTOMER']::text[])")
            .bind(application).bind(CODE).bind(format!("/micro/{CODE}")).execute(pool).await?;
        sqlx::query(
            "INSERT INTO applications(id,name,code) VALUES($1,'Owned Phase 4 system','system')",
        )
        .bind(system)
        .execute(pool)
        .await?;
        for app in [application, system] {
            sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
                .bind(tenant)
                .bind(app)
                .execute(pool)
                .await?;
            for org in [root, child, sibling] {
                sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)").bind(org).bind(app).execute(pool).await?;
            }
        }
        for (app, permission) in [
            (application, "notification:publish"),
            (system, "system:user"),
            (system, "system:audit"),
        ] {
            let menu = new_uuid();
            sqlx::query("INSERT INTO menus(id,application_id,title,type,permission,visible) VALUES($1,$2,$3,'BUTTON',$3,false)")
                .bind(menu).bind(app).bind(permission).execute(pool).await?;
            sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
                .bind(tenant)
                .bind(menu)
                .execute(pool)
                .await?;
            for org in [root, child, sibling] {
                sqlx::query(
                    "INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)",
                )
                .bind(org)
                .bind(menu)
                .execute(pool)
                .await?;
            }
        }
        let manager = member(
            pool,
            account(pool, "phase04-manager", true).await?,
            tenant,
            root,
            "EMPLOYEE",
            "ENABLED",
        )
        .await?;
        let admin = credentials(pool, &manager, application).await?;
        sqlx::query("INSERT INTO membership_data_grants(id,membership_id,application_id,resource_code,action_code,scope_type) VALUES($1,$2,$3,'notification','publish','TENANT')")
            .bind(new_uuid()).bind(manager.id).bind(application).execute(pool).await?;
        let config = Config::from_values(&HashMap::from([
            ("APP_ENV".into(), "test".into()),
            (
                "DATABASE_URL".into(),
                "postgres://unused/owned-phase04".into(),
            ),
            ("JWT_SECRET".into(), SECRET.into()),
            ("MEMBERSHIP_LIFECYCLE_ENABLED".into(), "true".into()),
            ("CUSTOMER_MEMBERSHIPS_ENABLED".into(), "true".into()),
            ("DUAL_MEMBERSHIPS_ENABLED".into(), "true".into()),
        ]))?;
        let state = crate::sso_test_fixture::state(pool.clone(), Arc::new(config));
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
            tenant,
            child,
            sibling,
            application,
            admin,
        })
    }
    async fn person(
        &self,
        label: &str,
        org: Uuid,
        kind: &'static str,
        status: &str,
    ) -> Result<Member> {
        member(
            &self.pool,
            account(&self.pool, label, false).await?,
            self.tenant,
            org,
            kind,
            status,
        )
        .await
    }
    async fn api(&self, token: &str, method: Method, path: &str, body: Value) -> Result<Reply> {
        call(&self.app, token, method, path, body).await
    }
    async fn publish(&self, key: &str, target: Value) -> Result<Reply> {
        self.api(&self.admin.app,Method::POST,&format!("/api/apps/{CODE}/notifications"),json!({"idempotencyKey":key,"target":target,"type":"INFO","title":key,"content":"Owned Phase 4 publication","link":format!("/micro/{CODE}/owned")})).await
    }
    async fn list(&self, credentials: &Credentials) -> Result<Value> {
        let r = self
            .api(
                &credentials.session,
                Method::GET,
                "/api/notifications?size=100",
                Value::Null,
            )
            .await?;
        expect(&r, StatusCode::OK, "identity inbox list")?;
        Ok(r.body)
    }
}

async fn fixture_notification(pool: &PgPool, target: &Member, label: &str) -> Result<Uuid> {
    let id = new_uuid();
    sqlx::query("INSERT INTO notifications(id,user_id,type,title,content,link,source_kind,tenant_id,organization_id,recipient_membership_id,recipient_member_type,recipient_identity_version) VALUES($1,$2,'INFO',$3,'Owned explicit receipt fixture',$9,'SYSTEM',$4,$5,$6,$7,$8)")
        .bind(id).bind(target.user).bind(label).bind(target.tenant).bind(target.org).bind(target.id).bind(target.kind).bind(target.version).bind(format!("/owned/{}/{}",target.id,target.version)).execute(pool).await?;
    Ok(id)
}

async fn stored_receipts(pool: &PgPool, target: &Member) -> Result<BTreeSet<Uuid>> {
    Ok(sqlx::query_scalar("SELECT id FROM notifications WHERE recipient_membership_id=$1 AND recipient_member_type=$2 AND recipient_identity_version=$3 ORDER BY id")
        .bind(target.id).bind(target.kind).bind(target.version).fetch_all(pool).await?.into_iter().collect())
}

fn ids(list: &Value) -> Result<BTreeSet<Uuid>> {
    list["records"]
        .as_array()
        .ok_or_else(|| anyhow!("inbox records missing"))?
        .iter()
        .map(|r| {
            r["id"]
                .as_str()
                .ok_or_else(|| anyhow!("notification ID missing"))?
                .parse()
                .map_err(Into::into)
        })
        .collect()
}

async fn notification_snapshot(pool: &PgPool) -> Result<Value> {
    Ok(sqlx::query_scalar("SELECT jsonb_build_object('notifications',(SELECT COALESCE(jsonb_agg(to_jsonb(n) ORDER BY n.id),'[]'::jsonb) FROM notifications n),'publications',(SELECT COALESCE(jsonb_agg(to_jsonb(p) ORDER BY p.id),'[]'::jsonb) FROM notification_publications p),'preferences',(SELECT COALESCE(jsonb_agg(to_jsonb(p) ORDER BY p.membership_id,p.identity_version),'[]'::jsonb) FROM notification_member_prefs p),'audits',(SELECT count(*) FROM audit_logs))")
        .fetch_one(pool).await?)
}

#[tokio::test]
#[ignore = "creates and drops identity_phase04_* databases using explicit IDENTITY_PHASE04_TEST_DATABASE_URL"]
async fn notification_identity_phase04_employee_audiences_deduplicate_and_emit_bound_hints(
) -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let h=Harness::create(&db.pool).await?;
        let employee=h.person("phase04-audience-dual",h.child,"EMPLOYEE","ENABLED").await?;
        let customer=member(&h.pool,employee.user,h.tenant,h.child,"CUSTOMER","ENABLED").await?;
        let other=member(&h.pool,employee.user,h.tenant,h.sibling,"EMPLOYEE","ENABLED").await?;
        let plain=h.person("phase04-audience-plain",h.child,"EMPLOYEE","ENABLED").await?;
        let only_customer=h.person("phase04-audience-customer-only",h.child,"CUSTOMER","ENABLED").await?;
        let disabled=h.person("phase04-audience-disabled",h.child,"EMPLOYEE","DISABLED").await?;
        let disabled_customer=member(&h.pool,disabled.user,h.tenant,h.child,"CUSTOMER","ENABLED").await?;
        let customer_credentials=credentials(&h.pool,&customer,h.application).await?;
        ensure!(crate::app_token::inspect(&h.pool,&customer_credentials.app,&jwt::verify_internal(&customer_credentials.app,SECRET,jwt::InternalUse::AppSession,Some(CODE))?).await?.is_none(),"customer-shaped publisher unexpectedly entered employee authority");
        let mut listener=PgListener::connect_with(&h.pool).await?;
        listener.listen("new_notification").await?;
        let target=json!({"kind":"USER","userId":employee.user,"organizationId":h.child});
        let published=h.publish("phase04-targeted",target.clone()).await?;
        expect(&published,StatusCode::OK,"employee-targeted publication on dual identity account")?;
        ensure!(published.body["recipientCount"]==1&&published.body["deliveryCount"]==1,"same-org customer duplicated employee delivery");
        let publication:Uuid=published.body["publicationId"].as_str().ok_or_else(||anyhow!("publication ID missing"))?.parse()?;
        let hints=tokio::time::timeout(Duration::from_secs(10),listener.recv()).await.map_err(|_|anyhow!("owned Phase 4 notification hint missing"))??;
        let hint:Value=serde_json::from_str(hints.payload())?;
        ensure!(keys(&hint)==BTreeSet::from(["hintVersion","id","userId","tenantId","organizationId","membershipId","memberType","identityVersion"]),"private hint did not have exactly the eight v1 fields");
        ensure!(hint["hintVersion"]=="1"&&hint["userId"]==json!(employee.user)&&hint["tenantId"]==json!(employee.tenant)&&hint["organizationId"]==json!(employee.org)&&hint["membershipId"]==json!(employee.id)&&hint["memberType"]=="EMPLOYEE"&&hint["identityVersion"].as_i64()==Some(1),"transactional hint lost its exact recipient member/version proof");
        let bound:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM notifications WHERE id=$1 AND publication_id=$2 AND recipient_membership_id=$3 AND recipient_member_type='EMPLOYEE' AND recipient_identity_version=1)")
            .bind(hint["id"].as_str().ok_or_else(||anyhow!("hint ID missing"))?.parse::<Uuid>()?).bind(publication).bind(employee.id).fetch_one(&h.pool).await?;
        ensure!(bound,"hint has no matching persisted member-bound delivery");
        let publisher_bound:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM notification_publications p JOIN platform_sessions s ON s.id=$2 WHERE p.id=$1 AND p.publisher_membership_id=s.membership_id AND p.publisher_member_type='EMPLOYEE' AND p.publisher_identity_version=s.identity_version)")
            .bind(publication).bind(h.admin.sid).fetch_one(&h.pool).await?;
        ensure!(publisher_bound,"publication did not bind the exact publisher member generation");
        let retry=h.publish("phase04-targeted",target).await?;
        expect(&retry,StatusCode::OK,"same employee publication retry")?;
        ensure!(retry.body["duplicate"]==true&&retry.body["publicationId"]==published.body["publicationId"],"member-bound publication retry changed old idempotency contract");
        let org=h.publish("phase04-org",json!({"kind":"ORGANIZATION","organizationId":h.child})).await?;
        expect(&org,StatusCode::OK,"employee organization broadcast")?;
        ensure!(org.body["recipientCount"]==2&&org.body["deliveryCount"]==2,"organization audience included customer/disabled rows or duplicated a dual identity");
        let tenant=h.publish("phase04-tenant",json!({"kind":"TENANT"})).await?;
        expect(&tenant,StatusCode::OK,"employee tenant broadcast")?;
        ensure!(tenant.body["recipientCount"]==3&&tenant.body["deliveryCount"]==4,"recipientCount must retain distinct-user meaning while deliveries distinguish employee organizations");
        let delivered:Vec<Uuid>=sqlx::query_scalar("SELECT DISTINCT recipient_membership_id FROM notifications WHERE source_kind='APP' ORDER BY recipient_membership_id").fetch_all(&h.pool).await?;
        ensure!(delivered.contains(&employee.id)&&delivered.contains(&other.id)&&delivered.contains(&plain.id)&&!delivered.contains(&customer.id)&&!delivered.contains(&only_customer.id)&&!delivered.contains(&disabled.id)&&!delivered.contains(&disabled_customer.id),"employee APP audience contains an incorrect identity");
        let before=notification_snapshot(&h.pool).await?;
        for target in [&only_customer,&disabled]{
            let denied=h.publish(&format!("phase04-invalid-target-{}",target.id),json!({"kind":"USER","userId":target.user,"organizationId":target.org})).await?;
            expect(&denied,StatusCode::NOT_FOUND,"customer-only/disabled-employee direct APP target")?;
            ensure!(notification_snapshot(&h.pool).await?==before,"unqualified direct target left publication, delivery or successful audit rows");
        }
        let customer_publish=h.api(&customer_credentials.app,Method::POST,&format!("/api/apps/{CODE}/notifications"),json!({"idempotencyKey":"phase04-customer-publisher","target":{"kind":"USER","userId":employee.user,"organizationId":h.child},"type":"INFO","title":"Unregistered customer sender","content":"fixture","link":format!("/micro/{CODE}/owned")})).await?;
        ensure!(matches!(customer_publish.status,StatusCode::UNAUTHORIZED|StatusCode::FORBIDDEN),"customer publisher borrowed employee account authority");
        ensure!(notification_snapshot(&h.pool).await?==before,"customer publish rejection changed delivery or audit state");
        let before_system:i64=sqlx::query_scalar("SELECT count(*) FROM notifications WHERE source_kind='SYSTEM'").fetch_one(&h.pool).await?;
        let mut tx=h.pool.begin().await?;
        crate::notifications::system_in_tx(&mut tx,&[(employee.user,h.tenant,h.child),(employee.user,h.tenant,h.child),(only_customer.user,h.tenant,h.child),(disabled.user,h.tenant,h.child)],"SECURITY","Owned employee SYSTEM compatibility","fixture","/owned").await?;
        tx.commit().await?;
        let system:Vec<(Uuid,String,i64)>=sqlx::query_as("SELECT recipient_membership_id,recipient_member_type,recipient_identity_version FROM notifications WHERE source_kind='SYSTEM' ORDER BY id").fetch_all(&h.pool).await?;
        ensure!(before_system==0&&system==vec![(employee.id,"EMPLOYEE".to_owned(),1)],"legacy SYSTEM helper did not resolve only the active employee identity");
        sqlx::query("UPDATE applications SET allowed_member_types=ARRAY['CUSTOMER']::text[] WHERE id=$1").bind(h.application).execute(&h.pool).await?;
        let before_console=notification_snapshot(&h.pool).await?;
        let console=h.api(&h.admin.session,Method::POST,"/api/notifications",json!({"applicationId":h.application,"idempotencyKey":"phase04-console-disallowed-member","target":{"kind":"USER","userId":employee.user,"organizationId":employee.org},"type":"INFO","title":"Owned console application member-type gate","content":"fixture","link":format!("/micro/{CODE}/owned")})).await?;
        expect(&console,StatusCode::FORBIDDEN,"employee console publication must honor application member-type policy without an app token")?;
        ensure!(notification_snapshot(&h.pool).await?==before_console,"disallowed employee console publication left publication, delivery or success audit rows");
        drop(listener);
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops identity_phase04_* databases using explicit IDENTITY_PHASE04_TEST_DATABASE_URL"]
async fn notification_identity_phase04_inbox_mutations_and_legacy_archive_never_cross_identity(
) -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let h=Harness::create(&db.pool).await?;
        let employee=h.person("phase04-inbox-dual",h.child,"EMPLOYEE","ENABLED").await?;
        let customer=member(&h.pool,employee.user,h.tenant,h.child,"CUSTOMER","ENABLED").await?;
        let other=member(&h.pool,employee.user,h.tenant,h.sibling,"EMPLOYEE","ENABLED").await?;
        let employee_credentials=credentials(&h.pool,&employee,h.application).await?;
        let customer_credentials=credentials(&h.pool,&customer,h.application).await?;
        let other_credentials=credentials(&h.pool,&other,h.application).await?;
        let mut own=Vec::new();let mut foreign=Vec::new();
        for index in 0..5 {
            own.push(fixture_notification(&h.pool,&employee,&format!("phase04-employee-{index}")).await?);
            foreign.push(fixture_notification(&h.pool,&customer,&format!("phase04-customer-{index}")).await?);
        }
        let other_id=fixture_notification(&h.pool,&other,"phase04-other-org").await?;
        let legacy=new_uuid();
        sqlx::query("INSERT INTO notifications(id,user_id,title,content,source_kind) VALUES($1,$2,'phase04-unbound-legacy','Owned old shape','LEGACY')").bind(legacy).bind(employee.user).execute(&h.pool).await?;
        let old_system=new_uuid();
        sqlx::query("INSERT INTO notifications(id,user_id,title,source_kind,tenant_id,organization_id) VALUES($1,$2,'phase04-unbound-system','SYSTEM',$3,$4)").bind(old_system).bind(employee.user).bind(h.tenant).bind(h.child).execute(&h.pool).await?;
        ensure!(ids(&h.list(&employee_credentials).await?)?==own.iter().copied().collect(),"employee inbox leaked customer/other-org/NULL-proof rows");
        expect(&h.api(&customer_credentials.session,Method::GET,"/api/notifications",Value::Null).await?,StatusCode::UNAUTHORIZED,"customer cannot open employee inbox")?;
        ensure!(stored_receipts(&h.pool,&customer).await?==foreign.iter().copied().collect(),"customer stored receipts changed during employee read");
        ensure!(ids(&h.list(&other_credentials).await?)?==BTreeSet::from([other_id]),"second employee organization reused first organization's receipt inbox");
        for (creds,expected) in [(&employee_credentials,5),(&other_credentials,1)]{
            let unread=h.api(&creds.session,Method::GET,"/api/notifications/unread-count",Value::Null).await?;expect(&unread,StatusCode::OK,"isolated unread count")?;ensure!(unread.body["count"]==expected,"unread count crossed identity");
        }
        for foreign_id in [foreign[0],other_id,legacy,old_system]{
            let read=h.api(&employee_credentials.session,Method::PUT,&format!("/api/notifications/{foreign_id}/read"),Value::Null).await?;
            expect(&read,StatusCode::OK,"cross-identity mark-read contract")?;ensure!(read.body["ok"]==false,"mark-read changed an invisible identity");
            let delete=h.api(&employee_credentials.session,Method::DELETE,&format!("/api/notifications/{foreign_id}"),Value::Null).await?;expect(&delete,StatusCode::NOT_FOUND,"cross-identity delete")?;
        }
        let batch=h.api(&employee_credentials.session,Method::PUT,"/api/notifications/read-batch",json!({"ids":[own[0],foreign[0],other_id,legacy,old_system]})).await?;
        expect(&batch,StatusCode::OK,"mixed identity read-batch")?;ensure!(batch.body["updated"]==1,"read-batch mutated another recipient");
        let all=h.api(&customer_credentials.session,Method::PUT,"/api/notifications/read-all",Value::Null).await?;
        expect(&all,StatusCode::UNAUTHORIZED,"customer cannot mutate employee inbox through read-all")?;
        let employee_unread=h.api(&employee_credentials.session,Method::GET,"/api/notifications/unread-count",Value::Null).await?;
        ensure!(employee_unread.body["count"]==4,"customer read-all changed employee read state");
        let employee_all=h.api(&employee_credentials.session,Method::PUT,"/api/notifications/read-all",Value::Null).await?;
        expect(&employee_all,StatusCode::OK,"employee exact-generation read-all")?;
        ensure!(employee_all.body["updated"]==4,"employee read-all crossed receipt identity");
        let deleted=h.api(&customer_credentials.session,Method::DELETE,"/api/notifications",json!({"ids":[foreign[0],own[0],other_id,legacy,old_system]})).await?;
        expect(&deleted,StatusCode::UNAUTHORIZED,"customer cannot mutate employee inbox through batch-delete")?;
        let own_batch=h.api(&employee_credentials.session,Method::DELETE,"/api/notifications",json!({"ids":[own[0],foreign[0],other_id,legacy,old_system]})).await?;
        expect(&own_batch,StatusCode::OK,"employee mixed-identity batch-delete")?;
        ensure!(own_batch.body["deleted"]==1,"employee batch-delete removed another receipt identity");
        let single=h.api(&employee_credentials.session,Method::DELETE,&format!("/api/notifications/{}",own[1]),Value::Null).await?;expect(&single,StatusCode::OK,"own employee delete")?;
        ensure!(ids(&h.list(&employee_credentials).await?)?.len()==3&&stored_receipts(&h.pool,&customer).await?.len()==5,"read/delete mutations mixed the two inboxes");
        let untouched:bool=sqlx::query_scalar("SELECT count(*)=3 AND bool_and(NOT is_read AND read_at IS NULL) FROM notifications WHERE id=ANY($1)").bind(vec![other_id,legacy,old_system]).fetch_one(&h.pool).await?;
        ensure!(untouched,"unbound archives or second-org receipt were mutated through mixed IDs");
        let archived=h.api(&h.admin.session,Method::GET,"/api/notifications/legacy?size=100",Value::Null).await?;
        expect(&archived,StatusCode::OK,"platform audit archive")?;
        let archived_ids=ids(&archived.body)?;
        ensure!(archived_ids.contains(&legacy)&&archived_ids.contains(&old_system),"NULL recipient proofs were not available only through explicit administrator archive");
        let denied=h.api(&customer_credentials.session,Method::GET,"/api/notifications/legacy",Value::Null).await?;
        ensure!(matches!(denied.status,StatusCode::UNAUTHORIZED|StatusCode::FORBIDDEN),"customer context accessed archive administration");
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops identity_phase04_* databases using explicit IDENTITY_PHASE04_TEST_DATABASE_URL"]
async fn notification_identity_phase04_public_payload_and_internal_context_proofs_are_separate(
) -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let h=Harness::create(&db.pool).await?;
        let employee=h.person("phase04-proof-dual",h.child,"EMPLOYEE","ENABLED").await?;
        let customer=member(&h.pool,employee.user,h.tenant,h.child,"CUSTOMER","ENABLED").await?;
        let employee_credentials=credentials(&h.pool,&employee,h.application).await?;
        let customer_credentials=credentials(&h.pool,&customer,h.application).await?;
        let employee_id=fixture_notification(&h.pool,&employee,"phase04-proof-employee").await?;
        let customer_id=fixture_notification(&h.pool,&customer,"phase04-proof-customer").await?;
        for path in [format!("/api/notifications/{customer_id}"),format!("/api/notifications/socket-context?notificationId={customer_id}"),format!("{PROOF_PATH}?notificationId={customer_id}")] {
            expect(&h.api(&customer_credentials.session,Method::GET,&path,Value::Null).await?,StatusCode::UNAUTHORIZED,"customer has no employee notification proof")?;
        }
        ensure!(stored_receipts(&h.pool,&customer).await?==BTreeSet::from([customer_id]),"employee proof checks changed customer receipt");
        {
            let (target,creds,own,foreign)=(&employee,&employee_credentials,employee_id,customer_id);
            let detail=h.api(&creds.session,Method::GET,&format!("/api/notifications/{own}"),Value::Null).await?;
            expect(&detail,StatusCode::OK,"owned identity detail")?;
            ensure!(detail.body["link"]==json!(format!("/owned/{}/{}",target.id,target.version)),"public notification jump link was mixed across recipient identities");
            ensure!(keys(&detail.body)==BTreeSet::from(["id","userId","type","title","content","link","isRead","readAt","createdAt","sourceKind","tenantId","organizationId","applicationId","publicationId","applicationName"]),"Notification 1 public payload acquired private receipt proof fields or changed its field allowlist");
            expect(&h.api(&creds.session,Method::GET,&format!("/api/notifications/{foreign}"),Value::Null).await?,StatusCode::NOT_FOUND,"foreign identity detail")?;
            let public=h.api(&creds.session,Method::GET,&format!("/api/notifications/socket-context?notificationId={own}"),Value::Null).await?;
            expect(&public,StatusCode::OK,"public socket-context owned delivery")?;
            ensure!(keys(&public.body)==BTreeSet::from(["contractVersion","userId","tenantId","organizationId","sessionId","expiresAt"])&&public.body["contractVersion"]=="1.0", "public socket-context changed its six-field 1.0 contract");
            let proof=h.api(&creds.session,Method::GET,&format!("{PROOF_PATH}?notificationId={own}"),Value::Null).await?;
            expect(&proof,StatusCode::OK,"internal exact identity delivery proof")?;
            ensure!(keys(&proof.body)==BTreeSet::from(["proofVersion","contractVersion","userId","tenantId","organizationId","sessionId","expiresAt","membershipId","memberType","identityVersion","accountVersion"]),"internal notification proof does not have exactly eleven fields");
            ensure!(proof.body["proofVersion"]=="1"&&proof.body["contractVersion"]=="1.0"&&proof.body["userId"]==json!(target.user)&&proof.body["tenantId"]==json!(target.tenant)&&proof.body["organizationId"]==json!(target.org)&&proof.body["sessionId"]==json!(creds.sid)&&proof.body["membershipId"]==json!(target.id)&&proof.body["memberType"]==target.kind&&proof.body["identityVersion"].as_i64()==Some(target.version)&&proof.body["accountVersion"].as_i64()==Some(creds.account_version),"internal proof guessed another member or changed integer version types");
            for prefix in ["/api/notifications/socket-context",PROOF_PATH]{
                expect(&h.api(&creds.session,Method::GET,&format!("{prefix}?notificationId={foreign}"),Value::Null).await?,StatusCode::NOT_FOUND,"replay/backfill proof for another identity")?;
                let connect=h.api(&creds.session,Method::GET,prefix,Value::Null).await?;expect(&connect,StatusCode::OK,"initial/reconnect proof binds exact active identity")?;
            }
        }
        let mut forged=jwt::verify_internal(&employee_credentials.session,SECRET,jwt::InternalUse::PlatformSession,Some(jwt::PLATFORM_API_AUDIENCE))?;
        forged.ver=Some(employee_credentials.account_version+1);
        let forged=jwt::sign_internal(&forged,jwt::InternalUse::PlatformSession,SECRET)?;
        expect(&h.api(&forged,Method::GET,PROOF_PATH,Value::Null).await?,StatusCode::UNAUTHORIZED,"JWT account version mismatch")?;
        expect(&h.api("invalid-owned-token",Method::GET,PROOF_PATH,Value::Null).await?,StatusCode::UNAUTHORIZED,"internal proof never accepts an unverified bearer")?;
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops identity_phase04_* databases using explicit IDENTITY_PHASE04_TEST_DATABASE_URL"]
async fn notification_identity_phase04_local_departure_and_replacement_preserve_other_receipts(
) -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let h=Harness::create(&db.pool).await?;
        for kind in ["EMPLOYEE","CUSTOMER"]{
            for operation in ["disable","delete","replace"]{
                let source=h.person(&format!("phase04-{kind}-{operation}"),h.child,kind,"ENABLED").await?;
                let target_kind=if kind=="EMPLOYEE"{"CUSTOMER"}else{"EMPLOYEE"};
                let target=member(&h.pool,source.user,h.tenant,h.child,target_kind,"ENABLED").await?;
                let second=member(&h.pool,source.user,h.tenant,h.sibling,"EMPLOYEE","ENABLED").await?;
                let source_credentials=credentials(&h.pool,&source,h.application).await?;
                let target_credentials=credentials(&h.pool,&target,h.application).await?;
                let second_credentials=credentials(&h.pool,&second,h.application).await?;
                let old_id=fixture_notification(&h.pool,&source,"phase04-old-source").await?;
                let target_id=fixture_notification(&h.pool,&target,"phase04-other-identity").await?;
                let second_id=fixture_notification(&h.pool,&second,"phase04-other-org").await?;
                let old_row:Value=sqlx::query_scalar("SELECT to_jsonb(n) FROM notifications n WHERE id=$1").bind(old_id).fetch_one(&h.pool).await?;
                expect(&h.api(&source_credentials.session,Method::GET,&format!("{PROOF_PATH}?notificationId={old_id}"),Value::Null).await?,if kind=="EMPLOYEE" {StatusCode::OK}else{StatusCode::UNAUTHORIZED},"original employee proof/customer refusal")?;
                let (method,path,body)=match operation {
                    "disable"=>(Method::PUT,format!("/api/memberships/{}/status",source.id),json!({"expectedIdentityVersion":1,"status":"DISABLED","reason":REASON})),
                    "delete"=>(Method::DELETE,format!("/api/memberships/{}",source.id),json!({"expectedIdentityVersion":1,"reason":REASON})),
                    "replace"=>(Method::POST,format!("/api/memberships/{}/replace-identity",source.id),json!({"memberType":target_kind,"expectedIdentityVersion":1,"expectedTargetIdentityVersion":1,"accountBindingConfirmed":true,"bindingSource":"Owned Phase 4 explicit target","reason":REASON})),
                    _=>unreachable!(),
                };
                expect(&h.api(&h.admin.session,method,&path,body).await?,StatusCode::OK,&format!("local receipt identity lifecycle change {kind}/{operation}"))?;
                for path in ["/api/notifications".to_owned(),"/api/notifications/socket-context".to_owned(),PROOF_PATH.to_owned(),format!("{PROOF_PATH}?notificationId={old_id}")]{
                    expect(&h.api(&source_credentials.session,Method::GET,&path,Value::Null).await?,StatusCode::UNAUTHORIZED,"old identity connection/reconnect/replay after local departure")?;
                }
                ensure!(stored_receipts(&h.pool,&target).await?==BTreeSet::from([target_id]),"local lifecycle change mixed the other identity receipt");
                if target.kind=="EMPLOYEE" {ensure!(ids(&h.list(&target_credentials).await?)?==BTreeSet::from([target_id]),"local lifecycle revoked another employee identity");}
                ensure!(ids(&h.list(&second_credentials).await?)?==BTreeSet::from([second_id]),"local lifecycle change revoked another organization's employee receipt");
                expect(&h.api(&target_credentials.session,Method::GET,&format!("{PROOF_PATH}?notificationId={target_id}"),Value::Null).await?,if target.kind=="EMPLOYEE" {StatusCode::OK}else{StatusCode::UNAUTHORIZED},"other employee proof/customer refusal remains unchanged")?;
                let retained:Value=sqlx::query_scalar("SELECT to_jsonb(n) FROM notifications n WHERE id=$1").bind(old_id).fetch_one(&h.pool).await?;
                ensure!(retained==old_row,"local lifecycle rewrote/deleted the receipt tombstone instead of denying the old proof");
                let returned=if operation=="delete"{
                    member(&h.pool,source.user,h.tenant,h.child,kind,"ENABLED").await?
                }else{
                    expect(&h.api(&h.admin.session,Method::PUT,&format!("/api/memberships/{}/status",source.id),json!({"expectedIdentityVersion":2,"status":"ENABLED","reason":REASON})).await?,StatusCode::OK,"explicit source identity reenable")?;
                    Member{version:3,..source.clone()}
                };
                let fresh=credentials(&h.pool,&returned,h.application).await?;
                ensure!(stored_receipts(&h.pool,&returned).await?.is_empty(),"new identity generation inherited an old notification receipt");
                if returned.kind=="EMPLOYEE" {ensure!(ids(&h.list(&fresh).await?)?.is_empty(),"new employee generation exposed old receipt");}
                expect(&h.api(&fresh.session,Method::GET,&format!("{PROOF_PATH}?notificationId={old_id}"),Value::Null).await?,if returned.kind=="EMPLOYEE" {StatusCode::NOT_FOUND}else{StatusCode::UNAUTHORIZED},"new generation cannot replay old receipt")?;
                let new_id=fixture_notification(&h.pool,&returned,"phase04-current-generation").await?;
                ensure!(stored_receipts(&h.pool,&returned).await?==BTreeSet::from([new_id]),"new identity generation lost current exact stored proof");
                if returned.kind=="EMPLOYEE" {ensure!(ids(&h.list(&fresh).await?)?==BTreeSet::from([new_id]),"new employee generation did not receive only current proof");}
                expect(&h.api(&source_credentials.session,Method::GET,PROOF_PATH,Value::Null).await?,StatusCode::UNAUTHORIZED,"reenable cannot restore old online connection proof")?;
            }
        }
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops identity_phase04_* databases using explicit IDENTITY_PHASE04_TEST_DATABASE_URL"]
async fn notification_identity_phase04_preferences_are_exact_member_generation_and_keep_wire_shape(
) -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let h=Harness::create(&db.pool).await?;
        let employee=h.person("phase04-prefs-dual",h.child,"EMPLOYEE","ENABLED").await?;
        let customer=member(&h.pool,employee.user,h.tenant,h.child,"CUSTOMER","ENABLED").await?;
        let other=member(&h.pool,employee.user,h.tenant,h.sibling,"EMPLOYEE","ENABLED").await?;
        let employee_credentials=credentials(&h.pool,&employee,h.application).await?;
        let customer_credentials=credentials(&h.pool,&customer,h.application).await?;
        let other_credentials=credentials(&h.pool,&other,h.application).await?;
        sqlx::query("INSERT INTO notification_prefs(user_id,dnd_enabled,dnd_start,dnd_end,muted_types) VALUES($1,true,'01:02','03:04','SECURITY')").bind(employee.user).execute(&h.pool).await?;
        let default=json!({"dndEnabled":false,"dndStart":"22:00","dndEnd":"07:00","mutedTypes":[]});
        for creds in [&employee_credentials,&other_credentials]{
            let r=h.api(&creds.session,Method::GET,"/api/notifications/prefs",Value::Null).await?;expect(&r,StatusCode::OK,"member preference default")?;
            ensure!(r.body==default,"old account-level preference was copied into a current member");
        }
        let employee_prefs=json!({"dndEnabled":true,"dndStart":"19:00","dndEnd":"06:00","mutedTypes":["INFO"]});
        let customer_prefs=json!({"dndEnabled":true,"dndStart":"20:00","dndEnd":"08:00","mutedTypes":["WARNING"]});
        {
            let (creds,body)=(&employee_credentials,&employee_prefs);
            let r=h.api(&creds.session,Method::PUT,"/api/notifications/prefs",body.clone()).await?;expect(&r,StatusCode::OK,"exact identity preference update")?;
            let read=h.api(&creds.session,Method::GET,"/api/notifications/prefs",Value::Null).await?;
            ensure!(read.body==*body&&keys(&read.body)==BTreeSet::from(["dndEnabled","dndStart","dndEnd","mutedTypes"]),"member preferences changed old JSON DTO or another identity's value");
        }
        sqlx::query("INSERT INTO notification_member_prefs(membership_id,identity_version,dnd_enabled,dnd_start,dnd_end,muted_types) VALUES($1,1,true,'20:00','08:00','WARNING')").bind(customer.id).execute(&h.pool).await?;
        let customer_row:Value=sqlx::query_scalar("SELECT to_jsonb(p) FROM notification_member_prefs p WHERE membership_id=$1").bind(customer.id).fetch_one(&h.pool).await?;
        expect(&h.api(&customer_credentials.session,Method::PUT,"/api/notifications/prefs",customer_prefs.clone()).await?,StatusCode::UNAUTHORIZED,"customer cannot write employee preferences")?;
        ensure!(h.api(&other_credentials.session,Method::GET,"/api/notifications/prefs",Value::Null).await?.body==default,"same-user second-org preferences were shared");
        expect(&h.api(&h.admin.session,Method::PUT,&format!("/api/memberships/{}/status",employee.id),json!({"expectedIdentityVersion":1,"status":"DISABLED","reason":REASON})).await?,StatusCode::OK,"disable preference owner")?;
        expect(&h.api(&h.admin.session,Method::PUT,&format!("/api/memberships/{}/status",employee.id),json!({"expectedIdentityVersion":2,"status":"ENABLED","reason":REASON})).await?,StatusCode::OK,"reenable preference owner")?;
        let returned=Member{version:3,..employee.clone()};
        let fresh=credentials(&h.pool,&returned,h.application).await?;
        ensure!(h.api(&fresh.session,Method::GET,"/api/notifications/prefs",Value::Null).await?.body==default,"new member generation inherited the old generation's preference");
        expect(&h.api(&customer_credentials.session,Method::GET,"/api/notifications/prefs",Value::Null).await?,StatusCode::UNAUTHORIZED,"customer cannot read employee preferences")?;
        let retained_customer_row:Value=sqlx::query_scalar("SELECT to_jsonb(p) FROM notification_member_prefs p WHERE membership_id=$1").bind(customer.id).fetch_one(&h.pool).await?;
        ensure!(retained_customer_row==customer_row,"employee preference generation change affected stored customer preference");
        let rows:i64=sqlx::query_scalar("SELECT count(*) FROM notification_member_prefs WHERE membership_id=ANY($1)").bind(vec![employee.id,customer.id,other.id]).fetch_one(&h.pool).await?;
        ensure!(rows==2,"preference reads copied old rows or fabricated another generation");
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops identity_phase04_* databases using explicit IDENTITY_PHASE04_TEST_DATABASE_URL"]
async fn notification_identity_phase04_account_revocation_and_authority_failure_fail_closed(
) -> Result<()> {
    let db = Database::create().await?;
    let result = async {
        let h = Harness::create(&db.pool).await?;
        let employee = h
            .person("phase04-global-dual", h.child, "EMPLOYEE", "ENABLED")
            .await?;
        let customer = member(
            &h.pool,
            employee.user,
            h.tenant,
            h.child,
            "CUSTOMER",
            "ENABLED",
        )
        .await?;
        let other = member(
            &h.pool,
            employee.user,
            h.tenant,
            h.sibling,
            "EMPLOYEE",
            "ENABLED",
        )
        .await?;
        let employee_credentials = credentials(&h.pool, &employee, h.application).await?;
        let customer_credentials = credentials(&h.pool, &customer, h.application).await?;
        let other_credentials = credentials(&h.pool, &other, h.application).await?;
        let id = fixture_notification(&h.pool, &employee, "phase04-authority-employee").await?;
        fixture_notification(&h.pool, &customer, "phase04-authority-customer").await?;
        expect(
            &h.api(
                &customer_credentials.session,
                Method::GET,
                PROOF_PATH,
                Value::Null,
            )
            .await?,
            StatusCode::UNAUTHORIZED,
            "customer source refused before account revocation",
        )?;
        for creds in [&employee_credentials, &other_credentials] {
            expect(
                &h.api(&creds.session, Method::GET, PROOF_PATH, Value::Null)
                    .await?,
                StatusCode::OK,
                "valid context before global account revocation",
            )?;
        }
        sqlx::query("UPDATE users SET session_version=session_version+1 WHERE id=$1")
            .bind(employee.user)
            .execute(&h.pool)
            .await?;
        for creds in [
            &employee_credentials,
            &customer_credentials,
            &other_credentials,
        ] {
            for path in [
                "/api/notifications",
                "/api/notifications/unread-count",
                "/api/notifications/prefs",
                "/api/notifications/socket-context",
                PROOF_PATH,
            ] {
                expect(
                    &h.api(&creds.session, Method::GET, path, Value::Null)
                        .await?,
                    StatusCode::UNAUTHORIZED,
                    "global account version revokes all identity connections",
                )?;
            }
        }
        let fresh = credentials(&h.pool, &employee, h.application).await?;
        let proof = h
            .api(&fresh.session, Method::GET, PROOF_PATH, Value::Null)
            .await?;
        expect(&proof, StatusCode::OK, "fresh account-generation context")?;
        ensure!(
            proof.body["accountVersion"].as_i64() == Some(employee_credentials.account_version + 1),
            "new context proof did not use authoritative account version"
        );
        ensure!(
            ids(&h.list(&fresh).await?)? == BTreeSet::from([id]),
            "global account revocation wrongly rewrote the unchanged membership receipt generation"
        );
        let inspector = db.independent_pool().await?;
        let before = notification_snapshot(&inspector).await?;
        h.pool.close().await;
        let prefs =
            json!({"dndEnabled":true,"dndStart":"21:00","dndEnd":"05:00","mutedTypes":["INFO"]});
        for (method, path, body) in [
            (Method::GET, "/api/notifications".to_owned(), Value::Null),
            (Method::GET, format!("/api/notifications/{id}"), Value::Null),
            (
                Method::GET,
                "/api/notifications/unread-count".to_owned(),
                Value::Null,
            ),
            (
                Method::PUT,
                format!("/api/notifications/{id}/read"),
                Value::Null,
            ),
            (
                Method::PUT,
                "/api/notifications/read-batch".to_owned(),
                json!({"ids":[id]}),
            ),
            (
                Method::PUT,
                "/api/notifications/read-all".to_owned(),
                Value::Null,
            ),
            (
                Method::DELETE,
                format!("/api/notifications/{id}"),
                Value::Null,
            ),
            (
                Method::DELETE,
                "/api/notifications".to_owned(),
                json!({"ids":[id]}),
            ),
            (
                Method::GET,
                "/api/notifications/prefs".to_owned(),
                Value::Null,
            ),
            (Method::PUT, "/api/notifications/prefs".to_owned(), prefs),
            (
                Method::GET,
                "/api/notifications/socket-context".to_owned(),
                Value::Null,
            ),
            (
                Method::GET,
                format!("{PROOF_PATH}?notificationId={id}"),
                Value::Null,
            ),
        ] {
            expect(
                &h.api(&fresh.session, method, &path, body).await?,
                StatusCode::SERVICE_UNAVAILABLE,
                "authority failure cannot authorize inbox or socket operation",
            )?;
        }
        ensure!(
            notification_snapshot(&inspector).await? == before,
            "503 failure changed receipt, preference, publication or success audit state"
        );
        inspector.close().await;
        Ok(())
    }
    .await;
    db.finish(result).await
}

async fn wait_for_mutation_barrier(pool: &PgPool, blocker: i32, pattern: &str) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(15),async {
        loop {
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)) AND query ILIKE $2)")
                .bind(blocker).bind(pattern).fetch_one(pool).await?;
            if waiting{return Ok::<(),anyhow::Error>(());}
            tokio::task::yield_now().await;
        }
    }).await.map_err(|_|anyhow!("owned Phase 4 HTTP mutation never reached its observed row-lock barrier"))?
}

#[tokio::test]
#[ignore = "creates and drops identity_phase04_* databases using explicit IDENTITY_PHASE04_TEST_DATABASE_URL"]
async fn notification_identity_phase04_mutation_waits_recheck_member_and_account_before_commit(
) -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let h=Harness::create(&db.pool).await?;
        for scenario in ["read_one","read_batch","read_all","delete_one","delete_batch","preferences"]{
            let kind="EMPLOYEE";
            let actor=h.person(&format!("phase04-wait-{scenario}"),h.child,kind,"ENABLED").await?;
            let counterpart=member(&h.pool,actor.user,h.tenant,h.child,if kind=="EMPLOYEE"{"CUSTOMER"}else{"EMPLOYEE"},"ENABLED").await?;
            let actor_credentials=credentials(&h.pool,&actor,h.application).await?;
            let counterpart_credentials=credentials(&h.pool,&counterpart,h.application).await?;
            let id=fixture_notification(&h.pool,&actor,"phase04-wait-owned-receipt").await?;
            let other_id=fixture_notification(&h.pool,&counterpart,"phase04-wait-other-receipt").await?;
            if scenario=="preferences"{
                expect(&h.api(&actor_credentials.session,Method::PUT,"/api/notifications/prefs",json!({"dndEnabled":false,"dndStart":"22:00","dndEnd":"07:00","mutedTypes":[]})).await?,StatusCode::OK,"preference barrier seed")?;
            }
            let before=notification_snapshot(&h.pool).await?;
            let mut blocker=h.pool.begin().await?;
            let blocker_pid:i32=sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *blocker).await?;
            if scenario=="preferences"{
                sqlx::query("SELECT membership_id FROM notification_member_prefs WHERE membership_id=$1 AND identity_version=1 FOR UPDATE").bind(actor.id).execute(&mut *blocker).await?;
            }else{
                sqlx::query("SELECT id FROM notifications WHERE id=$1 FOR UPDATE").bind(id).execute(&mut *blocker).await?;
            }
            let (method,path,body,pattern)=match scenario {
                "read_one"=>(Method::PUT,format!("/api/notifications/{id}/read"),Value::Null,"%UPDATE notifications%"),
                "read_batch"=>(Method::PUT,"/api/notifications/read-batch".to_owned(),json!({"ids":[id,other_id]}),"%UPDATE notifications%"),
                "read_all"=>(Method::PUT,"/api/notifications/read-all".to_owned(),Value::Null,"%UPDATE notifications%"),
                "delete_one"=>(Method::DELETE,format!("/api/notifications/{id}"),Value::Null,"%DELETE FROM notifications%"),
                "delete_batch"=>(Method::DELETE,"/api/notifications".to_owned(),json!({"ids":[id,other_id]}),"%DELETE FROM notifications%"),
                "preferences"=>(Method::PUT,"/api/notifications/prefs".to_owned(),json!({"dndEnabled":true,"dndStart":"18:00","dndEnd":"08:00","mutedTypes":["SECURITY"]}),"%INSERT INTO notification_member_prefs%"),
                _=>unreachable!(),
            };
            let request_app=h.app.clone();let request_token=actor_credentials.session.clone();
            let request=tokio::spawn(async move{call(&request_app,&request_token,method,&path,body).await});
            if let Err(error)=wait_for_mutation_barrier(&h.pool,blocker_pid,pattern).await {
                blocker.rollback().await?;request.abort();let _=request.await;return Err(error);
            }
            let global=matches!(scenario,"delete_one"|"preferences");
            if global {
                sqlx::query("UPDATE users SET session_version=session_version+1 WHERE id=$1").bind(actor.user).execute(&mut *blocker).await?;
            }else{
                sqlx::query("UPDATE memberships SET status='DISABLED',identity_version=identity_version+1 WHERE id=$1").bind(actor.id).execute(&mut *blocker).await?;
            }
            blocker.commit().await?;
            let reply=tokio::time::timeout(Duration::from_secs(20),request).await.map_err(|_|anyhow!("revoked notification mutation failed to finish after row-lock release"))???;
            expect(&reply,StatusCode::UNAUTHORIZED,"notification mutation revoked during a real late row-lock wait")?;
            ensure!(notification_snapshot(&h.pool).await?==before,"{scenario} committed notification/preferences or success audit after identity revocation");
            if !global {
                ensure!(stored_receipts(&h.pool,&counterpart).await?==BTreeSet::from([other_id]),"local waiting-mutation revocation changed other identity receipt");
                expect(&h.api(&counterpart_credentials.session,Method::GET,PROOF_PATH,Value::Null).await?,StatusCode::UNAUTHORIZED,"customer counterpart remains outside employee authority")?;
            }else{
                expect(&h.api(&counterpart_credentials.session,Method::GET,PROOF_PATH,Value::Null).await?,StatusCode::UNAUTHORIZED,"global waiting-mutation revocation covers other identity")?;
            }
        }
        for scenario in ["application_revoked","member_type_removed"] {
            let actor=h.person(&format!("phase04-app-wait-{scenario}"),h.child,"EMPLOYEE","ENABLED").await?;
            let actor_credentials=credentials(&h.pool,&actor,h.application).await?;
            let publication=h.publish(&format!("phase04-app-wait-{scenario}"),json!({"kind":"USER","userId":actor.user,"organizationId":actor.org})).await?;
            expect(&publication,StatusCode::OK,"APP notification for application-metadata wait")?;
            let id:Uuid=sqlx::query_scalar("SELECT id FROM notifications WHERE publication_id=$1")
                .bind(publication.body["publicationId"].as_str().ok_or_else(||anyhow!("APP wait publication missing"))?.parse::<Uuid>()?).fetch_one(&h.pool).await?;
            let before=notification_snapshot(&h.pool).await?;
            let mut blocker=h.pool.begin().await?;
            let blocker_pid:i32=sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *blocker).await?;
            sqlx::query("SELECT id FROM notifications WHERE id=$1 FOR UPDATE").bind(id).execute(&mut *blocker).await?;
            let request_app=h.app.clone();let request_token=actor_credentials.session.clone();
            let request=tokio::spawn(async move{call(&request_app,&request_token,Method::PUT,&format!("/api/notifications/{id}/read"),Value::Null).await});
            if let Err(error)=wait_for_mutation_barrier(&h.pool,blocker_pid,"%UPDATE notifications%").await{
                blocker.rollback().await?;request.abort();let _=request.await;return Err(error);
            }
            if scenario=="application_revoked"{
                sqlx::query("DELETE FROM organization_applications WHERE organization_id=$1 AND application_id=$2").bind(h.child).bind(h.application).execute(&mut *blocker).await?;
            }else{
                sqlx::query("UPDATE applications SET allowed_member_types=ARRAY['CUSTOMER']::text[] WHERE id=$1").bind(h.application).execute(&mut *blocker).await?;
            }
            blocker.commit().await?;
            let reply=tokio::time::timeout(Duration::from_secs(20),request).await.map_err(|_|anyhow!("APP metadata-revoked mutation did not finish"))???;
            expect(&reply,StatusCode::FORBIDDEN,"APP metadata revoked during actual notification row wait")?;
            ensure!(notification_snapshot(&h.pool).await?==before,"APP metadata revocation left read-state or success audit changes");
            expect(&h.api(&actor_credentials.session,Method::GET,PROOF_PATH,Value::Null).await?,StatusCode::OK,"APP metadata revocation did not revoke the general identity context")?;
            if scenario=="application_revoked"{
                sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)").bind(h.child).bind(h.application).execute(&h.pool).await?;
            }else{
                sqlx::query("UPDATE applications SET allowed_member_types=ARRAY['EMPLOYEE','CUSTOMER']::text[] WHERE id=$1").bind(h.application).execute(&h.pool).await?;
            }
        }
        Ok(())
    }.await;
    db.finish(result).await
}

const SOCKET_ORIGIN: &str = "http://127.0.0.1:49044";
const METRICS: &str = "owned-phase04-notification-metrics-only";

struct ExternalNotifications {
    child: Child,
    platform: tokio::task::JoinHandle<()>,
    output: Option<std::thread::JoinHandle<()>>,
    client: reqwest::Client,
    platform_origin: String,
    origin: String,
}
impl Drop for ExternalNotifications {
    fn drop(&mut self) {
        self.platform.abort();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if let Some(output) = self.output.take() {
            let _ = output.join();
        }
    }
}
impl ExternalNotifications {
    async fn start(db: &Database, app: Router) -> Result<Self> {
        let binary = std::env::var_os("IDENTITY_PHASE04_NOTIFICATION_BINARY").ok_or_else(|| {
            anyhow!(
                "explicit IDENTITY_PHASE04_NOTIFICATION_BINARY is required; no old binary fallback"
            )
        })?;
        let binary = std::path::PathBuf::from(binary);
        ensure!(
            binary.is_absolute() && binary.is_file(),
            "Phase 4 notification binary must be an explicitly selected existing absolute file"
        );
        let raw = std::env::var("IDENTITY_PHASE04_TEST_DATABASE_URL")
            .map_err(|_| anyhow!("explicit Phase 4 administration URL missing"))?;
        let mut database_url =
            reqwest::Url::parse(&raw).map_err(|_| anyhow!("Phase 4 child database URL invalid"))?;
        database_url.set_path(&format!("/{}", db.name));
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let platform_origin = format!("http://{}", listener.local_addr()?);
        let platform = tokio::spawn(async move {
            let _ = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await;
        });
        let mut command = Command::new(binary);
        command.env_clear();
        for key in ["SYSTEMROOT", "WINDIR", "PATH"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .env("DATABASE_URL", database_url.as_str())
            .env("PLATFORM_API_BASE_URL", &platform_origin)
            .env("NOTIFICATION_ENV", "test")
            .env("NOTIFICATION_ALLOWED_ORIGINS", SOCKET_ORIGIN)
            .env("NOTIFICATION_HOST", "127.0.0.1")
            .env("NOTIFICATION_PORT", "0")
            .env("NOTIFICATION_SOCKET_PATH", "/socket.io/")
            .env("NOTIFICATION_AUTH_TIMEOUT_MS", "1000")
            .env("NOTIFICATION_AUTH_RECHECK_MS", "1000")
            .env("NOTIFICATION_AUTH_WAIT_MS", "2000")
            .env("NOTIFICATION_HEALTH_PROBE_MS", "1000")
            .env("NOTIFICATION_DATABASE_RETRY_MS", "100")
            .env("NOTIFICATION_SHUTDOWN_ON_STDIN", "1")
            .env("METRICS_TOKEN", METRICS)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(_) => {
                platform.abort();
                let _ = platform.await;
                return Err(anyhow!(
                    "explicit Phase 4 notification binary could not start"
                ));
            }
        };
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                platform.abort();
                let _ = platform.await;
                return Err(anyhow!("notification child stdout missing"));
            }
        };
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let output = std::thread::spawn(move || {
            for line in BufReader::new(stdout)
                .lines()
                .map_while(std::result::Result::ok)
            {
                if let Ok(value) = serde_json::from_str::<Value>(&line) {
                    if value["event"] == "ready" {
                        if let Some(port) = value["port"].as_u64() {
                            let _ = ready_tx.send(port);
                        }
                    }
                }
            }
        });
        let mut rig = Self {
            child,
            platform,
            output: Some(output),
            client,
            platform_origin,
            origin: String::new(),
        };
        let port =
            tokio::task::spawn_blocking(move || ready_rx.recv_timeout(Duration::from_secs(12)))
                .await
                .map_err(|_| anyhow!("notification readiness task failed"))?
                .map_err(|_| anyhow!("notification child did not report a listening port"))?;
        rig.origin = format!("http://127.0.0.1:{port}");
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let ready = rig
                .client
                .get(format!("{}/health/ready", rig.origin))
                .send()
                .await?;
            if ready.status().is_success() {
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "real platform/notification dependencies did not become ready"
            );
            tokio::task::yield_now().await;
        }
        Ok(rig)
    }
    async fn http(&self, token: &str, method: Method, path: &str, body: Value) -> Result<Reply> {
        let response = self
            .client
            .request(method, format!("{}{path}", self.platform_origin))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        Ok(Reply {
            status,
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        })
    }
    async fn idle(&self, expected_hints: u64) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut stable = 0;
        loop {
            let text = self
                .client
                .get(format!("{}/metrics", self.origin))
                .bearer_auth(METRICS)
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            let metric = |name: &str| -> Result<u64> {
                text.lines()
                    .find(|line| line.starts_with(&format!("{name}{{")))
                    .and_then(|line| line.split_whitespace().last())
                    .ok_or_else(|| anyhow!("notification processing metric missing"))?
                    .parse()
                    .map_err(Into::into)
            };
            if metric("saas_notification_hints_received_total")? >= expected_hints
                && metric("saas_notification_pending_hints")? == 0
                && metric("saas_notification_active_partitions")? == 0
            {
                stable += 1;
                if stable == 3 {
                    return Ok(());
                }
            } else {
                stable = 0;
            }
            ensure!(
                Instant::now() < deadline,
                "real notification hint processing never reached its bounded idle barrier"
            );
            tokio::task::yield_now().await;
        }
    }
    async fn shutdown(&mut self) -> Result<()> {
        drop(self.child.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait()? {
                ensure!(
                    status.success(),
                    "notification child failed graceful shutdown"
                );
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "notification child did not shut down after owned stdin EOF"
            );
            tokio::task::yield_now().await;
        }
        self.platform.abort();
        if let Some(output) = self.output.take() {
            output
                .join()
                .map_err(|_| anyhow!("notification output reader failed"))?;
        }
        Ok(())
    }
}

struct Polling {
    client: reqwest::Client,
    url: String,
    events: VecDeque<(String, Value)>,
    namespace: bool,
    closed: bool,
}
impl Polling {
    async fn open(rig: &ExternalNotifications) -> Result<Self> {
        let initial = format!("{}/socket.io/?EIO=4&transport=polling", rig.origin);
        let body = rig
            .client
            .get(&initial)
            .header("Origin", SOCKET_ORIGIN)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let open: Value = serde_json::from_str(
            body.strip_prefix('0')
                .ok_or_else(|| anyhow!("Engine.IO open packet missing"))?,
        )?;
        let sid = open["sid"]
            .as_str()
            .ok_or_else(|| anyhow!("Engine.IO sid missing"))?;
        let mut socket = Self {
            client: rig.client.clone(),
            url: format!("{initial}&sid={sid}"),
            events: VecDeque::new(),
            namespace: false,
            closed: false,
        };
        socket.post("40".to_owned()).await?;
        while !socket.namespace {
            socket.poll().await?;
            ensure!(
                !socket.closed,
                "Socket.IO default namespace did not connect"
            );
        }
        Ok(socket)
    }
    async fn post(&self, packet: String) -> Result<()> {
        self.client
            .post(&self.url)
            .header("Origin", SOCKET_ORIGIN)
            .header("Content-Type", "text/plain;charset=UTF-8")
            .body(packet)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
    async fn poll(&mut self) -> Result<()> {
        let response = self
            .client
            .get(&self.url)
            .header("Origin", SOCKET_ORIGIN)
            .send()
            .await?;
        if !response.status().is_success() {
            ensure!(
                matches!(response.status().as_u16(), 400 | 404 | 410),
                "notification polling failed outside retired-session statuses"
            );
            self.closed = true;
            return Ok(());
        }
        let body = response.text().await?;
        for packet in body.split('\u{1e}') {
            if packet == "2" {
                self.post("3".to_owned()).await?;
            } else if packet == "1" || packet == "41" {
                self.closed = true;
            } else if packet.starts_with("40") {
                self.namespace = true;
            } else if let Some(payload) = packet.strip_prefix("42") {
                let value: Value = serde_json::from_str(payload)?;
                self.events.push_back((
                    value[0]
                        .as_str()
                        .ok_or_else(|| anyhow!("Socket.IO event name missing"))?
                        .to_owned(),
                    value[1].clone(),
                ));
            } else {
                ensure!(
                    packet == "6" || packet.is_empty(),
                    "unknown Engine.IO packet in owned integration"
                );
            }
        }
        Ok(())
    }
    async fn next(&mut self) -> Result<Option<(String, Value)>> {
        loop {
            if let Some(event) = self.events.pop_front() {
                return Ok(Some(event));
            }
            if self.closed {
                return Ok(None);
            }
            self.poll().await?;
        }
    }
    async fn authenticate(&self, token: &str) -> Result<()> {
        self.post(format!("42{}", json!(["authenticate",{"token":token}])))
            .await
    }
    async fn accepted(
        rig: &ExternalNotifications,
        credentials: &Credentials,
        member: &Member,
    ) -> Result<Self> {
        let mut socket = Self::open(rig).await?;
        socket.authenticate(&credentials.session).await?;
        let (name, ack) = socket
            .next()
            .await?
            .ok_or_else(|| anyhow!("Socket.IO authentication acknowledgement missing"))?;
        ensure!(
            name == "authenticated"
                && keys(&ack)
                    == BTreeSet::from([
                        "contractVersion",
                        "userId",
                        "tenantId",
                        "organizationId",
                        "expiresAt"
                    ])
                && ack["contractVersion"] == "1.0"
                && ack["userId"] == json!(member.user)
                && ack["tenantId"] == json!(member.tenant)
                && ack["organizationId"] == json!(member.org),
            "actual platform proof failed or leaked private fields into public authenticated event"
        );
        Ok(socket)
    }
    async fn notification(&mut self, id: Uuid, member: &Member) -> Result<()> {
        let (name, event) = self
            .next()
            .await?
            .ok_or_else(|| anyhow!("expected actual Socket.IO notification missing"))?;
        ensure!(
            name == "notification"
                && event["id"] == json!(id)
                && event["userId"] == json!(member.user)
                && event["organizationId"] == json!(member.org),
            "another receipt identity reached the Socket.IO connection"
        );
        ensure!(
            keys(&event)
                == BTreeSet::from([
                    "contractVersion",
                    "id",
                    "userId",
                    "tenantId",
                    "organizationId",
                    "applicationId",
                    "publicationId",
                    "type",
                    "title",
                    "content",
                    "link",
                    "createdAt"
                ])
                && event["contractVersion"] == "1.0",
            "public Notification 1 Socket.IO payload acquired private proof fields"
        );
        Ok(())
    }
    async fn rejected(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(8);
        while let Some((name, _)) = self.next().await? {
            ensure!(
                name == "unauthorized",
                "revoked Socket.IO connection received a notification or renewed acknowledgement"
            );
            ensure!(
                Instant::now() < deadline,
                "revoked Socket.IO connection did not close"
            );
        }
        ensure!(self.closed, "revoked Engine.IO transport remained open");
        Ok(())
    }
}

async fn fixture_hint(pool: &PgPool, id: Uuid, member: &Member) -> Result<()> {
    sqlx::query("SELECT pg_notify('new_notification',$1)").bind(json!({"hintVersion":"1","id":id,"userId":member.user,"tenantId":member.tenant,"organizationId":member.org,"membershipId":member.id,"memberType":member.kind,"identityVersion":member.version}).to_string()).execute(pool).await?;
    Ok(())
}

async fn publish_over_http(
    rig: &ExternalNotifications,
    h: &Harness,
    key: &str,
    target: &Member,
) -> Result<Uuid> {
    let reply=rig.http(&h.admin.app,Method::POST,&format!("/api/apps/{CODE}/notifications"),json!({"idempotencyKey":key,"target":{"kind":"USER","userId":target.user,"organizationId":target.org},"type":"INFO","title":key,"content":"Actual platform HTTP publication","link":format!("/micro/{CODE}/owned")})).await?;
    expect(
        &reply,
        StatusCode::OK,
        "actual HTTP platform APP publication",
    )?;
    sqlx::query_scalar("SELECT id FROM notifications WHERE publication_id=$1 AND recipient_membership_id=$2 AND recipient_identity_version=$3").bind(reply.body["publicationId"].as_str().ok_or_else(||anyhow!("HTTP publication ID missing"))?.parse::<Uuid>()?).bind(target.id).bind(target.version).fetch_one(&h.pool).await.map_err(Into::into)
}

#[tokio::test]
#[ignore = "requires explicit Phase 4 database and IDENTITY_PHASE04_NOTIFICATION_BINARY; runs actual platform HTTP and external Socket.IO binary"]
async fn notification_identity_phase04_actual_platform_and_external_socketio_keep_receipt_identity(
) -> Result<()> {
    let db = Database::create().await?;
    let result = async {
        let h = Harness::create(&db.pool).await?;
        let employee = h
            .person("phase04-live-dual", h.child, "EMPLOYEE", "ENABLED")
            .await?;
        let other = member(
            &h.pool,
            employee.user,
            h.tenant,
            h.sibling,
            "EMPLOYEE",
            "ENABLED",
        )
        .await?;
        let employee_credentials = credentials(&h.pool, &employee, h.application).await?;
        let other_credentials = credentials(&h.pool, &other, h.application).await?;
        let customer = member(
            &h.pool,
            employee.user,
            h.tenant,
            h.child,
            "CUSTOMER",
            "ENABLED",
        )
        .await?;
        let customer_credentials = credentials(&h.pool, &customer, h.application).await?;
        let mut rig = ExternalNotifications::start(&db, h.app.clone()).await?;
        let mut customer_rejected = Polling::open(&rig).await?;
        customer_rejected
            .authenticate(&customer_credentials.session)
            .await?;
        customer_rejected.rejected().await?;
        let mut employee_socket = Polling::accepted(&rig, &employee_credentials, &employee).await?;
        let mut other_socket = Polling::accepted(&rig, &other_credentials, &other).await?;
        let employee_id =
            publish_over_http(&rig, &h, "phase04-live-employee-first", &employee).await?;
        employee_socket.notification(employee_id, &employee).await?;
        rig.idle(1).await?;
        let other_id = fixture_notification(&h.pool, &other, "phase04-live-explicit-other").await?;
        fixture_hint(&h.pool, other_id, &other).await?;
        other_socket.notification(other_id, &other).await?;
        rig.idle(2).await?;
        let employee_sentinel =
            publish_over_http(&rig, &h, "phase04-live-employee-sentinel", &employee).await?;
        employee_socket
            .notification(employee_sentinel, &employee)
            .await?;
        rig.idle(3).await?;
        expect(
            &rig.http(
                &h.admin.session,
                Method::PUT,
                &format!("/api/memberships/{}/status", employee.id),
                json!({"expectedIdentityVersion":1,"status":"DISABLED","reason":REASON}),
            )
            .await?,
            StatusCode::OK,
            "actual HTTP employee revocation while sockets connected",
        )?;
        let other_sentinel =
            fixture_notification(&h.pool, &other, "phase04-live-other-after-employee-stop").await?;
        fixture_hint(&h.pool, other_sentinel, &other).await?;
        other_socket.notification(other_sentinel, &other).await?;
        rig.idle(4).await?;
        employee_socket.rejected().await?;
        let mut reconnect = Polling::open(&rig).await?;
        reconnect
            .authenticate(&employee_credentials.session)
            .await?;
        reconnect.rejected().await?;
        expect(
            &rig.http(
                &h.admin.session,
                Method::PUT,
                &format!("/api/memberships/{}/status", employee.id),
                json!({"expectedIdentityVersion":2,"status":"ENABLED","reason":REASON}),
            )
            .await?,
            StatusCode::OK,
            "actual HTTP employee new generation",
        )?;
        let returned = Member {
            version: 3,
            ..employee.clone()
        };
        let fresh = credentials(&h.pool, &returned, h.application).await?;
        let mut fresh_socket = Polling::accepted(&rig, &fresh, &returned).await?;
        fixture_hint(&h.pool, employee_id, &employee).await?;
        rig.idle(5).await?;
        let fresh_id =
            publish_over_http(&rig, &h, "phase04-live-current-generation", &returned).await?;
        fresh_socket.notification(fresh_id, &returned).await?;
        rig.idle(6).await?;
        expect(
            &rig.http(
                &h.admin.session,
                Method::PUT,
                &format!("/api/memberships/{}/status", other.id),
                json!({"expectedIdentityVersion":1,"status":"DISABLED","reason":REASON}),
            )
            .await?,
            StatusCode::OK,
            "actual HTTP reverse other revocation",
        )?;
        other_socket.rejected().await?;
        expect(
            &rig.http(
                &h.admin.session,
                Method::PUT,
                &format!("/api/memberships/{}/status", customer.id),
                json!({"expectedIdentityVersion":1,"status":"DISABLED","reason":REASON}),
            )
            .await?,
            StatusCode::OK,
            "customer revocation does not affect accepted employee socket",
        )?;
        let survivor = publish_over_http(
            &rig,
            &h,
            "phase04-live-employee-after-other-stop",
            &returned,
        )
        .await?;
        fresh_socket.notification(survivor, &returned).await?;
        rig.idle(7).await?;
        expect(
            &rig.http(
                &fresh.session,
                Method::GET,
                &format!("{PROOF_PATH}?notificationId={employee_id}"),
                Value::Null,
            )
            .await?,
            StatusCode::NOT_FOUND,
            "actual HTTP replay proof denies old employee receipt generation",
        )?;
        rig.shutdown().await?;
        drop(rig);
        Ok(())
    }
    .await;
    db.finish(result).await
}
