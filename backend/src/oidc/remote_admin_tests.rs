use super::*;
use axum::body::{to_bytes, Body};
use axum::http::Request as HttpRequest;
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

fn deadline() -> Deadline {
    Deadline {
        at: tokio::time::Instant::now() + Duration::from_secs(3),
        dispatched: Arc::new(Mutex::new(None)),
    }
}
fn command() -> Command {
    let target = Target::new(Method::POST, "/keys/publish").unwrap();
    let body = Some(json!({"kid":"owned-a"}));
    Command {
        digest: digest(&target, &body).unwrap(),
        target,
        body,
        operation: uuid7::new_uuid(),
    }
}
fn reply(status: u16, value: Value) -> Reply {
    Reply {
        status,
        content_type: Some("application/json; charset=utf-8".into()),
        bytes: serde_json::to_vec(&value).unwrap(),
    }
}
struct FakeBoundary {
    events: Arc<Mutex<Vec<&'static str>>>,
    deny: bool,
    fail_ack: bool,
}
impl Boundary for FakeBoundary {
    fn accept<'a>(&'a self, _: &'a Command) -> AdminFuture<'a, ()> {
        Box::pin(async move {
            if self.deny {
                self.events.lock().unwrap().push("DENIED");
                return Err(AppError::forbidden("denied"));
            }
            self.events.lock().unwrap().push("ACCEPT_COMMIT");
            Ok(())
        })
    }
    fn acknowledge<'a>(&'a self, _: Uuid) -> AdminFuture<'a, ()> {
        Box::pin(async move {
            self.events.lock().unwrap().push(if self.fail_ack {
                "ACK_FAILED"
            } else {
                "ACK_COMMIT"
            });
            if self.fail_ack {
                Err(AppError::Unavailable)
            } else {
                Ok(())
            }
        })
    }
}
struct FakeTransport {
    events: Arc<Mutex<Vec<&'static str>>>,
    calls: AtomicUsize,
    response: Mutex<Option<Reply>>,
}
impl Transport for FakeTransport {
    fn send<'a>(&'a self, _: &'a Command) -> AdminFuture<'a, Reply> {
        Box::pin(async move {
            assert_eq!(self.events.lock().unwrap().first(), Some(&"ACCEPT_COMMIT"));
            self.events.lock().unwrap().push("NETWORK");
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.response
                .lock()
                .unwrap()
                .take()
                .ok_or(AppError::Unavailable)
        })
    }
}
fn ports(deny: bool, fail_ack: bool, response: Option<Reply>) -> (FakeBoundary, FakeTransport) {
    let events = Arc::new(Mutex::new(Vec::new()));
    (
        FakeBoundary {
            events: events.clone(),
            deny,
            fail_ack,
        },
        FakeTransport {
            events,
            calls: AtomicUsize::new(0),
            response: Mutex::new(response),
        },
    )
}
#[test]
fn phase12_remote_admin_fixed_targets_reject_url_and_dot_segment_normalization() {
    for key in [
        ".",
        "..",
        "../clients",
        "%2e%2e",
        "%2f",
        "k?x=1",
        "k#fragment",
        "a/b",
        "a.b",
        "",
    ] {
        assert!(kid(key).is_err());
    }
    for (method, path) in [
        (Method::POST, "/clients/../keys/activate"),
        (Method::DELETE, "/keys/.."),
        (Method::GET, "https://evil.test/clients"),
        (Method::GET, "/clients?x=1"),
        (Method::POST, "/keys/k/revoke/../activate"),
    ] {
        assert!(Target::new(method, path).is_err());
    }
    for (method, path) in [
        (Method::GET, "/clients"),
        (Method::POST, "/clients"),
        (Method::POST, "/keys/publish"),
        (Method::POST, "/keys/activate"),
        (Method::DELETE, "/keys/owned_a-1"),
        (Method::POST, "/keys/owned_a-1/revoke"),
    ] {
        assert!(Target::new(method, path).is_ok());
    }
    let id = uuid7::new_uuid();
    assert!(Target::new(Method::PATCH, &format!("/clients/{id}")).is_ok());
    assert!(Target::from_intent("GET", &format!("{PREFIX}/clients")).is_err());
}
#[test]
fn phase12_remote_admin_digest_uses_raw_body_full_path_and_canonical_keys() {
    let target = Target::new(Method::POST, "/keys/publish").unwrap();
    let value = Some(json!({"kid":"owned-a"}));
    let reference=b"{\"body\":{\"kid\":\"owned-a\"},\"method\":\"POST\",\"path\":\"/internal/identity/oidc/keys/publish\"}";
    assert_eq!(
        digest(&target, &value).unwrap(),
        hex::encode(Sha256::digest(reference))
    );
    let first = Some(json!({"z":[1,2],"a":{"y":null,"x":false}}));
    let second =
        Some(serde_json::from_str::<Value>(r#"{"a":{"x":false,"y":null},"z":[1,2]}"#).unwrap());
    assert_eq!(
        digest(&target, &first).unwrap(),
        digest(&target, &second).unwrap()
    );
    assert_ne!(
        digest(&target, &first).unwrap(),
        digest(&target, &Some(json!({"z":[2,1],"a":{"y":null,"x":false}}))).unwrap()
    );
    assert_eq!(
        digest(&target, &None).unwrap(),
        digest(&target, &Some(Value::Null)).unwrap()
    );
    assert!(prepare(
        &target,
        Some(json!({"kid":"owned-a","privateKeyPem":"must-not-persist"}))
    )
    .is_err());
}
#[test]
fn phase12_remote_admin_outgoing_request_contains_only_its_service_credential() {
    let http = HttpTransport {
        client: reqwest::Client::new(),
        origin: "https://auth.saas.test".into(),
        authorization: "Basic private-test-credential".into(),
    };
    let command = command();
    let request = http.request(&command).unwrap();
    assert_eq!(
        request.url().as_str(),
        format!("https://auth.saas.test{PREFIX}/keys/publish")
    );
    assert_eq!(
        request.headers()[header::AUTHORIZATION],
        "Basic private-test-credential"
    );
    assert!(!request.headers().contains_key(header::COOKIE));
    assert!(!request.headers().contains_key("x-csrf-token"));
    assert_eq!(
        request.headers()["x-identity-operation"],
        command.operation.to_string()
    );
    assert_eq!(
        request.headers()["x-identity-operation-digest"],
        command.digest
    );
    let lookup = Command {
        target: Target::new(Method::GET, &format!("/operations/{}", command.operation)).unwrap(),
        body: None,
        operation: command.operation,
        digest: command.digest,
    };
    let request = http.request(&lookup).unwrap();
    assert!(!request.headers().contains_key("x-identity-operation"));
    assert!(request
        .headers()
        .contains_key("x-identity-operation-digest"));
    assert!(request.body().is_none());
}
#[tokio::test]
async fn phase12_remote_admin_requires_accept_commit_and_ack_commit_before_success() {
    let (boundary, transport) = ports(
        false,
        false,
        Some(reply(200, json!({"kid":"owned-a","status":"PUBLISHED"}))),
    );
    let result = execute(&boundary, &transport, &command(), deadline()).await;
    assert!(result.is_ok());
    assert_eq!(
        *boundary.events.lock().unwrap(),
        ["ACCEPT_COMMIT", "NETWORK", "ACK_COMMIT"]
    );
    let (boundary, transport) = ports(true, false, None);
    assert!(execute(&boundary, &transport, &command(), deadline())
        .await
        .is_err());
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn phase12_remote_admin_unknown_network_or_ack_result_preserves_original_operation() {
    for response in [
        None,
        Some(reply(200, json!({"kid":"owned-a","status":"PUBLISHED"}))),
    ] {
        let fail_ack = response.is_some();
        let (boundary, transport) = ports(false, fail_ack, response);
        let command = command();
        let error = execute(&boundary, &transport, &command, deadline())
            .await
            .err()
            .unwrap();
        assert_eq!(error.operation, Some(command.operation));
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["operationId"], command.operation.to_string());
        assert!(value.get("result").is_none());
        assert!(value.get("clientSecret").is_none());
    }
}
#[tokio::test]
async fn phase12_remote_admin_budget_exhaustion_never_dispatches_and_query_rejected_before_auth() {
    let (boundary, transport) = ports(false, false, None);
    let mut expired = deadline();
    expired.at = tokio::time::Instant::now() - Duration::from_millis(1);
    assert!(execute(&boundary, &transport, &command(), expired)
        .await
        .is_err());
    assert_eq!(transport.calls.load(Ordering::SeqCst), 0);
    assert!(boundary.events.lock().unwrap().is_empty());
    let app = Router::new()
        .route("/bounded", get(|| async { Json(json!({"ok":true})) }))
        .route_layer(middleware::from_fn(bounded));
    let response = app
        .oneshot(
            HttpRequest::builder()
                .uri("/bounded?path=evil")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
}
#[test]
fn phase12_remote_admin_response_contract_rejects_private_fields_and_maps_errors_safely() {
    let target = Target::new(Method::POST, "/keys/publish").unwrap();
    for value in [
        json!({"kid":"owned-a","status":"PUBLISHED","privateKeyPem":"private"}),
        json!({"error":"leaky details"}),
        json!({"kid":"owned-a","status":"ACTIVE"}),
    ] {
        assert!(decode(reply(200, value), &target).is_err());
    }
    for status in [401, 403, 429, 500, 503] {
        assert!(matches!(
            decode(reply(status, json!({"message":"private"})), &target),
            Err(AppError::Unavailable)
        ));
    }
    assert!(matches!(
        decode(reply(409, Value::Null), &target),
        Err(AppError::Conflict(_))
    ));
    assert!(decode(
        Reply {
            status: 200,
            content_type: Some("text/html".into()),
            bytes: vec![]
        },
        &target
    )
    .is_err());
    assert!(decode(
        Reply {
            status: 200,
            content_type: Some("application/json".into()),
            bytes: vec![0; LIMIT + 1]
        },
        &target
    )
    .is_err());
}
#[test]
fn phase12_remote_admin_only_expired_intent_and_locked_receipt_absence_is_not_applied() {
    let operation = uuid7::new_uuid();
    let target = Target::new(Method::POST, "/keys/publish").unwrap();
    assert!(matches!(
        resolve_lookup(reply(404, Value::Null), &target, operation, false),
        Err(AppError::Unavailable)
    ));
    for status in [401, 403, 429, 500, 503] {
        assert!(matches!(
            resolve_lookup(reply(status, Value::Null), &target, operation, true),
            Err(AppError::Unavailable)
        ));
    }
    let absent = resolve_lookup(reply(404, Value::Null), &target, operation, true).unwrap();
    assert_eq!(
        absent,
        json!({"operationId":operation,"state":"NOT_APPLIED","result":null})
    );
    let completed = resolve_lookup(
        reply(200, json!({"kid":"owned-a","status":"PUBLISHED"})),
        &target,
        operation,
        true,
    )
    .unwrap();
    assert_eq!(completed["state"], "COMPLETED");
    assert_eq!(completed["result"]["kid"], "owned-a");
}
