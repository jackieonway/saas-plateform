#!/usr/bin/env python3
"""Validate the quarantined UUID U05 runtime without seeding business fixtures.

This is an operator-invoked, mutating HTTP acceptance tool, not a launcher. It
creates only real login/OAuth credentials and protocol requests. It never opens
a database connection, changes registrations, rotates secrets, or follows a
callback URL. Run under Linux with a new private output directory:

  python3 validate-uuid-u05-runtime.py /private/handoff.json /private/result-01

The handoff format is ``saas-uuid-u05-runtime/1``. Required keys:
platformOrigin, platformEnvFile, oldPlatformEnvFile, oldStateFile, login
{username,password}, context {userId,tenantId,organizationId}, clients (keyed
like old preview state, including oauth:auth, oauth:device, oauth:cc),
applications [{code,applicationId,clientKey,resource,action,context?}],
sourceCredentialRows, targetCredentialRowsBeforeRuntime (six-table count
objects), credentialEvidenceSha256. Each client has clientId, clientType,
clientSecret (CONFIDENTIAL only), scope and redirectUri (auth client only).
The main context must be the platform administrator's platform context for the
client registry inspection; put business contexts on individual applications.
Optional: oldEnvFiles, previousClientSecretHashes (opaque clientId -> old DB
SHA-256), oldCredentialSamples [{kind,value,clientKey?,appCode?,redirectUri?,
codeVerifier?,codeChallenge?,codeChallengeMethod?}]. Real authorization-code
replays require the original PKCE verifier; a missing verifier is INCOMPLETE,
not a successful epoch-rejection proof. If a challenge is supplied, its match
is also verified locally. OAuth access replays require clientKey for both
userinfo and the frozen-12 digest-based introspection check.
Sample kinds: session, platform_refresh, app_access, oauth_access,
oauth_refresh, authorization_code, device_code, login_ticket.

All handoff/input files must be owned mode 0600 regular files without symlinks.
Only status/counts/check identifiers escape the output directory. Every HTTP
request/response and every derived claim remains in a mode 0600 transcript.
Source/table counts are evidence supplied by the operator, not DB observations
made by this tool; their evidence digest is retained in the result. A missing
real old code/device sample is reported honestly: source-empty plus synthetic
rejection, or INCOMPLETE if source rows existed. Session JWTs use sid/ver;
OAuth/app access JWTs use jti. This frozen migration-12 contract deliberately
does not assert any membership-policy/SSO Phase 2 behavior.
"""

import base64
import hashlib
import hmac
import ipaddress
import json
import os
from pathlib import Path
import re
import secrets
import stat
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid


CREDENTIAL_TABLES = (
    "oauth_authorization_codes", "oauth_authorization_requests",
    "oauth_device_codes", "oauth_tokens", "platform_session_refresh_tokens",
    "platform_sessions",
)
DEVICE_GRANT = "urn:ietf:params:oauth:grant-type:device_code"
IDENTIFIER = re.compile(r"^[A-Za-z0-9:_-]{1,100}$")


class CheckFailed(Exception):
    """Only a predefined check identifier may reach a public failure field."""


def require(condition, code):
    if not condition:
        raise CheckFailed(code)


def private_path(value):
    require(os.name == "posix", "LINUX_PRIVATE_FILES_REQUIRED")
    path = Path(value)
    require(path.is_absolute() and ".." not in path.parts, "PRIVATE_ABSOLUTE_PATH_REQUIRED")
    for part in [path, *path.parents]:
        require(not part.is_symlink(), "PRIVATE_SYMLINK_REJECTED")
    info = path.stat()
    require(stat.S_ISREG(info.st_mode), "PRIVATE_REGULAR_FILE_REQUIRED")
    require(info.st_uid == os.geteuid(), "PRIVATE_OWNER_REQUIRED")
    require(stat.S_IMODE(info.st_mode) == 0o600, "PRIVATE_MODE_0600_REQUIRED")
    require(info.st_size <= 8 * 1024 * 1024, "PRIVATE_FILE_TOO_LARGE")
    return path


def read_json(value):
    return json.loads(private_path(value).read_text(encoding="utf-8-sig"))


def read_env(value):
    result = {}
    for line in private_path(value).read_text(encoding="utf-8-sig").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("export "):
            line = line[7:]
        require("=" in line, "PRIVATE_ENV_SYNTAX")
        key, data = line.split("=", 1)
        key, data = key.strip(), data.strip()
        require(re.fullmatch(r"[A-Z][A-Z0-9_]*", key) is not None, "PRIVATE_ENV_KEY")
        require(key not in result, "PRIVATE_ENV_DUPLICATE_KEY")
        if len(data) >= 2 and data[0] == data[-1] and data[0] in "\"'":
            data = data[1:-1]
        result[key] = data
    return result


def write_private(path, value):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2)
        stream.write("\n")


def b64decode(value):
    return base64.urlsafe_b64decode(value + "=" * (-len(value) % 4))


def b64encode(value):
    return base64.urlsafe_b64encode(value).decode("ascii").rstrip("=")


def valid_v7(value):
    try:
        parsed = uuid.UUID(value)
        return parsed.version == 7 and parsed.variant == uuid.RFC_4122
    except (ValueError, TypeError, AttributeError):
        return False


def decode_jwt(token, secret):
    try:
        header, payload, signature = token.split(".")
        require(json.loads(b64decode(header)).get("alg") == "HS256", "JWT_ALGORITHM")
        expected = hmac.new(secret.encode(), (header + "." + payload).encode(), hashlib.sha256).digest()
        require(hmac.compare_digest(expected, b64decode(signature)), "JWT_SIGNATURE")
        claims = json.loads(b64decode(payload))
        require(isinstance(claims, dict), "JWT_PAYLOAD")
        return claims
    except CheckFailed:
        raise
    except (ValueError, TypeError, KeyError):
        raise CheckFailed("JWT_ENCODING") from None


def signature_valid(token, secret):
    try:
        decode_jwt(token, secret)
        return True
    except CheckFailed:
        return False


