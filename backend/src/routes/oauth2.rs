use crate::util::uuid7::ParseUuid7;
use std::collections::HashMap;

use axum::extract::{Form, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::app_token;
use crate::error::AppError;
use crate::extractors::{bearer_token, ApiJson, CurrentUser};
use crate::oauth_issuance::{self, IssuanceIdentity, StoredBinding, UserBinding};
use crate::routes::oauth_admin::{fetch_client, ClientFull};
use crate::session;
use crate::state::AppState;
use crate::util::{audit, hash, jwt};

/// /oauth 前缀的标准 OAuth2 端点 (form-urlencoded)
pub fn oauth_router() -> Router<AppState> {
    Router::new()
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .route("/userinfo", get(userinfo))
        .route("/introspect", post(introspect))
        .route("/revoke", post(revoke))
        .route("/device/authorization", post(device_authorization))
        .route("/.well-known/openid-configuration", get(discovery))
}

/// /api 前缀的会话鉴权端点 (consent 授权确认 + 微前端会话令牌签发)
pub fn consent_routes() -> Router<AppState> {
    Router::new()
        .route("/api/oauth/consent/{request_id}", get(get_consent))
        .route(
            "/api/oauth/consent/{request_id}/decision",
            post(decide_consent),
        )
        .route("/api/oauth/session-token", post(session_token))
}

// ===== OAuth 风格错误 {"error": "...", "error_description": "..."} =====

pub struct OAuthError {
    status: StatusCode,
    body: Value,
    retry_after: Option<u64>,
}

impl OAuthError {
    fn new(status: StatusCode, code: &str, desc: &str) -> Self {
        Self {
            status,
            body: json!({"error": code, "error_description": desc}),
            retry_after: None,
        }
    }
    pub fn bad_request(code: &str, desc: &str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, desc)
    }
    pub fn invalid_client(desc: &str) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "invalid_client", desc)
    }
    pub fn invalid_grant(desc: &str) -> Self {
        Self::bad_request("invalid_grant", desc)
    }
    pub fn invalid_request(desc: &str) -> Self {
        Self::bad_request("invalid_request", desc)
    }
    pub fn invalid_token(desc: &str) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "invalid_token", desc)
    }
    fn rate_limited(retry: u64) -> Self {
        let mut error = Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "temporarily_unavailable",
            "认证请求过多，请稍后重试",
        );
        error.retry_after = Some(retry.max(1));
        error
    }
    pub fn unavailable(_e: impl std::fmt::Display) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "temporarily_unavailable",
            "授权服务暂不可用，请稍后重试",
        )
    }
    pub fn server(_e: impl std::fmt::Display) -> Self {
        tracing::error!(
            error_code = "OAUTH_SERVER_ERROR",
            "OAuth server operation failed"
        );
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "服务器内部错误",
        )
    }

    fn account_port(error: AppError) -> Self {
        match error {
            AppError::Unavailable => Self::unavailable("account authority unavailable"),
            AppError::RateLimited(retry) => Self::rate_limited(retry),
            other => Self::server(other),
        }
    }
}

