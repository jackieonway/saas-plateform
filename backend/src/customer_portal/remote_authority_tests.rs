use super::*;
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
};

fn wire(client: Uuid) -> Value {
    let user = Uuid::now_v7();
    json!({"contractVersion":"customer-access-proof/1","tokenType":"access_token",
        "clientPurpose":"CUSTOMER_PORTAL","allowedMemberTypes":["CUSTOMER"],"scope":["openid","profile","offline_access"],
        "source":{"issuer":"https://auth.saas.test","clientId":client,"grantId":Uuid::now_v7(),"globalSessionId":Uuid::now_v7(),
            "userId":user,"accountVersion":"0","clientPolicyVersion":"1","authTime":Utc::now().timestamp()-5,
            "expiresAt":Utc::now()+chrono::Duration::seconds(45)},
        "account":{"id":user,"username":"owned-phase12","realName":"Owned fixture","avatar":""}})
}
fn response(status: u16, value: Value) -> Response {
    Response {
        status,
        content_type: Some("application/json; charset=utf-8".into()),
        body: serde_json::to_vec(&value).unwrap(),
    }
}
struct Script {
    calls: AtomicUsize,
    values: Mutex<VecDeque<Response>>,
}
impl ProofTransport for Script {
    fn send<'a>(&'a self, request: Request<'a>) -> HttpFuture<'a> {
        Box::pin(async move {
            assert_eq!(request.contract_version, "customer-access-proof-request/1");
            assert!(canonical_secret(request.oidc_access_token));
            assert!(uuid7::is_uuid_v7(request.expected_client_id));
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self
                .values
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected proof request"))
        })
    }
}
fn authority(client: Uuid, values: Vec<Response>) -> (RemoteAuthority, Arc<Script>) {
    let script = Arc::new(Script {
        calls: AtomicUsize::new(0),
        values: Mutex::new(values.into()),
    });
    (
        RemoteAuthority {
            issuer: "https://auth.saas.test".into(),
            customer_client: client,
            transport: script.clone(),
            slots: Arc::new(tokio::sync::Semaphore::new(16)),
        },
        script,
    )
}

#[test]
fn phase12_customer_authority_configuration_rejects_url_redirection_and_noncanonical_material() {
    assert!(exact_origin("https://identity.saas.test:9443").is_ok());
    for origin in [
        "http://identity.saas.test",
        "https://identity.saas.test/",
        "https://name:secret@identity.saas.test",
        "https://identity.saas.test/internal",
        "https://identity.saas.test?url=evil",
        "https://identity.saas.test#fragment",
    ] {
        assert!(exact_origin(origin).is_err());
    }
    for id in [
        "00000000-0000-4000-8000-000000000001",
        "019a0000-0000-7000-C000-000000000001",
        "019A0000-0000-7000-8000-000000000001",
    ] {
        assert!(canonical_id(id).is_err());
    }
    for token in [
        "Bearer token",
        "refresh_token",
        "eyJhbGciOiJSUzI1NiJ9.claim.signature",
        "",
    ] {
        assert!(!canonical_secret(token));
    }
    for raw in ["-1", "01", "+1", " 1", "1.0", "9223372036854775808"] {
        assert!(generation(raw, 0).is_err());
    }
    assert_eq!(generation("0", 0).unwrap(), 0);
    assert!(generation("0", 1).is_err());
}

