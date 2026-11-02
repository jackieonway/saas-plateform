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
    AccountSnapshot, AuthenticationPort, CreatedAccount, CredentialHash, NewAccount,
    PasswordChange, PreparedPassword,
};
use crate::state::AppState;
use crate::util::{password, uuid7};

const SECRET: &str = "identity-phase07-owned-fixture-signing-key";
const PASSWORD: &str = "Owned-Phase07-Fixture-Password9!";
const NEW_PASSWORD: &str = "Owned-Phase07-Replacement-Password8!";

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
                url.fragment().is_none() &&
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
        let expected_cluster = std::env::var("IDENTITY_PHASE07_TEST_CLUSTER_ID")
            .map_err(|_| anyhow!("explicit owned Phase 7 cluster identifier required"))?;
        ensure!(
            expected_cluster
                .parse::<u64>()
                .is_ok_and(|value| value > 0 && value.to_string() == expected_cluster),
            "invalid owned Phase 7 cluster identifier"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options.clone())
            .await
            .map_err(|_| anyhow!("isolated Phase 7 administration connection failed"))?;
        let observed_cluster: String =
            sqlx::query_scalar("SELECT system_identifier::text FROM pg_control_system()")
                .fetch_one(&admin)
                .await?;
        ensure!(
            observed_cluster == expected_cluster,
            "refusing another PostgreSQL cluster"
        );
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
    let _ = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok(Reply { status })
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

