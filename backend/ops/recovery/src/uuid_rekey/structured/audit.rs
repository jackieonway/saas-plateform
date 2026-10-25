//! Closed historical audit projections. Never re-hash a historical snapshot.
use serde_json::Value;
use uuid::Uuid;

use crate::{Error, Result};

type Resolver<'a> = dyn Fn(&str, Uuid) -> Result<Uuid> + 'a;

pub(super) fn transform(
    action: &str,
    detail: &mut Value,
    map: &super::super::mapping::TypedMap,
    registry: &Value,
) -> Result<()> {
    transform_with(
        action,
        detail,
        &|authority, old| map.resolve_historical(authority, old),
        registry,
    )
}

/// An owner deletion event can attest an absent source entity. Merely appearing
/// in a historical snapshot or in an arbitrary UUID-shaped string cannot.
pub(super) fn deletion_proof(
    action: &str,
    detail: &Value,
    registry: &Value,
) -> Result<Option<(&'static str, Uuid)>> {
    let authority = match (detail["objectType"].as_str(), action) {
        (Some("application"), "DELETE_APPLICATION") => "platform.application",
        (Some("department"), "DELETE_DEPARTMENT") => "platform.department",
        (Some("menu"), "DELETE_MENU") => "platform.menu",
        (Some("role"), "DELETE_ROLE") => "platform.role",
        (Some("tenant"), "DELETE_TENANT") => "platform.tenant",
        (Some("organization"), "DELETE_ORGANIZATION") => "platform.organization",
        (Some("user"), "DELETE_USER") => "platform.user",
        (Some("oauth_client"), "DELETE_CLIENT") => "platform.oauth_client",
        _ => return Ok(None),
    };
    // This verifies the entire source schema and its repeated identity without
    // looking up or allocating mappings. User summaries intentionally omit id;
    // their owner-produced envelope supplies the subject identity instead.
    let mut validated = detail.clone();
    transform_with(action, &mut validated, &|_, id| Ok(id), registry)?;
    Ok(Some((authority, parse_uuid(&detail["objectId"])?)))
}

/// Explicit proof for excluded credentials named by their own validated owner
/// event. Ordinary nested references never gain tombstone provenance here.
pub(super) fn credential_proofs(
    action: &str,
    detail: &Value,
    registry: &Value,
) -> Result<Vec<(&'static str, Uuid)>> {
    let authority = match (detail["objectType"].as_str(), action) {
        (Some("platform_session"), "CONTEXT_SWITCH" | "LOGOUT" | "REVOKE_SESSION") => {
            "platform.platform_session"
        }
        (Some("oauth_consent"), "CONSENT_APPROVE" | "CONSENT_DENY") => {
            "platform.oauth_authorization_request"
        }
        (Some("oauth_token" | "oauth_credential"), "TOKEN_REVOKED")
        | (Some("app_token"), "TOKEN_ISSUED") => "platform.oauth_token",
        (Some("oauth_device_authorization"), "DEVICE_APPROVE" | "DEVICE_DENY") => {
            "platform.oauth_device_authorization"
        }
        _ => return Ok(Vec::new()),
    };
    let mut validated = detail.clone();
    transform_with(action, &mut validated, &|_, id| Ok(id), registry)?;
    let mut proofs = vec![(authority, parse_uuid(&detail["objectId"])?)];
    if detail["objectType"] == "app_token" {
        proofs.push(("platform.oauth_jti", parse_uuid(&detail["after"]["jti"])?));
    }
    Ok(proofs)
}

fn transform_with(
    action: &str,
    detail: &mut Value,
    resolve: &Resolver<'_>,
    registry: &Value,
) -> Result<()> {
    // Validation failure must never leave a partly rewritten historical record.
    let mut candidate = detail.clone();
    envelope(action, &mut candidate, resolve, registry)?;
    *detail = candidate;
    Ok(())
}

