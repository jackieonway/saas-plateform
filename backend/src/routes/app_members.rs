//! Minimal direct-member view for business assignment, protected by the current
//! application's operation permission and independent resource data grant.
//! Legacy user-ID responses always resolve one enabled EMPLOYEE relationship;
//! another relationship type can neither qualify nor duplicate that target.
use crate::util::uuid7::ParseUuid7;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::directory_authorization::Access;
use crate::error::AppError;
use crate::state::AppState;
use chrono::{DateTime, Utc};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/apps/{appCode}/members", get(list))
        .route("/api/apps/{appCode}/members/{userId}", get(one))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MemberQuery {
    pub(crate) resource: String,
    pub(crate) action: String,
    #[serde(deserialize_with = "crate::util::uuid7::deserialize")]
    pub(crate) organization_id: Uuid,
    pub(crate) page: Option<i64>,
    pub(crate) size: Option<i64>,
    pub(crate) q: Option<String>,
}

fn validate(query: &MemberQuery, single: bool) -> Result<(i64, i64, String), AppError> {
    crate::util::uuid7::validate_uuid(query.organization_id)?;
    if single && (query.page.is_some() || query.size.is_some() || query.q.is_some()) {
        return Err(AppError::bad_request("单成员校验不接受分页或搜索参数"));
    }
    let page = query.page.unwrap_or(1);
    let size = query.size.unwrap_or(20);
    let q = query.q.as_deref().unwrap_or("").trim();
    if q.contains('\0') {
        return Err(AppError::bad_request("搜索关键词不能包含空字符"));
    }
    if !(1..=1_000_000).contains(&page) || !(1..=50).contains(&size) || q.chars().count() > 100 {
        return Err(AppError::bad_request("分页或搜索参数超出范围"));
    }
    let pattern = if q.is_empty() {
        String::new()
    } else {
        format!(
            "%{}%",
            q.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        )
    };
    Ok((page, size, pattern))
}

fn uuid_of(ctx: &Value, parent: &str) -> Result<Uuid, AppError> {
    ctx[parent]["id"]
        .as_str()
        .and_then(|value| value.parse_uuid7().ok())
        .ok_or_else(|| AppError::unauthorized("应用上下文无效"))
}

fn organization_ids(value: &Value) -> Option<Vec<Uuid>> {
    let ids = value.as_array()?;
    if ids.len() > 10_000 {
        return None;
    }
    ids.iter()
        .map(|id| id.as_str()?.parse_uuid7().ok())
        .collect()
}

fn organization_allowed(
    ctx: &Value,
    grant: &Value,
    org: Uuid,
    target: Uuid,
) -> Result<bool, AppError> {
    use saas_platform_backend::authorization_policy::{
        member_organization_allowed, MemberScope, OrganizationCeiling,
    };
    let finite = match ctx.get("organizationScopeIds") {
        Some(value) if !value.is_null() => Some(
            organization_ids(value)
                .filter(|ids| ids.contains(&org))
                .ok_or_else(|| AppError::unauthorized("应用组织数据范围无效"))?,
        ),
        _ => None,
    };
    let ceiling = match (ctx.get("organizationScopeIds"), finite.as_deref()) {
        (None, _) => OrganizationCeiling::Current,
        (Some(Value::Null), _) => OrganizationCeiling::Tenant,
        (_, Some(ids)) => OrganizationCeiling::Finite(ids),
        _ => return Err(AppError::unauthorized("应用组织数据范围无效")),
    };
    let ids = organization_ids(&grant["organizationIds"])
        .ok_or_else(|| AppError::forbidden("成员用途的数据授权范围无效"))?;
    let scope = match grant["scope"].as_str() {
        Some("SELF") => MemberScope::SelfScope,
        Some("ORG") => MemberScope::Org,
        Some("ORG_SUBTREE") => MemberScope::OrgSubtree,
        Some("TENANT") => MemberScope::Tenant,
        _ => return Ok(false),
    };
    Ok(member_organization_allowed(
        scope, &ids, ceiling, org, target,
    ))
}

