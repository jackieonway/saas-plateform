use super::*;
use serde_json::json;

const OLD_USER: &str = "00112233-4455-4677-8899-aabbccddeeff";
const OLD_ORG: &str = "10112233-4455-4677-8899-aabbccddeeff";
const OLD_TICKET: &str = "20112233-4455-4677-8899-aabbccddeeff";
const OLD_REVISION: &str = "30112233-4455-4677-8899-aabbccddeeff";

fn registry() -> Value {
    serde_json::from_str(include_str!("../../../uuid-references-v2.json"))
        .expect("fixture registry")
}

fn fixture_map() -> TypedMap {
    let authorities = [
        "platform.user",
        "platform.organization",
        "work_orders.ticket",
        "platform.application_url_revision",
        "platform.manual_region_source",
    ];
    let mut map = TypedMap::new(
        1_700_000_000_000,
        authorities.into_iter().map(str::to_owned).collect(),
    )
    .expect("fixture policy");
    for (authority, old) in [
        ("platform.user", OLD_USER),
        ("platform.organization", OLD_ORG),
        ("work_orders.ticket", OLD_TICKET),
        ("platform.application_url_revision", OLD_REVISION),
    ] {
        map.allocate(authority, Uuid::parse_str(old).expect("fixture UUID"), None)
            .expect("fixture allocation");
    }
    map
}

#[test]
fn menu_ticket_pattern_is_configuration_but_concrete_link_inputs_stay_strict() {
    let map = fixture_map();
    let registry = registry();
    let template = "/micro/work-orders/tickets/:id";
    let mut row = json!({"path":template,"link_url":null});
    let original = row.clone();
    transform_row("platform", "menus", &mut row, &map, &registry).unwrap();
    assert_eq!(row, original);
    for (table, field, route) in [
        ("notifications", "link", template),
        ("menus", "link_url", template),
        ("menus", "path", "/micro/work-orders/tickets/:id?record=1"),
        ("menus", "path", "/micro/work-orders/tickets/new"),
        ("menus", "path", "/micro/work-orders/tickets/:other"),
    ] {
        let mut invalid = json!({field:route});
        let before = invalid.clone();
        assert!(transform_row("platform", table, &mut invalid, &map, &registry).is_err());
        assert_eq!(invalid, before);
    }
    let mut missing_owner = registry.clone();
    missing_owner["routeReferences"]
        .as_array_mut()
        .unwrap()
        .retain(|entry| entry["id"] != "other-links");
    assert!(transform_row("platform", "menus", &mut row, &map, &missing_owner).is_err());
}

#[test]
fn strict_json_rejects_duplicates_nested_in_arrays_and_preserves_precise_numbers() {
    for raw in [
        r#"{"x":1,"x":2}"#,
        r#"{"x":[{"a":1,"a":2}]}"#,
        r#"{"x":1,"\u0078":2}"#,
        "{}{}",
    ] {
        assert!(parse_strict(raw).is_err());
    }
    let raw = r#"{"amount":900719925474099312345678901234567890,"decimal":0.12345678901234567890123456789,"exponent":1.2300e100}"#;
    let parsed = parse_strict(raw).expect("precise JSON");
    assert_eq!(
        parsed["amount"].to_string(),
        "900719925474099312345678901234567890"
    );
    assert_eq!(
        parsed["decimal"].to_string(),
        "0.12345678901234567890123456789"
    );
    assert_eq!(parsed["exponent"].to_string(), "1.2300e+100");
}

#[test]
fn publication_remaps_only_owner_fields_and_recomputes_owner_canonical_hash() {
    let map = fixture_map();
    let body = json!({"target":{"kind":"USER","userId":OLD_USER,"organizationId":OLD_ORG},"type":"INFO","title":"含旧 ID 的自由文本","content":OLD_USER,"link":format!("/micro/work-orders/tickets/{OLD_TICKET}")});
    let mut row = json!({"request_hash":sha256(encode(&body).expect("body").as_bytes()),"request_body":body,"idempotency_key":"opaque-event-1"});
    transform_row(
        "platform",
        "notification_publications",
        &mut row,
        &map,
        &registry(),
    )
    .expect("publication conversion");
    assert_eq!(row["request_body"]["content"], OLD_USER);
    assert!(!row["request_body"]
        .as_object()
        .expect("body")
        .contains_key("idempotencyKey"));
    assert_eq!(row["idempotency_key"], "opaque-event-1");
    let target = map
        .resolve("platform.user", Uuid::parse_str(OLD_USER).expect("source"))
        .expect("mapping");
    assert_eq!(row["request_body"]["target"]["userId"], target.to_string());
    assert_eq!(
        row["request_hash"],
        sha256(
            encode(&row["request_body"])
                .expect("mapped body")
                .as_bytes()
        )
    );
}

