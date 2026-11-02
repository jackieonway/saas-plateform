use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::config::Config;
use crate::error::AppError;
use crate::extractors::CurrentUser;
use crate::routes::contact::{normalize_phone, Contact};
use crate::state::AppState;
use axum::http::HeaderMap;
use rand::RngCore;
use serde::Serialize;
use serde_json::{json, Value};

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
pub(super) type PasswordContext<'a> = (
    &'a Config,
    &'a dyn crate::platform_accounts::AccountAdministrationPort,
);

/// Provision a contact in the same transaction as the entity and managed role.
/// The returned password is never persisted, logged, or returned by read APIs.
pub(super) async fn provision_contact_administrator(
    tx: &mut Transaction<'_, Postgres>,
    (config, accounts): PasswordContext<'_>,
    tenant_id: Uuid,
    org_id: Uuid,
    contact: &Contact,
    role_code: &str,
    existing_org_scope: Option<&[Uuid]>,
) -> Result<Option<ContactAdministrator>, AppError> {
    if contact.is_empty() {
        return Ok(None);
    }
    contact.validate(true)?;
    let existing = existing_contact_account(tx, contact, existing_org_scope).await?;
    let username = contact.username();
    let (user_id, username, created, initial_password) = if let Some((id, username)) = existing {
        (id, username, false, None)
    } else {
        // Shared phone/email values are not person identity keys. Distinct
        // contacts receive independent accounts and never inherit each other's grants.
        let occupied: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE username=$1)")
                .bind(&username)
                .fetch_one(&mut **tx)
                .await?;
        let username = if occupied {
            format!("admin-{}", crate::util::uuid7::new_uuid().simple())
        } else {
            username
        };
        let initial_password = initial_contact_password()?;
        let password = accounts.prepare_password(&initial_password).await?;
        let created = {
            let mut transaction = crate::authentication::PostgresAccountTransaction::new(tx);
            accounts
                .create(
                    &mut transaction,
                    crate::platform_accounts::NewAccount {
                        id: crate::util::uuid7::new_uuid(),
                        username: &username,
                        real_name: &contact.contact_name,
                        email: &contact.contact_email,
                        phone: &contact.contact_phone,
                        gender: "UNKNOWN",
                    },
                    &password,
                )
                .await?
        };
        let id = created.id;
        (id, username, true, Some(initial_password))
    };
    attach_contact_administrator(
        tx,
        config,
        tenant_id,
        org_id,
        role_code,
        ContactAccount {
            user_id,
            username,
            created,
            initial_password,
        },
    )
    .await
    .map(Some)
}

pub(super) fn initial_contact_password() -> Result<String, AppError> {
    let mut bytes = [0_u8; 16];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| anyhow::anyhow!("initial password generation failed"))?;
    Ok(hex::encode(bytes))
}

pub(super) async fn existing_contact_account(
    tx: &mut Transaction<'_, Postgres>,
    contact: &Contact,
    existing_org_scope: Option<&[Uuid]>,
) -> Result<Option<(Uuid, String)>, AppError> {
    let username = contact.username();
    let phone = normalize_phone(&contact.contact_phone);
    let mut locks = vec![format!("contact:username:{username}")];
    if !phone.is_empty() {
        locks.push(format!("contact:phone:{phone}"));
    }
    if !contact.contact_email.is_empty() {
        locks.push(format!("contact:email:{}", contact.contact_email));
    }
    locks.sort();
    locks.dedup();
    for key in locks {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(key)
            .execute(&mut **tx)
            .await?;
    }
    let existing: Vec<ContactUserRow> = sqlx::query_as(
        "SELECT id, username, COALESCE(real_name,''), COALESCE(phone,''), COALESCE(email,''), status, is_super_admin \
         FROM users u WHERE trim(COALESCE(real_name,''))=$1 \
         AND regexp_replace(COALESCE(phone,''), '[[:space:]()\\-]', '', 'g')=$2 \
         AND lower(trim(COALESCE(email,'')))=$3 AND status='ENABLED' AND is_super_admin=false \
         AND ($4::uuid[] IS NULL OR EXISTS(SELECT 1 FROM memberships m \
             WHERE m.user_id=u.id AND m.organization_id=ANY($4) \
             AND m.member_type='EMPLOYEE' AND m.status='ENABLED')) LIMIT 2 FOR UPDATE",
    ).bind(&contact.contact_name).bind(&phone).bind(&contact.contact_email).bind(existing_org_scope)
        .fetch_all(&mut **tx).await?;
    let mut existing = if existing.len() == 1 {
        existing.into_iter().next()
    } else {
        None
    };
    // Membership maintenance takes the same account lock. Recheck after waiting
    // for it so a concurrently removed member cannot be reused outside scope.
    if let (Some(candidate), Some(org_ids)) = (&existing, existing_org_scope) {
        let still_in_scope: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM memberships WHERE user_id=$1 AND organization_id=ANY($2) \
             AND member_type='EMPLOYEE' AND status='ENABLED')",
        )
        .bind(candidate.0)
        .bind(org_ids)
        .fetch_one(&mut **tx)
        .await?;
        if !still_in_scope {
            existing = None;
        }
    }
    Ok(existing.map(|(id, username, _, _, _, _, _)| (id, username)))
}

