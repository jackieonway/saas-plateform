//! Data-only contract shared by the platform and its independent deployment agent.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
pub mod zip;

/// A fixed static-service template. Runtime commands are never part of a job.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Template {
    pub id: String,
    pub application_code: String,
    pub health_path: String,
}

/// Public target policy; credentials and Docker authority stay in operator config.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Target {
    pub id: String,
    pub name: String,
    pub public_origin: String,
    pub repositories: Vec<String>,
    pub templates: Vec<Template>,
}

/// An immutable deployment intent. Unknown keys (commands, mounts, ports) fail closed.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Intent {
    pub release_id: String,
    pub request_id: String,
    pub target_id: String,
    pub template_id: String,
    pub image_ref: String,
}

/// A single leased operation; nonce and attempt fence stale receipts.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Work {
    pub job_id: String,
    pub release_id: String,
    pub application_code: String,
    pub environment: String,
    pub version: String,
    pub target_id: String,
    pub template_id: String,
    pub image_ref: String,
    pub policy_revision: String,
    pub operation: String,
    pub attempt: i32,
    pub nonce: String,
}

/// Authenticated completion evidence. Raw engine output must never cross this boundary.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Receipt {
    pub nonce: String,
    pub attempt: i32,
    pub state: String,
    pub image_id: Option<String>,
    pub container_name: String,
    pub route_sha256: Option<String>,
    pub error_code: Option<String>,
}

/// Safe identifiers are one path/engine-name segment, never shell or URL fragments.
pub fn segment(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && !value.contains("..")
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}

/// Accept only an exact allowlisted repository with a lowercase SHA-256 digest.
pub fn image_allowed(reference: &str, repositories: &[String]) -> bool {
    let Some((repository, digest)) = reference.split_once("@sha256:") else {
        return false;
    };
    digest.len() == 64
        && digest
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        && repositories.iter().any(|allowed| allowed == repository)
        && !repository.contains("..")
        && !repository.contains('@')
        && repository.contains('/')
        && !repository.starts_with('/')
        && repository
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"./:_-".contains(&c))
}

/// Stable content identity used for both the public policy revision and route bytes.
pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Bind a receipt/job to the complete configured public target policy.
pub fn revision(target: &Target) -> String {
    // Struct serialization has no fallible custom serializers or non-string keys.
    sha256(serde_json::to_string(target).unwrap_or_default().as_bytes())
}

/// Engine identity is independent of environment/job so identical cross-env promotion reuses bytes.
pub fn container_name(target: &str, app: &str, version: &str, image: &str) -> String {
    let digest = sha256(format!("{target}\n{app}\n{version}\n{image}").as_bytes());
    format!("sd-{target}-{app}-{}", &digest[..20])
}

/// Public immutable route prefix; caller must validate both path segments first.
pub fn version_path(app: &str, version: &str) -> String {
    format!("/sub-apps/{app}/versions/{version}/")
}

/// Fixed Nginx include. Validated path segments and derived DNS name are its only inputs.
pub fn route_config(work: &Work) -> String {
    let prefix = version_path(&work.application_code, &work.version);
    let name = container_name(
        &work.target_id,
        &work.application_code,
        &work.version,
        &work.image_ref,
    );
    format!(
        "location ^~ {prefix} {{\n    proxy_pass http://{name}:8080/;\n    proxy_set_header Host $host;\n    proxy_set_header Authorization \"\";\n    proxy_connect_timeout 3s;\n    proxy_read_timeout 10s;\n}}\n"
    )
}

/// Validate an operation against local policy before any engine or filesystem access.
pub fn work_allowed(work: &Work, target: &Target) -> bool {
    segment(&target.id, 24)
        && work.target_id == target.id
        && segment(&work.application_code, 48)
        && segment(&work.version, 64)
        && work.policy_revision == revision(target)
        && work.attempt > 0
        && ["DEPLOY", "CLEANUP"].contains(&work.operation.as_str())
        && image_allowed(&work.image_ref, &target.repositories)
        && target.templates.iter().any(|t| {
            t.id == work.template_id
                && t.application_code == work.application_code
                && segment(&t.id, 32)
                && t.health_path == "/healthz"
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target() -> Target {
        Target {
            id: "local-static".into(),
            name: "Local".into(),
            public_origin: "https://host.test".into(),
            repositories: vec!["registry.test:5000/example/web".into()],
            templates: vec![Template {
                id: "static".into(),
                application_code: "stores".into(),
                health_path: "/healthz".into(),
            }],
        }
    }
    fn work() -> Work {
        let t = target();
        Work {
            job_id: "job".into(),
            release_id: "release".into(),
            application_code: "stores".into(),
            environment: "development".into(),
            version: "1.0.0".into(),
            target_id: t.id.clone(),
            template_id: "static".into(),
            image_ref: format!("registry.test:5000/example/web@sha256:{}", "a".repeat(64)),
            policy_revision: revision(&t),
            operation: "DEPLOY".into(),
            attempt: 1,
            nonce: "nonce".into(),
        }
    }
    #[test]
    fn digest_repository_and_target_boundaries() {
        let t = target();
        let w = work();
        assert!(work_allowed(&w, &t));
        for image in [
            "registry.test:5000/example/web:latest".into(),
            format!("foreign.test/example/web@sha256:{}", "a".repeat(64)),
            format!("registry.test:5000/example/web@sha256:{}", "A".repeat(64)),
        ] {
            let mut bad = w.clone();
            bad.image_ref = image;
            assert!(!work_allowed(&bad, &t))
        }
        let mut bad = w.clone();
        bad.target_id = "other".into();
        assert!(!work_allowed(&bad, &t));
        bad = w;
        bad.version = "../escape".into();
        assert!(!work_allowed(&bad, &t));
    }
    #[test]
    fn unknown_commands_and_mounts_are_rejected() {
        let mut v = serde_json::to_value(work()).unwrap();
        for key in ["command", "volumes", "ports", "entrypoint"] {
            v[key] = serde_json::json!(["anything"]);
            assert!(serde_json::from_value::<Work>(v.clone()).is_err());
            v.as_object_mut().unwrap().remove(key);
        }
    }
    #[test]
    fn policy_revision_and_template_are_fenced() {
        let mut t = target();
        let w = work();
        t.templates[0].application_code = "work-orders".into();
        assert!(!work_allowed(&w, &t));
        t = target();
        t.public_origin = "https://changed.test".into();
        assert!(!work_allowed(&w, &t));
    }
    #[test]
    fn names_bind_version_and_digest_without_job_identity() {
        let w = work();
        let name = container_name(&w.target_id, &w.application_code, &w.version, &w.image_ref);
        assert!(segment(&name, 128));
        assert_ne!(
            name,
            container_name(&w.target_id, &w.application_code, "2.0.0", &w.image_ref)
        );
        assert_ne!(
            name,
            container_name(
                &w.target_id,
                &w.application_code,
                &w.version,
                &format!("registry.test:5000/example/web@sha256:{}", "b".repeat(64))
            )
        );
    }
}
