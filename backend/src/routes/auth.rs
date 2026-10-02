use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::AppError;
use crate::authorization;
use crate::extractors::{bearer_token, ApiJson, CurrentUser};
use crate::routes::page_json;
use crate::session;
use crate::state::AppState;
use crate::util::{audit, jwt, password as pw};

/// 登录限流: 同一 用户名+IP 在 10 分钟窗口内最多 5 次失败尝试 (内存态, 进程重启即清零)
static LOGIN_ATTEMPTS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, (u32, i64)>>,
> = std::sync::OnceLock::new();
const LOGIN_WINDOW_SECS: i64 = 600;
const LOGIN_MAX_FAILURES: u32 = 5;

fn attempts_map() -> &'static std::sync::Mutex<std::collections::HashMap<String, (u32, i64)>> {
    LOGIN_ATTEMPTS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn attempt_key(username: &str, ip: &str) -> String {
    format!("{username}|{ip}")
}

fn login_locked(username: &str, ip: &str) -> bool {
    let map = attempts_map();
    let guard = match map.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    let now = Utc::now().timestamp();
    match guard.get(&attempt_key(username, ip)) {
        Some((count, window_start)) => {
            now - window_start <= LOGIN_WINDOW_SECS && *count >= LOGIN_MAX_FAILURES
        }
        None => false,
    }
}

fn record_login_failure(username: &str, ip: &str) {
    let map = attempts_map();
    let Ok(mut guard) = map.lock() else { return };
    let now = Utc::now().timestamp();
    let key = attempt_key(username, ip);
    let entry = guard.entry(key).or_insert((0, now));
    if now - entry.1 > LOGIN_WINDOW_SECS {
        *entry = (0, now);
    }
    entry.0 += 1;
}

