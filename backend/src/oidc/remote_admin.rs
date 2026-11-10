//! Fresh platform authorization is committed before private OIDC commands.
//! Unknown results are reconciled by operation lookup, never command replay.

use crate::{
    error::AppError,
    extractors::{ApiJson, CurrentUser},
    state::AppState,
    util::uuid7,
};
use axum::{
    extract::{DefaultBodyLimit, Extension, Path, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;

const LIMIT: usize = 65536;
const PREFIX: &str = "/internal/identity/auth";
const AUTH_PREFIX: &str = "/internal/identity/auth";
const AUTH_CLIENTS_PREFIX: &str = "/internal/identity/auth/clients";
const AUTH_CONSENT_GROUPS_PREFIX: &str = "/internal/identity/auth/consent-groups";
const REVIEW_PREFIX: &str = "/internal/identity/auth/review/applications";
const INVITATIONS_PREFIX: &str = "/internal/identity/auth/invitations";
#[derive(Clone)]
struct Deadline {
    at: tokio::time::Instant,
    dispatched: Arc<Mutex<Option<Uuid>>>,
}
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/identity/applications/{id}/projection",
            get(get_application_projection).put(project_application),
        )
        .route("/api/identity/applications/operations/{id}", get(operation))
        .route(
            "/api/identity/clients",
            get(list_auth_clients).post(create_auth_client),
        )
        .route(
            "/api/identity/clients/{id}",
            get(get_auth_client).patch(update_auth_client),
        )
        .route(
            "/api/identity/clients/{id}/credentials",
            post(rotate_auth_client_credential),
        )
        .route(
            "/api/identity/clients/{id}/credentials/revoke",
            post(revoke_auth_client_credentials),
        )
        .route("/api/identity/client-operations/{id}", get(operation))
        .layer(DefaultBodyLimit::max(LIMIT))
        .route_layer(middleware::from_fn(bounded))
}
async fn bounded(mut request: Request, next: Next) -> Response {
    let deadline = Deadline {
        at: tokio::time::Instant::now() + Duration::from_secs(3),
        dispatched: Arc::new(Mutex::new(None)),
    };
    request.extensions_mut().insert(deadline.clone());
    let client_list_query =
        request.method() == Method::GET && request.uri().path() == "/api/identity/clients";
    let mut response = if request.uri().query().is_some() && !client_list_query {
        AppError::bad_request("认证管理入口不接受查询参数").into_response()
    } else {
        match tokio::time::timeout_at(deadline.at, next.run(request)).await {
            Ok(response) => response,
            Err(_) => Failure {
                error: AppError::Unavailable,
                operation: deadline.dispatched.lock().ok().and_then(|value| *value),
            }
            .into_response(),
        }
    };
    for (name, value) in [
        ("cache-control", "no-store"),
        ("pragma", "no-cache"),
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    response
}
fn id(raw: &str) -> Result<Uuid, AppError> {
    let value = uuid7::parse_uuid(raw).map_err(|_| AppError::bad_request("OIDC 标识无效"))?;
    if value.to_string() != raw {
        return Err(AppError::bad_request("OIDC 标识无效"));
    }
    Ok(value)
}
fn kid(raw: &str) -> Result<String, AppError> {
    if raw.is_empty()
        || raw.len() > 80
        || !raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return Err(AppError::bad_request("kid 无效"));
    }
    Ok(raw.into())
}

