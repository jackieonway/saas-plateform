//! Read-only authentication of complete UUID source and converted packages.
//! No database connection is opened. Plaintext needed for source coverage and
//! manifests is private tmpfs; all other payload verification streams to a sink.
#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

use super::{codec, export, mapping::TypedMap, rows, structured, transform};
use crate::{config::private_file, io, native::Scratch, Error, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{BufRead, BufReader, Read},
    path::{Component, Path, PathBuf},
};

const MAX_MANIFEST: u64 = 64 * 1024 * 1024;
const MAX_ROW: u64 = 16 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    profile: String,
    source_dir: PathBuf,
    converted_dirs: Vec<PathBuf>,
    scratch: PathBuf,
    registry: PathBuf,
    key_file: PathBuf,
}

/// Authenticated offline inputs for import. Deliberately no Debug: manifests
/// and mappings contain private source identities and operational metadata.
pub(crate) struct VerifiedPackage {
    pub(crate) source: export::Snapshot,
    pub(crate) converted: transform::Converted,
    pub(crate) mapping: TypedMap,
    pub(crate) registry: Value,
    pub(crate) source_digest: codec::FileDigest,
    pub(crate) converted_digest: codec::FileDigest,
    pub(crate) verified_files: u64,
    pub(crate) plaintext_bytes: u64,
    source_stats: Stats,
    converted_stats: Stats,
}

#[derive(Clone, Copy, Default)]
struct Stats {
    files: u64,
    bytes: u64,
    frames: u64,
}

impl Stats {
    fn add(&mut self, digest: &codec::FileDigest) -> Result<()> {
        self.files = self
            .files
            .checked_add(1)
            .ok_or(Error("verification count overflow"))?;
        self.bytes = self
            .bytes
            .checked_add(digest.plaintext_bytes)
            .ok_or(Error("verification size overflow"))?;
        self.frames = self
            .frames
            .checked_add(digest.frames)
            .ok_or(Error("verification frame count overflow"))?;
        Ok(())
    }

    fn combine(self, other: Self) -> Result<Self> {
        Ok(Self {
            files: self
                .files
                .checked_add(other.files)
                .ok_or(Error("verification count overflow"))?,
            bytes: self
                .bytes
                .checked_add(other.bytes)
                .ok_or(Error("verification size overflow"))?,
            frames: self
                .frames
                .checked_add(other.frames)
                .ok_or(Error("verification frame count overflow"))?,
        })
    }
}

struct VerifiedSource {
    manifest: export::Snapshot,
    digest: codec::FileDigest,
    registry: Value,
    registry_hash: String,
    expected_mapping: TypedMap,
    stats: Stats,
}

struct VerifiedConversion {
    manifest: transform::Converted,
    digest: codec::FileDigest,
    mapping: TypedMap,
    stats: Stats,
}

#[derive(PartialEq, Eq, Serialize)]
struct TableEvidence {
    kind: String,
    schema: String,
    table: String,
    policy: String,
    source_rows: u64,
    target_rows: u64,
    target_plaintext_sha256: String,
    target_plaintext_bytes: u64,
    target_frames: u64,
    proof_plaintext_sha256: String,
    proof_plaintext_bytes: u64,
    proof_frames: u64,
}

#[derive(PartialEq, Eq)]
struct ReplayEvidence {
    manifest_digest: codec::FileDigest,
    mapping_digest: codec::FileDigest,
    tables: Vec<TableEvidence>,
    authorities: BTreeMap<String, usize>,
}

struct SourceCoverage {
    mapping: TypedMap,
    collector: structured::SourceCollector,
}

fn safe_path(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        return Err(Error("absolute normalized verification path required"));
    }
    for parent in path.ancestors() {
        let metadata = io(fs::symlink_metadata(parent))?;
        if metadata.file_type().is_symlink() {
            return Err(Error("verification symlink path refused"));
        }
    }
    Ok(())
}

