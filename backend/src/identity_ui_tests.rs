//! Phase 5 browser identity contracts on explicitly selected disposable databases.
//! No .env or existing application database is used, and only owned databases are removed.
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
use crate::util::{jwt, password, uuid7};

const SECRET: &str = "identity-phase05-owned-fixture-signing-key";
const PASSWORD: &str = "Owned-Phase05-Fixture-Password9!";
const CUSTOMER_SOURCE: &str = "identity-phase05-customer-source";

struct Database {
    pool: PgPool,
    admin: PgPool,
    name: String,
}

impl Database {
    async fn create() -> Result<Self> {
        let url = std::env::var("IDENTITY_PHASE05_TEST_DATABASE_URL")
            .map_err(|_| anyhow!("explicit IDENTITY_PHASE05_TEST_DATABASE_URL is required"))?;
        let options = PgConnectOptions::from_str(&url)
            .map_err(|_| anyhow!("invalid isolated Phase 5 database configuration"))?;
        ensure!(
            options.get_database() == Some("postgres"),
            "Phase 5 fixture administration must explicitly select postgres"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("isolated Phase 5 administration connection failed"))?;
        let name = format!("identity_phase05_{}", uuid7::new_uuid().simple());
        ensure!(name.len() <= 63, "owned database name is too long");
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
                return Err(anyhow!("owned Phase 5 database connection failed"));
            }
        };
        let database = Self { pool, admin, name };
        // These exercise the current HTTP adapters, including their nullable
        // BFF binding and source projection, rather than a historical upgrade.
        let migrator = sqlx::migrate!("./migrations");
        if let Err(error) = migrator.run(&database.pool).await {
            let _ = database.finish(Ok(())).await;
            return Err(error.into());
        }
        Ok(database)
    }

    async fn finish(self, result: Result<()>) -> Result<()> {
        ensure!(
            self.name.starts_with("identity_phase05_")
                && self.name.len() <= 63
                && self
                    .name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_'),
            "refusing cleanup outside an owned Phase 5 database"
        );
        let closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let removed = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await;
        self.admin.close().await;
        eprintln!(
            "identity_phase05_cleanup database={} assertions_ok={} database_removed={} pool_closed={closed}",
            self.name,
            result.is_ok(),
            removed.is_ok()
        );
        result?;
        removed?;
        ensure!(closed, "owned Phase 5 pool did not close");
        Ok(())
    }
}

fn app(pool: &PgPool, lifecycle: bool, customer: bool, dual: bool) -> Result<Router> {
    let config = Config::from_values(&HashMap::from([
        ("APP_ENV".into(), "test".into()),
        (
            "DATABASE_URL".into(),
            "postgres://unused/identity-phase05".into(),
        ),
        ("JWT_SECRET".into(), SECRET.into()),
        ("MEMBERSHIP_LIFECYCLE_ENABLED".into(), lifecycle.to_string()),
        ("CUSTOMER_MEMBERSHIPS_ENABLED".into(), customer.to_string()),
        ("DUAL_MEMBERSHIPS_ENABLED".into(), dual.to_string()),
    ]))?;
    let state = AppState::new(pool.clone(), Arc::new(config));
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
}

async fn call(
    app: &Router,
    method: Method,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> Result<Reply> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-request-id", uuid7::new_uuid().to_string())
        .extension(ConnectInfo("192.0.2.35:30035".parse::<SocketAddr>()?));
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = tokio::time::timeout(
        Duration::from_secs(25),
        app.clone()
            .oneshot(request.body(Body::from(serde_json::to_vec(&body)?))?),
    )
    .await
    .map_err(|_| anyhow!("owned Phase 5 HTTP request timed out: {path}"))??;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok(Reply {
        status,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    })
}

fn expect(reply: &Reply, expected: StatusCode, operation: &str) -> Result<()> {
    // Token-bearing successful responses must never enter error output.
    ensure!(
        reply.status == expected,
        "{operation}: expected {expected}, received {} (error={})",
        reply.status,
        reply
            .body
            .get("error")
            .or_else(|| reply.body.get("code"))
            .unwrap_or(&Value::Null)
    );
    Ok(())
}

