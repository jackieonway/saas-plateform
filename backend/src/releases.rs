//! Server-side release checks. No browser assertion can mark a release as PASSED.
use std::collections::HashSet;
use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::{Client, Url};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::config::Config;
use crate::error::AppError;

const MAX_BODY: usize = 4 * 1024 * 1024;
const MAX_TOTAL_BODY: usize = 16 * 1024 * 1024;
const MAX_CHECKS: usize = 64;
const MAX_REDIRECTS: usize = 5;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Resource {
    pub url: String,
    pub kind: String,
}

#[derive(Clone, Debug, sqlx::FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseRow {
    pub id: Uuid,
    pub application_id: Uuid,
    pub environment: String,
    pub version: String,
    pub entry_type: String,
    pub entry_url: String,
    #[serde(skip_serializing)]
    pub runtime_mode: String,
    pub health_url: Option<String>,
    pub resources: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_digest: Option<String>,
    pub validation_status: String,
    pub validation_result: Value,
    pub checked_at: Option<DateTime<Utc>>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

pub const RELEASE_SELECT: &str = "SELECT r.id, r.application_id, r.environment, r.version, \
    r.entry_type, r.entry_url, (SELECT a.runtime_mode FROM applications a WHERE a.id=r.application_id) AS runtime_mode, \
    r.health_url, r.resources, r.artifact_digest, r.validation_status, \
    r.validation_result, r.checked_at, r.created_at, \
    EXISTS (SELECT 1 FROM application_deployments d WHERE d.application_id = r.application_id \
      AND d.environment = r.environment AND d.active_release_id = r.id) AS active ";

/// Precise origin allowlisting is the SSRF boundary. Operators may explicitly approve
/// internal gateway origins; request data cannot add hosts, ports, credentials or proxies.
pub struct FetchPolicy {
    gateway: Url,
    allowed: HashSet<String>,
    https_only: bool,
}

fn url_origin(url: &Url) -> String {
    url.origin().ascii_serialization()
}

fn parse_origin(raw: &str) -> Result<Url, AppError> {
    let url = Url::parse(raw.trim())
        .map_err(|_| AppError::bad_request("微应用网关/allowlist 必须是合法 HTTP(S) origin"))?;
    if !["http", "https"].contains(&url.scheme())
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AppError::bad_request(
            "微应用网关/allowlist 只接受无凭据、路径、query 和 fragment 的 HTTP(S) origin",
        ));
    }
    Ok(url)
}

impl FetchPolicy {
    pub fn new(config: &Config, environment: &str) -> Result<Self, AppError> {
        let gateway = parse_origin(&config.micro_gateway_origin)?;
        let allowed = config
            .micro_allowed_origins
            .iter()
            .map(|s| parse_origin(s).map(|u| url_origin(&u)))
            .collect::<Result<HashSet<_>, _>>()?;
        if allowed.is_empty() {
            return Err(AppError::bad_request("MICRO_ALLOWED_ORIGINS 不能为空"));
        }
        Ok(Self {
            gateway,
            allowed,
            https_only: config
                .application_environment
                .eq_ignore_ascii_case("production")
                || environment.eq_ignore_ascii_case("production"),
        })
    }

    pub fn resolve(&self, raw: &str, base: Option<&Url>) -> Result<Url, AppError> {
        let raw = raw.trim();
        if raw.is_empty()
            || raw.chars().count() > 2000
            || raw.contains('\\')
            || raw.chars().any(char::is_control)
        {
            return Err(AppError::bad_request(
                "发布 URL 不能为空、含反斜线/控制字符或超过 2000 字符",
            ));
        }
        let url = base
            .unwrap_or(&self.gateway)
            .join(raw)
            .map_err(|_| AppError::bad_request("发布 URL 格式错误"))?;
        self.check(&url)?;
        Ok(url)
    }

