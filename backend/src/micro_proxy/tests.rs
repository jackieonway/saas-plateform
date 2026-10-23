use super::*;
use axum::{extract::Request, routing::any};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn strips_execution_metadata_without_reencoding_upstream_query() {
    assert_eq!(
        filtered_query(Some(
            "asset=a%2Fb&__saas_execution=e&__saas_lease=l&__platform_entry=r&v=2"
        ))
        .unwrap(),
        Some("asset=a%2Fb&v=2".into())
    );
    assert_eq!(filtered_query(Some("__saas_lease=only")).unwrap(), None);
}

struct Server {
    origin: String,
    task: Option<tokio::task::JoinHandle<()>>,
    process: Option<std::process::Child>,
}

impl Server {
    async fn start(router: Router, responses: Value) -> Self {
        if let Some(node) = std::env::var_os("MICRO_PROXY_TEST_NODE") {
            use std::io::BufRead;
            let mut command = std::process::Command::new(node);
            command
                .arg(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("src/micro_proxy/http_fixture.mjs"),
                )
                .arg(responses.to_string())
                .stdout(std::process::Stdio::piped());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x08000000);
            }
            let mut process = command.spawn().expect("start optional Node HTTP fixture");
            let mut origin = String::new();
            std::io::BufReader::new(process.stdout.take().unwrap())
                .read_line(&mut origin)
                .unwrap();
            assert!(
                origin.starts_with("http://127.0.0.1:"),
                "fixture did not report a loopback origin"
            );
            return Self {
                origin: origin.trim().into(),
                task: None,
                process: Some(process),
            };
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self {
            origin,
            task: Some(task),
            process: None,
        }
    }

    async fn external_requests(&self) -> Vec<(Method, String, HeaderMap)> {
        let rows: Vec<Value> = Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("{}/__requests", self.origin))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        rows.into_iter()
            .map(|row| {
                let mut headers = HeaderMap::new();
                for (key, value) in row["headers"].as_object().unwrap() {
                    if let Some(value) = value.as_str() {
                        headers.insert(
                            axum::http::HeaderName::from_bytes(key.as_bytes()).unwrap(),
                            value.parse().unwrap(),
                        );
                    }
                }
                (
                    Method::from_bytes(row["method"].as_str().unwrap().as_bytes()).unwrap(),
                    row["url"].as_str().unwrap().into(),
                    headers,
                )
            })
            .collect()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
        if let Some(process) = &mut self.process {
            let _ = process.kill();
            let _ = process.wait();
        }
    }
}

fn config(origin: &str) -> Config {
    let mut config = Config::from_values(&Default::default()).unwrap();
    config.micro_gateway_origin = origin.into();
    config.micro_allowed_origins = vec![origin.into()];
    config
}

fn policy(origin: &str) -> FetchPolicy {
    FetchPolicy::new(&config(origin), "development").unwrap()
}

