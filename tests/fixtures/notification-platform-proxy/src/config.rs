use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use reqwest::Url;

use crate::Error;

/// Validated loopback addresses. The upstream must be an HTTP origin using a
/// literal loopback IP, without credentials, path, query, or fragment.
#[derive(Clone)]
pub struct Config {
    pub(crate) upstream: Url,
    pub(crate) bind: SocketAddr,
}

impl Config {
    /// Read mandatory `PHASE23_UPSTREAM` and optional `PHASE23_PROXY_PORT` (0).
    /// Rejects invalid/unsafe values using messages that contain no input text.
    pub fn from_env() -> Result<Self, Error> {
        let upstream = std::env::var("PHASE23_UPSTREAM")
            .map_err(|error| Error::with_source("PHASE23_UPSTREAM is required", error))?;
        let port = std::env::var("PHASE23_PROXY_PORT").unwrap_or_else(|_| "0".into());
        Self::parse(&upstream, &port)
    }

    /// Validate an explicit upstream origin and port for an isolated test run.
    /// The listener always binds IPv4 loopback, including when port 0 is used.
    pub fn parse(upstream: &str, port: &str) -> Result<Self, Error> {
        let url = Url::parse(upstream)
            .map_err(|error| Error::with_source("invalid PHASE23_UPSTREAM", error))?;
        let address = url
            .host_str()
            .and_then(|host| host.trim_matches(['[', ']']).parse::<IpAddr>().ok());
        if url.scheme() != "http"
            || !address.is_some_and(|address| address.is_loopback())
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(Error::message(
                "PHASE23_UPSTREAM must be a literal loopback HTTP origin",
            ));
        }
        let port = port
            .parse::<u16>()
            .map_err(|error| Error::with_source("invalid PHASE23_PROXY_PORT", error))?;
        Ok(Self {
            upstream: url,
            bind: SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_is_fixed_to_loopback_origin() {
        for address in ["http://127.0.0.1:3210", "http://[::1]:3210"] {
            assert!(Config::parse(address, "0").is_ok());
        }
        for address in [
            "https://127.0.0.1:3210",
            "http://localhost:3210",
            "http://192.168.1.1:3210",
            "http://0.0.0.0:3210",
            "http://127.0.0.1:3210/other",
            "http://secret@127.0.0.1:3210",
            "http://127.0.0.1:3210?secret=value",
            "http://127.0.0.1:3210#value",
        ] {
            let error = Config::parse(address, "0").err().unwrap();
            assert!(!format!("{error:?}").contains("secret"));
        }
        assert!(Config::parse("http://127.0.0.1", "65536").is_err());
    }
}