/// A confirmed account may be attached only by the platform transaction that
/// owns the organization, managed role and membership authorization.
pub(super) struct ContactAccount {
    pub user_id: Uuid,
    pub username: String,
    pub created: bool,
    pub initial_password: Option<String>,
}

pub(super) async fn attach_contact_administrator(
    tx: &mut Transaction<'_, Postgres>,
    config: &Config,
    tenant_id: Uuid,
    org_id: Uuid,
    role_code: &str,
    account: ContactAccount,
) -> Result<ContactAdministrator, AppError> {
    let ContactAccount {
        user_id,
        username,
        created,
        initial_password,
    } = account;
    let account_enabled: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM users WHERE id=$1 AND status='ENABLED' AND is_super_admin=false FOR NO KEY UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?;
    if account_enabled.is_none() {
        return Err(AppError::conflict("联系人账号不可用，请重新确认"));
    }
    let (role_id, role_name): (Uuid, String) = sqlx::query_as(
        "SELECT id,name FROM roles WHERE tenant_id=$1 AND organization_id=$2 AND code=$3 AND built_in=true",
    ).bind(tenant_id).bind(org_id).bind(role_code).fetch_one(&mut **tx).await?;
    crate::membership_lifecycle::ensure_employee_creation_allowed_in_tx(
        tx,
        user_id,
        org_id,
        config.dual_memberships_enabled,
    )
    .await?;
    sqlx::query("INSERT INTO memberships(user_id,organization_id,position,member_type, id) VALUES($1,$2,$3,'EMPLOYEE', $4) ON CONFLICT(user_id,organization_id,member_type) DO NOTHING")
        .bind(user_id).bind(org_id).bind(&role_name).bind(crate::util::uuid7::new_uuid()).execute(&mut **tx).await?;
    let employee_enabled: bool = sqlx::query_scalar(
        "SELECT status='ENABLED' FROM memberships \
         WHERE user_id=$1 AND organization_id=$2 AND member_type='EMPLOYEE'",
    )
    .bind(user_id)
    .bind(org_id)
    .fetch_one(&mut **tx)
    .await?;
    if !employee_enabled {
        return Err(AppError::conflict(
            "员工身份已停用，请通过成员生命周期接口显式启用",
        ));
    }
    sqlx::query("INSERT INTO user_roles(user_id,role_id,organization_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
        .bind(user_id).bind(role_id).bind(org_id).execute(&mut **tx).await?;
    if config.identity_remote_enabled {
        // Zero inserted rows is valid for an existing exact binding, but a
        // trigger may also silently suppress a missing assignment. Completion
        // must prove the role that this particular follow-up promised.
        let assigned:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM user_roles WHERE user_id=$1 AND role_id=$2 AND organization_id=$3)")
            .bind(user_id).bind(role_id).bind(org_id).fetch_one(&mut **tx).await?;
        if !assigned {
            return Err(AppError::Unavailable);
        }
    }
    Ok(ContactAdministrator {
        user_id,
        username,
        role_name,
        created,
        already_initialized: false,
        initial_password,
    })
}

