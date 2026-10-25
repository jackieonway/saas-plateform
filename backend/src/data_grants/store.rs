//! SQL storage boundary for the controlled data-grant use case.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::{Target, MAX_GRANTS, MAX_ORGANIZATIONS};
use crate::{authorization, error::AppError, extractors::CurrentUser};

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
pub(super) struct GrantRow {
    pub id: Uuid,
    pub resource: String,
    pub action: String,
    pub scope: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow, Serialize)]
struct OrganizationNode {
    id: Uuid,
    parent_id: Option<Uuid>,
    active: bool,
    expires_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow, Serialize)]
struct TenantState {
    active: bool,
    is_platform: bool,
    expires_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

pub(super) struct Runtime {
    pub actor_platform_root: bool,
    pub target_platform_root: bool,
    pub permissions: Vec<String>,
    pub target_ancestors: Vec<Uuid>,
    pub subtree: Vec<Uuid>,
    pub actor_subtree: Vec<Uuid>,
    pub registered: BTreeSet<String>,
    pub fingerprint: Value,
}

pub(super) struct ActorRuntime {
    pub platform_root: bool,
    pub permissions: Vec<String>,
    pub subtree: Vec<Uuid>,
    pub ancestors: Vec<Uuid>,
    pub identity: authorization::FreshEmployee,
}

/// Users precede memberships, matching the existing membership/password paths.
/// Tenant/organization rows deliberately receive no later strong locks: legacy
/// onboarding acquires those parents before existing users. Authorization is
/// re-read after waits, and again after mutations before commit.
pub(super) async fn lock_target(
    tx: &mut Transaction<'_, Postgres>,
    actor: &CurrentUser,
    target: &Target,
) -> Result<(), AppError> {
    let mut users = vec![actor.user_id, target.user_id];
    users.sort_unstable();
    users.dedup();
    let locked: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM users WHERE id=ANY($1) ORDER BY id FOR NO KEY UPDATE")
            .bind(&users)
            .fetch_all(&mut **tx)
            .await?;
    if !locked.contains(&actor.user_id) {
        return Err(AppError::unauthorized("会话用户已失效"));
    }
    authorization::require_employee_context_in_tx(tx, actor).await?;
    if !locked.contains(&target.user_id) {
        return Err(AppError::not_found("目标成员不存在"));
    }
    let membership: Option<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT user_id, organization_id FROM memberships WHERE id=$1 \
            AND member_type='EMPLOYEE' AND status='ENABLED' FOR UPDATE",
    )
    .bind(target.membership_id)
    .fetch_optional(&mut **tx)
    .await?;
    if membership != Some((target.user_id, target.organization_id)) {
        return Err(AppError::not_found("目标成员与用户或组织不匹配"));
    }
    let organization_matches: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM organizations WHERE id=$1 AND tenant_id=$2)",
    )
    .bind(target.organization_id)
    .bind(target.tenant_id)
    .fetch_one(&mut **tx)
    .await?;
    if !organization_matches {
        return Err(AppError::not_found("目标租户与组织不匹配"));
    }
    Ok(())
}

pub(super) async fn grants(
    tx: &mut Transaction<'_, Postgres>,
    target: &Target,
) -> Result<Vec<GrantRow>, AppError> {
    let mut rows: Vec<GrantRow> = sqlx::query_as(
        "SELECT id, resource_code AS resource, action_code AS action, scope_type AS scope, updated_at \
         FROM membership_data_grants WHERE membership_id=$1 AND application_id=$2 \
         ORDER BY resource_code, action_code LIMIT $3",
    )
    .bind(target.membership_id)
    .bind(target.application_id)
    .bind((MAX_GRANTS + 1) as i64)
    .fetch_all(&mut **tx)
    .await?;
    rows.sort_unstable_by(|left, right| {
        (&left.resource, &left.action).cmp(&(&right.resource, &right.action))
    });
    Ok(rows)
}

