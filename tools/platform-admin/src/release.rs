use crate::{
    SafeError,
    bootstrap::find_app,
    contract::{is_environment, is_version, read_json, rows, uuid_field},
    http::{ManagementApi, is_loopback},
    options::Options,
};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Release {
    environment: String,
    version: String,
    entry_type: String,
    entry_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    health_url: Option<String>,
    resources: Vec<Resource>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Resource {
    url: String,
    kind: String,
}
pub(crate) fn safe_url(raw: &str, base: &Url, options: &Options) -> Result<Url, SafeError> {
    if raw.is_empty()
        || raw.chars().count() > 2000
        || raw.starts_with("//")
        || raw
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control() || matches!(ch, '\\' | '?' | '#'))
    {
        return Err(SafeError::new("INVALID_RELEASE"));
    }
    // Inspect raw path components before Url can normalize traversal. A single
    // leading ./ is allowed for the packager's ordinary relative resources.
    for segment in raw.split('/') {
        if segment == ".." {
            return Err(SafeError::new("INVALID_RELEASE"));
        }
        if segment.contains('%') {
            let source = segment.as_bytes();
            let mut decoded = Vec::new();
            let mut index = 0;
            while index < source.len() {
                if source[index] == b'%' {
                    let high = source
                        .get(index + 1)
                        .and_then(|value| (*value as char).to_digit(16));
                    let low = source
                        .get(index + 2)
                        .and_then(|value| (*value as char).to_digit(16));
                    match (high, low) {
                        (Some(high), Some(low)) => {
                            decoded.push((high * 16 + low) as u8);
                            index += 3;
                        }
                        _ => return Err(SafeError::new("INVALID_RELEASE")),
                    }
                } else {
                    decoded.push(source[index]);
                    index += 1;
                }
            }
            if decoded == b"."
                || decoded == b".."
                || decoded
                    .iter()
                    .any(|byte| matches!(byte, b'/' | b'\\' | b'%') || byte.is_ascii_control())
                || std::str::from_utf8(&decoded).is_err()
            {
                return Err(SafeError::new("INVALID_RELEASE"));
            }
        }
    }
    let url = base
        .join(raw)
        .map_err(|_| SafeError::new("INVALID_RELEASE"))?;
    if url.as_str().chars().count() > 2000
        || !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(SafeError::new("INVALID_RELEASE"));
    }
    if url.scheme() != "https"
        && !(options.environment == "development"
            && options.allow_loopback_http
            && is_loopback(&url))
    {
        return Err(SafeError::new("HTTPS_REQUIRED"));
    }
    Ok(url)
}
pub(crate) fn read(options: &Options) -> Result<Value, SafeError> {
    let path = options
        .release_file
        .as_deref()
        .ok_or_else(|| SafeError::new("INVALID_ARGUMENT"))?;
    validate(
        read_json(Path::new(path), "INVALID_RELEASE")?,
        options,
        None,
    )
}