fn expect(reply: &Reply, status: StatusCode, label: &str) -> Result<()> {
    // Never include a token-bearing response body, plaintext password or PHC hash.
    ensure!(
        reply.status == status,
        "{label}: expected {status}, received {}",
        reply.status
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
}

impl UnavailableAccounts {
    fn new(inner: Arc<dyn AccountAdministrationPort>) -> Self {
        Self {
            inner,
            creates: AtomicUsize::new(0),
            replacements: AtomicUsize::new(0),
            statuses: AtomicUsize::new(0),
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
}

struct Fixture {
    state: AppState,
    subject: Uuid,
    unrelated: Uuid,
    tenant: Uuid,
    root: Uuid,
    other_org: Uuid,
    admin: String,
    employee: String,
    other_employee: String,
    unaffected: String,
}
impl Fixture {
    async fn create(pool: &PgPool) -> Result<Self> {
        let tenant = uuid7::new_uuid();
        let root = uuid7::new_uuid();
        let other_org = uuid7::new_uuid();
        sqlx::query("INSERT INTO tenants(id,name,code,is_platform) VALUES($1,'Canonical account fixture','canonical-account',true)").bind(tenant).execute(pool).await?;
        sqlx::query(
            "INSERT INTO organizations(id,tenant_id,name) VALUES($1,$2,'Canonical account root')",
        )
        .bind(root)
        .bind(tenant)
        .execute(pool)
        .await?;
        sqlx::query("INSERT INTO organizations(id,tenant_id,parent_id,name) VALUES($1,$2,$3,'Canonical account branch')").bind(other_org).bind(tenant).bind(root).execute(pool).await?;
        let encoded = password::hash_password(PASSWORD)?;
        let mut accounts = Vec::new();
        for (username, admin) in [
            ("canonical-account-admin", true),
            ("canonical-account-subject", false),
            ("canonical-account-independent", false),
        ] {
            let id = uuid7::new_uuid();
            sqlx::query("INSERT INTO users(id,username,password_hash,is_super_admin,session_version) VALUES($1,$2,$3,$4,1)")
                .bind(id).bind(username).bind(&encoded).bind(admin).execute(pool).await?;
            sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES($1,$2,$3,'EMPLOYEE')")
                .bind(uuid7::new_uuid()).bind(id).bind(root).execute(pool).await?;
            accounts.push(id);
        }
        let (administrator, subject, unrelated) = (accounts[0], accounts[1], accounts[2]);
        for (org, kind) in [(other_org, "EMPLOYEE"), (root, "CUSTOMER")] {
            sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES($1,$2,$3,$4)")
                .bind(uuid7::new_uuid()).bind(subject).bind(org).bind(kind).execute(pool).await?;
        }
        let application = uuid7::new_uuid();
        let menu = uuid7::new_uuid();
        sqlx::query("INSERT INTO applications(id,name,code,allowed_member_types) VALUES($1,'Canonical management','canonical-account-management',ARRAY['EMPLOYEE'])")
            .bind(application).execute(pool).await?;
        sqlx::query("INSERT INTO menus(id,application_id,type,title,permission,path) VALUES($1,$2,'MENU','Account management','system:user','/system/users')")
            .bind(menu).bind(application).execute(pool).await?;
        sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2)")
            .bind(tenant)
            .bind(application)
            .execute(pool)
            .await?;
        sqlx::query(
            "INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2)",
        )
        .bind(root)
        .bind(application)
        .execute(pool)
        .await?;
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
        let config = Config::from_values(&HashMap::from([
            ("APP_ENV".into(), "test".into()),
            (
                "DATABASE_URL".into(),
                "postgres://unused/canonical-account".into(),
            ),
            ("JWT_SECRET".into(), SECRET.into()),
            ("MEMBERSHIP_LIFECYCLE_ENABLED".into(), "true".into()),
            ("CUSTOMER_MEMBERSHIPS_ENABLED".into(), "true".into()),
            ("DUAL_MEMBERSHIPS_ENABLED".into(), "true".into()),
        ]))?;
        let state = crate::sso_test_fixture::state(pool.clone(), Arc::new(config));
        let admin =
            crate::sso_test_fixture::token(pool, administrator, tenant, root, SECRET).await?;
        let employee = crate::sso_test_fixture::token(pool, subject, tenant, root, SECRET).await?;
        let other_employee =
            crate::sso_test_fixture::token(pool, subject, tenant, other_org, SECRET).await?;
        let unaffected =
            crate::sso_test_fixture::token(pool, unrelated, tenant, root, SECRET).await?;
        Ok(Self {
            state,
            subject,
            unrelated,
            tenant,
            root,
            other_org,
            admin,
            employee,
            other_employee,
            unaffected,
        })
    }
    fn app(&self) -> Router {
        router(self.state.clone())
    }
}
async fn membership_snapshot(pool: &PgPool, user: Uuid) -> Result<Value> {
    Ok(sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(m) ORDER BY id),'[]'::jsonb) FROM memberships m WHERE user_id=$1")
        .bind(user).fetch_one(pool).await?)
}
async fn account_state_snapshot(pool: &PgPool) -> Result<Value> {
    // Compare in memory; never emit PHC, bearer, Cookie or ciphertext values.
    Ok(sqlx::query_scalar("SELECT jsonb_build_object( \
        'users',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY id),'[]'::jsonb) FROM users t), \
        'members',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY id),'[]'::jsonb) FROM memberships t), \
        'sessions',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY id),'[]'::jsonb) FROM platform_sessions t), \
        'grants',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY id),'[]'::jsonb) FROM auth_grants t), \
        'access',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY id),'[]'::jsonb) FROM auth_access_tokens t), \
        'audit',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY id),'[]'::jsonb) FROM audit_logs t), \
        'notifications',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY id),'[]'::jsonb) FROM notifications t))")
        .fetch_one(pool).await?)
}
async fn assert_subject_revoked(app: &Router, fixture: &Fixture) -> Result<()> {
    for bearer in [&fixture.employee, &fixture.other_employee] {
        expect(
            &call(
                app,
                Method::GET,
                "/api/auth/profile",
                Some(bearer),
                Value::Null,
            )
            .await?,
            StatusCode::UNAUTHORIZED,
            "old subject context",
        )?;
    }
    for bearer in [&fixture.admin, &fixture.unaffected] {
        expect(
            &call(
                app,
                Method::GET,
                "/api/auth/profile",
                Some(bearer),
                Value::Null,
            )
            .await?,
            StatusCode::OK,
            "independent account context",
        )?;
    }
    Ok(())
}

