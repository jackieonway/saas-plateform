//! Coordinated, read-only local-preview exports. Row plaintext exists only in
//! private tmpfs scratch and is sealed table by table before it is removed.
#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports, unused_mut))]
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

mod admission;
pub(crate) use admission::{validate_admission, Proof as AdmissionProof};

pub(crate) const SNAPSHOT_FORMAT: &str = "saas-uuid-source-snapshot/1";
pub(crate) const MANIFEST_PURPOSE: &str = "saas-uuid-source-manifest/1";

/// All manifest keys use snake_case. Rows use the explicitly named createdAtMs.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Snapshot {
    pub(crate) format: String,
    pub(crate) batch_id: Uuid,
    pub(crate) frozen_ms: u64,
    pub(crate) registry_sha256: String,
    pub(crate) databases: Vec<DatabaseSnapshot>,
    pub(crate) admission_before: Vec<AdmissionProof>,
    pub(crate) admission_after: Vec<AdmissionProof>,
    pub(crate) quiescence_sha256: String,
    pub(crate) quiescence_file: String,
    pub(crate) quiescence_purpose: String,
    pub(crate) quiescence_digest: super::codec::FileDigest,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DatabaseSnapshot {
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) catalog: Value,
    pub(crate) snapshot: String,
    pub(crate) lsn: String,
    pub(crate) final_lsn: String,
    pub(crate) sequences: Vec<SequenceState>,
    pub(crate) tables: Vec<TableSnapshot>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SequenceState {
    pub(crate) schema: String,
    pub(crate) name: String,
    pub(crate) last_value: i64,
    pub(crate) is_called: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TableSnapshot {
    pub(crate) schema: String,
    pub(crate) table: String,
    pub(crate) rows: u64,
    /// Relative filename inside the newly created encrypted export directory.
    pub(crate) file: String,
    pub(crate) purpose: String,
    pub(crate) digest: super::codec::FileDigest,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RowEnvelope {
    pub(crate) row: Value,
    #[serde(rename = "createdAtMs", deserialize_with = "nullable_millis")]
    pub(crate) created_at_ms: Option<i64>,
}

use crate::{
    config::private_file,
    io,
    native::{create_private, require_tmpfs_directory, Scratch},
};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_LINE: usize = 16 * 1024 * 1024;
const MAX_REGISTRY: u64 = 4 * 1024 * 1024;
const QUIESCENCE_FORMAT: &str = "saas-uuid-quiescence/1";
const SOURCES: [(&str, &str); 3] = [
    ("platform", "qiankun_platform_preview_20261024"),
    ("stores", "qiankun_stores_preview_20261024"),
    ("work_orders", "qiankun_orders_preview_20261024"),
];
const WRITERS: [&str; 6] = [
    "qiankun-platform-preview-20261024",
    "qiankun-stores-preview-20261024",
    "codex-qiankun-orders-preview-api-20261024",
    "qiankun-preview-notification-20261024",
    "qiankun-preview-examples-oauth-20261024",
    "qiankun-preview-examples-chat-20261024",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DatabaseConfig {
    kind: String,
    name: String,
    expected_schema_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportConfig {
    profile: String,
    host: String,
    port: u16,
    user: String,
    databases: Vec<DatabaseConfig>,
    scratch: PathBuf,
    registry: PathBuf,
    key_file: PathBuf,
    output_dir: PathBuf,
    quiescence_file: PathBuf,
    timeout_seconds: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QuiescentDatabase {
    kind: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Quiescence {
    format: String,
    profile: String,
    writes_quiesced: bool,
    databases: Vec<QuiescentDatabase>,
    stopped_writers: Vec<String>,
    captured_ms: u64,
    writer_evidence_sha256: String,
}

fn nullable_millis<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<i64>, D::Error> {
    Option::<i64>::deserialize(deserializer)
}

fn strict_json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let text = std::str::from_utf8(bytes).map_err(|_| Error("export JSON must be valid UTF-8"))?;
    let value = super::structured::parse_strict(text)?;
    serde_json::from_value(value).map_err(|_| Error("invalid typed export JSON contract"))
}

fn hash(bytes: &[u8]) -> String {
    crate::bundle::hash(bytes)
}

fn is_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn safe_path(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        return Err(Error("absolute normalized export paths required"));
    }
    for parent in path.ancestors() {
        match fs::symlink_metadata(parent) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(Error("export symlink paths rejected"))
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Error("export path inspection failed")),
        }
    }
    Ok(())
}

impl ExportConfig {
    fn validate_connection(&self) -> Result<()> {
        let local = matches!(self.host.as_str(), "127.0.0.1" | "::1")
            || self.host.starts_with('/') && Path::new(&self.host).is_absolute();
        let kinds: BTreeSet<_> = self.databases.iter().map(|db| db.kind.as_str()).collect();
        let names: BTreeSet<_> = self.databases.iter().map(|db| db.name.as_str()).collect();
        if self.profile != "uuid-local-preview"
            || !local
            || self.port == 0
            || self.user != "postgres"
            || !(10..=1800).contains(&self.timeout_seconds)
            || self.databases.len() != 3
            || names.len() != 3
            || kinds != BTreeSet::from(["platform", "stores", "work_orders"])
            || self.databases.iter().any(|db| {
                !SOURCES.contains(&(db.kind.as_str(), db.name.as_str()))
                    || !is_hash(&db.expected_schema_sha256)
            })
        {
            return Err(Error(
                "three explicit local preview sources and reviewed schema hashes required",
            ));
        }
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        self.validate_connection()?;
        if self.host.starts_with('/') {
            safe_path(Path::new(&self.host))?;
        }
        for path in [
            &self.scratch,
            &self.registry,
            &self.key_file,
            &self.output_dir,
            &self.quiescence_file,
        ] {
            safe_path(path)?;
        }
        if self.key_file.starts_with(&self.output_dir)
            || self.registry.starts_with(&self.output_dir)
            || self.quiescence_file.starts_with(&self.output_dir)
            || fs::symlink_metadata(&self.output_dir).is_ok()
        {
            return Err(Error("new separate encrypted output directory required"));
        }
        Ok(())
    }
}

impl Quiescence {
    fn validate(&self, config: &ExportConfig, now_ms: u64) -> Result<()> {
        let expected: BTreeSet<_> = config
            .databases
            .iter()
            .map(|db| (db.kind.as_str(), db.name.as_str()))
            .collect();
        let actual: BTreeSet<_> = self
            .databases
            .iter()
            .map(|db| (db.kind.as_str(), db.name.as_str()))
            .collect();
        let writers: BTreeSet<_> = self.stopped_writers.iter().map(String::as_str).collect();
        if self.format != QUIESCENCE_FORMAT
            || self.profile != "uuid-local-preview"
            || !self.writes_quiesced
            || self.databases.len() != 3
            || expected != actual
            || self.stopped_writers.len() != WRITERS.len()
            || writers != BTreeSet::from(WRITERS)
            || !is_hash(&self.writer_evidence_sha256)
            || self.captured_ms == 0
            || self.captured_ms > now_ms
            || now_ms - self.captured_ms > 3_600_000
            || self.captured_ms >= (1_u64 << 48)
        {
            return Err(Error("recent matching writer-quiescence evidence required"));
        }
        Ok(())
    }
}

/// Validate the decrypted, authenticated operational evidence against its source
/// manifest. Offline validation does not impose a new wall-clock expiry.
pub(crate) fn validate_quiescence(snapshot: &Snapshot, bytes: &[u8]) -> Result<()> {
    let evidence: Quiescence = strict_json(bytes)?;
    let expected: BTreeSet<_> = snapshot
        .databases
        .iter()
        .map(|db| (db.kind.as_str(), db.name.as_str()))
        .collect();
    let actual: BTreeSet<_> = evidence
        .databases
        .iter()
        .map(|db| (db.kind.as_str(), db.name.as_str()))
        .collect();
    let kinds: BTreeSet<_> = snapshot
        .databases
        .iter()
        .map(|db| db.kind.as_str())
        .collect();
    let names: BTreeSet<_> = snapshot
        .databases
        .iter()
        .map(|db| db.name.as_str())
        .collect();
    let writers: BTreeSet<_> = evidence
        .stopped_writers
        .iter()
        .map(String::as_str)
        .collect();
    if snapshot.format != SNAPSHOT_FORMAT
        || snapshot.batch_id.get_version_num() != 7
        || snapshot.batch_id.get_variant() != uuid::Variant::RFC4122
        || snapshot.databases.len() != 3
        || expected.len() != 3
        || names.len() != 3
        || kinds != BTreeSet::from(["platform", "stores", "work_orders"])
        || evidence.format != QUIESCENCE_FORMAT
        || evidence.profile != "uuid-local-preview"
        || !evidence.writes_quiesced
        || evidence.databases.len() != 3
        || actual != expected
        || evidence.stopped_writers.len() != WRITERS.len()
        || writers != BTreeSet::from(WRITERS)
        || evidence.captured_ms != snapshot.frozen_ms
        || evidence.captured_ms == 0
        || evidence.captured_ms >= (1_u64 << 48)
        || !is_hash(&evidence.writer_evidence_sha256)
        || snapshot.quiescence_file != "quiescence.enc"
        || snapshot.quiescence_purpose
            != format!("saas-uuid-source-quiescence/1/{}", snapshot.batch_id)
        || hash(bytes) != snapshot.quiescence_sha256
        || snapshot.quiescence_digest.sha256 != snapshot.quiescence_sha256
        || snapshot.quiescence_digest.plaintext_bytes != bytes.len() as u64
    {
        return Err(Error(
            "quiescence body differs from its authenticated source manifest",
        ));
    }
    Ok(())
}

fn read_registry(path: &Path) -> Result<Vec<u8>> {
    safe_path(path)?;
    let metadata = io(fs::symlink_metadata(path))?;
    if !metadata.is_file() || metadata.len() > MAX_REGISTRY {
        return Err(Error("bounded regular reference registry required"));
    }
    io(fs::read(path))
}

fn create_output(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    io(builder.create(path))?;
    verify_owner_mode(&io(fs::metadata(path))?, 0o700)
}

fn new_plaintext(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = io(options.open(path))?;
    verify_owner_mode(&io(file.metadata())?, 0o600)?;
    Ok(file)
}

fn verify_owner_mode(metadata: &fs::Metadata, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let uid = io(fs::metadata("/proc/self"))?.uid();
        if metadata.uid() != uid || metadata.mode() & 0o777 != mode {
            return Err(Error(
                "export artifacts require actual private ownership and modes",
            ));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (metadata, mode);
    }
    Ok(())
}

fn sql_identifier(value: &str) -> Result<String> {
    if value.is_empty()
        || value.len() > 63
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        || value.as_bytes()[0].is_ascii_digit()
    {
        return Err(Error("unreviewed SQL identifier in export catalog"));
    }
    Ok(format!("\"{value}\""))
}

struct TablePlan {
    schema: String,
    table: String,
    columns: BTreeSet<String>,
    primary_key: Vec<String>,
    time_column: Option<String>,
    expected_rows: u64,
}

fn text_field<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or(Error("export catalog identity missing"))
}

fn plans(catalog: &Value) -> Result<Vec<TablePlan>> {
    let relations = catalog["relations"]
        .as_array()
        .ok_or(Error("export table catalog missing"))?;
    let material = catalog["schema_material"]
        .as_array()
        .ok_or(Error("export column catalog missing"))?;
    let constraints = catalog["constraints"]
        .as_array()
        .ok_or(Error("export key catalog missing"))?;
    let mut output = Vec::new();
    let mut identities = BTreeSet::new();
    for relation in relations {
        let schema = text_field(relation, "schema")?;
        let table = text_field(relation, "table")?;
        sql_identifier(schema)?;
        sql_identifier(table)?;
        if !identities.insert((schema, table)) {
            return Err(Error("duplicate export table registration"));
        }
        let mut columns = BTreeMap::new();
        for column in material.iter().filter(|item| {
            item["kind"] == "column" && item["schema"] == schema && item["table"] == table
        }) {
            let name = text_field(column, "column")?;
            sql_identifier(name)?;
            if columns
                .insert(name.to_owned(), text_field(column, "type")?.to_owned())
                .is_some()
            {
                return Err(Error("duplicate export column registration"));
            }
        }
        let keys: Vec<_> = constraints
            .iter()
            .filter(|item| {
                item["type"] == "p" && item["schema"] == schema && item["table"] == table
            })
            .collect();
        if columns.is_empty() || keys.len() != 1 || keys[0]["validated"] != true {
            return Err(Error(
                "every exported table requires its actual validated primary key",
            ));
        }
        let primary_key: Vec<String> = keys[0]["columns"]
            .as_array()
            .ok_or(Error("export primary-key columns missing"))?
            .iter()
            .map(|column| {
                column
                    .as_str()
                    .map(str::to_owned)
                    .ok_or(Error("invalid primary-key column"))
            })
            .collect::<Result<_>>()?;
        if primary_key.is_empty()
            || primary_key
                .iter()
                .any(|column| !columns.contains_key(column))
            || primary_key.iter().collect::<BTreeSet<_>>().len() != primary_key.len()
        {
            return Err(Error("export primary key differs from table columns"));
        }
        let time_column = columns
            .get("created_at")
            .filter(|kind| kind.as_str() == "timestamp with time zone")
            .map(|_| "created_at".to_owned());
        output.push(TablePlan {
            schema: schema.into(),
            table: table.into(),
            columns: columns.into_keys().collect(),
            primary_key,
            time_column,
            expected_rows: relation["row_count"]
                .as_u64()
                .ok_or(Error("export row count missing"))?,
        });
    }
    output.sort_by(|a, b| (&a.schema, &a.table).cmp(&(&b.schema, &b.table)));
    Ok(output)
}

/// Ordered sequence definitions must be the exact same objects captured in the
/// reviewed schema material; state is read separately because it is not MVCC.
pub(crate) fn sequence_names(catalog: &Value) -> Result<Vec<(String, String)>> {
    let names = |items: Vec<&Value>| -> Result<BTreeMap<(String, String), Value>> {
        let mut output = BTreeMap::new();
        for item in items {
            let schema = text_field(item, "schema")?;
            let name = text_field(item, "name")?;
            sql_identifier(schema)?;
            sql_identifier(name)?;
            if item["kind"] != "sequence"
                || output
                    .insert((schema.into(), name.into()), item.clone())
                    .is_some()
            {
                return Err(Error("duplicate or unregistered sequence definition"));
            }
        }
        Ok(output)
    };
    let actual = names(
        catalog["sequences"]
            .as_array()
            .ok_or(Error("source sequence definitions missing"))?
            .iter()
            .collect(),
    )?;
    let material = names(
        catalog["schema_material"]
            .as_array()
            .ok_or(Error("source schema material missing"))?
            .iter()
            .filter(|item| item["kind"] == "sequence")
            .collect(),
    )?;
    if actual != material {
        return Err(Error(
            "source sequences differ from reviewed schema material",
        ));
    }
    Ok(actual.into_keys().collect())
}

fn sequence_script(names: &[(String, String)], phase: &str) -> Result<String> {
    if !matches!(phase, "before" | "after") {
        return Err(Error("unknown sequence capture phase"));
    }
    let mut sql = String::new();
    for (schema, name) in names {
        let relation = format!("{}.{}", sql_identifier(schema)?, sql_identifier(name)?);
        sql.push_str(&format!("SELECT jsonb_build_object('kind','sequence','phase','{phase}','schema','{schema}','name','{name}',\
            'last_value',last_value::bigint,'is_called',is_called) FROM {relation};\n"));
    }
    Ok(sql)
}

fn catalog_body() -> Result<&'static str> {
    crate::uuid_inventory::catalog_query()
        .trim()
        .strip_prefix("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;")
        .and_then(|sql| sql.trim().strip_suffix("COMMIT;"))
        .and_then(|sql| sql.trim_start().strip_prefix("SET TRANSACTION READ ONLY;"))
        .ok_or(Error("unsupported read-only catalog script contract"))
}

