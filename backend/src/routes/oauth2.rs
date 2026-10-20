use std::collections::HashMap;

use axum::extract::{Form, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::app_token;
use crate::oauth_issuance::{self, IssuanceIdentity};
use crate::error::AppError;
use crate::extractors::{bearer_token, ApiJson, CurrentUser};
use crate::routes::oauth_admin::{fetch_client, ClientFull};
use crate::state::AppState;
use crate::session;
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
        .route("/api/oauth/consent/{request_id}/decision", post(decide_consent))
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
        let mut error = Self::new(StatusCode::TOO_MANY_REQUESTS, "temporarily_unavailable", "认证请求过多，请稍后重试");
        error.retry_after = Some(retry.max(1));
        error
    }
    pub fn server(_e: impl std::fmt::Display) -> Self {
        tracing::error!(error_code="OAUTH_SERVER_ERROR", "OAuth server operation failed");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "server_error", "服务器内部错误")
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
        OAuthError::server(e)
    }
}

// ===== 工具 =====

/// OAuth 端点 IP 限流 (滑动窗口 60 秒最多 30 次):
/// 未超限返回客户端 IP 供后续审计使用; 超限返回 429 + Retry-After 响应
fn check_oauth_rate(state: &AppState, headers: &HeaderMap) -> Result<String, Response> {
    let ip = audit::client_ip(headers);
    match state.record_oauth_request(&ip) {
        None => Ok(ip),
        Some(retry_after) => Err(
            AppError::RateLimited(retry_after as u64).into_response(),
        ),
    }
}

