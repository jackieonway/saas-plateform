use saas_platform_admin::run;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    release: PathBuf,
    artifact: PathBuf,
    inventory: PathBuf,
}
impl Fixture {
    fn new(app: &str, environment: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "platform-artifact-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self {
            release: root.join("release.json"),
            artifact: root.join("artifact.json"),
            inventory: root.join("inventory"),
            root,
        };
        fixture.write_release(app, environment, "v1");
        fixture.write_artifact(app, "https://platform.example.com", 'a');
        fixture
    }
    fn write_release(&self, app: &str, environment: &str, version: &str) {
        write_json(
            &self.release,
            &json!({"environment":environment,"version":version,"entryType":"HTML","entryUrl":format!("/sub-apps/{app}/versions/{version}/index.html"),"healthUrl":null,"resources":[{"url":"./app.js","kind":"SCRIPT"},{"url":"./style.css","kind":"STYLE"}]}),
        );
    }
    fn write_artifact(&self, app: &str, origin: &str, marker: char) {
        let hex = marker.to_string().repeat(64);
        write_json(
            &self.artifact,
            &json!({"schemaVersion":1,"applicationCode":app,"publicOrigin":origin,"source":{"kind":"OCI_IMAGE","identity":format!("registry.example.com/team/{app}@sha256:{hex}"),"targetPlatform":"linux/amd64"},"digest":format!("sha256:{hex}"),"commit":"a".repeat(40),"buildId":"build-v1","inputsSha256":"a".repeat(64)}),
        );
    }
    fn args(&self, command: &str, app: &str, environment: &str, apply: bool) -> Vec<String> {
        let mut args = vec![
            command.into(),
            "--app-code".into(),
            app.into(),
            "--environment".into(),
            environment.into(),
            "--release-file".into(),
            self.release.to_str().unwrap().into(),
            "--artifact-file".into(),
            self.artifact.to_str().unwrap().into(),
            "--inventory-dir".into(),
            self.inventory.to_str().unwrap().into(),
        ];
        if environment == "development" {
            args.push("--allow-loopback-http".into());
        }
        if apply {
            args.push("--apply".into());
        }
        args
    }
    fn bind(&self, app: &str, environment: &str) -> Value {
        run(
            self.args("artifact-bind", app, environment, true),
            &BTreeMap::new(),
        )
        .unwrap()
    }
    fn verify(&self, app: &str, environment: &str) -> Value {
        run(
            self.args("artifact-verify", app, environment, false),
            &BTreeMap::new(),
        )
        .unwrap()
    }
    fn final_path(&self, sequence: u64) -> PathBuf {
        self.inventory.join(format!("{sequence:020}.json"))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}
fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn category(args: Vec<String>) -> String {
    run(args, &BTreeMap::new()).unwrap_err().category().into()
}

#[test]
fn both_applications_bind_offline_idempotently_and_never_echo_candidate_or_secrets() {
    for app in ["work-orders", "stores"] {
        let fixture = Fixture::new(app, "production");
        let env = BTreeMap::from([
            ("PLATFORM_API_ORIGIN".into(), "INVALID_CANARY".into()),
            ("PLATFORM_ADMIN_BEARER".into(), "BEARER_CANARY".into()),
            ("PLATFORM_CLIENT_SECRET".into(), "SECRET_CANARY".into()),
        ]);
        let plan = run(
            fixture.args("artifact-bind", app, "production", false),
            &env,
        )
        .unwrap();
        assert_eq!(plan["wouldAppend"], true);
        assert_eq!(plan["localWrites"], false);
        assert!(!fixture.inventory.exists());
        assert_eq!(
            category(fixture.args("artifact-verify", app, "production", false)),
            "ARTIFACT_STORE_MISSING"
        );
        let first = run(fixture.args("artifact-bind", app, "production", true), &env).unwrap();
        assert_eq!(first["sequence"], 1);
        assert_eq!(first["appended"], true);
        let bytes = fs::read(fixture.final_path(1)).unwrap();
        assert_eq!(fixture.bind(app, "production")["appended"], false);
        let verified = fixture.verify(app, "production");
        assert_eq!(verified["mode"], "VERIFIED");
        assert_eq!(verified["remoteDigestStored"], false);
        assert_eq!(fs::read(fixture.final_path(1)).unwrap(), bytes);
        let output = format!("{plan}{first}{verified}");
        for private in [
            "CANARY",
            "registry.example.com",
            "platform.example.com",
            fixture.root.to_str().unwrap(),
        ] {
            assert!(!output.contains(private));
        }
        let record = read_json(&fixture.final_path(1));
        assert_eq!(
            record["release"]["entryUrl"],
            format!("https://platform.example.com/sub-apps/{app}/versions/v1/index.html")
        );
        assert!(record["release"].get("healthUrl").is_none());
    }
}

