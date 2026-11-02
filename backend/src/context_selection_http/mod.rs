//! Platform-owned choice/confirmation authority behind purpose-specific services.
pub(crate) mod authority;
pub(crate) mod consumption;
mod store;

use crate::{
    error::AppError,
    extractors::ApiJson,
    state::AppState,
    util::{audit, uuid7},
};
use axum::{
    extract::{DefaultBodyLimit, RawQuery, Request, State},
    http::{header, HeaderMap, HeaderValue},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use saas_platform_backend::{
    context_selection::{Binding, ContractVersion, Credential, Id, Selection},
    context_selection_store::{self as selections, StoreError},
};
use serde::Deserialize;
use serde_json::{json, Value};

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/internal/platform/context-selection/choices",
            post(choices),
        )
        .route(
            "/internal/platform/context-selection/confirm",
            post(confirm),
        )
        .route(
            "/internal/platform/context-selection/consume",
            post(crate::customer_portal::selection::consume),
        )
        .route(
            "/internal/platform/context-selection/result",
            post(crate::customer_portal::selection::result),
        )
        .layer(DefaultBodyLimit::max(16384))
        .layer(middleware::from_fn_with_state((), bounded))
}

async fn bounded(State(()): State<()>, request: Request, next: Next) -> Response {
    let mut response =
        if request.uri().query().is_some() || request.headers().contains_key(header::COOKIE) {
            Fault(AppError::bad_request("选择私有端口不接受查询或 Cookie")).into_response()
        } else {
            match tokio::time::timeout(std::time::Duration::from_secs(3), next.run(request)).await {
                Ok(response) => response,
                Err(_) => Fault(AppError::Unavailable).into_response(),
            }
        };
    for (name, value) in [
        ("cache-control", "no-store"),
        ("pragma", "no-cache"),
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    response
}

pub(crate) struct Fault(AppError);
impl From<AppError> for Fault {
    fn from(e: AppError) -> Self {
        Self(e)
    }
}
impl From<sqlx::Error> for Fault {
    fn from(e: sqlx::Error) -> Self {
        Self(e.into())
    }
}
impl From<StoreError> for Fault {
    fn from(e: StoreError) -> Self {
        use saas_platform_backend::context_selection::SelectionError;
        Self(match e {
            StoreError::Selection(SelectionError::Invalid) => AppError::bad_request("选择绑定无效"),
            StoreError::Selection(SelectionError::Expired) => AppError::unauthorized("选择已到期"),
            StoreError::Selection(_) => AppError::conflict("选择已经变化或消费"),
            StoreError::NotFound => AppError::forbidden("选择凭据不属于当前用途"),
            StoreError::Capacity => AppError::RateLimited(1),
            StoreError::Database(e) => e.into(),
            StoreError::Inconsistent => AppError::Unavailable,
        })
    }
}
impl IntoResponse for Fault {
    fn into_response(self) -> Response {
        use axum::http::StatusCode;
        let (status, code) = match self.0 {
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, "invalid_selection_request"),
            AppError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "invalid_selection_source"),
            AppError::Forbidden(_) => (StatusCode::FORBIDDEN, "selection_not_allowed"),
            AppError::Conflict(_) => (StatusCode::CONFLICT, "selection_changed"),
            AppError::RateLimited(retry) => {
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    [("retry-after", retry.to_string())],
                    Json(json!({"error":"selection_rate_limited"})),
                )
                    .into_response()
            }
            _ => (StatusCode::SERVICE_UNAVAILABLE, "selection_unavailable"),
        };
        (status, Json(json!({"error":code}))).into_response()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Query {
    contract_version: ContractVersion,
    source_proof: Credential,
    expected_client_id: Id,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Confirmation {
    contract_version: ContractVersion,
    source_proof: Credential,
    expected_client_id: Id,
    membership_id: Id,
}

fn raw(secret: &Credential) -> Result<String, AppError> {
    // Serialize only at this backend boundary; no Debug/Display exposure exists.
    serde_json::to_value(secret)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or(AppError::Unavailable)
}

async fn prove(
    state: &AppState,
    headers: &HeaderMap,
    source: &str,
    client: Id,
) -> Result<(store::Principal, authority::SourceProof), AppError> {
    let principal = store::authenticate(&state.pool, headers, client).await?;
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
            return Err(AppError::forbidden("选择服务凭据必须与其他用途分离"));
        }
    }
    let proof = state
        .context_selection_authority
        .verify(source, client, None)
        .await?;
    proof.validate(
        state
            .config
            .oidc_issuer
            .as_deref()
            .ok_or(AppError::Unavailable)?,
        client,
    )?;
    Ok((principal, proof))
}

async fn choices(
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(_): RawQuery,
    ApiJson(body): ApiJson<Query>,
) -> Result<Json<Value>, Fault> {
    let _version = body.contract_version;
    let _permit = state
        .context_selection_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| AppError::Unavailable)?;
    if let Some(retry) = state.login_limits.admit_source(&audit::client_ip(&headers)) {
        return Err(AppError::RateLimited(retry).into());
    }
    let source = raw(&body.source_proof)?;
    let (principal, proof) = prove(&state, &headers, &source, body.expected_client_id).await?;
    let mut tx = state.pool.begin().await?;
    store::revalidate(&mut tx, &principal, body.expected_client_id).await?;
    let registration = store::registration(&mut tx, &proof).await?;
    let binding = store::binding(&proof, &registration)?;
    binding
        .validate(&proof.source.issuer, chrono::Utc::now())
        .map_err(|_| AppError::unauthorized("选择绑定无效"))?;
    let members = store::members(&mut tx, &binding, None).await?;
    let fresh = state
        .context_selection_authority
        .verify(&source, body.expected_client_id, None)
        .await?;
    if fresh != proof {
        return Err(AppError::conflict("选择来源已经变化").into());
    }
    tx.commit().await?;
    Ok(Json(
        json!({"contractVersion":"context-selection/1","transactionId":proof.transaction_id,"intent":proof.intent,"members":members,"expiresAt":proof.source_expires_at.min(proof.interaction_expires_at)}),
    ))
}

