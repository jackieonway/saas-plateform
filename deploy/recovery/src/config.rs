use crate::{io, Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Database {
    pub kind: String,
    pub source_service: String,
    pub target: String,
    pub owner: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub profile: String,
    pub service_file: PathBuf,
    pub admin_service: String,
    pub databases: Vec<Database>,
    pub key_file: PathBuf,
    pub scratch: PathBuf,
    pub local_bundle: PathBuf,
    pub artifact_root: PathBuf,
    pub restored_artifacts: PathBuf,
    pub remote_url: String,
    pub remote_token_file: PathBuf,
    pub ca_file: PathBuf,
    pub monitor_url: String,
    pub monitor_token_file: PathBuf,
    pub sequence_file: PathBuf,
    pub timeout_seconds: u64,
}
pub(crate) fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && s.as_bytes()[0].is_ascii_lowercase()
}
pub(crate) fn private_file(p: &Path) -> Result<Vec<u8>> {
    if !p.is_absolute() {
        return Err(Error("absolute private file required"));
    }
    let m = io(fs::symlink_metadata(p))?;
    if !m.is_file() || m.file_type().is_symlink() {
        return Err(Error("private regular file required"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let current = io(fs::metadata("/proc/self"))?.uid();
        if m.mode() & 0o777 != 0o600 || m.uid() != current {
            return Err(Error("private file must be owned mode 0600"));
        }
    }
    if m.len() > 1024 * 1024 {
        return Err(Error("private file too large"));
    }
    io(fs::read(p))
}
impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let c: Self = serde_json::from_slice(&private_file(path)?)
            .map_err(|_| Error("invalid configuration"))?;
        if c.profile != "pilot"
            || !(1..=1800).contains(&c.timeout_seconds)
            || !identifier(&c.admin_service)
        {
            return Err(Error("unsupported profile or budget"));
        }
        let kinds: BTreeSet<_> = c.databases.iter().map(|d| d.kind.as_str()).collect();
        if c.databases.len() != 3 || kinds != BTreeSet::from(["platform", "stores", "work_orders"])
        {
            return Err(Error("three distinct database kinds required"));
        }
        let mut targets = BTreeSet::new();
        for d in &c.databases {
            if !identifier(&d.source_service)
                || !identifier(&d.target)
                || !identifier(&d.owner)
                || !d.target.starts_with("recovery_")
                || !targets.insert(&d.target)
            {
                return Err(Error("unsafe or duplicate target"));
            }
        }
        for p in [
            &c.scratch,
            &c.local_bundle,
            &c.artifact_root,
            &c.restored_artifacts,
            &c.sequence_file,
        ] {
            if !p.is_absolute() {
                return Err(Error("absolute paths required"));
            }
        }
        if c.key_file == c.local_bundle
            || c.key_file.starts_with(&c.artifact_root)
            || c.key_file.starts_with(&c.restored_artifacts)
        {
            return Err(Error("key must be separate from recovery material"));
        }
        for url in [&c.remote_url, &c.monitor_url] {
            let u = reqwest::Url::parse(url).map_err(|_| Error("invalid endpoint"))?;
            if u.scheme() != "https"
                || !u.username().is_empty()
                || u.password().is_some()
                || u.query().is_some()
                || u.fragment().is_some()
            {
                return Err(Error("verified HTTPS endpoint required"));
            }
        }
        private_file(&c.service_file)?;
        private_file(&c.remote_token_file)?;
        private_file(&c.monitor_token_file)?;
        Ok(c)
    }
    pub fn client(&self) -> Result<reqwest::blocking::Client> {
        let pem = io(fs::read(&self.ca_file))?;
        let cert = reqwest::Certificate::from_pem(&pem).map_err(|_| Error("invalid CA"))?;
        reqwest::blocking::Client::builder()
            .no_proxy()
            .tls_built_in_root_certs(false)
            .add_root_certificate(cert)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(self.timeout_seconds))
            .build()
            .map_err(|_| Error("HTTPS client failed"))
    }
}
