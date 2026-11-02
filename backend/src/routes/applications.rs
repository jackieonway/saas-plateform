use crate::util::uuid7::ParseUuid7;
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
use crate::identity_policy::MemberType;
use crate::state::AppState;
use crate::util::audit;

use super::admission_policy::{self, PolicyUpdate};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/applications", get(list).post(create))
        .route(
            "/api/applications/{id}",
            get(get_one).put(update).delete(delete),
        )
        .route(
            "/api/applications/{id}/access-policy",
            get(get_policy).put(update_policy),
        )
}

#[derive(sqlx::FromRow, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ApplicationRow {
    id: Uuid,
    code: String,
    name: String,
    icon: String,
    base_path: String,
    /// 微应用固定 HTML；历史与内置页面的旧字段仍用于读取。
    entry_type: String,
    /// qiankun HTML 入口；内置平台页面可为空。
    entry_url: String,
    runtime_mode: String,
    description: String,
    sort_order: i32,
    status: String,
    allowed_member_types: Vec<String>,
    access_policy_version: i64,
    created_at: DateTime<Utc>,
    menu_count: i64,
}

const LIST_SELECT: &str = "SELECT a.id, a.code, a.name, COALESCE(a.icon,'') AS icon, COALESCE(a.base_path,'') AS base_path, \
 a.entry_type, COALESCE(a.entry_url,'') AS entry_url, a.runtime_mode, COALESCE(a.description,'') AS description, a.sort_order, a.status, a.allowed_member_types, a.access_policy_version, a.created_at, \
 (SELECT count(*) FROM menus m WHERE m.application_id = a.id) AS menu_count";

fn audit_summary(row: &ApplicationRow) -> Value {
    json!({"id":row.id,"name":row.name,"code":row.code,"entryType":row.entry_type,
        "entryConfigured":!row.entry_url.is_empty(),"runtimeMode":row.runtime_mode,
        "status":row.status,"sortOrder":row.sort_order,"menuCount":row.menu_count,
        "allowedMemberTypes":row.allowed_member_types,"accessPolicyVersion":row.access_policy_version})
}

async fn locked_application(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
) -> Result<ApplicationRow, AppError> {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM applications WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| AppError::not_found("应用不存在"))?;
    // Related counts must use a statement snapshot taken after lock waits.
    sqlx::query_as(&format!("{LIST_SELECT} FROM applications a WHERE a.id=$1"))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| AppError::not_found("应用不存在"))
}

/// GET /api/applications — 全量列表
async fn list(State(state): State<AppState>, user: CurrentUser) -> Result<Json<Value>, AppError> {
    RequirePermission("system:application")
        .check(&state, &user)
        .await?;
    state.require_platform(&user).await?;
    let rows: Vec<ApplicationRow> = sqlx::query_as(&format!(
        "{LIST_SELECT} FROM applications a ORDER BY a.sort_order, a.created_at"
    ))
    .fetch_all(&state.pool)
    .await?;
    let records: Vec<Value> = rows
        .iter()
        .map(|r| serde_json::to_value(r).unwrap_or_default())
        .collect();
    Ok(Json(Value::Array(records)))
}

async fn get_one(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:application")
        .check(&state, &user)
        .await?;
    state.require_platform(&user).await?;
    let id: Uuid = id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let row: ApplicationRow = sqlx::query_as(&format!(
        "{LIST_SELECT} FROM applications a WHERE a.id = $1"
    ))
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
    #[serde(default, deserialize_with = "admission_policy::present_value")]
    allowed_member_types: Option<Vec<MemberType>>,
}

fn default_status() -> String {
    "ENABLED".into()
}

fn default_entry_type() -> String {
    "HTML".into()
}

fn default_runtime_mode() -> String {
    "TRUSTED".into()
}

async fn authorize_runtime_mode(
    state: &AppState,
    user: &CurrentUser,
    mode: &str,
) -> Result<(), AppError> {
    if mode != "TRUSTED" {
        return Err(AppError::bad_request("runtimeMode 仅支持 TRUSTED"));
    }
    // Explicit trust approval requires a platform administrator with application permission.
    RequirePermission("system:application")
        .check(state, user)
        .await?;
    state.require_platform(user).await
}

/// 新增和更新统一使用 HTML；历史枚举仅用于保留的停用记录。
fn parse_entry_type(t: &str) -> Result<String, AppError> {
    match t {
        "HTML" => Ok(t.to_string()),
        _ => Err(AppError::bad_request("入口类型仅支持 HTML")),
    }
}

