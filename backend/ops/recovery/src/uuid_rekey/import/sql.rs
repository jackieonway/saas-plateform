//! Pure plans for importing authenticated rows into an independently migrated
//! target. No connections, files, source writes, migration repair or COMMIT.
use super::super::{export, transform};
use crate::{Error, Result};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub(super) struct TablePlan {
    pub(super) schema: String,
    pub(super) table: String,
    pub(super) columns: Vec<String>,
    pub(super) primary_key: Vec<String>,
    pub(super) rows: u64,
    pub(super) policy: String,
    /// The old row envelope cannot distinguish SQL NULL from top-level JSON
    /// null in a nullable JSON column. The caller must reject that ambiguity.
    pub(super) nullable_json_columns: Vec<String>,
    json_columns: Vec<String>,
}

#[cfg(all(test, target_os = "linux"))]
pub(super) fn hierarchy_fixture_plan(catalog: &Value, table: &str, rows: u64) -> TablePlan {
    assert!(matches!(table, "organizations" | "departments"));
    let columns = column_map(catalog, &("public".into(), table.into())).unwrap();
    assert!(columns.iter().all(|column| column["type"] != "jsonb"));
    TablePlan {
        schema: "public".into(),
        table: table.into(),
        columns: columns
            .iter()
            .map(|column| text(column, "column").unwrap().into())
            .collect(),
        primary_key: vec!["id".into()],
        rows,
        policy: "RETAIN_REMAP".into(),
        nullable_json_columns: Vec::new(),
        json_columns: Vec::new(),
    }
}

type Identity = (String, String);

fn known_kind(kind: &str) -> Result<()> {
    if matches!(kind, "platform" | "stores" | "work_orders") {
        Ok(())
    } else {
        Err(Error("unknown import database kind"))
    }
}

fn array<'a>(value: &'a Value, key: &str) -> Result<&'a [Value]> {
    value[key]
        .as_array()
        .map(Vec::as_slice)
        .ok_or(Error("incomplete import catalog array"))
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .ok_or(Error("incomplete import catalog identity"))
}

fn ident(value: &str) -> Result<String> {
    if value.is_empty()
        || value.len() > 63
        || !value
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_lowercase() || i > 0 && b.is_ascii_digit())
    {
        return Err(Error("unsupported import SQL identifier"));
    }
    Ok(format!("\"{value}\""))
}

fn identity(value: &Value) -> Result<Identity> {
    let schema = text(value, "schema")?;
    let table = text(value, "table")?;
    if schema != "public" {
        return Err(Error("only registered public import tables are supported"));
    }
    ident(schema)?;
    ident(table)?;
    Ok((schema.into(), table.into()))
}

fn relation(plan: &TablePlan) -> Result<String> {
    if plan.schema != "public" {
        return Err(Error("only public import relations are supported"));
    }
    Ok(format!("{}.{}", ident(&plan.schema)?, ident(&plan.table)?))
}

fn relations(catalog: &Value) -> Result<BTreeMap<Identity, u64>> {
    let mut result = BTreeMap::new();
    for item in array(catalog, "relations")? {
        let key = identity(item)?;
        let rows = item["row_count"]
            .as_u64()
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or(Error("invalid import row count"))?;
        if item["kind"] != "r"
            || item["is_partition"] != false
            || result.insert(key, rows).is_some()
        {
            return Err(Error("ordinary nonpartitioned import tables required"));
        }
    }
    Ok(result)
}

fn column_map(catalog: &Value, key: &Identity) -> Result<Vec<Value>> {
    let mut positions = BTreeMap::new();
    let mut names = BTreeSet::new();
    for item in array(catalog, "schema_material")?
        .iter()
        .filter(|v| v["kind"] == "column" && v["schema"] == key.0 && v["table"] == key.1)
    {
        let name = text(item, "column")?;
        ident(name)?;
        let position = item["position"]
            .as_u64()
            .filter(|n| *n > 0 && *n <= i16::MAX as u64)
            .ok_or(Error("invalid import column position"))?;
        if item["identity"] != ""
            || item["generated"] != ""
            || item["dimensions"] != 0
            || item["local"] != true
            || item["inherited"] != 0
            || text(item, "type")? == "json"
            || !item["not_null"].is_boolean()
            || !names.insert(name)
            || positions.insert(position, item.clone()).is_some()
        {
            return Err(Error("unsupported or duplicate import column"));
        }
    }
    if positions.is_empty() {
        return Err(Error("import table has no complete column catalog"));
    }
    Ok(positions.into_values().collect())
}

fn strings(value: &Value) -> Result<Vec<String>> {
    let mut seen = BTreeSet::new();
    value
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or(Error("import constraint columns missing"))?
        .iter()
        .map(|v| {
            let name = v
                .as_str()
                .ok_or(Error("invalid import constraint column"))?;
            ident(name)?;
            if !seen.insert(name) {
                return Err(Error("duplicate import constraint column"));
            }
            Ok(name.into())
        })
        .collect()
}

fn constraints(catalog: &Value) -> Result<BTreeMap<(Identity, String), Value>> {
    let mut result = BTreeMap::new();
    for item in array(catalog, "constraints")? {
        let key = identity(item)?;
        let name = text(item, "name")?.to_owned();
        ident(&name)?;
        if !item["validated"].is_boolean()
            // PostgreSQL 18 records NOT NULL as a real catalog constraint.
            // Keep it in the exact source/target comparison just like other
            // constraints; PostgreSQL 17 simply supplies no such rows.
            || !matches!(text(item, "type")?, "c" | "p" | "u" | "f" | "n")
            || item["type"] != "c" && item["validated"] != true
            || result.insert((key, name), item.clone()).is_some()
        {
            return Err(Error("unvalidated or unsupported import constraint"));
        }
    }
    Ok(result)
}

fn compact_sql(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '"')
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .replace("pg_catalog.", "")
}

fn uuid_guard(kind: &str, table: &str, column: &str, constraint: &Value) -> bool {
    let expected_name = if kind == "platform" {
        format!("uuid_v7_{column}")
    } else {
        format!("{table}_{column}_uuid_v7")
    };
    let Some(definition) = constraint["definition"].as_str() else {
        return false;
    };
    // Preserve parentheses: moving the shift into get_byte's index argument
    // changes the predicate even though its remaining tokens are identical.
    // These are a finite set of owner-SQL / PG17-deparser spellings, not an
    // expression parser or a permissive substring check.
    let expected = if kind == "platform" {
        vec![format!("CHECK (public.is_uuid_v7({column}))")]
    } else {
        let version = format!("((get_byte(uuid_send({column}),6)>>4)=7)");
        let variant = format!("((get_byte(uuid_send({column}),8)&192)=128)");
        let predicate = format!("({version} AND {variant})");
        let owner_predicate = format!(
            "(get_byte(uuid_send({column}),6)>>4)=7 AND (get_byte(uuid_send({column}),8)&192)=128"
        );
        if kind == "work_orders" {
            vec![
                format!("CHECK (({column} IS NULL) OR {predicate})"),
                format!("CHECK ((({column} IS NULL) OR {predicate}))"),
                format!("CHECK ({column} IS NULL OR ({owner_predicate}))"),
            ]
        } else {
            vec![
                format!("CHECK ({predicate})"),
                format!("CHECK ({owner_predicate})"),
            ]
        }
    };
    constraint["name"] == expected_name
        && constraint["type"] == "c"
        && constraint["validated"] == true
        && constraint["columns"] == serde_json::json!([column])
        && expected
            .iter()
            .any(|value| compact_sql(definition) == compact_sql(value))
}

