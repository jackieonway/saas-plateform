//! Current logout JWT contract regression, independent of any retired provider.
use crate::util::uuid7;
use anyhow::{anyhow, ensure, Result};
use base64::Engine;
use serde_json::{json, Value};
use uuid::Uuid;
const ORIGIN: &str = "https://sso-fixture.example.invalid";
mod canonical;
// Generated test-only RSA fixture; never deployment key material.
const KEY_A_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC09dwNsQP7vfcH
jnpvava5qClcjTXKqyuhI6cBa8pWoXIO6nzlwICoOjaApNkNMXhblq2wD3m1gKWG
Tz25I7EYJ6F7BCLIhBgOjv3vCxXxcy+LIC3kEpAwFknr75n3GVF7aLUeb6Cnbfed
KTw5FBgpf8BfOa6pHAlZUI25yYcIWFMZeZ02GY0jLZagrKf+ozbpYmfxvN9Ov4q7
X1t2LuC9vMnzWKl2PXamHGRmsV/X/xhzOfEXXEUICs6zv9ZRNCnHjFmbhmd6InBb
lJu2OAnWJaGulnZvv95EAIqezbZzbpSwrcOLX/pSMHdujFSL/1yQuzoANE2ZgCMd
HIyPGOElAgMBAAECggEBAI+7MFuXnAhaLFnOKyVQGkQAc0nVx0IAjxjgTofsa7w/
Y27BhQ6fxhDYm+22Ache4LAukCplQFCPq+lwCyCAXutGnxYshMwrT0hQ/xTdA0LS
MJMarhiXPftT2McTeoEvlO6mjiVsmHgj+ibUV8cmVMKxe8Sm9jnP0QUl3EwS67QN
zY+KNkFoFJ1i9NCwQGL5+eAkgIM6aIij446+ABtN9gBLyCJZxOL7EQB6EMPpG9wE
MWmXnMObm2x3BhvbT5tru9QqZtEWe9CvJKXzgqkE+M3mffJwYfc6TWYRrMbdhv47
ZyTDwIIBzcmTkmQZQ9mBmCR6eB+YPCEFKmcF0rPelYkCgYEA1AH+gt8+KyixWvWE
xgoWmFkWJTyvoCpRziSCnK4G5Ym47GkXWqIzrHPRVivrI78S+OdEH2sdZK8BeD2Y
CMCFBrTCqJAabdSyh0hroUUnNBo3jtDY+b5U5FXmRe7m2b5276K39mnGf7UfexcD
QQ5iVNUUVBiEB7tZbfvlxeqKPFsCgYEA2oKa7ti9KCLLig2MnfaVY4wlMGJbQbRO
hDTjf5RcGvZQuKeWG3idCK+5xcw/nwgSuOZggfneeCA2SQBsESn7AVs1cIDx7Sif
y4q+oQzPUY3wygbGK5P1OigttbWQ2PhuHKOZiK8jHUD8RXlIxN5DmoOutHCVWnK/
B5zt5x9h0H8CgYBy0hmre4jKCyHy52Hfd/tAy47o3/MmRYVEphVvEfl+O5bBwzD4
zzw5PAi2KNcS5xs5XtdQRM0IiJe9LW633rytIx0B+fm2XinKhvhzwWY4G7LXUFkS
+Xg0qikq2vaAG12C75e+hw8Qf2HrYk1K4KV02GpIGXb4UQlgPbr1886mXQKBgBNW
i6+nrtXmfmzxVgiE0iPFO2VUzw65jllU9O1Ni/k24130Nuynd/5UzvQaZ2cDp+SF
PvVBz9aBw0wipW2/cV9lWw1h3AdJGRtCZCyMnaxyC2VVxCQHLCTBeYojT3ZkRhpy
wBvApDLqlWjwmugRY5pBHS0K6azfith5ovRZaqaxAoGAUZsYvxogPk1EkcTBih/A
1mKwSQGUwBIuIP+Ei/HM3cyyJgASldjw5qpNA90Rm0Y3vFE+/ibzZ4mfYjcHGQ42
5CpRpKojUVIgB1WVM7ZeP4FFM8CjsrjEsXOv4ttWp2OfjpxswiTb+BQTboPh2UWK
oeOPG3v0ydLSAGjg4ngsej0=
-----END PRIVATE KEY-----"#;
const KEY_A_N: &str = "tPXcDbED-733B456b2r2uagpXI01yqsroSOnAWvKVqFyDup85cCAqDo2gKTZDTF4W5atsA95tYClhk89uSOxGCehewQiyIQYDo797wsV8XMviyAt5BKQMBZJ6--Z9xlRe2i1Hm-gp233nSk8ORQYKX_AXzmuqRwJWVCNucmHCFhTGXmdNhmNIy2WoKyn_qM26WJn8bzfTr-Ku19bdi7gvbzJ81ipdj12phxkZrFf1_8YcznxF1xFCArOs7_WUTQpx4xZm4ZneiJwW5SbtjgJ1iWhrpZ2b7_eRACKns22c26UsK3Di1_6UjB3boxUi_9ckLs6ADRNmYAjHRyMjxjhJQ";

fn payload(client: Uuid, sid: &str, subject: &str) -> Value {
    let now = chrono::Utc::now().timestamp();
    json!({"iss":ORIGIN,"aud":client,"sid":sid,"sub":subject,
        "jti":uuid7::new_uuid(),"iat":now,"exp":now+120,
        "events":{"http://schemas.openid.net/event/backchannel-logout":{}}})
}

