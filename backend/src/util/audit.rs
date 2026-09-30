use axum::http::header::USER_AGENT;
use axum::http::HeaderMap;
use sqlx::PgPool;
use uuid::Uuid;

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

/// 审计日志条目 (tenant/organization 名称由 SQL 子查询填充)
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
    let result = sqlx::query(
        "INSERT INTO audit_logs \
         (user_id, username, tenant_id, tenant_name, organization_id, organization_name, action, detail, ip, user_agent) \
         VALUES ($1, $2, $3, (SELECT name FROM tenants WHERE id = $3), \
                 $4, (SELECT name FROM organizations WHERE id = $4), $5, $6, $7, $8)",
    )
    .bind(info.user_id)
    .bind(info.username)
    .bind(info.tenant_id)
    .bind(info.organization_id)
    .bind(info.action)
    .bind(info.detail)
    .bind(info.ip)
    .bind(info.user_agent)
    .execute(pool)
    .await;
    if let Err(e) = result {
        tracing::error!("审计日志写入失败: {e}");
    }
}
