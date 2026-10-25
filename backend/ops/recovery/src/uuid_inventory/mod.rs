//! Local preview inventory is deliberately separate from backup/restore: it has
//! no write SQL, restore target, remote endpoint, or credential invalidation path.
mod catalog;

use crate::{
    config::{identifier, private_file},
    io,
    native::{create_private, Scratch},
    Error, Result,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Database {
    kind: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    profile: String,
    host: String,
    port: u16,
    user: String,
    databases: Vec<Database>,
    scratch: PathBuf,
    reference_registry: PathBuf,
    report_file: PathBuf,
    timeout_seconds: u64,
}

impl Config {
    fn validate(&self) -> Result<()> {
        let local_host = matches!(self.host.as_str(), "127.0.0.1" | "::1")
            || (Path::new(&self.host).is_absolute() && self.host.starts_with('/'));
        let kinds: BTreeSet<_> = self.databases.iter().map(|d| d.kind.as_str()).collect();
        let names: BTreeSet<_> = self.databases.iter().map(|d| &d.name).collect();
        if self.profile != "uuid-local-preview"
            || !local_host
            || self.port == 0
            || !identifier(&self.user)
            || !(10..=300).contains(&self.timeout_seconds)
            || self.databases.len() != 3
            || names.len() != 3
            || kinds != BTreeSet::from(["platform", "stores", "work_orders"])
            || self
                .databases
                .iter()
                .any(|d| !identifier(&d.name) || !d.name.contains("_preview_"))
        {
            return Err(Error(
                "only three explicit local preview sources are supported",
            ));
        }
        if !self.scratch.is_absolute()
            || !self.reference_registry.is_absolute()
            || !self.report_file.is_absolute()
            || self.report_file.exists()
            || self.report_file == self.reference_registry
        {
            return Err(Error("absolute paths and a new report file are required"));
        }
        Ok(())
    }
}

fn hash(bytes: &[u8]) -> String {
    crate::bundle::hex(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref())
}

fn query(c: &Config, scratch: &Scratch, database: &str) -> Result<Value> {
    let sql_path = scratch.0.join("inventory.sql");
    let output_path = scratch.0.join("inventory.stdout");
    let error_path = scratch.0.join("inventory.stderr");
    let mut command = Command::new("psql");
    // libpq environment must never supply a hidden remote service, write option,
    // connection string, or password. Local peer authentication is preferred.
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("PG") {
            command.env_remove(name);
        }
    }
    command
        .args([
            "-X",
            "-q",
            "-A",
            "-t",
            "--no-password",
            "-v",
            "ON_ERROR_STOP=1",
            "--host",
            &c.host,
            "--port",
            &c.port.to_string(),
            "--username",
            &c.user,
            "--dbname",
            database,
            "--file",
        ])
        .arg(&sql_path)
        .env("PGCONNECT_TIMEOUT", "10")
        .env(
            "PGOPTIONS",
            "-c default_transaction_read_only=on -c statement_timeout=10000 -c lock_timeout=3000",
        )
        .env("PGPASSFILE", "/dev/null")
        .stdin(Stdio::null())
        .stdout(Stdio::from(io(File::create(&output_path))?))
        .stderr(Stdio::from(io(File::create(&error_path))?));
    let mut child = command
        .spawn()
        .map_err(|_| Error("inventory psql unavailable"))?;
    let deadline = Instant::now() + Duration::from_secs(c.timeout_seconds);
    loop {
        if let Some(status) = io(child.try_wait())? {
            if !status.success() {
                return Err(Error("read-only inventory failed; native details redacted"));
            }
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error("inventory deadline exceeded"));
        }
        thread::sleep(Duration::from_millis(20));
    }
    if io(fs::metadata(&output_path))?.len() > 16 * 1024 * 1024 {
        return Err(Error("inventory response too large"));
    }
    serde_json::from_slice(&io(fs::read(output_path))?)
        .map_err(|_| Error("invalid inventory response"))
}

fn column_keys(columns: &Value) -> Result<BTreeSet<(String, String, String)>> {
    let columns = columns
        .as_array()
        .ok_or(Error("inventory columns must be an array"))?;
    let mut keys = BTreeSet::new();
    for column in columns {
        let key = |field| {
            column
                .get(field)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or(Error("inventory column identity missing"))
        };
        if !keys.insert((key("schema")?, key("table")?, key("column")?)) {
            return Err(Error("duplicate inventory column registration"));
        }
    }
    Ok(keys)
}