fn fence_sql(config: &ExportConfig, kind: &str) -> String {
    let sources = config
        .databases
        .iter()
        .map(|db| format!("'{}'", db.name))
        .collect::<Vec<_>>()
        .join(",");
    format!("SELECT pg_catalog.jsonb_build_object('kind','{kind}','database',current_database(),\
        'snapshot',pg_catalog.pg_current_snapshot()::text,'lsn',pg_catalog.pg_current_wal_lsn()::text,\
        'default_read_only',current_setting('default_transaction_read_only'),\
        'database_read_only',EXISTS(SELECT 1 FROM pg_catalog.pg_db_role_setting s \
        JOIN pg_catalog.pg_database d ON d.oid=s.setdatabase WHERE d.datname=current_database() \
        AND s.setrole=0 AND s.setconfig @> ARRAY['default_transaction_read_only=on']),\
        'other_connections',(SELECT count(*) FROM pg_catalog.pg_stat_activity \
        WHERE datname IN ({sources}) AND pid<>pg_backend_pid()),\
        'prepared_transactions',(SELECT count(*) FROM pg_catalog.pg_prepared_xacts p WHERE p.database IN ({sources})));\n")
}

fn catalog_script(config: &ExportConfig) -> Result<String> {
    Ok(format!(
        "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\n\
        SET TRANSACTION READ ONLY;\nSET LOCAL row_security=off;\nSET LOCAL timezone='UTC';\n\
        SET LOCAL search_path=pg_catalog;\nSET LOCAL stats_fetch_consistency=none;\n{}{}\n",
        fence_sql(config, "snapshot"),
        catalog_body()?
    ))
}

