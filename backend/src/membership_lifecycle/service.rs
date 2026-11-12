use std::collections::HashMap;
use std::str::FromStr;

use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::authorization;
use crate::error::AppError;
use crate::extractors::CurrentUser;
use crate::identity_policy::{MemberType, MembershipStatus};
use crate::routes::{page_json, paging};
use crate::state::AppState;
use crate::util::{audit, uuid7};

const COLUMNS: &str = "m.id, m.user_id, m.organization_id, m.member_type, m.status, \
    m.identity_version, m.department_id, COALESCE(m.position,'') AS position, m.updated_at";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NewAccountMembership {
    #[serde(deserialize_with = "uuid7::deserialize")]
    pub organization_id: Uuid,
    pub member_type: MemberType,
    #[serde(default)]
    pub position: String,
    pub reason: String,
}

pub(crate) fn validate_member_phone(phone: &str) -> Result<(), AppError> {
    let normalized: String = phone
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '(' | ')' | '-'))
        .collect();
    if normalized.is_empty() {
        return Err(AppError::bad_request("电话必填"));
    }
    let digits = normalized.strip_prefix('+').unwrap_or(&normalized);
    if phone.len() > 30 || digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return Err(AppError::bad_request("电话号码格式不正确"));
    }
    Ok(())
}

pub(crate) async fn validate_new_account_membership(
    state: &AppState,
    actor: &CurrentUser,
    tx: &mut Transaction<'_, Postgres>,
    input: &NewAccountMembership,
) -> Result<Uuid, AppError> {
    if !state.config.membership_lifecycle_enabled {
        return Err(AppError::forbidden("成员生命周期写入尚未开放"));
    }
    creation_gate(state, input.member_type)?;
    validate_reason(&input.reason)?;
    if input.position.trim().chars().count() > 50 {
        return Err(AppError::bad_request("岗位名称最多 50 字符"));
    }
    if input.member_type == MemberType::Customer && !input.position.trim().is_empty() {
        return Err(AppError::bad_request("客户身份不得绑定员工岗位"));
    }
    Ok(authority_in_tx(tx, actor, input.organization_id)
        .await?
        .tenant_id)
}