async fn organization_chain(
    tx: &mut Transaction<'_, Postgres>,
    tenant: Uuid,
    organization: Uuid,
) -> Result<Vec<OrganizationNode>, AppError> {
    let rows: Vec<OrganizationNode> = sqlx::query_as(
        "WITH RECURSIVE chain AS ( \
         SELECT id, parent_id, tenant_id, status, expires_at, updated_at FROM organizations \
         WHERE id=$1 AND tenant_id=$2 UNION \
         SELECT p.id,p.parent_id,p.tenant_id,p.status,p.expires_at,p.updated_at \
         FROM organizations p JOIN chain c ON p.id=c.parent_id WHERE p.tenant_id=$2) \
         SELECT id,parent_id,(status='ENABLED' AND (expires_at IS NULL OR expires_at>clock_timestamp())) \
         AS active,expires_at,updated_at FROM chain LIMIT $3",
    )
    .bind(organization)
    .bind(tenant)
    .bind((MAX_ORGANIZATIONS + 1) as i64)
    .fetch_all(&mut **tx)
    .await?;
    if rows.len() > MAX_ORGANIZATIONS {
        return Err(AppError::bad_request("组织范围超过 10000 项上限"));
    }
    if rows.is_empty() {
        return Err(AppError::not_found("目标租户与组织不匹配"));
    }
    if rows.iter().any(|node| !node.active) || !rows.iter().any(|node| node.parent_id.is_none()) {
        return Err(AppError::forbidden("组织或祖先组织已停用、到期或关系无效"));
    }
    Ok(rows)
}

async fn tenant_state(
    tx: &mut Transaction<'_, Postgres>,
    tenant: Uuid,
) -> Result<TenantState, AppError> {
    let state: TenantState = sqlx::query_as(
        "SELECT (status='ENABLED' AND (expires_at IS NULL OR expires_at>clock_timestamp())) AS active, \
         is_platform,expires_at,updated_at FROM tenants WHERE id=$1",
    )
    .bind(tenant)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| AppError::not_found("目标租户不存在"))?;
    if !state.active {
        return Err(AppError::forbidden("租户已停用或到期"));
    }
    Ok(state)
}

async fn organization_subtree(
    tx: &mut Transaction<'_, Postgres>,
    tenant: Uuid,
    organization: Uuid,
) -> Result<Vec<Uuid>, AppError> {
    // No outer ORDER BY: PostgreSQL can stop recursive expansion once LIMIT
    // detects an oversized tree. Sort the bounded result in memory instead.
    let mut ids: Vec<Uuid> = sqlx::query_scalar(
        "WITH RECURSIVE tree AS ( \
         SELECT id,tenant_id FROM organizations WHERE id=$1 AND tenant_id=$2 \
         AND status='ENABLED' AND (expires_at IS NULL OR expires_at>clock_timestamp()) UNION \
         SELECT o.id,o.tenant_id FROM organizations o JOIN tree p ON o.parent_id=p.id \
         WHERE o.tenant_id=$2 AND o.status='ENABLED' \
         AND (o.expires_at IS NULL OR o.expires_at>clock_timestamp())) \
         SELECT id FROM tree LIMIT $3",
    )
    .bind(organization)
    .bind(tenant)
    .bind((MAX_ORGANIZATIONS + 1) as i64)
    .fetch_all(&mut **tx)
    .await?;
    if ids.len() > MAX_ORGANIZATIONS {
        return Err(AppError::bad_request("组织范围超过 10000 项上限"));
    }
    ids.sort_unstable();
    Ok(ids)
}

pub(super) async fn actor_runtime(
    tx: &mut Transaction<'_, Postgres>,
    actor: &CurrentUser,
) -> Result<ActorRuntime, AppError> {
    let actor_identity = authorization::require_employee_context_in_tx(tx, actor).await?;
    let actor_tenant = tenant_state(tx, actor.tenant_id).await?;
    let actor_chain = organization_chain(tx, actor.tenant_id, actor.org_id).await?;
    let actor_platform_root = actor_tenant.is_platform
        && actor_chain
            .iter()
            .any(|node| node.id == actor.org_id && node.parent_id.is_none());
    let permissions = actor_permissions(tx, actor, actor_identity.is_super_admin).await?;
    let subtree = if actor_platform_root {
        Vec::new()
    } else {
        organization_subtree(tx, actor.tenant_id, actor.org_id).await?
    };
    let mut ancestors: Vec<Uuid> = actor_chain.iter().map(|node| node.id).collect();
    ancestors.sort_unstable();
    if authorization::require_employee_context_in_tx(tx, actor).await? != actor_identity {
        return Err(AppError::unauthorized("员工身份已在数据授权计算期间变更"));
    }
    Ok(ActorRuntime {
        platform_root: actor_platform_root,
        permissions,
        subtree,
        ancestors,
        identity: actor_identity,
    })
}