async fn body(response: Response) -> String {
    String::from_utf8(
        axum::body::to_bytes(response.into_body(), MAX_BODY + 1)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}

#[test]
fn directory_mapping_keeps_entry_and_asset_queries_and_rejects_escapes() {
    let policy = policy("http://127.0.0.1:3005");
    let entry = "http://127.0.0.1:3005/child/index.html?locale=zh";
    assert_eq!(
        Target::new(&policy, entry, "", None).unwrap().url.as_str(),
        entry
    );
    assert_eq!(
        Target::new(&policy, entry, "assets/a.js", Some("v=1&tag=a%2Fb"))
            .unwrap()
            .url
            .as_str(),
        "http://127.0.0.1:3005/child/assets/a.js?v=1&tag=a%2Fb"
    );
    assert_eq!(
        Target::new(&policy, entry, "orders/123", None)
            .unwrap()
            .url
            .as_str(),
        "http://127.0.0.1:3005/child/orders/123"
    );
    for path in [
        "../private.js",
        "%2e%2e/private.js",
        "%252e%252e/private.js",
        "assets%2fa.js",
        "/other/app.js",
        "//evil.test/a.js",
        "assets\\a.js",
        "api/users",
        "oauth/token",
        ".env",
        "assets/.git/config",
        "socket.io/",
        "release.json",
        "server.rs",
        "assets/a%00.js",
    ] {
        assert!(Target::new(&policy, entry, path, None).is_err(), "{path}");
    }
    assert!(Target::new(&policy, "https://other.example.test/", "", None).is_err());
    assert!(
        FetchPolicy::new(&config("http://127.0.0.1:3005"), "production")
            .unwrap()
            .resolve(entry, None)
            .is_err()
    );
}

#[test]
fn execution_queries_are_removed_without_reencoding_business_queries() {
    let query = "x=a%2Fb+z&__platform_entry=legacy&__saas_execution=cache-key&locale=zh";
    assert_eq!(
        filtered_query(Some(query)).unwrap().as_deref(),
        Some("x=a%2Fb+z&locale=zh")
    );
    assert_eq!(
        filtered_query(Some("__saas_execution=cache-key")).unwrap(),
        None
    );
    assert!(filtered_query(Some(&"a".repeat(2049))).is_err());
}

#[tokio::test]
async fn actual_http_static_transport_has_no_credentials_and_preserves_mime_head_and_404() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let upstream = Server::start(Router::new().fallback(any(move |request: Request| {
        let recorded = recorded.clone();
        async move {
            recorded.lock().unwrap().push((
                request.method().clone(),
                request.uri().to_string(),
                request.headers().clone(),
            ));
            let (status, mime, text) = if request.uri().path() == "/child/missing.js" {
                (StatusCode::NOT_FOUND, "text/html", "not found")
            } else {
                (
                    StatusCode::OK,
                    "application/javascript",
                    "export const answer = 42;",
                )
            };
            (
                status,
                [
                    ("content-type", mime),
                    ("set-cookie", "platform_session=evil; Path=/"),
                    ("x-request-id", "upstream-platform-id"),
                    ("www-authenticate", "Basic"),
                    ("service-worker-allowed", "/"),
                    ("content-security-policy", "default-src *"),
                    ("etag", "\"static-v1\""),
                ],
                text,
            )
        }
    })), json!({
        "*": {"headers":{"content-type":"application/javascript","set-cookie":"platform_session=evil; Path=/","x-request-id":"upstream-platform-id","www-authenticate":"Basic","service-worker-allowed":"/","content-security-policy":"default-src *","etag":"\"static-v1\""},"body":"export const answer = 42;"},
        "/child/missing.js": {"status":404,"headers":{"content-type":"text/html"},"body":"not found"}
    }))
    .await;
    let config = config(&upstream.origin);
    let entry = format!("{}/child/index.html", upstream.origin);
    let client = proxy_client(&config).unwrap();
    let policy = policy(&upstream.origin);
    let response = fetch_static(
        &client,
        &policy,
        Target::new(&policy, &entry, "assets/a.js", Some("v=3")).unwrap(),
        &Method::GET,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/javascript"
    );
    assert_eq!(response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(response.headers()[header::ETAG], "\"static-v1\"");
    for name in [
        "set-cookie",
        "www-authenticate",
        "service-worker-allowed",
        "x-request-id",
    ] {
        assert!(!response.headers().contains_key(name), "{name}");
    }
    assert!(response.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap()
        .contains("worker-src 'none'"));
    assert_eq!(body(response).await, "export const answer = 42;");
    let response = fetch_static(
        &client,
        &policy,
        Target::new(&policy, &entry, "assets/a.js", None).unwrap(),
        &Method::HEAD,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body(response).await, "");
    let response = fetch_static(
        &client,
        &policy,
        Target::new(&policy, &entry, "missing.js", None).unwrap(),
        &Method::GET,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(body(response).await, "not found");
    let requests = if upstream.process.is_some() {
        upstream.external_requests().await
    } else {
        requests.lock().unwrap().clone()
    };
    assert_eq!(requests[0].1, "/child/assets/a.js?v=3");
    assert_eq!(requests[1].0, Method::HEAD);
    for (_, _, headers) in requests.iter() {
        for name in [
            "cookie",
            "authorization",
            "origin",
            "referer",
            "x-real-ip",
            "x-forwarded-for",
            "x-tenant-id",
            "x-request-id",
        ] {
            assert!(!headers.contains_key(name), "{name}");
        }
        assert_eq!(headers[header::ACCEPT_ENCODING], "identity");
    }
}

#[tokio::test]
async fn redirects_are_revalidated_and_cannot_escape_the_registered_directory_or_origin() {
    let outside_requests = Arc::new(AtomicUsize::new(0));
    let seen = outside_requests.clone();
    let outside = Server::start(
        Router::new().fallback(any(move || {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::Relaxed);
                "must not be fetched"
            }
        })),
        json!({"*":{"body":"must not be fetched"}}),
    )
    .await;
    let outside_origin = outside.origin.clone();
    let external_origin = outside.origin.clone();
    let upstream = Server::start(Router::new().fallback(any(move |request: Request| {
        let outside_origin = outside_origin.clone();
        async move {
            match request.uri().path() {
                "/child/good.js" => (
                    StatusCode::FOUND,
                    [(header::LOCATION, "assets/final.js".to_string())],
                    "",
                )
                    .into_response(),
                "/child/escape.js" => (
                    StatusCode::FOUND,
                    [(header::LOCATION, "/private.js".to_string())],
                    "",
                )
                    .into_response(),
                "/child/other.js" => (
                    StatusCode::FOUND,
                    [(header::LOCATION, format!("{outside_origin}/a.js"))],
                    "",
                )
                    .into_response(),
                "/child/loop.js" => (
                    StatusCode::FOUND,
                    [(header::LOCATION, "loop.js".to_string())],
                    "",
                )
                    .into_response(),
                "/child/spa.js" => (
                    [(header::CONTENT_TYPE, "text/html")],
                    "<html>SPA fallback</html>",
                )
                    .into_response(),
                "/child/data" => (
                    [(header::CONTENT_TYPE, "application/json")],
                    "{\"private\":true}",
                )
                    .into_response(),
                "/child/too-big.js" => (
                    [(header::CONTENT_TYPE, "application/javascript")],
                    // Hyper derives Content-Length from the body's exact size.
                    // An empty body would rewrite the advertised overflow to 0.
                    Body::from(vec![b'x'; MAX_BODY + 1]),
                )
                    .into_response(),
                _ => (
                    [(header::CONTENT_TYPE, "application/javascript")],
                    "window.loaded=true;",
                )
                    .into_response(),
            }
        }
    })), json!({
        "*":{"headers":{"content-type":"application/javascript"},"body":"window.loaded=true;"},
        "/child/good.js":{"status":302,"headers":{"location":"assets/final.js"}},
        "/child/escape.js":{"status":302,"headers":{"location":"/private.js"}},
        "/child/other.js":{"status":302,"headers":{"location":format!("{external_origin}/a.js")}},
        "/child/loop.js":{"status":302,"headers":{"location":"loop.js"}},
        "/child/spa.js":{"headers":{"content-type":"text/html"},"body":"<html>SPA fallback</html>"},
        "/child/data":{"headers":{"content-type":"application/json"},"body":"{\"private\":true}"},
        "/child/too-big.js":{"headers":{"content-type":"application/javascript"},"bodyBytes":MAX_BODY + 1}
    }))
    .await;
    // Even another operator-allowlisted origin is not part of this application's root.
    let mut config = config(&upstream.origin);
    config.micro_allowed_origins.push(outside.origin.clone());
    let policy = FetchPolicy::new(&config, "development").unwrap();
    let client = proxy_client(&config).unwrap();
    let entry = format!("{}/child/index.html", upstream.origin);
    let response = fetch_static(
        &client,
        &policy,
        Target::new(&policy, &entry, "good.js", None).unwrap(),
        &Method::GET,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body(response).await, "window.loaded=true;");
    for path in [
        "escape.js",
        "other.js",
        "loop.js",
        "spa.js",
        "data",
        "too-big.js",
    ] {
        let response = fetch_static(
            &client,
            &policy,
            Target::new(&policy, &entry, path, None).unwrap(),
            &Method::GET,
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY, "{path}");
    }
    assert_eq!(outside_requests.load(Ordering::Relaxed), 0);
    if outside.process.is_some() {
        assert!(outside.external_requests().await.is_empty());
    }
}

