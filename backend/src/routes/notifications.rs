use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{delete, get, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::{ApiJson, CurrentUser, RequirePermission};
use crate::routes::{page_json, paging};
use crate::state::AppState;
use crate::util::audit;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/notifications", get(list).post(send).delete(batch_remove))
        .route("/api/notifications/unread-count", get(unread_count))
        .route("/api/notifications/prefs", get(get_prefs).put(update_prefs))
        .route("/api/notifications/read-all", put(read_all))
        .route("/api/notifications/read-batch", put(read_batch))
        .route("/api/notifications/{id}/read", put(mark_read))
        .route("/api/notifications/{id}", delete(remove))
}

/// pg_notify payload 大小上限 (字节): 超限时截断 content 后重试, 仍超限则放弃推送仅入库
const NOTIFY_PAYLOAD_MAX_BYTES: usize = 8000;

/// 按 UTF-8 字符边界截断字符串
fn truncate_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// 构造 new_notification 通道的 JSON payload
fn notify_payload(
    user_id: Uuid,
    id: Uuid,
    ntype: &str,
    title: &str,
    content: &str,
    link: &str,
    created_at: &DateTime<Utc>,
) -> String {
    json!({
        "userId": user_id,
        "id": id,
        "type": ntype,
        "title": title,
        "content": content,
        "link": link,
        "createdAt": created_at.to_rfc3339(),
    })
    .to_string()
}

/// 向单个用户写一条通知并向 PostgreSQL new_notification 通道发实时推送事件
/// (供其他模块调用; INSERT 与 pg_notify 失败均仅记日志, 不影响主流程)
pub async fn notify(
    pool: &PgPool,
    user_id: Uuid,
    ntype: &str,
    title: &str,
    content: &str,
    link: &str,
) {
    let inserted = sqlx::query_as::<_, (Uuid, DateTime<Utc>)>(
        "INSERT INTO notifications (user_id, type, title, content, link) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id, created_at",
    )
    .bind(user_id)
    .bind(ntype)
    .bind(title)
    .bind(content)
    .bind(link)
    .fetch_optional(pool)
    .await;
    let Some((id, created_at)) = inserted.inspect_err(|e| tracing::warn!("写通知失败: {e}")).ok().flatten() else {
        return;
    };
    // 实时推送事件: pg_notify payload 上限 8000 字节, content 超限时截断 (库中仍存全文)
    let mut payload = notify_payload(user_id, id, ntype, title, content, link, &created_at);
    if payload.len() > NOTIFY_PAYLOAD_MAX_BYTES {
        let truncated = truncate_utf8(content, 6000);
        tracing::warn!("通知 payload 超过 8000 字节, content 截断后推送 (通知 id={id})");
        payload = notify_payload(user_id, id, ntype, title, &truncated, link, &created_at);
        if payload.len() > NOTIFY_PAYLOAD_MAX_BYTES {
            tracing::warn!("通知 payload 截断后仍超限, 放弃推送 (通知 id={id})");
            return;
        }
    }
    let _ = sqlx::query("SELECT pg_notify('new_notification', $1)")
        .bind(&payload)
        .execute(pool)
        .await
        .inspect_err(|e| tracing::warn!("pg_notify new_notification 失败: {e}"));
}

type NotificationRow = (Uuid, String, String, String, String, bool, Option<chrono::DateTime<chrono::Utc>>, chrono::DateTime<chrono::Utc>);

fn row_json(r: NotificationRow) -> Value {
    let (id, ntype, title, content, link, is_read, read_at, created_at) = r;
    json!({
        "id": id, "type": ntype, "title": title, "content": content,
        "link": link, "isRead": is_read, "readAt": read_at, "createdAt": created_at,
    })
}

