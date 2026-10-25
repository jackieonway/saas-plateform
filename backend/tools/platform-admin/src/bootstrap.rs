use crate::{
    SafeError,
    contract::{KnownApp, is_uuid, rows, uuid_field},
    http::ManagementApi,
    options::Options,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Definition {
    schema_version: String,
    application: Application,
    oauth_client: OAuthClient,
    menus: Vec<Menu>,
    grants: Vec<Value>,
}
#[derive(Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Application {
    code: String,
    name: String,
    icon: String,
    base_path: String,
    entry_type: String,
    entry_url: String,
    runtime_mode: String,
    auto_create_menu: bool,
    description: String,
    sort_order: i64,
    status: String,
    allowed_member_types: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OAuthClient {
    client_name: String,
    client_type: String,
    grant_types: Vec<String>,
    scopes: Vec<String>,
    redirect_uris: Vec<String>,
    access_token_validity: i64,
    refresh_token_validity: i64,
    provisioning: String,
    subject_mode: String,
    allowed_member_types: Vec<String>,
}
#[derive(Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Menu {
    key: String,
    #[serde(deserialize_with = "required_parent")]
    parent_key: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    title: String,
    path: String,
    permission: String,
    sort_order: i64,
    visible: bool,
    link_target: String,
    #[serde(default)]
    link_url: Option<String>,
    #[serde(default)]
    status: Option<String>,
}
fn required_parent<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}
fn load(known: KnownApp) -> Result<Definition, SafeError> {
    serde_json::from_str(known.bootstrap())
        .map_err(|_| SafeError::new("INVALID_BOOTSTRAP_MANIFEST"))
}
pub(crate) fn definition(known: KnownApp) -> Result<(), SafeError> {
    let value = load(known)?;
    validate_definition(&value, known)
}
fn validate_definition(value: &Definition, known: KnownApp) -> Result<(), SafeError> {
    let app = &value.application;
    let client = &value.oauth_client;
    if value.schema_version != "1.0"
        || app.code != known.code()
        || app.runtime_mode != "TRUSTED"
        || app.entry_type != "HTML"
        || app.base_path != known.base_path()
        || app.entry_url != known.entry_url()
        || app.auto_create_menu
        || app.status != "ENABLED"
        || app.allowed_member_types != ["EMPLOYEE"]
        || !value.grants.is_empty()
        || value.menus.len() != known.menus().len()
        || client.client_name != known.client_name()
        || client.client_type != "CONFIDENTIAL"
        || client.grant_types != ["client_credentials"]
        || client.scopes != ["openid"]
        || !client.redirect_uris.is_empty()
        || client.access_token_validity != 300
        || client.refresh_token_validity != 300
        || client.provisioning.is_empty()
        || client.subject_mode != "MACHINE"
        || !client.allowed_member_types.is_empty()
    {
        return Err(SafeError::new("INVALID_BOOTSTRAP_MANIFEST"));
    }
    for (index, menu) in value.menus.iter().enumerate() {
        let expected = &known.menus()[index];
        if menu.key != expected.key
            || menu.parent_key.as_deref() != if index == 0 { None } else { Some("root") }
            || menu.kind != expected.kind
            || menu.path != expected.path
            || menu.permission != expected.permission
            || menu.visible != expected.visible
            || menu.link_target != expected.target()
            || menu.sort_order != index as i64
            || (known == KnownApp::Stores
                && (menu.link_url.as_deref() != Some("")
                    || menu.status.as_deref() != Some("ENABLED")))
        {
            return Err(SafeError::new("INVALID_BOOTSTRAP_MANIFEST"));
        }
    }
    Ok(())
}

