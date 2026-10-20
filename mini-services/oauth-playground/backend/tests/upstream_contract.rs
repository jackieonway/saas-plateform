//! Integration contracts use a real local HTTP platform. Browser-facing calls
//! enter the public router, so none of these tests access private session state.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use oauth_playground::{app, Config};
use serde_json::{json, Value};
use tokio::{net::TcpListener, sync::Notify, sync::Semaphore, task::JoinHandle};
use tower::ServiceExt;

const PUBLIC_ORIGIN: &str = "http://127.0.0.1:3010";
const CC_SECRET: &str = "private-cc-secret";
const AUTH_SECRET: &str = "private-auth-secret";
const CC_ACCESS: &str = "upstream-only-cc-access";
const CC_REFRESH: &str = "upstream-only-cc-refresh";
const AUTH_ACCESS: &str = "upstream-only-auth-access";
const ROTATED_ACCESS: &str = "upstream-only-rotated-access";
const ROTATED_REFRESH: &str = "upstream-only-rotated-refresh";
const PRIVATE_DESCRIPTION: &str = "private-upstream-error-description";

const NORMAL: usize = 0;
const FAIL_REFRESH: usize = 1;
const FAIL_REVOKE: usize = 2;
const REDIRECT: usize = 3;
const OVERSIZED: usize = 4;
const TIMEOUT: usize = 5;
const FAIL_VERBOSE: usize = 6;

#[derive(Clone)]
struct Recorded {
    path: String,
    headers: HeaderMap,
    form: HashMap<String, String>,
}

struct PlatformState {
    records: Mutex<Vec<Recorded>>,
    mode: AtomicUsize,
    pause_next: AtomicBool,
    entered: Notify,
    release: Semaphore,
}

struct Platform {
    origin: String,
    state: Arc<PlatformState>,
    server: JoinHandle<()>,
}

impl Platform {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(PlatformState {
            records: Mutex::new(Vec::new()),
            mode: AtomicUsize::new(NORMAL),
            pause_next: AtomicBool::new(false),
            entered: Notify::new(),
            release: Semaphore::new(0),
        });
        let router = Router::new()
            .fallback(platform_request)
            .with_state(state.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self {
            origin,
            state,
            server,
        }
    }

    fn router(&self) -> Router {
        let env = HashMap::from([
            ("OAUTH_PUBLIC_ORIGIN".into(), PUBLIC_ORIGIN.into()),
            ("OAUTH_PLATFORM_ORIGIN".into(), self.origin.clone()),
            ("OAUTH_ALLOW_INSECURE_LOOPBACK".into(), "true".into()),
            ("OAUTH_AUTH_CLIENT_ID".into(), "auth-client".into()),
            ("OAUTH_AUTH_CLIENT_SECRET".into(), AUTH_SECRET.into()),
            ("OAUTH_CC_CLIENT_ID".into(), "cc-client".into()),
            ("OAUTH_CC_CLIENT_SECRET".into(), CC_SECRET.into()),
            ("OAUTH_DEVICE_CLIENT_ID".into(), "public-device".into()),
            ("OAUTH_HTTP_TIMEOUT_SECONDS".into(), "1".into()),
        ]);
        app(Config::from_values(env).unwrap()).unwrap()
    }

    fn records(&self) -> Vec<Recorded> {
        self.state.records.lock().unwrap().clone()
    }

    fn exchanges(&self, grant: &str) -> Vec<Recorded> {
        self.records()
            .into_iter()
            .filter(|r| {
                r.path == "/oauth/token"
                    && r.form.get("grant_type").map(String::as_str) == Some(grant)
            })
            .collect()
    }

    fn set_mode(&self, mode: usize) {
        self.state.mode.store(mode, Ordering::SeqCst);
    }

    fn pause_next_exchange(&self) {
        self.state.pause_next.store(true, Ordering::SeqCst);
    }

    async fn wait_for_exchange(&self) {
        tokio::time::timeout(Duration::from_secs(3), self.state.entered.notified())
            .await
            .expect("platform received the first exchange");
    }
}

