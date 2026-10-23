//! Anonymous static-resource transport for registered URL micro applications.
//! This is deliberately not an API, arbitrary-URL, cookie or WebSocket proxy.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    body::Body,
    extract::{Extension, OriginalUri, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use percent_encoding::percent_decode_str;
use reqwest::{Client, Url};
use tokio::sync::Semaphore;

use crate::{config::Config, error::AppError, releases::FetchPolicy, state::AppState};

const MAX_BODY: usize = 16 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

struct ProxyRuntime {
    slots: Arc<Semaphore>,
    sources: Mutex<HashMap<String, (Instant, u32)>>,
}

impl ProxyRuntime {
    fn new() -> Self {
        Self {
            slots: Arc::new(Semaphore::new(24)),
            sources: Mutex::new(HashMap::new()),
        }
    }

    fn check_rate(&self, source: &str) -> Result<(), AppError> {
        let mut sources = self.sources.lock().map_err(|_| AppError::Unavailable)?;
        let now = Instant::now();
        let window = Duration::from_secs(60);
        if sources.len() >= 4096 && !sources.contains_key(source) {
            sources.retain(|_, (start, _)| now.duration_since(*start) < window);
            if sources.len() >= 4096 {
                return Err(AppError::RateLimited(60));
            }
        }
        let (start, requests) = sources.entry(source.to_owned()).or_insert((now, 0));
        if now.duration_since(*start) >= window {
            *start = now;
            *requests = 0;
        }
        if *requests >= 600 {
            return Err(AppError::RateLimited(60));
        }
        *requests += 1;
        Ok(())
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/micro-proxy",
            get(|| async { static_error(StatusCode::NOT_FOUND) }),
        )
        .route(
            "/micro-proxy/",
            get(|| async { static_error(StatusCode::NOT_FOUND) }),
        )
        .route(
            "/micro-proxy/{app_code}",
            get(|| async { static_error(StatusCode::NOT_FOUND) }),
        )
        .route(
            "/micro-proxy/{app_code}/",
            get(|| async { static_error(StatusCode::NOT_FOUND) }),
        )
        .route("/micro-proxy/{app_code}/{revision}", get(proxy))
        .route("/micro-proxy/{app_code}/{revision}/", get(proxy))
        .route("/micro-proxy/{app_code}/{revision}/{*path}", get(proxy))
        .layer(Extension(Arc::new(ProxyRuntime::new())))
}

async fn proxy(
    State(state): State<AppState>,
    Extension(runtime): Extension<Arc<ProxyRuntime>>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
) -> Response {
    let result = serve(&state, &runtime, &uri, &method, &headers).await;
    let mut response = match result {
        Ok(response) => response,
        Err(error) => error.into_response(),
    };
    static_headers(response.headers_mut());
    response
}

async fn serve(
    state: &AppState,
    runtime: &ProxyRuntime,
    uri: &Uri,
    method: &Method,
    headers: &HeaderMap,
) -> Result<Response, AppError> {
    runtime.check_rate(
        headers
            .get("x-real-ip")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("unknown"),
    )?;
    let _slot = runtime
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| AppError::Unavailable)?;
    let suffix = uri.path().strip_prefix("/micro-proxy/").unwrap_or("");
    let mut parts = suffix.splitn(3, '/');
    let code = parts.next().unwrap_or("");
    let revision = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    if code.is_empty()
        || code.len() > 64
        || !code
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Ok(static_error(StatusCode::NOT_FOUND));
    }
    let entry = crate::application_entries::lookup_proxy_entry(
        &state.pool,
        code,
        &state.config.application_environment,
    )
    .await?
    .ok_or_else(|| AppError::not_found("微应用代理入口不存在或未启用"))?;
    if revision != entry.revision.to_string() {
        return Err(AppError::not_found("微应用入口版本已变更，请重新打开应用"));
    }
    let query = filtered_query(uri.query())?;
    let policy = FetchPolicy::new(&state.config, &state.config.application_environment)?;
    let target = Target::new(&policy, &entry.entry_url, path, query.as_deref())?;
    // Canonical directory URLs preserve relative static-resource resolution.
    if !suffix[code.len() + 1..].contains('/') {
        let location = match uri.query() {
            Some(query) => format!("/micro-proxy/{code}/{revision}/?{query}"),
            None => format!("/micro-proxy/{code}/{revision}/"),
        };
        return Ok((
            StatusCode::PERMANENT_REDIRECT,
            [(header::LOCATION, location)],
        )
            .into_response());
    }
    let client = proxy_client(&state.config)?;
    match tokio::time::timeout(
        REQUEST_TIMEOUT,
        fetch_static(&client, &policy, target, method),
    )
    .await
    {
        Ok(response) => Ok(response),
        Err(_) => Ok(static_error(StatusCode::GATEWAY_TIMEOUT)),
    }
}