#[test]
fn phase12_customer_authority_accepts_only_exact_access_customer_rp_proofs() {
    let client = Uuid::now_v7();
    let value = wire(client);
    let proof = decode(
        response(200, value.clone()),
        "https://auth.saas.test",
        client,
    )
    .unwrap();
    assert_eq!(proof.source.account_version, 0);
    assert_eq!(proof.source.client_id, client);
    assert_eq!(proof.account.id, proof.source.user_id);
    for (field, replacement) in [
        ("contractVersion", json!("oidc-account-proof/1")),
        ("tokenType", json!("refresh_token")),
        ("clientPurpose", json!("PLATFORM_CONSOLE")),
        ("allowedMemberTypes", json!(["EMPLOYEE", "CUSTOMER"])),
        ("scope", json!(["openid"])),
        ("scope", json!(["openid", "profile", "profile"])),
    ] {
        let mut bad = value.clone();
        bad[field] = replacement;
        assert!(decode(response(200, bad), "https://auth.saas.test", client).is_err());
    }
    for (field, replacement) in [
        ("issuer", json!("https://another.saas.test")),
        ("clientId", json!(Uuid::now_v7())),
        ("accountVersion", json!("-1")),
        ("clientPolicyVersion", json!("0")),
        ("authTime", json!(Utc::now().timestamp() + 61)),
        (
            "expiresAt",
            json!(Utc::now() - chrono::Duration::seconds(1)),
        ),
    ] {
        let mut bad = value.clone();
        bad["source"][field] = replacement;
        assert!(decode(response(200, bad), "https://auth.saas.test", client).is_err());
    }
    let mut foreign = value.clone();
    foreign["account"]["id"] = json!(Uuid::now_v7());
    assert!(decode(response(200, foreign), "https://auth.saas.test", client).is_err());
    let mut credentials = value;
    credentials["account"]["passwordHash"] = json!("must-never-cross-the-port");
    assert!(decode(response(200, credentials), "https://auth.saas.test", client).is_err());
}

#[test]
fn phase12_customer_authority_distinguishes_invalid_source_from_dependency_failure() {
    let client = Uuid::now_v7();
    assert!(matches!(
        decode(
            response(401, json!({"error":"invalid_token"})),
            "https://auth.saas.test",
            client
        ),
        Err(AppError::Unauthorized(_))
    ));
    for status in [302, 400, 403, 404, 429, 500, 503] {
        assert!(matches!(
            decode(
                response(status, json!({"error":"unavailable"})),
                "https://auth.saas.test",
                client
            ),
            Err(AppError::Unavailable)
        ));
    }
    for response in [
        Response {
            status: 200,
            content_type: Some("text/html".into()),
            body: vec![],
        },
        Response {
            status: 200,
            content_type: Some("application/json".into()),
            body: vec![b'x'; LIMIT + 1],
        },
        Response {
            status: 200,
            content_type: Some("application/json".into()),
            body: b"not-json".to_vec(),
        },
    ] {
        assert!(matches!(
            decode(response, "https://auth.saas.test", client),
            Err(AppError::Unavailable)
        ));
    }
}

#[tokio::test]
async fn phase12_customer_authority_checks_inputs_before_http_and_never_caches_proofs() {
    let client = Uuid::now_v7();
    let first = wire(client);
    let mut changed = first.clone();
    changed["source"]["grantId"] = json!(Uuid::now_v7());
    let (port, script) = authority(client, vec![response(200, first), response(200, changed)]);
    for token in ["refresh_token", "Bearer token", ""] {
        assert!(port.verify_access(token, client).await.is_err());
    }
    assert!(port
        .verify_access(&"a".repeat(64), Uuid::now_v7())
        .await
        .is_err());
    assert_eq!(script.calls.load(Ordering::SeqCst), 0);
    let first = port.verify_access(&"a".repeat(64), client).await.unwrap();
    let second = port.verify_access(&"a".repeat(64), client).await.unwrap();
    assert_eq!(script.calls.load(Ordering::SeqCst), 2);
    assert_ne!(first.source.grant_id, second.source.grant_id);
}

#[tokio::test]
async fn phase12_customer_authority_saturation_and_authority_errors_fail_closed_without_retries() {
    let client = Uuid::now_v7();
    let (port, script) = authority(client, vec![]);
    let permits: Vec<_> = (0..16)
        .map(|_| port.slots.clone().try_acquire_owned().unwrap())
        .collect();
    assert!(matches!(
        port.verify_access(&"a".repeat(64), client).await,
        Err(AppError::Unavailable)
    ));
    assert_eq!(script.calls.load(Ordering::SeqCst), 0);
    drop(permits);
    let (port, script) = authority(client, vec![response(503, json!({"error":"unavailable"}))]);
    assert!(matches!(
        port.verify_access(&"a".repeat(64), client).await,
        Err(AppError::Unavailable)
    ));
    assert_eq!(script.calls.load(Ordering::SeqCst), 1);
}

