//! Cookie and CSRF boundaries around the platform relying-party use cases.

use super::{context, protocol, store, transport::Chain};
use crate::{error::AppError, extractors::ApiJson, state::AppState, util::audit};
use axum::{
    extract::{RawQuery, Request, State},
    http::{header, HeaderMap, HeaderValue},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/config", get(configuration))
        .route("/auth/login", get(login))
        .route("/auth/callback", get(callback))
        .route("/auth/session", get(session))
        .route("/auth/context-token", post(context_token))
        .route("/auth/logout", post(logout))
        .route_layer(middleware::from_fn(protected_headers))
}
async fn protected_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    for (name, value) in [
        ("cache-control", "no-store"),
        ("pragma", "no-cache"),
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    response
        .headers_mut()
        .append(header::VARY, HeaderValue::from_static("Origin, Cookie"));
    response
}
async fn configuration(State(state): State<AppState>) -> Json<Value> {
    Json(json!({"mode":if state.config.bff_enabled{"BFF"}else{"LEGACY"}}))
}

fn bad() -> AppError {
    AppError::bad_request("平台登录请求参数无效")
}
fn parameters(raw: Option<String>) -> Result<std::collections::BTreeMap<String, String>, AppError> {
    let raw = raw.unwrap_or_default();
    if raw.len() > 16384 {
        return Err(bad());
    }
    let mut values = std::collections::BTreeMap::new();
    for pair in raw.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let decode = |raw: &str| -> Result<String, AppError> {
            let bytes = raw.as_bytes();
            let mut index = 0;
            while index < bytes.len() {
                if bytes[index] == b'%' {
                    if index + 2 >= bytes.len()
                        || !bytes[index + 1].is_ascii_hexdigit()
                        || !bytes[index + 2].is_ascii_hexdigit()
                    {
                        return Err(bad());
                    }
                    index += 3;
                } else {
                    index += 1;
                }
            }
            let value = percent_encoding::percent_decode_str(&raw.replace('+', " "))
                .decode_utf8()
                .map_err(|_| bad())?
                .into_owned();
            if value.chars().any(char::is_control) {
                return Err(bad());
            }
            Ok(value)
        };
        let key = decode(key)?;
        let value = decode(value)?;
        if key.is_empty()
            || key.len() > 80
            || value.len() > 4096
            || values.len() >= 16
            || values.insert(key, value).is_some()
        {
            return Err(bad());
        }
    }
    Ok(values)
}
fn navigation(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    super::enabled(state)?;
    if headers.contains_key(header::AUTHORIZATION)
        || !super::unique(headers, "host")?.is_some_and(|host| {
            host.eq_ignore_ascii_case(
                state
                    .config
                    .public_app_url
                    .strip_prefix("https://")
                    .unwrap_or(""),
            )
        })
    {
        return Err(AppError::forbidden("平台登录主机无效"));
    }
    Ok(())
}
fn return_path(raw: &str) -> Result<String, AppError> {
    if raw.is_empty() || raw.len() > 2048 {
        return Err(bad());
    }
    let mut decoded = raw.to_owned();
    for _ in 0..8 {
        if !decoded.starts_with('/')
            || decoded.starts_with("//")
            || decoded.contains('\\')
            || decoded.chars().any(char::is_control)
        {
            return Err(bad());
        }
        let url =
            reqwest::Url::parse(&format!("https://return.invalid{decoded}")).map_err(|_| bad())?;
        if url.host_str() != Some("return.invalid")
            || url.fragment().is_some()
            || url.path() == "/auth"
            || url.path().starts_with("/auth/")
            || url.path() == "/login"
            || url.path().starts_with("/login/")
            || url.path() == "/sso/login"
            || url.path().starts_with("/sso/login/")
        {
            return Err(bad());
        }
        let next = percent_encoding::percent_decode_str(&decoded)
            .decode_utf8()
            .map_err(|_| bad())?
            .into_owned();
        if next == decoded {
            return Ok(raw.to_owned());
        }
        decoded = next;
    }
    Err(bad())
}
fn with_cookie(
    mut response: Response,
    name: &str,
    value: &str,
    ttl: i64,
) -> Result<Response, AppError> {
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&super::cookie(name, value, ttl))
            .map_err(|_| AppError::Unavailable)?,
    );
    Ok(response)
}