async fn user(pool: &PgPool, name: &str, account_version: i64, login: bool) -> Result<Uuid> {
    let id = uuid7::new_uuid();
    let password_hash = if login {
        password::hash_password(PASSWORD)?
    } else {
        "phase05-unusable-fixture-password".into()
    };
    sqlx::query("INSERT INTO users(id,username,password_hash,session_version) VALUES($1,$2,$3,$4)")
        .bind(id)
        .bind(name)
        .bind(password_hash)
        .bind(account_version)
        .execute(pool)
        .await?;
    Ok(id)
}

async fn tenant(pool: &PgPool) -> Result<(Uuid, Uuid)> {
    let tenant = uuid7::new_uuid();
    sqlx::query("INSERT INTO tenants(id,name,code) VALUES($1,'Owned Phase 5','identity-phase05')")
        .bind(tenant)
        .execute(pool)
        .await?;
    Ok((tenant, organization(pool, tenant, None).await?))
}

async fn organization(pool: &PgPool, tenant: Uuid, parent: Option<Uuid>) -> Result<Uuid> {
    let id = uuid7::new_uuid();
    sqlx::query("INSERT INTO organizations(id,tenant_id,parent_id,name) VALUES($1,$2,$3,'Owned Phase 5 organization')")
        .bind(id).bind(tenant).bind(parent).execute(pool).await?;
    Ok(id)
}

#[derive(Clone, Copy)]
struct Member {
    id: Uuid,
    user: Uuid,
    tenant: Uuid,
    org: Uuid,
    kind: &'static str,
    status: &'static str,
    version: i64,
    account_version: i64,
}

async fn member(
    pool: &PgPool,
    user: Uuid,
    org: Uuid,
    kind: &'static str,
    status: &'static str,
    version: i64,
) -> Result<Member> {
    let id = uuid7::new_uuid();
    let (tenant, account_version): (Uuid, i64) = sqlx::query_as(
        "SELECT o.tenant_id,u.session_version FROM organizations o CROSS JOIN users u WHERE o.id=$1 AND u.id=$2",
    ).bind(org).bind(user).fetch_one(pool).await?;
    sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,status,identity_version) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(id).bind(user).bind(org).bind(kind).bind(status).bind(version).execute(pool).await?;
    Ok(Member {
        id,
        user,
        tenant,
        org,
        kind,
        status,
        version,
        account_version,
    })
}

async fn allow_owned_dual_fixture(pool: &PgPool) -> Result<()> {
    // Phase 6 contraction is simulated only inside this newly owned database.
    sqlx::query("ALTER TABLE memberships DROP CONSTRAINT memberships_user_id_organization_id_key")
        .execute(pool)
        .await?;
    Ok(())
}

async fn session(pool: &PgPool, member: Member) -> Result<String> {
    let source = if member.kind == "EMPLOYEE" {
        "platform-console-legacy"
    } else {
        sqlx::query("INSERT INTO oauth_clients(id,client_id,client_name,client_type,grant_types,subject_mode,allowed_member_types) VALUES($1,$2,'Owned Phase 5 customer source','PUBLIC','authorization_code','USER',ARRAY['CUSTOMER']::text[]) ON CONFLICT(client_id) DO NOTHING")
            .bind(uuid7::new_uuid()).bind(CUSTOMER_SOURCE).execute(pool).await?;
        CUSTOMER_SOURCE
    };
    let sid = uuid7::new_uuid();
    sqlx::query("INSERT INTO platform_sessions(id,user_id,tenant_id,organization_id,expires_at,account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version) VALUES($1,$2,$3,$4,now()+interval '1 hour',$5,$6,$7,$8,'urn:saas:legacy-platform',$9,1)")
        .bind(sid).bind(member.user).bind(member.tenant).bind(member.org).bind(member.account_version)
        .bind(member.id).bind(member.kind).bind(member.version).bind(source).execute(pool).await?;
    ensure!(
        session_active(pool, sid).await?,
        "fixture session must be active"
    );
    let now = chrono::Utc::now().timestamp();
    jwt::sign(
        &jwt::Claims {
            sub: member.user.to_string(),
            typ: "session".into(),
            tenant: Some(member.tenant.to_string()),
            org: Some(member.org.to_string()),
            cid: None,
            scope: None,
            sid: Some(sid.to_string()),
            ver: Some(member.account_version),
            aud: None,
            jti: None,
            iat: now,
            exp: now + 3600,
        },
        SECRET,
    )
}

