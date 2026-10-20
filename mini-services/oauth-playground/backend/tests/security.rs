//! HTTP security regressions exercise the public router without an OAuth server.
//! Platform-backed success paths are covered by the separate integration smoke.

use std::collections::HashMap;

use axum::{
    body::{to_bytes, Body},
    http::{header, HeaderMap, Request, StatusCode},
    Router,
};
use oauth_playground::{app, Config};
use serde_json::{json, Value};
use tower::ServiceExt;

const ORIGIN: &str = "https://playground.example.test";
const AUTH_SECRET: &str = "private-auth-secret-security-regression";
const CC_SECRET: &str = "private-cc-secret-security-regression";

fn values() -> HashMap<String, String> {
    HashMap::from([
        ("OAUTH_PUBLIC_ORIGIN".into(), ORIGIN.into()),
        (
            "OAUTH_PLATFORM_ORIGIN".into(),
            "https://platform.example.test".into(),
        ),
    ])
}

fn router(with_clients: bool) -> Router {
    let mut env = values();
    if with_clients {
        env.extend([
            ("OAUTH_AUTH_CLIENT_ID".into(), "test-web".into()),
            ("OAUTH_AUTH_CLIENT_SECRET".into(), AUTH_SECRET.into()),
            ("OAUTH_DEVICE_CLIENT_ID".into(), "test-device".into()),
            ("OAUTH_CC_CLIENT_ID".into(), "test-service".into()),
            ("OAUTH_CC_CLIENT_SECRET".into(), CC_SECRET.into()),
        ]);
    }
    app(Config::from_values(env).expect("valid test configuration"))
        .expect("valid router configuration")
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
    raw: String,
}

async fn request(
    router: &Router,
    method: &str,
    path: &str,
    origin: Option<&str>,
    cookie: Option<&str>,
    content_type: Option<&str>,
    body: &str,
) -> Reply {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    let response = router
        .clone()
        .oneshot(builder.body(Body::from(body.to_owned())).unwrap())
        .await
        .expect("infallible router");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("bounded response body");
    let raw = String::from_utf8(bytes.to_vec()).expect("UTF-8 response");
    let body = serde_json::from_str(&raw).unwrap_or(Value::Null);
    Reply {
        status,
        headers,
        body,
        raw,
    }
}

async fn post(router: &Router, path: &str, cookie: &str, body: Value) -> Reply {
    request(
        router,
        "POST",
        path,
        Some(ORIGIN),
        Some(cookie),
        Some("application/json"),
        &body.to_string(),
    )
    .await
}

async fn session(router: &Router) -> (String, Reply) {
    let reply = request(router, "GET", "/api/config", None, None, None, "").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.raw);
    let cookie = reply
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| value.to_str().unwrap())
        .find(|value| value.to_ascii_lowercase().contains("httponly"))
        .expect("configuration creates an HttpOnly session cookie")
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    (cookie, reply)
}

fn assert_no_store(reply: &Reply) {
    assert!(
        reply
            .headers
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.split(',').any(|part| part.trim() == "no-store")),
        "sensitive response must use Cache-Control: no-store: {:?}",
        reply.headers
    );
}

fn assert_client_error(reply: &Reply) {
    assert!(
        reply.status.is_client_error(),
        "{}: {}",
        reply.status,
        reply.raw
    );
    assert_no_store(reply);
}

#[tokio::test]
async fn config_exposes_capabilities_and_never_private_credentials() {
    let router = router(false);
    let (_, reply) = session(&router).await;
    assert_no_store(&reply);
    for flow in [
        "authorization_code",
        "device",
        "client_credentials",
        "password",
    ] {
        assert_eq!(
            reply.body["flows"][flow]["enabled"], false,
            "{flow}: {}",
            reply.raw
        );
    }

    let router = self::router(true);
    let (_, reply) = session(&router).await;
    assert_no_store(&reply);
    for forbidden in [AUTH_SECRET, CC_SECRET, "client_secret", "code_verifier"] {
        assert!(
            !reply.raw.contains(forbidden),
            "configuration leaked {forbidden}"
        );
    }
    assert_eq!(reply.body["flows"]["password"]["enabled"], false);
}

