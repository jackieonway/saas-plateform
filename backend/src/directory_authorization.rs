//! Explicit request-local authority for internal and private resource directories.
use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use saas_platform_backend::snapshot::Revision;
use serde_json::Value;

use crate::{
    app_authorization::{self, RequestContext},
    confidential_clients::{self, Caller},
    error::AppError,
    resource_authority::{self, Proof, Resource},
    state::AppState,
};

const WINDOW_SECONDS: i64 = 3;

pub(crate) enum Purpose {
    Members,
    Organizations,
    Notification,
}

/// Public Bearer routes can construct only Internal. Private POST routes first
/// authenticate a resource service and retain the original credential domain.
pub(crate) enum Access<'a> {
    Internal(&'a HeaderMap),
    Private(&'a PrivateAccess),
}

impl Access<'_> {
    pub(crate) async fn context(
        &self,
        state: &AppState,
        code: &str,
    ) -> Result<RequestContext, AppError> {
        match self {
            Self::Internal(headers) => {
                app_authorization::request_context(state, code, headers).await
            }
            Self::Private(access) => {
                access.live(code)?;
                Ok(RequestContext {
                    context: access.initial.context.clone(),
                    revision: access.initial.revision,
                    binding: access.initial.binding.clone(),
                })
            }
        }
    }

    pub(crate) async fn revalidate(
        &self,
        state: &AppState,
        code: &str,
        expected: Revision,
    ) -> Result<Option<Value>, AppError> {
        match self {
            Self::Internal(headers) => {
                app_authorization::revalidate_context(state, code, headers, expected).await
            }
            Self::Private(access) => {
                access.revalidate(state, code, expected).await?;
                Ok(None)
            }
        }
    }
}

enum Domain {
    AppSession,
    Canonical(Box<Proof>),
}

/// Contains an access token and service headers. Never Debug, Serialize or log.
pub(crate) struct PrivateAccess {
    code: String,
    headers: HeaderMap,
    token: String,
    caller: Caller,
    resource: Resource,
    domain: Domain,
    initial: RequestContext,
    expires_at: DateTime<Utc>,
}

fn invalid() -> AppError {
    AppError::unauthorized("目录服务凭据或原认证来源无效")
}

pub(crate) fn token_valid(token: &str) -> bool {
    !token.is_empty() && token.len() <= 8185 && !token.chars().any(char::is_whitespace)
}

fn employee(checked: &RequestContext, caller: &Caller) -> Result<(), AppError> {
    if checked.binding.member_type != "EMPLOYEE"
        || !caller
            .allowed_member_types
            .iter()
            .any(|kind| kind == "EMPLOYEE")
    {
        return Err(AppError::forbidden("员工目录不接受客户或机器身份"));
    }
    Ok(())
}

fn context_deadline(checked: &RequestContext) -> Result<DateTime<Utc>, AppError> {
    checked.context["expiresAt"]
        .as_str()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc).min(checked.binding.expires_at))
        .ok_or(AppError::Unavailable)
}

impl PrivateAccess {
    pub(crate) async fn authenticate(
        state: &AppState,
        code: &str,
        headers: HeaderMap,
        token: String,
        resource: &str,
        action: &str,
        purpose: Purpose,
    ) -> Result<Self, AppError> {
        let deadline = Utc::now() + chrono::Duration::seconds(WINDOW_SECONDS);
        if !token_valid(&token) {
            return Err(invalid());
        }
        let caller = confidential_clients::authenticate(state, &headers).await?;
        let resource = resource_authority::resource(state, &caller, code, resource, action).await?;
        let policy = &state.config.data_grant_policy;
        let registered = match purpose {
            Purpose::Members => policy
                .assignment_members(
                    caller.application_id,
                    &resource.resource_code,
                    &resource.action_code,
                )
                .is_some(),
            Purpose::Organizations => policy
                .organization_lookup(
                    caller.application_id,
                    &resource.resource_code,
                    &resource.action_code,
                )
                .is_some(),
            Purpose::Notification => {
                resource.resource_code == "notification"
                    && resource.action_code == "publish"
                    && policy
                        .operation(caller.application_id, "notification", "publish")
                        .is_some()
            }
        };
        if !registered {
            return Err(AppError::bad_request("该资源动作未登记当前目录用途"));
        }
        let (domain, initial) = if resource_authority::is_internal(state, &token).await? {
            let checked = app_authorization::request_context_bound(
                state,
                code,
                &token,
                Some(caller.application_id),
            )
            .await?;
            (Domain::AppSession, checked)
        } else {
            let _permit = state
                .app_authorization_slots
                .try_acquire()
                .map_err(|_| AppError::Unavailable)?;
            let proof = resource_authority::verify(state, &token, &resource).await?;
            if proof.subject_mode != "USER"
                || proof
                    .context
                    .as_ref()
                    .is_none_or(|context| context.member_type != "EMPLOYEE")
            {
                return Err(AppError::forbidden("员工目录不接受客户或机器身份"));
            }
            let checked = app_authorization::external_context(state, &proof, &resource).await?;
            (Domain::Canonical(Box::new(proof)), checked)
        };
        employee(&initial, &caller)?;
        let expires_at = deadline.min(context_deadline(&initial)?);
        if expires_at <= Utc::now() {
            return Err(AppError::Unavailable);
        }
        Ok(Self {
            code: code.to_owned(),
            headers,
            token,
            caller,
            resource,
            domain,
            initial,
            expires_at,
        })
    }