fn validate_client_list_query(mut query: ClientListQuery) -> Result<ClientListQuery, AppError> {
    for raw in [&query.application_id, &query.partner_id]
        .into_iter()
        .flatten()
    {
        id(raw)?;
    }
    if query
        .ownership
        .as_deref()
        .is_some_and(|v| !matches!(v, "FIRST_PARTY" | "THIRD_PARTY"))
        || query
            .client_type
            .as_deref()
            .is_some_and(|v| !matches!(v, "PUBLIC" | "CONFIDENTIAL"))
        || query
            .subject_mode
            .as_deref()
            .is_some_and(|v| !matches!(v, "USER" | "MACHINE"))
        || query
            .status
            .as_deref()
            .is_some_and(|v| !matches!(v, "ENABLED" | "DISABLED" | "RETIRED"))
        || query.limit.is_some_and(|v| !(1..=100).contains(&v))
        || query.offset.is_some_and(|v| !(0..=100_000).contains(&v))
    {
        return Err(AppError::bad_request("统一客户端筛选或分页参数无效"));
    }
    if let Some(environment) = query.environment.as_mut() {
        *environment = environment.to_ascii_lowercase();
        if environment.is_empty()
            || environment.len() > 32
            || !environment
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
            || !environment
                .as_bytes()
                .first()
                .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        {
            return Err(AppError::bad_request("客户端环境筛选无效"));
        }
    }
    Ok(query)
}
fn opaque(raw: &str) -> bool {
    raw.len() == 64
        && raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn public_client_identifier(raw: &str) -> bool {
    // auth_clients.client_id is a protocol identifier, separate from its
    // UUIDv7 row id. Match the canonical SQL character bound and the identity
    // protocol's control-character rejection without normalizing its value.
    !raw.is_empty() && raw.chars().count() <= 100 && !raw.chars().any(char::is_control)
}
#[derive(Clone)]
struct Target {
    method: Method,
    path: String,
    sync_read_permission: Option<&'static str>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClientListQuery {
    application_id: Option<String>,
    partner_id: Option<String>,
    ownership: Option<String>,
    environment: Option<String>,
    client_type: Option<String>,
    subject_mode: Option<String>,
    status: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}
impl Target {
    fn new(method: Method, relative: &str) -> Result<Self, AppError> {
        let valid = match (method.as_str(), relative) {
            ("POST", "/keys/publish" | "/keys/activate") => true,
            ("DELETE", path) if path.starts_with("/keys/") => kid(&path[6..]).is_ok(),
            ("POST", path) if path.starts_with("/keys/") && path.ends_with("/revoke") => {
                kid(&path[6..path.len() - 7]).is_ok()
            }
            ("GET", path) if path.starts_with("/key-operations/") => id(&path[16..]).is_ok(),
            _ => false,
        };
        if !valid {
            return Err(AppError::bad_request("认证管理路径无效"));
        }
        Ok(Self {
            method,
            path: format!("{PREFIX}{relative}"),
            sync_read_permission: None,
        })
    }
    fn project(application: Uuid) -> Self {
        Self {
            method: Method::PUT,
            path: format!("{AUTH_PREFIX}/applications/{application}"),
            sync_read_permission: None,
        }
    }
    fn get_projection(application: Uuid) -> Self {
        Self {
            method: Method::GET,
            path: format!("{AUTH_PREFIX}/applications/{application}"),
            sync_read_permission: None,
        }
    }
    fn auth_client(method: Method, suffix: &str) -> Result<Self, AppError> {
        let target = Self {
            method,
            path: format!("{AUTH_CLIENTS_PREFIX}{suffix}"),
            sync_read_permission: None,
        };
        if !valid_auth_client_target(&target.method, target.route_path()) {
            return Err(AppError::bad_request("统一客户端管理路径无效"));
        }
        Ok(target)
    }
    #[cfg(test)]
    fn consent_group_list(application: Uuid) -> Self {
        Self {
            method: Method::GET,
            path: format!("{AUTH_PREFIX}/applications/{application}/consent-groups"),
            sync_read_permission: None,
        }
    }
    fn consent_group_create() -> Self {
        Self {
            method: Method::POST,
            path: AUTH_CONSENT_GROUPS_PREFIX.into(),
            sync_read_permission: None,
        }
    }
    fn consent_group_member(group: Uuid) -> Self {
        Self {
            method: Method::POST,
            path: format!("{AUTH_CONSENT_GROUPS_PREFIX}/{group}/clients"),
            sync_read_permission: None,
        }
    }
    fn review(method: Method, application: Uuid, environment: &str) -> Result<Self, AppError> {
        if (method != Method::GET && method != Method::PUT)
            || !matches!(environment, "test" | "production")
        {
            return Err(AppError::bad_request("审核权威路径或环境无效"));
        }
        Ok(Self {
            method,
            path: format!("{REVIEW_PREFIX}/{application}/{environment}"),
            sync_read_permission: None,
        })
    }
    #[cfg(test)]
    fn review_sync_read(
        application: Uuid,
        environment: &str,
        permission: &'static str,
    ) -> Result<Self, AppError> {
        // Only the current review row selects this write permission. It grants
        // one application's sync metadata read and always requires fresh TOTP.
        if !matches!(
            permission,
            "system:oauth:review" | "system:oauth:publish" | "system:oauth:disable"
        ) {
            return Err(AppError::forbidden("审核同步读取权限合同无效"));
        }
        let mut target = Self::review(Method::GET, application, environment)?;
        target.sync_read_permission = Some(permission);
        Ok(target)
    }
    fn invitation(method: Method, invitation: Option<Uuid>) -> Result<Self, AppError> {
        if (method != Method::GET && method != Method::POST)
            || (invitation.is_some() && method != Method::POST)
        {
            return Err(AppError::bad_request("邀请管理路径无效"));
        }
        Ok(Self {
            method,
            path: invitation
                .map(|id| format!("{INVITATIONS_PREFIX}/{id}/revoke"))
                .unwrap_or_else(|| INVITATIONS_PREFIX.to_owned()),
            sync_read_permission: None,
        })
    }
    fn reviewed(&self) -> bool {
        self.route_path().starts_with(&format!("{REVIEW_PREFIX}/"))
            || self
                .route_path()
                .starts_with(&format!("{AUTH_PREFIX}/review-operations/"))
    }
    fn invitations(&self) -> bool {
        self.route_path() == INVITATIONS_PREFIX
            || self
                .route_path()
                .starts_with(&format!("{INVITATIONS_PREFIX}/"))
            || self
                .route_path()
                .starts_with(&format!("{AUTH_PREFIX}/invitation-operations/"))
    }
    fn auth_client_list(query: &ClientListQuery) -> Result<Self, AppError> {
        let mut pairs = Vec::new();
        macro_rules! add {
            ($key:literal,$value:expr) => {
                if let Some(value) = $value {
                    pairs.push(format!("{}={}", $key, value));
                }
            };
        }
        add!("applicationId", query.application_id.as_deref());
        add!("partnerId", query.partner_id.as_deref());
        add!("ownership", query.ownership.as_deref());
        add!("environment", query.environment.as_deref());
        add!("clientType", query.client_type.as_deref());
        add!("subjectMode", query.subject_mode.as_deref());
        add!("status", query.status.as_deref());
        if let Some(limit) = query.limit {
            pairs.push(format!("limit={limit}"));
        }
        if let Some(offset) = query.offset {
            pairs.push(format!("offset={offset}"));
        }
        let suffix = if pairs.is_empty() {
            String::new()
        } else {
            format!("?{}", pairs.join("&"))
        };
        Self::auth_client(Method::GET, &suffix)
    }
    fn route_path(&self) -> &str {
        self.path.split('?').next().unwrap_or(&self.path)
    }
    fn auth_admin(&self) -> bool {
        let path = self.route_path();
        path == AUTH_CLIENTS_PREFIX
            || path.starts_with(&format!("{AUTH_CLIENTS_PREFIX}/"))
            || path == AUTH_CONSENT_GROUPS_PREFIX
            || path.starts_with(&format!("{AUTH_CONSENT_GROUPS_PREFIX}/"))
            || path
                .strip_prefix(&format!("{AUTH_PREFIX}/applications/"))
                .is_some_and(|suffix| {
                    suffix
                        .strip_suffix("/consent-groups")
                        .is_some_and(|application| id(application).is_ok())
                })
            || path.starts_with(&format!("{AUTH_PREFIX}/client-operations/"))
    }
    fn projection(&self) -> bool {
        let path = self.route_path();
        path.strip_prefix(&format!("{AUTH_PREFIX}/applications/"))
            .is_some_and(|suffix| id(suffix).is_ok())
            || path.starts_with(&format!("{AUTH_PREFIX}/operations/"))
    }
    fn operation_lookup(&self, operation: Uuid) -> Self {
        let path = if self.reviewed() {
            format!("{AUTH_PREFIX}/review-operations/{operation}")
        } else if self.invitations() {
            format!("{AUTH_PREFIX}/invitation-operations/{operation}")
        } else if self.projection() {
            format!("{AUTH_PREFIX}/operations/{operation}")
        } else if self.auth_admin() {
            format!("{AUTH_PREFIX}/client-operations/{operation}")
        } else {
            format!("{PREFIX}/key-operations/{operation}")
        };
        Self {
            method: Method::GET,
            path,
            sync_read_permission: None,
        }
    }
    fn mutation(&self) -> bool {
        self.method != Method::GET
    }
    fn requires_recent(&self) -> bool {
        self.mutation() || self.sync_read_permission.is_some()
    }
    fn lookup(&self) -> bool {
        let path = self.route_path();
        self.method == Method::GET
            && (path.starts_with(&format!("{PREFIX}/key-operations/"))
                || path.starts_with(&format!("{AUTH_PREFIX}/operations/"))
                || path.starts_with(&format!("{AUTH_PREFIX}/client-operations/"))
                || path.starts_with(&format!("{AUTH_PREFIX}/review-operations/"))
                || path.starts_with(&format!("{AUTH_PREFIX}/invitation-operations/")))
    }
    fn from_intent(method: &str, path: &str) -> Result<Self, AppError> {
        let method = Method::from_bytes(method.as_bytes()).map_err(|_| AppError::Unavailable)?;
        if let Some(suffix) = path.strip_prefix(&format!("{REVIEW_PREFIX}/")) {
            let (application, environment) = suffix.split_once('/').ok_or(AppError::Unavailable)?;
            let target = Self::review(method, id(application)?, environment)?;
            return if target.mutation() && target.path == path {
                Ok(target)
            } else {
                Err(AppError::Unavailable)
            };
        }
        if path == INVITATIONS_PREFIX {
            let target = Self::invitation(method, None)?;
            return if target.mutation() {
                Ok(target)
            } else {
                Err(AppError::Unavailable)
            };
        }
        if let Some(suffix) = path.strip_prefix(&format!("{INVITATIONS_PREFIX}/")) {
            let raw = suffix
                .strip_suffix("/revoke")
                .ok_or(AppError::Unavailable)?;
            let target = Self::invitation(method, Some(id(raw)?))?;
            return if target.path == path {
                Ok(target)
            } else {
                Err(AppError::Unavailable)
            };
        }
        if path.starts_with(&format!("{PREFIX}/keys/")) {
            let target = Self::new(
                method,
                path.strip_prefix(PREFIX).ok_or(AppError::Unavailable)?,
            )?;
            return if target.mutation() && target.path == path {
                Ok(target)
            } else {
                Err(AppError::Unavailable)
            };
        }
        if let Some(suffix) = path.strip_prefix(&format!("{AUTH_PREFIX}/")) {
            if let Some(client_suffix) = suffix.strip_prefix("clients") {
                let target = Self::auth_client(method.clone(), client_suffix)?;
                return if target.mutation() && target.path == path {
                    Ok(target)
                } else {
                    Err(AppError::Unavailable)
                };
            }
            if suffix == "consent-groups" {
                let target = Self::consent_group_create();
                return if method == Method::POST && target.path == path {
                    Ok(target)
                } else {
                    Err(AppError::Unavailable)
                };
            }
            if let Some(group_suffix) = suffix.strip_prefix("consent-groups/") {
                let (raw_group, action) = group_suffix
                    .split_once('/')
                    .map_or((group_suffix, ""), |(group, action)| (group, action));
                if method != Method::POST || action != "clients" || id(raw_group).is_err() {
                    return Err(AppError::Unavailable);
                }
                let target = Self::consent_group_member(id(raw_group)?);
                return if target.path == path {
                    Ok(target)
                } else {
                    Err(AppError::Unavailable)
                };
            }
            let application = suffix
                .strip_prefix("applications/")
                .ok_or(AppError::Unavailable)?;
            if method != Method::PUT || id(application).is_err() {
                return Err(AppError::Unavailable);
            }
            let target = Self::project(id(application)?);
            return if target.path == path {
                Ok(target)
            } else {
                Err(AppError::Unavailable)
            };
        }
        let target = Self::new(
            method,
            path.strip_prefix(PREFIX).ok_or(AppError::Unavailable)?,
        )
        .map_err(|_| AppError::Unavailable)?;
        if !target.mutation() || target.path != path {
            return Err(AppError::Unavailable);
        }
        Ok(target)
    }
}
fn valid_auth_client_target(method: &Method, path: &str) -> bool {
    if path == AUTH_CLIENTS_PREFIX {
        return method == Method::GET || method == Method::POST;
    }
    let Some(suffix) = path.strip_prefix(&format!("{AUTH_CLIENTS_PREFIX}/")) else {
        return false;
    };
    let (raw_id, action) = suffix
        .split_once('/')
        .map_or((suffix, ""), |(id, action)| (id, action));
    id(raw_id).is_ok()
        && matches!(
            (method.as_str(), action),
            ("GET", "") | ("PATCH", "") | ("POST", "credentials") | ("POST", "credentials/revoke")
        )
}
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys: Vec<_> = object.keys().collect();
            keys.sort();
            let mut result = serde_json::Map::new();
            for key in keys {
                result.insert(key.clone(), canonical(&object[key]));
            }
            Value::Object(result)
        }
        Value::Array(array) => Value::Array(array.iter().map(canonical).collect()),
        _ => value.clone(),
    }
}
fn digest(target: &Target, body: &Option<Value>) -> Result<String, AppError> {
    let bytes = serde_json::to_vec(&canonical(
        &json!({"method":target.method.as_str(),"path":target.path,"body":body}),
    ))
    .map_err(|_| AppError::bad_request("认证管理请求无效"))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyCommand {
    kid: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApplicationProjectionCommand {
    application_id: String,
    expected_version: Option<i64>,
    projection: ApplicationProjection,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApplicationProjection {
    ownership: String,
    partner_id: Option<String>,
    status: String,
    display_name: String,
    purpose: String,
    approved_scopes: Vec<String>,
}
fn prepare(target: &Target, value: Option<Value>) -> Result<Option<Value>, AppError> {
    let bad = || AppError::bad_request("认证管理请求无效");
    if target.reviewed() {
        if target.method == Method::GET {
            return if value.is_none() {
                Ok(None)
            } else {
                Err(bad())
            };
        }
        let body = value.ok_or_else(bad)?;
        let suffix = target
            .path
            .strip_prefix(&format!("{REVIEW_PREFIX}/"))
            .ok_or_else(bad)?;
        let (application, environment) = suffix.split_once('/').ok_or_else(bad)?;
        if !exact(
            &body,
            &[
                "applicationId",
                "environment",
                "requestId",
                "requestVersion",
                "actorUserId",
                "actorSessionId",
                "expectedProjectionVersion",
                "expectedAdmissionVersion",
                "partnerId",
                "manifest",
                "requestedBy",
                "reviewedBy",
                "publishedBy",
                "contactReference",
                "domainReference",
                "status",
            ],
        ) || body["applicationId"].as_str() != Some(application)
            || body["environment"].as_str() != Some(environment)
            || !body["manifest"].is_object()
            || !body["requestVersion"]
                .as_i64()
                .is_some_and(|version| version > 0)
            || !matches!(body["status"].as_str(), Some("APPROVED" | "SUSPENDED"))
            || [
                "applicationId",
                "requestId",
                "actorUserId",
                "actorSessionId",
                "partnerId",
                "requestedBy",
                "reviewedBy",
                "contactReference",
                "domainReference",
            ]
            .iter()
            .any(|key| body[*key].as_str().is_none_or(|value| id(value).is_err()))
            || (!body["publishedBy"].is_null()
                && body["publishedBy"]
                    .as_str()
                    .is_none_or(|value| id(value).is_err()))
            || ["expectedProjectionVersion", "expectedAdmissionVersion"]
                .iter()
                .any(|key| {
                    !body[*key].is_null() && !body[*key].as_i64().is_some_and(|version| version > 0)
                })
        {
            return Err(bad());
        }
        return Ok(Some(body));
    }
    if target.invitations() {
        if target.method == Method::GET {
            return if value.is_none() {
                Ok(None)
            } else {
                Err(bad())
            };
        }
        let body = value.ok_or_else(bad)?;
        let valid = if target.path == INVITATIONS_PREFIX {
            exact(
                &body,
                &["actorUserId", "actorSessionId", "recipientRef", "purpose"],
            ) && body["recipientRef"].as_str().is_some_and(|value| {
                !value.is_empty()
                    && value.len() <= 256
                    && value.trim() == value
                    && !value.chars().any(char::is_control)
            }) && body["purpose"] == "ACCOUNT_ONBOARDING"
        } else {
            exact(&body, &["actorUserId", "actorSessionId", "expectedVersion"])
                && body["expectedVersion"]
                    .as_i64()
                    .is_some_and(|version| version > 0)
        };
        if !valid
            || ["actorUserId", "actorSessionId"]
                .iter()
                .any(|key| body[*key].as_str().is_none_or(|value| id(value).is_err()))
        {
            return Err(bad());
        }
        return Ok(Some(body));
    }
    if target.projection() {
        if target.method == Method::GET {
            return if value.is_none() {
                Ok(None)
            } else {
                Err(AppError::bad_request("认证应用投影查询无请求体"))
            };
        }
        let raw = value.clone().ok_or_else(bad)?;
        let command: ApplicationProjectionCommand =
            serde_json::from_value(raw).map_err(|_| AppError::bad_request("认证应用投影无效"))?;
        let application = id(&command.application_id)?;
        let members_valid = command
            .projection
            .partner_id
            .as_deref()
            .map(id)
            .transpose()?;
        if target.path != Target::project(application).path
            || command.expected_version.is_some_and(|v| v < 1)
            || !matches!(
                command.projection.ownership.as_str(),
                "FIRST_PARTY" | "THIRD_PARTY"
            )
            || ((command.projection.ownership == "THIRD_PARTY") != members_valid.is_some())
            || !matches!(
                command.projection.status.as_str(),
                "DRAFT" | "PENDING_REVIEW" | "APPROVED" | "SUSPENDED" | "RETIRED"
            )
            || command.projection.display_name.trim().is_empty()
            || command.projection.display_name.chars().count() > 200
            || command.projection.purpose.trim().is_empty()
            || command.projection.purpose.chars().count() > 2000
            || command.projection.approved_scopes.len() > 64
            || command.projection.approved_scopes.iter().any(|scope| {
                scope.is_empty()
                    || scope.len() > 128
                    || !scope.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b"._:-".contains(&b)
                    })
            })
        {
            return Err(AppError::bad_request("认证应用投影无效"));
        }
        return Ok(Some(value.ok_or_else(bad)?));
    }
    if target.auth_admin() {
        let path = target.route_path();
        if target.method == Method::GET {
            return if value.is_none() {
                Ok(None)
            } else {
                Err(AppError::bad_request("统一客户端查询不接受请求体"))
            };
        }
        let mut value = value.clone().ok_or_else(bad)?;
        if path == AUTH_CONSENT_GROUPS_PREFIX && target.method == Method::POST {
            let environment = value["environment"].as_str().unwrap_or_default();
            let environment_is_valid = !environment.is_empty()
                && environment.len() <= 32
                && environment
                    .as_bytes()
                    .first()
                    .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
                && environment.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'_' | b'-')
                });
            if !exact(
                &value,
                &[
                    "applicationId",
                    "environment",
                    "displayName",
                    "purpose",
                    "contextMode",
                ],
            ) || value["applicationId"]
                .as_str()
                .is_none_or(|raw| id(raw).is_err())
                || !environment_is_valid
                || value["displayName"]
                    .as_str()
                    .is_none_or(|name| name.trim().is_empty() || name.chars().count() > 120)
                || value["purpose"]
                    .as_str()
                    .is_none_or(|purpose| purpose.is_empty() || purpose.chars().count() > 2000)
                || !matches!(
                    value["contextMode"].as_str(),
                    Some("ACCOUNT_ONLY" | "PLATFORM_CONTEXT")
                )
            {
                return Err(AppError::bad_request("同意组登记字段无效"));
            }
            return Ok(Some(value));
        }
        if let Some(raw_group) = path
            .strip_prefix(&format!("{AUTH_CONSENT_GROUPS_PREFIX}/"))
            .and_then(|suffix| suffix.strip_suffix("/clients"))
        {
            if id(raw_group).is_err()
                || !exact(&value, &["clientId", "expectedGroupVersion"])
                || value["clientId"]
                    .as_str()
                    .is_none_or(|raw| id(raw).is_err())
                || !value["expectedGroupVersion"]
                    .as_i64()
                    .is_some_and(|version| version >= 1)
            {
                return Err(AppError::bad_request("同意组成员登记字段无效"));
            }
            return Ok(Some(value));
        }
        let credential_action =
            path.ends_with("/credentials") || path.ends_with("/credentials/revoke");
        if credential_action {
            if !exact(&value, &["expectedVersion"])
                || !value["expectedVersion"].as_i64().is_some_and(|v| v >= 1)
            {
                return Err(AppError::bad_request("客户端凭据版本无效"));
            }
        } else {
            let object = value.as_object_mut().ok_or_else(bad)?;
            object
                .entry("logoutMode")
                .or_insert(json!("SHORT_SESSION_ONLINE_CHECK"));
            object.entry("backchannelLogoutUri").or_insert(Value::Null);
            object.entry("backchannelTargetId").or_insert(Value::Null);
            object.entry("autoAuthorize").or_insert(json!(false));
            for field in ["accessTokenTtlSeconds","refreshTokenTtlSeconds","clientTokenTtlSeconds"] {
                object.entry(field).or_insert(Value::Null);
            }
            let keys = [
                "expectedVersion",
                "applicationId",
                "environment",
                "displayName",
                "clientType",
                "runtimeKind",
                "subjectMode",
                "contextMode",
                "allowedMemberTypes",
                "grantTypes",
                "tokenEndpointAuthMethod",
                "redirectUris",
                "postLogoutRedirectUris",
                "logoutMode",
                "backchannelLogoutUri",
                "backchannelTargetId",
                "status",
                "consentGroupId",
                "permissions",
                "autoAuthorize",
                "accessTokenTtlSeconds",
                "refreshTokenTtlSeconds",
                "clientTokenTtlSeconds",
            ];
            let create = path == AUTH_CLIENTS_PREFIX;
            if !exact(&value, &keys)
                || (create && !value["expectedVersion"].is_null())
                || (!create
                    && !value["expectedVersion"]
                        .as_i64()
                        .is_some_and(|version| version >= 1))
                || value["applicationId"]
                    .as_str()
                    .is_none_or(|raw| id(raw).is_err())
                || !value["permissions"].is_array()
                || !client_logout_fields_valid(&value)
                || !client_token_policy_valid(&value, false)
                || (create && value["logoutMode"] != "SHORT_SESSION_ONLINE_CHECK")
            {
                return Err(AppError::bad_request("统一客户端请求字段无效"));
            }
        }
        return Ok(Some(value));
    }
    match (target.method.as_str(), target.path.strip_prefix(PREFIX)) {
        ("POST", Some("/keys/publish" | "/keys/activate")) => {
            let command: KeyCommand =
                serde_json::from_value(value.clone().ok_or_else(bad)?).map_err(|_| bad())?;
            kid(&command.kid)?;
        }
        _ if value.is_some() => return Err(bad()),
        _ => {}
    }
    if serde_json::to_vec(&value).map_err(|_| bad())?.len() > LIMIT {
        return Err(AppError::bad_request("认证管理请求过大"));
    }
    Ok(value)
}
// These private types intentionally have no Debug: body/response can contain a
// just-issued RP secret, which must never enter logging, intents or audits.
struct Command {
    target: Target,
    body: Option<Value>,
    operation: Uuid,
    digest: String,
}
fn operation_started_at(operation: Uuid) -> Result<chrono::DateTime<chrono::Utc>, AppError> {
    let milliseconds = operation.as_bytes()[..6]
        .iter()
        .fold(0_i64, |value, byte| (value << 8) | i64::from(*byte));
    chrono::DateTime::from_timestamp_millis(milliseconds)
        .ok_or_else(|| AppError::bad_request("操作标识时间无效"))
}
fn browser_operation(headers: &HeaderMap, target: &Target) -> Result<Uuid, AppError> {
    let mut values = headers.get_all("x-identity-operation").iter();
    match (values.next(), values.next()) {
        (None, None) => Ok(uuid7::new_uuid()),
        (Some(raw), None) if target.mutation() => id(raw
            .to_str()
            .map_err(|_| AppError::bad_request("操作标识无效"))?),
        _ => Err(AppError::bad_request("操作标识只能在管理写入中出现一次")),
    }
}
async fn lock_operation(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    operation: Uuid,
) -> Result<(), AppError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,6351749))")
        .bind(operation.to_string())
        .execute(&mut **tx)
        .await
        .map_err(|_| AppError::Unavailable)?;
    Ok(())
}
struct Reply {
    status: u16,
    content_type: Option<String>,
    bytes: Vec<u8>,
}
type AdminFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>;
trait Boundary: Send + Sync {
    fn accept<'a>(&'a self, command: &'a Command) -> AdminFuture<'a, ()>;
    fn acknowledge<'a>(
        &'a self,
        operation: Uuid,
        projection: bool,
        auth_admin: bool,
    ) -> AdminFuture<'a, ()>;
    fn reconcile<'a>(&'a self, _command: &'a Command, _result: &'a Value) -> AdminFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}
