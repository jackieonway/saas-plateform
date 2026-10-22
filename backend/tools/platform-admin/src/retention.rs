//! Read-only preview projection. No caller can turn this command into apply.
use crate::{
    SafeError,
    bootstrap::find_app,
    contract::{is_environment, is_version, uuid_field},
    http::ManagementApi,
    options::Options,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(crate) fn execute(api: &ManagementApi, options: &Options) -> Result<Value, SafeError> {
    let invalid = || SafeError::new("INVALID_RETENTION_PREVIEW");
    let app = find_app(api, options.app_id.as_deref(), options.app())?
        .ok_or_else(|| SafeError::new("APPLICATION_NOT_FOUND"))?;
    let id = uuid_field(&app, "id", "APPLICATION_CONFLICT")?;
    let response = api.request(
        "GET",
        &format!(
            "/api/applications/{id}/releases/retention-preview?minimumAgeDays={}&keepLatest={}",
            options.minimum_age_days, options.keep_latest
        ),
        None,
    )?;
    if response["applicationId"] != id
        || response["mode"] != "DRY_RUN"
        || response["applyAvailable"] != false
        || response["deletionPermitted"] != false
        || response["policy"]["minimumAgeDays"] != options.minimum_age_days
        || response["policy"]["keepLatest"] != options.keep_latest
    {
        return Err(invalid());
    }
    let rows = response["records"]
        .as_array()
        .filter(|rows| rows.len() <= 5000)
        .ok_or_else(invalid)?;
    let mut ids = BTreeSet::new();
    let mut projected = Vec::new();
    let mut protected = 0;
    for row in rows {
        let release = uuid_field(row, "releaseId", "INVALID_RETENTION_PREVIEW")?;
        let version = row["version"]
            .as_str()
            .filter(|v| is_version(v))
            .ok_or_else(invalid)?;
        let environment = row["environment"]
            .as_str()
            .filter(|v| is_environment(v))
            .ok_or_else(invalid)?;
        if !ids.insert(release.to_owned()) {
            return Err(invalid());
        }
        let reasons = row["reasons"].as_array().ok_or_else(invalid)?;
        for reason in reasons {
            if !matches!(
                reason.as_str(),
                Some(
                    "FUTURE_TIMESTAMP"
                        | "MINIMUM_AGE"
                        | "LATEST_VERSIONS"
                        | "ACTIVE"
                        | "PREVIOUS"
                        | "IN_FLIGHT_OPERATION"
                        | "EXTERNAL_OWNERSHIP_REQUIRES_REVIEW"
                        | "ARTIFACT_IDENTITY_UNKNOWN"
                        | "CROSS_ENVIRONMENT_REFERENCE"
                        | "ACTIVE_REFERENCE"
                        | "PREVIOUS_REFERENCE"
                        | "RETAINED_VERSION_REFERENCE"
                        | "SHARED_ARTIFACT_REFERENCE"
                )
            ) {
                return Err(invalid());
            }
        }
        let disposition = if reasons.is_empty() {
            "MANUAL_REVIEW"
        } else {
            protected += 1;
            "PROTECTED"
        };
        if row["disposition"] != disposition
            || !matches!(row["artifactKind"].as_str(), Some("ZIP" | "EXTERNAL_URL"))
            || row["artifactKind"] == "EXTERNAL_URL"
                && !reasons
                    .iter()
                    .any(|reason| reason == "EXTERNAL_OWNERSHIP_REQUIRES_REVIEW")
        {
            return Err(invalid());
        }
        projected.push(json!({"releaseId":release,"version":version,"environment":environment,"artifactKind":row["artifactKind"],"disposition":disposition,"reasons":reasons}));
    }
    if response["total"] != rows.len()
        || response["protected"] != protected
        || response["manualReview"] != rows.len() - protected
    {
        return Err(invalid());
    }
    Ok(
        json!({"ok":true,"command":"retention-preview","mode":"DRY_RUN","applicationId":id,"policy":{"minimumAgeDays":options.minimum_age_days,"keepLatest":options.keep_latest,"approval":"ISOLATED_CANDIDATE_ONLY"},"applyAvailable":false,"deletionPermitted":false,"total":rows.len(),"protected":protected,"manualReview":rows.len()-protected,"records":projected}),
    )
}
