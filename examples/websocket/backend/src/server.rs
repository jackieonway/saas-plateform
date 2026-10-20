use crate::{
    ServerConfig,
    room::{ChatError, ChatRoom, JoinRequest, MessageRequest},
};
use axum::{
    Router,
    extract::{Request, State},
    http::{Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use chrono::{SecondsFormat, Utc};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use socketioxide::{
    SocketIo,
    extract::{SocketRef, TryData},
};
use std::{future::Future, io, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::Mutex};

/// Serve the reference chat on an already-bound loopback listener.
///
/// The shutdown future closes all Engine.IO sessions before HTTP shutdown.
/// IO failures are returned to the entrypoint; individual emit failures are
/// logged without exposing usernames or message contents.
pub async fn serve(
    listener: TcpListener,
    config: ServerConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    if !listener.local_addr()?.ip().is_loopback() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "chat listener must be loopback",
        ));
    }
    let room = Arc::new(Mutex::new(ChatRoom::default()));
    let (layer, io) = SocketIo::builder()
        .req_path("/")
        .ping_interval(Duration::from_secs(25))
        .ping_timeout(Duration::from_secs(60))
        .max_buffer_size(128)
        .max_payload(20_000)
        .ws_max_message_size(20_000)
        .ws_max_frame_size(20_000)
        .build_layer();
    io.ns("/", async move |socket: SocketRef, io: SocketIo| {
        register_handlers(socket, io, room.clone());
    });
    let app = Router::new()
        .fallback(|| async { StatusCode::NOT_FOUND })
        .layer(layer)
        // The guard wraps Engine.IO itself, including direct WebSocket upgrades.
        .layer(middleware::from_fn_with_state(config, origin_guard));
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown.await;
            io.close().await;
        })
        .await
}

fn register_handlers(socket: SocketRef, io: SocketIo, room: Arc<Mutex<ChatRoom>>) {
    let join_room = room.clone();
    let join_io = io.clone();
    socket.on(
        "join",
        async move |socket: SocketRef, TryData::<Value>(data)| {
            let data = match data
                .map_err(|_| ChatError::invalid_payload())
                .and_then(parse_object::<JoinRequest>)
            {
                Ok(data) => data,
                Err(error) => {
                    reject(&socket, error);
                    return;
                }
            };
            // One room lock keeps membership changes and broadcasts in the same order.
            // It is never held across network IO: Socket.IO emit only queues bounded packets.
            let mut room = join_room.lock().await;
            if !socket.connected() {
                return;
            }
            match room.join(&socket.id.to_string(), data.username, timestamp()) {
                Ok(joined) => {
                    if let Some(event) = joined {
                        log_emit(join_io.emit("user-joined", &event).await);
                    }
                    log_emit(socket.emit("users-list", &json!({"users": room.users()})));
                }
                Err(error) => reject(&socket, error),
            }
        },
    );
    let message_room = room.clone();
    let message_io = io.clone();
    socket.on(
        "message",
        async move |socket: SocketRef, TryData::<Value>(data)| {
            let data = match data
                .map_err(|_| ChatError::invalid_payload())
                .and_then(parse_object::<MessageRequest>)
            {
                Ok(data) => data,
                Err(error) => {
                    reject(&socket, error);
                    return;
                }
            };
            let mut room = message_room.lock().await;
            if !socket.connected() {
                return;
            }
            match room.message(&socket.id.to_string(), data, timestamp()) {
                Ok(message) => log_emit(message_io.emit("message", &message).await),
                Err(error) => reject(&socket, error),
            }
        },
    );
    socket.on("test", async |socket: SocketRef, TryData::<Value>(data)| {
        match data {
            Ok(data) => log_emit(socket.emit("test-response", &json!({ "message": "Server received test message", "data": data, "timestamp": timestamp() }))),
            Err(_) => reject(&socket, ChatError::invalid_payload()),
        }
    });
    socket.on_disconnect(async move |socket: SocketRef| {
        let mut room = room.lock().await;
        if let Some(event) = room.leave(&socket.id.to_string(), timestamp()) {
            log_emit(io.emit("user-left", &event).await);
        }
    });
}

fn parse_object<T: DeserializeOwned>(value: Value) -> Result<T, ChatError> {
    // Serde structs also accept positional arrays; the chat wire contract only
    // admits JSON objects, so enforce that shape before typed deserialization.
    if !value.is_object() {
        return Err(ChatError::invalid_payload());
    }
    serde_json::from_value(value).map_err(|_| ChatError::invalid_payload())
}

async fn origin_guard(
    State(config): State<ServerConfig>,
    request: Request,
    next: Next,
) -> Response {
    let mut origins = request.headers().get_all(header::ORIGIN).iter();
    let origin = origins.next().cloned();
    if origins.next().is_some()
        || origin
            .as_ref()
            .is_some_and(|origin| !config.origins.contains(origin))
    {
        return (StatusCode::FORBIDDEN, "Origin is not allowed").into_response();
    }
    let preflight = request.method() == Method::OPTIONS
        && request
            .headers()
            .contains_key(header::ACCESS_CONTROL_REQUEST_METHOD);
    let mut response = if preflight {
        StatusCode::NO_CONTENT.into_response()
    } else {
        next.run(request).await
    };
    if let Some(origin) = origin {
        response
            .headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        response
            .headers_mut()
            .append(header::VARY, axum::http::HeaderValue::from_static("Origin"));
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            axum::http::HeaderValue::from_static("GET, POST, OPTIONS"),
        );
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            axum::http::HeaderValue::from_static("Content-Type"),
        );
    }
    response
}

fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn reject(socket: &SocketRef, error: ChatError) {
    log_emit(socket.emit("error", &error));
}

fn log_emit<T: std::fmt::Display>(result: Result<(), T>) {
    if let Err(error) = result {
        eprintln!("chat emit failed: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_requires_a_json_object_with_the_declared_fields() {
        let request: JoinRequest = parse_object(json!({"username": "Alice"})).unwrap();
        assert_eq!(request.username, "Alice");
        for value in [
            Value::Null,
            json!(["Alice"]),
            json!([]),
            json!("Alice"),
            json!(1),
            json!(true),
            json!({}),
            json!({"username": 1}),
            json!({"username": "Alice", "extra": true}),
        ] {
            assert_eq!(
                parse_object::<JoinRequest>(value).err(),
                Some(ChatError::invalid_payload())
            );
        }
    }

    #[test]
    fn message_requires_a_json_object_with_the_declared_fields() {
        assert!(
            parse_object::<MessageRequest>(json!({"username": "Alice", "content": "hello"}))
                .is_ok()
        );
        for value in [
            Value::Null,
            json!(["Alice", "hello"]),
            json!([]),
            json!("hello"),
            json!(1),
            json!(true),
            json!({}),
            json!({"username": "Alice"}),
            json!({"username": "Alice", "content": 1}),
            json!({"username": "Alice", "content": "hello", "extra": true}),
        ] {
            assert_eq!(
                parse_object::<MessageRequest>(value).err(),
                Some(ChatError::invalid_payload())
            );
        }
    }
}
