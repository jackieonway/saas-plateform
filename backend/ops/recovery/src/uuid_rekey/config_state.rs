//! Old preview snapshots are inventory inputs only. No source snapshot, token,
//! credential, business ID or stage-complete flag is emitted for runtime reuse.
use crate::{Error, Result};
use serde_json::{Map, Value};

fn object<'a>(value: &'a Value, allowed: &[&str]) -> Result<&'a Map<String, Value>> {
    let rows = value
        .as_object()
        .ok_or(Error("closed preview inventory object required"))?;
    if rows.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(Error("unknown closed preview inventory field"));
    }
    Ok(rows)
}
fn leaf(value: &Value, allowed: &[&str], nested: &[&str]) -> Result<()> {
    for (key, value) in object(value, allowed)? {
        if !nested.contains(&key.as_str()) && (value.is_array() || value.is_object()) {
            return Err(Error("closed preview inventory scalar field required"));
        }
    }
    Ok(())
}
fn string_arrays(value: &Value, keys: &[&str]) -> Result<()> {
    for key in keys {
        if let Some(value) = value.get(key) {
            if !value
                .as_array()
                .is_some_and(|rows| rows.iter().all(Value::is_string))
            {
                return Err(Error("closed preview inventory string array required"));
            }
        }
    }
    Ok(())
}
fn definition(value: &Value) -> Result<()> {
    object(
        value,
        &[
            "schemaVersion",
            "application",
            "oauthClient",
            "menus",
            "grants",
        ],
    )?;
    if value
        .get("schemaVersion")
        .is_some_and(|version| version != "1.0")
    {
        return Err(Error("preview definition schema version required"));
    }
    leaf(
        &value["application"],
        &[
            "code",
            "name",
            "icon",
            "basePath",
            "entryType",
            "entryUrl",
            "runtimeMode",
            "autoCreateMenu",
            "description",
            "sortOrder",
            "status",
        ],
        &[],
    )?;
    let menus = value["menus"]
        .as_array()
        .ok_or(Error("preview definition menus required"))?;
    for menu in menus {
        leaf(
            menu,
            &[
                "key",
                "parentKey",
                "type",
                "title",
                "path",
                "permission",
                "sortOrder",
                "visible",
                "linkTarget",
                "icon",
            ],
            &[],
        )?;
    }
    if let Some(client) = value.get("oauthClient") {
        leaf(
            client,
            &[
                "clientName",
                "clientType",
                "requirePkce",
                "grantTypes",
                "scopes",
                "redirectUris",
                "accessTokenValidity",
                "refreshTokenValidity",
                "provisioning",
                "description",
            ],
            &["grantTypes", "scopes", "redirectUris"],
        )?;
        string_arrays(client, &["grantTypes", "scopes", "redirectUris"])?;
    }
    if let Some(grants) = value.get("grants") {
        if !grants.as_array().is_some_and(Vec::is_empty) {
            return Err(Error(
                "embedded preview definition grants require separate registered owner",
            ));
        }
    }
    Ok(())
}
fn organization(value: &mut Value) -> Result<()> {
    leaf(
        value,
        &[
            "id",
            "tenantId",
            "parentId",
            "name",
            "code",
            "organizationType",
            "contactName",
            "contactPhone",
            "contactEmail",
            "sortOrder",
            "description",
            "status",
            "expiresAt",
            "province",
            "city",
            "district",
            "township",
            "village",
            "address",
            "longitude",
            "latitude",
            "departmentCount",
            "userCount",
            "children",
            "administrator",
        ],
        &["children", "administrator"],
    )?;
    if let Some(children) = value.get("children") {
        if !children.as_array().is_some_and(Vec::is_empty) {
            return Err(Error(
                "unregistered recursive preview organization tree requires regeneration",
            ));
        }
    }
    if let Some(administrator) = value.get_mut("administrator") {
        leaf(
            administrator,
            &[
                "userId",
                "username",
                "roleName",
                "created",
                "alreadyInitialized",
                "initialPassword",
            ],
            &[],
        )?;
        administrator
            .as_object_mut()
            .ok_or(Error("closed administrator object required"))?
            .remove("initialPassword");
    }
    Ok(())
}

