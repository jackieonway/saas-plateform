//! Server-side RP transaction regressions against the canonical schema.
//!
//! The transport is an explicit credential/proof fixture, with no public router,
//! password handler or alternate issuer. Real identity HTTPS interoperability is
//! verified by the owned integration harness. Every database is fresh and removed.
use super::{signed, KEY_A_N, KEY_A_PEM, ORIGIN};
use crate::{
    bff,
    config::Config,
    error::AppError,
    state::AppState,
    util::{hash, uuid7},
};
use anyhow::{anyhow, ensure, Result};
use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::{header, HeaderMap, Method, Request, StatusCode},
    Router,
};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool,
};
use std::{
    collections::HashMap,
    future::Future,
    net::SocketAddr,
    str::FromStr,
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicU8, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::Notify;
use tower::ServiceExt;
use uuid::Uuid;

const CONSOLE: &str = "https://console-fixture.example.invalid";
const BINDER: &str = "__Host-saas_console_login";
const COOKIE: &str = "__Host-saas_console";

struct Database {
    pool: PgPool,
    admin: PgPool,
    name: String,
    marker: String,
}
impl Database {
    async fn create() -> Result<Self> {
        let raw = std::env::var("IDENTITY_PHASE24_TEST_DATABASE_URL")
            .map_err(|_| anyhow!("explicit Phase24 administration URL required"))?;
        let expected = std::env::var("IDENTITY_PHASE24_TEST_CLUSTER_ID")
            .map_err(|_| anyhow!("explicit Phase24 cluster fingerprint required"))?;
        let options = PgConnectOptions::from_str(&raw)
            .map_err(|_| anyhow!("invalid Phase24 administration configuration"))?;
        ensure!(
            options.get_database() == Some("postgres")
                && options.get_port() == 55494
                && matches!(options.get_host(), "127.0.0.1" | "localhost" | "::1"),
            "only explicit owned 55494 loopback administration is allowed"
        );
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await?;
        let actual: String =
            sqlx::query_scalar("SELECT system_identifier::text FROM pg_control_system()")
                .fetch_one(&admin)
                .await?;
        ensure!(
            actual == expected && expected.parse::<u64>().is_ok_and(|value| value > 0),
            "owned cluster fingerprint changed"
        );
        let id = uuid7::new_uuid();
        let name = format!("identity_phase24_bff_{}", id.simple());
        let marker = format!("identity-phase24-bff/task-owned/1/{id}");
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin)
            .await?;
        sqlx::query(&format!("COMMENT ON DATABASE \"{name}\" IS '{marker}'"))
            .execute(&admin)
            .await?;
        let opened = PgPoolOptions::new()
            .max_connections(12)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(options.database(&name))
            .await;
        let pool = match opened {
            Ok(pool) => pool,
            Err(_) => {
                sqlx::query(&format!("DROP DATABASE \"{name}\" WITH(FORCE)"))
                    .execute(&admin)
                    .await?;
                admin.close().await;
                return Err(anyhow!("owned Phase24 BFF database connection failed"));
            }
        };
        let database = Self {
            pool,
            admin,
            name,
            marker,
        };
        if let Err(error) = sqlx::migrate!("./migrations").run(&database.pool).await {
            database.finish(Ok(())).await?;
            return Err(error.into());
        }
        Ok(database)
    }
    async fn finish(self, assertion: Result<()>) -> Result<()> {
        let marker: String = sqlx::query_scalar(
            "SELECT shobj_description(oid,'pg_database') FROM pg_database WHERE datname=$1",
        )
        .bind(&self.name)
        .fetch_one(&self.admin)
        .await?;
        ensure!(
            marker == self.marker && self.name.starts_with("identity_phase24_bff_"),
            "refusing foreign database cleanup"
        );
        let closed = tokio::time::timeout(Duration::from_secs(5), self.pool.close())
            .await
            .is_ok();
        let removed = sqlx::query(&format!("DROP DATABASE \"{}\" WITH(FORCE)", self.name))
            .execute(&self.admin)
            .await;
        self.admin.close().await;
        eprintln!(
            "phase24_bff_cleanup removed={} pool_closed={closed}",
            removed.is_ok()
        );
        removed?;
        ensure!(closed, "owned fixture pool remained open");
        assertion
    }
}
async fn isolated<F, Fut>(case: F) -> Result<()>
where
    F: FnOnce(PgPool) -> Fut + Send + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    let database = Database::create().await?;
    let pool = database.pool.clone();
    let result = tokio::spawn(async move { case(pool).await })
        .await
        .map_err(|_| anyhow!("owned BFF case panicked or was cancelled"))
        .and_then(|result| result);
    database.finish(result).await
}

