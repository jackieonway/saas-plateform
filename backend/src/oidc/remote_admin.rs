//! Fresh platform authorization is committed before private OIDC commands.
//! Unknown results are reconciled by operation lookup, never command replay.

use crate::{
    error::AppError,
    extractors::{ApiJson, CurrentUser},
    state::AppState,
    util::uuid7,
};
use axum::{
    extract::{DefaultBodyLimit, Extension, Path, Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;

const LIMIT: usize = 65536;
const PREFIX: &str = "/internal/identity/oidc";
#[derive(Clone)]
struct Deadline {
    at: tokio::time::Instant,
    dispatched: Arc<Mutex<Option<Uuid>>>,
}
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/oidc/clients", get(list).post(create))
        .route("/api/oidc/clients/{id}", axum::routing::patch(update))
        .route("/api/oidc/keys/publish", post(publish))
        .route("/api/oidc/keys/activate", post(activate))
        .route("/api/oidc/keys/{kid}", axum::routing::delete(retire))
        .route("/api/oidc/keys/{kid}/revoke", post(revoke))
        .route("/api/oidc/operations/{id}", get(operation))
        .layer(DefaultBodyLimit::max(LIMIT))
        .route_layer(middleware::from_fn(bounded))
}
async fn bounded(mut request: Request, next: Next) -> Response {
    let deadline = Deadline {
        at: tokio::time::Instant::now() + Duration::from_secs(3),
        dispatched: Arc::new(Mutex::new(None)),
    };
    request.extensions_mut().insert(deadline.clone());
    let mut response = if request.uri().query().is_some() {
        AppError::bad_request("OIDC 管理入口不接受查询参数").into_response()
    } else {
        match tokio::time::timeout_at(deadline.at, next.run(request)).await {
            Ok(response) => response,
            Err(_) => Failure {
                error: AppError::Unavailable,
                operation: deadline.dispatched.lock().ok().and_then(|value| *value),
            }
            .into_response(),
        }
    };
    for (name, value) in [
        ("cache-control", "no-store"),
        ("pragma", "no-cache"),
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    response
}
fn id(raw: &str) -> Result<Uuid, AppError> {
    let value = uuid7::parse_uuid(raw).map_err(|_| AppError::bad_request("OIDC 标识无效"))?;
    if value.to_string() != raw {
        return Err(AppError::bad_request("OIDC 标识无效"));
    }
    Ok(value)
}
fn kid(raw: &str) -> Result<String, AppError> {
    if raw.is_empty()
        || raw.len() > 80
        || !raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return Err(AppError::bad_request("kid 无效"));
    }
    Ok(raw.into())
}
fn opaque(raw: &str) -> bool {
    raw.len() == 64
        && raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
#[derive(Clone)]
struct Target {
    method: Method,
    path: String,
}
impl Target {
    fn new(method: Method, relative: &str) -> Result<Self, AppError> {
        let valid = match (method.as_str(), relative) {
            ("GET" | "POST", "/clients") | ("POST", "/keys/publish" | "/keys/activate") => true,
            ("PATCH", path) if path.starts_with("/clients/") => id(&path[9..]).is_ok(),
            ("DELETE", path) if path.starts_with("/keys/") => kid(&path[6..]).is_ok(),
            ("POST", path) if path.starts_with("/keys/") && path.ends_with("/revoke") => {
                kid(&path[6..path.len() - 7]).is_ok()
            }
            ("GET", path) if path.starts_with("/operations/") => id(&path[12..]).is_ok(),
            _ => false,
        };
        if !valid {
            return Err(AppError::bad_request("OIDC 管理路径无效"));
        }
        Ok(Self {
            method,
            path: format!("{PREFIX}{relative}"),
        })
    }
    fn mutation(&self) -> bool {
        self.method != Method::GET
    }
    fn lookup(&self) -> bool {
        self.method == Method::GET && self.path.starts_with(&format!("{PREFIX}/operations/"))
    }
    fn from_intent(method: &str, path: &str) -> Result<Self, AppError> {
        let method = Method::from_bytes(method.as_bytes()).map_err(|_| AppError::Unavailable)?;
        let target = Self::new(
            method,
            path.strip_prefix(PREFIX).ok_or(AppError::Unavailable)?,
        )
        .map_err(|_| AppError::Unavailable)?;
        if !target.mutation() || target.path != path {
            return Err(AppError::Unavailable);
        }
        Ok(target)
    }
}
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys: Vec<_> = object.keys().collect();
            keys.sort();
            let mut result = serde_json::Map::new();
            for key in keys {
                result.insert(key.clone(), canonical(&object[key]));
            }
            Value::Object(result)
        }
        Value::Array(array) => Value::Array(array.iter().map(canonical).collect()),
        _ => value.clone(),
    }
}
fn digest(target: &Target, body: &Option<Value>) -> Result<String, AppError> {
    let bytes = serde_json::to_vec(&canonical(
        &json!({"method":target.method.as_str(),"path":target.path,"body":body}),
    ))
    .map_err(|_| AppError::bad_request("OIDC 管理请求无效"))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyCommand {
    kid: String,
}
fn prepare(target: &Target, value: Option<Value>) -> Result<Option<Value>, AppError> {
    let bad = || AppError::bad_request("OIDC 管理请求无效");
    match (target.method.as_str(), target.path.strip_prefix(PREFIX)) {
        ("POST", Some("/clients")) => {
            serde_json::from_value::<super::clients::Registration>(value.clone().ok_or_else(bad)?)
                .map_err(|_| bad())?;
        }
        ("PATCH", _) => {
            serde_json::from_value::<super::clients::PolicyUpdate>(value.clone().ok_or_else(bad)?)
                .map_err(|_| bad())?;
        }
        ("POST", Some("/keys/publish" | "/keys/activate")) => {
            let command: KeyCommand =
                serde_json::from_value(value.clone().ok_or_else(bad)?).map_err(|_| bad())?;
            kid(&command.kid)?;
        }
        _ if value.is_some() => return Err(bad()),
        _ => {}
    }
    if serde_json::to_vec(&value).map_err(|_| bad())?.len() > LIMIT {
        return Err(AppError::bad_request("OIDC 管理请求过大"));
    }
    Ok(value)
}
// These private types intentionally have no Debug: body/response can contain a
// just-issued RP secret, which must never enter logging, intents or audits.
struct Command {
    target: Target,
    body: Option<Value>,
    operation: Uuid,
    digest: String,
}
struct Reply {
    status: u16,
    content_type: Option<String>,
    bytes: Vec<u8>,
}
type AdminFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>;
trait Boundary: Send + Sync {
    fn accept<'a>(&'a self, command: &'a Command) -> AdminFuture<'a, ()>;
    fn acknowledge<'a>(&'a self, operation: Uuid) -> AdminFuture<'a, ()>;
}
trait Transport: Send + Sync {
    fn send<'a>(&'a self, command: &'a Command) -> AdminFuture<'a, Reply>;
}
struct PlatformBoundary<'a> {
    state: &'a AppState,
    user: &'a CurrentUser,
    headers: &'a HeaderMap,
}
impl Boundary for PlatformBoundary<'_> {
    fn accept<'a>(&'a self, command: &'a Command) -> AdminFuture<'a, ()> {
        Box::pin(async move {
            if !self.state.config.identity_remote_enabled {
                return Err(AppError::not_found("独立认证管理未启用"));
            }
            self.state.require(self.user, "system:oauth").await?;
            let mut tx = super::clients::admin_tx(self.state, self.user).await?;
            if command.target.mutation() {
                let service = id(self
                    .state
                    .config
                    .identity_admin_service_client_id
                    .as_deref()
                    .ok_or(AppError::Unavailable)?)?;
                let written=sqlx::query("INSERT INTO platform_oidc_command_intents(id,actor_user_id,tenant_id,organization_id,session_id,service_client_id,request_digest,method,path,accepted_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,clock_timestamp())")
                .bind(command.operation).bind(self.user.user_id).bind(self.user.tenant_id).bind(self.user.org_id).bind(self.user.session_id).bind(service).bind(&command.digest)
                .bind(command.target.method.as_str()).bind(&command.target.path).execute(&mut *tx).await.map_err(|_|AppError::Unavailable)?.rows_affected();
                if written != 1 {
                    return Err(AppError::Unavailable);
                }
                super::clients::admin_audit(&mut tx,self.user,self.headers,"OIDC_REMOTE_ACCEPT",&json!({"operationId":command.operation,"resource":command.target.path,"requestDigest":command.digest}).to_string()).await?;
            }
            // No transaction or platform row lock crosses the back-channel.
            tx.commit().await.map_err(|_| AppError::Unavailable)?;
            Ok(())
        })
    }
    fn acknowledge<'a>(&'a self, operation: Uuid) -> AdminFuture<'a, ()> {
        Box::pin(async move {
            let mut tx = self
                .state
                .pool
                .begin()
                .await
                .map_err(|_| AppError::Unavailable)?;
            super::clients::admin_audit(
                &mut tx,
                self.user,
                self.headers,
                "OIDC_REMOTE_ACK",
                &json!({"operationId":operation}).to_string(),
            )
            .await?;
            tx.commit().await.map_err(|_| AppError::Unavailable)?;
            Ok(())
        })
    }
}
struct HttpTransport {
    client: reqwest::Client,
    origin: String,
    authorization: String,
}
impl HttpTransport {
    fn configured(state: &AppState) -> Result<Self, AppError> {
        let origin = state
            .config
            .identity_service_url
            .as_deref()
            .ok_or(AppError::Unavailable)?;
        let url = reqwest::Url::parse(origin).map_err(|_| AppError::Unavailable)?;
        if url.scheme() != "https"
            || url.origin().ascii_serialization() != origin
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(AppError::Unavailable);
        }
        let service = state
            .config
            .identity_admin_service_client_id
            .as_deref()
            .ok_or(AppError::Unavailable)?;
        id(service).map_err(|_| AppError::Unavailable)?;
        let secret = state
            .config
            .identity_admin_service_client_secret
            .as_deref()
            .ok_or(AppError::Unavailable)?;
        if !opaque(secret) {
            return Err(AppError::Unavailable);
        }
        Ok(Self {
            client: state
                .config
                .identity_http_client
                .clone()
                .ok_or(AppError::Unavailable)?,
            origin: origin.into(),
            authorization: format!("Basic {}", STANDARD.encode(format!("{service}:{secret}"))),
        })
    }
    fn request(&self, command: &Command) -> Result<reqwest::Request, AppError> {
        let mut request = self
            .client
            .request(
                command.target.method.clone(),
                format!("{}{}", self.origin, command.target.path),
            )
            .header("authorization", &self.authorization)
            .header("accept", "application/json");
        if command.target.mutation() {
            request = request
                .header("x-identity-operation", command.operation.to_string())
                .header("x-identity-operation-digest", &command.digest);
        }
        if command.target.lookup() {
            request = request.header("x-identity-operation-digest", &command.digest);
        }
        if let Some(body) = &command.body {
            request = request.header("content-type", "application/json").body(
                serde_json::to_vec(body).map_err(|_| AppError::bad_request("OIDC 管理请求无效"))?,
            );
        }
        request.build().map_err(|_| AppError::Unavailable)
    }
}
impl Transport for HttpTransport {
    fn send<'a>(&'a self, command: &'a Command) -> AdminFuture<'a, Reply> {
        Box::pin(async move {
            let mut response = self
                .client
                .execute(self.request(command)?)
                .await
                .map_err(|_| AppError::Unavailable)?;
            if response
                .content_length()
                .is_some_and(|length| length > LIMIT as u64)
            {
                return Err(AppError::Unavailable);
            }
            let status = response.status().as_u16();
            let content_type = response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| AppError::Unavailable)? {
                if bytes.len() + chunk.len() > LIMIT {
                    return Err(AppError::Unavailable);
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(Reply {
                status,
                content_type,
                bytes,
            })
        })
    }
}
fn exact(value: &Value, keys: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|o| o.len() == keys.len() && keys.iter().all(|k| o.contains_key(*k)))
}
fn client_projection(value: &Value, created: bool) -> bool {
    let mut keys = vec![
        "id",
        "clientId",
        "name",
        "purpose",
        "subjectMode",
        "clientType",
        "allowedMemberTypes",
        "redirectUris",
        "postLogoutRedirectUris",
        "refreshEnabled",
        "status",
        "accessPolicyVersion",
    ];
    if created {
        keys.push("clientSecret");
    }
    exact(value, &keys)
        && value["id"].as_str().is_some_and(|v| id(v).is_ok())
        && value["clientId"] == value["id"]
        && value["name"]
            .as_str()
            .is_some_and(|v| !v.is_empty() && v.chars().count() <= 120)
        && value["subjectMode"] == "USER"
        && value["clientType"] == "CONFIDENTIAL"
        && value["purpose"]
            .as_str()
            .is_some_and(|v| matches!(v, "ACCOUNT" | "PLATFORM_CONSOLE" | "CUSTOMER_PORTAL"))
        && value["allowedMemberTypes"].as_array().is_some_and(|a| {
            !a.is_empty() && a.len() <= 2 && a.iter().all(|v| v == "EMPLOYEE" || v == "CUSTOMER")
        })
        && ["redirectUris", "postLogoutRedirectUris"].iter().all(|k| {
            value[*k]
                .as_array()
                .is_some_and(|a| a.len() <= 16 && a.iter().all(Value::is_string))
        })
        && value["refreshEnabled"].is_boolean()
        && value["status"]
            .as_str()
            .is_some_and(|v| matches!(v, "ENABLED" | "DISABLED"))
        && value["accessPolicyVersion"]
            .as_i64()
            .is_some_and(|v| v >= 1)
        && (!created || value["clientSecret"].as_str().is_some_and(opaque))
}
fn safe_result(target: &Target, value: &Value) -> bool {
    match (target.method.as_str(), target.path.strip_prefix(PREFIX)) {
        ("GET", Some("/clients")) => {
            exact(value, &["records"])
                && value["records"].as_array().is_some_and(|a| {
                    a.len() <= 200 && a.iter().all(|v| client_projection(v, false))
                })
        }
        ("POST", Some("/clients")) => client_projection(value, true),
        ("PATCH", Some(path)) => {
            client_projection(value, false) && value["id"].as_str() == Some(&path[9..])
        }
        (_, Some(path)) if path.starts_with("/keys/") => {
            let expected = if path == "/keys/publish" {
                "PUBLISHED"
            } else if path == "/keys/activate" {
                "ACTIVE"
            } else if target.method == Method::DELETE {
                "RETIRED"
            } else {
                "REVOKED"
            };
            exact(value, &["kid", "status"])
                && value["kid"].as_str().is_some_and(|v| kid(v).is_ok())
                && value["status"] == expected
                && (path == "/keys/publish"
                    || path == "/keys/activate"
                    || value["kid"].as_str()
                        == Some(if target.method == Method::DELETE {
                            &path[6..]
                        } else {
                            &path[6..path.len() - 7]
                        }))
        }
        _ => false,
    }
}
fn decode(reply: Reply, target: &Target) -> Result<Value, AppError> {
    if !matches!(reply.status, 200 | 201) {
        return Err(match reply.status {
            400 | 422 => AppError::bad_request("OIDC 配置无效"),
            404 => AppError::not_found("OIDC 配置不存在"),
            409 => AppError::conflict("OIDC 配置版本、操作绑定或密钥生命周期已变化"),
            _ => AppError::Unavailable,
        });
    }
    if reply.bytes.len() > LIMIT
        || !reply.content_type.as_deref().is_some_and(|v| {
            v.split(';')
                .next()
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
        })
    {
        return Err(AppError::Unavailable);
    }
    let result = serde_json::from_slice(&reply.bytes).map_err(|_| AppError::Unavailable)?;
    if !safe_result(target, &result) {
        return Err(AppError::Unavailable);
    }
    Ok(result)
}
fn resolve_lookup(
    reply: Reply,
    target: &Target,
    operation: Uuid,
    expired: bool,
) -> Result<Value, AppError> {
    if reply.status == 404 {
        // Identity has taken the mutation's same operation advisory lock before
        // proving absence. After the immutable intent expires, no later new
        // execution can pass its independent database-clock deadline check.
        return if expired {
            Ok(json!({"operationId":operation,"state":"NOT_APPLIED","result":null}))
        } else {
            Err(AppError::Unavailable)
        };
    }
    let result = decode(reply, target)?;
    if target.method == Method::POST
        && target.path == format!("{PREFIX}/clients")
        && result["id"].as_str() != Some(operation.to_string().as_str())
    {
        return Err(AppError::Unavailable);
    }
    Ok(json!({"operationId":operation,"state":"COMPLETED","result":result}))
}
struct Failure {
    error: AppError,
    operation: Option<Uuid>,
}
impl From<AppError> for Failure {
    fn from(error: AppError) -> Self {
        Self {
            error,
            operation: None,
        }
    }
}
impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        if let Some(operation) = self.operation {
            (StatusCode::SERVICE_UNAVAILABLE,Json(json!({"code":"OIDC_REMOTE_RESULT_UNKNOWN","message":"操作结果尚未确认，请查询原操作结果，勿重复提交","operationId":operation}))).into_response()
        } else {
            self.error.into_response()
        }
    }
}
async fn execute(
    boundary: &dyn Boundary,
    transport: &dyn Transport,
    command: &Command,
    deadline: Deadline,
) -> Result<Value, Failure> {
    if tokio::time::Instant::now() >= deadline.at {
        return Err(AppError::Unavailable.into());
    }
    tokio::time::timeout_at(deadline.at, boundary.accept(command))
        .await
        .map_err(|_| AppError::Unavailable)??;
    if tokio::time::Instant::now() >= deadline.at {
        return Err(AppError::Unavailable.into());
    }
    if command.target.mutation() {
        *deadline
            .dispatched
            .lock()
            .map_err(|_| AppError::Unavailable)? = Some(command.operation);
    }
    let result = async {
        let reply = transport.send(command).await?;
        let value = decode(reply, &command.target)?;
        if command.target.method == Method::POST
            && command.target.path == format!("{PREFIX}/clients")
            && value["id"].as_str() != Some(command.operation.to_string().as_str())
        {
            return Err(AppError::Unavailable);
        }
        if matches!(
            command.target.path.strip_prefix(PREFIX),
            Some("/keys/publish" | "/keys/activate")
        ) && value["kid"] != command.body.as_ref().ok_or(AppError::Unavailable)?["kid"]
        {
            return Err(AppError::Unavailable);
        }
        if command.target.mutation() {
            boundary
                .acknowledge(command.operation)
                .await
                .map_err(|_| AppError::Unavailable)?;
        }
        if tokio::time::Instant::now() >= deadline.at {
            return Err(AppError::Unavailable);
        }
        Ok(value)
    };
    match tokio::time::timeout_at(deadline.at, result).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) if !matches!(error, AppError::Unavailable) => Err(error.into()),
        Ok(Err(error)) => Err(Failure {
            error,
            operation: command.target.mutation().then_some(command.operation),
        }),
        Err(_) => Err(Failure {
            error: AppError::Unavailable,
            operation: command.target.mutation().then_some(command.operation),
        }),
    }
}
async fn call(
    state: &AppState,
    user: &CurrentUser,
    headers: &HeaderMap,
    target: Target,
    value: Option<Value>,
    deadline: Deadline,
) -> Result<Json<Value>, Failure> {
    let _permit = state
        .identity_admin_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| AppError::Unavailable)?;
    let value = prepare(&target, value)?;
    let command = Command {
        digest: digest(&target, &value)?,
        target,
        body: value,
        operation: uuid7::new_uuid(),
    };
    Ok(Json(
        execute(
            &PlatformBoundary {
                state,
                user,
                headers,
            },
            &HttpTransport::configured(state)?,
            &command,
            deadline,
        )
        .await?,
    ))
}
async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Extension(d): Extension<Deadline>,
) -> Result<Json<Value>, Failure> {
    call(
        &state,
        &user,
        &headers,
        Target::new(Method::GET, "/clients")?,
        None,
        d,
    )
    .await
}
async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Extension(d): Extension<Deadline>,
    ApiJson(v): ApiJson<Value>,
) -> Result<Json<Value>, Failure> {
    call(
        &state,
        &user,
        &headers,
        Target::new(Method::POST, "/clients")?,
        Some(v),
        d,
    )
    .await
}
async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(raw): Path<String>,
    Extension(d): Extension<Deadline>,
    ApiJson(v): ApiJson<Value>,
) -> Result<Json<Value>, Failure> {
    id(&raw)?;
    call(
        &state,
        &user,
        &headers,
        Target::new(Method::PATCH, &format!("/clients/{raw}"))?,
        Some(v),
        d,
    )
    .await
}
async fn publish(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Extension(d): Extension<Deadline>,
    ApiJson(v): ApiJson<Value>,
) -> Result<Json<Value>, Failure> {
    call(
        &state,
        &user,
        &headers,
        Target::new(Method::POST, "/keys/publish")?,
        Some(v),
        d,
    )
    .await
}
async fn activate(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Extension(d): Extension<Deadline>,
    ApiJson(v): ApiJson<Value>,
) -> Result<Json<Value>, Failure> {
    call(
        &state,
        &user,
        &headers,
        Target::new(Method::POST, "/keys/activate")?,
        Some(v),
        d,
    )
    .await
}
async fn retire(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(raw): Path<String>,
    Extension(d): Extension<Deadline>,
) -> Result<Json<Value>, Failure> {
    kid(&raw)?;
    call(
        &state,
        &user,
        &headers,
        Target::new(Method::DELETE, &format!("/keys/{raw}"))?,
        None,
        d,
    )
    .await
}
async fn revoke(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(raw): Path<String>,
    Extension(d): Extension<Deadline>,
) -> Result<Json<Value>, Failure> {
    kid(&raw)?;
    call(
        &state,
        &user,
        &headers,
        Target::new(Method::POST, &format!("/keys/{raw}/revoke"))?,
        None,
        d,
    )
    .await
}

