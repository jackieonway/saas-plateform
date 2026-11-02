"""Task-owned TLS gateway; no request URLs, headers or bodies are logged."""

import argparse
import base64
import http.client
import http.server
import importlib.util
import io
import mimetypes
import pathlib
import re
import select
import socket
import ssl
import threading
import time
import urllib.parse


UUID7 = r"[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}"
KID = r"[A-Za-z0-9_-]{1,80}"
IDENTITY_PORT_HEADER = "X-Task-Identity-Port"
WEBSOCKET_MAX_SECONDS = 600
WEBSOCKET_IDLE_SECONDS = 45
WEBSOCKET_MAX_BYTES = 64 * 1024 * 1024


def public_business_target(method, host, path, query, private, enabled):
    """One owned public Console origin; preserve the real application paths."""
    if not enabled or private or host != 'console.saas.test':
        return None
    decoded = urllib.parse.unquote(path)
    if ('\\' in decoded or '%' in decoded or decoded != path
            or any(part in ('.', '..') for part in decoded.split('/'))):
        return None
    if path.startswith('/stores-api/'):
        return 23031
    if path.startswith('/work-orders-api/'):
        return 23004
    if path == '/socket.io/' and method in ('GET', 'POST', 'OPTIONS'):
        return 23003
    return None


def business_static_target(workspace, url_path):
    """No SPA fallback, traversal or files outside the two frozen bundles."""
    match = re.fullmatch(r'/sub/(stores|work-orders)/(.*)', url_path)
    if not match or '\\' in url_path or urllib.parse.unquote(url_path) != url_path:
        return None
    relative = match[2] or 'index.html'
    if any(part in ('.', '..') for part in relative.split('/')):
        return None
    base = (workspace / ('saas-' + match[1]) / 'frontend/dist').resolve()
    target = (base / relative).resolve()
    if target.is_relative_to(base) and target.is_file():
        return target
    return None


def phase25_business_enabled(task, static_workspace):
    """Only enable extra routes for the prepared current SOURCE fixture."""
    if not (task / 'phase25-business-manifest.json').exists():
        return False
    source = pathlib.Path(__file__).resolve().parent
    spec = importlib.util.spec_from_file_location('phase25_gateway_task', source / 'phase23-environment.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    # environment.sh's root namespace controller binds TLS/443 and launches
    # this process as root. Preserve Task's root-caller guard for that chain.
    owner = module.Task()
    if (not owner.state or owner.state.get('prepared') is not True or owner.state.get('mode') != 'SOURCE'
            or owner.state.get('sourceQuarantined')
            or not re.fullmatch(r'phase25-tools-r[1-9][0-9]*', owner.state.get('snapshot', ''))
            or static_workspace != task / ('source-snapshot-' + owner.state['snapshot'])):
        raise ValueError('PHASE25_GATEWAY_SOURCE_BINDING_REQUIRED')
    owner.snapshot(all_inputs=False)
    spec = importlib.util.spec_from_file_location('phase25_gateway_fixture', source / 'phase25-business-fixture.py')
    fixture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fixture)
    business = fixture.validate_manifest(owner, owner.json(task / 'phase25-business-manifest.json'))
    environment = owner.json(task / 'https-environment.json')
    owner.environment_binding(environment, 'SOURCE')
    database = owner.json(task / 'https-database.json')
    if (business.get('prepared') is not True or business['toolsSnapshot'] != owner.state['snapshot']
            or business['baseDatabase'] != database['database']
            or business['deploymentId'] != environment['ids']['deployment']
            or business['clusterId'] != owner.cluster):
        raise ValueError('PHASE25_GATEWAY_MANIFEST_BINDING_REQUIRED')
    return True


def relay_websocket(left, right, *, maximum_seconds=WEBSOCKET_MAX_SECONDS,
                    idle_seconds=WEBSOCKET_IDLE_SECONDS, maximum_bytes=WEBSOCKET_MAX_BYTES):
    """Bounded raw-byte relay. It never parses or creates a WebSocket frame."""
    started = last_activity = time.monotonic()
    transferred = 0
    while True:
        now = time.monotonic()
        remaining = min(maximum_seconds - (now - started), idle_seconds - (now - last_activity))
        if remaining <= 0:
            return
        ready = [channel for channel in (left, right) if isinstance(channel, ssl.SSLSocket) and channel.pending()]
        if not ready:
            ready, _, _ = select.select([left, right], [], [], min(remaining, 1))
        for channel in ready:
            data = channel.recv(min(65536, maximum_bytes - transferred + 1))
            if not data:
                return
            transferred += len(data)
            if transferred > maximum_bytes:
                return
            (right if channel is left else left).sendall(data)
            last_activity = time.monotonic()


