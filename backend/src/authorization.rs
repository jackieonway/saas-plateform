use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::CurrentUser;
use crate::state::ORG_ANCESTORS_SQL;
use crate::util::tree;

type MenuRow = (
    Uuid,
    Option<Uuid>,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    i32,
    bool,
    DateTime<Utc>,
);

/// Fresh employee proof for the one context selected by the current session.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct FreshEmployee {
    pub(crate) membership_id: Uuid,
    pub(crate) identity_version: i64,
    pub(crate) account_version: i64,
    pub(crate) is_super_admin: bool,
}

#[derive(sqlx::FromRow)]
struct ContextIdentity {
    membership_id: Uuid,
    member_type: String,
    identity_version: i64,
    account_version: i64,
    is_super_admin: bool,
}

pub(crate) async fn require_employee_context(
    pool: &PgPool,
    actor: &CurrentUser,
) -> Result<FreshEmployee, AppError> {
    let mut connection = pool.acquire().await.map_err(|_| AppError::Unavailable)?;
    employee_context(&mut connection, actor).await
}

/// Recheck inside the caller's existing transaction after its ordered locks.
/// This helper deliberately adds no locks or alternate transaction snapshot.
pub(crate) async fn require_employee_context_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    actor: &CurrentUser,
) -> Result<FreshEmployee, AppError> {
    employee_context(tx.as_mut(), actor).await
}

async fn employee_context(
    connection: &mut PgConnection,
    actor: &CurrentUser,
) -> Result<FreshEmployee, AppError> {
    let identity: Option<ContextIdentity> = sqlx::query_as(
        "SELECT s.membership_id,s.member_type,s.identity_version,s.account_version,u.is_super_admin \
         FROM platform_sessions s JOIN users u ON u.id=s.user_id \
         WHERE s.id=$1 AND s.user_id=$2 AND s.tenant_id=$3 AND s.organization_id=$4 \
         AND platform_session_active(s.id)",
    ).bind(actor.session_id).bind(actor.user_id).bind(actor.tenant_id).bind(actor.org_id)
        .fetch_optional(connection).await.map_err(|_| AppError::Unavailable)?;
    let identity = identity.ok_or_else(|| AppError::unauthorized("当前成员会话已失效"))?;
    if identity.member_type != "EMPLOYEE" {
        return Err(AppError::forbidden("当前成员身份不是员工"));
    }
    Ok(FreshEmployee {
        membership_id: identity.membership_id,
        identity_version: identity.identity_version,
        account_version: identity.account_version,
        is_super_admin: identity.is_super_admin,
    })
}

/// Employee qualification precedes either ordinary roles or administrator bypass.
pub(crate) async fn context_menu_rows(
    pool: &PgPool,
    actor: &CurrentUser,
    app_id: Option<Uuid>,
) -> Result<Vec<Value>, AppError> {
    let proof = require_employee_context(pool, actor).await?;
    let rows = if proof.is_super_admin {
        granted_menu_rows(pool, actor.tenant_id, actor.org_id, app_id).await?
    } else {
        effective_menu_rows(pool, actor.user_id, actor.tenant_id, actor.org_id, app_id).await?
    };
    // Permission reads can wait behind concurrent edits. Never return a role
    // projection using an identity or administrator flag changed during it.
    if require_employee_context(pool, actor).await? != proof {
        return Err(AppError::unauthorized("员工身份已在授权计算期间变更"));
    }
    Ok(rows)
}

/// Permission projection inside an already ordered mutation transaction.
pub(crate) async fn context_menu_rows_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    actor: &CurrentUser,
    app_id: Option<Uuid>,
) -> Result<Vec<Value>, AppError> {
    let proof = require_employee_context_in_tx(tx, actor).await?;
    let rows: Vec<MenuRow> = if proof.is_super_admin {
        sqlx::query_as(granted_menu_sql())
            .bind(actor.tenant_id)
            .bind(actor.org_id)
            .bind(app_id)
            .fetch_all(&mut **tx)
            .await?
    } else {
        sqlx::query_as(&effective_menu_sql())
            .bind(actor.org_id)
            .bind(actor.user_id)
            .bind(actor.tenant_id)
            .bind(app_id)
            .fetch_all(&mut **tx)
            .await?
    };
    if require_employee_context_in_tx(tx, actor).await? != proof {
        return Err(AppError::unauthorized("员工身份已在授权计算期间变更"));
    }
    Ok(rows_to_json(rows))
}

/// Employee role projection for an already authenticated subject lookup.
/// Request authorization uses `context_menu_rows` to prove the selected session.
pub async fn effective_menu_rows(
    pool: &PgPool,
    user_id: Uuid,
    tenant_id: Uuid,
    org_id: Uuid,
    app_id: Option<Uuid>,
) -> Result<Vec<Value>, AppError> {
    let sql = effective_menu_sql();
    let rows: Vec<MenuRow> = sqlx::query_as(&sql)
        .bind(org_id)
        .bind(user_id)
        .bind(tenant_id)
        .bind(app_id)
        .fetch_all(pool)
        .await?;
    Ok(rows_to_json(rows))
}

