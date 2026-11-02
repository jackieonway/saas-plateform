//! Strict relying-party validation. Offline signatures never replace online
//! user, global-session and registered-client authority.

use super::{
    store,
    transport::{Chain, Endpoint, OidcRequest, OidcResponse},
};
use crate::error::AppError;
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use chrono::{DateTime, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

pub(super) struct AccountProof {
    pub subject: String,
    pub user_id: Uuid,
    pub global_session_id: Uuid,
    pub grant_id: Uuid,
    pub account_version: i64,
    pub client_id: Uuid,
    pub client_policy_version: i64,
    pub auth_time: i64,
    pub expires_at: DateTime<Utc>,
    pub absolute_expires_at: DateTime<Utc>,
}
pub(super) struct Verified {
    pub public_sid: String,
    pub selection: Option<crate::context_selection_http::consumption::Extension>,
    pub proof: AccountProof,
    pub access: String,
    pub refresh: String,
    pub absolute_expires_at: DateTime<Utc>,
}
struct Identity {
    subject: String,
    sid: String,
    auth_time: i64,
    expires_at: DateTime<Utc>,
}

fn invalid() -> AppError {
    AppError::unauthorized("OIDC 认证响应无效，请重新登录")
}
fn object(value: &Value) -> Result<&serde_json::Map<String, Value>, AppError> {
    value.as_object().ok_or_else(invalid)
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, AppError> {
    value.get(key).and_then(Value::as_str).ok_or_else(invalid)
}
fn number(value: &Value, key: &str) -> Result<i64, AppError> {
    value.get(key).and_then(Value::as_i64).ok_or_else(invalid)
}
fn id(value: &str) -> Result<Uuid, AppError> {
    let id = crate::util::uuid7::parse_uuid(value).map_err(|_| invalid())?;
    if id.to_string() != value {
        return Err(invalid());
    }
    Ok(id)
}
fn generation(value: &str, min: i64) -> Result<i64, AppError> {
    let number = value.parse::<i64>().map_err(|_| invalid())?;
    if number < min || number.to_string() != value {
        return Err(invalid());
    }
    Ok(number)
}

fn json(response: OidcResponse) -> Result<Value, AppError> {
    if response.body.len() > 65536 {
        return Err(AppError::Unavailable);
    }
    let value: Value = serde_json::from_slice(&response.body).map_err(|_| AppError::Unavailable)?;
    if response.status != 200 {
        return Err(match value.get("error").and_then(Value::as_str) {
            Some("invalid_grant" | "invalid_token" | "access_denied") => invalid(),
            _ => AppError::Unavailable,
        });
    }
    object(&value)?;
    Ok(value)
}
fn basic(chain: &Chain<'_>) -> Result<String, AppError> {
    let client = store::client(chain.state)?;
    let secret = chain
        .state
        .config
        .bff_oidc_client_secret
        .as_deref()
        .ok_or(AppError::Unavailable)?;
    Ok(format!(
        "Basic {}",
        STANDARD.encode(format!("{client}:{secret}"))
    ))
}

fn source_basic(chain: &Chain<'_>) -> Result<String, AppError> {
    let client = chain
        .state
        .config
        .context_source_service_client_id
        .as_deref()
        .ok_or(AppError::Unavailable)?;
    let secret = chain
        .state
        .config
        .context_source_service_client_secret
        .as_deref()
        .ok_or(AppError::Unavailable)?;
    if !super::canonical_secret(secret) {
        return Err(AppError::Unavailable);
    }
    Ok(format!(
        "Basic {}",
        STANDARD.encode(format!("{client}:{secret}"))
    ))
}
async fn call(
    chain: &Chain<'_>,
    endpoint: Endpoint,
    authorization: Option<String>,
    form: Vec<(String, String)>,
) -> Result<Value, AppError> {
    json(
        chain
            .send(OidcRequest {
                json: None,
                endpoint,
                authorization,
                form,
            })
            .await?,
    )
}

pub(super) async fn discovery(chain: &Chain<'_>) -> Result<(), AppError> {
    let issuer = store::issuer(chain.state)?;
    let document = call(chain, Endpoint::Discovery, None, vec![]).await?;
    if text(&document, "issuer")? != issuer {
        return Err(AppError::Unavailable);
    }
    for (name, endpoint) in [
        ("authorization_endpoint", "/oauth/authorize"),
        ("token_endpoint", "/oauth/token"),
        ("jwks_uri", "/oauth/jwks"),
        ("userinfo_endpoint", "/oauth/userinfo"),
        ("introspection_endpoint", "/oauth/introspect"),
        ("revocation_endpoint", "/oauth/revoke"),
    ] {
        if document.get(name).and_then(Value::as_str)
            != Some(format!("{issuer}{endpoint}").as_str())
        {
            return Err(AppError::Unavailable);
        }
    }
    for (name, required) in [
        ("response_types_supported", "code"),
        ("id_token_signing_alg_values_supported", "RS256"),
        (
            "token_endpoint_auth_methods_supported",
            "client_secret_basic",
        ),
        ("code_challenge_methods_supported", "S256"),
    ] {
        if !document
            .get(name)
            .and_then(Value::as_array)
            .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(required)))
        {
            return Err(AppError::Unavailable);
        }
    }
    Ok(())
}

