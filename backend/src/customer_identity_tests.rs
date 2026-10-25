//! Phase 6 customer contracts use real OAuth exchanges and disposable databases.
//! Only IDENTITY_PHASE06_TEST_DATABASE_URL is read; existing databases are never migrated.
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
use crate::util::{hash, jwt, password, uuid7};

const SECRET: &str = "identity-phase06-owned-fixture-signing-key";
const PASSWORD: &str = "Owned-Phase06-Fixture-Password9!";
const CLIENT_SECRET: &str = "owned-phase06-client-secret-not-a-deployment-credential";
const CUSTOMER_PATHS: [&str; 3] = [
    "/api/customer/context",
    "/api/customer/profile",
    "/api/customer/applications",
];

struct Database {
    pool: PgPool,
    admin: PgPool,
    name: String,
}

impl Database {
    async fn create() -> Result<Self> {
        let url = std::env::var("IDENTITY_PHASE06_TEST_DATABASE_URL")
            .map_err(|_| anyhow!("explicit IDENTITY_PHASE06_TEST_DATABASE_URL is required"))?;
        let options = PgConnectOptions::from_str(&url)
            .map_err(|_| anyhow!("invalid isolated Phase 6 database configuration"))?;
        ensure!(
            options.get_database() == Some("postgres"),
            "Phase 6 fixture administration must explicitly select postgres"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("isolated Phase 6 administration connection failed"))?;
        let name = format!("identity_phase06_{}", uuid7::new_uuid().simple());
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
                return Err(anyhow!("owned Phase 6 database connection failed"));
            }
        };
        let database = Self { pool, admin, name };
        if let Err(error) = sqlx::migrate!("./migrations").run(&database.pool).await {
            let _ = database.finish(Ok(())).await;
            return Err(error.into());
        }
        Ok(database)
    }

    async fn finish(self, result: Result<()>) -> Result<()> {
        ensure!(
            self.name.starts_with("identity_phase06_")
                && self.name.len() <= 63
                && self
                    .name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_'),
            "refusing cleanup outside an owned Phase 6 database"
        );
        let closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let removed = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await;
        self.admin.close().await;
        eprintln!(
            "identity_phase06_cleanup database={} assertions_ok={} database_removed={} pool_closed={closed}",
            self.name,
            result.is_ok(),
            removed.is_ok()
        );
        result?;
        removed?;
        ensure!(closed, "owned Phase 6 pool did not close");
        Ok(())
    }
}

fn app(pool: &PgPool, registration: Option<(Uuid, Uuid)>) -> Result<Router> {
    let mut values = HashMap::from([
        ("APP_ENV".into(), "test".into()),
        (
            "DATABASE_URL".into(),
            "postgres://unused/identity-phase06".into(),
        ),
        ("JWT_SECRET".into(), SECRET.into()),
    ]);
    if let Some((client, application)) = registration {
        values.insert("CUSTOMER_PROFILE_CLIENT_REF".into(), client.to_string());
        values.insert(
            "CUSTOMER_PROFILE_APPLICATION_ID".into(),
            application.to_string(),
        );
    }
    let state = AppState::new(pool.clone(), Arc::new(Config::from_values(&values)?));
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

async fn request(
    app: &Router,
    method: Method,
    path: &str,
    token: Option<&str>,
    content_type: &str,
    body: String,
) -> Result<Reply> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, content_type)
        .header("x-request-id", uuid7::new_uuid().to_string())
        .extension(ConnectInfo("192.0.2.36:30036".parse::<SocketAddr>()?));
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = tokio::time::timeout(
        Duration::from_secs(25),
        app.clone().oneshot(request.body(Body::from(body))?),
    )
    .await
    .map_err(|_| anyhow!("owned Phase 6 HTTP request timed out: {path}"))??;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok(Reply {
        status,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    })
}

async fn call(
    app: &Router,
    method: Method,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> Result<Reply> {
    request(
        app,
        method,
        path,
        token,
        "application/json",
        body.to_string(),
    )
    .await
}

async fn form(app: &Router, fields: &[(&str, String)]) -> Result<Reply> {
    let encoded = fields
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                percent_encoding::utf8_percent_encode(key, percent_encoding::NON_ALPHANUMERIC),
                percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC)
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    request(
        app,
        Method::POST,
        "/oauth/token",
        None,
        "application/x-www-form-urlencoded",
        encoded,
    )
    .await
}

fn expect(reply: &Reply, expected: StatusCode, operation: &str) -> Result<()> {
    // Successful token-bearing bodies never enter evidence or failure output.
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

fn exact_keys(value: &Value, expected: &[&str], label: &str) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("{label} is not an object"))?;
    let actual: BTreeSet<&str> = object.keys().map(String::as_str).collect();
    ensure!(
        actual == expected.iter().copied().collect(),
        "{label} changed its exact public fields"
    );
    Ok(())
}