async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response, AppError> {
    navigation(&state, &headers)?;
    let parameters = parameters(raw)?;
    if parameters.keys().any(|name| name != "return") {
        return Err(bad());
    }
    let return_path = return_path(parameters.get("return").map(String::as_str).unwrap_or("/"))?;
    if let Some(retry) = state.login_limits.admit_source(&audit::client_ip(&headers)) {
        return Err(AppError::RateLimited(retry));
    }
    let chain = Chain::new(&state)?;
    protocol::discovery(&chain).await?;
    let binder = super::cookie_value(&headers, super::LOGIN_COOKIE)?
        .map(str::to_owned)
        .unwrap_or(super::secret()?);
    let cookie_hash = super::digest(&binder);
    let state_secret = super::secret()?;
    let nonce = super::secret()?;
    let verifier = super::secret()?;
    let flow_id = crate::util::uuid7::new_uuid();
    let client = store::client(&state)?;
    let issuer = store::issuer(&state)?;
    let encrypted =
        store::keys(&state)?.seal(flow_id, issuer, client, "login-verifier", &verifier)?;
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| AppError::Unavailable)?;
    let browser = if let Some(browser) = store::browser(&mut *tx, &cookie_hash, true).await? {
        browser
    } else {
        let id = crate::util::uuid7::new_uuid();
        // Expired binders are never interpreted as a previous authenticated
        // browser generation. The stable opaque value remains non-authorizing.
        sqlx::query("INSERT INTO console_browsers(id,token_hash,generation,created_at,expires_at) VALUES($1,$2,0,clock_timestamp(),clock_timestamp()+interval '1 day') \
            ON CONFLICT(token_hash) DO UPDATE SET generation=console_browsers.generation+1,current_session_id=NULL,expires_at=clock_timestamp()+interval '1 day'")
            .bind(id).bind(&cookie_hash).execute(&mut *tx).await?;
        store::browser(&mut *tx, &cookie_hash, true)
            .await?
            .ok_or(AppError::Unavailable)?
    };
    sqlx::query("UPDATE console_browsers SET expires_at=GREATEST(expires_at,clock_timestamp()+interval '1 day') WHERE id=$1")
        .bind(browser.id).execute(&mut *tx).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('console-login-capacity-v1',0))")
        .execute(&mut *tx)
        .await
        .map_err(|_| AppError::Unavailable)?;
    sqlx::query("DELETE FROM console_login_flows WHERE id IN (SELECT id FROM console_login_flows WHERE consumed_at IS NOT NULL OR expires_at<=clock_timestamp() ORDER BY expires_at LIMIT 1024)")
        .execute(&mut *tx).await?;
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM console_login_flows WHERE consumed_at IS NULL AND expires_at>clock_timestamp()")
        .fetch_one(&mut *tx).await.map_err(|_|AppError::Unavailable)?;
    if count >= state.config.login_max_entries as i64 {
        return Err(AppError::RateLimited(1));
    }
    sqlx::query("INSERT INTO console_login_flows(id,browser_id,browser_generation,state_hash,nonce_hash,verifier_ciphertext,return_path,created_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,clock_timestamp(),clock_timestamp()+$8*interval '1 second')")
        .bind(flow_id).bind(browser.id).bind(browser.generation).bind(super::digest(&state_secret)).bind(super::digest(&nonce))
        .bind(encrypted).bind(&return_path).bind(state.config.bff_login_ttl_seconds as f64).execute(&mut *tx).await?;
    tx.commit().await?;
    let mut target = reqwest::Url::parse(&format!("{issuer}/oidc/authorize"))
        .map_err(|_| AppError::Unavailable)?;
    target
        .query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &client.to_string())
        .append_pair(
            "redirect_uri",
            state
                .config
                .bff_oidc_redirect_uri
                .as_deref()
                .ok_or(AppError::Unavailable)?,
        )
        .append_pair("scope", "openid profile offline_access")
        .append_pair("state", &state_secret)
        .append_pair("nonce", &nonce)
        .append_pair("code_challenge_method", "S256")
        .append_pair(
            "code_challenge",
            &URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        );
    with_cookie(
        Redirect::to(target.as_str()).into_response(),
        super::LOGIN_COOKIE,
        &binder,
        86400,
    )
}