#[derive(Clone)]
struct Source {
    access: String,
    refresh: String,
    user: Uuid,
    grant: Uuid,
    global: Uuid,
    public_sid: String,
    subject: String,
    nonce: String,
    family: Uuid,
    generation: i64,
    auth_time: i64,
    absolute: DateTime<Utc>,
}
struct Code {
    source: Source,
    challenge: String,
}
struct Gate {
    endpoint: AtomicU8,
    entered: Notify,
    release: Notify,
}
impl Gate {
    fn new() -> Self {
        Self {
            endpoint: AtomicU8::new(0),
            entered: Notify::new(),
            release: Notify::new(),
        }
    }
    fn arm(&self, endpoint: u8) {
        self.endpoint.store(endpoint, Ordering::SeqCst);
    }
    async fn reached(&self) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(3), self.entered.notified())
            .await
            .map_err(|_| anyhow!("canonical transport did not reach its response barrier"))
    }
    async fn response(&self, endpoint: u8) {
        if self
            .endpoint
            .compare_exchange(endpoint, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.entered.notify_one();
            self.release.notified().await;
        }
    }
}
struct Transport {
    pool: PgPool,
    inner: Arc<dyn bff::OidcTransport>,
    client: Uuid,
    codes: Mutex<HashMap<String, Code>>,
    refresh: Mutex<HashMap<String, Source>>,
    gate: Gate,
    refreshes: AtomicUsize,
    logout_sends: AtomicUsize,
    logout_lookups: AtomicUsize,
    lost_refresh: AtomicBool,
    lost_logout: AtomicBool,
    id_ttl: AtomicI64,
    last_id_exp: AtomicI64,
    proof_ttl: AtomicI64,
    last_proof_exp: AtomicI64,
    graft_sid: AtomicBool,
}
impl Transport {
    fn invalid() -> AppError {
        AppError::unauthorized("canonical fixture credential rejected")
    }
    fn response(status: u16, value: Value) -> Result<bff::OidcResponse, AppError> {
        Ok(bff::OidcResponse {
            status,
            body: serde_json::to_vec(&value).map_err(|_| AppError::Unavailable)?,
        })
    }
    fn lock<T>(value: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, AppError> {
        value.lock().map_err(|_| AppError::Unavailable)
    }
    fn form(request: &bff::OidcRequest, key: &str) -> Result<String, AppError> {
        let mut values = request.form.iter().filter(|(name, _)| name == key);
        let value = values.next().ok_or_else(Self::invalid)?.1.clone();
        if values.next().is_some() {
            return Err(Self::invalid());
        }
        Ok(value)
    }
    fn token_response(&self, source: &Source) -> Result<Value, AppError> {
        let now = Utc::now().timestamp();
        let exp = now + self.id_ttl.load(Ordering::SeqCst);
        self.last_id_exp.store(exp, Ordering::SeqCst);
        let sid = if self.graft_sid.load(Ordering::SeqCst) {
            hash::sha256_hex("wrong-new-source")
        } else {
            source.public_sid.clone()
        };
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some("phase10-a".into());
        header.typ = Some("JWT".into());
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(KEY_A_PEM.as_bytes())
            .map_err(|_| AppError::Unavailable)?;
        let id_token = jsonwebtoken::encode(
            &header,
            &json!({"iss":ORIGIN,"aud":self.client,"sub":source.subject,
            "sid":sid,"nonce":source.nonce,"auth_time":source.auth_time,"iat":now,"exp":exp}),
            &key,
        )
        .map_err(|_| AppError::Unavailable)?;
        Ok(
            json!({"access_token":source.access,"refresh_token":source.refresh,"token_type":"Bearer",
            "scope":"openid profile","expires_in":300,"id_token":id_token}),
        )
    }
    async fn rotate(&self, old: &str) -> Result<Source, AppError> {
        let source = Self::lock(&self.refresh)?
            .get(old)
            .cloned()
            .ok_or_else(Self::invalid)?;
        let mut tx = self.pool.begin().await?;
        let consumed = sqlx::query("UPDATE auth_refresh_tokens SET consumed_at=clock_timestamp() WHERE token_digest=$1 AND consumed_at IS NULL AND revoked_at IS NULL AND expires_at>clock_timestamp()")
            .bind(hash::sha256_hex(old)).execute(&mut *tx).await?.rows_affected();
        if consumed != 1 {
            return Err(Self::invalid());
        }
        let access = hash::sha256_hex(&uuid7::new_uuid().to_string());
        let refresh = hash::sha256_hex(&uuid7::new_uuid().to_string());
        let id = uuid7::new_uuid();
        sqlx::query("INSERT INTO auth_access_tokens(id,grant_id,token_digest,expires_at) VALUES($1,$2,$3,$4)")
            .bind(id).bind(source.grant).bind(hash::sha256_hex(&access)).bind(source.absolute).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO auth_access_token_scopes(token_id,resource_id,scope_id) SELECT $1,resource_id,scope_id FROM auth_grant_scopes WHERE grant_id=$2")
            .bind(id).bind(source.grant).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO auth_refresh_tokens(id,family_id,token_digest,generation,expires_at) VALUES($1,$2,$3,$4,$5)")
            .bind(uuid7::new_uuid()).bind(source.family).bind(hash::sha256_hex(&refresh)).bind(source.generation+1).bind(source.absolute).execute(&mut *tx).await?;
        tx.commit().await?;
        let rotated = Source {
            access,
            refresh: refresh.clone(),
            generation: source.generation + 1,
            ..source
        };
        Self::lock(&self.refresh)?.insert(refresh, rotated.clone());
        Ok(rotated)
    }
    async fn logout(
        &self,
        request: &bff::OidcRequest,
        operation: Uuid,
        lookup: bool,
    ) -> Result<bff::OidcResponse, AppError> {
        let token = Self::form(request, "token")?;
        let scope = Self::form(request, "scope")?;
        let source = Self::lock(&self.refresh)?
            .get(&token)
            .cloned()
            .ok_or_else(Self::invalid)?;
        if Self::form(request, "operation_id")? != operation.to_string()
            || Self::form(request, "sid")? != source.public_sid
            || Self::form(request, "sub")? != source.subject
        {
            return Err(Self::invalid());
        }
        let digest = hash::sha256_hex(
            &serde_json::to_string(&request.form).map_err(|_| AppError::Unavailable)?,
        );
        if lookup {
            self.logout_lookups.fetch_add(1, Ordering::SeqCst);
            let saved: Option<(String, Value)> = sqlx::query_as("SELECT request_digest,result_json FROM auth_logout_command_receipts WHERE operation_id=$1")
                .bind(operation).fetch_optional(&self.pool).await?;
            return match saved {
                Some((same, value)) if same == digest => Self::response(200, value),
                Some(_) => Err(Self::invalid()),
                None => Self::response(404, json!({"code":"NOT_FOUND"})),
            };
        }
        self.logout_sends.fetch_add(1, Ordering::SeqCst);
        let mut tx = self.pool.begin().await?;
        let generation: i64 =
            sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1 FOR UPDATE")
                .bind(source.user)
                .fetch_one(&mut *tx)
                .await?;
        if scope == "GLOBAL" {
            sqlx::query("UPDATE users SET session_version=$2 WHERE id=$1")
                .bind(source.user)
                .bind(generation + 1)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE sso_sessions SET revoked_at=COALESCE(revoked_at,clock_timestamp()) WHERE user_id=$1")
                .bind(source.user).execute(&mut *tx).await?;
        } else if scope != "CURRENT_APP" {
            return Err(Self::invalid());
        }
        sqlx::query(
            "UPDATE auth_grants SET revoked_at=COALESCE(revoked_at,clock_timestamp()) WHERE id=$1",
        )
        .bind(source.grant)
        .execute(&mut *tx)
        .await?;
        let receipt = json!({"operationId":operation,"scope":scope,"state":"COMPLETED","revokedGrantCount":1,
            "revokedSsoCount":if scope=="GLOBAL"{1}else{0},"notificationCount":0});
        sqlx::query("INSERT INTO auth_logout_command_receipts(operation_id,client_id,grant_id,user_id,token_digest,scope,public_sid,public_sub,request_digest,result_json) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
            .bind(operation).bind(self.client).bind(source.grant).bind(source.user).bind(hash::sha256_hex(&token)).bind(scope)
            .bind(&source.public_sid).bind(&source.subject).bind(digest).bind(&receipt).execute(&mut *tx).await?;
        tx.commit().await?;
        self.gate.response(3).await;
        if self.lost_logout.load(Ordering::SeqCst) {
            return Err(AppError::Unavailable);
        }
        Self::response(200, receipt)
    }
}
impl bff::OidcTransport for Transport {
    fn send<'a>(&'a self, request: bff::OidcRequest) -> bff::OidcFuture<'a> {
        Box::pin(async move {
            use bff::Endpoint;
            match request.endpoint {
                Endpoint::Discovery => Self::response(
                    200,
                    json!({"issuer":ORIGIN,"authorization_endpoint":format!("{ORIGIN}/oauth/authorize"),
                    "token_endpoint":format!("{ORIGIN}/oauth/token"),"jwks_uri":format!("{ORIGIN}/oauth/jwks"),
                    "userinfo_endpoint":format!("{ORIGIN}/oauth/userinfo"),"introspection_endpoint":format!("{ORIGIN}/oauth/introspect"),
                    "revocation_endpoint":format!("{ORIGIN}/oauth/revoke"),"response_types_supported":["code"],"id_token_signing_alg_values_supported":["RS256"],
                    "token_endpoint_auth_methods_supported":["client_secret_basic"],"code_challenge_methods_supported":["S256"]}),
                ),
                Endpoint::Jwks => Self::response(
                    200,
                    json!({"keys":[{"kid":"phase10-a","kty":"RSA","alg":"RS256","use":"sig","n":KEY_A_N,"e":"AQAB"}]}),
                ),
                Endpoint::AccountProof | Endpoint::UserInfo => {
                    let proof = matches!(request.endpoint, Endpoint::AccountProof);
                    let mut response = self.inner.send(request).await?;
                    if proof {
                        let ttl = self.proof_ttl.load(Ordering::SeqCst);
                        if ttl > 0 {
                            let end = Utc::now().timestamp() + ttl;
                            self.last_proof_exp.store(end, Ordering::SeqCst);
                            let mut body: Value = serde_json::from_slice(&response.body)
                                .map_err(|_| AppError::Unavailable)?;
                            body["expiresAt"] =
                                json!(DateTime::from_timestamp(end, 0)
                                    .ok_or(AppError::Unavailable)?);
                            response.body =
                                serde_json::to_vec(&body).map_err(|_| AppError::Unavailable)?;
                        }
                        self.gate.response(2).await;
                    }
                    Ok(response)
                }
                Endpoint::Token => {
                    if request.authorization.as_deref()
                        != Some(&format!(
                            "Basic {}",
                            STANDARD.encode(format!("{}:{}", self.client, "a".repeat(64)))
                        ))
                    {
                        return Err(Self::invalid());
                    }
                    let kind = Self::form(&request, "grant_type")?;
                    let refresh = kind == "refresh_token";
                    let source = if refresh {
                        self.refreshes.fetch_add(1, Ordering::SeqCst);
                        self.rotate(&Self::form(&request, "refresh_token")?).await?
                    } else {
                        if kind != "authorization_code" {
                            return Err(Self::invalid());
                        }
                        let code = Self::lock(&self.codes)?
                            .remove(&Self::form(&request, "code")?)
                            .ok_or_else(Self::invalid)?;
                        if URL_SAFE_NO_PAD.encode(Sha256::digest(
                            Self::form(&request, "code_verifier")?.as_bytes(),
                        )) != code.challenge
                        {
                            return Err(Self::invalid());
                        }
                        code.source
                    };
                    let response = self.token_response(&source)?;
                    self.gate.response(1).await;
                    if refresh && self.lost_refresh.load(Ordering::SeqCst) {
                        return Err(AppError::Unavailable);
                    }
                    Self::response(200, response)
                }
                Endpoint::GlobalLogout => {
                    let operation = Uuid::parse_str(&Self::form(&request, "operation_id")?)
                        .map_err(|_| Self::invalid())?;
                    self.logout(&request, operation, false).await
                }
                Endpoint::LogoutOperation(operation) => {
                    self.logout(&request, operation, true).await
                }
                _ => Err(AppError::Unavailable),
            }
        })
    }
}

