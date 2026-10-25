"""Pure and fake-runner fixtures; never connect PostgreSQL or Docker."""
import contextlib
import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / "recreate-uuid-preview-targets.py"
SPEC = importlib.util.spec_from_file_location("recreate_uuid_preview_targets", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
M = MODULE


def proof(sealed=True, new_generation=False):
    owner_names = [owner for _, _, owner in M.TARGETS]
    role_names = ["postgres", *M.SOURCE_ROLES, *owner_names]
    ids = {name: index + 10 for index, name in enumerate(role_names)}
    databases = []
    for index, name in enumerate(M.SOURCES):
        databases.append({"name": name, "oid": 1000 + index, "owner": M.SOURCE_ROLES[0],
                          "owner_oid": ids[M.SOURCE_ROLES[0]], "encoding": "UTF8", "collate": "C.UTF-8",
                          "ctype": "C.UTF-8", "allow_connections": True, "is_template": False,
                          "readonly": True, "public_connect": True, "backends": 0, "prepared": 0, "comment": None})
    for index, (_, name, owner) in enumerate(M.TARGETS):
        databases.append({"name": name, "oid": (3000 if new_generation else 2000) + index, "owner": owner,
                          "owner_oid": ids[owner], "encoding": "UTF8", "collate": "C.UTF-8", "ctype": "C.UTF-8",
                          "allow_connections": True, "is_template": False, "readonly": sealed,
                          "public_connect": False, "backends": 0, "prepared": 0, "comment": None})
    databases.sort(key=lambda item: item["name"])
    roles = []
    for name in sorted(role_names):
        admin = name == "postgres"
        roles.append({"name": name, "oid": ids[name], "login": admin, "superuser": admin,
                      "create_db": admin, "create_role": admin, "replication": admin, "bypass_rls": admin,
                      "privileged_set": True, "can_admin_owners": False,
                      "connect": [{"name": db["name"], "oid": db["oid"],
                                   "allowed": admin or db["name"] in M.SOURCES or name == db["owner"]}
                                  for db in databases]})
    return {"actor": "postgres", "session_actor": "postgres", "actor_oid": ids["postgres"], "maintenance_oid": 5,
            "maintenance_database": "postgres", "system_identifier": "1234567890123456789", "local_socket": True,
            "port": 5432, "server_version_num": 180006, "readonly_observer": True, "isolation": "read committed",
            "roles": roles, "databases": databases}


def writers():
    items = []
    for index, name in enumerate(M.WRITERS):
        identifier = f"{index + 1:064x}"
        if name == M.AUTO_REMOVED:
            items.append({"name": name, "present": False, "id": None, "state": None})
        else:
            items.append({"name": name, "present": True, "id": identifier,
                          "state": {"id": identifier, "name": "/" + name, "running": False,
                                    "paused": False, "restarting": False, "pid": 0, "restart_policy": "no"}})
    return items


def imported_marker(payload="a"):
    return "saas-uuid-import/1/019a8c20-0000-7000-8000-000000000001/" + payload * 64


def receipt(original=None):
    return M.receipt_document(original or proof(), writers(), True, "b" * 64)


class PureFixtures(unittest.TestCase):
    def test_complete_fixed_group_and_reviewed_autoremove_are_valid(self):
        self.assertEqual(len(M.validate_proof(proof())), 6)
        M.validate_writers(writers())
        M.validate_receipt(receipt(), proof(), writers(), "b" * 64)

    def test_unsealed_active_template_remote_and_source_privilege_drift_refuse(self):
        for field, invalid in [("actor", "application"), ("session_actor", "application"), ("port", 5433),
                               ("local_socket", False), ("readonly_observer", False), ("isolation", "repeatable read")]:
            changed = proof()
            changed[field] = invalid
            with self.subTest(field=field), self.assertRaises(M.RecreateError):
                M.validate_proof(changed)
        target_name = M.TARGETS[0][1]
        for field, invalid in [("readonly", False), ("backends", 1), ("prepared", 1), ("public_connect", True),
                               ("owner", "postgres"), ("is_template", True), ("allow_connections", False),
                               ("comment", "ACTIVE_PRODUCTION"), ("oid", True)]:
            changed = proof()
            target = next(item for item in changed["databases"] if item["name"] == target_name)
            target[field] = invalid
            with self.subTest(field=field), self.assertRaises(M.RecreateError):
                M.validate_proof(changed)
        for field in ("login", "superuser"):
            changed = proof()
            next(item for item in changed["roles"] if item["name"] == M.SOURCE_ROLES[0])[field] = True
            with self.subTest(field=field), self.assertRaises(M.RecreateError):
                M.validate_proof(changed)
        changed = proof()
        next(item for item in changed["databases"] if item["name"] == M.SOURCES[0])["readonly"] = False
        with self.assertRaises(M.RecreateError):
            M.validate_proof(changed)

    def test_login_connect_set_admin_and_createrole_bypasses_refuse(self):
        for dangerous in ("connect", "superuser", "create_role", "privileged_set", "can_admin_owners"):
            changed = proof()
            role = copy.deepcopy(changed["roles"][0])
            role.update(name="new_service_login", oid=900, login=True, superuser=False, create_db=False,
                        create_role=False, replication=False, bypass_rls=False, privileged_set=False, can_admin_owners=False)
            for connect in role["connect"]:
                connect["allowed"] = False
            if dangerous == "connect":
                role["connect"][0]["allowed"] = True
            else:
                role[dangerous] = True
            changed["roles"].append(role)
            with self.subTest(dangerous=dangerous), self.assertRaises(M.RecreateError):
                M.validate_proof(changed)

    def test_receipt_checks_exact_oids_tool_policy_assertions_and_marker_transition(self):
        original = receipt()
        current = proof()
        target = next(item for item in current["databases"] if item["name"] == M.TARGETS[0][1])
        target["comment"] = imported_marker()
        M.validate_receipt(original, current, writers(), "b" * 64)
        current["system_identifier"] = "999"
        with self.assertRaises(M.RecreateError):
            M.validate_receipt(original, current, writers(), "b" * 64)
        for field, invalid in [("status", M.UNMIGRATED), ("policy", "generic-recovery"), ("tool_sha256", "c" * 64)]:
            changed = receipt()
            changed[field] = invalid
            with self.subTest(field=field), self.assertRaises(M.RecreateError):
                M.validate_receipt(changed, proof(), writers(), "b" * 64)
        changed = receipt()
        changed["trusted_maintenance"]["never_opened_asserted"] = False
        with self.assertRaises(M.RecreateError):
            M.validate_receipt(changed, proof(), writers(), "b" * 64)
        changed = proof()
        old_oid = changed["databases"][0]["oid"]
        changed["databases"][0]["oid"] += 10000
        for role in changed["roles"]:
            for connect in role["connect"]:
                if connect["oid"] == old_oid:
                    connect["oid"] += 10000
        M.validate_proof(changed)
        with self.assertRaises(M.RecreateError):
            M.validate_receipt(original, changed, writers(), "b" * 64)

    def test_only_canonical_v7_import_markers_and_unchanged_recorded_marker_pass(self):
        self.assertTrue(M.marker(None))
        self.assertTrue(M.marker(imported_marker()))
        for invalid in ["", "saas-uuid-import/1/019a8c20-0000-4000-8000-000000000001/" + "a" * 64,
                        imported_marker() + "'", "ACTIVE", imported_marker().upper()]:
            self.assertFalse(M.marker(invalid))
        initial = proof()
        next(item for item in initial["databases"] if item["name"] == M.TARGETS[0][1])["comment"] = imported_marker()
        original = receipt(initial)
        changed = copy.deepcopy(initial)
        next(item for item in changed["databases"] if item["name"] == M.TARGETS[0][1])["comment"] = imported_marker("c")
        with self.assertRaises(M.RecreateError):
            M.validate_receipt(original, changed, writers(), "b" * 64)

    def test_stopped_writers_cannot_restart_disappear_or_change_generation(self):
        for field, invalid in [("running", True), ("paused", True), ("restarting", True), ("pid", 99), ("restart_policy", "always")]:
            changed = writers()
            changed[0]["state"][field] = invalid
            with self.subTest(field=field), self.assertRaises(M.RecreateError):
                M.validate_writers(changed)
        changed = writers()
        changed[0] = {"name": M.WRITERS[0], "present": False, "id": None, "state": None}
        with self.assertRaises(M.RecreateError):
            M.validate_writers(changed)
        changed = writers()
        changed[0]["id"] = changed[0]["state"]["id"] = "a" * 64
        M.validate_writers(changed)
        with self.assertRaises(M.RecreateError):
            M.validate_receipt(receipt(), proof(), changed, "b" * 64)

    def test_recreation_sql_has_only_three_fixed_drops_and_a_post_lock_identity_guard(self):
        sql = M.recreation_sql(proof())
        mutations = [line for line in sql.splitlines() if line.startswith(("DROP ", "CREATE ", "ALTER ", "REVOKE "))]
        drops = [line for line in mutations if line.startswith("DROP ")]
        self.assertEqual(drops, [f'DROP DATABASE "{name}";' for _, name, _ in M.TARGETS])
        self.assertNotIn("FORCE", sql)
        self.assertNotIn("DROP ROLE", sql)
        for statement in mutations:
            self.assertFalse(any(source in statement for source in M.SOURCES))
            self.assertFalse(any(role in statement for role in M.SOURCE_ROLES))
        self.assertLess(sql.index("pg_advisory_lock"), sql.index("AS exact_group"))
        self.assertLess(sql.index("\\if :exact_group"), sql.index("DROP DATABASE"))
        self.assertIn("\\gset", sql)
        self.assertNotIn("\\\\gset", sql)
        self.assertIn("TEMPLATE template0", sql)
        self.assertIn("pg_execute_server_program", sql)
        self.assertIn("pg_write_server_files", sql)

    def test_recreated_group_retains_sources_and_roles_but_requires_new_empty_generation(self):
        before = proof()
        after = proof(sealed=False, new_generation=True)
        M.validate_recreated(before, after)
        for field, invalid in [("readonly", True), ("comment", imported_marker()), ("owner_oid", 999)]:
            changed = copy.deepcopy(after)
            next(item for item in changed["databases"] if item["name"] == M.TARGETS[0][1])[field] = invalid
            with self.subTest(field=field), self.assertRaises(M.RecreateError):
                M.validate_recreated(before, changed)
        changed = copy.deepcopy(after)
        next(item for item in changed["databases"] if item["name"] == M.SOURCES[0])["comment"] = "changed"
        with self.assertRaises(M.RecreateError):
            M.validate_recreated(before, changed)

    def test_json_duplicate_keys_and_extra_receipt_fields_refuse(self):
        with self.assertRaises(M.RecreateError):
            M.decode('{"format":1,"format":2}')
        changed = receipt()
        changed["unreviewed"] = True
        with self.assertRaises(M.RecreateError):
            M.validate_receipt(changed, proof(), writers(), "b" * 64)


@unittest.skipUnless(sys.platform == "linux", "private Linux receipt paths")
class FilesystemFixtures(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="uuid-u04-recreate-")
        self.path = Path(self.directory.name)
        self.path.chmod(0o700)

    def tearDown(self):
        self.directory.cleanup()

    def test_receipt_io_is_private_new_canonical_and_symlink_free(self):
        output = self.path / "record.json"
        descriptor = M.reserve_output(output)
        M.finish_output(output, descriptor, receipt())
        restored, checksum = M.read_receipt(output)
        self.assertEqual(restored, json.loads(output.read_bytes()))
        self.assertEqual(checksum, hashlib.sha256(output.read_bytes()).hexdigest())
        self.assertEqual(output.stat().st_mode & 0o7777, 0o600)
        with patch.object(M.os, "geteuid", return_value=os.geteuid() + 1), self.assertRaises(M.RecreateError):
            M.read_receipt(output)
        output.chmod(0o4600)
        with self.assertRaises(M.RecreateError):
            M.read_receipt(output)
        output.chmod(0o600)
        with self.assertRaises(M.RecreateError):
            M.reserve_output(output)
        (self.path / "link").symlink_to(output)
        with self.assertRaises(M.RecreateError):
            M.canonical_path(str(self.path / "link"))
        for invalid in [str(self.path) + "/./record.json", str(self.path) + "//record.json", str(self.path) + "/../record.json"]:
            with self.assertRaises(M.RecreateError):
                M.canonical_path(invalid)
        self.path.chmod(0o755)
        with self.assertRaises(M.RecreateError):
            M.reserve_output(self.path / "wrong-owner-mode.json")
        self.path.chmod(0o700)

    def test_fake_runner_record_and_execute_never_touch_real_databases(self):
        class FakeRunner:
            def __init__(self):
                self.mutations = []
            def observe(self):
                return proof(sealed=not self.mutations, new_generation=bool(self.mutations))
            def writers(self):
                return writers()
            def sql(self, statement, readonly=True):
                self.mutations.append((statement, readonly))
                return ""
        old = self.path / "old.json"
        new = self.path / "new.json"
        runner = FakeRunner()
        with contextlib.redirect_stdout(io.StringIO()):
            M.main(["--output", str(old), "--assert-u04-never-opened"], runner=runner)
        self.assertEqual(runner.mutations, [])
        old_bytes = old.read_bytes()
        with contextlib.redirect_stdout(io.StringIO()):
            M.main(["--execute", "--receipt", str(old), "--output", str(new), "--assert-u04-never-opened"], runner=runner)
        self.assertEqual(len(runner.mutations), 1)
        self.assertFalse(runner.mutations[0][1])
        self.assertEqual(old.read_bytes(), old_bytes)
        result = json.loads(new.read_bytes())
        self.assertEqual(result["status"], M.UNMIGRATED)
        self.assertEqual(result["parent_receipt_sha256"], hashlib.sha256(old_bytes).hexdigest())
        with self.assertRaises(M.RecreateError):
            M.validate_receipt(result, proof(), writers(), hashlib.sha256(SCRIPT.read_bytes()).hexdigest())

    def test_bad_recorded_oid_or_missing_assertion_refuses_before_any_mutation(self):
        class FakeRunner:
            mutations = []
            def observe(self):
                return proof()
            def writers(self):
                return writers()
            def sql(self, *args, **kwargs):
                self.mutations.append(args)
                raise AssertionError("must not execute")
        old = self.path / "old.json"
        baseline = receipt()
        baseline["tool_sha256"] = hashlib.sha256(SCRIPT.read_bytes()).hexdigest()
        baseline["source_and_target_proof"]["system_identifier"] = "999"
        M.finish_output(old, M.reserve_output(old), baseline)
        runner = FakeRunner()
        with self.assertRaises(M.RecreateError):
            M.main(["--execute", "--receipt", str(old), "--output", str(self.path / "new.json"),
                    "--assert-u04-never-opened"], runner=runner)
        self.assertEqual(runner.mutations, [])
        self.assertFalse((self.path / "new.json").exists())
        with self.assertRaises(M.RecreateError):
            M.main(["--execute", "--receipt", str(old), "--output", str(self.path / "new2.json")], runner=runner)
        self.assertEqual(runner.mutations, [])

    def test_failed_ddl_keeps_original_receipt_and_records_quarantined_failure(self):
        class FakeRunner:
            def observe(self):
                return proof()
            def writers(self):
                return writers()
            def sql(self, *_args, **_kwargs):
                raise M.RecreateError("fixture DDL failure")
        old = self.path / "old.json"
        failed = self.path / "failed.json"
        original = receipt()
        original["tool_sha256"] = hashlib.sha256(SCRIPT.read_bytes()).hexdigest()
        M.finish_output(old, M.reserve_output(old), original)
        old_bytes = old.read_bytes()
        with self.assertRaises(M.RecreateError):
            M.main(["--execute", "--receipt", str(old), "--output", str(failed),
                    "--assert-u04-never-opened"], runner=FakeRunner())
        self.assertEqual(old.read_bytes(), old_bytes)
        result = json.loads(failed.read_bytes())
        self.assertEqual(result["status"], "FAILED_GROUP_REMAINS_QUARANTINED")
        self.assertEqual(result["parent_receipt_sha256"], hashlib.sha256(old_bytes).hexdigest())
        self.assertEqual(failed.stat().st_mode & 0o7777, 0o600)
        with self.assertRaises(M.RecreateError):
            M.validate_receipt(result, proof(), writers(), hashlib.sha256(SCRIPT.read_bytes()).hexdigest())

    def test_unasserted_observation_cannot_become_destructive_receipt(self):
        class FakeRunner:
            def observe(self):
                return proof()
            def writers(self):
                return writers()
            def sql(self, *_args, **_kwargs):
                raise AssertionError("recording must not call DDL")
        observed = self.path / "observation.json"
        with contextlib.redirect_stdout(io.StringIO()):
            M.main(["--output", str(observed)], runner=FakeRunner())
        result = json.loads(observed.read_bytes())
        self.assertEqual(result["status"], "OBSERVED_ONLY_NOT_DESTRUCTIVE_INPUT")
        with self.assertRaises(M.RecreateError):
            M.validate_receipt(result, proof(), writers(), hashlib.sha256(SCRIPT.read_bytes()).hexdigest())


if __name__ == "__main__":
    unittest.main()
