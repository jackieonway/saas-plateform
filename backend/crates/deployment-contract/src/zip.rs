//! Data-only ZIP distribution protocol. Archive extraction remains in the platform.
use crate::{segment, sha256};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Maximum public files in the existing ZIP contract.
pub const MAX_FILES: usize = 49;
/// Maximum decoded bytes in one public file.
pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
/// Maximum decoded bytes in one release.
pub const MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;
/// Bounded JSON transport including base64 overhead.
pub const MAX_WIRE_BYTES: usize = 24 * 1024 * 1024;

/// Closed validation failures; no input or credential values enter error messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Policy,
    Manifest,
    Command,
    Payload,
    Digest,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Policy => "POLICY_REJECTED",
            Self::Manifest => "INVALID_MANIFEST",
            Self::Command => "INVALID_COMMAND",
            Self::Payload => "INVALID_PAYLOAD",
            Self::Digest => "DIGEST_MISMATCH",
        })
    }
}
impl std::error::Error for Error {}

/// A required node. Credentials and CA references never enter this public policy.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub origin: String,
}
/// All nodes are required; there is no optional or best-effort traffic recipient.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Group {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub public_origin: String,
    pub applications: Vec<String>,
    pub nodes: Vec<Node>,
}
/// The existing schema-1 canonical file metadata, not an archive checksum.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct File {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}
/// Field order matches the platform's immutable ZIP manifest.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub entry_type: String,
    pub entry: String,
    pub files: Vec<File>,
    pub artifact_sha256: String,
}
/// Public bytes travel as bounded base64, never as a URL or local filesystem path.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PayloadFile {
    pub path: String,
    pub data_base64: String,
}
/// Already safely extracted files plus their authoritative canonical identity.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Payload {
    pub manifest: Manifest,
    pub files: Vec<PayloadFile>,
}
/// Immutable browser intent; node topology is exclusively server-owned.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Intent {
    pub release_id: String,
    pub group_id: String,
    pub request_id: String,
}
/// A fresh challenge binds each authenticated response to one node and attempt.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Work {
    pub schema_version: u32,
    pub job_id: String,
    pub attempt: i32,
    pub nonce: String,
    pub group_id: String,
    pub policy_revision: String,
    pub node_id: String,
    pub application_code: String,
    pub version: String,
    pub artifact_sha256: String,
}
/// Install has no commands, destinations, URLs, mounts, or extraction options.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Install {
    pub work: Work,
    pub payload: Payload,
}
/// Verification rehashes the actual installed tree and does not install or repair it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Verify {
    pub work: Work,
    pub manifest: Manifest,
}
/// Completion evidence; the node constructs identity from validated work and disk bytes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Receipt {
    pub schema_version: u32,
    pub work: Work,
    pub operation: String,
    pub manifest_sha256: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub reused: bool,
}

/// Require a canonical HTTPS origin, with no path, userinfo, query or fragment.
pub fn https_origin(value: &str) -> bool {
    url::Url::parse(value)
        .is_ok_and(|u| u.scheme() == "https" && u.origin().ascii_serialization() == value)
}
/// Portable application-directory identity inherited from ZIP upload.
pub fn application(value: &str) -> bool {
    (2..=32).contains(&value.len())
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}
/// Canonical lowercase SHA256 text.
pub fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
/// A UUIDv7 correlation key with the RFC variant, never a filename or a secret.
pub fn correlation(value: &str) -> bool {
    value.len() == 36
        && value.as_bytes()[14] == b'7'
        && matches!(
            value.as_bytes()[19],
            b'8' | b'9' | b'a' | b'b' | b'A' | b'B'
        )
        && value.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}