fn envelope(
    action: &str,
    value: &mut Value,
    resolve: &Resolver<'_>,
    registry: &Value,
) -> Result<()> {
    shape(value, "objectType:s objectId:s before:v after:v", resolve)?;
    let object_type = value["objectType"]
        .as_str()
        .ok_or(Error("invalid audit discriminator"))?
        .to_owned();
    let authority = match object_type.as_str() {
        "application" | "application_url_entry" | "application_deployment" => {
            "platform.application"
        }
        "application_release" | "application_release_validation" => "platform.release",
        "department" => "platform.department",
        "menu" => "platform.menu",
        "role" | "role_menus" | "role_users" => "platform.role",
        "tenant" | "platformSettings" => "platform.tenant",
        "organization" => "platform.organization",
        "user" | "user_memberships" | "user_roles" | "user_sessions" => "platform.user",
        "membership_data_grants" => "platform.membership",
        "notificationPublication" => "platform.notification_publication",
        "platform_session" => "platform.platform_session",
        "oauth_client" => "platform.oauth_client",
        "oauth_consent" => "platform.oauth_authorization_request",
        "app_token" | "oauth_token" | "oauth_credential" => "platform.oauth_token",
        "oauth_device_authorization" => "platform.oauth_device_authorization",
        "oauth_history" => "",
        _ => return Err(Error("unregistered audit object type")),
    };
    let envelope_registry = registry["structuredReferences"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["id"] == "audit-change-envelope")
        })
        .ok_or(Error("audit registry missing envelope"))?;
    if object_type != "oauth_history"
        && envelope_registry["authorityCases"][&object_type].as_str() != Some(authority)
    {
        return Err(Error("audit authority differs from registry"));
    }
    if object_type == "oauth_history" {
        if action != "OAUTH_PURGE"
            || parse_uuid(&value["objectId"])? != Uuid::nil()
            || !envelope_registry["aggregateSentinels"]
                .as_array()
                .is_some_and(|items| {
                    items.iter().any(|item| {
                        item["when"]["/objectType"] == "oauth_history"
                            && item["path"] == "/objectId"
                            && item["valueClass"] == "nil_UUID"
                    })
                })
        {
            return Err(Error("invalid audit aggregate sentinel"));
        }
        pair(
            value,
            "eligibleTokenCount:n eligibleDeviceCount:n",
            "eligibleTokenCount:n eligibleDeviceCount:n",
            Mode::Update,
            resolve,
        )?;
        if value["after"]["eligibleTokenCount"] != 0 || value["after"]["eligibleDeviceCount"] != 0 {
            return Err(Error("invalid audit purge result"));
        }
        return Ok(());
    }
    let old_id = parse_uuid(&value["objectId"])?;
    if old_id.is_nil() {
        return Err(Error("nil audit object UUID"));
    }
    let new_id = resolve(authority, old_id)?;
    value["objectId"] = Value::String(new_id.to_string());
    match (object_type.as_str(), action) {
        ("application", "CREATE_APPLICATION") => pair(value, APP, APP, Mode::Create, resolve)?,
        ("application", "UPDATE_APPLICATION") => pair(value, APP, APP_UPDATED, Mode::Update, resolve)?,
        ("application", "DELETE_APPLICATION") => pair(value, APP, APP, Mode::Delete, resolve)?,
        ("application_url_entry", "CONFIGURE_APPLICATION_URL_ENTRY") => pair(value, URL_ENTRY, URL_ENTRY, Mode::Upsert, resolve)?,
        ("application_url_entry", "CLEAR_APPLICATION_URL_ENTRY") => pair(value, URL_ENTRY, URL_ENTRY, Mode::Delete, resolve)?,
        ("application_release", "REGISTER_APPLICATION_RELEASE") => pair(value, RELEASE, RELEASE, Mode::Create, resolve)?,
        ("application_release", "ADOPT_APPLICATION_RELEASE_ARTIFACT")
        | ("application_release_validation", "PREFLIGHT_APPLICATION_RELEASE") => pair(value, RELEASE, RELEASE, Mode::Update, resolve)?,
        ("application_deployment", "ACTIVATE_APPLICATION_RELEASE") => pair(value, DEPLOYMENT, DEPLOYMENT, Mode::Upsert, resolve)?,
        ("application_deployment", "ROLLBACK_APPLICATION_RELEASE") => pair(value, DEPLOYMENT, DEPLOYMENT, Mode::Update, resolve)?,
        ("department", "CREATE_DEPARTMENT") => pair(value, DEPARTMENT, DEPARTMENT, Mode::Create, resolve)?,
        ("department", "UPDATE_DEPARTMENT") => pair(value, DEPARTMENT, DEPARTMENT_UPDATED, Mode::Update, resolve)?,
        ("department", "DELETE_DEPARTMENT") => pair(value, DEPARTMENT, DEPARTMENT, Mode::Delete, resolve)?,
        ("menu", "CREATE_MENU") => pair(value, MENU, MENU, Mode::Create, resolve)?,
        ("menu", "UPDATE_MENU") => pair(value, MENU, MENU_UPDATED, Mode::Update, resolve)?,
        ("menu", "DELETE_MENU") => pair(value, MENU, MENU, Mode::Delete, resolve)?,
        ("role", "CREATE_ROLE") => pair(value, ROLE, ROLE, Mode::Create, resolve)?,
        ("role", "UPDATE_ROLE") => pair(value, ROLE, ROLE_UPDATED, Mode::Update, resolve)?,
        ("role", "DELETE_ROLE") => pair(value, ROLE, ROLE, Mode::Delete, resolve)?,
        ("role_menus", "ROLE_MENUS") => pair(value, ROLE_MENUS, ROLE_MENUS, Mode::Update, resolve)?,
        ("role_users", "UPDATE_ROLE") => pair(value, ROLE_USERS, ROLE_USERS, Mode::Update, resolve)?,
        ("tenant", "CREATE_TENANT") => pair(value, TENANT_FULL, TENANT_CREATED, Mode::Create, resolve)?,
        ("tenant", "UPDATE_TENANT") => pair(value, TENANT_CONFIG, TENANT_CONFIG, Mode::Update, resolve)?,
        ("tenant", "DELETE_TENANT") => pair(value, TENANT_FULL, TENANT_FULL, Mode::Delete, resolve)?,
        ("tenant", "TENANT_AUTH") => pair(value, TENANT_AUTH, TENANT_AUTH, Mode::Update, resolve)?,
        ("tenant", "INITIALIZE_ADMINISTRATOR") => pair(value, TENANT_FULL, TENANT_ADMIN, Mode::Update, resolve)?,
        ("organization", "CREATE_ORGANIZATION") => pair(value, ORG_FULL, ORG_CREATED, Mode::Create, resolve)?,
        ("organization", "UPDATE_ORGANIZATION") => pair(value, ORG_CONFIG, ORG_CONFIG, Mode::Update, resolve)?,
        ("organization", "DELETE_ORGANIZATION") => pair(value, ORG_FULL, ORG_FULL, Mode::Delete, resolve)?,
        ("organization", "ORG_AUTH") => pair(value, ORG_AUTH, ORG_AUTH, Mode::Update, resolve)?,
        ("organization", "INITIALIZE_ADMINISTRATOR") => pair(value, ORG_FULL, ORG_ADMIN, Mode::Update, resolve)?,
        ("user", "CREATE_USER") => pair(value, USER, USER, Mode::Create, resolve)?,
        ("user", "UPDATE_USER") => pair(value, USER, USER_UPDATED, Mode::Update, resolve)?,
        ("user", "DELETE_USER") => pair(value, USER, USER, Mode::Delete, resolve)?,
        ("user", "RESET_PASSWORD") => pair(value, USER, USER_RESET, Mode::Update, resolve)?,
        ("user", "UPDATE_PASSWORD") => pair(value, "username:s sessionVersion:n", "username:s sessionVersion:n passwordChanged:b revokedSessions:n", Mode::Update, resolve)?,
        ("user_memberships", "UPDATE_USER") => {
            pair(value, "username:s items:a", "username:s items:a changedOrganizationIds:a revokedSessions:n", Mode::Update, resolve)?;
            for side in ["before", "after"] {
                for item in array_mut(&mut value[side]["items"])? {
                    shape(item, "organizationId:u=platform.organization organizationName:s departmentId:?u=platform.department positionSet:?b", resolve)?;
                }
            }
            for item in array_mut(&mut value["after"]["changedOrganizationIds"])? { uuid_value(item, "platform.organization", resolve)?; }
        }
        ("user_roles", "UPDATE_USER") => {
            pair(value, "username:s items:a", "username:s items:a", Mode::Update, resolve)?;
            for side in ["before", "after"] {
                for item in array_mut(&mut value[side]["items"])? {
                    shape(item, "roleId:u=platform.role roleName:s organizationId:u=platform.organization", resolve)?;
                }
            }
        }
        ("membership_data_grants", "DATA_GRANT_CHANGE") => {
            pair(value, "target:o version:h grants:a", "target:o version:h grants:a", Mode::Update, resolve)?;
            for side in ["before", "after"] {
                shape(&mut value[side]["target"], "tenantId:u=platform.tenant organizationId:u=platform.organization userId:u=platform.user membershipId:u=platform.membership applicationId:u=platform.application", resolve)?;
                if value[side]["target"]["membershipId"].as_str() != Some(new_id.to_string().as_str()) {
                    return Err(Error("audit grant target differs from object"));
                }
                for row in array_mut(&mut value[side]["grants"])? {
                    shape(row, "id:u=platform.membership_data_grant resource:s action:s scope:s", resolve)?;
                }
            }
        }
        ("user_sessions", "KICK_SESSIONS") => pair(value, USER_SESSIONS, USER_SESSIONS, Mode::Update, resolve)?,
        ("oauth_client", "CREATE_CLIENT") => pair(value, CLIENT, CLIENT, Mode::Create, resolve)?,
        ("oauth_client", "UPDATE_CLIENT") => pair(value, CLIENT, CLIENT_UPDATED, Mode::Update, resolve)?,
        ("oauth_client", "DELETE_CLIENT") => pair(value, CLIENT, CLIENT, Mode::Delete, resolve)?,
        ("oauth_credential", "TOKEN_REVOKED") => pair(value, "id:u=platform.oauth_token name:s clientId:s tokenType:s revoked:b", "id:u=platform.oauth_token name:s clientId:s tokenType:s revoked:b", Mode::Update, resolve)?,
        ("platform_session", "CONTEXT_SWITCH") => pair(value,
            "previousSessionId:u=platform.platform_session organizationId:u=platform.organization tenantId:u=platform.tenant username:s",
            "organizationId:u=platform.organization organizationName:s tenantId:u=platform.tenant username:s", Mode::Update, resolve)?,
        ("platform_session", "LOGOUT" | "REVOKE_SESSION") => pair(value, "username:s revoked:b", "username:s revoked:b", Mode::Update, resolve)?,
        ("oauth_consent", "CONSENT_APPROVE") => pair(value, "clientId:s consumed:b", "clientId:s consumed:b decision:s scope:s", Mode::Update, resolve)?,
        ("oauth_consent", "CONSENT_DENY") => pair(value, "clientId:s consumed:b", "clientId:s consumed:b decision:s", Mode::Update, resolve)?,
        ("app_token", "TOKEN_ISSUED") => pair(value, "", "applicationId:u=platform.application clientId:s audience:s jti:u=platform.oauth_jti expiresAt:s", Mode::Create, resolve)?,
        ("oauth_token", "TOKEN_REVOKED") => pair(value, "clientId:s tokenType:s tokenUse:s revoked:b", "clientId:s tokenType:s tokenUse:s revoked:b", Mode::Update, resolve)?,
        ("oauth_device_authorization", "DEVICE_APPROVE") => pair(value, "clientId:s status:s", "clientId:s status:s userId:u=platform.user tenantId:u=platform.tenant organizationId:u=platform.organization", Mode::Update, resolve)?,
        ("oauth_device_authorization", "DEVICE_DENY") => {
            // Both producers use DEVICE_DENY; their closed shapes are distinct.
            if value["before"].get("id").is_some() {
                pair(value, "id:u=platform.oauth_device_authorization name:s clientId:s status:s", "id:u=platform.oauth_device_authorization name:s clientId:s status:s", Mode::Update, resolve)?;
                require_string(&value["after"]["status"], "EXPIRED")?;
            } else {
                pair(value, "clientId:s status:s", "clientId:s status:s", Mode::Update, resolve)?;
                require_string(&value["after"]["status"], "DENIED")?;
            }
        }
        ("notificationPublication", "PUBLISH_NOTIFICATION") => pair(value, "", "recipientCount:n deliveryCount:n", Mode::Create, resolve)?,
        ("platformSettings", "UPDATE_PLATFORM_SETTINGS") => pair(value,
            "platformName:s platformTenantName:s platformTenantCode:s primaryColor:s brandingHash:h",
            "platformName:s platformTenantName:s platformTenantCode:s primaryColor:s brandingHash:h", Mode::Update, resolve)?,
        _ => return Err(Error("unregistered audit action or producer schema")),
    }
    validate_producer_values(&object_type, action, value)?;
    // Repeated object identities are a consistency proof, never a second issuer.
    for side in ["before", "after"] {
        if value[side].is_null() {
            continue;
        }
        let id_key = if object_type == "application_url_entry" {
            "applicationId"
        } else {
            "id"
        };
        if let Some(id) = value[side].get(id_key) {
            if id.as_str() != Some(new_id.to_string().as_str()) {
                return Err(Error("audit snapshot identity differs from object"));
            }
        }
    }
    Ok(())
}