def websocket_response(upstream, *, maximum_seconds=5):
    """Read exactly through HTTP headers; leave all frame bytes on the socket."""
    raw = bytearray()
    deadline = time.monotonic() + maximum_seconds
    original_timeout = upstream.gettimeout()
    try:
        while not raw.endswith(b'\r\n\r\n'):
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError('PHASE25_GATEWAY_UPGRADE_HEADER_DEADLINE')
            upstream.settimeout(min(remaining, original_timeout or maximum_seconds))
            chunk = upstream.recv(1)
            if not chunk or len(raw) >= 65536:
                raise ValueError('PHASE25_GATEWAY_UPGRADE_HEADER_INVALID')
            raw.extend(chunk)
    finally:
        upstream.settimeout(original_timeout)
    line, fields = bytes(raw).split(b'\r\n', 1)
    if not re.fullmatch(rb'HTTP/1\.[01] [1-5][0-9]{2}[^\r\n]*', line):
        raise ValueError('PHASE25_GATEWAY_UPGRADE_STATUS_INVALID')
    status = int(line.split(b' ', 2)[1])
    headers = http.client.parse_headers(io.BytesIO(fields))
    if status == 101 and (headers.get('Upgrade', '').lower() != 'websocket'
            or 'upgrade' not in {token.strip().lower() for token in headers.get('Connection', '').split(',')}
            or not headers.get('Sec-WebSocket-Accept')):
        raise ValueError('PHASE25_GATEWAY_UPGRADE_RESPONSE_INVALID')
    return bytes(raw), status, headers


class IdentityTokenUpstreams:
    """Alternate only the owned Phase23 public token requests."""
    def __init__(self, ports=None):
        if ports is not None and (not isinstance(ports, (tuple, list))
                or any(type(port) is not int for port in ports)
                or tuple(ports) != (23441, 23442)):
            raise ValueError("Invalid owned identity token upstreams")
        self.ports = tuple(ports) if ports is not None else None
        self.next = 0
        self.lock = threading.Lock()

    def select(self, method, host, path, query, private):
        if (self.ports is None or private or method != "POST" or host != "auth.saas.test"
                or path != "/oauth/token" or query):
            return None
        with self.lock:
            port = self.ports[self.next]
            self.next = (self.next + 1) % len(self.ports)
            return port


def proxy_request_headers(items, hop_headers):
    return {key: value for key, value in items
            if key.lower() not in hop_headers | {
                "x-forwarded-proto", "x-forwarded-host", "x-forwarded-for",
                IDENTITY_PORT_HEADER.lower(),
            }}


def proxy_response_headers(items, hop_headers, identity_port=None):
    if identity_port is not None and (type(identity_port) is not int or identity_port not in (23441, 23442)):
        raise ValueError("Invalid owned identity response upstream")
    result = [(key, value) for key, value in items
              if key.lower() not in hop_headers | {
                  "content-length", "server", "date", IDENTITY_PORT_HEADER.lower(),
              }]
    if identity_port is not None:
        result.append((IDENTITY_PORT_HEADER, str(identity_port)))
    return result


