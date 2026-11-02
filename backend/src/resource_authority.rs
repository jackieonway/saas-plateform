//! Canonical OAuth resource boundary. Public `sub` is never a database key.
use crate::{
    app_token::AppTokenInfo,
    confidential_clients::Caller,
    error::AppError,
    state::AppState,
    util::{hash, uuid7::is_uuid_v7},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub(crate) struct Resource {
    pub application_id: Uuid,
    pub resource_code: String,
    pub action_code: String,
    pub canonical_resource: String,
    pub audience: String,
    pub oauth_scope: String,
    pub application_policy_version: i64,
}

pub(crate) async fn resource(
    state: &AppState,
    caller: &Caller,
    code: &str,
    resource: &str,
    action: &str,
) -> Result<Resource, AppError> {
    if caller.purpose != "APP_RESOURCE"
        || caller.audience != code
        || !caller
            .allowed_operations
            .iter()
            .any(|operation| operation == &format!("{resource}:{action}"))
    {
        return Err(AppError::forbidden("资源服务用途、受众或动作未获登记"));
    }
    sqlx::query_as("SELECT b.application_id,b.resource_code,b.action_code,b.canonical_resource,b.audience,b.oauth_scope,a.access_policy_version AS application_policy_version FROM platform_resource_bindings b JOIN applications a ON a.id=b.application_id AND a.status='ENABLED' WHERE b.application_id=$1 AND a.code=$2 AND b.resource_code=$3 AND b.action_code=$4 AND b.enabled AND b.audience=$5")
        .bind(caller.application_id).bind(code).bind(resource).bind(action).bind(&caller.audience)
        .fetch_optional(&state.pool).await.map_err(|_|AppError::Unavailable)?
        .ok_or_else(||AppError::forbidden("目标资源、scope 或 audience 未登记"))
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Source {
    pub grant_id: Uuid,
    pub client_policy_version: String,
    pub application_projection_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global_session_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_time: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_ref: Option<Uuid>,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Context {
    pub membership_id: Uuid,
    pub member_type: String,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Proof {
    pub contract_version: String,
    pub issuer: String,
    pub token_use: String,
    pub subject_mode: String,
    pub client_id: Uuid,
    pub application_id: Uuid,
    pub resource: String,
    pub audience: String,
    pub scope: Vec<String>,
    pub expires_at: DateTime<Utc>,
    pub source: Source,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<Context>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_version: Option<String>,
}

fn invalid() -> AppError {
    AppError::unauthorized("统一资源凭据或原来源已失效")
}
pub(crate) fn version(value: &str) -> Result<i64, AppError> {
    if value.is_empty() || value.starts_with('0') || !value.bytes().all(|c| c.is_ascii_digit()) {
        return Err(invalid());
    }
    value
        .parse::<i64>()
        .ok()
        .filter(|v| *v >= 1)
        .ok_or_else(invalid)
}

impl Proof {
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, AppError> {
        let value: Value = serde_json::from_slice(bytes).map_err(|_| AppError::Unavailable)?;
        if value["subjectMode"] == "MACHINE" {
            let source = value["source"].as_object().ok_or(AppError::Unavailable)?;
            if value
                .as_object()
                .is_some_and(|o| o.contains_key("sub") || o.contains_key("context"))
                || source.len() != 3
                || ![
                    "grantId",
                    "clientPolicyVersion",
                    "applicationProjectionVersion",
                ]
                .into_iter()
                .all(|key| source.contains_key(key))
            {
                return Err(AppError::Unavailable);
            }
        } else if value["subjectMode"] == "USER"
            && value
                .as_object()
                .is_some_and(|o| o.contains_key("credentialVersion"))
        {
            return Err(AppError::Unavailable);
        }
        serde_json::from_slice(bytes).map_err(|_| AppError::Unavailable)
    }
    pub(crate) fn validate(&self, issuer: &str, resource: &Resource) -> Result<(), AppError> {
        if self.contract_version != "resource-access-proof/1"
            || self.issuer != issuer
            || self.token_use != "access_token"
            || self.resource != resource.canonical_resource
            || self.audience != resource.audience
            || !self.scope.contains(&resource.oauth_scope)
            || self.scope.is_empty()
            || self.scope.len() > 128
            || self
                .scope
                .iter()
                .any(|scope| scope.is_empty() || scope.chars().any(char::is_whitespace))
            || self
                .scope
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.scope.len()
            || self.expires_at <= Utc::now()
            || ![self.client_id, self.application_id, self.source.grant_id]
                .into_iter()
                .all(is_uuid_v7)
        {
            return Err(invalid());
        }
        version(&self.source.client_policy_version)?;
        version(&self.source.application_projection_version)?;
        match self.subject_mode.as_str() {
            "USER" => {
                if self.credential_version.is_some()
                    || !self
                        .sub
                        .as_ref()
                        .is_some_and(|sub| !sub.is_empty() && sub.len() <= 255)
                    || !self.source.global_session_id.is_some_and(is_uuid_v7)
                    || !self.source.user_id.is_some_and(is_uuid_v7)
                    || !self.source.context_ref.is_some_and(is_uuid_v7)
                    || !self
                        .source
                        .auth_time
                        .is_some_and(|time| time > 0 && time <= Utc::now().timestamp())
                {
                    return Err(invalid());
                }
                version(self.source.account_version.as_deref().ok_or_else(invalid)?)?;
                let context = self.context.as_ref().ok_or_else(invalid)?;
                if !is_uuid_v7(context.membership_id)
                    || !matches!(context.member_type.as_str(), "EMPLOYEE" | "CUSTOMER")
                {
                    return Err(invalid());
                }
            }
            "MACHINE" => {
                version(self.credential_version.as_deref().ok_or_else(invalid)?)?;
                if self.sub.is_some()
                    || self.context.is_some()
                    || self.source.global_session_id.is_some()
                    || self.source.user_id.is_some()
                    || self.source.account_version.is_some()
                    || self.source.auth_time.is_some()
                    || self.source.context_ref.is_some()
                {
                    return Err(invalid());
                }
            }
            _ => return Err(invalid()),
        }
        Ok(())
    }
}

pub(crate) async fn verify(
    state: &AppState,
    token: &str,
    resource: &Resource,
) -> Result<Proof, AppError> {
    if token.is_empty() || token.len() > 8185 || token.chars().any(char::is_whitespace) {
        return Err(invalid());
    }
    let config = &state.config;
    let client = config
        .identity_http_client
        .as_ref()
        .ok_or(AppError::Unavailable)?;
    let origin = config
        .identity_service_url
        .as_deref()
        .ok_or(AppError::Unavailable)?;
    let issuer = config.oidc_issuer.as_deref().ok_or(AppError::Unavailable)?;
    let id = config
        .resource_proof_service_client_id
        .as_deref()
        .ok_or(AppError::Unavailable)?;
    let secret = config
        .resource_proof_service_client_secret
        .as_deref()
        .ok_or(AppError::Unavailable)?;
    let mut response=client.post(format!("{origin}/internal/identity/auth/resource-access-proof"))
        .basic_auth(id,Some(secret)).header("accept","application/json").header("content-type","application/json")
        .body(serde_json::to_vec(&json!({"contractVersion":"resource-access-proof-request/1","token":token,"resource":resource.canonical_resource,"scope":resource.oauth_scope,"audience":resource.audience})).map_err(|_|AppError::Unavailable)?)
        .send().await.map_err(|_|AppError::Unavailable)?;
    match response.status().as_u16() {
        200 => {}
        400 | 401 | 403 => return Err(invalid()),
        _ => return Err(AppError::Unavailable),
    }
    if !response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(';').next() == Some("application/json"))
    {
        return Err(AppError::Unavailable);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| AppError::Unavailable)? {
        if bytes.len() + chunk.len() > 32768 {
            return Err(AppError::Unavailable);
        }
        bytes.extend_from_slice(&chunk);
    }
    let proof = Proof::decode(&bytes)?;
    proof.validate(issuer, resource)?;
    Ok(proof)
}

/// Select only by the authoritative internal digest registry. Revoked and
/// malformed known capabilities stay in this domain and cannot fall through.
pub(crate) async fn is_internal(state: &AppState, token: &str) -> Result<bool, AppError> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM platform_app_capabilities WHERE token_hash=$1)")
        .bind(hash::sha256_hex(token))
        .fetch_one(&state.pool)
        .await
        .map_err(|_| AppError::Unavailable)
}

pub(crate) async fn user_binding(
    state: &AppState,
    proof: &Proof,
    resource: &Resource,
) -> Result<AppTokenInfo, AppError> {
    use saas_platform_backend::context_selection::{MemberType, Selection};
    if proof.subject_mode != "USER" {
        return Err(invalid());
    }
    let context = proof.context.as_ref().ok_or_else(invalid)?;
    if context.member_type != "EMPLOYEE" {
        return Err(AppError::forbidden("员工资源拒绝客户身份"));
    }
    let raw: Option<String> = sqlx::query_scalar(
        "SELECT selection::text FROM context_selections selected \
         JOIN context_selection_registrations rp ON rp.client_id=selected.client_id AND rp.enabled \
           AND rp.client_registry='AUTH' AND rp.client_policy_version=$3 AND rp.member_type=$4 \
           AND rp.application_id=$5 \
         JOIN applications source_app ON source_app.id=rp.application_id AND source_app.status='ENABLED' \
           AND source_app.access_policy_version=rp.application_policy_version AND $4=ANY(source_app.allowed_member_types) \
         WHERE selected.transaction_id=$1 AND selected.client_id=$2 AND selected.consumed_at IS NOT NULL \
           AND selected.resource_revoked_at IS NULL",
    )
    .bind(proof.source.context_ref)
    .bind(proof.client_id)
    .bind(version(&proof.source.client_policy_version)?)
    .bind(&context.member_type)
    .bind(proof.application_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|_| AppError::Unavailable)?;
    let selected: Selection =
        serde_json::from_str(&raw.ok_or_else(invalid)?).map_err(|_| AppError::Unavailable)?;
    let source = &selected.binding.source;
    let member = &selected.member;
    let account_version = version(
        proof
            .source
            .account_version
            .as_deref()
            .ok_or_else(invalid)?,
    )?;
    if source.issuer != proof.issuer
        || Some(source.user_id.uuid()) != proof.source.user_id
        || Some(source.global_session_id.uuid()) != proof.source.global_session_id
        || source.account_version.value() != account_version
        || Some(source.auth_time.timestamp()) != proof.source.auth_time
        || selected.binding.registration.client_id.uuid() != proof.client_id
        || selected.binding.registration.client_policy_version.value()
            != version(&proof.source.client_policy_version)?
        || member.user_id != source.user_id
        || member.membership_id.uuid() != context.membership_id
        || member.member_type != MemberType::Employee
        || selected.binding.registration.member_type != MemberType::Employee
        || !selected
            .binding
            .registration
            .application
            .as_ref()
            .is_some_and(|application| application.id.uuid() == proof.application_id)
    {
        return Err(invalid());
    }
    let active:bool=sqlx::query_scalar("SELECT identity_binding_active($1,$2,$3,$4,$5,'EMPLOYEE',$6) AND EXISTS(SELECT 1 FROM applications a JOIN tenant_applications ta ON ta.application_id=a.id AND ta.tenant_id=$2 JOIN organization_applications oa ON oa.application_id=a.id AND oa.organization_id=$3 WHERE a.id=$7 AND a.access_policy_version=$8 AND a.status='ENABLED' AND 'EMPLOYEE'=ANY(a.allowed_member_types)) AND EXISTS(SELECT 1 FROM applications a JOIN tenant_applications ta ON ta.application_id=a.id AND ta.tenant_id=$2 JOIN organization_applications oa ON oa.application_id=a.id AND oa.organization_id=$3 WHERE a.id=$9 AND a.access_policy_version=$10 AND a.status='ENABLED' AND 'EMPLOYEE'=ANY(a.allowed_member_types))")
        .bind(member.user_id.uuid()).bind(member.tenant_id.uuid()).bind(member.organization_id.uuid()).bind(account_version).bind(member.membership_id.uuid()).bind(member.identity_version.value()).bind(resource.application_id).bind(resource.application_policy_version)
        .bind(proof.application_id).bind(selected.binding.registration.application.as_ref().ok_or_else(invalid)?.policy_version.value())
        .fetch_one(&state.pool).await.map_err(|_|AppError::Unavailable)?;
    if !active {
        return Err(invalid());
    }
    Ok(AppTokenInfo {
        user_id: member.user_id.uuid(),
        client_id: proof.client_id.to_string(),
        tenant_id: member.tenant_id.uuid(),
        organization_id: member.organization_id.uuid(),
        application_id: resource.application_id,
        audience: resource.audience.clone(),
        jti: proof.source.grant_id,
        scope: proof.scope.join(" "),
        expires_at: proof.expires_at,
        session_id: proof.source.global_session_id.ok_or_else(invalid)?,
        membership_id: member.membership_id.uuid(),
        member_type: "EMPLOYEE".into(),
        identity_version: member.identity_version.value(),
        account_version,
    })
}

/// A machine result is a separate DTO. It never contains a fabricated person.
pub(crate) async fn machine_delegation(
    state: &AppState,
    proof: &Proof,
    resource: &Resource,
) -> Result<Value, AppError> {
    if proof.subject_mode != "MACHINE" {
        return Err(invalid());
    }
    let version = version(proof.credential_version.as_deref().ok_or_else(invalid)?)?;
    let values:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('delegationId',d.id,'version',d.version::text,'targetApplicationId',d.application_id,'tenantId',d.tenant_id,'organizationId',d.organization_id,'resource',d.resource,'action',d.action,'scope',d.scope_type,'organizationIds',d.organization_ids,'expiresAt',LEAST(d.expires_at,t.expires_at,CASE WHEN d.organization_id IS NULL THEN NULL ELSE org_chain_expires_at(d.organization_id) END,(SELECT min(org_chain_expires_at(allowed.id)) FROM unnest(d.organization_ids) allowed(id)),$8::timestamptz)) FROM platform_machine_delegations d JOIN applications a ON a.id=d.application_id AND a.status='ENABLED' JOIN tenants t ON t.id=d.tenant_id AND t.status='ENABLED' AND (t.expires_at IS NULL OR t.expires_at>clock_timestamp()) JOIN tenant_applications ta ON ta.tenant_id=d.tenant_id AND ta.application_id=d.application_id WHERE d.client_id=$1 AND d.source_application_id=$2 AND d.credential_version=$3 AND d.application_id=$4 AND d.resource=$5 AND d.action=$6 AND d.status='ENABLED' AND (d.expires_at IS NULL OR d.expires_at>clock_timestamp()) AND a.access_policy_version=$7 AND (d.scope_type='TENANT' OR EXISTS(SELECT 1 FROM organizations o JOIN organization_applications oa ON oa.organization_id=o.id AND oa.application_id=d.application_id WHERE o.id=d.organization_id AND o.tenant_id=d.tenant_id AND org_chain_active(o.id) AND (org_chain_expires_at(o.id) IS NULL OR org_chain_expires_at(o.id)>clock_timestamp())))")
        .bind(proof.client_id).bind(proof.application_id).bind(version).bind(resource.application_id).bind(&resource.resource_code).bind(&resource.action_code).bind(resource.application_policy_version)
        .bind(proof.expires_at)
        .fetch_all(&state.pool).await.map_err(|_|AppError::Unavailable)?;
    if values.len() != 1 {
        return Err(AppError::forbidden(
            "机器客户端必须登记唯一独立业务数据授权",
        ));
    }
    let value = values.into_iter().next().ok_or(AppError::Unavailable)?;
    let scope = value["scope"].as_str().ok_or(AppError::Unavailable)?;
    let tenant = value["tenantId"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .ok_or(AppError::Unavailable)?;
    let org = value["organizationId"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok());
    let ids = value["organizationIds"]
        .as_array()
        .ok_or(AppError::Unavailable)?
        .iter()
        .map(|id| {
            id.as_str()
                .and_then(|id| Uuid::parse_str(id).ok())
                .filter(|id| is_uuid_v7(*id))
                .ok_or(AppError::Unavailable)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if ids.iter().collect::<std::collections::BTreeSet<_>>().len() != ids.len()
        || !matches!(scope, "ORG" | "ORG_SUBTREE" | "TENANT")
        || (scope == "ORG" && org.is_none_or(|org| ids != [org]))
        || (scope == "ORG_SUBTREE" && (ids.is_empty() || org.is_none_or(|org| !ids.contains(&org))))
        || (scope == "TENANT" && (!ids.is_empty() || org.is_some()))
    {
        return Err(AppError::forbidden("机器数据范围登记无效"));
    }
    let active:bool=sqlx::query_scalar("SELECT platform_machine_scope_valid($2,$4,$3,$1) AND NOT EXISTS(SELECT 1 FROM unnest($1::uuid[]) allowed(id) WHERE NOT EXISTS(SELECT 1 FROM organization_applications oa WHERE oa.organization_id=allowed.id AND oa.application_id=$5))")
        .bind(&ids).bind(tenant).bind(scope).bind(org).bind(resource.application_id).fetch_one(&state.pool).await.map_err(|_|AppError::Unavailable)?;
    if !active {
        return Err(AppError::forbidden("机器数据范围组织不可用或越界"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::uuid7::fixture_uuid;
    fn resource() -> Resource {
        Resource {
            application_id: fixture_uuid(1),
            resource_code: "store".into(),
            action_code: "read".into(),
            canonical_resource: "stores".into(),
            audience: "stores".into(),
            oauth_scope: "stores:read".into(),
            application_policy_version: 1,
        }
    }
    fn user() -> Value {
        json!({"contractVersion":"resource-access-proof/1","issuer":"https://identity.test","tokenUse":"access_token","subjectMode":"USER","clientId":fixture_uuid(2),"applicationId":fixture_uuid(3),"resource":"stores","audience":"stores","scope":["stores:read"],"expiresAt":Utc::now()+chrono::Duration::seconds(60),"sub":"public-pairwise-opaque-subject","source":{"grantId":fixture_uuid(4),"clientPolicyVersion":"1","applicationProjectionVersion":"2","userId":fixture_uuid(5),"globalSessionId":fixture_uuid(6),"accountVersion":"3","authTime":Utc::now().timestamp()-1,"contextRef":fixture_uuid(7)},"context":{"membershipId":fixture_uuid(8),"memberType":"EMPLOYEE"}})
    }
    fn parse(value: &Value) -> Result<Proof, AppError> {
        let p = Proof::decode(&serde_json::to_vec(value).unwrap())?;
        p.validate("https://identity.test", &resource())?;
        Ok(p)
    }
    #[test]
    fn canonical_user_proof_rejects_wrong_authority_scope_resource_mode_and_generations() {
        let base = user();
        let good = parse(&base).unwrap();
        assert_ne!(
            good.application_id,
            resource().application_id,
            "caller and target applications are separate"
        );
        assert_eq!(good.sub.as_deref(), Some("public-pairwise-opaque-subject"));
        for (field, value) in [
            ("contractVersion", json!("old/1")),
            ("issuer", json!("https://other.test")),
            ("tokenUse", json!("id_token")),
            ("subjectMode", json!("ACCOUNT_ONLY")),
            ("audience", json!("work-orders")),
            ("resource", json!("work-orders")),
            ("scope", json!(["profile"])),
            ("scope", json!(["stores:read", "stores:read"])),
            (
                "expiresAt",
                json!(Utc::now() - chrono::Duration::seconds(1)),
            ),
        ] {
            let mut changed = base.clone();
            changed[field] = value;
            assert!(parse(&changed).is_err(), "{field}");
        }
        for value in ["0", "01", "-1", "9223372036854775808"] {
            let mut changed = base.clone();
            changed["source"]["accountVersion"] = json!(value);
            assert!(parse(&changed).is_err());
        }
        for field in ["contextRef", "globalSessionId", "userId"] {
            let mut changed = base.clone();
            changed["source"].as_object_mut().unwrap().remove(field);
            assert!(parse(&changed).is_err(), "{field}");
        }
        let mut changed = base;
        changed["source"]["userId"] = json!(Uuid::new_v4());
        assert!(parse(&changed).is_err());
    }
    #[test]
    fn machine_wire_contract_contains_no_person_even_null_fields() {
        let mut machine = user();
        let map = machine.as_object_mut().unwrap();
        map.remove("sub");
        map.remove("context");
        map.insert("subjectMode".into(), json!("MACHINE"));
        map.insert("credentialVersion".into(), json!("4"));
        machine["source"] = json!({"grantId":fixture_uuid(4),"clientPolicyVersion":"1","applicationProjectionVersion":"2"});
        assert!(parse(&machine).is_ok());
        for field in ["sub", "context"] {
            let mut changed = machine.clone();
            changed[field] = Value::Null;
            assert!(parse(&changed).is_err());
        }
        for field in [
            "userId",
            "globalSessionId",
            "accountVersion",
            "authTime",
            "contextRef",
        ] {
            let mut changed = machine.clone();
            changed["source"][field] = Value::Null;
            assert!(parse(&changed).is_err());
        }
        let mut user = user();
        user["credentialVersion"] = Value::Null;
        assert!(parse(&user).is_err());
    }
}