def loopback_origin(value):
    parsed = urllib.parse.urlsplit(value)
    try:
        valid_address = ipaddress.ip_address(parsed.hostname).is_loopback
    except (ValueError, TypeError):
        valid_address = False
    require(parsed.scheme == "http" and valid_address and parsed.port is not None,
            "EXPLICIT_HTTP_LOOPBACK_ORIGIN_REQUIRED")
    require(not parsed.username and not parsed.password and not parsed.query and not parsed.fragment
            and parsed.path in ("", "/"), "BARE_LOOPBACK_ORIGIN_REQUIRED")
    return value.rstrip("/")


def original_pkce_verifier(sample):
    verifier = sample.get("codeVerifier")
    if not isinstance(verifier, str) or not re.fullmatch(r"[A-Za-z0-9._~-]{43,128}", verifier):
        return None
    challenge = sample.get("codeChallenge")
    if challenge is not None:
        method = sample.get("codeChallengeMethod", "S256")
        require(method in ("S256", "plain"), "OLD_CODE_PKCE_METHOD")
        expected = b64encode(hashlib.sha256(verifier.encode()).digest()) if method == "S256" else verifier
        require(hmac.compare_digest(expected, challenge), "OLD_CODE_ORIGINAL_PKCE_MISMATCH")
    return verifier


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        return None


