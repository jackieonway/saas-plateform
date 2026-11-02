"""A routing-only TLS listener for the owned full-version business validation.

No authentication claims are generated here. Business servers validate the
actual capability through the private platform port and canonical identity.
"""
import http.client
import http.server
import json
import ssl
from urllib.parse import unquote, urlsplit


def business_target(method, raw_path):
    parsed = urlsplit(raw_path)
    if parsed.scheme or parsed.netloc or not parsed.path.startswith('/') or '\\' in parsed.path:
        return None
    decoded = unquote(parsed.path)
    if '\\' in decoded or '/internal' in decoded or any(part in ('.', '..') for part in decoded.split('/')):
        return None
    if parsed.path.startswith('/stores-api/'):
        return 23031
    if parsed.path.startswith('/work-orders-api/'):
        return 23004
    if parsed.path == '/socket.io/' and method in ('GET', 'POST', 'OPTIONS'):
        return 23003
    if parsed.path in ('/health/live', '/health/ready') and method in ('GET', 'HEAD') and not parsed.query:
        return 23003
    return None


def create_server(task, workspace):
    # The root environment controller has already checked the same manifest.
    # This additional check prevents accidental exposure from old fixtures.
    manifest = json.loads((task / 'phase25-business-manifest.json').read_text())
    if manifest.get('contractVersion') != 'unified-auth-phase25-business-manifest/1':
        raise RuntimeError('PHASE25_BUSINESS_GATEWAY_BINDING_REQUIRED')
    hop = {'connection', 'keep-alive', 'proxy-authenticate', 'proxy-authorization',
           'te', 'trailer', 'transfer-encoding', 'upgrade', 'x-forwarded-for',
           'x-forwarded-host', 'x-forwarded-proto', 'x-real-ip'}

    class Handler(http.server.BaseHTTPRequestHandler):
        protocol_version = 'HTTP/1.1'
        disable_nagle_algorithm = True

        def log_message(self, *_args):
            pass

        def fail(self, status):
            self.send_response(status)
            self.send_header('Content-Length', '0')
            self.send_header('Cache-Control', 'no-store')
            self.end_headers()

        def route(self):
            if self.headers.get('Host') != 'console.saas.test:24447':
                return self.fail(404)
            target = business_target(self.command, self.path)
            if target is None:
                return self.fail(404)
            if self.headers.get('Transfer-Encoding'):
                return self.fail(400)
            try:
                size = int(self.headers.get('Content-Length', '0'))
            except ValueError:
                return self.fail(400)
            if not 0 <= size <= 65536:
                return self.fail(413)
            body = self.rfile.read(size) if size else None
            headers = {key: value for key, value in self.headers.items() if key.lower() not in hop}
            headers.update({'X-Forwarded-Proto': 'https', 'X-Forwarded-Host': 'console.saas.test:24447',
                            'X-Real-IP': '127.0.0.1'})
            upstream = http.client.HTTPConnection('127.0.0.1', target, timeout=20)
            try:
                upstream.request(self.command, self.path, body, headers)
                response = upstream.getresponse()
                data = response.read(2 * 1024 * 1024 + 1)
                if len(data) > 2 * 1024 * 1024:
                    return self.fail(502)
                self.send_response(response.status)
                for key, value in response.getheaders():
                    if key.lower() not in hop | {'content-length', 'server', 'date'}:
                        self.send_header(key, value)
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                if self.command != 'HEAD':
                    self.wfile.write(data)
            except (OSError, http.client.HTTPException):
                self.fail(502)
            finally:
                upstream.close()

        do_GET = do_POST = do_HEAD = do_OPTIONS = do_PUT = do_DELETE = do_PATCH = route

    class Server(http.server.ThreadingHTTPServer):
        daemon_threads = True
        request_queue_size = 16
        allow_reuse_address = True

        def get_request(self):
            connection, address = super().get_request()
            connection.settimeout(25)
            return connection, address

    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.load_cert_chain(workspace / '.local-run/server.pem', workspace / '.local-run/server.key')
    server = Server(('127.0.0.2', 24447), Handler)
    server.socket = context.wrap_socket(server.socket, server_side=True)
    return server