impl IntoResponse for OAuthError {
    fn into_response(self) -> Response {
        let mut response = (self.status, Json(self.body)).into_response();
        if let Some(retry) = self.retry_after {
            if let Ok(value) = axum::http::HeaderValue::from_str(&retry.to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
        }
        response
    }
}

impl From<sqlx::Error> for OAuthError {
    fn from(e: sqlx::Error) -> Self {
        match AppError::from(e) {
            AppError::Unavailable => OAuthError::unavailable("database authority unavailable"),
            other => OAuthError::server(other),
        }
    }
}

// ===== 工具 =====

/// Explicit legacy human consumers survive BFF rollout only by exact client ID.
/// Application service credentials and machine grants have separate admission.
pub(crate) fn require_legacy_human_client(
    state: &AppState,
    client_id: &str,
) -> Result<(), AppError> {
    if state.config.bff_enabled
        && !state
            .config
            .bff_legacy_human_clients
            .iter()
            .any(|allowed| allowed == client_id)
    {
        return Err(AppError::forbidden("该旧用户授权客户端未获兼容准入"));
    }
    Ok(())
}

fn legacy_human_client(state: &AppState, client_id: &str) -> Result<(), OAuthError> {
    require_legacy_human_client(state, client_id).map_err(|_| {
        OAuthError::bad_request("unauthorized_client", "该旧用户授权客户端未获兼容准入")
    })
}

fn platform_proof_error(error: AppError) -> OAuthError {
    match error {
        AppError::Unavailable => OAuthError::unavailable("platform authority unavailable"),
        AppError::Server(_) => OAuthError::server("platform authority failed"),
        _ => OAuthError::invalid_grant("来源平台会话已失效"),
    }
}

fn token_authority_error(error: AppError) -> OAuthError {
    match error {
        AppError::Unavailable => OAuthError::unavailable("token authority unavailable"),
        AppError::Server(_) => OAuthError::server("token authority failed"),
        _ => OAuthError::invalid_token("来源认证已失效"),
    }
}

/// OAuth 端点 IP 限流 (滑动窗口 60 秒最多 30 次):
/// 未超限返回客户端 IP 供后续审计使用; 超限返回 429 + Retry-After 响应
fn check_oauth_rate(state: &AppState, headers: &HeaderMap) -> Result<String, AppError> {
    let ip = audit::client_ip(headers);
    match state.record_oauth_request(&ip) {
        None => Ok(ip),
        Some(retry_after) => Err(AppError::RateLimited(retry_after as u64)),
    }
}

fn encode_query(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Basic 客户端认证 (client_id:client_secret base64, 分量 URL 解码)
fn parse_basic_auth(headers: &HeaderMap) -> Option<(String, String)> {
    crate::confidential_clients::parse_basic_auth(headers)
}

/// 客户端认证: client_secret_basic 或 client_secret_post
async fn authenticate_client(
    state: &AppState,
    headers: &HeaderMap,
    form: &HashMap<String, String>,
) -> Result<ClientFull, OAuthError> {
    let (client_id, client_secret) = match parse_basic_auth(headers) {
        Some((u, p)) => (Some(u), Some(p)),
        None => (
            form.get("client_id").cloned(),
            form.get("client_secret").cloned(),
        ),
    };
    let cid = client_id.ok_or_else(|| OAuthError::invalid_client("缺少 client_id"))?;
    let client = fetch_client(&state.pool, &cid)
        .await
        .map_err(OAuthError::unavailable)?
        .ok_or_else(|| OAuthError::invalid_client("客户端不存在"))?;
    if client.status != "ENABLED" {
        return Err(OAuthError::invalid_client("客户端已禁用"));
    }
    if client.client_type == "CONFIDENTIAL" {
        let secret = client_secret.ok_or_else(|| OAuthError::invalid_client("缺少客户端密钥"))?;
        let expect = client.client_secret_hash.clone().unwrap_or_default();
        if hash::sha256_hex(&secret) != expect {
            return Err(OAuthError::invalid_client("客户端密钥错误"));
        }
    }
    Ok(client)
}

/// 解析 scope 参数: 空则使用客户端默认; 必须为客户端注册 scope 的子集
fn resolve_scope(requested: Option<&String>, client: &ClientFull) -> Result<String, OAuthError> {
    let allowed = client.scope_list();
    match requested.map(|s| s.trim()).filter(|s| !s.is_empty()) {
        None => Ok(allowed.join(" ")),
        Some(s) => {
            for p in s.split_whitespace() {
                if !allowed.iter().any(|a| a == p) {
                    return Err(OAuthError::bad_request(
                        "invalid_scope",
                        "请求的 scope 超出客户端注册范围",
                    ));
                }
            }
            Ok(s.split_whitespace().collect::<Vec<_>>().join(" "))
        }
    }
}

fn append_query(uri: &str, pairs: &[(&str, String)]) -> String {
    let sep = if uri.contains('?') { '&' } else { '?' };
    let qs = pairs
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{}={}", k, encode_query(v)))
        .collect::<Vec<_>>()
        .join("&");
    if qs.is_empty() {
        uri.to_string()
    } else {
        format!("{}{}{}", uri, sep, qs)
    }
}

fn registered_redirect(client: &ClientFull, redirect_uri: &str) -> bool {
    crate::routes::oauth_admin::split_csv(&client.redirect_uris)
        .iter()
        .any(|registered| registered == redirect_uri)
}

fn valid_s256_challenge(challenge: &str, method: &str) -> bool {
    method == "S256"
        && challenge.len() == 43
        && base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(challenge)
            .is_ok_and(|bytes| bytes.len() == 32)
}

fn valid_pkce_verifier(verifier: &str) -> bool {
    (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._~".contains(&byte))
}

/// A pending consent is only a request, not a snapshot of lasting authority.
/// Recheck current registration before displaying or returning its callback.
fn consent_registration_active(client: &ClientFull, row: &ConsentDetail) -> bool {
    let scopes = client.scope_list();
    client.status == "ENABLED"
        && client.subject_mode != "MACHINE"
        && row.client_policy_version == Some(client.access_policy_version)
        && row.application_id == client.application_id
        && client.has_grant("authorization_code")
        && registered_redirect(client, &row.redirect_uri)
        && (!client.require_pkce
            || valid_s256_challenge(&row.code_challenge, &row.code_challenge_method))
        && row
            .scope
            .split_whitespace()
            .all(|scope| scopes.iter().any(|allowed| allowed == scope))
}

// ===== 授权码端点 =====

/// GET /oauth/authorize — 校验后 302 跳转前端 consent 页
async fn authorize(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response, OAuthError> {
    let client_id = q
        .get("client_id")
        .cloned()
        .ok_or_else(|| OAuthError::invalid_request("缺少 client_id"))?;
    legacy_human_client(&state, &client_id)?;
    let redirect_uri = q
        .get("redirect_uri")
        .cloned()
        .ok_or_else(|| OAuthError::invalid_request("缺少 redirect_uri"))?;
    let response_type = q.get("response_type").map(|s| s.as_str()).unwrap_or("");
    let client = fetch_client(&state.pool, &client_id)
        .await
        .map_err(OAuthError::unavailable)?
        .ok_or_else(|| OAuthError::invalid_client("客户端不存在"))?;
    if client.status != "ENABLED" {
        return Err(OAuthError::invalid_client("客户端已禁用"));
    }
    if !client.has_grant("authorization_code") {
        return Err(OAuthError::bad_request(
            "unauthorized_client",
            "该客户端未启用授权码模式",
        ));
    }
    if response_type != "code" {
        return Err(OAuthError::bad_request(
            "unsupported_response_type",
            "仅支持 response_type=code",
        ));
    }
    if !registered_redirect(&client, &redirect_uri) {
        return Err(OAuthError::invalid_request(
            "redirect_uri 未在客户端注册列表中",
        ));
    }
    let scope = resolve_scope(q.get("scope"), &client)?;
    let code_challenge = q.get("code_challenge").cloned().unwrap_or_default();
    let method = q.get("code_challenge_method").cloned().unwrap_or_default();
    if client.require_pkce && !valid_s256_challenge(&code_challenge, &method) {
        return Err(OAuthError::invalid_request(
            "该客户端要求 PKCE S256，请提供有效 code_challenge 和 code_challenge_method=S256",
        ));
    }
    if !code_challenge.is_empty() && method != "S256" && method != "plain" {
        return Err(OAuthError::invalid_request(
            "code_challenge_method 仅支持 S256 或 plain",
        ));
    }
    let state_param = q.get("state").cloned().unwrap_or_default();
    let id = crate::util::uuid7::new_uuid();
    let mut tx = begin_issuance(&state, &client, None, None).await?;
    let registration = oauth_issuance::client_registration(&mut tx, &client.client_id)
        .await?
        .ok_or_else(|| OAuthError::invalid_client("客户端用途或应用已失效"))?;
    sqlx::query(
        "INSERT INTO oauth_authorization_requests \
         (id, client_id, redirect_uri, scope, state, code_challenge, code_challenge_method, expires_at, client_policy_version,application_id,application_policy_version) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, now() + interval '600 seconds',$8,$9,$10)",
    )
    .bind(id)
    .bind(&client.client_id)
    .bind(&redirect_uri)
    .bind(&scope)
    .bind(&state_param)
    .bind(&code_challenge)
    .bind(&method)
    .bind(registration.0).bind(registration.1).bind(registration.2)
    .execute(&mut *tx).await?;
    tx.commit().await?;
    let base = state.config.public_base();
    let location = format!("{}/consent?request_id={}", base, id);
    Ok((StatusCode::FOUND, [("Location", location)]).into_response())
}

#[derive(sqlx::FromRow)]
struct ConsentDetail {
    client_id: String,
    redirect_uri: String,
    scope: String,
    state: String,
    code_challenge: String,
    code_challenge_method: String,
    expires_at: DateTime<Utc>,
    consumed: bool,
    client_policy_version: Option<i64>,
    application_id: Option<Uuid>,
    application_policy_version: Option<i64>,
}

async fn load_consent(pool: &sqlx::PgPool, id: Uuid) -> Result<Option<ConsentDetail>, AppError> {
    let row = sqlx::query_as::<_, ConsentDetail>(
        "SELECT client_id, redirect_uri, scope, COALESCE(state,'') AS state, \
         COALESCE(code_challenge,'') AS code_challenge, COALESCE(code_challenge_method,'') AS code_challenge_method, \
         expires_at, consumed, client_policy_version,application_id,application_policy_version FROM oauth_authorization_requests WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// GET /api/oauth/consent/{requestId} — 过期/消费 → 404
async fn get_consent(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(request_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let _ = user;
    let id: Uuid = request_id
        .parse_uuid7()
        .map_err(|_| AppError::not_found("授权请求不存在或已失效"))?;
    let row = load_consent(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("授权请求不存在或已失效"))?;
    require_legacy_human_client(&state, &row.client_id)?;
    if row.consumed || row.expires_at < Utc::now() {
        return Err(AppError::not_found("授权请求不存在或已失效"));
    }
    let client = fetch_client(&state.pool, &row.client_id)
        .await?
        .filter(|client| consent_registration_active(client, &row))
        .ok_or_else(|| AppError::not_found("客户端授权配置已变更，请重新发起授权"))?;
    let mut connection = state
        .pool
        .acquire()
        .await
        .map_err(|_| AppError::Unavailable)?;
    let registration = oauth_issuance::client_registration(&mut connection, &row.client_id)
        .await
        .map_err(|_| AppError::Unavailable)?;
    if registration
        != row
            .client_policy_version
            .map(|version| (version, row.application_id, row.application_policy_version))
    {
        return Err(AppError::not_found("客户端授权配置已变更，请重新发起授权"));
    }
    Ok(Json(json!({
        "client": {"clientId": row.client_id, "clientName": client.client_name, "description": client.description},
        "scope": row.scope, "redirectUri": row.redirect_uri, "state": row.state,
        "expiresAt": row.expires_at,
    })))
}

#[derive(Deserialize)]
pub struct DecisionBody {
    pub approve: bool,
}

/// POST /api/oauth/consent/{requestId}/decision — approve 签发 code (60s), deny 返回 error 重定向
async fn decide_consent(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    Path(request_id): Path<String>,
    ApiJson(body): ApiJson<DecisionBody>,
) -> Result<Json<Value>, AppError> {
    let id: Uuid = request_id
        .parse_uuid7()
        .map_err(|_| AppError::not_found("授权请求不存在或已失效"))?;
    let row = load_consent(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("授权请求不存在或已失效"))?;
    require_legacy_human_client(&state, &row.client_id)?;
    if row.consumed || row.expires_at < Utc::now() {
        return Err(AppError::not_found("授权请求不存在或已失效"));
    }
    let client = fetch_client(&state.pool, &row.client_id)
        .await?
        .ok_or_else(|| AppError::not_found("客户端已失效"))?;
    let mut tx = begin_issuance(&state, &client, Some(user.user_id), Some(user.session_id))
        .await
        .map_err(oauth_error_into)?;
    if !consent_registration_active(&client, &row) {
        return Err(AppError::bad_request(
            "客户端授权配置已变更，请重新发起授权",
        ));
    }
    session::require_active_in_tx(
        &mut tx,
        user.user_id,
        user.tenant_id,
        user.org_id,
        user.session_id,
    )
    .await?;
    let binding = binding_from_session(&mut tx, &user, &client).await?;
    if Some(binding.client_policy_version) != row.client_policy_version
        || binding.application_id != row.application_id
        || binding.application_policy_version != row.application_policy_version
    {
        return Err(AppError::unauthorized(
            "客户端授权策略已变更，请重新发起授权",
        ));
    }
    let marked = sqlx::query(
        "UPDATE oauth_authorization_requests SET consumed = true WHERE id = $1 AND consumed = false AND expires_at > clock_timestamp()",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    if marked.rows_affected() == 0 {
        return Err(AppError::not_found("授权请求不存在或已失效"));
    }
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let redirect_to = if body.approve {
        let code = hash::random_token(43);
        let inserted = sqlx::query(
            "INSERT INTO oauth_authorization_codes \
             (code_hash, client_id, user_id, tenant_id, organization_id, redirect_uri, scope, code_challenge, code_challenge_method, expires_at, id, \
              account_version,membership_id,member_type,identity_version,client_policy_version,application_id,application_policy_version,platform_session_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now() + interval '60 seconds', $10,$11,$12,$13,$14,$15,$16,$17,$18)",
        )
        .bind(hash::sha256_hex(&code))
        .bind(&row.client_id)
        .bind(user.user_id)
        .bind(user.tenant_id)
        .bind(user.org_id)
        .bind(&row.redirect_uri)
        .bind(&row.scope)
        .bind(&row.code_challenge)
        .bind(&row.code_challenge_method)
        .bind(crate::util::uuid7::new_uuid())
        .bind(binding.account_version).bind(binding.membership_id).bind(&binding.member_type)
        .bind(binding.identity_version).bind(binding.client_policy_version)
        .bind(binding.application_id).bind(binding.application_policy_version).bind(binding.platform_session_id).execute(&mut *tx)
        .await?;
        if inserted.rows_affected() != 1 {
            return Err(AppError::Server(anyhow::anyhow!(
                "authorization code was not persisted"
            )));
        }
        let detail = audit::change_detail(
            "oauth_consent",
            id,
            json!({"clientId": row.client_id, "consumed": false}),
            json!({"clientId": row.client_id, "consumed": true, "decision": "APPROVED", "scope": row.scope}),
        );
        audit::write_in_tx(
            &mut tx,
            audit::AuditInfo {
                user_id: Some(user.user_id),
                username: &user.username,
                tenant_id: Some(user.tenant_id),
                organization_id: Some(user.org_id),
                action: "CONSENT_APPROVE",
                detail: &detail,
                ip: &ip,
                user_agent: &ua,
            },
        )
        .await?;
        tx.commit().await?;
        append_query(
            &row.redirect_uri,
            &[("code", code), ("state", row.state.clone())],
        )
    } else {
        let detail = audit::change_detail(
            "oauth_consent",
            id,
            json!({"clientId": row.client_id, "consumed": false}),
            json!({"clientId": row.client_id, "consumed": true, "decision": "DENIED"}),
        );
        audit::write_in_tx(
            &mut tx,
            audit::AuditInfo {
                user_id: Some(user.user_id),
                username: &user.username,
                tenant_id: Some(user.tenant_id),
                organization_id: Some(user.org_id),
                action: "CONSENT_DENY",
                detail: &detail,
                ip: &ip,
                user_agent: &ua,
            },
        )
        .await?;
        tx.commit().await?;
        append_query(
            &row.redirect_uri,
            &[
                ("error", "access_denied".into()),
                ("state", row.state.clone()),
            ],
        )
    };
    Ok(Json(json!({"redirectTo": redirect_to})))
}

// ===== 微前端会话令牌 (迁移 0011, 仿乾坤 HTML 入口子应用) =====

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionTokenBody {
    #[serde(default)]
    app_code: String,
    #[serde(default)]
    application_service_client_ref: Option<String>,
}

/// Session APIs retain 401 for a retired parent proof even though the shared
/// OAuth token path represents that rejection as invalid_grant with status 400.
fn oauth_error_into(e: OAuthError) -> AppError {
    let desc = e
        .body
        .get("error_description")
        .and_then(Value::as_str)
        .unwrap_or("OAuth 处理失败")
        .to_string();
    match e.status {
        StatusCode::BAD_REQUEST
            if e.body.get("error").and_then(Value::as_str) == Some("invalid_grant") =>
        {
            AppError::unauthorized(desc)
        }
        StatusCode::BAD_REQUEST => AppError::bad_request(desc),
        StatusCode::UNAUTHORIZED => AppError::unauthorized(desc),
        StatusCode::SERVICE_UNAVAILABLE => AppError::Unavailable,
        _ => AppError::Server(anyhow::anyhow!(desc)),
    }
}

/// POST /api/oauth/session-token — 会话鉴权, 为当前会话用户签发微前端子应用访问令牌
///
/// body: {"appCode": "vue-workshop"}；applications.code 定位应用 → oauth_clients.application_id
/// 精确选择应用服务客户端，继承当前成员与来源准入，签发至多 5 分钟的专用令牌。
/// → 201 {"accessToken","expiresIn","tokenType":"Bearer"}；明文 token 仅出现在响应体,
/// 不落库不打日志 (库内仅存 token_hash)。
async fn session_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<SessionTokenBody>,
) -> Result<Response, AppError> {
    let source_claims = jwt::verify(&bearer_token(&headers)?, &state.config.jwt_secret)
        .map_err(|_| AppError::unauthorized("来源会话凭证已过期"))?;
    let source_deadline = DateTime::<Utc>::from_timestamp(source_claims.exp, 0)
        .ok_or_else(|| AppError::unauthorized("来源会话期限无效"))?;
    let app_code = body.app_code.trim();
    if app_code.is_empty() {
        return Err(AppError::bad_request("缺少 appCode"));
    }
    let app_id: Uuid =
        sqlx::query_scalar("SELECT id FROM applications WHERE code = $1 AND status = 'ENABLED'")
            .bind(app_code)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| AppError::not_found("应用不可用"))?;
    // Check the selected tenant and organization before exposing service-client
    // configuration. Recheck after acquiring issuance locks as grants can change
    // while this request waits.
    const APP_GRANT_SQL: &str = "SELECT EXISTS(SELECT 1 FROM tenant_applications ta \
        JOIN organization_applications oa ON oa.application_id=ta.application_id \
        WHERE ta.tenant_id=$1 AND oa.organization_id=$2 AND ta.application_id=$3)";
    let granted: bool = sqlx::query_scalar(APP_GRANT_SQL)
        .bind(user.tenant_id)
        .bind(user.org_id)
        .bind(app_id)
        .fetch_one(&state.pool)
        .await
        .map_err(|_| AppError::Unavailable)?;
    if !granted {
        return Err(AppError::not_found("应用不可用"));
    }
    let mut service_clients: Vec<String> = sqlx::query_scalar(
        "SELECT client_id FROM oauth_clients WHERE application_id=$1 AND status='ENABLED' \
         AND client_type='CONFIDENTIAL' AND ($2::text IS NULL OR client_id=$2)",
    )
    .bind(app_id)
    .bind(body.application_service_client_ref.as_deref())
    .fetch_all(&state.pool)
    .await
    .map_err(|_| AppError::Unavailable)?;
    if service_clients.is_empty() {
        return Err(AppError::not_found("该应用未关联可用的服务客户端"));
    }
    if service_clients.len() != 1 {
        return Err(AppError::bad_request(
            "请指定唯一 applicationServiceClientRef",
        ));
    }
    let client = fetch_client(&state.pool, &service_clients.remove(0))
        .await
        .map_err(|_| AppError::Unavailable)?
        .ok_or_else(|| AppError::not_found("应用服务客户端已失效"))?;
    let mut tx = begin_issuance(&state, &client, Some(user.user_id), Some(user.session_id))
        .await
        .map_err(oauth_error_into)?;
    let granted: bool = sqlx::query_scalar(APP_GRANT_SQL)
        .bind(user.tenant_id)
        .bind(user.org_id)
        .bind(app_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| AppError::Unavailable)?;
    if !granted {
        return Err(AppError::not_found("应用不可用"));
    }
    #[derive(sqlx::FromRow)]
    struct ParentProof {
        account_version: i64,
        membership_id: Uuid,
        member_type: String,
        identity_version: i64,
        origin_issuer: String,
        origin_client_ref: String,
        origin_client_policy_version: i64,
        expires_at: DateTime<Utc>,
    }
    let proof=sqlx::query_as::<_,ParentProof>(
        "SELECT account_version,membership_id,member_type,identity_version,origin_issuer,origin_client_ref,origin_client_policy_version,expires_at \
         FROM platform_sessions WHERE id=$1 AND user_id=$2 AND tenant_id=$3 AND organization_id=$4 \
           AND platform_session_active(id)",
    ).bind(user.session_id).bind(user.user_id).bind(user.tenant_id).bind(user.org_id)
    .fetch_optional(&mut *tx).await.map_err(|_|AppError::Unavailable)?
    .ok_or_else(||AppError::unauthorized("平台上下文已失效"))?;
    let application_policy_version: i64 = sqlx::query_scalar(
        "SELECT a.access_policy_version FROM applications a \
         JOIN oauth_clients c ON c.application_id=a.id AND c.client_id=$2 AND c.status='ENABLED' \
           AND c.client_type='CONFIDENTIAL' AND c.access_policy_version=$3 \
           AND (c.subject_mode='MACHINE' OR $4=ANY(c.allowed_member_types)) \
         WHERE a.id=$1 AND a.code=$5 AND a.status='ENABLED' AND $4=ANY(a.allowed_member_types)",
    )
    .bind(app_id)
    .bind(&client.client_id)
    .bind(client.access_policy_version)
    .bind(&proof.member_type)
    .bind(app_code)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| AppError::Unavailable)?
    .ok_or_else(|| AppError::forbidden("当前成员不能访问该应用"))?;
    let parent_expires = proof.expires_at;
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let scope = resolve_scope(None, &client).map_err(oauth_error_into)?;
    let now = Utc::now();
    let ttl = client.access_token_validity.min(300);
    if ttl <= 0 {
        return Err(AppError::bad_request("客户端令牌有效期配置无效"));
    }
    let expires_at = (now + chrono::Duration::seconds(ttl))
        .min(parent_expires)
        .min(source_deadline);
    let expires_in = (expires_at - now).num_seconds();
    if expires_in <= 0 {
        return Err(AppError::unauthorized("会话即将过期，请重新登录"));
    }
    let jti = crate::util::uuid7::new_uuid();
    let claims = jwt::Claims {
        sub: user.user_id.to_string(),
        typ: "access".into(),
        tenant: Some(user.tenant_id.to_string()),
        org: Some(user.org_id.to_string()),
        cid: Some(client.client_id.clone()),
        scope: Some(scope.clone()),
        sid: None,
        ver: None,
        aud: Some(app_code.to_string()),
        jti: Some(jti.to_string()),
        iat: now.timestamp(),
        exp: expires_at.timestamp(),
    };
    let access_token = jwt::sign(&claims, &state.config.jwt_secret)?;
    let token_id: Uuid = sqlx::query_scalar(
        "INSERT INTO oauth_tokens \
         (token_hash, token_type, jwt, user_id, client_id, tenant_id, organization_id, \
          scope, expires_at, token_use, audience, jti, application_id, platform_session_id, id,storage_version, \
          account_version,membership_id,member_type,identity_version,client_policy_version,application_policy_version, \
          origin_issuer,origin_client_ref,origin_client_policy_version,application_service_client_ref) \
         VALUES ($1, 'access', NULL, $2, $3, $4, $5, $6, $7, 'app', $8, $9, $10, $11, $12,2, \
          $13,$14,$15,$16,$17,$18,$19,$20,$21,$3) RETURNING id",
    )
    .bind(hash::sha256_hex(&access_token))
    .bind(user.user_id)
    .bind(&client.client_id)
    .bind(user.tenant_id)
    .bind(user.org_id)
    .bind(&scope)
    .bind(expires_at)
    .bind(app_code)
    .bind(jti)
    .bind(app_id)
    .bind(user.session_id)
    .bind(crate::util::uuid7::new_uuid())
    .bind(proof.account_version).bind(proof.membership_id).bind(&proof.member_type).bind(proof.identity_version)
    .bind(client.access_policy_version).bind(application_policy_version)
    .bind(&proof.origin_issuer).bind(&proof.origin_client_ref).bind(proof.origin_client_policy_version)
    .fetch_one(&mut *tx)
    .await?;
    let detail = audit::change_detail(
        "app_token",
        token_id,
        Value::Null,
        json!({"applicationId": app_id, "clientId": client.client_id, "audience": app_code,
            "jti": jti, "expiresAt": expires_at}),
    );
    audit::write_in_tx_for_app(
        &mut tx,
        audit::AuditInfo {
            user_id: Some(user.user_id),
            username: &user.username,
            tenant_id: Some(user.tenant_id),
            organization_id: Some(user.org_id),
            action: "TOKEN_ISSUED",
            detail: &detail,
            ip: &ip,
            user_agent: &ua,
        },
        app_id,
    )
    .await?;
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM platform_sessions s WHERE s.id=$1 \
         AND platform_source_active(s.origin_issuer,s.origin_client_ref,s.origin_client_policy_version,s.member_type) \
         AND platform_session_active(s.id) AND $2::timestamptz>clock_timestamp() \
         AND $3::timestamptz>clock_timestamp())",
    )
        .bind(user.session_id)
        .bind(parent_expires.min(source_deadline))
        .bind(DateTime::<Utc>::from_timestamp(claims.exp, 0))
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| AppError::Unavailable)?;
    if !active {
        return Err(AppError::unauthorized("平台上下文已失效"));
    }
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "accessToken": access_token,
            "expiresIn": expires_in,
            "tokenType": "Bearer",
            "audience": app_code,
        })),
    )
        .into_response())
}