def identity_private_allowed(method, path):
    """Allow only implemented private Identity routes on the task TLS port."""
    method = "GET" if method == "HEAD" else method
    fixed = {
        ("POST", "/internal/identity/accounts/candidates"),
        ("POST", "/internal/identity/accounts/verify"),
        ("POST", "/internal/identity/accounts/login"),
        ("POST", "/internal/identity/accounts/commands"),
        ("POST", "/internal/identity/customer-access-proof"),
        ("POST", "/internal/identity/auth/account-proof"),
        ("POST", "/internal/identity/auth/customer-account-proof"),
        ("POST", "/internal/identity/auth/resource-access-proof"),
        ("POST", "/internal/identity/auth/context-interactions"),
        ("POST", "/internal/identity/auth/context-selection/source"),
        ("GET", "/internal/identity/auth/clients"),
        ("POST", "/internal/identity/auth/clients"),
        ("POST", "/internal/identity/auth/consent-groups"),
        ("GET", "/internal/identity/auth/invitations"),
        ("POST", "/internal/identity/auth/invitations"),
        ("POST", "/internal/identity/auth/keys/publish"),
        ("POST", "/internal/identity/auth/keys/activate"),
        ("POST", "/internal/identity/security/mfa/recover"),
    }
    if (method, path) in fixed:
        return True
    dynamic = (
        ({"GET"}, rf"/internal/identity/accounts/(?:operations/)?{UUID7}"),
        ({"GET", "PATCH"}, rf"/internal/identity/auth/clients/{UUID7}"),
        ({"POST"}, rf"/internal/identity/auth/clients/{UUID7}/credentials(?:/revoke)?"),
        ({"GET", "PUT"}, rf"/internal/identity/auth/applications/{UUID7}"),
        ({"GET"}, rf"/internal/identity/auth/applications/{UUID7}/consent-groups"),
        ({"POST"}, rf"/internal/identity/auth/consent-groups/{UUID7}/clients"),
        ({"GET"}, rf"/internal/identity/auth/(?:operations|client-operations|review-operations|invitation-operations)/{UUID7}"),
        ({"GET", "PUT"}, rf"/internal/identity/auth/review/applications/{UUID7}/(?:test|production)"),
        ({"GET"}, rf"/internal/identity/auth/invitations/{UUID7}"),
        ({"POST"}, rf"/internal/identity/auth/invitations/{UUID7}/revoke"),
        ({"GET"}, rf"/internal/identity/auth/key-operations/{UUID7}"),
        ({"DELETE"}, rf"/internal/identity/auth/keys/{KID}"),
        ({"POST"}, rf"/internal/identity/auth/keys/{KID}/revoke"),
    )
    return any(method in methods and re.fullmatch(pattern, path) for methods, pattern in dynamic)


def platform_resource_authorization_allowed(method, path, query):
    """Use one private origin for exact resource authorization and directories."""
    return (method == "POST" and not query and path in {
        "/api/apps/stores/authorize", "/api/apps/work-orders/authorize",
    })


