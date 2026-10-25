use crate::{
    bundle::{self, Bundle, Database},
    config::{private_file, Config},
    io,
    native::{self, Scratch},
    Error, Result,
};
use std::{
    fs,
    io::Read,
    path::Path,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
const MIGRATIONS:&str="SELECT version::text||':'||encode(checksum,'hex')||':'||success::text FROM _sqlx_migrations ORDER BY version";
const RESET: &str = include_str!("../invalidate.sql");
fn now() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|t| t.as_secs())
        .map_err(|_| Error("clock before epoch"))
}
fn token(path: &Path) -> Result<String> {
    String::from_utf8(private_file(path)?)
        .map(|s| s.trim().to_owned())
        .map_err(|_| Error("invalid token file"))
}
fn remote(c: &Config, upload: Option<&[u8]>) -> Result<Vec<u8>> {
    let client = c.client()?;
    let t = token(&c.remote_token_file)?;
    if let Some(bytes) = upload {
        let result = client
            .put(&c.remote_url)
            .bearer_auth(&t)
            .header("If-None-Match", "*")
            .body(bytes.to_vec())
            .send()
            .map_err(|_| Error("remote upload unavailable"))?;
        if result.status() != reqwest::StatusCode::CREATED {
            return Err(Error("remote immutable publication refused"));
        }
    }
    let mut reply = client
        .get(&c.remote_url)
        .bearer_auth(t)
        .send()
        .map_err(|_| Error("remote copy unavailable"))?;
    if reply.status() != reqwest::StatusCode::OK {
        return Err(Error("remote copy unavailable"));
    }
    if reply
        .content_length()
        .is_some_and(|len| len > bundle::MAX + 64)
    {
        return Err(Error("remote bundle too large"));
    }
    let mut bytes = Vec::new();
    io(Read::by_ref(&mut reply)
        .take(bundle::MAX + 65)
        .read_to_end(&mut bytes))?;
    if bytes.len() as u64 > bundle::MAX + 64 {
        return Err(Error("remote bundle too large"));
    }
    Ok(bytes)
}
fn event(c: &Config, component: &str, healthy: bool) -> Result<()> {
    // The operation lock serializes this durable sequence across timer/CLI runs.
    let previous = if c.sequence_file.exists() {
        String::from_utf8(private_file(&c.sequence_file)?)
            .map_err(|_| Error("sequence invalid"))?
            .parse::<u64>()
            .map_err(|_| Error("sequence invalid"))?
    } else {
        0
    };
    let sequence = previous.checked_add(1).ok_or(Error("sequence exhausted"))?;
    let temporary = c.sequence_file.with_extension("next");
    native::create_private(&temporary, sequence.to_string().as_bytes())?;
    io(fs::rename(&temporary, &c.sequence_file))?;
    let body = serde_json::json!({"component":component,"healthy":healthy,"sequence":sequence});
    println!(
        "{}",
        serde_json::json!({"component":component,"healthy":healthy,"sequence":sequence,"local_event":true})
    );
    let reply = c
        .client()?
        .post(&c.monitor_url)
        .bearer_auth(token(&c.monitor_token_file)?)
        .json_body(&body)?;
    if !reply {
        return Err(Error("monitor event rejected"));
    }
    Ok(())
}
trait JsonBody {
    fn json_body(self, body: &serde_json::Value) -> Result<bool>;
}
impl JsonBody for reqwest::blocking::RequestBuilder {
    fn json_body(self, body: &serde_json::Value) -> Result<bool> {
        let reply = self
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .map_err(|_| Error("monitor event unavailable"))?;
        Ok(reply.status().is_success())
    }
}
struct Lock(std::path::PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.0);
    }
}
/// Execute a bounded offline operation using a private, operator-owned config.
/// `uuid-inventory` instead accepts the separate local-preview read-only config.
/// Production PITR is deliberately rejected: this implementation accepts only
/// the pilot 24h/4h dump profile. Errors retain any created targets and encrypted files.
pub fn execute(operation: &str, path: &Path) -> Result<String> {
    if operation == "uuid-config-rekey" {
        return crate::uuid_rekey::config_rekey::execute(path);
    }
    if operation == "uuid-inventory" {
        return crate::uuid_inventory::execute(path);
    }
    if operation == "uuid-export" {
        return crate::uuid_rekey::export::execute(path);
    }
    if operation == "uuid-transform" {
        return crate::uuid_rekey::transform::execute(path);
    }
    if operation == "uuid-verify-package" {
        return crate::uuid_rekey::verify::execute(path);
    }
    if operation == "uuid-import" {
        return crate::uuid_rekey::import::execute(path);
    }
    if operation == "uuid-seal-evidence" {
        return crate::uuid_rekey::seal_evidence(path);
    }
    if operation == "uuid-open-evidence" {
        return crate::uuid_rekey::evidence::execute(path);
    }
    if !["backup", "backup-new", "verify", "age", "restore"].contains(&operation) {
        return Err(Error("unknown operation"));
    }
    let mut c = Config::load(path)?;
    let scheduled = operation == "backup-new";
    let operation = if scheduled { "backup" } else { operation };
    if scheduled {
        let suffix = format!("-{}-{}", now()?, bundle::hex(&bundle::random()?));
        c.remote_url.push_str(&suffix);
        let mut local = c.local_bundle.as_os_str().to_os_string();
        local.push(&suffix);
        c.local_bundle = local.into();
    }
    let lock_path = c.sequence_file.with_extension("lock");
    io(fs::create_dir(&lock_path))?;
    let _lock = Lock(lock_path);
    let component = match operation {
        "backup" => "backup_run",
        "age" => "backup_age",
        _ => "backup_verify",
    };
    let started = Instant::now();
    let result = perform(&c, operation).and_then(|report| {
        if scheduled {
            let pointer = c.sequence_file.with_extension("latest.json");
            let temporary = c.sequence_file.with_extension("latest.next");
            native::create_private(
                &temporary,
                &serde_json::to_vec(&c).map_err(|_| Error("latest config serialization failed"))?,
            )?;
            io(fs::rename(temporary, pointer))?;
        }
        Ok(report)
    });
    // A monitoring failure makes the command fail, even if the data operation succeeded.
    let delivery = event(&c, component, result.is_ok());
    if delivery.is_err() && result.is_err() {
        eprintln!("monitor delivery also failed; original recovery error follows");
    }
    if result.is_ok() {
        delivery?;
    }
    result.map(|mut report| {
        report["operation_seconds"] = serde_json::json!(started.elapsed().as_secs_f64());
        report.to_string()
    })
}
fn perform(c: &Config, op: &str) -> Result<serde_json::Value> {
    let key = private_file(&c.key_file)?;
    if key.len() != 32 {
        return Err(Error("key must contain 32 random bytes"));
    }
    if op == "backup" {
        if std::env::var("BACKUP_WRITES_QUIESCED").as_deref() != Ok("true") {
            return Err(Error(
                "coordinated writer quiescence must be explicitly confirmed",
            ));
        }
        if c.local_bundle.exists() {
            return Err(Error("local bundle already exists"));
        }
        let scratch = Scratch::new(&c.scratch)?;
        let mut b = Bundle {
            format: 1,
            recovery_point: now()?,
            databases: vec![],
            entries: vec![],
        };
        for d in &c.databases {
            let name = native::sql(
                c,
                &scratch,
                &d.source_service,
                None,
                "SELECT current_database()",
            )?
            .trim()
            .to_owned();
            let migrations = native::sql(c, &scratch, &d.source_service, None, MIGRATIONS)?;
            let version = native::sql(c, &scratch, &d.source_service, None, "SHOW server_version")?;
            let catalog = if d.kind == "platform" {
                String::new()
            } else {
                crate::schema::capture(c, &scratch, &d.source_service, None)?
            };
            let dump = scratch.0.join(format!("{}.dump", d.kind));
            let output = dump.to_str().ok_or(Error("non UTF-8 scratch"))?;
            native::run(
                c,
                &scratch,
                "pg_dump",
                &d.source_service,
                None,
                &[
                    "--format=custom",
                    "--no-owner",
                    "--no-privileges",
                    "--file",
                    output,
                ],
            )?;
            let meta = io(fs::metadata(&dump))?;
            if meta.len() > bundle::MAX
                || b.entries.iter().map(|e| e.data.len() as u64).sum::<u64>() + meta.len() * 4 / 3
                    > bundle::MAX
            {
                return Err(Error("dump capacity exceeded"));
            }
            b.entries.push(bundle::entry(
                format!("db/{}.dump", d.kind),
                io(fs::read(&dump))?,
            ));
            io(fs::remove_file(dump))?;
            b.databases.push(Database {
                kind: d.kind.clone(),
                source_name: name,
                migrations,
                server_version: version,
                catalog,
            });
        }
        bundle::collect(&c.artifact_root, &c.artifact_root, &mut b.entries)?;
        bundle::validate(&b)?;
        let encrypted = bundle::seal(&b, &key)?;
        native::create_private(&c.local_bundle, &encrypted)?;
        let retrieved = remote(c, Some(&encrypted))?;
        if bundle::hash(&retrieved) != bundle::hash(&encrypted) {
            return Err(Error("remote copy differs"));
        }
        bundle::open(&retrieved, &key)?;
        return Ok(
            serde_json::json!({"status":"PASS","profile":"pilot","remote_retrieval_authenticated":true,"recovery_point":b.recovery_point,"bundle_sha256":bundle::hash(&retrieved),"entries":b.entries.len()}),
        );
    }
    // Always fetch the remote object; a same-host cached dump cannot satisfy acceptance.
    let retrieved = remote(c, None)?;
    let b = bundle::open(&retrieved, &key)?;
    let age = now()?
        .checked_sub(b.recovery_point)
        .ok_or(Error("future recovery point refused"))?;
    if op == "age" && age > 86400 {
        return Err(Error("remote recovery point older than pilot RPO"));
    }
    if op != "restore" {
        return Ok(
            serde_json::json!({"status":"PASS","profile":"pilot","remote_retrieval_authenticated":true,"recovery_point":b.recovery_point,"age_seconds":age,"entries":b.entries.len(),"bundle_sha256":bundle::hash(&retrieved)}),
        );
    }
    let scratch = Scratch::new(&c.scratch)?;
    if c.restored_artifacts.exists() {
        return Err(Error("artifact target must be new"));
    }
    // All existence/source checks precede the first CREATE DATABASE. CREATE itself
    // refuses a racing existing target; no --clean, DROP or destructive retry exists.
    for d in &c.databases {
        if b.databases.iter().any(|s| s.source_name == d.target) {
            return Err(Error("source and target must differ"));
        }
        let live_source = native::sql(
            c,
            &scratch,
            &d.source_service,
            None,
            "SELECT current_database()",
        )?;
        if live_source.trim() == d.target {
            return Err(Error("source and target must differ"));
        }
        let exists = native::sql(
            c,
            &scratch,
            &c.admin_service,
            None,
            &format!(
                "SELECT count(*) FROM pg_database WHERE datname='{}'",
                d.target
            ),
        )?;
        if exists.trim() != "0" {
            return Err(Error("existing target refused"));
        }
        let owner = native::sql(
            c,
            &scratch,
            &c.admin_service,
            None,
            &format!("SELECT count(*) FROM pg_roles WHERE rolname='{}' AND NOT rolsuper AND NOT rolcreatedb AND NOT rolcreaterole AND NOT rolreplication", d.owner),
        )?;
        if owner.trim() != "1" {
            return Err(Error("target owner must be provisioned separately"));
        }
    }
    for d in &c.databases {
        native::sql(
            c,
            &scratch,
            &c.admin_service,
            None,
            &format!(
                "CREATE DATABASE \"{}\" OWNER \"{}\" TEMPLATE template0",
                d.target, d.owner
            ),
        )?;
        let e = b
            .entries
            .iter()
            .find(|e| e.path == format!("db/{}.dump", d.kind))
            .ok_or(Error("dump missing"))?;
        let dump = scratch.0.join("restore.dump");
        native::create_private(&dump, &bundle::decode(e)?)?;
        native::run(
            c,
            &scratch,
            "pg_restore",
            &c.admin_service,
            Some(&d.target),
            &[
                "--dbname",
                &d.target,
                "--no-owner",
                "--no-privileges",
                "--single-transaction",
                "--exit-on-error",
                "--role",
                &d.owner,
                dump.to_str().ok_or(Error("non UTF-8 scratch"))?,
            ],
        )?;
        io(fs::remove_file(dump))?;
        if d.kind != "platform" {
            crate::schema::canonicalize(c, &scratch, &d.kind, &d.target)?;
            let restored_catalog =
                crate::schema::capture(c, &scratch, &c.admin_service, Some(&d.target))?;
            if b.databases
                .iter()
                .find(|s| s.kind == d.kind)
                .map(|s| s.catalog.as_str())
                != Some(restored_catalog.as_str())
            {
                return Err(Error(
                    "restored business catalog differs from authenticated source",
                ));
            }
        }
        let actual = native::sql(c, &scratch, &c.admin_service, Some(&d.target), MIGRATIONS)?;
        if b.databases
            .iter()
            .find(|s| s.kind == d.kind)
            .map(|s| s.migrations.as_str())
            != Some(actual.as_str())
        {
            return Err(Error("restored migration fingerprint mismatch"));
        }
        if d.kind == "platform" {
            native::sql(c, &scratch, &c.admin_service, Some(&d.target), RESET)?;
        }
    }
    io(fs::create_dir(&c.restored_artifacts))?;
    for e in &b.entries {
        if let Some(relative) = e.path.strip_prefix("artifacts/") {
            let dest = c.restored_artifacts.join(relative);
            io(fs::create_dir_all(
                dest.parent().ok_or(Error("artifact parent missing"))?,
            ))?;
            native::create_private(&dest, &bundle::decode(e)?)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                io(fs::set_permissions(
                    &dest,
                    fs::Permissions::from_mode(if e.executable { 0o755 } else { 0o644 }),
                ))?;
            }
        }
    }
    Ok(
        serde_json::json!({"status":"QUARANTINED","profile":"pilot","recovery_point":b.recovery_point,"age_seconds":age,"credentials_invalidated":true,"production_pitr":"NOT_ACCEPTED","business_acceptance_required":true}),
    )
}
