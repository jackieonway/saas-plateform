"""Pure migration environment and diagnostic isolation; no DB or subprocess."""
import importlib.util
import os
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

loader = importlib.util.spec_from_file_location('prepare_migration_environment_target',
    Path(__file__).with_name('prepare.py'))
prepare = importlib.util.module_from_spec(loader)
loader.loader.exec_module(prepare)


class MigrationEnvironmentTests(unittest.TestCase):
    def test_pg_prefixes_removed_only_for_migration_and_exact_url_overrides_ambient(self):
        ambient = {'PATH': '/owned/bin', 'HOME': '/owned/home', 'PGOPTIONS': 'PRIVATE_LOG_OPTIONS',
            'PGPASSWORD': 'PRIVATE_PASSWORD', 'pguser': 'foreign', 'pGPORT': '5432', 'pg': 'rejected',
            'DATABASE_URL': 'PRIVATE_FOREIGN_TARGET', 'LANG': 'C.UTF-8'}
        target = 'postgres://postgres@127.0.0.1:55494/unified_auth_https_owned'
        with patch.dict(os.environ, ambient, clear=True):
            child = prepare.migration_environment(target)
            self.assertEqual(child, {'PATH': '/owned/bin', 'HOME': '/owned/home', 'LANG': 'C.UTF-8',
                                     'DATABASE_URL': target})
            self.assertEqual(dict(os.environ), ambient)
            with patch.object(prepare.subprocess, 'run', return_value=SimpleNamespace(
                    returncode=0, stdout='PURE_PSQL_RESULT', stderr='')) as run:
                self.assertEqual(prepare.psql(target, 'SELECT 1'), 'PURE_PSQL_RESULT')
            self.assertNotIn('env', run.call_args.kwargs)
            self.assertEqual(os.environ['PGOPTIONS'], 'PRIVATE_LOG_OPTIONS')

    def test_known_diagnostic_code_is_returned_without_private_stderr(self):
        canary = 'postgres://user:PRIVATE_PASSWORD@foreign/private_database'
        for code in prepare.MIGRATION_ERROR_CODES:
            result = SimpleNamespace(stderr=('Error: ' + code + ': ' + canary).encode())
            for portal in (False, True):
                message = str(prepare.migration_failure(result, portal))
                self.assertIn(code, message)
                self.assertNotIn('PRIVATE_', message)
                self.assertNotIn('postgres://', message)

    def test_unknown_migration_prefix_or_embedded_code_never_becomes_diagnostic(self):
        for raw in (b'PRIVATE_CANARY', b'MIGRATION_PRIVATE_CANARY',
                    b'MIGRATION_DATABASE_URL_INVALID_PRIVATE_CANARY',
                    b'PRIVATE_MIGRATION_DATABASE_URL_INVALID', b'X' * 65536 + b' MIGRATION_DATABASE_URL_INVALID'):
            self.assertEqual(str(prepare.migration_failure(SimpleNamespace(stderr=raw))),
                             'current migration command failed')


if __name__ == '__main__':
    unittest.main()
