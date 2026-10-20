use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::{ApiJson, CurrentUser};
use crate::notifications::system_in_tx;
use crate::state::AppState;
use crate::util::audit;

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
    let mut tx = state.pool.begin().await?;
    let row = sqlx::query_as::<_, (Uuid, String, String, DateTime<Utc>)>(
        "SELECT id, client_id, status, expires_at FROM oauth_device_codes WHERE UPPER(user_code) = $1 FOR UPDATE",
    )
    .bind(&code)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("设备码不存在"))?;
    let (id, client_id, status, expires_at) = row;
    if status != "PENDING" {
        return Err(AppError::conflict("该设备授权请求已处理"));
    }
    if expires_at < Utc::now() {
        let changed = sqlx::query("UPDATE oauth_device_codes SET status = 'EXPIRED' WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        if changed.rows_affected() != 1 {
            return Err(AppError::Server(anyhow::anyhow!("device expiry did not update one row")));
        }
        tx.commit().await?;
        return Err(AppError::conflict("该设备授权请求已过期"));
    }
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    if body.approve {
        let changed = sqlx::query(
            "UPDATE oauth_device_codes SET status = 'APPROVED', user_id = $2, tenant_id = $3, organization_id = $4 WHERE id = $1",
        )
        .bind(id)
        .bind(user.user_id)
        .bind(user.tenant_id)
        .bind(user.org_id)
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(AppError::Server(anyhow::anyhow!("device approval did not update one row")));
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
        tx.commit().await?;
    } else {
        let changed = sqlx::query("UPDATE oauth_device_codes SET status = 'DENIED' WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        if changed.rows_affected() != 1 {
            return Err(AppError::Server(anyhow::anyhow!("device denial did not update one row")));
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
        tx.commit().await?;
    }
    Ok(Json(json!({"ok": true})))
}
