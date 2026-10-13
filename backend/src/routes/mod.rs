pub mod applications;
pub mod application_releases;
pub mod app_context;
pub mod app_members;
pub mod audit;
pub mod auth;
pub mod departments;
pub mod device;
pub mod external;
pub mod menus;
pub mod maps;
pub mod monitor;
pub mod notifications;
mod platform_settings;
mod onboarding;
pub mod oauth2;
pub mod oauth_admin;
pub mod organizations;
pub mod roles;
pub mod regions;
pub mod search;
pub mod tenants;
pub mod users;

use std::collections::HashMap;

use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{http::StatusCode, Json, Router};
use serde_json::{json, Value};

use crate::state::AppState;

pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/api/health", get(health))
        .merge(crate::observability::routes())
        .merge(auth::routes())
        .merge(tenants::routes())
        .merge(applications::routes())
        .merge(application_releases::routes())
        .merge(app_context::routes())
        .merge(app_members::routes())
        .merge(menus::routes())
        .merge(maps::routes())
        .merge(organizations::routes())
        .merge(departments::routes())
        .merge(users::routes())
        .merge(roles::routes())
        .merge(regions::routes())
        .merge(search::routes())
        .merge(oauth_admin::routes())
        .merge(external::routes())
        .merge(oauth2::consent_routes())
        .merge(device::routes())
        .merge(notifications::routes())
        .merge(platform_settings::routes())
        .merge(monitor::routes())
        .merge(audit::routes())
}

/// 分页参数 (page ≥ 1, 1 ≤ size ≤ 200) → (page, size, offset)
pub(crate) fn paging(params: &HashMap<String, String>) -> (i64, i64, i64) {
    let page: i64 = params
        .get("page")
        .and_then(|v| v.parse().ok())
        .filter(|v| *v >= 1)
        .unwrap_or(1);
    let size: i64 = params
        .get("size")
        .and_then(|v| v.parse().ok())
        .filter(|v| (1..=200).contains(v))
        .unwrap_or(10);
    (page, size, (page - 1) * size)
}

/// 分页响应 {"records":[...],"total":n,"page":p,"size":s}
pub(crate) fn page_json(records: Vec<Value>, total: i64, page: i64, size: i64) -> Value {
    json!({"records": records, "total": total, "page": page, "size": size})
}

async fn health(axum::extract::State(state): axum::extract::State<AppState>) -> Response {
    let db_up = tokio::time::timeout(std::time::Duration::from_secs(1),
        sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&state.pool)).await.is_ok_and(|value| value.is_ok());
    if db_up {
        (StatusCode::OK, Json(json!({"status": "UP", "version": "1.0.0", "database": "UP"}))).into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status": "DOWN", "version": "1.0.0", "database": "DOWN"})),
        )
            .into_response()
    }
}
