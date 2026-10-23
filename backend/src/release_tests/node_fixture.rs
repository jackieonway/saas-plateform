//! Optional real HTTP fixture for hosts that cannot accept Rust test-process traffic.
//!
//! `MICRO_PROXY_TEST_NODE` names the Node executable; no shell, global network
//! settings, or existing application services are involved. The script mirrors
//! the native fixture in the parent module and logs only paths and Origin.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use super::*;

type PendingCommands = Arc<Mutex<HashMap<u64, Sender<()>>>>;
type NodeInput = Arc<Mutex<ChildStdin>>;

pub(super) struct NodeHttp {
    child: Child,
    input: NodeInput,
    pending: PendingCommands,
    next_command: AtomicU64,
    reader: Option<thread::JoinHandle<()>>,
}

fn send(input: &NodeInput, value: &Value) -> Result<()> {
    let mut input = input
        .lock()
        .map_err(|_| anyhow!("Node HTTP fixture input lock poisoned"))?;
    serde_json::to_writer(&mut *input, value)
        .map_err(|_| anyhow!("writing Node HTTP fixture command failed"))?;
    input
        .write_all(b"\n")
        .and_then(|_| input.flush())
        .map_err(|_| anyhow!("sending Node HTTP fixture command failed"))
}

impl NodeHttp {
    pub(super) fn set_unavailable(&self, path: &str, unavailable: bool) -> Result<()> {
        let id = self.next_command.fetch_add(1, Ordering::SeqCst);
        let (acknowledged, acknowledgement) = mpsc::channel();
        self.pending
            .lock()
            .map_err(|_| anyhow!("Node HTTP fixture command lock poisoned"))?
            .insert(id, acknowledged);
        let result = send(
            &self.input,
            &json!({ "op": "availability", "id": id, "path": path, "unavailable": unavailable }),
        )
        .and_then(|_| {
            acknowledgement
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| anyhow!("Node HTTP fixture availability acknowledgement timed out"))
        });
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&id);
        }
        result
    }
}

