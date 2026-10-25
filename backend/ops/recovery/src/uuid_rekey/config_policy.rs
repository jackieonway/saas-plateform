//! Closed DATA_GRANT_POLICY owner contract for offline configuration rekeying.
//!
//! Validation does not normalize or serialize the supplied value. Only the
//! caller's registered UUID pointers may change the original configuration.

use std::collections::HashSet;

use serde::Deserialize;
use serde_json::Value;
use uuid::{Uuid, Variant};

use crate::{Error, Result};

const MAX_POLICY_BYTES: usize = 4 * 1024 * 1024;
const MAX_APPLICATIONS: usize = 64;
const MAX_ENTRIES: usize = 128;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Policy {
    applications: Vec<ApplicationPolicy>,
    management_scopes: Vec<ManagementScope>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApplicationPolicy {
    application_id: String,
    capabilities: Vec<Capability>,
    #[serde(default)]
    member_lookups: Vec<MemberLookup>,
    #[serde(default)]
    organization_lookups: Vec<OrganizationLookup>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Capability {
    resource: String,
    action: String,
    scopes: Vec<GrantScope>,
}

#[derive(Clone, Copy, Deserialize, Eq, Hash, PartialEq)]
enum GrantScope {
    #[serde(rename = "SELF")]
    SelfOnly,
    #[serde(rename = "ORG")]
    Org,
    #[serde(rename = "ORG_SUBTREE")]
    OrgSubtree,
    #[serde(rename = "TENANT")]
    Tenant,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MemberLookup {
    resource: String,
    action: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OrganizationLookup {
    resource: String,
    action: String,
    organization_type: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManagementScope {
    tenant_id: String,
    organization_id: String,
    include_descendants: bool,
    read: bool,
    write: bool,
}

/// Source configuration accepts canonical v4/v7 identities; converted
/// configuration must satisfy the owner's strict UUIDv7 boundary.
pub(super) fn validate(value: &Value, target: bool) -> Result<()> {
    let bytes =
        serde_json::to_vec(value).map_err(|_| Error("data-grant policy serialization failed"))?;
    if bytes.len() > MAX_POLICY_BYTES {
        return Err(Error("data-grant policy exceeds the supported size"));
    }
    let policy: Policy = serde_json::from_value(value.clone())
        .map_err(|_| Error("data-grant policy does not match its closed owner schema"))?;
    if policy.applications.len() > MAX_APPLICATIONS || policy.management_scopes.len() > MAX_ENTRIES
    {
        return Err(Error("data-grant policy exceeds owner collection limits"));
    }
    let mut applications = HashSet::new();
    for app in &policy.applications {
        let application_id = identity(&app.application_id, target)?;
        if !applications.insert(application_id)
            || app.capabilities.len() > MAX_ENTRIES
            || app.member_lookups.len() > MAX_ENTRIES
            || app.organization_lookups.len() > MAX_ENTRIES
        {
            return Err(Error(
                "data-grant policy application identities or collection limits are invalid",
            ));
        }
        let mut capabilities = HashSet::new();
        for capability in &app.capabilities {
            if !valid_identifier(&capability.resource)
                || !valid_identifier(&capability.action)
                || !capabilities.insert((capability.resource.as_str(), capability.action.as_str()))
                || capability.scopes.is_empty()
                || capability.scopes.len() > 4
            {
                return Err(Error("data-grant policy capability declaration is invalid"));
            }
            let mut scopes = HashSet::new();
            if !capability.scopes.iter().all(|scope| scopes.insert(*scope)) {
                return Err(Error("data-grant policy capability scopes are duplicated"));
            }
        }
        let mut lookups = HashSet::new();
        for lookup in &app.member_lookups {
            let pair = (lookup.resource.as_str(), lookup.action.as_str());
            if !valid_identifier(&lookup.resource)
                || !valid_identifier(&lookup.action)
                || !lookups.insert(pair)
                || !capabilities.contains(&pair)
            {
                return Err(Error(
                    "data-grant policy member lookup is not a unique same-application capability",
                ));
            }
        }
        let mut organization_lookups = HashSet::new();
        for lookup in &app.organization_lookups {
            let pair = (lookup.resource.as_str(), lookup.action.as_str());
            if lookup.organization_type != "STORE"
                || !valid_identifier(&lookup.resource)
                || !valid_identifier(&lookup.action)
                || !organization_lookups.insert(pair)
                || !capabilities.contains(&pair)
            {
                return Err(Error(
                    "data-grant policy organization lookup is not a unique STORE capability",
                ));
            }
        }
    }
    let mut boundaries = HashSet::new();
    for boundary in &policy.management_scopes {
        let pair = (
            identity(&boundary.tenant_id, target)?,
            identity(&boundary.organization_id, target)?,
        );
        if !boundaries.insert(pair) {
            return Err(Error(
                "data-grant policy management boundaries are duplicated",
            ));
        }
        // These fields have no additional semantic restrictions. Their exact
        // boolean type is nevertheless part of the closed owner contract.
        let _ = (boundary.include_descendants, boundary.read, boundary.write);
    }
    Ok(())
}

fn identity(value: &str, target: bool) -> Result<Uuid> {
    let id = Uuid::parse_str(value)
        .map_err(|_| Error("data-grant policy identity is not a supported canonical UUID"))?;
    let version = id.get_version_num();
    if id.is_nil()
        || id.get_variant() != Variant::RFC4122
        || id.to_string() != value
        || (target && version != 7)
        || (!target && !matches!(version, 4 | 7))
    {
        return Err(Error(
            "data-grant policy identity is not a supported canonical UUID",
        ));
    }
    Ok(id)
}

fn valid_identifier(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 100
        && bytes[0].is_ascii_lowercase()
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn id(serial: u64) -> String {
        format!("01900000-0000-7000-8000-{serial:012x}")
    }

    fn fixture() -> Value {
        json!({
            "applications": [{
                "applicationId": id(1),
                "capabilities": [{"resource":"ticket","action":"read","scopes":["ORG","SELF"]}],
                "memberLookups": [{"resource":"ticket","action":"read"}],
                "organizationLookups": [{"resource":"ticket","action":"read","organizationType":"STORE"}]
            }],
            "managementScopes": [{
                "tenantId":id(2),"organizationId":id(3),
                "includeDescendants":false,"read":true,"write":false
            }]
        })
    }

    #[test]
    fn validation_preserves_original_values_and_order() {
        let value = fixture();
        let original = value.clone();
        validate(&value, false).unwrap();
        validate(&value, true).unwrap();
        assert_eq!(value, original);
        let mut old_policy = fixture();
        old_policy["applications"][0]
            .as_object_mut()
            .unwrap()
            .remove("memberLookups");
        old_policy["applications"][0]
            .as_object_mut()
            .unwrap()
            .remove("organizationLookups");
        validate(&old_policy, true).unwrap();
        validate(&json!({"applications":[],"managementScopes":[]}), true).unwrap();
    }

    #[test]
    fn source_v4_is_accepted_but_target_v4_and_noncanonical_ids_are_rejected() {
        let mut value = fixture();
        value["applications"][0]["applicationId"] = json!("550e8400-e29b-41d4-a716-446655440000");
        validate(&value, false).unwrap();
        assert!(validate(&value, true).is_err());
        for invalid in [
            "550E8400-E29B-41D4-A716-446655440000",
            "550e8400e29b41d4a716446655440000",
            "00000000-0000-0000-0000-000000000000",
            "01900000-0000-7000-0000-000000000001",
            "550e8400-e29b-11d4-a716-446655440000",
        ] {
            value["applications"][0]["applicationId"] = json!(invalid);
            assert!(validate(&value, false).is_err());
        }
    }

    #[test]
    fn closed_schema_rejects_unknown_fields_at_every_owner_level_and_redacts_errors() {
        for pointer in [
            "",
            "/applications/0",
            "/applications/0/capabilities/0",
            "/applications/0/memberLookups/0",
            "/applications/0/organizationLookups/0",
            "/managementScopes/0",
        ] {
            let mut value = fixture();
            value.pointer_mut(pointer).unwrap()["untrusted-secret-key"] = json!("secret-value");
            let error = validate(&value, true).unwrap_err().to_string();
            assert!(!error.contains("untrusted-secret-key"));
            assert!(!error.contains("secret-value"));
        }
        for pointer in ["/applications", "/managementScopes"] {
            let mut value = fixture();
            *value.pointer_mut(pointer).unwrap() = Value::Null;
            assert!(validate(&value, true).is_err());
        }
        let mut value = fixture();
        value["managementScopes"][0]["read"] = json!("true");
        assert!(validate(&value, true).is_err());
    }

    #[test]
    fn owner_collection_limits_accept_maximum_and_reject_overflow() {
        let mut value = fixture();
        value["applications"] = json!((1..=MAX_APPLICATIONS)
            .map(|serial| json!({"applicationId":id(serial as u64),"capabilities":[]}))
            .collect::<Vec<_>>());
        validate(&value, true).unwrap();
        value["applications"].as_array_mut().unwrap().push(json!({
            "applicationId":id(1000),"capabilities":[]
        }));
        assert!(validate(&value, true).is_err());

        let mut value = fixture();
        value["applications"][0]["capabilities"] = json!((0..MAX_ENTRIES)
            .map(|serial| json!({"resource":format!("record-{serial}"),"action":"read","scopes":["SELF"]}))
            .collect::<Vec<_>>());
        for field in ["memberLookups", "organizationLookups"] {
            value["applications"][0][field] = json!((0..MAX_ENTRIES)
                .map(|serial| {
                    let mut lookup = json!({"resource":format!("record-{serial}"),"action":"read"});
                    if field == "organizationLookups" {
                        lookup["organizationType"] = json!("STORE");
                    }
                    lookup
                })
                .collect::<Vec<_>>());
        }
        validate(&value, true).unwrap();
        for field in ["capabilities", "memberLookups", "organizationLookups"] {
            let mut overflow = value.clone();
            let extra = overflow["applications"][0][field][0].clone();
            overflow["applications"][0][field]
                .as_array_mut()
                .unwrap()
                .push(extra);
            assert!(validate(&overflow, true).is_err());
        }
        value["managementScopes"] = json!((1..=MAX_ENTRIES)
            .map(|serial| json!({
                "tenantId":id(2),"organizationId":id(serial as u64),
                "includeDescendants":false,"read":true,"write":false
            }))
            .collect::<Vec<_>>());
        validate(&value, true).unwrap();
        value["managementScopes"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "tenantId":id(2),"organizationId":id(1000),
                "includeDescendants":false,"read":true,"write":false
            }));
        assert!(validate(&value, true).is_err());
    }

    #[test]
    fn lookup_registration_is_unique_and_bound_to_its_own_application() {
        for field in ["memberLookups", "organizationLookups"] {
            let mut value = fixture();
            value["applications"][0][field][0]["action"] = json!("write");
            let mut other = fixture()["applications"][0].clone();
            other["applicationId"] = json!(id(4));
            other["capabilities"][0]["action"] = json!("write");
            other["memberLookups"] = json!([]);
            other["organizationLookups"] = json!([]);
            value["applications"].as_array_mut().unwrap().push(other);
            assert!(validate(&value, true).is_err());

            let mut value = fixture();
            let duplicate = value["applications"][0][field][0].clone();
            value["applications"][0][field]
                .as_array_mut()
                .unwrap()
                .push(duplicate);
            assert!(validate(&value, true).is_err());
        }
        let mut value = fixture();
        value["applications"][0]["organizationLookups"][0]["organizationType"] = json!("COMPANY");
        assert!(validate(&value, true).is_err());
    }

    #[test]
    fn duplicate_identities_capabilities_boundaries_and_scopes_are_rejected() {
        for pointer in [
            "/applications",
            "/applications/0/capabilities",
            "/managementScopes",
        ] {
            let mut value = fixture();
            let entries = value.pointer_mut(pointer).unwrap().as_array_mut().unwrap();
            entries.push(entries[0].clone());
            assert!(validate(&value, true).is_err());
        }
        let mut value = fixture();
        value["applications"][0]["capabilities"][0]["scopes"] = json!(["ORG", "ORG"]);
        assert!(validate(&value, true).is_err());
    }

    #[test]
    fn identifiers_and_scope_labels_match_the_exact_owner_contract() {
        for field in ["resource", "action"] {
            for invalid in ["", "Ticket", "1record", "a:b", "a.b", "a ", "中文"] {
                let mut value = fixture();
                value["applications"][0]["capabilities"][0][field] = json!(invalid);
                assert!(validate(&value, true).is_err());
            }
        }
        for invalid in [json!([]), json!(["org"]), json!(["ALL"]), json!([null])] {
            let mut value = fixture();
            value["applications"][0]["capabilities"][0]["scopes"] = invalid;
            assert!(validate(&value, true).is_err());
        }
        let mut value = fixture();
        value["applications"][0]["capabilities"][0]["scopes"] =
            json!(["TENANT", "ORG_SUBTREE", "SELF", "ORG"]);
        validate(&value, true).unwrap();
    }
}