#[derive(Clone, sqlx::FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
struct Membership {
    id: Uuid,
    user_id: Uuid,
    organization_id: Uuid,
    member_type: String,
    status: String,
    identity_version: i64,
    department_id: Option<Uuid>,
    position: String,
    updated_at: DateTime<Utc>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CreateMembership {
    #[serde(deserialize_with = "uuid7::deserialize")]
    user_id: Uuid,
    #[serde(deserialize_with = "uuid7::deserialize")]
    organization_id: Uuid,
    member_type: MemberType,
    #[serde(default, deserialize_with = "uuid7::deserialize_optional")]
    department_id: Option<Uuid>,
    #[serde(default)]
    position: String,
    #[serde(default)]
    account_binding_confirmed: bool,
    binding_source: Option<String>,
    reason: String,
    #[serde(skip)]
    expected_phone: Option<String>,
}

pub(crate) async fn bind_account_by_phone(
    state: &AppState,
    actor: &CurrentUser,
    headers: &HeaderMap,
    user_id: Uuid,
    phone: &str,
    input: &NewAccountMembership,
) -> Result<Value, AppError> {
    create(
        state,
        actor,
        headers,
        CreateMembership {
            user_id,
            organization_id: input.organization_id,
            member_type: input.member_type,
            department_id: None,
            position: input.position.clone(),
            account_binding_confirmed: true,
            binding_source: Some("管理员按唯一电话自动匹配已有账号".into()),
            reason: input.reason.clone(),
            expected_phone: Some(
                phone
                    .chars()
                    .filter(|c| !c.is_whitespace() && !matches!(c, '(' | ')' | '-'))
                    .collect(),
            ),
        },
    )
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ExpectedVersion {
    expected_identity_version: i64,
    reason: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct StatusChange {
    expected_identity_version: i64,
    status: MembershipStatus,
    reason: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct IdentityTransition {
    member_type: MemberType,
    expected_identity_version: i64,
    expected_target_identity_version: Option<i64>,
    #[serde(default)]
    enable_target: bool,
    #[serde(default)]
    account_binding_confirmed: bool,
    binding_source: Option<String>,
    reason: String,
}

async fn authorize(state: &AppState, actor: &CurrentUser, writing: bool) -> Result<(), AppError> {
    if writing && !state.config.membership_lifecycle_enabled {
        return Err(AppError::forbidden("成员生命周期写入尚未开放"));
    }
    authorization::require_employee_context(&state.pool, actor).await?;
    state.require(actor, "system:user").await
}

fn creation_gate(state: &AppState, member_type: MemberType) -> Result<(), AppError> {
    if member_type == MemberType::Customer && !state.config.customer_memberships_enabled {
        return Err(AppError::forbidden("客户成员创建尚未开放"));
    }
    Ok(())
}

fn binding_source(
    member_type: MemberType,
    confirmed: bool,
    source: Option<&str>,
) -> Result<Option<String>, AppError> {
    if member_type != MemberType::Customer {
        return Ok(None);
    }
    let source = source.map(str::trim).filter(|value| !value.is_empty());
    if !confirmed || source.is_none() {
        return Err(AppError::bad_request(
            "绑定客户账号须明确确认账号归属并提供绑定来源",
        ));
    }
    let source = source.unwrap_or_default();
    if source.chars().count() > 200 || source.chars().any(char::is_control) {
        return Err(AppError::bad_request("绑定来源须为最多 200 字符的单行文字"));
    }
    Ok(Some(source.to_owned()))
}

fn check_version(row: &Membership, expected: i64) -> Result<(), AppError> {
    if expected < 1 {
        return Err(AppError::bad_request(
            "expectedIdentityVersion 必须大于等于 1",
        ));
    }
    if row.identity_version != expected {
        return Err(AppError::conflict("成员身份已变更，请重新读取后重试"));
    }
    Ok(())
}

fn validate_reason(reason: &str) -> Result<&str, AppError> {
    let reason = reason.trim();
    if reason.is_empty() || reason.chars().count() > 500 || reason.chars().any(char::is_control) {
        return Err(AppError::bad_request(
            "变更原因须为非空、最多 500 字符且不含控制字符的文字",
        ));
    }
    Ok(reason)
}

#[cfg(test)]
mod creation_phone_tests {
    use super::validate_member_phone;

    #[test]
    fn employee_and_customer_phone_is_required() {
        for phone in ["", "  ", " () - ", "+", "not-a-number"] {
            assert!(validate_member_phone(phone).is_err());
        }
        for phone in ["13800000001", "+86 (138) 0000-0001"] {
            assert!(validate_member_phone(phone).is_ok());
        }
    }
}

async fn lock_users(
    tx: &mut Transaction<'_, Postgres>,
    actor: &CurrentUser,
    user_id: Uuid,
    enabled: bool,
) -> Result<(), AppError> {
    let mut ids = vec![actor.user_id, user_id];
    ids.sort_unstable();
    ids.dedup();
    // Parent rows receive no subsequent strong lock: onboarding holds them
    // before existing users. Re-read authorization after waits instead.
    let locked: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id,status FROM users WHERE id=ANY($1) ORDER BY id FOR NO KEY UPDATE",
    )
    .bind(&ids)
    .fetch_all(&mut **tx)
    .await?;
    if !locked.iter().any(|(id, _)| *id == actor.user_id) {
        return Err(AppError::unauthorized("会话用户已失效"));
    }
    match locked
        .iter()
        .find(|(id, _)| *id == user_id)
        .map(|(_, status)| status.as_str())
    {
        None => Err(AppError::not_found("账号不存在")),
        Some(status) if enabled && status != "ENABLED" => {
            Err(AppError::bad_request("仅能为有效账号绑定成员身份"))
        }
        Some(_) => Ok(()),
    }
}

/// A legacy employee-only writer must hold the account lock before calling.
/// It may not create an employee beside a customer while dual creation is off.
pub(crate) async fn ensure_employee_creation_allowed_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    organization_id: Uuid,
    dual_memberships_enabled: bool,
) -> Result<(), AppError> {
    let phone: String = sqlx::query_scalar("SELECT COALESCE(phone,'') FROM users WHERE id=$1")
        .bind(user_id)
        .fetch_one(&mut **tx)
        .await?;
    validate_member_phone(&phone)?;
    if dual_memberships_enabled {
        return Ok(());
    }
    let customer_only: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM memberships WHERE user_id=$1 AND organization_id=$2 \
         AND member_type='CUSTOMER') AND NOT EXISTS(SELECT 1 FROM memberships \
         WHERE user_id=$1 AND organization_id=$2 AND member_type='EMPLOYEE')",
    )
    .bind(user_id)
    .bind(organization_id)
    .fetch_one(&mut **tx)
    .await?;
    if customer_only {
        return Err(AppError::forbidden("同组织第二身份创建尚未开放"));
    }
    Ok(())
}