    pub fn check(&self, url: &Url) -> Result<(), AppError> {
        if !["http", "https"].contains(&url.scheme())
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(AppError::bad_request(
                "发布 URL 仅支持无凭据和 fragment 的 HTTP(S) 地址",
            ));
        }
        if self.https_only && url.scheme() != "https" {
            return Err(AppError::bad_request("production 发布与预检必须使用 HTTPS"));
        }
        if !self.allowed.contains(&url_origin(url)) {
            return Err(AppError::bad_request(
                "URL origin 未列入 MICRO_ALLOWED_ORIGINS，已阻止服务端访问",
            ));
        }
        Ok(())
    }

    fn browser_origin(&self) -> String {
        url_origin(&self.gateway)
    }

    pub fn stored_url(&self, raw: &str, resolved: &Url) -> String {
        // Preserve gateway paths for the browser; all other references use canonical URLs.
        if raw.trim().starts_with('/')
            && !raw.trim().starts_with("//")
            && url_origin(resolved) == url_origin(&self.gateway)
        {
            match resolved.query() {
                Some(q) => format!("{}?{q}", resolved.path()),
                None => resolved.path().to_string(),
            }
        } else {
            resolved.to_string()
        }
    }
}

pub fn validate_environment(value: &str) -> Result<String, AppError> {
    if value.is_empty()
        || value.len() > 32
        || !value.as_bytes()[0].is_ascii_lowercase() && !value.as_bytes()[0].is_ascii_digit()
        || !value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
    {
        return Err(AppError::bad_request(
            "environment 仅接受 1–32 位小写字母、数字、下划线和中划线，首位为字母/数字",
        ));
    }
    Ok(value.to_string())
}

pub fn validate_version(value: &str) -> Result<String, AppError> {
    if value.is_empty()
        || value.len() > 64
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || value.contains("..")
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || [b'-', b'_', b'.'].contains(&c))
    {
        return Err(AppError::bad_request(
            "version 仅接受 1–64 位字母、数字、点、下划线和中划线，不能含 ..",
        ));
    }
    Ok(value.to_string())
}