fn proxy_client(config: &Config) -> Result<Client, AppError> {
    // Explicitly disable system HTTP proxies as well as automatic redirects. No
    // incoming headers, including cookies, auth, Origin or platform IDs, are copied.
    let mut client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(2))
        .timeout(REQUEST_TIMEOUT)
        .user_agent("SaaSPlatform/1.0 micro-static-proxy");
    for certificate in &config.micro_ca_certificates {
        client = client.add_root_certificate(certificate.clone());
    }
    client.build().map_err(|_| AppError::Unavailable)
}

fn filtered_query(query: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(query) = query else { return Ok(None) };
    if query.len() > 2048 || query.chars().any(char::is_control) {
        return Err(AppError::bad_request("微应用资源 query 过长或包含控制字符"));
    }
    let mut kept = Vec::new();
    for pair in query.split('&') {
        let (name, _) = pair.split_once('=').unwrap_or((pair, ""));
        let name = percent_decode_str(name)
            .decode_utf8()
            .map_err(|_| AppError::bad_request("query 编码无效"))?;
        if !matches!(
            name.as_ref(),
            "__platform_entry" | "__saas_execution" | "__saas_lease"
        ) && !pair.is_empty()
        {
            kept.push(pair);
        }
    }
    Ok((!kept.is_empty()).then(|| kept.join("&")))
}

struct Target {
    url: Url,
    root: Url,
    extension: Option<String>,
}

impl Target {
    fn new(
        policy: &FetchPolicy,
        entry: &str,
        path: &str,
        query: Option<&str>,
    ) -> Result<Self, AppError> {
        let entry = policy.resolve(entry, None)?;
        let root = entry
            .join(".")
            .map_err(|_| AppError::bad_request("入口目录无效"))?;
        validate_path(root.path().trim_start_matches('/'))?;
        validate_path(path)?;
        let extension = resource_extension(if path.is_empty() { entry.path() } else { path })?;
        let mut url = if path.is_empty() {
            entry
        } else {
            root.join(path)
                .map_err(|_| AppError::bad_request("微应用资源路径无效"))?
        };
        if let Some(query) = query {
            url.set_query(Some(query));
        }
        // Query from the configured HTML entry never leaks onto unrelated assets.
        if !path.is_empty() && query.is_none() {
            url.set_query(None);
        }
        policy.check(&url)?;
        ensure_scope(&url, &root)?;
        Ok(Self {
            url,
            root,
            extension,
        })
    }
}

fn validate_path(path: &str) -> Result<(), AppError> {
    if path.len() > 2000 || path.starts_with('/') || path.contains('\\') {
        return Err(AppError::bad_request("微应用资源路径无效"));
    }
    for segment in path
        .trim_end_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
    {
        let segment = percent_decode_str(segment)
            .decode_utf8()
            .map_err(|_| AppError::bad_request("微应用资源路径编码无效"))?;
        if segment.starts_with('.')
            || segment.contains(['/', '\\', ':', '%'])
            || segment.chars().any(char::is_control)
        {
            return Err(AppError::bad_request(
                "微应用资源路径不能跨目录或访问隐藏文件",
            ));
        }
        if matches!(
            segment.to_ascii_lowercase().as_str(),
            "api"
                | "oauth"
                | "internal"
                | "metrics"
                | "socket.io"
                | "node_modules"
                | "staging"
                | "incoming"
                | "uploads"
                | "backups"
                | "tmp"
                | "release.json"
        ) {
            return Err(AppError::bad_request("微应用代理仅支持公开页面与静态资源"));
        }
    }
    Ok(())
}

fn resource_extension(path: &str) -> Result<Option<String>, AppError> {
    let last = path.rsplit('/').next().unwrap_or("");
    let last = percent_decode_str(last)
        .decode_utf8()
        .map_err(|_| AppError::bad_request("资源名称编码无效"))?;
    let Some((_, extension)) = last.rsplit_once('.') else {
        return Ok(None);
    };
    let extension = extension.to_ascii_lowercase();
    if !matches!(
        extension.as_str(),
        "html"
            | "htm"
            | "js"
            | "mjs"
            | "cjs"
            | "css"
            | "json"
            | "map"
            | "webmanifest"
            | "wasm"
            | "svg"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "avif"
            | "ico"
            | "woff"
            | "woff2"
            | "ttf"
            | "otf"
            | "eot"
            | "txt"
    ) {
        return Err(AppError::not_found("不支持的微应用静态资源类型"));
    }
    Ok(Some(extension))
}

fn ensure_scope(url: &Url, root: &Url) -> Result<(), AppError> {
    if url.origin() != root.origin() || !url.path().starts_with(root.path()) {
        return Err(AppError::bad_request("微应用资源或跳转超出登记入口目录"));
    }
    validate_path(url.path().trim_start_matches('/'))
}

