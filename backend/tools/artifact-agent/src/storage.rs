use crate::{
    Error,
    config::{Config, no_links, read},
};
use deployment_contract::{
    sha256,
    zip::{self, Install, Manifest, Receipt, Verify, Work},
};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
const MANIFEST: &str = ".artifact-manifest.json";

fn directory(path: &Path, private: bool) -> Result<(), Error> {
    #[cfg(not(unix))]
    let _ = private;
    no_links(path)?;
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => {}
        Ok(_) => return Err(Error::Conflict),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let b = fs::DirBuilder::new();
            #[cfg(unix)]
            let mut b = b;
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                b.mode(if private { 0o700 } else { 0o755 });
            }
            b.create(path)?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}
fn lock(c: &Config) -> Result<File, Error> {
    let p = c.root_directory.join(".distribution.lock");
    no_links(&p)?;
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&p)?;
    if !file.metadata()?.is_file() {
        return Err(Error::Policy);
    }
    file.try_lock().map_err(|e| match e {
        std::fs::TryLockError::WouldBlock => Error::Busy,
        std::fs::TryLockError::Error(e) => e.into(),
    })?;
    Ok(file)
}
fn destination(c: &Config, w: &Work, m: &Manifest) -> Result<PathBuf, Error> {
    zip::validate_work(w, &c.policy, &c.node_id)?;
    zip::validate_manifest(m)?;
    if w.artifact_sha256 != m.artifact_sha256 {
        return Err(Error::Corrupt);
    }
    let p = c
        .root_directory
        .join(&w.application_code)
        .join("versions")
        .join(&w.version);
    no_links(&p)?;
    Ok(p)
}
fn walk(
    root: &Path,
    relative: &Path,
    files: &mut BTreeMap<String, (u64, String)>,
) -> Result<(), Error> {
    let path = root.join(relative);
    no_links(&path)?;
    for e in fs::read_dir(path)? {
        let e = e?;
        let name = e.file_name().into_string().map_err(|_| Error::Corrupt)?;
        let rel = relative.join(&name);
        let meta = fs::symlink_metadata(e.path())?;
        if meta.is_symlink() {
            return Err(Error::Corrupt);
        }
        if meta.is_dir() {
            if rel.as_os_str().len() > 500 {
                return Err(Error::Corrupt);
            }
            walk(root, &rel, files)?
        } else if meta.is_file() {
            let normalized = rel.to_str().ok_or(Error::Corrupt)?.replace('\\', "/");
            if normalized == MANIFEST {
                continue;
            }
            if !zip::file_path(&normalized) || files.len() >= zip::MAX_FILES {
                return Err(Error::Corrupt);
            }
            let bytes = read(&e.path(), zip::MAX_FILE_BYTES as u64)?;
            files.insert(normalized, (bytes.len() as u64, sha256(&bytes)));
        } else {
            return Err(Error::Corrupt);
        }
    }
    Ok(())
}
fn check(path: &Path, m: &Manifest) -> Result<(), Error> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Error::Missing),
        Err(e) => return Err(e.into()),
    };
    if !meta.is_dir() || meta.is_symlink() {
        return Err(Error::Corrupt);
    }
    let stored: Manifest =
        serde_json::from_slice(&read(&path.join(MANIFEST), 65536)?).map_err(|_| Error::Corrupt)?;
    if stored != *m {
        return Err(Error::Conflict);
    }
    let mut actual = BTreeMap::new();
    walk(path, Path::new(""), &mut actual)?;
    if actual.len() != m.files.len()
        || m.files
            .iter()
            .any(|f| actual.get(&f.path) != Some(&(f.size, f.sha256.clone())))
    {
        return Err(Error::Corrupt);
    }
    Ok(())
}
fn write_file(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    no_links(path)?;
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o644);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
fn sync_directories(path: &Path) -> Result<(), Error> {
    no_links(path)?;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let meta = fs::symlink_metadata(entry.path())?;
        if meta.is_symlink() {
            return Err(Error::Corrupt);
        }
        if meta.is_dir() {
            sync_directories(&entry.path())?;
        }
    }
    File::open(path)?.sync_all()?;
    Ok(())
}
#[cfg(target_os = "linux")]
fn publish(from: &Path, to: &Path) -> Result<(), Error> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};
    renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE).map_err(std::io::Error::from)?;
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn publish(_from: &Path, _to: &Path) -> Result<(), Error> {
    Err(Error::Unsupported)
}

