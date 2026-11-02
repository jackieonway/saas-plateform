"""Strict TLS transport for the isolated issuer and independent standard RP.

This task gateway supplies no identity claims, login shortcuts or token results.
Every public request reaches the actual issuer or the delivered Node BFF.
"""

import argparse
import http.client
import http.server
import importlib.util
from pathlib import Path
import ssl
import threading
from urllib.parse import unquote, urlsplit

base_gateway = Path(__file__).with_name("base-gateway.py")
if not base_gateway.is_file():
    base_gateway = Path(__file__).with_name("gateway.py")
spec = importlib.util.spec_from_file_location("phase21_base_gateway", base_gateway)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
identity_private_allowed = module.identity_private_allowed


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--task-dir", type=Path, required=True)
    parser.add_argument("--workspace", type=Path, required=True)
    parser.add_argument("--static-workspace", type=Path)
    args = parser.parse_args()
    task = args.task_dir.resolve(strict=True)
    if (task != Path("/tmp/rust-saas-unified-auth-20261028")
            or not (task / "OWNER").is_file() or not (task / "https-database.json").is_file()):
        raise SystemExit("Owned Phase 21 fixture required")
    targets = {"auth.saas.test": 23441, "customer.saas.test": 23101, "console.saas.test": 23001}
    hop = {"connection", "keep-alive", "proxy-authenticate", "proxy-authorization", "te",
           "trailer", "transfer-encoding", "upgrade", "x-forwarded-for", "x-forwarded-host", "x-forwarded-proto"}

    class Gateway(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_args):
            pass

        def fail(self, status):
            self.send_response(status)
            self.send_header("Content-Length", "0")
            self.end_headers()

        def route(self):
            host = self.headers.get("Host", "").removesuffix(":24444" if self.server.private else ":443")
            parsed = urlsplit(self.path)
            if (host not in targets or parsed.scheme or parsed.netloc
                    or not parsed.path.startswith("/") or "\\" in parsed.path):
                return self.fail(404)
            internal = unquote(parsed.path).split("/")[1] == "internal"
            if self.server.private:
                if (host != "auth.saas.test" or parsed.query or not (
                        self.command in ("GET", "HEAD") and parsed.path == "/health/ready"
                        or internal and identity_private_allowed(self.command, parsed.path))):
                    return self.fail(404)
            elif internal:
                return self.fail(404)
            if self.headers.get("Transfer-Encoding"):
                return self.fail(400)
            try:
                length = int(self.headers.get("Content-Length", "0"))
            except ValueError:
                return self.fail(400)
            if not 0 <= length <= 65536:
                return self.fail(413)
            body = self.rfile.read(length) if length else None
            headers = {key: value for key, value in self.headers.items() if key.lower() not in hop}
            headers.update({"X-Forwarded-Proto": "https", "X-Forwarded-Host": host,
                            "X-Forwarded-For": "127.0.0.1"})
            port = 23103 if host == "customer.saas.test" and parsed.path == "/demo-resource" else targets[host]
            upstream = http.client.HTTPConnection("127.0.0.1", port, timeout=15)
            try:
                upstream.request(self.command, self.path, body, headers)
                response = upstream.getresponse()
                data = response.read(1024 * 1024 + 1)
                if len(data) > 1024 * 1024:
                    return self.fail(502)
                self.send_response(response.status)
                for key, value in response.getheaders():
                    if key.lower() not in hop | {"content-length", "server", "date"}:
                        self.send_header(key, value)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                if self.command != "HEAD":
                    self.wfile.write(data)
            except (OSError, http.client.HTTPException):
                self.fail(502)
            finally:
                upstream.close()

        do_GET = do_POST = do_HEAD = do_OPTIONS = do_DELETE = do_PUT = route

    class Server(http.server.ThreadingHTTPServer):
        daemon_threads = True
        request_queue_size = 64
        allow_reuse_address = True

        def get_request(self):
            connection, address = super().get_request()
            connection.settimeout(20)
            return connection, address

    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.load_cert_chain(args.workspace / ".local-run/server.pem", args.workspace / ".local-run/server.key")
    servers = []
    try:
        for port, private in ((443, False), (24444, True)):
            server = Server(("127.0.0.2", port), Gateway)
            server.private = private
            server.socket = context.wrap_socket(server.socket, server_side=True)
            servers.append(server)
            threading.Thread(target=server.serve_forever, daemon=True).start()
        print("PHASE21_TASK_TLS_READY exact_hosts=3 private=identity_only", flush=True)
        threading.Event().wait()
    finally:
        for server in servers:
            server.server_close()


if __name__ == "__main__":
    main()