impl Drop for NodeHttp {
    fn drop(&mut self) {
        // This exclusively owned process has no persistent data or external services.
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

pub(super) async fn start() -> Result<HttpFixture> {
    tokio::task::spawn_blocking(start_blocking)
        .await
        .map_err(|_| anyhow!("starting Node HTTP fixture worker failed"))?
}

fn start_blocking() -> Result<HttpFixture> {
    let executable = std::env::var_os("MICRO_PROXY_TEST_NODE")
        .ok_or_else(|| anyhow!("MICRO_PROXY_TEST_NODE must name the Node executable"))?;
    let mut command = Command::new(executable);
    command
        .arg("-e")
        .arg(SCRIPT)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command
        .spawn()
        .map_err(|_| anyhow!("starting Node HTTP fixture process failed"))?;
    let Some(input) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(anyhow!("Node HTTP fixture input pipe unavailable"));
    };
    let Some(output) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(anyhow!("Node HTTP fixture output pipe unavailable"));
    };
    let input = Arc::new(Mutex::new(input));
    let pending: PendingCommands = Arc::new(Mutex::new(HashMap::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let request_headers = Arc::new(Mutex::new(Vec::new()));
    let denied_hits = Arc::new(AtomicUsize::new(0));
    let cors_requests = Arc::new(Mutex::new(Vec::new()));
    let unavailable = Arc::new(Mutex::new(HashSet::new()));
    let (ready, startup) = mpsc::channel();
    let reader_input = input.clone();
    let reader_pending = pending.clone();
    let reader_requests = requests.clone();
    let reader_headers = request_headers.clone();
    let reader_hits = denied_hits.clone();
    let reader_cors = cors_requests.clone();
    let reader = thread::spawn(move || {
        let mut ready = Some(ready);
        for line in BufReader::new(output).lines() {
            let Ok(line) = line else { break };
            let Ok(event) = serde_json::from_str::<Value>(&line) else {
                break;
            };
            match event["type"].as_str() {
                Some("ready") => {
                    let Some(sender) = ready.take() else { break };
                    let (Some(origin), Some(other_origin)) =
                        (event["origin"].as_str(), event["otherOrigin"].as_str())
                    else {
                        break;
                    };
                    if sender
                        .send((origin.to_owned(), other_origin.to_owned()))
                        .is_err()
                    {
                        break;
                    }
                }
                Some("request") => {
                    let (Some(path), Some(id)) = (event["path"].as_str(), event["id"].as_u64())
                    else {
                        break;
                    };
                    let origin = event["origin"].as_str().map(str::to_owned);
                    if event["server"] == "main" {
                        if let Ok(mut requests) = reader_requests.lock() {
                            requests.push(path.to_owned());
                        }
                        if let Ok(mut headers) = reader_headers.lock() {
                            headers.push((path.to_owned(), origin));
                        }
                    } else if event["server"] == "other" {
                        reader_hits.fetch_add(1, Ordering::SeqCst);
                        if let Ok(mut requests) = reader_cors.lock() {
                            requests.push((path.to_owned(), origin));
                        }
                    } else {
                        break;
                    }
                    // Node responds only after Rust records the event, so even a
                    // test that immediately checks its logs has deterministic state.
                    if send(&reader_input, &json!({ "op": "logged", "id": id })).is_err() {
                        break;
                    }
                }
                Some("ack") => {
                    if let Some(id) = event["id"].as_u64() {
                        if let Ok(mut pending) = reader_pending.lock() {
                            if let Some(sender) = pending.remove(&id) {
                                let _ = sender.send(());
                            }
                        }
                    }
                }
                _ => break,
            }
        }
        // Wake any control callers immediately if the process/pipe terminates.
        if let Ok(mut pending) = reader_pending.lock() {
            pending.clear();
        }
    });
    let node = NodeHttp {
        child,
        input,
        pending,
        next_command: AtomicU64::new(1),
        reader: Some(reader),
    };
    let (origin, other_origin) = startup
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| anyhow!("Node HTTP fixture did not become ready"))?;
    Ok(HttpFixture {
        origin,
        other_origin,
        requests,
        request_headers,
        denied_hits,
        cors_requests,
        unavailable,
        servers: Vec::new(),
        node: Some(node),
    })
}

const SCRIPT: &str = r#"
'use strict';
const http = require('node:http');
const readline = require('node:readline');
const unavailable = new Set();
const waitingRequests = new Map();
let requestId = 1;
let gatewayOrigin;
let otherOrigin;

function emit(value) {
  process.stdout.write(JSON.stringify(value) + '\n');
}

function reply(res, status, headers, body) {
  res.writeHead(status, headers);
  res.end(body);
}

function frameHeaders(headers, version) {
  version = version.replace(/-remote$/, '');
  switch (version) {
    case 'xfo-deny': headers['X-Frame-Options'] = 'DENY'; break;
    case 'xfo-sameorigin': headers['X-Frame-Options'] = 'SAMEORIGIN'; break;
    case 'csp-none': headers['Content-Security-Policy'] = "frame-ancestors 'none'"; break;
    case 'csp-wrong': headers['Content-Security-Policy'] = 'frame-ancestors https://wrong.invalid'; break;
    case 'csp-self': headers['Content-Security-Policy'] = "frame-ancestors 'self'"; break;
    case 'csp-gateway': headers['Content-Security-Policy'] = 'frame-ancestors ' + gatewayOrigin; break;
    case 'csp-star': headers['Content-Security-Policy'] = 'frame-ancestors *'; break;
    case 'csp-multiple-none': headers['Content-Security-Policy'] = ['frame-ancestors *', "frame-ancestors 'none'"]; break;
    case 'csp-combined-none': headers['Content-Security-Policy'] = "frame-ancestors *, frame-ancestors 'none'"; break;
  }
  return headers;
}

function otherResource(path, res) {
  let mime, body;
  if (path.endsWith('/main.js')) {
    mime = 'application/javascript';
    body = 'window.__MICRO_APP__ = { mount: function () {}, unmount: function () {} };';
  } else if (path.endsWith('.js')) {
    mime = 'application/javascript';
    body = 'export async function mount() {} export async function unmount() {}';
  } else if (path.endsWith('.css')) {
    mime = 'text/css';
    body = 'body { color: #333; }';
  } else {
    mime = 'text/html';
    body = '<!doctype html><html><head><link rel="stylesheet" href="./styles.css"></head><body><script src="./main.js"></script></body></html>';
  }
  const headers = { 'Content-Type': mime };
  if (path.includes('star-cors')) headers['Access-Control-Allow-Origin'] = '*';
  else if (path.includes('null-cors')) headers['Access-Control-Allow-Origin'] = 'null';
  else if (path.includes('gateway-cors')) headers['Access-Control-Allow-Origin'] = gatewayOrigin;
  else if (path.includes('wrong-cors')) headers['Access-Control-Allow-Origin'] = 'https://wrong.invalid';
  reply(res, 200, frameHeaders(headers, path.split('/')[2] || ''), body);
}

function fixtureResource(path, res) {
  if (unavailable.has(path)) {
    return reply(res, 503, { 'Content-Type': 'text/plain; charset=utf-8' }, 'fixture temporarily unavailable');
  }
  const parts = path.split('/');
  const version = parts[2] || '';
  const file = parts[parts.length - 1] || '';
  if (version === 'bad404' || file.startsWith('missing.')) {
    return reply(res, 404, { 'Content-Type': 'text/plain; charset=utf-8' }, 'fixture resource missing');
  }
  if (version === 'badredirect' && file === 'index.html') {
    return reply(res, 302, { Location: otherOrigin + '/micro/badredirect/index.html' }, '');
  }
  if (path === '/micro/directoryredirect/index.html') {
    return reply(res, 302, { Location: './nested/index.html' }, '');
  }
  if (version === 'loopredirect' && file === 'index.html') {
    return reply(res, 307, { Location: path }, '');
  }
  if (file === 'health') {
    return reply(res, version === 'badhealth' ? 503 : 200, { 'Content-Type': 'application/json' }, '{"ok":true}');
  }
  if (file === 'index.html') {
    const mime = version === 'badmime' ? 'application/json' : 'text/html; charset=utf-8';
    const script = version === 'badresource404' ? 'missing.js' : 'main.js';
    const base = version === 'basechange' ? '<base href="./assets/">' : '';
    const moduleUrl = version.includes('-cdn-') ? otherOrigin + '/micro/' + version + '/entry.js' : './entry.js';
    const module = version === 'module-inline-trusted'
      ? '<script type="module">export const fixture = true;</script>'
      : version.startsWith('module-')
        ? '<script type="module" src="' + moduleUrl + '"></script>'
        : '<script src="./' + script + '"></script>';
    const preload = version.startsWith('modulepreload-') ? '<link rel="modulepreload" href="' + moduleUrl + '">' : '';
    const body = '<!doctype html><html><head>' + base + preload + '<link rel="stylesheet" href="./styles.css"></head>' +
      '<body><div id="app"></div>' + module + '</body></html>';
    return reply(res, 200, frameHeaders({ 'Content-Type': mime }, version), body);
  }
  if (file.endsWith('.js')) {
    const headers = { 'Content-Type': version === 'badesm' || version === 'badresourcemime' ? 'text/html' : 'application/javascript; charset=utf-8' };
    const body = file === 'main.js'
      ? 'window.__MICRO_APP__ = { mount: function () {}, unmount: function () {} };'
      : 'export async function mount() {} export async function unmount() {}';
    if (version.endsWith('no-cors')) {}
    else if (version.endsWith('null-cors')) headers['Access-Control-Allow-Origin'] = 'null';
    else if (version.endsWith('gateway-cors')) headers['Access-Control-Allow-Origin'] = gatewayOrigin;
    else if (version.endsWith('wrong-cors')) headers['Access-Control-Allow-Origin'] = 'https://wrong.invalid';
    else headers['Access-Control-Allow-Origin'] = '*';
    return reply(res, 200, headers, body);
  }
  if (file.endsWith('.css')) {
    return reply(res, 200, { 'Content-Type': version === 'badstylemime' ? 'text/html' : 'text/css; charset=utf-8' }, 'body { color: #222; }');
  }
  if (file === 'logo.png') {
    return reply(res, 200, { 'Content-Type': 'image/png' }, Buffer.from([0x89, 0x50, 0x4e, 0x47]));
  }
  reply(res, 404, { 'Content-Type': 'text/plain; charset=utf-8' }, 'fixture path missing');
}

function handler(server, serve) {
  return (req, res) => {
    const path = req.url.split('?')[0];
    const id = requestId++;
    waitingRequests.set(id, () => serve(path, res));
    res.once('close', () => waitingRequests.delete(id));
    emit({ type: 'request', server, id, path, origin: typeof req.headers.origin === 'string' ? req.headers.origin : null });
  };
}

const main = http.createServer(handler('main', fixtureResource));
const other = http.createServer(handler('other', otherResource));
const input = readline.createInterface({ input: process.stdin });
input.on('line', line => {
  let command;
  try { command = JSON.parse(line); } catch (_) { process.exit(2); }
  if (command.op === 'logged') {
    const respond = waitingRequests.get(command.id);
    waitingRequests.delete(command.id);
    if (respond) respond();
  } else if (command.op === 'availability' && typeof command.path === 'string' && typeof command.unavailable === 'boolean') {
    if (command.unavailable) unavailable.add(command.path);
    else unavailable.delete(command.path);
    emit({ type: 'ack', id: command.id });
  } else process.exit(2);
});
input.on('close', () => process.exit(0));

function listen(server) {
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
}
Promise.all([listen(main), listen(other)]).then(() => {
  gatewayOrigin = 'http://127.0.0.1:' + main.address().port;
  otherOrigin = 'http://127.0.0.1:' + other.address().port;
  emit({ type: 'ready', origin: gatewayOrigin, otherOrigin });
}).catch(() => process.exit(1));
"#;

/// Real Node HTTP transport for an unchanged Axum router. Only the listener is
/// outside the Rust test process; methods, paths, headers, bodies and responses
/// still pass through the same Router used by the native fixture.
pub(super) struct RouterHttp {
    pub(super) origin: String,
    child: Option<tokio::process::Child>,
    server: Option<JoinHandle<()>>,
}

impl RouterHttp {
    pub(super) async fn close(mut self) -> Result<()> {
        if let Some(server) = self.server.take() {
            server.abort();
            let _ = server.await;
        }
        if let Some(mut child) = self.child.take() {
            child.start_kill()?;
            child.wait().await?;
        }
        Ok(())
    }
}

impl Drop for RouterHttp {
    fn drop(&mut self) {
        if let Some(server) = &self.server {
            server.abort();
        }
        // kill_on_drop also covers startup errors and cancelled test futures.
        if let Some(child) = &mut self.child {
            let _ = child.start_kill();
        }
    }
}

pub(super) async fn start_router(app: Router) -> Result<RouterHttp> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    let executable = std::env::var_os("MICRO_PROXY_TEST_NODE")
        .ok_or_else(|| anyhow!("MICRO_PROXY_TEST_NODE must name the Node executable"))?;
    let mut command = tokio::process::Command::new(executable);
    command
        .arg("-e")
        .arg(ROUTER_SCRIPT)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = command
        .spawn()
        .map_err(|_| anyhow!("starting Node router HTTP fixture failed"))?;
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("Node router fixture input unavailable"))?;
    let output = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Node router fixture output unavailable"))?;
    let mut lines = tokio::io::BufReader::new(output).lines();
    let first = tokio::time::timeout(Duration::from_secs(10), lines.next_line())
        .await
        .map_err(|_| anyhow!("Node router HTTP fixture startup timed out"))??
        .ok_or_else(|| anyhow!("Node router HTTP fixture exited during startup"))?;
    let ready: Value = serde_json::from_str(&first)?;
    ensure!(
        ready["type"] == "ready",
        "invalid Node router fixture startup"
    );
    let origin = ready["origin"]
        .as_str()
        .ok_or_else(|| anyhow!("Node router fixture origin unavailable"))?
        .to_owned();
    let address = origin
        .strip_prefix("http://")
        .ok_or_else(|| anyhow!("invalid Node router fixture origin"))?
        .parse::<std::net::SocketAddr>()?;
    ensure!(address.ip().is_loopback() && address.port() != 0);
    let server = tokio::spawn(async move {
        while let Ok(Some(line)) = lines.next_line().await {
            let event = match serde_json::from_str::<Value>(&line) {
                Ok(event) => event,
                Err(_) => break,
            };
            let id = event["id"].clone();
            let response = match router_response(&app, event).await {
                Ok(response) => response,
                Err(_) => json!({"id":id,"status":500,"headers":[],"body":""}),
            };
            let Ok(mut bytes) = serde_json::to_vec(&response) else {
                break;
            };
            bytes.push(b'\n');
            if input.write_all(&bytes).await.is_err() || input.flush().await.is_err() {
                break;
            }
        }
    });
    Ok(RouterHttp {
        origin,
        child: Some(child),
        server: Some(server),
    })
}

