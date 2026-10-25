//! Offline conversion of authenticated source evidence. Completion is published
//! only after every table and its row proofs have been authenticated and sealed.
#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports, unused_mut))]
use super::{codec, export, mapping::TypedMap, rows, structured};
use crate::{
    config::private_file,
    io,
    native::{create_private, Scratch},
    Error, Result,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Read, Write},
    path::{Component, Path, PathBuf},
};
use uuid::{Uuid, Variant};

pub(crate) const FORMAT: &str = "saas-uuid-transformed-snapshot/1";
pub(crate) const MANIFEST_PURPOSE: &str = "saas-uuid-transformed-manifest/1";
const MAX_LINE: u64 = 16 * 1024 * 1024;
const MAX_MANIFEST: u64 = 64 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    profile: String,
    source_dir: PathBuf,
    output_dir: PathBuf,
    scratch: PathBuf,
    registry: PathBuf,
    key_file: PathBuf,
    resume_mapping: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Converted {
    pub(crate) format: String,
    pub(crate) batch_id: Uuid,
    pub(crate) frozen_ms: u64,
    pub(crate) registry_sha256: String,
    pub(crate) source_manifest_sha256: String,
    pub(crate) mapping_file: String,
    pub(crate) mapping_purpose: String,
    pub(crate) mapping_digest: codec::FileDigest,
    pub(crate) source: export::Snapshot,
    pub(crate) databases: Vec<ConvertedDatabase>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConvertedDatabase {
    pub(crate) kind: String,
    pub(crate) tables: Vec<ConvertedTable>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConvertedTable {
    pub(crate) schema: String,
    pub(crate) table: String,
    pub(crate) policy: String,
    pub(crate) source_rows: u64,
    pub(crate) rows: u64,
    pub(crate) file: String,
    pub(crate) purpose: String,
    pub(crate) digest: codec::FileDigest,
    pub(crate) proof_file: String,
    pub(crate) proof_purpose: String,
    pub(crate) proof_digest: codec::FileDigest,
}

fn safe_path(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(Error("absolute normalized migration path required"));
    }
    for parent in path.ancestors() {
        match fs::symlink_metadata(parent) {
            Ok(m) if m.file_type().is_symlink() => return Err(Error("migration symlink refused")),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Error("migration path inaccessible")),
        }
    }
    Ok(())
}

fn private_directory(path: &Path) -> Result<()> {
    safe_path(path)?;
    let m = io(fs::symlink_metadata(path))?;
    if !m.is_dir() {
        return Err(Error("private migration directory required"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if m.mode() & 0o777 != 0o700 || m.uid() != io(fs::metadata("/proc/self"))?.uid() {
            return Err(Error("migration directory must be owned mode 0700"));
        }
    }
    Ok(())
}

fn bounded_file(path: &Path, limit: u64) -> Result<Vec<u8>> {
    safe_path(path)?;
    let m = io(fs::symlink_metadata(path))?;
    if !m.is_file() || m.len() > limit {
        return Err(Error("bounded migration file required"));
    }
    let mut bytes = Vec::new();
    io(File::open(path))?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error("migration read failed"))?;
    if bytes.len() as u64 > limit {
        return Err(Error("migration file limit exceeded"));
    }
    Ok(bytes)
}

fn create_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    io(options.open(path))
}

fn create_directory(path: &Path) -> Result<()> {
    safe_path(path)?;
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    io(builder.create(path))
}

fn hash(bytes: &[u8]) -> String {
    crate::bundle::hash(bytes)
}

fn registry(path: &Path) -> Result<(Value, String)> {
    let bytes = bounded_file(path, 4 * 1024 * 1024)?;
    if bytes != include_bytes!("../../uuid-references-v2.json") {
        return Err(Error("reviewed v2 registry bytes required"));
    }
    let value = structured::parse_strict(
        std::str::from_utf8(&bytes).map_err(|_| Error("registry UTF-8 required"))?,
    )?;
    Ok((value, hash(&bytes)))
}

pub(crate) fn authorities(registry: &Value) -> Result<BTreeSet<String>> {
    let mut result = BTreeSet::new();
    for key in ["entities", "nonRowAuthorities"] {
        for item in registry[key]
            .as_array()
            .ok_or(Error("mapping authorities missing"))?
        {
            let name = item["authority"]
                .as_str()
                .ok_or(Error("mapping authority missing"))?;
            if !result.insert(name.to_owned()) {
                return Err(Error("duplicate mapping authority"));
            }
        }
    }
    Ok(result)
}

