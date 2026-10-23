use saas_platform_admin::run;
use std::collections::BTreeMap;

#[test]
fn remote_preview_uses_only_get_and_projects_safe_fields() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::{Duration, Instant},
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let id = "11111111-1111-4111-8111-111111111111";
    let handle = std::thread::spawn(move || {
        for index in 0..2 {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(_) => panic!("preview fixture deadline"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = [0; 8192];
            let size = stream.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..size]);
            let expected = if index == 0 {
                "/api/applications".to_owned()
            } else {
                format!(
                    "/api/applications/{id}/releases/retention-preview?minimumAgeDays=60&keepLatest=8"
                )
            };
            assert_eq!(
                request.lines().next().unwrap(),
                format!("GET {expected} HTTP/1.1")
            );
            let body = if index == 0 {
                serde_json::json!([{"id":id,"code":"work-orders","basePath":"/micro/work-orders","entryType":"HTML","runtimeMode":"TRUSTED","status":"ENABLED"}])
            } else {
                serde_json::json!({"applicationId":id,"mode":"DRY_RUN","applyAvailable":false,"deletionPermitted":false,"policy":{"minimumAgeDays":60,"keepLatest":8},"total":1,"protected":1,"manualReview":0,"records":[{"releaseId":"22222222-2222-4222-8222-222222222222","version":"v1","environment":"production","artifactKind":"ZIP","disposition":"PROTECTED","reasons":["ACTIVE"],"privatePath":"SECRET_SERVER_FIELD"}],"secret":"SECRET_SERVER_FIELD"})
            }.to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",body.len()).unwrap();
        }
    });
    let env = BTreeMap::from([
        ("PLATFORM_API_ORIGIN".into(), origin),
        ("PLATFORM_ADMIN_BEARER".into(), "phase31-test-canary".into()),
    ]);
    let result = run(
        [
            "retention-preview",
            "--environment",
            "development",
            "--allow-loopback-http",
            "--minimum-age-days",
            "60",
            "--keep-latest",
            "8",
        ]
        .into_iter()
        .map(str::to_owned),
        &env,
    );
    handle.join().unwrap();
    let result = result.unwrap();
    assert_eq!(result["protected"], 1);
    assert!(!result.to_string().contains("SECRET_SERVER_FIELD"));
    assert!(!result.to_string().contains("phase31-test-canary"));
}

#[test]
fn retention_is_remote_read_only_and_never_accepts_apply_or_weaker_policy() {
    for args in [
        vec!["retention-preview", "--apply"],
        vec!["retention-preview", "--minimum-age-days", "29"],
        vec!["retention-preview", "--keep-latest", "4"],
        vec!["retention-preview", "--keep-latest", "5001"],
        vec!["plan", "--minimum-age-days", "30"],
    ] {
        let error = run(args.into_iter().map(str::to_owned), &BTreeMap::new()).unwrap_err();
        assert_eq!(error.category(), "INVALID_ARGUMENT");
    }
    // A successful-looking offline PLAN would be wrong: this must read actual references.
    let error = run(["retention-preview".to_owned()], &BTreeMap::new()).unwrap_err();
    assert_eq!(error.category(), "INVALID_ORIGIN");
}
