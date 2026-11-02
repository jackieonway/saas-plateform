//! Capture read-only catalog metadata for authenticated business backups.
use crate::{
    config::Config,
    native::{self, Scratch},
    Result,
};
pub(crate) fn capture(c: &Config, s: &Scratch, service: &str, db: Option<&str>) -> Result<String> {
    native::sql(c, s, service, db, include_str!("../catalog.sql"))
}
