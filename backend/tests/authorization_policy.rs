use saas_platform_backend::{
    authorization_policy::{
        application_binding, member_organization_allowed, MemberScope, OrganizationCeiling,
    },
    paging::Paging,
    snapshot::{consistent, Revision, Snapshot, SnapshotError},
};
use uuid::Uuid;

// Stable, valid v7 fixture identities; no runtime generation policy is exercised.
fn fixture_uuid(value: u128) -> Uuid {
    Uuid::from_u128(0x019a_1163_0000_7000_8000_0000_0000_0000 | value)
}

#[test]
fn paging_has_stable_defaults_and_capped_checked_offsets() {
    assert_eq!(Paging::from_raw(None, None).parts(), (1, 10, 0));
    assert_eq!(
        Paging::from_raw(Some("3"), Some("200")).parts(),
        (3, 200, 400)
    );
    assert_eq!(
        Paging::from_raw(Some("9223372036854775807"), Some("200")).parts(),
        (1_000_000, 200, 199_999_800)
    );
    for page in ["-1", "0", "bad", "18446744073709551615"] {
        assert_eq!(
            Paging::from_raw(Some(page), Some("201")).parts(),
            (1, 10, 0)
        );
    }
}

#[test]
fn client_and_token_bindings_are_both_required() {
    let a = fixture_uuid(1);
    let b = fixture_uuid(2);
    assert!(application_binding(a, a, "stores", "stores"));
    assert!(!application_binding(a, b, "stores", "stores"));
    assert!(!application_binding(a, a, "work-orders", "stores"));
}

#[test]
fn member_grants_never_expand_a_separate_ceiling_or_self_owner() {
    let org = fixture_uuid(1);
    let child = fixture_uuid(2);
    let peer = fixture_uuid(3);
    let ids = [org, child];
    assert!(member_organization_allowed(
        MemberScope::OrgSubtree,
        &ids,
        OrganizationCeiling::Finite(&ids),
        org,
        child
    ));
    for scope in [
        MemberScope::Tenant,
        MemberScope::OrgSubtree,
        MemberScope::SelfScope,
    ] {
        let grant = if scope == MemberScope::Tenant {
            &[][..]
        } else {
            &[org, child, peer][..]
        };
        assert!(!member_organization_allowed(
            scope,
            grant,
            OrganizationCeiling::Finite(&ids),
            org,
            peer
        ));
    }
    assert!(!member_organization_allowed(
        MemberScope::SelfScope,
        &ids,
        OrganizationCeiling::Tenant,
        org,
        child
    ));
    assert!(!member_organization_allowed(
        MemberScope::Tenant,
        &[],
        OrganizationCeiling::Current,
        org,
        child
    ));
}

#[tokio::test]
async fn fresh_revision_rebuilds_stale_snapshot_and_bounds_churn() {
    let mut generation = 0;
    let result = consistent(
        || {
            generation += 1;
            std::future::ready(Ok::<_, &str>(Snapshot {
                value: generation,
                revision: Revision([generation; 32]),
            }))
        },
        || std::future::ready(Ok(Revision([2; 32]))),
    )
    .await;
    assert_eq!(result, Ok(2));
    let result = consistent(
        || {
            std::future::ready(Ok::<_, &str>(Snapshot {
                value: (),
                revision: Revision([1; 32]),
            }))
        },
        || std::future::ready(Ok(Revision([2; 32]))),
    )
    .await;
    assert_eq!(result, Err(SnapshotError::Changed));
    let result = consistent(
        || {
            std::future::ready(Ok::<_, &str>(Snapshot {
                value: (),
                revision: Revision([1; 32]),
            }))
        },
        || std::future::ready(Err("revoked")),
    )
    .await;
    assert_eq!(result, Err(SnapshotError::Store("revoked")));
}
