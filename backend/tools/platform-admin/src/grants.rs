//! Offline grant intent and bounded orchestration through the public management API.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::SafeError;
use crate::contract::{KnownApp, is_uuid, read_json};
use crate::http::ManagementApi;

const INVALID: &str = "INVALID_GRANT_MANIFEST";
const PRECONDITION: &str = "GRANT_PRECONDITION_FAILED";
const WARNINGS: [&str; 7] = [
    "Role menu and user PUTs replace complete sets without CAS. Pause concurrent administration of every affected role until verification finishes.",
    "Approve every current role member: added menus affect the complete existing and requested audience, including inherited role permissions.",
    "Tenant and organization requests contain only positive deltas; platform consistency cleanup can remove already inconsistent authorizations.",
    "The menu management API does not expose menu status. The formal data-grant API checks enabled capability menus; protected application requests enforce current menu state.",
    "Direct memberships are verified and never created, replaced or removed. Membership recreation requires a new manifest and live query.",
    "Data grants require independent read/write permissions, an actual platform-root session and explicit server policy; a session or broad management permission cannot substitute.",
    "On any failure or unconfirmed write, inspect the exact targets before deciding on a fresh plan. There is no automatic retry, rollback or deletion.",
];

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema_version: String,
    application_code: KnownApp,
    application_id: String,
    menu_ids: BTreeMap<String, String>,
    #[serde(default)]
    targets: Vec<Target>,
}

#[derive(Clone, Deserialize, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Grant {
    resource: String,
    action: String,
    scope: String,
}

type Request<'a> = dyn FnMut(&str, &str, Option<&Value>) -> Result<Value, SafeError> + 'a;

fn require(condition: bool, category: &'static str) -> Result<(), SafeError> {
    if condition {
        Ok(())
    } else {
        Err(SafeError::new(category))
    }
}

fn normalized_id(id: &mut String) -> Result<(), SafeError> {
    require(
        is_uuid(id) && id != "00000000-0000-0000-0000-000000000000",
        INVALID,
    )?;
    id.make_ascii_lowercase();
    Ok(())
}

fn load(
    path: &Path,
    selected: Option<KnownApp>,
    selected_id: Option<&str>,
) -> Result<Manifest, SafeError> {
    let manifest = validate(read_json(path, INVALID)?)?;
    require(
        selected.is_none_or(|app| app == manifest.application_code),
        "APPLICATION_SELECTION_CONFLICT",
    )?;
    require(
        selected_id.is_none_or(|id| id.eq_ignore_ascii_case(&manifest.application_id)),
        "APPLICATION_SELECTION_CONFLICT",
    )?;
    Ok(manifest)
}

fn validate(value: Value) -> Result<Manifest, SafeError> {
    let mut manifest: Manifest =
        serde_json::from_value(value).map_err(|_| SafeError::new(INVALID))?;
    require(manifest.schema_version == "1.0", INVALID)?;
    normalized_id(&mut manifest.application_id)?;
    require(
        manifest.menu_ids.len() == manifest.application_code.menus().len()
            && manifest
                .application_code
                .menus()
                .iter()
                .all(|menu| manifest.menu_ids.contains_key(menu.key)),
        INVALID,
    )?;
    let mut menus = BTreeSet::new();
    for id in manifest.menu_ids.values_mut() {
        normalized_id(id)?;
        require(menus.insert(id.clone()), INVALID)?;
    }
    let mut memberships = BTreeSet::new();
    let mut user_orgs = BTreeSet::new();
    let mut org_tenants = BTreeMap::new();
    let mut role_owners = BTreeMap::new();
    for target in &mut manifest.targets {
        for id in [
            &mut target.tenant_id,
            &mut target.organization_id,
            &mut target.role_id,
            &mut target.user_id,
            &mut target.membership_id,
        ] {
            normalized_id(id)?;
        }
        require(
            memberships.insert(target.membership_id.clone())
                && user_orgs.insert((target.user_id.clone(), target.organization_id.clone())),
            INVALID,
        )?;
        if let Some(previous) =
            org_tenants.insert(target.organization_id.clone(), target.tenant_id.clone())
        {
            require(previous == target.tenant_id, INVALID)?;
        }
        let owner = (target.tenant_id.clone(), target.organization_id.clone());
        if let Some(previous) = role_owners.insert(target.role_id.clone(), owner.clone()) {
            require(previous == owner, INVALID)?;
        }
        let mut permissions = BTreeSet::new();
        for permission in &target.permissions {
            require(
                manifest
                    .application_code
                    .permissions()
                    .contains(&permission.as_str())
                    && permissions.insert(permission),
                INVALID,
            )?;
        }
        let mut pairs = BTreeSet::new();
        for grant in &target.data_grants {
            require(
                manifest
                    .application_code
                    .permissions()
                    .contains(&format!("{}:{}", grant.resource, grant.action).as_str())
                    && valid_scope(&grant.scope)
                    && pairs.insert((&grant.resource, &grant.action)),
                INVALID,
            )?;
        }
    }
    Ok(manifest)
}

