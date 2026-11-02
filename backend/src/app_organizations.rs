//! Application-owned operation proof plus a fresh platform STORE projection.
//! Directory metadata never proves a business record's SELF ownership.
use crate::util::uuid7::ParseUuid7;
mod store;

use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use uuid::Uuid;

use crate::{directory_authorization::Access, error::AppError, state::AppState, util::hash};
use saas_platform_backend::authorization_policy::{
    member_organization_allowed, MemberScope, OrganizationCeiling,
};

const WINDOW_SECONDS: i64 = 3;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LookupQuery {
    pub(crate) resource: String,
    pub(crate) action: String,
    pub(crate) page: Option<i64>,
    pub(crate) size: Option<i64>,
    pub(crate) q: Option<String>,
}

struct Search {
    page: i64,
    size: i64,
    keyword: String,
}

fn search(query: &LookupQuery, single: bool) -> Result<Search, AppError> {
    if single && (query.page.is_some() || query.size.is_some() || query.q.is_some()) {
        return Err(AppError::bad_request("单组织校验不接受分页或搜索参数"));
    }
    let result = Search {
        page: query.page.unwrap_or(1),
        size: query.size.unwrap_or(20),
        keyword: query.q.as_deref().unwrap_or("").trim().to_owned(),
    };
    if !(1..=1_000_000).contains(&result.page)
        || !(1..=50).contains(&result.size)
        || result.keyword.contains('\0')
        || result.keyword.chars().count() > 100
    {
        return Err(AppError::bad_request("组织目录分页或搜索参数超出范围"));
    }
    Ok(result)
}

fn uuid(value: &Value) -> Result<Uuid, AppError> {
    value
        .as_str()
        .and_then(|text| text.parse_uuid7().ok())
        .filter(|id: &Uuid| !id.is_nil())
        .ok_or_else(|| AppError::unauthorized("应用组织上下文无效"))
}

fn ids(value: &Value) -> Result<Vec<Uuid>, AppError> {
    let values = value
        .as_array()
        .filter(|items| items.len() <= 10000)
        .ok_or_else(|| AppError::unauthorized("应用组织范围无效"))?;
    let result = values.iter().map(uuid).collect::<Result<Vec<_>, _>>()?;
    if result.iter().collect::<BTreeSet<_>>().len() != result.len() {
        return Err(AppError::unauthorized("应用组织范围无效"));
    }
    Ok(result)
}

/// Discovery uses the member-directory target rule. Known-row metadata may
/// resolve SELF's grant/ceiling intersection, but never proves business ownership.
fn allowed_ids(
    context: &Value,
    grant: &Value,
    current: Uuid,
    known_metadata: bool,
) -> Result<Option<Vec<Uuid>>, AppError> {
    let ceiling_ids = match context.get("organizationScopeIds") {
        Some(value) if !value.is_null() => Some(ids(value)?),
        _ => None,
    };
    let ceiling = match (context.get("organizationScopeIds"), ceiling_ids.as_deref()) {
        (None, _) => OrganizationCeiling::Current,
        (Some(Value::Null), _) => OrganizationCeiling::Tenant,
        (_, Some(values)) if values.contains(&current) => OrganizationCeiling::Finite(values),
        _ => return Err(AppError::unauthorized("应用组织范围无效")),
    };
    let granted = ids(&grant["organizationIds"])?;
    let scope = match grant["scope"].as_str() {
        Some("SELF") => MemberScope::SelfScope,
        Some("ORG") => MemberScope::Org,
        Some("ORG_SUBTREE") => MemberScope::OrgSubtree,
        Some("TENANT") => MemberScope::Tenant,
        _ => return Err(AppError::forbidden("组织用途数据授权无效")),
    };
    if known_metadata && matches!(scope, MemberScope::SelfScope) {
        let hard = match &ceiling {
            OrganizationCeiling::Current => Some(vec![current]),
            OrganizationCeiling::Tenant => None,
            OrganizationCeiling::Finite(values) => Some(values.to_vec()),
        };
        return Ok(match hard {
            Some(values) => Some(
                values
                    .into_iter()
                    .filter(|id| granted.is_empty() || granted.contains(id))
                    .collect(),
            ),
            None if granted.is_empty() => None,
            None => Some(granted),
        });
    }
    let permits = |target| {
        member_organization_allowed(
            scope,
            &granted,
            match &ceiling {
                OrganizationCeiling::Current => OrganizationCeiling::Current,
                OrganizationCeiling::Tenant => OrganizationCeiling::Tenant,
                OrganizationCeiling::Finite(values) => OrganizationCeiling::Finite(values),
            },
            current,
            target,
        )
    };
    if !permits(current) {
        return Err(AppError::forbidden("组织用途数据授权无效"));
    }
    if matches!(scope, MemberScope::Tenant) && matches!(ceiling, OrganizationCeiling::Tenant) {
        return Ok(None);
    }
    let candidates = match scope {
        MemberScope::SelfScope | MemberScope::Org => vec![current],
        MemberScope::OrgSubtree => granted.clone(),
        MemberScope::Tenant => ceiling_ids.clone().unwrap_or_else(|| vec![current]),
    };
    Ok(Some(
        candidates.into_iter().filter(|id| permits(*id)).collect(),
    ))
}

