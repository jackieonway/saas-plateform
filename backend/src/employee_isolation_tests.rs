//! Phase 3 employee authorization contracts on owned disposable PostgreSQL databases.
//! Only IDENTITY_PHASE03_TEST_DATABASE_URL is read; no business .env or preview fallback.
use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use axum::{middleware, Router};
use serde_json::{json, Value};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::config::Config;
use crate::state::AppState;
use crate::util::{hash, jwt, uuid7::new_uuid};

const SECRET: &str = "owned-phase03-employee-isolation-fixture-only";
const CODE: &str = "phase03-employees";
const ORIGIN: &str = "phase03-owned-human";
const SERVICE: &str = "phase03-owned-service";
const REASON: &str = "Owned Phase 3 employee isolation contract mutation";

struct Database {
    pool: PgPool,
    admin: PgPool,
    name: String,
}

impl Database {
    async fn create() -> Result<Self> {
        let url = std::env::var("IDENTITY_PHASE03_TEST_DATABASE_URL")
            .map_err(|_| anyhow!("explicit IDENTITY_PHASE03_TEST_DATABASE_URL is required"))?;
        let options = PgConnectOptions::from_str(&url)
            .map_err(|_| anyhow!("invalid owned Phase 3 database configuration"))?;
        ensure!(
            options.get_database() == Some("postgres"),
            "Phase 3 administration must explicitly select postgres"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("owned Phase 3 administration connection failed"))?;
        let name = format!("identity_phase03_{}", new_uuid().simple());
        ensure!(
            name.len() <= 63,
            "owned Phase 3 database name exceeds limit"
        );
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await?;
        let pool = match PgPoolOptions::new()
            .max_connections(10)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.database(&name))
            .await
        {
            Ok(pool) => pool,
            Err(_) => {
                let _ = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
                    .execute(&admin)
                    .await;
                return Err(anyhow!("owned Phase 3 fixture connection failed"));
            }
        };
        let db = Self { pool, admin, name };
        if let Err(error) = sqlx::migrate!("./migrations").run(&db.pool).await {
            let _ = db.finish(Ok(())).await;
            return Err(error.into());
        }
        // This is exclusively target-schema test preparation, not a published
        // Phase 6 contraction migration and never touches the default schema.
        if let Err(error) = sqlx::query(
            "ALTER TABLE memberships DROP CONSTRAINT IF EXISTS memberships_user_id_organization_id_key",
        )
        .execute(&db.pool)
        .await
        {
            let _ = db.finish(Ok(())).await;
            return Err(error.into());
        }
        Ok(db)
    }

    async fn finish(self, result: Result<()>) -> Result<()> {
        ensure!(
            self.name.starts_with("identity_phase03_")
                && self.name.len() <= 63
                && self
                    .name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_'),
            "refusing cleanup outside owned Phase 3 fixture"
        );
        let closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let removed = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await;
        self.admin.close().await;
        eprintln!("identity_phase03_cleanup database={} assertions_ok={} database_removed={} pool_closed={closed}",self.name,result.is_ok(),removed.is_ok());
        result?;
        removed?;
        ensure!(closed, "owned Phase 3 pool did not close");
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
        .extension(ConnectInfo("192.0.2.43:30043".parse::<SocketAddr>()?))
        .body(Body::from(serde_json::to_vec(&body)?))?;
    let response = tokio::time::timeout(Duration::from_secs(25), app.clone().oneshot(request))
        .await
        .map_err(|_| anyhow!("owned Phase 3 HTTP request timed out: {path}"))??;
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
        "{operation}: expected {status}, got {} (error={})",
        reply.status,
        reply
            .body
            .get("code")
            .or_else(|| reply.body.get("error"))
            .unwrap_or(&Value::Null)
    );
    Ok(())
}

fn rejected(reply: &Reply, operation: &str) -> Result<()> {
    ensure!(
        matches!(
            reply.status,
            StatusCode::BAD_REQUEST
                | StatusCode::UNAUTHORIZED
                | StatusCode::FORBIDDEN
                | StatusCode::NOT_FOUND
        ),
        "{operation}: expected an explicit authorization/target rejection, got {}",
        reply.status
    );
    Ok(())
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
}

async fn user(pool: &PgPool, label: &str, super_admin: bool) -> Result<Uuid> {
    let id = new_uuid();
    sqlx::query("INSERT INTO users(id,username,password_hash,is_super_admin,real_name) VALUES($1,$2,'owned-unusable-phase03-password',$3,$2)")
        .bind(id).bind(label).bind(super_admin).execute(pool).await?;
    Ok(id)
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

async fn role(pool: &PgPool, tenant: Uuid, org: Uuid, label: &str, menus: &[Uuid]) -> Result<Uuid> {
    let id = new_uuid();
    sqlx::query("INSERT INTO roles(id,tenant_id,organization_id,name,code) VALUES($1,$2,$3,$4,$4)")
        .bind(id)
        .bind(tenant)
        .bind(org)
        .bind(label)
        .execute(pool)
        .await?;
    for menu in menus {
        sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)")
            .bind(id)
            .bind(menu)
            .execute(pool)
            .await?;
    }
    Ok(id)
}

async fn attach_role(pool: &PgPool, subject: Uuid, role: Uuid, org: Uuid) -> Result<()> {
    sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
        .bind(subject).bind(role).bind(org).execute(pool).await?;
    Ok(())
}

async fn has_role(pool: &PgPool, subject: Uuid, role: Uuid) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM user_roles WHERE user_id=$1 AND role_id=$2)",
    )
    .bind(subject)
    .bind(role)
    .fetch_one(pool)
    .await?)
}

async fn active(pool: &PgPool, sid: Uuid) -> Result<bool> {
    Ok(sqlx::query_scalar("SELECT platform_session_active($1)")
        .bind(sid)
        .fetch_one(pool)
        .await?)
}

async fn platform_refresh(pool: &PgPool, sid: Uuid) -> Result<String> {
    let mut tx = pool.begin().await?;
    let token = crate::session::issue_refresh_in_tx(
        &mut tx,
        sid,
        0,
        chrono::Utc::now() + chrono::Duration::hours(1),
    )
    .await?;
    tx.commit().await?;
    Ok(token)
}

async fn refresh_session(app: &Router, token: &str) -> Result<Reply> {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/oauth/token")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header("x-request-id", new_uuid().to_string())
        .extension(ConnectInfo("192.0.2.43:30043".parse::<SocketAddr>()?))
        .body(Body::from(format!(
            "grant_type=refresh_token&refresh_token={}",
            percent_encoding::utf8_percent_encode(token, percent_encoding::NON_ALPHANUMERIC)
        )))?;
    let response = tokio::time::timeout(Duration::from_secs(25), app.clone().oneshot(request))
        .await
        .map_err(|_| anyhow!("owned Phase 3 platform refresh timed out"))??;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok(Reply {
        status,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    })
}

