use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::error::AppError;
use crate::extractors::{ApiJson, CurrentUser};
use crate::state::AppState;
use crate::util::uuid7;

use super::service::{self, CreateMembership, ExpectedVersion, IdentityTransition, StatusChange};

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/memberships", get(list).post(create))
        .route("/api/memberships/capabilities", get(capabilities))
        .route("/api/memberships/{id}", get(detail).delete(delete))
        .route("/api/memberships/{id}/status", put(status))
        .route(
            "/api/memberships/{id}/second-identity",
            post(second_identity),
        )
        .route(
            "/api/memberships/{id}/replace-identity",
            post(replace_identity),
        )
}

/// Report the same server gates enforced by lifecycle mutations. Reading them
/// stays available to authorized employee administrators while writes are closed.
async fn capabilities(
    State(state): State<AppState>,
    actor: CurrentUser,
) -> Result<Json<Value>, AppError> {
    let proof = crate::authorization::require_employee_context(&state.pool, &actor).await?;
    state.require(&actor, "system:user").await?;
    if crate::authorization::require_employee_context(&state.pool, &actor).await? != proof {
        return Err(AppError::unauthorized("成员身份已在能力读取期间变更"));
    }
    Ok(Json(json!({
        "lifecycleEnabled": state.config.membership_lifecycle_enabled,
        "customerMembershipsEnabled": state.config.customer_memberships_enabled,
        "dualMembershipsEnabled": state.config.dual_memberships_enabled,
    })))
}

async fn list(
    State(state): State<AppState>,
    actor: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    service::list(&state, &actor, &params).await.map(Json)
}

async fn detail(
    State(state): State<AppState>,
    actor: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    service::detail(&state, &actor, uuid7::parse_uuid(&id)?)
        .await
        .map(Json)
}

async fn create(
    State(state): State<AppState>,
    actor: CurrentUser,
    headers: HeaderMap,
    ApiJson(input): ApiJson<CreateMembership>,
) -> Result<Json<Value>, AppError> {
    service::create(&state, &actor, &headers, input)
        .await
        .map(Json)
}

async fn status(
    State(state): State<AppState>,
    actor: CurrentUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    ApiJson(input): ApiJson<StatusChange>,
) -> Result<Json<Value>, AppError> {
    service::change_status(&state, &actor, &headers, uuid7::parse_uuid(&id)?, input)
        .await
        .map(Json)
}

async fn delete(
    State(state): State<AppState>,
    actor: CurrentUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    ApiJson(input): ApiJson<ExpectedVersion>,
) -> Result<Json<Value>, AppError> {
    service::delete(&state, &actor, &headers, uuid7::parse_uuid(&id)?, input)
        .await
        .map(Json)
}

async fn second_identity(
    State(state): State<AppState>,
    actor: CurrentUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    ApiJson(input): ApiJson<IdentityTransition>,
) -> Result<Json<Value>, AppError> {
    service::transition(
        &state,
        &actor,
        &headers,
        uuid7::parse_uuid(&id)?,
        input,
        false,
    )
    .await
    .map(Json)
}

async fn replace_identity(
    State(state): State<AppState>,
    actor: CurrentUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    ApiJson(input): ApiJson<IdentityTransition>,
) -> Result<Json<Value>, AppError> {
    service::transition(
        &state,
        &actor,
        &headers,
        uuid7::parse_uuid(&id)?,
        input,
        true,
    )
    .await
    .map(Json)
}
