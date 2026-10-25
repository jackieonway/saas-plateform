//! Typed, offline UUID rewriting for catalog-registered row columns only.
//!
//! The caller must first prove exact catalog coverage. JSON row values cannot
//! distinguish a PostgreSQL UUID array/domain from an ordinary JSON business
//! value, so text shape never establishes a column's mapping authority here.
//! Future `uuidArrays` entries use the scalar registration fields plus positive
//! `arrayLayers` and boolean `elementNullable`; domains need an owner handler.

use std::collections::BTreeSet;

use serde_json::{Map, Value};
use uuid::{Uuid, Variant};

use super::mapping::TypedMap;
use crate::{Error, Result};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Entity,
    Reference,
    ReferencePrimaryKey,
    Purpose,
    Discriminated,
}

#[derive(Clone, Copy)]
enum Shape {
    Scalar,
    Array {
        layers: usize,
        element_nullable: bool,
    },
}

struct Column<'a> {
    name: &'a str,
    authority: &'a str,
    role: Role,
    nullable: bool,
    historical: bool,
    shape: Shape,
}

/// Allocate authoritative IDs even for excluded credential rows: historical
/// projections may need their identity while the credentials remain unimported.
/// Any allocation error aborts the batch; discard its partially populated map.
pub(crate) fn allocate_row(
    kind: &str,
    table: &str,
    row: &Value,
    created_at_ms: Option<i64>,
    map: &mut TypedMap,
    registry: &Value,
) -> Result<()> {
    let object = object(row)?;
    let columns = columns(kind, table, object, registry)?;
    let mut allocations = Vec::new();
    // Validate every registered value before adding any authoritative mappings.
    // Forward references are deliberately not resolved during allocation.
    for column in &columns {
        let value = required(object, column.name)?;
        let mut ids = Vec::new();
        visit_value(column, value, &mut |id| {
            ids.push(id);
            Ok(Value::String(id.to_string()))
        })?;
        if matches!(column.role, Role::Entity | Role::Purpose) {
            allocations.extend(ids.into_iter().map(|id| (column.authority, id)));
        }
    }
    for (authority, id) in allocations {
        map.allocate(authority, id, created_at_ms)?;
    }
    Ok(())
}

/// Rewrite the registered UUID columns after the complete source allocation
/// pass. A failed reference leaves this row unchanged.
pub(crate) fn rewrite_row(
    kind: &str,
    table: &str,
    row: &mut Value,
    map: &TypedMap,
    registry: &Value,
) -> Result<()> {
    let source = object(row)?;
    let columns = columns(kind, table, source, registry)?;
    let mut replacements = Vec::with_capacity(columns.len());
    for column in &columns {
        replacements.push((
            column.name.to_owned(),
            resolved_value(column, required(source, column.name)?, map)?,
        ));
    }
    let target = row
        .as_object_mut()
        .ok_or(Error("invalid UUID row object"))?;
    for (name, value) in replacements {
        target.insert(name, value);
    }
    Ok(())
}

/// Verify typed identity closure and exact preservation of every other field.
/// Call before any separately registered structured-value transformation.
pub(crate) fn verify_rewrite(
    kind: &str,
    table: &str,
    source: &Value,
    target: &Value,
    map: &TypedMap,
    registry: &Value,
) -> Result<()> {
    let source = object(source)?;
    let target = object(target)?;
    let columns = columns(kind, table, source, registry)?;
    if source.len() != target.len() || source.keys().any(|name| !target.contains_key(name)) {
        return Err(Error("UUID row column set changed"));
    }
    for (name, original) in source {
        let actual = required(target, name)?;
        let valid = match columns.iter().find(|column| column.name == name) {
            Some(column) => resolved_value(column, original, map)? == *actual,
            None => original == actual,
        };
        if !valid {
            return Err(Error("UUID row rewrite verification failed"));
        }
    }
    Ok(())
}

fn object(row: &Value) -> Result<&Map<String, Value>> {
    row.as_object().ok_or(Error("invalid UUID row object"))
}

fn required<'a>(object: &'a Map<String, Value>, name: &str) -> Result<&'a Value> {
    object
        .get(name)
        .ok_or(Error("missing registered row column"))
}