pub(crate) fn policy<'a>(
    registry: &'a Value,
    kind: &str,
    schema: &str,
    table: &str,
) -> Result<&'a str> {
    let db = registry["databases"]
        .as_array()
        .ok_or(Error("database registry missing"))?
        .iter()
        .find(|d| d["kind"] == kind)
        .ok_or(Error("unknown database authority"))?;
    db["tables"]
        .as_array()
        .ok_or(Error("table registry missing"))?
        .iter()
        .find(|t| t["schema"] == schema && t["table"] == table)
        .and_then(|t| t["retention"].as_str())
        .ok_or(Error("unknown table retention policy"))
}

pub(crate) fn validate_source(
    source: &export::Snapshot,
    registry: &Value,
    registry_hash: &str,
) -> Result<()> {
    let kinds: BTreeSet<_> = source.databases.iter().map(|d| d.kind.as_str()).collect();
    if source.format != export::SNAPSHOT_FORMAT
        || source.registry_sha256 != registry_hash
        || source.batch_id.get_version_num() != 7
        || source.batch_id.get_variant() != Variant::RFC4122
        || source.frozen_ms == 0
        || source.frozen_ms >= 1_u64 << 48
        || source.databases.len() != 3
        || kinds != BTreeSet::from(["platform", "stores", "work_orders"])
    {
        return Err(Error("source snapshot identity or registry mismatch"));
    }
    if source.admission_before.len() != 3 || source.admission_after.len() != 3 {
        return Err(Error("complete fresh source admission proofs required"));
    }
    let admission = &source.admission_before[0];
    for proof in source
        .admission_before
        .iter()
        .chain(&source.admission_after)
    {
        export::validate_admission(proof)?;
        if proof != admission {
            return Err(Error("source admission identities changed during export"));
        }
    }
    let mut files = BTreeSet::new();
    for db in &source.databases {
        if db.snapshot.is_empty() || parse_lsn(&db.final_lsn)? < parse_lsn(&db.lsn)? {
            return Err(Error("invalid source snapshot or WAL evidence"));
        }
        let expected_name = match db.kind.as_str() {
            "platform" => "qiankun_platform_preview_20261024",
            "stores" => "qiankun_stores_preview_20261024",
            "work_orders" => "qiankun_orders_preview_20261024",
            _ => return Err(Error("unregistered preview source")),
        };
        if db.name != expected_name {
            return Err(Error("unregistered preview source"));
        }
        let expected_schema = match db.kind.as_str() {
            "platform" => "2a16ccb1b285bb8613a56b33a84df02032ad8a190c2e5b20697ab35fd4b084c2",
            "stores" => "cdb1c3a42077033d8692afe8b09b8dc192c41a0491d327cc4fe686624843ba08",
            "work_orders" => "ce5768293dd291e16320641a31f8bef97e127acf83b21416fd5718d5a11be96c",
            _ => return Err(Error("unregistered preview source")),
        };
        let schema_material = serde_json::to_vec(&db.catalog["schema_material"])
            .map_err(|_| Error("source schema serialization failed"))?;
        if hash(&schema_material) != expected_schema {
            return Err(Error("source schema differs from reviewed U01 catalog"));
        }
        crate::uuid_inventory::validate_catalog(&db.kind, &db.name, &db.catalog, registry)?;
        let expected_sequences = export::sequence_names(&db.catalog)?;
        let actual_sequences: Vec<_> = db
            .sequences
            .iter()
            .map(|s| (s.schema.clone(), s.name.clone()))
            .collect();
        if actual_sequences != expected_sequences {
            return Err(Error("source sequence state coverage mismatch"));
        }
        for sequence in &db.sequences {
            let definition = db.catalog["sequences"]
                .as_array()
                .ok_or(Error("source sequence definitions missing"))?
                .iter()
                .find(|s| s["schema"] == sequence.schema && s["name"] == sequence.name)
                .ok_or(Error("source sequence definition missing"))?;
            let minimum = definition["min"]
                .as_i64()
                .ok_or(Error("source sequence minimum invalid"))?;
            let maximum = definition["max"]
                .as_i64()
                .ok_or(Error("source sequence maximum invalid"))?;
            if sequence.last_value < minimum || sequence.last_value > maximum {
                return Err(Error("source sequence state outside reviewed range"));
            }
        }
        let expected: BTreeSet<_> = db.catalog["relations"]
            .as_array()
            .ok_or(Error("source relations missing"))?
            .iter()
            .map(|t| {
                Ok((
                    t["schema"].as_str().ok_or(Error("source schema missing"))?,
                    t["table"].as_str().ok_or(Error("source table missing"))?,
                ))
            })
            .collect::<Result<_>>()?;
        let actual: BTreeSet<_> = db
            .tables
            .iter()
            .map(|t| (t.schema.as_str(), t.table.as_str()))
            .collect();
        if actual != expected || actual.len() != db.tables.len() {
            return Err(Error("source table coverage mismatch"));
        }
        for table in &db.tables {
            let expected_file = format!("{}.{}.{}.ndjson.enc", db.kind, table.schema, table.table);
            if table.file != expected_file
                || !files.insert(&table.file)
                || table.purpose
                    != format!(
                        "saas-uuid-source-table/1/{}/{}/{}/{}",
                        source.batch_id, db.kind, table.schema, table.table
                    )
            {
                return Err(Error("source file identity mismatch"));
            }
            let count = db.catalog["relations"]
                .as_array()
                .ok_or(Error("source relations missing"))?
                .iter()
                .find(|r| r["schema"] == table.schema && r["table"] == table.table)
                .and_then(|r| r["row_count"].as_u64())
                .ok_or(Error("source row count missing"))?;
            if count != table.rows {
                return Err(Error("source table count mismatch"));
            }
            policy(registry, &db.kind, &table.schema, &table.table)?;
        }
    }
    Ok(())
}

