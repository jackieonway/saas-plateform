pub(crate) use crate::app_authorization::current_context;
use crate::{error::AppError, state::AppState};
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    routing::get,
    Json, Router,
};
use serde_json::Value;
pub fn routes() -> Router<AppState> {
    Router::new().route("/api/apps/{appCode}/context", get(context))
}
async fn context(
    State(state): State<AppState>,
    Path(app_code): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    Ok(Json(current_context(&state, &app_code, &headers).await?))
}
