//! Phase 1 membership contracts on disposable, explicitly authorized PostgreSQL databases.
//! Only IDENTITY_PHASE01_TEST_DATABASE_URL is read; preview settings and .env are never used.
use std::collections::{BTreeMap, HashMap};
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
use crate::util::{jwt, uuid7::new_uuid};

const SECRET: &str = "identity-phase01-isolated-fixture-only";
const MEMBERSHIP_REASON: &str = "Owned Phase 1 membership contract mutation";

struct Database {
    pool: PgPool,
    admin: PgPool,
    name: String,
}

impl Database {
    async fn create(version: i64) -> Result<Self> {
        let url = std::env::var("IDENTITY_PHASE01_TEST_DATABASE_URL")
            .map_err(|_| anyhow!("explicit IDENTITY_PHASE01_TEST_DATABASE_URL is required"))?;
        let options = PgConnectOptions::from_str(&url)
            .map_err(|_| anyhow!("invalid isolated identity database configuration"))?;
        ensure!(
            options.get_database() == Some("postgres"),
            "identity fixture administration must explicitly select postgres"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("isolated identity administration connection failed"))?;
        let name = format!("identity_phase01_contract_{}", new_uuid().simple());
        ensure!(
            name.len() <= 63,
            "owned database name exceeds PostgreSQL limit"
        );
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await?;
        let pool = match PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.database(&name))
            .await
        {
            Ok(pool) => pool,
            Err(_) => {
                let _ = sqlx::query(&format!("DROP DATABASE \"{name}\" WITH (FORCE)"))
                    .execute(&admin)
                    .await;
                return Err(anyhow!("owned identity fixture connection failed"));
            }
        };
        let database = Self { pool, admin, name };
        let migrator = sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(
                sqlx::migrate!("./migrations")
                    .iter()
                    .filter(|migration| migration.version <= version)
                    .cloned()
                    .collect(),
            ),
            ..sqlx::migrate::Migrator::DEFAULT
        };
        if let Err(error) = migrator.run(&database.pool).await {
            let _ = database.finish(Ok(())).await;
            return Err(error.into());
        }
        Ok(database)
    }

    async fn finish(self, result: Result<()>) -> Result<()> {
        ensure!(
            self.name.starts_with("identity_phase01_contract_")
                && self.name.len() <= 63
                && self
                    .name
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_'),
            "refusing cleanup outside the owned identity fixture"
        );
        let closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let cleanup = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await;
        let closed = closed
            || tokio::time::timeout(Duration::from_secs(5), self.pool.close())
                .await
                .is_ok();
        self.admin.close().await;
        eprintln!(
            "identity_phase01_cleanup database={} assertions_ok={} database_removed={} pool_closed={closed}",
            self.name,
            result.is_ok(),
            cleanup.is_ok()
        );
        result?;
        cleanup?;
        ensure!(closed, "owned identity fixture pool did not close");
        Ok(())
    }
}

fn settings(lifecycle: bool, customer: bool, dual: bool) -> Result<Config> {
    Config::from_values(&HashMap::from([
        ("APP_ENV".into(), "test".into()),
        (
            "DATABASE_URL".into(),
            "postgres://unused/identity-contract".into(),
        ),
        ("JWT_SECRET".into(), SECRET.into()),
        ("MEMBERSHIP_LIFECYCLE_ENABLED".into(), lifecycle.to_string()),
        ("CUSTOMER_MEMBERSHIPS_ENABLED".into(), customer.to_string()),
        ("DUAL_MEMBERSHIPS_ENABLED".into(), dual.to_string()),
    ]))
}

fn app(pool: &PgPool, lifecycle: bool, customer: bool, dual: bool) -> Result<Router> {
    let state = AppState::new(pool.clone(), Arc::new(settings(lifecycle, customer, dual)?));
    Ok(crate::routes::api_router()
        .nest("/oauth", crate::routes::oauth2::oauth_router())
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

struct Reply {
    status: StatusCode,
    body: Value,
    request_id: Uuid,
}

async fn call(app: &Router, token: &str, method: Method, path: &str, body: Value) -> Result<Reply> {
    let mut body = body;
    // Existing contract cases exercise their specific business preconditions.
    // Supply an explicit fixture reason there; reason validation itself uses
    // call_raw so missing and malformed values reach HTTP unchanged.
    if membership_write(&method, path) {
        if let Some(fields) = body.as_object_mut() {
            fields
                .entry("reason")
                .or_insert_with(|| json!(MEMBERSHIP_REASON));
        }
    }
    call_raw(app, token, method, path, body).await
}

fn membership_write(method: &Method, path: &str) -> bool {
    matches!(*method, Method::POST | Method::PUT | Method::DELETE)
        && (path == "/api/memberships" || path.starts_with("/api/memberships/"))
}

async fn call_raw(
    app: &Router,
    token: &str,
    method: Method,
    path: &str,
    body: Value,
) -> Result<Reply> {
    let request_id = new_uuid();
    let request = Request::builder()
        .method(method.clone())
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-request-id", request_id.to_string())
        .extension(ConnectInfo("192.0.2.31:30031".parse::<SocketAddr>()?))
        .body(Body::from(serde_json::to_vec(&body)?))?;
    let response = tokio::time::timeout(Duration::from_secs(25), app.clone().oneshot(request))
        .await
        .map_err(|_| anyhow!("identity API timeout: {method} {path}"))??;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    let body =
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes)));
    Ok(Reply {
        status,
        body,
        request_id,
    })
}

fn successful(reply: &Reply) -> Result<Uuid> {
    ensure!(
        reply.status.is_success(),
        "identity API returned {}: {}",
        reply.status,
        reply.body
    );
    let id: Uuid = reply.body["id"]
        .as_str()
        .ok_or_else(|| anyhow!("member DTO lacks id"))?
        .parse()?;
    ensure!(
        crate::util::uuid7::is_uuid_v7(id),
        "member DTO id is not v7"
    );
    for key in [
        "userId",
        "organizationId",
        "memberType",
        "status",
        "identityVersion",
        "updatedAt",
    ] {
        ensure!(!reply.body[key].is_null(), "member DTO lacks {key}");
    }
    Ok(id)
}

async fn user(pool: &PgPool, label: &str, admin: bool) -> Result<Uuid> {
    let id = new_uuid();
    sqlx::query("INSERT INTO users(id,username,password_hash,is_super_admin) VALUES($1,$2,'owned-fixture-not-a-real-password',$3)")
        .bind(id).bind(label).bind(admin).execute(pool).await?;
    Ok(id)
}

async fn member(pool: &PgPool, user: Uuid, org: Uuid, kind: &str) -> Result<Uuid> {
    let id = new_uuid();
    sqlx::query(
        "INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES($1,$2,$3,$4)",
    )
    .bind(id)
    .bind(user)
    .bind(org)
    .bind(kind)
    .execute(pool)
    .await?;
    Ok(id)
}

