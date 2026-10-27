use crate::util::uuid7::ParseUuid7;
use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::authorization;
use crate::error::AppError;
use crate::extractors::{ApiJson, CurrentUser, RequirePermission};
use crate::notifications::system_in_tx;
use crate::routes::{page_json, paging};
use crate::state::AppState;
use crate::util::audit;

pub(super) mod remote;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/users/account-operations/{id}",
            get(remote::status).post(remote::recover),
        )
        .route("/api/users", get(list).post(create))
        .route("/api/users/{id}", put(update).delete(delete))
        .route("/api/users/{id}/status", put(update_status))
        .route("/api/users/batch-status", post(batch_status))
        .route("/api/users/{id}/reset-password", post(reset_password))
        .route(
            "/api/users/{id}/memberships",
            get(get_memberships).put(put_memberships),
        )
        .route("/api/users/{id}/roles", get(get_roles).put(put_roles))
}

#[derive(sqlx::FromRow, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UserListRow {
    id: Uuid,
    username: String,
    real_name: String,
    email: String,
    phone: String,
    gender: String,
    status: String,
    last_login_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    role_count: i64,
}

type UserMembershipRow = (
    Uuid,
    Uuid,
    String,
    Option<Uuid>,
    Option<String>,
    Uuid,
    String,
    String,
    i64,
);

/// Correlated proof for a role's ancestor scope. Arguments are fixed SQL
/// expressions supplied by these internal adapters, never request values.
/// The supporting employee must itself be inside the administrator's scope.
pub(super) fn role_employee_support_sql(user: &str, role_org: &str, scope: &str) -> String {
    format!(
        "EXISTS (WITH RECURSIVE employee_ancestors AS ( \
         SELECT o.id,o.parent_id,o.tenant_id FROM memberships em \
         JOIN users eu ON eu.id=em.user_id JOIN organizations o ON o.id=em.organization_id \
         JOIN tenants et ON et.id=o.tenant_id \
         WHERE em.user_id={user} AND em.member_type='EMPLOYEE' AND em.status='ENABLED' \
         AND eu.status='ENABLED' AND et.status='ENABLED' \
         AND (et.expires_at IS NULL OR et.expires_at>clock_timestamp()) \
         AND org_chain_active(o.id) \
         AND (org_chain_expires_at(o.id) IS NULL OR org_chain_expires_at(o.id)>clock_timestamp()) \
         AND ({scope}::uuid[] IS NULL OR em.organization_id=ANY({scope})) \
         UNION SELECT p.id,p.parent_id,p.tenant_id FROM organizations p \
         JOIN employee_ancestors a ON a.parent_id=p.id AND a.tenant_id=p.tenant_id) \
         SELECT 1 FROM employee_ancestors WHERE id={role_org})"
    )
}

pub(super) async fn require_employee_permission_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    actor: &CurrentUser,
    permission: &str,
) -> Result<(), AppError> {
    let menus = authorization::context_menu_rows_in_tx(tx, actor, None).await?;
    if !authorization::permissions_of(&menus).contains(permission) {
        return Err(AppError::forbidden(format!("无操作权限: {permission}")));
    }
    Ok(())
}

/// Rebuild the managed subtree after lock waits and before commit. A stale
/// pre-transaction array cannot authorize an organization moved out of scope.
pub(super) async fn require_scope_unchanged_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    actor: &CurrentUser,
    expected: Option<&[Uuid]>,
) -> Result<(), AppError> {
    authorization::require_employee_context_in_tx(tx, actor).await?;
    let platform: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM tenants t JOIN organizations o ON o.tenant_id=t.id \
         WHERE t.id=$1 AND o.id=$2 AND t.is_platform AND o.parent_id IS NULL)",
    )
    .bind(actor.tenant_id)
    .bind(actor.org_id)
    .fetch_one(&mut **tx)
    .await?;
    if platform && expected.is_none() {
        return Ok(());
    }
    if platform || expected.is_none() {
        return Err(AppError::conflict("组织管理范围已变更，请重新读取后重试"));
    }
    let mut current: Vec<Uuid> = sqlx::query_scalar(
        "WITH RECURSIVE scoped AS (SELECT id FROM organizations WHERE id=$1 AND tenant_id=$2 \
         UNION SELECT o.id FROM organizations o JOIN scoped s ON o.parent_id=s.id \
         WHERE o.tenant_id=$2) SELECT id FROM scoped ORDER BY id",
    )
    .bind(actor.org_id)
    .bind(actor.tenant_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut expected = expected.unwrap_or_default().to_vec();
    current.sort_unstable();
    expected.sort_unstable();
    if current != expected {
        return Err(AppError::conflict("组织管理范围已变更，请重新读取后重试"));
    }
    Ok(())
}

/// 非平台根组织仅管理当前组织及其下级组织的成员。
async fn ensure_user_in_scope(
    state: &AppState,
    user: &CurrentUser,
    target: Uuid,
) -> Result<(), AppError> {
    if let Some(org_ids) = state.scoped_org_ids(user).await? {
        let belongs: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM memberships WHERE user_id = $1 AND organization_id = ANY($2))",
        )
        .bind(target)
        .bind(&org_ids)
        .fetch_one(&state.pool)
        .await?;
        if !belongs {
            return Err(AppError::not_found("用户不存在"));
        }
    }
    Ok(())
}

/// 锁住全局用户行并在同一事务内校验成员归属，避免成员关系并发变更与账号修改交错。
async fn lock_global_user_mutation(
    state: &AppState,
    user: &CurrentUser,
    target: Uuid,
    tx: &mut Transaction<'_, Postgres>,
) -> Result<Vec<Uuid>, AppError> {
    let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE id = $1 FOR UPDATE")
        .bind(target)
        .fetch_optional(&mut **tx)
        .await?;
    if exists.is_none() {
        return Err(AppError::not_found("用户不存在"));
    }
    let tenants: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT o.tenant_id FROM memberships m \
         JOIN organizations o ON o.id = m.organization_id WHERE m.user_id = $1",
    )
    .bind(target)
    .fetch_all(&mut **tx)
    .await?;
    if let Some(org_ids) = state.scoped_org_ids(user).await? {
        let memberships: Vec<Uuid> =
            sqlx::query_scalar("SELECT organization_id FROM memberships WHERE user_id = $1")
                .bind(target)
                .fetch_all(&mut **tx)
                .await?;
        if !memberships.iter().any(|org| org_ids.contains(org)) {
            return Err(AppError::not_found("用户不存在"));
        }
        if memberships.iter().any(|org| !org_ids.contains(org)) {
            return Err(AppError::forbidden(
                "此账号还有管理范围外的成员关系，全局资料仅平台可修改",
            ));
        }
        let outside_roles: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM user_roles ur JOIN roles r ON r.id = ur.role_id \
             WHERE ur.user_id = $1 AND (NOT (r.organization_id = ANY($2)) OR NOT (ur.organization_id = ANY($2))))",
        )
        .bind(target)
        .bind(&org_ids)
        .fetch_one(&mut **tx)
        .await?;
        if outside_roles {
            return Err(AppError::forbidden(
                "此账号还有管理范围外的角色，全局资料仅平台可修改",
            ));
        }
    }
    Ok(tenants)
}

