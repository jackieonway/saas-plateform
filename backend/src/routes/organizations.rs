use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::AppError;
use crate::extractors::{ApiJson, CurrentUser, RequirePermission};
use crate::routes::contact::{Contact, ContactUpdate};
use crate::routes::onboarding;
use crate::routes::tenants::validate_coordinates;
use crate::state::AppState;
use crate::util::{audit, tree};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/organizations/tree", get(tree_handler))
        .route("/api/organizations", post(create))
        .route("/api/organizations/{id}", put(update).delete(delete))
        .route("/api/organizations/{id}/status", put(update_status))
        .route(
            "/api/organizations/{id}/initialize-administrator",
            post(initialize_administrator),
        )
        .route(
            "/api/organizations/{id}/authorizations",
            get(get_authorizations).put(put_authorizations),
        )
}

#[derive(sqlx::FromRow)]
struct OrgRow {
    id: Uuid,
    tenant_id: Uuid,
    parent_id: Option<Uuid>,
    name: String,
    code: String,
    organization_type: String,
    contact_name: String,
    contact_phone: String,
    contact_email: String,
    sort_order: i32,
    description: String,
    status: String,
    expires_at: Option<DateTime<Utc>>,
    province: String,
    city: String,
    district: String,
    township: String,
    village: String,
    address: String,
    longitude: Option<f64>,
    latitude: Option<f64>,
}

const ORG_SELECT: &str = "SELECT o.id, o.tenant_id, o.parent_id, o.name, COALESCE(o.code,'') AS code, o.organization_type, o.contact_name, o.contact_phone, o.contact_email, \
 o.sort_order, COALESCE(o.description,'') AS description, o.status, o.expires_at, \
 o.province, o.city, o.district, o.township, o.village, o.address, o.longitude, o.latitude";

/// Keep ordinary edit/no-op comparisons limited to this locked object's fields.
async fn org_configuration_snapshot(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
) -> Result<(Uuid, Value), AppError> {
    let (tenant_id, parent_id, name, code, organization_type, status, expires_at): (
        Uuid,
        Option<Uuid>,
        String,
        String,
        String,
        String,
        Option<DateTime<Utc>>,
    ) = sqlx::query_as(
        "SELECT tenant_id,parent_id,name,COALESCE(code,''),organization_type,status,expires_at \
         FROM organizations WHERE id=$1 FOR NO KEY UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| AppError::not_found("组织不存在"))?;
    let configuration: String = sqlx::query_scalar(
        "SELECT jsonb_build_array(name,code,organization_type,status,expires_at,sort_order,contact_name,contact_phone,contact_email, \
         description,province,city,district,township,village,address,longitude,latitude)::text FROM organizations WHERE id=$1",
    ).bind(id).fetch_one(&mut **tx).await?;
    Ok((
        tenant_id,
        json!({
            "id": id, "tenantId": tenant_id, "parentId": parent_id, "name": name, "code": code,
            "organizationType": organization_type, "status": status, "expiresAt": expires_at,
            "configurationHash": crate::util::hash::sha256_hex(&configuration),
        }),
    ))
}

/// The caller holds this organization's row lock. The platform root is the
/// only cross-tenant administration context, so losing it locks out recovery
/// through every ordinary management endpoint as well.
async fn is_platform_root_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
) -> Result<bool, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM organizations o JOIN tenants t ON t.id=o.tenant_id \
         WHERE o.id=$1 AND o.parent_id IS NULL AND t.is_platform)",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await?)
}

