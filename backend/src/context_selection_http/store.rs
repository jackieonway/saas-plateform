use super::authority::SourceProof;
use crate::{error::AppError, util::hash};
use axum::http::{header, HeaderMap};
use base64::{engine::general_purpose::STANDARD, Engine};
use saas_platform_backend::{
    context_selection::{
        ApplicationBinding, Binding, Id, Member, MemberType, Registration, Version,
    },
    context_selection_store::Consumer,
};
use serde::Serialize;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

pub(super) struct Principal {
    pub id: Uuid,
    secret_hash: String,
    purpose: &'static str,
}

impl Principal {
    pub fn distinct_from(&self, secret: Option<&str>) -> bool {
        !secret.is_some_and(|s| equal(&self.secret_hash, &hash::sha256_hex(s)))
    }
}

fn equal(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

fn basic(headers: &HeaderMap) -> Result<(Id, String), AppError> {
    let invalid = || AppError::forbidden("选择服务认证无效");
    if headers.contains_key(header::COOKIE) {
        return Err(invalid());
    }
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values
        .next()
        .and_then(|v| v.to_str().ok())
        .ok_or_else(invalid)?;
    if values.next().is_some() {
        return Err(invalid());
    }
    let value = value
        .strip_prefix("Basic ")
        .filter(|s| s.len() <= 2048)
        .ok_or_else(invalid)?;
    let bytes = STANDARD.decode(value).map_err(|_| invalid())?;
    if STANDARD.encode(&bytes) != value {
        return Err(invalid());
    }
    let raw = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
    let (id, secret) = raw.split_once(':').ok_or_else(invalid)?;
    let id: Id =
        serde_json::from_value(serde_json::Value::String(id.to_owned())).map_err(|_| invalid())?;
    if secret.len() != 64
        || !secret
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid());
    }
    Ok((id, hash::sha256_hex(secret)))
}

pub(super) async fn authenticate(
    pool: &PgPool,
    headers: &HeaderMap,
    client: Id,
) -> Result<Principal, AppError> {
    authenticate_as(pool, headers, client, "CONTEXT_SELECTION_AUTHORITY").await
}

pub(super) async fn authenticate_as(
    pool: &PgPool,
    headers: &HeaderMap,
    client: Id,
    purpose: &'static str,
) -> Result<Principal, AppError> {
    let (id, digest) = basic(headers)?;
    let hash:Option<String>=sqlx::query_scalar("SELECT s.secret_hash FROM context_selection_services s WHERE s.id=$1 AND s.status='ENABLED' AND s.purpose=$3 AND $2=ANY(s.allowed_oidc_clients) AND NOT EXISTS(SELECT 1 FROM oauth_clients c WHERE c.client_secret_hash=s.secret_hash)")
        .bind(id.uuid()).bind(client.uuid()).bind(purpose).fetch_optional(pool).await?;
    if !hash.as_ref().is_some_and(|v| equal(v, &digest)) {
        return Err(AppError::forbidden("选择服务用途或 RP 范围无效"));
    }
    Ok(Principal {
        id: id.uuid(),
        secret_hash: digest,
        purpose,
    })
}