fn string<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value[name]
        .as_str()
        .ok_or(Error("invalid UUID row registry"))
}

fn array<'a>(value: &'a Value, name: &str) -> Result<&'a [Value]> {
    value[name]
        .as_array()
        .map(Vec::as_slice)
        .ok_or(Error("invalid UUID row registry"))
}

fn table_name(table: &str) -> Result<(&str, &str)> {
    let (schema, table) = table.split_once('.').unwrap_or(("public", table));
    if !identifier(schema) || !identifier(table) {
        return Err(Error("invalid registered row table"));
    }
    Ok((schema, table))
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        && !value.as_bytes()[0].is_ascii_digit()
}

fn same_table(entry: &Value, schema: &str, table: &str) -> Result<bool> {
    Ok(string(entry, "schema")? == schema && string(entry, "table")? == table)
}

fn columns<'a>(
    kind: &str,
    table: &str,
    row: &Map<String, Value>,
    registry: &'a Value,
) -> Result<Vec<Column<'a>>> {
    let (schema, table) = table_name(table)?;
    let mut databases = array(registry, "databases")?
        .iter()
        .filter(|database| database["kind"].as_str() == Some(kind));
    let database = databases.next().ok_or(Error("unknown UUID row database"))?;
    if databases.next().is_some() {
        return Err(Error("duplicate UUID row database"));
    }
    let mut retention = None;
    for entry in array(database, "tables")? {
        if same_table(entry, schema, table)?
            && retention.replace(string(entry, "retention")?).is_some()
        {
            return Err(Error("duplicate UUID row table"));
        }
    }
    let retention = retention.ok_or(Error("unknown UUID row table"))?;
    if !matches!(
        retention,
        "RETAIN_REMAP"
            | "CREDENTIAL_EXCLUDED"
            | "HISTORY_TYPED_PROJECTION"
            | "TASK_RECONCILE"
            | "RETAIN_IDEMPOTENCY"
            | "REBUILD_METADATA"
    ) {
        return Err(Error("invalid UUID row retention"));
    }
    for domain in array(database, "uuidDomains")? {
        if same_table(domain, schema, table)? {
            return Err(Error("UUID row domains require an explicit owner handler"));
        }
    }
    let mut result = Vec::new();
    let mut names = BTreeSet::new();
    for (entries, is_array) in [("columns", false), ("uuidArrays", true)] {
        for entry in array(database, entries)? {
            if !same_table(entry, schema, table)? {
                continue;
            }
            let name = string(entry, "column")?;
            if !identifier(name) || !names.insert(name) {
                return Err(Error("invalid or duplicate UUID row column"));
            }
            let role = match string(entry, "role")? {
                "entity_primary_key" => Role::Entity,
                "reference" => Role::Reference,
                "reference_primary_key" => Role::ReferencePrimaryKey,
                "purpose_scoped_identifier" => Role::Purpose,
                "discriminated_reference" => Role::Discriminated,
                _ => return Err(Error("unknown UUID row column role")),
            };
            let mut authority = string(entry, "authority")?;
            if role == Role::Discriminated {
                let cases = &entry["authorityCases"];
                let selector = string(cases, "selectorColumn")?;
                let value = required(row, selector)?
                    .as_str()
                    .ok_or(Error("invalid UUID reference discriminator"))?;
                let choices = cases["cases"]
                    .as_object()
                    .ok_or(Error("invalid UUID row registry"))?;
                authority = choices
                    .get(value)
                    .and_then(Value::as_str)
                    .ok_or(Error("unknown UUID reference discriminator"))?;
            }
            if role == Role::ReferencePrimaryKey && !authority.starts_with("platform.") {
                return Err(Error("reference primary key requires platform authority"));
            }
            let nullable = entry["nullable"]
                .as_bool()
                .ok_or(Error("invalid UUID row registry"))?;
            let column_retention = string(entry, "retention")?;
            if column_retention != retention {
                return Err(Error("UUID row retention policy mismatch"));
            }
            let historical = retention == "HISTORY_TYPED_PROJECTION";
            let shape = if is_array {
                let layers = entry["arrayLayers"]
                    .as_u64()
                    .filter(|layers| (1..=16).contains(layers))
                    .ok_or(Error("invalid UUID array registration"))?
                    as usize;
                let element_nullable = entry["elementNullable"]
                    .as_bool()
                    .ok_or(Error("invalid UUID array registration"))?;
                Shape::Array {
                    layers,
                    element_nullable,
                }
            } else {
                Shape::Scalar
            };
            let value = required(row, name)?;
            if role == Role::Purpose && !value.is_null() {
                for scope in array(entry, "mappingScopeColumns")? {
                    let name = scope.as_str().ok_or(Error("invalid UUID mapping scope"))?;
                    let scope_value = required(row, name)?;
                    if !matches!(
                        scope_value,
                        Value::String(_) | Value::Number(_) | Value::Bool(_)
                    ) {
                        return Err(Error("invalid UUID mapping scope"));
                    }
                }
            }
            result.push(Column {
                name,
                authority,
                role,
                nullable,
                historical,
                shape,
            });
        }
    }
    Ok(result)
}