    fn live(&self, code: &str) -> Result<(), AppError> {
        if self.code != code {
            return Err(invalid());
        }
        if self.expires_at <= Utc::now() {
            return Err(AppError::Unavailable);
        }
        Ok(())
    }

    pub(crate) async fn revalidate(
        &self,
        state: &AppState,
        code: &str,
        expected: Revision,
    ) -> Result<(), AppError> {
        self.live(code)?;
        let latest = match &self.domain {
            Domain::AppSession => {
                app_authorization::request_context_bound(
                    state,
                    code,
                    &self.token,
                    Some(self.caller.application_id),
                )
                .await?
            }
            Domain::Canonical(original) => {
                let _permit = state
                    .app_authorization_slots
                    .try_acquire()
                    .map_err(|_| AppError::Unavailable)?;
                let proof = resource_authority::verify(state, &self.token, &self.resource).await?;
                if &proof != original.as_ref() {
                    return Err(invalid());
                }
                app_authorization::external_context(state, &proof, &self.resource).await?
            }
        };
        employee(&latest, &self.caller)?;
        if latest.binding != self.initial.binding {
            return Err(invalid());
        }
        if expected != self.initial.revision || latest.revision != expected {
            return Err(AppError::Unavailable);
        }
        if confidential_clients::recheck(state, &self.headers).await? != self.caller
            || resource_authority::resource(
                state,
                &self.caller,
                code,
                &self.resource.resource_code,
                &self.resource.action_code,
            )
            .await?
                != self.resource
        {
            return Err(invalid());
        }
        if context_deadline(&latest)? < self.expires_at {
            return Err(AppError::Unavailable);
        }
        self.live(code)
    }

    /// Synchronous after the final target-row read: no new authority wait may
    /// extend this request's original three-second directory window.
    pub(crate) fn expires_at(&self) -> Result<DateTime<Utc>, AppError> {
        self.live(&self.code)?;
        Ok(self.expires_at)
    }

    pub(crate) fn initial(&self) -> &RequestContext {
        &self.initial
    }

    /// Digest only: recovery remains in the exact original credential/source
    /// namespace and cannot acquire a replacement organization's authority.
    pub(crate) fn source_digest(&self) -> String {
        let authority = match &self.domain {
            Domain::AppSession => serde_json::json!({"domain":"INTERNAL_APP_SESSION"}),
            Domain::Canonical(proof) => {
                serde_json::json!({"domain":"CANONICAL_USER", "proof":proof})
            }
        };
        crate::util::hash::sha256_hex(
            &serde_json::json!([
                "resource-notification/1",
                self.code,
                crate::util::hash::sha256_hex(&self.token),
                authority,
                self.initial.binding
            ])
            .to_string(),
        )
    }

    /// No HTTP awaits while a publication transaction owns foreign-key locks.
    /// The online source was checked immediately before starting that transaction;
    /// this final snapshot checks every locally owned binding and policy again.
    pub(crate) async fn revalidate_local(&self, state: &AppState) -> Result<(), AppError> {
        self.live(&self.code)?;
        let latest = match &self.domain {
            Domain::AppSession => {
                let claims = crate::util::jwt::verify_internal(
                    &self.token,
                    &state.config.jwt_secret,
                    crate::util::jwt::InternalUse::AppSession,
                    Some(&self.code),
                )
                .map_err(|_| invalid())?;
                let binding = crate::app_token::inspect(&state.pool, &self.token, &claims)
                    .await?
                    .ok_or_else(invalid)?;
                app_authorization::local_resource_context(state, binding, true).await?
            }
            Domain::Canonical(proof) => {
                app_authorization::external_context(state, proof, &self.resource).await?
            }
        };
        employee(&latest, &self.caller)?;
        if latest.binding != self.initial.binding
            || latest.revision != self.initial.revision
            || confidential_clients::recheck(state, &self.headers).await? != self.caller
            || resource_authority::resource(
                state,
                &self.caller,
                &self.code,
                &self.resource.resource_code,
                &self.resource.action_code,
            )
            .await?
                != self.resource
        {
            return Err(invalid());
        }
        if context_deadline(&latest)? < self.expires_at {
            return Err(AppError::Unavailable);
        }
        self.live(&self.code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_directory_token_bounds_reject_ambiguous_input() {
        assert!(token_valid("opaque-token-value"));
        for value in [String::new(), " a".into(), "a\nb".into(), "a".repeat(8186)] {
            assert!(!token_valid(&value));
        }
    }
}