struct MutationAuthority {
    tenant_id: Uuid,
    fingerprint: Value,
}

/// Recheck using the same transaction so a pool connection cannot wait on an
/// account lock held by this operation. Real time governs every expiry.
async fn authority_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    actor: &CurrentUser,
    organization_id: Uuid,
) -> Result<MutationAuthority, AppError> {
    let identity: Option<(bool, i64, Uuid, i64, DateTime<Utc>)> = sqlx::query_as(
        "SELECT u.is_super_admin,u.session_version,m.id,m.identity_version,s.expires_at \
         FROM users u JOIN platform_sessions s ON s.user_id=u.id \
         JOIN memberships m ON m.id=s.membership_id AND m.user_id=u.id \
           AND m.organization_id=s.organization_id AND m.identity_version=s.identity_version \
         JOIN organizations o ON o.id=s.organization_id AND o.tenant_id=s.tenant_id \
         JOIN tenants t ON t.id=o.tenant_id \
         WHERE u.id=$1 AND s.id=$2 AND s.organization_id=$3 AND s.tenant_id=$4 \
         AND u.status='ENABLED' AND m.member_type='EMPLOYEE' AND m.status='ENABLED' \
         AND public.platform_session_active(s.id) \
         AND t.status='ENABLED' AND (t.expires_at IS NULL OR t.expires_at>clock_timestamp()) \
         AND org_chain_active(o.id) \
         AND (org_chain_expires_at(o.id) IS NULL OR org_chain_expires_at(o.id)>clock_timestamp())",
    )
    .bind(actor.user_id)
    .bind(actor.session_id)
    .bind(actor.org_id)
    .bind(actor.tenant_id)
    .fetch_optional(&mut **tx)
    .await?;
    let identity = identity.ok_or_else(|| AppError::unauthorized("当前会话或员工身份已失效"))?;
    let menu_rows = if identity.0 {
        sqlx::query(authorization::granted_menu_sql())
            .bind(actor.tenant_id)
            .bind(actor.org_id)
            .bind(None::<Uuid>)
            .fetch_all(&mut **tx)
            .await?
    } else {
        sqlx::query(&authorization::effective_menu_sql())
            .bind(actor.org_id)
            .bind(actor.user_id)
            .bind(actor.tenant_id)
            .bind(None::<Uuid>)
            .fetch_all(&mut **tx)
            .await?
    };
    let mut permissions: Vec<String> = menu_rows
        .iter()
        // Both shared MenuRow projections place permission seventh; their
        // COALESCE expression has no stable SQL column alias.
        .map(|row| row.try_get::<String, _>(6))
        .collect::<Result<_, _>>()?;
    permissions.sort_unstable();
    permissions.dedup();
    if !permissions
        .iter()
        .any(|permission| permission == "system:user")
    {
        return Err(AppError::forbidden("无操作权限: system:user"));
    }
    let tenant_id: Option<Uuid> = sqlx::query_scalar(
        "WITH RECURSIVE scoped AS ( \
         SELECT id FROM organizations WHERE id=$2 AND tenant_id=$3 \
         UNION SELECT o.id FROM organizations o JOIN scoped s ON o.parent_id=s.id \
         WHERE o.tenant_id=$3) \
         SELECT o.tenant_id FROM organizations o JOIN tenants t ON t.id=o.tenant_id \
         WHERE o.id=$1 AND t.status='ENABLED' \
         AND (t.expires_at IS NULL OR t.expires_at>clock_timestamp()) AND org_chain_active(o.id) \
         AND (org_chain_expires_at(o.id) IS NULL OR org_chain_expires_at(o.id)>clock_timestamp()) \
         AND (EXISTS(SELECT 1 FROM organizations a JOIN tenants p ON p.id=a.tenant_id \
             WHERE a.id=$2 AND p.id=$3 AND p.is_platform AND a.parent_id IS NULL) \
              OR (o.tenant_id=$3 AND o.id IN (SELECT id FROM scoped)))",
    )
    .bind(organization_id)
    .bind(actor.org_id)
    .bind(actor.tenant_id)
    .fetch_optional(&mut **tx)
    .await?;
    let tenant_id =
        tenant_id.ok_or_else(|| AppError::not_found("组织不存在或不在有效管理范围内"))?;
    let scope: Value = sqlx::query_scalar(
        "WITH RECURSIVE chain AS ( \
         SELECT id,parent_id,tenant_id,status,expires_at,updated_at FROM organizations WHERE id=ANY($1) \
         UNION SELECT o.id,o.parent_id,o.tenant_id,o.status,o.expires_at,o.updated_at \
         FROM organizations o JOIN chain c ON c.parent_id=o.id AND c.tenant_id=o.tenant_id) \
         SELECT jsonb_build_object( \
         'organizations',(SELECT jsonb_agg(to_jsonb(c) ORDER BY c.id) FROM chain c), \
         'tenants',(SELECT jsonb_agg(jsonb_build_object( \
             'id',t.id,'status',t.status,'isPlatform',t.is_platform,'expiresAt',t.expires_at,'updatedAt',t.updated_at) \
             ORDER BY t.id) FROM tenants t WHERE t.id IN (SELECT tenant_id FROM chain)))",
    ).bind(&[organization_id,actor.org_id][..]).fetch_one(&mut **tx).await?;
    Ok(MutationAuthority {
        tenant_id,
        fingerprint: json!({"identity":identity,"permissions":permissions,"scope":scope}),
    })
}

