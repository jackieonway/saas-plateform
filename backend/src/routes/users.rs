use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::{ApiJson, CurrentUser, RequirePermission};
use crate::routes::notifications::notify;
use crate::routes::{page_json, paging};
use crate::state::AppState;
use crate::util::{audit, password as pw};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/users", get(list).post(create))
        .route("/api/users/{id}", put(update).delete(delete))
        .route("/api/users/{id}/status", put(update_status))
        .route("/api/users/batch-status", post(batch_status))
        .route("/api/users/{id}/reset-password", post(reset_password))
        .route("/api/users/{id}/memberships", get(get_memberships).put(put_memberships))
        .route("/api/users/{id}/roles", get(get_roles).put(put_roles))
}

#[derive(sqlx::FromRow, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UserListRow {
    id: Uuid,
    username: String,
    real_name: String,
    email: String,
    phone: String,
    gender: String,
    status: String,
    last_login_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    role_count: i64,
}

/// 目标用户是否属于指定租户 (经 memberships → organizations 判定);
/// 查询失败时返回 false (fail-closed, 防隔离层异常时漏防)
async fn user_belongs_to_tenant(pool: &sqlx::PgPool, target: Uuid, tenant: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM memberships m JOIN organizations o ON o.id = m.organization_id \
         WHERE m.user_id = $1 AND o.tenant_id = $2)",
    )
    .bind(target)
    .bind(tenant)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or(false)
}

/// 非平台上下文的目标用户隔离校验: 目标必须属于当前租户 (不存在交由后续 not_found)
async fn ensure_user_in_scope(state: &AppState, user: &CurrentUser, target: Uuid) -> Result<(), AppError> {
    if state.is_platform_context(user).await? {
        return Ok(());
    }
    if user_belongs_to_tenant(&state.pool, target, user.tenant_id).await {
        Ok(())
    } else {
        Err(AppError::forbidden("目标用户不属于当前租户"))
    }
}