pub(super) enum RemoteScope {
    Platform,
    OrganizationCreation {
        tenant_id: Uuid,
        parent_id: Option<Uuid>,
    },
    ExistingOrganization {
        tenant_id: Uuid,
        organization_id: Uuid,
    },
}

pub(super) struct RemoteRequirements {
    pub permission: &'static str,
    pub initializing: bool,
    pub scope: RemoteScope,
}

/// Revalidate permissions and target scope in the acceptance/follow-up
/// transaction. No account network request is made while this transaction lives.
pub(super) async fn require_remote_authority_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    actor: &CurrentUser,
    requirements: &RemoteRequirements,
) -> Result<Option<Vec<Uuid>>, AppError> {
    let rows = crate::authorization::context_menu_rows_in_tx(tx, actor, None).await?;
    let permissions = crate::authorization::permissions_of(&rows);
    let mut required = vec![requirements.permission];
    if requirements.initializing {
        required.extend(["system:user", "system:role"]);
    }
    if required
        .iter()
        .any(|permission| !permissions.contains(*permission))
    {
        return Err(AppError::forbidden("当前上下文没有管理员初始化权限"));
    }
    let platform: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM tenants t JOIN organizations o ON o.tenant_id=t.id \
         WHERE t.id=$1 AND o.id=$2 AND t.is_platform AND o.parent_id IS NULL)",
    )
    .bind(actor.tenant_id)
    .bind(actor.org_id)
    .fetch_one(&mut **tx)
    .await?;
    let scoped: Option<Vec<Uuid>> = if platform {
        None
    } else {
        Some(sqlx::query_scalar(
            "WITH RECURSIVE scoped AS (SELECT id FROM organizations WHERE id=$1 AND tenant_id=$2 \
             UNION SELECT o.id FROM organizations o JOIN scoped p ON o.parent_id=p.id \
             WHERE o.tenant_id=$2) SELECT id FROM scoped",
        ).bind(actor.org_id).bind(actor.tenant_id).fetch_all(&mut **tx).await?)
    };
    match &requirements.scope {
        RemoteScope::Platform if !platform => {
            return Err(AppError::forbidden("仅平台租户根组织可执行平台管理操作"));
        }
        RemoteScope::Platform => {}
        RemoteScope::OrganizationCreation {
            tenant_id,
            parent_id,
        } => {
            if !platform && *tenant_id != actor.tenant_id {
                return Err(AppError::not_found("租户不存在"));
            }
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tenants WHERE id=$1)")
                    .bind(tenant_id)
                    .fetch_one(&mut **tx)
                    .await?;
            if !exists {
                return Err(AppError::not_found("租户不存在"));
            }
            if let Some(parent_id) = parent_id {
                let exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM organizations WHERE id=$1 AND tenant_id=$2 \
                     AND ($3::uuid[] IS NULL OR id=ANY($3)))",
                )
                .bind(parent_id)
                .bind(tenant_id)
                .bind(scoped.as_deref())
                .fetch_one(&mut **tx)
                .await?;
                if !exists {
                    return Err(AppError::not_found("父组织不存在"));
                }
            } else if !platform {
                let root: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM organizations WHERE id=$1 AND tenant_id=$2 AND parent_id IS NULL)")
                    .bind(actor.org_id).bind(tenant_id).fetch_one(&mut **tx).await?;
                if !root {
                    return Err(AppError::forbidden("请在当前组织或下级组织下创建组织"));
                }
            }
        }
        RemoteScope::ExistingOrganization {
            tenant_id,
            organization_id,
        } => {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM organizations WHERE id=$1 AND tenant_id=$2 \
                 AND ($3::uuid[] IS NULL OR id=ANY($3)))",
            )
            .bind(organization_id)
            .bind(tenant_id)
            .bind(scoped.as_deref())
            .fetch_one(&mut **tx)
            .await?;
            if !exists {
                return Err(AppError::not_found("组织不存在"));
            }
            if requirements.permission == "system:tenant" && !platform {
                return Err(AppError::forbidden("仅平台租户根组织可执行平台管理操作"));
            }
        }
    }
    crate::authorization::require_employee_context_in_tx(tx, actor).await?;
    Ok(scoped)
}