fn private_directory(path: &Path) -> Result<()> {
    safe_path(path)?;
    let metadata = io(fs::symlink_metadata(path))?;
    if !metadata.is_dir() {
        return Err(Error("private verification directory required"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o7777 != 0o700
            || metadata.uid() != io(fs::metadata("/proc/self"))?.uid()
        {
            return Err(Error("verification directory must be owned mode 0700"));
        }
    }
    Ok(())
}

fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    safe_path(path)?;
    let metadata = io(fs::symlink_metadata(path))?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(Error("bounded verification file required"));
    }
    let mut bytes = Vec::new();
    io(io(File::open(path))?
        .take(limit + 1)
        .read_to_end(&mut bytes))?;
    if bytes.len() as u64 > limit {
        return Err(Error("verification file limit exceeded"));
    }
    Ok(bytes)
}

fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    structured::parse_strict(
        std::str::from_utf8(bytes).map_err(|_| Error("verification JSON UTF-8 required"))?,
    )?;
    serde_json::from_slice(bytes).map_err(|_| Error("invalid verification JSON contract"))
}

fn open_bytes(
    path: &Path,
    scratch: &Scratch,
    key: &[u8],
    purpose: &str,
    limit: u64,
) -> Result<(Vec<u8>, codec::FileDigest)> {
    let plain = scratch
        .0
        .join(format!("verify-{}.json", uuid::Uuid::now_v7()));
    let digest = codec::open_file(path, &plain, key, purpose)?;
    let result = bounded(&plain, limit);
    let removed = io(fs::remove_file(&plain));
    let bytes = result?;
    removed?;
    Ok((bytes, digest))
}

fn verify_payload(
    directory: &Path,
    filename: &str,
    purpose: &str,
    key: &[u8],
    expected: &codec::FileDigest,
    stats: &mut Stats,
) -> Result<()> {
    if Path::new(filename).components().count() != 1
        || filename.contains(['/', '\\'])
        || filename.is_empty()
    {
        return Err(Error("verification payload filename refused"));
    }
    let actual = codec::verify_file(&directory.join(filename), key, purpose)?;
    if actual != *expected {
        return Err(Error("authenticated package payload digest mismatch"));
    }
    stats.add(&actual)
}