/// Subtree totals preserve the extent of cascading creation/deletion in audits.
async fn org_audit_snapshot(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
) -> Result<(Uuid, Value), AppError> {
    let (tenant_id, mut snapshot) = org_configuration_snapshot(tx, id).await?;
    let (
        members,
        roles,
        assignments,
        applications,
        menus,
        subtree_orgs,
        subtree_members,
        subtree_roles,
    ): (i64, i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "WITH RECURSIVE subtree(id) AS (SELECT id FROM organizations WHERE id=$1 \
         UNION ALL SELECT o.id FROM organizations o JOIN subtree s ON o.parent_id=s.id) \
         SELECT (SELECT count(*) FROM memberships WHERE organization_id=$1), \
         (SELECT count(*) FROM roles WHERE organization_id=$1), \
         (SELECT count(*) FROM user_roles WHERE organization_id=$1), \
         (SELECT count(*) FROM organization_applications WHERE organization_id=$1), \
         (SELECT count(*) FROM organization_menus WHERE organization_id=$1), \
         (SELECT count(*) FROM subtree), \
         (SELECT count(*) FROM memberships WHERE organization_id IN (SELECT id FROM subtree)), \
         (SELECT count(*) FROM roles WHERE organization_id IN (SELECT id FROM subtree))",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await?;
    let authorization: String = sqlx::query_scalar(
        "SELECT jsonb_build_array( \
         (SELECT COALESCE(jsonb_agg(application_id ORDER BY application_id),'[]'::jsonb) FROM organization_applications WHERE organization_id=$1), \
         (SELECT COALESCE(jsonb_agg(menu_id ORDER BY menu_id),'[]'::jsonb) FROM organization_menus WHERE organization_id=$1), \
         (SELECT COALESCE(jsonb_agg(jsonb_build_array(rm.role_id,rm.menu_id) ORDER BY rm.role_id,rm.menu_id),'[]'::jsonb) \
          FROM role_menus rm JOIN roles r ON r.id=rm.role_id WHERE r.organization_id=$1))::text",
    ).bind(id).fetch_one(&mut **tx).await?;
    snapshot["memberCount"] = json!(members);
    snapshot["roleCount"] = json!(roles);
    snapshot["roleAssignmentCount"] = json!(assignments);
    snapshot["applicationCount"] = json!(applications);
    snapshot["menuCount"] = json!(menus);
    snapshot["subtreeOrganizationCount"] = json!(subtree_orgs);
    snapshot["subtreeMemberCount"] = json!(subtree_members);
    snapshot["subtreeRoleCount"] = json!(subtree_roles);
    snapshot["authorizationHash"] = json!(crate::util::hash::sha256_hex(&authorization));
    Ok((tenant_id, snapshot))
}

fn org_authorization_summary(snapshot: Value) -> Value {
    json!({
        "id": snapshot["id"], "tenantId": snapshot["tenantId"], "name": snapshot["name"],
        "applicationCount": snapshot["applicationCount"], "menuCount": snapshot["menuCount"],
        "authorizationHash": snapshot["authorizationHash"],
    })
}

fn org_json(r: &OrgRow) -> Value {
    json!({
        "id": r.id, "tenantId": r.tenant_id, "parentId": r.parent_id, "name": r.name, "code": r.code,
        "organizationType": r.organization_type,
        "contactName": r.contact_name, "contactPhone": r.contact_phone, "contactEmail": r.contact_email,
        "sortOrder": r.sort_order, "description": r.description, "status": r.status,
        "expiresAt": r.expires_at,
        "province": r.province, "city": r.city, "district": r.district,
        "township": r.township, "village": r.village, "address": r.address,
        "longitude": r.longitude, "latitude": r.latitude,
    })
}

/// 三态可空字段反序列化: 键缺省 → None(不更新); 显式 null → Some(None)(清除置空); 有值 → Some(Some(v))
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Deserialize::deserialize(de).map(Some)
}

/// DB 层兜底: 根组织唯一索引 uq_org_root_per_tenant 冲突 → 友好 409 (并发窗口下 check-then-insert 竞态由唯一索引封死)
fn map_root_unique_violation(e: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(ref db) = e {
        if db.code().as_deref() == Some("23505")
            && db.constraint() == Some("uq_org_root_per_tenant")
        {
            return AppError::conflict("该租户已存在根组织, 每个租户只能有一个根组织");
        }
    }
    e.into()
}

/// GET /api/organizations/tree?tenantId=
/// 节点额外携带 departmentCount / userCount 统计 (架构图视图使用)
async fn tree_handler(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:organization")
        .check(&state, &user)
        .await?;
    let tenant_id: Uuid = match params.get("tenantId") {
        Some(value) => value
            .parse()
            .map_err(|_| AppError::bad_request("tenantId 格式错误"))?,
        None if state.is_platform_context(&user).await? => {
            return Err(AppError::bad_request("缺少 tenantId 参数"))
        }
        None => user.tenant_id,
    };
    state.ensure_tenant_scope(&user, tenant_id, "租户").await?;
    let scope = state.scoped_org_ids(&user).await?;
    let rows: Vec<OrgRow> = sqlx::query_as(&format!(
        "{ORG_SELECT} FROM organizations o WHERE o.tenant_id = $1 \
         AND ($2::uuid[] IS NULL OR o.id = ANY($2)) ORDER BY o.sort_order, o.created_at"
    ))
    .bind(tenant_id)
    .bind(scope.as_deref())
    .fetch_all(&state.pool)
    .await?;

    // 统计每个组织的部门数与直属成员数
    let org_ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let mut dept_counts: HashMap<Uuid, i64> = HashMap::new();
    let mut member_counts: HashMap<Uuid, i64> = HashMap::new();
    if !org_ids.is_empty() {
        let dept_rows: Vec<(Uuid, i64)> =
            sqlx::query_as("SELECT organization_id, COUNT(*) FROM departments WHERE organization_id = ANY($1) GROUP BY organization_id")
                .bind(&org_ids)
                .fetch_all(&state.pool)
                .await?;
        dept_counts = dept_rows.into_iter().collect();
        let member_rows: Vec<(Uuid, i64)> =
            sqlx::query_as("SELECT organization_id, COUNT(*) FROM memberships WHERE organization_id = ANY($1) GROUP BY organization_id")
                .bind(&org_ids)
                .fetch_all(&state.pool)
                .await?;
        member_counts = member_rows.into_iter().collect();
    }

    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            let mut v = org_json(r);
            if scope.is_some() && r.id == user.org_id {
                v["parentId"] = Value::Null;
            }
            v["departmentCount"] = json!(dept_counts.get(&r.id).copied().unwrap_or(0));
            v["userCount"] = json!(member_counts.get(&r.id).copied().unwrap_or(0));
            v
        })
        .collect();
    Ok(Json(Value::Array(tree::build_tree(items))))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrgBody {
    #[serde(flatten)]
    contact: Contact,
    tenant_id: String,
    #[serde(default)]
    parent_id: Option<String>,
    name: String,
    #[serde(default = "default_organization_type")]
    organization_type: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    sort_order: i32,
    #[serde(default)]
    description: String,
    #[serde(default)]
    province: String,
    #[serde(default)]
    city: String,
    #[serde(default)]
    district: String,
    #[serde(default)]
    township: String,
    #[serde(default)]
    village: String,
    #[serde(default)]
    address: String,
    #[serde(default)]
    longitude: Option<f64>,
    #[serde(default)]
    latitude: Option<f64>,
    /// 过期时间: 缺省/null = 永久
    #[serde(default)]
    expires_at: Option<DateTime<Utc>>,
}