pub(super) struct RemoteContact {
    pub account: Option<ContactAccount>,
    pub operation_id: Option<Uuid>,
}

/// Commit a durable, authorized intent before sending any create command. The
/// random initial password remains only in this stack frame and the one response.
pub(super) async fn prepare_remote_contact(
    state: &AppState,
    headers: &HeaderMap,
    actor: &CurrentUser,
    contact: &Contact,
    requirements: &RemoteRequirements,
    mut followup: Value,
) -> Result<RemoteContact, AppError> {
    let remote = state
        .remote_accounts
        .as_ref()
        .ok_or(AppError::Unavailable)?;
    let mut tx = state.pool.begin().await?;
    let scope = require_remote_authority_in_tx(&mut tx, actor, requirements).await?;
    if contact.is_empty() {
        tx.commit().await?;
        return Ok(RemoteContact {
            account: None,
            operation_id: None,
        });
    }
    contact.validate(true)?;
    if let Some((id, username)) =
        existing_contact_account(&mut tx, contact, scope.as_deref()).await?
    {
        tx.commit().await?;
        let snapshot = remote.snapshot(id).await?;
        if snapshot.status != "ENABLED" || snapshot.username != username {
            return Err(AppError::conflict("联系人账号不可用，请重新确认"));
        }
        return Ok(RemoteContact {
            account: Some(ContactAccount {
                user_id: id,
                username,
                created: false,
                initial_password: None,
            }),
            operation_id: None,
        });
    }
    let mut username = contact.username();
    let occupied: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE username=$1)")
        .bind(&username)
        .fetch_one(&mut *tx)
        .await?;
    if occupied {
        username = format!("admin-{}", crate::util::uuid7::new_uuid().simple());
    }
    let initial_password = initial_contact_password()?;
    let id = crate::util::uuid7::new_uuid();
    let command = remote.command(
        id,
        0,
        "CREATE",
        json!({
            "username":username, "realName":contact.contact_name, "email":contact.contact_email,
            "phone":contact.contact_phone, "gender":"UNKNOWN", "password":initial_password,
        }),
        None,
    )?;
    followup["account"] = json!({"id":id,"username":username});
    require_remote_authority_in_tx(&mut tx, actor, requirements).await?;
    crate::authentication::commands::accept_in_tx(&mut tx, actor, headers, &command, followup)
        .await?;
    tx.commit().await?;
    let operation_id = command.operation_id;
    let result = async {
        let receipt = crate::authentication::commands::dispatch(state, &command).await?;
        crate::authentication::commands::record_receipt(state, &receipt).await?;
        let snapshot = remote.snapshot(id).await?;
        if snapshot.status != "ENABLED" || snapshot.username != username {
            return Err(AppError::conflict("联系人账号不可用，请重新确认"));
        }
        Ok(RemoteContact {
            account: Some(ContactAccount {
                user_id: id,
                username,
                created: true,
                initial_password: Some(initial_password),
            }),
            operation_id: Some(operation_id),
        })
    }
    .await;
    result.map_err(|error| followup_error(error, Some(operation_id)))
}

pub(super) fn followup_error(error: AppError, operation: Option<Uuid>) -> AppError {
    match operation {
        Some(id) => AppError::conflict(format!(
            "账号操作 {id} 的平台后续工作尚未完成；请查询并恢复原操作"
        )),
        None => error,
    }
}

pub(super) fn same_contact(left: &Contact, right: &Contact) -> bool {
    left.contact_name == right.contact_name
        && left.contact_phone == right.contact_phone
        && left.contact_email == right.contact_email
}

