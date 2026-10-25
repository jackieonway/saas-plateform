//! Finite operator grant-intent DTO, mirrored from platform-admin's owner.
use super::parse_id;
use crate::{Error, Result};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema_version: String,
    application_code: String,
    application_id: String,
    menu_ids: BTreeMap<String, String>,
    #[serde(default)]
    targets: Vec<Target>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Target {
    tenant_id: String,
    organization_id: String,
    role_id: String,
    user_id: String,
    membership_id: String,
    permissions: Vec<String>,
    data_grants: Vec<Grant>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Grant {
    resource: String,
    action: String,
    scope: String,
}

const ORDERS: &[&str] = &[
    "ticket:read",
    "ticket:create",
    "ticket:update",
    "ticket:delete",
    "ticket:export",
    "notification:publish",
];
const STORES: &[&str] = &[
    "store:read",
    "store:create",
    "store:update",
    "store-staff:read",
    "store-daily-report:read",
    "store-daily-report:create",
    "store-daily-report:update",
];

fn require(value: bool) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(Error("invalid closed grant intent contract"))
    }
}

pub(super) fn validate(value: &Value, target_version: bool) -> Result<()> {
    let manifest: Manifest = serde_json::from_value(value.clone())
        .map_err(|_| Error("invalid closed grant intent DTO"))?;
    require(manifest.schema_version == "1.0")?;
    parse_id(&manifest.application_id, target_version)?;
    let permissions = match manifest.application_code.as_str() {
        "work-orders" => ORDERS,
        "stores" => STORES,
        _ => return Err(Error("unsupported grant intent application")),
    };
    let mut keys = BTreeSet::from(["root"]);
    keys.extend(permissions.iter().copied());
    if manifest.application_code == "stores" {
        keys.insert("store-daily-report:detail");
    }
    require(
        manifest
            .menu_ids
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            == keys,
    )?;
    let mut menu_ids = BTreeSet::new();
    for id in manifest.menu_ids.values() {
        require(menu_ids.insert(parse_id(id, target_version)?))?;
    }
    let mut memberships = BTreeSet::new();
    let mut user_orgs = BTreeSet::new();
    let mut org_tenants = BTreeMap::new();
    let mut role_owners = BTreeMap::new();
    for target in &manifest.targets {
        for id in [
            &target.tenant_id,
            &target.organization_id,
            &target.role_id,
            &target.user_id,
            &target.membership_id,
        ] {
            parse_id(id, target_version)?;
        }
        require(
            memberships.insert(&target.membership_id)
                && user_orgs.insert((&target.user_id, &target.organization_id)),
        )?;
        if let Some(previous) = org_tenants.insert(&target.organization_id, &target.tenant_id) {
            require(previous == &target.tenant_id)?;
        }
        let owner = (&target.tenant_id, &target.organization_id);
        if let Some(previous) = role_owners.insert(&target.role_id, owner) {
            require(previous == owner)?;
        }
        let mut selected = BTreeSet::new();
        for permission in &target.permissions {
            require(permissions.contains(&permission.as_str()) && selected.insert(permission))?;
        }
        let mut grants = BTreeSet::new();
        for grant in &target.data_grants {
            let permission = format!("{}:{}", grant.resource, grant.action);
            require(
                permissions.contains(&permission.as_str())
                    && grants.insert((&grant.resource, &grant.action))
                    && ["SELF", "ORG", "ORG_SUBTREE", "TENANT"].contains(&grant.scope.as_str()),
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn id(n: usize) -> String {
        format!("00000000-0000-4000-8000-{n:012x}")
    }
    fn fixture(app: &str) -> Value {
        let mut menu_ids = serde_json::Map::new();
        let permissions = if app == "stores" { STORES } else { ORDERS };
        for (index, permission) in std::iter::once("root")
            .chain(permissions.iter().copied())
            .enumerate()
        {
            menu_ids.insert(permission.into(), json!(id(index + 10)));
        }
        if app == "stores" {
            menu_ids.insert("store-daily-report:detail".into(), json!(id(50)));
        }
        json!({"schemaVersion":"1.0", "applicationCode":app,"applicationId":id(1),"menuIds":menu_ids,
            "targets":[{"tenantId":id(2),"organizationId":id(3),"roleId":id(4),"userId":id(5),"membershipId":id(6),
                "permissions":[permissions[0]],"dataGrants":[{"resource":permissions[0].split(':').next().unwrap(),"action":"read","scope":"ORG"}]}]})
    }
    #[test]
    fn finite_owner_contract_preserves_original_values_and_rejects_source_v4_as_target() {
        for app in ["stores", "work-orders"] {
            let value = fixture(app);
            let copy = value.clone();
            validate(&value, false).unwrap();
            assert_eq!(value, copy);
            assert!(validate(&value, true).is_err());
        }
    }
    #[test]
    fn grant_contract_rejects_unknown_fields_placeholder_ids_and_menus() {
        for pointer in [
            "/unexpected",
            "/targets/0/unexpected",
            "/targets/0/dataGrants/0/unexpected",
        ] {
            let mut value = fixture("work-orders");
            let (parent, key) = pointer.rsplit_once('/').unwrap();
            value
                .pointer_mut(parent)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert(key.into(), json!("secret-marker"));
            let error = validate(&value, false).unwrap_err();
            assert!(!error.0.contains("secret-marker"));
        }
        let mut value = fixture("work-orders");
        value["applicationId"] = json!("<APPLICATION_ID>");
        assert!(validate(&value, false).is_err());
        let mut value = fixture("stores");
        value["menuIds"]
            .as_object_mut()
            .unwrap()
            .remove("store-daily-report:detail");
        assert!(validate(&value, false).is_err());
    }
    #[test]
    fn grant_contract_rejects_duplicate_targets_conflicting_owners_and_invalid_grants() {
        let mut value = fixture("work-orders");
        let target = value["targets"][0].clone();
        value["targets"].as_array_mut().unwrap().push(target);
        assert!(validate(&value, false).is_err());
        for field in ["tenantId", "organizationId"] {
            let mut value = fixture("work-orders");
            let mut target = value["targets"][0].clone();
            target["membershipId"] = json!(id(100));
            target["userId"] = json!(id(101));
            target[field] = json!(id(102));
            value["targets"].as_array_mut().unwrap().push(target);
            assert!(validate(&value, false).is_err());
        }
        for invalid in [
            json!(["ticket:read", "ticket:read"]),
            json!(["system:admin"]),
        ] {
            let mut value = fixture("work-orders");
            value["targets"][0]["permissions"] = invalid;
            assert!(validate(&value, false).is_err());
        }
        let mut value = fixture("work-orders");
        value["targets"][0]["dataGrants"][0]["scope"] = json!("GLOBAL");
        assert!(validate(&value, false).is_err());
    }
}