struct Fixture {
    pool: PgPool,
    state: AppState,
    app: Router,
    transport: Arc<Transport>,
    tenant: Uuid,
    org: Uuid,
    user: Uuid,
    other: Uuid,
    member: Uuid,
}
struct Flow {
    binder: String,
    state: String,
    nonce: String,
    target: reqwest::Url,
}
#[derive(Clone)]
struct Session {
    cookie: String,
    id: Uuid,
    source: Source,
}
struct Reply {
    status: StatusCode,
    body: Value,
    headers: HeaderMap,
}
impl Fixture {
    async fn new(pool: PgPool) -> Result<Self> {
        let tenant = uuid7::new_uuid();
        let org = uuid7::new_uuid();
        let user = uuid7::new_uuid();
        let other = uuid7::new_uuid();
        let member = uuid7::new_uuid();
        sqlx::query("INSERT INTO tenants(id,name,code) VALUES($1,'Owned BFF fixture',$2)")
            .bind(tenant)
            .bind(format!("bff-{tenant}"))
            .execute(&pool)
            .await?;
        sqlx::query(
            "INSERT INTO organizations(id,tenant_id,name) VALUES($1,$2,'Owned BFF organization')",
        )
        .bind(org)
        .bind(tenant)
        .execute(&pool)
        .await?;
        for id in [user, other] {
            sqlx::query("INSERT INTO users(id,username,password_hash,session_version) VALUES($1,$2,'test-only-unused-password-hash',1)")
                .bind(id).bind(format!("bff-{id}")).execute(&pool).await?;
            sqlx::query("INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES($1,$2,$3,'EMPLOYEE')")
                .bind(if id==user {member} else {uuid7::new_uuid()}).bind(id).bind(org).execute(&pool).await?;
        }
        let mut config = Config::from_values(&HashMap::from([
            ("APP_ENV".into(), "test".into()),
            ("JWT_SECRET".into(), "owned-phase24-bff-signing-key".into()),
            ("PUBLIC_APP_URL".into(), CONSOLE.into()),
            ("MEMBERSHIP_LIFECYCLE_ENABLED".into(), "true".into()),
        ]))?;
        config.bff_oidc_redirect_uri = Some(format!("{CONSOLE}/auth/callback"));
        config.bff_authority_timeout_ms = 10_000;
        let mut state = crate::sso_test_fixture::state(pool.clone(), Arc::new(config));
        let client = state
            .config
            .bff_oidc_client_id
            .ok_or_else(|| anyhow!("fixture client absent"))?;
        let transport = Arc::new(Transport {
            pool: pool.clone(),
            inner: state.bff_transport.clone(),
            client,
            codes: Mutex::new(HashMap::new()),
            refresh: Mutex::new(HashMap::new()),
            gate: Gate::new(),
            refreshes: AtomicUsize::new(0),
            logout_sends: AtomicUsize::new(0),
            logout_lookups: AtomicUsize::new(0),
            lost_refresh: AtomicBool::new(false),
            lost_logout: AtomicBool::new(false),
            id_ttl: AtomicI64::new(300),
            last_id_exp: AtomicI64::new(0),
            proof_ttl: AtomicI64::new(0),
            last_proof_exp: AtomicI64::new(0),
            graft_sid: AtomicBool::new(false),
        });
        state.bff_transport = transport.clone();
        let app = Self::router(state.clone());
        Ok(Self {
            pool,
            state,
            app,
            transport,
            tenant,
            org,
            user,
            other,
            member,
        })
    }
    fn router(state: AppState) -> Router {
        crate::routes::api_router().with_state(state)
    }
    fn other_instance(&self) -> Router {
        let mut state = AppState::new(self.pool.clone(), self.state.config.clone());
        state.bff_transport = self.transport.clone();
        Self::router(state)
    }
    async fn flow(&self, binder: Option<&str>, step_up: bool) -> Result<Flow> {
        let path = if step_up {
            "/auth/step-up?return=%2Fsystem%2Foauth%2Fclients"
        } else {
            "/auth/login?return=%2F"
        };
        let reply = request(&self.app, Method::GET, path, binder, None, Value::Null).await?;
        expect(&reply, StatusCode::SEE_OTHER)?;
        let target = reqwest::Url::parse(
            reply
                .headers
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| anyhow!("missing canonical redirect"))?,
        )?;
        ensure!(
            target.origin().ascii_serialization() == ORIGIN && target.path() == "/oauth/authorize",
            "untrusted authorization redirect"
        );
        let parameters: HashMap<_, _> = target.query_pairs().into_owned().collect();
        ensure!(
            parameters.len() == target.query_pairs().count(),
            "repeated authorization parameters"
        );
        let take = |name: &str| {
            parameters
                .get(name)
                .cloned()
                .ok_or_else(|| anyhow!("missing authorization parameter {name}"))
        };
        let binder = cookie(&reply, BINDER)?;
        Ok(Flow {
            binder,
            state: take("state")?,
            nonce: take("nonce")?,
            target,
        })
    }
    async fn issue(&self, flow: &Flow, user: Uuid) -> Result<(String, Source)> {
        let platform =
            crate::sso_test_fixture::session(&self.pool, user, self.tenant, self.org).await?;
        let (id,global,grant,public_sid,subject,auth_time,absolute,access_ciphertext):(Uuid,Uuid,Uuid,String,String,i64,DateTime<Utc>,String)=sqlx::query_as("SELECT b.id,b.global_session_id,b.oidc_grant_id,b.oidc_public_sid,b.oidc_public_sub,b.auth_time,b.expires_at,b.access_ciphertext FROM bff_sessions b JOIN platform_sessions s ON s.bff_session_id=b.id WHERE s.id=$1")
            .bind(platform).fetch_one(&self.pool).await?;
        let keys = self
            .state
            .config
            .bff_token_keys
            .as_deref()
            .ok_or_else(|| anyhow!("missing fixture key"))?;
        let access = keys.open(
            id,
            ORIGIN,
            self.transport.client,
            "oidc-access",
            &access_ciphertext,
        )?;
        let refresh = hash::sha256_hex(&uuid7::new_uuid().to_string());
        let family: Uuid =
            sqlx::query_scalar("SELECT id FROM auth_refresh_families WHERE grant_id=$1")
                .bind(grant)
                .fetch_one(&self.pool)
                .await?;
        sqlx::query("INSERT INTO auth_refresh_tokens(id,family_id,token_digest,generation,expires_at) VALUES($1,$2,$3,1,$4)")
            .bind(uuid7::new_uuid()).bind(family).bind(hash::sha256_hex(&refresh)).bind(absolute).execute(&self.pool).await?;
        // The current logout receipt trigger requires the actual original RP
        // binding, in addition to the grant and its credential. Never weaken
        // that check to accommodate a partial transport fixture.
        sqlx::query("INSERT INTO auth_rp_sessions(grant_id,client_id,user_id,source_session_id,public_sid,public_sub) VALUES($1,$2,$3,$4,$5,$6)")
            .bind(grant).bind(self.transport.client).bind(user).bind(global).bind(&public_sid).bind(&subject)
            .execute(&self.pool).await?;
        let source = Source {
            access,
            refresh: refresh.clone(),
            user,
            grant,
            global,
            public_sid,
            subject,
            nonce: flow.nonce.clone(),
            family,
            generation: 1,
            auth_time,
            absolute,
        };
        Transport::lock(&self.transport.refresh)?.insert(refresh, source.clone());
        let code = hash::sha256_hex(&uuid7::new_uuid().to_string());
        let challenge = flow
            .target
            .query_pairs()
            .find(|(k, _)| k == "code_challenge")
            .map(|(_, v)| v.into_owned())
            .ok_or_else(|| anyhow!("missing PKCE challenge"))?;
        Transport::lock(&self.transport.codes)?.insert(
            code.clone(),
            Code {
                source: source.clone(),
                challenge,
            },
        );
        Ok((code, source))
    }
    async fn callback(&self, flow: &Flow, code: &str) -> Result<Reply> {
        let mut url = reqwest::Url::parse(&format!("{CONSOLE}/auth/callback"))?;
        url.query_pairs_mut()
            .append_pair("state", &flow.state)
            .append_pair("code", code)
            .append_pair("iss", ORIGIN);
        request(
            &self.app,
            Method::GET,
            &format!("{}?{}", url.path(), url.query().unwrap_or_default()),
            Some(&flow.binder),
            None,
            Value::Null,
        )
        .await
    }
    async fn login(&self, user: Uuid, binder: Option<&str>) -> Result<(Flow, Session)> {
        let flow = self.flow(binder, false).await?;
        let (code, source) = self.issue(&flow, user).await?;
        let reply = self.callback(&flow, &code).await?;
        expect(&reply, StatusCode::SEE_OTHER)?;
        let cookie = cookie(&reply, COOKIE)?;
        let raw = cookie
            .split_once('=')
            .ok_or_else(|| anyhow!("malformed test cookie"))?
            .1;
        let id = sqlx::query_scalar("SELECT id FROM bff_sessions WHERE token_hash=$1")
            .bind(hash::sha256_hex(raw))
            .fetch_one(&self.pool)
            .await?;
        Ok((flow, Session { cookie, id, source }))
    }
    async fn near_refresh(&self, session: &Session) -> Result<()> {
        sqlx::query("UPDATE bff_sessions SET access_expires_at=clock_timestamp()+interval '10 seconds' WHERE id=$1").bind(session.id).execute(&self.pool).await?;
        Ok(())
    }
}
async fn request(
    app: &Router,
    method: Method,
    path: &str,
    cookie: Option<&str>,
    csrf: Option<&str>,
    body: Value,
) -> Result<Reply> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "console-fixture.example.invalid")
        .header("origin", CONSOLE)
        .header("content-type", "application/json")
        .extension(ConnectInfo(SocketAddr::from(([192, 0, 2, 2], 32124))));
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie)
    }
    if let Some(csrf) = csrf {
        builder = builder.header("x-csrf-token", csrf)
    }
    let response = tokio::time::timeout(
        Duration::from_secs(15),
        app.clone()
            .oneshot(builder.body(Body::from(body.to_string()))?),
    )
    .await
    .map_err(|_| anyhow!("owned BFF HTTP request timed out"))??;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok(Reply {
        status,
        headers,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    })
}
fn expect(reply: &Reply, status: StatusCode) -> Result<()> {
    ensure!(
        reply.status == status,
        "expected {status}, received {}",
        reply.status
    );
    Ok(())
}
fn cookie(reply: &Reply, name: &str) -> Result<String> {
    let prefix = format!("{name}=");
    let values = reply
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter(|v| v.starts_with(&prefix))
        .collect::<Vec<_>>();
    ensure!(values.len() == 1, "missing or repeated RP cookie");
    let fields = values[0].split(';').map(str::trim).collect::<Vec<_>>();
    ensure!(
        fields.contains(&"Secure")
            && fields.contains(&"HttpOnly")
            && fields.contains(&"Path=/")
            && fields.contains(&"SameSite=Lax"),
        "weakened RP cookie"
    );
    Ok(fields[0].into())
}
fn no_cookie(reply: &Reply) -> Result<()> {
    ensure!(
        !reply.headers.contains_key(header::SET_COOKIE),
        "late or indeterminate response changed browser cookie"
    );
    Ok(())
}
async fn snapshot(app: &Router, session: &Session) -> Result<Reply> {
    request(
        app,
        Method::GET,
        "/auth/session",
        Some(&session.cookie),
        None,
        Value::Null,
    )
    .await
}
async fn choose(fixture: &Fixture, session: &Session, view: &Reply) -> Result<Reply> {
    let csrf = view.body["csrfToken"]
        .as_str()
        .ok_or_else(|| anyhow!("missing CSRF response"))?;
    request(&fixture.app,Method::POST,"/auth/context-token",Some(&session.cookie),Some(csrf),json!({"expectedVersion":view.body["version"],"membershipId":fixture.member,"organizationId":fixture.org})).await
}
async fn logout(app: &Router, session: &Session, view: &Reply, scope: &str) -> Result<Reply> {
    request(
        app,
        Method::POST,
        "/auth/logout",
        Some(&session.cookie),
        view.body["csrfToken"].as_str(),
        json!({"expectedVersion":view.body["version"],"scope":scope}),
    )
    .await
}
async fn waiting_for_lock(pool: &PgPool) -> Result<()> {
    let start = tokio::time::Instant::now();
    loop {
        let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid() AND wait_event_type='Lock')").fetch_one(pool).await?;
        if waiting {
            return Ok(());
        }
        ensure!(
            start.elapsed() < Duration::from_secs(3),
            "BFF transaction did not wait for the held source lock"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
async fn past(pool: &PgPool, deadline: i64) -> Result<()> {
    let start = tokio::time::Instant::now();
    loop {
        let expired: bool = sqlx::query_scalar("SELECT clock_timestamp()>=to_timestamp($1)")
            .bind(deadline as f64)
            .fetch_one(pool)
            .await?;
        if expired {
            return Ok(());
        }
        ensure!(
            start.elapsed() < Duration::from_secs(4),
            "fixture deadline did not elapse"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
#[ignore = "requires owned IDENTITY_PHASE24_TEST_DATABASE_URL/CLUSTER_ID"]
async fn canonical_cross_instance_refresh_single_winner_and_lost_reply_never_replays() -> Result<()>
{
    isolated(|pool|async move{
        let f=Fixture::new(pool).await?;let(_,session)=f.login(f.user,None).await?;
        let before:(String,String,DateTime<Utc>,i64)=sqlx::query_as("SELECT access_ciphertext,refresh_ciphertext,expires_at,version FROM bff_sessions WHERE id=$1").bind(session.id).fetch_one(&f.pool).await?;
        f.near_refresh(&session).await?;f.transport.gate.arm(1);
        let app=f.app.clone();let first=session.clone();let pending=tokio::spawn(async move{snapshot(&app,&first).await});
        f.transport.gate.reached().await?;
        let other=f.other_instance();let raced=snapshot(&other,&session).await?;
        f.transport.gate.release.notify_one();let winner=pending.await??;
        expect(&raced,StatusCode::SERVICE_UNAVAILABLE)?;expect(&winner,StatusCode::OK)?;
        ensure!(f.transport.refreshes.load(Ordering::SeqCst)==1,"two instances replayed a rotating credential");
        let after:(String,String,DateTime<Utc>,i64,bool,Option<Uuid>)=sqlx::query_as("SELECT access_ciphertext,refresh_ciphertext,expires_at,version,refresh_uncertain,refresh_lease_id FROM bff_sessions WHERE id=$1").bind(session.id).fetch_one(&f.pool).await?;
        ensure!(after.0!=before.0&&after.1!=before.1&&after.2==before.2&&after.3==before.3&&!after.4&&after.5.is_none(),"rotation changed generation/deadline or retained lease");
        let(_,unknown)=f.login(f.user,None).await?;f.near_refresh(&unknown).await?;
        f.transport.lost_refresh.store(true,Ordering::SeqCst);let sent=f.transport.refreshes.load(Ordering::SeqCst);
        expect(&snapshot(&f.app,&unknown).await?,StatusCode::SERVICE_UNAVAILABLE)?;
        let consumed:bool=sqlx::query_scalar("SELECT consumed_at IS NOT NULL FROM auth_refresh_tokens WHERE token_digest=$1").bind(hash::sha256_hex(&unknown.source.refresh)).fetch_one(&f.pool).await?;
        ensure!(consumed,"lost-response fixture did not commit the credential's consumption");
        f.transport.lost_refresh.store(false,Ordering::SeqCst);
        expect(&snapshot(&other,&unknown).await?,StatusCode::SERVICE_UNAVAILABLE)?;
        sqlx::query("UPDATE bff_sessions SET refresh_uncertain=false,refresh_started_at=clock_timestamp()-interval '1 hour' WHERE id=$1").bind(unknown.id).execute(&f.pool).await?;
        expect(&snapshot(&other,&unknown).await?,StatusCode::SERVICE_UNAVAILABLE)?;
        ensure!(f.transport.refreshes.load(Ordering::SeqCst)==sent+1,"unknown or expired lease retried its original refresh");Ok(())
    }).await
}

#[tokio::test]
#[ignore = "requires owned IDENTITY_PHASE24_TEST_DATABASE_URL/CLUSTER_ID"]
async fn canonical_post_network_account_lock_rechecks_original_source() -> Result<()> {
    isolated(|pool|async move{
        let f=Fixture::new(pool).await?;let(_,session)=f.login(f.user,None).await?;let view=snapshot(&f.app,&session).await?;expect(&view,StatusCode::OK)?;
        let mut owner=f.pool.begin().await?;sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE").bind(f.user).execute(&mut *owner).await?;
        let app=f.app.clone();let saved=session.clone();let csrf=view.body["csrfToken"].as_str().unwrap().to_owned();let body=json!({"expectedVersion":view.body["version"],"membershipId":f.member,"organizationId":f.org});
        let pending=tokio::spawn(async move{request(&app,Method::POST,"/auth/context-token",Some(&saved.cookie),Some(&csrf),body).await});
        let waited=waiting_for_lock(&f.pool).await;sqlx::query("UPDATE users SET session_version=session_version+1 WHERE id=$1").bind(f.user).execute(&mut *owner).await?;
        sqlx::query("UPDATE sso_sessions SET revoked_at=clock_timestamp() WHERE id=$1").bind(session.source.global).execute(&mut *owner).await?;owner.commit().await?;
        let response=pending.await??;waited?;expect(&response,StatusCode::UNAUTHORIZED)?;
        let unchanged:(i64,Uuid,i64)=sqlx::query_as("SELECT account_version,global_session_id,(SELECT count(*) FROM platform_sessions WHERE bff_session_id=b.id) FROM bff_sessions b WHERE id=$1").bind(session.id).fetch_one(&f.pool).await?;
        ensure!(unchanged==(1,session.source.global,0),"late source check partially installed or rebound context");
        let(_,fresh)=f.login(f.user,None).await?;let fresh_view=snapshot(&f.app,&fresh).await?;expect(&fresh_view,StatusCode::OK)?;
        expect(&choose(&f,&fresh,&fresh_view).await?,StatusCode::OK)?;Ok(())
    }).await
}

#[tokio::test]
#[ignore = "requires owned IDENTITY_PHASE24_TEST_DATABASE_URL/CLUSTER_ID"]
async fn canonical_short_id_and_online_access_deadlines_cannot_commit_after_lock_wait() -> Result<()>
{
    isolated(|pool| async move {
        let f = Fixture::new(pool).await?;
        for (refreshing, short_proof) in
            [(false, false), (true, false), (false, true), (true, true)]
        {
            let flow = f.flow(None, false).await?;
            let (code, source) = f.issue(&flow, f.user).await?;
            let initial = f.callback(&flow, &code).await?;
            expect(&initial, StatusCode::SEE_OTHER)?;
            let cookie = cookie(&initial, COOKIE)?;
            let raw = cookie.split_once('=').unwrap().1;
            let id: Uuid = sqlx::query_scalar("SELECT id FROM bff_sessions WHERE token_hash=$1")
                .bind(hash::sha256_hex(raw))
                .fetch_one(&f.pool)
                .await?;
            let session = Session { cookie, id, source };
            let next = if refreshing {
                None
            } else {
                Some(f.flow(None, false).await?)
            };
            let next_code = if let Some(flow) = &next {
                Some(f.issue(flow, f.user).await?.0)
            } else {
                None
            };
            let baseline: i64 = sqlx::query_scalar("SELECT count(*) FROM bff_sessions")
                .fetch_one(&f.pool)
                .await?;
            let before: String =
                sqlx::query_scalar("SELECT access_ciphertext FROM bff_sessions WHERE id=$1")
                    .bind(id)
                    .fetch_one(&f.pool)
                    .await?;
            if refreshing {
                f.near_refresh(&session).await?;
            }
            f.transport
                .id_ttl
                .store(if short_proof { 300 } else { 2 }, Ordering::SeqCst);
            f.transport
                .proof_ttl
                .store(if short_proof { 2 } else { 0 }, Ordering::SeqCst);
            f.transport.gate.arm(2);
            let mut owner = f.pool.begin().await?;
            sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
                .bind(f.user)
                .execute(&mut *owner)
                .await?;
            let app = f.app.clone();
            let session = session.clone();
            let callback_input = next.zip(next_code);
            let pending = tokio::spawn(async move {
                if let Some((flow, code)) = callback_input {
                    let path = format!(
                        "/auth/callback?state={}&code={code}&iss={}",
                        flow.state,
                        percent_encoding::utf8_percent_encode(
                            ORIGIN,
                            percent_encoding::NON_ALPHANUMERIC
                        )
                    );
                    request(
                        &app,
                        Method::GET,
                        &path,
                        Some(&flow.binder),
                        None,
                        Value::Null,
                    )
                    .await
                } else {
                    snapshot(&app, &session).await
                }
            });
            f.transport.gate.reached().await?;
            f.transport.gate.release.notify_one();
            let waited = waiting_for_lock(&f.pool).await;
            let deadline = if short_proof {
                f.transport.last_proof_exp.load(Ordering::SeqCst)
            } else {
                f.transport.last_id_exp.load(Ordering::SeqCst)
            };
            past(&f.pool, deadline).await?;
            owner.commit().await?;
            let result = pending.await??;
            waited?;
            expect(&result, StatusCode::UNAUTHORIZED)?;
            no_cookie(&result)?;
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM bff_sessions")
                .fetch_one(&f.pool)
                .await?;
            let unchanged: String =
                sqlx::query_scalar("SELECT access_ciphertext FROM bff_sessions WHERE id=$1")
                    .bind(id)
                    .fetch_one(&f.pool)
                    .await?;
            ensure!(
                count == baseline && unchanged == before,
                "expired ID proof committed a callback or rotated access"
            );
            f.transport.id_ttl.store(300, Ordering::SeqCst);
            f.transport.proof_ttl.store(0, Ordering::SeqCst);
        }
        Ok(())
    })
    .await
}

#[tokio::test]
#[ignore = "requires owned IDENTITY_PHASE24_TEST_DATABASE_URL/CLUSTER_ID"]
async fn canonical_cookie_reload_recovers_only_original_unknown_logout() -> Result<()> {
    isolated(|pool|async move{
        let f=Fixture::new(pool).await?;
        for scope in ["PLATFORM","GLOBAL"]{
            let(_,session)=f.login(f.user,None).await?;let view=snapshot(&f.app,&session).await?;expect(&view,StatusCode::OK)?;
            let generation:i64=sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1").bind(f.user).fetch_one(&f.pool).await?;
            let sends=f.transport.logout_sends.load(Ordering::SeqCst);let lookups=f.transport.logout_lookups.load(Ordering::SeqCst);
            f.transport.lost_logout.store(true,Ordering::SeqCst);
            let lost=logout(&f.app,&session,&view,scope).await?;expect(&lost,StatusCode::SERVICE_UNAVAILABLE)?;no_cookie(&lost)?;
            let pending:(Uuid,String,i64)=sqlx::query_as("SELECT logout_request_id,logout_scope,version FROM bff_sessions WHERE id=$1").bind(session.id).fetch_one(&f.pool).await?;
            let expected_version = pending.2.to_string();
            ensure!(pending.1==scope&&view.body["version"].as_str()==Some(expected_version.as_str()),"unknown logout changed original scope/version");
            let revoked:DateTime<Utc>=sqlx::query_scalar("SELECT revoked_at FROM auth_grants WHERE id=$1").bind(session.source.grant).fetch_one(&f.pool).await?;
            let opposite=if scope=="GLOBAL"{"PLATFORM"}else{"GLOBAL"};
            expect(&logout(&f.app,&session,&view,opposite).await?,StatusCode::CONFLICT)?;
            expect(&request(&f.app,Method::GET,"/auth/session?scope=GLOBAL",Some(&session.cookie),None,Value::Null).await?,StatusCode::BAD_REQUEST)?;
            ensure!(f.transport.logout_sends.load(Ordering::SeqCst)==sends+1,"scope replacement caused a new upstream write");
            f.transport.lost_logout.store(false,Ordering::SeqCst);
            let recovered=snapshot(&f.other_instance(),&session).await?;expect(&recovered,StatusCode::UNAUTHORIZED)?;no_cookie(&recovered)?;
            let final_state:(Uuid,String,bool,bool)=sqlx::query_as("SELECT logout_request_id,logout_scope,revoked_at IS NOT NULL,logout_confirmed_at IS NOT NULL FROM bff_sessions WHERE id=$1").bind(session.id).fetch_one(&f.pool).await?;
            ensure!(final_state==(pending.0,scope.to_owned(),true,true),"recovery replaced or failed to close original durable command");
            let receipt:(String,Value)=sqlx::query_as("SELECT scope,result_json FROM auth_logout_command_receipts WHERE operation_id=$1").bind(pending.0).fetch_one(&f.pool).await?;
            ensure!(receipt.0==if scope=="GLOBAL"{"GLOBAL"}else{"CURRENT_APP"}&&receipt.1["operationId"]==pending.0.to_string(),"wrong original receipt recovered");
            let after:DateTime<Utc>=sqlx::query_scalar("SELECT revoked_at FROM auth_grants WHERE id=$1").bind(session.source.grant).fetch_one(&f.pool).await?;
            let current:i64=sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1").bind(f.user).fetch_one(&f.pool).await?;
            ensure!(after==revoked&&current==generation+if scope=="GLOBAL"{1}else{0},"receipt recovery repeated or broadened upstream revocation");
            ensure!(f.transport.logout_sends.load(Ordering::SeqCst)==sends+1&&f.transport.logout_lookups.load(Ordering::SeqCst)==lookups+2,"recovery resent a write instead of looking up original operation");
            let counts=(f.transport.logout_sends.load(Ordering::SeqCst),f.transport.logout_lookups.load(Ordering::SeqCst));
            expect(&snapshot(&f.app,&session).await?,StatusCode::UNAUTHORIZED)?;
            ensure!(counts==(f.transport.logout_sends.load(Ordering::SeqCst),f.transport.logout_lookups.load(Ordering::SeqCst)),"confirmed old cookie initiated another authority command");
        }Ok(())
    }).await
}

#[tokio::test]
#[ignore = "requires owned IDENTITY_PHASE24_TEST_DATABASE_URL/CLUSTER_ID"]
async fn canonical_late_logout_cannot_erase_new_account_cookie() -> Result<()> {
    isolated(|pool| async move {
        let f = Fixture::new(pool).await?;
        let (flow, old) = f.login(f.user, None).await?;
        let view = snapshot(&f.app, &old).await?;
        expect(&view, StatusCode::OK)?;
        f.transport.gate.arm(3);
        let app = f.app.clone();
        let saved = old.clone();
        let pending = tokio::spawn(async move { logout(&app, &saved, &view, "GLOBAL").await });
        f.transport.gate.reached().await?;
        let newer = f.login(f.other, Some(&flow.binder)).await;
        f.transport.gate.release.notify_one();
        let late = pending.await??;
        let (_, newer) = newer?;
        expect(&late, StatusCode::CONFLICT)?;
        no_cookie(&late)?;
        let valid = snapshot(&f.other_instance(), &newer).await?;
        expect(&valid, StatusCode::OK)?;
        ensure!(
            valid.body["account"]["id"] == f.other.to_string() && old.cookie != newer.cookie,
            "late logout rebound the replacement account"
        );
        let commands = (
            f.transport.logout_sends.load(Ordering::SeqCst),
            f.transport.logout_lookups.load(Ordering::SeqCst),
        );
        let retired = snapshot(&f.app, &old).await?;
        // The old durable logout is confirmed, but this browser now points to
        // another source. Its recovery response must report that exact conflict
        // rather than describe the replacement browser as anonymous.
        expect(&retired, StatusCode::CONFLICT)?;
        ensure!(retired.body["code"] == "CONFLICT", "old logout recovery omitted its superseded-browser conflict");
        no_cookie(&retired)?;
        let closed: (bool, bool, bool, Uuid) = sqlx::query_as("SELECT b.revoked_at IS NOT NULL,b.logout_confirmed_at IS NOT NULL,bff_session_active(b.id),browser.current_session_id FROM bff_sessions b JOIN console_browsers browser ON browser.id=b.browser_id WHERE b.id=$1")
            .bind(old.id).fetch_one(&f.pool).await?;
        ensure!(closed == (true, true, false, newer.id), "old logout recovery revived its source or changed the replacement browser pointer");
        ensure!(commands == (
            f.transport.logout_sends.load(Ordering::SeqCst),
            f.transport.logout_lookups.load(Ordering::SeqCst),
        ), "confirmed superseded logout initiated another authority command");
        // Even a fully successful response may arrive after another login. The
        // response itself carries no Cookie mutation capable of erasing it.
        let valid = snapshot(&f.app, &newer).await?;
        let delayed = logout(&f.app, &newer, &valid, "PLATFORM").await?;
        expect(&delayed, StatusCode::OK)?;
        no_cookie(&delayed)?;
        let (_, third) = f.login(f.user, Some(&flow.binder)).await?;
        expect(&snapshot(&f.app, &third).await?, StatusCode::OK)?;
        no_cookie(&delayed)?;
        Ok(())
    })
    .await
}

#[tokio::test]
#[ignore = "requires owned IDENTITY_PHASE24_TEST_DATABASE_URL/CLUSTER_ID"]
async fn canonical_refreshed_id_token_cannot_graft_a_new_public_sid() -> Result<()> {
    isolated(|pool|async move{
        let f=Fixture::new(pool).await?;let(_,session)=f.login(f.user,None).await?;
        let before:(String,String)=sqlx::query_as("SELECT access_ciphertext,oidc_public_sid FROM bff_sessions WHERE id=$1").bind(session.id).fetch_one(&f.pool).await?;
        f.near_refresh(&session).await?;f.transport.graft_sid.store(true,Ordering::SeqCst);
        let rejected=snapshot(&f.app,&session).await?;expect(&rejected,StatusCode::UNAUTHORIZED)?;no_cookie(&rejected)?;
        let after:(String,String,bool)=sqlx::query_as("SELECT access_ciphertext,oidc_public_sid,refresh_uncertain FROM bff_sessions WHERE id=$1").bind(session.id).fetch_one(&f.pool).await?;
        ensure!(after.0==before.0&&after.1==before.1&&after.2,"a refresh changed original source or became replayable");Ok(())
    }).await
}

fn logout_token(client: Uuid, source: &Source, ttl: i64) -> Result<String> {
    let now = Utc::now().timestamp();
    ensure!(
        (1..=120).contains(&ttl),
        "invalid remaining notification window"
    );
    // A short remaining lifetime must retain the canonical 120-second signed
    // lifetime. Changing exp alone would fail validation before reaching SQL.
    let issued = now - (120 - ttl);
    signed(
        &json!({"iss":ORIGIN,"aud":client,"sid":source.public_sid,"sub":source.subject,"jti":uuid7::new_uuid(),
            "iat":issued,"exp":now+ttl,"events":{"http://schemas.openid.net/event/backchannel-logout":{}}}),
    )
}

#[tokio::test]
#[ignore = "requires owned IDENTITY_PHASE24_TEST_DATABASE_URL/CLUSTER_ID"]
async fn canonical_backchannel_before_callback_persists_fence_without_revoking_new_source(
) -> Result<()> {
    isolated(|pool|async move{
        let f=Fixture::new(pool).await?;let flow=f.flow(None,false).await?;let(code,source)=f.issue(&flow,f.user).await?;
        let token=logout_token(f.transport.client,&source,120)?;
        ensure!(bff::backchannel::receive(&f.state,&token).await?,"new canonical notification was not applied");
        let before:i64=sqlx::query_scalar("SELECT count(*) FROM bff_sessions").fetch_one(&f.pool).await?;
        let denied=f.callback(&flow,&code).await?;expect(&denied,StatusCode::UNAUTHORIZED)?;no_cookie(&denied)?;
        let after:i64=sqlx::query_scalar("SELECT count(*) FROM bff_sessions").fetch_one(&f.pool).await?;
        ensure!(before==after,"callback created a local session after original sid was fenced");
        let(_,replacement)=f.login(f.user,Some(&flow.binder)).await?;
        ensure!(!bff::backchannel::receive(&f.state,&token).await?,"exact notification replay applied twice");
        let valid=snapshot(&f.app,&replacement).await?;expect(&valid,StatusCode::OK)?;
        let pointer:Uuid=sqlx::query_scalar("SELECT current_session_id FROM console_browsers WHERE id=(SELECT browser_id FROM bff_sessions WHERE id=$1)").bind(replacement.id).fetch_one(&f.pool).await?;
        ensure!(pointer==replacement.id,"old notification cleared a newer source's browser pointer");Ok(())
    }).await
}

#[tokio::test]
#[ignore = "requires owned IDENTITY_PHASE24_TEST_DATABASE_URL/CLUSTER_ID"]
async fn canonical_backchannel_original_deadline_rolls_back_after_source_lock_wait() -> Result<()> {
    isolated(|pool|async move{
        let f=Fixture::new(pool).await?;let(_,session)=f.login(f.user,None).await?;
        let token=logout_token(f.transport.client,&session.source,2)?;
        let payload:Value=serde_json::from_slice(&URL_SAFE_NO_PAD.decode(token.split('.').nth(1).ok_or_else(||anyhow!("missing JWT body"))?)?)?;
        let expiry=payload["exp"].as_i64().ok_or_else(||anyhow!("missing JWT expiry"))?;
        let key=serde_json::to_string(&["oidc-rp-source/1",ORIGIN,&f.transport.client.to_string(),&session.source.public_sid])?;
        let mut owner=f.pool.begin().await?;sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))").bind(key).execute(&mut *owner).await?;
        let state=f.state.clone();let pending=tokio::spawn(async move{bff::backchannel::receive(&state,&token).await});
        let waited=waiting_for_lock(&f.pool).await;past(&f.pool,expiry).await?;owner.commit().await?;
        let response=pending.await?;waited?;ensure!(response.is_err(),"expired original notification survived lock wait");
        let counts:(i64,i64,bool)=sqlx::query_as("SELECT (SELECT count(*) FROM platform_backchannel_logout_sources),(SELECT count(*) FROM platform_backchannel_logout_receipts),(SELECT revoked_at IS NULL FROM bff_sessions WHERE id=$1)").bind(session.id).fetch_one(&f.pool).await?;
        ensure!(counts==(0,0,true),"expired notification partially committed source fence/receipt/revocation");Ok(())
    }).await
}

#[tokio::test]
#[ignore = "requires owned IDENTITY_PHASE24_TEST_DATABASE_URL/CLUSTER_ID"]
async fn canonical_step_up_requires_primary_and_keeps_new_browser_bound_flow() -> Result<()> {
    isolated(|pool|async move{
        let f=Fixture::new(pool).await?;let ordinary=f.flow(None,false).await?;
        let before:i64=sqlx::query_scalar("SELECT count(*) FROM bff_sessions").fetch_one(&f.pool).await?;
        let stepped=f.flow(Some(&ordinary.binder),true).await?;
        let values:HashMap<_,_>=stepped.target.query_pairs().into_owned().collect();
        ensure!(values.get("prompt").map(String::as_str)==Some("login")&&values.get("max_age").map(String::as_str)==Some("0")
            &&values.get("scope").map(String::as_str)==Some("openid profile")&&values.get("code_challenge_method").map(String::as_str)==Some("S256")
            &&!values.contains_key("client_secret")&&!values.contains_key("code_verifier"),"step-up weakened primary RP authentication policy");
        ensure!(!ordinary.target.query_pairs().any(|(k,_)|k=="prompt"||k=="max_age"),"ordinary login forced primary reauthentication");
        ensure!(stepped.binder==ordinary.binder&&stepped.state!=ordinary.state&&stepped.nonce!=ordinary.nonce,"step-up replaced binder or reused state/nonce");
        let flows:Vec<(Uuid,String,bool)>=sqlx::query_as("SELECT browser_id,return_path,consumed_at IS NOT NULL FROM console_login_flows WHERE state_hash=ANY($1)")
            .bind(vec![hash::sha256_hex(&ordinary.state),hash::sha256_hex(&stepped.state)]).fetch_all(&f.pool).await?;
        ensure!(flows.len()==2&&flows[0].0==flows[1].0&&flows.iter().all(|v|!v.2),"step-up did not retain a fresh unconsumed browser flow");
        for path in ["/auth/step-up?prompt=none","/auth/step-up?return=%2F%2Fattacker.invalid","/auth/step-up?return=%2Fauth%2Fcallback"]{
            expect(&request(&f.app,Method::GET,path,Some(&ordinary.binder),None,Value::Null).await?,StatusCode::BAD_REQUEST)?;
        }
        let after:i64=sqlx::query_scalar("SELECT count(*) FROM bff_sessions").fetch_one(&f.pool).await?;
        ensure!(before==after,"starting step-up created an unverified local session");
        let(code,_) =f.issue(&stepped,f.user).await?;
        let wrong=f.flow(None,false).await?;
        let path=format!("/auth/callback?state={}&code={code}&iss={}",stepped.state,percent_encoding::utf8_percent_encode(ORIGIN,percent_encoding::NON_ALPHANUMERIC));
        expect(&request(&f.app,Method::GET,&path,Some(&wrong.binder),None,Value::Null).await?,StatusCode::UNAUTHORIZED)?;
        expect(&f.callback(&stepped,&code).await?,StatusCode::SEE_OTHER)?;
        // An old tab must not overwrite the step-up's newly committed source.
        let(old_code,_)=f.issue(&ordinary,f.user).await?;
        expect(&f.callback(&ordinary,&old_code).await?,StatusCode::CONFLICT)?;Ok(())
    }).await
}
