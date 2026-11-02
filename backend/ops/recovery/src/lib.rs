//! Coordinated encrypted backups and authenticated snapshot verification.
//! Native PostgreSQL tools run with deadlines. Database restoration is refused
//! before any configuration access; canonical identity recovery has its own tool.
mod bundle;
mod config;
mod native;
mod schema;
mod workflow;

pub use workflow::execute;

/// Redacted, finite errors: native stderr, URLs and configuration never escape.
#[derive(Debug)]
pub struct Error(pub(crate) &'static str);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;
fn io<T>(value: std::io::Result<T>) -> Result<T> {
    value.map_err(|_| Error("filesystem operation failed"))
}
