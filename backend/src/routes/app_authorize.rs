//! Private business-resource boundary with separate OAuth and app authorities.
use crate::{
    app_authorization, confidential_clients, error::AppError, resource_authority, state::AppState,
};
use axum::{
    extract::{DefaultBodyLimit, Form, Path, State},
    http::HeaderMap,
    routing::post,
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    token: String,
    resource: String,
    action: String,
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/apps/{appCode}/authorize", post(authorize))
        .layer(DefaultBodyLimit::max(32 * 1024))
}

fn operation(
    context: &Value,
    binding: &crate::app_token::AppTokenInfo,
    input: &Request,
) -> Result<(), AppError> {
    let permission = format!("{}:{}", input.resource, input.action);
    if !context["permissions"]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item.as_str() == Some(&permission)))
    {
        return Err(AppError::OperationDenied);
    }
    let user = binding.user_id.to_string();
    let tenant = binding.tenant_id.to_string();
    let count = context["dataGrants"].as_array().map_or(0, |items| {
        items
            .iter()
            .filter(|grant| {
                grant["resource"] == input.resource
                    && grant["action"] == input.action
                    && grant["subjectUserId"].as_str() == Some(&user)
                    && grant["tenantId"].as_str() == Some(&tenant)
            })
            .count()
    });
    if count != 1 {
        return Err(AppError::forbidden("缺少当前成员独立数据授权"));
    }
    Ok(())
}

async fn authorize(
    State(state): State<AppState>,
    Path(code): Path<String>,
    headers: HeaderMap,
    Form(input): Form<Request>,
) -> Result<Json<Value>, AppError> {
    tokio::time::timeout(
        std::time::Duration::from_millis(state.config.app_authorization_timeout_ms),
        execute(&state, &code, &headers, &input),
    )
    .await
    .map_err(|_| AppError::Unavailable)?
}

async fn execute(
    state: &AppState,
    code: &str,
    headers: &HeaderMap,
    input: &Request,
) -> Result<Json<Value>, AppError> {
    if input.token.is_empty()
        || input.token.len() > 8185
        || input.token.chars().any(char::is_whitespace)
    {
        return Err(AppError::unauthorized("资源凭据无效"));
    }
    let caller = confidential_clients::authenticate(state, headers).await?;
    let resource =
        resource_authority::resource(state, &caller, code, &input.resource, &input.action).await?;
    let capability = state
        .config
        .data_grant_policy
        .operation(caller.application_id, &input.resource, &input.action)
        .ok_or_else(|| AppError::bad_request("应用资源动作未登记"))?;
    let reply = if resource_authority::is_internal(state, &input.token).await? {
        let checked = app_authorization::request_context_bound(
            state,
            code,
            &input.token,
            Some(caller.application_id),
        )
        .await?;
        if !caller
            .allowed_member_types
            .contains(&checked.binding.member_type)
        {
            return Err(AppError::forbidden("服务未登记该成员类型"));
        }
        operation(&checked.context, &checked.binding, input)?;
        let info = checked.binding;
        json!({"contractVersion":"resource-authorization/1","issuer":crate::util::jwt::INTERNAL_ISSUER,"tokenUse":"app_session","subjectMode":"USER",
            "resource":resource.canonical_resource,"action":input.action,"targetApplicationId":resource.application_id,"audience":resource.audience,
            "expiresAt":checked.context["expiresAt"],"principal":{"userId":info.user_id,"membershipId":info.membership_id,"memberType":info.member_type,"identityVersion":info.identity_version.to_string(),"accountVersion":info.account_version.to_string(),"tenantId":info.tenant_id,"organizationId":info.organization_id},
            "introspection":{"active":true,"scope":info.scope,"client_id":info.client_id,"token_type":"Bearer","token_use":"app_session","iss":crate::util::jwt::INTERNAL_ISSUER,"aud":info.audience,"jti":info.jti,"exp":info.expires_at.timestamp()},"context":checked.context})
    } else {
        let proof = resource_authority::verify(state, &input.token, &resource).await?;
        if proof.subject_mode == "MACHINE" {
            let delegation =
                resource_authority::machine_delegation(state, &proof, &resource).await?;
            let scope = delegation["scope"]
                .as_str()
                .and_then(crate::data_grants::policy::GrantScope::from_str_value)
                .ok_or_else(|| AppError::forbidden("机器数据范围未登记"))?;
            if !capability.scopes.contains(&scope) {
                return Err(AppError::forbidden("机器数据范围超出资源策略"));
            }
            let fresh = resource_authority::verify(state, &input.token, &resource).await?;
            if fresh != proof
                || resource_authority::machine_delegation(state, &fresh, &resource).await?
                    != delegation
            {
                return Err(AppError::Unavailable);
            }
            json!({"contractVersion":"resource-authorization/1","issuer":proof.issuer,"tokenUse":"access_token","subjectMode":"MACHINE","clientId":proof.client_id,"sourceApplicationId":proof.application_id,"targetApplicationId":resource.application_id,"resource":proof.resource,"action":input.action,"audience":proof.audience,"scope":proof.scope,"credentialVersion":proof.credential_version,"expiresAt":delegation["expiresAt"],"delegation":delegation})
        } else {
            let context = proof
                .context
                .as_ref()
                .ok_or_else(|| AppError::unauthorized("缺少业务成员"))?;
            if !caller.allowed_member_types.contains(&context.member_type) {
                return Err(AppError::forbidden("服务未登记该成员类型"));
            }
            let checked = app_authorization::external_context(state, &proof, &resource).await?;
            operation(&checked.context, &checked.binding, input)?;
            let fresh = resource_authority::verify(state, &input.token, &resource).await?;
            let latest = app_authorization::external_context(state, &fresh, &resource).await?;
            if fresh != proof
                || latest.revision != checked.revision
                || latest.binding != checked.binding
            {
                return Err(AppError::Unavailable);
            }
            operation(&latest.context, &latest.binding, input)?;
            let info = latest.binding;
            json!({"contractVersion":"resource-authorization/1","issuer":proof.issuer,"tokenUse":"access_token","subjectMode":"USER","clientId":proof.client_id,"sourceApplicationId":proof.application_id,"targetApplicationId":resource.application_id,"resource":proof.resource,"action":input.action,"audience":proof.audience,"expiresAt":latest.context["expiresAt"],
                "principal":{"userId":info.user_id,"membershipId":info.membership_id,"memberType":info.member_type,"identityVersion":info.identity_version.to_string(),"accountVersion":info.account_version.to_string(),"tenantId":info.tenant_id,"organizationId":info.organization_id},
                "introspection":{"active":true,"scope":proof.scope.join(" "),"client_id":proof.client_id,"token_type":"Bearer","token_use":"access_token","sub":proof.sub,"iss":proof.issuer,"aud":proof.audience,"exp":proof.expires_at.timestamp()},"context":latest.context})
        }
    };
    if confidential_clients::recheck(state, headers).await? != caller
        || resource_authority::resource(state, &caller, code, &input.resource, &input.action)
            .await?
            != resource
    {
        return Err(AppError::unauthorized("资源服务或目标登记已改变"));
    }
    let expires = reply["expiresAt"]
        .as_str()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .ok_or(AppError::Unavailable)?;
    if expires <= Utc::now() {
        return Err(AppError::unauthorized("资源凭据已在验证期间过期"));
    }
    Ok(Json(reply))
}
