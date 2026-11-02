use std::collections::HashMap;

use axum::extract::State;
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::{CurrentUser, RequirePermission};
use crate::routes::audit::csv_escape;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/monitor/overview", get(overview))
        .route("/api/monitor/system", get(system))
        .route("/api/monitor/tenant-insights", get(tenant_insights))
        .route(
            "/api/monitor/tenant-insights/export",
            get(export_tenant_insights_csv),
        )
}

type RecentLoginRow = (String, String, String, DateTime<Utc>);
type TrendRow = (DateTime<Utc>, i64);
type InsightRow = (Uuid, String, String, String, i64, i64, i64, i64, i64);
type InsightTrendRow = (Uuid, DateTime<Utc>, i64);

fn fill_trend(rows: &[TrendRow]) -> Vec<Value> {
    let mut map: HashMap<String, i64> = HashMap::new();
    for (day, count) in rows {
        map.insert(day.format("%Y-%m-%d").to_string(), *count);
    }
    let mut out: Vec<Value> = Vec::new();
    for i in (0..7).rev() {
        let day = Utc::now() - Duration::days(i);
        let key = day.format("%Y-%m-%d").to_string();
        let count = map.get(&key).copied().unwrap_or(0);
        out.push(json!({"date": day.format("%m-%d").to_string(), "count": count}));
    }
    out
}

/// 租户洞察数据行 (JSON 响应与 CSV 导出共用)
struct TenantInsightData {
    id: Uuid,
    name: String,
    code: String,
    status: String,
    users: i64,
    organizations: i64,
    roles: i64,
    logins7d: i64,
    active_platform_sessions: i64,
    /// 近 7 天 (含今天) 每日 LOGIN 数, 旧 → 新, 缺天补 0
    logins_trend: Vec<i64>,
}

/// 租户洞察公共查询 (JSON 列表与 CSV 导出共用, 口径见 tenant_insights 文档)
///
/// 口径说明:
/// - users: users 表无 tenant_id 列, 按 memberships → organizations.tenant_id 归属去重统计;
/// - organizations / roles: 限定当前组织及下级组织；平台根上下文可统计全平台;
/// - logins7d / logins_trend: audit_logs 自带 tenant_id (LOGIN 审计写入时均携带登录上下文租户);
/// - active_platform_sessions: platform_sessions 中绑定该租户、未撤销且未到期的平台会话。
/// - 所有成员、日志、令牌与趋势均应用同一组织范围，避免租户总计泄漏上级或平级数据。
async fn load_tenant_insights(
    pool: &PgPool,
    tenant_scope: Option<Uuid>,
    organization_scope: Option<&[Uuid]>,
) -> Result<Vec<TenantInsightData>, AppError> {
    let rows: Vec<InsightRow> = sqlx::query_as(
        "SELECT t.id, t.name, COALESCE(t.code, '') AS code, t.status, \
         (SELECT count(DISTINCT m.user_id) FROM memberships m \
           JOIN organizations o ON o.id = m.organization_id WHERE o.tenant_id = t.id \
           AND ($2::uuid[] IS NULL OR o.id = ANY($2))), \
         (SELECT count(*) FROM organizations o WHERE o.tenant_id = t.id AND ($2::uuid[] IS NULL OR o.id = ANY($2))), \
         (SELECT count(*) FROM roles r WHERE r.tenant_id = t.id AND ($2::uuid[] IS NULL OR r.organization_id = ANY($2))), \
         (SELECT count(*) FROM audit_logs a WHERE a.action = 'LOGIN' AND a.tenant_id = t.id \
           AND a.created_at >= date_trunc('day', now()) - interval '6 days' \
           AND ($2::uuid[] IS NULL OR a.organization_id = ANY($2))), \
         (SELECT count(*) FROM platform_sessions ps WHERE ps.revoked_at IS NULL \
           AND ps.expires_at > now() AND ps.tenant_id = t.id \
           AND ($2::uuid[] IS NULL OR ps.organization_id = ANY($2))) \
         FROM tenants t WHERE ($1::uuid IS NULL OR t.id = $1) ORDER BY t.created_at",
    )
    .bind(tenant_scope)
    .bind(organization_scope)
    .fetch_all(pool)
    .await?;
    let tenant_ids: Vec<Uuid> = rows.iter().map(|r| r.0).collect();

    // 近 7 天 (含今天) 每租户每日 LOGIN 数, 缺天在代码侧补 0
    let trend_rows: Vec<InsightTrendRow> = sqlx::query_as(
        "SELECT a.tenant_id, date_trunc('day', a.created_at) AS day, count(*)::bigint \
         FROM audit_logs a WHERE a.action = 'LOGIN' AND a.tenant_id = ANY($1) \
         AND ($2::uuid[] IS NULL OR a.organization_id = ANY($2)) \
         AND a.created_at >= date_trunc('day', now()) - interval '6 days' \
         GROUP BY a.tenant_id, date_trunc('day', a.created_at)",
    )
    .bind(&tenant_ids)
    .bind(organization_scope)
    .fetch_all(pool)
    .await?;
    let mut trend_map: HashMap<(Uuid, String), i64> = HashMap::new();
    for (tid, day, count) in trend_rows {
        trend_map.insert((tid, day.format("%Y-%m-%d").to_string()), count);
    }

    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                name,
                code,
                status,
                users,
                organizations,
                roles,
                logins7d,
                active_platform_sessions,
            )| {
                let logins_trend: Vec<i64> = (0..7)
                    .rev()
                    .map(|i| {
                        let key = (Utc::now() - Duration::days(i))
                            .format("%Y-%m-%d")
                            .to_string();
                        trend_map.get(&(id, key)).copied().unwrap_or(0)
                    })
                    .collect();
                TenantInsightData {
                    id,
                    name,
                    code,
                    status,
                    users,
                    organizations,
                    roles,
                    logins7d,
                    active_platform_sessions,
                    logins_trend,
                }
            },
        )
        .collect())
}