async fn tenant(pool: &PgPool, label: &str) -> Result<(Uuid, Uuid)> {
    let id = new_uuid();
    sqlx::query("INSERT INTO tenants(id,name,code) VALUES($1,$2,$2)")
        .bind(id)
        .bind(label)
        .execute(pool)
        .await?;
    let org = organization(pool, id, None, label).await?;
    Ok((id, org))
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

async fn session(pool: &PgPool, user: Uuid, tenant: Uuid, org: Uuid) -> Result<(String, Uuid)> {
    let version: i64 = sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
        .bind(user)
        .fetch_one(pool)
        .await?;
    let id = new_uuid();
    let membership: Option<(Uuid,String,i64)> = sqlx::query_as(
        "SELECT id,member_type,identity_version FROM memberships WHERE user_id=$1 AND organization_id=$2 AND member_type='EMPLOYEE'"
    ).bind(user).bind(org).fetch_optional(pool).await?;
    if let Some((membership_id, member_type, identity_version)) = membership {
        sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version) SELECT $1,$2,$3,$4,now()+interval '1 hour',$5,$6,$7,$8,'urn:saas:legacy-platform',client_id,access_policy_version FROM oauth_clients WHERE client_id='platform-console-legacy'")
            .bind(id).bind(user).bind(tenant).bind(org).bind(version).bind(membership_id).bind(member_type).bind(identity_version).execute(pool).await?;
    } else {
        // Intentional old unbound fixture for the customer legacy rejection case.
        sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at) VALUES($1,$2,$3,$4,now()+interval '1 hour')")
            .bind(id).bind(user).bind(tenant).bind(org).execute(pool).await?;
    }
    let now = chrono::Utc::now().timestamp();
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
            exp: now + 3600,
        },
        SECRET,
    )?;
    Ok((token, id))
}

struct Harness {
    pool: PgPool,
    app: Router,
    tenant: Uuid,
    root: Uuid,
    child: Uuid,
    sibling: Uuid,
    foreign: Uuid,
    actor: Uuid,
    admin: String,
    child_admin: String,
    unprivileged: String,
}

impl Harness {
    async fn create(pool: &PgPool) -> Result<Self> {
        let (tenant, root) = tenant(pool, "identity-scope-a").await?;
        let child = organization(pool, tenant, Some(root), "Identity child").await?;
        let sibling = organization(pool, tenant, Some(root), "Identity sibling").await?;
        let (_, foreign) = self::tenant(pool, "identity-scope-b").await?;
        let actor = user(pool, "identity-manager", true).await?;
        member(pool, actor, root, "EMPLOYEE").await?;
        member(pool, actor, child, "EMPLOYEE").await?;
        let plain = user(pool, "identity-unprivileged", false).await?;
        member(pool, plain, root, "EMPLOYEE").await?;
        let application = new_uuid();
        sqlx::query("INSERT INTO applications(id,name,code,base_path) VALUES($1,'Identity management','system','/micro/system')")
            .bind(application).execute(pool).await?;
        let menu = new_uuid();
        sqlx::query("INSERT INTO menus(id,application_id,title,permission,path) VALUES($1,$2,'Identity members','system:user','/system/users')")
            .bind(menu).bind(application).execute(pool).await?;
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
            .bind(tenant)
            .bind(application)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
            .bind(tenant)
            .bind(menu)
            .execute(pool)
            .await?;
        for org in [root, child] {
            sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)")
                .bind(org).bind(application).execute(pool).await?;
            sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
                .bind(org)
                .bind(menu)
                .execute(pool)
                .await?;
        }
        sqlx::raw_sql(
            "CREATE TABLE identity_phase01_audit_fault(enabled boolean NOT NULL); \
            INSERT INTO identity_phase01_audit_fault VALUES(false); \
            CREATE FUNCTION identity_phase01_reject_audit() RETURNS trigger LANGUAGE plpgsql AS $$ \
            BEGIN IF EXISTS(SELECT 1 FROM identity_phase01_audit_fault WHERE enabled) THEN \
              RAISE EXCEPTION 'owned identity audit fault' USING ERRCODE='P0001'; \
            END IF; RETURN NEW; END $$; \
            CREATE TRIGGER identity_phase01_audit_fault BEFORE INSERT ON audit_logs \
            FOR EACH ROW EXECUTE FUNCTION identity_phase01_reject_audit();",
        )
        .execute(pool)
        .await?;
        Ok(Self {
            pool: pool.clone(),
            app: app(pool, true, true, true)?,
            tenant,
            root,
            child,
            sibling,
            foreign,
            actor,
            admin: session(pool, actor, tenant, root).await?.0,
            child_admin: session(pool, actor, tenant, child).await?.0,
            unprivileged: session(pool, plain, tenant, root).await?.0,
        })
    }

    async fn call(&self, method: Method, path: &str, body: Value) -> Result<Reply> {
        let audit_reason = membership_write(&method, path).then(|| {
            body["reason"]
                .as_str()
                .unwrap_or(MEMBERSHIP_REASON)
                .trim()
                .to_string()
        });
        let reply = call(&self.app, &self.admin, method, path, body).await?;
        if reply.status.is_success() {
            if let Some(expected) = audit_reason {
                let reasons: Vec<Option<String>> = sqlx::query_scalar(
                    "SELECT detail::jsonb->>'reason' FROM audit_logs WHERE request_id=$1",
                )
                .bind(reply.request_id)
                .fetch_all(&self.pool)
                .await?;
                ensure!(
                    reasons == vec![Some(expected)],
                    "successful membership mutation lost its real trimmed audit reason"
                );
            }
        }
        Ok(reply)
    }

    async fn create_member(&self, user: Uuid, org: Uuid, kind: &str) -> Result<Reply> {
        self.call(
            Method::POST,
            "/api/memberships",
            json!({
                "userId":user,"organizationId":org,"memberType":kind,
                "accountBindingConfirmed":true,"bindingSource":"owned invitation acceptance fixture"
            }),
        )
        .await
    }

    async fn snapshot(&self) -> Result<BTreeMap<String, String>> {
        let mut queries = Vec::new();
        for table in [
            "memberships",
            "platform_sessions",
            "platform_session_refresh_tokens",
            "oauth_tokens",
            "oauth_authorization_codes",
            "oauth_device_codes",
            "membership_data_grants",
            "user_roles",
            "audit_logs",
        ] {
            queries.push(format!("SELECT '{table}'::text,COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb)::text FROM {table} t"));
        }
        let rows: Vec<(String, String)> = sqlx::query_as(&queries.join(" UNION ALL "))
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().collect())
    }

    async fn audit_fault(&self, method: Method, path: &str, body: Value) -> Result<()> {
        let before = self.snapshot().await?;
        sqlx::query("UPDATE identity_phase01_audit_fault SET enabled=true")
            .execute(&self.pool)
            .await?;
        let reply = self.call(method, path, body).await;
        sqlx::query("UPDATE identity_phase01_audit_fault SET enabled=false")
            .execute(&self.pool)
            .await?;
        let reply = reply?;
        ensure!(
            reply.status == StatusCode::INTERNAL_SERVER_ERROR,
            "audit rejection returned {}: {}",
            reply.status,
            reply.body
        );
        ensure!(
            self.snapshot().await? == before,
            "audit fault committed a membership or credential change"
        );
        Ok(())
    }
}

async fn db_reject(pool: &PgPool, sql: &str, state: &str) -> Result<()> {
    let mut tx = pool.begin().await?;
    let result = sqlx::query(sql).execute(&mut *tx).await;
    tx.rollback().await?;
    let error = result
        .err()
        .ok_or_else(|| anyhow!("invalid identity DB fixture unexpectedly succeeded"))?;
    ensure!(
        error.as_database_error().and_then(|e| e.code()).as_deref() == Some(state),
        "invalid identity DB fixture returned unexpected SQLSTATE"
    );
    Ok(())
}

