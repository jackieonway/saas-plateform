use std::{sync::Arc, time::Duration};

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{header, HeaderMap, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use reqwest::{redirect::Policy, Client, RequestBuilder, Url};
use serde::de::DeserializeOwned;
use serde_json::json;
use tokio::sync::{watch, Semaphore};

use crate::{
    gates::{valid_token, GateInput, Gates, ReleaseInput, DEADLINE},
    Config, Error,
};

const BODY_LIMIT: usize = 4096;
const MAX_INFLIGHT: usize = 32;
const CONTEXT_PATH: &str = "/api/notifications/socket-context";

pub(crate) struct Proxy {
    pub(crate) stopping: watch::Sender<bool>,
    gates: Gates,
    upstream: Url,
    client: Client,
    permits: Arc<Semaphore>,
}

impl Proxy {
    pub(crate) fn new(config: &Config) -> Result<Arc<Self>, Error> {
        let client = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .timeout(DEADLINE)
            .build()
            .map_err(|error| Error::with_source("cannot create upstream client", error))?;
        let (stopping, _) = watch::channel(false);
        Ok(Arc::new(Self {
            stopping,
            gates: Gates::default(),
            upstream: config.upstream.clone(),
            client,
            permits: Arc::new(Semaphore::new(MAX_INFLIGHT)),
        }))
    }

    pub(crate) fn router(self: &Arc<Self>) -> Router {
        Router::new()
            .route(
                CONTEXT_PATH,
                get(forward).head(|| async {
                    failure(StatusCode::METHOD_NOT_ALLOWED, "only GET is supported")
                }),
            )
            .route("/test/gates", post(insert_gate))
            .route("/test/release", post(release_gate))
            .route("/test/state", get(gate_state))
            .fallback(|| async { failure(StatusCode::NOT_FOUND, "fixture route not found") })
            .layer(DefaultBodyLimit::max(BODY_LIMIT))
            .with_state(Arc::clone(self))
    }
}

async fn insert_gate(State(proxy): State<Arc<Proxy>>, bytes: Bytes) -> Response {
    let result = decode::<GateInput>(&bytes).and_then(|input| proxy.gates.insert(input));
    match result {
        Ok(()) => Json(json!({"ok":true})).into_response(),
        Err(message) => failure(StatusCode::BAD_REQUEST, message),
    }
}

async fn release_gate(State(proxy): State<Arc<Proxy>>, bytes: Bytes) -> Response {
    let result = decode::<ReleaseInput>(&bytes).and_then(|input| proxy.gates.release(input));
    match result {
        Ok(()) => Json(json!({"ok":true})).into_response(),
        Err(message) => failure(StatusCode::BAD_REQUEST, message),
    }
}

async fn gate_state(State(proxy): State<Arc<Proxy>>) -> Response {
    match proxy.gates.snapshot() {
        Ok(snapshot) => Json(snapshot).into_response(),
        Err(message) => failure(StatusCode::SERVICE_UNAVAILABLE, message),
    }
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, &'static str> {
    serde_json::from_slice(bytes).map_err(|_| "invalid fixture JSON")
}

async fn forward(State(proxy): State<Arc<Proxy>>, uri: Uri, headers: HeaderMap) -> Response {
    let query = match validate_path(&uri) {
        Ok(query) => query,
        Err(message) => return failure(StatusCode::BAD_REQUEST, message),
    };
    let credentials = match bearer(&headers) {
        Ok(credentials) => credentials,
        Err(message) => return failure(StatusCode::UNAUTHORIZED, message),
    };
    let request_id = headers.get("x-request-id");
    if headers.get_all("x-request-id").iter().count() > 1
        || request_id.is_some_and(|value| value.len() > 128 || value.to_str().is_err())
    {
        return failure(StatusCode::BAD_REQUEST, "invalid request identifier");
    }
    let Ok(_permit) = Arc::clone(&proxy.permits).try_acquire_owned() else {
        return failure(
            StatusCode::TOO_MANY_REQUESTS,
            "fixture upstream capacity exceeded",
        );
    };
    // Missing credentials always reach the real upstream without entering a
    // gate. Each configured gate chooses delivery-only or all-context timing.
    let delivery = query.is_some();
    let gate = match credentials.map_or(Ok(None), |(_, token)| proxy.gates.enter(token, delivery)) {
        Ok(gate) => gate,
        Err(message) => return failure(StatusCode::SERVICE_UNAVAILABLE, message),
    };
    let operation = async {
        if let Some(gate) = &gate {
            gate.delay().await;
        }
        let request = upstream_request(
            &proxy,
            query,
            credentials.map(|(authorization, _)| authorization),
            request_id,
        );
        let mut response = request
            .send()
            .await
            .map_err(|_| "upstream request failed")?;
        let status = response.status();
        if status.is_redirection() {
            return Err("upstream redirect rejected");
        }
        if response
            .content_length()
            .is_some_and(|length| length > BODY_LIMIT as u64)
        {
            return Err("upstream response exceeds fixture body bound");
        }
        let mut bytes = Vec::with_capacity(BODY_LIMIT);
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "upstream response failed")?
        {
            if bytes.len().saturating_add(chunk.len()) > BODY_LIMIT {
                return Err("upstream response exceeds fixture body bound");
            }
            bytes.extend_from_slice(&chunk);
        }
        if credentials.is_some_and(|(_, token)| {
            bytes
                .windows(token.len())
                .any(|part| part == token.as_bytes())
        }) {
            return Err("upstream response contained a credential");
        }
        Ok((status, [(header::CONTENT_TYPE, "application/json")], bytes).into_response())
    };
    let mut stopping = proxy.stopping.subscribe();
    if *stopping.borrow() {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "fixture shutting down");
    }
    tokio::select! {
        _ = stopping.changed() => failure(StatusCode::SERVICE_UNAVAILABLE, "fixture shutting down"),
        result = tokio::time::timeout(DEADLINE, operation) => match result {
            Ok(Ok(response)) => response,
            Ok(Err(message)) => failure(StatusCode::BAD_GATEWAY, message),
            Err(_) => failure(StatusCode::GATEWAY_TIMEOUT, "fixture upstream deadline exceeded"),
        },
    }
}

