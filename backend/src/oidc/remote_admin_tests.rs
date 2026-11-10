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

#[test]
fn client_token_policy_proxy_rejects_unbounded_or_ambiguous_settings() {
    let mut value=json!({"autoAuthorize":false,"accessTokenTtlSeconds":null,"refreshTokenTtlSeconds":null,"clientTokenTtlSeconds":null});
    assert!(client_token_policy_valid(&value,false));
    for invalid in [json!(-1),json!(0),json!(3601),json!("300"),json!(1.5)] {
        value["accessTokenTtlSeconds"]=invalid;
        assert!(!client_token_policy_valid(&value,false));
    }
    value["accessTokenTtlSeconds"]=json!(60);
    value["refreshTokenTtlSeconds"]=json!(300);
    assert!(client_token_policy_valid(&value,false));
    value["autoAuthorize"]=json!("true");
    assert!(!client_token_policy_valid(&value,false));
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
#[test]
fn phase05_browser_operation_handle_is_preserved_and_rejects_ambiguous_headers() {
    let target = Target::auth_client(Method::POST, "").unwrap();
    let operation = uuid7::new_uuid();
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-identity-operation",
        operation.to_string().parse().unwrap(),
    );
    assert_eq!(browser_operation(&headers, &target).unwrap(), operation);
    assert!(operation_started_at(operation).unwrap() <= chrono::Utc::now());
    assert!(browser_operation(&headers, &Target::auth_client(Method::GET, "").unwrap()).is_err());
    headers.append(
        "x-identity-operation",
        operation.to_string().parse().unwrap(),
    );
    assert!(browser_operation(&headers, &target).is_err());
    headers.clear();
    headers.insert("x-identity-operation", "not-a-uuid".parse().unwrap());
    assert!(browser_operation(&headers, &target).is_err());
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
    fn acknowledge<'a>(&'a self, _: Uuid, _: bool, _: bool) -> AdminFuture<'a, ()> {
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
        (Method::POST, "/keys/publish"),
        (Method::POST, "/keys/activate"),
        (Method::DELETE, "/keys/owned_a-1"),
        (Method::POST, "/keys/owned_a-1/revoke"),
    ] {
        assert!(Target::new(method, path).is_ok());
    }
    let id = uuid7::new_uuid();
    assert!(Target::new(Method::PATCH, &format!("/clients/{id}")).is_err());
    assert!(Target::from_intent("GET", &format!("{PREFIX}/clients")).is_err());
    let projection = Target::project(id);
    assert_eq!(projection.path, format!("{AUTH_PREFIX}/applications/{id}"));
    assert!(Target::from_intent("PUT", &projection.path).is_ok());
    assert!(Target::from_intent("POST", &projection.path).is_err());
    assert!(Target::project(Uuid::now_v7()).projection());
    let operation = uuid7::new_uuid();
    let lookup = projection.operation_lookup(operation);
    assert_eq!(lookup.path, format!("{AUTH_PREFIX}/operations/{operation}"));
    assert!(lookup.lookup());
    assert!(!lookup.mutation());
}

#[test]
fn phase03_projection_payload_is_strict_versioned_and_contains_no_business_authority() {
    let application = uuid7::new_uuid();
    let target = Target::project(application);
    let body = json!({
        "applicationId":application,
        "expectedVersion":null,
        "projection":{
            "ownership":"FIRST_PARTY",
            "partnerId":null,
            "status":"APPROVED",
            "displayName":"Platform app",
            "purpose":"Platform application projection",
            "approvedScopes":[]
        }
    });
    assert!(prepare(&target, Some(body.clone())).is_ok());
    assert!(prepare(&target, Some(json!({"applicationId":application,"expectedVersion":null,"projection":{"ownership":"FIRST_PARTY","partnerId":null,"status":"APPROVED","displayName":"Platform app","purpose":"Platform app","approvedScopes":[],"tenantId":"forbidden"}}))).is_err());
    assert!(prepare(&target, Some(json!({"applicationId":uuid7::new_uuid(),"expectedVersion":null,"projection":{"ownership":"FIRST_PARTY","partnerId":null,"status":"APPROVED","displayName":"Platform app","purpose":"Platform app","approvedScopes":[]}}))).is_err());
    let projected = json!({"applicationId":application,"projectionVersion":1,"status":"APPROVED"});
    assert!(safe_result(&target, &projected));
    assert!(!safe_result(
        &target,
        &json!({"applicationId":application,"projectionVersion":1,"status":"APPROVED","tenantId":"must-not-project"})
    ));
    assert!(!safe_result(
        &target,
        &json!({"applicationId":uuid7::new_uuid(),"projectionVersion":1,"status":"APPROVED"})
    ));
}