impl Drop for Platform {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn platform_request(State(state): State<Arc<PlatformState>>, request: Request) -> Response {
    let path = request.uri().path().to_owned();
    let headers = request.headers().clone();
    let body = to_bytes(request.into_body(), 64 * 1024).await.unwrap();
    let form: HashMap<String, String> = url::form_urlencoded::parse(&body).into_owned().collect();
    state.records.lock().unwrap().push(Recorded {
        path: path.clone(),
        headers,
        form: form.clone(),
    });
    if matches!(path.as_str(), "/oauth/token" | "/oauth/revoke")
        && state.pause_next.swap(false, Ordering::SeqCst)
    {
        state.entered.notify_one();
        state.release.acquire().await.unwrap().forget();
    }
    let mode = state.mode.load(Ordering::SeqCst);
    if path == "/oauth/token" {
        match mode {
            REDIRECT => return (StatusCode::FOUND, [(header::LOCATION, "/redirect-target?secret=private-upstream-error-description")]).into_response(),
            OVERSIZED => return Json(json!({"payload":"x".repeat(256 * 1024 + 1)})).into_response(),
            TIMEOUT => tokio::time::sleep(Duration::from_millis(1500)).await,
            FAIL_VERBOSE => return (StatusCode::BAD_REQUEST, Json(json!({"error":"private_unknown_error","error_description":format!("{CC_SECRET} {AUTH_SECRET} {CC_ACCESS} {PRIVATE_DESCRIPTION}")}))).into_response(),
            FAIL_REFRESH if form.get("grant_type").map(String::as_str) == Some("refresh_token") => {
                return (StatusCode::BAD_REQUEST, Json(json!({"error":"invalid_grant","error_description":CC_REFRESH}))).into_response();
            }
            _ => {}
        }
        let grant = form
            .get("grant_type")
            .map(String::as_str)
            .unwrap_or_default();
        let (access, refresh) = match grant {
            "authorization_code" => (AUTH_ACCESS, CC_REFRESH),
            "refresh_token" => (ROTATED_ACCESS, ROTATED_REFRESH),
            _ => (CC_ACCESS, CC_REFRESH),
        };
        return Json(json!({"access_token":access,"refresh_token":refresh,"token_type":"Bearer","expires_in":900,"scope":"profile"})).into_response();
    }
    match path.as_str() {
        "/oauth/introspect" => Json(json!({"active":true,"client_id":"cc-client","access_token":CC_ACCESS,"nested":{"refresh_token":CC_REFRESH}})).into_response(),
        "/oauth/userinfo" => Json(json!({"sub":"test-user","access_token":CC_ACCESS})).into_response(),
        "/oauth/revoke" if mode == FAIL_REVOKE => (StatusCode::BAD_REQUEST, Json(json!({"error":"invalid_token","error_description":CC_ACCESS}))).into_response(),
        "/oauth/revoke" => Json(json!({})).into_response(),
        "/redirect-target" => Json(json!({"access_token":CC_ACCESS,"refresh_token":CC_REFRESH,"token_type":"Bearer","expires_in":900})).into_response(),
        _ => (StatusCode::NOT_FOUND, Json(json!({"error":"not_found"}))).into_response(),
    }
}

struct Reply {
    status: StatusCode,
    body: Value,
    raw: String,
}

async fn post(router: &Router, cookie: &str, action: &str, body: Value) -> Reply {
    let request = Request::builder()
        .method("POST")
        .uri(format!("/api/{action}"))
        .header(header::ORIGIN, PUBLIC_ORIGIN)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, cookie)
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let raw = String::from_utf8(bytes.to_vec()).unwrap();
    let body = serde_json::from_str(&raw).expect("JSON response");
    Reply { status, body, raw }
}

async fn session(router: &Router) -> String {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .into()
}

fn selection(flow: &str) -> Value {
    json!({"flow":flow})
}

fn assert_success(reply: &Reply) {
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.raw);
    assert_private(reply);
}

fn assert_error(reply: &Reply, code: &str) {
    assert!(
        reply.status.is_client_error() || reply.status.is_server_error(),
        "{}: {}",
        reply.status,
        reply.raw
    );
    assert_eq!(reply.body["error"], code, "{}", reply.raw);
    assert_private(reply);
}