fn clear_login_failures(username: &str, ip: &str) {
    let map = attempts_map();
    if let Ok(mut guard) = map.lock() {
        guard.remove(&attempt_key(username, ip));
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/auth/login", post(login))
        .route("/api/auth/context", post(context))
        .route("/api/auth/profile", get(profile))
        .route("/api/auth/menus", get(menus))
        .route("/api/auth/permissions", get(permissions))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/password", put(change_password))
        .route("/api/auth/login-history", get(login_history))
        .route("/api/auth/my-sessions", get(my_sessions))
        .route("/api/auth/sessions/{id}", axum::routing::delete(revoke_my_session))
}

#[derive(sqlx::FromRow)]
struct MembershipRow {
    organization_id: Uuid,
    organization_name: String,
    organization_path: Option<String>,
    tenant_id: Uuid,
    tenant_name: String,
    department_id: Option<Uuid>,
    department_name: Option<String>,
}

async fn fetch_memberships(pool: &sqlx::PgPool, user_id: Uuid) -> Result<Vec<MembershipRow>, AppError> {
    let rows = sqlx::query_as::<_, MembershipRow>(
        "SELECT m.organization_id, o.name AS organization_name, org_path(o.id) AS organization_path, \
         o.tenant_id, t.name AS tenant_name, m.department_id, d.name AS department_name \
         FROM memberships m \
         JOIN organizations o ON o.id = m.organization_id \
         JOIN tenants t ON t.id = o.tenant_id \
         LEFT JOIN departments d ON d.id = m.department_id \
         WHERE m.user_id = $1 AND org_chain_active(o.id) AND t.status = 'ENABLED' \
         AND (o.expires_at IS NULL OR o.expires_at > now()) \
         AND (t.expires_at IS NULL OR t.expires_at > now()) \
         ORDER BY m.created_at",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

fn membership_json(m: &MembershipRow) -> Value {
    json!({
        "organizationId": m.organization_id,
        "organizationName": m.organization_name,
        "organizationPath": m.organization_path.clone().unwrap_or_default(),
        "tenantId": m.tenant_id,
        "tenantName": m.tenant_name,
        "departmentId": m.department_id,
        "departmentName": m.department_name,
    })
}

#[derive(Deserialize)]
struct LoginBody {
    username: String,
    password: String,
}

/// POST /api/auth/login — 校验凭证 → memberships + credential ticket (10min)
async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<LoginBody>,
) -> Result<Json<Value>, AppError> {
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let username = body.username.trim().to_string();
    if username.is_empty() || body.password.is_empty() {
        return Err(AppError::bad_request("用户名与密码不能为空"));
    }
    // 登录限流: 失败次数过多直接拒绝
    if login_locked(&username, &ip) {
        audit::write(&state.pool, audit::AuditInfo {
            user_id: None, username: &username, tenant_id: None, organization_id: None,
            action: "LOGIN_FAILED", detail: "尝试过于频繁被限流", ip: &ip, user_agent: &ua,
        }).await;
        return Err(AppError::forbidden("登录尝试过于频繁, 请 10 分钟后再试"));
    }
    let row = sqlx::query_as::<_, (Uuid, String, String, bool, i64)>(
        "SELECT id, password_hash, status, is_super_admin, session_version FROM users WHERE username = $1",
    )
    .bind(&username)
    .fetch_optional(&state.pool)
    .await?;

    let Some((user_id, hash, status, _, session_version)) = row else {
        record_login_failure(&username, &ip);
        audit::write(&state.pool, audit::AuditInfo {
            user_id: None, username: &username, tenant_id: None, organization_id: None,
            action: "LOGIN_FAILED", detail: "用户不存在", ip: &ip, user_agent: &ua,
        }).await;
        return Err(AppError::unauthorized("用户名或密码错误"));
    };
    if status != "ENABLED" {
        audit::write(&state.pool, audit::AuditInfo {
            user_id: Some(user_id), username: &username, tenant_id: None, organization_id: None,
            action: "LOGIN_FAILED", detail: "账号已禁用", ip: &ip, user_agent: &ua,
        }).await;
        return Err(AppError::unauthorized("账号已被禁用"));
    }
    if !pw::verify_password(&hash, &body.password) {
        record_login_failure(&username, &ip);
        audit::write(&state.pool, audit::AuditInfo {
            user_id: Some(user_id), username: &username, tenant_id: None, organization_id: None,
            action: "LOGIN_FAILED", detail: "密码错误", ip: &ip, user_agent: &ua,
        }).await;
        return Err(AppError::unauthorized("用户名或密码错误"));
    }
    clear_login_failures(&username, &ip);

    let memberships = fetch_memberships(&state.pool, user_id).await?;
    // 登录拦截: 过期过滤后无任何可用组织上下文 → 拒绝登录 (在签发 ticket 之前)
    if memberships.is_empty() {
        audit::write(&state.pool, audit::AuditInfo {
            user_id: Some(user_id), username: &username, tenant_id: None, organization_id: None,
            action: "LOGIN_FAILED", detail: "所属租户或组织已到期, 无法登录", ip: &ip, user_agent: &ua,
        }).await;
        return Err(AppError::forbidden("所属租户或组织已到期, 无法登录"));
    }
    let now = Utc::now().timestamp();
    let claims = jwt::Claims {
        sub: user_id.to_string(),
        typ: "credential".into(),
        tenant: None,
        org: None,
        cid: None,
        scope: None,
        sid: None,
        ver: Some(session_version),
        aud: None,
        jti: None,
        exp: now + state.config.credential_ticket_ttl_minutes * 60,
        iat: now,
    };
    let ticket = jwt::sign(&claims, &state.config.jwt_secret)?;

    sqlx::query("UPDATE users SET last_login_at = now(), updated_at = now() WHERE id = $1")
        .bind(user_id)
        .execute(&state.pool)
        .await?;

    let (t_id, o_id) = memberships
        .first()
        .map(|m| (Some(m.tenant_id), Some(m.organization_id)))
        .unwrap_or((None, None));
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user_id), username: &username, tenant_id: t_id, organization_id: o_id,
        action: "LOGIN", detail: "登录成功, 签发登录票据", ip: &ip, user_agent: &ua,
    }).await;

    Ok(Json(json!({
        "ticket": ticket,
        "memberships": memberships.iter().map(membership_json).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextBody {
    organization_id: String,
}

/// POST /api/auth/context — ticket (首次选择组织) 或 session (切换组织) → 会话 JWT
async fn context(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<ContextBody>,
) -> Result<Json<Value>, AppError> {
    let token = bearer_token(&headers)?;
    let claims = jwt::verify(&token, &state.config.jwt_secret)
        .map_err(|_| AppError::unauthorized("凭证无效或已过期"))?;
    if claims.typ != "credential" && claims.typ != "session" {
        return Err(AppError::unauthorized("凭证类型错误, 需要 ticket 或会话令牌"));
    }
    let user_id: Uuid = claims
        .sub
        .parse()
        .map_err(|_| AppError::unauthorized("凭证无效"))?;
    let claim_version = claims.ver.ok_or_else(|| AppError::unauthorized("旧版凭证已失效，请重新登录"))?;
    let old_session_id = if claims.typ == "session" {
        let sid: Uuid = claims.sid.as_deref()
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or_else(|| AppError::unauthorized("旧版会话已失效，请重新登录"))?;
        let old_tenant = claims.tenant.as_deref().and_then(|value| Uuid::parse_str(value).ok())
            .ok_or_else(|| AppError::unauthorized("会话缺少租户上下文"))?;
        let old_org = claims.org.as_deref().and_then(|value| Uuid::parse_str(value).ok())
            .ok_or_else(|| AppError::unauthorized("会话缺少组织上下文"))?;
        session::active_user(&state.pool, user_id, old_tenant, old_org, sid, claim_version).await?;
        Some(sid)
    } else {
        None
    };
    let org_id: Uuid = body
        .organization_id
        .parse()
        .map_err(|_| AppError::bad_request("组织 ID 格式错误"))?;

    let user = sqlx::query_as::<_, (String, String, String, String, String, String, bool, String, i64)>(
        "SELECT username, COALESCE(real_name,''), COALESCE(email,''), COALESCE(phone,''), \
         COALESCE(avatar,''), COALESCE(gender,'UNKNOWN'), is_super_admin, status, session_version FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::unauthorized("用户不存在"))?;
    let (username, real_name, email, phone, avatar, gender, is_super, status, session_version) = user;
    if status != "ENABLED" {
        return Err(AppError::unauthorized("账号已被禁用"));
    }
    if session_version != claim_version {
        return Err(AppError::unauthorized("凭证已撤销，请重新登录"));
    }

    let org = sqlx::query_as::<_, (Uuid, String, String, Uuid, String, String, bool, Option<DateTime<Utc>>, Option<DateTime<Utc>>)>(
        "SELECT o.id, o.name, org_path(o.id), o.tenant_id, t.name, t.code, t.is_platform, org_chain_expires_at(o.id), t.expires_at \
         FROM memberships m \
         JOIN organizations o ON o.id = m.organization_id \
         JOIN tenants t ON t.id = o.tenant_id \
         WHERE m.user_id = $1 AND m.organization_id = $2 \
         AND org_chain_active(o.id) AND t.status = 'ENABLED'",
    )
    .bind(user_id)
    .bind(org_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::forbidden("您在该组织没有成员身份"))?;
    let (org_id, org_name, org_path, tenant_id, tenant_name, tenant_code, tenant_is_platform, org_expires, tenant_expires) = org;
    // 过期拦截: 选定的组织或其租户已到期 → 拒绝签发会话令牌 (会话内不逐请求校验, 到期后重新登录/切换即被拦)
    if org_expires.is_some_and(|e| e <= Utc::now()) {
        return Err(AppError::forbidden("该组织已到期, 无法切换"));
    }
    if tenant_expires.is_some_and(|e| e <= Utc::now()) {
        return Err(AppError::forbidden("所属租户已到期, 无法登录"));
    }

    let now = Utc::now().timestamp();
    let ttl = state.config.session_ttl_hours * 3600;
    if ttl <= 0 {
        return Err(AppError::bad_request("会话有效期配置无效"));
    }
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let mut tx = state.pool.begin().await?;
    let (session_id, expires_at): (Uuid, DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO platform_sessions \
         (user_id, tenant_id, organization_id, expires_at, ip, user_agent) \
         VALUES ($1, $2, $3, LEAST(now() + make_interval(secs => $4::float8), \
                $7::timestamptz, $8::timestamptz), $5, $6) \
         RETURNING id, expires_at",
    )
    .bind(user_id).bind(tenant_id).bind(org_id).bind(ttl as f64).bind(&ip).bind(&ua)
    .bind(org_expires).bind(tenant_expires)
    .fetch_one(&mut *tx).await?;
    if let Some(old_id) = old_session_id {
        let revoked = sqlx::query(
            "UPDATE platform_sessions SET revoked_at = now() \
             WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL AND expires_at > now()",
        ).bind(old_id).bind(user_id).execute(&mut *tx).await?.rows_affected();
        if revoked != 1 {
            return Err(AppError::unauthorized("会话已失效，请重新登录"));
        }
    }
    let session = jwt::Claims {
        sub: user_id.to_string(),
        typ: "session".into(),
        tenant: Some(tenant_id.to_string()),
        org: Some(org_id.to_string()),
        cid: None,
        scope: None,
        sid: Some(session_id.to_string()),
        ver: Some(session_version),
        aud: None,
        jti: None,
        exp: expires_at.timestamp(),
        iat: now,
    };
    let access_token = jwt::sign(&session, &state.config.jwt_secret)?;
    tx.commit().await?;

    if claims.typ == "session" {
        audit::write(&state.pool, audit::AuditInfo {
            user_id: Some(user_id), username: &username,
            tenant_id: Some(tenant_id), organization_id: Some(org_id),
            action: "CONTEXT_SWITCH", detail: &format!("切换组织上下文: {}", org_name), ip: &ip, user_agent: &ua,
        }).await;
    }

    Ok(Json(json!({
        "accessToken": access_token,
        "expiresIn": ttl,
        "user": {
            "id": user_id, "username": username, "realName": real_name, "email": email,
            "phone": phone, "avatar": avatar, "gender": gender, "isSuperAdmin": is_super,
        },
        "tenant": {"id": tenant_id, "name": tenant_name, "code": tenant_code, "isPlatform": tenant_is_platform},
        "organization": {"id": org_id, "name": org_name, "path": org_path},
    })))
}

/// GET /api/auth/profile
async fn profile(State(state): State<AppState>, user: CurrentUser) -> Result<Json<Value>, AppError> {
    let u = sqlx::query_as::<_, (String, String, String, String, String, String, bool)>(
        "SELECT username, COALESCE(real_name,''), COALESCE(email,''), COALESCE(phone,''), \
         COALESCE(avatar,''), COALESCE(gender,'UNKNOWN'), is_super_admin FROM users WHERE id = $1",
    )
    .bind(user.user_id)
    .fetch_one(&state.pool)
    .await?;
    let (username, real_name, email, phone, avatar, gender, is_super) = u;

    let tenant = sqlx::query_as::<_, (Uuid, String, String, bool)>(
        "SELECT id, name, COALESCE(code,''), is_platform FROM tenants WHERE id = $1",
    )
    .bind(user.tenant_id)
    .fetch_one(&state.pool)
    .await?;

    let org = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT id, name, org_path(id) FROM organizations WHERE id = $1",
    )
    .bind(user.org_id)
    .fetch_one(&state.pool)
    .await?;

    let apps = sqlx::query_as::<_, (Uuid, String, String, String, String, String, String)>(
        "SELECT a.id, a.code, a.name, COALESCE(a.icon,''), COALESCE(a.base_path,''), \
         COALESCE(a.entry_type,'ESM'), COALESCE(a.entry_url,'') \
         FROM applications a \
         JOIN tenant_applications ta ON ta.application_id = a.id AND ta.tenant_id = $1 \
         JOIN organization_applications oa ON oa.application_id = a.id AND oa.organization_id = $2 \
         WHERE a.status = 'ENABLED' ORDER BY a.sort_order, a.created_at",
    )
    .bind(user.tenant_id)
    .bind(user.org_id)
    .fetch_all(&state.pool)
    .await?;
    let apps_json: Vec<Value> = apps
        .into_iter()
        .map(|(id, code, name, icon, base_path, entry_type, entry_url)| {
            json!({"id": id, "code": code, "name": name, "icon": icon, "basePath": base_path,
                   "entryType": entry_type, "entryUrl": entry_url})
        })
        .collect();

    let memberships = fetch_memberships(&state.pool, user.user_id).await?;

    Ok(Json(json!({
        "user": {
            "id": user.user_id, "username": username, "realName": real_name, "email": email,
            "phone": phone, "avatar": avatar, "gender": gender, "isSuperAdmin": is_super,
        },
        "tenant": {"id": tenant.0, "name": tenant.1, "code": tenant.2, "isPlatform": tenant.3},
        "organization": {"id": org.0, "name": org.1, "path": org.2},
        "applications": apps_json,
        "memberships": memberships.iter().map(membership_json).collect::<Vec<_>>(),
    })))
}

/// GET /api/auth/menus?applicationId= — 角色 ∩ 组织授权 ∩ 租户授权 的可见菜单树
async fn menus(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    let app_id: Uuid = params
        .get("applicationId")
        .and_then(|v| Uuid::parse_str(v).ok())
        .ok_or_else(|| AppError::bad_request("缺少 applicationId 参数"))?;

    let rows = if user.is_super_admin {
        authorization::all_app_menu_rows(&state.pool, app_id).await?
    } else {
        authorization::effective_menu_rows(
            &state.pool, user.user_id, user.tenant_id, user.org_id, Some(app_id),
        ).await?
    };
    Ok(Json(json!({"menus": authorization::menu_tree(rows)})))
}

/// GET /api/auth/permissions
async fn permissions(State(state): State<AppState>, user: CurrentUser) -> Result<Json<Value>, AppError> {
    let roles = state.effective_roles(user.user_id, user.org_id).await?;
    let perms: Vec<String> = if user.is_super_admin {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT permission FROM menus WHERE status = 'ENABLED' AND permission <> '' ORDER BY permission",
        )
        .fetch_all(&state.pool)
        .await?;
        rows.into_iter().map(|(p,)| p).collect()
    } else {
        let mut v: Vec<String> = state.permissions_of(user.user_id, user.org_id).await?.into_iter().collect();
        v.sort();
        v
    };
    Ok(Json(json!({"permissions": perms, "roles": roles})))
}

/// POST /api/auth/logout
async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    session::revoke_one(&state.pool, user.user_id, user.session_id).await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "LOGOUT", detail: "退出登录", ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PasswordBody {
    old_password: String,
    new_password: String,
}

/// PUT /api/auth/password
async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<PasswordBody>,
) -> Result<Json<Value>, AppError> {
    if body.new_password.len() < 6 {
        return Err(AppError::bad_request("新密码长度不能少于 6 位"));
    }
    let (hash,): (String,) = sqlx::query_as("SELECT password_hash FROM users WHERE id = $1")
        .bind(user.user_id)
        .fetch_one(&state.pool)
        .await?;
    if !pw::verify_password(&hash, &body.old_password) {
        return Err(AppError::bad_request("原密码不正确"));
    }
    let new_hash = pw::hash_password(&body.new_password)?;
    session::revoke_all(&state.pool, user.user_id, Some(&new_hash)).await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "UPDATE_PASSWORD", detail: "修改个人密码", ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(json!({"ok": true})))
}

const LOGIN_HISTORY_WHERE: &str = "user_id = $1 AND action IN ('LOGIN', 'LOGIN_FAILED')";

type LoginHistoryTuple = (i64, String, String, String, String, String, DateTime<Utc>);

/// GET /api/auth/login-history — 本人登录历史时间线 (登录成功/失败)
///
/// 个人安全中心自助查询: 仅本人审计行 (user_id = 当前用户), 只含 LOGIN / LOGIN_FAILED。
/// 用户名不存在或被限流等场景写入的 LOGIN_FAILED 行 user_id 为 NULL,
/// 不属于任何用户, 自然不会出现在任何人的历史里 (admin 密码错误的失败行则带有 user_id)。
/// 分页: page ≥ 1 (非法回落 1), size 上限 50 (超出截断, 非法/未传默认 10), 按时间倒序。
async fn login_history(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    let page: i64 = params
        .get("page")
        .and_then(|v| v.parse().ok())
        .filter(|v| *v >= 1)
        .unwrap_or(1);
    // size: 非法/未传 → 默认 10; 超出上限 50 → 截断为 50
    let size: i64 = params
        .get("size")
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|v| *v >= 1)
        .map(|v| v.min(50))
        .unwrap_or(10);
    let offset = (page - 1) * size;

    let rows: Vec<LoginHistoryTuple> = sqlx::query_as(&format!(
        "SELECT id, action, COALESCE(ip,''), COALESCE(detail,''), COALESCE(tenant_name,''), \
         COALESCE(organization_name,''), created_at \
         FROM audit_logs WHERE {LOGIN_HISTORY_WHERE} \
         ORDER BY created_at DESC, id DESC LIMIT $2 OFFSET $3"
    ))
    .bind(user.user_id)
    .bind(size)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;
    let total: i64 =
        sqlx::query_scalar(&format!("SELECT count(*) FROM audit_logs WHERE {LOGIN_HISTORY_WHERE}"))
            .bind(user.user_id)
            .fetch_one(&state.pool)
            .await?;

    let records: Vec<Value> = rows
        .into_iter()
        .map(|(id, action, ip, detail, tenant_name, organization_name, created_at)| {
            json!({
                "id": id, "action": action, "ip": ip, "detail": detail,
                "tenantName": tenant_name, "organizationName": organization_name,
                "createdAt": created_at,
            })
        })
        .collect();
    Ok(Json(page_json(records, total, page, size)))
}