#[test]
fn phase04_auth_client_targets_use_their_own_receipt_and_service_route() {
    let client = uuid7::new_uuid();
    let operation = uuid7::new_uuid();
    let create = Target::auth_client(Method::POST, "").unwrap();
    assert_eq!(create.path, AUTH_CLIENTS_PREFIX);
    assert!(create.auth_admin());
    assert!(!create.projection());
    assert!(Target::from_intent("POST", &create.path).is_ok());

    let rotate = Target::auth_client(Method::POST, &format!("/{client}/credentials")).unwrap();
    assert_eq!(
        rotate.path,
        format!("{AUTH_CLIENTS_PREFIX}/{client}/credentials")
    );
    let lookup = rotate.operation_lookup(operation);
    assert_eq!(
        lookup.path,
        format!("{AUTH_PREFIX}/client-operations/{operation}")
    );
    assert!(lookup.lookup());
    assert!(lookup.auth_admin());
    assert!(!lookup.projection());
    assert!(Target::from_intent("POST", &rotate.path).is_ok());
    assert!(Target::from_intent(
        "POST",
        &format!("{AUTH_CLIENTS_PREFIX}/{client}/credentials/unbounded")
    )
    .is_err());
}

#[test]
fn phase04_auth_client_proxy_rejects_extra_authority_fields() {
    let create = Target::auth_client(Method::POST, "").unwrap();
    let mut body = json!({
        "expectedVersion":null,
        "applicationId":uuid7::new_uuid(),
        "environment":"test",
        "displayName":"External BFF",
        "clientType":"CONFIDENTIAL",
        "runtimeKind":"WEB_BFF",
        "subjectMode":"USER",
        "contextMode":"ACCOUNT_ONLY",
        "allowedMemberTypes":[],
        "grantTypes":["authorization_code"],
        "tokenEndpointAuthMethod":"client_secret_basic",
        "redirectUris":["https://client.example.test/callback"],
        "postLogoutRedirectUris":[],
        "logoutMode":"SHORT_SESSION_ONLINE_CHECK","backchannelLogoutUri":null,"backchannelTargetId":null,
        "status":"DISABLED",
        "consentGroupId":null,
        "permissions":[{"resourceKey":"userinfo","scopes":["openid"]}]
    });
    assert!(prepare(&create, Some(body.clone())).is_ok());
    body["tenantId"] = json!(uuid7::new_uuid());
    assert!(prepare(&create, Some(body)).is_err());

    let client = uuid7::new_uuid();
    let rotate = Target::auth_client(Method::POST, &format!("/{client}/credentials")).unwrap();
    assert!(prepare(&rotate, Some(json!({"expectedVersion":1}))).is_ok());
    assert!(prepare(
        &rotate,
        Some(json!({"expectedVersion":1,"clientSecret":"caller-controlled"}))
    )
    .is_err());
}