async fn session_active(pool: &PgPool, id: Uuid) -> Result<bool> {
    Ok(sqlx::query_scalar("SELECT platform_session_active($1)")
        .bind(id)
        .fetch_one(pool)
        .await?)
}

fn proof(body: &Value, expected: Member) -> Result<()> {
    let id: Uuid = body["membershipId"]
        .as_str()
        .ok_or_else(|| anyhow!("public identity proof lacks membershipId"))?
        .parse()?;
    ensure!(uuid7::is_uuid_v7(id), "public membershipId must be UUID v7");
    ensure!(
        id == expected.id,
        "public proof selected another membership"
    );
    ensure!(
        body["memberType"] == "EMPLOYEE",
        "public proof is not employee"
    );
    ensure!(
        body["identityVersion"].as_str() == Some(expected.version.to_string().as_str()),
        "identityVersion must preserve the actual positive generation as a decimal string"
    );
    ensure!(
        body["accountVersion"].as_str() == Some(expected.account_version.to_string().as_str()),
        "accountVersion must preserve the actual nonnegative generation as a decimal string"
    );
    ensure!(
        body["user"]["id"] == json!(expected.user),
        "public proof has another user"
    );
    ensure!(
        body["organization"]["id"] == json!(expected.org),
        "public proof has another organization"
    );
    ensure!(
        body["tenant"]["id"] == json!(expected.tenant),
        "public proof has another tenant"
    );
    Ok(())
}

fn choices(body: &Value, expected: &[Member]) -> Result<()> {
    let rows = body["memberships"]
        .as_array()
        .ok_or_else(|| anyhow!("public membership list is missing"))?;
    ensure!(
        rows.len() == expected.len(),
        "public membership list lost or added identities"
    );
    let mut actual = BTreeMap::new();
    for row in rows {
        let id: Uuid = row["membershipId"]
            .as_str()
            .ok_or_else(|| anyhow!("membership choice lacks membershipId"))?
            .parse()?;
        ensure!(uuid7::is_uuid_v7(id), "membership choice ID is not UUID v7");
        ensure!(
            actual.insert(id, row).is_none(),
            "duplicate membership choice"
        );
    }
    for member in expected {
        let row = actual
            .get(&member.id)
            .ok_or_else(|| anyhow!("actual membership identity is missing"))?;
        ensure!(
            row["memberType"] == member.kind,
            "membership type was inferred incorrectly"
        );
        ensure!(
            row["status"] == member.status,
            "membership status was inferred incorrectly"
        );
        ensure!(
            row["organizationId"] == json!(member.org),
            "membership organization differs"
        );
        ensure!(
            row["userId"] == json!(member.user),
            "membership account differs"
        );
        ensure!(
            row["identityVersion"].as_str() == Some(member.version.to_string().as_str()),
            "membership generation must be its actual decimal string"
        );
        let reason = if member.kind == "CUSTOMER" {
            Some("CLIENT_MEMBER_TYPE_NOT_ALLOWED")
        } else if member.status == "DISABLED" {
            Some("MEMBERSHIP_DISABLED")
        } else {
            None
        };
        ensure!(
            row["consoleSelectable"] == json!(reason.is_none()),
            "console choice eligibility differs"
        );
        ensure!(
            row["selectionDisabledReason"] == json!(reason),
            "console choice reason differs"
        );
    }
    Ok(())
}