fn source_uuid(value: &Value) -> Result<Uuid> {
    let text = value.as_str().ok_or(Error("invalid UUID row value type"))?;
    let id = Uuid::parse_str(text).map_err(|_| Error("invalid UUID row identifier"))?;
    if text.len() != 36
        || id.to_string() != text
        || id.is_nil()
        || id.get_variant() != Variant::RFC4122
        || !(1..=8).contains(&id.get_version_num())
    {
        return Err(Error("invalid UUID row identifier"));
    }
    Ok(id)
}

fn visit_value(
    column: &Column<'_>,
    value: &Value,
    transform: &mut impl FnMut(Uuid) -> Result<Value>,
) -> Result<Value> {
    if value.is_null() {
        return if column.nullable {
            Ok(Value::Null)
        } else {
            Err(Error("null UUID row identifier is forbidden"))
        };
    }
    match column.shape {
        Shape::Scalar => transform(source_uuid(value)?),
        Shape::Array {
            layers,
            element_nullable,
        } => visit_array(value, layers, element_nullable, transform),
    }
}

fn visit_array(
    value: &Value,
    layers: usize,
    element_nullable: bool,
    transform: &mut impl FnMut(Uuid) -> Result<Value>,
) -> Result<Value> {
    if layers == 0 {
        return if value.is_null() && element_nullable {
            Ok(Value::Null)
        } else {
            transform(source_uuid(value)?)
        };
    }
    let values = value.as_array().ok_or(Error("invalid UUID array shape"))?;
    values
        .iter()
        .map(|value| visit_array(value, layers - 1, element_nullable, transform))
        .collect::<Result<Vec<_>>>()
        .map(Value::Array)
}

