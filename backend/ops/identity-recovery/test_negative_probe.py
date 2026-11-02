"""File-backed negatives with real CLI/crypto; PostgreSQL preflight is mocked.

Only uniquely owned TemporaryDirectory files are created, then removed. No DB,
HBA, network proxy, application, Docker, dump or restore command is executed.
"""
import base64
import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import Mock, patch

import core
import inventory
import negative_probe as probe
import witness


TASK = "019aa31b-6112-7000-8000-000000000001"
SOURCE = "019aa31b611270008000000000000003"


@unittest.skipUnless(os.name == "posix", "anchored POSIX fixture required")
class ProbeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="rust-saas-unified-auth-negative-unit-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.root.chmod(0o700)
        core.private_write(self.root / "OWNER", (self.root.name + "-pg55494\n").encode())
        self.config_path = self.root / "config.json"
        value = {"format": core.FORMAT, "taskId": TASK, "clusterId": "7701548153213487841",
                 "issuer": "https://auth.saas.test", "taskRoot": str(self.root),
                 "workDirectory": str(self.root / "work"), "witnessDirectory": str(self.root / "witness"),
                 "floorFile": str(self.root / "floor.json"), "keyFile": str(self.root / "keys.json"),
                 "adminConnectionFile": str(self.root / "admin.json"), "databases": {},
                 "privateMaterials": {"runtime": str(self.root / "runtime.json")},
                 "pgBin": "/usr/lib/postgresql/18/bin"}
        for kind, prefix in (("platform", "https"), ("portal", "portal")):
            value["databases"][kind] = {"source": "unified_auth_" + prefix + "_" + SOURCE,
                "target": ("unified_auth_restore_" if kind == "platform" else "unified_auth_restore_portal_") + SOURCE,
                "owner": "uah_unit_" + ("migration_owner" if kind == "platform" else "portal_owner"),
                "sourceMarker": "identity-deployment/1/" + TASK,
                "targetMarker": "phase23-recovery/2/" + TASK,
                "connectionFile": str(self.root / "admin.json")}
        core.private_write(self.config_path, value)
        core.private_write(self.root / "admin.json", {"PGHOST": "127.0.0.1", "PGPORT": "55494",
                                                     "PGUSER": "postgres", "PGDATABASE": "postgres"})
        core.private_write(self.root / "runtime.json", b"owned-test-material-no-real-secret")
        self.config = core.load_config(self.config_path)
        core.initialize(self.config)
        self.material = core.keys(self.config)
        archives = {}
        for kind in ("platform", "portal"):
            archives[kind] = core.encrypt(self.config, self.material, kind,
                b"PGDMP-owned-file-fixture-not-a-real-dump", core.binding(self.config))
        archives["materials"] = core.encrypt(self.config, self.material, "materials",
            core.canonical({"runtime": base64.b64encode(b"fixture").decode()}), core.binding(self.config))
        archives["checkpoint-facts"] = core.encrypt(self.config, self.material, "checkpoint-facts",
            core.canonical({"platform": {}, "portal": {}}), core.binding(self.config))
        self.saved = {"format": core.FORMAT, "binding": core.binding(self.config), "createdAtMs": 100,
                      "recoveryPointMs": 99, "databases": {}, "catalogs": {}, "roles": {},
                      "inventory": inventory.empty_inventory(), "archives": archives}
        core.authenticated_write(self.config["workDirectory"] / "checkpoint.json", self.saved,
                                 self.material["mac"], "checkpoint")
        self.payload = witness.make_payload(inventory.empty_inventory(), TASK, 100, {})
        self.store = core.witness_store(self.config, self.material, self.saved)
        self.store.append(self.payload, expected_sequence=0)
        core.save_floor(self.config, self.material, self.saved, self.store.load_latest(1))
        self.preflight = Mock(return_value={"sourceConnectionCount": 0, "targetDatabaseCount": 0})

    def later(self, save=True):
        if save:
            result = probe.save_baseline(self.config_path, 1)
            self.assertEqual(result["status"], "BASELINE_SAVED")
        self.payload = witness.make_payload(inventory.empty_inventory(), TASK, 101, {}, self.payload)
        self.store.append(self.payload, expected_sequence=1)
        facts = core.encrypt(self.config, self.material, "deny-facts-00000002",
            core.canonical({"platform": {}, "portal": {}}), self.saved["binding"])
        core.save_floor(self.config, self.material, self.saved, self.store.load_latest(2), replace=True, facts=facts)

    def test_nine_real_file_and_cli_loaders_reject_and_restore_exact_bytes(self):
        self.later()
        before = {str(path.relative_to(self.root)): path.read_bytes()
                  for path in self.root.rglob("*") if path.is_file()}
        # Real crypto and real recovery.py status subprocesses; only PG is mocked.
        result = probe.run_probe(self.config_path, 2, preflight=self.preflight)
        self.assertEqual(result["status"], "PASS")
        self.assertEqual(result["validationLayer"], probe.LAYER)
        self.assertFalse(result["restoreSqlExecuted"])
        self.assertFalse(result["databaseWrites"])
        self.assertEqual([case["case"] for case in result["cases"]], list(probe.CASE_NAMES))
        self.assertTrue(all(case["originalBytesRestored"] for case in result["cases"]))
        self.assertEqual(self.preflight.call_count, 18)
        after = {str(path.relative_to(self.root)): path.read_bytes()
                 for path in self.root.rglob("*") if path.is_file()}
        self.assertEqual(before, after)

    def test_baseline_requires_true_sequence_one_and_probe_independent_floor(self):
        with self.assertRaisesRegex(core.RecoveryError, "BASELINE_SEQUENCE_REQUIRED"):
            probe.save_baseline(self.config_path, 2)
        self.later(save=False)
        with self.assertRaisesRegex(core.RecoveryError, "BASELINE_MUST_BE_FIRST_SEQUENCE"):
            probe.save_baseline(self.config_path, 1)
        with self.assertRaisesRegex(core.RecoveryError, "LATER_INDEPENDENT_SEQUENCE_REQUIRED"):
            probe.run_probe(self.config_path, 1, preflight=self.preflight)
        self.preflight.assert_not_called()

    def test_tampered_baseline_rejects_before_mutation(self):
        self.later()
        path = probe.baseline_directory(self.config) / "current.bin"
        original = path.read_bytes()
        core.private_write(path, bytes([original[0] ^ 1]) + original[1:], replace=True)
        with self.assertRaisesRegex(core.RecoveryError, "BASELINE_COPY_CHANGED"):
            probe.run_probe(self.config_path, 2, preflight=self.preflight)
        self.preflight.assert_not_called()
        self.assertEqual(self.store.load_latest(2)["document"]["sequence"], 2)

    def test_finally_restores_on_loader_exception_and_leaves_no_journal(self):
        self.later()
        key = self.config["keyFile"]
        original = key.read_bytes()
        with self.assertRaisesRegex(RuntimeError, "test-only"):
            with probe.mutation(self.config, "test-finally", key, b"changed"):
                raise RuntimeError("test-only")
        self.assertEqual(key.read_bytes(), original)
        self.assertFalse((probe.baseline_directory(self.config) / "probe-journal-test-finally.json").exists())

    def test_missing_file_and_extra_file_mutations_restore_exact_original_set(self):
        self.later()
        path = self.config["witnessDirectory"] / "witness-00000001.json"
        original = path.read_bytes()
        with probe.mutation(self.config, "test-missing", path, None):
            self.assertFalse(path.exists())
        self.assertEqual(path.read_bytes(), original)
        extra = self.config["witnessDirectory"] / "witness-00000003.json"
        with probe.mutation(self.config, "test-extra", extra, b"extra"):
            self.assertEqual(extra.read_bytes(), b"extra")
        self.assertFalse(extra.exists())

    def test_concurrent_change_is_preserved_with_private_before_image(self):
        self.later()
        path = self.config["keyFile"]
        before = path.read_bytes()
        with self.assertRaisesRegex(core.RecoveryError, "CONCURRENT_FILE_CHANGE"):
            with probe.mutation(self.config, "test-conflict", path, b"probe-change"):
                core.private_write(path, b"another-writer", replace=True)
        self.assertEqual(path.read_bytes(), b"another-writer")
        journal = probe.baseline_directory(self.config) / "probe-journal-test-conflict.json"
        raw = core.decode(core.private_read(journal))
        self.assertEqual(base64.b64decode(raw["before"]), before)
        self.assertEqual(journal.stat().st_mode & 0o777, 0o600)

    def test_change_exact_never_overwrites_unexpected_existing_bytes(self):
        path = self.config["keyFile"]
        original = path.read_bytes()
        with self.assertRaisesRegex(core.RecoveryError, "CONCURRENT_FILE_CHANGE"):
            probe.change_exact(path, b"wrong-before", b"must-not-overwrite")
        self.assertEqual(path.read_bytes(), original)

    def test_readonly_pg_preflight_checks_zero_connections_and_absent_targets(self):
        with patch.object(core, "guard") as guard, patch.object(core, "Pg") as pg:
            pg.return_value.json.return_value = {"sourceConnectionCount": 0, "targetDatabaseCount": 0}
            self.assertEqual(probe.readonly_preflight(self.config)["targetDatabaseCount"], 0)
            statement = pg.return_value.json.call_args.args[0]
            self.assertTrue(statement.startswith("SELECT "))
            self.assertNotIn("CREATE DATABASE", statement)
            guard.assert_called_once_with(self.config)
            for result in ({"sourceConnectionCount": 1, "targetDatabaseCount": 0},
                           {"sourceConnectionCount": 0, "targetDatabaseCount": 1}):
                pg.return_value.json.return_value = result
                with self.assertRaisesRegex(core.RecoveryError, "QUIET_SOURCES_AND_ABSENT_TARGETS_REQUIRED"):
                    probe.readonly_preflight(self.config)

    def test_unstructured_tool_failure_is_not_a_successful_negative(self):
        result = subprocess.CompletedProcess([], 2, stdout=b"", stderr=b"connection failed")
        with patch.object(probe.subprocess, "run", return_value=result):
            with self.assertRaises(core.RecoveryError):
                probe.status_rejection(self.config_path, 2)
        result = subprocess.CompletedProcess([], 0, stdout=b'{"status":"VERIFIED"}', stderr=b"")
        with patch.object(probe.subprocess, "run", return_value=result):
            with self.assertRaisesRegex(core.RecoveryError, "EXPECTED_CLI_REJECTION_MISSING"):
                probe.status_rejection(self.config_path, 2)

    def test_preexisting_corruption_does_not_count_as_a_successful_probe(self):
        self.later()
        path = self.config["workDirectory"] / self.saved["archives"]["platform"]["filename"]
        original = path.read_bytes()
        corrupted = bytes([original[0] ^ 1]) + original[1:]
        core.private_write(path, corrupted, replace=True)
        with self.assertRaisesRegex(core.RecoveryError, "ARCHIVE_DIGEST_MISMATCH"):
            probe.run_probe(self.config_path, 2, preflight=self.preflight)
        self.preflight.assert_not_called()
        self.assertEqual(path.read_bytes(), corrupted)
        self.assertFalse(list(probe.baseline_directory(self.config).glob("probe-journal-*")))

    def test_tool_timeout_restores_bytes_checks_pg_again_and_reports_completed_cases(self):
        self.later()
        path = self.config["workDirectory"] / "checkpoint.json"
        original = path.read_bytes()
        with patch.object(probe, "status_rejection", side_effect=subprocess.TimeoutExpired("fixture", 30)):
            with self.assertRaises(probe.ProbeFailure) as caught:
                probe.run_probe(self.config_path, 2, preflight=self.preflight)
        result = caught.exception.cases
        self.assertEqual([(row["case"], row["status"]) for row in result],
                         [("ciphertext-tamper", "PASS"), ("manifest-tamper", "FAIL")])
        self.assertTrue(result[-1]["originalBytesRestored"])
        self.assertEqual(result[-1]["preflightAfter"]["targetDatabaseCount"], 0)
        self.assertEqual(self.preflight.call_count, 4)
        self.assertEqual(path.read_bytes(), original)
        self.assertFalse(list(probe.baseline_directory(self.config).glob("probe-journal-*")))

    def test_cli_failure_never_prints_raw_exception_or_secret(self):
        output = io.StringIO()
        with patch("sys.argv", ["negative_probe.py", "--stage", "probe", "--config", "/private/cfg",
                                "--minimum-sequence", "2"]), \
             patch.object(probe, "run_probe", side_effect=RuntimeError("PHC-password-cookie-do-not-print")), \
             contextlib.redirect_stderr(output):
            self.assertEqual(probe.main(), 2)
        self.assertNotIn("PHC-password", output.getvalue())
        self.assertEqual(json.loads(output.getvalue())["code"], "RECOVERY_REJECTED")


if __name__ == "__main__":
    unittest.main()
