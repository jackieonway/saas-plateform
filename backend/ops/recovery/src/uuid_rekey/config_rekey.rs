//! Closed, offline configuration owners. Only registered typed UUID fields are
//! rekeyed; old preview state is retired without publishing identity values.
#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

#[path = "config_grants.rs"]
mod grants;
#[path = "config_policy.rs"]
mod policy;
#[path = "config_state.rs"]
mod state;

use super::{codec, mapping::TypedMap, structured, verify};
use crate::{bundle, io, native, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Component, Path, PathBuf},
};
use uuid::{Uuid, Variant};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    profile: String,
    source_dir: PathBuf,
    converted_dir: PathBuf,
    scratch: PathBuf,
    registry: PathBuf,
    key_file: PathBuf,
    output_dir: PathBuf,
    inputs: Vec<Input>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    owner: String,
    path: PathBuf,
}

#[derive(Clone, Copy)]
struct Field {
    path: &'static str,
    authority: &'static str,
    nullable: bool,
}

macro_rules! field {
    ($path:literal, $authority:literal) => {
        Field {
            path: $path,
            authority: $authority,
            nullable: false,
        }
    };
    ($path:literal, $authority:literal, nullable) => {
        Field {
            path: $path,
            authority: $authority,
            nullable: true,
        }
    };
}
const POLICY: &[Field] = &[
    field!("/applications/*/applicationId", "platform.application"),
    field!("/managementScopes/*/tenantId", "platform.tenant"),
    field!(
        "/managementScopes/*/organizationId",
        "platform.organization"
    ),
];
const GRANTS: &[Field] = &[
    field!("/applicationId", "platform.application"),
    field!("/menuIds/*", "platform.menu"),
    field!("/targets/*/tenantId", "platform.tenant"),
    field!("/targets/*/organizationId", "platform.organization"),
    field!("/targets/*/roleId", "platform.role"),
    field!("/targets/*/userId", "platform.user"),
    field!("/targets/*/membershipId", "platform.membership"),
];
const STATE: &[Field] = &[
    field!("/applications/*/id", "platform.application"),
    field!("/applications/*/menuIds/*", "platform.menu"),
    field!("/clients/*/id", "platform.oauth_client"),
    field!("/clients/*/applicationId", "platform.application", nullable),
    field!("/adminId", "platform.user"),
    field!("/businessTenant/id", "platform.tenant"),
    field!("/businessOrganization/id", "platform.organization"),
    field!("/businessOrganization/tenantId", "platform.tenant"),
    field!("/storeOrganization/id", "platform.organization"),
    field!(
        "/storeOrganization/parentId",
        "platform.organization",
        nullable
    ),
    field!("/storeOrganization/tenantId", "platform.tenant"),
    field!("/storeOrganization/administrator/userId", "platform.user"),
    field!("/membership/membershipId", "platform.membership"),
    field!("/membership/organizationId", "platform.organization"),
    field!("/membership/tenantId", "platform.tenant"),
    field!("/role/id", "platform.role"),
    field!("/role/organizationId", "platform.organization"),
    field!("/role/tenantId", "platform.tenant"),
    field!("/storeRole/id", "platform.role"),
    field!("/storeRole/organizationId", "platform.organization"),
    field!("/storeRole/tenantId", "platform.tenant"),
    field!("/platformSession/user/id", "platform.user"),
    field!("/platformSession/tenant/id", "platform.tenant"),
    field!("/platformSession/organization/id", "platform.organization"),
    field!("/businessSession/user/id", "platform.user"),
    field!("/businessSession/tenant/id", "platform.tenant"),
    field!("/businessSession/organization/id", "platform.organization"),
    field!("/membership/departmentId", "platform.department", nullable),
    field!(
        "/businessOrganization/parentId",
        "platform.organization",
        nullable
    ),
];

fn contract(owner: &str) -> Result<(&'static [Field], &'static str)> {
    match owner {
        "data-grant-policy" => Ok((
            POLICY,
            "private_field_level_rewrite_then_validate_closed_policy_no_values_in_reports",
        )),
        "platform-admin-grants" => Ok((
            GRANTS,
            "rewrite_actual_operator_files_preserve_placeholder_examples",
        )),
        "preview-private-state" => Ok((STATE, "private_inventory_then_regenerate_target_state")),
        _ => Err(Error("unsupported closed configuration owner")),
    }
}

