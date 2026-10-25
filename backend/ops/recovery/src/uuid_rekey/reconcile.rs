//! Offline verification of a complete row projection, including owner hashes.
//!
//! Reuse the registered owners to derive the one permitted target from source
//! evidence; do not maintain a second set of JSON, URL or hash exemptions here.

use serde::Serialize;
use serde_json::{Map, Value};

use super::{mapping::TypedMap, rows, structured};
use crate::{Error, Result};

/// Private row evidence: digests bind semantic JSON serialization, not the
/// original evidence-file bytes (whose digest is recorded by the caller).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectionProof {
    pub(crate) source_sha256: String,
    pub(crate) target_sha256: String,
    /// Exact unchanged top-level columns. Changed owner JSON/text fields are
    /// bound in full by target_sha256, including their preserved inner hashes.
    pub(crate) business_sha256: String,
    pub(crate) retained_columns: u64,
    pub(crate) transformed_columns: u64,
}

/// Verify the complete projection after scalar and structured transformation.
/// Valid v7 syntax, a recomputed payload hash, or an unchanged row count alone
/// cannot satisfy this check: every resulting value must match its source owner.
pub(crate) fn verify_projection(
    kind: &str,
    table: &str,
    source: &Value,
    target: &Value,
    map: &TypedMap,
    registry: &Value,
) -> Result<()> {
    checked_expected(kind, table, source, target, map, registry).map(|_| ())
}

/// Produce an exact business-field digest only after full owner verification.
/// No field is omitted because its name contains "id", "hash", or "sha256".
pub(crate) fn projection_proof(
    kind: &str,
    table: &str,
    source: &Value,
    target: &Value,
    map: &TypedMap,
    registry: &Value,
) -> Result<ProjectionProof> {
    verify_projection(kind, table, source, target, map, registry)?;
    let source_fields = source
        .as_object()
        .ok_or(Error("invalid projection source row"))?;
    let target_fields = target
        .as_object()
        .ok_or(Error("invalid projection target row"))?;
    let mut retained = Map::new();
    for (column, value) in source_fields {
        if target_fields.get(column) == Some(value) {
            retained.insert(column.clone(), value.clone());
        }
    }
    let retained_columns = u64::try_from(retained.len())
        .map_err(|_| Error("projection evidence column count overflow"))?;
    let transformed_columns = u64::try_from(source_fields.len() - retained.len())
        .map_err(|_| Error("projection evidence column count overflow"))?;
    Ok(ProjectionProof {
        source_sha256: digest(kind, table, source)?,
        target_sha256: digest(kind, table, target)?,
        business_sha256: digest(kind, table, &Value::Object(retained))?,
        retained_columns,
        transformed_columns,
    })
}

fn checked_expected(
    kind: &str,
    table: &str,
    source: &Value,
    target: &Value,
    map: &TypedMap,
    registry: &Value,
) -> Result<Value> {
    let mut expected = source.clone();
    rows::rewrite_row(kind, table, &mut expected, map, registry)?;
    rows::verify_rewrite(kind, table, source, &expected, map, registry)?;
    structured::transform_row(kind, table, &mut expected, map, registry)?;
    if expected != *target {
        return Err(Error(
            "row projection differs from its registered source owners",
        ));
    }
    Ok(expected)
}

