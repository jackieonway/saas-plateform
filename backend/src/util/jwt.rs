use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

/// JWT 载荷 (HS256):
/// - session:    sub=用户id, tenant/org, TTL 12h
/// - credential: sub=用户id (登录票据, TTL 10min)
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

pub fn sign(claims: &Claims, secret: &str) -> anyhow::Result<String> {
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
}