#[derive(Clone, Copy)]
struct Member {
    id: Uuid,
    user: Uuid,
    tenant: Uuid,
    org: Uuid,
    version: i64,
    account_version: i64,
    kind: &'static str,
}

struct Client {
    id: Uuid,
    public_id: String,
    application: Uuid,
}

async fn member(pool: &PgPool, user: Uuid, org: Uuid, kind: &'static str) -> Result<Member> {
    let id = uuid7::new_uuid();
    let (tenant, account_version): (Uuid, i64) = sqlx::query_as(
        "SELECT o.tenant_id,u.session_version FROM organizations o CROSS JOIN users u WHERE o.id=$1 AND u.id=$2",
    ).bind(org).bind(user).fetch_one(pool).await?;
    sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,identity_version) VALUES($1,$2,$3,$4,5)")
        .bind(id).bind(user).bind(org).bind(kind).execute(pool).await?;
    Ok(Member {
        id,
        user,
        tenant,
        org,
        version: 5,
        account_version,
        kind,
    })
}

async fn organization(pool: &PgPool, tenant: Uuid, parent: Option<Uuid>) -> Result<Uuid> {
    let id = uuid7::new_uuid();
    sqlx::query("INSERT INTO organizations(id,tenant_id,parent_id,name) VALUES($1,$2,$3,'Owned customer organization')")
        .bind(id).bind(tenant).bind(parent).execute(pool).await?;
    Ok(id)
}

async fn application(pool: &PgPool, tenant: Uuid, org: Uuid, kinds: &[&str]) -> Result<Uuid> {
    let id = uuid7::new_uuid();
    sqlx::query("INSERT INTO applications(id,name,code,base_path,status,allowed_member_types) VALUES($1,'Owned customer profile',$2,'/owned/customer','ENABLED',$3)")
        .bind(id).bind(format!("phase06-{}", id.simple())).bind(kinds).execute(pool).await?;
    sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
        .bind(tenant)
        .bind(id)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)",
    )
    .bind(org)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(id)
}

async fn client(pool: &PgPool, application: Uuid, kinds: &[&str], machine: bool) -> Result<Client> {
    let id = uuid7::new_uuid();
    let public_id = format!("owned-phase06-{}", id.simple());
    sqlx::query("INSERT INTO oauth_clients(id,client_id,client_name,client_type,client_secret_hash,grant_types,scopes,subject_mode,allowed_member_types,application_id,status) VALUES($1,$2,'Owned customer profile client','CONFIDENTIAL',$3,$4,'profile,other',$5,$6,$7,'ENABLED')")
        .bind(id).bind(&public_id).bind(hash::sha256_hex(CLIENT_SECRET))
        .bind(if machine { "client_credentials" } else { "password,refresh_token" })
        .bind(if machine { "MACHINE" } else { "USER" }).bind(kinds).bind(application).execute(pool).await?;
    Ok(Client {
        id,
        public_id,
        application,
    })
}

struct Harness {
    app: Router,
    customer: Member,
    username: String,
    client: Client,
}

impl Harness {
    async fn create(pool: &PgPool, password_hash: &str, kinds: &[&str]) -> Result<Self> {
        let tenant = uuid7::new_uuid();
        sqlx::query("INSERT INTO tenants(id,name,code) VALUES($1,'Owned customer tenant',$2)")
            .bind(tenant)
            .bind(format!("phase06-{}", tenant.simple()))
            .execute(pool)
            .await?;
        let org = organization(pool, tenant, None).await?;
        let user = uuid7::new_uuid();
        let username = format!("phase06-{}", user.simple());
        // A customer never inherits this account's global administrator flag.
        sqlx::query("INSERT INTO users(id,username,password_hash,session_version,is_super_admin,real_name,avatar,email,phone) VALUES($1,$2,$3,3,true,'Owned customer','/owned-phase06-avatar.png','private@example.invalid','+10000000000')")
            .bind(user).bind(&username).bind(password_hash).execute(pool).await?;
        let customer = member(pool, user, org, "CUSTOMER").await?;
        let application = application(pool, tenant, org, kinds).await?;
        let client = client(pool, application, kinds, false).await?;
        let app = app(pool, Some((client.id, application)))?;
        Ok(Self {
            app,
            customer,
            username,
            client,
        })
    }
}

struct Token {
    access: String,
    refresh: String,
}

