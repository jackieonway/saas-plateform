use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::{ApiJson, CurrentUser, RequirePermission};
use crate::notifications::system_in_tx;
use crate::routes::{page_json, paging};
use crate::session;
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
        Err(AppError::not_found("用户不存在"))
    }
}

/// 锁住全局用户行并在同一事务内校验成员归属，避免成员关系并发变更与账号修改交错。
async fn lock_global_user_mutation(
    state: &AppState,
    user: &CurrentUser,
    target: Uuid,
    tx: &mut Transaction<'_, Postgres>,
) -> Result<Vec<Uuid>, AppError> {
    let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE id = $1 FOR UPDATE")
    .bind(target)
    .fetch_optional(&mut **tx)
    .await?;
    if exists.is_none() {
        return Err(AppError::not_found("用户不存在"));
    }
    let tenants: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT o.tenant_id FROM memberships m \
         JOIN organizations o ON o.id = m.organization_id WHERE m.user_id = $1",
    )
    .bind(target)
    .fetch_all(&mut **tx)
    .await?;
    if !state.is_platform_context(user).await? {
        if !tenants.contains(&user.tenant_id) {
            return Err(AppError::not_found("用户不存在"));
        }
        if tenants.len() > 1 {
            return Err(AppError::forbidden("此账号的全局资料仅平台可修改"));
        }
    }
    Ok(tenants)
}

async fn user_tenant_ids(pool: &sqlx::PgPool, target: Uuid) -> Result<Vec<Uuid>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT DISTINCT o.tenant_id FROM memberships m \
         JOIN organizations o ON o.id = m.organization_id WHERE m.user_id = $1",
    )
    .bind(target)
    .fetch_all(pool)
    .await?)
}

async fn user_role_tenant_ids_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    target: Uuid,
) -> Result<Vec<Uuid>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT DISTINCT r.tenant_id FROM user_roles ur \
         JOIN roles r ON r.id = ur.role_id WHERE ur.user_id = $1",
    )
    .bind(target)
    .fetch_all(&mut **tx)
    .await?)
}

/// 全局账号事件只投递到明确受影响租户中的有效直接成员上下文。
/// 没有有效成员关系时返回空集合，保留业务操作与审计而不猜测接收组织。
async fn system_recipient_contexts(
    tx: &mut Transaction<'_, Postgres>,
    target: Uuid,
    affected_tenants: &[Uuid],
) -> Result<Vec<(Uuid, Uuid, Uuid)>, AppError> {
    Ok(sqlx::query_as(
        "SELECT m.user_id, o.tenant_id, o.id \
         FROM memberships m \
         JOIN users u ON u.id = m.user_id \
         JOIN organizations o ON o.id = m.organization_id \
         JOIN tenants t ON t.id = o.tenant_id \
         WHERE m.user_id = $1 AND o.tenant_id = ANY($2) \
           AND u.status = 'ENABLED' AND t.status = 'ENABLED' \
           AND (t.expires_at IS NULL OR t.expires_at > now()) \
           AND org_chain_active(o.id) \
         ORDER BY o.tenant_id, o.id",
    )
    .bind(target)
    .bind(affected_tenants)
    .fetch_all(&mut **tx)
    .await?)
}

fn merge_tenant_ids(mut before: Vec<Uuid>, after: Vec<Uuid>) -> Vec<Uuid> {
    before.extend(after);
    before.sort_unstable();
    before.dedup();
    before
}