fn string_field(value: &Value, key: &str) -> Result<String> {
    value[key]
        .as_str()
        .map(str::to_owned)
        .ok_or(Error("catalog reference field missing"))
}

fn validate_foreign_keys(report: &Value, registered: &Value) -> Result<()> {
    let keys = |fks: Vec<&Value>,
                source_key: &str,
                schema_key: &str,
                table_key: &str,
                target_key: &str|
     -> Result<BTreeSet<String>> {
        let mut groups = BTreeSet::new();
        for fk in fks {
            let columns = |key| -> Result<Vec<String>> {
                fk[key]
                    .as_array()
                    .ok_or(Error("FK columns missing"))?
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(str::to_owned)
                            .ok_or(Error("FK column must be text"))
                    })
                    .collect()
            };
            let source = columns(source_key)?;
            let target = columns(target_key)?;
            if source.is_empty() || source.len() != target.len() {
                return Err(Error("FK arity mismatch"));
            }
            let group = json!([
                string_field(fk, "schema")?,
                string_field(fk, "table")?,
                source,
                string_field(fk, schema_key)?,
                string_field(fk, table_key)?,
                target
            ])
            .to_string();
            if !groups.insert(group) {
                return Err(Error("duplicate FK group"));
            }
        }
        Ok(groups)
    };
    let expected = keys(
        registered["foreignKeys"]
            .as_array()
            .ok_or(Error("registered FK groups missing"))?
            .iter()
            .collect(),
        "sourceColumns",
        "referencedSchema",
        "referencedTable",
        "targetColumns",
    )?;
    let actual = keys(
        report["constraints"]
            .as_array()
            .ok_or(Error("catalog constraints missing"))?
            .iter()
            .filter(|k| k["type"].as_str() == Some("f"))
            .collect(),
        "columns",
        "referenced_schema",
        "referenced_table",
        "referenced_columns",
    )?;
    if expected != actual {
        return Err(Error(
            "actual ordered foreign key groups differ from typed registry",
        ));
    }
    Ok(())
}

fn validate_report(database: &Database, report: &Value, registry: &Value) -> Result<()> {
    let metadata = &report["metadata"];
    if metadata["database"].as_str() != Some(database.name.as_str())
        || metadata["transaction_read_only"].as_str() != Some("on")
        || !(metadata["server_address"].is_null() && metadata.get("server_address").is_some()
            || matches!(
                metadata["server_address"].as_str(),
                Some("127.0.0.1" | "::1")
            ))
        || metadata["transaction_isolation"].as_str() != Some("repeatable read")
        || metadata["row_security"].as_str() != Some("off")
    {
        return Err(Error(
            "inventory source identity or read-only contract mismatch",
        ));
    }
    let registered = registry["databases"]
        .as_array()
        .ok_or(Error("invalid reference registry"))?
        .iter()
        .find(|d| d["kind"].as_str() == Some(database.kind.as_str()))
        .ok_or(Error("database missing from reference registry"))?;
    if column_keys(&report["uuid_columns"])? != column_keys(&registered["columns"])? {
        return Err(Error(
            "actual UUID catalog differs from typed reference registry",
        ));
    }
    // Current baseline has scalar UUIDs only. Discovery captures domains/arrays,
    // but they cannot silently inherit scalar mappings when the schema drifts.
    for column in report["uuid_columns"]
        .as_array()
        .ok_or(Error("catalog columns missing"))?
    {
        if column["declared_type"].as_str() != Some("uuid")
            || column["array_layers"].as_u64() != Some(0)
            || column["domains"]
                .as_array()
                .is_none_or(|domains| !domains.is_empty())
        {
            return Err(Error(
                "UUID domain or array requires an explicit reviewed registry extension",
            ));
        }
    }
    validate_foreign_keys(report, registered)?;
    let policies = registry["retentionPolicies"]
        .as_object()
        .ok_or(Error("retention policies missing"))?;
    for column in registered["columns"]
        .as_array()
        .ok_or(Error("invalid reference columns"))?
    {
        let authority = column["authority"]
            .as_str()
            .ok_or(Error("typed authority missing"))?;
        let role = column["role"]
            .as_str()
            .ok_or(Error("typed reference role missing"))?;
        if authority.is_empty()
            || ![
                "entity_primary_key",
                "reference_primary_key",
                "reference",
                "purpose_scoped_identifier",
                "discriminated_reference",
            ]
            .contains(&role)
            || !column["retention"]
                .as_str()
                .is_some_and(|p| policies.contains_key(p))
        {
            return Err(Error("incomplete typed reference or retention policy"));
        }
    }
    let table_keys = |tables: &Value, registered: bool| -> Result<BTreeSet<(String, String)>> {
        let mut keys = BTreeSet::new();
        for table in tables
            .as_array()
            .ok_or(Error("table registration missing"))?
        {
            let schema = table["schema"]
                .as_str()
                .ok_or(Error("table schema missing"))?;
            let name = table["table"].as_str().ok_or(Error("table name missing"))?;
            if !keys.insert((schema.to_owned(), name.to_owned()))
                || registered
                    && !table["retention"]
                        .as_str()
                        .is_some_and(|p| policies.contains_key(p))
            {
                return Err(Error("duplicate table or unknown retention policy"));
            }
        }
        Ok(keys)
    };
    if table_keys(&report["relations"], false)? != table_keys(&registered["tables"], true)? {
        return Err(Error("actual tables differ from retention registry"));
    }
    Ok(())
}

