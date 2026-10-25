//! Phase 8 SSO contracts use real HTTP handlers and only owned disposable databases.
//! Only IDENTITY_PHASE08_TEST_DATABASE_URL is read; no dotenv or existing database is migrated.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::{middleware, Router};
use serde_json::{json, Value};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::config::Config;
use crate::state::AppState;
use crate::util::{hash, password, uuid7};

const SECRET: &str = "identity-phase08-owned-fixture-signing-key";
const PASSWORD: &str = "Owned-Phase08-Fixture-Password9!";
const NEW_PASSWORD: &str = "Owned-Phase08-Replacement-Password8!";
const CLIENT_SECRET: &str = "phase08-owned-client-fixture-not-a-deployment-secret";
const REDIRECT: &str = "https://phase08.example.invalid/callback";

struct Database {
    pool: PgPool,
    admin: PgPool,
    name: String,
}

impl Database {
    async fn create() -> Result<Self> {
        let raw = std::env::var("IDENTITY_PHASE08_TEST_DATABASE_URL")
            .map_err(|_| anyhow!("explicit IDENTITY_PHASE08_TEST_DATABASE_URL is required"))?;
        let url = reqwest::Url::parse(&raw)
            .map_err(|_| anyhow!("invalid isolated Phase 8 database configuration"))?;
        let port = url
            .port()
            .ok_or_else(|| anyhow!("Phase 8 administration requires an explicit port"))?;
        let loopback = |host: &str| {
            matches!(
                host.trim_matches(&['[', ']'][..]),
                "127.0.0.1" | "localhost" | "::1"
            )
        };
        ensure!(matches!(url.scheme(),"postgres"|"postgresql") &&
                url.host_str().is_some_and(loopback) && port>0 && !url.username().is_empty() &&
                url.password().is_some() && url.fragment().is_none() &&
                url.query_pairs().all(|(key,value)| matches!(key.as_ref(),"sslmode"|"ssl-mode") &&
                    matches!(value.as_ref(),"disable"|"prefer"|"require")),
                "Phase 8 administration requires an explicit loopback host, port and account without redirection");
        let options = PgConnectOptions::from_str(&raw)
            .map_err(|_| anyhow!("invalid isolated Phase 8 database configuration"))?;
        ensure!(
            options.get_database() == Some("postgres")
                && loopback(options.get_host())
                && options.get_port() == port
                && !options.get_username().is_empty(),
            "Phase 8 administration must explicitly select postgres"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("isolated Phase 8 administration connection failed"))?;
        let name = format!("identity_phase08_{}", uuid7::new_uuid().simple());
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
                admin.close().await;
                return Err(anyhow!("owned Phase 8 database connection failed"));
            }
        };
        let database = Self { pool, admin, name };
        if let Err(error) = sqlx::migrate!("./migrations").run(&database.pool).await {
            let _ = database.finish(Ok(())).await;
            return Err(error.into());
        }
        Ok(database)
    }

    async fn finish(self, assertions: Result<()>) -> Result<()> {
        let suffix = self.name.strip_prefix("identity_phase08_");
        ensure!(
            suffix.is_some_and(|value| value.len() == 32
                && value.bytes().all(|ch| ch.is_ascii_hexdigit())
                && Uuid::parse_str(value).is_ok_and(uuid7::is_uuid_v7)),
            "refusing cleanup outside an owned Phase 8 database"
        );
        let closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let removed = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await;
        self.admin.close().await;
        eprintln!(
            "identity_phase08_cleanup database={} assertions_ok={} database_removed={} pool_closed={closed}",
            self.name, assertions.is_ok(), removed.is_ok()
        );
        assertions?;
        removed?;
        ensure!(closed, "owned Phase 8 pool did not close");
        Ok(())
    }
}

fn router(state: AppState) -> Router {
    crate::routes::api_router()
        .nest("/oauth", crate::routes::oauth2::oauth_router())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::observability::observe,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::normalize_client_ip,
        ))
        .with_state(state)
}

struct Reply {
    status: StatusCode,
    body: Value,
    headers: HeaderMap,
}

async fn request(
    app: &Router,
    method: Method,
    path: &str,
    bearer: Option<&str>,
    content_type: &str,
    body: String,
) -> Result<Reply> {
    exchange(
        app,
        method,
        path,
        bearer,
        body,
        &[("content-type", content_type)],
    )
    .await
}

async fn exchange(
    app: &Router,
    method: Method,
    path: &str,
    bearer: Option<&str>,
    body: String,
    headers: &[(&str, &str)],
) -> Result<Reply> {
    static SOURCE: AtomicU16 = AtomicU16::new(1);
    let source = SOURCE.fetch_add(1, Ordering::SeqCst);
    let peer = SocketAddr::from(([192, 0, (source >> 8) as u8, source as u8], 30038));
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("x-request-id", uuid7::new_uuid().to_string())
        .extension(ConnectInfo(peer));
    if let Some(token) = bearer {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = tokio::time::timeout(
        Duration::from_secs(25),
        app.clone().oneshot(request.body(Body::from(body))?),
    )
    .await
    .map_err(|_| anyhow!("owned Phase 8 HTTP request timed out"))??;
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok(Reply {
        status,
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
        headers,
    })
}

async fn call(
    app: &Router,
    method: Method,
    path: &str,
    bearer: Option<&str>,
    body: Value,
) -> Result<Reply> {
    request(
        app,
        method,
        path,
        bearer,
        "application/json",
        body.to_string(),
    )
    .await
}

async fn form(app: &Router, path: &str, fields: &[(&str, String)]) -> Result<Reply> {
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
        path,
        None,
        "application/x-www-form-urlencoded",
        encoded,
    )
    .await
}

fn expect(reply: &Reply, status: StatusCode, label: &str) -> Result<()> {
    // Never include a token-bearing response body, plaintext password or PHC hash.
    ensure!(
        reply.status == status,
        "{label}: expected {status}, received {}",
        reply.status
    );
    Ok(())
}

fn text<'a>(body: &'a Value, name: &str) -> Result<&'a str> {
    body[name]
        .as_str()
        .ok_or_else(|| anyhow!("missing public response field {name}"))
}

#[derive(Clone)]
struct Account {
    id: Uuid,
    username: String,
}

#[derive(Clone, Copy)]
struct Member {
    id: Uuid,
    user: Uuid,
    org: Uuid,
    kind: &'static str,
}

#[derive(Clone)]
struct Client {
    id: Uuid,
    public_id: String,
    application: Option<Uuid>,
}

struct Platform {
    access: String,
    refresh: String,
    ticket: String,
}

struct Token {
    access: String,
    refresh: String,
}

struct Fixture {
    state: AppState,
    admin: Account,
    subject: Account,
    unrelated: Account,
    memberless: Account,
    admin_member: Member,
    employees: [Member; 2],
    customer: Member,
    unrelated_member: Member,
    legacy: Client,
    customer_client: Client,
    service: Client,
    employee_application_code: String,
}

async fn account(
    pool: &PgPool,
    username: &str,
    encoded: &str,
    administrator: bool,
) -> Result<Account> {
    let id = uuid7::new_uuid();
    sqlx::query("INSERT INTO users(id,username,password_hash,is_super_admin,real_name) VALUES($1,$2,$3,$4,'Owned Phase 8 account')")
        .bind(id).bind(username).bind(encoded).bind(administrator).execute(pool).await?;
    Ok(Account {
        id,
        username: username.into(),
    })
}

async fn member(pool: &PgPool, user: Uuid, org: Uuid, kind: &'static str) -> Result<Member> {
    let id = uuid7::new_uuid();
    sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type,status,identity_version) VALUES($1,$2,$3,$4,'ENABLED',1)")
        .bind(id).bind(user).bind(org).bind(kind).execute(pool).await?;
    Ok(Member {
        id,
        user,
        org,
        kind,
    })
}

async fn application(
    pool: &PgPool,
    tenant: Uuid,
    orgs: &[Uuid],
    code: &str,
    kind: &str,
) -> Result<Uuid> {
    let id = uuid7::new_uuid();
    sqlx::query("INSERT INTO applications(id,name,code,allowed_member_types) VALUES($1,'Owned Phase 8 application',$2,$3)")
        .bind(id).bind(code).bind(vec![kind]).execute(pool).await?;
    sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
        .bind(tenant)
        .bind(id)
        .execute(pool)
        .await?;
    for org in orgs {
        sqlx::query(
            "INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)",
        )
        .bind(org)
        .bind(id)
        .execute(pool)
        .await?;
    }
    Ok(id)
}

async fn client(
    pool: &PgPool,
    mode: &str,
    kind: Option<&str>,
    application: Option<Uuid>,
) -> Result<Client> {
    let id = uuid7::new_uuid();
    let public_id = format!("phase08-{}", id.simple());
    let grants = if mode == "MACHINE" {
        "client_credentials"
    } else {
        "password,refresh_token,authorization_code,urn:ietf:params:oauth:grant-type:device_code"
    };
    sqlx::query("INSERT INTO oauth_clients(id,client_id,client_name,client_type,client_secret_hash,subject_mode,allowed_member_types,application_id,grant_types,scopes,redirect_uris) VALUES($1,$2,'Owned Phase 8 client','CONFIDENTIAL',$3,$4,$5,$6,$7,'profile',$8)")
        .bind(id).bind(&public_id).bind(hash::sha256_hex(CLIENT_SECRET)).bind(mode)
        .bind(kind.into_iter().collect::<Vec<_>>()).bind(application).bind(grants).bind(REDIRECT)
        .execute(pool).await?;
    Ok(Client {
        id,
        public_id,
        application,
    })
}