async fn password_exchange(
    app: &Router,
    client: &Client,
    username: &str,
    membership: Option<Uuid>,
    scope: &str,
) -> Result<Reply> {
    let mut fields = vec![
        ("grant_type", "password".into()),
        ("client_id", client.public_id.clone()),
        ("client_secret", CLIENT_SECRET.into()),
        ("username", username.into()),
        ("password", PASSWORD.into()),
        ("scope", scope.into()),
    ];
    if let Some(membership) = membership {
        fields.push(("membership_id", membership.to_string()));
    }
    form(app, &fields).await
}

async fn accepted_token(
    pool: &PgPool,
    reply: &Reply,
    member: Member,
    client: &Client,
) -> Result<Token> {
    expect(
        reply,
        StatusCode::OK,
        "real customer OAuth password or refresh exchange",
    )?;
    let access = reply.body["access_token"]
        .as_str()
        .ok_or_else(|| anyhow!("OAuth access token missing"))?
        .to_string();
    let refresh = reply.body["refresh_token"]
        .as_str()
        .ok_or_else(|| anyhow!("OAuth refresh token missing"))?
        .to_string();
    let claims = jwt::verify(&access, SECRET)?;
    ensure!(
        claims.typ == "access" && claims.sub == member.user.to_string(),
        "OAuth subject differs"
    );
    ensure!(
        claims.cid.as_deref() == Some(client.public_id.as_str()),
        "OAuth client differs"
    );
    ensure!(
        claims.tenant.as_deref() == Some(member.tenant.to_string().as_str()),
        "OAuth tenant differs"
    );
    ensure!(
        claims.org.as_deref() == Some(member.org.to_string().as_str()),
        "OAuth organization differs"
    );
    let actual: (String,String,Uuid,Uuid,Uuid,Uuid,String,i64,i64,Uuid,i64,i64) = sqlx::query_as(
        "SELECT token_use,token_type,user_id,tenant_id,organization_id,membership_id,member_type,identity_version,account_version,application_id,client_policy_version,application_policy_version FROM oauth_tokens WHERE token_hash=$1 AND NOT revoked",
    ).bind(hash::sha256_hex(&access)).fetch_one(pool).await?;
    let policies: (i64,i64) = sqlx::query_as("SELECT c.access_policy_version,a.access_policy_version FROM oauth_clients c JOIN applications a ON a.id=c.application_id WHERE c.id=$1")
        .bind(client.id).fetch_one(pool).await?;
    ensure!(
        actual
            == (
                "oauth".into(),
                "access".into(),
                member.user,
                member.tenant,
                member.org,
                member.id,
                member.kind.into(),
                member.version,
                member.account_version,
                client.application,
                policies.0,
                policies.1
            ),
        "real exchange did not persist its exact ordinary OAuth identity proof"
    );
    Ok(Token { access, refresh })
}

async fn issue(pool: &PgPool, harness: &Harness, member: Member) -> Result<Token> {
    let reply = password_exchange(
        &harness.app,
        &harness.client,
        &harness.username,
        Some(member.id),
        "profile",
    )
    .await?;
    accepted_token(pool, &reply, member, &harness.client).await
}

async fn context_dto(pool: &PgPool, body: &Value, member: Member, application: Uuid) -> Result<()> {
    exact_keys(
        body,
        &[
            "contractVersion",
            "purpose",
            "userId",
            "membershipId",
            "memberType",
            "identityVersion",
            "accountVersion",
            "tenant",
            "organization",
            "application",
        ],
        "customer context",
    )?;
    ensure!(
        body["contractVersion"] == "customer-profile/1" && body["purpose"] == "CUSTOMER_PROFILE",
        "independent customer contract changed"
    );
    ensure!(
        body["userId"] == json!(member.user) && body["membershipId"] == json!(member.id),
        "customer context selected another subject or member"
    );
    ensure!(
        body["memberType"] == "CUSTOMER",
        "customer context merged an employee identity"
    );
    ensure!(
        body["identityVersion"].as_str() == Some(member.version.to_string().as_str()),
        "customer generation must be its exact decimal string"
    );
    ensure!(
        body["accountVersion"].as_str() == Some(member.account_version.to_string().as_str()),
        "customer account generation must be its exact decimal string"
    );
    for (value, id, label) in [
        (&body["tenant"], member.tenant, "tenant"),
        (&body["organization"], member.org, "organization"),
    ] {
        exact_keys(value, &["id", "name"], label)?;
        ensure!(
            value["id"] == json!(id) && value["name"].as_str().is_some_and(|name| !name.is_empty()),
            "customer {label} differs"
        );
    }
    exact_keys(
        &body["application"],
        &["id", "code", "name", "entryPath"],
        "customer application",
    )?;
    let (code, name): (String, String) =
        sqlx::query_as("SELECT code,name FROM applications WHERE id=$1")
            .bind(application)
            .fetch_one(pool)
            .await?;
    ensure!(
        body["application"]
            == json!({"id":application,"code":code,"name":name,"entryPath":"/api/customer/profile"}),
        "customer application is not the exact registered purpose"
    );
    for id in [
        member.id,
        member.user,
        member.tenant,
        member.org,
        application,
    ] {
        ensure!(
            uuid7::is_uuid_v7(id),
            "customer DTO identity is not UUID v7"
        );
    }
    Ok(())
}