pub(super) async fn revalidate(
    tx: &mut Transaction<'_, Postgres>,
    principal: &Principal,
    client: Id,
) -> Result<(), AppError> {
    let current:Option<String>=sqlx::query_scalar("SELECT s.secret_hash FROM context_selection_services s WHERE s.id=$1 AND s.status='ENABLED' AND s.purpose=$3 AND $2=ANY(s.allowed_oidc_clients) AND NOT EXISTS(SELECT 1 FROM oauth_clients c WHERE c.client_secret_hash=s.secret_hash) FOR SHARE OF s")
        .bind(principal.id).bind(client.uuid()).bind(principal.purpose).fetch_optional(&mut **tx).await?;
    if !current
        .as_ref()
        .is_some_and(|v| equal(v, &principal.secret_hash))
    {
        return Err(AppError::forbidden("选择服务登记已变化"));
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
pub(super) struct Rp {
    client_id: Uuid,
    client_policy_version: i64,
    member_type: String,
    application_id: Option<Uuid>,
    application_policy_version: Option<i64>,
    consumer_service_id: Option<Uuid>,
}

fn id(value: Uuid) -> Result<Id, AppError> {
    Id::new(value).map_err(|_| AppError::Unavailable)
}
fn version(value: i64) -> Result<Version, AppError> {
    Version::new(value).map_err(|_| AppError::Unavailable)
}

impl Rp {
    pub fn consumer(&self) -> Result<Consumer, AppError> {
        Ok(Consumer {
            client_id: id(self.client_id)?,
            service_id: self.consumer_service_id.map(id).transpose()?,
        })
    }
}

pub(super) async fn registration(
    tx: &mut Transaction<'_, Postgres>,
    proof: &SourceProof,
) -> Result<Rp, AppError> {
    let row:Rp=sqlx::query_as("SELECT client_id,client_policy_version,member_type,application_id,application_policy_version,consumer_service_id FROM context_selection_registrations WHERE client_id=$1 AND enabled FOR SHARE")
        .bind(proof.client.id.uuid()).fetch_optional(&mut **tx).await?.ok_or_else(||AppError::forbidden("RP 未登记身份选择用途"))?;
    let purpose = match row.member_type.as_str() {
        "EMPLOYEE" => "PLATFORM_CONSOLE",
        "CUSTOMER" => "CUSTOMER_PORTAL",
        _ => return Err(AppError::Unavailable),
    };
    if row.client_policy_version != proof.client.policy_version.value()
        || proof.client.purpose != purpose
        || proof.client.allowed_member_types != [row.member_type.clone()]
    {
        return Err(AppError::forbidden("RP 策略或成员用途不匹配"));
    }
    if let Some(application) = row.application_id {
        let valid:Option<Uuid>=sqlx::query_scalar("SELECT id FROM applications WHERE id=$1 AND status='ENABLED' AND access_policy_version=$2 AND $3=ANY(allowed_member_types) FOR SHARE")
            .bind(application).bind(row.application_policy_version).bind(&row.member_type).fetch_optional(&mut **tx).await?;
        if valid.is_none() {
            return Err(AppError::forbidden("目标应用策略已失效"));
        }
    } else if row.member_type != "EMPLOYEE" {
        return Err(AppError::forbidden("此用途必须登记目标应用"));
    }
    if let Some(service) = row.consumer_service_id {
        let valid:Option<Uuid>=sqlx::query_scalar("SELECT id FROM context_selection_services WHERE id=$1 AND purpose='CONTEXT_SELECTION_CONSUMER' AND status='ENABLED' AND $2=ANY(allowed_oidc_clients) FOR SHARE")
            .bind(service).bind(row.client_id).fetch_optional(&mut **tx).await?;
        if valid.is_none() {
            return Err(AppError::forbidden("选择消费方已失效"));
        }
    }
    Ok(row)
}

pub(super) fn binding(proof: &SourceProof, rp: &Rp) -> Result<Binding, AppError> {
    Ok(Binding {
        transaction_id: proof.transaction_id,
        source: proof.source.clone(),
        intent: proof.intent.clone(),
        registration: Registration {
            client_id: id(rp.client_id)?,
            client_policy_version: version(rp.client_policy_version)?,
            member_type: match rp.member_type.as_str() {
                "EMPLOYEE" => MemberType::Employee,
                "CUSTOMER" => MemberType::Customer,
                _ => return Err(AppError::Unavailable),
            },
            redirect_uri: proof.client.redirect_uri.clone(),
            application: rp
                .application_id
                .zip(rp.application_policy_version)
                .map(|(v, g)| {
                    Ok::<_, AppError>(ApplicationBinding {
                        id: id(v)?,
                        policy_version: version(g)?,
                    })
                })
                .transpose()?,
        },
    })
}

#[derive(PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Choice {
    #[serde(flatten)]
    pub member: Member,
    tenant_name: String,
    organization_name: String,
    organization_path: String,
}

#[derive(sqlx::FromRow)]
struct Row {
    membership_id: Uuid,
    organization_id: Uuid,
    tenant_id: Uuid,
    identity_version: i64,
    tenant_name: String,
    organization_name: String,
    organization_path: String,
}

pub(super) async fn members(
    tx: &mut Transaction<'_, Postgres>,
    binding: &Binding,
    selected: Option<Id>,
) -> Result<Vec<Choice>, AppError> {
    let member_type = match binding.registration.member_type {
        MemberType::Employee => "EMPLOYEE",
        MemberType::Customer => "CUSTOMER",
    };
    let application = binding
        .registration
        .application
        .as_ref()
        .map(|v| v.id.uuid());
    let rows:Vec<Row>=sqlx::query_as("SELECT m.id AS membership_id,m.organization_id,o.tenant_id,m.identity_version,t.name AS tenant_name,o.name AS organization_name,COALESCE(org_path(o.id),'') AS organization_path FROM memberships m JOIN organizations o ON o.id=m.organization_id JOIN tenants t ON t.id=o.tenant_id WHERE m.user_id=$1 AND m.member_type=$2 AND identity_binding_active($1,t.id,o.id,$3,m.id,$2,m.identity_version) AND ($4::uuid IS NULL OR (EXISTS(SELECT 1 FROM tenant_applications WHERE tenant_id=t.id AND application_id=$4) AND EXISTS(SELECT 1 FROM organization_applications WHERE organization_id=o.id AND application_id=$4))) AND ($5::uuid IS NULL OR m.id=$5) ORDER BY t.name,o.name,m.id LIMIT 257")
        .bind(binding.source.user_id.uuid()).bind(member_type).bind(binding.source.account_version.value()).bind(application).bind(selected.map(Id::uuid)).fetch_all(&mut **tx).await?;
    if rows.len() > 256 {
        return Err(AppError::bad_request("可选身份过多，请收窄 RP 应用范围"));
    }
    rows.into_iter()
        .map(|r| {
            Ok(Choice {
                member: Member {
                    user_id: binding.source.user_id,
                    tenant_id: id(r.tenant_id)?,
                    organization_id: id(r.organization_id)?,
                    membership_id: id(r.membership_id)?,
                    member_type: binding.registration.member_type,
                    identity_version: version(r.identity_version)?,
                },
                tenant_name: r.tenant_name,
                organization_name: r.organization_name,
                organization_path: r.organization_path,
            })
        })
        .collect()
}

pub(super) async fn lock_account(
    tx: &mut Transaction<'_, Postgres>,
    proof: &SourceProof,
) -> Result<(), AppError> {
    let found: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM users WHERE id=$1 AND status='ENABLED' AND session_version=$2 FOR SHARE",
    )
    .bind(proof.source.user_id.uuid())
    .bind(proof.source.account_version.value())
    .fetch_optional(&mut **tx)
    .await?;
    if found.is_none() {
        return Err(AppError::unauthorized("原账号代际已失效"));
    }
    Ok(())
}

pub(super) async fn lock_member(
    tx: &mut Transaction<'_, Postgres>,
    member: Id,
) -> Result<(), AppError> {
    let found: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM memberships WHERE id=$1 FOR SHARE")
            .bind(member.uuid())
            .fetch_optional(&mut **tx)
            .await?;
    if found.is_none() {
        return Err(AppError::forbidden("成员已删除"));
    }
    Ok(())
}
