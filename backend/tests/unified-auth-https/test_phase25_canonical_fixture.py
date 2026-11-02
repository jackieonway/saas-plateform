"""Current-version fixture contracts with all process, SQL and file writes mocked.

These tests call the actual fixture entrypoints. They prove generated inputs and
rejection ordering; they do not claim PostgreSQL or HTTPS validation.
"""
import contextlib
import copy
import importlib.util
import io
import json
from pathlib import Path
import re
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch


HERE = Path(__file__).resolve().parent
SCHEMA = (HERE.parents[1] / 'migrations/0001_init.sql').read_text()


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, HERE / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


prepare = load('phase25_prepare_contract', 'prepare.py')
phase21 = load('phase25_interop_contract', 'phase21-fixture.py')
phase23_test = load('phase25_key_fixture_contract', 'phase23-fixture-selftest.py')
TASK = phase23_test.TASK
CLUSTER = phase23_test.CLUSTER
ADMIN = phase23_test.ADMIN


def forbidden(*_args, **_kwargs):
    raise AssertionError('REAL_IO_FORBIDDEN')


def exercise_prepare():
    writes, sql = {}, []

    def psql(_database, statement):
        sql.append(statement)
        if statement.startswith('SELECT system_identifier'):
            return CLUSTER
        if statement.startswith('SELECT EXISTS'):
            return 'f'
        if statement.startswith('SELECT to_reg'):
            return 't'
        return ''

    key = SimpleNamespace(public_key=lambda: SimpleNamespace(public_numbers=lambda:
        SimpleNamespace(n=(1 << 2047) + 17)), private_bytes=lambda *_args: b'PURE_MOCK_KEY')
    argv = ['prepare.py', '--task-dir', str(TASK), '--admin-url', ADMIN, '--cluster-id', CLUSTER,
            '--migration-bin', str(TASK / 'bin/platform-migrate'), '--backend-source', '/owned/backend',
            '--portal-migration-bin', str(TASK / 'bin/customer-portal-migrate'), '--tls-ca-file', '/owned/ca.pem',
            '--include-portal', '--include-phase19', '--include-phase20']
    with contextlib.ExitStack() as stack:
        stack.enter_context(patch.object(sys, 'argv', argv))
        stack.enter_context(patch.object(Path, 'resolve', lambda self, **_kw: self))
        stack.enter_context(patch.object(Path, 'exists', return_value=False))
        stack.enter_context(patch.object(Path, 'is_file', return_value=True))
        stack.enter_context(patch.object(Path, 'read_text', return_value='SELECT 1;'))
        stack.enter_context(patch.object(Path, 'write_text', side_effect=forbidden))
        stack.enter_context(patch.object(prepare, 'resolve_backend_source', return_value=Path('/owned/backend')))
        stack.enter_context(patch.object(prepare, 'psql', side_effect=psql))
        stack.enter_context(patch.object(prepare, 'save', side_effect=lambda p, value:
            writes.__setitem__(p.name, copy.deepcopy(value))))
        stack.enter_context(patch.object(prepare, 'prepare_portal_database', return_value='postgres://PURE_PORTAL'))
        stack.enter_context(patch.object(prepare.subprocess, 'run', return_value=SimpleNamespace(returncode=0)))
        stack.enter_context(patch.object(prepare.subprocess, 'Popen', side_effect=forbidden))
        stack.enter_context(patch.object(prepare.rsa, 'generate_private_key', return_value=key))
        stack.enter_context(patch.object(prepare, 'Argon2id', return_value=SimpleNamespace(
            derive_phc_encoded=lambda _value: 'PURE_MOCK_ARGON2')))
        stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
        prepare.main()
    return writes, sql


def exercise_phase21(environment, manifest, *, observed_marker=None):
    writes, queries, mutations = {}, [], []

    def read_text(filename, *_args, **_kwargs):
        if filename.name == 'https-database.json':
            return json.dumps(manifest)
        if filename.name == 'https-environment.json':
            return json.dumps(environment)
        raise AssertionError('UNEXPECTED_READ')

    def psql(_database, statement):
        if statement.startswith('SELECT '):
            queries.append(statement)
            return observed_marker or CLUSTER + '|identity-deployment/1/' + environment['ids']['deployment'] + '|0'
        mutations.append(statement)
        return ''

    error = None
    with contextlib.ExitStack() as stack:
        stack.enter_context(patch.object(sys, 'argv', ['phase21-fixture.py', '--task-dir', str(TASK),
            '--admin-url', ADMIN, '--rp-origin', 'https://customer.saas.test:24446']))
        stack.enter_context(patch.object(Path, 'resolve', lambda self, **_kw: self))
        stack.enter_context(patch.object(Path, 'is_file', return_value=True))
        stack.enter_context(patch.object(Path, 'glob', return_value=iter(())))
        stack.enter_context(patch.object(Path, 'exists', return_value=False))
        stack.enter_context(patch.object(Path, 'read_text', read_text))
        stack.enter_context(patch.object(Path, 'write_text', side_effect=lambda *_a, **_k: None))
        stack.enter_context(patch.object(Path, 'chmod', side_effect=lambda *_a, **_k: None))
        stack.enter_context(patch.object(phase21, 'save', side_effect=lambda p, value:
            writes.__setitem__(p.name, copy.deepcopy(value))))
        stack.enter_context(patch.object(phase21.cluster_binding, 'load_binding',
            return_value={'systemIdentifier': CLUSTER, 'dataDirectory': str(TASK / 'data')}))
        stack.enter_context(patch.object(phase21, 'psql', side_effect=psql))
        stack.enter_context(patch.object(phase21, 'Argon2id', return_value=SimpleNamespace(
            derive_phc_encoded=lambda _value: 'PURE_MOCK_ARGON2')))
        stack.enter_context(patch.object(prepare.subprocess, 'run', side_effect=forbidden))
        stack.enter_context(patch.object(prepare.subprocess, 'Popen', side_effect=forbidden))
        stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
        try:
            phase21.main()
        except RuntimeError as caught:
            error = caught
    return writes, queries, mutations, error


