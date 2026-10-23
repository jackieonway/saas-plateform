use saas_platform_admin::run;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    thread,
};

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}
fn temp_json(value: &Value) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "platform-admin-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    path
}
fn release() -> Value {
    json!({"environment":"production","version":"1.2.3","entryType":"HTML","entryUrl":"/sub-apps/work-orders/versions/1.2.3/index.html","resources":[{"url":"./app.js","kind":"SCRIPT"}]})
}
#[test]
fn default_and_unapplied_commands_are_offline_even_with_invalid_secrets() {
    let env = BTreeMap::from([
        (
            "PLATFORM_API_ORIGIN".into(),
            "https://secret.invalid/path?token=CANARY".into(),
        ),
        ("PLATFORM_ADMIN_BEARER".into(), "SECRET_CANARY".into()),
        ("PLATFORM_CLIENT_SECRET".into(), "NEVER_READ".into()),
    ]);
    for values in [
        vec![],
        vec!["bootstrap"],
        vec!["rollback"],
        vec![
            "preflight",
            "--release-id",
            "11111111-1111-1111-1111-111111111111",
        ],
    ] {
        let output = run(args(&values), &env).unwrap();
        assert_eq!(output["externalWrites"], false);
        assert_eq!(output["mode"], "PLAN");
        assert!(!output.to_string().contains("CANARY"));
    }
}

