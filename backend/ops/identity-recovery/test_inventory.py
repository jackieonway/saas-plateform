"""Pure canonical inventory tests: no database, files, sockets or containers."""
import copy
import json
from pathlib import Path
import re
import unittest
import uuid

import inventory as inv


ID = "019a1ec0-0000-7000-8000-000000000001"
TIME = "2026-10-28T00:00:00+00:00"
LATER = "2026-10-29T00:00:00+00:00"


def sample(kind, table, **overrides):
    spec = inv.TABLES[kind][table]
    row = {}
    for key, field in spec.fields.items():
        if field.nullable:
            row[key] = None
        elif field.redacted or key.endswith(("_hash", "_digest")):
            row[key] = "a" * 64
        elif field.pg_type == "uuid":
            row[key] = ID
        elif field.pg_type.startswith("_"):
            row[key] = []
        elif field.pg_type in ("int2", "int4", "int8"):
            row[key] = 1
        elif field.pg_type == "bool":
            row[key] = False
        elif field.pg_type in ("timestamptz", "timestamp"):
            row[key] = LATER if "expires" in key else TIME
        else:
            row[key] = "public-metadata"
    if "issuer" in row:
        row["issuer"] = "https://auth.saas.test"
    if "status" in row:
        row["status"] = "ENABLED"
    if table == "auth_signing_keys":
        row.update(kid="rotation-A", n="public-modulus", e="AQAB", active=False)
    if table == "auth_delivery_jobs":
        row.update(kind="LOGOUT", state="PENDING", max_attempts=8, attempt_count=0,
                   target_id=ID, target_version=1, grant_id=ID)
    if table == "auth_consents":
        row.update(status="ACTIVE", authorized_at=TIME)
    if table.endswith("backchannel_logout_sources"):
        row.update(sid_digest="b" * 64, sub_digest="c" * 64)
    if table in ("bff_sessions", "portal_sessions"):
        row.update(oidc_public_sid="b" * 64, oidc_public_sub="opaque-subject")
    row.update(overrides)
    return row


def value_with(kind, table, row):
    value = inv.empty_inventory()
    value["databases"][kind][table] = [row]
    return value


def raw_empty(kind):
    return {"catalog": {name: {"columns": {
        field.source: {"type": field.pg_type, "nullable": field.nullable}
        for field in spec.fields.values()}, "pk": list(spec.pk)}
        for name, spec in inv.TABLES[kind].items()},
        "tables": {name: [] for name in inv.TABLES[kind]}}