pub(super) async fn policy_root_active(
    tx: &mut Transaction<'_, Postgres>,
    tenant: Uuid,
    organization: Uuid,
) -> Result<bool, AppError> {
    let result = async {
        tenant_state(tx, tenant).await?;
        organization_chain(tx, tenant, organization).await?;
        Ok::<_, AppError>(())
    }
    .await;
    match result {
        Ok(()) => Ok(true),
        Err(AppError::Forbidden(_)) | Err(AppError::NotFound(_)) => Ok(false),
        Err(error) => Err(error),
    }
}

pub(super) async fn ancestors(
    tx: &mut Transaction<'_, Postgres>,
    tenant: Uuid,
    organization: Uuid,
) -> Result<Vec<Uuid>, AppError> {
    Ok(organization_chain(tx, tenant, organization)
        .await?
        .into_iter()
        .map(|node| node.id)
        .collect())
}

pub(super) async fn runtime(
    tx: &mut Transaction<'_, Postgres>,
    actor: &CurrentUser,
    target: &Target,
    registered_permissions: &[String],
) -> Result<Runtime, AppError> {
    let actor_runtime = actor_runtime(tx, actor).await?;
    let target_version: Option<(i64,i64)> = sqlx::query_as(
        "SELECT u.session_version,m.identity_version FROM users u JOIN memberships m ON m.user_id=u.id \
         JOIN organizations o ON o.id=m.organization_id WHERE u.id=$1 AND u.status='ENABLED' \
         AND m.id=$2 AND m.organization_id=$3 AND o.tenant_id=$4 \
         AND m.member_type='EMPLOYEE' AND m.status='ENABLED'",
    )
    .bind(target.user_id)
    .bind(target.membership_id)
    .bind(target.organization_id)
    .bind(target.tenant_id)
    .fetch_optional(&mut **tx)
    .await?;
    let target_version = target_version.ok_or_else(|| AppError::forbidden("目标直接成员已失效"))?;
    let target_tenant = tenant_state(tx, target.tenant_id).await?;
    let target_chain = organization_chain(tx, target.tenant_id, target.organization_id).await?;
    let target_platform_root = target_tenant.is_platform
        && target_chain
            .iter()
            .any(|node| node.id == target.organization_id && node.parent_id.is_none());
    let app: Option<(String, DateTime<Utc>, Vec<String>,i64)> = sqlx::query_as(
        "SELECT a.status,a.updated_at,a.allowed_member_types,a.access_policy_version FROM applications a \
         JOIN tenant_applications ta ON ta.application_id=a.id AND ta.tenant_id=$2 \
         JOIN organization_applications oa ON oa.application_id=a.id AND oa.organization_id=$3 WHERE a.id=$1",
    )
    .bind(target.application_id)
    .bind(target.tenant_id)
    .bind(target.organization_id)
    .fetch_optional(&mut **tx)
    .await?;
    let app = app.ok_or_else(|| AppError::forbidden("目标应用未授权给租户或组织"))?;
    if app.0 != "ENABLED" || !app.2.iter().any(|kind| kind == "EMPLOYEE") {
        return Err(AppError::forbidden("目标应用已停用"));
    }
    let registry: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT permission FROM menus WHERE application_id=$1 AND status='ENABLED' \
         AND permission=ANY($2) ORDER BY permission",
    )
    .bind(target.application_id)
    .bind(registered_permissions)
    .fetch_all(&mut **tx)
    .await?;
    let subtree = organization_subtree(tx, target.tenant_id, target.organization_id).await?;
    let mut target_ancestors: Vec<Uuid> = target_chain.iter().map(|node| node.id).collect();
    target_ancestors.sort_unstable();
    let registered = registry.iter().cloned().collect();
    let fingerprint = json!({
        "targetVersion":target_version,"targetTenant":target_tenant,"targetChain":target_chain,
        "application":app,"registry":registry,"subtree":subtree,
    });
    Ok(Runtime {
        actor_platform_root: actor_runtime.platform_root,
        target_platform_root,
        permissions: actor_runtime.permissions,
        target_ancestors,
        subtree,
        actor_subtree: actor_runtime.subtree,
        registered,
        fingerprint,
    })
}