fn exact_files(directory: &Path, expected: &BTreeSet<String>) -> Result<()> {
    let actual = io(fs::read_dir(directory))?
        .map(|entry| {
            let entry = io(entry)?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| Error("package filename UTF-8 required"))?;
            if !io(entry.file_type())?.is_file() {
                return Err(Error("package contains an unregistered file kind"));
            }
            Ok(name)
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if actual != *expected {
        return Err(Error("package file coverage differs from manifest"));
    }
    Ok(())
}

fn source_rows(
    path: &Path,
    expected_rows: u64,
    mut visit: impl FnMut(export::RowEnvelope) -> Result<()>,
) -> Result<()> {
    let mut reader = BufReader::new(io(File::open(path))?);
    let mut count = 0_u64;
    loop {
        let mut line = Vec::new();
        let length = io(Read::by_ref(&mut reader)
            .take(MAX_ROW + 1)
            .read_until(b'\n', &mut line))?;
        if length == 0 {
            break;
        }
        if length as u64 > MAX_ROW || line.last() != Some(&b'\n') || count >= expected_rows {
            return Err(Error("verification source row framing or count mismatch"));
        }
        let row: export::RowEnvelope = parse(&line)?;
        if !row.row.is_object() {
            return Err(Error("verification source row object required"));
        }
        visit(row)?;
        count = count
            .checked_add(1)
            .ok_or(Error("verification row count overflow"))?;
    }
    if count != expected_rows {
        return Err(Error("verification source row count mismatch"));
    }
    Ok(())
}

fn collect_source_table(
    source_dir: &Path,
    scratch: &Scratch,
    key: &[u8],
    db: &export::DatabaseSnapshot,
    table: &export::TableSnapshot,
    registry: &Value,
    coverage: &mut SourceCoverage,
) -> Result<()> {
    let plain = scratch
        .0
        .join(format!("verify-source-{}.ndjson", uuid::Uuid::now_v7()));
    let digest = codec::open_file(&source_dir.join(&table.file), &plain, key, &table.purpose)?;
    if digest != table.digest {
        let _ = fs::remove_file(&plain);
        return Err(Error("source coverage payload digest mismatch"));
    }
    let columns: BTreeSet<_> = db.catalog["schema_material"]
        .as_array()
        .ok_or(Error("source columns missing"))?
        .iter()
        .filter(|column| {
            column["kind"] == "column"
                && column["schema"] == table.schema
                && column["table"] == table.table
        })
        .map(|column| {
            column["column"]
                .as_str()
                .ok_or(Error("source column name missing"))
        })
        .collect::<Result<_>>()?;
    if columns.is_empty() {
        let _ = fs::remove_file(&plain);
        return Err(Error("source column coverage missing"));
    }
    let result = source_rows(&plain, table.rows, |row| {
        let actual: BTreeSet<_> = row
            .row
            .as_object()
            .ok_or(Error("source row object required"))?
            .keys()
            .map(String::as_str)
            .collect();
        if actual != columns {
            return Err(Error("source row column coverage mismatch"));
        }
        rows::allocate_row(
            &db.kind,
            &table.table,
            &row.row,
            row.created_at_ms,
            &mut coverage.mapping,
            registry,
        )?;
        if structured::needs_source_collection(&db.kind, &table.table) {
            let bytes = serde_json::to_vec(&row.row)
                .map_err(|_| Error("source evidence serialization failed"))?;
            coverage.collector.observe(
                &db.kind,
                &table.table,
                &row.row,
                &crate::bundle::hash(&bytes),
            )?;
        }
        Ok(())
    });
    let removed = io(fs::remove_file(&plain));
    result?;
    removed
}

fn load_source(
    source_dir: &Path,
    scratch: &Scratch,
    registry_path: &Path,
    key: &[u8],
) -> Result<VerifiedSource> {
    private_directory(source_dir)?;
    let registry_bytes = bounded(registry_path, 4 * 1024 * 1024)?;
    if registry_bytes != include_bytes!("../../uuid-references-v2.json") {
        return Err(Error("verification requires reviewed registry bytes"));
    }
    let registry: Value = parse(&registry_bytes)?;
    let registry_hash = crate::bundle::hash(&registry_bytes);
    let (manifest_bytes, digest) = open_bytes(
        &source_dir.join("manifest.enc"),
        scratch,
        key,
        export::MANIFEST_PURPOSE,
        MAX_MANIFEST,
    )?;
    let source: export::Snapshot = parse(&manifest_bytes)?;
    transform::validate_source(&source, &registry, &registry_hash)?;
    let mut stats = Stats::default();
    stats.add(&digest)?;
    let (quiescence_bytes, quiescence_digest) = open_bytes(
        &source_dir.join("quiescence.enc"),
        scratch,
        key,
        &source.quiescence_purpose,
        1024 * 1024,
    )?;
    if quiescence_digest != source.quiescence_digest {
        return Err(Error("source quiescence digest mismatch"));
    }
    export::validate_quiescence(&source, &quiescence_bytes)?;
    stats.add(&quiescence_digest)?;
    let mut expected_files =
        BTreeSet::from(["manifest.enc".to_owned(), "quiescence.enc".to_owned()]);
    for db in &source.databases {
        for table in &db.tables {
            if !expected_files.insert(table.file.clone()) {
                return Err(Error("duplicate source payload filename"));
            }
            verify_payload(
                source_dir,
                &table.file,
                &table.purpose,
                key,
                &table.digest,
                &mut stats,
            )?;
        }
    }
    exact_files(source_dir, &expected_files)?;
    let mut coverage = SourceCoverage {
        mapping: TypedMap::new(source.frozen_ms, transform::authorities(&registry)?)?,
        collector: structured::SourceCollector::new(),
    };
    for db in &source.databases {
        for table in &db.tables {
            collect_source_table(
                source_dir,
                scratch,
                key,
                db,
                table,
                &registry,
                &mut coverage,
            )?;
        }
    }
    coverage
        .collector
        .finish(&mut coverage.mapping, &registry)?;
    Ok(VerifiedSource {
        manifest: source,
        digest,
        registry,
        registry_hash,
        expected_mapping: coverage.mapping,
        stats,
    })
}

fn validate_converted(
    source: &export::Snapshot,
    source_digest: &codec::FileDigest,
    converted: &transform::Converted,
    registry: &Value,
    registry_hash: &str,
) -> Result<BTreeSet<String>> {
    if converted.format != transform::FORMAT
        || converted.batch_id != source.batch_id
        || converted.frozen_ms != source.frozen_ms
        || converted.registry_sha256 != registry_hash
        || converted.source_manifest_sha256 != source_digest.sha256
        || converted.mapping_file != "typed-map.enc"
        || converted.mapping_purpose != format!("{}/typed-map", source.batch_id)
        || converted.databases.len() != source.databases.len()
        || serde_json::to_vec(&converted.source)
            .map_err(|_| Error("source identity serialization failed"))?
            != serde_json::to_vec(source)
                .map_err(|_| Error("source identity serialization failed"))?
    {
        return Err(Error("converted package source binding mismatch"));
    }
    let mut files = BTreeSet::from(["manifest.enc".to_owned(), "typed-map.enc".to_owned()]);
    for (source_db, converted_db) in source.databases.iter().zip(&converted.databases) {
        if source_db.kind != converted_db.kind
            || source_db.tables.len() != converted_db.tables.len()
        {
            return Err(Error("converted database coverage mismatch"));
        }
        for (original, table) in source_db.tables.iter().zip(&converted_db.tables) {
            let policy =
                transform::policy(registry, &source_db.kind, &original.schema, &original.table)?;
            let excluded = matches!(policy, "CREDENTIAL_EXCLUDED" | "REBUILD_METADATA");
            if table.schema != original.schema
                || table.table != original.table
                || table.policy != policy
                || table.source_rows != original.rows
                || table.rows != if excluded { 0 } else { original.rows }
                || table.file
                    != format!(
                        "{}.{}.{}.ndjson.enc",
                        source_db.kind, original.schema, original.table
                    )
                || table.proof_file
                    != format!(
                        "{}.{}.{}.proof.enc",
                        source_db.kind, original.schema, original.table
                    )
                || table.purpose
                    != format!(
                        "{}/converted/{}/{}/{}",
                        source.batch_id, source_db.kind, original.schema, original.table
                    )
                || table.proof_purpose
                    != format!(
                        "{}/proof/{}/{}/{}",
                        source.batch_id, source_db.kind, original.schema, original.table
                    )
                || !files.insert(table.file.clone())
                || !files.insert(table.proof_file.clone())
            {
                return Err(Error("converted table identity or retention mismatch"));
            }
        }
    }
    Ok(files)
}

fn load_conversion(
    source: &VerifiedSource,
    directory: &Path,
    scratch: &Scratch,
    key: &[u8],
) -> Result<VerifiedConversion> {
    private_directory(directory)?;
    let (bytes, digest) = open_bytes(
        &directory.join("manifest.enc"),
        scratch,
        key,
        transform::MANIFEST_PURPOSE,
        MAX_MANIFEST,
    )?;
    let converted: transform::Converted = parse(&bytes)?;
    let files = validate_converted(
        &source.manifest,
        &source.digest,
        &converted,
        &source.registry,
        &source.registry_hash,
    )?;
    exact_files(directory, &files)?;
    let mut stats = Stats::default();
    stats.add(&digest)?;
    let (map_bytes, map_digest) = open_bytes(
        &directory.join(&converted.mapping_file),
        scratch,
        key,
        &converted.mapping_purpose,
        MAX_MANIFEST,
    )?;
    if map_digest != converted.mapping_digest {
        return Err(Error("converted mapping digest mismatch"));
    }
    let mapping = TypedMap::from_bytes(
        &map_bytes,
        source.manifest.frozen_ms,
        transform::authorities(&source.registry)?,
    )?;
    mapping.same_source_coverage(&source.expected_mapping)?;
    stats.add(&map_digest)?;
    for db in &converted.databases {
        for table in &db.tables {
            verify_payload(
                directory,
                &table.file,
                &table.purpose,
                key,
                &table.digest,
                &mut stats,
            )?;
            verify_payload(
                directory,
                &table.proof_file,
                &table.proof_purpose,
                key,
                &table.proof_digest,
                &mut stats,
            )?;
        }
    }
    Ok(VerifiedConversion {
        manifest: converted,
        digest,
        mapping,
        stats,
    })
}

/// Fully authenticate one complete source/converted pair for an offline owner.
/// Source coverage is independently reconstructed before accepting the map.
pub(crate) fn load_verified(
    source_dir: &Path,
    converted_dir: &Path,
    scratch: &Scratch,
    registry_path: &Path,
    key: &[u8],
) -> Result<VerifiedPackage> {
    if source_dir.starts_with(converted_dir) || converted_dir.starts_with(source_dir) {
        return Err(Error("separate verification package directories required"));
    }
    let source = load_source(source_dir, scratch, registry_path, key)?;
    let converted = load_conversion(&source, converted_dir, scratch, key)?;
    let stats = source.stats.combine(converted.stats)?;
    Ok(VerifiedPackage {
        source: source.manifest,
        converted: converted.manifest,
        mapping: converted.mapping,
        registry: source.registry,
        source_digest: source.digest,
        converted_digest: converted.digest,
        verified_files: stats.files,
        plaintext_bytes: stats.bytes,
        source_stats: source.stats,
        converted_stats: converted.stats,
    })
}

fn table_evidence(converted: &transform::Converted) -> Vec<TableEvidence> {
    converted
        .databases
        .iter()
        .flat_map(|db| {
            db.tables.iter().map(move |table| TableEvidence {
                kind: db.kind.clone(),
                schema: table.schema.clone(),
                table: table.table.clone(),
                policy: table.policy.clone(),
                source_rows: table.source_rows,
                target_rows: table.rows,
                target_plaintext_sha256: table.digest.sha256.clone(),
                target_plaintext_bytes: table.digest.plaintext_bytes,
                target_frames: table.digest.frames,
                proof_plaintext_sha256: table.proof_digest.sha256.clone(),
                proof_plaintext_bytes: table.proof_digest.plaintext_bytes,
                proof_frames: table.proof_digest.frames,
            })
        })
        .collect()
}

fn replay_evidence(
    manifest: &transform::Converted,
    digest: &codec::FileDigest,
    mapping: &TypedMap,
) -> ReplayEvidence {
    ReplayEvidence {
        manifest_digest: digest.clone(),
        mapping_digest: manifest.mapping_digest.clone(),
        tables: table_evidence(manifest),
        authorities: mapping
            .authority_counts()
            .into_iter()
            .map(|(name, count)| (name.to_owned(), count))
            .collect(),
    }
}

fn conversion_report(evidence: &ReplayEvidence, entries: usize, stats: Stats) -> Value {
    json!({"manifest":evidence.manifest_digest,"mapping":evidence.mapping_digest,"mapping_entries":entries,
        "mapping_authorities":evidence.authorities,"tables":evidence.tables,"verified_files":stats.files,
        "plaintext_bytes":stats.bytes,"frames":stats.frames})
}

pub(crate) fn execute(path: &Path) -> Result<String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        Err(Error("package verification requires Linux private tmpfs"))
    }
    #[cfg(target_os = "linux")]
    {
        safe_path(path)?;
        let config: Config = parse(&private_file(path)?)?;
        if config.profile != "uuid-local-preview"
            || config.converted_dirs.is_empty()
            || config.converted_dirs.len() > 8
        {
            return Err(Error(
                "local preview and one to eight converted packages required",
            ));
        }
        let mut directories = BTreeSet::new();
        for directory in &config.converted_dirs {
            if directory.starts_with(&config.source_dir)
                || config.source_dir.starts_with(directory)
                || !directories.insert(directory)
                || config.key_file.starts_with(directory)
            {
                return Err(Error("separate unique verification directories required"));
            }
        }
        safe_path(&config.scratch)?;
        safe_path(&config.key_file)?;
        if config.key_file.starts_with(&config.source_dir) {
            return Err(Error("separate verification key required"));
        }
        let key = private_file(&config.key_file)?;
        if key.len() != 32 {
            return Err(Error("verification key must contain 32 bytes"));
        }
        let scratch = Scratch::new(&config.scratch)?;
        let first_directory = config
            .converted_dirs
            .first()
            .ok_or(Error("converted verification package required"))?;
        let package = load_verified(
            &config.source_dir,
            first_directory,
            &scratch,
            &config.registry,
            &key,
        )?;
        let mut stats = package.source_stats.combine(package.converted_stats)?;
        if stats.files != package.verified_files || stats.bytes != package.plaintext_bytes {
            return Err(Error("verified package summary mismatch"));
        }
        let first = replay_evidence(
            &package.converted,
            &package.converted_digest,
            &package.mapping,
        );
        let mut reports = vec![conversion_report(
            &first,
            package.mapping.len(),
            package.converted_stats,
        )];
        let source = VerifiedSource {
            registry_hash: package.source.registry_sha256.clone(),
            manifest: package.source,
            digest: package.source_digest,
            registry: package.registry,
            expected_mapping: package.mapping,
            stats: package.source_stats,
        };
        for directory in config.converted_dirs.iter().skip(1) {
            let converted = load_conversion(&source, directory, &scratch, &key)?;
            let replay =
                replay_evidence(&converted.manifest, &converted.digest, &converted.mapping);
            if first != replay {
                return Err(Error("converted package replay plaintext differs"));
            }
            reports.push(conversion_report(
                &replay,
                converted.mapping.len(),
                converted.stats,
            ));
            stats = stats.combine(converted.stats)?;
        }
        let source_tables: Vec<_> = source.manifest.databases.iter().flat_map(|db| db.tables.iter().map(move |table| {
            json!({"kind":db.kind,"schema":table.schema,"table":table.table,"rows":table.rows,"digest":table.digest})
        })).collect();
        Ok(json!({"status":"AUTHENTICATED_UUID_PACKAGES_VERIFIED","source_manifest_plaintext_sha256":source.digest.sha256,
            "source_manifest":source.digest,"source_tables":source_tables,"source_plaintext_bytes":source.stats.bytes,"source_frames":source.stats.frames,
            "registry_sha256":source.registry_hash,"source_files":source.stats.files,"converted_packages":reports,
            "compared_packages":config.converted_dirs.len(),"same_plaintext_across_conversions":config.converted_dirs.len()>1,
            "verified_files":stats.files,"plaintext_bytes":stats.bytes,"frames":stats.frames}).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest() -> codec::FileDigest {
        codec::FileDigest {
            sha256: "a".repeat(64),
            plaintext_bytes: 12,
            frames: 1,
        }
    }

    fn fixture() -> (export::Snapshot, transform::Converted, Value, String) {
        let registry: Value = parse(include_bytes!("../../uuid-references-v2.json")).unwrap();
        let registry_hash = crate::bundle::hash(include_bytes!("../../uuid-references-v2.json"));
        let batch = uuid::Uuid::now_v7();
        let tables = [("users", 3), ("oauth_tokens", 2)]
            .into_iter()
            .map(|(table, count)| export::TableSnapshot {
                schema: "public".into(),
                table: table.into(),
                rows: count,
                file: format!("platform.public.{table}.ndjson.enc"),
                purpose: format!("saas-uuid-source-table/1/{batch}/platform/public/{table}"),
                digest: digest(),
            })
            .collect();
        let source = export::Snapshot {
            format: export::SNAPSHOT_FORMAT.into(),
            batch_id: batch,
            frozen_ms: 1_792_800_000_123,
            registry_sha256: registry_hash.clone(),
            admission_before: Vec::new(),
            admission_after: Vec::new(),
            databases: vec![export::DatabaseSnapshot {
                kind: "platform".into(),
                name: "qiankun_platform_preview_20261024".into(),
                catalog: Value::Null,
                snapshot: "100:100:".into(),
                lsn: "0/10".into(),
                final_lsn: "0/20".into(),
                sequences: Vec::new(),
                tables,
            }],
            quiescence_sha256: "b".repeat(64),
            quiescence_file: "quiescence.enc".into(),
            quiescence_purpose: format!("saas-uuid-source-quiescence/1/{batch}"),
            quiescence_digest: digest(),
        };
        let converted_tables = source.databases[0]
            .tables
            .iter()
            .map(|table| {
                let policy =
                    transform::policy(&registry, "platform", "public", &table.table).unwrap();
                transform::ConvertedTable {
                    schema: "public".into(),
                    table: table.table.clone(),
                    policy: policy.into(),
                    source_rows: table.rows,
                    rows: if policy == "CREDENTIAL_EXCLUDED" {
                        0
                    } else {
                        table.rows
                    },
                    file: table.file.clone(),
                    purpose: format!("{batch}/converted/platform/public/{}", table.table),
                    digest: digest(),
                    proof_file: format!("platform.public.{}.proof.enc", table.table),
                    proof_purpose: format!("{batch}/proof/platform/public/{}", table.table),
                    proof_digest: digest(),
                }
            })
            .collect();
        let converted = transform::Converted {
            format: transform::FORMAT.into(),
            batch_id: batch,
            frozen_ms: source.frozen_ms,
            registry_sha256: registry_hash.clone(),
            source_manifest_sha256: digest().sha256,
            mapping_file: "typed-map.enc".into(),
            mapping_purpose: format!("{batch}/typed-map"),
            mapping_digest: digest(),
            source: parse(&serde_json::to_vec(&source).unwrap()).unwrap(),
            databases: vec![transform::ConvertedDatabase {
                kind: "platform".into(),
                tables: converted_tables,
            }],
        };
        (source, converted, registry, registry_hash)
    }

    #[test]
    fn converted_manifest_binds_source_mapping_and_embedded_evidence() {
        let (source, converted, registry, registry_hash) = fixture();
        assert_eq!(
            validate_converted(&source, &digest(), &converted, &registry, &registry_hash)
                .unwrap()
                .len(),
            6
        );
        let original = serde_json::to_value(&converted).unwrap();
        for (pointer, invalid) in [
            ("/source_manifest_sha256", json!("b".repeat(64))),
            ("/mapping_file", json!("../typed-map.enc")),
            ("/mapping_purpose", json!("other-batch/typed-map")),
            ("/source/databases/0/snapshot", json!("101:101:")),
            ("/source/databases/0/final_lsn", json!("0/30")),
            ("/frozen_ms", json!(source.frozen_ms + 1)),
        ] {
            let mut changed = original.clone();
            *changed.pointer_mut(pointer).unwrap() = invalid;
            let changed: transform::Converted = serde_json::from_value(changed).unwrap();
            assert!(
                validate_converted(&source, &digest(), &changed, &registry, &registry_hash)
                    .is_err(),
                "{pointer}"
            );
        }
    }

    #[test]
    fn converted_tables_require_exact_coverage_purpose_policy_and_exclusion() {
        let (source, converted, registry, registry_hash) = fixture();
        let original = serde_json::to_value(&converted).unwrap();
        for (pointer, invalid) in [
            ("/databases/0/kind", json!("stores")),
            ("/databases/0/tables/0/file", json!("../source.ndjson.enc")),
            ("/databases/0/tables/0/proof_file", json!("manifest.enc")),
            (
                "/databases/0/tables/0/proof_purpose",
                json!("incorrect-purpose"),
            ),
            ("/databases/0/tables/0/policy", json!("CREDENTIAL_EXCLUDED")),
            ("/databases/0/tables/0/rows", json!(2)),
            ("/databases/0/tables/0/source_rows", json!(4)),
            ("/databases/0/tables/1/rows", json!(2)),
            ("/databases/0/tables", json!([])),
        ] {
            let mut changed = original.clone();
            *changed.pointer_mut(pointer).unwrap() = invalid;
            let changed: transform::Converted = serde_json::from_value(changed).unwrap();
            assert!(
                validate_converted(&source, &digest(), &changed, &registry, &registry_hash)
                    .is_err(),
                "{pointer}"
            );
        }
    }

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let directory =
                std::env::temp_dir().join(format!("uuid-verify-test-{}", uuid::Uuid::now_v7()));
            fs::create_dir(&directory).unwrap();
            Self(directory)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn source_row_parser_rejects_duplicate_keys_missing_framing_and_count_drift() {
        let directory = Directory::new();
        let path = directory.0.join("rows");
        fs::write(&path, b"{\"row\":{\"n\":9223372036854775807,\"amount\":0.000000000000000000001},\"createdAtMs\":null}\n").unwrap();
        source_rows(&path, 1, |row| {
            assert_eq!(row.row["n"].to_string(), "9223372036854775807");
            assert_eq!(row.row["amount"].to_string(), "0.000000000000000000001");
            Ok(())
        })
        .unwrap();
        assert!(source_rows(&path, 0, |_| Ok(())).is_err());
        assert!(source_rows(&path, 2, |_| Ok(())).is_err());
        for invalid in [
            "{\"row\":{},\"createdAtMs\":null}",
            "{\"row\":{\"n\":1,\"n\":2},\"createdAtMs\":null}\n",
            "{\"row\":{}}\n",
        ] {
            fs::write(&path, invalid).unwrap();
            assert!(source_rows(&path, 1, |_| Ok(())).is_err());
        }
    }

    #[test]
    fn package_inventory_rejects_extra_files_and_nested_directories() {
        let directory = Directory::new();
        fs::write(directory.0.join("manifest.enc"), []).unwrap();
        let expected = BTreeSet::from(["manifest.enc".to_owned()]);
        exact_files(&directory.0, &expected).unwrap();
        fs::write(directory.0.join("unregistered.enc"), []).unwrap();
        assert!(exact_files(&directory.0, &expected).is_err());
        fs::remove_file(directory.0.join("unregistered.enc")).unwrap();
        fs::create_dir(directory.0.join("unregistered.enc")).unwrap();
        assert!(exact_files(&directory.0, &expected).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn real_sink_readback_compares_complete_digest_and_rejects_ciphertext_damage() {
        use std::os::unix::fs::DirBuilderExt;
        let root =
            PathBuf::from("/dev/shm").join(format!("uuid-verify-private-{}", uuid::Uuid::now_v7()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let directory = Directory(root);
        let plain = directory.0.join("plain");
        let encrypted = directory.0.join("payload.enc");
        crate::native::create_private(&plain, b"private migration evidence").unwrap();
        let key = [0x51_u8; 32];
        let digest = codec::seal_file(&plain, &encrypted, &key, "fixture/source/table").unwrap();
        let mut stats = Stats::default();
        verify_payload(
            &directory.0,
            "payload.enc",
            "fixture/source/table",
            &key,
            &digest,
            &mut stats,
        )
        .unwrap();
        assert_eq!((stats.files, stats.bytes, stats.frames), (1, 26, 1));
        let mut mismatch = digest.clone();
        mismatch.plaintext_bytes += 1;
        assert!(verify_payload(
            &directory.0,
            "payload.enc",
            "fixture/source/table",
            &key,
            &mismatch,
            &mut stats
        )
        .is_err());
        assert_eq!(stats.files, 1);
        let mut ciphertext = fs::read(&encrypted).unwrap();
        let last = ciphertext.len() - 1;
        ciphertext[last] ^= 1;
        fs::write(&encrypted, ciphertext).unwrap();
        assert!(verify_payload(
            &directory.0,
            "payload.enc",
            "fixture/source/table",
            &key,
            &digest,
            &mut stats
        )
        .is_err());
        assert_eq!(stats.files, 1);
    }
}