async fn revalidate(
    tx: &mut Transaction<'_, Postgres>,
    actor: &CurrentUser,
    organization_id: Uuid,
    before: &MutationAuthority,
) -> Result<(), AppError> {
    let after = authority_in_tx(tx, actor, organization_id).await?;
    if before.tenant_id != after.tenant_id || before.fingerprint != after.fingerprint {
        return Err(AppError::conflict(
            "管理权限或组织状态已变更，请重新读取后重试",
        ));
    }
    Ok(())
}

fn protect_current_employee(actor: &CurrentUser, row: &Membership) -> Result<(), AppError> {
    if actor.user_id == row.user_id
        && actor.org_id == row.organization_id
        && row.member_type == "EMPLOYEE"
    {
        return Err(AppError::forbidden(
            "不能通过成员管理变更当前登录员工身份，请由其他管理员操作",
        ));
    }
    Ok(())
}

async fn lock_memberships(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    organization_id: Uuid,
) -> Result<Vec<Membership>, AppError> {
    Ok(sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM memberships m WHERE m.user_id=$1 \
         AND m.organization_id=$2 ORDER BY m.id FOR UPDATE OF m"
    ))
    .bind(user_id)
    .bind(organization_id)
    .fetch_all(&mut **tx)
    .await?)
}

async fn locate(state: &AppState, id: Uuid) -> Result<Membership, AppError> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM memberships m WHERE m.id=$1"
    ))
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found("成员身份不存在"))
}

pub(super) async fn list(
    state: &AppState,
    actor: &CurrentUser,
    params: &HashMap<String, String>,
) -> Result<Value, AppError> {
    authorize(state, actor, false).await?;
    let organization_id = uuid7::optional_query(params.get("organizationId"))?;
    let user_id = uuid7::optional_query(params.get("userId"))?;
    let tenant_id = uuid7::optional_query(params.get("tenantId"))?;
    let member_type = params
        .get("memberType")
        .map(|v| MemberType::from_str(v))
        .transpose()
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    let status = params
        .get("status")
        .map(|v| MembershipStatus::from_str(v))
        .transpose()
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    let scope = state.scoped_org_ids(actor).await?;
    if let Some(id) = organization_id {
        state.ensure_org_scope(actor, id, "组织").await?;
    }
    if scope.is_some() && tenant_id.is_some_and(|id| id != actor.tenant_id) {
        return Err(AppError::not_found("租户不存在"));
    }
    let (page, size, offset) = paging(params);
    let predicate = "($1::uuid IS NULL OR m.organization_id=$1) \
         AND ($2::uuid IS NULL OR m.user_id=$2) \
         AND ($3::uuid[] IS NULL OR m.organization_id=ANY($3)) \
         AND ($4::text IS NULL OR m.member_type=$4) \
         AND ($5::text IS NULL OR m.status=$5) \
         AND ($6::uuid IS NULL OR o.tenant_id=$6)";
    let rows: Vec<Membership> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM memberships m JOIN organizations o ON o.id=m.organization_id \
         WHERE {predicate} ORDER BY m.id LIMIT $7 OFFSET $8"
    ))
    .bind(organization_id)
    .bind(user_id)
    .bind(scope.as_deref())
    .bind(member_type.map(MemberType::as_str))
    .bind(status.map(MembershipStatus::as_str))
    .bind(tenant_id)
    .bind(size)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;
    let total: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM memberships m JOIN organizations o ON o.id=m.organization_id WHERE {predicate}"
    ))
    .bind(organization_id).bind(user_id).bind(scope.as_deref())
    .bind(member_type.map(MemberType::as_str)).bind(status.map(MembershipStatus::as_str))
    .bind(tenant_id).fetch_one(&state.pool).await?;
    Ok(page_json(
        rows.iter().map(|row| json!(row)).collect(),
        total,
        page,
        size,
    ))
}