async fn router_response(app: &Router, event: Value) -> Result<Value> {
    use base64::{engine::general_purpose::STANDARD, Engine};

    ensure!(
        event["type"] == "request",
        "invalid Node router fixture request"
    );
    let text = |field: &str| {
        event[field]
            .as_str()
            .ok_or_else(|| anyhow!("invalid Node router fixture request field"))
    };
    let mut request = Request::builder()
        .method(text("method")?)
        .uri(text("path")?);
    for pair in event["headers"]
        .as_array()
        .ok_or_else(|| anyhow!("invalid Node router fixture headers"))?
    {
        request = request.header(
            pair[0]
                .as_str()
                .ok_or_else(|| anyhow!("invalid Node router fixture header name"))?,
            pair[1]
                .as_str()
                .ok_or_else(|| anyhow!("invalid Node router fixture header value"))?,
        );
    }
    let body = STANDARD.decode(text("body")?)?;
    ensure!(
        body.len() <= 1024 * 1024,
        "Node router fixture body too large"
    );
    let response = app.clone().oneshot(request.body(Body::from(body))?).await?;
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| Ok((name.as_str().to_owned(), value.to_str()?.to_owned())))
        .collect::<Result<Vec<_>>>()?;
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok(json!({"id":event["id"],"status":status,"headers":headers,"body":STANDARD.encode(bytes)}))
}