#[test]
fn cross_environment_origin_can_change_but_artifact_and_existing_url_are_immutable() {
    let fixture = Fixture::new("stores", "production");
    fixture.bind("stores", "production");
    let original = fs::read(fixture.final_path(1)).unwrap();
    fixture.write_release("stores", "staging", "v1");
    fixture.write_artifact("stores", "https://staging.example.com", 'a');
    assert_eq!(fixture.bind("stores", "staging")["sequence"], 2);
    fixture.write_release("stores", "qa", "v1");
    fixture.write_artifact("stores", "https://platform.example.com", 'a');
    assert_eq!(fixture.bind("stores", "qa")["sequence"], 3);
    for field in ["digest", "source", "commit", "buildId", "inputsSha256"] {
        fixture.write_artifact("stores", "https://another.example.com", 'a');
        let mut artifact = read_json(&fixture.artifact);
        match field {
            "digest" => {
                artifact["digest"] = json!(format!("sha256:{}", "b".repeat(64)));
                artifact["source"]["identity"] = json!(format!(
                    "registry.example.com/team/stores@sha256:{}",
                    "b".repeat(64)
                ));
            }
            "source" => {
                artifact["source"]["identity"] = json!(format!(
                    "other.example.com/team/stores@sha256:{}",
                    "a".repeat(64)
                ))
            }
            "commit" => artifact["commit"] = json!("b".repeat(40)),
            "buildId" => artifact["buildId"] = json!("other-build"),
            _ => artifact["inputsSha256"] = json!("b".repeat(64)),
        }
        write_json(&fixture.artifact, &artifact);
        assert_eq!(
            category(fixture.args("artifact-bind", "stores", "qa", true)),
            "ARTIFACT_CONFLICT"
        );
        assert_eq!(
            category(fixture.args("register", "stores", "qa", true)),
            "ARTIFACT_CONFLICT"
        );
    }
    fixture.write_artifact("stores", "https://platform.example.com", 'a');
    let mut release = read_json(&fixture.release);
    release["healthUrl"] = json!("/different-health");
    write_json(&fixture.release, &release);
    assert_eq!(
        category(fixture.args("artifact-bind", "stores", "qa", false)),
        "ARTIFACT_CONFLICT"
    );
    assert_eq!(fs::read(fixture.final_path(1)).unwrap(), original);
    assert!(!fixture.final_path(4).exists());
}

#[test]
fn source_kinds_and_target_architecture_cannot_change_an_existing_version() {
    let fixture = Fixture::new("work-orders", "production");
    fixture.bind("work-orders", "production");
    let mut artifact = read_json(&fixture.artifact);
    artifact["source"]["targetPlatform"] = json!("linux/arm64");
    write_json(&fixture.artifact, &artifact);
    assert_eq!(
        category(fixture.args("artifact-bind", "work-orders", "production", true)),
        "ARTIFACT_CONFLICT"
    );
    artifact["source"] = json!({"kind":"STATIC_ARTIFACT","identity":"https://artifacts.example.com/immutable/v1.json"});
    write_json(&fixture.artifact, &artifact);
    assert_eq!(
        category(fixture.args("artifact-bind", "work-orders", "production", true)),
        "ARTIFACT_CONFLICT"
    );
    fixture.write_release("work-orders", "production", "static-v1");
    assert_eq!(fixture.bind("work-orders", "production")["sequence"], 2);
}