trait Transport: Send + Sync {
    fn send<'a>(&'a self, command: &'a Command) -> AdminFuture<'a, Reply>;
}
trait Metadata: Send + Sync {
    fn client<'a>(&'a self, client: Uuid) -> AdminFuture<'a, Value>;
}
struct PlatformBoundary<'a> {
    state: &'a AppState,
    user: &'a CurrentUser,
    headers: &'a HeaderMap,
    metadata: Option<&'a dyn Metadata>,
}
struct Authorization {
    permission: &'static str,
    application: Option<Uuid>,
}

fn proposed_permission(command: &Command) -> &'static str {
    if let Some(permission) = command.target.sync_read_permission {
        return permission;
    }
    crate::auth_review::permission_for(
        &command.target.method,
        &command.target.path,
        command.body.as_ref(),
    )
}

async fn preauthorize(
    state: &AppState,
    user: &CurrentUser,
    command: &Command,
) -> Result<(), AppError> {
    let permission = proposed_permission(command);
    let candidates =
        if command.target.method == Method::PATCH && permission == "system:oauth:disable" {
            vec![permission, "system:oauth:scopes"]
        } else {
            vec![permission]
        };
    for permission in candidates {
        match crate::auth_review::management_tx(
            state,
            user,
            permission,
            command.target.requires_recent(),
        )
        .await
        {
            Ok(tx) => {
                tx.commit().await.map_err(|_| AppError::Unavailable)?;
                return Ok(());
            }
            Err(AppError::Forbidden(_) | AppError::OperationDenied) => {}
            Err(error) => return Err(error),
        }
    }
    Err(AppError::forbidden(
        "当前管理角色或最近5分钟TOTP不足以执行此认证操作",
    ))
}

