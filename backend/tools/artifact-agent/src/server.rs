use crate::{Error, config::Config, storage};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use deployment_contract::zip;
use std::{sync::Arc, time::Duration};

#[derive(Clone)]
struct Context {
    config: Arc<Config>,
    requests: Arc<tokio::sync::Semaphore>,
    disk: Arc<tokio::sync::Semaphore>,
}
fn response(e: Error) -> Response {
    let status = match e {
        Error::Authentication => StatusCode::UNAUTHORIZED,
        Error::Busy => StatusCode::LOCKED,
        Error::Missing => StatusCode::NOT_FOUND,
        Error::Conflict | Error::Corrupt | Error::Contract(zip::Error::Digest) => {
            StatusCode::CONFLICT
        }
        Error::Io(_) | Error::Unsupported => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::BAD_REQUEST,
    };
    (status, Json(serde_json::json!({"code":e.code()}))).into_response()
}
async fn guard(State(c): State<Context>, request: Request, next: Next) -> Response {
    let actual = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));
    let expected = match c.config.token() {
        Ok(t) => t,
        Err(e) => return response(e),
    };
    let Some(actual) = actual else {
        return response(Error::Authentication);
    };
    let a = deployment_contract::sha256(actual.as_bytes());
    let b = deployment_contract::sha256(expected.as_bytes());
    if a.bytes().zip(b.bytes()).fold(0u8, |v, (a, b)| v | (a ^ b)) != 0 {
        return response(Error::Authentication);
    }
    let _permit = match c.requests.try_acquire() {
        Ok(p) => p,
        Err(_) => return response(Error::Busy),
    };
    match tokio::time::timeout(Duration::from_secs(45), next.run(request)).await {
        Ok(r) => r,
        Err(_) => (
            StatusCode::REQUEST_TIMEOUT,
            Json(serde_json::json!({"code":"REQUEST_DEADLINE"})),
        )
            .into_response(),
    }
}
async fn health(State(c): State<Context>) -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"UP","nodeId":c.config.node_id}))
}
async fn install(State(c): State<Context>, bytes: Bytes) -> Response {
    let permit = match c.disk.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return response(Error::Busy),
    };
    match tokio::task::spawn_blocking(move || {
        // A timed-out HTTP request cannot release a still-running disk task's slot.
        let _permit = permit;
        let request: zip::Install = serde_json::from_slice(&bytes)?;
        storage::install(&c.config, request)
    })
    .await
    {
        Ok(Ok(r)) => Json(r).into_response(),
        Ok(Err(e)) => response(e),
        Err(_) => response(Error::Corrupt),
    }
}
async fn verify(State(c): State<Context>, bytes: Bytes) -> Response {
    let permit = match c.disk.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return response(Error::Busy),
    };
    match tokio::task::spawn_blocking(move || {
        // A timed-out HTTP request cannot release a still-running disk task's slot.
        let _permit = permit;
        let request: zip::Verify = serde_json::from_slice(&bytes)?;
        storage::verify(&c.config, request)
    })
    .await
    {
        Ok(Ok(r)) => Json(r).into_response(),
        Ok(Err(e)) => response(e),
        Err(_) => response(Error::Corrupt),
    }
}
pub(super) async fn serve(config: Config) -> Result<(), Error> {
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    let c = Context {
        config: Arc::new(config),
        requests: Arc::new(tokio::sync::Semaphore::new(2)),
        disk: Arc::new(tokio::sync::Semaphore::new(2)),
    };
    let app = Router::new()
        .route("/agent/health", get(health))
        .route("/agent/install", post(install))
        .route("/agent/verify", post(verify))
        .layer(DefaultBodyLimit::max(zip::MAX_WIRE_BYTES))
        .layer(middleware::from_fn_with_state(c.clone(), guard))
        .with_state(c);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}
async fn shutdown() {
    #[cfg(unix)]
    {
        let ctrl = tokio::signal::ctrl_c();
        let term = async {
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(mut s) => {
                    s.recv().await;
                }
                Err(_) => std::future::pending::<()>().await,
            }
        };
        tokio::select! {_=ctrl=>{},_=term=>{}}
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
