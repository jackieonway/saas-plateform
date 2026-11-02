use std::process::Command;

#[test]
fn cli_restore_refuses_without_configuration_or_native_tools() {
    for config in [
        "relative-private-canary.json",
        "",
        "/",
        "/missing-canonical-recovery/config.json",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_saas-recovery"))
            .args(["restore", config])
            .env_clear()
            .env("PATH", "")
            .output()
            .expect("the built recovery CLI must execute without external tools");
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            "recovery failed: CANONICAL_IDENTITY_RECOVERY_REQUIRED\nCanonical identity recovery: backend/ops/identity-recovery/README.md\n"
        );
    }
}
