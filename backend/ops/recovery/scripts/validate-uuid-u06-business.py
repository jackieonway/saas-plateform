#!/usr/bin/env python3
"""U06 final/recovery business HTTP probes; never writes the U05 generation."""
import argparse
from datetime import datetime
import json
import os
from pathlib import Path
import re
import secrets
import sys
import time
import urllib.error
import urllib.request

import importlib.util

SPEC = importlib.util.spec_from_file_location("uuid_u06_http_probe", Path(__file__).with_name("validate-uuid-u06-runtime.py"))
HTTP = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HTTP)
RUNTIME, require = HTTP.RUNTIME, HTTP.require
FORMAT = "saas-uuid-u06-business/1"


def config_of(value):
    generation = value.get("generation") if isinstance(value, dict) else None
    require(generation in ("final", "recovery"), "FINAL_OR_RECOVERY_ONLY")
    expected = {"format", "generation", "output_dir", "http_config_file"}
    if generation == "recovery":
        expected |= {"final_receipt_file", "final_receipt_sha256"}
    require(set(value) == expected and value["format"] == FORMAT, "CLOSED_BUSINESS_CONFIG_REQUIRED")
    root = RUNTIME.path_checked(value["output_dir"])
    RUNTIME.private_directory(HTTP.BASE)
    require(root.parent == HTTP.BASE and re.fullmatch("business-" + generation + "-[a-z0-9-]{1,60}", root.name) and not root.exists(), "NEW_FIXED_BUSINESS_OUTPUT_REQUIRED")
    http = HTTP.config_of(RUNTIME.private_json(value["http_config_file"]), require_new_output=False)
    require(http["generation"] == generation, "BUSINESS_HTTP_GENERATION_MISMATCH")
    final = None
    if generation == "recovery":
        final, _ = RUNTIME.checked_json(value["final_receipt_file"], value["final_receipt_sha256"])
        RUNTIME.fields(final, ("format", "generation", "status", "run", "identity_sha256", "store_before", "store_after", "ticket_created", "ticket_updated", "ticket_request", "inbox_rows", "business_contexts"))
        require(final["format"] == "saas-uuid-u06-business-receipt/1" and final["generation"] == "final" and final["status"] == "PASS", "COMPLETE_FINAL_BUSINESS_RECEIPT_REQUIRED")
    return value, http, final


def records(value):
    result = value if isinstance(value, list) else value.get("records", value.get("items", [])) if isinstance(value, dict) else None
    require(isinstance(result, list), "COMPLETE_API_RECORDS_REQUIRED")
    return result


def allowed_request(method, path):
    if method == "GET":
        return bool(re.fullmatch(r"/stores-api/stores(?:\?page=1&size=50|/[0-9a-f-]{36}(?:/staff\?page=1&size=50)?)", path) or re.fullmatch(r"/work-orders-api/tickets/[0-9a-f-]{36}", path) or re.fullmatch(r"/api/notifications\?page=[1-9][0-9]{0,3}&size=100", path))
    if method == "POST":
        return path == "/work-orders-api/tickets"
    if method == "PATCH":
        return bool(re.fullmatch(r"/(?:stores-api/stores|work-orders-api/tickets)/[0-9a-f-]{36}", path))
    return False


