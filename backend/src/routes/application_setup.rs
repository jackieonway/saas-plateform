//! Read-only onboarding evidence and an additive, atomic access configuration.
use std::collections::HashSet;

use axum::{
    extract::{rejection::QueryRejection, Path, Query, State},
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{
    error::AppError,
    extractors::{ApiJson, CurrentUser, RequirePermission},
    session,
    state::AppState,
    util::audit,
};

const ACCESS_PERMISSIONS: [&str; 3] = ["system:tenant", "system:authorization", "system:role"];
const ALL_PERMISSIONS: [&str; 4] = [
    "system:application",
    "system:tenant",
    "system:authorization",
    "system:role",
];
const ENTRY_MENU: &str = "m.status='ENABLED' AND m.visible AND m.type='MENU' \
    AND btrim(COALESCE(m.path,'')) LIKE '/%' AND btrim(COALESCE(m.path,'')) NOT LIKE '//%' \
    AND btrim(COALESCE(m.path,'')) !~ '[:*]' AND COALESCE(m.link_url,'')='' \
    AND EXISTS(SELECT 1 FROM applications entry_app WHERE entry_app.id=m.application_id \
      AND (rtrim(entry_app.base_path,'/')<>('/micro/' || entry_app.code) \
        OR btrim(m.path)=('/micro/' || entry_app.code) \
        OR starts_with(btrim(m.path),'/micro/' || entry_app.code || '/')))";

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/applications/{id}/setup", get(read))
        .route(
            "/api/applications/{id}/setup/organizations",
            get(organization_options),
        )
        .route(
            "/api/applications/{id}/setup/access",
            post(configure_access),
        )
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetupQuery {
    #[serde(default = "development")]
    environment: String,
    #[serde(default, deserialize_with = "crate::util::uuid7::deserialize_optional")]
    tenant_id: Option<Uuid>,
    #[serde(default, deserialize_with = "crate::util::uuid7::deserialize_optional")]
    organization_id: Option<Uuid>,
    #[serde(default, deserialize_with = "crate::util::uuid7::deserialize_optional")]
    role_id: Option<Uuid>,
}

fn development() -> String {
    "development".into()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OrganizationQuery {
    #[serde(deserialize_with = "crate::util::uuid7::deserialize")]
    tenant_id: Uuid,
}

#[derive(sqlx::FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
struct OrganizationChoice {
    id: Uuid,
    tenant_id: Uuid,
    parent_id: Option<Uuid>,
    name: String,
    code: String,
    status: String,
    expires_at: Option<DateTime<Utc>>,
}

/// Access configuration needs organization identities, not the organization
/// administration permission or its contact/location payload.
async fn organization_options(
    State(state): State<AppState>,
    actor: CurrentUser,
    Path(id): Path<Uuid>,
    query: Result<Query<OrganizationQuery>, QueryRejection>,
) -> Result<Json<Value>, AppError> {
    let id = crate::util::uuid7::validate_uuid(id)?;
    state.require_platform(&actor).await?;
    RequirePermission("system:application")
        .check(&state, &actor)
        .await?;
    let Query(query) = query.map_err(|_| AppError::bad_request("请选择有效的租户 ID"))?;
    if !crate::util::uuid7::is_uuid_v7(query.tenant_id) {
        return Err(AppError::bad_request("请选择有效的租户 ID"));
    }
    let mut tx = state.pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    if !can_configure(&actor_permissions(&mut tx, &actor).await?) {
        return Err(AppError::forbidden("无访问范围配置权限"));
    }
    application(&mut tx, id, false).await?;
    let active: bool = sqlx::query_scalar(
        "SELECT status='ENABLED' AND (expires_at IS NULL OR expires_at>clock_timestamp()) \
         FROM tenants WHERE id=$1",
    )
    .bind(query.tenant_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::bad_request("目标租户不存在"))?;
    if !active {
        return Err(AppError::forbidden("目标租户已停用或到期"));
    }
    let choices: Vec<OrganizationChoice> = sqlx::query_as(
        "WITH RECURSIVE available AS ( \
           SELECT o.id,o.tenant_id,o.parent_id,o.name,o.code,o.status,o.expires_at,o.sort_order,o.created_at \
           FROM organizations o WHERE o.tenant_id=$1 AND o.parent_id IS NULL \
             AND o.status='ENABLED' AND (o.expires_at IS NULL OR o.expires_at>clock_timestamp()) \
           UNION SELECT o.id,o.tenant_id,o.parent_id,o.name,o.code,o.status,o.expires_at,o.sort_order,o.created_at \
           FROM organizations o JOIN available p ON o.parent_id=p.id \
           WHERE o.tenant_id=$1 AND o.status='ENABLED' \
             AND (o.expires_at IS NULL OR o.expires_at>clock_timestamp())) \
         SELECT id,tenant_id,parent_id,name,COALESCE(code,'') AS code,status,expires_at \
         FROM available ORDER BY sort_order,created_at,id",
    )
    .bind(query.tenant_id)
    .fetch_all(&mut *tx)
    .await?;
    let choices = choices
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(anyhow::Error::from)?;
    tx.commit().await?;
    Ok(Json(json!(crate::util::tree::build_tree(choices))))
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
struct Target {
    tenant_id: Uuid,
    organization_id: Uuid,
    role_id: Uuid,
}

impl SetupQuery {
    fn validate(&self) -> Result<Option<Target>, AppError> {
        if !["development", "staging", "production"].contains(&self.environment.as_str()) {
            return Err(AppError::bad_request("发布环境无效"));
        }
        match (self.tenant_id, self.organization_id, self.role_id) {
            (None, None, None) => Ok(None),
            (Some(tenant_id), Some(organization_id), Some(role_id))
                if ![tenant_id, organization_id, role_id]
                    .iter()
                    .any(Uuid::is_nil) =>
            {
                Ok(Some(Target {
                    tenant_id,
                    organization_id,
                    role_id,
                }))
            }
            _ => Err(AppError::bad_request("请完整提供有效的租户、组织与角色 ID")),
        }
    }
}

#[derive(sqlx::FromRow)]
struct Application {
    code: String,
    base_path: String,
    entry_type: String,
    runtime_mode: String,
    status: String,
}

async fn application(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    lock: bool,
) -> Result<Application, AppError> {
    let suffix = if lock { " FOR NO KEY UPDATE" } else { "" };
    sqlx::query_as(&format!(
        "SELECT code,COALESCE(base_path,'') AS base_path,entry_type,runtime_mode, \
         status FROM applications WHERE id=$1{suffix}"
    ))
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| AppError::not_found("应用不存在"))
}

/// Read fresh permissions through the same transaction after any lock waits.
/// A super administrator only bypasses role membership, never tenant/org caps.
pub(crate) async fn actor_permissions(
    tx: &mut Transaction<'_, Postgres>,
    actor: &CurrentUser,
) -> Result<HashSet<String>, AppError> {
    let active: Option<(bool, bool)> = sqlx::query_as(
        "SELECT u.is_super_admin,(t.is_platform AND o.parent_id IS NULL) \
         FROM platform_sessions s JOIN users u ON u.id=s.user_id AND u.status='ENABLED' \
         JOIN tenants t ON t.id=s.tenant_id AND t.status='ENABLED' \
         JOIN organizations o ON o.id=s.organization_id AND o.tenant_id=t.id \
         JOIN memberships ms ON ms.id=s.membership_id AND ms.user_id=u.id AND ms.organization_id=o.id \
           AND ms.member_type='EMPLOYEE' AND ms.status='ENABLED' \
           AND ms.identity_version=s.identity_version \
         WHERE s.id=$1 AND s.user_id=$2 AND s.tenant_id=$3 AND s.organization_id=$4 \
         AND s.member_type='EMPLOYEE' AND platform_session_active(s.id) \
         AND (t.expires_at IS NULL OR t.expires_at>clock_timestamp()) AND org_chain_active(o.id) \
         AND (org_chain_expires_at(o.id) IS NULL OR org_chain_expires_at(o.id)>clock_timestamp())",
    )
    .bind(actor.session_id)
    .bind(actor.user_id)
    .bind(actor.tenant_id)
    .bind(actor.org_id)
    .fetch_optional(&mut **tx)
    .await?;
    let (super_admin, platform) =
        active.ok_or_else(|| AppError::unauthorized("会话已失效，请重新登录"))?;
    if !platform {
        return Err(AppError::forbidden("仅平台根组织可管理应用接入"));
    }
    let permissions: Vec<String> = sqlx::query_scalar(
        "WITH RECURSIVE anc AS (SELECT id,parent_id FROM organizations WHERE id=$1 AND tenant_id=$2 \
         UNION SELECT o.id,o.parent_id FROM organizations o JOIN anc c ON o.id=c.parent_id WHERE o.tenant_id=$2) \
         SELECT DISTINCT m.permission FROM menus m JOIN applications a ON a.id=m.application_id AND a.status='ENABLED' \
         JOIN tenant_applications ta ON ta.application_id=a.id AND ta.tenant_id=$2 \
         JOIN organization_applications oa ON oa.application_id=a.id AND oa.organization_id=$1 \
         JOIN tenant_menus tm ON tm.menu_id=m.id AND tm.tenant_id=$2 \
         JOIN organization_menus om ON om.menu_id=m.id AND om.organization_id=$1 \
         WHERE m.status='ENABLED' AND m.permission=ANY($5) AND ($4 OR EXISTS( \
         SELECT 1 FROM role_menus rm JOIN roles r ON r.id=rm.role_id AND r.status='ENABLED' \
         JOIN anc ON anc.id=r.organization_id JOIN user_roles ur ON ur.role_id=r.id AND ur.user_id=$3 \
         WHERE rm.menu_id=m.id))",
    )
    .bind(actor.org_id).bind(actor.tenant_id).bind(actor.user_id).bind(super_admin)
    .bind(ALL_PERMISSIONS).fetch_all(&mut **tx).await?;
    let permissions: HashSet<_> = permissions.into_iter().collect();
    if !permissions.contains("system:application") {
        return Err(AppError::forbidden("无操作权限: system:application"));
    }
    Ok(permissions)
}

fn can_configure(permissions: &HashSet<String>) -> bool {
    ACCESS_PERMISSIONS
        .iter()
        .all(|permission| permissions.contains(*permission))
}

/// Onboarding/contact initialization already locks parents before existing users.
async fn validate_target_parents(
    tx: &mut Transaction<'_, Postgres>,
    target: Target,
    lock: bool,
) -> Result<(), AppError> {
    let suffix = if lock { " FOR NO KEY UPDATE" } else { "" };
    let tenant_active: bool = sqlx::query_scalar(&format!(
        "SELECT status='ENABLED' AND (expires_at IS NULL OR expires_at>clock_timestamp()) \
         FROM tenants WHERE id=$1{suffix}"
    ))
    .bind(target.tenant_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| AppError::bad_request("目标租户不存在"))?;
    let org_tenant: Uuid = sqlx::query_scalar(&format!(
        "SELECT tenant_id FROM organizations WHERE id=$1{suffix}"
    ))
    .bind(target.organization_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| AppError::bad_request("目标组织不存在"))?;
    if org_tenant != target.tenant_id {
        return Err(AppError::bad_request("租户与组织的所属关系不一致"));
    }
    let org_active: bool = sqlx::query_scalar("SELECT org_chain_active($1) AND (org_chain_expires_at($1) IS NULL OR org_chain_expires_at($1)>clock_timestamp())")
        .bind(target.organization_id)
        .fetch_one(&mut **tx)
        .await?;
    if !tenant_active || !org_active {
        return Err(AppError::forbidden("目标租户或组织已停用或到期"));
    }
    Ok(())
}

async fn validate_target_role(
    tx: &mut Transaction<'_, Postgres>,
    target: Target,
    lock: bool,
) -> Result<bool, AppError> {
    let suffix = if lock { " FOR NO KEY UPDATE" } else { "" };
    let (role_tenant, role_org, role_status, managed): (Uuid, Uuid, String, bool) =
        sqlx::query_as(&format!(
        "SELECT tenant_id,organization_id,status,built_in AND COALESCE(code,'') IN ('tenant_admin','org_admin') \
         FROM roles WHERE id=$1{suffix}"
    ))
        .bind(target.role_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| AppError::bad_request("目标角色不存在"))?;
    if role_tenant != target.tenant_id || role_org != target.organization_id {
        return Err(AppError::bad_request("租户、组织与角色的所属关系不一致"));
    }
    if role_status != "ENABLED" {
        return Err(AppError::forbidden("目标角色已停用"));
    }
    Ok(managed)
}

async fn validate_target(
    tx: &mut Transaction<'_, Postgres>,
    target: Target,
    lock: bool,
) -> Result<bool, AppError> {
    validate_target_parents(tx, target, lock).await?;
    validate_target_role(tx, target, lock).await
}

async fn access_evidence(
    tx: &mut Transaction<'_, Postgres>,
    app_id: Uuid,
    target: Target,
) -> Result<Value, AppError> {
    validate_target(tx, target, false).await?;
    let (tenant_authorized, organization_authorized, role_authorized): (bool, bool, bool) = sqlx::query_as(&format!(
        "WITH entries AS (SELECT m.id FROM menus m WHERE m.application_id=$1 AND {ENTRY_MENU}), \
         tenant_entries AS (SELECT e.id FROM entries e JOIN tenant_menus tm ON tm.menu_id=e.id AND tm.tenant_id=$2 \
           WHERE EXISTS(SELECT 1 FROM tenant_applications WHERE tenant_id=$2 AND application_id=$1)), \
         organization_entries AS (SELECT e.id FROM tenant_entries e JOIN organization_menus om ON om.menu_id=e.id AND om.organization_id=$3 \
           WHERE EXISTS(SELECT 1 FROM organization_applications WHERE organization_id=$3 AND application_id=$1)) \
         SELECT EXISTS(SELECT 1 FROM tenant_entries),EXISTS(SELECT 1 FROM organization_entries), \
           EXISTS(SELECT 1 FROM organization_entries e JOIN role_menus rm ON rm.menu_id=e.id AND rm.role_id=$4)"
    )).bind(app_id).bind(target.tenant_id).bind(target.organization_id).bind(target.role_id)
        .fetch_one(&mut **tx).await?;
    let employee_support =
        super::users::role_employee_support_sql("ur.user_id", "ur.organization_id", "NULL::uuid[]");
    let assigned_user_count: i64 = sqlx::query_scalar(&format!(
        "SELECT count(DISTINCT ur.user_id) FROM user_roles ur \
         JOIN users u ON u.id=ur.user_id AND u.status='ENABLED' \
         WHERE ur.role_id=$1 AND ur.organization_id=$2 AND {employee_support}",
    ))
    .bind(target.role_id)
    .bind(target.organization_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(
        json!({"tenantId":target.tenant_id,"organizationId":target.organization_id,"roleId":target.role_id,
        "tenantAuthorized":tenant_authorized,"organizationAuthorized":organization_authorized,
        "roleAuthorized":role_authorized,"assignedUserCount":assigned_user_count}),
    )
}

async fn read(
    State(state): State<AppState>,
    actor: CurrentUser,
    Path(id): Path<Uuid>,
    query: Result<Query<SetupQuery>, QueryRejection>,
) -> Result<Json<Value>, AppError> {
    let id = crate::util::uuid7::validate_uuid(id)?;
    RequirePermission("system:application")
        .check(&state, &actor)
        .await?;
    state.require_platform(&actor).await?;
    let Query(query) = query.map_err(|_| AppError::bad_request("应用接入查询参数无效"))?;
    let target = query.validate()?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let permissions = actor_permissions(&mut tx, &actor).await?;
    let can_configure_access = can_configure(&permissions);
    if target.is_some() && !can_configure_access {
        return Err(AppError::forbidden("无访问范围配置权限"));
    }
    let app = application(&mut tx, id, false).await?;
    let micro = app.base_path.trim_end_matches('/') == format!("/micro/{}", app.code);
    let (entry_source, proxy_enabled): (String, bool) =
        sqlx::query_as("SELECT source,proxy_enabled FROM application_effective_entry($1,$2)")
            .bind(id)
            .bind(&query.environment)
            .fetch_one(&mut *tx)
            .await?;
    let url_entry_configured = entry_source == "URL";
    let qiankun = !micro || (app.entry_type == "HTML" && app.runtime_mode == "TRUSTED");
    let release_required = micro && app.entry_type == "HTML" && !url_entry_configured;
    let deployment: Option<(String, String, String)> = sqlx::query_as(
        "SELECT r.version,r.validation_status,r.entry_type FROM application_deployments d \
         JOIN application_releases r ON r.id=d.active_release_id AND r.application_id=d.application_id AND r.environment=d.environment \
         WHERE d.application_id=$1 AND d.environment=$2",
    ).bind(id).bind(&query.environment).fetch_optional(&mut *tx).await?;
    let release_validated = if !qiankun {
        false
    } else if release_required {
        deployment
            .as_ref()
            .is_some_and(|(_, status, kind)| status == "PASSED" && kind == "HTML")
    } else {
        url_entry_configured || !micro
    };
    let (menu_count, entry_menu_count): (i64, i64) = sqlx::query_as(&format!(
        "SELECT count(*),count(*) FILTER(WHERE {ENTRY_MENU}) FROM menus m WHERE m.application_id=$1"
    ))
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    let access = match target {
        Some(target) => access_evidence(&mut tx, id, target).await?,
        None => Value::Null,
    };
    tx.commit().await?;
    Ok(Json(
        json!({"applicationId":id,"environment":query.environment,"applicationEnabled":app.status=="ENABLED",
        "entryType":app.entry_type,"runtimeMode":app.runtime_mode,"releaseRequired":release_required,
        "activeVersion":if url_entry_configured { None } else { deployment.map(|value|value.0) },"releaseValidated":release_validated,
        "urlEntryConfigured":url_entry_configured,"entrySource":entry_source,"proxyEnabled":proxy_enabled,
        "menuCount":menu_count,"entryMenuCount":entry_menu_count,"canConfigureAccess":can_configure_access,"access":access}),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AccessBody {
    #[serde(deserialize_with = "crate::util::uuid7::deserialize")]
    tenant_id: Uuid,
    #[serde(deserialize_with = "crate::util::uuid7::deserialize")]
    organization_id: Uuid,
    #[serde(deserialize_with = "crate::util::uuid7::deserialize")]
    role_id: Uuid,
    #[serde(deserialize_with = "crate::util::uuid7::deserialize_vec")]
    menu_ids: Vec<Uuid>,
}

fn normalize_menu_ids(ids: Vec<Uuid>) -> Result<Vec<Uuid>, AppError> {
    if ids.is_empty() || ids.len() > 500 || ids.iter().any(Uuid::is_nil) {
        return Err(AppError::bad_request("请选择 1 至 500 个有效菜单"));
    }
    let mut ids = ids;
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

async fn configure_access(
    State(state): State<AppState>,
    actor: CurrentUser,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    ApiJson(body): ApiJson<AccessBody>,
) -> Result<Json<Value>, AppError> {
    let id = crate::util::uuid7::validate_uuid(id)?;
    state.require_platform(&actor).await?;
    for permission in ALL_PERMISSIONS {
        RequirePermission(permission).check(&state, &actor).await?;
    }
    let menus = normalize_menu_ids(body.menu_ids)?;
    let target = Target {
        tenant_id: body.tenant_id,
        organization_id: body.organization_id,
        role_id: body.role_id,
    };
    if [target.tenant_id, target.organization_id, target.role_id]
        .iter()
        .any(Uuid::is_nil)
    {
        return Err(AppError::bad_request("目标 ID 无效"));
    }
    let mut tx = state.pool.begin().await?;
    // The app lock also serializes menu moves/removals before their FK inserts.
    let app = application(&mut tx, id, true).await?;
    if app.status != "ENABLED" {
        return Err(AppError::forbidden("应用已停用"));
    }
    validate_target_parents(&mut tx, target, true).await?;
    // Role membership edits lock their users before their role. Taking the
    // actor lock after tenant/org parents but before the target role matches
    // both contact initialization (parents -> U) and membership edits (U -> R).
    session::require_active_in_tx(
        &mut tx,
        actor.user_id,
        actor.tenant_id,
        actor.org_id,
        actor.session_id,
    )
    .await?;
    if !can_configure(&actor_permissions(&mut tx, &actor).await?) {
        return Err(AppError::forbidden("访问范围配置权限已撤销"));
    }
    let managed = validate_target_role(&mut tx, target, true).await?;
    if managed {
        return Err(AppError::conflict(
            "默认管理员角色随组织授权自动同步，请选择普通角色",
        ));
    }
    let valid: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM menus WHERE application_id=$1 AND status='ENABLED' AND id=ANY($2)",
    )
    .bind(id)
    .bind(&menus)
    .fetch_one(&mut *tx)
    .await?;
    if valid != menus.len() as i64 {
        return Err(AppError::bad_request("存在不属于本应用或已停用的菜单"));
    }
    let mut added = sqlx::query("INSERT INTO tenant_applications(tenant_id,application_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(target.tenant_id).bind(id).execute(&mut *tx).await?.rows_affected();
    added += sqlx::query("INSERT INTO tenant_menus(tenant_id,menu_id) SELECT $1,unnest($2::uuid[]) ON CONFLICT DO NOTHING")
        .bind(target.tenant_id).bind(&menus).execute(&mut *tx).await?.rows_affected();
    added += sqlx::query("INSERT INTO organization_applications(organization_id,application_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(target.organization_id).bind(id).execute(&mut *tx).await?.rows_affected();
    added += sqlx::query("INSERT INTO organization_menus(organization_id,menu_id) SELECT $1,unnest($2::uuid[]) ON CONFLICT DO NOTHING")
        .bind(target.organization_id).bind(&menus).execute(&mut *tx).await?.rows_affected();
    added += sqlx::query("INSERT INTO role_menus(role_id,menu_id) SELECT $1,unnest($2::uuid[]) ON CONFLICT DO NOTHING")
        .bind(target.role_id).bind(&menus).execute(&mut *tx).await?.rows_affected();
    if added > 0 {
        super::onboarding::sync_tenant_admin(&mut tx, target.tenant_id).await?;
        super::onboarding::sync_org_admin(&mut tx, target.organization_id).await?;
        let detail =
            json!({"target":target,"menuIds":menus,"added":added,"mode":"ADDITIVE"}).to_string();
        audit::write_in_tx_for_app(
            &mut tx,
            audit::AuditInfo {
                user_id: Some(actor.user_id),
                username: &actor.username,
                tenant_id: Some(target.tenant_id),
                organization_id: Some(target.organization_id),
                action: "APPLICATION_SETUP_ACCESS",
                detail: &detail,
                ip: &audit::client_ip(&headers),
                user_agent: &audit::ua_of(&headers),
            },
            id,
        )
        .await?;
    }
    // Recheck after audit or managed-role synchronization waits before commit.
    if !can_configure(&actor_permissions(&mut tx, &actor).await?) {
        return Err(AppError::forbidden("访问范围配置权限已撤销"));
    }
    validate_target(&mut tx, target, false).await?;
    tx.commit().await?;
    state.invalidate_perm_cache();
    Ok(Json(json!({"ok":true,"added":added})))
}

#[cfg(test)]
#[path = "application_setup_tests.rs"]
mod tests;
