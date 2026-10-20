//! HTTP boundary for the controlled data-grant use case.

use axum::{
    extract::{rejection::QueryRejection, Query, State},
    http::HeaderMap,
    routing::get,
    Json, Router,
};

use crate::{
    data_grants::{self, ChangeRequest, OptionsQuery, OptionsResponse, Preview, Snapshot, Target},
    error::AppError,
    extractors::{ApiJson, CurrentUser},
    state::AppState,
    util::audit,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/data-grants", get(read).patch(change))
        .route("/api/data-grants/options", get(options))
        .route("/api/data-grants/preview", axum::routing::post(preview))
}

async fn options(
    State(state): State<AppState>,
    actor: CurrentUser,
    query: Result<Query<OptionsQuery>, QueryRejection>,
) -> Result<Json<OptionsResponse>, AppError> {
    let Query(query) = query.map_err(|_| AppError::bad_request("数据授权选项查询参数无效"))?;
    data_grants::options(&state, &actor, query).await.map(Json)
}

async fn read(
    State(state): State<AppState>,
    actor: CurrentUser,
    target: Result<Query<Target>, QueryRejection>,
) -> Result<Json<Snapshot>, AppError> {
    let Query(target) =
        target.map_err(|_| AppError::bad_request("查询必须提供完整且有效的目标 UUID"))?;
    data_grants::read(&state, &actor, target).await.map(Json)
}

async fn preview(
    State(state): State<AppState>,
    actor: CurrentUser,
    ApiJson(request): ApiJson<ChangeRequest>,
) -> Result<Json<Preview>, AppError> {
    data_grants::preview(&state, &actor, request)
        .await
        .map(Json)
}

async fn change(
    State(state): State<AppState>,
    actor: CurrentUser,
    headers: HeaderMap,
    ApiJson(request): ApiJson<ChangeRequest>,
) -> Result<Json<Snapshot>, AppError> {
    data_grants::change(
        &state,
        &actor,
        request,
        &audit::client_ip(&headers),
        &audit::ua_of(&headers),
    )
    .await
    .map(Json)
}