#[tokio::test]
async fn resource_body_limit_accepts_exact_size_and_rejects_declared_and_chunked_overflow() {
    let upstream = Server::start(
        Router::new().fallback(any(|request: Request| async move {
            let size = if request.uri().path() == "/child/exact.js" {
                MAX_BODY
            } else {
                MAX_BODY + 1
            };
            let body = Body::from(vec![b'x'; size]);
            let body = if request.uri().path() == "/child/chunked.js" {
                // An unknown body size makes Hyper select valid chunked framing.
                Body::from_stream(body.into_data_stream())
            } else {
                body
            };
            ([(header::CONTENT_TYPE, "application/javascript")], body).into_response()
        })),
        json!({
            "/child/exact.js":{"headers":{"content-type":"application/javascript"},"bodyBytes":MAX_BODY},
            "/child/declared.js":{"headers":{"content-type":"application/javascript"},"bodyBytes":MAX_BODY + 1},
            "/child/chunked.js":{"headers":{"content-type":"application/javascript","transfer-encoding":"chunked"},"bodyBytes":MAX_BODY + 1}
        }),
    )
    .await;
    let config = config(&upstream.origin);
    let policy = FetchPolicy::new(&config, "development").unwrap();
    let client = proxy_client(&config).unwrap();
    let entry = format!("{}/child/index.html", upstream.origin);
    for (path, length, expected) in [
        ("exact.js", Some(MAX_BODY as u64), StatusCode::OK),
        (
            "declared.js",
            Some((MAX_BODY + 1) as u64),
            StatusCode::BAD_GATEWAY,
        ),
        ("chunked.js", None, StatusCode::BAD_GATEWAY),
    ] {
        let wire = client
            .get(format!("{}/child/{path}", upstream.origin))
            .send()
            .await
            .unwrap();
        assert_eq!(wire.status(), StatusCode::OK, "fixture status: {path}");
        assert_eq!(wire.content_length(), length, "fixture wire length: {path}");
        let expected_bytes = if path == "exact.js" {
            MAX_BODY
        } else {
            MAX_BODY + 1
        };
        assert_eq!(
            wire.bytes().await.unwrap().len(),
            expected_bytes,
            "fixture complete body: {path}"
        );
        let response = fetch_static(
            &client,
            &policy,
            Target::new(&policy, &entry, path, None).unwrap(),
            &Method::GET,
        )
        .await;
        assert_eq!(response.status(), expected, "{path}");
        if expected == StatusCode::OK {
            assert_eq!(body(response).await.len(), MAX_BODY, "{path}");
        }
    }
}