/// 所有可维护的应用统一配置，不为内置路由保留旧加载类型。
fn validate_runtime_entry(entry_type: &str, runtime_mode: &str) -> Result<(), AppError> {
    if entry_type != "HTML" || runtime_mode != "TRUSTED" {
        return Err(AppError::bad_request(
            "应用仅支持 qiankun（HTML + TRUSTED）；停用的旧配置须由管理员明确重新配置",
        ));
    }
    Ok(())
}

/// 独立微应用入口必填；内置路由可空，非空地址统一校验。
fn validate_entry_url(required: bool, raw: &str) -> Result<String, AppError> {
    let url = raw.trim();
    if url.is_empty() {
        if required {
            return Err(AppError::bad_request(
                "HTML 入口应用必须配置入口地址 entryUrl",
            ));
        }
        return Ok(String::new());
    }
    if !url.starts_with("http://") && !url.starts_with("https://") && !url.starts_with('/') {
        return Err(AppError::bad_request(
            "入口地址必须以 http:// 、https:// 或 / 开头",
        ));
    }
    if url.chars().count() > 500 {
        return Err(AppError::bad_request("入口地址长度不能超过 500 字符"));
    }
    if url.starts_with("//") || url.contains('\\') || url.chars().any(char::is_control) {
        return Err(AppError::bad_request(
            "入口地址不能包含反斜线、控制字符或省略协议的外部来源",
        ));
    }
    let parsed = reqwest::Url::parse("https://host.invalid/")
        .and_then(|base| base.join(url))
        .map_err(|_| AppError::bad_request("入口地址格式错误"))?;
    if !["http", "https"].contains(&parsed.scheme())
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query_pairs().any(|(key, _)| {
            [
                "token",
                "access_token",
                "session",
                "session_token",
                "client_secret",
            ]
            .contains(&key.to_ascii_lowercase().as_str())
        })
    {
        return Err(AppError::bad_request(
            "入口地址必须为有效 HTTP(S) 地址且不能携带凭据或会话参数",
        ));
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
    RequirePermission("system:application")
        .check(&state, &user)
        .await?;
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
            || !code
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
    {
        return Err(AppError::bad_request(
            "HTML 微应用编码仅支持 2–32 位小写字母、数字和中划线",
        ));
    }
    if body.auto_create_menu && entry_type != "HTML" {
        return Err(AppError::bad_request(
            "自动创建首页菜单仅适用于 HTML 微应用",
        ));
    }
    if body.auto_create_menu {
        RequirePermission("system:menu")
            .check(&state, &user)
            .await?;
    }
    let base_path =
        if body.auto_create_menu || (entry_type == "HTML" && body.base_path.trim().is_empty()) {
            format!("/micro/{code}")
        } else {
            body.base_path.trim().to_string()
        };
    validate_runtime_entry(&entry_type, &body.runtime_mode)?;
    let default_entry_url = format!("/sub-apps/{code}/index.html");
    let micro = base_path.trim_end_matches('/') == format!("/micro/{code}");
    let raw_entry_url = if micro && entry_type == "HTML" && body.entry_url.trim().is_empty() {
        default_entry_url.as_str()
    } else {
        body.entry_url.as_str()
    };
    let entry_url = validate_entry_url(micro, raw_entry_url)?;
    let allowed_member_types = admission_policy::member_strings(
        body.allowed_member_types
            .unwrap_or_else(|| vec![MemberType::Employee]),
        false,
    )?;
    let mut tx = state.pool.begin().await?;
    crate::session::require_active_in_tx(
        &mut tx,
        user.user_id,
        user.tenant_id,
        user.org_id,
        user.session_id,
    )
    .await?;
    let mut row: ApplicationRow = sqlx::query_as(
        "INSERT INTO applications (code, name, icon, base_path, entry_type, entry_url, description, sort_order, status, runtime_mode, id, allowed_member_types) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
         RETURNING id, code, name, COALESCE(icon,'') AS icon, COALESCE(base_path,'') AS base_path, \
         entry_type, COALESCE(entry_url,'') AS entry_url, runtime_mode, COALESCE(description,'') AS description, \
         sort_order, status, allowed_member_types, access_policy_version, created_at, 0::bigint AS menu_count",
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
    .bind(crate::util::uuid7::new_uuid())
    .bind(&allowed_member_types).fetch_one(&mut *tx)
    .await?;
    if body.auto_create_menu {
        let inserted = sqlx::query(
            "INSERT INTO menus (application_id, type, title, path, sort_order, visible, link_target, id) \
             VALUES ($1, 'MENU', $2, $3, 1, true, 'MICRO', $4)",
        )
        .bind(row.id)
        .bind(&name)
        .bind(format!("/micro/{code}"))
        .bind(crate::util::uuid7::new_uuid()).execute(&mut *tx)
        .await?;
        if inserted.rows_affected() != 1 {
            return Err(AppError::Server(anyhow::anyhow!(
                "application menu was not persisted"
            )));
        }
        row.menu_count = 1;
    }
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let detail = audit::change_detail("application", row.id, Value::Null, audit_summary(&row));
    audit::write_in_tx_for_app(
        &mut tx,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(user.tenant_id),
            organization_id: Some(user.org_id),
            action: "CREATE_APPLICATION",
            detail: &detail,
            ip: &ip,
            user_agent: &ua,
        },
        row.id,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(serde_json::to_value(&row).unwrap_or_default()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApplicationUpdate {
    #[serde(default, deserialize_with = "admission_policy::present_value")]
    allowed_member_types: Option<Vec<MemberType>>,
    #[serde(default)]
    access_policy_version: Option<i64>,
    #[serde(default)]
    reason: Option<String>,
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

fn policy_json(row: &Value) -> Value {
    json!({"id":row["id"],"status":row["status"],
        "allowedMemberTypes":row["allowedMemberTypes"],"accessPolicyVersion":row["accessPolicyVersion"]})
}

async fn get_policy(
    state: State<AppState>,
    user: CurrentUser,
    id: Path<String>,
) -> Result<Json<Value>, AppError> {
    let Json(row) = get_one(state, user, id).await?;
    Ok(Json(policy_json(&row)))
}

async fn update_policy(
    state: State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    id: Path<String>,
    ApiJson(body): ApiJson<PolicyUpdate>,
) -> Result<Json<Value>, AppError> {
    let Json(row) = update(
        state,
        headers,
        user,
        id,
        ApiJson(ApplicationUpdate {
            allowed_member_types: Some(body.allowed_member_types),
            access_policy_version: Some(body.access_policy_version),
            reason: Some(body.reason),
            name: None,
            icon: None,
            base_path: None,
            entry_type: None,
            entry_url: None,
            runtime_mode: None,
            description: None,
            sort_order: None,
            status: None,
        }),
    )
    .await?;
    Ok(Json(policy_json(&row)))
}

/// PUT /api/applications/{id}
async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ApplicationUpdate>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:application")
        .check(&state, &user)
        .await?;
    state.require_platform(&user).await?;
    let id: Uuid = id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
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
    let new_entry_url = body.entry_url.as_ref().map(|url| url.trim().to_owned());
    let mut tx = state.pool.begin().await?;
    crate::session::require_active_in_tx(
        &mut tx,
        user.user_id,
        user.tenant_id,
        user.org_id,
        user.session_id,
    )
    .await?;
    let before = locked_application(&mut tx, id).await?;
    crate::session::require_active_in_tx(
        &mut tx,
        user.user_id,
        user.tenant_id,
        user.org_id,
        user.session_id,
    )
    .await?;
    RequirePermission("system:application")
        .check(&state, &user)
        .await?;
    state.require_platform(&user).await?;
    admission_policy::check_version(
        body.access_policy_version,
        before.access_policy_version,
        body.allowed_member_types.is_some(),
    )?;
    let allowed_member_types = body
        .allowed_member_types
        .map(|members| admission_policy::member_strings(members, false))
        .transpose()?;
    let policy_reason = if allowed_member_types.is_some() {
        Some(admission_policy::reason(body.reason.as_deref())?)
    } else {
        None
    };
    // 校验合并后的完整配置，覆盖仅修改入口类型、运行模式或挂载路径的补丁。
    let final_type = new_entry_type.as_deref().unwrap_or(&before.entry_type);
    let final_base_path = body.base_path.as_deref().unwrap_or(&before.base_path);
    let micro = final_base_path.trim_end_matches('/') == format!("/micro/{}", before.code);
    validate_entry_url(micro, new_entry_url.as_deref().unwrap_or(&before.entry_url))?;
    validate_runtime_entry(
        final_type,
        body.runtime_mode.as_deref().unwrap_or(&before.runtime_mode),
    )?;
    let row: ApplicationRow = sqlx::query_as(
        "UPDATE applications SET name = COALESCE($2, name), icon = COALESCE($3, icon), \
         base_path = COALESCE($4, base_path), entry_type = COALESCE($5, entry_type), \
         entry_url = COALESCE($6, entry_url), description = COALESCE($7, description), \
         sort_order = COALESCE($8, sort_order), status = COALESCE($9, status), runtime_mode = COALESCE($10, runtime_mode), allowed_member_types=COALESCE($11,allowed_member_types), updated_at = now() \
         WHERE id = $1 RETURNING id, code, name, COALESCE(icon,'') AS icon, COALESCE(base_path,'') AS base_path, \
         entry_type, COALESCE(entry_url,'') AS entry_url, runtime_mode, COALESCE(description,'') AS description, \
         sort_order, status, allowed_member_types, access_policy_version, created_at, \
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
    .bind(&allowed_member_types)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("应用不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let mut after = audit_summary(&row);
    if before.access_policy_version != row.access_policy_version {
        crate::credential_binding::revoke_application_policy_in_tx(&mut tx, id).await?;
    }
    if let Some(reason) = policy_reason {
        after["reason"] = json!(reason);
    }
    let changed_fields: Vec<&str> = [
        ("name", before.name != row.name),
        ("icon", before.icon != row.icon),
        ("basePath", before.base_path != row.base_path),
        ("entryType", before.entry_type != row.entry_type),
        ("entryUrl", before.entry_url != row.entry_url),
        ("runtimeMode", before.runtime_mode != row.runtime_mode),
        ("description", before.description != row.description),
        ("sortOrder", before.sort_order != row.sort_order),
        ("status", before.status != row.status),
        (
            "allowedMemberTypes",
            before.allowed_member_types != row.allowed_member_types,
        ),
    ]
    .into_iter()
    .filter_map(|(field, changed)| changed.then_some(field))
    .collect();
    if !changed_fields.is_empty() {
        after["changedFields"] = json!(changed_fields);
        let detail = audit::change_detail("application", id, audit_summary(&before), after);
        audit::write_in_tx_for_app(
            &mut tx,
            audit::AuditInfo {
                user_id: Some(user.user_id),
                username: &user.username,
                tenant_id: Some(user.tenant_id),
                organization_id: Some(user.org_id),
                action: "UPDATE_APPLICATION",
                detail: &detail,
                ip: &ip,
                user_agent: &ua,
            },
            id,
        )
        .await?;
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
    RequirePermission("system:application")
        .check(&state, &user)
        .await?;
    state.require_platform(&user).await?;
    let id: Uuid = id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let mut tx = state.pool.begin().await?;
    crate::session::require_active_in_tx(
        &mut tx,
        user.user_id,
        user.tenant_id,
        user.org_id,
        user.session_id,
    )
    .await?;
    let before = locked_application(&mut tx, id).await?;
    crate::session::require_active_in_tx(
        &mut tx,
        user.user_id,
        user.tenant_id,
        user.org_id,
        user.session_id,
    )
    .await?;
    RequirePermission("system:application")
        .check(&state, &user)
        .await?;
    state.require_platform(&user).await?;
    let (_name,): (String,) =
        sqlx::query_as("DELETE FROM applications WHERE id = $1 RETURNING name")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| AppError::not_found("应用不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let detail = audit::change_detail("application", id, audit_summary(&before), Value::Null);
    audit::write_in_tx_for_app(
        &mut tx,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(user.tenant_id),
            organization_id: Some(user.org_id),
            action: "DELETE_APPLICATION",
            detail: &detail,
            ip: &ip,
            user_agent: &ua,
        },
        id,
    )
    .await?;
    tx.commit().await?;
    state.invalidate_perm_cache();
    Ok(Json(json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_entry_rejects_browser_normalization_and_credentials() {
        for invalid in [
            "//external.test/app/",
            "/\\external.test/app/",
            "https://user:secret@external.test/app/",
            "https://external.test/a\nb/",
            "http://[invalid/",
            "/app/?access_token=secret",
            "/app/?CLIENT_SECRET=secret",
        ] {
            assert!(validate_entry_url(true, invalid).is_err(), "{invalid:?}");
        }
        for valid in [
            "/sub-apps/demo/versions/v1/index.html",
            "https://external.test/v1/index.html?locale=zh",
        ] {
            assert_eq!(validate_entry_url(true, valid).unwrap(), valid);
        }
        assert!(validate_entry_url(true, "").is_err());
        assert_eq!(validate_entry_url(false, "").unwrap(), "");
    }

    #[test]
    fn all_application_configs_only_accept_html_trusted() {
        for (entry_type, runtime) in [("ESM", "IFRAME"), ("ESM", "TRUSTED"), ("HTML", "IFRAME")] {
            assert!(validate_runtime_entry(entry_type, runtime).is_err());
        }
        assert!(validate_runtime_entry("HTML", "TRUSTED").is_ok());
        assert!(parse_entry_type("ESM").is_err());
    }
}
