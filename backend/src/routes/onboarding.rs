use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::error::AppError;
use crate::config::Config;
use crate::routes::contact::{normalize_phone, Contact};
use crate::util::password;
use rand::RngCore;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ContactAdministrator {
    pub user_id: Uuid,
    pub username: String,
    pub role_name: String,
    pub created: bool,
    pub already_initialized: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_password: Option<String>,
}

type ContactUserRow = (Uuid, String, String, String, String, String, bool);

/// Credential policy and the shared process budget are passed as one capability.
pub(super) type PasswordContext<'a> = (&'a Config, &'a password::PasswordService);

/// Provision a contact in the same transaction as the entity and managed role.
/// The returned password is never persisted, logged, or returned by read APIs.
pub(super) async fn provision_contact_administrator(
    tx: &mut Transaction<'_, Postgres>, (config, passwords): PasswordContext<'_>, tenant_id: Uuid, org_id: Uuid,
    contact: &Contact, role_code: &str, existing_org_scope: Option<&[Uuid]>,
) -> Result<Option<ContactAdministrator>, AppError> {
    if contact.is_empty() { return Ok(None); }
    contact.validate(true)?;
    let username = contact.username();
    let phone = normalize_phone(&contact.contact_phone);
    let mut locks = vec![format!("contact:username:{username}")];
    if !phone.is_empty() { locks.push(format!("contact:phone:{phone}")); }
    if !contact.contact_email.is_empty() { locks.push(format!("contact:email:{}", contact.contact_email)); }
    locks.sort();
    locks.dedup();
    for key in locks {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(key).execute(&mut **tx).await?;
    }
    let existing: Vec<ContactUserRow> = sqlx::query_as(
        "SELECT id, username, COALESCE(real_name,''), COALESCE(phone,''), COALESCE(email,''), status, is_super_admin \
         FROM users u WHERE trim(COALESCE(real_name,''))=$1 \
         AND regexp_replace(COALESCE(phone,''), '[[:space:]()\\-]', '', 'g')=$2 \
         AND lower(trim(COALESCE(email,'')))=$3 AND status='ENABLED' AND is_super_admin=false \
         AND ($4::uuid[] IS NULL OR EXISTS(SELECT 1 FROM memberships m \
             WHERE m.user_id=u.id AND m.organization_id=ANY($4))) LIMIT 2 FOR UPDATE",
    ).bind(&contact.contact_name).bind(&phone).bind(&contact.contact_email).bind(existing_org_scope)
        .fetch_all(&mut **tx).await?;
    let mut existing = if existing.len() == 1 { existing.into_iter().next() } else { None };
    // Membership maintenance takes the same account lock. Recheck after waiting
    // for it so a concurrently removed member cannot be reused outside scope.
    if let (Some(candidate), Some(org_ids)) = (&existing, existing_org_scope) {
        let still_in_scope: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM memberships WHERE user_id=$1 AND organization_id=ANY($2))",
        ).bind(candidate.0).bind(org_ids).fetch_one(&mut **tx).await?;
        if !still_in_scope { existing = None; }
    }
    let (user_id, username, created, initial_password) = if let Some((id, username, _, _, _, _, _)) = existing {
        (id, username, false, None)
    } else {
        // Shared phone/email values are not person identity keys. Distinct
        // contacts receive independent accounts and never inherit each other's grants.
        let occupied: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE username=$1)")
            .bind(&username).fetch_one(&mut **tx).await?;
        let username = if occupied { format!("admin-{}", Uuid::new_v4().simple()) } else { username };
        let mut bytes = [0_u8; 16];
        rand::rngs::OsRng.try_fill_bytes(&mut bytes).map_err(|_| anyhow::anyhow!("initial password generation failed"))?;
        let initial_password = hex::encode(bytes);
        password::validate_for_config(&initial_password, config)?;
        let hash = passwords.hash(&initial_password).await?;
        let id = sqlx::query_scalar(
            "INSERT INTO users(username,password_hash,real_name,phone,email) VALUES($1,$2,$3,$4,$5) RETURNING id",
        ).bind(&username).bind(hash).bind(&contact.contact_name).bind(&contact.contact_phone)
            .bind(&contact.contact_email).fetch_one(&mut **tx).await?;
        (id, username, true, Some(initial_password))
    };
    let (role_id, role_name): (Uuid, String) = sqlx::query_as(
        "SELECT id,name FROM roles WHERE tenant_id=$1 AND organization_id=$2 AND code=$3 AND built_in=true",
    ).bind(tenant_id).bind(org_id).bind(role_code).fetch_one(&mut **tx).await?;
    sqlx::query("INSERT INTO memberships(user_id,organization_id,position) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
        .bind(user_id).bind(org_id).bind(&role_name).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
        .bind(user_id).bind(role_id).bind(org_id).execute(&mut **tx).await?;
    Ok(Some(ContactAdministrator { user_id, username, role_name, created, already_initialized: false, initial_password }))
}

