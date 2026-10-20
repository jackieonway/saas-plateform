use crate::{config::Client, session::Token, Config};
use axum::http::StatusCode;
use reqwest::{Client as HttpClient, Method};
use serde_json::Value;
use std::time::Instant;

const MAX_BODY: usize = 256 * 1024;

pub(crate) struct Upstream {
    http: HttpClient,
}
pub(crate) struct Failure {
    pub(crate) status: StatusCode,
    pub(crate) code: &'static str,
}
impl Failure {
    pub(crate) fn new(status: StatusCode, code: &'static str) -> Self {
        Self { status, code }
    }
    fn unavailable() -> Self {
        Self::new(StatusCode::BAD_GATEWAY, "upstream_unavailable")
    }
}
impl Upstream {
    pub(crate) fn new(config: &Config) -> Result<Self, crate::ConfigError> {
        let http = HttpClient::builder()
            // Credentials go directly to the configured origin; ambient OS or
            // environment proxies must not silently change that trust path.
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(config.timeout)
            .connect_timeout(config.timeout)
            .build()
            .map_err(|_| crate::ConfigError("HTTP_CLIENT"))?;
        Ok(Self { http })
    }
    pub(crate) async fn request(
        &self,
        config: &Config,
        method: Method,
        path: &'static str,
        client: Option<&Client>,
        form: &[(String, String)],
        bearer: Option<&str>,
    ) -> Result<Value, Failure> {
        // Every caller supplies a compile-time path. Neither browser bodies nor
        // platform responses choose a destination or alter redirect policy.
        let url = config
            .platform
            .join(path)
            .map_err(|_| Failure::unavailable())?;
        let mut request = self
            .http
            .request(method, url)
            .header("Accept", "application/json");
        if let Some(client) = client {
            if let Some(secret) = &client.secret {
                // OAuth Basic credentials are form-encoded before Base64.
                let id: String =
                    url::form_urlencoded::byte_serialize(client.id.as_bytes()).collect();
                let secret: String =
                    url::form_urlencoded::byte_serialize(secret.as_bytes()).collect();
                request = request.basic_auth(id, Some(secret));
            }
        }
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        if !form.is_empty() {
            request = request.form(form);
        }
        let mut response = request.send().await.map_err(|_| Failure::unavailable())?;
        let status = response.status();
        if status.is_redirection()
            || response
                .content_length()
                .is_some_and(|n| n > MAX_BODY as u64)
        {
            return Err(Failure::unavailable());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Failure::unavailable())? {
            if body.len() + chunk.len() > MAX_BODY {
                return Err(Failure::unavailable());
            }
            body.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&body).map_err(|_| Failure::unavailable())?;
        if !status.is_success() {
            let code = match value.get("error").and_then(Value::as_str) {
                Some("authorization_pending") => "authorization_pending",
                Some("slow_down") => "slow_down",
                Some("access_denied") => "access_denied",
                Some("expired_token") => "expired_token",
                Some("invalid_grant") => "invalid_grant",
                Some("unsupported_grant_type") => "unsupported_flow",
                Some("unauthorized_client") => "unauthorized_client",
                Some("invalid_client") => "invalid_client",
                Some("invalid_scope") => "invalid_scope",
                Some("invalid_token") => "invalid_token",
                _ => "upstream_rejected",
            };
            let status = if code == "slow_down" || status == StatusCode::TOO_MANY_REQUESTS {
                StatusCode::TOO_MANY_REQUESTS
            } else if status.is_server_error() {
                StatusCode::BAD_GATEWAY
            } else if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
                status
            } else {
                StatusCode::BAD_REQUEST
            };
            return Err(Failure::new(status, code));
        }
        Ok(value)
    }
    pub(crate) async fn token(
        &self,
        config: &Config,
        client: &Client,
        mut form: Vec<(String, String)>,
    ) -> Result<Token, Failure> {
        form.push(("client_id".into(), client.id.clone()));
        let value = self
            .request(
                config,
                Method::POST,
                "/oauth/token",
                Some(client),
                &form,
                None,
            )
            .await?;
        let access = token_string(&value, "access_token")?;
        let refresh = match value.get("refresh_token") {
            Some(Value::String(v)) if !v.is_empty() && v.len() <= 16384 => Some(v.clone()),
            None | Some(Value::Null) => None,
            _ => return Err(Failure::unavailable()),
        };
        let token_type = value
            .get("token_type")
            .and_then(Value::as_str)
            .filter(|v| v.eq_ignore_ascii_case("bearer"))
            .ok_or_else(Failure::unavailable)?
            .to_owned();
        let ttl = value
            .get("expires_in")
            .and_then(Value::as_u64)
            .filter(|v| *v > 0)
            .ok_or_else(Failure::unavailable)?
            .min(config.token_ttl.as_secs());
        let scope = value
            .get("scope")
            .and_then(Value::as_str)
            .unwrap_or(&config.scope)
            .to_owned();
        if scope.len() > 512 || scope.chars().any(char::is_control) {
            return Err(Failure::unavailable());
        }
        let now = Instant::now();
        Ok(Token {
            access,
            refresh,
            token_type,
            scope,
            expires: now + std::time::Duration::from_secs(ttl),
            retain_until: now + config.token_ttl,
            expires_in: ttl,
        })
    }
}
fn token_string(value: &Value, key: &str) -> Result<String, Failure> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= 16384 && !v.chars().any(char::is_control))
        .map(str::to_owned)
        .ok_or_else(Failure::unavailable)
}