pub(super) async fn verification_key(
    chain: &Chain<'_>,
    token: &str,
) -> Result<DecodingKey, AppError> {
    if token.len() > 16384 {
        return Err(invalid());
    }
    let header = jsonwebtoken::decode_header(token).map_err(|_| invalid())?;
    if header.alg != Algorithm::RS256 {
        return Err(invalid());
    }
    let kid = header
        .kid
        .as_deref()
        .filter(|kid| {
            !kid.is_empty()
                && kid.len() <= 80
                && kid
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        })
        .ok_or_else(invalid)?;
    let jwks = call(chain, Endpoint::Jwks, None, vec![]).await?;
    let keys = jwks
        .get("keys")
        .and_then(Value::as_array)
        .filter(|keys| !keys.is_empty() && keys.len() <= 32)
        .ok_or_else(invalid)?;
    let mut seen = std::collections::HashSet::new();
    let mut matching = None;
    for key in keys {
        let key_id = text(key, "kid")?;
        if !seen.insert(key_id) {
            return Err(invalid());
        }
        if ["d", "p", "q", "dp", "dq", "qi", "k", "privateKeyPem"]
            .iter()
            .any(|name| key.get(*name).is_some())
        {
            return Err(invalid());
        }
        if key_id == kid {
            matching = Some(key);
        }
    }
    let key = matching.ok_or_else(invalid)?;
    if text(key, "kty")? != "RSA"
        || text(key, "alg")? != "RS256"
        || text(key, "use")? != "sig"
        || text(key, "e")? != "AQAB"
    {
        return Err(invalid());
    }
    let n = text(key, "n")?;
    let modulus = URL_SAFE_NO_PAD.decode(n).map_err(|_| invalid())?;
    let bits = modulus
        .first()
        .map(|first| modulus.len() * 8 - first.leading_zeros() as usize)
        .unwrap_or(0);
    if !(2048..=4096).contains(&bits)
        || modulus.first() == Some(&0)
        || URL_SAFE_NO_PAD.encode(&modulus) != n
    {
        return Err(invalid());
    }
    DecodingKey::from_rsa_components(n, "AQAB").map_err(|_| invalid())
}