class Runner:
    def __init__(self, config, output):
        self.config = config
        self.origin = loopback_origin(config["platformOrigin"])
        self.output = Path(output)
        require(self.output.is_absolute() and ".." not in self.output.parts,
                "PRIVATE_OUTPUT_ABSOLUTE_REQUIRED")
        for part in [self.output.parent, *self.output.parent.parents]:
            require(not part.is_symlink(), "PRIVATE_OUTPUT_SYMLINK_REJECTED")
        require(self.output.parent.stat().st_uid == os.geteuid(), "PRIVATE_OUTPUT_OWNER_REQUIRED")
        self.output.mkdir(mode=0o700)
        self.transcript = []
        self.checks = []
        self.coverage = {}
        self.current = "INPUT_VALIDATION"
        self.last_oauth = 0.0
        self.http = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
        self.new_env = read_env(config["platformEnvFile"])
        self.old_env = read_env(config["oldPlatformEnvFile"])
        self.new_secret = self.new_env["JWT_SECRET"]
        self.old_secret = self.old_env["JWT_SECRET"]
        self.old_state = read_json(config["oldStateFile"])
        self.clients = config["clients"]
        self.context = config["context"]
        for field in ("userId", "tenantId", "organizationId"):
            require(valid_v7(self.context[field]), "EXPECTED_CONTEXT_MUST_BE_V7")
        for field, state_field in (("userId", "user"), ("tenantId", "tenant"), ("organizationId", "organization")):
            old = self.old_state.get("platformSession", {}).get(state_field, {}).get("id")
            require(old and old != self.context[field], "EXPECTED_CONTEXT_MUST_BE_REKEYED")
        require(re.fullmatch(r"[0-9a-f]{64}", config["credentialEvidenceSha256"]),
                "CREDENTIAL_EVIDENCE_DIGEST_REQUIRED")
        for counts in (config["sourceCredentialRows"], config["targetCredentialRowsBeforeRuntime"]):
            require(set(counts) == set(CREDENTIAL_TABLES), "CREDENTIAL_EVIDENCE_SIX_TABLES_REQUIRED")
            require(all(type(value) is int and value >= 0 for value in counts.values()),
                    "CREDENTIAL_EVIDENCE_COUNTS_INVALID")
        require(all(value == 0 for value in config["targetCredentialRowsBeforeRuntime"].values()),
                "TARGET_CREDENTIAL_EPOCH_NOT_EMPTY")

    def call(self, method, path, *, body=None, form=None, bearer=None, basic=None):
        require(path.startswith(("/api/", "/oauth/")) and not path.startswith("//"), "HTTP_PATH")
        if path in ("/oauth/token", "/oauth/device/authorization"):
            # These share a real 30/minute per-IP limit; do not turn rate limiting
            # into a false credential rejection. Keep this single runner below it.
            delay = 2.2 - (time.monotonic() - self.last_oauth)
            if delay > 0:
                time.sleep(delay)
            self.last_oauth = time.monotonic()
        headers = {"Accept": "application/json"}
        data = None
        if body is not None:
            data = json.dumps(body).encode()
            headers["Content-Type"] = "application/json"
        if form is not None:
            data = urllib.parse.urlencode(form).encode()
            headers["Content-Type"] = "application/x-www-form-urlencoded"
        if bearer:
            headers["Authorization"] = "Bearer " + bearer
        if basic:
            # Components are URL encoded before base64, as required by this server.
            pair = ":".join(urllib.parse.quote(value, safe="") for value in basic)
            headers["Authorization"] = "Basic " + base64.b64encode(pair.encode()).decode()
        request = urllib.request.Request(self.origin + path, data, headers, method=method)
        entry = {"method": method, "path": path, "body": body, "form": form,
                 "headers": headers, "check": self.current}
        self.transcript.append(entry)
        try:
            try:
                response = self.http.open(request, timeout=20)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                raw = response.read(2 * 1024 * 1024 + 1)
                require(len(raw) <= 2 * 1024 * 1024, "HTTP_RESPONSE_TOO_LARGE")
                result = {"status": response.code, "headers": {key.lower(): value for key, value in response.headers.items()},
                          "raw": raw.decode("utf-8", errors="replace")}
                try:
                    result["json"] = json.loads(raw)
                except (ValueError, UnicodeDecodeError):
                    result["json"] = None
                entry["response"] = result
                require(response.code != 429, "RATE_LIMITED_CANNOT_PROVE_REJECTION")
                require(response.code < 500, "SERVER_ERROR_CANNOT_PROVE_REJECTION")
                return result
        except (urllib.error.URLError, OSError, TimeoutError):
            entry["transportError"] = True
            raise CheckFailed("LOOPBACK_HTTP_TRANSPORT") from None

    def ok(self, response, code="HTTP_SUCCESS_REQUIRED"):
        require(200 <= response["status"] < 300 and isinstance(response["json"], dict), code)
        return response["json"]

    def rejected(self, response, errors=None):
        require(response["status"] in (400, 401, 403, 404, 409, 422), "CREDENTIAL_REJECTION_REQUIRED")
        if errors:
            require(isinstance(response["json"], dict) and response["json"].get("error") in errors,
                    "OAUTH_ERROR_REQUIRED")

    def check(self, name, action):
        require(IDENTIFIER.fullmatch(name) is not None, "STATIC_CHECK_IDENTIFIER_REQUIRED")
        self.current = name
        action()
        self.checks.append({"check": name, "status": "PASS"})

    def client_form(self, key, form):
        client = self.clients[key]
        result = {**form, "client_id": client["clientId"]}
        if client["clientType"] == "CONFIDENTIAL":
            result["client_secret"] = client["clientSecret"]
        return result

    def token(self, key, form):
        return self.call("POST", "/oauth/token", form=self.client_form(key, form))

    def claims(self, token, kind, context=None, client=None, audience=None):
        claims = decode_jwt(token, self.new_secret)
        require(not signature_valid(token, self.old_secret), "NEW_JWT_MUST_REJECT_OLD_KEY")
        require(claims.get("typ") == kind, "JWT_TYPE")
        now = int(time.time())
        require(type(claims.get("exp")) is int and claims["exp"] > now, "JWT_FRESH_EXPIRY")
        require(type(claims.get("iat")) is int and now - 120 <= claims["iat"] <= now + 5,
                "JWT_FRESH_ISSUANCE")
        if context:
            require(claims.get("sub") == context["userId"] and claims.get("tenant") == context["tenantId"]
                    and claims.get("org") == context["organizationId"], "JWT_EXPECTED_MAPPED_CONTEXT")
            require(all(valid_v7(claims.get(key)) for key in ("sub", "tenant", "org")), "JWT_CONTEXT_V7")
        if kind == "session":
            require(valid_v7(claims.get("sid")) and type(claims.get("ver")) is int, "SESSION_SID_VERSION")
        if kind == "access":
            require(valid_v7(claims.get("jti")), "ACCESS_JTI_V7")
        if client:
            require(claims.get("cid") == client["clientId"], "ACCESS_CLIENT_BINDING")
        if audience:
            require(claims.get("aud") == audience, "ACCESS_AUDIENCE")
        self.transcript.append({"check": self.current, "verifiedClaims": claims})
        return claims

    def establish_session(self, context):
        result = self.ok(self.call("POST", "/api/auth/context",
                                 body={"organizationId": context["organizationId"], "userId": context["userId"]},
                                 bearer=self.ticket))
        require(result["user"]["id"] == context["userId"] and result["tenant"]["id"] == context["tenantId"]
                and result["organization"]["id"] == context["organizationId"], "SESSION_EXPECTED_CONTEXT")
        self.claims(result["accessToken"], "session", context)
        self.ok(self.call("GET", "/api/auth/profile", bearer=result["accessToken"]))
        return result

    def login(self):
        result = self.ok(self.call("POST", "/api/auth/login", body=self.config["login"]))
        self.ticket = result["ticket"]
        claims = decode_jwt(self.ticket, self.new_secret)
        require(claims.get("typ") == "credential", "FROZEN12_SINGLE_ACCOUNT_TICKET_REQUIRED")
        require(claims.get("sub") == self.context["userId"], "LOGIN_EXPECTED_MAPPED_USER")
        require(not signature_valid(self.ticket, self.old_secret), "LOGIN_TICKET_KEY_ROTATED")
        memberships = result.get("memberships", [])
        require(any(item.get("organizationId") == self.context["organizationId"] for item in memberships),
                "LOGIN_EXPECTED_MEMBERSHIP")
        self.session = self.establish_session(self.context)

    def client_rotation(self):
        require(self.new_secret != self.old_secret and len(self.new_secret) >= 32, "JWT_KEY_ROTATION_REQUIRED")
        response = self.call("GET", "/api/oauth/clients?size=200", bearer=self.session["accessToken"])
        require(response["status"] == 200, "CLIENT_LIST_REQUIRED")
        raw = response["json"]
        rows = raw if isinstance(raw, list) else raw.get("records", raw.get("items", []))
        if isinstance(raw, dict) and "total" in raw:
            require(len(rows) == raw["total"], "CLIENT_LIST_MUST_BE_COMPLETE")
        actual = {row["clientId"]: row for row in rows if row["clientType"] == "CONFIDENTIAL"}
        known = {client["clientId"]: client for client in self.clients.values()
                 if client["clientType"] == "CONFIDENTIAL"}
        require(set(actual) == set(known), "EVERY_CONFIDENTIAL_CLIENT_REQUIRED")
        old = {client["clientId"]: client["clientSecret"] for client in self.old_state.get("clients", {}).values()
               if client.get("clientId") and client.get("clientSecret")}
        for filename in self.config.get("oldEnvFiles", []):
            env = read_env(filename)
            for prefix in ("PLATFORM", "OAUTH_AUTH", "OAUTH_CC", "OAUTH_DEVICE"):
                if env.get(prefix + "_CLIENT_ID") and env.get(prefix + "_CLIENT_SECRET"):
                    old[env[prefix + "_CLIENT_ID"]] = env[prefix + "_CLIENT_SECRET"]
        replayed = 0
        hashed_only = 0
        for cid, client in known.items():
            require(valid_v7(actual[cid]["id"]), "CLIENT_ROW_ID_V7")
            new = client["clientSecret"]
            old_hash = self.config.get("previousClientSecretHashes", {}).get(cid)
            if cid in old:
                require(new != old[cid], "CLIENT_SECRET_MUST_CHANGE")
                self.rejected(self.call("POST", "/oauth/introspect", form={"token": "u05-rotation-probe"},
                                        basic=(cid, old[cid])), {"invalid_client"})
                replayed += 1
            else:
                require(isinstance(old_hash, str) and re.fullmatch(r"[0-9a-f]{64}", old_hash)
                        and hashlib.sha256(new.encode()).hexdigest() != old_hash,
                        "OLD_CLIENT_SECRET_OR_HASH_REQUIRED")
                hashed_only += 1
            result = self.ok(self.call("POST", "/oauth/introspect", form={"token": "u05-rotation-probe"},
                                       basic=(cid, new)))
            require(result.get("active") is False, "INACTIVE_PROBE_REQUIRED")
        self.coverage["confidentialClients"] = len(known)
        self.coverage["oldClientSecretsReplayed"] = replayed
        self.coverage["oldClientSecretsHashEvidenceOnly"] = hashed_only

    def replay_old_oauth_access(self, sample):
        value = sample["value"]
        client = self.clients[sample["clientKey"]]
        old_claims = decode_jwt(value, self.old_secret)
        require(old_claims.get("typ") == "access" and old_claims.get("cid") == client["clientId"],
                "OLD_OAUTH_CLIENT_BINDING")
        require(client["clientType"] == "CONFIDENTIAL", "OLD_OAUTH_INTROSPECTION_CONFIDENTIAL_CLIENT_REQUIRED")
        self.rejected(self.call("GET", "/oauth/userinfo", bearer=value), {"invalid_token"})
        # Ordinary OAuth introspection authenticates the stored digest,
        # independently of JWT signature validation. A rotated JWT key
        # alone cannot prove that a mistakenly preserved row is dead.
        result = self.ok(self.call("POST", "/oauth/introspect", form={"token": value},
                                  basic=(client["clientId"], client["clientSecret"])))
        require(result.get("active") is False, "OLD_OAUTH_DIGEST_MUST_BE_INACTIVE")

    def old_credentials(self):
        samples = list(self.config.get("oldCredentialSamples", []))
        for key in ("platformSession", "businessSession"):
            session = self.old_state.get(key, {})
            for field, kind in (("accessToken", "session"), ("refreshToken", "platform_refresh")):
                if session.get(field):
                    samples.append({"kind": kind, "value": session[field]})
        for code, value in self.old_state.get("applicationTokens", {}).items():
            if value.get("accessToken"):
                samples.append({"kind": "app_access", "value": value["accessToken"], "appCode": code})
        counts = {}
        expired_jwt_count = 0
        code_verifier_unavailable = 0
        seen = set()
        for sample in samples:
            kind, value = sample["kind"], sample["value"]
            require(isinstance(value, str) and value, "OLD_SAMPLE_REQUIRED")
            fingerprint = (kind, hashlib.sha256(value.encode()).digest())
            if fingerprint in seen:
                continue
            seen.add(fingerprint)
            if kind == "authorization_code" and original_pkce_verifier(sample) is None:
                # An arbitrary verifier can reject an otherwise still-active
                # preserved code. Do not call that an epoch rejection.
                code_verifier_unavailable += 1
                continue
            counts[kind] = counts.get(kind, 0) + 1
            if value.count(".") == 2:
                old_claims = decode_jwt(value, self.old_secret)
                require(not signature_valid(value, self.new_secret), "OLD_JWT_ACCEPTED_NEW_KEY")
                expired_jwt_count += int(old_claims.get("exp", 0) <= int(time.time()))
            if kind == "session":
                self.rejected(self.call("GET", "/api/auth/profile", bearer=value))
            elif kind == "platform_refresh":
                self.rejected(self.call("POST", "/oauth/token", form={"grant_type": "refresh_token", "refresh_token": value}),
                              {"invalid_grant"})
            elif kind == "app_access":
                require(re.fullmatch(r"[a-z0-9-]{1,100}", sample["appCode"]), "APP_CODE")
                self.rejected(self.call("GET", "/api/apps/" + sample["appCode"] + "/context", bearer=value))
            elif kind == "oauth_access":
                self.replay_old_oauth_access(sample)
            elif kind == "oauth_refresh":
                self.rejected(self.token(sample["clientKey"], {"grant_type": "refresh_token", "refresh_token": value}), {"invalid_grant"})
            elif kind == "authorization_code":
                self.rejected(self.token(sample["clientKey"], {"grant_type": kind, "code": value,
                              "redirect_uri": sample["redirectUri"], "code_verifier": original_pkce_verifier(sample)}), {"invalid_grant"})
            elif kind == "device_code":
                self.rejected(self.token(sample["clientKey"], {"grant_type": DEVICE_GRANT, "device_code": value}), {"invalid_grant"})
            elif kind == "login_ticket":
                self.rejected(self.call("POST", "/api/auth/context", bearer=value,
                              body={"organizationId": self.context["organizationId"]}))
            else:
                raise CheckFailed("UNKNOWN_OLD_CREDENTIAL_KIND")
        require(counts.get("session", 0) > 0 and counts.get("platform_refresh", 0) > 0
                and counts.get("app_access", 0) > 0, "REAL_OLD_PREVIEW_CREDENTIALS_REQUIRED")
        self.coverage["oldCredentialSamplesReplayed"] = counts
        self.coverage["oldJwtSamplesAlreadyExpired"] = expired_jwt_count
        self.coverage["oldCodeSamplesOriginalVerifierUnavailable"] = code_verifier_unavailable
        for kind, table in (("authorization_code", "oauth_authorization_codes"), ("device_code", "oauth_device_codes")):
            if counts.get(kind, 0):
                self.coverage[kind + "Evidence"] = "REAL_OLD_SAMPLE_REPLAY"
            elif self.config["sourceCredentialRows"][table] == 0:
                self.coverage[kind + "Evidence"] = "SOURCE_EMPTY_AND_SYNTHETIC_NEGATIVE"
            else:
                self.coverage[kind + "Evidence"] = "SOURCE_NONEMPTY_REAL_RAW_UNAVAILABLE"
        if code_verifier_unavailable:
            self.coverage["authorization_codeEvidence"] = "RAW_SAMPLE_ORIGINAL_PKCE_VERIFIER_UNAVAILABLE"
        for kind in ("oauth_access", "oauth_refresh"):
            if counts.get(kind, 0):
                self.coverage[kind + "Evidence"] = "REAL_OLD_SAMPLE_REPLAY"
            elif self.config["sourceCredentialRows"]["oauth_tokens"] == 0:
                self.coverage[kind + "Evidence"] = "SOURCE_TOKEN_TABLE_EMPTY_NO_REAL_RAW_SAMPLE"
            else:
                self.coverage[kind + "Evidence"] = "SOURCE_TOKEN_TABLE_CLEARED_REAL_RAW_UNAVAILABLE"
        self.rejected(self.token("oauth:auth", {"grant_type": "authorization_code", "code": secrets.token_urlsafe(32),
                      "redirect_uri": self.clients["oauth:auth"]["redirectUri"], "code_verifier": "A" * 43}), {"invalid_grant"})
        self.rejected(self.token("oauth:device", {"grant_type": DEVICE_GRANT, "device_code": secrets.token_urlsafe(32)}), {"invalid_grant"})

    def platform_refresh(self):
        previous = self.session["refreshToken"]
        result = self.ok(self.call("POST", "/oauth/token", form={"grant_type": "refresh_token", "refresh_token": previous}))
        self.claims(result["access_token"], "session", self.context)
        require(result["refresh_token"] != previous, "PLATFORM_REFRESH_ROTATION")
        self.session["accessToken"] = result["access_token"]
        self.session["refreshToken"] = result["refresh_token"]
        self.rejected(self.call("POST", "/oauth/token", form={"grant_type": "refresh_token", "refresh_token": previous}), {"invalid_grant"})
        self.rejected(self.token("oauth:auth", {"grant_type": "refresh_token", "refresh_token": result["refresh_token"]}), {"invalid_request"})

    def applications(self):
        self.application_tokens = {}
        applications = self.config["applications"]
        require(bool(applications), "APPLICATIONS_REQUIRED")
        for app in applications:
            code = app["code"]
            require(re.fullmatch(r"[a-z0-9-]{1,100}", code), "APP_CODE")
            context = app.get("context", self.context)
            session = self.session if context == self.context else self.establish_session(context)
            issued = self.ok(self.call("POST", "/api/oauth/session-token", body={"appCode": code}, bearer=session["accessToken"]))
            token = issued["accessToken"]
            self.claims(token, "access", context, self.clients[app["clientKey"]], code)
            result = self.ok(self.call("GET", "/api/apps/" + code + "/context", bearer=token))
            require(result["appCode"] == code and result["application"]["id"] == app["applicationId"]
                    and result["user"]["id"] == context["userId"] and result["tenant"]["id"] == context["tenantId"]
                    and result["organization"]["id"] == context["organizationId"], "APP_CONTEXT_MAPPED_REFERENCES")
            client = self.clients[app["clientKey"]]
            introspection = self.ok(self.call("POST", "/oauth/introspect", form={"token": token},
                                             basic=(client["clientId"], client["clientSecret"])))
            require(introspection.get("active") is True and introspection.get("sub") == context["userId"]
                    and valid_v7(introspection.get("jti")), "APP_INTROSPECTION_BINDING")
            if app.get("resource") and app.get("action"):
                result = self.ok(self.call("POST", "/api/apps/" + code + "/authorize",
                                 form={"token": token, "resource": app["resource"], "action": app["action"]},
                                 basic=(client["clientId"], client["clientSecret"])))
                require(result.get("contractVersion") == "1.0" and result["introspection"]["active"] is True,
                        "APP_AUTHORIZATION_CONTRACT")
                require(result["context"]["organization"]["id"] == context["organizationId"], "APP_AUTHORIZATION_CONTEXT")
            self.application_tokens[code] = {"accessToken": token, "context": result}
        if len(applications) >= 2:
            first, second = applications[:2]
            token = self.application_tokens[first["code"]]["accessToken"]
            self.rejected(self.call("GET", "/api/apps/" + second["code"] + "/context", bearer=token))
            second_client = self.clients[second["clientKey"]]
            result = self.ok(self.call("POST", "/oauth/introspect", form={"token": token},
                            basic=(second_client["clientId"], second_client["clientSecret"])))
            require(result.get("active") is False, "CROSS_APPLICATION_INTROSPECTION_REJECTED")
        self.coverage["applicationsValidated"] = len(applications)

    def oauth_authorization(self):
        client = self.clients["oauth:auth"]
        verifier = secrets.token_urlsafe(48)
        state_value = secrets.token_urlsafe(24)
        form = {"client_id": client["clientId"], "redirect_uri": client["redirectUri"], "response_type": "code",
                "scope": client.get("scope", "profile"), "state": state_value,
                "code_challenge": b64encode(hashlib.sha256(verifier.encode()).digest()), "code_challenge_method": "S256"}
        missing = {key: value for key, value in form.items() if not key.startswith("code_challenge")}
        self.rejected(self.call("GET", "/oauth/authorize?" + urllib.parse.urlencode(missing)), {"invalid_request"})
        bad_redirect = {**form, "redirect_uri": "http://127.0.0.1:1/unregistered-u05-callback"}
        self.rejected(self.call("GET", "/oauth/authorize?" + urllib.parse.urlencode(bad_redirect)), {"invalid_request"})
        response = self.call("GET", "/oauth/authorize?" + urllib.parse.urlencode(form))
        require(response["status"] == 302, "AUTHORIZATION_CONSENT_REDIRECT")
        request_id = urllib.parse.parse_qs(urllib.parse.urlsplit(response["headers"]["location"]).query)["request_id"][0]
        require(valid_v7(request_id), "AUTHORIZATION_REQUEST_ID_V7")
        consent_path = "/api/oauth/consent/" + request_id
        self.ok(self.call("GET", consent_path, bearer=self.session["accessToken"]))
        approved = self.ok(self.call("POST", consent_path + "/decision", body={"approve": True}, bearer=self.session["accessToken"]))
        callback = urllib.parse.urlsplit(approved["redirectTo"])
        registered = urllib.parse.urlsplit(client["redirectUri"])
        require((callback.scheme, callback.netloc, callback.path) == (registered.scheme, registered.netloc, registered.path),
                "REGISTERED_CALLBACK_ONLY")
        query = urllib.parse.parse_qs(callback.query)
        require(query.get("state") == [state_value] and len(query.get("code", [])) == 1, "CALLBACK_CODE_STATE")
        exchange = {"grant_type": "authorization_code", "code": query["code"][0],
                    "redirect_uri": client["redirectUri"], "code_verifier": verifier}
        self.rejected(self.token("oauth:auth", {**exchange, "code_verifier": "A" * 43}), {"invalid_grant"})
        self.rejected(self.token("oauth:auth", {**exchange, "redirect_uri": "http://127.0.0.1:1/wrong-u05-callback"}), {"invalid_grant"})
        issued = self.ok(self.token("oauth:auth", exchange))
        self.claims(issued["access_token"], "access", self.context, client)
        self.rejected(self.token("oauth:auth", exchange), {"invalid_grant"})
        info = self.ok(self.call("GET", "/oauth/userinfo", bearer=issued["access_token"]))
        require(info.get("sub") == self.context["userId"], "OAUTH_USERINFO_MAPPED_SUBJECT")
        old_refresh = issued["refresh_token"]
        self.rejected(self.token("oauth:auth", {"grant_type": "refresh_token", "refresh_token": old_refresh,
                      "scope": "u05-unregistered-scope"}), {"invalid_scope"})
        refreshed = self.ok(self.token("oauth:auth", {"grant_type": "refresh_token", "refresh_token": old_refresh}))
        self.claims(refreshed["access_token"], "access", self.context, client)
        require(refreshed["refresh_token"] != old_refresh, "OAUTH_REFRESH_ROTATION")
        self.rejected(self.token("oauth:auth", {"grant_type": "refresh_token", "refresh_token": old_refresh}), {"invalid_grant"})

    def oauth_client_credentials(self):
        client = self.clients["oauth:cc"]
        form = {"grant_type": "client_credentials", "scope": client.get("scope", "profile")}
        issued = self.ok(self.token("oauth:cc", form))
        claims = self.claims(issued["access_token"], "access", client=client)
        # OAuth client_id is opaque; a machine subject must preserve its literal value.
        require(claims.get("sub") == client["clientId"] and not claims.get("tenant") and not claims.get("org"),
                "MACHINE_OPAQUE_SUBJECT_BOUNDARY")
        response = self.ok(self.call("POST", "/oauth/introspect", form={"token": issued["access_token"]},
                                    basic=(client["clientId"], client["clientSecret"])))
        require(response.get("active") is True, "MACHINE_TOKEN_ACTIVE")
        self.rejected(self.call("GET", "/api/auth/profile", bearer=issued["access_token"]))
        self.rejected(self.call("GET", "/oauth/userinfo", bearer=issued["access_token"]), {"invalid_token"})
        invalid = self.client_form("oauth:cc", form)
        invalid["client_secret"] = secrets.token_urlsafe(40)
        self.rejected(self.call("POST", "/oauth/token", form=invalid), {"invalid_client"})

    def oauth_device(self):
        client = self.clients["oauth:device"]
        issued = self.ok(self.call("POST", "/oauth/device/authorization",
                                  form=self.client_form("oauth:device", {"scope": client.get("scope", "profile")})))
        poll = {"grant_type": DEVICE_GRANT, "device_code": issued["device_code"]}
        self.rejected(self.token("oauth:device", poll), {"authorization_pending"})
        self.rejected(self.token("oauth:device", poll), {"slow_down"})
        user_code = urllib.parse.quote(issued["user_code"], safe="")
        self.ok(self.call("GET", "/api/device/" + user_code, bearer=self.session["accessToken"]))
        self.ok(self.call("POST", "/api/device/" + user_code + "/decision", body={"approve": True}, bearer=self.session["accessToken"]))
        # The last pending poll persisted a timestamp, so approval does not bypass
        # the registered polling interval. Respect it before the successful grant.
        time.sleep(max(5, int(issued["interval"])) + 0.2)
        result = self.ok(self.token("oauth:device", poll))
        self.claims(result["access_token"], "access", self.context, client)
        self.rejected(self.token("oauth:device", poll), {"expired_token"})
        self.rejected(self.call("POST", "/api/device/" + user_code + "/decision", body={"approve": True},
                                bearer=self.session["accessToken"]))

    def old_ids(self):
        old_ids = set()
        for key in ("platformSession", "businessSession"):
            old_ids.add(self.old_state.get(key, {}).get("organization", {}).get("id"))
        old_ids.discard(None)
        require(bool(old_ids), "REAL_OLD_ORGANIZATION_IDS_REQUIRED")
        for old_id in old_ids:
            require(not valid_v7(old_id) and old_id != self.context["organizationId"], "OLD_ID_NEGATIVE_REQUIRED")
            response = self.call("POST", "/api/auth/context", bearer=self.ticket, body={"organizationId": old_id})
            require(response["status"] == 400, "OLD_ORGANIZATION_UUID_VERSION_REJECTION")
        # This negative fixture deliberately changes variant bits; it is never
        # persisted or used to manufacture a target UUID.
        bad_variant = bytearray(uuid.UUID(self.context["organizationId"]).bytes)
        bad_variant[8] &= 0x3f
        invalid = str(uuid.UUID(bytes=bytes(bad_variant)))
        require(not valid_v7(invalid), "BAD_VARIANT_FIXTURE")
        response = self.call("POST", "/api/auth/context", bearer=self.ticket, body={"organizationId": invalid})
        require(response["status"] == 400, "ORGANIZATION_UUID_VARIANT_REJECTION")
        for app in self.config["applications"]:
            old = self.old_state.get("applications", {}).get(app["code"], {}).get("id")
            if old:
                require(not valid_v7(old), "OLD_APPLICATION_NEGATIVE_REQUIRED")
                response = self.call("GET", "/api/applications/" + old, bearer=self.session["accessToken"])
                require(response["status"] == 400, "OLD_APPLICATION_UUID_VERSION_REJECTION")

    def run(self):
        self.check("new_login_and_context", self.login)
        self.check("jwt_and_confidential_secret_rotation", self.client_rotation)
        self.check("old_credential_replays", self.old_credentials)
        self.check("platform_refresh_rotation", self.platform_refresh)
        self.check("application_context_and_authorization", self.applications)
        self.check("oauth_code_pkce_callback_refresh", self.oauth_authorization)
        self.check("oauth_client_credentials_boundary", self.oauth_client_credentials)
        self.check("oauth_device_lifecycle", self.oauth_device)
        self.check("old_uuid_and_variant_rejection", self.old_ids)

    def finish(self, failure=None):
        incomplete = any(value in ("SOURCE_NONEMPTY_REAL_RAW_UNAVAILABLE", "RAW_SAMPLE_ORIGINAL_PKCE_VERIFIER_UNAVAILABLE")
                         for value in self.coverage.values() if isinstance(value, str))
        summary = {"format": "saas-uuid-u05-runtime-result/1", "status": "FAIL" if failure else ("INCOMPLETE" if incomplete else "PASS"),
                   "scope": "QUARANTINED_PREVIEW_FROZEN_MIGRATION_12", "checks": self.checks,
                   "coverage": self.coverage, "httpRequests": len([entry for entry in self.transcript if "method" in entry]),
                   "credentialEvidenceSha256": self.config["credentialEvidenceSha256"],
                   "limitations": ["Operator-supplied DB counts; no DB connection by this tool",
                                   "No external RP account linking, production traffic or final cutover validated"]}
        if failure:
            summary["failedCheck"] = self.current
            summary["failureCode"] = failure
        write_private(self.output / "transcript.private.json", self.transcript)
        if hasattr(self, "session"):
            write_private(self.output / "new-credentials.private.json", {
                "session": self.session, "applicationTokens": getattr(self, "application_tokens", {})})
        write_private(self.output / "summary.json", summary)
        print(json.dumps(summary, ensure_ascii=True))
        return 0 if summary["status"] == "PASS" else 1


