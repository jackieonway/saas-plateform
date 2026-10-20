use crate::{error::AppError, state::AppState};
use deployment_contract::zip::{self, Group};
use serde::Deserialize;
use std::{collections::HashSet, io::Read, path::Path};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Config {
    pub groups: Vec<ConfiguredGroup>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ConfiguredGroup {
    pub policy: Group,
    pub nodes: Vec<Credential>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Credential {
    pub node_id: String,
    pub ca_ref: String,
    pub token_ref: String,
}

pub(super) fn private_file(path: &str, max: u64) -> Result<Vec<u8>, AppError> {
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(AppError::Unavailable);
    }
    for ancestor in p.ancestors() {
        let m = std::fs::symlink_metadata(ancestor).map_err(|_| AppError::Unavailable)?;
        if m.is_symlink() {
            return Err(AppError::Unavailable);
        }
    }
    let m = std::fs::symlink_metadata(p).map_err(|_| AppError::Unavailable)?;
    if !m.is_file() || m.len() > max {
        return Err(AppError::Unavailable);
    }
    let mut bytes = Vec::new();
    std::fs::File::open(p)
        .map_err(|_| AppError::Unavailable)?
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| AppError::Unavailable)?;
    if bytes.len() as u64 > max {
        return Err(AppError::Unavailable);
    }
    Ok(bytes)
}
pub(super) fn token(c: &Credential) -> Result<String, AppError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(&c.token_ref)
            .map_err(|_| AppError::Unavailable)?
            .permissions()
            .mode()
            & 0o077
            != 0
        {
            return Err(AppError::Unavailable);
        }
    }
    let value = String::from_utf8(private_file(&c.token_ref, 1024)?)
        .map_err(|_| AppError::Unavailable)?
        .trim()
        .to_owned();
    if !(32..=512).contains(&value.len())
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
    {
        return Err(AppError::Unavailable);
    }
    Ok(value)
}
pub(super) fn load(state: &AppState) -> Result<Config, AppError> {
    let Some(path) = std::env::var_os("ZIP_DISTRIBUTION_CONFIG_FILE") else {
        return Ok(Config { groups: vec![] });
    };
    let path = path.to_str().ok_or(AppError::Unavailable)?;
    let c: Config =
        serde_json::from_slice(&private_file(path, 65536)?).map_err(|_| AppError::Unavailable)?;
    if c.groups.len() > 8 {
        return Err(AppError::Unavailable);
    }
    let gateway = reqwest::Url::parse(&state.config.micro_gateway_origin)
        .map_err(|_| AppError::Unavailable)?
        .origin()
        .ascii_serialization();
    let mut groups = HashSet::new();
    let mut applications = HashSet::new();
    let mut origins = HashSet::new();
    let mut secrets = HashSet::new();
    let images = crate::managed_deployments::public_targets()?;
    for g in &c.groups {
        zip::validate_group(&g.policy).map_err(|_| AppError::Unavailable)?;
        if !groups.insert(&g.policy.id)
            || g.policy.public_origin != gateway
            || g.nodes.len() != g.policy.nodes.len()
        {
            return Err(AppError::Unavailable);
        }
        for app in &g.policy.applications {
            if !applications.insert(app)
                || images.iter().any(|t| {
                    t.public_origin == g.policy.public_origin
                        && t.templates.iter().any(|x| x.application_code == *app)
                })
            {
                return Err(AppError::Unavailable);
            }
        }
        let mut refs = HashSet::new();
        for node in &g.policy.nodes {
            if !origins.insert(&node.origin) {
                return Err(AppError::Unavailable);
            }
            let credential = g
                .nodes
                .iter()
                .find(|r| r.node_id == node.id)
                .ok_or(AppError::Unavailable)?;
            if !refs.insert(&credential.node_id)
                || !secrets.insert(deployment_contract::sha256(token(credential)?.as_bytes()))
            {
                return Err(AppError::Unavailable);
            }
            reqwest::Certificate::from_pem(&private_file(&credential.ca_ref, 65536)?)
                .map_err(|_| AppError::Unavailable)?;
        }
    }
    Ok(c)
}
