use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::{ApiJson, CurrentUser, RequirePermission};
use crate::state::AppState;
use crate::util::{audit, tree};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/departments/organizations", get(organization_options))
        .route("/api/departments/tree", get(tree_handler))
        .route("/api/departments", post(create))
        .route("/api/departments/{id}", put(update).delete(delete))
}

type DeptRowTuple = (Uuid, Uuid, Option<Uuid>, String, String, i32, String, String);
type DepartmentOrgRowTuple = (Uuid, Uuid, Option<Uuid>, String, String, String);

/// GET /api/departments/organizations — 当前会话组织及下级组织的部门管理选择范围。
async fn organization_options(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:department").check(&state, &user).await?;
    let rows: Vec<DepartmentOrgRowTuple> = sqlx::query_as(
        "WITH RECURSIVE scoped_orgs AS ( \
         SELECT id, parent_id FROM organizations WHERE id = $1 AND tenant_id = $2 \
         UNION SELECT o.id, o.parent_id FROM organizations o \
         JOIN scoped_orgs parent ON o.parent_id = parent.id WHERE o.tenant_id = $2 ) \
         SELECT o.id, o.tenant_id, CASE WHEN o.id = $1 THEN NULL::uuid ELSE o.parent_id END, \
         o.name, COALESCE(o.code, ''), o.status \
         FROM organizations o JOIN scoped_orgs scoped ON scoped.id = o.id \
         ORDER BY o.sort_order, o.created_at",
    )
    .bind(user.org_id)
    .bind(user.tenant_id)
    .fetch_all(&state.pool)
    .await?;
    let items = rows
        .into_iter()
        .map(|(id, tenant_id, parent_id, name, code, status)| {
            json!({
                "id": id, "tenantId": tenant_id, "parentId": parent_id,
                "name": name, "code": code, "status": status,
            })
        })
        .collect();
    Ok(Json(Value::Array(tree::build_tree(items))))
}

#[cfg(test)]
#[path = "department_organization_tests.rs"]
mod organization_tests;

/// 部门必须归属当前组织或下级组织；越界与不存在统一返回 404。
async fn ensure_dept_in_scope(state: &AppState, user: &CurrentUser, dept_id: Uuid) -> Result<(), AppError> {
    let org_id: Uuid = sqlx::query_scalar("SELECT organization_id FROM departments WHERE id=$1")
        .bind(dept_id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::not_found("部门不存在"))?;
    state.ensure_org_scope(user, org_id, "部门").await
}

fn dept_json(r: &DeptRowTuple) -> Value {
    let (id, organization_id, parent_id, name, code, sort_order, description, status) = r;
    json!({
        "id": id, "organizationId": organization_id, "parentId": parent_id, "name": name,
        "code": code, "sortOrder": sort_order, "description": description, "status": status,
    })
}