fn bearer(headers: &HeaderMap) -> Result<Option<(&HeaderValue, &str)>, &'static str> {
    if headers.get_all(header::AUTHORIZATION).iter().count() > 1 {
        return Err("multiple bearer credentials are unsupported");
    }
    let Some(authorization) = headers.get(header::AUTHORIZATION) else {
        return Ok(None);
    };
    let token = authorization
        .to_str()
        .ok()
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| valid_token(value))
        .ok_or("invalid bearer credential")?;
    Ok(Some((authorization, token)))
}

fn upstream_request(
    proxy: &Proxy,
    query: Option<&str>,
    authorization: Option<&HeaderValue>,
    request_id: Option<&HeaderValue>,
) -> RequestBuilder {
    let mut url = proxy.upstream.clone();
    url.set_path(CONTEXT_PATH);
    url.set_query(query);
    let mut request = proxy.client.get(url);
    if let Some(authorization) = authorization {
        request = request.header(header::AUTHORIZATION, authorization);
    }
    if let Some(request_id) = request_id {
        request = request.header("x-request-id", request_id);
    }
    request
}

fn validate_path(uri: &Uri) -> Result<Option<&str>, &'static str> {
    if uri.path() != CONTEXT_PATH {
        return Err("unsupported platform path");
    }
    let Some(query) = uri.query() else {
        return Ok(None);
    };
    let Some(id) = query.strip_prefix("notificationId=") else {
        return Err("unsupported platform query");
    };
    if id.len() != 36
        || !id.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
    {
        return Err("invalid notification identifier");
    }
    Ok(Some(query))
}