#[test]
fn publication_discriminators_and_source_hash_fail_atomically() {
    let map = fixture_map();
    for target in [
        json!({"kind":"TENANT","userId":OLD_USER}),
        json!({"kind":"USER","userId":OLD_USER}),
        json!({"kind":"FUTURE"}),
    ] {
        let body = json!({"target":target,"type":"INFO","title":"x","content":"","link":""});
        let mut row = json!({"request_body":body,"request_hash":sha256(encode(&body).expect("body").as_bytes()),"idempotency_key":"event-1"});
        let original = row.clone();
        assert!(transform_row(
            "platform",
            "notification_publications",
            &mut row,
            &map,
            &registry()
        )
        .is_err());
        assert_eq!(row, original);
    }
    let body = json!({"target":{"kind":"TENANT"},"type":"INFO","title":"x","content":"","link":""});
    let mut row =
        json!({"request_body":body,"request_hash":"a".repeat(64),"idempotency_key":"event-1"});
    let original = row.clone();
    assert!(transform_row(
        "platform",
        "notification_publications",
        &mut row,
        &map,
        &registry()
    )
    .is_err());
    assert_eq!(row, original);
}

fn reminder_row() -> Value {
    let payload = Reminder {
        idempotency_key: "event-1".into(),
        target: ReminderTarget {
            kind: "USER".into(),
            user_id: OLD_USER.into(),
            organization_id: OLD_ORG.into(),
        },
        notification_type: "INFO".into(),
        title: "工单提醒 WO-000001".into(),
        content: "请查看工单 WO-000001 的详情并处理。".into(),
        link: format!("/micro/work-orders/tickets/{OLD_TICKET}"),
    };
    let raw = encode(&payload).expect("owner payload");
    json!({"request_hash":sha256(raw.as_bytes()),"request_json":raw,"idempotency_key":"event-1","recipient_user_id":OLD_USER,"target_organization_id":OLD_ORG,"ticket_id":OLD_TICKET})
}

#[test]
fn reminder_hash_binds_exact_owner_struct_order_and_mapped_row_identity() {
    let map = fixture_map();
    let mut row = reminder_row();
    for (field, authority, source) in [
        ("recipient_user_id", "platform.user", OLD_USER),
        ("target_organization_id", "platform.organization", OLD_ORG),
        ("ticket_id", "work_orders.ticket", OLD_TICKET),
    ] {
        row[field] = Value::String(
            map.resolve(authority, Uuid::parse_str(source).expect("source"))
                .expect("mapping")
                .to_string(),
        );
    }
    transform_row(
        "work_orders",
        "work_order_reminder_intents",
        &mut row,
        &map,
        &registry(),
    )
    .expect("reminder conversion");
    let raw = row["request_json"].as_str().expect("text");
    assert!(raw
        .starts_with("{\"idempotencyKey\":\"event-1\",\"target\":{\"kind\":\"USER\",\"userId\":"));
    assert_eq!(row["request_hash"], sha256(raw.as_bytes()));
    let payload = parse_strict(raw).expect("mapped payload");
    assert_eq!(payload["target"]["userId"], row["recipient_user_id"]);
    assert_eq!(
        payload["link"],
        format!(
            "/micro/work-orders/tickets/{}",
            row["ticket_id"].as_str().expect("ticket")
        )
    );
}

#[test]
fn reminder_noncanonical_bytes_and_scope_mismatch_do_not_mutate() {
    let map = fixture_map();
    let mut reordered = reminder_row();
    let body = parse_strict(reordered["request_json"].as_str().expect("text")).expect("payload");
    let raw = encode(&body).expect("sorted Value differs from owner order");
    reordered["request_json"] = json!(raw);
    reordered["request_hash"] = json!(sha256(raw.as_bytes()));
    let mut wrong_scope = reminder_row();
    wrong_scope["recipient_user_id"] = json!(OLD_ORG);
    for mut row in [reordered, wrong_scope] {
        let original = row.clone();
        assert!(transform_row(
            "work_orders",
            "work_order_reminder_intents",
            &mut row,
            &map,
            &registry()
        )
        .is_err());
        assert_eq!(row, original);
    }
}