async fn audit_user_change(
    state: &AppState,
    headers: &HeaderMap,
    actor: &CurrentUser,
    target_tenants: &[Uuid],
    action: &'static str,
    detail: &str,
) {
    let ip = audit::client_ip(headers);
    let ua = audit::ua_of(headers);
    if target_tenants.is_empty() {
        audit::write(&state.pool, audit::AuditInfo {
            user_id: Some(actor.user_id), username: &actor.username,
            tenant_id: None, organization_id: None,
            action, detail, ip: &ip, user_agent: &ua,
        }).await;
    } else {
        for tenant_id in target_tenants {
            audit::write(&state.pool, audit::AuditInfo {
                user_id: Some(actor.user_id), username: &actor.username,
                tenant_id: Some(*tenant_id), organization_id: None,
                action, detail, ip: &ip, user_agent: &ua,
            }).await;
        }
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
         (SELECT count(*) FROM user_roles ur JOIN roles r ON r.id = ur.role_id \
          WHERE ur.user_id = u.id AND ($3::uuid IS NULL OR r.tenant_id = $3)) AS role_count \
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
             LEFT JOIN departments d ON d.id = m.department_id \
             WHERE m.user_id = ANY($1) AND ($2::uuid IS NULL OR o.tenant_id = $2)",
        )
        .bind(&user_ids)
        .bind(tenant_filter)
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

/// POST /api/users — 租户管理员创建的账号直接加入当前组织
async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<UserBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user")
        .check(&state, &user)
        .await?;
    let username = body.username.trim().to_string();
    if username.is_empty() {
        return Err(AppError::bad_request(
            "用户名不能为空",
        ));
    }
    pw::validate_for_config(&body.password, &state.config)?;
    let gender = parse_gender(&body.gender)?;
    let hash = pw::hash_password(&body.password)?;
    let platform = state.is_platform_context(&user).await?;
    let mut tx = state.pool.begin().await?;
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
    .fetch_one(&mut *tx)
    .await?;
    if !platform {
        sqlx::query("INSERT INTO memberships (user_id, organization_id) VALUES ($1, $2)")
            .bind(row.0)
            .bind(user.org_id)
            .execute(&mut *tx)
            .await?;
        system_in_tx(
            &mut tx,
            &[(row.0, user.tenant_id, user.org_id)],
            "INFO",
            "欢迎加入 Dante Cloud",
            &format!(
                "管理员为您创建了账号「{}」。首次登录后建议前往个人中心修改初始密码。",
                username
            ),
            "/profile",
        )
        .await?;
    }
    tx.commit().await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(
        &state.pool,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(user.tenant_id),
            organization_id: Some(user.org_id),
            action: "CREATE_USER",
            detail: &format!("创建用户: {}", username),
            ip: &ip,
            user_agent: &ua,
        },
    )
    .await;
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
    if let Some(g) = &body.gender {
        parse_gender(g)?;
    }
    let mut tx = state.pool.begin().await?;
    let target_tenants = lock_global_user_mutation(&state, &user, id, &mut tx).await?;
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
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("用户不存在"))?;
    tx.commit().await?;
    audit_user_change(&state, &headers, &user, &target_tenants, "UPDATE_USER",
        &format!("更新用户资料: {}", username)).await;
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
    if id == user.user_id {
        return Err(AppError::bad_request("不能删除当前登录用户"));
    }
    let mut tx = state.pool.begin().await?;
    let target_tenants = lock_global_user_mutation(&state, &user, id, &mut tx).await?;
    let (username,): (String,) = sqlx::query_as("DELETE FROM users WHERE id = $1 RETURNING username")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::not_found("用户不存在"))?;
    tx.commit().await?;
    state.invalidate_perm_cache();
    audit_user_change(&state, &headers, &user, &target_tenants, "DELETE_USER",
        &format!("删除用户: {}", username)).await;
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
    if id == user.user_id && body.status == "DISABLED" {
        return Err(AppError::bad_request("不能禁用当前登录用户"));
    }
    let mut tx = state.pool.begin().await?;
    let target_tenants = lock_global_user_mutation(&state, &user, id, &mut tx).await?;
    let (username,): (String,) = sqlx::query_as(
        "UPDATE users SET status = $2, updated_at = now() WHERE id = $1 RETURNING username",
    )
    .bind(id)
    .bind(&body.status)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("用户不存在"))?;
    tx.commit().await?;
    audit_user_change(&state, &headers, &user, &target_tenants, "UPDATE_USER",
        &format!("用户状态变更为 {}: {}", body.status, username)).await;
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
    // 跨租户目标和共享账号直接计入 failed, 不执行不中断整批
    let mut seen = std::collections::HashSet::new();
    let (mut targets, mut skipped, mut failed): (Vec<Uuid>, Vec<Uuid>, Vec<Uuid>) = (Vec::new(), Vec::new(), Vec::new());
    for id in body.user_ids {
        if !seen.insert(id) {
            continue;
        }
        if id == user.user_id {
            skipped.push(id);
        } else {
            targets.push(id);
        }
    }
    let mut succeeded: Vec<Uuid> = Vec::new();
    for id in &targets {
        let mut tx = state.pool.begin().await?;
        let target_tenants = match lock_global_user_mutation(&state, &user, *id, &mut tx).await {
            Ok(tenants) => tenants,
            Err(AppError::NotFound(_)) | Err(AppError::Forbidden(_)) => {
                failed.push(*id);
                continue;
            }
            Err(error) => return Err(error),
        };
        let n = sqlx::query("UPDATE users SET status = $2, updated_at = now() WHERE id = $1")
            .bind(id)
            .bind(&body.status)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        tx.commit().await?;
        if n > 0 {
            succeeded.push(*id);
            audit_user_change(&state, &headers, &user, &target_tenants, "UPDATE_USER",
                &format!("批量用户状态变更为 {}: {}", body.status, id)).await;
        } else {
            failed.push(*id);
        }
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
    RequirePermission("system:user")
        .check(&state, &user)
        .await?;
    pw::validate_for_config(&body.new_password, &state.config)?;
    let id: Uuid = id
        .parse()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let hash = pw::hash_password(&body.new_password)?;
    let mut tx = state.pool.begin().await?;
    let target_tenants = lock_global_user_mutation(&state, &user, id, &mut tx).await?;
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    session::revoke_all_in_tx(&mut tx, id, Some(&hash)).await?;
    let contexts = system_recipient_contexts(&mut tx, id, &target_tenants).await?;
    system_in_tx(
        &mut tx,
        &contexts,
        "SECURITY",
        "密码已被管理员重置",
        &format!(
            "管理员 {} 刚刚重置了您的登录密码。若非您本人申请,请立即联系平台管理员。",
            user.username
        ),
        "/profile",
    )
    .await?;
    tx.commit().await?;
    audit_user_change(
        &state,
        &headers,
        &user,
        &target_tenants,
        "RESET_PASSWORD",
        &format!("重置用户密码: {}", username),
    )
    .await;
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
         WHERE m.user_id = $1 AND ($2::uuid IS NULL OR o.tenant_id = $2) ORDER BY m.created_at",
    )
    .bind(id)
    .bind(if state.is_platform_context(&user).await? { None } else { Some(user.tenant_id) })
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
    let platform = state.is_platform_context(&user).await?;
    let before_tenants = if platform { user_tenant_ids(&state.pool, uid).await? } else { vec![user.tenant_id] };
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
        let unique_count = org_ids.iter().collect::<std::collections::HashSet<_>>().len();
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM organizations WHERE id = ANY($1) \
                                        AND ($2::uuid IS NULL OR tenant_id = $2)")
            .bind(&org_ids)
            .bind(if platform { None } else { Some(user.tenant_id) })
            .fetch_one(&state.pool)
            .await?;
        if n != unique_count as i64 {
            return Err(AppError::not_found("组织不存在"));
        }
    }
    let flat_depts: Vec<Uuid> = dept_ids.iter().filter_map(|d| *d).collect();
    if !flat_depts.is_empty() {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM departments WHERE id = ANY($1)")
            .bind(&flat_depts)
            .fetch_one(&state.pool)
            .await?;
        if n != flat_depts.iter().collect::<std::collections::HashSet<_>>().len() as i64 {
            return Err(AppError::not_found("部门不存在"));
        }
    }
    for (org, dept) in org_ids.iter().zip(&dept_ids) {
        if let Some(dept) = dept {
            let valid: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM departments WHERE id = $1 AND organization_id = $2)",
            ).bind(dept).bind(org).fetch_one(&state.pool).await?;
            if !valid { return Err(AppError::not_found("部门不存在")); }
        }
    }
    // 去重 (同组织只保留第一条)
    let mut seen = std::collections::HashSet::new();
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM memberships m USING organizations o \
                 WHERE m.organization_id = o.id AND m.user_id = $1 \
                 AND ($2::uuid IS NULL OR o.tenant_id = $2)")
        .bind(uid)
        .bind(if platform { None } else { Some(user.tenant_id) })
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
    let after_tenants = if platform { user_tenant_ids(&state.pool, uid).await? } else { vec![user.tenant_id] };
    audit_user_change(&state, &headers, &user, &merge_tenant_ids(before_tenants, after_tenants),
        "UPDATE_USER", &format!("更新用户 {} 成员关系: {} 项", uid, body.items.len())).await;
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
        "SELECT ur.role_id, ur.organization_id FROM user_roles ur \
         JOIN roles r ON r.id = ur.role_id WHERE ur.user_id = $1 \
         AND ($2::uuid IS NULL OR r.tenant_id = $2)",
    )
    .bind(id)
    .bind(if state.is_platform_context(&user).await? { None } else { Some(user.tenant_id) })
    .fetch_all(&state.pool)
    .await?;
    // 附带角色/组织名称(用户详情抽屉使用; 旧字段保持不变)
    let named: Vec<(Uuid, Uuid, String, String, String)> = sqlx::query_as(
        "SELECT ur.role_id, ur.organization_id, COALESCE(r.name,''), COALESCE(r.code,''), COALESCE(o.name,'') \
         FROM user_roles ur \
         LEFT JOIN roles r ON r.id = ur.role_id \
         LEFT JOIN organizations o ON o.id = ur.organization_id \
         WHERE ur.user_id = $1 AND ($2::uuid IS NULL OR r.tenant_id = $2) ORDER BY r.name",
    )
    .bind(id)
    .bind(if state.is_platform_context(&user).await? { None } else { Some(user.tenant_id) })
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
    RequirePermission("system:user")
        .check(&state, &user)
        .await?;
    let uid: Uuid = id
        .parse()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_user_in_scope(&state, &user, uid).await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1)")
        .bind(uid)
        .fetch_one(&state.pool)
        .await?;
    if !exists {
        return Err(AppError::not_found("用户不存在"));
    }
    let platform = state.is_platform_context(&user).await?;
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
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM roles WHERE id = ANY($1) \
                                        AND ($2::uuid IS NULL OR tenant_id = $2)",
        )
        .bind(&role_ids)
        .bind(if platform { None } else { Some(user.tenant_id) })
        .fetch_one(&state.pool)
        .await?;
        if n != role_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len() as i64
        {
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
                return Err(AppError::bad_request(
                    "角色授权上下文组织与角色所属组织不一致",
                ));
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
    let locked: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE id = $1 FOR UPDATE")
        .bind(uid)
        .fetch_optional(&mut *tx)
        .await?;
    if locked.is_none() {
        return Err(AppError::not_found("用户不存在"));
    }
    let before_tenants = if platform {
        user_role_tenant_ids_in_tx(&mut tx, uid).await?
    } else {
        vec![user.tenant_id]
    };
    sqlx::query(
        "DELETE FROM user_roles ur USING roles r \
                 WHERE ur.role_id = r.id AND ur.user_id = $1 \
                 AND ($2::uuid IS NULL OR r.tenant_id = $2)",
    )
    .bind(uid)
    .bind(if platform { None } else { Some(user.tenant_id) })
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
    let after_tenants = if platform {
        user_role_tenant_ids_in_tx(&mut tx, uid).await?
    } else {
        vec![user.tenant_id]
    };
    let affected_tenants = merge_tenant_ids(before_tenants, after_tenants);
    let contexts = system_recipient_contexts(&mut tx, uid, &affected_tenants).await?;
    system_in_tx(
        &mut tx,
        &contexts,
        "INFO",
        "您的角色分配已更新",
        &format!(
            "管理员 {} 更新了您的角色分配(共 {} 项)。新角色将在下次进入相应组织上下文时生效。",
            user.username,
            parsed.len()
        ),
        "/profile",
    )
    .await?;
    tx.commit().await?;
    state.invalidate_perm_cache();
    audit_user_change(
        &state,
        &headers,
        &user,
        &affected_tenants,
        "UPDATE_USER",
        &format!("更新用户 {} 角色分配: {} 项", uid, body.items.len()),
    )
    .await;
    Ok(Json(json!({"ok": true})))
}