/// GET /api/users — 分页 + keyword/organizationId/tenantId 过滤
/// 非平台租户强制仅看本租户用户 (忽略传入 tenantId, 防跨租户枚举)
async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user").check(&state, &user).await?;
    let (page, size, offset) = paging(&params);
    let kw = params.get("keyword").cloned().unwrap_or_default();
    let org_filter: Option<Uuid> = params
        .get("organizationId")
        .and_then(|v| Uuid::parse_str(v).ok());
    let tenant_filter: Option<Uuid> = if state.is_platform_context(&user).await? {
        params.get("tenantId").and_then(|v| Uuid::parse_str(v).ok())
    } else {
        Some(user.tenant_id)
    };

    let where_clause = "($1 = '' OR u.username ILIKE '%' || $1 || '%' OR u.real_name ILIKE '%' || $1 || '%' \
     OR u.email ILIKE '%' || $1 || '%' OR u.phone ILIKE '%' || $1 || '%') \
     AND ($2::uuid IS NULL OR u.id IN (SELECT user_id FROM memberships WHERE organization_id = $2)) \
     AND ($3::uuid IS NULL OR u.id IN (SELECT m.user_id FROM memberships m JOIN organizations o ON o.id = m.organization_id WHERE o.tenant_id = $3))";

    let rows: Vec<UserListRow> = sqlx::query_as(&format!(
        "SELECT u.id, u.username, COALESCE(u.real_name,'') AS real_name, COALESCE(u.email,'') AS email, \
         COALESCE(u.phone,'') AS phone, COALESCE(u.gender,'UNKNOWN') AS gender, u.status, u.last_login_at, u.created_at, \
         (SELECT count(*) FROM user_roles ur WHERE ur.user_id = u.id) AS role_count \
         FROM users u WHERE {where_clause} ORDER BY u.created_at, u.id LIMIT $4 OFFSET $5"
    ))
    .bind(&kw)
    .bind(org_filter)
    .bind(tenant_filter)
    .bind(size)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;
    let total: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM users u WHERE {where_clause}"))
        .bind(&kw)
        .bind(org_filter)
        .bind(tenant_filter)
        .fetch_one(&state.pool)
        .await?;

    let user_ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let mut memberships_map: HashMap<Uuid, Vec<Value>> = HashMap::new();
    if !user_ids.is_empty() {
        let mrows: Vec<(Uuid, Uuid, String, Option<Uuid>, Option<String>)> = sqlx::query_as(
            "SELECT m.user_id, m.organization_id, o.name, m.department_id, d.name \
             FROM memberships m JOIN organizations o ON o.id = m.organization_id \
             LEFT JOIN departments d ON d.id = m.department_id WHERE m.user_id = ANY($1)",
        )
        .bind(&user_ids)
        .fetch_all(&state.pool)
        .await?;
        for (uid, org_id, org_name, dept_id, dept_name) in mrows {
            memberships_map.entry(uid).or_default().push(json!({
                "organizationId": org_id, "organizationName": org_name,
                "departmentId": dept_id, "departmentName": dept_name,
            }));
        }
    }
    let records: Vec<Value> = rows
        .iter()
        .map(|r| {
            let mut v = serde_json::to_value(r).unwrap_or_default();
            v["memberships"] = json!(memberships_map.get(&r.id).cloned().unwrap_or_default());
            v
        })
        .collect();
    Ok(Json(page_json(records, total, page, size)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserBody {
    username: String,
    password: String,
    #[serde(default)]
    real_name: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    phone: String,
    #[serde(default = "default_gender")]
    gender: String,
}

fn default_gender() -> String {
    "UNKNOWN".into()
}

fn parse_gender(g: &str) -> Result<String, AppError> {
    match g {
        "MALE" | "FEMALE" | "UNKNOWN" => Ok(g.to_string()),
        _ => Err(AppError::bad_request("性别仅支持 MALE / FEMALE / UNKNOWN")),
    }
}

/// POST /api/users — 创建后默认无 membership
async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<UserBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user").check(&state, &user).await?;
    let username = body.username.trim().to_string();
    if username.is_empty() || body.password.len() < 6 {
        return Err(AppError::bad_request("用户名不能为空且密码长度不能少于 6 位"));
    }
    let gender = parse_gender(&body.gender)?;
    let hash = pw::hash_password(&body.password)?;
    let row: (Uuid, String) = sqlx::query_as(
        "INSERT INTO users (username, password_hash, real_name, email, phone, gender) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id, username",
    )
    .bind(&username)
    .bind(&hash)
    .bind(&body.real_name)
    .bind(&body.email)
    .bind(&body.phone)
    .bind(&gender)
    .fetch_one(&state.pool)
    .await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "CREATE_USER", detail: &format!("创建用户: {}", username), ip: &ip, user_agent: &ua,
    }).await;
    notify(
        &state.pool, row.0, "INFO",
        "欢迎加入 Dante Cloud",
        &format!("管理员为您创建了账号「{}」。首次登录后建议前往个人中心修改初始密码。", username),
        "/profile",
    ).await;
    Ok(Json(json!({"id": row.0, "username": row.1})))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserUpdate {
    #[serde(default)]
    real_name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    phone: Option<String>,
    #[serde(default)]
    gender: Option<String>,
}

/// PUT /api/users/{id}
async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UserUpdate>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user").check(&state, &user).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_user_in_scope(&state, &user, id).await?;
    if let Some(g) = &body.gender {
        parse_gender(g)?;
    }
    let (username,): (String,) = sqlx::query_as(
        "UPDATE users SET real_name = COALESCE($2, real_name), email = COALESCE($3, email), \
         phone = COALESCE($4, phone), gender = COALESCE($5, gender), updated_at = now() \
         WHERE id = $1 RETURNING username",
    )
    .bind(id)
    .bind(&body.real_name)
    .bind(&body.email)
    .bind(&body.phone)
    .bind(&body.gender)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found("用户不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "UPDATE_USER", detail: &format!("更新用户资料: {}", username), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(json!({"ok": true})))
}

