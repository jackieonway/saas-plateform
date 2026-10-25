//! Request-local authorization context use case; SQL is confined to store.
mod store;
use chrono::Utc;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::app_token;
use crate::authorization;
use crate::error::AppError;
use crate::extractors::bearer_token;
use crate::public_contract::{
    AppAuthorizationContext, ContextContractVersion, MicroApplication, MicroDataGrant,
    MicroDataScope, MicroMenuItem, MicroOrganization, MicroRole, MicroTenant, MicroUser,
    OptionalNullable,
};
use crate::state::AppState;
use crate::util::{hash, jwt};

use axum::http::HeaderMap;
use saas_platform_backend::snapshot::{self, Revision, Snapshot, SnapshotError};
use sha2::{Digest, Sha256};

pub(crate) struct RequestContext {
    pub(crate) context: Value,
    pub(crate) revision: Revision,
    pub(crate) binding: app_token::AppTokenInfo,
}

fn revision(
    data: &store::ContextRows,
    binding: &app_token::AppTokenInfo,
) -> Result<Revision, AppError> {
    let bytes = serde_json::to_vec(&(data, binding)).map_err(anyhow::Error::from)?;
    Ok(Revision(Sha256::digest(bytes).into()))
}

async fn read(
    state: &AppState,
    app_code: &str,
    token: &str,
    caller: Option<Uuid>,
) -> Result<Snapshot<RequestContext>, AppError> {
    let claims = jwt::verify(token, &state.config.jwt_secret)
        .map_err(|_| AppError::unauthorized("应用令牌无效或已过期"))?;
    let binding = app_token::inspect_authorized(state, token, &claims)
        .await?
        .ok_or_else(|| AppError::unauthorized("应用令牌无效或未获授权"))?;
    if let Some(caller) = caller {
        if !saas_platform_backend::authorization_policy::application_binding(
            caller,
            binding.application_id,
            app_code,
            &binding.audience,
        ) {
            return Err(AppError::unauthorized("应用令牌与机密客户端不匹配"));
        }
    } else if binding.audience != app_code {
        return Err(AppError::forbidden("应用受众不匹配"));
    }
    if binding.member_type != "EMPLOYEE" {
        return Err(AppError::forbidden("当前应用授权入口仅允许员工身份"));
    }
    let data = store::load(&state.pool, &binding, &state.config.application_environment).await?;
    let version = revision(&data, &binding)?;
    let context = render(binding.clone(), data)?;
    Ok(Snapshot {
        value: RequestContext {
            context,
            revision: version,
            binding,
        },
        revision: version,
    })
}

// This is a new, independent statement snapshot, not a check inside the old RR
// transaction. No long positive cache and no serialization point for auth writes.
async fn fresh(
    state: &AppState,
    app_code: &str,
    token: &str,
    caller: Option<Uuid>,
) -> Result<Snapshot<RequestContext>, AppError> {
    let result = read(state, app_code, token, caller).await?;
    let claims = jwt::verify(token, &state.config.jwt_secret)
        .map_err(|_| AppError::unauthorized("应用令牌无效或已过期"))?;
    let binding = app_token::inspect_authorized(state, token, &claims)
        .await?
        .ok_or_else(|| AppError::unauthorized("应用令牌已在上下文计算期间撤销"))?;
    if binding != result.value.binding {
        return Err(AppError::unauthorized("应用令牌身份已改变"));
    }
    Ok(result)
}

pub(crate) async fn request_context(
    state: &AppState,
    app_code: &str,
    headers: &HeaderMap,
) -> Result<RequestContext, AppError> {
    let token = bearer_token(headers)?;
    request_context_bound(state, app_code, &token, None).await
}

pub(crate) async fn request_context_bound(
    state: &AppState,
    app_code: &str,
    token: &str,
    caller: Option<Uuid>,
) -> Result<RequestContext, AppError> {
    let timer = crate::observability::stages().timer(match app_code {
        "work-orders" => operational_telemetry::Stage::ContextWorkOrders,
        "stores" => operational_telemetry::Stage::ContextStores,
        _ => operational_telemetry::Stage::ContextOther,
    });
    let result = request_context_bound_inner(state, app_code, token, caller).await;
    timer.finish(crate::observability::outcome(&result));
    result
}

async fn request_context_bound_inner(
    state: &AppState,
    app_code: &str,
    token: &str,
    caller: Option<Uuid>,
) -> Result<RequestContext, AppError> {
    if token.is_empty() || token.len() > 8185 || token.chars().any(char::is_whitespace) {
        return Err(AppError::unauthorized("应用令牌无效"));
    }
    let _permit = state
        .app_authorization_slots
        .try_acquire()
        .map_err(|_| AppError::Unavailable)?;
    tokio::time::timeout(
        std::time::Duration::from_millis(state.config.app_authorization_timeout_ms),
        snapshot::consistent(
            || read(state, app_code, token, caller),
            || async { Ok(fresh(state, app_code, token, caller).await?.revision) },
        ),
    )
    .await
    .map_err(|_| AppError::Unavailable)?
    .map_err(|error| match error {
        SnapshotError::Store(error) => error,
        SnapshotError::Changed => AppError::Unavailable,
    })
}

pub(crate) async fn current_context(
    state: &AppState,
    app_code: &str,
    headers: &HeaderMap,
) -> Result<Value, AppError> {
    Ok(request_context(state, app_code, headers).await?.context)
}