fn validate_uuid_columns(kind: &str, catalog: &Value, registry_db: &Value) -> Result<()> {
    let mut expected = BTreeSet::new();
    for column in array(registry_db, "columns")? {
        let (schema, table) = identity(column)?;
        let name = text(column, "column")?.to_owned();
        ident(&name)?;
        if !expected.insert((schema, table, name)) {
            return Err(Error("duplicate registered import UUID column"));
        }
    }
    let guards = constraints(catalog)?;
    let mut actual = BTreeSet::new();
    for column in array(catalog, "uuid_columns")? {
        let (schema, table) = identity(column)?;
        let name = text(column, "column")?;
        ident(name)?;
        let key = (schema.clone(), table.clone(), name.to_owned());
        let material: Vec<_> = array(catalog, "schema_material")?
            .iter()
            .filter(|v| {
                v["kind"] == "column"
                    && v["schema"] == schema
                    && v["table"] == table
                    && v["column"] == name
            })
            .collect();
        if column["declared_type"] != "uuid"
            || column["array_layers"] != 0
            || column["domains"] != serde_json::json!([])
            || column.get("default") != Some(&Value::Null)
            || material.len() != 1
            || material[0]["type"] != "uuid"
            || material[0].get("default") != Some(&Value::Null)
            || !actual.insert(key)
            || guards
                .values()
                .filter(|v| v["schema"] == schema && v["table"] == table)
                .filter(|v| uuid_guard(kind, &table, name, v))
                .count()
                != 1
        {
            return Err(Error(
                "target UUIDv7 guard or removed-default contract mismatch",
            ));
        }
    }
    let material_uuids: BTreeSet<_> = array(catalog, "schema_material")?
        .iter()
        .filter(|v| v["kind"] == "column" && v["type"] == "uuid")
        .map(|v| {
            let (schema, table) = identity(v)?;
            Ok((schema, table, text(v, "column")?.to_owned()))
        })
        .collect::<Result<_>>()?;
    if expected != actual || actual != material_uuids {
        return Err(Error("target UUID columns differ from the typed registry"));
    }
    if kind == "platform" {
        let functions: Vec<_> = array(catalog, "functions")?
            .iter()
            .filter(|v| v["schema"] == "public" && v["name"] == "is_uuid_v7")
            .collect();
        if functions.len() != 1 {
            return Err(Error("unique target UUIDv7 predicate required"));
        }
        let function = functions[0];
        let definition = text(function, "definition")?;
        let body: Vec<_> = definition.split("$function$").collect();
        if function["arguments"] != "value uuid"
            || function["security_definer"] != false
            || function["volatility"] != "i"
            || function["parallel"] != "s"
            || function["configuration"] != serde_json::json!(["search_path=pg_catalog"])
            || !definition.split_whitespace().any(|s| s == "STRICT")
            || body.len() != 3
            || compact_sql(body[1]) != compact_sql(
                "SELECT (get_byte(uuid_send(value),6)>>4)=7 AND (get_byte(uuid_send(value),8)&192)=128"
            )
        {
            return Err(Error(
                "target UUIDv7 predicate differs from the owner migration",
            ));
        }
    }
    Ok(())
}