async fn uuid_baseline(pool: &PgPool) -> Result<(usize, i64, i64)> {
    let columns: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT table_name,column_name,column_default FROM information_schema.columns \
         WHERE table_schema='public' AND udt_name='uuid' ORDER BY table_name,column_name",
    )
    .fetch_all(pool)
    .await?;
    ensure!(
        columns.len() >= 107,
        "the expected platform UUID schema was not fully examined"
    );
    let mut values = 0;
    for (table, column, default) in &columns {
        ensure!(
            default.is_none(),
            "UUID column {table}.{column} still depends on a generator default"
        );
        ensure!(
            [table, column].iter().all(|name| name
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_')),
            "unexpected UUID schema identifier"
        );
        let (present,invalid): (i64,i64) = sqlx::query_as(&format!(
            "SELECT count(\"{column}\"),count(*) FILTER(WHERE \"{column}\" IS NOT NULL AND NOT public.is_uuid_v7(\"{column}\")) FROM \"{table}\""))
            .fetch_one(pool).await?;
        ensure!(invalid == 0, "invalid system UUID at {table}.{column}");
        values += present;
    }
    let (keys, unvalidated): (i64, i64) = sqlx::query_as(
        "SELECT count(*),count(*) FILTER(WHERE NOT convalidated) FROM pg_constraint c \
         JOIN pg_namespace n ON n.oid=c.connamespace WHERE n.nspname='public' AND contype='f'",
    )
    .fetch_one(pool)
    .await?;
    ensure!(
        keys > 0 && unvalidated == 0,
        "foreign keys were not fully validated"
    );
    let orphaned: i64 = sqlx::query_scalar("SELECT count(*) FROM memberships m \
        LEFT JOIN users u ON u.id=m.user_id LEFT JOIN organizations o ON o.id=m.organization_id \
        LEFT JOIN departments d ON d.id=m.department_id WHERE u.id IS NULL OR o.id IS NULL \
        OR (m.department_id IS NOT NULL AND (d.id IS NULL OR d.organization_id<>m.organization_id))")
        .fetch_one(pool).await?;
    ensure!(
        orphaned == 0,
        "member/organization/department foreign references are inconsistent"
    );
    db_reject(pool,"INSERT INTO users(id,username,password_hash) VALUES('019a1163-0000-4000-8000-000000000001','negative-v4-subject','fixture')","23514").await?;
    db_reject(pool,"INSERT INTO users(id,username,password_hash) VALUES('019a1163-0000-7000-c000-000000000001','negative-variant-subject','fixture')","23514").await?;
    db_reject(
        pool,
        "INSERT INTO users(username,password_hash) VALUES('negative-implicit-id','fixture')",
        "23502",
    )
    .await?;
    let absent = new_uuid();
    db_reject(
        pool,
        &format!(
            "INSERT INTO memberships(id,user_id,organization_id) VALUES('{}','{absent}','{}')",
            new_uuid(),
            new_uuid()
        ),
        "23503",
    )
    .await?;
    Ok((columns.len(), values, keys))
}

async fn seed_identity_snapshot(pool: &PgPool) -> Result<BTreeMap<String, String>> {
    let tables: Vec<String> = sqlx::query_scalar("SELECT tablename FROM pg_tables WHERE schemaname='public' AND tablename<>'_sqlx_migrations' ORDER BY tablename")
        .fetch_all(pool).await?;
    let mut statements = Vec::new();
    for table in tables {
        ensure!(
            table
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_'),
            "unexpected seed table identifier"
        );
        statements.push(format!("SELECT '{table}'::text,COALESCE(jsonb_agg(to_jsonb(r) ORDER BY to_jsonb(r)::text),'[]'::jsonb)::text FROM \"{table}\" r"));
    }
    let rows: Vec<(String, String)> = sqlx::query_as(&statements.join(" UNION ALL "))
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().collect())
}

async fn database_contract(h: &Harness) -> Result<()> {
    let constraints: Vec<String> = sqlx::query_scalar(
        "SELECT conname FROM pg_constraint WHERE conrelid='memberships'::regclass AND contype='u'",
    )
    .fetch_all(&h.pool)
    .await?;
    ensure!(
        constraints
            .iter()
            .any(|name| name == "memberships_user_id_organization_id_key")
            && constraints
                .iter()
                .any(|name| name == "memberships_user_organization_type_key"),
        "expansion did not retain both membership unique constraints"
    );
    let subject = user(&h.pool, "identity-db-contract", false).await?;
    let employee = member(&h.pool, subject, h.root, "EMPLOYEE").await?;
    for change in [
        "member_type='ALIEN'",
        "status='UNKNOWN'",
        "identity_version=0",
        "member_type='CUSTOMER'",
        "user_id=NULL",
        "organization_id=NULL",
    ] {
        db_reject(
            &h.pool,
            &format!("UPDATE memberships SET {change} WHERE id='{employee}'"),
            "23514",
        )
        .await?;
    }
    db_reject(
        &h.pool,
        &format!("UPDATE memberships SET status='DISABLED' WHERE id='{employee}'"),
        "23514",
    )
    .await?;
    db_reject(&h.pool, &format!("INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES('{}','{subject}','{}','CUSTOMER')", new_uuid(), h.root), "23505").await?;
    for (table, key, value) in [
        ("applications", "allowed_member_types", "ARRAY[]::text[]"),
        (
            "applications",
            "allowed_member_types",
            "ARRAY['ALIEN']::text[]",
        ),
        (
            "applications",
            "allowed_member_types",
            "ARRAY['EMPLOYEE','EMPLOYEE']::text[]",
        ),
        ("applications", "access_policy_version", "0"),
    ] {
        db_reject(
            &h.pool,
            &format!("UPDATE {table} SET {key}={value}"),
            "23514",
        )
        .await?;
    }
    let client = new_uuid();
    sqlx::query("INSERT INTO oauth_clients(id,client_id,client_name) VALUES($1,'identity-user','Identity user')")
        .bind(client).execute(&h.pool).await?;
    let defaults: (String,Vec<String>,i64) = sqlx::query_as("SELECT subject_mode,allowed_member_types,access_policy_version FROM oauth_clients WHERE id=$1")
        .bind(client).fetch_one(&h.pool).await?;
    ensure!(
        defaults == ("USER".into(), vec!["EMPLOYEE".into()], 1),
        "ordinary client defaults widened identity admission"
    );
    for change in [
        "subject_mode='UNKNOWN'",
        "allowed_member_types=ARRAY[]::text[]",
        "allowed_member_types=ARRAY['ALIEN']::text[]",
        "access_policy_version=0",
        "subject_mode='MACHINE'",
        "grant_types='client_credentials'",
        "grant_types=E'\\tclient_credentials\\n'",
        "subject_mode='LEGACY_MIXED',grant_types='client_credentials,authorization_code'",
    ] {
        db_reject(
            &h.pool,
            &format!("UPDATE oauth_clients SET {change} WHERE id='{client}'"),
            "23514",
        )
        .await?;
    }
    let mixed = new_uuid();
    db_reject(&h.pool, &format!("INSERT INTO oauth_clients(id,client_id,client_name,subject_mode,grant_types) VALUES('{mixed}','identity-mixed','Forbidden mixed','LEGACY_MIXED','client_credentials,authorization_code')"), "23514").await?;
    sqlx::query("INSERT INTO oauth_clients(id,client_id,client_name,subject_mode,grant_types,allowed_member_types) VALUES($1,'identity-machine','Machine fixture','MACHINE','client_credentials',ARRAY[]::text[])")
        .bind(new_uuid()).execute(&h.pool).await?;
    let normalized: Vec<String> = sqlx::query_scalar("SELECT oauth_client_grant_set(string_to_array(E'\\tclient_credentials\\n, ,client_credentials',','))")
        .fetch_one(&h.pool).await?;
    ensure!(
        normalized == vec!["client_credentials"],
        "SQL grant normalization differs from runtime CSV trimming"
    );
    eprintln!("identity_phase01 database_constraints=PASS expansion_dual_constraints=PASS");
    Ok(())
}