/// GET /api/notifications — 本人通知列表
///
/// 组合过滤 (三者任意组合):
/// - filter=UNREAD 只看未读;
/// - type 只看指定类型 (白名单, 非法 400);
/// - q 全文搜索 title/content (ILIKE, trim 非空时生效)。
async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    let (page, size, offset) = paging(&params);
    let unread_only = params.get("filter").map(|v| v == "UNREAD").unwrap_or(false);
    // type 白名单校验: match 产出的仅是四个编译期字面量之一 (非法值直接 400),
    // 拼接进 where 的只是该字面量而非用户输入原文, 无注入面; user_id/分页仍走绑定参数
    let type_filter: Option<&'static str> = match params.get("type").map(String::as_str) {
        None => None,
        Some("INFO") => Some("INFO"),
        Some("SUCCESS") => Some("SUCCESS"),
        Some("WARNING") => Some("WARNING"),
        Some("SECURITY") => Some("SECURITY"),
        Some(_) => {
            return Err(AppError::bad_request("type 仅支持 INFO / SUCCESS / WARNING / SECURITY"));
        }
    };
    // q 全文搜索: 先对 q 中的 \ % _ 做转义再拼 %...%, 配合 ESCAPE '\\' 按字面量匹配
    // (q 含通配符时按字面量搜索, 不穿透为通配符); q 为空串时 $2='' 短路该条件 → 不过滤
    // (ILIKE 由迁移 0006 的 pg_trgm GIN 索引自动加速, 无需在本查询改写; 数据量小或 q<3 字符时
    //  planner 仍可能选 user_id 索引/顺序扫描, 属正常代价决策)
    let q_trimmed = params.get("q").map(|v| v.trim()).unwrap_or("");
    let q_pattern = if q_trimmed.is_empty() {
        String::new()
    } else {
        let escaped = q_trimmed
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        format!("%{escaped}%")
    };
    let mut where_clause =
        String::from("user_id = $1 AND ($2 = '' OR (title ILIKE $2 ESCAPE '\\' OR content ILIKE $2 ESCAPE '\\'))");
    if unread_only {
        where_clause.push_str(" AND is_read = false");
    }
    if let Some(t) = type_filter {
        where_clause.push_str(" AND type = '");
        where_clause.push_str(t);
        where_clause.push('\'');
    }
    // count 与列表共用同一 where 子句与绑定参数, 保证 total 一致
    let total: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM notifications WHERE {where_clause}"))
        .bind(user.user_id)
        .bind(&q_pattern)
        .fetch_one(&state.pool)
        .await?;
    let rows: Vec<NotificationRow> = sqlx::query_as(&format!(
        "SELECT id, type, title, content, link, is_read, read_at, created_at \
         FROM notifications WHERE {where_clause} ORDER BY created_at DESC, id LIMIT $3 OFFSET $4"
    ))
    .bind(user.user_id)
    .bind(&q_pattern)
    .bind(size)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;
    let records: Vec<Value> = rows.into_iter().map(row_json).collect();
    Ok(Json(page_json(records, total, page, size)))
}

/// 通知偏好默认值 (无行时 GET 返回)
const PREFS_DEFAULT: (bool, &str, &str) = (false, "22:00", "07:00");

/// 偏好响应 JSON: {"dndEnabled":bool,"dndStart":"HH:MM","dndEnd":"HH:MM","mutedTypes":[...]}
fn prefs_json(dnd_enabled: bool, dnd_start: &str, dnd_end: &str, muted_types: &[&str]) -> Value {
    json!({
        "dndEnabled": dnd_enabled, "dndStart": dnd_start, "dndEnd": dnd_end,
        "mutedTypes": muted_types,
    })
}

/// muted_types 存储格式 (逗号分隔) → 数组 (空串/空项 → 空数组)
fn split_muted_types(muted: &str) -> Vec<&str> {
    muted.split(',').map(str::trim).filter(|s| !s.is_empty()).collect()
}

/// GET /api/notifications/prefs — 本人通知偏好 (免打扰 + 静音类型), 无行返回默认值
///
/// 静态段路由, 与 /api/notifications/{id} 参数路由共存 (同 unread-count / read-all 模式)。
async fn get_prefs(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    let row: Option<(bool, String, String, String)> = sqlx::query_as(
        "SELECT dnd_enabled, dnd_start, dnd_end, muted_types FROM notification_prefs WHERE user_id = $1",
    )
    .bind(user.user_id)
    .fetch_optional(&state.pool)
    .await?;
    let (dnd_enabled, dnd_start, dnd_end, muted) = row
        .map(|(e, s, t, m)| (e, s, t, m))
        .unwrap_or_else(|| {
            (PREFS_DEFAULT.0, PREFS_DEFAULT.1.to_string(), PREFS_DEFAULT.2.to_string(), String::new())
        });
    let muted_types = split_muted_types(&muted);
    Ok(Json(prefs_json(dnd_enabled, &dnd_start, &dnd_end, &muted_types)))
}

/// PUT /api/notifications/prefs 请求体
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrefsBody {
    dnd_enabled: bool,
    dnd_start: String,
    dnd_end: String,
    muted_types: Vec<String>,
}

