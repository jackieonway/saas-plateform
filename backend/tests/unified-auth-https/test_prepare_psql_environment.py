"""Pure admin/runtime psql separation; no PostgreSQL or subprocess is started."""
import importlib.util
import os
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

loader = importlib.util.spec_from_file_location('prepare_psql_environment_target', Path(__file__).with_name('prepare.py'))
prepare = importlib.util.module_from_spec(loader)
loader.loader.exec_module(prepare)


class PsqlEnvironmentTests(unittest.TestCase):
    token = '0123456789abcdef0123456789abcdef'
    role = 'uah_' + token[-12:] + '_portal_runtime'
    database = 'unified_auth_portal_' + token
    runtime_url = 'postgres://' + role + ':PURE_PRIVATE_PASSWORD@127.0.0.1:55494/' + database
    query = 'SELECT current_user=session_user AND current_user=' + prepare.literal(role)

    def test_runtime_probe_drops_superuser_gucs_without_mutating_admin_environment(self):
        ambient = {'PATH': '/owned/bin', 'PGOPTIONS': '-c log_statement=none -c log_min_error_statement=panic',
                   'pGOPTIONS': 'PRIVATE_SUPERUSER_OPTION', 'PGUSER': 'foreign-role', 'LANG': 'C.UTF-8'}
        with patch.dict(os.environ, ambient, clear=True), patch.object(prepare.subprocess, 'run',
                return_value=SimpleNamespace(returncode=0, stdout='t\n', stderr='')) as run:
            self.assertEqual(prepare.psql(self.runtime_url, self.query), 't')
            command = run.call_args
            self.assertEqual(command.kwargs['env'], {'PATH': '/owned/bin', 'LANG': 'C.UTF-8',
                'PGOPTIONS': '-c default_transaction_read_only=on'})
            self.assertEqual(command.args[0][-1], self.runtime_url)
            self.assertIn('-X', command.args[0])
            self.assertEqual(command.kwargs['input'], self.query)
            self.assertEqual(dict(os.environ), ambient)
            prepare.psql('postgres://postgres@127.0.0.1:55494/' + self.database,
                         'SELECT pg_catalog.current_database()')
            self.assertNotIn('env', run.call_args.kwargs)
            self.assertEqual(dict(os.environ), ambient)

    def test_runtime_branch_cannot_execute_secret_write_or_extra_statement(self):
        with patch.object(prepare.subprocess, 'run') as run:
            for sql in ('INSERT INTO users VALUES (\'PRIVATE_CANARY\')', 'SELECT 1',
                        self.query + '; SELECT 1', self.query + '; UPDATE users SET status=\'ENABLED\''):
                with self.subTest(sql=sql), self.assertRaisesRegex(RuntimeError, 'fixed identity probe'):
                    prepare.psql(self.runtime_url, sql)
            run.assert_not_called()

    def test_runtime_probe_rejects_foreign_role_database_or_endpoint(self):
        with patch.object(prepare.subprocess, 'run') as run:
            for url in (self.runtime_url.replace('127.0.0.1', '127.0.0.2'),
                        self.runtime_url.replace(':55494/', ':5432/'),
                        self.runtime_url + '?options=PRIVATE_CANARY',
                        self.runtime_url.replace(self.database, 'postgres'),
                        self.runtime_url.replace(self.role, 'foreign_runtime')):
                with self.subTest(url=url), self.assertRaisesRegex(RuntimeError, 'fixed identity probe'):
                    prepare.psql(url, self.query)
            run.assert_not_called()

    def test_error_and_fatal_diagnostics_only_return_structured_sqlstate(self):
        for prefix in ('ERROR', 'FATAL'):
            diagnostic = prefix + ':  42501: PRIVATE_CANARY postgres://user:PRIVATE_PASSWORD@host/db'
            with patch.object(prepare.subprocess, 'run', return_value=SimpleNamespace(
                    returncode=2, stdout='', stderr=diagnostic)):
                with self.assertRaisesRegex(RuntimeError, r'^owned HTTPS fixture SQL failed \(SQLSTATE 42501\)$'):
                    prepare.psql('postgres://postgres@127.0.0.1:55494/' + self.database, 'SELECT 1')


if __name__ == '__main__':
    unittest.main()