async fn user_tenant_ids_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    target: Uuid,
) -> Result<Vec<Uuid>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT DISTINCT o.tenant_id FROM memberships m \
         JOIN organizations o ON o.id = m.organization_id WHERE m.user_id = $1",
    )
    .bind(target)
    .fetch_all(&mut **tx)
    .await?)
}

async fn user_role_tenant_ids_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    target: Uuid,
) -> Result<Vec<Uuid>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT DISTINCT r.tenant_id FROM user_roles ur \
         JOIN roles r ON r.id = ur.role_id WHERE ur.user_id = $1",
    )
    .bind(target)
    .fetch_all(&mut **tx)
    .await?)
}

/// 全局账号事件只投递到明确受影响租户中的有效直接成员上下文。
/// 没有有效成员关系时返回空集合，保留业务操作与审计而不猜测接收组织。
async fn system_recipient_contexts(
    tx: &mut Transaction<'_, Postgres>,
    target: Uuid,
    affected_tenants: &[Uuid],
) -> Result<Vec<(Uuid, Uuid, Uuid)>, AppError> {
    Ok(sqlx::query_as(
        "SELECT m.user_id, o.tenant_id, o.id \
         FROM memberships m \
         JOIN users u ON u.id = m.user_id \
         JOIN organizations o ON o.id = m.organization_id \
         JOIN tenants t ON t.id = o.tenant_id \
         WHERE m.user_id = $1 AND o.tenant_id = ANY($2) \
           AND u.status = 'ENABLED' AND t.status = 'ENABLED' \
           AND (t.expires_at IS NULL OR t.expires_at > now()) \
           AND org_chain_active(o.id) \
         ORDER BY o.tenant_id, o.id",
    )
    .bind(target)
    .bind(affected_tenants)
    .fetch_all(&mut **tx)
    .await?)
}

fn merge_tenant_ids(mut before: Vec<Uuid>, after: Vec<Uuid>) -> Vec<Uuid> {
    before.extend(after);
    before.sort_unstable();
    before.dedup();
    before
}

/// Read identity summaries only after the caller has locked the target user.
/// Contact values and credentials are excluded from the durable audit payload.
async fn user_summary_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    target: Uuid,
) -> Result<Value, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT jsonb_build_object('username', username, 'realName', real_name, \
         'status', status, 'gender', gender, 'emailSet', COALESCE(email,'')<>'', \
         'phoneSet', COALESCE(phone,'')<>'', 'sessionVersion', session_version) \
         FROM users WHERE id=$1",
    )
    .bind(target)
    .fetch_one(&mut **tx)
    .await?)
}

async fn memberships_summary_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    target: Uuid,
    scoped_orgs: Option<&[Uuid]>,
) -> Result<Value, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT COALESCE(jsonb_agg(jsonb_build_object( \
         'organizationId', m.organization_id, 'organizationName', o.name, \
         'membershipId', m.id, 'memberType', m.member_type, 'status', m.status, \
         'identityVersion', m.identity_version, 'departmentId', m.department_id, 'positionSet', m.position<>'') \
         ORDER BY m.organization_id), '[]'::jsonb) \
         FROM memberships m JOIN organizations o ON o.id=m.organization_id \
         WHERE m.user_id=$1 AND m.member_type='EMPLOYEE' \
         AND ($2::uuid[] IS NULL OR m.organization_id=ANY($2))",
    )
    .bind(target)
    .bind(scoped_orgs)
    .fetch_one(&mut **tx)
    .await?)
}

async fn roles_summary_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    target: Uuid,
    scoped_orgs: Option<&[Uuid]>,
) -> Result<Value, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT COALESCE(jsonb_agg(jsonb_build_object('roleId', ur.role_id, \
         'roleName', r.name, 'organizationId', ur.organization_id) \
         ORDER BY ur.role_id, ur.organization_id), '[]'::jsonb) \
         FROM user_roles ur JOIN roles r ON r.id=ur.role_id \
         WHERE ur.user_id=$1 AND ($2::uuid[] IS NULL OR \
         (r.organization_id=ANY($2) AND ur.organization_id=ANY($2)))",
    )
    .bind(target)
    .bind(scoped_orgs)
    .fetch_one(&mut **tx)
    .await?)
}

async fn audit_user_change_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    headers: &HeaderMap,
    actor: &CurrentUser,
    target_tenants: &[Uuid],
    action: &'static str,
    detail: &str,
) -> Result<(), AppError> {
    let ip = audit::client_ip(headers);
    let ua = audit::ua_of(headers);
    if target_tenants.is_empty() {
        audit::write_in_tx(
            tx,
            audit::AuditInfo {
                user_id: Some(actor.user_id),
                username: &actor.username,
                tenant_id: None,
                organization_id: None,
                action,
                detail,
                ip: &ip,
                user_agent: &ua,
            },
        )
        .await?;
    } else {
        for tenant_id in target_tenants {
            audit::write_in_tx(
                tx,
                audit::AuditInfo {
                    user_id: Some(actor.user_id),
                    username: &actor.username,
                    tenant_id: Some(*tenant_id),
                    organization_id: None,
                    action,
                    detail,
                    ip: &ip,
                    user_agent: &ua,
                },
            )
            .await?;
        }
    }
    Ok(())
}