#[test]
fn strict_candidate_schema_size_and_argument_boundaries_fail_before_network() {
    let fixture = Fixture::new("work-orders", "production");
    let good = read_json(&fixture.artifact);
    for (field, value) in [
        ("unknown", json!("BEARER_CANARY")),
        ("applicationCode", json!("stores")),
        (
            "publicOrigin",
            json!("https://user:SECRET_CANARY@platform.example.com"),
        ),
        ("publicOrigin", json!("https://platform.example.com/")),
        ("commit", json!("bad")),
        ("digest", json!(format!("sha256:{}", "b".repeat(64)))),
        ("buildId", json!("../private")),
    ] {
        let mut artifact = good.clone();
        artifact[field] = value;
        write_json(&fixture.artifact, &artifact);
        let error = run(
            fixture.args("artifact-bind", "work-orders", "production", true),
            &BTreeMap::new(),
        )
        .unwrap_err();
        assert_eq!(error.category(), "INVALID_ARTIFACT");
        assert!(!error.to_json().to_string().contains("CANARY"));
        assert!(!fixture.inventory.exists());
    }
    fs::write(
        &fixture.artifact,
        format!(
            "{{\"schemaVersion\":1,\"schemaVersion\":1,\"padding\":\"{}\"}}",
            "x".repeat(1024 * 1024)
        ),
    )
    .unwrap();
    assert_eq!(
        category(fixture.args("artifact-bind", "work-orders", "production", true)),
        "INVALID_ARTIFACT"
    );
    fs::write(
        &fixture.artifact,
        "{\"schemaVersion\":1,\"schemaVersion\":1}",
    )
    .unwrap();
    assert_eq!(
        category(fixture.args("artifact-bind", "work-orders", "production", true)),
        "INVALID_ARTIFACT"
    );
    write_json(&fixture.artifact, &good);
    for command in ["artifact-verify", "release-reconcile"] {
        assert_eq!(
            category(fixture.args(command, "work-orders", "production", true)),
            "INVALID_ARGUMENT"
        );
    }
    let mut args = fixture.args("artifact-bind", "work-orders", "production", true);
    args.extend([
        "--app-id".into(),
        "11111111-1111-7111-8111-111111111111".into(),
    ]);
    assert_eq!(category(args), "INVALID_ARGUMENT");
    let mut args = fixture.args("artifact-bind", "work-orders", "production", true);
    let pos = args.iter().position(|v| v == "--inventory-dir").unwrap();
    args[pos + 1] = "relative-inventory".into();
    assert_eq!(category(args), "INVALID_ARTIFACT_PATH");
    assert!(!fixture.inventory.exists());
}

#[test]
fn corrupt_committed_history_gaps_unknown_files_and_oversized_pending_fail_closed() {
    for damage in [
        "corrupt",
        "gap",
        "unknown",
        "pending-size",
        "record-schema",
        "record-conflict",
    ] {
        let fixture = Fixture::new("work-orders", "production");
        fixture.bind("work-orders", "production");
        let original = fs::read(fixture.final_path(1)).unwrap();
        match damage {
            "corrupt" => fs::write(fixture.final_path(1), "{broken").unwrap(),
            "gap" => fs::rename(fixture.final_path(1), fixture.final_path(2)).unwrap(),
            "unknown" => fs::write(fixture.inventory.join("foreign.json"), "{}").unwrap(),
            "pending-size" => fs::write(
                fixture.inventory.join(".pending-1-2"),
                vec![b'x'; 1024 * 1024 + 1],
            )
            .unwrap(),
            "record-schema" => {
                let mut record = read_json(&fixture.final_path(1));
                record["extra"] = json!(true);
                write_json(&fixture.final_path(1), &record);
            }
            _ => {
                let mut record = read_json(&fixture.final_path(1));
                record["sequence"] = json!(2);
                record["artifact"]["buildId"] = json!("conflicting-build");
                write_json(&fixture.final_path(2), &record);
            }
        }
        assert_eq!(
            category(fixture.args("artifact-bind", "work-orders", "production", true)),
            "INVALID_ARTIFACT_STORE"
        );
        if !matches!(damage, "corrupt" | "gap" | "record-schema") {
            assert_eq!(fs::read(fixture.final_path(1)).unwrap(), original);
        }
    }
}

#[test]
fn ignored_pending_is_never_committed_or_removed_by_another_invocation_and_lock_is_bounded() {
    let fixture = Fixture::new("work-orders", "production");
    fixture.bind("work-orders", "production");
    let pending = fixture.inventory.join(".pending-123-456-0");
    fs::write(&pending, "{incomplete").unwrap();
    fixture.verify("work-orders", "production");
    assert_eq!(fs::read_to_string(&pending).unwrap(), "{incomplete");
    let lock = File::options()
        .read(true)
        .write(true)
        .open(fixture.inventory.join(".lock"))
        .unwrap();
    lock.try_lock().unwrap();
    assert_eq!(
        category(fixture.args("artifact-verify", "work-orders", "production", false)),
        "ARTIFACT_STORE_BUSY"
    );
    drop(lock);
    fixture.verify("work-orders", "production");
}

