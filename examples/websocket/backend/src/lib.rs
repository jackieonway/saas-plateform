//! Local Socket.IO chat reference. This crate has no platform identity or notification API.

mod config;
mod room;
mod server;

pub use config::{ConfigError, ServerConfig};
pub use server::serve;