pub(crate) fn execute(path: &Path) -> Result<String> {
    let c: Config = serde_json::from_slice(&private_file(path)?)
        .map_err(|_| Error("invalid inventory configuration"))?;
    c.validate()?;
    crate::native::require_tmpfs_directory(
        c.report_file
            .parent()
            .ok_or(Error("report parent missing"))?,
    )?;
    let registry_bytes = io(fs::read(&c.reference_registry))?;
    let registry: Value = serde_json::from_slice(&registry_bytes)
        .map_err(|_| Error("invalid reference registry JSON"))?;
    let scratch = Scratch::new(&c.scratch)?;
    create_private(
        &scratch.0.join("inventory.sql"),
        catalog::query().as_bytes(),
    )?;
    let mut databases = Vec::new();
    for database in &c.databases {
        let mut report = query(&c, &scratch, &database.name)?;
        validate_report(database, &report, &registry)?;
        let material = serde_json::to_vec(&report["schema_material"])
            .map_err(|_| Error("schema serialization failed"))?;
        report["schema_fingerprint_sha256"] = json!(hash(&material));
        databases.push(json!({"kind": database.kind, "source": database.name, "catalog": report}));
    }
    let captured = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error("clock before epoch"))?
        .as_secs();
    let report = json!({
        "format": "saas-uuid-inventory/1", "environment": "local-preview",
        "captured_at_unix_seconds": captured,
        "snapshot_contract": "independent-repeatable-read-read-only; NOT a coordinated export",
        "reference_registry_sha256": hash(&registry_bytes), "databases": databases,
        "uninspected_environments": ["remote-development", "production"]
    });
    let bytes =
        serde_json::to_vec_pretty(&report).map_err(|_| Error("inventory serialization failed"))?;
    create_private(&c.report_file, &bytes)?;
    let summary: Vec<_> = report["databases"]
        .as_array()
        .ok_or(Error("inventory databases missing"))?
        .iter()
        .map(|d| {
            json!({
                "kind": d["kind"], "database": d["source"],
                "server_version_num": d["catalog"]["metadata"]["server_version_num"],
                "uuid_columns": d["catalog"]["uuid_columns"].as_array().map(Vec::len),
                "schema_fingerprint_sha256": d["catalog"]["schema_fingerprint_sha256"]
            })
        })
        .collect();
    Ok(json!({"status": "READ_ONLY_INVENTORY_VERIFIED", "report_sha256": hash(&bytes), "registry_coverage": "exact", "databases": summary}).to_string())
}

pub(crate) fn catalog_query() -> &'static str {
    catalog::query()
}