async fn membership_reason_contract(h: &Harness) -> Result<()> {
    let source_user = user(&h.pool, "identity-reason-source", false).await?;
    let source = member(&h.pool, source_user, h.root, "EMPLOYEE").await?;
    let new_user = user(&h.pool, "identity-reason-create", false).await?;
    let operations = [
        (
            Method::POST,
            "/api/memberships".to_string(),
            json!({"userId":new_user,"organizationId":h.root,"memberType":"EMPLOYEE"}),
        ),
        (
            Method::PUT,
            format!("/api/memberships/{source}/status"),
            json!({"status":"DISABLED","expectedIdentityVersion":1}),
        ),
        (
            Method::DELETE,
            format!("/api/memberships/{source}"),
            json!({"expectedIdentityVersion":1}),
        ),
        (
            Method::POST,
            format!("/api/memberships/{source}/second-identity"),
            json!({"memberType":"CUSTOMER","expectedIdentityVersion":1,
                "accountBindingConfirmed":true,"bindingSource":"owned reason fixture"}),
        ),
        (
            Method::POST,
            format!("/api/memberships/{source}/replace-identity"),
            json!({"memberType":"CUSTOMER","expectedIdentityVersion":1,
                "accountBindingConfirmed":true,"bindingSource":"owned reason fixture"}),
        ),
    ];
    let invalid_reasons = [
        ("missing", None),
        ("empty", Some(json!(""))),
        ("whitespace", Some(json!(" \t\u{3000} "))),
        ("over_500_characters", Some(json!("原".repeat(501)))),
        ("internal_control", Some(json!("owned\nreason"))),
        ("null", Some(Value::Null)),
    ];
    for (method, path, base) in operations {
        for (label, reason) in &invalid_reasons {
            let mut body = base.clone();
            if let Some(reason) = reason {
                body["reason"] = reason.clone();
            }
            let before = h.snapshot().await?;
            let reply = call_raw(&h.app, &h.admin, method.clone(), &path, body).await?;
            ensure!(
                reply.status == StatusCode::BAD_REQUEST,
                "{method} {path} accepted {label} membership reason"
            );
            ensure!(
                h.snapshot().await? == before,
                "{method} {path} changed memberships, credentials or audits for {label} reason"
            );
            let audits: i64 =
                sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE request_id=$1")
                    .bind(reply.request_id)
                    .fetch_one(&h.pool)
                    .await?;
            ensure!(
                audits == 0,
                "invalid reason emitted a successful membership audit"
            );
        }
    }
    let actual_reason = "Accepted invitation reference and verified organization binding";
    let created = h
        .call(
            Method::POST,
            "/api/memberships",
            json!({
                "userId":new_user,"organizationId":h.root,"memberType":"EMPLOYEE",
                "reason":format!("  {actual_reason}\u{3000} ")
            }),
        )
        .await?;
    successful(&created)?;
    let reason: String =
        sqlx::query_scalar("SELECT detail::jsonb->>'reason' FROM audit_logs WHERE request_id=$1")
            .bind(created.request_id)
            .fetch_one(&h.pool)
            .await?;
    ensure!(
        reason == actual_reason,
        "audit recorded an action placeholder or untrimmed reason"
    );
    let boundary_reason = "核".repeat(500);
    let boundary = h
        .call(
            Method::POST,
            "/api/memberships",
            json!({
                "userId":new_user,"organizationId":h.child,"memberType":"EMPLOYEE",
                "reason":boundary_reason
            }),
        )
        .await?;
    successful(&boundary)?;
    eprintln!("identity_phase01 reason_required_all_write_dtos=PASS invalid_reason_no_write_audit=PASS real_trimmed_reason_audit=PASS reason_500_unicode_character_boundary=PASS");
    Ok(())
}