impl Fixture {
    async fn create(pool: &PgPool) -> Result<Self> {
        // Only this test-owned database gets the already-accepted Phase 6 target
        // shape. Historical migrations remain intact and production rollout
        // readiness is not bypassed by this explicitly APP_ENV=test fixture.
        sqlx::query(
            "ALTER TABLE memberships DROP CONSTRAINT memberships_user_id_organization_id_key",
        )
        .execute(pool)
        .await?;
        let tenant = uuid7::new_uuid();
        sqlx::query("INSERT INTO tenants(id,name,code,is_platform) VALUES($1,'Owned Phase 8 tenant','admin-tenant',true)")
            .bind(tenant).execute(pool).await?;
        let root = uuid7::new_uuid();
        sqlx::query(
            "INSERT INTO organizations(id,tenant_id,name) VALUES($1,$2,'Owned Phase 8 root')",
        )
        .bind(root)
        .bind(tenant)
        .execute(pool)
        .await?;
        let employee_org = uuid7::new_uuid();
        let customer_org = uuid7::new_uuid();
        for (org, name) in [
            (employee_org, "Owned Phase 8 second employee"),
            (customer_org, "Owned Phase 8 customer"),
        ] {
            sqlx::query(
                "INSERT INTO organizations(id,tenant_id,parent_id,name) VALUES($1,$2,$3,$4)",
            )
            .bind(org)
            .bind(tenant)
            .bind(root)
            .bind(name)
            .execute(pool)
            .await?;
        }
        let encoded = password::hash_password(PASSWORD)?;
        let admin = account(pool, "phase08-admin", &encoded, true).await?;
        let subject = account(pool, "phase08-subject", &encoded, false).await?;
        let unrelated = account(pool, "phase08-unrelated", &encoded, false).await?;
        let memberless = account(pool, "phase08-memberless", &encoded, false).await?;
        let admin_member = member(pool, admin.id, root, "EMPLOYEE").await?;
        let employees = [
            member(pool, subject.id, root, "EMPLOYEE").await?,
            member(pool, subject.id, employee_org, "EMPLOYEE").await?,
        ];
        let customer = member(pool, subject.id, root, "CUSTOMER").await?;
        let unrelated_member = member(pool, unrelated.id, root, "EMPLOYEE").await?;
        let employee_application_code = "phase08-employee".to_string();
        let employee_app = application(
            pool,
            tenant,
            &[root, employee_org],
            &employee_application_code,
            "EMPLOYEE",
        )
        .await?;
        for (permission, path) in [
            ("system:user", "/system/users"),
            ("system:oauth", "/system/oauth/tokens"),
        ] {
            let menu = uuid7::new_uuid();
            sqlx::query("INSERT INTO menus(id,application_id,type,title,permission,path) VALUES($1,$2,'MENU','Owned Phase 8 administrative permission',$3,$4)")
                .bind(menu).bind(employee_app).bind(permission).bind(path).execute(pool).await?;
            sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) VALUES($1,$2)")
                .bind(tenant)
                .bind(menu)
                .execute(pool)
                .await?;
            sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) VALUES($1,$2)")
                .bind(root)
                .bind(menu)
                .execute(pool)
                .await?;
        }
        let customer_app =
            application(pool, tenant, &[root], "phase08-customer", "CUSTOMER").await?;
        // Exercise the existing OAuth password/refresh protocol through an
        // explicitly human client. A new fixture is never a grandfathered
        // LEGACY_MIXED registration; machine grants use their own client below.
        let legacy = client(pool, "USER", Some("EMPLOYEE"), None).await?;
        let customer_client = client(pool, "USER", Some("CUSTOMER"), Some(customer_app)).await?;
        let service = client(pool, "MACHINE", None, Some(employee_app)).await?;
        let config = Config::from_values(&HashMap::from([
            ("APP_ENV".into(), "test".into()),
            (
                "DATABASE_URL".into(),
                "postgres://unused/identity-phase08".into(),
            ),
            ("JWT_SECRET".into(), SECRET.into()),
            ("MEMBERSHIP_LIFECYCLE_ENABLED".into(), "true".into()),
            ("CUSTOMER_MEMBERSHIPS_ENABLED".into(), "true".into()),
            ("DUAL_MEMBERSHIPS_ENABLED".into(), "true".into()),
            ("SSO_ENABLED".into(), "true".into()),
            ("SSO_ORIGIN".into(), "https://auth.example.invalid".into()),
            (
                "PUBLIC_APP_URL".into(),
                "https://console.example.invalid".into(),
            ),
            (
                "CUSTOMER_PROFILE_CLIENT_REF".into(),
                customer_client.id.to_string(),
            ),
            (
                "CUSTOMER_PROFILE_APPLICATION_ID".into(),
                customer_app.to_string(),
            ),
            ("LOGIN_SOURCE_REQUESTS".into(), "1000".into()),
            ("LOGIN_USER_FAILURES".into(), "1000".into()),
        ]))?;
        Ok(Self {
            state: AppState::new(pool.clone(), Arc::new(config)),
            admin,
            subject,
            unrelated,
            memberless,
            admin_member,
            employees,
            customer,
            unrelated_member,
            legacy,
            customer_client,
            service,
            employee_application_code,
        })
    }

    fn app(&self) -> Router {
        router(self.state.clone())
    }
}

async fn login(app: &Router, account: &Account, supplied: &str) -> Result<Reply> {
    call(
        app,
        Method::POST,
        "/api/auth/login",
        None,
        json!({"username":account.username,"password":supplied}),
    )
    .await
}

async fn platform(
    app: &Router,
    account: &Account,
    membership: Member,
    supplied: &str,
) -> Result<Platform> {
    let authenticated = login(app, account, supplied).await?;
    expect(
        &authenticated,
        StatusCode::OK,
        "real account password authentication",
    )?;
    let ticket = text(&authenticated.body, "ticket")?.to_owned();
    let selected = call(
        app,
        Method::POST,
        "/api/auth/context",
        Some(&ticket),
        json!({"membershipId":membership.id,"organizationId":membership.org}),
    )
    .await?;
    expect(&selected, StatusCode::OK, "real concrete employee context")?;
    ensure!(
        selected.body["membershipId"] == json!(membership.id)
            && selected.body["memberType"] == "EMPLOYEE",
        "platform selected another concrete member or kind"
    );
    Ok(Platform {
        access: text(&selected.body, "accessToken")?.to_owned(),
        refresh: text(&selected.body, "refreshToken")?.to_owned(),
        ticket,
    })
}

async fn oauth(
    app: &Router,
    client: &Client,
    account: &Account,
    member: Member,
    supplied: &str,
) -> Result<Reply> {
    form(
        app,
        "/oauth/token",
        &[
            ("grant_type", "password".into()),
            ("client_id", client.public_id.clone()),
            ("client_secret", CLIENT_SECRET.into()),
            ("username", account.username.clone()),
            ("password", supplied.into()),
            ("membership_id", member.id.to_string()),
            ("scope", "profile".into()),
        ],
    )
    .await
}

async fn token(pool: &PgPool, reply: Reply, member: Member, client: &Client) -> Result<Token> {
    expect(
        &reply,
        StatusCode::OK,
        "real ordinary OAuth password exchange",
    )?;
    let access = text(&reply.body, "access_token")?.to_owned();
    let refresh = text(&reply.body, "refresh_token")?.to_owned();
    let proof: (String, Uuid, Uuid, String, i64, Option<Uuid>) = sqlx::query_as(
        "SELECT token_use,user_id,membership_id,member_type,account_version,application_id FROM oauth_tokens WHERE token_hash=$1 AND NOT revoked")
        .bind(hash::sha256_hex(&access)).fetch_one(pool).await?;
    ensure!(
        proof
            == (
                "oauth".into(),
                member.user,
                member.id,
                member.kind.into(),
                0,
                client.application
            ),
        "ordinary OAuth did not retain its exact persisted account/member/client purpose"
    );
    Ok(Token { access, refresh })
}

async fn membership_snapshot(pool: &PgPool, user: Uuid) -> Result<Value> {
    Ok(sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(m) ORDER BY m.id),'[]'::jsonb) FROM memberships m WHERE user_id=$1")
        .bind(user).fetch_one(pool).await?)
}

struct Issued {
    admin: Platform,
    employees: Vec<Platform>,
    human: Vec<(Client, Token)>,
    app: String,
    unrelated: Platform,
    machine: String,
}

async fn issued(pool: &PgPool, fixture: &Fixture) -> Result<Issued> {
    let app = fixture.app();
    let admin = platform(&app, &fixture.admin, fixture.admin_member, PASSWORD).await?;
    let mut employees = Vec::new();
    let mut human = Vec::new();
    for employee in fixture.employees {
        employees.push(platform(&app, &fixture.subject, employee, PASSWORD).await?);
        human.push((
            fixture.legacy.clone(),
            token(
                pool,
                oauth(&app, &fixture.legacy, &fixture.subject, employee, PASSWORD).await?,
                employee,
                &fixture.legacy,
            )
            .await?,
        ));
    }
    let customer = token(
        pool,
        oauth(
            &app,
            &fixture.customer_client,
            &fixture.subject,
            fixture.customer,
            PASSWORD,
        )
        .await?,
        fixture.customer,
        &fixture.customer_client,
    )
    .await?;
    let profile = call(
        &app,
        Method::GET,
        "/api/customer/profile",
        Some(&customer.access),
        Value::Null,
    )
    .await?;
    expect(
        &profile,
        StatusCode::OK,
        "registered real customer profile remains available",
    )?;
    ensure!(
        profile.body["context"]["membershipId"] == json!(fixture.customer.id)
            && profile.body["account"]["id"] == json!(fixture.subject.id)
            && profile.body["account"].get("isSuperAdmin").is_none()
            && profile.body["account"].get("passwordHash").is_none(),
        "customer account contract broadened"
    );
    human.push((fixture.customer_client.clone(), customer));
    let app_token = call(
        &app,
        Method::POST,
        "/api/oauth/session-token",
        Some(&employees[0].access),
        json!({"appCode":fixture.employee_application_code}),
    )
    .await?;
    expect(
        &app_token,
        StatusCode::CREATED,
        "real employee application token",
    )?;
    let app_access = text(&app_token.body, "accessToken")?.to_owned();
    expect(
        &call(
            &app,
            Method::GET,
            "/oauth/userinfo",
            Some(&app_access),
            Value::Null,
        )
        .await?,
        StatusCode::OK,
        "real application credential positive control",
    )?;
    let unrelated = platform(&app, &fixture.unrelated, fixture.unrelated_member, PASSWORD).await?;
    let machine = form(
        &app,
        "/oauth/token",
        &[
            ("grant_type", "client_credentials".into()),
            ("client_id", fixture.service.public_id.clone()),
            ("client_secret", CLIENT_SECRET.into()),
        ],
    )
    .await?;
    expect(&machine, StatusCode::OK, "real unrelated machine grant")?;
    Ok(Issued {
        admin,
        employees,
        human,
        app: app_access,
        unrelated,
        machine: text(&machine.body, "access_token")?.to_owned(),
    })
}

