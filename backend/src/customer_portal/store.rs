//! Platform persistence contains immutable source bindings, never OIDC tokens.

use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::{Executor, PgPool, Postgres};
use uuid::Uuid;

use super::{authority::Source, equal, invalid};
use crate::{
    error::AppError,
    state::AppState,
    util::{hash, uuid7},
};

#[derive(Clone, PartialEq, sqlx::FromRow)]
pub(super) struct Registration {
    pub id: Uuid,
    pub application_id: Uuid,
    pub client_policy_version: i64,
    pub application_policy_version: i64,
    application_code: String,
    application_name: String,
    client_id: String,
    secret_hash: String,
}

pub(super) async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Registration, AppError> {
    let caller = crate::confidential_clients::authenticate(state, headers).await?;
    let (_, secret) = crate::confidential_clients::parse_basic_auth(headers).ok_or_else(invalid)?;
    let (client, application) = state
        .config
        .customer_profile_client_ref
        .zip(state.config.customer_profile_application_id)
        .ok_or(AppError::Unavailable)?;
    let row: Option<Registration> = sqlx::query_as(
        "SELECT c.id,c.application_id,c.access_policy_version AS client_policy_version, \
         a.access_policy_version AS application_policy_version,a.code AS application_code, \
         a.name AS application_name,c.client_id,COALESCE(c.client_secret_hash,'') AS secret_hash \
         FROM oauth_clients c JOIN applications a ON a.id=c.application_id \
         WHERE c.id=$1 AND c.application_id=$2 AND c.client_id=$3 \
         AND c.client_type='CONFIDENTIAL' AND c.subject_mode='USER' AND c.status='ENABLED' \
         AND c.allowed_member_types=ARRAY['CUSTOMER']::text[] \
         AND 'profile'=ANY(regexp_split_to_array(trim(COALESCE(c.scopes,'')),'[[:space:]]*,[[:space:]]*')) \
         AND a.status='ENABLED' AND 'CUSTOMER'=ANY(a.allowed_member_types)",
    ).bind(client).bind(application).bind(&caller.client_id).fetch_optional(&state.pool).await.map_err(|_|AppError::Unavailable)?;
    row.filter(|row| {
        row.application_id == caller.application_id
            && equal(&row.secret_hash, &hash::sha256_hex(&secret))
    })
    .ok_or_else(invalid)
}

#[derive(Clone, PartialEq, sqlx::FromRow)]
pub(super) struct Member {
    pub membership_id: Uuid,
    user_id: Uuid,
    pub organization_id: Uuid,
    pub tenant_id: Uuid,
    pub identity_version: i64,
    organization_name: String,
    tenant_name: String,
    org_expires: Option<DateTime<Utc>>,
    tenant_expires: Option<DateTime<Utc>>,
}

impl Member {
    pub fn choice(&self) -> Value {
        json!({"membershipId":self.membership_id,"userId":self.user_id,"memberType":"CUSTOMER",
            "identityVersion":self.identity_version.to_string(),"organizationId":self.organization_id,
            "organizationName":self.organization_name,"tenantId":self.tenant_id,"tenantName":self.tenant_name})
    }

    pub fn context(&self, source: &Source, registration: &Registration) -> Value {
        json!({"contractVersion":"customer-profile/1","purpose":"CUSTOMER_PROFILE","userId":self.user_id,
            "membershipId":self.membership_id,"memberType":"CUSTOMER","identityVersion":self.identity_version.to_string(),
            "accountVersion":source.account_version.to_string(),"tenant":{"id":self.tenant_id,"name":self.tenant_name},
            "organization":{"id":self.organization_id,"name":self.organization_name},
            "application":{"id":registration.application_id,"code":registration.application_code,
                "name":registration.application_name,"entryPath":"/api/customer/profile"}})
    }
}