pub(super) fn install(c: &Config, request: Install) -> Result<Receipt, Error> {
    let m = &request.payload.manifest;
    let w = &request.work;
    let target = destination(c, w, m)?;
    let files = zip::decode(&request.payload)?;
    let _lock = lock(c)?;
    match check(&target, m) {
        Ok(()) => return Ok(zip::receipt(w, m, "INSTALL", true)?),
        Err(Error::Missing) => {}
        Err(e) => return Err(e),
    }
    let staging = c.root_directory.join(".staging");
    directory(&staging, true)?;
    let stage = staging.join(uuid::Uuid::now_v7().to_string());
    directory(&stage, true)?;
    let payload = stage.join("payload");
    let result = (|| {
        directory(&payload, false)?;
        write_file(
            &stage.join("owner.json"),
            &serde_json::to_vec(
                &serde_json::json!({"nodeId":c.node_id,"jobId":w.job_id,"artifactSha256":m.artifact_sha256}),
            )?,
        )?;
        for (name, bytes) in &files {
            let path = payload.join(name);
            let mut current = payload.clone();
            if let Some(parent) = Path::new(name).parent() {
                for p in parent.components() {
                    current.push(p);
                    directory(&current, false)?;
                }
            }
            write_file(&path, bytes)?;
        }
        write_file(&payload.join(MANIFEST), &serde_json::to_vec(m)?)?;
        check(&payload, m)?;
        sync_directories(&payload)?;
        let app = c.root_directory.join(&w.application_code);
        directory(&app, false)?;
        directory(&app.join("versions"), false)?;
        // No replacement fallback: a conflicting or concurrently published version survives.
        publish(&payload, &target)?;
        File::open(target.parent().ok_or(Error::Policy)?)?.sync_all()?;
        File::open(&app)?.sync_all()?;
        File::open(&c.root_directory)?.sync_all()?;
        Ok(zip::receipt(w, m, "INSTALL", false)?)
    })();
    // Only this invocation's fresh private UUID stage may be removed. Never the installed tree.
    if fs::symlink_metadata(&stage).is_ok_and(|m| m.is_dir() && !m.is_symlink())
        && let Err(e) = fs::remove_dir_all(&stage)
        && result.is_ok()
    {
        return Err(e.into());
    }
    result
}
pub(super) fn verify(c: &Config, request: Verify) -> Result<Receipt, Error> {
    let target = destination(c, &request.work, &request.manifest)?;
    let _lock = lock(c)?;
    check(&target, &request.manifest)?;
    Ok(zip::receipt(
        &request.work,
        &request.manifest,
        "VERIFY",
        true,
    )?)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::{fs::DirBuilderExt, fs::MetadataExt, fs::PermissionsExt};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("artifact-agent-test-{}", uuid::Uuid::now_v7()));
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn config(root: &Path) -> Config {
        Config {
            policy: zip::Group {
                schema_version: 1,
                id: "local".into(),
                name: "Local".into(),
                public_origin: "https://public.test".into(),
                applications: vec!["stores".into()],
                nodes: vec![
                    zip::Node {
                        id: "a".into(),
                        origin: "https://a.test".into(),
                    },
                    zip::Node {
                        id: "b".into(),
                        origin: "https://b.test".into(),
                    },
                ],
            },
            node_id: "a".into(),
            bind: "127.0.0.1:3009".parse().unwrap(),
            root_directory: root.to_path_buf(),
            token_file: root.join("unused-token"),
        }
    }

    fn payload(bytes: &[u8], encoded: &str) -> zip::Payload {
        let files = vec![zip::File {
            path: "index.html".into(),
            size: bytes.len() as u64,
            sha256: sha256(bytes),
        }];
        // Preserve the schema-1 canonical field order in this wire fixture.
        let canonical = format!(
            "{{\"schemaVersion\":1,\"entryType\":\"HTML\",\"entry\":\"index.html\",\"files\":{}}}",
            serde_json::to_string(&files).unwrap()
        );
        zip::Payload {
            manifest: Manifest {
                schema_version: 1,
                entry_type: "HTML".into(),
                entry: "index.html".into(),
                files,
                artifact_sha256: sha256(canonical.as_bytes()),
            },
            files: vec![zip::PayloadFile {
                path: "index.html".into(),
                data_base64: encoded.into(),
            }],
        }
    }

    fn work(c: &Config, manifest: &Manifest) -> Work {
        Work {
            schema_version: 1,
            job_id: uuid::Uuid::now_v7().to_string(),
            attempt: 1,
            nonce: uuid::Uuid::now_v7().to_string(),
            group_id: c.policy.id.clone(),
            policy_revision: zip::revision(&c.policy).unwrap(),
            node_id: c.node_id.clone(),
            application_code: "stores".into(),
            version: "v1".into(),
            artifact_sha256: manifest.artifact_sha256.clone(),
        }
    }

    #[test]
    fn linux_v7_install_verify_and_reuse_preserve_immutable_files_and_fences() {
        let root = TestDirectory::new();
        let c = config(&root.0);
        let p = payload(b"hello", "aGVsbG8=");
        let w = work(&c, &p.manifest);
        let installed = install(
            &c,
            Install {
                work: w.clone(),
                payload: p.clone(),
            },
        )
        .unwrap();
        assert!(!installed.reused);
        assert!(zip::receipt_matches(&installed, &w, &p.manifest, "INSTALL"));
        let target = root.0.join("stores/versions/v1/index.html");
        assert_eq!(fs::read(&target).unwrap(), b"hello");
        let inode = fs::metadata(&target).unwrap().ino();
        let staging = root.0.join(".staging");
        assert_eq!(
            fs::metadata(&staging).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(fs::read_dir(&staging).unwrap().count(), 0);

        let mut verification = w.clone();
        verification.nonce = uuid::Uuid::now_v7().to_string();
        let verified = verify(
            &c,
            Verify {
                work: verification.clone(),
                manifest: p.manifest.clone(),
            },
        )
        .unwrap();
        assert!(zip::receipt_matches(
            &verified,
            &verification,
            &p.manifest,
            "VERIFY"
        ));
        assert!(!zip::receipt_matches(&verified, &w, &p.manifest, "VERIFY"));
        let reused = install(
            &c,
            Install {
                work: w.clone(),
                payload: p.clone(),
            },
        )
        .unwrap();
        assert!(reused.reused);
        assert_eq!(fs::metadata(&target).unwrap().ino(), inode);

        let conflict = payload(b"world", "d29ybGQ=");
        let mut conflict_work = w.clone();
        conflict_work.artifact_sha256 = conflict.manifest.artifact_sha256.clone();
        assert!(matches!(
            install(
                &c,
                Install {
                    work: conflict_work,
                    payload: conflict
                }
            ),
            Err(Error::Conflict)
        ));
        assert_eq!(fs::read(&target).unwrap(), b"hello");
        assert_eq!(fs::metadata(&target).unwrap().ino(), inode);

        for invalid in [
            "11111111-2222-4333-8444-555555555555", // Deliberate legacy v4 negative.
            "00000000-0000-0000-0000-000000000000", // Nil negative.
            "01962345-6789-7abc-cdef-0123456789ab", // Non-RFC variant negative.
        ] {
            let mut invalid_work = w.clone();
            invalid_work.nonce = invalid.into();
            assert!(matches!(
                verify(
                    &c,
                    Verify {
                        work: invalid_work,
                        manifest: p.manifest.clone()
                    }
                ),
                Err(Error::Contract(zip::Error::Command))
            ));
            let mut invalid_work = w.clone();
            invalid_work.job_id = invalid.into();
            assert!(matches!(
                install(
                    &c,
                    Install {
                        work: invalid_work,
                        payload: p.clone()
                    }
                ),
                Err(Error::Contract(zip::Error::Command))
            ));
        }
        assert_eq!(fs::read(&target).unwrap(), b"hello");
    }

    #[test]
    fn linux_publish_uses_noreplace_even_when_the_destination_is_empty() {
        let root = TestDirectory::new();
        let source = root.0.join("source");
        let target = root.0.join("target");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("new"), b"new bytes").unwrap();
        fs::create_dir(&target).unwrap();
        let target_inode = fs::metadata(&target).unwrap().ino();
        let error = publish(&source, &target).unwrap_err();
        assert!(
            matches!(error, Error::Io(error) if error.kind() == std::io::ErrorKind::AlreadyExists)
        );
        assert_eq!(fs::metadata(&target).unwrap().ino(), target_inode);
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
        assert_eq!(fs::read(source.join("new")).unwrap(), b"new bytes");
    }
}
