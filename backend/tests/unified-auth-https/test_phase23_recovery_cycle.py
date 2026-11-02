"""Pure orchestrator checks. Every PG/child invocation is mocked."""
import copy
import importlib.util
import signal
from pathlib import Path
import subprocess
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

_path = Path(__file__).with_name("phase23-recovery-cycle.py")
_spec = importlib.util.spec_from_file_location("phase23_recovery_cycle", _path)
cycle = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(cycle)
core = cycle.core


class CycleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="phase23-cycle-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.root.chmod(0o700)
        self.args = types.SimpleNamespace(**{name: self.root / name for name in
            ("config", "environment", "selection", "output_env", "output_rearm", "output")}, minimum_sequence=2)
        for name in ("config", "environment", "selection"):
            core.private_write(getattr(self.args, name), b"private-input")
        self.config = {"taskRoot": str(self.root), "databases": {
            "platform": {"owner": "uah_owned_migration_owner", "source": "owned_platform"},
            "portal": {"owner": "uah_owned_portal_owner", "source": "owned_portal"}}}

    def execute(self, handler):
        with patch.object(core, "load_config", return_value=self.config), \
             patch.object(core, "keys", return_value={}), \
             patch.object(core, "checkpoint", return_value={"saved": True}), \
             patch.object(core, "latest_witness", return_value=({}, {"sequence": 2})), \
             patch.object(cycle, "source_fenced"), patch.object(cycle, "invoke", side_effect=handler):
            return cycle.run_cycle(self.args)

    def good_child(self, argv):
        step = "adapter-rearm" if Path(argv[2]).name == "phase23-recovery-files.py" else argv[3]
        if step == "adapter-rearm":
            core.private_write(self.args.output_env, b"new-private-environment")
            core.private_write(self.args.output_rearm, b"new-private-rearm")
        return {"status": cycle.EXPECTED[step], "secret": "must-never-export", "opensTraffic": False}

    def test_exact_sequence_only_private_paths_and_safe_receipt(self):
        calls = []
        def child(argv):
            calls.append(argv)
            return self.good_child(argv)
        result = self.execute(child)
        self.assertEqual([row["step"] for row in result["stages"]], list(cycle.EXPECTED))
        self.assertEqual(result["status"], "GATE_PASSED_QUARANTINED")
        self.assertFalse(result["completeRtoClaim"])
        self.assertFalse(result["opensTraffic"])
        self.assertNotIn("must-never-export", core.private_read(self.args.output).decode())
        self.assertTrue(all("-B" in argv and "--config" in argv and "--minimum-sequence" in argv for argv in calls))
        self.assertTrue(all(core.private_read(getattr(self.args, name)) == b"private-input"
                            for name in ("config", "environment", "selection")))

    def test_child_failure_records_quarantine_without_raw_exception(self):
        def child(_):
            raise RuntimeError("postgres://raw-password-and-private-response")
        result = self.execute(child)
        self.assertEqual(result["status"], "QUARANTINED")
        self.assertEqual(result["failedStep"], "restore-new")
        self.assertTrue(result["databaseWritesMayHaveOccurred"])
        self.assertNotIn("raw-password", core.private_read(self.args.output).decode())

    def test_mutated_source_input_stops_before_second_child(self):
        calls = []
        def child(argv):
            calls.append(argv)
            result = self.good_child(argv)
            core.private_write(self.args.environment, b"unexpected-owner-change", replace=True)
            return result
        result = self.execute(child)
        self.assertEqual(len(calls), 1)
        self.assertEqual(result["status"], "QUARANTINED")
        self.assertEqual(result["failedStep"], "reconcile")
        self.assertEqual(result["code"], "RECOVERY_CYCLE_INPUT_CHANGED")

    def test_unexpected_success_status_cannot_advance(self):
        result = self.execute(lambda _: {"status": "OK"})
        self.assertEqual(result["status"], "QUARANTINED")
        self.assertEqual(result["code"], "RECOVERY_CYCLE_UNEXPECTED_CHILD_STATUS")
        self.assertEqual(result["stages"], [])

    def test_source_runtime_connect_must_really_be_revoked(self):
        with patch.object(core, "guard") as guard, patch.object(core, "Pg") as pg:
            pg.return_value.query.return_value = "f"
            with self.assertRaisesRegex(core.RecoveryError, "SOURCE_RUNTIME_CONNECT_NOT_FENCED"):
                cycle.source_fenced(self.config)
            guard.assert_called_once_with(self.config)
            query = pg.return_value.query.call_args.args[0]
            self.assertEqual(query.count("NOT has_database_privilege"), 3)
            self.assertIn("uah_owned_identity_runtime", query)
            self.assertIn("uah_owned_platform_runtime", query)
            self.assertIn("uah_owned_portal_runtime", query)

    def test_child_timeout_and_stderr_never_escape(self):
        with patch.object(subprocess, "Popen") as factory, patch.object(cycle.os, "killpg") as kill:
            process = factory.return_value
            process.pid = 777001
            process.communicate.side_effect = [subprocess.TimeoutExpired(["private"], 1), (b"", b"secret-cookie")]
            with self.assertRaisesRegex(core.RecoveryError, "CHILD_TIMEOUT_OR_UNAVAILABLE"):
                cycle.invoke(["private"])
            kill.assert_called_once_with(777001, signal.SIGTERM)
            self.assertTrue(factory.call_args.kwargs["start_new_session"])
            self.assertEqual(process.communicate.call_count, 2)
        with patch.object(subprocess, "Popen") as factory:
            factory.return_value.returncode = 1
            factory.return_value.communicate.return_value = (b"", b"secret-cookie")
            with self.assertRaises(core.RecoveryError) as caught:
                cycle.invoke(["private"])
            self.assertNotIn("secret-cookie", str(caught.exception))

    def test_keyboard_interrupt_reaps_only_the_new_owned_process_group(self):
        with patch.object(subprocess, "Popen") as factory, patch.object(cycle.os, "killpg") as kill:
            process = factory.return_value
            process.pid = 777001
            process.communicate.side_effect = [KeyboardInterrupt(), (b"", b"")]
            with self.assertRaises(KeyboardInterrupt):
                cycle.invoke(["owned-private-cli"])
            kill.assert_called_once_with(777001, signal.SIGTERM)
            self.assertEqual(process.communicate.call_count, 2)

    def test_interrupt_receipt_is_written_before_cancel_propagates(self):
        with self.assertRaises(KeyboardInterrupt):
            self.execute(lambda _: (_ for _ in ()).throw(KeyboardInterrupt()))
        receipt = core.decode(core.private_read(self.args.output))
        self.assertEqual(receipt["status"], "QUARANTINED")
        self.assertEqual(receipt["code"], "RECOVERY_CYCLE_INTERRUPTED")

    def test_launch_signal_is_deferred_until_child_reference_is_assigned(self):
        with patch.object(subprocess, "Popen") as factory, patch.object(cycle.os, "killpg") as kill:
            process = unittest.mock.MagicMock()
            process.pid = 777002
            process.communicate.return_value = (b"", b"")
            def spawn_then_signal(*_args, **_kwargs):
                # The real asynchronous handler runs at the exact boundary that
                # formerly raised before `process = Popen(...)` could complete.
                signal.getsignal(signal.SIGTERM)(signal.SIGTERM, None)
                return process
            factory.side_effect = spawn_then_signal
            with cycle.interrupt_signals(), self.assertRaises(KeyboardInterrupt):
                cycle.invoke(["owned-private-cli"])
            kill.assert_called_once_with(777002, signal.SIGTERM)
            process.communicate.assert_called_once_with(timeout=5)

    def test_real_sigterm_is_converted_to_controlled_interrupt(self):
        program = ("import importlib.util,os,signal,sys; "
            "s=importlib.util.spec_from_file_location('cycle',sys.argv[1]); "
            "m=importlib.util.module_from_spec(s); s.loader.exec_module(m); "
            "\ntry:"
            "\n with m.interrupt_signals(): os.kill(os.getpid(),signal.SIGTERM)"
            "\nexcept KeyboardInterrupt: raise SystemExit(7)"
            "\nraise SystemExit(0)")
        result = subprocess.run([sys.executable, "-B", "-c", program, str(_path)],
                                capture_output=True, timeout=3, check=False)
        self.assertEqual(result.returncode, 7)
        self.assertEqual(result.stdout, b"")


if __name__ == "__main__":
    unittest.main()