#[cfg(unix)]
mod tls {
    use super::*;
    use anyhow::{anyhow, ensure, Result};
    use std::os::unix::fs::PermissionsExt;
    use std::{path::PathBuf, process::Stdio};
    use tokio::{
        io::{AsyncBufReadExt, BufReader},
        process::{Child, Command},
    };

    struct Fixture {
        directory: PathBuf,
        child: Child,
        port: u16,
    }
    impl Fixture {
        async fn new(value: &Value) -> Result<Self> {
            ensure!(
                std::env::var("IDENTITY_PHASE12_TLS_TESTS").as_deref() == Ok("TASK_OWNED"),
                "explicit TASK_OWNED TLS test opt-in required"
            );
            let directory = std::env::temp_dir()
                .join(format!("identity-phase12-authority-tls-{}", Uuid::now_v7()));
            std::fs::create_dir(&directory)?;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
            let openssl = |arguments: &[&str]| -> Result<()> {
                let status = std::process::Command::new("openssl")
                    .current_dir(&directory)
                    .args(arguments)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .map_err(|_| anyhow!("owned TLS fixture openssl unavailable"))?;
                ensure!(
                    status.success(),
                    "owned TLS certificate provisioning failed"
                );
                Ok(())
            };
            openssl(&[
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-keyout",
                "ca.key",
                "-out",
                "ca.pem",
                "-days",
                "1",
                "-subj",
                "/CN=Phase12 Owned Authority CA",
                "-addext",
                "basicConstraints=critical,CA:TRUE",
            ])?;
            openssl(&[
                "req",
                "-new",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-keyout",
                "server.key",
                "-out",
                "server.csr",
                "-subj",
                "/CN=localhost",
            ])?;
            std::fs::write(directory.join("extensions"),"basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:localhost\n")?;
            openssl(&[
                "x509",
                "-req",
                "-in",
                "server.csr",
                "-CA",
                "ca.pem",
                "-CAkey",
                "ca.key",
                "-CAcreateserial",
                "-days",
                "1",
                "-out",
                "server.pem",
                "-extfile",
                "extensions",
            ])?;
            for file in ["ca.key", "server.key"] {
                std::fs::set_permissions(
                    directory.join(file),
                    std::fs::Permissions::from_mode(0o600),
                )?;
            }
            std::fs::write(
                directory.join("response.json"),
                serde_json::to_vec(&json!({"mode":"success","body":value}))?,
            )?;
            std::fs::write(
                directory.join("server.py"),
                r#"
import base64, http.server, json, pathlib, ssl
root=pathlib.Path(__file__).parent
class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self,*args): pass
    def do_GET(self):
        (root/'unexpected-redirect-follow').write_text('rejected')
        self.send_response(501); self.end_headers()
    def do_POST(self):
        try:
            length=int(self.headers.get('Content-Length','0'))
            value=json.loads(self.rfile.read(length)) if 0<length<=4096 else None
            expected='Basic '+base64.b64encode(('019a0000-0000-7000-8000-000000000001:'+('b'*64)).encode()).decode()
            if self.path!='/internal/identity/customer-access-proof' or self.headers.get('Authorization')!=expected or value.get('contractVersion')!='customer-access-proof-request/1' or value.get('oidcAccessToken')!='a'*64:
                self.send_response(403); self.end_headers(); return
            settings=json.loads((root/'response.json').read_text())
            mode=settings['mode']
            if mode=='redirect':
                self.send_response(302); self.send_header('Location','https://localhost:'+str(self.server.server_port)+'/credential-leak'); self.end_headers(); return
            if mode=='oversize':
                body=b'x'*65537
            else: body=json.dumps(settings['body']).encode()
            self.send_response(200); self.send_header('Content-Type','application/json'); self.send_header('Content-Length',str(len(body))); self.send_header('Cache-Control','no-store'); self.end_headers(); self.wfile.write(body)
        except Exception:
            self.send_response(503); self.end_headers()
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
context=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(str(root/'server.pem'),str(root/'server.key'))
server.socket=context.wrap_socket(server.socket,server_side=True)
print(server.server_port,flush=True)
server.serve_forever()
"#,
            )?;
            let mut child = Command::new("python3")
                .arg(directory.join("server.py"))
                .current_dir(&directory)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|_| anyhow!("owned TLS fixture Python unavailable"))?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| anyhow!("owned TLS fixture missing readiness"))?;
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            tokio::time::timeout(Duration::from_secs(3), reader.read_line(&mut line))
                .await
                .map_err(|_| anyhow!("owned TLS fixture readiness timed out"))??;
            let port = line
                .trim()
                .parse::<u16>()
                .map_err(|_| anyhow!("owned TLS fixture readiness invalid"))?;
            Ok(Self {
                directory,
                child,
                port,
            })
        }
        fn authority(&self, client: Uuid, host: &str, trusted: bool) -> Result<RemoteAuthority> {
            let ca = self.directory.join("ca.pem");
            Ok(RemoteAuthority {
                issuer: "https://auth.saas.test".into(),
                customer_client: client,
                transport: Arc::new(HttpTransport {
                    client: http_client(if trusted { ca.to_str() } else { None })?,
                    endpoint: format!("https://{host}:{}{PATH}", self.port),
                    authorization: format!(
                        "Basic {}",
                        STANDARD.encode(format!(
                            "019a0000-0000-7000-8000-000000000001:{}",
                            "b".repeat(64)
                        ))
                    ),
                }),
                slots: Arc::new(tokio::sync::Semaphore::new(16)),
            })
        }
        async fn finish(mut self) -> Result<()> {
            self.child.kill().await?;
            let _ = self.child.wait().await?;
            ensure!(
                self.directory.parent() == Some(std::env::temp_dir().as_path())
                    && self.directory.file_name().is_some_and(|name| name
                        .to_string_lossy()
                        .starts_with("identity-phase12-authority-tls-")),
                "unsafe owned TLS cleanup target"
            );
            std::fs::remove_dir_all(&self.directory)?;
            Ok(())
        }
    }
    #[tokio::test]
    #[ignore = "requires explicit TASK_OWNED TLS fixtures, Python3 and OpenSSL"]
    async fn phase12_customer_authority_real_tls_validates_ca_hostname_redirects_and_response_limits(
    ) -> Result<()> {
        let client = Uuid::now_v7();
        let value = wire(client);
        let fixture = Fixture::new(&value).await?;
        let result = async {
            let valid = fixture.authority(client, "localhost", true)?;
            let proof = valid.verify_access(&"a".repeat(64), client).await?;
            ensure!(
                proof.source.client_id == client,
                "valid TLS source was changed"
            );
            ensure!(
                matches!(
                    fixture
                        .authority(client, "localhost", false)?
                        .verify_access(&"a".repeat(64), client)
                        .await,
                    Err(AppError::Unavailable)
                ),
                "untrusted TLS accepted"
            );
            ensure!(
                matches!(
                    fixture
                        .authority(client, "127.0.0.1", true)?
                        .verify_access(&"a".repeat(64), client)
                        .await,
                    Err(AppError::Unavailable)
                ),
                "wrong hostname accepted"
            );
            for mode in ["redirect", "oversize"] {
                std::fs::write(
                    fixture.directory.join("response.json"),
                    serde_json::to_vec(&json!({"mode":mode,"body":value}))?,
                )?;
                ensure!(
                    matches!(
                        valid.verify_access(&"a".repeat(64), client).await,
                        Err(AppError::Unavailable)
                    ),
                    "unsafe TLS authority response accepted"
                );
                ensure!(
                    !fixture
                        .directory
                        .join("unexpected-redirect-follow")
                        .exists(),
                    "authority redirect was followed"
                );
            }
            Ok(())
        }
        .await;
        fixture.finish().await?;
        result
    }
}