async fn expansion_api_contract(h: &Harness) -> Result<()> {
    let target = user(&h.pool, "identity-api-target", false).await?;
    let disabled = app(&h.pool, false, false, false)?;
    let request = json!({"userId":target,"organizationId":h.root,"memberType":"EMPLOYEE"});
    let reply = call(
        &disabled,
        &h.admin,
        Method::POST,
        "/api/memberships",
        request.clone(),
    )
    .await?;
    ensure!(
        reply.status == StatusCode::FORBIDDEN,
        "lifecycle writes are enabled by default"
    );
    let denied = call(
        &h.app,
        &h.unprivileged,
        Method::POST,
        "/api/memberships",
        request.clone(),
    )
    .await?;
    ensure!(
        denied.status == StatusCode::FORBIDDEN,
        "membership write bypassed system:user"
    );
    for org in [h.sibling, h.foreign] {
        let denied = call(
            &h.app,
            &h.child_admin,
            Method::POST,
            "/api/memberships",
            json!({"userId":target,"organizationId":org,"memberType":"EMPLOYEE"}),
        )
        .await?;
        ensure!(
            denied.status == StatusCode::NOT_FOUND,
            "out-of-scope member creation returned {}",
            denied.status
        );
    }
    let customer_off = app(&h.pool, true, false, false)?;
    let blocked = call(
        &customer_off,
        &h.admin,
        Method::POST,
        "/api/memberships",
        json!({
        "userId":target,"organizationId":h.root,"memberType":"CUSTOMER",
        "accountBindingConfirmed":true,"bindingSource":"owned fixture"}),
    )
    .await?;
    ensure!(
        blocked.status == StatusCode::FORBIDDEN,
        "customer creation ignored default gate"
    );
    for body in [
        json!({"userId":target,"organizationId":h.root}),
        json!({"userId":target,"organizationId":h.root,"memberType":"ALIEN"}),
        json!({"userId":target,"organizationId":h.root,"memberType":"CUSTOMER"}),
        json!({"userId":target,"organizationId":h.root,"memberType":"CUSTOMER","accountBindingConfirmed":true,"bindingSource":""}),
        json!({"userId":target,"organizationId":h.root,"memberType":"CUSTOMER","accountBindingConfirmed":true,"bindingSource":"owned","position":"employee-only"}),
    ] {
        let invalid = h.call(Method::POST, "/api/memberships", body).await?;
        ensure!(
            invalid.status == StatusCode::BAD_REQUEST,
            "invalid explicit membership input returned {}: {}",
            invalid.status,
            invalid.body
        );
    }
    h.audit_fault(Method::POST, "/api/memberships", request.clone())
        .await?;
    let first = h.call(Method::POST, "/api/memberships", request.clone());
    let second = h.call(Method::POST, "/api/memberships", request);
    let (first, second) = tokio::join!(first, second);
    let first = first?;
    let second = second?;
    ensure!((first.status.is_success() && second.status == StatusCode::CONFLICT)
        || (second.status.is_success() && first.status == StatusCode::CONFLICT),
        "concurrent duplicate creation did not deterministically yield one success and one 409: {},{}", first.status,second.status);
    let created = if first.status.is_success() {
        first
    } else {
        second
    };
    let employee = successful(&created)?;
    ensure!(
        created.body["memberType"] == "EMPLOYEE" && created.body["identityVersion"] == 1,
        "create member DTO has incorrect lifecycle defaults"
    );
    let detail_path = format!("/api/memberships/{employee}");
    let detail = h.call(Method::GET, &detail_path, Value::Null).await?;
    ensure!(
        successful(&detail)? == employee,
        "member detail did not locate the precise member ID"
    );
    let denied = call(
        &h.app,
        &h.child_admin,
        Method::GET,
        &detail_path,
        Value::Null,
    )
    .await?;
    ensure!(
        denied.status == StatusCode::NOT_FOUND,
        "member detail escaped the actor's organization subtree"
    );
    let listing = h
        .call(
            Method::GET,
            &format!("/api/memberships?userId={target}&memberType=EMPLOYEE"),
            Value::Null,
        )
        .await?;
    ensure!(
        listing.status == StatusCode::OK
            && listing.body["total"] == 1
            && listing.body["records"][0]["id"] == employee.to_string(),
        "member listing did not apply explicit identity filter"
    );
    for (method, path, body) in [
        (
            Method::PUT,
            format!("{detail_path}/status"),
            json!({"status":"DISABLED","expectedIdentityVersion":1}),
        ),
        (
            Method::DELETE,
            detail_path.clone(),
            json!({"expectedIdentityVersion":1}),
        ),
        (
            Method::POST,
            format!("{detail_path}/second-identity"),
            json!({"memberType":"CUSTOMER","expectedIdentityVersion":1,"accountBindingConfirmed":true,"bindingSource":"owned"}),
        ),
        (
            Method::POST,
            format!("{detail_path}/replace-identity"),
            json!({"memberType":"CUSTOMER","expectedIdentityVersion":1,"accountBindingConfirmed":true,"bindingSource":"owned"}),
        ),
    ] {
        let denied = call(&disabled, &h.admin, method, &path, body).await?;
        ensure!(
            denied.status == StatusCode::FORBIDDEN,
            "a lifecycle mutation bypassed the default disabled gate"
        );
    }
    let blocked = h.create_member(target, h.root, "CUSTOMER").await?;
    ensure!(
        blocked.status == StatusCode::CONFLICT,
        "extension schema accepted a second identity"
    );
    let default_second = call(&customer_off,&h.admin,Method::POST,&format!("/api/memberships/{employee}/second-identity"),json!({"memberType":"CUSTOMER","expectedIdentityVersion":1,"accountBindingConfirmed":true,"bindingSource":"owned"})).await?;
    ensure!(
        default_second.status == StatusCode::FORBIDDEN,
        "second identity ignored creation gates"
    );
    let (_, sid) = session(&h.pool, target, h.tenant, h.root).await?;
    let status_path = format!("/api/memberships/{employee}/status");
    h.audit_fault(
        Method::PUT,
        &status_path,
        json!({"status":"DISABLED","expectedIdentityVersion":1}),
    )
    .await?;
    let changed = h
        .call(
            Method::PUT,
            &status_path,
            json!({"status":"DISABLED","expectedIdentityVersion":1}),
        )
        .await?;
    ensure!(
        successful(&changed)? == employee
            && changed.body["identityVersion"] == 2
            && changed.body["status"] == "DISABLED",
        "disable did not bump member version"
    );
    let revoked: bool =
        sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM platform_sessions WHERE id=$1")
            .bind(sid)
            .fetch_one(&h.pool)
            .await?;
    ensure!(
        revoked,
        "employee status change retained old platform session"
    );
    let stale = h
        .call(
            Method::PUT,
            &status_path,
            json!({"status":"ENABLED","expectedIdentityVersion":1}),
        )
        .await?;
    ensure!(
        stale.status == StatusCode::CONFLICT,
        "stale status version did not return 409"
    );
    let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE request_id=$1")
        .bind(stale.request_id)
        .fetch_one(&h.pool)
        .await?;
    ensure!(
        audits == 0,
        "rejected stale mutation emitted a success audit"
    );
    let bulk = h
        .call(
            Method::PUT,
            &format!("/api/users/{target}/memberships"),
            json!({"items":[{"organizationId":h.root,"position":"retained disabled employee"}]}),
        )
        .await?;
    ensure!(
        bulk.status.is_success(),
        "legacy employee bulk update failed: {}",
        bulk.body
    );
    let stored: (String, i64) =
        sqlx::query_as("SELECT status,identity_version FROM memberships WHERE id=$1")
            .bind(employee)
            .fetch_one(&h.pool)
            .await?;
    ensure!(
        stored == ("DISABLED".into(), 2),
        "legacy bulk update silently reenabled or replaced employee"
    );
    let deletion = format!("/api/memberships/{employee}");
    let missing_version = h.call(Method::DELETE, &deletion, json!({})).await?;
    ensure!(
        missing_version.status == StatusCode::BAD_REQUEST,
        "delete accepted a missing expected version"
    );
    h.audit_fault(
        Method::DELETE,
        &deletion,
        json!({"expectedIdentityVersion":2}),
    )
    .await?;
    let removed = h
        .call(
            Method::DELETE,
            &deletion,
            json!({"expectedIdentityVersion":2}),
        )
        .await?;
    ensure!(
        removed.status.is_success(),
        "explicit employee removal failed: {}",
        removed.body
    );
    eprintln!("identity_phase01 lifecycle_gates_scope_concurrency=PASS status_audit_rollback=PASS legacy_disabled_employee=PASS");
    Ok(())
}