fn assert_private(reply: &Reply) {
    for forbidden in [
        CC_SECRET,
        AUTH_SECRET,
        CC_ACCESS,
        CC_REFRESH,
        AUTH_ACCESS,
        ROTATED_ACCESS,
        ROTATED_REFRESH,
        PRIVATE_DESCRIPTION,
        "access_token",
        "refresh_token",
        "code_verifier",
    ] {
        assert!(
            !reply.raw.contains(forbidden),
            "browser response leaked {forbidden}: {}",
            reply.raw
        );
    }
}

fn assert_basic(record: &Recorded, client: &str, secret: &str) {
    assert_eq!(
        record.headers[header::AUTHORIZATION],
        format!("Basic {}", STANDARD.encode(format!("{client}:{secret}")))
    );
    assert_eq!(
        record.form.get("client_id").map(String::as_str),
        Some(client)
    );
    assert!(record.headers[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .starts_with("application/x-www-form-urlencoded"));
}

#[tokio::test]
async fn cc_tokens_are_owned_by_their_client_and_session_and_never_returned_to_browser() {
    let platform = Platform::start().await;
    let router = platform.router();
    let cookie = session(&router).await;
    let other = session(&router).await;
    let issued = post(&router, &cookie, "cc-token", json!({})).await;
    assert_success(&issued);
    assert_eq!(issued.body["granted"], true);
    assert_eq!(issued.body["hasRefreshToken"], true);
    assert_eq!(issued.body["flow"], "client_credentials");
    let grants = platform.exchanges("client_credentials");
    assert_eq!(grants.len(), 1);
    assert_basic(&grants[0], "cc-client", CC_SECRET);

    let introspected = post(
        &router,
        &cookie,
        "introspect",
        selection("client_credentials"),
    )
    .await;
    assert_success(&introspected);
    assert_eq!(introspected.body["active"], true);
    let introspect = platform
        .records()
        .into_iter()
        .find(|r| r.path == "/oauth/introspect")
        .unwrap();
    assert_basic(&introspect, "cc-client", CC_SECRET);
    assert_eq!(
        introspect.form.get("token").map(String::as_str),
        Some(CC_ACCESS)
    );

    let userinfo = post(
        &router,
        &cookie,
        "userinfo",
        selection("client_credentials"),
    )
    .await;
    assert_success(&userinfo);
    assert_eq!(userinfo.body["sub"], "test-user");
    let userinfo = platform
        .records()
        .into_iter()
        .find(|r| r.path == "/oauth/userinfo")
        .unwrap();
    assert_eq!(
        userinfo.headers[header::AUTHORIZATION],
        format!("Bearer {CC_ACCESS}")
    );
    let before = platform.records().len();
    for (session, flow) in [
        (&other, "client_credentials"),
        (&cookie, "authorization_code"),
    ] {
        assert_error(
            &post(&router, session, "userinfo", selection(flow)).await,
            "no_token",
        );
    }
    assert_eq!(
        platform.records().len(),
        before,
        "invalid ownership must never call the platform"
    );
}

#[tokio::test]
async fn revoke_is_terminal_locally_even_if_the_platform_rejects_it() {
    let platform = Platform::start().await;
    let router = platform.router();
    let cookie = session(&router).await;
    assert_success(&post(&router, &cookie, "cc-token", json!({})).await);
    platform.set_mode(FAIL_REVOKE);
    assert_error(
        &post(&router, &cookie, "revoke", selection("client_credentials")).await,
        "invalid_token",
    );
    let revoke = platform
        .records()
        .into_iter()
        .find(|r| r.path == "/oauth/revoke")
        .unwrap();
    assert_basic(&revoke, "cc-client", CC_SECRET);
    assert_eq!(
        revoke.form.get("token").map(String::as_str),
        Some(CC_ACCESS)
    );
    let before = platform.records().len();
    for action in ["userinfo", "refresh-token", "revoke"] {
        assert_error(
            &post(&router, &cookie, action, selection("client_credentials")).await,
            "no_token",
        );
    }
    assert_eq!(platform.records().len(), before);
}

#[tokio::test]
async fn in_flight_revoke_cannot_race_refresh_or_reuse_the_removed_token() {
    let platform = Platform::start().await;
    let router = platform.router();
    let cookie = session(&router).await;
    assert_success(&post(&router, &cookie, "cc-token", json!({})).await);
    platform.pause_next_exchange();
    let first_router = router.clone();
    let first_cookie = cookie.clone();
    let revoke = tokio::spawn(async move {
        post(
            &first_router,
            &first_cookie,
            "revoke",
            selection("client_credentials"),
        )
        .await
    });
    platform.wait_for_exchange().await;
    let before = platform.records().len();
    for action in ["refresh-token", "userinfo", "revoke"] {
        let blocked = post(&router, &cookie, action, selection("client_credentials")).await;
        assert_error(&blocked, "flow_busy");
        assert_eq!(blocked.status, StatusCode::CONFLICT);
    }
    let blocked = post(&router, &cookie, "cc-token", json!({})).await;
    assert_error(&blocked, "flow_busy");
    assert_eq!(blocked.status, StatusCode::CONFLICT);
    assert_eq!(platform.records().len(), before);
    platform.state.release.add_permits(1);
    let revoked = revoke.await.unwrap();
    assert_success(&revoked);
    assert_eq!(revoked.body["revoked"], true);
    assert!(platform.exchanges("refresh_token").is_empty());
    let revocations: Vec<_> = platform
        .records()
        .into_iter()
        .filter(|r| r.path == "/oauth/revoke")
        .collect();
    assert_eq!(revocations.len(), 2);
    for (record, token, hint) in [
        (&revocations[0], CC_ACCESS, "access_token"),
        (&revocations[1], CC_REFRESH, "refresh_token"),
    ] {
        assert_basic(record, "cc-client", CC_SECRET);
        assert_eq!(record.form.get("token").map(String::as_str), Some(token));
        assert_eq!(
            record.form.get("token_type_hint").map(String::as_str),
            Some(hint)
        );
    }
    let before = platform.records().len();
    for action in ["userinfo", "refresh-token"] {
        assert_error(
            &post(&router, &cookie, action, selection("client_credentials")).await,
            "no_token",
        );
    }
    assert_eq!(platform.records().len(), before);
}

#[tokio::test]
async fn concurrent_callbacks_exchange_a_state_once_and_send_pkce_only_to_platform() {
    let platform = Platform::start().await;
    let router = platform.router();
    let cookie = session(&router).await;
    let started = post(&router, &cookie, "auth-start", json!({})).await;
    assert_success(&started);
    let authorize = url::Url::parse(started.body["authorizeUrl"].as_str().unwrap()).unwrap();
    let query: HashMap<_, _> = authorize.query_pairs().into_owned().collect();
    assert_eq!(
        query.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
    assert!(!query.contains_key("code_verifier"));
    let input = json!({"state":started.body["state"],"code":"one-use-code"});
    platform.pause_next_exchange();
    let first_router = router.clone();
    let first_cookie = cookie.clone();
    let first_input = input.clone();
    let first =
        tokio::spawn(
            async move { post(&first_router, &first_cookie, "callback", first_input).await },
        );
    platform.wait_for_exchange().await;
    assert_error(
        &post(&router, &cookie, "callback", input.clone()).await,
        "invalid_state",
    );
    platform.state.release.add_permits(1);
    assert_success(&first.await.unwrap());
    let exchanges = platform.exchanges("authorization_code");
    assert_eq!(exchanges.len(), 1);
    let exchange = &exchanges[0];
    assert_basic(exchange, "auth-client", AUTH_SECRET);
    let verifier = exchange
        .form
        .get("code_verifier")
        .expect("server-held PKCE verifier");
    assert!(verifier.len() >= 43);
    use sha2::{Digest, Sha256};
    assert_eq!(
        query.get("code_challenge").unwrap(),
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()))
    );
    assert_eq!(
        exchange.form.get("redirect_uri").map(String::as_str),
        Some("http://127.0.0.1:3010/callback")
    );
    assert_error(
        &post(&router, &cookie, "callback", input).await,
        "invalid_state",
    );
    assert_eq!(platform.exchanges("authorization_code").len(), 1);
}