fn valid_scope(scope: &str) -> bool {
    matches!(scope, "SELF" | "ORG" | "ORG_SUBTREE" | "TENANT")
}

fn identity(manifest: &Manifest, target: &Target) -> Value {
    json!({"tenantId":target.tenant_id,"organizationId":target.organization_id,
        "userId":target.user_id,"membershipId":target.membership_id,"applicationId":manifest.application_id})
}

fn query(manifest: &Manifest, target: &Target) -> String {
    // Every value has already passed closed UUID validation, so no escaping or arbitrary URL is accepted.
    format!(
        "/api/data-grants?tenantId={}&organizationId={}&userId={}&membershipId={}&applicationId={}",
        target.tenant_id,
        target.organization_id,
        target.user_id,
        target.membership_id,
        manifest.application_id
    )
}

fn selected_menus(manifest: &Manifest, target: &Target) -> BTreeSet<String> {
    manifest
        .application_code
        .menus()
        .iter()
        .filter(|menu| {
            menu.key == "root"
                || target
                    .permissions
                    .iter()
                    .any(|permission| permission == menu.permission)
        })
        .filter_map(|menu| manifest.menu_ids.get(menu.key).cloned())
        .collect()
}

#[derive(Default)]
struct RoleIntent {
    menus: BTreeSet<String>,
    users: BTreeSet<String>,
}

#[derive(Default)]
struct Intent {
    tenants: BTreeMap<String, BTreeSet<String>>,
    organizations: BTreeMap<String, BTreeSet<String>>,
    roles: BTreeMap<String, RoleIntent>,
}

fn intent(manifest: &Manifest) -> Intent {
    let mut intent = Intent::default();
    for target in &manifest.targets {
        let menus = selected_menus(manifest, target);
        intent
            .tenants
            .entry(target.tenant_id.clone())
            .or_default()
            .extend(menus.clone());
        intent
            .organizations
            .entry(target.organization_id.clone())
            .or_default()
            .extend(menus.clone());
        let role = intent.roles.entry(target.role_id.clone()).or_default();
        role.menus.extend(menus);
        role.users.insert(target.user_id.clone());
    }
    intent
}

fn authorization_body(application_id: &str, menus: &BTreeSet<String>) -> Value {
    json!({"applications":[{"applicationId":application_id,"authorized":true}],
        "menus":menus.iter().map(|id|json!({"menuId":id,"authorized":true})).collect::<Vec<_>>()})
}

