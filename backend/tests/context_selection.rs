use chrono::{Duration, TimeZone, Utc};
use saas_platform_backend::context_selection::{
    ApplicationBinding, Binding, CodeExtension, Credential, Id, Intent, Member, MemberType,
    Registration, Selection, SelectionError, Source, Version,
};
use serde_json::json;
use uuid::Uuid;

const ISSUER: &str = "https://auth.saas.test";

fn id(n: u128) -> Id {
    Id::new(Uuid::from_u128(
        0x019a_1163_0000_7000_8000_0000_0000_0000 | n,
    ))
    .unwrap()
}

fn version(n: i64) -> Version {
    Version::new(n).unwrap()
}

fn now() -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_790_000_000, 0).single().unwrap()
}

fn binding() -> Binding {
    Binding {
        transaction_id: id(1),
        source: Source {
            issuer: ISSUER.to_owned(),
            user_id: id(2),
            account_version: version(0),
            global_session_id: id(3),
            interaction_id: id(4),
            auth_time: now() - Duration::seconds(30),
        },
        registration: Registration {
            client_id: id(5),
            client_policy_version: version(1),
            member_type: MemberType::Employee,
            redirect_uri: "https://platform.saas.test/auth/callback".to_owned(),
            application: Some(ApplicationBinding {
                id: id(6),
                policy_version: version(1),
            }),
        },
        intent: Intent::Switch {
            session_id: id(7),
            expected_version: version(1),
        },
    }
}

fn member() -> Member {
    Member {
        user_id: id(2),
        tenant_id: id(8),
        organization_id: id(9),
        membership_id: id(10),
        member_type: MemberType::Employee,
        identity_version: version(0),
    }
}

fn selection() -> Selection {
    Selection::confirm(
        binding(),
        member(),
        ISSUER,
        now(),
        now() + Duration::minutes(10),
        now() + Duration::minutes(5),
    )
    .unwrap()
}

#[test]
fn rejects_noncanonical_identity_and_numeric_or_repaired_versions() {
    let canonical = id(1).uuid().to_string();
    assert_eq!(
        serde_json::from_value::<Id>(json!(canonical)).unwrap(),
        id(1)
    );
    for raw in [
        canonical.to_uppercase(),
        canonical.replace('-', ""),
        Uuid::nil().to_string(),
    ] {
        assert!(serde_json::from_value::<Id>(json!(raw)).is_err());
    }
    for raw in [
        json!(0),
        json!("00"),
        json!("+1"),
        json!("-1"),
        json!(" 1"),
        json!("9223372036854775808"),
    ] {
        assert!(serde_json::from_value::<Version>(raw).is_err());
    }
    assert_eq!(serde_json::to_value(version(0)).unwrap(), json!("0"));
    assert!(Version::new(-1).is_err());
}