#[tokio::test]
async fn concurrent_refresh_rotates_once_and_a_failed_refresh_does_not_restore_old_tokens() {
    let platform = Platform::start().await;
    let router = platform.router();
    let cookie = session(&router).await;
    assert_success(&post(&router, &cookie, "cc-token", json!({})).await);
    platform.pause_next_exchange();
    let first_router = router.clone();
    let first_cookie = cookie.clone();
    let first = tokio::spawn(async move {
        post(
            &first_router,
            &first_cookie,
            "refresh-token",
            selection("client_credentials"),
        )
        .await
    });
    platform.wait_for_exchange().await;
    assert_error(
        &post(
            &router,
            &cookie,
            "refresh-token",
            selection("client_credentials"),
        )
        .await,
        "flow_busy",
    );
    platform.state.release.add_permits(1);
    assert_success(&first.await.unwrap());
    let exchanges = platform.exchanges("refresh_token");
    assert_eq!(exchanges.len(), 1);
    assert_basic(&exchanges[0], "cc-client", CC_SECRET);
    assert_eq!(
        exchanges[0].form.get("refresh_token").map(String::as_str),
        Some(CC_REFRESH)
    );
    assert_success(
        &post(
            &router,
            &cookie,
            "userinfo",
            selection("client_credentials"),
        )
        .await,
    );
    let userinfo = platform
        .records()
        .into_iter()
        .find(|r| r.path == "/oauth/userinfo")
        .unwrap();
    assert_eq!(
        userinfo.headers[header::AUTHORIZATION],
        format!("Bearer {ROTATED_ACCESS}")
    );

    platform.set_mode(FAIL_REFRESH);
    assert_error(
        &post(
            &router,
            &cookie,
            "refresh-token",
            selection("client_credentials"),
        )
        .await,
        "invalid_grant",
    );
    let exchanges = platform.exchanges("refresh_token");
    assert_eq!(exchanges.len(), 2);
    assert_eq!(
        exchanges[1].form.get("refresh_token").map(String::as_str),
        Some(ROTATED_REFRESH)
    );
    let before = platform.records().len();
    for action in ["userinfo", "refresh-token"] {
        assert_error(
            &post(&router, &cookie, action, selection("client_credentials")).await,
            "no_token",
        );
    }
    assert_eq!(platform.records().len(), before);
}

