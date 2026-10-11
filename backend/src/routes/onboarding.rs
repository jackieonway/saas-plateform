use serde::Deserialize;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::config::Config;
use crate::error::AppError;
use crate::util::password;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Administrator {
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub real_name: String,
    #[serde(default)]
    pub email: String,
}

async fn administrator_user(
    tx: &mut Transaction<'_, Postgres>, config: &Config, admin: &Administrator,
    tenant_id: Uuid, require_existing_membership: bool,
) -> Result<Uuid, AppError> {
    let username = admin.username.trim();
    if username.is_empty() { return Err(AppError::bad_request("管理员用户名不能为空")); }
    let existing: Option<(Uuid, String, bool)> = sqlx::query_as(
        "SELECT id, status, is_super_admin FROM users WHERE username = $1 FOR UPDATE",
    ).bind(username).fetch_optional(&mut **tx).await?;
    if let Some((id, status, super_admin)) = existing {
        if status != "ENABLED" { return Err(AppError::bad_request("管理员账号未启用")); }
        if super_admin { return Err(AppError::bad_request("平台超级管理员不能作为租户或组织的默认管理员")); }
        if !admin.password.is_empty() {
            return Err(AppError::bad_request("该用户名已存在；选择已有账号时请勿填写初始密码"));
        }
        if require_existing_membership {
            let belongs: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM memberships m JOIN organizations o ON o.id = m.organization_id \
                 WHERE m.user_id = $1 AND o.tenant_id = $2)",
            ).bind(id).bind(tenant_id).fetch_one(&mut **tx).await?;
            if !belongs { return Err(AppError::bad_request("组织管理员必须是本租户已有成员，或创建新账号")); }
        }
        Ok(id)
    } else {
        if admin.password.is_empty() { return Err(AppError::bad_request("新管理员账号需要填写初始密码")); }
        password::validate_for_config(&admin.password, config)?;
        let hash = password::hash_password(&admin.password)?;
        let id = sqlx::query_scalar(
            "INSERT INTO users (username, password_hash, real_name, email) VALUES ($1, $2, $3, $4) RETURNING id",
        ).bind(username).bind(hash).bind(admin.real_name.trim()).bind(admin.email.trim())
            .fetch_one(&mut **tx).await?;
        Ok(id)
    }
}

pub(super) async fn provision_tenant(
    tx: &mut Transaction<'_, Postgres>, config: &Config, tenant_id: Uuid, root_id: Uuid,
    admin: &Administrator,
) -> Result<(), AppError> {
    let user_id = administrator_user(tx, config, admin, tenant_id, false).await?;
    bind_administrator(tx, user_id, tenant_id, root_id, "租户管理员", "tenant_admin").await?;
    let system: Uuid = sqlx::query_scalar("SELECT id FROM applications WHERE code = 'system' AND status = 'ENABLED'")
        .fetch_optional(&mut **tx).await?.ok_or_else(|| AppError::bad_request("系统管理应用尚未配置"))?;
    sqlx::query("INSERT INTO tenant_applications (tenant_id, application_id) VALUES ($1, $2)")
        .bind(tenant_id).bind(system).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO organization_applications (organization_id, application_id) VALUES ($1, $2)")
        .bind(root_id).bind(system).execute(&mut **tx).await?;
    let menus: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM menus WHERE application_id = $1 AND status = 'ENABLED' \
         AND permission = ANY($2)",
    ).bind(system).bind(&management_permissions()).fetch_all(&mut **tx).await?;
    grant_menus(tx, tenant_id, root_id, &menus, true).await?;
    Ok(())
}

pub(super) async fn provision_organization(
    tx: &mut Transaction<'_, Postgres>, config: &Config, tenant_id: Uuid, org_id: Uuid,
    parent_id: Uuid, admin: &Administrator,
) -> Result<(), AppError> {
    let user_id = administrator_user(tx, config, admin, tenant_id, true).await?;
    bind_administrator(tx, user_id, tenant_id, org_id, "组织管理员", "org_admin").await?;
    let menus: Vec<Uuid> = sqlx::query_scalar(
        "SELECT m.id FROM menus m JOIN tenant_menus tm ON tm.menu_id = m.id AND tm.tenant_id = $1 \
         JOIN organization_menus om ON om.menu_id = m.id AND om.organization_id = $2 \
         WHERE m.status = 'ENABLED' AND m.permission = ANY($3)",
    ).bind(tenant_id).bind(parent_id).bind(&management_permissions())
        .fetch_all(&mut **tx).await?;
    sqlx::query(
        "INSERT INTO organization_applications (organization_id, application_id) \
         SELECT $1, oa.application_id FROM organization_applications oa \
         JOIN tenant_applications ta ON ta.application_id = oa.application_id AND ta.tenant_id = $3 \
         WHERE oa.organization_id = $2 AND oa.application_id IN \
         (SELECT DISTINCT application_id FROM menus WHERE id = ANY($4)) ON CONFLICT DO NOTHING",
    ).bind(org_id).bind(parent_id).bind(tenant_id).bind(&menus).execute(&mut **tx).await?;
    grant_menus(tx, tenant_id, org_id, &menus, false).await?;
    Ok(())
}

