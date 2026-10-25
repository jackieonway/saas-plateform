use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

/// Runtime configuration deliberately has no Debug implementation: it contains secrets.
#[derive(Clone)]
#[allow(dead_code)]
pub struct Config {
    pub app_env: String,
    pub database_url: String,
    pub db_max_connections: u32,
    pub db_acquire_timeout_ms: u64,
    pub db_statement_timeout_ms: u64,
    pub db_lock_timeout_ms: u64,
    pub app_authorization_timeout_ms: u64,
    pub app_authorization_max_concurrent: usize,
    pub auto_create_database: bool,
    pub(crate) imported_database: Option<crate::database::ImportedDatabase>,
    pub bind_addr: String,
    pub jwt_secret: String,
    pub metrics_token: String,
    pub bootstrap_admin_username: Option<String>,
    pub bootstrap_admin_password: Option<String>,
    pub session_ttl_hours: i64,
    pub credential_ticket_ttl_minutes: i64,
    pub access_token_ttl_seconds: i64,
    pub refresh_token_ttl_seconds: i64,
    pub public_app_url: String,
    pub application_environment: String,
    pub micro_gateway_origin: String,
    pub micro_allowed_origins: Vec<String>,
    pub cors_allowed_origins: Vec<String>,
    pub trusted_proxy_ips: Vec<IpAddr>,
    pub(crate) data_grant_policy: crate::data_grants::policy::Policy,
    pub password_concurrency: usize,
    pub password_queue_capacity: usize,
    pub password_queue_timeout_ms: u64,
    pub password_execution_timeout_ms: u64,
    pub login_max_entries: usize,
    pub login_window_seconds: u64,
    pub login_source_requests: u32,
    pub login_user_failures: u32,
    pub login_identifier_max_bytes: usize,
    pub login_verify_budget_ms: u64,
    pub micro_ca_cert_file: Option<String>,
    pub micro_ca_certificates: Vec<reqwest::Certificate>,
    pub oauth_introspection_client_requests_per_minute: usize,
    pub oauth_auth_failure_requests_per_minute: usize,
    pub oauth_introspection_attempts_per_minute: usize,
    pub oauth_auto_purge: bool,
    /// Writes outside isolated test fixtures require a verified Phase 6 receipt.
    pub membership_lifecycle_enabled: bool,
    pub customer_memberships_enabled: bool,
    pub dual_memberships_enabled: bool,
    pub(crate) membership_readiness: Option<crate::database::MembershipReadiness>,
    /// Explicit CUSTOMER_PROFILE registration uses internal OAuth client IDs.
    pub customer_profile_client_ref: Option<uuid::Uuid>,
    pub customer_profile_application_id: Option<uuid::Uuid>,
    pub customer_portal_port_enabled: bool,
    pub customer_portal_oidc_client_id: Option<uuid::Uuid>,
    /// Independent identity deployment never falls back to local credentials.
    pub identity_remote_enabled: bool,
    pub identity_service_url: Option<String>,
    pub identity_service_client_id: Option<String>,
    pub identity_service_client_secret: Option<String>,
    pub identity_service_ca_file: Option<String>,
    pub customer_authority_service_client_id: Option<String>,
    pub customer_authority_service_client_secret: Option<String>,
    pub identity_admin_service_client_id: Option<String>,
    pub identity_admin_service_client_secret: Option<String>,
    pub(crate) identity_http_client: Option<reqwest::Client>,
    pub sso_enabled: bool,
    pub sso_origin: Option<String>,
    pub sso_session_ttl_seconds: i64,
    pub sso_preauth_ttl_seconds: i64,
    pub oidc_enabled: bool,
    pub oidc_issuer: Option<String>,
    pub(crate) oidc_keys: Option<Arc<crate::oidc::KeySet>>,
    pub oidc_code_ttl_seconds: i64,
    pub oidc_token_ttl_seconds: i64,
    pub oidc_refresh_ttl_seconds: i64,
    pub oidc_interaction_ttl_seconds: i64,
    pub bff_enabled: bool,
    pub bff_legacy_login_enabled: bool,
    pub bff_legacy_human_clients: Vec<String>,
    pub bff_oidc_issuer: Option<String>,
    pub bff_oidc_client_id: Option<uuid::Uuid>,
    pub bff_oidc_client_secret: Option<String>,
    pub bff_oidc_redirect_uri: Option<String>,
    pub bff_authority_timeout_ms: u64,
    pub bff_max_concurrent: usize,
    pub bff_context_ttl_seconds: i64,
    pub bff_login_ttl_seconds: i64,
    pub(crate) bff_token_keys: Option<Arc<crate::bff::CipherKeys>>,
    pub(crate) bff_http_client: Option<reqwest::Client>,
    pub static_dir: String,
    pub micro_apps_dir: String,
}

fn value(env: &HashMap<String, String>, key: &str, fallback: &str) -> String {
    env.get(key)
        .filter(|v| !v.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| fallback.into())
}

fn optional(env: &HashMap<String, String>, key: &str) -> Option<String> {
    env.get(key).filter(|v| !v.trim().is_empty()).cloned()
}

fn optional_v7(env: &HashMap<String, String>, key: &str) -> anyhow::Result<Option<uuid::Uuid>> {
    env.get(key)
        .map(|raw| {
            let id = uuid::Uuid::parse_str(raw)
                .map_err(|_| anyhow::anyhow!("{key} must be a canonical UUIDv7"))?;
            anyhow::ensure!(
                crate::util::uuid7::is_uuid_v7(id) && id.to_string() == *raw,
                "{key} must be a canonical UUIDv7"
            );
            Ok(id)
        })
        .transpose()
}

fn number(
    env: &HashMap<String, String>,
    key: &str,
    fallback: i64,
    min: i64,
    max: i64,
) -> anyhow::Result<i64> {
    let raw = value(env, key, &fallback.to_string());
    let n: i64 = raw
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("{key} must be an integer"))?;
    anyhow::ensure!(
        (min..=max).contains(&n),
        "{key} is outside the supported range"
    );
    Ok(n)
}

fn boolean(env: &HashMap<String, String>, key: &str, fallback: bool) -> anyhow::Result<bool> {
    match optional(env, key).map(|s| s.trim().to_ascii_lowercase()) {
        None => Ok(fallback),
        Some(raw) if matches!(raw.as_str(), "true" | "1" | "yes" | "on") => Ok(true),
        Some(raw) if matches!(raw.as_str(), "false" | "0" | "no" | "off") => Ok(false),
        _ => anyhow::bail!("{key} must be a boolean"),
    }
}

fn origins(raw: &str, key: &str, production: bool) -> anyhow::Result<Vec<String>> {
    let mut result = Vec::new();
    for origin in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let url = reqwest::Url::parse(origin)
            .map_err(|_| anyhow::anyhow!("{key} must contain exact origins"))?;
        anyhow::ensure!(
            url.origin().ascii_serialization() == origin
                && url.username().is_empty()
                && url.password().is_none()
                && matches!(url.scheme(), "http" | "https")
                && (!production || url.scheme() == "https"),
            "{key} must contain exact {} origins without paths or credentials",
            if production { "HTTPS" } else { "HTTP(S)" }
        );
        if !result.iter().any(|item| item == origin) {
            result.push(origin.to_owned());
        }
    }
    Ok(result)
}