/// Prepare permissions and an unassigned administrator role. Accounts and role
/// membership are maintained separately through the user and role APIs.
pub(super) async fn initialize_tenant(
    tx: &mut Transaction<'_, Postgres>, tenant_id: Uuid, root_id: Uuid,
) -> Result<(), AppError> {
    create_administrator_role(tx, tenant_id, root_id, "超级管理员", "tenant_admin").await?;
    let system: Uuid = sqlx::query_scalar("SELECT id FROM applications WHERE code = 'system' AND status = 'ENABLED'")
        .fetch_optional(&mut **tx).await?.ok_or_else(|| AppError::bad_request("系统管理应用尚未配置"))?;
    sqlx::query("INSERT INTO tenant_applications (tenant_id, application_id) VALUES ($1, $2)")
        .bind(tenant_id).bind(system).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO organization_applications (organization_id, application_id) VALUES ($1, $2)")
        .bind(root_id).bind(system).execute(&mut **tx).await?;
    let menus: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM menus WHERE application_id = $1 AND status = 'ENABLED' \
         AND permission = ANY($2)",
    ).bind(system).bind(management_permissions()).fetch_all(&mut **tx).await?;
    grant_menus(tx, tenant_id, root_id, &menus, true).await?;
    Ok(())
}

pub(super) async fn initialize_organization(
    tx: &mut Transaction<'_, Postgres>, tenant_id: Uuid, org_id: Uuid,
    parent_id: Option<Uuid>,
) -> Result<(), AppError> {
    let Some(parent_id) = parent_id else {
        create_administrator_role(tx, tenant_id, org_id, "超级管理员", "tenant_admin").await?;
        return sync_tenant_admin(tx, tenant_id).await;
    };
    create_administrator_role(tx, tenant_id, org_id, "超级管理员", "org_admin").await?;
    let menus: Vec<Uuid> = sqlx::query_scalar(
        "SELECT m.id FROM menus m JOIN tenant_menus tm ON tm.menu_id = m.id AND tm.tenant_id = $1 \
         JOIN organization_menus om ON om.menu_id = m.id AND om.organization_id = $2 \
         WHERE m.status = 'ENABLED' AND m.permission = ANY($3)",
    ).bind(tenant_id).bind(parent_id).bind(management_permissions())
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

async fn create_administrator_role(
    tx: &mut Transaction<'_, Postgres>, tenant_id: Uuid,
    org_id: Uuid, name: &str, code: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO roles (tenant_id, organization_id, name, code, built_in) \
         VALUES ($1, $2, $3, $4, true)",
    ).bind(tenant_id).bind(org_id).bind(name).bind(code).execute(&mut **tx).await?;
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

/// Explicit, serialized initialization for existing entities. Never replaces an
/// administrator or changes an existing account password/profile.
pub(super) async fn initialize_existing_administrator(
    tx: &mut Transaction<'_, Postgres>, passwords: PasswordContext<'_>, tenant_id: Uuid,
    org_id: Uuid, contact: &Contact, role_code: &str, existing_org_scope: Option<&[Uuid]>,
) -> Result<ContactAdministrator, AppError> {
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR NO KEY UPDATE")
        .bind(org_id).fetch_one(&mut **tx).await?;
    let existing: Option<(Uuid, String, String, String, String, bool, bool)> = sqlx::query_as(
        "SELECT u.id,u.username,r.name,u.status,r.status,r.built_in AND r.code=$2, \
         EXISTS(SELECT 1 FROM memberships m WHERE m.user_id=u.id AND m.organization_id=ur.organization_id) \
         FROM user_roles ur JOIN roles r ON r.id=ur.role_id JOIN users u ON u.id=ur.user_id \
         WHERE ur.organization_id=$1 AND r.organization_id=$1 \
         AND (r.code IN ('tenant_admin','org_admin','super_admin','star_admin') \
              OR r.name IN ('租户管理员','组织管理员','平台管理员','超级管理员')) \
         ORDER BY ur.created_at,u.id LIMIT 1",
    ).bind(org_id).bind(role_code).fetch_optional(&mut **tx).await?;
    if let Some((user_id, username, role_name, user_status, role_status, managed, member)) = existing {
        if !managed || !member || user_status != "ENABLED" || role_status != "ENABLED" {
            return Err(AppError::bad_request("该组织已有管理员，请在用户管理和角色管理中单独维护"));
        }
        return Ok(ContactAdministrator { user_id, username, role_name, created: false,
            already_initialized: true, initial_password: None });
    }
    if contact.is_empty() {
        return Err(AppError::bad_request("请先保存联系人姓名，并至少填写电话或 Email"));
    }
    contact.validate(true)?;
    let role: Option<(Uuid, bool, String)> = sqlx::query_as(
        "SELECT id,built_in,status FROM roles WHERE organization_id=$1 AND code=$2",
    ).bind(org_id).bind(role_code).fetch_optional(&mut **tx).await?;
    match role {
        Some((_, true, status)) if status == "ENABLED" => {},
        Some(_) => return Err(AppError::bad_request("管理员角色不可用，请在角色管理中维护")),
        None => {
            let name = "超级管理员";
            let occupied: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM roles WHERE organization_id=$1 AND name=$2)")
                .bind(org_id).bind(name).fetch_one(&mut **tx).await?;
            if occupied { return Err(AppError::bad_request("已有同名管理员角色，请在角色管理中维护")); }
            create_administrator_role(tx, tenant_id, org_id, name, role_code).await?;
        }
    }
    sync_org_admin(tx, org_id).await?;
    provision_contact_administrator(tx, passwords, tenant_id, org_id, contact, role_code, existing_org_scope)
        .await?.ok_or_else(|| AppError::bad_request("联系人信息不完整"))
}
