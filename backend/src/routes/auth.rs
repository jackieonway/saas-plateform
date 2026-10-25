use crate::util::uuid7::ParseUuid7;
use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::authorization;
use crate::error::AppError;
use crate::extractors::{bearer_token, ApiJson, CurrentUser};
use crate::routes::page_json;
use crate::session;
use crate::state::AppState;
use crate::util::{audit, jwt};

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
         WHERE m.user_id = $1 \
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

fn console_selectable(membership: &MembershipRow) -> bool {
    membership.member_type == "EMPLOYEE" && membership.status == "ENABLED"
}

#[derive(Deserialize)]
struct LoginBody {
    username: String,
    password: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CredentialAccount {
    #[serde(deserialize_with = "crate::util::uuid7::deserialize")]
    user_id: Uuid,
    version: i64,
    memberships: Vec<CredentialMembership>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CredentialMembership {
    #[serde(deserialize_with = "crate::util::uuid7::deserialize")]
    membership_id: Uuid,
    #[serde(deserialize_with = "crate::util::uuid7::deserialize")]
    organization_id: Uuid,
    identity_version: i64,
}

#[derive(Serialize, Deserialize)]
struct CredentialSelection {
    #[serde(flatten)]
    claims: jwt::Claims,
    accounts: Vec<CredentialAccount>,
    origin_client_policy_version: i64,
}

const LEGACY_CLIENT_REF: &str = "platform-console-legacy";
const LEGACY_ISSUER: &str = "urn:saas:legacy-platform";

fn require_legacy_login(state: &AppState) -> Result<(), AppError> {
    if state.config.bff_enabled && !state.config.bff_legacy_login_enabled {
        return Err(AppError::forbidden("请通过统一登录入口认证"));
    }
    Ok(())
}

async fn legacy_source_policy(connection: &mut sqlx::PgConnection) -> Result<i64, AppError> {
    sqlx::query_scalar(
        "SELECT access_policy_version FROM oauth_clients WHERE client_id=$1 \
         AND status='ENABLED' AND subject_mode='USER' AND application_id IS NULL \
         AND allowed_member_types=ARRAY['EMPLOYEE']::text[] FOR SHARE",
    )
    .bind(LEGACY_CLIENT_REF)
    .fetch_optional(connection)
    .await
    .map_err(|_| AppError::Unavailable)?
    .ok_or_else(|| AppError::unauthorized("员工登录来源已停用，请重新登录"))
}

/// Trusted owned-database fixture equivalent of a completed password proof.
#[cfg(test)]
pub(crate) async fn fixture_credential_ticket(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    secret: &str,
) -> anyhow::Result<String> {
    let version: i64 = sqlx::query_scalar("SELECT session_version FROM users WHERE id=$1")
        .bind(user_id)
        .fetch_one(pool)
        .await?;
    let origin_client_policy_version = legacy_source_policy(&mut *pool.acquire().await?).await?;
    let memberships = fetch_memberships(pool, user_id)
        .await?
        .into_iter()
        .filter(console_selectable)
        .map(|membership| CredentialMembership {
            membership_id: membership.membership_id,
            organization_id: membership.organization_id,
            identity_version: membership.identity_version,
        })
        .collect();
    let now = Utc::now().timestamp();
    jwt::sign_payload(
        &CredentialSelection {
            claims: jwt::Claims {
                sub: user_id.to_string(),
                typ: "credential".into(),
                tenant: None,
                org: None,
                cid: None,
                scope: None,
                sid: None,
                ver: Some(version),
                aud: None,
                jti: None,
                exp: now + 300,
                iat: now,
            },
            accounts: vec![CredentialAccount {
                user_id,
                version,
                memberships,
            }],
            origin_client_policy_version,
        },
        secret,
    )
}

/// Password proof precedes tenant/organization selection, including shared aliases.
async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<LoginBody>,
) -> Result<Json<Value>, AppError> {
    require_legacy_login(&state)?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    if body.username.len() > state.config.login_identifier_max_bytes || body.password.len() > 1024 {
        return Err(AppError::bad_request("登录账号或密码超出长度上限"));
    }
    let identifier = body.username.trim().to_string();
    if identifier.is_empty() || body.password.is_empty() {
        return Err(AppError::bad_request("登录账号与密码不能为空"));
    }
    let phone = crate::routes::contact::phone_identifier(&identifier);
    let email = if identifier.contains('@') {
        identifier.to_lowercase()
    } else {
        String::new()
    };
    if let Some(retry) = state.login_limits.admit_source(&ip) {
        return Err(AppError::RateLimited(retry));
    }
    let input_key = format!(
        "input:{}",
        crate::util::hash::sha256_hex(if !phone.is_empty() {
            &phone
        } else if !email.is_empty() {
            &email
        } else {
            &identifier
        },)
    );
    if let Some(retry) = state.login_limits.check_account(&input_key) {
        return Err(AppError::RateLimited(retry));
    }
    let rows = state
        .authentication
        .candidates(crate::platform_accounts::LoginIdentifier {
            identifier: &identifier,
            phone: &phone,
            email: &email,
        })
        .await?;
    if rows.len() > 64 {
        return Err(AppError::bad_request(
            "该电话或 Email 关联账号过多，请使用其他登录账号",
        ));
    }
    if rows.is_empty() {
        state.login_limits.record_failure(&input_key);
        audit::write(
            &state.pool,
            audit::AuditInfo {
                user_id: None,
                username: &identifier,
                tenant_id: None,
                organization_id: None,
                action: "LOGIN_FAILED",
                detail: "用户不存在",
                ip: &ip,
                user_agent: &ua,
            },
        )
        .await;
        return Err(AppError::unauthorized("用户名或密码错误"));
    }
    let mut accounts = Vec::new();
    let mut choices = Vec::new();
    let mut locked = None;
    let mut authenticated_without_membership = false;
    let mut failures = Vec::new();
    let origin_client_policy_version = legacy_source_policy(
        &mut *state
            .pool
            .acquire()
            .await
            .map_err(|_| AppError::Unavailable)?,
    )
    .await?;
    let verify_deadline = tokio::time::Instant::now()
        + std::time::Duration::from_millis(state.config.login_verify_budget_ms);
    for row in rows {
        if tokio::time::Instant::now() >= verify_deadline {
            return Err(AppError::RateLimited(1));
        }
        let account_key = format!("user:{}", row.id);
        if let Some(retry) = state.login_limits.check_account(&account_key) {
            locked = Some(retry);
            continue;
        }
        if row.status != "ENABLED" {
            continue;
        }
        let valid = tokio::time::timeout_at(
            verify_deadline,
            state.authentication.verify(&row, &body.password),
        )
        .await
        .map_err(|_| AppError::RateLimited(1))??;
        if !valid {
            failures.push((row.id, row.username));
            continue;
        }
        state.login_limits.clear_account(&account_key);
        let memberships = fetch_memberships(&state.pool, row.id).await?;
        if !memberships.iter().any(console_selectable) {
            authenticated_without_membership = true;
            continue;
        }
        for membership in &memberships {
            let mut choice = membership_json(membership);
            choice["userId"] = json!(row.id);
            choice["username"] = json!(row.username);
            choice["realName"] = json!(row.real_name);
            choices.push(choice);
        }
        state.accounts.record_login(row.id).await?;
        let first = memberships
            .iter()
            .find(|membership| console_selectable(membership))
            .ok_or_else(|| AppError::forbidden("没有可用于控制台的员工身份"))?;
        audit::write(
            &state.pool,
            audit::AuditInfo {
                user_id: Some(row.id),
                username: &row.username,
                tenant_id: Some(first.tenant_id),
                organization_id: Some(first.organization_id),
                action: "LOGIN",
                detail: "登录成功, 签发登录票据",
                ip: &ip,
                user_agent: &ua,
            },
        )
        .await;
        accounts.push(CredentialAccount {
            user_id: row.id,
            version: row.account_version,
            memberships: memberships
                .iter()
                .filter(|membership| console_selectable(membership))
                .map(|membership| CredentialMembership {
                    membership_id: membership.membership_id,
                    organization_id: membership.organization_id,
                    identity_version: membership.identity_version,
                })
                .collect(),
        });
    }
    let Some(first) = accounts.first() else {
        // A successful shared-alias login must not count as a failed attempt
        // against the other accounts using that alias.
        if !authenticated_without_membership {
            state.login_limits.record_failure(&input_key);
            for (id, username) in failures {
                state.login_limits.record_failure(&format!("user:{id}"));
                audit::write(
                    &state.pool,
                    audit::AuditInfo {
                        user_id: Some(id),
                        username: &username,
                        tenant_id: None,
                        organization_id: None,
                        action: "LOGIN_FAILED",
                        detail: "密码错误",
                        ip: &ip,
                        user_agent: &ua,
                    },
                )
                .await;
            }
        }
        if let Some(retry) = locked {
            return Err(AppError::RateLimited(retry));
        }
        if authenticated_without_membership {
            return Err(AppError::forbidden("没有可用于平台控制台的有效员工身份"));
        }
        return Err(AppError::unauthorized("用户名或密码错误"));
    };
    state.login_limits.clear_account(&input_key);
    let now = Utc::now().timestamp();
    let account_count = accounts.len();
    let claims = jwt::Claims {
        sub: first.user_id.to_string(),
        typ: if account_count > 1 {
            "credential_multi"
        } else {
            "credential"
        }
        .into(),
        tenant: None,
        org: None,
        cid: None,
        scope: None,
        sid: None,
        ver: Some(first.version),
        aud: None,
        jti: None,
        exp: now + state.config.credential_ticket_ttl_minutes * 60,
        iat: now,
    };
    let ticket = jwt::sign_payload(
        &CredentialSelection {
            claims,
            accounts,
            origin_client_policy_version,
        },
        &state.config.jwt_secret,
    )?;
    Ok(Json(
        json!({ "ticket": ticket, "memberships": choices, "accountCount": account_count }),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ContextBody {
    #[serde(default)]
    organization_id: Option<String>,
    #[serde(default, deserialize_with = "crate::util::uuid7::deserialize_optional")]
    membership_id: Option<Uuid>,
    #[serde(default)]
    #[serde(deserialize_with = "crate::util::uuid7::deserialize_optional")]
    user_id: Option<Uuid>,
}

#[derive(sqlx::FromRow)]
struct ContextUserRow {
    username: String,
    real_name: String,
    email: String,
    phone: String,
    avatar: String,
    gender: String,
    is_super_admin: bool,
    status: String,
    session_version: i64,
}

#[derive(sqlx::FromRow)]
struct ContextOrganizationRow {
    org_id: Uuid,
    org_name: String,
    org_path: String,
    tenant_id: Uuid,
    tenant_name: String,
    tenant_code: String,
    tenant_is_platform: bool,
    org_is_root: bool,
    org_expires: Option<DateTime<Utc>>,
    tenant_expires: Option<DateTime<Utc>>,
}

/// POST /api/auth/context — select an exact employee membership from a bound proof.
async fn context(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<ContextBody>,
) -> Result<Json<Value>, AppError> {
    require_legacy_login(&state)?;
    let token = bearer_token(&headers)?;
    let claims = jwt::verify(&token, &state.config.jwt_secret)
        .map_err(|_| AppError::unauthorized("凭证无效或已过期"))?;
    if !matches!(
        claims.typ.as_str(),
        "credential" | "credential_multi" | "session"
    ) {
        return Err(AppError::unauthorized(
            "凭证类型错误, 需要 ticket 或会话令牌",
        ));
    }
    let requested_org = body
        .organization_id
        .as_deref()
        .map(|value| {
            value
                .parse_uuid7()
                .map_err(|_| AppError::bad_request("组织 ID 格式错误"))
        })
        .transpose()?;
    if requested_org.is_none() && body.membership_id.is_none() {
        return Err(AppError::bad_request("请选择成员身份"));
    }
    // A password ticket binds the exact identity generations offered at login.
    // Missing snapshots are rejected instead of silently reading newer versions.
    let selection: Option<CredentialSelection> = if claims.typ == "session" {
        None
    } else {
        Some(
            jwt::verify_payload(&token, &state.config.jwt_secret)
                .map_err(|_| AppError::unauthorized("旧版登录选择凭证已失效，请重新登录"))?,
        )
    };
    let (user_id, claim_version) = if let Some(selection) = &selection {
        let candidates: Vec<&CredentialAccount> = selection
            .accounts
            .iter()
            .filter(|account| body.user_id.is_none_or(|id| id == account.user_id))
            .filter(|account| {
                account.memberships.iter().any(|membership| {
                    body.membership_id
                        .is_none_or(|id| id == membership.membership_id)
                        && requested_org.is_none_or(|id| id == membership.organization_id)
                })
            })
            .collect();
        if candidates.is_empty() {
            return Err(AppError::forbidden("所选账号或成员身份未通过本次登录验证"));
        }
        if candidates.len() > 1 {
            return Err(AppError::bad_request(
                "请选择本次登录验证过的账号及成员身份",
            ));
        }
        (candidates[0].user_id, candidates[0].version)
    } else {
        let id = claims
            .sub
            .parse_uuid7()
            .map_err(|_| AppError::unauthorized("凭证无效"))?;
        if body.user_id.is_some_and(|selected| selected != id) {
            return Err(AppError::forbidden("不能通过当前凭证切换用户账号"));
        }
        (
            id,
            claims
                .ver
                .ok_or_else(|| AppError::unauthorized("旧版凭证已失效，请重新登录"))?,
        )
    };
    let old_session_id = if claims.typ == "session" {
        Some(
            claims
                .sid
                .as_deref()
                .and_then(|value| crate::util::uuid7::parse_uuid(value).ok())
                .ok_or_else(|| AppError::unauthorized("旧版会话已失效，请重新登录"))?,
        )
    } else {
        None
    };
    if let Some(id) = old_session_id {
        let bff_source: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM platform_sessions WHERE id=$1 AND bff_session_id IS NOT NULL)",
        )
        .bind(id)
        .fetch_one(&state.pool)
        .await
        .map_err(|_| AppError::Unavailable)?;
        if bff_source {
            return Err(AppError::unauthorized(
                "BFF 上下文只能通过受保护的 /auth/context-token 获取",
            ));
        }
    }
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| AppError::Unavailable)?;
    let user: Option<ContextUserRow> = sqlx::query_as(
        "SELECT username,COALESCE(real_name,'') AS real_name,COALESCE(email,'') AS email, \
         COALESCE(phone,'') AS phone,COALESCE(avatar,'') AS avatar, \
         COALESCE(gender,'UNKNOWN') AS gender,is_super_admin,status,session_version \
         FROM users WHERE id=$1 FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| AppError::Unavailable)?;
    let ContextUserRow {
        username,
        real_name,
        email,
        phone,
        avatar,
        gender,
        is_super_admin: is_super,
        status,
        session_version,
    } = user.ok_or_else(|| AppError::unauthorized("用户不存在"))?;
    if status != "ENABLED" || session_version != claim_version {
        return Err(AppError::unauthorized("凭证已撤销，请重新登录"));
    }
    // Account then source client: match revocation/policy writers' lock order.
    let source_version = legacy_source_policy(&mut tx).await?;
    if selection
        .as_ref()
        .is_some_and(|proof| proof.origin_client_policy_version != source_version)
    {
        return Err(AppError::unauthorized("登录来源策略已变更，请重新登录"));
    }
    if let Some(old_id) = old_session_id {
        let old_tenant = claims
            .tenant
            .as_deref()
            .and_then(|v| crate::util::uuid7::parse_uuid(v).ok())
            .ok_or_else(|| AppError::unauthorized("会话缺少租户上下文"))?;
        let old_org = claims
            .org
            .as_deref()
            .and_then(|v| crate::util::uuid7::parse_uuid(v).ok())
            .ok_or_else(|| AppError::unauthorized("会话缺少组织上下文"))?;
        let valid: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM platform_sessions s WHERE s.id=$1 AND s.user_id=$2 \
             AND s.tenant_id=$3 AND s.organization_id=$4 AND s.account_version=$5 \
             AND s.member_type='EMPLOYEE' AND s.origin_client_ref=$6 AND s.origin_issuer=$7 \
             AND s.origin_client_policy_version=$8 AND s.bff_session_id IS NULL AND platform_session_active(s.id))",
        )
        .bind(old_id)
        .bind(user_id)
        .bind(old_tenant)
        .bind(old_org)
        .bind(claim_version)
        .bind(LEGACY_CLIENT_REF)
        .bind(LEGACY_ISSUER)
        .bind(source_version)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| AppError::Unavailable)?;
        if !valid {
            return Err(AppError::unauthorized("会话已失效，请重新登录"));
        }
    }
    let memberships: Vec<(Uuid, Uuid, i64)> = sqlx::query_as(
        "SELECT id,organization_id,identity_version FROM memberships WHERE user_id=$1 \
         AND member_type='EMPLOYEE' AND status='ENABLED' \
         AND ($2::uuid IS NULL OR organization_id=$2) AND ($3::uuid IS NULL OR id=$3)",
    )
    .bind(user_id)
    .bind(requested_org)
    .bind(body.membership_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|_| AppError::Unavailable)?;
    if memberships.is_empty() {
        return Err(AppError::forbidden("您在该组织没有所选员工成员身份"));
    }
    if memberships.len() > 1 {
        return Err(AppError::bad_request("请选择唯一且有效的员工成员身份"));
    }
    let (membership_id, org_id, identity_version) = memberships[0];
    if let Some(selection) = &selection {
        let bound = selection
            .accounts
            .iter()
            .find(|account| account.user_id == user_id)
            .is_some_and(|account| {
                account.memberships.iter().any(|membership| {
                    membership.membership_id == membership_id
                        && membership.organization_id == org_id
                        && membership.identity_version == identity_version
                })
            });
        if !bound {
            return Err(AppError::unauthorized("成员身份已变更，请重新登录"));
        }
    }
    let org: Option<ContextOrganizationRow> =
        sqlx::query_as(
            "SELECT o.id AS org_id,o.name AS org_name,org_path(o.id) AS org_path,o.tenant_id, \
             t.name AS tenant_name,t.code AS tenant_code,t.is_platform AS tenant_is_platform, \
             o.parent_id IS NULL AS org_is_root,org_chain_expires_at(o.id) AS org_expires, \
             t.expires_at AS tenant_expires \
             FROM organizations o JOIN tenants t ON t.id=o.tenant_id \
             WHERE o.id=$1 AND org_chain_active(o.id) AND t.status='ENABLED' \
             AND (t.expires_at IS NULL OR t.expires_at>clock_timestamp()) \
             AND (org_chain_expires_at(o.id) IS NULL OR org_chain_expires_at(o.id)>clock_timestamp())",
        ).bind(org_id).fetch_optional(&mut *tx).await.map_err(|_| AppError::Unavailable)?;
    let ContextOrganizationRow {
        org_id,
        org_name,
        org_path,
        tenant_id,
        tenant_name,
        tenant_code,
        tenant_is_platform,
        org_is_root,
        org_expires,
        tenant_expires,
    } = org.ok_or_else(|| AppError::forbidden("所属租户或组织不可用"))?;
    let now = Utc::now().timestamp();
    let ttl = state.config.session_ttl_hours * 3600;
    let refresh_ttl = state.config.refresh_token_ttl_seconds;
    if ttl <= 0 || refresh_ttl <= 0 {
        return Err(AppError::bad_request("会话有效期配置无效"));
    }
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let (session_id,expires_at): (Uuid,DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO platform_sessions \
         (user_id,tenant_id,organization_id,expires_at,ip,user_agent,id,account_version, \
          membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version) \
         VALUES($1,$2,$3,LEAST(clock_timestamp()+make_interval(secs=>$4::float8), \
         $7::timestamptz,$8::timestamptz),$5,$6,$9,$10,$11,'EMPLOYEE',$12,$13,$14,$15) \
         RETURNING id,expires_at",
    ).bind(user_id).bind(tenant_id).bind(org_id).bind(ttl.max(refresh_ttl) as f64)
        .bind(&ip).bind(&ua).bind(org_expires).bind(tenant_expires)
        .bind(crate::util::uuid7::new_uuid()).bind(session_version).bind(membership_id)
        .bind(identity_version).bind(LEGACY_ISSUER).bind(LEGACY_CLIENT_REF).bind(source_version)
        .fetch_one(&mut *tx).await?;
    if let Some(old_id) = old_session_id {
        let revoked = sqlx::query(
            "UPDATE platform_sessions SET revoked_at=clock_timestamp() WHERE id=$1 AND user_id=$2 \
             AND revoked_at IS NULL AND expires_at>clock_timestamp()",
        )
        .bind(old_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if revoked != 1 {
            return Err(AppError::unauthorized("会话已失效，请重新登录"));
        }
    }
    let access_expiry = (now + ttl).min(expires_at.timestamp());
    let refresh_expiry =
        DateTime::from_timestamp((now + refresh_ttl).min(expires_at.timestamp()), 0)
            .ok_or_else(|| AppError::bad_request("刷新令牌有效期配置无效"))?;
    if access_expiry <= now || refresh_expiry <= Utc::now() {
        return Err(AppError::forbidden("租户或组织有效期不足，无法签发会话"));
    }
    let refresh_token =
        session::issue_refresh_in_tx(&mut tx, session_id, session_version, refresh_expiry).await?;
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
    if old_session_id.is_some() {
        audit::write_in_tx(&mut tx,audit::AuditInfo {
            user_id:Some(user_id),username:&username,tenant_id:Some(tenant_id),
            organization_id:Some(org_id),action:"CONTEXT_SWITCH",
            detail:&audit::change_detail("platform_session",session_id,
                json!({"previousSessionId":old_session_id,"organizationId":claims.org,
                    "tenantId":claims.tenant,"username":username}),
                json!({"organizationId":org_id,"organizationName":org_name,"membershipId":membership_id,
                    "identityVersion":identity_version.to_string(),"tenantId":tenant_id,"username":username})),
            ip:&ip,user_agent:&ua,
        }).await?;
    }
    tx.commit().await?;
    Ok(Json(json!({
        "accessToken":access_token,"expiresIn":access_expiry-now,
        "refreshToken":refresh_token,"refreshExpiresIn":refresh_expiry.timestamp()-now,
        "platformContext":tenant_is_platform && org_is_root,
        "membershipId":membership_id,"memberType":"EMPLOYEE","identityVersion":identity_version.to_string(),
        "accountVersion":session_version.to_string(),
        "user":{"id":user_id,"username":username,"realName":real_name,"email":email,
            "phone":phone,"avatar":avatar,"gender":gender,"isSuperAdmin":is_super},
        "tenant":{"id":tenant_id,"name":tenant_name,"code":tenant_code,"isPlatform":tenant_is_platform},
        "organization":{"id":org_id,"name":org_name,"path":org_path},
    })))
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

/// POST /api/auth/logout
async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let mut tx = state.pool.begin().await?;
    // Follow the same user-before-session lock order as refresh/password changes.
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(user.user_id)
        .fetch_one(&mut *tx)
        .await?;
    let changed = sqlx::query(
        "UPDATE platform_sessions SET revoked_at=now() \
         WHERE id=$1 AND user_id=$2 AND revoked_at IS NULL",
    )
    .bind(user.session_id)
    .bind(user.user_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed != 1 {
        return Err(AppError::unauthorized("会话已失效，请重新登录"));
    }
    audit::write_in_tx(
        &mut tx,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(user.tenant_id),
            organization_id: Some(user.org_id),
            action: "LOGOUT",
            detail: &audit::change_detail(
                "platform_session",
                user.session_id,
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
