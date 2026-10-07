//! Bounded operational telemetry. Request contents and authentication values
//! never enter logs or metric labels. Browser events are authenticated hints,
//! not an authoritative record of application availability.
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{DefaultBodyLimit, MatchedPath, Path, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::{bearer_token, ApiJson};
use crate::routes::app_context;
use crate::state::AppState;
use crate::util::{hash, jwt};

#[derive(Clone)]
pub struct RequestContext {
    pub id: Uuid,
    pub tenant_id: Option<Uuid>,
    pub organization_id: Option<Uuid>,
    pub app_code: Option<String>,
}

tokio::task_local! { static CONTEXT: RequestContext; }

pub fn request_id() -> Option<Uuid> { CONTEXT.try_with(|value| value.id).ok() }
pub fn app_code() -> Option<String> { CONTEXT.try_with(|value| value.app_code.clone()).ok().flatten() }

const BUCKETS: [f64; 9] = [0.005, 0.01, 0.025, 0.05, 0.1, 0.3, 0.5, 1.0, 5.0];
#[derive(Default)]
struct Observation { count: u64, seconds: f64, buckets: [u64; 9] }
#[derive(Default)]
struct Registry {
    requests: BTreeMap<(String, String, u16), Observation>,
    token_failures: BTreeMap<&'static str, u64>,
    runtime_events: BTreeMap<(String, String, String), u64>,
    event_rates: BTreeMap<(Uuid, Uuid, String), (Instant, u32)>,
}
static METRICS: OnceLock<Mutex<Registry>> = OnceLock::new();
fn registry() -> &'static Mutex<Registry> { METRICS.get_or_init(|| Mutex::new(Registry::default())) }

pub fn record_auth_failure(reason: &'static str) {
    let label = match reason {
        "invalid_client" | "inactive_token" | "expired_token" | "invalid_token" => reason,
        _ => "other",
    };
    if let Ok(mut metrics) = registry().lock() { *metrics.token_failures.entry(label).or_default() += 1; }
}

fn valid_app(value: &str) -> bool {
    !value.is_empty() && value.len() <= 100 && value.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

fn identity(headers: &HeaderMap, state: &AppState) -> RequestContext {
    let id = headers.get("x-request-id").and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok()).unwrap_or_else(Uuid::new_v4);
    // Only a valid signature and current expiry can attribute an attempted
    // request. Database authorization remains the responsibility of extractors.
    let claims = bearer_token(headers).ok().filter(|token| token.len() <= 8192)
        .and_then(|token| jwt::verify(&token, &state.config.jwt_secret).ok());
    RequestContext { id,
        tenant_id: claims.as_ref().and_then(|value| value.tenant.as_deref()).and_then(|value| value.parse().ok()),
        organization_id: claims.as_ref().and_then(|value| value.org.as_deref()).and_then(|value| value.parse().ok()),
        app_code: claims.and_then(|value| value.aud).filter(|value| valid_app(value)),
    }
}

pub async fn observe(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    let context = identity(request.headers(), &state);
    request.headers_mut().insert("x-request-id", context.id.to_string().parse().expect("UUID header"));
    let route = request.extensions().get::<MatchedPath>().map(|value| value.as_str()).unwrap_or("unmatched").to_string();
    let method = match request.method().as_str() {
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS" => request.method().as_str(),
        _ => "OTHER",
    }.to_string();
    request.extensions_mut().insert(context.clone());
    let start = Instant::now();
    let mut response = CONTEXT.scope(context.clone(), next.run(request)).await;
    let seconds = start.elapsed().as_secs_f64();
    let status = response.status().as_u16();
    response.headers_mut().insert("x-request-id", context.id.to_string().parse().expect("UUID header"));
    if let Ok(mut metrics) = registry().lock() {
        let entry = metrics.requests.entry((route.clone(), method.clone(), status)).or_default();
        entry.count += 1; entry.seconds += seconds;
        for (index, bound) in BUCKETS.iter().enumerate() { if seconds <= *bound { entry.buckets[index] += 1; } }
    }
    tracing::info!(event = "http_request", request_id = %context.id, route = %route, method = %method,
        status, duration_ms = seconds * 1000.0, tenant_id = context.tenant_id.map(|value| value.to_string()).as_deref().unwrap_or(""),
        organization_id = context.organization_id.map(|value| value.to_string()).as_deref().unwrap_or(""),
        application = context.app_code.as_deref().unwrap_or(""));
    response
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/internal/metrics", get(metrics))
        .route("/api/health/live", get(live))
        .merge(Router::new().route("/api/apps/{appCode}/runtime-events", post(runtime_event))
            .layer(DefaultBodyLimit::max(1024)))
}

async fn live() -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Json(json!({"status":"UP","service":"platform"}))).into_response()
}