#[test]
fn source_rate_and_concurrency_are_bounded() {
    let runtime = ProxyRuntime::new();
    for _ in 0..600 {
        runtime.check_rate("127.0.0.1").unwrap();
    }
    assert!(matches!(
        runtime.check_rate("127.0.0.1"),
        Err(AppError::RateLimited(_))
    ));
    let permits: Vec<_> = (0..24)
        .map(|_| runtime.slots.clone().try_acquire_owned().unwrap())
        .collect();
    assert!(runtime.slots.clone().try_acquire_owned().is_err());
    drop(permits);
    assert!(runtime.slots.clone().try_acquire_owned().is_ok());
}

#[tokio::test]
async fn outer_platform_cors_keeps_anonymous_opaque_static_access_and_missing_proxy_stays_404() {
    use tower::ServiceExt;
    let config = Arc::new(config("http://127.0.0.1:3005"));
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://localhost/unused_proxy_routing_test")
        .unwrap();
    let state = AppState::new(pool, config.clone());
    let router = routes()
        .with_state(state)
        .layer(crate::bootstrap::cors_layer(&config));
    let response = router
        .oneshot(
            axum::http::Request::builder()
                .uri("/micro-proxy/missing-app/")
                .header(header::ORIGIN, "null")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    assert!(!response
        .headers()
        .contains_key(header::ACCESS_CONTROL_ALLOW_CREDENTIALS));
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
}