/// GET /api/monitor/tenant-insights — 按租户的洞察统计 (全部租户含禁用);
/// 非平台根上下文仅返回自己租户内当前组织及下级组织的洞察。
async fn tenant_insights(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:monitor")
        .check(&state, &user)
        .await?;
    let tenant_scope = if state.is_platform_context(&user).await? {
        None
    } else {
        Some(user.tenant_id)
    };
    let organization_scope = state.scoped_org_ids(&user).await?;
    let rows =
        load_tenant_insights(&state.pool, tenant_scope, organization_scope.as_deref()).await?;
    let insights: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "tenantId": r.id, "tenantName": r.name, "tenantCode": r.code, "status": r.status,
                "users": r.users, "organizations": r.organizations, "roles": r.roles,
                "logins7d": r.logins7d, "activePlatformSessions": r.active_platform_sessions, "loginsTrend": r.logins_trend,
            })
        })
        .collect();
    Ok(Json(json!({ "insights": insights })))
}

/// GET /api/monitor/tenant-insights/export — 租户洞察 CSV 导出 (带 BOM, Excel 友好)
///
/// 权限与 tenant-insights 一致 (system:monitor); 数据复用 load_tenant_insights 公共查询;
/// 趋势列 7 个数字用分号连接, 避免破坏 CSV 逗号分隔。
async fn export_tenant_insights_csv(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Response, AppError> {
    RequirePermission("system:monitor")
        .check(&state, &user)
        .await?;
    let tenant_scope = if state.is_platform_context(&user).await? {
        None
    } else {
        Some(user.tenant_id)
    };
    let organization_scope = state.scoped_org_ids(&user).await?;
    let rows =
        load_tenant_insights(&state.pool, tenant_scope, organization_scope.as_deref()).await?;
    let mut csv = String::from(
        "\u{FEFF}租户名称,租户编码,状态,用户数,组织数,角色数,近7天登录,活跃平台会话,近7天登录趋势\n",
    );
    for r in &rows {
        // 状态输出中文 (Excel 友好); 非预期状态原样输出
        let status_cn = match r.status.as_str() {
            "ENABLED" => "启用",
            "DISABLED" => "禁用",
            other => other,
        };
        let trend = r
            .logins_trend
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(";");
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{}\n",
            csv_escape(&r.name),
            csv_escape(&r.code),
            status_cn,
            r.users,
            r.organizations,
            r.roles,
            r.logins7d,
            r.active_platform_sessions,
            trend,
        ));
    }
    let filename = format!("tenant-insights-{}.csv", Utc::now().format("%Y%m%d"));
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

/// 读 /proc/self/status 的 VmRSS 行解析 KB → MB (读失败/解析失败返回 0.0, 不 panic)
fn read_rss_mb() -> f64 {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return 0.0;
    };
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb: f64 = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0);
            return kb / 1024.0;
        }
    }
    0.0
}

