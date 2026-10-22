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
use crate::routes::{page_json, paging};
use crate::state::AppState;
use crate::util::audit;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/tenants", get(list).post(create))
        .route("/api/tenants/options", get(options))
        .route("/api/tenants/{id}", put(update).delete(delete))
        .route("/api/tenants/{id}/status", put(update_status))
        .route(
            "/api/tenants/{id}/initialize-administrator",
            post(initialize_administrator),
        )
        .route(
            "/api/tenants/{id}/authorizations",
            get(get_authorizations).put(put_authorizations),
        )
}

/// Minimal tenant choices for authorized tenant-scoped management pages.
/// An ordinary context can only select its own tenant, including global administrators.
async fn options(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Result<Json<Value>, AppError> {
    let platform = state.is_platform_context(&user).await?;
    let permissions = state.context_permissions(&user).await?;
    if ![
        "system:tenant",
        "system:organization",
        "system:department",
        "system:user",
        "system:role",
        "system:authorization",
        "system:audit",
    ]
    .iter()
    .any(|permission| permissions.contains(*permission))
    {
        return Err(AppError::forbidden("无租户选择权限"));
    }
    let scope = if platform { None } else { Some(user.tenant_id) };
    let rows: Vec<(Uuid, String, String, bool)> = sqlx::query_as(
        "SELECT id,name,COALESCE(code,''),is_platform FROM tenants \
         WHERE ($1::uuid IS NULL OR id=$1) ORDER BY created_at,id",
    )
    .bind(scope)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!(rows
        .into_iter()
        .map(|(id, name, code, is_platform)| {
            json!({"id":id,"name":name,"code":code,"isPlatform":is_platform})
        })
        .collect::<Vec<_>>())))
}

const LIST_SELECT: &str = "SELECT t.id, t.name, t.code, t.tenant_type, t.contact_name, t.contact_phone, t.contact_email, COALESCE(t.description,'') AS description, t.status, t.is_platform, t.created_at, t.expires_at, \
 t.province, t.city, t.district, t.township, t.village, t.address, t.longitude, t.latitude, \
 (SELECT count(*) FROM organizations o WHERE o.tenant_id = t.id AND ($6::uuid[] IS NULL OR o.id = ANY($6))) AS organization_count, \
 (SELECT count(DISTINCT m.user_id) FROM memberships m JOIN organizations o ON o.id = m.organization_id \
  WHERE o.tenant_id = t.id AND ($6::uuid[] IS NULL OR o.id = ANY($6))) AS user_count, \
 (SELECT count(*) FROM tenant_applications ta WHERE ta.tenant_id = t.id \
  AND ($6::uuid[] IS NULL OR EXISTS (SELECT 1 FROM organization_applications oa \
       WHERE oa.organization_id = $7 AND oa.application_id = ta.application_id))) AS authorized_app_count";

#[derive(sqlx::FromRow, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TenantRow {
    id: Uuid,
    name: String,
    code: String,
    tenant_type: String,
    contact_name: String,
    contact_phone: String,
    contact_email: String,
    description: String,
    status: String,
    is_platform: bool,
    created_at: DateTime<Utc>,
    /// 过期时间: null = 永久
    expires_at: Option<DateTime<Utc>>,
    province: String,
    city: String,
    district: String,
    township: String,
    village: String,
    address: String,
    longitude: Option<f64>,
    latitude: Option<f64>,
    organization_count: i64,
    user_count: i64,
    authorized_app_count: i64,
}

/// Only fields protected by this row lock participate in ordinary edit/no-op
/// decisions; independent role/member edits must not become this request's event.
async fn tenant_configuration_snapshot(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
) -> Result<Value, AppError> {
    let (name, code, tenant_type, status, is_platform, expires_at):
        (String, String, String, String, bool, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT name,code,tenant_type,status,is_platform,expires_at FROM tenants WHERE id=$1 FOR NO KEY UPDATE",
    ).bind(id).fetch_optional(&mut **tx).await?
        .ok_or_else(|| AppError::not_found("租户不存在"))?;
    // Hash explicit editable fields so contact/body-only updates remain visible
    // without persisting their contents or treating updated_at as a change.
    let configuration: String = sqlx::query_scalar(
        "SELECT jsonb_build_array(name,code,tenant_type,status,expires_at,contact_name,contact_phone,contact_email, \
         description,province,city,district,township,village,address,longitude,latitude)::text FROM tenants WHERE id=$1",
    ).bind(id).fetch_one(&mut **tx).await?;
    Ok(json!({
        "id": id, "name": name, "code": code, "tenantType": tenant_type,
        "status": status, "isPlatform": is_platform, "expiresAt": expires_at,
        "configurationHash": crate::util::hash::sha256_hex(&configuration),
    }))
}