def main(stop_event=None, identity_token_ports=None):
    token_upstreams = IdentityTokenUpstreams(identity_token_ports)
    parser = argparse.ArgumentParser()
    parser.add_argument("--task-dir", type=pathlib.Path, required=True)
    parser.add_argument("--workspace", type=pathlib.Path, required=True)
    parser.add_argument("--static-workspace", type=pathlib.Path,
                        help="Owned source snapshot containing frozen frontend bundles")
    args = parser.parse_args()
    task, workspace = args.task_dir.resolve(), args.workspace.resolve()
    if not (task / "OWNER").is_file():
        raise SystemExit("Task ownership marker is required")
    static_workspace = args.static_workspace.resolve(strict=True) if args.static_workspace else workspace
    if static_workspace != workspace and not static_workspace.is_relative_to(task):
        raise SystemExit("Static bundles must be in the workspace or owned task snapshot")
    business_enabled = phase25_business_enabled(task, static_workspace)
    destinations = {
        "auth.saas.test": 23441,
        "console.saas.test": 23001,
        "customer.saas.test": 23101,
    }
    hop_headers = {
        "connection", "keep-alive", "proxy-authenticate", "proxy-authorization",
        "te", "trailer", "transfer-encoding", "upgrade",
    }

    class Gateway(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_args):
            pass

        def fail(self, status):
            self.send_response(status)
            self.send_header("Content-Length", "0")
            self.end_headers()

        def route(self):
            host = self.headers.get("Host", "")
            if self.server.private:
                if not host.endswith(":24444"):
                    return self.fail(404)
                host = host[:-6]
            elif host.endswith(":443"):
                host = host[:-4]
            if host not in destinations:
                return self.fail(404)
            parsed = urllib.parse.urlsplit(self.path)
            if parsed.scheme or parsed.netloc or not parsed.path.startswith("/") or "\\" in parsed.path:
                return self.fail(400)
            decoded = urllib.parse.unquote(parsed.path)
            internal = decoded == "/internal" or decoded.startswith("/internal/")
            readiness_probe = (
                self.server.private and host == "auth.saas.test"
                and self.command in ("GET", "HEAD")
                and parsed.path == "/health/ready" and not parsed.query
            )
            resource_authorization = (
                self.server.private and host == "console.saas.test"
                and platform_resource_authorization_allowed(self.command, parsed.path, parsed.query)
            )
            notification_proof = (self.server.private and host == "console.saas.test"
                and self.command == "GET" and parsed.path == '/api/internal/notifications/context-proof')
            task_delivery = (
                self.server.private and host == "auth.saas.test" and self.command == "POST"
                and not parsed.query and parsed.path in {
                    "/internal/task-delivery/invitations", "/internal/task-delivery/backchannel",
                }
            )
            if (not self.server.private and host == "console.saas.test"
                    and re.fullmatch(r"/api/apps/[^/]+/authorize", decoded)):
                return self.fail(404)
            if (not self.server.private and host == "console.saas.test"
                    and decoded == '/api/internal/notifications/context-proof'):
                return self.fail(404)
            if internal != self.server.private and not readiness_probe and not resource_authorization and not notification_proof:
                return self.fail(404)
            if self.server.private and host == "customer.saas.test":
                return self.fail(404)
            if (self.server.private and host == "auth.saas.test" and not readiness_probe
                    and not task_delivery and not identity_private_allowed(self.command, parsed.path)):
                return self.fail(404)
            business_port = public_business_target(self.command, host, parsed.path, parsed.query,
                                                    self.server.private, business_enabled)
            if (business_enabled and not self.server.private and host == 'console.saas.test'
                    and parsed.path.startswith(('/stores-api/', '/work-orders-api/', '/socket.io'))
                    and business_port is None):
                return self.fail(404)
            if business_enabled and not self.server.private and host == 'console.saas.test' and parsed.path.startswith('/sub/'):
                if self.command not in ('GET', 'HEAD'):
                    return self.fail(405)
                target = business_static_target(static_workspace, parsed.path)
                return self.static_file(target) if target else self.fail(404)
            if business_port == 23003 and self.headers.get('Upgrade'):
                return self.websocket(business_port, parsed.query)
            if (not self.server.private and host in ("console.saas.test", "customer.saas.test")
                    and business_port is None
                    and not parsed.path.startswith(("/auth/", "/api/", "/micro-proxy/"))
                    and parsed.path not in ("/auth", "/api", "/health", "/ready")):
                return self.static(host, parsed.path)
            if self.headers.get("Transfer-Encoding"):
                return self.fail(400)
            try:
                length = int(self.headers.get("Content-Length", "0"))
            except ValueError:
                return self.fail(400)
            if not 0 <= length <= 1024 * 1024:
                return self.fail(413)
            body = self.rfile.read(length) if length else None
            headers = proxy_request_headers(self.headers.items(), hop_headers)
            headers.update({
                "X-Forwarded-Proto": "https", "X-Forwarded-Host": self.headers["Host"],
                "X-Forwarded-For": "127.0.0.1",
            })
            identity_port = token_upstreams.select(self.command, host, parsed.path, parsed.query, self.server.private)
            selected_port = identity_port if identity_port is not None else 23443 if task_delivery else business_port or destinations[host]
            upstream = http.client.HTTPConnection("127.0.0.1", selected_port, timeout=15)
            try:
                target_path = "/" + parsed.path.rsplit("/", 1)[1] if task_delivery else self.path
                upstream.request(self.command, target_path, body=body, headers=headers)
                response = upstream.getresponse()
                data = response.read(16 * 1024 * 1024 + 1)
                if len(data) > 16 * 1024 * 1024:
                    return self.fail(502)
                self.send_response(response.status)
                for key, value in proxy_response_headers(response.getheaders(), hop_headers, identity_port):
                    self.send_header(key, value)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                if self.command != "HEAD":
                    self.wfile.write(data)
            except (OSError, http.client.HTTPException):
                self.fail(502)
            finally:
                upstream.close()

        def websocket(self, port, query):
            parameters = urllib.parse.parse_qs(query, keep_blank_values=True)
            try:
                key = self.headers.get('Sec-WebSocket-Key', '')
                valid_key = len(base64.b64decode(key, validate=True)) == 16
            except (ValueError, TypeError):
                valid_key = False
            if (self.command != 'GET' or self.headers.get('Transfer-Encoding')
                    or self.headers.get('Content-Length', '0') != '0'
                    or self.headers.get('Upgrade', '').lower() != 'websocket'
                    or self.headers.get('Connection', '').strip().lower() != 'upgrade'
                    or self.headers.get('Sec-WebSocket-Version') != '13' or not valid_key
                    or len(self.headers.get_all('Sec-WebSocket-Key', [])) != 1
                    or parameters.get('EIO') != ['4'] or parameters.get('transport') != ['websocket']):
                return self.fail(400)
            if not self.server.websocket_slots.acquire(blocking=False):
                return self.fail(503)
            upstream = None
            response_sent = False
            self.close_connection = True
            try:
                headers = proxy_request_headers(self.headers.items(), hop_headers)
                if any(not re.fullmatch(r"[!#$%&'*+.^_`|~0-9A-Za-z-]+", key)
                       or '\r' in value or '\n' in value for key, value in headers.items()):
                    return self.fail(400)
                headers.update({'Connection': 'Upgrade', 'Upgrade': 'websocket',
                    'X-Forwarded-Proto': 'https', 'X-Forwarded-Host': self.headers['Host'],
                    'X-Forwarded-For': '127.0.0.1'})
                upstream = socket.create_connection(('127.0.0.1', port), timeout=5)
                upstream.settimeout(5)
                request = self.command + ' ' + self.path + ' HTTP/1.1\r\n'
                request += ''.join(key + ': ' + value + '\r\n' for key, value in headers.items()) + '\r\n'
                upstream.sendall(request.encode('iso-8859-1'))
                raw, status, response_headers = websocket_response(upstream)
                if status != 101:
                    # A rejected upgrade is never a raw HTTP tunnel: preserve
                    # the real bounded response, then close both connections.
                    if response_headers.get('Transfer-Encoding'):
                        return self.fail(502)
                    size = int(response_headers.get('Content-Length', '0'))
                    if not 0 <= size <= 1024 * 1024:
                        return self.fail(502)
                    data = bytearray()
                    deadline = time.monotonic() + 5
                    while len(data) < size:
                        remaining = deadline - time.monotonic()
                        if remaining <= 0:
                            raise TimeoutError('PHASE25_GATEWAY_REJECTED_UPGRADE_BODY_DEADLINE')
                        upstream.settimeout(min(5, remaining))
                        chunk = upstream.recv(min(65536, size - len(data)))
                        if not chunk:
                            raise ValueError('PHASE25_GATEWAY_REJECTED_UPGRADE_BODY_TRUNCATED')
                        data.extend(chunk)
                    self.connection.sendall(raw + data)
                    response_sent = True
                    return
                self.connection.settimeout(5)
                self.connection.sendall(raw)
                response_sent = True
                relay_websocket(self.connection, upstream)
            except (OSError, ValueError, http.client.HTTPException):
                if not response_sent:
                    self.fail(502)
            finally:
                if upstream is not None:
                    upstream.close()
                self.server.websocket_slots.release()

        def static(self, host, url_path):
            frontend = (
                "saas-customer-portal/frontend/dist" if host == "customer.saas.test"
                else "saas-plateform/frontend/dist"
            )
            base = (static_workspace / frontend).resolve()
            target = (base / urllib.parse.unquote(url_path).lstrip("/")).resolve()
            if not target.is_relative_to(base):
                return self.fail(404)
            if not target.is_file():
                target = base / "index.html"
            if not target.is_file():
                return self.fail(503)
            return self.static_file(target)

        def static_file(self, target):
            data = target.read_bytes()
            self.send_response(200)
            self.send_header("Content-Type", mimetypes.guess_type(target.name)[0] or "application/octet-stream")
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            if self.command != "HEAD":
                self.wfile.write(data)

        do_GET = do_POST = do_PUT = do_PATCH = do_DELETE = do_OPTIONS = do_HEAD = route

    class Server(http.server.ThreadingHTTPServer):
        daemon_threads = True
        request_queue_size = 64
        allow_reuse_address = True
        websocket_slots = threading.BoundedSemaphore(8)

        def get_request(self):
            connection, address = super().get_request()
            connection.settimeout(20)
            return connection, address

    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    # These explicitly supplied local test certificates must already be trusted
    # by the browser. No certificate trust or TLS verification is disabled here.
    context.load_cert_chain(workspace / ".local-run/server.pem", workspace / ".local-run/server.key")
    servers = []
    try:
        for port, private in ((443, False), (24444, True)):
            server = Server(("127.0.0.2", port), Gateway)
            server.private = private
            server.socket = context.wrap_socket(server.socket, server_side=True)
            servers.append(server)
            threading.Thread(target=server.serve_forever, daemon=True).start()
        print("TASK_HTTPS_READY public=127.0.0.2:443 private=127.0.0.2:24444", flush=True)
        (stop_event or threading.Event()).wait()
    except KeyboardInterrupt:
        pass
    finally:
        for server in servers:
            server.shutdown()
            server.server_close()


if __name__ == "__main__":
    main()