fn table_script(
    plans: &[TablePlan],
    config: &ExportConfig,
    sequences: &[(String, String)],
) -> Result<String> {
    let mut script = format!(
        "SET LOCAL statement_timeout='{}s';\n",
        config.timeout_seconds
    );
    script.push_str(&sequence_script(sequences, "before")?);
    for plan in plans {
        let relation = format!(
            "{}.{}",
            sql_identifier(&plan.schema)?,
            sql_identifier(&plan.table)?
        );
        let order = plan
            .primary_key
            .iter()
            .map(|column| sql_identifier(column).map(|column| format!("t.{column}")))
            .collect::<Result<Vec<_>>>()?
            .join(",");
        let created = match &plan.time_column {
            Some(column) => {
                let column = sql_identifier(column)?;
                format!("CASE WHEN pg_catalog.isfinite(t.{column}) THEN floor(extract(epoch FROM t.{column})*1000)::bigint ELSE NULL END")
            }
            None => "NULL::bigint".into(),
        };
        script.push_str(&format!("SELECT jsonb_build_object('kind','table_begin','schema','{}','table','{}');\n\
            SELECT jsonb_build_object('kind','row','row',to_jsonb(t),'createdAtMs',{created}) \
            FROM ONLY {relation} t ORDER BY {order};\n\
            SELECT jsonb_build_object('kind','table_end','schema','{}','table','{}','rows',count(*)) FROM ONLY {relation};\n",
            plan.schema,plan.table,plan.schema,plan.table));
    }
    // Repeat catalog/count capture and the transaction/source fence before COMMIT.
    // Cluster WAL is recorded separately from the application admission proof.
    script.push_str(catalog_body()?);
    script.push_str(&sequence_script(sequences, "after")?);
    script.push_str(&fence_sql(config, "done"));
    script.push_str("COMMIT;\n");
    Ok(script)
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Event {
    Sequence {
        phase: String,
        schema: String,
        name: String,
        last_value: i64,
        is_called: bool,
    },
    Snapshot {
        database: String,
        snapshot: String,
        lsn: String,
        default_read_only: String,
        database_read_only: bool,
        other_connections: u64,
        prepared_transactions: u64,
    },
    TableBegin {
        schema: String,
        table: String,
    },
    Row {
        row: Value,
        #[serde(rename = "createdAtMs", deserialize_with = "nullable_millis")]
        created_at_ms: Option<i64>,
    },
    TableEnd {
        schema: String,
        table: String,
        rows: u64,
    },
    Done {
        database: String,
        snapshot: String,
        lsn: String,
        default_read_only: String,
        database_read_only: bool,
        other_connections: u64,
        prepared_transactions: u64,
    },
}

fn parse_event(bytes: &[u8]) -> Result<Event> {
    strict_json(bytes)
}

#[derive(Clone)]
struct SourceFence {
    snapshot: String,
    lsn: String,
}

fn validate_fence(event: Event, database_name: &str, initial: bool) -> Result<SourceFence> {
    match (event,initial) {
        (Event::Snapshot { database,snapshot,lsn,default_read_only,database_read_only,other_connections,prepared_transactions },true)
        | (Event::Done { database,snapshot,lsn,default_read_only,database_read_only,other_connections,prepared_transactions },false)
            if database==database_name && default_read_only=="on" && database_read_only
                && other_connections==0 && prepared_transactions==0
                && !snapshot.is_empty() && !lsn.is_empty() => Ok(SourceFence { snapshot,lsn }),
        _=>Err(Error("source is not database-read-only, exclusively quiescent and free of prepared transactions")),
    }
}

fn bounded_line<R: BufRead>(reader: &mut R) -> Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let available = reader
            .fill_buf()
            .map_err(|_| Error("export stream read failed"))?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(Error("unterminated export stream record"))
            };
        }
        let used = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |at| at + 1);
        if line.len().saturating_add(used) > MAX_LINE {
            return Err(Error("export stream record exceeds bounded limit"));
        }
        let complete = available[used - 1] == b'\n';
        line.extend_from_slice(&available[..used]);
        reader.consume(used);
        if complete {
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if line.is_empty() {
                return Err(Error("empty export stream record"));
            }
            return Ok(Some(line));
        }
    }
}

