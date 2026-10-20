//! Narrow, framework-independent authorization, paging and retention use cases.
//! Database models and HTTP handlers stay private to the executable adapter.
pub mod authorization_policy;
pub mod paging;
pub mod retention;
pub mod snapshot;