/// A context can authorize a new grant only with the proof it already holds.
async fn binding_from_session(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user: &CurrentUser,
    client: &ClientFull,
) -> Result<UserBinding, AppError> {
    let proof:Option<(i64,Uuid,String,i64)>=sqlx::query_as(
        "SELECT account_version,membership_id,member_type,identity_version FROM platform_sessions \
         WHERE id=$1 AND user_id=$2 AND tenant_id=$3 AND organization_id=$4 AND platform_session_active(id)",
    ).bind(user.session_id).bind(user.user_id).bind(user.tenant_id).bind(user.org_id)
    .fetch_optional(&mut **tx).await.map_err(|_| AppError::Unavailable)?;
    let (account_version, membership_id, member_type, identity_version) =
        proof.ok_or_else(|| AppError::unauthorized("平台上下文已失效"))?;
    let (client_policy_version, application_id, application_policy_version) =
        oauth_issuance::client_registration(tx.as_mut(), &client.client_id)
            .await
            .map_err(|_| AppError::Unavailable)?
            .ok_or_else(|| AppError::unauthorized("客户端用途已失效"))?;
    let binding = UserBinding {
        account_version,
        membership_id,
        member_type,
        identity_version,
        client_policy_version,
        application_id,
        application_policy_version,
        platform_session_id: Some(user.session_id),
    };
    if oauth_issuance::binding_identity(
        tx.as_mut(),
        user.user_id,
        user.tenant_id,
        user.org_id,
        &client.client_id,
        &binding,
    )
    .await
    .map_err(|_| AppError::Unavailable)?
    .is_none()
    {
        return Err(AppError::unauthorized("成员准入已失效"));
    }
    Ok(binding)
}