/// Produces a strict offline review plan. This function never constructs a network client.
pub(crate) fn plan(
    path: &Path,
    selected: Option<KnownApp>,
    selected_id: Option<&str>,
) -> Result<Value, SafeError> {
    let manifest = load(path, selected, selected_id)?;
    let intent = intent(&manifest);
    let requests = intent
        .tenants
        .iter()
        .map(|(id, menus)| ("tenants", id, menus))
        .chain(
            intent
                .organizations
                .iter()
                .map(|(id, menus)| ("organizations", id, menus)),
        )
        .map(|(kind, id, menus)| {
            json!({"method":"PUT","path":format!("/api/{kind}/{id}/authorizations"),
            "body":authorization_body(&manifest.application_id,menus)})
        })
        .collect::<Vec<_>>();
    Ok(
        json!({"ok":true,"schemaVersion":"1.0","applicationCode":manifest.application_code,"applicationId":manifest.application_id,
        "mode":"PLAN","offline":true,"writes":false,"externalWrites":false,"requests":requests,
        "roleMerges":intent.roles.iter().map(|(id,role)|json!({"roleId":id,"addMenuIds":role.menus,"addUserIds":role.users,
            "readCurrentSetsBeforeWrite":true,"preserveExisting":true,"requiresExclusiveAdministrationWindow":true})).collect::<Vec<_>>(),
        "ownershipChecks":manifest.targets.iter().map(|target|json!({"target":identity(&manifest,target),"roleId":target.role_id})).collect::<Vec<_>>(),
        "dataGrants":manifest.targets.iter().map(|target|json!({"target":identity(&manifest,target),"upsert":target.data_grants,"remove":[],
            "requiresLiveVersion":true,"sequence":["GET","POST_PREVIEW","PATCH_SAME_VERSION"],"readPath":query(&manifest,target)})).collect::<Vec<_>>(),
        "warnings":WARNINGS}),
    )
}

/// Applies the reviewed intent using only the public API; never retries or rolls back writes.
pub(crate) fn apply(
    path: &Path,
    api: &ManagementApi,
    selected: Option<KnownApp>,
    selected_id: Option<&str>,
) -> Result<Value, SafeError> {
    let manifest = load(path, selected, selected_id)?;
    apply_with(&manifest, &mut |method, path, body| {
        api.request(method, path, body)
    })
}

fn apply_with(manifest: &Manifest, request: &mut Request<'_>) -> Result<Value, SafeError> {
    let mut completed = Vec::new();
    let mut attempted = Value::Null;
    let result = execute(manifest, request, &mut completed, &mut attempted);
    result.map_err(|error| {
        let error = if !completed.is_empty() || !attempted.is_null() { error.uncertain() } else { error };
        error.with_completed(json!({"completedCount":completed.len(),"completed":completed,
            "attempted":attempted,"reconciliationRequired":true,"nextAction":"INSPECT_AND_RECONCILE_BEFORE_RETRY"}))
    })
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, SafeError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| SafeError::new(PRECONDITION))
}

fn id(value: &Value, key: &str) -> Result<String, SafeError> {
    let value = text(value, key)?;
    require(
        is_uuid(value) && value != "00000000-0000-0000-0000-000000000000",
        PRECONDITION,
    )?;
    Ok(value.to_ascii_lowercase())
}

fn array(value: &Value) -> Result<&Vec<Value>, SafeError> {
    value.as_array().ok_or_else(|| SafeError::new(PRECONDITION))
}

fn active(value: &Value, now: DateTime<Utc>) -> Result<(), SafeError> {
    require(text(value, "status")? == "ENABLED", PRECONDITION)?;
    match value.get("expiresAt") {
        Some(Value::Null) => Ok(()),
        Some(Value::String(expiry)) => {
            let expiry =
                DateTime::parse_from_rfc3339(expiry).map_err(|_| SafeError::new(PRECONDITION))?;
            require(expiry > now, PRECONDITION)
        }
        _ => Err(SafeError::new(PRECONDITION)),
    }
}

fn flatten(value: &Value) -> Result<Vec<&Value>, SafeError> {
    let mut pending: Vec<&Value> = array(value)?.iter().collect();
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    while let Some(node) = pending.pop() {
        require(
            result.len() < 10000 && seen.insert(id(node, "id")?),
            PRECONDITION,
        )?;
        if let Some(children) = node.get("children") {
            pending.extend(array(children)?);
        }
        result.push(node);
    }
    Ok(result)
}

fn find<'a>(records: &'a [Value], target: &str) -> Result<&'a Value, SafeError> {
    let matches = records
        .iter()
        .filter(|value| {
            value
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| id.eq_ignore_ascii_case(target))
        })
        .collect::<Vec<_>>();
    require(matches.len() == 1, PRECONDITION)?;
    Ok(matches[0])
}