fn permission_set(value: &Value) -> Option<BTreeMap<String, Vec<String>>> {
    let mut permissions = BTreeMap::new();
    for permission in value.as_array()? {
        let resource = permission["resourceKey"].as_str()?.to_owned();
        let mut scopes = permission["scopes"]
            .as_array()?
            .iter()
            .map(|scope| scope.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()?;
        scopes.sort();
        if scopes.windows(2).any(|pair| pair[0] == pair[1])
            || permissions.insert(resource, scopes).is_some()
        {
            return None;
        }
    }
    Some(permissions)
}

fn stop_only(body: &Value, current: &Value) -> bool {
    matches!(body["status"].as_str(), Some("DISABLED" | "RETIRED"))
        && ["logoutMode", "backchannelLogoutUri", "backchannelTargetId"]
            .iter()
            .all(|key| body.get(*key).is_some_and(|value| value == &current[*key]))
        && body.as_object().is_some_and(|body| {
            body.iter().all(|(key, value)| match key.as_str() {
                "expectedVersion" | "status" => true,
                "permissions" => permission_set(value)
                    .zip(permission_set(&current["permissions"]))
                    .is_some_and(|(left, right)| left == right),
                _ => value == &current[key],
            })
        })
}

async fn client_owner(
    state: &AppState,
    command: &Command,
    client: Uuid,
) -> Result<Value, AppError> {
    let target = Target::auth_client(Method::GET, &format!("/{client}"))?;
    let transport = HttpTransport::configured(state, &target)?;
    let request = Command {
        target: target.clone(),
        body: None,
        operation: command.operation,
        digest: String::new(),
    };
    decode(transport.send(&request).await?, &target)
}

async fn current_client_owner(
    state: &AppState,
    command: &Command,
    client: Uuid,
    metadata: Option<&dyn Metadata>,
) -> Result<Value, AppError> {
    if let Some(metadata) = metadata {
        let value = metadata.client(client).await?;
        if !auth_client_projection(&value)
            || value["id"].as_str() != Some(client.to_string().as_str())
        {
            return Err(AppError::Unavailable);
        }
        return Ok(value);
    }
    client_owner(state, command, client).await
}

async fn authorize_command(
    state: &AppState,
    user: &CurrentUser,
    command: &Command,
    metadata: Option<&dyn Metadata>,
) -> Result<Authorization, AppError> {
    preauthorize(state, user, command).await?;
    let path = command.target.route_path();
    let mut permission = proposed_permission(command);
    let mut application = None;
    if command.target.reviewed() {
        let raw = path
            .strip_prefix(&format!("{REVIEW_PREFIX}/"))
            .and_then(|suffix| suffix.split_once('/'))
            .map(|(application, _)| application)
            .ok_or(AppError::Unavailable)?;
        application = Some(id(raw)?);
    } else if command.target.projection() {
        application = path
            .strip_prefix(&format!("{AUTH_PREFIX}/applications/"))
            .map(id)
            .transpose()?;
    } else if command.target.auth_admin() {
        if let Some(suffix) = path.strip_prefix(&format!("{AUTH_CLIENTS_PREFIX}/")) {
            let client = id(suffix.split('/').next().ok_or(AppError::Unavailable)?)?;
            let current = current_client_owner(state, command, client, metadata).await?;
            let owner = current["applicationId"]
                .as_str()
                .ok_or(AppError::Unavailable)?;
            application = Some(id(owner)?);
            if command
                .body
                .as_ref()
                .and_then(|body| body.get("applicationId"))
                .is_some_and(|value| value.as_str() != Some(owner))
            {
                return Err(AppError::forbidden(
                    "客户端登记归属与当前身份权威记录不匹配",
                ));
            }
            if command.target.method == Method::PATCH {
                permission = if command
                    .body
                    .as_ref()
                    .is_some_and(|body| stop_only(body, &current))
                {
                    "system:oauth:disable"
                } else {
                    "system:oauth:scopes"
                };
            }
        } else if let Some(client) = command
            .body
            .as_ref()
            .and_then(|body| body["clientId"].as_str())
        {
            let current = current_client_owner(state, command, id(client)?, metadata).await?;
            application = Some(id(current["applicationId"]
                .as_str()
                .ok_or(AppError::Unavailable)?)?);
        } else if let Some(raw) = command
            .body
            .as_ref()
            .and_then(|body| body["applicationId"].as_str())
        {
            application = Some(id(raw)?);
        } else if let Some(raw) = path
            .strip_prefix(&format!("{AUTH_PREFIX}/applications/"))
            .and_then(|suffix| suffix.strip_suffix("/consent-groups"))
        {
            application = Some(id(raw)?);
        } else if path == AUTH_CLIENTS_PREFIX {
            let url =
                reqwest::Url::parse(&format!("https://private.invalid{}", command.target.path))
                    .map_err(|_| AppError::Unavailable)?;
            application = url
                .query_pairs()
                .find(|(key, _)| key == "applicationId")
                .map(|(_, raw)| id(&raw))
                .transpose()?;
        }
    }
    let mut tx = crate::auth_review::management_tx(
        state,
        user,
        permission,
        command.target.requires_recent(),
    )
    .await?;
    if let Some(application) = application {
        crate::auth_review::require_application(&mut tx, user, application).await?;
    } else if command.target.auth_admin()
        && !crate::auth_review::global_operator(&mut tx, user).await?
    {
        return Err(AppError::forbidden(
            "请明确指定已获委派的应用；跨应用客户端列表仅对全局审核人员开放",
        ));
    }
    tx.commit().await.map_err(|_| AppError::Unavailable)?;
    Ok(Authorization {
        permission,
        application,
    })
}
impl Boundary for PlatformBoundary<'_> {
    fn accept<'a>(&'a self, command: &'a Command) -> AdminFuture<'a, ()> {
        Box::pin(async move {
            if !self.state.config.identity_remote_enabled {
                return Err(AppError::not_found("独立认证管理未启用"));
            }
            let authorization =
                authorize_command(self.state, self.user, command, self.metadata).await?;
            let mut tx = crate::auth_review::management_tx(
                self.state,
                self.user,
                authorization.permission,
                command.target.requires_recent(),
            )
            .await?;
            if let Some(application) = authorization.application {
                crate::auth_review::require_application(&mut tx, self.user, application).await?;
            }
            if command.target.mutation() {
                let service = service_id(self.state, &command.target)?;
                lock_operation(&mut tx, command.operation).await?;
                // Bound admission by the browser's UUID timestamp. Once the
                // window closes, a locked absent-intent lookup can prove that
                // even a delayed request can never start this operation later.
                let started_at = operation_started_at(command.operation)?;
                let written=sqlx::query("INSERT INTO platform_auth_command_intents(id,actor_user_id,tenant_id,organization_id,session_id,service_client_id,request_digest,method,path,accepted_at,management_permission,application_ref) SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,clock_timestamp(),$11,$12 WHERE clock_timestamp() BETWEEN $10::timestamptz-interval '30 seconds' AND $10::timestamptz+interval '60 seconds' ON CONFLICT(id) DO NOTHING")
                .bind(command.operation).bind(self.user.user_id).bind(self.user.tenant_id).bind(self.user.org_id).bind(self.user.session_id).bind(service).bind(&command.digest)
                .bind(command.target.method.as_str()).bind(&command.target.path).bind(started_at).bind(authorization.permission).bind(authorization.application).execute(&mut *tx).await.map_err(|_|AppError::Unavailable)?.rows_affected();
                if written != 1 {
                    let same: Option<bool> = sqlx::query_scalar("SELECT actor_user_id=$2 AND tenant_id=$3 AND organization_id=$4 AND session_id=$5 AND service_client_id=$6 AND request_digest=$7 AND method=$8 AND path=$9 AND management_permission=$10 AND application_ref IS NOT DISTINCT FROM $11 FROM platform_auth_command_intents WHERE id=$1")
                        .bind(command.operation).bind(self.user.user_id).bind(self.user.tenant_id).bind(self.user.org_id).bind(self.user.session_id).bind(service).bind(&command.digest).bind(command.target.method.as_str()).bind(&command.target.path).bind(authorization.permission).bind(authorization.application)
                        .fetch_optional(&mut *tx).await.map_err(|_|AppError::Unavailable)?;
                    match same {
                        Some(true) => {
                            tx.commit().await.map_err(|_| AppError::Unavailable)?;
                            return Ok(());
                        }
                        Some(false) => {
                            return Err(AppError::conflict("原操作已绑定另一请求或会话"))
                        }
                        None => return Err(AppError::Unavailable),
                    }
                }
                let action = if command.target.reviewed() {
                    "IDENTITY_AUTH_REVIEW_COMMAND_ACCEPT"
                } else if command.target.invitations() {
                    "IDENTITY_AUTH_INVITATION_COMMAND_ACCEPT"
                } else if command.target.projection() {
                    "IDENTITY_APPLICATION_PROJECTION_ACCEPT"
                } else if command.target.auth_admin() {
                    "IDENTITY_AUTH_CLIENT_COMMAND_ACCEPT"
                } else {
                    "IDENTITY_KEY_COMMAND_ACCEPT"
                };
                admin_audit(&mut tx,self.user,self.headers,action,&json!({"operationId":command.operation,"resource":command.target.path,"requestDigest":command.digest}).to_string()).await?;
            }
            // No transaction or platform row lock crosses the back-channel.
            tx.commit().await.map_err(|_| AppError::Unavailable)?;
            Ok(())
        })
    }
    fn acknowledge<'a>(
        &'a self,
        operation: Uuid,
        projection: bool,
        auth_admin: bool,
    ) -> AdminFuture<'a, ()> {
        Box::pin(async move {
            let mut tx = self
                .state
                .pool
                .begin()
                .await
                .map_err(|_| AppError::Unavailable)?;
            admin_audit(
                &mut tx,
                self.user,
                self.headers,
                if projection {
                    "IDENTITY_APPLICATION_PROJECTION_ACK"
                } else if auth_admin {
                    "IDENTITY_AUTH_CLIENT_COMMAND_ACK"
                } else {
                    "IDENTITY_KEY_COMMAND_ACK"
                },
                &json!({"operationId":operation}).to_string(),
            )
            .await?;
            tx.commit().await.map_err(|_| AppError::Unavailable)?;
            Ok(())
        })
    }
    fn reconcile<'a>(&'a self, command: &'a Command, result: &'a Value) -> AdminFuture<'a, ()> {
        Box::pin(async move {
            if !command.target.reviewed() {
                return Ok(());
            }
            let mut tx = self
                .state
                .pool
                .begin()
                .await
                .map_err(|_| AppError::Unavailable)?;
            crate::auth_review::complete_remote(&mut tx, command.operation, result).await?;
            admin_audit(
                &mut tx,
                self.user,
                self.headers,
                "IDENTITY_AUTH_REVIEW_COMMAND_ACK",
                &json!({"operationId":command.operation}).to_string(),
            )
            .await?;
            tx.commit().await.map_err(|_| AppError::Unavailable)
        })
    }
}
struct HttpTransport {
    client: reqwest::Client,
    origin: String,
    authorization: String,
}
fn registration<'a>(state: &'a AppState, target: &Target) -> (Option<&'a str>, Option<&'a str>) {
    if target.reviewed() {
        (
            state.config.identity_review_service_client_id.as_deref(),
            state
                .config
                .identity_review_service_client_secret
                .as_deref(),
        )
    } else if target.invitations() {
        (
            state
                .config
                .identity_invitation_service_client_id
                .as_deref(),
            state
                .config
                .identity_invitation_service_client_secret
                .as_deref(),
        )
    } else if target.projection() {
        (
            state
                .config
                .identity_projection_service_client_id
                .as_deref(),
            state
                .config
                .identity_projection_service_client_secret
                .as_deref(),
        )
    } else if target.auth_admin() {
        (
            state
                .config
                .identity_auth_admin_service_client_id
                .as_deref(),
            state
                .config
                .identity_auth_admin_service_client_secret
                .as_deref(),
        )
    } else {
        (
            state.config.identity_key_admin_service_client_id.as_deref(),
            state
                .config
                .identity_key_admin_service_client_secret
                .as_deref(),
        )
    }
}
impl HttpTransport {
    fn configured(state: &AppState, target: &Target) -> Result<Self, AppError> {
        let origin = state
            .config
            .identity_service_url
            .as_deref()
            .ok_or(AppError::Unavailable)?;
        let url = reqwest::Url::parse(origin).map_err(|_| AppError::Unavailable)?;
        if url.scheme() != "https"
            || url.origin().ascii_serialization() != origin
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(AppError::Unavailable);
        }
        let (service, secret) = registration(state, target);
        let service = service.ok_or(AppError::Unavailable)?;
        id(service).map_err(|_| AppError::Unavailable)?;
        let secret = secret.ok_or(AppError::Unavailable)?;
        if !opaque(secret) {
            return Err(AppError::Unavailable);
        }
        Ok(Self {
            client: state
                .config
                .identity_http_client
                .clone()
                .ok_or(AppError::Unavailable)?,
            origin: origin.into(),
            authorization: format!("Basic {}", STANDARD.encode(format!("{service}:{secret}"))),
        })
    }
    fn request(&self, command: &Command) -> Result<reqwest::Request, AppError> {
        let mut request = self
            .client
            .request(
                command.target.method.clone(),
                format!("{}{}", self.origin, command.target.path),
            )
            .header("authorization", &self.authorization)
            .header("accept", "application/json");
        if command.target.mutation() {
            request = request
                .header("x-identity-operation", command.operation.to_string())
                .header("x-identity-operation-digest", &command.digest);
        }
        if command.target.lookup() {
            request = request.header("x-identity-operation-digest", &command.digest);
        }
        if let Some(body) = &command.body {
            request = request.header("content-type", "application/json").body(
                serde_json::to_vec(body).map_err(|_| AppError::bad_request("认证管理请求无效"))?,
            );
        }
        request.build().map_err(|_| AppError::Unavailable)
    }
}
fn service_id(state: &AppState, target: &Target) -> Result<Uuid, AppError> {
    let raw = registration(state, target).0.ok_or(AppError::Unavailable)?;
    id(raw).map_err(|_| AppError::Unavailable)
}
impl Transport for HttpTransport {
    fn send<'a>(&'a self, command: &'a Command) -> AdminFuture<'a, Reply> {
        Box::pin(async move {
            let mut response = self
                .client
                .execute(self.request(command)?)
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
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| AppError::Unavailable)? {
                if bytes.len() + chunk.len() > LIMIT {
                    return Err(AppError::Unavailable);
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(Reply {
                status,
                content_type,
                bytes,
            })
        })
    }
}
fn exact(value: &Value, keys: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|o| o.len() == keys.len() && keys.iter().all(|k| o.contains_key(*k)))
}
fn client_token_policy_valid(value: &Value, response: bool) -> bool {
    let ttl = |name: &str, refresh: bool| value[name].is_null() || value[name].as_i64().is_some_and(|seconds|
        if refresh { (300..=604800).contains(&seconds) } else { (60..=3600).contains(&seconds) });
    let basic = value["autoAuthorize"].is_boolean() && ttl("accessTokenTtlSeconds",false)
        && ttl("refreshTokenTtlSeconds",true) && ttl("clientTokenTtlSeconds",false);
    if !response { return basic; }
    let defaults = &value["tokenDefaults"];
    basic && value["refreshTokenStrategy"] == "ROTATE"
        && exact(defaults, &["accessTokenTtlSeconds","refreshTokenTtlSeconds","offlineRefreshTokenTtlSeconds","clientTokenTtlSeconds"])
        && ["accessTokenTtlSeconds","clientTokenTtlSeconds"].iter().all(|key| defaults[*key].as_i64().is_some_and(|v| (60..=3600).contains(&v)))
        && ["refreshTokenTtlSeconds","offlineRefreshTokenTtlSeconds"].iter().all(|key| defaults[*key].as_i64().is_some_and(|v| (300..=604800).contains(&v)))
}