async fn tenant_audit_snapshot(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
) -> Result<Value, AppError> {
    let mut snapshot = tenant_configuration_snapshot(tx, id).await?;
    let (organizations, members, roles, role_assignments, applications, menus):
        (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM organizations WHERE tenant_id=$1), \
         (SELECT count(*) FROM memberships m JOIN organizations o ON o.id=m.organization_id WHERE o.tenant_id=$1), \
         (SELECT count(*) FROM roles WHERE tenant_id=$1), \
         (SELECT count(*) FROM user_roles ur JOIN roles r ON r.id=ur.role_id WHERE r.tenant_id=$1), \
         (SELECT count(*) FROM tenant_applications WHERE tenant_id=$1), \
         (SELECT count(*) FROM tenant_menus WHERE tenant_id=$1)",
    ).bind(id).fetch_one(&mut **tx).await?;
    let authorization: String = sqlx::query_scalar(
        "SELECT jsonb_build_array( \
         (SELECT COALESCE(jsonb_agg(application_id ORDER BY application_id),'[]'::jsonb) FROM tenant_applications WHERE tenant_id=$1), \
         (SELECT COALESCE(jsonb_agg(menu_id ORDER BY menu_id),'[]'::jsonb) FROM tenant_menus WHERE tenant_id=$1), \
         (SELECT COALESCE(jsonb_agg(jsonb_build_array(oa.organization_id,oa.application_id) ORDER BY oa.organization_id,oa.application_id),'[]'::jsonb) \
          FROM organization_applications oa JOIN organizations o ON o.id=oa.organization_id WHERE o.tenant_id=$1), \
         (SELECT COALESCE(jsonb_agg(jsonb_build_array(om.organization_id,om.menu_id) ORDER BY om.organization_id,om.menu_id),'[]'::jsonb) \
          FROM organization_menus om JOIN organizations o ON o.id=om.organization_id WHERE o.tenant_id=$1), \
         (SELECT COALESCE(jsonb_agg(jsonb_build_array(rm.role_id,rm.menu_id) ORDER BY rm.role_id,rm.menu_id),'[]'::jsonb) \
          FROM role_menus rm JOIN roles r ON r.id=rm.role_id WHERE r.tenant_id=$1))::text",
    ).bind(id).fetch_one(&mut **tx).await?;
    snapshot["organizationCount"] = json!(organizations);
    snapshot["memberCount"] = json!(members);
    snapshot["roleCount"] = json!(roles);
    snapshot["roleAssignmentCount"] = json!(role_assignments);
    snapshot["applicationCount"] = json!(applications);
    snapshot["menuCount"] = json!(menus);
    snapshot["authorizationHash"] = json!(crate::util::hash::sha256_hex(&authorization));
    Ok(snapshot)
}

fn tenant_authorization_summary(snapshot: Value) -> Value {
    json!({
        "id": snapshot["id"], "name": snapshot["name"],
        "applicationCount": snapshot["applicationCount"], "menuCount": snapshot["menuCount"],
        "authorizationHash": snapshot["authorizationHash"],
    })
}

