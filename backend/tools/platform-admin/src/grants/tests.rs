use super::*;

fn uuid(number: u32) -> String {
    format!("00000000-0000-4000-8000-{number:012x}")
}

fn manifest_value() -> Value {
    manifest_value_for(KnownApp::WorkOrders)
}

fn manifest_value_for(app: KnownApp) -> Value {
    let menus = app
        .menus()
        .iter()
        .enumerate()
        .map(|(index, menu)| (menu.key.to_string(), json!(uuid(index as u32 + 10))))
        .collect::<serde_json::Map<_, _>>();
    let permission = app.permissions()[0];
    let (resource, action) = permission.split_once(':').unwrap();
    json!({"schemaVersion":"1.0","applicationCode":app,"applicationId":uuid(1),"menuIds":menus,
        "targets":[{"tenantId":uuid(2),"organizationId":uuid(3),"roleId":uuid(4),"userId":uuid(5),"membershipId":uuid(6),
            "permissions":[permission, app.permissions()[1]],"dataGrants":[{"resource":resource,"action":action,"scope":"ORG"}]}]})
}

fn manifest() -> Manifest {
    validate(manifest_value()).unwrap()
}

#[test]
fn closed_apps_reject_cross_app_permissions_pairs_and_menu_keys() {
    for app in [KnownApp::WorkOrders, KnownApp::Stores] {
        let other = if app == KnownApp::Stores {
            KnownApp::WorkOrders
        } else {
            KnownApp::Stores
        };
        let original = manifest_value_for(app);
        let mut bad = original.clone();
        bad["targets"][0]["permissions"] = json!([other.permissions()[0]]);
        assert_eq!(validate(bad).err().unwrap().category(), INVALID);
        let mut bad = original.clone();
        let (resource, action) = other.permissions()[0].split_once(':').unwrap();
        bad["targets"][0]["dataGrants"] =
            json!([{"resource":resource,"action":action,"scope":"ORG"}]);
        assert_eq!(validate(bad).err().unwrap().category(), INVALID);
        let mut bad = original;
        bad["menuIds"][other.permissions()[0]] = json!(uuid(500));
        assert_eq!(validate(bad).err().unwrap().category(), INVALID);
    }
    let mut bad = manifest_value_for(KnownApp::Stores);
    bad["applicationCode"] = json!("unknown-app");
    assert_eq!(validate(bad).err().unwrap().category(), INVALID);
    for (resource, action) in [
        ("store", "delete"),
        ("store-staff", "update"),
        ("store-daily-report", "summary"),
        ("store-daily-report", "delete"),
    ] {
        let mut bad = manifest_value_for(KnownApp::Stores);
        bad["targets"][0]["dataGrants"] =
            json!([{"resource":resource,"action":action,"scope":"ORG"}]);
        assert_eq!(validate(bad).err().unwrap().category(), INVALID);
    }
}

#[test]
fn stores_read_permission_selects_both_report_menus_without_inventing_an_action() {
    let mut value = manifest_value_for(KnownApp::Stores);
    value["targets"][0]["permissions"] = json!(["store-daily-report:read"]);
    let manifest = validate(value).unwrap();
    let selected = selected_menus(&manifest, &manifest.targets[0]);
    assert_eq!(
        selected,
        BTreeSet::from([
            manifest.menu_ids["root"].clone(),
            manifest.menu_ids["store-daily-report:read"].clone(),
            manifest.menu_ids["store-daily-report:detail"].clone(),
        ])
    );
    assert_eq!(manifest.application_code.permissions().len(), 9);
}

#[test]
fn stores_menu_validation_rejects_cross_app_paths_permissions_and_order() {
    let manifest = validate(manifest_value_for(KnownApp::Stores)).unwrap();
    let mut fake = Fake::new(manifest.clone());
    let response = fake
        .call("GET", "/api/menus/tree?applicationId=fixture", None)
        .unwrap();
    check_menus(&manifest, &response["menus"]).unwrap();
    for (field, value) in [
        ("applicationId", json!(uuid(500))),
        ("path", json!("/micro/work-orders/tickets/:id")),
        ("permission", json!("ticket:read")),
        ("sortOrder", json!(99)),
    ] {
        let mut bad = response.clone();
        bad["menus"][8][field] = value;
        assert_eq!(
            check_menus(&manifest, &bad["menus"])
                .unwrap_err()
                .category(),
            PRECONDITION
        );
    }
}