/// Include the version as a literal directory segment. Query/fragment aliases cannot
/// represent immutable releases. Redirects and same-origin code stay inside this root.
pub fn version_root(entry: &Url, version: &str) -> Result<String, AppError> {
    if entry.query().is_some() {
        return Err(AppError::bad_request(
            "版本入口不能带 query，请使用固定版本目录",
        ));
    }
    let segments: Vec<_> = entry.path().split('/').collect();
    let index = segments
        .iter()
        .position(|s| *s == version)
        .filter(|i| *i + 1 < segments.len())
        .ok_or_else(|| {
            AppError::bad_request(
                "entryUrl 必须包含与 version 相同的固定目录段，例如 /sub/vue/1.0.0/index.html",
            )
        })?;
    Ok(format!("{}/", segments[..=index].join("/")))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Check {
    url: String,
    kind: String,
    passed: bool,
    status_code: Option<u16>,
    content_type: Option<String>,
    bytes: usize,
    message: String,
}

struct Fetched {
    check: Check,
    final_url: Url,
    body: Vec<u8>,
}

fn mime_ok(kind: &str, mime: &str) -> bool {
    let mime = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match kind {
        "HTML" => ["text/html", "application/xhtml+xml"].contains(&mime.as_str()),
        "SCRIPT" => [
            "text/javascript",
            "application/javascript",
            "text/ecmascript",
            "application/ecmascript",
        ]
        .contains(&mime.as_str()),
        "STYLE" => mime == "text/css",
        "ASSET" => {
            !mime.is_empty() && !["text/html", "application/xhtml+xml"].contains(&mime.as_str())
        }
        "HEALTH" => true,
        _ => false,
    }
}

fn failure(url: &Url, kind: &str, message: impl Into<String>) -> Fetched {
    Fetched {
        check: Check {
            url: url.to_string(),
            kind: kind.into(),
            passed: false,
            status_code: None,
            content_type: None,
            bytes: 0,
            message: message.into(),
        },
        final_url: url.clone(),
        body: Vec::new(),
    }
}

fn frame_source_matches(source: &str, ancestor: &Url, protected: &Url) -> bool {
    if source == "*" {
        return true;
    }
    if source == "'self'" {
        return url_origin(ancestor) == url_origin(protected);
    }
    if source.ends_with(':') && !source.contains('/') {
        return source.trim_end_matches(':') == ancestor.scheme();
    }
    let raw = if source.contains("://") {
        source.to_string()
    } else {
        format!("{}://{source}", protected.scheme())
    };
    let authority = raw
        .split("//")
        .nth(1)
        .unwrap_or("")
        .split('/')
        .next()
        .unwrap_or("");
    let any_port = authority.ends_with(":*");
    let raw = if any_port {
        raw.replacen(":*", "", 1)
    } else {
        raw
    };
    let wildcard = raw.contains("://*.");
    let raw = raw.replacen("://*.", "://wildcard-marker.", 1);
    let Ok(source) = Url::parse(&raw) else {
        return false;
    };
    if !source.username().is_empty() || source.password().is_some() {
        return false;
    }
    let scheme_matches = source.scheme() == ancestor.scheme()
        || source.scheme() == "http" && ancestor.scheme() == "https";
    let port_matches = any_port
        || source.port_or_known_default() == ancestor.port_or_known_default()
        || source.scheme() == "http"
            && source.port_or_known_default() == Some(80)
            && ancestor.scheme() == "https"
            && ancestor.port_or_known_default() == Some(443);
    let host_matches = match (source.host_str(), ancestor.host_str()) {
        (Some(source), Some(target)) if wildcard => source
            .strip_prefix("wildcard-marker.")
            .is_some_and(|suffix| target.ends_with(&format!(".{suffix}"))),
        (Some(source), Some(target)) => source == target,
        _ => false,
    };
    scheme_matches && port_matches && host_matches
}

fn frame_error(
    headers: &reqwest::header::HeaderMap,
    protected: &Url,
    origin: &str,
) -> Option<String> {
    let ancestor = Url::parse(origin).ok()?;
    for header in headers.get_all("x-frame-options") {
        let value = header.to_str().unwrap_or("");
        for directive in value.split(',').map(str::trim) {
            if directive.eq_ignore_ascii_case("DENY") {
                return Some("IFRAME 入口的 X-Frame-Options: DENY 阻止宿主嵌入".into());
            }
            if directive.eq_ignore_ascii_case("SAMEORIGIN") && url_origin(protected) != origin {
                return Some("IFRAME 入口的 X-Frame-Options: SAMEORIGIN 与宿主来源不匹配".into());
            }
            if !directive.eq_ignore_ascii_case("SAMEORIGIN") && !directive.is_empty() {
                return Some(
                    "IFRAME 入口包含不支持的 X-Frame-Options，请配置现代 frame-ancestors".into(),
                );
            }
        }
    }
    for header in headers.get_all("content-security-policy") {
        let Ok(header) = header.to_str() else {
            return Some("IFRAME 入口 CSP 无法解析".into());
        };
        for policy in header.split(',') {
            for directive in policy.split(';') {
                let mut tokens = directive.split_whitespace();
                if tokens
                    .next()
                    .is_some_and(|name| name.eq_ignore_ascii_case("frame-ancestors"))
                {
                    let sources: Vec<_> = tokens.collect();
                    if sources.contains(&"'none'")
                        || !sources
                            .iter()
                            .any(|source| frame_source_matches(source, &ancestor, protected))
                    {
                        return Some(format!(
                            "IFRAME 入口 CSP frame-ancestors 不允许宿主来源 {origin}"
                        ));
                    }
                    break;
                }
            }
        }
    }
    None
}

async fn fetch(
    client: &Client,
    policy: &FetchPolicy,
    url: Url,
    kind: &str,
    entry_root: Option<&str>,
    entry_origin: &str,
    pin_entry: bool,
    cors_origin: Option<&str>,
    frame_origin: Option<&str>,
) -> Fetched {
    let mut current = url.clone();
    for redirect in 0..=MAX_REDIRECTS {
        if let Err(error) = policy.check(&current) {
            return failure(&current, kind, error.to_string());
        }
        // HTML redirects and same-origin JavaScript/CSS cannot escape the pinned directory.
        if let Some(root) = entry_root {
            if (pin_entry || url_origin(&current) == entry_origin)
                && (!current.path().starts_with(root) || pin_entry && current.query().is_some())
            {
                return failure(&current, kind, "入口或同源脚本/样式移出了固定版本目录");
            }
        }
        if pin_entry
            && kind == "HTML"
            && (url_origin(&current) != url_origin(&url)
                || current.join(".").ok() != url.join(".").ok())
        {
            return failure(
                &current,
                kind,
                "HTML 入口 redirect 改变了来源或资源解析目录，宿主无法使用同一基址",
            );
        }
        let mut request = client.get(current.clone());
        if let Some(origin) = cors_origin {
            request = request.header(reqwest::header::ORIGIN, origin);
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                return failure(
                    &current,
                    kind,
                    if error.is_timeout() {
                        "HTTP 请求超时（单请求上限 4 秒）"
                    } else {
                        "HTTP 请求失败，请检查 DNS、证书和网络连通性"
                    },
                )
            }
        };
        let status = response.status();
        if let Some(origin) = cors_origin {
            if url_origin(&current) != origin {
                let allowed = response
                    .headers()
                    .get(reqwest::header::ACCESS_CONTROL_ALLOW_ORIGIN)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("");
                if allowed != "*" && allowed != origin {
                    return failure(
                        &current,
                        kind,
                        if origin == "null" {
                            "IFRAME opaque origin 模块/CORS 资源要求 Access-Control-Allow-Origin: * 或 null"
                        } else {
                            "跨源 TRUSTED 入口或资源缺少允许宿主 origin 的 Access-Control-Allow-Origin"
                        },
                    );
                }
            }
        }
        if status.is_redirection() {
            if redirect == MAX_REDIRECTS {
                return failure(&current, kind, "redirect 次数超过 5 次");
            }
            let next = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|h| h.to_str().ok());
            current = match next.and_then(|s| current.join(s).ok()) {
                Some(next) => next,
                None => return failure(&current, kind, "redirect 缺少有效 Location"),
            };
            continue;
        }
        if let Some(origin) = frame_origin {
            if let Some(message) = frame_error(response.headers(), &current, origin) {
                return failure(&current, kind, message);
            }
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let mut check = Check {
            url: current.to_string(),
            kind: kind.into(),
            passed: false,
            status_code: Some(status.as_u16()),
            content_type,
            bytes: 0,
            message: String::new(),
        };
        if !status.is_success() {
            check.message = format!("HTTP {}，资源不可用", status.as_u16());
            return Fetched {
                check,
                final_url: current,
                body: Vec::new(),
            };
        }
        if !mime_ok(kind, check.content_type.as_deref().unwrap_or("")) {
            check.message = format!(
                "MIME 不匹配：{kind} 资源必须返回对应 Content-Type，不能使用 SPA HTML fallback"
            );
            return Fetched {
                check,
                final_url: current,
                body: Vec::new(),
            };
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_BODY as u64)
        {
            check.message = "响应超过 4 MiB 上限".into();
            return Fetched {
                check,
                final_url: current,
                body: Vec::new(),
            };
        }
        let mut response = response;
        let mut body = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) if body.len() + chunk.len() <= MAX_BODY => {
                    body.extend_from_slice(&chunk)
                }
                Ok(Some(_)) => {
                    check.message = "响应超过 4 MiB 上限".into();
                    break;
                }
                Ok(None) => {
                    check.passed = kind == "HEALTH" || !body.is_empty();
                    check.message = if check.passed {
                        "通过".into()
                    } else {
                        "入口或资源为空".into()
                    };
                    break;
                }
                Err(_) => {
                    check.message = "读取 HTTP 响应失败或超时".into();
                    break;
                }
            }
        }
        check.bytes = body.len();
        return Fetched {
            check,
            final_url: current,
            body,
        };
    }
    failure(&url, kind, "redirect 超限")
}

