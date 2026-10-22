use crate::{config::Config, io, Error, Result};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
pub(crate) struct Scratch(pub PathBuf);
pub(crate) fn create_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = io(options.open(path))?;
    io(f.write_all(bytes))?;
    io(f.sync_all())
}
impl Scratch {
    pub fn new(root: &Path) -> Result<Self> {
        let meta = io(fs::symlink_metadata(root))?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(Error("scratch directory required"));
        }
        let out = Command::new("findmnt")
            .args(["-n", "-o", "FSTYPE", "-T"])
            .arg(root)
            .output()
            .map_err(|_| Error("tmpfs check failed"))?;
        if !out.status.success() || String::from_utf8_lossy(&out.stdout).trim() != "tmpfs" {
            return Err(Error("plaintext scratch must be tmpfs; no disk fallback"));
        }
        let nonce = crate::bundle::random()?;
        let name = format!("recovery-{}", crate::bundle::hex(&nonce));
        let dir = root.join(name);
        io(fs::create_dir(&dir))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            io(fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)))?;
        }
        Ok(Self(dir))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
pub(crate) fn run(
    c: &Config,
    s: &Scratch,
    program: &str,
    service: &str,
    database: Option<&str>,
    args: &[&str],
) -> Result<Vec<u8>> {
    let out = s.0.join("stdout");
    let err = s.0.join("stderr");
    let mut command = Command::new(program);
    command
        .args(args)
        .env_remove("PGPASSWORD")
        .env_remove("PGDATABASE")
        .env_remove("PGOPTIONS")
        .env_remove("PGHOST")
        .env_remove("PGPORT")
        .env_remove("PGUSER")
        .env_remove("PGSERVICE")
        .env("PGSERVICEFILE", &c.service_file)
        .env("PGSERVICE", service)
        .env("PGCONNECT_TIMEOUT", "10")
        .env(
            "PGOPTIONS",
            format!(
                "-c statement_timeout={} -c lock_timeout=10000",
                c.timeout_seconds * 1000
            ),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::from(io(File::create(&out))?))
        .stderr(Stdio::from(io(File::create(&err))?));
    if let Some(db) = database {
        command.arg("--dbname").arg(db);
    }
    let mut child = command
        .spawn()
        .map_err(|_| Error("native tool unavailable"))?;
    let deadline = Instant::now() + Duration::from_secs(c.timeout_seconds);
    loop {
        if let Some(status) = io(child.try_wait())? {
            if !status.success() {
                let stderr = io(fs::read_to_string(&err))?;
                return Err(Error(if stderr.contains("symbol lookup error") {
                    "native client ABI incompatible"
                } else if stderr.contains("permission denied") {
                    "native database permission denied"
                } else if stderr.contains("does not exist") {
                    "native database object missing"
                } else if stderr.contains("could not connect") || stderr.contains("connection") {
                    "native database connection failed"
                } else if program == "pg_dump" {
                    "native pg_dump failed"
                } else if program == "pg_restore" {
                    "native pg_restore failed; targets remain quarantined"
                } else {
                    "native SQL failed; targets remain quarantined"
                }));
            }
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error("native operation deadline exceeded"));
        }
        thread::sleep(Duration::from_millis(20));
    }
    let bytes = io(fs::read(&out))?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(Error("native response too large"));
    }
    Ok(bytes)
}
pub(crate) fn sql(
    c: &Config,
    s: &Scratch,
    service: &str,
    db: Option<&str>,
    query: &str,
) -> Result<String> {
    let bytes = run(
        c,
        s,
        "psql",
        service,
        db,
        &["-X", "-A", "-t", "-v", "ON_ERROR_STOP=1", "-c", query],
    )?;
    String::from_utf8(bytes).map_err(|_| Error("invalid native response"))
}