/// A global administrator still observes the selected tenant and organization grants.
/// Only the user-role layer is bypassed; application and menu grants remain mandatory.
/// The caller must first prove its exact employee context and fresh administrator flag.
pub async fn granted_menu_rows(
    pool: &PgPool,
    tenant_id: Uuid,
    org_id: Uuid,
    app_id: Option<Uuid>,
) -> Result<Vec<Value>, AppError> {
    let rows: Vec<MenuRow> = sqlx::query_as(granted_menu_sql())
        .bind(tenant_id)
        .bind(org_id)
        .bind(app_id)
        .fetch_all(pool)
        .await?;
    Ok(rows_to_json(rows))
}

fn rows_to_json(rows: Vec<MenuRow>) -> Vec<Value> {
    rows.into_iter()
        .map(
            |(
                id,
                parent,
                mtype,
                title,
                path,
                icon,
                permission,
                link_url,
                link_target,
                sort,
                visible,
                _,
            )| {
                json!({
                    "id": id, "parentId": parent, "type": mtype, "title": title,
                    "path": path, "icon": icon, "permission": permission,
                    "linkUrl": link_url, "linkTarget": link_target,
                    "sortOrder": sort, "visible": visible,
                })
            },
        )
        .collect()
}

pub fn permissions_of(rows: &[Value]) -> HashSet<String> {
    rows.iter()
        .filter_map(|row| row["permission"].as_str())
        .filter(|code| !code.is_empty())
        .map(str::to_owned)
        .collect()
}

pub fn menu_tree(rows: Vec<Value>) -> Vec<Value> {
    tree::build_tree(rows)
}

pub(crate) fn effective_menu_sql() -> String {
    format!(
        "{ORG_ANCESTORS_SQL} SELECT DISTINCT m.id, m.parent_id, m.type, m.title, \
         COALESCE(m.path,''), COALESCE(m.icon,''), COALESCE(m.permission,''), \
         m.link_url, m.link_target, m.sort_order, m.visible, m.created_at \
         FROM menus m \
         JOIN role_menus rm ON rm.menu_id = m.id \
         JOIN roles r ON r.id = rm.role_id AND r.status = 'ENABLED' AND r.tenant_id=$3 \
         JOIN anc ON anc.id = r.organization_id \
         JOIN user_roles ur ON ur.role_id = r.id AND ur.user_id = $2 \
         JOIN tenant_menus tm ON tm.menu_id = m.id AND tm.tenant_id = $3 \
         JOIN organization_menus om ON om.menu_id = m.id AND om.organization_id = $1 \
         JOIN tenant_applications ta ON ta.application_id = m.application_id AND ta.tenant_id = $3 \
         JOIN organization_applications oa ON oa.application_id = m.application_id AND oa.organization_id = $1 \
         JOIN applications app ON app.id = m.application_id AND app.status = 'ENABLED' \
         WHERE m.status = 'ENABLED' AND ($4::uuid IS NULL OR m.application_id = $4) \
         AND 'EMPLOYEE'=ANY(app.allowed_member_types) \
         AND EXISTS(SELECT 1 FROM memberships employee JOIN users subject ON subject.id=employee.user_id \
           JOIN organizations selected ON selected.id=employee.organization_id AND selected.tenant_id=$3 \
           JOIN tenants tenant ON tenant.id=selected.tenant_id \
           WHERE employee.user_id=$2 AND employee.organization_id=$1 AND employee.member_type='EMPLOYEE' \
           AND employee.status='ENABLED' AND subject.status='ENABLED' AND tenant.status='ENABLED' \
           AND (tenant.expires_at IS NULL OR tenant.expires_at>clock_timestamp()) \
           AND org_chain_active(selected.id)) \
         ORDER BY m.sort_order, m.created_at"
    )
}

pub(crate) fn granted_menu_sql() -> &'static str {
    "SELECT DISTINCT m.id, m.parent_id, m.type, m.title, COALESCE(m.path,''), \
         COALESCE(m.icon,''), COALESCE(m.permission,''), m.link_url, m.link_target, \
         m.sort_order, m.visible, m.created_at FROM menus m \
         JOIN applications app ON app.id = m.application_id AND app.status = 'ENABLED' \
         JOIN organizations org ON org.id = $2 AND org.tenant_id = $1 \
         JOIN tenant_applications ta ON ta.application_id = app.id AND ta.tenant_id = $1 \
         JOIN organization_applications oa ON oa.application_id = app.id AND oa.organization_id = $2 \
         JOIN tenant_menus tm ON tm.menu_id = m.id AND tm.tenant_id = $1 \
         JOIN organization_menus om ON om.menu_id = m.id AND om.organization_id = $2 \
         WHERE m.status = 'ENABLED' AND ($3::uuid IS NULL OR m.application_id = $3) \
         AND 'EMPLOYEE'=ANY(app.allowed_member_types) \
         ORDER BY m.sort_order, m.created_at"
}