type MySessionTuple = (
    String,
    i64,
    i64,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<String>,
);

type PlatformSessionTuple = (
    Uuid, Uuid, String, Uuid, String, DateTime<Utc>, DateTime<Utc>,
    Option<DateTime<Utc>>, String, String, bool,
);

/// GET /api/auth/my-sessions — 本人 OAuth 令牌按客户端聚合的会话视图
///
/// 个人安全中心自助查询: 仅本人令牌行 (user_id = 当前用户, client_credentials 的
/// user_id NULL 行天然排除)。每客户端一行:
/// - activeAccess / activeRefresh: 未吊销且未过期的 access / refresh 计数
/// - lastIssuedAt: 全历史最近签发时间; lastExpiresAt: 活跃令牌最晚过期时间 (无活跃为 null)
/// - scope: 最近一条 access 令牌的 scope (array_agg 按签发时间倒序取首个, 无 access 行为空串)
/// - revokedTotal (顶层): 本人全部已吊销令牌总数 (无任何令牌时为 0)
async fn my_sessions(State(state): State<AppState>, user: CurrentUser) -> Result<Json<Value>, AppError> {
    let rows: Vec<MySessionTuple> = sqlx::query_as(
        "SELECT client_id, \
         COUNT(*) FILTER (WHERE token_type = 'access' AND revoked = false AND expires_at > now()) AS active_access, \
         COUNT(*) FILTER (WHERE token_type = 'refresh' AND revoked = false AND expires_at > now()) AS active_refresh, \
         max(created_at) AS last_issued_at, \
         max(expires_at) FILTER (WHERE revoked = false AND expires_at > now()) AS last_expires_at, \
         (array_agg(scope ORDER BY created_at DESC) FILTER (WHERE token_type = 'access'))[1] AS scope \
         FROM oauth_tokens WHERE user_id = $1 \
         GROUP BY client_id \
         ORDER BY last_issued_at DESC",
    )
    .bind(user.user_id)
    .fetch_all(&state.pool)
    .await?;
    let revoked_total: i64 =
        sqlx::query_scalar("SELECT count(*) FROM oauth_tokens WHERE user_id = $1 AND revoked = true")
            .bind(user.user_id)
            .fetch_one(&state.pool)
            .await?;

    let sessions: Vec<Value> = rows
        .into_iter()
        .map(
            |(client_id, active_access, active_refresh, last_issued_at, last_expires_at, scope)| {
                json!({
                    "clientId": client_id,
                    "activeAccess": active_access,
                    "activeRefresh": active_refresh,
                    "lastIssuedAt": last_issued_at,
                    "lastExpiresAt": last_expires_at,
                    "scope": scope.unwrap_or_default(),
                })
            },
        )
        .collect();
    let platform_rows: Vec<PlatformSessionTuple> = sqlx::query_as(
        "SELECT s.id, s.tenant_id, t.name, s.organization_id, o.name, \
         s.issued_at, s.expires_at, s.revoked_at, s.ip, s.user_agent, \
         (s.revoked_at IS NULL AND s.expires_at > now() \
          AND t.status = 'ENABLED' AND org_chain_active(o.id) \
          AND (t.expires_at IS NULL OR t.expires_at > now()) \
          AND EXISTS(SELECT 1 FROM memberships m WHERE m.user_id = s.user_id \
                     AND m.organization_id = s.organization_id)) AS active \
         FROM platform_sessions s \
         JOIN tenants t ON t.id = s.tenant_id \
         JOIN organizations o ON o.id = s.organization_id \
         WHERE s.user_id = $1 ORDER BY s.issued_at DESC, s.id DESC LIMIT 100",
    )
    .bind(user.user_id)
    .fetch_all(&state.pool)
    .await?;
    let platform_sessions: Vec<Value> = platform_rows.into_iter()
        .map(|(id, tenant_id, tenant_name, organization_id, organization_name,
              issued_at, expires_at, revoked_at, ip, user_agent, active)| {
            json!({
                "id": id, "tenantId": tenant_id, "tenantName": tenant_name,
                "organizationId": organization_id, "organizationName": organization_name,
                "issuedAt": issued_at, "expiresAt": expires_at, "revokedAt": revoked_at,
                "ip": ip, "userAgent": user_agent, "active": active,
                "isCurrent": id == user.session_id,
            })
        }).collect();
    Ok(Json(json!({"sessions": sessions, "revokedTotal": revoked_total,
                   "platformSessions": platform_sessions})))
}

/// DELETE /api/auth/sessions/{id} — 用户撤销自己的指定设备会话。
async fn revoke_my_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let id: Uuid = id.parse().map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let target: Option<(Uuid, Uuid)> = sqlx::query_as(
        "UPDATE platform_sessions SET revoked_at = now() \
         WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL \
         RETURNING tenant_id, organization_id",
    )
    .bind(id).bind(user.user_id).fetch_optional(&state.pool).await?;
    let (tenant_id, org_id) = target.ok_or_else(|| AppError::not_found("会话不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write(&state.pool, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(tenant_id), organization_id: Some(org_id),
        action: "REVOKE_SESSION", detail: &format!("撤销设备会话: {}", id),
        ip: &ip, user_agent: &ua,
    }).await;
    Ok(Json(json!({"ok": true})))
}
