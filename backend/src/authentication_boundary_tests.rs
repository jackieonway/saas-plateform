//! Phase 7 port contracts use real HTTP handlers and only owned disposable databases.
//! Only IDENTITY_PHASE07_TEST_DATABASE_URL is read; no dotenv or existing database is migrated.
use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
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
use crate::error::AppError;
use crate::platform_accounts::{
    AccountAdministrationPort, AccountCredential, AccountFuture, AccountMutationTx,
    AccountSnapshot, AuthenticationPort, CreatedAccount, CredentialHash, LoginIdentifier,
    NewAccount, PasswordChange, PreparedPassword,
};
use crate::state::AppState;
use crate::util::{hash, password, uuid7};

const SECRET: &str = "identity-phase07-owned-fixture-signing-key";
const PASSWORD: &str = "Owned-Phase07-Fixture-Password9!";
const NEW_PASSWORD: &str = "Owned-Phase07-Replacement-Password8!";
const CLIENT_SECRET: &str = "phase07-owned-client-fixture-not-a-deployment-secret";
const REDIRECT: &str = "https://phase07.example.invalid/callback";

struct Database {
    pool: PgPool,
    admin: PgPool,
    name: String,
}

impl Database {
    async fn create() -> Result<Self> {
        let raw = std::env::var("IDENTITY_PHASE07_TEST_DATABASE_URL")
            .map_err(|_| anyhow!("explicit IDENTITY_PHASE07_TEST_DATABASE_URL is required"))?;
        let url = reqwest::Url::parse(&raw)
            .map_err(|_| anyhow!("invalid isolated Phase 7 database configuration"))?;
        let port = url
            .port()
            .ok_or_else(|| anyhow!("Phase 7 administration requires an explicit port"))?;
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
                "Phase 7 administration requires an explicit loopback host, port and account without redirection");
        let options = PgConnectOptions::from_str(&raw)
            .map_err(|_| anyhow!("invalid isolated Phase 7 database configuration"))?;
        ensure!(
            options.get_database() == Some("postgres")
                && loopback(options.get_host())
                && options.get_port() == port
                && !options.get_username().is_empty(),
            "Phase 7 administration must explicitly select postgres"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("isolated Phase 7 administration connection failed"))?;
        let name = format!("identity_phase07_{}", uuid7::new_uuid().simple());
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
                return Err(anyhow!("owned Phase 7 database connection failed"));
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
        let suffix = self.name.strip_prefix("identity_phase07_");
        ensure!(
            suffix.is_some_and(|value| value.len() == 32
                && value.bytes().all(|ch| ch.is_ascii_hexdigit())
                && Uuid::parse_str(value).is_ok_and(uuid7::is_uuid_v7)),
            "refusing cleanup outside an owned Phase 7 database"
        );
        let closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let removed = sqlx::query(&format!("DROP DATABASE \"{}\" WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await;
        self.admin.close().await;
        eprintln!(
            "identity_phase07_cleanup database={} assertions_ok={} database_removed={} pool_closed={closed}",
            self.name, assertions.is_ok(), removed.is_ok()
        );
        assertions?;
        removed?;
        ensure!(closed, "owned Phase 7 pool did not close");
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
    retry_after: Option<String>,
}

async fn request(
    app: &Router,
    method: Method,
    path: &str,
    bearer: Option<&str>,
    content_type: &str,
    body: String,
) -> Result<Reply> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, content_type)
        .header("x-request-id", uuid7::new_uuid().to_string())
        .extension(ConnectInfo("192.0.2.37:30037".parse::<SocketAddr>()?));
    if let Some(token) = bearer {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = tokio::time::timeout(
        Duration::from_secs(25),
        app.clone().oneshot(request.body(Body::from(body))?),
    )
    .await
    .map_err(|_| anyhow!("owned Phase 7 HTTP request timed out"))??;
    let status = response.status();
    let retry_after = response
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok(Reply {
        status,
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
        retry_after,
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
    sqlx::query("INSERT INTO users(id,username,password_hash,is_super_admin,real_name) VALUES($1,$2,$3,$4,'Owned Phase 7 account')")
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
    sqlx::query("INSERT INTO applications(id,name,code,allowed_member_types) VALUES($1,'Owned Phase 7 application',$2,$3)")
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
    let public_id = format!("phase07-{}", id.simple());
    let grants = if mode == "MACHINE" {
        "client_credentials"
    } else {
        "password,refresh_token,authorization_code,urn:ietf:params:oauth:grant-type:device_code"
    };
    sqlx::query("INSERT INTO oauth_clients(id,client_id,client_name,client_type,client_secret_hash,subject_mode,allowed_member_types,application_id,grant_types,scopes,redirect_uris) VALUES($1,$2,'Owned Phase 7 client','CONFIDENTIAL',$3,$4,$5,$6,$7,'profile',$8)")
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
        let tenant = uuid7::new_uuid();
        sqlx::query("INSERT INTO tenants(id,name,code,is_platform) VALUES($1,'Owned Phase 7 tenant','admin-tenant',true)")
            .bind(tenant).execute(pool).await?;
        let root = uuid7::new_uuid();
        sqlx::query(
            "INSERT INTO organizations(id,tenant_id,name) VALUES($1,$2,'Owned Phase 7 root')",
        )
        .bind(root)
        .bind(tenant)
        .execute(pool)
        .await?;
        let employee_org = uuid7::new_uuid();
        let customer_org = uuid7::new_uuid();
        for (org, name) in [
            (employee_org, "Owned Phase 7 second employee"),
            (customer_org, "Owned Phase 7 customer"),
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
        let admin = account(pool, "phase07-admin", &encoded, true).await?;
        let subject = account(pool, "phase07-subject", &encoded, false).await?;
        let unrelated = account(pool, "phase07-unrelated", &encoded, false).await?;
        let admin_member = member(pool, admin.id, root, "EMPLOYEE").await?;
        let employees = [
            member(pool, subject.id, root, "EMPLOYEE").await?,
            member(pool, subject.id, employee_org, "EMPLOYEE").await?,
        ];
        let customer = member(pool, subject.id, customer_org, "CUSTOMER").await?;
        let unrelated_member = member(pool, unrelated.id, root, "EMPLOYEE").await?;
        let employee_application_code = "phase07-employee".to_string();
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
            sqlx::query("INSERT INTO menus(id,application_id,type,title,permission,path) VALUES($1,$2,'MENU','Owned Phase 7 administrative permission',$3,$4)")
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
        let customer_app = application(
            pool,
            tenant,
            &[customer_org],
            "phase07-customer",
            "CUSTOMER",
        )
        .await?;
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
                "postgres://unused/identity-phase07".into(),
            ),
            ("JWT_SECRET".into(), SECRET.into()),
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

struct MemoryAccountTransaction {
    id: Uuid,
    encoded: String,
    version: i64,
    revocations: usize,
    snapshot_checks: usize,
}

impl AccountMutationTx for MemoryAccountTransaction {
    fn lock_account(&mut self, id: Uuid) -> AccountFuture<'_, AccountSnapshot> {
        Box::pin(async move {
            if id != self.id {
                return Err(AppError::not_found("owned fake account not found"));
            }
            Ok(AccountSnapshot {
                id,
                account_version: self.version,
                credential: CredentialHash::new(self.encoded.clone()),
            })
        })
    }

    fn credential_matches<'a>(
        &'a mut self,
        _credential: &'a AccountCredential,
    ) -> AccountFuture<'a, bool> {
        self.snapshot_checks += 1;
        // A deliberately rejected repository proof must flow back unchanged.
        Box::pin(async { Ok(false) })
    }

    fn create<'a>(
        &'a mut self,
        account: NewAccount<'a>,
        _password: &'a PreparedPassword,
    ) -> AccountFuture<'a, CreatedAccount> {
        Box::pin(async move {
            Ok(CreatedAccount {
                id: account.id,
                username: account.username.into(),
            })
        })
    }

    fn set_status<'a>(&'a mut self, id: Uuid, _status: &'a str) -> AccountFuture<'a, bool> {
        Box::pin(async move { Ok(id == self.id) })
    }

    fn revoke_all<'a>(
        &'a mut self,
        id: Uuid,
        password: Option<&'a PreparedPassword>,
    ) -> AccountFuture<'a, u64> {
        Box::pin(async move {
            if id != self.id {
                return Err(AppError::not_found("owned fake account not found"));
            }
            self.revocations += 1;
            self.version += 1;
            if let Some(password) = password {
                self.encoded = password.encoded().to_owned();
            }
            Ok(2)
        })
    }

    fn account_version(&mut self, id: Uuid) -> AccountFuture<'_, i64> {
        Box::pin(async move {
            if id != self.id {
                return Err(AppError::not_found("owned fake account not found"));
            }
            Ok(self.version)
        })
    }
}