fn validate_producer_values(object_type: &str, action: &str, value: &Value) -> Result<()> {
    let changed_fields: Option<(&str, &[&str])> = match (object_type, action) {
        ("application", "UPDATE_APPLICATION") => Some((
            "changedFields",
            &[
                "name",
                "icon",
                "basePath",
                "entryType",
                "entryUrl",
                "runtimeMode",
                "description",
                "sortOrder",
                "status",
            ],
        )),
        ("department", "UPDATE_DEPARTMENT") => Some((
            "changedFields",
            &["name", "code", "sortOrder", "description"],
        )),
        ("menu", "UPDATE_MENU") => Some((
            "changedFields",
            &[
                "parentId",
                "type",
                "title",
                "path",
                "icon",
                "component",
                "permission",
                "sortOrder",
                "visible",
                "linkUrl",
                "linkTarget",
            ],
        )),
        ("role", "UPDATE_ROLE") => Some(("changedFields", &["name", "code", "description"])),
        ("user", "UPDATE_USER") => {
            Some(("updatedFields", &["realName", "email", "phone", "gender"]))
        }
        _ => None,
    };
    if let Some((key, allowed)) = changed_fields {
        if let Some(fields) = value["after"].get(key) {
            let fields = fields
                .as_array()
                .ok_or(Error("invalid audit change fields"))?;
            let mut seen = std::collections::BTreeSet::new();
            if fields.is_empty()
                || fields.iter().any(|field| {
                    let Some(name) = field.as_str() else {
                        return true;
                    };
                    !allowed.contains(&name) || !seen.insert(name)
                })
            {
                return Err(Error("unknown audit changed field"));
            }
        }
    }
    for key in [
        "passwordChanged",
        "redirectUrisChanged",
        "scopesChanged",
        "descriptionChanged",
        "credentialRotated",
    ] {
        if value["after"].get(key).is_some_and(|flag| flag != true) {
            return Err(Error("invalid audit change marker"));
        }
    }
    for side in ["before", "after"] {
        let state = &value[side];
        if state.get("administratorUserId").is_some() != state.get("administratorCreated").is_some()
        {
            return Err(Error("incomplete audit administrator identity"));
        }
        if object_type == "application_url_entry"
            && !state.is_null()
            && state["entryConfigured"] != true
        {
            return Err(Error("invalid audit URL entry summary"));
        }
    }
    if matches!(
        (object_type, action),
        ("platform_session", "LOGOUT" | "REVOKE_SESSION")
            | ("oauth_token" | "oauth_credential", "TOKEN_REVOKED")
    ) && (value["before"]["revoked"] != false || value["after"]["revoked"] != true)
    {
        return Err(Error("invalid audit credential revocation"));
    }
    if object_type == "oauth_consent" {
        if value["before"]["consumed"] != false || value["after"]["consumed"] != true {
            return Err(Error("invalid audit consent consumption"));
        }
        require_string(
            &value["after"]["decision"],
            if action == "CONSENT_APPROVE" {
                "APPROVED"
            } else {
                "DENIED"
            },
        )?;
    }
    if object_type == "oauth_device_authorization"
        && (action == "DEVICE_APPROVE" || value["before"].get("id").is_none())
    {
        require_string(&value["before"]["status"], "PENDING")?;
        require_string(
            &value["after"]["status"],
            if action == "DEVICE_APPROVE" {
                "APPROVED"
            } else {
                "DENIED"
            },
        )?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Mode {
    Create,
    Update,
    Delete,
    Upsert,
}

fn pair(
    value: &mut Value,
    before: &str,
    after: &str,
    mode: Mode,
    resolve: &Resolver<'_>,
) -> Result<()> {
    match mode {
        Mode::Create if !value["before"].is_null() => {
            return Err(Error("audit creation has before state"))
        }
        Mode::Delete if !value["after"].is_null() => {
            return Err(Error("audit deletion has after state"))
        }
        _ => {}
    }
    if !matches!(mode, Mode::Create) && !(matches!(mode, Mode::Upsert) && value["before"].is_null())
    {
        shape(&mut value["before"], before, resolve)?;
    }
    if !matches!(mode, Mode::Delete) {
        shape(&mut value["after"], after, resolve)?;
    }
    Ok(())
}

/// The schema language is private static source code, never supplied by input.
/// `key?` is an absent optional key; `?type` permits an explicit JSON null.
fn shape(value: &mut Value, spec: &str, resolve: &Resolver<'_>) -> Result<()> {
    let fields: Vec<_> = spec
        .split_whitespace()
        .map(|field| {
            field
                .split_once(':')
                .ok_or(Error("invalid static audit schema"))
        })
        .collect::<Result<_>>()?;
    let object = value
        .as_object_mut()
        .ok_or(Error("audit payload is not an object"))?;
    if object.keys().any(|key| {
        !fields
            .iter()
            .any(|(name, _)| name.trim_end_matches('?') == key)
    }) {
        return Err(Error("unknown audit payload field"));
    }
    for (name, kind) in fields {
        let optional = name.ends_with('?');
        let Some(field) = object.get_mut(name.trim_end_matches('?')) else {
            if optional {
                continue;
            }
            return Err(Error("missing audit payload field"));
        };
        if kind.starts_with('?') && field.is_null() {
            continue;
        }
        let kind = kind.trim_start_matches('?');
        if let Some(authority) = kind.strip_prefix("u=") {
            uuid_value(field, authority, resolve)?;
            continue;
        }
        let valid = match kind {
            "s" => field.is_string(),
            "b" => field.is_boolean(),
            "i" => field.as_i64().is_some() || field.as_u64().is_some(),
            "n" => field.as_u64().is_some(),
            "h" => field
                .as_str()
                .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())),
            "a" => field.is_array(),
            "o" => field.is_object(),
            "v" => true,
            "ss" => field
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_string)),
            _ => false,
        };
        if !valid {
            return Err(Error("invalid audit payload field type"));
        }
    }
    Ok(())
}

