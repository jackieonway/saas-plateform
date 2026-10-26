use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::{bearer_token, ApiJson, CurrentUser};
use crate::notifications::system_in_tx;
use crate::state::AppState;
use crate::util::{audit, jwt};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/device/{user_code}", get(get_device))
        .route("/api/device/{user_code}/decision", post(decide_device))
}

/// GET /api/device/{userCode} — 设备码确认页信息
async fn get_device(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(user_code): Path<String>,
) -> Result<Json<Value>, AppError> {
    let _ = user;
    let code = user_code.trim().to_uppercase();
    if code.is_empty() {
        return Err(AppError::bad_request("缺少设备码"));
    }
    let row = sqlx::query_as::<_, (String, String, String, String, DateTime<Utc>)>(
        "SELECT d.client_id, COALESCE(c.client_name, '') AS client_name, COALESCE(d.scope, '') AS scope, \
         CASE WHEN d.status = 'PENDING' AND d.expires_at < now() THEN 'EXPIRED' ELSE d.status END AS status, \
         d.expires_at \
         FROM oauth_device_codes d LEFT JOIN oauth_clients c ON c.client_id = d.client_id \
         WHERE UPPER(d.user_code) = $1",
    )
    .bind(&code)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found("设备码不存在"))?;
    let (client_id, client_name, scope, status, expires_at) = row;
    Ok(Json(json!({
        "clientId": client_id, "clientName": client_name, "scope": scope,
        "status": status, "expiresAt": expires_at,
    })))
}

#[derive(Deserialize)]
pub struct DecisionBody {
    pub approve: bool,
}