#[cfg(unix)]
#[test]
fn symlink_inputs_ancestors_inventory_lock_and_journal_are_refused() {
    use std::os::unix::fs::symlink;
    for target in [
        "input",
        "ancestor",
        "inventory",
        "lock",
        "journal",
        "pending",
    ] {
        let fixture = Fixture::new("work-orders", "production");
        if target == "input" {
            fs::rename(&fixture.artifact, fixture.root.join("real.json")).unwrap();
            symlink(fixture.root.join("real.json"), &fixture.artifact).unwrap();
        } else if target == "ancestor" {
            let real = fixture.root.join("real");
            fs::create_dir(&real).unwrap();
            symlink(&real, fixture.root.join("linked")).unwrap();
            let mut args = fixture.args("artifact-bind", "work-orders", "production", true);
            let i = args.iter().position(|v| v == "--inventory-dir").unwrap();
            args[i + 1] = fixture
                .root
                .join("linked/inventory")
                .to_str()
                .unwrap()
                .into();
            assert_eq!(category(args), "INVALID_ARTIFACT_PATH");
            continue;
        } else if target == "inventory" {
            let real = fixture.root.join("real");
            fs::create_dir(&real).unwrap();
            symlink(real, &fixture.inventory).unwrap();
        } else {
            fixture.bind("work-orders", "production");
            let path = match target {
                "lock" => fixture.inventory.join(".lock"),
                "journal" => fixture.final_path(1),
                _ => fixture.inventory.join(".pending-1-2"),
            };
            if path.exists() {
                fs::rename(&path, fixture.root.join("real.json")).unwrap();
            } else {
                fs::write(fixture.root.join("real.json"), "{}").unwrap();
            }
            symlink(fixture.root.join("real.json"), path).unwrap();
        }
        let error = category(fixture.args("artifact-bind", "work-orders", "production", true));
        assert!(matches!(
            error.as_str(),
            "INVALID_ARTIFACT_PATH" | "INVALID_ARTIFACT_STORE"
        ));
    }
}

#[test]
fn concurrent_cli_processes_commit_one_winner_and_conflicts_preserve_its_bytes() {
    for different in [false, true] {
        let fixture = Fixture::new("work-orders", "production");
        let other = fixture.root.join("other.json");
        let mut changed = read_json(&fixture.artifact);
        if different {
            changed["digest"] = json!(format!("sha256:{}", "b".repeat(64)));
            changed["source"]["identity"] = json!(format!(
                "registry.example.com/team/work-orders@sha256:{}",
                "b".repeat(64)
            ));
        }
        write_json(&other, &changed);
        let args_a = fixture.args("artifact-bind", "work-orders", "production", true);
        let mut args_b = args_a.clone();
        let i = args_b.iter().position(|v| v == "--artifact-file").unwrap();
        args_b[i + 1] = other.to_str().unwrap().into();
        let a = Command::new(env!("CARGO_BIN_EXE_saas-platform-admin"))
            .args(&args_a)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let b = Command::new(env!("CARGO_BIN_EXE_saas-platform-admin"))
            .args(&args_b)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let outputs = [a.wait_with_output().unwrap(), b.wait_with_output().unwrap()];
        assert!(outputs.iter().any(|v| v.status.success()));
        let winner = fs::read(fixture.final_path(1)).unwrap();
        for args in [args_a, args_b] {
            let result = run(args, &BTreeMap::new());
            if different {
                assert!(result.is_ok() || result.unwrap_err().category() == "ARTIFACT_CONFLICT");
            } else {
                assert_eq!(result.unwrap()["appended"], false);
            }
            assert_eq!(fs::read(fixture.final_path(1)).unwrap(), winner);
        }
        assert!(!fixture.final_path(2).exists());
    }
}

