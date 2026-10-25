//! Import an authenticated UUID package into three explicitly prepared, empty
//! local targets. Creation/migrations and all service lifecycle remain external.
#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports, unused_mut))]
mod metadata;
mod session;
mod sql;

use super::{codec, export, reconcile, structured, transform, verify};
use crate::{config::private_file, io, native::Scratch, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::BufReader,
    path::{Component, Path, PathBuf},
};

const MAX_LINE: u64 = 16 * 1024 * 1024;
const TARGETS: [(&str, &str, &str); 3] = [
    (
        "platform",
        "qiankun_platform_uuid7_u04_20261024",
        "qiankun_platform_uuid7_u04_owner_20261024",
    ),
    (
        "stores",
        "qiankun_stores_uuid7_u04_20261024",
        "qiankun_stores_uuid7_u04_owner_20261024",
    ),
    (
        "work_orders",
        "qiankun_orders_uuid7_u04_20261024",
        "qiankun_orders_uuid7_u04_owner_20261024",
    ),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    kind: String,
    name: String,
    owner: String,
    expected_schema_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    profile: String,
    mode: String,
    seed: bool,
    host: String,
    port: u16,
    user: String,
    databases: Vec<Target>,
    source_dir: PathBuf,
    converted_dir: PathBuf,
    registry: PathBuf,
    key_file: PathBuf,
    scratch: PathBuf,
    output_dir: PathBuf,
    timeout_seconds: u64,
}

fn strict<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let text = std::str::from_utf8(bytes).map_err(|_| Error("import JSON UTF-8 required"))?;
    serde_json::from_value(structured::parse_strict(text)?)
        .map_err(|_| Error("invalid typed import evidence"))
}

fn safe_path(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|p| matches!(p, Component::CurDir | Component::ParentDir))
    {
        return Err(Error("absolute normalized import paths required"));
    }
    for parent in path.ancestors() {
        match fs::symlink_metadata(parent) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(Error("import symlink path rejected"))
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Error("import path inspection failed")),
        }
    }
    Ok(())
}

fn hash(value: &Value) -> Result<String> {
    Ok(crate::bundle::hash(&serde_json::to_vec(value).map_err(
        |_| Error("import evidence serialization failed"),
    )?))
}

fn is_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Config {
    fn validate(&self) -> Result<()> {
        let identities: BTreeSet<_> = self
            .databases
            .iter()
            .map(|t| (t.kind.as_str(), t.name.as_str(), t.owner.as_str()))
            .collect();
        if self.profile != "uuid-local-preview"
            || self.mode != "import"
            || self.seed
            || self.host != "/run/postgresql"
            || self.port != 5432
            || self.user != "postgres"
            || !(10..=1800).contains(&self.timeout_seconds)
            || self.databases.len() != 3
            || identities != BTreeSet::from(TARGETS)
            || self
                .databases
                .iter()
                .any(|t| !is_hash(&t.expected_schema_sha256))
        {
            return Err(Error(
                "fixed local U04 targets and exclusive import mode required",
            ));
        }
        for path in [
            &self.source_dir,
            &self.converted_dir,
            &self.registry,
            &self.key_file,
            &self.scratch,
            &self.output_dir,
        ] {
            safe_path(path)?;
        }
        if self.source_dir == self.converted_dir
            || self.source_dir.starts_with(&self.output_dir)
            || self.converted_dir.starts_with(&self.output_dir)
            || self.key_file.starts_with(&self.output_dir)
            || self.registry.starts_with(&self.output_dir)
            || self.output_dir.starts_with(&self.source_dir)
            || self.output_dir.starts_with(&self.converted_dir)
            || fs::symlink_metadata(&self.output_dir).is_ok()
        {
            return Err(Error("new separate import evidence directory required"));
        }
        Ok(())
    }
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        io(file.set_permissions(fs::Permissions::from_mode(0o600)))?;
        let m = io(file.metadata())?;
        if !m.is_file()
            || m.mode() & 0o7777 != 0o600
            || m.uid() != io(fs::metadata("/proc/self"))?.uid()
        {
            return Err(Error("owned private import file required"));
        }
    }
    Ok(file)
}

fn private_output(path: &Path) -> Result<()> {
    safe_path(path)?;
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    io(builder.create(path))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        io(fs::set_permissions(path, fs::Permissions::from_mode(0o700)))?;
        let m = io(fs::metadata(path))?;
        if m.mode() & 0o7777 != 0o700 || m.uid() != io(fs::metadata("/proc/self"))?.uid() {
            return Err(Error("owned private import directory required"));
        }
    }
    Ok(())
}