pub(super) fn followup_uuid(value: &Value, field: &str) -> Result<Uuid, AppError> {
    use crate::util::uuid7::ParseUuid7;
    let encoded = value[field].as_str().ok_or(AppError::Unavailable)?;
    let id = encoded.parse_uuid7().map_err(|_| AppError::Unavailable)?;
    if id.to_string() != encoded {
        return Err(AppError::Unavailable);
    }
    Ok(id)
}

pub(super) async fn confirm_remote_contact_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    contact: &Contact,
    scope: Option<&[Uuid]>,
    remote: &RemoteContact,
) -> Result<(), AppError> {
    if let Some(account) = &remote.account {
        if !account.created
            && existing_contact_account(tx, contact, scope)
                .await?
                .map(|(id, _)| id)
                != Some(account.user_id)
        {
            return Err(AppError::conflict("联系人账号或成员范围已变更"));
        }
    }
    Ok(())
}

pub(super) struct PendingOnboarding {
    pub operation_id: Uuid,
    pub target_user_id: Uuid,
    pub digest: String,
    pub state: String,
    pub followup: Value,
    pub receipt: Option<Value>,
}

pub(super) async fn load_operation(
    state: &AppState,
    actor: &CurrentUser,
    operation: Uuid,
    kind_prefix: &str,
) -> Result<PendingOnboarding, AppError> {
    let row: Option<(Uuid, String, String, Value, Option<Value>)> = sqlx::query_as(
        "SELECT target_user_id,canonical_digest,state,followup,receipt \
         FROM platform_account_command_intents WHERE id=$1 AND actor_user_id=$2",
    )
    .bind(operation)
    .bind(actor.user_id)
    .fetch_optional(&state.pool)
    .await?;
    let (target_user_id, digest, status, followup, receipt) =
        row.ok_or_else(|| AppError::not_found("账号操作不存在"))?;
    if !followup["kind"]
        .as_str()
        .is_some_and(|kind| kind.starts_with(kind_prefix))
    {
        return Err(AppError::not_found("账号操作不存在"));
    }
    Ok(PendingOnboarding {
        operation_id: operation,
        target_user_id,
        digest,
        state: status,
        followup,
        receipt,
    })
}

pub(super) async fn recover_contact(
    state: &AppState,
    pending: &PendingOnboarding,
) -> Result<RemoteContact, AppError> {
    if pending.state == "COMPLETED" || pending.state == "REJECTED" {
        return Err(AppError::conflict("账号操作已结束"));
    }
    if let Some(receipt) = pending.receipt.as_ref() {
        confirmed_receipt(
            receipt,
            pending.operation_id,
            pending.target_user_id,
            &pending.digest,
        )?;
    } else {
        let receipt = crate::authentication::commands::reconcile(
            state,
            pending.operation_id,
            pending.target_user_id,
            &pending.digest,
        )
        .await?;
        crate::authentication::commands::record_receipt(state, &receipt).await?;
    }
    let remote = state
        .remote_accounts
        .as_ref()
        .ok_or(AppError::Unavailable)?;
    let snapshot = remote.snapshot(pending.target_user_id).await?;
    let username = pending.followup["account"]["username"]
        .as_str()
        .ok_or(AppError::Unavailable)?;
    if snapshot.status != "ENABLED" || snapshot.username != username {
        return Err(AppError::conflict("联系人账号不可用，请重新确认"));
    }
    Ok(RemoteContact {
        account: Some(ContactAccount {
            user_id: pending.target_user_id,
            username: username.to_owned(),
            created: true,
            initial_password: None,
        }),
        operation_id: Some(pending.operation_id),
    })
}

fn confirmed_receipt(
    value: &Value,
    operation: Uuid,
    target: Uuid,
    digest: &str,
) -> Result<(), AppError> {
    let receipt: crate::authentication::remote::Receipt =
        serde_json::from_value(value.clone()).map_err(|_| AppError::Unavailable)?;
    let before: i64 = receipt
        .before_version
        .parse()
        .map_err(|_| AppError::Unavailable)?;
    let after: i64 = receipt
        .after_version
        .parse()
        .map_err(|_| AppError::Unavailable)?;
    if receipt.operation_id != operation
        || receipt.target_user_id != target
        || receipt.canonical_digest != digest
        || receipt.state != "COMPLETED"
        || digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || before != 0
        || after != 1
        || before.to_string() != receipt.before_version
        || after.to_string() != receipt.after_version
    {
        return Err(AppError::Unavailable);
    }
    Ok(())
}