#[test]
fn wire_rejects_unknown_and_mixed_intent_fields() {
    let mut wire = serde_json::to_value(binding()).unwrap();
    assert_eq!(
        serde_json::from_value::<Binding>(wire.clone()).unwrap(),
        binding()
    );
    wire["source"]["membershipId"] = json!(id(10));
    assert!(serde_json::from_value::<Binding>(wire).is_err());
    for intent in [
        json!({"kind":"LOGIN","sessionId":id(7),"expectedVersion":"1"}),
        json!({"kind":"SWITCH","sessionId":id(7)}),
        json!({"kind":"SWITCH","sessionId":id(7),"expectedVersion":"1","tenantId":id(8)}),
    ] {
        assert!(serde_json::from_value::<Intent>(intent).is_err());
    }
    assert!(serde_json::from_str::<Intent>(r#"{"kind":"LOGIN","kind":"SWITCH"}"#).is_err());
    let duplicated = format!(
        r#"{{"kind":"SWITCH","sessionId":"{}","expectedVersion":"1","expectedVersion":"2"}}"#,
        id(7).uuid()
    );
    assert!(serde_json::from_str::<Intent>(&duplicated).is_err());
}

#[test]
fn validates_issuer_callback_generations_and_original_auth_time() {
    assert_eq!(binding().validate(ISSUER, now()), Ok(()));
    let mut candidate = binding();
    candidate.source.issuer = "https://other.saas.test".to_owned();
    assert_eq!(
        candidate.validate(ISSUER, now()),
        Err(SelectionError::Invalid)
    );
    for callback in [
        "http://platform.saas.test/auth/callback",
        "https://u:p@platform.saas.test/cb",
        "https://platform.saas.test/cb#secret",
        "//platform.saas.test/cb",
    ] {
        let mut candidate = binding();
        candidate.registration.redirect_uri = callback.to_owned();
        assert_eq!(
            candidate.validate(ISSUER, now()),
            Err(SelectionError::Invalid)
        );
    }
    for mutator in [
        |v: &mut Binding| v.source.auth_time = now() + Duration::seconds(1),
        |v: &mut Binding| v.registration.client_policy_version = version(0),
        |v: &mut Binding| v.registration.application.as_mut().unwrap().policy_version = version(0),
        |v: &mut Binding| {
            v.intent = Intent::Switch {
                session_id: id(7),
                expected_version: version(0),
            }
        },
    ] {
        let mut candidate = binding();
        mutator(&mut candidate);
        assert_eq!(
            candidate.validate(ISSUER, now()),
            Err(SelectionError::Invalid)
        );
    }
}

#[test]
fn confirmation_rejects_another_account_and_customer_for_employee_rp() {
    for mutate in [
        |m: &mut Member| m.user_id = id(11),
        |m: &mut Member| m.member_type = MemberType::Customer,
    ] {
        let mut candidate = member();
        mutate(&mut candidate);
        assert_eq!(
            Selection::confirm(
                binding(),
                candidate,
                ISSUER,
                now(),
                now() + Duration::minutes(5),
                now() + Duration::minutes(5)
            ),
            Err(SelectionError::Changed)
        );
    }
    let mut customer_binding = binding();
    customer_binding.registration.member_type = MemberType::Customer;
    customer_binding.intent = Intent::Login {};
    let mut customer = member();
    customer.member_type = MemberType::Customer;
    assert!(Selection::confirm(
        customer_binding,
        customer,
        ISSUER,
        now(),
        now() + Duration::minutes(5),
        now() + Duration::minutes(5)
    )
    .is_ok());
}

#[test]
fn original_deadlines_cap_credential_lifetime_and_are_not_repaired() {
    assert_eq!(selection().expires_at, now() + Duration::seconds(120));
    for (source, interaction, seconds) in [(10, 60, 10), (60, 10, 10), (120, 120, 120)] {
        let selected = Selection::confirm(
            binding(),
            member(),
            ISSUER,
            now(),
            now() + Duration::seconds(source),
            now() + Duration::seconds(interaction),
        )
        .unwrap();
        assert_eq!(selected.expires_at, now() + Duration::seconds(seconds));
    }
    for deadline in [now(), now() - Duration::seconds(1)] {
        assert_eq!(
            Selection::confirm(
                binding(),
                member(),
                ISSUER,
                now(),
                deadline,
                now() + Duration::minutes(5)
            ),
            Err(SelectionError::Expired)
        );
        assert_eq!(
            Selection::confirm(
                binding(),
                member(),
                ISSUER,
                now(),
                now() + Duration::minutes(5),
                deadline
            ),
            Err(SelectionError::Expired)
        );
    }
}

#[test]
fn every_original_binding_is_required_at_consumption() {
    let selected = selection();
    assert_eq!(
        selected.check_consumption(&binding(), &member(), now(), false),
        Ok(())
    );
    let mutators: &[fn(&mut Binding)] = &[
        |v| v.transaction_id = id(11),
        |v| v.source.issuer = "https://other.saas.test".to_owned(),
        |v| v.source.user_id = id(11),
        |v| v.source.account_version = version(1),
        |v| v.source.global_session_id = id(11),
        |v| v.source.interaction_id = id(11),
        |v| v.source.auth_time -= Duration::seconds(1),
        |v| v.registration.client_id = id(11),
        |v| v.registration.client_policy_version = version(2),
        |v| v.registration.member_type = MemberType::Customer,
        |v| v.registration.redirect_uri = "https://platform.saas.test/another".to_owned(),
        |v| v.registration.application.as_mut().unwrap().id = id(11),
        |v| v.registration.application.as_mut().unwrap().policy_version = version(2),
        |v| v.registration.application = None,
        |v| v.intent = Intent::Login {},
        |v| {
            v.intent = Intent::Switch {
                session_id: id(11),
                expected_version: version(1),
            }
        },
        |v| {
            v.intent = Intent::Switch {
                session_id: id(7),
                expected_version: version(2),
            }
        },
    ];
    for mutate in mutators {
        let mut candidate = binding();
        mutate(&mut candidate);
        assert_eq!(
            selected.check_consumption(&candidate, &member(), now(), false),
            Err(SelectionError::Changed)
        );
    }
    let mutators: &[fn(&mut Member)] = &[
        |v| v.user_id = id(11),
        |v| v.tenant_id = id(11),
        |v| v.organization_id = id(11),
        |v| v.membership_id = id(11),
        |v| v.member_type = MemberType::Customer,
        |v| v.identity_version = version(1),
    ];
    for mutate in mutators {
        let mut candidate = member();
        mutate(&mut candidate);
        assert_eq!(
            selected.check_consumption(&binding(), &candidate, now(), false),
            Err(SelectionError::Changed)
        );
    }
}

#[test]
fn replay_and_deadline_boundary_cannot_issue_another_context() {
    let selected = selection();
    assert_eq!(
        selected.check_consumption(&binding(), &member(), now(), true),
        Err(SelectionError::Consumed)
    );
    assert_eq!(
        selected.check_consumption(&binding(), &member(), selected.expires_at, false),
        Err(SelectionError::Expired)
    );
    assert_eq!(
        selected.check_consumption(&binding(), &member(), now() - Duration::seconds(1), false),
        Err(SelectionError::Expired)
    );
    assert_eq!(
        selected.check_consumption(
            &binding(),
            &member(),
            selected.expires_at - Duration::nanoseconds(1),
            false
        ),
        Ok(())
    );
}

#[test]
fn code_extension_is_versioned_and_only_carries_a_canonical_opaque_credential() {
    let wire = json!({
        "contractVersion":"context-selection/1", "transactionId":id(1),
        "credential":"abcdef0123456789".repeat(4)
    });
    let decoded: CodeExtension = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
    for (key, value) in [
        ("contractVersion", json!("context-selection/2")),
        ("credential", json!("A".repeat(64))),
        ("credential", json!("a".repeat(63))),
        ("membershipId", json!(id(10))),
    ] {
        let mut candidate = wire.clone();
        candidate[key] = value;
        assert!(serde_json::from_value::<CodeExtension>(candidate).is_err());
    }
    let credential = Credential::parse("a".repeat(64)).unwrap();
    assert_eq!(
        credential.digest(),
        "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb"
    );
}

#[test]
fn recovery_does_not_extend_deadline_or_accept_another_operation() {
    let selected = selection();
    assert_eq!(
        selected.check_recovery(&binding(), &member(), now(), true),
        Ok(())
    );
    assert_eq!(
        selected.check_recovery(&binding(), &member(), now(), false),
        Err(SelectionError::Changed)
    );
    assert_eq!(
        selected.check_recovery(&binding(), &member(), selected.expires_at, true),
        Err(SelectionError::Expired)
    );
    let mut another = binding();
    another.intent = Intent::Switch {
        session_id: id(7),
        expected_version: version(2),
    };
    assert_eq!(
        selected.check_recovery(&another, &member(), now(), true),
        Err(SelectionError::Changed)
    );
}