/// GET /api/tenants — 分页; 非平台租户仅能看到自己的租户 (只读视角)
/// 所有上下文均需 system:tenant；写操作另需平台上下文。
async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppError> {
    let platform = state.is_platform_context(&user).await?;
    RequirePermission("system:tenant")
        .check(&state, &user)
        .await?;
    let (page, size, offset) = paging(&params);
    let kw = params.get("keyword").cloned().unwrap_or_default();
    let st = params.get("status").cloned().unwrap_or_default();
    // 平台根看全部；其余上下文仅看本租户，统计值限制在当前组织子树。
    let scope: Option<Uuid> = if platform { None } else { Some(user.tenant_id) };
    let org_scope = state.scoped_org_ids(&user).await?;
    let rows: Vec<TenantRow> = sqlx::query_as(&format!(
        "{LIST_SELECT} FROM tenants t \
         WHERE ($1 = '' OR t.name ILIKE '%' || $1 || '%' OR t.code ILIKE '%' || $1 || '%') \
         AND ($4 = '' OR t.status = $4) \
         AND ($5::uuid IS NULL OR t.id = $5) \
         ORDER BY t.created_at LIMIT $2 OFFSET $3"
    ))
    .bind(&kw)
    .bind(size)
    .bind(offset)
    .bind(&st)
    .bind(scope)
    .bind(&org_scope)
    .bind(user.org_id)
    .fetch_all(&state.pool)
    .await?;
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tenants t WHERE ($1 = '' OR t.name ILIKE '%' || $1 || '%' OR t.code ILIKE '%' || $1 || '%') \
         AND ($2 = '' OR t.status = $2) AND ($3::uuid IS NULL OR t.id = $3)",
    )
    .bind(&kw)
    .bind(&st)
    .bind(scope)
    .fetch_one(&state.pool)
    .await?;
    let records: Vec<Value> = rows
        .iter()
        .map(|r| serde_json::to_value(r).unwrap_or_default())
        .collect();
    Ok(Json(page_json(records, total, page, size)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TenantBody {
    #[serde(flatten)]
    contact: Contact,
    name: String,
    code: String,
    #[serde(default = "default_tenant_type")]
    tenant_type: String,
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

fn default_tenant_type() -> String {
    "COMPANY".to_string()
}

fn validate_tenant_type(tenant_type: &str) -> Result<(), AppError> {
    if [
        "PERSONAL",
        "COMPANY",
        "PUBLIC_INSTITUTION",
        "GOVERNMENT",
        "OTHER",
    ]
    .contains(&tenant_type)
    {
        Ok(())
    } else {
        Err(AppError::bad_request(
            "租户类型仅支持个人、公司、事业单位、政府或其他",
        ))
    }
}

fn root_organization_type(tenant_type: &str) -> &str {
    if tenant_type == "PERSONAL" {
        "OTHER"
    } else {
        tenant_type
    }
}

/// POST /api/tenants
async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<TenantBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:tenant")
        .check(&state, &user)
        .await?;
    state.require_platform(&user).await?;
    let name = body.name.trim().to_string();
    if name.is_empty() || body.code.trim().is_empty() {
        return Err(AppError::bad_request("租户名称与编码不能为空"));
    }
    validate_tenant_identity(&name, body.code.trim(), false)?;
    validate_tenant_type(&body.tenant_type)?;
    let contact = body.contact.normalized();
    contact.validate(true)?;
    validate_coordinates(body.longitude, body.latitude)?;
    let mut tx = state.pool.begin().await?;
    let mut row: TenantRow = sqlx::query_as(
        "INSERT INTO tenants (name, code, description, expires_at, province, city, district, township, village, address, longitude, latitude, tenant_type, contact_name, contact_phone, contact_email) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16) RETURNING id, name, code, tenant_type, contact_name, contact_phone, contact_email, \
         description, status, false AS is_platform, created_at, expires_at, province, city, district, township, village, address, longitude, latitude, \
         0::bigint AS organization_count, 0::bigint AS user_count, 0::bigint AS authorized_app_count"
    )
    .bind(&name)
    .bind(body.code.trim())
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
    .bind(&body.tenant_type)
    .bind(&contact.contact_name)
    .bind(&contact.contact_phone)
    .bind(&contact.contact_email)
    .fetch_one(&mut *tx)
    .await?;
    let root_id: Uuid = sqlx::query_scalar(
        "INSERT INTO organizations (tenant_id, name, code, province, city, district, township, village, address, longitude, latitude, organization_type, contact_name, contact_phone, contact_email) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15) RETURNING id",
    ).bind(row.id).bind(&name).bind(body.code.trim())
        .bind(&row.province).bind(&row.city).bind(&row.district)
        .bind(&row.township).bind(&row.village).bind(&row.address)
        .bind(row.longitude).bind(row.latitude)
        .bind(root_organization_type(&body.tenant_type))
        .bind(&contact.contact_name).bind(&contact.contact_phone).bind(&contact.contact_email)
        .fetch_one(&mut *tx).await?;
    onboarding::initialize_tenant(&mut tx, row.id, root_id).await?;
    let administrator = onboarding::provision_contact_administrator(
        &mut tx,
        (&state.config, &state.passwords),
        row.id,
        root_id,
        &contact,
        "tenant_admin",
        None,
    )
    .await?;
    row.organization_count = 1;
    row.user_count = i64::from(administrator.is_some());
    row.authorized_app_count = 1;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let mut after = tenant_audit_snapshot(&mut tx, row.id).await?;
    after["rootOrganizationId"] = json!(root_id);
    if let Some(administrator) = &administrator {
        after["administratorUserId"] = json!(administrator.user_id);
        after["administratorCreated"] = json!(administrator.created);
    }
    let detail = audit::change_detail("tenant", row.id, Value::Null, after);
    audit::write_in_tx(
        &mut tx,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(row.id),
            organization_id: Some(user.org_id),
            action: "CREATE_TENANT",
            detail: &detail,
            ip: &ip,
            user_agent: &ua,
        },
    )
    .await?;
    tx.commit().await?;
    state.invalidate_perm_cache();
    let mut response = serde_json::to_value(&row).map_err(anyhow::Error::from)?;
    if let Some(administrator) = administrator {
        response["administrator"] =
            serde_json::to_value(administrator).map_err(anyhow::Error::from)?;
    }
    Ok(Json(response))
}

/// 三态可空字段反序列化: 键缺省 → None(不更新); 显式 null → Some(None)(清除置空); 有值 → Some(Some(v))
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Deserialize::deserialize(de).map(Some)
}