async fn target_schema_contract(h: &Harness) -> Result<()> {
    // This contraction exists ONLY inside the owned, disposable test database.
    // The shipping migration intentionally keeps the old unique constraint.
    sqlx::query(
        "ALTER TABLE memberships DROP CONSTRAINT IF EXISTS memberships_user_id_organization_id_key",
    )
    .execute(&h.pool)
    .await?;
    let subject = user(&h.pool, "identity-dual-subject", false).await?;
    let employee = successful(&h.create_member(subject, h.child, "EMPLOYEE").await?)?;
    let second_path = format!("/api/memberships/{employee}/second-identity");
    let second_body = json!({"memberType":"CUSTOMER","expectedIdentityVersion":1,
        "accountBindingConfirmed":true,"bindingSource":"owned customer invitation fixture"});
    h.audit_fault(Method::POST, &second_path, second_body.clone())
        .await?;
    let added = h
        .call(Method::POST, &second_path, second_body.clone())
        .await?;
    let customer = successful(&added)?;
    ensure!(
        customer != employee && added.body["memberType"] == "CUSTOMER",
        "second identity reused employee row"
    );
    db_reject(
        &h.pool,
        &format!("UPDATE memberships SET position='Employee-only field' WHERE id='{customer}'"),
        "23514",
    )
    .await?;
    let source: (String, i64) =
        sqlx::query_as("SELECT status,identity_version FROM memberships WHERE id=$1")
            .bind(employee)
            .fetch_one(&h.pool)
            .await?;
    ensure!(
        source == ("ENABLED".into(), 1),
        "adding a second identity changed source employee"
    );
    let no_target_version = h
        .call(Method::POST, &second_path, second_body.clone())
        .await?;
    ensure!(
        no_target_version.status == StatusCode::BAD_REQUEST,
        "existing target reuse accepted no target generation"
    );
    let mut reuse = second_body.clone();
    reuse["expectedTargetIdentityVersion"] = json!(1);
    let reused = h.call(Method::POST, &second_path, reuse.clone()).await?;
    ensure!(
        successful(&reused)? == customer,
        "target reuse allocated a duplicate membership"
    );
    let disabled = h
        .call(
            Method::PUT,
            &format!("/api/memberships/{customer}/status"),
            json!({"status":"DISABLED","expectedIdentityVersion":1}),
        )
        .await?;
    successful(&disabled)?;
    reuse["expectedTargetIdentityVersion"] = json!(2);
    let denied = h.call(Method::POST, &second_path, reuse.clone()).await?;
    ensure!(
        denied.status == StatusCode::CONFLICT,
        "disabled target was implicitly restored"
    );
    reuse["enableTarget"] = json!(true);
    reuse["expectedTargetIdentityVersion"] = json!(1);
    let stale = h.call(Method::POST, &second_path, reuse.clone()).await?;
    ensure!(
        stale.status == StatusCode::CONFLICT,
        "stale disabled target generation was accepted"
    );
    reuse["expectedTargetIdentityVersion"] = json!(2);
    h.audit_fault(Method::POST, &second_path, reuse.clone())
        .await?;
    let enabled = h.call(Method::POST, &second_path, reuse).await?;
    ensure!(
        successful(&enabled)? == customer
            && enabled.body["identityVersion"] == 3
            && enabled.body["status"] == "ENABLED",
        "explicit target enable lost ID or version"
    );
    let (_, employee_sid) = session(&h.pool, subject, h.tenant, h.child).await?;
    let bulk_path = format!("/api/users/{subject}/memberships");
    let before_customer: String =
        sqlx::query_scalar("SELECT to_jsonb(m)::text FROM memberships m WHERE id=$1")
            .bind(customer)
            .fetch_one(&h.pool)
            .await?;
    let bulk = h
        .call(
            Method::PUT,
            &bulk_path,
            json!({"items":[{"organizationId":h.child,"position":"Employee profile only"}]}),
        )
        .await?;
    ensure!(
        bulk.status.is_success(),
        "legacy dual-row bulk update failed: {}",
        bulk.body
    );
    let after_customer: String =
        sqlx::query_scalar("SELECT to_jsonb(m)::text FROM memberships m WHERE id=$1")
            .bind(customer)
            .fetch_one(&h.pool)
            .await?;
    ensure!(
        before_customer == after_customer,
        "legacy employee bulk update changed customer row"
    );
    let removed = h
        .call(
            Method::DELETE,
            &format!("/api/memberships/{customer}"),
            json!({"expectedIdentityVersion":3}),
        )
        .await?;
    ensure!(
        removed.status.is_success(),
        "customer removal failed: {}",
        removed.body
    );
    let employee_live: bool =
        sqlx::query_scalar("SELECT revoked_at IS NULL FROM platform_sessions WHERE id=$1")
            .bind(employee_sid)
            .fetch_one(&h.pool)
            .await?;
    ensure!(
        employee_live,
        "customer deletion revoked the same-organization employee session"
    );
    let readded = h
        .call(Method::POST, &second_path, second_body.clone())
        .await?;
    let replacement_target = successful(&readded)?;
    let replace_path = format!("/api/memberships/{employee}/replace-identity");
    let replace = json!({"memberType":"CUSTOMER","expectedIdentityVersion":1,"expectedTargetIdentityVersion":1,
        "accountBindingConfirmed":true,"bindingSource":"owned explicit replacement fixture"});
    let other_employee = successful(&h.create_member(subject, h.sibling, "EMPLOYEE").await?)?;
    let (_, other_session) = session(&h.pool, subject, h.tenant, h.sibling).await?;
    let application: Uuid = sqlx::query_scalar("SELECT id FROM applications WHERE code='system'")
        .fetch_one(&h.pool)
        .await?;
    for membership in [employee, other_employee] {
        sqlx::query("INSERT INTO membership_data_grants(id,membership_id,application_id,resource_code,action_code,scope_type) VALUES($1,$2,$3,'replacement-fixture','read','SELF')")
            .bind(new_uuid()).bind(membership).bind(application).execute(&h.pool).await?;
    }
    let ancestor_role = new_uuid();
    let departed_role = new_uuid();
    for (role, org) in [(ancestor_role, h.root), (departed_role, h.child)] {
        sqlx::query("INSERT INTO roles(id,tenant_id,organization_id,name) VALUES($1,$2,$3,'Replacement role fixture')")
            .bind(role).bind(h.tenant).bind(org).execute(&h.pool).await?;
        sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3)")
            .bind(subject)
            .bind(role)
            .bind(org)
            .execute(&h.pool)
            .await?;
    }
    h.audit_fault(Method::POST, &replace_path, replace.clone())
        .await?;
    let replaced = h.call(Method::POST, &replace_path, replace).await?;
    ensure!(
        successful(&replaced)? == replacement_target,
        "replacement did not reuse existing customer row"
    );
    let source: (String, i64, String) =
        sqlx::query_as("SELECT status,identity_version,member_type FROM memberships WHERE id=$1")
            .bind(employee)
            .fetch_one(&h.pool)
            .await?;
    ensure!(
        source == ("DISABLED".into(), 2, "EMPLOYEE".into()),
        "replacement mutated source type or omitted source disable/version"
    );
    let grants: Vec<Uuid> = sqlx::query_scalar("SELECT membership_id FROM membership_data_grants WHERE membership_id=ANY($1) ORDER BY membership_id")
        .bind(vec![employee,other_employee]).fetch_all(&h.pool).await?;
    ensure!(
        grants == vec![other_employee],
        "replacement cleared another member's grant or retained departed source grant"
    );
    let roles: Vec<Uuid> =
        sqlx::query_scalar("SELECT role_id FROM user_roles WHERE user_id=$1 AND role_id=ANY($2)")
            .bind(subject)
            .bind(vec![ancestor_role, departed_role])
            .fetch_all(&h.pool)
            .await?;
    ensure!(
        roles == vec![ancestor_role],
        "replacement role cleanup ignored support from the other organization employee"
    );
    let other_live: bool =
        sqlx::query_scalar("SELECT revoked_at IS NULL FROM platform_sessions WHERE id=$1")
            .bind(other_session)
            .fetch_one(&h.pool)
            .await?;
    ensure!(
        other_live,
        "replacement revoked another organization's employee session"
    );

    let only_customer = user(&h.pool, "identity-customer-only", false).await?;
    let customer_id = member(&h.pool, only_customer, h.root, "CUSTOMER").await?;
    let old_path = format!("/api/users/{only_customer}/memberships");
    let old_body =
        json!({"items":[{"organizationId":h.root,"position":"New employee relationship"}]});
    let closed = app(&h.pool, true, true, false)?;
    let denied = call(&closed, &h.admin, Method::PUT, &old_path, old_body.clone()).await?;
    ensure!(
        denied.status == StatusCode::FORBIDDEN,
        "legacy bulk creation bypassed dual identity gate"
    );
    let allowed = h.call(Method::PUT, &old_path, old_body).await?;
    ensure!(
        allowed.status.is_success(),
        "customer-only legacy employee addition failed: {}",
        allowed.body
    );
    let rows: Vec<(Uuid,String)> = sqlx::query_as("SELECT id,member_type FROM memberships WHERE user_id=$1 AND organization_id=$2 ORDER BY member_type")
        .bind(only_customer).bind(h.root).fetch_all(&h.pool).await?;
    ensure!(
        rows.len() == 2
            && rows
                .iter()
                .any(|row| row == &(customer_id, "CUSTOMER".into()))
            && rows
                .iter()
                .any(|row| row.0 != customer_id && row.1 == "EMPLOYEE"),
        "legacy employee addition overwrote customer identity"
    );
    let emptied = h.call(Method::PUT, &old_path, json!({"items":[]})).await?;
    ensure!(
        emptied.status.is_success(),
        "legacy employee removal failed: {}",
        emptied.body
    );
    let remaining: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM memberships WHERE user_id=$1 AND organization_id=$2")
            .bind(only_customer)
            .bind(h.root)
            .fetch_all(&h.pool)
            .await?;
    ensure!(
        remaining == vec![customer_id],
        "legacy employee removal deleted customer identity"
    );
    let injected = h
        .call(
            Method::PUT,
            &old_path,
            json!({"items":[{"organizationId":h.root,"memberType":"CUSTOMER","status":"ENABLED"}]}),
        )
        .await?;
    ensure!(
        injected.status == StatusCode::BAD_REQUEST,
        "legacy untyped DTO accepted member type/status injection"
    );
    eprintln!("identity_phase01 target_fixture_second_replace_reuse=PASS customer_delete_keeps_employee_session=PASS legacy_bulk_dual=PASS");
    Ok(())
}