fn strong_secret(raw: &str, key: &str) -> anyhow::Result<()> {
    let lower = raw.to_ascii_lowercase();
    let unique: HashSet<u8> = raw.bytes().collect();
    anyhow::ensure!(
        raw.len() >= 32
            && raw.len() <= 4096
            && unique.len() >= 8
            && !raw.chars().any(char::is_whitespace)
            && !raw.chars().any(char::is_control)
            && ![
                "change-me",
                "change_me",
                "replace-me",
                "replace_me",
                "dev-secret",
                "example-secret"
            ]
            .iter()
            .any(|word| lower.contains(word)),
        "{key} must be an explicit random secret of at least 32 bytes, not a placeholder"
    );
    Ok(())
}

fn bff_http_client(env: &HashMap<String, String>) -> anyhow::Result<Option<reqwest::Client>> {
    if !boolean(env, "BFF_ENABLED", false)? {
        return Ok(None);
    }
    let mut builder = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .connect_timeout(std::time::Duration::from_secs(1))
        .timeout(std::time::Duration::from_millis(number(
            env,
            "BFF_AUTHORITY_TIMEOUT_MS",
            3000,
            100,
            10000,
        )? as u64))
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .user_agent("SaaSPlatform-BFF/1");
    if let Some(path) = optional(env, "BFF_CA_CERT_FILE") {
        anyhow::ensure!(
            Path::new(&path).is_absolute(),
            "BFF_CA_CERT_FILE must be absolute"
        );
        let bytes =
            std::fs::read(&path).map_err(|_| anyhow::anyhow!("BFF_CA_CERT_FILE cannot be read"))?;
        anyhow::ensure!(bytes.len() <= 1024 * 1024, "BFF_CA_CERT_FILE is too large");
        let certificates = reqwest::Certificate::from_pem_bundle(&bytes)
            .map_err(|_| anyhow::anyhow!("BFF_CA_CERT_FILE must contain valid certificates"))?;
        anyhow::ensure!(
            !certificates.is_empty(),
            "BFF_CA_CERT_FILE has no certificates"
        );
        for certificate in certificates {
            builder = builder.add_root_certificate(certificate);
        }
    }
    Ok(Some(builder.build().map_err(|_| {
        anyhow::anyhow!("BFF HTTPS client configuration is invalid")
    })?))
}

fn validate_identity_secret(secret: Option<&str>) -> anyhow::Result<()> {
    anyhow::ensure!(
        secret.is_some_and(|value| value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))),
        "IDENTITY_SERVICE_SECRET_MUST_BE_ISSUED_OPAQUE_SECRET"
    );
    Ok(())
}

fn identity_http_client(env: &HashMap<String, String>) -> anyhow::Result<Option<reqwest::Client>> {
    if !boolean(env, "IDENTITY_REMOTE_ENABLED", false)? {
        return Ok(None);
    }
    let path = optional(env, "IDENTITY_SERVICE_CA_FILE")
        .ok_or_else(|| anyhow::anyhow!("IDENTITY_SERVICE_CA_FILE_REQUIRED"))?;
    anyhow::ensure!(
        Path::new(&path).is_absolute(),
        "IDENTITY_SERVICE_CA_FILE_MUST_BE_ABSOLUTE"
    );
    let bytes =
        std::fs::read(&path).map_err(|_| anyhow::anyhow!("IDENTITY_SERVICE_CA_FILE_UNREADABLE"))?;
    anyhow::ensure!(
        !bytes.is_empty() && bytes.len() <= 1024 * 1024,
        "IDENTITY_SERVICE_CA_FILE_SIZE_INVALID"
    );
    let certificates = reqwest::Certificate::from_pem_bundle(&bytes)
        .map_err(|_| anyhow::anyhow!("IDENTITY_SERVICE_CA_FILE_INVALID"))?;
    anyhow::ensure!(!certificates.is_empty(), "IDENTITY_SERVICE_CA_FILE_INVALID");
    let mut builder = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(std::time::Duration::from_secs(1))
        .timeout(std::time::Duration::from_secs(3))
        .user_agent("SaaSPlatform-Identity/1");
    for certificate in certificates {
        builder = builder.add_root_certificate(certificate);
    }
    Ok(Some(builder.build().map_err(|_| {
        anyhow::anyhow!("IDENTITY_SERVICE_TRANSPORT_INVALID")
    })?))
}

pub fn production_mode(app_env: &str, application_env: &str) -> bool {
    app_env.trim().eq_ignore_ascii_case("production")
        || application_env.trim().eq_ignore_ascii_case("production")
}

/// OS/service values win. Production never searches ancestor directories for .env.
pub fn load_environment() -> anyhow::Result<()> {
    if let Ok(path) = std::env::var("APP_ENV_FILE") {
        anyhow::ensure!(
            Path::new(&path).is_absolute(),
            "APP_ENV_FILE must be an absolute path"
        );
        dotenvy::from_path(&path).map_err(|_| anyhow::anyhow!("APP_ENV_FILE cannot be loaded"))?;
        return Ok(());
    }
    if production_mode(
        &std::env::var("APP_ENV").unwrap_or_default(),
        &std::env::var("APPLICATION_ENVIRONMENT").unwrap_or_default(),
    ) {
        return Ok(());
    }
    let path = Path::new(".env");
    if path.is_file() {
        let items = dotenvy::from_path_iter(path)
            .map_err(|_| anyhow::anyhow!("local environment file cannot be read"))?
            .collect::<Result<HashMap<_, _>, _>>()
            .map_err(|_| anyhow::anyhow!("local environment file cannot be parsed"))?;
        let effective = |key: &str| {
            std::env::var(key)
                .ok()
                .or_else(|| items.get(key).cloned())
                .unwrap_or_default()
        };
        anyhow::ensure!(
            !production_mode(&effective("APP_ENV"), &effective("APPLICATION_ENVIRONMENT")),
            "production requires OS environment or explicit APP_ENV_FILE"
        );
        dotenvy::from_path(path)
            .map_err(|_| anyhow::anyhow!("local environment file cannot be loaded"))?;
    }
    Ok(())
}

impl Config {
    /// Existing in-process tests use development defaults and replace fixture values.
    #[cfg(test)]
    pub fn from_env() -> Self {
        Self::try_from_env().expect("invalid runtime configuration")
    }
    pub fn try_from_env() -> anyhow::Result<Self> {
        let values = std::env::vars_os()
            .map(|(key, value)| {
                let invalid = || anyhow::anyhow!("CONFIGURATION_VALUES_MUST_BE_UTF8");
                Ok((
                    key.into_string().map_err(|_| invalid())?,
                    value.into_string().map_err(|_| invalid())?,
                ))
            })
            .collect::<anyhow::Result<HashMap<_, _>>>()?;
        Self::from_values(&values)
    }

