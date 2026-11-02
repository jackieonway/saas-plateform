"""Failed prepare cleanup regression; PG, subprocesses and services are mocked."""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

loader = importlib.util.spec_from_file_location('phase23_environment_preparation',
    Path(__file__).with_name('phase23-environment.py'))
environment = importlib.util.module_from_spec(loader)
loader.loader.exec_module(environment)


class PreparationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='phase23-prepare-cleanup-pure-', dir='/tmp')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.root.chmod(0o700)
        patched = patch.object(environment, 'TASK', self.root)
        patched.start()
        self.addCleanup(patched.stop)
        self.task = environment.Task.__new__(environment.Task)
        self.task.uid, self.task.gid = os.getuid(), os.getgid()
        self.task.username = 'owned-unprivileged'
        self.task.state_path = self.root / 'phase23-environment-state.json'
        self.task.state = {'prepared': False, 'mode': 'SOURCE', 'processes': {},
                           'dockerBefore': 'PURE_MOCK', 'filesystemBefore': 'PURE_MOCK'}
        self.task.save()
        for method in ('stop_pending_spawns', 'stop_service', 'stop_auxiliary', 'stop_process'):
            setattr(self.task, method, Mock())
        self.task.pg = Mock(return_value=json.dumps({'nonDefaultDatabases': 0, 'taskRoles': 0}))
        self.tls = self.root / 'phase23-server.key'
        self.task.write(self.tls, b'PURE_TEMPORARY_KEY_SENTINEL')
        self.workspace = self.root / 'phase23-gateway-workspace'
        (self.workspace / '.local-run').mkdir(mode=0o700, parents=True)
        (self.workspace / '.local-run/server.key').write_bytes(b'PURE_OWNED_KEY_COPY')
        self.binding = self.root / environment.cluster_binding.NAME
        self.binding.write_text('{"publicBindingSentinel":true}')
        self.binding.chmod(0o644)
        self.unrelated = self.root / 'unrelated-sentinel'
        self.unrelated.write_bytes(b'UNCHANGED')

    def cleanup(self):
        with patch.object(environment, 'run', return_value=b'PURE_DF_NOT_REAL_DOCKER'):
            return environment.cleanup(self.task, SimpleNamespace(hba_restored=True))

    def test_runuser_is_absolute_with_minimal_child_path(self):
        with patch.object(environment, 'run', return_value=b'pure') as run:
            self.assertEqual(self.task.user(['python3', '-B', 'owned.py']), b'pure')
        self.assertEqual(run.call_args.args[0],
            ['/usr/sbin/runuser', '-u', 'owned-unprivileged', '--', 'python3', '-B', 'owned.py'])
        self.assertNotIn('/usr/sbin', run.call_args.kwargs['environment']['PATH'].split(':'))

    def test_guarded_zero_resources_allows_owned_partial_file_cleanup(self):
        result = self.cleanup()
        self.assertEqual(result['result'], 'PASS')
        self.assertTrue(result['emptyPreparationVerifiedByGuardedPostgres'])
        self.assertEqual((result['databasesRemaining'], result['rolesRemaining']), (0, 0))
        self.assertEqual((result['databasesRemoved'], result['rolesRemoved']), (0, 0))
        self.assertFalse(self.tls.exists())
        self.assertFalse(self.workspace.exists())
        self.assertFalse(self.task.state_path.exists())
        self.assertTrue(self.binding.exists())
        self.assertEqual(self.unrelated.read_bytes(), b'UNCHANGED')
        statements = [call.args[0] for call in self.task.pg.call_args_list]
        self.assertEqual(len(statements), 1)
        self.assertTrue(statements[0].startswith('SELECT jsonb_build_object'))
        self.assertIn("datname NOT IN('postgres','template0','template1')", statements[0])
        self.assertIn("starts_with(rolname,'uah_')", statements[0])
        self.assertNotIn('DROP', statements[0])

    def test_unknown_database_or_role_keeps_files_and_never_drops(self):
        for observed in ({'nonDefaultDatabases': 1, 'taskRoles': 0},
                         {'nonDefaultDatabases': 0, 'taskRoles': 8}):
            with self.subTest(observed=observed):
                self.task.pg.reset_mock()
                self.task.pg.return_value = json.dumps(observed)
                result = self.cleanup()
                self.assertEqual(result['result'], 'FAIL')
                self.assertIn('EMPTY_PREPARATION_RESOURCE_GUARD', result['errors'])
                self.assertTrue(self.tls.exists())
                self.assertTrue(self.workspace.exists())
                self.assertTrue(self.task.state_path.exists())
                self.assertFalse(any('DROP' in call.args[0] for call in self.task.pg.call_args_list))

    def test_missing_manifest_is_not_enough_if_prepare_complete_or_one_manifest_exists(self):
        self.task.state['prepared'] = True
        with self.assertRaisesRegex(RuntimeError, 'EMPTY_PREPARATION_NOT_PROVEN'):
            self.task.empty_preparation_resources()
        self.task.state['prepared'] = False
        for name in ('https-database.json', 'phase23-resources.json'):
            path = self.root / name
            path.write_bytes(b'PRESERVE_UNKNOWN_MANIFEST')
            with self.subTest(manifest=name), self.assertRaisesRegex(RuntimeError, 'EMPTY_PREPARATION_NOT_PROVEN'):
                self.task.empty_preparation_resources()
            self.assertEqual(path.read_bytes(), b'PRESERVE_UNKNOWN_MANIFEST')
            path.unlink()
        self.task.pg.assert_not_called()

    def test_unavailable_guard_or_noninteger_count_fails_closed(self):
        self.task.pg.side_effect = RuntimeError('PHASE23_CLUSTER')
        result = self.cleanup()
        self.assertEqual(result['result'], 'FAIL')
        self.assertTrue(self.tls.exists())
        self.task.pg.side_effect = None
        for observed in ({'nonDefaultDatabases': False, 'taskRoles': 0},
                         {'nonDefaultDatabases': 0}, {'nonDefaultDatabases': 0, 'taskRoles': '0'}):
            self.task.pg.return_value = json.dumps(observed)
            with self.subTest(observed=observed), self.assertRaisesRegex(RuntimeError, 'EMPTY_PREPARATION_UNKNOWN_RESOURCES'):
                self.task.empty_preparation_resources()
        self.assertTrue(self.tls.exists())


if __name__ == '__main__':
    unittest.main()