// ===== 令牌签发 =====

struct IssuedToken {
    response: Value,
}

/// Serialize issuance with user revocation and application/client policy changes.
async fn begin_issuance<'a>(
    state: &'a AppState,
    client: &ClientFull,
    user_id: Option<Uuid>,
    source_session_id: Option<Uuid>,
) -> Result<sqlx::Transaction<'a, sqlx::Postgres>, OAuthError> {
    if let Some(id) = source_session_id {
        crate::bff::require_platform_session(state, id)
            .await
            .map_err(platform_proof_error)?;
    }
    let mut tx = state.pool.begin().await?;
    oauth_issuance::lock_user(&mut tx, user_id).await?;
    if let Some(app_id) = client.application_id {
        let app: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM applications WHERE id=$1 AND status='ENABLED' FOR UPDATE",
        )
        .bind(app_id)
        .fetch_optional(&mut *tx)
        .await?;
        if app.is_none() {
            return Err(OAuthError::invalid_client("应用已失效"));
        }
    }
    let mut client_ids = vec![client.client_id.clone()];
    if let Some(sid) = source_session_id {
        let owner = user_id.ok_or_else(|| OAuthError::invalid_grant("来源会话缺少账号"))?;
        if let Some(source) = session::lock_oidc_source_in_tx(&mut tx, sid, owner)
            .await
            .map_err(platform_proof_error)?
        {
            client_ids.push(source);
        }
    }
    client_ids.sort();
    client_ids.dedup();
    let locked: Vec<(String, String, i64, Option<Uuid>)> = sqlx::query_as(
        "SELECT client_id,status,access_policy_version,application_id FROM oauth_clients \
         WHERE client_id=ANY($1) ORDER BY client_id FOR UPDATE",
    )
    .bind(&client_ids)
    .fetch_all(&mut *tx)
    .await?;
    if locked.len() != client_ids.len()
        || !locked.iter().any(|(id, status, version, app)| {
            id == &client.client_id
                && status == "ENABLED"
                && *version == client.access_policy_version
                && *app == client.application_id
        })
    {
        return Err(OAuthError::invalid_client("客户端配置已变更，请重新请求"));
    }
    if let Some(id) = source_session_id {
        session::require_source_active_in_tx(&mut tx, id)
            .await
            .map_err(platform_proof_error)?;
    }
    Ok(tx)
}

/// Sign and persist the complete pair and its issuance audit in the caller's transaction.
#[allow(clippy::too_many_arguments)]
async fn issue_tokens_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    state: &AppState,
    client: &ClientFull,
    user_id: Option<Uuid>,
    username: &str,
    tenant_id: Option<Uuid>,
    org_id: Option<Uuid>,
    binding: StoredBinding,
    scope: &str,
    ip: &str,
    ua: &str,
) -> Result<IssuedToken, OAuthError> {
    let parent_session = binding.platform_session_id;
    let parent_deadline: Option<DateTime<Utc>> = if let Some(id) = parent_session {
        Some(sqlx::query_scalar("SELECT expires_at FROM platform_sessions WHERE id=$1 AND platform_session_active(id)")
            .bind(id).fetch_optional(&mut **tx).await?
            .ok_or_else(|| OAuthError::invalid_grant("来源平台会话已失效"))?)
    } else {
        None
    };
    let allowed = client.scope_list();
    if scope
        .split_whitespace()
        .any(|part| !allowed.iter().any(|value| value == part))
    {
        return Err(OAuthError::bad_request(
            "invalid_scope",
            "scope 超出客户端当前注册范围",
        ));
    }
    let response = oauth_issuance::issue_in_tx(
        tx,
        state,
        client,
        IssuanceIdentity {
            user_id,
            tenant_id,
            organization_id: org_id,
            binding,
        },
        scope,
    )
    .await
    .map_err(|e| match e {
        AppError::Unauthorized(_) => OAuthError::invalid_grant("用户或授权上下文已失效"),
        AppError::Unavailable => OAuthError::unavailable("authority check failed"),
        _ => OAuthError::server(e),
    })?;
    let audit_written = sqlx::query(
        "INSERT INTO audit_logs(user_id,username,tenant_id,organization_id,action,detail,ip,user_agent,request_id) \
         VALUES($1,COALESCE((SELECT username FROM users WHERE id=$1),$2),$3,$4,'TOKEN_ISSUED',$5,$6,$7,$8)",
    )
    .bind(user_id).bind(if username.is_empty() { &client.client_id } else { username })
    .bind(tenant_id).bind(org_id)
    .bind(format!("签发令牌: 客户端 {} scope={}", client.client_id, scope))
    .bind(ip).bind(ua).bind(crate::observability::request_id())
    .execute(&mut **tx).await?;
    if audit_written.rows_affected() != 1 {
        return Err(OAuthError::server("issuance audit was not persisted"));
    }
    if let Some(id) = parent_session {
        session::require_source_active_in_tx(tx, id)
            .await
            .map_err(platform_proof_error)?;
        let live: bool = sqlx::query_scalar("SELECT $1::timestamptz>clock_timestamp()")
            .bind(parent_deadline)
            .fetch_one(&mut **tx)
            .await?;
        if !live {
            return Err(OAuthError::invalid_grant("来源平台会话已过期"));
        }
    }
    Ok(IssuedToken { response })
}