pub(super) async fn detail(
    state: &AppState,
    actor: &CurrentUser,
    id: Uuid,
) -> Result<Value, AppError> {
    authorize(state, actor, false).await?;
    let row = locate(state, id).await?;
    state
        .ensure_org_scope(actor, row.organization_id, "成员身份")
        .await?;
    Ok(json!(row))
}

async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    organization_id: Uuid,
    member_type: MemberType,
    department_id: Option<Uuid>,
    position: &str,
) -> Result<Membership, AppError> {
    let phone: String = sqlx::query_scalar("SELECT COALESCE(phone,'') FROM users WHERE id=$1")
        .bind(user_id)
        .fetch_one(&mut **tx)
        .await?;
    validate_member_phone(&phone)?;
    let id = uuid7::new_uuid();
    sqlx::query(
        "INSERT INTO memberships (id,user_id,organization_id,member_type,status,identity_version,department_id,position) \
         VALUES ($1,$2,$3,$4,'ENABLED',1,$5,$6)",
    )
    .bind(id).bind(user_id).bind(organization_id).bind(member_type.as_str())
    .bind(department_id).bind(position).execute(&mut **tx).await?;
    fetch_in_tx(tx, id).await
}

async fn fetch_in_tx(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<Membership, AppError> {
    Ok(sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM memberships m WHERE m.id=$1"
    ))
    .bind(id)
    .fetch_one(&mut **tx)
    .await?)
}

pub(super) async fn create(
    state: &AppState,
    actor: &CurrentUser,
    headers: &HeaderMap,
    input: CreateMembership,
) -> Result<Value, AppError> {
    authorize(state, actor, true).await?;
    let reason = validate_reason(&input.reason)?;
    creation_gate(state, input.member_type)?;
    let binding = binding_source(
        input.member_type,
        input.account_binding_confirmed,
        input.binding_source.as_deref(),
    )?;
    let position = input.position.trim();
    if position.chars().count() > 50 {
        return Err(AppError::bad_request("岗位名称最多 50 字符"));
    }
    if input.member_type == MemberType::Customer
        && (input.department_id.is_some() || !position.is_empty())
    {
        return Err(AppError::bad_request("客户身份不得绑定员工部门或岗位"));
    }
    let mut tx = state.pool.begin().await?;
    let authority = authority_in_tx(&mut tx, actor, input.organization_id).await?;
    lock_users(&mut tx, actor, input.user_id, true).await?;
    let phone: String = sqlx::query_scalar("SELECT COALESCE(phone,'') FROM users WHERE id=$1")
        .bind(input.user_id)
        .fetch_one(&mut *tx)
        .await?;
    validate_member_phone(&phone)?;
    if let Some(expected) = &input.expected_phone {
        let current: String = phone
            .chars()
            .filter(|c| !c.is_whitespace() && !matches!(c, '(' | ')' | '-'))
            .collect();
        if &current != expected {
            return Err(AppError::conflict("账号电话已变更，请重新提交"));
        }
    }
    revalidate(&mut tx, actor, input.organization_id, &authority).await?;
    let tenant_id = authority.tenant_id;
    let members = lock_memberships(&mut tx, input.user_id, input.organization_id).await?;
    revalidate(&mut tx, actor, input.organization_id, &authority).await?;
    if members
        .iter()
        .any(|m| m.member_type == input.member_type.as_str())
    {
        return Err(AppError::conflict("该账号在组织内已有同类型身份"));
    }
    if !members.is_empty() && !state.config.dual_memberships_enabled {
        return Err(AppError::forbidden("同组织第二身份创建尚未开放"));
    }
    if let Some(department_id) = input.department_id {
        let valid: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM departments WHERE id=$1 AND organization_id=$2")
                .bind(department_id)
                .bind(input.organization_id)
                .fetch_optional(&mut *tx)
                .await?;
        if valid.is_none() {
            return Err(AppError::bad_request("部门必须属于成员组织"));
        }
    }
    let row = insert(
        &mut tx,
        input.user_id,
        input.organization_id,
        input.member_type,
        input.department_id,
        position,
    )
    .await?;
    audit_change(
        &mut tx,
        actor,
        headers,
        tenant_id,
        &row,
        "MEMBERSHIP_CREATE",
        reason,
        json!(null),
        json!({"membership":row,"bindingSource":binding}),
    )
    .await?;
    revalidate(&mut tx, actor, input.organization_id, &authority).await?;
    tx.commit().await?;
    Ok(json!(row))
}

