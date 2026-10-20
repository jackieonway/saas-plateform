//! Confidential business-client authorization, with no browser or hash shortcut.
use crate::{app_authorization, confidential_clients, error::AppError, state::AppState};
use axum::{
    extract::{DefaultBodyLimit, Form, Path, State},
    http::HeaderMap,
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    token: String,
    resource: String,
    action: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Reply {
    contract_version: &'static str,
    introspection: Value,
    context: Value,
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/apps/{appCode}/authorize", post(authorize))
        .layer(DefaultBodyLimit::max(32 * 1024))
}

async fn authorize(
    State(state): State<AppState>,
    Path(code): Path<String>,
    headers: HeaderMap,
    Form(input): Form<Request>,
) -> Result<Json<Reply>, AppError> {
    tokio::time::timeout(std::time::Duration::from_millis(state.config.app_authorization_timeout_ms),async {
        let caller=confidential_clients::authenticate(&state,&headers).await?;
        let checked=app_authorization::request_context_bound(&state,&code,&input.token,Some(caller.application_id)).await?;
        let grant_policy=&state.config.data_grant_policy;
        let _operation=grant_policy.operation(caller.application_id,&input.resource,&input.action)
            .ok_or_else(||AppError::bad_request("应用资源动作未登记"))?;
        let permission=format!("{}:{}",input.resource,input.action);
        if !checked.context["permissions"].as_array().is_some_and(|items|items.iter().any(|item|item.as_str()==Some(permission.as_str()))) {
            return Err(AppError::OperationDenied);
        }
        let user=checked.binding.user_id.to_string();let tenant=checked.binding.tenant_id.to_string();
        let count=checked.context["dataGrants"].as_array().map_or(0,|items|items.iter().filter(|grant|
            grant["resource"]==input.resource && grant["action"]==input.action && grant["subjectUserId"].as_str()==Some(user.as_str()) && grant["tenantId"].as_str()==Some(tenant.as_str())
        ).count());
        if count!=1 {return Err(AppError::forbidden("缺少当前成员的独立数据授权"));}
        // Secret rotation, disablement or application rebinding during snapshot
        // assembly is checked outside the original credential query.
        if confidential_clients::recheck(&state,&headers).await? != caller {return Err(AppError::unauthorized("机密客户端绑定已改变"));}
        let info=checked.binding;
        Ok(Json(Reply {contract_version:"1.0",introspection:json!({"active":true,"scope":info.scope,"client_id":info.client_id,"token_type":"Bearer",
            "sub":info.user_id,"tenant_id":info.tenant_id,"organization_id":info.organization_id,"aud":info.audience,"jti":info.jti,"exp":info.expires_at.timestamp()}),context:checked.context}))
    }).await.map_err(|_|AppError::Unavailable)?
}