fn resolved_value(column: &Column<'_>, value: &Value, map: &TypedMap) -> Result<Value> {
    visit_value(column, value, &mut |old| {
        let mapped = if column.historical && column.role != Role::ReferencePrimaryKey {
            map.resolve_historical(column.authority, old)?
        } else {
            map.resolve(column.authority, old)?
        };
        Ok(Value::String(mapped.to_string()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const FROZEN: u64 = 1_792_800_000_123;

    fn old(payload: u128) -> Uuid {
        Uuid::from_u128(0x0011_2233_4455_4000_8000_0000_0000_0000 | payload)
    }

    fn registry() -> Value {
        serde_json::from_str(include_str!("../../uuid-references.json")).unwrap()
    }

    fn mapping() -> TypedMap {
        TypedMap::new(
            FROZEN,
            [
                "platform.user",
                "platform.organization",
                "platform.tenant",
                "platform.platform_session",
                "platform.oauth_token",
                "platform.oauth_jti",
                "stores.store",
                "stores.staff_legacy",
                "stores.daily_report",
                "stores.audit_event",
                "telemetry.request",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        )
        .unwrap()
    }

    fn column(table: &str, name: &str, authority: &str, role: &str, nullable: bool) -> Value {
        json!({"schema":"public","table":table,"column":name,"authority":authority,
            "role":role,"nullable":nullable,"retention":"RETAIN_REMAP"})
    }

    fn custom_registry(columns: Vec<Value>) -> Value {
        json!({"databases":[{"kind":"fixture","tables":[{
            "schema":"public","table":"fixture_rows","retention":"RETAIN_REMAP"}],
            "columns":columns,"uuidArrays":[],"uuidDomains":[]}]})
    }

    #[test]
    fn same_old_value_has_distinct_entities_and_platform_references_cross_databases() {
        let registry = registry();
        let mut map = mapping();
        let id = old(1);
        for (table, row) in [
            ("users", json!({"id":id})),
            (
                "organizations",
                json!({"id":id,"tenant_id":old(2),"parent_id":null}),
            ),
            ("tenants", json!({"id":old(2)})),
        ] {
            allocate_row("platform", table, &row, Some(1000), &mut map, &registry).unwrap();
        }
        let source = json!({"id":id,"tenant_id":old(2),"organization_id":id,
            "owner_user_id":id,"manager_user_id":id,"note":id.to_string()});
        allocate_row("stores", "stores", &source, None, &mut map, &registry).unwrap();
        let mut target = source.clone();
        rewrite_row("stores", "public.stores", &mut target, &map, &registry).unwrap();
        assert_ne!(target["id"], target["owner_user_id"]);
        assert_ne!(target["id"], target["organization_id"]);
        assert_ne!(target["owner_user_id"], target["organization_id"]);
        assert_eq!(
            target["owner_user_id"],
            json!(map.resolve("platform.user", id).unwrap())
        );
        assert_eq!(
            target["organization_id"],
            json!(map.resolve("platform.organization", id).unwrap())
        );
        assert_eq!(target["note"], source["note"]);
        verify_rewrite("stores", "stores", &source, &target, &map, &registry).unwrap();
        let mut wrong_binding = target.clone();
        wrong_binding["owner_user_id"] = target["organization_id"].clone();
        assert!(
            verify_rewrite("stores", "stores", &source, &wrong_binding, &map, &registry).is_err()
        );
        let counter = json!({"tenant_id":old(2),"next_number":9223372036854775807_i64});
        let before = map.len();
        allocate_row(
            "work_orders",
            "work_order_counters",
            &counter,
            None,
            &mut map,
            &registry,
        )
        .unwrap();
        assert_eq!(map.len(), before);
        let mut rewritten = counter.clone();
        rewrite_row(
            "work_orders",
            "work_order_counters",
            &mut rewritten,
            &map,
            &registry,
        )
        .unwrap();
        assert_eq!(
            rewritten["tenant_id"],
            json!(map.resolve("platform.tenant", old(2)).unwrap())
        );
        assert_eq!(rewritten["next_number"], counter["next_number"]);
    }

    #[test]
    fn references_never_allocate_and_tombstones_only_satisfy_explicit_history() {
        let registry = registry();
        let mut map = mapping();
        let source = json!({"user_id":old(1)});
        allocate_row(
            "platform",
            "notification_prefs",
            &source,
            None,
            &mut map,
            &registry,
        )
        .unwrap();
        assert!(map.is_empty());
        let mut target = source.clone();
        assert!(rewrite_row(
            "platform",
            "notification_prefs",
            &mut target,
            &map,
            &registry
        )
        .is_err());
        let operational = custom_registry(vec![column(
            "fixture_rows",
            "user_id",
            "platform.user",
            "reference",
            false,
        )]);
        assert!(rewrite_row("fixture", "fixture_rows", &mut target, &map, &operational).is_err());
        assert_eq!(source, target);
        map.attest_tombstone(
            "platform.user",
            old(1),
            None,
            &"a".repeat(64),
            "deleted_source_entity",
        )
        .unwrap();
        assert!(rewrite_row("fixture", "fixture_rows", &mut target, &map, &operational).is_err());
        assert!(rewrite_row(
            "platform",
            "notification_prefs",
            &mut target,
            &map,
            &registry
        )
        .is_err());
        let history = json!({"id":9223372036854775807_i64,"user_id":old(1),"tenant_id":null,
            "organization_id":null,"application_id":null,"request_id":old(9)});
        allocate_row(
            "platform",
            "audit_logs",
            &history,
            None,
            &mut map,
            &registry,
        )
        .unwrap();
        let mut projected = history.clone();
        rewrite_row("platform", "audit_logs", &mut projected, &map, &registry).unwrap();
        assert_eq!(
            projected["user_id"],
            json!(map.resolve_historical("platform.user", old(1)).unwrap())
        );
        assert_eq!(projected["id"], history["id"]);
        verify_rewrite(
            "platform",
            "audit_logs",
            &history,
            &projected,
            &map,
            &registry,
        )
        .unwrap();
    }

    #[test]
    fn excluded_credential_primary_keys_are_allocated_without_creating_references() {
        let registry = registry();
        let mut map = mapping();
        let row = json!({"id":old(1),"user_id":null,"tenant_id":null,"organization_id":null,
            "jti":old(2),"application_id":null,"platform_session_id":null});
        allocate_row(
            "platform",
            "oauth_tokens",
            &row,
            Some(1000),
            &mut map,
            &registry,
        )
        .unwrap();
        assert!(map.resolve("platform.oauth_token", old(1)).is_ok());
        assert!(map.resolve("platform.oauth_jti", old(2)).is_ok());
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn discriminated_history_uses_registered_authority_and_unknown_cases_fail() {
        let registry = registry();
        let mut map = mapping();
        for authority in [
            "platform.tenant",
            "platform.organization",
            "platform.user",
            "stores.store",
        ] {
            map.allocate(authority, old(1), None).unwrap();
        }
        let source = json!({"id":old(3),"tenant_id":old(1),"organization_id":old(1),
            "actor_user_id":old(1),"entity_id":old(1),"resource_code":"store","request_id":old(4)});
        allocate_row(
            "stores",
            "store_audit_events",
            &source,
            None,
            &mut map,
            &registry,
        )
        .unwrap();
        let mut target = source.clone();
        rewrite_row("stores", "store_audit_events", &mut target, &map, &registry).unwrap();
        assert_eq!(
            target["entity_id"],
            json!(map.resolve("stores.store", old(1)).unwrap())
        );
        assert_ne!(target["entity_id"], target["actor_user_id"]);
        for selector in [json!("unknown-kind"), Value::Null, json!(17)] {
            let mut unknown = source.clone();
            unknown["resource_code"] = selector;
            assert!(rewrite_row(
                "stores",
                "store_audit_events",
                &mut unknown,
                &map,
                &registry
            )
            .is_err());
        }
        let mut missing = source;
        missing.as_object_mut().unwrap().remove("resource_code");
        assert!(rewrite_row(
            "stores",
            "store_audit_events",
            &mut missing,
            &map,
            &registry
        )
        .is_err());
    }

    #[test]
    fn registered_multidimensional_arrays_map_only_typed_elements_and_preserve_nulls() {
        let mut registry = custom_registry(vec![]);
        let mut registration = column(
            "fixture_rows",
            "user_ids",
            "platform.user",
            "reference",
            true,
        );
        registration["arrayLayers"] = json!(2);
        registration["elementNullable"] = json!(true);
        registry["databases"][0]["uuidArrays"] = json!([registration]);
        let mut map = mapping();
        for id in [old(1), old(2)] {
            map.allocate("platform.user", id, None).unwrap();
        }
        let source = json!({"user_ids":[[old(1),null],[old(2),old(1)]],"business":[1,2,3]});
        let before = map.len();
        allocate_row(
            "fixture",
            "fixture_rows",
            &source,
            None,
            &mut map,
            &registry,
        )
        .unwrap();
        assert_eq!(map.len(), before);
        let mut target = source.clone();
        rewrite_row("fixture", "fixture_rows", &mut target, &map, &registry).unwrap();
        assert_eq!(
            target["user_ids"][0][0],
            json!(map.resolve("platform.user", old(1)).unwrap())
        );
        assert!(target["user_ids"][0][1].is_null());
        assert_eq!(target["business"], source["business"]);
        verify_rewrite("fixture", "fixture_rows", &source, &target, &map, &registry).unwrap();
        for invalid in [
            json!([old(1)]),
            json!([[old(1), 17]]),
            json!([null]),
            json!([[Uuid::nil()]]),
        ] {
            let mut row = json!({"user_ids":invalid});
            let original = row.clone();
            assert!(rewrite_row("fixture", "fixture_rows", &mut row, &map, &registry).is_err());
            assert_eq!(row, original);
        }
        let mut nullable = json!({"user_ids":null});
        rewrite_row("fixture", "fixture_rows", &mut nullable, &map, &registry).unwrap();
        let mut empty = json!({"user_ids":[]});
        rewrite_row("fixture", "fixture_rows", &mut empty, &map, &registry).unwrap();
        registry["databases"][0]["uuidArrays"][0]["elementNullable"] = json!(false);
        let mut forbidden_null = source;
        assert!(rewrite_row(
            "fixture",
            "fixture_rows",
            &mut forbidden_null,
            &map,
            &registry
        )
        .is_err());
    }

    #[test]
    fn exact_business_numbers_and_unregistered_structured_values_are_unchanged() {
        let registry = custom_registry(vec![column(
            "fixture_rows",
            "id",
            "platform.user",
            "entity_primary_key",
            false,
        )]);
        let mut map = mapping();
        let raw = format!(
            r#"{{"id":"{}","counter":9223372036854775807,"amount":12345678901234567890.123456789012345678901,"payload":{{"userId":"{}"}},"notes":"{}"}}"#,
            old(1),
            old(1),
            old(1)
        );
        let source: Value = serde_json::from_str(&raw).unwrap();
        allocate_row(
            "fixture",
            "fixture_rows",
            &source,
            None,
            &mut map,
            &registry,
        )
        .unwrap();
        let mut target = source.clone();
        rewrite_row("fixture", "fixture_rows", &mut target, &map, &registry).unwrap();
        verify_rewrite("fixture", "fixture_rows", &source, &target, &map, &registry).unwrap();
        for name in ["counter", "amount", "payload", "notes"] {
            assert_eq!(source[name].to_string(), target[name].to_string());
        }
        target["amount"] = json!(1);
        assert!(
            verify_rewrite("fixture", "fixture_rows", &source, &target, &map, &registry).is_err()
        );
        target = source.clone();
        target["id"] = json!(map.resolve("platform.user", old(1)).unwrap());
        target["extra"] = json!(true);
        assert!(
            verify_rewrite("fixture", "fixture_rows", &source, &target, &map, &registry).is_err()
        );
    }

    #[test]
    fn malformed_registered_values_unknown_contracts_and_domains_fail_without_value_leaks() {
        let registry = custom_registry(vec![column(
            "fixture_rows",
            "id",
            "platform.user",
            "entity_primary_key",
            false,
        )]);
        let mut map = mapping();
        for invalid in [
            Value::Null,
            json!(1),
            json!(true),
            json!({}),
            json!([old(1)]),
            json!(Uuid::nil()),
            json!("00112233-4455-4000-c000-000000000001"),
            json!("private-invalid-input"),
        ] {
            let row = json!({"id":invalid});
            let error = allocate_row("fixture", "fixture_rows", &row, None, &mut map, &registry)
                .err()
                .unwrap();
            assert!(!error.to_string().contains("private-invalid-input"));
            assert!(!format!("{error:?}").contains(&old(1).to_string()));
        }
        assert!(map.is_empty());
        assert!(allocate_row(
            "fixture",
            "fixture_rows",
            &json!({}),
            None,
            &mut map,
            &registry
        )
        .is_err());
        for (kind, table) in [
            ("unknown", "fixture_rows"),
            ("fixture", "unknown"),
            ("fixture", "private.fixture_rows"),
        ] {
            assert!(allocate_row(
                kind,
                table,
                &json!({"id":old(1)}),
                None,
                &mut map,
                &registry
            )
            .is_err());
        }
        let mut domains = registry.clone();
        domains["databases"][0]["uuidDomains"] =
            json!([{"schema":"public","table":"fixture_rows","column":"id"}]);
        assert!(allocate_row(
            "fixture",
            "fixture_rows",
            &json!({"id":old(1)}),
            None,
            &mut map,
            &domains
        )
        .is_err());
        let mut duplicate = registry.clone();
        let entry = duplicate["databases"][0]["columns"][0].clone();
        duplicate["databases"][0]["columns"]
            .as_array_mut()
            .unwrap()
            .push(entry);
        assert!(allocate_row(
            "fixture",
            "fixture_rows",
            &json!({"id":old(1)}),
            None,
            &mut map,
            &duplicate
        )
        .is_err());
    }

    #[test]
    fn complete_validation_and_missing_references_never_partially_change_rows() {
        let registry = custom_registry(vec![
            column(
                "fixture_rows",
                "id",
                "platform.user",
                "entity_primary_key",
                false,
            ),
            column("fixture_rows", "owner", "platform.user", "reference", false),
        ]);
        let mut map = mapping();
        let malformed = json!({"id":old(1),"owner":"invalid-private-value"});
        assert!(allocate_row(
            "fixture",
            "fixture_rows",
            &malformed,
            None,
            &mut map,
            &registry
        )
        .is_err());
        assert!(map.is_empty());
        let source = json!({"id":old(1),"owner":old(2)});
        allocate_row(
            "fixture",
            "fixture_rows",
            &source,
            None,
            &mut map,
            &registry,
        )
        .unwrap();
        let mut target = source.clone();
        assert!(rewrite_row("fixture", "fixture_rows", &mut target, &map, &registry).is_err());
        assert_eq!(target, source);
        assert_eq!(map.len(), 1);
        assert!(map.resolve("platform.user", old(2)).is_err());
    }

    #[test]
    fn only_explicit_history_allows_tombstones_and_purpose_scopes_do_not_reallocate() {
        let mut map = mapping();
        map.attest_tombstone(
            "platform.user",
            old(1),
            None,
            &"a".repeat(64),
            "deleted_source_entity",
        )
        .unwrap();
        for table_policy in ["RETAIN_REMAP", "HISTORY_TYPED_PROJECTION"] {
            for column_policy in ["RETAIN_REMAP", "HISTORY_TYPED_PROJECTION"] {
                let mut registry = custom_registry(vec![column(
                    "fixture_rows",
                    "user_id",
                    "platform.user",
                    "reference",
                    false,
                )]);
                registry["databases"][0]["tables"][0]["retention"] = json!(table_policy);
                registry["databases"][0]["columns"][0]["retention"] = json!(column_policy);
                let mut row = json!({"user_id":old(1)});
                assert_eq!(
                    rewrite_row("fixture", "fixture_rows", &mut row, &map, &registry).is_ok(),
                    table_policy == "HISTORY_TYPED_PROJECTION"
                        && column_policy == "HISTORY_TYPED_PROJECTION"
                );
            }
        }
        let mut purpose = column(
            "fixture_rows",
            "request_id",
            "telemetry.request",
            "purpose_scoped_identifier",
            false,
        );
        purpose["mappingScopeColumns"] = json!(["attempt"]);
        let registry = custom_registry(vec![purpose]);
        let first = json!({"request_id":old(2),"attempt":1});
        let second = json!({"request_id":old(2),"attempt":2});
        allocate_row("fixture", "fixture_rows", &first, None, &mut map, &registry).unwrap();
        let allocated = map.len();
        allocate_row(
            "fixture",
            "fixture_rows",
            &second,
            None,
            &mut map,
            &registry,
        )
        .unwrap();
        assert_eq!(map.len(), allocated);
        let mut first_target = first.clone();
        let mut second_target = second.clone();
        rewrite_row(
            "fixture",
            "fixture_rows",
            &mut first_target,
            &map,
            &registry,
        )
        .unwrap();
        rewrite_row(
            "fixture",
            "fixture_rows",
            &mut second_target,
            &map,
            &registry,
        )
        .unwrap();
        assert_eq!(first_target["request_id"], second_target["request_id"]);
        assert_eq!(first_target["attempt"], 1);
        assert_eq!(second_target["attempt"], 2);
        for invalid in [
            json!({"request_id":old(2)}),
            json!({"request_id":old(2),"attempt":null}),
        ] {
            assert!(allocate_row(
                "fixture",
                "fixture_rows",
                &invalid,
                None,
                &mut map,
                &registry
            )
            .is_err());
        }
    }
}