/// Revoke by the immutable relation ID, including customer generations.
async fn revoke_membership_credentials(
    tx: &mut Transaction<'_, Postgres>,
    row: &Membership,
) -> Result<(), AppError> {
    sqlx::query("SELECT public.revoke_membership_credentials($1)")
        .bind(row.id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn update_status(
    tx: &mut Transaction<'_, Postgres>,
    row: &Membership,
    status: MembershipStatus,
) -> Result<Membership, AppError> {
    let next = row
        .identity_version
        .checked_add(1)
        .ok_or_else(|| AppError::conflict("身份版本已达到上限"))?;
    sqlx::query(
        "UPDATE memberships SET status=$2,identity_version=$3,updated_at=now() WHERE id=$1",
    )
    .bind(row.id)
    .bind(status.as_str())
    .bind(next)
    .execute(&mut **tx)
    .await?;
    revoke_membership_credentials(tx, row).await?;
    fetch_in_tx(tx, row.id).await
}

pub(super) async fn change_status(
    state: &AppState,
    actor: &CurrentUser,
    headers: &HeaderMap,
    id: Uuid,
    input: StatusChange,
) -> Result<Value, AppError> {
    authorize(state, actor, true).await?;
    let reason = validate_reason(&input.reason)?;
    let located = locate(state, id).await?;
    protect_current_employee(actor, &located)?;
    let mut tx = state.pool.begin().await?;
    let authority = authority_in_tx(&mut tx, actor, located.organization_id).await?;
    lock_users(
        &mut tx,
        actor,
        located.user_id,
        input.status == MembershipStatus::Enabled,
    )
    .await?;
    revalidate(&mut tx, actor, located.organization_id, &authority).await?;
    let tenant_id = authority.tenant_id;
    let rows = lock_memberships(&mut tx, located.user_id, located.organization_id).await?;
    revalidate(&mut tx, actor, located.organization_id, &authority).await?;
    let before = rows
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| AppError::not_found("成员身份不存在"))?;
    check_version(before, input.expected_identity_version)?;
    let after = update_status(&mut tx, before, input.status).await?;
    audit_change(
        &mut tx,
        actor,
        headers,
        tenant_id,
        &after,
        "MEMBERSHIP_STATUS",
        reason,
        json!(before),
        json!(after),
    )
    .await?;
    revalidate(&mut tx, actor, located.organization_id, &authority).await?;
    tx.commit().await?;
    Ok(json!(after))
}

pub(super) async fn delete(
    state: &AppState,
    actor: &CurrentUser,
    headers: &HeaderMap,
    id: Uuid,
    input: ExpectedVersion,
) -> Result<Value, AppError> {
    authorize(state, actor, true).await?;
    let reason = validate_reason(&input.reason)?;
    let located = locate(state, id).await?;
    protect_current_employee(actor, &located)?;
    let mut tx = state.pool.begin().await?;
    let authority = authority_in_tx(&mut tx, actor, located.organization_id).await?;
    lock_users(&mut tx, actor, located.user_id, false).await?;
    revalidate(&mut tx, actor, located.organization_id, &authority).await?;
    let tenant_id = authority.tenant_id;
    let rows = lock_memberships(&mut tx, located.user_id, located.organization_id).await?;
    revalidate(&mut tx, actor, located.organization_id, &authority).await?;
    let before = rows
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| AppError::not_found("成员身份不存在"))?;
    check_version(before, input.expected_identity_version)?;
    // Preserve the final version in the audit even though the member is removed.
    let removed = update_status(&mut tx, before, MembershipStatus::Disabled).await?;
    sqlx::query("DELETE FROM memberships WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    cleanup_departed_employee_roles(&mut tx, before).await?;
    audit_change(
        &mut tx,
        actor,
        headers,
        tenant_id,
        &removed,
        "MEMBERSHIP_DELETE",
        reason,
        json!(before),
        json!({"removed":true,"identityVersion":removed.identity_version}),
    )
    .await?;
    revalidate(&mut tx, actor, located.organization_id, &authority).await?;
    tx.commit().await?;
    Ok(json!({"id":id,"deleted":true,"identityVersion":removed.identity_version}))
}

pub(super) async fn transition(
    state: &AppState,
    actor: &CurrentUser,
    headers: &HeaderMap,
    id: Uuid,
    input: IdentityTransition,
    replacing: bool,
) -> Result<Value, AppError> {
    authorize(state, actor, true).await?;
    let reason = validate_reason(&input.reason)?;
    creation_gate(state, input.member_type)?;
    if !state.config.dual_memberships_enabled {
        return Err(AppError::forbidden("同组织第二身份创建尚未开放"));
    }
    let binding = binding_source(
        input.member_type,
        input.account_binding_confirmed,
        input.binding_source.as_deref(),
    )?;
    let located = locate(state, id).await?;
    if replacing {
        protect_current_employee(actor, &located)?;
    }
    let mut tx = state.pool.begin().await?;
    let authority = authority_in_tx(&mut tx, actor, located.organization_id).await?;
    lock_users(&mut tx, actor, located.user_id, true).await?;
    revalidate(&mut tx, actor, located.organization_id, &authority).await?;
    let tenant_id = authority.tenant_id;
    let rows = lock_memberships(&mut tx, located.user_id, located.organization_id).await?;
    revalidate(&mut tx, actor, located.organization_id, &authority).await?;
    let source = rows
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| AppError::not_found("源成员身份不存在"))?;
    check_version(source, input.expected_identity_version)?;
    if source.member_type == input.member_type.as_str() {
        return Err(AppError::bad_request(
            "目标类型必须与源成员身份不同，不可原地修改类型",
        ));
    }
    let target_before = rows
        .iter()
        .find(|m| m.member_type == input.member_type.as_str());
    let target = if let Some(existing) = target_before {
        let expected = input.expected_target_identity_version.ok_or_else(|| {
            AppError::bad_request("目标已存在，须提供 expectedTargetIdentityVersion")
        })?;
        check_version(existing, expected)?;
        if existing.status == "DISABLED" {
            if !input.enable_target {
                return Err(AppError::conflict(
                    "目标身份已停用，须明确 enableTarget 才能启用",
                ));
            }
            // A lifecycle conversion does not restore a departed employee's
            // stored role configuration. Other supported organization roles
            // remain untouched by the precise cleanup below.
            cleanup_departed_employee_roles(&mut tx, existing).await?;
            clear_membership_grants(&mut tx, existing.id).await?;
            update_status(&mut tx, existing, MembershipStatus::Enabled).await?
        } else {
            existing.clone()
        }
    } else {
        if input.expected_target_identity_version.is_some() {
            return Err(AppError::conflict("预期目标身份已经不存在"));
        }
        insert(
            &mut tx,
            source.user_id,
            source.organization_id,
            input.member_type,
            None,
            "",
        )
        .await?
    };
    let source_after = if replacing {
        let disabled = update_status(&mut tx, source, MembershipStatus::Disabled).await?;
        cleanup_departed_employee_roles(&mut tx, source).await?;
        clear_membership_grants(&mut tx, source.id).await?;
        disabled
    } else {
        source.clone()
    };
    let action = if replacing {
        "MEMBERSHIP_REPLACE_IDENTITY"
    } else {
        "MEMBERSHIP_SECOND_IDENTITY"
    };
    audit_change(
        &mut tx,
        actor,
        headers,
        tenant_id,
        &target,
        action,
        reason,
        json!({"source":source,"target":target_before}),
        json!({"source":source_after,"target":target,"bindingSource":binding}),
    )
    .await?;
    revalidate(&mut tx, actor, located.organization_id, &authority).await?;
    tx.commit().await?;
    let mut result = json!(target);
    result["sourceMembershipId"] = json!(source.id);
    Ok(result)
}