async fn context_proof(pool: &PgPool, reply: &Reply, member: Member) -> Result<(String, Uuid)> {
    expect(reply, StatusCode::OK, "select employee context")?;
    proof(&reply.body, member)?;
    let token = reply.body["accessToken"]
        .as_str()
        .ok_or_else(|| anyhow!("context lacks an access token"))?
        .to_string();
    let claims = jwt::verify(&token, SECRET)?;
    let sid: Uuid = claims
        .sid
        .ok_or_else(|| anyhow!("session lacks sid"))?
        .parse()?;
    let actual: (Uuid, String, i64, i64, Uuid, Uuid, Uuid) = sqlx::query_as(
        "SELECT membership_id,member_type,identity_version,account_version,user_id,tenant_id,organization_id FROM platform_sessions WHERE id=$1 AND platform_session_active(id)",
    ).bind(sid).fetch_one(pool).await?;
    ensure!(
        actual
            == (
                member.id,
                "EMPLOYEE".into(),
                member.version,
                member.account_version,
                member.user,
                member.tenant,
                member.org
            ),
        "public context proof differs from the active server session"
    );
    Ok((token, sid))
}

async fn membership_snapshot(pool: &PgPool, user: Uuid) -> Result<String> {
    Ok(sqlx::query_scalar(
        "SELECT COALESCE(jsonb_agg(to_jsonb(m) ORDER BY m.id),'[]'::jsonb)::text FROM memberships m WHERE m.user_id=$1",
    ).bind(user).fetch_one(pool).await?)
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE05_TEST_DATABASE_URL and disposable database creation"]
async fn phase05_login_and_profile_show_identity_metadata_without_expanding_ticket_authority(
) -> Result<()> {
    let db = Database::create().await?;
    let result: Result<()> = async {
        allow_owned_dual_fixture(&db.pool).await?;
        let (tenant, root) = tenant(&db.pool).await?;
        let child = organization(&db.pool, tenant, Some(root)).await?;
        let disabled_org = organization(&db.pool, tenant, Some(root)).await?;
        let user = user(&db.pool, "phase05-browser-metadata", 9, true).await?;
        let employee = member(&db.pool, user, root, "EMPLOYEE", "ENABLED", 7).await?;
        let customer = member(&db.pool, user, root, "CUSTOMER", "ENABLED", 13).await?;
        let descendant = member(&db.pool, user, child, "EMPLOYEE", "ENABLED", 11).await?;
        let disabled = member(&db.pool, user, disabled_org, "EMPLOYEE", "DISABLED", 17).await?;
        let expected = [employee, customer, descendant, disabled];
        let app = app(&db.pool, false, false, false)?;
        let login = call(
            &app,
            Method::POST,
            "/api/auth/login",
            None,
            json!({"username":"phase05-browser-metadata","password":PASSWORD}),
        )
        .await?;
        expect(
            &login,
            StatusCode::OK,
            "login with an employee and other identity choices",
        )?;
        choices(&login.body, &expected)?;
        ensure!(
            login.body["accountCount"] == 1,
            "identity choices must not become account choices"
        );
        let ticket = login.body["ticket"]
            .as_str()
            .ok_or_else(|| anyhow!("login lacks ticket"))?;
        let payload: Value = jwt::verify_payload(ticket, SECRET)?;
        let accounts = payload["accounts"]
            .as_array()
            .ok_or_else(|| anyhow!("ticket lacks accounts"))?;
        ensure!(
            accounts.len() == 1 && accounts[0]["userId"] == json!(user),
            "ticket account differs"
        );
        let snapshots = accounts[0]["memberships"]
            .as_array()
            .ok_or_else(|| anyhow!("ticket lacks membership snapshots"))?;
        let actual: BTreeMap<Uuid, (Uuid, i64)> = snapshots
            .iter()
            .map(|snapshot| -> Result<_> {
                Ok((
                    snapshot["membershipId"]
                        .as_str()
                        .ok_or_else(|| anyhow!("ticket membership lacks ID"))?
                        .parse()?,
                    (
                        snapshot["organizationId"]
                            .as_str()
                            .ok_or_else(|| anyhow!("ticket membership lacks organization"))?
                            .parse()?,
                        snapshot["identityVersion"]
                            .as_i64()
                            .ok_or_else(|| anyhow!("ticket membership lacks exact generation"))?,
                    ),
                ))
            })
            .collect::<Result<_>>()?;
        ensure!(
            snapshots.len() == 2
                && actual
                    == BTreeMap::from([
                        (employee.id, (employee.org, employee.version)),
                        (descendant.id, (descendant.org, descendant.version))
                    ]),
            "disabled or customer display choices expanded password ticket authority"
        );
        let selected = call(
            &app,
            Method::POST,
            "/api/auth/context",
            Some(ticket),
            json!({"membershipId":employee.id}),
        )
        .await?;
        let (token, _) = context_proof(&db.pool, &selected, employee).await?;
        let profile = call(
            &app,
            Method::GET,
            "/api/auth/profile",
            Some(&token),
            Value::Null,
        )
        .await?;
        expect(&profile, StatusCode::OK, "read employee profile")?;
        proof(&profile.body, employee)?;
        choices(&profile.body, &expected)?;
        Ok(())
    }
    .await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE05_TEST_DATABASE_URL and disposable database creation"]
async fn phase05_context_selection_is_exact_and_switching_does_not_modify_memberships() -> Result<()>
{
    let db = Database::create().await?;
    let result: Result<()> = async {
        allow_owned_dual_fixture(&db.pool).await?;
        let (tenant, root) = tenant(&db.pool).await?;
        let child = organization(&db.pool, tenant, Some(root)).await?;
        let customer_org = organization(&db.pool, tenant, Some(root)).await?;
        let disabled_org = organization(&db.pool, tenant, Some(root)).await?;
        let actor = user(&db.pool, "phase05-switching", 4, false).await?;
        let employee = member(&db.pool, actor, root, "EMPLOYEE", "ENABLED", 5).await?;
        let customer = member(&db.pool, actor, root, "CUSTOMER", "ENABLED", 7).await?;
        let descendant = member(&db.pool, actor, child, "EMPLOYEE", "ENABLED", 9).await?;
        member(&db.pool, actor, customer_org, "CUSTOMER", "ENABLED", 3).await?;
        let disabled = member(&db.pool, actor, disabled_org, "EMPLOYEE", "DISABLED", 11).await?;
        let outsider = user(&db.pool, "phase05-other-user", 0, false).await?;
        let foreign = member(&db.pool, outsider, root, "EMPLOYEE", "ENABLED", 1).await?;
        let app = app(&db.pool, false, false, false)?;
        let ticket =
            crate::routes::auth::fixture_credential_ticket(&db.pool, actor, SECRET).await?;
        let before = membership_snapshot(&db.pool, actor).await?;
        for (body, status, label) in [
            (
                json!({"membershipId":customer.id,"organizationId":root}),
                StatusCode::FORBIDDEN,
                "customer ID must not fall back to same-organization employee",
            ),
            (
                json!({"membershipId":disabled.id}),
                StatusCode::FORBIDDEN,
                "disabled employee selection",
            ),
            (
                json!({"membershipId":foreign.id}),
                StatusCode::FORBIDDEN,
                "another user's membership selection",
            ),
            (
                json!({"membershipId":employee.id,"organizationId":child}),
                StatusCode::FORBIDDEN,
                "membership and organization mismatch",
            ),
            (
                json!({"membershipId":employee.id,"userId":outsider}),
                StatusCode::FORBIDDEN,
                "membership and account mismatch",
            ),
            (
                json!({"organizationId":customer_org}),
                StatusCode::FORBIDDEN,
                "organization-only customer fallback",
            ),
            (
                json!({"membershipId":Uuid::new_v4()}),
                StatusCode::BAD_REQUEST,
                "UUID v4 membership",
            ),
            (
                json!({"membershipId":employee.id,"userId":Uuid::new_v4()}),
                StatusCode::BAD_REQUEST,
                "UUID v4 account",
            ),
            (
                json!({"organizationId":Uuid::new_v4()}),
                StatusCode::BAD_REQUEST,
                "UUID v4 organization",
            ),
            (
                json!({"membershipId":employee.id,"memberType":"EMPLOYEE"}),
                StatusCode::BAD_REQUEST,
                "unknown selection field",
            ),
            (json!({}), StatusCode::BAD_REQUEST, "missing selection"),
        ] {
            let reply = call(&app, Method::POST, "/api/auth/context", Some(&ticket), body).await?;
            expect(&reply, status, label)?;
        }
        let compatible = call(
            &app,
            Method::POST,
            "/api/auth/context",
            Some(&ticket),
            json!({"organizationId":root}),
        )
        .await?;
        let (old_token, old_sid) = context_proof(&db.pool, &compatible, employee).await?;
        let rejected = call(
            &app,
            Method::POST,
            "/api/auth/context",
            Some(&old_token),
            json!({"membershipId":customer.id,"organizationId":root}),
        )
        .await?;
        expect(
            &rejected,
            StatusCode::FORBIDDEN,
            "session customer selection must not fall back",
        )?;
        ensure!(
            session_active(&db.pool, old_sid).await?,
            "rejected selection revoked a valid session"
        );
        let switched = call(
            &app,
            Method::POST,
            "/api/auth/context",
            Some(&old_token),
            json!({"membershipId":descendant.id,"organizationId":child,"userId":actor}),
        )
        .await?;
        let (fresh_token, fresh_sid) = context_proof(&db.pool, &switched, descendant).await?;
        ensure!(
            old_sid != fresh_sid,
            "context switch reused the old session"
        );
        ensure!(
            !session_active(&db.pool, old_sid).await?,
            "context switch left the old session active"
        );
        ensure!(
            membership_snapshot(&db.pool, actor).await? == before,
            "ordinary context selection mutated membership identity or status"
        );
        let old_profile = call(
            &app,
            Method::GET,
            "/api/auth/profile",
            Some(&old_token),
            Value::Null,
        )
        .await?;
        expect(
            &old_profile,
            StatusCode::UNAUTHORIZED,
            "profile from revoked old context",
        )?;
        let fresh_profile = call(
            &app,
            Method::GET,
            "/api/auth/profile",
            Some(&fresh_token),
            Value::Null,
        )
        .await?;
        expect(
            &fresh_profile,
            StatusCode::OK,
            "profile after context switch",
        )?;
        proof(&fresh_profile.body, descendant)?;

        // A ticket may not silently select a replacement generation of its offered identity.
        sqlx::query("UPDATE memberships SET identity_version=identity_version+1 WHERE id=$1")
            .bind(descendant.id)
            .execute(&db.pool)
            .await?;
        let stale = call(
            &app,
            Method::POST,
            "/api/auth/context",
            Some(&ticket),
            json!({"membershipId":descendant.id}),
        )
        .await?;
        expect(
            &stale,
            StatusCode::UNAUTHORIZED,
            "ticket with a stale exact identity generation",
        )?;
        let ticket =
            crate::routes::auth::fixture_credential_ticket(&db.pool, actor, SECRET).await?;
        let current = call(
            &app,
            Method::POST,
            "/api/auth/context",
            Some(&ticket),
            json!({"membershipId":descendant.id}),
        )
        .await?;
        context_proof(
            &db.pool,
            &current,
            Member {
                version: descendant.version + 1,
                ..descendant
            },
        )
        .await?;
        Ok(())
    }
    .await;
    db.finish(result).await
}

async fn grant_member_management(pool: &PgPool, manager: Member) -> Result<()> {
    let app = uuid7::new_uuid();
    let menu = uuid7::new_uuid();
    let role = uuid7::new_uuid();
    sqlx::query("INSERT INTO applications(id,name,code,base_path) VALUES($1,'Owned member management','system','/system')")
        .bind(app).execute(pool).await?;
    sqlx::query("INSERT INTO menus(id,application_id,type,title,permission,visible) VALUES($1,$2,'BUTTON','Owned member management','system:user',false)")
        .bind(menu).bind(app).execute(pool).await?;
    sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
        .bind(manager.tenant)
        .bind(app)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)",
    )
    .bind(manager.org)
    .bind(app)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
        .bind(manager.tenant)
        .bind(menu)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
        .bind(manager.org)
        .bind(menu)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO roles(id,tenant_id,organization_id,name,code) VALUES($1,$2,$3,'Owned member manager','phase05-member-manager')")
        .bind(role).bind(manager.tenant).bind(manager.org).execute(pool).await?;
    sqlx::query("INSERT INTO role_menus(role_id,menu_id) VALUES($1,$2)")
        .bind(role)
        .bind(menu)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3)")
        .bind(manager.user)
        .bind(role)
        .bind(manager.org)
        .execute(pool)
        .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE05_TEST_DATABASE_URL and disposable database creation"]
