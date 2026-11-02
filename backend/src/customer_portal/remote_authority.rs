//! HTTPS implementation of the consumer-owned customer account proof port.
//! This capability accepts access tokens only; it has no database connection,
//! account commands, member selection, credential cache or arbitrary URLs.

use std::{future::Future, path::Path, pin::Pin, sync::Arc, time::Duration};

use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{config::Config, error::AppError, util::uuid7};

use super::authority::{Account, AccountProof, AccountProofPort, ProofFuture, Source};

const PATH: &str = "/internal/identity/customer-access-proof";
const LIMIT: usize = 65536;

// Credential-bearing request and transport deliberately implement neither
// Debug nor Serialize outside the exact private wire request below.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Request<'a> {
    contract_version: &'static str,
    oidc_access_token: &'a str,
    expected_client_id: Uuid,
}
struct Response {
    status: u16,
    content_type: Option<String>,
    body: Vec<u8>,
}
type HttpFuture<'a> = Pin<Box<dyn Future<Output = Result<Response, AppError>> + Send + 'a>>;
trait ProofTransport: Send + Sync {
    fn send<'a>(&'a self, request: Request<'a>) -> HttpFuture<'a>;
}
struct HttpTransport {
    client: reqwest::Client,
    endpoint: String,
    authorization: String,
}
impl ProofTransport for HttpTransport {
    fn send<'a>(&'a self, request: Request<'a>) -> HttpFuture<'a> {
        Box::pin(async move {
            let body = serde_json::to_vec(&request).map_err(|_| AppError::Unavailable)?;
            let mut response = self
                .client
                .post(&self.endpoint)
                .header("authorization", &self.authorization)
                .header("accept", "application/json")
                .header("content-type", "application/json")
                .body(body)
                .send()
                .await
                .map_err(|_| AppError::Unavailable)?;
            if response
                .content_length()
                .is_some_and(|length| length > LIMIT as u64)
            {
                return Err(AppError::Unavailable);
            }
            let status = response.status().as_u16();
            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| AppError::Unavailable)? {
                if body.len() + chunk.len() > LIMIT {
                    return Err(AppError::Unavailable);
                }
                body.extend_from_slice(&chunk);
            }
            Ok(Response {
                status,
                content_type,
                body,
            })
        })
    }
}
struct RemoteAuthority {
    issuer: String,
    customer_client: Uuid,
    transport: Arc<dyn ProofTransport>,
    slots: Arc<tokio::sync::Semaphore>,
}

fn canonical_secret(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn canonical_id(value: &str) -> Result<Uuid, AppError> {
    let id = uuid7::parse_uuid(value).map_err(|_| AppError::Unavailable)?;
    if id.to_string() != value {
        return Err(AppError::Unavailable);
    }
    Ok(id)
}
fn exact_origin(origin: &str) -> anyhow::Result<()> {
    let url = reqwest::Url::parse(origin)
        .map_err(|_| anyhow::anyhow!("identity authority origin is invalid"))?;
    anyhow::ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/"
            && url.origin().ascii_serialization() == origin,
        "identity authority requires an exact HTTPS origin"
    );
    Ok(())
}
fn http_client(ca_file: Option<&str>) -> anyhow::Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(1))
        .timeout(Duration::from_secs(3))
        .user_agent("SaaSPlatform-CustomerAuthority/1");
    if let Some(path) = ca_file {
        anyhow::ensure!(
            Path::new(path).is_absolute(),
            "identity authority CA path must be absolute"
        );
        let bytes = std::fs::read(path)
            .map_err(|_| anyhow::anyhow!("identity authority CA cannot be read"))?;
        anyhow::ensure!(
            bytes.len() <= LIMIT,
            "identity authority CA bundle is too large"
        );
        let certificates = reqwest::Certificate::from_pem_bundle(&bytes)
            .map_err(|_| anyhow::anyhow!("identity authority CA bundle is invalid"))?;
        anyhow::ensure!(
            !certificates.is_empty(),
            "identity authority CA bundle is empty"
        );
        for certificate in certificates {
            builder = builder.add_root_certificate(certificate);
        }
    }
    builder
        .build()
        .map_err(|_| anyhow::anyhow!("identity authority HTTPS client is invalid"))
}