async fn clear_membership_grants(
    tx: &mut Transaction<'_, Postgres>,
    membership_id: Uuid,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM membership_data_grants WHERE membership_id=$1")
        .bind(membership_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// A role attached to an ancestor continues to be supported by another active
/// employee under that ancestor. Customers never provide that support.
async fn cleanup_departed_employee_roles(
    tx: &mut Transaction<'_, Postgres>,
    row: &Membership,
) -> Result<(), AppError> {
    if row.member_type != "EMPLOYEE" {
        return Ok(());
    }
    cleanup_employee_roles_after_departure_in_tx(tx, row.user_id, &[row.organization_id]).await
}

/// The caller holds the account lock and has completed all employee changes.
/// Evaluate the final active support set once, so bulk moves retain an ancestor
/// role supported by a newly retained or added employee in another organization.
pub(crate) async fn cleanup_employee_roles_after_departure_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    departed_orgs: &[Uuid],
) -> Result<(), AppError> {
    if departed_orgs.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "WITH RECURSIVE supported AS ( \
         SELECT o.id,o.parent_id,o.tenant_id FROM memberships m \
         JOIN organizations o ON o.id=m.organization_id \
         JOIN users u ON u.id=m.user_id JOIN tenants t ON t.id=o.tenant_id \
         WHERE m.user_id=$1 AND m.member_type='EMPLOYEE' \
         AND m.status='ENABLED' AND u.status='ENABLED' AND t.status='ENABLED' \
         AND (t.expires_at IS NULL OR t.expires_at>clock_timestamp()) AND org_chain_active(o.id) \
         AND (org_chain_expires_at(o.id) IS NULL OR org_chain_expires_at(o.id)>clock_timestamp()) \
         UNION SELECT p.id,p.parent_id,p.tenant_id FROM organizations p \
         JOIN supported s ON s.parent_id=p.id AND s.tenant_id=p.tenant_id), \
         departed AS ( \
         SELECT id,parent_id,tenant_id FROM organizations WHERE id=ANY($2) \
         UNION SELECT p.id,p.parent_id,p.tenant_id FROM organizations p \
         JOIN departed d ON d.parent_id=p.id AND d.tenant_id=p.tenant_id) \
         DELETE FROM user_roles ur WHERE ur.user_id=$1 \
         AND ur.organization_id IN (SELECT id FROM departed) \
         AND ur.organization_id NOT IN (SELECT id FROM supported)",
    )
    .bind(user_id)
    .bind(departed_orgs)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn audit_change(
    tx: &mut Transaction<'_, Postgres>,
    actor: &CurrentUser,
    headers: &HeaderMap,
    tenant_id: Uuid,
    membership: &Membership,
    action: &'static str,
    reason: &str,
    before: Value,
    after: Value,
) -> Result<(), AppError> {
    let mut revoked_membership_ids = Vec::new();
    if matches!(action, "MEMBERSHIP_STATUS" | "MEMBERSHIP_DELETE") {
        revoked_membership_ids.push(membership.id);
    }
    if action == "MEMBERSHIP_REPLACE_IDENTITY" {
        revoked_membership_ids.extend(
            before["source"]["id"]
                .as_str()
                .and_then(|id| uuid7::parse_uuid(id).ok()),
        );
    }
    if matches!(
        action,
        "MEMBERSHIP_REPLACE_IDENTITY" | "MEMBERSHIP_SECOND_IDENTITY"
    ) && before["target"]["status"] == "DISABLED"
    {
        revoked_membership_ids.push(membership.id);
    }
    let detail = json!({
        "objectType":"membership", "objectId":membership.id, "before":before, "after":after,
        "actorContext":{
            "tenantId":actor.tenant_id,"organizationId":actor.org_id,"sessionId":actor.session_id,
        },
        "reason":reason,
        "credentialRevocation":{
            "mode":"MEMBERSHIP_GENERATION", "membershipIds":revoked_membership_ids,
        },
    });
    let detail = detail.to_string();
    let ip = audit::client_ip(headers);
    let user_agent = audit::ua_of(headers);
    audit::write_in_tx(
        tx,
        audit::AuditInfo {
            user_id: Some(actor.user_id),
            username: &actor.username,
            tenant_id: Some(tenant_id),
            organization_id: Some(membership.organization_id),
            action,
            detail: &detail,
            ip: &ip,
            user_agent: &user_agent,
        },
    )
    .await
}
