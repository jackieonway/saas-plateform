//! Platform administrators manage platform sessions; global revocation goes through Identity.
use crate::{
    error::AppError,
    extractors::CurrentUser,
    state::AppState,
    util::{audit, uuid7::ParseUuid7},
};
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};
use std::collections::HashMap;
use uuid::Uuid;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/sessions", get(list))
        .route("/api/admin/session-users", get(users))
        .route("/api/admin/sessions/{id}/revoke", post(revoke))
        .route("/api/admin/session-users/{id}/revoke-all", post(revoke_all))
}

pub(crate) async fn require_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    actor: &CurrentUser,
    write: bool,
) -> Result<(), AppError> {
    crate::session::require_active_in_tx(
        tx,
        actor.user_id,
        actor.tenant_id,
        actor.org_id,
        actor.session_id,
    )
    .await?;
    super::users::require_scope_unchanged_in_tx(tx, actor, None).await?;
    super::users::require_employee_permission_in_tx(tx, actor, "system:session").await?;
    if write {
        super::users::require_employee_permission_in_tx(tx, actor, "system:session:revoke").await?;
    }
    Ok(())
}

fn filters(params: &HashMap<String, String>) -> (String, bool) {
    (
        params.get("keyword").cloned().unwrap_or_default(),
        params.get("activeOnly").is_some_and(|v| v == "true"),
    )
}

async fn list(
    State(state): State<AppState>,
    actor: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    let (page, size, offset) = super::paging(&params);
    let (keyword, active) = filters(&params);
    let mut tx = state.pool.begin().await?;
    require_in_tx(&mut tx, &actor, false).await?;
    let total:i64=sqlx::query_scalar("SELECT count(*) FROM platform_sessions s JOIN users u ON u.id=s.user_id WHERE (u.username ILIKE '%'||$1||'%' OR u.real_name ILIKE '%'||$1||'%') AND (NOT $2 OR platform_session_active(s.id))")
        .bind(&keyword).bind(active).fetch_one(&mut *tx).await?;
    let records:Vec<crate::session::DeviceSession>=sqlx::query_as(
        "SELECT s.id,s.user_id,u.username,COALESCE(u.real_name,'') AS real_name,s.tenant_id,t.name AS tenant_name,s.organization_id,o.name AS organization_name,s.issued_at,s.expires_at,s.revoked_at,s.ip,s.user_agent,platform_session_active(s.id) AS active,(s.id=$3 AND s.user_id=$4) AS is_current \
         FROM platform_sessions s JOIN users u ON u.id=s.user_id JOIN tenants t ON t.id=s.tenant_id JOIN organizations o ON o.id=s.organization_id AND o.tenant_id=s.tenant_id \
         WHERE (u.username ILIKE '%'||$1||'%' OR u.real_name ILIKE '%'||$1||'%') AND (NOT $2 OR platform_session_active(s.id)) ORDER BY s.issued_at DESC,s.id DESC LIMIT $5 OFFSET $6")
        .bind(keyword).bind(active).bind(actor.session_id).bind(actor.user_id).bind(size).bind(offset).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(
        json!({"records":records,"total":total,"page":page,"size":size}),
    ))
}

async fn users(
    State(state): State<AppState>,
    actor: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    let (page, size, offset) = super::paging(&params);
    let (keyword, active) = filters(&params);
    let mut tx = state.pool.begin().await?;
    require_in_tx(&mut tx, &actor, false).await?;
    let total:i64=sqlx::query_scalar("SELECT count(*) FROM users u WHERE (u.username ILIKE '%'||$1||'%' OR u.real_name ILIKE '%'||$1||'%') AND EXISTS(SELECT 1 FROM platform_sessions s WHERE s.user_id=u.id AND (NOT $2 OR platform_session_active(s.id)))")
        .bind(&keyword).bind(active).fetch_one(&mut *tx).await?;
    let records:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('userId',u.id,'username',u.username,'realName',COALESCE(u.real_name,''),'sessions',count(*),'activeSessions',count(*) FILTER(WHERE platform_session_active(s.id)),'lastIssuedAt',max(s.issued_at),'isCurrentUser',u.id=$3) FROM users u JOIN platform_sessions s ON s.user_id=u.id WHERE (u.username ILIKE '%'||$1||'%' OR u.real_name ILIKE '%'||$1||'%') GROUP BY u.id HAVING NOT $2 OR bool_or(platform_session_active(s.id)) ORDER BY max(s.issued_at) DESC,u.id LIMIT $4 OFFSET $5")
        .bind(keyword).bind(active).bind(actor.user_id).bind(size).bind(offset).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(super::page_json(records, total, page, size)))
}

async fn revoke(
    State(state): State<AppState>,
    actor: CurrentUser,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    state.require_platform(&actor).await?;
    state.require(&actor, "system:session:revoke").await?;
    let id = id.parse_uuid7()?;
    let mut tx = state.pool.begin().await?;
    let owner: Uuid = sqlx::query_scalar("SELECT user_id FROM platform_sessions WHERE id=$1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::not_found("会话不存在"))?;
    let mut locks = vec![owner, actor.user_id];
    locks.sort_unstable();
    locks.dedup();
    sqlx::query("SELECT id FROM users WHERE id=ANY($1) ORDER BY id FOR UPDATE")
        .bind(locks)
        .fetch_all(&mut *tx)
        .await?;
    require_in_tx(&mut tx, &actor, true).await?;
    let bff: Option<Uuid> =
        sqlx::query_scalar("SELECT bff_session_id FROM platform_sessions WHERE id=$1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if let Some(bff) = bff {
        sqlx::query("SELECT id FROM bff_sessions WHERE id=$1 AND user_id=$2 FOR UPDATE")
            .bind(bff)
            .bind(owner)
            .fetch_one(&mut *tx)
            .await?;
    }
    let changed=sqlx::query("UPDATE platform_sessions SET revoked_at=clock_timestamp() WHERE id=$1 AND revoked_at IS NULL").bind(id).execute(&mut *tx).await?.rows_affected();
    if let Some(bff) = bff {
        crate::bff::revoke_browser_in_tx(&mut tx, bff).await?;
    }
    if changed > 0 {
        audit::write_in_tx(
            &mut tx,
            audit::AuditInfo {
                user_id: Some(actor.user_id),
                username: &actor.username,
                tenant_id: Some(actor.tenant_id),
                organization_id: Some(actor.org_id),
                action: "ADMIN_REVOKE_SESSION",
                detail: &audit::change_detail(
                    "platform_session",
                    id,
                    json!({"userId":owner,"revoked":false}),
                    json!({"userId":owner,"revoked":true}),
                ),
                ip: &audit::client_ip(&headers),
                user_agent: &audit::ua_of(&headers),
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"ok":true,"revoked":changed})))
}

async fn revoke_all(
    State(state): State<AppState>,
    actor: CurrentUser,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    state.require_platform(&actor).await?;
    state.require(&actor, "system:session").await?;
    state.require(&actor, "system:session:revoke").await?;
    let target = id.parse_uuid7()?;
    super::users::remote::mutate(
        &state,
        &headers,
        &actor,
        target,
        "REVOKE_ALL",
        json!({}),
        None,
    )
    .await
    .map(Json)
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