fn parse_uuid(value: &Value) -> Result<Uuid> {
    let raw = value.as_str().ok_or(Error("audit UUID is not text"))?;
    let uuid = Uuid::parse_str(raw).map_err(|_| Error("invalid audit UUID"))?;
    if uuid.hyphenated().to_string() != raw.to_ascii_lowercase() {
        return Err(Error("audit UUID is not canonical"));
    }
    Ok(uuid)
}

fn uuid_value(value: &mut Value, authority: &str, resolve: &Resolver<'_>) -> Result<()> {
    let old = parse_uuid(value)?;
    if old.is_nil() {
        return Err(Error("nil audit entity UUID"));
    }
    *value = Value::String(resolve(authority, old)?.to_string());
    Ok(())
}

fn array_mut(value: &mut Value) -> Result<&mut Vec<Value>> {
    value
        .as_array_mut()
        .ok_or(Error("invalid audit collection"))
}

fn require_string(value: &Value, expected: &str) -> Result<()> {
    if value.as_str() != Some(expected) {
        return Err(Error("audit result differs from producer schema"));
    }
    Ok(())
}

const APP: &str = "id:u=platform.application name:s code:s entryType:s entryConfigured:b runtimeMode:s status:s sortOrder:i menuCount:n";
const APP_UPDATED: &str = "id:u=platform.application name:s code:s entryType:s entryConfigured:b runtimeMode:s status:s sortOrder:i menuCount:n changedFields:ss";
const URL_ENTRY: &str = "applicationId:u=platform.application environment:s proxyEnabled:b revision:u=platform.application_url_revision entryConfigured:b";
const RELEASE: &str = "id:u=platform.release name:s applicationId:u=platform.application environment:s entryType:s resourceCount:n artifactBound:b validationStatus:s checkedAt:?s";
const DEPLOYMENT: &str = "id:u=platform.application name:s environment:s activeReleaseId:u=platform.release previousReleaseId:?u=platform.release entryType:s";
const DEPARTMENT: &str = "id:u=platform.department organizationId:u=platform.organization parentId:?u=platform.department name:s code:s sortOrder:i status:s";
const DEPARTMENT_UPDATED: &str = "id:u=platform.department organizationId:u=platform.organization parentId:?u=platform.department name:s code:s sortOrder:i status:s changedFields:ss";
const MENU: &str = "id:u=platform.menu applicationId:u=platform.application parentId:?u=platform.menu type:s name:s permission:s sortOrder:i visible:b";
const MENU_UPDATED: &str = "id:u=platform.menu applicationId:u=platform.application parentId:?u=platform.menu type:s name:s permission:s sortOrder:i visible:b changedFields:ss";
const ROLE: &str = "id:u=platform.role name:s code:s status:s builtIn:b";
const ROLE_UPDATED: &str = "id:u=platform.role name:s code:s status:s builtIn:b changedFields:ss";
const ROLE_MENUS: &str = "id:u=platform.role name:s menuCount:n menuIdsSha256:h";
const ROLE_USERS: &str = "id:u=platform.role name:s userCount:n userBindingsSha256:h";
const TENANT_CONFIG: &str = "id:u=platform.tenant name:s code:s tenantType:s status:s isPlatform:b expiresAt:?s configurationHash:h";
const TENANT_FULL: &str = "id:u=platform.tenant name:s code:s tenantType:s status:s isPlatform:b expiresAt:?s configurationHash:h organizationCount:n memberCount:n roleCount:n roleAssignmentCount:n applicationCount:n menuCount:n authorizationHash:h";
const TENANT_CREATED: &str = "id:u=platform.tenant name:s code:s tenantType:s status:s isPlatform:b expiresAt:?s configurationHash:h organizationCount:n memberCount:n roleCount:n roleAssignmentCount:n applicationCount:n menuCount:n authorizationHash:h rootOrganizationId:u=platform.organization administratorUserId?:u=platform.user administratorCreated?:b";
const TENANT_ADMIN: &str = "id:u=platform.tenant name:s code:s tenantType:s status:s isPlatform:b expiresAt:?s configurationHash:h organizationCount:n memberCount:n roleCount:n roleAssignmentCount:n applicationCount:n menuCount:n authorizationHash:h administratorUserId:u=platform.user administratorCreated:b";
const TENANT_AUTH: &str =
    "id:u=platform.tenant name:s applicationCount:n menuCount:n authorizationHash:h";
