#!/usr/bin/env python3
"""Capture/replay real U06 HTTP credentials without database or business writes."""
import argparse
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import sys
import time
import urllib.parse
import urllib.request


def imported(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


U05 = imported("uuid_u05_http", "validate-uuid-u05-runtime.py")
RUNTIME = imported("uuid_u06_materials", "prepare-uuid-u06-runtime.py")
require = U05.require
FORMAT = "saas-uuid-u06-http/1"
BASE = Path("/var/lib/saas-uuid-u06-20261024")
HANDOFF = Path("/var/lib/saas-uuid-u05-materials-20261024/credential-handoff.private.json")
SOURCE_ENV = Path("/var/lib/saas-uuid-u05-runtime-20261024/env/platform.env")
APP_CODES = {"stores", "work-orders", "vue-workshop", "legacy-jquery", "report-center", "oauth-playground", "websocket-chat"}
SAMPLE_KINDS = {"session", "platform_refresh", "app_access", "oauth_access", "oauth_refresh", "authorization_code", "device_code", "login_ticket"}


def sha(value):
    return hashlib.sha256(value).hexdigest()


def config_of(value, require_new_output=True):
    generation = value.get("generation") if isinstance(value, dict) else None
    require(generation in ("source", "final", "recovery"), "FIXED_GENERATION_REQUIRED")
    expected = {"format", "generation", "output_dir"}
    if generation != "source":
        expected |= {"source_capture_file", "source_capture_sha256"}
    if generation == "recovery":
        expected |= {"original_source_capture_file", "original_source_capture_sha256"}
    require(set(value) == expected and value["format"] == FORMAT, "CLOSED_U06_HTTP_CONFIG_REQUIRED")
    output = RUNTIME.path_checked(value["output_dir"])
    require(output.parent == BASE and re.fullmatch("http-" + generation + r"-[a-z0-9-]{1,60}", output.name), "FIXED_NEW_OUTPUT_REQUIRED")
    RUNTIME.private_directory(BASE)
    require(not require_new_output or not output.exists(), "NEW_HTTP_OUTPUT_REQUIRED")
    if generation != "source":
        RUNTIME.path_checked(value["source_capture_file"])
        RUNTIME.digest(value["source_capture_sha256"])
    if generation == "recovery":
        RUNTIME.path_checked(value["original_source_capture_file"])
        RUNTIME.digest(value["original_source_capture_sha256"])
    return value


def topology(value):
    require(value.get("format") == "saas-uuid-u05-runtime/1" and value.get("platformOrigin") == RUNTIME.ORIGIN, "FIXED_SOURCE_HANDOFF_REQUIRED")
    require(set(value.get("login", {})) == {"username", "password"} and all(isinstance(v, str) and v for v in value["login"].values()), "PRIVATE_LOGIN_REQUIRED")
    context = value.get("context", {})
    require(all(U05.valid_v7(context.get(k)) for k in ("userId", "tenantId", "organizationId")), "ORIGINAL_V7_CONTEXT_REQUIRED")
    applications = value.get("applications")
    require(isinstance(applications, list) and len(applications) == 7 and {a.get("code") for a in applications} == APP_CODES, "SEVEN_ORIGINAL_APPS_REQUIRED")
    for app in applications:
        require(U05.valid_v7(app.get("applicationId")) and app.get("clientKey") in value["clients"], "V7_APPLICATION_BINDING_REQUIRED")
        require(all(U05.valid_v7(app.get("context", context).get(k)) for k in ("userId", "tenantId", "organizationId")), "V7_BUSINESS_CONTEXT_REQUIRED")
    clients = value.get("clients", {})
    confidential = {c["clientId"]: c["clientSecret"] for c in clients.values() if c.get("clientType") == "CONFIDENTIAL"}
    require(len(confidential) == 12 and len(clients) == 14 and {"oauth:auth", "oauth:cc", "oauth:device"} <= set(clients), "ORIGINAL_CLIENT_TOPOLOGY_REQUIRED")
    require(sha(RUNTIME.canonical(sorted(confidential))) == RUNTIME.SOURCE_IDS_SHA256, "OPAQUE_CLIENT_IDS_MUST_BE_PRESERVED")
    return copy.deepcopy(value)


def read_environment(path):
    # Check the stronger U06 leaf/parent contract before consuming the existing
    # U05 dotenv parser. No shell expansion or process environment is involved.
    RUNTIME.private_bytes(str(path))
    return U05.read_env(str(path))


def capture_input(value, generation):
    required = {"format", "generation", "captured_at", "identity_sha256", "samples", "session", "applicationTokens", "clients_sha256", "client_statuses", "oauth", "unusedCode", "unusedDevice"}
    require(isinstance(value, dict) and set(value) == required and value["format"] == "saas-uuid-u06-capture/1" and value["generation"] == generation, "COMPLETE_PREDECESSOR_CAPTURE_REQUIRED")
    require(type(value["captured_at"]) is int and isinstance(value["samples"], list), "CAPTURE_TIME_AND_SAMPLES_REQUIRED")
    counts = {}
    for sample in value["samples"]:
        require(isinstance(sample, dict) and sample.get("kind") in SAMPLE_KINDS and isinstance(sample.get("value"), str) and sample["value"], "REAL_PREDECESSOR_SAMPLE_REQUIRED")
        counts[sample["kind"]] = counts.get(sample["kind"], 0) + 1
        if sample["kind"] == "authorization_code":
            require(U05.original_pkce_verifier(sample) is not None, "REAL_ORIGINAL_PKCE_REQUIRED")
    require(all(counts.get(k, 0) > 0 for k in SAMPLE_KINDS) and counts.get("app_access") == 7, "ALL_REAL_SAMPLE_KINDS_REQUIRED")
    return value


def verified_capture(path, expected_sha256, generation):
    value, _ = RUNTIME.checked_json(path, expected_sha256)
    capture = capture_input(value, generation)
    summary = RUNTIME.private_json(str(Path(path).parent / "summary.json"))
    require(summary.get("format") == "saas-uuid-u06-http-result/1" and summary.get("status") == "PASS" and summary.get("generation") == generation and summary.get("capture_sha256") == expected_sha256 and summary.get("identity_sha256") == capture["identity_sha256"], "SUCCESSFUL_PREDECESSOR_CAPTURE_RECEIPT_REQUIRED")
    return capture


class Runner(U05.Runner):
    """Reuse U05 transport and protocol checks, with original v7 identities."""
    def __init__(self, config, create_output=True):
        self.config = config
        self.generation = config["generation"]
        self.origin = RUNTIME.ORIGIN
        self.output = Path(config["output_dir"])
        self.transcript, self.checks, self.coverage = [], [], {}
        self.current, self.last_oauth = "INPUT_VALIDATION", 0.0
        self.http = urllib.request.build_opener(urllib.request.ProxyHandler({}), U05.NoRedirect())
        self.handoff = topology(RUNTIME.private_json(str(HANDOFF)))
        self.clients = self.handoff["clients"]
        self.source_clients = copy.deepcopy(self.clients)
        self.context = self.handoff["context"]
        self.identity = {"context": self.context, "applications": self.handoff["applications"]}
        self.identity_sha = sha(RUNTIME.canonical(self.identity))
        self.config = {**config, "login": self.handoff["login"], "applications": self.handoff["applications"]}
        if self.generation == "source":
            env = read_environment(SOURCE_ENV)
            self.new_secret = env["JWT_SECRET"]
            require(sha(self.new_secret.encode()) == RUNTIME.SOURCE_JWT_SHA256, "ORIGINAL_U05_JWT_REQUIRED")
            old_client_hashes = {c["clientId"]: sha(c["clientSecret"].encode()) for c in self.clients.values() if c["clientType"] == "CONFIDENTIAL"}
            require(sha(RUNTIME.canonical(old_client_hashes)) == RUNTIME.SOURCE_CLIENT_HASHES_SHA256, "ORIGINAL_U05_CLIENT_SECRETS_REQUIRED")
            self.old_secret = None
        else:
            current_root = RUNTIME.BASE / self.generation
            self.new_secret = read_environment(current_root / "env/platform.env")["JWT_SECRET"]
            prior_env = SOURCE_ENV if self.generation == "final" else RUNTIME.BASE / "final/env/platform.env"
            self.old_secret = read_environment(prior_env)["JWT_SECRET"]
            require(self.new_secret != self.old_secret, "JWT_EPOCH_MUST_CHANGE")
            epoch_root = BASE / ("epoch-" + self.generation)
            epoch = RUNTIME.private_json(str(epoch_root / "credential-epoch.json"))
            predecessor = RUNTIME.private_json(str(RUNTIME.BASE / "final/credential-epoch.json")) if self.generation == "recovery" else None
            RUNTIME.validate_epoch(epoch, self.generation, predecessor)
            private = RUNTIME.private_json(str(epoch_root / "client-secrets.private.json"))
            RUNTIME.fields(private, ("format", "generation", "clients"))
            require(private["format"] == "saas-uuid-u06-client-secrets/1" and private["generation"] == self.generation, "CURRENT_PRIVATE_CLIENT_EPOCH_REQUIRED")
            expected = {c["client_id"]: c["new_secret_hash"] for c in epoch["confidential_clients"]}
            require(isinstance(private["clients"], dict) and set(private["clients"]) == set(expected), "ALL_TWELVE_NEW_RAW_SECRETS_REQUIRED")
            require(all(isinstance(s, str) and sha(s.encode()) == expected[c] for c, s in private["clients"].items()) and sha(self.new_secret.encode()) == epoch["jwt_secret_sha256"], "NEW_RAW_SECRETS_EPOCH_MISMATCH")
            self.previous_clients = copy.deepcopy(self.clients)
            if self.generation == "recovery":
                final = RUNTIME.private_json(str(BASE / "epoch-final/client-secrets.private.json"))
                for client in self.previous_clients.values():
                    if client["clientType"] == "CONFIDENTIAL":
                        client["clientSecret"] = final["clients"][client["clientId"]]
            for client in self.clients.values():
                if client["clientType"] == "CONFIDENTIAL":
                    client["clientSecret"] = private["clients"][client["clientId"]]
            self.predecessor = verified_capture(config["source_capture_file"], config["source_capture_sha256"], "source" if self.generation == "final" else "final")
            require(self.predecessor["identity_sha256"] == self.identity_sha, "U06_MUST_PRESERVE_ORIGINAL_V7_IDENTITIES")
            old_client_hashes = {c["clientId"]: sha(c["clientSecret"].encode()) for c in self.previous_clients.values() if c["clientType"] == "CONFIDENTIAL"}
            require(self.predecessor["clients_sha256"] == sha(RUNTIME.canonical(old_client_hashes)), "RAW_PREDECESSOR_CLIENT_SECRET_COMMITMENT_REQUIRED")
            if self.generation == "recovery":
                self.original_source = verified_capture(config["original_source_capture_file"], config["original_source_capture_sha256"], "source")
                self.original_secret = read_environment(SOURCE_ENV)["JWT_SECRET"]
                require(self.original_source["identity_sha256"] == self.identity_sha and sha(self.original_secret.encode()) == RUNTIME.SOURCE_JWT_SHA256, "ORIGINAL_SOURCE_REPLAY_BINDING_REQUIRED")
                original_hashes = {c["clientId"]: sha(c["clientSecret"].encode()) for c in self.source_clients.values() if c["clientType"] == "CONFIDENTIAL"}
                require(self.original_source["clients_sha256"] == sha(RUNTIME.canonical(original_hashes)), "ORIGINAL_SOURCE_CLIENT_COMMITMENT_REQUIRED")
        if create_output:
            self.output.mkdir(mode=0o700)
            self.output.chmod(0o700)

    def claims(self, token, kind, context=None, client=None, audience=None):
        claims = U05.decode_jwt(token, self.new_secret)
        require(claims.get("typ") == kind, "JWT_TYPE")
        now = int(time.time())
        require(type(claims.get("exp")) is int and claims["exp"] > now, "JWT_FRESH_EXPIRY")
        require(type(claims.get("iat")) is int and now - 120 <= claims["iat"] <= now + 5, "JWT_FRESH_ISSUANCE")
        if self.old_secret is not None:
            require(not U05.signature_valid(token, self.old_secret), "NEW_JWT_MUST_REJECT_PREDECESSOR_KEY")
        if context:
            require(all(claims.get(key) == context[field] and U05.valid_v7(claims.get(key)) for key, field in (("sub", "userId"), ("tenant", "tenantId"), ("org", "organizationId"))), "ORIGINAL_V7_CONTEXT_MUST_BE_PRESERVED")
        if kind == "session":
            require(U05.valid_v7(claims.get("sid")) and type(claims.get("ver")) is int, "SESSION_SID_VERSION")
        if kind == "access":
            require(U05.valid_v7(claims.get("jti")), "ACCESS_JTI_V7")
        if client:
            require(claims.get("cid") == client["clientId"], "OPAQUE_CLIENT_ID_PRESERVED")
        if audience:
            require(claims.get("aud") == audience, "APPLICATION_AUDIENCE")
        self.transcript.append({"check": self.current, "verifiedClaims": claims})
        return claims

    def login(self):
        result = self.ok(self.call("POST", "/api/auth/login", body=self.config["login"]))
        self.ticket = result["ticket"]
        claims = U05.decode_jwt(self.ticket, self.new_secret)
        require(claims.get("typ") == "credential" and claims.get("sub") == self.context["userId"], "ORIGINAL_USER_LOGIN_REQUIRED")
        self.session = self.establish_session(self.context)

    def client_authentication(self):
        result = self.call("GET", "/api/oauth/clients?size=200", bearer=self.session["accessToken"])
        require(result["status"] == 200, "COMPLETE_CLIENT_REGISTRY_REQUIRED")
        raw = result["json"]
        rows = raw if isinstance(raw, list) else raw.get("records", raw.get("items", []))
        require(isinstance(rows, list) and (not isinstance(raw, dict) or "total" not in raw or len(rows) == raw["total"]), "CLIENT_REGISTRY_TRUNCATED")
        confidential = {r["clientId"]: r for r in rows if r["clientType"] == "CONFIDENTIAL"}
        known = {c["clientId"]: c for c in self.clients.values() if c["clientType"] == "CONFIDENTIAL"}
        require(set(confidential) == set(known) and len(known) == 12, "ALL_CONFIDENTIAL_CLIENTS_REQUIRED")
        statuses = {cid: row["status"] for cid, row in confidential.items()}
        require(list(statuses.values()).count("ENABLED") == 11 and list(statuses.values()).count("DISABLED") == 1, "PRESERVED_ELEVEN_ACTIVE_ONE_DISABLED_REQUIRED")
        self.statuses = statuses
        for cid, client in known.items():
            require(U05.valid_v7(confidential[cid]["id"]), "PRESERVED_V7_CLIENT_ROW_REQUIRED")
            response = self.call("POST", "/oauth/introspect", form={"token": "u06-authentication-probe"}, basic=(cid, client["clientSecret"]))
            if statuses[cid] == "ENABLED":
                require(self.ok(response).get("active") is False, "ACTIVE_CLIENT_AUTHENTICATION_REQUIRED")
            else:
                self.rejected(response, {"invalid_client"})
        self.coverage.update({"confidentialClients": 12, "activeClientsAuthenticated": 11, "disabledClientCorrectSecretRejected": 1})

    def authorization_code(self):
        client = self.clients["oauth:auth"]
        verifier, state_value = secrets.token_urlsafe(48), secrets.token_urlsafe(24)
        challenge = U05.b64encode(hashlib.sha256(verifier.encode()).digest())
        form = {"client_id": client["clientId"], "redirect_uri": client["redirectUri"], "response_type": "code", "scope": client.get("scope", "profile"), "state": state_value, "code_challenge": challenge, "code_challenge_method": "S256"}
        response = self.call("GET", "/oauth/authorize?" + urllib.parse.urlencode(form))
        require(response["status"] == 302, "REAL_CONSENT_REDIRECT_REQUIRED")
        location = urllib.parse.urlsplit(response["headers"]["location"])
        request_id = urllib.parse.parse_qs(location.query).get("request_id", [])
        require(len(request_id) == 1 and U05.valid_v7(request_id[0]), "REAL_CONSENT_REQUEST_REQUIRED")
        consent_path = "/api/oauth/consent/" + request_id[0]
        self.ok(self.call("GET", consent_path, bearer=self.session["accessToken"]))
        issued = int(time.time())
        approved = self.ok(self.call("POST", consent_path + "/decision", body={"approve": True}, bearer=self.session["accessToken"]))
        # Parse the callback as data; never navigate or perform a request to it.
        callback, registered = urllib.parse.urlsplit(approved["redirectTo"]), urllib.parse.urlsplit(client["redirectUri"])
        require((callback.scheme, callback.netloc, callback.path) == (registered.scheme, registered.netloc, registered.path), "REGISTERED_CALLBACK_ONLY")
        query = urllib.parse.parse_qs(callback.query)
        require(query.get("state") == [state_value] and len(query.get("code", [])) == 1, "REAL_CODE_AND_STATE_REQUIRED")
        return {"kind": "authorization_code", "value": query["code"][0], "clientKey": "oauth:auth", "redirectUri": client["redirectUri"], "codeVerifier": verifier, "codeChallenge": challenge, "codeChallengeMethod": "S256", "issuedAt": issued, "expiresAt": issued + 60}

    def ordinary_oauth(self):
        code = self.authorization_code()
        issued = self.ok(self.token("oauth:auth", {"grant_type": "authorization_code", "code": code["value"], "redirect_uri": code["redirectUri"], "code_verifier": code["codeVerifier"]}))
        self.claims(issued["access_token"], "access", self.context, self.clients["oauth:auth"])
        require(isinstance(issued.get("refresh_token"), str) and issued["refresh_token"], "REAL_OAUTH_REFRESH_REQUIRED")
        info = self.ok(self.call("GET", "/oauth/userinfo", bearer=issued["access_token"]))
        require(info.get("sub") == self.context["userId"], "ORDINARY_OAUTH_SUBJECT_REQUIRED")
        active = self.ok(self.call("POST", "/oauth/introspect", form={"token": issued["access_token"]}, basic=(self.clients["oauth:auth"]["clientId"], self.clients["oauth:auth"]["clientSecret"])))
        require(active.get("active") is True, "ORDINARY_OAUTH_DIGEST_ACTIVE_AT_CAPTURE")
        self.oauth = issued

    def unused_device(self):
        issued = self.ok(self.call("POST", "/oauth/device/authorization", form=self.client_form("oauth:device", {"scope": self.clients["oauth:device"].get("scope", "profile")})))
        now = int(time.time())
        user_code = urllib.parse.quote(issued["user_code"], safe="")
        self.ok(self.call("GET", "/api/device/" + user_code, bearer=self.session["accessToken"]))
        self.ok(self.call("POST", "/api/device/" + user_code + "/decision", body={"approve": True}, bearer=self.session["accessToken"]))
        require(type(issued.get("expires_in")) is int and issued["expires_in"] > 0, "REAL_DEVICE_EXPIRY_REQUIRED")
        return {"kind": "device_code", "value": issued["device_code"], "clientKey": "oauth:device", "issuedAt": now, "expiresAt": now + issued["expires_in"], "approvedUnused": True}

    def create_capture(self):
        # Capture short-lived code last so its 60-second window is maximized.
        self.check("real_ordinary_oauth_access_refresh", self.ordinary_oauth)
        self.unused_device_sample = self.unused_device()
        self.unused_code_sample = self.authorization_code()
        samples = [{"kind": "login_ticket", "value": self.ticket}, {"kind": "session", "value": self.session["accessToken"]}, {"kind": "platform_refresh", "value": self.session["refreshToken"]}]
        samples += [{"kind": "app_access", "value": token["accessToken"], "appCode": code} for code, token in self.application_tokens.items()]
        samples += [{"kind": "oauth_access", "value": self.oauth["access_token"], "clientKey": "oauth:auth"}, {"kind": "oauth_refresh", "value": self.oauth["refresh_token"], "clientKey": "oauth:auth"}, self.unused_code_sample, self.unused_device_sample]
        for sample in samples:
            if sample["value"].count(".") == 2:
                sample["expiresAt"] = U05.decode_jwt(sample["value"], self.new_secret)["exp"]
        raw_clients = {c["clientId"]: sha(c["clientSecret"].encode()) for c in self.clients.values() if c["clientType"] == "CONFIDENTIAL"}
        self.capture = {"format": "saas-uuid-u06-capture/1", "generation": self.generation, "captured_at": int(time.time()), "identity_sha256": self.identity_sha, "samples": samples, "session": self.session, "applicationTokens": self.application_tokens, "clients_sha256": sha(RUNTIME.canonical(raw_clients)), "client_statuses": self.statuses, "oauth": self.oauth, "unusedCode": self.unused_code_sample, "unusedDevice": self.unused_device_sample}
        capture_input(self.capture, self.generation)

    def replay_samples(self, capture, signing_secret):
        counts, expired = {}, {}
        for sample in capture["samples"]:
            kind, value = sample["kind"], sample["value"]
            counts[kind] = counts.get(kind, 0) + 1
            expired[kind] = expired.get(kind, 0) + int(sample.get("expiresAt", int(time.time()) + 1) <= int(time.time()))
            if value.count(".") == 2:
                U05.decode_jwt(value, signing_secret)
                require(not U05.signature_valid(value, self.new_secret), "OLD_JWT_SIGNATURE_REJECTED_BY_NEW_KEY")
            if kind == "session":
                require(self.call("GET", "/api/auth/profile", bearer=value)["status"] == 401, "OLD_SESSION_MUST_RETURN_401")
            elif kind == "platform_refresh":
                self.rejected(self.call("POST", "/oauth/token", form={"grant_type": "refresh_token", "refresh_token": value}), {"invalid_grant"})
            elif kind == "app_access":
                require(sample.get("appCode") in APP_CODES, "OLD_APP_BINDING_REQUIRED")
                require(self.call("GET", "/api/apps/" + sample["appCode"] + "/context", bearer=value)["status"] == 401, "OLD_APP_TOKEN_MUST_RETURN_401")
            elif kind == "oauth_access":
                self.rejected(self.call("GET", "/oauth/userinfo", bearer=value), {"invalid_token"})
                client = self.clients[sample["clientKey"]]
                result = self.ok(self.call("POST", "/oauth/introspect", form={"token": value}, basic=(client["clientId"], client["clientSecret"])))
                require(result.get("active") is False, "OLD_OAUTH_DIGEST_MUST_BE_INACTIVE")
            elif kind == "oauth_refresh":
                self.rejected(self.token(sample["clientKey"], {"grant_type": "refresh_token", "refresh_token": value}), {"invalid_grant"})
            elif kind == "authorization_code":
                self.rejected(self.token(sample["clientKey"], {"grant_type": "authorization_code", "code": value, "redirect_uri": sample["redirectUri"], "code_verifier": U05.original_pkce_verifier(sample)}), {"invalid_grant"})
            elif kind == "device_code":
                self.rejected(self.token(sample["clientKey"], {"grant_type": U05.DEVICE_GRANT, "device_code": value}), {"invalid_grant"})
            elif kind == "login_ticket":
                self.rejected(self.call("POST", "/api/auth/context", bearer=value, body={"organizationId": self.context["organizationId"]}))
        return counts, expired

    def replay_predecessor(self):
        counts, expired = self.replay_samples(self.predecessor, self.old_secret)
        self.coverage["realPredecessorSamplesReplayed"], self.coverage["predecessorSamplesExpiredAtReplay"] = counts, expired
        if self.generation == "recovery":
            counts, expired = self.replay_samples(self.original_source, self.original_secret)
            self.coverage["originalSourceSamplesReplayed"], self.coverage["originalSourceSamplesExpiredAtReplay"] = counts, expired
        self.coverage["identityPreserved"] = True

    def reject_old_client_secrets(self):
        count = 0
        for client_set in ([self.previous_clients, self.source_clients] if self.generation == "recovery" else [self.previous_clients]):
            for client in client_set.values():
                if client["clientType"] != "CONFIDENTIAL":
                    continue
                self.rejected(self.call("POST", "/oauth/introspect", form={"token": "u06-old-secret-probe"}, basic=(client["clientId"], client["clientSecret"])), {"invalid_client"})
                count += 1
        self.coverage["oldConfidentialSecretsRejected"] = count
        require(count == (24 if self.generation == "recovery" else 12), "ALL_OLD_SECRET_REPLAYS_REQUIRED")

    def run(self):
        self.check("fresh_original_v7_login_context", self.login)
        self.check("eleven_active_one_disabled_client_authentication", self.client_authentication)
        if self.generation != "source":
            self.check("real_predecessor_credential_replays", self.replay_predecessor)
        self.check("seven_original_application_protocols", self.applications)
        if self.generation != "source":
            self.check("platform_refresh_rotation", self.platform_refresh)
            self.check("oauth_code_pkce_callback_refresh", self.oauth_authorization)
            self.check("oauth_client_credentials_boundary", self.oauth_client_credentials)
            self.check("oauth_device_lifecycle", self.oauth_device)
        self.check("fresh_next_epoch_capture", self.create_capture)
        # Failures are deliberately last because failed introspection has a
        # cumulative per-IP limiter. Successful auth and digest checks run first.
        if self.generation != "source":
            self.check("all_twelve_previous_client_secrets_rejected", self.reject_old_client_secrets)

    def finish(self, failure=None):
        RUNTIME.write_new(self.output / "transcript.private.json", json.dumps(self.transcript, indent=2, ensure_ascii=True) + "\n")
        capture_sha = None
        if hasattr(self, "capture"):
            raw = json.dumps(self.capture, indent=2, ensure_ascii=True).encode() + b"\n"
            RUNTIME.write_new(self.output / "capture.private.json", raw)
            capture_sha = sha(raw)
        counts = {}
        for sample in getattr(self, "capture", {}).get("samples", []):
            counts[sample["kind"]] = counts.get(sample["kind"], 0) + 1
        summary = {"format": "saas-uuid-u06-http-result/1", "generation": self.generation, "status": "FAIL" if failure else "PASS", "checks": self.checks, "coverage": self.coverage, "http_requests": sum("method" in item for item in self.transcript), "captured_sample_counts": counts, "capture_sha256": capture_sha, "identity_sha256": self.identity_sha, "database_connections": 0, "business_writes": 0, "external_callbacks_requested": 0, "natural_expiry_is_epoch_proof": False}
        if failure:
            summary["failed_check"] = self.current
            summary["failure_code"] = failure
        RUNTIME.write_new(self.output / "summary.json", json.dumps(summary, indent=2, ensure_ascii=True) + "\n")
        return summary


def execute(config_file, check=False):
    require(sys.platform == "linux" and os.geteuid() == 0, "LINUX_ROOT_REQUIRED")
    config = config_of(RUNTIME.private_json(config_file))
    if check:
        # Validate private inputs without mkdir, login or any HTTP request.
        Runner(config, create_output=False)
        return {"status": "CHECKED_NO_HTTP", "generation": config["generation"], "database_connections": 0, "http_requests": 0}
    runner = Runner(config)
    try:
        runner.run()
        return runner.finish()
    except Exception as error:
        code = str(error) if isinstance(error, U05.CheckFailed) and re.fullmatch(r"[A-Z][A-Z0-9_]{1,100}", str(error)) else "PRIVATE_ACCEPTANCE_FAILED"
        return runner.finish(code)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("config")
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--run", action="store_true")
    args = parser.parse_args()
    try:
        result = execute(args.config, args.check)
        print(json.dumps(result, ensure_ascii=True))
        return int(result["status"] == "FAIL")
    except Exception:
        print("U06 HTTP acceptance rejected; inspect fixed private input contract", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