async fn actor_permissions(
    tx: &mut Transaction<'_, Postgres>,
    actor: &CurrentUser,
    super_admin: bool,
) -> Result<Vec<String>, AppError> {
    Ok(sqlx::query_scalar(
        "WITH RECURSIVE anc AS (SELECT id,parent_id FROM organizations WHERE id=$1 AND tenant_id=$2 \
         UNION SELECT o.id,o.parent_id FROM organizations o JOIN anc c ON o.id=c.parent_id WHERE o.tenant_id=$2) \
         SELECT DISTINCT m.permission FROM menus m JOIN applications a ON a.id=m.application_id AND a.status='ENABLED' \
         JOIN tenant_applications ta ON ta.application_id=a.id AND ta.tenant_id=$2 \
         JOIN organization_applications oa ON oa.application_id=a.id AND oa.organization_id=$1 \
         JOIN tenant_menus tm ON tm.menu_id=m.id AND tm.tenant_id=$2 \
         JOIN organization_menus om ON om.menu_id=m.id AND om.organization_id=$1 \
         WHERE m.status='ENABLED' AND 'EMPLOYEE'=ANY(a.allowed_member_types) \
         AND m.permission=ANY($5) AND ($4 OR EXISTS( \
         SELECT 1 FROM role_menus rm JOIN roles r ON r.id=rm.role_id AND r.status='ENABLED' AND r.tenant_id=$2 \
         JOIN anc ON anc.id=r.organization_id JOIN user_roles ur ON ur.role_id=r.id AND ur.user_id=$3 \
         WHERE rm.menu_id=m.id)) ORDER BY m.permission",
    )
    .bind(actor.org_id)
    .bind(actor.tenant_id)
    .bind(actor.user_id)
    .bind(super_admin)
    .bind([super::READ_PERMISSION, super::WRITE_PERMISSION])
    .fetch_all(&mut **tx)
    .await?)
}

/// Insertions precede locking old grant rows. Their application FK key-share
/// lock therefore cannot invert application deletion's app-before-grant order.
pub(super) async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    target: &Target,
    resource: &str,
    action: &str,
    scope: &str,
) -> Result<(), AppError> {
    let changed = sqlx::query(
        "INSERT INTO membership_data_grants(membership_id,application_id,resource_code,action_code,scope_type, id) \
         VALUES($1,$2,$3,$4,$5, $6)",
    ).bind(target.membership_id).bind(target.application_id).bind(resource).bind(action).bind(scope)
        .bind(crate::util::uuid7::new_uuid()).execute(&mut **tx).await?.rows_affected();
    require_one(changed)
}

pub(super) async fn update(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    scope: &str,
) -> Result<(), AppError> {
    let changed = sqlx::query(
        "UPDATE membership_data_grants SET scope_type=$2,updated_at=clock_timestamp() WHERE id=$1",
    )
    .bind(id)
    .bind(scope)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    require_one(changed)
}

pub(super) async fn remove(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<(), AppError> {
    let changed = sqlx::query("DELETE FROM membership_data_grants WHERE id=$1")
        .bind(id)
        .execute(&mut **tx)
        .await?
        .rows_affected();
    require_one(changed)
}

fn require_one(changed: u64) -> Result<(), AppError> {
    if changed != 1 {
        return Err(AppError::conflict("数据授权已变化，请重新查询"));
    }
    Ok(())
}