fn default_organization_type() -> String {
    "COMPANY".to_string()
}

fn validate_organization_type(organization_type: &str) -> Result<(), AppError> {
    if [
        "COMPANY",
        "SUBSIDIARY",
        "BRANCH",
        "STORE",
        "PUBLIC_INSTITUTION",
        "GOVERNMENT",
        "OTHER",
    ]
    .contains(&organization_type)
    {
        Ok(())
    } else {
        Err(AppError::bad_request(
            "组织类型仅支持公司、子公司、分公司、门店、事业单位、政府或其他",
        ))
    }
}

async fn parse_parent(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    parent: &Option<String>,
) -> Result<Option<Uuid>, AppError> {
    match parent {
        None => Ok(None),
        Some(p) if p.trim().is_empty() => Ok(None),
        Some(p) => {
            let pid: Uuid = p
                .parse()
                .map_err(|_| AppError::bad_request("parentId 格式错误"))?;
            let (parent_tenant,): (Uuid,) =
                sqlx::query_as("SELECT tenant_id FROM organizations WHERE id = $1")
                    .bind(pid)
                    .fetch_optional(pool)
                    .await?
                    .ok_or_else(|| AppError::not_found("父组织不存在"))?;
            if parent_tenant != tenant_id {
                return Err(AppError::not_found("父组织不存在"));
            }
            Ok(Some(pid))
        }
    }
}