/// Validate every configured recipient; all must have distinct identities and origins.
pub fn validate_group(g: &Group) -> Result<(), Error> {
    if g.schema_version != 1
        || !segment(&g.id, 24)
        || g.name.is_empty()
        || g.name.len() > 80
        || !https_origin(&g.public_origin)
        || !(2..=8).contains(&g.nodes.len())
        || g.applications.is_empty()
        || g.applications.len() > 8
    {
        return Err(Error::Policy);
    }
    let mut apps = HashSet::new();
    let mut ids = HashSet::new();
    let mut origins = HashSet::new();
    if g.applications
        .iter()
        .any(|a| !application(a) || !apps.insert(a))
        || g.nodes.iter().any(|n| {
            !segment(&n.id, 24)
                || !https_origin(&n.origin)
                || !ids.insert(&n.id)
                || !origins.insert(&n.origin)
        })
    {
        return Err(Error::Policy);
    }
    Ok(())
}
/// Public policy hash excludes token and CA file locations and secret contents.
pub fn revision(g: &Group) -> Result<String, Error> {
    validate_group(g)?;
    serde_json::to_vec(g)
        .map(|v| sha256(&v))
        .map_err(|_| Error::Policy)
}
/// Apply the platform's portable public-file path constraints before filesystem access.
pub fn file_path(path: &str) -> bool {
    if path.is_empty()
        || path.len() > 500
        || path.starts_with('/')
        || path.contains(['\\', ':', '?', '#'])
        || path.chars().any(char::is_control)
        || path.eq_ignore_ascii_case("release.json")
    {
        return false;
    }
    path.split('/').all(|p| {
        if p.is_empty() || p == "." || p == ".." || p.len() > 120 || p.ends_with(['.', ' ']) {
            return false;
        }
        let lower = p.to_ascii_lowercase();
        let stem = p.split('.').next().unwrap_or("").to_ascii_uppercase();
        !lower.starts_with(".env")
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
/// Verify the canonical file list and artifact checksum before any payload is accepted.
pub fn validate_manifest(m: &Manifest) -> Result<(), Error> {
    if m.schema_version != 1
        || m.entry_type != "HTML"
        || m.entry != "index.html"
        || m.files.is_empty()
        || m.files.len() > MAX_FILES
        || !digest(&m.artifact_sha256)
    {
        return Err(Error::Manifest);
    }
    let mut total = 0u64;
    let mut names = HashSet::new();
    let mut previous = "";
    for f in &m.files {
        total = total.checked_add(f.size).ok_or(Error::Manifest)?;
        if !file_path(&f.path)
            || f.path.as_str() <= previous
            || !names.insert(f.path.to_ascii_lowercase())
            || f.size == 0
            || f.size > MAX_FILE_BYTES as u64
            || !digest(&f.sha256)
            || total > MAX_TOTAL_BYTES as u64
        {
            return Err(Error::Manifest);
        }
        previous = &f.path;
    }
    if !m.files.iter().any(|f| f.path == m.entry) {
        return Err(Error::Manifest);
    }
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Canonical<'a> {
        schema_version: u32,
        entry_type: &'a str,
        entry: &'a str,
        files: &'a [File],
    }
    let bytes = serde_json::to_vec(&Canonical {
        schema_version: m.schema_version,
        entry_type: &m.entry_type,
        entry: &m.entry,
        files: &m.files,
    })
    .map_err(|_| Error::Manifest)?;
    if sha256(&bytes) != m.artifact_sha256 {
        return Err(Error::Digest);
    }
    Ok(())
}
/// Strict node-local authorization of an installation or verification challenge.
pub fn validate_work(w: &Work, g: &Group, node: &str) -> Result<(), Error> {
    if w.schema_version != 1
        || !correlation(&w.job_id)
        || !correlation(&w.nonce)
        || !(1..=1000).contains(&w.attempt)
        || w.group_id != g.id
        || w.node_id != node
        || !g.nodes.iter().any(|n| n.id == node)
        || !g.applications.contains(&w.application_code)
        || !segment(&w.version, 64)
        || !digest(&w.artifact_sha256)
        || w.policy_revision != revision(g)?
    {
        return Err(Error::Command);
    }
    Ok(())
}
/// Validate and decode one bounded public payload without touching disk.
pub fn decode(p: &Payload) -> Result<Vec<(String, Vec<u8>)>, Error> {
    validate_manifest(&p.manifest)?;
    if p.files.len() != p.manifest.files.len() {
        return Err(Error::Payload);
    }
    p.files
        .iter()
        .zip(&p.manifest.files)
        .map(|(data, f)| {
            if data.path != f.path || data.data_base64.len() > MAX_FILE_BYTES.div_ceil(3) * 4 {
                return Err(Error::Payload);
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&data.data_base64)
                .map_err(|_| Error::Payload)?;
            if bytes.len() as u64 != f.size || sha256(&bytes) != f.sha256 {
                return Err(Error::Digest);
            }
            Ok((f.path.clone(), bytes))
        })
        .collect()
}
/// Fingerprint the complete manifest for an authenticated response.
pub fn manifest_fingerprint(m: &Manifest) -> Result<String, Error> {
    validate_manifest(m)?;
    serde_json::to_vec(m)
        .map(|v| sha256(&v))
        .map_err(|_| Error::Manifest)
}
/// Construct receipt metadata only after the node has checked the actual disk tree.
pub fn receipt(w: &Work, m: &Manifest, operation: &str, reused: bool) -> Result<Receipt, Error> {
    Ok(Receipt {
        schema_version: 1,
        work: w.clone(),
        operation: operation.to_owned(),
        manifest_sha256: manifest_fingerprint(m)?,
        file_count: m.files.len(),
        total_bytes: m.files.iter().map(|f| f.size).sum(),
        reused,
    })
}
/// Reject stale, foreign or partially matching evidence even over an authenticated channel.
pub fn receipt_matches(r: &Receipt, w: &Work, m: &Manifest, operation: &str) -> bool {
    receipt(w, m, operation, r.reused).is_ok_and(|expected| expected == *r)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn payload() -> Payload {
        let bytes = b"<html>version</html>";
        let files = vec![File {
            path: "index.html".into(),
            size: bytes.len() as u64,
            sha256: sha256(bytes),
        }];
        // Use the existing platform manifest's explicit field order, not a JSON map.
        let text = format!(
            "{{\"schemaVersion\":1,\"entryType\":\"HTML\",\"entry\":\"index.html\",\"files\":{}}}",
            serde_json::to_string(&files).unwrap()
        );
        Payload {
            manifest: Manifest {
                schema_version: 1,
                entry_type: "HTML".into(),
                entry: "index.html".into(),
                files,
                artifact_sha256: sha256(text.as_bytes()),
            },
            files: vec![PayloadFile {
                path: "index.html".into(),
                data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            }],
        }
    }
    #[test]
    fn canonical_bytes_and_digest_are_verified() {
        let mut p = payload();
        assert_eq!(decode(&p).unwrap()[0].1, b"<html>version</html>");
        p.files[0].data_base64 = base64::engine::general_purpose::STANDARD.encode(b"different");
        assert_eq!(decode(&p).unwrap_err(), Error::Digest)
    }
    #[test]
    fn paths_and_manifest_aliases_fail_closed() {
        for p in [
            "../index.html",
            "/index.html",
            "x\\index.html",
            ".env",
            ".npmrc",
            "dist/.NPMRC",
            "frontend/.npmrc",
            "x/.git/file",
            "index.html/../x",
            "CON.txt",
            "x/.artifact-manifest.json",
            "release.json",
        ] {
            assert!(!file_path(p), "{p}")
        }
        let mut p = payload();
        p.manifest.files.push(p.manifest.files[0].clone());
        assert_eq!(validate_manifest(&p.manifest), Err(Error::Manifest));
    }
    #[test]
    fn topology_is_required_and_commands_are_closed() {
        let g = Group {
            schema_version: 1,
            id: "local".into(),
            name: "Local".into(),
            public_origin: "https://public.test".into(),
            applications: vec!["stores".into()],
            nodes: vec![
                Node {
                    id: "a".into(),
                    origin: "https://a.test".into(),
                },
                Node {
                    id: "b".into(),
                    origin: "https://b.test".into(),
                },
            ],
        };
        assert!(validate_group(&g).is_ok());
        let w = Work {
            schema_version: 1,
            job_id: "00000000-0000-7000-8000-000000000001".into(),
            attempt: 1,
            nonce: "00000000-0000-7000-8000-000000000002".into(),
            group_id: g.id.clone(),
            policy_revision: revision(&g).unwrap(),
            node_id: "a".into(),
            application_code: "stores".into(),
            version: "v1".into(),
            artifact_sha256: payload().manifest.artifact_sha256,
        };
        assert!(validate_work(&w, &g, "a").is_ok());
        assert!(validate_work(&w, &g, "b").is_err());
        // Version directories follow the existing release contract. The node is
        // Linux-only; Windows device-name rules apply to portable payload files.
        for version in ["CON", "v1.", "1.2.3-beta_1"] {
            let mut legacy = w.clone();
            legacy.version = version.into();
            assert!(validate_work(&legacy, &g, "a").is_ok());
        }
        let mut raw = serde_json::to_value(&w).unwrap();
        raw["command"] = serde_json::json!(["sh", "-c", "anything"]);
        assert!(serde_json::from_value::<Work>(raw).is_err());
    }
    #[test]
    fn altered_nonce_or_manifest_receipt_cannot_be_adopted() {
        let p = payload();
        let w:Work=serde_json::from_value(serde_json::json!({"schemaVersion":1,"jobId":"00000000-0000-7000-8000-000000000001","attempt":1,"nonce":"00000000-0000-7000-8000-000000000002","groupId":"local","policyRevision":"a".repeat(64),"nodeId":"a","applicationCode":"stores","version":"v1","artifactSha256":p.manifest.artifact_sha256})).unwrap();
        let mut r = receipt(&w, &p.manifest, "VERIFY", true).unwrap();
        assert!(receipt_matches(&r, &w, &p.manifest, "VERIFY"));
        r.work.attempt = 2;
        assert!(!receipt_matches(&r, &w, &p.manifest, "VERIFY"));
    }
}