fn client_logout_fields_valid(value: &Value) -> bool {
    match value["logoutMode"].as_str() {
        Some("SHORT_SESSION_ONLINE_CHECK") => {
            value["backchannelLogoutUri"].is_null() && value["backchannelTargetId"].is_null()
        }
        Some("BACK_CHANNEL") => {
            value["subjectMode"] == "USER"
                && value["clientType"] == "CONFIDENTIAL"
                && value["backchannelTargetId"]
                    .as_str()
                    .is_some_and(|raw| id(raw).is_ok())
                && value["backchannelLogoutUri"].as_str().is_some_and(|raw| {
                    reqwest::Url::parse(raw).is_ok_and(|url| {
                        raw.len() <= 2048
                            && url.scheme() == "https"
                            && url.host_str().is_some()
                            && url.username().is_empty()
                            && url.password().is_none()
                            && url.query().is_none()
                            && url.fragment().is_none()
                            && url.as_str() == raw
                            && !raw.contains('*')
                    })
                })
        }
        _ => false,
    }
}
fn auth_client_projection(value: &Value) -> bool {
    exact(
        value,
        &[
            "id",
            "clientId",
            "displayName",
            "applicationId",
            "applicationName",
            "ownership",
            "partnerId",
            "environment",
            "clientType",
            "runtimeKind",
            "subjectMode",
            "contextMode",
            "allowedMemberTypes",
            "grantTypes",
            "tokenEndpointAuthMethod",
            "redirectUris",
            "postLogoutRedirectUris",
            "logoutMode",
            "backchannelLogoutUri",
            "backchannelTargetId",
            "status",
            "version",
            "policyVersion",
            "createdAt",
            "updatedAt",
            "consentGroupId",
            "permissions",
            "autoAuthorize",
            "accessTokenTtlSeconds",
            "refreshTokenTtlSeconds",
            "clientTokenTtlSeconds",
            "refreshTokenStrategy",
            "tokenDefaults",
        ],
    ) && client_logout_fields_valid(value) && client_token_policy_valid(value, true)
        && value["id"].as_str().is_some_and(|v| id(v).is_ok())
        && value["clientId"]
            .as_str()
            .is_some_and(public_client_identifier)
        && value["applicationId"]
            .as_str()
            .is_some_and(|v| id(v).is_ok())
        && value["applicationName"]
            .as_str()
            .is_some_and(|v| !v.trim().is_empty() && v.chars().count() <= 200)
        && value["ownership"]
            .as_str()
            .is_some_and(|v| matches!(v, "FIRST_PARTY" | "THIRD_PARTY"))
        && (value["partnerId"].is_null()
            || value["partnerId"].as_str().is_some_and(|v| id(v).is_ok()))
        && value["displayName"]
            .as_str()
            .is_some_and(|v| !v.trim().is_empty() && v.chars().count() <= 120)
        && value["environment"].as_str().is_some_and(|v| {
            !v.is_empty()
                && v.len() <= 32
                && v.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        })
        && value["clientType"]
            .as_str()
            .is_some_and(|v| matches!(v, "PUBLIC" | "CONFIDENTIAL"))
        && value["runtimeKind"]
            .as_str()
            .is_some_and(|v| matches!(v, "WEB_BFF" | "SPA" | "NATIVE" | "SERVICE"))
        && value["subjectMode"]
            .as_str()
            .is_some_and(|v| matches!(v, "USER" | "MACHINE"))
        && value["contextMode"]
            .as_str()
            .is_some_and(|v| matches!(v, "ACCOUNT_ONLY" | "PLATFORM_CONTEXT" | "NONE"))
        && value["allowedMemberTypes"]
            .as_array()
            .is_some_and(|values| {
                values.len() <= 2 && values.iter().all(|v| v == "EMPLOYEE" || v == "CUSTOMER")
            })
        && value["grantTypes"].as_array().is_some_and(|values| {
            !values.is_empty()
                && values.len() <= 2
                && values.iter().all(|v| {
                    v == "authorization_code" || v == "refresh_token" || v == "client_credentials"
                })
        })
        && value["tokenEndpointAuthMethod"]
            .as_str()
            .is_some_and(|v| matches!(v, "client_secret_basic" | "none"))
        && ["redirectUris", "postLogoutRedirectUris"]
            .iter()
            .all(|key| {
                value[*key]
                    .as_array()
                    .is_some_and(|items| items.len() <= 16 && items.iter().all(Value::is_string))
            })
        && value["status"]
            .as_str()
            .is_some_and(|v| matches!(v, "ENABLED" | "DISABLED" | "RETIRED"))
        && value["version"].as_i64().is_some_and(|v| v >= 1)
        && value["policyVersion"].as_i64().is_some_and(|v| v >= 1)
        && value["createdAt"].is_string()
        && value["updatedAt"].is_string()
        && (value["consentGroupId"].is_null()
            || value["consentGroupId"]
                .as_str()
                .is_some_and(|v| id(v).is_ok()))
        && value["permissions"].as_array().is_some_and(|permissions| {
            permissions.len() <= 64
                && permissions.iter().all(|permission| {
                    exact(permission, &["resourceKey", "audience", "scopes"])
                        && permission["resourceKey"]
                            .as_str()
                            .is_some_and(|v| !v.is_empty() && v.len() <= 64)
                        && permission["audience"]
                            .as_str()
                            .is_some_and(|v| !v.is_empty() && v.len() <= 500)
                        && permission["scopes"].as_array().is_some_and(|scopes| {
                            !scopes.is_empty()
                                && scopes.len() <= 64
                                && scopes.iter().all(Value::is_string)
                        })
                })
        })
}
fn consent_group_projection(value: &Value, include_clients: bool) -> bool {
    let common = [
        "id",
        "applicationId",
        "applicationName",
        "environment",
        "displayName",
        "purpose",
        "subjectMode",
        "contextMode",
        "status",
        "version",
    ];
    let keys = if include_clients {
        [&common[..], &["clients"][..]].concat()
    } else {
        common.to_vec()
    };
    exact(value, &keys)
        && value["id"].as_str().is_some_and(|raw| id(raw).is_ok())
        && value["applicationId"]
            .as_str()
            .is_some_and(|raw| id(raw).is_ok())
        && value["applicationName"]
            .as_str()
            .is_some_and(|name| !name.trim().is_empty() && name.chars().count() <= 200)
        && value["environment"].as_str().is_some_and(|environment| {
            !environment.is_empty()
                && environment.len() <= 32
                && environment
                    .as_bytes()
                    .first()
                    .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
                && environment.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'_' | b'-')
                })
        })
        && value["displayName"]
            .as_str()
            .is_some_and(|name| !name.trim().is_empty() && name.chars().count() <= 120)
        && value["purpose"]
            .as_str()
            .is_some_and(|purpose| !purpose.is_empty() && purpose.chars().count() <= 2000)
        && value["subjectMode"] == "USER"
        && value["contextMode"]
            .as_str()
            .is_some_and(|mode| matches!(mode, "ACCOUNT_ONLY" | "PLATFORM_CONTEXT"))
        && value["status"]
            .as_str()
            .is_some_and(|status| matches!(status, "ACTIVE" | "SUSPENDED" | "RETIRED"))
        && value["version"]
            .as_i64()
            .is_some_and(|version| version >= 1)
        && (!include_clients
            || value["clients"].as_array().is_some_and(|clients| {
                clients.len() <= 500
                    && clients.iter().all(|client| {
                        exact(client, &["id", "displayName"])
                            && client["id"].as_str().is_some_and(|raw| id(raw).is_ok())
                            && client["displayName"].as_str().is_some_and(|name| {
                                !name.trim().is_empty() && name.chars().count() <= 120
                            })
                    })
            }))
}