const ORG_CONFIG: &str = "id:u=platform.organization tenantId:u=platform.tenant parentId:?u=platform.organization name:s code:s organizationType:s status:s expiresAt:?s configurationHash:h";
const ORG_FULL: &str = "id:u=platform.organization tenantId:u=platform.tenant parentId:?u=platform.organization name:s code:s organizationType:s status:s expiresAt:?s configurationHash:h memberCount:n roleCount:n roleAssignmentCount:n applicationCount:n menuCount:n subtreeOrganizationCount:n subtreeMemberCount:n subtreeRoleCount:n authorizationHash:h";
const ORG_CREATED: &str = "id:u=platform.organization tenantId:u=platform.tenant parentId:?u=platform.organization name:s code:s organizationType:s status:s expiresAt:?s configurationHash:h memberCount:n roleCount:n roleAssignmentCount:n applicationCount:n menuCount:n subtreeOrganizationCount:n subtreeMemberCount:n subtreeRoleCount:n authorizationHash:h administratorUserId?:u=platform.user administratorCreated?:b";
const ORG_ADMIN: &str = "id:u=platform.organization tenantId:u=platform.tenant parentId:?u=platform.organization name:s code:s organizationType:s status:s expiresAt:?s configurationHash:h memberCount:n roleCount:n roleAssignmentCount:n applicationCount:n menuCount:n subtreeOrganizationCount:n subtreeMemberCount:n subtreeRoleCount:n authorizationHash:h administratorUserId:u=platform.user administratorCreated:b";
const ORG_AUTH: &str = "id:u=platform.organization tenantId:u=platform.tenant name:s applicationCount:n menuCount:n authorizationHash:h";
const USER: &str =
    "username:s realName:?s status:s gender:?s emailSet:b phoneSet:b sessionVersion:n";
