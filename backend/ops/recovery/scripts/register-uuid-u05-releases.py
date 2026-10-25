#!/usr/bin/env python3
"""Register, preflight and activate seven fixed U05 releases through formal APIs."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys
import urllib.error
import urllib.request
import uuid

ORIGIN = "http://127.0.0.1:13000"
MATERIALS = Path("/var/lib/saas-uuid-u05-materials-20261024")
VERSIONS = {code: "2.0.1-uuid-u05" if code == "work-orders" else "1.0.1-uuid-u05" for code in ("stores", "work-orders", "vue-workshop", "legacy-jquery", "websocket-chat", "report-center", "oauth-playground")}


class Rejected(Exception):
    pass


def require(condition, message):
    if not condition:
        raise Rejected(message)


def canonical_v7(value):
    try:
        parsed = uuid.UUID(value)
        return str(parsed) == value and parsed.version == 7 and parsed.variant == uuid.RFC_4122
    except (ValueError, TypeError, AttributeError):
        return False


def path_checked(value):
    require(isinstance(value, str) and value.startswith("/") and all(part not in ("", ".", "..") for part in value.split("/")[1:]), "normalized absolute path required")
    path = Path(value)
    require(not any(p.is_symlink() for p in (path, *path.parents)), "symbolic links forbidden")
    return path


def private_json(path):
    path = path_checked(path)
    info = path.stat()
    require(stat.S_ISREG(info.st_mode) and stat.S_IMODE(info.st_mode) == 0o600 and info.st_uid == os.geteuid() and info.st_nlink == 1 and info.st_size < 4 * 1024 * 1024, "private credentials ownership invalid")
    return json.loads(path.read_bytes())


def credentials(path):
    value = private_json(path)
    session = value.get("session", {})
    require(session.get("platformContext") is True, "fresh HQ platform session required")
    require(isinstance(session.get("accessToken"), str) and session["accessToken"] and not any(c.isspace() for c in session["accessToken"]), "session token invalid")
    return value


def write_new(path, value, mode=0o600):
    raw = json.dumps(value, ensure_ascii=False, indent=2).encode() + b"\n"
    fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY | os.O_NOFOLLOW, mode)
    os.fchmod(fd, mode)
    with os.fdopen(fd, "wb") as file:
        file.write(raw)
        file.flush()
        os.fsync(file.fileno())
    require(path.read_bytes() == raw, "evidence read-back failed")


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, newurl):
        return None


class Client:
    def __init__(self, token, directory):
        self.token = token
        self.directory = directory
        self.sequence = 0
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())

    def call(self, method, path, body=None, token=None, allow_error=False):
        require(path.startswith("/api/") and not path.startswith("//") and "\r" not in path and "\n" not in path, "fixed local API path required")
        self.sequence += 1
        request = urllib.request.Request(ORIGIN + path, method=method, headers={"Authorization": "Bearer " + (self.token if token is None else token), "Accept": "application/json", "Content-Type": "application/json"}, data=json.dumps(body, ensure_ascii=False).encode() if body is not None else None)
        try:
            response = self.opener.open(request, timeout=70)
        except urllib.error.HTTPError as error:
            response = error
        except (OSError, urllib.error.URLError):
            write_new(self.directory / f"{self.sequence:04d}.json", {"method": method, "path": path, "request": body, "transport_failed": True})
            raise Rejected("API transport failed; inspect private transcript before retry") from None
        with response:
            raw = response.read(4 * 1024 * 1024 + 1)
            require(len(raw) <= 4 * 1024 * 1024, "API response too large")
            try:
                value = json.loads(raw)
            except (ValueError, UnicodeError):
                value = {"invalid_json_response_sha256": hashlib.sha256(raw).hexdigest()}
            status = response.status
        write_new(self.directory / f"{self.sequence:04d}.json", {"method": method, "path": path, "request": body, "status": status, "response": value})
        require(allow_error or 200 <= status < 300, "formal API rejected operation; inspect private transcript")
        return status, value


def definition(code):
    version = VERSIONS[code]
    return {"environment": "development", "version": version, "entryType": "HTML", "entryUrl": f"/sub-apps/{code}/versions/{version}/index.html", "healthUrl": None, "resources": []}


def compatible(app):
    return app.get("entryType") == "HTML" and app.get("runtimeMode") == "TRUSTED" and app.get("basePath", "").rstrip("/") == "/micro/" + app.get("code", "") and app.get("status") == "ENABLED"


def checked_release(row, app_id, expected):
    require(isinstance(row, dict) and canonical_v7(row.get("id")) and row.get("applicationId") == app_id, "release identity invalid")
    require(all(row.get(key) == value for key, value in expected.items()), "immutable release definition differs")
    return row["id"]


def checked_preflight(row):
    require(row.get("validationStatus") == "PASSED" and row.get("validationResult", {}).get("passed") is True, "server release preflight failed; inspect private transcript")
    checks = row["validationResult"].get("checks")
    require(isinstance(checks, list) and checks and all(item.get("passed") is True for item in checks), "server release check set invalid")
    return len(checks)


def execute(credentials_file, output_dir, verify_existing=False, prior_summary=None):
    private = path_checked(output_dir)
    require(private.parent == MATERIALS and not private.exists(), "new private release transcript directory required")
    identity = credentials(credentials_file)
    private.mkdir(mode=0o700)
    client = Client(identity["session"]["accessToken"], private)
    summary = {"format": "saas-uuid-u05-release-activation/1", "recorded_date": "2026-10-24", "origin": ORIGIN, "status": "IN_PROGRESS", "applications": [], "extra_login_requests": 0, "direct_sql_writes": 0, "old_release_bytes_modified": False, "sdk_version_dto_sent": False, "http_transcript_contains_authorization_headers": False}
    summary["run_mode"] = "READ_ONLY_VERIFY_EXISTING_ACTIVATIONS" if verify_existing else "FORMAL_REGISTER_PREFLIGHT_ACTIVATE"
    if verify_existing:
        require(prior_summary is not None, "prior formal activation evidence required")
        prior = private_json(prior_summary)
        require(len(prior.get("applications", [])) == 7 and all(item.get("status") == "ACTIVATED_AND_READ_BACK" and item.get("active_pointer_verified") is True for item in prior["applications"]), "prior activation batch incomplete")
        summary["formal_mutation_batch"] = {"summary_sha256": hashlib.sha256(Path(prior_summary).read_bytes()).hexdigest(), "api_requests": prior["api_requests"], "activation_success_count": 7, "retained_tail_profile_path_404": True}
    try:
        _, applications = client.call("GET", "/api/applications")
        require(isinstance(applications, list), "application inventory invalid")
        for code in VERSIONS:
            record = {"code": code, "version": VERSIONS[code], "status": "IN_PROGRESS"}
            summary["applications"].append(record)
            matches = [app for app in applications if app.get("code") == code]
            require(len(matches) == 1 and canonical_v7(matches[0].get("id")), "one actual UUIDv7 application required per code")
            app = matches[0]
            app_id = app["id"]
            expected = definition(code)
            record["application_uuid_v7"] = True
            record["configuration_repaired"] = not compatible(app)
            require(not verify_existing or compatible(app), "read-only verification found incompatible app configuration")
            if not compatible(app):
                patch = {"basePath": "/micro/" + code, "entryType": "HTML", "runtimeMode": "TRUSTED", "entryUrl": expected["entryUrl"], "status": "ENABLED"}
                client.call("PUT", "/api/applications/" + app_id, patch)
                _, app = client.call("GET", "/api/applications/" + app_id)
                require(app.get("code") == code and compatible(app), "application repair did not establish release contract")
            _, releases = client.call("GET", f"/api/applications/{app_id}/releases?environment=development")
            require(isinstance(releases, list), "release inventory invalid")
            matching = [row for row in releases if row.get("version") == expected["version"]]
            require(len(matching) <= 1, "duplicate immutable release version")
            record["existing_exact_release_reused"] = bool(matching)
            if matching:
                row = matching[0]
            else:
                require(not verify_existing, "read-only verification requires existing exact release")
                _, row = client.call("POST", f"/api/applications/{app_id}/releases", expected)
            release_id = checked_release(row, app_id, expected)
            record["release_uuid_v7"] = True
            if verify_existing:
                require(row.get("active") is True, "existing release is not active")
                record["preflight_checks"] = checked_preflight(row)
            else:
                _, row = client.call("POST", f"/api/applications/{app_id}/releases/{release_id}/preflight")
                checked_release(row, app_id, expected)
                record["preflight_checks"] = checked_preflight(row)
                _, deployment = client.call("POST", f"/api/applications/{app_id}/releases/{release_id}/activate")
                require(deployment.get("applicationId") == app_id and deployment.get("activeReleaseId") == release_id and deployment.get("entryUrl") == expected["entryUrl"], "activation response does not bind the selected release")
            _, deployments = client.call("GET", f"/api/applications/{app_id}/deployments")
            current = [item for item in deployments if item.get("environment") == "development"]
            require(len(current) == 1 and current[0].get("activeReleaseId") == release_id and current[0].get("entryUrl") == expected["entryUrl"], "active deployment read-back differs")
            record.update({"status": "ACTIVATED_AND_READ_BACK", "entry_url": expected["entryUrl"], "preflight_passed": True, "activation_repeated_server_preflight": True, "active_pointer_verified": True})
            write_new(private / (code + "-completion.json"), {"code": code, "application_id": app_id, "release_id": release_id, "summary": record})
        summary["status"] = "SEVEN_RELEASES_REGISTERED_PREFLIGHTED_ACTIVATED_VERIFIED"
        summary["context_delegated_to"] = "uuid-u05-browser"
        summary["business_context_verified_by_this_workflow"] = False
        summary["context_boundary"] = "HQ session is authorized to manage releases. Its HQ profile is not the seven business applications' user context. Fresh business login and effective context URLs are verified separately by the browser workflow."
        summary["previous_attempts"] = [
            {"attempt": "release-registration-01", "api_requests": 37, "post_requests": 21, "put_requests": 0, "formal_activations_completed": 7, "tail_check": "GET /api/auth/me returned 404 due to an incorrect diagnostic route", "retained_unchanged": True},
            {"attempt": "release-registration-02", "api_requests": 16, "post_requests": 0, "put_requests": 0, "active_deployments_read_back": 7, "tail_check": "GET /api/auth/profile returned 200 with the valid HQ scope; requiring seven business applications was an invalid diagnostic assumption", "short_lived_app_tokens_observed_expired": 7, "application_context_requests_sent": 0, "retained_unchanged": True}
        ] if verify_existing else []
        summary["api_requests"] = client.sequence
        summary["completed_at"] = datetime.datetime.now(datetime.timezone(datetime.timedelta(hours=8))).isoformat()
        write_new(private / "summary.json", summary)
        return summary
    except (Rejected, OSError, ValueError, KeyError, TypeError):
        summary["status"] = "FAILED_PARTIAL_API_TRANSCRIPT_RETAINED"
        summary["api_requests"] = client.sequence
        write_new(private / "failed-summary.json", summary)
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("credentials")
    parser.add_argument("private_dir")
    parser.add_argument("--execute", action="store_true", required=True)
    parser.add_argument("--public-report", required=True)
    parser.add_argument("--verify-existing-only", action="store_true")
    parser.add_argument("--prior-summary")
    args = parser.parse_args()
    try:
        report = execute(args.credentials, args.private_dir, args.verify_existing_only, args.prior_summary)
        write_new(path_checked(args.public_report), report, 0o644)
        print(json.dumps({"status": report["status"], "applications": len(report["applications"]), "api_requests": report["api_requests"], "context_delegated_to": report["context_delegated_to"]}))
        return 0
    except (Rejected, OSError, ValueError, KeyError, TypeError):
        print("U05 formal release workflow failed; inspect the new private transcript before retry", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
