"""Offline final/recovery business probes against in-memory HTTP row fixtures."""
import copy
from datetime import datetime, timezone
import importlib.util
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("u06_business", Path(__file__).parents[1] / "validate-uuid-u06-business.py")
SPEC.loader.exec_module(B := importlib.util.module_from_spec(SPEC))
USER = "019a1163-0000-7000-8000-000000000001"
TENANT = "019a1163-0000-7000-8000-000000000002"
ORG = "019a1163-0000-7000-8000-000000000003"
STORE = "019a1163-0000-7000-8000-000000000004"
TICKET = "019a1163-0000-7000-8000-000000000005"
CONTEXT = {"userId": USER, "tenantId": TENANT, "organizationId": ORG}


def state():
    lease = datetime.fromtimestamp(time.time() + 2.5, timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")
    return {"store": {"id": STORE, "tenantId": TENANT, "organizationId": ORG, "ownerUserId": USER, "name": "Synthetic store", "organization": {"id": ORG, "name": "Synthetic store", "contractVersion": "1.0", "contextVersion": "a" * 64, "revision": "b" * 64, "organizationType": "STORE", "status": "ENABLED", "expiresAt": lease}, "businessContact": "preserved", "version": 4, "createdAt": "2026-10-01T00:00:00.000Z", "updatedAt": "2026-10-01T00:00:00.000Z"}, "tickets": {}, "idempotency": {}, "notifications": [{"id": USER, "publicationId": TENANT, "applicationId": ORG, "link": "/micro/work-orders/tickets/" + TICKET, "isRead": False}]}


class Synthetic(B.Runner):
    def __init__(self, root, generation, server, final=None):
        self.output = root
        root.mkdir(mode=0o700)
        self.generation, self.server, self.final = generation, server, final
        self.run_id = "u06-" + generation + "-synthetic"
        self.identity_sha, self.current = "a" * 64, "INPUT"
        self.api_receipts, self.transcript, self.checks = [], [], []
        self.business_contexts = {"stores": CONTEXT, "work-orders": CONTEXT}
        self.business_tokens = {"stores": "synthetic-stores-token", "work-orders": "synthetic-orders-token"}
        self.business_sessions = {"work-orders": {"accessToken": "synthetic-session"}}
        self.receipt = {"format": "saas-uuid-u06-business-receipt/1", "generation": generation, "status": "RUNNING", "run": self.run_id, "identity_sha256": self.identity_sha}

    def establish_business_contexts(self):
        self.receipt["business_contexts"] = self.business_contexts

    def request(self, method, path, **kwargs):
        if not B.allowed_request(method, path):
            raise AssertionError("Unexpected HTTP endpoint")
        self.api_receipts.append({"method": method, "path": path, **kwargs})
        if path.startswith("/stores-api/stores?"):
            return {"records": [copy.deepcopy(self.server["store"])]}
        if path.startswith("/stores-api/stores/"):
            if method == "PATCH":
                body = kwargs["body"]
                self.assert_version(body["version"], self.server["store"]["version"])
                self.server["store"].update(businessContact=body["businessContact"], version=body["version"] + 1, updatedAt="2026-10-24T12:00:01.001Z")
            row = copy.deepcopy(self.server["store"])
            row["organization"]["expiresAt"] = datetime.fromtimestamp(time.time() + 2.5, timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")
            B.validate_store_lease(row, time.time())
            return row
        if path.startswith("/api/notifications?"):
            return {"records": copy.deepcopy(self.server["notifications"]), "total": len(self.server["notifications"])}
        if path == "/work-orders-api/tickets":
            key = kwargs["idempotency"]
            if key in self.server["idempotency"]:
                return copy.deepcopy(self.server["idempotency"][key])
            identifier = TICKET if self.generation == "final" else "019a1163-0000-7000-8000-000000000006"
            row = {"id": identifier, "number": "SYNTHETIC-" + self.generation, "title": kwargs["body"]["title"], "description": kwargs["body"]["description"], "priority": "NORMAL", "status": "OPEN", "tenantId": TENANT, "organizationId": ORG, "organizationName": "Synthetic", "ownerUserId": USER, "ownerDisplayName": "Synthetic", "assigneeUserId": None, "assigneeDisplayName": None, "version": 1, "createdAt": "2026-10-24T12:00:00.123Z", "updatedAt": "2026-10-24T12:00:00.123Z"}
            self.server["tickets"][identifier] = copy.deepcopy(row)
            self.server["idempotency"][key] = copy.deepcopy(row)
            return row
        identifier = path.rsplit("/", 1)[1]
        row = self.server["tickets"][identifier]
        if method == "PATCH":
            body = kwargs["body"]
            self.assert_version(body["version"], row["version"])
            row.update(description=body["description"], status=body["status"], version=row["version"] + 1, updatedAt="2026-10-24T12:00:02.234Z")
        return copy.deepcopy(row)

    def assert_version(self, actual, expected):
        if actual != expected:
            raise AssertionError("Unexpected synthetic version")


@unittest.skipUnless(sys.platform == "linux" and os.geteuid() == 0, "Linux private modes required")
class BusinessContract(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.server = state()

    def final_receipt(self):
        runner = Synthetic(self.root / "final", "final", self.server)
        runner.run()
        summary = runner.finish()
        self.assertEqual(summary["status"], "PASS")
        return runner

    def test_final_versioned_store_ticket_update_idempotency_and_private_full_receipt(self):
        runner = self.final_receipt()
        receipt = runner.receipt
        self.assertEqual(receipt["store_after"]["version"], 5)
        self.assertEqual(receipt["store_before"]["businessContact"], receipt["store_after"]["businessContact"])
        self.assertEqual(receipt["ticket_created"]["version"], 1)
        self.assertEqual(receipt["ticket_updated"]["version"], 2)
        self.assertEqual(receipt["ticket_updated"]["status"], "IN_PROGRESS")
        self.assertEqual(receipt["ticket_updated"]["createdAt"], receipt["ticket_created"]["createdAt"])
        self.assertEqual(len(self.server["tickets"]), 1)
        self.assertEqual(receipt["inbox_rows"], self.server["notifications"])
        self.assertEqual((runner.output / "business-receipt.private.json").stat().st_mode & 0o777, 0o600)
        self.assertTrue(all(r["method"] == "GET" for r in runner.api_receipts if r["path"].startswith("/api/notifications")))

    def test_recovery_preserves_complete_final_rows_and_creates_one_new_v7_ticket(self):
        final = self.final_receipt().receipt
        runner = Synthetic(self.root / "recovery", "recovery", self.server, final)
        runner.run()
        result = runner.finish()
        self.assertEqual(result["status"], "PASS")
        self.assertEqual(runner.receipt["preserved_ticket"], final["ticket_updated"])
        self.assertTrue(B.store_rows_equal(runner.receipt["preserved_store"], final["store_after"]))
        self.assertEqual(runner.receipt["preserved_idempotency_response"], final["ticket_created"])
        self.assertEqual(len(self.server["tickets"]), 2)
        self.assertNotEqual(runner.receipt["ticket_created"]["id"], final["ticket_created"]["id"])

    def test_recovery_rejects_changed_timestamp_version_owner_store_and_inbox_link(self):
        final = self.final_receipt().receipt
        changes = (("ticket-time", lambda s: s["tickets"][TICKET].update(updatedAt="changed")), ("ticket-version", lambda s: s["tickets"][TICKET].update(version=3)), ("ticket-owner", lambda s: s["tickets"][TICKET].update(ownerUserId=TENANT)), ("store-version", lambda s: s["store"].update(version=6)), ("store-owner", lambda s: s["store"].update(ownerUserId=TENANT)), ("inbox-link", lambda s: s["notifications"][0].update(link="/changed")))
        for name, mutate in changes:
            server = copy.deepcopy(self.server)
            mutate(server)
            runner = Synthetic(self.root / name, "recovery", server, final)
            with self.subTest(change=name), self.assertRaises(B.HTTP.U05.CheckFailed):
                runner.run()
            self.assertEqual(len(server["tickets"]), 1)

    def test_recovery_rejects_missing_original_idempotency_response(self):
        final = self.final_receipt().receipt
        self.server["idempotency"][final["ticket_request"]["idempotency_key"]]["createdAt"] = "changed"
        runner = Synthetic(self.root / "recovery", "recovery", self.server, final)
        with self.assertRaises(B.HTTP.U05.CheckFailed):
            runner.run()

    def test_source_generation_and_unknown_fields_rejected_without_http(self):
        for config in ({"format": B.FORMAT, "generation": "source", "output_dir": "/tmp/source", "http_config_file": "/tmp/no"}, {"format": B.FORMAT, "generation": "final", "unknown": "x"}):
            with self.assertRaises(B.HTTP.U05.CheckFailed):
                B.config_of(config)

    def test_request_allowlist_forbids_notification_publish_remind_delete_external_and_source_paths(self):
        for method, path in (("POST", "/api/notifications"), ("POST", "/work-orders-api/tickets/" + TICKET + "/remind"), ("DELETE", "/work-orders-api/tickets/" + TICKET), ("GET", "http://external.invalid/"), ("PATCH", "/api/applications/" + ORG), ("PUT", "/api/notifications/read-all")):
            self.assertFalse(B.allowed_request(method, path))

    def test_actual_transport_records_private_full_response_and_real_v7_request_id(self):
        runner = B.Runner.__new__(B.Runner)
        runner.output = self.root
        runner.origin = "http://127.0.0.1:13000"
        runner.api_receipts = []
        class Response(io.BytesIO):
            code = 200
            headers = {"x-request-id": USER}
        class Opener:
            def open(self, request, timeout):
                self.request = request
                return Response(json.dumps(state()["store"]).encode())
        runner.http = Opener()
        row = runner.request("GET", "/stores-api/stores/" + STORE, token="synthetic-no-real-token")
        self.assertTrue(B.store_rows_equal(row, state()["store"]))
        receipt = json.loads((self.root / "api-0001.private.json").read_text())
        self.assertEqual(receipt["response"], row)
        self.assertEqual(receipt["request_id"], USER)
        self.assertEqual((self.root / "api-0001.private.json").stat().st_mode & 0o777, 0o600)
        self.assertGreater(receipt["response_received_at"], 0)

    def test_real_patch_get_lease_difference_preserves_every_other_response_field(self):
        # Exact lease timestamps observed in final attempt01, with synthetic
        # identities/business data; no real subjects or row values are embedded.
        patched = state()["store"]
        reread = copy.deepcopy(patched)
        patched["organization"]["expiresAt"] = "2026-10-24T12:44:08.949Z"
        reread["organization"]["expiresAt"] = "2026-10-24T12:44:09.065Z"
        observed = datetime(2026, 10, 24, 12, 44, 6, 100000, timezone.utc).timestamp()
        B.validate_store_lease(patched, observed)
        B.validate_store_lease(reread, observed + 0.116)
        self.assertNotEqual(patched, reread)
        self.assertTrue(B.store_rows_equal(patched, reread))
        for mutate in (lambda r: r.update(version=r["version"] + 1), lambda r: r.update(ownerUserId=TENANT), lambda r: r.update(updatedAt="changed"), lambda r: r["organization"].update(revision="c" * 64), lambda r: r["organization"].update(contextVersion="d" * 64), lambda r: r["organization"].update(addedField="unreviewed")):
            changed = copy.deepcopy(reread)
            mutate(changed)
            self.assertFalse(B.store_rows_equal(patched, changed))

    def test_organization_lease_must_be_utc_milliseconds_fresh_and_at_most_three_seconds(self):
        observed = datetime(2026, 10, 24, 12, 44, 6, tzinfo=timezone.utc).timestamp()
        for value in (None, "invalid", "2026-10-24T12:44:08.000+00:00", "2026-10-24T12:44:06.000Z", "2026-10-24T12:44:09.001Z", "2026-99-24T12:44:08.000Z"):
            row = state()["store"]
            row["organization"]["expiresAt"] = value
            with self.subTest(lease=value), self.assertRaises(B.HTTP.U05.CheckFailed):
                B.validate_store_lease(row, observed)


if __name__ == "__main__":
    unittest.main()
