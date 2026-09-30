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

use crate::error::AppError;
use crate::extractors::{bearer_token, ApiJson, CurrentUser};
use crate::routes::oauth_admin::{fetch_client, ClientFull};
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
        .route("/api/oauth/consent/{request_id}/decision", post(decide_consent))
        .route("/api/oauth/session-token", post(session_token))
}

// ===== OAuth 风格错误 {"error": "...", "error_description": "..."} =====

pub struct OAuthError {
    status: StatusCode,
    body: Value,
}

impl OAuthError {
    fn new(status: StatusCode, code: &str, desc: &str) -> Self {
        Self {
            status,
            body: json!({"error": code, "error_description": desc}),
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
    pub fn server(e: impl std::fmt::Display) -> Self {
        tracing::error!("OAuth 服务器错误: {e}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "server_error", "服务器内部错误")
    }
}

impl IntoResponse for OAuthError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
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

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < b.len() => match (hex_val(b[i + 1]), hex_val(b[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(h * 16 + l);
                    i += 3;
                }
                _ => {
                    out.push(b[i]);
                    i += 1;
                }
            },
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
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
fn parse_basic_auth(headers: &HeaderMap) -> Option<(String, String)> {
    use base64::Engine;
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let b64 = v.strip_prefix("Basic ")?;
    let decoded = base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok()?;
    let s = String::from_utf8(decoded).ok()?;
    let (u, p) = s.split_once(':')?;
    Some((percent_decode(u), percent_decode(p)))
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
    let id: Uuid = request_id.parse().map_err(|_| AppError::not_found("授权请求不存在或已失效"))?;
    let row = load_consent(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("授权请求不存在或已失效"))?;
    if row.consumed || row.expires_at < Utc::now() {
        return Err(AppError::not_found("授权请求不存在或已失效"));
    }
    let marked = sqlx::query(
        "UPDATE oauth_authorization_requests SET consumed = true WHERE id = $1 AND consumed = false",
    )
    .bind(id)
    .execute(&state.pool)
    .await?;
    if marked.rows_affected() == 0 {
        return Err(AppError::not_found("授权请求不存在或已失效"));
    }
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let redirect_to = if body.approve {
        let code = hash::random_token(43);
        sqlx::query(
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
        .execute(&state.pool)
        .await?;
        audit::write(&state.pool, audit::AuditInfo {
            user_id: Some(user.user_id), username: &user.username,
            tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
            action: "CONSENT_APPROVE", detail: &format!("批准授权: 客户端 {} scope={}", row.client_id, row.scope), ip: &ip, user_agent: &ua,
        }).await;
        append_query(&row.redirect_uri, &[("code", code), ("state", row.state.clone())])
    } else {
        audit::write(&state.pool, audit::AuditInfo {
            user_id: Some(user.user_id), username: &user.username,
            tenant_id: Some(user.tenant_id), organization_id: Some(user.org_id),
            action: "CONSENT_DENY", detail: &format!("拒绝授权: 客户端 {}", row.client_id), ip: &ip, user_agent: &ua,
        }).await;
        append_query(&row.redirect_uri, &[("error", "access_denied".into()), ("state", row.state.clone())])
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
/// 取第一个关联客户端 → 应用或客户端不存在 404 → 复用 issue_tokens 签发 access token
/// (JWT, token_hash 落库, TTL = 客户端 access_token_validity, 关联 user_id/client_id)
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
    let app_id: Uuid = sqlx::query_scalar("SELECT id FROM applications WHERE code = $1")
        .bind(app_code)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found("应用不存在"))?;
    let shell_client_id: String =
        sqlx::query_scalar("SELECT client_id FROM oauth_clients WHERE application_id = $1 ORDER BY created_at LIMIT 1")
            .bind(app_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| AppError::not_found("该应用未关联 OAuth 客户端"))?;
    let client = fetch_client(&state.pool, &shell_client_id)
        .await?
        .ok_or_else(|| AppError::not_found("该应用未关联 OAuth 客户端"))?;
    if client.status != "ENABLED" {
        return Err(AppError::forbidden("客户端已禁用"));
    }
    let ip = audit::client_ip(&headers);
    let ua = audit::ua_of(&headers);
    let scope = resolve_scope(None, &client).map_err(oauth_error_into)?;
    let issued = issue_tokens(
        &state,
        &client,
        Some(user.user_id),
        &user.username,
        Some(user.tenant_id),
        Some(user.org_id),
        &scope,
        &ip,
        &ua,
    )
    .await
    .map_err(oauth_error_into)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "accessToken": issued.access_token,
            "expiresIn": issued.expires_in,
            "tokenType": "Bearer",
        })),
    )
        .into_response())
}

// ===== 令牌签发 =====

