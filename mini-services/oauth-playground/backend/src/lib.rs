//! A standalone OAuth demonstration API. Credentials and tokens remain in its
//! bounded, process-local sessions; the browser receives summaries only.
mod config;
mod http;
mod session;
mod upstream;

pub use config::{Config, ConfigError};
pub use http::app;