    pub fn from_values(env: &HashMap<String, String>) -> anyhow::Result<Self> {
        let initialization_mode = env.get("PLATFORM_DATABASE_INIT_MODE").map(String::as_str);
        crate::database::ensure_server_initialization_mode(initialization_mode)?;
        let receipt_keys = [
            "PLATFORM_EXPECTED_DATABASE",
            "PLATFORM_IMPORT_MARKER",
            "PLATFORM_EXPECTED_SCHEMA_SHA256",
        ];
        let imported_database = if initialization_mode == Some("imported") {
            let required = |key| {
                env.get(key)
                    .map(String::as_str)
                    .ok_or_else(|| anyhow::anyhow!("IMPORTED_DATABASE_CONFIGURATION_INVALID"))
            };
            Some(crate::database::ImportedDatabase::new(
                required(receipt_keys[0])?,
                required(receipt_keys[1])?,
                required(receipt_keys[2])?,
            )?)
        } else {
            anyhow::ensure!(
                !receipt_keys.iter().any(|key| env.contains_key(*key)),
                "IMPORTED_DATABASE_FIELDS_REQUIRE_IMPORTED_MODE"
            );
            None
        };
        let app_env = value(env, "APP_ENV", "development");
        let application_environment = value(env, "APPLICATION_ENVIRONMENT", "development");
        let production = production_mode(&app_env, &application_environment);
        let membership_readiness = match (
            env.get("MEMBERSHIP_READINESS_RECEIPT"),
            env.get("MEMBERSHIP_READINESS_MANIFEST_SHA256"),
        ) {
            (None, None) => None,
            (Some(receipt), Some(manifest)) => Some(crate::database::MembershipReadiness::new(
                receipt,
                manifest,
                if production {
                    "production"
                } else {
                    app_env.as_str()
                },
            )?),
            _ => anyhow::bail!("MEMBERSHIP_READINESS_CONFIGURATION_INVALID"),
        };
        anyhow::ensure!(
            matches!(
                app_env.trim().to_ascii_lowercase().as_str(),
                "development" | "test" | "production"
            ),
            "APP_ENV must be development, test or production"
        );
        let public_app_url = value(env, "PUBLIC_APP_URL", "");
        let micro_gateway_origin = value(
            env,
            "MICRO_GATEWAY_ORIGIN",
            if public_app_url.is_empty() {
                "http://localhost:3000"
            } else {
                &public_app_url
            },
        );
        let micro_allowed_origins = origins(
            &value(env, "MICRO_ALLOWED_ORIGINS", &micro_gateway_origin),
            "MICRO_ALLOWED_ORIGINS",
            production,
        )?;
        let cors_allowed_origins = origins(
            &value(
                env,
                "CORS_ALLOWED_ORIGINS",
                if public_app_url.is_empty() {
                    "http://localhost:3000,http://localhost:5173,http://127.0.0.1:3000,http://127.0.0.1:5173"
                } else {
                    &public_app_url
                },
            ),
            "CORS_ALLOWED_ORIGINS",
            production,
        )?;
        let trusted_proxy_ips = value(env, "TRUSTED_PROXY_IPS", "")
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|ip| {
                ip.parse::<IpAddr>().map_err(|_| {
                    anyhow::anyhow!("TRUSTED_PROXY_IPS must contain exact IP addresses")
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let micro_ca_cert_file = optional(env, "MICRO_CA_CERT_FILE");
        let micro_ca_certificates = match &micro_ca_cert_file {
            Some(path) => {
                anyhow::ensure!(
                    Path::new(path).is_absolute(),
                    "MICRO_CA_CERT_FILE must be an absolute path"
                );
                let bytes = std::fs::read(path)
                    .map_err(|_| anyhow::anyhow!("MICRO_CA_CERT_FILE cannot be read"))?;
                anyhow::ensure!(
                    bytes.len() <= 1024 * 1024,
                    "MICRO_CA_CERT_FILE is too large"
                );
                let certs = reqwest::Certificate::from_pem_bundle(&bytes).map_err(|_| {
                    anyhow::anyhow!("MICRO_CA_CERT_FILE must contain valid PEM certificates")
                })?;
                anyhow::ensure!(!certs.is_empty(), "MICRO_CA_CERT_FILE has no certificates");
                let mut builder = reqwest::Client::builder().no_proxy();
                for certificate in &certs {
                    builder = builder.add_root_certificate(certificate.clone());
                }
                builder.build().map_err(|_| {
                    anyhow::anyhow!("MICRO_CA_CERT_FILE contains an unusable CA certificate")
                })?;
                certs
            }
            None => Vec::new(),
        };
        let config = Self {
            app_env,
            application_environment,
            database_url: value(
                env,
                "DATABASE_URL",
                "postgres://platform@127.0.0.1:5432/saas_platform",
            ),
            auto_create_database: boolean(env, "AUTO_CREATE_DATABASE", false)?,
            imported_database,
            db_max_connections: number(env, "DB_MAX_CONNECTIONS", 10, 1, 128)? as u32,
            db_acquire_timeout_ms: number(env, "DB_ACQUIRE_TIMEOUT_MS", 5000, 100, 30000)? as u64,
            db_statement_timeout_ms: number(env, "DB_STATEMENT_TIMEOUT_MS", 5000, 100, 60000)?
                as u64,
            db_lock_timeout_ms: number(env, "DB_LOCK_TIMEOUT_MS", 5000, 100, 30000)? as u64,
            app_authorization_timeout_ms: number(
                env,
                "APP_AUTHORIZATION_TIMEOUT_MS",
                5000,
                100,
                30000,
            )? as u64,
            app_authorization_max_concurrent: number(
                env,
                "APP_AUTHORIZATION_MAX_CONCURRENT",
                32,
                1,
                128,
            )? as usize,
            bind_addr: value(env, "BIND_ADDR", "0.0.0.0:3001"),
            jwt_secret: value(env, "JWT_SECRET", "saas-platform-dev-secret-change-me"),
            metrics_token: value(env, "METRICS_TOKEN", ""),
            bootstrap_admin_username: optional(env, "BOOTSTRAP_ADMIN_USERNAME"),
            bootstrap_admin_password: optional(env, "BOOTSTRAP_ADMIN_PASSWORD"),
            session_ttl_hours: number(env, "SESSION_TTL_HOURS", 12, 1, 720)?,
            credential_ticket_ttl_minutes: number(env, "CREDENTIAL_TICKET_TTL_MINUTES", 10, 1, 60)?,
            access_token_ttl_seconds: number(env, "ACCESS_TOKEN_TTL_SECONDS", 7200, 1, 86400)?,
            refresh_token_ttl_seconds: number(
                env,
                "REFRESH_TOKEN_TTL_SECONDS",
                604800,
                1,
                7776000,
            )?,
            public_app_url,
            micro_gateway_origin,
            micro_allowed_origins,
            cors_allowed_origins,
            trusted_proxy_ips,
            data_grant_policy: match env.get("DATA_GRANT_POLICY") {
                Some(raw) => crate::data_grants::policy::Policy::from_json(raw)?,
                None => crate::data_grants::policy::Policy::default(),
            },
            password_concurrency: number(env, "PASSWORD_CONCURRENCY", 2, 1, 32)? as usize,
            password_queue_capacity: number(env, "PASSWORD_QUEUE_CAPACITY", 8, 0, 1024)? as usize,
            password_queue_timeout_ms: number(env, "PASSWORD_QUEUE_TIMEOUT_MS", 250, 1, 30000)?
                as u64,
            password_execution_timeout_ms: number(
                env,
                "PASSWORD_EXECUTION_TIMEOUT_MS",
                10000,
                1,
                120000,
            )? as u64,
            login_max_entries: number(env, "LOGIN_MAX_ENTRIES", 4096, 3, 1000000)? as usize,
            login_window_seconds: number(env, "LOGIN_WINDOW_SECONDS", 600, 1, 86400)? as u64,
            login_source_requests: number(env, "LOGIN_SOURCE_REQUESTS", 60, 1, 100000)? as u32,
            login_user_failures: number(env, "LOGIN_USER_FAILURES", 5, 1, 1000)? as u32,
            login_identifier_max_bytes: number(env, "LOGIN_IDENTIFIER_MAX_BYTES", 320, 1, 1024)?
                as usize,
            login_verify_budget_ms: number(env, "LOGIN_VERIFY_BUDGET_MS", 10000, 1, 120000)? as u64,
            micro_ca_cert_file,
            micro_ca_certificates,
            oauth_introspection_client_requests_per_minute: number(
                env,
                "OAUTH_INTROSPECTION_CLIENT_REQUESTS_PER_MINUTE",
                600,
                1,
                100000,
            )? as usize,
            oauth_auth_failure_requests_per_minute: number(
                env,
                "OAUTH_AUTH_FAILURE_REQUESTS_PER_MINUTE",
                30,
                1,
                10000,
            )? as usize,
            oauth_introspection_attempts_per_minute: number(
                env,
                "OAUTH_INTROSPECTION_ATTEMPTS_PER_MINUTE",
                3000,
                1,
                1000000,
            )? as usize,
            oauth_auto_purge: boolean(env, "OAUTH_AUTO_PURGE", false)?,
            membership_lifecycle_enabled: boolean(env, "MEMBERSHIP_LIFECYCLE_ENABLED", false)?,
            customer_memberships_enabled: boolean(env, "CUSTOMER_MEMBERSHIPS_ENABLED", false)?,
            dual_memberships_enabled: boolean(env, "DUAL_MEMBERSHIPS_ENABLED", false)?,
            membership_readiness,
            customer_profile_client_ref: optional_v7(env, "CUSTOMER_PROFILE_CLIENT_REF")?,
            customer_profile_application_id: optional_v7(env, "CUSTOMER_PROFILE_APPLICATION_ID")?,
            customer_portal_port_enabled: boolean(env, "CUSTOMER_PORTAL_PORT_ENABLED", false)?,
            customer_portal_oidc_client_id: optional_v7(env, "CUSTOMER_PORTAL_OIDC_CLIENT_ID")?,
            identity_remote_enabled: boolean(env, "IDENTITY_REMOTE_ENABLED", false)?,
            identity_service_url: optional(env, "IDENTITY_SERVICE_URL"),
            identity_service_client_id: optional_v7(env, "IDENTITY_SERVICE_CLIENT_ID")?
                .map(|id| id.to_string()),
            identity_service_client_secret: optional(env, "IDENTITY_SERVICE_CLIENT_SECRET"),
            identity_service_ca_file: optional(env, "IDENTITY_SERVICE_CA_FILE"),
            customer_authority_service_client_id: optional_v7(
                env,
                "CUSTOMER_AUTHORITY_SERVICE_CLIENT_ID",
            )?
            .map(|id| id.to_string()),
            customer_authority_service_client_secret: optional(
                env,
                "CUSTOMER_AUTHORITY_SERVICE_CLIENT_SECRET",
            ),
            identity_admin_service_client_id: optional_v7(env, "IDENTITY_ADMIN_SERVICE_CLIENT_ID")?
                .map(|id| id.to_string()),
            identity_admin_service_client_secret: optional(
                env,
                "IDENTITY_ADMIN_SERVICE_CLIENT_SECRET",
            ),
            identity_http_client: identity_http_client(env)?,
            sso_enabled: boolean(env, "SSO_ENABLED", false)?,
            sso_origin: optional(env, "SSO_ORIGIN")
                .map(|raw| {
                    let allowed = origins(&raw, "SSO_ORIGIN", true)?;
                    anyhow::ensure!(
                        allowed.len() == 1 && allowed[0] == raw,
                        "SSO_ORIGIN must be one exact HTTPS origin"
                    );
                    Ok::<_, anyhow::Error>(raw)
                })
                .transpose()?,
            sso_session_ttl_seconds: number(env, "SSO_SESSION_TTL_SECONDS", 43200, 1, 86400)?,
            sso_preauth_ttl_seconds: number(env, "SSO_PREAUTH_TTL_SECONDS", 300, 30, 600)?,
            oidc_enabled: boolean(env, "OIDC_ENABLED", false)?,
            oidc_issuer: optional(env, "OIDC_ISSUER"),
            oidc_keys: if boolean(env, "OIDC_ENABLED", false)? {
                Some(Arc::new(crate::oidc::KeySet::load(
                    &optional(env, "OIDC_KEYSET_PATH")
                        .ok_or_else(|| anyhow::anyhow!("OIDC_KEYSET_PATH_REQUIRED"))?,
                )?))
            } else {
                None
            },
            oidc_code_ttl_seconds: number(env, "OIDC_CODE_TTL_SECONDS", 120, 1, 300)?,
            oidc_token_ttl_seconds: number(env, "OIDC_TOKEN_TTL_SECONDS", 300, 1, 600)?,
            oidc_refresh_ttl_seconds: number(env, "OIDC_REFRESH_TTL_SECONDS", 43200, 1, 86400)?,
            oidc_interaction_ttl_seconds: number(
                env,
                "OIDC_INTERACTION_TTL_SECONDS",
                300,
                30,
                600,
            )?,
            bff_enabled: boolean(env, "BFF_ENABLED", false)?,
            bff_legacy_login_enabled: boolean(env, "BFF_LEGACY_LOGIN_ENABLED", false)?,
            bff_legacy_human_clients: value(env, "BFF_LEGACY_HUMAN_CLIENTS", "")
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect(),
            bff_oidc_issuer: optional(env, "BFF_OIDC_ISSUER"),
            bff_oidc_client_id: optional_v7(env, "BFF_OIDC_CLIENT_ID")?,
            bff_oidc_client_secret: optional(env, "BFF_OIDC_CLIENT_SECRET"),
            bff_oidc_redirect_uri: optional(env, "BFF_OIDC_REDIRECT_URI"),
            bff_authority_timeout_ms: number(env, "BFF_AUTHORITY_TIMEOUT_MS", 3000, 100, 10000)?
                as u64,
            bff_max_concurrent: number(env, "BFF_MAX_CONCURRENT", 16, 1, 64)? as usize,
            bff_context_ttl_seconds: number(env, "BFF_CONTEXT_TTL_SECONDS", 300, 1, 300)?,
            bff_login_ttl_seconds: number(env, "BFF_LOGIN_TTL_SECONDS", 300, 30, 600)?,
            bff_token_keys: if boolean(env, "BFF_ENABLED", false)? {
                Some(Arc::new(crate::bff::CipherKeys::load(
                    &optional(env, "BFF_TOKEN_KEY_FILE")
                        .ok_or_else(|| anyhow::anyhow!("BFF_TOKEN_KEY_FILE_REQUIRED"))?,
                )?))
            } else {
                None
            },
            bff_http_client: bff_http_client(env)?,
            static_dir: value(env, "STATIC_DIR", ""),
            micro_apps_dir: value(env, "MICRO_APPS_DIR", ""),
        };
        if config.membership_lifecycle_enabled
            || config.customer_memberships_enabled
            || config.dual_memberships_enabled
        {
            anyhow::ensure!(
                (config.app_env == "test" && !production)
                    || config.membership_readiness.is_some(),
                "MEMBERSHIP_READINESS_REQUIRED: identity writes require an environment-bound Phase 6 receipt"
            );
            anyhow::ensure!(
                config.membership_lifecycle_enabled,
                "MEMBERSHIP_LIFECYCLE_ENABLED is required for customer or dual identity writes"
            );
        }
        anyhow::ensure!(
            config.customer_profile_client_ref.is_some()
                == config.customer_profile_application_id.is_some(),
            "CUSTOMER_PROFILE_REGISTRATION_PAIR_REQUIRED"
        );
        if config.customer_portal_port_enabled {
            anyhow::ensure!(
                (config.oidc_enabled || config.identity_remote_enabled)
                    && config.customer_portal_oidc_client_id.is_some()
                    && config.customer_profile_client_ref.is_some()
                    && config.customer_profile_application_id.is_some(),
                "CUSTOMER_PORTAL_SOURCE_AND_PROFILE_REGISTRATION_REQUIRED"
            );
        }
        if config.identity_remote_enabled {
            anyhow::ensure!(
                !config.sso_enabled
                    && !config.oidc_enabled
                    && !config.auto_create_database
                    && config.imported_database.is_none(),
                "IDENTITY_REMOTE_REQUIRES_SEPARATE_PROVIDER_AND_EXPLICIT_DATABASE"
            );
            let endpoint = config
                .identity_service_url
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("IDENTITY_SERVICE_URL_REQUIRED"))?;
            let allowed = origins(endpoint, "IDENTITY_SERVICE_URL", true)?;
            anyhow::ensure!(
                allowed.len() == 1 && allowed[0] == endpoint,
                "IDENTITY_SERVICE_URL_MUST_BE_EXACT_HTTPS_ORIGIN"
            );
            anyhow::ensure!(
                config.oidc_issuer.as_deref() == Some(endpoint),
                "IDENTITY_REMOTE_REQUIRES_STABLE_ISSUER"
            );
            anyhow::ensure!(
                config.identity_service_client_id.is_some(),
                "IDENTITY_SERVICE_CLIENT_ID_REQUIRED"
            );
            validate_identity_secret(config.identity_service_client_secret.as_deref())?;
            if config.customer_portal_port_enabled {
                anyhow::ensure!(
                    config.customer_authority_service_client_id.is_some(),
                    "CUSTOMER_AUTHORITY_SERVICE_CLIENT_ID_REQUIRED"
                );
                validate_identity_secret(
                    config.customer_authority_service_client_secret.as_deref(),
                )?;
                anyhow::ensure!(
                    config.customer_authority_service_client_id
                        != config.identity_service_client_id,
                    "IDENTITY_SERVICE_PURPOSES_MUST_BE_SEPARATE"
                );
            }
            anyhow::ensure!(
                config.identity_admin_service_client_id.is_some()
                    == config.identity_admin_service_client_secret.is_some(),
                "IDENTITY_ADMIN_SERVICE_PAIR_REQUIRED"
            );
            if config.identity_admin_service_client_id.is_some() {
                validate_identity_secret(config.identity_admin_service_client_secret.as_deref())?;
                anyhow::ensure!(
                    config.identity_admin_service_client_id != config.identity_service_client_id
                        && config.identity_admin_service_client_id
                            != config.customer_authority_service_client_id,
                    "IDENTITY_SERVICE_PURPOSES_MUST_BE_SEPARATE"
                );
            }
        }
        if config.sso_enabled {
            let origin = config
                .sso_origin
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("SSO_ORIGIN_REQUIRED"))?;
            let identity =
                reqwest::Url::parse(origin).map_err(|_| anyhow::anyhow!("SSO_ORIGIN_INVALID"))?;
            let platform_origins = origins(&config.public_app_url, "PUBLIC_APP_URL", false)?;
            anyhow::ensure!(
                platform_origins.len() == 1 && platform_origins[0] == config.public_app_url,
                "PUBLIC_APP_URL must be one exact HTTP(S) origin when SSO is enabled"
            );
            let platform = reqwest::Url::parse(&config.public_app_url)
                .map_err(|_| anyhow::anyhow!("PUBLIC_APP_URL_INVALID"))?;
            anyhow::ensure!(
                identity.host_str() != platform.host_str(),
                "SSO_ORIGIN must use a separate authentication host"
            );
        }
        if config.oidc_enabled {
            anyhow::ensure!(config.sso_enabled, "OIDC_REQUIRES_SSO");
            let issuer = config
                .oidc_issuer
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("OIDC_ISSUER_REQUIRED"))?;
            let allowed = origins(issuer, "OIDC_ISSUER", true)?;
            anyhow::ensure!(
                allowed.len() == 1
                    && allowed[0] == issuer
                    && config.sso_origin.as_deref() == Some(issuer),
                "OIDC_ISSUER must equal the exact HTTPS SSO_ORIGIN"
            );
        }
        anyhow::ensure!(
            config.bff_legacy_human_clients.len() <= 64
                && config
                    .bff_legacy_human_clients
                    .iter()
                    .all(|value| value.len() <= 256 && !value.chars().any(char::is_control)),
            "BFF legacy client allowlist is invalid"
        );
        if config.bff_enabled {
            let platform = origins(&config.public_app_url, "PUBLIC_APP_URL", true)?;
            anyhow::ensure!(
                platform.len() == 1 && platform[0] == config.public_app_url,
                "BFF requires one exact HTTPS PUBLIC_APP_URL"
            );
            let issuer = config
                .bff_oidc_issuer
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("BFF_OIDC_ISSUER_REQUIRED"))?;
            let identity = origins(issuer, "BFF_OIDC_ISSUER", true)?;
            anyhow::ensure!(
                identity.len() == 1 && identity[0] == issuer,
                "BFF_OIDC_ISSUER must be one exact HTTPS origin"
            );
            let platform_url = reqwest::Url::parse(&config.public_app_url)
                .map_err(|_| anyhow::anyhow!("BFF platform origin is invalid"))?;
            let issuer_url = reqwest::Url::parse(issuer)
                .map_err(|_| anyhow::anyhow!("BFF issuer is invalid"))?;
            anyhow::ensure!(
                platform_url.host_str() != issuer_url.host_str(),
                "BFF requires a separate RP hostname"
            );
            anyhow::ensure!(
                config
                    .oidc_issuer
                    .as_deref()
                    .is_none_or(|configured| configured == issuer),
                "BFF issuer differs from this environment's provider issuer"
            );
            anyhow::ensure!(
                config.bff_oidc_client_id.is_some(),
                "BFF_OIDC_CLIENT_ID_REQUIRED"
            );
            let secret = config
                .bff_oidc_client_secret
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("BFF_OIDC_CLIENT_SECRET_REQUIRED"))?;
            anyhow::ensure!(
                secret.len() == 64
                    && secret
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "BFF client secret must be the issued opaque secret"
            );
            anyhow::ensure!(
                config.bff_oidc_redirect_uri.as_deref()
                    == Some(format!("{}/auth/callback", config.public_app_url).as_str()),
                "BFF_OIDC_REDIRECT_URI must equal the RP origin plus /auth/callback"
            );
        }
        if let Some(imported) = &config.imported_database {
            anyhow::ensure!(
                !config.auto_create_database,
                "IMPORTED_DATABASE_AUTO_CREATE_FORBIDDEN"
            );
            anyhow::ensure!(
                config.bootstrap_admin_username.is_none()
                    && config.bootstrap_admin_password.is_none(),
                "IMPORTED_DATABASE_BOOTSTRAP_CREDENTIALS_FORBIDDEN"
            );
            anyhow::ensure!(
                optional(env, "DATABASE_URL").is_some(),
                "IMPORTED_DATABASE_EXPLICIT_URL_REQUIRED"
            );
            imported.validate_url(&config.database_url)?;
            anyhow::ensure!(
                optional(env, "JWT_SECRET").is_some(),
                "IMPORTED_DATABASE_EXPLICIT_JWT_SECRET_REQUIRED"
            );
            strong_secret(&config.jwt_secret, "JWT_SECRET")?;
        }
        config
            .bind_addr
            .parse::<SocketAddr>()
            .map_err(|_| anyhow::anyhow!("BIND_ADDR must be a socket address"))?;
        if !config.micro_apps_dir.is_empty() {
            anyhow::ensure!(
                Path::new(&config.micro_apps_dir).is_absolute(),
                "MICRO_APPS_DIR must be an absolute path"
            );
        }
        if production {
            anyhow::ensure!(
                optional(env, "DATABASE_URL").is_some(),
                "DATABASE_URL is required in production"
            );
            strong_secret(&config.jwt_secret, "JWT_SECRET")?;
            strong_secret(&config.metrics_token, "METRICS_TOKEN")?;
            anyhow::ensure!(
                config.metrics_token.len() <= 256
                    && config
                        .metrics_token
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-'),
                "METRICS_TOKEN must be a URL-safe secret of at most 256 bytes"
            );
            anyhow::ensure!(
                config.metrics_token != config.jwt_secret,
                "METRICS_TOKEN must be independent of JWT_SECRET"
            );
            anyhow::ensure!(
                !config.public_app_url.is_empty(),
                "PUBLIC_APP_URL is required in production"
            );
            anyhow::ensure!(
                origins(&config.public_app_url, "PUBLIC_APP_URL", true)?.len() == 1,
                "PUBLIC_APP_URL must be one exact HTTPS origin"
            );
            anyhow::ensure!(
                origins(&config.micro_gateway_origin, "MICRO_GATEWAY_ORIGIN", true)?.len() == 1,
                "MICRO_GATEWAY_ORIGIN must be one exact HTTPS origin"
            );
            anyhow::ensure!(
                !config.micro_allowed_origins.is_empty() && !config.cors_allowed_origins.is_empty(),
                "production origin allowlists must not be empty"
            );
        }
        let url = reqwest::Url::parse(&config.database_url)
            .map_err(|_| anyhow::anyhow!("DATABASE_URL must be a PostgreSQL URL"))?;
        anyhow::ensure!(
            matches!(url.scheme(), "postgres" | "postgresql"),
            "DATABASE_URL must be a PostgreSQL URL"
        );
        if config.bootstrap_admin_username.is_some() || config.bootstrap_admin_password.is_some() {
            let username = config.bootstrap_admin_username.as_deref().ok_or_else(|| {
                anyhow::anyhow!("BOOTSTRAP_ADMIN_USERNAME is required with bootstrap password")
            })?;
            let password = config.bootstrap_admin_password.as_deref().ok_or_else(|| {
                anyhow::anyhow!("BOOTSTRAP_ADMIN_PASSWORD is required with bootstrap username")
            })?;
            anyhow::ensure!(
                username.len() <= 100
                    && username.trim() == username
                    && !username.chars().any(char::is_control),
                "BOOTSTRAP_ADMIN_USERNAME is invalid"
            );
            anyhow::ensure!(
                password.chars().count() >= 12
                    && password.len() <= 1024
                    && !password.chars().any(char::is_control)
                    && !crate::seed::is_demo_password(password),
                "BOOTSTRAP_ADMIN_PASSWORD must be a new password of at least 12 characters"
            );
            crate::util::password::validate_for_config(password, &config).map_err(|_| {
                anyhow::anyhow!(
                    "BOOTSTRAP_ADMIN_PASSWORD does not satisfy runtime password requirements"
                )
            })?;
        }
        Ok(config)
    }
    pub fn is_production(&self) -> bool {
        production_mode(&self.app_env, &self.application_environment)
    }
    pub fn public_base(&self) -> String {
        self.public_app_url.trim_end_matches('/').to_owned()
    }
}