#[test]
fn offline_bootstrap_plans_show_qiankun_entry_defaults_for_both_apps() {
    let invalid_environment = BTreeMap::from([
        ("PLATFORM_API_ORIGIN".into(), "not a remote origin".into()),
        ("PLATFORM_ADMIN_BEARER".into(), "SECRET_CANARY".into()),
    ]);
    for (app, entry) in [
        ("work-orders", "http://127.0.0.1:3013/index.html"),
        ("stores", "http://127.0.0.1:3014/index.html"),
    ] {
        for command in ["plan", "bootstrap"] {
            let output = run(args(&[command, "--app-code", app]), &invalid_environment).unwrap();
            assert_eq!(output["mode"], "PLAN");
            assert_eq!(output["externalWrites"], false);
            assert_eq!(output["runtimeMode"], "TRUSTED");
            assert_eq!(output["entryType"], "HTML");
            assert_eq!(output["entryUrl"], entry);
            assert_eq!(output["basePath"], format!("/micro/{app}"));
            assert_eq!(output["grants"], json!([]));
            assert!(!output.to_string().contains("CANARY"));
        }
    }
}
#[test]
fn register_is_strict_offline_and_needs_no_zip() {
    let path = temp_json(&release());
    let output = run(
        args(&["register", "--release-file", path.to_str().unwrap()]),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(output["version"], "1.2.3");
    for (key, value) in [
        ("image", json!("private")),
        ("digest", json!("sha256:private")),
        ("target", json!("private")),
    ] {
        let mut invalid = release();
        invalid[key] = value;
        fs::write(&path, invalid.to_string()).unwrap();
        assert_eq!(
            run(
                args(&["register", "--release-file", path.to_str().unwrap()]),
                &BTreeMap::new()
            )
            .unwrap_err()
            .category(),
            "INVALID_RELEASE"
        );
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn app_selection_is_closed_and_each_release_prefix_belongs_to_the_selected_app() {
    let environment = BTreeMap::new();
    assert_eq!(
        run(args(&["--app-code", "unknown-app"]), &environment)
            .unwrap_err()
            .category(),
        "INVALID_APPLICATION_CODE"
    );
    assert_eq!(
        run(
            args(&["--app-code", "stores", "--app-code", "stores"]),
            &environment
        )
        .unwrap_err()
        .category(),
        "INVALID_ARGUMENT"
    );
    let plan = run(args(&["bootstrap", "--app-code", "stores"]), &environment).unwrap();
    assert_eq!(plan["applicationCode"], "stores");
    assert_eq!(plan["permissions"].as_array().unwrap().len(), 9);
    assert_eq!(
        run(args(&[]), &environment).unwrap()["permissions"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
    let path = temp_json(&release());
    assert_eq!(
        run(
            args(&[
                "register",
                "--app-code",
                "stores",
                "--release-file",
                path.to_str().unwrap()
            ]),
            &environment
        )
        .unwrap_err()
        .category(),
        "INVALID_RELEASE"
    );
    let mut stores = release();
    stores["entryUrl"] = json!("/sub-apps/stores/versions/1.2.3/index.html");
    fs::write(&path, stores.to_string()).unwrap();
    assert!(
        run(
            args(&[
                "register",
                "--app-code",
                "stores",
                "--release-file",
                path.to_str().unwrap()
            ]),
            &environment
        )
        .is_ok()
    );
    assert_eq!(
        run(
            args(&["register", "--release-file", path.to_str().unwrap()]),
            &environment
        )
        .unwrap_err()
        .category(),
        "INVALID_RELEASE"
    );
    stores["resources"] =
        json!([{"url":"/sub-apps/work-orders/versions/1.2.3/app.js","kind":"SCRIPT"}]);
    fs::write(&path, stores.to_string()).unwrap();
    assert_eq!(
        run(
            args(&[
                "register",
                "--app-code",
                "stores",
                "--release-file",
                path.to_str().unwrap()
            ]),
            &environment
        )
        .unwrap_err()
        .category(),
        "INVALID_RELEASE"
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn grants_use_manifest_app_and_reject_explicit_selection_conflicts_before_network() {
    let mut manifest: Value = serde_json::from_str(include_str!(
        "../manifests/stores-grants.manifest.example.json"
    ))
    .unwrap();
    manifest["applicationId"] = json!("11111111-1111-4111-8111-111111111111");
    for (index, value) in manifest["menuIds"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .enumerate()
    {
        *value = json!(format!("00000000-0000-4000-8000-{:012x}", index + 10));
    }
    let path = temp_json(&manifest);
    let environment =
        BTreeMap::from([("PLATFORM_API_ORIGIN".into(), "invalid secret origin".into())]);
    for command in ["grants-plan", "grants-apply"] {
        assert_eq!(
            run(
                args(&[
                    command,
                    "--app-id",
                    "22222222-2222-4222-8222-222222222222",
                    "--manifest",
                    path.to_str().unwrap()
                ]),
                &environment
            )
            .unwrap_err()
            .category(),
            "APPLICATION_SELECTION_CONFLICT"
        );
        assert!(
            run(
                args(&[
                    command,
                    "--app-id",
                    "11111111-1111-4111-8111-111111111111",
                    "--manifest",
                    path.to_str().unwrap()
                ]),
                &environment
            )
            .is_ok()
        );
        let output = run(
            args(&[command, "--manifest", path.to_str().unwrap()]),
            &environment,
        )
        .unwrap();
        assert_eq!(output["applicationCode"], "stores");
        assert!(
            run(
                args(&[
                    command,
                    "--app-code",
                    "stores",
                    "--manifest",
                    path.to_str().unwrap()
                ]),
                &environment
            )
            .is_ok()
        );
        assert_eq!(
            run(
                args(&[
                    command,
                    "--app-code",
                    "work-orders",
                    "--manifest",
                    path.to_str().unwrap()
                ]),
                &environment
            )
            .unwrap_err()
            .category(),
            "APPLICATION_SELECTION_CONFLICT"
        );
    }
    assert_eq!(
        run(
            args(&[
                "grants-apply",
                "--apply",
                "--app-code",
                "work-orders",
                "--manifest",
                path.to_str().unwrap()
            ]),
            &environment
        )
        .unwrap_err()
        .category(),
        "APPLICATION_SELECTION_CONFLICT"
    );
    manifest["applicationCode"] = json!("unknown-app");
    fs::write(&path, manifest.to_string()).unwrap();
    assert_eq!(
        run(
            args(&["grants-plan", "--manifest", path.to_str().unwrap()]),
            &environment
        )
        .unwrap_err()
        .category(),
        "INVALID_GRANT_MANIFEST"
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn repeated_stores_bootstrap_reads_exact_existing_menus_without_secret_or_client_writes() {
    let app_id = "11111111-1111-4111-8111-111111111111";
    let root = "00000000-0000-4000-8000-00000000000a";
    let bootstrap: Value =
        serde_json::from_str(include_str!("../manifests/stores-bootstrap.manifest.json")).unwrap();
    let app = json!({"id":app_id,"code":"stores","runtimeMode":"TRUSTED","entryType":"HTML","basePath":"/micro/stores","status":"ENABLED"});
    let client = json!({"id":"22222222-2222-4222-8222-222222222222","clientId":"owned-client","clientName":"stores-client",
        "clientType":"CONFIDENTIAL","status":"ENABLED","grantTypes":["client_credentials"],"scopes":["openid"],"redirectUris":[],
        "accessTokenValidity":300,"refreshTokenValidity":300,"applicationId":app_id,"clientSecret":"REMOTE_SECRET_CANARY"});
    let menus = bootstrap["menus"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(index, menu)| {
            let mut row = menu.clone();
            let object = row.as_object_mut().unwrap();
            object.remove("key");
            object.remove("parentKey");
            object.remove("status");
            row["id"] = json!(format!("00000000-0000-4000-8000-{:012x}", index + 10));
            row["applicationId"] = json!(app_id);
            row["parentId"] = if index == 0 { Value::Null } else { json!(root) };
            row
        })
        .collect::<Vec<_>>();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for _ in 0..6 {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        thread::sleep(std::time::Duration::from_millis(5))
                    }
                    Err(_) => panic!("owned bootstrap fixture did not receive a request"),
                }
            };
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut bytes = [0; 8192];
            let size = stream.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..size]).to_string();
            let path = request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            assert!(
                request.starts_with("GET "),
                "duplicate bootstrap must never create, rotate or rewrite an existing object"
            );
            let body = match path {
                "/api/applications" => json!([app.clone()]),
                "/api/oauth/clients" => json!([client.clone()]),
                value if value.starts_with("/api/menus/tree?applicationId=") => {
                    json!({"menus":menus})
                }
                _ => panic!("unexpected bootstrap endpoint"),
            }
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
            requests.push(request);
        }
        requests
    });
    let mut environment = env(origin);
    environment.insert("PLATFORM_CLIENT_ID".into(), "owned-client".into());
    environment.insert(
        "PLATFORM_CLIENT_SECRET".into(),
        "LOCAL_SECRET_CANARY".into(),
    );
    for _ in 0..2 {
        let output = run(
            args(&[
                "bootstrap",
                "--app-code",
                "stores",
                "--apply",
                "--environment",
                "development",
                "--allow-loopback-http",
            ]),
            &environment,
        )
        .unwrap();
        assert_eq!(output["created"], json!([]));
        assert_eq!(output["menuIds"].as_object().unwrap().len(), 11);
        assert!(!output.to_string().contains("CANARY"));
    }
    let requests = handle.join().unwrap();
    assert_eq!(requests.len(), 6);
    assert!(
        requests
            .iter()
            .all(|request| !request.contains("SECRET_CANARY"))
    );
}
#[test]
fn url_and_option_attacks_return_fixed_categories() {
    for values in [
        vec!["--bearer", "CANARY"],
        vec!["--client-secret", "CANARY"],
        vec!["bootstrap", "--apply", "--apply"],
        vec!["inspect", "--apply"],
        vec!["plan", "--timeout-ms", "0"],
    ] {
        assert_eq!(
            run(args(&values), &BTreeMap::new()).unwrap_err().category(),
            "INVALID_ARGUMENT"
        );
    }
    let path = temp_json(&release());
    for url in [
        "//evil.invalid/index.html",
        "/sub-apps/work-orders/versions/1.2.3/%2e%2e/index.html",
        "/sub-apps/work-orders/versions/1.2.3/%252fsecret.html",
        "https://user:CANARY@invalid/sub-apps/work-orders/versions/1.2.3/index.html",
        "/sub-apps/work-orders/versions/1.2.3/index.html?CANARY",
    ] {
        let mut invalid = release();
        invalid["entryUrl"] = json!(url);
        fs::write(&path, invalid.to_string()).unwrap();
        let error = run(
            args(&["register", "--release-file", path.to_str().unwrap()]),
            &BTreeMap::new(),
        )
        .unwrap_err();
        assert!(!error.to_string().contains("CANARY"));
        assert_eq!(error.category(), "INVALID_RELEASE");
    }
    fs::remove_file(path).unwrap();
}
fn server(response: &'static [u8]) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let handle = thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && std::time::Instant::now() < deadline =>
                {
                    thread::sleep(std::time::Duration::from_millis(5))
                }
                Err(_) => {
                    panic!("TCP fixture unavailable; local loopback networking must be enabled")
                }
            }
        };
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        stream
            .set_write_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut data = [0; 8192];
        let size = stream.read(&mut data).unwrap();
        let request = String::from_utf8_lossy(&data[..size]).into_owned();
        stream.write_all(response).unwrap();
        request
    });
    (format!("http://{address}"), handle)
}
fn env(origin: String) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("PLATFORM_API_ORIGIN".into(), origin),
        ("PLATFORM_ADMIN_BEARER".into(), "BEARER_CANARY".into()),
    ])
}
#[test]
fn production_http_is_rejected_before_connection() {
    let error = run(args(&["releases"]), &env("http://127.0.0.1:1".into())).unwrap_err();
    assert_eq!(error.category(), "HTTPS_REQUIRED");
}
#[test]
fn redirects_and_response_bodies_never_leak() {
    for response in [
        b"HTTP/1.1 302 Found\r\nLocation: http://CANARY.invalid/token\r\nContent-Length: 0\r\n\r\n"
            .as_slice(),
        b"HTTP/1.1 403 Forbidden\r\nContent-Length: 18\r\n\r\nBODY_SECRET_CANARY!".as_slice(),
    ] {
        let (origin, handle) = server(response);
        let error = run(
            args(&[
                "releases",
                "--environment",
                "development",
                "--allow-loopback-http",
                "--timeout-ms",
                "1000",
            ]),
            &env(origin),
        )
        .unwrap_err();
        let request = handle.join().unwrap();
        assert!(request.starts_with("GET /api/applications "));
        assert!(!error.to_string().contains("CANARY"));
        assert!(!format!("{error:?}").contains("CANARY"));
        assert!(!error.to_json().to_string().contains("CANARY"));
        assert!(matches!(
            error.category(),
            "REDIRECT_REJECTED" | "HTTP_ERROR"
        ));
    }
}