pub(super) fn validate_and_discard(value: &mut Value) -> Result<()> {
    object(
        value,
        &[
            "origin",
            "fingerprint",
            "applications",
            "clients",
            "adminId",
            "businessTenant",
            "businessOrganization",
            "storeOrganization",
            "membership",
            "role",
            "storeRole",
            "platformSession",
            "businessSession",
            "applicationTokens",
            "prepared",
            "grantsApplied",
            "storeAccessConfigured",
        ],
    )?;
    // Signed credentials and all their embedded identity claims are retired as
    // whole values; they never enter the UUID remapper or output artifact.
    let root = value
        .as_object_mut()
        .ok_or(Error("closed preview inventory required"))?;
    for key in [
        "platformSession",
        "businessSession",
        "applicationTokens",
        "fingerprint",
        "prepared",
        "grantsApplied",
        "storeAccessConfigured",
    ] {
        root.remove(key);
    }
    if !value["origin"].is_string() {
        return Err(Error("preview origin required"));
    }
    let applications = value
        .get("applications")
        .and_then(Value::as_object)
        .ok_or(Error("preview application map required"))?;
    for application in applications.values() {
        leaf(
            application,
            &[
                "id",
                "code",
                "name",
                "icon",
                "basePath",
                "entryType",
                "entryUrl",
                "runtimeMode",
                "description",
                "sortOrder",
                "status",
                "createdAt",
                "menuCount",
                "menuIds",
                "definition",
            ],
            &["menuIds", "definition"],
        )?;
        if let Some(menus) = application.get("menuIds") {
            if !menus
                .as_object()
                .is_some_and(|rows| rows.values().all(Value::is_string))
            {
                return Err(Error("preview logical menu map required"));
            }
        }
        if let Some(config) = application.get("definition") {
            definition(config)?;
        }
    }
    let clients = value
        .get_mut("clients")
        .and_then(Value::as_object_mut)
        .ok_or(Error("preview client map required"))?;
    for client in clients.values_mut() {
        leaf(
            client,
            &[
                "id",
                "clientId",
                "clientName",
                "clientType",
                "requirePkce",
                "description",
                "redirectUris",
                "grantTypes",
                "scopes",
                "accessTokenValidity",
                "refreshTokenValidity",
                "status",
                "clientSecret",
                "applicationId",
            ],
            &["redirectUris", "grantTypes", "scopes"],
        )?;
        string_arrays(client, &["redirectUris", "grantTypes", "scopes"])?;
        if !client["clientId"].is_string() {
            return Err(Error("opaque preview OAuth client identifier required"));
        }
        client
            .as_object_mut()
            .ok_or(Error("closed preview client object required"))?
            .remove("clientSecret");
    }
    if let Some(tenant) = value.get("businessTenant") {
        leaf(tenant, &["id", "name", "code", "isPlatform"], &[])?;
    }
    for key in ["businessOrganization", "storeOrganization"] {
        if let Some(row) = value.get_mut(key) {
            organization(row)?;
        }
    }
    if let Some(membership) = value.get("membership") {
        leaf(
            membership,
            &[
                "membershipId",
                "organizationId",
                "tenantId",
                "departmentId",
                "position",
                "organizationName",
                "departmentName",
            ],
            &[],
        )?;
    }
    for key in ["role", "storeRole"] {
        if let Some(role) = value.get(key) {
            leaf(
                role,
                &[
                    "id",
                    "name",
                    "code",
                    "description",
                    "organizationId",
                    "organizationName",
                    "tenantId",
                    "status",
                    "builtIn",
                    "menuCount",
                    "userCount",
                    "createdAt",
                ],
                &[],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn source_state_credentials_and_stage_flags_are_discarded_whole() {
        let mut value = json!({"origin":"http://127.0.0.1:3001","fingerprint":"old", "applications":{},
            "clients":{"opaque":{"id":"00000000-0000-4000-8000-000000000001","clientId":"00000000-0000-4000-8000-000000000001","clientSecret":"secret-marker"}},
            "storeOrganization":{"id":"00000000-0000-4000-8000-000000000001","administrator":{"initialPassword":"password-marker"}},
            "platformSession":{"accessToken":"token-marker","unknownClaim":{"id":"signed-never-remap"}},
            "businessSession":{"accessToken":"token-marker"},"applicationTokens":{"app":"token-marker"},"prepared":true,"grantsApplied":true,"storeAccessConfigured":true});
        validate_and_discard(&mut value).unwrap();
        let bytes = value.to_string();
        for marker in [
            "secret-marker",
            "password-marker",
            "token-marker",
            "signed-never-remap",
            "fingerprint",
            "prepared",
            "grantsApplied",
            "storeAccessConfigured",
        ] {
            assert!(!bytes.contains(marker));
        }
        assert_eq!(
            value["clients"]["opaque"]["clientId"],
            "00000000-0000-4000-8000-000000000001"
        );
    }
    #[test]
    fn unknown_fields_and_unregistered_recursive_tree_fail_closed() {
        let base = json!({"origin":"http://127.0.0.1:3001","applications":{},"clients":{}});
        let mut value = base.clone();
        value["unexpected"] = json!("private-value");
        assert!(validate_and_discard(&mut value).is_err());
        let mut value = base.clone();
        value["clients"] = json!({"key":{"clientId":"opaque","unknown":true}});
        assert!(validate_and_discard(&mut value).is_err());
        let mut value = base;
        value["businessOrganization"] =
            json!({"children":[{"id":"00000000-0000-4000-8000-000000000001"}]});
        let error = validate_and_discard(&mut value).unwrap_err();
        assert_eq!(
            error.0,
            "unregistered recursive preview organization tree requires regeneration"
        );
    }
    #[test]
    fn embedded_definition_version_cannot_hide_unregistered_nested_fields() {
        for version in [
            json!(1),
            json!({"unregistered":{"secret":"marker"}}),
            json!("2.0"),
        ] {
            let mut value = json!({"origin":"http://127.0.0.1:3001","clients":{},
                "applications":{"code":{"definition":{"schemaVersion":version,"application":{},"menus":[]}}}});
            let error = validate_and_discard(&mut value).unwrap_err();
            assert_eq!(error.0, "preview definition schema version required");
        }
    }
}
