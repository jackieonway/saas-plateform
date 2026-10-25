"""Synthetic offline HTTP contracts; no network, database or service operations."""
import copy
import hashlib
import hmac
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest
import urllib.parse
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("u06_http", Path(__file__).parents[1] / "validate-uuid-u06-runtime.py")
V = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(V)
CONTEXT = {"userId": "019a1163-0000-7000-8000-000000000001", "tenantId": "019a1163-0000-7000-8000-000000000002", "organizationId": "019a1163-0000-7000-8000-000000000003"}


def secret(i):
    return "isolated_private_synthetic_" + str(i).zfill(40)


def jwt(secret_value, **claims):
    now = int(time.time())
    claims = {"iat": now, "exp": now + 600, "sid": CONTEXT["userId"], "jti": CONTEXT["userId"], "ver": 1, **claims}
    parts = [V.U05.b64encode(json.dumps(v).encode()) for v in ({"alg": "HS256"}, claims)]
    signed = ".".join(parts)
    return signed + "." + V.U05.b64encode(hmac.new(secret_value.encode(), signed.encode(), hashlib.sha256).digest())


class SyntheticHTTP(V.Runner):
    def call(self, method, path, *, body=None, form=None, bearer=None, basic=None):
        self.transcript.append({"method": method, "path": path, "body": body, "form": form, "bearer": bearer, "basic": basic})
        result = {"status": 200, "json": {}, "headers": {}}
        def token(kind, client=None, audience=None):
            return jwt(self.new_secret, typ=kind, sub=CONTEXT["userId"], tenant=CONTEXT["tenantId"], org=CONTEXT["organizationId"], **({"cid": client} if client else {}), **({"aud": audience} if audience else {}))
        if path == "/api/auth/login":
            result["json"] = {"ticket": token("credential")}
        elif path == "/api/auth/context":
            result["json"] = {"user": {"id": CONTEXT["userId"]}, "tenant": {"id": CONTEXT["tenantId"]}, "organization": {"id": CONTEXT["organizationId"]}, "accessToken": token("session"), "refreshToken": secret(600)}
        elif path.startswith("/api/oauth/clients?"):
            result["json"] = [{"id": CONTEXT["userId"], "clientId": c["clientId"], "clientType": c["clientType"], "status": "DISABLED" if k == "disabled" else "ENABLED"} for k, c in self.clients.items()]
        elif path == "/oauth/introspect":
            client = next((c for k, c in self.clients.items() if c["clientId"] == basic[0]), None)
            disabled = basic[0] == self.clients["disabled"]["clientId"]
            if disabled or client is None or client.get("clientSecret") != basic[1]:
                result.update(status=401, json={"error": "invalid_client"})
            elif form["token"].count(".") == 2:
                claims = V.U05.decode_jwt(form["token"], self.new_secret)
                result["json"] = {"active": claims["cid"] == basic[0], "sub": CONTEXT["userId"], "jti": CONTEXT["userId"]}
            else:
                result["json"] = {"active": False}
        elif path == "/api/oauth/session-token":
            app = next(a for a in self.config["applications"] if a["code"] == body["appCode"])
            result["json"] = {"accessToken": token("access", self.clients[app["clientKey"]]["clientId"], app["code"])}
        elif path.startswith("/api/apps/"):
            code, endpoint = path.split("/")[3:]
            app = next(a for a in self.config["applications"] if a["code"] == code)
            if endpoint == "context":
                claims = V.U05.decode_jwt(bearer, self.new_secret)
                if claims["aud"] != code:
                    result.update(status=401)
                else:
                    result["json"] = {"appCode": code, "application": {"id": app["applicationId"]}, "user": {"id": CONTEXT["userId"]}, "tenant": {"id": CONTEXT["tenantId"]}, "organization": {"id": CONTEXT["organizationId"]}}
            else:
                result["json"] = {"contractVersion": "1.0", "introspection": {"active": True}, "context": {"organization": {"id": CONTEXT["organizationId"]}}}
        elif path.startswith("/oauth/authorize?"):
            self.authorization_form = urllib.parse.parse_qs(urllib.parse.urlsplit(path).query)
            result.update(status=302, headers={"location": "/consent?request_id=" + CONTEXT["userId"]})
        elif path.endswith("/decision") and path.startswith("/api/oauth/consent/"):
            query = urllib.parse.urlencode({"state": self.authorization_form["state"][0], "code": secret(len(self.transcript))})
            result["json"] = {"redirectTo": self.clients["oauth:auth"]["redirectUri"] + "?" + query}
        elif path == "/oauth/token":
            result["json"] = {"access_token": token("access", self.clients["oauth:auth"]["clientId"]), "refresh_token": secret(700)}
        elif path == "/oauth/userinfo":
            result["json"] = {"sub": CONTEXT["userId"]}
        elif path == "/oauth/device/authorization":
            result["json"] = {"device_code": secret(800), "user_code": "SYNTHETIC", "expires_in": 600, "interval": 5}
        elif path == "/api/auth/profile" or path.startswith("/api/device/") or path.startswith("/api/oauth/consent/"):
            pass
        else:
            raise AssertionError("Unexpected synthetic HTTP endpoint")
        return result


