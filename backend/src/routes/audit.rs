use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::{CurrentUser, RequirePermission};
use crate::routes::{page_json, paging};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/audit-logs", get(list))
        .route("/api/audit-logs/export", get(export_csv))
}

type AuditRowTuple = (
    i64,
    String,
    Option<Uuid>,
    String,
    String,
    String,
    String,
    String,
    DateTime<Utc>,
    String,
    Option<Uuid>,
    Option<Uuid>,
);

const AUDIT_WHERE: &str =
    "($1 = '' OR username ILIKE '%' || $1 || '%' OR detail ILIKE '%' || $1 || '%' \
     OR tenant_name ILIKE '%' || $1 || '%' OR organization_name ILIKE '%' || $1 || '%' \
     OR request_id::text ILIKE '%' || $1 || '%' OR application_id::text ILIKE '%' || $1 || '%') \
     AND ($2 = '' OR action = $2) \
     AND ($3::uuid IS NULL OR tenant_id = $3) \
     AND ($5::uuid IS NULL OR request_id = $5) \
     AND ($6::uuid IS NULL OR application_id = $6) \
     AND ($7::uuid[] IS NULL OR organization_id = ANY($7) OR ($8 AND organization_id IS NULL))";

/// Only a tenant root may include legacy tenant-level rows with no organization.
/// Descendant contexts cannot attribute those rows to their own data boundary.
async fn organization_scope(
    state: &AppState,
    user: &CurrentUser,
) -> Result<(Option<Vec<Uuid>>, bool), AppError> {
    let ids = state.scoped_org_ids(user).await?;
    let include_unassigned = if ids.is_some() {
        sqlx::query_scalar::<_, bool>(
            "SELECT parent_id IS NULL FROM organizations WHERE id=$1 AND tenant_id=$2",
        )
        .bind(user.org_id)
        .bind(user.tenant_id)
        .fetch_optional(&state.pool)
        .await?
        .unwrap_or(false)
    } else {
        true
    };
    Ok((ids, include_unassigned))
}

fn correlation_filter(
    params: &HashMap<String, String>,
    key: &str,
) -> Result<Option<Uuid>, AppError> {
    match params.get(key).filter(|value| !value.is_empty()) {
        Some(value) => value
            .parse()
            .map(Some)
            .map_err(|_| AppError::bad_request(format!("{key} 参数非法"))),
        None => Ok(None),
    }
}

/// 解析 tenant_id 筛选参数: 空/缺省 = 不过滤(全部); "none" = 仅平台级(NULL 租户)行; 其他 = 指定租户 uuid
/// 返回 (附加 SQL 片段, 绑定值); 非平台用户 scope 已强制本租户, 再叠加任意 tenant 过滤只会收窄/清空, 无越权面
fn tenant_filter_clause(tf: &str) -> Result<(&'static str, Option<Uuid>), AppError> {
    if tf.is_empty() {
        Ok(("", None))
    } else if tf == "none" {
        Ok((" AND tenant_id IS NULL", None))
    } else {
        let id = Uuid::parse_str(tf).map_err(|_| AppError::bad_request("tenant_id 参数非法"))?;
        Ok((" AND ($4::uuid IS NULL OR tenant_id = $4)", Some(id)))
    }
}