#[tokio::test]
async fn upstream_redirects_oversized_bodies_timeouts_and_verbose_errors_are_bounded_and_private() {
    let platform = Platform::start().await;
    let router = platform.router();
    let cookie = session(&router).await;
    for mode in [REDIRECT, OVERSIZED, TIMEOUT, FAIL_VERBOSE] {
        platform.set_mode(mode);
        let rejected = post(&router, &cookie, "cc-token", json!({})).await;
        assert_error(
            &rejected,
            if mode == FAIL_VERBOSE {
                "upstream_rejected"
            } else {
                "upstream_unavailable"
            },
        );
        assert!(
            !rejected.raw.contains(&platform.origin),
            "upstream URL leaked"
        );
        assert_error(
            &post(
                &router,
                &cookie,
                "userinfo",
                selection("client_credentials"),
            )
            .await,
            "no_token",
        );
    }
    assert!(
        platform
            .records()
            .iter()
            .all(|r| r.path != "/redirect-target"),
        "credentials must never follow an upstream redirect"
    );
    assert_eq!(platform.exchanges("client_credentials").len(), 4);
    platform.set_mode(NORMAL);
    assert_success(&post(&router, &cookie, "cc-token", json!({})).await);
    assert_eq!(platform.exchanges("client_credentials").len(), 5);
}

#[tokio::test]
async fn public_device_client_cannot_introspect_with_another_flows_credentials() {
    let platform = Platform::start().await;
    let router = platform.router();
    let cookie = session(&router).await;
    let reply = post(&router, &cookie, "introspect", selection("device")).await;
    assert_error(&reply, "unsupported_flow");
    assert!(platform.records().is_empty());
}