async fn confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(_): RawQuery,
    ApiJson(body): ApiJson<Confirmation>,
) -> Result<Json<Value>, Fault> {
    let _version = body.contract_version;
    let _permit = state
        .context_selection_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| AppError::Unavailable)?;
    if let Some(retry) = state.login_limits.admit_source(&audit::client_ip(&headers)) {
        return Err(AppError::RateLimited(retry).into());
    }
    let source = raw(&body.source_proof)?;
    let (principal, proof) = prove(&state, &headers, &source, body.expected_client_id).await?;
    let mut tx = state.pool.begin().await?;
    store::lock_account(&mut tx, &proof).await?;
    store::revalidate(&mut tx, &principal, body.expected_client_id).await?;
    let registration = store::registration(&mut tx, &proof).await?;
    let binding: Binding = store::binding(&proof, &registration)?;
    let members = store::members(&mut tx, &binding, Some(body.membership_id)).await?;
    let member = members
        .into_iter()
        .next()
        .ok_or_else(|| AppError::forbidden("成员不存在或已失去当前应用资格"))?;
    store::lock_member(&mut tx, body.membership_id).await?;
    let member = store::members(&mut tx, &binding, Some(body.membership_id))
        .await?
        .into_iter()
        .next()
        .filter(|v| v == &member)
        .ok_or_else(|| AppError::conflict("成员状态已变化"))?;
    let now = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut *tx)
        .await?;
    let selected = Selection::confirm(
        binding,
        member.member.clone(),
        &proof.source.issuer,
        now,
        proof.source_expires_at,
        proof.interaction_expires_at,
    )
    .map_err(|_| AppError::unauthorized("选择来源已经失效"))?;
    let consumer = registration.consumer()?;
    selections::lock_issue_capacity(&mut tx).await?;
    let (credential, issued) = if let Some(existing) = selections::recover_issued(
        &mut tx,
        selected.binding.transaction_id,
        &selected.binding,
        &selected.member,
        consumer,
        selected.expires_at,
    )
    .await?
    {
        if existing.selection.expires_at > selected.expires_at {
            return Err(AppError::conflict("原选择期限已变化").into());
        }
        let credential = Credential::parse(
            state
                .config
                .bff_token_keys
                .as_deref()
                .ok_or(AppError::Unavailable)?
                .open(
                    selected.binding.transaction_id.uuid(),
                    &selected.binding.source.issuer,
                    selected.binding.registration.client_id.uuid(),
                    "context-selection-credential",
                    &existing.credential_ciphertext,
                )?,
        )
        .map_err(|_| AppError::Unavailable)?;
        if crate::util::hash::sha256_hex(credential.as_secret()) != existing.credential_hash {
            return Err(AppError::Unavailable.into());
        }
        (credential, existing.selection)
    } else {
        let credential =
            Credential::parse(crate::sso::opaque_secret()?).map_err(|_| AppError::Unavailable)?;
        let credential_ciphertext = state
            .config
            .bff_token_keys
            .as_deref()
            .ok_or(AppError::Unavailable)?
            .seal(
                selected.binding.transaction_id.uuid(),
                &selected.binding.source.issuer,
                selected.binding.registration.client_id.uuid(),
                "context-selection-credential",
                credential.as_secret(),
            )?;
        let issued = selections::issue(
            &mut tx,
            &selected,
            &credential,
            &credential_ciphertext,
            consumer,
            state.config.login_max_entries.min(u32::MAX as usize) as u32,
        )
        .await?;
        (credential, issued)
    };
    let fresh = state
        .context_selection_authority
        .verify(&source, body.expected_client_id, None)
        .await?;
    if fresh != proof {
        return Err(AppError::conflict("选择来源已经变化").into());
    }
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id=$1")
        .bind(proof.source.user_id.uuid())
        .fetch_one(&mut *tx)
        .await?;
    audit::write_in_tx(&mut tx,audit::AuditInfo{user_id:Some(proof.source.user_id.uuid()),username:&username,tenant_id:Some(selected.member.tenant_id.uuid()),organization_id:Some(selected.member.organization_id.uuid()),action:"CONTEXT_SELECTION_CONFIRM",detail:&json!({"transactionId":selected.binding.transaction_id,"membershipId":selected.member.membership_id,"clientId":selected.binding.registration.client_id,"serviceId":principal.id,"operationId":uuid7::new_uuid()}).to_string(),ip:&audit::client_ip(&headers),user_agent:&audit::ua_of(&headers)}).await?;
    tx.commit().await?;
    Ok(Json(
        json!({"contractVersion":"context-selection/1","transactionId":issued.binding.transaction_id,"credential":credential,"expiresAt":issued.expires_at}),
    ))
}

#[cfg(test)]
mod tests;