/// GET /api/departments/tree?organizationId=
async fn tree_handler(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:department").check(&state, &user).await?;
    let org_id: Uuid = params
        .get("organizationId")
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or_else(|| AppError::bad_request("缺少 organizationId 参数"))?;
    state.ensure_org_scope(&user, org_id, "组织").await?;
    let rows: Vec<DeptRowTuple> = sqlx::query_as(
        "SELECT d.id, d.organization_id, d.parent_id, d.name, COALESCE(d.code,'') AS code, \
         d.sort_order, COALESCE(d.description,'') AS description, d.status \
         FROM departments d WHERE d.organization_id = $1 ORDER BY d.sort_order, d.created_at",
    )
    .bind(org_id)
    .fetch_all(&state.pool)
    .await?;
    let items: Vec<Value> = rows.iter().map(dept_json).collect();
    Ok(Json(Value::Array(tree::build_tree(items))))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeptBody {
    organization_id: String,
    #[serde(default)]
    parent_id: Option<String>,
    name: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    sort_order: i32,
    #[serde(default)]
    description: String,
}

/// POST /api/departments
async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<DeptBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:department").check(&state, &user).await?;
    let org_id: Uuid = body
        .organization_id
        .parse()
        .map_err(|_| AppError::bad_request("organizationId 格式错误"))?;
    let org_tenant: Option<Uuid> = sqlx::query_scalar("SELECT tenant_id FROM organizations WHERE id = $1")
        .bind(org_id)
        .fetch_optional(&state.pool)
        .await?;
    let org_tenant = org_tenant.ok_or_else(|| AppError::not_found("组织不存在"))?;
    state.ensure_org_scope(&user, org_id, "组织").await?;
    let parent_id: Option<Uuid> = match &body.parent_id {
        None => None,
        Some(p) if p.trim().is_empty() => None,
        Some(p) => {
            let pid: Uuid = p.parse().map_err(|_| AppError::bad_request("parentId 格式错误"))?;
            let (parent_org,): (Uuid,) =
                sqlx::query_as("SELECT organization_id FROM departments WHERE id = $1")
                    .bind(pid)
                    .fetch_optional(&state.pool)
                    .await?
                    .ok_or_else(|| AppError::not_found("父部门不存在"))?;
            if parent_org != org_id {
                return Err(AppError::not_found("父部门不存在"));
            }
            Some(pid)
        }
    };
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::bad_request("部门名称不能为空"));
    }
    let row: DeptRowTuple = sqlx::query_as(
        "INSERT INTO departments (organization_id, parent_id, name, code, sort_order, description) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         RETURNING id, organization_id, parent_id, name, COALESCE(code,'') AS code, sort_order, \
         COALESCE(description,'') AS description, status",
    )
    .bind(org_id)
    .bind(parent_id)
    .bind(&name)
    .bind(&body.code)
    .bind(body.sort_order)
    .bind(&body.description)
    .fetch_one(&state.pool)
    .await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(org_tenant), organization_id: Some(org_id),
        action: "CREATE_DEPARTMENT", detail: &format!("创建部门: {}", name), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(dept_json(&row)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeptUpdate {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    sort_order: Option<i32>,
    #[serde(default)]
    description: Option<String>,
}

/// PUT /api/departments/{id}
async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<DeptUpdate>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:department").check(&state, &user).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_dept_in_scope(&state, &user, id).await?;
    if let Some(name) = &body.name {
        if name.trim().is_empty() {
            return Err(AppError::bad_request("部门名称不能为空"));
        }
    }
    let row: DeptRowTuple = sqlx::query_as(
        "UPDATE departments SET name = COALESCE($2, name), code = COALESCE($3, code), \
         sort_order = COALESCE($4, sort_order), description = COALESCE($5, description), updated_at = now() \
         WHERE id = $1 \
         RETURNING id, organization_id, parent_id, name, COALESCE(code,'') AS code, sort_order, \
         COALESCE(description,'') AS description, status",
    )
    .bind(id)
    .bind(&body.name)
    .bind(&body.code)
    .bind(body.sort_order)
    .bind(&body.description)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found("部门不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(
            sqlx::query_scalar("SELECT tenant_id FROM organizations WHERE id = $1")
                .bind(row.1).fetch_one(&state.pool).await?
        ), organization_id: Some(row.1),
        action: "UPDATE_DEPARTMENT", detail: &format!("更新部门: {}", row.3), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(dept_json(&row)))
}

/// DELETE /api/departments/{id}
async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:department").check(&state, &user).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_dept_in_scope(&state, &user, id).await?;
    let (name, org_id): (String, Uuid) = sqlx::query_as("DELETE FROM departments WHERE id = $1 RETURNING name, organization_id")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found("部门不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(
            sqlx::query_scalar("SELECT tenant_id FROM organizations WHERE id = $1")
                .bind(org_id).fetch_one(&state.pool).await?
        ), organization_id: Some(org_id),
        action: "DELETE_DEPARTMENT", detail: &format!("删除部门: {}", name), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(json!({"ok": true})))
}