async fn phase05_capabilities_reflect_server_gates_and_customer_console_access_stays_closed(
) -> Result<()> {
    let db = Database::create().await?;
    let result: Result<()> = async {
        let (_, root) = tenant(&db.pool).await?;
        let manager = user(&db.pool, "phase05-capability-manager", 0, false).await?;
        let manager = member(&db.pool, manager, root, "EMPLOYEE", "ENABLED", 1).await?;
        let plain = user(&db.pool, "phase05-capability-plain", 0, false).await?;
        let plain = member(&db.pool, plain, root, "EMPLOYEE", "ENABLED", 1).await?;
        let customer = user(&db.pool, "phase05-capability-customer", 0, true).await?;
        let customer = member(&db.pool, customer, root, "CUSTOMER", "ENABLED", 1).await?;
        let target = user(&db.pool, "phase05-capability-write-target", 0, false).await?;
        grant_member_management(&db.pool, manager).await?;
        let manager_token = session(&db.pool, manager).await?;
        let plain_token = session(&db.pool, plain).await?;
        // This customer session is active under a customer-allowing source, so
        // employee-only API rejection cannot be explained by a broken fixture.
        let customer_token = session(&db.pool, customer).await?;
        for (lifecycle, customer_enabled, dual) in [
            (false, false, false),
            (true, false, false),
            (true, true, false),
            (true, true, true),
        ] {
            let app = app(&db.pool, lifecycle, customer_enabled, dual)?;
            let capability = call(&app, Method::GET, "/api/memberships/capabilities",
                Some(&manager_token), Value::Null).await?;
            expect(&capability, StatusCode::OK, "authorized employee capability read")?;
            ensure!(
                capability.body == json!({"lifecycleEnabled":lifecycle,"customerMembershipsEnabled":customer_enabled,"dualMembershipsEnabled":dual}),
                "capability response must exactly match independent server gates"
            );
            let plain_reply = call(&app, Method::GET, "/api/memberships/capabilities",
                Some(&plain_token), Value::Null).await?;
            expect(&plain_reply, StatusCode::FORBIDDEN, "capability read requires system:user")?;
            let customer_reply = call(&app, Method::GET, "/api/memberships/capabilities",
                Some(&customer_token), Value::Null).await?;
            expect(&customer_reply, StatusCode::UNAUTHORIZED, "customer capability access")?;
            if !lifecycle {
                let write = call(&app, Method::POST, "/api/memberships", Some(&manager_token),
                    json!({"userId":target,"organizationId":root,"memberType":"EMPLOYEE",
                        "reason":"Owned Phase 5 closed lifecycle gate verification"})).await?;
                expect(&write, StatusCode::FORBIDDEN, "closed lifecycle gate still rejects writes")?;
                let count: i64 = sqlx::query_scalar("SELECT count(*) FROM memberships WHERE user_id=$1")
                    .bind(target).fetch_one(&db.pool).await?;
                ensure!(count == 0, "closed lifecycle capability read enabled membership writes");
            }
        }
        let app = app(&db.pool, true, true, true)?;
        let login = call(&app, Method::POST, "/api/auth/login", None,
            json!({"username":"phase05-capability-customer","password":PASSWORD})).await?;
        expect(&login, StatusCode::FORBIDDEN, "customer-only legacy login")?;
        let ticket = crate::routes::auth::fixture_credential_ticket(&db.pool, customer.user, SECRET).await?;
        let context = call(&app, Method::POST, "/api/auth/context", Some(&ticket),
            json!({"membershipId":customer.id,"organizationId":root})).await?;
        expect(&context, StatusCode::FORBIDDEN, "customer-only credential context")?;
        let context = call(&app, Method::POST, "/api/auth/context", Some(&customer_token),
            json!({"organizationId":root})).await?;
        expect(&context, StatusCode::UNAUTHORIZED, "customer-only session context")?;
        let profile = call(&app, Method::GET, "/api/auth/profile", Some(&customer_token), Value::Null).await?;
        expect(&profile, StatusCode::UNAUTHORIZED, "customer-only employee profile")?;
        Ok(())
    }.await;
    db.finish(result).await
}
