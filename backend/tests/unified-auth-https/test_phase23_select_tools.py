"""Pure stopped-source tool selection tests; no DB, processes or network."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

loader = importlib.util.spec_from_file_location('phase23_select_tools_target',
    Path(__file__).with_name('phase23-environment.py'))
environment = importlib.util.module_from_spec(loader)
loader.loader.exec_module(environment)


class SelectToolsTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='phase23-select-tools-pure-', dir='/tmp')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.root.chmod(0o700)
        patched = patch.object(environment, 'TASK', self.root)
        patched.start()
        self.addCleanup(patched.stop)
        self.snapshot = self.root / 'source-snapshot-phase23-tools-r9'
        self.source = self.snapshot / 'saas-plateform/backend/tests/unified-auth-https/tool.py'
        self.source.parent.mkdir(parents=True)
        self.source.write_bytes(b'PURE_PUBLIC_TOOL')
        self.source.chmod(0o644)
        self.configuration = self.root / 'https-environment.json'
        self.configuration.write_bytes(b'PRIVATE_CONFIG_UNCHANGED_SENTINEL')
        self.task = Mock(uid=os.getuid())
        self.task.state = {'prepared': True, 'mode': 'SOURCE', 'sourceQuarantined': False,
            'targetOpen': False, 'gate': None, 'snapshot': 'phase23-tools-r8', 'snapshotSha256': 'a' * 64,
            'buildSnapshot': 'phase23-apps-r2', 'buildSnapshotSha256': 'b' * 64,
            'processes': {}, 'pendingSpawns': {}, 'privateUnrelatedState': 'PRESERVED'}
        self.original = copy.deepcopy(self.task.state)
        self.manifest = {'snapshotSha256': 'c' * 64,
            'inputs': {self.source.relative_to(self.snapshot).as_posix(): 'd' * 64}}
        self.task.snapshot.return_value = (self.snapshot, self.manifest, {'unused': True})
        self.task.guard_databases.return_value = {}
        self.task.json.return_value = {'privateCanary': 'PRIVATE_CONFIG_SENTINEL'}
        self.args = SimpleNamespace(snapshot='phase23-tools-r9', build_snapshot='phase23-apps-r2', hba_restored=True)

    def select(self):
        return environment.select_tools(self.task, self.args)

    def assert_unchanged(self):
        self.assertEqual(self.task.state, self.original)
        self.task.save.assert_not_called()
        self.task.pg.assert_not_called()
        self.task.write.assert_not_called()
        self.assertEqual(self.configuration.read_bytes(), b'PRIVATE_CONFIG_UNCHANGED_SENTINEL')

    def test_success_selects_only_tools_and_keeps_build_database_and_secrets(self):
        result = self.select()
        expected = {**self.original, 'snapshot': self.args.snapshot, 'snapshotSha256': 'c' * 64}
        self.assertEqual(self.task.state, expected)
        self.task.snapshot.assert_called_once_with('phase23-tools-r9', build_label='phase23-apps-r2')
        self.assertEqual(self.task.guard_databases.call_args_list[0].args, ('SOURCE',))
        self.assertEqual(self.task.guard_databases.call_args_list[0].kwargs, {'quiet': True})
        self.assertEqual(self.task.guard_databases.call_args_list[1].args, ('RESTORE',))
        self.assertEqual(self.task.guard_databases.call_args_list[1].kwargs, {'quiet': True, 'absent_ok': True})
        self.task.guard_roles.assert_called_once()
        self.task.environment_binding.assert_called_once_with(self.task.json.return_value, 'SOURCE')
        self.task.save.assert_called_once()
        self.task.pg.assert_not_called()
        self.task.write.assert_not_called()
        self.assertTrue(result['buildSnapshotUnchanged'])
        self.assertNotIn('PRIVATE_', json.dumps(result))
        self.assertEqual(self.configuration.read_bytes(), b'PRIVATE_CONFIG_UNCHANGED_SENTINEL')

    def test_hba_ack_state_or_original_build_missing_rejects_before_validation(self):
        for target, key, value in ((self.args, 'hba_restored', False), (self.args, 'build_snapshot', 'phase23-apps-r3'),
                (self.args, 'snapshot', 'phase23-apps-r2'), (self.task.state, 'prepared', False),
                (self.task.state, 'mode', 'RESTORE'), (self.task.state, 'sourceQuarantined', True),
                (self.task.state, 'targetOpen', True), (self.task.state, 'gate', {'not': 'empty'})):
            old = target[key] if isinstance(target, dict) else getattr(target, key)
            if isinstance(target, dict): target[key] = value
            else: setattr(target, key, value)
            before = copy.deepcopy(self.task.state)
            with self.subTest(key=key, value=value), self.assertRaises(environment.EnvironmentFailure):
                self.select()
            self.assertEqual(self.task.state, before)
            self.task.snapshot.assert_not_called()
            self.task.save.assert_not_called()
            if isinstance(target, dict): target[key] = old
            else: setattr(target, key, old)
        self.assert_unchanged()

    def test_any_daemon_aux_namespace_proxy_or_pending_lease_blocks_selection(self):
        names = ['namespace.pid', 'namespace.identity', 'gateway.pid', 'phase23-rp.pid', 'collector.pid',
            'phase23-scram-proxy.pid', 'browser-proxy.pid', 'phase23-spawn-untracked.json',
            *('service-' + name + '.pid' for name in environment.DAEMONS)]
        for name in names:
            path = self.root / name
            path.write_bytes(b'PRESERVE_UNTRACKED_LEASE')
            with self.subTest(lease=name), self.assertRaises(environment.EnvironmentFailure):
                self.select()
            self.assertEqual(path.read_bytes(), b'PRESERVE_UNTRACKED_LEASE')
            path.unlink()
        for key in ('processes', 'pendingSpawns'):
            self.task.state[key] = {'untracked': {'pid': 12345}}
            with self.subTest(state=key), self.assertRaisesRegex(environment.EnvironmentFailure, 'TOOLS_PENDING_PROCESSES'):
                self.select()
            self.task.state[key] = {}
        self.task.snapshot.assert_not_called()
        self.assert_unchanged()

    def test_guard_or_compiled_input_failure_and_existing_target_keep_selection_unchanged(self):
        for method in ('guard_databases', 'guard_roles', 'environment_binding', 'snapshot'):
            getattr(self.task, method).side_effect = environment.EnvironmentFailure('COMPILED_APPLICATION_INPUT_DRIFT')
            with self.subTest(method=method), self.assertRaises(environment.EnvironmentFailure):
                self.select()
            getattr(self.task, method).side_effect = None
            self.assert_unchanged()
        self.task.guard_databases.side_effect = lambda mode, **kwargs: {} if mode == 'SOURCE' else {'portal': 'exists'}
        with self.assertRaisesRegex(environment.EnvironmentFailure, 'TOOLS_TARGET_ALREADY_EXISTS'):
            self.select()
        self.assert_unchanged()

    def test_group_writable_new_code_or_untraversable_snapshot_keeps_old_selection(self):
        self.source.chmod(0o660)
        with self.assertRaisesRegex(environment.EnvironmentFailure, 'TOOLS_SNAPSHOT_NOT_OWNER_READABLE'):
            self.select()
        self.assert_unchanged()
        self.source.chmod(0o644)
        self.snapshot.chmod(0o600)
        with self.assertRaisesRegex(environment.EnvironmentFailure, 'TOOLS_SNAPSHOT_NOT_OWNER_TRAVERSABLE'):
            self.select()
        self.assert_unchanged()


if __name__ == '__main__':
    unittest.main()
