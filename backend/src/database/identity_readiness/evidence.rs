use std::{
    collections::BTreeSet,
    path::{Component, Path},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{caused, fail, Failure, MembershipReadinessError};

const KINDS: [&str; 5] = [
    "PLATFORM_INSTANCE",
    "CLI",
    "IMPORTER",
    "BATCH",
    "OFFLINE_JOB",
];
const GATES: [&str; 10] = [
    "backend",
    "frontend",
    "notification",
    "contracts",
    "phase_01",
    "phase_02",
    "phase_03",
    "phase_04",
    "phase_05",
    "phase_06",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    environment: String,
    scope: String,
    target_database: String,
    target_database_oid: i64,
    target_cluster_system_identifier: String,
    platform_binary_sha256: String,
    generated_at: DateTime<Utc>,
    inventory: Vec<Asset>,
    gates: Vec<Gate>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    path: String,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Asset {
    id: String,
    kind: String,
    status: String,
    binary_sha256: Option<String>,
    evidence: Artifact,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Gate {
    id: String,
    status: String,
    evidence: Artifact,
}

pub(super) struct VerifiedEvidence {
    pub environment: String,
    pub scope: String,
    pub target_database: String,
    pub target_database_oid: i64,
    pub target_cluster_system_identifier: String,
    pub platform_binary_sha256: String,
    pub manifest_sha256: String,
    pub inventory: serde_json::Value,
    pub gates: serde_json::Value,
}

pub(super) fn lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn bounded_file(path: &Path, limit: u64) -> Result<Vec<u8>, MembershipReadinessError> {
    let file = std::fs::File::open(path).map_err(|source| caused(Failure::Evidence, source))?;
    let metadata = file
        .metadata()
        .map_err(|source| caused(Failure::Evidence, source))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > limit {
        return Err(fail(Failure::Evidence));
    }
    // Take a bounded snapshot from the same opened handle, including when a
    // writer replaces or grows an artifact after metadata inspection.
    use std::io::Read;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| caused(Failure::Evidence, source))?;
    if bytes.is_empty() || bytes.len() as u64 > limit {
        return Err(fail(Failure::Evidence));
    }
    Ok(bytes)
}

pub(super) fn binary_sha256(path: &Path) -> Result<String, MembershipReadinessError> {
    // Platform artifacts may be large; stream rather than retaining the binary.
    let mut file = std::fs::File::open(path).map_err(|source| caused(Failure::Binary, source))?;
    if !file
        .metadata()
        .map_err(|source| caused(Failure::Binary, source))?
        .is_file()
    {
        return Err(fail(Failure::Binary));
    }
    use std::io::Read;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|source| caused(Failure::Binary, source))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn verify_artifact(base: &Path, artifact: &Artifact) -> Result<(), MembershipReadinessError> {
    let path = Path::new(&artifact.path);
    if !lower_sha256(&artifact.sha256)
        || artifact.path.is_empty()
        || artifact.path.contains('\\')
        || !path.components().all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(fail(Failure::Evidence));
    }
    let target = std::fs::canonicalize(base.join(path))
        .map_err(|source| caused(Failure::Evidence, source))?;
    if !target.starts_with(base) {
        return Err(fail(Failure::Evidence));
    }
    let bytes = bounded_file(&target, 16 * 1024 * 1024)?;
    if hex::encode(Sha256::digest(&bytes)) != artifact.sha256 {
        return Err(fail(Failure::Evidence));
    }
    Ok(())
}

// The manifest is an operator inventory, not automatic discovery of offline
// writers. Hashed nonempty artifacts make that declaration reviewable; the
// deployment owner remains responsible for its completeness and provenance.
pub(super) fn load(path: &Path) -> Result<VerifiedEvidence, MembershipReadinessError> {
    let bytes = bounded_file(path, 1024 * 1024)?;
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|source| caused(Failure::Evidence, source))?;
    let now = Utc::now();
    if manifest.schema_version != 1
        || !matches!(
            (manifest.environment.as_str(), manifest.scope.as_str()),
            ("development", "TASK_OWNED") | ("production", "PRODUCTION")
        )
        || manifest.target_database.is_empty()
        || manifest.target_database.len() > 63
        || manifest.target_database_oid <= 0
        || manifest.target_cluster_system_identifier.is_empty()
        || manifest.target_cluster_system_identifier.len() > 20
        || !manifest
            .target_cluster_system_identifier
            .bytes()
            .all(|b| b.is_ascii_digit())
        || manifest.target_cluster_system_identifier.starts_with('0')
        || manifest.target_database.chars().any(char::is_control)
        || matches!(
            manifest.target_database.as_str(),
            "postgres" | "template0" | "template1"
        )
        || !lower_sha256(&manifest.platform_binary_sha256)
        || manifest.generated_at > now + chrono::Duration::minutes(5)
        || manifest.generated_at < now - chrono::Duration::hours(24)
        || manifest.inventory.len() > 128
        || manifest.inventory.is_empty()
        || manifest.gates.len() != GATES.len()
    {
        return Err(fail(Failure::Evidence));
    }
    let base = std::fs::canonicalize(
        path.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new(".")),
    )
    .map_err(|source| caused(Failure::Evidence, source))?;
    let mut ids = BTreeSet::new();
    let mut kinds = BTreeSet::new();
    let mut active_platform = false;
    for asset in &manifest.inventory {
        if asset.id.is_empty()
            || asset.id.len() > 128
            || asset.id.chars().any(char::is_control)
            || !ids.insert(&asset.id)
            || !KINDS.contains(&asset.kind.as_str())
            || !matches!(
                asset.status.as_str(),
                "READY" | "RETIRED" | "NOT_APPLICABLE"
            )
            || asset
                .binary_sha256
                .as_deref()
                .is_some_and(|v| !lower_sha256(v))
            || (asset.status == "READY" && asset.binary_sha256.is_none())
            || (asset.status == "NOT_APPLICABLE" && asset.binary_sha256.is_some())
        {
            return Err(fail(Failure::Evidence));
        }
        if asset.kind == "PLATFORM_INSTANCE" && asset.status == "READY" {
            if asset.binary_sha256.as_deref() != Some(&manifest.platform_binary_sha256) {
                return Err(fail(Failure::Evidence));
            }
            active_platform = true;
        }
        kinds.insert(asset.kind.as_str());
        verify_artifact(&base, &asset.evidence)?;
    }
    if !active_platform || !KINDS.iter().all(|kind| kinds.contains(kind)) {
        return Err(fail(Failure::Evidence));
    }
    let mut gates = BTreeSet::new();
    for gate in &manifest.gates {
        if !GATES.contains(&gate.id.as_str())
            || !gates.insert(gate.id.as_str())
            || gate.status != "PASS"
        {
            return Err(fail(Failure::Evidence));
        }
        verify_artifact(&base, &gate.evidence)?;
    }
    Ok(VerifiedEvidence {
        environment: manifest.environment,
        scope: manifest.scope,
        target_database: manifest.target_database,
        target_database_oid: manifest.target_database_oid,
        target_cluster_system_identifier: manifest.target_cluster_system_identifier,
        platform_binary_sha256: manifest.platform_binary_sha256,
        manifest_sha256: hex::encode(Sha256::digest(&bytes)),
        inventory: serde_json::to_value(manifest.inventory)
            .map_err(|source| caused(Failure::Evidence, source))?,
        gates: serde_json::to_value(manifest.gates)
            .map_err(|source| caused(Failure::Evidence, source))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (std::path::PathBuf, serde_json::Value) {
        let base = std::env::temp_dir().join(format!("identity-evidence-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir(&base).unwrap();
        std::fs::write(base.join("result.json"), b"{\"fixture_assertions\":true}").unwrap();
        let hash = hex::encode(Sha256::digest(b"{\"fixture_assertions\":true}"));
        let binary = "a".repeat(64);
        let artifact = serde_json::json!({"path":"result.json", "sha256":hash});
        let inventory: Vec<_> = KINDS.iter().map(|kind| serde_json::json!({
            "id":kind,"kind":kind,"status":if *kind=="PLATFORM_INSTANCE" {"READY"} else {"NOT_APPLICABLE"},
            "binary_sha256":if *kind=="PLATFORM_INSTANCE" {Some(binary.clone())} else {None},"evidence":artifact
        })).collect();
        let gates: Vec<_> = GATES
            .iter()
            .map(|id| serde_json::json!({"id":id,"status":"PASS","evidence":artifact}))
            .collect();
        (
            base,
            serde_json::json!({"schema_version":1,"environment":"development","scope":"TASK_OWNED",
            "target_database":"phase06_fixture","target_database_oid":12345,"target_cluster_system_identifier":"1234567890123456789","platform_binary_sha256":binary,"generated_at":Utc::now(),
            "inventory":inventory,"gates":gates}),
        )
    }

    #[test]
    fn evidence_requires_actual_bounded_artifacts_and_complete_inventory() {
        let (base, valid) = fixture();
        let path = base.join("manifest.json");
        let check = |value: &serde_json::Value| {
            std::fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
            load(&path)
        };
        assert!(check(&valid).is_ok());
        for mutate in 0..8 {
            let mut invalid = valid.clone();
            match mutate {
                0 => invalid["inventory"][0]["binary_sha256"] = "b".repeat(64).into(),
                1 => invalid["inventory"]
                    .as_array_mut()
                    .unwrap()
                    .pop()
                    .map(|_| ())
                    .unwrap(),
                2 => invalid["gates"][0]["status"] = "SKIPPED".into(),
                3 => invalid["gates"][0]["evidence"]["path"] = "../result.json".into(),
                4 => invalid["gates"][0]["evidence"]["sha256"] = "b".repeat(64).into(),
                5 => {
                    invalid["generated_at"] = (Utc::now() - chrono::Duration::hours(25))
                        .to_rfc3339()
                        .into()
                }
                6 => invalid["environment"] = "production".into(),
                _ => invalid["unknown"] = true.into(),
            }
            assert!(
                check(&invalid).is_err(),
                "invalid evidence {mutate} accepted"
            );
        }
        std::fs::write(base.join("result.json"), b"tampered").unwrap();
        assert!(check(&valid).is_err());
        assert!(base.is_absolute() && base.starts_with(std::env::temp_dir()));
        std::fs::remove_dir_all(base).unwrap();
    }
}
