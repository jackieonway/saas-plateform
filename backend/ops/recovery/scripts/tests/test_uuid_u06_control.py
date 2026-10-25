"""U06 control boundary regressions; no business database or runtime access."""
import copy
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

SPEC = importlib.util.spec_from_file_location("u06_control", Path(__file__).parents[1] / "uuid-u06-control.py")
CONTROL = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CONTROL)


class ReceiptContract(unittest.TestCase):
    def targets(self):
        return [{**target, "oid": 100 + i, "owner_oid": 200 + i,
                 "raw_schema_sha256": "a" * 64, "semantic_schema_sha256": "b" * 64}
                for i, target in enumerate(CONTROL.TARGETS)]

    def test_exact_targets_pass_all_created_stages(self):
        CONTROL.validate_targets("PREPARED", [])
        for stage in ("CREATED", "MIGRATED", "SEALED"):
            CONTROL.validate_targets(stage, self.targets())

    def test_receipt_cannot_skip_reorder_duplicate_or_spoof_targets(self):
        good = self.targets()
        cases = [[], good[:-1], good[::-1], good + good[:1]]
        for field, value in (("oid", True), ("oid", -1), ("oid", "100"),
                             ("oid", 4294967296), ("owner_oid", 201),
                             ("owner", "postgres"), ("kind", "stores"),
                             ("set", "recovery"), ("raw_schema_sha256", "bad")):
            bad = copy.deepcopy(good)
            bad[0][field] = value
            cases.append(bad)
        for bad in cases:
            with self.subTest(targets=bad), self.assertRaises(CONTROL.Invalid):
                CONTROL.validate_targets("MIGRATED", bad)
        with self.assertRaises(CONTROL.Invalid):
            CONTROL.validate_targets("PREPARED", good)


class AdmissionContract(unittest.TestCase):
    def setUp(self):
        self.runtime = [{"name": "runtime_" + str(i), "oid": 20 + i} for i in range(4)]
        self.sources = [{"database": "source_" + str(i), "owner": "owner_" + str(i)} for i in range(3)]

    def proof(self, frozen=False):
        roles = [{"name": "postgres", "oid": 10, "login": True, "superuser": True,
                  "createrole": True, "privileged_set": True, "privileged_set_other": True,
                  "can_admin_source_roles": True, "connect": True}]
        roles += [{**r, "login": not frozen, "superuser": False, "createrole": False,
                   "privileged_set": True, "privileged_set_other": False,
                   "can_admin_source_roles": False, "connect": True} for r in self.runtime]
        return {"maintenance_database": "postgres", "user": "postgres", "session_user": "postgres",
                "local": True, "isolation": "read committed", "transaction_read_only": True,
                "default_read_only": True, "roles": roles}

    def check(self, proof, frozen=False):
        with patch.object(CONTROL, "sql", return_value=[proof]):
            return CONTROL.source_admission(self.sources, self.runtime, frozen)

    def test_only_exact_four_running_roles_or_all_frozen_pass(self):
        self.check(self.proof())
        self.check(self.proof(True), True)

    def test_unknown_login_connect_indirect_set_or_admin_is_rejected(self):
        for field in ("connect", "superuser", "createrole", "privileged_set", "can_admin_source_roles"):
            proof = self.proof()
            unknown = {**proof["roles"][1], "name": "unexpected", "oid": 99,
                       "connect": False, "privileged_set": False}
            unknown[field] = True
            proof["roles"].append(unknown)
            with self.subTest(field=field), self.assertRaises(CONTROL.Invalid):
                self.check(proof)

    def test_running_runtime_cannot_escalate_and_frozen_cannot_login(self):
        for field in ("superuser", "createrole", "privileged_set_other", "can_admin_source_roles"):
            proof = self.proof()
            proof["roles"][1][field] = True
            with self.subTest(field=field), self.assertRaises(CONTROL.Invalid):
                self.check(proof)
        proof = self.proof(True)
        proof["roles"][1]["login"] = True
        with self.assertRaises(CONTROL.Invalid):
            self.check(proof, True)


@unittest.skipUnless(os.name == "posix", "requires Linux process groups")
class MigrationCleanup(unittest.TestCase):
    def test_real_parent_and_child_group_are_killed_and_reaped(self):
        with tempfile.TemporaryDirectory() as raw:
            path = Path(raw) / "child.pid"
            program = ("import os,time; pid=os.fork(); "
                       "open(" + repr(str(path)) + ",'w').write(str(pid)) if pid else None; time.sleep(60)")
            process = subprocess.Popen([sys.executable, "-c", program], start_new_session=True)
            self.addCleanup(lambda: process.poll() is None and CONTROL.close_process_group(process))
            deadline = CONTROL.time.monotonic() + 5
            while not path.exists():
                self.assertLess(CONTROL.time.monotonic(), deadline)
                CONTROL.time.sleep(0.02)
            self.assertGreaterEqual(len(CONTROL.group_members(process.pid)), 2)
            CONTROL.close_process_group(process)
            self.assertEqual(CONTROL.group_members(process.pid), [])

    def test_timeout_reaps_fsyncs_and_checks_backend_zero_before_failure(self):
        with tempfile.TemporaryDirectory() as raw:
            phase = Mock(directory=Path(raw))
            fake = Mock(pid=9999999, returncode=-9)
            fake.wait.side_effect = subprocess.TimeoutExpired("synthetic", 150)
            target = CONTROL.TARGETS[0]
            with patch.object(CONTROL, "sha_file", return_value=CONTROL.MIGRATORS["platform"][1]), \
                    patch.object(CONTROL.subprocess, "Popen", return_value=fake) as popen, \
                    patch.object(CONTROL, "close_process_group") as close, \
                    patch.object(CONTROL, "target_idle") as idle, \
                    patch.object(CONTROL.os, "fsync", wraps=os.fsync) as fsync:
                with self.assertRaisesRegex(CONTROL.Invalid, "MIGRATOR_TIMEOUT"):
                    CONTROL.migrate_target(target, {**target, "oid": 100, "owner_oid": 200}, phase)
                self.assertTrue(popen.call_args.kwargs["start_new_session"])
                close.assert_called_once_with(fake)
                self.assertTrue(idle.call_args.kwargs["terminate"])
                self.assertGreaterEqual(fsync.call_count, 3)
                self.assertEqual(sorted(p.name for p in Path(raw).iterdir()),
                                 [target["name"] + ".stderr", target["name"] + ".stdout"])


if __name__ == "__main__":
    unittest.main()
