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
    let legacy = c.route_directory.join(format!(
        "{}-{}-{}.conf",
        c.policy.id, w.application_code, w.version
    ));
    // Retain already installed routes, but do not assign new work to the ambiguous
    // hyphen-joined name: app `stores-v1`/version `2` aliases `stores`/`v1-2`.
    if load(&legacy)?.is_some_and(|bytes| bytes == route_config(w).as_bytes()) {
        return Ok(legacy);
    }
    let identity = format!("{}\n{}\n{}", c.policy.id, w.application_code, w.version);
    Ok(c.route_directory.join(format!(
        "route-{}.conf",
        deployment_contract::sha256(identity.as_bytes())
    )))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Routes(PathBuf);

    impl Routes {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "saas-agent-routes-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Routes {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn fixture(routes: &Routes) -> (Config, Work) {
        let c: Config = serde_json::from_value(serde_json::json!({
            "policy": {"id":"local", "name":"Local", "publicOrigin":"https://host.test",
                "repositories":["registry.test/static"], "templates":[
                    {"id":"static-stores-v1", "applicationCode":"stores-v1", "healthPath":"/healthz"},
                    {"id":"static-stores", "applicationCode":"stores", "healthPath":"/healthz"}]},
            "platformOrigin":"https://host.test", "caFile":"ca.pem", "tokenFile":"token",
            "network":"local", "gatewayContainer":"gateway", "routeDirectory":routes.0,
            "stateDirectory":routes.0, "owner":"local"
        }))
        .unwrap();
        let w: Work = serde_json::from_value(serde_json::json!({
            "jobId":"00000000-0000-0000-0000-000000000001", "releaseId":"release", "applicationCode":"stores-v1",
            "environment":"development", "version":"2", "targetId":"local", "templateId":"static-stores-v1",
            "imageRef":format!("registry.test/static@sha256:{}", "a".repeat(64)),
            "policyRevision":deployment_contract::revision(&c.policy), "operation":"DEPLOY", "attempt":1,
            "nonce":"00000000-0000-0000-0000-000000000002"
        }))
        .unwrap();
        (c, w)
    }

    #[test]
    fn distinct_application_version_pairs_have_distinct_route_files() {
        let routes = Routes::new();
        let (c, first) = fixture(&routes);
        let mut second = first.clone();
        second.application_code = "stores".into();
        second.version = "v1-2".into();
        second.template_id = "static-stores".into();
        assert!(deployment_contract::work_allowed(&first, &c.policy));
        assert!(deployment_contract::work_allowed(&second, &c.policy));

        let first_path = destination(&c, &first).unwrap();
        let second_path = destination(&c, &second).unwrap();
        assert_ne!(first_path, second_path);
        atomic(&first_path, route_config(&first).as_bytes()).unwrap();
        atomic(&second_path, route_config(&second).as_bytes()).unwrap();
        assert_eq!(
            load(&first_path).unwrap().unwrap(),
            route_config(&first).as_bytes()
        );
        assert_eq!(
            load(&second_path).unwrap().unwrap(),
            route_config(&second).as_bytes()
        );
    }

    #[test]
    fn existing_legacy_route_is_reused_without_blocking_an_aliased_pair() {
        let routes = Routes::new();
        let (c, first) = fixture(&routes);
        let legacy = routes.0.join("local-stores-v1-2.conf");
        fs::write(&legacy, route_config(&first)).unwrap();
        assert_eq!(destination(&c, &first).unwrap(), legacy);

        let mut second = first.clone();
        second.application_code = "stores".into();
        second.version = "v1-2".into();
        second.template_id = "static-stores".into();
        assert_ne!(destination(&c, &second).unwrap(), legacy);
        assert_eq!(
            load(&legacy).unwrap().unwrap(),
            route_config(&first).as_bytes()
        );
    }
}