async fn identity(chain: &Chain<'_>, token: &str, nonce_hash: &str) -> Result<Identity, AppError> {
    if jsonwebtoken::decode_header(token)
        .map_err(|_| invalid())?
        .typ
        .as_deref()
        != Some("JWT")
    {
        return Err(invalid());
    }
    let decoding = verification_key(chain, token).await?;
    let client = store::client(chain.state)?.to_string();
    let issuer = store::issuer(chain.state)?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.leeway = 60;
    validation.set_issuer(&[issuer]);
    validation.set_audience(&[client.as_str()]);
    validation.set_required_spec_claims(&[
        "iss",
        "sub",
        "aud",
        "exp",
        "iat",
        "nonce",
        "auth_time",
        "sid",
    ]);
    let claims = jsonwebtoken::decode::<Value>(token, &decoding, &validation)
        .map_err(|_| invalid())?
        .claims;
    if text(&claims, "iss")? != issuer
        || text(&claims, "aud")? != client
        || !super::equal(&super::digest(text(&claims, "nonce")?), nonce_hash)
        || claims
            .get("azp")
            .is_some_and(|azp| azp.as_str() != Some(client.as_str()))
    {
        return Err(invalid());
    }
    let now = Utc::now().timestamp();
    let subject = text(&claims, "sub")?;
    let sid = text(&claims, "sid")?;
    let iat = number(&claims, "iat")?;
    let exp = number(&claims, "exp")?;
    let auth_time = number(&claims, "auth_time")?;
    if subject.is_empty()
        || subject.len() > 256
        || !subject.bytes().all(|byte| byte.is_ascii_graphic())
        || !super::canonical_secret(sid)
        || iat <= 0
        || iat > now + 60
        || exp <= iat
        || exp - iat > 660
        || exp <= now
        || auth_time <= 0
        || auth_time > iat + 60
        || auth_time > now + 60
    {
        return Err(invalid());
    }
    let expires_at = DateTime::from_timestamp(exp, 0).ok_or_else(invalid)?;
    Ok(Identity {
        subject: subject.to_owned(),
        sid: sid.to_owned(),
        auth_time,
        expires_at,
    })
}

async fn inspect(chain: &Chain<'_>, token: &str) -> Result<AccountProof, AppError> {
    let response = chain
        .send(OidcRequest {
            endpoint: Endpoint::AccountProof,
            authorization: Some(source_basic(chain)?),
            form: vec![],
            json: Some(serde_json::json!({
                "contractVersion":"platform-account-proof-request/1",
                "oidcAccessToken":token,
                "expectedClientId":store::client(chain.state)?
            })),
        })
        .await?;
    parse_account_proof(
        &json(response)?,
        store::client(chain.state)?,
        store::issuer(chain.state)?,
        Utc::now(),
    )
}

fn parse_account_proof(
    value: &Value,
    client: Uuid,
    issuer: &str,
    now: DateTime<Utc>,
) -> Result<AccountProof, AppError> {
    if text(value, "contractVersion")? != "platform-account-proof/1"
        || text(value, "clientId")? != client.to_string()
        || text(value, "issuer")? != issuer
        || text(value, "clientPurpose")? != "PLATFORM_CONSOLE"
        || text(value, "contextMode")? != "PLATFORM_CONTEXT"
        || value.get("allowedMemberTypes") != Some(&serde_json::json!(["EMPLOYEE"]))
        || !value
            .get("scope")
            .and_then(Value::as_array)
            .is_some_and(|scopes| {
                scopes.len() == 2
                    && scopes.iter().any(|scope| scope.as_str() == Some("openid"))
                    && scopes.iter().any(|scope| scope.as_str() == Some("profile"))
            })
    {
        return Err(invalid());
    }
    let expires_at = chrono::DateTime::parse_from_rfc3339(text(value, "expiresAt")?)
        .map(|value| value.with_timezone(&Utc))
        .ok()
        .filter(|expires| *expires > now)
        .ok_or_else(invalid)?;
    let absolute_expires_at =
        chrono::DateTime::parse_from_rfc3339(text(value, "absoluteExpiresAt")?)
            .map(|value| value.with_timezone(&Utc))
            .ok()
            .filter(|expires| *expires >= expires_at)
            .ok_or_else(invalid)?;
    let subject = text(value, "sub")?;
    let auth_time = number(value, "authTime")?;
    if subject.is_empty()
        || subject.len() > 256
        || subject.chars().any(char::is_control)
        || auth_time <= 0
        || auth_time > now.timestamp() + 60
        || auth_time >= expires_at.timestamp()
    {
        return Err(invalid());
    }
    Ok(AccountProof {
        subject: subject.to_owned(),
        user_id: id(text(value, "userId")?)?,
        global_session_id: id(text(value, "globalSessionId")?)?,
        grant_id: id(text(value, "grantId")?)?,
        account_version: generation(text(value, "accountVersion")?, 0)?,
        client_id: client,
        client_policy_version: generation(text(value, "clientPolicyVersion")?, 1)?,
        auth_time,
        expires_at,
        absolute_expires_at,
    })
}
fn same(proof: &AccountProof, session: &store::Session) -> Result<(), AppError> {
    if proof.subject != session.oidc_public_sub
        || proof.user_id != session.user_id
        || proof.global_session_id != session.global_session_id
        || proof.grant_id != session.oidc_grant_id
        || proof.account_version != session.account_version
        || proof.client_id != session.oidc_client_id
        || proof.client_policy_version != session.oidc_client_policy_version
        || proof.auth_time != session.auth_time
    {
        return Err(invalid());
    }
    Ok(())
}
async fn userinfo(chain: &Chain<'_>, access: &str, subject: &str) -> Result<(), AppError> {
    let value = call(
        chain,
        Endpoint::UserInfo,
        Some(format!("Bearer {access}")),
        vec![],
    )
    .await?;
    if text(&value, "sub")? != subject {
        return Err(invalid());
    }
    Ok(())
}
pub(super) async fn existing(
    chain: &Chain<'_>,
    session: &store::Session,
    access: &str,
) -> Result<AccountProof, AppError> {
    let proof = inspect(chain, access).await?;
    same(&proof, session)?;
    userinfo(chain, access, &proof.subject).await?;
    Ok(proof)
}

