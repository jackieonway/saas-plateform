//! Authenticated management proxy for the sole external identity authority.
//! This module never signs OAuth/OIDC tokens or serves a protocol endpoint.
mod remote_admin;
pub(crate) use remote_admin::routes as remote_admin_routes;
#[cfg(test)]
pub(crate) use remote_admin::{review_current, review_sync, Failure as ReviewSyncFailure};
