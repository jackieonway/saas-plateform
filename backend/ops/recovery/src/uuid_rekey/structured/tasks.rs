//! Closed historical task payloads. Receipts remain original archived evidence;
//! changing a correlation key would manufacture a new authenticated completion.
//! Nonempty task-effect tables require explicit reconciliation before import.
use super::super::mapping::TypedMap;
use crate::{Error, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use uuid::Uuid;

const BAD: &str = "task payload does not match a registered source schema";

fn decode<T: DeserializeOwned>(value: &Value) -> Result<T> {
    serde_json::from_value(value.clone()).map_err(|_| Error(BAD))
}

fn object<'a>(value: &'a Value, keys: &[&str]) -> Result<&'a serde_json::Map<String, Value>> {
    let item = value.as_object().ok_or(Error(BAD))?;
    if item.keys().any(|key| !keys.contains(&key.as_str())) {
        return Err(Error(BAD));
    }
    Ok(item)
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value.get(key).and_then(Value::as_str).ok_or(Error(BAD))
}

fn integer(value: &Value, key: &str) -> Result<u64> {
    value.get(key).and_then(Value::as_u64).ok_or(Error(BAD))
}

fn boolean(value: &Value, key: &str) -> Result<()> {
    value.get(key).and_then(Value::as_bool).ok_or(Error(BAD))?;
    Ok(())
}

fn nullable_text(value: &Value, key: &str) -> Result<()> {
    let field = value.get(key).ok_or(Error(BAD))?;
    if !field.is_null() && !field.is_string() {
        return Err(Error(BAD));
    }
    Ok(())
}

fn source_uuid(raw: &str) -> Result<Uuid> {
    let id = Uuid::parse_str(raw).map_err(|_| Error(BAD))?;
    if raw.len() != 36 || id.is_nil() || id.get_variant() != uuid::Variant::RFC4122 {
        return Err(Error(BAD));
    }
    Ok(id)
}

