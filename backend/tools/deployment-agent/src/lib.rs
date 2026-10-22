//! Independent worker. Only fixed Docker argv and operator-owned route files are used.
mod engine;
mod routing;
use deployment_contract::{Receipt, Target, Work, container_name, work_allowed};
use reqwest::blocking::Client;
use serde::Deserialize;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

type Result<T> = std::result::Result<T, &'static str>;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Config {
    policy: Target,
    platform_origin: String,
    ca_file: PathBuf,
    token_file: PathBuf,
    network: String,
    gateway_container: String,
    route_directory: PathBuf,
    state_directory: PathBuf,
    owner: String,
}

impl Config {
    fn load(path: &Path) -> Result<Self> {
        if !path.is_absolute() {
            return Err("POLICY_REJECTED");
        }
        let bytes = fs::read(path).map_err(|_| "CONFIG_UNAVAILABLE")?;
        if bytes.len() > 65536 {
            return Err("POLICY_REJECTED");
        }
        let c: Self = serde_json::from_slice(&bytes).map_err(|_| "POLICY_REJECTED")?;
        let origin = reqwest::Url::parse(&c.platform_origin).map_err(|_| "POLICY_REJECTED")?;
        let public = reqwest::Url::parse(&c.policy.public_origin).map_err(|_| "POLICY_REJECTED")?;
        if origin.scheme() != "https"
            || origin.origin().ascii_serialization() != c.platform_origin
            || public.scheme() != "https"
            || public.origin().ascii_serialization() != c.policy.public_origin
            || !deployment_contract::segment(&c.policy.id, 24)
            || !deployment_contract::segment(&c.network, 100)
            || !deployment_contract::segment(&c.gateway_container, 100)
            || !deployment_contract::segment(&c.owner, 100)
            || c.policy.repositories.is_empty()
            || c.policy.repositories.len() > 32
            || c.policy.templates.is_empty()
            || c.policy.templates.len() > 16
        {
            return Err("POLICY_REJECTED");
        }
        for path in [
            &c.ca_file,
            &c.token_file,
            &c.route_directory,
            &c.state_directory,
        ] {
            if !path.is_absolute()
                || fs::symlink_metadata(path)
                    .map_err(|_| "CONFIG_UNAVAILABLE")?
                    .is_symlink()
            {
                return Err("POLICY_REJECTED");
            }
        }
        if !c.route_directory.is_dir()
            || !c.state_directory.is_dir()
            || !c.ca_file.is_file()
            || !c.token_file.is_file()
        {
            return Err("POLICY_REJECTED");
        }
        Ok(c)
    }

    fn token(&self) -> Result<String> {
        if fs::metadata(&self.token_file)
            .map_err(|_| "CONFIG_UNAVAILABLE")?
            .len()
            > 1024
        {
            return Err("POLICY_REJECTED");
        }
        let token = fs::read_to_string(&self.token_file)
            .map_err(|_| "CONFIG_UNAVAILABLE")?
            .trim()
            .to_owned();
        if token.len() < 32
            || token.len() > 512
            || !token
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        {
            return Err("POLICY_REJECTED");
        }
        Ok(token)
    }
}

fn client(config: &Config) -> Result<Client> {
    let ca = fs::read(&config.ca_file).map_err(|_| "CONFIG_UNAVAILABLE")?;
    let cert = reqwest::Certificate::from_pem(&ca).map_err(|_| "TLS_FAILED")?;
    Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .add_root_certificate(cert)
        .build()
        .map_err(|_| "TLS_FAILED")
}

fn claim(client: &Client, c: &Config, token: &str) -> Result<Option<Work>> {
    let response = client
        .post(format!(
            "{}/api/deployment-agent/{}/claim",
            c.platform_origin, c.policy.id
        ))
        .bearer_auth(token)
        .send()
        .map_err(|_| "CONTROL_UNAVAILABLE")?;
    if !response.status().is_success() {
        return Err("CONTROL_REJECTED");
    }
    let mut bytes = Vec::new();
    response
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| "CONTROL_UNAVAILABLE")?;
    if bytes.len() > 65536 {
        return Err("POLICY_REJECTED");
    }
    serde_json::from_slice(&bytes).map_err(|_| "POLICY_REJECTED")
}