struct IssuedToken {
    response: Value,
    /// 明文 access token (JWT) — 仅供响应返回, 不落库不写日志
    access_token: String,
    /// access token 有效期 (秒)
    expires_in: i64,
}

/// 签发 access (+refresh) 并写库与审计
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
    let now = Utc::now().timestamp();
    let access_ttl = client.access_token_validity;
    let claims = jwt::Claims {
        sub: user_id
            .map(|u| u.to_string())
            .unwrap_or_else(|| client.client_id.clone()),
        typ: "access".into(),
        tenant: tenant_id.map(|t| t.to_string()),
        org: org_id.map(|o| o.to_string()),
        cid: Some(client.client_id.clone()),
        scope: Some(scope.to_string()),
        exp: now + access_ttl,
        iat: now,
    };
    let access = jwt::sign(&claims, &state.config.jwt_secret).map_err(OAuthError::server)?;
    // 同一秒内重复请求会得到完全相同的 JWT (确定性签名 + 秒级 iat/exp),
    // 唯一冲突时改为刷新既有行的过期时间 (返回的令牌串本就相同, 幂等)
    sqlx::query(
        "INSERT INTO oauth_tokens (token_hash, token_type, jwt, user_id, client_id, tenant_id, organization_id, scope, expires_at) \
         VALUES ($1, 'access', $2, $3, $4, $5, $6, $7, now() + make_interval(secs => $8::float8)) \
         ON CONFLICT (token_hash) DO UPDATE SET expires_at = EXCLUDED.expires_at",
    )
    .bind(hash::sha256_hex(&access))
    .bind(&access)
    .bind(user_id)
    .bind(&client.client_id)
    .bind(tenant_id)
    .bind(org_id)
    .bind(scope)
    .bind(access_ttl)
    .execute(&state.pool)
    .await
    .map_err(OAuthError::server)?;

    let mut response = json!({
        "access_token": access,
        "token_type": "Bearer",
        "expires_in": access_ttl,
        "scope": scope,
    });
    if client.has_grant("refresh_token") {
        let refresh = hash::random_token(43);
        sqlx::query(
            "INSERT INTO oauth_tokens (token_hash, token_type, jwt, user_id, client_id, tenant_id, organization_id, scope, expires_at) \
             VALUES ($1, 'refresh', NULL, $2, $3, $4, $5, $6, now() + make_interval(secs => $7::float8))",
        )
        .bind(hash::sha256_hex(&refresh))
        .bind(user_id)
        .bind(&client.client_id)
        .bind(tenant_id)
        .bind(org_id)
        .bind(scope)
        .bind(client.refresh_token_validity)
        .execute(&state.pool)
        .await
        .map_err(OAuthError::server)?;
        response["refresh_token"] = json!(refresh);
    }
    audit::write(&state.pool, audit::AuditInfo {
        user_id,
        username: if username.is_empty() { &client.client_id } else { username },
        tenant_id,
        organization_id: org_id,
        action: "TOKEN_ISSUED",
        detail: &format!("签发令牌: 客户端 {} scope={}", client.client_id, scope),
        ip,
        user_agent: ua,
    })
    .await;
    Ok(IssuedToken {
        response,
        access_token: access,
        expires_in: access_ttl,
    })
}