/// POST /api/organizations
async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<OrgBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:organization")
        .check(&state, &user)
        .await?;
    validate_organization_type(&body.organization_type)?;
    let contact = body.contact.normalized();
    contact.validate(true)?;
    let existing_org_scope = state.scoped_org_ids(&user).await?;
    let allow_existing_global = existing_org_scope.is_none();
    let tenant_id: Uuid = body
        .tenant_id
        .parse()
        .map_err(|_| AppError::bad_request("tenantId 格式错误"))?;
    // 非平台租户只能在本租户下建组织
    state.ensure_tenant_scope(&user, tenant_id, "租户").await?;
    let tenant_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tenants WHERE id = $1)")
            .bind(tenant_id)
            .fetch_one(&state.pool)
            .await?;
    if !tenant_exists {
        return Err(AppError::bad_request("所属租户不存在"));
    }
    let parent_id = parse_parent(&state.pool, tenant_id, &body.parent_id).await?;
    if let Some(parent) = parent_id {
        state.ensure_org_scope(&user, parent, "父组织").await?;
    } else if !allow_existing_global {
        let current_is_root: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM organizations WHERE id=$1 AND tenant_id=$2 AND parent_id IS NULL)",
        ).bind(user.org_id).bind(tenant_id).fetch_one(&state.pool).await?;
        if !current_is_root {
            return Err(AppError::bad_request("请在当前组织或下级组织下创建组织"));
        }
    }
    // 根组织唯一: 每个租户仅允许一个 parent_id IS NULL 的组织 (应用层前置校验, DB 唯一索引兜底)
    if parent_id.is_none() {
        let root_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM organizations WHERE tenant_id = $1 AND parent_id IS NULL)",
        )
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await?;
        if root_exists {
            return Err(AppError::conflict(
                "该租户已存在根组织, 每个租户只能有一个根组织",
            ));
        }
    }
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::bad_request("组织名称不能为空"));
    }
    validate_coordinates(body.longitude, body.latitude)?;
    let mut tx = state.pool.begin().await?;
    let row: OrgRow = sqlx::query_as(
        "INSERT INTO organizations (tenant_id, parent_id, name, code, sort_order, description, expires_at, \
         province, city, district, township, village, address, longitude, latitude, organization_type, contact_name, contact_phone, contact_email) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19) \
         RETURNING id, tenant_id, parent_id, name, COALESCE(code,'') AS code, organization_type, contact_name, contact_phone, contact_email, sort_order, \
         COALESCE(description,'') AS description, status, expires_at, province, city, district, township, village, address, longitude, latitude",
    )
    .bind(tenant_id)
    .bind(parent_id)
    .bind(&name)
    .bind(&body.code)
    .bind(body.sort_order)
    .bind(&body.description)
    .bind(body.expires_at)
    .bind(body.province.trim())
    .bind(body.city.trim())
    .bind(body.district.trim())
    .bind(body.township.trim())
    .bind(body.village.trim())
    .bind(body.address.trim())
    .bind(body.longitude)
    .bind(body.latitude)
    .bind(&body.organization_type)
    .bind(&contact.contact_name)
    .bind(&contact.contact_phone)
    .bind(&contact.contact_email)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_root_unique_violation)?;
    onboarding::initialize_organization(&mut tx, tenant_id, row.id, parent_id).await?;
    let role_code = if parent_id.is_none() {
        "tenant_admin"
    } else {
        "org_admin"
    };
    let administrator = onboarding::provision_contact_administrator(
        &mut tx,
        (&state.config, &state.passwords),
        tenant_id,
        row.id,
        &contact,
        role_code,
        existing_org_scope.as_deref(),
    )
    .await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let (_, mut after) = org_audit_snapshot(&mut tx, row.id).await?;
    if let Some(administrator) = &administrator {
        after["administratorUserId"] = json!(administrator.user_id);
        after["administratorCreated"] = json!(administrator.created);
    }
    let detail = audit::change_detail("organization", row.id, Value::Null, after);
    audit::write_in_tx(
        &mut tx,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(tenant_id),
            organization_id: Some(row.id),
            action: "CREATE_ORGANIZATION",
            detail: &detail,
            ip: &ip,
            user_agent: &ua,
        },
    )
    .await?;
    tx.commit().await?;
    state.invalidate_perm_cache();
    let mut response = org_json(&row);
    if let Some(administrator) = administrator {
        response["administrator"] =
            serde_json::to_value(administrator).map_err(anyhow::Error::from)?;
    }
    Ok(Json(response))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrgUpdate {
    #[serde(flatten)]
    contact: ContactUpdate,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    organization_type: Option<String>,
    #[serde(default)]
    sort_order: Option<i32>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    province: Option<String>,
    #[serde(default)]
    city: Option<String>,
    #[serde(default)]
    district: Option<String>,
    #[serde(default)]
    township: Option<String>,
    #[serde(default)]
    village: Option<String>,
    #[serde(default)]
    address: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    longitude: Option<Option<f64>>,
    #[serde(default, deserialize_with = "double_option")]
    latitude: Option<Option<f64>>,
    /// 过期时间三态: 缺省不更新 / 显式 null 清除(永久) / 有值则设置
    /// (注: 本端点不支持修改 parent_id, 根组织归属仅由创建时决定)
    #[serde(default, deserialize_with = "double_option")]
    expires_at: Option<Option<DateTime<Utc>>>,
}

#[cfg(test)]
mod location_contract_tests {
    use super::{validate_organization_type, OrgBody, OrgUpdate};
    use serde_json::json;

    #[test]
    fn organization_creation_accepts_legacy_and_five_level_locations() {
        let mut payload = json!({
            "tenantId": "tenant-id", "name": "业务组织",
            "province": "浙江省", "city": "杭州市", "district": "西湖区",
        });
        let legacy: OrgBody = serde_json::from_value(payload.clone()).unwrap();
        assert_eq!(legacy.organization_type, "COMPANY");
        assert_eq!(
            (legacy.township.as_str(), legacy.village.as_str()),
            ("", "")
        );

        payload["township"] = json!("转塘街道");
        payload["village"] = json!("龙王沙社区");
        let full: OrgBody = serde_json::from_value(payload).unwrap();
        assert_eq!(full.township, "转塘街道");
        assert_eq!(full.village, "龙王沙社区");
    }

    #[test]
    fn organization_types_follow_the_contract() {
        for organization_type in [
            "COMPANY",
            "SUBSIDIARY",
            "BRANCH",
            "STORE",
            "PUBLIC_INSTITUTION",
            "GOVERNMENT",
            "OTHER",
        ] {
            assert!(validate_organization_type(organization_type).is_ok());
            let body: OrgBody = serde_json::from_value(json!({
                "tenantId": "tenant-id", "name": "业务组织", "organizationType": organization_type,
            }))
            .unwrap();
            assert_eq!(body.organization_type, organization_type);
        }
        for invalid in ["", "branch", "PERSONAL", "UNKNOWN"] {
            assert!(validate_organization_type(invalid).is_err());
        }
        let omitted: OrgUpdate = serde_json::from_value(json!({"name": "新名称"})).unwrap();
        assert_eq!(omitted.organization_type, None);
        let update: OrgUpdate =
            serde_json::from_value(json!({"organizationType": "BRANCH"})).unwrap();
        assert_eq!(update.organization_type.as_deref(), Some("BRANCH"));
    }

