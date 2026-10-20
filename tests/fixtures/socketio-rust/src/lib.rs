//! Minimal loopback fixture for the Phase 22 transport decision.
//! Tokens and notifications are synthetic. No platform API or database is used.

use axum::{
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use socketioxide::{
    extract::{SocketRef, TryData},
    SocketIo,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::watch;

const AUTH_WAIT: Duration = Duration::from_millis(400);

#[derive(Clone, Copy)]
enum Identity {
    A,
    B,
    SameUserTenantB,
    SameUserOrgB,
}

impl Identity {
    fn fields(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::A => ("user-a", "tenant-a", "org-a"),
            Self::B => ("user-b", "tenant-b", "org-b"),
            Self::SameUserTenantB => ("user-a", "tenant-b", "org-a"),
            Self::SameUserOrgB => ("user-a", "tenant-a", "org-b"),
        }
    }
    fn room(self) -> String {
        let (user, tenant, org) = self.fields();
        format!("tenant:{tenant}:org:{org}:user:{user}")
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Authentication {
    token: String,
}

fn token_identity(value: &Value) -> Option<(Identity, Duration)> {
    // serde also accepts a struct as a positional sequence; the wire contract requires an object.
    if !value.is_object() {
        return None;
    }
    let auth: Authentication = serde_json::from_value(value.clone()).ok()?;
    let token = auth.token.trim();
    if token.is_empty()
        || token.len() > 4096
        || token.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return None;
    }
    match token {
        "valid-a" => Some((Identity::A, Duration::from_secs(60))),
        "valid-b" => Some((Identity::B, Duration::from_secs(60))),
        "same-user-tenant-b" => Some((Identity::SameUserTenantB, Duration::from_secs(60))),
        "same-user-org-b" => Some((Identity::SameUserOrgB, Duration::from_secs(60))),
        "expiring" => Some((Identity::A, Duration::from_secs(1))),
        _ => None,
    }
}

fn reject(socket: SocketRef) {
    if socket.connected() {
        let _ = socket.emit(
            "unauthorized",
            &json!({"code":"SESSION_INVALID", "message":"实时通知会话已失效"}),
        );
        let _ = socket.disconnect();
    }
}

async fn connect(socket: SocketRef) {
    let attempted = Arc::new(AtomicBool::new(false));
    let (deadline, mut changes) = watch::channel(Some(Instant::now() + AUTH_WAIT));
    let on_disconnect = deadline.clone();
    socket.on_disconnect(async move || {
        let _ = on_disconnect.send(None);
    });
    let watcher = socket.clone();
    tokio::spawn(async move {
        loop {
            let Some(at) = *changes.borrow_and_update() else {
                return;
            };
            tokio::select! {
                result = changes.changed() => { if result.is_err() { return; } },
                _ = tokio::time::sleep_until(at.into()) => { reject(watcher); return; },
            }
        }
    });
    socket.on(
        "authenticate",
        move |socket: SocketRef, TryData::<Value>(data)| {
            // Socketioxide constructs this future in packet order, then spawns it.
            // Consume the attempt and finish synthetic auth before spawning: an async
            // closure could otherwise let a later credential win the scheduler race.
            let first = !attempted.swap(true, Ordering::AcqRel);
            authenticate(socket, data.ok(), first, &deadline);
            async {}
        },
    );
}

fn authenticate(
    socket: SocketRef,
    data: Option<Value>,
    first: bool,
    deadline: &watch::Sender<Option<Instant>>,
) {
    let Some((identity, lifetime)) = data.as_ref().and_then(token_identity).filter(|_| first)
    else {
        reject(socket);
        return;
    };
    if !socket.connected() {
        return;
    }
    let (user, tenant, org) = identity.fields();
    let expires_at = (Utc::now() + lifetime).to_rfc3339_opts(SecondsFormat::Millis, true);
    socket.join(identity.room());
    let _ = deadline.send(Some(Instant::now() + lifetime));
    let _ = socket.emit(
        "authenticated",
        &json!({
            "contractVersion":"1.0", "userId":user, "tenantId":tenant,
            "organizationId":org, "expiresAt":expires_at,
        }),
    );
}

#[derive(Clone)]
struct Fixture {
    io: SocketIo,
    origin: HeaderValue,
}

async fn origin_guard(State(state): State<Fixture>, request: Request, next: Next) -> Response {
    let socket_path = request.uri().path().starts_with("/socket.io");
    if !socket_path {
        return next.run(request).await;
    }
    let mut origins = request.headers().get_all("origin").iter();
    if origins.next() != Some(&state.origin) || origins.next().is_some() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let preflight = request.method() == Method::OPTIONS;
    let mut response = if preflight {
        StatusCode::NO_CONTENT.into_response()
    } else {
        next.run(request).await
    };
    let headers = response.headers_mut();
    headers.insert("access-control-allow-origin", state.origin);
    headers.insert(
        "access-control-allow-methods",
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    headers.insert(
        "access-control-allow-headers",
        HeaderValue::from_static("content-type"),
    );
    headers.insert("vary", HeaderValue::from_static("Origin"));
    response
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Notification {
    recipient: String,
    id: String,
}

async fn notification(State(state): State<Fixture>, Json(input): Json<Notification>) -> StatusCode {
    let identity = match input.recipient.as_str() {
        "a" => Identity::A,
        "b" => Identity::B,
        "tenant-b" => Identity::SameUserTenantB,
        "org-b" => Identity::SameUserOrgB,
        _ => return StatusCode::BAD_REQUEST,
    };
    if input.id.is_empty() || input.id.len() > 128 {
        return StatusCode::BAD_REQUEST;
    }
    let (user, tenant, org) = identity.fields();
    let event = json!({"contractVersion":"1.0", "id":input.id, "userId":user,
        "tenantId":tenant, "organizationId":org, "applicationId":null, "publicationId":null,
        "type":"INFO", "title":"Rust 通知 🦀", "content":"protocol fixture", "link":"/dashboard",
        "createdAt":Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)});
    match state
        .io
        .to(identity.room())
        .emit("notification", &event)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// Build the test peer and its shutdown handle. The caller must bind to loopback.
/// `origin` must be an exact credential-free browser origin; malformed values fail.
pub fn router(origin: String) -> Result<(Router, SocketIo), &'static str> {
    if !origin.starts_with("http://127.0.0.1:") || origin[17..].parse::<u16>().is_err() {
        return Err("fixture requires an exact IPv4 loopback HTTP origin");
    }
    let origin = HeaderValue::from_str(&origin).map_err(|_| "invalid origin")?;
    let (layer, io) = SocketIo::builder()
        .req_path("/socket.io")
        .ping_interval(Duration::from_millis(200))
        .ping_timeout(Duration::from_millis(500))
        .max_payload(8192)
        .max_buffer_size(32)
        .build_layer();
    io.ns("/", connect);
    let state = Fixture {
        io: io.clone(),
        origin,
    };
    let app = Router::new()
        .route(
            "/health/live",
            get(|| async { Json(json!({"status":"UP", "fixture":true})) }),
        )
        .route("/__fixture__/notification", post(notification))
        .with_state(state.clone())
        .layer(layer)
        .layer(middleware::from_fn_with_state(state, origin_guard));
    Ok((app, io))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authentication_is_strict_and_has_no_identity_override() {
        assert!(token_identity(&json!({"token":" valid-a "})).is_some());
        for value in [
            json!({"token":"expired"}),
            json!({"token":"invalid"}),
            json!({"token":"valid-a", "userId":"user-b"}),
            json!({"token":42}),
            json!(["valid-a"]),
            json!({"token":"valid- a"}),
            json!({"token":"\u{7f}"}),
            json!({"token":"x".repeat(4097)}),
        ] {
            assert!(token_identity(&value).is_none());
        }
    }

    #[tokio::test]
    async fn origins_reject_polling_and_upgrade_before_engine_layer() {
        use tower::ServiceExt;
        let (app, io) = router("http://127.0.0.1:3000".into()).unwrap();
        for transport in ["polling", "websocket"] {
            for origin in [None, Some("null"), Some("http://127.0.0.1:3001")] {
                let mut request =
                    Request::builder().uri(format!("/socket.io/?EIO=4&transport={transport}"));
                if let Some(origin) = origin {
                    request = request.header("origin", origin);
                }
                let response = app
                    .clone()
                    .oneshot(request.body(axum::body::Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::FORBIDDEN);
            }
        }
        io.close().await;
    }
}