struct LookupProof {
    identity: (Uuid, Uuid, Uuid, Uuid),
    revision: saas_platform_backend::snapshot::Revision,
}
async fn authorized(
    state: &AppState,
    app_code: &str,
    access: &Access<'_>,
    query: &MemberQuery,
) -> Result<LookupProof, AppError> {
    let checked = access.context(state, app_code).await?;
    let identity = authorize_projection(state, &checked.context, query).await?;
    Ok(LookupProof {
        identity,
        revision: checked.revision,
    })
}
async fn validate_target(
    state: &AppState,
    target: Uuid,
    tenant: Uuid,
    app: Uuid,
) -> Result<(), AppError> {
    let valid_org: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM organizations o \
         JOIN tenants t ON t.id=o.tenant_id AND t.status='ENABLED' AND (t.expires_at IS NULL OR t.expires_at>now()) \
         JOIN applications a ON a.id=$3 AND a.status='ENABLED' \
         JOIN tenant_applications ta ON ta.tenant_id=t.id AND ta.application_id=a.id \
         JOIN organization_applications oa ON oa.organization_id=o.id AND oa.application_id=a.id \
         WHERE o.id=$1 AND o.tenant_id=$2 AND org_chain_active(o.id))"
    ).bind(target).bind(tenant).bind(app).fetch_one(&state.pool).await?;
    if !valid_org {
        return Err(AppError::not_found("组织不存在或不能访问当前应用"));
    }

    Ok(())
}
async fn authorize_projection(
    state: &AppState,
    ctx: &Value,
    query: &MemberQuery,
) -> Result<(Uuid, Uuid, Uuid, Uuid), AppError> {
    let user = uuid_of(ctx, "user")?;
    let tenant = uuid_of(ctx, "tenant")?;
    let org = uuid_of(ctx, "organization")?;
    let app = uuid_of(ctx, "application")?;
    let capability = state
        .config
        .data_grant_policy
        .assignment_members(app, &query.resource, &query.action)
        .ok_or_else(|| AppError::bad_request("成员分配用途未登记或资源动作不受支持"))?;
    let permission = format!("{}:{}", capability.resource, capability.action);
    if !ctx["permissions"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|value| value.as_str() == Some(&permission))
    }) {
        return Err(AppError::forbidden("缺少当前应用的成员用途操作权限"));
    }
    let grant = ctx["dataGrants"]
        .as_array()
        .and_then(|items| {
            items.iter().find(|item| {
                item["resource"] == capability.resource
                    && item["action"] == capability.action
                    && item["subjectUserId"].as_str() == Some(user.to_string().as_str())
                    && item["tenantId"].as_str() == Some(tenant.to_string().as_str())
            })
        })
        .ok_or_else(|| AppError::forbidden("缺少成员用途的独立数据授权"))?;
    if !organization_allowed(ctx, grant, org, query.organization_id)? {
        return Err(AppError::not_found("组织不存在或不在授权范围内"));
    }
    validate_target(state, query.organization_id, tenant, app).await?;
    Ok((user, tenant, org, app))
}

async fn recheck(
    state: &AppState,
    app_code: &str,
    access: &Access<'_>,
    query: &MemberQuery,
    proof: &LookupProof,
) -> Result<(), AppError> {
    if let Some(context) = access.revalidate(state, app_code, proof.revision).await? {
        if authorize_projection(state, &context, query).await? != proof.identity {
            return Err(AppError::unauthorized("应用上下文已改变"));
        }
    } else {
        validate_target(
            state,
            query.organization_id,
            proof.identity.1,
            proof.identity.3,
        )
        .await?;
    }
    Ok(())
}

pub(crate) struct LookupResult {
    pub(crate) data: Value,
    pub(crate) expires_at: Option<DateTime<Utc>>,
}

async fn load(
    state: &AppState,
    query: &MemberQuery,
    identity: &(Uuid, Uuid, Uuid, Uuid),
    page: i64,
    size: i64,
    pattern: &str,
    target: Option<Uuid>,
) -> Result<LookupResult, AppError> {
    let row:Option<(Value,Option<DateTime<Utc>>)>=sqlx::query_as(
        "WITH target AS (SELECT LEAST(t.expires_at,org_chain_expires_at(o.id)) AS expires_at \
         FROM organizations o JOIN tenants t ON t.id=o.tenant_id AND t.status='ENABLED' \
           AND (t.expires_at IS NULL OR t.expires_at>clock_timestamp()) \
         JOIN applications a ON a.id=$4 AND a.status='ENABLED' AND 'EMPLOYEE'=ANY(a.allowed_member_types) \
         JOIN tenant_applications ta ON ta.tenant_id=t.id AND ta.application_id=a.id \
         JOIN organization_applications oa ON oa.organization_id=o.id AND oa.application_id=a.id \
         WHERE o.id=$1 AND o.tenant_id=$3 AND org_chain_active(o.id) \
           AND (org_chain_expires_at(o.id) IS NULL OR org_chain_expires_at(o.id)>clock_timestamp())), \
         matching AS (SELECT u.id,COALESCE(NULLIF(trim(u.real_name),''),u.username) AS display_name \
           FROM memberships m JOIN users u ON u.id=m.user_id CROSS JOIN target \
           WHERE m.organization_id=$1 AND m.member_type='EMPLOYEE' AND m.status='ENABLED' \
             AND u.status='ENABLED' AND ($7::uuid IS NULL OR m.user_id=$7) \
             AND ($2='' OR COALESCE(NULLIF(trim(u.real_name),''),u.username) ILIKE $2 ESCAPE CHR(92))) \
         SELECT jsonb_build_object('records',COALESCE((SELECT jsonb_agg(jsonb_build_object( \
             'id',id,'displayName',display_name,'organizationId',$1) ORDER BY id) \
             FROM (SELECT id,display_name FROM matching ORDER BY id LIMIT $6::bigint OFFSET ($5::bigint-1)*$6::bigint) page),'[]'::jsonb), \
             'total',(SELECT count(*) FROM matching),'page',$5,'size',$6),target.expires_at FROM target")
        .bind(query.organization_id).bind(pattern).bind(identity.1).bind(identity.3)
        .bind(page).bind(size).bind(target).fetch_optional(&state.pool).await?;
    let (mut data, expires_at) =
        row.ok_or_else(|| AppError::not_found("组织不存在或不能访问当前应用"))?;
    if target.is_some() {
        data = data["records"]
            .as_array()
            .and_then(|rows| rows.first())
            .cloned()
            .ok_or_else(|| AppError::not_found("有效直接成员不存在"))?;
    }
    Ok(LookupResult { data, expires_at })
}