const USER_UPDATED: &str = "username:s realName:?s status:s gender:?s emailSet:b phoneSet:b sessionVersion:n updatedFields?:ss";
const USER_RESET: &str = "username:s realName:?s status:s gender:?s emailSet:b phoneSet:b sessionVersion:n passwordChanged:b revokedSessions:n";
const USER_SESSIONS: &str =
    "id:u=platform.user name:s sessionVersion:n unrevokedSessionCount:n unrevokedCredentialCount:n";
const CLIENT: &str = "id:u=platform.oauth_client name:s clientId:s clientType:s requirePkce:b status:s applicationId:?u=platform.application grantTypes:s scopeCount:n redirectUriCount:n accessTokenValidity:i refreshTokenValidity:i";
const CLIENT_UPDATED: &str = "id:u=platform.oauth_client name:s clientId:s clientType:s requirePkce:b status:s applicationId:?u=platform.application grantTypes:s scopeCount:n redirectUriCount:n accessTokenValidity:i refreshTokenValidity:i redirectUrisChanged?:b scopesChanged?:b descriptionChanged?:b credentialRotated?:b";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const OLD: &str = "00000000-0000-4000-8000-000000000001";
    const OTHER: &str = "00000000-0000-4000-8000-000000000002";
    const NEW: &str = "01900000-0000-7000-8000-000000000001";
    const NEW_OTHER: &str = "01900000-0000-7000-8000-000000000002";

    fn registry() -> Value {
        serde_json::from_str(include_str!("../../../uuid-references.json"))
            .expect("registry fixture")
    }
    fn resolve(_: &str, old: Uuid) -> Result<Uuid> {
        match old.to_string().as_str() {
            OLD => Ok(Uuid::parse_str(NEW).expect("fixture")),
            OTHER => Ok(Uuid::parse_str(NEW_OTHER).expect("fixture")),
            _ => Err(Error("fixture mapping missing")),
        }
    }
    fn audit(kind: &str, before: Value, after: Value) -> Value {
        json!({"objectType":kind,"objectId":OLD,"before":before,"after":after})
    }

    #[test]
    fn same_action_device_producers_have_distinct_closed_shapes() {
        let mut user = audit(
            "oauth_device_authorization",
            json!({"clientId":OLD,"status":"PENDING"}),
            json!({"clientId":OLD,"status":"DENIED"}),
        );
        transform_with("DEVICE_DENY", &mut user, &resolve, &registry()).expect("user projection");
        assert_eq!(user["objectId"], NEW);
        assert_eq!(user["before"]["clientId"], OLD);
        assert!(user["before"].get("id").is_none());
        let mut admin = audit(
            "oauth_device_authorization",
            json!({"id":OLD,"name":"c","clientId":OLD,"status":"PENDING"}),
            json!({"id":OLD,"name":"c","clientId":OLD,"status":"EXPIRED"}),
        );
        transform_with("DEVICE_DENY", &mut admin, &resolve, &registry()).expect("admin projection");
        assert_eq!(admin["before"]["id"], NEW);
        assert_eq!(admin["after"]["clientId"], OLD);
    }

    #[test]
    fn oauth_consent_and_logout_do_not_manufacture_nested_ids() {
        for (kind, action, before, after) in [
            (
                "oauth_consent",
                "CONSENT_DENY",
                json!({"clientId":OLD,"consumed":false}),
                json!({"clientId":OLD,"consumed":true,"decision":"DENIED"}),
            ),
            (
                "platform_session",
                "LOGOUT",
                json!({"username":OLD,"revoked":false}),
                json!({"username":OLD,"revoked":true}),
            ),
        ] {
            let mut value = audit(kind, before.clone(), after.clone());
            transform_with(action, &mut value, &resolve, &registry()).expect("projection");
            assert_eq!(value["objectId"], NEW);
            assert_eq!(value["before"], before);
            assert_eq!(value["after"], after);
        }
    }

    #[test]
    fn historical_digest_remains_original_and_unknown_shape_is_atomic() {
        let hash = "a".repeat(64);
        let state = json!({"id":OLD,"name":"r","menuCount":2,"menuIdsSha256":hash});
        let mut value = audit("role_menus", state.clone(), state);
        transform_with("ROLE_MENUS", &mut value, &resolve, &registry()).expect("projection");
        assert_eq!(value["after"]["menuIdsSha256"], hash);
        let mut unknown = value;
        unknown["after"]["unexpected"] = json!({"id":OTHER});
        let saved = unknown.clone();
        assert!(transform_with("ROLE_MENUS", &mut unknown, &resolve, &registry()).is_err());
        assert_eq!(unknown, saved);
    }

    #[test]
    fn wrong_action_missing_mapping_and_aggregate_misuse_fail_closed() {
        let mut value = audit(
            "platform_session",
            json!({"username":"u","revoked":false}),
            json!({"username":"u","revoked":true}),
        );
        assert!(transform_with("UPDATE_USER", &mut value, &resolve, &registry()).is_err());
        let original = value.clone();
        assert!(transform_with(
            "LOGOUT",
            &mut value,
            &|_, _| Err(Error("missing")),
            &registry()
        )
        .is_err());
        assert_eq!(value, original);
        let mut purge = json!({"objectType":"oauth_history","objectId":Uuid::nil(),"before":{"eligibleTokenCount":1,"eligibleDeviceCount":0},"after":{"eligibleTokenCount":0,"eligibleDeviceCount":0}});
        let saved = purge.clone();
        transform_with(
            "OAUTH_PURGE",
            &mut purge,
            &|_, _| Err(Error("sentinel must not resolve")),
            &registry(),
        )
        .expect("sentinel");
        assert_eq!(purge, saved);
        assert!(transform_with("TOKEN_REVOKED", &mut purge, &resolve, &registry()).is_err());
    }

    #[test]
    fn target_membership_and_url_revision_keep_separate_authorities() {
        let calls = std::cell::RefCell::new(Vec::new());
        let mapper = |authority: &str, old: Uuid| {
            calls.borrow_mut().push(authority.to_owned());
            resolve(authority, old)
        };
        let mut entry = audit(
            "application_url_entry",
            Value::Null,
            json!({"applicationId":OLD,"environment":"DEV","proxyEnabled":true,"revision":OTHER,"entryConfigured":true}),
        );
        transform_with(
            "CONFIGURE_APPLICATION_URL_ENTRY",
            &mut entry,
            &mapper,
            &registry(),
        )
        .expect("URL projection");
        assert_eq!(entry["after"]["revision"], NEW_OTHER);
        assert!(calls
            .borrow()
            .iter()
            .any(|a| a == "platform.application_url_revision"));
        assert!(entry["before"].is_null());
    }

    #[test]
    fn real_membership_and_grant_shapes_map_nested_ids_without_rehashing() {
        let before = json!({"username":OLD,"items":[{"organizationId":OTHER,"organizationName":OLD,"departmentId":null,"positionSet":false}]});
        let after = json!({"username":OLD,"items":[{"organizationId":OTHER,"organizationName":OLD,"departmentId":OLD,"positionSet":true}],"changedOrganizationIds":[OTHER],"revokedSessions":1});
        let mut memberships = audit("user_memberships", before, after);
        transform_with("UPDATE_USER", &mut memberships, &resolve, &registry())
            .expect("membership projection");
        assert_eq!(
            memberships["after"]["items"][0]["organizationId"],
            NEW_OTHER
        );
        assert_eq!(memberships["after"]["items"][0]["departmentId"], NEW);
        assert_eq!(memberships["after"]["items"][0]["organizationName"], OLD);
        assert!(memberships["before"]["items"][0]["departmentId"].is_null());
        assert_eq!(memberships["after"]["changedOrganizationIds"][0], NEW_OTHER);
        let version = "b".repeat(64);
        let state = json!({"target":{"tenantId":OTHER,"organizationId":OTHER,"userId":OTHER,"membershipId":OLD,"applicationId":OTHER},"version":version,"grants":[{"id":OTHER,"resource":"ticket","action":"read","scope":"SELF"}]});
        let mut grant = audit("membership_data_grants", state.clone(), state);
        transform_with("DATA_GRANT_CHANGE", &mut grant, &resolve, &registry())
            .expect("grant projection");
        assert_eq!(grant["after"]["target"]["membershipId"], NEW);
        assert_eq!(grant["after"]["grants"][0]["id"], NEW_OTHER);
        assert_eq!(grant["after"]["version"], version);
    }

    #[test]
    fn known_generic_producer_snapshots_are_accepted_and_remain_closed() {
        let hash = "c".repeat(64);
        let fixtures = [
            (
                "application",
                "CREATE_APPLICATION",
                json!({"id":OLD,"name":"App","code":"app","entryType":"HTML","entryConfigured":true,"runtimeMode":"TRUSTED","status":"ENABLED","sortOrder":0,"menuCount":0}),
            ),
            (
                "application_release",
                "REGISTER_APPLICATION_RELEASE",
                json!({"id":OLD,"name":"1.0.0","applicationId":OTHER,"environment":"DEV","entryType":"HTML","resourceCount":2,"artifactBound":true,"validationStatus":"PENDING","checkedAt":null}),
            ),
            (
                "department",
                "CREATE_DEPARTMENT",
                json!({"id":OLD,"organizationId":OTHER,"parentId":null,"name":"Support","code":"s","sortOrder":0,"status":"ENABLED"}),
            ),
            (
                "menu",
                "CREATE_MENU",
                json!({"id":OLD,"applicationId":OTHER,"parentId":null,"type":"MENU","name":"Tickets","permission":"ticket:read","sortOrder":0,"visible":true}),
            ),
            (
                "role",
                "CREATE_ROLE",
                json!({"id":OLD,"name":"Operator","code":"op","status":"ENABLED","builtIn":false}),
            ),
            (
                "user",
                "CREATE_USER",
                json!({"username":OLD,"realName":null,"status":"ENABLED","gender":null,"emailSet":false,"phoneSet":false,"sessionVersion":0}),
            ),
            (
                "tenant",
                "CREATE_TENANT",
                json!({"id":OLD,"name":"Tenant","code":"t","tenantType":"COMPANY","status":"ENABLED","isPlatform":false,"expiresAt":null,"configurationHash":hash,"organizationCount":1,"memberCount":0,"roleCount":1,"roleAssignmentCount":0,"applicationCount":1,"menuCount":0,"authorizationHash":hash,"rootOrganizationId":OTHER}),
            ),
            (
                "organization",
                "CREATE_ORGANIZATION",
                json!({"id":OLD,"tenantId":OTHER,"parentId":null,"name":"Store","code":"s","organizationType":"STORE","status":"ENABLED","expiresAt":null,"configurationHash":hash,"memberCount":0,"roleCount":0,"roleAssignmentCount":0,"applicationCount":0,"menuCount":0,"subtreeOrganizationCount":1,"subtreeMemberCount":0,"subtreeRoleCount":0,"authorizationHash":hash}),
            ),
        ];
        for (kind, action, after) in fixtures {
            let mut value = audit(kind, Value::Null, after);
            transform_with(action, &mut value, &resolve, &registry()).expect(kind);
            assert_eq!(value["objectId"], NEW);
            let mut invalid = value;
            invalid["after"]["unregisteredExtension"] = json!(true);
            assert!(transform_with(action, &mut invalid, &resolve, &registry()).is_err());
        }
    }

    #[test]
    fn credentials_use_distinct_authorities_even_when_source_strings_match() {
        let mapper = |authority: &str, _: Uuid| {
            let id = match authority {
                "platform.oauth_token" => NEW,
                "platform.application" => NEW_OTHER,
                "platform.oauth_jti" => "01900000-0000-7000-8000-000000000003",
                _ => return Err(Error("unexpected fixture authority")),
            };
            Ok(Uuid::parse_str(id).expect("fixture"))
        };
        let mut token = audit(
            "app_token",
            Value::Null,
            json!({"applicationId":OLD,"clientId":OLD,"audience":"app","jti":OLD,"expiresAt":"2026-10-24T00:00:00Z"}),
        );
        transform_with("TOKEN_ISSUED", &mut token, &mapper, &registry()).expect("token history");
        assert_eq!(token["objectId"], NEW);
        assert_eq!(token["after"]["applicationId"], NEW_OTHER);
        assert_eq!(
            token["after"]["jti"],
            "01900000-0000-7000-8000-000000000003"
        );
        assert_eq!(token["after"]["clientId"], OLD);
    }

    #[test]
    fn public_adapter_accepts_attested_credential_history_without_allocation() {
        let mut mapping = super::super::super::mapping::TypedMap::new(
            1_800_000_000_000,
            ["platform.platform_session".to_owned()].into(),
        )
        .expect("mapping policy");
        let old = Uuid::parse_str(OLD).expect("fixture");
        let target = mapping
            .attest_tombstone(
                "platform.platform_session",
                old,
                None,
                &"d".repeat(64),
                "excluded_credential",
            )
            .expect("attestation");
        assert!(mapping.resolve("platform.platform_session", old).is_err());
        let mut value = audit(
            "platform_session",
            json!({"username":"u","revoked":false}),
            json!({"username":"u","revoked":true}),
        );
        transform("LOGOUT", &mut value, &mapping, &registry()).expect("attested projection");
        assert_eq!(value["objectId"], target.to_string());
        assert_eq!(mapping.len(), 1);
    }

    #[test]
    fn only_validated_owner_deletions_prove_deleted_entities() {
        let state = json!({"id":OLD,"name":"App","code":"app","entryType":"HTML","entryConfigured":true,"runtimeMode":"TRUSTED","status":"ENABLED","sortOrder":0,"menuCount":0});
        let mut value = audit("application", state, Value::Null);
        let source = value.clone();
        assert_eq!(
            deletion_proof("DELETE_APPLICATION", &value, &registry()).expect("proof"),
            Some((
                "platform.application",
                Uuid::parse_str(OLD).expect("fixture")
            ))
        );
        assert_eq!(value, source);
        assert!(deletion_proof("DELETE_USER", &value, &registry())
            .expect("unrelated")
            .is_none());
        value["before"]["id"] = json!(OTHER);
        assert!(deletion_proof("DELETE_APPLICATION", &value, &registry()).is_err());
        let user = audit(
            "user",
            json!({"username":"u","realName":null,"status":"ENABLED","gender":null,"emailSet":false,"phoneSet":false,"sessionVersion":0}),
            Value::Null,
        );
        assert_eq!(
            deletion_proof("DELETE_USER", &user, &registry())
                .expect("user proof")
                .expect("owner")
                .0,
            "platform.user"
        );
        let mut bad = user;
        bad["after"] = json!({});
        assert!(deletion_proof("DELETE_USER", &bad, &registry()).is_err());
    }

    #[test]
    fn credential_attestations_exclude_arbitrary_snapshot_references() {
        let token = audit(
            "app_token",
            Value::Null,
            json!({"applicationId":OTHER,"clientId":OTHER,"audience":"app","jti":OTHER,"expiresAt":"2026-10-24T00:00:00Z"}),
        );
        let proofs = credential_proofs("TOKEN_ISSUED", &token, &registry()).expect("token proof");
        assert_eq!(
            proofs,
            vec![
                (
                    "platform.oauth_token",
                    Uuid::parse_str(OLD).expect("fixture")
                ),
                (
                    "platform.oauth_jti",
                    Uuid::parse_str(OTHER).expect("fixture")
                )
            ]
        );
        let context = audit(
            "platform_session",
            json!({"previousSessionId":OTHER,"organizationId":OTHER,"tenantId":OTHER,"username":"u"}),
            json!({"organizationId":OTHER,"organizationName":"Org","tenantId":OTHER,"username":"u"}),
        );
        let proofs =
            credential_proofs("CONTEXT_SWITCH", &context, &registry()).expect("session proof");
        assert_eq!(
            proofs,
            vec![(
                "platform.platform_session",
                Uuid::parse_str(OLD).expect("fixture")
            )]
        );
        let mut unknown = token;
        unknown["after"]["otherCredentialId"] = json!(OTHER);
        assert!(credential_proofs("TOKEN_ISSUED", &unknown, &registry()).is_err());
    }
}