/// GET /api/users — 分页 + keyword/organizationId/tenantId 过滤
/// 除平台根组织外，仅返回当前组织及下级组织的用户与成员关系。
async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user")
        .check(&state, &user)
        .await?;
    let (page, size, offset) = paging(&params);
    let kw = params.get("keyword").cloned().unwrap_or_default();
    let member_type = match params.get("memberType").map(String::as_str) {
        None => None,
        Some("EMPLOYEE") => Some("EMPLOYEE"),
        Some("CUSTOMER") => Some("CUSTOMER"),
        _ => return Err(AppError::bad_request("成员类型仅支持 EMPLOYEE / CUSTOMER")),
    };
    let org_filter = crate::util::uuid7::optional_query(params.get("organizationId"))?;
    let scoped_orgs = state.scoped_org_ids(&user).await?;
    if let Some(org) = org_filter {
        state.ensure_org_scope(&user, org, "组织").await?;
    }
    let requested_tenant = crate::util::uuid7::optional_query(params.get("tenantId"))?;
    let tenant_filter: Option<Uuid> = if scoped_orgs.is_none() {
        requested_tenant
    } else {
        Some(user.tenant_id)
    };

    let where_clause = "NOT EXISTS(SELECT 1 FROM identity_account_tombstones tomb WHERE tomb.user_id=u.id) AND ($1 = '' OR u.username ILIKE '%' || $1 || '%' OR u.real_name ILIKE '%' || $1 || '%' \
     OR u.email ILIKE '%' || $1 || '%' OR u.phone ILIKE '%' || $1 || '%') \
     AND ($2::uuid IS NULL OR u.id IN (SELECT user_id FROM memberships WHERE organization_id = $2)) \
     AND ($3::uuid IS NULL OR u.id IN (SELECT m.user_id FROM memberships m JOIN organizations o ON o.id = m.organization_id WHERE o.tenant_id = $3)) \
     AND ($4::uuid[] IS NULL OR EXISTS(SELECT 1 FROM memberships m WHERE m.user_id = u.id AND m.organization_id = ANY($4))) \
     AND ($5::text IS NULL OR EXISTS(SELECT 1 FROM memberships typed_m JOIN organizations typed_o ON typed_o.id=typed_m.organization_id \
       WHERE typed_m.user_id=u.id AND typed_m.member_type=$5 \
       AND ($2::uuid IS NULL OR typed_m.organization_id=$2) \
       AND ($3::uuid IS NULL OR typed_o.tenant_id=$3) \
       AND ($4::uuid[] IS NULL OR typed_m.organization_id=ANY($4))))";

    let employee_support = role_employee_support_sql(
        "ur.user_id",
        "ur.organization_id",
        "(CASE WHEN $2::uuid IS NULL THEN $4::uuid[] ELSE ARRAY[$2::uuid] END)",
    );
    let rows: Vec<UserListRow> = sqlx::query_as(&format!(
        "SELECT u.id, u.username, COALESCE(u.real_name,'') AS real_name, COALESCE(u.email,'') AS email, \
         COALESCE(u.phone,'') AS phone, COALESCE(u.gender,'UNKNOWN') AS gender, u.status, u.last_login_at, u.created_at, \
         (SELECT count(*) FROM user_roles ur JOIN roles r ON r.id = ur.role_id \
          WHERE ur.user_id = u.id AND ur.organization_id=r.organization_id AND {employee_support} \
          AND ($3::uuid IS NULL OR r.tenant_id = $3) \
          AND ($4::uuid[] IS NULL OR (r.organization_id = ANY($4) AND ur.organization_id = ANY($4)))) AS role_count \
         FROM users u WHERE {where_clause} ORDER BY u.created_at, u.id LIMIT $6 OFFSET $7"
    ))
    .bind(&kw)
    .bind(org_filter)
    .bind(tenant_filter)
    .bind(&scoped_orgs)
    .bind(member_type)
    .bind(size)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;
    let total: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM users u WHERE {where_clause}"
    ))
    .bind(&kw)
    .bind(org_filter)
    .bind(tenant_filter)
    .bind(&scoped_orgs)
    .bind(member_type)
    .fetch_one(&state.pool)
    .await?;

    let user_ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let mut memberships_map: HashMap<Uuid, Vec<Value>> = HashMap::new();
    if !user_ids.is_empty() {
        let mrows: Vec<UserMembershipRow> = sqlx::query_as(
            "SELECT m.user_id, m.organization_id, o.name, m.department_id, d.name, \
             m.id, m.member_type, m.status, m.identity_version \
             FROM memberships m JOIN organizations o ON o.id = m.organization_id \
             LEFT JOIN departments d ON d.id = m.department_id \
             WHERE m.user_id = ANY($1) AND ($2::uuid IS NULL OR o.tenant_id = $2) \
             AND ($3::uuid[] IS NULL OR m.organization_id = ANY($3)) \
             AND ($4::text IS NULL OR m.member_type=$4)",
        )
        .bind(&user_ids)
        .bind(tenant_filter)
        .bind(&scoped_orgs)
        .bind(member_type)
        .fetch_all(&state.pool)
        .await?;
        for (
            uid,
            org_id,
            org_name,
            dept_id,
            dept_name,
            membership_id,
            member_type,
            status,
            identity_version,
        ) in mrows
        {
            memberships_map.entry(uid).or_default().push(json!({
                "organizationId": org_id, "organizationName": org_name,
                "departmentId": dept_id, "departmentName": dept_name,
                "membershipId": membership_id, "memberType": member_type,
                "status": status, "identityVersion": identity_version,
            }));
        }
    }
    let records: Vec<Value> = rows
        .iter()
        .map(|r| {
            let mut v = serde_json::to_value(r).unwrap_or_default();
            v["memberships"] = json!(memberships_map.get(&r.id).cloned().unwrap_or_default());
            v
        })
        .collect();
    Ok(Json(page_json(records, total, page, size)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserBody {
    username: String,
    password: String,
    #[serde(default)]
    real_name: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    phone: String,
    #[serde(default = "default_gender")]
    gender: String,
}

fn default_gender() -> String {
    "UNKNOWN".into()
}

fn parse_gender(g: &str) -> Result<String, AppError> {
    match g {
        "MALE" | "FEMALE" | "UNKNOWN" => Ok(g.to_string()),
        _ => Err(AppError::bad_request("性别仅支持 MALE / FEMALE / UNKNOWN")),
    }
}

/// POST /api/users — 租户管理员创建的账号直接加入当前组织
async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<UserBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user")
        .check(&state, &user)
        .await?;
    let username = body.username.trim().to_string();
    if username.is_empty() {
        return Err(AppError::bad_request("用户名不能为空"));
    }
    state.accounts.validate_password(&body.password)?;
    let gender = parse_gender(&body.gender)?;
    if state.remote_accounts.is_some() {
        return remote::create(&state, &headers, &user, &username, &body, &gender)
            .await
            .map(Json);
    }
    let password = state.accounts.prepare_password(&body.password).await?;
    let platform = state.scoped_org_ids(&user).await?.is_none();
    let welcome_title = if platform {
        None
    } else {
        let branding = crate::platform_settings::load_branding(&state.pool).await?;
        Some(format!("欢迎加入 {}", branding.platform_name))
    };
    let mut tx = state.pool.begin().await?;
    let created = {
        let mut accounts = crate::authentication::PostgresAccountTransaction::new(&mut tx);
        state
            .accounts
            .create(
                &mut accounts,
                crate::platform_accounts::NewAccount {
                    id: crate::util::uuid7::new_uuid(),
                    username: &username,
                    real_name: &body.real_name,
                    email: &body.email,
                    phone: &body.phone,
                    gender: &gender,
                },
                &password,
            )
            .await?
    };
    let row = (created.id, created.username);
    if let Some(welcome_title) = welcome_title {
        sqlx::query("INSERT INTO memberships (user_id, organization_id, member_type, id) VALUES ($1, $2, 'EMPLOYEE', $3)")
            .bind(row.0)
            .bind(user.org_id)
            .bind(crate::util::uuid7::new_uuid())
            .execute(&mut *tx)
            .await?;
        system_in_tx(
            &mut tx,
            &[(row.0, user.tenant_id, user.org_id)],
            "INFO",
            &welcome_title,
            &format!(
                "管理员为您创建了账号「{}」。首次登录后建议前往个人中心修改初始密码。",
                username
            ),
            "/profile",
        )
        .await?;
    }
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let after = user_summary_in_tx(&mut tx, row.0).await?;
    audit::write_in_tx(
        &mut tx,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(user.tenant_id),
            organization_id: Some(user.org_id),
            action: "CREATE_USER",
            detail: &audit::change_detail("user", row.0, Value::Null, after),
            ip: &ip,
            user_agent: &ua,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"id": row.0, "username": row.1})))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserUpdate {
    #[serde(default)]
    real_name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    phone: Option<String>,
    #[serde(default)]
    gender: Option<String>,
}