#[test]
fn phase04_auth_client_responses_are_bounded_and_keep_one_time_secret_out_of_lists() {
    let client = uuid7::new_uuid();
    let application = uuid7::new_uuid();
    let record = json!({
    "autoAuthorize":false,"accessTokenTtlSeconds":null,"refreshTokenTtlSeconds":null,"clientTokenTtlSeconds":null,
    "refreshTokenStrategy":"ROTATE","tokenDefaults":{"accessTokenTtlSeconds":300,"refreshTokenTtlSeconds":1800,"offlineRefreshTokenTtlSeconds":86400,"clientTokenTtlSeconds":300},
        "id":client,
        "clientId":client,
        "displayName":"Console BFF",
        "applicationId":application,
        "applicationName":"Platform console",
        "ownership":"FIRST_PARTY",
        "partnerId":null,
        "environment":"test",
        "clientType":"CONFIDENTIAL",
        "runtimeKind":"WEB_BFF",
        "subjectMode":"USER",
        "contextMode":"ACCOUNT_ONLY",
        "allowedMemberTypes":[],
        "grantTypes":["authorization_code"],
        "tokenEndpointAuthMethod":"client_secret_basic",
        "redirectUris":["https://client.example.test/callback"],
        "postLogoutRedirectUris":[],
        "logoutMode":"SHORT_SESSION_ONLINE_CHECK","backchannelLogoutUri":null,"backchannelTargetId":null,
        "status":"ENABLED",
        "version":1,
        "policyVersion":1,
        "createdAt":"2026-10-28T00:00:00Z",
        "updatedAt":"2026-10-28T00:00:00Z",
        "consentGroupId":null,
        "permissions":[{"resourceKey":"userinfo","audience":"https://auth.example.test/userinfo","scopes":["openid"]}]
    });
    let detail = Target::auth_client(Method::GET, &format!("/{client}")).unwrap();
    assert!(safe_result(&detail, &record));
    let mut backchannel = record.clone();
    backchannel["logoutMode"] = json!("BACK_CHANNEL");
    backchannel["backchannelLogoutUri"] =
        json!("https://client.example.test/auth/backchannel-logout");
    backchannel["backchannelTargetId"] = json!(uuid7::new_uuid());
    assert!(safe_result(&detail, &backchannel));
    for (field, value) in [
        (
            "backchannelLogoutUri",
            json!("https://client.example.test/logout?token=bad"),
        ),
        ("backchannelTargetId", Value::Null),
        ("subjectMode", json!("MACHINE")),
        ("logoutMode", json!("SHORT_SESSION_ONLINE_CHECK")),
    ] {
        let mut invalid = backchannel.clone();
        invalid[field] = value;
        assert!(!safe_result(&detail, &invalid));
    }
    assert!(!safe_result(
        &detail,
        &json!({"id":client,"secret":"must-not-return"})
    ));

    let rotate = Target::auth_client(Method::POST, &format!("/{client}/credentials")).unwrap();
    let result = json!({
        "clientId":client,
        "credentialId":uuid7::new_uuid(),
        "credentialVersion":2,
        "clientVersion":3,
        "previousCredentialExpiresAt":null,
        "credentialExpiresAt":"2027-01-26T00:00:00Z",
        "clientSecret":"a".repeat(64)
    });
    assert!(safe_result(&rotate, &result));
    assert!(!safe_result(
        &rotate,
        &json!({"clientId":client,"credentialId":uuid7::new_uuid(),"credentialVersion":2,"clientVersion":3,"previousCredentialExpiresAt":null,"clientSecret":"a".repeat(64),"tenantId":"leak"})
    ));

    let list = Target::auth_client_list(&ClientListQuery {
        application_id: None,
        partner_id: None,
        ownership: None,
        environment: None,
        client_type: None,
        subject_mode: None,
        status: None,
        limit: Some(10),
        offset: Some(0),
    })
    .unwrap();
    let page = json!({"records":[record],"page":{"limit":10,"offset":0,"total":1}});
    assert!(safe_result(&list, &page));
}