async fn metrics(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    let token = bearer_token(&headers)?;
    if state.config.metrics_token.is_empty() || token.len() > 1024 ||
        hash::sha256_hex(&token) != hash::sha256_hex(&state.config.metrics_token) {
        return Err(AppError::unauthorized("指标访问需要运维凭据"));
    }
    let database_up = tokio::time::timeout(Duration::from_secs(1),
        sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&state.pool)).await.is_ok_and(|value| value.is_ok());
    let mut output = format!("# TYPE dante_service_up gauge\ndante_service_up 1\n# TYPE dante_database_up gauge\ndante_database_up {}\n# TYPE dante_uptime_seconds gauge\ndante_uptime_seconds {}\n",
        u8::from(database_up), (chrono::Utc::now() - state.started_at).num_seconds().max(0));
    output.push_str("# TYPE dante_http_requests_total counter\n# TYPE dante_http_request_duration_seconds histogram\n# TYPE dante_http_authentication_failures_total counter\n# TYPE dante_http_permission_denials_total counter\n");
    let data = registry().lock().map_err(|_| AppError::Server(anyhow::anyhow!("metrics unavailable")))?;
    for ((route, method, status), value) in &data.requests {
        // MatchedPath is a registered static template; no URI segments, query,
        // tenant UUIDs or request IDs are used as metric labels.
        let labels = format!("route=\"{}\",method=\"{}\",status=\"{}\"", route.replace('"', "\\\""), method, status);
        output.push_str(&format!("dante_http_requests_total{{{labels}}} {}\n", value.count));
        for (index, bound) in BUCKETS.iter().enumerate() {
            output.push_str(&format!("dante_http_request_duration_seconds_bucket{{{labels},le=\"{bound}\"}} {}\n", value.buckets[index]));
        }
        output.push_str(&format!("dante_http_request_duration_seconds_bucket{{{labels},le=\"+Inf\"}} {}\ndante_http_request_duration_seconds_sum{{{labels}}} {}\ndante_http_request_duration_seconds_count{{{labels}}} {}\n", value.count, value.seconds, value.count));
        if *status == 401 { output.push_str(&format!("dante_http_authentication_failures_total{{{labels}}} {}\n", value.count)); }
        if *status == 403 { output.push_str(&format!("dante_http_permission_denials_total{{{labels}}} {}\n", value.count)); }
    }
    output.push_str("# TYPE dante_token_validation_failures_total counter\n");
    for (reason, count) in &data.token_failures { output.push_str(&format!("dante_token_validation_failures_total{{reason=\"{reason}\"}} {count}\n")); }
    output.push_str("# TYPE dante_micro_runtime_events_total counter\n");
    for ((app, kind, stage), count) in &data.runtime_events { output.push_str(&format!("dante_micro_runtime_events_total{{application=\"{app}\",kind=\"{kind}\",stage=\"{stage}\"}} {count}\n")); }
    Ok(([(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8"), (header::CACHE_CONTROL, "no-store")], output).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeEvent { kind: String, stage: String }

async fn runtime_event(State(state): State<AppState>, Path(app): Path<String>, headers: HeaderMap,
    ApiJson(body): ApiJson<RuntimeEvent>) -> Result<Response, AppError> {
    if !matches!(body.kind.as_str(), "LOAD_FAILED" | "LIFECYCLE_FAILED") ||
        !matches!(body.stage.as_str(), "mount" | "update" | "unmount") || !valid_app(&app) {
        return Err(AppError::bad_request("微应用运行事件类型无效"));
    }
    let context = app_context::current_context(&state, &app, &headers).await?;
    let user: Uuid = context["user"]["id"].as_str().and_then(|value| value.parse().ok())
        .ok_or_else(|| AppError::unauthorized("应用上下文无效"))?;
    let tenant: Uuid = context["tenant"]["id"].as_str().and_then(|value| value.parse().ok())
        .ok_or_else(|| AppError::unauthorized("应用上下文无效"))?;
    let mut metrics = registry().lock().map_err(|_| AppError::Server(anyhow::anyhow!("metrics unavailable")))?;
    metrics.event_rates.retain(|_, (start, _)| start.elapsed() < Duration::from_secs(60));
    if metrics.event_rates.len() >= 4096 && !metrics.event_rates.contains_key(&(user, tenant, app.clone())) {
        return Err(AppError::RateLimited(60));
    }
    let entry = metrics.event_rates.entry((user, tenant, app.clone())).or_insert_with(|| (Instant::now(), 0));
    if entry.1 >= 30 { return Err(AppError::RateLimited(60)); }
    entry.1 += 1;
    let event_key = (app.clone(), body.kind.clone(), body.stage.clone());
    if metrics.runtime_events.len() >= 4096 && !metrics.runtime_events.contains_key(&event_key) {
        return Err(AppError::RateLimited(60));
    }
    *metrics.runtime_events.entry(event_key).or_default() += 1;
    tracing::warn!(event = "micro_runtime_event", request_id = request_id().map(|value| value.to_string()).as_deref().unwrap_or(""), application = %app,
        tenant_id = %tenant, organization_id = context["organization"]["id"].as_str().unwrap_or(""), kind = %body.kind, stage = %body.stage);
    Ok(StatusCode::NO_CONTENT.into_response())
}
