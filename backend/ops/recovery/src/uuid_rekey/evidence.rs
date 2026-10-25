//! U06 evidence opening only. Existing sealing and migration contracts stay closed.
use crate::{Error, Result};
use std::path::Path;

pub(crate) fn execute(path: &Path) -> Result<String> {
    #[cfg(target_os = "linux")]
    {
        linux::execute(path)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        Err(Error("U06 evidence opening requires Linux tmpfs"))
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::{
        native::Scratch,
        uuid_rekey::{codec, structured},
    };
    use serde::Deserialize;
    use std::{
        fs::{self, File, Metadata, OpenOptions},
        io::{Read, Write},
        os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        path::PathBuf,
    };

    const INPUT_ROOT: &str = "/var/lib/saas-uuid-u06-20261024";
    const OUTPUT_ROOT: &str = "/dev/shm/saas-uuid-u06-20261024";
    // Linux UAPI O_NOFOLLOW. File handles are also checked against lstat.
    const O_NOFOLLOW: i32 = 0o400000;
    // A raced FIFO/device must not block before the descriptor type check.
    const O_NONBLOCK: i32 = 0o4000;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Config {
        profile: String,
        input: PathBuf,
        output: PathBuf,
        key_file: PathBuf,
        purpose: String,
        expected_sha256: String,
    }

    fn safe_path(path: &Path) -> Result<()> {
        let raw = path
            .to_str()
            .ok_or(Error("canonical U06 file path required"))?;
        if !raw.starts_with('/')
            || raw.chars().any(char::is_control)
            || raw[1..]
                .split('/')
                .any(|part| part.is_empty() || matches!(part, "." | ".."))
        {
            return Err(Error("canonical U06 file path required"));
        }
        for ancestor in path.ancestors() {
            match fs::symlink_metadata(ancestor) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(Error("U06 symlink path rejected"))
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(Error("U06 path inspection failed")),
            }
        }
        Ok(())
    }

    fn private(meta: &Metadata, directory: bool) -> Result<()> {
        let uid = fs::metadata("/proc/self")
            .map_err(|_| Error("U06 ownership check failed"))?
            .uid();
        if meta.uid() != uid
            || meta.mode() & 0o7777 != if directory { 0o700 } else { 0o600 }
            || if directory {
                !meta.is_dir()
            } else {
                !meta.is_file() || meta.nlink() != 1
            }
        {
            return Err(Error(
                "U06 owned private single-link file or directory required",
            ));
        }
        Ok(())
    }

    fn same(left: &Metadata, right: &Metadata) -> bool {
        left.dev() == right.dev() && left.ino() == right.ino()
    }

    fn open_private(path: &Path) -> Result<File> {
        safe_path(path)?;
        let before =
            fs::symlink_metadata(path).map_err(|_| Error("U06 private file inspection failed"))?;
        private(&before, false)?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(O_NOFOLLOW | O_NONBLOCK)
            .open(path)
            .map_err(|_| Error("U06 private file open failed"))?;
        let held = file
            .metadata()
            .map_err(|_| Error("U06 private file inspection failed"))?;
        let after =
            fs::symlink_metadata(path).map_err(|_| Error("U06 private file inspection failed"))?;
        private(&held, false)?;
        private(&after, false)?;
        safe_path(path)?;
        if !same(&before, &held) || !same(&held, &after) || before.len() != held.len() {
            return Err(Error("U06 private file identity changed"));
        }
        Ok(file)
    }

    fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
        let file = open_private(path)?;
        if file
            .metadata()
            .map_err(|_| Error("U06 private file inspection failed"))?
            .len()
            > limit
        {
            return Err(Error("U06 private file size rejected"));
        }
        let mut bytes = Vec::new();
        file.take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error("U06 private file read failed"))?;
        if bytes.len() as u64 > limit {
            return Err(Error("U06 private file size rejected"));
        }
        Ok(bytes)
    }

    fn under_root(path: &Path, root: &Path, batch: &str) -> Result<()> {
        safe_path(path)?;
        let relative = path
            .strip_prefix(root)
            .map_err(|_| Error("fixed U06 evidence roots required"))?;
        let mut parts = relative.components();
        if parts.next().and_then(|part| part.as_os_str().to_str()) != Some(batch)
            || parts.next().is_none()
        {
            return Err(Error("U06 evidence batch paths disagree"));
        }
        let mut parent = path.parent().ok_or(Error("U06 output parent required"))?;
        loop {
            private(
                &fs::symlink_metadata(parent)
                    .map_err(|_| Error("U06 private parent inspection failed"))?,
                true,
            )?;
            if parent == root {
                break;
            }
            parent = parent
                .parent()
                .ok_or(Error("fixed U06 evidence roots required"))?;
        }
        Ok(())
    }

    impl Config {
        fn validate(&self, input_root: &Path, output_root: &Path) -> Result<()> {
            let parts: Vec<_> = self.purpose.split('/').collect();
            let segment = |value: &str| {
                !value.is_empty()
                    && value.len() <= 100
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            };
            if self.profile != "uuid-local-u06"
                || self.purpose.len() > 256
                || parts.len() != 4
                || parts[0] != "saas-uuid-preview-freeze"
                || parts[1] != "1"
                || !segment(parts[2])
                || !parts[3].starts_with("u06-")
                || !segment(parts[3])
                || parts[3].len() <= 4
                || self.expected_sha256.len() != 64
                || !self
                    .expected_sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || self.input == self.key_file
                || self.input == self.output
                || self.output == self.key_file
            {
                return Err(Error("closed U06 evidence opening contract required"));
            }
            under_root(&self.input, input_root, parts[2])?;
            under_root(&self.output, output_root, parts[2])?;
            safe_path(&self.key_file)?;
            let parent = self
                .output
                .parent()
                .ok_or(Error("U06 output parent required"))?;
            crate::native::require_tmpfs_directory(output_root)?;
            crate::native::require_tmpfs_directory(parent)?;
            match fs::symlink_metadata(&self.output) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                _ => Err(Error("new U06 plaintext output required")),
            }
        }
    }

    struct Publication {
        path: PathBuf,
        identity: Metadata,
        committed: bool,
    }
    impl Drop for Publication {
        fn drop(&mut self) {
            if !self.committed
                && fs::symlink_metadata(&self.path).is_ok_and(|meta| same(&meta, &self.identity))
            {
                let _ = fs::remove_file(&self.path);
            }
        }
    }

    pub(super) fn execute(path: &Path) -> Result<String> {
        open_with_roots(path, Path::new(INPUT_ROOT), Path::new(OUTPUT_ROOT))
    }

    // Root arguments are internal only. The CLI always uses the fixed roots;
    // unit fixtures supply their own isolated directories instead of U06 data.
    fn open_with_roots(path: &Path, input_root: &Path, output_root: &Path) -> Result<String> {
        let bytes = bounded(path, 1024 * 1024)?;
        let text =
            std::str::from_utf8(&bytes).map_err(|_| Error("invalid U06 opening configuration"))?;
        let config: Config = serde_json::from_value(structured::parse_strict(text)?)
            .map_err(|_| Error("invalid U06 opening configuration"))?;
        config.validate(input_root, output_root)?;
        let key = bounded(&config.key_file, 32)?;
        if key.len() != 32 {
            return Err(Error("U06 key must contain 32 random bytes"));
        }
        let mut input = open_private(&config.input)?;
        let original = input
            .metadata()
            .map_err(|_| Error("U06 source inspection failed"))?;
        let parent = config
            .output
            .parent()
            .ok_or(Error("U06 output parent required"))?;
        let scratch = Scratch::new(parent)?;
        private(
            &fs::symlink_metadata(&scratch.0)
                .map_err(|_| Error("U06 temporary directory inspection failed"))?,
            true,
        )?;
        let temporary = scratch.0.join("authenticated-plaintext");
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| Error("U06 temporary output creation failed"))?;
        output
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| Error("U06 output permissions failed"))?;
        let digest = codec::open_stream(&mut input, &mut output, &key, &config.purpose)?;
        if digest.sha256 != config.expected_sha256 {
            return Err(Error("U06 authenticated plaintext digest mismatch"));
        }
        let source_now = input
            .metadata()
            .map_err(|_| Error("U06 source inspection failed"))?;
        private(&source_now, false)?;
        if !same(&original, &source_now) || original.len() != source_now.len() {
            return Err(Error("U06 source identity changed during authentication"));
        }
        output
            .flush()
            .and_then(|_| output.sync_all())
            .map_err(|_| Error("U06 plaintext synchronization failed"))?;
        let identity = output
            .metadata()
            .map_err(|_| Error("U06 output inspection failed"))?;
        private(&identity, false)?;
        config.validate(input_root, output_root)?;
        // Hard-link creation is atomic and never replaces a competing path.
        // Retire the private staging name before publishing a successful result.
        fs::hard_link(&temporary, &config.output)
            .map_err(|_| Error("new U06 plaintext output required"))?;
        let mut publication = Publication {
            path: config.output.clone(),
            identity,
            committed: false,
        };
        fs::remove_file(&temporary).map_err(|_| Error("U06 staging retirement failed"))?;
        File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|_| Error("U06 output directory synchronization failed"))?;
        safe_path(&config.output)?;
        let published = fs::symlink_metadata(&config.output)
            .map_err(|_| Error("U06 published output inspection failed"))?;
        private(&published, false)?;
        if !same(&published, &publication.identity) {
            return Err(Error("U06 published output identity changed"));
        }
        publication.committed = true;
        Ok(
            serde_json::json!({"status":"PRIVATE_U06_EVIDENCE_OPENED", "sha256":digest.sha256,
            "plaintext_bytes":digest.plaintext_bytes,"frames":digest.frames})
            .to_string(),
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use serde_json::{json, Value};
        struct Fixture {
            input_root: PathBuf,
            output_root: PathBuf,
            input_dir: PathBuf,
            output_dir: PathBuf,
            config: PathBuf,
            value: Value,
            plain: Vec<u8>,
        }
        fn write(path: &Path, bytes: &[u8]) {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .unwrap();
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .unwrap();
            file.write_all(bytes).unwrap();
        }
        impl Fixture {
            fn new() -> Self {
                let batch = format!("test-{}", uuid::Uuid::now_v7());
                let input_root = std::env::temp_dir().join(format!("u06-codec-input-{batch}"));
                let output_root = Path::new("/dev/shm").join(format!("u06-codec-output-{batch}"));
                let input_dir = input_root.join(&batch);
                let output_dir = output_root.join(&batch);
                for root in [&input_root, &output_root, &input_dir, &output_dir] {
                    fs::create_dir_all(root).unwrap();
                    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
                }
                let plain = vec![b'x'; 1024 * 1024 + 17];
                let original = output_dir.join("original");
                let key_file = output_dir.join("key");
                let input = input_dir.join("archive.enc");
                write(&original, &plain);
                write(&key_file, &[7; 32]);
                let purpose = format!("saas-uuid-preview-freeze/1/{batch}/u06-snapshot");
                let seal = output_dir.join("seal.json");
                write(
                    &seal,
                    &serde_json::to_vec(
                        &json!({"profile":"uuid-local-preview","input":original,"output":input,
                    "key_file":key_file,"purpose":purpose}),
                    )
                    .unwrap(),
                );
                let report: Value =
                    serde_json::from_str(&crate::execute("uuid-seal-evidence", &seal).unwrap())
                        .unwrap();
                let config = output_dir.join("open.json");
                let value = json!({"profile":"uuid-local-u06","input":input,"output":output_dir.join("opened"),
                    "key_file":key_file,"purpose":purpose,"expected_sha256":report["sha256"]});
                Self {
                    input_root,
                    output_root,
                    input_dir,
                    output_dir,
                    config,
                    value,
                    plain,
                }
            }
            fn run(&self) -> Result<String> {
                fs::write(&self.config, serde_json::to_vec(&self.value).unwrap()).unwrap();
                fs::set_permissions(&self.config, fs::Permissions::from_mode(0o600)).unwrap();
                open_with_roots(&self.config, &self.input_root, &self.output_root)
            }
            fn output(&self) -> PathBuf {
                PathBuf::from(self.value["output"].as_str().unwrap())
            }
            fn clean_failure(&self) {
                let error = self.run().unwrap_err().to_string();
                assert!(!error.contains(self.value["purpose"].as_str().unwrap()));
                assert!(!self.output().exists());
                assert!(fs::read_dir(&self.output_dir).unwrap().all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("recovery-")));
            }
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.input_root);
                let _ = fs::remove_dir_all(&self.output_root);
            }
        }
        #[test]
        fn real_seal_open_roundtrip_preserves_multiframe_bytes_and_never_overwrites() {
            let f = Fixture::new();
            let report: Value = serde_json::from_str(&f.run().unwrap()).unwrap();
            assert_eq!(report["status"], "PRIVATE_U06_EVIDENCE_OPENED");
            assert_eq!(report["frames"], 2);
            assert_eq!(fs::read(f.output()).unwrap(), f.plain);
            let meta = fs::metadata(f.output()).unwrap();
            assert_eq!(meta.mode() & 0o7777, 0o600);
            assert_eq!(meta.nlink(), 1);
            assert!(f.run().is_err());
            assert_eq!(fs::read(f.output()).unwrap(), f.plain);
        }
        #[test]
        fn truncation_wrong_key_purpose_and_plaintext_digest_never_publish() {
            for variant in ["truncate", "footer", "key", "purpose", "digest", "trailing"] {
                let mut f = Fixture::new();
                match variant {
                    "truncate" => {
                        let p = f.value["input"].as_str().unwrap();
                        let file = OpenOptions::new().write(true).open(p).unwrap();
                        file.set_len(file.metadata().unwrap().len() - 1).unwrap();
                    }
                    "footer" => {
                        let path = f.value["input"].as_str().unwrap();
                        let mut bytes = fs::read(path).unwrap();
                        *bytes.last_mut().unwrap() ^= 1;
                        fs::write(path, bytes).unwrap();
                    }
                    "key" => {
                        fs::write(f.value["key_file"].as_str().unwrap(), [8; 32]).unwrap();
                    }
                    "purpose" => {
                        f.value["purpose"] = json!(f.value["purpose"]
                            .as_str()
                            .unwrap()
                            .replace("u06-snapshot", "u06-other"));
                    }
                    "digest" => f.value["expected_sha256"] = json!("0".repeat(64)),
                    "trailing" => OpenOptions::new()
                        .append(true)
                        .open(f.value["input"].as_str().unwrap())
                        .unwrap()
                        .write_all(b"x")
                        .unwrap(),
                    _ => unreachable!(),
                }
                f.clean_failure();
            }
        }
        #[test]
        fn input_key_and_config_links_or_public_permissions_are_rejected() {
            for field in ["input", "key_file"] {
                for link in ["symlink", "hardlink", "public"] {
                    let mut f = Fixture::new();
                    let original = PathBuf::from(f.value[field].as_str().unwrap());
                    let alias = original.with_extension("alias");
                    match link {
                        "symlink" => std::os::unix::fs::symlink(&original, &alias).unwrap(),
                        "hardlink" => fs::hard_link(&original, &alias).unwrap(),
                        "public" => {
                            fs::set_permissions(&original, fs::Permissions::from_mode(0o644))
                                .unwrap();
                        }
                        _ => unreachable!(),
                    }
                    if link != "public" {
                        f.value[field] = json!(alias);
                    }
                    f.clean_failure();
                }
            }
            for hard in [false, true] {
                let f = Fixture::new();
                fs::write(&f.config, serde_json::to_vec(&f.value).unwrap()).unwrap();
                fs::set_permissions(&f.config, fs::Permissions::from_mode(0o600)).unwrap();
                let alias = f.output_dir.join("config-alias");
                if hard {
                    fs::hard_link(&f.config, &alias).unwrap();
                } else {
                    std::os::unix::fs::symlink(&f.config, &alias).unwrap();
                }
                assert!(crate::execute("uuid-open-evidence", &alias).is_err());
                assert!(!f.output().exists());
            }
        }
        #[test]
        fn closed_roots_batches_purpose_and_output_parent_policy_are_required() {
            for variant in ["disk", "batch", "purpose", "profile", "parent", "traversal"] {
                let mut f = Fixture::new();
                match variant {
                    "disk" => f.value["output"] = json!(f.input_dir.join("opened")),
                    "batch" => {
                        f.value["purpose"] =
                            json!("saas-uuid-preview-freeze/1/other-batch/u06-snapshot")
                    }
                    "purpose" => {
                        f.value["purpose"] = json!(f.value["purpose"]
                            .as_str()
                            .unwrap()
                            .replace("u06-snapshot", "old-snapshot"))
                    }
                    "profile" => f.value["profile"] = json!("uuid-local-preview"),
                    "parent" => {
                        fs::set_permissions(&f.output_dir, fs::Permissions::from_mode(0o755))
                            .unwrap()
                    }
                    "traversal" => {
                        f.value["output"] = json!(format!("{}/./opened", f.output_dir.display()))
                    }
                    _ => unreachable!(),
                }
                f.clean_failure();
            }
        }

        #[test]
        fn existing_output_symlink_is_preserved_and_disk_plaintext_is_rejected() {
            let f = Fixture::new();
            let original = f.output_dir.join("original");
            std::os::unix::fs::symlink(&original, f.output()).unwrap();
            let before = fs::symlink_metadata(f.output()).unwrap();
            assert!(f.run().is_err());
            let after = fs::symlink_metadata(f.output()).unwrap();
            assert!(after.file_type().is_symlink() && same(&before, &after));
            assert_eq!(fs::read_link(f.output()).unwrap(), original);
            assert_eq!(fs::read(&original).unwrap(), f.plain);

            let mut f = Fixture::new();
            f.value["output"] = json!(f.input_dir.join("disk-output"));
            fs::write(&f.config, serde_json::to_vec(&f.value).unwrap()).unwrap();
            fs::set_permissions(&f.config, fs::Permissions::from_mode(0o600)).unwrap();
            if crate::native::require_tmpfs_directory(&f.input_root).is_err() {
                assert!(open_with_roots(&f.config, &f.input_root, &f.input_root).is_err());
            }
            assert!(!f.output().exists());
        }
    }
}
