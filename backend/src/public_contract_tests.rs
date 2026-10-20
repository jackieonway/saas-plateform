//! Public wire consumers only: no database, platform authorization, or future business API.

#[path = "../../examples/micro-app-sdk/generated/protocol.rs"]
mod independent_consumer;

use crate::public_contract::{self as protocol, OptionalNullable};
use serde_json::{json, Value};

fn fixtures() -> Value {
    serde_json::from_str(include_str!(
        "../../examples/micro-app-sdk/contracts/fixtures/public-wire.json"
    ))
    .expect("valid public fixtures")
}

fn context() -> Value {
    fixtures()["context"].clone()
}

fn props() -> Value {
    let mut value = context();
    value["contractVersion"] = json!(protocol::MICRO_PROPS_VERSION);
    value["contextEpoch"] = json!(1);
    value["tokenExpiresAt"] = json!("2099-01-01T00:00:00Z");
    value["token"] = json!("fixture-application-token-not-a-credential");
    value["basePath"] = json!("/micro/orders");
    value["routePath"] = json!("/micro/orders/1");
    value["theme"] = json!("light");
    value.as_object_mut().expect("object").remove("application");
    value
}

fn bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("serializable wire fixture")
}

#[test]
fn public_contract_sources_and_vendored_rust_are_exact() {
    assert_eq!(
        include_str!("public_contract.rs"),
        include_str!("../../examples/micro-app-sdk/generated/protocol.rs")
    );
    let release: Value =
        serde_json::from_str(include_str!("../../examples/micro-app-sdk/release.json"))
            .expect("registered release");
    let schema: Value = serde_json::from_str(include_str!(
        "../../examples/micro-app-sdk/generated/schema.json"
    ))
    .expect("generated schema");
    assert_eq!(release["sdkVersion"], protocol::SDK_VERSION);
    assert_eq!(release["contracts"]["context"], protocol::CONTEXT_VERSION);
    assert_eq!(
        release["contracts"]["microProps"],
        protocol::MICRO_PROPS_VERSION
    );
    assert_eq!(release["contracts"]["iframe"], protocol::IFRAME_VERSION);
    assert_eq!(
        release["contracts"]["notification"],
        protocol::NOTIFICATION_VERSION
    );
    assert_eq!(
        schema["$defs"]["AppAuthorizationContext"]["properties"]["organizationScopeIds"]
            ["maxItems"],
        protocol::ORGANIZATION_LIMIT
    );
    assert_eq!(
        release["limits"]["maxJsonResponseBytes"],
        protocol::RESPONSE_LIMIT_BYTES
    );
}

#[test]
fn public_contract_independent_consumer_roundtrips_actual_context_and_props() {
    let original = context();
    let producer =
        protocol::AppAuthorizationContext::from_json(&bytes(&original)).expect("platform DTO");
    let consumer = independent_consumer::AppAuthorizationContext::from_json(&bytes(&original))
        .expect("independent wire consumer");
    assert_eq!(serde_json::to_value(producer).expect("wire"), original);
    assert_eq!(serde_json::to_value(consumer).expect("wire"), original);
    let original = props();
    let consumer = independent_consumer::MicroPropsWire::from_json(&bytes(&original))
        .expect("props wire consumer");
    assert_eq!(serde_json::to_value(consumer).expect("wire"), original);
}

#[test]
fn public_contract_unknown_versions_fail_without_disclosing_payloads() {
    for version in ["0.9", "2.0", "9.0", "1.0\nfixture-secret"] {
        let mut value = context();
        value["contractVersion"] = json!(version);
        let error = protocol::AppAuthorizationContext::from_json(&bytes(&value))
            .err()
            .expect("unknown context version rejected");
        assert_eq!(error.to_string(), "PUBLIC_CONTRACT_INVALID");
    }
    let mut value = props();
    value["contractVersion"] = json!("3.0");
    assert!(protocol::MicroPropsWire::from_json(&bytes(&value)).is_err());
    let mut value = props();
    value["sdkVersion"] = json!("9.0.0");
    assert!(protocol::MicroPropsWire::from_json(&bytes(&value)).is_err());
    value["sdkVersion"] = json!(protocol::SDK_VERSION);
    assert!(protocol::MicroPropsWire::from_json(&bytes(&value)).is_ok());
    for name in ["notificationSocketContext", "realtimeNotification"] {
        let mut value = fixtures()[name].clone();
        value["contractVersion"] = json!("9.0");
        if name == "notificationSocketContext" {
            assert!(serde_json::from_value::<protocol::NotificationSocketContext>(value).is_err());
        } else {
            assert!(serde_json::from_value::<protocol::RealtimeNotification>(value).is_err());
        }
    }
}