@unittest.skipUnless(sys.platform == "linux" and os.geteuid() == 0, "Linux root modes required")
class HttpContract(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.base = self.root / "u06"
        self.base.mkdir(mode=0o700)
        self.runtime_base = self.root / "runtime"
        self.handoff = {"format": "saas-uuid-u05-runtime/1", "platformOrigin": V.RUNTIME.ORIGIN, "login": {"username": "synthetic-account", "password": secret(999)}, "context": CONTEXT, "clients": {}, "applications": []}
        for i, code in enumerate(sorted(V.APP_CODES)):
            key = "app:" + code
            self.handoff["clients"][key] = {"clientId": "opaque-" + str(i), "clientType": "CONFIDENTIAL", "clientSecret": secret(i), "scope": "profile"}
            self.handoff["applications"].append({"code": code, "applicationId": CONTEXT["userId"], "clientKey": key, "context": CONTEXT, "resource": "read", "action": "list"})
        for i, key in enumerate(("oauth:auth", "oauth:cc", "extra:1", "extra:2", "disabled"), 7):
            self.handoff["clients"][key] = {"clientId": "opaque-" + str(i), "clientType": "CONFIDENTIAL", "clientSecret": secret(i), "scope": "profile"}
        self.handoff["clients"]["oauth:auth"]["redirectUri"] = "https://external-example.invalid/callback"
        self.handoff["clients"]["oauth:device"] = {"clientId": "opaque-public-device", "clientType": "PUBLIC", "scope": "profile"}
        self.handoff["clients"]["public:extra"] = {"clientId": "opaque-public-other", "clientType": "PUBLIC", "scope": "profile"}
        clients = {c["clientId"]: V.sha(c["clientSecret"].encode()) for c in self.handoff["clients"].values() if c["clientType"] == "CONFIDENTIAL"}
        handoff_path = self.private(self.root / "handoff.json", self.handoff)
        self.source_env = self.root / "platform.env"
        self.source_env.write_text("JWT_SECRET=" + secret(99) + "\n")
        self.source_env.chmod(0o600)
        for module, name, value in ((V, "BASE", self.base), (V, "HANDOFF", handoff_path), (V, "SOURCE_ENV", self.source_env), (V.RUNTIME, "BASE", self.runtime_base), (V.RUNTIME, "SOURCE_IDS_SHA256", V.sha(V.RUNTIME.canonical(sorted(clients)))), (V.RUNTIME, "SOURCE_CLIENT_HASHES_SHA256", V.sha(V.RUNTIME.canonical(clients))), (V.RUNTIME, "SOURCE_JWT_SHA256", V.sha(secret(99).encode()))):
            active = patch.object(module, name, value)
            active.start()
            self.addCleanup(active.stop)
        self.config = {"format": V.FORMAT, "generation": "source", "output_dir": str(self.base / "http-source-synthetic")}

    def private(self, path, value):
        path.write_text(json.dumps(value))
        path.chmod(0o600)
        return path

    def capture_source(self):
        runner = SyntheticHTTP(V.config_of(self.config))
        runner.run()
        return runner, runner.finish()

    def test_capture_creates_all_real_sample_shapes_without_callbacks_or_business_writes(self):
        runner, summary = self.capture_source()
        self.assertEqual(summary["status"], "PASS")
        self.assertEqual(summary["captured_sample_counts"]["app_access"], 7)
        self.assertEqual(len(runner.capture["samples"]), 14)
        self.assertEqual(summary["coverage"]["activeClientsAuthenticated"], 11)
        self.assertEqual(summary["coverage"]["disabledClientCorrectSecretRejected"], 1)
        self.assertTrue(runner.capture["unusedDevice"]["approvedUnused"])
        self.assertEqual(runner.capture["unusedCode"]["expiresAt"] - runner.capture["unusedCode"]["issuedAt"], 60)
        self.assertIsNotNone(V.U05.original_pkce_verifier(runner.capture["unusedCode"]))
        self.assertNotIn("isolated_private_synthetic", json.dumps(summary))
        for entry in runner.transcript:
            if "method" in entry:
                self.assertTrue(entry["path"].startswith(("/api/", "/oauth/")))
                self.assertEqual(urllib.parse.urlsplit(entry["path"]).netloc, "")
        for path in runner.output.iterdir():
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)

    def test_check_reads_all_source_inputs_but_never_creates_output_or_calls_http(self):
        config = self.private(self.root / "config.json", self.config)
        with patch.object(V.U05.Runner, "call", side_effect=AssertionError("network forbidden")):
            self.assertEqual(V.execute(str(config), True)["http_requests"], 0)
        self.assertFalse(Path(self.config["output_dir"]).exists())
        self.source_env.write_text("JWT_SECRET=changed\n")
        with self.assertRaises(V.U05.CheckFailed):
            V.execute(str(config), True)

    def test_original_v7_equal_identity_is_accepted_without_v4_old_state(self):
        runner, _ = self.capture_source()
        token = runner.session["accessToken"]
        self.assertEqual(runner.claims(token, "session", CONTEXT)["org"], CONTEXT["organizationId"])
        self.assertNotIn("oldStateFile", runner.config)
        wrong = {**CONTEXT, "organizationId": "019a1163-0000-7000-8000-000000000099"}
        with self.assertRaises(V.U05.CheckFailed):
            runner.claims(token, "session", wrong)

    def test_capture_input_requires_every_kind_and_original_pkce(self):
        runner, _ = self.capture_source()
        for mutate in (lambda c: c.update(samples=[s for s in c["samples"] if s["kind"] != "device_code"]), lambda c: next(s for s in c["samples"] if s["kind"] == "authorization_code").pop("codeVerifier"), lambda c: c.update(generation="final")):
            capture = copy.deepcopy(runner.capture)
            mutate(capture)
            with self.assertRaises(V.U05.CheckFailed):
                V.capture_input(capture, "source")

    def test_unknown_config_external_output_and_modified_client_commitment_rejected(self):
        for key, value in (("unknown", "value"), ("generation", "u05"), ("output_dir", "/tmp/public-output")):
            with self.assertRaises((V.U05.CheckFailed, V.RUNTIME.Invalid)):
                V.config_of({**self.config, key: value})
        changed = copy.deepcopy(self.handoff)
        changed["clients"]["disabled"]["clientSecret"] = secret(888)
        self.private(V.HANDOFF, changed)
        with self.assertRaises(V.U05.CheckFailed):
            V.Runner(self.config, create_output=False)

    def test_disabled_correct_secret_must_reject_and_never_count_as_active(self):
        runner = SyntheticHTTP(V.config_of(self.config))
        runner.login()
        original = runner.call
        def incorrect_disabled(method, path, **kw):
            response = original(method, path, **kw)
            if path == "/oauth/introspect" and kw.get("basic", (None,))[0] == runner.clients["disabled"]["clientId"]:
                return {"status": 200, "json": {"active": False}, "headers": {}}
            return response
        runner.call = incorrect_disabled
        with self.assertRaises(V.U05.CheckFailed):
            runner.client_authentication()

    def test_old_sample_replays_use_current_secret_for_digest_and_original_pkce(self):
        runner, _ = self.capture_source()
        runner.predecessor = copy.deepcopy(runner.capture)
        runner.old_secret, runner.new_secret = runner.new_secret, secret(1000)
        for sample in runner.predecessor["samples"]:
            sample["expiresAt"] = 1
        calls = []
        def rejected(method, path, **kwargs):
            calls.append((method, path, kwargs))
            if path == "/oauth/introspect":
                return {"status": 200, "json": {"active": False}}
            return {"status": 401 if method == "GET" or path == "/api/auth/context" else 400, "json": {"error": "invalid_token" if path == "/oauth/userinfo" else "invalid_grant"}}
        runner.call = rejected
        runner.replay_predecessor()
        self.assertEqual(runner.coverage["realPredecessorSamplesReplayed"]["app_access"], 7)
        self.assertEqual(sum(runner.coverage["predecessorSamplesExpiredAtReplay"].values()), 14)
        exchanged = next(args[2]["form"] for args in calls if args[2].get("form", {}).get("grant_type") == "authorization_code")
        self.assertEqual(exchanged["code_verifier"], runner.capture["unusedCode"]["codeVerifier"])
        introspection = next(args[2] for args in calls if args[1] == "/oauth/introspect")
        self.assertEqual(introspection["basic"][1], runner.clients["oauth:auth"]["clientSecret"])

    def test_all_twelve_old_client_secrets_are_actually_replayed(self):
        runner, _ = self.capture_source()
        runner.previous_clients = copy.deepcopy(runner.clients)
        calls = []
        def rejected(method, path, **kwargs):
            calls.append(kwargs)
            return {"status": 401, "json": {"error": "invalid_client"}}
        runner.call = rejected
        runner.reject_old_client_secrets()
        self.assertEqual(len(calls), 12)
        self.assertEqual(runner.coverage["oldConfidentialSecretsRejected"], 12)

    def test_old_application_tokens_require_401_and_recovery_replays_both_generations(self):
        runner, _ = self.capture_source()
        runner.predecessor = copy.deepcopy(runner.capture)
        runner.original_source = copy.deepcopy(runner.capture)
        runner.old_secret = runner.original_secret = runner.new_secret
        runner.new_secret = secret(1000)
        runner.generation = "recovery"
        def rejected(method, path, **kwargs):
            if path == "/oauth/introspect":
                return {"status": 200, "json": {"active": False}}
            return {"status": 401 if method == "GET" or path == "/api/auth/context" else 400, "json": {"error": "invalid_token" if path == "/oauth/userinfo" else "invalid_grant"}}
        runner.call = rejected
        runner.replay_predecessor()
        self.assertEqual(runner.coverage["originalSourceSamplesReplayed"]["app_access"], 7)
        self.assertEqual(runner.coverage["realPredecessorSamplesReplayed"]["app_access"], 7)
        original = runner.call
        def forbidden_instead_of_unauthorized(method, path, **kwargs):
            if path.startswith("/api/apps/"):
                return {"status": 403, "json": {}}
            return original(method, path, **kwargs)
        runner.call = forbidden_instead_of_unauthorized
        with self.assertRaises(V.U05.CheckFailed):
            runner.replay_predecessor()

    def test_only_successful_capture_summary_can_feed_the_next_generation(self):
        runner, summary = self.capture_source()
        path = runner.output / "capture.private.json"
        self.assertEqual(V.verified_capture(str(path), summary["capture_sha256"], "source"), runner.capture)
        summary["status"] = "FAIL"
        self.private(runner.output / "summary.json", summary)
        with self.assertRaises(V.U05.CheckFailed):
            V.verified_capture(str(path), summary["capture_sha256"], "source")


if __name__ == "__main__":
    unittest.main()
