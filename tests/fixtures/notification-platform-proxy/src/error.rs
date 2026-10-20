use std::{error, fmt};

/// Sanitized startup/runtime failure. Formatting never exposes configuration,
/// credentials, request headers, response bodies, or network error text.
pub struct Error {
    message: &'static str,
    source: Option<Box<dyn error::Error + Send + Sync>>,
}

impl Error {
    pub(crate) fn message(message: &'static str) -> Self {
        Self {
            message,
            source: None,
        }
    }

    pub(crate) fn with_source(
        message: &'static str,
        source: impl error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            message,
            source: Some(Box::new(source)),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(self.message)
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, output)
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.source.as_deref().map(|value| value as _)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting_does_not_disclose_sources() {
        let error = Error::with_source("upstream failed", std::io::Error::other("secret-bearer"));
        assert_eq!(format!("{error}"), "upstream failed");
        assert_eq!(format!("{error:?}"), "upstream failed");
        assert!(std::error::Error::source(&error).is_some());
    }
}