#[test]
fn public_contract_missing_null_and_bounded_organization_scopes_stay_distinct() {
    let mut old = context();
    old.as_object_mut()
        .expect("object")
        .remove("organizationScopeIds");
    let parsed =
        protocol::AppAuthorizationContext::from_json(&bytes(&old)).expect("legacy field absence");
    assert!(matches!(
        parsed.organization_scope_ids,
        OptionalNullable::Missing
    ));
    assert_eq!(
        parsed.organization_ceiling().expect("safe fallback"),
        Some(vec!["fixture-organization"])
    );
    assert_eq!(serde_json::to_value(parsed).expect("wire"), old);

    let mut root = context();
    root["organizationScopeIds"] = Value::Null;
    let parsed =
        protocol::AppAuthorizationContext::from_json(&bytes(&root)).expect("explicit root scope");
    assert!(matches!(
        parsed.organization_scope_ids,
        OptionalNullable::Null
    ));
    assert_eq!(parsed.organization_ceiling().expect("root shape"), None);
    assert_eq!(serde_json::to_value(parsed).expect("wire"), root);

    let parsed =
        protocol::AppAuthorizationContext::from_json(&bytes(&context())).expect("bounded scope");
    assert_eq!(
        parsed.organization_ceiling().expect("bounded"),
        Some(vec!["fixture-organization", "fixture-child"])
    );
}

#[test]
fn public_contract_invalid_or_over_limit_organization_scopes_fail_closed() {
    for scope in [
        json!([]),
        json!(["other-organization"]),
        json!(["fixture-organization", ""]),
        json!("unbounded"),
        json!(vec![
            "fixture-organization";
            protocol::ORGANIZATION_LIMIT + 1
        ]),
    ] {
        let mut value = context();
        value["organizationScopeIds"] = scope;
        assert!(protocol::AppAuthorizationContext::from_json(&bytes(&value)).is_err());
    }
    let mut value = context();
    value["dataGrants"][0]["organizationIds"] = json!(vec![
        "fixture-organization";
        protocol::ORGANIZATION_LIMIT + 1
    ]);
    assert!(protocol::AppAuthorizationContext::from_json(&bytes(&value)).is_err());
}

#[test]
fn public_contract_props_require_safe_epoch_ceiling_and_unchanged_application_boundary() {
    for (field, value) in [
        ("contextEpoch", json!(0)),
        ("contextEpoch", json!(-1)),
        ("contextEpoch", json!(9_007_199_254_740_992_i64)),
        ("basePath", json!("/micro/other")),
        ("token", json!("")),
        ("organizationScopeIds", json!(["other"])),
    ] {
        let mut candidate = props();
        candidate[field] = value;
        assert!(protocol::MicroPropsWire::from_json(&bytes(&candidate)).is_err());
    }
    let mut missing = props();
    missing
        .as_object_mut()
        .expect("object")
        .remove("organizationScopeIds");
    assert!(protocol::MicroPropsWire::from_json(&bytes(&missing)).is_err());
    let mut missing = context();
    missing["menus"][0]
        .as_object_mut()
        .expect("menu")
        .remove("parentId");
    assert!(protocol::AppAuthorizationContext::from_json(&bytes(&missing)).is_err());
}

