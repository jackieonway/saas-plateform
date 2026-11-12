//! Explicit, audited management of one organization identity at a time.
//!
//! Writes stay disabled outside isolated test environments until the complete
//! isolation baseline passes. Credentials revoke by exact membership generation.

mod routes;
mod service;

pub(crate) use routes::routes;
pub(crate) use service::bind_account_by_phone;
pub(crate) use service::cleanup_employee_roles_after_departure_in_tx;
pub(crate) use service::ensure_employee_creation_allowed_in_tx;
pub(crate) use service::{
    validate_member_phone, validate_new_account_membership, NewAccountMembership,
};