/// DELETE /api/users/{id}
async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user").check(&state, &user).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_user_in_scope(&state, &user, id).await?;
    if id == user.user_id {
        return Err(AppError::bad_request("不能删除当前登录用户"));
    }
    let (username,): (String,) = sqlx::query_as("DELETE FROM users WHERE id = $1 RETURNING username")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found("用户不存在"))?;
    state.invalidate_perm_cache();
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "DELETE_USER", detail: &format!("删除用户: {}", username), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct StatusBody {
    status: String,
}

/// PUT /api/users/{id}/status
async fn update_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<StatusBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user").check(&state, &user).await?;
    if !["ENABLED", "DISABLED"].contains(&body.status.as_str()) {
        return Err(AppError::bad_request("状态仅支持 ENABLED / DISABLED"));
    }
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_user_in_scope(&state, &user, id).await?;
    if id == user.user_id && body.status == "DISABLED" {
        return Err(AppError::bad_request("不能禁用当前登录用户"));
    }
    let (username,): (String,) = sqlx::query_as(
        "UPDATE users SET status = $2, updated_at = now() WHERE id = $1 RETURNING username",
    )
    .bind(id)
    .bind(&body.status)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found("用户不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "UPDATE_USER", detail: &format!("用户状态变更为 {}: {}", body.status, username), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchStatusBody {
    user_ids: Vec<Uuid>,
    status: String,
}

/// POST /api/users/batch-status — 批量启用/禁用
///
/// 与单条 PUT /{id}/status 行为一致(相同权限点 + 相同 UPDATE 语句), 差异:
/// - 服务端自我防护: 当前登录用户自身 id 无条件跳过(计入 skipped, 不计成败)
/// - 重复 id 去重(保序)后只执行一次
/// - 不存在的 id 计入 failed, 成功的计入 succeeded, 经响应逐类传达
/// - 上限 100 个 id, 超出 BAD_REQUEST(防误操作)
///
/// 事务性: 逐条独立 UPDATE, 不包整体事务 —— 单行 UPDATE 自身原子, 部分成功/部分失败
/// 是可接受的(语义由响应 succeeded/failed/skipped 传达), 且避免单行异常回滚拖垮整批;
/// 仅数据库级错误(连接等)才中断后续行并向上返回 500。
async fn batch_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<BatchStatusBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user").check(&state, &user).await?;
    if !["ENABLED", "DISABLED"].contains(&body.status.as_str()) {
        return Err(AppError::bad_request("状态仅支持 ENABLED / DISABLED"));
    }
    if body.user_ids.is_empty() || body.user_ids.len() > 100 {
        return Err(AppError::bad_request("userIds 数量必须在 1 到 100 之间"));
    }
    // 去重(保序) + 过滤自身(防止把自己禁用/启用, 计入 skipped)
    // 跨租户目标(非平台上下文)直接计入 failed, 不执行不中断整批
    let platform = state.is_platform_context(&user).await?;
    let mut seen = std::collections::HashSet::new();
    let (mut targets, mut skipped, mut failed): (Vec<Uuid>, Vec<Uuid>, Vec<Uuid>) = (Vec::new(), Vec::new(), Vec::new());
    for id in body.user_ids {
        if !seen.insert(id) {
            continue;
        }
        if id == user.user_id {
            skipped.push(id);
        } else if !platform && !user_belongs_to_tenant(&state.pool, id, user.tenant_id).await {
            failed.push(id);
        } else {
            targets.push(id);
        }
    }
    let mut succeeded: Vec<Uuid> = Vec::new();
    for id in &targets {
        let n = sqlx::query("UPDATE users SET status = $2, updated_at = now() WHERE id = $1")
            .bind(id)
            .bind(&body.status)
            .execute(&state.pool)
            .await?
            .rows_affected();
        if n > 0 {
            succeeded.push(*id);
        } else {
            failed.push(*id);
        }
    }
    if !succeeded.is_empty() {
        let verb = if body.status == "ENABLED" { "启用" } else { "禁用" };
        let ip = audit::client_ip(&headers);
        let ua = audit::ua_of(&headers);
        audit::write(&state.pool, audit::AuditInfo {
            user_id: Some(user.user_id), username: &user.username,
            tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
            action: "UPDATE_USER", detail: &format!("批量{} {} 个用户", verb, succeeded.len()), ip: &ip, user_agent: &ua,
        }).await;
    }
    Ok(Json(json!({ "succeeded": succeeded, "failed": failed, "skipped": skipped })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResetPasswordBody {
    new_password: String,
}

/// POST /api/users/{id}/reset-password
async fn reset_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ResetPasswordBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user").check(&state, &user).await?;
    if body.new_password.len() < 6 {
        return Err(AppError::bad_request("新密码长度不能少于 6 位"));
    }
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_user_in_scope(&state, &user, id).await?;
    let hash = pw::hash_password(&body.new_password)?;
    let (username,): (String,) =
        sqlx::query_as("UPDATE users SET password_hash = $2, updated_at = now() WHERE id = $1 RETURNING username")
            .bind(id)
            .bind(&hash)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| AppError::not_found("用户不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "RESET_PASSWORD", detail: &format!("重置用户密码: {}", username), ip: &ip, user_agent: &ua,
    }).await;
    notify(
        &state.pool, id, "SECURITY",
        "密码已被管理员重置",
        &format!("管理员 {} 刚刚重置了您的登录密码。若非您本人申请,请立即联系平台管理员。", user.username),
        "/profile",
    ).await;
    Ok(Json(json!({"ok": true})))
}