/// Use an HTML parser for actual browser URL references, including a document base URL.
/// Runtime-only imports belong in the immutable resources manifest.
struct HtmlAssets {
    resources: Vec<Resource>,
    native_module: bool,
}

fn discover_html(body: &[u8], base: &Url, policy: &FetchPolicy) -> Result<HtmlAssets, AppError> {
    let text =
        std::str::from_utf8(body).map_err(|_| AppError::bad_request("HTML 入口必须为 UTF-8"))?;
    let html = Html::parse_document(text);
    let native_module = html
        .select(&Selector::parse("script[type]").unwrap())
        .any(|script| {
            script
                .value()
                .attr("type")
                .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("module"))
        });
    let mut base = base.clone();
    if let Some(node) = html.select(&Selector::parse("base[href]").unwrap()).next() {
        let declared = policy.resolve(node.value().attr("href").unwrap_or(""), Some(&base))?;
        if declared.join(".").ok() != base.join(".").ok() {
            return Err(AppError::bad_request(
                "HTML base href 改变了资源解析目录，请在版本入口中使用直接相对地址",
            ));
        }
        base = declared;
    }
    let mut resources = Vec::new();
    for node in html.select(&Selector::parse("script[src], link[href]").unwrap()) {
        let element = node.value();
        let (raw, kind) = if element.name() == "script" {
            let module = element
                .attr("type")
                .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("module"));
            let cors = element.attr("crossorigin");
            if cors.is_some_and(|value| value.eq_ignore_ascii_case("use-credentials")) {
                return Err(AppError::bad_request(
                    "静态发布预检仅支持匿名资源，请移除 crossorigin=use-credentials",
                ));
            }
            (
                element.attr("src").unwrap_or(""),
                if module {
                    "MODULE"
                } else if cors.is_some() {
                    "SCRIPT_CORS"
                } else {
                    "SCRIPT"
                },
            )
        } else {
            let rel = element.attr("rel").unwrap_or("").to_ascii_lowercase();
            let rel: Vec<_> = rel.split_whitespace().collect();
            let kind = if rel.contains(&"stylesheet") {
                "STYLE"
            } else if rel.contains(&"modulepreload") {
                "MODULE"
            } else if rel.contains(&"preload") {
                match element.attr("as").unwrap_or("") {
                    "script" => "SCRIPT",
                    "style" => "STYLE",
                    _ => continue,
                }
            } else {
                continue;
            };
            let cors = element.attr("crossorigin");
            if cors.is_some_and(|value| value.eq_ignore_ascii_case("use-credentials")) {
                return Err(AppError::bad_request(
                    "静态发布预检仅支持匿名资源，请移除 crossorigin=use-credentials",
                ));
            }
            let kind = match (kind, cors.is_some()) {
                ("SCRIPT", true) => "SCRIPT_CORS",
                ("STYLE", true) => "STYLE_CORS",
                _ => kind,
            };
            (element.attr("href").unwrap_or(""), kind)
        };
        let url = policy.resolve(raw, Some(&base))?;
        resources.push(Resource {
            url: url.to_string(),
            kind: kind.to_string(),
        });
        if resources.len() > MAX_CHECKS {
            return Err(AppError::bad_request(
                "HTML 脚本/样式超过 64 个预检资源上限",
            ));
        }
    }
    let native_module = native_module || resources.iter().any(|resource| resource.kind == "MODULE");
    Ok(HtmlAssets {
        resources,
        native_module,
    })
}