async fn username_of(pool: &sqlx::PgPool, user_id: Option<Uuid>) -> String {
    match user_id {
        None => String::new(),
        Some(uid) => {
            sqlx::query_scalar::<_, String>("SELECT username FROM users WHERE id = $1")
                .bind(uid)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten()
                .unwrap_or_default()
        }
    }
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
    let client = authenticate_client(&state, &headers, &form).await?;
    let grant = form.get("grant_type").cloned().unwrap_or_default();
    if grant.is_empty() {
        return Err(OAuthError::invalid_request("缺少 grant_type"));
    }
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
    Ok(Json(result.response).into_response())
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
    let code = form.get("code").ok_or_else(|| OAuthError::invalid_request("缺少 code"))?;
    let redirect_uri = form
        .get("redirect_uri")
        .ok_or_else(|| OAuthError::invalid_request("缺少 redirect_uri"))?;
    let row = sqlx::query_as::<_, AuthCodeRow>(
        "SELECT id, client_id, user_id, tenant_id, organization_id, redirect_uri, scope, \
         COALESCE(code_challenge,'') AS code_challenge, COALESCE(code_challenge_method,'') AS code_challenge_method, \
         expires_at, consumed FROM oauth_authorization_codes WHERE code_hash = $1",
    )
    .bind(hash::sha256_hex(code))
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| OAuthError::invalid_grant("授权码无效"))?;
    if row.consumed {
        return Err(OAuthError::invalid_grant("授权码已被使用"));
    }
    if row.expires_at < Utc::now() {
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
        "UPDATE oauth_authorization_codes SET consumed = true WHERE id = $1 AND consumed = false",
    )
    .bind(row.id)
    .execute(&state.pool)
    .await?;
    if marked.rows_affected() == 0 {
        return Err(OAuthError::invalid_grant("授权码已被使用"));
    }
    let username = username_of(&state.pool, Some(row.user_id)).await;
    issue_tokens(state, client, Some(row.user_id), &username, row.tenant_id, row.organization_id, &row.scope, ip, ua).await
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
    let row = sqlx::query_as::<_, RefreshRow>(
        "SELECT id, user_id, client_id, tenant_id, organization_id, COALESCE(scope,'') AS scope, \
         expires_at, revoked FROM oauth_tokens WHERE token_hash = $1 AND token_type = 'refresh'",
    )
    .bind(hash::sha256_hex(rt))
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| OAuthError::invalid_grant("刷新令牌无效"))?;
    if row.client_id != client.client_id {
        return Err(OAuthError::invalid_grant("刷新令牌与客户端不匹配"));
    }
    if row.revoked {
        return Err(OAuthError::invalid_grant("刷新令牌已被吊销"));
    }
    if row.expires_at < Utc::now() {
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
                    return Err(OAuthError::bad_request("invalid_scope", "请求的 scope 超出客户端注册范围"));
                }
            }
            s.split_whitespace().collect::<Vec<_>>().join(" ")
        }
    };
    let original: Vec<&str> = row.scope.split_whitespace().collect();
    for p in scope.split_whitespace() {
        if !original.iter().any(|o| *o == p) {
            return Err(OAuthError::bad_request("invalid_scope", "scope 超出原令牌范围"));
        }
    }
    let final_scope = if scope.is_empty() { row.scope.clone() } else { scope };
    // 轮换: 旧 refresh 吊销
    sqlx::query("UPDATE oauth_tokens SET revoked = true WHERE id = $1")
        .bind(row.id)
        .execute(&state.pool)
        .await?;
    let username = username_of(&state.pool, row.user_id).await;
    issue_tokens(state, client, row.user_id, &username, row.tenant_id, row.organization_id, &final_scope, ip, ua).await
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
    let username = form.get("username").ok_or_else(|| OAuthError::invalid_request("缺少 username"))?;
    let password = form.get("password").ok_or_else(|| OAuthError::invalid_request("缺少 password"))?;
    let row = sqlx::query_as::<_, (Uuid, String, String, bool)>(
        "SELECT id, password_hash, status, is_super_admin FROM users WHERE username = $1",
    )
    .bind(username)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| OAuthError::invalid_grant("用户名或密码错误"))?;
    let (user_id, password_hash, status, _) = row;
    if status != "ENABLED" {
        return Err(OAuthError::invalid_grant("用户已被禁用"));
    }
    if !crate::util::password::verify_password(&password_hash, password) {
        return Err(OAuthError::invalid_grant("用户名或密码错误"));
    }
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
    issue_tokens(state, client, Some(user_id), username, Some(ctx.0), Some(ctx.1), &scope, ip, ua).await
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
    let row = sqlx::query_as::<_, DeviceRow>(
        "SELECT id, client_id, COALESCE(scope,'') AS scope, status, user_id, tenant_id, organization_id, \
         interval_seconds, last_polled_at, expires_at FROM oauth_device_codes WHERE device_code_hash = $1",
    )
    .bind(hash::sha256_hex(device_code))
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| OAuthError::invalid_grant("设备码无效"))?;
    if row.client_id != client.client_id {
        return Err(OAuthError::invalid_grant("设备码与客户端不匹配"));
    }
    let now = Utc::now();
    let mut status = row.status.clone();
    if status == "PENDING" && now > row.expires_at {
        sqlx::query("UPDATE oauth_device_codes SET status = 'EXPIRED' WHERE id = $1")
            .bind(row.id)
            .execute(&state.pool)
            .await?;
        status = "EXPIRED".into();
    }
    // 记录轮询时间 (每次)
    sqlx::query("UPDATE oauth_device_codes SET last_polled_at = now() WHERE id = $1")
        .bind(row.id)
        .execute(&state.pool)
        .await?;
    if status == "EXPIRED" {
        return Err(OAuthError::bad_request("expired_token", "设备码已过期"));
    }
    if let Some(last) = row.last_polled_at {
        if (now - last).num_seconds() < row.interval_seconds {
            return Err(OAuthError::bad_request("slow_down", "轮询过于频繁, 请稍后再试"));
        }
    }
    match status.as_str() {
        "DENIED" => Err(OAuthError::bad_request("access_denied", "用户拒绝了该设备授权")),
        "PENDING" => Err(OAuthError::bad_request("authorization_pending", "设备授权尚未完成")),
        "APPROVED" => {
            // 标记已消费, 防止重复签发
            let marked = sqlx::query(
                "UPDATE oauth_device_codes SET status = 'EXPIRED' WHERE id = $1 AND status = 'APPROVED'",
            )
            .bind(row.id)
            .execute(&state.pool)
            .await?;
            if marked.rows_affected() == 0 {
                return Err(OAuthError::bad_request("expired_token", "设备码已使用"));
            }
            let username = username_of(&state.pool, row.user_id).await;
            issue_tokens(state, client, row.user_id, &username, row.tenant_id, row.organization_id, &row.scope, ip, ua).await
        }
        _ => Err(OAuthError::invalid_grant("设备码状态异常")),
    }
}