fn registered(registry: &Value, owner: &str) -> Result<&'static [Field]> {
    let (fields, operation) = contract(owner)?;
    let owners = registry["structuredReferences"]
        .as_array()
        .ok_or(Error("configuration registry required"))?;
    let matching: Vec<_> = owners.iter().filter(|row| row["id"] == owner).collect();
    let row = matching
        .first()
        .filter(|_| matching.len() == 1)
        .ok_or(Error("unique registered configuration owner required"))?;
    let paths = row["paths"]
        .as_array()
        .ok_or(Error("configuration paths required"))?;
    if row["format"] != "json"
        || row["unknownUuidPath"] != "block"
        || row["operation"] != operation
        || !row["location"]["configuration"].is_string()
        || paths.len() != fields.len()
    {
        return Err(Error("closed configuration registry contract mismatch"));
    }
    let mut seen = BTreeSet::new();
    for path in paths {
        let value = path["path"]
            .as_str()
            .ok_or(Error("registered configuration path required"))?;
        let field = fields
            .iter()
            .find(|field| field.path == value)
            .ok_or(Error("unregistered closed configuration path"))?;
        if !seen.insert(value)
            || path["authority"] != field.authority
            || path["nullable"] != field.nullable
            || path["cardinality"] != if value.contains('*') { "many" } else { "one" }
        {
            return Err(Error("configuration path authority contract mismatch"));
        }
    }
    Ok(fields)
}

pub(super) fn parse_id(value: &str, target: bool) -> Result<Uuid> {
    let id = Uuid::parse_str(value).map_err(|_| Error("configuration UUID required"))?;
    if id.to_string() != value
        || id.is_nil()
        || id.get_variant() != Variant::RFC4122
        || !matches!(id.get_version_num(), 4 | 7)
        || target && id.get_version_num() != 7
    {
        return Err(Error("canonical configuration UUID version required"));
    }
    Ok(id)
}

type Counts = BTreeMap<String, usize>;
type Locations = BTreeSet<Vec<String>>;

fn rewrite_field(
    value: &mut Value,
    tokens: &[&str],
    location: &mut Vec<String>,
    field: Field,
    mapping: &TypedMap,
    counts: &mut Counts,
    allowed: &mut Locations,
) -> Result<()> {
    if tokens.is_empty() {
        allowed.insert(location.clone());
        if value.is_null() && field.nullable {
            return Ok(());
        }
        let old = parse_id(
            value
                .as_str()
                .ok_or(Error("typed configuration UUID string required"))?,
            false,
        )?;
        let new = mapping.resolve(field.authority, old)?;
        *value = Value::String(new.to_string());
        *counts.entry(field.authority.to_owned()).or_default() += 1;
        return Ok(());
    }
    let token = tokens[0];
    if token == "*" {
        match value {
            Value::Array(rows) => {
                for (index, row) in rows.iter_mut().enumerate() {
                    location.push(index.to_string());
                    rewrite_field(row, &tokens[1..], location, field, mapping, counts, allowed)?;
                    location.pop();
                }
            }
            Value::Object(rows) => {
                for (key, row) in rows.iter_mut() {
                    location.push(key.clone());
                    rewrite_field(row, &tokens[1..], location, field, mapping, counts, allowed)?;
                    location.pop();
                }
            }
            _ => {
                return Err(Error(
                    "registered configuration wildcard container required",
                ))
            }
        }
    } else if let Some(row) = value.as_object_mut().and_then(|rows| rows.get_mut(token)) {
        location.push(token.to_owned());
        rewrite_field(row, &tokens[1..], location, field, mapping, counts, allowed)?;
        location.pop();
    }
    Ok(())
}

