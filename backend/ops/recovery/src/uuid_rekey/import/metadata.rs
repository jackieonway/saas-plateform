//! Read-only proof of histories written by the actual target service migrators.
//! Source metadata is never imported, repaired, or used to bless target history.

use ring::digest;
use serde::Deserialize;
use serde_json::Value;

use crate::{Error, Result};

struct Migration {
    version: i64,
    description: &'static str,
    bytes: &'static [u8],
    legacy_version: Option<&'static str>,
}

macro_rules! migration {
    ($version:literal, $description:literal, $directory:literal, $file:literal) => {
        Migration {
            version: $version,
            description: $description,
            bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), $directory, $file)),
            legacy_version: None,
        }
    };
    ($version:literal, $description:literal, $directory:literal, $file:literal, legacy) => {
        Migration {
            version: $version,
            description: $description,
            bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), $directory, $file)),
            legacy_version: Some($file),
        }
    };
}

// These bytes have the same line endings as the service binaries built from
// this checkout. A newly initialized target uses canonical history, including
// platform v1; old platform compatibility checksums do not apply to new targets.
const PLATFORM: &[Migration] = &[
    migration!(1, "init", "/../../migrations/", "0001_init.sql"),
    migration!(
        2,
        "release artifacts",
        "/../../migrations/",
        "0002_release_artifacts.sql"
    ),
    migration!(
        3,
        "oauth issuance",
        "/../../migrations/",
        "0003_oauth_issuance.sql"
    ),
    migration!(
        4,
        "release operation protection",
        "/../../migrations/",
        "0004_release_operation_protection.sql"
    ),
    migration!(
        5,
        "managed image deployments",
        "/../../migrations/",
        "0005_managed_image_deployments.sql"
    ),
    migration!(
        6,
        "zip distribution",
        "/../../migrations/",
        "0006_zip_distribution.sql"
    ),
    migration!(
        7,
        "oauth pkce policy",
        "/../../migrations/",
        "0007_oauth_pkce_policy.sql"
    ),
    migration!(
        8,
        "url entries",
        "/../../migrations/",
        "0008_url_entries.sql"
    ),
    migration!(
        9,
        "qiankun only",
        "/../../migrations/",
        "0009_qiankun_only.sql"
    ),
    migration!(
        10,
        "unified application config",
        "/../../migrations/",
        "0010_unified_application_config.sql"
    ),
    migration!(
        11,
        "store organizations",
        "/../../migrations/",
        "0011_store_organizations.sql"
    ),
    migration!(12, "uuid v7", "/../../migrations/", "0012_uuid_v7.sql"),
];

const STORES: &[Migration] = &[
    migration!(
        1,
        "stores",
        "/../../../../saas-stores/backend/migrations/",
        "0001_stores.sql"
    ),
    migration!(
        2,
        "platform owned store identity",
        "/../../../../saas-stores/backend/migrations/",
        "0002_platform_owned_store_identity.sql"
    ),
    migration!(
        3,
        "uuid v7",
        "/../../../../saas-stores/backend/migrations/",
        "0003_uuid_v7.sql"
    ),
];

// Work orders constructs these four descriptions and legacy filenames itself;
// it does not obtain its SQLx history by blindly executing migration files.
const WORK_ORDERS: &[Migration] = &[
    migration!(
        1,
        "work orders",
        "/../../../../saas-work-orders/backend/migrations/",
        "0001_work_orders.sql",
        legacy
    ),
    migration!(
        2,
        "create idempotency",
        "/../../../../saas-work-orders/backend/migrations/",
        "0002_create_idempotency.sql",
        legacy
    ),
    migration!(
        3,
        "reminder intents",
        "/../../../../saas-work-orders/backend/migrations/",
        "0003_reminder_intents.sql",
        legacy
    ),
    migration!(
        4,
        "uuid v7",
        "/../../../../saas-work-orders/backend/migrations/",
        "0004_uuid_v7.sql",
        legacy
    ),
];

const WORK_ORDERS_BASELINE_SHA256: &str =
    "a2e4224febd195fdd663a8954c3be4b76a8a06338521056c6809ebaa57d04090";