/// Normalize only the existing URL-registration fields. Artifact metadata stays
/// in the local journal and never enters the platform request body.
pub(crate) fn validate(
    value: Value,
    options: &Options,
    origin: Option<&str>,
) -> Result<Value, SafeError> {
    let mut release: Release =
        serde_json::from_value(value).map_err(|_| SafeError::new("INVALID_RELEASE"))?;
    if release.environment != options.environment
        || !is_environment(&release.environment)
        || !is_version(&release.version)
        || release.entry_type != "HTML"
        || release.resources.len() > 48
    {
        return Err(SafeError::new("INVALID_RELEASE"));
    }
    let synthetic = Url::parse(origin.unwrap_or("https://release.invalid/"))
        .map_err(|_| SafeError::new("INVALID_RELEASE"))?;
    let entry = safe_url(&release.entry_url, &synthetic, options)?;
    let prefix = format!(
        "/sub-apps/{}/versions/{}/",
        options.app().code(),
        release.version
    );
    if origin.is_some() && entry.origin() != synthetic.origin()
        || !entry.path().starts_with(&prefix)
        || !(entry.path().ends_with(".html") || entry.path().ends_with(".htm"))
    {
        return Err(SafeError::new("INVALID_RELEASE"));
    }
    if let Some(health) = &mut release.health_url {
        let url = safe_url(
            health,
            &entry
                .origin()
                .ascii_serialization()
                .parse()
                .map_err(|_| SafeError::new("INVALID_RELEASE"))?,
            options,
        )?;
        if origin.is_some() {
            *health = url.to_string();
        }
    }
    let mut seen = BTreeSet::new();
    for resource in &mut release.resources {
        if !matches!(resource.kind.as_str(), "SCRIPT" | "STYLE" | "ASSET") {
            return Err(SafeError::new("INVALID_RELEASE"));
        }
        let url = safe_url(&resource.url, &entry, options)?;
        if !url.path().starts_with(&prefix)
            || url.origin() != entry.origin()
            || url == entry
            || !seen.insert(url.as_str().to_owned())
        {
            return Err(SafeError::new("INVALID_RELEASE"));
        }
        if origin.is_some() {
            resource.url = url.to_string();
        }
    }
    if origin.is_some() {
        release.entry_url = entry.to_string();
        release
            .resources
            .sort_by(|a, b| (&a.url, &a.kind).cmp(&(&b.url, &b.kind)));
    }
    serde_json::to_value(release).map_err(|_| SafeError::new("INVALID_RELEASE"))
}
fn summary(row: &Value, app_id: &str, environment: &str) -> Result<Value, SafeError> {
    let id = uuid_field(row, "id", "INVALID_CONTRACT")?;
    if row["applicationId"] != app_id
        || row["environment"] != environment
        || row["entryType"] != "HTML"
        || !row["version"].as_str().is_some_and(is_version)
        || !matches!(
            row["validationStatus"].as_str(),
            Some("DRAFT" | "PASSED" | "FAILED")
        )
    {
        return Err(SafeError::new("INVALID_CONTRACT"));
    }
    Ok(
        json!({"id":id,"environment":environment,"version":row["version"],"validationStatus":row["validationStatus"],"active":row["active"]==true}),
    )
}
fn deployment(row: &Value, app_id: &str) -> Result<Value, SafeError> {
    if row["applicationId"] != app_id
        || row["entryType"] != "HTML"
        || !row["environment"].as_str().is_some_and(is_environment)
        || !row["version"].as_str().is_some_and(is_version)
    {
        return Err(SafeError::new("INVALID_CONTRACT"));
    }
    let active = uuid_field(row, "activeReleaseId", "INVALID_CONTRACT")?;
    let previous = if row.get("previousReleaseId").is_some_and(Value::is_null) {
        None
    } else {
        Some(uuid_field(row, "previousReleaseId", "INVALID_CONTRACT")?)
    };
    Ok(
        json!({"applicationId":app_id,"environment":row["environment"],"activeReleaseId":active,"previousReleaseId":previous,"version":row["version"]}),
    )
}
fn definition(row: &Value, options: &Options, origin: &str) -> Result<Value, SafeError> {
    // A manifest proves our expected origin, not the server's gateway setting.
    // Never reinterpret a remote root entry/health as the expected origin.
    expected_origin(row)?;
    let mut projected = json!({"environment":row["environment"],"version":row["version"],"entryType":row["entryType"],"entryUrl":row["entryUrl"],"resources":row["resources"]});
    if !row["healthUrl"].is_null() {
        projected["healthUrl"] = row["healthUrl"].clone();
    }
    validate(projected, options, Some(origin))
}
pub(crate) fn expected_origin(expected: &Value) -> Result<String, SafeError> {
    let entry = expected["entryUrl"]
        .as_str()
        .and_then(|raw| Url::parse(raw).ok())
        .ok_or_else(|| SafeError::new("RECONCILIATION_ORIGIN_REQUIRED"))?;
    // The server resolves health against MICRO_GATEWAY_ORIGIN, which can differ
    // from both the management API and the absolute entry's origin. A local
    // artifact binding supplies that explicit public origin and has already
    // normalized the expected fields; unpaired relative health is ambiguous.
    if expected["healthUrl"]
        .as_str()
        .is_some_and(|raw| Url::parse(raw).is_err())
    {
        return Err(SafeError::new("RECONCILIATION_ORIGIN_REQUIRED"));
    }
    Ok(entry.origin().ascii_serialization())
}
pub(crate) fn execute(
    api: &ManagementApi,
    options: &Options,
    artifact: Option<&Value>,
) -> Result<Value, SafeError> {
    let app = find_app(api, options.app_id.as_deref(), options.app())?
        .ok_or_else(|| SafeError::new("APPLICATION_NOT_FOUND"))?;
    let app_id = uuid_field(&app, "id", "APPLICATION_CONFLICT")?;
    let base = format!("/api/applications/{app_id}");
    let environment = &options.environment;
    if matches!(options.command.as_str(), "deployments" | "rollback") {
        let response = api.request("GET", &format!("{base}/deployments"), None)?;
        let summaries = rows(&response)?
            .iter()
            .map(|row| deployment(row, app_id))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|row| row["environment"] == *environment)
            .collect::<Vec<_>>();
        if options.command == "deployments" {
            return Ok(json!({"ok":true,"command":"deployments","deployments":summaries}));
        }
        if summaries.len() != 1 || summaries[0]["previousReleaseId"].is_null() {
            return Err(SafeError::new("ROLLBACK_UNAVAILABLE"));
        }
        let response = api.request(
            "POST",
            &format!("{base}/deployments/{environment}/rollback"),
            None,
        )?;
        let result = deployment(&response, app_id).map_err(SafeError::uncertain)?;
        if result["environment"] != *environment
            || result["activeReleaseId"] != summaries[0]["previousReleaseId"]
            || result["previousReleaseId"] != summaries[0]["activeReleaseId"]
        {
            return Err(SafeError::new("DEPLOYMENT_CONFLICT").uncertain());
        }
        return Ok(json!({"ok":true,"mode":"APPLIED","command":"rollback","deployment":result}));
    }
    let response = api.request(
        "GET",
        &format!("{base}/releases?environment={environment}"),
        None,
    )?;
    let releases = rows(&response)?
        .iter()
        .map(|row| summary(row, app_id, environment))
        .collect::<Result<Vec<_>, _>>()?;
    let mut unique = BTreeSet::new();
    let mut versions = BTreeSet::new();
    if releases.iter().any(|row| {
        !unique.insert(row["id"].clone().to_string())
            || !versions.insert(row["version"].clone().to_string())
    }) {
        return Err(SafeError::new("INVALID_CONTRACT"));
    }
    if options.command == "releases" {
        return Ok(
            json!({"ok":true,"command":"releases","applicationId":app_id,"releases":releases}),
        );
    }
    if options.command == "release-reconcile" {
        let expected = artifact.ok_or_else(|| SafeError::new("INVALID_RELEASE"))?;
        let origin = expected_origin(expected)?;
        let expected = validate(expected.clone(), options, Some(&origin))?;
        let Some((index, target)) = releases
            .iter()
            .enumerate()
            .find(|(_, row)| row["version"] == expected["version"])
        else {
            return Ok(
                json!({"ok":true,"command":"release-reconcile","applicationId":app_id,"reconciliation":"ABSENT","externalWrites":false,"remoteDigestStored":false}),
            );
        };
        let actual = definition(&rows(&response)?[index], options, &origin);
        if actual
            .as_ref()
            .is_err_and(|error| error.category() == "RECONCILIATION_ORIGIN_REQUIRED")
        {
            return Err(SafeError::new("RECONCILIATION_ORIGIN_REQUIRED"));
        }
        if !actual.as_ref().is_ok_and(|value| value == &expected) {
            return Err(SafeError::new("RELEASE_DEFINITION_CONFLICT").with_completed(json!({"reconciliation":"CONFLICT","applicationId":app_id,"release":target,"externalWrites":false,"remoteDigestStored":false})));
        }
        return Ok(
            json!({"ok":true,"command":"release-reconcile","applicationId":app_id,"reconciliation":"MATCHED","release":target,"externalWrites":false,"remoteDigestStored":false}),
        );
    }
    if options.command == "register" {
        let artifact = artifact.ok_or_else(|| SafeError::new("INVALID_RELEASE"))?;
        if releases
            .iter()
            .any(|row| row["version"] == artifact["version"])
        {
            return Err(SafeError::new("VERSION_EXISTS"));
        }
        let response = api.request("POST", &format!("{base}/releases"), Some(artifact))?;
        let result = summary(&response, app_id, environment).map_err(SafeError::uncertain)?;
        if result["version"] != artifact["version"] || result["validationStatus"] != "DRAFT" {
            return Err(SafeError::new("INVALID_CONTRACT").uncertain());
        }
        if options.artifact_file.is_some() {
            let origin = expected_origin(artifact)?;
            let actual = definition(&response, options, &origin);
            if actual
                .as_ref()
                .is_err_and(|error| error.category() == "RECONCILIATION_ORIGIN_REQUIRED")
            {
                return Err(SafeError::new("RECONCILIATION_ORIGIN_REQUIRED").uncertain());
            }
            if !actual.as_ref().is_ok_and(|value| value == artifact) {
                return Err(SafeError::new("INVALID_CONTRACT").uncertain());
            }
        }
        return Ok(
            json!({"ok":true,"mode":"APPLIED","command":"register","release":result,"releaseActivated":false,"artifactBindingVerified":options.artifact_file.is_some(),"remoteDigestStored":false}),
        );
    }
    let target = releases
        .iter()
        .find(|row| row["id"].as_str() == options.release_id.as_deref())
        .ok_or_else(|| SafeError::new("RELEASE_NOT_FOUND"))?;
    if options.command == "activate" && target["validationStatus"] != "PASSED" {
        return Err(SafeError::new("PREFLIGHT_REQUIRED"));
    }
    let response = api.request(
        "POST",
        &format!(
            "{base}/releases/{}/{}",
            target["id"]
                .as_str()
                .ok_or_else(|| SafeError::new("INVALID_CONTRACT"))?,
            options.command
        ),
        None,
    )?;
    if options.command == "preflight" {
        let result = summary(&response, app_id, environment).map_err(SafeError::uncertain)?;
        if result["id"] != target["id"] {
            return Err(SafeError::new("INVALID_CONTRACT").uncertain());
        }
        if result["validationStatus"] != "PASSED" {
            return Err(SafeError::new("PREFLIGHT_FAILED"));
        }
        return Ok(
            json!({"ok":true,"mode":"APPLIED","command":"preflight","release":result,"releaseActivated":false}),
        );
    }
    let result = deployment(&response, app_id).map_err(SafeError::uncertain)?;
    if result["environment"] != *environment
        || result["activeReleaseId"] != target["id"]
        || result["version"] != target["version"]
    {
        return Err(SafeError::new("DEPLOYMENT_CONFLICT").uncertain());
    }
    Ok(json!({"ok":true,"mode":"APPLIED","command":"activate","deployment":result}))
}