#[test]
fn phase25_public_client_identifiers_are_opaque_but_internal_route_ids_stay_bound() {
    let client = uuid7::new_uuid();
    let record = json!({
    "autoAuthorize":false,"accessTokenTtlSeconds":null,"refreshTokenTtlSeconds":null,"clientTokenTtlSeconds":null,
    "refreshTokenStrategy":"ROTATE","tokenDefaults":{"accessTokenTtlSeconds":300,"refreshTokenTtlSeconds":1800,"offlineRefreshTokenTtlSeconds":86400,"clientTokenTtlSeconds":300},
        "id":client,"clientId":client,"displayName":"Owned standard RP",
        "applicationId":uuid7::new_uuid(),"applicationName":"Owned application",
        "ownership":"FIRST_PARTY","partnerId":null,"environment":"test",
        "clientType":"CONFIDENTIAL","runtimeKind":"WEB_BFF","subjectMode":"USER",
        "contextMode":"ACCOUNT_ONLY","allowedMemberTypes":[],"grantTypes":["authorization_code"],
        "tokenEndpointAuthMethod":"client_secret_basic","redirectUris":["https://client.example.test/callback"],
        "postLogoutRedirectUris":[],"logoutMode":"SHORT_SESSION_ONLINE_CHECK",
        "backchannelLogoutUri":null,"backchannelTargetId":null,"status":"ENABLED",
        "version":1,"policyVersion":1,"createdAt":"2026-10-29T00:00:00Z",
        "updatedAt":"2026-10-29T00:00:00Z","consentGroupId":null,"permissions":[]
    });
    let detail = Target::auth_client(Method::GET, &format!("/{client}")).unwrap();
    for public_id in [
        client.to_string(),
        "p21-standard-rp".into(),
        "界".repeat(100),
        " RP with spaces ".into(),
    ] {
        let mut value = record.clone();
        value["clientId"] = json!(public_id);
        assert!(
            safe_result(&detail, &value),
            "valid public client identifier rejected"
        );
    }
    for public_id in [
        Value::Null,
        json!(42),
        json!(""),
        json!("a".repeat(101)),
        json!("界".repeat(101)),
        json!("bad\nclient"),
        json!("bad\rclient"),
        json!("bad\u{0085}client"),
        json!("bad\0client"),
    ] {
        let mut value = record.clone();
        value["clientId"] = public_id;
        assert!(
            !safe_result(&detail, &value),
            "malformed public client identifier accepted"
        );
    }
    for internal_id in [
        json!(uuid7::new_uuid()),
        json!("p21-standard-rp"),
        json!(Uuid::new_v4()),
        Value::Null,
    ] {
        let mut value = record.clone();
        value["id"] = internal_id;
        assert!(
            !safe_result(&detail, &value),
            "public identifier relaxed the internal route binding"
        );
    }
}

