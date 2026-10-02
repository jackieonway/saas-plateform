use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Json, Router};
use chrono::Utc;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::app_token;
use crate::authorization;
use crate::error::AppError;
use crate::extractors::bearer_token;
use crate::state::{AppState, ORG_ANCESTORS_SQL};
use crate::util::{hash, jwt};

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/apps/{appCode}/context", get(context))
}

async fn context(
    State(state): State<AppState>, Path(app_code): Path<String>, headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    Ok(Json(current_context(&state, &app_code, &headers).await?))
}

pub(crate) async fn current_context(
    state: &AppState, app_code: &str, headers: &HeaderMap,
) -> Result<Value, AppError> {
    let token = bearer_token(headers)?;
    let claims = jwt::verify(&token, &state.config.jwt_secret)
        .map_err(|_| AppError::unauthorized("应用令牌无效或已过期"))?;
    let binding = app_token::inspect(&state.pool, &token, &claims).await?
        .ok_or_else(|| AppError::unauthorized("应用令牌无效或未获授权"))?;
    if binding.audience != app_code {
        return Err(AppError::forbidden("应用受众不匹配"));
    }

    let identity: (String, String, String, String, String, String, String, bool) = sqlx::query_as(
        "SELECT u.username, COALESCE(u.real_name,''), t.code, t.name, o.name, \
         COALESCE(org_path(o.id),''), a.name, u.is_super_admin \
         FROM users u JOIN tenants t ON t.id = $2 \
         JOIN organizations o ON o.id = $3 AND o.tenant_id = t.id \
         JOIN applications a ON a.id = $4 \
         WHERE u.id = $1",
    ).bind(binding.user_id).bind(binding.tenant_id).bind(binding.organization_id)
     .bind(binding.application_id).fetch_one(&state.pool).await?;

    let menu_rows = if identity.7 {
        authorization::all_app_menu_rows(&state.pool, binding.application_id).await?
    } else {
        authorization::effective_menu_rows(
            &state.pool, binding.user_id, binding.tenant_id,
            binding.organization_id, Some(binding.application_id),
        ).await?
    };
    let mut permissions: Vec<String> = authorization::permissions_of(&menu_rows).into_iter().collect();
    permissions.sort();
    let role_sql = format!(
        "{ORG_ANCESTORS_SQL} SELECT DISTINCT r.id, r.name, COALESCE(r.code,'') \
         FROM user_roles ur JOIN roles r ON r.id = ur.role_id AND r.status = 'ENABLED' \
         JOIN anc ON anc.id = r.organization_id \
         JOIN role_menus rm ON rm.role_id = r.id \
         JOIN menus m ON m.id = rm.menu_id AND m.application_id = $3 AND m.status = 'ENABLED' \
         JOIN tenant_menus tm ON tm.menu_id = m.id AND tm.tenant_id = $4 \
         JOIN organization_menus om ON om.menu_id = m.id AND om.organization_id = $1 \
         WHERE ur.user_id = $2 ORDER BY r.id"
    );
    let roles: Vec<(Uuid, String, String)> = sqlx::query_as(&role_sql)
        .bind(binding.organization_id).bind(binding.user_id).bind(binding.application_id)
        .bind(binding.tenant_id)
        .fetch_all(&state.pool).await?;

    let grants: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT g.resource_code, g.action_code, g.scope_type \
         FROM membership_data_grants g \
         JOIN memberships member ON member.id = g.membership_id \
         WHERE member.user_id = $1 AND member.organization_id = $2 \
           AND g.application_id = $3 \
         ORDER BY g.resource_code, g.action_code",
    ).bind(binding.user_id).bind(binding.organization_id).bind(binding.application_id)
     .fetch_all(&state.pool).await?;
    let subtree: Vec<Uuid> = sqlx::query_scalar(
        "WITH RECURSIVE descendants AS ( \
         SELECT id, tenant_id FROM organizations WHERE id = $1 AND tenant_id = $2 \
           AND status = 'ENABLED' AND (expires_at IS NULL OR expires_at > now()) \
         UNION ALL SELECT child.id, child.tenant_id FROM organizations child \
         JOIN descendants parent ON child.parent_id = parent.id \
         WHERE child.tenant_id = $2 AND child.status = 'ENABLED' \
           AND (child.expires_at IS NULL OR child.expires_at > now())) \
         SELECT id FROM descendants ORDER BY id",
    ).bind(binding.organization_id).bind(binding.tenant_id)
     .fetch_all(&state.pool).await?;
    let data_grants: Vec<Value> = grants.into_iter().map(|(resource, action, scope)| {
        let org_ids: Vec<Uuid> = match scope.as_str() {
            "ORG" => vec![binding.organization_id],
            "ORG_SUBTREE" => subtree.clone(),
            _ => Vec::new(),
        };
        json!({
            "resource": resource, "action": action, "scope": scope,
            "subjectUserId": binding.user_id, "tenantId": binding.tenant_id,
            "organizationIds": org_ids,
        })
    }).collect();

    // 哈希输入使用平铺、按 id 排序的菜单，避免树节点构建顺序影响版本。
    let mut version_menus = menu_rows.clone();
    version_menus.sort_by_key(|menu| menu["id"].as_str().unwrap_or_default().to_owned());
    let version_input = json!({
        "user": binding.user_id, "tenant": binding.tenant_id,
        "organization": binding.organization_id, "application": binding.application_id,
        "identity": identity, "roles": roles, "permissions": permissions,
        "menus": version_menus, "dataGrants": data_grants,
    });
    let version = hash::sha256_hex(&version_input.to_string());
    // Live execution/deployment metadata is separate from the authorization hash.
    // A new token cannot use a stale profile's TRUSTED mode after a downgrade.
    let runtime: (String, String, String, Option<String>, Option<Uuid>) = sqlx::query_as(
        "SELECT a.runtime_mode,COALESCE(r.entry_type,a.entry_type),COALESCE(r.entry_url,a.entry_url),r.version,r.id \
         FROM applications a LEFT JOIN application_deployments d ON d.application_id=a.id AND d.environment=$2 \
         LEFT JOIN application_releases r ON r.application_id=a.id AND r.environment=d.environment AND r.id=d.active_release_id \
         WHERE a.id=$1",
    ).bind(binding.application_id).bind(&state.config.application_environment)
     .fetch_one(&state.pool).await?;
    // The mode update may have happened while assembling the authorization context.
    if app_token::inspect(&state.pool, &token, &claims).await?.is_none() {
        return Err(AppError::unauthorized("应用令牌已在上下文计算期间撤销"));
    }
    let expires_at = std::cmp::min(binding.expires_at, Utc::now() + chrono::Duration::seconds(30));
    Ok(json!({
        "contractVersion": "1.0", "contextVersion": version,
        "expiresAt": expires_at, "appCode": binding.audience,
        "application": {"id": binding.application_id, "name": identity.6,
            "runtimeMode": runtime.0, "entryType": runtime.1, "entryUrl": runtime.2,
            "deploymentVersion": runtime.3, "entryVersionId": runtime.4},
        "user": {"id": binding.user_id, "username": identity.0, "displayName": identity.1},
        "tenant": {"id": binding.tenant_id, "code": identity.2, "name": identity.3},
        "organization": {"id": binding.organization_id, "name": identity.4, "path": identity.5},
        "roles": roles.into_iter().map(|(id, name, code)| json!({"id": id, "name": name, "code": code})).collect::<Vec<_>>(),
        "permissions": permissions,
        "dataGrants": data_grants,
        "menus": authorization::menu_tree(menu_rows),
    }))
}