async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response, AppError> {
    navigation(&state, &headers)?;
    let values = parameters(raw)?;
    let state_value = values
        .get("state")
        .filter(|value| super::canonical_secret(value))
        .ok_or_else(bad)?;
    if values.get("iss").map(String::as_str) != Some(store::issuer(&state)?) {
        return Err(AppError::unauthorized("OIDC callback issuer 不匹配"));
    }
    let has_error = values.contains_key("error");
    if has_error == values.contains_key("code") {
        return Err(bad());
    }
    let code = values
        .get("code")
        .filter(|code| !code.is_empty() && code.len() <= 2048);
    if !has_error && code.is_none() {
        return Err(bad());
    }
    let binder = super::cookie_value(&headers, super::LOGIN_COOKIE)?
        .ok_or_else(|| AppError::unauthorized("登录请求不属于当前浏览器"))?;
    let hash = super::digest(binder);
    let initial: store::Flow =
        sqlx::query_as("SELECT * FROM console_login_flows WHERE state_hash=$1")
            .bind(super::digest(state_value))
            .fetch_optional(&state.pool)
            .await
            .map_err(|_| AppError::Unavailable)?
            .ok_or_else(|| AppError::unauthorized("登录请求不存在或已失效"))?;
    let mut claim = state
        .pool
        .begin()
        .await
        .map_err(|_| AppError::Unavailable)?;
    let browser = store::browser(&mut *claim, &hash, true)
        .await?
        .ok_or_else(|| AppError::unauthorized("登录浏览器绑定已失效"))?;
    if browser.id != initial.browser_id {
        return Err(AppError::unauthorized("登录请求不属于当前浏览器"));
    }
    let flow: store::Flow =
        sqlx::query_as("SELECT * FROM console_login_flows WHERE id=$1 FOR UPDATE")
            .bind(initial.id)
            .fetch_one(&mut *claim)
            .await
            .map_err(|_| AppError::Unavailable)?;
    if flow.consumed_at.is_some() {
        return Err(AppError::unauthorized("登录 callback 已消费"));
    }
    if browser.generation != flow.browser_generation {
        return Err(AppError::conflict(
            "其他标签页已完成登录，请重新加载平台会话",
        ));
    }
    store::deadline(&mut claim, flow.expires_at).await?;
    sqlx::query("UPDATE console_login_flows SET consumed_at=clock_timestamp() WHERE id=$1")
        .bind(flow.id)
        .execute(&mut *claim)
        .await?;
    claim.commit().await?;
    if has_error {
        return Ok(Redirect::to("/login?authenticationError=1").into_response());
    }
    let client = store::client(&state)?;
    let issuer = store::issuer(&state)?;
    let verifier = store::keys(&state)?.open(
        flow.id,
        issuer,
        client,
        "login-verifier",
        &flow.verifier_ciphertext,
    )?;
    let chain = Chain::new(&state)?;
    let verified =
        protocol::exchange(&chain, code.ok_or_else(bad)?, &verifier, &flow.nonce_hash).await?;
    let cookie = super::secret()?;
    let csrf = super::csrf(&cookie);
    let id = crate::util::uuid7::new_uuid();
    let access = store::keys(&state)?.seal(id, issuer, client, "oidc-access", &verified.access)?;
    let refresh =
        store::keys(&state)?.seal(id, issuer, client, "oidc-refresh", &verified.refresh)?;
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| AppError::Unavailable)?;
    store::lock_user(&mut tx, verified.proof.user_id).await?;
    store::lock_client(&mut tx, client, issuer).await?;
    store::proof(&mut tx, &verified.proof, issuer).await?;
    let current = store::browser(&mut *tx, &hash, true)
        .await?
        .ok_or_else(|| AppError::unauthorized("登录浏览器绑定已失效"))?;
    if current.id != flow.browser_id || current.generation != flow.browser_generation {
        return Err(AppError::conflict(
            "其他登录已完成，本次 callback 不能覆盖当前账号",
        ));
    }
    if let Some(previous) = current.current_session_id {
        store::revoke(&mut tx, previous).await?;
    }
    let expires = verified.absolute_expires_at;
    sqlx::query("INSERT INTO bff_sessions(id,browser_id,token_hash,csrf_hash,user_id,account_version,global_session_id,oidc_client_id,oidc_grant_id,oidc_client_policy_version,issuer,auth_time,nonce_hash,access_ciphertext,refresh_ciphertext,access_expires_at,version,created_at,expires_at) \
        VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,1,clock_timestamp(),$17)")
        .bind(id).bind(current.id).bind(super::digest(&cookie)).bind(super::digest(&csrf)).bind(verified.proof.user_id)
        .bind(verified.proof.account_version).bind(verified.proof.global_session_id).bind(client).bind(verified.proof.grant_id)
        .bind(verified.proof.client_policy_version).bind(issuer).bind(verified.proof.auth_time).bind(&flow.nonce_hash)
        .bind(access).bind(refresh).bind(verified.proof.expires_at).bind(expires).execute(&mut *tx).await?;
    sqlx::query("UPDATE console_browsers SET generation=generation+1,current_session_id=$2,expires_at=GREATEST(expires_at,$3) WHERE id=$1")
        .bind(current.id).bind(id).bind(expires).execute(&mut *tx).await?;
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id=$1")
        .bind(verified.proof.user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| AppError::Unavailable)?;
    audit::write_in_tx(
        &mut tx,
        audit::AuditInfo {
            user_id: Some(verified.proof.user_id),
            username: &username,
            tenant_id: None,
            organization_id: None,
            action: "BFF_LOGIN",
            detail: &json!({"bffSessionId":id,"clientId":client,"source":"OIDC"}).to_string(),
            ip: &audit::client_ip(&headers),
            user_agent: &audit::ua_of(&headers),
        },
    )
    .await?;
    store::proof(&mut tx, &verified.proof, issuer).await?;
    store::deadline(
        &mut tx,
        verified
            .proof
            .expires_at
            .min(verified.id_expires_at)
            .min(expires)
            .min(flow.expires_at),
    )
    .await?;
    tx.commit().await?;
    with_cookie(
        Redirect::to(&flow.return_path).into_response(),
        super::SESSION_COOKIE,
        &cookie,
        (expires - Utc::now()).num_seconds(),
    )
}