class InventoryTests(unittest.TestCase):
    def reject(self, action, category):
        with self.assertRaisesRegex(inv.InventoryError, "^INVENTORY_" + category + "$"):
            action()

    def test_explicit_canonical_table_coverage_matches_current_sources(self):
        backend = Path(__file__).resolve().parents[2]
        source = (backend / "migrations/0001_init.sql").read_text(encoding="utf-8")
        canonical = set(re.findall(r"CREATE TABLE public\.(auth_\w+)\s*\(", source))
        self.assertEqual(canonical, {table for table in inv.TABLES["platform"]
                                    if table.startswith("auth_")})
        portal = backend.parents[1] / "saas-customer-portal/backend/migrations/0001_customer_portal.sql"
        portal_tables = set(re.findall(r"CREATE TABLE public\.(\w+)\s*\(", portal.read_text(encoding="utf-8")))
        self.assertEqual(portal_tables, set(inv.TABLES["portal"]))
        self.assertNotIn("oidc_clients", inv.TABLES["platform"])
        self.assertNotIn("oauth_clients", inv.TABLES["platform"])

    def test_field_contract_preserves_critical_boundary_columns(self):
        tables = inv.TABLES["platform"]
        self.assertEqual(tables["platform_app_capabilities"].pk, ("token_hash",))
        self.assertEqual(tables["auth_client_permissions"].pk, ("client_id", "resource_id", "scope_id"))
        self.assertTrue(tables["auth_logout_command_receipts"].append_only)
        self.assertTrue(tables["platform_backchannel_logout_sources"].append_only)
        self.assertTrue(tables["bff_sessions"].fields["refresh_uncertain"].pg_type == "bool")
        self.assertIn("logout_confirmed_at", tables["bff_sessions"].fields)
        self.assertIn("selection_sha256", tables["context_selections"].fields)

    def test_membership_and_location_columns_match_current_ddl_completely(self):
        source = (Path(__file__).resolve().parents[2] / "migrations/0001_init.sql").read_text()
        for table in ("memberships", "organizations", "tenants"):
            with self.subTest(table=table):
                body = re.search(r"CREATE TABLE public\." + table + r" \((.*?)\n\);", source, re.S)
                self.assertIsNotNone(body)
                columns = {match.group(1) or match.group(2) for match in re.finditer(
                    r'^\s+(?:"([a-z_][a-z0-9_]*)"|([a-z_][a-z0-9_]*))\s+[a-z]', body.group(1), re.M)}
                self.assertEqual(columns, {field.source for field in inv.TABLES["platform"][table].fields.values()})
        self.assertEqual(inv.TABLES["platform"]["memberships"].fields["position_sha256"],
                         inv.Field("position", "varchar", True, True))
        for table in ("organizations", "tenants"):
            for coordinate in ("latitude", "longitude"):
                self.assertEqual(inv.TABLES["platform"][table].fields[coordinate + "_sha256"],
                                 inv.Field(coordinate, "float8", True, True))

    def test_nonnull_position_and_coordinates_are_hashed_before_witness(self):
        for table, updates in (("memberships", {"position": "PRIVATE_POSITION_CANARY"}),
                               ("organizations", {"latitude": 12.3456789, "longitude": 123.456789}),
                               ("tenants", {"latitude": -12.3456789, "longitude": -123.456789})):
            with self.subTest(table=table):
                projected = sample("platform", table)
                raw = {field.source: projected[name] for name, field in inv.TABLES["platform"][table].fields.items()}
                raw.update(updates)
                observed = inv.project_row("platform", table, raw)
                inv.validate_inventory(value_with("platform", table, observed))
                for column, original in updates.items():
                    self.assertNotIn(column, observed)
                    self.assertEqual(observed[column + "_sha256"], inv.digest(original))
                    self.assertNotIn(str(original), json.dumps(observed))

    def test_empty_has_every_family_and_is_valid(self):
        value = inv.empty_inventory()
        self.assertIs(inv.validate_inventory(value), value)
        self.assertEqual(inv.highwater(value), {})

    def test_format_missing_and_extra_families_are_rejected(self):
        value = inv.empty_inventory()
        del value["databases"]["platform"]["auth_invitations"]
        self.reject(lambda: inv.validate_inventory(value), "FAMILIES_INVALID")
        value = inv.empty_inventory()
        value["databases"]["portal"]["legacy"] = []
        self.reject(lambda: inv.validate_inventory(value), "FAMILIES_INVALID")

    def test_unknown_and_missing_columns_are_rejected(self):
        row = sample("platform", "users")
        row["password_hash"] = "must-not-enter"
        value = value_with("platform", "users", row)
        self.reject(lambda: inv.validate_inventory(value), "COLUMNS_INVALID")
        del row["password_hash"]
        del row["session_version"]
        self.reject(lambda: inv.validate_inventory(value), "COLUMNS_INVALID")

    def test_duplicate_pk_and_uuid_v4_rejected(self):
        value = value_with("platform", "users", sample("platform", "users"))
        value["databases"]["platform"]["users"] *= 2
        self.reject(lambda: inv.validate_inventory(value), "DUPLICATE_PK")
        value["databases"]["platform"]["users"] = [sample("platform", "users", id=str(uuid.uuid4()))]
        self.reject(lambda: inv.validate_inventory(value), "UUID_INVALID")

    def test_booleans_are_not_version_integers(self):
        value = value_with("platform", "users", sample("platform", "users", session_version=True))
        self.reject(lambda: inv.validate_inventory(value), "INTEGER_INVALID")

    def test_timestamp_requires_actual_date_and_zone(self):
        value = value_with("platform", "users", sample("platform", "users", created_at="2026-99-99T00:00:00+00:00"))
        self.reject(lambda: inv.validate_inventory(value), "TIMESTAMP_INVALID")
        value["databases"]["platform"]["users"][0]["created_at"] = "2026-10-28T00:00:00"
        self.reject(lambda: inv.validate_inventory(value), "TIMESTAMP_ZONE_REQUIRED")

    def test_highwater_covers_counters_and_key_validation_deadline(self):
        value = value_with("platform", "users", sample("platform", "users", session_version=12))
        value["databases"]["platform"]["auth_signing_keys"] = [sample("platform", "auth_signing_keys", last_signed_exp=LATER)]
        floor = inv.highwater(value)
        self.assertEqual(floor['platform/users/["' + ID + '"]/session_version'], 12)
        self.assertIsInstance(floor['platform/auth_signing_keys/["rotation-A"]/last_signed_exp'], int)

    def test_counter_and_key_window_cannot_regress(self):
        previous = value_with("platform", "users", sample("platform", "users", session_version=7))
        value = copy.deepcopy(previous)
        value["databases"]["platform"]["users"][0]["session_version"] = 6
        self.reject(lambda: inv.validate_inventory(value, previous), "HIGHWATER_REGRESSION")
        previous = value_with("platform", "auth_signing_keys", sample("platform", "auth_signing_keys", last_signed_exp=LATER))
        value = copy.deepcopy(previous)
        value["databases"]["platform"]["auth_signing_keys"][0]["last_signed_exp"] = TIME
        self.reject(lambda: inv.validate_inventory(value, previous), "KEY_WINDOW_REGRESSION")

    def test_key_material_cannot_be_rebound(self):
        previous = value_with("platform", "auth_signing_keys", sample("platform", "auth_signing_keys"))
        value = copy.deepcopy(previous)
        value["databases"]["platform"]["auth_signing_keys"][0]["n"] = "other-public-modulus"
        self.reject(lambda: inv.validate_inventory(value, previous), "IMMUTABLE_FACT_CHANGED")

    def test_source_tombstone_cannot_be_deleted_or_changed(self):
        previous = value_with("portal", "portal_backchannel_logout_sources", sample("portal", "portal_backchannel_logout_sources"))
        value = inv.empty_inventory()
        self.reject(lambda: inv.validate_inventory(value, previous), "DURABLE_FACT_OMITTED")
        value = copy.deepcopy(previous)
        value["databases"]["portal"]["portal_backchannel_logout_sources"][0]["sub_digest"] = "d" * 64
        self.reject(lambda: inv.validate_inventory(value, previous), "IMMUTABLE_FACT_CHANGED")

    def test_delivery_attempt_budget_and_terminal_state_cannot_reset(self):
        previous = value_with("platform", "auth_delivery_jobs", sample("platform", "auth_delivery_jobs", attempt_count=3))
        value = copy.deepcopy(previous)
        value["databases"]["platform"]["auth_delivery_jobs"][0]["attempt_count"] = 2
        self.reject(lambda: inv.validate_inventory(value, previous), "HIGHWATER_REGRESSION")
        previous["databases"]["platform"]["auth_delivery_jobs"][0]["state"] = "TERMINAL"
        value = copy.deepcopy(previous)
        value["databases"]["platform"]["auth_delivery_jobs"][0]["state"] = "PENDING"
        self.reject(lambda: inv.validate_inventory(value, previous), "TERMINAL_FACT_CHANGED")

    def test_delivered_job_may_attach_original_consent_event_once(self):
        previous = value_with("platform", "auth_delivery_jobs", sample("platform", "auth_delivery_jobs", state="DELIVERED"))
        value = copy.deepcopy(previous)
        value["databases"]["platform"]["auth_delivery_jobs"][0]["consent_event_id"] = ID
        inv.validate_inventory(value, previous)
        changed = copy.deepcopy(value)
        changed["databases"]["platform"]["auth_delivery_jobs"][0]["consent_event_id"] = None
        self.reject(lambda: inv.validate_inventory(changed, value), "CONSENT_EVENT_CHANGED")

    def test_job_cannot_omit_its_target_or_source(self):
        value = value_with("platform", "auth_delivery_jobs", sample("platform", "auth_delivery_jobs", target_id=None, target_version=None))
        self.reject(lambda: inv.validate_inventory(value), "DELIVERY_TARGET_INVALID")
        value = value_with("platform", "auth_delivery_jobs", sample("platform", "auth_delivery_jobs", grant_id=None))
        self.reject(lambda: inv.validate_inventory(value), "DELIVERY_SOURCE_INVALID")

    def test_old_bff_logout_operation_cannot_change(self):
        previous = value_with("platform", "bff_sessions", sample("platform", "bff_sessions", logout_request_id=ID, logout_scope="GLOBAL", logout_confirmed_at=TIME))
        value = copy.deepcopy(previous)
        value["databases"]["platform"]["bff_sessions"][0]["logout_scope"] = "PLATFORM"
        self.reject(lambda: inv.validate_inventory(value, previous), "LOGOUT_OPERATION_CHANGED")

    def test_bff_nullable_groups_cannot_be_partly_cleared(self):
        value = value_with("portal", "portal_sessions", sample("portal", "portal_sessions", membership_id=ID))
        self.reject(lambda: inv.validate_inventory(value), "BFF_PAIR_INVALID")

    def test_secret_projection_covers_phc_json_and_callback_material(self):
        raw = raw_empty("platform")
        projected = sample("platform", "users")
        record = {field.source: projected[key] for key, field in inv.TABLES["platform"]["users"].fields.items()}
        record["password_hash"] = "$argon2id$fixture-PHC-not-a-valid-credential"
        record["username"] = "fixture-private-name"
        raw["tables"]["users"] = [record]
        result = inv.project_database("platform", raw)
        encoded = json.dumps(result)
        self.assertNotIn("$argon2id$", encoded)
        self.assertNotIn("fixture-private-name", encoded)
        self.assertEqual(result["users"][0]["password_hash_sha256"], inv.digest(record["password_hash"]))
        self.assertTrue(inv.TABLES["platform"]["auth_authorization_requests"].fields["state_value_sha256"].redacted)
        self.assertTrue(inv.TABLES["platform"]["auth_authorization_requests"].fields["nonce_value_sha256"].redacted)
        self.assertTrue(inv.TABLES["platform"]["auth_logout_command_receipts"].fields["result_json_sha256"].redacted)

    def test_live_catalog_type_nullability_pk_and_extra_auth_drift_rejected(self):
        raw = raw_empty("portal")
        raw["catalog"]["portal_sessions"]["columns"]["account_version"]["type"] = "text"
        self.reject(lambda: inv.project_database("portal", raw), "SCHEMA_COLUMN_DRIFT")
        raw = raw_empty("portal")
        raw["catalog"]["portal_sessions"]["pk"] = ["user_id"]
        self.reject(lambda: inv.project_database("portal", raw), "SCHEMA_COLUMN_DRIFT")
        raw = raw_empty("portal")
        raw["catalog"]["auth_unhandled_security_fact"] = {}
        self.reject(lambda: inv.project_database("portal", raw), "SCHEMA_TABLE_DRIFT")

    def test_duplicate_json_keys_rejected_without_echoing_value(self):
        self.reject(lambda: inv.project_database("portal", '{"catalog":{},"catalog":"private-string","tables":{}}'), "DUPLICATE_JSON_KEY")

    def test_capture_uses_one_statement_per_database_and_sorted_snapshot(self):
        calls = []
        def query(kind, sql):
            calls.append((kind, sql))
            self.assertTrue(sql.startswith("SELECT jsonb_build_object("))
            self.assertIn("LIMIT 20001", sql)
            self.assertNotIn("UPDATE ", sql)
            return json.dumps(raw_empty(kind))
        result = inv.capture(query)
        self.assertEqual([item[0] for item in calls], ["platform", "portal"])
        self.assertEqual(result, inv.empty_inventory())

    def test_row_and_text_limits_reject_without_truncating(self):
        value = inv.empty_inventory()
        value["databases"]["platform"]["users"] = [sample("platform", "users")] * (inv.MAX_ROWS + 1)
        self.reject(lambda: inv.validate_inventory(value), "ROWS_INVALID")
        value = value_with("platform", "users", sample("platform", "users", status="x" * (inv.MAX_TEXT + 1)))
        self.reject(lambda: inv.validate_inventory(value), "TEXT_INVALID")


if __name__ == "__main__":
    unittest.main()
