use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;

pub type ActiveUser = (String, String, bool);

/// 每个受保护请求直接查库：会话撤销、成员移除、租户/组织停用或到期立即生效。
pub async fn active_user(
    pool: &PgPool,
    user_id: Uuid,
    tenant_id: Uuid,
    org_id: Uuid,
    session_id: Uuid,
    version: i64,
) -> Result<ActiveUser, AppError> {
    let row: Option<ActiveUser> = sqlx::query_as(
        "SELECT u.username, COALESCE(u.real_name,''), u.is_super_admin \
         FROM platform_sessions s \
         JOIN users u ON u.id = s.user_id \
         JOIN organizations o ON o.id = s.organization_id AND o.tenant_id = s.tenant_id \
         JOIN tenants t ON t.id = s.tenant_id \
         JOIN memberships m ON m.user_id = s.user_id AND m.organization_id = s.organization_id \
         WHERE s.id = $1 AND s.user_id = $2 AND s.tenant_id = $3 AND s.organization_id = $4 \
           AND u.session_version = $5 AND u.status = 'ENABLED' \
           AND t.status = 'ENABLED' AND org_chain_active(o.id) \
           AND (t.expires_at IS NULL OR t.expires_at > now()) \
           AND s.revoked_at IS NULL AND s.expires_at > now()",
    )
    .bind(session_id)
    .bind(user_id)
    .bind(tenant_id)
    .bind(org_id)
    .bind(version)
    .fetch_optional(pool)
    .await?;
    row.ok_or_else(|| AppError::unauthorized("会话已失效，请重新登录"))
}

/// 修改密码或管理员踢出时原子提升版本并撤销全部平台 session。
pub async fn revoke_all(
    pool: &PgPool,
    user_id: Uuid,
    new_password_hash: Option<&str>,
) -> Result<u64, AppError> {
    let mut tx = pool.begin().await?;
    let revoked = revoke_all_in_tx(&mut tx, user_id, new_password_hash).await?;
    tx.commit().await?;
    Ok(revoked)
}

pub async fn revoke_all_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    new_password_hash: Option<&str>,
) -> Result<u64, AppError> {
    let updated = sqlx::query(
        "UPDATE users SET session_version = session_version + 1, \
         password_hash = COALESCE($2, password_hash), updated_at = now() WHERE id = $1",
    )
    .bind(user_id)
    .bind(new_password_hash)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(AppError::not_found("用户不存在"));
    }
    let revoked = sqlx::query(
        "UPDATE platform_sessions SET revoked_at = now() \
         WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok(revoked)
}

pub async fn revoke_one(pool: &PgPool, user_id: Uuid, session_id: Uuid) -> Result<(), AppError> {
    let changed = sqlx::query(
        "UPDATE platform_sessions SET revoked_at = now() \
         WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
    )
    .bind(session_id)
    .bind(user_id)
    .execute(pool)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::unauthorized("会话已失效，请重新登录"));
    }
    Ok(())
}
