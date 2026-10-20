use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::util::{hash, jwt::Claims};

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
}

/// 应用令牌必须同时满足数据库撤销状态、签发会话、成员和两级应用授权。
pub async fn inspect(
    pool: &PgPool,
    token: &str,
    claims: &Claims,
) -> Result<Option<AppTokenInfo>, sqlx::Error> {
    if claims.typ != "access" {
        return Ok(None);
    }
    let row: Option<(Uuid, String, Uuid, Uuid, Uuid, String, Uuid, String, DateTime<Utc>, Uuid)> =
        sqlx::query_as(
            "SELECT tok.user_id, tok.client_id, tok.tenant_id, tok.organization_id, \
             tok.application_id, tok.audience, tok.jti, COALESCE(tok.scope,''), tok.expires_at, tok.platform_session_id \
             FROM oauth_tokens tok \
             JOIN oauth_clients client ON client.client_id = tok.client_id \
               AND client.status = 'ENABLED' AND client.client_type = 'CONFIDENTIAL' \
               AND client.application_id = tok.application_id \
             JOIN applications app ON app.id = tok.application_id \
               AND app.status = 'ENABLED' AND app.code = tok.audience \
             JOIN users u ON u.id = tok.user_id AND u.status = 'ENABLED' \
             JOIN organizations o ON o.id = tok.organization_id AND o.tenant_id = tok.tenant_id \
               AND org_chain_active(o.id) \
             JOIN tenants tenant ON tenant.id = tok.tenant_id \
               AND tenant.status = 'ENABLED' \
               AND (tenant.expires_at IS NULL OR tenant.expires_at > now()) \
             JOIN memberships member ON member.user_id = tok.user_id \
               AND member.organization_id = tok.organization_id \
             JOIN tenant_applications ta ON ta.tenant_id = tok.tenant_id \
               AND ta.application_id = tok.application_id \
             JOIN organization_applications oa ON oa.organization_id = tok.organization_id \
               AND oa.application_id = tok.application_id \
             JOIN platform_sessions session ON session.id = tok.platform_session_id \
               AND session.user_id = tok.user_id AND session.tenant_id = tok.tenant_id \
               AND session.organization_id = tok.organization_id \
               AND session.revoked_at IS NULL AND session.expires_at > now() \
             WHERE tok.token_hash = $1 AND tok.token_type = 'access' \
               AND tok.token_use = 'app' AND tok.revoked = false AND tok.expires_at > now()",
        )
        .bind(hash::sha256_hex(token))
        .fetch_optional(pool)
        .await?;
    let Some((
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
    )) = row
    else {
        return Ok(None);
    };
    if claims.sub != user_id.to_string()
        || claims.tenant.as_deref() != Some(tenant_id.to_string().as_str())
        || claims.org.as_deref() != Some(organization_id.to_string().as_str())
        || claims.cid.as_deref() != Some(client_id.as_str())
        || claims.aud.as_deref() != Some(audience.as_str())
        || claims.jti.as_deref() != Some(jti.to_string().as_str())
    {
        return Ok(None);
    }
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
    }))
}
