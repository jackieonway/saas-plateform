//! Notification-only session proof. Employee administration keeps `CurrentUser`.
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{error::AppError, extractors::bearer_token, state::AppState, util::jwt};

#[derive(sqlx::FromRow)]
pub(crate) struct NotificationContext {
    pub(crate) user_id: Uuid,
    pub(crate) username: String,
    pub(crate) tenant_id: Uuid,
    pub(crate) org_id: Uuid,
    pub(crate) session_id: Uuid,
    pub(crate) membership_id: Uuid,
    pub(crate) member_type: String,
    pub(crate) identity_version: i64,
    pub(crate) account_version: i64,
    pub(crate) expires_at: DateTime<Utc>,
}

const FRESH: &str = "SELECT EXISTS(SELECT 1 FROM platform_sessions s \
    WHERE s.id=$1 AND s.user_id=$2 AND s.tenant_id=$3 AND s.organization_id=$4 \
    AND s.membership_id=$5 AND s.member_type=$6 AND s.identity_version=$7 \
    AND s.account_version=$8 AND $9::timestamptz>clock_timestamp() \
    AND platform_session_active(s.id))";

impl NotificationContext {
    pub(crate) async fn revalidate(&self, pool: &PgPool) -> Result<(), AppError> {
        let valid: bool = sqlx::query_scalar(FRESH)
            .bind(self.session_id)
            .bind(self.user_id)
            .bind(self.tenant_id)
            .bind(self.org_id)
            .bind(self.membership_id)
            .bind(&self.member_type)
            .bind(self.identity_version)
            .bind(self.account_version)
            .bind(self.expires_at)
            .fetch_one(pool)
            .await
            .map_err(|_| AppError::Unavailable)?;
        Self::valid(valid)
    }

    /// A new READ COMMITTED statement after row-lock waits, before committing.
    pub(crate) async fn revalidate_in_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<(), AppError> {
        let valid: bool = sqlx::query_scalar(FRESH)
            .bind(self.session_id)
            .bind(self.user_id)
            .bind(self.tenant_id)
            .bind(self.org_id)
            .bind(self.membership_id)
            .bind(&self.member_type)
            .bind(self.identity_version)
            .bind(self.account_version)
            .bind(self.expires_at)
            .fetch_one(&mut **tx)
            .await
            .map_err(|_| AppError::Unavailable)?;
        Self::valid(valid)
    }

    fn valid(valid: bool) -> Result<(), AppError> {
        if valid {
            Ok(())
        } else {
            Err(AppError::unauthorized("通知成员会话已失效，请重新认证"))
        }
    }
}

impl FromRequestParts<AppState> for NotificationContext {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        let claims = jwt::verify_internal(
            &bearer_token(&parts.headers)?,
            &state.config.jwt_secret,
            jwt::InternalUse::PlatformSession,
            Some(jwt::PLATFORM_API_AUDIENCE),
        )
        .map_err(|_| AppError::unauthorized("会话凭证无效或已过期"))?;
        if claims.typ != "platform_session" {
            return Err(AppError::unauthorized("通知需要具体成员会话"));
        }
        let id = |value: Option<&str>| {
            value
                .and_then(|value| crate::util::uuid7::parse_uuid(value).ok())
                .ok_or_else(|| AppError::unauthorized("会话缺少有效绑定，请重新认证"))
        };
        let user_id = id(Some(&claims.sub))?;
        let tenant_id = id(claims.tenant.as_deref())?;
        let org_id = id(claims.org.as_deref())?;
        let session_id = id(claims.sid.as_deref())?;
        let account_version = claims
            .ver
            .filter(|version| *version >= 1)
            .ok_or_else(|| AppError::unauthorized("会话缺少账号版本，请重新认证"))?;
        let expiry = DateTime::<Utc>::from_timestamp(claims.exp, 0)
            .filter(|expiry| *expiry > Utc::now())
            .ok_or_else(|| AppError::unauthorized("会话已过期"))?;
        let context: Option<Self> = sqlx::query_as(
            "SELECT s.user_id,u.username,s.tenant_id,s.organization_id AS org_id,s.id AS session_id, \
             s.membership_id,s.member_type,s.identity_version,s.account_version,LEAST(s.expires_at,$6) AS expires_at \
             FROM platform_sessions s JOIN users u ON u.id=s.user_id \
             WHERE s.id=$1 AND s.user_id=$2 AND s.tenant_id=$3 AND s.organization_id=$4 \
             AND s.account_version=$5 AND s.member_type IN ('EMPLOYEE','CUSTOMER') \
             AND platform_session_active(s.id)")
            .bind(session_id).bind(user_id).bind(tenant_id).bind(org_id).bind(account_version).bind(expiry)
            .fetch_optional(&state.pool).await.map_err(|_| AppError::Unavailable)?;
        let context =
            context.ok_or_else(|| AppError::unauthorized("通知成员会话已失效，请重新认证"))?;
        crate::bff::require_platform_session(state, context.session_id).await?;
        context.revalidate(&state.pool).await?;
        Ok(context)
    }
}