#[test]
fn phase04_auth_admin_transport_uses_only_basic_and_bound_operation_headers() {
    let client = uuid7::new_uuid();
    let command = Command {
        target: Target::auth_client(Method::POST, &format!("/{client}/credentials")).unwrap(),
        body: Some(json!({"expectedVersion":2})),
        operation: uuid7::new_uuid(),
        digest: "a".repeat(64),
    };
    let http = HttpTransport {
        client: reqwest::Client::new(),
        origin: "https://identity.saas.test".into(),
        authorization: "Basic isolated-auth-admin-test".into(),
    };
    let request = http.request(&command).unwrap();
    assert_eq!(
        request.url().as_str(),
        format!("https://identity.saas.test{AUTH_CLIENTS_PREFIX}/{client}/credentials")
    );
    assert_eq!(
        request.headers()[header::AUTHORIZATION],
        "Basic isolated-auth-admin-test"
    );
    assert_eq!(
        request.headers()["x-identity-operation"],
        command.operation.to_string()
    );
    assert_eq!(
        request.headers()["x-identity-operation-digest"],
        command.digest
    );
    assert!(!request.headers().contains_key(header::COOKIE));
    assert!(!request.headers().contains_key("x-csrf-token"));
}
#[test]
fn phase12_remote_admin_digest_uses_raw_body_full_path_and_canonical_keys() {
    let target = Target::new(Method::POST, "/keys/publish").unwrap();
    let value = Some(json!({"kid":"owned-a"}));
    let reference=b"{\"body\":{\"kid\":\"owned-a\"},\"method\":\"POST\",\"path\":\"/internal/identity/auth/keys/publish\"}";
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
        target: Target::new(
            Method::GET,
            &format!("/key-operations/{}", command.operation),
        )
        .unwrap(),
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
    for status in [401, 403] {
        assert!(matches!(
            decode(reply(status, json!({"message":"private"})), &target),
            Err(AppError::Forbidden(_))
        ));
    }
    for status in [429, 500, 503] {
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
    for status in [401, 403] {
        assert!(matches!(
            resolve_lookup(reply(status, Value::Null), &target, operation, true),
            Err(AppError::Forbidden(_))
        ));
    }
    for status in [429, 500, 503] {
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

#[test]
fn phase19_review_and_invitation_targets_keep_receipt_contracts_and_metadata_separate() {
    let application = uuid7::new_uuid();
    let operation = uuid7::new_uuid();
    for environment in ["test", "production"] {
        let target = Target::review(Method::PUT, application, environment).unwrap();
        assert!(Target::from_intent("PUT", &target.path).is_ok());
        assert!(Target::from_intent("GET", &target.path).is_err());
        assert_eq!(
            target.operation_lookup(operation).path,
            format!("{AUTH_PREFIX}/review-operations/{operation}")
        );
        assert!(target.operation_lookup(operation).lookup());
        let result = json!({"applicationId":application,"environment":environment,
            "projectionVersion":1,"admissionVersion":1,"status":"APPROVED",
            "requestId":uuid7::new_uuid(),"requestVersion":2});
        assert!(safe_result(&target, &result));
        let mut leak = result.clone();
        leak["clientSecret"] = json!("never-return");
        assert!(!safe_result(&target, &leak));
        let mut wrong = result;
        wrong["environment"] = json!("other");
        assert!(!safe_result(&target, &wrong));
        let current = Target::review(Method::GET, application, environment).unwrap();
        assert!(safe_result(
            &current,
            &json!({"applicationId":application,
            "environment":environment,"projectionVersion":null,"admissionVersion":null,
            "status":"UNREGISTERED"})
        ));
    }
    for environment in ["Test", "production/../test", "test?x=1", ""] {
        assert!(Target::review(Method::PUT, application, environment).is_err());
    }
    let target = Target::invitation(Method::POST, None).unwrap();
    assert!(Target::from_intent("POST", &target.path).is_ok());
    assert_eq!(
        target.operation_lookup(operation).path,
        format!("{AUTH_PREFIX}/invitation-operations/{operation}")
    );
    let body = json!({"actorUserId":uuid7::new_uuid(),"actorSessionId":uuid7::new_uuid(),
        "recipientRef":"recipient@example.test","purpose":"ACCOUNT_ONBOARDING"});
    assert!(prepare(&target, Some(body.clone())).is_ok());
    let mut injected = body;
    injected["token"] = json!("never-browser-controlled");
    assert!(prepare(&target, Some(injected)).is_err());
    let metadata = json!({"id":operation,"recipientRef":"recipient@example.test",
        "purpose":"ACCOUNT_ONBOARDING","status":"ACTIVE","version":1,
        "createdAt":"2026-10-28T00:00:00Z","expiresAt":"2026-10-29T00:00:00Z",
        "revokedAt":null,"consumedAt":null});
    assert!(safe_result(&target, &metadata));
    let mut leaked = metadata.clone();
    leaked["token"] = json!("never-return");
    assert!(!safe_result(&target, &leaked));
    let mut expiry = metadata.clone();
    expiry["expiresAt"] = json!("2026-10-30T00:00:00Z");
    assert!(!safe_result(&target, &expiry));
    let revoke = Target::invitation(Method::POST, Some(uuid7::new_uuid())).unwrap();
    assert!(!safe_result(&revoke, &metadata));
    assert!(Target::from_intent(
        "POST",
        &format!("{INVITATIONS_PREFIX}/{operation}/delivery-material")
    )
    .is_err());
    assert!(
        Target::from_intent("POST", &format!("{INVITATIONS_PREFIX}/{operation}/consume")).is_err()
    );
}

#[test]
fn phase19_internal_sync_metadata_read_uses_only_its_fixed_sensitive_write_permission() {
    let application = uuid7::new_uuid();
    for permission in [
        "system:oauth:review",
        "system:oauth:publish",
        "system:oauth:disable",
    ] {
        let target = Target::review_sync_read(application, "test", permission).unwrap();
        let command = Command {
            target,
            body: None,
            operation: uuid7::new_uuid(),
            digest: String::new(),
        };
        assert_eq!(proposed_permission(&command), permission);
        assert!(!command.target.mutation());
        assert!(command.target.requires_recent());
        let lookup = command.target.operation_lookup(command.operation);
        assert!(lookup.sync_read_permission.is_none());
    }
    for permission in [
        "system:oauth:view",
        "system:oauth:register",
        "system:oauth:scopes",
        "system:oauth:invite",
        "system:oauth",
        "caller-controlled",
    ] {
        assert!(Target::review_sync_read(application, "test", permission).is_err());
    }
    let target = Target::review(Method::GET, application, "test").unwrap();
    let command = Command {
        target,
        body: None,
        operation: uuid7::new_uuid(),
        digest: String::new(),
    };
    assert_eq!(proposed_permission(&command), "system:oauth:view");
    assert!(!command.target.requires_recent());
    let mutation = Target::review(Method::PUT, application, "test").unwrap();
    assert!(Target::from_intent("PUT", &mutation.path)
        .unwrap()
        .sync_read_permission
        .is_none());
}

#[test]
fn phase19_disable_only_authority_cannot_hide_metadata_callback_or_scope_changes() {
    assert!(Target::new(Method::PATCH, &format!("/clients/{}", uuid7::new_uuid())).is_err());
    let current = json!({"displayName":"Owned client","applicationId":uuid7::new_uuid(),
        "environment":"test","clientType":"CONFIDENTIAL","runtimeKind":"WEB_BFF",
        "subjectMode":"USER","contextMode":"ACCOUNT_ONLY","allowedMemberTypes":[],
        "grantTypes":["authorization_code"],"tokenEndpointAuthMethod":"client_secret_basic",
        "redirectUris":["https://client.example.test/callback"],"postLogoutRedirectUris":[],
        "logoutMode":"SHORT_SESSION_ONLINE_CHECK","backchannelLogoutUri":null,"backchannelTargetId":null,
        "consentGroupId":null,"permissions":[{"resourceKey":"userinfo",
            "audience":"https://identity.example.test/userinfo","scopes":["profile","openid"]}]});
    let mut body = current.clone();
    body["expectedVersion"] = json!(1);
    body["status"] = json!("DISABLED");
    body["permissions"] = json!([{"resourceKey":"userinfo","scopes":["openid","profile"]}]);
    assert!(stop_only(&body, &current));
    for (field, replacement) in [
        ("displayName", json!("Changed")),
        (
            "redirectUris",
            json!(["https://other.example.test/callback"]),
        ),
        ("applicationId", json!(uuid7::new_uuid())),
        (
            "permissions",
            json!([{"resourceKey":"userinfo","scopes":["openid"]}]),
        ),
        ("status", json!("ENABLED")),
        ("logoutMode", json!("BACK_CHANNEL")),
        (
            "backchannelLogoutUri",
            json!("https://client.example.test/new-logout"),
        ),
        ("backchannelTargetId", json!(uuid7::new_uuid())),
    ] {
        let mut changed = body.clone();
        changed[field] = replacement;
        assert!(
            !stop_only(&changed, &current),
            "hidden {field} change used disable authority"
        );
    }
    let mut duplicate = body;
    duplicate["permissions"] = json!([{"resourceKey":"userinfo","scopes":["openid","openid"]}]);
    assert!(!stop_only(&duplicate, &current));
}

struct ReviewBoundary {
    events: Arc<Mutex<Vec<&'static str>>>,
    fail_reconcile: bool,
}
impl Boundary for ReviewBoundary {
    fn accept<'a>(&'a self, _: &'a Command) -> AdminFuture<'a, ()> {
        Box::pin(async move {
            self.events.lock().unwrap().push("ACCEPT_COMMIT");
            Ok(())
        })
    }
    fn acknowledge<'a>(&'a self, _: Uuid, _: bool, _: bool) -> AdminFuture<'a, ()> {
        panic!("review reconciliation must own its ACK transaction")
    }
    fn reconcile<'a>(&'a self, _: &'a Command, _: &'a Value) -> AdminFuture<'a, ()> {
        Box::pin(async move {
            self.events.lock().unwrap().push("RECONCILE");
            if self.fail_reconcile {
                Err(AppError::Unavailable)
            } else {
                Ok(())
            }
        })
    }
}

