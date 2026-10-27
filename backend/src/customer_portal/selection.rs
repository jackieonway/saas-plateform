//! Customer selection commits to the platform before the independent portal DB.
//! Recovery returns the same encrypted capability, never another issued token.
use super::{authority::AccountProof, store};
use crate::{
    context_selection_http::{consumption as selection, Fault},
    error::AppError,
    extractors::ApiJson,
    state::AppState,
    util::audit,
};
use axum::{extract::State, http::HeaderMap, Json};
use saas_platform_backend::{
    context_selection::{Id, Intent, MemberType, Version},
    context_selection_store::{self, ContextResult},
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Request {
    pub contract_version: saas_platform_backend::context_selection::ContractVersion,
    pub transaction_id: Id,
    pub credential: saas_platform_backend::context_selection::Credential,
    pub expected_client_id: Id,
    pub oidc_access_token: String,
    pub bff_session_id: Id,
    pub version: Version,
    #[serde(default)]
    pub old_oidc_access_token: Option<String>,
}

pub(crate) async fn consume(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<Request>,
) -> Result<Json<Value>, Fault> {
    execute(&state, &headers, body, false)
        .await
        .map(Json)
        .map_err(Into::into)
}
pub(crate) async fn result(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<Request>,
) -> Result<Json<Value>, Fault> {
    execute(&state, &headers, body, true)
        .await
        .map(Json)
        .map_err(Into::into)
}

fn same_account(
    proof: &crate::context_selection_http::authority::SourceProof,
    account: &AccountProof,
) -> Result<(), AppError> {
    let source = &account.source;
    if source.user_id != proof.source.user_id.uuid()
        || source.global_session_id != proof.source.global_session_id.uuid()
        || source.account_version != proof.source.account_version.value()
        || source.auth_time != proof.source.auth_time.timestamp()
        || source.client_id != proof.client.id.uuid()
        || source.client_policy_version != proof.client.policy_version.value()
    {
        return Err(AppError::unauthorized("客户选择与账号来源不匹配"));
    }
    Ok(())
}

async fn execute(
    state: &AppState,
    headers: &HeaderMap,
    body: Request,
    recover: bool,
) -> Result<Value, AppError> {
    let extension = selection::Extension {
        contract_version: body.contract_version,
        transaction_id: body.transaction_id,
        credential: body.credential,
    };
    if !state.config.customer_portal_port_enabled
        || Some(body.expected_client_id.uuid()) != state.config.customer_portal_oidc_client_id
        || body.version.value() < 1
        || !super::canonical_secret(&body.oidc_access_token)
    {
        return Err(AppError::forbidden("客户选择入口未登记"));
    }
    let _permit = state
        .context_selection_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| AppError::Unavailable)?;
    if let Some(retry) = state.login_limits.admit_source(&audit::client_ip(headers)) {
        return Err(AppError::RateLimited(retry));
    }
    let principal =
        selection::authenticate_consumer(state, headers, body.expected_client_id).await?;
    let consumer = principal.consumer(body.expected_client_id)?;
    let proof = selection::source(
        state,
        &extension,
        body.expected_client_id,
        &body.oidc_access_token,
    )
    .await?;
    let account = state
        .customer_portal_authority
        .verify_access(&body.oidc_access_token, body.expected_client_id.uuid())
        .await?;
    account.validate(&proof.source.issuer, body.expected_client_id.uuid())?;
    same_account(&proof, &account)?;
    let old = match (&proof.intent, body.old_oidc_access_token.as_deref()) {
        (Intent::Login {}, None) => None,
        (
            Intent::Switch {
                expected_version,
                session_id,
            },
            Some(access),
        ) if body.bff_session_id != *session_id
            && body.version.value() > expected_version.value() =>
        {
            let old = state
                .customer_portal_authority
                .verify_access(access, body.expected_client_id.uuid())
                .await?;
            old.validate(&proof.source.issuer, body.expected_client_id.uuid())?;
            if old.source.user_id != account.source.user_id
                || old.source.account_version != account.source.account_version
            {
                return Err(AppError::unauthorized("原门户账号已变化"));
            }
            Some(old)
        }
        _ => return Err(AppError::bad_request("原门户会话证明缺失或混用")),
    };
    let registration = store::selection_registration(state).await?;
    let cipher = state
        .config
        .bff_token_keys
        .as_deref()
        .ok_or(AppError::Unavailable)?;
    let mut tx = state.pool.begin().await?;
    // Same order as the existing customer resource issuance use case.
    for (table, id) in [
        ("users", account.source.user_id),
        ("applications", registration.application_id),
        ("oauth_clients", registration.id),
    ] {
        sqlx::query(&format!("SELECT id FROM {table} WHERE id=$1 FOR UPDATE"))
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    principal
        .revalidate(&mut tx, body.expected_client_id)
        .await?;
    let (binding, member) = selection::validate(&mut tx, &extension, &proof, consumer).await?;
    if member.member_type != MemberType::Customer
        || binding
            .registration
            .application
            .as_ref()
            .map(|a| a.id.uuid())
            != Some(registration.application_id)
    {
        return Err(AppError::forbidden("客户应用或身份用途不匹配"));
    }
    let saved:Option<(Option<String>,Option<String>)>=sqlx::query_as("SELECT result_ciphertext,result::text FROM context_selections WHERE transaction_id=$1 AND credential_hash=$2 FOR UPDATE")
        .bind(extension.transaction_id.uuid()).bind(extension.credential.digest()).fetch_optional(&mut *tx).await?;
    let saved = saved.ok_or_else(|| AppError::forbidden("选择凭据不存在"))?;
    if recover && saved.1.is_none() {
        return Ok(json!({"status":"PENDING"}));
    }
    let (token, record, selected) = if recover {
        let reference = context_selection_store::recover(
            &mut tx,
            &extension.credential,
            consumer,
            &binding,
            &member,
        )
        .await
        .map_err(|_| AppError::conflict("选择结果不可恢复"))?;
        let ContextResult::Customer {
            context_id,
            version,
        } = reference
        else {
            return Err(AppError::forbidden("客户不能恢复员工结果"));
        };
        if version != body.version {
            return Err(AppError::conflict("原门户结果版本不匹配"));
        }
        let token = cipher.open(
            extension.transaction_id.uuid(),
            &account.source.issuer,
            account.source.client_id,
            "customer-selection-result",
            saved.0.as_deref().ok_or(AppError::Unavailable)?,
        )?;
        let record = store::context(&mut *tx, &token, true)
            .await?
            .ok_or_else(|| AppError::unauthorized("客户结果已失效"))?;
        if record.id != context_id.uuid() {
            return Err(AppError::unauthorized("客户结果引用不匹配"));
        }
        record.require_source(&registration, &account.source)?;
        let selected = store::member(
            &mut *tx,
            &registration,
            &account.source,
            member.membership_id.uuid(),
        )
        .await?
        .ok_or_else(|| AppError::forbidden("客户成员已失效"))?;
        record.require_member(&selected)?;
        let association:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM customer_portal_contexts WHERE id=$1 AND bff_session_id=$2 AND bff_context_version=$3)").bind(record.id).bind(body.bff_session_id.uuid()).bind(body.version.value()).fetch_one(&mut *tx).await?;
        if !association {
            return Err(AppError::forbidden("结果不属于原门户会话"));
        }
        (token, record, selected)
    } else {
        if saved.1.is_some() {
            return Err(AppError::conflict("选择已消费，请恢复原结果"));
        }
        if let (
            Intent::Switch {
                session_id,
                expected_version,
            },
            Some(old),
        ) = (&proof.intent, &old)
        {
            let previous:Option<(Uuid,String)>=sqlx::query_as("SELECT id,token_hash FROM customer_portal_contexts WHERE bff_session_id=$1 AND bff_context_version=$2 AND user_id=$3 AND oidc_client_id=$4 AND oidc_grant_id=$5 AND revoked_at IS NULL AND expires_at>clock_timestamp() FOR UPDATE")
                .bind(session_id.uuid()).bind(expected_version.value()).bind(old.source.user_id).bind(old.source.client_id).bind(old.source.grant_id).fetch_optional(&mut *tx).await?;
            let previous = previous.ok_or_else(|| AppError::conflict("原客户上下文已变化"))?;
            store::final_proof(
                &mut *tx,
                &registration,
                &old.source,
                body.old_oidc_access_token
                    .as_deref()
                    .ok_or(AppError::Unavailable)?,
                &[],
                None,
            )
            .await?;
            let _previous = previous;
            sqlx::query("UPDATE customer_portal_contexts SET revoked_at=clock_timestamp() WHERE bff_session_id=$1 AND bff_context_version=$2 AND user_id=$3 AND oidc_client_id=$4 AND revoked_at IS NULL")
            .bind(session_id.uuid()).bind(expected_version.value()).bind(old.source.user_id).bind(old.source.client_id)
            .execute(&mut *tx)
            .await?;
        }
        let issued = store::issue_in_tx(
            &mut tx,
            &registration,
            &account.source,
            member.membership_id.uuid(),
            member.organization_id.uuid(),
            member.identity_version.value(),
        )
        .await?;
        sqlx::query("UPDATE customer_portal_contexts SET bff_session_id=$2,bff_context_version=$3 WHERE id=$1").bind(issued.1.id).bind(body.bff_session_id.uuid()).bind(body.version.value()).execute(&mut *tx).await?;
        let encrypted = cipher.seal(
            extension.transaction_id.uuid(),
            &account.source.issuer,
            account.source.client_id,
            "customer-selection-result",
            &issued.0,
        )?;
        sqlx::query("UPDATE context_selections SET result_ciphertext=$2 WHERE transaction_id=$1 AND consumed_at IS NULL").bind(extension.transaction_id.uuid()).bind(encrypted).execute(&mut *tx).await?;
        context_selection_store::consume(
            &mut tx,
            &extension.credential,
            consumer,
            &binding,
            &member,
            &ContextResult::Customer {
                context_id: Id::new(issued.1.id).map_err(|_| AppError::Unavailable)?,
                version: body.version,
            },
        )
        .await
        .map_err(|_| AppError::conflict("选择已过期或被其他会话消费"))?;
        audit::write_in_tx(&mut tx,audit::AuditInfo{user_id:Some(account.source.user_id),username:&account.account.username,tenant_id:Some(member.tenant_id.uuid()),organization_id:Some(member.organization_id.uuid()),action:"CONTEXT_SELECTION_CONSUME",detail:&json!({"transactionId":extension.transaction_id,"bffSessionId":body.bff_session_id,"version":body.version,"customerContextId":issued.1.id,"membershipId":member.membership_id}).to_string(),ip:&audit::client_ip(headers),user_agent:&audit::ua_of(headers)}).await?;
        issued
    };
    store::final_proof(
        &mut *tx,
        &registration,
        &account.source,
        &body.oidc_access_token,
        std::slice::from_ref(&selected),
        Some((&token, &record)),
    )
    .await?;
    let current = selection::source(
        state,
        &extension,
        body.expected_client_id,
        &body.oidc_access_token,
    )
    .await?;
    if current != proof {
        return Err(AppError::conflict("原选择来源已变化"));
    }
    tx.commit().await?;
    Ok(
        json!({"status":"COMMITTED","customerContext":token,"expiresAt":record.expires_at,"context":selected.context(&account.source,&registration),"account":account.account}),
    )
}