fn verify_app(row: &Value, expected: Option<&str>, known: KnownApp) -> Result<(), SafeError> {
    let id = uuid_field(row, "id", "APPLICATION_CONFLICT")?;
    if row["code"] != known.code()
        || expected.is_some_and(|expected| expected != id)
        || row["runtimeMode"] != "TRUSTED"
        || row["entryType"] != "HTML"
        || row["basePath"] != known.base_path()
        || !matches!(row["status"].as_str(), Some("ENABLED" | "DISABLED"))
        || row["allowedMemberTypes"] != json!(["EMPLOYEE"])
        || !row["accessPolicyVersion"]
            .as_i64()
            .is_some_and(|version| version > 0)
    {
        return Err(SafeError::new("APPLICATION_CONFLICT"));
    }
    Ok(())
}
pub(crate) fn find_app(
    api: &ManagementApi,
    expected: Option<&str>,
    known: KnownApp,
) -> Result<Option<Value>, SafeError> {
    let response = api.request("GET", "/api/applications", None)?;
    let owned: Vec<_> = rows(&response)?
        .iter()
        .filter(|row| row["code"] == known.code())
        .collect();
    if owned.len() > 1 {
        return Err(SafeError::new("APPLICATION_CONFLICT"));
    }
    match owned.first() {
        Some(row) => {
            verify_app(row, expected, known)?;
            Ok(Some((*row).clone()))
        }
        None if expected.is_some() => Err(SafeError::new("APPLICATION_NOT_FOUND")),
        None => Ok(None),
    }
}
fn verify_client(row: &Value, app_id: Option<&str>, known: KnownApp) -> Result<(), SafeError> {
    uuid_field(row, "id", "CLIENT_CONFLICT")?;
    if row["clientName"] != known.client_name()
        || row["clientType"] != "CONFIDENTIAL"
        || row["status"] != "ENABLED"
        || row["accessTokenValidity"] != 300
        || row["refreshTokenValidity"] != 300
        || row["subjectMode"] != "MACHINE"
        || row["allowedMemberTypes"] != json!([])
        || !row["accessPolicyVersion"]
            .as_i64()
            .is_some_and(|version| version > 0)
    {
        return Err(SafeError::new("CLIENT_CONFLICT"));
    }
    for (key, desired) in [
        ("grantTypes", json!(["client_credentials"])),
        ("scopes", json!(["openid"])),
        ("redirectUris", json!([])),
    ] {
        if row[key] != desired {
            return Err(SafeError::new("CLIENT_CONFLICT"));
        }
    }
    if !row
        .get("applicationId")
        .is_some_and(|value| value.is_null() || value.as_str().is_some_and(is_uuid))
    {
        return Err(SafeError::new("CLIENT_CONFLICT"));
    }
    if !row["applicationId"].is_null() && row["applicationId"].as_str() != app_id {
        return Err(SafeError::new("CLIENT_OWNERSHIP_CONFLICT"));
    }
    Ok(())
}
fn find_client(
    api: &ManagementApi,
    env: &BTreeMap<String, String>,
    app: Option<&Value>,
    known: KnownApp,
) -> Result<Value, SafeError> {
    let client_id = env
        .get("PLATFORM_CLIENT_ID")
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && !value
                    .chars()
                    .any(|ch| ch.is_whitespace() || ch.is_control())
        })
        .ok_or_else(|| SafeError::new("CLIENT_ID_REQUIRED"))?;
    let response = api.request("GET", "/api/oauth/clients", None)?;
    let response = rows(&response)?;
    let matching: Vec<_> = response
        .iter()
        .filter(|row| row["clientId"] == *client_id)
        .collect();
    let named: Vec<_> = response
        .iter()
        .filter(|row| row["clientName"] == known.client_name())
        .collect();
    if matching.len() != 1 || named.len() != 1 || matching[0]["id"] != named[0]["id"] {
        return Err(SafeError::new("CLIENT_CONFLICT"));
    }
    let row = matching[0];
    let app_id = app.and_then(|app| app["id"].as_str());
    verify_client(row, app_id, known)?;
    if app_id.is_some_and(|id| {
        response
            .iter()
            .any(|other| other["id"] != row["id"] && other["applicationId"] == id)
    }) {
        return Err(SafeError::new("CLIENT_OWNERSHIP_CONFLICT"));
    }
    Ok(row.clone())
}
fn flatten(tree: &Value, app_id: &str) -> Result<Vec<Value>, SafeError> {
    fn visit(
        input: &[Value],
        app_id: &str,
        depth: usize,
        seen: &mut BTreeSet<String>,
        output: &mut Vec<Value>,
    ) -> Result<(), SafeError> {
        if depth > 32 || output.len() + input.len() > 1024 {
            return Err(SafeError::new("INVALID_CONTRACT"));
        }
        for row in input {
            let id = uuid_field(row, "id", "MENU_OWNERSHIP_CONFLICT")?;
            if row["applicationId"] != app_id
                || !seen.insert(id.to_owned())
                || !row
                    .get("parentId")
                    .is_some_and(|id| id.is_null() || id.as_str().is_some_and(is_uuid))
            {
                return Err(SafeError::new("MENU_OWNERSHIP_CONFLICT"));
            }
            output.push(row.clone());
            if let Some(children) = row.get("children") {
                visit(rows(children)?, app_id, depth + 1, seen, output)?;
            }
        }
        Ok(())
    }
    let input = tree
        .get("menus")
        .ok_or_else(|| SafeError::new("INVALID_CONTRACT"))?;
    let mut output = Vec::new();
    visit(rows(input)?, app_id, 0, &mut BTreeSet::new(), &mut output)?;
    Ok(output)
}
fn existing<'a>(
    rows: &'a [Value],
    desired: &Menu,
    root: Option<&str>,
    known: KnownApp,
) -> Result<Option<&'a Value>, SafeError> {
    let candidates: Vec<_> = rows
        .iter()
        .filter(|row| {
            if desired.key == "root" {
                row["path"] == desired.path
            } else if desired.kind == "MENU" && row["path"] == desired.path {
                true
            } else if row["permission"] != desired.permission {
                false
            } else {
                // Two reviewed report MENU paths share read. Exclude only that
                // known sibling; a wrong path with this permission is a conflict.
                !known.menus().iter().any(|other| {
                    other.key != desired.key
                        && other.permission == desired.permission
                        && row["path"] == other.path
                })
            }
        })
        .collect();
    if candidates.len() > 1 {
        return Err(SafeError::new("MENU_CONFLICT"));
    }
    if let Some(row) = candidates.first() {
        let parent = if desired.parent_key.is_some() {
            root
        } else {
            None
        };
        if row["type"] != desired.kind
            || row["parentId"] != json!(parent)
            || row["path"] != desired.path
            || row["permission"] != desired.permission
            || row["visible"] != desired.visible
            || row["linkTarget"] != desired.link_target
            || row["linkUrl"] != ""
            || (desired.status.is_some() && row["sortOrder"] != desired.sort_order)
        {
            return Err(SafeError::new("MENU_CONFLICT"));
        }
        Ok(Some(row))
    } else {
        Ok(None)
    }
}
struct Inspection {
    app: Option<Value>,
    client: Value,
    rows: Vec<Value>,
    ids: BTreeMap<String, String>,
}
fn inspect_internal(
    api: &ManagementApi,
    env: &BTreeMap<String, String>,
    options: &Options,
) -> Result<Inspection, SafeError> {
    let app = find_app(api, options.app_id.as_deref(), options.app())?;
    // Validate the explicitly selected existing client before creating anything.
    let client = find_client(api, env, app.as_ref(), options.app())?;
    let menus = if let Some(app) = &app {
        flatten(
            &api.request(
                "GET",
                &format!(
                    "/api/menus/tree?applicationId={}",
                    uuid_field(app, "id", "APPLICATION_CONFLICT")?
                ),
                None,
            )?,
            uuid_field(app, "id", "APPLICATION_CONFLICT")?,
        )?
    } else {
        Vec::new()
    };
    let mut ids = BTreeMap::new();
    let desired = load(options.app())?;
    for menu in &desired.menus {
        if let Some(row) = existing(
            &menus,
            menu,
            ids.get("root").map(String::as_str),
            options.app(),
        )? {
            ids.insert(
                menu.key.clone(),
                uuid_field(row, "id", "MENU_CONFLICT")?.to_owned(),
            );
        }
    }
    Ok(Inspection {
        app,
        client,
        rows: menus,
        ids,
    })
}
pub(crate) fn inspect(
    api: &ManagementApi,
    env: &BTreeMap<String, String>,
    options: &Options,
) -> Result<Value, SafeError> {
    let state = inspect_internal(api, env, options)?;
    Ok(
        json!({"ok":true,"mode":"READ_ONLY","applicationId":state.app.as_ref().map(|app|app["id"].clone()),"clientRecordId":state.client["id"],"clientAssociated":state.app.as_ref().is_some_and(|app|state.client["applicationId"]==app["id"]),"applicationStatus":state.app.as_ref().map(|app|app["status"].clone()),"menuIds":state.ids,"missingMenus":load(options.app())?.menus.iter().filter(|menu|!state.ids.contains_key(&menu.key)).map(|menu|menu.key.clone()).collect::<Vec<_>>(),"grants":[]}),
    )
}
pub(crate) fn apply(
    api: &ManagementApi,
    env: &BTreeMap<String, String>,
    options: &Options,
) -> Result<Value, SafeError> {
    let mut state = inspect_internal(api, env, options)?;
    let desired = load(options.app())?;
    let mut created = Vec::<String>::new();
    let result = (|| {
        if state.app.is_none() {
            let body = serde_json::to_value(&desired.application)
                .map_err(|_| SafeError::new("INVALID_BOOTSTRAP_MANIFEST"))?;
            let app = api.request("POST", "/api/applications", Some(&body))?;
            verify_app(&app, None, options.app()).map_err(SafeError::uncertain)?;
            state.app = Some(app);
            created.push("application".into());
        }
        let app = state
            .app
            .as_ref()
            .ok_or_else(|| SafeError::new("APPLICATION_NOT_FOUND"))?;
        let app_id = uuid_field(app, "id", "APPLICATION_CONFLICT")?;
        if state.client["applicationId"].is_null() {
            state.client = find_client(api, env, Some(app), options.app())?;
        }
        if state.client["applicationId"].is_null() {
            let client = api.request(
                "PUT",
                &format!(
                    "/api/oauth/clients/{}",
                    uuid_field(&state.client, "id", "CLIENT_CONFLICT")?
                ),
                Some(&json!({"applicationId":app_id})),
            )?;
            verify_client(&client, Some(app_id), options.app()).map_err(SafeError::uncertain)?;
            if client["applicationId"] != app_id
                || client["clientId"].as_str() != env.get("PLATFORM_CLIENT_ID").map(String::as_str)
            {
                return Err(SafeError::new("CLIENT_OWNERSHIP_CONFLICT").uncertain());
            }
            state.client = client;
            created.push("clientAssociation".into());
        }
        for menu in &desired.menus {
            let root = state.ids.get("root").map(String::as_str);
            let id = if let Some(row) = existing(&state.rows, menu, root, options.app())? {
                uuid_field(row, "id", "MENU_CONFLICT")?.to_owned()
            } else {
                let mut body = serde_json::to_value(menu)
                    .map_err(|_| SafeError::new("INVALID_BOOTSTRAP_MANIFEST"))?;
                if let Some(body) = body.as_object_mut() {
                    body.remove("key");
                    body.remove("parentKey");
                    // Current menu API has no status field; availability is checked by the grant/auth chain.
                    body.remove("status");
                }
                body["applicationId"] = json!(app_id);
                body["parentId"] = json!(if menu.parent_key.is_some() {
                    root
                } else {
                    None
                });
                body["linkUrl"] = json!("");
                body["icon"] = json!("");
                body["component"] = json!("");
                let row = api.request("POST", "/api/menus", Some(&body))?;
                let id = uuid_field(&row, "id", "MENU_OWNERSHIP_CONFLICT")
                    .map_err(SafeError::uncertain)?
                    .to_owned();
                if row["applicationId"] != app_id {
                    return Err(SafeError::new("MENU_OWNERSHIP_CONFLICT").uncertain());
                }
                if state.rows.iter().any(|row| row["id"] == id) {
                    return Err(SafeError::new("MENU_CONFLICT").uncertain());
                }
                state.rows.push(row);
                if existing(&state.rows, menu, root, options.app())
                    .map_err(SafeError::uncertain)?
                    .is_none()
                {
                    return Err(SafeError::new("MENU_CONFLICT").uncertain());
                }
                created.push(menu.key.clone());
                id
            };
            state.ids.insert(menu.key.clone(), id);
        }
        Ok(
            json!({"ok":true,"mode":"APPLIED","command":"bootstrap","applicationId":app_id,"clientRecordId":state.client["id"],"menuIds":state.ids,"created":created,"grants":[],"releaseActivated":false}),
        )
    })();
    result.map_err(|error|if created.is_empty(){error}else{error.uncertain().with_completed(json!({"created":created,"applicationId":state.app.as_ref().map(|app|app["id"].clone()),"menuIds":state.ids}))})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid(n: u32) -> String {
        format!("00000000-0000-7000-8000-{n:012x}")
    }

    #[test]
    fn embedded_apps_are_exact_and_cross_app_definitions_fail_closed() {
        for app in [KnownApp::WorkOrders, KnownApp::Stores] {
            definition(app).unwrap();
        }
        let mut stores = load(KnownApp::Stores).unwrap();
        assert!(validate_definition(&stores, KnownApp::WorkOrders).is_err());
        let report_detail = stores
            .menus
            .iter()
            .position(|menu| menu.key == "store-daily-report:detail")
            .unwrap();
        stores.menus[report_detail].permission = "store-daily-report:detail".into();
        assert!(validate_definition(&stores, KnownApp::Stores).is_err());
        let mut stores = load(KnownApp::Stores).unwrap();
        stores.menus[report_detail].sort_order -= 1;
        assert!(validate_definition(&stores, KnownApp::Stores).is_err());
        let mut stores = load(KnownApp::Stores).unwrap();
        stores.oauth_client.client_name = "work-orders-client".into();
        assert!(validate_definition(&stores, KnownApp::Stores).is_err());
    }

    #[test]
    fn qiankun_bootstrap_and_existing_apps_reject_legacy_runtime_modes() {
        for known in [KnownApp::WorkOrders, KnownApp::Stores] {
            for runtime in ["IFRAME", "SANDBOX"] {
                let mut definition = load(known).unwrap();
                definition.application.runtime_mode = runtime.into();
                assert!(validate_definition(&definition, known).is_err());
            }
            let mut definition = load(known).unwrap();
            definition.application.entry_type = "ESM".into();
            assert!(validate_definition(&definition, known).is_err());

            // Existing registrations may use an administrator-configured URL.
            // Inspect and repeat bootstrap must validate the runtime without
            // replacing that URL with the local development default.
            let mut app = json!({"id":uuid(1),"code":known.code(),"runtimeMode":"TRUSTED","entryType":"HTML",
                "entryUrl":"https://apps.example.com/index.html","basePath":known.base_path(),"status":"ENABLED","allowedMemberTypes":["EMPLOYEE"],"accessPolicyVersion":1});
            verify_app(&app, None, known).unwrap();
            for runtime in ["IFRAME", "SANDBOX"] {
                app["runtimeMode"] = json!(runtime);
                assert!(verify_app(&app, None, known).is_err());
            }
            app["runtimeMode"] = json!("TRUSTED");
            app["entryType"] = json!("ESM");
            assert!(verify_app(&app, None, known).is_err());
        }
    }

    #[test]
    fn stores_report_menus_share_permission_but_match_independent_paths() {
        let definition = load(KnownApp::Stores).unwrap();
        let report_detail = definition
            .menus
            .iter()
            .position(|menu| menu.key == "store-daily-report:detail")
            .unwrap();
        let root = uuid(1);
        let rows = definition.menus.iter().enumerate().map(|(index, menu)| {
            json!({"id":uuid(index as u32 + 2),"applicationId":uuid(40),"parentId":if index == 0 { None } else { Some(&root) },
                "type":menu.kind,"path":menu.path,"permission":menu.permission,"visible":menu.visible,
                "linkTarget":menu.link_target,"linkUrl":"","sortOrder":menu.sort_order})
        }).collect::<Vec<_>>();
        for menu in &definition.menus {
            assert!(
                existing(&rows, menu, Some(&root), KnownApp::Stores)
                    .unwrap()
                    .is_some()
            );
            assert!(
                existing(&rows, menu, Some(&root), KnownApp::Stores)
                    .unwrap()
                    .is_some()
            );
        }
        let mut bad = rows.clone();
        bad[report_detail]["path"] = json!("/micro/work-orders/tickets/:id");
        bad[report_detail]["applicationId"] = json!(uuid(41));
        assert!(flatten(&json!({"menus":bad}), &uuid(40)).is_err());
        let mut bad = rows.clone();
        bad[report_detail]["sortOrder"] = json!(definition.menus[report_detail].sort_order - 1);
        assert!(
            existing(
                &bad,
                &definition.menus[report_detail],
                Some(&root),
                KnownApp::Stores
            )
            .is_err()
        );
        let mut bad = rows.clone();
        bad[report_detail]["path"] = json!("/micro/work-orders/tickets/:id");
        assert!(
            existing(
                &bad,
                &definition.menus[report_detail],
                Some(&root),
                KnownApp::Stores
            )
            .is_err()
        );
        let work_orders = load(KnownApp::WorkOrders).unwrap();
        let bad_work = json!({"type":"MENU","parentId":root,"path":"/micro/work-orders/wrong","permission":"ticket:read","visible":false,"linkTarget":"MICRO","linkUrl":""});
        assert!(
            existing(
                &[bad_work],
                &work_orders.menus[1],
                Some(&root),
                KnownApp::WorkOrders
            )
            .is_err()
        );
    }

    #[test]
    fn only_exact_precreated_client_and_application_can_be_bound() {
        let id = uuid(1);
        let mut client = json!({"id":uuid(2),"clientName":"stores-client","clientType":"CONFIDENTIAL","status":"ENABLED",
            "accessTokenValidity":300,"refreshTokenValidity":300,"grantTypes":["client_credentials"],"scopes":["openid"],"redirectUris":[],"applicationId":null,"clientSecret":"NEVER_PROJECTED","subjectMode":"MACHINE","allowedMemberTypes":[],"accessPolicyVersion":1});
        verify_client(&client, Some(&id), KnownApp::Stores).unwrap();
        assert!(verify_client(&client, Some(&id), KnownApp::WorkOrders).is_err());
        client["applicationId"] = json!(uuid(3));
        assert_eq!(
            verify_client(&client, Some(&id), KnownApp::Stores)
                .unwrap_err()
                .category(),
            "CLIENT_OWNERSHIP_CONFLICT"
        );
        client["applicationId"] = Value::Null;
        client["clientType"] = json!("PUBLIC");
        assert!(verify_client(&client, Some(&id), KnownApp::Stores).is_err());
        let app = json!({"id":id,"code":"work-orders","runtimeMode":"TRUSTED","entryType":"HTML","basePath":"/micro/work-orders","status":"ENABLED","allowedMemberTypes":["EMPLOYEE"],"accessPolicyVersion":1});
        assert!(verify_app(&app, None, KnownApp::Stores).is_err());
    }

    #[test]
    fn bootstrap_refuses_customer_apps_or_user_clients_without_rewriting_their_policy() {
        for known in [KnownApp::WorkOrders, KnownApp::Stores] {
            let mut definition = load(known).unwrap();
            definition
                .application
                .allowed_member_types
                .push("CUSTOMER".into());
            assert!(validate_definition(&definition, known).is_err());
            let mut definition = load(known).unwrap();
            definition.oauth_client.subject_mode = "USER".into();
            definition.oauth_client.allowed_member_types = vec!["EMPLOYEE".into()];
            assert!(validate_definition(&definition, known).is_err());
            let mut app = json!({"id":uuid(1),"code":known.code(),"runtimeMode":"TRUSTED","entryType":"HTML","basePath":known.base_path(),"status":"ENABLED","allowedMemberTypes":["EMPLOYEE","CUSTOMER"],"accessPolicyVersion":2});
            assert!(verify_app(&app, None, known).is_err());
            app["allowedMemberTypes"] = json!(["EMPLOYEE"]);
            verify_app(&app, None, known).unwrap();
            app["accessPolicyVersion"] = json!(0);
            assert!(verify_app(&app, None, known).is_err());
        }
    }
}