async fn accessible(pool: &PgPool, harness: &Harness, token: &str, member: Member) -> Result<()> {
    let context = call(
        &harness.app,
        Method::GET,
        CUSTOMER_PATHS[0],
        Some(token),
        Value::Null,
    )
    .await?;
    expect(&context, StatusCode::OK, "customer context")?;
    context_dto(pool, &context.body, member, harness.client.application).await?;
    let profile = call(
        &harness.app,
        Method::GET,
        CUSTOMER_PATHS[1],
        Some(token),
        Value::Null,
    )
    .await?;
    expect(&profile, StatusCode::OK, "customer profile")?;
    exact_keys(&profile.body, &["context", "account"], "customer profile")?;
    ensure!(
        profile.body["context"] == context.body,
        "profile selected another context"
    );
    exact_keys(
        &profile.body["account"],
        &["id", "username", "realName", "avatar"],
        "minimal customer account",
    )?;
    ensure!(
        profile.body["account"]
            == json!({"id":member.user,"username":harness.username,"realName":"Owned customer","avatar":"/owned-phase06-avatar.png"}),
        "profile exposed or substituted account data"
    );
    let applications = call(
        &harness.app,
        Method::GET,
        CUSTOMER_PATHS[2],
        Some(token),
        Value::Null,
    )
    .await?;
    expect(&applications, StatusCode::OK, "customer applications")?;
    exact_keys(
        &applications.body,
        &["contractVersion", "context", "applications"],
        "customer applications",
    )?;
    ensure!(
        applications.body["contractVersion"] == "customer-profile/1"
            && applications.body["context"] == context.body,
        "application list selected another context"
    );
    ensure!(
        applications.body["applications"] == json!([context.body["application"]]),
        "customer application list broadened beyond the registered mini application"
    );
    Ok(())
}

async fn denied(app: &Router, token: &str, status: StatusCode, label: &str) -> Result<()> {
    for path in CUSTOMER_PATHS {
        let reply = call(app, Method::GET, path, Some(token), Value::Null).await?;
        expect(&reply, status, label)?;
    }
    Ok(())
}

