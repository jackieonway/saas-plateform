//! Mutable URL entry configuration. Release definitions remain immutable; a URL
//! row explicitly overrides the release pointer for one environment only.
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::{ApiJson, CurrentUser, RequirePermission};
use crate::releases::{self, FetchPolicy};
use crate::state::AppState;
use crate::util::audit;

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/api/applications/{id}/url-entry",
        get(read).put(save).delete(remove),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryQuery {
    environment: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EntryBody {
    entry_url: String,
    #[serde(default = "default_proxy")]
    proxy_enabled: bool,
}

fn default_proxy() -> bool {
    true
}

#[derive(sqlx::FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
struct EntryRow {
    application_id: Uuid,
    environment: String,
    entry_url: String,
    proxy_enabled: bool,
    revision: Uuid,
    updated_at: DateTime<Utc>,
}

impl EntryRow {
    fn response(&self, code: &str) -> Value {
        let mut value = json!(self);
        value["browserEntryUrl"] = json!(browser_entry_url(
            code,
            &self.entry_url,
            self.proxy_enabled,
            self.revision
        ));
        value
    }

    fn audit_summary(&self) -> Value {
        // URLs can contain upstream query values. They are never audit data.
        json!({"applicationId":self.application_id,"environment":self.environment,
            "proxyEnabled":self.proxy_enabled,"revision":self.revision,"entryConfigured":true})
    }
}

pub struct ProxyEntry {
    pub entry_url: String,
    pub revision: Uuid,
}

/// The proxy has no user-selected host: it can only resolve an explicitly
/// approved, enabled URL entry in the server's current application environment.
pub async fn lookup_proxy_entry(
    pool: &PgPool,
    code: &str,
    environment: &str,
) -> Result<Option<ProxyEntry>, AppError> {
    let row: Option<(String, Uuid)> = sqlx::query_as(
        "SELECT u.entry_url,u.revision FROM application_url_entries u \
         JOIN applications a ON a.id=u.application_id \
         WHERE a.code=$1 AND u.environment=$2 AND u.proxy_enabled \
           AND a.status='ENABLED' AND a.entry_type='HTML' \
           AND a.runtime_mode='TRUSTED' AND rtrim(a.base_path,'/')='/micro/' || a.code",
    )
    .bind(code)
    .bind(environment)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(entry_url, revision)| ProxyEntry {
        entry_url,
        revision,
    }))
}

pub fn browser_entry_url(code: &str, upstream: &str, proxy: bool, revision: Uuid) -> String {
    if !proxy {
        return upstream.to_owned();
    }
    let mut result = format!("/micro-proxy/{code}/{revision}/");
    if let Some((_, query)) = upstream.split_once('?') {
        result.push('?');
        result.push_str(query);
    }
    result
}

fn validated_url(policy: &FetchPolicy, raw: &str) -> Result<String, AppError> {
    let url = Url::parse(raw.trim())
        .map_err(|_| AppError::bad_request("直接入口必须是完整的 HTTP(S) 地址"))?;
    policy.check(&url)?;
    if url.as_str().len() > 2000 || raw.contains('\\') || raw.chars().any(char::is_control) {
        return Err(AppError::bad_request("入口 URL 含非法字符或超过 2000 字符"));
    }
    if url.query_pairs().any(|(name, _)| {
        [
            "__platform_entry",
            "__saas_execution",
            "__saas_lease",
            "token",
            "access_token",
            "session",
            "session_token",
            "client_secret",
        ]
        .contains(&name.to_ascii_lowercase().as_str())
    }) {
        return Err(AppError::bad_request(
            "入口 URL 不可携带平台保留参数或登录凭据",
        ));
    }
    let decoded_path = percent_encoding::percent_decode_str(url.path())
        .decode_utf8()
        .map_err(|_| AppError::bad_request("入口 URL 路径无效"))?;
    let html_path = decoded_path.to_ascii_lowercase();
    if !url.path().ends_with('/') && !html_path.ends_with(".html") && !html_path.ends_with(".htm") {
        return Err(AppError::bad_request("请填写最终 HTML 入口（.html/.htm）或以 / 结尾的目录地址，例如 https://app.example.com/app/，不要填写需要跳转补全目录的地址"));
    }
    let path_segments = decoded_path
        .trim_start_matches('/')
        .split('/')
        .collect::<Vec<_>>();
    if path_segments.first() == Some(&"sub-apps") && path_segments.get(2) == Some(&"versions") {
        return Err(AppError::bad_request(
            "平台托管的固定版本目录请使用版本发布、激活和回滚，不可配置为直接地址",
        ));
    }
    // Deliberately no version_root: this address is a mutable environment URL.
    Ok(url.to_string())
}