#[test]
fn stores_grants_keep_unlisted_historical_resources_and_are_repeatable() {
    let manifest = validate(manifest_value_for(KnownApp::Stores)).unwrap();
    let mut fake = Fake::new(manifest);
    let historical = row("ticket", "read", "ORG", json!(uuid(95)));
    fake.grants.insert(uuid(6), vec![historical.clone()]);
    fake.run().unwrap();
    assert_eq!(fake.grants[&uuid(6)][0], historical);
    assert_eq!(fake.patch_count, 1);
    let patch = fake
        .calls
        .iter()
        .find(|call| call.0 == "PATCH")
        .unwrap()
        .2
        .as_ref()
        .unwrap();
    assert_eq!(patch["remove"], json!([]));
    assert_eq!(patch["upsert"][0]["resource"], "store");
    assert_eq!(patch["expectedVersion"], "a".repeat(64));
    let old_writes = fake.writes().len();
    fake.run().unwrap();
    assert_eq!(fake.writes().len(), old_writes);
    assert_eq!(fake.patch_count, 1);
    assert!(fake.role_menus.contains(&uuid(90)) && fake.role_users.contains(&uuid(91)));
}

fn row(resource: &str, action: &str, scope: &str, id: Value) -> Value {
    json!({"id":id,"resource":resource,"action":action,"scope":scope,"effectiveScope":scope,"organizationIds":[uuid(3)]})
}

struct Fake {
    manifest: Manifest,
    calls: Vec<(String, String, Option<Value>)>,
    tenant_menus: BTreeSet<String>,
    org_menus: BTreeSet<String>,
    role_menus: BTreeSet<String>,
    role_users: BTreeSet<String>,
    grants: BTreeMap<String, Vec<Value>>,
    bad_membership: bool,
    bad_menu: bool,
    bad_preview: bool,
    bad_role_after_write: bool,
    unknown_patch: bool,
    fail_patch_number: usize,
    patch_count: usize,
    role_write_count: usize,
}

impl Fake {
    fn new(manifest: Manifest) -> Self {
        Self {
            manifest,
            calls: Vec::new(),
            tenant_menus: BTreeSet::new(),
            org_menus: BTreeSet::new(),
            role_menus: BTreeSet::from([uuid(90)]),
            role_users: BTreeSet::from([uuid(91)]),
            grants: BTreeMap::new(),
            bad_membership: false,
            bad_menu: false,
            bad_preview: false,
            bad_role_after_write: false,
            unknown_patch: false,
            fail_patch_number: 0,
            patch_count: 0,
            role_write_count: 0,
        }
    }

    fn page(records: Vec<Value>) -> Value {
        json!({"total":records.len(),"records":records,"page":1,"size":200})
    }

    fn auth(&self, menus: &BTreeSet<String>) -> Value {
        json!({"applications":[{"id":self.manifest.application_id,"authorized":!menus.is_empty()}],
            "menus":{self.manifest.application_id.clone():self.manifest.menu_ids.values().map(|menu|json!({"id":menu,"authorized":menus.contains(menu)})).collect::<Vec<_>>()}})
    }

    fn snapshot(&self, target: &Value) -> Value {
        let membership = target["membershipId"].as_str().unwrap();
        json!({"target":target,"version":"a".repeat(64),"grants":self.grants.get(membership).cloned().unwrap_or_default(),
            "capabilities":self.manifest.targets[0].data_grants.iter().map(|grant| json!({"resource":grant.resource,"action":grant.action,"scopes":["SELF","ORG"]})).collect::<Vec<_>>()})
    }