pub(crate) async fn lookup(
    state: &AppState,
    code: &str,
    headers: &HeaderMap,
    query: LookupQuery,
    target: Option<Uuid>,
) -> Result<Value, AppError> {
    lookup_with_access(state, code, Access::Internal(headers), query, target).await
}

pub(crate) async fn lookup_with_access(
    state: &AppState,
    code: &str,
    access: Access<'_>,
    query: LookupQuery,
    target: Option<Uuid>,
) -> Result<Value, AppError> {
    let deadline = Utc::now() + chrono::Duration::seconds(WINDOW_SECONDS);
    tokio::time::timeout(
        std::time::Duration::from_secs(WINDOW_SECONDS as u64),
        lookup_inner(state, code, access, query, target, deadline),
    )
    .await
    .map_err(|_| AppError::Unavailable)?
}

async fn lookup_inner(
    state: &AppState,
    code: &str,
    access: Access<'_>,
    query: LookupQuery,
    target: Option<Uuid>,
    mut deadline: DateTime<Utc>,
) -> Result<Value, AppError> {
    if target.is_some_and(|id| !crate::util::uuid7::is_uuid_v7(id)) {
        return Err(AppError::bad_request("组织 ID 无效"));
    }
    let search = search(&query, target.is_some())?;
    let checked = access.context(state, code).await?;
    let context = &checked.context;
    let app = uuid(&context["application"]["id"])?;
    let user = uuid(&context["user"]["id"])?;
    let tenant = uuid(&context["tenant"]["id"])?;
    let current = uuid(&context["organization"]["id"])?;
    if state
        .config
        .data_grant_policy
        .organization_lookup(app, &query.resource, &query.action)
        .is_none()
    {
        return Err(AppError::bad_request("组织目录用途未登记"));
    }
    let permission = format!("{}:{}", query.resource, query.action);
    if !context["permissions"]
        .as_array()
        .is_some_and(|values| values.iter().any(|v| v.as_str() == Some(&permission)))
    {
        return Err(AppError::forbidden("缺少组织用途操作权限"));
    }
    let grants = context["dataGrants"]
        .as_array()
        .ok_or_else(|| AppError::forbidden("缺少组织用途数据授权"))?;
    let matching = grants
        .iter()
        .filter(|grant| {
            grant["resource"] == query.resource
                && grant["action"] == query.action
                && grant["subjectUserId"].as_str() == Some(user.to_string().as_str())
                && grant["tenantId"].as_str() == Some(tenant.to_string().as_str())
        })
        .collect::<Vec<_>>();
    if matching.len() != 1 {
        return Err(AppError::forbidden("缺少组织用途数据授权"));
    }
    let allowed = allowed_ids(context, matching[0], current, target.is_some())?;
    let first = store::load(
        &state.pool,
        tenant,
        app,
        allowed.as_deref(),
        &search,
        target,
    )
    .await?;
    if target.is_some() && first.records.is_empty() {
        return Err(AppError::not_found("组织不存在或不在授权范围内"));
    }
    // An independent statement detects target type/state/application-grant or
    // metadata changes not represented by the actor's context version.
    if access
        .revalidate(state, code, checked.revision)
        .await?
        .is_some()
    {
        return Err(AppError::Unavailable);
    }
    let latest = store::load(
        &state.pool,
        tenant,
        app,
        allowed.as_deref(),
        &search,
        target,
    )
    .await?;
    if serde_json::to_value(&first).map_err(anyhow::Error::from)?
        != serde_json::to_value(&latest).map_err(anyhow::Error::from)?
    {
        return Err(AppError::Unavailable);
    }
    let context_deadline = context["expiresAt"]
        .as_str()
        .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .ok_or_else(|| AppError::unauthorized("应用上下文期限无效"))?
        .with_timezone(&Utc);
    deadline = deadline
        .min(context_deadline)
        .min(checked.binding.expires_at);
    for row in &latest.records {
        if let Some(expiry) = row.authority_expires_at {
            deadline = deadline.min(expiry);
        }
    }
    if deadline <= Utc::now() {
        return Err(AppError::Unavailable);
    }
    let expires = deadline.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut records = Vec::with_capacity(latest.records.len());
    for row in latest.records {
        let revision = hash::sha256_hex(&serde_json::to_string(&row).map_err(anyhow::Error::from)?);
        let mut value = serde_json::to_value(row).map_err(anyhow::Error::from)?;
        if let Some(object) = value.as_object_mut() {
            object.remove("updatedAt");
            object.remove("authorityExpiresAt");
        }
        value["contractVersion"] = json!("1.0");
        value["contextVersion"] = context["contextVersion"].clone();
        value["revision"] = json!(revision);
        value["expiresAt"] = json!(expires);
        records.push(value);
    }
    let response = if target.is_some() {
        records
            .into_iter()
            .next()
            .ok_or_else(|| AppError::not_found("组织不存在"))?
    } else {
        json!({"contractVersion":"1.0","contextVersion":context["contextVersion"],"expiresAt":expires,
            "records":records,"total":latest.total,"page":search.page,"size":search.size})
    };
    if serde_json::to_vec(&response)
        .map_err(anyhow::Error::from)?
        .len()
        > 1024 * 1024
    {
        return Err(AppError::bad_request("组织目录响应超过 1 MiB 上限"));
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn self_never_widens_to_descendants_and_missing_ceiling_is_current_only() {
        let current = crate::util::uuid7::new_uuid();
        let child = crate::util::uuid7::new_uuid();
        let context = json!({"organizationScopeIds":[current,child]});
        let grant = json!({"scope":"SELF","organizationIds":[current,child]});
        assert_eq!(
            allowed_ids(&context, &grant, current, false).unwrap(),
            Some(vec![current])
        );
        let grant = json!({"scope":"ORG_SUBTREE","organizationIds":[current,child]});
        assert_eq!(
            allowed_ids(&json!({}), &grant, current, false).unwrap(),
            Some(vec![current])
        );
        assert_eq!(
            allowed_ids(&context, &grant, current, false).unwrap(),
            Some(vec![current, child])
        );
    }
    #[test]
    fn invalid_directory_boundaries_fail_closed() {
        let current = crate::util::uuid7::new_uuid();
        let other = crate::util::uuid7::new_uuid();
        assert!(allowed_ids(
            &json!({"organizationScopeIds":[other]}),
            &json!({"scope":"TENANT","organizationIds":[]}),
            current,
            false
        )
        .is_err());
        assert!(allowed_ids(
            &json!({"organizationScopeIds":null}),
            &json!({"scope":"TENANT","organizationIds":[current]}),
            current,
            false
        )
        .is_err());
    }

    #[test]
    fn self_known_metadata_stays_in_the_grant_and_hard_ceiling_without_owner_claims() {
        let current = crate::util::uuid7::new_uuid();
        let child = crate::util::uuid7::new_uuid();
        let sibling = crate::util::uuid7::new_uuid();
        let context = json!({"organizationScopeIds":[current,child]});
        let grant = json!({"scope":"SELF","organizationIds":[current,child,sibling]});
        assert_eq!(
            allowed_ids(&context, &grant, current, true).unwrap(),
            Some(vec![current, child])
        );
        assert_eq!(
            allowed_ids(&json!({}), &grant, current, true).unwrap(),
            Some(vec![current])
        );
    }
}