fn safe_result(target: &Target, value: &Value) -> bool {
    if target.reviewed() {
        let Some((application, environment)) = target
            .route_path()
            .strip_prefix(&format!("{REVIEW_PREFIX}/"))
            .and_then(|suffix| suffix.split_once('/'))
        else {
            return false;
        };
        if id(application).is_err()
            || !matches!(environment, "test" | "production")
            || value["applicationId"].as_str() != Some(application)
            || value["environment"].as_str() != Some(environment)
        {
            return false;
        }
        let common = value["projectionVersion"]
            .as_i64()
            .is_some_and(|version| version > 0)
            && value["admissionVersion"]
                .as_i64()
                .is_some_and(|version| version > 0)
            && matches!(value["status"].as_str(), Some("APPROVED" | "SUSPENDED"));
        if target.method == Method::GET {
            return exact(
                value,
                &[
                    "applicationId",
                    "environment",
                    "projectionVersion",
                    "admissionVersion",
                    "status",
                ],
            ) && (common
                || (value["status"] == "UNREGISTERED"
                    && value["admissionVersion"].is_null()
                    && (value["projectionVersion"].is_null()
                        || value["projectionVersion"]
                            .as_i64()
                            .is_some_and(|version| version > 0))));
        }
        return common
            && exact(
                value,
                &[
                    "applicationId",
                    "environment",
                    "projectionVersion",
                    "admissionVersion",
                    "status",
                    "requestId",
                    "requestVersion",
                ],
            )
            && value["requestId"]
                .as_str()
                .is_some_and(|raw| id(raw).is_ok())
            && value["requestVersion"]
                .as_i64()
                .is_some_and(|version| version > 0);
    }
    if target.invitations() {
        if target.method == Method::GET && target.path == INVITATIONS_PREFIX {
            return exact(value, &["records", "limit"])
                && value["limit"] == 50
                && value["records"].as_array().is_some_and(|records| {
                    records.len() <= 50 && records.iter().all(invitation_metadata)
                });
        }
        return invitation_metadata(value)
            && (target.path == INVITATIONS_PREFIX
                || target
                    .path
                    .strip_prefix(&format!("{INVITATIONS_PREFIX}/"))
                    .and_then(|raw| raw.strip_suffix("/revoke"))
                    .is_some_and(|raw| value["id"].as_str() == Some(raw)));
    }
    if target.projection() {
        let Some(application) = target
            .path
            .strip_prefix(&format!("{AUTH_PREFIX}/applications/"))
        else {
            return false;
        };
        if id(application).is_err() {
            return false;
        }
        if target.method == Method::GET {
            return exact(
                value,
                &[
                    "applicationId",
                    "partnerId",
                    "ownership",
                    "status",
                    "displayName",
                    "purpose",
                    "approvedScopes",
                    "projectionVersion",
                ],
            ) && value["applicationId"].as_str() == Some(application)
                && value["projectionVersion"].as_i64().is_some_and(|v| v >= 1)
                && value["ownership"]
                    .as_str()
                    .is_some_and(|v| matches!(v, "FIRST_PARTY" | "THIRD_PARTY"))
                && value["status"].as_str().is_some_and(|v| {
                    matches!(
                        v,
                        "DRAFT" | "PENDING_REVIEW" | "APPROVED" | "SUSPENDED" | "RETIRED"
                    )
                })
                && value["approvedScopes"].as_array().is_some_and(|values| {
                    values.len() <= 64 && values.iter().all(Value::is_string)
                })
                && value["displayName"]
                    .as_str()
                    .is_some_and(|v| !v.is_empty() && v.len() <= 800)
                && value["purpose"]
                    .as_str()
                    .is_some_and(|v| !v.is_empty() && v.len() <= 8000)
                && (value["partnerId"].is_null()
                    || value["partnerId"].as_str().is_some_and(|v| id(v).is_ok()));
        }
        return exact(value, &["applicationId", "projectionVersion", "status"])
            && value["applicationId"].as_str() == Some(application)
            && value["projectionVersion"].as_i64().is_some_and(|v| v >= 1)
            && value["status"].as_str().is_some_and(|v| {
                matches!(
                    v,
                    "DRAFT" | "PENDING_REVIEW" | "APPROVED" | "SUSPENDED" | "RETIRED"
                )
            });
    }
    if target.auth_admin() {
        let path = target.route_path();
        if target.method == Method::GET
            && path
                .strip_prefix(&format!("{AUTH_PREFIX}/applications/"))
                .and_then(|suffix| suffix.strip_suffix("/consent-groups"))
                .is_some_and(|application| id(application).is_ok())
        {
            return exact(value, &["groups"])
                && value["groups"].as_array().is_some_and(|groups| {
                    groups.len() <= 1000
                        && groups
                            .iter()
                            .all(|group| consent_group_projection(group, true))
                });
        }
        if path == AUTH_CONSENT_GROUPS_PREFIX && target.method == Method::POST {
            return consent_group_projection(value, false);
        }
        if let Some(raw_group) = path
            .strip_prefix(&format!("{AUTH_CONSENT_GROUPS_PREFIX}/"))
            .and_then(|suffix| suffix.strip_suffix("/clients"))
        {
            return target.method == Method::POST
                && id(raw_group).is_ok()
                && exact(value, &["groupId", "clientId", "version"])
                && value["groupId"].as_str() == Some(raw_group)
                && value["clientId"]
                    .as_str()
                    .is_some_and(|client| id(client).is_ok())
                && value["version"]
                    .as_i64()
                    .is_some_and(|version| version >= 2);
        }
        if path == AUTH_CLIENTS_PREFIX && target.method == Method::GET {
            return exact(value, &["records", "page"])
                && value["records"].as_array().is_some_and(|records| {
                    records.len() <= 100 && records.iter().all(auth_client_projection)
                })
                && exact(&value["page"], &["limit", "offset", "total"])
                && value["page"]["limit"]
                    .as_i64()
                    .is_some_and(|v| (1..=100).contains(&v))
                && value["page"]["offset"]
                    .as_i64()
                    .is_some_and(|v| (0..=100000).contains(&v))
                && value["page"]["total"].as_i64().is_some_and(|v| v >= 0);
        }
        if target.method == Method::GET
            || target.method == Method::POST
            || target.method == Method::PATCH
        {
            if path == AUTH_CLIENTS_PREFIX && target.method == Method::POST {
                return auth_client_projection(value);
            }
            if let Some(raw) = path.strip_prefix(&format!("{AUTH_CLIENTS_PREFIX}/")) {
                let (client, suffix) = raw.split_once('/').unwrap_or((raw, ""));
                if id(client).is_err() {
                    return false;
                }
                if suffix.is_empty() && target.method == Method::GET {
                    return auth_client_projection(value) && value["id"].as_str() == Some(client);
                }
                if suffix.is_empty() && target.method == Method::PATCH {
                    return auth_client_projection(value) && value["id"].as_str() == Some(client);
                }
                if suffix == "credentials" && target.method == Method::POST {
                    return exact(
                        value,
                        &[
                            "clientId",
                            "credentialId",
                            "credentialVersion",
                            "clientVersion",
                            "previousCredentialExpiresAt",
                            "credentialExpiresAt",
                            "clientSecret",
                        ],
                    ) && value["clientId"].as_str() == Some(client)
                        && value["credentialId"]
                            .as_str()
                            .is_some_and(|v| id(v).is_ok())
                        && value["credentialVersion"].as_i64().is_some_and(|v| v >= 1)
                        && value["clientVersion"].as_i64().is_some_and(|v| v >= 1)
                        && (value["previousCredentialExpiresAt"].is_null()
                            || value["previousCredentialExpiresAt"].is_string())
                        && value["credentialExpiresAt"]
                            .as_str()
                            .is_some_and(|raw| chrono::DateTime::parse_from_rfc3339(raw).is_ok())
                        && value["clientSecret"].as_str().is_some_and(opaque);
                }
                if suffix == "credentials/revoke" && target.method == Method::POST {
                    return exact(
                        value,
                        &["clientId", "revokedCredentials", "version", "policyVersion"],
                    ) && value["clientId"].as_str() == Some(client)
                        && value["revokedCredentials"].as_i64().is_some_and(|v| v >= 0)
                        && value["version"].as_i64().is_some_and(|v| v >= 1)
                        && value["policyVersion"].as_i64().is_some_and(|v| v >= 1);
                }
            }
        }
        return false;
    }
    match (target.method.as_str(), target.path.strip_prefix(PREFIX)) {
        (_, Some(path)) if path.starts_with("/keys/") => {
            let expected = if path == "/keys/publish" {
                "PUBLISHED"
            } else if path == "/keys/activate" {
                "ACTIVE"
            } else if target.method == Method::DELETE {
                "RETIRED"
            } else {
                "REVOKED"
            };
            exact(value, &["kid", "status"])
                && value["kid"].as_str().is_some_and(|v| kid(v).is_ok())
                && value["status"] == expected
                && (path == "/keys/publish"
                    || path == "/keys/activate"
                    || value["kid"].as_str()
                        == Some(if target.method == Method::DELETE {
                            &path[6..]
                        } else {
                            &path[6..path.len() - 7]
                        }))
        }
        _ => false,
    }
}
fn invitation_metadata(value: &Value) -> bool {
    let timestamp = |key: &str| {
        value[key]
            .as_str()
            .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
    };
    exact(
        value,
        &[
            "id",
            "recipientRef",
            "purpose",
            "status",
            "version",
            "createdAt",
            "expiresAt",
            "revokedAt",
            "consumedAt",
        ],
    ) && value["id"].as_str().is_some_and(|raw| id(raw).is_ok())
        && value["recipientRef"].as_str().is_some_and(|raw| {
            !raw.is_empty() && raw.len() <= 256 && !raw.chars().any(char::is_control)
        })
        && value["purpose"] == "ACCOUNT_ONBOARDING"
        && matches!(
            value["status"].as_str(),
            Some("ACTIVE" | "EXPIRED" | "REVOKED" | "CONSUMED")
        )
        && value["version"].as_i64().is_some_and(|version| version > 0)
        && timestamp("createdAt")
            .zip(timestamp("expiresAt"))
            .is_some_and(|(created, expires)| expires - created == chrono::Duration::hours(24))
        && ["revokedAt", "consumedAt"]
            .iter()
            .all(|key| value[*key].is_null() || timestamp(key).is_some())
}
fn decode(reply: Reply, target: &Target) -> Result<Value, AppError> {
    if !matches!(reply.status, 200 | 201) {
        return Err(match reply.status {
            400 | 422 => AppError::bad_request("OIDC 配置无效"),
            401 | 403 => AppError::forbidden("身份权威拒绝当前管理授权或近期二次认证"),
            404 => AppError::not_found("OIDC 配置不存在"),
            409 => AppError::conflict("OIDC 配置版本、操作绑定或密钥生命周期已变化"),
            _ => AppError::Unavailable,
        });
    }
    if reply.bytes.len() > LIMIT
        || !reply.content_type.as_deref().is_some_and(|v| {
            v.split(';')
                .next()
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
        })
    {
        return Err(AppError::Unavailable);
    }
    let result = serde_json::from_slice(&reply.bytes).map_err(|_| AppError::Unavailable)?;
    if !safe_result(target, &result) {
        return Err(AppError::Unavailable);
    }
    Ok(result)
}
fn resolve_lookup(
    reply: Reply,
    target: &Target,
    operation: Uuid,
    expired: bool,
) -> Result<Value, AppError> {
    if reply.status == 404 {
        // Identity has taken the mutation's same operation advisory lock before
        // proving absence. After the immutable intent expires, no later new
        // execution can pass its independent database-clock deadline check.
        return if expired {
            Ok(json!({"operationId":operation,"state":"NOT_APPLIED","result":null}))
        } else {
            Err(AppError::Unavailable)
        };
    }
    let result = decode(reply, target)?;
    if target.method == Method::POST
        && (target.route_path() == AUTH_CLIENTS_PREFIX
            || target.route_path() == AUTH_CONSENT_GROUPS_PREFIX
            || target.route_path() == INVITATIONS_PREFIX)
        && result["id"].as_str() != Some(operation.to_string().as_str())
    {
        return Err(AppError::Unavailable);
    }
    if target.auth_admin()
        && target.method == Method::POST
        && target.route_path().ends_with("/credentials")
        && result["credentialId"].as_str() != Some(operation.to_string().as_str())
    {
        return Err(AppError::Unavailable);
    }
    Ok(json!({"operationId":operation,"state":"COMPLETED","result":result}))
}
pub(crate) struct Failure {
    error: AppError,
    operation: Option<Uuid>,
}
impl Failure {
    #[cfg(test)]
    pub(crate) fn with_operation(mut self, operation: Uuid) -> Self {
        self.operation = Some(operation);
        self
    }
}

#[cfg(test)]
fn fresh_deadline() -> Deadline {
    Deadline {
        at: tokio::time::Instant::now() + Duration::from_secs(3),
        dispatched: Arc::new(Mutex::new(None)),
    }
}

#[cfg(test)]
pub(crate) async fn review_current(
    state: &AppState,
    user: &CurrentUser,
    headers: &HeaderMap,
    application: Uuid,
    environment: &str,
    permission: &'static str,
) -> Result<Value, Failure> {
    let mut headers = headers.clone();
    headers.remove("x-identity-operation");
    call(
        state,
        user,
        &headers,
        Target::review_sync_read(application, environment, permission)?,
        None,
        fresh_deadline(),
    )
    .await
    .map(|Json(value)| value)
}

