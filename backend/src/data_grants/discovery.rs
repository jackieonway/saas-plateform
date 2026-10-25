//! Read-only discovery for the existing controlled grant-management boundary.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use sqlx::{types::Json, Postgres, Transaction};
use uuid::Uuid;

use super::{
    ensure_size,
    policy::{READ_PERMISSION, WRITE_PERMISSION},
    store, Target, MAX_ORGANIZATIONS,
};
use crate::{error::AppError, extractors::CurrentUser, state::AppState};

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OptionsQuery {
    #[serde(default, deserialize_with = "crate::util::uuid7::deserialize_optional")]
    application_id: Option<Uuid>,
    #[serde(default, deserialize_with = "crate::util::uuid7::deserialize_optional")]
    user_id: Option<Uuid>,
    keyword: Option<String>,
    page: Option<i64>,
    size: Option<i64>,
}

struct Search {
    application_id: Option<Uuid>,
    user_id: Option<Uuid>,
    keyword: String,
    page: i64,
    size: i64,
    offset: i64,
}

impl TryFrom<OptionsQuery> for Search {
    type Error = AppError;

    fn try_from(query: OptionsQuery) -> Result<Self, Self::Error> {
        if query
            .application_id
            .is_some_and(|id| !crate::util::uuid7::is_uuid_v7(id))
            || query
                .user_id
                .is_some_and(|id| !crate::util::uuid7::is_uuid_v7(id))
        {
            return Err(AppError::bad_request("筛选标识不能是空 UUID"));
        }
        let keyword = query.keyword.unwrap_or_default();
        let keyword = keyword.trim();
        if keyword.contains('\0') {
            return Err(AppError::bad_request("搜索关键词不能包含空字符"));
        }
        if keyword.len() > 400 || keyword.chars().count() > 100 {
            return Err(AppError::bad_request("搜索关键词不能超过 100 个字符"));
        }
        let page = query.page.unwrap_or(1);
        let size = query.size.unwrap_or(20);
        if !(1..=1_000_000).contains(&page) || !(1..=50).contains(&size) {
            return Err(AppError::bad_request(
                "page 必须为 1 到 1000000，size 必须为 1 到 50",
            ));
        }
        let offset = page
            .checked_sub(1)
            .and_then(|page| page.checked_mul(size))
            .ok_or_else(|| AppError::bad_request("分页范围无效"))?;
        Ok(Self {
            application_id: query.application_id,
            user_id: query.user_id,
            keyword: keyword.to_owned(),
            page,
            size,
            offset,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct ApplicationOption {
    #[serde(deserialize_with = "crate::util::uuid7::deserialize")]
    id: Uuid,
    code: String,
    name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MemberOption {
    target: Target,
    username: String,
    display_name: String,
    tenant_name: String,
    organization_name: String,
    can_write: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct OptionsResponse {
    applications: Vec<ApplicationOption>,
    records: Vec<MemberOption>,
    total: i64,
    page: i64,
    size: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemberRow {
    #[serde(deserialize_with = "crate::util::uuid7::deserialize")]
    tenant_id: Uuid,
    #[serde(deserialize_with = "crate::util::uuid7::deserialize")]
    organization_id: Uuid,
    #[serde(deserialize_with = "crate::util::uuid7::deserialize")]
    user_id: Uuid,
    #[serde(deserialize_with = "crate::util::uuid7::deserialize")]
    membership_id: Uuid,
    username: String,
    display_name: String,
    tenant_name: String,
    organization_name: String,
}

#[derive(sqlx::FromRow)]
struct Rows {
    oversized: bool,
    applications: Json<Vec<ApplicationOption>>,
    records: Json<Vec<MemberRow>>,
    total: i64,
}

/// The recursive state has at most two variants per organization (expand or
/// exact root). A demand-driven LIMIT therefore detects more than 10000 unique
/// organizations without first expanding an unbounded configured subtree.
/// The candidate joins and all emitted names use this same statement snapshot.
const OPTIONS_SQL: &str = "WITH RECURSIVE managed_tree AS ( \
    SELECT o.id,o.tenant_id,r.expand FROM unnest($1::uuid[],$2::uuid[],$3::bool[]) r(tenant_id,organization_id,expand) \
    JOIN organizations o ON o.id=r.organization_id AND o.tenant_id=r.tenant_id \
    WHERE o.status='ENABLED' AND (o.expires_at IS NULL OR o.expires_at>clock_timestamp()) \
    UNION SELECT o.id,o.tenant_id,p.expand FROM organizations o JOIN managed_tree p ON o.parent_id=p.id \
    WHERE p.expand AND o.tenant_id=p.tenant_id AND o.status='ENABLED' \
    AND (o.expires_at IS NULL OR o.expires_at>clock_timestamp())), \
    bounded_tree AS MATERIALIZED (SELECT * FROM managed_tree LIMIT $13), \
    organization_budget AS (SELECT count(DISTINCT id)>$14 AS oversized FROM bounded_tree), \
    readable_organizations AS MATERIALIZED ( \
    SELECT DISTINCT o.id,o.tenant_id,o.name AS organization_name,t.name AS tenant_name \
    FROM bounded_tree b JOIN organizations o ON o.id=b.id AND o.tenant_id=b.tenant_id \
    JOIN tenants t ON t.id=o.tenant_id AND t.status='ENABLED' \
    AND (t.expires_at IS NULL OR t.expires_at>clock_timestamp()) \
    WHERE ($4 OR (o.tenant_id=$5 AND o.id=ANY($6::uuid[]))) AND org_chain_active(o.id) \
    AND (org_chain_expires_at(o.id) IS NULL OR org_chain_expires_at(o.id)>clock_timestamp())), \
    eligible_members AS NOT MATERIALIZED ( \
    SELECT m.id AS membership_id,m.user_id,m.organization_id,o.tenant_id,u.username, \
    COALESCE(NULLIF(btrim(u.real_name),''),u.username) AS display_name,o.tenant_name,o.organization_name \
    FROM memberships m JOIN readable_organizations o ON o.id=m.organization_id \
    JOIN users u ON u.id=m.user_id AND u.status='ENABLED' \
    WHERE m.member_type='EMPLOYEE' AND m.status='ENABLED' AND ($8::uuid IS NULL OR u.id=$8) \
    AND EXISTS(SELECT 1 FROM tenant_applications ta \
    JOIN organization_applications oa ON oa.application_id=ta.application_id AND oa.organization_id=o.id \
    JOIN applications a ON a.id=ta.application_id AND a.status='ENABLED' AND 'EMPLOYEE'=ANY(a.allowed_member_types) \
    WHERE ta.tenant_id=o.tenant_id AND ta.application_id=ANY($7::uuid[]))), \
    available_applications AS MATERIALIZED ( \
    SELECT a.id,a.code,a.name FROM applications a WHERE a.id=ANY($7::uuid[]) AND a.status='ENABLED' \
    AND 'EMPLOYEE'=ANY(a.allowed_member_types) \
    AND EXISTS (SELECT 1 FROM eligible_members m \
    JOIN tenant_applications ta ON ta.tenant_id=m.tenant_id AND ta.application_id=a.id \
    JOIN organization_applications oa ON oa.organization_id=m.organization_id AND oa.application_id=a.id)), \
    matching_members AS MATERIALIZED ( \
    SELECT m.* FROM eligible_members m \
    JOIN tenant_applications ta ON ta.tenant_id=m.tenant_id AND ta.application_id=$9 \
    JOIN organization_applications oa ON oa.organization_id=m.organization_id AND oa.application_id=$9 \
    WHERE $9::uuid IS NOT NULL AND EXISTS(SELECT 1 FROM available_applications a WHERE a.id=$9) \
    AND ($10='' OR strpos(lower(m.username),lower($10))>0 OR strpos(lower(m.display_name),lower($10))>0)), \
    page_members AS (SELECT * FROM matching_members ORDER BY username,organization_id,membership_id LIMIT $11 OFFSET $12) \
    SELECT (SELECT oversized FROM organization_budget) AS oversized, \
    COALESCE((SELECT jsonb_agg(jsonb_build_object('id',a.id,'code',a.code,'name',a.name) ORDER BY a.code,a.id) \
    FROM available_applications a),'[]'::jsonb) AS applications, \
    COALESCE((SELECT jsonb_agg(jsonb_build_object('tenantId',m.tenant_id,'organizationId',m.organization_id, \
    'userId',m.user_id,'membershipId',m.membership_id,'username',m.username,'displayName',m.display_name, \
    'tenantName',m.tenant_name,'organizationName',m.organization_name) ORDER BY m.username,m.organization_id,m.membership_id) \
    FROM page_members m),'[]'::jsonb) AS records, (SELECT count(*) FROM matching_members) AS total";

pub(super) async fn options(
    state: &AppState,
    actor: &CurrentUser,
    query: OptionsQuery,
) -> Result<OptionsResponse, AppError> {
    let search = Search::try_from(query)?;
    super::preliminary_access(state, actor, super::Operation::Read).await?;
    let mut tx = state.pool.begin().await?;
    let present =
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE id=$1 FOR NO KEY UPDATE")
            .bind(actor.user_id)
            .fetch_optional(&mut *tx)
            .await?;
    if present.is_none() {
        return Err(AppError::unauthorized("会话用户已失效"));
    }
    let initial = store::actor_runtime(&mut tx, actor).await?;
    require_read(&initial)?;
    let roots = readable_roots(&mut tx, state, actor, &initial).await?;
    let root_tenants: Vec<Uuid> = roots.iter().map(|root| root.0).collect();
    let root_organizations: Vec<Uuid> = roots.iter().map(|root| root.1).collect();
    let root_expansions: Vec<bool> = roots.iter().map(|root| root.2).collect();
    let applications: Vec<Uuid> = state
        .config
        .data_grant_policy
        .applications
        .iter()
        .map(|app| app.application_id)
        .collect();
    let rows: Rows = sqlx::query_as(OPTIONS_SQL)
        .bind(root_tenants)
        .bind(root_organizations)
        .bind(root_expansions)
        .bind(initial.platform_root)
        .bind(actor.tenant_id)
        .bind(&initial.subtree)
        .bind(applications)
        .bind(search.user_id)
        .bind(search.application_id)
        .bind(&search.keyword)
        .bind(search.size)
        .bind(search.offset)
        .bind((MAX_ORGANIZATIONS * 2 + 1) as i64)
        .bind(MAX_ORGANIZATIONS as i64)
        .fetch_one(&mut *tx)
        .await?;
    if rows.oversized {
        return Err(AppError::bad_request("管理组织范围超过 10000 项发现上限"));
    }
    let mut ancestor_cache = HashMap::new();
    if initial.platform_root {
        for record in &rows.records.0 {
            if ancestor_cache.contains_key(&record.organization_id) {
                continue;
            }
            let ancestors =
                match store::ancestors(&mut tx, record.tenant_id, record.organization_id).await {
                    Ok(ancestors) => ancestors,
                    Err(AppError::Forbidden(_)) | Err(AppError::NotFound(_)) => Vec::new(),
                    Err(error) => return Err(error),
                };
            ancestor_cache.insert(record.organization_id, ancestors);
        }
    }
    // Recheck after every database wait. If the actor's effective organization
    // boundary moved, return no previously collected names from the old scope.
    let current = store::actor_runtime(&mut tx, actor).await?;
    require_read(&current)?;
    if initial.identity != current.identity
        || initial.platform_root != current.platform_root
        || initial.subtree != current.subtree
        || initial.ancestors != current.ancestors
    {
        return Err(AppError::forbidden("当前只读管理范围已变化，请重新查询"));
    }
    let can_write = current.platform_root
        && current
            .permissions
            .iter()
            .any(|permission| permission == WRITE_PERMISSION);
    let mut records = Vec::with_capacity(rows.records.0.len());
    if let Some(application_id) = search.application_id {
        for record in rows.records.0 {
            let allowed_write = can_write
                && ancestor_cache
                    .get(&record.organization_id)
                    .is_some_and(|ancestors| !ancestors.is_empty())
                && state
                    .config
                    .data_grant_policy
                    .management_scopes
                    .iter()
                    .any(|boundary| {
                        boundary.write
                            && boundary.tenant_id == record.tenant_id
                            && (boundary.organization_id == record.organization_id
                                || (boundary.include_descendants
                                    && ancestor_cache.get(&record.organization_id).is_some_and(
                                        |ancestors| ancestors.contains(&boundary.organization_id),
                                    )))
                    });
            records.push(MemberOption {
                target: Target {
                    tenant_id: record.tenant_id,
                    organization_id: record.organization_id,
                    user_id: record.user_id,
                    membership_id: record.membership_id,
                    application_id,
                },
                username: record.username,
                display_name: record.display_name,
                tenant_name: record.tenant_name,
                organization_name: record.organization_name,
                can_write: allowed_write,
            });
        }
    }
    let response = OptionsResponse {
        applications: rows.applications.0,
        records,
        total: rows.total,
        page: search.page,
        size: search.size,
    };
    ensure_size(&response)?;
    tx.commit().await?;
    Ok(response)
}

fn require_read(actor: &store::ActorRuntime) -> Result<(), AppError> {
    if !actor
        .permissions
        .iter()
        .any(|permission| permission == READ_PERMISSION)
    {
        return Err(AppError::forbidden("缺少独立数据授权查询权限"));
    }
    Ok(())
}

async fn readable_roots(
    tx: &mut Transaction<'_, Postgres>,
    state: &AppState,
    actor: &CurrentUser,
    runtime: &store::ActorRuntime,
) -> Result<BTreeSet<(Uuid, Uuid, bool)>, AppError> {
    let mut roots = BTreeSet::new();
    for boundary in state
        .config
        .data_grant_policy
        .management_scopes
        .iter()
        .filter(|boundary| boundary.read)
    {
        let organization = if runtime.platform_root {
            if !store::policy_root_active(tx, boundary.tenant_id, boundary.organization_id).await? {
                continue;
            }
            boundary.organization_id
        } else {
            if boundary.tenant_id != actor.tenant_id {
                continue;
            }
            if runtime.subtree.contains(&boundary.organization_id) {
                boundary.organization_id
            } else if boundary.include_descendants
                && runtime.ancestors.contains(&boundary.organization_id)
            {
                actor.org_id
            } else {
                continue;
            }
        };
        roots.insert((
            boundary.tenant_id,
            organization,
            boundary.include_descendants,
        ));
    }
    Ok(roots)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_bounds_preserve_literal_unicode_and_use_a_safe_maximum_offset() {
        assert!(matches!(
            Search::try_from(OptionsQuery {
                keyword: Some("text\0query".into()),
                ..OptionsQuery::default()
            }),
            Err(AppError::BadRequest(_))
        ));
        let search = Search::try_from(OptionsQuery {
            keyword: Some("  %_\\  ".into()),
            page: Some(1_000_000),
            size: Some(50),
            ..OptionsQuery::default()
        })
        .unwrap();
        assert_eq!(search.keyword, "%_\\");
        assert_eq!(search.offset, 49_999_950);
        assert!(Search::try_from(OptionsQuery {
            keyword: Some("界".repeat(100)),
            ..OptionsQuery::default()
        })
        .is_ok());
        assert!(matches!(
            Search::try_from(OptionsQuery {
                keyword: Some("界".repeat(101)),
                ..OptionsQuery::default()
            }),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            Search::try_from(OptionsQuery {
                page: Some(i64::MAX),
                size: Some(i64::MAX),
                ..OptionsQuery::default()
            }),
            Err(AppError::BadRequest(_))
        ));
    }
}