#[test]
fn duplicate_keys_and_oversized_manifests_fail_without_environment() {
    let path = temp_json(&release());
    fs::write(&path, r#"{"version":"1","version":"2"}"#).unwrap();
    let error = run(
        args(&["register", "--release-file", path.to_str().unwrap()]),
        &BTreeMap::new(),
    )
    .unwrap_err();
    assert_eq!(error.category(), "INVALID_RELEASE");
    fs::write(&path, vec![b' '; 1024 * 1024 + 1]).unwrap();
    assert_eq!(
        run(
            args(&["register", "--release-file", path.to_str().unwrap()]),
            &BTreeMap::new()
        )
        .unwrap_err()
        .category(),
        "INVALID_RELEASE"
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn origin_configuration_is_exact_and_never_echoed() {
    for origin in [
        "https://user:ORIGIN_CANARY@localhost",
        "https://localhost/path",
        "https://localhost?ORIGIN_CANARY",
        "https://localhost/#ORIGIN_CANARY",
        "https://localhost\\evil",
        "http://127.1",
    ] {
        let error = run(args(&["releases"]), &env(origin.into())).unwrap_err();
        assert_eq!(error.category(), "INVALID_ORIGIN");
        assert!(!error.to_json().to_string().contains("CANARY"));
    }
}

#[test]
fn explicit_ca_configuration_accepts_certificates_only_and_is_bounded() {
    let path = temp_json(&json!(null));
    for content in [
        "-----BEGIN PRIVATE KEY-----\nCA_SECRET_CANARY\n-----END PRIVATE KEY-----".to_owned(),
        "a".repeat(65537),
        "-----BEGIN CERTIFICATE-----\nINVALID_CANARY\n-----END CERTIFICATE-----".to_owned(),
        "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----".to_owned(),
    ] {
        fs::write(&path, content).unwrap();
        let mut environment = env("https://127.0.0.1:1".into());
        environment.insert(
            "PLATFORM_CA_FILE".into(),
            path.to_string_lossy().into_owned(),
        );
        let error = run(args(&["releases"]), &environment).unwrap_err();
        assert_eq!(error.category(), "INVALID_CA_CONFIGURATION");
        assert!(!error.to_json().to_string().contains("CANARY"));
        assert!(
            !error
                .to_json()
                .to_string()
                .contains(&path.to_string_lossy().into_owned())
        );
    }
    fs::remove_file(path).unwrap();
}