/// PUT /api/users/{id}
async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UserUpdate>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user")
        .check(&state, &user)
        .await?;
    let id: Uuid = id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    if let Some(g) = &body.gender {
        parse_gender(g)?;
    }
    if state.remote_accounts.is_some() {
        return remote::mutate(&state,&headers,&user,id,"UPDATE_PROFILE",json!({"realName":body.real_name,"email":body.email,"phone":body.phone,"gender":body.gender}),None).await.map(Json);
    }
    let mut tx = state.pool.begin().await?;
    let target_tenants = lock_global_user_mutation(&state, &user, id, &mut tx).await?;
    let before = user_summary_in_tx(&mut tx, id).await?;
    let (real_name, email, phone, gender): (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as("SELECT real_name, email, phone, gender FROM users WHERE id=$1")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    let changed_fields: Vec<&str> = [
        (
            "realName",
            body.real_name
                .as_ref()
                .is_some_and(|value| Some(value) != real_name.as_ref()),
        ),
        (
            "email",
            body.email
                .as_ref()
                .is_some_and(|value| Some(value) != email.as_ref()),
        ),
        (
            "phone",
            body.phone
                .as_ref()
                .is_some_and(|value| Some(value) != phone.as_ref()),
        ),
        (
            "gender",
            body.gender
                .as_ref()
                .is_some_and(|value| Some(value) != gender.as_ref()),
        ),
    ]
    .into_iter()
    .filter_map(|(field, changed)| changed.then_some(field))
    .collect();
    let _: (String,) = sqlx::query_as(
        "UPDATE users SET real_name = COALESCE($2, real_name), email = COALESCE($3, email), \
         phone = COALESCE($4, phone), gender = COALESCE($5, gender), updated_at = now() \
         WHERE id = $1 RETURNING username",
    )
    .bind(id)
    .bind(&body.real_name)
    .bind(&body.email)
    .bind(&body.phone)
    .bind(&body.gender)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("用户不存在"))?;
    if !changed_fields.is_empty() {
        let mut after = user_summary_in_tx(&mut tx, id).await?;
        after["updatedFields"] = json!(changed_fields);
        audit_user_change_in_tx(
            &mut tx,
            &headers,
            &user,
            &target_tenants,
            "UPDATE_USER",
            &audit::change_detail("user", id, before, after),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"ok": true})))
}

/// DELETE /api/users/{id}
async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user")
        .check(&state, &user)
        .await?;
    let id: Uuid = id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    if id == user.user_id {
        return Err(AppError::bad_request("不能删除当前登录用户"));
    }
    if state.remote_accounts.is_some() {
        return remote::mutate(&state, &headers, &user, id, "RETIRE", json!({}), None)
            .await
            .map(Json);
    }
    let mut tx = state.pool.begin().await?;
    let target_tenants = lock_global_user_mutation(&state, &user, id, &mut tx).await?;
    let before = user_summary_in_tx(&mut tx, id).await?;
    let _: (String,) = sqlx::query_as("DELETE FROM users WHERE id = $1 RETURNING username")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::not_found("用户不存在"))?;
    audit_user_change_in_tx(
        &mut tx,
        &headers,
        &user,
        &target_tenants,
        "DELETE_USER",
        &audit::change_detail("user", id, before, Value::Null),
    )
    .await?;
    tx.commit().await?;
    state.invalidate_perm_cache();
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct StatusBody {
    status: String,
}