#[test]
fn public_contract_notification_publish_targets_limits_and_unknown_fields_are_checked() {
    let body = fixtures()["notificationPublish"].clone();
    for target in [
        json!({"kind":"USER","userId":"fixture-user","organizationId":"fixture-organization"}),
        json!({"kind":"ORGANIZATION","organizationId":"fixture-organization"}),
        json!({"kind":"TENANT"}),
    ] {
        let mut value = body.clone();
        value["target"] = target;
        let dto =
            protocol::NotificationPublishBody::from_json(&bytes(&value)).expect("valid target");
        assert_eq!(serde_json::to_value(dto).expect("wire"), value);
    }
    for (field, value) in [
        (
            "target",
            json!({"kind":"TENANT","organizationId":"untrusted"}),
        ),
        ("target", json!({"kind":"USER","userId":"fixture-user"})),
        ("type", json!("UNKNOWN")),
        ("idempotencyKey", json!("bad key")),
        ("title", json!(" ")),
        ("title", json!("字".repeat(201))),
        ("content", json!("字".repeat(8193))),
        ("link", json!("x".repeat(301))),
        ("unknown", json!("fixture-secret")),
    ] {
        let mut candidate = body.clone();
        candidate[field] = value;
        let error = protocol::NotificationPublishBody::from_json(&bytes(&candidate))
            .err()
            .expect("invalid body");
        assert_eq!(error.to_string(), "PUBLIC_CONTRACT_INVALID");
    }
}

#[test]
fn public_contract_notification_wire_keeps_public_ack_and_publication_ownership_coherent() {
    let value = fixtures();
    let public: protocol::NotificationSocketContext =
        serde_json::from_value(value["notificationSocketContext"].clone()).expect("public ack");
    assert_eq!(
        serde_json::to_value(public).expect("wire"),
        value["notificationSocketContext"]
    );
    let mut platform = value["notificationSocketContext"].clone();
    platform["sessionId"] = json!("fixture-session-internal-only");
    let internal: protocol::NotificationPlatformSocketContext =
        serde_json::from_value(platform.clone()).expect("platform socket identity");
    assert_eq!(serde_json::to_value(internal).expect("wire"), platform);
    let public: protocol::NotificationSocketContext =
        serde_json::from_value(platform).expect("public projection");
    assert!(serde_json::to_value(public)
        .expect("wire")
        .get("sessionId")
        .is_none());
    let publication: protocol::NotificationPublication =
        serde_json::from_value(value["notificationPublication"].clone()).expect("publication");
    assert_eq!(
        serde_json::to_value(publication).expect("wire"),
        value["notificationPublication"]
    );
    for (application, publication, valid) in [
        (json!("app"), json!("publication"), true),
        (Value::Null, Value::Null, true),
        (json!("app"), Value::Null, false),
    ] {
        let mut event = value["realtimeNotification"].clone();
        event["applicationId"] = application;
        event["publicationId"] = publication;
        let dto: protocol::RealtimeNotification =
            serde_json::from_value(event).expect("event shape");
        assert_eq!(dto.validate().is_ok(), valid);
    }
    for (kind, application, publication, valid) in [
        ("APP", json!("app"), json!("publication"), true),
        ("SYSTEM", Value::Null, Value::Null, true),
        ("SYSTEM", json!("app"), json!("publication"), false),
    ] {
        let mut row = value["notificationRecord"].clone();
        row["sourceKind"] = json!(kind);
        row["applicationId"] = application;
        row["publicationId"] = publication;
        let dto: protocol::NotificationRecord = serde_json::from_value(row).expect("row shape");
        assert_eq!(dto.validate().is_ok(), valid);
    }
}

#[test]
fn public_contract_bounded_readers_return_only_fixed_credential_free_errors() {
    let huge = vec![b' '; protocol::RESPONSE_LIMIT_BYTES + 1];
    assert_eq!(
        protocol::AppAuthorizationContext::from_json(&huge)
            .err()
            .expect("response cap")
            .to_string(),
        "PUBLIC_CONTRACT_INVALID"
    );
    assert!(protocol::MicroPropsWire::from_json(&huge).is_err());
    assert!(protocol::NotificationPublishBody::from_json(&huge).is_err());
    let error = protocol::AppAuthorizationContext::from_json(b"Bearer fixture-secret-not-json")
        .err()
        .expect("malformed response");
    assert_eq!(format!("{error:?}"), "ProtocolError");
    assert_eq!(error.to_string(), "PUBLIC_CONTRACT_INVALID");
}
