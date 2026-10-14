use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::state::ORG_ANCESTORS_SQL;
use crate::util::tree;

type MenuRow = (Uuid, Option<Uuid>, String, String, String, String, String, Option<String>, Option<String>, i32, bool, DateTime<Utc>);

/// 菜单显示和操作权限使用同一批已授权菜单行；传入 None 可跨已授权应用计算平台操作权限。
pub async fn effective_menu_rows(
    pool: &PgPool, user_id: Uuid, tenant_id: Uuid, org_id: Uuid, app_id: Option<Uuid>,
) -> Result<Vec<Value>, AppError> {
    let sql = format!(
        "{ORG_ANCESTORS_SQL} SELECT DISTINCT m.id, m.parent_id, m.type, m.title, \
         COALESCE(m.path,''), COALESCE(m.icon,''), COALESCE(m.permission,''), \
         m.link_url, m.link_target, m.sort_order, m.visible, m.created_at \
         FROM menus m \
         JOIN role_menus rm ON rm.menu_id = m.id \
         JOIN roles r ON r.id = rm.role_id AND r.status = 'ENABLED' \
         JOIN anc ON anc.id = r.organization_id \
         JOIN user_roles ur ON ur.role_id = r.id AND ur.user_id = $2 \
         JOIN tenant_menus tm ON tm.menu_id = m.id AND tm.tenant_id = $3 \
         JOIN organization_menus om ON om.menu_id = m.id AND om.organization_id = $1 \
         JOIN tenant_applications ta ON ta.application_id = m.application_id AND ta.tenant_id = $3 \
         JOIN organization_applications oa ON oa.application_id = m.application_id AND oa.organization_id = $1 \
         JOIN applications app ON app.id = m.application_id AND app.status = 'ENABLED' \
         WHERE m.status = 'ENABLED' AND ($4::uuid IS NULL OR m.application_id = $4) \
         ORDER BY m.sort_order, m.created_at"
    );
    let rows: Vec<MenuRow> = sqlx::query_as(&sql)
        .bind(org_id).bind(user_id).bind(tenant_id).bind(app_id)
        .fetch_all(pool).await?;
    Ok(rows_to_json(rows))
}

/// A global administrator still observes the selected tenant and organization grants.
/// Only the user-role layer is bypassed; application and menu grants remain mandatory.
pub async fn granted_menu_rows(
    pool: &PgPool, tenant_id: Uuid, org_id: Uuid, app_id: Option<Uuid>,
) -> Result<Vec<Value>, AppError> {
    let rows: Vec<MenuRow> = sqlx::query_as(
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
         ORDER BY m.sort_order, m.created_at",
    ).bind(tenant_id).bind(org_id).bind(app_id).fetch_all(pool).await?;
    Ok(rows_to_json(rows))
}

fn rows_to_json(rows: Vec<MenuRow>) -> Vec<Value> {
    rows.into_iter().map(|(id, parent, mtype, title, path, icon, permission, link_url, link_target, sort, visible, _)| {
        json!({
            "id": id, "parentId": parent, "type": mtype, "title": title,
            "path": path, "icon": icon, "permission": permission,
            "linkUrl": link_url, "linkTarget": link_target,
            "sortOrder": sort, "visible": visible,
        })
    }).collect()
}

pub fn permissions_of(rows: &[Value]) -> HashSet<String> {
    rows.iter().filter_map(|row| row["permission"].as_str())
        .filter(|code| !code.is_empty()).map(str::to_owned).collect()
}

pub fn menu_tree(rows: Vec<Value>) -> Vec<Value> {
    tree::build_tree(rows)
}
