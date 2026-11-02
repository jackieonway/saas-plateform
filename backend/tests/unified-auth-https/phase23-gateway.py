"""Three core HTTPS origins plus a separate port for the real standard RP.

The same validated certificate covers customer.saas.test at both ports. This
gateway never supplies authentication results, claims or database shortcuts.
"""
import argparse
import http.client
import http.server
import signal
import ssl
import threading
import importlib.util
from pathlib import Path
from urllib.parse import unquote, urlsplit

import gateway


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--task-dir', type=Path, required=True)
    parser.add_argument('--workspace', type=Path, required=True)
    parser.add_argument('--static-workspace', type=Path)
    args = parser.parse_args()
    task = args.task_dir.resolve(strict=True)
    if (task != Path('/tmp/rust-saas-unified-auth-20261028')
            or not (task / 'OWNER').is_file() or not (task / 'https-database.json').is_file()):
        raise SystemExit('PHASE23_OWNED_SOURCE_REQUIRED')
    stop = threading.Event()
    failed = threading.Event()
    original = gateway.identity_private_allowed
    gateway.identity_private_allowed = lambda method, path: (
        method == 'GET' and path == '/internal/identity/metrics') or original(method, path)

    def base():
        try:
            gateway.main(stop, identity_token_ports=(23441, 23442))
        except BaseException:
            failed.set()
            stop.set()

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
            parsed = urlsplit(self.path)
            if (self.headers.get('Host') != 'customer.saas.test:24446'
                    or parsed.scheme or parsed.netloc or not parsed.path.startswith('/')
                    or '\\' in parsed.path):
                return self.fail(404)
            decoded = unquote(parsed.path)
            if decoded == '/internal' or decoded.startswith('/internal/'):
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
            headers.update({'X-Forwarded-Proto': 'https', 'X-Forwarded-Host': 'customer.saas.test:24446',
                            'X-Real-IP': '127.0.0.1'})
            target = 23103 if parsed.path == '/demo-resource' else 23102
            upstream = http.client.HTTPConnection('127.0.0.1', target, timeout=10)
            try:
                upstream.request(self.command, self.path, body, headers)
                response = upstream.getresponse()
                data = response.read(1024 * 1024 + 1)
                if len(data) > 1024 * 1024:
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

        do_GET = do_POST = do_HEAD = do_OPTIONS = do_DELETE = do_PUT = route

    class Server(http.server.ThreadingHTTPServer):
        daemon_threads = True
        request_queue_size = 8
        allow_reuse_address = True

        def get_request(self):
            connection, address = super().get_request()
            connection.settimeout(10)
            return connection, address

    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.load_cert_chain(args.workspace / '.local-run/server.pem', args.workspace / '.local-run/server.key')
    server = Server(('127.0.0.2', 24446), Handler)
    server.socket = context.wrap_socket(server.socket, server_side=True)
    base_thread = threading.Thread(target=base, daemon=True)
    rp_thread = threading.Thread(target=server.serve_forever, daemon=True)
    business = None
    if (task / 'phase25-business-manifest.json').exists():
        spec = importlib.util.spec_from_file_location('phase25_business_gateway', Path(__file__).with_name('phase25-business-gateway.py'))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        business = module.create_server(task, args.workspace)
    for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(sig, lambda *_args: stop.set())
    try:
        base_thread.start()
        rp_thread.start()
        if business:
            threading.Thread(target=business.serve_forever, daemon=True).start()
        print('PHASE23_TLS_RP_READY port=24446', flush=True)
        stop.wait()
    finally:
        if business:
            business.shutdown()
            business.server_close()
        stop.set()
        server.shutdown()
        server.server_close()
        base_thread.join(timeout=8)
        rp_thread.join(timeout=2)
    if failed.is_set() or base_thread.is_alive():
        raise SystemExit('PHASE23_TLS_GATEWAY_FAILED')


if __name__ == '__main__':
    main()