#[tokio::test]
async fn phase19_review_success_requires_reconciliation_and_failure_preserves_original_operation() {
    for fail_reconcile in [false, true] {
        let application = uuid7::new_uuid();
        let events = Arc::new(Mutex::new(Vec::new()));
        let boundary = ReviewBoundary {
            events: events.clone(),
            fail_reconcile,
        };
        let transport = FakeTransport {
            events,
            calls: AtomicUsize::new(0),
            response: Mutex::new(Some(reply(
                200,
                json!({"applicationId":application,
                "environment":"test","projectionVersion":1,"admissionVersion":1,
                "status":"APPROVED","requestId":uuid7::new_uuid(),"requestVersion":2}),
            ))),
        };
        let target = Target::review(Method::PUT, application, "test").unwrap();
        let command = Command {
            target,
            body: None,
            operation: uuid7::new_uuid(),
            digest: "a".repeat(64),
        };
        let result = execute(&boundary, &transport, &command, deadline()).await;
        if fail_reconcile {
            assert_eq!(result.err().unwrap().operation, Some(command.operation));
        } else {
            assert!(result.is_ok());
        }
        assert_eq!(
            *boundary.events.lock().unwrap(),
            ["ACCEPT_COMMIT", "NETWORK", "RECONCILE"]
        );
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1);
    }
    let operation = uuid7::new_uuid();
    let failure = Failure::from(AppError::forbidden("source changed after local commit"))
        .with_operation(operation);
    let response = failure.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["operationId"], operation.to_string());
}