fn failure(status: StatusCode, message: &'static str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::{Method, Request},
    };
    use tower::ServiceExt;

    #[test]
    fn health_probe_and_unrecognized_credentials_are_built_for_real_upstream() {
        let proxy = Proxy::new(&Config::parse("http://127.0.0.1:3210", "0").unwrap()).unwrap();
        let headers = HeaderMap::new();
        let credentials = bearer(&headers).unwrap();
        assert!(credentials.is_none());
        let request_id = HeaderValue::from_static("probe-request");
        let request = upstream_request(&proxy, None, None, Some(&request_id))
            .build()
            .unwrap();
        assert_eq!(
            request.url().as_str(),
            "http://127.0.0.1:3210/api/notifications/socket-context"
        );
        assert_eq!(request.method(), reqwest::Method::GET);
        assert!(!request.headers().contains_key(header::AUTHORIZATION));
        assert_eq!(request.headers()["x-request-id"], request_id);

        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer not-a-platform-session"),
        );
        let (authorization, _) = bearer(&headers).unwrap().unwrap();
        let request = upstream_request(&proxy, None, Some(authorization), None)
            .build()
            .unwrap();
        assert_eq!(request.headers()[header::AUTHORIZATION], authorization);
        headers.append(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer second"),
        );
        assert!(bearer(&headers).is_err());
    }

    #[test]
    fn only_exact_context_path_and_single_uuid_query_are_allowed() {
        for path in [
            CONTEXT_PATH,
            "/api/notifications/socket-context?notificationId=12345678-1234-1234-1234-123456789abc",
        ] {
            assert!(validate_path(&path.parse().unwrap()).is_ok());
        }
        for path in ["/api/other", "/api/notifications/socket-context?", "/api/notifications/socket-context?target=http://127.0.0.1", "/api/notifications/socket-context?notificationId=oops", "/api/notifications/socket-context?notificationId=12345678-1234-1234-1234-123456789abc&extra=value", "/api/notifications/socket-context?notificationId=12345678-1234-1234-1234-123456789abc&notificationId=12345678-1234-1234-1234-123456789abc"] {
            assert!(validate_path(&path.parse().unwrap()).is_err(), "{path}");
        }
    }

    #[tokio::test]
    async fn control_errors_and_state_never_echo_credentials() {
        let proxy = Proxy::new(&Config::parse("http://127.0.0.1:1", "0").unwrap()).unwrap();
        let app = proxy.router();
        let response = app.clone().oneshot(Request::builder().method(Method::POST).uri("/test/gates")
            .body(Body::from(r#"{"label":"secret-bearer!","token":"secret-bearer","deliveryOnly":true,"delayMs":1}"#)).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), BODY_LIMIT).await.unwrap();
        assert!(!String::from_utf8_lossy(&body).contains("secret-bearer"));
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/test/gates")
                    .body(Body::from("x".repeat(BODY_LIMIT + 1)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(CONTEXT_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn all_context_gate_holds_initial_auth_until_cancellation_without_network() {
        let proxy = Proxy::new(&Config::parse("http://127.0.0.1:1", "0").unwrap()).unwrap();
        let app = proxy.router();
        let configured = app.clone().oneshot(Request::builder().method(Method::POST)
            .uri("/test/gates").body(Body::from(r#"{"label":"auth-race","token":"platform-session","deliveryOnly":false,"delayMs":15000}"#)).unwrap()).await.unwrap();
        assert_eq!(configured.status(), StatusCode::OK);
        let request = tokio::spawn(
            app.oneshot(
                Request::builder()
                    .uri(CONTEXT_PATH)
                    .header(header::AUTHORIZATION, "Bearer platform-session")
                    .body(Body::empty())
                    .unwrap(),
            ),
        );
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if proxy.gates.snapshot().unwrap()["gates"][0]["inflight"] == 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // Cancel while the real-platform request is held before network I/O.
        proxy.stopping.send_replace(true);
        let response = request.await.unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let snapshot = proxy.gates.snapshot().unwrap();
        assert_eq!(snapshot["gates"][0]["inflight"], 0);
        assert_eq!(snapshot["gates"][0]["count"], 1);
    }

    #[tokio::test]
    async fn outbound_work_is_rejected_at_the_concurrency_bound() {
        let proxy = Proxy::new(&Config::parse("http://127.0.0.1:1", "0").unwrap()).unwrap();
        let _permits = Arc::clone(&proxy.permits)
            .acquire_many_owned(MAX_INFLIGHT as u32)
            .await
            .unwrap();
        let response = proxy
            .router()
            .oneshot(
                Request::builder()
                    .uri(CONTEXT_PATH)
                    .header(header::AUTHORIZATION, "Bearer secret-bearer")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }
}