async fn revoked(app: &Router, fixture: &Fixture, old: &Issued) -> Result<()> {
    for platform in &old.employees {
        expect(
            &call(
                app,
                Method::GET,
                "/api/auth/profile",
                Some(&platform.access),
                Value::Null,
            )
            .await?,
            StatusCode::UNAUTHORIZED,
            "account-wide old platform access",
        )?;
        let refresh = form(
            app,
            "/oauth/token",
            &[
                ("grant_type", "refresh_token".into()),
                ("refresh_token", platform.refresh.clone()),
            ],
        )
        .await?;
        expect(
            &refresh,
            StatusCode::BAD_REQUEST,
            "account-wide old platform refresh",
        )?;
        ensure!(
            refresh.body["error"] == "invalid_grant",
            "old platform refresh failed for unrelated reason"
        );
        expect(
            &call(
                app,
                Method::POST,
                "/api/auth/context",
                Some(&platform.ticket),
                json!({"membershipId":fixture.employees[0].id}),
            )
            .await?,
            StatusCode::UNAUTHORIZED,
            "old credential selection ticket",
        )?;
    }
    for (client, pair) in &old.human {
        expect(
            &call(
                app,
                Method::GET,
                "/oauth/userinfo",
                Some(&pair.access),
                Value::Null,
            )
            .await?,
            StatusCode::UNAUTHORIZED,
            "account-wide old ordinary OAuth access",
        )?;
        let refresh = form(
            app,
            "/oauth/token",
            &[
                ("grant_type", "refresh_token".into()),
                ("client_id", client.public_id.clone()),
                ("client_secret", CLIENT_SECRET.into()),
                ("refresh_token", pair.refresh.clone()),
            ],
        )
        .await?;
        expect(
            &refresh,
            StatusCode::BAD_REQUEST,
            "account-wide old ordinary OAuth refresh",
        )?;
        ensure!(
            refresh.body["error"] == "invalid_grant",
            "old OAuth refresh failed for unrelated reason"
        );
    }
    expect(
        &call(
            app,
            Method::GET,
            "/api/customer/profile",
            Some(&old.human[2].1.access),
            Value::Null,
        )
        .await?,
        StatusCode::UNAUTHORIZED,
        "old customer access after global account mutation",
    )?;
    expect(
        &call(
            app,
            Method::GET,
            "/oauth/userinfo",
            Some(&old.app),
            Value::Null,
        )
        .await?,
        StatusCode::UNAUTHORIZED,
        "old application token after global account mutation",
    )?;
    expect(
        &call(
            app,
            Method::GET,
            "/api/auth/profile",
            Some(&old.unrelated.access),
            Value::Null,
        )
        .await?,
        StatusCode::OK,
        "unrelated account remains authenticated",
    )?;
    let machine = form(
        app,
        "/oauth/introspect",
        &[
            ("client_id", fixture.service.public_id.clone()),
            ("client_secret", CLIENT_SECRET.into()),
            ("token", old.machine.clone()),
        ],
    )
    .await?;
    expect(&machine, StatusCode::OK, "unrelated machine introspection")?;
    ensure!(
        machine.body["active"] == true,
        "global human account change revoked a machine credential"
    );
    Ok(())
}

const ORIGIN: &str = "https://auth.example.invalid";
const SESSION_COOKIE: &str = "__Host-saas_sso";
const PREAUTH_COOKIE: &str = "__Host-saas_sso_preauth";

struct Sso {
    cookie: String,
    csrf: String,
}

fn no_store(reply: &Reply) -> Result<()> {
    ensure!(
        reply
            .headers
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.split(',').any(|part| part.trim() == "no-store")),
        "SSO success or error response is cacheable"
    );
    Ok(())
}

fn response_cookie(reply: &Reply, name: &str) -> Result<String> {
    let prefix = format!("{name}=");
    let cookies = reply
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter(|value| value.starts_with(&prefix))
        .collect::<Vec<_>>();
    ensure!(
        cookies.len() == 1,
        "SSO response omitted or repeated its expected cookie"
    );
    let raw = cookies[0];
    let attributes = raw.split(';').map(str::trim).collect::<Vec<_>>();
    ensure!(
        attributes.contains(&"Secure")
            && attributes.contains(&"HttpOnly")
            && attributes.contains(&"SameSite=Lax")
            && attributes.contains(&"Path=/")
            && !attributes
                .iter()
                .any(|part| part.to_ascii_lowercase().starts_with("domain=")),
        "SSO cookie broadened its origin or weakened browser protections"
    );
    Ok(attributes[0].to_owned())
}

fn cookie_secret(cookie: &str) -> Result<&str> {
    let value = cookie
        .split_once('=')
        .map(|(_, value)| value)
        .ok_or_else(|| anyhow!("owned SSO fixture cookie has no value"))?;
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|ch| ch.is_ascii_digit() || (b'a'..=b'f').contains(&ch)),
        "SSO cookie is not a canonical 256-bit opaque value"
    );
    Ok(value)
}

async fn browser(
    app: &Router,
    method: Method,
    path: &str,
    cookie: Option<&str>,
    csrf: Option<&str>,
    body: Value,
) -> Result<Reply> {
    let mut headers = vec![("origin", ORIGIN), ("content-type", "application/json")];
    if let Some(cookie) = cookie {
        headers.push(("cookie", cookie));
    }
    if let Some(csrf) = csrf {
        headers.push(("x-csrf-token", csrf));
    }
    let reply = exchange(app, method, path, None, body.to_string(), &headers).await?;
    no_store(&reply)?;
    Ok(reply)
}

async fn challenge(app: &Router) -> Result<Sso> {
    let reply = browser(
        app,
        Method::GET,
        "/api/sso/preauth",
        None,
        None,
        Value::Null,
    )
    .await?;
    expect(
        &reply,
        StatusCode::OK,
        "owned SSO preauthentication challenge",
    )?;
    let cookie = response_cookie(&reply, PREAUTH_COOKIE)?;
    cookie_secret(&cookie)?;
    Ok(Sso {
        cookie,
        csrf: text(&reply.body, "csrfToken")?.to_owned(),
    })
}

async fn sso_login(app: &Router, account: &Account, supplied: &str) -> Result<Sso> {
    let proof = challenge(app).await?;
    let reply = browser(
        app,
        Method::POST,
        "/api/sso/login",
        Some(&proof.cookie),
        Some(&proof.csrf),
        json!({"username":account.username,"password":supplied}),
    )
    .await?;
    expect(
        &reply,
        StatusCode::OK,
        "actual member-independent SSO password authentication",
    )?;
    ensure!(
        reply.body["contractVersion"] == "sso-session/1"
            && reply.body["account"]["id"] == json!(account.id)
            && reply.body["account"]["username"] == account.username,
        "SSO login changed the stable account contract"
    );
    let cookie = response_cookie(&reply, SESSION_COOKIE)?;
    cookie_secret(&cookie)?;
    ensure!(
        cookie_secret(&cookie)? != cookie_secret(&proof.cookie)?,
        "SSO login reused its preauthentication secret"
    );
    let cleared = response_cookie(&reply, PREAUTH_COOKIE)?;
    ensure!(
        cleared == format!("{PREAUTH_COOKIE}="),
        "successful login retained its browser challenge"
    );
    Ok(Sso {
        cookie,
        csrf: text(&reply.body, "csrfToken")?.to_owned(),
    })
}

async fn sso_session(app: &Router, sso: &Sso) -> Result<Reply> {
    browser(
        app,
        Method::GET,
        "/api/sso/session",
        Some(&sso.cookie),
        None,
        Value::Null,
    )
    .await
}

async fn session_id(pool: &PgPool, sso: &Sso) -> Result<Uuid> {
    Ok(
        sqlx::query_scalar("SELECT id FROM sso_sessions WHERE token_hash=$1")
            .bind(hash::sha256_hex(cookie_secret(&sso.cookie)?))
            .fetch_one(pool)
            .await?,
    )
}

async fn approve_code(app: &Router, client: &Client, parent: &Platform) -> Result<String> {
    let query = [
        ("client_id", client.public_id.as_str()),
        ("redirect_uri", REDIRECT),
        ("response_type", "code"),
        ("scope", "profile"),
        ("state", "owned-phase08-state"),
    ]
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
    let pending = call(
        app,
        Method::GET,
        &format!("/oauth/authorize?{query}"),
        None,
        Value::Null,
    )
    .await?;
    expect(&pending, StatusCode::FOUND, "real pending OAuth consent")?;
    let location = pending
        .headers
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow!("real authorization omitted consent location"))?;
    let location = reqwest::Url::parse(ORIGIN)?.join(location)?;
    let request_id = location
        .query_pairs()
        .find(|(key, _)| key == "request_id")
        .map(|(_, value)| value.into_owned())
        .ok_or_else(|| anyhow!("real consent omitted request ID"))?;
    let approved = call(
        app,
        Method::POST,
        &format!("/api/oauth/consent/{request_id}/decision"),
        Some(&parent.access),
        json!({"approve":true}),
    )
    .await?;
    expect(
        &approved,
        StatusCode::OK,
        "real platform-authorized code approval",
    )?;
    let redirect = reqwest::Url::parse(text(&approved.body, "redirectTo")?)?;
    ensure!(
        redirect
            .query_pairs()
            .any(|(key, value)| key == "state" && value == "owned-phase08-state"),
        "real consent lost its original state"
    );
    redirect
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .ok_or_else(|| anyhow!("real approval omitted its code"))
}

async fn approve_device(app: &Router, client: &Client, parent: &Platform) -> Result<String> {
    let pending = form(
        app,
        "/oauth/device/authorization",
        &[
            ("client_id", client.public_id.clone()),
            ("client_secret", CLIENT_SECRET.into()),
            ("scope", "profile".into()),
        ],
    )
    .await?;
    expect(&pending, StatusCode::OK, "real pending OAuth device")?;
    let user_code = text(&pending.body, "user_code")?;
    let approved = call(
        app,
        Method::POST,
        &format!("/api/device/{user_code}/decision"),
        Some(&parent.access),
        json!({"approve":true}),
    )
    .await?;
    expect(
        &approved,
        StatusCode::OK,
        "real platform-authorized device approval",
    )?;
    Ok(text(&pending.body, "device_code")?.to_owned())
}

