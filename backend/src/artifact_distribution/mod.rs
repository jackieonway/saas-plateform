//! Optional ZIP replication authority. Nodes own local files; this service only uses HTTPS.
use crate::util::uuid7::ParseUuid7;
mod config;
mod gates;
mod routes;
mod transport;
mod worker;
use crate::{
    error::AppError,
    extractors::{CurrentUser, RequirePermission},
    state::AppState,
};
pub(crate) use gates::{defer_upload_probe, require_ready, verify_nodes};
pub(crate) use routes::routes;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
pub(crate) use worker::start;

pub(super) async fn authorize(
    state: &AppState,
    user: &CurrentUser,
    write: bool,
) -> Result<(), AppError> {
    state.require_platform(user).await?;
    RequirePermission(if write {
        "system:distribution:write"
    } else {
        "system:distribution:read"
    })
    .check(state, user)
    .await
}
pub(super) fn uuid(value: &str) -> Result<Uuid, AppError> {
    value
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("ID格式错误"))
}
pub(super) async fn lock(tx: &mut Transaction<'_, Postgres>, app: Uuid) -> Result<(), AppError> {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM applications WHERE id=$1 FOR UPDATE")
        .bind(app)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| AppError::not_found("应用不存在"))?;
    Ok(())
}
pub(super) async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    app: Uuid,
    user: Option<&CurrentUser>,
    action: &str,
    detail: serde_json::Value,
) -> Result<(), AppError> {
    let detail = detail.to_string();
    crate::util::audit::write_in_tx_for_app(
        tx,
        crate::util::audit::AuditInfo {
            user_id: user.map(|u| u.user_id),
            username: user.map_or("artifact-distributor", |u| u.username.as_str()),
            tenant_id: user.map(|u| u.tenant_id),
            organization_id: user.map(|u| u.org_id),
            action,
            detail: &detail,
            ip: "internal",
            user_agent: "zip-distribution",
        },
        app,
    )
    .await
}
