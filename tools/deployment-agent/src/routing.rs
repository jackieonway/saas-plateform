use super::{Config, Result, engine};
use deployment_contract::{Work, route_config};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};

fn destination(c: &Config, w: &Work) -> Result<PathBuf> {
    // Caller validated all segments; canonical directory must remain the configured owned directory.
    if fs::symlink_metadata(&c.route_directory)
        .map_err(|_| "ROUTE_FAILED")?
        .is_symlink()
    {
        return Err("OWNERSHIP_CONFLICT");
    }
    let name = format!("{}-{}-{}.conf", c.policy.id, w.application_code, w.version);
    Ok(c.route_directory.join(name))
}

fn load(path: &PathBuf) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !meta.is_symlink() && meta.len() < 16384 => {
            Ok(Some(fs::read(path).map_err(|_| "ROUTE_FAILED")?))
        }
        Ok(_) => Err("OWNERSHIP_CONFLICT"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("ROUTE_FAILED"),
    }
}

fn atomic(path: &PathBuf, bytes: &[u8]) -> Result<()> {
    let pending = path.with_extension(format!("{}-pending", std::process::id()));
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)
        .map_err(|_| "ROUTE_FAILED")?;
    if f.write_all(bytes).and_then(|_| f.sync_all()).is_err() {
        let _ = fs::remove_file(&pending);
        return Err("ROUTE_FAILED");
    }
    drop(f);
    let r = fs::rename(&pending, path).map_err(|_| "ROUTE_FAILED");
    if r.is_err() {
        let _ = fs::remove_file(pending);
    }
    r
}

fn reload(c: &Config) -> Result<()> {
    engine::docker(c, &["exec", &c.gateway_container, "nginx", "-t"], 10)
        .map_err(|_| "ROUTE_FAILED")?;
    engine::docker(
        c,
        &["exec", &c.gateway_container, "nginx", "-s", "reload"],
        10,
    )
    .map_err(|_| "ROUTE_FAILED")?;
    Ok(())
}

pub(super) fn install(c: &Config, w: &Work) -> Result<()> {
    let path = destination(c, w)?;
    let bytes = route_config(w).into_bytes();
    let before = load(&path)?;
    if before.as_ref().is_some_and(|b| *b != bytes) {
        return Err("OWNERSHIP_CONFLICT");
    }
    if before.is_none() {
        atomic(&path, &bytes)?;
    }
    if reload(c).is_err() {
        if let Some(old) = before {
            atomic(&path, &old)?;
        } else {
            fs::remove_file(&path).map_err(|_| "ROUTE_FAILED")?;
        }
        let _ = reload(c);
        return Err("ROUTE_FAILED");
    }
    Ok(())
}

pub(super) fn remove(c: &Config, w: &Work) -> Result<()> {
    let path = destination(c, w)?;
    let Some(before) = load(&path)? else {
        return Ok(());
    };
    if before != route_config(w).as_bytes() {
        return Err("OWNERSHIP_CONFLICT");
    }
    fs::remove_file(&path).map_err(|_| "CLEANUP_FAILED")?;
    if reload(c).is_err() {
        atomic(&path, &before)?;
        let _ = reload(c);
        return Err("ROUTE_FAILED");
    }
    Ok(())
}