// This is the actual stores readiness contract. The prior snapshots remain
// historical: comparing v1 directly with today's v1 function would be wrong
// after later owner migrations deliberately change the underlying catalog.
const STORES_CONTRACT: &str = "SELECT count(*)=1 AND COALESCE(bool_and(v3.fingerprint=public.stores_schema_fingerprint_v3() AND v3.legacy_fingerprint=v2.fingerprint AND v2.legacy_fingerprint=v1.fingerprint),false) FROM public._stores_schema_contract_v3 AS v3 JOIN public._stores_schema_contract_v2 AS v2 USING(singleton) JOIN public._stores_schema_contract AS v1 USING(singleton) WHERE v3.singleton";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    sqlx_count: u64,
    sqlx: Vec<SqlxRow>,
    legacy_count: u64,
    legacy: Vec<LegacyRow>,
    schema_contract_valid: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SqlxRow {
    version: i64,
    checksum: String,
    success: bool,
    description_matches: bool,
    installed_on_present: bool,
    execution_time_valid: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyRow {
    version_matches: bool,
    checksum: String,
    applied_at_present: bool,
}

fn migrations(kind: &str) -> Result<&'static [Migration]> {
    match kind {
        "platform" => Ok(PLATFORM),
        "stores" => Ok(STORES),
        "work_orders" => {
            if checksum(&digest::SHA256, WORK_ORDERS[0].bytes) != WORK_ORDERS_BASELINE_SHA256 {
                return Err(Error("embedded work-order migration baseline mismatch"));
            }
            Ok(WORK_ORDERS)
        }
        _ => Err(Error("unsupported target migration kind")),
    }
}

fn checksum(algorithm: &'static digest::Algorithm, bytes: &[u8]) -> String {
    let digest = digest::digest(algorithm, bytes);
    let alphabet = b"0123456789abcdef";
    let mut output = String::with_capacity(digest.as_ref().len() * 2);
    for byte in digest.as_ref() {
        output.push(char::from(alphabet[usize::from(byte >> 4)]));
        output.push(char::from(alphabet[usize::from(byte & 15)]));
    }
    output
}

fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// One read-only JSON result, containing only counts, versions, digests and
/// booleans. Missing history relations/functions cause the caller's SQL session
/// to fail; they must never be repaired here. `schema_contract_valid=true` is
/// the explicit not-applicable value for platform and work orders.
pub(super) fn query(kind: &str) -> Result<String> {
    let expected = migrations(kind)?;
    let description_cases = expected
        .iter()
        .map(|migration| {
            format!(
                "WHEN {} THEN m.description={}",
                migration.version,
                literal(migration.description)
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    // Bound emitted rows even if an isolated target has unexpected metadata;
    // the independent count still rejects every omitted/extra version.
    let sqlx_limit = expected.len() + 1;
    let sqlx = format!(
        "COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object(\
         'version',m.version,'checksum',pg_catalog.encode(m.checksum,'hex'),\
         'success',m.success,\
         'description_matches',CASE m.version {description_cases} ELSE false END,\
         'installed_on_present',m.installed_on IS NOT NULL,\
         'execution_time_valid',m.execution_time>=-1) ORDER BY m.version) \
         FROM (SELECT version,description,checksum,success,installed_on,execution_time \
         FROM public._sqlx_migrations ORDER BY version LIMIT {sqlx_limit}) AS m),'[]'::jsonb)"
    );
    let (legacy_count, legacy) = if kind == "work_orders" {
        let cases = expected
            .iter()
            .enumerate()
            .map(|(index, migration)| {
                migration
                    .legacy_version
                    .map(|version| {
                        format!("WHEN {} THEN m.version={}", index + 1, literal(version))
                    })
                    .ok_or(Error("embedded work-order legacy version missing"))
            })
            .collect::<Result<Vec<_>>>()?
            .join(" ");
        (
            "(SELECT count(*) FROM public.work_order_migrations)".to_owned(),
            format!(
                "COALESCE((SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object(\
                 'version_matches',CASE m.ordinal {cases} ELSE false END,\
                 'checksum',m.checksum,'applied_at_present',m.applied_at IS NOT NULL) \
                 ORDER BY m.ordinal) FROM (SELECT version,checksum,applied_at,\
                 pg_catalog.row_number() OVER (ORDER BY version) AS ordinal \
                 FROM public.work_order_migrations ORDER BY version LIMIT {}) AS m),'[]'::jsonb)",
                expected.len() + 1
            ),
        )
    } else {
        ("0".to_owned(), "'[]'::jsonb".to_owned())
    };
    let contract = if kind == "stores" {
        format!("({STORES_CONTRACT})")
    } else {
        "true".to_owned()
    };
    let query = format!(
        "SELECT pg_catalog.jsonb_build_object(\
         'sqlx_count',(SELECT count(*) FROM public._sqlx_migrations),\
         'sqlx',{sqlx},'legacy_count',{legacy_count},'legacy',{legacy},\
         'schema_contract_valid',{contract});"
    );
    // The stores owner contract fingerprints pg_get_* deparse output using
    // its migrator's public,pg_catalog context. Preserve those exact bytes,
    // then restore the importer's pinned catalog context before the next SQL.
    // No relation or function lookup below relies on the temporary path.
    if kind == "stores" {
        Ok(format!(
            "SET LOCAL search_path=public,pg_catalog;\n{query}\nSET LOCAL search_path=pg_catalog;"
        ))
    } else {
        Ok(query)
    }
}

/// Require complete canonical migration identity, not merely the latest
/// version or a successful prefix. Errors never echo rows or native SQL.
pub(super) fn validate(kind: &str, value: &Value) -> Result<()> {
    let expected = migrations(kind)?;
    let metadata: Metadata = serde_json::from_value(value.clone())
        .map_err(|_| Error("invalid target migration metadata result"))?;
    let expected_count =
        u64::try_from(expected.len()).map_err(|_| Error("target migration count overflow"))?;
    if metadata.sqlx_count != expected_count
        || metadata.sqlx.len() != expected.len()
        || !metadata.schema_contract_valid
    {
        return Err(Error("target migration metadata mismatch"));
    }
    for (row, migration) in metadata.sqlx.iter().zip(expected) {
        if row.version != migration.version
            || row.checksum != checksum(&digest::SHA384, migration.bytes)
            || !row.success
            || !row.description_matches
            || !row.installed_on_present
            // Match the real SQLx verifier: -1 is a valid interrupted timing
            // bookkeeping value, while migration success is checked separately.
            || !row.execution_time_valid
        {
            return Err(Error("target SQLx migration history mismatch"));
        }
    }
    if kind == "work_orders" {
        if metadata.legacy_count != expected_count || metadata.legacy.len() != expected.len() {
            return Err(Error("target work-order legacy history mismatch"));
        }
        for (row, migration) in metadata.legacy.iter().zip(expected) {
            if !row.version_matches
                || row.checksum != checksum(&digest::SHA256, migration.bytes)
                || !row.applied_at_present
            {
                return Err(Error("target work-order legacy history mismatch"));
            }
        }
    } else if metadata.legacy_count != 0 || !metadata.legacy.is_empty() {
        return Err(Error("unexpected target legacy migration metadata"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn valid(kind: &str) -> Value {
        let expected = migrations(kind).expect("supported kind");
        let sqlx = expected
            .iter()
            .map(|migration| {
                json!({
                    "version":migration.version,
                    "checksum":checksum(&digest::SHA384,migration.bytes),
                    "success":true,"description_matches":true,
                    "installed_on_present":true,"execution_time_valid":true
                })
            })
            .collect::<Vec<_>>();
        let legacy = if kind == "work_orders" {
            expected
                .iter()
                .map(|migration| {
                    json!({"version_matches":true,
                    "checksum":checksum(&digest::SHA256,migration.bytes),
                    "applied_at_present":true})
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        json!({"sqlx_count":sqlx.len(),"sqlx":sqlx,"legacy_count":legacy.len(),
            "legacy":legacy,"schema_contract_valid":true})
    }

    #[test]
    fn complete_real_migration_bytes_validate_for_all_services() {
        for (kind, count) in [("platform", 12), ("stores", 3), ("work_orders", 4)] {
            let value = valid(kind);
            assert_eq!(value["sqlx_count"], count);
            validate(kind, &value).expect("complete exact history");
            let rows = value["sqlx"].as_array().expect("rows");
            assert!(rows
                .iter()
                .all(|row| row["checksum"].as_str().unwrap().len() == 96));
        }
        assert_eq!(
            checksum(&digest::SHA256, WORK_ORDERS[0].bytes),
            WORK_ORDERS_BASELINE_SHA256
        );
        assert_eq!(
            WORK_ORDERS[1].legacy_version,
            Some("0002_create_idempotency.sql")
        );
        assert_eq!(WORK_ORDERS[1].description, "create idempotency");
    }

    #[test]
    fn missing_extra_and_same_count_wrong_versions_are_rejected() {
        for kind in ["platform", "stores", "work_orders"] {
            let mut missing = valid(kind);
            missing["sqlx"].as_array_mut().unwrap().pop();
            assert!(validate(kind, &missing).is_err());

            let mut extra = valid(kind);
            let mut row = extra["sqlx"][0].clone();
            row["version"] = json!(999);
            extra["sqlx"].as_array_mut().unwrap().push(row);
            let count = extra["sqlx_count"].as_u64().unwrap() + 1;
            extra["sqlx_count"] = json!(count);
            assert!(validate(kind, &extra).is_err());

            let mut replaced = valid(kind);
            replaced["sqlx"][0]["version"] = json!(999);
            assert!(validate(kind, &replaced).is_err());

            let mut wrong_count = valid(kind);
            wrong_count["sqlx_count"] = json!(999);
            assert!(validate(kind, &wrong_count).is_err());
        }
    }

    #[test]
    fn sqlx_checksum_and_every_success_proof_are_required() {
        for kind in ["platform", "stores", "work_orders"] {
            let mut changed = valid(kind);
            changed["sqlx"][0]["checksum"] = json!("0".repeat(96));
            assert!(validate(kind, &changed).is_err());
            for field in [
                "success",
                "description_matches",
                "installed_on_present",
                "execution_time_valid",
            ] {
                let mut changed = valid(kind);
                changed["sqlx"][0][field] = json!(false);
                assert!(validate(kind, &changed).is_err(), "{kind} {field}");
            }
        }
    }

    #[test]
    fn stores_contract_failure_is_rejected_after_history_success() {
        let mut value = valid("stores");
        value["schema_contract_valid"] = json!(false);
        assert!(validate("stores", &value).is_err());
    }

    #[test]
    fn work_orders_requires_both_complete_distinct_checksum_histories() {
        let mut swapped = valid("work_orders");
        swapped["legacy"][0]["checksum"] = swapped["sqlx"][0]["checksum"].clone();
        assert!(validate("work_orders", &swapped).is_err());

        let mut changed_checksum = valid("work_orders");
        changed_checksum["legacy"][0]["checksum"] = json!("0".repeat(64));
        assert!(validate("work_orders", &changed_checksum).is_err());

        for field in ["version_matches", "applied_at_present"] {
            let mut changed = valid("work_orders");
            changed["legacy"][0][field] = json!(false);
            assert!(validate("work_orders", &changed).is_err());
        }
        let mut missing = valid("work_orders");
        missing["legacy"].as_array_mut().unwrap().pop();
        assert!(validate("work_orders", &missing).is_err());

        let mut extra = valid("work_orders");
        let row = extra["legacy"][0].clone();
        extra["legacy"].as_array_mut().unwrap().push(row);
        extra["legacy_count"] = json!(5);
        assert!(validate("work_orders", &extra).is_err());
    }

    #[test]
    fn malformed_unknown_and_cross_service_fields_are_rejected() {
        let mut missing = valid("platform");
        missing.as_object_mut().unwrap().remove("sqlx");
        assert!(validate("platform", &missing).is_err());

        let mut extra = valid("platform");
        extra["unexpected"] = json!(true);
        assert!(validate("platform", &extra).is_err());
        let mut extra_row = valid("platform");
        extra_row["sqlx"][0]["unexpected"] = json!(true);
        assert!(validate("platform", &extra_row).is_err());

        let mut bad_type = valid("platform");
        bad_type["sqlx_count"] = json!("12");
        assert!(validate("platform", &bad_type).is_err());

        let mut legacy = valid("stores");
        legacy["legacy_count"] = json!(1);
        legacy["legacy"] = json!([{"version_matches":true,"checksum":"0".repeat(64),
            "applied_at_present":true}]);
        assert!(validate("stores", &legacy).is_err());
        assert!(validate("stores", &valid("work_orders")).is_err());
        assert!(validate("unknown", &valid("platform")).is_err());
        assert!(query("unknown").is_err());
    }

    #[test]
    fn query_is_bounded_read_only_and_checks_real_owner_contracts() {
        for kind in ["platform", "stores", "work_orders"] {
            let statement = query(kind).expect("read-only statement");
            if kind == "stores" {
                assert!(statement.starts_with("SET LOCAL search_path=public,pg_catalog;\nSELECT pg_catalog.jsonb_build_object("));
                assert!(statement.ends_with(";\nSET LOCAL search_path=pg_catalog;"));
                assert_eq!(statement.matches(';').count(), 3);
            } else {
                assert!(statement.starts_with("SELECT pg_catalog.jsonb_build_object("));
                assert_eq!(statement.matches(';').count(), 1);
            }
            for mutation in [
                "INSERT ", "UPDATE ", "DELETE ", "CREATE ", "ALTER ", "DROP ",
            ] {
                assert!(!statement.contains(mutation));
            }
            let limit = migrations(kind).unwrap().len() + 1;
            assert!(statement.contains(&format!("ORDER BY version LIMIT {limit}")));
            assert!(statement.contains("'execution_time_valid',m.execution_time>=-1"));
            assert!(!statement.contains("installed_on',"));
        }
        let stores = query("stores").unwrap();
        assert!(stores.contains(STORES_CONTRACT));
        let orders = query("work_orders").unwrap();
        assert!(orders.contains("WHEN 2 THEN m.version='0002_create_idempotency.sql'"));
        assert!(orders.contains("WHEN 2 THEN m.description='create idempotency'"));
        assert!(orders.contains("pg_catalog.row_number() OVER (ORDER BY version)"));
    }
}
