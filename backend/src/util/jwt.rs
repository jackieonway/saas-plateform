use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

/// JWT 载荷 (HS256):
/// - session:    sub=用户id, tenant/org, TTL 12h
/// - access:     sub=用户id或client_id, cid/tenant/org/scope
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub typ: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tenant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub org: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ver: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub aud: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub jti: Option<String>,
    pub exp: i64,
    pub iat: i64,
}

#[cfg(test)]
pub fn sign(claims: &Claims, secret: &str) -> anyhow::Result<String> {
    sign_payload(claims, secret)
}

pub(crate) fn sign_payload<T: Serialize>(claims: &T, secret: &str) -> anyhow::Result<String> {
    encode(
        &Header::new(jsonwebtoken::Algorithm::HS256),
        claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| anyhow::anyhow!("JWT 签发失败: {e}"))
}

pub fn verify(token: &str, secret: &str) -> anyhow::Result<Claims> {
    // 应用受众由具体资源服务/内省调用显式校验；此处仅验证签名与时效。
    let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
    validation.validate_aud = false;
    validation.leeway = 0;
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|e| anyhow::anyhow!("JWT 校验失败: {e}"))?;
    anyhow::ensure!(
        data.claims.exp > chrono::Utc::now().timestamp(),
        "JWT has expired"
    );
    Ok(data.claims)
}

/// A private capability authority. This URI has no public OAuth endpoints.
pub(crate) const INTERNAL_ISSUER: &str = "urn:saas:platform:internal-capabilities";
pub(crate) const PLATFORM_API_AUDIENCE: &str = "urn:saas:platform:api";

#[derive(Clone, Copy)]
pub(crate) enum InternalUse {
    PlatformSession,
    AppSession,
}
impl InternalUse {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::PlatformSession => "platform_session",
            Self::AppSession => "app_session",
        }
    }
}

#[derive(Serialize, Deserialize)]
struct InternalClaims {
    iss: String,
    token_use: String,
    #[serde(flatten)]
    claims: Claims,
}

pub(crate) fn sign_internal(
    claims: &Claims,
    token_use: InternalUse,
    secret: &str,
) -> anyhow::Result<String> {
    anyhow::ensure!(claims.typ == token_use.as_str(), "capability type mismatch");
    anyhow::ensure!(
        claims.aud.as_deref().is_some_and(|aud| !aud.is_empty()),
        "missing capability audience"
    );
    sign_payload(
        &InternalClaims {
            iss: INTERNAL_ISSUER.into(),
            token_use: token_use.as_str().into(),
            claims: claims.clone(),
        },
        secret,
    )
}

/// No prefix, unverified claim, or JWT signature alone selects this authority.
pub(crate) fn verify_internal(
    token: &str,
    secret: &str,
    token_use: InternalUse,
    audience: Option<&str>,
) -> anyhow::Result<Claims> {
    let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
    validation.set_issuer(&[INTERNAL_ISSUER]);
    validation.leeway = 0;
    if let Some(audience) = audience {
        validation.set_audience(&[audience]);
    } else {
        validation.validate_aud = false;
    }
    let claims = decode::<InternalClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )?
    .claims;
    anyhow::ensure!(
        claims.token_use == token_use.as_str() && claims.claims.typ == token_use.as_str(),
        "wrong capability use"
    );
    let now = chrono::Utc::now().timestamp();
    anyhow::ensure!(
        claims.claims.exp > now
            && claims.claims.iat <= now
            && claims.claims.exp > claims.claims.iat,
        "invalid capability time"
    );
    anyhow::ensure!(
        claims
            .claims
            .aud
            .as_deref()
            .is_some_and(|aud| !aud.is_empty()),
        "missing capability audience"
    );
    if matches!(token_use, InternalUse::AppSession) {
        anyhow::ensure!(
            claims.claims.exp - claims.claims.iat <= 300,
            "app capability exceeds lifetime"
        );
    }
    Ok(claims.claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn claims(exp: i64) -> Claims {
        Claims {
            sub: "fixture-user".into(),
            typ: "credential".into(),
            tenant: None,
            org: None,
            cid: None,
            scope: None,
            sid: None,
            ver: None,
            aud: None,
            jti: None,
            exp,
            iat: chrono::Utc::now().timestamp() - 1,
        }
    }
    #[test]
    fn strict_expiration_and_signature() {
        let key = "jwt-unit-fixture-only";
        let now = chrono::Utc::now().timestamp();
        for exp in [now - 59, now - 1, now] {
            assert!(verify(&sign(&claims(exp), key).unwrap(), key).is_err());
        }
        let token = sign(&claims(now + 60), key).unwrap();
        assert!(verify(&token, key).is_ok());
        assert!(verify(&token, "other-key").is_err());
        let mut forged = token.into_bytes();
        let last = forged.len() - 2;
        forged[last] = if forged[last] == b'a' { b'b' } else { b'a' };
        assert!(verify(std::str::from_utf8(&forged).unwrap(), key).is_err());
    }

    #[test]
    fn internal_authorities_reject_cross_use_issuer_audience_and_unbound_claims() {
        let key = "phase15-internal-key";
        let mut app = claims(chrono::Utc::now().timestamp() + 60);
        app.typ = "app_session".into();
        app.aud = Some("stores".into());
        let token = sign_internal(&app, InternalUse::AppSession, key).unwrap();
        assert!(verify_internal(&token, key, InternalUse::AppSession, Some("stores")).is_ok());
        assert!(
            verify_internal(&token, key, InternalUse::AppSession, Some("work-orders")).is_err()
        );
        assert!(verify_internal(
            &token,
            key,
            InternalUse::PlatformSession,
            Some(PLATFORM_API_AUDIENCE)
        )
        .is_err());
        assert!(verify_internal(
            &sign(&app, key).unwrap(),
            key,
            InternalUse::AppSession,
            Some("stores")
        )
        .is_err());
        let base = serde_json::to_value(InternalClaims {
            iss: INTERNAL_ISSUER.into(),
            token_use: "app_session".into(),
            claims: app.clone(),
        })
        .unwrap();
        for (field, value) in [
            ("iss", serde_json::json!("https://identity.invalid")),
            ("token_use", serde_json::json!("access_token")),
            ("typ", serde_json::json!("id_token")),
            ("aud", serde_json::json!("work-orders")),
            ("exp", serde_json::json!(app.iat + 301)),
            ("iat", serde_json::json!(app.exp + 1)),
        ] {
            let mut changed = base.clone();
            changed[field] = value;
            let wrong = sign_payload(&changed, key).unwrap();
            assert!(
                verify_internal(&wrong, key, InternalUse::AppSession, Some("stores")).is_err(),
                "{field}"
            );
        }
        for field in ["iss", "token_use", "aud"] {
            let mut changed = base.clone();
            changed.as_object_mut().unwrap().remove(field);
            assert!(
                verify_internal(
                    &sign_payload(&changed, key).unwrap(),
                    key,
                    InternalUse::AppSession,
                    Some("stores")
                )
                .is_err(),
                "missing {field}"
            );
        }
        let mut console = app;
        console.typ = "platform_session".into();
        console.aud = Some(PLATFORM_API_AUDIENCE.into());
        let token = sign_internal(&console, InternalUse::PlatformSession, key).unwrap();
        assert!(verify_internal(
            &token,
            key,
            InternalUse::PlatformSession,
            Some(PLATFORM_API_AUDIENCE)
        )
        .is_ok());
        assert!(verify_internal(
            &token,
            key,
            InternalUse::AppSession,
            Some(PLATFORM_API_AUDIENCE)
        )
        .is_err());
    }
}