#[test]
fn exact_routes_use_separate_authorities_and_unknown_encoded_references_block() {
    let map = fixture_map();
    let registry = registry();
    let ticket = map
        .resolve(
            "work_orders.ticket",
            Uuid::parse_str(OLD_TICKET).expect("source"),
        )
        .expect("mapping");
    assert_eq!(
        rewrite_route(
            &format!("/micro/work-orders/tickets/{OLD_TICKET}?view=detail"),
            &map,
            &registry
        )
        .expect("route"),
        format!("/micro/work-orders/tickets/{ticket}?view=detail")
    );
    let revision = map
        .resolve(
            "platform.application_url_revision",
            Uuid::parse_str(OLD_REVISION).expect("source"),
        )
        .expect("mapping");
    assert_eq!(
        rewrite_route(
            &format!("/micro-proxy/work-orders/{OLD_REVISION}/assets/index.js"),
            &map,
            &registry
        )
        .expect("proxy"),
        format!("/micro-proxy/work-orders/{revision}/assets/index.js")
    );
    for url in [
        format!("/unknown/{OLD_USER}"),
        format!("/unknown/prefix{OLD_USER}suffix"),
        format!("/unknown/{}", OLD_USER.replace('-', "%2d")),
        format!("/micro/work-orders/tickets/{OLD_TICKET}?owner={OLD_USER}"),
        format!("https://unreviewed.example/assets/{OLD_USER}.png"),
    ] {
        assert!(rewrite_route(&url, &map, &registry).is_err());
    }
    let mut external = registry.clone();
    external["opaqueExternalUrlOrigins"] = json!(["https://owner.example"]);
    let url = format!("https://owner.example/assets/{OLD_USER}.png");
    assert_eq!(
        rewrite_route(&url, &map, &external).expect("attested external owner"),
        url
    );
}

fn region_detail(action: &str, id: &str) -> Value {
    let body = json!({"id":id,"version":"2026-custom","code":"110000","parentCode":"0","name":"测试","namePinyin":"ceshi","nameShort":"cs","level":2,"type":null,"typeName":null,"postCode":null,"sourceUrl":null});
    json!({"objectType":"region","objectId":id,"before":if action=="CREATE_REGION"{Value::Null}else{body.clone()},"after":if action=="DELETE_REGION"{Value::Null}else{body}})
}

#[test]
fn compact_region_source_needs_independent_create_and_live_row_evidence() {
    let mut map = fixture_map();
    let registry = registry();
    let compact = Uuid::parse_str(OLD_USER)
        .expect("source")
        .simple()
        .to_string();
    let mut collector = SourceCollector::new();
    collector
        .observe(
            "platform",
            "regions",
            &json!({"source_id":compact,"version":"a"}),
            &"a".repeat(64),
        )
        .expect("observe row");
    assert!(collector.finish(&mut map, &registry).is_err());
    let detail = region_detail("CREATE_REGION", &compact);
    collector
        .observe(
            "platform",
            "audit_logs",
            &json!({"action":"CREATE_REGION","detail":encode(&detail).expect("detail")}),
            &"b".repeat(64),
        )
        .expect("observe owner");
    collector
        .finish(&mut map, &registry)
        .expect("proven ownership");
    let mut first = json!({"source_id":compact,"version":"a"});
    let mut copied = json!({"source_id":compact,"version":"b"});
    transform_row("platform", "regions", &mut first, &map, &registry).expect("manual source");
    transform_row("platform", "regions", &mut copied, &map, &registry).expect("copied source");
    assert_eq!(first["source_id"], copied["source_id"]);
    let target = Uuid::parse_str(first["source_id"].as_str().expect("compact")).expect("target");
    assert_eq!(target.get_version_num(), 7);
    assert_eq!(first["source_id"].as_str().expect("compact").len(), 32);
    let mut imported = json!({"source_id":"1100000000000000001"});
    transform_row("platform", "regions", &mut imported, &map, &registry).expect("external source");
    assert_eq!(imported["source_id"], "1100000000000000001");
}

#[test]
fn deleted_manual_source_is_only_a_historical_tombstone() {
    let mut map = fixture_map();
    let registry = registry();
    let compact = Uuid::parse_str(OLD_TICKET)
        .expect("source")
        .simple()
        .to_string();
    let mut collector = SourceCollector::new();
    for action in ["CREATE_REGION", "DELETE_REGION"] {
        let detail = region_detail(action, &compact);
        collector
            .observe(
                "platform",
                "audit_logs",
                &json!({"action":action,"detail":encode(&detail).expect("detail")}),
                &"c".repeat(64),
            )
            .expect("owner event");
    }
    collector
        .finish(&mut map, &registry)
        .expect("deletion proof");
    assert!(map
        .resolve(
            "platform.manual_region_source",
            Uuid::parse_str(&compact).expect("source")
        )
        .is_err());
    assert!(map
        .resolve_historical(
            "platform.manual_region_source",
            Uuid::parse_str(&compact).expect("source")
        )
        .is_ok());
}