/// Build the remote capability once at startup. A caller selecting remote mode
/// must propagate construction errors and must never fall back to local DB.
pub(crate) fn configured(config: &Config) -> anyhow::Result<Arc<dyn AccountProofPort>> {
    let origin = config
        .identity_service_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("IDENTITY_SERVICE_URL is required"))?;
    let issuer = config
        .oidc_issuer
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("OIDC_ISSUER is required"))?;
    let service_id = config
        .customer_authority_service_client_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("CUSTOMER_AUTHORITY_SERVICE_CLIENT_ID is required"))?;
    let service_secret = config
        .customer_authority_service_client_secret
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("CUSTOMER_AUTHORITY_SERVICE_CLIENT_SECRET is required"))?;
    let customer_client = config
        .customer_portal_oidc_client_id
        .ok_or_else(|| anyhow::anyhow!("CUSTOMER_PORTAL_OIDC_CLIENT_ID is required"))?;
    exact_origin(origin)?;
    exact_origin(issuer)?;
    let id = canonical_id(service_id).map_err(|_| {
        anyhow::anyhow!("customer authority service client must be canonical RFC UUID v7")
    })?;
    anyhow::ensure!(
        canonical_secret(service_secret) && uuid7::is_uuid_v7(customer_client),
        "customer authority service credentials are invalid"
    );
    anyhow::ensure!(
        id != customer_client
            && Some(id) != config.bff_oidc_client_id
            && config.identity_service_client_id.as_deref() != Some(service_id)
            && config.identity_service_client_secret.as_deref() != Some(service_secret)
            && config.bff_oidc_client_secret.as_deref() != Some(service_secret),
        "customer authority requires independent service credentials"
    );
    Ok(Arc::new(RemoteAuthority {
        issuer: issuer.to_owned(),
        customer_client,
        transport: Arc::new(HttpTransport {
            client: http_client(config.identity_service_ca_file.as_deref())?,
            endpoint: format!("{origin}{PATH}"),
            authorization: format!(
                "Basic {}",
                STANDARD.encode(format!("{id}:{service_secret}"))
            ),
        }),
        slots: Arc::new(tokio::sync::Semaphore::new(16)),
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireAccount {
    id: String,
    username: String,
    real_name: String,
    avatar: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireSource {
    issuer: String,
    client_id: String,
    grant_id: String,
    global_session_id: String,
    user_id: String,
    account_version: String,
    client_policy_version: String,
    auth_time: i64,
    expires_at: DateTime<Utc>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireProof {
    contract_version: String,
    token_type: String,
    client_purpose: String,
    allowed_member_types: Vec<String>,
    scope: Vec<String>,
    source: WireSource,
    account: WireAccount,
}
fn generation(value: &str, minimum: i64) -> Result<i64, AppError> {
    let parsed = value.parse::<i64>().map_err(|_| AppError::Unavailable)?;
    if parsed < minimum || parsed.to_string() != value {
        return Err(AppError::Unavailable);
    }
    Ok(parsed)
}
fn decode(response: Response, issuer: &str, client: Uuid) -> Result<AccountProof, AppError> {
    if response.status == 401 {
        return Err(AppError::unauthorized("客户门户账号认证来源无效或已失效"));
    }
    if response.status != 200
        || response.body.len() > LIMIT
        || !response.content_type.as_deref().is_some_and(|kind| {
            kind.split(';')
                .next()
                .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"))
        })
    {
        return Err(AppError::Unavailable);
    }
    let wire: WireProof =
        serde_json::from_slice(&response.body).map_err(|_| AppError::Unavailable)?;
    if wire.contract_version != "customer-access-proof/1" {
        return Err(AppError::Unavailable);
    }
    let invalid = || AppError::unauthorized("客户门户账号认证来源无效或已失效");
    let mut seen = std::collections::HashSet::new();
    if wire.token_type != "access_token"
        || wire.client_purpose != "CUSTOMER_PORTAL"
        || wire.allowed_member_types != ["CUSTOMER"]
        || wire.scope.len() > 32
        || wire.scope.iter().any(|scope| {
            scope.is_empty()
                || scope.len() > 80
                || scope.bytes().any(|b| !b.is_ascii_graphic())
                || !seen.insert(scope.as_str())
        })
        || !["openid", "profile"]
            .iter()
            .all(|required| wire.scope.iter().any(|scope| scope.as_str() == *required))
    {
        return Err(invalid());
    }
    let proof = AccountProof {
        source: Source {
            issuer: wire.source.issuer,
            client_id: canonical_id(&wire.source.client_id)?,
            grant_id: canonical_id(&wire.source.grant_id)?,
            global_session_id: canonical_id(&wire.source.global_session_id)?,
            user_id: canonical_id(&wire.source.user_id)?,
            account_version: generation(&wire.source.account_version, 1)?,
            client_policy_version: generation(&wire.source.client_policy_version, 1)?,
            auth_time: wire.source.auth_time,
            expires_at: wire.source.expires_at,
        },
        account: Account {
            id: canonical_id(&wire.account.id)?,
            username: wire.account.username,
            real_name: wire.account.real_name,
            avatar: wire.account.avatar,
        },
    };
    if proof.account.username.is_empty()
        || proof.account.username.len() > 256
        || proof.account.real_name.len() > 256
        || proof.account.avatar.len() > 4096
        || [
            &proof.account.username,
            &proof.account.real_name,
            &proof.account.avatar,
        ]
        .iter()
        .any(|value| value.chars().any(char::is_control))
    {
        return Err(AppError::Unavailable);
    }
    proof.validate(issuer, client)?;
    Ok(proof)
}
impl AccountProofPort for RemoteAuthority {
    fn verify_access<'a>(&'a self, access: &'a str, expected_client: Uuid) -> ProofFuture<'a> {
        Box::pin(async move {
            if !canonical_secret(access) || expected_client != self.customer_client {
                return Err(AppError::unauthorized("客户门户账号认证来源无效或已失效"));
            }
            let _permit = self
                .slots
                .clone()
                .try_acquire_owned()
                .map_err(|_| AppError::Unavailable)?;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
            let response = tokio::time::timeout_at(
                deadline,
                self.transport.send(Request {
                    contract_version: "customer-access-proof-request/1",
                    oidc_access_token: access,
                    expected_client_id: expected_client,
                }),
            )
            .await
            .map_err(|_| AppError::Unavailable)??;
            decode(response, &self.issuer, self.customer_client)
        })
    }
}

#[cfg(test)]
#[path = "remote_authority_tests.rs"]
mod tests;
