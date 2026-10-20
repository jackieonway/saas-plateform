use std::{collections::HashMap, net::SocketAddr, time::Duration};
use url::Url;

/// Validated private configuration. Debug intentionally excludes credentials.
#[derive(Clone)]
pub struct Config {
    pub(crate) public: Url,
    pub(crate) platform: Url,
    pub(crate) scope: String,
    pub(crate) clients: [Option<Client>; 4],
    pub(crate) bind: SocketAddr,
    pub(crate) session_ttl: Duration,
    pub(crate) state_ttl: Duration,
    pub(crate) token_ttl: Duration,
    pub(crate) max_sessions: usize,
    pub(crate) max_states: usize,
    pub(crate) timeout: Duration,
}

#[derive(Clone)]
pub(crate) struct Client {
    pub(crate) id: String,
    pub(crate) secret: Option<String>,
}

/// A configuration failure containing a setting name, never its value.
#[derive(Debug, thiserror::Error)]
#[error("invalid or missing OAuth configuration: {0}")]
pub struct ConfigError(pub(crate) &'static str);

impl Config {
    /// Loads process environment without opening files or printing secrets.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_values(std::env::vars().collect())
    }

    /// Builds configuration from explicit values, useful for isolated deployments
    /// and tests. Origins require HTTPS, except explicitly allowed loopback HTTP.
    /// Clients absent from the map are advertised as unavailable.
    pub fn from_values(values: HashMap<String, String>) -> Result<Self, ConfigError> {
        let insecure = boolean(&values, "OAUTH_ALLOW_INSECURE_LOOPBACK", false)?;
        let public = origin(&values, "OAUTH_PUBLIC_ORIGIN", insecure)?;
        let platform = origin(&values, "OAUTH_PLATFORM_ORIGIN", insecure)?;
        let scope = values
            .get("OAUTH_SCOPE")
            .cloned()
            .unwrap_or("profile".into());
        if scope.is_empty() || scope.len() > 512 || scope.chars().any(char::is_control) {
            return Err(ConfigError("OAUTH_SCOPE"));
        }
        let password = boolean(&values, "OAUTH_PASSWORD_ENABLED", false)?;
        Ok(Self {
            public,
            platform,
            scope,
            clients: [
                client(
                    &values,
                    "OAUTH_AUTH_CLIENT_ID",
                    "OAUTH_AUTH_CLIENT_SECRET",
                    true,
                )?,
                client(
                    &values,
                    "OAUTH_DEVICE_CLIENT_ID",
                    "OAUTH_DEVICE_CLIENT_SECRET",
                    false,
                )?,
                client(
                    &values,
                    "OAUTH_CC_CLIENT_ID",
                    "OAUTH_CC_CLIENT_SECRET",
                    true,
                )?,
                if password {
                    Some(
                        client(
                            &values,
                            "OAUTH_PASSWORD_CLIENT_ID",
                            "OAUTH_PASSWORD_CLIENT_SECRET",
                            true,
                        )?
                        .ok_or(ConfigError("OAUTH_PASSWORD_CLIENT_ID"))?,
                    )
                } else {
                    None
                },
            ],
            bind: values
                .get("OAUTH_BIND_ADDR")
                .map(String::as_str)
                .unwrap_or("127.0.0.1:3011")
                .parse()
                .map_err(|_| ConfigError("OAUTH_BIND_ADDR"))?,
            session_ttl: Duration::from_secs(number(
                &values,
                "OAUTH_SESSION_TTL_SECONDS",
                1800,
                1,
                3600,
            )?),
            state_ttl: Duration::from_secs(number(
                &values,
                "OAUTH_STATE_TTL_SECONDS",
                300,
                1,
                600,
            )?),
            token_ttl: Duration::from_secs(number(
                &values,
                "OAUTH_TOKEN_TTL_SECONDS",
                3600,
                1,
                3600,
            )?),
            max_sessions: number(&values, "OAUTH_MAX_SESSIONS", 200, 1, 10000)? as usize,
            max_states: number(&values, "OAUTH_MAX_STATES", 200, 1, 10000)? as usize,
            timeout: Duration::from_secs(number(&values, "OAUTH_HTTP_TIMEOUT_SECONDS", 5, 1, 15)?),
        })
    }

    /// Returns the validated listener address. Reverse proxies must expose the
    /// configured public origin and forward `/api/` to this listener.
    pub fn bind_addr(&self) -> SocketAddr {
        self.bind
    }

    pub(crate) fn public_origin(&self) -> String {
        self.public.origin().ascii_serialization()
    }
    pub(crate) fn platform_origin(&self) -> String {
        self.platform.origin().ascii_serialization()
    }
    pub(crate) fn secure(&self) -> bool {
        self.public.scheme() == "https"
    }
    pub(crate) fn cookie_name(&self) -> &'static str {
        if self.secure() {
            "__Host-oauth_playground"
        } else {
            "oauth_playground"
        }
    }
    pub(crate) fn callback(&self) -> String {
        format!("{}/callback", self.public_origin())
    }
}