type MembershipTuple = (Uuid, Uuid, Option<Uuid>, String, String);

/// GET /api/users/{id}/memberships
async fn get_memberships(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user").check(&state, &user).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_user_in_scope(&state, &user, id).await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1)")
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    if !exists {
        return Err(AppError::not_found("用户不存在"));
    }
    let rows: Vec<MembershipTuple> = sqlx::query_as(
        "SELECT m.organization_id, o.tenant_id, m.department_id, m.position, o.name \
         FROM memberships m JOIN organizations o ON o.id = m.organization_id \
         WHERE m.user_id = $1 ORDER BY m.created_at",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let dept_ids: Vec<Uuid> = rows.iter().filter_map(|r| r.2).collect();
    let mut dept_names: std::collections::HashMap<Uuid, String> = std::collections::HashMap::new();
    if !dept_ids.is_empty() {
        let named: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT id, name FROM departments WHERE id = ANY($1)",
        )
        .bind(&dept_ids)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();
        for (did, name) in named {
            dept_names.insert(did, name);
        }
    }
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(organization_id, tenant_id, department_id, position, org_name)| {
            json!({
                "organizationId": organization_id, "tenantId": tenant_id,
                "departmentId": department_id, "position": position, "organizationName": org_name,
                "departmentName": department_id.and_then(|d| dept_names.get(&d).cloned()),
            })
        })
        .collect();
    Ok(Json(Value::Array(items)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MembershipsBody {
    #[serde(default)]
    items: Vec<MembershipItem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MembershipItem {
    organization_id: String,
    #[serde(default)]
    department_id: Option<String>,
    #[serde(default)]
    position: String,
}

/// PUT /api/users/{id}/memberships — 全量替换
async fn put_memberships(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<MembershipsBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user").check(&state, &user).await?;
    let uid: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_user_in_scope(&state, &user, uid).await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1)")
        .bind(uid)
        .fetch_one(&state.pool)
        .await?;
    if !exists {
        return Err(AppError::not_found("用户不存在"));
    }
    let mut org_ids: Vec<Uuid> = Vec::new();
    let mut dept_ids: Vec<Option<Uuid>> = Vec::new();
    let mut positions: Vec<String> = Vec::new();
    for item in &body.items {
        let org: Uuid = item
            .organization_id
            .parse()
            .map_err(|_| AppError::bad_request("organizationId 格式错误"))?;
        org_ids.push(org);
        let dept: Option<Uuid> = match &item.department_id {
            Some(d) if !d.trim().is_empty() => Some(
                d.parse()
                    .map_err(|_| AppError::bad_request("departmentId 格式错误"))?,
            ),
            _ => None,
        };
        dept_ids.push(dept);
        positions.push(item.position.clone());
    }
    if !org_ids.is_empty() {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM organizations WHERE id = ANY($1)")
            .bind(&org_ids)
            .fetch_one(&state.pool)
            .await?;
        if n != org_ids.len() as i64 {
            return Err(AppError::bad_request("存在无效的组织"));
        }
        // 非平台上下文: 成员组织必须全部属于当前租户 (防把用户迁入其他租户)
        if !state.is_platform_context(&user).await? {
            let n: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM organizations WHERE id = ANY($1) AND tenant_id = $2",
            )
            .bind(&org_ids)
            .bind(user.tenant_id)
            .fetch_one(&state.pool)
            .await?;
            if n != org_ids.len() as i64 {
                return Err(AppError::forbidden("组织不属于当前租户"));
            }
        }
    }
    let flat_depts: Vec<Uuid> = dept_ids.iter().filter_map(|d| *d).collect();
    if !flat_depts.is_empty() {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM departments WHERE id = ANY($1)")
            .bind(&flat_depts)
            .fetch_one(&state.pool)
            .await?;
        if n != flat_depts.len() as i64 {
            return Err(AppError::bad_request("存在无效的部门"));
        }
    }
    // 去重 (同组织只保留第一条)
    let mut seen = std::collections::HashSet::new();
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM memberships WHERE user_id = $1")
        .bind(uid)
        .execute(&mut *tx)
        .await?;
    for (i, org) in org_ids.iter().enumerate() {
        if !seen.insert(*org) {
            continue;
        }
        sqlx::query(
            "INSERT INTO memberships (user_id, organization_id, department_id, position) VALUES ($1, $2, $3, $4)",
        )
        .bind(uid)
        .bind(org)
        .bind(dept_ids[i])
        .bind(&positions[i])
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "UPDATE_USER", detail: &format!("更新用户成员关系: {} 项", body.items.len()), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(json!({"ok": true})))
}

type UserRoleTuple = (Uuid, Uuid);

/// GET /api/users/{id}/roles
async fn get_roles(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user").check(&state, &user).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_user_in_scope(&state, &user, id).await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1)")
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    if !exists {
        return Err(AppError::not_found("用户不存在"));
    }
    let rows: Vec<UserRoleTuple> = sqlx::query_as(
        "SELECT ur.role_id, ur.organization_id FROM user_roles ur WHERE ur.user_id = $1",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    // 附带角色/组织名称(用户详情抽屉使用; 旧字段保持不变)
    let named: Vec<(Uuid, Uuid, String, String, String)> = sqlx::query_as(
        "SELECT ur.role_id, ur.organization_id, COALESCE(r.name,''), COALESCE(r.code,''), COALESCE(o.name,'') \
         FROM user_roles ur \
         LEFT JOIN roles r ON r.id = ur.role_id \
         LEFT JOIN organizations o ON o.id = ur.organization_id \
         WHERE ur.user_id = $1 ORDER BY r.name",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    let name_map: std::collections::HashMap<Uuid, (String, String, String)> = named
        .into_iter()
        .map(|(rid, _, rname, rcode, oname)| (rid, (rname, rcode, oname)))
        .collect();
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(role_id, organization_id)| {
            let extra = name_map.get(&role_id);
            json!({
                "roleId": role_id, "organizationId": organization_id,
                "roleName": extra.map(|e| e.0.clone()).unwrap_or_default(),
                "roleCode": extra.map(|e| e.1.clone()).unwrap_or_default(),
                "organizationName": extra.map(|e| e.2.clone()).unwrap_or_default(),
            })
        })
        .collect();
    Ok(Json(Value::Array(items)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RolesBody {
    #[serde(default)]
    items: Vec<RoleItem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoleItem {
    role_id: String,
    #[serde(default)]
    organization_id: String,
}

/// PUT /api/users/{id}/roles — 角色所属组织需在用户 membership 组织(或其祖先)内
async fn put_roles(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<RolesBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user").check(&state, &user).await?;
    let uid: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_user_in_scope(&state, &user, uid).await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1)")
        .bind(uid)
        .fetch_one(&state.pool)
        .await?;
    if !exists {
        return Err(AppError::not_found("用户不存在"));
    }
    let mut parsed: Vec<(Uuid, Uuid)> = Vec::new();
    for item in &body.items {
        let role: Uuid = item
            .role_id
            .parse()
            .map_err(|_| AppError::bad_request("roleId 格式错误"))?;
        let org: Uuid = if item.organization_id.trim().is_empty() {
            let (role_org,): (Uuid,) =
                sqlx::query_as("SELECT organization_id FROM roles WHERE id = $1")
                    .bind(role)
                    .fetch_optional(&state.pool)
                    .await?
                    .ok_or_else(|| AppError::bad_request("角色不存在"))?;
            role_org
        } else {
            item.organization_id
                .parse()
                .map_err(|_| AppError::bad_request("organizationId 格式错误"))?
        };
        parsed.push((role, org));
    }
    if !parsed.is_empty() {
        let role_ids: Vec<Uuid> = parsed.iter().map(|(r, _)| *r).collect();
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM roles WHERE id = ANY($1)")
            .bind(&role_ids)
            .fetch_one(&state.pool)
            .await?;
        if n != role_ids.len() as i64 {
            return Err(AppError::bad_request("存在无效的角色"));
        }
        // 校验角色组织 ∈ (membership 组织 ∪ 其祖先)
        for (role, org) in &parsed {
            let (role_org,): (Uuid,) =
                sqlx::query_as("SELECT organization_id FROM roles WHERE id = $1")
                    .bind(role)
                    .fetch_one(&state.pool)
                    .await?;
            if role_org != *org {
                return Err(AppError::bad_request("角色授权上下文组织与角色所属组织不一致"));
            }
            let valid: bool = sqlx::query_scalar(
                "WITH RECURSIVE anc AS ( \
                 SELECT m.organization_id AS id, om.parent_id FROM memberships m \
                 JOIN organizations om ON om.id = m.organization_id WHERE m.user_id = $1 \
                 UNION ALL SELECT o.id, o.parent_id FROM organizations o JOIN anc ON o.id = anc.parent_id ) \
                 SELECT EXISTS(SELECT 1 FROM anc WHERE id = $2)",
            )
            .bind(uid)
            .bind(org)
            .fetch_one(&state.pool)
            .await?;
            if !valid {
                return Err(AppError::bad_request(
                    "角色所属组织必须在用户成员组织或其祖先范围内",
                ));
            }
        }
    }
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM user_roles WHERE user_id = $1")
        .bind(uid)
        .execute(&mut *tx)
        .await?;
    for (role, org) in &parsed {
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_id, organization_id) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(uid)
        .bind(role)
        .bind(org)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    state.invalidate_perm_cache();
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "UPDATE_USER", detail: &format!("更新用户角色分配: {} 项", body.items.len()), ip: &ip, user_agent: &ua,
    }).await;
    notify(
        &state.pool, uid, "INFO",
        "您的角色分配已更新",
        &format!("管理员 {} 更新了您的角色分配(共 {} 项)。新角色将在下次进入相应组织上下文时生效。", user.username, parsed.len()),
        "/profile",
    ).await;
    Ok(Json(json!({"ok": true})))
}