/// Validate complete column/constraint/table coverage and return only retained
/// business tables, in FK dependency order. Self edges stay in one INSERT.
pub(super) fn plans(
    kind: &str,
    target_catalog: &Value,
    source: &export::DatabaseSnapshot,
    converted: &transform::ConvertedDatabase,
    registry: &Value,
) -> Result<Vec<TablePlan>> {
    known_kind(kind)?;
    if source.kind != kind || converted.kind != kind {
        return Err(Error("import database kinds disagree"));
    }
    let registered: Vec<_> = array(registry, "databases")?
        .iter()
        .filter(|v| v["kind"] == kind)
        .collect();
    if registered.len() != 1 {
        return Err(Error("unique import database registry required"));
    }
    let registry_db = registered[0];
    let mut policies = BTreeMap::new();
    for table in array(registry_db, "tables")? {
        let policy = text(table, "retention")?;
        if !matches!(
            policy,
            "RETAIN_REMAP"
                | "HISTORY_TYPED_PROJECTION"
                | "TASK_RECONCILE"
                | "RETAIN_IDEMPOTENCY"
                | "CREDENTIAL_EXCLUDED"
                | "REBUILD_METADATA"
        ) || policies.insert(identity(table)?, policy).is_some()
        {
            return Err(Error("unsupported or duplicate import retention policy"));
        }
    }
    let source_relations = relations(&source.catalog)?;
    let target_relations = relations(target_catalog)?;
    let source_keys: BTreeSet<_> = source_relations.keys().cloned().collect();
    if source_keys != policies.keys().cloned().collect() {
        return Err(Error("source tables differ from import registry"));
    }
    let mut target_keys = source_keys.clone();
    let v3 = ("public".to_owned(), "_stores_schema_contract_v3".to_owned());
    if kind == "stores" && !target_keys.insert(v3.clone()) {
        return Err(Error(
            "source unexpectedly contains target-only schema metadata",
        ));
    }
    if target_keys != target_relations.keys().cloned().collect() {
        return Err(Error(
            "target tables differ from real-migration metadata allowance",
        ));
    }
    let metadata: BTreeSet<_> = policies
        .iter()
        .filter(|(_, policy)| **policy == "REBUILD_METADATA")
        .map(|(key, _)| key.1.as_str())
        .collect();
    let expected_metadata = match kind {
        "platform" => BTreeSet::from(["_sqlx_migrations"]),
        "stores" => BTreeSet::from([
            "_sqlx_migrations",
            "_stores_schema_contract",
            "_stores_schema_contract_v2",
        ]),
        "work_orders" => BTreeSet::from(["_sqlx_migrations", "work_order_migrations"]),
        _ => return Err(Error("unknown import metadata kind")),
    };
    if metadata != expected_metadata {
        return Err(Error("unexpected import migration metadata policy"));
    }
    validate_uuid_columns(kind, target_catalog, registry_db)?;
    let source_constraints = constraints(&source.catalog)?;
    let target_constraints = constraints(target_catalog)?;
    for (key, constraint) in &source_constraints {
        if target_constraints.get(key) != Some(constraint) {
            return Err(Error(
                "target ordinary constraints differ from frozen source",
            ));
        }
    }
    for ((key, _), constraint) in &target_constraints {
        if source_constraints.contains_key(&(key.clone(), text(constraint, "name")?.into()))
            || kind == "stores" && *key == v3
        {
            continue;
        }
        let columns = strings(&constraint["columns"])?;
        if columns.len() != 1 || !uuid_guard(kind, &key.1, &columns[0], constraint) {
            return Err(Error("unregistered target constraint added during import"));
        }
    }
    let mut source_tables = BTreeMap::new();
    for table in &source.tables {
        let key = (table.schema.clone(), table.table.clone());
        if source_relations.get(&key) != Some(&table.rows)
            || source_tables.insert(key, table).is_some()
        {
            return Err(Error("source table manifest differs from catalog"));
        }
    }
    let mut converted_tables = BTreeMap::new();
    for table in &converted.tables {
        let key = (table.schema.clone(), table.table.clone());
        if converted_tables.insert(key, table).is_some() {
            return Err(Error("duplicate converted import table"));
        }
    }
    if source_tables.keys().collect::<BTreeSet<_>>() != source_keys.iter().collect()
        || converted_tables.keys().collect::<BTreeSet<_>>() != source_keys.iter().collect()
    {
        return Err(Error(
            "complete source and converted table manifests required",
        ));
    }
    let mut pending = BTreeMap::new();
    for key in &source_keys {
        let policy = policies[key];
        let converted_table = converted_tables[key];
        let source_rows = source_relations[key];
        let excluded = matches!(policy, "REBUILD_METADATA" | "CREDENTIAL_EXCLUDED");
        if converted_table.policy != policy
            || converted_table.source_rows != source_rows
            || converted_table.rows != if excluded { 0 } else { source_rows }
        {
            return Err(Error("converted import counts or policy mismatch"));
        }
        let source_columns = column_map(&source.catalog, key)?;
        let target_columns = column_map(target_catalog, key)?;
        if source_columns.len() != target_columns.len() {
            return Err(Error("target column set differs from frozen source"));
        }
        for (source_column, target_column) in source_columns.iter().zip(&target_columns) {
            let mut normalized = source_column.clone();
            if source_column["type"] == "uuid" {
                normalized["default"] = Value::Null;
            }
            if &normalized != target_column {
                return Err(Error("target ordinary column differs from frozen source"));
            }
        }
        if policy == "REBUILD_METADATA" {
            continue;
        }
        let settings = kind == "platform" && key.1 == "platform_settings";
        if target_relations[key] != u64::from(settings) || settings && converted_table.rows != 1 {
            return Err(Error(
                "target business tables must be fresh migration defaults only",
            ));
        }
        if excluded {
            continue;
        }
        let primary_keys: Vec<_> = target_constraints
            .values()
            .filter(|v| v["schema"] == key.0 && v["table"] == key.1 && v["type"] == "p")
            .collect();
        if primary_keys.len() != 1 || primary_keys[0]["validated"] != true {
            return Err(Error("one actual import primary key required"));
        }
        let columns: Vec<String> = target_columns
            .iter()
            .map(|v| text(v, "column").map(str::to_owned))
            .collect::<Result<_>>()?;
        let primary_key = strings(&primary_keys[0]["columns"])?;
        if primary_key.iter().any(|v| !columns.contains(v)) {
            return Err(Error("import primary key not present in full column set"));
        }
        let json_columns = target_columns
            .iter()
            .filter(|v| v["type"] == "jsonb")
            .map(|v| text(v, "column").map(str::to_owned))
            .collect::<Result<_>>()?;
        let nullable_json_columns = target_columns
            .iter()
            .filter(|v| v["type"] == "jsonb" && v["not_null"] == false)
            .map(|v| text(v, "column").map(str::to_owned))
            .collect::<Result<_>>()?;
        pending.insert(
            key.clone(),
            TablePlan {
                schema: key.0.clone(),
                table: key.1.clone(),
                columns,
                primary_key,
                rows: converted_table.rows,
                policy: policy.into(),
                nullable_json_columns,
                json_columns,
            },
        );
    }
    let mut dependencies: BTreeMap<Identity, BTreeSet<Identity>> = pending
        .keys()
        .map(|key| (key.clone(), BTreeSet::new()))
        .collect();
    for constraint in target_constraints.values().filter(|v| v["type"] == "f") {
        let child = identity(constraint)?;
        let parent = (
            text(constraint, "referenced_schema")?.to_owned(),
            text(constraint, "referenced_table")?.to_owned(),
        );
        if !target_relations.contains_key(&parent)
            || strings(&constraint["columns"])?.len()
                != strings(&constraint["referenced_columns"])?.len()
        {
            return Err(Error("invalid import foreign key graph"));
        }
        if child != parent && pending.contains_key(&parent) {
            if let Some(deps) = dependencies.get_mut(&child) {
                deps.insert(parent);
            }
        }
    }
    let mut output = Vec::new();
    let mut done = BTreeSet::new();
    while !pending.is_empty() {
        let next = pending
            .keys()
            .find(|key| dependencies[*key].is_subset(&done))
            .cloned()
            .ok_or(Error("cross-table import foreign key cycle"))?;
        done.insert(next.clone());
        output.push(
            pending
                .remove(&next)
                .ok_or(Error("import dependency plan missing"))?,
        );
    }
    Ok(output)
}

pub(super) fn begin_sql(kind: &str) -> Result<String> {
    known_kind(kind)?;
    Ok("BEGIN ISOLATION LEVEL READ COMMITTED READ WRITE;\n \
        SET LOCAL search_path=pg_catalog;\nSET LOCAL row_security=off;\n \
        SET LOCAL timezone='UTC';\nSET LOCAL DateStyle='ISO, YMD';\n \
        SET LOCAL extra_float_digits=3;\nSET LOCAL bytea_output='hex';\n \
        SET LOCAL standard_conforming_strings=on;\nSET LOCAL lock_timeout='3s';\n \
        SET CONSTRAINTS ALL IMMEDIATE;\n \
        DO $uuid_import$ BEGIN IF current_setting('session_replication_role') <> 'origin' THEN \
        RAISE EXCEPTION 'import requires enabled origin constraint triggers'; END IF; END $uuid_import$;\n".into())
}

