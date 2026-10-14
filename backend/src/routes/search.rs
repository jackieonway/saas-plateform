use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::CurrentUser;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/search", get(search))
}

/// 分区权限检查: 无权限返回 false (跳过查询, 该分区返回空数组), 其余错误向上传播
async fn has_partition_perm(
    state: &AppState,
    user: &CurrentUser,
    perm: &'static str,
) -> Result<bool, AppError> {
    match state.require(user, perm).await {
        Ok(()) => Ok(true),
        Err(AppError::Forbidden(_)) => Ok(false),
        Err(e) => Err(e),
    }
}

/// GET /api/search?q=<关键词>&limit=<每类上限 1..10, 默认 5> — 全局实体搜索 (命令面板)
/// 四个分区按权限分级: users 需 system:user / tenants 需 system:tenant /
/// organizations 需 system:organization / roles 需 system:role; 无权限分区返回空数组
async fn search(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    let q = params.get("q").cloned().unwrap_or_default().trim().to_string();
    if q.is_empty() {
        return Err(AppError::bad_request("搜索关键词不能为空"));
    }
    let limit: i64 = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .filter(|v| (1..=10).contains(v))
        .unwrap_or(5);
    // 平台根以外的上下文只查询当前租户，组织实体另受当前组织子树限制。
    let scope: Option<Uuid> = if state.is_platform_context(&user).await? {
        None
    } else {
        Some(user.tenant_id)
    };
    let org_scope = state.scoped_org_ids(&user).await?;

    // users: username/real_name ILIKE, organizationName 取第一个 membership 的组织名
    let mut users: Vec<Value> = Vec::new();
    if has_partition_perm(&state, &user, "system:user").await? {
        let rows: Vec<(Uuid, String, String, Option<String>, String)> = sqlx::query_as(
            "SELECT u.id, u.username, COALESCE(u.real_name,'') AS real_name, \
             (SELECT o.name FROM memberships m JOIN organizations o ON o.id = m.organization_id \
              WHERE m.user_id = u.id AND ($3::uuid IS NULL OR o.tenant_id = $3) \
              AND ($4::uuid[] IS NULL OR o.id = ANY($4)) \
              ORDER BY m.created_at, m.id LIMIT 1) AS organization_name, \
             u.status \
             FROM users u \
             WHERE (u.username ILIKE '%' || $1 || '%' OR u.real_name ILIKE '%' || $1 || '%') \
             AND ($3::uuid IS NULL OR u.id IN (SELECT m.user_id FROM memberships m \
                  JOIN organizations o ON o.id = m.organization_id WHERE o.tenant_id = $3)) \
             AND ($4::uuid[] IS NULL OR EXISTS (SELECT 1 FROM memberships m \
                  WHERE m.user_id = u.id AND m.organization_id = ANY($4))) \
             ORDER BY u.created_at, u.id LIMIT $2",
        )
        .bind(&q)
        .bind(limit)
        .bind(scope)
        .bind(&org_scope)
        .fetch_all(&state.pool)
        .await?;
        users = rows
            .into_iter()
            .map(|(id, username, real_name, organization_name, status)| {
                json!({
                    "id": id, "username": username, "realName": real_name,
                    "organizationName": organization_name, "status": status,
                })
            })
            .collect();
    }

    // tenants: name/code ILIKE (非平台仅能搜到本租户)
    let mut tenants: Vec<Value> = Vec::new();
    if has_partition_perm(&state, &user, "system:tenant").await? {
        let rows: Vec<(Uuid, String, String, String)> = sqlx::query_as(
            "SELECT id, name, COALESCE(code,'') AS code, status FROM tenants \
             WHERE (name ILIKE '%' || $1 || '%' OR code ILIKE '%' || $1 || '%') \
             AND ($3::uuid IS NULL OR id = $3) \
             ORDER BY created_at, id LIMIT $2",
        )
        .bind(&q)
        .bind(limit)
        .bind(scope)
        .fetch_all(&state.pool)
        .await?;
        tenants = rows
            .into_iter()
            .map(|(id, name, code, status)| {
                json!({"id": id, "name": name, "code": code, "status": status})
            })
            .collect();
    }

    // Paths begin at the selected organization, so child contexts expose no ancestor labels.
    let mut organizations: Vec<Value> = Vec::new();
    if has_partition_perm(&state, &user, "system:organization").await? {
        let rows: Vec<(Uuid, String, String, String)> = sqlx::query_as(
            "WITH RECURSIVE scoped_paths AS ( \
               SELECT o.id, o.name::text AS path FROM organizations o \
               WHERE ($3::uuid IS NULL OR o.tenant_id = $3) \
                 AND (($4::uuid[] IS NULL AND o.parent_id IS NULL) OR ($4::uuid[] IS NOT NULL AND o.id = $5)) \
               UNION ALL \
               SELECT o.id, p.path || ' / ' || o.name FROM organizations o \
               JOIN scoped_paths p ON o.parent_id = p.id \
               WHERE ($3::uuid IS NULL OR o.tenant_id = $3) AND ($4::uuid[] IS NULL OR o.id = ANY($4)) \
             ) SELECT o.id, o.name, COALESCE(o.code,'') AS code, p.path FROM organizations o \
             JOIN scoped_paths p ON p.id = o.id \
             WHERE (o.name ILIKE '%' || $1 || '%' OR o.code ILIKE '%' || $1 || '%') \
             AND ($3::uuid IS NULL OR o.tenant_id = $3) AND ($4::uuid[] IS NULL OR o.id = ANY($4)) \
             ORDER BY o.created_at, o.id LIMIT $2",
        )
        .bind(&q)
        .bind(limit)
        .bind(scope)
        .bind(&org_scope)
        .bind(user.org_id)
        .fetch_all(&state.pool)
        .await?;
        organizations = rows
            .into_iter()
            .map(|(id, name, code, path)| {
                json!({"id": id, "name": name, "code": code, "path": path})
            })
            .collect();
    }

    // roles: name/code ILIKE, organizationName 为角色所属组织名
    let mut roles: Vec<Value> = Vec::new();
    if has_partition_perm(&state, &user, "system:role").await? {
        let rows: Vec<(Uuid, String, String, Option<String>)> = sqlx::query_as(
            "SELECT r.id, r.name, COALESCE(r.code,'') AS code, o.name AS organization_name \
             FROM roles r LEFT JOIN organizations o ON o.id = r.organization_id \
             WHERE (r.name ILIKE '%' || $1 || '%' OR r.code ILIKE '%' || $1 || '%') \
             AND ($3::uuid IS NULL OR r.tenant_id = $3) \
             AND ($4::uuid[] IS NULL OR r.organization_id = ANY($4)) \
             ORDER BY r.created_at, r.id LIMIT $2",
        )
        .bind(&q)
        .bind(limit)
        .bind(scope)
        .bind(&org_scope)
        .fetch_all(&state.pool)
        .await?;
        roles = rows
            .into_iter()
            .map(|(id, name, code, organization_name)| {
                json!({"id": id, "name": name, "code": code, "organizationName": organization_name})
            })
            .collect();
    }

    Ok(Json(json!({
        "users": users,
        "tenants": tenants,
        "organizations": organizations,
        "roles": roles,
    })))
}