/// POST /api/device/{userCode}/decision — 批准/拒绝 (仅 PENDING 可决策)
async fn decide_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(user_code): Path<String>,
    ApiJson(body): ApiJson<DecisionBody>,
) -> Result<Json<Value>, AppError> {
    let code = user_code.trim().to_uppercase();
    let claims = jwt::verify(&bearer_token(&headers)?, &state.config.jwt_secret)
        .map_err(|_| AppError::unauthorized("批准会话凭证已过期"))?;
    let authority_deadline = DateTime::<Utc>::from_timestamp(claims.exp, 0)
        .ok_or_else(|| AppError::unauthorized("批准会话期限无效"))?;
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| AppError::Unavailable)?;
    crate::session::require_active_in_tx(
        &mut tx,
        user.user_id,
        user.tenant_id,
        user.org_id,
        user.session_id,
    )
    .await?;
    // Lock the target registration before the one-time credential, matching
    // token exchange and policy writers rather than upgrading an old request.
    let registration: Option<(String, Option<Uuid>)> = sqlx::query_as(
        "SELECT client_id,application_id FROM oauth_device_codes WHERE UPPER(user_code)=$1",
    )
    .bind(&code)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| AppError::Unavailable)?;
    let (registered_client, registered_app) =
        registration.ok_or_else(|| AppError::not_found("设备码不存在"))?;
    if let Some(application_id) = registered_app {
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM applications WHERE id=$1 FOR SHARE")
            .bind(application_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| AppError::Unavailable)?;
    }
    crate::session::lock_oidc_source_in_tx(&mut tx, user.session_id, user.user_id).await?;
    let clients = vec![registered_client.clone()];
    sqlx::query_scalar::<_, String>(
        "SELECT client_id FROM oauth_clients WHERE client_id=ANY($1) ORDER BY client_id FOR SHARE",
    )
    .bind(clients)
    .fetch_all(&mut *tx)
    .await
    .map_err(|_| AppError::Unavailable)?;
    crate::session::require_active_in_tx(
        &mut tx,
        user.user_id,
        user.tenant_id,
        user.org_id,
        user.session_id,
    )
    .await?;
    let row = sqlx::query_as::<_, (Uuid, String, String, DateTime<Utc>, Option<i64>, Option<Uuid>, Option<i64>)>(
        "SELECT id,client_id,status,expires_at,client_policy_version,application_id,application_policy_version \
         FROM oauth_device_codes WHERE UPPER(user_code) = $1 FOR UPDATE",
    )
    .bind(&code)
    .fetch_optional(&mut *tx)
    .await.map_err(|_| AppError::Unavailable)?
    .ok_or_else(|| AppError::not_found("设备码不存在"))?;
    let (
        id,
        client_id,
        status,
        expires_at,
        client_policy_version,
        application_id,
        application_policy_version,
    ) = row;
    if client_id != registered_client || application_id != registered_app {
        return Err(AppError::conflict("设备授权登记已变更，请重新发起"));
    }
    if status != "PENDING" {
        return Err(AppError::conflict("该设备授权请求已处理"));
    }
    if expires_at <= Utc::now() {
        let changed = sqlx::query("UPDATE oauth_device_codes SET status = 'EXPIRED' WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        if changed.rows_affected() != 1 {
            return Err(AppError::Server(anyhow::anyhow!(
                "device expiry did not update one row"
            )));
        }
        tx.commit().await?;
        return Err(AppError::conflict("该设备授权请求已过期"));
    }
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    if body.approve {
        let (account_version,membership_id,member_type,identity_version): (i64,Uuid,String,i64) =
            sqlx::query_as(
                "SELECT account_version,membership_id,member_type,identity_version FROM platform_sessions \
                 WHERE id=$1 AND user_id=$2 AND platform_session_active(id)",
            ).bind(user.session_id).bind(user.user_id).fetch_optional(&mut *tx)
                .await.map_err(|_| AppError::Unavailable)?
                .ok_or_else(|| AppError::unauthorized("批准会话的成员身份已失效"))?;
        let binding = crate::oauth_issuance::UserBinding {
            account_version,
            membership_id,
            member_type,
            identity_version,
            client_policy_version: client_policy_version
                .ok_or_else(|| AppError::unauthorized("旧版设备授权缺少策略绑定，请重新发起"))?,
            application_id,
            application_policy_version,
            platform_session_id: Some(user.session_id),
        };
        if crate::oauth_issuance::binding_identity(
            tx.as_mut(),
            user.user_id,
            user.tenant_id,
            user.org_id,
            &client_id,
            &binding,
        )
        .await
        .map_err(|_| AppError::Unavailable)?
        .is_none()
        {
            return Err(AppError::unauthorized(
                "客户端策略或成员身份已变更，请重新发起设备授权",
            ));
        }
        let changed = sqlx::query(
            "UPDATE oauth_device_codes SET status='APPROVED',user_id=$2,tenant_id=$3,organization_id=$4, \
             account_version=$5,membership_id=$6,member_type=$7,identity_version=$8,platform_session_id=$9 \
             WHERE id=$1",
        )
        .bind(id)
        .bind(user.user_id)
        .bind(user.tenant_id)
        .bind(user.org_id)
        .bind(binding.account_version)
        .bind(binding.membership_id)
        .bind(&binding.member_type)
        .bind(binding.identity_version)
        .bind(user.session_id)
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(AppError::Server(anyhow::anyhow!(
                "device approval did not update one row"
            )));
        }
        system_in_tx(
            &mut tx,
            &[(user.user_id, user.tenant_id, user.org_id)],
            "SECURITY",
            "新的设备授权已批准",
            &format!(
                "您批准了客户端 {} 的设备授权请求(设备码 {})。若非本人操作,请立即联系管理员。",
                client_id, code
            ),
            "/system/oauth/tokens",
        )
        .await?;
        let detail = audit::change_detail(
            "oauth_device_authorization",
            id,
            json!({"clientId": client_id, "status": status}),
            json!({"clientId": client_id, "status": "APPROVED", "userId": user.user_id,
                "tenantId": user.tenant_id, "organizationId": user.org_id}),
        );
        audit::write_in_tx(
            &mut tx,
            audit::AuditInfo {
                user_id: Some(user.user_id),
                username: &user.username,
                tenant_id: Some(user.tenant_id),
                organization_id: Some(user.org_id),
                action: "DEVICE_APPROVE",
                detail: &detail,
                ip: &ip,
                user_agent: &ua,
            },
        )
        .await?;
        require_decision_authority(&mut tx, user.session_id, authority_deadline).await?;
        tx.commit().await?;
    } else {
        let changed = sqlx::query("UPDATE oauth_device_codes SET status = 'DENIED' WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        if changed.rows_affected() != 1 {
            return Err(AppError::Server(anyhow::anyhow!(
                "device denial did not update one row"
            )));
        }
        let detail = audit::change_detail(
            "oauth_device_authorization",
            id,
            json!({"clientId": client_id, "status": status}),
            json!({"clientId": client_id, "status": "DENIED"}),
        );
        audit::write_in_tx(
            &mut tx,
            audit::AuditInfo {
                user_id: Some(user.user_id),
                username: &user.username,
                tenant_id: Some(user.tenant_id),
                organization_id: Some(user.org_id),
                action: "DEVICE_DENY",
                detail: &detail,
                ip: &ip,
                user_agent: &ua,
            },
        )
        .await?;
        require_decision_authority(&mut tx, user.session_id, authority_deadline).await?;
        tx.commit().await?;
    }
    Ok(Json(json!({"ok": true})))
}

async fn require_decision_authority(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    session_id: Uuid,
    deadline: DateTime<Utc>,
) -> Result<(), AppError> {
    crate::session::require_source_active_in_tx(tx, session_id).await?;
    let valid: bool = sqlx::query_scalar("SELECT $1::timestamptz>clock_timestamp()")
        .bind(deadline)
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| AppError::Unavailable)?;
    if !valid {
        return Err(AppError::unauthorized("批准会话凭证已过期"));
    }
    Ok(())
}