/// The caller sends CSV ordinal/body records followed by the psql COPY end line.
pub(super) fn stage_sql(plan: &TablePlan) -> Result<String> {
    let table = relation(plan)?;
    Ok(format!(
        "LOCK TABLE ONLY {table} IN ACCESS EXCLUSIVE MODE;\n \
        CREATE TEMP TABLE uuid_import_stage (ordinal bigint PRIMARY KEY CHECK (ordinal>0), \
        body jsonb NOT NULL CHECK (pg_catalog.jsonb_typeof(body)='object')) ON COMMIT DROP;\n \
        COPY pg_temp.uuid_import_stage(ordinal,body) FROM STDIN WITH (FORMAT csv);\n"
    ))
}

/// One INSERT for the entire table preserves immediate self-FKs. A later
/// statement checks actual target rows, never a data-modifying CTE snapshot.
pub(super) fn insert_verify_sql(plan: &TablePlan) -> Result<String> {
    let table = relation(plan)?;
    if plan.rows > i64::MAX as u64
        || plan.columns.is_empty()
        || plan.primary_key.is_empty()
        || !matches!(
            plan.policy.as_str(),
            "RETAIN_REMAP" | "HISTORY_TYPED_PROJECTION" | "TASK_RECONCILE" | "RETAIN_IDEMPOTENCY"
        )
    {
        return Err(Error("invalid bounded import table plan"));
    }
    let columns = plan
        .columns
        .iter()
        .map(|v| ident(v))
        .collect::<Result<Vec<_>>>()?;
    let keys = plan
        .primary_key
        .iter()
        .map(|v| ident(v))
        .collect::<Result<Vec<_>>>()?;
    if plan.columns.iter().collect::<BTreeSet<_>>().len() != plan.columns.len()
        || plan.primary_key.iter().collect::<BTreeSet<_>>().len() != plan.primary_key.len()
        || plan.primary_key.iter().any(|v| !plan.columns.contains(v))
        || plan.json_columns.iter().any(|v| !plan.columns.contains(v))
        || plan
            .nullable_json_columns
            .iter()
            .any(|v| !plan.json_columns.contains(v))
    {
        return Err(Error("invalid import table column/key plan"));
    }
    let mut sorted_columns = plan.columns.clone();
    sorted_columns.sort();
    let column_array = sorted_columns
        .iter()
        .map(|v| format!("'{v}'"))
        .collect::<Vec<_>>()
        .join(",");
    let expected_values = plan
        .columns
        .iter()
        .zip(&columns)
        .map(|(name, quoted)| {
            if plan.json_columns.contains(name) {
                format!("s.body->'{name}'")
            } else {
                format!("p.{quoted}")
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    let expected_columns = columns
        .iter()
        .map(|v| format!("(e.value).{v}"))
        .collect::<Vec<_>>()
        .join(",");
    let expected_keys = keys
        .iter()
        .map(|v| format!("(e.value).{v}"))
        .collect::<Vec<_>>()
        .join(",");
    let null_keys = keys
        .iter()
        .map(|v| format!("(e.value).{v} IS NULL"))
        .collect::<Vec<_>>()
        .join(" OR ");
    let join = keys
        .iter()
        .map(|v| format!("t.{v}=(e.value).{v}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let mut sql = format!(
        "DO $uuid_import$ BEGIN\n \
        IF (SELECT count(*) FROM pg_temp.uuid_import_stage)<>{rows} \
        OR (SELECT COALESCE(min(ordinal),0) FROM pg_temp.uuid_import_stage)<>{minimum} \
        OR (SELECT COALESCE(max(ordinal),0) FROM pg_temp.uuid_import_stage)<>{rows} THEN \
        RAISE EXCEPTION 'import ordinal coverage mismatch'; END IF;\n \
        IF EXISTS(SELECT 1 FROM pg_temp.uuid_import_stage s WHERE \
         (SELECT COALESCE(array_agg(k ORDER BY k COLLATE \"C\"),ARRAY[]::text[]) \
        FROM pg_catalog.jsonb_object_keys(s.body) AS fields(k)) \
        IS DISTINCT FROM ARRAY[{column_array}]::text[]) THEN \
        RAISE EXCEPTION 'import full column coverage mismatch'; END IF;\n",
        rows = plan.rows,
        minimum = u64::from(plan.rows > 0)
    );
    if !plan.nullable_json_columns.is_empty() {
        let ambiguous = plan
            .nullable_json_columns
            .iter()
            .map(|v| format!("s.body->'{v}'='null'::jsonb"))
            .collect::<Vec<_>>()
            .join(" OR ");
        sql.push_str(&format!(
            "IF EXISTS(SELECT 1 FROM pg_temp.uuid_import_stage s WHERE {ambiguous}) THEN \
            RAISE EXCEPTION 'nullable JSON null requires frozen source evidence'; END IF;\n"
        ));
    }
    sql.push_str("END $uuid_import$;\n");
    sql.push_str(&format!("CREATE TEMP TABLE uuid_import_expected ON COMMIT DROP AS \
        SELECT s.ordinal,ROW({expected_values})::{table} AS value \
        FROM pg_temp.uuid_import_stage s CROSS JOIN LATERAL \
        pg_catalog.jsonb_populate_record(NULL::{table},s.body) AS p;\n \
        ALTER TABLE pg_temp.uuid_import_expected ADD PRIMARY KEY(ordinal);\n \
        DO $uuid_import$ BEGIN\n \
        IF EXISTS(SELECT 1 FROM pg_temp.uuid_import_expected e WHERE {null_keys}) \
        OR EXISTS(SELECT 1 FROM pg_temp.uuid_import_expected e GROUP BY {expected_keys} HAVING count(*)<>1) \
        OR EXISTS(SELECT 1 FROM pg_temp.uuid_import_expected e JOIN pg_temp.uuid_import_stage s USING(ordinal) \
        WHERE pg_catalog.to_jsonb(e.value) IS DISTINCT FROM s.body) THEN \
        RAISE EXCEPTION 'import typed row conversion changed data or key'; END IF;\n"));
    if plan.table == "platform_settings" {
        sql.push_str("IF (SELECT count(*) FROM ONLY public.platform_settings)<>1 OR EXISTS( \
            SELECT 1 FROM ONLY public.platform_settings t WHERE \
            (pg_catalog.to_jsonb(t)-'updated_at') IS DISTINCT FROM \
            '{\"singleton\":true,\"platform_name\":\"SaaS Platform\",\"platform_description\":\"多租户权限管理平台\",\"logo_url\":\"\",\"login_title\":\"欢迎回来\",\"login_description\":\"管理您的租户与应用权限\",\"footer_text\":\"© 2026 SaaS Platform · Powered by Rust & React\",\"primary_color\":\"#008577\"}'::jsonb) THEN \
            RAISE EXCEPTION 'target platform settings are not fresh migration defaults'; END IF;\n");
    } else {
        sql.push_str(&format!(
            "IF EXISTS(SELECT 1 FROM ONLY {table}) THEN \
            RAISE EXCEPTION 'target import table is not empty'; END IF;\n"
        ));
    }
    if plan.table == "store_staff" {
        sql.push_str(
            "IF (SELECT count(*) FROM pg_catalog.pg_trigger WHERE \
            tgrelid='public.store_staff'::regclass AND tgname='store_staff_legacy_read_only' \
            AND NOT tgisinternal AND tgenabled='O')<>1 THEN \
            RAISE EXCEPTION 'legacy staff guard is not enabled'; END IF;\n",
        );
    }
    sql.push_str("END $uuid_import$;\n");
    if plan.table == "platform_settings" {
        sql.push_str("DELETE FROM ONLY public.platform_settings;\n");
    }
    if plan.table == "store_staff" {
        sql.push_str(
            "ALTER TABLE public.store_staff DISABLE TRIGGER store_staff_legacy_read_only;\n",
        );
    }
    // Historical hierarchy guards resolve their own tables through the caller's
    // search_path. Public is fenced to the reviewed owner; keep catalog first
    // and expose public only while the real INSERT triggers execute.
    sql.push_str(&format!(
        "SET LOCAL search_path=pg_catalog,public;\n \
        INSERT INTO {table} ({}) SELECT {expected_columns} \
        FROM pg_temp.uuid_import_expected e ORDER BY e.ordinal;\n \
        SET LOCAL search_path=pg_catalog;\n",
        columns.join(",")
    ));
    if plan.table == "store_staff" {
        sql.push_str(
            "ALTER TABLE public.store_staff ENABLE TRIGGER store_staff_legacy_read_only;\n",
        );
    }
    sql.push_str(&format!("SET CONSTRAINTS ALL IMMEDIATE;\n \
        DO $uuid_import$ BEGIN\n \
        IF (SELECT count(*) FROM ONLY {table})<>{rows} \
        OR (SELECT count(*) FROM pg_temp.uuid_import_expected e JOIN ONLY {table} t ON {join})<>{rows} \
        OR EXISTS(SELECT 1 FROM pg_temp.uuid_import_expected e LEFT JOIN ONLY {table} t ON {join} \
        GROUP BY e.ordinal HAVING count(t.{first_key})<>1) \
        OR EXISTS(SELECT 1 FROM pg_temp.uuid_import_expected e JOIN ONLY {table} t ON {join} \
        WHERE pg_catalog.to_jsonb(t) IS DISTINCT FROM pg_catalog.to_jsonb(e.value)) THEN \
        RAISE EXCEPTION 'import actual row verification failed'; END IF;\n \
        END $uuid_import$;\n \
        SELECT pg_catalog.jsonb_build_object('kind','import_row','schema','{schema}','table','{name}', \
        'ordinal',e.ordinal,'row',pg_catalog.to_jsonb(t)) \
        FROM pg_temp.uuid_import_expected e JOIN ONLY {table} t ON {join} ORDER BY e.ordinal;\n \
        SELECT pg_catalog.jsonb_build_object('kind','table_verified','schema','{schema}', \
        'table','{name}','rows',{rows}::bigint);\n \
        DROP TABLE pg_temp.uuid_import_expected;\nDROP TABLE pg_temp.uuid_import_stage;\n",
        rows=plan.rows, first_key=keys[0], schema=plan.schema, name=plan.table));
    Ok(sql)
}

/// The caller still verifies final catalog, package proofs and sequence state
/// before explicitly committing this transaction.
pub(super) fn finish_sql(kind: &str) -> Result<String> {
    known_kind(kind)?;
    let mut sql = String::from("SET CONSTRAINTS ALL IMMEDIATE;\nDO $uuid_import$ BEGIN\n \
        IF current_setting('session_replication_role')<>'origin' \
        OR EXISTS(SELECT 1 FROM pg_catalog.pg_constraint c JOIN pg_catalog.pg_namespace n \
        ON n.oid=c.connamespace WHERE n.nspname='public' AND NOT c.convalidated \
        AND (c.contype='f' OR (c.contype='c' \
        AND (left(c.conname,8)='uuid_v7_' OR right(c.conname,8)='_uuid_v7')))) \
        OR EXISTS(SELECT 1 FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_class c ON c.oid=t.tgrelid \
        JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND t.tgenabled<>'O') \
        OR EXISTS(SELECT 1 FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class c ON c.oid=i.indrelid \
        JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' \
        AND (NOT i.indisvalid OR NOT i.indisready)) THEN \
        RAISE EXCEPTION 'import final constraint or trigger state mismatch'; END IF;\n");
    if kind == "platform" {
        for name in ["organizations", "departments", "menus"] {
            sql.push_str(&format!("IF (WITH RECURSIVE reachable(id) AS ( \
                SELECT id FROM ONLY public.{name} WHERE parent_id IS NULL UNION ALL \
                SELECT child.id FROM ONLY public.{name} child JOIN reachable parent ON child.parent_id=parent.id) \
                SELECT count(*) FROM reachable)<>(SELECT count(*) FROM ONLY public.{name}) THEN \
                RAISE EXCEPTION 'import hierarchy contains a cycle or missing parent'; END IF;\n"));
        }
    }
    sql.push_str("END $uuid_import$;\n");
    if kind == "stores" {
        // Match the real migrator's deparse context for its historical hash,
        // and restore the importer's fixed path before any later statement.
        sql.push_str("SET LOCAL search_path=public,pg_catalog;\nDO $uuid_import$ BEGIN\n \
            IF NOT(SELECT count(*)=1 AND COALESCE(bool_and( \
            v3.fingerprint=public.stores_schema_fingerprint_v3() \
            AND v3.legacy_fingerprint=v2.fingerprint AND v2.legacy_fingerprint=v1.fingerprint),false) \
            FROM public._stores_schema_contract_v3 v3 JOIN public._stores_schema_contract_v2 v2 USING(singleton) \
            JOIN public._stores_schema_contract v1 USING(singleton) WHERE v3.singleton) THEN \
            RAISE EXCEPTION 'import final stores schema contract mismatch'; END IF;\n \
            END $uuid_import$;\nSET LOCAL search_path=pg_catalog;\n");
    }
    sql.push_str("SELECT pg_catalog.jsonb_build_object('kind','business_verified');\n");
    Ok(sql)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn digest() -> super::super::super::codec::FileDigest {
        super::super::super::codec::FileDigest {
            sha256: "a".repeat(64),
            plaintext_bytes: 0,
            frames: 0,
        }
    }

    fn column(table: &str, name: &str, position: u64, ty: &str, not_null: bool) -> Value {
        json!({"kind":"column","schema":"public","table":table,"position":position,
            "column":name,"type":ty,"not_null":not_null,"identity":"","generated":"",
            "dimensions":0,"storage":"p","local":true,"inherited":0,
            "collation_schema":null,"collation":null,
            "default":if ty=="uuid" { json!("gen_random_uuid()") } else { Value::Null }})
    }

    fn constraint(table: &str, name: &str, ty: &str, columns: &[&str]) -> Value {
        json!({"kind":"constraint","schema":"public","table":table,"name":name,
            "type":ty,"definition":"fixture ordinary constraint","columns":columns,
            "validated":true,"deferrable":false,"deferred":false,"no_inherit":false})
    }

    fn fixture() -> (
        Value,
        export::DatabaseSnapshot,
        transform::ConvertedDatabase,
        Value,
    ) {
        let specs = [
            ("_sqlx_migrations", "REBUILD_METADATA", 1),
            ("work_order_migrations", "REBUILD_METADATA", 1),
            ("nodes", "RETAIN_REMAP", 3),
            ("children", "RETAIN_REMAP", 2),
            ("credentials", "CREDENTIAL_EXCLUDED", 2),
        ];
        let columns = vec![
            column("_sqlx_migrations", "version", 1, "bigint", true),
            column("work_order_migrations", "version", 1, "text", true),
            column("nodes", "name", 1, "character varying(100)", true),
            column("nodes", "id", 2, "uuid", true),
            column("nodes", "parent_id", 3, "uuid", false),
            column("nodes", "receipt", 4, "jsonb", false),
            column("nodes", "facts", 5, "jsonb", true),
            column("children", "id", 1, "bigint", true),
            column("children", "node_id", 2, "uuid", true),
            column("credentials", "id", 1, "uuid", true),
        ];
        let mut constraints = vec![
            constraint(
                "_sqlx_migrations",
                "_sqlx_migrations_pkey",
                "p",
                &["version"],
            ),
            constraint(
                "work_order_migrations",
                "work_order_migrations_pkey",
                "p",
                &["version"],
            ),
            constraint("nodes", "nodes_pkey", "p", &["id"]),
            constraint("children", "children_pkey", "p", &["id"]),
            constraint("credentials", "credentials_pkey", "p", &["id"]),
        ];
        for (table, name, key) in [
            ("nodes", "nodes_parent_id_fkey", "parent_id"),
            ("children", "children_node_id_fkey", "node_id"),
        ] {
            let mut fk = constraint(table, name, "f", &[key]);
            fk["referenced_schema"] = json!("public");
            fk["referenced_table"] = json!("nodes");
            fk["referenced_columns"] = json!(["id"]);
            constraints.push(fk);
        }
        let mut historical = constraint("credentials", "historical_not_valid", "c", &["id"]);
        historical["validated"] = json!(false);
        constraints.push(historical);
        let source_catalog = json!({
            "relations":specs.iter().map(|(table,_,rows)| json!({"schema":"public","table":table,
                "kind":"r","is_partition":false,"row_count":rows})).collect::<Vec<_>>(),
            "schema_material":columns,"constraints":constraints,"functions":[],"uuid_columns":[]
        });
        let mut target = source_catalog.clone();
        for relation in target["relations"].as_array_mut().unwrap() {
            if !relation["table"].as_str().unwrap().ends_with("migrations") {
                relation["row_count"] = json!(0);
            }
        }
        let mut registered_columns = Vec::new();
        for item in target["schema_material"].as_array_mut().unwrap() {
            if item["type"] == "uuid" {
                item["default"] = Value::Null;
                registered_columns
                    .push(json!({"schema":"public","table":item["table"],"column":item["column"]}));
            }
        }
        for item in &registered_columns {
            let table = item["table"].as_str().unwrap();
            let column = item["column"].as_str().unwrap();
            let mut check = constraint(table, &format!("{table}_{column}_uuid_v7"), "c", &[column]);
            check["definition"] = json!(format!("CHECK (({column} IS NULL) OR (((pg_catalog.get_byte(pg_catalog.uuid_send({column}), 6) >> 4) = 7) AND ((pg_catalog.get_byte(pg_catalog.uuid_send({column}), 8) & 192) = 128)))"));
            target["constraints"].as_array_mut().unwrap().push(check);
            target["uuid_columns"].as_array_mut().unwrap().push(json!({"schema":"public","table":table,
                "column":column,"declared_type":"uuid","array_layers":0,"domains":[],"default":null}));
        }
        let registry = json!({"databases":[{"kind":"work_orders","columns":registered_columns,
            "tables":specs.iter().map(|(table,policy,_)|json!({"schema":"public","table":table,"retention":policy})).collect::<Vec<_>>()}]});
        let source = export::DatabaseSnapshot {
            kind: "work_orders".into(),
            name: "fixture_preview".into(),
            catalog: source_catalog,
            snapshot: "1:2:".into(),
            lsn: "0/10".into(),
            final_lsn: "0/20".into(),
            sequences: vec![],
            tables: specs
                .iter()
                .map(|(table, _, rows)| export::TableSnapshot {
                    schema: "public".into(),
                    table: (*table).into(),
                    rows: *rows,
                    file: String::new(),
                    purpose: String::new(),
                    digest: digest(),
                })
                .collect(),
        };
        let converted = transform::ConvertedDatabase {
            kind: "work_orders".into(),
            tables: specs
                .iter()
                .map(|(table, policy, rows)| transform::ConvertedTable {
                    schema: "public".into(),
                    table: (*table).into(),
                    policy: (*policy).into(),
                    source_rows: *rows,
                    rows: if matches!(*policy, "REBUILD_METADATA" | "CREDENTIAL_EXCLUDED") {
                        0
                    } else {
                        *rows
                    },
                    file: String::new(),
                    purpose: String::new(),
                    digest: digest(),
                    proof_file: String::new(),
                    proof_purpose: String::new(),
                    proof_digest: digest(),
                })
                .collect(),
        };
        (target, source, converted, registry)
    }

    #[test]
    fn full_columns_use_actual_position_and_foreign_keys_order_retained_tables() {
        let (target, source, converted, registry) = fixture();
        let result = plans("work_orders", &target, &source, &converted, &registry).unwrap();
        assert_eq!(
            result.iter().map(|p| p.table.as_str()).collect::<Vec<_>>(),
            ["nodes", "children"]
        );
        assert_eq!(
            result[0].columns,
            ["name", "id", "parent_id", "receipt", "facts"]
        );
        assert_eq!(result[0].primary_key, ["id"]);
        assert_eq!(result[0].nullable_json_columns, ["receipt"]);
        assert_eq!(result[0].json_columns, ["receipt", "facts"]);
        assert_eq!(result[0].rows, 3);
        assert_eq!(result[0].policy, "RETAIN_REMAP");
    }

    fn fixture_with_not_null_constraint() -> (
        Value,
        export::DatabaseSnapshot,
        transform::ConvertedDatabase,
        Value,
    ) {
        let (mut target, mut source, converted, registry) = fixture();
        let mut not_null = constraint("nodes", "nodes_id_not_null", "n", &["id"]);
        not_null["definition"] = json!("NOT NULL id");
        source.catalog["constraints"]
            .as_array_mut()
            .unwrap()
            .push(not_null.clone());
        target["constraints"].as_array_mut().unwrap().push(not_null);
        (target, source, converted, registry)
    }

    #[test]
    fn matching_postgres_18_not_null_constraints_preserve_import_plans() {
        let (target, source, converted, registry) = fixture_with_not_null_constraint();
        let result = plans("work_orders", &target, &source, &converted, &registry).unwrap();
        assert_eq!(
            result.iter().map(|p| p.table.as_str()).collect::<Vec<_>>(),
            ["nodes", "children"]
        );
    }

    #[test]
    fn postgres_18_not_null_constraints_require_exact_source_target_match() {
        let (mut target, source, converted, registry) = fixture_with_not_null_constraint();
        target["constraints"].as_array_mut().unwrap().pop();
        assert!(plans("work_orders", &target, &source, &converted, &registry).is_err());

        let (target, mut source, converted, registry) = fixture_with_not_null_constraint();
        source.catalog["constraints"].as_array_mut().unwrap().pop();
        assert!(plans("work_orders", &target, &source, &converted, &registry).is_err());

        for (field, changed) in [
            ("definition", json!("NOT NULL parent_id")),
            ("columns", json!(["parent_id"])),
            ("name", json!("nodes_renamed_not_null")),
        ] {
            let (mut target, source, converted, registry) = fixture_with_not_null_constraint();
            target["constraints"]
                .as_array_mut()
                .unwrap()
                .last_mut()
                .unwrap()[field] = changed;
            assert!(
                plans("work_orders", &target, &source, &converted, &registry).is_err(),
                "changed NOT NULL {field} must fail"
            );
        }
    }

    #[test]
    fn unvalidated_not_null_and_unknown_constraint_types_fail_even_when_matching() {
        for (field, invalid) in [
            ("validated", json!(false)),
            ("validated", Value::Null),
            ("type", json!("x")),
        ] {
            let (mut target, mut source, converted, registry) = fixture_with_not_null_constraint();
            target["constraints"]
                .as_array_mut()
                .unwrap()
                .last_mut()
                .unwrap()[field] = invalid.clone();
            source.catalog["constraints"]
                .as_array_mut()
                .unwrap()
                .last_mut()
                .unwrap()[field] = invalid;
            assert!(
                plans("work_orders", &target, &source, &converted, &registry).is_err(),
                "matching invalid constraint {field} must fail"
            );
        }
    }

    #[test]
    fn altered_columns_unknown_tables_and_incomplete_manifests_fail() {
        let (mut target, source, converted, registry) = fixture();
        target["schema_material"][2]["type"] = json!("text");
        assert!(plans("work_orders", &target, &source, &converted, &registry).is_err());
        let (mut target, source, converted, registry) = fixture();
        target["relations"].as_array_mut().unwrap().push(json!({"schema":"public","table":"surprise","kind":"r","is_partition":false,"row_count":0}));
        assert!(plans("work_orders", &target, &source, &converted, &registry).is_err());
        let (target, mut source, converted, registry) = fixture();
        source.tables.pop();
        assert!(plans("work_orders", &target, &source, &converted, &registry).is_err());
        let (target, source, mut converted, registry) = fixture();
        converted.tables.pop();
        assert!(plans("work_orders", &target, &source, &converted, &registry).is_err());
        let (mut target, source, converted, registry) = fixture();
        target["schema_material"][2]["generated"] = json!("s");
        assert!(plans("work_orders", &target, &source, &converted, &registry).is_err());
    }

    #[test]
    fn uuid_defaults_invalid_guards_and_changed_checks_are_rejected() {
        let (mut target, source, converted, registry) = fixture();
        target["schema_material"][3]["default"] = json!("gen_random_uuid()");
        assert!(plans("work_orders", &target, &source, &converted, &registry).is_err());
        let (mut target, source, converted, registry) = fixture();
        target["constraints"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap()["validated"] = json!(false);
        assert!(plans("work_orders", &target, &source, &converted, &registry).is_err());
        let (mut target, source, converted, registry) = fixture();
        target["constraints"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap()["definition"] = json!("CHECK (true)");
        assert!(plans("work_orders", &target, &source, &converted, &registry).is_err());
        let (mut target, source, converted, registry) = fixture();
        target["constraints"][0]["definition"] = json!("changed ordinary primary key");
        assert!(plans("work_orders", &target, &source, &converted, &registry).is_err());
        let (mut target, source, converted, registry) = fixture();
        target["uuid_columns"].as_array_mut().unwrap().pop();
        assert!(plans("work_orders", &target, &source, &converted, &registry).is_err());
    }

    #[test]
    fn uuid_guard_comparison_preserves_operator_and_function_argument_parentheses() {
        let mut guard = constraint("nodes", "nodes_id_uuid_v7", "c", &["id"]);
        guard["definition"] = json!("CHECK ((((get_byte(uuid_send(id),6)>>4)=7) AND ((get_byte(uuid_send(id),8)&192)=128)))");
        assert!(uuid_guard("stores", "nodes", "id", &guard));
        guard["definition"] = json!("CHECK (((get_byte(uuid_send(id),(6>>4))=7) AND ((get_byte(uuid_send(id),8)&192)=128)))");
        assert!(!uuid_guard("stores", "nodes", "id", &guard));
        guard["definition"] = json!("CHECK (((id IS NULL) OR (((get_byte(uuid_send(id),6)>>4)=7) AND ((get_byte(uuid_send(id),8)&192)=128))))");
        assert!(uuid_guard("work_orders", "nodes", "id", &guard));
        let (mut target, _, _, registry) = fixture();
        for guard in target["constraints"].as_array_mut().unwrap() {
            if guard["name"].as_str().unwrap().ends_with("_uuid_v7") {
                let column = guard["columns"][0].as_str().unwrap().to_owned();
                guard["name"] = json!(format!("uuid_v7_{column}"));
                guard["definition"] = json!(format!("CHECK (public.is_uuid_v7({column}))"));
            }
        }
        let definition="CREATE OR REPLACE FUNCTION public.is_uuid_v7(value uuid)\n RETURNS boolean\n LANGUAGE sql\n IMMUTABLE PARALLEL SAFE STRICT\n SET search_path TO 'pg_catalog'\n AS $function$ SELECT (get_byte(uuid_send(value),6)>>4)=7 AND (get_byte(uuid_send(value),8)&192)=128 $function$\n";
        target["functions"] = json!([{"schema":"public","name":"is_uuid_v7","arguments":"value uuid",
            "security_definer":false,"volatility":"i","parallel":"s","configuration":["search_path=pg_catalog"],
            "definition":definition}]);
        assert!(validate_uuid_columns("platform", &target, &registry["databases"][0]).is_ok());
        target["functions"][0]["definition"] = json!(definition.replace(
            "(get_byte(uuid_send(value),6)>>4)=7",
            "get_byte(uuid_send(value),(6>>4))=7"
        ));
        assert!(validate_uuid_columns("platform", &target, &registry["databases"][0]).is_err());
    }

    #[test]
    fn excluded_rows_dirty_targets_and_cross_table_cycles_fail_closed() {
        let (target, source, mut converted, registry) = fixture();
        converted.tables.last_mut().unwrap().rows = 2;
        assert!(plans("work_orders", &target, &source, &converted, &registry).is_err());
        let (mut target, source, converted, registry) = fixture();
        target["relations"][2]["row_count"] = json!(1);
        assert!(plans("work_orders", &target, &source, &converted, &registry).is_err());
        let (mut target, mut source, converted, registry) = fixture();
        let mut fk = constraint("nodes", "nodes_child_fkey", "f", &["id"]);
        fk["referenced_schema"] = json!("public");
        fk["referenced_table"] = json!("children");
        fk["referenced_columns"] = json!(["id"]);
        source.catalog["constraints"]
            .as_array_mut()
            .unwrap()
            .push(fk.clone());
        target["constraints"].as_array_mut().unwrap().push(fk);
        assert!(plans("work_orders", &target, &source, &converted, &registry).is_err());
    }

    #[test]
    fn csv_staging_and_single_insert_verify_each_ordinal_by_full_primary_key() {
        let (target, source, converted, registry) = fixture();
        let plan = plans("work_orders", &target, &source, &converted, &registry)
            .unwrap()
            .remove(0);
        let stage = stage_sql(&plan).unwrap();
        assert!(stage
            .contains("COPY pg_temp.uuid_import_stage(ordinal,body) FROM STDIN WITH (FORMAT csv)"));
        assert!(stage.contains("ordinal bigint PRIMARY KEY CHECK (ordinal>0)"));
        let sql = insert_verify_sql(&plan).unwrap();
        assert_eq!(sql.matches("INSERT INTO").count(), 1);
        assert!(sql.contains(
            "ROW(p.\"name\",p.\"id\",p.\"parent_id\",s.body->'receipt',s.body->'facts')"
        ));
        assert!(sql.contains("s.body->'receipt'='null'::jsonb"));
        assert!(!sql.contains("s.body->'facts'='null'::jsonb"));
        assert!(sql.contains("pg_catalog.to_jsonb(e.value) IS DISTINCT FROM s.body"));
        assert!(sql.contains("GROUP BY e.ordinal HAVING count(t.\"id\")<>1"));
        assert!(sql.contains("'ordinal',e.ordinal,'row',pg_catalog.to_jsonb(t)"));
        assert!(sql.contains("'kind','table_verified'"));
        assert!(sql.contains("DROP TABLE pg_temp.uuid_import_stage"));
        assert!(!sql.contains("ON CONFLICT"));
        assert!(!sql.contains("COMMIT;"));
        assert!(!sql.contains("\\."));
        assert!(!sql.contains("LATERALpg_catalog"));
        assert!(!sql.contains("ASSELECT"));
    }

    #[test]
    fn special_restore_guards_are_narrow_and_reenabled_before_verification() {
        let (target, source, converted, registry) = fixture();
        let mut plan = plans("work_orders", &target, &source, &converted, &registry)
            .unwrap()
            .remove(0);
        plan.table = "store_staff".into();
        let sql = insert_verify_sql(&plan).unwrap();
        let disable = sql
            .find("DISABLE TRIGGER store_staff_legacy_read_only")
            .unwrap();
        let insert = sql.find("INSERT INTO").unwrap();
        let enable = sql
            .find("ENABLE TRIGGER store_staff_legacy_read_only")
            .unwrap();
        let verify = sql.find("import actual row verification failed").unwrap();
        assert!(disable < insert && insert < enable && enable < verify);
        let trigger_context = sql
            .find("SET LOCAL search_path=pg_catalog,public;")
            .unwrap();
        let restored = sql.rfind("SET LOCAL search_path=pg_catalog;").unwrap();
        assert!(trigger_context < insert && insert < restored && restored < enable);
        assert!(!sql.contains("DISABLE TRIGGER ALL"));
        plan.table = "platform_settings".into();
        plan.rows = 1;
        let sql = insert_verify_sql(&plan).unwrap();
        assert!(sql.contains("target platform settings are not fresh migration defaults"));
        assert!(sql.contains("\"primary_color\":\"#008577\""));
        assert!(
            sql.find("fresh migration defaults").unwrap()
                < sql
                    .find("DELETE FROM ONLY public.platform_settings")
                    .unwrap()
        );
    }

    #[test]
    fn final_checks_keep_not_valid_history_and_reject_hierarchy_cycles() {
        let begin = begin_sql("platform").unwrap();
        assert!(begin.starts_with("BEGIN ISOLATION LEVEL READ COMMITTED READ WRITE;"));
        assert!(begin.contains("SET LOCAL timezone='UTC'"));
        let finish = finish_sql("platform").unwrap();
        for table in ["organizations", "departments", "menus"] {
            assert!(finish.contains(&format!("FROM ONLY public.{table} WHERE parent_id IS NULL")));
        }
        assert!(finish.contains("c.contype='f' OR (c.contype='c'"));
        assert!(finish.contains("'uuid_v7_'"));
        assert!(finish.contains("t.tgenabled<>'O'"));
        assert!(finish.contains("'kind','business_verified'"));
        assert!(!finish.contains("COMMIT;"));
        let stores = finish_sql("stores").unwrap();
        let context = stores
            .find("SET LOCAL search_path=public,pg_catalog;")
            .unwrap();
        let fingerprint = stores.find("stores_schema_fingerprint_v3").unwrap();
        let restored = stores.rfind("SET LOCAL search_path=pg_catalog;").unwrap();
        assert!(context < fingerprint && fingerprint < restored);
        assert!(restored < stores.find("'kind','business_verified'").unwrap());
        assert!(begin_sql("unknown").is_err());
        assert!(finish_sql("unknown").is_err());
        let (target, source, converted, registry) = fixture();
        let mut plan = plans("work_orders", &target, &source, &converted, &registry)
            .unwrap()
            .remove(0);
        plan.table = "nodes;DROP TABLE users".into();
        assert!(stage_sql(&plan).is_err());
    }
}