/// PUT /api/users/{id}/status
async fn update_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<StatusBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user")
        .check(&state, &user)
        .await?;
    if !["ENABLED", "DISABLED"].contains(&body.status.as_str()) {
        return Err(AppError::bad_request("状态仅支持 ENABLED / DISABLED"));
    }
    let id: Uuid = id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    if id == user.user_id && body.status == "DISABLED" {
        return Err(AppError::bad_request("不能禁用当前登录用户"));
    }
    if state.remote_accounts.is_some() {
        return remote::mutate(
            &state,
            &headers,
            &user,
            id,
            "SET_STATUS",
            json!({"status":body.status}),
            None,
        )
        .await
        .map(Json);
    }
    let mut tx = state.pool.begin().await?;
    let target_tenants = lock_global_user_mutation(&state, &user, id, &mut tx).await?;
    let before = user_summary_in_tx(&mut tx, id).await?;
    let changed = {
        let mut accounts = crate::authentication::PostgresAccountTransaction::new(&mut tx);
        state
            .accounts
            .set_status(&mut accounts, id, &body.status)
            .await?
    };
    if !changed {
        return Err(AppError::not_found("用户不存在"));
    }
    let after = user_summary_in_tx(&mut tx, id).await?;
    if before != after {
        audit_user_change_in_tx(
            &mut tx,
            &headers,
            &user,
            &target_tenants,
            "UPDATE_USER",
            &audit::change_detail("user", id, before, after),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchStatusBody {
    #[serde(deserialize_with = "crate::util::uuid7::deserialize_vec")]
    user_ids: Vec<Uuid>,
    status: String,
}

/// POST /api/users/batch-status — 批量启用/禁用
///
/// 与单条 PUT /{id}/status 行为一致(相同权限点 + 相同 UPDATE 语句), 差异:
/// - 服务端自我防护: 当前登录用户自身 id 无条件跳过(计入 skipped, 不计成败)
/// - 重复 id 去重(保序)后只执行一次
/// - 不存在的 id 计入 failed, 成功的计入 succeeded, 经响应逐类传达
/// - 上限 100 个 id, 超出 BAD_REQUEST(防误操作)
///
/// 事务性: 每个用户的 UPDATE 与审计使用独立事务，不包整体事务。部分成功/部分失败
/// 是可接受的(语义由响应 succeeded/failed/skipped 传达), 且避免单行异常回滚拖垮整批;
/// 仅数据库级错误(连接等)才中断后续行并向上返回 500。
async fn batch_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<BatchStatusBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user")
        .check(&state, &user)
        .await?;
    if !["ENABLED", "DISABLED"].contains(&body.status.as_str()) {
        return Err(AppError::bad_request("状态仅支持 ENABLED / DISABLED"));
    }
    if body.user_ids.is_empty() || body.user_ids.len() > 100 {
        return Err(AppError::bad_request("userIds 数量必须在 1 到 100 之间"));
    }
    // 去重(保序) + 过滤自身(防止把自己禁用/启用, 计入 skipped)
    // 跨租户目标和共享账号直接计入 failed, 不执行不中断整批
    let mut seen = std::collections::HashSet::new();
    let (mut targets, mut skipped, mut failed): (Vec<Uuid>, Vec<Uuid>, Vec<Uuid>) =
        (Vec::new(), Vec::new(), Vec::new());
    for id in body.user_ids {
        if !seen.insert(id) {
            continue;
        }
        if id == user.user_id {
            skipped.push(id);
        } else {
            targets.push(id);
        }
    }
    let mut succeeded: Vec<Uuid> = Vec::new();
    for id in &targets {
        if state.remote_accounts.is_some() {
            match remote::mutate(
                &state,
                &headers,
                &user,
                *id,
                "SET_STATUS",
                json!({"status":body.status}),
                None,
            )
            .await
            {
                Ok(_) => succeeded.push(*id),
                Err(AppError::NotFound(_)) | Err(AppError::Forbidden(_)) => failed.push(*id),
                Err(error) => return Err(error),
            }
            continue;
        }
        let mut tx = state.pool.begin().await?;
        let target_tenants = match lock_global_user_mutation(&state, &user, *id, &mut tx).await {
            Ok(tenants) => tenants,
            Err(AppError::NotFound(_)) | Err(AppError::Forbidden(_)) => {
                failed.push(*id);
                continue;
            }
            Err(error) => return Err(error),
        };
        let before = user_summary_in_tx(&mut tx, *id).await?;
        let changed = {
            let mut accounts = crate::authentication::PostgresAccountTransaction::new(&mut tx);
            state
                .accounts
                .set_status(&mut accounts, *id, &body.status)
                .await?
        };
        if changed {
            let after = user_summary_in_tx(&mut tx, *id).await?;
            if before != after {
                audit_user_change_in_tx(
                    &mut tx,
                    &headers,
                    &user,
                    &target_tenants,
                    "UPDATE_USER",
                    &audit::change_detail("user", *id, before, after),
                )
                .await?;
            }
            tx.commit().await?;
            succeeded.push(*id);
        } else {
            failed.push(*id);
        }
    }
    Ok(Json(
        json!({ "succeeded": succeeded, "failed": failed, "skipped": skipped }),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResetPasswordBody {
    new_password: String,
}

/// POST /api/users/{id}/reset-password
async fn reset_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ResetPasswordBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user")
        .check(&state, &user)
        .await?;
    state.accounts.validate_password(&body.new_password)?;
    let id: Uuid = id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    if state.remote_accounts.is_some() {
        return remote::mutate(
            &state,
            &headers,
            &user,
            id,
            "RESET_PASSWORD",
            json!({"newPassword":body.new_password}),
            None,
        )
        .await
        .map(Json);
    }
    let password = state.accounts.prepare_password(&body.new_password).await?;
    let mut tx = state.pool.begin().await?;
    let target_tenants = lock_global_user_mutation(&state, &user, id, &mut tx).await?;
    let before = user_summary_in_tx(&mut tx, id).await?;
    let change = {
        let mut accounts = crate::authentication::PostgresAccountTransaction::new(&mut tx);
        state
            .accounts
            .replace_password(&mut accounts, id, &password, None)
            .await?
    };
    let contexts = system_recipient_contexts(&mut tx, id, &target_tenants).await?;
    system_in_tx(
        &mut tx,
        &contexts,
        "SECURITY",
        "密码已被管理员重置",
        &format!(
            "管理员 {} 刚刚重置了您的登录密码。若非您本人申请,请立即联系平台管理员。",
            user.username
        ),
        "/profile",
    )
    .await?;
    let mut after = user_summary_in_tx(&mut tx, id).await?;
    after["passwordChanged"] = json!(true);
    after["revokedSessions"] = json!(change.revoked_sessions);
    audit_user_change_in_tx(
        &mut tx,
        &headers,
        &user,
        &target_tenants,
        "RESET_PASSWORD",
        &audit::change_detail("user", id, before, after),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok": true})))
}

type MembershipTuple = (Uuid, Uuid, Option<Uuid>, String, String, Uuid, String, i64);

/// GET /api/users/{id}/memberships
async fn get_memberships(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user")
        .check(&state, &user)
        .await?;
    let id: Uuid = id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_user_in_scope(&state, &user, id).await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1)")
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    if !exists {
        return Err(AppError::not_found("用户不存在"));
    }
    let rows: Vec<MembershipTuple> = sqlx::query_as(
        "SELECT m.organization_id, o.tenant_id, m.department_id, m.position, o.name, m.id, m.status, m.identity_version \
         FROM memberships m JOIN organizations o ON o.id = m.organization_id \
         WHERE m.user_id = $1 AND m.member_type='EMPLOYEE' \
         AND ($2::uuid[] IS NULL OR m.organization_id = ANY($2)) ORDER BY m.created_at",
    )
    .bind(id)
    .bind(state.scoped_org_ids(&user).await?)
    .fetch_all(&state.pool)
    .await?;
    let dept_ids: Vec<Uuid> = rows.iter().filter_map(|r| r.2).collect();
    let mut dept_names: std::collections::HashMap<Uuid, String> = std::collections::HashMap::new();
    if !dept_ids.is_empty() {
        let named: Vec<(Uuid, String)> =
            sqlx::query_as("SELECT id, name FROM departments WHERE id = ANY($1)")
                .bind(&dept_ids)
                .fetch_all(&state.pool)
                .await
                .unwrap_or_default();
        for (did, name) in named {
            dept_names.insert(did, name);
        }
    }
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(organization_id, tenant_id, department_id, position, org_name, membership_id, status, identity_version)| {
            json!({
                "membershipId": membership_id,
                "memberType": "EMPLOYEE", "status": status, "identityVersion": identity_version,
                "organizationId": organization_id, "tenantId": tenant_id,
                "departmentId": department_id, "position": position, "organizationName": org_name,
                "departmentName": department_id.and_then(|d| dept_names.get(&d).cloned()),
            })
        })
        .collect();
    Ok(Json(Value::Array(items)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MembershipsBody {
    #[serde(default)]
    items: Vec<MembershipItem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MembershipItem {
    organization_id: String,
    #[serde(default)]
    department_id: Option<String>,
    #[serde(default)]
    position: String,
}

/// PUT /api/users/{id}/memberships — 仅替换管理范围内员工关系，保留客户及范围外关系。
async fn put_memberships(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<MembershipsBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user")
        .check(&state, &user)
        .await?;
    let uid: Uuid = id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_user_in_scope(&state, &user, uid).await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1)")
        .bind(uid)
        .fetch_one(&state.pool)
        .await?;
    if !exists {
        return Err(AppError::not_found("用户不存在"));
    }
    let scoped_orgs = state.scoped_org_ids(&user).await?;
    let platform = scoped_orgs.is_none();
    let mut org_ids: Vec<Uuid> = Vec::new();
    let mut dept_ids: Vec<Option<Uuid>> = Vec::new();
    let mut positions: Vec<String> = Vec::new();
    for item in &body.items {
        let org: Uuid = item
            .organization_id
            .parse_uuid7()
            .map_err(|_| AppError::bad_request("organizationId 格式错误"))?;
        org_ids.push(org);
        let dept: Option<Uuid> = match &item.department_id {
            Some(d) if !d.trim().is_empty() => Some(
                d.parse_uuid7()
                    .map_err(|_| AppError::bad_request("departmentId 格式错误"))?,
            ),
            _ => None,
        };
        dept_ids.push(dept);
        positions.push(item.position.clone());
    }
    if !org_ids.is_empty() {
        let unique_count = org_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len();
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM organizations WHERE id = ANY($1) \
                                        AND ($2::uuid[] IS NULL OR id = ANY($2))",
        )
        .bind(&org_ids)
        .bind(&scoped_orgs)
        .fetch_one(&state.pool)
        .await?;
        if n != unique_count as i64 {
            return Err(AppError::not_found("组织不存在"));
        }
    }
    let flat_depts: Vec<Uuid> = dept_ids.iter().filter_map(|d| *d).collect();
    if !flat_depts.is_empty() {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM departments WHERE id = ANY($1)")
            .bind(&flat_depts)
            .fetch_one(&state.pool)
            .await?;
        if n != flat_depts
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len() as i64
        {
            return Err(AppError::not_found("部门不存在"));
        }
    }
    for (org, dept) in org_ids.iter().zip(&dept_ids) {
        if let Some(dept) = dept {
            let valid: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM departments WHERE id = $1 AND organization_id = $2)",
            )
            .bind(dept)
            .bind(org)
            .fetch_one(&state.pool)
            .await?;
            if !valid {
                return Err(AppError::not_found("部门不存在"));
            }
        }
    }
    // Duplicate employee identities are a conflict, never an implicit merge.
    if org_ids
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len()
        != org_ids.len()
    {
        return Err(AppError::conflict("同一组织只能提交一条员工身份"));
    }
    let mut seen = std::collections::HashSet::new();
    let mut tx = state.pool.begin().await?;
    lock_scoped_member(&mut tx, &user, uid, scoped_orgs.as_deref()).await?;
    require_employee_permission_in_tx(&mut tx, &user, "system:user").await?;
    require_scope_unchanged_in_tx(&mut tx, &user, scoped_orgs.as_deref()).await?;
    let deferred_self_departure = if uid == user.user_id && !org_ids.contains(&user.org_id) {
        let proof = authorization::require_employee_context_in_tx(&mut tx, &user).await?;
        // This exact member remains the authority for the operation until the
        // last proof. Lock it now, before taking its credential/cascade locks.
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM memberships WHERE id=$1 AND user_id=$2 \
             AND organization_id=$3 AND member_type='EMPLOYEE' FOR UPDATE",
        )
        .bind(proof.membership_id)
        .bind(uid)
        .bind(user.org_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::unauthorized("当前员工身份已失效"))?;
        Some(proof.membership_id)
    } else {
        None
    };
    let before_tenants = if platform {
        user_tenant_ids_in_tx(&mut tx, uid).await?
    } else {
        vec![user.tenant_id]
    };
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id=$1")
        .bind(uid)
        .fetch_one(&mut *tx)
        .await?;
    let before = memberships_summary_in_tx(&mut tx, uid, scoped_orgs.as_deref()).await?;
    let before_items: Vec<(Uuid, Option<Uuid>, String)> = sqlx::query_as(
        "SELECT organization_id, department_id, position FROM memberships WHERE user_id=$1 AND member_type='EMPLOYEE' \
         AND ($2::uuid[] IS NULL OR organization_id=ANY($2)) ORDER BY organization_id",
    )
    .bind(uid)
    .bind(&scoped_orgs)
    .fetch_all(&mut *tx)
    .await?;
    // Preserve retained membership IDs, data grants and active sessions. The
    // membership deletion trigger revokes only contexts actually removed.
    let mut departed_orgs: Vec<Uuid> = sqlx::query_scalar(
        "DELETE FROM memberships m USING organizations o \
                 WHERE m.organization_id = o.id AND m.user_id = $1 AND m.member_type='EMPLOYEE' \
                 AND ($2::uuid[] IS NULL OR m.organization_id = ANY($2)) \
                 AND NOT (m.organization_id = ANY($3)) \
                 AND ($4::uuid IS NULL OR m.id<>$4) RETURNING m.organization_id",
    )
    .bind(uid)
    .bind(&scoped_orgs)
    .bind(&org_ids)
    .bind(deferred_self_departure)
    .fetch_all(&mut *tx)
    .await?;
    for (i, org) in org_ids.iter().enumerate() {
        if !seen.insert(*org) {
            continue;
        }
        crate::membership_lifecycle::ensure_employee_creation_allowed_in_tx(
            &mut tx,
            uid,
            *org,
            state.config.dual_memberships_enabled,
        )
        .await?;
        sqlx::query(
            "INSERT INTO memberships (user_id, organization_id, department_id, position, member_type, id) VALUES ($1, $2, $3, $4, 'EMPLOYEE', $5) \
             ON CONFLICT(user_id,organization_id,member_type) DO UPDATE \
             SET department_id=EXCLUDED.department_id, position=EXCLUDED.position",
        )
        .bind(uid)
        .bind(org)
        .bind(dept_ids[i])
        .bind(&positions[i])
        .bind(crate::util::uuid7::new_uuid()).execute(&mut *tx)
        .await?;
    }
    if deferred_self_departure.is_some() {
        departed_orgs.push(user.org_id);
    } else {
        crate::membership_lifecycle::cleanup_employee_roles_after_departure_in_tx(
            &mut tx,
            uid,
            &departed_orgs,
        )
        .await?;
    }
    let after_tenants = if platform {
        user_tenant_ids_in_tx(&mut tx, uid).await?
    } else {
        vec![user.tenant_id]
    };
    let mut after = memberships_summary_in_tx(&mut tx, uid, scoped_orgs.as_deref()).await?;
    if let Some(deferred) = deferred_self_departure {
        // Audit the requested final employee set while the last self-removal
        // is deliberately pending, so audit rejection can still roll it back.
        if let Some(items) = after.as_array_mut() {
            items.retain(|item| item["membershipId"] != json!(deferred));
        }
    }
    let after_items: Vec<(Uuid, Option<Uuid>, String)> = sqlx::query_as(
        "SELECT organization_id, department_id, position FROM memberships WHERE user_id=$1 AND member_type='EMPLOYEE' \
         AND ($2::uuid[] IS NULL OR organization_id=ANY($2)) \
         AND ($3::uuid IS NULL OR id<>$3) ORDER BY organization_id",
    )
    .bind(uid)
    .bind(&scoped_orgs)
    .bind(deferred_self_departure)
    .fetch_all(&mut *tx)
    .await?;
    if before_items != after_items {
        let before_by_org: HashMap<_, _> = before_items
            .iter()
            .map(|(org, dept, position)| (*org, (*dept, position.as_str())))
            .collect();
        let after_by_org: HashMap<_, _> = after_items
            .iter()
            .map(|(org, dept, position)| (*org, (*dept, position.as_str())))
            .collect();
        let mut changed_organizations: Vec<Uuid> = before_by_org
            .keys()
            .chain(after_by_org.keys())
            .copied()
            .filter(|org| before_by_org.get(org) != after_by_org.get(org))
            .collect();
        changed_organizations.sort_unstable();
        changed_organizations.dedup();
        audit_user_change_in_tx(
            &mut tx,
            &headers,
            &user,
            &merge_tenant_ids(before_tenants, after_tenants),
            "UPDATE_USER",
            &audit::change_detail(
                "user_memberships",
                uid,
                json!({"username": username, "items": before}),
                json!({"username": username, "items": after,
                    "changedOrganizationIds": changed_organizations}),
            ),
        )
        .await?;
    }
    if let Some(deferred) = deferred_self_departure {
        lock_self_departure_children_in_tx(&mut tx, uid, deferred).await?;
    }
    require_scope_unchanged_in_tx(&mut tx, &user, scoped_orgs.as_deref()).await?;
    require_employee_permission_in_tx(&mut tx, &user, "system:user").await?;
    if let Some(deferred) = deferred_self_departure {
        let removed = sqlx::query(
            "DELETE FROM memberships WHERE id=$1 AND user_id=$2 AND member_type='EMPLOYEE'",
        )
        .bind(deferred)
        .bind(uid)
        .execute(&mut *tx)
        .await?;
        if removed.rows_affected() != 1 {
            return Err(AppError::conflict("当前员工身份已变更，请重新读取后重试"));
        }
        crate::membership_lifecycle::cleanup_employee_roles_after_departure_in_tx(
            &mut tx,
            uid,
            &departed_orgs,
        )
        .await?;
    }
    tx.commit().await?;
    state.invalidate_perm_cache();
    Ok(Json(json!({"ok": true})))
}

/// Put every runtime row-lock wait before the final self-removal authority
/// proof. The caller already holds the account and exact membership locks:
/// issuers/role writers cannot add rows, and the member FK blocks new grants.
async fn lock_self_departure_children_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    membership_id: Uuid,
) -> Result<(), AppError> {
    for query in [
        // Application deletion cascades grant rows before token rows. Keep
        // that same child order without acquiring an application parent lock.
        "SELECT id FROM membership_data_grants WHERE membership_id=$1 ORDER BY id FOR UPDATE",
        "SELECT id FROM platform_sessions WHERE membership_id=$1 ORDER BY id FOR UPDATE",
        "SELECT r.id FROM platform_session_refresh_tokens r JOIN platform_sessions s ON s.id=r.session_id \
         WHERE s.membership_id=$1 ORDER BY r.id FOR UPDATE OF r",
        "SELECT id FROM oauth_tokens WHERE membership_id=$1 ORDER BY id FOR UPDATE",
        "SELECT id FROM oauth_authorization_codes WHERE membership_id=$1 ORDER BY id FOR UPDATE",
        "SELECT id FROM oauth_device_codes WHERE membership_id=$1 ORDER BY id FOR UPDATE",
    ] {
        sqlx::query_scalar::<_, Uuid>(query).bind(membership_id).fetch_all(&mut **tx).await?;
    }
    // Final ancestor cleanup can affect several departed employee relations.
    sqlx::query("SELECT user_id FROM user_roles WHERE user_id=$1 ORDER BY role_id,organization_id FOR UPDATE")
        .bind(user_id).execute(&mut **tx).await?;
    Ok(())
}

