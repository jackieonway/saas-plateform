//! One-node immutable installation. No subprocess, Docker, remote URL or shell execution.
mod config;
mod server;
mod storage;

/// Safe outward codes retain underlying errors for library callers without printing inputs.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Json(serde_json::Error),
    Contract(deployment_contract::zip::Error),
    Policy,
    Authentication,
    Busy,
    Missing,
    Conflict,
    Corrupt,
    Unsupported,
}
impl Error {
    /// Stable code suitable for HTTP and service logs; never includes secrets or raw paths.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io(_) => "NODE_IO_FAILED",
            Self::Json(_) => "INVALID_JSON",
            Self::Contract(deployment_contract::zip::Error::Digest) => "DIGEST_MISMATCH",
            Self::Contract(_) | Self::Policy => "POLICY_REJECTED",
            Self::Authentication => "UNAUTHORIZED",
            Self::Busy => "NODE_BUSY",
            Self::Missing => "ARTIFACT_MISSING",
            Self::Conflict => "VERSION_CONFLICT",
            Self::Corrupt => "ARTIFACT_CORRUPT",
            Self::Unsupported => "UNSUPPORTED_PLATFORM",
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Json(e) => Some(e),
            Self::Contract(e) => Some(e),
            _ => None,
        }
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
impl From<deployment_contract::zip::Error> for Error {
    fn from(e: deployment_contract::zip::Error) -> Self {
        Self::Contract(e)
    }
}

/// Serve only a configured loopback listener behind the node's authenticated HTTPS gateway.
/// Linux RENAME_NOREPLACE is required; unsupported systems fail before listening.
pub async fn run(path: &std::path::Path) -> Result<(), Error> {
    if !cfg!(target_os = "linux") {
        return Err(Error::Unsupported);
    }
    server::serve(config::Config::load(path)?).await
}