fn result(checks: &[Check]) -> Value {
    json!({"passed": !checks.is_empty() && checks.iter().all(|c| c.passed),
        "checkedAt": Utc::now(), "checks": checks,
        "limits": {"requestTimeoutSeconds": 4, "totalTimeoutSeconds": 30,
            "maxResponseBytes": MAX_BODY, "maxTotalBytes": MAX_TOTAL_BODY,
            "maxChecks": MAX_CHECKS, "maxRedirects": MAX_REDIRECTS}})
}

async fn run(config: &Config, release: &ReleaseRow) -> Value {
    let setup = (|| -> Result<_, AppError> {
        let policy = FetchPolicy::new(config, &release.environment)?;
        let entry = policy.resolve(&release.entry_url, None)?;
        let root = version_root(&entry, &release.version)?;
        let mut client_builder = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(4))
            .user_agent("Platform-Application-Preflight/1.0");
        for certificate in &config.micro_ca_certificates {
            client_builder = client_builder.add_root_certificate(certificate.clone());
        }
        let client = client_builder
            .build()
            .map_err(|_| AppError::bad_request("无法建立 HTTP 预检客户端"))?;
        Ok((policy, entry, root, client))
    })();
    let (policy, entry, root, client) = match setup {
        Ok(setup) => setup,
        Err(error) => {
            return json!({"passed":false,"checkedAt":Utc::now(),"checks":[{
            "url":release.entry_url,"kind":"POLICY","passed":false,"message":error.to_string()}]})
        }
    };
    let entry_kind = if release.entry_type == "HTML" {
        "HTML"
    } else {
        "SCRIPT"
    };
    let entry_origin = url_origin(&entry);
    let browser_origin = policy.browser_origin();
    let cors_origin = if release.runtime_mode == "TRUSTED" {
        Some(browser_origin.as_str())
    } else {
        None
    };
    let fetched = fetch(
        &client,
        &policy,
        entry.clone(),
        entry_kind,
        Some(&root),
        &entry_origin,
        true,
        cors_origin,
        if release.runtime_mode == "IFRAME" && release.entry_type == "HTML" {
            Some(browser_origin.as_str())
        } else {
            None
        },
    )
    .await;
    let resource_origin = url_origin(&fetched.final_url);
    let mut checks = Vec::new();
    let mut resources: Vec<Resource> = match serde_json::from_value(release.resources.clone()) {
        Ok(resources) => resources,
        Err(_) => {
            checks.push(
                failure(
                    &entry,
                    "RESOURCE_MANIFEST",
                    "resources 不是合法 URL/kind 资源清单",
                )
                .check,
            );
            Vec::new()
        }
    };
    let mut native_module_document = false;
    for resource in &mut resources {
        if resource.kind == "SCRIPT" {
            resource.kind = "MANIFEST_SCRIPT".into();
        }
    }
    if fetched.check.passed && release.entry_type == "HTML" {
        match discover_html(&fetched.body, &fetched.final_url, &policy) {
            Ok(discovered) => {
                native_module_document = discovered.native_module;
                if native_module_document && release.runtime_mode == "TRUSTED" {
                    checks.push(failure(&entry, "HTML_MODULE", "TRUSTED HTML 加载器不支持原生 module 脚本；请选择 IFRAME HTML 或 TRUSTED ESM 入口").check);
                }
                resources.extend(discovered.resources);
            }
            Err(error) => checks.push(failure(&entry, "HTML_RESOURCES", error.to_string()).check),
        }
    }
    let mut total_bytes = fetched.check.bytes;
    let entry_passed = fetched.check.passed;
    checks.push(fetched.check);
    // Still check explicit resources and health after an entry failure for actionable diagnostics.
    let mut seen = HashSet::new();
    resources.sort_by_key(|resource| {
        if resource.kind == "MODULE"
            || resource.kind.ends_with("_CORS")
            || native_module_document && resource.kind == "MANIFEST_SCRIPT"
        {
            0
        } else {
            1
        }
    });
    for mut resource in resources {
        let opaque_cors = resource.kind == "MODULE"
            || resource.kind.ends_with("_CORS")
            || native_module_document && resource.kind == "MANIFEST_SCRIPT";
        resource.kind = match resource.kind.as_str() {
            "MODULE" | "SCRIPT_CORS" | "MANIFEST_SCRIPT" => "SCRIPT".into(),
            "STYLE_CORS" => "STYLE".into(),
            _ => resource.kind,
        };
        let resolved = match policy.resolve(&resource.url, Some(&entry)) {
            Ok(url) => url,
            Err(error) => {
                checks.push(Check {
                    url: resource.url,
                    kind: resource.kind,
                    passed: false,
                    status_code: None,
                    content_type: None,
                    bytes: 0,
                    message: error.to_string(),
                });
                continue;
            }
        };
        if !seen.insert((resolved.to_string(), resource.kind.clone())) {
            continue;
        }
        if checks.len() >= MAX_CHECKS {
            checks.push(failure(&entry, "LIMIT", "资源数超过 64 个预检上限").check);
            break;
        }
        let code_root = if ["SCRIPT", "STYLE"].contains(&resource.kind.as_str()) {
            Some(root.as_str())
        } else {
            None
        };
        let resource_cors = if ["SCRIPT", "STYLE"].contains(&resource.kind.as_str()) {
            if release.runtime_mode == "IFRAME" && opaque_cors {
                Some("null")
            } else {
                cors_origin
            }
        } else {
            None
        };
        let fetched = fetch(
            &client,
            &policy,
            resolved,
            &resource.kind,
            code_root,
            &resource_origin,
            false,
            resource_cors,
            None,
        )
        .await;
        total_bytes += fetched.check.bytes;
        checks.push(fetched.check);
        if total_bytes > MAX_TOTAL_BODY {
            checks.push(failure(&entry, "LIMIT", "预检总响应超过 16 MiB 上限").check);
            break;
        }
    }
    if let Some(health) = &release.health_url {
        if checks.len() >= MAX_CHECKS {
            checks.push(failure(&entry, "LIMIT", "资源和健康检查超过 64 个预检上限").check);
            return result(&checks);
        }
        match policy.resolve(health, None) {
            Ok(url) => {
                let fetched = fetch(
                    &client,
                    &policy,
                    url,
                    "HEALTH",
                    None,
                    &entry_origin,
                    false,
                    None,
                    None,
                )
                .await;
                total_bytes += fetched.check.bytes;
                checks.push(fetched.check);
                if total_bytes > MAX_TOTAL_BODY {
                    checks.push(failure(&entry, "LIMIT", "预检总响应超过 16 MiB 上限").check);
                }
            }
            Err(error) => checks.push(failure(&entry, "HEALTH", error.to_string()).check),
        }
    }
    let mut result = result(&checks);
    result["entryAvailable"] = json!(entry_passed);
    result
}