pub(super) async fn lock_operation_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    remote: &RemoteContact,
) -> Result<(), AppError> {
    if let Some(operation) = remote.operation_id {
        let (state, target, digest, receipt): (String, Uuid, String, Option<Value>) = sqlx::query_as(
            "SELECT state,target_user_id,canonical_digest,receipt FROM platform_account_command_intents WHERE id=$1 FOR UPDATE",
        ).bind(operation).fetch_one(&mut **tx).await?;
        if state != "UNKNOWN"
            || remote.account.as_ref().map(|account| account.user_id) != Some(target)
        {
            return Err(AppError::conflict("账号操作已结束或状态不允许"));
        }
        confirmed_receipt(
            receipt.as_ref().ok_or(AppError::Unavailable)?,
            operation,
            target,
            &digest,
        )?;
    }
    Ok(())
}

pub(super) async fn complete_operation_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    operation: Option<Uuid>,
) -> Result<(), AppError> {
    if let Some(operation) = operation {
        crate::authentication::commands::complete_in_tx(tx, operation).await?;
    }
    Ok(())
}

/// Prepare permissions and an unassigned administrator role. Accounts and role
/// membership are maintained separately through the user and role APIs.
pub(super) async fn initialize_tenant(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    root_id: Uuid,
) -> Result<(), AppError> {
    create_administrator_role(tx, tenant_id, root_id, "超级管理员", "tenant_admin").await?;
    let system: Uuid = sqlx::query_scalar(
        "SELECT id FROM applications WHERE code = 'system' AND status = 'ENABLED'",
    )
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| AppError::bad_request("系统管理应用尚未配置"))?;
    sqlx::query("INSERT INTO tenant_applications (tenant_id, application_id) VALUES ($1, $2)")
        .bind(tenant_id)
        .bind(system)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "INSERT INTO organization_applications (organization_id, application_id) VALUES ($1, $2)",
    )
    .bind(root_id)
    .bind(system)
    .execute(&mut **tx)
    .await?;
    let menus: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM menus WHERE application_id = $1 AND status = 'ENABLED' \
         AND permission = ANY($2)",
    )
    .bind(system)
    .bind(management_permissions())
    .fetch_all(&mut **tx)
    .await?;
    grant_menus(tx, tenant_id, root_id, &menus, true).await?;
    Ok(())
}

pub(super) async fn initialize_organization(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    org_id: Uuid,
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
    )
    .bind(tenant_id)
    .bind(parent_id)
    .bind(management_permissions())
    .fetch_all(&mut **tx)
    .await?;
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
    vec![
        "dashboard:view",
        "system:organization",
        "system:department",
        "system:authorization",
        "system:user",
        "system:role",
    ]
}

async fn create_administrator_role(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    org_id: Uuid,
    name: &str,
    code: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO roles (tenant_id, organization_id, name, code, built_in, id) \
         VALUES ($1, $2, $3, $4, true, $5)",
    )
    .bind(tenant_id)
    .bind(org_id)
    .bind(name)
    .bind(code)
    .bind(crate::util::uuid7::new_uuid())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn grant_menus(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    org_id: Uuid,
    menus: &[Uuid],
    grant_tenant: bool,
) -> Result<(), AppError> {
    if grant_tenant {
        sqlx::query("INSERT INTO tenant_menus (tenant_id, menu_id) SELECT $1, unnest($2::uuid[])")
            .bind(tenant_id)
            .bind(menus)
            .execute(&mut **tx)
            .await?;
    }
    sqlx::query(
        "INSERT INTO organization_menus (organization_id, menu_id) SELECT $1, unnest($2::uuid[])",
    )
    .bind(org_id)
    .bind(menus)
    .execute(&mut **tx)
    .await?;
    sqlx::query("INSERT INTO role_menus (role_id, menu_id) \
         SELECT r.id, unnest($2::uuid[]) FROM roles r WHERE r.organization_id = $1 AND r.built_in = true \
         AND r.code IN ('tenant_admin', 'org_admin')")
        .bind(org_id).bind(menus).execute(&mut **tx).await?;
    Ok(())
}

