"""Offline switch regressions; no Docker, SQL, runtime or socket operations."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch
from types import SimpleNamespace

SPEC = importlib.util.spec_from_file_location("u06_switch", Path(__file__).parents[1] / "uuid-u06-switch.py")
S = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(S)


class LaunchContract(unittest.TestCase):
    def fixture(self, generation="final"):
        root = S.P["BASE"] / generation
        manifest = {"generation": generation,
                    "images": {k: {"name": v[0], "id": v[1]} for k, v in S.P["IMAGES"].items()},
                    "services": {s: {"binary": {"sha256": h}} for s, h in S.C["RUNTIME_BINARIES"].items()}}
        return root, manifest, S.P["launch_plan"](root, manifest)

    def test_both_fixed_generations_reconstruct_seven_commands(self):
        for generation in ("final", "recovery"):
            root, manifest, saved = self.fixture(generation)
            plan = S.checked_plan(root, manifest, saved)
            self.assertEqual(len(plan["commands"]), 7)
            self.assertEqual({c["service"] for c in plan["commands"]}, set(S.SERVICES))
            self.assertTrue(all(c["argv"][:2] == ["docker", "run"] for c in plan["commands"]))
            self.assertEqual([c["service"] for c in plan["commands"] if c["stdin_open"]], ["examples_chat"])

    def test_saved_plan_cannot_add_shell_args_change_image_or_skip_service(self):
        root, manifest, good = self.fixture()
        cases = []
        bad = copy.deepcopy(good)
        bad["commands"][0]["argv"] = ["sh", "-c", "echo should-never-run"]
        cases.append(bad)
        bad = copy.deepcopy(good)
        bad["commands"][0]["argv"].append("--privileged")
        cases.append(bad)
        bad = copy.deepcopy(good)
        bad["commands"][0]["argv"][-4] = "sha256:" + "0" * 64
        cases.append(bad)
        bad = copy.deepcopy(good)
        bad["commands"].pop()
        cases.append(bad)
        bad = copy.deepcopy(good)
        bad["commands"].reverse()
        cases.append(bad)
        for bad in cases:
            with self.subTest(plan=bad), self.assertRaises(S.Invalid):
                S.checked_plan(root, manifest, bad)

    def test_manifest_cannot_change_root_image_or_frozen_binary(self):
        root, manifest, plan = self.fixture()
        with self.assertRaises(S.Invalid):
            S.checked_plan(root.parent / "other", manifest, plan)
        for path in ("image", "binary"):
            bad = copy.deepcopy(manifest)
            if path == "image":
                bad["images"]["runtime"]["id"] = "sha256:" + "0" * 64
            else:
                bad["services"]["platform"]["binary"]["sha256"] = "0" * 64
            with self.subTest(path=path), self.assertRaises(S.Invalid):
                S.checked_plan(root, bad, plan)


class ContainerContract(unittest.TestCase):
    def fixture(self, service="platform", generation="final"):
        root = S.P["BASE"] / generation
        gateway = service == "gateway"
        mounts = [(root / "static", "/srv/frontends", False), (root / "nginx.conf", "/etc/nginx/nginx.conf", False)] if gateway else [
            (root / "bin" / service, "/run/u06/service", False), (root / "env" / (service + ".env"), "/run/u06/service.env", False),
            (root / "runner.py", "/run/u06/runner.py", False), (root / "empty.env", "/run/u06/empty.env", False),
            (root / "state" / service, "/run/u06/state", True)]
        item = {"Id": "a" * 64, "Name": "/" + S.fixed_name(generation, service),
                "Image": S.P["IMAGES"]["gateway" if gateway else "runtime"][1], "RestartCount": 0,
                "Config": {"Labels": {"saas.uuid-stage": "U06", "saas.uuid-generation": generation,
                           "saas.uuid-runtime": "20261024", "saas.uuid-service": service},
                           "OpenStdin": service == "examples_chat", "Tty": False, "User": "101:101" if gateway else "",
                           "Entrypoint": ["/usr/sbin/nginx"] if gateway else ["/usr/bin/python3"],
                           "Cmd": ["-g", "daemon off;"] if gateway else ["-I", "/run/u06/runner.py", S.C["RUNTIME_BINARIES"][service]]},
                "State": {"Running": False, "Paused": False, "Restarting": False, "Pid": 0, "StartedAt": "fixed-start"},
                "HostConfig": {"NetworkMode": "host", "ReadonlyRootfs": True, "Init": True, "CapDrop": ["ALL"],
                               "SecurityOpt": ["no-new-privileges"], "RestartPolicy": {"Name": "no"}, "AutoRemove": False,
                               "LogConfig": {"Type": "none"}, "Tmpfs": {"/tmp": "rw,noexec,nosuid,nodev,size=64m"},
                               "Privileged": False, "PidMode": ""},
                "Mounts": [{"Type": "bind", "Source": str(s), "Destination": d, "RW": rw} for s, d, rw in mounts]}
        return {"root": root}, item

    def check(self, prepared, item, service="platform"):
        with patch.dict(S.C, {"docker": Mock(return_value=json.dumps([item]))}):
            return S.container_record("final", service, prepared, False)

    def test_each_fixed_service_stopped_policy_matches(self):
        for service in S.SERVICES:
            prepared, item = self.fixture(service)
            proof = self.check(prepared, item, service)
            self.assertEqual(proof["id"], item["Id"])
            self.assertEqual(proof["processes"], [])

    def test_policy_drift_unknown_mount_and_other_generation_rejected(self):
        prepared, good = self.fixture()
        cases = []
        for field, value in (("Privileged", True), ("ReadonlyRootfs", False), ("NetworkMode", "bridge"),
                             ("CapAdd", ["SYS_ADMIN"]), ("Binds", ["/:/host"]), ("PidMode", "host"),
                             ("Tmpfs", {"/tmp": "rw,exec"})):
            bad = copy.deepcopy(good)
            bad["HostConfig"][field] = value
            cases.append(bad)
        bad = copy.deepcopy(good)
        bad["Mounts"].append({"Type": "bind", "Source": "/", "Destination": "/host", "RW": True})
        cases.append(bad)
        bad = copy.deepcopy(good)
        bad["Mounts"][0]["Source"] = str(S.P["BASE"] / "recovery/bin/platform")
        cases.append(bad)
        bad = copy.deepcopy(good)
        bad["Config"]["Labels"]["saas.uuid-generation"] = "recovery"
        cases.append(bad)
        bad = copy.deepcopy(good)
        bad["Config"]["Cmd"] = ["-c", "print('arbitrary')"]
        cases.append(bad)
        for bad in cases:
            with self.subTest(item=bad), self.assertRaises(S.Invalid):
                self.check(prepared, bad)


class ExclusiveContract(unittest.TestCase):
    def item(self, name, running, labels=None):
        return {"Id": "b" * 64, "Name": "/" + name, "Config": {"Labels": labels or {}},
                "State": {"Running": running, "Paused": False, "Restarting": False}}

    def test_all_other_generations_must_stop_and_new_names_must_be_absent(self):
        old = self.item("qiankun-uuid-u05-platform-20261024", True)
        with patch.object(S, "all_containers", return_value=[old]), self.assertRaises(S.Invalid):
            S.mutually_exclusive("final", False)
        old["State"]["Running"] = False
        with patch.object(S, "all_containers", return_value=[old]):
            self.assertEqual(len(S.mutually_exclusive("final", False)), 1)
        current = self.item(S.fixed_name("final", "platform"), False)
        with patch.object(S, "all_containers", return_value=[old, current]), self.assertRaises(S.Invalid):
            S.mutually_exclusive("final", False)
        with patch.object(S, "all_containers", return_value=[old, current]):
            S.mutually_exclusive("final", True)

    def test_labelled_other_generation_cannot_hide_behind_unknown_name(self):
        item = self.item("unexpected-name", True, {"saas.uuid-stage": "U06"})
        with patch.object(S, "all_containers", return_value=[item]), self.assertRaises(S.Invalid):
            S.mutually_exclusive("final", True)


class CodeContract(unittest.TestCase):
    def test_mutating_dependency_after_capture_is_rejected(self):
        phase = Mock(script_sha256="a" * 64)
        with patch.dict(S.C, {"sha_file": Mock(return_value="a" * 64)}), self.assertRaises(S.Invalid):
            S.code_guard(phase)


class QuarantineContract(unittest.TestCase):
    def prepared(self):
        return {"roles": {"runtime_platform": 100, "runtime_stores": 101, "runtime_orders": 102, "runtime_notification": 103},
                "target_receipt": {"targets": {s: {"database": "db_" + s, "database_oid": 200 + i, "owner_oid": 300 + i}
                                              for i, s in enumerate(("platform", "stores", "work_orders"))}}}

    def test_fence_and_one_stop_failure_do_not_skip_other_ids_or_any_target(self):
        with tempfile.TemporaryDirectory() as raw:
            phase = Mock(directory=Path(raw))
            prepared, calls = self.prepared(), []
            records = [{"service": s, "id": character * 64} for s, character in (("platform", "a"), ("stores", "b"))]
            def docker(arguments):
                calls.append(arguments)
                identifier = arguments[-1]
                service = "platform" if identifier.startswith("a") else "stores"
                if arguments[0] == "stop":
                    if service == "stores":
                        raise S.Invalid("SYNTHETIC_STOP_FAILURE")
                    return identifier
                return json.dumps([{"Id": identifier, "Name": "/" + S.fixed_name("final", service),
                                    "Image": S.P["IMAGES"]["runtime"][1], "Config": {"Labels": {
                                    "saas.uuid-stage": "U06", "saas.uuid-generation": "final", "saas.uuid-runtime": "20261024", "saas.uuid-service": service}},
                                    "State": {"Running": False, "Paused": False, "Restarting": False, "Pid": 0}}])
            idle = Mock(side_effect=[S.Invalid("SYNTHETIC_FIRST_IDLE_FAILURE"), None, None])
            with patch.dict(S.C, {"cluster": Mock(return_value={"fixed": True}), "docker": docker,
                                  "sql": Mock(side_effect=S.Invalid("SYNTHETIC_FENCE_FAILURE")),
                                  "target_idle": idle, "source_generation": Mock()}), \
                    patch.object(S, "database_proof"), patch.object(S, "mutually_exclusive"), \
                    patch.object(S, "all_containers", return_value=[]):
                status = S.quarantine_generation("final", prepared, {"cluster": {"fixed": True}}, phase, records, [])
            self.assertEqual(status, "FAILED_CLEANUP_INCOMPLETE")
            self.assertEqual(len([c for c in calls if c[0] == "stop"]), 2)
            self.assertEqual(idle.call_count, 3)
            report = json.loads((Path(raw) / "quarantine.json").read_text())
            self.assertEqual(len([r for r in report["steps"] if r["status"] == "FAILED"]), 3)

    def test_cleanup_role_guard_is_idempotent_for_already_nologin(self):
        sql = Mock()
        with patch.dict(S.C, {"sql": sql}):
            S.quarantine_roles(self.prepared(), Mock())
        statement = sql.call_args.args[0]
        self.assertNotIn("rolcanlogin", statement)
        self.assertEqual(statement.count(" NOLOGIN;"), 4)
        self.assertEqual(statement.count("AND NOT (rolsuper"), 4)

    def test_timeout_unknown_id_with_invalid_full_policy_is_never_stopped(self):
        with tempfile.TemporaryDirectory() as raw:
            phase, docker = Mock(directory=Path(raw)), Mock()
            unknown = {"Id": "f" * 64, "Name": "/" + S.fixed_name("final", "platform"),
                       "State": {"Running": False, "Paused": False, "Restarting": False}}
            with patch.dict(S.C, {"cluster": Mock(return_value={"fixed": True}), "sql": Mock(), "docker": docker,
                                  "target_idle": Mock(), "source_generation": Mock()}), \
                    patch.object(S, "database_proof"), patch.object(S, "mutually_exclusive"), \
                    patch.object(S, "all_containers", return_value=[unknown]), \
                    patch.object(S, "container_record", side_effect=S.Invalid("INVALID_STATIC_POLICY")) as check:
                status = S.quarantine_generation("final", self.prepared(), {"cluster": {"fixed": True}}, phase, [], ["platform"])
            self.assertEqual(status, "FAILED_CLEANUP_INCOMPLETE")
            docker.assert_not_called()
            self.assertIsNone(check.call_args.kwargs["running"])
            self.assertFalse(check.call_args.kwargs["verify_process"])

    def test_freeze_first_stop_failure_always_invokes_full_quarantine(self):
        with tempfile.TemporaryDirectory() as raw:
            base = Path(raw) / "switch"
            records = [{"service": s, "id": str(i) * 64} for i, s in enumerate(S.SERVICES, 1)]
            prepared = {**self.prepared(), "commitment": {"fixed": True}}
            proof = {"roles": [], "targets": []}
            sealed = {"stage": "SEALED", "cluster": {"fixed": True}}
            previous = {"format": S.FORMAT, "status": "CAPTURED_RUNNING", "script_sha256": S.C["sha_file"](S.SCRIPT),
                        "helpers": S.HELPERS, "generation": "final", "sealed_receipt": "/fixed/sealed.json", "sealed_receipt_sha256": "a" * 64,
                        "material": prepared["commitment"], "containers": records, "database_proof": proof}
            args = SimpleNamespace(check_code=False, check=False, start=False, capture=False, freeze=True,
                                   receipt=Path("/fixed/capture.json"), generation=None, sealed_receipt=None, output=base / "freeze-test")
            lock = Mock()
            lock.process.stdout.readline.return_value = "t"
            with patch.object(S, "BASE", base), patch.object(S, "private_json", return_value=previous), \
                    patch.object(S, "material", return_value=prepared), patch.object(S, "database_proof", return_value=proof), \
                    patch.object(S, "mutually_exclusive", return_value=[]), \
                    patch.object(S, "container_record", side_effect=lambda g, s, p: next(r for r in records if r["service"] == s)), \
                    patch.object(S, "quarantine_generation", return_value="QUARANTINED_NOLOGIN_IDLE") as cleanup, \
                    patch.dict(S.C, {"read_receipt": Mock(return_value=(sealed, "a" * 64)), "Advisory": Mock(return_value=lock),
                                     "source_generation": Mock(), "cluster": Mock(return_value=sealed["cluster"]),
                                     "docker": Mock(side_effect=S.Invalid("ORIGINAL_STOP_FAILURE"))}):
                with self.assertRaisesRegex(S.Invalid, "ORIGINAL_STOP_FAILURE"):
                    S.run(args)
            cleanup.assert_called_once()
            self.assertEqual(cleanup.call_args.args[4], records)
            self.assertEqual(cleanup.call_args.args[5], [])
            failure = json.loads((args.output / "failure.json").read_text())
            self.assertEqual(failure["cleanup"], "QUARANTINED_NOLOGIN_IDLE")
            self.assertFalse((args.output / "receipt.json").exists())


if __name__ == "__main__":
    unittest.main()