#[test]
fn interrupted_writer_worker() {
    let Ok(raw) = std::env::var("ARTIFACT_TEST_WORKER") else {
        return;
    };
    let data: Value = serde_json::from_str(&raw).unwrap();
    let root = PathBuf::from(data["inventory"].as_str().unwrap());
    if data["phase"] == "after" {
        let args: Vec<String> = serde_json::from_value(data["args"].clone()).unwrap();
        run(args, &BTreeMap::new()).unwrap();
    }
    let lock = File::options()
        .read(true)
        .write(true)
        .open(root.join(".lock"))
        .unwrap();
    lock.try_lock().unwrap();
    if data["phase"] == "before" {
        let mut pending = File::create(root.join(".pending-987-654-0")).unwrap();
        pending.write_all(b"{interrupted").unwrap();
        pending.sync_all().unwrap();
    }
    println!("WORKER_READY");
    std::io::stdout().flush().unwrap();
    let mut byte = [0];
    let _ = std::io::stdin().read(&mut byte);
}

#[test]
fn killed_writer_releases_os_lock_and_committed_binding_survives_lost_ack() {
    for phase in ["before", "after"] {
        let fixture = Fixture::new("work-orders", "production");
        fixture.bind("work-orders", "production");
        let original = fs::read(fixture.final_path(1)).unwrap();
        fixture.write_release("work-orders", "production", "v2");
        let worker = json!({"inventory":fixture.inventory,"phase":phase,"args":fixture.args("artifact-bind", "work-orders", "production", true)});
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "interrupted_writer_worker", "--nocapture"])
            .env("ARTIFACT_TEST_WORKER", worker.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut reader = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            if line.contains("WORKER_READY") {
                break;
            }
        }
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(fs::read(fixture.final_path(1)).unwrap(), original);
        if phase == "before" {
            assert!(!fixture.final_path(2).exists());
            assert_eq!(
                category(fixture.args("artifact-verify", "work-orders", "production", false)),
                "ARTIFACT_BINDING_MISSING"
            );
            assert!(fixture.inventory.join(".pending-987-654-0").exists());
        } else {
            assert_eq!(fixture.verify("work-orders", "production")["sequence"], 2);
            assert_eq!(fixture.bind("work-orders", "production")["appended"], false);
        }
    }
}

fn platform_fixture(
    release: Value,
    register: bool,
) -> (String, thread::JoinHandle<Vec<(String, Value)>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let mut seen = Vec::new();
        for _ in 0..if register { 3 } else { 2 } {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse::<usize>().unwrap();
                }
            }
            let mut bytes = vec![0; length];
            reader.read_exact(&mut bytes).unwrap();
            let body = if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap()
            };
            let response = if first.contains("GET /api/applications ") {
                json!([{"id":"11111111-1111-7111-8111-111111111111","code":"work-orders","entryType":"HTML","runtimeMode":"TRUSTED","basePath":"/micro/work-orders","status":"ENABLED","allowedMemberTypes":["EMPLOYEE"],"accessPolicyVersion":1}])
            } else if first.starts_with("POST ") {
                let mut row = body.clone();
                row["id"] = json!("22222222-2222-7222-8222-222222222222");
                row["applicationId"] = json!("11111111-1111-7111-8111-111111111111");
                row["validationStatus"] = json!("DRAFT");
                if let Some(overrides) = release.as_object() {
                    for (key, value) in overrides {
                        row[key] = value.clone();
                    }
                }
                row
            } else if register {
                json!([])
            } else {
                release.clone()
            };
            let encoded = response.to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",encoded.len(),encoded).unwrap();
            seen.push((first, body));
        }
        seen
    });
    (origin, handle)
}
fn remote_env(origin: String) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("PLATFORM_API_ORIGIN".into(), origin),
        ("PLATFORM_ADMIN_BEARER".into(), "BEARER_CANARY".into()),
    ])
}

fn unpaired_args(fixture: &Fixture) -> Vec<String> {
    let mut args = fixture.args("release-reconcile", "work-orders", "development", false);
    for flag in ["--artifact-file", "--inventory-dir"] {
        let index = args.iter().position(|value| value == flag).unwrap();
        args.drain(index..index + 2);
    }
    args
}