    #[test]
    fn organization_local_region_updates_distinguish_omission_from_clear() {
        let omitted: OrgUpdate = serde_json::from_value(json!({"name": "新名称"})).unwrap();
        assert_eq!(omitted.township, None);
        assert_eq!(omitted.village, None);
        let selected: OrgUpdate = serde_json::from_value(json!({
            "township": "转塘街道", "village": "龙王沙社区",
        }))
        .unwrap();
        assert_eq!(selected.township.as_deref(), Some("转塘街道"));
        assert_eq!(selected.village.as_deref(), Some("龙王沙社区"));
        let cleared: OrgUpdate =
            serde_json::from_value(json!({"township": "", "village": ""})).unwrap();
        assert_eq!(cleared.township.as_deref(), Some(""));
        assert_eq!(cleared.village.as_deref(), Some(""));
    }
}

/// PUT /api/organizations/{id}
async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<OrgUpdate>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:organization")
        .check(&state, &user)
        .await?;
    let id: Uuid = id
        .parse()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_org_in_scope(&state, &user, id).await?;
    if let Some(name) = &body.name {
        if name.trim().is_empty() {
            return Err(AppError::bad_request("组织名称不能为空"));
        }
    }
    if let Some(organization_type) = &body.organization_type {
        validate_organization_type(organization_type)?;
    }
    let contact = body.contact.normalized()?;
    let mut tx = state.pool.begin().await?;
    let (_, before) = org_configuration_snapshot(&mut tx, id).await?;
    if body.expires_at.flatten().is_some() && is_platform_root_in_tx(&mut tx, id).await? {
        return Err(AppError::bad_request("平台根组织不允许设置过期时间"));
    }
    let current_coordinates: (Option<f64>, Option<f64>) =
        sqlx::query_as("SELECT longitude, latitude FROM organizations WHERE id = $1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    validate_coordinates(
        body.longitude.unwrap_or(current_coordinates.0),
        body.latitude.unwrap_or(current_coordinates.1),
    )?;
    let row: OrgRow = sqlx::query_as(
        "UPDATE organizations SET name = COALESCE($2, name), code = COALESCE($3, code), \
         sort_order = COALESCE($4, sort_order), description = COALESCE($5, description), \
         expires_at = CASE WHEN $6 THEN $7::timestamptz ELSE expires_at END, \
         province = COALESCE($8, province), city = COALESCE($9, city), district = COALESCE($10, district), \
         township = COALESCE($11, township), village = COALESCE($12, village), address = COALESCE($13, address), \
         longitude = CASE WHEN $14 THEN $15::double precision ELSE longitude END, \
         latitude = CASE WHEN $16 THEN $17::double precision ELSE latitude END, \
         organization_type = COALESCE($18, organization_type), contact_name = COALESCE($19, contact_name), \
         contact_phone = COALESCE($20, contact_phone), contact_email = COALESCE($21, contact_email), updated_at = now() \
         WHERE id = $1 \
         RETURNING id, tenant_id, parent_id, name, COALESCE(code,'') AS code, organization_type, contact_name, contact_phone, contact_email, sort_order, \
         COALESCE(description,'') AS description, status, expires_at, province, city, district, township, village, address, longitude, latitude",
    )
    .bind(id)
    .bind(&body.name)
    .bind(&body.code)
    .bind(body.sort_order)
    .bind(&body.description)
    .bind(body.expires_at.is_some())
    .bind(body.expires_at.flatten())
    .bind(body.province.as_deref().map(str::trim))
    .bind(body.city.as_deref().map(str::trim))
    .bind(body.district.as_deref().map(str::trim))
    .bind(body.township.as_deref().map(str::trim))
    .bind(body.village.as_deref().map(str::trim))
    .bind(body.address.as_deref().map(str::trim))
    .bind(body.longitude.is_some())
    .bind(body.longitude.flatten())
    .bind(body.latitude.is_some())
    .bind(body.latitude.flatten())
    .bind(&body.organization_type)
    .bind(&contact.contact_name)
    .bind(&contact.contact_phone)
    .bind(&contact.contact_email)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("组织不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let (_, after) = org_configuration_snapshot(&mut tx, id).await?;
    if before != after {
        let detail = audit::change_detail("organization", id, before, after);
        audit::write_in_tx(
            &mut tx,
            audit::AuditInfo {
                user_id: Some(user.user_id),
                username: &user.username,
                tenant_id: Some(row.tenant_id),
                organization_id: Some(id),
                action: "UPDATE_ORGANIZATION",
                detail: &detail,
                ip: &ip,
                user_agent: &ua,
            },
        )
        .await?;
    }
    tx.commit().await?;
    state.invalidate_perm_cache();
    Ok(Json(org_json(&row)))
}