#[cfg(test)]
pub(crate) async fn review_sync(
    state: &AppState,
    user: &CurrentUser,
    headers: &HeaderMap,
    application: Uuid,
    environment: &str,
    mut body: Value,
) -> Result<Value, Failure> {
    if !headers.contains_key("x-identity-operation") {
        return Err(AppError::bad_request("审核同步必须查询或复用浏览器保存的原操作编号").into());
    }
    let object = body
        .as_object_mut()
        .ok_or_else(|| AppError::bad_request("审核同步字段无效"))?;
    object.insert("applicationId".into(), json!(application));
    object.insert("environment".into(), json!(environment));
    object.insert("actorUserId".into(), json!(user.user_id));
    object.insert("actorSessionId".into(), json!(user.session_id));
    call(
        state,
        user,
        headers,
        Target::review(Method::PUT, application, environment)?,
        Some(body),
        fresh_deadline(),
    )
    .await
    .map(|Json(value)| value)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg(test)]
struct InvitationIssue {
    recipient_ref: String,
    purpose: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg(test)]
struct InvitationRevoke {
    expected_version: i64,
}

#[cfg(test)]
async fn list_invitations(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Extension(deadline): Extension<Deadline>,
) -> Result<Json<Value>, Failure> {
    call(
        &state,
        &user,
        &headers,
        Target::invitation(Method::GET, None)?,
        None,
        deadline,
    )
    .await
}

#[cfg(test)]
async fn issue_invitation(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Extension(deadline): Extension<Deadline>,
    ApiJson(body): ApiJson<InvitationIssue>,
) -> Result<Json<Value>, Failure> {
    let body = json!({"actorUserId":user.user_id,"actorSessionId":user.session_id,"recipientRef":body.recipient_ref,"purpose":body.purpose});
    call(
        &state,
        &user,
        &headers,
        Target::invitation(Method::POST, None)?,
        Some(body),
        deadline,
    )
    .await
}