struct Session {
    child: Option<Child>,
    input: Option<SyncSender<String>>,
    output: Option<Receiver<Result<Vec<u8>>>>,
    workers: Vec<JoinHandle<Result<()>>>,
    deadline: Instant,
    fence: Option<SourceFence>,
    final_lsn: Option<String>,
    sequence_names: Vec<(String, String)>,
    sequence_states: Vec<SequenceState>,
}

impl Session {
    fn start(config: &ExportConfig, scratch: &Scratch, database: &str) -> Result<Self> {
        let error = new_plaintext(&scratch.0.join(format!("{database}.stderr")))?;
        let mut command = Command::new("psql");
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("PG") {
                command.env_remove(name);
            }
        }
        command.args(["-X", "-q", "-A", "-t", "--no-password", "-v", "ON_ERROR_STOP=1", "-v", "FETCH_COUNT=1000",
            "--host", &config.host, "--port", &config.port.to_string(), "--username", &config.user,
            "--dbname", database, "--file=-"])
            .env("PGPASSFILE", "/dev/null").env("PGCONNECT_TIMEOUT", "10")
            .env("PGOPTIONS", format!("-c default_transaction_read_only=on -c statement_timeout={} -c lock_timeout=3000", config.timeout_seconds*1000))
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::from(error));
        Self::start_process(command, Duration::from_secs(config.timeout_seconds))
    }

    fn start_process(mut command: Command, budget: Duration) -> Result<Self> {
        let child = command
            .spawn()
            .map_err(|_| Error("read-only export process unavailable"))?;
        let mut session = Self {
            child: Some(child),
            input: None,
            output: None,
            workers: Vec::new(),
            deadline: Instant::now() + budget,
            fence: None,
            final_lsn: None,
            sequence_names: Vec::new(),
            sequence_states: Vec::new(),
        };
        let child = session
            .child
            .as_mut()
            .ok_or(Error("export process unavailable"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or(Error("export input unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or(Error("export output unavailable"))?;
        let (input, requests) = mpsc::sync_channel::<String>(1);
        let (responses, output) = mpsc::sync_channel(1);
        session.input = Some(input);
        session.output = Some(output);
        let writer = thread::Builder::new()
            .name("uuid-export-sql".into())
            .spawn(move || {
                let mut stdin = stdin;
                for script in requests {
                    stdin
                        .write_all(script.as_bytes())
                        .and_then(|_| stdin.flush())
                        .map_err(|_| Error("export SQL stream write failed"))?;
                }
                Ok(())
            })
            .map_err(|_| Error("export input worker unavailable"))?;
        session.workers.push(writer);
        let reader = thread::Builder::new()
            .name("uuid-export-records".into())
            .spawn(move || {
                let mut stdout = BufReader::new(stdout);
                loop {
                    match bounded_line(&mut stdout) {
                        Ok(Some(line)) => {
                            if responses.send(Ok(line)).is_err() {
                                return Ok(());
                            }
                        }
                        Ok(None) => return Ok(()),
                        Err(error) => {
                            let _ = responses.send(Err(error));
                            return Err(Error("export stream reader failed"));
                        }
                    }
                }
            })
            .map_err(|_| Error("export output worker unavailable"))?;
        session.workers.push(reader);
        Ok(session)
    }

    fn send(&mut self, script: String, final_script: bool) -> Result<()> {
        self.input
            .as_ref()
            .ok_or(Error("export SQL stream already closed"))?
            .send(script)
            .map_err(|_| Error("export SQL stream closed early"))?;
        if final_script {
            self.input.take();
        }
        Ok(())
    }

    fn line(&mut self) -> Result<Option<Vec<u8>>> {
        loop {
            let remaining = self
                .deadline
                .checked_duration_since(Instant::now())
                .ok_or(Error("export operation deadline exceeded"))?;
            match self
                .output
                .as_ref()
                .ok_or(Error("export output closed"))?
                .recv_timeout(remaining.min(Duration::from_millis(100)))
            {
                Ok(line) => return line.map(Some),
                Err(RecvTimeoutError::Disconnected) => return Ok(None),
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    fn event(&mut self) -> Result<Event> {
        parse_event(
            &self
                .line()?
                .ok_or(Error("export stream ended before completion"))?,
        )
    }

    fn finish(&mut self) -> Result<()> {
        if self.line()?.is_some() {
            return Err(Error("extra export stream record after completion"));
        }
        loop {
            let child = self.child.as_mut().ok_or(Error("export process missing"))?;
            if let Some(status) = child
                .try_wait()
                .map_err(|_| Error("export process status failed"))?
            {
                if !status.success() {
                    return Err(Error("read-only export failed; native details redacted"));
                }
                break;
            }
            if Instant::now() >= self.deadline {
                return Err(Error("export operation deadline exceeded"));
            }
            thread::sleep(Duration::from_millis(10));
        }
        for worker in self.workers.drain(..) {
            worker
                .join()
                .map_err(|_| Error("export stream worker failed"))??;
        }
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.input.take();
        self.output.take();
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn write_row<W: Write>(
    writer: &mut W,
    plan: &TablePlan,
    row: Value,
    created_at_ms: Option<i64>,
) -> Result<()> {
    let object = row
        .as_object()
        .ok_or(Error("export row must be an object"))?;
    if object.keys().cloned().collect::<BTreeSet<_>>() != plan.columns
        || plan
            .primary_key
            .iter()
            .any(|column| object.get(column).is_none_or(Value::is_null))
        || plan.time_column.is_none() && created_at_ms.is_some()
    {
        return Err(Error(
            "export row has unregistered columns or invalid key/time metadata",
        ));
    }
    serde_json::to_writer(&mut *writer, &RowEnvelope { row, created_at_ms })
        .map_err(|_| Error("export row serialization failed"))?;
    writer
        .write_all(b"\n")
        .map_err(|_| Error("export row write failed"))
}

fn export_database(
    config: &ExportConfig,
    db: &DatabaseConfig,
    registry: &Value,
    scratch: &Scratch,
    batch_id: Uuid,
    key: &[u8],
) -> Result<DatabaseSnapshot> {
    let mut session = Session::start(config, scratch, &db.name)?;
    session.send(catalog_script(config)?, false)?;
    let fence = validate_fence(session.event()?, &db.name, true)?;
    session.fence = Some(fence.clone());
    let (snapshot, lsn) = (fence.snapshot, fence.lsn);
    let mut catalog: Value = strict_json(&session.line()?.ok_or(Error("source catalog missing"))?)?;
    crate::uuid_inventory::validate_catalog(&db.kind, &db.name, &catalog, registry)?;
    let schema_bytes = serde_json::to_vec(&catalog["schema_material"])
        .map_err(|_| Error("export schema serialization failed"))?;
    let schema_hash = hash(&schema_bytes);
    if schema_hash != db.expected_schema_sha256 {
        return Err(Error(
            "source schema differs from reviewed inventory baseline",
        ));
    }
    catalog["schema_fingerprint_sha256"] = json!(schema_hash);
    let plans = plans(&catalog)?;
    session.sequence_names = sequence_names(&catalog)?;
    let tables = capture_tables(&mut session, config, db, plans, scratch, batch_id, key)?;
    Ok(DatabaseSnapshot {
        kind: db.kind.clone(),
        name: db.name.clone(),
        catalog,
        snapshot,
        lsn,
        final_lsn: session
            .final_lsn
            .clone()
            .ok_or(Error("final source WAL observation missing"))?,
        sequences: session.sequence_states.clone(),
        tables,
    })
}

fn capture_tables(
    session: &mut Session,
    config: &ExportConfig,
    db: &DatabaseConfig,
    table_plans: Vec<TablePlan>,
    scratch: &Scratch,
    batch_id: Uuid,
    key: &[u8],
) -> Result<Vec<TableSnapshot>> {
    let script = table_script(&table_plans, config, &session.sequence_names)?;
    session.send(script, true)?;
    let before_sequences = read_sequences(session, "before")?;
    let mut tables = Vec::new();
    for plan in table_plans {
        match session.event()? {
            Event::TableBegin { schema, table } if schema == plan.schema && table == plan.table => {
            }
            _ => return Err(Error("export table stream order differs from catalog")),
        }
        let plain = scratch
            .0
            .join(format!("{}.{}.{}.ndjson", db.kind, plan.schema, plan.table));
        let file = new_plaintext(&plain)?;
        let mut writer = BufWriter::new(file);
        let mut rows = 0_u64;
        loop {
            match session.event()? {
                Event::Row { row, created_at_ms } => {
                    write_row(&mut writer, &plan, row, created_at_ms)?;
                    rows = rows
                        .checked_add(1)
                        .ok_or(Error("export row count overflow"))?;
                    if rows > plan.expected_rows {
                        return Err(Error("export rows exceed catalog snapshot count"));
                    }
                }
                Event::TableEnd {
                    schema,
                    table,
                    rows: server_rows,
                } if schema == plan.schema
                    && table == plan.table
                    && rows == server_rows
                    && rows == plan.expected_rows =>
                {
                    break
                }
                _ => {
                    return Err(Error(
                        "export table boundaries or counts differ from catalog",
                    ))
                }
            }
        }
        io(writer.flush())?;
        io(writer.get_ref().sync_all())?;
        drop(writer);
        let filename = format!("{}.{}.{}.ndjson.enc", db.kind, plan.schema, plan.table);
        let purpose = format!(
            "saas-uuid-source-table/1/{batch_id}/{}/{}/{}",
            db.kind, plan.schema, plan.table
        );
        let digest =
            super::codec::seal_file(&plain, &config.output_dir.join(&filename), key, &purpose)?;
        io(fs::remove_file(&plain))?;
        tables.push(TableSnapshot {
            schema: plan.schema,
            table: plan.table,
            rows,
            file: filename,
            purpose,
            digest,
        });
    }
    let final_catalog: Value = strict_json(
        &session
            .line()?
            .ok_or(Error("final source catalog missing"))?,
    )?;
    let material = serde_json::to_vec(&final_catalog["schema_material"])
        .map_err(|_| Error("final schema serialization failed"))?;
    if hash(&material) != db.expected_schema_sha256 {
        return Err(Error(
            "final source schema changed; fresh frozen export required",
        ));
    }
    let final_plans = plans(&final_catalog)?;
    if final_plans.len() != tables.len()
        || final_plans.iter().zip(&tables).any(|(plan, table)| {
            plan.schema != table.schema
                || plan.table != table.table
                || plan.expected_rows != table.rows
        })
    {
        return Err(Error(
            "final source table counts changed; fresh frozen export required",
        ));
    }
    if sequence_names(&final_catalog)? != session.sequence_names {
        return Err(Error(
            "source sequence coverage changed; fresh frozen export required",
        ));
    }
    let after_sequences = read_sequences(session, "after")?;
    if before_sequences != after_sequences {
        return Err(Error(
            "source sequence state changed; fresh frozen export required",
        ));
    }
    session.sequence_states = before_sequences;
    let final_fence = validate_fence(session.event()?, &db.name, false)?;
    let initial = session
        .fence
        .as_ref()
        .ok_or(Error("initial source fence missing"))?;
    if final_fence.snapshot != initial.snapshot {
        return Err(Error(
            "source snapshot changed; fresh frozen export required",
        ));
    }
    // Cluster WAL also includes checkpoint and unrelated-database activity.
    // Application admission is proven by fresh maintenance-DB observations.
    session.final_lsn = Some(final_fence.lsn);
    session.finish()?;
    Ok(tables)
}

fn read_sequences(session: &mut Session, expected_phase: &str) -> Result<Vec<SequenceState>> {
    let names = session.sequence_names.clone();
    let mut states = Vec::new();
    for (expected_schema, expected_name) in names {
        match session.event()? {
            Event::Sequence {
                phase,
                schema,
                name,
                last_value,
                is_called,
            } if phase == expected_phase && schema == expected_schema && name == expected_name => {
                states.push(SequenceState {
                    schema,
                    name,
                    last_value,
                    is_called,
                })
            }
            _ => {
                return Err(Error(
                    "source sequence state differs from exact registered coverage",
                ))
            }
        }
    }
    Ok(states)
}

/// Export all source tables as encrypted evidence. No source writes, schema
/// migration, service lifecycle or real cutover operation is performed here.
pub(crate) fn execute(config_path: &Path) -> Result<String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = config_path;
        Err(Error("source export requires Linux private tmpfs"))
    }
    #[cfg(target_os = "linux")]
    {
        if std::env::var("UUID_WRITES_QUIESCED").as_deref() != Ok("true") {
            return Err(Error(
                "explicit UUID_WRITES_QUIESCED=true assertion required",
            ));
        }
        safe_path(config_path)?;
        let config: ExportConfig = strict_json(&private_file(config_path)?)?;
        config.validate()?;
        require_tmpfs_directory(&config.scratch)?;
        let key = private_file(&config.key_file)?;
        if key.len() != 32 {
            return Err(Error("export key must contain 32 random bytes"));
        }
        let registry_bytes = read_registry(&config.registry)?;
        if registry_bytes.as_slice() != include_bytes!("../../uuid-references-v2.json") {
            return Err(Error(
                "export requires the exact reviewed v2 reference registry",
            ));
        }
        let registry: Value = strict_json(&registry_bytes)?;
        let registered = registry["databases"]
            .as_array()
            .ok_or(Error("export registry databases missing"))?;
        let count = |key: &str| -> Result<usize> {
            registered
                .iter()
                .map(|db| {
                    db[key]
                        .as_array()
                        .map(Vec::len)
                        .ok_or(Error("export registry baseline missing"))
                })
                .try_fold(0_usize, |sum, n| {
                    sum.checked_add(n?).ok_or(Error("registry count overflow"))
                })
        };
        if registered.len() != 3
            || count("tables")? != 53
            || count("columns")? != 147
            || count("foreignKeys")? != 67
        {
            return Err(Error(
                "unsupported three-database reference registry baseline",
            ));
        }
        let evidence = private_file(&config.quiescence_file)?;
        let quiescence: Quiescence = strict_json(&evidence)?;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error("clock before epoch"))?
            .as_millis();
        quiescence.validate(
            &config,
            u64::try_from(now_ms).map_err(|_| Error("clock outside supported range"))?,
        )?;
        let scratch = Scratch::new(&config.scratch)?;
        create_output(&config.output_dir)?;
        let batch_id = Uuid::now_v7();
        let mut databases = Vec::new();
        let mut admission_before = Vec::new();
        let mut admission_after = Vec::new();
        for db in &config.databases {
            let before = admission::observe(&config, &scratch)?;
            validate_admission(&before)?;
            if admission_before
                .first()
                .is_some_and(|first| first != &before)
            {
                return Err(Error(
                    "source admission identity changed; fresh frozen export required",
                ));
            }
            admission_before.push(before);
            databases.push(export_database(
                &config, db, &registry, &scratch, batch_id, &key,
            )?);
            let after = admission::observe(&config, &scratch)?;
            validate_admission(&after)?;
            if admission_before.first() != Some(&after) {
                return Err(Error(
                    "source admission identity changed; fresh frozen export required",
                ));
            }
            admission_after.push(after);
        }
        let quiescence_file = "quiescence.enc".to_owned();
        let quiescence_purpose = format!("saas-uuid-source-quiescence/1/{batch_id}");
        let evidence_plain = scratch.0.join("quiescence.json");
        create_private(&evidence_plain, &evidence)?;
        let quiescence_digest = super::codec::seal_file(
            &evidence_plain,
            &config.output_dir.join(&quiescence_file),
            &key,
            &quiescence_purpose,
        )?;
        io(fs::remove_file(&evidence_plain))?;
        let snapshot = Snapshot {
            format: SNAPSHOT_FORMAT.into(),
            batch_id,
            frozen_ms: quiescence.captured_ms,
            registry_sha256: hash(&registry_bytes),
            databases,
            admission_before,
            admission_after,
            quiescence_sha256: hash(&evidence),
            quiescence_file,
            quiescence_purpose,
            quiescence_digest,
        };
        validate_quiescence(&snapshot, &evidence)?;
        let manifest = serde_json::to_vec(&snapshot)
            .map_err(|_| Error("export manifest serialization failed"))?;
        let plain = scratch.0.join("manifest.json");
        create_private(&plain, &manifest)?;
        let digest = super::codec::seal_file(
            &plain,
            &config.output_dir.join("manifest.enc"),
            &key,
            MANIFEST_PURPOSE,
        )?;
        io(fs::remove_file(plain))?;
        let tables: usize = snapshot.databases.iter().map(|db| db.tables.len()).sum();
        let rows = snapshot
            .databases
            .iter()
            .flat_map(|db| &db.tables)
            .try_fold(0_u64, |total, table| {
                total
                    .checked_add(table.rows)
                    .ok_or(Error("export total row count overflow"))
            })?;
        Ok(json!({"status":"ENCRYPTED_READ_ONLY_SOURCE_EXPORT_VERIFIED", "databases":3,
            "tables":tables,"rows":rows,"registry_sha256":snapshot.registry_sha256,
            "quiescence_sha256":snapshot.quiescence_sha256,"manifest_plaintext_sha256":digest.sha256}).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn config() -> ExportConfig {
        let base = std::env::temp_dir().join(format!("uuid-export-fixture-{}", Uuid::now_v7()));
        ExportConfig {
            profile: "uuid-local-preview".into(),
            host: "127.0.0.1".into(),
            port: 5432,
            user: "postgres".into(),
            databases: SOURCES
                .map(|(kind, name)| DatabaseConfig {
                    kind: kind.into(),
                    name: name.into(),
                    expected_schema_sha256: "a".repeat(64),
                })
                .into(),
            scratch: base.join("scratch"),
            registry: base.join("registry.json"),
            key_file: base.join("key"),
            output_dir: base.join("encrypted"),
            quiescence_file: base.join("quiescence.json"),
            timeout_seconds: 10,
        }
    }

    fn catalog() -> Value {
        json!({
            "relations":[{"schema":"public","table":"example","row_count":2}],
            "sequences":[],
            "schema_material":[
                {"kind":"column","schema":"public","table":"example","column":"id","type":"uuid"},
                {"kind":"column","schema":"public","table":"example","column":"ordinal","type":"bigint"},
                {"kind":"column","schema":"public","table":"example","column":"amount","type":"numeric"},
                {"kind":"column","schema":"public","table":"example","column":"created_at","type":"timestamp with time zone"}
            ],
            "constraints":[{"schema":"public","table":"example","type":"p","validated":true,"columns":["ordinal","id"]}]
        })
    }

    #[test]
    fn explicit_local_sources_hashes_and_private_output_are_required() {
        let mut c = config();
        assert!(c.validate().is_ok());
        c.host = "remote.example.test".into();
        assert!(c.validate().is_err());
        c.host = "localhost".into();
        assert!(c.validate().is_err());
        c.host = "::1".into();
        assert!(c.validate().is_ok());
        c.user = "other_admin".into();
        assert!(c.validate().is_err());
        c.user = "postgres".into();
        c.databases[0].expected_schema_sha256 = "not-a-hash".into();
        assert!(c.validate().is_err());
        c.databases[0].expected_schema_sha256 = "a".repeat(64);
        c.databases[0].name = c.databases[1].name.clone();
        assert!(c.validate().is_err());
        c.databases[0].name = "production".into();
        assert!(c.validate().is_err());
        c.databases[0].name = "platform_preview_fixture".into();
        assert!(c.validate().is_err());
        c.databases[0].name = SOURCES[0].1.into();
        assert!(c.validate().is_ok());
        c.key_file = c.output_dir.join("key");
        assert!(c.validate().is_err());
        let mut observing = config();
        observing.output_dir = std::env::temp_dir();
        assert!(observing.validate().is_err());
        assert!(observing.validate_connection().is_ok());
        assert!(sql_identifier("_sqlx_migrations").is_ok());
        for value in [
            "",
            "1table",
            "a;DROP TABLE users",
            "a.b",
            "a'",
            "a\"",
            "a\n",
        ] {
            assert!(sql_identifier(value).is_err());
        }
    }

    #[test]
    fn frozen_time_is_bound_to_recent_complete_quiescence_evidence() {
        let c = config();
        let mut evidence = Quiescence {
            format: QUIESCENCE_FORMAT.into(),
            profile: c.profile.clone(),
            writes_quiesced: true,
            databases: c
                .databases
                .iter()
                .map(|db| QuiescentDatabase {
                    kind: db.kind.clone(),
                    name: db.name.clone(),
                })
                .collect(),
            stopped_writers: WRITERS.map(str::to_owned).into(),
            captured_ms: 1_700_000_000_000,
            writer_evidence_sha256: "a".repeat(64),
        };
        assert!(evidence.validate(&c, 1_700_000_000_001).is_ok());
        assert!(evidence.validate(&c, 1_699_999_999_999).is_err());
        assert!(evidence.validate(&c, 1_700_003_600_001).is_err());
        evidence.stopped_writers.pop();
        assert!(evidence.validate(&c, 1_700_000_000_001).is_err());
        evidence.stopped_writers = WRITERS.map(str::to_owned).into();
        evidence.databases[0].name = "different_preview_fixture".into();
        assert!(evidence.validate(&c, 1_700_000_000_001).is_err());
    }

    #[test]
    fn export_sql_keeps_one_read_only_transaction_and_actual_composite_key_order() {
        let c = config();
        let plans = plans(&catalog()).unwrap();
        let sql = catalog_script(&c).unwrap() + &table_script(&plans, &c, &[]).unwrap();
        assert_eq!(
            sql.matches("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;")
                .count(),
            1
        );
        assert_eq!(sql.matches("COMMIT;").count(), 1);
        assert!(sql.contains("SET TRANSACTION READ ONLY;"));
        assert!(sql.contains("default_transaction_read_only=on"));
        assert!(sql.contains("pg_stat_activity"));
        assert!(!sql.contains("pg_stat_activity.query"));
        assert!(sql.contains("ORDER BY t.\"ordinal\",t.\"id\""));
        assert!(sql.contains("FROM ONLY \"public\".\"example\""));
        assert!(sql.contains("isfinite(t.\"created_at\")"));
        for forbidden in [
            "INSERT ",
            "UPDATE ",
            "DELETE ",
            "TRUNCATE ",
            "CREATE ",
            "DROP ",
            "\\gexec",
            "\\!",
        ] {
            assert!(!sql.contains(forbidden), "unexpected writable SQL");
        }
    }

    #[test]
    fn unknown_columns_invalid_primary_keys_and_untrusted_times_fail_closed() {
        let mut c = catalog();
        c["constraints"][0]["columns"] = json!(["missing"]);
        assert!(plans(&c).is_err());
        c["constraints"][0]["columns"] = json!(["id", "id"]);
        assert!(plans(&c).is_err());
        c["constraints"][0]["columns"] = json!(["id"]);
        c["schema_material"][3]["type"] = json!("timestamp without time zone");
        let plan = plans(&c).unwrap().remove(0);
        assert!(plan.time_column.is_none());
        let row = json!({"id":"fixture","ordinal":1,"amount":1,"created_at":"untrusted"});
        assert!(write_row(&mut Vec::new(), &plan, row.clone(), Some(1)).is_err());
        assert!(write_row(&mut Vec::new(), &plan, row.clone(), None).is_ok());
        let mut extra = row;
        extra["new_unknown"] = json!("unreviewed");
        assert!(write_row(&mut Vec::new(), &plan, extra, None).is_err());
    }

    #[test]
    fn row_envelopes_preserve_arbitrary_precision_values_without_raw_line_splitting() {
        let plan = plans(&catalog()).unwrap().remove(0);
        let row: Value = serde_json::from_str(r#"{"id":"fixture","ordinal":123456789012345678901234567890,"amount":12345678901234567890.12345678901234567890,"created_at":"text\nline"}"#).unwrap();
        let mut bytes = Vec::new();
        write_row(&mut bytes, &plan, row.clone(), Some(1_700_000_000_000)).unwrap();
        assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 1);
        let envelope: RowEnvelope = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(envelope.row, row);
        assert_eq!(envelope.created_at_ms, Some(1_700_000_000_000));
        assert!(String::from_utf8(bytes)
            .unwrap()
            .contains("12345678901234567890.12345678901234567890"));
        let mut bad = envelope.row;
        bad["id"] = Value::Null;
        assert!(write_row(&mut Vec::new(), &plan, bad, None).is_err());
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("fixture"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        assert!(write_row(&mut Broken, &plan, row, None).is_err());
    }

    #[test]
    fn stream_records_reject_unknown_missing_duplicate_and_unbounded_fields() {
        for bytes in [
            br#"{"kind":"unknown"}"#.as_slice(),
            br#"{"kind":"row","row":{"id":1,"id":2},"createdAtMs":null}"#.as_slice(),
            br#"{"kind":"row","row":{},"createdAtMs":null,"extra":true}"#.as_slice(),
            br#"{"kind":"row","row":{}}"#.as_slice(),
            br#"{"kind":"row","row":{},"createdAtMs":null,"createdAtMs":1}"#.as_slice(),
        ] {
            assert!(parse_event(bytes).is_err());
        }
        assert!(serde_json::from_str::<RowEnvelope>(r#"{"row":{}}"#).is_err());
        let mut reader = Cursor::new(
            b"{\"kind\":\"table_begin\",\"schema\":\"public\",\"table\":\"fixture\"}\r\n",
        );
        assert!(matches!(
            parse_event(&bounded_line(&mut reader).unwrap().unwrap()).unwrap(),
            Event::TableBegin { .. }
        ));
        assert!(bounded_line(&mut reader).unwrap().is_none());
        assert!(bounded_line(&mut Cursor::new(b"partial")).is_err());
        assert!(bounded_line(&mut Cursor::new(b"\n")).is_err());
        assert!(bounded_line(&mut Cursor::new(vec![b'x'; MAX_LINE + 1])).is_err());
    }

    #[test]
    fn waiting_stream_has_a_deadline_even_without_any_stdout() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut session = Session {
            child: None,
            input: None,
            output: Some(receiver),
            workers: Vec::new(),
            deadline: Instant::now() + Duration::from_millis(5),
            fence: None,
            final_lsn: None,
            sequence_names: Vec::new(),
            sequence_states: Vec::new(),
        };
        assert!(session.line().is_err());
        drop(sender);
    }

    #[test]
    fn sequence_definitions_require_exact_coverage_and_states_remain_signed_integers() {
        let mut source = catalog();
        let definition =
            json!({"kind":"sequence","schema":"public","name":"counter","type":"bigint"});
        source["sequences"] = json!([definition.clone()]);
        assert!(sequence_names(&source).is_err());
        source["schema_material"]
            .as_array_mut()
            .unwrap()
            .push(definition.clone());
        let names = sequence_names(&source).unwrap();
        assert!(names == vec![("public".into(), "counter".into())]);
        let sql = sequence_script(&names, "before").unwrap();
        assert!(
            sql.contains("last_value::bigint")
                && sql.contains("'is_called',is_called")
                && sql.contains("FROM \"public\".\"counter\"")
        );
        for forbidden in ["nextval", "setval", "MAX(", "CREATE ", "UPDATE "] {
            assert!(!sql.contains(forbidden));
        }
        source["sequences"].as_array_mut().unwrap().push(definition);
        assert!(sequence_names(&source).is_err());
        for last in ["1.5", "\"1\"", "9223372036854775808"] {
            let raw = format!(
                r#"{{"kind":"sequence","phase":"before","schema":"public","name":"counter","last_value":{last},"is_called":true}}"#
            );
            assert!(parse_event(raw.as_bytes()).is_err());
        }
    }

    fn fence_event(database: &str, kind: &str) -> Value {
        json!({"kind":kind,"database":database,"snapshot":"1:2:","lsn":"0/10",
            "default_read_only":"on","database_read_only":true,"other_connections":0,"prepared_transactions":0})
    }

    #[test]
    fn idle_connections_prepared_transactions_and_wrong_phase_reject_source_fences() {
        let original = fence_event("platform_preview_fixture", "snapshot");
        let parse = |value: &Value| parse_event(&serde_json::to_vec(value).unwrap()).unwrap();
        assert!(validate_fence(parse(&original), "platform_preview_fixture", true).is_ok());
        assert!(validate_fence(parse(&original), "platform_preview_fixture", false).is_err());
        for (field, value) in [
            ("other_connections", json!(1)),
            ("prepared_transactions", json!(1)),
            ("database_read_only", json!(false)),
            ("default_read_only", json!("off")),
        ] {
            let mut invalid = original.clone();
            invalid[field] = value;
            assert!(validate_fence(parse(&invalid), "platform_preview_fixture", true).is_err());
        }
        assert!(validate_fence(parse(&original), "different_preview_fixture", true).is_err());
    }

    #[test]
    fn decrypted_quiescence_body_is_bound_to_manifest_sources_time_and_cipher_purpose() {
        let c = config();
        let bytes=serde_json::to_vec(&json!({"format":QUIESCENCE_FORMAT,"profile":"uuid-local-preview",
            "writes_quiesced":true,"captured_ms":1_700_000_000_000_u64,"stopped_writers":WRITERS,
            "writer_evidence_sha256":"a".repeat(64),
            "databases":c.databases.iter().map(|db|json!({"kind":db.kind,"name":db.name})).collect::<Vec<_>>() })).unwrap();
        let batch_id = Uuid::now_v7();
        let mut snapshot = Snapshot {
            format: SNAPSHOT_FORMAT.into(),
            batch_id,
            frozen_ms: 1_700_000_000_000,
            registry_sha256: "b".repeat(64),
            admission_before: Vec::new(),
            admission_after: Vec::new(),
            quiescence_sha256: hash(&bytes),
            quiescence_file: "quiescence.enc".into(),
            quiescence_purpose: format!("saas-uuid-source-quiescence/1/{batch_id}"),
            quiescence_digest: super::super::codec::FileDigest {
                sha256: hash(&bytes),
                plaintext_bytes: bytes.len() as u64,
                frames: 1,
            },
            databases: c
                .databases
                .iter()
                .map(|db| DatabaseSnapshot {
                    kind: db.kind.clone(),
                    name: db.name.clone(),
                    catalog: Value::Null,
                    snapshot: "1:2:".into(),
                    lsn: "0/10".into(),
                    final_lsn: "0/20".into(),
                    sequences: Vec::new(),
                    tables: Vec::new(),
                })
                .collect(),
        };
        assert!(validate_quiescence(&snapshot, &bytes).is_ok());
        snapshot.frozen_ms += 1;
        assert!(validate_quiescence(&snapshot, &bytes).is_err());
        snapshot.frozen_ms -= 1;
        snapshot.quiescence_purpose.push('x');
        assert!(validate_quiescence(&snapshot, &bytes).is_err());
        assert!(strict_json::<Value>(br#"{"outer":{"id":1,"id":2}}"#).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_fixture_stream_seals_each_table_and_checks_final_catalog_without_a_database() {
        for (after_value, final_snapshot, final_lsn, failure) in [
            (999_999, "1:2:", "0/10", None),
            (999_999, "1:2:", "0/20", None),
            (
                1_000_000,
                "1:2:",
                "0/20",
                Some("source sequence state changed; fresh frozen export required"),
            ),
            (
                999_999,
                "2:3:",
                "0/20",
                Some("source snapshot changed; fresh frozen export required"),
            ),
        ] {
            let scratch = Scratch::new(Path::new("/dev/shm")).unwrap();
            let mut c = config();
            c.output_dir = scratch.0.join("encrypted");
            create_output(&c.output_dir).unwrap();
            let mut source = catalog();
            let definition = json!({"kind":"sequence","schema":"public","name":"opaque_business_counter",
            "type":"bigint","start":1,"increment":1,"min":1,"max":i64::MAX,"cache":1,"cycle":false});
            source["sequences"] = json!([definition.clone()]);
            source["schema_material"]
                .as_array_mut()
                .unwrap()
                .push(definition);
            c.databases[0].expected_schema_sha256 =
                hash(&serde_json::to_vec(&source["schema_material"]).unwrap());
            let db = &c.databases[0];
            let mut records = vec![
                json!({"kind":"sequence","phase":"before","schema":"public",
            "name":"opaque_business_counter","last_value":999_999,"is_called":false}),
                json!({"kind":"table_begin","schema":"public","table":"example"}),
            ];
            for index in 1..=2 {
                records.push(json!({"kind":"row","createdAtMs":1_700_000_000_000_u64,
                "row":{"id":Uuid::now_v7(),"ordinal":index,"amount":index,"created_at":"2023-11-14T22:13:20Z"}}));
            }
            records.push(json!({"kind":"table_end","schema":"public","table":"example","rows":2}));
            records.push(source.clone());
            records.push(json!({"kind":"sequence","phase":"after","schema":"public",
            "name":"opaque_business_counter","last_value":after_value,"is_called":false}));
            let mut done = fence_event(&db.name, "done");
            done["snapshot"] = json!(final_snapshot);
            done["lsn"] = json!(final_lsn);
            records.push(done);
            let events = records
                .iter()
                .map(|record| format!("{record}\n"))
                .collect::<String>();
            let mut command = Command::new("sh");
            command
                .args([
                    "-c",
                    "cat >/dev/null; printf '%s' \"$1\"",
                    "fixture",
                    &events,
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            let mut session = Session::start_process(command, Duration::from_secs(5)).unwrap();
            session.fence = Some(SourceFence {
                snapshot: "1:2:".into(),
                lsn: "0/10".into(),
            });
            session.sequence_names = sequence_names(&source).unwrap();
            let key = [0x35; 32];
            let result = capture_tables(
                &mut session,
                &c,
                db,
                plans(&source).unwrap(),
                &scratch,
                Uuid::now_v7(),
                &key,
            );
            if let Some(expected) = failure {
                assert_eq!(result.err().unwrap().0, expected);
                continue;
            }
            let tables = result.unwrap();
            assert_eq!(session.final_lsn.as_deref(), Some(final_lsn));
            assert!(
                session.sequence_states.len() == 1
                    && session.sequence_states[0].last_value == 999_999
                    && !session.sequence_states[0].is_called,
                "sequence high-water/is_called was replaced by table maxima"
            );
            assert_eq!(tables.len(), 1);
            assert_eq!(tables[0].rows, 2);
            assert!(!scratch.0.join("platform.public.example.ndjson").exists());
            let opened = scratch.0.join("opened.ndjson");
            let digest = super::super::codec::open_file(
                &c.output_dir.join(&tables[0].file),
                &opened,
                &key,
                &tables[0].purpose,
            )
            .unwrap();
            assert_eq!(digest, tables[0].digest);
            let rows = fs::read_to_string(opened).unwrap();
            assert_eq!(rows.lines().count(), 2);
            assert!(rows
                .lines()
                .all(|line| strict_json::<RowEnvelope>(line.as_bytes()).is_ok()));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_exit_and_timeout_are_checked_without_any_database() {
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "printf '{\"kind\":\"table_begin\",\"schema\":\"public\",\"table\":\"fixture\"}\\n'; exit 7",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut session = Session::start_process(command, Duration::from_secs(2)).unwrap();
        session.input.take();
        assert!(matches!(session.event().unwrap(), Event::TableBegin { .. }));
        assert!(session.finish().is_err());
        drop(session);
        let mut command = Command::new("sleep");
        command
            .arg("3")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let start = Instant::now();
        let mut session = Session::start_process(command, Duration::from_millis(10)).unwrap();
        assert!(session.line().is_err());
        drop(session);
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "deadline cleanup retained a running process"
        );
    }
}
