//! Bounded, loopback-only test proxy for Phase 23's real platform checks.
//!
//! A delay gate controls timing, never authorization. Every forwarded request
//! retains its bearer and must be accepted by the configured real platform.

mod config;
mod error;
mod gates;
mod proxy;
mod runtime;

pub use config::Config;
pub use error::Error;
pub use runtime::run;