fn digest(kind: &str, table: &str, row: &Value) -> Result<String> {
    let table = table.strip_prefix("public.").unwrap_or(table);
    let evidence = serde_json::json!({"schemaVersion":1,"kind":kind,"table":table,"row":row});
    let bytes = serde_json::to_vec(&evidence)
        .map_err(|_| Error("projection evidence serialization failed"))?;
    Ok(crate::bundle::hash(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    const FROZEN: u64 = 1_792_800_000_123;

    fn old(payload: u128) -> Uuid {
        Uuid::from_u128(0x0011_2233_4455_4000_8000_0000_0000_0000 | payload)
    }

    fn registry() -> Value {
        serde_json::from_str(include_str!("../../uuid-references-v2.json")).unwrap()
    }

    fn mapping() -> TypedMap {
        let mut map = TypedMap::new(
            FROZEN,
            [
                "platform.user",
                "platform.organization",
                "platform.tenant",
                "platform.application",
                "platform.notification_publication",
                "platform.role",
                "work_orders.ticket",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        )
        .unwrap();
        for authority in [
            "platform.user",
            "platform.organization",
            "platform.tenant",
            "platform.application",
            "platform.notification_publication",
            "platform.role",
            "work_orders.ticket",
        ] {
            map.allocate(authority, old(1), None).unwrap();
        }
        map
    }

    fn mapped(map: &TypedMap, authority: &str) -> String {
        map.resolve(authority, old(1)).unwrap().to_string()
    }

    fn hash(value: &Value) -> String {
        crate::bundle::hash(&serde_json::to_vec(value).unwrap())
    }

    // Golden rows are assembled from the source owner's public DTO shape and
    // exact typed mappings, without running either transformation to make them.
    fn publication(map: &TypedMap) -> (Value, Value) {
        let source_body = json!({"target":{"kind":"USER","userId":old(1),"organizationId":old(1)},
            "type":"INFO","title":"业务标题","content":old(1).to_string(),
            "link":format!("/micro/work-orders/tickets/{}", old(1))});
        let source = json!({"id":old(1),"tenant_id":old(1),"publisher_organization_id":old(1),
            "application_id":old(1),"publisher_user_id":old(1),"request_body":source_body,
            "request_hash":hash(&source_body),"idempotency_key":"event-1","created_at":"2026-10-24T08:00:00Z"});
        let body = json!({"target":{"kind":"USER","userId":mapped(map,"platform.user"),
            "organizationId":mapped(map,"platform.organization")},"type":"INFO","title":"业务标题",
            "content":old(1).to_string(),"link":format!("/micro/work-orders/tickets/{}",mapped(map,"work_orders.ticket"))});
        let target = json!({"id":mapped(map,"platform.notification_publication"),"tenant_id":mapped(map,"platform.tenant"),
            "publisher_organization_id":mapped(map,"platform.organization"),"application_id":mapped(map,"platform.application"),
            "publisher_user_id":mapped(map,"platform.user"),"request_body":body,"request_hash":hash(&body),
            "idempotency_key":"event-1","created_at":"2026-10-24T08:00:00Z"});
        (source, target)
    }

    #[test]
    fn correct_projection_matches_golden_owner_shape_and_proof_is_stable() {
        let map = mapping();
        let registry = registry();
        let (source, target) = publication(&map);
        let original = source.clone();
        let target_saved = target.clone();
        verify_projection(
            "platform",
            "notification_publications",
            &source,
            &target,
            &map,
            &registry,
        )
        .unwrap();
        let proof = projection_proof(
            "platform",
            "notification_publications",
            &source,
            &target,
            &map,
            &registry,
        )
        .unwrap();
        let qualified = projection_proof(
            "platform",
            "public.notification_publications",
            &source,
            &target,
            &map,
            &registry,
        )
        .unwrap();
        assert_eq!(proof.business_sha256, qualified.business_sha256);
        assert_eq!(proof.target_sha256, qualified.target_sha256);
        assert_ne!(proof.source_sha256, proof.target_sha256);
        assert_eq!(proof.retained_columns, 2);
        assert_eq!(proof.transformed_columns, 7);
        let retained_evidence = json!({"schemaVersion":1,"kind":"platform","table":"notification_publications",
            "row":{"idempotency_key":"event-1","created_at":"2026-10-24T08:00:00Z"}});
        assert_eq!(proof.business_sha256, hash(&retained_evidence));
        assert_eq!(
            proof.source_sha256,
            hash(&json!({"schemaVersion":1,"kind":"platform",
            "table":"notification_publications","row":source}))
        );
        assert_eq!(
            proof.target_sha256,
            hash(&json!({"schemaVersion":1,"kind":"platform",
            "table":"notification_publications","row":target}))
        );
        let mut another_source = source.clone();
        let mut another_target = target.clone();
        another_source["created_at"] = json!("2026-10-25T08:00:00Z");
        another_target["created_at"] = another_source["created_at"].clone();
        let another = projection_proof(
            "platform",
            "notification_publications",
            &another_source,
            &another_target,
            &map,
            &registry,
        )
        .unwrap();
        assert_ne!(proof.business_sha256, another.business_sha256);
        assert_eq!(source, original);
        assert_eq!(target, target_saved);
    }

    #[test]
    fn missing_or_wrong_scalar_embedded_and_route_mappings_are_rejected() {
        let map = mapping();
        let registry = registry();
        let (source, target) = publication(&map);
        for (pointer, replacement) in [
            (
                "/publisher_user_id",
                json!(mapped(&map, "platform.organization")),
            ),
            ("/publisher_user_id", json!(old(1))),
            ("/request_body/target/userId", json!(old(1))),
            (
                "/request_body/target/userId",
                json!(mapped(&map, "platform.organization")),
            ),
            ("/request_body/link", source["request_body"]["link"].clone()),
        ] {
            let mut incorrect = target.clone();
            *incorrect.pointer_mut(pointer).unwrap() = replacement;
            assert!(verify_projection(
                "platform",
                "notification_publications",
                &source,
                &incorrect,
                &map,
                &registry
            )
            .is_err());
        }
        assert!(verify_projection(
            "platform",
            "notification_publications",
            &source,
            &source,
            &map,
            &registry
        )
        .is_err());
    }

    #[test]
    fn hashes_are_owner_bound_and_valid_rehash_cannot_hide_business_tampering() {
        let map = mapping();
        let registry = registry();
        let (source, target) = publication(&map);
        for bad_hash in [source["request_hash"].clone(), json!("b".repeat(64))] {
            let mut incorrect = target.clone();
            incorrect["request_hash"] = bad_hash;
            assert!(verify_projection(
                "platform",
                "notification_publications",
                &source,
                &incorrect,
                &map,
                &registry
            )
            .is_err());
        }
        let mut tampered = target.clone();
        tampered["request_body"]["title"] = json!("changed business title");
        tampered["request_hash"] = json!(hash(&tampered["request_body"]));
        assert!(verify_projection(
            "platform",
            "notification_publications",
            &source,
            &tampered,
            &map,
            &registry
        )
        .is_err());
        let mut bad_source = source.clone();
        bad_source["request_hash"] = json!("a".repeat(64));
        assert!(verify_projection(
            "platform",
            "notification_publications",
            &bad_source,
            &target,
            &map,
            &registry
        )
        .is_err());
    }

    #[test]
    fn reminder_projection_requires_the_exact_owner_order_and_row_identity() {
        let map = mapping();
        let registry = registry();
        let raw = |user: &str, org: &str, ticket: &str| {
            format!(
                r#"{{"idempotencyKey":"event-1","target":{{"kind":"USER","userId":"{user}","organizationId":"{org}"}},"type":"INFO","title":"工单提醒 WO-000001","content":"请查看工单 WO-000001 的详情并处理。","link":"/micro/work-orders/tickets/{ticket}"}}"#
            )
        };
        let source_json = raw(
            &old(1).to_string(),
            &old(1).to_string(),
            &old(1).to_string(),
        );
        let source = json!({"tenant_id":old(1),"organization_id":old(1),"application_id":old(1),
            "user_id":old(1),"ticket_id":old(1),"target_organization_id":old(1),"recipient_user_id":old(1),
            "idempotency_key":"event-1","request_json":source_json,
            "request_hash":crate::bundle::hash(source_json.as_bytes()),"version":9223372036854775807_i64});
        let mut target = source.clone();
        for (column, authority) in [
            ("tenant_id", "platform.tenant"),
            ("organization_id", "platform.organization"),
            ("application_id", "platform.application"),
            ("user_id", "platform.user"),
            ("ticket_id", "work_orders.ticket"),
            ("target_organization_id", "platform.organization"),
            ("recipient_user_id", "platform.user"),
        ] {
            target[column] = json!(mapped(&map, authority));
        }
        let target_json = raw(
            &mapped(&map, "platform.user"),
            &mapped(&map, "platform.organization"),
            &mapped(&map, "work_orders.ticket"),
        );
        target["request_hash"] = json!(crate::bundle::hash(target_json.as_bytes()));
        target["request_json"] = json!(target_json);
        verify_projection(
            "work_orders",
            "work_order_reminder_intents",
            &source,
            &target,
            &map,
            &registry,
        )
        .unwrap();
        let payload: Value = serde_json::from_str(&target_json).unwrap();
        let reordered = serde_json::to_string(&payload).unwrap();
        assert_ne!(reordered, target_json);
        let mut noncanonical = target.clone();
        noncanonical["request_hash"] = json!(crate::bundle::hash(reordered.as_bytes()));
        noncanonical["request_json"] = json!(reordered);
        assert!(verify_projection(
            "work_orders",
            "work_order_reminder_intents",
            &source,
            &noncanonical,
            &map,
            &registry
        )
        .is_err());
        target["recipient_user_id"] = json!(mapped(&map, "platform.organization"));
        assert!(verify_projection(
            "work_orders",
            "work_order_reminder_intents",
            &source,
            &target,
            &map,
            &registry
        )
        .is_err());
    }

    #[test]
    fn precise_amounts_bigints_timestamps_and_opaque_hashes_cannot_be_lost_or_changed() {
        let map = mapping();
        let registry = registry();
        let source: Value = serde_json::from_str(&format!(r#"{{"tenant_id":"{}","next_number":9223372036854775807,"amount":12345678901234567890.123456789012345678901,"business_sha256":"{}","business_date":"2026-10-24","payload":{{"freeText":"{}"}}}}"#,old(1),"a".repeat(64),old(1))).unwrap();
        let mut target = source.clone();
        target["tenant_id"] = json!(mapped(&map, "platform.tenant"));
        let proof = projection_proof(
            "work_orders",
            "work_order_counters",
            &source,
            &target,
            &map,
            &registry,
        )
        .unwrap();
        assert_eq!(proof.retained_columns, 5);
        assert_eq!(proof.transformed_columns, 1);
        for (column, changed) in [
            ("next_number", json!(9007199254740992_u64)),
            ("amount", json!(1.25)),
            ("business_sha256", json!("b".repeat(64))),
            ("business_date", json!("2026-10-25")),
            ("payload", json!({"freeText":mapped(&map,"platform.user")})),
        ] {
            let mut incorrect = target.clone();
            incorrect[column] = changed;
            assert!(projection_proof(
                "work_orders",
                "work_order_counters",
                &source,
                &incorrect,
                &map,
                &registry
            )
            .is_err());
            let mut missing = target.clone();
            missing.as_object_mut().unwrap().remove(column);
            assert!(verify_projection(
                "work_orders",
                "work_order_counters",
                &source,
                &missing,
                &map,
                &registry
            )
            .is_err());
        }
    }

    #[test]
    fn historical_owner_hash_stays_original_inside_changed_audit_text() {
        let map = mapping();
        let registry = registry();
        let historical_hash = "a".repeat(64);
        let state =
            json!({"id":old(1),"name":"role","menuCount":2,"menuIdsSha256":historical_hash});
        let detail =
            json!({"objectType":"role_menus","objectId":old(1),"before":state,"after":state});
        let source = json!({"id":9223372036854775807_i64,"user_id":old(1),"tenant_id":null,"organization_id":null,
            "request_id":null,"application_id":null,"action":"ROLE_MENUS","detail":serde_json::to_string(&detail).unwrap()});
        let new_state = json!({"id":mapped(&map,"platform.role"),"name":"role","menuCount":2,"menuIdsSha256":historical_hash});
        let projected_detail = json!({"objectType":"role_menus","objectId":mapped(&map,"platform.role"),"before":new_state,"after":new_state});
        let mut target = source.clone();
        target["user_id"] = json!(mapped(&map, "platform.user"));
        target["detail"] = json!(serde_json::to_string(&projected_detail).unwrap());
        verify_projection("platform", "audit_logs", &source, &target, &map, &registry).unwrap();
        let proof =
            projection_proof("platform", "audit_logs", &source, &target, &map, &registry).unwrap();
        assert_eq!(
            proof.target_sha256,
            hash(&json!({"schemaVersion":1,"kind":"platform","table":"audit_logs","row":target}))
        );
        let mut tampered_detail = projected_detail;
        tampered_detail["after"]["menuIdsSha256"] = json!("b".repeat(64));
        target["detail"] = json!(serde_json::to_string(&tampered_detail).unwrap());
        assert!(
            verify_projection("platform", "audit_logs", &source, &target, &map, &registry).is_err()
        );
    }

    #[test]
    fn unknown_contracts_missing_authority_and_extra_columns_fail_without_private_values() {
        let map = mapping();
        let registry = registry();
        let (source, mut target) = publication(&map);
        target["private-extra"] = json!(old(1));
        let error = verify_projection(
            "platform",
            "notification_publications",
            &source,
            &target,
            &map,
            &registry,
        )
        .err()
        .unwrap();
        assert!(!error.to_string().contains(&old(1).to_string()));
        assert!(!format!("{error:?}").contains("private-extra"));
        assert!(verify_projection(
            "platform",
            "unknown-private-table",
            &source,
            &source,
            &map,
            &registry
        )
        .is_err());
        let empty = TypedMap::new(FROZEN, ["platform.user".to_owned()].into()).unwrap();
        assert!(verify_projection(
            "platform",
            "notification_publications",
            &source,
            &source,
            &empty,
            &registry
        )
        .is_err());
    }
}
