//! Server-owned data-grant capability and management-boundary configuration.
//!
//! Only startup configuration constructs the effective policy. A caller's own
//! business grants never contribute capabilities or management boundaries.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub(crate) const READ_PERMISSION: &str = "system:data-grant:read";
pub(crate) const WRITE_PERMISSION: &str = "system:data-grant:write";

const MAX_POLICY_BYTES: usize = 4 * 1024 * 1024;
const MAX_APPLICATIONS: usize = 64;
const MAX_CAPABILITIES: usize = 128;
const MAX_MEMBER_LOOKUPS: usize = 128;
const MAX_MANAGEMENT_SCOPES: usize = 128;

/// Scope labels have no privilege ordering. Authorization must explicitly test
/// configured membership and the runtime organization's boundary.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub(crate) enum GrantScope {
    #[serde(rename = "SELF")]
    SelfOnly,
    #[serde(rename = "ORG")]
    Org,
    #[serde(rename = "ORG_SUBTREE")]
    OrgSubtree,
    #[serde(rename = "TENANT")]
    Tenant,
}

impl GrantScope {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SelfOnly => "SELF",
            Self::Org => "ORG",
            Self::OrgSubtree => "ORG_SUBTREE",
            Self::Tenant => "TENANT",
        }
    }

    pub(crate) fn from_str_value(value: &str) -> Option<Self> {
        match value {
            "SELF" => Some(Self::SelfOnly),
            "ORG" => Some(Self::Org),
            "ORG_SUBTREE" => Some(Self::OrgSubtree),
            "TENANT" => Some(Self::Tenant),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Capability {
    pub(crate) resource: String,
    pub(crate) action: String,
    pub(crate) scopes: Vec<GrantScope>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ApplicationPolicy {
    pub(crate) application_id: Uuid,
    pub(crate) capabilities: Vec<Capability>,
    /// Assignment lookup is an explicit additional use of a registered pair.
    /// It does not grant directory access to every data-grant capability.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) member_lookups: Vec<MemberLookup>,
    /// Organization discovery is a separate, explicitly registered purpose.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) organization_lookups: Vec<OrganizationLookup>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OrganizationLookup {
    resource: String,
    action: String,
    organization_type: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MemberLookup {
    resource: String,
    action: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManagementScope {
    pub(crate) tenant_id: Uuid,
    pub(crate) organization_id: Uuid,
    pub(crate) include_descendants: bool,
    pub(crate) read: bool,
    pub(crate) write: bool,
}

/// Default is an empty allowlist. Explicit JSON requires both arrays and every
/// nested field, so a misspelled or omitted boundary cannot silently widen it.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Policy {
    pub(crate) applications: Vec<ApplicationPolicy>,
    pub(crate) management_scopes: Vec<ManagementScope>,
}

impl Policy {
    /// Parse and validate once during configuration loading. Error text never
    /// echoes submitted configuration values. Sorting produces deterministic
    /// serialization for policy versions without assigning scopes a hierarchy.
    pub(crate) fn from_json(raw: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(
            raw.len() <= MAX_POLICY_BYTES,
            "DATA_GRANT_POLICY exceeds the supported size"
        );
        let mut policy: Self = serde_json::from_str(raw).map_err(|_| {
            anyhow::anyhow!("DATA_GRANT_POLICY must match the strict policy JSON schema")
        })?;
        anyhow::ensure!(
            policy.applications.len() <= MAX_APPLICATIONS,
            "DATA_GRANT_POLICY permits at most 64 applications"
        );
        anyhow::ensure!(
            policy.management_scopes.len() <= MAX_MANAGEMENT_SCOPES,
            "DATA_GRANT_POLICY permits at most 128 management scopes"
        );
        let mut applications = HashSet::new();
        for app in &mut policy.applications {
            anyhow::ensure!(
                !app.application_id.is_nil() && applications.insert(app.application_id),
                "DATA_GRANT_POLICY application IDs must be non-nil and unique"
            );
            anyhow::ensure!(
                app.capabilities.len() <= MAX_CAPABILITIES,
                "DATA_GRANT_POLICY permits at most 128 capabilities per application"
            );
            let mut capabilities = HashSet::new();
            for capability in &mut app.capabilities {
                anyhow::ensure!(
                    valid_identifier(&capability.resource) && valid_identifier(&capability.action),
                    "DATA_GRANT_POLICY resources and actions must match [a-z][a-z0-9_-]{{0,99}}"
                );
                anyhow::ensure!(
                    capabilities.insert((capability.resource.clone(), capability.action.clone())),
                    "DATA_GRANT_POLICY resource/action pairs must be unique per application"
                );
                anyhow::ensure!(
                    !capability.scopes.is_empty() && capability.scopes.len() <= 4,
                    "DATA_GRANT_POLICY capabilities require between 1 and 4 scopes"
                );
                let mut scopes = HashSet::new();
                anyhow::ensure!(
                    capability.scopes.iter().all(|scope| scopes.insert(*scope)),
                    "DATA_GRANT_POLICY capability scopes must be unique"
                );
                // Lexical labels are a serialization convention, never a rank.
                capability
                    .scopes
                    .sort_unstable_by_key(|scope| scope.as_str());
            }
            app.capabilities.sort_unstable_by(|left, right| {
                (&left.resource, &left.action).cmp(&(&right.resource, &right.action))
            });
            anyhow::ensure!(
                app.member_lookups.len() <= MAX_MEMBER_LOOKUPS,
                "DATA_GRANT_POLICY permits at most 128 assignment member lookups per application"
            );
            let mut lookups = HashSet::new();
            for lookup in &app.member_lookups {
                anyhow::ensure!(
                    valid_identifier(&lookup.resource) && valid_identifier(&lookup.action)
                        && lookups.insert((lookup.resource.as_str(), lookup.action.as_str()))
                        && app.capabilities.iter().any(|capability| capability.resource == lookup.resource && capability.action == lookup.action),
                    "DATA_GRANT_POLICY assignment member lookups must be unique registered resource/action pairs in the same application"
                );
            }
            app.member_lookups.sort_unstable_by(|left, right| {
                (&left.resource, &left.action).cmp(&(&right.resource, &right.action))
            });
            anyhow::ensure!(
                app.organization_lookups.len() <= MAX_MEMBER_LOOKUPS,
                "DATA_GRANT_POLICY permits at most 128 organization lookups per application"
            );
            let mut organization_lookups = HashSet::new();
            for lookup in &app.organization_lookups {
                anyhow::ensure!(lookup.organization_type == "STORE"
                    && valid_identifier(&lookup.resource) && valid_identifier(&lookup.action)
                    && organization_lookups.insert((lookup.resource.as_str(),lookup.action.as_str()))
                    && app.capabilities.iter().any(|capability| capability.resource == lookup.resource && capability.action == lookup.action),
                    "DATA_GRANT_POLICY organization lookups require unique same-application capabilities and STORE type");
            }
            app.organization_lookups.sort_unstable_by(|left, right| {
                (&left.resource, &left.action).cmp(&(&right.resource, &right.action))
            });
        }
        let mut boundaries = HashSet::new();
        for boundary in &policy.management_scopes {
            anyhow::ensure!(
                !boundary.tenant_id.is_nil() && !boundary.organization_id.is_nil(),
                "DATA_GRANT_POLICY management scope IDs must be non-nil"
            );
            anyhow::ensure!(
                boundaries.insert((boundary.tenant_id, boundary.organization_id)),
                "DATA_GRANT_POLICY tenant/organization boundaries must be unique"
            );
        }
        policy
            .applications
            .sort_unstable_by_key(|app| app.application_id);
        policy
            .management_scopes
            .sort_unstable_by_key(|boundary| (boundary.tenant_id, boundary.organization_id));
        Ok(policy)
    }

    /// Resolve only an explicitly registered resource/action in this app.
    pub(crate) fn operation(
        &self,
        application: Uuid,
        resource: &str,
        action: &str,
    ) -> Option<&Capability> {
        self.applications
            .iter()
            .find(|app| app.application_id == application)?
            .capabilities
            .iter()
            .find(|capability| capability.resource == resource && capability.action == action)
    }

    /// Resolve only an explicitly registered assignment purpose in this app.
    /// Allowed grant scopes remain management policy, not an ordering of the
    /// effective scopes emitted by a current organization context.
    pub(crate) fn assignment_members(
        &self,
        application: Uuid,
        resource: &str,
        action: &str,
    ) -> Option<&Capability> {
        let app = self
            .applications
            .iter()
            .find(|app| app.application_id == application)?;
        if !app
            .member_lookups
            .iter()
            .any(|lookup| lookup.resource == resource && lookup.action == action)
        {
            return None;
        }
        app.capabilities
            .iter()
            .find(|capability| capability.resource == resource && capability.action == action)
    }

    /// Metadata lookup never grants access to a business row or its owner.
    pub(crate) fn organization_lookup(
        &self,
        application: Uuid,
        resource: &str,
        action: &str,
    ) -> Option<&Capability> {
        let app = self
            .applications
            .iter()
            .find(|app| app.application_id == application)?;
        if !app.organization_lookups.iter().any(|lookup| {
            lookup.resource == resource
                && lookup.action == action
                && lookup.organization_type == "STORE"
        }) {
            return None;
        }
        app.capabilities
            .iter()
            .find(|capability| capability.resource == resource && capability.action == action)
    }
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
    use serde_json::{json, Value};

    fn fixture() -> Value {
        json!({
            "applications": [{
                "applicationId": Uuid::from_u128(1),
                "capabilities": [{"resource":"ticket", "action":"read", "scopes":["SELF","ORG"]}]
            }],
            "managementScopes": [{
                "tenantId":Uuid::from_u128(2), "organizationId":Uuid::from_u128(3),
                "includeDescendants":false, "read":true, "write":false
            }]
        })
    }

    fn parse(value: &Value) -> anyhow::Result<Policy> {
        Policy::from_json(&value.to_string())
    }

    #[test]
    fn member_lookup_defaults_preserve_existing_policy_and_capability_wire() {
        let mut value = fixture();
        let old = serde_json::to_string(&parse(&value).unwrap()).unwrap();
        assert!(
            !old.contains("memberLookups"),
            "legacy policy versions must not acquire an empty field"
        );
        value["applications"][0]["memberLookups"] = json!([]);
        let parsed = parse(&value).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), old);
        assert!(parsed
            .assignment_members(Uuid::from_u128(1), "ticket", "read")
            .is_none());
        value["applications"][0]["memberLookups"] = json!([{"resource":"ticket","action":"read"}]);
        let parsed = parse(&value).unwrap();
        let capability = parsed
            .assignment_members(Uuid::from_u128(1), "ticket", "read")
            .unwrap();
        let wire = serde_json::to_value(capability).unwrap();
        assert_eq!(
            wire.as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["action", "resource", "scopes"]
        );
        assert!(parsed
            .assignment_members(Uuid::from_u128(2), "ticket", "read")
            .is_none());
        assert!(parsed
            .assignment_members(Uuid::from_u128(1), "ticket", "create")
            .is_none());
        assert_ne!(serde_json::to_string(&parsed).unwrap(), old);
    }

    #[test]
    fn organization_lookups_are_independent_strict_and_bound_to_registered_store_purposes() {
        let mut value = fixture();
        let original = serde_json::to_string(&parse(&value).unwrap()).unwrap();
        assert!(!original.contains("organizationLookups"));
        assert!(parse(&value)
            .unwrap()
            .organization_lookup(Uuid::from_u128(1), "ticket", "read")
            .is_none());
        value["applications"][0]["organizationLookups"] = json!([]);
        assert_eq!(
            serde_json::to_string(&parse(&value).unwrap()).unwrap(),
            original
        );
        let valid = json!({"resource":"ticket","action":"read","organizationType":"STORE"});
        value["applications"][0]["organizationLookups"] = json!([valid]);
        assert!(parse(&value)
            .unwrap()
            .organization_lookup(Uuid::from_u128(1), "ticket", "read")
            .is_some());
        assert!(parse(&value)
            .unwrap()
            .assignment_members(Uuid::from_u128(1), "ticket", "read")
            .is_none());
        for invalid in [
            json!([{"resource":"ticket","action":"read"}]),
            json!([{"resource":"ticket","action":"read","organizationType":"COMPANY"}]),
            json!([{"resource":"ticket","action":"update","organizationType":"STORE"}]),
            json!([{"resource":"ticket","action":"read","organizationType":"STORE","tenantId":"anything"}]),
            json!([valid, valid]),
            json!(null),
        ] {
            value["applications"][0]["organizationLookups"] = invalid;
            assert!(parse(&value).is_err());
        }
    }

    #[test]
    fn member_lookup_declarations_are_strict_unique_and_bound_to_capabilities() {
        for declaration in [
            json!(null),
            json!({}),
            json!([{}]),
            json!([{"resource":"ticket","action":"read","purpose":"anything"}]),
            json!([{"resource":"ticket","action":"update"}]),
            json!([{"resource":"user","action":"read"}]),
            json!([{"resource":"ticket","action":"read"},{"resource":"ticket","action":"read"}]),
            json!([{"resource":"Ticket","action":"read"}]),
        ] {
            let mut value = fixture();
            value["applications"][0]["memberLookups"] = declaration;
            assert!(parse(&value).is_err());
        }
        let mut value = fixture();
        let mut other = value["applications"][0].clone();
        other["applicationId"] = json!(Uuid::from_u128(2));
        other["capabilities"][0]["action"] = json!("write");
        value["applications"].as_array_mut().unwrap().push(other);
        value["applications"][0]["memberLookups"] = json!([{"resource":"ticket","action":"write"}]);
        assert!(
            parse(&value).is_err(),
            "another application's capability cannot register a lookup"
        );
    }

    #[test]
    fn member_lookup_limits_and_canonical_order_do_not_invent_scope_hierarchy() {
        let mut value = fixture();
        value["applications"][0]["capabilities"] = json!((0..128)
            .map(
                |n| json!({"resource":format!("record-{n}"),"action":"assign","scopes":["TENANT"]})
            )
            .collect::<Vec<_>>());
        value["applications"][0]["memberLookups"] = json!((0..128)
            .map(|n| json!({"resource":format!("record-{n}"),"action":"assign"}))
            .collect::<Vec<_>>());
        let expected = serde_json::to_string(&parse(&value).unwrap()).unwrap();
        value["applications"][0]["memberLookups"]
            .as_array_mut()
            .unwrap()
            .reverse();
        let parsed = parse(&value).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), expected);
        assert_eq!(
            parsed
                .assignment_members(Uuid::from_u128(1), "record-0", "assign")
                .unwrap()
                .scopes,
            [GrantScope::Tenant]
        );
        value["applications"][0]["memberLookups"]
            .as_array_mut()
            .unwrap()
            .push(json!({"resource":"extra","action":"assign"}));
        assert!(parse(&value).is_err());
    }

    #[test]
    fn strict_schema_rejects_unknown_missing_and_ill_typed_fields_without_echoing_input() {
        for field in ["applications", "managementScopes"] {
            let mut value = fixture();
            value.as_object_mut().unwrap().remove(field);
            assert!(parse(&value).is_err());
        }
        for (level, fields) in [
            (0, vec!["applicationId", "capabilities"]),
            (1, vec!["resource", "action", "scopes"]),
            (
                2,
                vec![
                    "tenantId",
                    "organizationId",
                    "includeDescendants",
                    "read",
                    "write",
                ],
            ),
        ] {
            for field in fields {
                let mut value = fixture();
                let object = match level {
                    0 => &mut value["applications"][0],
                    1 => &mut value["applications"][0]["capabilities"][0],
                    _ => &mut value["managementScopes"][0],
                };
                object.as_object_mut().unwrap().remove(field);
                assert!(parse(&value).is_err(), "missing {field} was accepted");
            }
        }
        for level in 0..4 {
            let mut value = fixture();
            let object = match level {
                0 => &mut value,
                1 => &mut value["applications"][0],
                2 => &mut value["applications"][0]["capabilities"][0],
                _ => &mut value["managementScopes"][0],
            };
            object["untrusted-policy-secret"] = json!("not echoed");
            let error = parse(&value).unwrap_err().to_string();
            assert!(!error.contains("untrusted-policy-secret") && !error.contains("not echoed"));
        }
        for raw in [
            "",
            " ",
            "null",
            "[]",
            "{",
            "{\"applications\":[],\"applications\":[],\"managementScopes\":[]}",
        ] {
            assert!(Policy::from_json(raw).is_err());
        }
        let mut value = fixture();
        value["managementScopes"][0]["read"] = json!("true");
        assert!(parse(&value).is_err());
    }

    #[test]
    fn identifiers_and_scope_labels_are_exact_and_bounded() {
        for field in ["resource", "action"] {
            let mut value = fixture();
            value["applications"][0]["capabilities"][0][field] = json!("a".repeat(100));
            assert!(parse(&value).is_ok());
            for invalid in [
                "".into(),
                "a".repeat(101),
                "A".into(),
                "a:b".into(),
                " a".into(),
                "a ".into(),
                "a.b".into(),
                "1a".into(),
                "中".into(),
                "a\n".into(),
            ] {
                value["applications"][0]["capabilities"][0][field] = json!(invalid);
                assert!(parse(&value).is_err());
            }
            value["applications"][0]["capabilities"][0][field] = json!("a_b-01");
            assert!(parse(&value).is_ok());
        }
        for invalid in [
            json!([]),
            json!(["self"]),
            json!(["ALL"]),
            json!([null]),
            json!("SELF"),
        ] {
            let mut value = fixture();
            value["applications"][0]["capabilities"][0]["scopes"] = invalid;
            assert!(parse(&value).is_err());
        }
        for scope in [
            GrantScope::SelfOnly,
            GrantScope::Org,
            GrantScope::OrgSubtree,
            GrantScope::Tenant,
        ] {
            assert_eq!(GrantScope::from_str_value(scope.as_str()), Some(scope));
        }
        assert_eq!(GrantScope::from_str_value("org"), None);
    }

    #[test]
    fn duplicate_identities_pairs_scopes_and_boundaries_are_rejected() {
        let mut value = fixture();
        let app = value["applications"][0].clone();
        value["applications"].as_array_mut().unwrap().push(app);
        assert!(parse(&value).is_err());
        let mut value = fixture();
        let capability = value["applications"][0]["capabilities"][0].clone();
        value["applications"][0]["capabilities"]
            .as_array_mut()
            .unwrap()
            .push(capability);
        assert!(parse(&value).is_err());
        let mut value = fixture();
        value["applications"][0]["capabilities"][0]["scopes"] = json!(["ORG", "ORG"]);
        assert!(parse(&value).is_err());
        let mut value = fixture();
        let mut boundary = value["managementScopes"][0].clone();
        boundary["read"] = json!(false);
        boundary["write"] = json!(true);
        value["managementScopes"]
            .as_array_mut()
            .unwrap()
            .push(boundary);
        assert!(parse(&value).is_err());
        for field in ["applicationId", "tenantId", "organizationId"] {
            let mut value = fixture();
            if field == "applicationId" {
                value["applications"][0][field] = json!(Uuid::nil());
            } else {
                value["managementScopes"][0][field] = json!(Uuid::nil());
            }
            assert!(parse(&value).is_err());
        }
    }

    #[test]
    fn application_capability_and_boundary_limits_accept_maximum_and_reject_one_more() {
        let mut value = fixture();
        value["applications"] = json!((1..=64)
            .map(|id| json!({
                "applicationId":Uuid::from_u128(id), "capabilities":[]
            }))
            .collect::<Vec<_>>());
        assert!(parse(&value).is_ok());
        value["applications"]
            .as_array_mut()
            .unwrap()
            .push(json!({"applicationId":Uuid::from_u128(65),"capabilities":[]}));
        assert!(parse(&value).is_err());
        let mut value = fixture();
        value["applications"][0]["capabilities"] = json!((0..128).map(|id| json!({
            "resource":format!("resource-{id}"),"action":"read","scopes":["SELF","ORG","ORG_SUBTREE","TENANT"]
        })).collect::<Vec<_>>());
        assert!(parse(&value).is_ok());
        value["applications"][0]["capabilities"]
            .as_array_mut()
            .unwrap()
            .push(json!({"resource":"one-more","action":"read","scopes":["SELF"]}));
        assert!(parse(&value).is_err());
        let mut value = fixture();
        value["managementScopes"] = json!((1..=128)
            .map(|id| json!({
                "tenantId":Uuid::from_u128(1),"organizationId":Uuid::from_u128(id),
                "includeDescendants":false,"read":true,"write":false
            }))
            .collect::<Vec<_>>());
        assert!(parse(&value).is_ok());
        value["managementScopes"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "tenantId":Uuid::from_u128(1),"organizationId":Uuid::from_u128(129),
                "includeDescendants":false,"read":true,"write":false
            }));
        assert!(parse(&value).is_err());
        assert!(Policy::from_json(&" ".repeat(MAX_POLICY_BYTES + 1)).is_err());
    }

    #[test]
    fn canonical_serialization_ignores_set_order_without_expanding_authority() {
        let mut value = fixture();
        let mut second_app = value["applications"][0].clone();
        second_app["applicationId"] = json!(Uuid::from_u128(4));
        second_app["capabilities"][0]["resource"] = json!("store");
        value["applications"]
            .as_array_mut()
            .unwrap()
            .push(second_app);
        let mut second_capability = value["applications"][0]["capabilities"][0].clone();
        second_capability["action"] = json!("write");
        value["applications"][0]["capabilities"]
            .as_array_mut()
            .unwrap()
            .push(second_capability);
        let mut second_boundary = value["managementScopes"][0].clone();
        second_boundary["organizationId"] = json!(Uuid::from_u128(5));
        second_boundary["read"] = json!(false);
        second_boundary["write"] = json!(true);
        value["managementScopes"]
            .as_array_mut()
            .unwrap()
            .push(second_boundary);
        let expected = serde_json::to_string(&parse(&value).unwrap()).unwrap();
        value["applications"].as_array_mut().unwrap().reverse();
        value["managementScopes"].as_array_mut().unwrap().reverse();
        for app in value["applications"].as_array_mut().unwrap() {
            app["capabilities"].as_array_mut().unwrap().reverse();
            for capability in app["capabilities"].as_array_mut().unwrap() {
                capability["scopes"].as_array_mut().unwrap().reverse();
            }
        }
        assert_eq!(
            serde_json::to_string(&parse(&value).unwrap()).unwrap(),
            expected
        );
        let boundary = &parse(&value).unwrap().management_scopes[1];
        assert!(!boundary.read && boundary.write && !boundary.include_descendants);
        let mut value = fixture();
        value["applications"][0]["capabilities"][0]["scopes"] = json!(["TENANT"]);
        let policy = parse(&value).unwrap();
        assert_eq!(
            policy.applications[0].capabilities[0].scopes,
            [GrantScope::Tenant]
        );
        assert!(!policy.applications[0].capabilities[0]
            .scopes
            .contains(&GrantScope::Org));
        assert_eq!(
            serde_json::to_value(Policy::default()).unwrap(),
            json!({"applications":[],"managementScopes":[]})
        );
    }
}