/// HH:MM 时间校验: 匹配 ^([01]\d|2[0-3]):[0-5]\d$ (等价手写实现, 避免引入 regex 依赖)
fn valid_hhmm(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 5
        && b[2] == b':'
        && b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && b[3].is_ascii_digit()
        && b[4].is_ascii_digit()
        && (b[0] - b'0') * 10 + (b[1] - b'0') <= 23
        && (b[3] - b'0') * 10 + (b[4] - b'0') <= 59
}

/// PUT /api/notifications/prefs — 更新本人通知偏好 (UPSERT), 校验失败 400
async fn update_prefs(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<PrefsBody>,
) -> Result<Json<Value>, AppError> {
    if !valid_hhmm(&body.dnd_start) || !valid_hhmm(&body.dnd_end) {
        return Err(AppError::bad_request("dndStart / dndEnd 必须为 HH:MM 格式 (00:00 ~ 23:59)"));
    }
    for t in &body.muted_types {
        if !valid_type(t) {
            return Err(AppError::bad_request("mutedTypes 每项仅支持 INFO / SUCCESS / WARNING / SECURITY"));
        }
    }
    let muted_joined = body.muted_types.join(",");
    sqlx::query(
        "INSERT INTO notification_prefs (user_id, dnd_enabled, dnd_start, dnd_end, muted_types) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (user_id) DO UPDATE \
         SET dnd_enabled = $2, dnd_start = $3, dnd_end = $4, muted_types = $5, updated_at = now()",
    )
    .bind(user.user_id)
    .bind(body.dnd_enabled)
    .bind(&body.dnd_start)
    .bind(&body.dnd_end)
    .bind(&muted_joined)
    .execute(&state.pool)
    .await?;
    let dnd_switch = if body.dnd_enabled { "开" } else { "关" };
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "UPDATE_NOTIFICATION_PREFS",
        detail: &format!(
            "更新通知偏好: 免打扰 {dnd_switch} ({}-{}), 静音类型 {} 项",
            body.dnd_start, body.dnd_end, body.muted_types.len()
        ),
        ip: &ip, user_agent: &ua,
    }).await;
    let muted_refs: Vec<&str> = body.muted_types.iter().map(String::as_str).collect();
    Ok(Json(prefs_json(body.dnd_enabled, &body.dnd_start, &body.dnd_end, &muted_refs)))
}

/// GET /api/notifications/unread-count
async fn unread_count(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE user_id = $1 AND is_read = false")
        .bind(user.user_id)
        .fetch_one(&state.pool)
        .await?;
    Ok(Json(json!({ "count": count })))
}

/// PUT /api/notifications/{id}/read — 标记已读
async fn mark_read(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let n = sqlx::query(
        "UPDATE notifications SET is_read = true, read_at = now() WHERE id = $1 AND user_id = $2 AND is_read = false",
    )
    .bind(id)
    .bind(user.user_id)
    .execute(&state.pool)
    .await?
    .rows_affected();
    Ok(Json(json!({ "ok": n > 0 })))
}

/// PUT /api/notifications/read-all — 全部已读
async fn read_all(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    let n = sqlx::query(
        "UPDATE notifications SET is_read = true, read_at = now() WHERE user_id = $1 AND is_read = false",
    )
    .bind(user.user_id)
    .execute(&state.pool)
    .await?
    .rows_affected();
    Ok(Json(json!({ "updated": n })))
}

/// PUT /api/notifications/read-batch — 批量标记已读 (与 read-all 路径不同, 互不冲突)
async fn read_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<BatchIdsBody>,
) -> Result<Json<Value>, AppError> {
    check_batch_ids(&body.ids)?;
    let n = sqlx::query(
        "UPDATE notifications SET is_read = true, read_at = now() \
         WHERE user_id = $1 AND id = ANY($2) AND is_read = false",
    )
    .bind(user.user_id)
    .bind(&body.ids)
    .execute(&state.pool)
    .await?
    .rows_affected();
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "BATCH_READ_NOTIFICATIONS", detail: &format!("批量标记已读: {} 条", n), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(json!({ "updated": n })))
}

/// DELETE /api/notifications/{id} — 删除本人通知
async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let nid: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let deleted: Option<String> =
        sqlx::query_scalar("DELETE FROM notifications WHERE id = $1 AND user_id = $2 RETURNING title")
            .bind(nid)
            .bind(user.user_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| AppError::not_found("通知不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "DELETE_NOTIFICATION", detail: &format!("删除通知: {}", deleted.as_deref().unwrap_or("")), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(json!({ "ok": true })))
}

/// 批量操作请求体 {"ids":["uuid",...]}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchIdsBody {
    ids: Vec<Uuid>,
}

