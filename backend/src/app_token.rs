use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::util::{
    hash,
    jwt::Claims,
    uuid7::{is_uuid_v7, parse_uuid},
};
use crate::{error::AppError, state::AppState};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AppTokenInfo {
    pub user_id: Uuid,
    pub client_id: String,
    pub tenant_id: Uuid,
    pub organization_id: Uuid,
    pub application_id: Uuid,
    pub audience: String,
    pub jti: Uuid,
    pub scope: String,
    pub expires_at: DateTime<Utc>,
    pub session_id: Uuid,
    pub(crate) membership_id: Uuid,
    pub(crate) member_type: String,
    pub(crate) identity_version: i64,
    pub(crate) account_version: i64,
}

type AppTokenRow = (
    Uuid,
    String,
    Uuid,
    Uuid,
    Uuid,
    String,
    Uuid,
    String,
    DateTime<Utc>,
    Uuid,
    Uuid,
    String,
    i64,
    i64,
);

fn valid_claim_ids(claims: &Claims) -> bool {
    [
        Some(claims.sub.as_str()),
        claims.tenant.as_deref(),
        claims.org.as_deref(),
        claims.jti.as_deref(),
    ]
    .into_iter()
    .all(|value| value.is_some_and(|value| parse_uuid(value).is_ok()))
        && claims
            .sid
            .as_deref()
            .is_none_or(|value| parse_uuid(value).is_ok())
}

fn matches_binding(claims: &Claims, row: &AppTokenRow) -> bool {
    let (
        user_id,
        client_id,
        tenant_id,
        org_id,
        app_id,
        audience,
        jti,
        _,
        _,
        session_id,
        membership_id,
        member_type,
        identity_version,
        account_version,
    ) = row;
    [
        *user_id,
        *tenant_id,
        *org_id,
        *app_id,
        *jti,
        *session_id,
        *membership_id,
    ]
    .into_iter()
    .all(is_uuid_v7)
        && matches!(member_type.as_str(), "EMPLOYEE" | "CUSTOMER")
        && *identity_version >= 1
        && *account_version >= 0
        && claims.sub == user_id.to_string()
        && claims.tenant.as_deref() == Some(tenant_id.to_string().as_str())
        && claims.org.as_deref() == Some(org_id.to_string().as_str())
        && claims.cid.as_deref() == Some(client_id.as_str())
        && claims.aud.as_deref() == Some(audience.as_str())
        && claims.jti.as_deref() == Some(jti.to_string().as_str())
        && claims
            .sid
            .as_deref()
            .is_none_or(|sid| sid == session_id.to_string())
}

/// 应用令牌必须同时满足数据库撤销状态、签发会话、成员和两级应用授权。
pub async fn inspect(
    pool: &PgPool,
    token: &str,
    claims: &Claims,
) -> Result<Option<AppTokenInfo>, sqlx::Error> {
    if claims.typ != "access" || !valid_claim_ids(claims) {
        return Ok(None);
    }
    let row: Option<AppTokenRow> =
        sqlx::query_as(
            "SELECT tok.user_id, tok.client_id, tok.tenant_id, tok.organization_id, \
             tok.application_id, tok.audience, tok.jti, COALESCE(tok.scope,''), tok.expires_at, tok.platform_session_id, \
             tok.membership_id,tok.member_type,tok.identity_version,tok.account_version \
             FROM oauth_tokens tok \
             JOIN oauth_clients client ON client.client_id = tok.client_id \
               AND client.status = 'ENABLED' AND client.client_type = 'CONFIDENTIAL' \
               AND client.application_id = tok.application_id \
               AND client.access_policy_version = tok.client_policy_version \
               AND tok.application_service_client_ref = client.client_id \
               AND (client.subject_mode = 'MACHINE' OR \
                    (client.subject_mode IN ('USER','LEGACY_MIXED') \
                     AND tok.member_type = ANY(client.allowed_member_types))) \
             JOIN applications app ON app.id = tok.application_id \
               AND app.status = 'ENABLED' AND app.code = tok.audience \
               AND app.access_policy_version = tok.application_policy_version \
               AND tok.member_type = ANY(app.allowed_member_types) \
             JOIN users u ON u.id = tok.user_id AND u.status = 'ENABLED' \
               AND u.session_version = tok.account_version \
             JOIN organizations o ON o.id = tok.organization_id AND o.tenant_id = tok.tenant_id \
               AND org_chain_active(o.id) \
             JOIN tenants tenant ON tenant.id = tok.tenant_id \
               AND tenant.status = 'ENABLED' \
               AND (tenant.expires_at IS NULL OR tenant.expires_at > now()) \
             JOIN memberships member ON member.id = tok.membership_id \
               AND public.is_uuid_v7(tok.membership_id) \
               AND member.user_id = tok.user_id \
               AND member.organization_id = tok.organization_id \
               AND member.member_type = tok.member_type AND member.status='ENABLED' \
               AND member.identity_version = tok.identity_version \
             JOIN tenant_applications ta ON ta.tenant_id = tok.tenant_id \
               AND ta.application_id = tok.application_id \
             JOIN organization_applications oa ON oa.organization_id = tok.organization_id \
               AND oa.application_id = tok.application_id \
             JOIN platform_sessions session ON session.id = tok.platform_session_id \
               AND session.user_id = tok.user_id AND session.tenant_id = tok.tenant_id \
               AND session.organization_id = tok.organization_id \
               AND session.membership_id = tok.membership_id \
               AND session.member_type = tok.member_type \
               AND session.identity_version = tok.identity_version \
               AND session.account_version = tok.account_version \
               AND session.origin_issuer = tok.origin_issuer \
               AND session.origin_client_ref = tok.origin_client_ref \
               AND session.origin_client_policy_version = tok.origin_client_policy_version \
               AND public.platform_session_active(session.id) \
             WHERE tok.token_hash = $1 AND tok.token_type = 'access' \
               AND tok.token_use = 'app' AND tok.revoked = false AND tok.expires_at > now()",
        )
        .bind(hash::sha256_hex(token))
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    if !matches_binding(claims, &row) {
        return Ok(None);
    }
    let (
        user_id,
        client_id,
        tenant_id,
        organization_id,
        application_id,
        audience,
        jti,
        scope,
        expires_at,
        session_id,
        membership_id,
        member_type,
        identity_version,
        account_version,
    ) = row;
    Ok(Some(AppTokenInfo {
        user_id,
        client_id,
        tenant_id,
        organization_id,
        application_id,
        audience,
        jti,
        scope,
        expires_at,
        session_id,
        membership_id,
        member_type,
        identity_version,
        account_version,
    }))
}