type UserRoleTuple = (Uuid, Uuid);

/// 成员与角色维护先锁账号，再重查可管理成员关系，防止并发移出范围后继续写入。
async fn lock_scoped_member(
    tx: &mut Transaction<'_, Postgres>,
    actor: &CurrentUser,
    target: Uuid,
    scoped_orgs: Option<&[Uuid]>,
) -> Result<(), AppError> {
    let mut ids = vec![actor.user_id, target];
    ids.sort_unstable();
    ids.dedup();
    let locked: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM users WHERE id = ANY($1) ORDER BY id FOR NO KEY UPDATE")
            .bind(&ids)
            .fetch_all(&mut **tx)
            .await?;
    if !locked.contains(&target) {
        return Err(AppError::not_found("用户不存在"));
    }
    if let Some(org_ids) = scoped_orgs {
        let belongs: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM memberships WHERE user_id = $1 AND organization_id = ANY($2))",
        )
        .bind(target)
        .bind(org_ids)
        .fetch_one(&mut **tx)
        .await?;
        if !belongs {
            return Err(AppError::not_found("用户不存在"));
        }
    }
    Ok(())
}

/// The caller holds the user's row lock, shared with membership maintenance.
/// Validate after taking it so a member move cannot leave a newly assigned
/// role outside every remaining membership's ancestor chain.
pub(super) async fn ensure_role_membership_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    role_org: Uuid,
) -> Result<(), AppError> {
    let valid: bool = sqlx::query_scalar(&format!(
        "SELECT {}",
        role_employee_support_sql("$1", "$2", "NULL::uuid[]")
    ))
    .bind(user_id)
    .bind(role_org)
    .fetch_one(&mut **tx)
    .await?;
    if !valid {
        return Err(AppError::bad_request(
            "角色所属组织必须有有效员工身份或其后代员工身份支持",
        ));
    }
    Ok(())
}