/// 批量 ids 数量校验: 1..=100
fn check_batch_ids(ids: &[Uuid]) -> Result<(), AppError> {
    if ids.is_empty() || ids.len() > 100 {
        return Err(AppError::bad_request("ids 数量必须在 1 到 100 之间"));
    }
    Ok(())
}

/// DELETE /api/notifications — 批量删除本人通知
///
/// 与 DELETE /api/notifications/{id} (单删) 路径段数不同, 路由共存互不冲突;
/// 仅删除本人通知 (user_id 绑定参数), 不存在/非本人的 id 静默忽略。
async fn batch_remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<BatchIdsBody>,
) -> Result<Json<Value>, AppError> {
    check_batch_ids(&body.ids)?;
    let n = sqlx::query("DELETE FROM notifications WHERE user_id = $1 AND id = ANY($2)")
        .bind(user.user_id)
        .bind(&body.ids)
        .execute(&state.pool)
        .await?
        .rows_affected();
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "BATCH_DELETE_NOTIFICATIONS", detail: &format!("批量删除通知: {} 条", n), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(json!({ "deleted": n })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendBody {
    title: String,
    #[serde(default)]
    content: String,
    #[serde(default = "default_type")]
    r#type: String,
    #[serde(default)]
    link: String,
    /// 三选一: userId | orgId | tenantId (都不传 = 不可用)
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    org_id: Option<String>,
    #[serde(default)]
    tenant_id: Option<String>,
}

fn default_type() -> String {
    "INFO".to_string()
}

fn valid_type(t: &str) -> bool {
    matches!(t, "INFO" | "SUCCESS" | "WARNING" | "SECURITY")
}

/// POST /api/notifications — 管理员发送通知 (userId 单发 / orgId 组织广播 / tenantId 租户广播)
async fn send(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<SendBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user").check(&state, &user).await?;
    let title = body.title.trim();
    if title.is_empty() || title.len() > 200 {
        return Err(AppError::bad_request("标题不能为空且不超过 200 字"));
    }
    if !valid_type(&body.r#type) {
        return Err(AppError::bad_request("类型仅支持 INFO / SUCCESS / WARNING / SECURITY"));
    }
    let targets: Vec<Uuid> = if let Some(uid) = body.user_id.as_deref().filter(|v| !v.is_empty()) {
        let uid: Uuid = uid.parse().map_err(|_| AppError::bad_request("userId 格式错误"))?;
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1)")
            .bind(uid)
            .fetch_one(&state.pool)
            .await?;
        if !exists {
            return Err(AppError::not_found("目标用户不存在"));
        }
        vec![uid]
    } else if let Some(oid) = body.org_id.as_deref().filter(|v| !v.is_empty()) {
        let oid: Uuid = oid.parse().map_err(|_| AppError::bad_request("orgId 格式错误"))?;
        let rows: Vec<(Uuid,)> = sqlx::query_as(
            "SELECT DISTINCT m.user_id FROM memberships m \
             WHERE m.organization_id = $1",
        )
        .bind(oid)
        .fetch_all(&state.pool)
        .await?;
        if rows.is_empty() {
            return Err(AppError::bad_request("该组织下没有成员"));
        }
        rows.into_iter().map(|(u,)| u).collect()
    } else if let Some(tid) = body.tenant_id.as_deref().filter(|v| !v.is_empty()) {
        let tid: Uuid = tid.parse().map_err(|_| AppError::bad_request("tenantId 格式错误"))?;
        let rows: Vec<(Uuid,)> = sqlx::query_as(
            "SELECT DISTINCT m.user_id FROM memberships m \
             JOIN organizations o ON o.id = m.organization_id WHERE o.tenant_id = $1",
        )
        .bind(tid)
        .fetch_all(&state.pool)
        .await?;
        if rows.is_empty() {
            return Err(AppError::bad_request("该租户下没有成员"));
        }
        rows.into_iter().map(|(u,)| u).collect()
    } else {
        return Err(AppError::bad_request("需指定 userId / orgId / tenantId 之一"));
    };
    for uid in &targets {
        notify(&state.pool, *uid, &body.r#type, title, &body.content, &body.link).await;
    }
    let scope = if body.user_id.is_some() {
        format!("单用户 {}", targets[0])
    } else if body.org_id.is_some() {
        "组织广播".to_string()
    } else {
        "租户广播".to_string()
    };
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "SEND_NOTIFICATION", detail: &format!("发送通知({scope}): {} 人 · {}", targets.len(), title), ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(json!({ "sent": targets.len() })))
}