#[test]
fn phase07_consent_group_admin_port_is_fixed_typed_and_recoverable() {
    let application = uuid7::new_uuid();
    let group = uuid7::new_uuid();
    let client = uuid7::new_uuid();
    let list = Target::consent_group_list(application);
    assert_eq!(
        list.route_path(),
        format!("{AUTH_PREFIX}/applications/{application}/consent-groups")
    );
    assert!(list.auth_admin());
    assert!(!list.projection());

    let target = Target::consent_group_create();
    let body = json!({
        "applicationId":application,
        "environment":"production",
        "displayName":"Browser clients",
        "purpose":"Account sign-in",
        "contextMode":"ACCOUNT_ONLY"
    });
    assert_eq!(prepare(&target, Some(body.clone())).unwrap(), Some(body));
    assert!(Target::from_intent("POST", target.route_path()).is_ok());
    let operation = uuid7::new_uuid();
    let created = json!({
        "id":operation,
        "applicationId":application,
        "applicationName":"Account app",
        "environment":"production",
        "displayName":"Browser clients",
        "purpose":"Account sign-in",
        "subjectMode":"USER",
        "contextMode":"ACCOUNT_ONLY",
        "status":"ACTIVE",
        "version":1
    });
    assert!(safe_result(&target, &created));
    let completed = resolve_lookup(reply(200, created.clone()), &target, operation, true).unwrap();
    assert_eq!(completed["state"], "COMPLETED");
    assert_eq!(completed["result"], created);
    assert_eq!(
        target.operation_lookup(operation).route_path(),
        format!("{AUTH_PREFIX}/client-operations/{operation}")
    );

    let listed = json!({
        "groups":[{
            "id":operation,
            "applicationId":application,
            "applicationName":"Account app",
            "environment":"production",
            "displayName":"Browser clients",
            "purpose":"Account sign-in",
            "subjectMode":"USER",
            "contextMode":"ACCOUNT_ONLY",
            "status":"ACTIVE",
            "version":2,
            "clients":[{"id":client,"displayName":"Browser client"}]
        }]
    });
    assert!(safe_result(&list, &listed));
    assert!(!safe_result(
        &list,
        &json!({"groups":[{"secret":"forbidden"}]})
    ));

    let member = Target::consent_group_member(group);
    let member_body = json!({"clientId":client,"expectedGroupVersion":1});
    assert_eq!(
        prepare(&member, Some(member_body.clone())).unwrap(),
        Some(member_body)
    );
    let receipt = json!({"groupId":group,"clientId":client,"version":2});
    assert!(safe_result(&member, &receipt));
    assert!(Target::from_intent("POST", member.route_path()).is_ok());
    let invalid_context = json!({
        "applicationId":application,
        "environment":"production",
        "displayName":"Wrong context",
        "purpose":"Account sign-in",
        "contextMode":"NONE"
    });
    assert!(prepare(&target, Some(invalid_context)).is_err());
    assert!(!safe_result(
        &member,
        &json!({"groupId":client,"clientId":client,"version":2})
    ));
}
