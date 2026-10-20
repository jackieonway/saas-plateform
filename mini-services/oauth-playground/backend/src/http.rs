use crate::{
    config::Client,
    session::{Device, Flow, Sessions, Token},
    upstream::{Failure, Upstream},
    Config, ConfigError,
};
use axum::{
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};

struct App {
    config: Config,
    sessions: Mutex<Sessions>,
    upstream: Upstream,
    slots: Semaphore,
}

/// Builds an independently runnable router with bounded process-local sessions.
/// All browser mutations require JSON, an existing session and the exact public
/// Origin. No CORS relaxation or browser-supplied bearer material is supported.
pub fn app(config: Config) -> Result<Router, ConfigError> {
    let upstream = Upstream::new(&config)?;
    let state = Arc::new(App {
        config,
        sessions: Mutex::new(Sessions::default()),
        upstream,
        slots: Semaphore::new(32),
    });
    Ok(Router::new()
        .route("/health", get(health))
        .route("/api/health", get(health))
        .route("/api/config", get(configuration))
        .route(
            "/api/verifier",
            get(|| async { error(StatusCode::NOT_FOUND, "not_found") }),
        )
        .route("/api/{action}", post(dispatch))
        .fallback(|| async { error(StatusCode::NOT_FOUND, "not_found") })
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), protect))
        .with_state(state))
}
async fn health() -> Json<Value> {
    Json(json!({"status":"ok"}))
}

