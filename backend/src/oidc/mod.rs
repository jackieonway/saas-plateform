//! Authenticated management proxy for the sole external identity authority.
//! This module never signs OAuth/OIDC tokens or serves a protocol endpoint.
mod remote_admin;
pub(crate) use remote_admin::routes as remote_admin_routes;