fn paged(request: &mut Request<'_>, path: &str) -> Result<Vec<Value>, SafeError> {
    let mut records = Vec::new();
    let mut seen = BTreeSet::new();
    let mut total = None;
    for page in 1..=50_u64 {
        let separator = if path.contains('?') { '&' } else { '?' };
        let response = request(
            "GET",
            &format!("{path}{separator}page={page}&size=200"),
            None,
        )?;
        let count = response
            .get("total")
            .and_then(Value::as_u64)
            .ok_or_else(|| SafeError::new(PRECONDITION))?;
        require(
            count <= 10000
                && total.is_none_or(|previous| previous == count)
                && response["page"] == page
                && response["size"] == 200,
            PRECONDITION,
        )?;
        total = Some(count);
        let batch = array(&response["records"])?;
        require(
            batch.len() <= 200 && records.len() + batch.len() <= count as usize,
            PRECONDITION,
        )?;
        for record in batch {
            require(seen.insert(id(record, "id")?), PRECONDITION)?;
            records.push(record.clone());
        }
        if records.len() == count as usize {
            return Ok(records);
        }
        require(!batch.is_empty(), PRECONDITION)?;
    }
    Err(SafeError::new(PRECONDITION))
}

fn check_menus(manifest: &Manifest, response: &Value) -> Result<(), SafeError> {
    let menus = flatten(response)?;
    for (index, expected) in manifest.application_code.menus().iter().enumerate() {
        let key = expected.key;
        let expected_id = &manifest.menu_ids[key];
        let matches = menus
            .iter()
            .copied()
            .filter(|menu| {
                menu["id"]
                    .as_str()
                    .is_some_and(|value| value.eq_ignore_ascii_case(expected_id))
            })
            .collect::<Vec<_>>();
        require(matches.len() == 1, PRECONDITION)?;
        let menu = matches[0];
        require(
            id(menu, "applicationId")? == manifest.application_id
                && text(menu, "permission")? == expected.permission
                && text(menu, "linkUrl")?.is_empty(),
            PRECONDITION,
        )?;
        require(
            text(menu, "type")? == expected.kind
                && text(menu, "path")? == expected.path
                && menu["visible"] == expected.visible
                && text(menu, "linkTarget")? == expected.target()
                && (manifest.application_code != KnownApp::Stores
                    || menu["sortOrder"].as_u64() == Some(index as u64)),
            PRECONDITION,
        )?;
        if key == "root" {
            require(menu.get("parentId") == Some(&Value::Null), PRECONDITION)?;
        } else {
            require(
                id(menu, "parentId")? == manifest.menu_ids["root"],
                PRECONDITION,
            )?;
        }
    }
    Ok(())
}

fn membership(request: &mut Request<'_>, target: &Target) -> Result<(), SafeError> {
    let response = request(
        "GET",
        &format!("/api/users/{}/memberships", target.user_id),
        None,
    )?;
    let records = array(&response)?;
    let mut seen = BTreeSet::new();
    for row in records {
        require(seen.insert(id(row, "membershipId")?), PRECONDITION)?;
    }
    let matches = records
        .iter()
        .filter(|row| {
            row["membershipId"]
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case(&target.membership_id))
        })
        .collect::<Vec<_>>();
    require(matches.len() == 1, PRECONDITION)?;
    require(
        id(matches[0], "tenantId")? == target.tenant_id
            && id(matches[0], "organizationId")? == target.organization_id,
        PRECONDITION,
    )
}