pub async fn preflight(config: &Config, release: &ReleaseRow) -> Value {
    match tokio::time::timeout(Duration::from_secs(30), run(config, release)).await {
        Ok(result) => result,
        Err(_) => json!({"passed":false,"checkedAt":Utc::now(),"checks":[{
            "url":release.entry_url,"kind":"LIMIT","passed":false,"message":"预检超过 30 秒总时间上限"}]}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_directory_segments_and_origins_are_exact() {
        let mut config = Config::from_env();
        config.application_environment = "development".into();
        config.micro_gateway_origin = "http://localhost:3000".into();
        config.micro_allowed_origins = vec!["http://localhost:3000".into()];
        let policy = FetchPolicy::new(&config, "development").unwrap();
        let entry = policy
            .resolve("/micro/demo/1.0.0/index.html", None)
            .unwrap();
        assert_eq!(version_root(&entry, "1.0.0").unwrap(), "/micro/demo/1.0.0/");
        assert!(version_root(&entry, "1.0").is_err());
        for url in [
            "http://localhost:3001/app/1/a.js",
            "http://user@localhost:3000/app/1/a.js",
            "http://localhost.evil:3000/app/1/a.js",
            "file:///app/1/a.js",
            "//example.com/a.js",
        ] {
            assert!(policy.resolve(url, None).is_err());
        }
        assert!(FetchPolicy::new(&config, "production")
            .unwrap()
            .resolve("/micro/demo/1.0.0/index.html", None)
            .is_err());
        assert!(version_root(
            &policy.resolve("/app/1/index.html?latest=1", None).unwrap(),
            "1"
        )
        .is_err());
        assert!(validate_version("../latest").is_err());
    }

    #[test]
    fn html_parser_discovers_real_references_and_rejects_base_changes() {
        let mut config = Config::from_env();
        config.application_environment = "development".into();
        config.micro_gateway_origin = "http://localhost:3000".into();
        config.micro_allowed_origins = vec!["http://localhost:3000".into()];
        let policy = FetchPolicy::new(&config, "development").unwrap();
        let base = policy.resolve("/apps/demo/v1/index.html", None).unwrap();
        let resources = discover_html(
            br#"<!doctype html><html><head><base href="./">
            <link rel="stylesheet" href="main.css"><link rel="modulepreload" href="chunk.js">
            <link rel="icon" href="favicon.ico"></head><body><script src="main.js"></script>
            <!-- <script src="missing.js"></script> --></body></html>"#,
            &base,
            &policy,
        )
        .unwrap()
        .resources;
        assert_eq!(resources.len(), 3);
        assert!(resources
            .iter()
            .all(|r| r.url.starts_with("http://localhost:3000/apps/demo/v1/")));
        assert!(resources.iter().any(|r| r.kind == "STYLE"));
        assert!(discover_html(
            br#"<html><head><base href="./assets/"></head></html>"#,
            &base,
            &policy
        )
        .is_err());
    }
}