#[tokio::test]
async fn phase07_real_account_use_case_accepts_a_non_postgres_transaction_capability() -> Result<()>
{
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://phase07-owned@127.0.0.1:1/phase07-never-connect")?;
    let config = Config::from_values(&HashMap::from([
        ("APP_ENV".into(), "test".into()),
        (
            "DATABASE_URL".into(),
            "postgres://unused/phase07-never-connect".into(),
        ),
        ("JWT_SECRET".into(), SECRET.into()),
    ]))?;
    let state = AppState::new(pool.clone(), Arc::new(config));
    let id = uuid7::new_uuid();
    let encoded = password::hash_password(PASSWORD)?;
    let credential = AccountCredential::new(
        id,
        "phase07-capability".into(),
        "Owned fake account".into(),
        "ENABLED".into(),
        41,
        CredentialHash::new(encoded.clone()),
    );
    ensure!(
        state.authentication.verify(&credential, PASSWORD).await?,
        "real authentication rejected valid isolated credential"
    );
    let prepared = state.accounts.prepare_password(NEW_PASSWORD).await?;
    let mut tx = MemoryAccountTransaction {
        id,
        encoded,
        version: 41,
        revocations: 0,
        snapshot_checks: 0,
    };
    let changed = state
        .accounts
        .replace_password(&mut tx, id, &prepared, Some(&credential))
        .await?;
    ensure!(
        changed.before_version == 41
            && changed.after_version == 42
            && changed.revoked_sessions == 2
            && tx.version == 42
            && tx.revocations == 1,
        "account use case cannot run against its explicit fake transaction capability"
    );
    let rejected = state
        .accounts
        .replace_password(&mut tx, id, &prepared, Some(&credential))
        .await;
    ensure!(
        matches!(rejected, Err(AppError::Conflict(_))) && tx.version == 42 && tx.revocations == 1,
        "stale preverified password caused credential mutation before conflict"
    );
    ensure!(
        !state
            .authentication
            .credential_matches(&mut tx, &credential)
            .await?
            && tx.snapshot_checks == 1,
        "authentication bypassed the injected transaction proof capability"
    );
    ensure!(
        pool.size() == 0,
        "account use case opened an independent PostgreSQL connection"
    );
    pool.close().await;
    Ok(())
}

