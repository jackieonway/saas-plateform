use super::{Config, Result};
use deployment_contract::Work;
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    io::Read,
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
static NEXT: AtomicU64 = AtomicU64::new(0);

pub(super) fn docker(c: &Config, args: &[&str], seconds: u64) -> Result<String> {
    let path = c.state_directory.join(format!(
        "engine-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| "ENGINE_FAILED")?;
    let result = (|| {
        let mut child = Command::new("docker")
            .args(["--host", "unix:///var/run/docker.sock"])
            .args(args)
            .stdin(Stdio::null())
            .stdout(file)
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "ENGINE_FAILED")?;
        let deadline = Instant::now() + Duration::from_secs(seconds);
        loop {
            if let Some(exit) = child.try_wait().map_err(|_| "ENGINE_FAILED")? {
                if !exit.success() {
                    return Err("ENGINE_FAILED");
                }
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err("ENGINE_FAILED");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let mut bytes = Vec::new();
        fs::File::open(&path)
            .map_err(|_| "ENGINE_FAILED")?
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "ENGINE_FAILED")?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("ENGINE_FAILED");
        }
        String::from_utf8(bytes).map_err(|_| "ENGINE_FAILED")
    })();
    let _ = fs::remove_file(path);
    result
}

fn inspect(c: &Config, kind: &str, name: &str) -> Result<Value> {
    let out = docker(c, &[kind, "inspect", name], 10)?;
    let items: Vec<Value> = serde_json::from_str(&out).map_err(|_| "ENGINE_FAILED")?;
    items.into_iter().next().ok_or("ENGINE_FAILED")
}

pub(super) fn target(c: &Config) -> Result<()> {
    let network = inspect(c, "network", &c.network)?;
    let gateway = inspect(c, "container", &c.gateway_container)?;
    if network["Labels"]["saas.deployment.owner"] != c.owner
        || gateway["Config"]["Labels"]["saas.deployment.owner"] != c.owner
        || gateway["State"]["Running"] != true
    {
        return Err("OWNERSHIP_CONFLICT");
    }
    Ok(())
}

fn owned(c: &Config, w: &Work, name: &str) -> Result<Value> {
    let v = inspect(c, "container", name)?;
    let labels = &v["Config"]["Labels"];
    if labels["saas.deployment.owner"] != c.owner
        || labels["saas.deployment.target"] != c.policy.id
        || labels["saas.deployment.app"] != w.application_code
        || labels["saas.deployment.version"] != w.version
        || labels["saas.deployment.image"] != w.image_ref
    {
        return Err("OWNERSHIP_CONFLICT");
    }
    Ok(v)
}

fn exists(c: &Config, name: &str) -> Result<bool> {
    let output = docker(
        c,
        &[
            "ps",
            "--all",
            "--filter",
            &format!("name=^{name}$"),
            "--format",
            "{{.Names}}",
        ],
        10,
    )?;
    if output.trim().is_empty() {
        return Ok(false);
    }
    if output.trim() != name {
        return Err("OWNERSHIP_CONFLICT");
    }
    Ok(true)
}

pub(super) fn verify_if_present(c: &Config, w: &Work, name: &str) -> Result<()> {
    if exists(c, name)? {
        owned(c, w, name)?;
    }
    Ok(())
}

pub(super) fn deploy(c: &Config, w: &Work, name: &str) -> Result<String> {
    docker(c, &["pull", &w.image_ref], 180).map_err(|_| "IMAGE_UNAVAILABLE")?;
    let image = inspect(c, "image", &w.image_ref).map_err(|_| "IMAGE_UNAVAILABLE")?;
    let digest = image["Id"].as_str().ok_or("IMAGE_UNAVAILABLE")?.to_owned();
    let refs = image["RepoDigests"].as_array().ok_or("IMAGE_UNAVAILABLE")?;
    if !refs
        .iter()
        .any(|v| v.as_str() == Some(w.image_ref.as_str()))
    {
        return Err("IMAGE_UNAVAILABLE");
    }
    if !exists(c, name)? {
        docker(
            c,
            &[
                "create",
                "--name",
                name,
                "--label",
                &format!("saas.deployment.owner={}", c.owner),
                "--label",
                &format!("saas.deployment.target={}", c.policy.id),
                "--label",
                &format!("saas.deployment.app={}", w.application_code),
                "--label",
                &format!("saas.deployment.version={}", w.version),
                "--label",
                &format!("saas.deployment.image={}", w.image_ref),
                "--network",
                &c.network,
                "--user",
                "101:101",
                "--read-only",
                "--cap-drop",
                "ALL",
                "--security-opt",
                "no-new-privileges",
                "--pids-limit",
                "64",
                "--memory",
                "128m",
                "--cpus",
                "1",
                "--tmpfs",
                "/tmp:rw,noexec,nosuid,size=16m",
                "--log-driver",
                "local",
                "--log-opt",
                "max-size=1m",
                "--log-opt",
                "max-file=1",
                "--log-opt",
                "compress=false",
                "--entrypoint",
                "nginx",
                &w.image_ref,
                "-g",
                "daemon off;",
            ],
            30,
        )?;
    }
    let v = owned(c, w, name)?;
    if v["Image"].as_str() != Some(digest.as_str())
        || v["HostConfig"]["ReadonlyRootfs"] != true
        || v["Config"]["User"] != "101:101"
        || v["HostConfig"]["Privileged"] != false
        || v["HostConfig"]["CapDrop"] != serde_json::json!(["ALL"])
        || v["HostConfig"]["CapAdd"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
        || v["HostConfig"]["SecurityOpt"] != serde_json::json!(["no-new-privileges"])
        || v["HostConfig"]["NetworkMode"] != c.network
        || v["Config"]["Entrypoint"] != serde_json::json!(["nginx"])
        || v["Config"]["Cmd"] != serde_json::json!(["-g", "daemon off;"])
        || v["HostConfig"]["PortBindings"]
            .as_object()
            .is_some_and(|p| !p.is_empty())
        || v["HostConfig"]["Memory"] != 134217728
        || v["HostConfig"]["NanoCpus"] != 1000000000
        || v["HostConfig"]["PidsLimit"] != 64
        || v["Mounts"]
            .as_array()
            .is_none_or(|m| m.iter().any(|v| v["Type"] != "tmpfs"))
    {
        return Err("OWNERSHIP_CONFLICT");
    }
    if v["State"]["Running"] != true {
        docker(c, &["start", name], 20)?;
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if docker(
            c,
            &[
                "exec",
                name,
                "wget",
                "-q",
                "-T",
                "2",
                "-O",
                "/dev/null",
                "http://127.0.0.1:8080/healthz",
            ],
            5,
        )
        .is_ok()
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err("UNHEALTHY");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(digest)
}

pub(super) fn remove(c: &Config, w: &Work, name: &str) -> Result<()> {
    if exists(c, name)? {
        owned(c, w, name)?;
        docker(c, &["rm", "--force", name], 20).map_err(|_| "CLEANUP_FAILED")?;
    }
    Ok(())
}
