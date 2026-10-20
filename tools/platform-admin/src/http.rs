use crate::{
    SafeError,
    contract::{MAX_JSON_BYTES, strict_json},
    options::Options,
};
use reqwest::{
    Url,
    blocking::Client,
    header::{CONTENT_TYPE, HeaderValue},
    redirect::Policy,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    time::{Duration, Instant},
};

pub(crate) struct ManagementApi {
    client: Client,
    origin: Url,
    bearer: HeaderValue,
    deadline: Instant,
}
impl ManagementApi {
    pub(crate) fn new(
        env: &BTreeMap<String, String>,
        options: &Options,
    ) -> Result<Self, SafeError> {
        let deadline = Instant::now() + Duration::from_millis(options.timeout_ms);
        let raw = env
            .get("PLATFORM_API_ORIGIN")
            .ok_or_else(|| SafeError::new("INVALID_ORIGIN"))?;
        if raw.chars().any(|ch| ch.is_whitespace() || ch.is_control()) || raw.contains('\\') {
            return Err(SafeError::new("INVALID_ORIGIN"));
        }
        let origin = Url::parse(raw).map_err(|_| SafeError::new("INVALID_ORIGIN"))?;
        if !matches!(origin.scheme(), "http" | "https")
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.query().is_some()
            || origin.fragment().is_some()
            || origin.path() != "/"
            || origin.origin().ascii_serialization() != raw.strip_suffix('/').unwrap_or(raw)
        {
            return Err(SafeError::new("INVALID_ORIGIN"));
        }
        if origin.scheme() != "https"
            && !(options.environment == "development"
                && options.allow_loopback_http
                && is_loopback(&origin))
        {
            return Err(SafeError::new("HTTPS_REQUIRED"));
        }
        let bearer = env
            .get("PLATFORM_ADMIN_BEARER")
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= 8192
                    && !value
                        .chars()
                        .any(|ch| ch.is_whitespace() || ch.is_control())
            })
            .ok_or_else(|| SafeError::new("ADMIN_BEARER_REQUIRED"))?;
        let mut bearer = HeaderValue::from_str(&format!("Bearer {bearer}"))
            .map_err(|_| SafeError::new("ADMIN_BEARER_REQUIRED"))?;
        bearer.set_sensitive(true);
        let mut builder = Client::builder()
            .use_rustls_tls()
            .redirect(Policy::none())
            .no_proxy()
            .retry(reqwest::retry::never())
            .timeout(Duration::from_millis(options.timeout_ms))
            .connect_timeout(Duration::from_millis(options.timeout_ms));
        if let Some(path) = env.get("PLATFORM_CA_FILE") {
            let invalid = || SafeError::new("INVALID_CA_CONFIGURATION");
            let file = File::open(path).map_err(|_| invalid())?;
            if !file.metadata().map_err(|_| invalid())?.is_file() {
                return Err(invalid());
            }
            let mut pem = Vec::new();
            file.take(65537)
                .read_to_end(&mut pem)
                .map_err(|_| invalid())?;
            if pem.is_empty() || pem.len() > 65536 {
                return Err(invalid());
            }
            let text = std::str::from_utf8(&pem).map_err(|_| invalid())?;
            let mut rest = text.trim();
            let mut blocks = 0;
            while !rest.is_empty() {
                rest = rest
                    .strip_prefix("-----BEGIN CERTIFICATE-----")
                    .ok_or_else(invalid)?;
                let (body, tail) = rest
                    .split_once("-----END CERTIFICATE-----")
                    .ok_or_else(invalid)?;
                if body.is_empty()
                    || !body.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric()
                            || matches!(byte, b'+' | b'/' | b'=')
                            || byte.is_ascii_whitespace()
                    })
                {
                    return Err(invalid());
                }
                blocks += 1;
                rest = tail.trim();
            }
            let certificates =
                reqwest::Certificate::from_pem_bundle(&pem).map_err(|_| invalid())?;
            if blocks == 0 || certificates.len() != blocks {
                return Err(invalid());
            }
            for certificate in certificates {
                builder = builder.add_root_certificate(certificate);
            }
        }
        let client = builder.build().map_err(|_| {
            SafeError::new(if env.contains_key("PLATFORM_CA_FILE") {
                "INVALID_CA_CONFIGURATION"
            } else {
                "NETWORK_CONFIGURATION_ERROR"
            })
        })?;
        Ok(Self {
            client,
            origin,
            bearer,
            deadline,
        })
    }
    pub(crate) fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, SafeError> {
        if !matches!(method, "GET" | "POST" | "PUT" | "PATCH")
            || !path.starts_with("/api/")
            || path
                .chars()
                .any(|ch| ch.is_control() || matches!(ch, '\\' | '#'))
        {
            return Err(SafeError::new("INVALID_REQUEST"));
        }
        let target = self
            .origin
            .join(path)
            .map_err(|_| SafeError::new("INVALID_REQUEST"))?;
        if target.origin() != self.origin.origin() {
            return Err(SafeError::new("INVALID_REQUEST"));
        }
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| SafeError::new("INVALID_REQUEST"))?;
        let writing = method != reqwest::Method::GET;
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| SafeError::new("REQUEST_TIMEOUT"))?;
        let mut request = self
            .client
            .request(method, target)
            .timeout(remaining)
            .header("authorization", self.bearer.clone())
            .header("accept", "application/json");
        if let Some(body) = body {
            request = request.json(body);
        }
        let mut response = request.send().map_err(|error| {
            SafeError::http(
                if error.is_timeout() {
                    "REQUEST_TIMEOUT"
                } else {
                    "NETWORK_ERROR"
                },
                None,
                writing,
            )
        })?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            return Err(SafeError::http("REDIRECT_REJECTED", Some(status), writing));
        }
        if !(200..300).contains(&status) {
            return Err(SafeError::http("HTTP_ERROR", Some(status), writing));
        }
        let json_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|header| header.to_str().ok())
            .and_then(|header| header.split(';').next())
            .map(str::trim)
            .is_some_and(|value| {
                value.eq_ignore_ascii_case("application/json")
                    || value.to_ascii_lowercase().starts_with("application/")
                        && value.to_ascii_lowercase().ends_with("+json")
            });
        if !json_type {
            return Err(SafeError::http("INVALID_RESPONSE", Some(status), writing));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_JSON_BYTES as u64)
        {
            return Err(SafeError::http("RESPONSE_TOO_LARGE", Some(status), writing));
        }
        let mut bytes = Vec::new();
        response
            .by_ref()
            .take(MAX_JSON_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                SafeError::http(
                    if error.kind() == std::io::ErrorKind::TimedOut
                        || error
                            .get_ref()
                            .and_then(|inner| inner.downcast_ref::<reqwest::Error>())
                            .is_some_and(reqwest::Error::is_timeout)
                    {
                        "REQUEST_TIMEOUT"
                    } else {
                        "NETWORK_ERROR"
                    },
                    Some(status),
                    writing,
                )
            })?;
        if bytes.len() > MAX_JSON_BYTES {
            return Err(SafeError::http("RESPONSE_TOO_LARGE", Some(status), writing));
        }
        strict_json(&bytes).map_err(|_| SafeError::http("INVALID_RESPONSE", Some(status), writing))
    }
}
pub(crate) fn is_loopback(url: &Url) -> bool {
    matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    )
}