async fn allow_owned_dual_fixture(pool: &PgPool) -> Result<()> {
    // This simulation of the separately released contraction touches only an owned fixture.
    sqlx::query(
        "ALTER TABLE memberships DROP CONSTRAINT IF EXISTS memberships_user_id_organization_id_key",
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE06_TEST_DATABASE_URL and disposable database creation"]
async fn phase06_real_customer_oauth_reads_only_its_registered_minimal_profile() -> Result<()> {
    let db = Database::create().await?;
    let result: Result<()> = async {
        let password_hash = password::hash_password(PASSWORD)?;
        let harness = Harness::create(&db.pool, &password_hash, &["CUSTOMER"]).await?;
        let foreign = Harness::create(&db.pool, &password_hash, &["CUSTOMER"]).await?;
        application(
            &db.pool,
            harness.customer.tenant,
            harness.customer.org,
            &["EMPLOYEE"],
        )
        .await?;
        application(
            &db.pool,
            harness.customer.tenant,
            harness.customer.org,
            &["CUSTOMER"],
        )
        .await?;
        let token = issue(&db.pool, &harness, harness.customer).await?;
        accessible(&db.pool, &harness, &token.access, harness.customer).await?;
        let another_subject = password_exchange(
            &harness.app,
            &harness.client,
            &harness.username,
            Some(foreign.customer.id),
            "profile",
        )
        .await?;
        expect(
            &another_subject,
            StatusCode::BAD_REQUEST,
            "password proof cannot select another user's membership",
        )?;
        for path in CUSTOMER_PATHS {
            for (key, value) in [
                ("userId", foreign.customer.user.to_string()),
                ("membershipId", foreign.customer.id.to_string()),
                ("organizationId", foreign.customer.org.to_string()),
                ("tenantId", foreign.customer.tenant.to_string()),
                ("userinfo", "true".into()),
            ] {
                let reply = call(
                    &harness.app,
                    Method::GET,
                    &format!("{path}?{key}={value}"),
                    Some(&token.access),
                    Value::Null,
                )
                .await?;
                expect(
                    &reply,
                    StatusCode::BAD_REQUEST,
                    "customer API forbids query-based subject or context selection",
                )?;
            }
        }
        let refreshed = form(
            &harness.app,
            &[
                ("grant_type", "refresh_token".into()),
                ("client_id", harness.client.public_id.clone()),
                ("client_secret", CLIENT_SECRET.into()),
                ("refresh_token", token.refresh),
            ],
        )
        .await?;
        let refreshed =
            accepted_token(&db.pool, &refreshed, harness.customer, &harness.client).await?;
        accessible(&db.pool, &harness, &refreshed.access, harness.customer).await?;
        for path in ["/api/auth/profile", "/api/auth/permissions", "/api/users"] {
            let reply = call(
                &harness.app,
                Method::GET,
                path,
                Some(&refreshed.access),
                Value::Null,
            )
            .await?;
            expect(
                &reply,
                StatusCode::UNAUTHORIZED,
                "customer OAuth must not enter employee APIs",
            )?;
        }
        let legacy = call(
            &harness.app,
            Method::POST,
            "/api/oauth/session-token",
            Some(&refreshed.access),
            json!({"appCode":"system"}),
        )
        .await?;
        expect(
            &legacy,
            StatusCode::UNAUTHORIZED,
            "legacy application token issuance stays employee-only",
        )?;
        Ok(())
    }
    .await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE06_TEST_DATABASE_URL and disposable database creation"]
async fn phase06_dual_identity_requires_exact_selection_and_keeps_credential_kinds_separate(
) -> Result<()> {
    let db = Database::create().await?;
    let result: Result<()> = async {
        allow_owned_dual_fixture(&db.pool).await?;
        let password_hash = password::hash_password(PASSWORD)?;
        let harness = Harness::create(&db.pool, &password_hash, &["EMPLOYEE","CUSTOMER"]).await?;
        let employee = member(&db.pool, harness.customer.user, harness.customer.org, "EMPLOYEE").await?;
        let role = uuid7::new_uuid();
        sqlx::query("INSERT INTO roles(id,tenant_id,organization_id,name) VALUES($1,$2,$3,'Owned employee-only role')")
            .bind(role).bind(employee.tenant).bind(employee.org).execute(&db.pool).await?;
        sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3)")
            .bind(employee.user).bind(role).bind(employee.org).execute(&db.pool).await?;
        let before: String = sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(m) ORDER BY id)::text FROM memberships m WHERE user_id=$1")
            .bind(employee.user).fetch_one(&db.pool).await?;
        let ambiguous = password_exchange(&harness.app, &harness.client, &harness.username, None, "profile").await?;
        expect(&ambiguous, StatusCode::BAD_REQUEST, "dual client must not select its first membership")?;
        ensure!(ambiguous.body["error"] == "invalid_grant", "ambiguous identity must fail OAuth authentication");
        let employee_token = issue(&db.pool, &harness, employee).await?;
        let customer_token = issue(&db.pool, &harness, harness.customer).await?;
        denied(&harness.app, &employee_token.access, StatusCode::UNAUTHORIZED, "employee OAuth token is not a customer credential").await?;
        accessible(&db.pool, &harness, &customer_token.access, harness.customer).await?;
        let employee_info = call(&harness.app, Method::GET, "/oauth/userinfo", Some(&employee_token.access), Value::Null).await?;
        expect(&employee_info, StatusCode::OK, "employee OAuth token remains independently valid")?;
        ensure!(employee_info.body["roles"] == json!(["Owned employee-only role"]), "employee role fixture lacks real authority");
        let ticket = crate::routes::auth::fixture_credential_ticket(&db.pool, employee.user, SECRET).await?;
        let context = call(&harness.app, Method::POST, "/api/auth/context", Some(&ticket), json!({"membershipId":employee.id})).await?;
        expect(&context, StatusCode::OK, "real employee context issuance")?;
        let session = context.body["accessToken"].as_str().ok_or_else(|| anyhow!("employee context lacks token"))?;
        denied(&harness.app, session, StatusCode::UNAUTHORIZED, "platform session is not ordinary customer OAuth").await?;
        let machine = client(&db.pool, harness.client.application, &[], true).await?;
        let machine_reply = form(&harness.app, &[
            ("grant_type", "client_credentials".into()),("client_id", machine.public_id.clone()),
            ("client_secret", CLIENT_SECRET.into()),("scope", "profile".into()),
        ]).await?;
        expect(&machine_reply, StatusCode::OK, "real machine OAuth issuance")?;
        let machine_access = machine_reply.body["access_token"].as_str().ok_or_else(|| anyhow!("machine token missing"))?;
        let machine_registration = app(&db.pool, Some((machine.id, machine.application)))?;
        denied(&machine_registration, machine_access, StatusCode::UNAUTHORIZED, "registered machine client cannot claim customer membership").await?;
        let app_reply = call(&harness.app, Method::POST, "/api/oauth/session-token", Some(session),
            json!({"appCode":context_application_code(&db.pool,harness.client.application).await?,"applicationServiceClientRef":machine.public_id})).await?;
        expect(&app_reply, StatusCode::CREATED, "real legacy employee application token issuance")?;
        let app_access = app_reply.body["accessToken"].as_str().ok_or_else(|| anyhow!("application token missing"))?;
        denied(&harness.app, app_access, StatusCode::UNAUTHORIZED, "application token is not ordinary customer OAuth").await?;
        let after: String = sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(m) ORDER BY id)::text FROM memberships m WHERE user_id=$1")
            .bind(employee.user).fetch_one(&db.pool).await?;
        ensure!(before == after, "identity selection or customer reads created or changed membership rows");
        Ok(())
    }.await;
    db.finish(result).await
}