fn management_permissions() -> Vec<&'static str> {
    vec!["dashboard:view", "system:organization", "system:department", "system:authorization", "system:user", "system:role"]
}

async fn bind_administrator(
    tx: &mut Transaction<'_, Postgres>, user_id: Uuid, tenant_id: Uuid,
    org_id: Uuid, name: &str, code: &str,
) -> Result<(), AppError> {
    sqlx::query("INSERT INTO memberships (user_id, organization_id, position) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING")
        .bind(user_id).bind(org_id).bind(name).execute(&mut **tx).await?;
    let role_id: Uuid = sqlx::query_scalar(
        "INSERT INTO roles (tenant_id, organization_id, name, code, built_in) \
         VALUES ($1, $2, $3, $4, true) RETURNING id",
    ).bind(tenant_id).bind(org_id).bind(name).bind(code).fetch_one(&mut **tx).await?;
    sqlx::query("INSERT INTO user_roles (user_id, role_id, organization_id) VALUES ($1, $2, $3)")
        .bind(user_id).bind(role_id).bind(org_id).execute(&mut **tx).await?;
    Ok(())
}

async fn grant_menus(
    tx: &mut Transaction<'_, Postgres>, tenant_id: Uuid, org_id: Uuid,
    menus: &[Uuid], grant_tenant: bool,
) -> Result<(), AppError> {
    if grant_tenant {
        sqlx::query("INSERT INTO tenant_menus (tenant_id, menu_id) SELECT $1, unnest($2::uuid[])")
            .bind(tenant_id).bind(menus).execute(&mut **tx).await?;
    }
    sqlx::query("INSERT INTO organization_menus (organization_id, menu_id) SELECT $1, unnest($2::uuid[])")
        .bind(org_id).bind(menus).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO role_menus (role_id, menu_id) \
         SELECT r.id, unnest($2::uuid[]) FROM roles r WHERE r.organization_id = $1 AND r.built_in = true \
         AND r.code IN ('tenant_admin', 'org_admin')")
        .bind(org_id).bind(menus).execute(&mut **tx).await?;
    Ok(())
}

/// Tenant authorization is the ceiling. The root organization and its managed
/// administrator role follow it, so a newly granted menu is usable immediately.
pub(super) async fn sync_tenant_admin(
    tx: &mut Transaction<'_, Postgres>, tenant_id: Uuid,
) -> Result<(), AppError> {
    let root: Option<Uuid> = sqlx::query_scalar(
        "SELECT o.id FROM organizations o JOIN roles r ON r.organization_id = o.id \
         WHERE o.tenant_id = $1 AND o.parent_id IS NULL AND r.code = 'tenant_admin' AND r.built_in = true",
    ).bind(tenant_id).fetch_optional(&mut **tx).await?;
    let Some(root) = root else { return Ok(()); };
    sqlx::query(
        "INSERT INTO organization_applications (organization_id, application_id) \
         SELECT $1, application_id FROM tenant_applications WHERE tenant_id = $2 ON CONFLICT DO NOTHING",
    ).bind(root).bind(tenant_id).execute(&mut **tx).await?;
    sqlx::query(
        "INSERT INTO organization_menus (organization_id, menu_id) \
         SELECT $1, tm.menu_id FROM tenant_menus tm JOIN menus m ON m.id = tm.menu_id \
         JOIN tenant_applications ta ON ta.tenant_id = tm.tenant_id AND ta.application_id = m.application_id \
         WHERE tm.tenant_id = $2 ON CONFLICT DO NOTHING",
    ).bind(root).bind(tenant_id).execute(&mut **tx).await?;
    sync_org_admin(tx, root).await
}

pub(super) async fn sync_org_admin(
    tx: &mut Transaction<'_, Postgres>, org_id: Uuid,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO role_menus (role_id, menu_id) \
         SELECT r.id, om.menu_id FROM roles r JOIN organization_menus om ON om.organization_id = r.organization_id \
         JOIN organizations o ON o.id = r.organization_id \
         JOIN tenant_menus tm ON tm.tenant_id = o.tenant_id AND tm.menu_id = om.menu_id \
         JOIN menus m ON m.id = om.menu_id \
         JOIN organization_applications oa ON oa.organization_id = o.id AND oa.application_id = m.application_id \
         WHERE r.organization_id = $1 AND r.built_in = true AND r.code IN ('tenant_admin', 'org_admin') \
         ON CONFLICT DO NOTHING",
    ).bind(org_id).execute(&mut **tx).await?;
    Ok(())
}