fn parse_lsn(value: &str) -> Result<u64> {
    let (high, low) = value
        .split_once('/')
        .ok_or(Error("invalid source WAL position"))?;
    let part = |text: &str| {
        if text.is_empty() || text.len() > 8 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error("invalid source WAL position"));
        }
        u32::from_str_radix(text, 16)
            .map(u64::from)
            .map_err(|_| Error("invalid source WAL position"))
    };
    Ok((part(high)? << 32) | part(low)?)
}

fn visit_rows(
    path: &Path,
    expected_rows: u64,
    mut visit: impl FnMut(u64, export::RowEnvelope) -> Result<()>,
) -> Result<()> {
    let mut input = BufReader::new(io(File::open(path))?);
    let mut index = 0_u64;
    loop {
        let mut line = Vec::new();
        let length = io(Read::by_ref(&mut input)
            .take(MAX_LINE + 1)
            .read_until(b'\n', &mut line))?;
        if length == 0 {
            break;
        }
        if length as u64 > MAX_LINE || line.last() != Some(&b'\n') || index >= expected_rows {
            return Err(Error("source row framing or count invalid"));
        }
        let raw = std::str::from_utf8(&line).map_err(|_| Error("source row UTF-8 required"))?;
        structured::parse_strict(raw)?;
        let envelope: export::RowEnvelope =
            serde_json::from_str(raw).map_err(|_| Error("source row envelope invalid"))?;
        if !envelope.row.is_object() {
            return Err(Error("source row object required"));
        }
        visit(index, envelope)?;
        index = index
            .checked_add(1)
            .ok_or(Error("source row count overflow"))?;
    }
    if index != expected_rows {
        return Err(Error("source table count mismatch"));
    }
    Ok(())
}

fn with_table(
    source_dir: &Path,
    scratch: &Scratch,
    key: &[u8],
    table: &export::TableSnapshot,
    visit: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    let plain = scratch.0.join("source-table.ndjson");
    let digest = codec::open_file(&source_dir.join(&table.file), &plain, key, &table.purpose)?;
    if digest != table.digest {
        return Err(Error("source table authenticated digest mismatch"));
    }
    visit(&plain)?;
    io(fs::remove_file(plain))
}

fn append(writer: &mut BufWriter<File>, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *writer, value)
        .map_err(|_| Error("converted row serialization failed"))?;
    io(writer.write_all(b"\n"))
}

fn finish(mut writer: BufWriter<File>) -> Result<()> {
    io(writer.flush())?;
    io(writer.get_ref().sync_all())
}

