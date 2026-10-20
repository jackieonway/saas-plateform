use axum::http::{HeaderValue, Uri};
use std::{error::Error, fmt, net::SocketAddr};

/// Validated loopback listener and exact browser-origin allowlist.
#[derive(Clone)]
pub struct ServerConfig {
    bind_addr: SocketAddr,
    pub(crate) origins: Vec<HeaderValue>,
}

/// Configuration failure without echoing untrusted environment values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(&'static str);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl Error for ConfigError {}

impl ServerConfig {
    /// Read `CHAT_BIND_ADDR` and comma-separated `CHAT_ALLOWED_ORIGINS`.
    ///
    /// Defaults are `127.0.0.1:3008` and the Vite origins on localhost port 3007.
    /// Invalid, non-loopback, wildcard, and opaque origins are rejected.
    pub fn from_env() -> Result<Self, ConfigError> {
        let bind = read_env("CHAT_BIND_ADDR", "127.0.0.1:3008")?;
        let origins = read_env(
            "CHAT_ALLOWED_ORIGINS",
            "http://127.0.0.1:3007,http://localhost:3007",
        )?;
        Self::new(&bind, &origins)
    }

    /// Validate configuration without mutating process environment.
    ///
    /// Port zero is supported for isolated tests. Every configured origin must
    /// be an exact HTTP(S) origin with no credentials, path, query, or fragment.
    pub fn new(bind_addr: &str, origins: &str) -> Result<Self, ConfigError> {
        let bind_addr: SocketAddr = bind_addr
            .parse()
            .map_err(|_| ConfigError("CHAT_BIND_ADDR must be an IP socket address"))?;
        if !bind_addr.ip().is_loopback() {
            return Err(ConfigError("CHAT_BIND_ADDR must use a loopback address"));
        }
        let origins: Vec<_> = origins.split(',').map(str::trim).collect();
        if origins.is_empty() || origins.len() > 16 {
            return Err(ConfigError("CHAT_ALLOWED_ORIGINS requires 1 to 16 origins"));
        }
        let origins = origins
            .into_iter()
            .map(|origin| {
                let uri: Uri = origin
                    .parse()
                    .map_err(|_| ConfigError("CHAT_ALLOWED_ORIGINS contains an invalid origin"))?;
                let valid_scheme = matches!(uri.scheme_str(), Some("http" | "https"));
                let valid_authority = uri.authority().is_some_and(|authority| {
                    !authority.as_str().contains(['@', '*']) && !authority.host().is_empty()
                });
                // Comparing the origin form excludes even a trailing slash.
                let origin_form = format!(
                    "{}://{}",
                    uri.scheme_str().unwrap_or_default(),
                    uri.authority().map(|a| a.as_str()).unwrap_or_default()
                );
                if !valid_scheme || !valid_authority || origin != origin_form {
                    return Err(ConfigError(
                        "CHAT_ALLOWED_ORIGINS requires exact HTTP(S) origins",
                    ));
                }
                HeaderValue::from_str(origin)
                    .map_err(|_| ConfigError("CHAT_ALLOWED_ORIGINS contains an invalid header"))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { bind_addr, origins })
    }

    /// Requested loopback socket address; the bound listener exposes the actual port.
    pub fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }
}

fn read_env(name: &str, default: &str) -> Result<String, ConfigError> {
    match std::env::var(name) {
        Ok(value) => Ok(value),
        Err(std::env::VarError::NotPresent) => Ok(default.to_owned()),
        Err(std::env::VarError::NotUnicode(_)) => Err(ConfigError("configuration must be Unicode")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_and_exact_origins_are_accepted() {
        assert!(
            ServerConfig::new("127.0.0.1:0", "http://localhost:3007,https://chat.example").is_ok()
        );
        assert!(ServerConfig::new("[::1]:0", "http://[::1]:3007").is_ok());
        assert!(ServerConfig::new("0.0.0.0:3008", "http://localhost:3007").is_err());
        for origin in [
            "",
            "*",
            "null",
            "https://*.example",
            "http://a/",
            "http://a/path",
            "http://a?q=1",
            "http://a#x",
            "http://user:pass@a",
            "file://a",
            "http://a\r\nx: y",
        ] {
            assert!(
                ServerConfig::new("127.0.0.1:0", origin).is_err(),
                "{origin:?}"
            );
        }
    }
}