async fn credentials(pool: &PgPool, member: &Member, application: Uuid) -> Result<Credentials> {
    let sid = new_uuid();
    sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version) \
        VALUES($1,$2,$3,$4,now()+interval '1 hour',0,$5,$6,$7,'urn:saas:legacy-platform',$8,1)")
        .bind(sid).bind(member.user).bind(member.tenant).bind(member.org).bind(member.id).bind(member.kind).bind(member.version).bind(ORIGIN).execute(pool).await?;
    let now = chrono::Utc::now().timestamp();
    let session = jwt::sign(
        &jwt::Claims {
            sub: member.user.to_string(),
            typ: "session".into(),
            tenant: Some(member.tenant.to_string()),
            org: Some(member.org.to_string()),
            sid: Some(sid.to_string()),
            ver: Some(0),
            cid: None,
            scope: None,
            aud: None,
            jti: None,
            iat: now,
            exp: now + 3600,
        },
        SECRET,
    )?;
    let jti = new_uuid();
    let app = jwt::sign(
        &jwt::Claims {
            sub: member.user.to_string(),
            typ: "access".into(),
            tenant: Some(member.tenant.to_string()),
            org: Some(member.org.to_string()),
            sid: None,
            ver: None,
            cid: Some(SERVICE.into()),
            scope: Some("profile".into()),
            aud: Some(CODE.into()),
            jti: Some(jti.to_string()),
            iat: now,
            exp: now + 300,
        },
        SECRET,
    )?;
    sqlx::query("INSERT INTO oauth_tokens(id,token_hash,token_type,user_id,client_id,tenant_id,organization_id,expires_at,token_use,audience,jti,application_id,platform_session_id,storage_version,account_version,membership_id,member_type,identity_version,client_policy_version,application_policy_version,origin_issuer,origin_client_ref,origin_client_policy_version,application_service_client_ref) \
        VALUES($1,$2,'access',$3,$4,$5,$6,now()+interval '5 minutes','app',$7,$8,$9,$10,2,0,$11,$12,$13,1,1,'urn:saas:legacy-platform',$14,1,$4)")
        .bind(new_uuid()).bind(hash::sha256_hex(&app)).bind(member.user).bind(SERVICE).bind(member.tenant).bind(member.org)
        .bind(CODE).bind(jti).bind(application).bind(sid).bind(member.id).bind(member.kind).bind(member.version).bind(ORIGIN).execute(pool).await?;
    Ok(Credentials { sid, session, app })
}

struct Harness {
    pool: PgPool,
    app: Router,
    tenant: Uuid,
    root: Uuid,
    child: Uuid,
    sibling: Uuid,
    application: Uuid,
    menus: Vec<Uuid>,
    admin: Credentials,
    ancestor_role: Uuid,
}

impl Harness {
    async fn create(pool: &PgPool) -> Result<Self> {
        let tenant = new_uuid();
        sqlx::query("INSERT INTO tenants(id,name,code,is_platform) VALUES($1,'Owned Phase 3','owned-phase03',true)")
            .bind(tenant).execute(pool).await?;
        let root = organization(pool, tenant, None, "Phase 3 root").await?;
        let child = organization(pool, tenant, Some(root), "Phase 3 child").await?;
        let sibling = organization(pool, tenant, Some(root), "Phase 3 sibling").await?;
        let system = new_uuid();
        let application = new_uuid();
        sqlx::query("INSERT INTO applications(id,name,code,base_path) VALUES($1,'Phase 3 management','system','/system')").bind(system).execute(pool).await?;
        sqlx::query("INSERT INTO applications(id,name,code,base_path,allowed_member_types) VALUES($1,'Phase 3 business',$2,'/micro/phase03',ARRAY['EMPLOYEE','CUSTOMER']::text[])")
            .bind(application).bind(CODE).execute(pool).await?;
        let mut menus = Vec::new();
        for (app, permission) in [
            (system, "system:user"),
            (system, "system:role"),
            (system, "system:data-grant:read"),
            (system, "system:data-grant:write"),
            (application, "ticket:create"),
        ] {
            let id = new_uuid();
            sqlx::query("INSERT INTO menus(id,application_id,type,title,permission,visible) VALUES($1,$2,'BUTTON',$3,$3,false)")
                .bind(id).bind(app).bind(permission).execute(pool).await?;
            menus.push(id);
        }
        for app in [system, application] {
            sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
                .bind(tenant)
                .bind(app)
                .execute(pool)
                .await?;
            for org in [root, child, sibling] {
                sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)").bind(org).bind(app).execute(pool).await?;
            }
        }
        for menu in &menus {
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
        sqlx::query("INSERT INTO oauth_clients(id,client_id,client_name,client_type,grant_types,subject_mode,allowed_member_types) VALUES($1,$2,'Owned Phase 3 human','PUBLIC','authorization_code','USER',ARRAY['EMPLOYEE','CUSTOMER']::text[])")
            .bind(new_uuid()).bind(ORIGIN).execute(pool).await?;
        sqlx::query("INSERT INTO oauth_clients(id,client_id,client_name,client_type,client_secret_hash,grant_types,subject_mode,allowed_member_types,application_id) VALUES($1,$2,'Owned Phase 3 service','CONFIDENTIAL',$3,'client_credentials','MACHINE',ARRAY[]::text[],$4)")
            .bind(new_uuid()).bind(SERVICE).bind(hash::sha256_hex("owned-phase03-service-secret")).bind(application).execute(pool).await?;
        let manager = user(pool, "phase03-manager", true).await?;
        let manager = member(pool, manager, tenant, root, "EMPLOYEE", "ENABLED").await?;
        let admin = credentials(pool, &manager, application).await?;
        let ancestor_role = role(pool, tenant, root, "phase03-ancestor-role", &menus).await?;
        let policy = json!({"applications":[{"applicationId":application,"capabilities":[{"resource":"ticket","action":"create","scopes":["SELF","ORG","ORG_SUBTREE","TENANT"]}],
            "memberLookups":[{"resource":"ticket","action":"create"}]}],
            "managementScopes":[{"tenantId":tenant,"organizationId":root,"includeDescendants":true,"read":true,"write":true}]});
        let config = Config::from_values(&HashMap::from([
            ("APP_ENV".into(), "test".into()),
            (
                "DATABASE_URL".into(),
                "postgres://unused/owned-phase03".into(),
            ),
            ("JWT_SECRET".into(), SECRET.into()),
            ("MEMBERSHIP_LIFECYCLE_ENABLED".into(), "true".into()),
            ("CUSTOMER_MEMBERSHIPS_ENABLED".into(), "true".into()),
            ("DUAL_MEMBERSHIPS_ENABLED".into(), "true".into()),
            ("DATA_GRANT_POLICY".into(), policy.to_string()),
        ]))?;
        let state = AppState::new(pool.clone(), Arc::new(config));
        let app = crate::routes::api_router()
            .nest("/oauth", crate::routes::oauth2::oauth_router())
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
            root,
            child,
            sibling,
            application,
            menus,
            admin,
            ancestor_role,
        })
    }

    async fn person(
        &self,
        label: &str,
        org: Uuid,
        kind: &'static str,
        status: &str,
        super_admin: bool,
    ) -> Result<Member> {
        member(
            &self.pool,
            user(&self.pool, label, super_admin).await?,
            self.tenant,
            org,
            kind,
            status,
        )
        .await
    }

    async fn grant(&self, target: &Member) -> Result<()> {
        sqlx::query("INSERT INTO membership_data_grants(id,membership_id,application_id,resource_code,action_code,scope_type) VALUES($1,$2,$3,'ticket','create','ORG')")
            .bind(new_uuid()).bind(target.id).bind(self.application).execute(&self.pool).await?;
        Ok(())
    }

    async fn api(&self, method: Method, path: &str, body: Value) -> Result<Reply> {
        call(&self.app, &self.admin.session, method, path, body).await
    }

    fn target(&self, member: &Member) -> Value {
        json!({"tenantId":member.tenant,"organizationId":member.org,"userId":member.user,"membershipId":member.id,"applicationId":self.application})
    }

    fn target_path(&self, member: &Member) -> String {
        format!("/api/data-grants?tenantId={}&organizationId={}&userId={}&membershipId={}&applicationId={}",member.tenant,member.org,member.user,member.id,self.application)
    }

    async fn status(&self, member: &Member, status: &str) -> Result<Reply> {
        self.api(
            Method::PUT,
            &format!("/api/memberships/{}/status", member.id),
            json!({"expectedIdentityVersion":member.version,"status":status,"reason":REASON}),
        )
        .await
    }

    async fn bulk(&self, subject: Uuid, orgs: &[Uuid]) -> Result<Reply> {
        self.api(Method::PUT,&format!("/api/users/{subject}/memberships"),json!({"items":orgs.iter().map(|org|json!({"organizationId":org,"position":"Owned Phase 3 employee"})).collect::<Vec<_>>()})).await
    }
}