fn preflight(manifest: &Manifest, request: &mut Request<'_>) -> Result<(), SafeError> {
    if manifest.targets.is_empty() {
        return Ok(());
    }
    let app = request(
        "GET",
        &format!("/api/applications/{}", manifest.application_id),
        None,
    )?;
    require(
        id(&app, "id")? == manifest.application_id
            && text(&app, "code")? == manifest.application_code.code()
            && text(&app, "status")? == "ENABLED"
            && text(&app, "basePath")? == manifest.application_code.base_path()
            && text(&app, "runtimeMode")? == "TRUSTED"
            && text(&app, "entryType")? == "HTML",
        PRECONDITION,
    )?;
    let menus = request(
        "GET",
        &format!("/api/menus/tree?applicationId={}", manifest.application_id),
        None,
    )?;
    check_menus(manifest, &menus["menus"])?;
    let tenants = paged(request, "/api/tenants")?;
    let now = Utc::now();
    let mut orgs_by_tenant = BTreeMap::new();
    let mut roles_by_org = BTreeMap::new();
    let mut users_by_org = BTreeMap::new();
    for target in &manifest.targets {
        active(find(&tenants, &target.tenant_id)?, now)?;
        if !orgs_by_tenant.contains_key(&target.tenant_id) {
            let tree = request(
                "GET",
                &format!("/api/organizations/tree?tenantId={}", target.tenant_id),
                None,
            )?;
            orgs_by_tenant.insert(
                target.tenant_id.clone(),
                flatten(&tree)?.into_iter().cloned().collect::<Vec<_>>(),
            );
        }
        let org = find(&orgs_by_tenant[&target.tenant_id], &target.organization_id)?;
        require(id(org, "tenantId")? == target.tenant_id, PRECONDITION)?;
        active(org, now)?;
        let mut ancestor = org;
        let mut chain = BTreeSet::new();
        chain.insert(target.organization_id.clone());
        while ancestor.get("parentId") != Some(&Value::Null) {
            let parent = id(ancestor, "parentId")?;
            require(chain.insert(parent.clone()), PRECONDITION)?;
            ancestor = find(&orgs_by_tenant[&target.tenant_id], &parent)?;
            require(id(ancestor, "tenantId")? == target.tenant_id, PRECONDITION)?;
            active(ancestor, now)?;
        }
        if !roles_by_org.contains_key(&target.organization_id) {
            let filter = format!(
                "tenantId={}&organizationId={}",
                target.tenant_id, target.organization_id
            );
            roles_by_org.insert(
                target.organization_id.clone(),
                paged(request, &format!("/api/roles?{filter}"))?,
            );
            users_by_org.insert(
                target.organization_id.clone(),
                paged(request, &format!("/api/users?{filter}"))?,
            );
        }
        let role = find(&roles_by_org[&target.organization_id], &target.role_id)?;
        require(
            id(role, "tenantId")? == target.tenant_id
                && id(role, "organizationId")? == target.organization_id
                && text(role, "status")? == "ENABLED",
            PRECONDITION,
        )?;
        // Managed administrator roles synchronize their menus; never try to repair or overwrite them.
        require(
            role.get("builtIn")
                .and_then(Value::as_bool)
                .is_some_and(|builtin| {
                    !builtin || !matches!(role["code"].as_str(), Some("tenant_admin" | "org_admin"))
                }),
            PRECONDITION,
        )?;
        require(
            text(
                find(&users_by_org[&target.organization_id], &target.user_id)?,
                "status",
            )? == "ENABLED",
            PRECONDITION,
        )?;
        membership(request, target)?;
    }
    Ok(())
}

fn role_sets(
    request: &mut Request<'_>,
    role: &str,
) -> Result<(BTreeSet<String>, BTreeSet<String>), SafeError> {
    let menus = request("GET", &format!("/api/roles/{role}/menus"), None)?;
    let mut menu_ids = BTreeSet::new();
    for value in array(&menus["menuIds"])? {
        let value = value.as_str().ok_or_else(|| SafeError::new(PRECONDITION))?;
        require(
            is_uuid(value) && menu_ids.insert(value.to_ascii_lowercase()),
            PRECONDITION,
        )?;
    }
    let users = request("GET", &format!("/api/roles/{role}/users"), None)?;
    let mut user_ids = BTreeSet::new();
    for value in array(&users)? {
        require(user_ids.insert(id(value, "userId")?), PRECONDITION)?;
    }
    Ok((menu_ids, user_ids))
}

fn mark(kind: &str, id: &str) -> Value {
    json!({"kind":kind,"id":id})
}

fn write_ok(
    request: &mut Request<'_>,
    path: &str,
    body: &Value,
    operation: Value,
    completed: &mut Vec<Value>,
    attempted: &mut Value,
) -> Result<(), SafeError> {
    *attempted = operation.clone();
    let response = request("PUT", path, Some(body))?;
    require(response["ok"] == true, "GRANT_WRITE_RESULT_UNCONFIRMED")?;
    completed.push(operation);
    Ok(())
}

