mod app_authorization;
mod app_capabilities;
mod app_organizations;
mod app_token;
mod application_entries;
mod artifact_distribution;
mod auth_review;
mod authentication;
mod authorization;
mod bff;
mod bootstrap;
mod client_ip;
mod confidential_clients;
mod config;
mod context_selection_http;
mod credential_binding;
mod customer_portal;
mod data_grants;
mod directory_authorization;
use saas_platform_backend::database;
mod error;
mod extractors;
mod identity_policy;
mod login_limits;
mod managed_deployments;
mod membership_lifecycle;
mod micro_proxy;
mod notification_context;
mod notifications;
mod observability;
mod oidc;
mod platform_accounts;
mod platform_settings;
mod public_contract;
mod region_seed;
mod region_selection;
mod release_archive_maintenance;
mod release_archives;
mod release_operations;
mod releases;
mod resource_authority;
mod routes;
mod seed;
mod session;
mod sso;
mod state;
mod static_files;
mod util;

#[cfg(test)]
mod scope_tests;

#[cfg(test)]
mod release_tests;

#[cfg(test)]
mod notification_tests;

#[cfg(test)]
mod notification_identity_tests;

#[cfg(test)]
mod production_security_tests;

#[cfg(test)]
mod platform_settings_tests;

#[cfg(test)]
mod entity_creation_tests;

#[cfg(test)]
mod membership_lifecycle_tests;

#[cfg(test)]
mod identity_binding_tests;
#[cfg(test)]
mod identity_ui_tests;

#[cfg(test)]
mod customer_portal_tests;
#[cfg(test)]
mod identity_deployment_tests;

#[cfg(test)]
mod authentication_boundary_tests;

#[cfg(test)]
mod sso_session_tests;

#[cfg(test)]
mod bff_tests;
#[cfg(test)]
mod protocol_retirement_tests;

#[cfg(test)]
mod sso_only_tests;

#[cfg(test)]
mod sso_test_fixture;

#[cfg(test)]
mod employee_isolation_tests;

#[cfg(test)]
mod platform_session_refresh_tests;

#[cfg(test)]
mod management_audit_tests;

#[cfg(test)]
mod data_grant_tests;

#[cfg(test)]
mod data_grant_discovery_tests;

#[cfg(test)]
mod platform_admin_cli_tests;

#[cfg(test)]
mod authorization_isolation_tests;

#[cfg(test)]
mod organization_subtree_user_role_tests;

#[cfg(test)]
mod organization_subtree_reporting_tests;

#[cfg(test)]
mod public_contract_tests;

#[cfg(test)]
mod app_organizations_tests;

#[cfg(test)]
use bootstrap::normalize_client_ip;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    bootstrap::run().await
}