    fn call(&mut self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, SafeError> {
        self.calls
            .push((method.to_owned(), path.to_owned(), body.cloned()));
        let first = &self.manifest.targets[0];
        if path.starts_with("/api/applications/") {
            return Ok(
                json!({"id":self.manifest.application_id,"code":self.manifest.application_code,"status":"ENABLED",
                "basePath":self.manifest.application_code.base_path(),"runtimeMode":"TRUSTED","entryType":"HTML"}),
            );
        }
        if path.starts_with("/api/menus/tree?") {
            let menus = self.manifest.application_code.menus().iter().enumerate().map(|(index,menu)| {
                json!({"id":self.manifest.menu_ids[menu.key],"applicationId":if self.bad_menu {uuid(999)} else {self.manifest.application_id.clone()},
                    "parentId":if menu.key=="root" {Value::Null} else {json!(self.manifest.menu_ids["root"])},
                    "permission":menu.permission,"type":menu.kind,"path":menu.path,"visible":menu.visible,"linkTarget":menu.target(),"linkUrl":"","sortOrder":index})
            }).collect::<Vec<_>>();
            return Ok(json!({"menus":menus}));
        }
        if path.starts_with("/api/tenants?") {
            return Ok(Self::page(vec![
                json!({"id":first.tenant_id,"status":"ENABLED","expiresAt":null}),
            ]));
        }
        if path.starts_with("/api/organizations/tree?") {
            return Ok(
                json!([{"id":first.organization_id,"tenantId":first.tenant_id,"parentId":null,"status":"ENABLED","expiresAt":null}]),
            );
        }
        if path.starts_with("/api/roles?") {
            return Ok(Self::page(vec![
                json!({"id":first.role_id,"tenantId":first.tenant_id,"organizationId":first.organization_id,"status":"ENABLED","builtIn":false,"code":"reviewed"}),
            ]));
        }
        if path.starts_with("/api/users?") {
            return Ok(Self::page(
                self.manifest
                    .targets
                    .iter()
                    .map(|target| json!({"id":target.user_id,"status":"ENABLED"}))
                    .collect(),
            ));
        }
        if path.starts_with("/api/users/") && path.ends_with("/memberships") {
            let target = self
                .manifest
                .targets
                .iter()
                .find(|target| path.contains(&target.user_id))
                .unwrap();
            return Ok(
                json!([{"membershipId":target.membership_id,"organizationId":if self.bad_membership {uuid(999)} else {target.organization_id.clone()},"tenantId":target.tenant_id}]),
            );
        }
        if path.ends_with("/authorizations") {
            let tenant = path.starts_with("/api/tenants/");
            if method == "GET" {
                return Ok(self.auth(if tenant {
                    &self.tenant_menus
                } else {
                    &self.org_menus
                }));
            }
            let body = body.unwrap();
            assert!(
                body["applications"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|row| row["authorized"] == true)
            );
            let menus = if tenant {
                &mut self.tenant_menus
            } else {
                &mut self.org_menus
            };
            for item in body["menus"].as_array().unwrap() {
                assert_eq!(item["authorized"], true);
                menus.insert(item["menuId"].as_str().unwrap().to_owned());
            }
            return Ok(json!({"ok":true}));
        }
        if path.starts_with("/api/roles/") && path.ends_with("/menus") {
            if method == "GET" {
                let menus = if self.bad_role_after_write && self.role_write_count > 0 {
                    BTreeSet::new()
                } else {
                    self.role_menus.clone()
                };
                return Ok(json!({"menuIds":menus}));
            }
            self.role_write_count += 1;
            self.role_menus = body.unwrap()["menuIds"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| id.as_str().unwrap().to_owned())
                .collect();
            return Ok(json!({"ok":true}));
        }
        if path.starts_with("/api/roles/") && path.ends_with("/users") {
            if method == "GET" {
                return Ok(json!(
                    self.role_users
                        .iter()
                        .map(|id| json!({"userId":id}))
                        .collect::<Vec<_>>()
                ));
            }
            self.role_users = body.unwrap()["userIds"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| id.as_str().unwrap().to_owned())
                .collect();
            return Ok(json!({"ok":true}));
        }
        if path.starts_with("/api/data-grants?") {
            let target = self
                .manifest
                .targets
                .iter()
                .find(|target| path.contains(&target.membership_id))
                .unwrap();
            return Ok(self.snapshot(&identity(&self.manifest, target)));
        }
        if path == "/api/data-grants/preview" {
            let body = body.unwrap();
            let snapshot = self.snapshot(&body["target"]);
            let before = snapshot["grants"].as_array().unwrap().clone();
            let mut after = before.clone();
            let mut added = Vec::new();
            for grant in body["upsert"].as_array().unwrap() {
                let new = row(
                    grant["resource"].as_str().unwrap(),
                    grant["action"].as_str().unwrap(),
                    if self.bad_preview {
                        "TENANT"
                    } else {
                        grant["scope"].as_str().unwrap()
                    },
                    Value::Null,
                );
                added.push(new.clone());
                after.push(new);
            }
            return Ok(
                json!({"target":body["target"],"version":body["expectedVersion"],"before":before,"after":after,
                "changes":{"added":added,"updated":[],"removed":[]}}),
            );
        }
        if path == "/api/data-grants" && method == "PATCH" {
            self.patch_count += 1;
            if self.unknown_patch || self.fail_patch_number == self.patch_count {
                return Err(SafeError::http("NETWORK_ERROR", None, true));
            }
            let body = body.unwrap();
            let membership = body["target"]["membershipId"].as_str().unwrap();
            let grants = self.grants.entry(membership.to_owned()).or_default();
            for grant in body["upsert"].as_array().unwrap() {
                grants.push(row(
                    grant["resource"].as_str().unwrap(),
                    grant["action"].as_str().unwrap(),
                    grant["scope"].as_str().unwrap(),
                    json!(uuid(100 + self.patch_count as u32)),
                ));
            }
            return Ok(self.snapshot(&body["target"]));
        }
        panic!("unhandled fixture request {method} {path}");
    }

    fn run(&mut self) -> Result<Value, SafeError> {
        let manifest = self.manifest.clone();
        apply_with(&manifest, &mut |method, path, body| {
            self.call(method, path, body)
        })
    }

    fn writes(&self) -> Vec<&(String, String, Option<Value>)> {
        self.calls.iter().filter(|call| call.0 != "GET").collect()
    }
}

#[test]
fn strict_manifest_rejects_unknown_keys_nil_ids_duplicates_and_conflicting_owners() {
    for value in {
        let original = manifest_value();
        let mut variants = Vec::new();
        let mut value = original.clone();
        value["databaseUrl"] = json!("secret");
        variants.push(value);
        let mut value = original.clone();
        value["targets"][0]["secret"] = json!("secret");
        variants.push(value);
        let mut value = original.clone();
        value["applicationId"] = json!("00000000-0000-0000-0000-000000000000");
        variants.push(value);
        let mut value = original.clone();
        value["menuIds"]["ticket:create"] = value["menuIds"]["ticket:read"].clone();
        variants.push(value);
        let mut value = original.clone();
        value["targets"][0]["permissions"] = json!(["ticket:read", "ticket:read"]);
        variants.push(value);
        let mut value = original.clone();
        value["targets"][0]["dataGrants"] = json!([{"resource":"ticket","action":"read","scope":"ORG"},{"resource":"ticket","action":"read","scope":"TENANT"}]);
        variants.push(value);
        let mut value = original.clone();
        let duplicate = value["targets"][0].clone();
        value["targets"].as_array_mut().unwrap().push(duplicate);
        variants.push(value);
        let mut value = original;
        let mut duplicate = value["targets"][0].clone();
        duplicate["membershipId"] = json!(uuid(80));
        duplicate["userId"] = json!(uuid(81));
        duplicate["tenantId"] = json!(uuid(82));
        value["targets"].as_array_mut().unwrap().push(duplicate);
        variants.push(value);
        variants
    } {
        assert_eq!(validate(value).err().unwrap().category(), INVALID);
    }
}

#[test]
fn empty_targets_are_an_offline_no_op_even_in_apply() {
    let mut manifest = manifest();
    manifest.targets.clear();
    let output = apply_with(&manifest, &mut |_, _, _| {
        panic!("empty intent must not make HTTP requests")
    })
    .unwrap();
    assert_eq!(output["completedCount"], 0);
}

#[test]
fn preflight_rejects_cross_application_menu_or_wrong_direct_membership_without_writes() {
    for corrupt_menu in [true, false] {
        let mut fake = Fake::new(manifest());
        fake.bad_menu = corrupt_menu;
        fake.bad_membership = !corrupt_menu;
        assert_eq!(fake.run().unwrap_err().category(), PRECONDITION);
        assert!(fake.writes().is_empty());
    }
}

#[test]
fn preflight_rejects_legacy_runtime_before_any_grant_write() {
    for app in [KnownApp::WorkOrders, KnownApp::Stores] {
        let manifest = validate(manifest_value_for(app)).unwrap();
        for (runtime, entry) in [("IFRAME", "HTML"), ("SANDBOX", "HTML"), ("TRUSTED", "ESM")] {
            let mut requests = Vec::new();
            let error = apply_with(&manifest, &mut |method, path, _| {
                requests.push((method.to_owned(), path.to_owned()));
                assert_eq!(method, "GET");
                assert_eq!(
                    path,
                    format!("/api/applications/{}", manifest.application_id)
                );
                Ok(
                    json!({"id":manifest.application_id,"code":app.code(),"status":"ENABLED",
                    "basePath":app.base_path(),"runtimeMode":runtime,"entryType":entry}),
                )
            })
            .unwrap_err();
            assert_eq!(error.category(), PRECONDITION);
            assert_eq!(requests.len(), 1);
        }
    }
}

#[test]
fn applies_positive_deltas_preserves_role_audience_and_history_and_is_idempotent() {
    let mut fake = Fake::new(manifest());
    let historical = row("historical", "retired", "SELF", json!(uuid(95)));
    fake.grants.insert(uuid(6), vec![historical.clone()]);
    let output = fake.run().unwrap();
    assert_eq!(output["completedCount"], 5);
    assert!(fake.role_menus.contains(&uuid(90)));
    assert!(fake.role_users.contains(&uuid(91)));
    assert_eq!(fake.grants[&uuid(6)][0], historical);
    let preview = fake
        .calls
        .iter()
        .find(|call| call.0 == "POST")
        .unwrap()
        .2
        .as_ref()
        .unwrap();
    let patch = fake
        .calls
        .iter()
        .find(|call| call.0 == "PATCH")
        .unwrap()
        .2
        .as_ref()
        .unwrap();
    assert_eq!(preview, patch);
    assert_eq!(patch["expectedVersion"], "a".repeat(64));
    assert_eq!(patch["remove"], json!([]));
    let before = fake.writes().len();
    assert_eq!(fake.run().unwrap()["completedCount"], 0);
    assert_eq!(fake.writes().len(), before);
    assert!(
        !fake
            .calls
            .iter()
            .any(|call| call.0 != "GET" && call.1.ends_with("/memberships"))
    );
}

#[test]
fn scope_conflict_is_refused_without_preview_or_patch() {
    let mut fake = Fake::new(manifest());
    fake.grants.insert(
        uuid(6),
        vec![row("ticket", "read", "SELF", json!(uuid(95)))],
    );
    let error = fake.run().unwrap_err();
    assert_eq!(error.category(), "GRANT_SCOPE_CONFLICT");
    assert_eq!(fake.patch_count, 0);
    assert!(
        !fake
            .calls
            .iter()
            .any(|call| call.1 == "/api/data-grants/preview")
    );
    assert_eq!(error.to_json()["progress"]["completedCount"], 4);
}

#[test]
fn already_authorized_targets_reject_known_grant_conflicts_before_any_write() {
    let manifest = manifest();
    let menus = selected_menus(&manifest, &manifest.targets[0]);
    let mut fake = Fake::new(manifest);
    fake.tenant_menus = menus.clone();
    fake.org_menus = menus;
    fake.grants.insert(
        uuid(6),
        vec![row("ticket", "read", "SELF", json!(uuid(95)))],
    );
    let error = fake.run().unwrap_err();
    assert_eq!(error.category(), "GRANT_SCOPE_CONFLICT");
    assert!(fake.writes().is_empty());
    assert_eq!(error.to_json()["progress"]["completedCount"], 0);
    assert!(!error.outcome_unknown());
}

#[test]
fn grant_delta_requires_live_capability_and_preserves_existing_same_scope() {
    let manifest = manifest();
    let target = &manifest.targets[0];
    let mut snapshot = json!({"target":identity(&manifest,target),"version":"b".repeat(64),"grants":[],
        "capabilities":[{"resource":"ticket","action":"read","scopes":["SELF"]}]});
    assert_eq!(
        grant_delta(&manifest, target, &snapshot)
            .unwrap_err()
            .category(),
        PRECONDITION
    );
    snapshot["capabilities"][0]["scopes"] = json!(["ORG"]);
    assert_eq!(grant_delta(&manifest, target, &snapshot).unwrap().len(), 1);
    snapshot["grants"] = json!([row("ticket", "read", "ORG", json!(uuid(95)))]);
    snapshot["capabilities"] = json!([]);
    assert!(
        grant_delta(&manifest, target, &snapshot)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn owner_status_expiry_and_paginated_directory_integrity_fail_closed() {
    let now = DateTime::parse_from_rfc3339("2026-10-17T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    active(&json!({"status":"ENABLED","expiresAt":null}), now).unwrap();
    for owner in [
        json!({"status":"DISABLED","expiresAt":null}),
        json!({"status":"ENABLED","expiresAt":"2026-10-17T00:00:00Z"}),
        json!({"status":"ENABLED","expiresAt":"invalid"}),
        json!({"status":"ENABLED"}),
    ] {
        assert_eq!(active(&owner, now).unwrap_err().category(), PRECONDITION);
    }
    let duplicate =
        json!({"records":[{"id":uuid(2)},{"id":uuid(2)}],"page":1,"size":200,"total":2});
    assert_eq!(
        paged(&mut |_, _, _| Ok(duplicate.clone()), "/api/tenants")
            .unwrap_err()
            .category(),
        PRECONDITION
    );
    let oversized = json!({"records":[],"page":1,"size":200,"total":10001});
    assert_eq!(
        paged(&mut |_, _, _| Ok(oversized.clone()), "/api/tenants")
            .unwrap_err()
            .category(),
        PRECONDITION
    );
}

#[test]
fn mismatched_preview_never_reaches_patch() {
    let mut fake = Fake::new(manifest());
    fake.bad_preview = true;
    assert_eq!(fake.run().unwrap_err().category(), PRECONDITION);
    assert_eq!(fake.patch_count, 0);
}

#[test]
fn unknown_write_and_failed_readback_require_reconciliation_and_never_retry() {
    for patch_failure in [true, false] {
        let mut fake = Fake::new(manifest());
        fake.unknown_patch = patch_failure;
        fake.bad_role_after_write = !patch_failure;
        let error = fake.run().unwrap_err();
        assert!(error.outcome_unknown());
        assert_eq!(
            error.to_json()["nextAction"],
            "INSPECT_AND_RECONCILE_BEFORE_RETRY"
        );
        assert_eq!(fake.patch_count, usize::from(patch_failure));
        assert!(!fake.calls.iter().any(|call| call.0 == "DELETE"));
        let output = error.to_json().to_string();
        assert!(
            !output.contains("Bearer") && !output.contains("/api/") && !output.contains("username")
        );
    }
}

#[test]
fn multiple_targets_stop_after_partial_success_and_report_completed_membership_ids() {
    let mut manifest = manifest();
    let mut target = manifest.targets[0].clone();
    target.user_id = uuid(7);
    target.membership_id = uuid(8);
    manifest.targets.push(target);
    let mut fake = Fake::new(manifest);
    fake.fail_patch_number = 2;
    let error = fake.run().unwrap_err();
    assert_eq!(fake.patch_count, 2);
    let output = error.to_json();
    assert_eq!(output["progress"]["completedCount"], 5);
    assert_eq!(output["progress"]["completed"][4]["kind"], "DATA_GRANTS");
    assert_eq!(output["progress"]["completed"][4]["id"], uuid(6));
    assert_eq!(output["progress"]["attempted"]["id"], uuid(8));
    assert!(error.outcome_unknown());
}

#[test]
fn preview_rejects_hidden_removals_or_touched_untargeted_rows() {
    let target = json!({"tenantId":uuid(2),"organizationId":uuid(3),"userId":uuid(5),"membershipId":uuid(6),"applicationId":uuid(1)});
    let historical = row("ticket", "export", "ORG", json!(uuid(95)));
    let snapshot = json!({"target":target,"version":"a".repeat(64),"grants":[historical.clone()]});
    let body = json!({"target":target,"expectedVersion":"a".repeat(64),"upsert":[{"resource":"ticket","action":"read","scope":"ORG"}],"remove":[]});
    let added = row("ticket", "read", "ORG", Value::Null);
    let preview = json!({"target":target,"version":"a".repeat(64),"before":[historical.clone()],"after":[historical,added.clone()],"changes":{"added":[added],"updated":[],"removed":[]}});
    validate_preview(&preview, &snapshot, &body).unwrap();
    let mut removed = preview.clone();
    removed["after"].as_array_mut().unwrap().remove(0);
    assert_eq!(
        validate_preview(&removed, &snapshot, &body)
            .unwrap_err()
            .category(),
        PRECONDITION
    );
    let mut changed = preview;
    changed["after"][0]["scope"] = json!("TENANT");
    assert_eq!(
        validate_preview(&changed, &snapshot, &body)
            .unwrap_err()
            .category(),
        PRECONDITION
    );
}