fn execute(
    manifest: &Manifest,
    request: &mut Request<'_>,
    completed: &mut Vec<Value>,
    attempted: &mut Value,
) -> Result<Value, SafeError> {
    preflight(manifest, request)?;
    let intent = intent(manifest);
    // Read all affected role audiences before any write, then read again immediately before replacement.
    let mut initial_roles = BTreeMap::new();
    for role in intent.roles.keys() {
        initial_roles.insert(role.clone(), role_sets(request, role)?);
    }
    // Read every owner before writes. An application that is already authorized
    // also permits an early grant read, so known conflicts fail before side effects.
    // Unprovisioned targets must wait for their functional authorization to exist.
    let mut initial_tenants = BTreeMap::new();
    let mut initial_orgs = BTreeMap::new();
    for (kind, items, states) in [
        ("tenants", &intent.tenants, &mut initial_tenants),
        ("organizations", &intent.organizations, &mut initial_orgs),
    ] {
        for target in items.keys() {
            let response = request("GET", &format!("/api/{kind}/{target}/authorizations"), None)?;
            states.insert(
                target.clone(),
                authorization_sets(&response, &manifest.application_id)?,
            );
        }
    }
    for target in &manifest.targets {
        if !target.data_grants.is_empty()
            && initial_tenants[&target.tenant_id].0
            && initial_orgs[&target.organization_id].0
        {
            let snapshot = request("GET", &query(manifest, target), None)?;
            grant_delta(manifest, target, &snapshot)?;
        }
    }
    for (kind, items, operation) in [
        ("tenants", &intent.tenants, "TENANT_AUTHORIZATION"),
        (
            "organizations",
            &intent.organizations,
            "ORGANIZATION_AUTHORIZATION",
        ),
    ] {
        for (target, menus) in items {
            let path = format!("/api/{kind}/{target}/authorizations");
            let existing = request("GET", &path, None)?;
            let existing = authorization_sets(&existing, &manifest.application_id)?;
            if !existing.0 || !menus.is_subset(&existing.1) {
                write_ok(
                    request,
                    &path,
                    &authorization_body(&manifest.application_id, menus),
                    mark(operation, target),
                    completed,
                    attempted,
                )?;
                let verified =
                    authorization_sets(&request("GET", &path, None)?, &manifest.application_id)?;
                require(
                    verified.0 && menus.is_subset(&verified.1) && existing.1.is_subset(&verified.1),
                    "GRANT_WRITE_RESULT_UNCONFIRMED",
                )?;
            }
            *attempted = Value::Null;
        }
    }
    for (role, additions) in &intent.roles {
        let before = role_sets(request, role)?;
        require(
            initial_roles[role] == before,
            "GRANT_CONCURRENT_ROLE_CHANGE",
        )?;
        let menus = before
            .0
            .union(&additions.menus)
            .cloned()
            .collect::<BTreeSet<_>>();
        let users = before
            .1
            .union(&additions.users)
            .cloned()
            .collect::<BTreeSet<_>>();
        if menus != before.0 {
            write_ok(
                request,
                &format!("/api/roles/{role}/menus"),
                &json!({"menuIds":menus}),
                mark("ROLE_MENUS", role),
                completed,
                attempted,
            )?;
        }
        if users != before.1 {
            write_ok(
                request,
                &format!("/api/roles/{role}/users"),
                &json!({"userIds":users}),
                mark("ROLE_USERS", role),
                completed,
                attempted,
            )?;
        }
        let after = role_sets(request, role)?;
        require(
            after.0 == menus && after.1 == users,
            "GRANT_WRITE_RESULT_UNCONFIRMED",
        )?;
        *attempted = Value::Null;
    }
    for target in &manifest.targets {
        if target.data_grants.is_empty() {
            continue;
        }
        membership(request, target)?;
        let snapshot = request("GET", &query(manifest, target), None)?;
        let version = snapshot_version(&snapshot, &identity(manifest, target))?;
        let delta = grant_delta(manifest, target, &snapshot)?;
        if delta.is_empty() {
            continue;
        }
        let body = json!({"target":identity(manifest,target),"expectedVersion":version,"upsert":delta,"remove":[]});
        let preview = request("POST", "/api/data-grants/preview", Some(&body))?;
        validate_preview(&preview, &snapshot, &body)?;
        *attempted = mark("DATA_GRANTS", &target.membership_id);
        let after = request("PATCH", "/api/data-grants", Some(&body))?;
        snapshot_version(&after, &identity(manifest, target))?;
        validate_saved(&after["grants"], &preview["after"])?;
        completed.push(json!({"kind":"DATA_GRANTS","id":target.membership_id,"target":identity(manifest,target)}));
        *attempted = Value::Null;
    }
    Ok(
        json!({"ok":true,"mode":"APPLIED","applicationId":manifest.application_id,"completedCount":completed.len(),"completed":completed,"warnings":WARNINGS}),
    )
}

