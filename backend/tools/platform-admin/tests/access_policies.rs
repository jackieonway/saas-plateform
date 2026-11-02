use saas_platform_admin::run;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

const ID: &str = "11111111-1111-7111-8111-111111111111";
static COUNTER: AtomicU64 = AtomicU64::new(0);

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}
fn manifest(value: &Value) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "platform-policy-{}-{}.json",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&path, value.to_string()).unwrap();
    path
}
fn desired() -> Value {
    json!({"target":"application","id":ID,"allowedMemberTypes":["EMPLOYEE"],"accessPolicyVersion":2,"reason":" 按已批准的应用用途收紧员工准入 "})
}
fn policy(types: Value, version: i64) -> Value {
    json!({"id":ID,"allowedMemberTypes":types,"accessPolicyVersion":version,"status":"ENABLED","clientSecret":"REMOTE_SECRET_CANARY"})
}

fn server(
    responses: Vec<(&'static str, &'static str, u16, Value)>,
) -> (BTreeMap<String, String>, thread::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let handle = thread::spawn(move || {
        let mut bodies = Vec::new();
        for (method, path, status, value) in responses {
            let deadline = Instant::now() + Duration::from_secs(3);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("policy fixture did not receive request: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = Vec::new();
            let (header_end, content_len) = loop {
                let mut chunk = [0; 8192];
                let size = stream.read(&mut chunk).unwrap();
                assert!(size > 0);
                bytes.extend_from_slice(&chunk[..size]);
                if let Some(end) = bytes.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]);
                    let len = header
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    break (end + 4, len);
                }
            };
            while bytes.len() < header_end + content_len {
                let mut chunk = [0; 8192];
                let size = stream.read(&mut chunk).unwrap();
                assert!(size > 0);
                bytes.extend_from_slice(&chunk[..size]);
            }
            let header = String::from_utf8_lossy(&bytes[..header_end]);
            assert!(header.starts_with(&format!("{method} {path} HTTP/1.1\r\n")));
            assert!(!String::from_utf8_lossy(&bytes).contains("SECRET_CANARY"));
            bodies.push(if content_len == 0 {
                Value::Null
            } else {
                serde_json::from_slice(&bytes[header_end..header_end + content_len]).unwrap()
            });
            let body = value.to_string();
            write!(stream,"HTTP/1.1 {status} Result\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",body.len()).unwrap();
        }
        bodies
    });
    (
        BTreeMap::from([
            ("PLATFORM_API_ORIGIN".into(), origin),
            ("PLATFORM_ADMIN_BEARER".into(), "test-bearer".into()),
        ]),
        handle,
    )
}
fn apply(
    path: &std::path::Path,
    env: &BTreeMap<String, String>,
) -> Result<Value, saas_platform_admin::SafeError> {
    run(
        args(&[
            "access-policy-save",
            "--manifest",
            path.to_str().unwrap(),
            "--apply",
            "--environment",
            "development",
            "--allow-loopback-http",
        ]),
        env,
    )
}