/// Tenant authorization is the ceiling. The root organization and its managed
/// administrator role follow it, so a newly granted menu is usable immediately.
pub(super) async fn sync_tenant_admin(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
) -> Result<(), AppError> {
    let root: Option<Uuid> = sqlx::query_scalar(
        "SELECT o.id FROM organizations o JOIN roles r ON r.organization_id = o.id \
         WHERE o.tenant_id = $1 AND o.parent_id IS NULL AND r.code = 'tenant_admin' AND r.built_in = true",
    ).bind(tenant_id).fetch_optional(&mut **tx).await?;
    let Some(root) = root else {
        return Ok(());
    };
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
    tx: &mut Transaction<'_, Postgres>,
    org_id: Uuid,
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
    tx: &mut Transaction<'_, Postgres>,
    passwords: PasswordContext<'_>,
    tenant_id: Uuid,
    org_id: Uuid,
    contact: &Contact,
    role_code: &str,
    existing_org_scope: Option<&[Uuid]>,
) -> Result<ContactAdministrator, AppError> {
    if let Some(administrator) = existing_administrator(tx, org_id, role_code).await? {
        return Ok(administrator);
    }
    ensure_administrator_role(tx, tenant_id, org_id, contact, role_code).await?;
    provision_contact_administrator(
        tx,
        passwords,
        tenant_id,
        org_id,
        contact,
        role_code,
        existing_org_scope,
    )
    .await?
    .ok_or_else(|| AppError::bad_request("联系人信息不完整"))
}

pub(super) async fn existing_administrator(
    tx: &mut Transaction<'_, Postgres>,
    org_id: Uuid,
    role_code: &str,
) -> Result<Option<ContactAdministrator>, AppError> {
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR NO KEY UPDATE")
        .bind(org_id)
        .fetch_one(&mut **tx)
        .await?;
    let existing: Option<(Uuid, String, String, String, String, bool, bool)> = sqlx::query_as(
        "SELECT u.id,u.username,r.name,u.status,r.status,r.built_in AND r.code=$2, \
         EXISTS(SELECT 1 FROM memberships m WHERE m.user_id=u.id AND m.organization_id=ur.organization_id \
         AND m.member_type='EMPLOYEE' AND m.status='ENABLED') \
         FROM user_roles ur JOIN roles r ON r.id=ur.role_id JOIN users u ON u.id=ur.user_id \
         WHERE ur.organization_id=$1 AND r.organization_id=$1 \
         AND (r.code IN ('tenant_admin','org_admin','super_admin','star_admin') \
              OR r.name IN ('租户管理员','组织管理员','平台管理员','超级管理员')) \
         ORDER BY ur.created_at,u.id LIMIT 1",
    ).bind(org_id).bind(role_code).fetch_optional(&mut **tx).await?;
    if let Some((user_id, username, role_name, user_status, role_status, managed, member)) =
        existing
    {
        if !managed || !member || user_status != "ENABLED" || role_status != "ENABLED" {
            return Err(AppError::bad_request(
                "该组织已有管理员，请在用户管理和角色管理中单独维护",
            ));
        }
        return Ok(Some(ContactAdministrator {
            user_id,
            username,
            role_name,
            created: false,
            already_initialized: true,
            initial_password: None,
        }));
    }
    Ok(None)
}