async fn verified(
    chain: &Chain<'_>,
    response: Value,
    nonce_hash: &str,
    original: Option<&store::Session>,
) -> Result<Verified, AppError> {
    let access = text(&response, "access_token")?.to_owned();
    let refresh = text(&response, "refresh_token")?.to_owned();
    if !super::canonical_secret(&access)
        || !super::canonical_secret(&refresh)
        || text(&response, "token_type")? != "Bearer"
        || !(1..=600).contains(&number(&response, "expires_in")?)
    {
        return Err(invalid());
    }
    let scopes = text(&response, "scope")?;
    if ["openid", "profile"]
        .iter()
        .any(|required| !scopes.split_whitespace().any(|scope| scope == *required))
    {
        return Err(invalid());
    }
    let identity = match response.get("id_token") {
        Some(value) => {
            Some(identity(chain, value.as_str().ok_or_else(invalid)?, nonce_hash).await?)
        }
        None => None,
    };
    if original.is_none() && identity.is_none() {
        return Err(invalid());
    }
    let mut proof = inspect(chain, &access).await?;
    if identity.as_ref().is_some_and(|identity| {
        identity.subject != proof.subject || identity.auth_time != proof.auth_time
    }) {
        return Err(invalid());
    }
    if let Some(original) = original {
        same(&proof, original)?;
        if identity.as_ref().is_some_and(|identity| {
            identity.sid != original.oidc_public_sid || identity.subject != original.oidc_public_sub
        }) {
            return Err(invalid());
        }
    }
    userinfo(chain, &access, &proof.subject).await?;
    let public_sid = if let Some(original) = original {
        original.oidc_public_sid.clone()
    } else {
        identity.as_ref().ok_or_else(invalid)?.sid.clone()
    };
    if let Some(identity) = identity {
        proof.expires_at = proof.expires_at.min(identity.expires_at);
    }
    let absolute_expires_at = proof.absolute_expires_at;
    Ok(Verified {
        public_sid,
        selection: if original.is_none() {
            response
                .get("context_selection")
                .map(|v| serde_json::from_value(v.clone()).map_err(|_| invalid()))
                .transpose()?
        } else {
            if response.get("context_selection").is_some() {
                return Err(invalid());
            }
            None
        },
        proof,
        access,
        refresh,
        absolute_expires_at,
    })
}

