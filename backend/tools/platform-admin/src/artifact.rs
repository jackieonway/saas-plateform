//! Local immutable declarations. This module neither contacts a registry nor
//! claims that the platform stores digest metadata or controls Docker.
use crate::{
    SafeError,
    contract::{self, KnownApp},
    options::Options,
    release,
};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const MAX_RECORDS: usize = 4096;
const MAX_STORE_BYTES: u64 = 64 * 1024 * 1024;
static NONCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Artifact {
    schema_version: u8,
    application_code: KnownApp,
    public_origin: String,
    source: Source,
    digest: String,
    commit: String,
    build_id: String,
    inputs_sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Source {
    #[serde(rename = "OCI_IMAGE")]
    Image {
        identity: String,
        #[serde(rename = "targetPlatform")]
        target_platform: String,
    },
    #[serde(rename = "STATIC_ARTIFACT")]
    Static { identity: String },
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Binding {
    schema_version: u8,
    sequence: u64,
    artifact: Artifact,
    pub(crate) release: Value,
    public_version_url: String,
}
fn error(category: &'static str) -> SafeError {
    SafeError::new(category)
}
fn hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|v| hex(v, 64))
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains("..")
        && value
            .bytes()
            .enumerate()
            .all(|(i, b)| b.is_ascii_alphanumeric() || i > 0 && matches!(b, b'.' | b'_' | b'-'))
}
fn validate_artifact(value: &Artifact, options: &Options) -> Result<(), SafeError> {
    if value.schema_version != 1
        || value.application_code != options.app()
        || !digest(&value.digest)
        || !hex(&value.commit, 40)
        || !hex(&value.inputs_sha256, 64)
        || !identifier(&value.build_id)
    {
        return Err(error("INVALID_ARTIFACT"));
    }
    let base = Url::parse("https://artifact.invalid/").map_err(|_| error("INVALID_ARTIFACT"))?;
    let origin = release::safe_url(&value.public_origin, &base, options)
        .map_err(|_| error("INVALID_ARTIFACT"))?;
    if origin.origin().ascii_serialization() != value.public_origin || origin.path() != "/" {
        return Err(error("INVALID_ARTIFACT"));
    }
    match &value.source {
        Source::Image {
            identity,
            target_platform,
        } => {
            let Some((repository, image_digest)) = identity.rsplit_once('@') else {
                return Err(error("INVALID_ARTIFACT"));
            };
            let Some((registry, path)) = repository.split_once('/') else {
                return Err(error("INVALID_ARTIFACT"));
            };
            if identity.len() > 512
                || image_digest != value.digest
                || !matches!(target_platform.as_str(), "linux/amd64" | "linux/arm64")
                || registry.is_empty()
                || path.is_empty()
                || repository.contains("..")
                || !registry.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-' | b':')
                })
                || !path.split('/').all(|segment| {
                    !segment.is_empty()
                        && segment.bytes().enumerate().all(|(i, b)| {
                            b.is_ascii_lowercase()
                                || b.is_ascii_digit()
                                || i > 0 && matches!(b, b'.' | b'_' | b'-')
                        })
                })
            {
                return Err(error("INVALID_ARTIFACT"));
            }
            // URL parsing also rejects malformed registry ports/hosts without
            // resolving names or contacting any registry.
            let parsed = Url::parse(&format!("https://{registry}/"))
                .map_err(|_| error("INVALID_ARTIFACT"))?;
            if parsed.host_str().is_none()
                || parsed.origin().ascii_serialization() != format!("https://{registry}")
            {
                return Err(error("INVALID_ARTIFACT"));
            }
        }
        Source::Static { identity } => {
            let url = release::safe_url(identity, &base, options)
                .map_err(|_| error("INVALID_ARTIFACT"))?;
            if identity.len() > 2000
                || url.scheme() != "https"
                || url.as_str() != identity
                || !identity.starts_with("https://")
            {
                return Err(error("INVALID_ARTIFACT"));
            }
        }
    }
    Ok(())
}
fn reparse(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}
fn safe_path(path: &Path, missing_leaf: bool) -> Result<(), SafeError> {
    if !path.is_absolute()
        || path.components().any(|c| match c {
            Component::CurDir | Component::ParentDir => true,
            Component::Normal(value) => value
                .to_str()
                .is_none_or(|s| s.contains(':') || s.chars().any(char::is_control)),
            #[cfg(windows)]
            Component::Prefix(prefix) => !matches!(
                prefix.kind(),
                std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
            ),
            _ => false,
        })
    {
        return Err(error("INVALID_ARTIFACT_PATH"));
    }
    for (index, ancestor) in path.ancestors().enumerate() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if reparse(&metadata) || index > 0 && !metadata.is_dir() => {
                return Err(error("INVALID_ARTIFACT_PATH"));
            }
            Ok(_) => {}
            Err(e) if missing_leaf && index == 0 && e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(error("INVALID_ARTIFACT_PATH")),
        }
    }
    Ok(())
}
fn read_file(path: &Path, category: &'static str) -> Result<Value, SafeError> {
    safe_path(path, false)?;
    // Check before opening: opening a FIFO or device can itself block or have
    // side effects, even though a later handle metadata check would reject it.
    let before = fs::symlink_metadata(path).map_err(|_| error(category))?;
    if !before.is_file() || before.len() > contract::MAX_JSON_BYTES as u64 {
        return Err(error(category));
    }
    let mut file = File::open(path).map_err(|_| error(category))?;
    let metadata = file.metadata().map_err(|_| error(category))?;
    if !metadata.is_file() || metadata.len() > contract::MAX_JSON_BYTES as u64 {
        return Err(error(category));
    }
    safe_path(path, false)?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(contract::MAX_JSON_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error(category))?;
    if bytes.len() > contract::MAX_JSON_BYTES {
        return Err(error(category));
    }
    contract::strict_json(&bytes).map_err(|_| error(category))
}
fn root_path(options: &Options) -> Result<PathBuf, SafeError> {
    let root = PathBuf::from(
        options
            .inventory_dir
            .as_deref()
            .ok_or_else(|| error("INVALID_ARGUMENT"))?,
    );
    safe_path(&root, true)?;
    if root.parent().is_none() {
        return Err(error("INVALID_ARTIFACT_PATH"));
    }
    let resolved = if root.exists() {
        fs::canonicalize(&root)
    } else {
        fs::canonicalize(
            root.parent()
                .ok_or_else(|| error("INVALID_ARTIFACT_PATH"))?,
        )
        .map(|p| p.join(root.file_name().unwrap_or_default()))
    }
    .map_err(|_| error("INVALID_ARTIFACT_PATH"))?;
    for raw in [&options.artifact_file, &options.release_file] {
        let path = Path::new(raw.as_deref().ok_or_else(|| error("INVALID_ARGUMENT"))?);
        safe_path(path, false)?;
        let input = fs::canonicalize(path).map_err(|_| error("INVALID_ARTIFACT_PATH"))?;
        if input.starts_with(&resolved) || resolved.starts_with(&input) {
            return Err(error("INVALID_ARTIFACT_PATH"));
        }
    }
    Ok(root)
}
fn candidate(options: &Options) -> Result<Binding, SafeError> {
    let path = Path::new(
        options
            .artifact_file
            .as_deref()
            .ok_or_else(|| error("INVALID_ARGUMENT"))?,
    );
    let artifact: Artifact = serde_json::from_value(read_file(path, "INVALID_ARTIFACT")?)
        .map_err(|_| error("INVALID_ARTIFACT"))?;
    validate_artifact(&artifact, options)?;
    let release_path = Path::new(
        options
            .release_file
            .as_deref()
            .ok_or_else(|| error("INVALID_ARGUMENT"))?,
    );
    let release = release::validate(
        read_file(release_path, "INVALID_RELEASE")?,
        options,
        Some(&artifact.public_origin),
    )?;
    let public_version_url = format!(
        "{}/sub-apps/{}/versions/{}/",
        artifact.public_origin,
        artifact.application_code.code(),
        release["version"]
            .as_str()
            .ok_or_else(|| error("INVALID_RELEASE"))?
    );
    Ok(Binding {
        schema_version: 1,
        sequence: 0,
        artifact,
        release,
        public_version_url,
    })
}
fn same_artifact(a: &Binding, b: &Binding) -> bool {
    a.artifact.application_code == b.artifact.application_code
        && a.artifact.source == b.artifact.source
        && a.artifact.digest == b.artifact.digest
        && a.artifact.commit == b.artifact.commit
        && a.artifact.build_id == b.artifact.build_id
        && a.artifact.inputs_sha256 == b.artifact.inputs_sha256
}
fn same_definition(a: &Value, b: &Value) -> bool {
    ["version", "entryType", "entryUrl", "healthUrl", "resources"]
        .iter()
        .all(|key| a[*key] == b[*key])
}
fn matching(history: &[Binding], next: &Binding) -> Result<Option<u64>, SafeError> {
    let mut matched = None;
    for old in history
        .iter()
        .filter(|old| old.artifact.application_code == next.artifact.application_code)
    {
        if old.release["version"] == next.release["version"] && !same_artifact(old, next) {
            return Err(error("ARTIFACT_CONFLICT"));
        }
        if old.public_version_url == next.public_version_url
            && (!same_artifact(old, next) || !same_definition(&old.release, &next.release))
        {
            return Err(error("ARTIFACT_CONFLICT"));
        }
        if old.release["version"] == next.release["version"]
            && old.release["environment"] == next.release["environment"]
        {
            if !same_artifact(old, next) || old.release != next.release {
                return Err(error("ARTIFACT_CONFLICT"));
            }
            matched = Some(old.sequence);
        }
    }
    Ok(matched)
}
fn validate_record(record: &Binding) -> Result<(), SafeError> {
    if record.schema_version != 1 || record.sequence == 0 {
        return Err(error("INVALID_ARTIFACT_STORE"));
    }
    let environment = record.release["environment"]
        .as_str()
        .ok_or_else(|| error("INVALID_ARTIFACT_STORE"))?;
    let mut args = vec![
        "register".to_owned(),
        "--release-file".into(),
        "unused".into(),
        "--environment".into(),
        environment.into(),
        "--app-code".into(),
        record.artifact.application_code.code().into(),
    ];
    if environment == "development" {
        args.push("--allow-loopback-http".into());
    }
    let options = Options::parse(args).map_err(|_| error("INVALID_ARTIFACT_STORE"))?;
    validate_artifact(&record.artifact, &options).map_err(|_| error("INVALID_ARTIFACT_STORE"))?;
    let release = release::validate(
        record.release.clone(),
        &options,
        Some(&record.artifact.public_origin),
    )
    .map_err(|_| error("INVALID_ARTIFACT_STORE"))?;
    let expected = format!(
        "{}/sub-apps/{}/versions/{}/",
        record.artifact.public_origin,
        record.artifact.application_code.code(),
        release["version"]
            .as_str()
            .ok_or_else(|| error("INVALID_ARTIFACT_STORE"))?
    );
    if release != record.release || expected != record.public_version_url {
        return Err(error("INVALID_ARTIFACT_STORE"));
    }
    Ok(())
}
fn pending_name(name: &str) -> bool {
    name.strip_prefix(".pending-").is_some_and(|tail| {
        let parts: Vec<_> = tail.split('-').collect();
        tail.len() <= 100
            && parts.len() == 3
            && parts
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
            && parts[0].parse::<u32>().is_ok_and(|pid| pid > 0)
            && parts[1].parse::<u128>().is_ok_and(|nanos| nanos > 0)
            && parts[2].parse::<u64>().is_ok()
    })
}
struct Store {
    root: PathBuf,
    _lock: File,
    history: Vec<Binding>,
    total_bytes: u64,
}
impl Store {
    fn open(root: PathBuf, create: bool) -> Result<Option<Self>, SafeError> {
        if !root.exists() {
            if !create {
                return Ok(None);
            }
            match fs::create_dir(&root) {
                Ok(()) => {
                    #[cfg(unix)]
                    {
                        if File::open(
                            root.parent()
                                .ok_or_else(|| error("INVALID_ARTIFACT_PATH"))?,
                        )
                        .and_then(|parent| parent.sync_all())
                        .is_err()
                        {
                            return Err(error("ARTIFACT_WRITE_FAILED").uncertain());
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(error("ARTIFACT_WRITE_FAILED")),
            }
        }
        safe_path(&root, false)?;
        if !fs::metadata(&root)
            .map_err(|_| error("INVALID_ARTIFACT_STORE"))?
            .is_dir()
        {
            return Err(error("INVALID_ARTIFACT_STORE"));
        }
        let lock_path = root.join(".lock");
        safe_path(&lock_path, true)?;
        let lock = if create {
            let mut open = OpenOptions::new();
            open.read(true).write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                open.mode(0o600);
            }
            match open.open(&lock_path) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&lock_path)
                    .map_err(|_| error("INVALID_ARTIFACT_STORE"))?,
                Err(_) => return Err(error("ARTIFACT_WRITE_FAILED")),
            }
        } else {
            match OpenOptions::new().read(true).write(true).open(&lock_path) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    if fs::read_dir(&root)
                        .map_err(|_| error("INVALID_ARTIFACT_STORE"))?
                        .next()
                        .is_none()
                    {
                        return Ok(None);
                    }
                    return Err(error("INVALID_ARTIFACT_STORE"));
                }
                Err(_) => return Err(error("INVALID_ARTIFACT_STORE")),
            }
        };
        safe_path(&lock_path, false)?;
        if !lock
            .metadata()
            .map_err(|_| error("INVALID_ARTIFACT_STORE"))?
            .is_file()
        {
            return Err(error("INVALID_ARTIFACT_STORE"));
        }
        lock.try_lock().map_err(|e| match e {
            std::fs::TryLockError::WouldBlock => error("ARTIFACT_STORE_BUSY"),
            std::fs::TryLockError::Error(_) => error("INVALID_ARTIFACT_STORE"),
        })?;
        let mut store = Self {
            root,
            _lock: lock,
            history: Vec::new(),
            total_bytes: 0,
        };
        (store.history, store.total_bytes) = store.read_history()?;
        Ok(Some(store))
    }
    fn read_history(&self) -> Result<(Vec<Binding>, u64), SafeError> {
        let mut files = Vec::new();
        let mut total = 0u64;
        for entry in fs::read_dir(&self.root).map_err(|_| error("INVALID_ARTIFACT_STORE"))? {
            let entry = entry.map_err(|_| error("INVALID_ARTIFACT_STORE"))?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| error("INVALID_ARTIFACT_STORE"))?;
            let metadata =
                fs::symlink_metadata(entry.path()).map_err(|_| error("INVALID_ARTIFACT_STORE"))?;
            if reparse(&metadata)
                || !metadata.is_file()
                || metadata.len() > contract::MAX_JSON_BYTES as u64
            {
                return Err(error("INVALID_ARTIFACT_STORE"));
            }
            if name == ".lock" || pending_name(&name) {
                continue;
            }
            let sequence = name
                .strip_suffix(".json")
                .filter(|v| v.len() == 20 && v.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|v| *v > 0)
                .ok_or_else(|| error("INVALID_ARTIFACT_STORE"))?;
            total = total
                .checked_add(metadata.len())
                .ok_or_else(|| error("INVALID_ARTIFACT_STORE"))?;
            files.push((sequence, entry.path()));
            if files.len() > MAX_RECORDS || total > MAX_STORE_BYTES {
                return Err(error("INVALID_ARTIFACT_STORE"));
            }
        }
        files.sort_by_key(|(sequence, _)| *sequence);
        let mut history = Vec::new();
        for (index, (sequence, path)) in files.into_iter().enumerate() {
            if sequence != index as u64 + 1 {
                return Err(error("INVALID_ARTIFACT_STORE"));
            }
            let record: Binding = serde_json::from_value(
                read_file(&path, "INVALID_ARTIFACT_STORE")
                    .map_err(|_| error("INVALID_ARTIFACT_STORE"))?,
            )
            .map_err(|_| error("INVALID_ARTIFACT_STORE"))?;
            validate_record(&record)?;
            if record.sequence != sequence
                || matching(&history, &record)
                    .map_err(|_| error("INVALID_ARTIFACT_STORE"))?
                    .is_some()
            {
                return Err(error("INVALID_ARTIFACT_STORE"));
            }
            history.push(record);
        }
        Ok((history, total))
    }
    fn append(&self, mut record: Binding) -> Result<u64, SafeError> {
        if self.history.len() >= MAX_RECORDS {
            return Err(error("INVALID_ARTIFACT_STORE"));
        }
        record.sequence = self.history.len() as u64 + 1;
        let bytes = serde_json::to_vec(&record).map_err(|_| error("INVALID_ARTIFACT_STORE"))?;
        if bytes.len() > contract::MAX_JSON_BYTES
            || self.total_bytes + bytes.len() as u64 > MAX_STORE_BYTES
        {
            return Err(error("INVALID_ARTIFACT_STORE"));
        }
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| error("ARTIFACT_WRITE_FAILED"))?
            .as_nanos();
        let pending = self.root.join(format!(
            ".pending-{}-{}-{}",
            std::process::id(),
            nonce,
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        let final_path = self.root.join(format!("{:020}.json", record.sequence));
        safe_path(&self.root, false)?;
        let mut open = OpenOptions::new();
        open.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            open.mode(0o600);
        }
        let mut file = open
            .open(&pending)
            .map_err(|_| error("ARTIFACT_WRITE_FAILED"))?;
        let prepared = file.write_all(&bytes).and_then(|()| file.sync_all());
        if prepared.is_err() {
            drop(file);
            let _ = fs::remove_file(&pending);
            return Err(error("ARTIFACT_WRITE_FAILED"));
        }
        drop(file);
        // No replacing rename: the committed inode was completely synced and
        // hard_link fails if a final name already exists on Linux and Windows.
        if fs::hard_link(&pending, &final_path).is_err() {
            let committed = read_file(&final_path, "ARTIFACT_WRITE_FAILED")
                .ok()
                .and_then(|v| serde_json::from_value::<Binding>(v).ok());
            let _ = fs::remove_file(&pending);
            if committed.as_ref() == Some(&record) {
                return Ok(record.sequence);
            }
            return Err(error("ARTIFACT_WRITE_FAILED").uncertain());
        }
        #[cfg(unix)]
        {
            if File::open(&self.root)
                .and_then(|dir| dir.sync_all())
                .is_err()
            {
                let _ = fs::remove_file(&pending);
                return Err(error("ARTIFACT_WRITE_FAILED").uncertain());
            }
        }
        // A leftover own pending hard link does not invalidate the committed
        // record and cannot change it; only this invocation may remove it.
        let _ = fs::remove_file(&pending);
        Ok(record.sequence)
    }
}
fn output(
    options: &Options,
    next: &Binding,
    mode: &str,
    sequence: Option<u64>,
    appended: bool,
) -> Value {
    let mut value = json!({"ok":true,"mode":mode,"command":options.command,"applicationCode":options.app().code(),"environment":options.environment,"version":next.release["version"],"bindingState":if sequence.is_some(){"BOUND"}else{"UNBOUND"},"localWrites":appended,"externalWrites":false,"remoteDigestStored":false});
    if let Some(sequence) = sequence {
        value["sequence"] = json!(sequence);
    }
    if mode == "PLAN" {
        value["wouldAppend"] = json!(sequence.is_none());
    }
    if mode == "APPLIED" {
        value["appended"] = json!(appended);
    }
    value
}
pub(crate) fn verify(options: &Options) -> Result<Binding, SafeError> {
    let root = root_path(options)?;
    let mut next = candidate(options)?;
    let store = Store::open(root, false)?.ok_or_else(|| error("ARTIFACT_STORE_MISSING"))?;
    next.sequence =
        matching(&store.history, &next)?.ok_or_else(|| error("ARTIFACT_BINDING_MISSING"))?;
    Ok(next)
}
pub(crate) fn execute(options: &Options) -> Result<Value, SafeError> {
    if options.command == "artifact-verify" {
        let next = verify(options)?;
        return Ok(output(
            options,
            &next,
            "VERIFIED",
            Some(next.sequence),
            false,
        ));
    }
    let root = root_path(options)?;
    let next = candidate(options)?;
    let store = Store::open(root, options.apply)?;
    let sequence = store
        .as_ref()
        .map(|s| matching(&s.history, &next))
        .transpose()?
        .flatten();
    if !options.apply {
        return Ok(output(options, &next, "PLAN", sequence, false));
    }
    if let Some(sequence) = sequence {
        return Ok(output(options, &next, "APPLIED", Some(sequence), false));
    }
    let sequence = store
        .ok_or_else(|| error("ARTIFACT_STORE_MISSING"))?
        .append(next.clone())?;
    Ok(output(options, &next, "APPLIED", Some(sequence), true))
}