#[tokio::test]
#[ignore = "creates and drops identity_phase03_* databases using explicit IDENTITY_PHASE03_TEST_DATABASE_URL"]
async fn employee_isolation_phase03_customer_actor_and_directory_targets() -> Result<()> {
    let db = Database::create().await?;
    let result = async {
        let h = Harness::create(&db.pool).await?;
        let employee = h
            .person("phase03-dual-super", h.child, "EMPLOYEE", "ENABLED", true)
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
        attach_role(&h.pool, employee.user, h.ancestor_role, h.root).await?;
        h.grant(&employee).await?;
        let employee_token = credentials(&h.pool, &employee, h.application).await?;
        let customer_token = credentials(&h.pool, &customer, h.application).await?;
        ensure!(
            active(&h.pool, customer_token.sid).await?,
            "customer denial fixture is not a valid bound session"
        );
        let customer_claims = jwt::verify(&customer_token.app, SECRET)?;
        ensure!(
            crate::app_token::inspect(&h.pool, &customer_token.app, &customer_claims)
                .await?
                .is_some(),
            "customer denial fixture is not a valid admitted app token"
        );
        let customer_only = h
            .person(
                "phase03-customer-target",
                h.child,
                "CUSTOMER",
                "ENABLED",
                false,
            )
            .await?;
        let disabled = h
            .person(
                "phase03-disabled-target",
                h.child,
                "EMPLOYEE",
                "DISABLED",
                false,
            )
            .await?;
        let disabled_customer = member(
            &h.pool,
            disabled.user,
            h.tenant,
            h.child,
            "CUSTOMER",
            "ENABLED",
        )
        .await?;
        let plain = h
            .person(
                "phase03-plain-employee",
                h.child,
                "EMPLOYEE",
                "ENABLED",
                false,
            )
            .await?;
        let roles = call(
            &h.app,
            &employee_token.session,
            Method::GET,
            "/api/auth/permissions",
            Value::Null,
        )
        .await?;
        expect(
            &roles,
            StatusCode::OK,
            "dual employee can read employee permissions",
        )?;
        ensure!(
            roles.body.to_string().contains("system:user"),
            "employee ancestor/super authorization was lost because a customer row exists"
        );
        for path in [
            "/api/users".to_owned(),
            "/api/roles".to_owned(),
            "/api/auth/permissions".to_owned(),
            "/api/auth/menus".to_owned(),
            h.target_path(&employee),
            "/api/data-grants/options".to_owned(),
        ] {
            expect(
                &call(
                    &h.app,
                    &customer_token.session,
                    Method::GET,
                    &path,
                    Value::Null,
                )
                .await?,
                StatusCode::UNAUTHORIZED,
                "customer session cannot enter employee APIs",
            )?;
        }
        let context = format!("/api/apps/{CODE}/context");
        let list = format!(
            "/api/apps/{CODE}/members?resource=ticket&action=create&organizationId={}",
            h.child
        );
        expect(
            &call(
                &h.app,
                &customer_token.app,
                Method::GET,
                &context,
                Value::Null,
            )
            .await?,
            StatusCode::FORBIDDEN,
            "customer app context cannot inherit super/ancestor employee authorization",
        )?;
        expect(
            &call(&h.app, &customer_token.app, Method::GET, &list, Value::Null).await?,
            StatusCode::FORBIDDEN,
            "customer cannot call employee member directory",
        )?;
        expect(
            &call(
                &h.app,
                &employee_token.app,
                Method::GET,
                &context,
                Value::Null,
            )
            .await?,
            StatusCode::OK,
            "dual employee application context",
        )?;
        let reply = call(&h.app, &employee_token.app, Method::GET, &list, Value::Null).await?;
        expect(&reply, StatusCode::OK, "employee directory list")?;
        ensure!(
            reply.body["total"] == 2,
            "employee directory count included customer, disabled or duplicate dual rows"
        );
        let records = reply.body["records"]
            .as_array()
            .ok_or_else(|| anyhow!("member directory lacks records"))?;
        let ids: BTreeSet<Uuid> = records
            .iter()
            .map(|row| {
                row["id"]
                    .as_str()
                    .ok_or_else(|| anyhow!("directory row lacks id"))?
                    .parse()
                    .map_err(Into::into)
            })
            .collect::<Result<_>>()?;
        ensure!(
            ids == BTreeSet::from([employee.user, plain.user]),
            "employee directory returned wrong targets"
        );
        for (id, allowed) in [
            (employee.user, true),
            (plain.user, true),
            (customer_only.user, false),
            (customer_only.id, false),
            (customer.id, false),
            (disabled.user, false),
            (disabled_customer.id, false),
        ] {
            let path = format!(
                "/api/apps/{CODE}/members/{id}?resource=ticket&action=create&organizationId={}",
                h.child
            );
            expect(
                &call(&h.app, &employee_token.app, Method::GET, &path, Value::Null).await?,
                if allowed {
                    StatusCode::OK
                } else {
                    StatusCode::NOT_FOUND
                },
                "direct target submission follows directory employee rule",
            )?;
            expect(
                &call(&h.app, &customer_token.app, Method::GET, &path, Value::Null).await?,
                StatusCode::FORBIDDEN,
                "customer caller cannot bypass directory using a direct target ID",
            )?;
        }
        Ok(())
    }
    .await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops identity_phase03_* databases using explicit IDENTITY_PHASE03_TEST_DATABASE_URL"]
async fn employee_isolation_phase03_bidirectional_roles_and_data_grants() -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let h=Harness::create(&db.pool).await?;
        let employee=h.person("phase03-dual-role-target",h.child,"EMPLOYEE","ENABLED",false).await?;
        let customer=member(&h.pool,employee.user,h.tenant,h.child,"CUSTOMER","ENABLED").await?;
        let only_customer=h.person("phase03-only-customer-role-target",h.child,"CUSTOMER","ENABLED",false).await?;
        let disabled=h.person("phase03-disabled-role-target",h.child,"EMPLOYEE","DISABLED",false).await?;
        member(&h.pool,disabled.user,h.tenant,h.child,"CUSTOMER","ENABLED").await?;
        let selected_role=role(&h.pool,h.tenant,h.root,"phase03-selected-role",&h.menus).await?;
        for target in [&only_customer,&disabled] {
            rejected(&h.api(Method::PUT,&format!("/api/users/{}/roles",target.user),json!({"items":[{"roleId":selected_role,"organizationId":h.root}]})).await?,"user-to-role rejects absent employee qualification")?;
            rejected(&h.api(Method::PUT,&format!("/api/roles/{selected_role}/users"),json!({"userIds":[target.user]})).await?,"role-to-user rejects absent employee qualification")?;
            ensure!(!has_role(&h.pool,target.user,selected_role).await?,"failed employee-role assignment wrote a binding");
        }
        expect(&h.api(Method::PUT,&format!("/api/users/{}/roles",employee.user),json!({"items":[{"roleId":selected_role,"organizationId":h.root}]})).await?,StatusCode::OK,"dual user-to-role resolves valid employee")?;
        expect(&h.api(Method::PUT,&format!("/api/roles/{selected_role}/users"),json!({"userIds":[employee.user]})).await?,StatusCode::OK,"dual role-to-user resolves valid employee")?;
        let employee_credentials=credentials(&h.pool,&employee,h.application).await?;
        let customer_credentials=credentials(&h.pool,&customer,h.application).await?;
        expect(&call(&h.app,&employee_credentials.session,Method::GET,"/api/auth/permissions",Value::Null).await?,StatusCode::OK,"assigned employee role is usable")?;
        expect(&call(&h.app,&customer_credentials.session,Method::GET,"/api/auth/permissions",Value::Null).await?,StatusCode::UNAUTHORIZED,"same-account customer cannot use assigned employee role")?;
        // An old-format stale binding is an owned negative fixture, not a
        // production grant writer. Employee read paths must not expose it.
        attach_role(&h.pool,only_customer.user,selected_role,h.root).await?;
        attach_role(&h.pool,disabled.user,selected_role,h.root).await?;
        let customer_roles=h.api(Method::GET,&format!("/api/users/{}/roles",only_customer.user),Value::Null).await?;
        expect(&customer_roles,StatusCode::OK,"customer-only role projection")?;
        ensure!(customer_roles.body==json!([]),"customer-only query exposed stale employee roles");
        let role_users=h.api(Method::GET,&format!("/api/roles/{selected_role}/users"),Value::Null).await?;
        expect(&role_users,StatusCode::OK,"role user projection")?;
        ensure!(role_users.body.as_array().is_some_and(|rows|rows.iter().all(|row|row["userId"]!=json!(only_customer.user))),"platform-root role projection leaked customer-only qualification");
        ensure!(role_users.body.as_array().is_some_and(|rows|rows.iter().all(|row|row["userId"]!=json!(disabled.user))),"platform-root role projection treated disabled configuration as employee qualification");
        let role_list=h.api(Method::GET,"/api/roles",Value::Null).await?;
        expect(&role_list,StatusCode::OK,"role employee counts")?;
        let counted=role_list.body["records"].as_array().and_then(|rows|rows.iter().find(|row|row["id"]==json!(selected_role))).ok_or_else(||anyhow!("role count fixture not returned"))?;
        ensure!(counted["userCount"]==1,"role userCount included customer-only or disabled employee bindings");
        let disabled_roles=h.api(Method::GET,&format!("/api/users/{}/roles",disabled.user),Value::Null).await?;
        expect(&disabled_roles,StatusCode::OK,"disabled employee role projection")?;
        ensure!(disabled_roles.body==json!([])&&has_role(&h.pool,disabled.user,selected_role).await?,"disabled role projection either exposed or deleted preserved configuration");
        let snapshot=h.api(Method::GET,&h.target_path(&employee),Value::Null).await?;
        expect(&snapshot,StatusCode::OK,"dual employee exact data-grant target")?;
        for (target,eligible) in [(&employee,true),(&only_customer,false),(&disabled,false)] {
            let options=h.api(Method::GET,&format!("/api/data-grants/options?applicationId={}&userId={}",h.application,target.user),Value::Null).await?;
            expect(&options,StatusCode::OK,"data grant target options")?;
            ensure!(options.body["total"]==if eligible{1}else{0},"data-grant target options counted customer, disabled or duplicate dual members");
            if eligible {
                ensure!(options.body["records"][0]["target"]["membershipId"]==json!(employee.id),"data grant options selected the same-account customer row");
            }
        }
        for target in [&customer,&only_customer,&disabled] {
            let denied=h.api(Method::GET,&h.target_path(target),Value::Null).await?;
            ensure!(matches!(denied.status,StatusCode::FORBIDDEN|StatusCode::NOT_FOUND),"data-grant read did not reject the ineligible target");
            let change=json!({"target":h.target(target),"expectedVersion":snapshot.body["version"],"upsert":[{"resource":"ticket","action":"create","scope":"ORG"}],"remove":[]});
            for (method,path) in [(Method::POST,"/api/data-grants/preview"),(Method::PATCH,"/api/data-grants")] {
                let denied=h.api(method,path,change.clone()).await?;
                ensure!(matches!(denied.status,StatusCode::FORBIDDEN|StatusCode::NOT_FOUND),"data-grant mutation did not reject the ineligible target before version processing: {}",denied.status);
            }
            let count:i64=sqlx::query_scalar("SELECT count(*) FROM membership_data_grants WHERE membership_id=$1").bind(target.id).fetch_one(&h.pool).await?;
            ensure!(count==0,"rejected customer/disabled data grant mutated storage");
        }
        let change=json!({"target":h.target(&employee),"expectedVersion":snapshot.body["version"],"upsert":[{"resource":"ticket","action":"create","scope":"ORG"}],"remove":[]});
        expect(&h.api(Method::POST,"/api/data-grants/preview",change.clone()).await?,StatusCode::OK,"valid employee data-grant preview")?;
        expect(&h.api(Method::PATCH,"/api/data-grants",change).await?,StatusCode::OK,"valid employee data-grant write")?;
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops identity_phase03_* databases using explicit IDENTITY_PHASE03_TEST_DATABASE_URL"]
async fn employee_isolation_phase03_bulk_preserves_other_identities_and_role_support() -> Result<()>
{
    let db = Database::create().await?;
    let result=async {
        let h=Harness::create(&db.pool).await?;
        let employee=h.person("phase03-bulk-dual",h.child,"EMPLOYEE","ENABLED",false).await?;
        let customer=member(&h.pool,employee.user,h.tenant,h.child,"CUSTOMER","ENABLED").await?;
        let other=member(&h.pool,employee.user,h.tenant,h.sibling,"EMPLOYEE","ENABLED").await?;
        attach_role(&h.pool,employee.user,h.ancestor_role,h.root).await?;
        let employee_credentials=credentials(&h.pool,&employee,h.application).await?;
        let customer_credentials=credentials(&h.pool,&customer,h.application).await?;
        let other_credentials=credentials(&h.pool,&other,h.application).await?;
        expect(&h.status(&employee,"DISABLED").await?,StatusCode::OK,"temporary employee disable")?;
        ensure!(has_role(&h.pool,employee.user,h.ancestor_role).await?,"temporary disable deleted preserved role configuration");
        expect(&call(&h.app,&employee_credentials.session,Method::GET,"/api/auth/permissions",Value::Null).await?,StatusCode::UNAUTHORIZED,"disabled employee cannot use preserved roles")?;
        ensure!(active(&h.pool,customer_credentials.sid).await?&&active(&h.pool,other_credentials.sid).await?,"temporary employee disable crossed identity boundary");
        expect(&h.bulk(employee.user,&[h.child,h.sibling]).await?,StatusCode::OK,"old bulk preserves disabled employee row")?;
        let state:(Uuid,String,i64)=sqlx::query_as("SELECT id,status,identity_version FROM memberships WHERE id=$1").bind(employee.id).fetch_one(&h.pool).await?;
        ensure!(state==(employee.id,"DISABLED".into(),2),"old bulk implicitly enabled or replaced disabled employee");
        let added=h.person("phase03-bulk-customer-only",h.child,"CUSTOMER","ENABLED",false).await?;
        let created=h.bulk(added.user,&[h.child]).await?;
        expect(&created,StatusCode::OK,"old bulk adds independent employee beside existing customer in target schema")?;
        let employee_id:Uuid=sqlx::query_scalar("SELECT id FROM memberships WHERE user_id=$1 AND organization_id=$2 AND member_type='EMPLOYEE' AND status='ENABLED'").bind(added.user).bind(h.child).fetch_one(&h.pool).await?;
        ensure!(employee_id!=added.id,"old bulk changed a customer row into an employee");
        let target_orgs=[h.child];
        let (one,two)=tokio::join!(h.bulk(added.user,&target_orgs),h.bulk(added.user,&target_orgs));
        expect(&one?,StatusCode::OK,"first concurrent employee bulk")?;expect(&two?,StatusCode::OK,"second concurrent employee bulk")?;
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM memberships WHERE user_id=$1 AND organization_id=$2").bind(added.user).bind(h.child).fetch_one(&h.pool).await?;
        ensure!(count==2,"concurrent employee bulk duplicated or deleted an identity");
        expect(&h.bulk(added.user,&[]).await?,StatusCode::OK,"employee bulk removal keeps customer")?;
        let retained:Uuid=sqlx::query_scalar("SELECT id FROM memberships WHERE user_id=$1").bind(added.user).fetch_one(&h.pool).await?;
        ensure!(retained==added.id,"bulk removal changed customer identity");
        expect(&h.bulk(employee.user,&[h.sibling]).await?,StatusCode::OK,"bulk removes only one employee")?;
        ensure!(has_role(&h.pool,employee.user,h.ancestor_role).await?,"bulk removed ancestor role still supported by another active employee");
        ensure!(active(&h.pool,customer_credentials.sid).await?&&active(&h.pool,other_credentials.sid).await?,"bulk removal revoked retained member session");
        expect(&h.bulk(employee.user,&[]).await?,StatusCode::OK,"bulk removes final employee")?;
        ensure!(!has_role(&h.pool,employee.user,h.ancestor_role).await?,"bulk left unsupported ancestor role configuration");
        ensure!(active(&h.pool,customer_credentials.sid).await?&&!active(&h.pool,other_credentials.sid).await?,"bulk final employee removal did not retain customer independently");
        let version:i64=sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1").bind(employee.user).fetch_one(&h.pool).await?;
        ensure!(version==0,"local bulk employee maintenance performed global account revocation");
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops identity_phase03_* databases using explicit IDENTITY_PHASE03_TEST_DATABASE_URL"]
async fn employee_isolation_phase03_permanent_delete_and_replace_keep_supported_roles() -> Result<()>
{
    let db = Database::create().await?;
    let result=async {
        let h=Harness::create(&db.pool).await?;
        for replacing in [false,true] {
            let employee=h.person(if replacing{"phase03-replace-dual"}else{"phase03-delete-dual"},h.child,"EMPLOYEE","ENABLED",false).await?;
            let customer=member(&h.pool,employee.user,h.tenant,h.child,"CUSTOMER","ENABLED").await?;
            let other=member(&h.pool,employee.user,h.tenant,h.sibling,"EMPLOYEE","ENABLED").await?;
            let child_role=role(&h.pool,h.tenant,h.child,if replacing{"phase03-replace-child-role"}else{"phase03-delete-child-role"},&h.menus).await?;
            attach_role(&h.pool,employee.user,h.ancestor_role,h.root).await?;
            attach_role(&h.pool,employee.user,child_role,h.child).await?;
            let employee_credentials=credentials(&h.pool,&employee,h.application).await?;
            let customer_credentials=credentials(&h.pool,&customer,h.application).await?;
            let other_credentials=credentials(&h.pool,&other,h.application).await?;
            let changed=if replacing {
                h.api(Method::POST,&format!("/api/memberships/{}/replace-identity",employee.id),json!({"memberType":"CUSTOMER","expectedIdentityVersion":1,"expectedTargetIdentityVersion":1,
                    "accountBindingConfirmed":true,"bindingSource":"Owned Phase 3 existing-account confirmation","reason":REASON})).await?
            }else{
                h.api(Method::DELETE,&format!("/api/memberships/{}",employee.id),json!({"expectedIdentityVersion":1,"reason":REASON})).await?
            };
            expect(&changed,StatusCode::OK,"permanent employee departure")?;
            ensure!(!has_role(&h.pool,employee.user,child_role).await?,"permanent departure retained unsupported direct employee role");
            ensure!(has_role(&h.pool,employee.user,h.ancestor_role).await?,"permanent departure removed ancestor role supported by sibling employee");
            ensure!(!active(&h.pool,employee_credentials.sid).await?&&active(&h.pool,customer_credentials.sid).await?&&active(&h.pool,other_credentials.sid).await?,"permanent departure crossed member session boundary");
            expect(&call(&h.app,&other_credentials.session,Method::GET,"/api/auth/permissions",Value::Null).await?,StatusCode::OK,"sibling employee still uses ancestor role")?;
            expect(&h.api(Method::DELETE,&format!("/api/memberships/{}",other.id),json!({"expectedIdentityVersion":1,"reason":REASON})).await?,StatusCode::OK,"remove final supporting employee")?;
            ensure!(!has_role(&h.pool,employee.user,h.ancestor_role).await?,"final departure retained unsupported ancestor role");
            ensure!(active(&h.pool,customer_credentials.sid).await?,"final employee departure revoked remaining customer");
        }
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops identity_phase03_* databases using explicit IDENTITY_PHASE03_TEST_DATABASE_URL"]
async fn employee_isolation_phase03_fresh_identity_precedes_cached_administrator_flag() -> Result<()>
{
    let db = Database::create().await?;
    let result = async {
        let h = Harness::create(&db.pool).await?;
        let employee = h
            .person(
                "phase03-stale-super-flag",
                h.child,
                "EMPLOYEE",
                "ENABLED",
                false,
            )
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
        let employee_credentials = credentials(&h.pool, &employee, h.application).await?;
        let customer_credentials = credentials(&h.pool, &customer, h.application).await?;
        let config = Config::from_values(&HashMap::from([
            ("APP_ENV".into(), "test".into()),
            (
                "DATABASE_URL".into(),
                "postgres://unused/owned-phase03-flag".into(),
            ),
            ("JWT_SECRET".into(), SECRET.into()),
        ]))?;
        let state = AppState::new(h.pool.clone(), Arc::new(config));
        let mut actor = crate::extractors::CurrentUser {
            user_id: employee.user,
            username: "phase03-stale-super-flag".into(),
            real_name: String::new(),
            is_super_admin: true,
            tenant_id: h.tenant,
            org_id: h.child,
            session_id: employee_credentials.sid,
        };
        ensure!(
            matches!(
                state.require(&actor, "system:user").await,
                Err(crate::error::AppError::Forbidden(_))
            ),
            "cached true administrator flag bypassed current database state"
        );
        ensure!(
            state.context_permissions(&actor).await?.is_empty(),
            "unprivileged employee inherited stale administrator permissions"
        );
        sqlx::query("UPDATE users SET is_super_admin=true WHERE id=$1")
            .bind(employee.user)
            .execute(&h.pool)
            .await?;
        actor.is_super_admin = false;
        state.require(&actor, "system:user").await?;
        actor.session_id = customer_credentials.sid;
        actor.is_super_admin = true;
        ensure!(
            matches!(
                state.require(&actor, "system:user").await,
                Err(crate::error::AppError::Forbidden(_))
            ),
            "customer context inherited live administrator privileges from its account"
        );
        actor.session_id = employee_credentials.sid;
        sqlx::query("UPDATE users SET is_super_admin=false WHERE id=$1")
            .bind(employee.user)
            .execute(&h.pool)
            .await?;
        ensure!(
            matches!(
                state.require(&actor, "system:user").await,
                Err(crate::error::AppError::Forbidden(_))
            ),
            "removed administrator privileges survived in the actor projection"
        );
        Ok(())
    }
    .await;
    db.finish(result).await
}

async fn wait_for_scope_account_barrier(pool: &PgPool, blocker: i32) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity \
                 WHERE datname=current_database() AND wait_event_type='Lock' \
                 AND $1=ANY(pg_blocking_pids(pid)) AND query LIKE '%users%' \
                 AND query ILIKE '%FOR%UPDATE%')",
            )
            .bind(blocker)
            .fetch_one(pool)
            .await?;
            if waiting {
                return Ok::<(), anyhow::Error>(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(|_| anyhow!("owned Phase 3 request never reached the observed account-lock barrier"))?
}

async fn subject_snapshot(pool: &PgPool, subject: Uuid) -> Result<Value> {
    Ok(sqlx::query_scalar(
        "SELECT jsonb_build_object( \
         'members',(SELECT COALESCE(jsonb_agg(to_jsonb(m) ORDER BY m.id),'[]'::jsonb) FROM memberships m WHERE m.user_id=$1), \
         'roles',(SELECT COALESCE(jsonb_agg(to_jsonb(r) ORDER BY r.role_id,r.organization_id),'[]'::jsonb) FROM user_roles r WHERE r.user_id=$1), \
         'sessions',(SELECT COALESCE(jsonb_agg(jsonb_build_object('id',s.id,'member',s.membership_id,'identityVersion',s.identity_version,'revoked',s.revoked_at) ORDER BY s.id),'[]'::jsonb) FROM platform_sessions s WHERE s.user_id=$1), \
         'audits',(SELECT count(*) FROM audit_logs))")
        .bind(subject).fetch_one(pool).await?)
}

#[tokio::test]
#[ignore = "creates and drops identity_phase03_* databases using explicit IDENTITY_PHASE03_TEST_DATABASE_URL"]
async fn employee_isolation_phase03_scope_rechecked_after_real_account_lock_wait() -> Result<()> {
    let db = Database::create().await?;
    let result=async {
        let h=Harness::create(&db.pool).await?;
        for scenario in ["bulk","user_roles","role_users"] {
            let target_org=organization(&h.pool,h.tenant,Some(h.child),&format!("phase03-{scenario}-descendant")).await?;
            let actor=h.person(&format!("phase03-{scenario}-scoped-actor"),h.child,"EMPLOYEE","ENABLED",true).await?;
            let actor_credentials=credentials(&h.pool,&actor,h.application).await?;
            let target=h.person(&format!("phase03-{scenario}-scoped-target"),target_org,"EMPLOYEE","ENABLED",false).await?;
            let customer=member(&h.pool,target.user,h.tenant,target_org,"CUSTOMER","ENABLED").await?;
            let target_credentials=credentials(&h.pool,&target,h.application).await?;
            let customer_credentials=credentials(&h.pool,&customer,h.application).await?;
            let old_role=role(&h.pool,h.tenant,target_org,&format!("phase03-{scenario}-old-role"),&h.menus).await?;
            let selected_role=role(&h.pool,h.tenant,target_org,&format!("phase03-{scenario}-new-role"),&h.menus).await?;
            attach_role(&h.pool,target.user,old_role,target_org).await?;
            let before=subject_snapshot(&h.pool,target.user).await?;
            let (path,body)=match scenario {
                "bulk"=>(format!("/api/users/{}/memberships",target.user),json!({"items":[{"organizationId":target_org,"position":"must not be saved outside actor subtree"}]})),
                "user_roles"=>(format!("/api/users/{}/roles",target.user),json!({"items":[{"roleId":selected_role,"organizationId":target_org}]})),
                "role_users"=>(format!("/api/roles/{selected_role}/users"),json!({"userIds":[target.user]})),
                _=>unreachable!(),
            };
            let mut blocker=h.pool.begin().await?;
            let blocker_pid:i32=sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *blocker).await?;
            sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE").bind(target.user).execute(&mut *blocker).await?;
            let request_app=h.app.clone();
            let request_token=actor_credentials.session.clone();
            let request=tokio::spawn(async move {call(&request_app,&request_token,Method::PUT,&path,body).await});
            if let Err(error)=wait_for_scope_account_barrier(&h.pool,blocker_pid).await {
                blocker.rollback().await?;
                request.abort();let _=request.await;
                return Err(error);
            }
            // The request has already captured its original child subtree and
            // is demonstrably waiting. This independent committed hierarchy
            // change preserves both target identities but removes management.
            sqlx::query("UPDATE organizations SET parent_id=$2,updated_at=clock_timestamp() WHERE id=$1")
                .bind(target_org).bind(h.sibling).execute(&h.pool).await?;
            blocker.rollback().await?;
            let reply=tokio::time::timeout(Duration::from_secs(20),request).await
                .map_err(|_|anyhow!("scope-invalidated owned request did not finish"))???;
            ensure!(matches!(reply.status,StatusCode::FORBIDDEN|StatusCode::NOT_FOUND|StatusCode::CONFLICT),
                "{scenario} reused cached scope after target left the subtree: {}",reply.status);
            ensure!(subject_snapshot(&h.pool,target.user).await?==before,"{scenario} committed member, role, session or audit changes after scope was lost");
            ensure!(active(&h.pool,target_credentials.sid).await?&&active(&h.pool,customer_credentials.sid).await?,"scope rejection revoked valid target employee/customer contexts");
            ensure!(has_role(&h.pool,target.user,old_role).await?&&!has_role(&h.pool,target.user,selected_role).await?,"scope rejection changed role bindings");
        }
        Ok(())
    }.await;
    db.finish(result).await
}

async fn wait_for_statement_barrier(pool: &PgPool, blocker: i32, pattern: &str) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() \
                 AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)) AND query ILIKE $2)",
            )
            .bind(blocker)
            .bind(pattern)
            .fetch_one(pool)
            .await?;
            if waiting {
                return Ok::<(), anyhow::Error>(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(|_| {
        anyhow!("owned Phase 3 request did not reach its observed statement lock barrier")
    })?
}

async fn publication_counts(pool: &PgPool) -> Result<(i64, i64, i64)> {
    Ok(sqlx::query_as(
        "SELECT (SELECT count(*) FROM notification_publications), \
        (SELECT count(*) FROM notifications), \
        (SELECT count(*) FROM audit_logs WHERE action='PUBLISH_NOTIFICATION')",
    )
    .fetch_one(pool)
    .await?)
}

#[tokio::test]
#[ignore = "creates and drops identity_phase03_* databases using explicit IDENTITY_PHASE03_TEST_DATABASE_URL"]
async fn employee_isolation_phase03_publisher_rechecked_after_late_foreign_key_wait() -> Result<()>
{
    let db = Database::create().await?;
    let result=async {
        let h=Harness::create(&db.pool).await?;
        let publish_menu=new_uuid();
        sqlx::query("INSERT INTO menus(id,application_id,type,title,permission,visible) VALUES($1,$2,'BUTTON','Owned publisher','notification:publish',false)")
            .bind(publish_menu).bind(h.application).execute(&h.pool).await?;
        sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)").bind(h.tenant).bind(publish_menu).execute(&h.pool).await?;
        sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)").bind(h.child).bind(publish_menu).execute(&h.pool).await?;
        for scenario in ["membership","account_version"] {
            let actor=h.person(&format!("phase03-publisher-{scenario}"),h.child,"EMPLOYEE","ENABLED",true).await?;
            let customer=member(&h.pool,actor.user,h.tenant,h.child,"CUSTOMER","ENABLED").await?;
            let actor_credentials=credentials(&h.pool,&actor,h.application).await?;
            let customer_credentials=credentials(&h.pool,&customer,h.application).await?;
            let recipient=h.person(&format!("phase03-publisher-recipient-{scenario}"),h.child,"EMPLOYEE","ENABLED",false).await?;
            sqlx::query("INSERT INTO membership_data_grants(id,membership_id,application_id,resource_code,action_code,scope_type) VALUES($1,$2,$3,'notification','publish','ORG')")
                .bind(new_uuid()).bind(actor.id).bind(h.application).execute(&h.pool).await?;
            let before=publication_counts(&h.pool).await?;
            let mut blocker=h.pool.begin().await?;
            let blocker_pid:i32=sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *blocker).await?;
            sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE").bind(recipient.user).execute(&mut *blocker).await?;
            let request_app=h.app.clone();
            let request_token=actor_credentials.app.clone();
            let body=json!({"idempotencyKey":format!("phase03-late-fk-{scenario}"),
                "target":{"kind":"USER","userId":recipient.user,"organizationId":h.child},
                "type":"INFO","title":"Owned Phase 3 publisher revocation fixture","content":"Owned fixture",
                "link":format!("/micro/{CODE}/owned")});
            let request=tokio::spawn(async move {call(&request_app,&request_token,Method::POST,&format!("/api/apps/{CODE}/notifications"),body).await});
            if let Err(error)=wait_for_statement_barrier(&h.pool,blocker_pid,"%INSERT INTO notifications(%").await {
                blocker.rollback().await?;request.abort();let _=request.await;return Err(error);
            }
            // Plain publisher authority reads and publication insertion have
            // passed. The delivery recipient FK now waits on the distinct
            // recipient's user lock; publication has no publisher-user FK.
            // Revoke within the holder transaction without touching B's rows.
            if scenario=="membership" {
                sqlx::query("UPDATE memberships SET status='DISABLED',identity_version=identity_version+1 WHERE id=$1")
                    .bind(actor.id).execute(&mut *blocker).await?;
            }else{
                sqlx::query("UPDATE users SET session_version=session_version+1 WHERE id=$1")
                    .bind(actor.user).execute(&mut *blocker).await?;
            }
            blocker.commit().await?;
            let reply=tokio::time::timeout(Duration::from_secs(20),request).await
                .map_err(|_|anyhow!("revoked publisher did not finish after its foreign-key wait"))???;
            ensure!(matches!(reply.status,StatusCode::UNAUTHORIZED|StatusCode::FORBIDDEN),
                "publisher committed after {scenario} revocation during the late FK wait: {}",reply.status);
            ensure!(publication_counts(&h.pool).await?==before,"publisher revocation left publication, delivery or success audit rows");
            ensure!(!active(&h.pool,actor_credentials.sid).await?,"revoked publisher employee session remained active");
            ensure!(active(&h.pool,customer_credentials.sid).await?==(scenario=="membership"),"publisher revocation affected an incorrect identity scope");
        }
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops identity_phase03_* databases using explicit IDENTITY_PHASE03_TEST_DATABASE_URL"]
async fn employee_isolation_phase03_role_parent_locked_before_replacing_children() -> Result<()> {
    let db = Database::create().await?;
    let result = async {
        let h = Harness::create(&db.pool).await?;
        let target = h
            .person(
                "phase03-role-fk-target",
                h.child,
                "EMPLOYEE",
                "ENABLED",
                false,
            )
            .await?;
        let retained = role(&h.pool, h.tenant, h.root, "phase03-fk-retained", &h.menus).await?;
        let other = role(&h.pool, h.tenant, h.root, "phase03-fk-other", &h.menus).await?;
        attach_role(&h.pool, target.user, retained, h.root).await?;
        attach_role(&h.pool, target.user, other, h.root).await?;
        let before_audits: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs")
            .fetch_one(&h.pool)
            .await?;
        let mut deletion = h.pool.begin().await?;
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *deletion)
            .await?;
        sqlx::query("SELECT id FROM roles WHERE id=$1 FOR UPDATE")
            .bind(retained)
            .execute(&mut *deletion)
            .await?;
        let request_app = h.app.clone();
        let request_token = h.admin.session.clone();
        let target_user = target.user;
        let root = h.root;
        // {R,S}->{R} cannot short-circuit as an unchanged role assignment.
        let request = tokio::spawn(async move {
            call(
                &request_app,
                &request_token,
                Method::PUT,
                &format!("/api/users/{target_user}/roles"),
                json!({"items":[{"roleId":retained,"organizationId":root}]}),
            )
            .await
        });
        if let Err(error) =
            wait_for_statement_barrier(&h.pool, blocker_pid, "%FROM roles%FOR%KEY%SHARE%").await
        {
            deletion.rollback().await?;
            request.abort();
            let _ = request.await;
            return Err(error);
        }
        // B must be waiting for its requested parent before deleting any child.
        // Otherwise this cascade would wait for B while B waits for this parent.
        let removed = tokio::time::timeout(
            Duration::from_secs(10),
            sqlx::query("DELETE FROM roles WHERE id=$1")
                .bind(retained)
                .execute(&mut *deletion),
        )
        .await;
        match removed {
            Ok(result) => {
                result?;
            }
            Err(_) => {
                request.abort();
                let _ = request.await;
                deletion.rollback().await?;
                return Err(anyhow!(
                    "role parent/child lock order blocked the deleting transaction"
                ));
            }
        }
        deletion.commit().await?;
        let reply = tokio::time::timeout(Duration::from_secs(20), request)
            .await
            .map_err(|_| anyhow!("role replacement did not finish after parent deletion"))???;
        ensure!(
            matches!(
                reply.status,
                StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND | StatusCode::CONFLICT
            ),
            "role replacement did not safely reject deleted parent: {}",
            reply.status
        );
        ensure!(
            !has_role(&h.pool, target.user, retained).await?
                && has_role(&h.pool, target.user, other).await?,
            "failed replacement removed an unrelated retained child binding"
        );
        let after_audits: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs")
            .fetch_one(&h.pool)
            .await?;
        ensure!(
            after_audits == before_audits,
            "rejected role replacement committed success audit rows"
        );
        Ok(())
    }
    .await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "creates and drops identity_phase03_* databases using explicit IDENTITY_PHASE03_TEST_DATABASE_URL"]