async fn context_application_code(pool: &PgPool, application: Uuid) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT code FROM applications WHERE id=$1")
            .bind(application)
            .fetch_one(pool)
            .await?,
    )
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE06_TEST_DATABASE_URL and disposable database creation"]
async fn phase06_customer_entry_requires_exact_server_registration_scope_and_type_intersection(
) -> Result<()> {
    let db = Database::create().await?;
    let result: Result<()> = async {
        allow_owned_dual_fixture(&db.pool).await?;
        let password_hash = password::hash_password(PASSWORD)?;
        let harness = Harness::create(&db.pool, &password_hash, &["CUSTOMER"]).await?;
        let token = issue(&db.pool, &harness, harness.customer).await?;
        let absent = app(&db.pool, None)?;
        denied(
            &absent,
            &token.access,
            StatusCode::FORBIDDEN,
            "unregistered server must not infer a customer purpose",
        )
        .await?;
        let another_client =
            client(&db.pool, harness.client.application, &["CUSTOMER"], false).await?;
        let wrong_registration = app(
            &db.pool,
            Some((another_client.id, harness.client.application)),
        )?;
        denied(
            &wrong_registration,
            &token.access,
            StatusCode::UNAUTHORIZED,
            "client registration uses exact database ID, not an arbitrary related client",
        )
        .await?;
        let another_app = application(
            &db.pool,
            harness.customer.tenant,
            harness.customer.org,
            &["CUSTOMER"],
        )
        .await?;
        let wrong_app = app(&db.pool, Some((harness.client.id, another_app)))?;
        denied(
            &wrong_app,
            &token.access,
            StatusCode::UNAUTHORIZED,
            "registered client must be bound to the exact registered application",
        )
        .await?;
        let foreign_reply = password_exchange(
            &harness.app,
            &another_client,
            &harness.username,
            Some(harness.customer.id),
            "profile",
        )
        .await?;
        let foreign_token =
            accepted_token(&db.pool, &foreign_reply, harness.customer, &another_client).await?;
        denied(
            &harness.app,
            &foreign_token.access,
            StatusCode::UNAUTHORIZED,
            "another real human client cannot enter the registered customer API",
        )
        .await?;
        let other_app_client = client(&db.pool, another_app, &["CUSTOMER"], false).await?;
        let other_reply = password_exchange(
            &harness.app,
            &other_app_client,
            &harness.username,
            Some(harness.customer.id),
            "profile",
        )
        .await?;
        let other_token =
            accepted_token(&db.pool, &other_reply, harness.customer, &other_app_client).await?;
        denied(
            &harness.app,
            &other_token.access,
            StatusCode::UNAUTHORIZED,
            "another admitted customer application does not inherit this purpose",
        )
        .await?;
        let no_scope = password_exchange(
            &harness.app,
            &harness.client,
            &harness.username,
            Some(harness.customer.id),
            "other",
        )
        .await?;
        let no_scope =
            accepted_token(&db.pool, &no_scope, harness.customer, &harness.client).await?;
        denied(
            &harness.app,
            &no_scope.access,
            StatusCode::UNAUTHORIZED,
            "scope without profile grants no customer API access",
        )
        .await?;

        let employee = member(
            &db.pool,
            harness.customer.user,
            harness.customer.org,
            "EMPLOYEE",
        )
        .await?;
        let employee_app =
            application(&db.pool, employee.tenant, employee.org, &["EMPLOYEE"]).await?;
        let customer_client_employee_app =
            client(&db.pool, employee_app, &["CUSTOMER"], false).await?;
        let first_mismatch = password_exchange(
            &harness.app,
            &customer_client_employee_app,
            &harness.username,
            Some(harness.customer.id),
            "profile",
        )
        .await?;
        expect(
            &first_mismatch,
            StatusCode::BAD_REQUEST,
            "customer client cannot widen employee application admission",
        )?;
        ensure!(
            first_mismatch.body["error"] == "invalid_grant",
            "customer client/employee app mismatch returned wrong OAuth error"
        );
        let employee_client_customer_app =
            client(&db.pool, harness.client.application, &["EMPLOYEE"], false).await?;
        let second_mismatch = password_exchange(
            &harness.app,
            &employee_client_customer_app,
            &harness.username,
            Some(employee.id),
            "profile",
        )
        .await?;
        expect(
            &second_mismatch,
            StatusCode::BAD_REQUEST,
            "employee client cannot widen customer application admission",
        )?;
        ensure!(
            second_mismatch.body["error"] == "invalid_grant",
            "employee client/customer app mismatch returned wrong OAuth error"
        );
        let unsupported = password_exchange(
            &harness.app,
            &harness.client,
            &harness.username,
            Some(employee.id),
            "profile",
        )
        .await?;
        expect(
            &unsupported,
            StatusCode::BAD_REQUEST,
            "customer-only client cannot select an employee row",
        )?;
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM oauth_tokens WHERE client_id=ANY($1)")
                .bind(vec![
                    customer_client_employee_app.public_id,
                    employee_client_customer_app.public_id,
                ])
                .fetch_one(&db.pool)
                .await?;
        ensure!(
            count == 0,
            "type intersection failure persisted usable credentials"
        );
        Ok(())
    }
    .await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE06_TEST_DATABASE_URL and disposable database creation"]
async fn phase06_customer_reads_revalidate_every_live_authority_and_policy_generation() -> Result<()>
{
    let db = Database::create().await?;
    let result: Result<()> = async {
        let password_hash = password::hash_password(PASSWORD)?;
        for case in 0..18 {
            let harness = Harness::create(&db.pool, &password_hash, &["CUSTOMER"]).await?;
            let token = issue(&db.pool, &harness, harness.customer).await?;
            let before = call(&harness.app, Method::GET, CUSTOMER_PATHS[0], Some(&token.access), Value::Null).await?;
            expect(&before, StatusCode::OK, "customer authority before revocation")?;
            let (label, statement, id) = match case {
                0 => ("membership disabled", "UPDATE memberships SET status='DISABLED',identity_version=identity_version+1 WHERE id=$1", harness.customer.id),
                1 => ("membership deleted", "DELETE FROM memberships WHERE id=$1", harness.customer.id),
                2 => ("membership generation changed", "UPDATE memberships SET identity_version=identity_version+1 WHERE id=$1", harness.customer.id),
                3 => ("account disabled", "UPDATE users SET status='DISABLED' WHERE id=$1", harness.customer.user),
                4 => ("account generation changed", "UPDATE users SET session_version=session_version+1 WHERE id=$1", harness.customer.user),
                5 => ("client disabled", "UPDATE oauth_clients SET status='DISABLED' WHERE id=$1", harness.client.id),
                6 => ("client policy generation changed", "UPDATE oauth_clients SET access_policy_version=access_policy_version+1 WHERE id=$1", harness.client.id),
                7 => ("application disabled", "UPDATE applications SET status='DISABLED' WHERE id=$1", harness.client.application),
                8 => ("application policy generation changed", "UPDATE applications SET access_policy_version=access_policy_version+1 WHERE id=$1", harness.client.application),
                9 => ("tenant disabled", "UPDATE tenants SET status='DISABLED' WHERE id=$1", harness.customer.tenant),
                10 => ("organization disabled", "UPDATE organizations SET status='DISABLED' WHERE id=$1", harness.customer.org),
                11 => ("tenant application grant removed", "DELETE FROM tenant_applications WHERE application_id=$1", harness.client.application),
                12 => ("organization application grant removed", "DELETE FROM organization_applications WHERE application_id=$1", harness.client.application),
                13 => ("client member admission tightened", "UPDATE oauth_clients SET allowed_member_types=ARRAY['EMPLOYEE']::text[] WHERE id=$1", harness.client.id),
                14 => ("application member admission tightened", "UPDATE applications SET allowed_member_types=ARRAY['EMPLOYEE']::text[] WHERE id=$1", harness.client.application),
                15 => ("stored credentials expired", "UPDATE oauth_tokens SET expires_at=clock_timestamp()-interval '1 second' WHERE membership_id=$1", harness.customer.id),
                16 => ("stored credentials revoked", "UPDATE oauth_tokens SET revoked=true WHERE membership_id=$1", harness.customer.id),
                _ => ("profile scope removed from current registration", "UPDATE oauth_clients SET scopes='other' WHERE id=$1", harness.client.id),
            };
            sqlx::query(statement).bind(id).execute(&db.pool).await?;
            denied(&harness.app, &token.access, StatusCode::UNAUTHORIZED, label).await?;
            let refresh = form(&harness.app, &[
                ("grant_type", "refresh_token".into()),("client_id", harness.client.public_id.clone()),
                ("client_secret", CLIENT_SECRET.into()),("refresh_token", token.refresh),
            ]).await?;
            let expected = if matches!(case, 5 | 7) { StatusCode::UNAUTHORIZED } else { StatusCode::BAD_REQUEST };
            expect(&refresh, expected, "revoked customer identity cannot refresh into a new generation")?;
        }
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE06_TEST_DATABASE_URL and disposable database creation"]
async fn phase06_local_revocation_keeps_other_identity_and_other_organization_independent(
) -> Result<()> {
    let db = Database::create().await?;
    let result: Result<()> = async {
        allow_owned_dual_fixture(&db.pool).await?;
        let password_hash = password::hash_password(PASSWORD)?;
        let harness = Harness::create(&db.pool, &password_hash, &["EMPLOYEE","CUSTOMER"]).await?;
        let employee = member(&db.pool, harness.customer.user, harness.customer.org, "EMPLOYEE").await?;
        let other_org = organization(&db.pool, harness.customer.tenant, Some(harness.customer.org)).await?;
        sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)")
            .bind(other_org).bind(harness.client.application).execute(&db.pool).await?;
        let other_customer = member(&db.pool, harness.customer.user, other_org, "CUSTOMER").await?;
        let employee_token = issue(&db.pool, &harness, employee).await?;
        let customer_token = issue(&db.pool, &harness, harness.customer).await?;
        let other_token = issue(&db.pool, &harness, other_customer).await?;
        sqlx::query("UPDATE memberships SET status='DISABLED',identity_version=identity_version+1 WHERE id=$1")
            .bind(harness.customer.id).execute(&db.pool).await?;
        denied(&harness.app, &customer_token.access, StatusCode::UNAUTHORIZED, "local customer revocation").await?;
        accessible(&db.pool, &harness, &other_token.access, other_customer).await?;
        let employee_info = call(&harness.app, Method::GET, "/oauth/userinfo", Some(&employee_token.access), Value::Null).await?;
        expect(&employee_info, StatusCode::OK, "customer revocation must retain employee credentials")?;
        sqlx::query("UPDATE memberships SET status='ENABLED',identity_version=identity_version+1 WHERE id=$1")
            .bind(harness.customer.id).execute(&db.pool).await?;
        let current_customer = Member { version: harness.customer.version + 2, ..harness.customer };
        let fresh_customer = issue(&db.pool, &harness, current_customer).await?;
        denied(&harness.app, &customer_token.access, StatusCode::UNAUTHORIZED, "re-enabled customer must not revive old credentials").await?;
        sqlx::query("UPDATE memberships SET status='DISABLED',identity_version=identity_version+1 WHERE id=$1")
            .bind(employee.id).execute(&db.pool).await?;
        let employee_info = call(&harness.app, Method::GET, "/oauth/userinfo", Some(&employee_token.access), Value::Null).await?;
        expect(&employee_info, StatusCode::UNAUTHORIZED, "local employee revocation")?;
        accessible(&db.pool, &harness, &fresh_customer.access, current_customer).await?;
        accessible(&db.pool, &harness, &other_token.access, other_customer).await?;
        Ok(())
    }.await;
    db.finish(result).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE06_TEST_DATABASE_URL and disposable database creation"]
async fn phase06_customer_authority_unavailable_returns_503_without_cached_profile_access(
) -> Result<()> {
    let db = Database::create().await?;
    let result: Result<()> = async {
        let password_hash = password::hash_password(PASSWORD)?;
        let harness = Harness::create(&db.pool, &password_hash, &["CUSTOMER"]).await?;
        let token = issue(&db.pool, &harness, harness.customer).await?;
        accessible(&db.pool, &harness, &token.access, harness.customer).await?;
        // The independently connected admin pool remains open for owned cleanup.
        // Closing the resource pool deterministically fails every authority read.
        db.pool.close().await;
        denied(
            &harness.app,
            &token.access,
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable authority must not serve a previously valid customer projection",
        )
        .await?;
        Ok(())
    }
    .await;
    db.finish(result).await
}
