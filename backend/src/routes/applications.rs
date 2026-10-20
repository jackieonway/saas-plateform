
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::{ApiJson, CurrentUser, RequirePermission};
use crate::state::AppState;
use crate::util::audit;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/applications", get(list).post(create))
        .route("/api/applications/{id}", get(get_one).put(update).delete(delete))
}

#[derive(sqlx::FromRow, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ApplicationRow {
    id: Uuid,
    code: String,
    name: String,
    icon: String,
    base_path: String,
    /// 入口通道: 'ESM' (原生模块) | 'HTML' (微前端 HTML 入口, 迁移 0011)
    entry_type: String,
    /// HTML 通道 = 子应用 HTML 入口(如 /sub/vue/); ESM 通道 = 模块 URL; 空 = 未配置
    entry_url: String,
    runtime_mode: String,
    description: String,
    sort_order: i32,
    status: String,
    created_at: DateTime<Utc>,
    menu_count: i64,
}

const LIST_SELECT: &str = "SELECT a.id, a.code, a.name, COALESCE(a.icon,'') AS icon, COALESCE(a.base_path,'') AS base_path, \
 a.entry_type, COALESCE(a.entry_url,'') AS entry_url, a.runtime_mode, COALESCE(a.description,'') AS description, a.sort_order, a.status, a.created_at, \
 (SELECT count(*) FROM menus m WHERE m.application_id = a.id) AS menu_count";

fn audit_summary(row: &ApplicationRow) -> Value {
    json!({"id":row.id,"name":row.name,"code":row.code,"entryType":row.entry_type,
        "entryConfigured":!row.entry_url.is_empty(),"runtimeMode":row.runtime_mode,
        "status":row.status,"sortOrder":row.sort_order,"menuCount":row.menu_count})
}

async fn locked_application(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, id: Uuid,
) -> Result<ApplicationRow, AppError> {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM applications WHERE id=$1 FOR UPDATE")
        .bind(id).fetch_optional(&mut **tx).await?
        .ok_or_else(|| AppError::not_found("应用不存在"))?;
    // Related counts must use a statement snapshot taken after lock waits.
    sqlx::query_as(&format!("{LIST_SELECT} FROM applications a WHERE a.id=$1"))
        .bind(id).fetch_optional(&mut **tx).await?
        .ok_or_else(|| AppError::not_found("应用不存在"))
}

/// GET /api/applications — 全量列表
async fn list(State(state): State<AppState>, user: CurrentUser) -> Result<Json<Value>, AppError> {
    RequirePermission("system:application").check(&state, &user).await?;
    state.require_platform(&user).await?;
    let rows: Vec<ApplicationRow> =
        sqlx::query_as(&format!("{LIST_SELECT} FROM applications a ORDER BY a.sort_order, a.created_at"))
            .fetch_all(&state.pool)
            .await?;
    let records: Vec<Value> = rows.iter().map(|r| serde_json::to_value(r).unwrap_or_default()).collect();
    Ok(Json(Value::Array(records)))
}