fn nullable_string<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RowProof {
    index: u64,
    policy: String,
    source_sha256: String,
    #[serde(deserialize_with = "nullable_string")]
    target_sha256: Option<String>,
    business_proof: Value,
}

struct TableFiles {
    source: PathBuf,
    target: PathBuf,
    proof: PathBuf,
}
impl Drop for TableFiles {
    fn drop(&mut self) {
        for path in [&self.source, &self.target, &self.proof] {
            let _ = fs::remove_file(path);
        }
    }
}
impl TableFiles {
    fn open(
        config: &Config,
        scratch: &Scratch,
        key: &[u8],
        source: &export::TableSnapshot,
        target: &transform::ConvertedTable,
    ) -> Result<Self> {
        let files = Self {
            source: scratch.0.join("import-source.ndjson"),
            target: scratch.0.join("import-target.ndjson"),
            proof: scratch.0.join("import-proof.ndjson"),
        };
        for (cipher, plain, purpose, expected) in [
            (
                config.source_dir.join(&source.file),
                &files.source,
                source.purpose.as_str(),
                &source.digest,
            ),
            (
                config.converted_dir.join(&target.file),
                &files.target,
                target.purpose.as_str(),
                &target.digest,
            ),
            (
                config.converted_dir.join(&target.proof_file),
                &files.proof,
                target.proof_purpose.as_str(),
                &target.proof_digest,
            ),
        ] {
            if codec::open_file(&cipher, plain, key, purpose)? != *expected {
                return Err(Error("import table authenticated digest mismatch"));
            }
        }
        Ok(files)
    }

    fn visit(
        &self,
        package: &verify::VerifiedPackage,
        kind: &str,
        table: &transform::ConvertedTable,
        mut visit: impl FnMut(u64, &Value) -> Result<()>,
    ) -> Result<()> {
        let mut source = BufReader::new(io(File::open(&self.source))?);
        let mut target = BufReader::new(io(File::open(&self.target))?);
        let mut proofs = BufReader::new(io(File::open(&self.proof))?);
        let excluded = matches!(
            table.policy.as_str(),
            "REBUILD_METADATA" | "CREDENTIAL_EXCLUDED"
        );
        for index in 0..table.source_rows {
            let original: export::RowEnvelope = strict(
                &session::line(&mut source)?.ok_or(Error("import source evidence ended early"))?,
            )?;
            let proof: RowProof =
                strict(&session::line(&mut proofs)?.ok_or(Error("import row proof ended early"))?)?;
            if proof.index != index
                || proof.policy != table.policy
                || proof.source_sha256 != hash(&original.row)?
            {
                return Err(Error("import row source proof mismatch"));
            }
            if excluded {
                if proof.target_sha256.is_some() || !proof.business_proof.is_null() {
                    return Err(Error("excluded import table has target row proof"));
                }
            } else {
                let converted: Value = strict(
                    &session::line(&mut target)?
                        .ok_or(Error("import target evidence ended early"))?,
                )?;
                let business = serde_json::to_value(reconcile::projection_proof(
                    kind,
                    &table.table,
                    &original.row,
                    &converted,
                    &package.mapping,
                    &package.registry,
                )?)
                .map_err(|_| Error("import business proof serialization failed"))?;
                if proof.target_sha256.as_deref() != Some(hash(&converted)?.as_str())
                    || proof.business_proof != business
                {
                    return Err(Error("import target or business projection proof mismatch"));
                }
                visit(
                    index
                        .checked_add(1)
                        .ok_or(Error("import ordinal overflow"))?,
                    &converted,
                )?;
            }
        }
        if session::line(&mut source)?.is_some()
            || session::line(&mut target)?.is_some()
            || session::line(&mut proofs)?.is_some()
            || table.rows != if excluded { 0 } else { table.source_rows }
        {
            return Err(Error("import table evidence count mismatch"));
        }
        Ok(())
    }
}

fn csv(ordinal: u64, row: &Value) -> Result<Vec<u8>> {
    let bytes =
        serde_json::to_vec(row).map_err(|_| Error("import COPY row serialization failed"))?;
    if ordinal == 0 || !row.is_object() || bytes.len() as u64 > MAX_LINE {
        return Err(Error("import COPY row or ordinal invalid"));
    }
    let mut result = format!("{ordinal},\"").into_bytes();
    for byte in bytes {
        result.push(byte);
        if byte == b'"' {
            result.push(byte);
        }
    }
    result.extend_from_slice(b"\"\n");
    Ok(result)
}