pub(super) fn validate_coordinates(
    longitude: Option<f64>,
    latitude: Option<f64>,
) -> Result<(), AppError> {
    match (longitude, latitude) {
        (None, None) => Ok(()),
        (Some(lon), Some(lat))
            if lon.is_finite()
                && lat.is_finite()
                && (-180.0..=180.0).contains(&lon)
                && (-90.0..=90.0).contains(&lat) =>
        {
            Ok(())
        }
        _ => Err(AppError::bad_request(
            "经纬度需同时填写；经度范围 -180~180，纬度范围 -90~90",
        )),
    }
}

fn validate_tenant_identity(name: &str, code: &str, is_platform: bool) -> Result<(), AppError> {
    if name.trim().is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
        return Err(AppError::bad_request(
            "租户名称需为 1~100 个字符，且不能包含控制字符",
        ));
    }
    if code.trim().is_empty() || code.chars().count() > 50 || code.chars().any(char::is_control) {
        return Err(AppError::bad_request(
            "租户编码需为 1~50 个字符，且不能包含控制字符",
        ));
    }
    if is_platform && code != "admin-tenant" {
        return Err(AppError::bad_request("平台租户编码固定为 admin-tenant"));
    }
    if !is_platform && code == "admin-tenant" {
        return Err(AppError::bad_request("admin-tenant 为平台租户专用编码"));
    }
    Ok(())
}

#[cfg(test)]
mod coordinate_tests {
    use super::{
        root_organization_type, validate_coordinates, validate_tenant_identity,
        validate_tenant_type, TenantBody, TenantUpdate,
    };
    use serde_json::json;

    #[test]
    fn tenant_creation_accepts_legacy_and_five_level_locations() {
        let mut payload = json!({
            "name": "业务租户", "code": "business",
            "province": "浙江省", "city": "杭州市", "district": "西湖区",
        });
        let legacy: TenantBody = serde_json::from_value(payload.clone()).unwrap();
        assert_eq!(legacy.tenant_type, "COMPANY");
        assert_eq!(
            (legacy.township.as_str(), legacy.village.as_str()),
            ("", "")
        );

        payload["township"] = json!("转塘街道");
        payload["village"] = json!("龙王沙社区");
        let full: TenantBody = serde_json::from_value(payload).unwrap();
        assert_eq!(full.township, "转塘街道");
        assert_eq!(full.village, "龙王沙社区");
    }

    #[test]
    fn tenant_types_and_root_types_follow_the_contract() {
        for tenant_type in [
            "PERSONAL",
            "COMPANY",
            "PUBLIC_INSTITUTION",
            "GOVERNMENT",
            "OTHER",
        ] {
            assert!(validate_tenant_type(tenant_type).is_ok());
            let body: TenantBody = serde_json::from_value(json!({
                "name": "业务租户", "code": "business", "tenantType": tenant_type,
            }))
            .unwrap();
            assert_eq!(body.tenant_type, tenant_type);
            let expected_root_type = if tenant_type == "PERSONAL" {
                "OTHER"
            } else {
                tenant_type
            };
            assert_eq!(root_organization_type(tenant_type), expected_root_type);
        }
        for invalid in ["", "company", "SUBSIDIARY", "UNKNOWN"] {
            assert!(validate_tenant_type(invalid).is_err());
        }
        let omitted: TenantUpdate = serde_json::from_value(json!({"name": "新名称"})).unwrap();
        assert_eq!(omitted.tenant_type, None);
        let update: TenantUpdate =
            serde_json::from_value(json!({"tenantType": "GOVERNMENT"})).unwrap();
        assert_eq!(update.tenant_type.as_deref(), Some("GOVERNMENT"));
    }

    #[test]
    fn tenant_local_region_updates_distinguish_omission_from_clear() {
        let omitted: TenantUpdate = serde_json::from_value(json!({"name": "新名称"})).unwrap();
        assert_eq!(omitted.township, None);
        assert_eq!(omitted.village, None);
        let selected: TenantUpdate = serde_json::from_value(json!({
            "township": "转塘街道", "village": "龙王沙社区",
        }))
        .unwrap();
        assert_eq!(selected.township.as_deref(), Some("转塘街道"));
        assert_eq!(selected.village.as_deref(), Some("龙王沙社区"));
        let cleared: TenantUpdate =
            serde_json::from_value(json!({"township": "", "village": ""})).unwrap();
        assert_eq!(cleared.township.as_deref(), Some(""));
        assert_eq!(cleared.village.as_deref(), Some(""));
    }