#[tokio::test]
async fn session_cookie_is_httponly_lax_and_secure_in_production() {
    let router = router(false);
    let (_, reply) = session(&router).await;
    let cookie = reply.headers[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(cookie.contains("httponly"));
    assert!(cookie.contains("samesite=lax"));
    assert!(cookie.split(';').any(|part| part.trim() == "secure"));
    assert!(cookie.contains("path=/"));

    let mut env = values();
    env.insert("OAUTH_PUBLIC_ORIGIN".into(), "http://127.0.0.1:3014".into());
    env.insert("OAUTH_ALLOW_INSECURE_LOOPBACK".into(), "true".into());
    let local = app(Config::from_values(env).unwrap()).unwrap();
    let (_, reply) = session(&local).await;
    let cookie = reply.headers[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(cookie.contains("httponly"));
    assert!(cookie.contains("samesite=lax"));
    assert!(!cookie.split(';').any(|part| part.trim() == "secure"));
}

#[tokio::test]
async fn mutations_require_exact_origin_and_existing_session() {
    let router = router(true);
    let (cookie, _) = session(&router).await;
    for origin in [
        None,
        Some("null"),
        Some("https://attacker.example.test"),
        Some("https://playground.example.test.attacker.test"),
        Some("http://playground.example.test"),
    ] {
        let reply = request(
            &router,
            "POST",
            "/api/auth-start",
            origin,
            Some(&cookie),
            Some("application/json"),
            "{}",
        )
        .await;
        assert_client_error(&reply);
        assert_eq!(reply.body["error"], "origin_rejected", "{}", reply.raw);
    }
    let reply = request(
        &router,
        "POST",
        "/api/auth-start",
        Some(ORIGIN),
        None,
        Some("application/json"),
        "{}",
    )
    .await;
    assert_client_error(&reply);
}

#[tokio::test]
async fn mutations_reject_non_json_malformed_json_and_unknown_fields() {
    let router = router(true);
    let (cookie, _) = session(&router).await;
    for content_type in [
        None,
        Some("text/plain"),
        Some("application/x-www-form-urlencoded"),
    ] {
        let reply = request(
            &router,
            "POST",
            "/api/auth-start",
            Some(ORIGIN),
            Some(&cookie),
            content_type,
            "{}",
        )
        .await;
        assert_client_error(&reply);
    }
    let malformed = request(
        &router,
        "POST",
        "/api/auth-start",
        Some(ORIGIN),
        Some(&cookie),
        Some("application/json"),
        "{",
    )
    .await;
    assert_client_error(&malformed);
    let unknown = post(
        &router,
        "/api/auth-start",
        &cookie,
        json!({"unexpected":true}),
    )
    .await;
    assert_client_error(&unknown);
}

#[tokio::test]
async fn client_cannot_supply_server_owned_secrets_verifiers_or_tokens() {
    let router = router(true);
    let (cookie, _) = session(&router).await;
    for body in [
        json!({"client_secret":"attacker-secret"}),
        json!({"code_verifier":"attacker-verifier"}),
        json!({"verifier":"attacker-verifier"}),
    ] {
        let reply = post(&router, "/api/auth-start", &cookie, body).await;
        assert_client_error(&reply);
        assert_eq!(reply.body["error"], "invalid_request", "{}", reply.raw);
    }
    for path in [
        "/api/refresh-token",
        "/api/introspect",
        "/api/revoke",
        "/api/userinfo",
        "/api/external-menus",
        "/api/external-roles",
    ] {
        let reply = post(
            &router,
            path,
            &cookie,
            json!({"token":"attacker-token","access_token":"attacker-token","refresh_token":"attacker-token"}),
        )
        .await;
        assert_client_error(&reply);
        assert_ne!(
            reply.status,
            StatusCode::NOT_FOUND,
            "expected existing proxy route {path}"
        );
        assert_eq!(
            reply.body["error"], "invalid_request",
            "{path}: {}",
            reply.raw
        );
        assert!(!reply.raw.contains("attacker-token"));
    }
}

#[tokio::test]
async fn unavailable_flows_and_verifier_enumeration_are_explicitly_rejected() {
    let router = router(false);
    let (cookie, _) = session(&router).await;
    for path in [
        "/api/auth-start",
        "/api/device-start",
        "/api/cc-token",
        "/api/password-token",
    ] {
        let reply = post(&router, path, &cookie, json!({})).await;
        assert_client_error(&reply);
        assert_eq!(
            reply.body["error"], "unsupported_flow",
            "{path}: {}",
            reply.raw
        );
    }
    let verifier = request(
        &router,
        "GET",
        "/api/verifier?state=arbitrary-state",
        None,
        Some(&cookie),
        None,
        "",
    )
    .await;
    assert_eq!(verifier.status, StatusCode::NOT_FOUND);
    assert_no_store(&verifier);
    assert!(!verifier.raw.contains("code_verifier"));
}

#[tokio::test]
async fn authorization_state_is_session_bound_and_consumed_once_even_on_denial() {
    let router = router(true);
    let (first_cookie, _) = session(&router).await;
    let (second_cookie, _) = session(&router).await;
    assert_ne!(first_cookie, second_cookie);
    let started = post(&router, "/api/auth-start", &first_cookie, json!({})).await;
    assert_eq!(started.status, StatusCode::OK, "{}", started.raw);
    assert_no_store(&started);
    let state = started.body["state"]
        .as_str()
        .expect("opaque authorization state");
    assert!(!state.is_empty());
    for forbidden in ["code_verifier", "\"verifier\"", AUTH_SECRET, CC_SECRET] {
        assert!(
            !started.raw.contains(forbidden),
            "authorization start leaked {forbidden}"
        );
    }
    let denied = json!({"state":state,"error":"access_denied"});
    let wrong_session = post(&router, "/api/callback", &second_cookie, denied.clone()).await;
    assert_client_error(&wrong_session);
    assert_eq!(
        wrong_session.body["error"], "invalid_state",
        "{}",
        wrong_session.raw
    );

    // An attempted callback from a different session must not consume this state.
    let genuine_denial = post(&router, "/api/callback", &first_cookie, denied.clone()).await;
    assert_client_error(&genuine_denial);
    assert_eq!(
        genuine_denial.body["error"], "access_denied",
        "{}",
        genuine_denial.raw
    );

    let replay = post(&router, "/api/callback", &first_cookie, denied).await;
    assert_client_error(&replay);
    assert_eq!(replay.body["error"], "invalid_state", "{}", replay.raw);
    let unknown = post(
        &router,
        "/api/callback",
        &first_cookie,
        json!({"state":"unknown-state","error":"access_denied"}),
    )
    .await;
    assert_client_error(&unknown);
    assert_eq!(unknown.body["error"], "invalid_state", "{}", unknown.raw);
}