fn seal_bytes(
    scratch: &Scratch,
    name: &str,
    output: &Path,
    key: &[u8],
    purpose: &str,
    bytes: &[u8],
) -> Result<codec::FileDigest> {
    let path = scratch.0.join(name);
    create_private(&path, bytes)?;
    let digest = codec::seal_file(&path, output, key, purpose)?;
    io(fs::remove_file(path))?;
    Ok(digest)
}

pub(crate) fn execute(config_path: &Path) -> Result<String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = config_path;
        Err(Error("UUID conversion requires Linux private tmpfs"))
    }
    #[cfg(target_os = "linux")]
    {
        safe_path(config_path)?;
        let config: Config = serde_json::from_slice(&private_file(config_path)?)
            .map_err(|_| Error("invalid UUID conversion configuration"))?;
        if config.profile != "uuid-local-preview" {
            return Err(Error("local preview conversion profile required"));
        }
        private_directory(&config.source_dir)?;
        for p in [
            &config.output_dir,
            &config.scratch,
            &config.registry,
            &config.key_file,
        ] {
            safe_path(p)?;
        }
        if config.output_dir.starts_with(&config.source_dir)
            || config.source_dir.starts_with(&config.output_dir)
            || config.key_file.starts_with(&config.output_dir)
            || config.scratch.starts_with(&config.output_dir)
        {
            return Err(Error(
                "separate source, key, scratch and new output required",
            ));
        }
        let key = private_file(&config.key_file)?;
        if key.len() != 32 {
            return Err(Error("conversion key must contain 32 random bytes"));
        }
        let (registry, registry_hash) = registry(&config.registry)?;
        let scratch = Scratch::new(&config.scratch)?;
        let source_plain = scratch.0.join("source-manifest.json");
        let source_digest = codec::open_file(
            &config.source_dir.join("manifest.enc"),
            &source_plain,
            &key,
            export::MANIFEST_PURPOSE,
        )?;
        let source_bytes = bounded_file(&source_plain, MAX_MANIFEST)?;
        structured::parse_strict(
            std::str::from_utf8(&source_bytes)
                .map_err(|_| Error("source manifest UTF-8 required"))?,
        )?;
        let source: export::Snapshot =
            serde_json::from_slice(&source_bytes).map_err(|_| Error("invalid source manifest"))?;
        io(fs::remove_file(source_plain))?;
        validate_source(&source, &registry, &registry_hash)?;
        if source.quiescence_file != "quiescence.enc"
            || source.quiescence_purpose
                != format!("saas-uuid-source-quiescence/1/{}", source.batch_id)
        {
            return Err(Error("source quiescence file identity mismatch"));
        }
        let quiescence_plain = scratch.0.join("quiescence.json");
        let quiescence_digest = codec::open_file(
            &config.source_dir.join(&source.quiescence_file),
            &quiescence_plain,
            &key,
            &source.quiescence_purpose,
        )?;
        if quiescence_digest != source.quiescence_digest {
            return Err(Error("source quiescence authenticated digest mismatch"));
        }
        let quiescence_bytes = bounded_file(&quiescence_plain, 1024 * 1024)?;
        export::validate_quiescence(&source, &quiescence_bytes)?;
        io(fs::remove_file(quiescence_plain))?;
        let mapping_purpose = format!("{}/typed-map", source.batch_id);
        let resume_bytes = if let Some(path) = &config.resume_mapping {
            safe_path(path)?;
            let plain = scratch.0.join("resume-map.json");
            codec::open_file(path, &plain, &key, &mapping_purpose)?;
            let bytes = bounded_file(&plain, MAX_MANIFEST)?;
            io(fs::remove_file(plain))?;
            Some(bytes)
        } else {
            None
        };
        let mut map = TypedMap::new(source.frozen_ms, authorities(&registry)?)?;
        // No reference is allowed to allocate a live source entity. Complete the
        // authoritative primary-key pass across all three databases first.
        for db in &source.databases {
            for table in &db.tables {
                with_table(&config.source_dir, &scratch, &key, table, |plain| {
                    visit_rows(plain, table.rows, |_, row| {
                        rows::allocate_row(
                            &db.kind,
                            &table.table,
                            &row.row,
                            row.created_at_ms,
                            &mut map,
                            &registry,
                        )
                    })
                })?;
            }
        }
        let mut collector = structured::SourceCollector::new();
        for db in &source.databases {
            for table in db
                .tables
                .iter()
                .filter(|t| structured::needs_source_collection(&db.kind, &t.table))
            {
                with_table(&config.source_dir, &scratch, &key, table, |plain| {
                    visit_rows(plain, table.rows, |_, row| {
                        let evidence = hash(
                            &serde_json::to_vec(&row.row)
                                .map_err(|_| Error("source proof serialization failed"))?,
                        );
                        collector.observe(&db.kind, &table.table, &row.row, &evidence)
                    })
                })?;
            }
        }
        collector.finish(&mut map, &registry)?;
        if let Some(bytes) = &resume_bytes {
            let resumed = TypedMap::from_bytes(bytes, source.frozen_ms, authorities(&registry)?)?;
            resumed.same_source_coverage(&map)?;
            map = resumed;
        }
        let map_bytes = map.to_bytes()?;
        create_directory(&config.output_dir)?;
        let mapping_digest = seal_bytes(
            &scratch,
            "typed-map.json",
            &config.output_dir.join("typed-map.enc"),
            &key,
            &mapping_purpose,
            &map_bytes,
        )?;
        let mut databases = Vec::new();
        let mut source_rows = 0_u64;
        let mut target_rows = 0_u64;
        for db in &source.databases {
            let mut tables = Vec::new();
            for table in &db.tables {
                let retention = policy(&registry, &db.kind, &table.schema, &table.table)?;
                if !matches!(
                    retention,
                    "RETAIN_REMAP"
                        | "HISTORY_TYPED_PROJECTION"
                        | "RETAIN_IDEMPOTENCY"
                        | "TASK_RECONCILE"
                        | "CREDENTIAL_EXCLUDED"
                        | "REBUILD_METADATA"
                ) {
                    return Err(Error("unimplemented table retention policy"));
                }
                let excluded = matches!(retention, "CREDENTIAL_EXCLUDED" | "REBUILD_METADATA");
                let target_plain = scratch.0.join("converted.ndjson");
                let proof_plain = scratch.0.join("proof.ndjson");
                let mut output = BufWriter::new(create_file(&target_plain)?);
                let mut proof = BufWriter::new(create_file(&proof_plain)?);
                with_table(&config.source_dir, &scratch, &key, table, |plain| {
                    visit_rows(plain, table.rows, |index, envelope| {
                        let source_hash = hash(
                            &serde_json::to_vec(&envelope.row)
                                .map_err(|_| Error("source proof serialization failed"))?,
                        );
                        let mut business_proof = Value::Null;
                        let target_hash = if excluded {
                            Value::Null
                        } else {
                            let mut target = envelope.row.clone();
                            rows::rewrite_row(
                                &db.kind,
                                &table.table,
                                &mut target,
                                &map,
                                &registry,
                            )?;
                            rows::verify_rewrite(
                                &db.kind,
                                &table.table,
                                &envelope.row,
                                &target,
                                &map,
                                &registry,
                            )?;
                            structured::transform_row(
                                &db.kind,
                                &table.table,
                                &mut target,
                                &map,
                                &registry,
                            )?;
                            business_proof =
                                serde_json::to_value(super::reconcile::projection_proof(
                                    &db.kind,
                                    &table.table,
                                    &envelope.row,
                                    &target,
                                    &map,
                                    &registry,
                                )?)
                                .map_err(|_| Error("projection proof serialization failed"))?;
                            let digest = hash(
                                &serde_json::to_vec(&target)
                                    .map_err(|_| Error("target proof serialization failed"))?,
                            );
                            append(&mut output, &target)?;
                            Value::String(digest)
                        };
                        append(
                            &mut proof,
                            &json!({"index":index,"policy":retention,"source_sha256":source_hash,"target_sha256":target_hash,"business_proof":business_proof}),
                        )
                    })
                })?;
                finish(output)?;
                finish(proof)?;
                let file = format!("{}.{}.{}.ndjson.enc", db.kind, table.schema, table.table);
                let purpose = format!(
                    "{}/converted/{}/{}/{}",
                    source.batch_id, db.kind, table.schema, table.table
                );
                let proof_file = format!("{}.{}.{}.proof.enc", db.kind, table.schema, table.table);
                let proof_purpose = format!(
                    "{}/proof/{}/{}/{}",
                    source.batch_id, db.kind, table.schema, table.table
                );
                let digest = codec::seal_file(
                    &target_plain,
                    &config.output_dir.join(&file),
                    &key,
                    &purpose,
                )?;
                let proof_digest = codec::seal_file(
                    &proof_plain,
                    &config.output_dir.join(&proof_file),
                    &key,
                    &proof_purpose,
                )?;
                io(fs::remove_file(target_plain))?;
                io(fs::remove_file(proof_plain))?;
                let count = if excluded { 0 } else { table.rows };
                source_rows = source_rows
                    .checked_add(table.rows)
                    .ok_or(Error("source count overflow"))?;
                target_rows = target_rows
                    .checked_add(count)
                    .ok_or(Error("target count overflow"))?;
                tables.push(ConvertedTable {
                    schema: table.schema.clone(),
                    table: table.table.clone(),
                    policy: retention.into(),
                    source_rows: table.rows,
                    rows: count,
                    file,
                    purpose,
                    digest,
                    proof_file,
                    proof_purpose,
                    proof_digest,
                });
            }
            databases.push(ConvertedDatabase {
                kind: db.kind.clone(),
                tables,
            });
        }
        let converted = Converted {
            format: FORMAT.into(),
            batch_id: source.batch_id,
            frozen_ms: source.frozen_ms,
            registry_sha256: registry_hash.clone(),
            source_manifest_sha256: source_digest.sha256,
            mapping_file: "typed-map.enc".into(),
            mapping_purpose,
            mapping_digest: mapping_digest.clone(),
            source,
            databases,
        };
        let bytes = serde_json::to_vec(&converted)
            .map_err(|_| Error("converted manifest serialization failed"))?;
        let manifest_digest = seal_bytes(
            &scratch,
            "converted-manifest.json",
            &config.output_dir.join("manifest.enc"),
            &key,
            MANIFEST_PURPOSE,
            &bytes,
        )?;
        let table_counts: Vec<_> = converted.databases.iter().flat_map(|db| db.tables.iter().map(move |table| {
            json!({"kind":db.kind,"schema":table.schema,"table":table.table,"policy":table.policy,
                "source_rows":table.source_rows,"target_rows":table.rows,
                "target_plaintext_sha256":table.digest.sha256,"proof_plaintext_sha256":table.proof_digest.sha256})
        })).collect();
        Ok(json!({"status":"OFFLINE_TYPED_UUID_CONVERSION_VERIFIED","databases":3,"source_rows":source_rows,"target_rows":target_rows,"excluded_rows":source_rows-target_rows,"mapping_entries":map.len(),"mapping_authorities":map.authority_counts(),"tables":table_counts,"mapping_plaintext_sha256":mapping_digest.sha256,"manifest_plaintext_sha256":manifest_digest.sha256,"registry_sha256":registry_hash}).to_string())
    }
}