fn reject_unknown_uuid(
    value: &Value,
    path: &mut Vec<String>,
    allowed: &Locations,
    state: bool,
) -> Result<()> {
    match value {
        Value::String(text) if Uuid::parse_str(text).is_ok() => {
            let opaque = state
                && (path.len() == 3 && path[0] == "clients" && path[2] == "clientId"
                    || path.len() == 3 && path[0] == "applications" && path[2] == "code"
                    || path.len() == 2
                        && [
                            "businessTenant",
                            "businessOrganization",
                            "storeOrganization",
                            "role",
                            "storeRole",
                        ]
                        .contains(&path[0].as_str())
                        && path[1] == "code"
                    || path.len() == 5
                        && path[0] == "applications"
                        && path[2] == "definition"
                        && path[3] == "application"
                        && path[4] == "code"
                    || path.len() == 6
                        && path[0] == "applications"
                        && path[2] == "definition"
                        && path[3] == "menus"
                        && ["key", "parentKey"].contains(&path[5].as_str()));
            if !allowed.contains(path) && !opaque {
                return Err(Error("UUID at unregistered configuration path"));
            }
        }
        Value::Array(rows) => {
            for (index, row) in rows.iter().enumerate() {
                path.push(index.to_string());
                reject_unknown_uuid(row, path, allowed, state)?;
                path.pop();
            }
        }
        Value::Object(rows) => {
            for (key, row) in rows {
                path.push(key.clone());
                reject_unknown_uuid(row, path, allowed, state)?;
                path.pop();
            }
        }
        _ => {}
    }
    Ok(())
}

fn rekey(
    owner: &str,
    mut value: Value,
    fields: &[Field],
    mapping: &TypedMap,
) -> Result<(Value, Counts)> {
    if owner == "preview-private-state" {
        state::validate_and_discard(&mut value)?;
    } else if owner == "data-grant-policy" {
        policy::validate(&value, false)?;
    } else {
        grants::validate(&value, false)?;
    }
    let mut counts = Counts::new();
    let mut allowed = Locations::new();
    for field in fields {
        rewrite_field(
            &mut value,
            &field.path[1..].split('/').collect::<Vec<_>>(),
            &mut Vec::new(),
            *field,
            mapping,
            &mut counts,
            &mut allowed,
        )?;
    }
    reject_unknown_uuid(
        &value,
        &mut Vec::new(),
        &allowed,
        owner == "preview-private-state",
    )?;
    if owner == "preview-private-state" {
        // Never publish even sanitized source state: target IDs and stage flags
        // must be freshly queried and verified by the target runtime owner.
        value = json!({"status":"REGENERATION_REQUIRED", "reusable":false,
            "typed_reference_counts":counts, "fresh_login_required":true,
            "confidential_client_secret_rotation_required":true,
            "initial_password_discarded":true,
            "discarded_whole":["platformSession","businessSession","applicationTokens"],
            "recompute":["fingerprint"],
            "reverify":["prepared","grantsApplied","storeAccessConfigured"],
            "target_identity_source":"fresh authenticated target API query"});
    } else if owner == "data-grant-policy" {
        policy::validate(&value, true)?;
    } else {
        grants::validate(&value, true)?;
    }
    Ok((value, counts))
}

fn safe_path(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        return Err(Error("absolute normalized configuration path required"));
    }
    for parent in path.ancestors() {
        match fs::symlink_metadata(parent) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Error("configuration symlink refused"))
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Error("configuration path inaccessible")),
        }
    }
    Ok(())
}