/// POST /oauth/token — 5 种授权模式
async fn token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Response, OAuthError> {
    let ip = match check_oauth_rate(&state, &headers) {
        Ok(ip) => ip,
        Err(error) => return Ok(error.into_response()),
    };
    let ua = audit::ua_of(&headers);
    let grant = form.get("grant_type").cloned().unwrap_or_default();
    if grant.is_empty() {
        return Err(OAuthError::invalid_request("缺少 grant_type"));
    }
    if grant == "refresh_token"
        && form
            .get("refresh_token")
            .is_some_and(|token| session::is_platform_refresh(token))
    {
        if state.config.bff_enabled && !state.config.bff_legacy_login_enabled {
            return Err(OAuthError::invalid_grant("请通过统一登录入口刷新会话"));
        }
        // Platform refresh grants are public browser sessions, never OAuth
        // application grants. Supplying client credentials cannot convert one.
        if form.contains_key("client_id")
            || form.contains_key("client_secret")
            || parse_basic_auth(&headers).is_some()
        {
            return Err(OAuthError::invalid_request(
                "平台会话刷新不能携带 OAuth 客户端凭据",
            ));
        }
        if form.contains_key("scope") {
            return Err(OAuthError::invalid_request("平台会话刷新不能修改 scope"));
        }
        let refreshed = session::refresh(&state, &form["refresh_token"])
            .await
            .map_err(|error| match error {
                AppError::Unavailable => OAuthError::unavailable("platform session refresh failed"),
                AppError::Server(_) => OAuthError::server("platform session refresh failed"),
                _ => OAuthError::invalid_grant("平台刷新令牌无效或已失效，请重新登录"),
            })?;
        return Ok((
            [(header::CACHE_CONTROL, "no-store"), (header::PRAGMA, "no-cache")],
            Json(json!({
                "access_token": refreshed.access_token, "refresh_token": refreshed.refresh_token,
                "expires_in": refreshed.expires_in, "refresh_expires_in": refreshed.refresh_expires_in,
                "token_type": "Bearer",
            })),
        ).into_response());
    }
    let client = authenticate_client(&state, &headers, &form).await?;
    if !client.has_grant(&grant) {
        return Err(OAuthError::bad_request(
            "unauthorized_client",
            "该客户端未启用此授权模式",
        ));
    }
    let result = match grant.as_str() {
        "authorization_code" => grant_authorization_code(&state, &client, &form, &ip, &ua).await,
        "refresh_token" => grant_refresh_token(&state, &client, &form, &ip, &ua).await,
        "client_credentials" => grant_client_credentials(&state, &client, &form, &ip, &ua).await,
        "password" => grant_password(&state, &client, &form, &ip, &ua).await,
        "urn:ietf:params:oauth:grant-type:device_code" => {
            grant_device_code(&state, &client, &form, &ip, &ua).await
        }
        _ => Err(OAuthError::bad_request(
            "unsupported_grant_type",
            "不支持的授权模式",
        )),
    }?;
    Ok((
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::PRAGMA, "no-cache"),
        ],
        Json(result.response),
    )
        .into_response())
}

#[derive(sqlx::FromRow)]
struct AuthCodeRow {
    id: Uuid,
    client_id: String,
    user_id: Uuid,
    tenant_id: Option<Uuid>,
    organization_id: Option<Uuid>,
    redirect_uri: String,
    scope: String,
    code_challenge: String,
    code_challenge_method: String,
    expires_at: DateTime<Utc>,
    consumed: bool,
    #[sqlx(flatten)]
    binding: StoredBinding,
}

async fn grant_authorization_code(
    state: &AppState,
    client: &ClientFull,
    form: &HashMap<String, String>,
    ip: &str,
    ua: &str,
) -> Result<IssuedToken, OAuthError> {
    legacy_human_client(state, &client.client_id)?;
    let code = form
        .get("code")
        .ok_or_else(|| OAuthError::invalid_request("缺少 code"))?;
    let redirect_uri = form
        .get("redirect_uri")
        .ok_or_else(|| OAuthError::invalid_request("缺少 redirect_uri"))?;
    let digest = hash::sha256_hex(code);
    let (owner, parent_session): (Uuid, Option<Uuid>) = sqlx::query_as(
        "SELECT user_id,platform_session_id FROM oauth_authorization_codes WHERE code_hash=$1",
    )
    .bind(&digest)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| OAuthError::invalid_grant("授权码无效"))?;
    let mut tx = begin_issuance(state, client, Some(owner), parent_session).await?;
    let row = sqlx::query_as::<_, AuthCodeRow>(
        "SELECT id, client_id, user_id, tenant_id, organization_id, redirect_uri, scope, \
         COALESCE(code_challenge,'') AS code_challenge, COALESCE(code_challenge_method,'') AS code_challenge_method, \
         expires_at, consumed, account_version,membership_id,member_type,identity_version,client_policy_version,application_id,application_policy_version,platform_session_id FROM oauth_authorization_codes WHERE code_hash = $1 FOR UPDATE",
    )
    .bind(&digest)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| OAuthError::invalid_grant("授权码无效"))?;
    if row.consumed || row.user_id != owner || row.binding.platform_session_id != parent_session {
        return Err(OAuthError::invalid_grant("授权码已被使用"));
    }
    if row.expires_at <= Utc::now() {
        return Err(OAuthError::invalid_grant("授权码已过期"));
    }
    if row.client_id != client.client_id {
        return Err(OAuthError::invalid_grant("授权码与客户端不匹配"));
    }
    if &row.redirect_uri != redirect_uri {
        return Err(OAuthError::invalid_grant("redirect_uri 与授权请求不一致"));
    }
    if !registered_redirect(client, redirect_uri) {
        return Err(OAuthError::invalid_grant(
            "redirect_uri 已不在客户端当前注册列表中",
        ));
    }
    if client.require_pkce && !valid_s256_challenge(&row.code_challenge, &row.code_challenge_method)
    {
        return Err(OAuthError::invalid_grant(
            "该客户端当前要求 PKCE S256，请重新发起授权",
        ));
    }
    if !row.code_challenge.is_empty() {
        let verifier = form
            .get("code_verifier")
            .ok_or_else(|| OAuthError::invalid_request("缺少 code_verifier (PKCE)"))?;
        if client.require_pkce && !valid_pkce_verifier(verifier) {
            return Err(OAuthError::invalid_grant(
                "code_verifier 格式不符合 PKCE 要求",
            ));
        }
        let ok = match row.code_challenge_method.as_str() {
            "S256" => hash::b64url_sha256(verifier) == row.code_challenge,
            _ => verifier.as_str() == row.code_challenge.as_str(), // plain
        };
        if !ok {
            return Err(OAuthError::invalid_grant("PKCE 校验失败"));
        }
    }
    let marked = sqlx::query(
        "UPDATE oauth_authorization_codes SET consumed = true WHERE id = $1 AND consumed = false AND expires_at > clock_timestamp()",
    )
    .bind(row.id)
    .execute(&mut *tx)
    .await?;
    if marked.rows_affected() == 0 {
        return Err(OAuthError::invalid_grant("授权码已被使用"));
    }
    let result = issue_tokens_in_tx(
        &mut tx,
        state,
        client,
        Some(row.user_id),
        "",
        row.tenant_id,
        row.organization_id,
        row.binding,
        &row.scope,
        ip,
        ua,
    )
    .await?;
    tx.commit().await?;
    Ok(result)
}

#[derive(sqlx::FromRow)]
struct RefreshRow {
    id: Uuid,
    user_id: Option<Uuid>,
    client_id: String,
    tenant_id: Option<Uuid>,
    organization_id: Option<Uuid>,
    scope: String,
    expires_at: DateTime<Utc>,
    revoked: bool,
    #[sqlx(flatten)]
    binding: StoredBinding,
}

async fn grant_refresh_token(
    state: &AppState,
    client: &ClientFull,
    form: &HashMap<String, String>,
    ip: &str,
    ua: &str,
) -> Result<IssuedToken, OAuthError> {
    let rt = form
        .get("refresh_token")
        .ok_or_else(|| OAuthError::invalid_request("缺少 refresh_token"))?;
    let digest = hash::sha256_hex(rt);
    let (owner,parent_session): (Option<Uuid>,Option<Uuid>) = sqlx::query_as(
        "SELECT user_id,platform_session_id FROM oauth_tokens WHERE token_hash=$1 AND token_type='refresh' AND token_use='oauth'",
    ).bind(&digest).fetch_optional(&state.pool).await?
     .ok_or_else(|| OAuthError::invalid_grant("刷新令牌无效"))?;
    if owner.is_some() {
        legacy_human_client(state, &client.client_id)?;
    }
    let mut tx = begin_issuance(state, client, owner, parent_session).await?;
    let row = sqlx::query_as::<_, RefreshRow>(
        "SELECT id, user_id, client_id, tenant_id, organization_id, COALESCE(scope,'') AS scope, \
         expires_at, revoked, account_version,membership_id,member_type,identity_version,client_policy_version,application_id,application_policy_version,platform_session_id FROM oauth_tokens WHERE token_hash = $1 AND token_type = 'refresh' AND token_use='oauth' FOR UPDATE",
    )
    .bind(&digest)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| OAuthError::invalid_grant("刷新令牌无效"))?;
    if row.client_id != client.client_id
        || row.user_id != owner
        || row.binding.platform_session_id != parent_session
    {
        return Err(OAuthError::invalid_grant("刷新令牌与客户端不匹配"));
    }
    if row.user_id.is_none() && client.client_type != "CONFIDENTIAL" {
        return Err(OAuthError::bad_request(
            "unauthorized_client",
            "机器令牌刷新仅允许经过认证的机密客户端使用",
        ));
    }
    if row.revoked {
        return Err(OAuthError::invalid_grant("刷新令牌已被吊销"));
    }
    if row.expires_at <= Utc::now() {
        return Err(OAuthError::invalid_grant("刷新令牌已过期"));
    }
    // scope 可收窄, 不可扩大; 省略时沿用原令牌 scope (RFC 6749 §6: omitted → original grant)
    let requested = form
        .get("scope")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let scope = match requested {
        None => row.scope.clone(),
        Some(s) => {
            let allowed = client.scope_list();
            for p in s.split_whitespace() {
                if !allowed.iter().any(|a| a == p) {
                    return Err(OAuthError::bad_request(
                        "invalid_scope",
                        "请求的 scope 超出客户端注册范围",
                    ));
                }
            }
            s.split_whitespace().collect::<Vec<_>>().join(" ")
        }
    };
    let original: Vec<&str> = row.scope.split_whitespace().collect();
    for p in scope.split_whitespace() {
        if !original.contains(&p) {
            return Err(OAuthError::bad_request(
                "invalid_scope",
                "scope 超出原令牌范围",
            ));
        }
    }
    let final_scope = if scope.is_empty() {
        row.scope.clone()
    } else {
        scope
    };
    // 轮换: 旧 refresh 吊销
    let marked = sqlx::query("UPDATE oauth_tokens SET revoked = true WHERE id = $1 AND revoked=false AND expires_at > clock_timestamp()")
        .bind(row.id)
        .execute(&mut *tx)
        .await?;
    if marked.rows_affected() != 1 {
        return Err(OAuthError::invalid_grant("刷新令牌已被使用或已过期"));
    }
    let result = issue_tokens_in_tx(
        &mut tx,
        state,
        client,
        row.user_id,
        "",
        row.tenant_id,
        row.organization_id,
        row.binding,
        &final_scope,
        ip,
        ua,
    )
    .await?;
    tx.commit().await?;
    Ok(result)
}