fn encode_query(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Basic 客户端认证 (client_id:client_secret base64, 分量 URL 解码)
fn parse_basic_auth(headers:&HeaderMap)->Option<(String,String)> { crate::confidential_clients::parse_basic_auth(headers) }

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
        .map_err(OAuthError::server)?
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
                    return Err(OAuthError::bad_request("invalid_scope", "请求的 scope 超出客户端注册范围"));
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

// ===== 授权码端点 =====

/// GET /oauth/authorize — 校验后 302 跳转前端 consent 页
async fn authorize(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response, OAuthError> {
    let client_id = q.get("client_id").cloned().ok_or_else(|| OAuthError::invalid_request("缺少 client_id"))?;
    let redirect_uri = q
        .get("redirect_uri")
        .cloned()
        .ok_or_else(|| OAuthError::invalid_request("缺少 redirect_uri"))?;
    let response_type = q.get("response_type").map(|s| s.as_str()).unwrap_or("");
    let client = fetch_client(&state.pool, &client_id)
        .await
        .map_err(OAuthError::server)?
        .ok_or_else(|| OAuthError::invalid_client("客户端不存在"))?;
    if client.status != "ENABLED" {
        return Err(OAuthError::invalid_client("客户端已禁用"));
    }
    if !client.has_grant("authorization_code") {
        return Err(OAuthError::bad_request("unauthorized_client", "该客户端未启用授权码模式"));
    }
    if response_type != "code" {
        return Err(OAuthError::bad_request("unsupported_response_type", "仅支持 response_type=code"));
    }
    let registered: Vec<String> = crate::routes::oauth_admin::split_csv(&client.redirect_uris);
    if !registered.iter().any(|u| u == &redirect_uri) {
        return Err(OAuthError::invalid_request("redirect_uri 未在客户端注册列表中"));
    }
    let scope = resolve_scope(q.get("scope"), &client)?;
    let code_challenge = q.get("code_challenge").cloned().unwrap_or_default();
    let method = q.get("code_challenge_method").cloned().unwrap_or_default();
    if !code_challenge.is_empty() && method != "S256" && method != "plain" {
        return Err(OAuthError::invalid_request("code_challenge_method 仅支持 S256 或 plain"));
    }
    let state_param = q.get("state").cloned().unwrap_or_default();
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO oauth_authorization_requests \
         (id, client_id, redirect_uri, scope, state, code_challenge, code_challenge_method, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, now() + interval '600 seconds')",
    )
    .bind(id)
    .bind(&client.client_id)
    .bind(&redirect_uri)
    .bind(&scope)
    .bind(&state_param)
    .bind(&code_challenge)
    .bind(&method)
    .execute(&state.pool)
    .await
    .map_err(OAuthError::server)?;
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
}

async fn load_consent(pool: &sqlx::PgPool, id: Uuid) -> Result<Option<ConsentDetail>, AppError> {
    let row = sqlx::query_as::<_, ConsentDetail>(
        "SELECT client_id, redirect_uri, scope, COALESCE(state,'') AS state, \
         COALESCE(code_challenge,'') AS code_challenge, COALESCE(code_challenge_method,'') AS code_challenge_method, \
         expires_at, consumed FROM oauth_authorization_requests WHERE id = $1",
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
    let id: Uuid = request_id.parse().map_err(|_| AppError::not_found("授权请求不存在或已失效"))?;
    let row = load_consent(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("授权请求不存在或已失效"))?;
    if row.consumed || row.expires_at < Utc::now() {
        return Err(AppError::not_found("授权请求不存在或已失效"));
    }
    let client = fetch_client(&state.pool, &row.client_id).await?;
    let (client_name, description) = match client {
        Some(c) => (c.client_name, c.description),
        None => (row.client_id.clone(), String::new()),
    };
    Ok(Json(json!({
        "client": {"clientId": row.client_id, "clientName": client_name, "description": description},
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
        .parse()
        .map_err(|_| AppError::not_found("授权请求不存在或已失效"))?;
    let row = load_consent(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("授权请求不存在或已失效"))?;
    if row.consumed || row.expires_at < Utc::now() {
        return Err(AppError::not_found("授权请求不存在或已失效"));
    }
    let client = fetch_client(&state.pool, &row.client_id)
        .await?
        .ok_or_else(|| AppError::not_found("客户端已失效"))?;
    let mut tx = begin_issuance(&state, &client, Some(user.user_id))
        .await
        .map_err(oauth_error_into)?;
    if oauth_issuance::active_identity(
        &mut tx,
        Some(user.user_id),
        Some(user.tenant_id),
        Some(user.org_id),
        &row.client_id,
    )
    .await?
    .is_none()
    {
        return Err(AppError::unauthorized("授权上下文已失效"));
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
             (code_hash, client_id, user_id, tenant_id, organization_id, redirect_uri, scope, code_challenge, code_challenge_method, expires_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now() + interval '60 seconds')",
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
        .execute(&mut *tx)
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
}

/// OAuthError → AppError (会话端点复用 /oauth/token 签发路径, 统一错误口径)
fn oauth_error_into(e: OAuthError) -> AppError {
    let desc = e
        .body
        .get("error_description")
        .and_then(Value::as_str)
        .unwrap_or("OAuth 处理失败")
        .to_string();
    match e.status {
        StatusCode::BAD_REQUEST => AppError::bad_request(desc),
        StatusCode::UNAUTHORIZED => AppError::unauthorized(desc),
        _ => AppError::Server(anyhow::anyhow!(desc)),
    }
}

/// POST /api/oauth/session-token — 会话鉴权, 为当前会话用户签发微前端子应用访问令牌
///
/// body: {"appCode": "vue-workshop"}；applications.code 定位应用 → oauth_clients.application_id
/// 取第一个已启用的机密客户端，检查租户/组织两级授权并签发 5 分钟内的专用令牌。
/// → 201 {"accessToken","expiresIn","tokenType":"Bearer"}；明文 token 仅出现在响应体,
/// 不落库不打日志 (库内仅存 token_hash)。
async fn session_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: CurrentUser,
    ApiJson(body): ApiJson<SessionTokenBody>,
) -> Result<Response, AppError> {
    let app_code = body.app_code.trim();
    if app_code.is_empty() {
        return Err(AppError::bad_request("缺少 appCode"));
    }
    let app_id: Uuid = sqlx::query_scalar("SELECT id FROM applications WHERE code = $1 AND status = 'ENABLED'")
        .bind(app_code)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found("应用不可用"))?;
    let authorized: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM tenant_applications ta \
         JOIN organization_applications oa ON oa.application_id = ta.application_id \
         WHERE ta.tenant_id = $1 AND oa.organization_id = $2 AND ta.application_id = $3)",
    )
    .bind(user.tenant_id).bind(user.org_id).bind(app_id)
    .fetch_one(&state.pool).await?;
    if !authorized {
        return Err(AppError::not_found("应用不可用"));
    }
    let shell_client_id: String =
        sqlx::query_scalar("SELECT client_id FROM oauth_clients \
                            WHERE application_id = $1 AND status = 'ENABLED' \
                            AND client_type = 'CONFIDENTIAL' ORDER BY created_at LIMIT 1")
            .bind(app_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| AppError::not_found("该应用未关联 OAuth 客户端"))?;
    let client = fetch_client(&state.pool, &shell_client_id)
        .await?
        .ok_or_else(|| AppError::not_found("该应用未关联 OAuth 客户端"))?;
    if client.status != "ENABLED" || client.client_type != "CONFIDENTIAL"
        || client.application_id != Some(app_id) {
        return Err(AppError::not_found("应用不可用"));
    }
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let scope = resolve_scope(None, &client).map_err(oauth_error_into)?;
    let parent_expires: DateTime<Utc> = sqlx::query_scalar(
        "SELECT expires_at FROM platform_sessions WHERE id = $1 AND user_id = $2 \
         AND revoked_at IS NULL AND expires_at > now()",
    ).bind(user.session_id).bind(user.user_id)
     .fetch_optional(&state.pool).await?
     .ok_or_else(|| AppError::unauthorized("会话已失效"))?;
    let now = Utc::now();
    let ttl = client.access_token_validity.min(300);
    if ttl <= 0 { return Err(AppError::bad_request("客户端令牌有效期配置无效")); }
    let expires_at = std::cmp::min(now + chrono::Duration::seconds(ttl), parent_expires);
    let expires_in = (expires_at - now).num_seconds();
    if expires_in <= 0 { return Err(AppError::unauthorized("会话即将过期，请重新登录")); }
    let jti = Uuid::new_v4();
    let claims = jwt::Claims {
        sub: user.user_id.to_string(), typ: "access".into(),
        tenant: Some(user.tenant_id.to_string()), org: Some(user.org_id.to_string()),
        cid: Some(client.client_id.clone()), scope: Some(scope.clone()),
        sid: None, ver: None,
        aud: Some(app_code.to_string()), jti: Some(jti.to_string()),
        iat: now.timestamp(), exp: expires_at.timestamp(),
    };
    let access_token = jwt::sign(&claims, &state.config.jwt_secret)?;
    let mut tx = state.pool.begin().await?;
    let token_id: Uuid = sqlx::query_scalar(
        "INSERT INTO oauth_tokens \
         (token_hash, token_type, jwt, user_id, client_id, tenant_id, organization_id, \
          scope, expires_at, token_use, audience, jti, application_id, platform_session_id) \
         VALUES ($1, 'access', NULL, $2, $3, $4, $5, $6, $7, 'app', $8, $9, $10, $11) RETURNING id",
    )
    .bind(hash::sha256_hex(&access_token)).bind(user.user_id).bind(&client.client_id)
    .bind(user.tenant_id).bind(user.org_id).bind(&scope).bind(expires_at)
    .bind(app_code).bind(jti).bind(app_id).bind(user.session_id)
    .fetch_one(&mut *tx).await?;
    let detail = audit::change_detail("app_token", token_id, Value::Null,
        json!({"applicationId": app_id, "clientId": client.client_id, "audience": app_code,
            "jti": jti, "expiresAt": expires_at}));
    audit::write_in_tx_for_app(&mut tx, audit::AuditInfo {
        user_id: Some(user.user_id), username: &user.username,
        tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
        action: "TOKEN_ISSUED", detail: &detail,
        ip: &ip, user_agent: &ua,
    }, app_id).await?;
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

// ===== 令牌签发 =====

struct IssuedToken {
    response: Value,
}

type IssuanceClientConfig = (String, Option<String>, String, String, String, i64, i64);

/// Lock mutations in the same order as session/member revocation: user, client,
/// then the one-time credential. Re-read the authenticated client under its lock.
async fn begin_issuance<'a>(
    state: &'a AppState,
    client: &ClientFull,
    user_id: Option<Uuid>,
) -> Result<sqlx::Transaction<'a, sqlx::Postgres>, OAuthError> {
    let mut tx = state.pool.begin().await?;
    oauth_issuance::lock_user(&mut tx, user_id).await?;
    let current: Option<IssuanceClientConfig> =
        sqlx::query_as(
            "SELECT status, client_secret_hash, client_type, grant_types, scopes, \
             access_token_validity, refresh_token_validity FROM oauth_clients \
             WHERE id=$1 AND client_id=$2 FOR UPDATE",
        )
        .bind(client.id)
        .bind(&client.client_id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((status, secret, kind, grants, scopes, access_ttl, refresh_ttl)) = current else {
        return Err(OAuthError::invalid_client("客户端已失效"));
    };
    if status != "ENABLED"
        || secret != client.client_secret_hash
        || kind != client.client_type
        || grants != client.grant_types
        || scopes != client.scopes
        || access_ttl != client.access_token_validity
        || refresh_ttl != client.refresh_token_validity
    {
        return Err(OAuthError::invalid_client("客户端配置已变更，请重新请求"));
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
    scope: &str,
    ip: &str,
    ua: &str,
) -> Result<IssuedToken, OAuthError> {
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
        },
        scope,
    )
    .await
    .map_err(|e| match e {
        AppError::Unauthorized(_) => OAuthError::invalid_grant("用户或授权上下文已失效"),
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
    Ok(IssuedToken { response })
}

#[allow(clippy::too_many_arguments)]
async fn issue_tokens(
    state: &AppState,
    client: &ClientFull,
    user_id: Option<Uuid>,
    username: &str,
    tenant_id: Option<Uuid>,
    org_id: Option<Uuid>,
    scope: &str,
    ip: &str,
    ua: &str,
) -> Result<IssuedToken, OAuthError> {
    let mut tx = begin_issuance(state, client, user_id).await?;
    let result = issue_tokens_in_tx(
        &mut tx, state, client, user_id, username, tenant_id, org_id, scope, ip, ua,
    )
    .await?;
    tx.commit().await?;
    Ok(result)
}

/// POST /oauth/token — 5 种授权模式
async fn token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Result<Response, OAuthError> {
    let ip = match check_oauth_rate(&state, &headers) {
        Ok(ip) => ip,
        Err(resp) => return Ok(resp),
    };
    let ua = audit::ua_of(&headers);
    let grant = form.get("grant_type").cloned().unwrap_or_default();
    if grant.is_empty() {
        return Err(OAuthError::invalid_request("缺少 grant_type"));
    }
    if grant == "refresh_token" && form.get("refresh_token").is_some_and(|token| session::is_platform_refresh(token)) {
        // Platform refresh grants are public browser sessions, never OAuth
        // application grants. Supplying client credentials cannot convert one.
        if form.contains_key("client_id") || form.contains_key("client_secret") || parse_basic_auth(&headers).is_some() {
            return Err(OAuthError::invalid_request("平台会话刷新不能携带 OAuth 客户端凭据"));
        }
        if form.contains_key("scope") {
            return Err(OAuthError::invalid_request("平台会话刷新不能修改 scope"));
        }
        let refreshed = session::refresh(&state, &form["refresh_token"]).await.map_err(|error| match error {
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
        return Err(OAuthError::bad_request("unauthorized_client", "该客户端未启用此授权模式"));
    }
    let result = match grant.as_str() {
        "authorization_code" => grant_authorization_code(&state, &client, &form, &ip, &ua).await,
        "refresh_token" => grant_refresh_token(&state, &client, &form, &ip, &ua).await,
        "client_credentials" => grant_client_credentials(&state, &client, &form, &ip, &ua).await,
        "password" => grant_password(&state, &client, &form, &ip, &ua).await,
        "urn:ietf:params:oauth:grant-type:device_code" => grant_device_code(&state, &client, &form, &ip, &ua).await,
        _ => Err(OAuthError::bad_request("unsupported_grant_type", "不支持的授权模式")),
    }?;
    Ok(([(header::CACHE_CONTROL, "no-store"), (header::PRAGMA, "no-cache")], Json(result.response)).into_response())
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
}

async fn grant_authorization_code(
    state: &AppState,
    client: &ClientFull,
    form: &HashMap<String, String>,
    ip: &str,
    ua: &str,
) -> Result<IssuedToken, OAuthError> {
    let code = form
        .get("code")
        .ok_or_else(|| OAuthError::invalid_request("缺少 code"))?;
    let redirect_uri = form
        .get("redirect_uri")
        .ok_or_else(|| OAuthError::invalid_request("缺少 redirect_uri"))?;
    let digest = hash::sha256_hex(code);
    let owner: Uuid =
        sqlx::query_scalar("SELECT user_id FROM oauth_authorization_codes WHERE code_hash=$1")
            .bind(&digest)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| OAuthError::invalid_grant("授权码无效"))?;
    let mut tx = begin_issuance(state, client, Some(owner)).await?;
    let row = sqlx::query_as::<_, AuthCodeRow>(
        "SELECT id, client_id, user_id, tenant_id, organization_id, redirect_uri, scope, \
         COALESCE(code_challenge,'') AS code_challenge, COALESCE(code_challenge_method,'') AS code_challenge_method, \
         expires_at, consumed FROM oauth_authorization_codes WHERE code_hash = $1 FOR UPDATE",
    )
    .bind(&digest)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| OAuthError::invalid_grant("授权码无效"))?;
    if row.consumed || row.user_id != owner {
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
    if !row.code_challenge.is_empty() {
        let verifier = form
            .get("code_verifier")
            .ok_or_else(|| OAuthError::invalid_request("缺少 code_verifier (PKCE)"))?;
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
    let owner: Option<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM oauth_tokens WHERE token_hash=$1 AND token_type='refresh' AND token_use='oauth'",
    ).bind(&digest).fetch_optional(&state.pool).await?
     .ok_or_else(|| OAuthError::invalid_grant("刷新令牌无效"))?;
    let mut tx = begin_issuance(state, client, owner).await?;
    let row = sqlx::query_as::<_, RefreshRow>(
        "SELECT id, user_id, client_id, tenant_id, organization_id, COALESCE(scope,'') AS scope, \
         expires_at, revoked FROM oauth_tokens WHERE token_hash = $1 AND token_type = 'refresh' AND token_use='oauth' FOR UPDATE",
    )
    .bind(&digest)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| OAuthError::invalid_grant("刷新令牌无效"))?;
    if row.client_id != client.client_id || row.user_id != owner {
        return Err(OAuthError::invalid_grant("刷新令牌与客户端不匹配"));
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
        if !original.iter().any(|o| *o == p) {
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
    let scope = resolve_scope(form.get("scope"), client)?;
    issue_tokens(state, client, None, "", None, None, &scope, ip, ua).await
}

async fn grant_password(
    state: &AppState,
    client: &ClientFull,
    form: &HashMap<String, String>,
    ip: &str,
    ua: &str,
) -> Result<IssuedToken, OAuthError> {
    let username = form
        .get("username")
        .ok_or_else(|| OAuthError::invalid_request("缺少 username"))?;
    let password = form
        .get("password")
        .ok_or_else(|| OAuthError::invalid_request("缺少 password"))?;
    if username.is_empty() || username.len() > state.config.login_identifier_max_bytes || password.is_empty() || password.len() > 1024 {
        return Err(OAuthError::invalid_request("账号或密码超出允许长度"));
    }
    if let Some(retry) = state.login_limits.admit_source(ip) {
        return Err(OAuthError::rate_limited(retry));
    }
    let input_key = format!("input:{}", hash::sha256_hex(username));
    if let Some(retry) = state.login_limits.check_account(&input_key) {
        return Err(OAuthError::rate_limited(retry));
    }
    let row = sqlx::query_as::<_, (Uuid, String, String, bool)>(
        "SELECT id, password_hash, status, is_super_admin FROM users WHERE username = $1",
    )
    .bind(username)
    .fetch_optional(&state.pool)
    .await?;
    let Some(row) = row else {
        state.login_limits.record_failure(&input_key);
        return Err(OAuthError::invalid_grant("用户名或密码错误"));
    };
    let (user_id, password_hash, status, _) = row;
    let account_key = format!("user:{user_id}");
    if let Some(retry) = state.login_limits.check_account(&account_key) {
        return Err(OAuthError::rate_limited(retry));
    }
    if status != "ENABLED" {
        return Err(OAuthError::invalid_grant("用户已被禁用"));
    }
    let verified = state.passwords.verify(&password_hash, password).await.map_err(|error| match error {
        AppError::RateLimited(retry) => OAuthError::rate_limited(retry),
        other => OAuthError::server(other),
    })?;
    if !verified {
        state.login_limits.record_failure(&input_key);
        state.login_limits.record_failure(&account_key);
        return Err(OAuthError::invalid_grant("用户名或密码错误"));
    }
    state.login_limits.clear_account(&input_key);
    state.login_limits.clear_account(&account_key);
    // 默认取第一个 membership 作为上下文
    let ctx = sqlx::query_as::<_, (Uuid, Uuid)>(
        "SELECT o.tenant_id, m.organization_id FROM memberships m \
         JOIN organizations o ON o.id = m.organization_id WHERE m.user_id = $1 \
         ORDER BY m.created_at LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| OAuthError::invalid_grant("用户无组织成员关系, 无法签发令牌"))?;
    let scope = resolve_scope(form.get("scope"), client)?;
    let mut tx = begin_issuance(state, client, Some(user_id)).await?;
    let current_password: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    if current_password.as_deref() != Some(password_hash.as_str()) {
        return Err(OAuthError::invalid_grant("用户凭据已变更，请重新认证"));
    }
    let result = issue_tokens_in_tx(
        &mut tx,
        state,
        client,
        Some(user_id),
        username,
        Some(ctx.0),
        Some(ctx.1),
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
}

async fn grant_device_code(
    state: &AppState,
    client: &ClientFull,
    form: &HashMap<String, String>,
    ip: &str,
    ua: &str,
) -> Result<IssuedToken, OAuthError> {
    let device_code = form
        .get("device_code")
        .ok_or_else(|| OAuthError::invalid_request("缺少 device_code"))?;
    let digest = hash::sha256_hex(device_code);
    let owner: Option<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM oauth_device_codes WHERE device_code_hash=$1")
            .bind(&digest)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| OAuthError::invalid_grant("设备码无效"))?;
    let mut tx = begin_issuance(state, client, owner).await?;
    let row = sqlx::query_as::<_, DeviceRow>(
        "SELECT id, client_id, COALESCE(scope,'') AS scope, status, user_id, tenant_id, organization_id, \
         interval_seconds, last_polled_at, expires_at FROM oauth_device_codes WHERE device_code_hash = $1 FOR UPDATE",
    )
    .bind(&digest)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| OAuthError::invalid_grant("设备码无效"))?;
    if row.client_id != client.client_id || row.user_id != owner {
        return Err(OAuthError::invalid_grant("设备码与客户端不匹配"));
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
    let row = sqlx::query_as::<_, (Option<Uuid>, Option<Uuid>, Option<Uuid>, String, bool, DateTime<Utc>, String, String)>(
        "SELECT user_id, tenant_id, organization_id, COALESCE(scope,'') AS scope, revoked, expires_at, token_use, client_id \
         FROM oauth_tokens WHERE token_hash = $1 AND token_type = 'access'",
    )
    .bind(hash::sha256_hex(&token))
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| OAuthError::invalid_token("令牌无效或已过期"))?;
    let (user_id, tenant_id, org_id, scope, revoked, expires_at, token_use, client_id) = row;
    if revoked || expires_at <= Utc::now() {
        return Err(OAuthError::invalid_token("令牌已被吊销或已过期"));
    }
    if token_use == "app"
        && app_token::inspect(&state.pool, &token, &claims)
            .await?
            .is_none()
    {
        return Err(OAuthError::invalid_token("应用令牌已失效"));
    }
    if token_use == "oauth" {
        let mut connection = state.pool.acquire().await?;
        if oauth_issuance::active_identity(&mut connection, user_id, tenant_id, org_id, &client_id)
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
    let roles: Vec<Value> = if let Some(o) = org_id {
        state
            .effective_roles(user_id, o)
            .await
            .unwrap_or_default()
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
    let row = sqlx::query_as::<
        _,
        (
            Option<Uuid>,
            String,
            String,
            DateTime<Utc>,
            DateTime<Utc>,
            bool,
            Option<i64>,
            String,
            String,
            Option<Uuid>,
            Option<Uuid>,
        ),
    >(
        "SELECT user_id, client_id, COALESCE(scope,'') AS scope, created_at, expires_at, revoked, \
         EXTRACT(EPOCH FROM expires_at)::bigint AS exp_epoch, token_use \
         , token_type, tenant_id, organization_id \
         FROM oauth_tokens WHERE token_hash = $1",
    )
    .bind(hash::sha256_hex(token))
    .fetch_optional(&state.pool)
    .await?;
    let Some((
        user_id,
        client_id,
        scope,
        created_at,
        expires_at,
        revoked,
        exp_epoch,
        token_use,
        token_type,
        tenant_id,
        organization_id,
    )) = row
    else {
        crate::observability::record_auth_failure("inactive_token");
        return Ok(Json(json!({"active": false})).into_response());
    };
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
        let Some(info) = app_token::inspect(&state.pool, token, &claims).await? else {
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
    let mut connection = state.pool.acquire().await?;
    let identity = if token_use == "oauth" {
        oauth_issuance::active_identity(
            &mut connection,
            user_id,
            tenant_id,
            organization_id,
            &client_id,
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
        "exp": exp_epoch.unwrap_or_default(),
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
    if let Err(resp) = check_oauth_rate(&state, &headers) {
        return Ok(resp);
    }
    let client = authenticate_client(&state, &headers, &form).await?;
    let token = form
        .get("token")
        .ok_or_else(|| OAuthError::invalid_request("缺少 token"))?;
    let mut tx = state.pool.begin().await?;
    let row = sqlx::query_as::<_, (Uuid, String, bool, String, String, Option<Uuid>, Option<Uuid>)>(
        "SELECT id, client_id, revoked, token_type, token_use, tenant_id, organization_id \
         FROM oauth_tokens WHERE token_hash = $1 FOR UPDATE",
    )
    .bind(hash::sha256_hex(token))
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((id, owner_client, revoked, token_type, token_use, tenant_id, org_id)) = row {
        if owner_client == client.client_id && !revoked {
            let changed = sqlx::query("UPDATE oauth_tokens SET revoked = true WHERE id = $1 AND revoked = false")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            if changed.rows_affected() != 1 {
                return Err(OAuthError::server("token revocation did not update one row"));
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
            .await.map_err(OAuthError::server)?;
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
    if let Err(resp) = check_oauth_rate(&state, &headers) {
        return Ok(resp);
    }
    let client = authenticate_client(&state, &headers, &form).await?;
    if !client.has_grant("urn:ietf:params:oauth:grant-type:device_code") {
        return Err(OAuthError::bad_request("unauthorized_client", "该客户端未启用设备码模式"));
    }
    let scope = resolve_scope(form.get("scope"), &client)?;
    let user_code = hash::random_user_code();
    let device_code = hash::random_token(43);
    sqlx::query(
        "INSERT INTO oauth_device_codes \
         (device_code_hash, user_code, client_id, scope, interval_seconds, expires_at) \
         VALUES ($1, $2, $3, $4, 5, now() + interval '600 seconds')",
    )
    .bind(hash::sha256_hex(&device_code))
    .bind(&user_code)
    .bind(&client.client_id)
    .bind(&scope)
    .execute(&state.pool)
    .await?;
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