struct FakeAuthentication {
    inner: Arc<dyn AuthenticationPort>,
    // 0 accepts, 1 rejects, 2 is unavailable, 3 is capacity-limited.
    mode: AtomicUsize,
    candidates_calls: AtomicUsize,
    username_calls: AtomicUsize,
    verify_calls: AtomicUsize,
    locked_checks: AtomicUsize,
}

impl FakeAuthentication {
    fn new(inner: Arc<dyn AuthenticationPort>) -> Self {
        Self {
            inner,
            mode: AtomicUsize::new(0),
            candidates_calls: AtomicUsize::new(0),
            username_calls: AtomicUsize::new(0),
            verify_calls: AtomicUsize::new(0),
            locked_checks: AtomicUsize::new(0),
        }
    }
}

impl AuthenticationPort for FakeAuthentication {
    fn candidates<'a>(
        &'a self,
        identifier: LoginIdentifier<'a>,
    ) -> AccountFuture<'a, Vec<AccountCredential>> {
        self.candidates_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.candidates(identifier)
    }

    fn by_username<'a>(
        &'a self,
        username: &'a str,
    ) -> AccountFuture<'a, Option<AccountCredential>> {
        self.username_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.by_username(username)
    }

    fn by_user_id(&self, id: Uuid) -> AccountFuture<'_, AccountCredential> {
        self.inner.by_user_id(id)
    }

    fn verify<'a>(
        &'a self,
        _credential: &'a AccountCredential,
        _password: &'a str,
    ) -> AccountFuture<'a, bool> {
        self.verify_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            match self.mode.load(Ordering::SeqCst) {
                0 => Ok(true),
                1 => Ok(false),
                2 => Err(AppError::Unavailable),
                _ => Err(AppError::RateLimited(7)),
            }
        })
    }

    fn credential_matches<'a>(
        &'a self,
        tx: &'a mut dyn AccountMutationTx,
        credential: &'a AccountCredential,
    ) -> AccountFuture<'a, bool> {
        self.locked_checks.fetch_add(1, Ordering::SeqCst);
        self.inner.credential_matches(tx, credential)
    }
}