async fn authorize(state: &AppState, user: &CurrentUser) -> Result<(), AppError> {
    RequirePermission("system:application")
        .check(state, user)
        .await?;
    state.require_platform(user).await
}

async fn entry(
    tx: &mut Transaction<'_, Postgres>,
    app: Uuid,
    environment: &str,
) -> Result<Option<EntryRow>, AppError> {
    Ok(sqlx::query_as(
        "SELECT application_id,environment,entry_url,proxy_enabled,revision,updated_at \
         FROM application_url_entries WHERE application_id=$1 AND environment=$2",
    )
    .bind(app)
    .bind(environment)
    .fetch_optional(&mut **tx)
    .await?)
}

async fn read(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<Uuid>,
    Query(query): Query<EntryQuery>,
) -> Result<Json<Value>, AppError> {
    let id = crate::util::uuid7::validate_uuid(id)?;
    authorize(&state, &user).await?;
    let environment = releases::validate_environment(&query.environment)?;
    let mut tx = state.pool.begin().await?;
    let code: String = sqlx::query_scalar("SELECT code FROM applications WHERE id=$1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::not_found("应用不存在"))?;
    let row = entry(&mut tx, id, &environment).await?;
    tx.commit().await?;
    Ok(Json(
        row.map(|row| row.response(&code)).unwrap_or(Value::Null),
    ))
}

async fn save(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(query): Query<EntryQuery>,
    ApiJson(body): ApiJson<EntryBody>,
) -> Result<Json<Value>, AppError> {
    let id = crate::util::uuid7::validate_uuid(id)?;
    authorize(&state, &user).await?;
    let environment = releases::validate_environment(&query.environment)?;
    let policy = FetchPolicy::new(&state.config, &environment)?;
    let url = validated_url(&policy, &body.entry_url)?;
    let mut tx = state.pool.begin().await?;
    let (code, base_path, kind, mode): (String, String, String, String) = sqlx::query_as(
        "SELECT code,COALESCE(base_path,''),entry_type,runtime_mode FROM applications WHERE id=$1 FOR UPDATE",
    ).bind(id).fetch_optional(&mut *tx).await?
        .ok_or_else(|| AppError::not_found("应用不存在"))?;
    crate::session::require_active_in_tx(
        &mut tx,
        user.user_id,
        user.tenant_id,
        user.org_id,
        user.session_id,
    )
    .await?;
    crate::routes::platform_application_permissions(&mut tx, &user).await?;
    if kind != "HTML"
        || mode != "TRUSTED"
        || base_path.trim_end_matches('/') != format!("/micro/{code}")
    {
        return Err(AppError::bad_request(
            "直接地址仅支持 qiankun（HTML + TRUSTED）独立微应用，请先明确重新配置旧入口",
        ));
    }
    let before = entry(&mut tx, id, &environment).await?;
    if let Some(row) = &before {
        if row.entry_url == url && row.proxy_enabled == body.proxy_enabled {
            tx.commit().await?;
            return Ok(Json(row.response(&code)));
        }
    }
    sqlx::query(
        "INSERT INTO application_url_entries(application_id,environment,entry_url,proxy_enabled,revision,updated_by) \
         VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(application_id,environment) DO UPDATE \
         SET entry_url=EXCLUDED.entry_url,proxy_enabled=EXCLUDED.proxy_enabled,revision=EXCLUDED.revision,updated_by=EXCLUDED.updated_by,updated_at=now()",
    ).bind(id).bind(&environment).bind(url).bind(body.proxy_enabled).bind(crate::util::uuid7::new_uuid()).bind(user.user_id)
        .execute(&mut *tx).await?;
    let after = entry(&mut tx, id, &environment)
        .await?
        .ok_or_else(|| AppError::conflict("入口保存失败，请重试"))?;
    let detail = audit::change_detail(
        "application_url_entry",
        id,
        before.map(|row| row.audit_summary()).unwrap_or(Value::Null),
        after.audit_summary(),
    );
    write_audit(
        &mut tx,
        &user,
        &headers,
        id,
        "CONFIGURE_APPLICATION_URL_ENTRY",
        &detail,
    )
    .await?;
    crate::routes::platform_application_permissions(&mut tx, &user).await?;
    tx.commit().await?;
    Ok(Json(after.response(&code)))
}

async fn remove(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(query): Query<EntryQuery>,
) -> Result<Json<Value>, AppError> {
    let id = crate::util::uuid7::validate_uuid(id)?;
    authorize(&state, &user).await?;
    let environment = releases::validate_environment(&query.environment)?;
    let mut tx = state.pool.begin().await?;
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM applications WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::not_found("应用不存在"))?;
    crate::session::require_active_in_tx(
        &mut tx,
        user.user_id,
        user.tenant_id,
        user.org_id,
        user.session_id,
    )
    .await?;
    crate::routes::platform_application_permissions(&mut tx, &user).await?;
    clear_override(&mut tx, &user, &headers, id, &environment).await?;
    crate::routes::platform_application_permissions(&mut tx, &user).await?;
    tx.commit().await?;
    Ok(Json(Value::Null))
}

