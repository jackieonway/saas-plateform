//! Platform-owned customer membership and self-profile HTTP capability.
//! OIDC authenticates the account; only this module selects customer membership.

pub(crate) mod authority;
pub(crate) mod remote_authority;
pub(crate) mod selection;
mod store;

use axum::{
    extract::{DefaultBodyLimit, Request, State},
    http::{header, HeaderMap, HeaderValue, Uri},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{error::AppError, extractors::ApiJson, state::AppState, util::uuid7};
use authority::AccountProof;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/internal/customer-portal/memberships", post(memberships))
        .route("/internal/customer-portal/context", post(context))
        .route("/internal/customer-portal/profile", post(profile))
        .route("/internal/customer-portal/revoke", post(revoke))
        .layer(DefaultBodyLimit::max(16384))
        .layer(middleware::from_fn(private_response))
}

async fn private_response(request: Request, next: Next) -> Response {
    let mut response =
        match tokio::time::timeout(std::time::Duration::from_secs(3), next.run(request)).await {
            Ok(response) => response,
            Err(_) => AppError::Unavailable.into_response(),
        };
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}

fn invalid() -> AppError {
    AppError::unauthorized("客户门户认证或原客户上下文已失效")
}

fn canonical_secret(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn equal(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AccountRequest {
    oidc_access_token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Selection {
    oidc_access_token: String,
    #[serde(deserialize_with = "canonical_id")]
    membership_id: Uuid,
    #[serde(deserialize_with = "canonical_id")]
    organization_id: Uuid,
    expected_identity_version: String,
}

fn canonical_id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Uuid, D::Error> {
    let raw = String::deserialize(deserializer)?;
    let id = uuid7::parse_uuid(&raw).map_err(serde::de::Error::custom)?;
    if id.to_string() != raw {
        return Err(serde::de::Error::custom("需要规范 UUID v7"));
    }
    Ok(id)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ContextRequest {
    oidc_access_token: String,
    customer_context: String,
}

struct RequestProof<'a> {
    state: &'a AppState,
    headers: &'a HeaderMap,
    registration: store::Registration,
    account: AccountProof,
    deadline: tokio::time::Instant,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl<'a> RequestProof<'a> {
    async fn begin(
        state: &'a AppState,
        headers: &'a HeaderMap,
        uri: &Uri,
        access: &str,
    ) -> Result<Self, AppError> {
        if !state.config.customer_portal_port_enabled {
            return Err(AppError::not_found("客户门户内部入口未启用"));
        }
        if uri.query().is_some() || headers.get_all(header::AUTHORIZATION).iter().count() != 1 {
            return Err(AppError::bad_request(
                "客户门户内部入口不接受查询参数或重复认证头",
            ));
        }
        if !canonical_secret(access) {
            return Err(invalid());
        }
        let permit = state
            .customer_portal_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| AppError::Unavailable)?;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
        let registration = store::authenticate(state, headers).await?;
        let client = state
            .config
            .customer_portal_oidc_client_id
            .ok_or(AppError::Unavailable)?;
        let account = tokio::time::timeout_at(
            deadline,
            state
                .customer_portal_authority
                .verify_access(access, client),
        )
        .await
        .map_err(|_| AppError::Unavailable)??;
        let result = Self {
            state,
            headers,
            registration,
            account,
            deadline,
            _permit: permit,
        };
        result.validate_source()?;
        Ok(result)
    }

    fn validate_source(&self) -> Result<(), AppError> {
        self.account.validate(
            self.state
                .config
                .oidc_issuer
                .as_deref()
                .ok_or(AppError::Unavailable)?,
            self.state
                .config
                .customer_portal_oidc_client_id
                .ok_or(AppError::Unavailable)?,
        )
    }

    async fn revalidate(&self, access: &str) -> Result<(), AppError> {
        let current = tokio::time::timeout_at(
            self.deadline,
            self.state
                .customer_portal_authority
                .verify_access(access, self.account.source.client_id),
        )
        .await
        .map_err(|_| AppError::Unavailable)??;
        if current != self.account
            || store::authenticate(self.state, self.headers).await? != self.registration
        {
            return Err(invalid());
        }
        self.validate_source()
    }
}

async fn memberships(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    ApiJson(body): ApiJson<AccountRequest>,
) -> Result<Json<Value>, AppError> {
    let proof = RequestProof::begin(&state, &headers, &uri, &body.oidc_access_token).await?;
    let members = store::members(&state.pool, &proof.registration, &proof.account.source).await?;
    proof.revalidate(&body.oidc_access_token).await?;
    if store::members(&state.pool, &proof.registration, &proof.account.source).await? != members {
        return Err(invalid());
    }
    store::final_proof(
        &state.pool,
        &proof.registration,
        &proof.account.source,
        &body.oidc_access_token,
        &members,
        None,
    )
    .await?;
    Ok(Json(
        json!({"contractVersion":"customer-portal-account/1","account":proof.account.account,
        "memberships":members.iter().map(store::Member::choice).collect::<Vec<_>>(),"source":proof.account.source}),
    ))
}

async fn context(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    ApiJson(body): ApiJson<Selection>,
) -> Result<Json<Value>, AppError> {
    let version = body
        .expected_identity_version
        .parse::<i64>()
        .ok()
        .filter(|version| *version >= 1 && version.to_string() == body.expected_identity_version)
        .ok_or_else(|| AppError::bad_request("成员版本必须是规范正十进制字符串"))?;
    let proof = RequestProof::begin(&state, &headers, &uri, &body.oidc_access_token).await?;
    let (token, record, member) = store::issue(
        &state,
        &proof.registration,
        &proof.account.source,
        body.membership_id,
        body.organization_id,
        version,
    )
    .await?;
    proof.revalidate(&body.oidc_access_token).await?;
    store::revalidate(
        &state.pool,
        &proof.registration,
        &proof.account.source,
        &token,
        &record,
        &member,
    )
    .await?;
    store::final_proof(
        &state.pool,
        &proof.registration,
        &proof.account.source,
        &body.oidc_access_token,
        std::slice::from_ref(&member),
        Some((&token, &record)),
    )
    .await?;
    Ok(Json(
        json!({"customerContext":token,"expiresAt":record.expires_at,
        "context":member.context(&proof.account.source,&proof.registration),"account":proof.account.account}),
    ))
}

async fn profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    ApiJson(body): ApiJson<ContextRequest>,
) -> Result<Json<Value>, AppError> {
    let proof = RequestProof::begin(&state, &headers, &uri, &body.oidc_access_token).await?;
    let record = store::context(&state.pool, &body.customer_context, false)
        .await?
        .ok_or_else(invalid)?;
    record.require_source(&proof.registration, &proof.account.source)?;
    let member = store::member(
        &state.pool,
        &proof.registration,
        &proof.account.source,
        record.membership_id,
    )
    .await?
    .ok_or_else(invalid)?;
    record.require_member(&member)?;
    proof.revalidate(&body.oidc_access_token).await?;
    store::revalidate(
        &state.pool,
        &proof.registration,
        &proof.account.source,
        &body.customer_context,
        &record,
        &member,
    )
    .await?;
    store::final_proof(
        &state.pool,
        &proof.registration,
        &proof.account.source,
        &body.oidc_access_token,
        std::slice::from_ref(&member),
        Some((&body.customer_context, &record)),
    )
    .await?;
    Ok(Json(
        json!({"context":member.context(&proof.account.source,&proof.registration),"account":proof.account.account}),
    ))
}

async fn revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    ApiJson(body): ApiJson<ContextRequest>,
) -> Result<Json<Value>, AppError> {
    let proof = RequestProof::begin(&state, &headers, &uri, &body.oidc_access_token).await?;
    let record = store::context(&state.pool, &body.customer_context, true).await?;
    if let Some(record) = &record {
        record.require_source(&proof.registration, &proof.account.source)?;
    }
    proof.revalidate(&body.oidc_access_token).await?;
    if let Some(record) = record {
        sqlx::query("UPDATE customer_portal_contexts SET revoked_at=COALESCE(revoked_at,clock_timestamp()) WHERE id=$1 AND token_hash=$2")
            .bind(record.id).bind(crate::util::hash::sha256_hex(&body.customer_context))
            .execute(&state.pool).await.map_err(|_|AppError::Unavailable)?;
    }
    Ok(Json(json!({"revoked":true})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase11_portal_request_contract_rejects_credential_substitution_and_ambiguous_selection() {
        assert!(canonical_secret(&"a".repeat(64)));
        for invalid in [
            "eyJhbGciOiJSUzI1NiJ9.payload.signature",
            "psr_token",
            "Bearer token",
            "",
        ] {
            assert!(!canonical_secret(invalid));
        }
        let id = uuid7::new_uuid().to_string();
        assert!(
            serde_json::from_value::<Selection>(json!({"oidcAccessToken":"a".repeat(64),
            "membershipId":id,"organizationId":id,"expectedIdentityVersion":"1","userId":id}))
            .is_err()
        );
        assert!(
            serde_json::from_value::<Selection>(json!({"oidcAccessToken":"a".repeat(64),
            "membershipId":id.to_uppercase(),"organizationId":id,"expectedIdentityVersion":"1"}))
            .is_err()
        );
        assert!(serde_json::from_value::<ContextRequest>(
            json!({"idToken":"jwt","customerContext":"x"})
        )
        .is_err());
    }
}