/// CSV 字段转义: 含逗号/引号/换行时用双引号包裹并把内部引号翻倍 (导出端点共用)
pub(crate) fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// GET /api/audit-logs/export?keyword=&action= — 导出过滤后的审计日志 CSV (带 BOM, Excel 友好, 上限 10000 行)
async fn export_csv(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    RequirePermission("system:audit")
        .check(&state, &user)
        .await?;
    let kw = params.get("keyword").cloned().unwrap_or_default();
    let action = params.get("action").cloned().unwrap_or_default();
    // 非平台租户仅能导出本租户审计行 (平台级 NULL 租户行也仅平台可见)
    let scope: Option<Uuid> = if state.is_platform_context(&user).await? {
        None
    } else {
        Some(user.tenant_id)
    };
    let tf = params.get("tenant_id").cloned().unwrap_or_default();
    let (tf_sql, tf_val) = tenant_filter_clause(&tf)?;
    let request_id = correlation_filter(&params, "request_id")?;
    let application_id = correlation_filter(&params, "application_id")?;
    let (org_scope, include_unassigned) = organization_scope(&state, &user).await?;
    let rows: Vec<AuditRowTuple> = sqlx::query_as(&format!(
        "SELECT id, COALESCE(username,''), tenant_id, COALESCE(tenant_name,''), COALESCE(organization_name,''), \
         action, COALESCE(detail,''), COALESCE(ip,''), created_at, COALESCE(user_agent,''), request_id, application_id \
         FROM audit_logs WHERE {AUDIT_WHERE}{tf_sql} ORDER BY created_at DESC, id DESC LIMIT 10000"
    ))
    .bind(&kw)
    .bind(&action)
    .bind(scope)
    .bind(tf_val)
    .bind(request_id)
    .bind(application_id)
    .bind(&org_scope)
    .bind(include_unassigned)
    .fetch_all(&state.pool)
    .await?;

    let mut csv = String::from("\u{FEFF}时间,用户,租户,组织,动作,详情,IP,UA,请求编号,应用ID\n");
    for (
        _,
        username,
        _tenant_id,
        tenant_name,
        organization_name,
        act,
        detail,
        ip,
        created_at,
        user_agent,
        request_id,
        application_id,
    ) in rows
    {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{}\n",
            created_at.format("%Y-%m-%d %H:%M:%S"),
            csv_escape(&username),
            csv_escape(&tenant_name),
            csv_escape(&organization_name),
            csv_escape(&act),
            csv_escape(&detail),
            csv_escape(&ip),
            csv_escape(&user_agent),
            request_id
                .map(|value| value.to_string())
                .unwrap_or_default(),
            application_id
                .map(|value| value.to_string())
                .unwrap_or_default(),
        ));
    }
    let filename = format!("audit-logs-{}.csv", Utc::now().format("%Y%m%d-%H%M%S"));
    let mut resp = csv.into_response();
    resp.headers_mut()
        .insert(CONTENT_TYPE, "text/csv; charset=utf-8".parse().unwrap());
    resp.headers_mut().insert(
        CONTENT_DISPOSITION,
        format!("attachment; filename=\"{filename}\"")
            .parse()
            .unwrap(),
    );
    Ok(resp)
}

/// GET /api/audit-logs — 分页 + keyword + action 过滤; 非平台租户仅看本租户审计行
async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:audit")
        .check(&state, &user)
        .await?;
    let (page, size, offset) = paging(&params);
    let kw = params.get("keyword").cloned().unwrap_or_default();
    let action = params.get("action").cloned().unwrap_or_default();
    let scope: Option<Uuid> = if state.is_platform_context(&user).await? {
        None
    } else {
        Some(user.tenant_id)
    };
    let tf = params.get("tenant_id").cloned().unwrap_or_default();
    let (tf_sql, tf_val) = tenant_filter_clause(&tf)?;
    let request_id = correlation_filter(&params, "request_id")?;
    let application_id = correlation_filter(&params, "application_id")?;
    let (org_scope, include_unassigned) = organization_scope(&state, &user).await?;
    let where_clause = format!("{AUDIT_WHERE}{tf_sql}");
    let rows: Vec<AuditRowTuple> = sqlx::query_as(&format!(
        "SELECT id, COALESCE(username,''), tenant_id, COALESCE(tenant_name,''), COALESCE(organization_name,''), \
         action, COALESCE(detail,''), COALESCE(ip,''), created_at, COALESCE(user_agent,''), request_id, application_id \
         FROM audit_logs WHERE {where_clause} ORDER BY created_at DESC, id DESC LIMIT $9 OFFSET $10"
    ))
    .bind(&kw)
    .bind(&action)
    .bind(scope)
    .bind(tf_val)
    .bind(request_id)
    .bind(application_id)
    .bind(&org_scope)
    .bind(include_unassigned)
    .bind(size)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;
    let total: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM audit_logs WHERE {where_clause}"
    ))
    .bind(&kw)
    .bind(&action)
    .bind(scope)
    .bind(tf_val)
    .bind(request_id)
    .bind(application_id)
    .bind(&org_scope)
    .bind(include_unassigned)
    .fetch_one(&state.pool)
    .await?;
    let records: Vec<Value> = rows
        .into_iter()
        .map(|(id, username, tenant_id, tenant_name, organization_name, act, detail, ip, created_at, user_agent, request_id, application_id)| {
            json!({
                "id": id, "username": username, "tenantId": tenant_id, "tenantName": tenant_name,
                "organizationName": organization_name, "action": act, "detail": detail,
                "ip": ip, "createdAt": created_at, "userAgent": user_agent,
                "requestId": request_id, "applicationId": application_id,
            })
        })
        .collect();
    Ok(Json(page_json(records, total, page, size)))
}