struct UnavailableAccounts {
    inner: Arc<dyn AccountAdministrationPort>,
    creates: AtomicUsize,
    replacements: AtomicUsize,
    statuses: AtomicUsize,
    revocations: AtomicUsize,
}

impl UnavailableAccounts {
    fn new(inner: Arc<dyn AccountAdministrationPort>) -> Self {
        Self {
            inner,
            creates: AtomicUsize::new(0),
            replacements: AtomicUsize::new(0),
            statuses: AtomicUsize::new(0),
            revocations: AtomicUsize::new(0),
        }
    }
}

impl AccountAdministrationPort for UnavailableAccounts {
    fn validate_password(&self, password: &str) -> std::result::Result<(), AppError> {
        self.inner.validate_password(password)
    }

    fn prepare_password<'a>(&'a self, password: &'a str) -> AccountFuture<'a, PreparedPassword> {
        self.inner.prepare_password(password)
    }

    fn create<'a>(
        &'a self,
        _tx: &'a mut dyn AccountMutationTx,
        _account: NewAccount<'a>,
        _password: &'a PreparedPassword,
    ) -> AccountFuture<'a, CreatedAccount> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(AppError::Unavailable) })
    }

    fn replace_password<'a>(
        &'a self,
        _tx: &'a mut dyn AccountMutationTx,
        _id: Uuid,
        _password: &'a PreparedPassword,
        _expected: Option<&'a AccountCredential>,
    ) -> AccountFuture<'a, PasswordChange> {
        self.replacements.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(AppError::Unavailable) })
    }

    fn set_status<'a>(
        &'a self,
        _tx: &'a mut dyn AccountMutationTx,
        _id: Uuid,
        _status: &'a str,
    ) -> AccountFuture<'a, bool> {
        self.statuses.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(AppError::Unavailable) })
    }

    fn revoke_all<'a>(
        &'a self,
        _tx: &'a mut dyn AccountMutationTx,
        _id: Uuid,
    ) -> AccountFuture<'a, u64> {
        self.revocations.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(AppError::Unavailable) })
    }
}

