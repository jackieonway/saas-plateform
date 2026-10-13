use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Json, Router};

use crate::error::AppError;
use crate::extractors::{ApiJson, CurrentUser, RequirePermission};
use crate::platform_settings::{self, PlatformBranding, PlatformSettings, SettingsUpdate};
use crate::state::AppState;
use crate::util::audit;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/platform/branding", get(branding))
        .route("/api/platform/settings", get(settings).put(update))
}

async fn branding(State(state): State<AppState>) -> Result<Json<PlatformBranding>, AppError> {
    Ok(Json(platform_settings::load_branding(&state.pool).await?))
}

async fn require_settings(state: &AppState, user: &CurrentUser) -> Result<(), AppError> {
    state.require_platform(user).await?;
    RequirePermission("system:settings")
        .check(state, user)
        .await
}

async fn settings(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<PlatformSettings>, AppError> {
    require_settings(&state, &user).await?;
    Ok(Json(platform_settings::load_settings(&state.pool).await?))
}

async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<SettingsUpdate>,
) -> Result<Json<PlatformSettings>, AppError> {
    require_settings(&state, &user).await?;
    let settings = platform_settings::update_settings(&state.pool, body).await?;
    let ip = audit::client_ip(&headers);
    let user_agent = audit::ua_of(&headers);
    audit::write(
        &state.pool,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(user.tenant_id),
            organization_id: Some(user.org_id),
            action: "UPDATE_PLATFORM_SETTINGS",
            detail: &format!(
                "修改平台设置: {}, 平台租户名称: {}",
                settings.branding.platform_name, settings.platform_tenant_name
            ),
            ip: &ip,
            user_agent: &user_agent,
        },
    )
    .await;
    Ok(Json(settings))
}