class CanonicalFixtureTests(unittest.TestCase):
    def test_prepared_services_keep_distinct_current_issuer_and_key_contracts(self):
        writes, statements = exercise_prepare()
        env = writes['https-environment.json']
        self.assertEqual(env['identity']['OIDC_ISSUER'], 'https://auth.saas.test')
        self.assertEqual(env['platform']['OIDC_ISSUER'], env['platform']['BFF_OIDC_ISSUER'])
        self.assertEqual(env['portal']['OIDC_ISSUER'], env['identity']['OIDC_ISSUER'])
        self.assertIn('OIDC_KEYSET_PATH', env['identity'])
        self.assertNotIn('OIDC_KEYSET_PATH', env['platform'])
        for kind in ('identity', 'platform', 'portal'):
            self.assertFalse({'OIDC_CODE_TTL_SECONDS', 'OIDC_TOKEN_TTL_SECONDS',
                'OIDC_REFRESH_TTL_SECONDS', 'IDENTITY_ADMIN_SERVICE_CLIENT_ID'} & env[kind].keys())
        joined = '\n'.join(statements)
        self.assertIn('INSERT INTO auth_provider_settings(', joined)
        self.assertIn('INSERT INTO auth_signing_keys(', joined)
        self.assertNotRegex(joined, r'INSERT INTO (?:oidc_|oauth_)')
        for table in re.findall(r'INSERT INTO ([a-z_]+)\(', joined):
            self.assertIn('CREATE TABLE public.' + table + ' (', SCHEMA)

    def test_phase21_preserves_delivery_origins_and_adds_rp_only_once(self):
        writes, _ = exercise_prepare()
        env, manifest = writes['https-environment.json'], writes['https-database.json']
        original = env['identity']['AUTH_DELIVERY_TEST_ORIGINS'].split(',')
        for duplicate in (False, True):
            candidate = copy.deepcopy(env)
            if duplicate:
                candidate['identity']['AUTH_DELIVERY_TEST_ORIGINS'] += ',https://customer.saas.test:24446'
            generated, _, _, error = exercise_phase21(candidate, manifest)
            self.assertIsNone(error)
            origins = generated['https-environment.json']['identity']['AUTH_DELIVERY_TEST_ORIGINS'].split(',')
            self.assertEqual(origins, original + ['https://customer.saas.test:24446'])

    def test_phase21_rejects_foreign_role_prefix_before_sql_or_output(self):
        writes, _ = exercise_prepare()
        manifest = writes['https-database.json']
        manifest['rolePrefix'] = 'uah_ffffffffffff'
        output, queries, mutations, error = exercise_phase21(writes['https-environment.json'], manifest)
        self.assertIsInstance(error, RuntimeError)
        self.assertFalse(output or queries or mutations)

    def test_phase21_canonical_account_and_permission_separation(self):
        writes, _ = exercise_prepare()
        generated, _, mutations, error = exercise_phase21(writes['https-environment.json'], writes['https-database.json'])
        self.assertIsNone(error)
        sql = '\n'.join(mutations)
        self.assertEqual(sql.count("'Phase 21 pure account',1)"), 2)
        self.assertEqual(sql.count("'SERVICE','MACHINE','NONE'"), 2)
        self.assertEqual(sql.count("'WEB_BFF','USER','ACCOUNT_ONLY'"), 2)
        env = generated['https-environment.json']['phase21']
        self.assertEqual(env['rp']['third']['OIDC_SCOPES'], 'openid profile')
        self.assertEqual(env['rp']['offline']['OIDC_SCOPES'], 'openid profile offline_access')
        self.assertEqual(env['machine']['scope'], 'demo.read')
        self.assertNotEqual(env['publicClients']['third'], env['ids']['thirdClient'])
        for table in re.findall(r'INSERT INTO ([a-z_]+)\(', sql):
            self.assertIn('CREATE TABLE public.' + table + ' (', SCHEMA)

    def test_phase21_wrong_marker_rejects_before_any_mutation(self):
        writes, _ = exercise_prepare()
        output, _, mutations, error = exercise_phase21(writes['https-environment.json'],
            writes['https-database.json'], observed_marker='FOREIGN_MARKER')
        self.assertIsInstance(error, RuntimeError)
        self.assertFalse(output or mutations)

    def test_phase23_uses_key_admin_and_rejects_any_retired_table_presence(self):
        trace = phase23_test.exercise()
        self.assertIsNone(trace['error'])
        sql = trace['mutations'][0][1]
        self.assertIn("'KEY_ADMIN','ENABLED'", sql)
        env = next(row['value'] for row in trace['writes'] if row['path'].endswith('/https-environment.json'))
        self.assertIn('IDENTITY_KEY_ADMIN_SERVICE_CLIENT_ID', env['platform'])
        self.assertNotIn('IDENTITY_ADMIN_SERVICE_CLIENT_ID', env['platform'])
        rejected = phase23_test.exercise(legacy_present=True)
        self.assertEqual(str(rejected['error']), 'PHASE23_LEGACY_STATE_PRESENT')
        self.assertFalse(rejected['mutations'] or rejected['writes'])


if __name__ == '__main__':
    unittest.main()
