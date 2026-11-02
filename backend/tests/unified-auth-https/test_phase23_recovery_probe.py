"""Probe safety tests; no PG, HBA, application, Docker or network operation."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('phase23_recovery_probe',
    Path(__file__).with_name('phase23-recovery-probe.py'))
probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(probe)


class ProbeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='phase23-recovery-probe-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.ops = self.root / probe.OPS_RELATIVE
        self.ops.mkdir(parents=True)
        (self.ops / 'core.py').write_text('# public fixture\n')
        self.output = io.StringIO()

    def run_main(self, launch=None, extra=()):
        with patch.object(probe, 'TASK', self.root), patch.object(probe, 'WORKSPACE', self.root), \
             patch.object(probe.os, 'getuid', return_value=0), \
             patch.object(probe.cluster_binding, 'load_binding', return_value={'taskUid': 0}), \
             (patch.object(probe, 'invoke', side_effect=launch) if launch is not None else contextlib.nullcontext()), \
             contextlib.redirect_stdout(self.output):
            result = probe.main(['--snapshot', 'phase23-tools-r15', '--build-snapshot', 'phase23-apps-r2',
                                 '--workspace-ops', *extra])
        return result, json.loads(self.output.getvalue())

    def subprocess_fixture(self, fail_at=None):
        calls = []
        negatives = {'format': 'identity-recovery-negative-probe/1', 'status': 'PASS', 'sequence': 2,
            'validationLayer': 'REAL_AUTHENTICATED_FILE_LOADERS_AND_READONLY_PG_PREFLIGHT',
            'restoreSqlExecuted': False, 'databaseWrites': False,
            'cases': [{'case': name, 'status': 'PASS'} for name in (
                'ciphertext-tamper', 'manifest-tamper', 'wrong-encryption-key', 'mixed-source-binding',
                'signed-old-floor', 'signed-old-current', 'signed-old-record', 'missing-chain', 'extra-chain')]}
        def launch(command, **kwargs):
            self.assertEqual(kwargs, {'stdout': subprocess.PIPE, 'stderr': subprocess.PIPE, 'timeout': 240})
            command = list(map(str, command))
            if '-c' in command:
                stage = command[-2]
            elif any(Path(value).name == 'negative_probe.py' for value in command):
                stage = 'file-negative-' + command[command.index('--stage') + 1]
            elif any(Path(value).name == 'phase23-recovery-files.py' for value in command):
                stage = command[7]
            else:
                stage = command[3]
            calls.append((stage, command))
            value = {'result': 'PASS', 'processesRemaining': 0}
            if stage in ('capture', 'capture-deny'):
                value = {'status': 'PASS', 'result': {'sequence': 1 if stage == 'capture' else 2}}
            elif stage == 'file-negative-probe':
                value = negatives
            failed = stage == fail_at
            return subprocess.CompletedProcess(command, 2 if failed else 0,
                stdout=json.dumps(value).encode(), stderr=b'PRIVATE_STDERR_CANARY' if failed else b'')
        return calls, negatives, launch

    def test_file_negatives_save_original_sequence_then_probe_latest_without_restore(self):
        calls, negatives, launch = self.subprocess_fixture()
        with patch.object(probe.subprocess, 'run', side_effect=launch):
            result, report = self.run_main(extra=['--file-negatives'])
        self.assertEqual(result, 0)
        stages = [stage for stage, _ in calls]
        self.assertEqual(stages, ['prepare', 'start', 'stop-writers', 'prepare', 'init',
            'catalog-contract', 'capture', 'file-negative-save-baseline', 'capture-deny',
            'file-negative-probe', 'cleanup', 'cleanup'])
        baseline = calls[7][1]
        deny = calls[8][1]
        current = calls[9][1]
        self.assertEqual(baseline[-2:], ['--minimum-sequence', '1'])
        self.assertEqual(deny[-2:], ['capture-deny', '1'])
        self.assertEqual(current[-2:], ['--minimum-sequence', '2'])
        self.assertEqual(baseline[baseline.index('--config') + 1], str(self.root / 'phase23-recovery-config.json'))
        self.assertIn(str(self.ops / 'negative_probe.py'), baseline)
        self.assertEqual(report['recoveryNegatives'], negatives)
        self.assertEqual(len(report['recoveryNegatives']['cases']), 9)
        self.assertFalse(report['recoveryNegatives']['databaseWrites'])
        self.assertNotIn('quarantine-source', stages)
        self.assertNotIn('restore-new', stages)

    def test_file_negatives_finish_before_optional_restore_and_keep_same_sequence(self):
        calls, _, launch = self.subprocess_fixture()
        with patch.object(probe.subprocess, 'run', side_effect=launch):
            result, report = self.run_main(extra=['--file-negatives', '--through-restore'])
        self.assertEqual(result, 0)
        stages = [stage for stage, _ in calls]
        self.assertEqual(stages[7:-2], ['file-negative-save-baseline', 'capture-deny',
            'file-negative-probe', 'quarantine-source', 'restore-new',
            'catalog-privilege-negative', 'reconcile', 'gate'])
        for stage, command in calls:
            if stage in ('restore-new', 'catalog-privilege-negative', 'reconcile', 'gate'):
                self.assertEqual(command[-1], '2')
        self.assertEqual(report['contractVersion'], 'phase23-actual-recovery-probe/1')
        self.assertEqual(report['validationLayer'], 'ACTUAL_OWNED_POSTGRES_COMPONENT_PROBE_NO_BROWSER_OR_OLD_CARRIERS')
        self.assertFalse(report['hbaChanged'])

    def test_no_file_negative_flag_preserves_default_capture_and_restore_workflows(self):
        for extra, expected_deny in (([], False), (['--through-restore'], True)):
            with self.subTest(extra=extra):
                self.output = io.StringIO()
                calls, _, launch = self.subprocess_fixture()
                with patch.object(probe.subprocess, 'run', side_effect=launch):
                    result, report = self.run_main(extra=extra)
                self.assertEqual(result, 0)
                stages = [stage for stage, _ in calls]
                self.assertEqual('capture-deny' in stages, expected_deny)
                self.assertFalse(any(stage.startswith('file-negative-') for stage in stages))
                self.assertNotIn('recoveryNegatives', report)

    def test_file_negative_failure_stops_before_restore_and_always_runs_final_cleanup(self):
        for failed_stage in ('file-negative-save-baseline', 'capture-deny', 'file-negative-probe'):
            with self.subTest(failed_stage=failed_stage):
                self.output = io.StringIO()
                calls, negatives, launch = self.subprocess_fixture(fail_at=failed_stage)
                with patch.object(probe.subprocess, 'run', side_effect=launch):
                    result, report = self.run_main(extra=['--file-negatives', '--through-restore'])
                self.assertEqual(result, 2)
                stages = [stage for stage, _ in calls]
                self.assertEqual(stages[-2:], ['cleanup', 'cleanup'])
                self.assertEqual(calls[-1][1][-1], '--hba-restored')
                self.assertNotIn('quarantine-source', stages)
                self.assertNotIn('restore-new', stages)
                self.assertEqual(len(report['cleanup']), 2)
                self.assertTrue(all(item['passed'] for item in report['cleanup']))
                self.assertNotIn('PRIVATE_STDERR_CANARY', self.output.getvalue())
                if failed_stage == 'file-negative-probe':
                    self.assertEqual(report['recoveryNegatives'], negatives)

    def test_any_stage_failure_still_runs_both_cleanup_stages(self):
        calls = []
        def launch(command):
            calls.append(list(map(str, command)))
            if len(calls) == 2:
                raise RuntimeError('PRIVATE_EXCEPTION_MESSAGE_CANARY')
            return True, {'result': 'PASS', 'processesRemaining': 0}
        result, report = self.run_main(launch)
        self.assertEqual(result, 2)
        self.assertEqual(len(report['cleanup']), 2)
        self.assertEqual(calls[-2][3], 'cleanup')
        self.assertEqual(calls[-1][-1], '--hba-restored')
        self.assertNotIn('PRIVATE_EXCEPTION_MESSAGE_CANARY', self.output.getvalue())
        self.assertFalse(report['hbaChanged'])
        self.assertIn('NO_BROWSER_OR_OLD_CARRIERS', report['validationLayer'])

    def test_cleanup_retries_same_owned_scope_without_relaxing_guard(self):
        calls, final = [], 0
        def launch(command):
            nonlocal final
            calls.append(list(map(str, command)))
            if calls[-1][-1] == '--hba-restored':
                final += 1
                return final > 1, {'result': 'PASS' if final > 1 else 'FAIL', 'processesRemaining': 0}
            if len(calls) == 1:
                return False, {'status': 'FAIL'}
            return True, {'result': 'AWAIT_HBA', 'processesRemaining': 0}
        with patch.object(probe.time, 'sleep'):
            _, report = self.run_main(launch)
        self.assertEqual(final, 2)
        self.assertEqual(calls[-1], calls[-2])
        self.assertEqual(len(report['cleanup']), 3)
        self.assertTrue(report['cleanup'][-1]['passed'])

    def test_unstructured_child_output_does_not_escape_as_diagnostic(self):
        value = types.SimpleNamespace(returncode=2, stdout=b'PRIVATE_STDOUT_CANARY', stderr=b'PRIVATE_STDERR_CANARY')
        with patch.object(probe.subprocess, 'run', return_value=value):
            passed, report = probe.invoke(['owned-command'])
        self.assertFalse(passed)
        self.assertEqual(report, {'status': 'FAIL', 'code': 'UNSTRUCTURED_CHILD_FAILURE'})

    def test_real_diagnostic_subprocess_emits_only_class_and_trusted_frames(self):
        (self.ops / 'core.py').write_text(
            "def load_config(path): return {}\n"
            "def keys(config): return {}\n"
            "def capture(config, material): raise ValueError('PRIVATE_MESSAGE_LOCALS_CANARY')\n")
        result = subprocess.run([sys.executable, '-B', '-c', probe.PRIVATE_CALL,
            str(self.ops), str(self.root), 'capture', '0'], capture_output=True, timeout=3)
        self.assertEqual(result.returncode, 2)
        self.assertEqual(result.stderr, b'')
        self.assertNotIn(b'PRIVATE_MESSAGE_LOCALS_CANARY', result.stdout)
        report = json.loads(result.stdout)
        self.assertEqual(report['errors'][0], {'exceptionClass': 'ValueError', 'frames': [
            {'module': 'core.py', 'function': 'capture', 'line': 3}]})


if __name__ == '__main__':
    unittest.main()