def self_test():
    """Offline safety/contract regression tests; no runtime requests or DBs."""
    import tempfile
    import unittest
    from unittest.mock import patch

    class SafetyTests(unittest.TestCase):
        def test_uuid_contract(self):
            self.assertTrue(valid_v7("019a2200-0000-7000-8000-000000000001"))
            for value in (str(uuid.uuid4()), "019a2200-0000-7000-0000-000000000001", None, "invalid"):
                self.assertFalse(valid_v7(value))

        def test_hs256_signature_is_verified_independently_of_expiry(self):
            header = b64encode(b'{"alg":"HS256"}')
            payload = b64encode(b'{"sub":"private-fixture","exp":1}')
            unsigned = header + "." + payload
            signature = b64encode(hmac.new(b"old-key", unsigned.encode(), hashlib.sha256).digest())
            token = unsigned + "." + signature
            self.assertTrue(signature_valid(token, "old-key"))
            self.assertFalse(signature_valid(token, "new-key"))
            self.assertFalse(signature_valid(token + "corrupt", "old-key"))

        def test_real_code_requires_original_verifier_and_checks_available_challenge(self):
            self.assertIsNone(original_pkce_verifier({}))
            self.assertIsNone(original_pkce_verifier({"codeVerifier": "unknown"}))
            verifier = "original-verifier-" + "x" * 43
            sample = {"codeVerifier": verifier, "codeChallengeMethod": "S256",
                      "codeChallenge": b64encode(hashlib.sha256(verifier.encode()).digest())}
            self.assertEqual(original_pkce_verifier(sample), verifier)
            with self.assertRaises(CheckFailed):
                original_pkce_verifier({**sample, "codeVerifier": "A" * 43})

        def test_old_oauth_access_requires_inactive_digest_even_when_userinfo_rejects(self):
            runner = object.__new__(Runner)
            runner.old_secret = "old-key-fixture"
            runner.clients = {"oauth:auth": {"clientId": "opaque-fixture", "clientSecret": "new-secret-fixture",
                                             "clientType": "CONFIDENTIAL"}}
            header = b64encode(b'{"alg":"HS256"}')
            payload = b64encode(b'{"cid":"opaque-fixture","typ":"access","exp":1}')
            unsigned = header + "." + payload
            signature = b64encode(hmac.new(runner.old_secret.encode(), unsigned.encode(), hashlib.sha256).digest())
            sample = {"value": unsigned + "." + signature, "clientKey": "oauth:auth"}
            for active in (False, True):
                responses = iter([{"status": 401, "json": {"error": "invalid_token"}},
                                  {"status": 200, "json": {"active": active}}])
                calls = []

                def call(method, path, **kwargs):
                    calls.append((method, path, kwargs))
                    return next(responses)

                runner.call = call
                if active:
                    with self.assertRaisesRegex(CheckFailed, "OLD_OAUTH_DIGEST_MUST_BE_INACTIVE"):
                        runner.replay_old_oauth_access(sample)
                else:
                    runner.replay_old_oauth_access(sample)
                self.assertEqual([entry[1] for entry in calls], ["/oauth/userinfo", "/oauth/introspect"])
                self.assertEqual(calls[1][2]["basic"], ("opaque-fixture", "new-secret-fixture"))

        def test_old_oauth_client_misbinding_rejected_before_any_http(self):
            runner = object.__new__(Runner)
            runner.old_secret = "old-key-fixture"
            runner.clients = {"wrong-client": {"clientId": "different-opaque-client", "clientSecret": "new-secret-fixture",
                                               "clientType": "CONFIDENTIAL"}}
            header = b64encode(b'{"alg":"HS256"}')
            payload = b64encode(b'{"cid":"original-opaque-client","typ":"access","exp":1}')
            unsigned = header + "." + payload
            signature = b64encode(hmac.new(runner.old_secret.encode(), unsigned.encode(), hashlib.sha256).digest())
            sample = {"value": unsigned + "." + signature, "clientKey": "wrong-client"}
            with patch.object(runner, "call") as call:
                with self.assertRaisesRegex(CheckFailed, "OLD_OAUTH_CLIENT_BINDING"):
                    runner.replay_old_oauth_access(sample)
                call.assert_not_called()

        def test_loopback_no_credentials_or_external_redirect_targets(self):
            self.assertEqual(loopback_origin("http://127.0.0.1:13001"), "http://127.0.0.1:13001")
            for value in ("http://example.test:13001", "http://127.0.0.1:13001/secret", "http://secret@127.0.0.1:13001",
                          "http://127.0.0.1:13001?secret=1", "https://127.0.0.1:13001", "http://localhost:13001"):
                with self.assertRaises(CheckFailed):
                    loopback_origin(value)
            self.assertIsNone(NoRedirect().redirect_request(None, None, 302, "", {}, "http://example.test/secret"))

        @unittest.skipUnless(os.name == "posix", "Linux private file semantics")
        def test_private_file_and_symlink_rejection(self):
            with tempfile.TemporaryDirectory() as folder:
                path = Path(folder) / "input.json"
                write_private(path, {"private": "fixture"})
                self.assertEqual(read_json(str(path)), {"private": "fixture"})
                path.chmod(0o644)
                with self.assertRaises(CheckFailed):
                    read_json(str(path))
                path.chmod(0o600)
                link = Path(folder) / "link.json"
                link.symlink_to(path)
                with self.assertRaises(CheckFailed):
                    read_json(str(link))

        def test_rate_limit_and_server_failure_never_count_as_rejection(self):
            runner = object.__new__(Runner)
            for code in (200, 302, 429, 500, 503):
                with self.assertRaises(CheckFailed):
                    runner.rejected({"status": code, "json": {"error": "invalid_grant"}})
            runner.rejected({"status": 400, "json": {"error": "invalid_grant"}}, {"invalid_grant"})
            with self.assertRaises(CheckFailed):
                runner.rejected({"status": 401, "json": {"error": "invalid_client"}}, {"invalid_grant"})

        def test_public_failure_never_contains_exception_text(self):
            with patch.object(sys, "argv", ["validator", "/secret-does-not-exist", "/private/result"]):
                import io
                capture = io.StringIO()
                with patch("sys.stdout", capture):
                    self.assertEqual(main(), 1)
                self.assertNotIn("secret-does-not-exist", capture.getvalue())
                self.assertNotIn("Traceback", capture.getvalue())

        def test_http_does_not_follow_redirect_and_keeps_response_private(self):
            import http.server
            import threading
            calls = []

            class Handler(http.server.BaseHTTPRequestHandler):
                def do_GET(self):
                    calls.append(self.path)
                    self.send_response(302)
                    self.send_header("Location", "/secret-callback-token")
                    self.end_headers()
                    self.wfile.write(b'{"private":"response-secret"}')

                def log_message(self, *args):
                    pass

            server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                runner = object.__new__(Runner)
                runner.origin = "http://127.0.0.1:" + str(server.server_port)
                runner.current = "fixture_redirect"
                runner.transcript = []
                runner.http = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
                response = runner.call("GET", "/api/fixture", bearer="request-secret")
                self.assertEqual(response["status"], 302)
                self.assertEqual(response["headers"]["location"], "/secret-callback-token")
                self.assertEqual(calls, ["/api/fixture"])
                self.assertEqual(runner.transcript[0]["headers"]["Authorization"], "Bearer request-secret")
                self.assertEqual(response["json"], {"private": "response-secret"})
            finally:
                server.shutdown()
                thread.join(timeout=3)
                server.server_close()

        def test_public_summary_contains_no_private_http_or_claim_values(self):
            import io
            with tempfile.TemporaryDirectory() as folder:
                runner = object.__new__(Runner)
                runner.output = Path(folder)
                runner.config = {"credentialEvidenceSha256": "a" * 64}
                runner.checks = [{"check": "fixture", "status": "PASS"}]
                runner.coverage = {"samples": 2}
                runner.transcript = [{"method": "GET", "token": "private-token-fixture",
                                      "sub": "private-subject-fixture", "url": "private-database-url-fixture"}]
                capture = io.StringIO()
                with patch("sys.stdout", capture):
                    self.assertEqual(runner.finish(), 0)
                for value in ("private-token-fixture", "private-subject-fixture", "private-database-url-fixture"):
                    self.assertNotIn(value, capture.getvalue())
                self.assertIn("private-token-fixture", (Path(folder) / "transcript.private.json").read_text())
                if os.name == "posix":
                    self.assertEqual(stat.S_IMODE((Path(folder) / "transcript.private.json").stat().st_mode), 0o600)

    suite = unittest.defaultTestLoader.loadTestsFromTestCase(SafetyTests)
    result = unittest.TextTestRunner(verbosity=1).run(suite)
    return 0 if result.wasSuccessful() else 1


def main():
    if len(sys.argv) == 2 and sys.argv[1] == "--self-test":
        return self_test()
    runner = None
    try:
        require(len(sys.argv) == 3, "HANDOFF_AND_NEW_OUTPUT_REQUIRED")
        config = read_json(sys.argv[1])
        require(config.get("format") == "saas-uuid-u05-runtime/1", "HANDOFF_FORMAT")
        runner = Runner(config, sys.argv[2])
        runner.run()
        return runner.finish()
    except Exception as error:
        # Never print arbitrary exception messages: urllib, JSON parsers and OS
        # errors may include URLs, raw bodies, token values or private filenames.
        code = str(error) if isinstance(error, CheckFailed) and re.fullmatch(r"[A-Z0-9_]{1,100}", str(error)) else "VALIDATION_INTERNAL_OR_INPUT_ERROR"
        if runner:
            try:
                import traceback
                runner.transcript.append({"privateFailureType": type(error).__name__, "privateFailureTraceback": traceback.format_exc()})
                return runner.finish(code)
            except Exception:
                pass
        print(json.dumps({"status": "FAIL", "failureCode": code}))
        return 1


if __name__ == "__main__":
    sys.exit(main())
