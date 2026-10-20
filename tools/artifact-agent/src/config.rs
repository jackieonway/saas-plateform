use crate::Error;
use deployment_contract::zip::{self, Group};
use serde::Deserialize;
use std::{
    fs,
    io::Read,
    net::SocketAddr,
    path::{Path, PathBuf},
};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Config {
    pub policy: Group,
    pub node_id: String,
    pub bind: SocketAddr,
    pub root_directory: PathBuf,
    pub token_file: PathBuf,
}

pub(super) fn no_links(path: &Path) -> Result<(), Error> {
    if !path.is_absolute() {
        return Err(Error::Policy);
    }
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(m) if m.is_symlink() => return Err(Error::Policy),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
pub(super) fn read(path: &Path, max: u64) -> Result<Vec<u8>, Error> {
    no_links(path)?;
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.len() > max {
        return Err(Error::Policy);
    }
    let mut v = Vec::new();
    fs::File::open(path)?.take(max + 1).read_to_end(&mut v)?;
    if v.len() as u64 > max {
        return Err(Error::Policy);
    }
    Ok(v)
}
impl Config {
    pub fn load(path: &Path) -> Result<Self, Error> {
        let c: Self = serde_json::from_slice(&read(path, 65536)?)?;
        zip::validate_group(&c.policy)?;
        if !c.bind.ip().is_loopback()
            || c.bind.port() == 0
            || !c.policy.nodes.iter().any(|n| n.id == c.node_id)
        {
            return Err(Error::Policy);
        }
        no_links(&c.root_directory)?;
        no_links(&c.token_file)?;
        if !c.root_directory.is_dir()
            || c.root_directory.parent().is_none()
            || c.token_file
                .canonicalize()?
                .starts_with(c.root_directory.canonicalize()?)
        {
            return Err(Error::Policy);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if fs::metadata(&c.root_directory)?.permissions().mode() & 0o022 != 0
                || fs::metadata(&c.token_file)?.permissions().mode() & 0o077 != 0
            {
                return Err(Error::Policy);
            }
        }
        c.token()?;
        Ok(c)
    }
    pub fn token(&self) -> Result<String, Error> {
        let token = String::from_utf8(read(&self.token_file, 1024)?)
            .map_err(|_| Error::Policy)?
            .trim()
            .to_owned();
        if !(32..=512).contains(&token.len())
            || !token
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        {
            return Err(Error::Policy);
        }
        Ok(token)
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn configuration_cannot_request_additional_execution_authority() {
        let mut v = serde_json::json!({"policy":{"schemaVersion":1,"id":"local","name":"Local","publicOrigin":"https://public.test","applications":["stores"],"nodes":[{"id":"a","origin":"https://a.test"},{"id":"b","origin":"https://b.test"}]},"nodeId":"a","bind":"127.0.0.1:3009","rootDirectory":"/data/sub-apps","tokenFile":"/runtime/token"});
        assert!(serde_json::from_value::<super::Config>(v.clone()).is_ok());
        v["command"] = serde_json::json!(["sh", "-c", "anything"]);
        assert!(serde_json::from_value::<super::Config>(v).is_err());
    }
}