async fn grant_client_credentials(
    state: &AppState,
    client: &ClientFull,
    form: &HashMap<String, String>,
    ip: &str,
    ua: &str,
) -> Result<IssuedToken, OAuthError> {
    if client.client_type != "CONFIDENTIAL"
        || !matches!(client.subject_mode.as_str(), "MACHINE" | "LEGACY_MIXED")
    {
        return Err(OAuthError::bad_request(
            "unauthorized_client",
            "client_credentials 仅允许经过认证的机密客户端使用",
        ));
    }
    let scope = resolve_scope(form.get("scope"), client)?;
    let mut tx = begin_issuance(state, client, None, None).await?;
    let application_policy_version: Option<i64> = match client.application_id {
        Some(id) => {
            sqlx::query_scalar("SELECT access_policy_version FROM applications WHERE id=$1")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?
        }
        None => None,
    };
    let binding = StoredBinding {
        client_policy_version: Some(client.access_policy_version),
        application_id: client.application_id,
        application_policy_version,
        ..Default::default()
    };
    let result = issue_tokens_in_tx(
        &mut tx, state, client, None, "", None, None, binding, &scope, ip, ua,
    )
    .await?;
    tx.commit().await?;
    Ok(result)
}

async fn grant_password(
    state: &AppState,
    client: &ClientFull,
    form: &HashMap<String, String>,
    ip: &str,
    ua: &str,
) -> Result<IssuedToken, OAuthError> {
    legacy_human_client(state, &client.client_id)?;
    let username = form
        .get("username")
        .ok_or_else(|| OAuthError::invalid_request("缺少 username"))?;
    let password = form
        .get("password")
        .ok_or_else(|| OAuthError::invalid_request("缺少 password"))?;
    if username.is_empty()
        || username.len() > state.config.login_identifier_max_bytes
        || password.is_empty()
        || password.len() > 1024
    {
        return Err(OAuthError::invalid_request("账号或密码超出允许长度"));
    }
    if let Some(retry) = state.login_limits.admit_source(ip) {
        return Err(OAuthError::rate_limited(retry));
    }
    let input_key = format!("input:{}", hash::sha256_hex(username));
    if let Some(retry) = state.login_limits.check_account(&input_key) {
        return Err(OAuthError::rate_limited(retry));
    }
    let row = state
        .authentication
        .by_username(username)
        .await
        .map_err(OAuthError::account_port)?;
    let Some(row) = row else {
        state.login_limits.record_failure(&input_key);
        return Err(OAuthError::invalid_grant("用户名或密码错误"));
    };
    let user_id = row.id;
    let account_key = format!("user:{user_id}");
    if let Some(retry) = state.login_limits.check_account(&account_key) {
        return Err(OAuthError::rate_limited(retry));
    }
    if row.status != "ENABLED" {
        return Err(OAuthError::invalid_grant("用户已被禁用"));
    }
    let verified = state
        .authentication
        .verify(&row, password)
        .await
        .map_err(OAuthError::account_port)?;
    if !verified {
        state.login_limits.record_failure(&input_key);
        state.login_limits.record_failure(&account_key);
        return Err(OAuthError::invalid_grant("用户名或密码错误"));
    }
    state.login_limits.clear_account(&input_key);
    state.login_limits.clear_account(&account_key);
    let requested_membership = form
        .get("membership_id")
        .map(|value| {
            value
                .parse_uuid7()
                .map_err(|_| OAuthError::invalid_request("membership_id 无效"))
        })
        .transpose()?;
    let mut contexts = sqlx::query_as::<_, (Uuid, Uuid, Uuid)>(
        "SELECT o.tenant_id,m.organization_id,m.id FROM memberships m \
         JOIN organizations o ON o.id=m.organization_id JOIN tenants t ON t.id=o.tenant_id \
         WHERE m.user_id=$1 AND m.status='ENABLED' AND m.member_type=ANY($2) \
           AND ($3::uuid IS NULL OR m.id=$3) AND t.status='ENABLED' \
           AND (t.expires_at IS NULL OR t.expires_at>clock_timestamp()) AND org_chain_active(o.id)",
    )
    .bind(user_id)
    .bind(&client.allowed_member_types)
    .bind(requested_membership)
    .fetch_all(&state.pool)
    .await?;
    if contexts.len() != 1 {
        return Err(OAuthError::invalid_grant(
            "请选择唯一有效的 membership_id 后重新认证",
        ));
    }
    let ctx = contexts.remove(0);
    let scope = resolve_scope(form.get("scope"), client)?;
    let mut tx = begin_issuance(state, client, Some(user_id), None).await?;
    let matches = if state.remote_accounts.is_some() {
        // The HTTPS password verification completed before begin_issuance took
        // its user lock. Only the non-secret owner projection is read here.
        sqlx::query_scalar::<_, bool>(
            "SELECT status='ENABLED' AND session_version=$2 FROM users WHERE id=$1",
        )
        .bind(row.id)
        .bind(row.account_version)
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(false)
    } else {
        let mut accounts = crate::authentication::PostgresAccountTransaction::new(&mut tx);
        state
            .authentication
            .credential_matches(&mut accounts, &row)
            .await
            .map_err(OAuthError::account_port)?
    };
    if !matches {
        return Err(OAuthError::invalid_grant("用户凭据已变更，请重新认证"));
    }
    let binding =
        oauth_issuance::capture_binding(&mut tx, user_id, ctx.0, ctx.1, ctx.2, &client.client_id)
            .await?
            .ok_or_else(|| OAuthError::invalid_grant("成员准入已失效"))?;
    let result = issue_tokens_in_tx(
        &mut tx,
        state,
        client,
        Some(user_id),
        username,
        Some(ctx.0),
        Some(ctx.1),
        binding.into(),
        &scope,
        ip,
        ua,
    )
    .await?;
    tx.commit().await?;
    Ok(result)
}

#[derive(sqlx::FromRow)]
struct DeviceRow {
    id: Uuid,
    client_id: String,
    scope: String,
    status: String,
    user_id: Option<Uuid>,
    tenant_id: Option<Uuid>,
    organization_id: Option<Uuid>,
    interval_seconds: i64,
    last_polled_at: Option<DateTime<Utc>>,
    expires_at: DateTime<Utc>,
    #[sqlx(flatten)]
    binding: StoredBinding,
}

async fn grant_device_code(
    state: &AppState,
    client: &ClientFull,
    form: &HashMap<String, String>,
    ip: &str,
    ua: &str,
) -> Result<IssuedToken, OAuthError> {
    legacy_human_client(state, &client.client_id)?;
    let device_code = form
        .get("device_code")
        .ok_or_else(|| OAuthError::invalid_request("缺少 device_code"))?;
    let digest = hash::sha256_hex(device_code);
    // A terminal rejection does not need the parent proof that owner revocation
    // already retired. Commit polling metadata without carrying a row lock into
    // the back-channel; every nonterminal row still follows full issuance below.
    let expired = sqlx::query(
        "UPDATE oauth_device_codes SET status='EXPIRED',last_polled_at=clock_timestamp() \
         WHERE device_code_hash=$1 AND client_id=$2 \
         AND (status='EXPIRED' OR expires_at<=clock_timestamp())",
    )
    .bind(&digest)
    .bind(&client.client_id)
    .execute(&state.pool)
    .await?
    .rows_affected();
    if expired == 1 {
        return Err(OAuthError::bad_request("expired_token", "设备码已过期"));
    }
    let (owner, parent_session): (Option<Uuid>, Option<Uuid>) = sqlx::query_as(
        "SELECT user_id,platform_session_id FROM oauth_device_codes WHERE device_code_hash=$1",
    )
    .bind(&digest)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| OAuthError::invalid_grant("设备码无效"))?;
    let mut tx = begin_issuance(state, client, owner, parent_session).await?;
    let row = sqlx::query_as::<_, DeviceRow>(
        "SELECT id, client_id, COALESCE(scope,'') AS scope, status, user_id, tenant_id, organization_id, \
         interval_seconds, last_polled_at, expires_at, account_version,membership_id,member_type,identity_version,client_policy_version,application_id,application_policy_version,platform_session_id FROM oauth_device_codes WHERE device_code_hash = $1 FOR UPDATE",
    )
    .bind(&digest)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| OAuthError::invalid_grant("设备码无效"))?;
    if row.client_id != client.client_id
        || row.user_id != owner
        || row.binding.platform_session_id != parent_session
    {
        return Err(OAuthError::invalid_grant("设备码与客户端不匹配"));
    }
    let registration = oauth_issuance::client_registration(&mut tx, &client.client_id).await?;
    if registration
        != row.binding.client_policy_version.map(|version| {
            (
                version,
                row.binding.application_id,
                row.binding.application_policy_version,
            )
        })
    {
        return Err(OAuthError::invalid_grant("设备授权策略已变更，请重新授权"));
    }
    let now = Utc::now();
    let mut status = row.status.clone();
    if now >= row.expires_at {
        sqlx::query("UPDATE oauth_device_codes SET status = 'EXPIRED' WHERE id = $1")
            .bind(row.id)
            .execute(&mut *tx)
            .await?;
        status = "EXPIRED".into();
    }
    // Polling metadata and non-issuance terminal states commit independently of
    // OAuth error responses; approved consumption stays with the complete pair.
    sqlx::query("UPDATE oauth_device_codes SET last_polled_at = clock_timestamp() WHERE id = $1")
        .bind(row.id)
        .execute(&mut *tx)
        .await?;
    if status == "EXPIRED" {
        tx.commit().await?;
        return Err(OAuthError::bad_request("expired_token", "设备码已过期"));
    }
    if let Some(last) = row.last_polled_at {
        if (now - last).num_seconds() < row.interval_seconds {
            tx.commit().await?;
            return Err(OAuthError::bad_request(
                "slow_down",
                "轮询过于频繁, 请稍后再试",
            ));
        }
    }
    match status.as_str() {
        "DENIED" => {
            tx.commit().await?;
            Err(OAuthError::bad_request(
                "access_denied",
                "用户拒绝了该设备授权",
            ))
        }
        "PENDING" => {
            tx.commit().await?;
            Err(OAuthError::bad_request(
                "authorization_pending",
                "设备授权尚未完成",
            ))
        }
        "APPROVED" => {
            // 标记已消费, 防止重复签发
            let marked = sqlx::query(
                "UPDATE oauth_device_codes SET status = 'EXPIRED' WHERE id = $1 AND status = 'APPROVED' AND expires_at > clock_timestamp()",
            )
            .bind(row.id)
            .execute(&mut *tx)
            .await?;
            if marked.rows_affected() == 0 {
                return Err(OAuthError::bad_request("expired_token", "设备码已使用"));
            }
            if row.user_id.is_none() {
                return Err(OAuthError::invalid_grant("设备码缺少用户身份"));
            }
            let result = issue_tokens_in_tx(
                &mut tx,
                state,
                client,
                row.user_id,
                "",
                row.tenant_id,
                row.organization_id,
                row.binding,
                &row.scope,
                ip,
                ua,
            )
            .await?;
            tx.commit().await?;
            Ok(result)
        }
        _ => Err(OAuthError::invalid_grant("设备码状态异常")),
    }
}

