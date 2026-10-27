use crate::util::uuid7::ParseUuid7;
use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::authorization;
use crate::error::AppError;
use crate::extractors::{ApiJson, CurrentUser};
use crate::routes::page_json;
use crate::session;
use crate::state::AppState;
use crate::util::audit;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/auth/profile", get(profile))
        .route("/api/auth/menus", get(menus))
        .route("/api/auth/permissions", get(permissions))
        .route("/api/auth/password", put(change_password))
        .route("/api/auth/login-history", get(login_history))
        .route("/api/auth/my-sessions", get(my_sessions))
        .route(
            "/api/auth/sessions/{id}",
            axum::routing::delete(revoke_my_session),
        )
}

#[derive(sqlx::FromRow)]
struct MembershipRow {
    membership_id: Uuid,
    user_id: Uuid,
    member_type: String,
    status: String,
    identity_version: i64,
    organization_id: Uuid,
    organization_name: String,
    organization_path: Option<String>,
    tenant_id: Uuid,
    tenant_name: String,
    department_id: Option<Uuid>,
    department_name: Option<String>,
}

async fn fetch_memberships(
    pool: &sqlx::PgPool,
    user_id: Uuid,
) -> Result<Vec<MembershipRow>, AppError> {
    let rows = sqlx::query_as::<_, MembershipRow>(
        "SELECT m.id AS membership_id, m.user_id, m.member_type, m.status, m.identity_version, m.organization_id, o.name AS organization_name, org_path(o.id) AS organization_path, \
         o.tenant_id, t.name AS tenant_name, m.department_id, d.name AS department_name \
         FROM memberships m \
         JOIN organizations o ON o.id = m.organization_id \
         JOIN tenants t ON t.id = o.tenant_id \
         LEFT JOIN departments d ON d.id = m.department_id \
         WHERE m.user_id = $1 AND m.member_type = 'EMPLOYEE' \
         AND org_chain_active(o.id) AND t.status = 'ENABLED' \
         AND (o.expires_at IS NULL OR o.expires_at > now()) \
         AND (t.expires_at IS NULL OR t.expires_at > now()) \
         ORDER BY m.created_at,m.id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await.map_err(|_| AppError::Unavailable)?;
    Ok(rows)
}

fn membership_json(m: &MembershipRow) -> Value {
    let disabled_reason = if m.member_type != "EMPLOYEE" {
        Some("CLIENT_MEMBER_TYPE_NOT_ALLOWED")
    } else if m.status != "ENABLED" {
        Some("MEMBERSHIP_DISABLED")
    } else {
        None
    };
    json!({
        "membershipId": m.membership_id,
        "userId": m.user_id,
        "memberType": m.member_type,
        "status": m.status,
        "identityVersion": m.identity_version.to_string(),
        "consoleSelectable": disabled_reason.is_none(),
        "selectionDisabledReason": disabled_reason,
        "organizationId": m.organization_id,
        "organizationName": m.organization_name,
        "organizationPath": m.organization_path.clone().unwrap_or_default(),
        "tenantId": m.tenant_id,
        "tenantName": m.tenant_name,
        "departmentId": m.department_id,
        "departmentName": m.department_name,
    })
}

/// GET /api/auth/profile
async fn profile(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    let proof = authorization::require_employee_context(&state.pool, &user).await?;
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

    let apps = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            Option<Uuid>,
            String,
        ),
    >(
        "SELECT a.id, a.code, a.name, COALESCE(a.icon,''), COALESCE(a.base_path,''), \
         e.entry_type, e.entry_url, e.version, e.id, e.runtime_mode \
         FROM applications a \
         JOIN tenant_applications ta ON ta.application_id = a.id AND ta.tenant_id = $1 \
         JOIN organization_applications oa ON oa.application_id = a.id AND oa.organization_id = $2 \
         CROSS JOIN LATERAL application_effective_entry(a.id,$3) e \
         WHERE a.status = 'ENABLED' AND 'EMPLOYEE'=ANY(a.allowed_member_types) \
         ORDER BY a.sort_order, a.created_at",
    )
    .bind(user.tenant_id)
    .bind(user.org_id)
    .bind(&state.config.application_environment)
    .fetch_all(&state.pool)
    .await?;
    let apps_json: Vec<Value> = apps
        .into_iter()
        .map(
            |(
                id,
                code,
                name,
                icon,
                base_path,
                entry_type,
                entry_url,
                deployment_version,
                entry_version_id,
                runtime_mode,
            )| {
                json!({"id": id, "code": code, "name": name, "icon": icon, "basePath": base_path,
                   "entryType": entry_type, "entryUrl": entry_url,
                   "deploymentVersion": deployment_version, "entryVersionId": entry_version_id,
                   "runtimeMode": runtime_mode})
            },
        )
        .collect();

    let memberships = fetch_memberships(&state.pool, user.user_id).await?;
    let platform_context = state.can_manage_platform(&user).await?;
    if authorization::require_employee_context(&state.pool, &user).await? != proof {
        return Err(AppError::unauthorized("成员身份已在资料读取期间变更"));
    }

    Ok(Json(json!({
        "membershipId": proof.membership_id, "memberType": "EMPLOYEE",
        "identityVersion": proof.identity_version.to_string(),
        "accountVersion": proof.account_version.to_string(),
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
        .and_then(|v| crate::util::uuid7::parse_uuid(v).ok())
        .ok_or_else(|| AppError::bad_request("缺少 applicationId 参数"))?;

    let rows = authorization::context_menu_rows(&state.pool, &user, Some(app_id)).await?;
    Ok(Json(json!({"menus": authorization::menu_tree(rows)})))
}

/// GET /api/auth/permissions
async fn permissions(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    let roles = state.context_roles(&user).await?;
    let mut perms: Vec<String> = state
        .context_permissions(&user)
        .await?
        .into_iter()
        .collect();
    perms.sort();
    Ok(Json(json!({"permissions": perms, "roles": roles})))
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
    state.accounts.validate_password(&body.new_password)?;
    if state.remote_accounts.is_some() {
        return super::users::remote::mutate(
            &state,
            &headers,
            &user,
            user.user_id,
            "CHANGE_PASSWORD",
            json!({"oldPassword":body.old_password,"newPassword":body.new_password}),
            None,
        )
        .await
        .map(Json);
    }
    let credential = state.authentication.by_user_id(user.user_id).await?;
    if !state
        .authentication
        .verify(&credential, &body.old_password)
        .await?
    {
        return Err(AppError::bad_request("原密码不正确"));
    }
    let password = state.accounts.prepare_password(&body.new_password).await?;
    let mut tx = state.pool.begin().await?;
    {
        let mut accounts = crate::authentication::PostgresAccountTransaction::new(&mut tx);
        accounts
            .lock_verified_password_snapshot(&credential)
            .await?;
    }
    // Preserve the reset-conflict check above, then reject an original context
    // retired while this request waited. No account mutation has run yet.
    session::require_active_in_tx(
        &mut tx,
        user.user_id,
        user.tenant_id,
        user.org_id,
        user.session_id,
    )
    .await?;
    let change = {
        let mut accounts = crate::authentication::PostgresAccountTransaction::new(&mut tx);
        state
            .accounts
            .replace_password(&mut accounts, user.user_id, &password, Some(&credential))
            .await?
    };
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write_in_tx(
        &mut tx,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(user.tenant_id),
            organization_id: Some(user.org_id),
            action: "UPDATE_PASSWORD",
            detail: &audit::change_detail(
                "user",
                user.user_id,
                json!({"username": user.username, "sessionVersion": change.before_version}),
                json!({"username": user.username, "sessionVersion": change.after_version,
                "passwordChanged": true, "revokedSessions": change.revoked_sessions}),
            ),
            ip: &ip,
            user_agent: &ua,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok": true})))
}

const LOGIN_HISTORY_WHERE: &str = "user_id = $1 AND action IN ('LOGIN', 'LOGIN_FAILED')";

type LoginHistoryTuple = (i64, String, String, String, String, String, DateTime<Utc>);

fn login_history_paging(params: &HashMap<String, String>) -> (i64, i64, i64) {
    // Preserve this endpoint's 50-row cap while sharing the checked page bound.
    let size = params
        .get("size")
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value >= 1)
        .map(|value| value.min(50))
        .unwrap_or(10)
        .to_string();
    saas_platform_backend::paging::Paging::from_raw(
        params.get("page").map(String::as_str),
        Some(&size),
    )
    .parts()
}

/// GET /api/auth/login-history — 本人登录历史时间线 (登录成功/失败)
///
/// 个人安全中心自助查询: 仅本人审计行 (user_id = 当前用户), 只含 LOGIN / LOGIN_FAILED。
/// 用户名不存在或被限流等场景写入的 LOGIN_FAILED 行 user_id 为 NULL,
/// 不属于任何用户, 自然不会出现在任何人的历史里 (admin 密码错误的失败行则带有 user_id)。
/// 分页: page 限于 1..=1_000_000 (非法回落 1), size 上限 50, 按时间倒序。
async fn login_history(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    let (page, size, offset) = login_history_paging(&params);

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
    let total: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM audit_logs WHERE {LOGIN_HISTORY_WHERE}"
    ))
    .bind(user.user_id)
    .fetch_one(&state.pool)
    .await?;

    let records: Vec<Value> = rows
        .into_iter()
        .map(
            |(id, action, ip, detail, tenant_name, organization_name, created_at)| {
                json!({
                    "id": id, "action": action, "ip": ip, "detail": detail,
                    "tenantName": tenant_name, "organizationName": organization_name,
                    "createdAt": created_at,
                })
            },
        )
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
async fn my_sessions(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
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
    let revoked_total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM oauth_tokens WHERE user_id = $1 AND revoked = true",
    )
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
        &state.pool,
        Some(user.user_id),
        user.user_id,
        user.session_id,
    )
    .await?;
    Ok(Json(
        json!({"sessions": sessions, "revokedTotal": revoked_total,
                   "platformSessions": platform_sessions}),
    ))
}

/// DELETE /api/auth/sessions/{id} — 用户撤销自己的指定设备会话。
async fn revoke_my_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let id: Uuid = id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(user.user_id)
        .fetch_one(&mut *tx)
        .await?;
    let target: Option<(Uuid, Uuid)> = sqlx::query_as(
        "UPDATE platform_sessions SET revoked_at = now() \
         WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL \
         RETURNING tenant_id, organization_id",
    )
    .bind(id)
    .bind(user.user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (tenant_id, org_id) = target.ok_or_else(|| AppError::not_found("会话不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    audit::write_in_tx(
        &mut tx,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(tenant_id),
            organization_id: Some(org_id),
            action: "REVOKE_SESSION",
            detail: &audit::change_detail(
                "platform_session",
                id,
                json!({"username": user.username, "revoked": false}),
                json!({"username": user.username, "revoked": true}),
            ),
            ip: &ip,
            user_agent: &ua,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    use super::login_history_paging;
    use std::collections::HashMap;

    #[test]
    fn login_history_bounds_extreme_pages_without_overflow() {
        let params = HashMap::from([
            ("page".into(), i64::MAX.to_string()),
            ("size".into(), i64::MAX.to_string()),
        ]);
        assert_eq!(login_history_paging(&params), (1_000_000, 50, 49_999_950));
    }

    #[test]
    fn login_history_preserves_defaults_and_its_fifty_row_cap() {
        for (raw, expected) in [("0", 10), ("invalid", 10), ("51", 50), ("7", 7)] {
            let params = HashMap::from([("page".into(), "2".into()), ("size".into(), raw.into())]);
            assert_eq!(login_history_paging(&params), (2, expected, expected));
        }
        assert_eq!(login_history_paging(&HashMap::new()), (1, 10, 0));
    }
}