async fn account_state_snapshot(pool: &PgPool) -> Result<Value> {
    // Compared only in memory. Neither PHC material nor token-bearing rows are printed.
    Ok(sqlx::query_scalar("SELECT jsonb_build_object( \
        'users',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY id),'[]'::jsonb) FROM users t), \
        'memberships',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY id),'[]'::jsonb) FROM memberships t), \
        'sessions',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY id),'[]'::jsonb) FROM platform_sessions t), \
        'refresh',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY id),'[]'::jsonb) FROM platform_session_refresh_tokens t), \
        'oauth',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY id),'[]'::jsonb) FROM oauth_tokens t), \
        'codes',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY id),'[]'::jsonb) FROM oauth_authorization_codes t), \
        'devices',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY id),'[]'::jsonb) FROM oauth_device_codes t), \
        'audit',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY id),'[]'::jsonb) FROM audit_logs t), \
        'notifications',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY id),'[]'::jsonb) FROM notifications t))")
        .fetch_one(pool).await?)
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE07_TEST_DATABASE_URL and disposable database creation"]
async fn phase07_fake_authentication_replaces_both_password_consumers_without_granting_membership(
) -> Result<()> {
    let database = Database::create().await?;
    let assertions = async {
        let mut fixture = Fixture::create(&database.pool).await?;
        let fake = Arc::new(FakeAuthentication::new(fixture.state.authentication.clone()));
        fixture.state.authentication = fake.clone();
        let app = fixture.app();
        let wrong_password = "Deliberately-not-the-stored-Phase07-password!";
        let encoded:String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1")
            .bind(fixture.subject.id).fetch_one(&database.pool).await?;
        ensure!(!password::verify_password(&encoded,wrong_password),"fake acceptance positive control has a valid local password");
        let accepted = platform(&app,&fixture.subject,fixture.employees[0],wrong_password).await?;
        let oauth_reply = oauth(&app,&fixture.legacy,&fixture.subject,fixture.employees[0],wrong_password).await?;
        let _accepted = token(&database.pool,oauth_reply,fixture.employees[0],&fixture.legacy).await?;
        ensure!(fake.candidates_calls.load(Ordering::SeqCst)>0 && fake.username_calls.load(Ordering::SeqCst)>0 &&
                fake.verify_calls.load(Ordering::SeqCst)>=2 && fake.locked_checks.load(Ordering::SeqCst)>0,
                "console or ordinary OAuth bypassed the injected authentication port");
        expect(&call(&app,Method::POST,"/api/auth/context",Some(&accepted.ticket),
                     json!({"membershipId":fixture.customer.id,"organizationId":fixture.customer.org})).await?,
               StatusCode::FORBIDDEN,"fake account authentication cannot manufacture employee eligibility")?;
        for (username,has_customer) in [("phase07-customer-only",true),("phase07-no-membership",false)] {
            let additional = account(&database.pool,username,&encoded,false).await?;
            if has_customer { member(&database.pool,additional.id,fixture.customer.org,"CUSTOMER").await?; }
            expect(&login(&app,&additional,wrong_password).await?,StatusCode::FORBIDDEN,
                   "old console still requires its real employee membership")?;
        }
        let stored_tokens:i64 = sqlx::query_scalar("SELECT count(*) FROM oauth_tokens")
            .fetch_one(&database.pool).await?;
        let successful_logins:i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='LOGIN'")
            .fetch_one(&database.pool).await?;
        for (mode,api_status,oauth_status,oauth_error) in [
            (1,StatusCode::UNAUTHORIZED,StatusCode::BAD_REQUEST,"invalid_grant"),
            (2,StatusCode::SERVICE_UNAVAILABLE,StatusCode::SERVICE_UNAVAILABLE,"temporarily_unavailable"),
            (3,StatusCode::TOO_MANY_REQUESTS,StatusCode::TOO_MANY_REQUESTS,"temporarily_unavailable"),
        ] {
            fake.mode.store(mode,Ordering::SeqCst);
            let console = login(&app,&fixture.subject,PASSWORD).await?;
            expect(&console,api_status,"injected authentication rejection or dependency failure")?;
            ensure!(console.body.get("ticket").is_none() && console.body.get("accessToken").is_none(),
                    "failed authentication returned a partial platform credential");
            let ordinary = oauth(&app,&fixture.legacy,&fixture.subject,fixture.employees[0],PASSWORD).await?;
            expect(&ordinary,oauth_status,"legacy OAuth must preserve dependency failure semantics")?;
            ensure!(ordinary.body["error"] == oauth_error && ordinary.body.get("access_token").is_none(),
                    "ordinary OAuth converted dependency failure into success or wrong-password denial");
            if mode==3 { ensure!(console.retry_after.as_deref()==Some("7") && ordinary.retry_after.as_deref()==Some("7"),
                                "port capacity rejection lost Retry-After"); }
        }
        let after:(i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM oauth_tokens),(SELECT count(*) FROM audit_logs WHERE action='LOGIN')")
            .fetch_one(&database.pool).await?;
        ensure!(after==(stored_tokens,successful_logins),"rejected port operation persisted credentials or successful login audit");
        Ok(())
    }.await;
    database.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE07_TEST_DATABASE_URL and disposable database creation"]
async fn phase07_fake_account_administration_failures_leave_original_transactions_unchanged(
) -> Result<()> {
    let database = Database::create().await?;
    let assertions = async {
        let mut fixture = Fixture::create(&database.pool).await?;
        let old = issued(&database.pool,&fixture).await?;
        let baseline = account_state_snapshot(&database.pool).await?;
        let fake = Arc::new(UnavailableAccounts::new(fixture.state.accounts.clone()));
        fixture.state.accounts = fake.clone();
        let app = fixture.app();
        let requests = [
            (Method::POST,"/api/users".to_string(),old.admin.access.as_str(),json!({"username":"phase07-failed-create","password":NEW_PASSWORD})),
            (Method::POST,format!("/api/users/{}/reset-password",fixture.subject.id),old.admin.access.as_str(),json!({"newPassword":NEW_PASSWORD})),
            (Method::PUT,"/api/auth/password".to_string(),old.employees[0].access.as_str(),json!({"oldPassword":PASSWORD,"newPassword":NEW_PASSWORD})),
            (Method::PUT,format!("/api/users/{}/status",fixture.subject.id),old.admin.access.as_str(),json!({"status":"DISABLED"})),
            (Method::POST,"/api/users/batch-status".to_string(),old.admin.access.as_str(),json!({"userIds":[fixture.subject.id,fixture.unrelated.id],"status":"DISABLED"})),
            (Method::POST,"/api/oauth/tokens/kick".to_string(),old.admin.access.as_str(),json!({"userId":fixture.subject.id})),
        ];
        for (method,path,bearer,body) in requests {
            expect(&call(&app,method,&path,Some(bearer),body).await?,StatusCode::SERVICE_UNAVAILABLE,
                   "real management route must use its replaceable account port")?;
            ensure!(account_state_snapshot(&database.pool).await?==baseline,
                    "account port failure partially committed account, credentials, notification or audit state");
        }
        ensure!(fake.creates.load(Ordering::SeqCst)==1 && fake.replacements.load(Ordering::SeqCst)==2 &&
                fake.statuses.load(Ordering::SeqCst)==2 && fake.revocations.load(Ordering::SeqCst)==1,
                "one management consumer bypassed the injected account administration port");
        expect(&call(&app,Method::GET,"/api/auth/profile",Some(&old.employees[0].access),Value::Null).await?,
               StatusCode::OK,"rolled-back account operation retained original employee session")?;
        Ok(())
    }.await;
    database.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE07_TEST_DATABASE_URL and disposable database creation"]
async fn phase07_real_account_reset_preserves_protocols_and_revokes_all_human_contexts(
) -> Result<()> {
    let database = Database::create().await?;
    let assertions = async {
        let fixture = Fixture::create(&database.pool).await?;
        let app = fixture.app();
        let old = issued(&database.pool,&fixture).await?;
        let members = membership_snapshot(&database.pool,fixture.subject.id).await?;
        let before: i64 = sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
            .bind(fixture.subject.id).fetch_one(&database.pool).await?;
        let reset = call(&app,Method::POST,&format!("/api/users/{}/reset-password",fixture.subject.id),
                         Some(&old.admin.access),json!({"newPassword":NEW_PASSWORD})).await?;
        expect(&reset,StatusCode::OK,"real administrative account port reset")?;
        let (after,encoded): (i64,String) = sqlx::query_as("SELECT session_version,password_hash FROM users WHERE id=$1")
            .bind(fixture.subject.id).fetch_one(&database.pool).await?;
        ensure!(after == before+1,"reset must advance exactly one account generation");
        ensure!(password::verify_password(&encoded,NEW_PASSWORD) && !password::verify_password(&encoded,PASSWORD),
                "reset did not replace the actual account credential");
        ensure!(membership_snapshot(&database.pool,fixture.subject.id).await? == members,
                "global account reset changed concrete membership identity or status");
        revoked(&app,&fixture,&old).await?;
        expect(&login(&app,&fixture.subject,PASSWORD).await?,StatusCode::UNAUTHORIZED,"old password after reset")?;
        let replacement = platform(&app,&fixture.subject,fixture.employees[0],NEW_PASSWORD).await?;
        expect(&call(&app,Method::GET,"/api/auth/profile",Some(&replacement.access),Value::Null).await?,
               StatusCode::OK,"fresh password can establish fresh employee context")?;
        let audit: (i64,bool) = sqlx::query_as("SELECT count(*),COALESCE(bool_or(detail LIKE $2 OR detail LIKE $3),false) FROM audit_logs WHERE action='RESET_PASSWORD' AND detail LIKE $1")
            .bind(format!("%{}%",fixture.subject.id)).bind(format!("%{NEW_PASSWORD}%")).bind(format!("%{encoded}%"))
            .fetch_one(&database.pool).await?;
        ensure!(audit == (1,false),"reset audit missing, duplicated or contains credential material");
        Ok(())
    }.await;
    database.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE07_TEST_DATABASE_URL and disposable database creation"]
async fn phase07_global_status_port_never_resurrects_employee_customer_or_legacy_credentials(
) -> Result<()> {
    let database = Database::create().await?;
    let assertions = async {
        let fixture = Fixture::create(&database.pool).await?;
        let app = fixture.app();
        let old = issued(&database.pool, &fixture).await?;
        let members = membership_snapshot(&database.pool, fixture.subject.id).await?;
        let before: (i64, String) =
            sqlx::query_as("SELECT session_version,password_hash FROM users WHERE id=$1")
                .bind(fixture.subject.id)
                .fetch_one(&database.pool)
                .await?;
        for status in ["DISABLED", "DISABLED", "ENABLED"] {
            expect(
                &call(
                    &app,
                    Method::PUT,
                    &format!("/api/users/{}/status", fixture.subject.id),
                    Some(&old.admin.access),
                    json!({"status":status}),
                )
                .await?,
                StatusCode::OK,
                "real global account status command",
            )?;
            let actual: (i64, String, String) = sqlx::query_as(
                "SELECT session_version,password_hash,status FROM users WHERE id=$1",
            )
            .bind(fixture.subject.id)
            .fetch_one(&database.pool)
            .await?;
            ensure!(
                actual == (before.0 + 1, before.1.clone(), status.into()),
                "global status changed password or advanced the generation more than once"
            );
            revoked(&app, &fixture, &old).await?;
            ensure!(
                membership_snapshot(&database.pool, fixture.subject.id).await? == members,
                "global account status command rewrote employee/customer membership state"
            );
        }
        let fresh = platform(&app, &fixture.subject, fixture.employees[0], PASSWORD).await?;
        expect(
            &call(
                &app,
                Method::GET,
                "/api/auth/profile",
                Some(&fresh.access),
                Value::Null,
            )
            .await?,
            StatusCode::OK,
            "reenabled account requires fresh context",
        )?;
        Ok(())
    }
    .await;
    database.finish(assertions).await
}