fn grant_delta(
    manifest: &Manifest,
    target: &Target,
    snapshot: &Value,
) -> Result<Vec<Grant>, SafeError> {
    snapshot_version(snapshot, &identity(manifest, target))?;
    let before = grant_rows(&snapshot["grants"], false)?;
    let mut delta = Vec::new();
    for grant in &target.data_grants {
        if let Some(existing) = before.get(&(grant.resource.clone(), grant.action.clone())) {
            require(existing["scope"] == grant.scope, "GRANT_SCOPE_CONFLICT")?;
        } else {
            delta.push(grant.clone());
        }
    }
    let mut capabilities = BTreeMap::new();
    for capability in array(&snapshot["capabilities"])? {
        let pair = (
            text(capability, "resource")?.to_owned(),
            text(capability, "action")?.to_owned(),
        );
        let mut scopes = BTreeSet::new();
        for scope in array(&capability["scopes"])? {
            let scope = scope.as_str().ok_or_else(|| SafeError::new(PRECONDITION))?;
            require(
                valid_scope(scope) && scopes.insert(scope.to_owned()),
                PRECONDITION,
            )?;
        }
        require(
            !scopes.is_empty() && capabilities.insert(pair, scopes).is_none(),
            PRECONDITION,
        )?;
    }
    for grant in &delta {
        require(
            capabilities
                .get(&(grant.resource.clone(), grant.action.clone()))
                .is_some_and(|scopes| scopes.contains(&grant.scope)),
            PRECONDITION,
        )?;
    }
    Ok(delta)
}

fn authorization_sets(value: &Value, app: &str) -> Result<(bool, BTreeSet<String>), SafeError> {
    let apps = array(&value["applications"])?;
    let mut seen = BTreeSet::new();
    let mut authorized = false;
    for item in apps {
        let item_id = id(item, "id")?;
        require(seen.insert(item_id.clone()), PRECONDITION)?;
        let selected = item
            .get("authorized")
            .and_then(Value::as_bool)
            .ok_or_else(|| SafeError::new(PRECONDITION))?;
        if item_id == app {
            authorized = selected;
        }
    }
    let menus = value
        .get("menus")
        .and_then(Value::as_object)
        .ok_or_else(|| SafeError::new(PRECONDITION))?;
    let mut result = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for (application, items) in menus {
        require(is_uuid(application), PRECONDITION)?;
        for item in array(items)? {
            let menu_id = id(item, "id")?;
            require(ids.insert(menu_id.clone()), PRECONDITION)?;
            let enabled = item
                .get("authorized")
                .and_then(Value::as_bool)
                .ok_or_else(|| SafeError::new(PRECONDITION))?;
            if application.eq_ignore_ascii_case(app) && enabled {
                result.insert(menu_id);
            }
        }
    }
    Ok((authorized, result))
}

fn snapshot_version<'a>(snapshot: &'a Value, target: &Value) -> Result<&'a str, SafeError> {
    require(snapshot.get("target") == Some(target), PRECONDITION)?;
    let version = text(snapshot, "version")?;
    require(
        version.len() == 64
            && version
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        PRECONDITION,
    )?;
    Ok(version)
}

type GrantRows = BTreeMap<(String, String), Value>;

