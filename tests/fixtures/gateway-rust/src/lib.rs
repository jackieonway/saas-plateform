//! Test-only HTTP and WebSocket peer for gateway transport verification.
//!
//! This crate has no platform authorization, database or production service API.
//! Its polling response only resembles an Engine.IO open packet. It does not
//! implement an Engine.IO or Socket.IO server and cannot decide Phase 22 compatibility.

use std::net::{IpAddr, SocketAddr};

use axum::{
    body::Bytes,
    extract::{
        ws::{rejection::WebSocketUpgradeRejection, Message, WebSocket, WebSocketUpgrade},
        DefaultBodyLimit, State,
    },
    http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};

const MARKER: &str = "TRANSPORT_FIXTURE_ONLY";
const ALLOWED_ORIGIN: &str = "https://fixture.saas.test";

/// Explicit isolated bind and non-secret identity used to observe container replacement.
#[derive(Clone)]
pub struct Config {
    bind_addr: SocketAddr,
    instance_id: String,
}

impl Config {
    /// Parse explicit configuration without accepting public address binds by default.
    ///
    /// `BIND_ADDR` is mandatory. Loopback and RFC1918 IPv4 are accepted; a container
    /// wildcard requires `ALLOW_CONTAINER_BIND=1`. Its network must remain unpublished.
    /// `INSTANCE_ID` is a non-secret ASCII marker, limited to 64 characters.
    pub fn from_env() -> Result<Self, String> {
        let bind = std::env::var("BIND_ADDR").map_err(|_| "BIND_ADDR is required".to_owned())?;
        let instance = std::env::var("INSTANCE_ID").unwrap_or_else(|_| "fixture-a".to_owned());
        Self::parse(
            &bind,
            &instance,
            std::env::var("ALLOW_CONTAINER_BIND").as_deref() == Ok("1"),
        )
    }

    /// Validate configuration for a test. Rejects public binds and invalid markers.
    pub fn parse(bind: &str, instance_id: &str, allow_container: bool) -> Result<Self, String> {
        let bind_addr: SocketAddr = bind.parse().map_err(|_| "invalid BIND_ADDR".to_owned())?;
        let private = match bind_addr.ip() {
            IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
            IpAddr::V6(ip) => ip.is_loopback(),
        };
        if !private && !(allow_container && bind_addr.ip().is_unspecified()) {
            return Err("BIND_ADDR must be loopback/private; container wildcard requires ALLOW_CONTAINER_BIND=1".to_owned());
        }
        if instance_id.is_empty()
            || instance_id.len() > 64
            || !instance_id
                .bytes()
                .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'-' | b'_' | b'.'))
        {
            return Err(
                "INSTANCE_ID must be 1..64 ASCII letters, digits, dash, dot or underscore"
                    .to_owned(),
            );
        }
        Ok(Self {
            bind_addr,
            instance_id: instance_id.to_owned(),
        })
    }

    /// Return the validated address for the executable's listener.
    pub fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }

    /// Return the fixed non-secret deployment marker.
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }
}

/// Build only the approved transport fixture routes, with a 64 KiB request bound.
///
/// No Origin is accepted for non-browser probes. Browser origins are restricted to
/// `https://fixture.saas.test` and opaque `null`; rejected origins receive no CORS grant.
/// Only the fixture generates business CORS headers, so proxy duplication is observable.
pub fn router(config: Config) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/echo", get(echo).post(echo).options(preflight))
        .route("/oauth/echo", get(echo).post(echo).options(preflight))
        .route(
            "/work-orders-api/echo",
            get(echo).post(echo).options(preflight),
        )
        .route("/socket.io/", get(socket_get).post(echo).options(preflight))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(middleware::from_fn(cors))
        .with_state(config)
}

async fn health(State(config): State<Config>) -> Json<Value> {
    Json(json!({ "marker": MARKER, "instanceId": config.instance_id(), "status": "ok" }))
}

async fn cors(request: axum::extract::Request, next: Next) -> Response {
    let origin = request.headers().get(header::ORIGIN).cloned();
    if let Some(origin) = &origin {
        if origin != ALLOWED_ORIGIN && origin != "null" {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({ "marker": MARKER, "error": "fixture origin rejected" })),
            )
                .into_response();
        }
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("x-transport-fixture", HeaderValue::from_static(MARKER));
    if let Some(origin) = origin {
        let headers = response.headers_mut();
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        headers.insert(header::VARY, HeaderValue::from_static("Origin"));
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, OPTIONS"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("Authorization, Content-Type"),
        );
    }
    response
}

async fn preflight() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn echo(
    State(config): State<Config>,
    uri: Uri,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Json<Value> {
    Json(snapshot(&config, &uri, &method, &headers, &body))
}

fn snapshot(
    config: &Config,
    uri: &Uri,
    method: &Method,
    headers: &HeaderMap,
    body: &[u8],
) -> Value {
    let observed: serde_json::Map<String, Value> = [
        "host",
        "authorization",
        "origin",
        "forwarded",
        "x-forwarded-for",
        "x-forwarded-host",
        "x-forwarded-proto",
        "x-request-id",
        "upgrade",
        "connection",
    ]
    .into_iter()
    .map(|name| {
        (
            name.to_owned(),
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map_or(Value::Null, |value| Value::String(value.to_owned())),
        )
    })
    .collect();
    json!({
        "marker": MARKER,
        "instanceId": config.instance_id(),
        "method": method.as_str(),
        "uri": uri.to_string(),
        "path": uri.path(),
        "query": uri.query(),
        "body": String::from_utf8_lossy(body),
        "bodyBytes": body.len(),
        "headers": observed,
    })
}

async fn socket_get(
    State(config): State<Config>,
    uri: Uri,
    headers: HeaderMap,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    let metadata = snapshot(&config, &uri, &Method::GET, &headers, &[]);
    let transport = uri
        .query()
        .unwrap_or_default()
        .split('&')
        .find_map(|pair| pair.strip_prefix("transport="));
    match transport {
        Some("websocket") => match upgrade {
            Ok(upgrade) => upgrade
                .max_message_size(64 * 1024)
                .max_frame_size(64 * 1024)
                .on_upgrade(move |socket| socket_echo(socket, metadata))
                .into_response(),
            Err(rejection) => rejection.into_response(),
        },
        Some("polling") => (
            [(header::CONTENT_TYPE, "text/plain; charset=UTF-8")],
            format!(
                "0{}",
                json!({
                    "sid": config.instance_id(),
                    "upgrades": ["websocket"],
                    "pingInterval": 25000,
                    "pingTimeout": 20000,
                    "maxPayload": 65536,
                    "fixture": metadata,
                })
            ),
        )
            .into_response(),
        _ => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "marker": MARKER, "error": "transport must be polling or websocket" })),
        )
            .into_response(),
    }
}

async fn socket_echo(mut socket: WebSocket, metadata: Value) {
    if socket
        .send(Message::Text(metadata.to_string().into()))
        .await
        .is_err()
    {
        return;
    }
    while let Some(Ok(message)) = socket.recv().await {
        let reply = match message {
            Message::Text(text) => Message::Text(text),
            Message::Binary(bytes) => Message::Binary(bytes),
            Message::Ping(bytes) => Message::Pong(bytes),
            Message::Close(frame) => {
                let _ = socket.send(Message::Close(frame)).await;
                return;
            }
            Message::Pong(_) => continue,
        };
        if socket.send(reply).await.is_err() {
            return;
        }
    }
}