/// The production boundary also proves the originating BFF authentication over
/// its configured back channel. Pure database consumers retain `inspect`.
pub(crate) async fn inspect_authorized(
    state: &AppState,
    token: &str,
    claims: &Claims,
) -> Result<Option<AppTokenInfo>, AppError> {
    let info = inspect(&state.pool, token, claims)
        .await
        .map_err(|_| AppError::Unavailable)?;
    if let Some(info) = &info {
        match crate::bff::require_platform_session(state, info.session_id).await {
            Ok(()) => {}
            // Preserve inactive-token semantics for protocol introspection.
            Err(AppError::Unauthorized(_) | AppError::Forbidden(_)) => return Ok(None),
            Err(error) => return Err(error),
        }
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::uuid7::fixture_uuid;

    fn fixture() -> (Claims, AppTokenRow) {
        let row: AppTokenRow = (
            fixture_uuid(1),
            "opaque-client-id".into(),
            fixture_uuid(2),
            fixture_uuid(3),
            fixture_uuid(4),
            "opaque-app-code".into(),
            fixture_uuid(5),
            "profile".into(),
            Utc::now(),
            fixture_uuid(6),
            fixture_uuid(7),
            "EMPLOYEE".into(),
            1,
            0,
        );
        let claims = Claims {
            sub: row.0.to_string(),
            typ: "access".into(),
            tenant: Some(row.2.to_string()),
            org: Some(row.3.to_string()),
            cid: Some(row.1.clone()),
            scope: Some(row.7.clone()),
            sid: None,
            ver: None,
            aud: Some(row.5.clone()),
            jti: Some(row.6.to_string()),
            exp: Utc::now().timestamp() + 60,
            iat: Utc::now().timestamp(),
        };
        (claims, row)
    }

    fn invalid_ids() -> [Uuid; 3] {
        [
            Uuid::nil(),
            Uuid::new_v4(),
            Uuid::parse_str("019a1163-0000-7000-c000-000000000001").unwrap(),
        ]
    }

    #[tokio::test]
    async fn invalid_claim_ids_are_inactive_before_any_database_lookup() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/unused_app_token_uuid_test")
            .unwrap();
        let (base, _) = fixture();
        assert!(valid_claim_ids(&base));
        for invalid in invalid_ids() {
            for field in 0..5 {
                let mut claims = base.clone();
                let id = invalid.to_string();
                match field {
                    0 => claims.sub = id,
                    1 => claims.tenant = Some(id),
                    2 => claims.org = Some(id),
                    3 => claims.jti = Some(id),
                    _ => claims.sid = Some(id),
                }
                assert!(inspect(&pool, "token", &claims).await.unwrap().is_none());
            }
        }
        for field in 0..3 {
            let mut claims = base.clone();
            match field {
                0 => claims.tenant = None,
                1 => claims.org = None,
                _ => claims.jti = None,
            }
            assert!(inspect(&pool, "token", &claims).await.unwrap().is_none());
        }
    }

    #[test]
    fn stored_binding_requires_v7_and_optional_sid_matches_the_session() {
        let (base, row) = fixture();
        assert!(matches_binding(&base, &row));
        let mut with_sid = base.clone();
        with_sid.sid = Some(row.9.to_string());
        assert!(valid_claim_ids(&with_sid));
        assert!(matches_binding(&with_sid, &row));
        with_sid.sid = Some(fixture_uuid(99).to_string());
        assert!(!matches_binding(&with_sid, &row));
        for invalid in invalid_ids() {
            for field in 0..7 {
                let mut stored = row.clone();
                let mut claims = base.clone();
                match field {
                    0 => {
                        stored.0 = invalid;
                        claims.sub = invalid.to_string();
                    }
                    1 => {
                        stored.2 = invalid;
                        claims.tenant = Some(invalid.to_string());
                    }
                    2 => {
                        stored.3 = invalid;
                        claims.org = Some(invalid.to_string());
                    }
                    3 => stored.4 = invalid,
                    4 => {
                        stored.6 = invalid;
                        claims.jti = Some(invalid.to_string());
                    }
                    5 => stored.9 = invalid,
                    _ => stored.10 = invalid,
                }
                assert!(!matches_binding(&claims, &stored));
            }
        }
        let mut customer = row.clone();
        customer.11 = "CUSTOMER".into();
        assert!(matches_binding(&base, &customer));
        let mut unknown = row.clone();
        unknown.11 = "UNKNOWN".into();
        assert!(!matches_binding(&base, &unknown));
        let mut invalid_identity = row.clone();
        invalid_identity.12 = 0;
        assert!(!matches_binding(&base, &invalid_identity));
        let mut invalid_account = row.clone();
        invalid_account.13 = -1;
        assert!(!matches_binding(&base, &invalid_account));
    }
}