pub(super) async fn exchange(
    chain: &Chain<'_>,
    code: &str,
    verifier: &str,
    nonce_hash: &str,
) -> Result<Verified, AppError> {
    discovery(chain).await?;
    let redirect = chain
        .state
        .config
        .bff_oidc_redirect_uri
        .as_deref()
        .ok_or(AppError::Unavailable)?;
    let value = call(
        chain,
        Endpoint::Token,
        Some(basic(chain)?),
        vec![
            ("grant_type".into(), "authorization_code".into()),
            ("code".into(), code.into()),
            ("redirect_uri".into(), redirect.into()),
            ("code_verifier".into(), verifier.into()),
        ],
    )
    .await?;
    verified(chain, value, nonce_hash, None).await
}
pub(super) async fn refresh(
    chain: &Chain<'_>,
    original: &store::Session,
    refresh: &str,
) -> Result<Verified, AppError> {
    discovery(chain).await?;
    let value = call(
        chain,
        Endpoint::Token,
        Some(basic(chain)?),
        vec![
            ("grant_type".into(), "refresh_token".into()),
            ("refresh_token".into(), refresh.into()),
        ],
    )
    .await?;
    verified(chain, value, &original.nonce_hash, Some(original)).await
}
pub(super) async fn logout(
    chain: &Chain<'_>,
    session: &store::Session,
    refresh: &str,
    scope: &str,
    request: Uuid,
) -> Result<(), AppError> {
    if session.issuer != store::issuer(chain.state)?
        || session.oidc_client_id != store::client(chain.state)?
    {
        return Err(invalid());
    }
    let canonical_scope = match scope {
        "GLOBAL" => "GLOBAL",
        "PLATFORM" => "CURRENT_APP",
        _ => return Err(invalid()),
    };
    if !super::canonical_secret(&session.oidc_public_sid)
        || session.oidc_public_sub.is_empty()
        || session.oidc_public_sub.len() > 256
        || !session
            .oidc_public_sub
            .bytes()
            .all(|byte| byte.is_ascii_graphic())
    {
        return Err(invalid());
    }
    let form = vec![
        ("token".into(), refresh.into()),
        ("scope".into(), canonical_scope.into()),
        ("operation_id".into(), request.to_string()),
        ("sid".into(), session.oidc_public_sid.clone()),
        ("sub".into(), session.oidc_public_sub.clone()),
    ];
    // The same original credential remains a receipt proof after revocation.
    // Only an authenticated, explicit NOT_FOUND permits submitting this exact
    // operation; uncertainty never changes the operation or its source.
    let recovery = chain
        .send(OidcRequest {
            json: None,
            endpoint: Endpoint::LogoutOperation(request),
            authorization: Some(basic(chain)?),
            form: form.clone(),
        })
        .await?;
    let response = if recovery.status == 404
        && serde_json::from_slice::<Value>(&recovery.body)
            .ok()
            .is_some_and(|value| value["code"] == "NOT_FOUND")
    {
        chain
            .send(OidcRequest {
                json: None,
                endpoint: Endpoint::GlobalLogout,
                authorization: Some(basic(chain)?),
                form,
            })
            .await?
    } else {
        recovery
    };
    if response.status != 200 {
        json(response)?;
        return Err(AppError::Unavailable);
    }
    let receipt: LogoutReceipt =
        serde_json::from_slice(&response.body).map_err(|_| AppError::Unavailable)?;
    if receipt.operation_id != request
        || receipt.scope != canonical_scope
        || receipt.state != "COMPLETED"
        || receipt.revoked_grant_count < 0
        || receipt.revoked_sso_count < 0
        || receipt.notification_count < 0
    {
        return Err(AppError::Unavailable);
    }
    // Keep the original caller-owned identity in this command; never derive a
    // fresh account or sid from the result of its own revocation.
    if session.oidc_client_id != store::client(chain.state)? {
        return Err(invalid());
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LogoutReceipt {
    operation_id: Uuid,
    scope: String,
    state: String,
    revoked_grant_count: i64,
    revoked_sso_count: i64,
    notification_count: i64,
}

pub(super) async fn create_selection(
    chain: &Chain<'_>,
    transaction: Uuid,
    intent: Value,
    state_hash: &str,
    nonce_hash: &str,
    challenge: &str,
) -> Result<String, AppError> {
    let response = chain.send(OidcRequest { endpoint: Endpoint::ContextInteractions, authorization: Some(basic(chain)?), form: vec![],
        json: Some(serde_json::json!({"contractVersion":"context-selection/1","transactionId":transaction,"intent":intent,
            "redirectUri":chain.state.config.bff_oidc_redirect_uri,"stateHash":state_hash,"nonceHash":nonce_hash,"codeChallenge":challenge})) }).await?;
    let value = json(response)?;
    let reference = text(&value, "contextRequest")?;
    let expires = DateTime::parse_from_rfc3339(text(&value, "expiresAt")?)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| invalid())?;
    if text(&value, "contractVersion")? != "context-selection/1"
        || !super::canonical_secret(reference)
        || expires <= Utc::now()
    {
        return Err(invalid());
    }
    Ok(reference.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn proof(client: Uuid, now: DateTime<Utc>) -> Value {
        json!({
            "contractVersion":"platform-account-proof/1",
            "issuer":"https://auth.example.invalid",
            "clientId":client,
            "clientPurpose":"PLATFORM_CONSOLE",
            "contextMode":"PLATFORM_CONTEXT",
            "allowedMemberTypes":["EMPLOYEE"],
            "scope":["openid","profile"],
            "sub":"pairwise-account-with-no-uuid",
            "userId":Uuid::now_v7(),
            "globalSessionId":Uuid::now_v7(),
            "grantId":Uuid::now_v7(),
            "accountVersion":"0",
            "clientPolicyVersion":"1",
            "authTime":now.timestamp()-60,
            "expiresAt":now+chrono::Duration::minutes(5),
            "absoluteExpiresAt":now+chrono::Duration::hours(1)
        })
    }

    #[test]
    fn private_account_proof_keeps_opaque_subject_separate_from_internal_account() {
        let client = Uuid::now_v7();
        let now = Utc::now();
        let value = proof(client, now);
        let parsed = parse_account_proof(&value, client, "https://auth.example.invalid", now)
            .expect("valid private account proof");
        assert_eq!(parsed.subject, "pairwise-account-with-no-uuid");
        assert_eq!(parsed.user_id.to_string(), value["userId"]);
        assert_ne!(parsed.subject, parsed.user_id.to_string());
    }

    #[test]
    fn private_account_proof_rejects_wrong_audience_purpose_versions_and_deadlines() {
        let client = Uuid::now_v7();
        let now = Utc::now();
        let value = proof(client, now);
        for (field, invalid_value) in [
            ("contractVersion", json!("oidc-account-proof/1")),
            ("issuer", json!("https://other.example.invalid")),
            ("clientId", json!(Uuid::now_v7())),
            ("clientPurpose", json!("CUSTOMER_PORTAL")),
            ("contextMode", json!("ACCOUNT_ONLY")),
            ("allowedMemberTypes", json!(["CUSTOMER"])),
            ("allowedMemberTypes", json!(["EMPLOYEE", "CUSTOMER"])),
            ("scope", json!(["openid"])),
            ("scope", json!(["openid", "profile", "offline_access"])),
            ("sub", json!("")),
            ("sub", json!("invalid\nsubject")),
            ("userId", json!(Uuid::nil())),
            ("globalSessionId", json!(Uuid::nil())),
            ("grantId", json!(Uuid::nil())),
            ("accountVersion", json!("00")),
            ("clientPolicyVersion", json!("0")),
            ("authTime", json!(now.timestamp() + 61)),
            ("authTime", json!(0)),
            ("expiresAt", json!(now)),
            ("absoluteExpiresAt", json!(now)),
        ] {
            let mut invalid_proof = value.clone();
            invalid_proof[field] = invalid_value;
            assert!(
                parse_account_proof(&invalid_proof, client, "https://auth.example.invalid", now)
                    .is_err(),
                "invalid private proof field accepted: {field}"
            );
        }
    }
}
