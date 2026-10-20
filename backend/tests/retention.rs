use chrono::{Duration, TimeZone, Utc};
use saas_platform_backend::retention::{preview, Policy, Version};
use uuid::Uuid;

fn version(n: u128) -> Version {
    Version {
        id: Uuid::from_u128(n),
        application: Uuid::from_u128(1),
        environment: "production".into(),
        version: format!("v{n}"),
        created_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap() - Duration::days(n as i64),
        artifact_keys: vec![format!("zip:{n}")],
        active: false,
        previous: false,
        in_flight: false,
        platform_owned: true,
    }
}

#[test]
fn age_and_latest_are_a_union_and_parameters_cannot_shorten_candidates() {
    let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    let mut rows = (1..=8).map(version).collect::<Vec<_>>();
    rows[0].created_at = now - Duration::days(100);
    let result = preview(&rows, Policy::new(30, 5).unwrap(), now);
    assert!(result[0].reasons.is_empty());
    assert!(result[1..].iter().all(|r| !r.reasons.is_empty()));
    for row in &mut rows {
        row.created_at = now - Duration::days(100 + row.id.as_u128() as i64);
    }
    let result = preview(&rows, Policy::new(30, 5).unwrap(), now);
    assert_eq!(result.iter().filter(|r| r.reasons.is_empty()).count(), 3);
    assert!(Policy::new(29, 5).is_err());
    assert!(Policy::new(30, 4).is_err());
    rows[7].created_at = now - Duration::days(30);
    // A boundary can pass age but is still retained if it enters the latest five.
    let result = preview(&rows, Policy::new(30, 5).unwrap(), now);
    assert!(!result[7].reasons.contains("MINIMUM_AGE"));
    assert!(result[7].reasons.contains("LATEST_VERSIONS"));
}

#[test]
fn active_previous_pending_shared_and_external_never_become_review_candidates() {
    let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap() + Duration::days(200);
    let mut rows = (1..=12).map(version).collect::<Vec<_>>();
    rows[5].active = true;
    rows[6].previous = true;
    rows[7].in_flight = true;
    rows[8].platform_owned = false;
    rows[9].artifact_keys = rows[5].artifact_keys.clone();
    rows[9].environment = "staging".into();
    rows[10].version = rows[6].version.clone();
    rows[10].environment = "staging".into();
    let result = preview(&rows, Policy::new(30, 5).unwrap(), now);
    for (index, reason) in [
        (5, "ACTIVE"),
        (6, "PREVIOUS"),
        (7, "IN_FLIGHT_OPERATION"),
        (8, "EXTERNAL_OWNERSHIP_REQUIRES_REVIEW"),
        (9, "ACTIVE_REFERENCE"),
        (10, "PREVIOUS_REFERENCE"),
    ] {
        assert!(result[index].reasons.contains(reason), "{index}/{reason}");
    }
    assert!(result[9].reasons.contains("CROSS_ENVIRONMENT_REFERENCE"));
    // A rollback flips the pointers, preserving both versions.
    rows[5].active = false;
    rows[5].previous = true;
    rows[6].previous = false;
    rows[6].active = true;
    let result = preview(&rows, Policy::new(30, 5).unwrap(), now);
    assert!(result[5].reasons.contains("PREVIOUS"));
    assert!(result[6].reasons.contains("ACTIVE"));
}
