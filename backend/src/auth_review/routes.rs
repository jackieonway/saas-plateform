use super::{policy, service};
use crate::{
    error::AppError,
    extractors::{ApiJson, CurrentUser},
    state::AppState,
};
use axum::{
    extract::{DefaultBodyLimit, Path, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::Value;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/identity/review/security", get(security))
        .route(
            "/api/identity/review/partners",
            get(partners).post(partner_create),
        )
        .route(
            "/api/identity/review/applications",
            get(applications).post(application_create),
        )
        .route(
            "/api/identity/review/applications/{application}/requests",
            axum::routing::post(submit),
        )
        .route(
            "/api/identity/review/applications/{application}/operators",
            axum::routing::post(assign),
        )
        .route(
            "/api/identity/review/requests/{request}/decisions",
            axum::routing::post(decide),
        )
        .route(
            "/api/identity/review/requests/{request}/sync",
            axum::routing::post(sync),
        )
        .layer(DefaultBodyLimit::max(65536))
}
async fn security(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(service::security(&state, &user).await?))
}
async fn partners(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(service::partners(&state, &user).await?))
}
async fn applications(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    Ok(Json(service::applications(&state, &user).await?))
}
async fn partner_create(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    ApiJson(body): ApiJson<Value>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        service::partner_create(&state, &user, &headers, body).await?,
    ))
}
async fn application_create(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    ApiJson(body): ApiJson<Value>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        service::application_create(&state, &user, &headers, body).await?,
    ))
}
async fn submit(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(application): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        service::submit(&state, &user, &headers, policy::id(&application)?, body).await?,
    ))
}
async fn assign(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(application): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        service::assign(&state, &user, &headers, policy::id(&application)?, body).await?,
    ))
}
async fn decide(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(request): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> Result<Response, AppError> {
    let request = policy::id(&request)?;
    let (result, remote) = service::decide(&state, &user, &headers, request, body).await?;
    let operation = result["operationId"]
        .as_str()
        .ok_or(AppError::Unavailable)
        .and_then(policy::id)?;
    if !remote {
        if result["syncState"] == "PENDING" {
            return Ok(crate::oidc::ReviewSyncFailure::from(AppError::Unavailable)
                .with_operation(operation)
                .into_response());
        }
        return Ok(Json(result).into_response());
    }
    synchronize(&state, &user, &headers, request, None, Some(operation)).await
}
async fn sync(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(request): Path<String>,
    ApiJson(body): ApiJson<Value>,
) -> Result<Response, AppError> {
    synchronize(
        &state,
        &user,
        &headers,
        policy::id(&request)?,
        Some(body),
        None,
    )
    .await
}
async fn synchronize(
    state: &AppState,
    user: &CurrentUser,
    headers: &HeaderMap,
    request: uuid::Uuid,
    body: Option<Value>,
    mut committed_operation: Option<uuid::Uuid>,
) -> Result<Response, AppError> {
    let result: Result<Response, crate::oidc::ReviewSyncFailure> = async {
        let (application, environment, permission) =
            service::sync_context(state, user, request).await?;
        let current = crate::oidc::review_current(
            state,
            user,
            headers,
            application,
            &environment,
            permission,
        )
        .await?;
        let plan = service::prepare_sync(state, user, headers, request, body, &current).await?;
        if plan.payload.is_some() || plan.result["syncState"] == "PENDING" {
            committed_operation = Some(plan.operation);
        }
        let Some(payload) = plan.payload else {
            if plan.result["syncState"] == "PENDING" {
                return Err(AppError::Unavailable.into());
            }
            return Ok(Json(plan.result).into_response());
        };
        let remote =
            crate::oidc::review_sync(state, user, headers, application, &environment, payload)
                .await?;
        Ok(Json(service::synchronized_result(plan.result, &remote)?).into_response())
    }
    .await;
    Ok(match result {
        Ok(response) => response,
        Err(error) => match committed_operation {
            Some(operation) => error.with_operation(operation).into_response(),
            None => error.into_response(),
        },
    })
}
