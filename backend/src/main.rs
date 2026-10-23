mod app_authorization;
mod app_token;
mod application_entries;
mod artifact_distribution;
mod authorization;
mod bootstrap;
mod client_ip;
mod confidential_clients;
mod config;
mod data_grants;
mod database;
mod error;
mod extractors;
mod login_limits;
mod managed_deployments;
mod micro_proxy;
mod notifications;
mod oauth_issuance;
mod observability;
mod platform_settings;
mod public_contract;
mod region_seed;
mod region_selection;
mod release_archive_maintenance;
mod release_archives;
mod release_operations;
mod releases;
mod routes;
mod seed;
mod session;
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
mod production_security_tests;

#[cfg(test)]
mod platform_settings_tests;

#[cfg(test)]
mod entity_creation_tests;

#[cfg(test)]
mod platform_session_refresh_tests;

#[cfg(test)]
mod oauth_issuance_tests;

#[cfg(test)]
mod login_protection_tests;

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
use bootstrap::normalize_client_ip;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    bootstrap::run().await
}
