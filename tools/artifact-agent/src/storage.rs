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
    let stage = staging.join(uuid::Uuid::new_v4().to_string());
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