/// DELETE /api/organizations/{id} — 级联子组织/部门/成员/授权/角色
async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:organization")
        .check(&state, &user)
        .await?;
    let id: Uuid = id
        .parse()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_org_in_scope(&state, &user, id).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if is_platform_root_in_tx(&mut tx, id).await? {
        return Err(AppError::bad_request("平台根组织不允许删除"));
    }
    let (target_tenant, before) = org_audit_snapshot(&mut tx, id).await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let detail = audit::change_detail("organization", id, before, Value::Null);
    // The insert must see the target to preserve organization_name on deletion.
    audit::write_in_tx(
        &mut tx,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(target_tenant),
            organization_id: Some(id),
            action: "DELETE_ORGANIZATION",
            detail: &detail,
            ip: &ip,
            user_agent: &ua,
        },
    )
    .await?;
    // These composite NO ACTION checks must see the completed graph cascade,
    // after the paired single-column CASCADE FKs remove children and roles.
    sqlx::query("SET CONSTRAINTS fk_organizations_parent_same_tenant, fk_roles_organization_same_tenant DEFERRED")
        .execute(&mut *tx).await?;
    sqlx::query("DELETE FROM organizations WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET CONSTRAINTS fk_organizations_parent_same_tenant, fk_roles_organization_same_tenant IMMEDIATE")
        .execute(&mut *tx).await?;
    tx.commit().await?;
    state.invalidate_perm_cache();
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct StatusBody {
    status: String,
}

/// PUT /api/organizations/{id}/status
async fn update_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<StatusBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:organization")
        .check(&state, &user)
        .await?;
    if !["ENABLED", "DISABLED"].contains(&body.status.as_str()) {
        return Err(AppError::bad_request("状态仅支持 ENABLED / DISABLED"));
    }
    let id: Uuid = id
        .parse()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_org_in_scope(&state, &user, id).await?;
    let mut tx = state.pool.begin().await?;
    let (target_tenant, before) = org_configuration_snapshot(&mut tx, id).await?;
    if body.status == "DISABLED" && is_platform_root_in_tx(&mut tx, id).await? {
        return Err(AppError::bad_request("平台根组织不允许停用"));
    }
    sqlx::query("UPDATE organizations SET status = $2, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(&body.status)
        .execute(&mut *tx)
        .await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let (_, after) = org_configuration_snapshot(&mut tx, id).await?;
    if before != after {
        let detail = audit::change_detail("organization", id, before, after);
        audit::write_in_tx(
            &mut tx,
            audit::AuditInfo {
                user_id: Some(user.user_id),
                username: &user.username,
                tenant_id: Some(target_tenant),
                organization_id: Some(id),
                action: "UPDATE_ORGANIZATION",
                detail: &detail,
                ip: &ip,
                user_agent: &ua,
            },
        )
        .await?;
    }
    tx.commit().await?;
    state.invalidate_perm_cache();
    Ok(Json(json!({"ok": true})))
}

// ===== 组织授权 =====

#[derive(sqlx::FromRow)]
#[allow(dead_code)]
struct AppAuthRow {
    id: Uuid,
    code: String,
    name: String,
    icon: String,
    authorized: bool,
    menu_count: i64,
    authorized_menu_count: i64,
}

#[derive(sqlx::FromRow)]
struct MenuFlatRow {
    id: Uuid,
    parent_id: Option<Uuid>,
    r#type: String,
    title: String,
    path: String,
    icon: String,
    permission: String,
    link_url: String,
    link_target: String,
    sort_order: i32,
    visible: bool,
    authorized: bool,
}

async fn load_org_tenant(pool: &sqlx::PgPool, org_id: Uuid) -> Result<Uuid, AppError> {
    let row: Option<(Uuid,)> = sqlx::query_as("SELECT tenant_id FROM organizations WHERE id = $1")
        .bind(org_id)
        .fetch_optional(pool)
        .await?;
    row.map(|(t,)| t)
        .ok_or_else(|| AppError::not_found("组织不存在"))
}

/// 当前组织子树的目标组织隔离校验。
async fn ensure_org_in_scope(
    state: &AppState,
    user: &CurrentUser,
    org_id: Uuid,
) -> Result<(), AppError> {
    state.ensure_org_scope(user, org_id, "组织").await
}