// ===== 用户信息 =====

/// GET /oauth/userinfo — Bearer access token
async fn userinfo(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, OAuthError> {
    let token = bearer_token(&headers).map_err(|_| OAuthError::invalid_token("缺少访问令牌"))?;
    let claims = jwt::verify(&token, &state.config.jwt_secret).map_err(|_| OAuthError::invalid_token("令牌无效或已过期"))?;
    if claims.typ != "access" {
        return Err(OAuthError::invalid_token("令牌类型错误"));
    }
    let row = sqlx::query_as::<_, (Option<Uuid>, Option<Uuid>, Option<Uuid>, String, bool, DateTime<Utc>)>(
        "SELECT user_id, tenant_id, organization_id, COALESCE(scope,'') AS scope, revoked, expires_at \
         FROM oauth_tokens WHERE token_hash = $1 AND token_type = 'access'",
    )
    .bind(hash::sha256_hex(&token))
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| OAuthError::invalid_token("令牌无效或已过期"))?;
    let (user_id, tenant_id, org_id, scope, revoked, expires_at) = row;
    if revoked || expires_at < Utc::now() {
        return Err(OAuthError::invalid_token("令牌已被吊销或已过期"));
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
        Some(t) => sqlx::query_as::<_, (Uuid, String)>("SELECT id, name FROM tenants WHERE id = $1")
            .bind(t)
            .fetch_optional(&state.pool)
            .await?
            .map(|(id, name)| json!({"id": id, "name": name}))
            .unwrap_or(Value::Null),
        None => Value::Null,
    };
    let org_json = match org_id {
        Some(o) => sqlx::query_as::<_, (Uuid, String)>("SELECT id, name FROM organizations WHERE id = $1")
            .bind(o)
            .fetch_optional(&state.pool)
            .await?
            .map(|(id, name)| json!({"id": id, "name": name}))
            .unwrap_or(Value::Null),
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
    if let Err(resp) = check_oauth_rate(&state, &headers) {
        return Ok(resp);
    }
    let _client = authenticate_client(&state, &headers, &form).await?;
    let token = form.get("token").ok_or_else(|| OAuthError::invalid_request("缺少 token"))?;
    let row = sqlx::query_as::<_, (Option<Uuid>, String, String, DateTime<Utc>, DateTime<Utc>, bool, Option<i64>)>(
        "SELECT user_id, client_id, COALESCE(scope,'') AS scope, created_at, expires_at, revoked, \
         EXTRACT(EPOCH FROM expires_at)::bigint AS exp_epoch \
         FROM oauth_tokens WHERE token_hash = $1",
    )
    .bind(hash::sha256_hex(token))
    .fetch_optional(&state.pool)
    .await?;
    let Some((user_id, client_id, scope, created_at, expires_at, revoked, exp_epoch)) = row else {
        return Ok(Json(json!({"active": false})).into_response());
    };
    let active = !revoked && expires_at > Utc::now();
    if !active {
        return Ok(Json(json!({"active": false})).into_response());
    }
    let username = username_of(&state.pool, user_id).await;
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
    let token = form.get("token").ok_or_else(|| OAuthError::invalid_request("缺少 token"))?;
    let row = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id, client_id FROM oauth_tokens WHERE token_hash = $1",
    )
    .bind(hash::sha256_hex(token))
    .fetch_optional(&state.pool)
    .await?;
    if let Some((id, owner_client)) = row {
        if owner_client == client.client_id {
            sqlx::query("UPDATE oauth_tokens SET revoked = true WHERE id = $1")
                .bind(id)
                .execute(&state.pool)
                .await?;
            let ip = audit::client_ip(&headers);
            let ua = audit::ua_of(&headers);
            audit::write(&state.pool, audit::AuditInfo {
                user_id: None, username: client.client_id.as_str(), tenant_id: None, organization_id: None,
                action: "TOKEN_REVOKED", detail: "吊销令牌 (RFC7009)", ip: &ip, user_agent: &ua,
            }).await;
        }
    }
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