#[cfg(test)]
async fn revoke_invitation(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(raw): Path<String>,
    Extension(deadline): Extension<Deadline>,
    ApiJson(body): ApiJson<InvitationRevoke>,
) -> Result<Json<Value>, Failure> {
    let invitation = id(&raw)?;
    let body = json!({"actorUserId":user.user_id,"actorSessionId":user.session_id,"expectedVersion":body.expected_version});
    call(
        &state,
        &user,
        &headers,
        Target::invitation(Method::POST, Some(invitation))?,
        Some(body),
        deadline,
    )
    .await
}
impl From<AppError> for Failure {
    fn from(error: AppError) -> Self {
        Self {
            error,
            operation: None,
        }
    }
}
impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        if let Some(operation) = self.operation {
            (StatusCode::SERVICE_UNAVAILABLE,Json(json!({"code":"OIDC_REMOTE_RESULT_UNKNOWN","message":"操作结果尚未确认，请查询原操作结果，勿重复提交","operationId":operation}))).into_response()
        } else {
            self.error.into_response()
        }
    }
}
async fn execute(
    boundary: &dyn Boundary,
    transport: &dyn Transport,
    command: &Command,
    deadline: Deadline,
) -> Result<Value, Failure> {
    if tokio::time::Instant::now() >= deadline.at {
        return Err(AppError::Unavailable.into());
    }
    tokio::time::timeout_at(deadline.at, boundary.accept(command))
        .await
        .map_err(|_| AppError::Unavailable)??;
    if tokio::time::Instant::now() >= deadline.at {
        return Err(AppError::Unavailable.into());
    }
    if command.target.mutation() {
        *deadline
            .dispatched
            .lock()
            .map_err(|_| AppError::Unavailable)? = Some(command.operation);
    }
    let result = async {
        let reply = transport.send(command).await?;
        let value = decode(reply, &command.target)?;
        if command.target.method == Method::POST
            && (command.target.route_path() == AUTH_CLIENTS_PREFIX
                || command.target.route_path() == AUTH_CONSENT_GROUPS_PREFIX
                || command.target.route_path() == INVITATIONS_PREFIX)
            && value["id"].as_str() != Some(command.operation.to_string().as_str())
        {
            return Err(AppError::Unavailable);
        }
        if command.target.auth_admin()
            && command.target.method == Method::POST
            && command.target.route_path().ends_with("/credentials")
            && value["credentialId"].as_str() != Some(command.operation.to_string().as_str())
        {
            return Err(AppError::Unavailable);
        }
        if matches!(
            command.target.path.strip_prefix(PREFIX),
            Some("/keys/publish" | "/keys/activate")
        ) && value["kid"] != command.body.as_ref().ok_or(AppError::Unavailable)?["kid"]
        {
            return Err(AppError::Unavailable);
        }
        if command.target.mutation() {
            boundary
                .reconcile(command, &value)
                .await
                .map_err(|_| AppError::Unavailable)?;
        }
        if command.target.mutation() && !command.target.reviewed() {
            boundary
                .acknowledge(
                    command.operation,
                    command.target.projection(),
                    command.target.auth_admin(),
                )
                .await
                .map_err(|_| AppError::Unavailable)?;
        }
        if tokio::time::Instant::now() >= deadline.at {
            return Err(AppError::Unavailable);
        }
        Ok(value)
    };
    match tokio::time::timeout_at(deadline.at, result).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) if !matches!(error, AppError::Unavailable) => Err(error.into()),
        Ok(Err(error)) => Err(Failure {
            error,
            operation: command.target.mutation().then_some(command.operation),
        }),
        Err(_) => Err(Failure {
            error: AppError::Unavailable,
            operation: command.target.mutation().then_some(command.operation),
        }),
    }
}
async fn call(
    state: &AppState,
    user: &CurrentUser,
    headers: &HeaderMap,
    target: Target,
    value: Option<Value>,
    deadline: Deadline,
) -> Result<Json<Value>, Failure> {
    let _permit = state
        .identity_admin_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| AppError::Unavailable)?;
    let value = prepare(&target, value)?;
    let operation = browser_operation(headers, &target)?;
    let command = Command {
        digest: digest(&target, &value)?,
        target,
        body: value,
        operation,
    };
    preauthorize(state, user, &command).await?;
    let transport = HttpTransport::configured(state, &command.target)?;
    Ok(Json(
        execute(
            &PlatformBoundary {
                state,
                user,
                headers,
                metadata: None,
            },
            &transport,
            &command,
            deadline,
        )
        .await?,
    ))
}
#[cfg(test)]
async fn publish(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Extension(d): Extension<Deadline>,
    ApiJson(v): ApiJson<Value>,
) -> Result<Json<Value>, Failure> {
    call(
        &state,
        &user,
        &headers,
        Target::new(Method::POST, "/keys/publish")?,
        Some(v),
        d,
    )
    .await
}
#[cfg(test)]
async fn activate(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Extension(d): Extension<Deadline>,
    ApiJson(v): ApiJson<Value>,
) -> Result<Json<Value>, Failure> {
    call(
        &state,
        &user,
        &headers,
        Target::new(Method::POST, "/keys/activate")?,
        Some(v),
        d,
    )
    .await
}
#[cfg(test)]
async fn retire(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(raw): Path<String>,
    Extension(d): Extension<Deadline>,
) -> Result<Json<Value>, Failure> {
    kid(&raw)?;
    call(
        &state,
        &user,
        &headers,
        Target::new(Method::DELETE, &format!("/keys/{raw}"))?,
        None,
        d,
    )
    .await
}
#[cfg(test)]
async fn revoke(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(raw): Path<String>,
    Extension(d): Extension<Deadline>,
) -> Result<Json<Value>, Failure> {
    kid(&raw)?;
    call(
        &state,
        &user,
        &headers,
        Target::new(Method::POST, &format!("/keys/{raw}/revoke"))?,
        None,
        d,
    )
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProjectionRequest {
    expected_projection_version: Option<i64>,
    #[serde(default)]
    approved_scopes: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct PlatformApplication {
    name: String,
    description: String,
    status: String,
}

async fn projection_source(
    state: &AppState,
    user: &CurrentUser,
    raw_id: &str,
    permission: &'static str,
    recent: bool,
) -> Result<(Uuid, PlatformApplication), AppError> {
    if !state.config.identity_remote_enabled {
        return Err(AppError::not_found("独立认证管理未启用"));
    }
    let application_id = id(raw_id)?;
    let mut tx = crate::auth_review::management_tx(state, user, permission, recent).await?;
    crate::auth_review::require_application(&mut tx, user, application_id).await?;
    let application = sqlx::query_as::<_, PlatformApplication>(
        "SELECT name,description,status FROM applications WHERE id=$1",
    )
    .bind(application_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| AppError::not_found("应用不存在"))?;
    tx.commit().await?;
    Ok((application_id, application))
}

async fn get_application_projection(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(raw_id): Path<String>,
    Extension(d): Extension<Deadline>,
) -> Result<Json<Value>, Failure> {
    let (application_id, _) =
        projection_source(&state, &user, &raw_id, "system:oauth:view", false).await?;
    call(
        &state,
        &user,
        &headers,
        Target::get_projection(application_id),
        None,
        d,
    )
    .await
}

async fn project_application(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(raw_id): Path<String>,
    Extension(d): Extension<Deadline>,
    ApiJson(body): ApiJson<ProjectionRequest>,
) -> Result<Json<Value>, Failure> {
    let (application_id, application) =
        projection_source(&state, &user, &raw_id, "system:oauth:scopes", true).await?;
    if body
        .expected_projection_version
        .is_some_and(|version| version < 1)
    {
        return Err(AppError::bad_request("认证应用投影版本无效").into());
    }
    let status = match application.status.as_str() {
        "ENABLED" => "APPROVED",
        "DISABLED" => "SUSPENDED",
        _ => return Err(AppError::conflict("仅可投影已启用或已停用的平台应用").into()),
    };
    let purpose = if application.description.trim().is_empty() {
        format!("Platform application {}", application.name)
    } else {
        application.description
    };
    let command = json!({
        "applicationId":application_id,
        "expectedVersion":body.expected_projection_version,
        "projection":{
            "ownership":"FIRST_PARTY",
            "partnerId":null,
            "status":status,
            "displayName":application.name,
            "purpose":purpose,
            "approvedScopes":body.approved_scopes
        }
    });
    call(
        &state,
        &user,
        &headers,
        Target::project(application_id),
        Some(command),
        d,
    )
    .await
}

async fn list_auth_clients(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Extension(deadline): Extension<Deadline>,
    Query(query): Query<ClientListQuery>,
) -> Result<Json<Value>, Failure> {
    let query = validate_client_list_query(query)?;
    call(
        &state,
        &user,
        &headers,
        Target::auth_client_list(&query)?,
        None,
        deadline,
    )
    .await
}

#[cfg(test)]
async fn list_consent_groups(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(raw_application): Path<String>,
    Extension(deadline): Extension<Deadline>,
) -> Result<Json<Value>, Failure> {
    let application = id(&raw_application)?;
    call(
        &state,
        &user,
        &headers,
        Target::consent_group_list(application),
        None,
        deadline,
    )
    .await
}

#[cfg(test)]
async fn create_consent_group(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Extension(deadline): Extension<Deadline>,
    ApiJson(value): ApiJson<Value>,
) -> Result<Json<Value>, Failure> {
    call(
        &state,
        &user,
        &headers,
        Target::consent_group_create(),
        Some(value),
        deadline,
    )
    .await
}

#[cfg(test)]
async fn add_consent_group_client(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(raw_group): Path<String>,
    Extension(deadline): Extension<Deadline>,
    ApiJson(value): ApiJson<Value>,
) -> Result<Json<Value>, Failure> {
    let group = id(&raw_group)?;
    call(
        &state,
        &user,
        &headers,
        Target::consent_group_member(group),
        Some(value),
        deadline,
    )
    .await
}

async fn get_auth_client(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(raw_id): Path<String>,
    Extension(deadline): Extension<Deadline>,
) -> Result<Json<Value>, Failure> {
    let client = id(&raw_id)?;
    call(
        &state,
        &user,
        &headers,
        Target::auth_client(Method::GET, &format!("/{client}"))?,
        None,
        deadline,
    )
    .await
}

async fn create_auth_client(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Extension(deadline): Extension<Deadline>,
    ApiJson(value): ApiJson<Value>,
) -> Result<Json<Value>, Failure> {
    call(
        &state,
        &user,
        &headers,
        Target::auth_client(Method::POST, "")?,
        Some(value),
        deadline,
    )
    .await
}

async fn update_auth_client(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(raw_id): Path<String>,
    Extension(deadline): Extension<Deadline>,
    ApiJson(value): ApiJson<Value>,
) -> Result<Json<Value>, Failure> {
    let client = id(&raw_id)?;
    call(
        &state,
        &user,
        &headers,
        Target::auth_client(Method::PATCH, &format!("/{client}"))?,
        Some(value),
        deadline,
    )
    .await
}

async fn rotate_auth_client_credential(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(raw_id): Path<String>,
    Extension(deadline): Extension<Deadline>,
    ApiJson(value): ApiJson<Value>,
) -> Result<Json<Value>, Failure> {
    let client = id(&raw_id)?;
    call(
        &state,
        &user,
        &headers,
        Target::auth_client(Method::POST, &format!("/{client}/credentials"))?,
        Some(value),
        deadline,
    )
    .await
}

async fn revoke_auth_client_credentials(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(raw_id): Path<String>,
    Extension(deadline): Extension<Deadline>,
    ApiJson(value): ApiJson<Value>,
) -> Result<Json<Value>, Failure> {
    let client = id(&raw_id)?;
    call(
        &state,
        &user,
        &headers,
        Target::auth_client(Method::POST, &format!("/{client}/credentials/revoke"))?,
        Some(value),
        deadline,
    )
    .await
}

async fn operation(
    State(state): State<AppState>,
    user: CurrentUser,
    headers: HeaderMap,
    Path(raw): Path<String>,
    Extension(d): Extension<Deadline>,
) -> Result<Json<Value>, Failure> {
    let operation = id(&raw)?;
    let _permit = state
        .identity_admin_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| AppError::Unavailable)?;
    let result = async {
        if let Some(local) = crate::auth_review::local_operation(&state, &user, operation).await? {
            return Ok(local);
        }
        if !state.config.identity_remote_enabled {
            return Err(AppError::not_found("独立认证管理未启用"));
        }
        let row:Option<LookupIntent>=sqlx::query_as("SELECT method,path,request_digest,service_client_id,accepted_at<=clock_timestamp()-interval '60 seconds' AS expired,management_permission,application_ref FROM platform_auth_command_intents WHERE id=$1 AND actor_user_id=$2 AND tenant_id=$3 AND organization_id=$4 AND session_id=$5")
            .bind(operation).bind(user.user_id).bind(user.tenant_id).bind(user.org_id).bind(user.session_id).fetch_optional(&state.pool).await.map_err(|_|AppError::Unavailable)?;
        let Some(intent) = row else {
            let mut tx = lookup_management_tx(&state, &user).await?;
            lock_operation(&mut tx, operation).await?;
            let local:Option<PendingLocal>=sqlx::query_as("SELECT actor_user_id,actor_session_id,management_permission,application_id,remote_required,remote_result IS NOT NULL AS finished FROM platform_auth_review_operations WHERE operation_id=$1")
                .bind(operation).fetch_optional(&mut *tx).await?;
            if let Some(local) = local.as_ref() {
                if local.actor_user_id != user.user_id || local.actor_session_id != user.session_id
                {
                    return Err(AppError::forbidden("原审核操作不属于当前管理员会话"));
                }
                crate::auth_review::assert_actor(
                    &mut tx,
                    &state,
                    &user,
                    crate::auth_review::known_permission(&local.management_permission)?,
                    true,
                )
                .await?;
                if let Some(application) = local.application_id {
                    crate::auth_review::require_application(&mut tx, &user, application).await?;
                }
                if !local.remote_required || local.finished {
                    tx.commit().await.map_err(|_| AppError::Unavailable)?;
                    return crate::auth_review::local_operation(&state, &user, operation)
                        .await?
                        .ok_or(AppError::Unavailable);
                }
            }
            let (exists, same_session, closed): (bool, bool, bool) = sqlx::query_as("SELECT EXISTS(SELECT 1 FROM platform_auth_command_intents WHERE id=$1),EXISTS(SELECT 1 FROM platform_auth_command_intents WHERE id=$1 AND actor_user_id=$2 AND tenant_id=$3 AND organization_id=$4 AND session_id=$5),clock_timestamp()>$6::timestamptz+interval '60 seconds'")
                .bind(operation).bind(user.user_id).bind(user.tenant_id).bind(user.org_id).bind(user.session_id).bind(operation_started_at(operation)?).fetch_one(&mut *tx).await.map_err(|_| AppError::Unavailable)?;
            if exists {
                if !same_session {
                    return Err(AppError::forbidden(
                        "原操作不属于当前管理员会话，请返回发起操作的原会话查询",
                    ));
                }
                // Acceptance may have committed while this lookup waited for
                // the operation lock. Retry its original result next time.
                return Err(AppError::Unavailable);
            }
            if !closed {
                return Err(AppError::Unavailable);
            }
            if local.is_some() {
                crate::auth_review::complete_remote(
                    &mut tx,
                    operation,
                    &json!({"state":"NOT_APPLIED"}),
                )
                .await?;
                admin_audit(
                    &mut tx,
                    &user,
                    &headers,
                    "IDENTITY_AUTH_REVIEW_NOT_APPLIED",
                    &json!({"operationId":operation}).to_string(),
                )
                .await?;
                tx.commit().await.map_err(|_| AppError::Unavailable)?;
                return crate::auth_review::local_operation(&state, &user, operation)
                    .await?
                    .ok_or(AppError::Unavailable);
            }
            tx.commit().await.map_err(|_| AppError::Unavailable)?;
            return Ok(json!({"operationId":operation,"state":"NOT_APPLIED","result":null}));
        };
        let target = Target::from_intent(&intent.method, &intent.path)?;
        let permission = crate::auth_review::known_permission(&intent.management_permission)?;
        let mut tx = crate::auth_review::management_tx(&state, &user, permission, true).await?;
        if let Some(application) = intent.application_ref {
            crate::auth_review::require_application(&mut tx, &user, application).await?;
        }
        if !opaque(&intent.request_digest)
            || service_id(&state, &target)? != intent.service_client_id
        {
            return Err(AppError::conflict("原操作服务登记已变化，需重新配置"));
        }
        admin_audit(
            &mut tx,
            &user,
            &headers,
            if target.reviewed() {
                "IDENTITY_AUTH_REVIEW_COMMAND_LOOKUP"
            } else if target.invitations() {
                "IDENTITY_AUTH_INVITATION_COMMAND_LOOKUP"
            } else if target.projection() {
                "IDENTITY_APPLICATION_PROJECTION_LOOKUP"
            } else if target.auth_admin() {
                "IDENTITY_AUTH_CLIENT_COMMAND_LOOKUP"
            } else {
                "IDENTITY_KEY_COMMAND_RESULT_LOOKUP"
            },
            &json!({"operationId":operation}).to_string(),
        )
        .await?;
        tx.commit().await.map_err(|_| AppError::Unavailable)?;
        *d.dispatched.lock().map_err(|_| AppError::Unavailable)? = Some(operation);
        let command = Command {
            target: target.operation_lookup(operation),
            body: None,
            operation,
            digest: intent.request_digest,
        };
        let transport = HttpTransport::configured(&state, &command.target)?;
        let reply = transport.send(&command).await?;
        let value = resolve_lookup(reply, &target, operation, intent.expired)?;
        if value["state"] == "COMPLETED" {
            let boundary = PlatformBoundary {
                state: &state,
                user: &user,
                headers: &headers,
                metadata: None,
            };
            boundary
                .reconcile(&command, &value["result"])
                .await
                .map_err(|_| AppError::Unavailable)?;
            if target.reviewed() {
                return crate::auth_review::local_operation(&state, &user, operation)
                    .await?
                    .ok_or(AppError::Unavailable);
            }
            boundary
                .acknowledge(operation, target.projection(), target.auth_admin())
                .await
                .map_err(|_| AppError::Unavailable)?;
        } else if target.reviewed() && value["state"] == "NOT_APPLIED" {
            if let Some(local) =
                crate::auth_review::local_not_applied(&state, &user, operation).await?
            {
                return Ok(local);
            }
        }
        Ok(value)
    };
    match tokio::time::timeout_at(d.at, result).await {
        Ok(Ok(value)) => Ok(Json(value)),
        Ok(Err(error)) if !matches!(error, AppError::Unavailable) => Err(error.into()),
        Ok(Err(error)) => Err(Failure {
            error,
            operation: Some(operation),
        }),
        Err(_) => Err(Failure {
            error: AppError::Unavailable,
            operation: Some(operation),
        }),
    }
}

#[derive(sqlx::FromRow)]
struct LookupIntent {
    method: String,
    path: String,
    request_digest: String,
    service_client_id: Uuid,
    expired: bool,
    management_permission: String,
    application_ref: Option<Uuid>,
}

#[derive(sqlx::FromRow)]
struct PendingLocal {
    actor_user_id: Uuid,
    actor_session_id: Uuid,
    management_permission: String,
    application_id: Option<Uuid>,
    remote_required: bool,
    finished: bool,
}

async fn lookup_management_tx<'a>(
    state: &'a AppState,
    user: &CurrentUser,
) -> Result<sqlx::Transaction<'a, sqlx::Postgres>, AppError> {
    for permission in [
        "system:oauth:view",
        "system:oauth:invite",
        "system:oauth:review",
        "system:oauth:publish",
        "system:oauth:register",
        "system:oauth:rotate",
        "system:oauth:disable",
        "system:oauth:scopes",
        "system:oauth:keys",
    ] {
        match crate::auth_review::management_tx(state, user, permission, false).await {
            Ok(tx) => return Ok(tx),
            Err(AppError::Forbidden(_) | AppError::OperationDenied) => {}
            Err(error) => return Err(error),
        }
    }
    Err(AppError::forbidden("当前会话没有认证管理查询权限"))
}

#[cfg(test)]
#[path = "remote_admin_pg_tests.rs"]
mod pg_tests;
#[cfg(test)]
#[path = "remote_admin_tests.rs"]
mod tests;

async fn admin_audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user: &CurrentUser,
    headers: &HeaderMap,
    action: &str,
    detail: &str,
) -> Result<(), AppError> {
    crate::util::audit::write_in_tx(
        tx,
        crate::util::audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(user.tenant_id),
            organization_id: Some(user.org_id),
            action,
            detail,
            ip: &crate::util::audit::client_ip(headers),
            user_agent: &crate::util::audit::ua_of(headers),
        },
    )
    .await
}