/// GET /api/organizations/{id}/authorizations — 仅展示租户已授权范围
async fn get_authorizations(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:authorization")
        .check(&state, &user)
        .await?;
    let org_id: Uuid = id
        .parse()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_org_in_scope(&state, &user, org_id).await?;
    let tenant_id = load_org_tenant(&state.pool, org_id).await?;

    let apps: Vec<AppAuthRow> = sqlx::query_as(
        "SELECT a.id, a.code, a.name, COALESCE(a.icon,'') AS icon, \
         EXISTS(SELECT 1 FROM organization_applications oa WHERE oa.organization_id = $1 AND oa.application_id = a.id) AS authorized, \
         (SELECT count(*) FROM menus m JOIN tenant_menus tm ON tm.menu_id = m.id WHERE tm.tenant_id = $2 AND m.application_id = a.id) AS menu_count, \
         (SELECT count(*) FROM organization_menus om JOIN tenant_menus tm ON tm.menu_id = om.menu_id \
          WHERE om.organization_id = $1 AND tm.tenant_id = $2 AND om.menu_id IN (SELECT id FROM menus WHERE application_id = a.id)) AS authorized_menu_count \
         FROM applications a \
         JOIN tenant_applications ta ON ta.application_id = a.id AND ta.tenant_id = $2 \
         ORDER BY a.sort_order, a.created_at",
    )
    .bind(org_id)
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let mut menus_map = serde_json::Map::new();
    for app in &apps {
        let rows: Vec<MenuFlatRow> = sqlx::query_as(
            "SELECT m.id, m.parent_id, m.type, m.title, COALESCE(m.path,'') AS path, COALESCE(m.icon,'') AS icon, \
             COALESCE(m.permission,'') AS permission, m.link_url, m.link_target, m.sort_order, m.visible, \
             EXISTS(SELECT 1 FROM organization_menus om WHERE om.organization_id = $1 AND om.menu_id = m.id) AS authorized \
             FROM menus m \
             JOIN tenant_menus tm ON tm.menu_id = m.id AND tm.tenant_id = $2 \
             WHERE m.application_id = $3 ORDER BY m.sort_order, m.created_at",
        )
        .bind(org_id)
        .bind(tenant_id)
        .bind(app.id)
        .fetch_all(&state.pool)
        .await?;
        menus_map.insert(
            app.id.to_string(),
            Value::Array(
                rows.iter()
                    .map(|m| {
                        json!({
                            "id": m.id, "parentId": m.parent_id, "type": m.r#type, "title": m.title,
                            "path": m.path, "icon": m.icon, "permission": m.permission,
                            "sortOrder": m.sort_order, "visible": m.visible, "authorized": m.authorized,
                            "linkUrl": m.link_url, "linkTarget": m.link_target,
                        })
                    })
                    .collect(),
            ),
        );
    }
    let apps_json: Vec<Value> = apps
        .iter()
        .map(|a| {
            json!({
                "id": a.id, "code": a.code, "name": a.name, "icon": a.icon, "authorized": a.authorized,
                "menuCount": a.menu_count, "authorizedMenuCount": a.authorized_menu_count,
            })
        })
        .collect();
    Ok(Json(json!({
        "applications": apps_json,
        "menus": Value::Object(menus_map),
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthzBody {
    #[serde(default)]
    applications: Vec<AuthzItemApp>,
    #[serde(default)]
    menus: Vec<AuthzItemMenu>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthzItemApp {
    application_id: String,
    authorized: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthzItemMenu {
    menu_id: String,
    authorized: bool,
}

/// PUT /api/organizations/{id}/authorizations — 写入校验 ⊆ 租户授权; 取消时级联清理本组织角色菜单
async fn put_authorizations(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<AuthzBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:authorization")
        .check(&state, &user)
        .await?;
    let org_id: Uuid = id
        .parse()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_org_in_scope(&state, &user, org_id).await?;
    let tenant_id = load_org_tenant(&state.pool, org_id).await?;

    let app_ids: Vec<Uuid> = body
        .applications
        .iter()
        .map(|i| i.application_id.parse::<Uuid>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AppError::bad_request("applicationId 格式错误"))?;
    let menu_ids: Vec<Uuid> = body
        .menus
        .iter()
        .map(|i| i.menu_id.parse::<Uuid>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AppError::bad_request("menuId 格式错误"))?;

    let mut tx = state.pool.begin().await?;
    // Tenant revocation and this grant edit take the same parent-then-child
    // locks; recheck the parent's allowed sets only after acquiring them.
    sqlx::query("SELECT id FROM tenants WHERE id=$1 FOR NO KEY UPDATE")
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;
    let (_, before) = org_audit_snapshot(&mut tx, org_id).await?;
    let before = org_authorization_summary(before);

    // 校验: 应用/菜单必须已在租户授权范围内
    if !app_ids.is_empty() {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM tenant_applications WHERE tenant_id = $1 AND application_id = ANY($2)",
        )
        .bind(tenant_id)
        .bind(&app_ids)
        .fetch_one(&mut *tx)
        .await?;
        if n != app_ids.len() as i64 {
            return Err(AppError::bad_request("应用超出租户授权范围"));
        }
    }
    if !menu_ids.is_empty() {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM tenant_menus WHERE tenant_id = $1 AND menu_id = ANY($2)",
        )
        .bind(tenant_id)
        .bind(&menu_ids)
        .fetch_one(&mut *tx)
        .await?;
        if n != menu_ids.len() as i64 {
            return Err(AppError::bad_request("菜单超出租户授权范围"));
        }
    }

    for (item, app) in body.applications.iter().zip(app_ids.iter()) {
        if item.authorized {
            sqlx::query("INSERT INTO organization_applications (organization_id, application_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(org_id)
                .bind(app)
                .execute(&mut *tx)
                .await?;
        } else {
            // 取消应用: 清理本组织菜单授权 + 本组织角色的该应用菜单
            sqlx::query(
                "DELETE FROM organization_menus om USING menus m \
                 WHERE om.organization_id = $1 AND om.menu_id = m.id AND m.application_id = $2",
            )
            .bind(org_id)
            .bind(app)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "DELETE FROM role_menus rm USING roles r, menus m \
                 WHERE rm.role_id = r.id AND r.organization_id = $1 AND rm.menu_id = m.id AND m.application_id = $2",
            )
            .bind(org_id)
            .bind(app)
            .execute(&mut *tx)
            .await?;
            sqlx::query("DELETE FROM organization_applications WHERE organization_id = $1 AND application_id = $2")
                .bind(org_id)
                .bind(app)
                .execute(&mut *tx)
                .await?;
        }
    }
    for (item, menu) in body.menus.iter().zip(menu_ids.iter()) {
        if item.authorized {
            sqlx::query("INSERT INTO organization_menus (organization_id, menu_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(org_id)
                .bind(menu)
                .execute(&mut *tx)
                .await?;
        } else {
            // 取消菜单: 清理本组织角色的该菜单
            sqlx::query(
                "DELETE FROM role_menus rm USING roles r WHERE rm.role_id = r.id AND r.organization_id = $1 AND rm.menu_id = $2",
            )
            .bind(org_id)
            .bind(menu)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "DELETE FROM organization_menus WHERE organization_id = $1 AND menu_id = $2",
            )
            .bind(org_id)
            .bind(menu)
            .execute(&mut *tx)
            .await?;
        }
    }
    // 兜底对账: 组织菜单 ⊆ 组织应用 ∩ 租户菜单; 角色菜单 ⊆ 组织菜单 (仅本组织角色)
    sqlx::query(
        "DELETE FROM organization_menus om USING menus m \
         WHERE om.organization_id = $1 AND om.menu_id = m.id \
         AND NOT EXISTS (SELECT 1 FROM organization_applications oa WHERE oa.organization_id = $1 AND oa.application_id = m.application_id)",
    )
    .bind(org_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM role_menus rm USING roles r WHERE rm.role_id = r.id AND r.organization_id = $1 \
         AND NOT EXISTS (SELECT 1 FROM organization_menus om WHERE om.organization_id = $1 AND om.menu_id = rm.menu_id)",
    )
    .bind(org_id)
    .execute(&mut *tx)
    .await?;
    onboarding::sync_org_admin(&mut tx, org_id).await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let (_, after) = org_audit_snapshot(&mut tx, org_id).await?;
    let after = org_authorization_summary(after);
    if before != after {
        let detail = audit::change_detail("organization", org_id, before, after);
        audit::write_in_tx(
            &mut tx,
            audit::AuditInfo {
                user_id: Some(user.user_id),
                username: &user.username,
                tenant_id: Some(tenant_id),
                organization_id: Some(org_id),
                action: "ORG_AUTH",
                detail: &detail,
                ip: &ip,
                user_agent: &ua,
            },
        )
        .await?;
    }
    tx.commit().await?;
    state.invalidate_perm_cache();
    Ok(Json(json!({"ok": true})))
}

async fn initialize_administrator(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:organization")
        .check(&state, &user)
        .await?;
    RequirePermission("system:user")
        .check(&state, &user)
        .await?;
    RequirePermission("system:role")
        .check(&state, &user)
        .await?;
    let id: Uuid = id
        .parse()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    ensure_org_in_scope(&state, &user, id).await?;
    let existing_org_scope = state.scoped_org_ids(&user).await?;
    let mut tx = state.pool.begin().await?;
    let (_, before) = org_audit_snapshot(&mut tx, id).await?;
    let (tenant_id, parent_id, contact_name, contact_phone, contact_email): (Uuid, Option<Uuid>, String, String, String) = sqlx::query_as(
        "SELECT tenant_id,parent_id,contact_name,contact_phone,contact_email FROM organizations WHERE id=$1 FOR NO KEY UPDATE",
    ).bind(id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::not_found("组织不存在"))?;
    let org_id = id;
    let role_code = if parent_id.is_none() {
        "tenant_admin"
    } else {
        "org_admin"
    };
    let contact = Contact {
        contact_name,
        contact_phone,
        contact_email,
    }
    .normalized();
    let administrator = onboarding::initialize_existing_administrator(
        &mut tx,
        (&state.config, &state.passwords),
        tenant_id,
        org_id,
        &contact,
        role_code,
        existing_org_scope.as_deref(),
    )
    .await?;
    if !administrator.already_initialized {
        let ip = audit::client_ip(&headers);
        let ua = audit::ua_of(&headers);
        let (_, mut after) = org_audit_snapshot(&mut tx, id).await?;
        after["administratorUserId"] = json!(administrator.user_id);
        after["administratorCreated"] = json!(administrator.created);
        let detail = audit::change_detail("organization", id, before, after);
        audit::write_in_tx(
            &mut tx,
            audit::AuditInfo {
                user_id: Some(user.user_id),
                username: &user.username,
                tenant_id: Some(tenant_id),
                organization_id: Some(org_id),
                action: "INITIALIZE_ADMINISTRATOR",
                detail: &detail,
                ip: &ip,
                user_agent: &ua,
            },
        )
        .await?;
    }
    tx.commit().await?;
    state.invalidate_perm_cache();
    Ok(Json(json!({"administrator": administrator})))
}