/// Shared directory policy and target projection for public and private callers.
pub(crate) async fn lookup(
    state: &AppState,
    app_code: &str,
    access: Access<'_>,
    query: MemberQuery,
    target: Option<Uuid>,
) -> Result<LookupResult, AppError> {
    if let Some(target) = target {
        crate::util::uuid7::validate_uuid(target)?;
    }
    let (page, size, pattern) = validate(&query, target.is_some())?;
    let identity = authorized(state, app_code, &access, &query).await?;
    let first = load(
        state,
        &query,
        &identity.identity,
        page,
        size,
        &pattern,
        target,
    )
    .await?;
    recheck(state, app_code, &access, &query, &identity).await?;
    let latest = load(
        state,
        &query,
        &identity.identity,
        page,
        size,
        &pattern,
        target,
    )
    .await?;
    if first.data != latest.data || first.expires_at != latest.expires_at {
        return Err(AppError::Unavailable);
    }
    if latest.expires_at.is_some_and(|expiry| expiry <= Utc::now()) {
        return Err(AppError::Unavailable);
    }
    Ok(latest)
}

async fn list(
    State(state): State<AppState>,
    Path(app_code): Path<String>,
    headers: HeaderMap,
    Query(query): Query<MemberQuery>,
) -> Result<Json<Value>, AppError> {
    lookup(&state, &app_code, Access::Internal(&headers), query, None)
        .await
        .map(|result| Json(result.data))
}

async fn one(
    State(state): State<AppState>,
    Path((app_code, user_id)): Path<(String, Uuid)>,
    headers: HeaderMap,
    Query(query): Query<MemberQuery>,
) -> Result<Json<Value>, AppError> {
    lookup(
        &state,
        &app_code,
        Access::Internal(&headers),
        query,
        Some(user_id),
    )
    .await
    .map(|result| Json(result.data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn member_search_rejects_nul_and_preserves_literal_pattern_characters() {
        let mut query = MemberQuery {
            resource: "ticket".into(),
            action: "assign".into(),
            organization_id: crate::util::uuid7::fixture_uuid(1),
            page: None,
            size: None,
            q: Some("alice\0bob".into()),
        };
        // PostgreSQL text cannot encode NUL, so reject before either the count
        // or the member query can turn caller input into a database error.
        assert!(matches!(
            validate(&query, false),
            Err(AppError::BadRequest(_))
        ));
        query.q = Some("  %_\\  ".into());
        assert_eq!(
            validate(&query, false).unwrap(),
            (1, 20, "%\\%\\_\\\\%".into())
        );
        query.q = Some("界".repeat(100));
        assert!(validate(&query, false).is_ok());
        query.q = Some("界".repeat(101));
        assert!(matches!(
            validate(&query, false),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn assignment_scopes_intersect_a_separate_ceiling_and_self_stays_current() {
        let org = crate::util::uuid7::fixture_uuid(1);
        let child = crate::util::uuid7::fixture_uuid(2);
        let peer = crate::util::uuid7::fixture_uuid(3);
        for scope in ["SELF", "ORG_SUBTREE", "TENANT"] {
            let grant = json!({"scope":scope,"organizationIds":if scope=="TENANT" {vec![]} else {vec![org,child,peer]}});
            let ctx = json!({"organizationScopeIds":[org,child]});
            assert!(organization_allowed(&ctx, &grant, org, org).unwrap());
            assert_eq!(
                organization_allowed(&ctx, &grant, org, child).unwrap(),
                scope != "SELF"
            );
            assert!(!organization_allowed(&ctx, &grant, org, peer).unwrap());
            assert!(!organization_allowed(&json!({}), &grant, org, child).unwrap());
        }
        let tenant = json!({"scope":"TENANT","organizationIds":[]});
        assert!(
            organization_allowed(&json!({"organizationScopeIds":null}), &tenant, org, peer)
                .unwrap()
        );
        for ceiling in [json!([]), json!([child]), json!([org, "bad"]), json!("bad")] {
            assert!(organization_allowed(
                &json!({"organizationScopeIds":ceiling}),
                &tenant,
                org,
                org
            )
            .is_err());
        }
    }
}
