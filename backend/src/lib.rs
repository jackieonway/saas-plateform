//! Narrow authorization, paging, retention and database initialization interfaces.
//! Application models and HTTP handlers stay private to the executable adapter.
pub mod authorization_policy;
pub mod database;
pub mod paging;
pub mod retention;
pub mod snapshot;