#[derive(sqlx::FromRow)]
struct TokenInspectionRow {
    user_id: Option<Uuid>,
    tenant_id: Option<Uuid>,
    organization_id: Option<Uuid>,
    client_id: String,
    scope: String,
    revoked: bool,
    expires_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    token_type: String,
    token_use: String,
    #[sqlx(flatten)]
    binding: StoredBinding,
}

const TOKEN_INSPECTION_SQL:&str="SELECT user_id,tenant_id,organization_id,client_id,COALESCE(scope,'') AS scope,revoked,expires_at,created_at,token_type,token_use,account_version,membership_id,member_type,identity_version,client_policy_version,application_id,application_policy_version,platform_session_id FROM oauth_tokens WHERE token_hash=$1";

// ===== 用户信息 =====

/// GET /oauth/userinfo — Bearer access token
async fn userinfo(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, OAuthError> {
    let token = bearer_token(&headers).map_err(|_| OAuthError::invalid_token("缺少访问令牌"))?;
    let claims = jwt::verify(&token, &state.config.jwt_secret)
        .map_err(|_| OAuthError::invalid_token("令牌无效或已过期"))?;
    if claims.typ != "access" {
        return Err(OAuthError::invalid_token("令牌类型错误"));
    }
    let row = sqlx::query_as::<_, TokenInspectionRow>(TOKEN_INSPECTION_SQL)
        .bind(hash::sha256_hex(&token))
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| OAuthError::invalid_token("令牌无效或已过期"))?;
    let TokenInspectionRow {
        user_id,
        tenant_id,
        organization_id: org_id,
        scope,
        revoked,
        expires_at,
        token_use,
        client_id,
        binding,
        token_type,
        ..
    } = row;
    if token_type != "access" {
        return Err(OAuthError::invalid_token("令牌类型错误"));
    }
    if revoked || expires_at <= Utc::now() {
        return Err(OAuthError::invalid_token("令牌已被吊销或已过期"));
    }
    if token_use == "app"
        && app_token::inspect_authorized(&state, &token, &claims)
            .await
            .map_err(token_authority_error)?
            .is_none()
    {
        return Err(OAuthError::invalid_token("应用令牌已失效"));
    }
    if token_use == "oauth" {
        if let Some(id) = binding.platform_session_id {
            crate::bff::require_platform_session(&state, id)
                .await
                .map_err(token_authority_error)?;
        }
        let mut connection = state.pool.acquire().await?;
        if oauth_issuance::active_identity(
            &mut connection,
            user_id,
            tenant_id,
            org_id,
            &client_id,
            &binding,
        )
        .await?
        .is_none()
        {
            return Err(OAuthError::invalid_token("用户或授权上下文已失效"));
        }
    } else if token_use != "app" {
        return Err(OAuthError::invalid_token("令牌用途错误"));
    }
    let user_id = user_id.ok_or_else(|| OAuthError::invalid_token("该令牌没有关联用户"))?;
    let (username, real_name, email, phone) = sqlx::query_as::<_, (String, String, String, String)>(
        "SELECT username, COALESCE(real_name,''), COALESCE(email,''), COALESCE(phone,'') FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| OAuthError::invalid_token("用户不存在"))?;

    let tenant_json = match tenant_id {
        Some(t) => {
            sqlx::query_as::<_, (Uuid, String)>("SELECT id, name FROM tenants WHERE id = $1")
                .bind(t)
                .fetch_optional(&state.pool)
                .await?
                .map(|(id, name)| json!({"id": id, "name": name}))
                .unwrap_or(Value::Null)
        }
        None => Value::Null,
    };
    let org_json = match org_id {
        Some(o) => {
            sqlx::query_as::<_, (Uuid, String)>("SELECT id, name FROM organizations WHERE id = $1")
                .bind(o)
                .fetch_optional(&state.pool)
                .await?
                .map(|(id, name)| json!({"id": id, "name": name}))
                .unwrap_or(Value::Null)
        }
        None => Value::Null,
    };
    let roles: Vec<Value> =
        if let Some(o) = org_id.filter(|_| binding.member_type.as_deref() == Some("EMPLOYEE")) {
            state
                .effective_roles(user_id, o)
                .await
                .map_err(OAuthError::unavailable)?
                .into_iter()
                .map(|r| json!(r.name))
                .collect()
        } else {
            Vec::new()
        };
    Ok(Json(json!({
        "sub": user_id,
        "preferred_username": username,
        "real_name": real_name,
        "email": email,
        "phone": phone,
        "tenant": tenant_json,
        "organization": org_json,
        "roles": roles,
        "scope": scope,
    })))
}

// ===== 内省与吊销 =====

/// POST /oauth/introspect — RFC7662
async fn introspect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Response, OAuthError> {
    let ip = audit::client_ip(&headers);
    if let Some(retry_after) = state.record_introspection_attempt(&ip) {
        return Ok(AppError::RateLimited(retry_after as u64).into_response());
    }
    let permit = match state.introspection_auth_slots.try_acquire() {
        Ok(permit) => permit,
        Err(_) => return Ok(AppError::RateLimited(1).into_response()),
    };
    let authentication = authenticate_client(&state, &headers, &form).await;
    drop(permit);
    let caller = match authentication {
        Ok(caller) => caller,
        Err(error) => {
            if error.status == StatusCode::UNAUTHORIZED {
                crate::observability::record_auth_failure("invalid_client");
                if let Some(retry_after) = state.record_introspection_failure(&ip) {
                    return Ok(AppError::RateLimited(retry_after as u64).into_response());
                }
            }
            return Err(error);
        }
    };
    // Public client identifiers are not client authentication credentials.
    if caller.client_type != "CONFIDENTIAL" {
        crate::observability::record_auth_failure("invalid_client");
        if let Some(retry_after) = state.record_introspection_failure(&ip) {
            return Ok(AppError::RateLimited(retry_after as u64).into_response());
        }
        return Err(OAuthError::invalid_client("内省要求机密客户端凭据"));
    }
    if let Some(retry_after) = state.record_introspection_client(&caller.client_id) {
        return Ok(AppError::RateLimited(retry_after as u64).into_response());
    }
    let token = form
        .get("token")
        .ok_or_else(|| OAuthError::invalid_request("缺少 token"))?;
    let row = sqlx::query_as::<_, TokenInspectionRow>(TOKEN_INSPECTION_SQL)
        .bind(hash::sha256_hex(token))
        .fetch_optional(&state.pool)
        .await?;
    let Some(TokenInspectionRow {
        user_id,
        client_id,
        scope,
        created_at,
        expires_at,
        revoked,
        token_use,
        token_type,
        tenant_id,
        organization_id,
        binding,
    }) = row
    else {
        crate::observability::record_auth_failure("inactive_token");
        return Ok(Json(json!({"active":false})).into_response());
    };
    let exp_epoch = expires_at.timestamp();
    if token_type != "access" {
        crate::observability::record_auth_failure("invalid_token");
        return Ok(Json(json!({"active": false})).into_response());
    }
    if token_use == "app" {
        if caller.client_type != "CONFIDENTIAL" {
            return Ok(Json(json!({"active": false})).into_response());
        }
        let claims = match jwt::verify(token, &state.config.jwt_secret) {
            Ok(claims) => claims,
            Err(_) => {
                crate::observability::record_auth_failure("invalid_token");
                return Ok(Json(json!({"active": false})).into_response());
            }
        };
        let Some(info) = app_token::inspect_authorized(&state, token, &claims)
            .await
            .map_err(token_authority_error)?
        else {
            crate::observability::record_auth_failure("inactive_token");
            return Ok(Json(json!({"active": false})).into_response());
        };
        if caller.application_id != Some(info.application_id) {
            crate::observability::record_auth_failure("invalid_token");
            return Ok(Json(json!({"active": false})).into_response());
        }
        return Ok(Json(json!({
            "active": true, "scope": info.scope, "client_id": info.client_id,
            "token_type": "Bearer", "sub": info.user_id,
            "tenant_id": info.tenant_id, "organization_id": info.organization_id,
            "aud": info.audience, "jti": info.jti,
            "exp": info.expires_at.timestamp(), "iat": created_at.timestamp(),
        }))
        .into_response());
    }
    if caller.client_id != client_id {
        crate::observability::record_auth_failure("invalid_token");
        return Ok(Json(json!({"active": false})).into_response());
    }
    if token_use == "oauth" {
        if let Some(id) = binding.platform_session_id {
            match crate::bff::require_platform_session(&state, id).await {
                Ok(()) => {}
                Err(AppError::Unauthorized(_) | AppError::Forbidden(_)) => {
                    return Ok(Json(json!({"active":false})).into_response());
                }
                Err(error) => return Err(token_authority_error(error)),
            }
        }
    }
    let mut connection = state.pool.acquire().await?;
    let identity = if token_use == "oauth" {
        oauth_issuance::active_identity(
            &mut connection,
            user_id,
            tenant_id,
            organization_id,
            &client_id,
            &binding,
        )
        .await?
    } else {
        None
    };
    drop(connection);
    // Introspection authenticates opaque bearer material by its stored digest.
    // Preserve that contract for legacy callers; userinfo still verifies JWTs.
    let active = !revoked && expires_at > Utc::now() && identity.is_some();
    if !active {
        crate::observability::record_auth_failure(if expires_at <= Utc::now() {
            "expired_token"
        } else {
            "inactive_token"
        });
        return Ok(Json(json!({"active": false})).into_response());
    }
    let username = identity.unwrap_or_default();
    Ok(Json(json!({
        "active": true,
        "scope": scope,
        "client_id": client_id,
        "username": username,
        "token_type": "Bearer",
        "exp": exp_epoch,
        "iat": created_at.timestamp(),
        "sub": user_id.map(|u| Value::String(u.to_string())).unwrap_or(Value::Null),
    }))
    .into_response())
}

/// POST /oauth/revoke — RFC7009 (始终 200)
async fn revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Response, OAuthError> {
    if let Err(error) = check_oauth_rate(&state, &headers) {
        return Ok(error.into_response());
    }
    let client = authenticate_client(&state, &headers, &form).await?;
    let token = form
        .get("token")
        .ok_or_else(|| OAuthError::invalid_request("缺少 token"))?;
    let mut tx = state.pool.begin().await?;
    let row = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            bool,
            String,
            String,
            Option<Uuid>,
            Option<Uuid>,
        ),
    >(
        "SELECT id, client_id, revoked, token_type, token_use, tenant_id, organization_id \
         FROM oauth_tokens WHERE token_hash = $1 FOR UPDATE",
    )
    .bind(hash::sha256_hex(token))
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((id, owner_client, revoked, token_type, token_use, tenant_id, org_id)) = row {
        if owner_client == client.client_id && !revoked {
            let changed = sqlx::query(
                "UPDATE oauth_tokens SET revoked = true WHERE id = $1 AND revoked = false",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
            if changed.rows_affected() != 1 {
                return Err(OAuthError::server(
                    "token revocation did not update one row",
                ));
            }
            let ip = audit::client_ip(&headers);
            let ua = audit::ua_of(&headers);
            let detail = audit::change_detail(
                "oauth_token",
                id,
                json!({"clientId": owner_client, "tokenType": token_type, "tokenUse": token_use, "revoked": false}),
                json!({"clientId": owner_client, "tokenType": token_type, "tokenUse": token_use, "revoked": true}),
            );
            audit::write_in_tx(
                &mut tx,
                audit::AuditInfo {
                    user_id: None,
                    username: client.client_id.as_str(),
                    tenant_id,
                    organization_id: org_id,
                    action: "TOKEN_REVOKED",
                    detail: &detail,
                    ip: &ip,
                    user_agent: &ua,
                },
            )
            .await
            .map_err(OAuthError::server)?;
        }
    }
    tx.commit().await?;
    Ok(Json(json!({})).into_response())
}

// ===== 设备码 =====

/// POST /oauth/device/authorization — RFC8628
async fn device_authorization(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Response, OAuthError> {
    if let Err(error) = check_oauth_rate(&state, &headers) {
        return Ok(error.into_response());
    }
    let client = authenticate_client(&state, &headers, &form).await?;
    legacy_human_client(&state, &client.client_id)?;
    if !client.has_grant("urn:ietf:params:oauth:grant-type:device_code") {
        return Err(OAuthError::bad_request(
            "unauthorized_client",
            "该客户端未启用设备码模式",
        ));
    }
    let scope = resolve_scope(form.get("scope"), &client)?;
    let user_code = hash::random_user_code();
    let device_code = hash::random_token(43);
    let mut tx = begin_issuance(&state, &client, None, None).await?;
    let registration = oauth_issuance::client_registration(&mut tx, &client.client_id)
        .await?
        .ok_or_else(|| OAuthError::invalid_client("客户端用途或应用已失效"))?;
    sqlx::query(
        "INSERT INTO oauth_device_codes \
         (device_code_hash, user_code, client_id, scope, interval_seconds, expires_at, id,client_policy_version,application_id,application_policy_version) \
         VALUES ($1, $2, $3, $4, 5, now() + interval '600 seconds', $5,$6,$7,$8)",
    )
    .bind(hash::sha256_hex(&device_code))
    .bind(&user_code)
    .bind(&client.client_id)
    .bind(&scope)
    .bind(crate::util::uuid7::new_uuid())
    .bind(registration.0).bind(registration.1).bind(registration.2)
    .execute(&mut *tx).await?;
    tx.commit().await?;
    let base = state.config.public_base();
    Ok(Json(json!({
        "device_code": device_code,
        "user_code": user_code,
        "verification_uri": format!("{}/device", base),
        "verification_uri_complete": format!("{}/device?user_code={}", base, user_code),
        "expires_in": 600,
        "interval": 5,
    }))
    .into_response())
}

// ===== 发现文档 =====

/// GET /oauth/.well-known/openid-configuration
async fn discovery(State(state): State<AppState>, headers: HeaderMap) -> Json<Value> {
    let base = if !state.config.public_app_url.is_empty() {
        state.config.public_base()
    } else if let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) {
        format!("http://{}", host)
    } else {
        String::new()
    };
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{}/oauth/authorize", base),
        "token_endpoint": format!("{}/oauth/token", base),
        "device_authorization_endpoint": format!("{}/oauth/device/authorization", base),
        "userinfo_endpoint": format!("{}/oauth/userinfo", base),
        "introspection_endpoint": format!("{}/oauth/introspect", base),
        "revocation_endpoint": format!("{}/oauth/revoke", base),
        "response_types_supported": ["code"],
        "grant_types_supported": [
            "authorization_code", "refresh_token", "client_credentials", "password",
            "urn:ietf:params:oauth:grant-type:device_code"
        ],
        "subject_types_supported": ["public"],
        "code_challenge_methods_supported": ["S256", "plain"],
    }))
}