fn current_cookie(headers: &HeaderMap) -> Result<&str, AppError> {
    super::cookie_value(headers, super::SESSION_COOKIE)?
        .ok_or_else(|| AppError::unauthorized("平台尚未登录"))
}
fn csrf_proof(headers: &HeaderMap, cookie: &str, session: &store::Session) -> Result<(), AppError> {
    let csrf = super::unique(headers, "x-csrf-token")?
        .ok_or_else(|| AppError::forbidden("缺少平台 CSRF 证明"))?;
    if !super::canonical_secret(csrf)
        || !super::equal(csrf, &super::csrf(cookie))
        || !super::equal(&super::digest(csrf), &session.csrf_hash)
    {
        return Err(AppError::forbidden("平台 CSRF 证明无效"));
    }
    Ok(())
}
async fn session(
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response, AppError> {
    super::origin(&state, &headers, true)?;
    if !parameters(raw)?.is_empty() {
        return Err(bad());
    }
    let cookie = current_cookie(&headers)?;
    let original = store::session_cookie(&state, cookie).await?;
    if original.logout_request_id.is_some() {
        // Resume only the durable command previously authorized by a CSRF-
        // checked POST. The current GET has already passed the same-origin
        // read gate; no new scope, token or account is accepted from the URL.
        let scope = original.logout_scope.clone().ok_or(AppError::Unavailable)?;
        let mut recovery_headers = headers.clone();
        recovery_headers.insert(
            header::ORIGIN,
            HeaderValue::from_str(&state.config.public_app_url)
                .map_err(|_| AppError::Unavailable)?,
        );
        recovery_headers.insert(
            "x-csrf-token",
            HeaderValue::from_str(&super::csrf(cookie)).map_err(|_| AppError::Unavailable)?,
        );
        logout(
            State(state.clone()),
            recovery_headers,
            RawQuery(None),
            ApiJson(Logout {
                expected_version: original.version.to_string(),
                scope,
            }),
        )
        .await?;
        return Err(AppError::unauthorized("平台注销已确认，请重新登录"));
    }
    let chain = Chain::new(&state)?;
    let checked = store::online(&chain, original, true).await?;
    let mut view = context::view(&state, checked).await?;
    view["csrfToken"] = json!(super::csrf(cookie));
    Ok(Json(view).into_response())
}
async fn context_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    ApiJson(body): ApiJson<context::Selection>,
) -> Result<Json<Value>, AppError> {
    super::origin(&state, &headers, false)?;
    if !parameters(raw)?.is_empty() {
        return Err(bad());
    }
    let cookie = current_cookie(&headers)?;
    let original = store::session_cookie(&state, cookie).await?;
    csrf_proof(&headers, cookie, &original)?;
    let chain = Chain::new(&state)?;
    let checked = store::online(&chain, original, true).await?;
    Ok(Json(context::token(&state, &headers, checked, body).await?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Logout {
    expected_version: String,
    scope: String,
}

async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    ApiJson(body): ApiJson<Logout>,
) -> Result<Response, AppError> {
    super::origin(&state, &headers, false)?;
    if !parameters(raw)?.is_empty() {
        return Err(bad());
    }
    if !matches!(body.scope.as_str(), "PLATFORM" | "GLOBAL") {
        return Err(bad());
    }
    let version = body.expected_version.parse::<i64>().map_err(|_| bad())?;
    if version < 1 || version.to_string() != body.expected_version {
        return Err(bad());
    }
    let cookie = current_cookie(&headers)?;
    let original = store::session_cookie(&state, cookie).await?;
    csrf_proof(&headers, cookie, &original)?;
    if original.version != version {
        return Err(AppError::conflict("平台会话版本已变化"));
    }
    let chain = Chain::new(&state)?;
    let pending = if let Some(request) = original.logout_request_id {
        if original.logout_scope.as_deref() != Some(body.scope.as_str()) {
            return Err(AppError::conflict("已有不同范围的注销操作正在确认"));
        }
        request
    } else {
        let checked = store::online(&chain, original.clone(), true).await?;
        let request = crate::util::uuid7::new_uuid();
        let mut tx = state
            .pool
            .begin()
            .await
            .map_err(|_| AppError::Unavailable)?;
        store::lock_user(&mut tx, checked.user_id).await?;
        store::lock_client(&mut tx, checked.oidc_client_id, &checked.issuer).await?;
        let current = store::session_id(&mut *tx, checked.id, true)
            .await?
            .ok_or_else(|| AppError::unauthorized("平台会话已失效"))?;
        if current.version != version
            || current.logout_request_id.is_some()
            || current.refresh_lease_id.is_some()
            || current.refresh_uncertain
        {
            return Err(AppError::conflict("平台会话已变化，请重新加载"));
        }
        store::session_proof(&mut tx, &checked).await?;
        sqlx::query("UPDATE bff_sessions SET logout_request_id=$2,logout_scope=$3 WHERE id=$1")
            .bind(checked.id)
            .bind(request)
            .bind(&body.scope)
            .execute(&mut *tx)
            .await?;
        store::deadline(&mut tx, checked.expires_at.min(checked.access_expires_at)).await?;
        tx.commit().await?;
        request
    };
    // The pending command is durable before HTTP starts. No user/session row
    // lock crosses the back-channel, including account-wide revocation.
    let current = store::session_id(&state.pool, original.id, false)
        .await?
        .ok_or(AppError::Unavailable)?;
    if current.issuer != store::issuer(&state)? || current.oidc_client_id != store::client(&state)?
    {
        return Err(AppError::unauthorized("原注销认证来源已变化"));
    }
    let refresh = store::keys(&state)?.open(
        current.id,
        &current.issuer,
        current.oidc_client_id,
        "oidc-refresh",
        &current.refresh_ciphertext,
    )?;
    protocol::logout(&chain, &current, &refresh, &body.scope, pending).await?;
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| AppError::Unavailable)?;
    store::lock_user(&mut tx, current.user_id).await?;
    let browser: Option<(Uuid, Option<Uuid>)> =
        sqlx::query_as("SELECT id,current_session_id FROM console_browsers WHERE id=$1 FOR UPDATE")
            .bind(current.browser_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| AppError::Unavailable)?;
    let (_, browser_session) = browser.ok_or(AppError::Unavailable)?;
    if browser_session != Some(current.id) {
        return Err(AppError::conflict(
            "其他标签页已建立新会话，本次注销不能清除其 Cookie",
        ));
    }
    let closing = store::session_id(&mut *tx, current.id, true)
        .await?
        .ok_or(AppError::Unavailable)?;
    if closing.logout_request_id != Some(pending)
        || closing.logout_scope.as_deref() != Some(body.scope.as_str())
        || closing.version != version
    {
        return Err(AppError::conflict("注销会话已变化"));
    }
    store::revoke(&mut tx, closing.id).await?;
    sqlx::query("UPDATE console_browsers SET generation=generation+1,current_session_id=NULL WHERE id=$1 AND current_session_id=$2")
        .bind(closing.browser_id).bind(closing.id).execute(&mut *tx).await?;
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id=$1")
        .bind(closing.user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| AppError::Unavailable)?;
    audit::write_in_tx(
        &mut tx,
        audit::AuditInfo {
            user_id: Some(closing.user_id),
            username: &username,
            tenant_id: None,
            organization_id: None,
            action: "BFF_LOGOUT",
            detail: &json!({"bffSessionId":closing.id,"scope":body.scope,"requestId":pending})
                .to_string(),
            ip: &audit::client_ip(&headers),
            user_agent: &audit::ua_of(&headers),
        },
    )
    .await?;
    tx.commit().await?;
    // A fixed-name expiry may arrive after a newer callback and delete its
    // cookie. This old value is already non-authorizing on the server; login
    // replaces it and its original absolute Max-Age bounds natural retention.
    Ok(Json(json!({"redirectTo":"/login?signedOut=1"})).into_response())
}