#[allow(dead_code)]
pub type SharedConfig = Arc<Config>;

#[cfg(test)]
mod tests {
    use super::*;

    fn imported() -> HashMap<String, String> {
        [
            ("PLATFORM_DATABASE_INIT_MODE", "imported"),
            ("PLATFORM_EXPECTED_DATABASE", "isolated_imported_target"),
            ("PLATFORM_IMPORT_MARKER", "saas-uuid-import/1/019a1163-0000-7000-8000-000000000001/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            ("PLATFORM_EXPECTED_SCHEMA_SHA256", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            ("DATABASE_URL", "postgresql://service:private-canary@127.0.0.1:1/isolated_imported_target"),
            ("JWT_SECRET", "correct-Rust-imported-fixture-independent-key-39"),
        ].into_iter().map(|(key, value)| (key.into(), value.into())).collect()
    }

    #[test]
    fn imported_startup_is_explicit_and_rejects_seed_or_connection_conflicts() {
        let valid = imported();
        assert!(Config::from_values(&valid)
            .unwrap()
            .imported_database
            .is_some());
        assert!(Config::from_values(&HashMap::new())
            .unwrap()
            .imported_database
            .is_none());
        for key in [
            "PLATFORM_EXPECTED_DATABASE",
            "PLATFORM_IMPORT_MARKER",
            "PLATFORM_EXPECTED_SCHEMA_SHA256",
            "DATABASE_URL",
            "JWT_SECRET",
        ] {
            let mut invalid = valid.clone();
            invalid.remove(key);
            assert!(
                Config::from_values(&invalid).is_err(),
                "missing required imported setting accepted"
            );
        }
        for (key, value) in [
            ("AUTO_CREATE_DATABASE", "true"),
            ("BOOTSTRAP_ADMIN_USERNAME", "private-canary"),
            ("BOOTSTRAP_ADMIN_PASSWORD", "private-canary"),
            ("JWT_SECRET", "saas-platform-dev-secret-change-me"),
            ("DATABASE_URL", "postgresql://service:private-canary@127.0.0.1:1/old_source"),
            ("DATABASE_URL", "postgresql://service:private-canary@127.0.0.1:1/isolated_imported_target?dbname=old_source"),
        ] {
            let mut invalid = valid.clone();
            invalid.insert(key.into(), value.into());
            let error = Config::from_values(&invalid).err().expect("unsafe imported configuration accepted");
            assert!(!format!("{error:?}").contains("private-canary"));
        }
        for mode in [None, Some("seed")] {
            let mut invalid = valid.clone();
            if let Some(mode) = mode {
                invalid.insert("PLATFORM_DATABASE_INIT_MODE".into(), mode.into());
            } else {
                invalid.remove("PLATFORM_DATABASE_INIT_MODE");
            }
            assert!(Config::from_values(&invalid).is_err());
        }
    }

    fn production() -> HashMap<String, String> {
        [
            ("APP_ENV", "production"),
            ("DATABASE_URL", "postgres://test@localhost/test"),
            ("JWT_SECRET", "correct-Rust-unit-fixture-random-key-37"),
            ("METRICS_TOKEN", "metrics-Rust-unit-fixture-random-key-47"),
            ("PUBLIC_APP_URL", "https://localhost:8443"),
        ]
        .into_iter()
        .map(|(a, b)| (a.into(), b.into()))
        .collect()
    }
    #[test]
    fn production_defaults_are_explicit_and_safe() {
        let c = Config::from_values(&production()).unwrap();
        assert!(c.is_production());
        assert!(!c.oauth_auto_purge);
        assert!(!c.membership_lifecycle_enabled);
        assert!(!c.customer_memberships_enabled);
        assert!(!c.dual_memberships_enabled);
        assert!(!c.auto_create_database);
        assert_eq!(c.cors_allowed_origins, ["https://localhost:8443"]);
        assert_eq!(c.oauth_introspection_client_requests_per_minute, 600);
        assert!(c.bootstrap_admin_password.is_none());
    }

    #[test]
    fn identity_writes_are_closed_until_the_isolation_baseline() {
        for key in [
            "MEMBERSHIP_LIFECYCLE_ENABLED",
            "CUSTOMER_MEMBERSHIPS_ENABLED",
            "DUAL_MEMBERSHIPS_ENABLED",
        ] {
            for mode in ["development", "production"] {
                let mut values = production();
                values.insert("APP_ENV".into(), mode.into());
                values.insert(key.into(), "true".into());
                assert!(Config::from_values(&values).is_err());
            }
            let mut values = HashMap::from([
                ("APP_ENV".into(), "test".into()),
                ("MEMBERSHIP_LIFECYCLE_ENABLED".into(), "true".into()),
                (key.into(), "true".into()),
            ]);
            assert!(Config::from_values(&values).is_ok());
            values.insert("APPLICATION_ENVIRONMENT".into(), "production".into());
            assert!(Config::from_values(&values).is_err());
        }
        let missing_lifecycle = HashMap::from([
            ("APP_ENV".into(), "test".into()),
            ("CUSTOMER_MEMBERSHIPS_ENABLED".into(), "true".into()),
        ]);
        assert!(Config::from_values(&missing_lifecycle).is_err());
    }

    #[test]
    fn phase11_customer_portal_requires_explicit_source_and_profile_registration() {
        let default = Config::from_values(&HashMap::new()).unwrap();
        assert!(!default.customer_portal_port_enabled);
        assert!(default.customer_portal_oidc_client_id.is_none());
        let enabled = HashMap::from([("CUSTOMER_PORTAL_PORT_ENABLED".into(), "true".into())]);
        assert!(Config::from_values(&enabled).is_err());
        for invalid in [
            "019a1163-0000-4000-8000-000000000001",
            "019A1163-0000-7000-8000-000000000001",
            "not-a-client",
        ] {
            let values = HashMap::from([("CUSTOMER_PORTAL_OIDC_CLIENT_ID".into(), invalid.into())]);
            assert!(Config::from_values(&values).is_err());
        }
    }

    #[test]
    fn phase12_remote_identity_is_explicit_and_fails_closed_without_its_ca() {
        let default = Config::from_values(&HashMap::new()).unwrap();
        assert!(!default.identity_remote_enabled);
        assert!(default.identity_http_client.is_none());
        let enabled = HashMap::from([
            ("IDENTITY_REMOTE_ENABLED".into(), "true".into()),
            (
                "IDENTITY_SERVICE_CLIENT_SECRET".into(),
                "private-config-canary".into(),
            ),
        ]);
        let error = Config::from_values(&enabled).err().unwrap().to_string();
        assert!(error.contains("IDENTITY_SERVICE_CA_FILE_REQUIRED"));
        assert!(!error.contains("private-config-canary"));
    }

    #[test]
    fn phase12_service_registrations_require_canonical_v7_ids() {
        for key in [
            "IDENTITY_SERVICE_CLIENT_ID",
            "CUSTOMER_AUTHORITY_SERVICE_CLIENT_ID",
            "IDENTITY_ADMIN_SERVICE_CLIENT_ID",
        ] {
            for invalid in [
                "019a1163-0000-4000-8000-000000000001",
                "019A1163-0000-7000-8000-000000000001",
                "not-a-service",
            ] {
                assert!(
                    Config::from_values(&HashMap::from([(key.into(), invalid.into())])).is_err()
                );
            }
            assert!(Config::from_values(&HashMap::from([(
                key.into(),
                "019a1163-0000-7000-8000-000000000001".into()
            )]))
            .is_ok());
        }
    }

    #[test]
    fn phase12_service_secrets_reject_ambiguous_or_unissued_values() {
        assert!(validate_identity_secret(Some(&"a".repeat(64))).is_ok());
        for invalid in [
            "private-config-canary".to_owned(),
            "a".repeat(63),
            "A".repeat(64),
            format!(" {}", "a".repeat(64)),
        ] {
            let error = validate_identity_secret(Some(&invalid))
                .err()
                .unwrap()
                .to_string();
            assert!(!error.contains(&invalid));
        }
        assert!(validate_identity_secret(None).is_err());
    }

    #[test]
    fn customer_profile_registration_requires_a_canonical_rfc_v7_pair() {
        let client = "019a1163-0000-7000-8000-000000000001";
        let app = "019a1163-0000-7000-8000-000000000002";
        let valid = HashMap::from([
            ("CUSTOMER_PROFILE_CLIENT_REF".into(), client.into()),
            ("CUSTOMER_PROFILE_APPLICATION_ID".into(), app.into()),
        ]);
        let parsed = Config::from_values(&valid).unwrap();
        assert_eq!(
            parsed.customer_profile_client_ref.unwrap().to_string(),
            client
        );
        assert_eq!(
            parsed.customer_profile_application_id.unwrap().to_string(),
            app
        );
        for key in [
            "CUSTOMER_PROFILE_CLIENT_REF",
            "CUSTOMER_PROFILE_APPLICATION_ID",
        ] {
            let mut values = valid.clone();
            values.remove(key);
            assert!(Config::from_values(&values).is_err());
            for invalid in [
                "private-canary",
                "019a1163-0000-4000-8000-000000000001",
                "019a1163-0000-7000-c000-000000000001",
                "019A1163-0000-7000-8000-000000000001",
                "",
            ] {
                let mut values = valid.clone();
                values.insert(key.into(), invalid.into());
                let error = Config::from_values(&values).err().unwrap().to_string();
                assert!(!error.contains("private-canary"));
            }
        }
    }

    #[test]
    fn non_fixture_identity_writes_require_a_receipt_before_database_admission() {
        let mut values = production();
        values.insert("MEMBERSHIP_LIFECYCLE_ENABLED".into(), "true".into());
        values.insert("CUSTOMER_MEMBERSHIPS_ENABLED".into(), "true".into());
        assert!(Config::from_values(&values).is_err());
        values.insert(
            "MEMBERSHIP_READINESS_RECEIPT".into(),
            "019a1163-0000-7000-8000-000000000001".into(),
        );
        assert!(Config::from_values(&values).is_err());
        values.insert(
            "MEMBERSHIP_READINESS_MANIFEST_SHA256".into(),
            "a".repeat(64),
        );
        assert!(Config::from_values(&values).is_ok());
        values.insert(
            "MEMBERSHIP_READINESS_MANIFEST_SHA256".into(),
            "private-canary".into(),
        );
        let error = Config::from_values(&values).err().unwrap().to_string();
        assert!(!error.contains("private-canary"));
    }
    #[test]
    fn production_rejects_bad_config_without_echoing_values() {
        for (key, raw) in [
            (
                "JWT_SECRET",
                "change-me-in-production-please-use-random-32bytes",
            ),
            ("METRICS_TOKEN", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            ("PUBLIC_APP_URL", "http://localhost"),
            ("MICRO_ALLOWED_ORIGINS", "https://localhost/path"),
            ("CORS_ALLOWED_ORIGINS", "*"),
            ("CORS_ALLOWED_ORIGINS", "null"),
            ("SESSION_TTL_HOURS", "0"),
            ("ACCESS_TOKEN_TTL_SECONDS", "nope"),
            ("OAUTH_AUTO_PURGE", "typo"),
            ("TRUSTED_PROXY_IPS", "0.0.0.0/0"),
            ("MICRO_CA_CERT_FILE", "relative-secret-path"),
            ("DATABASE_URL", "http://sensitive:secret@host/test"),
        ] {
            let mut env = production();
            env.insert(key.into(), raw.into());
            let e = Config::from_values(&env)
                .err()
                .expect("unsafe configuration accepted")
                .to_string();
            assert!(!e.contains(raw), "configuration value echoed in error");
        }
    }
    #[test]
    fn application_production_cannot_be_bypassed_and_password_pair_validates() {
        let mut env = production();
        env.insert("APP_ENV".into(), "development".into());
        env.insert("APPLICATION_ENVIRONMENT".into(), "production".into());
        env.remove("JWT_SECRET");
        assert!(Config::from_values(&env).is_err());
        let mut env = production();
        env.insert("BOOTSTRAP_ADMIN_USERNAME".into(), "operator".into());
        assert!(Config::from_values(&env).is_err());
        env.insert(
            "BOOTSTRAP_ADMIN_PASSWORD".into(),
            "correct-new-admin-password".into(),
        );
        assert!(Config::from_values(&env).is_ok());
    }

    #[test]
    fn authorization_and_database_budgets_are_bounded() {
        for (key, minimum, maximum) in [
            ("DB_MAX_CONNECTIONS", 1, 128),
            ("DB_ACQUIRE_TIMEOUT_MS", 100, 30000),
            ("DB_STATEMENT_TIMEOUT_MS", 100, 60000),
            ("DB_LOCK_TIMEOUT_MS", 100, 30000),
            ("APP_AUTHORIZATION_TIMEOUT_MS", 100, 30000),
            ("APP_AUTHORIZATION_MAX_CONCURRENT", 1, 128),
        ] {
            for value in [minimum, maximum] {
                assert!(
                    Config::from_values(&HashMap::from([(key.into(), value.to_string())])).is_ok(),
                    "{key} valid boundary"
                );
            }
            for value in [minimum - 1, maximum + 1] {
                assert!(
                    Config::from_values(&HashMap::from([(key.into(), value.to_string())])).is_err(),
                    "{key} invalid boundary"
                );
            }
        }
    }

    #[test]
    fn login_and_password_budgets_are_bounded_and_allow_zero_waiters() {
        let defaults = Config::from_values(&HashMap::new()).unwrap();
        assert_eq!(
            (
                defaults.password_concurrency,
                defaults.password_queue_capacity
            ),
            (2, 8)
        );
        assert_eq!(
            (defaults.login_max_entries, defaults.login_window_seconds),
            (4096, 600)
        );
        for (key, invalid) in [
            ("PASSWORD_CONCURRENCY", "0"),
            ("PASSWORD_CONCURRENCY", "33"),
            ("PASSWORD_QUEUE_CAPACITY", "-1"),
            ("PASSWORD_QUEUE_CAPACITY", "1025"),
            ("PASSWORD_QUEUE_TIMEOUT_MS", "0"),
            ("PASSWORD_EXECUTION_TIMEOUT_MS", "120001"),
            ("LOGIN_MAX_ENTRIES", "1"),
            ("LOGIN_MAX_ENTRIES", "2"),
            ("LOGIN_WINDOW_SECONDS", "86401"),
            ("LOGIN_SOURCE_REQUESTS", "0"),
            ("LOGIN_USER_FAILURES", "0"),
            ("LOGIN_IDENTIFIER_MAX_BYTES", "1025"),
            ("LOGIN_VERIFY_BUDGET_MS", "0"),
        ] {
            let env = HashMap::from([(key.to_owned(), invalid.to_owned())]);
            assert!(
                Config::from_values(&env).is_err(),
                "{key} accepted an invalid budget"
            );
        }
        let no_queue = Config::from_values(&HashMap::from([(
            "PASSWORD_QUEUE_CAPACITY".into(),
            "0".into(),
        )]))
        .unwrap();
        assert_eq!(no_queue.password_queue_capacity, 0);
    }

    #[test]
    fn data_grant_policy_defaults_to_empty_and_explicit_json_is_strict() {
        let defaults = Config::from_values(&HashMap::new()).unwrap();
        assert!(defaults.data_grant_policy.applications.is_empty());
        assert!(defaults.data_grant_policy.management_scopes.is_empty());
        for raw in [
            "",
            " ",
            "{}",
            "null",
            "{\"applications\":[]}",
            "{\"applications\":[],\"managementScopes\":[],\"private-policy-secret\":true}",
        ] {
            let values = HashMap::from([("DATA_GRANT_POLICY".into(), raw.into())]);
            let error = Config::from_values(&values)
                .err()
                .expect("invalid policy accepted")
                .to_string();
            assert!(error.contains("DATA_GRANT_POLICY"));
            assert!(!error.contains("private-policy-secret"));
        }
        let values = HashMap::from([(
            "DATA_GRANT_POLICY".into(),
            "{\"applications\":[],\"managementScopes\":[]}".into(),
        )]);
        assert_eq!(
            Config::from_values(&values).unwrap().data_grant_policy,
            defaults.data_grant_policy
        );
    }

    #[test]
    fn data_grant_policy_loads_typed_explicit_capabilities_and_boundaries_once() {
        let policy = serde_json::json!({
            "applications":[{"applicationId":"00000000-0000-7000-8000-000000000001",
                "capabilities":[{"resource":"ticket","action":"read","scopes":["ORG"]}]}],
            "managementScopes":[{"tenantId":"00000000-0000-7000-8000-000000000002",
                "organizationId":"00000000-0000-7000-8000-000000000003",
                "includeDescendants":true,"read":false,"write":true}]
        });
        let mut values = HashMap::from([("DATA_GRANT_POLICY".into(), policy.to_string())]);
        let config = Config::from_values(&values).unwrap();
        values.insert("DATA_GRANT_POLICY".into(), "broken after startup".into());
        let capability = &config.data_grant_policy.applications[0].capabilities[0];
        assert_eq!(capability.resource, "ticket");
        assert_eq!(capability.action, "read");
        assert_eq!(capability.scopes[0].as_str(), "ORG");
        assert!(!config.data_grant_policy.management_scopes[0].read);
        assert!(config.data_grant_policy.management_scopes[0].write);
        assert!(config.data_grant_policy.management_scopes[0].include_descendants);
    }
}
