use crate::{
    SafeError,
    contract::{is_uuid, read_json},
    http::ManagementApi,
    options::Options,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Target {
    Application,
}
impl Target {
    fn name(self) -> &'static str {
        match self {
            Self::Application => "application",
        }
    }
    fn path(self, id: &str) -> String {
        let resource = match self {
            Self::Application => "applications",
        };
        format!("/api/{resource}/{id}/access-policy")
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    target: Target,
    id: String,
    allowed_member_types: Vec<String>,
    access_policy_version: i64,
    reason: String,
}

fn valid_types(types: &[String], allow_empty: bool) -> bool {
    (allow_empty || !types.is_empty())
        && types.len() <= 2
        && types
            .iter()
            .all(|value| matches!(value.as_str(), "EMPLOYEE" | "CUSTOMER"))
        && (types.len() < 2 || types[0] != types[1])
}

fn manifest(options: &Options) -> Result<Manifest, SafeError> {
    let invalid = || SafeError::new("INVALID_ACCESS_POLICY_MANIFEST");
    let value = read_json(options.manifest_path()?, "INVALID_ACCESS_POLICY_MANIFEST")?;
    if value.get("subjectMode").is_some() {
        return Err(invalid());
    }
    let mut input: Manifest = serde_json::from_value(value).map_err(|_| invalid())?;
    if !is_uuid(&input.id)
        || input.access_policy_version < 1
        || input.reason.trim().is_empty()
        || input.reason.trim().chars().count() > 500
        || input.reason.chars().any(char::is_control)
        || !valid_types(&input.allowed_member_types, false)
    {
        return Err(invalid());
    }
    input.id.make_ascii_lowercase();
    input.allowed_member_types.sort();
    input.reason = input.reason.trim().to_owned();
    Ok(input)
}

fn desired(input: &Manifest) -> Value {
    json!({"id":input.id,"allowedMemberTypes":input.allowed_member_types,"accessPolicyVersion":input.access_policy_version})
}

pub(crate) fn plan(options: &Options) -> Result<Value, SafeError> {
    let input = manifest(options)?;
    Ok(
        json!({"ok":true,"mode":"PLAN","command":"access-policy-save","target":input.target.name(),"desired":desired(&input),"reasonProvided":true,"externalWrites":false,"expectedVersionRequired":true}),
    )
}

// Project the policy contract only: management responses may contain additional
// fields, including client secrets, which must never be printed by this command.
fn project(value: &Value, _target: Target, id: &str) -> Result<Value, SafeError> {
    let invalid = || SafeError::new("INVALID_ACCESS_POLICY_RESPONSE");
    let types = value
        .get("allowedMemberTypes")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let mut types: Vec<String> = types
        .iter()
        .map(|value| value.as_str().map(str::to_owned).ok_or_else(invalid))
        .collect::<Result<_, _>>()?;
    let version = value
        .get("accessPolicyVersion")
        .and_then(Value::as_i64)
        .filter(|version| *version > 0)
        .ok_or_else(invalid)?;
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .filter(|status| matches!(*status, "ENABLED" | "DISABLED"))
        .ok_or_else(invalid)?;
    if value["id"] != id || !valid_types(&types, false) || value.get("subjectMode").is_some() {
        return Err(invalid());
    }
    types.sort();
    let output =
        json!({"id":id,"allowedMemberTypes":types,"accessPolicyVersion":version,"status":status});
    Ok(output)
}

pub(crate) fn execute(api: &ManagementApi, options: &Options) -> Result<Value, SafeError> {
    if options.command == "access-policy-read" {
        let target = match options.policy_target.as_deref() {
            Some("application") => Target::Application,
            _ => return Err(SafeError::new("INVALID_ARGUMENT")),
        };
        let id = options
            .policy_id
            .as_deref()
            .ok_or_else(|| SafeError::new("INVALID_ARGUMENT"))?
            .to_ascii_lowercase();
        let policy = project(&api.request("GET", &target.path(&id), None)?, target, &id)?;
        return Ok(json!({"ok":true,"mode":"READ_ONLY","target":target.name(),"policy":policy}));
    }
    let input = manifest(options)?;
    let path = input.target.path(&input.id);
    let current = project(&api.request("GET", &path, None)?, input.target, &input.id)?;
    if current["accessPolicyVersion"] != input.access_policy_version {
        return Err(SafeError::http("ACCESS_POLICY_CONFLICT", Some(409), false));
    }
    let changed = current["allowedMemberTypes"] != json!(input.allowed_member_types);
    let expected_version = input
        .access_policy_version
        .checked_add(i64::from(changed))
        .ok_or_else(|| SafeError::new("ACCESS_POLICY_VERSION_EXHAUSTED"))?;
    let mut body = desired(&input);
    body.as_object_mut()
        .ok_or_else(|| SafeError::new("INVALID_ACCESS_POLICY_MANIFEST"))?
        .remove("id");
    body["reason"] = json!(input.reason);
    let result = api.request("PUT", &path, Some(&body)).map_err(|error| {
        if error.status() == Some(409) {
            SafeError::http("ACCESS_POLICY_CONFLICT", Some(409), false)
        } else {
            error
        }
    })?;
    let saved = project(&result, input.target, &input.id).map_err(SafeError::uncertain)?;
    if saved["allowedMemberTypes"] != json!(input.allowed_member_types)
        || saved["accessPolicyVersion"] != expected_version
    {
        return Err(SafeError::new("ACCESS_POLICY_RESULT_CONFLICT").uncertain());
    }
    Ok(
        json!({"ok":true,"mode":"APPLIED","command":"access-policy-save","target":input.target.name(),"policy":saved,"changed":changed}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_projection_requires_application_policy_and_never_exposes_secrets() {
        let id = "11111111-1111-7111-8111-111111111111";
        let mut value = json!({"id":id,"allowedMemberTypes":["EMPLOYEE"],"accessPolicyVersion":1,"status":"ENABLED","secret":"NEVER_PRINT"});
        assert!(
            !project(&value, Target::Application, id)
                .unwrap()
                .to_string()
                .contains("NEVER_PRINT")
        );
        value["subjectMode"] = json!("USER");
        assert!(project(&value, Target::Application, id).is_err());
        value.as_object_mut().unwrap().remove("subjectMode");
        for types in [
            json!([]),
            json!(["EMPLOYEE", "EMPLOYEE"]),
            json!(["UNKNOWN"]),
        ] {
            value["allowedMemberTypes"] = types;
            assert!(project(&value, Target::Application, id).is_err());
        }
        value["allowedMemberTypes"] = json!(["EMPLOYEE"]);
        value["accessPolicyVersion"] = json!(0);
        assert!(project(&value, Target::Application, id).is_err());
    }
}
