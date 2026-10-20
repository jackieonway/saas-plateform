use axum::http::header::USER_AGENT;
use axum::http::HeaderMap;
use sqlx::{Executor, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;

/// 取客户端 IP: X-Real-IP → X-Forwarded-For 第一个 → 本机
pub fn client_ip(headers: &HeaderMap) -> String {
    if let Some(v) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        if !v.trim().is_empty() {
            return v.trim().to_string();
        }
    }
    if let Some(v) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = v.split(',').next() {
            if !first.trim().is_empty() {
                return first.trim().to_string();
            }
        }
    }
    "127.0.0.1".to_string()
}

/// 取客户端 User-Agent: 缺失/空白返回 "unknown", 截断到 500 字符 (与 audit_logs.user_agent 列宽一致)
pub fn ua_of(headers: &HeaderMap) -> String {
    let raw = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    match raw {
        Some(ua) => ua.chars().take(500).collect(),
        None => "unknown".to_string(),
    }
}

/// 审计日志条目。删除租户/组织前写入，以在同事务内保留名称快照。
pub struct AuditInfo<'a> {
    pub user_id: Option<Uuid>,
    pub username: &'a str,
    pub tenant_id: Option<Uuid>,
    pub organization_id: Option<Uuid>,
    pub action: &'a str,
    pub detail: &'a str,
    pub ip: &'a str,
    pub user_agent: &'a str,
}

/// 写审计日志 (失败仅记录错误, 不影响主流程)
pub async fn write(pool: &PgPool, info: AuditInfo<'_>) {
    write_with_application(pool, info, None).await;
}

async fn write_with_application(pool: &PgPool, info: AuditInfo<'_>, application_id: Option<Uuid>) {
    if insert(pool, info, application_id).await.is_err() {
        crate::observability::record_audit_failure();
        tracing::error!(event = "audit_write_failed", request_id = ?crate::observability::request_id());
    }
}

/// 与关键管理变更共用主事务；失败必须传回调用方，使整个事务回滚。
pub async fn write_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    info: AuditInfo<'_>,
) -> Result<(), AppError> {
    write_in_tx_with_application(tx, info, None).await
}

/// 显式绑定应用的事务审计，保留发布和通知的应用归属。
pub async fn write_in_tx_for_app(
    tx: &mut Transaction<'_, Postgres>,
    info: AuditInfo<'_>,
    application_id: Uuid,
) -> Result<(), AppError> {
    write_in_tx_with_application(tx, info, Some(application_id)).await
}

async fn write_in_tx_with_application(
    tx: &mut Transaction<'_, Postgres>,
    info: AuditInfo<'_>,
    application_id: Option<Uuid>,
) -> Result<(), AppError> {
    let result = insert(&mut **tx, info, application_id).await;
    if result.is_err() {
        crate::observability::record_audit_failure();
        tracing::error!(event = "audit_write_failed", request_id = ?crate::observability::request_id());
    }
    result
}

/// 只接受调用方显式选择的最小安全字段，禁止传入完整请求或含凭据的数据库行。
pub fn change_detail(
    object_type: &str,
    object_id: Uuid,
    before: serde_json::Value,
    after: serde_json::Value,
) -> String {
    serde_json::json!({
        "objectType": object_type,
        "objectId": object_id,
        "before": before,
        "after": after,
    })
    .to_string()
}

async fn insert<'e>(
    executor: impl Executor<'e, Database = Postgres>,
    info: AuditInfo<'_>,
    application_id: Option<Uuid>,
) -> Result<(), AppError> {
    let result = sqlx::query(
        "INSERT INTO audit_logs \
         (user_id, username, tenant_id, tenant_name, organization_id, organization_name, action, detail, ip, user_agent, request_id, application_id) \
         VALUES ($1, $2, $3, (SELECT name FROM tenants WHERE id = $3), \
                 $4, (SELECT name FROM organizations WHERE id = $4), $5, $6, $7, $8, $9, \
                 COALESCE($11,(SELECT id FROM applications WHERE code=$10)))",
    )
    .bind(info.user_id)
    .bind(info.username)
    .bind(info.tenant_id)
    .bind(info.organization_id)
    .bind(info.action)
    .bind(info.detail)
    .bind(info.ip)
    .bind(info.user_agent)
    .bind(crate::observability::request_id())
    .bind(crate::observability::app_code())
    .bind(application_id)
    .execute(executor)
    .await
    .map_err(|error| AppError::Server(anyhow::Error::new(error)))?;
    // A BEFORE INSERT trigger can silently skip the insert with RETURN NULL.
    // A successful statement alone does not prove a durable audit row exists.
    if result.rows_affected() != 1 {
        return Err(AppError::Server(anyhow::anyhow!(
            "audit insert did not create one row"
        )));
    }
    Ok(())
}
