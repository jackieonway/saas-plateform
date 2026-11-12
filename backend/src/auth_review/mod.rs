//! Manual partner intake and the platform's human authorization boundary.
//! Identity remains the registry and credential authority.
mod policy;
#[cfg(test)]
mod service;
mod store;
#[cfg(test)]
mod tests;

pub(crate) use store::{
    application_allowed, complete_remote, known_permission, local_not_applied, local_operation,
    management_tx, permission_for,
};

use crate::{error::AppError, extractors::CurrentUser, state::AppState};
#[cfg(test)]
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub(crate) async fn require_application(
    tx: &mut Transaction<'_, Postgres>,
    user: &CurrentUser,
    application: Uuid,
) -> Result<(), AppError> {
    if application_allowed(tx, user, application).await? {
        Ok(())
    } else {
        Err(AppError::forbidden("合作方管理员仅可管理明确委派的应用"))
    }
}

pub(crate) async fn global_operator(
    tx: &mut Transaction<'_, Postgres>,
    user: &CurrentUser,
) -> Result<bool, AppError> {
    let menus = crate::authorization::context_menu_rows_in_tx(tx, user, None).await?;
    let permissions = crate::authorization::permissions_of(&menus);
    let administrator: bool = sqlx::query_scalar("SELECT is_super_admin FROM users WHERE id=$1")
        .bind(user.user_id)
        .fetch_one(&mut **tx)
        .await?;
    Ok(administrator
        || permissions.contains(policy::REVIEW)
        || permissions.contains("system:oauth:assign"))
}

pub(crate) async fn assert_actor(
    tx: &mut Transaction<'_, Postgres>,
    state: &AppState,
    user: &CurrentUser,
    permission: &'static str,
    recent: bool,
) -> Result<(), AppError> {
    let active: bool =
        sqlx::query_scalar("SELECT platform_identity_management_actor_active($1,$2,$3,$4,$5)")
            .bind(user.user_id)
            .bind(user.session_id)
            .bind(permission)
            .bind(recent)
            .bind(state.config.app_env == "production")
            .fetch_one(&mut **tx)
            .await
            .map_err(|_| AppError::Unavailable)?;
    if !active {
        return Err(AppError::forbidden("需要当前平台管理权限；敏感操作须在认证中心完成最近5分钟TOTP，生产管理员必须绑定身份验证器"));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn manifest_value(value: policy::Manifest) -> Result<Value, AppError> {
    serde_json::to_value(policy::validate_manifest(value)?).map_err(|_| AppError::Unavailable)
}
