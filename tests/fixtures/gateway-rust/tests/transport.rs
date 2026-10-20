use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use gateway_rust_transport_fixture::{router, Config};
use serde_json::{json, Value};
use tower::ServiceExt;

fn config() -> Config {
    Config::parse("127.0.0.1:0", "fixture-test", false).unwrap()
}

async fn json_body(response: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await.unwrap()).unwrap()
}

#[test]
fn bind_is_explicit_and_isolated() {
    for address in ["127.0.0.1:3000", "10.4.0.2:3000", "[::1]:3000"] {
        assert!(Config::parse(address, "fixture-a", false).is_ok());
    }
    for address in ["8.8.8.8:3000", "0.0.0.0:3000", "[::]:3000", "invalid"] {
        assert!(Config::parse(address, "fixture-a", false).is_err());
    }
    assert!(Config::parse("0.0.0.0:3000", "fixture-a", true).is_ok());
    assert!(Config::parse("8.8.8.8:3000", "fixture-a", true).is_err());
    assert!(Config::parse("127.0.0.1:0", "contains secret\n", false).is_err());
}

#[tokio::test]
async fn route_prefixes_raw_query_body_and_forwarded_headers_are_observable() {
    for prefix in ["api", "oauth", "work-orders-api"] {
        let uri = format!("/{prefix}/echo?value=a%2Fb&value=c&empty=");
        let response = router(config())
            .oneshot(
                Request::post(&uri)
                    .header(header::AUTHORIZATION, "Bearer fixture-token")
                    .header(header::ORIGIN, "null")
                    .header("x-forwarded-proto", "https")
                    .header("x-forwarded-host", "fixture.saas.test")
                    .header("x-forwarded-for", "192.0.2.17")
                    .body(Body::from("42[\"echo\",{\"text\":\"hello\"}]"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
            "null"
        );
        assert_eq!(
            response
                .headers()
                .get_all(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .iter()
                .count(),
            1
        );
        let body = json_body(response).await;
        assert_eq!(body["uri"], uri);
        assert_eq!(body["method"], "POST");
        assert_eq!(body["query"], "value=a%2Fb&value=c&empty=");
        assert_eq!(body["body"], "42[\"echo\",{\"text\":\"hello\"}]");
        assert_eq!(body["headers"]["authorization"], "Bearer fixture-token");
        assert_eq!(body["headers"]["origin"], "null");
        assert_eq!(body["headers"]["x-forwarded-proto"], "https");
        assert_eq!(body["headers"]["x-forwarded-host"], "fixture.saas.test");
        assert_eq!(body["headers"]["x-forwarded-for"], "192.0.2.17");
        assert_eq!(body["marker"], "TRANSPORT_FIXTURE_ONLY");
    }
}

#[tokio::test]
async fn polling_preserves_raw_get_query_and_post_payload() {
    let uri = "/socket.io/?EIO=4&transport=polling&t=abc%2Bdef";
    let response = router(config())
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/plain; charset=UTF-8"
    );
    let bytes = to_bytes(response.into_body(), 128 * 1024).await.unwrap();
    assert_eq!(bytes[0], b'0');
    let packet: Value = serde_json::from_slice(&bytes[1..]).unwrap();
    assert_eq!(packet["sid"], "fixture-test");
    assert_eq!(packet["fixture"]["uri"], uri);
    assert_eq!(packet["fixture"]["marker"], "TRANSPORT_FIXTURE_ONLY");

    let response = router(config())
        .oneshot(
            Request::post("/socket.io/?EIO=4&transport=polling&sid=fixture-test")
                .body(Body::from("42[\"message\",\"opaque-origin\"]"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = json_body(response).await;
    assert_eq!(body["body"], "42[\"message\",\"opaque-origin\"]");
    assert_eq!(body["query"], "EIO=4&transport=polling&sid=fixture-test");
}

#[tokio::test]
async fn preflight_is_owned_once_by_fixture() {
    let response = router(config())
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/work-orders-api/echo")
                .header(header::ORIGIN, "https://fixture.saas.test")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        "https://fixture.saas.test"
    );
    assert_eq!(response.headers()[header::VARY], "Origin");
    assert!(to_bytes(response.into_body(), 1024)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn rejected_origins_never_receive_cors_grant() {
    for uri in ["/api/echo", "/socket.io/?EIO=4&transport=websocket"] {
        let response = router(config())
            .oneshot(
                Request::get(uri)
                    .header(header::ORIGIN, "https://unapproved.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none());
        assert_eq!(
            json_body(response).await["error"],
            "fixture origin rejected"
        );
    }
}

#[tokio::test]
async fn body_limit_and_unknown_route_fail_closed() {
    let response = router(config())
        .oneshot(
            Request::post("/api/echo")
                .body(Body::from(vec![b'x'; 64 * 1024 + 1]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let response = router(config())
        .oneshot(Request::get("/api/users").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn transport_and_upgrade_are_explicit() {
    for uri in ["/socket.io/", "/socket.io/?transport=websocket"] {
        let response = router(config())
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(response.status().is_client_error());
    }
}

#[tokio::test]
async fn health_records_instance_without_browser_cors_or_business_state() {
    let response = router(config())
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .is_none());
    assert_eq!(
        json_body(response).await,
        json!({ "marker": "TRANSPORT_FIXTURE_ONLY", "instanceId": "fixture-test", "status": "ok" })
    );
}