#[test]
fn preview_is_offline_and_invalid_changes_fail_before_credentials_or_network() {
    let path = manifest(&desired());
    let env = BTreeMap::from([("PLATFORM_API_ORIGIN".into(), "invalid-secret-origin".into())]);
    let output = run(
        args(&["access-policy-save", "--manifest", path.to_str().unwrap()]),
        &env,
    )
    .unwrap();
    assert_eq!(output["externalWrites"], false);
    assert_eq!(output["desired"]["accessPolicyVersion"], 2);
    assert!(!output.to_string().contains("按已批准"));
    let invalid = [
        ("allowedMemberTypes", json!([])),
        ("allowedMemberTypes", json!(["EMPLOYEE", "EMPLOYEE"])),
        ("allowedMemberTypes", json!(["ADMIN"])),
        ("accessPolicyVersion", json!(0)),
        ("reason", json!("   ")),
        ("reason", json!("reason\ncontrol")),
        ("reason", json!("字".repeat(501))),
        ("id", json!("11111111-1111-4111-8111-111111111111")),
        ("subjectMode", json!("USER")),
        ("subjectMode", Value::Null),
        ("clientSecret", json!("SECRET_CANARY")),
    ];
    for (field, value) in invalid {
        let mut input = desired();
        input[field] = value;
        fs::write(&path, input.to_string()).unwrap();
        let error = apply(&path, &env).unwrap_err();
        assert_eq!(error.category(), "INVALID_ACCESS_POLICY_MANIFEST");
        assert!(!error.to_string().contains("CANARY"));
    }
    for mode in [None, Some("LEGACY_MIXED"), Some("MACHINE")] {
        let mut input = desired();
        input["target"] = json!("client");
        if let Some(mode) = mode {
            input["subjectMode"] = json!(mode);
        }
        fs::write(&path, input.to_string()).unwrap();
        assert_eq!(
            apply(&path, &env).unwrap_err().category(),
            "INVALID_ACCESS_POLICY_MANIFEST"
        );
    }
    fs::write(
        &path,
        format!("{{\"target\":\"application\",\"target\":\"client\",\"id\":\"{ID}\"}}"),
    )
    .unwrap();
    assert_eq!(
        apply(&path, &env).unwrap_err().category(),
        "INVALID_ACCESS_POLICY_MANIFEST"
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn save_uses_explicit_old_version_and_only_projects_verified_policy() {
    let path = manifest(&desired());
    let endpoint = "/api/applications/11111111-1111-7111-8111-111111111111/access-policy";
    let (env, handle) = server(vec![
        (
            "GET",
            endpoint,
            200,
            policy(json!(["CUSTOMER", "EMPLOYEE"]), 2),
        ),
        ("PUT", endpoint, 200, policy(json!(["EMPLOYEE"]), 3)),
    ]);
    let result = apply(&path, &env).unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(result["policy"]["accessPolicyVersion"], 3);
    assert!(!result.to_string().contains("CANARY"));
    let bodies = handle.join().unwrap();
    assert_eq!(
        bodies[1],
        json!({"allowedMemberTypes":["EMPLOYEE"],"accessPolicyVersion":2,"reason":"按已批准的应用用途收紧员工准入"})
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn stale_preview_and_concurrent_change_never_retry_or_upgrade_the_expected_version() {
    let path = manifest(&desired());
    let endpoint = "/api/applications/11111111-1111-7111-8111-111111111111/access-policy";
    let (env, handle) = server(vec![("GET", endpoint, 200, policy(json!(["EMPLOYEE"]), 3))]);
    let error = apply(&path, &env).unwrap_err();
    assert_eq!(error.category(), "ACCESS_POLICY_CONFLICT");
    assert_eq!(error.status(), Some(409));
    assert!(!error.outcome_unknown());
    assert_eq!(handle.join().unwrap().len(), 1);
    let (env, handle) = server(vec![
        (
            "GET",
            endpoint,
            200,
            policy(json!(["CUSTOMER", "EMPLOYEE"]), 2),
        ),
        (
            "PUT",
            endpoint,
            409,
            json!({"error":"DO_NOT_ECHO_SECRET_CANARY"}),
        ),
    ]);
    let error = apply(&path, &env).unwrap_err();
    assert_eq!(error.category(), "ACCESS_POLICY_CONFLICT");
    assert!(!error.outcome_unknown());
    assert_eq!(handle.join().unwrap().len(), 2);
    fs::remove_file(path).unwrap();
}

#[test]
fn application_no_op_keeps_the_version() {
    let path = manifest(&desired());
    let current = policy(json!(["EMPLOYEE"]), 2);
    let endpoint = "/api/applications/11111111-1111-7111-8111-111111111111/access-policy";
    let (env, handle) = server(vec![
        ("GET", endpoint, 200, current.clone()),
        ("PUT", endpoint, 200, current),
    ]);
    let result = apply(&path, &env).unwrap();
    assert_eq!(result["changed"], false);
    assert_eq!(result["policy"]["accessPolicyVersion"], 2);
    let bodies = handle.join().unwrap();
    assert!(bodies[1].get("subjectMode").is_none());
    assert_eq!(bodies[1]["allowedMemberTypes"], json!(["EMPLOYEE"]));
    fs::remove_file(path).unwrap();
}

#[test]
fn mismatched_success_requires_reconciliation_without_retrying_the_write() {
    let path = manifest(&desired());
    let endpoint = "/api/applications/11111111-1111-7111-8111-111111111111/access-policy";
    let (env, handle) = server(vec![
        (
            "GET",
            endpoint,
            200,
            policy(json!(["CUSTOMER", "EMPLOYEE"]), 2),
        ),
        ("PUT", endpoint, 200, policy(json!(["CUSTOMER"]), 3)),
    ]);
    let error = apply(&path, &env).unwrap_err();
    assert_eq!(error.category(), "ACCESS_POLICY_RESULT_CONFLICT");
    assert!(error.outcome_unknown());
    assert_eq!(handle.join().unwrap().len(), 2);
    fs::remove_file(path).unwrap();
}

#[test]
fn read_is_get_only_and_cannot_apply_or_mix_application_selection() {
    for values in [
        vec![
            "access-policy-read",
            "--policy-target",
            "application",
            "--policy-id",
            ID,
            "--apply",
        ],
        vec![
            "access-policy-read",
            "--policy-target",
            "application",
            "--policy-id",
            ID,
            "--app-code",
            "stores",
        ],
    ] {
        assert_eq!(
            run(args(&values), &BTreeMap::new()).unwrap_err().category(),
            "INVALID_ARGUMENT"
        );
    }
    let endpoint = "/api/applications/11111111-1111-7111-8111-111111111111/access-policy";
    let (env, handle) = server(vec![("GET", endpoint, 200, policy(json!(["EMPLOYEE"]), 2))]);
    let result = run(
        args(&[
            "access-policy-read",
            "--policy-target",
            "application",
            "--policy-id",
            ID,
            "--environment",
            "development",
            "--allow-loopback-http",
        ]),
        &env,
    )
    .unwrap();
    assert_eq!(result["mode"], "READ_ONLY");
    assert!(result["policy"].get("subjectMode").is_none());
    assert!(!result.to_string().contains("CANARY"));
    assert_eq!(handle.join().unwrap(), vec![Value::Null]);
}
