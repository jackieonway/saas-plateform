//! Narrow application-token directory; platform management sessions are separate.
use axum::{
    extract::{rejection::QueryRejection, Path, Query, State},
    http::HeaderMap,
    routing::get,
    Json, Router,
};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    app_organizations::{self, LookupQuery},
    error::AppError,
    state::AppState,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/apps/{appCode}/organizations", get(list))
        .route(
            "/api/apps/{appCode}/organizations/{organizationId}",
            get(one),
        )
}

async fn list(
    State(state): State<AppState>,
    Path(code): Path<String>,
    headers: HeaderMap,
    query: Result<Query<LookupQuery>, QueryRejection>,
) -> Result<Json<Value>, AppError> {
    let Query(query) = query.map_err(|_| AppError::bad_request("组织目录查询参数无效"))?;
    app_organizations::lookup(&state, &code, &headers, query, None)
        .await
        .map(Json)
}

async fn one(
    State(state): State<AppState>,
    Path((code, id)): Path<(String, Uuid)>,
    headers: HeaderMap,
    query: Result<Query<LookupQuery>, QueryRejection>,
) -> Result<Json<Value>, AppError> {
    let Query(query) = query.map_err(|_| AppError::bad_request("组织目录查询参数无效"))?;
    app_organizations::lookup(&state, &code, &headers, query, Some(id))
        .await
        .map(Json)
}