async fn employee_isolation_phase03_bulk_self_departure_preserves_customer_and_rechecks_after_credential_wait(
) -> Result<()> {
    let db = Database::create().await?;
    let result = async {
        let h = Harness::create(&db.pool).await?;
        let actor = h
            .person(
                "phase03-self-departure",
                h.root,
                "EMPLOYEE",
                "ENABLED",
                false,
            )
            .await?;
        let customer = member(&h.pool, actor.user, h.tenant, h.root, "CUSTOMER", "ENABLED").await?;
        let actor_credentials = credentials(&h.pool, &actor, h.application).await?;
        let customer_credentials = credentials(&h.pool, &customer, h.application).await?;
        let actor_refresh = platform_refresh(&h.pool, actor_credentials.sid).await?;
        let customer_refresh = platform_refresh(&h.pool, customer_credentials.sid).await?;
        let actor_role = role(
            &h.pool,
            h.tenant,
            h.root,
            "phase03-self-departure-role",
            &h.menus,
        )
        .await?;
        attach_role(&h.pool, actor.user, actor_role, h.root).await?;
        ensure!(
            active(&h.pool, actor_credentials.sid).await?
                && active(&h.pool, customer_credentials.sid).await?,
            "self-departure requires independently active employee and customer fixtures"
        );
        expect(
            &call(
                &h.app,
                &actor_credentials.session,
                Method::PUT,
                &format!("/api/users/{}/memberships", actor.user),
                json!({"items":[]}),
            )
            .await?,
            StatusCode::OK,
            "legacy bulk permits authorized root employee self-departure",
        )?;
        let remaining: Vec<(Uuid, String)> =
            sqlx::query_as("SELECT id,member_type FROM memberships WHERE user_id=$1 ORDER BY id")
                .bind(actor.user)
                .fetch_all(&h.pool)
                .await?;
        ensure!(
            remaining == vec![(customer.id, "CUSTOMER".to_owned())],
            "self-departure did not remove only the actor's employee membership"
        );
        ensure!(
            !has_role(&h.pool, actor.user, actor_role).await?,
            "successful self-departure retained unsupported employee role configuration"
        );
        expect(
            &call(
                &h.app,
                &actor_credentials.session,
                Method::GET,
                "/api/auth/permissions",
                Value::Null,
            )
            .await?,
            StatusCode::UNAUTHORIZED,
            "self-departed employee session is invalid",
        )?;
        expect(
            &refresh_session(&h.app, &actor_refresh).await?,
            StatusCode::BAD_REQUEST,
            "self-departed employee platform refresh is invalid",
        )?;
        ensure!(
            !active(&h.pool, actor_credentials.sid).await?
                && active(&h.pool, customer_credentials.sid).await?,
            "self-departure revoked the wrong concrete identity context"
        );
        expect(
            &refresh_session(&h.app, &customer_refresh).await?,
            StatusCode::OK,
            "same-organization customer platform refresh survives employee self-departure",
        )?;

        // A successful self-departure must validate while its source membership
        // still exists. It must still close any real wait before that final
        // proof, including waits in the membership's credential descendants.
        for scenario in ["permission", "scope"] {
            let actor = h
                .person(
                    &format!("phase03-self-wait-{scenario}"),
                    h.root,
                    "EMPLOYEE",
                    "ENABLED",
                    false,
                )
                .await?;
            let customer =
                member(&h.pool, actor.user, h.tenant, h.root, "CUSTOMER", "ENABLED").await?;
            let actor_credentials = credentials(&h.pool, &actor, h.application).await?;
            let customer_credentials = credentials(&h.pool, &customer, h.application).await?;
            let actor_refresh = platform_refresh(&h.pool, actor_credentials.sid).await?;
            let customer_refresh = platform_refresh(&h.pool, customer_credentials.sid).await?;
            let actor_role = role(
                &h.pool,
                h.tenant,
                h.root,
                &format!("phase03-self-wait-role-{scenario}"),
                &h.menus,
            )
            .await?;
            attach_role(&h.pool, actor.user, actor_role, h.root).await?;
            let before = subject_snapshot(&h.pool, actor.user).await?;
            let mut blocker = h.pool.begin().await?;
            let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *blocker)
                .await?;
            sqlx::query(
                "SELECT id FROM platform_session_refresh_tokens WHERE token_hash=$1 FOR UPDATE",
            )
            .bind(hash::sha256_hex(&actor_refresh))
            .execute(&mut *blocker)
            .await?;
            let request_app = h.app.clone();
            let request_token = actor_credentials.session.clone();
            let actor_user = actor.user;
            let request = tokio::spawn(async move {
                call(
                    &request_app,
                    &request_token,
                    Method::PUT,
                    &format!("/api/users/{actor_user}/memberships"),
                    json!({"items":[]}),
                )
                .await
            });
            if let Err(error) = wait_for_statement_barrier(
                &h.pool,
                blocker_pid,
                "%FROM platform_session_refresh_tokens%",
            )
            .await
            {
                blocker.rollback().await?;
                request.abort();
                let _ = request.await;
                return Err(error);
            }
            if scenario == "permission" {
                sqlx::query("DELETE FROM role_menus WHERE role_id=$1 AND menu_id=$2")
                    .bind(actor_role)
                    .bind(h.menus[0])
                    .execute(&h.pool)
                    .await?;
            } else {
                sqlx::query(
                    "UPDATE tenants SET is_platform=false,updated_at=clock_timestamp() WHERE id=$1",
                )
                .bind(h.tenant)
                .execute(&h.pool)
                .await?;
            }
            blocker.rollback().await?;
            let reply = tokio::time::timeout(Duration::from_secs(20), request)
                .await
                .map_err(|_| anyhow!("self-departure did not finish after credential wait"))???;
            ensure!(
                matches!(
                    reply.status,
                    StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::CONFLICT
                ),
                "self-departure committed after {scenario} revocation during credential wait: {}",
                reply.status
            );
            ensure!(
                subject_snapshot(&h.pool, actor.user).await? == before,
                "rejected self-departure changed membership, roles, sessions or success audits"
            );
            ensure!(
                active(&h.pool, actor_credentials.sid).await?
                    && active(&h.pool, customer_credentials.sid).await?
                    && has_role(&h.pool, actor.user, actor_role).await?,
                "rejected self-departure removed employee/customer context or retained role"
            );
            expect(
                &refresh_session(&h.app, &actor_refresh).await?,
                StatusCode::OK,
                "rejected self-departure preserves employee platform refresh",
            )?;
            expect(
                &refresh_session(&h.app, &customer_refresh).await?,
                StatusCode::OK,
                "rejected self-departure preserves customer platform refresh",
            )?;
        }
        Ok(())
    }
    .await;
    db.finish(result).await
}