/// GET /api/monitor/system — 进程 + 数据库系统级监控
///
/// service 段来自本进程 (AppState.started_at / /proc/self/status);
/// database 段来自 PG 系统视图 (pg_stat_activity / pg_class);
/// tables 的 rows 为 pg_class.reltuples 优化器统计估计值 (非实时精确计数, 负值归 0)。
async fn system(State(state): State<AppState>, user: CurrentUser) -> Result<Json<Value>, AppError> {
    RequirePermission("system:monitor")
        .check(&state, &user)
        .await?;
    state.require_platform(&user).await?;

    let uptime_seconds = (Utc::now() - state.started_at).num_seconds().max(0);
    let branding = crate::platform_settings::load_branding(&state.pool).await?;
    let service = json!({
        "name": branding.platform_name,
        "version": env!("CARGO_PKG_VERSION"),
        "pid": std::process::id(),
        "startedAt": state.started_at,
        "uptimeSeconds": uptime_seconds,
        "memoryRssMb": read_rss_mb(),
    });

    let (product, server_started_at, db_size): (String, DateTime<Utc>, i64) = sqlx::query_as(
        "SELECT version(), pg_postmaster_start_time(), pg_database_size(current_database())",
    )
    .fetch_one(&state.pool)
    .await?;

    // 连接统计排除本条查询自身的连接 (pg_backend_pid)
    let (total, active, idle): (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*)::bigint, \
         count(*) FILTER (WHERE state = 'active')::bigint, \
         count(*) FILTER (WHERE state = 'idle')::bigint \
         FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid()",
    )
    .fetch_one(&state.pool)
    .await?;

    let tables: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT t.name, \
         GREATEST(COALESCE(c.reltuples, 0), 0)::bigint AS rows, \
         COALESCE(pg_total_relation_size(c.oid), 0)::bigint AS size_bytes \
         FROM (VALUES (1,'tenants'),(2,'organizations'),(3,'users'),(4,'roles'),(5,'menus'), \
               (6,'applications'),(7,'audit_logs'),(8,'notifications'),(9,'platform_sessions'),(10,'platform_app_capabilities')) AS t(ord, name) \
         LEFT JOIN pg_class c ON c.relname = t.name AND c.relkind = 'r' \
             AND c.relnamespace = (SELECT oid FROM pg_namespace WHERE nspname = current_schema()) \
         ORDER BY t.ord",
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(json!({
        "service": service,
        "database": {
            "product": product,
            "serverStartedAt": server_started_at,
            "databaseSizeBytes": db_size,
            "connections": { "total": total, "active": active, "idle": idle },
            "poolSize": state.pool.size(),
            "tables": tables.into_iter().map(|(name, rows, size_bytes)| {
                json!({ "name": name, "rows": rows, "sizeBytes": size_bytes })
            }).collect::<Vec<_>>(),
        },
    })))
}

/// GET /api/monitor/overview — 平台总览
async fn overview(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:monitor")
        .check(&state, &user)
        .await?;
    state.require_platform(&user).await?;

    let stats = sqlx::query_as::<_, (
        i64, i64, i64, i64, i64, i64, i64, i64, i64,
    )>(
        "SELECT \
         (SELECT count(*) FROM tenants), \
         (SELECT count(*) FROM organizations), \
         (SELECT count(*) FROM users), \
         (SELECT count(*) FROM roles), \
         (SELECT count(*) FROM applications), \
         (SELECT count(*) FROM menus), \
         (SELECT count(*) FROM platform_app_capabilities), \
         (SELECT count(*) FROM platform_sessions WHERE revoked_at IS NULL AND expires_at > now()), \
         (SELECT count(*) FROM audit_logs WHERE action = 'LOGIN' AND created_at >= date_trunc('day', now()))",
    )
    .fetch_one(&state.pool)
    .await?;
    let (
        tenant_count,
        organization_count,
        user_count,
        role_count,
        application_count,
        menu_count,
        internal_capability_count,
        active_platform_session_count,
        today_login_count,
    ) = stats;

    let recent: Vec<RecentLoginRow> = sqlx::query_as(
        "SELECT COALESCE(username,''), COALESCE(organization_name,''), COALESCE(ip,''), created_at \
         FROM audit_logs WHERE action = 'LOGIN' ORDER BY created_at DESC LIMIT 5",
    )
    .fetch_all(&state.pool)
    .await?;
    let recent_logins: Vec<Value> = recent
        .into_iter()
        .map(|(username, organization_name, ip, created_at)| {
            json!({"username": username, "organizationName": organization_name, "ip": ip, "createdAt": created_at})
        })
        .collect();

    let login_rows: Vec<TrendRow> = sqlx::query_as(
        "SELECT date_trunc('day', created_at) AS day, count(*) FROM audit_logs \
         WHERE action = 'LOGIN' AND created_at >= date_trunc('day', now()) - interval '6 days' \
         GROUP BY date_trunc('day', created_at) ORDER BY day",
    )
    .fetch_all(&state.pool)
    .await?;
    let audit_rows: Vec<TrendRow> = sqlx::query_as(
        "SELECT date_trunc('day', created_at) AS day, count(*) FROM audit_logs \
         WHERE created_at >= date_trunc('day', now()) - interval '6 days' \
         GROUP BY date_trunc('day', created_at) ORDER BY day",
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(json!({
        "tenantCount": tenant_count,
        "organizationCount": organization_count,
        "userCount": user_count,
        "roleCount": role_count,
        "applicationCount": application_count,
        "menuCount": menu_count,
        "internalCapabilityCount": internal_capability_count,
        "activePlatformSessionCount": active_platform_session_count,
        "todayLoginCount": today_login_count,
        "recentLogins": recent_logins,
        "loginTrend7d": fill_trend(&login_rows),
        "auditTrend7d": fill_trend(&audit_rows),
    })))
}
