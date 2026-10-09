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
    pub auto_create_database: bool,
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
    pub micro_ca_cert_file: Option<String>,
    pub micro_ca_certificates: Vec<reqwest::Certificate>,
    pub oauth_introspection_client_requests_per_minute: usize,
    pub oauth_auth_failure_requests_per_minute: usize,
    pub oauth_introspection_attempts_per_minute: usize,
    pub oauth_auto_purge: bool,
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

pub fn production_mode(app_env: &str, application_env: &str) -> bool {
    app_env.trim().eq_ignore_ascii_case("production")
        || application_env.trim().eq_ignore_ascii_case("production")
}

/// OS/service values win. Production never searches ancestor directories for .env.
pub fn load_environment() -> anyhow::Result<()> {
    if let Ok(path) = std::env::var("DANTE_ENV_FILE") {
        anyhow::ensure!(
            Path::new(&path).is_absolute(),
            "DANTE_ENV_FILE must be an absolute path"
        );
        dotenvy::from_path(&path)
            .map_err(|_| anyhow::anyhow!("DANTE_ENV_FILE cannot be loaded"))?;
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
            "production requires OS environment or explicit DANTE_ENV_FILE"
        );
        dotenvy::from_path(path)
            .map_err(|_| anyhow::anyhow!("local environment file cannot be loaded"))?;
    }
    Ok(())
}

impl Config {
    /// Existing in-process tests use development defaults and replace fixture values.
    pub fn from_env() -> Self {
        Self::try_from_env().expect("invalid runtime configuration")
    }
    pub fn try_from_env() -> anyhow::Result<Self> {
        Self::from_values(&std::env::vars().collect())
    }

    pub fn from_values(env: &HashMap<String, String>) -> anyhow::Result<Self> {
        let app_env = value(env, "APP_ENV", "development");
        let application_environment = value(env, "APPLICATION_ENVIRONMENT", "development");
        let production = production_mode(&app_env, &application_environment);
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
                "postgres://dante@127.0.0.1:5432/dante_cloud",
            ),
            auto_create_database: boolean(env, "AUTO_CREATE_DATABASE", false)?,
            bind_addr: value(env, "BIND_ADDR", "0.0.0.0:3001"),
            jwt_secret: value(env, "JWT_SECRET", "dante-cloud-dev-secret-change-me"),
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
            static_dir: value(env, "STATIC_DIR", ""),
            micro_apps_dir: value(env, "MICRO_APPS_DIR", ""),
        };
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
        assert!(!c.auto_create_database);
        assert_eq!(c.cors_allowed_origins, ["https://localhost:8443"]);
        assert_eq!(c.oauth_introspection_client_requests_per_minute, 600);
        assert!(c.bootstrap_admin_password.is_none());
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
}