fn private_directory(path: &Path) -> Result<()> {
    safe_path(path)?;
    let metadata = io(fs::symlink_metadata(path))?;
    if !metadata.is_dir() {
        return Err(Error("private configuration directory required"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o7777 != 0o700
            || metadata.uid() != io(fs::metadata("/proc/self"))?.uid()
        {
            return Err(Error("configuration directory must be owned mode 0700"));
        }
    }
    Ok(())
}

fn private_bytes(path: &Path, limit: u64) -> Result<Vec<u8>> {
    safe_path(path)?;
    let metadata = io(fs::symlink_metadata(path))?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(Error("bounded private configuration file required"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o7777 != 0o600
            || metadata.uid() != io(fs::metadata("/proc/self"))?.uid()
            || metadata.nlink() != 1
        {
            return Err(Error(
                "configuration file must be singly linked owned mode 0600",
            ));
        }
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // O_NOFOLLOW rejects a swapped final symlink; O_NONBLOCK avoids
        // blocking if a regular input is swapped for a FIFO before open.
        options.custom_flags(0x20000 | 0x800);
    }
    let file = io(options.open(path))?;
    let opened = io(file.metadata())?;
    if !opened.is_file() || opened.len() > limit {
        return Err(Error("opened bounded private configuration file required"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.dev() != metadata.dev()
            || opened.ino() != metadata.ino()
            || opened.mode() & 0o7777 != 0o600
            || opened.uid() != metadata.uid()
            || opened.nlink() != 1
        {
            return Err(Error("private configuration file changed during open"));
        }
    }
    let mut bytes = Vec::new();
    io(file.take(limit + 1).read_to_end(&mut bytes))?;
    if bytes.len() as u64 > limit {
        return Err(Error("configuration file limit exceeded"));
    }
    Ok(bytes)
}

fn parse(bytes: &[u8]) -> Result<Value> {
    structured::parse_strict(
        std::str::from_utf8(bytes).map_err(|_| Error("configuration UTF-8 required"))?,
    )
}

#[derive(Serialize)]
struct OutputEvidence {
    owner: String,
    file: String,
    source_sha256: String,
    target_sha256: String,
    source_bytes: usize,
    target_bytes: usize,
    typed_reference_counts: Counts,
    reusable: bool,
}

pub(crate) fn execute(path: &Path) -> Result<String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        Err(Error("configuration rekey requires Linux private tmpfs"))
    }
    #[cfg(target_os = "linux")]
    {
        let config: Config = serde_json::from_value(parse(&private_bytes(path, 1024 * 1024)?)?)
            .map_err(|_| Error("invalid closed configuration rekey contract"))?;
        if config.profile != "uuid-local-preview"
            || config.inputs.is_empty()
            || config.inputs.len() > 3
        {
            return Err(Error(
                "local preview and one to three closed owners required",
            ));
        }
        for candidate in [
            &config.source_dir,
            &config.converted_dir,
            &config.scratch,
            &config.registry,
            &config.key_file,
            &config.output_dir,
        ] {
            safe_path(candidate)?;
        }
        let parent = config
            .output_dir
            .parent()
            .ok_or(Error("configuration output parent required"))?;
        private_directory(parent)?;
        if config.output_dir.exists() {
            return Err(Error("new configuration output directory required"));
        }
        let mut owners = BTreeSet::new();
        let mut inputs = BTreeSet::new();
        for input in &config.inputs {
            contract(&input.owner)?;
            if !owners.insert(&input.owner) || !inputs.insert(&input.path) {
                return Err(Error("one distinct input per configuration owner required"));
            }
            safe_path(&input.path)?;
        }
        for protected in [
            config.source_dir.as_path(),
            config.converted_dir.as_path(),
            config.scratch.as_path(),
            config.key_file.as_path(),
            config.registry.as_path(),
            path,
        ]
        .into_iter()
        .chain(config.inputs.iter().map(|input| input.path.as_path()))
        {
            if config.output_dir.starts_with(protected) || protected.starts_with(&config.output_dir)
            {
                return Err(Error("separate configuration output required"));
            }
        }
        if config.key_file.starts_with(&config.source_dir)
            || config.key_file.starts_with(&config.converted_dir)
        {
            return Err(Error("separate configuration authentication key required"));
        }
        let key = private_bytes(&config.key_file, 32)?;
        if key.len() != 32 {
            return Err(Error(
                "configuration authentication key must contain 32 bytes",
            ));
        }
        let scratch = native::Scratch::new(&config.scratch)?;
        let package = verify::load_verified(
            &config.source_dir,
            &config.converted_dir,
            &scratch,
            &config.registry,
            &key,
        )?;
        let mut outputs = Vec::new();
        let mut evidence = Vec::new();
        for input in &config.inputs {
            let fields = registered(&package.registry, &input.owner)?;
            let limit = if input.owner == "data-grant-policy" {
                4 * 1024 * 1024
            } else if input.owner == "preview-private-state" {
                16 * 1024 * 1024
            } else {
                1024 * 1024
            };
            let original = private_bytes(&input.path, limit)?;
            let (target, counts) =
                rekey(&input.owner, parse(&original)?, fields, &package.mapping)?;
            let mut bytes = serde_json::to_vec_pretty(&target)
                .map_err(|_| Error("configuration serialization failed"))?;
            bytes.push(b'\n');
            let filename = format!("{}.json", input.owner);
            evidence.push(OutputEvidence {
                owner: input.owner.clone(),
                file: filename.clone(),
                source_sha256: bundle::hash(&original),
                target_sha256: bundle::hash(&bytes),
                source_bytes: original.len(),
                target_bytes: bytes.len(),
                typed_reference_counts: counts,
                reusable: input.owner != "preview-private-state",
            });
            outputs.push((filename, bytes));
        }
        let summary = json!({"format":"saas-uuid-config-rekey/1", "status":"CONFIGURATION_REKEY_COMPLETE",
            "batch_id":package.source.batch_id, "source_manifest_sha256":package.source_digest.sha256,
            "converted_manifest_sha256":package.converted_digest.sha256,
            "mapping_sha256":package.converted.mapping_digest.sha256,
            "registry_sha256":package.source.registry_sha256, "outputs":evidence,
            "fresh_login_required":true, "old_signed_credentials_reusable":false});
        let manifest = serde_json::to_vec_pretty(&summary)
            .map_err(|_| Error("configuration evidence serialization failed"))?;
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        io(builder.create(&config.output_dir))?;
        private_directory(&config.output_dir)?;
        for (filename, bytes) in outputs {
            let target = config.output_dir.join(filename);
            native::create_private(&target, &bytes)?;
            if private_bytes(&target, bytes.len() as u64)? != bytes {
                return Err(Error("configuration output read-back mismatch"));
            }
        }
        let plain = scratch.0.join("config-manifest.json");
        native::create_private(&plain, &manifest)?;
        let purpose = format!(
            "saas-uuid-config-rekey-manifest/1/{}",
            package.source.batch_id
        );
        let sealed = config.output_dir.join("manifest.uuid7");
        let digest = codec::seal_file(&plain, &sealed, &key, &purpose)?;
        if codec::verify_file(&sealed, &key, &purpose)? != digest {
            return Err(Error("configuration manifest read-back mismatch"));
        }
        io(fs::remove_file(&plain))?;
        native::create_private(&config.output_dir.join("summary.json"), &manifest)?;
        io(File::open(&config.output_dir))?
            .sync_all()
            .map_err(|_| Error("configuration directory sync failed"))?;
        Ok(
            json!({"status":"CONFIGURATION_REKEY_COMPLETE", "manifest":digest, "evidence":summary})
                .to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn old(n: u128) -> Uuid {
        Uuid::from_u128(0x00000000000040008000000000000000 | n)
    }
    fn map() -> TypedMap {
        let mut map = TypedMap::new(
            1_720_000_000_000,
            BTreeSet::from([
                "platform.application".into(),
                "platform.tenant".into(),
                "platform.organization".into(),
                "platform.oauth_client".into(),
                "platform.menu".into(),
            ]),
        )
        .unwrap();
        for authority in [
            "platform.application",
            "platform.tenant",
            "platform.organization",
            "platform.oauth_client",
            "platform.menu",
        ] {
            map.allocate(authority, old(1), None).unwrap();
        }
        map
    }
    #[test]
    fn field_wildcards_visit_map_values_preserve_logical_keys_and_use_authority() {
        let mapping = map();
        let mut value = json!({"items": {old(1).to_string(): {"id":old(1),"label":"keep"}}});
        let field = field!("/items/*/id", "platform.menu");
        let mut counts = Counts::new();
        let mut locations = Locations::new();
        rewrite_field(
            &mut value,
            &["items", "*", "id"],
            &mut vec![],
            field,
            &mapping,
            &mut counts,
            &mut locations,
        )
        .unwrap();
        let key = old(1).to_string();
        assert_eq!(
            value["items"][&key]["id"],
            mapping
                .resolve("platform.menu", old(1))
                .unwrap()
                .to_string()
        );
        assert_ne!(
            value["items"][&key]["id"],
            mapping
                .resolve("platform.application", old(1))
                .unwrap()
                .to_string()
        );
        assert_eq!(value["items"][&key]["label"], "keep");
        assert_eq!(counts["platform.menu"], 1);
    }
    #[test]
    fn unknown_uuid_values_and_missing_authority_mapping_are_blocked() {
        assert!(reject_unknown_uuid(
            &json!({"unexpected":old(1)}),
            &mut vec![],
            &Locations::new(),
            false
        )
        .is_err());
        let mut value = json!({"id":old(2)});
        assert!(rewrite_field(
            &mut value,
            &["id"],
            &mut vec![],
            field!("/id", "platform.menu"),
            &map(),
            &mut Counts::new(),
            &mut Locations::new()
        )
        .is_err());
    }
    #[test]
    fn policy_rekeys_only_registered_fields_and_preserves_every_other_value() {
        let value = json!({"applications":[{"applicationId":old(1),"capabilities":[{"resource":"ticket","action":"read","scopes":["ORG","SELF"]}],"memberLookups":[{"resource":"ticket","action":"read"}]}],
            "managementScopes":[{"tenantId":old(1),"organizationId":old(1),"includeDescendants":true,"read":true,"write":false}]});
        let mapping = map();
        let (target, counts) = rekey("data-grant-policy", value.clone(), POLICY, &mapping).unwrap();
        assert_eq!(
            target["applications"][0]["applicationId"],
            mapping
                .resolve("platform.application", old(1))
                .unwrap()
                .to_string()
        );
        assert_eq!(
            target["managementScopes"][0]["tenantId"],
            mapping
                .resolve("platform.tenant", old(1))
                .unwrap()
                .to_string()
        );
        assert_eq!(
            target["managementScopes"][0]["organizationId"],
            mapping
                .resolve("platform.organization", old(1))
                .unwrap()
                .to_string()
        );
        assert_eq!(
            target["applications"][0]["capabilities"],
            value["applications"][0]["capabilities"]
        );
        assert_eq!(
            target["applications"][0]["memberLookups"],
            value["applications"][0]["memberLookups"]
        );
        assert_eq!(counts.values().sum::<usize>(), 3);
    }
    #[test]
    fn complete_grant_intent_remaps_typed_values_and_preserves_keys_and_permissions() {
        let authorities = BTreeSet::from([
            "platform.application".into(),
            "platform.menu".into(),
            "platform.tenant".into(),
            "platform.organization".into(),
            "platform.role".into(),
            "platform.user".into(),
            "platform.membership".into(),
        ]);
        let mut mapping = TypedMap::new(1_720_000_000_000, authorities).unwrap();
        for authority in [
            "platform.application",
            "platform.tenant",
            "platform.organization",
            "platform.role",
            "platform.user",
            "platform.membership",
        ] {
            mapping.allocate(authority, old(1), None).unwrap();
        }
        let mut menus = serde_json::Map::new();
        for (index, key) in [
            "root",
            "ticket:read",
            "ticket:create",
            "ticket:update",
            "ticket:delete",
            "ticket:export",
            "notification:publish",
        ]
        .into_iter()
        .enumerate()
        {
            let id = old(index as u128 + 10);
            mapping.allocate("platform.menu", id, None).unwrap();
            menus.insert(key.into(), json!(id));
        }
        let source = json!({"schemaVersion":"1.0","applicationCode":"work-orders","applicationId":old(1),"menuIds":menus,
            "targets":[{"tenantId":old(1),"organizationId":old(1),"roleId":old(1),"userId":old(1),"membershipId":old(1),
                "permissions":["ticket:read"],"dataGrants":[{"resource":"ticket","action":"read","scope":"ORG"}]}]});
        let (target, counts) =
            rekey("platform-admin-grants", source.clone(), GRANTS, &mapping).unwrap();
        grants::validate(&target, true).unwrap();
        assert_eq!(
            source["menuIds"]
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>(),
            target["menuIds"]
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>()
        );
        assert_eq!(
            target["targets"][0]["permissions"],
            source["targets"][0]["permissions"]
        );
        assert_eq!(
            target["targets"][0]["dataGrants"],
            source["targets"][0]["dataGrants"]
        );
        assert_eq!(
            target["menuIds"]["root"],
            mapping
                .resolve("platform.menu", old(10))
                .unwrap()
                .to_string()
        );
        assert_eq!(counts.values().sum::<usize>(), 13);
    }
    #[test]
    fn preview_state_emits_only_regeneration_counts_and_never_identities_or_secrets() {
        let identity = old(1).to_string();
        let source = json!({"origin":"http://127.0.0.1:3001","adminId":old(1),"applications":{"logical":{"id":old(1),"code":identity,"menuIds":{"root":old(1)}}},
            "clients":{"opaque":{"id":old(1),"clientId":identity,"applicationId":old(1),"clientSecret":"secret-marker"}},
            "platformSession":{"accessToken":"token-marker"}});
        let mapping = map();
        // The test mapping deliberately lacks platform.user: all surviving
        // typed references still require source closure even for retirement.
        assert!(rekey("preview-private-state", source.clone(), STATE, &mapping).is_err());
        let mut source = source;
        source.as_object_mut().unwrap().remove("adminId");
        let (target, counts) = rekey("preview-private-state", source, STATE, &mapping).unwrap();
        assert_eq!(target["status"], "REGENERATION_REQUIRED");
        assert_eq!(counts.values().sum::<usize>(), 4);
        let output = target.to_string();
        for marker in [
            &identity,
            "secret-marker",
            "token-marker",
            "logical",
            "opaque",
        ] {
            assert!(!output.contains(marker));
        }
    }
    #[test]
    fn registered_owner_contract_is_finite_and_authority_bound() {
        let mut registry: Value =
            serde_json::from_str(include_str!("../../uuid-references-v2.json")).unwrap();
        assert_eq!(registered(&registry, "data-grant-policy").unwrap().len(), 3);
        assert_eq!(
            registered(&registry, "platform-admin-grants")
                .unwrap()
                .len(),
            7
        );
        assert_eq!(
            registered(&registry, "preview-private-state")
                .unwrap()
                .len(),
            29
        );
        assert!(registered(&registry, "bootstrap-output").is_err());
        let row = registry["structuredReferences"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| row["id"] == "data-grant-policy")
            .unwrap();
        row["paths"][0]["authority"] = json!("platform.user");
        assert!(registered(&registry, "data-grant-policy").is_err());
    }
    #[test]
    fn duplicate_json_and_noncanonical_ids_are_rejected_without_values() {
        assert!(parse(br#"{"applications":[],"applications":[]}"#).is_err());
        assert!(parse_id("00000000-0000-4000-8000-000000000001", false).is_ok());
        assert!(parse_id("00000000-0000-4000-8000-000000000001", true).is_err());
        assert!(parse_id("00000000-0000-7000-c000-000000000001", true).is_err());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn private_inputs_reject_permissions_symlinks_hardlinks_and_oversize() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let directory = std::env::temp_dir().join(format!("saas-config-test-{}", Uuid::now_v7()));
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let guard = native::Scratch(directory);
        private_directory(&guard.0).unwrap();
        let input = guard.0.join("input.json");
        native::create_private(&input, b"{}").unwrap();
        assert_eq!(private_bytes(&input, 2).unwrap(), b"{}");
        assert!(private_bytes(&input, 1).is_err());
        fs::set_permissions(&input, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(private_bytes(&input, 2).is_err());
        fs::set_permissions(&input, fs::Permissions::from_mode(0o600)).unwrap();
        let link = guard.0.join("symbolic.json");
        symlink(&input, &link).unwrap();
        assert!(private_bytes(&link, 2).is_err());
        let alias = guard.0.join("hard.json");
        fs::hard_link(&input, &alias).unwrap();
        assert!(private_bytes(&input, 2).is_err());
        assert!(private_bytes(&alias, 2).is_err());
    }
}