/// Reuse the request's projection, but never skip a fresh token/revocation check.
/// A changed projection is returned for the member-use-case's complete policy
/// reevaluation; an equal hash is only an internal change detector.
pub(crate) async fn revalidate_context(
    state: &AppState,
    app_code: &str,
    headers: &HeaderMap,
    expected: Revision,
) -> Result<Option<Value>, AppError> {
    let token = bearer_token(headers)?;
    let _permit = state
        .app_authorization_slots
        .try_acquire()
        .map_err(|_| AppError::Unavailable)?;
    let latest = tokio::time::timeout(
        std::time::Duration::from_millis(state.config.app_authorization_timeout_ms),
        fresh(state, app_code, &token, None),
    )
    .await
    .map_err(|_| AppError::Unavailable)??;
    Ok((latest.revision != expected).then_some(latest.value.context))
}

fn render(binding: app_token::AppTokenInfo, data: store::ContextRows) -> Result<Value, AppError> {
    if binding.member_type != "EMPLOYEE" {
        return Err(AppError::forbidden("客户身份不能消费员工角色和数据授权"));
    }
    let store::ContextRows {
        identity,
        menu_rows,
        roles,
        grants,
        subtree,
        can_manage_platform,
        runtime,
    } = data;
    let mut permissions: Vec<String> = authorization::permissions_of(&menu_rows)
        .into_iter()
        .collect();
    permissions.sort();
    let data_grants: Vec<MicroDataGrant> = grants
        .into_iter()
        .map(|(resource, action, scope)| {
            // Tenant-wide grants cannot expand the selected child organization's
            // hard data boundary, even when inherited roles cover the whole tenant.
            let scope = if scope == "TENANT" && !can_manage_platform {
                "ORG_SUBTREE".to_owned()
            } else {
                scope
            };
            let org_ids: Vec<Uuid> = match scope.as_str() {
                "ORG" => vec![binding.organization_id],
                "ORG_SUBTREE" => subtree.clone(),
                "SELF" if !can_manage_platform => subtree.clone(),
                _ => Vec::new(),
            };
            let scope = match scope.as_str() {
                "SELF" => MicroDataScope::SelfScope,
                "ORG" => MicroDataScope::Org,
                "ORG_SUBTREE" => MicroDataScope::OrgSubtree,
                "TENANT" => MicroDataScope::Tenant,
                _ => return Err(anyhow::anyhow!("PUBLIC_CONTRACT_INVALID")),
            };
            Ok(MicroDataGrant {
                resource,
                action,
                scope,
                subject_user_id: binding.user_id.to_string(),
                tenant_id: binding.tenant_id.to_string(),
                organization_ids: org_ids.into_iter().map(|id| id.to_string()).collect(),
            })
        })
        .collect::<Result<_, anyhow::Error>>()?;

    // 哈希输入使用平铺、按 id 排序的菜单，避免树节点构建顺序影响版本。
    let mut version_menus = menu_rows.clone();
    version_menus.sort_by_key(|menu| menu["id"].as_str().unwrap_or_default().to_owned());
    let version_input = json!({
        "user": binding.user_id, "tenant": binding.tenant_id,
        "organization": binding.organization_id, "application": binding.application_id,
        "membership": binding.membership_id, "memberType":binding.member_type,
        "identityVersion":binding.identity_version.to_string(), "accountVersion":binding.account_version.to_string(),
        "identity": identity, "roles": roles, "permissions": permissions,
        "menus": version_menus, "dataGrants": data_grants,
        "organizationScopeIds": if can_manage_platform { None } else { Some(&subtree) },
    });
    let version = hash::sha256_hex(&version_input.to_string());
    let expires_at = std::cmp::min(
        binding.expires_at,
        Utc::now() + chrono::Duration::seconds(30),
    );
    let context = AppAuthorizationContext {
        contract_version: ContextContractVersion::V1Dot0,
        context_version: version,
        expires_at: expires_at.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true),
        app_code: binding.audience,
        application: MicroApplication {
            id: binding.application_id.to_string(),
            name: identity.6,
            runtime_mode: Some(
                serde_json::from_value(json!(runtime.0)).map_err(anyhow::Error::from)?,
            ),
            entry_type: Some(
                serde_json::from_value(json!(runtime.1)).map_err(anyhow::Error::from)?,
            ),
            entry_url: Some(runtime.2),
            deployment_version: runtime
                .3
                .map_or(OptionalNullable::Null, OptionalNullable::Value),
            entry_version_id: runtime.4.map_or(OptionalNullable::Null, |id| {
                OptionalNullable::Value(id.to_string())
            }),
        },
        user: MicroUser {
            id: binding.user_id.to_string(),
            username: identity.0,
            display_name: identity.1,
        },
        tenant: MicroTenant {
            id: binding.tenant_id.to_string(),
            code: identity.2,
            name: identity.3,
        },
        organization: MicroOrganization {
            id: binding.organization_id.to_string(),
            name: identity.4,
            path: identity.5,
        },
        roles: roles
            .into_iter()
            .map(|(id, name, code)| MicroRole {
                id: id.to_string(),
                name,
                code,
            })
            .collect(),
        permissions,
        data_grants,
        organization_scope_ids: if can_manage_platform {
            OptionalNullable::Null
        } else {
            OptionalNullable::Value(subtree.into_iter().map(|id| id.to_string()).collect())
        },
        menus: authorization::menu_tree(menu_rows)
            .into_iter()
            .map(serde_json::from_value)
            .collect::<Result<Vec<MicroMenuItem>, _>>()
            .map_err(anyhow::Error::from)?,
    };
    context.validate().map_err(anyhow::Error::from)?;
    Ok(serde_json::to_value(context).map_err(anyhow::Error::from)?)
}
