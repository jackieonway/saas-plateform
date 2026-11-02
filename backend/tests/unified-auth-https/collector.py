"""Owned in-memory HTTPS-peer fixture, reached through the task TLS gateway.

Control stays on WSL loopback behind an independent secret. Neither requests,
delivery material, control responses nor headers are logged or written to disk.
"""
import argparse
import base64
import hashlib
import hmac
import http.server
import json
import os
import pathlib
import re
import signal
import threading
import urllib.parse
import uuid


def control_authorized(values, secret):
    return (isinstance(values, list) and len(values) == 1
            and isinstance(values[0], str) and isinstance(secret, str)
            and hmac.compare_digest(values[0].encode(), secret.encode()))


def logout_evidence(messages, request):
    """Read successful in-memory delivery metadata; this is not RSA validation."""
    if (not isinstance(request, dict) or set(request) != {'expectedKid', 'publicSidSha256'}
            or not isinstance(request['expectedKid'], str)
            or not re.fullmatch(r'[A-Za-z0-9_-]{1,80}', request['expectedKid'])
            or not isinstance(request['publicSidSha256'], str)
            or not re.fullmatch(r'[0-9a-f]{64}', request['publicSidSha256'])):
        raise ValueError('bounded logout evidence request required')

    def unique_object(pairs):
        result = {}
        for name, value in pairs:
            if name in result:
                raise ValueError('duplicate JWT member')
            result[name] = value
        return result

    def segment(value):
        if not value or not re.fullmatch(r'[A-Za-z0-9_-]+', value):
            raise ValueError('canonical JWT segment required')
        decoded = base64.urlsafe_b64decode(value + '=' * (-len(value) % 4))
        if base64.urlsafe_b64encode(decoded).rstrip(b'=').decode() != value:
            raise ValueError('canonical JWT segment required')
        return decoded

    count = 0
    for message in messages[-32:]:
        try:
            if not isinstance(message, dict) or message.get('kind') != 'LOGOUT':
                continue
            raw = message['body']['logout_token']
            if not isinstance(raw, str) or not 0 < len(raw) <= 65536:
                continue
            parts = raw.split('.')
            if len(parts) != 3:
                continue
            header = json.loads(segment(parts[0]).decode('utf-8'), object_pairs_hook=unique_object)
            payload = json.loads(segment(parts[1]).decode('utf-8'), object_pairs_hook=unique_object)
            segment(parts[2])
            if (not isinstance(header, dict) or not isinstance(payload, dict)
                    or header.get('kid') != request['expectedKid']
                    or header.get('alg') != 'RS256' or header.get('typ') != 'logout+jwt'
                    or not isinstance(payload.get('sid'), str) or not 0 < len(payload['sid']) <= 2048
                    or type(payload.get('iat')) is not int or type(payload.get('exp')) is not int
                    or payload['exp'] <= payload['iat']
                    or payload.get('events') != {'http://schemas.openid.net/event/backchannel-logout': {}}):
                continue
            digest = hashlib.sha256(payload['sid'].encode()).hexdigest()
            if hmac.compare_digest(digest, request['publicSidSha256']):
                count += 1
        except (ValueError, TypeError, KeyError, UnicodeError, RecursionError):
            continue
    return {'matched': count > 0, 'count': count, 'kid': request['expectedKid'] if count else None}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--task-dir", type=pathlib.Path, required=True)
    args = parser.parse_args()
    root = args.task_dir.resolve(strict=True)
    if root != pathlib.Path("/tmp/rust-saas-unified-auth-20261028") or not (root / "OWNER").is_file():
        raise SystemExit("Owned task directory required")
    config = json.loads((root / "https-environment.json").read_text())
    phase = config.get("phase20", {})
    if not phase.get("enabled") or not (root / "namespace.pid").exists():
        raise SystemExit("Prepared Phase 20 namespace required")
    secret = phase["collector"]["controlSecret"]
    if not isinstance(secret, str) or len(secret) != 64:
        raise SystemExit("Independent collector control secret required")
    recipient = phase["recipientRef"]
    kinds = ("INVITATION", "LOGOUT")
    lock = threading.Lock()
    modes = {kind: {"status": 200, "delayMs": 0} for kind in kinds}
    counts = {field: {kind: 0 for kind in kinds} for field in ("attempts", "delivered", "failures")}
    messages = []
    stopping = threading.Event()

    class Handler(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_args):
            pass

        def reply(self, status, body=None):
            data = json.dumps(body if body is not None else {"accepted": status == 200}).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Cache-Control", "no-store")
            self.send_header("Connection", "close")
            self.end_headers()
            self.close_connection = True
            try:
                self.wfile.write(data)
            except OSError:
                pass

        def body(self):
            if self.headers.get("Transfer-Encoding"):
                raise ValueError("bounded body required")
            length = int(self.headers.get("Content-Length", "0"))
            if not 0 < length <= 65536:
                raise ValueError("bounded body required")
            return self.rfile.read(length)

        def controlled(self):
            values = self.headers.get_all("X-Task-Control-Key") or []
            return control_authorized(values, secret)

        def do_GET(self):
            if self.path != "/control/stats" or not self.controlled():
                return self.reply(404)
            with lock:
                data = json.loads(json.dumps(counts))
            self.reply(200, data)

        def do_POST(self):
            try:
                if self.path.startswith("/control"):
                    if not self.controlled():
                        return self.reply(404)
                    data = json.loads(self.body())
                    if not isinstance(data, dict):
                        return self.reply(400)
                    if self.path == "/control":
                        if (set(data) != {"operation", "kind", "status", "delayMs"}
                                or data["operation"] != "setMode" or data["kind"] not in kinds
                                or type(data["status"]) is not int or data["status"] not in (200, 400, 503)
                                or type(data["delayMs"]) is not int or not 0 <= data["delayMs"] <= 3000):
                            return self.reply(400)
                        with lock:
                            modes[data["kind"]] = {"status": data["status"], "delayMs": data["delayMs"]}
                        return self.reply(200)
                    if self.path == "/control/peek":
                        if set(data) != {"kind", "invitationId"} or data["kind"] != "INVITATION":
                            return self.reply(400)
                        with lock:
                            found = next((item["body"] for item in reversed(messages)
                                if item["kind"] == "INVITATION" and item["body"].get("invitationId") == data["invitationId"]), None)
                        return self.reply(200, {"message": found}) if found else self.reply(404)
                    if self.path == "/control/logout-evidence":
                        with lock:
                            snapshot = list(messages)
                        return self.reply(200, logout_evidence(snapshot, data))
                    return self.reply(404)
                kind = {"/invitations": "INVITATION", "/backchannel": "LOGOUT"}.get(self.path)
                if kind is None:
                    return self.reply(404)
                raw = self.body()
                if kind == "INVITATION":
                    body = json.loads(raw)
                    if (not isinstance(body, dict) or body.get("contractVersion") != "account-invitation-delivery/1"
                            or body.get("recipientRef") != recipient or body.get("purpose") != "ACCOUNT_ONBOARDING"
                            or not isinstance(body.get("token"), str) or len(body["token"]) > 8192):
                        return self.reply(400)
                    if not isinstance(body.get("invitationId"), str):
                        return self.reply(400)
                    invitation = uuid.UUID(body["invitationId"])
                    if invitation.version != 7 or str(invitation) != body["invitationId"]:
                        return self.reply(400)
                else:
                    pairs = urllib.parse.parse_qsl(raw.decode(), strict_parsing=True)
                    if len(pairs) != 1 or pairs[0][0] != "logout_token" or not pairs[0][1]:
                        return self.reply(400)
                    body = {"logout_token": pairs[0][1]}
                with lock:
                    mode = dict(modes[kind])
                    counts["attempts"][kind] += 1
                stopping.wait(mode["delayMs"] / 1000)
                with lock:
                    counts["delivered" if mode["status"] == 200 else "failures"][kind] += 1
                    if mode["status"] == 200:
                        messages.append({"kind": kind, "body": body})
                        del messages[:-32]
                return self.reply(mode["status"])
            except (ValueError, TypeError, KeyError, UnicodeError):
                return self.reply(400)

    class Server(http.server.ThreadingHTTPServer):
        daemon_threads = True
        request_queue_size = 16

        def get_request(self):
            connection, address = super().get_request()
            connection.settimeout(5)
            return connection, address

    server = Server(("127.0.0.1", 23443), Handler)
    record = root / "collector.pid"
    if record.exists():
        server.server_close()
        raise SystemExit("Collector already has an ownership record")
    pid = os.getpid()
    start = pathlib.Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[19]
    record.write_text(json.dumps({"pid": pid, "startTime": start, "task": str(root)}))
    record.chmod(0o600)
    signal.signal(signal.SIGINT, lambda *_: stopping.set())
    signal.signal(signal.SIGTERM, lambda *_: stopping.set())

    def lease():
        while not stopping.wait(0.5):
            if not (root / "namespace.pid").exists() or (root / "collector.stop").exists():
                stopping.set()
        server.shutdown()

    threading.Thread(target=lease, daemon=True).start()
    try:
        print("TASK_IN_MEMORY_DELIVERY_COLLECTOR_READY loopback=23443", flush=True)
        server.serve_forever(poll_interval=0.2)
    finally:
        stopping.set()
        server.server_close()
        with lock:
            messages.clear()
        record.unlink(missing_ok=True)
        print("TASK_IN_MEMORY_DELIVERY_COLLECTOR_CLEANUP_DONE", flush=True)


if __name__ == "__main__":
    main()