#[cfg(test)]
mod session_adapter_error_tests {
    use super::*;

    #[test]
    fn retired_parent_proof_preserves_session_401_and_oauth_400() {
        let rejected = platform_proof_error(AppError::unauthorized("parent session retired"));
        assert_eq!(rejected.status, StatusCode::BAD_REQUEST);
        assert_eq!(rejected.body["error"], "invalid_grant");
        assert_eq!(
            oauth_error_into(rejected).into_response().status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            OAuthError::invalid_grant("parent session retired")
                .into_response()
                .status(),
            StatusCode::BAD_REQUEST
        );
        for (error, expected) in [
            (
                OAuthError::bad_request("invalid_scope", "scope rejected"),
                StatusCode::BAD_REQUEST,
            ),
            (
                OAuthError::invalid_request("request rejected"),
                StatusCode::BAD_REQUEST,
            ),
            (
                platform_proof_error(AppError::Unavailable),
                StatusCode::SERVICE_UNAVAILABLE,
            ),
        ] {
            assert_eq!(oauth_error_into(error).into_response().status(), expected);
        }
    }
}

#[cfg(test)]
mod pkce_policy_tests {
    use super::*;

    #[test]
    fn s256_challenges_are_canonical_sha256_base64url_values() {
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(valid_s256_challenge(challenge, "S256"));
        for method in ["", "plain", "s256"] {
            assert!(!valid_s256_challenge(challenge, method));
        }
        for invalid in [
            String::new(),
            "A".repeat(42),
            "_".repeat(43),
            format!("{challenge}="),
        ] {
            assert!(!valid_s256_challenge(&invalid, "S256"));
        }
    }

    #[test]
    fn pkce_verifiers_enforce_rfc7636_length_and_unreserved_characters() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert!(valid_pkce_verifier(verifier));
        assert_eq!(
            hash::b64url_sha256(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert!(valid_pkce_verifier(&"a".repeat(128)));
        assert!(valid_pkce_verifier(&"-._~".repeat(11)));
        for invalid in [
            "a".repeat(42),
            "a".repeat(129),
            "/".repeat(43),
            "界".repeat(43),
            format!("{} ", "a".repeat(43)),
        ] {
            assert!(!valid_pkce_verifier(&invalid));
        }
    }
}