async fn legacy_password_grant(
    h: &Harness,
    username: &str,
    password: &str,
) -> Result<(StatusCode, Value)> {
    let request = Request::builder().method(Method::POST).uri("/oauth/token")
        .header(header::CONTENT_TYPE,"application/x-www-form-urlencoded")
        .header("x-request-id",new_uuid().to_string())
        .extension(ConnectInfo("192.0.2.31:30031".parse::<SocketAddr>()?))
        .body(Body::from(format!("grant_type=password&client_id=identity-legacy-issuance&client_secret=owned-phase01-client&username={username}&password={password}")))?;
    let response = tokio::time::timeout(Duration::from_secs(25), h.app.clone().oneshot(request))
        .await
        .map_err(|_| anyhow!("legacy password issuance fixture timed out"))??;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}

async fn legacy_customer_issuance_guard(h: &Harness) -> Result<()> {
    const PASSWORD: &str = "Owned-identity-phase01-password-123!";
    const USERNAME: &str = "identity-legacy-issuance-subject";
    let subject = user(&h.pool, USERNAME, false).await?;
    let password_hash =
        tokio::task::spawn_blocking(|| crate::util::password::hash_password(PASSWORD)).await??;
    sqlx::query("UPDATE users SET password_hash=$2 WHERE id=$1")
        .bind(subject)
        .bind(password_hash)
        .execute(&h.pool)
        .await?;
    member(&h.pool, subject, h.root, "CUSTOMER").await?;
    sqlx::query("INSERT INTO oauth_clients(id,client_id,client_name,client_secret_hash,grant_types) VALUES($1,'identity-legacy-issuance','Legacy guard fixture',$2,'password')")
        .bind(new_uuid()).bind(crate::util::hash::sha256_hex("owned-phase01-client"))
        .execute(&h.pool).await?;
    let customer_login = h
        .call(
            Method::POST,
            "/api/auth/login",
            json!({"username":USERNAME,"password":PASSWORD}),
        )
        .await?;
    ensure!(
        customer_login.status == StatusCode::FORBIDDEN,
        "customer-only account acquired an old employee login ticket"
    );
    let now = chrono::Utc::now().timestamp();
    let account_version: i64 = sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
        .bind(subject)
        .fetch_one(&h.pool)
        .await?;
    // An already proven account is still insufficient for a platform employee context.
    let ticket = jwt::sign(
        &jwt::Claims {
            sub: subject.to_string(),
            typ: "credential".into(),
            tenant: None,
            org: None,
            cid: None,
            scope: None,
            sid: None,
            ver: Some(account_version),
            aud: None,
            jti: None,
            iat: now,
            exp: now + 300,
        },
        SECRET,
    )?;
    let context = call(
        &h.app,
        &ticket,
        Method::POST,
        "/api/auth/context",
        json!({"organizationId":h.root}),
    )
    .await?;
    ensure!(
        context.status == StatusCode::UNAUTHORIZED,
        "customer-only account acquired an old unbound employee context"
    );
    let (old_session, _) = session(&h.pool, subject, h.tenant, h.root).await?;
    let live = call(
        &h.app,
        &old_session,
        Method::GET,
        "/api/memberships",
        Value::Null,
    )
    .await?;
    ensure!(
        live.status == StatusCode::UNAUTHORIZED,
        "unbound old session treated a customer relationship as employee proof"
    );
    let denied = legacy_password_grant(h, USERNAME, PASSWORD).await?;
    ensure!(
        denied.0 == StatusCode::BAD_REQUEST && denied.1["error"] == "invalid_grant",
        "customer-only account acquired an old user OAuth token"
    );
    let issued: i64 = sqlx::query_scalar("SELECT count(*) FROM oauth_tokens WHERE user_id=$1")
        .bind(subject)
        .fetch_one(&h.pool)
        .await?;
    ensure!(
        issued == 0,
        "denied customer OAuth grant persisted credentials"
    );
    let employee = successful(&h.create_member(subject, h.root, "EMPLOYEE").await?)?;
    let employee_login = h
        .call(
            Method::POST,
            "/api/auth/login",
            json!({"username":USERNAME,"password":PASSWORD}),
        )
        .await?;
    ensure!(
        employee_login.status == StatusCode::OK,
        "valid employee beside a customer could not use legacy login"
    );
    let ticket = employee_login.body["ticket"]
        .as_str()
        .ok_or_else(|| anyhow!("employee login omitted its ticket"))?;
    let employee_context = call(
        &h.app,
        ticket,
        Method::POST,
        "/api/auth/context",
        json!({"organizationId":h.root}),
    )
    .await?;
    ensure!(
        employee_context.status == StatusCode::OK,
        "valid employee beside a customer could not select an employee context"
    );
    let granted = legacy_password_grant(h, USERNAME, PASSWORD).await?;
    ensure!(
        granted.0 == StatusCode::OK,
        "valid employee beside a customer could not obtain its legacy user OAuth grant"
    );
    ensure!(
        granted.1["access_token"].is_string(),
        "employee OAuth response lacks access_token"
    );
    let disabled = h
        .call(
            Method::PUT,
            &format!("/api/memberships/{employee}/status"),
            json!({"status":"DISABLED","expectedIdentityVersion":1}),
        )
        .await?;
    successful(&disabled)?;
    let denied = legacy_password_grant(h, USERNAME, PASSWORD).await?;
    ensure!(
        denied.0 == StatusCode::BAD_REQUEST && denied.1["error"] == "invalid_grant",
        "disabled employee was replaced by customer as old OAuth eligibility proof"
    );
    eprintln!("identity_phase01 legacy_customer_login_context_oauth_rejected=PASS employee_positive_and_disable=PASS");
    Ok(())
}

#[tokio::test]
#[ignore = "creates and drops identity_phase01_* databases via explicit IDENTITY_PHASE01_TEST_DATABASE_URL"]
async fn membership_lifecycle_phase01_contract_matrix() -> Result<()> {
    let database = Database::create(i64::MAX).await?;
    let result = async {
        let harness = Harness::create(&database.pool).await?;
        database_contract(&harness).await?;
        membership_reason_contract(&harness).await?;
        expansion_api_contract(&harness).await?;
        target_schema_contract(&harness).await?;
        legacy_customer_issuance_guard(&harness).await?;
        Ok(())
    }
    .await;
    database.finish(result).await
}