pub(super) async fn ensure_administrator_role(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    org_id: Uuid,
    contact: &Contact,
    role_code: &str,
) -> Result<(), AppError> {
    if contact.is_empty() {
        return Err(AppError::bad_request(
            "请先保存联系人姓名，并至少填写电话或 Email",
        ));
    }
    contact.validate(true)?;
    let role: Option<(Uuid, bool, String)> =
        sqlx::query_as("SELECT id,built_in,status FROM roles WHERE organization_id=$1 AND code=$2")
            .bind(org_id)
            .bind(role_code)
            .fetch_optional(&mut **tx)
            .await?;
    match role {
        Some((_, true, status)) if status == "ENABLED" => {}
        Some(_) => {
            return Err(AppError::bad_request(
                "管理员角色不可用，请在角色管理中维护",
            ))
        }
        None => {
            let name = "超级管理员";
            let occupied: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM roles WHERE organization_id=$1 AND name=$2)",
            )
            .bind(org_id)
            .bind(name)
            .fetch_one(&mut **tx)
            .await?;
            if occupied {
                return Err(AppError::bad_request(
                    "已有同名管理员角色，请在角色管理中维护",
                ));
            }
            create_administrator_role(tx, tenant_id, org_id, name, role_code).await?;
        }
    }
    sync_org_admin(tx, org_id).await?;
    Ok(())
}

#[cfg(test)]
mod recovery_contract_tests {
    use super::{confirmed_receipt, followup_error, followup_uuid, same_contact};
    use crate::{error::AppError, routes::contact::Contact};
    use serde_json::json;

    #[test]
    fn stored_receipt_must_match_the_exact_operation_target_and_digest() {
        let operation = crate::util::uuid7::new_uuid();
        let target = crate::util::uuid7::new_uuid();
        let digest = "a".repeat(64);
        let receipt = json!({"operationId":operation,"targetUserId":target,"canonicalDigest":digest,
            "state":"COMPLETED","beforeVersion":"0","afterVersion":"1","revokedSessions":0,"revokedTokens":0});
        assert!(confirmed_receipt(&receipt, operation, target, &digest).is_ok());
        for (field, changed) in [
            ("operationId", json!(crate::util::uuid7::new_uuid())),
            ("targetUserId", json!(crate::util::uuid7::new_uuid())),
            ("canonicalDigest", json!("b".repeat(64))),
            ("state", json!("UNKNOWN")),
            ("afterVersion", json!("0")),
            ("afterVersion", json!("2")),
            ("beforeVersion", json!("1")),
            ("afterVersion", json!("-1")),
            ("beforeVersion", json!("00")),
        ] {
            let mut corrupted = receipt.clone();
            corrupted[field] = changed;
            assert!(confirmed_receipt(&corrupted, operation, target, &digest).is_err());
        }
    }

    #[test]
    fn recovery_rejects_noncanonical_or_non_v7_entity_ids() {
        let id = uuid::Uuid::parse_str("019201e1-8a00-7000-8000-000000000abc").unwrap();
        assert_eq!(
            followup_uuid(&json!({"entityId":id}), "entityId").unwrap(),
            id
        );
        for value in [
            json!({}),
            json!({"entityId":7}),
            json!({"entityId":uuid::Uuid::new_v4()}),
            json!({"entityId":id.to_string().to_uppercase()}),
            json!({"entityId":format!(" {id}")}),
        ] {
            assert!(followup_uuid(&value, "entityId").is_err());
        }
    }

    #[test]
    fn changed_contact_cannot_consume_an_original_initialization_intent() {
        let first = Contact {
            contact_name: "甲".into(),
            contact_phone: "13800000000".into(),
            contact_email: "a@example.test".into(),
        };
        let mut changed = Contact {
            contact_name: first.contact_name.clone(),
            contact_phone: first.contact_phone.clone(),
            contact_email: first.contact_email.clone(),
        };
        assert!(same_contact(&first, &changed));
        changed.contact_email = "b@example.test".into();
        assert!(!same_contact(&first, &changed));
        let operation = crate::util::uuid7::new_uuid();
        let error = followup_error(AppError::Unavailable, Some(operation));
        assert!(matches!(error, AppError::Conflict(_)));
        assert!(error.to_string().contains(&operation.to_string()));
        assert!(matches!(
            followup_error(AppError::Unavailable, None),
            AppError::Unavailable
        ));
    }
}