def organization_lease(value):
    require(isinstance(value, str) and re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{3}Z", value), "STORE_RESPONSE_LEASE_UTC_REQUIRED")
    try:
        return datetime.fromisoformat(value).timestamp()
    except ValueError:
        raise HTTP.U05.CheckFailed("STORE_RESPONSE_LEASE_UTC_REQUIRED") from None


def validate_store_lease(row, received_at):
    organization = row.get("organization") if isinstance(row, dict) else None
    require(isinstance(organization, dict) and organization.get("contractVersion") == "1.0" and organization.get("organizationType") == "STORE" and organization.get("status") == "ENABLED", "STORE_RESPONSE_ORGANIZATION_CONTRACT_REQUIRED")
    require(all(isinstance(organization.get(k), str) and re.fullmatch("[0-9a-fA-F]{64}", organization[k]) for k in ("contextVersion", "revision")), "STORE_RESPONSE_ORGANIZATION_VERSION_REQUIRED")
    expires_at = organization_lease(organization.get("expiresAt"))
    # This is the platform STORE lookup's per-response three-second lease,
    # validated again by stores before returning the joined business record.
    require(received_at < expires_at <= received_at + 3, "STORE_RESPONSE_LEASE_MUST_BE_FRESH")


def store_rows_equal(left, right):
    def persistent_projection(row):
        require(isinstance(row, dict) and isinstance(row.get("organization"), dict), "STORE_COMPLETE_RESPONSE_REQUIRED")
        organization_lease(row["organization"].get("expiresAt"))
        organization = {k: v for k, v in row["organization"].items() if k != "expiresAt"}
        return {**row, "organization": organization}
    # Every business field and every other organization field remains exact;
    # only this precise response lease is checked independently on receipt.
    return RUNTIME.canonical(persistent_projection(left)) == RUNTIME.canonical(persistent_projection(right))


class Runner(HTTP.Runner):
    def __init__(self, config, http_config, final, create_output=True):
        # HTTP.Runner verifies new epoch secrets before any request. fresh login
        # must verify its JWT against this generation's key before business IO.
        super().__init__(http_config, create_output=False)
        self.output = Path(config["output_dir"])
        self.business_config, self.final = config, final
        self.run_id = "u06-" + self.generation + "-" + str(int(time.time())) + "-" + secrets.token_hex(4)
        self.api_receipts = []
        self.business_tokens, self.business_sessions, self.business_contexts = {}, {}, {}
        self.receipt = {"format": "saas-uuid-u06-business-receipt/1", "generation": self.generation, "status": "RUNNING", "run": self.run_id, "identity_sha256": self.identity_sha}
        if final is not None:
            require(final["identity_sha256"] == self.identity_sha, "ORIGINAL_V7_BUSINESS_IDENTITIES_REQUIRED")
        if create_output:
            self.output.mkdir(mode=0o700)
            self.output.chmod(0o700)
            RUNTIME.write_new(self.output / "request-plan.private.json", json.dumps({"generation": self.generation, "run": self.run_id, "ticket": self.ticket_request()}, indent=2) + "\n")

    def ticket_request(self):
        return {"idempotency_key": self.run_id + "-ticket", "body": {"title": "UUID U06 validation " + self.run_id, "description": "Local U06 controlled recovery validation", "priority": "NORMAL"}}

    def request(self, method, path, *, token, body=None, idempotency=None, status=200):
        require(allowed_request(method, path), "BUSINESS_ENDPOINT_OR_METHOD_REJECTED")
        headers = {"Accept": "application/json", "Origin": self.origin, "Authorization": "Bearer " + token}
        if body is not None:
            headers["Content-Type"] = "application/json"
        if idempotency:
            require(method == "POST" and path == "/work-orders-api/tickets", "IDEMPOTENCY_ONLY_FOR_OWN_CREATE")
            headers["Idempotency-Key"] = idempotency
        request = urllib.request.Request(self.origin + path, json.dumps(body).encode() if body is not None else None, headers, method=method)
        entry = {"method": method, "path": path, "body": body, "idempotency_key": idempotency}
        self.api_receipts.append(entry)
        try:
            try:
                response = self.http.open(request, timeout=20)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                raw = response.read(2 * 1024 * 1024 + 1)
                received_at = time.time()
                require(len(raw) <= 2 * 1024 * 1024, "BUSINESS_RESPONSE_LIMIT")
                value = json.loads(raw)
                entry.update(status=response.code, request_id=response.headers.get("x-request-id"), response_received_at=received_at, response=value)
                if path.startswith(("/stores-api/", "/work-orders-api/")):
                    require(HTTP.U05.valid_v7(entry["request_id"]), "BUSINESS_REQUEST_ID_V7_REQUIRED")
                require(response.code == status, "EXPECTED_BUSINESS_HTTP_STATUS_REQUIRED")
                if re.fullmatch(r"/stores-api/stores/[0-9a-f-]{36}", path):
                    validate_store_lease(value, received_at)
                return value
        finally:
            # Each response is immutable and private; a later failure cannot
            # erase a completed API operation's evidence.
            RUNTIME.write_new(self.output / f"api-{len(self.api_receipts):04d}.private.json", json.dumps(entry, indent=2, ensure_ascii=True) + "\n")

    def establish_business_contexts(self):
        self.login()  # Verifies the actual server uses this generation's JWT.
        for code in ("stores", "work-orders"):
            app = next(a for a in self.config["applications"] if a["code"] == code)
            expected = app.get("context", self.context)
            session = self.establish_session(expected)
            issued = self.ok(self.call("POST", "/api/oauth/session-token", body={"appCode": code}, bearer=session["accessToken"]))
            self.claims(issued["accessToken"], "access", expected, self.clients[app["clientKey"]], code)
            projection = self.ok(self.call("GET", "/api/apps/" + code + "/context", bearer=issued["accessToken"]))
            require(all(projection.get(name, {}).get("id") == expected[field] for name, field in (("user", "userId"), ("tenant", "tenantId"), ("organization", "organizationId"))), "BUSINESS_CONTEXT_ORIGINAL_V7_REQUIRED")
            self.business_tokens[code], self.business_sessions[code], self.business_contexts[code] = issued["accessToken"], session, expected
        if self.final:
            require(self.final["business_contexts"] == self.business_contexts, "RECOVERY_BUSINESS_CONTEXTS_CHANGED")
        self.receipt["business_contexts"] = self.business_contexts

    def identity_fields(self, row, context, ticket=False):
        require(isinstance(row, dict) and all(HTTP.U05.valid_v7(row.get(k)) for k in ("id", "tenantId", "organizationId", "ownerUserId")), "BUSINESS_ROW_V7_IDENTITIES_REQUIRED")
        require(row["tenantId"] == context["tenantId"] and type(row.get("version")) is int, "BUSINESS_ROW_TENANT_VERSION_REQUIRED")
        if ticket:
            require(row["ownerUserId"] == context["userId"] and row["organizationId"] == context["organizationId"], "TICKET_OWNER_ORGANIZATION_REQUIRED")
            require(isinstance(row.get("createdAt"), str) and isinstance(row.get("updatedAt"), str), "TICKET_FULL_TIMESTAMPS_REQUIRED")
        else:
            require(row.get("organization", {}).get("id") == row["organizationId"] and row["organization"].get("name") == row.get("name"), "STORE_PLATFORM_ORGANIZATION_ASSOCIATION_REQUIRED")

    def final_store(self):
        token = self.business_tokens["stores"]
        candidates = records(self.request("GET", "/stores-api/stores?page=1&size=50", token=token))
        require(candidates and HTTP.U05.valid_v7(candidates[0].get("id")), "EXISTING_STORE_REQUIRED")
        path = "/stores-api/stores/" + candidates[0]["id"]
        before = self.request("GET", path, token=token)
        self.identity_fields(before, self.business_contexts["stores"])
        require("businessContact" in before, "STORE_SAFE_VALUE_REQUIRED")
        updated = self.request("PATCH", path, token=token, body={"version": before["version"], "businessContact": before["businessContact"]})
        require(updated["version"] == before["version"] + 1 and all(updated[k] == before[k] for k in ("id", "tenantId", "organizationId", "ownerUserId", "businessContact")), "STORE_SAFE_VERSIONED_UPDATE_REQUIRED")
        after = self.request("GET", path, token=token)
        require(store_rows_equal(after, updated), "STORE_COMPLETE_REREAD_MISMATCH")
        self.receipt.update(store_before=before, store_after=after)

    def create_ticket(self):
        token = self.business_tokens["work-orders"]
        request = self.ticket_request()
        created = self.request("POST", "/work-orders-api/tickets", token=token, body=request["body"], idempotency=request["idempotency_key"], status=201)
        self.identity_fields(created, self.business_contexts["work-orders"], ticket=True)
        require(created.get("status") == "OPEN" and created["version"] == 1 and created.get("description") == request["body"]["description"] and created.get("title") == request["body"]["title"], "NEW_TICKET_INITIAL_VALUES_REQUIRED")
        repeated = self.request("POST", "/work-orders-api/tickets", token=token, body=request["body"], idempotency=request["idempotency_key"], status=201)
        require(repeated == created, "IDEMPOTENCY_MUST_RETURN_COMPLETE_ORIGINAL_ROW")
        self.receipt.update(ticket_request=request, ticket_created=created)
        return created

    def final_ticket(self):
        created = self.create_ticket()
        path = "/work-orders-api/tickets/" + created["id"]
        token = self.business_tokens["work-orders"]
        description = "Updated U06 final receipt " + self.run_id
        updated = self.request("PATCH", path, token=token, body={"version": created["version"], "description": description, "status": "IN_PROGRESS"})
        require(updated["id"] == created["id"] and updated["version"] == created["version"] + 1 and updated.get("status") == "IN_PROGRESS" and updated.get("description") == description and updated.get("createdAt") == created["createdAt"], "VERSIONED_TICKET_UPDATE_REQUIRED")
        self.identity_fields(updated, self.business_contexts["work-orders"], ticket=True)
        reread = self.request("GET", path, token=token)
        require(reread == updated, "FULL_TICKET_REREAD_MISMATCH")
        self.receipt["ticket_updated"] = reread

    def inbox(self):
        rows, total = [], None
        token = self.business_sessions["work-orders"]["accessToken"]
        for page in range(1, 101):
            response = self.request("GET", f"/api/notifications?page={page}&size=100", token=token)
            batch = records(response)
            observed_total = response.get("total") if isinstance(response, dict) else len(batch)
            require(type(observed_total) is int and observed_total >= 0 and (total is None or total == observed_total), "STABLE_INBOX_TOTAL_REQUIRED")
            total = observed_total
            rows.extend(batch)
            if len(rows) >= total:
                break
            require(batch, "INBOX_PAGINATION_INCOMPLETE")
        require(len(rows) == total and len({r["id"] for r in rows}) == len(rows), "COMPLETE_INBOX_REQUIRED")
        for row in rows:
            require(all(row.get(k) is None or HTTP.U05.valid_v7(row[k]) for k in ("id", "publicationId", "applicationId")), "INBOX_V7_REFERENCE_REQUIRED")
            link = row.get("link")
            require(link is None or isinstance(link, str), "INBOX_LINK_TYPE_REQUIRED")
        return sorted(rows, key=lambda r: r["id"])

    def recovery_read(self):
        store = self.request("GET", "/stores-api/stores/" + self.final["store_after"]["id"], token=self.business_tokens["stores"])
        self.identity_fields(store, self.business_contexts["stores"])
        require(store_rows_equal(store, self.final["store_after"]), "RECOVERY_STORE_COMPLETE_ROW_MISMATCH")
        ticket = self.request("GET", "/work-orders-api/tickets/" + self.final["ticket_updated"]["id"], token=self.business_tokens["work-orders"])
        self.identity_fields(ticket, self.business_contexts["work-orders"], ticket=True)
        require(ticket == self.final["ticket_updated"], "RECOVERY_TICKET_COMPLETE_ROW_MISMATCH")
        # Replaying final's original creation request must return the stored
        # original response, while GET above returns the later version.
        request = self.final["ticket_request"]
        replay = self.request("POST", "/work-orders-api/tickets", token=self.business_tokens["work-orders"], body=request["body"], idempotency=request["idempotency_key"], status=201)
        require(replay == self.final["ticket_created"], "RECOVERY_IDEMPOTENCY_RECEIPT_MISMATCH")
        self.receipt.update(preserved_store=store, preserved_ticket=ticket, preserved_idempotency_response=replay)

    def run(self):
        self.check("fresh_u06_business_contexts", self.establish_business_contexts)
        if self.generation == "final":
            self.check("store_same_value_versioned_update", self.final_store)
            self.check("ticket_create_idempotency_update_and_full_read", self.final_ticket)
            self.receipt["inbox_rows"] = self.inbox()
        else:
            self.check("recovery_complete_rows_and_idempotency_preserved", self.recovery_read)
            rows = self.inbox()
            require(rows == self.final["inbox_rows"], "RECOVERY_INBOX_ROWS_AND_LINKS_MISMATCH")
            self.receipt["inbox_rows"] = rows
            self.check("recovery_new_v7_ticket_write", self.create_ticket)
            ticket = self.request("GET", "/work-orders-api/tickets/" + self.receipt["ticket_created"]["id"], token=self.business_tokens["work-orders"])
            require(ticket == self.receipt["ticket_created"], "RECOVERY_NEW_TICKET_REREAD_MISMATCH")
        self.receipt["status"] = "PASS"

    def finish(self, failure=None):
        self.receipt["status"] = "FAIL" if failure else "PASS"
        raw = json.dumps(self.receipt, indent=2, ensure_ascii=True).encode() + b"\n"
        RUNTIME.write_new(self.output / "business-receipt.private.json", raw)
        RUNTIME.write_new(self.output / "authentication.private.json", json.dumps(self.transcript, indent=2, ensure_ascii=True) + "\n")
        summary = {"format": "saas-uuid-u06-business-result/1", "generation": self.generation, "status": self.receipt["status"], "checks": self.checks, "api_requests": len(self.api_receipts), "inbox_rows": len(self.receipt.get("inbox_rows", [])), "receipt_sha256": HTTP.sha(raw), "identity_sha256": self.identity_sha, "database_connections": 0, "notifications_published": 0, "created_tickets": int("ticket_created" in self.receipt), "source_u05_business_writes": 0, "dynamic_response_fields": ["store.organization.expiresAt"], "organization_response_lease_max_seconds": 3}
        if failure:
            summary.update(failed_check=self.current, failure_code=failure)
        RUNTIME.write_new(self.output / "summary.json", json.dumps(summary, indent=2) + "\n")
        return summary


def execute(config_file, check=False):
    require(sys.platform == "linux" and os.geteuid() == 0, "LINUX_ROOT_REQUIRED")
    config, http, final = config_of(RUNTIME.private_json(config_file))
    runner = Runner(config, http, final, create_output=not check)
    if check:
        return {"status": "CHECKED_NO_HTTP", "generation": config["generation"], "http_requests": 0, "business_writes": 0}
    try:
        runner.run()
        return runner.finish()
    except Exception as error:
        code = str(error) if isinstance(error, HTTP.U05.CheckFailed) and re.fullmatch("[A-Z][A-Z0-9_]{1,100}", str(error)) else "PRIVATE_BUSINESS_ACCEPTANCE_FAILED"
        return runner.finish(code)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("config")
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument("--check", action="store_true")
    action.add_argument("--run", action="store_true")
    args = parser.parse_args()
    try:
        result = execute(args.config, args.check)
        print(json.dumps(result, ensure_ascii=True))
        return int(result["status"] == "FAIL")
    except Exception:
        print("U06 business acceptance rejected; inspect fixed private input contract", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