#[test]
fn region_version_uuid_shaped_business_strings_remain_opaque() {
    let map = fixture_map();
    let detail = json!({"objectType":"regionVersion","objectId":OLD_USER,"before":null,"after":{"version":OLD_USER,"label":"custom","sourceVersion":OLD_ORG}});
    let mut row =
        json!({"action":"CREATE_REGION_VERSION","detail":encode(&detail).expect("detail")});
    transform_row("platform", "audit_logs", &mut row, &map, &registry())
        .expect("opaque version owner");
    assert_eq!(
        parse_strict(row["detail"].as_str().expect("detail")).expect("projection"),
        detail
    );
}

#[test]
fn unregistered_audit_and_duplicate_text_fields_fail_without_replacement() {
    let map = fixture_map();
    let mut login = json!({"action":"LOGIN","detail":format!("用户名自由文本 {OLD_USER}")});
    let original = login.clone();
    transform_row("platform", "audit_logs", &mut login, &map, &registry())
        .expect("known text producer");
    assert_eq!(login, original);
    for mut row in [
        json!({"action":"UNKNOWN","detail":"legacy"}),
        json!({"action":"UPDATE_USER","detail":"{\"objectType\":\"user\",\"objectType\":\"role\"}"}),
    ] {
        let original = row.clone();
        assert!(transform_row("platform", "audit_logs", &mut row, &map, &registry()).is_err());
        assert_eq!(row, original);
    }
}

#[test]
fn application_setup_uses_registered_non_envelope_schema_and_preserves_plain_text_history() {
    let authorities = [
        "platform.tenant",
        "platform.organization",
        "platform.role",
        "platform.menu",
    ];
    let mut map = TypedMap::new(
        1_700_000_000_000,
        authorities.into_iter().map(str::to_owned).collect(),
    )
    .expect("policy");
    for (authority, raw) in [
        ("platform.tenant", OLD_USER),
        ("platform.organization", OLD_ORG),
        ("platform.role", OLD_TICKET),
        ("platform.menu", OLD_TICKET),
    ] {
        map.allocate(authority, Uuid::parse_str(raw).expect("source"), None)
            .expect("allocation");
    }
    let detail = json!({"target":{"tenantId":OLD_USER,"organizationId":OLD_ORG,"roleId":OLD_TICKET},"menuIds":[OLD_TICKET],"added":4,"mode":"ADDITIVE"});
    let mut row =
        json!({"action":"APPLICATION_SETUP_ACCESS","detail":encode(&detail).expect("source")});
    transform_row("platform", "audit_logs", &mut row, &map, &registry()).expect("setup projection");
    let projected = parse_strict(row["detail"].as_str().expect("text")).expect("projection");
    assert_ne!(projected["target"]["roleId"], projected["menuIds"][0]);
    let mut legacy =
        json!({"action":"DELETE_NOTIFICATION","detail":format!("删除通知: 用户输入 {OLD_USER}")});
    let original = legacy.clone();
    transform_row("platform", "audit_logs", &mut legacy, &map, &registry())
        .expect("source-owned legacy text");
    assert_eq!(legacy, original);
    let mut invalid = json!({"action":"APPLICATION_SETUP_ACCESS","detail":encode(&json!({"target":detail["target"],"menuIds":[OLD_TICKET],"added":4,"mode":"REPLACE"})).expect("body")});
    let original = invalid.clone();
    assert!(transform_row("platform", "audit_logs", &mut invalid, &map, &registry()).is_err());
    assert_eq!(invalid, original);
}

#[test]
fn create_receipt_without_original_request_blocks_even_with_complete_response_snapshot() {
    let response = json!({"id":OLD_TICKET,"number":"WO-000001","title":"initial","description":"","priority":"NORMAL","status":"OPEN","tenantId":OLD_USER,"organizationId":OLD_ORG,"organizationName":"fixture","ownerUserId":OLD_USER,"ownerDisplayName":"fixture","assigneeUserId":null,"assigneeDisplayName":null,"version":1,"createdAt":"2026-01-01T00:00:00.000Z","updatedAt":"2026-01-01T00:00:00.000Z"});
    let mut row =
        json!({"response_json":encode(&response).expect("source"),"request_hash":"a".repeat(64)});
    let original = row.clone();
    let error = transform_row(
        "work_orders",
        "work_order_create_idempotency",
        &mut row,
        &fixture_map(),
        &registry(),
    )
    .expect_err("missing original canonical Create DTO");
    assert_eq!(
        error.to_string(),
        "original work-order create request unavailable"
    );
    assert_eq!(row, original);
}