#[cfg(test)]
#[path = "transform/tests.rs"]
mod adversarial_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wal_evidence_accepts_full_width_positions_and_rejects_missing_or_overflowed_parts() {
        assert_eq!(parse_lsn("0/0").unwrap(), 0);
        assert_eq!(parse_lsn("FFFFFFFF/FFFFFFFF").unwrap(), u64::MAX);
        assert!(parse_lsn("1/0").unwrap() > parse_lsn("0/FFFFFFFF").unwrap());
        for invalid in ["", "0", "0/", "/1", "0/100000000", "0/-1", "0/1/2", " 0/1"] {
            assert!(parse_lsn(invalid).is_err());
        }
    }

    #[test]
    fn strict_rows_preserve_arbitrary_precision_and_reject_duplicate_or_truncated_envelopes() {
        let root = std::env::temp_dir().join(format!("uuid-transform-test-{}", Uuid::now_v7()));
        fs::create_dir(&root).unwrap();
        let path = root.join("rows");
        fs::write(&path, b"{\"row\":{\"id\":9223372036854775807,\"amount\":123.1234567890123456789},\"createdAtMs\":null}\n").unwrap();
        visit_rows(&path, 1, |_, row| {
            assert_eq!(row.row["id"].to_string(), "9223372036854775807");
            assert_eq!(row.row["amount"].to_string(), "123.1234567890123456789");
            Ok(())
        })
        .unwrap();
        for bad in [
            "{\"row\":{},\"row\":{},\"createdAtMs\":null}\n",
            "{\"row\":{},\"createdAtMs\":null}",
            "{\"row\":{}}\n",
        ] {
            fs::write(&path, bad).unwrap();
            assert!(visit_rows(&path, 1, |_, _| Ok(())).is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }
}