    #[test]
    fn coordinates_require_a_valid_pair() {
        assert!(validate_coordinates(None, None).is_ok());
        assert!(validate_coordinates(Some(180.0), Some(-90.0)).is_ok());
        assert!(validate_coordinates(Some(121.47), None).is_err());
        assert!(validate_coordinates(Some(181.0), Some(30.0)).is_err());
        assert!(validate_coordinates(Some(f64::NAN), Some(30.0)).is_err());
    }

    #[test]
    fn platform_tenant_code_is_reserved_and_fixed() {
        assert!(validate_tenant_identity("平台管理租户", "admin-tenant", true).is_ok());
        assert!(validate_tenant_identity("平台管理租户", "platform", true).is_err());
        assert!(validate_tenant_identity("业务租户", "admin-tenant", false).is_err());
        assert!(validate_tenant_identity("业务租户", "business", false).is_ok());
        assert!(validate_tenant_identity(" ", "business", false).is_err());
        assert!(validate_tenant_identity("", "business", false).is_err());
        assert!(validate_tenant_identity("错误\n名称", "business", false).is_err());
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TenantUpdate {
    #[serde(flatten)]
    contact: ContactUpdate,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    tenant_type: Option<String>,
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
    #[serde(default, deserialize_with = "double_option")]
    expires_at: Option<Option<DateTime<Utc>>>,
}

/// PUT /api/tenants/{id}
async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<TenantUpdate>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:tenant")
        .check(&state, &user)
        .await?;
    state.require_platform(&user).await?;
    let id: Uuid = id
        .parse()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    if let Some(name) = &body.name {
        if name.trim().is_empty() {
            return Err(AppError::bad_request("租户名称不能为空"));
        }
    }
    if let Some(tenant_type) = &body.tenant_type {
        validate_tenant_type(tenant_type)?;
    }
    let contact = body.contact.normalized()?;
    let mut tx = state.pool.begin().await?;
    let before = tenant_configuration_snapshot(&mut tx, id).await?;
    let (longitude, latitude, current_name, current_code, is_platform): (
        Option<f64>,
        Option<f64>,
        String,
        String,
        bool,
    ) = sqlx::query_as(
        "SELECT longitude, latitude, name, code, is_platform FROM tenants WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("租户不存在"))?;
    validate_tenant_identity(
        body.name.as_deref().unwrap_or(&current_name).trim(),
        body.code.as_deref().unwrap_or(&current_code).trim(),
        is_platform,
    )?;
    if is_platform && body.expires_at.flatten().is_some() {
        return Err(AppError::bad_request("平台租户不允许设置过期时间"));
    }
    validate_coordinates(
        body.longitude.unwrap_or(longitude),
        body.latitude.unwrap_or(latitude),
    )?;
    let row = sqlx::query_as::<_, TenantRow>(
        // 注: 目标表别名 t 供 RETURNING 子查询引用 (无别名时 o.tenant_id = t.id 报 missing FROM-clause,
        // 此前该端点一直 500, 本任务修复: UPDATE tenants AS t ... WHERE t.id = $1)
        "UPDATE tenants AS t SET name = COALESCE($2, name), code = COALESCE($3, code), \
         description = COALESCE($4, description), \
         expires_at = CASE WHEN $5 THEN $6::timestamptz ELSE expires_at END, \
         province = COALESCE($7, province), city = COALESCE($8, city), district = COALESCE($9, district), \
         township = COALESCE($10, township), village = COALESCE($11, village), address = COALESCE($12, address), \
         longitude = CASE WHEN $13 THEN $14::double precision ELSE longitude END, \
         latitude = CASE WHEN $15 THEN $16::double precision ELSE latitude END, \
         tenant_type = COALESCE($17, tenant_type), contact_name = COALESCE($18, contact_name), \
         contact_phone = COALESCE($19, contact_phone), contact_email = COALESCE($20, contact_email), updated_at = now() \
         WHERE t.id = $1 RETURNING id, name, code, tenant_type, contact_name, contact_phone, contact_email, description, status, is_platform, created_at, expires_at, \
         province, city, district, township, village, address, longitude, latitude, \
         (SELECT count(*) FROM organizations o WHERE o.tenant_id = t.id) AS organization_count, \
         (SELECT count(DISTINCT m.user_id) FROM memberships m JOIN organizations o ON o.id = m.organization_id WHERE o.tenant_id = t.id) AS user_count, \
         (SELECT count(*) FROM tenant_applications ta WHERE ta.tenant_id = t.id) AS authorized_app_count"
    )
    .bind(id)
    .bind(body.name.as_deref().map(str::trim))
    .bind(body.code.as_deref().map(str::trim))
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
    .bind(&body.tenant_type)
    .bind(&contact.contact_name)
    .bind(&contact.contact_phone)
    .bind(&contact.contact_email)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("租户不存在"))?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let after = tenant_configuration_snapshot(&mut tx, id).await?;
    if before != after {
        let detail = audit::change_detail("tenant", id, before, after);
        audit::write_in_tx(
            &mut tx,
            audit::AuditInfo {
                user_id: Some(user.user_id),
                username: &user.username,
                tenant_id: Some(id),
                organization_id: Some(user.org_id),
                action: "UPDATE_TENANT",
                detail: &detail,
                ip: &ip,
                user_agent: &ua,
            },
        )
        .await?;
    }
    tx.commit().await?;
    state.invalidate_perm_cache();
    Ok(Json(serde_json::to_value(&row).unwrap_or_default()))
}

/// DELETE /api/tenants/{id} — 级联删除组织/授权/角色 (FK CASCADE); 平台租户禁删
async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:tenant")
        .check(&state, &user)
        .await?;
    state.require_platform(&user).await?;
    let id: Uuid = id
        .parse()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let mut tx = state.pool.begin().await?;
    // Deletion needs the stronger lock; ordinary edits leave FK checks free.
    sqlx::query("SELECT id FROM tenants WHERE id=$1 FOR UPDATE")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let before = tenant_audit_snapshot(&mut tx, id).await?;
    let is_platform: bool = sqlx::query_scalar("SELECT is_platform FROM tenants WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::not_found("租户不存在"))?;
    if is_platform {
        return Err(AppError::bad_request("平台租户不允许删除"));
    }
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let detail = audit::change_detail("tenant", id, before, Value::Null);
    // Insert while the target still exists so tenant_name survives its deletion.
    audit::write_in_tx(
        &mut tx,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(id),
            organization_id: Some(user.org_id),
            action: "DELETE_TENANT",
            detail: &detail,
            ip: &ip,
            user_agent: &ua,
        },
    )
    .await?;
    // A tenant cascade deletes organization trees and their roles. Defer only
    // these existing composite checks until the paired CASCADE FKs have settled.
    sqlx::query("SET CONSTRAINTS fk_organizations_parent_same_tenant, fk_roles_organization_same_tenant DEFERRED")
        .execute(&mut *tx).await?;
    sqlx::query("DELETE FROM tenants WHERE id=$1")
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

/// PUT /api/tenants/{id}/status
async fn update_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<StatusBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:tenant")
        .check(&state, &user)
        .await?;
    state.require_platform(&user).await?;
    if !["ENABLED", "DISABLED"].contains(&body.status.as_str()) {
        return Err(AppError::bad_request("状态仅支持 ENABLED / DISABLED"));
    }
    let id: Uuid = id
        .parse()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let mut tx = state.pool.begin().await?;
    let before = tenant_configuration_snapshot(&mut tx, id).await?;
    let is_platform: bool = sqlx::query_scalar("SELECT is_platform FROM tenants WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::not_found("租户不存在"))?;
    if is_platform {
        return Err(AppError::bad_request("平台租户不允许停用"));
    }
    sqlx::query("UPDATE tenants SET status = $2, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(&body.status)
        .execute(&mut *tx)
        .await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let after = tenant_configuration_snapshot(&mut tx, id).await?;
    if before != after {
        let detail = audit::change_detail("tenant", id, before, after);
        audit::write_in_tx(
            &mut tx,
            audit::AuditInfo {
                user_id: Some(user.user_id),
                username: &user.username,
                tenant_id: Some(id),
                organization_id: Some(user.org_id),
                action: "UPDATE_TENANT",
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

fn menu_flat_json(m: &MenuFlatRow) -> Value {
    json!({
        "id": m.id, "parentId": m.parent_id, "type": m.r#type, "title": m.title,
        "path": m.path, "icon": m.icon, "permission": m.permission,
        "sortOrder": m.sort_order, "visible": m.visible, "authorized": m.authorized,
        "linkUrl": m.link_url, "linkTarget": m.link_target,
    })
}

fn app_auth_json(a: &AppAuthRow) -> Value {
    json!({
        "id": a.id, "code": a.code, "name": a.name, "icon": a.icon, "authorized": a.authorized,
        "menuCount": a.menu_count, "authorizedMenuCount": a.authorized_menu_count,
    })
}

/// GET /api/tenants/{id}/authorizations — 全部应用 + 各应用全部菜单平铺 (authorized 标记)
/// 租户级授权是平台职能: 仅平台租户可操作
async fn get_authorizations(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:tenant")
        .check(&state, &user)
        .await?;
    state.require_platform(&user).await?;
    let tenant_id: Uuid = id
        .parse()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tenants WHERE id = $1)")
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await?;
    if !exists {
        return Err(AppError::not_found("租户不存在"));
    }
    let apps: Vec<AppAuthRow> = sqlx::query_as(
        "SELECT a.id, a.code, a.name, COALESCE(a.icon,'') AS icon, \
         EXISTS(SELECT 1 FROM tenant_applications ta WHERE ta.tenant_id = $1 AND ta.application_id = a.id) AS authorized, \
         (SELECT count(*) FROM menus m WHERE m.application_id = a.id) AS menu_count, \
         (SELECT count(*) FROM tenant_menus tm JOIN menus m ON m.id = tm.menu_id WHERE tm.tenant_id = $1 AND m.application_id = a.id) AS authorized_menu_count \
         FROM applications a ORDER BY a.sort_order, a.created_at",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let mut menus_map = serde_json::Map::new();
    for app in &apps {
        let rows: Vec<MenuFlatRow> = sqlx::query_as(
            "SELECT m.id, m.parent_id, m.type, m.title, COALESCE(m.path,'') AS path, COALESCE(m.icon,'') AS icon, \
             COALESCE(m.permission,'') AS permission, m.link_url, m.link_target, m.sort_order, m.visible, \
             EXISTS(SELECT 1 FROM tenant_menus tm WHERE tm.tenant_id = $1 AND tm.menu_id = m.id) AS authorized \
             FROM menus m WHERE m.application_id = $2 ORDER BY m.sort_order, m.created_at",
        )
        .bind(tenant_id)
        .bind(app.id)
        .fetch_all(&state.pool)
        .await?;
        menus_map.insert(
            app.id.to_string(),
            Value::Array(rows.iter().map(menu_flat_json).collect()),
        );
    }
    Ok(Json(json!({
        "applications": apps.iter().map(app_auth_json).collect::<Vec<_>>(),
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

/// PUT /api/tenants/{id}/authorizations — 全量对账 + 授权级联清理 (事务)
/// 租户级授权是平台职能: 仅平台租户可操作
async fn put_authorizations(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<AuthzBody>,
) -> Result<Json<Value>, AppError> {
    RequirePermission("system:tenant")
        .check(&state, &user)
        .await?;
    state.require_platform(&user).await?;
    let tenant_id: Uuid = id
        .parse()
        .map_err(|_| AppError::bad_request("ID 格式错误"))?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tenants WHERE id = $1)")
        .bind(tenant_id)
        .fetch_one(&state.pool)
        .await?;
    if !exists {
        return Err(AppError::not_found("租户不存在"));
    }

    let mut app_ids: Vec<Uuid> = Vec::new();
    let mut app_authed: Vec<bool> = Vec::new();
    for item in &body.applications {
        let app: Uuid = item
            .application_id
            .parse()
            .map_err(|_| AppError::bad_request("applicationId 格式错误"))?;
        app_ids.push(app);
        app_authed.push(item.authorized);
    }
    let mut menu_ids: Vec<Uuid> = Vec::new();
    let mut menu_authed: Vec<bool> = Vec::new();
    for item in &body.menus {
        let menu: Uuid = item
            .menu_id
            .parse()
            .map_err(|_| AppError::bad_request("menuId 格式错误"))?;
        menu_ids.push(menu);
        menu_authed.push(item.authorized);
    }

    // 校验应用/菜单存在
    if !app_ids.is_empty() {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM applications WHERE id = ANY($1)")
            .bind(&app_ids)
            .fetch_one(&state.pool)
            .await?;
        if n != app_ids.len() as i64 {
            return Err(AppError::bad_request("存在无效的应用 ID"));
        }
    }
    if !menu_ids.is_empty() {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM menus WHERE id = ANY($1)")
            .bind(&menu_ids)
            .fetch_one(&state.pool)
            .await?;
        if n != menu_ids.len() as i64 {
            return Err(AppError::bad_request("存在无效的菜单 ID"));
        }
    }

    let mut tx = state.pool.begin().await?;
    let before = tenant_authorization_summary(tenant_audit_snapshot(&mut tx, tenant_id).await?);
    for (app, authorized) in app_ids.iter().zip(app_authed.iter()) {
        if *authorized {
            sqlx::query("INSERT INTO tenant_applications (tenant_id, application_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(tenant_id)
                .bind(app)
                .execute(&mut *tx)
                .await?;
        } else {
            // 取消应用: 级联清理该租户所有组织的组织应用/组织菜单 + 该租户全部角色的角色菜单 + 租户菜单
            sqlx::query(
                "DELETE FROM organization_applications oa USING organizations o \
                 WHERE oa.organization_id = o.id AND o.tenant_id = $1 AND oa.application_id = $2",
            )
            .bind(tenant_id)
            .bind(app)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "DELETE FROM organization_menus om USING organizations o, menus m \
                 WHERE om.organization_id = o.id AND o.tenant_id = $1 AND om.menu_id = m.id AND m.application_id = $2",
            )
            .bind(tenant_id)
            .bind(app)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "DELETE FROM role_menus rm USING roles r, menus m \
                 WHERE rm.role_id = r.id AND r.tenant_id = $1 AND rm.menu_id = m.id AND m.application_id = $2",
            )
            .bind(tenant_id)
            .bind(app)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "DELETE FROM tenant_menus WHERE tenant_id = $1 AND menu_id IN (SELECT id FROM menus WHERE application_id = $2)",
            )
            .bind(tenant_id)
            .bind(app)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "DELETE FROM tenant_applications WHERE tenant_id = $1 AND application_id = $2",
            )
            .bind(tenant_id)
            .bind(app)
            .execute(&mut *tx)
            .await?;
        }
    }
    for (menu, authorized) in menu_ids.iter().zip(menu_authed.iter()) {
        if *authorized {
            sqlx::query("INSERT INTO tenant_menus (tenant_id, menu_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                .bind(tenant_id)
                .bind(menu)
                .execute(&mut *tx)
                .await?;
        } else {
            // 取消菜单: 级联清理该租户组织的组织菜单 + 该租户角色的角色菜单 + 租户菜单
            sqlx::query(
                "DELETE FROM organization_menus WHERE menu_id = $2 AND organization_id IN (SELECT id FROM organizations WHERE tenant_id = $1)",
            )
            .bind(tenant_id)
            .bind(menu)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "DELETE FROM role_menus rm USING roles r WHERE rm.role_id = r.id AND r.tenant_id = $1 AND rm.menu_id = $2",
            )
            .bind(tenant_id)
            .bind(menu)
            .execute(&mut *tx)
            .await?;
            sqlx::query("DELETE FROM tenant_menus WHERE tenant_id = $1 AND menu_id = $2")
                .bind(tenant_id)
                .bind(menu)
                .execute(&mut *tx)
                .await?;
        }
    }
    // 兜底对账: 保证 租户菜单⊆租户应用、组织授权⊆租户授权、角色菜单⊆组织授权
    sqlx::query(
        "DELETE FROM tenant_menus tm WHERE tm.tenant_id = $1 AND NOT EXISTS ( \
         SELECT 1 FROM tenant_applications ta JOIN menus m ON m.id = tm.menu_id \
         WHERE ta.tenant_id = $1 AND ta.application_id = m.application_id)",
    )
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM organization_applications oa WHERE oa.organization_id IN (SELECT id FROM organizations WHERE tenant_id = $1) \
         AND NOT EXISTS (SELECT 1 FROM tenant_applications ta WHERE ta.tenant_id = $1 AND ta.application_id = oa.application_id)",
    )
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM organization_menus om WHERE om.organization_id IN (SELECT id FROM organizations WHERE tenant_id = $1) \
         AND NOT EXISTS (SELECT 1 FROM tenant_menus tm WHERE tm.tenant_id = $1 AND tm.menu_id = om.menu_id)",
    )
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM role_menus rm USING roles r WHERE rm.role_id = r.id AND r.tenant_id = $1 \
         AND NOT EXISTS (SELECT 1 FROM organization_menus om WHERE om.organization_id = r.organization_id AND om.menu_id = rm.menu_id)",
    )
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;
    onboarding::sync_tenant_admin(&mut tx, tenant_id).await?;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let after = tenant_authorization_summary(tenant_audit_snapshot(&mut tx, tenant_id).await?);
    if before != after {
        let detail = audit::change_detail("tenant", tenant_id, before, after);
        audit::write_in_tx(
            &mut tx,
            audit::AuditInfo {
                user_id: Some(user.user_id),
                username: &user.username,
                tenant_id: Some(tenant_id),
                organization_id: Some(user.org_id),
                action: "TENANT_AUTH",
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
    RequirePermission("system:tenant")
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
    state.require_platform(&user).await?;
    let existing_org_scope = state.scoped_org_ids(&user).await?;
    let mut tx = state.pool.begin().await?;
    let before = tenant_audit_snapshot(&mut tx, id).await?;
    let (contact_name, contact_phone, contact_email): (String, String, String) = sqlx::query_as(
        "SELECT contact_name,contact_phone,contact_email FROM tenants WHERE id=$1 FOR NO KEY UPDATE",
    ).bind(id).fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::not_found("租户不存在"))?;
    let roots: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM organizations WHERE tenant_id=$1 AND parent_id IS NULL ORDER BY id",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    if roots.len() != 1 {
        return Err(AppError::bad_request(
            "租户需要唯一根组织，请先维护组织结构",
        ));
    }
    let tenant_id = id;
    let org_id = roots[0];
    let role_code = "tenant_admin";
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
        let mut after = tenant_audit_snapshot(&mut tx, id).await?;
        after["administratorUserId"] = json!(administrator.user_id);
        after["administratorCreated"] = json!(administrator.created);
        let detail = audit::change_detail("tenant", id, before, after);
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
