//! Explicitly offline UUID epoch conversion. Source snapshots, typed mappings,
//! and transformed rows are private authenticated files, never live SQL rewrites.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod codec;
pub(crate) mod config_rekey;
pub(crate) mod evidence;
pub(crate) mod export;
pub(crate) mod import;
mod mapping;
mod reconcile;
mod rows;
mod structured;
pub(crate) mod transform;
pub(crate) mod verify;

/// Seal an operator-collected private tmpfs evidence archive. This entry point
/// never opens a database or emits archive content.
pub(crate) fn seal_evidence(path: &std::path::Path) -> crate::Result<String> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Evidence {
        profile: String,
        input: std::path::PathBuf,
        output: std::path::PathBuf,
        key_file: std::path::PathBuf,
        purpose: String,
    }
    let config: Evidence = serde_json::from_slice(&crate::config::private_file(path)?)
        .map_err(|_| crate::Error("invalid evidence sealing configuration"))?;
    if config.profile != "uuid-local-preview"
        || !config.purpose.starts_with("saas-uuid-preview-freeze/1/")
        || config.key_file == config.input
        || config.key_file == config.output
    {
        return Err(crate::Error(
            "local preview evidence sealing policy required",
        ));
    }
    let key = crate::config::private_file(&config.key_file)?;
    let digest = codec::seal_file(&config.input, &config.output, &key, &config.purpose)?;
    if codec::verify_file(&config.output, &key, &config.purpose)? != digest {
        return Err(crate::Error(
            "sealed evidence read-back verification failed",
        ));
    }
    Ok(
        serde_json::json!({"status":"PRIVATE_EVIDENCE_SEALED", "sha256":digest.sha256,
        "plaintext_bytes":digest.plaintext_bytes,"frames":digest.frames})
        .to_string(),
    )
}