/// Caller must hold the application lock. Release activation and rollback call
/// this in their pointer transaction, so a mutable override cannot mask them.
pub async fn clear_override(
    tx: &mut Transaction<'_, Postgres>,
    user: &CurrentUser,
    headers: &HeaderMap,
    id: Uuid,
    environment: &str,
) -> Result<(), AppError> {
    if let Some(before) = entry(tx, id, environment).await? {
        let changed = sqlx::query(
            "DELETE FROM application_url_entries WHERE application_id=$1 AND environment=$2",
        )
        .bind(id)
        .bind(environment)
        .execute(&mut **tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(AppError::conflict("直接入口配置已变化，请重试"));
        }
        let detail = audit::change_detail(
            "application_url_entry",
            id,
            before.audit_summary(),
            Value::Null,
        );
        write_audit(
            tx,
            user,
            headers,
            id,
            "CLEAR_APPLICATION_URL_ENTRY",
            &detail,
        )
        .await?;
    }
    Ok(())
}

async fn write_audit(
    tx: &mut Transaction<'_, Postgres>,
    user: &CurrentUser,
    headers: &HeaderMap,
    id: Uuid,
    action: &str,
    detail: &str,
) -> Result<(), AppError> {
    audit::write_in_tx_for_app(
        tx,
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
        id,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_entry_keeps_query_and_changes_identity_on_reconfiguration() {
        let first = crate::util::uuid7::new_uuid();
        let second = crate::util::uuid7::new_uuid();
        let url = browser_entry_url(
            "orders",
            "https://orders.example.com/app/?theme=dark&next=a?b",
            true,
            first,
        );
        assert_eq!(
            url,
            format!("/micro-proxy/orders/{first}/?theme=dark&next=a?b")
        );
        assert_ne!(
            url,
            browser_entry_url(
                "orders",
                "https://orders.example.com/app/?theme=dark&next=a?b",
                true,
                second
            )
        );
        assert_eq!(
            browser_entry_url("orders", "https://orders.example.com/", false, first),
            "https://orders.example.com/"
        );
    }
}