const MEMBERS: &str = "SELECT m.id AS membership_id,m.user_id,m.organization_id,o.tenant_id,m.identity_version, \
    o.name AS organization_name,t.name AS tenant_name,org_chain_expires_at(o.id) AS org_expires,t.expires_at AS tenant_expires \
    FROM memberships m JOIN organizations o ON o.id=m.organization_id JOIN tenants t ON t.id=o.tenant_id \
    JOIN applications a ON a.id=$3 JOIN oauth_clients c ON c.id=$4 AND c.application_id=a.id \
    WHERE m.user_id=$1 AND m.member_type='CUSTOMER' \
    AND identity_binding_active($1,t.id,o.id,$2,m.id,'CUSTOMER',m.identity_version) \
    AND c.status='ENABLED' AND c.client_type='CONFIDENTIAL' AND c.subject_mode='USER' \
    AND c.allowed_member_types=ARRAY['CUSTOMER']::text[] AND c.access_policy_version=$5 \
    AND a.status='ENABLED' AND a.access_policy_version=$6 AND 'CUSTOMER'=ANY(a.allowed_member_types) \
    AND 'profile'=ANY(regexp_split_to_array(trim(COALESCE(c.scopes,'')),'[[:space:]]*,[[:space:]]*')) \
    AND EXISTS(SELECT 1 FROM tenant_applications WHERE tenant_id=t.id AND application_id=a.id) \
    AND EXISTS(SELECT 1 FROM organization_applications WHERE organization_id=o.id AND application_id=a.id)";

pub(super) async fn members(
    pool: &PgPool,
    registration: &Registration,
    source: &Source,
) -> Result<Vec<Member>, AppError> {
    sqlx::query_as(&format!("{MEMBERS} ORDER BY m.created_at,m.id"))
        .bind(source.user_id)
        .bind(source.account_version)
        .bind(registration.application_id)
        .bind(registration.id)
        .bind(registration.client_policy_version)
        .bind(registration.application_policy_version)
        .fetch_all(pool)
        .await
        .map_err(|_| AppError::Unavailable)
}

pub(super) async fn member<'e, E: Executor<'e, Database = Postgres>>(
    executor: E,
    registration: &Registration,
    source: &Source,
    id: Uuid,
) -> Result<Option<Member>, AppError> {
    sqlx::query_as(&format!("{MEMBERS} AND m.id=$7"))
        .bind(source.user_id)
        .bind(source.account_version)
        .bind(registration.application_id)
        .bind(registration.id)
        .bind(registration.client_policy_version)
        .bind(registration.application_policy_version)
        .bind(id)
        .fetch_optional(executor)
        .await
        .map_err(|_| AppError::Unavailable)
}

#[derive(Clone, PartialEq, sqlx::FromRow)]
pub(super) struct Context {
    pub id: Uuid,
    issuer: String,
    oidc_client_id: Uuid,
    oidc_grant_id: Uuid,
    global_session_id: Uuid,
    user_id: Uuid,
    account_version: i64,
    oidc_client_policy_version: i64,
    auth_time: i64,
    source_expires_at: DateTime<Utc>,
    profile_client_ref: Uuid,
    profile_client_policy_version: i64,
    application_id: Uuid,
    application_policy_version: i64,
    tenant_id: Uuid,
    organization_id: Uuid,
    pub membership_id: Uuid,
    member_type: String,
    identity_version: i64,
    pub expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
}