/// GET /api/users/{id}/roles
async fn get_roles(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user")
        .check(&state, &user)
        .await?;
    let id: Uuid = id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_user_in_scope(&state, &user, id).await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1)")
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    if !exists {
        return Err(AppError::not_found("用户不存在"));
    }
    let employee_support = role_employee_support_sql("ur.user_id", "ur.organization_id", "$2");
    let rows: Vec<UserRoleTuple> = sqlx::query_as(&format!(
        "SELECT ur.role_id, ur.organization_id FROM user_roles ur \
         JOIN roles r ON r.id = ur.role_id WHERE ur.user_id = $1 \
         AND ur.organization_id=r.organization_id AND {employee_support} \
         AND ($2::uuid[] IS NULL OR (r.organization_id = ANY($2) AND ur.organization_id = ANY($2)))",
    ))
    .bind(id)
    .bind(state.scoped_org_ids(&user).await?)
    .fetch_all(&state.pool)
    .await?;
    // 附带角色/组织名称(用户详情抽屉使用; 旧字段保持不变)
    let named: Vec<(Uuid, Uuid, String, String, String)> = sqlx::query_as(&format!(
        "SELECT ur.role_id, ur.organization_id, COALESCE(r.name,''), COALESCE(r.code,''), COALESCE(o.name,'') \
         FROM user_roles ur \
         LEFT JOIN roles r ON r.id = ur.role_id \
         LEFT JOIN organizations o ON o.id = ur.organization_id \
         WHERE ur.user_id = $1 AND ur.organization_id=r.organization_id AND {employee_support} \
         AND ($2::uuid[] IS NULL OR (r.organization_id = ANY($2) AND ur.organization_id = ANY($2))) ORDER BY r.name",
    ))
    .bind(id)
    .bind(state.scoped_org_ids(&user).await?)
    .fetch_all(&state.pool)
    .await?;
    let name_map: std::collections::HashMap<Uuid, (String, String, String)> = named
        .into_iter()
        .map(|(rid, _, rname, rcode, oname)| (rid, (rname, rcode, oname)))
        .collect();
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(role_id, organization_id)| {
            let extra = name_map.get(&role_id);
            json!({
                "roleId": role_id, "organizationId": organization_id,
                "roleName": extra.map(|e| e.0.clone()).unwrap_or_default(),
                "roleCode": extra.map(|e| e.1.clone()).unwrap_or_default(),
                "organizationName": extra.map(|e| e.2.clone()).unwrap_or_default(),
            })
        })
        .collect();
    Ok(Json(Value::Array(items)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RolesBody {
    #[serde(default)]
    items: Vec<RoleItem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoleItem {
    role_id: String,
    #[serde(default)]
    organization_id: String,
}

/// PUT /api/users/{id}/roles — 角色所属组织需在用户 membership 组织(或其祖先)内
async fn put_roles(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<RolesBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:user")
        .check(&state, &user)
        .await?;
    let uid: Uuid = id
        .parse_uuid7()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_user_in_scope(&state, &user, uid).await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1)")
        .bind(uid)
        .fetch_one(&state.pool)
        .await?;
    if !exists {
        return Err(AppError::not_found("用户不存在"));
    }
    let scoped_orgs = state.scoped_org_ids(&user).await?;
    let platform = scoped_orgs.is_none();
    let mut parsed: Vec<(Uuid, Uuid)> = Vec::new();
    for item in &body.items {
        let role: Uuid = item
            .role_id
            .parse_uuid7()
            .map_err(|_| AppError::bad_request("roleId 格式错误"))?;
        let org: Uuid = if item.organization_id.trim().is_empty() {
            let (role_org,): (Uuid,) =
                sqlx::query_as("SELECT organization_id FROM roles WHERE id = $1")
                    .bind(role)
                    .fetch_optional(&state.pool)
                    .await?
                    .ok_or_else(|| AppError::bad_request("角色不存在"))?;
            role_org
        } else {
            item.organization_id
                .parse_uuid7()
                .map_err(|_| AppError::bad_request("organizationId 格式错误"))?
        };
        parsed.push((role, org));
    }
    let mut tx = state.pool.begin().await?;
    lock_scoped_member(&mut tx, &user, uid, scoped_orgs.as_deref()).await?;
    require_employee_permission_in_tx(&mut tx, &user, "system:user").await?;
    require_scope_unchanged_in_tx(&mut tx, &user, scoped_orgs.as_deref()).await?;
    if !parsed.is_empty() {
        let mut role_ids: Vec<Uuid> = parsed.iter().map(|(r, _)| *r).collect();
        role_ids.sort_unstable();
        role_ids.dedup();
        // Take requested parent locks before changing any child rows. A role
        // delete locks its parent before cascading; deleting children first and
        // acquiring their parent only on reinsert would reverse that order.
        let locked_roles: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT id,organization_id FROM roles WHERE id = ANY($1) \
             AND ($2::uuid[] IS NULL OR organization_id = ANY($2)) \
             ORDER BY id FOR KEY SHARE",
        )
        .bind(&role_ids)
        .bind(&scoped_orgs)
        .fetch_all(&mut *tx)
        .await?;
        if locked_roles.len() != role_ids.len() {
            return Err(AppError::bad_request("存在无效的角色"));
        }
        require_employee_permission_in_tx(&mut tx, &user, "system:user").await?;
        require_scope_unchanged_in_tx(&mut tx, &user, scoped_orgs.as_deref()).await?;
        let locked_roles: HashMap<Uuid, Uuid> = locked_roles.into_iter().collect();
        // 校验角色组织 ∈ (membership 组织 ∪ 其祖先)
        for (role, org) in &parsed {
            if locked_roles.get(role) != Some(org) {
                return Err(AppError::bad_request(
                    "角色授权上下文组织与角色所属组织不一致",
                ));
            }
            ensure_role_membership_in_tx(&mut tx, uid, *org).await?;
        }
    }
    let employee_support = role_employee_support_sql("ur.user_id", "ur.organization_id", "$2");
    let current: Vec<(Uuid, Uuid)> = sqlx::query_as(&format!(
        "SELECT ur.role_id, ur.organization_id FROM user_roles ur JOIN roles r ON r.id=ur.role_id \
         WHERE ur.user_id=$1 AND ur.organization_id=r.organization_id AND {employee_support} \
         AND ($2::uuid[] IS NULL OR \
         (r.organization_id=ANY($2) AND ur.organization_id=ANY($2))) \
         ORDER BY ur.role_id, ur.organization_id",
    ))
    .bind(uid)
    .bind(&scoped_orgs)
    .fetch_all(&mut *tx)
    .await?;
    let mut requested = parsed.clone();
    requested.sort_unstable();
    requested.dedup();
    if current == requested {
        require_scope_unchanged_in_tx(&mut tx, &user, scoped_orgs.as_deref()).await?;
        require_employee_permission_in_tx(&mut tx, &user, "system:user").await?;
        tx.commit().await?;
        return Ok(Json(json!({"ok": true})));
    }
    let before_tenants = if platform {
        user_role_tenant_ids_in_tx(&mut tx, uid).await?
    } else {
        vec![user.tenant_id]
    };
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id=$1")
        .bind(uid)
        .fetch_one(&mut *tx)
        .await?;
    let before = roles_summary_in_tx(&mut tx, uid, scoped_orgs.as_deref()).await?;
    sqlx::query(&format!(
        "DELETE FROM user_roles ur USING roles r \
                 WHERE ur.role_id = r.id AND ur.user_id = $1 \
                 AND ur.organization_id=r.organization_id AND {employee_support} \
                 AND ($2::uuid[] IS NULL OR (r.organization_id = ANY($2) AND ur.organization_id = ANY($2)))",
    ))
    .bind(uid)
    .bind(&scoped_orgs)
    .execute(&mut *tx)
    .await?;
    for (role, org) in &parsed {
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_id, organization_id) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(uid)
        .bind(role)
        .bind(org)
        .execute(&mut *tx)
        .await?;
    }
    let after_tenants = if platform {
        user_role_tenant_ids_in_tx(&mut tx, uid).await?
    } else {
        vec![user.tenant_id]
    };
    let affected_tenants = merge_tenant_ids(before_tenants, after_tenants);
    let mut contexts = system_recipient_contexts(&mut tx, uid, &affected_tenants).await?;
    if let Some(org_ids) = &scoped_orgs {
        contexts.retain(|(_, _, org)| org_ids.contains(org));
    }
    system_in_tx(
        &mut tx,
        &contexts,
        "INFO",
        "您的角色分配已更新",
        &format!(
            "管理员 {} 更新了您的角色分配(共 {} 项)。新角色将在下次进入相应组织上下文时生效。",
            user.username,
            parsed.len()
        ),
        "/profile",
    )
    .await?;
    let after = roles_summary_in_tx(&mut tx, uid, scoped_orgs.as_deref()).await?;
    audit_user_change_in_tx(
        &mut tx,
        &headers,
        &user,
        &affected_tenants,
        "UPDATE_USER",
        &audit::change_detail(
            "user_roles",
            uid,
            json!({"username": username, "items": before}),
            json!({"username": username, "items": after}),
        ),
    )
    .await?;
    for (_, org) in &parsed {
        ensure_role_membership_in_tx(&mut tx, uid, *org).await?;
    }
    require_scope_unchanged_in_tx(&mut tx, &user, scoped_orgs.as_deref()).await?;
    require_employee_permission_in_tx(&mut tx, &user, "system:user").await?;
    tx.commit().await?;
    state.invalidate_perm_cache();
    Ok(Json(json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    use super::MembershipsBody;
    use serde_json::json;

    #[test]
    fn legacy_membership_request_cannot_smuggle_identity_or_status_changes() {
        let organization_id = crate::util::uuid7::new_uuid().to_string();
        let request = json!({"items": [{"organizationId": organization_id}]});
        assert!(serde_json::from_value::<MembershipsBody>(request).is_ok());
        for (field, value) in [
            ("memberType", json!("CUSTOMER")),
            ("status", json!("ENABLED")),
            ("identityVersion", json!(2)),
            ("membershipId", json!(crate::util::uuid7::new_uuid())),
        ] {
            let mut request = json!({"items": [{"organizationId": organization_id}]});
            request["items"][0][field] = value;
            assert!(
                serde_json::from_value::<MembershipsBody>(request).is_err(),
                "legacy membership update accepted {field}"
            );
        }
    }
}
