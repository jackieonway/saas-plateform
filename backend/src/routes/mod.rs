mod admission_policy;
#[cfg(test)]
mod admission_policy_tests;
mod app_authorize;
pub mod app_context;
pub mod app_members;
mod app_organizations;
pub mod application_releases;
mod application_setup;
pub(crate) use application_setup::actor_permissions as platform_application_permissions;
pub mod applications;
pub mod audit;
pub mod auth;
mod contact;
mod data_grants;
pub mod departments;
pub mod device;
pub mod external;
pub mod maps;
pub mod menus;
pub mod monitor;
pub mod notifications;
pub mod oauth2;
pub mod oauth_admin;
mod onboarding;
pub mod organizations;
mod platform_settings;
pub mod regions;
mod retention;
pub mod roles;
pub mod search;
pub mod tenants;
pub mod users;

use std::collections::HashMap;

use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{http::StatusCode, Json, Router};
use serde_json::{json, Value};

use crate::state::AppState;

#[cfg(test)]
pub fn api_router() -> Router<AppState> {
    api_router_for_deployment(false)
}

pub fn api_router_for_deployment(identity_remote: bool) -> Router<AppState> {
    let identity_routes = if identity_remote {
        crate::oidc::remote_admin_routes()
    } else {
        crate::sso::routes().merge(crate::oidc::routes())
    };
    Router::new()
        .route("/api/health", get(health))
        .merge(app_authorize::routes())
        .merge(crate::observability::routes())
        .merge(auth::routes())
        .merge(identity_routes)
        .merge(crate::bff::routes())
        .merge(crate::customer_identity::routes())
        .merge(crate::customer_portal::routes())
        .merge(tenants::routes())
        .merge(applications::routes())
        .merge(crate::application_entries::routes())
        .merge(crate::micro_proxy::routes())
        .merge(application_releases::routes())
        .merge(application_setup::routes())
        .merge(crate::managed_deployments::routes())
        .merge(crate::artifact_distribution::routes())
        .merge(retention::routes())
        .merge(app_context::routes())
        .merge(app_members::routes())
        .merge(app_organizations::routes())
        .merge(data_grants::routes())
        .merge(menus::routes())
        .merge(maps::routes())
        .merge(organizations::routes())
        .merge(departments::routes())
        .merge(users::routes())
        .merge(crate::membership_lifecycle::routes())
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
    saas_platform_backend::paging::Paging::from_raw(
        params.get("page").map(String::as_str),
        params.get("size").map(String::as_str),
    )
    .parts()
}

/// 分页响应 {"records":[...],"total":n,"page":p,"size":s}
pub(crate) fn page_json(records: Vec<Value>, total: i64, page: i64, size: i64) -> Value {
    json!({"records": records, "total": total, "page": page, "size": size})
}

async fn health(axum::extract::State(state): axum::extract::State<AppState>) -> Response {
    let db_up = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&state.pool),
    )
    .await
    .is_ok_and(|value| value.is_ok());
    let identity_up = if state.config.identity_remote_enabled {
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            let client = state.config.identity_http_client.as_ref()?;
            let origin = state.config.identity_service_url.as_deref()?;
            let issuer = state.config.oidc_issuer.as_deref()?;
            let mut response = client
                .get(format!("{origin}/health/ready"))
                .send()
                .await
                .ok()?;
            if response.status().as_u16() != 200 {
                return None;
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.ok()? {
                if bytes.len() + chunk.len() > 8192 {
                    return None;
                }
                bytes.extend_from_slice(&chunk);
            }
            let body: Value = serde_json::from_slice(&bytes).ok()?;
            (body == json!({"status":"UP","service":"saas-identity","issuer":issuer})).then_some(())
        })
        .await
        .is_ok_and(|result| result.is_some())
    } else {
        true
    };
    let mut body = json!({"status":if db_up && identity_up {"UP"} else {"DOWN"},
        "version":"1.0.0","database":if db_up {"UP"} else {"DOWN"}});
    if state.config.identity_remote_enabled {
        body["identity"] = json!(if identity_up { "UP" } else { "DOWN" });
    }
    if db_up && identity_up {
        (StatusCode::OK, Json(body)).into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
    }
}