fn valid_content_type(extension: Option<&str>, headers: &HeaderMap) -> bool {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match extension {
        None | Some("html" | "htm") => {
            content_type == "text/html" || content_type == "application/xhtml+xml"
        }
        Some("js" | "mjs" | "cjs") => matches!(
            content_type.as_str(),
            "application/javascript"
                | "text/javascript"
                | "application/ecmascript"
                | "text/ecmascript"
        ),
        Some("css") => content_type == "text/css",
        Some("json" | "map" | "webmanifest") => matches!(
            content_type.as_str(),
            "application/json" | "application/manifest+json"
        ),
        Some("wasm") => content_type == "application/wasm",
        Some("txt") => content_type == "text/plain",
        Some(_) => {
            content_type.starts_with("image/")
                || content_type.starts_with("font/")
                || matches!(
                    content_type.as_str(),
                    "application/octet-stream"
                        | "application/font-woff"
                        | "application/vnd.ms-fontobject"
                )
        }
    }
}

async fn fetch_static(
    client: &Client,
    policy: &FetchPolicy,
    target: Target,
    method: &Method,
) -> Response {
    let mut current = target.url;
    for attempt in 0..=MAX_REDIRECTS {
        if policy.check(&current).is_err() || ensure_scope(&current, &target.root).is_err() {
            return static_error(StatusCode::BAD_GATEWAY);
        }
        let upstream = client
            .request(method.clone(), current.clone())
            .header(header::ACCEPT_ENCODING, "identity")
            .header(header::ACCEPT, "*/*")
            .send()
            .await;
        let mut upstream = match upstream {
            Ok(response) => response,
            Err(error) => {
                return static_error(if error.is_timeout() {
                    StatusCode::GATEWAY_TIMEOUT
                } else {
                    StatusCode::BAD_GATEWAY
                })
            }
        };
        let status = upstream.status();
        if status.is_redirection() && status != StatusCode::NOT_MODIFIED {
            let next = upstream
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|location| policy.resolve(location, Some(&current)).ok());
            let Some(next) = next else {
                return static_error(StatusCode::BAD_GATEWAY);
            };
            // Following HTML across directories breaks its relative resource base.
            if attempt == MAX_REDIRECTS
                || ensure_scope(&next, &target.root).is_err()
                || (matches!(target.extension.as_deref(), None | Some("html" | "htm"))
                    && next.join(".").ok() != current.join(".").ok())
            {
                return static_error(StatusCode::BAD_GATEWAY);
            }
            current = next;
            continue;
        }
        if status.is_success()
            && !valid_content_type(target.extension.as_deref(), upstream.headers())
        {
            // A missing chunk served by an upstream SPA fallback must fail to load.
            return static_error(StatusCode::BAD_GATEWAY);
        }
        if upstream
            .content_length()
            .is_some_and(|size| size > MAX_BODY as u64)
        {
            return static_error(StatusCode::BAD_GATEWAY);
        }
        let mut response_headers = HeaderMap::new();
        // Everything else, including Set-Cookie, CSP/reporting, redirects,
        // service-worker scope, auth challenges and platform headers, is dropped.
        for name in [
            header::CONTENT_TYPE,
            header::CONTENT_ENCODING,
            header::ETAG,
            header::LAST_MODIFIED,
        ] {
            if let Some(value) = upstream.headers().get(&name) {
                response_headers.insert(name, value.clone());
            }
        }
        let mut body = Vec::new();
        if method != Method::HEAD {
            loop {
                match upstream.chunk().await {
                    Ok(Some(chunk)) if body.len().saturating_add(chunk.len()) <= MAX_BODY => {
                        body.extend_from_slice(&chunk)
                    }
                    Ok(None) => break,
                    _ => return static_error(StatusCode::BAD_GATEWAY),
                }
            }
        } else if let Some(length) = upstream.headers().get(header::CONTENT_LENGTH) {
            response_headers.insert(header::CONTENT_LENGTH, length.clone());
        }
        let mut response = (status, response_headers, Body::from(body)).into_response();
        static_headers(response.headers_mut());
        return response;
    }
    static_error(StatusCode::BAD_GATEWAY)
}

fn static_headers(headers: &mut HeaderMap) {
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    // A proxied site cannot register a worker controlling the platform origin.
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("worker-src 'none'; object-src 'none'; frame-ancestors 'self'"),
    );
}

fn static_error(status: StatusCode) -> Response {
    let mut response = (
        status,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "Micro application resource unavailable\n",
    )
        .into_response();
    static_headers(response.headers_mut());
    response
}

#[cfg(test)]
#[path = "micro_proxy/tests.rs"]
mod tests;
