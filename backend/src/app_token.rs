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
            .is_some_and(|value| parse_uuid(value).is_ok())
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
        && *account_version >= 1
        && claims.sub == user_id.to_string()
        && claims.tenant.as_deref() == Some(tenant_id.to_string().as_str())
        && claims.org.as_deref() == Some(org_id.to_string().as_str())
        && claims.cid.as_deref() == Some(client_id.as_str())
        && claims.aud.as_deref() == Some(audience.as_str())
        && claims.jti.as_deref() == Some(jti.to_string().as_str())
        && claims.sid.as_deref() == Some(session_id.to_string().as_str())
        && claims.ver == Some(*account_version)
        && claims.scope.as_deref() == Some(row.7.as_str())
        && claims.exp == row.8.timestamp()
}

/// 应用令牌必须同时满足数据库撤销状态、签发会话、成员和两级应用授权。
pub async fn inspect(
    pool: &PgPool,
    token: &str,
    claims: &Claims,
) -> Result<Option<AppTokenInfo>, sqlx::Error> {
    if claims.typ != "app_session" || !valid_claim_ids(claims) {
        return Ok(None);
    }
    let row: Option<AppTokenRow> =
        sqlx::query_as(
            "SELECT tok.user_id,tok.audience AS client_id,tok.tenant_id,tok.organization_id, \
             tok.application_id,tok.audience,tok.jti,tok.scope,tok.expires_at,tok.platform_session_id, \
             tok.membership_id,tok.member_type,tok.identity_version,tok.account_version \
             FROM platform_app_capabilities tok WHERE tok.token_hash=$1 \
             AND platform_app_capability_active(tok.token_hash)",
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
    let verified = crate::util::jwt::verify_internal(
        token,
        &state.config.jwt_secret,
        crate::util::jwt::InternalUse::AppSession,
        claims.aud.as_deref(),
    )
    .map_err(|_| AppError::unauthorized("应用能力用途或来源无效"))?;
    let info = inspect(&state.pool, token, &verified)
        .await
        .map_err(|_| AppError::Unavailable)?;
    if let Some(info) = &info {
        match crate::bff::require_platform_session(state, info.session_id).await {
            Ok(()) => {}
            // Preserve inactive-token semantics for protocol introspection.
            Err(AppError::Unauthorized(_) | AppError::Forbidden(_)) => return Ok(None),
            Err(error) => return Err(error),
        }
        let current = crate::util::jwt::verify_internal(
            token,
            &state.config.jwt_secret,
            crate::util::jwt::InternalUse::AppSession,
            Some(&info.audience),
        )
        .map_err(|_| AppError::unauthorized("应用能力已在来源验证期间过期"))?;
        if inspect(&state.pool, token, &current)
            .await
            .map_err(|_| AppError::Unavailable)?
            .as_ref()
            != Some(info)
        {
            return Ok(None);
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
            DateTime::from_timestamp(Utc::now().timestamp() + 60, 0).unwrap(),
            fixture_uuid(6),
            fixture_uuid(7),
            "EMPLOYEE".into(),
            1,
            1,
        );
        let claims = Claims {
            sub: row.0.to_string(),
            typ: "app_session".into(),
            tenant: Some(row.2.to_string()),
            org: Some(row.3.to_string()),
            cid: Some(row.1.clone()),
            scope: Some(row.7.clone()),
            sid: Some(row.9.to_string()),
            ver: Some(row.13),
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
    fn stored_binding_requires_original_sid_version_scope_and_deadline() {
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
        for account in [-1, 0] {
            let mut invalid_account = row.clone();
            invalid_account.13 = account;
            assert!(!matches_binding(&base, &invalid_account));
        }
    }
}