async fn get_one(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:application").check(&state, &user).await?;
    state.require_platform(&user).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let row: ApplicationRow = sqlx::query_as(&format!("{LIST_SELECT} FROM applications a WHERE a.id = $1"))
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found("应用不存在"))?;
    Ok(Json(serde_json::to_value(&row).unwrap_or_default()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApplicationBody {
    code: String,
    name: String,
    #[serde(default)]
    icon: String,
    #[serde(default)]
    base_path: String,
    #[serde(default = "default_entry_type")]
    entry_type: String,
    #[serde(default)]
    entry_url: String,
    #[serde(default = "default_runtime_mode")]
    runtime_mode: String,
    #[serde(default)]
    auto_create_menu: bool,
    #[serde(default)]
    description: String,
    #[serde(default)]
    sort_order: i32,
    #[serde(default = "default_status")]
    status: String,
}

fn default_status() -> String {
    "ENABLED".into()
}

fn default_entry_type() -> String {
    "ESM".into()
}

fn default_runtime_mode() -> String {
    "IFRAME".into()
}

async fn authorize_runtime_mode(state: &AppState, user: &CurrentUser, mode: &str) -> Result<(), AppError> {
    if !["IFRAME", "TRUSTED"].contains(&mode) {
        return Err(AppError::bad_request("runtimeMode 仅支持 IFRAME / TRUSTED"));
    }
    // Explicit trust approval requires a platform administrator with application permission.
    RequirePermission("system:application").check(state, user).await?;
    state.require_platform(user).await
}

/// 校验入口类型: 仅 ESM / HTML
fn parse_entry_type(t: &str) -> Result<String, AppError> {
    match t {
        "ESM" | "HTML" => Ok(t.to_string()),
        _ => Err(AppError::bad_request("入口类型仅支持 ESM / HTML")),
    }
}

/// 校验并归一入口地址 (与 link_url 同口径):
/// entry_type=HTML 时必填; 非空时必须 http(s):// 或 / 开头且 ≤ 500 字符; ESM 时可空
fn validate_entry_url(entry_type: &str, raw: &str) -> Result<String, AppError> {
    let url = raw.trim();
    if url.is_empty() {
        if entry_type == "HTML" {
            return Err(AppError::bad_request("HTML 入口应用必须配置入口地址 entryUrl"));
        }
        return Ok(String::new());
    }
    if !url.starts_with("http://") && !url.starts_with("https://") && !url.starts_with('/') {
        return Err(AppError::bad_request("入口地址必须以 http:// 、https:// 或 / 开头"));
    }
    if url.chars().count() > 500 {
        return Err(AppError::bad_request("入口地址长度不能超过 500 字符"));
    }
    Ok(url.to_string())
}

/// POST /api/applications
async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<ApplicationBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:application").check(&state, &user).await?;
    state.require_platform(&user).await?;
    authorize_runtime_mode(&state, &user, &body.runtime_mode).await?;
    let name = body.name.trim().to_string();
    if name.is_empty() || body.code.trim().is_empty() {
        return Err(AppError::bad_request("应用名称与编码不能为空"));
    }
    if !["ENABLED", "DISABLED"].contains(&body.status.as_str()) {
        return Err(AppError::bad_request("状态仅支持 ENABLED / DISABLED"));
    }
    let code = body.code.trim();
    let entry_type = parse_entry_type(body.entry_type.trim())?;
    if entry_type == "HTML"
        && (code.len() < 2
            || code.len() > 32
            || !code.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
    {
        return Err(AppError::bad_request("HTML 微应用编码仅支持 2–32 位小写字母、数字和中划线"));
    }
    if body.auto_create_menu && entry_type != "HTML" {
        return Err(AppError::bad_request("自动创建首页菜单仅适用于 HTML 微应用"));
    }
    if body.auto_create_menu {
        RequirePermission("system:menu").check(&state, &user).await?;
    }
    let base_path = if body.auto_create_menu || (entry_type == "HTML" && body.base_path.trim().is_empty()) {
        format!("/micro/{code}")
    } else {
        body.base_path.trim().to_string()
    };
    let default_entry_url = format!("/sub-apps/{code}/index.html");
    let default_esm_entry_url = format!("/src/micro-apps/{code}.tsx");
    let raw_entry_url = if entry_type == "HTML" && body.entry_url.trim().is_empty() {
        default_entry_url.as_str()
    } else if entry_type == "ESM" && body.entry_url.trim().is_empty()
        && base_path == format!("/micro/{code}") {
        default_esm_entry_url.as_str()
    } else {
        body.entry_url.as_str()
    };
    let entry_url = validate_entry_url(&entry_type, raw_entry_url)?;
    let mut tx = state.pool.begin().await?;
    let mut row: ApplicationRow = sqlx::query_as(
        "INSERT INTO applications (code, name, icon, base_path, entry_type, entry_url, description, sort_order, status, runtime_mode) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
         RETURNING id, code, name, COALESCE(icon,'') AS icon, COALESCE(base_path,'') AS base_path, \
         entry_type, COALESCE(entry_url,'') AS entry_url, runtime_mode, COALESCE(description,'') AS description, \
         sort_order, status, created_at, 0::bigint AS menu_count",
    )
    .bind(code)
    .bind(&name)
    .bind(&body.icon)
    .bind(&base_path)
    .bind(&entry_type)
    .bind(&entry_url)
    .bind(&body.description)
    .bind(body.sort_order)
    .bind(&body.status)
    .bind(&body.runtime_mode)
    .fetch_one(&mut *tx)
    .await?;
    if body.auto_create_menu {
        let inserted = sqlx::query(
            "INSERT INTO menus (application_id, type, title, path, sort_order, visible, link_target) \
             VALUES ($1, 'MENU', $2, $3, 1, true, 'MICRO')",
        )
        .bind(row.id)
        .bind(&name)
        .bind(format!("/micro/{code}"))
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() != 1 {
            return Err(AppError::Server(anyhow::anyhow!("application menu was not persisted")));
        }
        row.menu_count = 1;
    }
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let detail = audit::change_detail("application", row.id, Value::Null, audit_summary(&row));
    audit::write_in_tx_for_app(&mut tx, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "CREATE_APPLICATION", detail: &detail, ip: &ip, user_agent: &ua,
    }, row.id).await?;
    tx.commit().await?;
    Ok(Json(serde_json::to_value(&row).unwrap_or_default()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApplicationUpdate {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    base_path: Option<String>,
    #[serde(default)]
    entry_type: Option<String>,
    #[serde(default)]
    entry_url: Option<String>,
    #[serde(default)]
    runtime_mode: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    sort_order: Option<i32>,
    #[serde(default)]
    status: Option<String>,
}

/// PUT /api/applications/{id}
async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ApplicationUpdate>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:application").check(&state, &user).await?;
    state.require_platform(&user).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    if let Some(mode) = &body.runtime_mode {
        authorize_runtime_mode(&state, &user, mode).await?;
    }
    if let Some(name) = &body.name {
        if name.trim().is_empty() {
            return Err(AppError::bad_request("应用名称不能为空"));
        }
    }
    if let Some(status) = &body.status {
        if !["ENABLED", "DISABLED"].contains(&status.as_str()) {
            return Err(AppError::bad_request("状态仅支持 ENABLED / DISABLED"));
        }
    }
    let new_entry_type = match &body.entry_type {
        Some(t) => Some(parse_entry_type(t.trim())?),
        None => None,
    };
    // entry_url: 键缺省 = 不更新; 显式传值(含空串) = 按目标 entry_type 校验后更新
    let new_entry_url = match &body.entry_url {
        Some(u) => Some(validate_entry_url(new_entry_type.as_deref().unwrap_or("ESM"), u)?),
        None => None,
    };
    let mut tx = state.pool.begin().await?;
    let before = locked_application(&mut tx, id).await?;
    // HTML 完整性: 切到/保持 HTML 时, 最终 entry_url 不能为空 → 预读当前行做合并校验
    if new_entry_type.as_deref() == Some("HTML") || new_entry_type.is_none() {
        let final_type = new_entry_type.as_deref().unwrap_or(&before.entry_type);
        let final_url = new_entry_url.as_deref().unwrap_or(&before.entry_url);
        if final_type == "HTML" && final_url.is_empty() {
            return Err(AppError::bad_request("HTML 入口应用必须配置入口地址 entryUrl"));
        }
    }
    let row: ApplicationRow = sqlx::query_as(
        "UPDATE applications SET name = COALESCE($2, name), icon = COALESCE($3, icon), \
         base_path = COALESCE($4, base_path), entry_type = COALESCE($5, entry_type), \
         entry_url = COALESCE($6, entry_url), description = COALESCE($7, description), \
         sort_order = COALESCE($8, sort_order), status = COALESCE($9, status), runtime_mode = COALESCE($10, runtime_mode), updated_at = now() \
         WHERE id = $1 RETURNING id, code, name, COALESCE(icon,'') AS icon, COALESCE(base_path,'') AS base_path, \
         entry_type, COALESCE(entry_url,'') AS entry_url, runtime_mode, COALESCE(description,'') AS description, \
         sort_order, status, created_at, \
         (SELECT count(*) FROM menus m WHERE m.application_id = applications.id) AS menu_count"
    )
    .bind(id)
    .bind(&body.name)
    .bind(&body.icon)
    .bind(&body.base_path)
    .bind(&new_entry_type)
    .bind(&new_entry_url)
    .bind(&body.description)
    .bind(body.sort_order)
    .bind(&body.status)
    .bind(&body.runtime_mode)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("应用不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let mut after = audit_summary(&row);
    let changed_fields: Vec<&str> = [
        ("name",before.name != row.name),("icon",before.icon != row.icon),
        ("basePath",before.base_path != row.base_path),("entryType",before.entry_type != row.entry_type),
        ("entryUrl",before.entry_url != row.entry_url),("runtimeMode",before.runtime_mode != row.runtime_mode),
        ("description",before.description != row.description),("sortOrder",before.sort_order != row.sort_order),
        ("status",before.status != row.status),
    ].into_iter().filter_map(|(field,changed)| changed.then_some(field)).collect();
    if !changed_fields.is_empty() {
        after["changedFields"] = json!(changed_fields);
        let detail = audit::change_detail("application", id, audit_summary(&before), after);
        audit::write_in_tx_for_app(&mut tx, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "UPDATE_APPLICATION", detail: &detail, ip: &ip, user_agent: &ua,
        }, id).await?;
    }
    tx.commit().await?;
    Ok(Json(serde_json::to_value(&row).unwrap_or_default()))
}

/// DELETE /api/applications/{id} — 级联删除菜单与各层级授权
async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:application").check(&state, &user).await?;
    state.require_platform(&user).await?;
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let mut tx = state.pool.begin().await?;
    let before = locked_application(&mut tx, id).await?;
    let (_name,): (String,) = sqlx::query_as("DELETE FROM applications WHERE id = $1 RETURNING name")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::not_found("应用不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let detail = audit::change_detail("application", id, audit_summary(&before), Value::Null);
    audit::write_in_tx_for_app(&mut tx, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "DELETE_APPLICATION", detail: &detail, ip: &ip, user_agent: &ua,
    }, id).await?;
    tx.commit().await?;
    state.invalidate_perm_cache();
    Ok(Json(json!({"ok": true})))
}