async fn protect(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    let rejected = if request.method() == Method::POST {
        let origins: Vec<_> = request.headers().get_all(header::ORIGIN).iter().collect();
        if origins.len() != 1
            || origins[0].to_str().ok() != Some(app.config.public_origin().as_str())
        {
            Some(error(StatusCode::FORBIDDEN, "origin_rejected"))
        } else if !request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| {
                v.split(';')
                    .next()
                    .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
            })
        {
            Some(error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "invalid_request"))
        } else {
            None
        }
    } else {
        None
    };
    let mut response = match rejected {
        Some(r) => r,
        None => next.run(request).await,
    };
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
    );
    response
}
async fn configuration(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let now = Instant::now();
    let mut sessions = app.sessions.lock().await;
    sessions.cleanup(now);
    let existing = cookie(&headers, &app.config).filter(|id| sessions.entries.contains_key(id));
    let (id, fresh) = match existing {
        Some(id) => (id, false),
        None => match sessions.create(&app.config, now) {
            Some(id) => (id, true),
            None => return error(StatusCode::SERVICE_UNAVAILABLE, "capacity_exceeded"),
        },
    };
    let capability = |index: usize| json!({"enabled":app.config.clients[index].is_some()});
    let mut response=Json(json!({"publicOrigin":app.config.public_origin(),"platformOrigin":app.config.platform_origin(),"scope":app.config.scope,
        "flows":{"authorization_code":capability(0),"device":capability(1),"client_credentials":capability(2),"password":capability(3)}})).into_response();
    if fresh {
        let value = format!(
            "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{}",
            app.config.cookie_name(),
            id,
            app.config.session_ttl.as_secs(),
            if app.config.secure() { "; Secure" } else { "" }
        );
        if let Ok(value) = HeaderValue::from_str(&value) {
            response.headers_mut().insert(header::SET_COOKIE, value);
        }
    }
    response
}
async fn dispatch(
    State(app): State<Arc<App>>,
    Path(action): Path<String>,
    headers: HeaderMap,
    body: Result<Json<Value>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Some(id) = cookie(&headers, &app.config) else {
        return error(StatusCode::UNAUTHORIZED, "invalid_session");
    };
    {
        let mut store = app.sessions.lock().await;
        store.cleanup(Instant::now());
        if !store.entries.contains_key(&id) {
            return error(StatusCode::UNAUTHORIZED, "invalid_session");
        }
    }
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    };
    let Ok(_permit) = app.slots.try_acquire() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "capacity_exceeded");
    };
    let operation = crate::session::random();
    match tokio::time::timeout(
        app.config.timeout,
        execute(&app, &id, &operation, &action, body),
    )
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(failure)) => error(failure.status, failure.code),
        Err(_) => {
            // timeout has dropped the operation future before this lock. Only
            // reservations owned by this request are released; taken tokens,
            // verifiers and device codes remain invalid after unknown results.
            let mut store = app.sessions.lock().await;
            if let Some(session) = store.entries.get_mut(&id) {
                let owned: Vec<_> = session
                    .busy
                    .iter()
                    .filter_map(|(flow, owner)| (owner == &operation).then_some(*flow))
                    .collect();
                for flow in owned {
                    session.busy.remove(&flow);
                    session.tokens.remove(&flow);
                    if flow == Flow::Device {
                        session.device = None;
                    }
                }
            }
            error(StatusCode::BAD_GATEWAY, "upstream_unavailable")
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Callback {
    state: String,
    code: Option<String>,
    error: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Password {
    username: String,
    password: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    flow: Flow,
}
fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, Failure> {
    serde_json::from_value(value).map_err(|_| bad("invalid_request"))
}
fn bad(code: &'static str) -> Failure {
    Failure::new(StatusCode::BAD_REQUEST, code)
}
fn unsupported() -> Failure {
    Failure::new(StatusCode::BAD_REQUEST, "unsupported_flow")
}
fn owner(app: &App, flow: Flow) -> Result<&Client, Failure> {
    app.config.clients[flow.index()]
        .as_ref()
        .ok_or_else(unsupported)
}
async fn execute(
    app: &App,
    id: &str,
    operation: &str,
    action: &str,
    body: Value,
) -> Result<Value, Failure> {
    match action {
        "auth-start" => {
            let _: Empty = parse(body)?;
            auth_start(app, id).await
        }
        "callback" => callback(app, id, operation, parse(body)?).await,
        "device-start" => {
            let _: Empty = parse(body)?;
            device_start(app, id, operation).await
        }
        "device-token" => {
            let _: Empty = parse(body)?;
            device_token(app, id, operation).await
        }
        "cc-token" => {
            let _: Empty = parse(body)?;
            let client = owner(app, Flow::ClientCredentials)?;
            begin_flow(app, id, operation, Flow::ClientCredentials).await?;
            let token = app
                .upstream
                .token(
                    &app.config,
                    client,
                    vec![
                        ("grant_type".into(), "client_credentials".into()),
                        ("scope".into(), app.config.scope.clone()),
                    ],
                )
                .await;
            finish_flow(app, id, operation, Flow::ClientCredentials, token).await
        }
        "password-token" => {
            let client = owner(app, Flow::Password)?;
            let input: Password = parse(body)?;
            if input.username.is_empty()
                || input.username.len() > 200
                || input.password.is_empty()
                || input.password.len() > 4096
            {
                return Err(bad("invalid_request"));
            }
            begin_flow(app, id, operation, Flow::Password).await?;
            let token = app
                .upstream
                .token(
                    &app.config,
                    client,
                    vec![
                        ("grant_type".into(), "password".into()),
                        ("username".into(), input.username),
                        ("password".into(), input.password),
                        ("scope".into(), app.config.scope.clone()),
                    ],
                )
                .await;
            finish_flow(app, id, operation, Flow::Password, token).await
        }
        "refresh-token" => {
            let selection: Selection = parse(body)?;
            refresh(app, id, operation, selection.flow).await
        }
        "userinfo" | "introspect" | "revoke" | "external-menus" | "external-roles" => {
            let selection: Selection = parse(body)?;
            resource(app, id, operation, action, selection.flow).await
        }
        _ => Err(Failure::new(StatusCode::NOT_FOUND, "not_found")),
    }
}
async fn auth_start(app: &App, id: &str) -> Result<Value, Failure> {
    let client = owner(app, Flow::AuthorizationCode)?;
    let mut sessions = app.sessions.lock().await;
    let (state, verifier) = sessions
        .start_auth(id, &app.config, Instant::now())
        .ok_or_else(|| Failure::new(StatusCode::SERVICE_UNAVAILABLE, "capacity_exceeded"))?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = app
        .config
        .platform
        .join("/oauth/authorize")
        .map_err(|_| bad("invalid_request"))?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", client.id.as_str()),
        ("response_type", "code"),
        ("redirect_uri", &app.config.callback()),
        ("scope", &app.config.scope),
        ("state", &state),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
    ]);
    Ok(json!({"authorizeUrl":url.as_str(),"state":state}))
}
async fn callback(app: &App, id: &str, operation: &str, input: Callback) -> Result<Value, Failure> {
    let client = owner(app, Flow::AuthorizationCode)?;
    if input.state.is_empty() || input.state.len() > 128 {
        return Err(bad("invalid_state"));
    }
    let auth = app
        .sessions
        .lock()
        .await
        .consume_auth(id, &input.state, Instant::now())
        .ok_or_else(|| bad("invalid_state"))?;
    // Consumption precedes both cancellation and exchange, including network
    // failure. An outcome that cannot be confirmed never reuses a verifier.
    if input.error.is_some() {
        return Err(bad("access_denied"));
    }
    let code = input
        .code
        .filter(|v| !v.is_empty() && v.len() <= 4096)
        .ok_or_else(|| bad("invalid_request"))?;
    begin_flow(app, id, operation, Flow::AuthorizationCode).await?;
    let token = app
        .upstream
        .token(
            &app.config,
            client,
            vec![
                ("grant_type".into(), "authorization_code".into()),
                ("code".into(), code),
                ("redirect_uri".into(), app.config.callback()),
                ("code_verifier".into(), auth.verifier),
            ],
        )
        .await;
    finish_flow(app, id, operation, Flow::AuthorizationCode, token).await
}
async fn begin_flow(app: &App, id: &str, operation: &str, flow: Flow) -> Result<(), Failure> {
    let mut store = app.sessions.lock().await;
    store.cleanup(Instant::now());
    let session = store
        .entries
        .get_mut(id)
        .ok_or_else(|| bad("invalid_session"))?;
    if session.busy.contains_key(&flow) {
        return Err(Failure::new(StatusCode::CONFLICT, "flow_busy"));
    }
    session.busy.insert(flow, operation.into());
    session.tokens.remove(&flow);
    Ok(())
}
async fn finish_flow(
    app: &App,
    id: &str,
    operation: &str,
    flow: Flow,
    result: Result<Token, Failure>,
) -> Result<Value, Failure> {
    let mut store = app.sessions.lock().await;
    store.cleanup(Instant::now());
    let session = store
        .entries
        .get_mut(id)
        .ok_or_else(|| bad("invalid_session"))?;
    if session.busy.get(&flow).map(String::as_str) != Some(operation) {
        return Err(bad("flow_busy"));
    }
    session.busy.remove(&flow);
    let token = result?;
    let summary = token.summary(flow);
    session.tokens.insert(flow, token);
    Ok(summary)
}
async fn device_start(app: &App, id: &str, operation: &str) -> Result<Value, Failure> {
    let client = owner(app, Flow::Device)?;
    begin_flow(app, id, operation, Flow::Device).await?;
    {
        let mut store = app.sessions.lock().await;
        if let Some(session) = store.entries.get_mut(id) {
            session.device = None;
        }
    }
    let mut form = vec![
        ("client_id".into(), client.id.clone()),
        ("scope".into(), app.config.scope.clone()),
    ];
    // The device endpoint authenticates a public client by id, while a private
    // configured device client receives the same Basic policy as other flows.
    let result = app
        .upstream
        .request(
            &app.config,
            reqwest::Method::POST,
            "/oauth/device/authorization",
            Some(client),
            &form,
            None,
        )
        .await;
    form.clear();
    let mut store = app.sessions.lock().await;
    store.cleanup(Instant::now());
    let session = store
        .entries
        .get_mut(id)
        .ok_or_else(|| bad("invalid_session"))?;
    session.busy.remove(&Flow::Device);
    let value = result?;
    let device_code = value
        .get("device_code")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 4096)
        .ok_or_else(|| bad("upstream_rejected"))?;
    let user_code = value
        .get("user_code")
        .and_then(Value::as_str)
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 64
                && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        .ok_or_else(|| bad("upstream_rejected"))?;
    let expires = value
        .get("expires_in")
        .and_then(Value::as_u64)
        .filter(|s| *s > 0)
        .ok_or_else(|| bad("upstream_rejected"))?
        .min(600);
    let interval = value
        .get("interval")
        .and_then(Value::as_u64)
        .unwrap_or(5)
        .clamp(1, 60);
    let now = Instant::now();
    session.device = Some(Device {
        code: device_code.into(),
        expires: now + Duration::from_secs(expires),
        next_poll: now + Duration::from_secs(interval),
        interval: Duration::from_secs(interval),
    });
    let uri = format!("{}/device", app.config.platform_origin());
    let mut complete = app
        .config
        .platform
        .join("/device")
        .map_err(|_| bad("upstream_rejected"))?;
    complete
        .query_pairs_mut()
        .append_pair("user_code", user_code);
    Ok(
        json!({"user_code":user_code,"verification_uri":uri,"verification_uri_complete":complete.as_str(),"expires_in":expires,"interval":interval}),
    )
}
async fn device_token(app: &App, id: &str, operation: &str) -> Result<Value, Failure> {
    let client = owner(app, Flow::Device)?;
    let mut device = {
        let mut store = app.sessions.lock().await;
        store.cleanup(Instant::now());
        let session = store
            .entries
            .get_mut(id)
            .ok_or_else(|| bad("invalid_session"))?;
        if session.busy.contains_key(&Flow::Device) {
            return Err(Failure::new(StatusCode::CONFLICT, "flow_busy"));
        }
        let device = session.device.take().ok_or_else(|| bad("expired_token"))?;
        if Instant::now() < device.next_poll {
            session.device = Some(device);
            return Err(Failure::new(StatusCode::TOO_MANY_REQUESTS, "slow_down"));
        }
        session.busy.insert(Flow::Device, operation.into());
        device
    };
    let result = app
        .upstream
        .token(
            &app.config,
            client,
            vec![
                (
                    "grant_type".into(),
                    "urn:ietf:params:oauth:grant-type:device_code".into(),
                ),
                ("device_code".into(), device.code.clone()),
            ],
        )
        .await;
    if let Err(failure) = &result {
        if matches!(failure.code, "authorization_pending" | "slow_down") {
            if failure.code == "slow_down" {
                device.interval =
                    (device.interval + Duration::from_secs(5)).min(Duration::from_secs(60));
            }
            device.next_poll = Instant::now() + device.interval;
            let mut store = app.sessions.lock().await;
            store.cleanup(Instant::now());
            if let Some(session) = store.entries.get_mut(id) {
                if session.device.is_none() && device.expires > Instant::now() {
                    session.device = Some(device);
                }
            }
        }
    }
    finish_flow(app, id, operation, Flow::Device, result).await
}
async fn refresh(app: &App, id: &str, operation: &str, flow: Flow) -> Result<Value, Failure> {
    let client = owner(app, flow)?;
    let token = {
        let mut store = app.sessions.lock().await;
        store.cleanup(Instant::now());
        let session = store
            .entries
            .get_mut(id)
            .ok_or_else(|| bad("invalid_session"))?;
        if session.busy.contains_key(&flow) {
            return Err(Failure::new(StatusCode::CONFLICT, "flow_busy"));
        }
        let token = session
            .tokens
            .remove(&flow)
            .ok_or_else(|| bad("no_token"))?;
        if token.refresh.is_none() {
            session.tokens.insert(flow, token);
            return Err(unsupported());
        }
        session.busy.insert(flow, operation.into());
        token
    };
    let refresh = token.refresh.ok_or_else(unsupported)?;
    let result = app
        .upstream
        .token(
            &app.config,
            client,
            vec![
                ("grant_type".into(), "refresh_token".into()),
                ("refresh_token".into(), refresh),
            ],
        )
        .await;
    finish_flow(app, id, operation, flow, result).await
}
async fn resource(
    app: &App,
    id: &str,
    operation: &str,
    action: &str,
    flow: Flow,
) -> Result<Value, Failure> {
    let client = owner(app, flow)?;
    if matches!(action, "introspect") && client.secret.is_none() {
        return Err(unsupported());
    }
    let token = {
        let mut store = app.sessions.lock().await;
        store.cleanup(Instant::now());
        let session = store
            .entries
            .get_mut(id)
            .ok_or_else(|| bad("invalid_session"))?;
        if session.busy.contains_key(&flow) {
            return Err(Failure::new(StatusCode::CONFLICT, "flow_busy"));
        }
        if action == "revoke" {
            let token = session
                .tokens
                .remove(&flow)
                .ok_or_else(|| bad("no_token"))?;
            session.busy.insert(flow, operation.into());
            token
        } else {
            session
                .tokens
                .get(&flow)
                .filter(|token| token.expires > Instant::now())
                .cloned()
                .ok_or_else(|| bad("no_token"))?
        }
    };
    if action == "revoke" {
        // Remove local material before the request; partial or unknown outcomes
        // cannot restore an old token or race a refresh into the session.
        let mut values = vec![
            ("client_id".into(), client.id.clone()),
            ("token".into(), token.access),
            ("token_type_hint".into(), "access_token".into()),
        ];
        let result = async {
            app.upstream
                .request(
                    &app.config,
                    reqwest::Method::POST,
                    "/oauth/revoke",
                    Some(client),
                    &values,
                    None,
                )
                .await?;
            if let Some(refresh) = token.refresh {
                values[1].1 = refresh;
                values[2].1 = "refresh_token".into();
                app.upstream
                    .request(
                        &app.config,
                        reqwest::Method::POST,
                        "/oauth/revoke",
                        Some(client),
                        &values,
                        None,
                    )
                    .await?;
            }
            Ok::<(), Failure>(())
        }
        .await;
        {
            let mut store = app.sessions.lock().await;
            if let Some(session) = store.entries.get_mut(id) {
                if session.busy.get(&flow).map(String::as_str) == Some(operation) {
                    session.busy.remove(&flow);
                }
            }
        }
        result?;
        return Ok(json!({"revoked":true,"flow":flow}));
    }
    let (method, path, form, bearer) = match action {
        "userinfo" => (
            reqwest::Method::GET,
            "/oauth/userinfo",
            vec![],
            Some(token.access.as_str()),
        ),
        "introspect" => (
            reqwest::Method::POST,
            "/oauth/introspect",
            vec![
                ("client_id".into(), client.id.clone()),
                ("token".into(), token.access.clone()),
            ],
            None,
        ),
        "external-menus" => (
            reqwest::Method::GET,
            "/api/external/menus",
            vec![],
            Some(token.access.as_str()),
        ),
        "external-roles" => (
            reqwest::Method::GET,
            "/api/external/roles",
            vec![],
            Some(token.access.as_str()),
        ),
        _ => return Err(bad("invalid_request")),
    };
    let mut value = app
        .upstream
        .request(
            &app.config,
            method,
            path,
            if action == "introspect" {
                Some(client)
            } else {
                None
            },
            &form,
            bearer,
        )
        .await?;
    sanitize(&mut value);
    Ok(value)
}
fn sanitize(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            fields.retain(|key, _| {
                !matches!(
                    key.to_ascii_lowercase().as_str(),
                    "access_token"
                        | "refresh_token"
                        | "id_token"
                        | "client_secret"
                        | "code_verifier"
                        | "device_code"
                        | "authorization"
                        | "password"
                )
            });
            for value in fields.values_mut() {
                sanitize(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                sanitize(value);
            }
        }
        _ => {}
    }
}
fn cookie(headers: &HeaderMap, config: &Config) -> Option<String> {
    let mut found = None;
    for header in headers.get_all(header::COOKIE) {
        for part in header.to_str().ok()?.split(';') {
            let Some((key, value)) = part.trim().split_once('=') else {
                continue;
            };
            if key == config.cookie_name() {
                if found.is_some()
                    || value.len() != 43
                    || !value
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                {
                    return None;
                }
                found = Some(value.into());
            }
        }
    }
    found
}
fn error(status: StatusCode, code: &'static str) -> Response {
    let description = match code {
        "authorization_pending" => "Device approval is pending.",
        "slow_down" => "Wait before polling again.",
        "unsupported_flow" => "This demonstration capability is not configured or supported.",
        "invalid_state" => "Authorization state is invalid, expired or already consumed.",
        "access_denied" => "Authorization was denied.",
        "invalid_session" => "Load configuration to establish a new session.",
        "origin_rejected" => "The request origin is not allowed.",
        "capacity_exceeded" => "Demonstration capacity is temporarily unavailable.",
        "no_token" => "No usable token exists for this flow.",
        "flow_busy" => "This flow already has an operation in progress.",
        "upstream_unavailable" => "The configured platform request could not be completed.",
        _ => "The demonstration request was rejected.",
    };
    (
        status,
        Json(json!({"error":code,"error_description":description})),
    )
        .into_response()
}