async fn operation(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(raw): Path<String>,
    Extension(d): Extension<Deadline>,
) -> Result<Json<Value>, Failure> {
    let operation = id(&raw)?;
    let _permit = state
        .identity_admin_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| AppError::Unavailable)?;
    let result = async {
        if !state.config.identity_remote_enabled {
            return Err(AppError::not_found("独立认证管理未启用"));
        }
        state.require(&user, "system:oauth").await?;
        let mut tx = super::clients::admin_tx(&state, &user).await?;
        let row:Option<(String,String,String,Uuid,bool)>=sqlx::query_as("SELECT method,path,request_digest,service_client_id,accepted_at<=clock_timestamp()-interval '60 seconds' FROM platform_oidc_command_intents WHERE id=$1 AND actor_user_id=$2 AND tenant_id=$3 AND organization_id=$4")
            .bind(operation).bind(user.user_id).bind(user.tenant_id).bind(user.org_id).fetch_optional(&mut *tx).await.map_err(|_|AppError::Unavailable)?;
        let (method, path, digest, service, expired) =
            row.ok_or_else(|| AppError::not_found("原 OIDC 操作不存在或不属于当前管理员"))?;
        let target = Target::from_intent(&method, &path)?;
        if !opaque(&digest)
            || Some(service.to_string()).as_deref()
                != state.config.identity_admin_service_client_id.as_deref()
        {
            return Err(AppError::conflict("原操作服务登记已变化，需重新配置"));
        }
        super::clients::admin_audit(
            &mut tx,
            &user,
            &headers,
            "OIDC_REMOTE_RESULT_LOOKUP",
            &json!({"operationId":operation}).to_string(),
        )
        .await?;
        tx.commit().await.map_err(|_| AppError::Unavailable)?;
        *d.dispatched.lock().map_err(|_| AppError::Unavailable)? = Some(operation);
        let command = Command {
            target: Target::new(Method::GET, &format!("/operations/{operation}"))?,
            body: None,
            operation,
            digest,
        };
        let reply = HttpTransport::configured(&state)?.send(&command).await?;
        let value = resolve_lookup(reply, &target, operation, expired)?;
        if value["state"] == "COMPLETED" {
            PlatformBoundary {
                state: &state,
                user: &user,
                headers: &headers,
            }
            .acknowledge(operation)
            .await
            .map_err(|_| AppError::Unavailable)?;
        }
        Ok(value)
    };
    match tokio::time::timeout_at(d.at, result).await {
        Ok(Ok(value)) => Ok(Json(value)),
        Ok(Err(error)) if !matches!(error, AppError::Unavailable) => Err(error.into()),
        Ok(Err(error)) => Err(Failure {
            error,
            operation: Some(operation),
        }),
        Err(_) => Err(Failure {
            error: AppError::Unavailable,
            operation: Some(operation),
        }),
    }
}

#[cfg(test)]
#[path = "remote_admin_pg_tests.rs"]
mod pg_tests;
#[cfg(test)]
#[path = "remote_admin_tests.rs"]
mod tests;
