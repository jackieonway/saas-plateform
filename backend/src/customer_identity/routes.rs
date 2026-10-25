use axum::extract::State;
use axum::http::Uri;
use axum::routing::get;
use axum::{Json, Router};

use crate::error::AppError;
use crate::state::AppState;

use super::dto::{self, Presentation};
use super::proof::CustomerIdentity;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/customer/context", get(context))
        .route("/api/customer/profile", get(profile))
        .route("/api/customer/applications", get(applications))
}

async fn presentation(
    state: &AppState,
    identity: &CustomerIdentity,
    uri: &Uri,
) -> Result<Presentation, AppError> {
    if uri.query().is_some_and(|query| !query.is_empty()) {
        return Err(AppError::bad_request(
            "客户本人资料接口不接受主体或成员查询参数",
        ));
    }
    let proof = &identity.authority;
    let values: Presentation = sqlx::query_as(
        "SELECT t.name AS tenant_name,o.name AS organization_name, \
         a.code AS application_code,a.name AS application_name,u.username, \
         COALESCE(u.real_name,'') AS real_name,COALESCE(u.avatar,'') AS avatar \
         FROM users u JOIN tenants t ON t.id=$2 \
         JOIN organizations o ON o.id=$3 AND o.tenant_id=t.id \
         JOIN applications a ON a.id=$4 WHERE u.id=$1",
    )
    .bind(proof.user_id)
    .bind(proof.tenant_id)
    .bind(proof.organization_id)
    .bind(proof.application_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|_| AppError::Unavailable)?
    .ok_or_else(|| AppError::unauthorized("客户资料上下文已失效"))?;
    identity.revalidate(state).await?;
    Ok(values)
}

async fn context(
    State(state): State<AppState>,
    identity: CustomerIdentity,
    uri: Uri,
) -> Result<Json<dto::Context>, AppError> {
    let values = presentation(&state, &identity, &uri).await?;
    Ok(Json(dto::Context::new(&identity.authority, &values)))
}

async fn profile(
    State(state): State<AppState>,
    identity: CustomerIdentity,
    uri: Uri,
) -> Result<Json<dto::Profile>, AppError> {
    let values = presentation(&state, &identity, &uri).await?;
    Ok(Json(dto::Profile::new(&identity.authority, &values)))
}

async fn applications(
    State(state): State<AppState>,
    identity: CustomerIdentity,
    uri: Uri,
) -> Result<Json<dto::Applications>, AppError> {
    let values = presentation(&state, &identity, &uri).await?;
    Ok(Json(dto::Applications::new(&identity.authority, &values)))
}
