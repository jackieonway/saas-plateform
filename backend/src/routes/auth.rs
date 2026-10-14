use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
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

#[derive(sqlx::FromRow)]
struct LoginUserRow {
    id: Uuid,
    username: String,
    password_hash: String,
    status: String,
    session_version: i64,
    real_name: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CredentialAccount {
    user_id: Uuid,
    version: i64,
}

#[derive(Serialize, Deserialize)]
struct CredentialSelection {
    #[serde(flatten)]
    claims: jwt::Claims,
    accounts: Vec<CredentialAccount>,
}

/// Password proof precedes tenant/organization selection, including shared aliases.
async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<LoginBody>,
) -> Result<Json<Value>, AppError> {
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let identifier = body.username.trim().to_string();
    if identifier.is_empty() || body.password.is_empty() {
        return Err(AppError::bad_request("登录账号与密码不能为空"));
    }
    let phone = crate::routes::contact::phone_identifier(&identifier);
    let email = if identifier.contains('@') { identifier.to_lowercase() } else { String::new() };
    if phone.is_empty() && email.is_empty() && login_locked(&identifier, &ip) {
        return Err(AppError::forbidden("登录尝试过于频繁, 请 10 分钟后再试"));
    }
    let rows: Vec<LoginUserRow> = sqlx::query_as(
        "SELECT id, username, password_hash, status, session_version, COALESCE(real_name,'') AS real_name \
         FROM users WHERE username=$1 OR ($2<>'' AND regexp_replace(phone, '[[:space:]()\\-]', '', 'g')=$2) \
         OR ($3<>'' AND lower(email)=$3) ORDER BY id LIMIT 65",
    ).bind(&identifier).bind(phone).bind(email).fetch_all(&state.pool).await?;
    if rows.len() > 64 {
        return Err(AppError::bad_request("该电话或 Email 关联账号过多，请使用其他登录账号"));
    }
    if rows.is_empty() {
        record_login_failure(&identifier, &ip);
        audit::write(&state.pool, audit::AuditInfo {
            user_id: None, username: &identifier, tenant_id: None, organization_id: None,
            action: "LOGIN_FAILED", detail: "用户不存在", ip: &ip, user_agent: &ua,
        }).await;
        return Err(AppError::unauthorized("用户名或密码错误"));
    }
    let mut accounts = Vec::new();
    let mut choices = Vec::new();
    let mut locked = false;
    let mut authenticated_without_membership = false;
    let mut failures = Vec::new();
    for row in rows {
        if login_locked(&row.username, &ip) { locked = true; continue; }
        if row.status != "ENABLED" { continue; }
        let hash = row.password_hash;
        let secret = body.password.clone();
        let valid = tokio::task::spawn_blocking(move || pw::verify_password(&hash, &secret))
            .await.map_err(|_| anyhow::anyhow!("password verification worker failed"))?;
        if !valid {
            failures.push((row.id, row.username));
            continue;
        }
        clear_login_failures(&row.username, &ip);
        let memberships = fetch_memberships(&state.pool, row.id).await?;
        if memberships.is_empty() { authenticated_without_membership = true; continue; }
        for membership in &memberships {
            let mut choice = membership_json(membership);
            choice["userId"] = json!(row.id);
            choice["username"] = json!(row.username);
            choice["realName"] = json!(row.real_name);
            choices.push(choice);
        }
        sqlx::query("UPDATE users SET last_login_at=now(), updated_at=now() WHERE id=$1")
            .bind(row.id).execute(&state.pool).await?;
        let first = &memberships[0];
        audit::write(&state.pool, audit::AuditInfo {
            user_id: Some(row.id), username: &row.username, tenant_id: Some(first.tenant_id), organization_id: Some(first.organization_id),
            action: "LOGIN", detail: "登录成功, 签发登录票据", ip: &ip, user_agent: &ua,
        }).await;
        accounts.push(CredentialAccount { user_id: row.id, version: row.session_version });
    }
    let Some(first) = accounts.first() else {
        // A successful shared-alias login must not count as a failed attempt
        // against the other accounts using that alias.
        if !authenticated_without_membership {
            for (id, username) in failures {
                record_login_failure(&username, &ip);
                audit::write(&state.pool, audit::AuditInfo {
                    user_id: Some(id), username: &username, tenant_id: None, organization_id: None,
                    action: "LOGIN_FAILED", detail: "密码错误", ip: &ip, user_agent: &ua,
                }).await;
            }
        }
        if locked { return Err(AppError::forbidden("登录尝试过于频繁, 请 10 分钟后再试")); }
        if authenticated_without_membership {
            return Err(AppError::forbidden("所属租户或组织已到期, 无法登录"));
        }
        return Err(AppError::unauthorized("用户名或密码错误"));
    };
    let now = Utc::now().timestamp();
    let account_count = accounts.len();
    let claims = jwt::Claims {
        sub: first.user_id.to_string(),
        typ: if account_count > 1 { "credential_multi" } else { "credential" }.into(),
        tenant: None, org: None, cid: None, scope: None, sid: None,
        ver: Some(first.version), aud: None, jti: None,
        exp: now + state.config.credential_ticket_ttl_minutes * 60, iat: now,
    };
    let ticket = if account_count == 1 {
        jwt::sign(&claims, &state.config.jwt_secret)?
    } else {
        jwt::sign_payload(&CredentialSelection { claims, accounts }, &state.config.jwt_secret)?
    };
    Ok(Json(json!({ "ticket": ticket, "memberships": choices, "accountCount": account_count })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextBody {
    organization_id: String,
    #[serde(default)]
    user_id: Option<Uuid>,
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
    if claims.typ != "credential" && claims.typ != "credential_multi" && claims.typ != "session" {
        return Err(AppError::unauthorized("凭证类型错误, 需要 ticket 或会话令牌"));
    }
    let org_id: Uuid = body.organization_id.parse()
        .map_err(|_| AppError::bad_request("组织 ID 格式错误"))?;
    let (user_id, claim_version) = if claims.typ == "credential_multi" {
        let selection: CredentialSelection = jwt::verify_payload(&token, &state.config.jwt_secret)
            .map_err(|_| AppError::unauthorized("登录选择凭证无效或已过期"))?;
        let selected = if let Some(selected) = body.user_id { selected } else {
            let ids: Vec<Uuid> = selection.accounts.iter().map(|account| account.user_id).collect();
            let candidates: Vec<Uuid> = sqlx::query_scalar(
                "SELECT user_id FROM memberships WHERE organization_id=$1 AND user_id=ANY($2)",
            ).bind(org_id).bind(ids).fetch_all(&state.pool).await?;
            if candidates.len() != 1 { return Err(AppError::bad_request("请选择要进入该组织的账号")); }
            candidates[0]
        };
        let account = selection.accounts.iter().find(|account| account.user_id == selected)
            .ok_or_else(|| AppError::forbidden("所选账号未通过本次登录验证"))?;
        (account.user_id, account.version)
    } else {
        let id: Uuid = claims.sub.parse().map_err(|_| AppError::unauthorized("凭证无效"))?;
        if body.user_id.is_some_and(|selected| selected != id) {
            return Err(AppError::forbidden("不能通过当前凭证切换用户账号"));
        }
        (id, claims.ver.ok_or_else(|| AppError::unauthorized("旧版凭证已失效，请重新登录"))?)
    };
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

    let org = sqlx::query_as::<_, (Uuid, String, String, Uuid, String, String, bool, bool, Option<DateTime<Utc>>, Option<DateTime<Utc>>)>(
        "SELECT o.id, o.name, org_path(o.id), o.tenant_id, t.name, t.code, t.is_platform, o.parent_id IS NULL, org_chain_expires_at(o.id), t.expires_at \
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
    let (org_id, org_name, org_path, tenant_id, tenant_name, tenant_code, tenant_is_platform, org_is_root, org_expires, tenant_expires) = org;
    // 会话与刷新均受组织祖先链及租户有效期约束。
    if org_expires.is_some_and(|e| e <= Utc::now()) {
        return Err(AppError::forbidden("该组织已到期, 无法切换"));
    }
    if tenant_expires.is_some_and(|e| e <= Utc::now()) {
        return Err(AppError::forbidden("所属租户已到期, 无法登录"));
    }

    let now = Utc::now().timestamp();
    let ttl = state.config.session_ttl_hours * 3600;
    let refresh_ttl = state.config.refresh_token_ttl_seconds;
    if ttl <= 0 || refresh_ttl <= 0 {
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
    .bind(user_id).bind(tenant_id).bind(org_id).bind(ttl.max(refresh_ttl) as f64).bind(&ip).bind(&ua)
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
    let access_expiry = (now + ttl).min(expires_at.timestamp());
    let refresh_expiry = DateTime::from_timestamp((now + refresh_ttl).min(expires_at.timestamp()), 0)
        .ok_or_else(|| AppError::bad_request("刷新令牌有效期配置无效"))?;
    if access_expiry <= now || refresh_expiry <= Utc::now() {
        return Err(AppError::forbidden("租户或组织有效期不足，无法签发会话"));
    }
    let refresh_token = session::issue_refresh_in_tx(&mut tx, session_id, session_version, refresh_expiry).await?;
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
        exp: access_expiry,
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
        "expiresIn": access_expiry - now,
        "refreshToken": refresh_token,
        "refreshExpiresIn": refresh_expiry.timestamp() - now,
        "platformContext": tenant_is_platform && org_is_root,
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

    let apps = sqlx::query_as::<_, (Uuid, String, String, String, String, String, String, Option<String>, Option<Uuid>, String)>(
        "SELECT a.id, a.code, a.name, COALESCE(a.icon,''), COALESCE(a.base_path,''), \
         COALESCE(r.entry_type,a.entry_type,'ESM'), COALESCE(r.entry_url,a.entry_url,''), r.version, r.id, a.runtime_mode \
         FROM applications a \
         JOIN tenant_applications ta ON ta.application_id = a.id AND ta.tenant_id = $1 \
         JOIN organization_applications oa ON oa.application_id = a.id AND oa.organization_id = $2 \
         LEFT JOIN application_deployments d ON d.application_id=a.id AND d.environment=$3 \
         LEFT JOIN application_releases r ON r.application_id=a.id AND r.environment=d.environment AND r.id=d.active_release_id \
         WHERE a.status = 'ENABLED' ORDER BY a.sort_order, a.created_at",
    )
    .bind(user.tenant_id)
    .bind(user.org_id)
    .bind(&state.config.application_environment)
    .fetch_all(&state.pool)
    .await?;
    let apps_json: Vec<Value> = apps
        .into_iter()
        .map(|(id, code, name, icon, base_path, entry_type, entry_url, deployment_version, entry_version_id, runtime_mode)| {
            json!({"id": id, "code": code, "name": name, "icon": icon, "basePath": base_path,
                   "entryType": entry_type, "entryUrl": entry_url,
                   "deploymentVersion": deployment_version, "entryVersionId": entry_version_id,
                   "runtimeMode": runtime_mode})
        })
        .collect();

    let memberships = fetch_memberships(&state.pool, user.user_id).await?;
    let platform_context = state.can_manage_platform(&user).await?;

    Ok(Json(json!({
        "platformContext": platform_context,
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
        authorization::granted_menu_rows(
            &state.pool, user.tenant_id, user.org_id, Some(app_id),
        ).await?
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
    let mut perms: Vec<String> = state.context_permissions(&user).await?.into_iter().collect();
    perms.sort();
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
    pw::validate_for_config(&body.new_password, &state.config)?;
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
    let platform_sessions = session::device_sessions(
        &state.pool, Some(user.user_id), user.user_id, user.session_id,
    ).await?;
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
