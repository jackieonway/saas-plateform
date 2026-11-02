"""Capacity runner options, candidate dispatch and finally replay without I/O."""
import importlib.util
import ast
import json
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch


HERE = Path(__file__).resolve().parent


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, HERE / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


runner = load('phase25_capacity_runner_target', 'phase22-run.py')
cleanup = load('phase25_capacity_cleanup_target', 'phase22-cleanup-selftest.py')


class CapacityRunnerTests(unittest.TestCase):
    def test_both_filesystem_samples_accept_only_the_existing_owned_task(self):
        tree = ast.parse((HERE / 'phase22-run.py').read_text())
        samples = [node for node in ast.walk(tree) if isinstance(node, ast.Call)
            and isinstance(node.func, ast.Attribute) and node.func.attr == 'check_output'
            and node.args and isinstance(node.args[0], ast.List) and node.args[0].elts
            and isinstance(node.args[0].elts[0], ast.Constant) and node.args[0].elts[0].value == 'df']
        self.assertEqual(len(samples), 2)
        owned = Path('/owned/existing-task')
        def observe(command, **kwargs):
            # A removed old cache path must never become a mandatory preflight
            # or turn otherwise complete cleanup into a failure.
            self.assertEqual(command, ['df', '-B1', str(owned)])
            self.assertEqual(kwargs, {'text': True, 'timeout': 15})
            return 'OWNED_FILESYSTEM_OBSERVED'
        process = SimpleNamespace(check_output=Mock(side_effect=observe))
        for node in samples:
            code = compile(ast.Expression(body=node), '<capacity-filesystem-sample>', 'eval')
            self.assertEqual(eval(code, {'subprocess': process, 'TASK': owned}),
                             'OWNED_FILESYSTEM_OBSERVED')
        self.assertEqual(process.check_output.call_count, 2)

    def test_phase22_default_preserves_old_mode_and_phase25_requires_explicit_label(self):
        args = runner.options([])
        self.assertEqual((args.phase, args.label, args.faults_only), (22, 'phase22-final-r1', False))
        with self.assertRaisesRegex(RuntimeError, 'SNAPSHOT_LABEL_REJECTED'):
            runner.options(['--phase', '25'])
        for label in ('phase22-old', '../phase25-other', 'phase25-BAD'):
            with self.subTest(label=label), self.assertRaisesRegex(RuntimeError, 'SNAPSHOT_LABEL_REJECTED'):
                runner.options(['--phase', '25', '--label', label])

    def test_phase25_disallows_old_capacity_evidence_and_accepts_exact_build_label(self):
        for extra in ([], ['--build-label', 'phase25-candidate-r1']):
            args = runner.options(['--phase', '25', '--label', 'phase25-tools-r2', *extra])
            self.assertEqual(args.phase, 25)
        with self.assertRaisesRegex(RuntimeError, 'REQUIRES_FRESH_CAPACITY'):
            runner.options(['--phase', '25', '--label', 'phase25-tools-r2', '--faults-only'])
        with self.assertRaisesRegex(RuntimeError, 'SNAPSHOT_LABEL_REJECTED'):
            runner.options(['--phase', '25', '--label', 'phase25-tools-r2', '--build-label', 'phase23-old'])

    def test_phase25_uses_existing_full_candidate_verifier_with_explicit_build_snapshot(self):
        args = runner.options(['--phase', '25', '--label', 'phase25-tools-r2',
                               '--build-label', 'phase25-candidate-r1'])
        snapshot = runner.TASK / 'source-snapshot-phase25-tools-r2'
        manifest = {'snapshotSha256': 'a' * 64}
        attestation = {'snapshotLabel': 'phase25-candidate-r1', 'snapshotSha256': 'b' * 64,
            'builtServices': ['identity', 'platform', 'portal', 'stores', 'work-orders', 'notification', 'cli'],
            'binaries': {'PURE_MOCK_BIN': 'c' * 64}}
        task = Mock()
        task.snapshot.return_value = (snapshot, manifest, attestation)
        module = SimpleNamespace(Task=Mock(return_value=task))
        spec = SimpleNamespace(loader=SimpleNamespace(exec_module=lambda _module: None))
        with patch.object(runner.importlib.util, 'spec_from_file_location', return_value=spec), \
             patch.object(runner.importlib.util, 'module_from_spec', return_value=module):
            actual, proof = runner.candidate(args)
        task.snapshot.assert_called_once_with('phase25-tools-r2', build_label='phase25-candidate-r1')
        self.assertEqual(actual, snapshot)
        self.assertEqual(proof['binaries'], attestation['binaries'])
        self.assertEqual(proof['buildSnapshotLabel'], 'phase25-candidate-r1')

    def test_copied_controller_selects_metrics_gateway_and_prevents_import_bytecode(self):
        data = (HERE / 'environment.sh').read_bytes()
        generated = runner.capacity_controller(data)
        self.assertIn(b'gateway_script=phase22-gateway.py', generated)
        self.assertIn(b'case "$gateway_script" in phase22-gateway.py)', generated)
        self.assertIn(b'export PYTHONDONTWRITEBYTECODE=1', generated)
        self.assertNotIn(b'TASK_GATEWAY_SCRIPT', generated)
        self.assertEqual((HERE / 'environment.sh').read_bytes(), data)
        for invalid in (b'', data + data, data.replace(b'phase23-gateway.py', b'future-gateway.py')):
            with self.assertRaisesRegex(RuntimeError, 'GATEWAY_CONTROLLER_CONTRACT_CHANGED'):
                runner.capacity_controller(invalid)

    def test_current_runtime_report_requires_phase_and_fresh_capacity_marker(self):
        filename = runner.TASK / 'phase25-capacity-runtime.json'
        baseline = {'phase': 25, 'result': 'PASS', 'freshCapacityRequired': True}
        with patch.object(runner.cluster_binding, 'protected_read',
                return_value=(json.dumps(baseline).encode(), None)) as read:
            _raw, value = runner.runtime_report(filename, 1000, 25)
        read.assert_called_once_with(filename, uid=1000, mode=0o600, maximum=8 * 1024 * 1024)
        self.assertEqual(value, baseline)
        for changes in ({'phase': 22}, {'freshCapacityRequired': False}, {'result': 'UNKNOWN'},
                        {'capacityEvidence': {'reusedCompletedComponentEvidence': True}}):
            with self.subTest(changes=changes), patch.object(runner.cluster_binding, 'protected_read',
                    return_value=(json.dumps({**baseline, **changes}).encode(), None)), \
                    self.assertRaisesRegex(RuntimeError, 'RUNTIME_REPORT_REJECTED'):
                runner.runtime_report(filename, 1000, 25)

    def test_phase25_finally_preserves_ownership_and_continues_all_independent_cleanup(self):
        code, _digest = cleanup.runner_finally((HERE / 'phase22-run.py').read_text())
        for scenario in ('success', 'control-failure', 'database-failure', 'malformed-pid'):
            with self.subTest(scenario=scenario):
                result = cleanup.run_case(code, scenario, phase=25)
                self.assertEqual(result['result'], 'PASS')


if __name__ == '__main__':
    unittest.main()