impl Context {
    pub fn require_source(
        &self,
        registration: &Registration,
        source: &Source,
    ) -> Result<(), AppError> {
        if self.issuer != source.issuer
            || self.oidc_client_id != source.client_id
            || self.oidc_grant_id != source.grant_id
            || self.global_session_id != source.global_session_id
            || self.user_id != source.user_id
            || self.account_version != source.account_version
            || self.oidc_client_policy_version != source.client_policy_version
            || self.auth_time != source.auth_time
            || self.source_expires_at != source.expires_at
            || self.profile_client_ref != registration.id
            || self.profile_client_policy_version != registration.client_policy_version
            || self.application_id != registration.application_id
            || self.application_policy_version != registration.application_policy_version
            || self.member_type != "CUSTOMER"
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub fn require_member(&self, member: &Member) -> Result<(), AppError> {
        if self.membership_id != member.membership_id
            || self.organization_id != member.organization_id
            || self.tenant_id != member.tenant_id
            || self.identity_version != member.identity_version
            || self.user_id != member.user_id
        {
            return Err(invalid());
        }
        Ok(())
    }
}

pub(super) async fn context(
    pool: &PgPool,
    token: &str,
    include_retired: bool,
) -> Result<Option<Context>, AppError> {
    let raw = token
        .strip_prefix("cpc_")
        .filter(|raw| super::canonical_secret(raw))
        .ok_or_else(invalid)?;
    if raw.len() != 64 {
        return Err(invalid());
    }
    sqlx::query_as("SELECT * FROM customer_portal_contexts WHERE token_hash=$1 \
        AND ($2 OR (revoked_at IS NULL AND expires_at>clock_timestamp() AND source_expires_at>clock_timestamp()))")
        .bind(hash::sha256_hex(token)).bind(include_retired).fetch_optional(pool).await.map_err(|_|AppError::Unavailable)
}

pub(super) async fn revalidate(
    pool: &PgPool,
    registration: &Registration,
    source: &Source,
    token: &str,
    original: &Context,
    original_member: &Member,
) -> Result<(), AppError> {
    let current = context(pool, token, false).await?.ok_or_else(invalid)?;
    current.require_source(registration, source)?;
    let member = member(pool, registration, source, original.membership_id)
        .await?
        .ok_or_else(invalid)?;
    current.require_member(&member)?;
    if &current != original || &member != original_member {
        return Err(invalid());
    }
    let fresh: bool = sqlx::query_scalar(
        "SELECT $1::timestamptz>clock_timestamp() AND $2::timestamptz>clock_timestamp()",
    )
    .bind(source.expires_at)
    .bind(current.expires_at)
    .fetch_one(pool)
    .await
    .map_err(|_| AppError::Unavailable)?;
    if !fresh {
        return Err(invalid());
    }
    Ok(())
}

/// Last, single-statement checkpoint after all resource/authority awaits. The
/// authentication-owned boolean can only reject; two online proofs are still
/// mandatory. Local exact member/client/app bindings share this fresh snapshot.
pub(super) async fn final_proof(
    pool: &PgPool,
    registration: &Registration,
    source: &Source,
    access: &str,
    members: &[Member],
    context: Option<(&str, &Context)>,
) -> Result<(), AppError> {
    let ids = members
        .iter()
        .map(|member| member.membership_id)
        .collect::<Vec<_>>();
    let organizations = members
        .iter()
        .map(|member| member.organization_id)
        .collect::<Vec<_>>();
    let tenants = members
        .iter()
        .map(|member| member.tenant_id)
        .collect::<Vec<_>>();
    let versions = members
        .iter()
        .map(|member| member.identity_version)
        .collect::<Vec<_>>();
    let active:bool=sqlx::query_scalar(
        "SELECT customer_portal_account_source_active($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) \
         AND EXISTS(SELECT 1 FROM oauth_clients c JOIN applications a ON a.id=c.application_id \
             WHERE c.id=$15 AND a.id=$16 AND c.status='ENABLED' AND c.client_type='CONFIDENTIAL' \
             AND c.subject_mode='USER' AND c.allowed_member_types=ARRAY['CUSTOMER']::text[] \
             AND c.access_policy_version=$17 AND a.access_policy_version=$18 \
             AND c.client_id=$19 AND c.client_secret_hash=$20 AND a.status='ENABLED' \
             AND 'CUSTOMER'=ANY(a.allowed_member_types) \
             AND 'profile'=ANY(regexp_split_to_array(trim(COALESCE(c.scopes,'')),'[[:space:]]*,[[:space:]]*'))) \
         AND NOT EXISTS(SELECT 1 FROM unnest($11::uuid[],$12::uuid[],$13::uuid[],$14::bigint[]) \
             AS e(member_id,organization_id,tenant_id,identity_version) \
             WHERE NOT identity_binding_active($6,e.tenant_id,e.organization_id,$7,e.member_id,'CUSTOMER',e.identity_version) \
             OR NOT EXISTS(SELECT 1 FROM tenant_applications WHERE tenant_id=e.tenant_id AND application_id=$16) \
             OR NOT EXISTS(SELECT 1 FROM organization_applications WHERE organization_id=e.organization_id AND application_id=$16)) \
         AND ($21::uuid IS NULL OR EXISTS(SELECT 1 FROM customer_portal_contexts ctx \
             WHERE ctx.id=$21 AND ctx.token_hash=$22 AND ctx.revoked_at IS NULL \
             AND ctx.expires_at>clock_timestamp() AND ctx.source_expires_at=$10 \
             AND ctx.issuer=$2 AND ctx.oidc_client_id=$3 AND ctx.oidc_grant_id=$4 \
             AND ctx.global_session_id=$5 AND ctx.user_id=$6 AND ctx.account_version=$7 \
             AND ctx.oidc_client_policy_version=$8 AND ctx.auth_time=$9 \
             AND ctx.profile_client_ref=$15 AND ctx.application_id=$16 \
             AND ctx.profile_client_policy_version=$17 AND ctx.application_policy_version=$18 \
             AND ctx.membership_id=ANY($11) AND ctx.organization_id=ANY($12) AND ctx.tenant_id=ANY($13) \
             AND ctx.identity_version=ANY($14) AND ctx.member_type='CUSTOMER'))"
    ).bind(hash::sha256_hex(access)).bind(&source.issuer).bind(source.client_id).bind(source.grant_id)
        .bind(source.global_session_id).bind(source.user_id).bind(source.account_version).bind(source.client_policy_version)
        .bind(source.auth_time).bind(source.expires_at).bind(ids).bind(organizations).bind(tenants).bind(versions)
        .bind(registration.id).bind(registration.application_id).bind(registration.client_policy_version)
        .bind(registration.application_policy_version).bind(&registration.client_id).bind(&registration.secret_hash)
        .bind(context.map(|(_,record)|record.id)).bind(context.map(|(token,_)|hash::sha256_hex(token)))
        .fetch_one(pool).await.map_err(|_|AppError::Unavailable)?;
    if !active {
        return Err(invalid());
    }
    Ok(())
}

pub(super) async fn issue(
    state: &AppState,
    registration: &Registration,
    source: &Source,
    membership: Uuid,
    organization: Uuid,
    version: i64,
) -> Result<(String, Context, Member), AppError> {
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| AppError::Unavailable)?;
    // Match membership/account mutation ordering. No authority call is made
    // under these locks; the caller rechecks source and exact local generation.
    for (table, id) in [
        ("users", source.user_id),
        ("applications", registration.application_id),
        ("oauth_clients", registration.id),
    ] {
        sqlx::query(&format!("SELECT id FROM {table} WHERE id=$1 FOR UPDATE"))
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|_| AppError::Unavailable)?;
    }
    sqlx::query("SELECT id FROM memberships WHERE id=$1 FOR SHARE")
        .bind(membership)
        .execute(&mut *tx)
        .await
        .map_err(|_| AppError::Unavailable)?;
    let member = member(&mut *tx, registration, source, membership)
        .await?
        .filter(|member| {
            member.organization_id == organization && member.identity_version == version
        })
        .ok_or_else(|| AppError::forbidden("所选客户成员、组织或原成员版本不可用"))?;
    let mut expires = source.expires_at;
    if let Some(end) = member.org_expires {
        expires = expires.min(end);
    }
    if let Some(end) = member.tenant_expires {
        expires = expires.min(end);
    }
    let id = uuid7::new_uuid();
    let mut random = [0_u8; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut random)
        .map_err(|_| AppError::Unavailable)?;
    let token = format!("cpc_{}", hex::encode(random));
    let record: Context=sqlx::query_as("INSERT INTO customer_portal_contexts \
        (id,token_hash,issuer,oidc_client_id,oidc_grant_id,global_session_id,user_id,account_version,oidc_client_policy_version,auth_time,source_expires_at, \
        profile_client_ref,profile_client_policy_version,application_id,application_policy_version,tenant_id,organization_id,membership_id,member_type,identity_version,expires_at) \
        SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,'CUSTOMER',$19,$20 \
        WHERE $20::timestamptz>clock_timestamp() RETURNING *")
        .bind(id).bind(hash::sha256_hex(&token)).bind(&source.issuer).bind(source.client_id).bind(source.grant_id)
        .bind(source.global_session_id).bind(source.user_id).bind(source.account_version).bind(source.client_policy_version)
        .bind(source.auth_time).bind(source.expires_at).bind(registration.id).bind(registration.client_policy_version)
        .bind(registration.application_id).bind(registration.application_policy_version).bind(member.tenant_id)
        .bind(member.organization_id).bind(member.membership_id).bind(member.identity_version).bind(expires)
        .fetch_optional(&mut *tx).await.map_err(|_|AppError::Unavailable)?.ok_or_else(invalid)?;
    tx.commit().await.map_err(|_| AppError::Unavailable)?;
    Ok((token, record, member))
}
