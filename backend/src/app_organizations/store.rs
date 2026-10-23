//! A bounded directory projection in one PostgreSQL statement snapshot.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use super::Search;
use crate::error::AppError;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Organization {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub code: String,
    pub organization_type: String,
    pub status: String,
    pub path: String,
    pub province: String,
    pub city: String,
    pub district: String,
    pub township: String,
    pub village: String,
    pub address: String,
    pub contact_name: String,
    pub contact_phone: String,
    pub contact_email: String,
    pub updated_at: DateTime<Utc>,
    pub authority_expires_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize, Serialize)]
pub(super) struct Projection {
    pub oversized: bool,
    pub total: i64,
    pub records: Vec<Organization>,
}

pub(super) async fn load(
    pool: &PgPool,
    tenant: Uuid,
    application: Uuid,
    allowed: Option<&[Uuid]>,
    search: &Search,
    target: Option<Uuid>,
) -> Result<Projection, AppError> {
    let value: serde_json::Value = sqlx::query_scalar(
        r#"WITH candidates AS MATERIALIZED (
          SELECT o.id FROM organizations o
          JOIN tenants t ON t.id=o.tenant_id AND t.status='ENABLED'
            AND (t.expires_at IS NULL OR t.expires_at>clock_timestamp())
          JOIN applications a ON a.id=$2 AND a.status='ENABLED'
          JOIN tenant_applications ta ON ta.tenant_id=t.id AND ta.application_id=a.id
          JOIN organization_applications oa ON oa.organization_id=o.id AND oa.application_id=a.id
          WHERE o.tenant_id=$1 AND ($3::uuid[] IS NULL OR o.id=ANY($3))
            AND ($4::uuid IS NULL OR o.id=$4) AND o.organization_type='STORE'
            AND org_chain_active(o.id)
            AND (org_chain_expires_at(o.id) IS NULL OR org_chain_expires_at(o.id)>clock_timestamp())
            AND ($5='' OR strpos(lower(o.name),lower($5))>0 OR strpos(lower(COALESCE(o.code,'')),lower($5))>0)
          LIMIT 10001
        ), page AS (
          SELECT o.id,o.tenant_id AS "tenantId",o.parent_id AS "parentId",o.name,COALESCE(o.code,'') AS code,
            o.organization_type AS "organizationType",o.status,COALESCE(org_path(o.id),'') AS path,
            o.province,o.city,o.district,o.township,o.village,o.address,
            o.contact_name AS "contactName",o.contact_phone AS "contactPhone",o.contact_email AS "contactEmail",
            o.updated_at AS "updatedAt",LEAST(t.expires_at,org_chain_expires_at(o.id)) AS "authorityExpiresAt"
          FROM candidates c JOIN organizations o ON o.id=c.id JOIN tenants t ON t.id=o.tenant_id
          ORDER BY o.sort_order,o.name,o.id LIMIT $6 OFFSET $7
        ) SELECT jsonb_build_object('oversized',(SELECT count(*)>10000 FROM candidates),
          'total',(SELECT count(*) FROM candidates),
          'records',COALESCE((SELECT jsonb_agg(to_jsonb(page)) FROM page),'[]'::jsonb))"#,
    )
    .bind(tenant)
    .bind(application)
    .bind(allowed)
    .bind(target)
    .bind(&search.keyword)
    .bind(search.size)
    .bind((search.page - 1) * search.size)
    .fetch_one(pool)
    .await?;
    let result: Projection = serde_json::from_value(value)
        .map_err(|_| AppError::Server(anyhow::anyhow!("ORGANIZATION_LOOKUP_PROJECTION_INVALID")))?;
    if result.oversized {
        return Err(AppError::bad_request(
            "组织目录超过 10000 项上限，请收窄查询",
        ));
    }
    Ok(result)
}