#[tokio::test]
#[ignore = "fresh full migration plus real seed and UUID checks on an owned identity_phase01_* database"]
async fn membership_lifecycle_phase01_fresh_install_seed_v7() -> Result<()> {
    let database = Database::create(i64::MAX).await?;
    let result = async {
        let initial_credentials: i64 = sqlx::query_scalar("SELECT (SELECT count(*) FROM platform_sessions)+(SELECT count(*) FROM oauth_tokens)+(SELECT count(*) FROM oauth_authorization_codes)+(SELECT count(*) FROM oauth_device_codes)")
            .fetch_one(&database.pool).await?;
        ensure!(initial_credentials == 0,"fresh schema contains issued credentials");
        crate::seed::run(&database.pool).await?;
        let subjects: i64 = sqlx::query_scalar("SELECT count(*) FROM users").fetch_one(&database.pool).await?;
        ensure!(subjects > 0,"real seed did not initialize account fixtures");
        let baseline = uuid_baseline(&database.pool).await?;
        let before = seed_identity_snapshot(&database.pool).await?;
        crate::seed::run(&database.pool).await?;
        ensure!(before == seed_identity_snapshot(&database.pool).await?,"restarted seed changed IDs, grants or other persisted fixture state");
        let unsafe_members: i64 = sqlx::query_scalar("SELECT count(*) FROM memberships WHERE member_type<>'EMPLOYEE' OR status<>'ENABLED' OR identity_version<>1")
            .fetch_one(&database.pool).await?;
        let unsafe_policies: i64 = sqlx::query_scalar("SELECT (SELECT count(*) FROM applications WHERE allowed_member_types<>ARRAY['EMPLOYEE']::text[]) + (SELECT count(*) FROM oauth_clients WHERE (subject_mode='MACHINE' AND cardinality(allowed_member_types)<>0) OR (subject_mode<>'MACHINE' AND allowed_member_types<>ARRAY['EMPLOYEE']::text[]))")
            .fetch_one(&database.pool).await?;
        ensure!(unsafe_members == 0 && unsafe_policies == 0,"seed widened customer or client admission");
        db_reject(&database.pool,&format!("INSERT INTO users(id,username,password_hash) VALUES('{}','admin','fixture')",new_uuid()),"23505").await?;
        eprintln!("identity_phase01 fresh_full_install_seed=PASS seed_restart_idempotency=PASS uuid_columns={} uuid_values={} uuid_defaults=0 foreign_keys={} subjects={subjects}",baseline.0,baseline.1,baseline.2);
        Ok(())
    }.await;
    database.finish(result).await
}

async fn wait_for_identity_user_barrier(pool: &PgPool, owner: i32) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity \
                WHERE datname=current_database() AND wait_event_type='Lock' \
                AND $1=ANY(pg_blocking_pids(pid)) AND query LIKE '%users%' \
                AND query ILIKE '%FOR%UPDATE%')",
            )
            .bind(owner)
            .fetch_one(pool)
            .await?;
            if waiting {
                return Ok::<(), anyhow::Error>(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(|_| anyhow!("membership request did not reach its observed user lock barrier"))?
}

#[tokio::test]
#[ignore = "deterministic authority revocation during user-lock waits inside an owned identity_phase01_* database"]
async fn membership_lifecycle_phase01_authority_rechecked_after_lock_wait() -> Result<()> {
    let database = Database::create(i64::MAX).await?;
    let result = async {
        let mut h = Harness::create(&database.pool).await?;
        for scenario in ["session","permission","organization_status","organization_expiry","tenant_status","tenant_expiry"] {
            let target = user(&h.pool,&format!("identity-wait-{scenario}"),false).await?;
            let existing = if matches!(scenario,"permission" | "organization_expiry" | "tenant_expiry") {
                Some(member(&h.pool,target,h.child,"EMPLOYEE").await?)
            } else { None };
            let (fresh_actor,session_id) = session(&h.pool,h.actor,h.tenant,h.root).await?;
            h.admin = fresh_actor;
            let mut blocker = h.pool.begin().await?;
            let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *blocker).await?;
            sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
                .bind(target).execute(&mut *blocker).await?;
            let request_app = h.app.clone();
            let request_token = h.admin.clone();
            let target_org = h.child;
            let request = tokio::spawn(async move {
                if let Some(member) = existing {
                    call(&request_app,&request_token,Method::PUT,&format!("/api/memberships/{member}/status"),
                        json!({"status":"DISABLED","expectedIdentityVersion":1})).await
                } else {
                    call(&request_app,&request_token,Method::POST,"/api/memberships",json!({
                        "userId":target,"organizationId":target_org,"memberType":"EMPLOYEE"
                    })).await
                }
            });
            if let Err(error) = wait_for_identity_user_barrier(&h.pool,pid).await {
                blocker.rollback().await?;
                request.abort();
                let _ = request.await;
                return Err(error);
            }
            match scenario {
                "session" => {
                    sqlx::query("UPDATE platform_sessions SET revoked_at=clock_timestamp() WHERE id=$1")
                        .bind(session_id).execute(&h.pool).await?;
                }
                "permission" => {
                    sqlx::query("DELETE FROM organization_menus WHERE organization_id=$1 AND menu_id IN(SELECT id FROM menus WHERE permission='system:user')")
                        .bind(h.root).execute(&h.pool).await?;
                }
                "organization_status" => {
                    sqlx::query("UPDATE organizations SET status='DISABLED' WHERE id=$1")
                        .bind(h.child).execute(&h.pool).await?;
                }
                "organization_expiry" => {
                    // The request transaction already exists. Its fixed now()
                    // predates this expiry; only a current authority clock rejects it.
                    sqlx::query("UPDATE organizations SET expires_at=clock_timestamp() WHERE id=$1")
                        .bind(h.child).execute(&h.pool).await?;
                }
                "tenant_status" => {
                    sqlx::query("UPDATE tenants SET status='DISABLED' WHERE id=$1")
                        .bind(h.tenant).execute(&h.pool).await?;
                }
                "tenant_expiry" => {
                    sqlx::query("UPDATE tenants SET expires_at=clock_timestamp() WHERE id=$1")
                        .bind(h.tenant).execute(&h.pool).await?;
                }
                _ => unreachable!(),
            }
            blocker.rollback().await?;
            let reply = tokio::time::timeout(Duration::from_secs(20),request)
                .await.map_err(|_| anyhow!("authority-revoked membership request did not finish"))???;
            ensure!([StatusCode::UNAUTHORIZED,StatusCode::FORBIDDEN,StatusCode::NOT_FOUND].contains(&reply.status),
                "{scenario} changed during the lock wait but returned {}",reply.status);
            let written: i64 = sqlx::query_scalar("SELECT (SELECT count(*) FROM memberships WHERE user_id=$1)+(SELECT count(*) FROM audit_logs WHERE request_id=$2)")
                .bind(target).bind(reply.request_id).fetch_one(&h.pool).await?;
            ensure!(written == i64::from(existing.is_some()),"{scenario} revocation committed a member or success audit after the wait");
            if let Some(member) = existing {
                let stored: (String,i64) = sqlx::query_as("SELECT status,identity_version FROM memberships WHERE id=$1")
                    .bind(member).fetch_one(&h.pool).await?;
                ensure!(stored == ("ENABLED".into(),1),"{scenario} revocation still committed a status/version mutation");
            }
            sqlx::query("UPDATE organizations SET status='ENABLED',expires_at=NULL WHERE id=$1")
                .bind(h.child).execute(&h.pool).await?;
            sqlx::query("UPDATE tenants SET status='ENABLED',expires_at=NULL WHERE id=$1")
                .bind(h.tenant).execute(&h.pool).await?;
            sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) SELECT $1,id FROM menus WHERE permission='system:user' ON CONFLICT DO NOTHING")
                .bind(h.root).execute(&h.pool).await?;
            eprintln!("identity_phase01 authority_after_user_lock_wait={scenario} result=PASS");
        }
        Ok(())
    }.await;
    database.finish(result).await
}