async fn exchange_grant(app: &Router, client: &Client, kind: &str, value: &str) -> Result<Reply> {
    let mut fields = vec![
        ("client_id", client.public_id.clone()),
        ("client_secret", CLIENT_SECRET.into()),
    ];
    match kind {
        "code" => fields.extend([
            ("grant_type", "authorization_code".into()),
            ("code", value.into()),
            ("redirect_uri", REDIRECT.into()),
        ]),
        "device" => fields.extend([
            (
                "grant_type",
                "urn:ietf:params:oauth:grant-type:device_code".into(),
            ),
            ("device_code", value.into()),
        ]),
        _ => return Err(anyhow!("unknown owned Phase 8 grant kind")),
    }
    form(app, "/oauth/token", &fields).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE08_TEST_DATABASE_URL and disposable database creation"]
async fn phase08_memberless_password_authentication_cannot_authorize_a_business_context(
) -> Result<()> {
    let database = Database::create().await?;
    let assertions = async {
        let fixture = Fixture::create(&database.pool).await?;
        let app = fixture.app();
        let sso = sso_login(&app,&fixture.memberless,PASSWORD).await?;
        let reply = sso_session(&app,&sso).await?;
        expect(&reply,StatusCode::OK,"memberless global authentication state")?;
        let account_keys = reply.body["account"].as_object().ok_or_else(||anyhow!("SSO account DTO missing"))?
            .keys().map(String::as_str).collect::<std::collections::BTreeSet<_>>();
        ensure!(account_keys == std::collections::BTreeSet::from(["id","username","realName","avatar"]),
            "global authentication disclosed credential, member or platform authority fields");
        for key in ["membershipId","organizationId","tenantId","context","roles","permissions","isSuperAdmin","ticket","accessToken","refreshToken"] {
            ensure!(reply.body.get(key).is_none(), "global authentication manufactured business field {key}");
        }
        let count:i64 = sqlx::query_scalar("SELECT count(*) FROM memberships WHERE user_id=$1")
            .bind(fixture.memberless.id).fetch_one(&database.pool).await?;
        ensure!(count == 0,"SSO authentication implicitly created an organization identity");
        expect(&login(&app,&fixture.memberless,PASSWORD).await?,StatusCode::FORBIDDEN,"legacy employee login still requires employee membership")?;
        for path in ["/api/auth/profile","/api/memberships","/api/customer/profile"] {
            let reply = exchange(&app,Method::GET,path,None,String::new(),&[("origin",ORIGIN),("cookie",&sso.cookie)]).await?;
            expect(&reply,StatusCode::UNAUTHORIZED,"global cookie cannot replace a business bearer")?;
        }
        expect(&call(&app,Method::POST,"/api/auth/context",Some(cookie_secret(&sso.cookie)?),
            json!({"membershipId":fixture.admin_member.id})).await?,StatusCode::UNAUTHORIZED,"opaque global secret is not a context ticket")?;
        let hash_only:bool = sqlx::query_scalar("SELECT token_hash=$2 AND token_hash<>$3 AND csrf_hash<>$4 AND account_version=0 AND revoked_at IS NULL FROM sso_sessions WHERE user_id=$1")
            .bind(fixture.memberless.id).bind(hash::sha256_hex(cookie_secret(&sso.cookie)?))
            .bind(cookie_secret(&sso.cookie)?).bind(&sso.csrf).fetch_one(&database.pool).await?;
        ensure!(hash_only,"global session persisted a bearer or CSRF plaintext or wrong account generation");
        let changed=browser(&app,Method::PUT,"/api/sso/password",Some(&sso.cookie),Some(&sso.csrf),
            json!({"oldPassword":PASSWORD,"newPassword":NEW_PASSWORD})).await?;
        expect(&changed,StatusCode::OK,"memberless account can change its own password")?;
        ensure!(changed.body["ok"]==true && response_cookie(&changed,SESSION_COOKIE)?==format!("{SESSION_COOKIE}="),
            "memberless password command did not report success or clear the old global cookie");
        expect(&sso_session(&app,&sso).await?,StatusCode::UNAUTHORIZED,"memberless password change revokes its original global session")?;
        let (version,encoded):(i64,String)=sqlx::query_as("SELECT session_version,password_hash FROM users WHERE id=$1")
            .bind(fixture.memberless.id).fetch_one(&database.pool).await?;
        ensure!(version==1 && password::verify_password(&encoded,NEW_PASSWORD) && !password::verify_password(&encoded,PASSWORD),
            "memberless self command did not replace the actual password and advance one generation");
        let fresh=sso_login(&app,&fixture.memberless,NEW_PASSWORD).await?;
        expect(&sso_session(&app,&fresh).await?,StatusCode::OK,"memberless account freshly authenticates with its replacement password")?;
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM memberships WHERE user_id=$1")
            .bind(fixture.memberless.id).fetch_one(&database.pool).await?;
        ensure!(count==0,"memberless password change or fresh authentication created an organization identity");
        let logged_out=browser(&app,Method::POST,"/api/sso/logout",Some(&fresh.cookie),Some(&fresh.csrf),Value::Null).await?;
        expect(&logged_out,StatusCode::OK,"memberless account can globally log out")?;
        ensure!(logged_out.body["ok"]==true && response_cookie(&logged_out,SESSION_COOKIE)?==format!("{SESSION_COOKIE}="),
            "memberless logout did not report success or clear its browser credential");
        expect(&sso_session(&app,&fresh).await?,StatusCode::UNAUTHORIZED,"memberless logout invalidates the replacement global session")?;
        let final_state:(i64,i64)=sqlx::query_as("SELECT session_version,(SELECT count(*) FROM memberships WHERE user_id=$1) FROM users WHERE id=$1")
            .bind(fixture.memberless.id).fetch_one(&database.pool).await?;
        ensure!(final_state==(2,0),"memberless logout failed to advance the global generation or created an identity");
        Ok(())
    }.await;
    database.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE08_TEST_DATABASE_URL and disposable database creation"]
async fn phase08_cookie_origin_csrf_expiry_and_fixation_contracts_fail_closed() -> Result<()> {
    let database = Database::create().await?;
    let assertions = async {
        let fixture = Fixture::create(&database.pool).await?;
        let app = fixture.app();
        for headers in [
            vec![],
            vec![("origin","null")],
            vec![("origin","http://auth.example.invalid")],
            vec![("origin","https://auth.example.invalid.attacker.invalid")],
            vec![("origin","https://auth.example.invalid/")],
            vec![("origin",ORIGIN),("origin",ORIGIN)],
        ] {
            let reply=exchange(&app,Method::GET,"/api/sso/preauth",None,String::new(),&headers).await?;
            no_store(&reply)?;
            expect(&reply,StatusCode::FORBIDDEN,"preauthentication requires one exact Origin")?;
        }
        let proof = challenge(&app).await?;
        let body = json!({"username":fixture.memberless.username,"password":PASSWORD});
        expect(&browser(&app,Method::POST,"/api/sso/login",Some(&proof.cookie),None,body.clone()).await?,
            StatusCode::FORBIDDEN,"login CSRF omission")?;
        expect(&browser(&app,Method::POST,"/api/sso/login",Some(&proof.cookie),Some("wrong-proof"),body.clone()).await?,
            StatusCode::FORBIDDEN,"login CSRF forgery")?;
        let duplicate_csrf = exchange(&app,Method::POST,"/api/sso/login",None,body.to_string(),&[
            ("origin",ORIGIN),("content-type","application/json"),("cookie",&proof.cookie),
            ("x-csrf-token",&proof.csrf),("x-csrf-token",&proof.csrf),
        ]).await?;
        no_store(&duplicate_csrf)?;
        expect(&duplicate_csrf,StatusCode::FORBIDDEN,"repeated login CSRF header")?;
        let authenticated = sso_login(&app,&fixture.memberless,PASSWORD).await?;
        let sid=session_id(&database.pool,&authenticated).await?;
        let (version,hash_before):(i64,String)=sqlx::query_as("SELECT session_version,password_hash FROM users WHERE id=$1")
            .bind(fixture.memberless.id).fetch_one(&database.pool).await?;
        let fetched=sso_session(&app,&authenticated).await?;
        expect(&fetched,StatusCode::OK,"safe session read after browser reload")?;
        ensure!(text(&fetched.body,"csrfToken")?==authenticated.csrf
            && authenticated.csrf==hash::sha256_hex(&format!("sso-session-csrf-v1:{}",cookie_secret(&authenticated.cookie)?)),
            "session read cannot recover the same domain-separated CSRF proof");
        ensure!(!fetched.headers.contains_key(header::SET_COOKIE),"read-only global session lookup rotated browser state");
        for (method,path,body) in [
            (Method::POST,"/api/sso/logout",Value::Null),
            (Method::PUT,"/api/sso/password",json!({"oldPassword":PASSWORD,"newPassword":NEW_PASSWORD})),
        ] {
            for csrf in [None,Some("forged-session-proof")] {
                expect(&browser(&app,method.clone(),path,Some(&authenticated.cookie),csrf,body.clone()).await?,
                    StatusCode::FORBIDDEN,"global self command CSRF proof")?;
            }
            let wrong_origin=exchange(&app,method,path,None,body.to_string(),&[
                ("origin","https://attacker.invalid"),("content-type","application/json"),
                ("cookie",&authenticated.cookie),("x-csrf-token",&authenticated.csrf),
            ]).await?;
            no_store(&wrong_origin)?;
            expect(&wrong_origin,StatusCode::FORBIDDEN,"global self command exact Origin")?;
        }
        let unchanged:(i64,String)=sqlx::query_as("SELECT session_version,password_hash FROM users WHERE id=$1")
            .bind(fixture.memberless.id).fetch_one(&database.pool).await?;
        ensure!(unchanged==(version,hash_before),"rejected CSRF request changed the account");
        let forged=format!("{SESSION_COOKIE}={}","0".repeat(64));
        let malformed=format!("{SESSION_COOKIE}=untrusted-not-a-session");
        let duplicate=format!("{}; {}",authenticated.cookie,authenticated.cookie);
        let ambiguous=format!("{}; {}",authenticated.cookie,forged);
        for cookie in [&forged,&malformed,&duplicate,&ambiguous] {
            let response=browser(&app,Method::GET,"/api/sso/session",Some(cookie),None,Value::Null).await?;
            ensure!(response.status.is_client_error(),"forged, malformed or duplicate global cookie was accepted");
            ensure!(response.body.get("account").is_none(),"invalid global cookie disclosed authenticated account state");
        }
        let double_cookie_headers=exchange(&app,Method::GET,"/api/sso/session",None,String::new(),&[
            ("origin",ORIGIN),("cookie",&authenticated.cookie),("cookie",&authenticated.cookie),
        ]).await?;
        no_store(&double_cookie_headers)?;
        ensure!(double_cookie_headers.status.is_client_error(),"multiple Cookie headers bypassed duplicate rejection");
        let expired_challenge=challenge(&app).await?;
        sqlx::query("UPDATE sso_login_challenges SET created_at=now()-interval '1 hour',expires_at=now()-interval '1 second' WHERE token_hash=$1")
            .bind(hash::sha256_hex(cookie_secret(&expired_challenge.cookie)?)).execute(&database.pool).await?;
        expect(&browser(&app,Method::POST,"/api/sso/login",Some(&expired_challenge.cookie),Some(&expired_challenge.csrf),body.clone()).await?,
            StatusCode::FORBIDDEN,"expired preauthentication challenge")?;
        let single_use=challenge(&app).await?;
        let cookie_with_attacker_sid=format!("{}; {}",single_use.cookie,forged);
        let logged_in=browser(&app,Method::POST,"/api/sso/login",Some(&cookie_with_attacker_sid),Some(&single_use.csrf),body.clone()).await?;
        expect(&logged_in,StatusCode::OK,"valid password with an untrusted fixed browser sid")?;
        let rotated=response_cookie(&logged_in,SESSION_COOKIE)?;
        ensure!(cookie_secret(&rotated)?!=cookie_secret(&forged)?
            && cookie_secret(&rotated)?!=cookie_secret(&authenticated.cookie)?,
            "SSO password login adopted a fixed or existing global session secret");
        expect(&browser(&app,Method::POST,"/api/sso/login",Some(&single_use.cookie),Some(&single_use.csrf),body).await?,
            StatusCode::FORBIDDEN,"preauthentication challenge replay")?;
        sqlx::query("UPDATE sso_sessions SET authenticated_at=now()-interval '1 hour',expires_at=now()-interval '1 second' WHERE id=$1")
            .bind(sid).execute(&database.pool).await?;
        expect(&sso_session(&app,&authenticated).await?,StatusCode::UNAUTHORIZED,"expired global session")?;
        let rows:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM sso_sessions WHERE user_id=$1),(SELECT count(*) FROM memberships WHERE user_id=$1)")
            .bind(fixture.memberless.id).fetch_one(&database.pool).await?;
        ensure!(rows==(2,0),"rejected security requests persisted another global or membership session");
        let audit:String=sqlx::query_scalar("SELECT COALESCE(string_agg(to_jsonb(a)::text,''),'') FROM audit_logs a WHERE user_id=$1")
            .bind(fixture.memberless.id).fetch_one(&database.pool).await?;
        for sensitive in [PASSWORD,NEW_PASSWORD,cookie_secret(&authenticated.cookie)?,authenticated.csrf.as_str(),single_use.csrf.as_str()] {
            ensure!(!audit.contains(sensitive),"SSO audit persisted plaintext credential material");
        }
        Ok(())
    }.await;
    database.finish(assertions).await
}

async fn grant_pair(
    app: &Router,
    fixture: &Fixture,
    parent: &Platform,
) -> Result<(String, String)> {
    let positive_code = approve_code(app, &fixture.legacy, parent).await?;
    expect(
        &exchange_grant(app, &fixture.legacy, "code", &positive_code).await?,
        StatusCode::OK,
        "real code exchange positive control",
    )?;
    let positive_device = approve_device(app, &fixture.legacy, parent).await?;
    // Only this disposable test grant has its documented polling interval
    // removed, so the positive/negative checks do not rely on wall-clock sleeps.
    sqlx::query("UPDATE oauth_device_codes SET interval_seconds=0 WHERE device_code_hash=$1")
        .bind(hash::sha256_hex(&positive_device))
        .execute(&fixture.state.pool)
        .await?;
    expect(
        &exchange_grant(app, &fixture.legacy, "device", &positive_device).await?,
        StatusCode::OK,
        "real device exchange positive control",
    )?;
    let code = approve_code(app, &fixture.legacy, parent).await?;
    let device = approve_device(app, &fixture.legacy, parent).await?;
    sqlx::query("UPDATE oauth_device_codes SET interval_seconds=0 WHERE device_code_hash=$1")
        .bind(hash::sha256_hex(&device))
        .execute(&fixture.state.pool)
        .await?;
    Ok((code, device))
}

enum GrantRevocation {
    OwnerRevoke,
    StaleAuthority,
}

async fn grants_revoked(
    app: &Router,
    fixture: &Fixture,
    code: &str,
    device: &str,
    revocation: GrantRevocation,
) -> Result<()> {
    let (consumed, device_status, device_error) = match revocation {
        GrantRevocation::OwnerRevoke => (true, "EXPIRED", "expired_token"),
        GrantRevocation::StaleAuthority => (false, "APPROVED", "invalid_grant"),
    };
    let stored: (bool, bool, String, bool) = sqlx::query_as(
        "SELECT c.consumed,c.expires_at>clock_timestamp(),d.status,d.expires_at>clock_timestamp() \
        FROM oauth_authorization_codes c CROSS JOIN oauth_device_codes d \
        WHERE c.code_hash=$1 AND d.device_code_hash=$2",
    )
    .bind(hash::sha256_hex(code))
    .bind(hash::sha256_hex(device))
    .fetch_one(&fixture.state.pool)
    .await?;
    ensure!(stored==(consumed,true,device_status.into(),true),
        "revoked-grant fixture does not retain the expected physical retirement state and unexpired proof window");
    for (kind, value, expected_error) in [
        ("code", code, "invalid_grant"),
        ("device", device, device_error),
    ] {
        let reply = exchange_grant(app, &fixture.legacy, kind, value).await?;
        expect(
            &reply,
            StatusCode::BAD_REQUEST,
            "old approved human grant after global or local revocation",
        )?;
        ensure!(
            reply.body["error"] == expected_error,
            "retired {kind} must return its exact {expected_error} protocol error"
        );
        ensure!(
            reply.body.get("access_token").is_none(),
            "revoked grant minted a partial credential"
        );
    }
    Ok(())
}

async fn global_revoke_case(kind: &str) -> Result<()> {
    let database = Database::create().await?;
    let assertions=async {
        let fixture=Fixture::create(&database.pool).await?;
        let app=fixture.app();
        let old=issued(&database.pool,&fixture).await?;
        let primary=sso_login(&app,&fixture.subject,PASSWORD).await?;
        let secondary=sso_login(&app,&fixture.subject,PASSWORD).await?;
        let unrelated=sso_login(&app,&fixture.unrelated,PASSWORD).await?;
        let members=membership_snapshot(&database.pool,fixture.subject.id).await?;
        let (code,device)=grant_pair(&app,&fixture,&old.employees[0]).await?;
        let before:(i64,String)=sqlx::query_as("SELECT session_version,password_hash FROM users WHERE id=$1")
            .bind(fixture.subject.id).fetch_one(&database.pool).await?;
        let reply=match kind {
            "logout"=>browser(&app,Method::POST,"/api/sso/logout",Some(&primary.cookie),Some(&primary.csrf),Value::Null).await?,
            "password"=>browser(&app,Method::PUT,"/api/sso/password",Some(&primary.cookie),Some(&primary.csrf),
                json!({"oldPassword":PASSWORD,"newPassword":NEW_PASSWORD})).await?,
            "ban"=>call(&app,Method::PUT,&format!("/api/users/{}/status",fixture.subject.id),Some(&old.admin.access),json!({"status":"DISABLED"})).await?,
            "reset"=>call(&app,Method::POST,&format!("/api/users/{}/reset-password",fixture.subject.id),Some(&old.admin.access),json!({"newPassword":NEW_PASSWORD})).await?,
            "kick"=>call(&app,Method::POST,"/api/oauth/tokens/kick",Some(&old.admin.access),json!({"userId":fixture.subject.id})).await?,
            _=>return Err(anyhow!("unknown owned Phase 8 global revocation case")),
        };
        expect(&reply,StatusCode::OK,"real global account revocation command")?;
        if matches!(kind,"logout"|"password") {
            let cleared=response_cookie(&reply,SESSION_COOKIE)?;
            ensure!(cleared==format!("{SESSION_COOKIE}="),"global self command retained browser session cookie");
            ensure!(reply.body["ok"]==true,"global self command did not report its committed result");
        }
        let after:(i64,String)=sqlx::query_as("SELECT session_version,password_hash FROM users WHERE id=$1")
            .bind(fixture.subject.id).fetch_one(&database.pool).await?;
        ensure!(after.0==before.0+1,"global revocation did not advance exactly one account generation");
        if matches!(kind,"password"|"reset") {
            ensure!(password::verify_password(&after.1,NEW_PASSWORD) && !password::verify_password(&after.1,PASSWORD),
                "password command did not replace the real credential");
        } else { ensure!(after.1==before.1,"non-password global command rewrote the credential"); }
        for sso in [&primary,&secondary] {
            expect(&sso_session(&app,sso).await?,StatusCode::UNAUTHORIZED,"every old global session is invalid after account revocation")?;
        }
        expect(&sso_session(&app,&unrelated).await?,StatusCode::OK,"global revocation retained another account's SSO session")?;
        let physical:(i64,i64)=sqlx::query_as("SELECT count(*),count(*) FILTER(WHERE revoked_at IS NOT NULL) FROM sso_sessions WHERE user_id=$1")
            .bind(fixture.subject.id).fetch_one(&database.pool).await?;
        ensure!(physical==(2,2),"account owner did not atomically revoke all global sessions");
        revoked(&app,&fixture,&old).await?;
        grants_revoked(&app,&fixture,&code,&device,GrantRevocation::OwnerRevoke).await?;
        ensure!(membership_snapshot(&database.pool,fixture.subject.id).await?==members,
            "global account command changed independent member relationships");
        if kind=="ban" {
            let challenge=challenge(&app).await?;
            expect(&browser(&app,Method::POST,"/api/sso/login",Some(&challenge.cookie),Some(&challenge.csrf),
                json!({"username":fixture.subject.username,"password":PASSWORD})).await?,
                StatusCode::UNAUTHORIZED,"disabled account cannot establish a fresh SSO session")?;
            expect(&call(&app,Method::PUT,&format!("/api/users/{}/status",fixture.subject.id),Some(&old.admin.access),json!({"status":"ENABLED"})).await?,
                StatusCode::OK,"explicit account reenable")?;
            expect(&sso_session(&app,&primary).await?,StatusCode::UNAUTHORIZED,"reenable does not revive old global session")?;
        }
        let fresh=sso_login(&app,&fixture.subject,if matches!(kind,"password"|"reset") {NEW_PASSWORD} else {PASSWORD}).await?;
        expect(&sso_session(&app,&fresh).await?,StatusCode::OK,"explicit fresh authentication establishes a new generation")?;
        ensure!(session_id(&database.pool,&fresh).await?!=session_id(&database.pool,&primary).await?,
            "fresh password authentication revived the old global session ID");
        Ok(())
    }.await;
    database.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE08_TEST_DATABASE_URL and disposable database creation"]
async fn phase08_global_logout_revokes_sso_and_all_legacy_human_credentials() -> Result<()> {
    global_revoke_case("logout").await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE08_TEST_DATABASE_URL and disposable database creation"]
async fn phase08_self_password_revokes_sso_and_all_legacy_human_credentials() -> Result<()> {
    global_revoke_case("password").await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE08_TEST_DATABASE_URL and disposable database creation"]
async fn phase08_global_ban_revokes_sso_and_all_legacy_human_credentials() -> Result<()> {
    global_revoke_case("ban").await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE08_TEST_DATABASE_URL and disposable database creation"]
async fn phase08_admin_reset_revokes_sso_and_all_legacy_human_credentials() -> Result<()> {
    global_revoke_case("reset").await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE08_TEST_DATABASE_URL and disposable database creation"]
async fn phase08_global_kick_revokes_sso_and_all_legacy_human_credentials() -> Result<()> {
    global_revoke_case("kick").await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE08_TEST_DATABASE_URL and disposable database creation"]
async fn phase08_account_generation_denies_live_rows_before_physical_cleanup() -> Result<()> {
    let database = Database::create().await?;
    let assertions=async {
        let fixture=Fixture::create(&database.pool).await?;
        let app=fixture.app();
        let old=issued(&database.pool,&fixture).await?;
        let sso=sso_login(&app,&fixture.subject,PASSWORD).await?;
        let (code,device)=grant_pair(&app,&fixture,&old.employees[0]).await?;
        // Fault injection models delayed best-effort cleanup without an event
        // consumer. The account generation itself must close the authority gate.
        sqlx::query("UPDATE users SET session_version=session_version+1 WHERE id=$1")
            .bind(fixture.subject.id).execute(&database.pool).await?;
        // The real owner triggers synchronously revoke every family. Restore
        // physical liveness only as an explicit corruption/delayed-cleanup
        // injection; all independent account-version predicates must still deny.
        // No production trigger or constraint is disabled.
        sqlx::query("UPDATE sso_sessions SET revoked_at=NULL WHERE user_id=$1")
            .bind(fixture.subject.id).execute(&database.pool).await?;
        sqlx::query("UPDATE platform_sessions SET revoked_at=NULL WHERE user_id=$1")
            .bind(fixture.subject.id).execute(&database.pool).await?;
        sqlx::query("UPDATE platform_session_refresh_tokens r SET consumed_at=NULL FROM platform_sessions s WHERE r.session_id=s.id AND s.user_id=$1")
            .bind(fixture.subject.id).execute(&database.pool).await?;
        sqlx::query("UPDATE oauth_tokens SET revoked=false WHERE user_id=$1")
            .bind(fixture.subject.id).execute(&database.pool).await?;
        sqlx::query("UPDATE oauth_authorization_codes SET consumed=false WHERE code_hash=$1")
            .bind(hash::sha256_hex(&code)).execute(&database.pool).await?;
        sqlx::query("UPDATE oauth_device_codes SET status='APPROVED' WHERE device_code_hash=$1")
            .bind(hash::sha256_hex(&device)).execute(&database.pool).await?;
        let live:(bool,bool,bool,bool)=sqlx::query_as("SELECT \
            EXISTS(SELECT 1 FROM sso_sessions WHERE user_id=$1 AND revoked_at IS NULL), \
            EXISTS(SELECT 1 FROM oauth_tokens WHERE user_id=$1 AND NOT revoked), \
            EXISTS(SELECT 1 FROM oauth_authorization_codes WHERE code_hash=$2 AND NOT consumed), \
            EXISTS(SELECT 1 FROM oauth_device_codes WHERE device_code_hash=$3 AND status='APPROVED')")
            .bind(fixture.subject.id).bind(hash::sha256_hex(&code)).bind(hash::sha256_hex(&device))
            .fetch_one(&database.pool).await?;
        ensure!(live==(true,true,true,true),"fault injection accidentally completed physical credential cleanup");
        expect(&sso_session(&app,&sso).await?,StatusCode::UNAUTHORIZED,"global generation rejected physically live session")?;
        revoked(&app,&fixture,&old).await?;
        grants_revoked(&app,&fixture,&code,&device,GrantRevocation::StaleAuthority).await?;
        Ok(())
    }.await;
    database.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE08_TEST_DATABASE_URL and disposable database creation"]
async fn phase08_local_membership_revocation_preserves_global_sso_and_other_identities(
) -> Result<()> {
    let database = Database::create().await?;
    let assertions=async {
        let fixture=Fixture::create(&database.pool).await?;
        let app=fixture.app();
        let old=issued(&database.pool,&fixture).await?;
        let global=sso_login(&app,&fixture.subject,PASSWORD).await?;
        let (code,device)=grant_pair(&app,&fixture,&old.employees[0]).await?;
        let version:i64=sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
            .bind(fixture.subject.id).fetch_one(&database.pool).await?;
        let employee=fixture.employees[0];
        ensure!(employee.org==fixture.customer.org,"local isolation fixture does not exercise same-organization dual identities");
        let disable=call(&app,Method::PUT,&format!("/api/memberships/{}/status",employee.id),Some(&old.admin.access),
            json!({"status":"DISABLED","expectedIdentityVersion":1,"reason":"Owned Phase 8 employee departure"})).await?;
        expect(&disable,StatusCode::OK,"real local employee revocation")?;
        expect(&sso_session(&app,&global).await?,StatusCode::OK,"member revocation retains member-independent global authentication")?;
        expect(&call(&app,Method::GET,"/api/auth/profile",Some(&old.employees[0].access),Value::Null).await?,
            StatusCode::UNAUTHORIZED,"departed employee platform context")?;
        let refresh=form(&app,"/oauth/token",&[("grant_type","refresh_token".into()),("refresh_token",old.employees[0].refresh.clone())]).await?;
        expect(&refresh,StatusCode::BAD_REQUEST,"departed employee platform refresh")?;
        ensure!(refresh.body["error"]=="invalid_grant","departed employee refresh failed for unrelated reason");
        for index in [1,2] {
            expect(&call(&app,Method::GET,"/oauth/userinfo",Some(&old.human[index].1.access),Value::Null).await?,
                StatusCode::OK,"same-organization customer and other-organization employee survive local departure")?;
        }
        expect(&call(&app,Method::GET,"/api/customer/profile",Some(&old.human[2].1.access),Value::Null).await?,
            StatusCode::OK,"same-organization customer profile survives employee departure")?;
        expect(&call(&app,Method::GET,"/api/auth/profile",Some(&old.employees[1].access),Value::Null).await?,
            StatusCode::OK,"other organization platform context survives employee departure")?;
        grants_revoked(&app,&fixture,&code,&device,GrantRevocation::OwnerRevoke).await?;
        expect(&call(&app,Method::PUT,&format!("/api/memberships/{}/status",employee.id),Some(&old.admin.access),
            json!({"status":"ENABLED","expectedIdentityVersion":2,"reason":"Owned Phase 8 fresh employee generation"})).await?,
            StatusCode::OK,"explicit employee reenable")?;
        expect(&call(&app,Method::GET,"/api/auth/profile",Some(&old.employees[0].access),Value::Null).await?,
            StatusCode::UNAUTHORIZED,"reenable cannot revive departed employee context")?;
        let fresh_employee=platform(&app,&fixture.subject,employee,PASSWORD).await?;
        expect(&call(&app,Method::PUT,&format!("/api/memberships/{}/status",fixture.customer.id),Some(&old.admin.access),
            json!({"status":"DISABLED","expectedIdentityVersion":1,"reason":"Owned Phase 8 customer relationship ends"})).await?,
            StatusCode::OK,"real reverse local customer revocation")?;
        expect(&call(&app,Method::GET,"/api/customer/profile",Some(&old.human[2].1.access),Value::Null).await?,
            StatusCode::UNAUTHORIZED,"departed customer loses its own context")?;
        for context in [&fresh_employee,&old.employees[1],&old.unrelated] {
            expect(&call(&app,Method::GET,"/api/auth/profile",Some(&context.access),Value::Null).await?,
                StatusCode::OK,"customer revocation retains same-organization employee and other valid identities")?;
        }
        expect(&sso_session(&app,&global).await?,StatusCode::OK,"reverse customer departure retains global SSO")?;
        let current:i64=sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
            .bind(fixture.subject.id).fetch_one(&database.pool).await?;
        ensure!(current==version,"member-local lifecycle advanced the global account generation");
        let live:bool=sqlx::query_scalar("SELECT revoked_at IS NULL FROM sso_sessions WHERE id=$1")
            .bind(session_id(&database.pool,&global).await?).fetch_one(&database.pool).await?;
        ensure!(live,"member-local lifecycle physically revoked the global SSO session");
        Ok(())
    }.await;
    database.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE08_TEST_DATABASE_URL and disposable database creation"]
async fn phase08_unavailable_authority_returns_503_without_partial_authentication() -> Result<()> {
    let database = Database::create().await?;
    let assertions =
        async {
            let fixture = Fixture::create(&database.pool).await?;
            let app = fixture.app();
            let global = sso_login(&app, &fixture.memberless, PASSWORD).await?;
            let preauth = challenge(&app).await?;
            fixture.state.pool.close().await;
            for reply in [
                sso_session(&app, &global).await?,
                browser(
                    &app,
                    Method::POST,
                    "/api/sso/logout",
                    Some(&global.cookie),
                    Some(&global.csrf),
                    Value::Null,
                )
                .await?,
                browser(
                    &app,
                    Method::PUT,
                    "/api/sso/password",
                    Some(&global.cookie),
                    Some(&global.csrf),
                    json!({"oldPassword":PASSWORD,"newPassword":NEW_PASSWORD}),
                )
                .await?,
                browser(
                    &app,
                    Method::POST,
                    "/api/sso/login",
                    Some(&preauth.cookie),
                    Some(&preauth.csrf),
                    json!({"username":fixture.memberless.username,"password":PASSWORD}),
                )
                .await?,
                browser(
                    &app,
                    Method::GET,
                    "/api/sso/preauth",
                    None,
                    None,
                    Value::Null,
                )
                .await?,
            ] {
                expect(
                    &reply,
                    StatusCode::SERVICE_UNAVAILABLE,
                    "unavailable SSO authority is not a wrong-password rejection",
                )?;
                ensure!(reply.body.get("account").is_none() && reply.body.get("csrfToken").is_none()
                && !reply.headers.contains_key(header::SET_COOKIE),
                "authority failure returned partial authentication or replaced the browser cookie");
            }
            Ok(())
        }
        .await;
    database.finish(assertions).await
}

async fn wait_for_account_lock(pool: &PgPool, blocker: i32) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(20),async {
        loop {
            let observed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity \
                WHERE datname=current_database() AND wait_event_type='Lock' \
                AND $1=ANY(pg_blocking_pids(pid)) AND query ILIKE '%users%' AND query ILIKE '%FOR%UPDATE%')")
                .bind(blocker).fetch_one(pool).await?;
            if observed {return Ok::<_,anyhow::Error>(());}
            tokio::task::yield_now().await;
        }
    }).await.map_err(|_|anyhow!("global self command never reached its observed account-row lock barrier"))?
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE08_TEST_DATABASE_URL and disposable database creation"]
async fn phase08_pending_self_commands_recheck_original_global_sid_after_the_account_lock(
) -> Result<()> {
    let database = Database::create().await?;
    let assertions=async {
        let fixture=Fixture::create(&database.pool).await?;
        let app=fixture.app();
        let original_hash=password::hash_password(PASSWORD)?;
        let reset_hash=password::hash_password("Owned-Phase08-Concurrent-Administrator-Reset9!")?;
        for command in ["logout","password"] {
            for mutation in ["sid_revoked","sid_expired","version","status","reset"] {
                let account=account(&database.pool,&format!("phase08-pending-{command}-{mutation}"),&original_hash,false).await?;
                let global=sso_login(&app,&account,PASSWORD).await?;
                let sid=session_id(&database.pool,&global).await?;
                let audit_before:i64=sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE user_id=$1")
                    .bind(account.id).fetch_one(&database.pool).await?;
                let mut blocker=database.pool.begin().await?;
                let pid:i32=sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *blocker).await?;
                sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE").bind(account.id).execute(&mut *blocker).await?;
                let pending_app=app.clone();
                let mut pending=tokio::spawn(async move {
                    let (method,path,body)=if command=="logout" {
                        (Method::POST,"/api/sso/logout",Value::Null)
                    } else {
                        (Method::PUT,"/api/sso/password",json!({"oldPassword":PASSWORD,"newPassword":NEW_PASSWORD}))
                    };
                    browser(&pending_app,method,path,Some(&global.cookie),Some(&global.csrf),body).await
                });
                if let Err(error)=wait_for_account_lock(&database.pool,pid).await {
                    blocker.rollback().await?;
                    pending.abort();
                    let _=pending.await;
                    return Err(error);
                }
                match mutation {
                    "sid_revoked"=>{sqlx::query("UPDATE sso_sessions SET revoked_at=now() WHERE id=$1").bind(sid).execute(&mut *blocker).await?;},
                    "sid_expired"=>{sqlx::query("UPDATE sso_sessions SET authenticated_at=now()-interval '1 hour',expires_at=now()-interval '1 second' WHERE id=$1").bind(sid).execute(&mut *blocker).await?;},
                    "version"=>{sqlx::query("UPDATE users SET session_version=session_version+1 WHERE id=$1").bind(account.id).execute(&mut *blocker).await?;},
                    "status"=>{sqlx::query("UPDATE users SET status='DISABLED' WHERE id=$1").bind(account.id).execute(&mut *blocker).await?;},
                    "reset"=>{crate::session::revoke_all_in_tx(&mut blocker,account.id,Some(&reset_hash)).await?;},
                    _=>return Err(anyhow!("unknown owned global lock-wait mutation")),
                }
                let committed:(i64,String,String)=sqlx::query_as("SELECT session_version,password_hash,status FROM users WHERE id=$1")
                    .bind(account.id).fetch_one(&mut *blocker).await?;
                blocker.commit().await?;
                let reply=match tokio::time::timeout(Duration::from_secs(20),&mut pending).await {
                    Ok(reply)=>reply??,
                    Err(_)=>{pending.abort();let _=pending.await;return Err(anyhow!("global self command did not finish after the account-lock release"));}
                };
                expect(&reply,StatusCode::UNAUTHORIZED,"pending global self command must reject its original stale authentication context")?;
                let actual:(i64,String,String)=sqlx::query_as("SELECT session_version,password_hash,status FROM users WHERE id=$1")
                    .bind(account.id).fetch_one(&database.pool).await?;
                ensure!(actual==committed,"pending stale global command changed account generation, credential or status");
                let audit_after:i64=sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE user_id=$1")
                    .bind(account.id).fetch_one(&database.pool).await?;
                ensure!(audit_after==audit_before,"pending stale global command committed a successful audit");
            }
        }
        Ok(())
    }.await;
    database.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE08_TEST_DATABASE_URL and disposable database creation"]
async fn phase08_browser_same_origin_reads_use_metadata_without_weakening_writes() -> Result<()> {
    let database = Database::create().await?;
    let assertions=async {
        let fixture=Fixture::create(&database.pool).await?;
        let app=fixture.app();
        let preauth=exchange(&app,Method::GET,"/api/sso/preauth",None,String::new(),&[
            ("host","auth.example.invalid"),("sec-fetch-site","same-origin"),
            ("sec-fetch-mode","cors"),("sec-fetch-dest","empty"),
        ]).await?;
        no_store(&preauth)?;
        expect(&preauth,StatusCode::OK,"real browser same-origin GET preauth can omit Origin")?;
        let proof=Sso{cookie:response_cookie(&preauth,PREAUTH_COOKIE)?,csrf:text(&preauth.body,"csrfToken")?.to_owned()};
        let logged_in=browser(&app,Method::POST,"/api/sso/login",Some(&proof.cookie),Some(&proof.csrf),
            json!({"username":fixture.memberless.username,"password":PASSWORD})).await?;
        expect(&logged_in,StatusCode::OK,"real browser login after metadata-authenticated challenge")?;
        let cookie=response_cookie(&logged_in,SESSION_COOKIE)?;
        let csrf=text(&logged_in.body,"csrfToken")?;
        let read=exchange(&app,Method::GET,"/api/sso/session",None,String::new(),&[
            ("host","auth.example.invalid"),("sec-fetch-site","same-origin"),
            ("sec-fetch-mode","cors"),("sec-fetch-dest","empty"),("cookie",&cookie),
        ]).await?;
        no_store(&read)?;
        expect(&read,StatusCode::OK,"real browser same-origin GET session can omit Origin")?;
        ensure!(read.body["account"]["id"]==json!(fixture.memberless.id) && text(&read.body,"csrfToken")?==csrf,
            "same-origin GET failed to restore the authenticated account or CSRF proof");
        for path in ["/api/sso/preauth","/api/sso/session"] {
            for mut headers in [
                vec![("host","auth.example.invalid")],
                vec![("sec-fetch-site","same-origin")],
                vec![("host","auth.example.invalid"),("sec-fetch-site","same-site")],
                vec![("host","auth.example.invalid"),("sec-fetch-site","cross-site")],
                vec![("host","attacker.invalid"),("sec-fetch-site","same-origin")],
                vec![("host","auth.example.invalid"),("host","auth.example.invalid"),("sec-fetch-site","same-origin")],
                vec![("host","auth.example.invalid"),("sec-fetch-site","same-origin"),("sec-fetch-site","same-origin")],
                vec![("host","attacker.invalid"),("x-forwarded-host","auth.example.invalid"),("sec-fetch-site","same-origin")],
                vec![("origin","https://attacker.invalid"),("host","auth.example.invalid"),("sec-fetch-site","same-origin")],
                vec![("origin",ORIGIN),("origin",ORIGIN),("host","auth.example.invalid"),("sec-fetch-site","same-origin")],
            ] {
                headers.push(("cookie",&cookie));
                let reply=exchange(&app,Method::GET,path,None,String::new(),&headers).await?;
                no_store(&reply)?;
                expect(&reply,StatusCode::FORBIDDEN,"metadata fallback requires unique actual same-origin host and cannot override a bad Origin")?;
            }
        }
        let write=exchange(&app,Method::POST,"/api/sso/logout",None,String::new(),&[
            ("host","auth.example.invalid"),("sec-fetch-site","same-origin"),
            ("cookie",&cookie),("x-csrf-token",csrf),("content-type","application/json"),
        ]).await?;
        no_store(&write)?;
        expect(&write,StatusCode::FORBIDDEN,"safe-read metadata fallback never substitutes for write Origin")?;
        Ok(())
    }.await;
    database.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE08_TEST_DATABASE_URL and disposable database creation"]
async fn phase08_preverified_login_cannot_adopt_a_generation_committed_during_its_lock_wait(
) -> Result<()> {
    let database = Database::create().await?;
    let assertions=async {
        let fixture=Fixture::create(&database.pool).await?;
        let app=fixture.app();
        let original_hash=password::hash_password(PASSWORD)?;
        let reset_hash=password::hash_password("Owned-Phase08-Login-Race-Reset9!")?;
        for mutation in ["kick","ban","reset"] {
            let account=account(&database.pool,&format!("phase08-pending-login-{mutation}"),&original_hash,false).await?;
            let preauth=challenge(&app).await?;
            let mut blocker=database.pool.begin().await?;
            let pid:i32=sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *blocker).await?;
            sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE").bind(account.id).execute(&mut *blocker).await?;
            let pending_app=app.clone();
            let username=account.username.clone();
            let mut pending=tokio::spawn(async move {
                browser(&pending_app,Method::POST,"/api/sso/login",Some(&preauth.cookie),Some(&preauth.csrf),
                    json!({"username":username,"password":PASSWORD})).await
            });
            if let Err(error)=wait_for_account_lock(&database.pool,pid).await {
                blocker.rollback().await?;
                pending.abort();
                let _=pending.await;
                return Err(error);
            }
            if mutation=="ban" {
                sqlx::query("UPDATE users SET status='DISABLED' WHERE id=$1").bind(account.id).execute(&mut *blocker).await?;
            }
            crate::session::revoke_all_in_tx(&mut blocker,account.id,
                if mutation=="reset" {Some(reset_hash.as_str())} else {None}).await?;
            blocker.commit().await?;
            let reply=match tokio::time::timeout(Duration::from_secs(20),&mut pending).await {
                Ok(reply)=>reply??,
                Err(_)=>{pending.abort();let _=pending.await;return Err(anyhow!("pending SSO login did not finish after account-lock release"));}
            };
            expect(&reply,StatusCode::UNAUTHORIZED,"preverified password login must retain its original account snapshot")?;
            ensure!(!reply.headers.contains_key(header::SET_COOKIE) && reply.body.get("account").is_none(),
                "stale password proof returned a new global session");
            let rows:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM sso_sessions WHERE user_id=$1),(SELECT count(*) FROM audit_logs WHERE user_id=$1 AND action='SSO_LOGIN')")
                .bind(account.id).fetch_one(&database.pool).await?;
            ensure!(rows==(0,0),"stale password proof committed a session or successful login audit");
        }
        Ok(())
    }.await;
    database.finish(assertions).await
}

async fn failed_password_proof_snapshot(pool: &PgPool, user: Uuid) -> Result<Value> {
    // Compare PHC and token-hash material only in memory; assertions never
    // serialize this snapshot into diagnostics.
    Ok(sqlx::query_scalar("SELECT jsonb_build_object( \
        'account',(SELECT to_jsonb(u) FROM users u WHERE id=$1), \
        'sso',(SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY id),'[]'::jsonb) FROM sso_sessions s WHERE user_id=$1), \
        'sessions',(SELECT COALESCE(jsonb_agg(to_jsonb(s) ORDER BY id),'[]'::jsonb) FROM platform_sessions s WHERE user_id=$1), \
        'refresh',(SELECT COALESCE(jsonb_agg(to_jsonb(r) ORDER BY r.id),'[]'::jsonb) FROM platform_session_refresh_tokens r JOIN platform_sessions s ON s.id=r.session_id WHERE s.user_id=$1), \
        'oauth',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY id),'[]'::jsonb) FROM oauth_tokens t WHERE user_id=$1), \
        'codes',(SELECT COALESCE(jsonb_agg(to_jsonb(c) ORDER BY id),'[]'::jsonb) FROM oauth_authorization_codes c WHERE user_id=$1), \
        'devices',(SELECT COALESCE(jsonb_agg(to_jsonb(d) ORDER BY id),'[]'::jsonb) FROM oauth_device_codes d WHERE user_id=$1), \
        'audit',(SELECT COALESCE(jsonb_agg(to_jsonb(a) ORDER BY id),'[]'::jsonb) FROM audit_logs a WHERE user_id=$1))")
        .bind(user).fetch_one(pool).await?)
}

fn retry_after(reply: &Reply) -> Result<()> {
    expect(
        reply,
        StatusCode::TOO_MANY_REQUESTS,
        "shared password-proof failure budget",
    )?;
    let seconds = reply
        .headers
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    ensure!(
        seconds.is_some_and(|seconds| seconds > 0),
        "password proof throttling omitted a positive Retry-After"
    );
    ensure!(
        !reply.headers.contains_key(header::SET_COOKIE) && reply.body.get("account").is_none(),
        "throttled password proof returned partial authentication or changed a browser cookie"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE08_TEST_DATABASE_URL and disposable database creation"]
async fn phase08_self_password_failures_share_the_login_account_budget_without_side_effects(
) -> Result<()> {
    let database = Database::create().await?;
    let assertions=async {
        let mut fixture=Fixture::create(&database.pool).await?;
        let mut config=(*fixture.state.config).clone();
        config.login_user_failures=2;
        config.login_source_requests=1000;
        fixture.state=AppState::new(database.pool.clone(),Arc::new(config));
        let app=fixture.app();
        let old=issued(&database.pool,&fixture).await?;
        let global=sso_login(&app,&fixture.subject,PASSWORD).await?;
        let unrelated=sso_login(&app,&fixture.unrelated,PASSWORD).await?;
        let before=failed_password_proof_snapshot(&database.pool,fixture.subject.id).await?;
        let oversized=browser(&app,Method::PUT,"/api/sso/password",Some(&global.cookie),Some(&global.csrf),
            json!({"oldPassword":"x".repeat(1025),"newPassword":NEW_PASSWORD})).await?;
        expect(&oversized,StatusCode::BAD_REQUEST,"oversized old password is rejected before password-proof work")?;
        for _ in 0..2 {
            let rejected=browser(&app,Method::PUT,"/api/sso/password",Some(&global.cookie),Some(&global.csrf),
                json!({"oldPassword":"Owned-Phase08-Deliberately-Wrong-Old-Password9!","newPassword":NEW_PASSWORD})).await?;
            expect(&rejected,StatusCode::BAD_REQUEST,"real incorrect old password consumes one account failure")?;
        }
        let limited=browser(&app,Method::PUT,"/api/sso/password",Some(&global.cookie),Some(&global.csrf),
            json!({"oldPassword":"Owned-Phase08-Deliberately-Wrong-Old-Password9!","newPassword":NEW_PASSWORD})).await?;
        retry_after(&limited)?;
        // Each request uses a distinct fixture source address and the source
        // limit is deliberately ample. No input:{username} failure was recorded
        // by the self-command, so this fresh challenge isolates user:{id}.
        let preauth=challenge(&app).await?;
        let login=browser(&app,Method::POST,"/api/sso/login",Some(&preauth.cookie),Some(&preauth.csrf),
            json!({"username":fixture.subject.username,"password":PASSWORD})).await?;
        retry_after(&login)?;
        let correct_but_limited=browser(&app,Method::PUT,"/api/sso/password",Some(&global.cookie),Some(&global.csrf),
            json!({"oldPassword":PASSWORD,"newPassword":NEW_PASSWORD})).await?;
        retry_after(&correct_but_limited)?;
        ensure!(failed_password_proof_snapshot(&database.pool,fixture.subject.id).await?==before,
            "failed or throttled password proofs changed the password, generation, credentials, revocations or audit");
        expect(&sso_session(&app,&global).await?,StatusCode::OK,"proof throttling does not revoke the authenticated global session")?;
        for context in &old.employees {
            expect(&call(&app,Method::GET,"/api/auth/profile",Some(&context.access),Value::Null).await?,
                StatusCode::OK,"proof throttling retains existing employee business contexts")?;
        }
        for (_,pair) in &old.human {
            expect(&call(&app,Method::GET,"/oauth/userinfo",Some(&pair.access),Value::Null).await?,
                StatusCode::OK,"proof throttling retains employee and customer OAuth credentials")?;
        }
        expect(&sso_session(&app,&unrelated).await?,StatusCode::OK,"another account remains independently authenticated")?;
        let fresh_unrelated=sso_login(&app,&fixture.unrelated,PASSWORD).await?;
        expect(&sso_session(&app,&fresh_unrelated).await?,StatusCode::OK,"another account can still prove its password and establish a fresh session")?;
        ensure!(failed_password_proof_snapshot(&database.pool,fixture.subject.id).await?==before,
            "another account's successful authentication cleared or mutated the throttled account's state");
        let still_limited=browser(&app,Method::PUT,"/api/sso/password",Some(&global.cookie),Some(&global.csrf),
            json!({"oldPassword":PASSWORD,"newPassword":NEW_PASSWORD})).await?;
        retry_after(&still_limited)?;
        Ok(())
    }.await;
    database.finish(assertions).await
}

async fn wait_for_audit_lock(pool: &PgPool, blocker: i32) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let observed: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity \
                WHERE datname=current_database() AND wait_event_type='Lock' \
                AND $1=ANY(pg_blocking_pids(pid)) AND query ILIKE '%INSERT INTO audit_logs%')",
            )
            .bind(blocker)
            .fetch_one(pool)
            .await?;
            if observed {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(|_| {
        anyhow!("global self command never reached its observed audit-insert lock barrier")
    })?
}

async fn wait_for_database_expiry(pool: &PgPool, sid: Uuid) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let expired: bool = sqlx::query_scalar(
                "SELECT clock_timestamp()>expires_at FROM sso_sessions WHERE id=$1",
            )
            .bind(sid)
            .fetch_one(pool)
            .await?;
            if expired {
                return Ok::<_, anyhow::Error>(());
            }
            // Poll the authoritative database clock. The observed audit barrier
            // stays held until the original proof has actually expired.
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| anyhow!("database clock did not cross the owned global-session expiry"))?
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE08_TEST_DATABASE_URL and disposable database creation"]
async fn phase08_self_commands_expiring_during_audit_wait_roll_back_before_commit() -> Result<()> {
    let database = Database::create().await?;
    let assertions=async {
        let fixture=Fixture::create(&database.pool).await?;
        let app=fixture.app();
        // This hook is installed only in the disposable, owned test database.
        // The top-level INSERT remains observable in pg_stat_activity while
        // its BEFORE trigger waits on the test's held transaction advisory lock.
        sqlx::raw_sql("CREATE FUNCTION phase08_hold_sso_audit() RETURNS trigger LANGUAGE plpgsql AS $$ \
            BEGIN IF NEW.action IN ('SSO_LOGOUT','SSO_UPDATE_PASSWORD') THEN \
            PERFORM pg_advisory_xact_lock(8038,8); END IF; RETURN NEW; END $$; \
            CREATE TRIGGER phase08_hold_sso_audit BEFORE INSERT ON audit_logs \
            FOR EACH ROW EXECUTE FUNCTION phase08_hold_sso_audit();")
            .execute(&database.pool).await?;
        for command in ["logout","password"] {
            let global=sso_login(&app,&fixture.memberless,PASSWORD).await?;
            let sid=session_id(&database.pool,&global).await?;
            let mut blocker=database.pool.begin().await?;
            let pid:i32=sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *blocker).await?;
            sqlx::query("SELECT pg_advisory_xact_lock(8038,8)").execute(&mut *blocker).await?;
            sqlx::query("UPDATE sso_sessions SET expires_at=clock_timestamp()+interval '10 seconds' WHERE id=$1")
                .bind(sid).execute(&database.pool).await?;
            let before=failed_password_proof_snapshot(&database.pool,fixture.memberless.id).await?;
            let pending_app=app.clone();
            let mut pending=tokio::spawn(async move {
                let (method,path,body)=if command=="logout" {
                    (Method::POST,"/api/sso/logout",Value::Null)
                } else {
                    (Method::PUT,"/api/sso/password",json!({"oldPassword":PASSWORD,"newPassword":NEW_PASSWORD}))
                };
                browser(&pending_app,method,path,Some(&global.cookie),Some(&global.csrf),body).await
            });
            if let Err(error)=wait_for_audit_lock(&database.pool,pid).await {
                blocker.rollback().await?;
                pending.abort();
                let _=pending.await;
                return Err(error);
            }
            let still_valid:bool=sqlx::query_scalar("SELECT clock_timestamp()<expires_at FROM sso_sessions WHERE id=$1")
                .bind(sid).fetch_one(&database.pool).await?;
            if !still_valid {
                blocker.rollback().await?;
                pending.abort();
                let _=pending.await;
                return Err(anyhow!("global proof expired before the observed audit barrier; final-commit guard was not exercised"));
            }
            if let Err(error)=wait_for_database_expiry(&database.pool,sid).await {
                blocker.rollback().await?;
                pending.abort();
                let _=pending.await;
                return Err(error);
            }
            blocker.commit().await?;
            let reply=match tokio::time::timeout(Duration::from_secs(10),&mut pending).await {
                Ok(reply)=>reply??,
                Err(_)=>{pending.abort();let _=pending.await;return Err(anyhow!("global self command did not finish after its audit barrier was released"));}
            };
            expect(&reply,StatusCode::UNAUTHORIZED,"global self command must reject expiry after audit and before commit")?;
            ensure!(!reply.headers.contains_key(header::SET_COOKIE),"expired self command cleared or replaced browser credentials despite rollback");
            ensure!(failed_password_proof_snapshot(&database.pool,fixture.memberless.id).await?==before,
                "audit-wait expiry partially committed password, account generation, credential revocations or successful audit");
        }
        Ok(())
    }.await;
    database.finish(assertions).await
}
