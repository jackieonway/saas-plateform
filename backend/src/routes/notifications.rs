use crate::error::AppError;
use crate::extractors::{bearer_token, ApiJson, CurrentUser, RequirePermission};
use crate::notifications::{self, valid_type, PublishBody, Publisher, Target};
use crate::routes::{page_json, paging};
use crate::state::AppState;
use crate::util::{audit, jwt};
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/apps/{appCode}/notifications", post(app_publish))
        .route(
            "/api/notifications",
            get(list).post(send).delete(batch_remove),
        )
        .route("/api/notifications/legacy", get(legacy))
        .route("/api/notifications/socket-context", get(socket_context))
        .route("/api/notifications/unread-count", get(unread_count))
        .route("/api/notifications/prefs", get(get_prefs).put(update_prefs))
        .route("/api/notifications/read-all", put(read_all))
        .route("/api/notifications/read-batch", put(read_batch))
        .route("/api/notifications/{id}/read", put(mark_read))
        .route("/api/notifications/{id}", delete(remove))
        .layer(DefaultBodyLimit::max(64 * 1024))
}

// Shared by every inbox operation and per-delivery socket authorization.
const VISIBLE: &str = "n.user_id=$1 AND n.tenant_id=$2 AND n.organization_id=$3 \
    AND (n.source_kind='SYSTEM' OR (n.source_kind='APP' AND EXISTS( \
    SELECT 1 FROM applications a JOIN tenant_applications ta ON ta.application_id=a.id AND ta.tenant_id=n.tenant_id \
    JOIN organization_applications oa ON oa.application_id=a.id AND oa.organization_id=n.organization_id \
    WHERE a.id=n.application_id AND a.status='ENABLED')))";

#[derive(sqlx::FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
struct NotificationRow {
    id: Uuid,
    user_id: Uuid,
    r#type: String,
    title: String,
    content: String,
    link: String,
    is_read: bool,
    read_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    source_kind: String,
    tenant_id: Option<Uuid>,
    organization_id: Option<Uuid>,
    application_id: Option<Uuid>,
    publication_id: Option<Uuid>,
    application_name: Option<String>,
}
const COLUMNS: &str = "n.id,n.user_id,n.type,n.title,COALESCE(n.content,'') AS content,COALESCE(n.link,'') AS link, \
    n.is_read,n.read_at,n.created_at,n.source_kind,n.tenant_id,n.organization_id,n.application_id,n.publication_id, \
    p.application_name";

async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    let (page, size, offset) = paging(&params);
    let unread = params.get("filter").is_some_and(|v| v == "UNREAD");
    let type_filter = params.get("type").map(String::as_str).unwrap_or("");
    if params.contains_key("type") && !valid_type(type_filter) {
        return Err(AppError::bad_request("type 不合法"));
    }
    let q = params.get("q").map(|v| v.trim()).unwrap_or("");
    if q.contains('\0') {
        return Err(AppError::bad_request("搜索关键词不能包含空字符"));
    }
    let pattern = if q.is_empty() {
        String::new()
    } else {
        format!(
            "%{}%",
            q.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        )
    };
    let predicate = format!(
        "{VISIBLE} AND (NOT $4 OR NOT n.is_read) AND ($5='' OR n.type=$5) \
        AND ($6='' OR n.title ILIKE $6 ESCAPE '\\' OR n.content ILIKE $6 ESCAPE '\\')"
    );
    let total: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM notifications n WHERE {predicate}"
    ))
    .bind(user.user_id)
    .bind(user.tenant_id)
    .bind(user.org_id)
    .bind(unread)
    .bind(type_filter)
    .bind(&pattern)
    .fetch_one(&state.pool)
    .await?;
    let records: Vec<NotificationRow> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM notifications n LEFT JOIN notification_publications p ON p.id=n.publication_id \
         WHERE {predicate} ORDER BY n.created_at DESC,n.id LIMIT $7 OFFSET $8"))
        .bind(user.user_id).bind(user.tenant_id).bind(user.org_id).bind(unread).bind(type_filter).bind(&pattern)
        .bind(size).bind(offset).fetch_all(&state.pool).await?;
    Ok(Json(page_json(
        records.into_iter().map(|r| json!(r)).collect(),
        total,
        page,
        size,
    )))
}

