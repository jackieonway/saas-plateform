//! Consumer-owned HTTP capability. Endpoints are an enum, never caller URLs.

use crate::{config::Config, error::AppError, state::AppState};
use std::{future::Future, pin::Pin, sync::Arc};
use uuid::Uuid;

#[derive(Clone, Copy)]
pub(crate) enum Endpoint {
    Discovery,
    Jwks,
    Token,
    UserInfo,
    AccountProof,
    #[cfg(test)]
    Revoke,
    GlobalLogout,
    LogoutOperation(Uuid),
    ContextInteractions,
}
impl Endpoint {
    pub(crate) fn path(self) -> String {
        match self {
            Self::Discovery => "/.well-known/openid-configuration".into(),
            Self::Jwks => "/oauth/jwks".into(),
            Self::Token => "/oauth/token".into(),
            Self::UserInfo => "/oauth/userinfo".into(),
            Self::AccountProof => "/internal/identity/auth/account-proof".into(),
            #[cfg(test)]
            Self::Revoke => "/oauth/revoke".into(),
            Self::GlobalLogout => "/oauth/logout".into(),
            Self::LogoutOperation(operation) => format!("/oauth/logout/operations/{operation}"),
            Self::ContextInteractions => "/internal/identity/auth/context-interactions".into(),
        }
    }
    pub(crate) fn is_post(self) -> bool {
        matches!(
            self,
            Self::Token
                | Self::AccountProof
                | Self::GlobalLogout
                | Self::LogoutOperation(_)
                | Self::ContextInteractions
        ) || self.is_legacy_revoke()
    }

    #[cfg(test)]
    fn is_legacy_revoke(self) -> bool {
        matches!(self, Self::Revoke)
    }
    #[cfg(not(test))]
    fn is_legacy_revoke(self) -> bool {
        false
    }

    fn is_private(self) -> bool {
        matches!(self, Self::AccountProof | Self::ContextInteractions)
    }
}

/// May contain credentials; never derive Debug or Serialize.
pub(crate) struct OidcRequest {
    pub endpoint: Endpoint,
    pub authorization: Option<String>,
    pub form: Vec<(String, String)>,
    pub json: Option<serde_json::Value>,
}
pub(crate) struct OidcResponse {
    pub status: u16,
    pub body: Vec<u8>,
}
pub(crate) type OidcFuture<'a> =
    Pin<Box<dyn Future<Output = Result<OidcResponse, AppError>> + Send + 'a>>;
pub(crate) trait OidcTransport: Send + Sync {
    fn send<'a>(&'a self, request: OidcRequest) -> OidcFuture<'a>;
}

struct HttpTransport {
    client: Option<reqwest::Client>,
    issuer: Option<String>,
    private_client: Option<reqwest::Client>,
    private_origin: Option<String>,
}
pub(crate) fn configured(config: &Config) -> Arc<dyn OidcTransport> {
    Arc::new(HttpTransport {
        client: config.bff_http_client.clone(),
        issuer: config.bff_oidc_issuer.clone(),
        private_client: config.identity_http_client.clone(),
        private_origin: config.identity_service_url.clone(),
    })
}

pub(crate) fn encode_form(parameters: &[(String, String)]) -> String {
    // URL's mature form encoder is already present through reqwest/url.
    let mut url =
        reqwest::Url::parse("https://form.invalid/").unwrap_or_else(|_| unreachable!("static URL"));
    url.query_pairs_mut().extend_pairs(
        parameters
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );
    url.query().unwrap_or("").to_owned()
}

impl OidcTransport for HttpTransport {
    fn send<'a>(&'a self, request: OidcRequest) -> OidcFuture<'a> {
        Box::pin(async move {
            let (client, origin) = if request.endpoint.is_private() {
                (self.private_client.as_ref(), self.private_origin.as_deref())
            } else {
                (self.client.as_ref(), self.issuer.as_deref())
            };
            let client = client.ok_or(AppError::Unavailable)?;
            let origin = origin.ok_or(AppError::Unavailable)?;
            let url = format!("{origin}{}", request.endpoint.path());
            let mut outgoing = if let Some(body) = request.json {
                client
                    .post(url)
                    .header("content-type", "application/json")
                    .body(serde_json::to_vec(&body).map_err(|_| AppError::Unavailable)?)
            } else if request.endpoint.is_post() {
                client
                    .post(url)
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(encode_form(&request.form))
            } else {
                client.get(url)
            };
            outgoing = outgoing.header("accept", "application/json");
            if let Some(authorization) = request.authorization {
                outgoing = outgoing.header("authorization", authorization);
            }
            let mut response = outgoing.send().await.map_err(|_| AppError::Unavailable)?;
            if response
                .content_length()
                .is_some_and(|length| length > 65536)
            {
                return Err(AppError::Unavailable);
            }
            let status = response.status().as_u16();
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| AppError::Unavailable)? {
                if body.len() + chunk.len() > 65536 {
                    return Err(AppError::Unavailable);
                }
                body.extend_from_slice(&chunk);
            }
            Ok(OidcResponse { status, body })
        })
    }
}

pub(super) struct Chain<'a> {
    pub state: &'a AppState,
    deadline: tokio::time::Instant,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
impl<'a> Chain<'a> {
    pub(super) fn new(state: &'a AppState) -> Result<Self, AppError> {
        let permit = state
            .bff_authority_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| AppError::Unavailable)?;
        Ok(Self {
            state,
            deadline: tokio::time::Instant::now()
                + std::time::Duration::from_millis(state.config.bff_authority_timeout_ms),
            _permit: permit,
        })
    }
    pub(super) async fn send(&self, request: OidcRequest) -> Result<OidcResponse, AppError> {
        tokio::time::timeout_at(self.deadline, self.state.bff_transport.send(request))
            .await
            .map_err(|_| AppError::Unavailable)?
    }
}

#[cfg(test)]
mod tests {
    use super::Endpoint;

    #[test]
    fn account_proof_and_context_interaction_use_only_the_private_identity_origin() {
        for endpoint in [Endpoint::AccountProof, Endpoint::ContextInteractions] {
            assert!(endpoint.is_private());
            assert!(endpoint.path().starts_with("/internal/identity/auth/"));
            assert!(endpoint.is_post());
        }
        for endpoint in [
            Endpoint::Discovery,
            Endpoint::Jwks,
            Endpoint::Token,
            Endpoint::UserInfo,
            Endpoint::Revoke,
        ] {
            assert!(!endpoint.is_private());
            assert!(!endpoint.path().starts_with("/oidc/"));
        }
    }
}
