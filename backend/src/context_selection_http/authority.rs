//! Consumer-owned account/interaction proof port. No membership or permit cache.
use crate::{config::Config, error::AppError};
use chrono::{DateTime, Utc};
use saas_platform_backend::context_selection::{ContractVersion, Id, Intent, Source, Version};
use serde::Deserialize;
use serde_json::json;
use std::{future::Future, pin::Pin, sync::Arc};

#[derive(Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Client {
    pub id: Id,
    pub policy_version: Version,
    pub purpose: String,
    pub allowed_member_types: Vec<String>,
    pub redirect_uri: String,
    #[serde(default)]
    pub client_registry: Option<String>,
    #[serde(default)]
    pub subject_mode: Option<String>,
    #[serde(default)]
    pub context_mode: Option<String>,
    #[serde(default)]
    pub application_id: Option<Id>,
}

#[derive(Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SourceProof {
    pub contract_version: ContractVersion,
    pub transaction_id: Id,
    pub intent: Intent,
    pub source: Source,
    pub client: Client,
    pub source_expires_at: DateTime<Utc>,
    pub interaction_expires_at: DateTime<Utc>,
}

impl SourceProof {
    pub fn validate(&self, issuer: &str, client: Id) -> Result<(), AppError> {
        if self.source.issuer != issuer
            || self.client.id != client
            || self.client.policy_version.value() < 1
            || self.source.auth_time > Utc::now()
            || self.source_expires_at <= Utc::now()
            || self.interaction_expires_at <= Utc::now()
            || self.client.client_registry.as_deref() != Some("AUTH")
            || self.client.subject_mode.as_deref() != Some("USER")
            || self.client.context_mode.as_deref() != Some("PLATFORM_CONTEXT")
            || self.client.application_id.is_none()
        {
            return Err(AppError::unauthorized("原选择交互来源无效"));
        }
        Ok(())
    }
}

pub(crate) type SourceFuture<'a> =
    Pin<Box<dyn Future<Output = Result<SourceProof, AppError>> + Send + 'a>>;

pub(crate) trait SourcePort: Send + Sync {
    fn verify<'a>(
        &'a self,
        source: &'a str,
        client: Id,
        access: Option<&'a str>,
    ) -> SourceFuture<'a>;
}

struct Remote {
    pool: sqlx::PgPool,
    client: Option<reqwest::Client>,
    origin: Option<String>,
    service_id: Option<String>,
    secret: Option<String>,
    issuer: Option<String>,
}

pub(crate) fn configured(config: &Config, pool: &sqlx::PgPool) -> Arc<dyn SourcePort> {
    Arc::new(Remote {
        pool: pool.clone(),
        client: config.identity_http_client.clone(),
        origin: config.identity_service_url.clone(),
        service_id: config.context_source_service_client_id.clone(),
        secret: config.context_source_service_client_secret.clone(),
        issuer: config.oidc_issuer.clone(),
    })
}

impl SourcePort for Remote {
    fn verify<'a>(
        &'a self,
        source: &'a str,
        expected_client: Id,
        access: Option<&'a str>,
    ) -> SourceFuture<'a> {
        Box::pin(async move {
            let client = self.client.as_ref().ok_or(AppError::Unavailable)?;
            let origin = self.origin.as_ref().ok_or(AppError::Unavailable)?;
            let registry: String = sqlx::query_scalar(
                "SELECT client_registry FROM context_selection_registrations WHERE client_id=$1 AND enabled",
            ).bind(expected_client.uuid()).fetch_optional(&self.pool).await
                .map_err(|_| AppError::Unavailable)?
                .ok_or_else(|| AppError::unauthorized("选择客户端未登记"))?;
            let path = match registry.as_str() {
                "AUTH" => "/internal/identity/auth/context-selection/source",
                _ => return Err(AppError::Unavailable),
            };
            let endpoint = format!("{origin}{path}");
            let id = self.service_id.as_ref().ok_or(AppError::Unavailable)?;
            let secret = self.secret.as_ref().ok_or(AppError::Unavailable)?;
            let issuer = self.issuer.as_ref().ok_or(AppError::Unavailable)?;
            let request = if let Some(access) = access {
                json!({"contractVersion":"context-selection/1","transactionId":source,"oidcAccessToken":access,"expectedClientId":expected_client})
            } else {
                json!({"contractVersion":"context-selection/1","sourceProof":source,"expectedClientId":expected_client})
            };
            let body = serde_json::to_vec(&request).map_err(|_| AppError::Unavailable)?;
            let mut response = client
                .post(endpoint)
                .basic_auth(id, Some(secret))
                .header("accept", "application/json")
                .header("content-type", "application/json")
                .body(body)
                .send()
                .await
                .map_err(|_| AppError::Unavailable)?;
            match response.status().as_u16() {
                200 => {}
                400 | 401 | 403 | 409 | 410 => {
                    return Err(AppError::unauthorized("原选择交互来源已失效"))
                }
                _ => return Err(AppError::Unavailable),
            }
            if response.content_length().is_some_and(|v| v > 65536)
                || !response
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| {
                        v.split(';')
                            .next()
                            .is_some_and(|v| v.trim() == "application/json")
                    })
            {
                return Err(AppError::Unavailable);
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| AppError::Unavailable)? {
                if bytes.len() + chunk.len() > 65536 {
                    return Err(AppError::Unavailable);
                }
                bytes.extend_from_slice(&chunk);
            }
            let proof: SourceProof =
                serde_json::from_slice(&bytes).map_err(|_| AppError::Unavailable)?;
            proof.validate(issuer, expected_client)?;
            if proof.client.client_registry.as_deref() != Some(registry.as_str()) {
                return Err(AppError::unauthorized("原选择交互客户端登记来源不匹配"));
            }
            Ok(proof)
        })
    }
}
