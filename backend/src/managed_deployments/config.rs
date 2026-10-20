use crate::error::AppError;
use deployment_contract::{image_allowed, segment, Target};
use serde::Deserialize;
use std::{collections::HashSet, path::Path};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Config {
    pub targets: Vec<ConfiguredTarget>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ConfiguredTarget {
    pub policy: Target,
    pub agent_token_ref: String,
}

pub(super) fn load() -> Result<Config, AppError> {
    let Some(path) = std::env::var_os("DEPLOYMENT_CONFIG_FILE") else {
        return Ok(Config { targets: vec![] });
    };
    let bytes = std::fs::read(path).map_err(|_| AppError::Unavailable)?;
    if bytes.len() > 65536 {
        return Err(AppError::Unavailable);
    }
    let config: Config = serde_json::from_slice(&bytes).map_err(|_| AppError::Unavailable)?;
    if config.targets.len() > 16 {
        return Err(AppError::Unavailable);
    }
    let mut ids = HashSet::new();
    let mut tokens = HashSet::new();
    for t in &config.targets {
        // A bearer for one node must never authenticate another configured node.
        if !tokens.insert(deployment_contract::sha256(token(t)?.as_bytes())) {
            return Err(AppError::Unavailable);
        }
        let url =
            reqwest::Url::parse(&t.policy.public_origin).map_err(|_| AppError::Unavailable)?;
        if !segment(&t.policy.id, 24)
            || !ids.insert(&t.policy.id)
            || t.policy.name.is_empty()
            || t.policy.name.len() > 80
            || url.scheme() != "https"
            || url.origin().ascii_serialization() != t.policy.public_origin
            || !Path::new(&t.agent_token_ref).is_absolute()
            || t.policy.repositories.is_empty()
            || t.policy.repositories.len() > 32
            || t.policy.templates.is_empty()
            || t.policy.templates.len() > 16
        {
            return Err(AppError::Unavailable);
        }
        let mut templates = HashSet::new();
        for template in &t.policy.templates {
            if !templates.insert(&template.id)
                || !segment(&template.id, 32)
                || !segment(&template.application_code, 48)
                || template.health_path != "/healthz"
            {
                return Err(AppError::Unavailable);
            }
        }
        for repo in &t.policy.repositories {
            if !image_allowed(
                &format!("{repo}@sha256:{}", "0".repeat(64)),
                std::slice::from_ref(repo),
            ) {
                return Err(AppError::Unavailable);
            }
        }
    }
    Ok(config)
}

pub(super) fn token(target: &ConfiguredTarget) -> Result<String, AppError> {
    let meta =
        std::fs::symlink_metadata(&target.agent_token_ref).map_err(|_| AppError::Unavailable)?;
    if !meta.is_file() || meta.len() > 1024 {
        return Err(AppError::Unavailable);
    }
    let token = std::fs::read_to_string(&target.agent_token_ref)
        .map_err(|_| AppError::Unavailable)?
        .trim()
        .to_owned();
    if token.len() < 32
        || token.len() > 512
        || !token
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
    {
        return Err(AppError::Unavailable);
    }
    Ok(token)
}

pub(super) fn authenticate<'a>(
    config: &'a Config,
    id: &str,
    headers: &axum::http::HeaderMap,
) -> Result<&'a ConfiguredTarget, AppError> {
    let target = config
        .targets
        .iter()
        .find(|t| t.policy.id == id)
        .ok_or_else(|| AppError::unauthorized("部署agent身份无效"))?;
    let actual = crate::extractors::bearer_token(headers)?;
    let expected = token(target)?;
    let a = deployment_contract::sha256(actual.as_bytes());
    let b = deployment_contract::sha256(expected.as_bytes());
    let difference = a.bytes().zip(b.bytes()).fold(0_u8, |v, (a, b)| v | (a ^ b));
    if difference != 0 {
        return Err(AppError::unauthorized("部署agent身份无效"));
    }
    Ok(target)
}