fn catalog_sql(timeout: u64) -> Result<String> {
    let sql = crate::uuid_inventory::catalog_query()
        .trim()
        .strip_prefix("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;")
        .and_then(|s| s.trim().strip_suffix("COMMIT;"))
        .and_then(|s| s.trim_start().strip_prefix("SET TRANSACTION READ ONLY;"))
        .ok_or(Error("unsupported target catalog query contract"))?;
    Ok(format!(
        "{sql}\nSET LOCAL statement_timeout='{timeout}s';\n"
    ))
}

fn schema_hash(catalog: &Value) -> Result<String> {
    hash(&catalog["schema_material"])
}

fn ident(value: &str) -> Result<String> {
    if value.is_empty()
        || value.len() > 63
        || value.as_bytes()[0].is_ascii_digit()
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(Error("unreviewed import SQL identifier"));
    }
    Ok(format!("\"{value}\""))
}

fn fence_sql(target: &Target) -> String {
    let databases = TARGETS
        .iter()
        .map(|(_, name, _)| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(",");
    let owners = TARGETS
        .iter()
        .map(|(_, _, owner)| format!("'{owner}'"))
        .collect::<Vec<_>>()
        .join(",");
    format!("SELECT jsonb_build_object('database',current_database(),'database_oid',d.oid::bigint,'owner_oid',r.oid::bigint,\
        'actor_valid',current_user='postgres' AND session_user='postgres',\
        'local',inet_server_addr() IS NULL AND inet_server_port() IS NULL AND current_setting('port')='5432',\
        'transaction_valid',current_setting('transaction_isolation')='read committed' AND NOT current_setting('transaction_read_only')::boolean,\
        'owner_valid',r.rolname='{owner}' AND NOT r.rolcanlogin AND NOT r.rolsuper,\
        'database_valid',d.datallowconn AND NOT d.datistemplate,\
        'object_ownership_valid',\
          NOT EXISTS(SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind IN ('r','p','i','I','S','v','m','f') AND c.relowner<>d.datdba) AND \
          NOT EXISTS(SELECT 1 FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public' AND p.proowner<>d.datdba AND NOT EXISTS(SELECT 1 FROM pg_depend e WHERE e.classid='pg_proc'::regclass AND e.objid=p.oid AND e.deptype='e')) AND \
          (SELECT n.nspowner=d.datdba OR n.nspowner=(SELECT oid FROM pg_roles WHERE rolname='pg_database_owner') FROM pg_namespace n WHERE n.nspname='public'),\
        'acl_valid',\
          NOT EXISTS(SELECT 1 FROM pg_namespace n CROSS JOIN LATERAL aclexplode(COALESCE(n.nspacl,acldefault('n',n.nspowner))) a WHERE n.nspname='public' AND a.privilege_type='CREATE' AND a.grantee NOT IN (d.datdba,(SELECT oid FROM pg_roles WHERE rolname='postgres'),(SELECT oid FROM pg_roles WHERE rolname='pg_database_owner'))) AND \
          NOT EXISTS(SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace CROSS JOIN LATERAL aclexplode(COALESCE(c.relacl,acldefault(CASE WHEN c.relkind='S' THEN 's'::\"char\" ELSE 'r'::\"char\" END,c.relowner))) a WHERE n.nspname='public' AND c.relkind IN ('r','p','S','v','m','f') AND a.privilege_type IN ('INSERT','UPDATE','DELETE','TRUNCATE','REFERENCES','TRIGGER','MAINTAIN','USAGE') AND a.grantee NOT IN (d.datdba,(SELECT oid FROM pg_roles WHERE rolname='postgres'))) AND \
          NOT EXISTS(SELECT 1 FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace CROSS JOIN LATERAL aclexplode(COALESCE(p.proacl,acldefault('f',p.proowner))) a WHERE n.nspname='public' AND NOT EXISTS(SELECT 1 FROM pg_depend e WHERE e.classid='pg_proc'::regclass AND e.objid=p.oid AND e.deptype='e') AND (a.grantee NOT IN (0,d.datdba,(SELECT oid FROM pg_roles WHERE rolname='postgres')) OR a.grantee=0 AND a.is_grantable)) AND \
          NOT EXISTS(SELECT 1 FROM pg_default_acl x CROSS JOIN LATERAL aclexplode(x.defaclacl) a WHERE x.defaclrole=d.datdba AND (x.defaclnamespace=0 OR x.defaclnamespace=(SELECT oid FROM pg_namespace WHERE nspname='public')) AND a.privilege_type IN ('CREATE','INSERT','UPDATE','DELETE','TRUNCATE','REFERENCES','TRIGGER','MAINTAIN','USAGE') AND a.grantee NOT IN (d.datdba,(SELECT oid FROM pg_roles WHERE rolname='postgres'))),\
        'bad_data_writers',(SELECT count(*) FROM pg_roles l WHERE l.rolcanlogin AND l.rolname<>'postgres' AND (\
          has_schema_privilege(l.oid,'public','CREATE') OR \
          EXISTS(SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE CASE WHEN n.nspname='public' AND c.relkind IN ('r','p','v','m','f') THEN has_table_privilege(l.oid,c.oid,'INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER') ELSE false END) OR \
          EXISTS(SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE CASE WHEN n.nspname='public' AND c.relkind='S' THEN has_sequence_privilege(l.oid,c.oid,'USAGE,UPDATE') ELSE false END))),\
        'default_read_only',EXISTS(SELECT 1 FROM pg_db_role_setting s WHERE s.setdatabase=d.oid AND s.setrole=0 AND s.setconfig @> ARRAY['default_transaction_read_only=on']),\
        'other_backends',(SELECT count(*) FROM pg_stat_activity WHERE datname IN ({databases}) AND pid<>pg_backend_pid()),\
        'prepared_transactions',(SELECT count(*) FROM pg_prepared_xacts WHERE database IN ({databases})),\
        'bad_logins',(SELECT count(*) FROM pg_roles l WHERE l.rolcanlogin AND l.rolname<>'postgres' AND (l.rolsuper OR l.rolcreaterole OR \
          EXISTS(SELECT 1 FROM pg_database b WHERE b.datname IN ({databases}) AND has_database_privilege(l.oid,b.oid,'CONNECT')) OR \
          EXISTS(SELECT 1 FROM pg_roles p WHERE (p.rolname IN ({owners}) OR p.rolsuper OR p.rolcreaterole) AND pg_has_role(l.oid,p.oid,'SET')) OR \
          EXISTS(SELECT 1 FROM pg_auth_members m JOIN pg_roles p ON p.oid=m.roleid WHERE p.rolname IN ({owners}) AND m.admin_option AND pg_has_role(l.oid,m.member,'SET')))),\
        'comment',shobj_description(d.oid,'pg_database')) FROM pg_database d JOIN pg_roles r ON r.oid=d.datdba WHERE d.datname=current_database();\n",owner=target.owner)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Fence {
    database: String,
    database_oid: u32,
    owner_oid: u32,
    actor_valid: bool,
    local: bool,
    transaction_valid: bool,
    owner_valid: bool,
    database_valid: bool,
    object_ownership_valid: bool,
    acl_valid: bool,
    bad_data_writers: u64,
    default_read_only: bool,
    other_backends: u64,
    prepared_transactions: u64,
    bad_logins: u64,
    #[serde(deserialize_with = "nullable_string")]
    comment: Option<String>,
}

fn checked_fence(value: Value, target: &Target, comment: Option<&str>) -> Result<Fence> {
    let f: Fence = strict(
        &serde_json::to_vec(&value).map_err(|_| Error("import fence serialization failed"))?,
    )?;
    if f.database != target.name
        || f.database_oid == 0
        || f.owner_oid == 0
        || !f.actor_valid
        || !f.local
        || !f.transaction_valid
        || !f.owner_valid
        || !f.database_valid
        || !f.object_ownership_valid
        || !f.acl_valid
        || f.bad_data_writers != 0
        || !f.default_read_only
        || f.other_backends != 0
        || f.prepared_transactions != 0
        || f.bad_logins != 0
        || f.comment.as_deref() != comment
    {
        return Err(Error(
            "target admission, clean epoch, or ownership fence failed",
        ));
    }
    Ok(f)
}

fn database<'a>(
    package: &'a verify::VerifiedPackage,
    kind: &str,
) -> Result<(
    &'a export::DatabaseSnapshot,
    &'a transform::ConvertedDatabase,
)> {
    Ok((
        package
            .source
            .databases
            .iter()
            .find(|d| d.kind == kind)
            .ok_or(Error("source database missing"))?,
        package
            .converted
            .databases
            .iter()
            .find(|d| d.kind == kind)
            .ok_or(Error("converted database missing"))?,
    ))
}

fn final_counts(catalog: &Value, converted: &transform::ConvertedDatabase) -> Result<()> {
    let relations = catalog["relations"]
        .as_array()
        .ok_or(Error("final target relations missing"))?;
    for table in &converted.tables {
        if table.policy == "REBUILD_METADATA" {
            continue;
        }
        let count = relations
            .iter()
            .find(|r| r["schema"] == table.schema && r["table"] == table.table)
            .and_then(|r| r["row_count"].as_u64())
            .ok_or(Error("final target table count missing"))?;
        if count != table.rows {
            return Err(Error("final target retention row count mismatch"));
        }
    }
    Ok(())
}

fn event(value: Value, expected: &str) -> Result<()> {
    if value.as_object().is_none_or(|o| o.len() != 1) || value["kind"] != expected {
        return Err(Error("unexpected import completion evidence"));
    }
    Ok(())
}

fn sequence_restore(
    session: &mut session::Session,
    source: &export::DatabaseSnapshot,
    target_catalog: &Value,
) -> Result<()> {
    if export::sequence_names(target_catalog)? != export::sequence_names(&source.catalog)? {
        return Err(Error(
            "target sequence definitions differ from source coverage",
        ));
    }
    for sequence in &source.sequences {
        if source.kind != "platform"
            || sequence.schema != "public"
            || sequence.name != "audit_logs_id_seq"
        {
            return Err(Error("unregistered import sequence"));
        }
        let next = if sequence.is_called {
            sequence.last_value.checked_add(1)
        } else {
            Some(sequence.last_value)
        }
        .ok_or(Error("source audit sequence exhausted"))?;
        session.sql(format!("SELECT jsonb_build_object('kind','sequence_safe','safe',COALESCE((SELECT max(id) FROM public.audit_logs),0)::numeric<{next}::numeric);\n"))?;
        let safe = session.value()?;
        if safe != json!({"kind":"sequence_safe","safe":true}) {
            return Err(Error(
                "source audit sequence would collide with retained audit IDs",
            ));
        }
        let name = format!("{}.{}", ident(&sequence.schema)?, ident(&sequence.name)?);
        session.sql(format!("SELECT jsonb_build_object('kind','sequence_set','last_value',pg_catalog.setval('{name}'::regclass,{},{}));\nSELECT jsonb_build_object('kind','sequence_verified','last_value',last_value,'is_called',is_called) FROM {name};\n",sequence.last_value,sequence.is_called))?;
        if session.value()? != json!({"kind":"sequence_set","last_value":sequence.last_value})
            || session.value()?
                != json!({"kind":"sequence_verified","last_value":sequence.last_value,"is_called":sequence.is_called})
        {
            return Err(Error(
                "target sequence restoration mismatch; target remains quarantined",
            ));
        }
    }
    Ok(())
}

/// Import only into the fixed, independently migrated U04 rehearsal targets.
pub(crate) fn execute(path: &Path) -> Result<String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        Err(Error("UUID import requires Linux private tmpfs"))
    }
    #[cfg(target_os = "linux")]
    {
        if std::env::var("UUID_TARGETS_QUIESCED").as_deref() != Ok("true") {
            return Err(Error("explicit UUID_TARGETS_QUIESCED=true required"));
        }
        safe_path(path)?;
        let config: Config = strict(&private_file(path)?)?;
        config.validate()?;
        let key = private_file(&config.key_file)?;
        if key.len() != 32 {
            return Err(Error("import key must contain 32 random bytes"));
        }
        let scratch = Scratch::new(&config.scratch)?;
        let package = verify::load_verified(
            &config.source_dir,
            &config.converted_dir,
            &scratch,
            &config.registry,
            &key,
        )?;
        // Every source, target and proof line is checked before the first target write.
        for converted in &package.converted.databases {
            let (source, _) = database(&package, &converted.kind)?;
            for table in &converted.tables {
                let original = source
                    .tables
                    .iter()
                    .find(|t| t.schema == table.schema && t.table == table.table)
                    .ok_or(Error("source table missing"))?;
                TableFiles::open(&config, &scratch, &key, original, table)?.visit(
                    &package,
                    &converted.kind,
                    table,
                    |_, _| Ok(()),
                )?;
            }
        }
        let mut identities = Vec::new();
        for target in &config.databases {
            let (source, converted) = database(&package, &target.kind)?;
            let mut session = session::Session::start(&config, target, &scratch)?;
            session.sql(sql::begin_sql(&target.kind)?)?;
            session.sql(fence_sql(target))?;
            let identity = checked_fence(session.value()?, target, None)?;
            session.sql(metadata::query(&target.kind)?)?;
            metadata::validate(&target.kind, &session.value()?)?;
            session.sql(catalog_sql(config.timeout_seconds)?)?;
            let catalog = session.value()?;
            if schema_hash(&catalog)? != target.expected_schema_sha256 {
                return Err(Error(
                    "prepared target schema differs from reviewed initialization",
                ));
            }
            sql::plans(&target.kind, &catalog, source, converted, &package.registry)?;
            session.sql("ROLLBACK;\n".into())?;
            session.finish()?;
            identities.push(identity);
        }
        private_output(&config.output_dir)?;
        let marker = format!(
            "saas-uuid-import/1/{}/{}",
            package.converted.batch_id, package.converted_digest.sha256
        );
        let mut reports = Vec::new();
        for (target, identity) in config.databases.iter().zip(&identities) {
            let (source, converted) = database(&package, &target.kind)?;
            let mut session = session::Session::start(&config, target, &scratch)?;
            session.sql(sql::begin_sql(&target.kind)?)?;
            session.sql(fence_sql(target))?;
            let start = checked_fence(session.value()?, target, None)?;
            if start.database_oid != identity.database_oid || start.owner_oid != identity.owner_oid
            {
                return Err(Error("target identity changed after preflight"));
            }
            session.sql(metadata::query(&target.kind)?)?;
            metadata::validate(&target.kind, &session.value()?)?;
            session.sql(catalog_sql(config.timeout_seconds)?)?;
            let catalog = session.value()?;
            if schema_hash(&catalog)? != target.expected_schema_sha256 {
                return Err(Error("target schema changed after preflight"));
            }
            let plans = sql::plans(&target.kind, &catalog, source, converted, &package.registry)?;
            for plan in plans {
                let table = converted
                    .tables
                    .iter()
                    .find(|t| t.schema == plan.schema && t.table == plan.table)
                    .ok_or(Error("converted table missing"))?;
                let original = source
                    .tables
                    .iter()
                    .find(|t| t.schema == plan.schema && t.table == plan.table)
                    .ok_or(Error("source table missing"))?;
                let files = TableFiles::open(&config, &scratch, &key, original, table)?;
                session.sql(sql::stage_sql(&plan)?)?;
                let mut buffer = Vec::with_capacity(65536);
                files.visit(&package, &target.kind, table, |ordinal, row| {
                    if plan
                        .nullable_json_columns
                        .iter()
                        .any(|c| row.get(c).is_some_and(Value::is_null))
                    {
                        return Err(Error(
                            "ambiguous nullable JSON requires authenticated source null evidence",
                        ));
                    }
                    buffer.extend_from_slice(&csv(ordinal, row)?);
                    if buffer.len() >= 65536 {
                        session.send(std::mem::take(&mut buffer))?;
                    }
                    Ok(())
                })?;
                if !buffer.is_empty() {
                    session.send(buffer)?;
                }
                session.sql("\\.\n".into())?;
                session.sql(sql::insert_verify_sql(&plan)?)?;
                files.visit(&package,&target.kind,table,|ordinal,row| {
                    let actual=session.value()?;
                    if actual!=json!({"kind":"import_row","schema":plan.schema,"table":plan.table,"ordinal":ordinal,"row":row}) { return Err(Error("target row differs from its source ordinal projection proof")); }
                    Ok(())
                })?;
                if session.value()?
                    != json!({"kind":"table_verified","schema":plan.schema,"table":plan.table,"rows":plan.rows})
                {
                    return Err(Error("target table verification count mismatch"));
                }
            }
            session.sql(sql::finish_sql(&target.kind)?)?;
            event(session.value()?, "business_verified")?;
            session.sql(metadata::query(&target.kind)?)?;
            metadata::validate(&target.kind, &session.value()?)?;
            session.sql(catalog_sql(config.timeout_seconds)?)?;
            let final_catalog = session.value()?;
            if schema_hash(&final_catalog)? != target.expected_schema_sha256 {
                return Err(Error("final target schema or trigger state changed"));
            }
            final_counts(&final_catalog, converted)?;
            sequence_restore(&mut session, source, &final_catalog)?;
            session.sql(format!(
                "COMMENT ON DATABASE {} IS '{marker}';\n",
                ident(&target.name)?
            ))?;
            session.sql(fence_sql(target))?;
            let end = checked_fence(session.value()?, target, Some(&marker))?;
            if end.database_oid != start.database_oid || end.owner_oid != start.owner_oid {
                return Err(Error("target identity changed during import"));
            }
            session
                .sql("COMMIT;\nSELECT jsonb_build_object('kind','target_committed');\n".into())?;
            event(session.value()?, "target_committed")?;
            session.finish()?;
            let report = json!({"kind":target.kind,"database":target.name,"database_oid":end.database_oid,"owner_oid":end.owner_oid,"schema_sha256":target.expected_schema_sha256,"rows":converted.tables.iter().map(|t|t.rows).sum::<u64>(),"marker":marker});
            let plain = scratch.0.join(format!("committed-{}.json", target.kind));
            let mut file = new_plaintext(&plain)?;
            io(serde_json::to_writer(&mut file, &report).map_err(std::io::Error::other))?;
            io(file.sync_all())?;
            drop(file);
            let committed = config
                .output_dir
                .join(format!("committed-{}.enc", target.kind));
            let purpose = format!(
                "saas-uuid-import-commit/1/{}/{}",
                package.converted.batch_id, target.kind
            );
            let committed_digest = codec::seal_file(&plain, &committed, &key, &purpose)?;
            if codec::verify_file(&committed, &key, &purpose)? != committed_digest {
                return Err(Error(
                    "target commit evidence read-back authentication failed",
                ));
            }
            io(fs::remove_file(plain))?;
            reports.push(report);
        }
        let report = json!({"format":"saas-uuid-import-completion/1","status":"QUARANTINED_THREE_TARGET_IMPORT_VERIFIED","batch_id":package.converted.batch_id,"source_manifest_sha256":package.source_digest.sha256,"converted_manifest_sha256":package.converted_digest.sha256,"databases":reports,"services_started":false});
        let plain = scratch.0.join("import-completion.json");
        let mut file = new_plaintext(&plain)?;
        io(serde_json::to_writer(&mut file, &report).map_err(std::io::Error::other))?;
        io(file.sync_all())?;
        drop(file);
        let digest = codec::seal_file(
            &plain,
            &config.output_dir.join("complete.enc"),
            &key,
            &format!(
                "saas-uuid-import-completion/1/{}",
                package.converted.batch_id
            ),
        )?;
        io(fs::remove_file(plain))?;
        if codec::verify_file(
            &config.output_dir.join("complete.enc"),
            &key,
            &format!(
                "saas-uuid-import-completion/1/{}",
                package.converted.batch_id
            ),
        )? != digest
        {
            return Err(Error("import completion read-back authentication failed"));
        }
        Ok(json!({"status":"QUARANTINED_THREE_TARGET_IMPORT_VERIFIED","databases":3,"batch_id":package.converted.batch_id,"completion_sha256":digest.sha256,"services_started":false}).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Operator diagnostic only: writes the actual generated queries without
    /// opening PostgreSQL. The combined script forces a read-only transaction.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires UUID_IMPORT_SQL_DIR in a private tmpfs parent"]
    fn write_private_preflight_sql_for_operator() {
        use std::io::Write;
        let directory = PathBuf::from(
            std::env::var_os("UUID_IMPORT_SQL_DIR")
                .expect("explicit SQL output directory required"),
        );
        safe_path(&directory).unwrap();
        crate::native::require_tmpfs_directory(directory.parent().unwrap()).unwrap();
        private_output(&directory).unwrap();
        for (kind, name, owner) in TARGETS {
            let target = Target {
                kind: kind.into(),
                name: name.into(),
                owner: owner.into(),
                expected_schema_sha256: "a".repeat(64),
            };
            let readonly = sql::begin_sql(kind)
                .unwrap()
                .replace("READ COMMITTED READ WRITE", "READ COMMITTED READ ONLY");
            let phases = [
                ("01-begin", readonly),
                ("02-fence", fence_sql(&target)),
                ("03-metadata", metadata::query(kind).unwrap()),
                ("04-catalog", catalog_sql(1800).unwrap()),
                ("05-rollback", "ROLLBACK;\n".into()),
            ];
            let mut combined = String::from("\\set ON_ERROR_STOP on\n");
            for (phase, body) in phases {
                let mut file =
                    new_plaintext(&directory.join(format!("{kind}-{phase}.sql"))).unwrap();
                file.write_all(body.as_bytes()).unwrap();
                file.sync_all().unwrap();
                combined.push_str(&format!(
                    "\\echo UUID_IMPORT_DIAGNOSTIC_{kind}_{phase}\n{body}"
                ));
            }
            let mut file =
                new_plaintext(&directory.join(format!("{kind}-readonly-preflight.sql"))).unwrap();
            file.write_all(combined.as_bytes()).unwrap();
            file.sync_all().unwrap();
        }
        println!("PRIVATE_READ_ONLY_PREFLIGHT_SQL_WRITTEN files=18 no_database_connections=true");
    }
    fn target() -> Target {
        Target {
            kind: TARGETS[0].0.into(),
            name: TARGETS[0].1.into(),
            owner: TARGETS[0].2.into(),
            expected_schema_sha256: "a".repeat(64),
        }
    }
    #[test]
    fn owner_acl_admission_and_existing_epoch_markers_are_required_even_for_empty_business_data() {
        let target = target();
        let clean = json!({"database":target.name,"database_oid":100,"owner_oid":200,"actor_valid":true,"local":true,"transaction_valid":true,"owner_valid":true,"database_valid":true,"object_ownership_valid":true,"acl_valid":true,"bad_data_writers":0,"default_read_only":true,"other_backends":0,"prepared_transactions":0,"bad_logins":0,"comment":null});
        assert!(checked_fence(clean.clone(), &target, None).is_ok());
        for name in [
            "actor_valid",
            "local",
            "transaction_valid",
            "owner_valid",
            "database_valid",
            "object_ownership_valid",
            "acl_valid",
            "default_read_only",
        ] {
            let mut bad = clean.clone();
            bad[name] = json!(false);
            assert!(checked_fence(bad, &target, None).is_err());
        }
        for name in [
            "other_backends",
            "prepared_transactions",
            "bad_logins",
            "bad_data_writers",
        ] {
            let mut bad = clean.clone();
            bad[name] = json!(1);
            assert!(checked_fence(bad, &target, None).is_err());
        }
        let mut marked = clean.clone();
        marked["comment"] = json!("saas-uuid-import/1/already-imported");
        assert!(checked_fence(marked.clone(), &target, None).is_err());
        assert!(
            checked_fence(marked, &target, Some("saas-uuid-import/1/already-imported")).is_ok()
        );
        let mut legacy = clean;
        legacy["database_oid"] = json!("100");
        assert!(checked_fence(legacy, &target, None).is_err());
    }

    #[test]
    fn source_names_seed_mode_and_nonfixed_target_connections_are_rejected_before_opening_psql() {
        let base = std::env::temp_dir().join(format!("uuid-import-test-{}", uuid::Uuid::now_v7()));
        let value = json!({"profile":"uuid-local-preview","mode":"import","seed":false,"host":"/run/postgresql","port":5432,"user":"postgres","databases":TARGETS.iter().map(|(kind,name,owner)|json!({"kind":kind,"name":name,"owner":owner,"expected_schema_sha256":"a".repeat(64)})).collect::<Vec<_>>(),"source_dir":base.join("source"),"converted_dir":base.join("converted"),"registry":base.join("registry"),"key_file":base.join("key"),"scratch":base.join("scratch"),"output_dir":base.join("output"),"timeout_seconds":1800});
        assert!(strict::<Config>(&serde_json::to_vec(&value).unwrap())
            .unwrap()
            .validate()
            .is_ok());
        for (pointer, bad) in [
            ("/seed", json!(true)),
            ("/mode", json!("seed")),
            ("/host", json!("/var/run/postgresql")),
            ("/user", json!("another_admin")),
            ("/port", json!(5433)),
            (
                "/databases/0/name",
                json!("qiankun_platform_preview_20261024"),
            ),
            ("/databases/0/owner", json!("postgres")),
        ] {
            let mut changed = value.clone();
            *changed.pointer_mut(pointer).unwrap() = bad;
            assert!(strict::<Config>(&serde_json::to_vec(&changed).unwrap())
                .unwrap()
                .validate()
                .is_err());
        }
    }
    #[test]
    fn csv_preserves_backslashes_quotes_newlines_and_exact_numbers_without_sql_literals() {
        let row:Value=structured::parse_strict(r#"{"text":"\\.\n\"quoted\"\\u001f","amount":9007199254740993.123456789,"date":"2026-10-24"}"#).unwrap();
        let bytes = csv(1, &row).unwrap();
        assert_eq!(bytes.iter().filter(|b| **b == b'\n').count(), 1);
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("1,\"{") && text.ends_with("\"\n"));
        let restored = text
            .strip_prefix("1,\"")
            .unwrap()
            .strip_suffix("\"\n")
            .unwrap()
            .replace("\"\"", "\"");
        assert_eq!(structured::parse_strict(&restored).unwrap(), row);
        assert!(restored.contains("9007199254740993.123456789"));
        assert!(csv(0, &row).is_err());
        assert!(csv(1, &Value::Null).is_err());
    }
    #[test]
    fn missing_nullable_proof_field_duplicate_json_and_noninteger_ordinal_fail_closed() {
        assert!(strict::<RowProof>(
            br#"{"index":0,"policy":"RETAIN_REMAP","source_sha256":"x","business_proof":null}"#
        )
        .is_err());
        assert!(strict::<Value>(br#"{"outer":{"a":1,"a":2}}"#).is_err());
        assert!(strict::<RowProof>(br#"{"index":0.1,"policy":"RETAIN_REMAP","source_sha256":"x","target_sha256":null,"business_proof":null}"#).is_err());
    }
}