#[tokio::test]
async fn canonical_platform_routes_do_not_consume_local_password_authentication() -> Result<()> {
    let pool = PgPoolOptions::new().connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")?;
    let config = Config::from_values(&HashMap::from([
        ("APP_ENV".into(), "test".into()),
        (
            "DATABASE_URL".into(),
            "postgres://unused:unused@127.0.0.1:1/unused".into(),
        ),
        ("JWT_SECRET".into(), SECRET.into()),
    ]))?;
    let mut state = AppState::new(pool.clone(), Arc::new(config));
    let fake = Arc::new(FakeAuthentication::new(state.authentication.clone()));
    state.authentication = fake.clone();
    let app = router(state);
    for path in [
        "/api/auth/login",
        "/api/auth/context",
        "/oauth/token",
        "/oauth/device/code",
    ] {
        expect(
            &call(
                &app,
                Method::POST,
                path,
                None,
                json!({"username":"owned","password":PASSWORD}),
            )
            .await?,
            StatusCode::NOT_FOUND,
            "retired password or issuer route",
        )?;
    }
    ensure!(
        fake.candidates_calls.load(Ordering::SeqCst) == 0
            && fake.username_calls.load(Ordering::SeqCst) == 0
            && fake.verify_calls.load(Ordering::SeqCst) == 0
            && pool.size() == 0,
        "removed authentication route still consumed a local credential or database connection"
    );
    pool.close().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE07_TEST_DATABASE_URL and disposable database creation"]
async fn canonical_account_administration_failure_keeps_original_transactions() -> Result<()> {
    let database = Database::create().await?;
    let assertions = async {
        let mut fixture = Fixture::create(&database.pool).await?;
        let fake = Arc::new(UnavailableAccounts::new(fixture.state.accounts.clone()));
        fixture.state.accounts = fake.clone();
        let app = fixture.app();
        let baseline = account_state_snapshot(&database.pool).await?;
        for (method, path, bearer, body) in [
            (
                Method::POST,
                "/api/users".to_string(),
                fixture.admin.as_str(),
                json!({"username":"canonical-failed-create","password":NEW_PASSWORD}),
            ),
            (
                Method::POST,
                format!("/api/users/{}/reset-password", fixture.subject),
                fixture.admin.as_str(),
                json!({"newPassword":NEW_PASSWORD}),
            ),
            (
                Method::PUT,
                "/api/auth/password".to_string(),
                fixture.employee.as_str(),
                json!({"oldPassword":PASSWORD,"newPassword":NEW_PASSWORD}),
            ),
            (
                Method::PUT,
                format!("/api/users/{}/status", fixture.subject),
                fixture.admin.as_str(),
                json!({"status":"DISABLED"}),
            ),
            (
                Method::POST,
                "/api/users/batch-status".to_string(),
                fixture.admin.as_str(),
                json!({"userIds":[fixture.subject,fixture.unrelated],"status":"DISABLED"}),
            ),
        ] {
            expect(
                &call(&app, method, &path, Some(bearer), body).await?,
                StatusCode::SERVICE_UNAVAILABLE,
                "account port dependency failure",
            )?;
            ensure!(
                account_state_snapshot(&database.pool).await? == baseline,
                "failed account command committed account, session, audit or notification changes"
            );
        }
        ensure!(
            fake.creates.load(Ordering::SeqCst) == 1
                && fake.replacements.load(Ordering::SeqCst) == 2
                && fake.statuses.load(Ordering::SeqCst) == 2,
            "management consumer bypassed the account port"
        );
        expect(
            &call(
                &app,
                Method::GET,
                "/api/auth/profile",
                Some(&fixture.employee),
                Value::Null,
            )
            .await?,
            StatusCode::OK,
            "original session after account failure",
        )?;
        Ok(())
    }
    .await;
    database.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE07_TEST_DATABASE_URL and disposable database creation"]
async fn canonical_account_reset_preserves_members_and_revokes_exact_subject() -> Result<()> {
    let database = Database::create().await?;
    let assertions=async {
        let fixture=Fixture::create(&database.pool).await?;
        let app=fixture.app();
        let members=membership_snapshot(&database.pool,fixture.subject).await?;
        let version:i64=sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1").bind(fixture.subject).fetch_one(&database.pool).await?;
        expect(&call(&app,Method::POST,&format!("/api/users/{}/reset-password",fixture.subject),Some(&fixture.admin),json!({"newPassword":NEW_PASSWORD})).await?,StatusCode::OK,"current administrative reset")?;
        let (after,encoded):(i64,String)=sqlx::query_as("SELECT session_version,password_hash FROM users WHERE id=$1").bind(fixture.subject).fetch_one(&database.pool).await?;
        ensure!(after==version+1 && password::verify_password(&encoded,NEW_PASSWORD) && !password::verify_password(&encoded,PASSWORD),"reset did not replace the credential and exact account generation");
        ensure!(membership_snapshot(&database.pool,fixture.subject).await?==members,"account reset changed member relationships");
        assert_subject_revoked(&app,&fixture).await?;
        let fresh=crate::sso_test_fixture::token(&database.pool,fixture.subject,fixture.tenant,fixture.root,SECRET).await?;
        expect(&call(&app,Method::GET,"/api/auth/profile",Some(&fresh),Value::Null).await?,StatusCode::OK,"fresh canonical source after reset")?;
        let audit:(i64,bool)=sqlx::query_as("SELECT count(*),COALESCE(bool_or(detail LIKE $2 OR detail LIKE $3),false) FROM audit_logs WHERE action='RESET_PASSWORD' AND detail LIKE $1")
            .bind(format!("%{}%",fixture.subject)).bind(format!("%{NEW_PASSWORD}%")).bind(format!("%{encoded}%")).fetch_one(&database.pool).await?;
        ensure!(audit==(1,false),"reset audit missing, duplicated or contains credential material");
        Ok(())
    }.await;
    database.finish(assertions).await
}

#[tokio::test]
#[ignore = "requires explicit IDENTITY_PHASE07_TEST_DATABASE_URL and disposable database creation"]
async fn canonical_account_status_does_not_restore_old_member_sessions() -> Result<()> {
    let database = Database::create().await?;
    let assertions = async {
        let fixture = Fixture::create(&database.pool).await?;
        let app = fixture.app();
        let members = membership_snapshot(&database.pool, fixture.subject).await?;
        let before: (i64, String) =
            sqlx::query_as("SELECT session_version,password_hash FROM users WHERE id=$1")
                .bind(fixture.subject)
                .fetch_one(&database.pool)
                .await?;
        for status in ["DISABLED", "DISABLED", "ENABLED"] {
            expect(
                &call(
                    &app,
                    Method::PUT,
                    &format!("/api/users/{}/status", fixture.subject),
                    Some(&fixture.admin),
                    json!({"status":status}),
                )
                .await?,
                StatusCode::OK,
                "current account status",
            )?;
            let actual: (i64, String, String) = sqlx::query_as(
                "SELECT session_version,password_hash,status FROM users WHERE id=$1",
            )
            .bind(fixture.subject)
            .fetch_one(&database.pool)
            .await?;
            ensure!(
                actual == (before.0 + 1, before.1.clone(), status.into()),
                "status command changed credential or account generation unexpectedly"
            );
            ensure!(
                membership_snapshot(&database.pool, fixture.subject).await? == members,
                "global status changed member relationships"
            );
            assert_subject_revoked(&app, &fixture).await?;
        }
        let fresh = crate::sso_test_fixture::token(
            &database.pool,
            fixture.subject,
            fixture.tenant,
            fixture.other_org,
            SECRET,
        )
        .await?;
        expect(
            &call(
                &app,
                Method::GET,
                "/api/auth/profile",
                Some(&fresh),
                Value::Null,
            )
            .await?,
            StatusCode::OK,
            "reenabled account needs a fresh canonical source",
        )?;
        Ok(())
    }
    .await;
    database.finish(assertions).await
}
