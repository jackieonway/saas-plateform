//! Registered customer self-profile access, separate from employee authorization.
//!
//! Ordinary OAuth access tokens retain the existing issuance and revocation
//! contract. This resource adapter adds an exact server-owned purpose and entry.

mod dto;
mod proof;
mod routes;

pub(crate) use routes::routes;

const CONTRACT_VERSION: &str = "customer-profile/1";
const PURPOSE: &str = "CUSTOMER_PROFILE";
const ENTRY_PATH: &str = "/api/customer/profile";