fn execute(c: &Config, w: &Work) -> Receipt {
    let name = container_name(&w.target_id, &w.application_code, &w.version, &w.image_ref);
    let mut receipt = Receipt {
        nonce: w.nonce.clone(),
        attempt: w.attempt,
        state: "FAILED".into(),
        image_id: None,
        container_name: name,
        route_sha256: None,
        error_code: None,
    };
    let result = (|| {
        if !work_allowed(w, &c.policy)
            || !deployment_contract::segment(&w.job_id, 36)
            || w.job_id.len() != 36
            || !deployment_contract::segment(&w.nonce, 36)
            || w.nonce.len() != 36
        {
            return Err("POLICY_REJECTED");
        }
        let lock_path = c.state_directory.join("target.lock");
        if fs::symlink_metadata(&lock_path).is_ok_and(|m| !m.is_file() || m.is_symlink()) {
            return Err("OWNERSHIP_CONFLICT");
        }
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(lock_path)
            .map_err(|_| "ENGINE_FAILED")?;
        lock.try_lock().map_err(|_| "ENGINE_FAILED")?;
        engine::target(c)?;
        if w.operation == "DEPLOY" {
            let image = engine::deploy(c, w, &receipt.container_name)?;
            routing::install(c, w)?;
            receipt.state = "READY".into();
            receipt.image_id = Some(image);
            receipt.route_sha256 = Some(deployment_contract::sha256(
                deployment_contract::route_config(w).as_bytes(),
            ));
        } else {
            engine::verify_if_present(c, w, &receipt.container_name)?;
            routing::remove(c, w)?;
            engine::remove(c, w, &receipt.container_name)?;
            receipt.state = "CLEANED".into();
        }
        Ok(())
    })();
    if let Err(code) = result {
        receipt.error_code = Some(code.into())
    }
    receipt
}

/// Load operator policy and execute one leased operation, or poll when explicitly requested.
/// Errors are fixed codes; raw HTTP/engine output and credentials are never printed.
pub fn run(path: &Path, watch: bool) -> Result<()> {
    loop {
        // Reload on every poll so token revocation and target policy changes take effect.
        let config = Config::load(path)?;
        let token = config.token()?;
        let client = client(&config)?;
        let Some(work) = claim(&client, &config, &token)? else {
            if !watch {
                println!("{{\"status\":\"IDLE\"}}");
                return Ok(());
            }
            std::thread::sleep(Duration::from_secs(2));
            continue;
        };
        let receipt = execute(&config, &work);
        let result = client
            .post(format!(
                "{}/api/deployment-agent/{}/jobs/{}/receipt",
                config.platform_origin, config.policy.id, work.job_id
            ))
            .bearer_auth(&token)
            .json(&receipt)
            .send()
            .map_err(|_| "RECEIPT_UNCONFIRMED")?;
        if !result.status().is_success() {
            return Err("RECEIPT_REJECTED");
        }
        println!(
            "{}",
            serde_json::json!({"jobId":work.job_id,"attempt":work.attempt,"state":receipt.state,"errorCode":receipt.error_code})
        );
        if !watch {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_value() -> serde_json::Value {
        serde_json::json!({
            "policy": {"id":"local", "name":"Local", "publicOrigin":"https://example.test",
                "repositories":["registry.test/static"], "templates":[{"id":"static", "applicationCode":"stores", "healthPath":"/healthz"}]},
            "platformOrigin":"https://example.test", "caFile":"/not-present/ca.pem",
            "tokenFile":"/not-present/token", "network":"local", "gatewayContainer":"gateway",
            "routeDirectory":"/not-present/routes", "stateDirectory":"/not-present/state", "owner":"local"
        })
    }

    fn config() -> Config {
        serde_json::from_value(config_value()).unwrap()
    }

    #[test]
    fn foreign_or_changed_work_is_refused_before_any_engine_or_filesystem_access() {
        let c = config();
        let mut w = Work {
            job_id: "00000000-0000-0000-0000-000000000001".into(),
            release_id: "release".into(),
            application_code: "stores".into(),
            environment: "development".into(),
            version: "1.0.0".into(),
            target_id: "foreign".into(),
            template_id: "static".into(),
            image_ref: format!("registry.test/static@sha256:{}", "a".repeat(64)),
            policy_revision: deployment_contract::revision(&c.policy),
            operation: "DEPLOY".into(),
            attempt: 1,
            nonce: "00000000-0000-0000-0000-000000000002".into(),
        };
        for invalid in ["target", "revision", "command"] {
            w.target_id = if invalid == "target" {
                "foreign"
            } else {
                "local"
            }
            .into();
            w.policy_revision = if invalid == "revision" {
                "bad".into()
            } else {
                deployment_contract::revision(&c.policy)
            };
            w.operation = if invalid == "command" {
                "SHELL"
            } else {
                "DEPLOY"
            }
            .into();
            let receipt = execute(&c, &w);
            assert_eq!(receipt.state, "FAILED");
            assert_eq!(receipt.error_code.as_deref(), Some("POLICY_REJECTED"));
            assert!(receipt.image_id.is_none() && receipt.route_sha256.is_none());
        }
    }

    #[test]
    fn operator_config_refuses_extra_execution_authority() {
        assert!(serde_json::from_value::<Config>(config_value()).is_ok());
        for key in ["command", "dockerSocket", "mounts"] {
            let mut value = config_value();
            value[key] = serde_json::json!(["unauthorized"]);
            assert!(serde_json::from_value::<Config>(value).is_err());
        }
    }
}