#[test]
fn reconcile_requires_proven_origin_before_http_and_cannot_match_ambiguous_health() {
    let fixture = Fixture::new("work-orders", "development");
    assert_eq!(
        category(unpaired_args(&fixture)),
        "RECONCILIATION_ORIGIN_REQUIRED"
    );
    let mut expected = read_json(&fixture.release);
    expected["entryUrl"] =
        json!("https://static.example.com/sub-apps/work-orders/versions/v1/index.html");
    expected["healthUrl"] = json!("/healthz");
    write_json(&fixture.release, &expected);
    // The API origin, entry origin and server MICRO_GATEWAY_ORIGIN can all
    // differ. Treating this root health as static.example.com would produce a
    // false MATCHED against a semantically different remote definition.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let error = run(
        unpaired_args(&fixture),
        &remote_env(format!("http://{}", listener.local_addr().unwrap())),
    )
    .unwrap_err();
    assert_eq!(error.category(), "RECONCILIATION_ORIGIN_REQUIRED");
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert!(!fixture.inventory.exists());
    expected["healthUrl"] = json!("https://gateway.example.com/healthz");
    write_json(&fixture.release, &expected);
    let mut row = expected;
    row["id"] = json!("22222222-2222-7222-8222-222222222222");
    row["applicationId"] = json!("11111111-1111-7111-8111-111111111111");
    row["validationStatus"] = json!("DRAFT");
    // Resources are relative to the proven absolute entry, so their origin is
    // unambiguous and remains valid when the management API has another origin.
    row["resources"] = json!([{"url":"https://static.example.com/sub-apps/work-orders/versions/v1/app.js","kind":"SCRIPT"},{"url":"https://static.example.com/sub-apps/work-orders/versions/v1/style.css","kind":"STYLE"}]);
    let (origin, handle) = platform_fixture(json!([row.clone()]), false);
    let output = run(unpaired_args(&fixture), &remote_env(origin)).unwrap();
    assert_eq!(output["reconciliation"], "MATCHED");
    assert!(
        handle
            .join()
            .unwrap()
            .iter()
            .all(|(request, _)| request.starts_with("GET "))
    );
    row["healthUrl"] = json!("/healthz");
    let (origin, handle) = platform_fixture(json!([row]), false);
    assert_eq!(
        run(unpaired_args(&fixture), &remote_env(origin))
            .unwrap_err()
            .category(),
        "RECONCILIATION_ORIGIN_REQUIRED"
    );
    assert!(
        handle
            .join()
            .unwrap()
            .iter()
            .all(|(request, _)| request.starts_with("GET "))
    );
    fixture.write_artifact("work-orders", "https://static.example.com", 'a');
    fixture.bind("work-orders", "development");
    let mut ambiguous_remote = read_json(&fixture.release);
    ambiguous_remote["id"] = json!("22222222-2222-7222-8222-222222222222");
    ambiguous_remote["applicationId"] = json!("11111111-1111-7111-8111-111111111111");
    ambiguous_remote["validationStatus"] = json!("DRAFT");
    ambiguous_remote["entryUrl"] = json!("/sub-apps/work-orders/versions/v1/index.html");
    ambiguous_remote["healthUrl"] = json!("/healthz");
    let (origin, handle) = platform_fixture(json!([ambiguous_remote]), false);
    assert_eq!(
        run(
            fixture.args("release-reconcile", "work-orders", "development", false),
            &remote_env(origin)
        )
        .unwrap_err()
        .category(),
        "RECONCILIATION_ORIGIN_REQUIRED"
    );
    assert!(
        handle
            .join()
            .unwrap()
            .iter()
            .all(|(request, _)| request.starts_with("GET "))
    );
}