async fn legacy(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    state.require_platform(&user).await?;
    RequirePermission("system:audit")
        .check(&state, &user)
        .await?;
    let (page, size, offset) = paging(&params);
    let total: i64 =
        sqlx::query_scalar("SELECT count(*) FROM notifications WHERE source_kind='LEGACY'")
            .fetch_one(&state.pool)
            .await?;
    let records: Vec<NotificationRow> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM notifications n LEFT JOIN notification_publications p ON p.id=n.publication_id \
         WHERE n.source_kind='LEGACY' ORDER BY n.created_at DESC,n.id LIMIT $1 OFFSET $2"))
        .bind(size).bind(offset).fetch_all(&state.pool).await?;
    Ok(Json(page_json(
        records.into_iter().map(|r| json!(r)).collect(),
        total,
        page,
        size,
    )))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SocketQuery {
    notification_id: Option<Uuid>,
}
async fn socket_context(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Query(query): Query<SocketQuery>,
) -> Result<Json<Value>, AppError> {
    if let Some(id) = query.notification_id {
        let visible: bool = sqlx::query_scalar(&format!(
            "SELECT EXISTS(SELECT 1 FROM notifications n WHERE {VISIBLE} AND n.id=$4)"
        ))
        .bind(user.user_id)
        .bind(user.tenant_id)
        .bind(user.org_id)
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
        if !visible {
            return Err(AppError::not_found("通知不存在"));
        }
    }
    let token = bearer_token(&headers)?;
    let claims = jwt::verify(&token, &state.config.jwt_secret)
        .map_err(|_| AppError::unauthorized("会话已失效"))?;
    let db_expiry: DateTime<Utc> =
        sqlx::query_scalar("SELECT expires_at FROM platform_sessions WHERE id=$1")
            .bind(user.session_id)
            .fetch_one(&state.pool)
            .await?;
    let jwt_expiry = DateTime::<Utc>::from_timestamp(claims.exp, 0)
        .ok_or_else(|| AppError::unauthorized("会话时效无效"))?;
    if jwt_expiry <= Utc::now() {
        return Err(AppError::unauthorized("会话凭证已过期"));
    }
    Ok(Json(
        json!({ "contractVersion": "1.0", "userId": user.user_id, "tenantId": user.tenant_id,
        "organizationId": user.org_id, "sessionId": user.session_id, "expiresAt": db_expiry.min(jwt_expiry) }),
    ))
}

async fn app_publish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(app_code): Path<String>,
    ApiJson(body): ApiJson<PublishBody>,
) -> Result<Json<Value>, AppError> {
    let token = bearer_token(&headers)?;
    let claims = jwt::verify(&token, &state.config.jwt_secret)
        .map_err(|_| AppError::unauthorized("应用凭证无效或已过期"))?;
    let info = crate::app_token::inspect(&state.pool, &token, &claims)
        .await?
        .ok_or_else(|| AppError::unauthorized("需要有效应用专用令牌"))?;
    if info.audience != app_code {
        return Err(AppError::forbidden("应用令牌 audience 与路由不一致"));
    }
    let actor = Publisher {
        user_id: info.user_id,
        tenant_id: info.tenant_id,
        organization_id: info.organization_id,
        application_id: info.application_id,
        session_id: info.session_id,
        token_hash: Some(crate::util::hash::sha256_hex(&token)),
        token_expires_at: Some(
            DateTime::<Utc>::from_timestamp(claims.exp, 0)
                .ok_or_else(|| AppError::unauthorized("应用凭证时效无效"))?
                .min(info.expires_at),
        ),
    };
    let result = notifications::publish(
        &state.pool,
        actor,
        body,
        &audit::client_ip(&headers),
        &audit::ua_of(&headers),
    )
    .await?;
    Ok(Json(json!(result)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SendBody {
    application_id: Uuid,
    idempotency_key: String,
    target: Target,
    r#type: String,
    title: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    link: String,
}
async fn send(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<SendBody>,
) -> Result<Json<Value>, AppError> {
    let claims = jwt::verify(&bearer_token(&headers)?, &state.config.jwt_secret)
        .map_err(|_| AppError::unauthorized("会话凭证已失效"))?;
    let actor = Publisher {
        user_id: user.user_id,
        tenant_id: user.tenant_id,
        organization_id: user.org_id,
        application_id: body.application_id,
        session_id: user.session_id,
        token_hash: None,
        token_expires_at: Some(
            DateTime::<Utc>::from_timestamp(claims.exp, 0)
                .ok_or_else(|| AppError::unauthorized("会话时效无效"))?,
        ),
    };
    let body = PublishBody {
        idempotency_key: body.idempotency_key,
        target: body.target,
        r#type: body.r#type,
        title: body.title,
        content: body.content,
        link: body.link,
    };
    let result = notifications::publish(
        &state.pool,
        actor,
        body,
        &audit::client_ip(&headers),
        &audit::ua_of(&headers),
    )
    .await?;
    let mut value = json!(result);
    value["sent"] = json!(result.recipient_count);
    Ok(Json(value))
}

async fn unread_count(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    let count: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM notifications n WHERE {VISIBLE} AND NOT n.is_read"
    ))
    .bind(user.user_id)
    .bind(user.tenant_id)
    .bind(user.org_id)
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(json!({ "count": count })))
}
async fn mark_read(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let n = sqlx::query(&format!("UPDATE notifications n SET is_read=true,read_at=now() WHERE {VISIBLE} AND n.id=$4 AND NOT n.is_read"))
        .bind(user.user_id).bind(user.tenant_id).bind(user.org_id).bind(id).execute(&state.pool).await?.rows_affected();
    Ok(Json(json!({ "ok": n>0 })))
}
async fn read_all(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    let n = sqlx::query(&format!(
        "UPDATE notifications n SET is_read=true,read_at=now() WHERE {VISIBLE} AND NOT n.is_read"
    ))
    .bind(user.user_id)
    .bind(user.tenant_id)
    .bind(user.org_id)
    .execute(&state.pool)
    .await?
    .rows_affected();
    Ok(Json(json!({ "updated": n })))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchIdsBody {
    ids: Vec<Uuid>,
}
fn check_batch_ids(ids: &[Uuid]) -> Result<(), AppError> {
    if ids.is_empty() || ids.len() > 100 {
        return Err(AppError::bad_request("ids 数量必须在 1 到 100 之间"));
    }
    Ok(())
}
async fn read_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<BatchIdsBody>,
) -> Result<Json<Value>, AppError> {
    check_batch_ids(&body.ids)?;
    let n = sqlx::query(&format!("UPDATE notifications n SET is_read=true,read_at=now() WHERE {VISIBLE} AND n.id=ANY($4) AND NOT n.is_read"))
        .bind(user.user_id).bind(user.tenant_id).bind(user.org_id).bind(&body.ids).execute(&state.pool).await?.rows_affected();
    mutation_audit(
        &state,
        &user,
        &headers,
        "BATCH_READ_NOTIFICATIONS",
        &format!("批量标记已读: {n} 条"),
    )
    .await;
    Ok(Json(json!({ "updated": n })))
}
async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let title: String = sqlx::query_scalar(&format!(
        "DELETE FROM notifications n WHERE {VISIBLE} AND n.id=$4 RETURNING title"
    ))
    .bind(user.user_id)
    .bind(user.tenant_id)
    .bind(user.org_id)
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found("通知不存在"))?;
    mutation_audit(
        &state,
        &user,
        &headers,
        "DELETE_NOTIFICATION",
        &format!("删除通知: {title}"),
    )
    .await;
    Ok(Json(json!({ "ok": true })))
}
async fn batch_remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<BatchIdsBody>,
) -> Result<Json<Value>, AppError> {
    check_batch_ids(&body.ids)?;
    let n = sqlx::query(&format!(
        "DELETE FROM notifications n WHERE {VISIBLE} AND n.id=ANY($4)"
    ))
    .bind(user.user_id)
    .bind(user.tenant_id)
    .bind(user.org_id)
    .bind(&body.ids)
    .execute(&state.pool)
    .await?
    .rows_affected();
    mutation_audit(
        &state,
        &user,
        &headers,
        "BATCH_DELETE_NOTIFICATIONS",
        &format!("批量删除通知: {n} 条"),
    )
    .await;
    Ok(Json(json!({ "deleted": n })))
}
async fn mutation_audit(
    state: &AppState,
    user: &CurrentUser,
    headers: &HeaderMap,
    action: &str,
    detail: &str,
) {
    audit::write(
        &state.pool,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(user.tenant_id),
            organization_id: Some(user.org_id),
            action,
            detail,
            ip: &audit::client_ip(headers),
            user_agent: &audit::ua_of(headers),
        },
    )
    .await;
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
    muted
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
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
    let (dnd_enabled, dnd_start, dnd_end, muted) = row.unwrap_or_else(|| {
        (
            PREFS_DEFAULT.0,
            PREFS_DEFAULT.1.to_string(),
            PREFS_DEFAULT.2.to_string(),
            String::new(),
        )
    });
    let muted_types = split_muted_types(&muted);
    Ok(Json(prefs_json(
        dnd_enabled,
        &dnd_start,
        &dnd_end,
        &muted_types,
    )))
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
        return Err(AppError::bad_request(
            "dndStart / dndEnd 必须为 HH:MM 格式 (00:00 ~ 23:59)",
        ));
    }
    for t in &body.muted_types {
        if !valid_type(t) {
            return Err(AppError::bad_request(
                "mutedTypes 每项仅支持 INFO / SUCCESS / WARNING / SECURITY",
            ));
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
    audit::write(
        &state.pool,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(user.tenant_id),
            organization_id: Some(user.org_id),
            action: "UPDATE_NOTIFICATION_PREFS",
            detail: &format!(
                "更新通知偏好: 免打扰 {dnd_switch} ({}-{}), 静音类型 {} 项",
                body.dnd_start,
                body.dnd_end,
                body.muted_types.len()
            ),
            ip: &ip,
            user_agent: &ua,
        },
    )
    .await;
    let muted_refs: Vec<&str> = body.muted_types.iter().map(String::as_str).collect();
    Ok(Json(prefs_json(
        body.dnd_enabled,
        &body.dnd_start,
        &body.dnd_end,
        &muted_refs,
    )))
}
