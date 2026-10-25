//! Consumer-owned HTTP capability. Endpoints are an enum, never caller URLs.

use crate::{config::Config, error::AppError, state::AppState};
use std::{future::Future, pin::Pin, sync::Arc};

#[derive(Clone, Copy)]
pub(crate) enum Endpoint {
    Discovery,
    Jwks,
    Token,
    UserInfo,
    Introspect,
    Revoke,
    GlobalLogout,
}
impl Endpoint {
    pub(crate) fn path(self) -> &'static str {
        match self {
            Self::Discovery => "/.well-known/openid-configuration",
            Self::Jwks => "/oidc/jwks",
            Self::Token => "/oidc/token",
            Self::UserInfo => "/oidc/userinfo",
            Self::Introspect => "/oidc/introspect",
            Self::Revoke => "/oidc/revoke",
            Self::GlobalLogout => "/oidc/account/logout",
        }
    }
    pub(crate) fn is_post(self) -> bool {
        matches!(
            self,
            Self::Token | Self::Introspect | Self::Revoke | Self::GlobalLogout
        )
    }
}

/// May contain credentials; never derive Debug or Serialize.
pub(crate) struct OidcRequest {
    pub endpoint: Endpoint,
    pub authorization: Option<String>,
    pub form: Vec<(String, String)>,
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
}
pub(crate) fn configured(config: &Config) -> Arc<dyn OidcTransport> {
    Arc::new(HttpTransport {
        client: config.bff_http_client.clone(),
        issuer: config.bff_oidc_issuer.clone(),
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
            let client = self.client.as_ref().ok_or(AppError::Unavailable)?;
            let issuer = self.issuer.as_deref().ok_or(AppError::Unavailable)?;
            let url = format!("{issuer}{}", request.endpoint.path());
            let mut outgoing = if request.endpoint.is_post() {
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