fn signed(body: &Value) -> Result<String> {
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.typ = Some("logout+jwt".into());
    header.kid = Some("phase10-a".into());
    Ok(jsonwebtoken::encode(
        &header,
        body,
        &jsonwebtoken::EncodingKey::from_rsa_pem(KEY_A_PEM.as_bytes())?,
    )?)
}

fn signed_raw(body: &str, header: &str) -> Result<String> {
    let der = base64::engine::general_purpose::STANDARD.decode(
        KEY_A_PEM
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect::<String>(),
    )?;
    let key = ring::signature::RsaKeyPair::from_pkcs8(&der)
        .map_err(|_| anyhow!("invalid fixed test signing key"))?;
    let input = format!(
        "{}.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(header),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(body)
    );
    let mut signature = vec![0; key.public().modulus_len()];
    key.sign(
        &ring::signature::RSA_PKCS1_SHA256,
        &ring::rand::SystemRandom::new(),
        input.as_bytes(),
        &mut signature,
    )
    .map_err(|_| anyhow!("fixed test signing failed"))?;
    Ok(format!(
        "{input}.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature)
    ))
}

#[test]
fn logout_signature_domain_and_original_short_claims_are_strict() -> Result<()> {
    let client = uuid7::new_uuid();
    let body = payload(client, &"a".repeat(64), "pairwise-no-uuid-subject");
    let key = jsonwebtoken::DecodingKey::from_rsa_components(KEY_A_N, "AQAB")?;
    let now = chrono::Utc::now().timestamp();
    let valid = |token: &str| {
        crate::bff::backchannel::valid_fixture(token, &key, ORIGIN, &client.to_string(), now)
    };
    ensure!(valid(&signed(&body)?), "valid fixed signed logout rejected");
    for (field, value) in [
        ("iss", json!("https://other.example.invalid")),
        ("aud", json!(uuid7::new_uuid())),
        ("aud", json!([client])),
        ("sid", json!(uuid7::new_uuid())),
        ("sub", json!("")),
        ("sub", json!("wrong\nsubject")),
        ("jti", json!(Uuid::nil())),
        ("nonce", json!("not-a-logout")),
        ("events", json!({})),
        (
            "events",
            json!({"http://schemas.openid.net/event/backchannel-logout":{"x":1}}),
        ),
        (
            "events",
            json!({"http://schemas.openid.net/event/backchannel-logout":{},"other":{}}),
        ),
        ("iat", json!(now + 6)),
        ("iat", json!(now - 121)),
        ("exp", json!(now)),
        ("exp", json!(now + 121)),
    ] {
        let mut wrong = body.clone();
        wrong[field] = value;
        ensure!(
            !valid(&signed(&wrong)?),
            "invalid logout claim accepted: {field}"
        );
    }
    let mut future = body.clone();
    future["iat"] = json!(now + 6);
    future["exp"] = json!(now + 126);
    ensure!(
        !valid(&signed(&future)?),
        "future iat bypassed exact TTL validation"
    );
    future["iat"] = json!(now + 5);
    future["exp"] = json!(now + 125);
    ensure!(
        valid(&signed(&future)?),
        "permitted five-second clock skew rejected"
    );
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("phase10-a".into());
    let borrowed = jsonwebtoken::encode(
        &header,
        &body,
        &jsonwebtoken::EncodingKey::from_rsa_pem(KEY_A_PEM.as_bytes())?,
    )?;
    ensure!(
        !valid(&borrowed),
        "ID token header was accepted as a logout token"
    );
    let mut damaged = signed(&body)?.into_bytes();
    let index = damaged.len() - 8;
    damaged[index] = if damaged[index] == b'A' { b'B' } else { b'A' };
    ensure!(
        !valid(std::str::from_utf8(&damaged)?),
        "invalid signature accepted"
    );
    Ok(())
}

#[test]
fn logout_rejects_signed_duplicate_claims_and_untrusted_header_extensions() -> Result<()> {
    let client = uuid7::new_uuid();
    let body = payload(client, &"b".repeat(64), "opaque-account");
    let key = jsonwebtoken::DecodingKey::from_rsa_components(KEY_A_N, "AQAB")?;
    let now = chrono::Utc::now().timestamp();
    let valid = |token: &str| {
        crate::bff::backchannel::valid_fixture(token, &key, ORIGIN, &client.to_string(), now)
    };
    let ordinary = serde_json::to_string(&body)?;
    let duplicate = ordinary.replacen('{', &format!("{{\"iss\":{},", json!(ORIGIN)), 1);
    ensure!(
        !valid(&signed_raw(
            &duplicate,
            r#"{"alg":"RS256","kid":"phase10-a","typ":"logout+jwt"}"#
        )?),
        "duplicate signed issuer accepted"
    );
    for header in [
        r#"{"alg":"RS256","kid":"phase10-a","typ":"logout+jwt","jku":"https://attacker.invalid/jwks"}"#,
        r#"{"alg":"RS256","kid":"phase10-a","typ":"logout+jwt","crit":["unsafe"]}"#,
        r#"{"alg":"RS256","kid":"phase10-a","kid":"other","typ":"logout+jwt"}"#,
    ] {
        ensure!(
            !valid(&signed_raw(&ordinary, header)?),
            "unsupported JWT header accepted"
        );
    }
    Ok(())
}
