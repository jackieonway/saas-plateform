use super::*;

#[test]
fn rollout_arguments_are_explicit_and_never_fall_through_to_a_server() {
    let args = |values: &[&str]| values.iter().map(OsString::from).collect::<Vec<_>>();
    assert!(arguments(args(&[
        "inspect",
        "--platform-binary",
        "platform.exe",
        "--environment",
        "development"
    ]))
    .is_ok());
    assert!(arguments(args(&[
        "apply",
        "--manifest",
        "manifest.json",
        "--platform-binary",
        "platform.exe"
    ]))
    .is_ok());
    for values in [
        vec![],
        vec!["--help"],
        vec!["--seed"],
        vec!["apply"],
        vec!["inspect", "--platform-binary", "platform.exe"],
        vec![
            "apply",
            "--manifest",
            "manifest.json",
            "--platform-binary",
            "platform.exe",
            "--environment",
            "production",
        ],
        vec![
            "inspect",
            "--platform-binary",
            "platform.exe",
            "--environment",
            "test",
        ],
        vec![
            "inspect",
            "--platform-binary",
            "one",
            "--platform-binary",
            "two",
            "--environment",
            "production",
        ],
    ] {
        assert!(
            arguments(args(&values)).is_err(),
            "ambiguous invocation accepted"
        );
    }
}

#[test]
fn receipt_contract_is_canonical_environment_bound_and_sanitized() {
    let id = "019a1163-0000-7000-8000-000000000001";
    let hash = "a".repeat(64);
    assert!(MembershipReadiness::new(id, &hash, "development").is_ok());
    assert!(MembershipReadiness::new(id, &hash, "production").is_ok());
    for invalid in [
        "private-canary",
        "019a1163-0000-4000-8000-000000000001",
        "019a1163-0000-7000-c000-000000000001",
    ] {
        let error = MembershipReadiness::new(invalid, &hash, "production")
            .err()
            .unwrap();
        assert!(!format!("{error:?}").contains("private-canary"));
    }
    assert!(MembershipReadiness::new(id, &hash.to_uppercase(), "production").is_err());
    assert!(MembershipReadiness::new(id, &hash, "test").is_err());
    let error = caused(Failure::Connection, anyhow::anyhow!("private-canary"));
    assert!(!format!("{error}").contains("private-canary"));
    assert!(!format!("{error:?}").contains("private-canary"));
    assert!(error.source().is_some());
}

#[test]
fn contraction_is_independent_restrict_only_and_never_a_sqlx_migration() {
    assert!(CONTRACTION_SQL
        .contains("DROP CONSTRAINT memberships_user_id_organization_id_key RESTRICT"));
    assert!(!CONTRACTION_SQL.contains("CASCADE"));
    assert!(!super::super::MIGRATIONS.iter().any(|m| m
        .sql
        .contains("DROP CONSTRAINT memberships_user_id_organization_id_key")));
    assert!(RECEIPT_SQL
        .contains("REVOKE ALL ON TABLE public.identity_membership_readiness_receipts FROM PUBLIC"));
    assert!(!RECEIPT_SQL.contains("_sqlx_migrations"));
}