fn boolean(
    values: &HashMap<String, String>,
    name: &'static str,
    default: bool,
) -> Result<bool, ConfigError> {
    match values.get(name).map(String::as_str) {
        None => Ok(default),
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        _ => Err(ConfigError(name)),
    }
}
fn number(
    values: &HashMap<String, String>,
    name: &'static str,
    default: u64,
    min: u64,
    max: u64,
) -> Result<u64, ConfigError> {
    let n = match values.get(name) {
        Some(v) => v.parse().map_err(|_| ConfigError(name))?,
        None => default,
    };
    if n < min || n > max {
        Err(ConfigError(name))
    } else {
        Ok(n)
    }
}
fn origin(
    values: &HashMap<String, String>,
    name: &'static str,
    insecure: bool,
) -> Result<Url, ConfigError> {
    let value = values.get(name).ok_or(ConfigError(name))?;
    let url = Url::parse(value).map_err(|_| ConfigError(name))?;
    let host = url.host_str().ok_or(ConfigError(name))?;
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
    if !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !(url.scheme() == "https" || (insecure && url.scheme() == "http" && loopback))
    {
        return Err(ConfigError(name));
    }
    Ok(url)
}
fn client(
    values: &HashMap<String, String>,
    id_name: &'static str,
    secret_name: &'static str,
    confidential: bool,
) -> Result<Option<Client>, ConfigError> {
    let id = values.get(id_name).filter(|v| !v.is_empty());
    let secret = values.get(secret_name).filter(|v| !v.is_empty()).cloned();
    let Some(id) = id else {
        return if secret.is_some() {
            Err(ConfigError(id_name))
        } else {
            Ok(None)
        };
    };
    if id.len() > 200 || id.chars().any(|c| c.is_control() || c == ':') {
        return Err(ConfigError(id_name));
    }
    if confidential && secret.is_none() {
        return Err(ConfigError(secret_name));
    }
    if secret
        .as_ref()
        .is_some_and(|s| s.len() > 4096 || s.chars().any(char::is_control))
    {
        return Err(ConfigError(secret_name));
    }
    Ok(Some(Client {
        id: id.clone(),
        secret,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn values(public: &str, platform: &str, insecure: bool) -> HashMap<String, String> {
        [
            ("OAUTH_PUBLIC_ORIGIN", public),
            ("OAUTH_PLATFORM_ORIGIN", platform),
            (
                "OAUTH_ALLOW_INSECURE_LOOPBACK",
                if insecure { "true" } else { "false" },
            ),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect()
    }
    #[test]
    fn rejects_unsafe_origins_and_malformed_settings_without_values() {
        for bad in [
            "http://example.com",
            "https://user:secret@example.com",
            "https://example.com/path",
            "https://example.com/?token=private",
            "https://example.com/#fragment",
        ] {
            let err = Config::from_values(values(bad, "https://platform.test", true))
                .err()
                .unwrap();
            assert_eq!(
                err.to_string(),
                "invalid or missing OAuth configuration: OAUTH_PUBLIC_ORIGIN"
            );
        }
        assert!(Config::from_values(values(
            "http://localhost:3010",
            "http://127.0.0.1:3000",
            false
        ))
        .is_err());
        assert!(
            Config::from_values(values("http://[::1]:3010", "http://127.0.0.1:3000", true)).is_ok()
        );
        assert!(Config::from_values(values(
            "https://playground.test",
            "https://platform.test",
            false
        ))
        .is_ok());
    }
}
