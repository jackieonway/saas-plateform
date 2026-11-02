//! Explicit platform administration with offline plans and redacted failures.
//! The binary reads only named environment values after identifying a remote command.
mod access_policy;
mod artifact;
mod bootstrap;
mod contract;
mod error;
mod grants;
mod http;
mod options;
mod release;
mod retention;

pub use error::SafeError;
use http::ManagementApi;
use options::Options;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Execute CLI arguments with explicitly supplied environment values.
/// Plans never consult environment values or create an HTTP client. Errors expose
/// fixed categories only; uncertain writes must be inspected before another attempt.
pub fn run(
    args: impl IntoIterator<Item = String>,
    env: &BTreeMap<String, String>,
) -> Result<Value, SafeError> {
    let options = Options::parse(args)?;
    if options.command == "help" {
        return Ok(json!({"ok":true,"usage":options::USAGE}));
    }
    bootstrap::definition(options.app())?;
    if options.command == "access-policy-save" && !options.apply {
        return access_policy::plan(&options);
    }
    if matches!(
        options.command.as_str(),
        "artifact-bind" | "artifact-verify"
    ) {
        return artifact::execute(&options);
    }
    if options.command == "grants-plan" || (options.command == "grants-apply" && !options.apply) {
        return grants::plan(
            options.manifest_path()?,
            options.app_code,
            options.app_id.as_deref(),
        );
    }
    // The guard runs before client construction, including every HTTP read.
    let artifact = if options.artifact_file.is_some() {
        Some(artifact::verify(&options)?.release)
    } else if matches!(options.command.as_str(), "register" | "release-reconcile") {
        Some(release::read(&options)?)
    } else {
        None
    };
    if options.command == "release-reconcile" {
        release::expected_origin(
            artifact
                .as_ref()
                .ok_or_else(|| SafeError::new("INVALID_RELEASE"))?,
        )?;
    }
    if options.is_plan() {
        let mut result = json!({"ok":true,"mode":"PLAN","command":options.command,"applicationCode":options.app().code(),"basePath":options.app().base_path(),"entryUrl":options.app().entry_url(),"runtimeMode":"TRUSTED","entryType":"HTML","environment":options.environment,"externalWrites":false,"permissions":options.app().permissions(),"grants":[],"credentialAuthority":"IDENTITY_AUTH_REGISTRY","activation":"SEPARATE_EXPLICIT_COMMAND"});
        if matches!(options.command.as_str(), "plan" | "bootstrap") {
            result["applicationAccessPolicy"] = json!({"allowedMemberTypes":["EMPLOYEE"]});
        }
        if let Some(artifact) = artifact {
            result["version"] = artifact["version"].clone();
        }
        if options.command == "register" {
            result["artifactBindingVerified"] = json!(options.artifact_file.is_some());
            result["remoteDigestStored"] = json!(false);
        }
        if let Some(id) = &options.release_id {
            result["releaseId"] = json!(id);
        }
        return Ok(result);
    }
    // Validate local grant input before loading credentials or making a request.
    if options.command == "grants-apply" {
        grants::plan(
            options.manifest_path()?,
            options.app_code,
            options.app_id.as_deref(),
        )?;
    }
    if options.command == "access-policy-save" {
        access_policy::plan(&options)?;
    }
    let api = ManagementApi::new(env, &options)?;
    let result = match options.command.as_str() {
        "inspect" => bootstrap::inspect(&api, &options),
        "bootstrap" => bootstrap::apply(&api, &options),
        "access-policy-read" | "access-policy-save" => access_policy::execute(&api, &options),
        "grants-apply" => grants::apply(
            options.manifest_path()?,
            &api,
            options.app_code,
            options.app_id.as_deref(),
        ),
        "retention-preview" => retention::execute(&api, &options),
        _ => release::execute(&api, &options, artifact.as_ref()),
    };
    // The response contract exposes safe identifiers and version summaries. Even
    // those strings must not echo the credential if a broken or hostile peer puts
    // it in an otherwise valid field. Client secrets are never read at all.
    if let Some(bearer) = env
        .get("PLATFORM_ADMIN_BEARER")
        .filter(|value| !value.is_empty())
    {
        let output = match &result {
            Ok(value) => value.clone(),
            Err(error) => error.to_json(),
        };
        if contains_credential(&output, bearer) {
            let error = SafeError::new("SENSITIVE_OUTPUT_REJECTED");
            return Err(
                if options.apply || result.as_ref().is_err_and(SafeError::outcome_unknown) {
                    error.uncertain()
                } else {
                    error
                },
            );
        }
    }
    result
}

fn contains_credential(value: &Value, bearer: &str) -> bool {
    match value {
        Value::String(value) => value.contains(bearer),
        Value::Array(values) => values
            .iter()
            .any(|value| contains_credential(value, bearer)),
        Value::Object(values) => values
            .values()
            .any(|value| contains_credential(value, bearer)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::contains_credential;
    use serde_json::json;

    #[test]
    fn credentials_cannot_hide_in_success_or_progress_values() {
        let credential = "BEARER_CANARY";
        assert!(contains_credential(
            &json!({"releases":[{"version":"prefix-BEARER_CANARY-suffix"}]}),
            credential
        ));
        assert!(contains_credential(
            &json!({"progress":{"completed":[credential]}}),
            credential
        ));
        assert!(!contains_credential(
            &json!({"version":"1.2.3","status":"DRAFT"}),
            credential
        ));
    }
}
