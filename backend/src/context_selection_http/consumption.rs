//! Shared authority checks for BFF consumers. All writes belong to their caller's transaction.
use super::{authority::SourceProof, store};
use crate::{error::AppError, state::AppState};
use saas_platform_backend::{
    context_selection::{Binding, ContractVersion, Credential, Id, Member, Selection},
    context_selection_store::Consumer,
};
use serde::Deserialize;
use sqlx::{Postgres, Transaction};

pub(crate) struct ConsumerPrincipal(store::Principal);
impl ConsumerPrincipal {
    pub fn consumer(&self, client: Id) -> Result<Consumer, AppError> {
        Ok(Consumer {
            client_id: client,
            service_id: Some(Id::new(self.0.id).map_err(|_| AppError::Unavailable)?),
        })
    }
    pub async fn revalidate(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        client: Id,
    ) -> Result<(), AppError> {
        store::revalidate(tx, &self.0, client).await
    }
}
pub(crate) async fn authenticate_consumer(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    client: Id,
) -> Result<ConsumerPrincipal, AppError> {
    let principal =
        store::authenticate_as(&state.pool, headers, client, "CONTEXT_SELECTION_CONSUMER").await?;
    for secret in [
        state.config.context_source_service_client_secret.as_deref(),
        state.config.identity_service_client_secret.as_deref(),
        state
            .config
            .identity_key_admin_service_client_secret
            .as_deref(),
        state
            .config
            .customer_authority_service_client_secret
            .as_deref(),
        state.config.bff_oidc_client_secret.as_deref(),
        Some(state.config.jwt_secret.as_str()),
    ] {
        if !principal.distinct_from(secret) {
            return Err(AppError::forbidden("选择消费凭据必须独立登记"));
        }
    }
    Ok(ConsumerPrincipal(principal))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Extension {
    pub contract_version: ContractVersion,
    pub transaction_id: Id,
    pub credential: Credential,
}

pub(crate) async fn source(
    state: &AppState,
    extension: &Extension,
    client: Id,
    access: &str,
) -> Result<SourceProof, AppError> {
    let _version = extension.contract_version;
    let proof = state
        .context_selection_authority
        .verify(
            &extension.transaction_id.uuid().to_string(),
            client,
            Some(access),
        )
        .await?;
    proof.validate(
        state
            .config
            .oidc_issuer
            .as_deref()
            .ok_or(AppError::Unavailable)?,
        client,
    )?;
    if proof.transaction_id != extension.transaction_id {
        return Err(AppError::unauthorized("选择事务不匹配"));
    }
    Ok(proof)
}

pub(crate) async fn validate(
    tx: &mut Transaction<'_, Postgres>,
    extension: &Extension,
    proof: &SourceProof,
    consumer: Consumer,
) -> Result<(Binding, Member), AppError> {
    let rp = store::registration(tx, proof).await?;
    if rp.consumer()? != consumer {
        return Err(AppError::forbidden("选择消费服务不匹配"));
    }
    let binding = store::binding(proof, &rp)?;
    let encoded: Option<String> = sqlx::query_scalar("SELECT selection::text FROM context_selections WHERE credential_hash=$1 AND transaction_id=$2 AND client_id=$3 AND consumer_service_id IS NOT DISTINCT FROM $4::uuid")
        .bind(extension.credential.digest()).bind(extension.transaction_id.uuid()).bind(consumer.client_id.uuid()).bind(consumer.service_id.map(Id::uuid)).fetch_optional(&mut **tx).await?;
    let selected: Selection =
        serde_json::from_str(&encoded.ok_or_else(|| AppError::forbidden("选择凭据不属于此请求"))?)
            .map_err(|_| AppError::Unavailable)?;
    store::lock_member(tx, selected.member.membership_id).await?;
    let member = store::members(tx, &binding, Some(selected.member.membership_id))
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| AppError::forbidden("所选成员已失去应用资格"))?
        .member;
    if selected.binding != binding || selected.member != member {
        return Err(AppError::conflict("所选身份或授权版本已变化"));
    }
    selected
        .check_consumption(&binding, &member, chrono::Utc::now(), false)
        .map_err(|_| AppError::conflict("选择凭据已过期或变化"))?;
    Ok((binding, member))
}