fn grant_rows(value: &Value, preview: bool) -> Result<GrantRows, SafeError> {
    let rows = array(value)?;
    require(rows.len() <= 128, PRECONDITION)?;
    let mut result = BTreeMap::new();
    let mut ids = BTreeSet::new();
    for row in rows {
        let resource = text(row, "resource")?;
        let action = text(row, "action")?;
        require(
            valid_code(resource)
                && valid_code(action)
                && valid_scope(text(row, "scope")?)
                && valid_scope(text(row, "effectiveScope")?),
            PRECONDITION,
        )?;
        match row.get("id") {
            Some(Value::Null) if preview => {}
            Some(Value::String(value)) => {
                require(
                    is_uuid(value)
                        && value != "00000000-0000-0000-0000-000000000000"
                        && ids.insert(value.to_ascii_lowercase()),
                    PRECONDITION,
                )?;
            }
            _ => return Err(SafeError::new(PRECONDITION)),
        }
        let mut orgs = BTreeSet::new();
        let organization_ids = array(&row["organizationIds"])?;
        require(organization_ids.len() <= 10000, PRECONDITION)?;
        for org in organization_ids {
            let org = org.as_str().ok_or_else(|| SafeError::new(PRECONDITION))?;
            require(
                is_uuid(org) && orgs.insert(org.to_ascii_lowercase()),
                PRECONDITION,
            )?;
        }
        require(
            result
                .insert((resource.to_owned(), action.to_owned()), row.clone())
                .is_none(),
            PRECONDITION,
        )?;
    }
    Ok(result)
}

fn valid_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || index > 0 && (byte.is_ascii_digit() || matches!(byte, b'_' | b'-'))
        })
}

fn validate_preview(preview: &Value, snapshot: &Value, body: &Value) -> Result<(), SafeError> {
    require(
        snapshot_version(preview, &body["target"])?
            == body["expectedVersion"]
                .as_str()
                .ok_or_else(|| SafeError::new(PRECONDITION))?,
        PRECONDITION,
    )?;
    let before = grant_rows(&snapshot["grants"], false)?;
    require(
        grant_rows(&preview["before"], false)? == before,
        PRECONDITION,
    )?;
    let after = grant_rows(&preview["after"], true)?;
    let mut expected_keys: BTreeSet<_> = before.keys().cloned().collect();
    let mut changes = BTreeMap::new();
    for grant in array(&body["upsert"])? {
        let key = (
            text(grant, "resource")?.to_owned(),
            text(grant, "action")?.to_owned(),
        );
        expected_keys.insert(key.clone());
        changes.insert(key, grant);
    }
    require(
        after.keys().cloned().collect::<BTreeSet<_>>() == expected_keys,
        PRECONDITION,
    )?;
    let mut added = BTreeMap::new();
    let mut updated = BTreeMap::new();
    for (key, row) in &after {
        if let Some(grant) = changes.get(key) {
            require(row["scope"] == grant["scope"], PRECONDITION)?;
            if let Some(previous) = before.get(key) {
                require(row["id"] == previous["id"], PRECONDITION)?;
                if row["scope"] != previous["scope"] {
                    updated.insert(key.clone(), row.clone());
                } else {
                    require(row == previous, PRECONDITION)?;
                }
            } else {
                require(row["id"].is_null(), PRECONDITION)?;
                added.insert(key.clone(), row.clone());
            }
        } else {
            require(before.get(key) == Some(row), PRECONDITION)?;
        }
    }
    require(
        grant_rows(&preview["changes"]["added"], true)? == added
            && grant_rows(&preview["changes"]["updated"], false)? == updated
            && array(&preview["changes"]["removed"])?.is_empty(),
        PRECONDITION,
    )
}

fn validate_saved(saved: &Value, preview: &Value) -> Result<(), SafeError> {
    let saved = grant_rows(saved, false)?;
    let preview = grant_rows(preview, true)?;
    require(
        saved.len() == preview.len(),
        "GRANT_WRITE_RESULT_UNCONFIRMED",
    )?;
    for (key, before) in preview {
        let after = saved
            .get(&key)
            .ok_or_else(|| SafeError::new("GRANT_WRITE_RESULT_UNCONFIRMED"))?;
        let mut expected = before.clone();
        if before["id"].is_null() {
            expected["id"] = after["id"].clone();
        }
        require(expected == *after, "GRANT_WRITE_RESULT_UNCONFIRMED")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