#[test]
fn raw_and_canonical_url_limits_are_checked_before_a_version_is_reserved() {
    let fixture = Fixture::new("work-orders", "production");
    let original = read_json(&fixture.release);
    let prefix = "https://platform.example.com/sub-apps/work-orders/versions/v1/";
    let exact = format!("{prefix}{}.html", "a".repeat(2000 - prefix.len() - 5));
    assert_eq!(exact.chars().count(), 2000);
    let mut release = original.clone();
    release["entryUrl"] = json!(exact);
    write_json(&fixture.release, &release);
    assert_eq!(
        run(
            fixture.args("artifact-bind", "work-orders", "production", false),
            &BTreeMap::new()
        )
        .unwrap()["wouldAppend"],
        true
    );
    assert!(!fixture.inventory.exists());
    for field in ["entry", "resource", "health", "canonical", "unicode"] {
        let mut release = original.clone();
        match field {
            "entry" => {
                release["entryUrl"] = json!(format!(
                    "{prefix}{}.html",
                    "a".repeat(2001 - prefix.len() - 5)
                ))
            }
            "resource" => {
                release["resources"][0]["url"] = json!(format!(
                    "{prefix}{}.js",
                    "a".repeat(2001 - prefix.len() - 3)
                ))
            }
            "health" => {
                release["healthUrl"] = json!(format!(
                    "https://health.example.com/{}",
                    "a".repeat(2001 - "https://health.example.com/".len())
                ))
            }
            "canonical" => {
                let path = format!(
                    "/sub-apps/work-orders/versions/v1/{}.html",
                    "a".repeat(1955)
                );
                assert!(path.chars().count() <= 2000);
                release["entryUrl"] = json!(path);
            }
            _ => release["resources"][0]["url"] = json!(format!("./{}.js", "门".repeat(300))),
        }
        write_json(&fixture.release, &release);
        assert_eq!(
            category(fixture.args("artifact-bind", "work-orders", "production", true)),
            "INVALID_RELEASE",
            "{field}"
        );
        assert!(
            !fixture.inventory.exists(),
            "invalid {field} must not reserve the version"
        );
    }
}

#[test]
fn guarded_register_sends_only_existing_fields_and_reconcile_compares_complete_definition() {
    let fixture = Fixture::new("work-orders", "development");
    fixture.bind("work-orders", "development");
    let (origin, handle) = platform_fixture(json!([]), true);
    let output = run(
        fixture.args("register", "work-orders", "development", true),
        &remote_env(origin),
    )
    .unwrap();
    assert_eq!(output["artifactBindingVerified"], true);
    assert_eq!(output["remoteDigestStored"], false);
    let requests = handle.join().unwrap();
    assert_eq!(requests.len(), 3);
    let body = &requests[2].1;
    let (origin, handle) = platform_fixture(
        json!({"entryUrl":"/sub-apps/work-orders/versions/v1/index.html"}),
        true,
    );
    let error = run(
        fixture.args("register", "work-orders", "development", true),
        &remote_env(origin),
    )
    .unwrap_err();
    assert_eq!(error.category(), "RECONCILIATION_ORIGIN_REQUIRED");
    assert!(error.outcome_unknown());
    assert_eq!(error.to_json()["reconciliationRequired"], true);
    assert_eq!(handle.join().unwrap().len(), 3);
    let mut keys: Vec<_> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "entryType",
            "entryUrl",
            "environment",
            "resources",
            "version"
        ]
    );
    let mut row = body.clone();
    row["id"] = json!("22222222-2222-7222-8222-222222222222");
    row["applicationId"] = json!("11111111-1111-7111-8111-111111111111");
    row["validationStatus"] = json!("DRAFT");
    for scenario in ["match", "absent", "entry", "resource", "health"] {
        let mut actual = row.clone();
        match scenario {
            "entry" => {
                actual["entryUrl"] = json!(
                    "https://platform.example.com/sub-apps/work-orders/versions/v1/other.html"
                )
            }
            "resource" => {
                actual["resources"][0]["url"] = json!(
                    "https://platform.example.com/sub-apps/work-orders/versions/v1/changed.js"
                )
            }
            "health" => {
                actual["healthUrl"] = json!("https://platform.example.com/different-health")
            }
            _ => {}
        }
        let (origin, handle) = platform_fixture(
            if scenario == "absent" {
                json!([])
            } else {
                json!([actual])
            },
            false,
        );
        let result = run(
            fixture.args("release-reconcile", "work-orders", "development", false),
            &remote_env(origin),
        );
        if scenario == "match" {
            let value = result.unwrap();
            assert_eq!(value["reconciliation"], "MATCHED");
            assert_eq!(value["remoteDigestStored"], false);
        } else if scenario == "absent" {
            assert_eq!(result.unwrap()["reconciliation"], "ABSENT");
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.category(), "RELEASE_DEFINITION_CONFLICT");
            assert!(!error.outcome_unknown());
            assert_eq!(error.to_json()["progress"]["reconciliation"], "CONFLICT");
        }
        assert!(
            handle
                .join()
                .unwrap()
                .iter()
                .all(|(request, _)| request.starts_with("GET "))
        );
    }
}