const ROUTER_SCRIPT: &str = r#"
'use strict';
const http = require('node:http');
const readline = require('node:readline');
const pending = new Map();
let nextId = 1;
const server = http.createServer({ maxHeaderSize: 16384 }, (req, res) => {
  const chunks = [];
  let size = 0;
  req.on('data', chunk => {
    size += chunk.length;
    if (size > 1048576) req.destroy();
    else chunks.push(chunk);
  });
  req.on('end', () => {
    const id = nextId++;
    pending.set(id, res);
    res.once('close', () => pending.delete(id));
    const headers = [];
    for (let n = 0; n < req.rawHeaders.length; n += 2) headers.push([req.rawHeaders[n], req.rawHeaders[n + 1]]);
    process.stdout.write(JSON.stringify({ type: 'request', id, method: req.method,
      path: req.url, headers, body: Buffer.concat(chunks).toString('base64') }) + '\n');
  });
});
server.requestTimeout = 30000;
const input = readline.createInterface({ input: process.stdin });
input.on('line', line => {
  let response;
  try { response = JSON.parse(line); } catch (_) { process.exit(2); }
  const res = pending.get(response.id);
  if (!res) return;
  pending.delete(response.id);
  res.writeHead(response.status, response.headers.flat());
  res.end(Buffer.from(response.body, 'base64'));
});
input.on('close', () => server.close(() => process.exit(0)));
server.once('error', () => process.exit(1));
server.listen(0, '127.0.0.1', () => process.stdout.write(JSON.stringify({
  type: 'ready', origin: 'http://127.0.0.1:' + server.address().port,
}) + '\n'));
"#;