pub(crate) fn validate_catalog(
    kind: &str,
    name: &str,
    report: &Value,
    registry: &Value,
) -> Result<()> {
    validate_report(
        &Database {
            kind: kind.to_owned(),
            name: name.to_owned(),
        },
        report,
        registry,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config {
            profile: "uuid-local-preview".into(),
            host: "127.0.0.1".into(),
            port: 5432,
            user: "postgres".into(),
            databases: [
                ("platform", "platform_preview_test"),
                ("stores", "stores_preview_test"),
                ("work_orders", "orders_preview_test"),
            ]
            .into_iter()
            .map(|(kind, name)| Database {
                kind: kind.into(),
                name: name.into(),
            })
            .collect(),
            scratch: std::env::temp_dir(),
            reference_registry: std::env::temp_dir().join("registry.json"),
            report_file: std::env::temp_dir().join("not-created-uuid-inventory.json"),
            timeout_seconds: 30,
        }
    }
    #[test]
    fn restricts_inventory_to_three_explicit_local_preview_sources() {
        let mut c = config();
        assert!(c.validate().is_ok());
        c.host = "remote.example".into();
        assert!(c.validate().is_err());
        c.host = "127.0.0.1".into();
        c.databases[0].name = "platform_production".into();
        assert!(c.validate().is_err());
        c.databases[0].name = c.databases[1].name.clone();
        assert!(c.validate().is_err());
    }
    #[test]
    fn refuses_unregistered_columns_and_wrong_connection_identity() {
        let db = &config().databases[0];
        let columns = json!([{"schema":"public","table":"users","column":"id","authority":"platform.user","role":"entity_primary_key","retention":"RETAIN_REMAP","foreignKeys":[],"declared_type":"uuid","array_layers":0,"domains":[]}]);
        let tables = json!([{"schema":"public","table":"users","retention":"RETAIN_REMAP"}]);
        let mut report = json!({"metadata":{"database": db.name, "transaction_read_only":"on", "transaction_isolation":"repeatable read", "row_security":"off", "server_address":null},"uuid_columns":columns,"relations":tables,"constraints":[]});
        let registry = json!({"retentionPolicies":{"RETAIN_REMAP":{}},"databases":[{"kind":"platform","columns":columns,"tables":tables,"foreignKeys":[]}]});
        assert!(validate_report(db, &report, &registry).is_ok());
        report["uuid_columns"][0]["column"] = json!("unknown_id");
        assert!(validate_report(db, &report, &registry).is_err());
        report["uuid_columns"] = columns.clone();
        report["metadata"]["transaction_read_only"] = json!("off");
        assert!(validate_report(db, &report, &registry).is_err());
        report["metadata"]["transaction_read_only"] = json!("on");
        report["metadata"]["server_address"] = json!("10.0.0.1");
        assert!(validate_report(db, &report, &registry).is_err());
        report["metadata"]["server_address"] = Value::Null;
        report["uuid_columns"][0]["declared_type"] = json!("uuid[]");
        assert!(validate_report(db, &report, &registry).is_err());
        report["uuid_columns"] = columns;
        report["constraints"] = json!([{"type":"f","schema":"public","table":"users","columns":["id"],"referenced_schema":"public","referenced_table":"other_users","referenced_columns":["id"]}]);
        assert!(validate_report(db, &report, &registry).is_err());
        report["constraints"] = json!([]);
        report["relations"] = json!([]);
        assert!(validate_report(db, &report, &registry).is_err());
    }

    #[test]
    fn rejects_split_composite_and_non_uuid_foreign_key_drift() {
        let registry = json!({"foreignKeys":[{"schema":"public","table":"items","sourceColumns":["version","id"],"referencedSchema":"public","referencedTable":"releases","targetColumns":["version","id"]}]});
        let mut report = json!({"constraints":[{"type":"f","schema":"public","table":"items","columns":["version","id"],"referenced_schema":"public","referenced_table":"releases","referenced_columns":["version","id"]}]});
        assert!(validate_foreign_keys(&report, &registry).is_ok());
        report["constraints"] = json!([
            {"type":"f","schema":"public","table":"items","columns":["version"],"referenced_schema":"public","referenced_table":"releases","referenced_columns":["version"]},
            {"type":"f","schema":"public","table":"items","columns":["id"],"referenced_schema":"public","referenced_table":"releases","referenced_columns":["id"]}
        ]);
        assert!(validate_foreign_keys(&report, &registry).is_err());
        let registry = json!({"foreignKeys":[{"schema":"public","table":"regions","sourceColumns":["version"],"referencedSchema":"public","referencedTable":"region_versions","targetColumns":["version"]}]});
        let report = json!({"constraints":[]});
        assert!(validate_foreign_keys(&report, &registry).is_err());
    }
}