fn digest(raw: &str) -> bool {
    raw.len() == 64
        && raw
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn hash(bytes: &[u8]) -> String {
    crate::bundle::hex(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref())
}

fn segment(raw: &str, maximum: usize) -> bool {
    !raw.is_empty()
        && raw.len() <= maximum
        && !raw.contains("..")
        && raw.as_bytes()[0].is_ascii_alphanumeric()
        && raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn https_origin(raw: &str) -> bool {
    reqwest::Url::parse(raw)
        .is_ok_and(|url| url.scheme() == "https" && url.origin().ascii_serialization() == raw)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManagedReceipt {
    nonce: String,
    attempt: i32,
    state: String,
    image_id: Option<String>,
    container_name: String,
    route_sha256: Option<String>,
    error_code: Option<String>,
}

fn managed_receipt(value: &Value) -> Result<ManagedReceipt> {
    // Option fields are serialized explicitly by the source owner. Missing
    // fields are not another schema revision merely because serde accepts them.
    for key in ["imageId", "routeSha256", "errorCode"] {
        nullable_text(value, key)?;
    }
    let receipt: ManagedReceipt = decode(value)?;
    source_uuid(&receipt.nonce)?;
    if !(1..=1000).contains(&receipt.attempt) || !segment(&receipt.container_name, 128) {
        return Err(Error(BAD));
    }
    match receipt.state.as_str() {
        "READY" => {
            if !receipt
                .image_id
                .as_deref()
                .is_some_and(|id| id.strip_prefix("sha256:").is_some_and(digest))
                || !receipt.route_sha256.as_deref().is_some_and(digest)
                || receipt.error_code.is_some()
            {
                return Err(Error(BAD));
            }
        }
        "CLEANED" => {
            if receipt.image_id.is_some()
                || receipt.route_sha256.is_some()
                || receipt.error_code.is_some()
            {
                return Err(Error(BAD));
            }
        }
        "FAILED" => {
            if receipt.image_id.is_some()
                || receipt.route_sha256.is_some()
                || !receipt.error_code.as_deref().is_some_and(managed_error)
            {
                return Err(Error(BAD));
            }
        }
        _ => return Err(Error(BAD)),
    }
    Ok(receipt)
}

fn managed_error(raw: &str) -> bool {
    [
        "POLICY_REJECTED",
        "ENGINE_FAILED",
        "IMAGE_UNAVAILABLE",
        "UNHEALTHY",
        "ROUTE_FAILED",
        "OWNERSHIP_CONFLICT",
        "CLEANUP_FAILED",
        "ARTIFACT_MISSING",
    ]
    .contains(&raw)
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArtifactFile {
    path: String,
    size: u64,
    sha256: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    entry_type: String,
    entry: String,
    files: Vec<ArtifactFile>,
    artifact_sha256: String,
}

fn file_path(raw: &str) -> bool {
    if raw.is_empty()
        || raw.len() > 500
        || raw.starts_with('/')
        || raw.contains(['\\', ':', '?', '#'])
        || raw.chars().any(char::is_control)
        || raw.eq_ignore_ascii_case("release.json")
    {
        return false;
    }
    raw.split('/').all(|part| {
        let lower = part.to_ascii_lowercase();
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        !part.is_empty()
            && part != "."
            && part != ".."
            && part.len() <= 120
            && !part.ends_with(['.', ' '])
            && !lower.starts_with(".env")
            && ![
                ".git",
                ".npmrc",
                ".yarnrc",
                ".yarnrc.yml",
                ".artifact-manifest.json",
            ]
            .contains(&lower.as_str())
            && !["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
            && !(stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    })
}

fn manifest(value: &Value, html_only: bool) -> Result<Manifest> {
    let manifest: Manifest = decode(value)?;
    let entry = match manifest.entry_type.as_str() {
        "HTML" => "index.html",
        "ESM" if !html_only => "entry.mjs",
        _ => return Err(Error(BAD)),
    };
    if manifest.schema_version != 1
        || manifest.entry != entry
        || manifest.files.is_empty()
        || manifest.files.len() > 49
        || !digest(&manifest.artifact_sha256)
    {
        return Err(Error(BAD));
    }
    let mut names = BTreeSet::new();
    let mut previous = "";
    let mut total = 0u64;
    for file in &manifest.files {
        total = total.checked_add(file.size).ok_or(Error(BAD))?;
        if !file_path(&file.path)
            || file.path.as_str() <= previous
            || !names.insert(file.path.to_ascii_lowercase())
            || file.size == 0
            || file.size > 4 * 1024 * 1024
            || total > 16 * 1024 * 1024
            || !digest(&file.sha256)
        {
            return Err(Error(BAD));
        }
        previous = &file.path;
    }
    if !manifest.files.iter().any(|file| file.path == entry) {
        return Err(Error(BAD));
    }
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Canonical<'a> {
        schema_version: u32,
        entry_type: &'a str,
        entry: &'a str,
        files: &'a [ArtifactFile],
    }
    let bytes = serde_json::to_vec(&Canonical {
        schema_version: manifest.schema_version,
        entry_type: &manifest.entry_type,
        entry: &manifest.entry,
        files: &manifest.files,
    })
    .map_err(|_| Error(BAD))?;
    if hash(&bytes) != manifest.artifact_sha256 {
        return Err(Error(BAD));
    }
    Ok(manifest)
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Node {
    id: String,
    origin: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Group {
    schema_version: u32,
    id: String,
    name: String,
    public_origin: String,
    applications: Vec<String>,
    nodes: Vec<Node>,
}

fn policy(value: &Value) -> Result<Group> {
    let policy: Group = decode(value)?;
    if policy.schema_version != 1
        || !segment(&policy.id, 24)
        || policy.name.is_empty()
        || policy.name.len() > 80
        || !https_origin(&policy.public_origin)
        || !(2..=8).contains(&policy.nodes.len())
        || !(1..=8).contains(&policy.applications.len())
    {
        return Err(Error(BAD));
    }
    let mut apps = BTreeSet::new();
    let mut nodes = BTreeSet::new();
    let mut origins = BTreeSet::new();
    if policy.applications.iter().any(|app| {
        !(2..=32).contains(&app.len())
            || !app.as_bytes()[0].is_ascii_alphanumeric()
            || !app
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || !apps.insert(app)
    }) || policy.nodes.iter().any(|node| {
        !segment(&node.id, 24)
            || !https_origin(&node.origin)
            || !nodes.insert(&node.id)
            || !origins.insert(&node.origin)
    }) {
        return Err(Error(BAD));
    }
    Ok(policy)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ZipWork {
    schema_version: u32,
    job_id: String,
    attempt: i32,
    nonce: String,
    group_id: String,
    policy_revision: String,
    node_id: String,
    application_code: String,
    version: String,
    artifact_sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ZipReceipt {
    schema_version: u32,
    work: ZipWork,
    operation: String,
    manifest_sha256: String,
    file_count: usize,
    total_bytes: u64,
    reused: bool,
}

fn zip_receipt(value: &Value) -> Result<ZipReceipt> {
    let receipt: ZipReceipt = decode(value)?;
    let work = &receipt.work;
    source_uuid(&work.job_id)?;
    source_uuid(&work.nonce)?;
    if receipt.schema_version != 1
        || work.schema_version != 1
        || !(1..=1000).contains(&work.attempt)
        || !segment(&work.group_id, 24)
        || !segment(&work.node_id, 24)
        || !segment(&work.application_code, 48)
        || !segment(&work.version, 64)
        || !digest(&work.policy_revision)
        || !digest(&work.artifact_sha256)
        || !["INSTALL", "VERIFY"].contains(&receipt.operation.as_str())
        || !digest(&receipt.manifest_sha256)
        || !(1..=49).contains(&receipt.file_count)
        || !(1..=16 * 1024 * 1024).contains(&receipt.total_bytes)
        || (receipt.operation == "VERIFY" && !receipt.reused)
    {
        return Err(Error(BAD));
    }
    Ok(receipt)
}

fn row_reference(
    row: &Value,
    column: &str,
    source: &str,
    authority: &str,
    map: &TypedMap,
) -> Result<()> {
    let source = source_uuid(source)?;
    let target = map.resolve(authority, source)?;
    let actual = source_uuid(text(row, column)?)?;
    if actual != source && actual != target {
        return Err(Error("archived receipt does not match its owning row"));
    }
    Ok(())
}

fn validate_resources(value: &Value) -> Result<()> {
    let resources = value.as_array().ok_or(Error(BAD))?;
    if resources.len() > 48 {
        return Err(Error(BAD));
    }
    for resource in resources {
        object(resource, &["url", "kind"])?;
        if !["SCRIPT", "STYLE", "ASSET", "MODULE"].contains(&text(resource, "kind")?) {
            return Err(Error(BAD));
        }
        resource_url(text(resource, "url")?)?;
    }
    Ok(())
}

fn resource_url(raw: &str) -> Result<()> {
    if raw.is_empty() || raw.len() > 8192 || raw.chars().any(char::is_control) {
        return Err(Error(BAD));
    }
    let base =
        reqwest::Url::parse("https://historical-resource.invalid/").map_err(|_| Error(BAD))?;
    let url = base.join(raw).map_err(|_| Error(BAD))?;
    if !["http", "https"].contains(&url.scheme())
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Error(BAD));
    }
    Ok(())
}

fn validation(value: &Value) -> Result<()> {
    let root = object(
        value,
        &["passed", "checkedAt", "checks", "limits", "entryAvailable"],
    )?;
    if root.is_empty() {
        return Ok(()); // DRAFT default from the original schema.
    }
    boolean(value, "passed")?;
    if text(value, "checkedAt")?.is_empty() {
        return Err(Error(BAD));
    }
    let checks = value
        .get("checks")
        .and_then(Value::as_array)
        .ok_or(Error(BAD))?;
    if checks.len() > 64 {
        return Err(Error(BAD));
    }
    for check in checks {
        object(
            check,
            &[
                "url",
                "kind",
                "passed",
                "statusCode",
                "contentType",
                "bytes",
                "message",
            ],
        )?;
        text(check, "url")?;
        text(check, "kind")?;
        text(check, "message")?;
        boolean(check, "passed")?;
        if let Some(status) = check.get("statusCode") {
            if !status.is_null() && status.as_u64().is_none_or(|code| code > u16::MAX as u64) {
                return Err(Error(BAD));
            }
        }
        if check.get("contentType").is_some() {
            nullable_text(check, "contentType")?;
        }
        if check.get("bytes").is_some() {
            integer(check, "bytes")?;
        }
    }
    if value.get("entryAvailable").is_some() {
        boolean(value, "entryAvailable")?;
    }
    if let Some(limits) = value.get("limits") {
        let fields = [
            "requestTimeoutSeconds",
            "totalTimeoutSeconds",
            "maxResponseBytes",
            "maxTotalBytes",
            "maxChecks",
            "maxRedirects",
        ];
        object(limits, &fields)?;
        for field in fields {
            integer(limits, field)?;
        }
    }
    Ok(())
}

/// Validate archived task data after scalar columns have been mapped. This
/// function deliberately leaves receipts, hashes and preflight evidence intact.
pub(super) fn transform(
    kind: &str,
    table: &str,
    row: &mut Value,
    map: &TypedMap,
    registry: &Value,
) -> Result<bool> {
    if kind != "platform" {
        return Ok(false);
    }
    match table {
        "managed_deployment_jobs" => {
            super::require_handler(registry, "managed-deployment-receipt")?;
            let operation = text(row, "operation")?;
            let payload = text(row, "payload_sha256")?;
            if !matches!(operation, "DEPLOY" | "CLEANUP")
                || (operation == "DEPLOY" && !digest(payload))
                || (operation == "CLEANUP" && payload != "cleanup")
            {
                return Err(Error(BAD));
            }
            let value = row.get("receipt").ok_or(Error(BAD))?;
            if !value.is_null() {
                let receipt = managed_receipt(value)?;
                if integer(row, "attempt")? != receipt.attempt as u64
                    || text(row, "state")? != receipt.state
                    || (operation == "DEPLOY" && receipt.state == "CLEANED")
                    || (operation == "CLEANUP" && receipt.state == "READY")
                {
                    return Err(Error(BAD));
                }
                row_reference(
                    row,
                    "nonce",
                    &receipt.nonce,
                    "platform.managed_deployment_attempt",
                    map,
                )?;
            }
            // A source archive is not proof that an engine-side effect can be
            // adopted or safely retried in the new identity epoch.
            return Err(Error("task effect reconciliation required"));
        }
        "zip_distribution_nodes" => {
            super::require_handler(registry, "zip-node-receipt")?;
            let value = row.get("receipt").ok_or(Error(BAD))?;
            if !value.is_null() {
                let receipt = zip_receipt(value)?;
                if text(row, "node_id")? != receipt.work.node_id
                    || integer(row, "attempt")? != receipt.work.attempt as u64
                    || text(row, "state")? != "READY"
                {
                    return Err(Error(BAD));
                }
                row_reference(
                    row,
                    "job_id",
                    &receipt.work.job_id,
                    "platform.zip_distribution_job",
                    map,
                )?;
                // operation=VERIFY also occurs during a normal attempt. Without
                // the original job phase no nonce authority can be inferred.
            }
            return Err(Error("task effect reconciliation required"));
        }
        "application_release_artifacts" => {
            super::require_handler(registry, "application_release_artifacts-manifest")?;
            let m = manifest(row.get("manifest").ok_or(Error(BAD))?, false)?;
            if text(row, "artifact_digest")? != m.artifact_sha256 {
                return Err(Error(BAD));
            }
        }
        "zip_distribution_jobs" => {
            super::require_handler(registry, "zip_distribution_jobs-manifest")?;
            super::require_handler(registry, "zip_distribution_jobs-policy")?;
            let m = manifest(row.get("manifest").ok_or(Error(BAD))?, true)?;
            let p = policy(row.get("policy").ok_or(Error(BAD))?)?;
            let revision = hash(&serde_json::to_vec(&p).map_err(|_| Error(BAD))?);
            if text(row, "artifact_digest")? != m.artifact_sha256
                || text(row, "policy_revision")? != revision
                || text(row, "group_id")? != p.id
                || !digest(text(row, "payload_sha256")?)
            {
                return Err(Error(BAD));
            }
            return Err(Error("task effect reconciliation required"));
        }
        "application_releases" => {
            super::require_handler(registry, "release-resources")?;
            super::require_handler(registry, "release-validation-evidence")?;
            validate_resources(row.get("resources").ok_or(Error(BAD))?)?;
            let result = row.get("validation_result").ok_or(Error(BAD))?;
            validation(result)?;
            if !result.as_object().is_some_and(|value| value.is_empty())
                || row.get("validation_status").and_then(Value::as_str) == Some("PASSED")
            {
                return Err(Error("task effect reconciliation required"));
            }
            if text(row, "validation_status")? != "DRAFT"
                || !row.get("checked_at").is_some_and(Value::is_null)
            {
                return Err(Error(BAD));
            }
            let mut resources = row["resources"].clone();
            for resource in resources.as_array_mut().ok_or(Error(BAD))? {
                resource["url"] =
                    Value::String(super::rewrite_route(text(resource, "url")?, map, registry)?);
            }
            row["resources"] = resources;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn audit_shape(action: &str, detail: &Value) -> Result<bool> {
    let keys: &[&str] = match action {
        "MANAGED_DEPLOYMENT_CREATE" => &["jobId", "releaseId", "targetId", "imageRef"],
        "MANAGED_DEPLOYMENT_RETRY" | "ZIP_DISTRIBUTION_RETRY" => &["jobId"],
        "MANAGED_DEPLOYMENT_CLEANUP" => &["jobId", "artifactId"],
        "MANAGED_DEPLOYMENT_REJECT" => &[
            "jobId",
            "targetId",
            "policyChanged",
            "protectedCleanup",
            "errorCode",
        ],
        "MANAGED_DEPLOYMENT_CLAIM" => &["jobId", "targetId", "attempt"],
        "MANAGED_DEPLOYMENT_RECEIPT" => &["jobId", "targetId", "state", "attempt", "errorCode"],
        "ZIP_DISTRIBUTION_CREATE" => &[
            "jobId",
            "releaseId",
            "groupId",
            "artifactDigest",
            "requiredNodes",
        ],
        "ZIP_DISTRIBUTION_REJECT" => &["jobId", "errorCode"],
        "ZIP_DISTRIBUTION_CLAIM" => &["jobId", "attempt", "nonce"],
        "ZIP_DISTRIBUTION_NODE" => &["jobId", "nodeId", "attempt", "receipt", "errorCode"],
        "ZIP_DISTRIBUTION_READY" | "ZIP_DISTRIBUTION_FAILED" => &[
            "jobId",
            "attempt",
            "allRequiredNodesReady",
            "policyUnchanged",
        ],
        "ZIP_VERIFY_BEGIN" => &["jobId", "nonce"],
        "ZIP_VERIFY_FINISH" => &["jobId", "nonce", "ready"],
        _ if action.starts_with("MANAGED_DEPLOYMENT_")
            || action.starts_with("ZIP_DISTRIBUTION_")
            || action.starts_with("ZIP_VERIFY_") =>
        {
            return Err(Error("unregistered task audit action"))
        }
        _ => return Ok(false),
    };
    let fields = object(detail, keys)?;
    if fields.len() != keys.len() {
        return Err(Error(BAD));
    }
    for key in keys {
        match *key {
            "attempt" => {
                if !(1..=1000).contains(&integer(detail, key)?) {
                    return Err(Error(BAD));
                }
            }
            "requiredNodes" => {
                if !(2..=8).contains(&integer(detail, key)?) {
                    return Err(Error(BAD));
                }
            }
            "policyChanged"
            | "protectedCleanup"
            | "allRequiredNodesReady"
            | "policyUnchanged"
            | "ready" => boolean(detail, key)?,
            "receipt" => {
                let value = detail.get(key).ok_or(Error(BAD))?;
                if !value.is_null() {
                    let receipt = zip_receipt(value)?;
                    if receipt.work.job_id != text(detail, "jobId")?
                        || receipt.work.node_id != text(detail, "nodeId")?
                        || receipt.work.attempt as u64 != integer(detail, "attempt")?
                    {
                        return Err(Error(BAD));
                    }
                }
            }
            "errorCode"
                if matches!(
                    action,
                    "MANAGED_DEPLOYMENT_RECEIPT" | "ZIP_DISTRIBUTION_NODE"
                ) =>
            {
                nullable_text(detail, key)?
            }
            "jobId" | "releaseId" | "artifactId" | "nonce" => {
                source_uuid(text(detail, key)?)?;
            }
            "artifactDigest" => {
                if !digest(text(detail, key)?) {
                    return Err(Error(BAD));
                }
            }
            _ => {
                text(detail, key)?;
            }
        }
    }
    Ok(true)
}

/// Project registered audit references. Embedded node receipts require separate
/// reconciliation; their original bytes remain in the encrypted source evidence.
pub(super) fn audit(action: &str, detail: &mut Value, map: &TypedMap) -> Result<bool> {
    if !audit_shape(action, detail)? {
        return Ok(false);
    }
    if action == "ZIP_DISTRIBUTION_NODE"
        && detail.get("receipt").is_some_and(|value| !value.is_null())
    {
        return Err(Error(
            "historical receipt projection requires reconciliation",
        ));
    }
    let mut projected = detail.clone();
    let job_authority = if action.starts_with("MANAGED_DEPLOYMENT_") {
        "platform.managed_deployment_job"
    } else {
        "platform.zip_distribution_job"
    };
    for (key, authority) in [
        ("jobId", job_authority),
        ("releaseId", "platform.release"),
        ("artifactId", "platform.managed_image_artifact"),
    ] {
        if projected.get(key).is_some() {
            let old = source_uuid(text(&projected, key)?)?;
            projected[key] = Value::String(map.resolve_historical(authority, old)?.to_string());
        }
    }
    if projected.get("nonce").is_some() {
        let authority = if action == "ZIP_DISTRIBUTION_CLAIM" {
            "platform.zip_distribution_attempt"
        } else if ["ZIP_VERIFY_BEGIN", "ZIP_VERIFY_FINISH"].contains(&action) {
            "platform.zip_distribution_verification"
        } else {
            return Err(Error("task audit nonce authority is ambiguous"));
        };
        let old = source_uuid(text(&projected, "nonce")?)?;
        projected["nonce"] = Value::String(map.resolve_historical(authority, old)?.to_string());
    }
    *detail = projected;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const OLD: &str = "00000000-0000-4000-8000-000000000001";

    fn receipt() -> Value {
        json!({"schemaVersion":1,"work":{"schemaVersion":1,"jobId":OLD,"attempt":1,
            "nonce":"00000000-0000-4000-8000-000000000002","groupId":"local",
            "policyRevision":"a".repeat(64),"nodeId":"a","applicationCode":"stores",
            "version":"v1","artifactSha256":"b".repeat(64)},"operation":"VERIFY",
            "manifestSha256":"c".repeat(64),"fileCount":1,"totalBytes":1,"reused":true})
    }

    #[test]
    fn historical_receipt_is_closed_and_not_rewritten() {
        let value = receipt();
        let original = value.clone();
        assert!(zip_receipt(&value).is_ok());
        assert_eq!(value, original);
        let mut unknown = value.clone();
        unknown["work"]["command"] = json!(["sh"]);
        assert!(zip_receipt(&unknown).is_err());
        let mut foreign = value;
        foreign["work"]["nonce"] = json!("opaque-nonce");
        assert!(zip_receipt(&foreign).is_err());
    }

    #[test]
    fn verification_audit_actions_are_registered_and_unknown_actions_fail_closed() {
        assert!(audit_shape("ZIP_VERIFY_BEGIN", &json!({"jobId":OLD,"nonce":OLD})).unwrap());
        assert!(audit_shape(
            "ZIP_VERIFY_FINISH",
            &json!({"jobId":OLD,"nonce":OLD,"ready":false})
        )
        .unwrap());
        assert!(audit_shape("ZIP_VERIFY_FUTURE", &json!({})).is_err());
        assert!(audit_shape(
            "MANAGED_DEPLOYMENT_CLAIM",
            &json!({"jobId":OLD,"targetId":"edge","attempt":1,"nonce":OLD})
        )
        .is_err());
        assert!(!audit_shape("UPDATE_APPLICATION", &json!({})).unwrap());
    }

    #[test]
    fn node_audit_validates_ownership_and_keeps_original_receipt() {
        let detail =
            json!({"jobId":OLD,"nodeId":"a","attempt":1,"receipt":receipt(),"errorCode":null});
        assert!(audit_shape("ZIP_DISTRIBUTION_NODE", &detail).unwrap());
        let mut foreign = detail;
        foreign["nodeId"] = json!("b");
        assert!(audit_shape("ZIP_DISTRIBUTION_NODE", &foreign).is_err());
    }

    #[test]
    fn manifest_digest_uses_declared_field_order_and_esm_is_archive_only() {
        let canonical = format!("{{\"schemaVersion\":1,\"entryType\":\"ESM\",\"entry\":\"entry.mjs\",\"files\":[{{\"path\":\"entry.mjs\",\"size\":1,\"sha256\":\"{}\"}}]}}", "a".repeat(64));
        let mut value: Value = serde_json::from_str(&canonical).unwrap();
        value["artifactSha256"] = json!(hash(canonical.as_bytes()));
        assert!(manifest(&value, false).is_ok());
        assert!(manifest(&value, true).is_err());
        value["files"][0]["size"] = json!(2);
        assert!(manifest(&value, false).is_err());
    }

    #[test]
    fn archived_validation_has_all_known_source_shapes_and_blocks_extensions() {
        assert!(validation(&json!({})).is_ok());
        let mut failed = json!({"passed":false,"checkedAt":"2026-01-01T00:00:00Z", "checks":[{"url":"/entry","kind":"POLICY","passed":false,"message":"policy rejected"}]});
        assert!(validation(&failed).is_ok());
        failed["checks"][0]["newIdentity"] = json!(OLD);
        assert!(validation(&failed).is_err());
    }

    #[test]
    fn resource_urls_use_registered_owner_routes_and_reject_unknown_extensions() {
        assert!(resource_url(&format!("https://static.test/app.js?key={OLD}")).is_ok());
        let map = mapping();
        assert!(super::super::rewrite_route(
            &format!("https://static.test/{OLD}/app.js"),
            &map,
            &json!({})
        )
        .is_err());
        assert!(super::super::rewrite_route(
            "https://static.test/%300000000-0000-4000-8000-000000000001/app.js",
            &map,
            &json!({})
        )
        .is_err());
        assert!(validate_resources(&json!([{"url":"/app.js","kind":"SCRIPT","id":OLD}])).is_err());
    }

    fn mapping() -> TypedMap {
        TypedMap::new(
            1_800_000_000_000,
            BTreeSet::from([
                "platform.managed_deployment_attempt".to_owned(),
                "platform.managed_deployment_job".to_owned(),
                "platform.zip_distribution_job".to_owned(),
                "platform.zip_distribution_attempt".to_owned(),
                "platform.zip_distribution_verification".to_owned(),
                "platform.application_url_revision".to_owned(),
            ]),
        )
        .unwrap()
    }

    fn registry() -> Value {
        json!({"structuredReferences":[
            {"id":"managed-deployment-receipt"},
            {"id":"zip-node-receipt"},
            {"id":"application_release_artifacts-manifest"},
            {"id":"zip_distribution_jobs-manifest"},
            {"id":"zip_distribution_jobs-policy"},
            {"id":"release-resources"},
            {"id":"release-validation-evidence"}
        ],"routeReferences":[{"id":"platform-url-entry"}]})
    }

    #[test]
    fn nonempty_cleanup_tasks_block_without_rewriting_a_sentinel_or_state() {
        let map = mapping();
        let mut row = json!({"operation":"CLEANUP","payload_sha256":"cleanup","state":"QUEUED","receipt":null});
        let original = row.clone();
        assert_eq!(
            transform(
                "platform",
                "managed_deployment_jobs",
                &mut row,
                &map,
                &registry()
            )
            .unwrap_err()
            .0,
            "task effect reconciliation required"
        );
        assert_eq!(row, original);
        row["payload_sha256"] = json!("a".repeat(64));
        assert_eq!(
            transform(
                "platform",
                "managed_deployment_jobs",
                &mut row,
                &map,
                &registry()
            )
            .unwrap_err()
            .0,
            BAD
        );
    }

    #[test]
    fn audit_projection_maps_only_the_action_authority_and_is_atomic() {
        let mut map = mapping();
        let old = source_uuid(OLD).unwrap();
        let job = map
            .allocate("platform.zip_distribution_job", old, None)
            .unwrap();
        let nonce = map
            .allocate("platform.zip_distribution_verification", old, None)
            .unwrap();
        let mut value = json!({"jobId":OLD,"nonce":OLD,"ready":true});
        assert!(audit("ZIP_VERIFY_FINISH", &mut value, &map).unwrap());
        assert_eq!(value["jobId"], job.to_string());
        assert_eq!(value["nonce"], nonce.to_string());
        let mut missing_attempt = json!({"jobId":OLD,"attempt":1,"nonce":OLD});
        let original = missing_attempt.clone();
        assert!(audit("ZIP_DISTRIBUTION_CLAIM", &mut missing_attempt, &map).is_err());
        assert_eq!(missing_attempt, original);
    }

    #[test]
    fn embedded_receipt_cannot_become_live_projection_even_with_complete_mapping() {
        let map = mapping();
        let mut value =
            json!({"jobId":OLD,"nodeId":"a","attempt":1,"receipt":receipt(),"errorCode":null});
        let original = value.clone();
        assert_eq!(
            audit("ZIP_DISTRIBUTION_NODE", &mut value, &map)
                .unwrap_err()
                .0,
            "historical receipt projection requires reconciliation"
        );
        assert_eq!(value, original);
    }

    #[test]
    fn draft_resources_map_registered_routes_and_validation_cannot_be_reused() {
        let mut map = mapping();
        let old = source_uuid(OLD).unwrap();
        let target = map
            .allocate("platform.application_url_revision", old, None)
            .unwrap();
        let registry = registry();
        let mut draft = json!({"resources":[{"url":format!("/micro-proxy/stores/{OLD}/app.js"),"kind":"SCRIPT"}],"validation_result":{},"validation_status":"DRAFT","checked_at":null});
        assert!(transform(
            "platform",
            "application_releases",
            &mut draft,
            &map,
            &registry
        )
        .unwrap());
        assert_eq!(
            draft["resources"][0]["url"],
            format!("/micro-proxy/stores/{target}/app.js")
        );
        draft["validation_status"] = json!("PASSED");
        let original = draft.clone();
        assert_eq!(
            transform(
                "platform",
                "application_releases",
                &mut draft,
                &map,
                &registry
            )
            .unwrap_err()
            .0,
            "task effect reconciliation required"
        );
        assert_eq!(draft, original);
    }

    #[test]
    fn zip_policy_hash_is_closed_and_nonempty_jobs_require_reconciliation() {
        let canonical = format!("{{\"schemaVersion\":1,\"entryType\":\"HTML\",\"entry\":\"index.html\",\"files\":[{{\"path\":\"index.html\",\"size\":1,\"sha256\":\"{}\"}}]}}", "a".repeat(64));
        let mut m: Value = serde_json::from_str(&canonical).unwrap();
        m["artifactSha256"] = json!(hash(canonical.as_bytes()));
        let p = json!({"schemaVersion":1,"id":"local","name":"Local","publicOrigin":"https://public.test","applications":["stores"],"nodes":[{"id":"a","origin":"https://a.test"},{"id":"b","origin":"https://b.test"}]});
        let revision = hash(&serde_json::to_vec(&policy(&p).unwrap()).unwrap());
        let mut row = json!({"manifest":m,"artifact_digest":hash(canonical.as_bytes()),"policy":p,"group_id":"local","policy_revision":revision,"payload_sha256":"d".repeat(64)});
        let original = row.clone();
        assert_eq!(
            transform(
                "platform",
                "zip_distribution_jobs",
                &mut row,
                &mapping(),
                &registry()
            )
            .unwrap_err()
            .0,
            "task effect reconciliation required"
        );
        assert_eq!(row, original);
        row["policy"]["nodes"][0]["newId"] = json!(OLD);
        assert_eq!(
            transform(
                "platform",
                "zip_distribution_jobs",
                &mut row,
                &mapping(),
                &registry()
            )
            .unwrap_err()
            .0,
            BAD
        );
    }

    #[test]
    fn managed_archive_compares_source_nonce_to_mapped_row_and_never_reissues_receipt() {
        let mut map = mapping();
        let target = map
            .allocate(
                "platform.managed_deployment_attempt",
                source_uuid(OLD).unwrap(),
                None,
            )
            .unwrap();
        let receipt = json!({"nonce":OLD,"attempt":1,"state":"READY","imageId":format!("sha256:{}", "a".repeat(64)),"containerName":"sd-edge-stores-0123456789","routeSha256":"b".repeat(64),"errorCode":null});
        let mut row = json!({"operation":"DEPLOY","payload_sha256":"c".repeat(64),"receipt":receipt,"attempt":1,"state":"READY","nonce":target.to_string()});
        let original = row.clone();
        assert_eq!(
            transform(
                "platform",
                "managed_deployment_jobs",
                &mut row,
                &map,
                &registry()
            )
            .unwrap_err()
            .0,
            "task effect reconciliation required"
        );
        assert_eq!(row, original);
        row["receipt"]["imageId"] = Value::Null;
        assert_eq!(
            transform(
                "platform",
                "managed_deployment_jobs",
                &mut row,
                &map,
                &registry()
            )
            .unwrap_err()
            .0,
            BAD
        );
    }
}
