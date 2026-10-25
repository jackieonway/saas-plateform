"""Offline U06 epoch contracts. No PostgreSQL, source material or HTTP access."""
import contextlib
import copy
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('u06_epoch', Path(__file__).parents[1] / 'prepare-uuid-u06-epoch.py')
E = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(E)
MARKER = 'saas-uuid-import/1/019a1163-0000-7000-8000-000000000001/' + 'a' * 64


def state_source():
    old = {f'client-{i}': f'old-isolated-secret-{i:040d}' for i in range(12)}
    clients = [{'client_id': c, 'client_type': 'CONFIDENTIAL', 'client_secret_hash': E.sha(s.encode()),
                'status': 'DISABLED' if c == 'client-11' else 'ENABLED', 'redirect_uris': 'https://external.example.invalid/callback',
                'description': "retain quotes ' $$ ; -- and Unicode 中文"} for c, s in old.items()]
    clients.append({'client_id': 'public-device', 'client_type': 'PUBLIC', 'client_secret_hash': None, 'status': 'ENABLED'})
    clients[0]['application_id'] = 'app-stores'
    clients[1]['application_id'] = 'app-orders'
    state = {'clients': clients, 'users': [{'id': 'synthetic-user', 'session_version': 7, 'password_hash': 'opaque-unchanged', 'updated_at': 'same'}],
             'applications': [{'id': 'app-stores', 'code': 'stores'}, {'id': 'app-orders', 'code': 'work-orders'}], 'credentials': dict.fromkeys(E.CREDENTIALS, 0)}
    source = {s: {} for s in E.SERVICES}
    source['platform'] = {'DATABASE_URL': 'old-database', 'JWT_SECRET': 'old-jwt-' + 'a' * 48, 'METRICS_TOKEN': 'old-platform-metric'}
    source['stores'] = {'STORES_DATABASE_URL': 'old-stores', 'PLATFORM_CLIENT_ID': 'client-0', 'PLATFORM_CLIENT_SECRET': old['client-0'], 'STORES_METRICS_TOKEN': 'old-store-metric'}
    source['work_orders'] = {'WORK_ORDERS_DATABASE_URL': 'old-orders', 'PLATFORM_CLIENT_ID': 'client-1', 'PLATFORM_CLIENT_SECRET': old['client-1'], 'WORK_ORDERS_METRICS_TOKEN': 'old-order-metric'}
    source['notification'] = {'DATABASE_URL': 'old-reader', 'METRICS_TOKEN': 'old-notification-metric'}
    source['examples_oauth'] = {'OAUTH_AUTH_CLIENT_ID': 'client-2', 'OAUTH_AUTH_CLIENT_SECRET': old['client-2'], 'OAUTH_DEVICE_CLIENT_ID': 'public-device', 'OAUTH_CC_CLIENT_ID': 'client-3', 'OAUTH_CC_CLIENT_SECRET': old['client-3']}
    return state, source


def receipt(generation):
    return {'format': 'saas-uuid-u06-target-receipts/1', 'generation': generation,
            'targets': {k: {'database': d['database'], 'owner': d['owner'], 'database_oid': 100 + i, 'owner_oid': 200 + i,
                            'schema_sha256': 'a' * 64, 'import_marker': MARKER} for i, (k, d) in enumerate(E.names(generation).items())}}


class PureContracts(unittest.TestCase):
    def test_generation_and_exact_receipt_reject_crossing_or_unknown_fields(self):
        for generation in ('final', 'recovery'):
            good = receipt(generation)
            self.assertEqual(E.validate_receipt(good, generation), good)
            for key in ('database', 'owner'):
                bad = copy.deepcopy(good)
                bad['targets']['platform'][key] = E.names('recovery' if generation == 'final' else 'final')['platform'][key]
                with self.assertRaises(E.Invalid):
                    E.validate_receipt(bad, generation)
            bad = copy.deepcopy(good)
            bad['targets']['platform']['database_oid'] = True
            with self.assertRaises(E.Invalid):
                E.validate_receipt(bad, generation)
            bad = copy.deepcopy(good)
            bad['extra'] = 1
            with self.assertRaises(E.Invalid):
                E.validate_receipt(bad, generation)

    def test_fresh_material_keeps_opaque_ids_and_changes_every_secret(self):
        state, source = state_source()
        first = E.generate_material(state, source, 'final')
        second = E.generate_material(state, source, 'recovery')
        self.assertEqual(set(first['epoch']), {'format', 'generation', 'jwt_secret_sha256', 'previous_jwt_secret_sha256', 'confidential_clients', 'secrets_fresh'})
        self.assertEqual(len(first['client_secrets']), 12)
        self.assertEqual(len(set(first['client_secrets'].values()) | set(first['passwords'].values())), 16)
        self.assertTrue(set(first['client_secrets'].values()).isdisjoint(second['client_secrets'].values()))
        for s in E.SERVICES:
            self.assertEqual(set(first['env'][s]), E.INPUT_KEYS[s])
        for record in first['epoch']['confidential_clients']:
            self.assertNotEqual(record['previous_secret_hash'], record['new_secret_hash'])
        self.assertEqual(first['env']['stores']['PLATFORM_CLIENT_ID'], source['stores']['PLATFORM_CLIENT_ID'])
        self.assertEqual(first['env']['examples_oauth']['OAUTH_DEVICE_CLIENT_ID'], 'public-device')
        self.assertEqual(first['env']['examples_chat'], {})

    def test_wrong_source_env_secret_is_rejected(self):
        state, source = state_source()
        source['stores']['PLATFORM_CLIENT_SECRET'] = 'different-previous-generation'
        with self.assertRaisesRegex(E.Invalid, 'PREVIOUS_ENV_CLIENT_SECRET_HASH_MISMATCH'):
            E.generate_material(state, source, 'final')

    def test_rotation_projection_changes_only_hash_and_session_version(self):
        state, source = state_source()
        material = E.generate_material(state, source, 'final')
        after = E.rotate_expected(state, material['hashes'])
        self.assertEqual(after['users'][0]['session_version'], 8)
        for before_client, after_client in zip(state['clients'], after['clients']):
            if before_client['client_type'] == 'CONFIDENTIAL':
                before_client = {**before_client, 'client_secret_hash': after_client['client_secret_hash']}
            self.assertEqual(before_client, after_client)
        self.assertEqual(after['clients'][11]['status'], 'DISABLED')
        self.assertEqual(after['applications'], state['applications'])
        self.assertEqual(state['users'][0]['session_version'], 7)

    def test_sql_does_not_interpolate_dollar_delimited_restored_data(self):
        state, source = state_source()
        material = E.generate_material(state, source, 'final')
        sql = E.rotate_sql(state, material['hashes'])
        self.assertNotIn('DO $$', sql)
        self.assertEqual(sql.count('UPDATE public.users SET session_version=session_version+1;'), 1)
        self.assertIn('credential before-image mismatch', sql)
        self.assertIn('credential after-image mismatch', sql)
        self.assertNotIn('updated_at=', sql)
        self.assertNotIn('TRUNCATE ', sql)
        self.assertNotIn('DROP ', sql)

    def test_grants_exclude_metadata_and_notification_has_no_dml(self):
        for kind in E.KINDS:
            tables = [*E.METADATA[kind], 'business_table', 'notifications']
            sql = E.grants_sql(kind, 'final', tables)
            dml = next(line for line in sql.splitlines() if line.startswith('GRANT INSERT'))
            for table in E.METADATA[kind]:
                self.assertNotIn('public.' + E.ident(table), dml)
            self.assertIn('default_transaction_read_only=off', sql)
            self.assertNotIn('GRANT ALL', sql)
            self.assertNotIn('TRUNCATE', sql)
            if kind == 'platform':
                reader = E.ident(E.runtime_roles('final')['notification'])
                reader_lines = [line for line in sql.splitlines() if reader in line]
                self.assertEqual(len(reader_lines), 4)
                self.assertFalse(any('INSERT' in line or 'SEQUENCE' in line or 'CREATE ON' in line for line in reader_lines))

    def test_roles_start_nologin_and_never_reuse_u05_names(self):
        sql = E.create_roles_sql('final', {s: 'synthetic-password' for s in E.runtime_roles('final')})
        self.assertEqual(sql.count('CREATE ROLE '), 4)
        self.assertEqual(sql.count(' NOLOGIN '), 4)
        self.assertNotIn(' LOGIN ', sql)
        self.assertNotIn('u05', sql)
        self.assertNotIn(' ALTER ROLE ', sql)

    def test_duplicate_keys_and_nonfinite_envelopes_reject(self):
        for raw in ('{"x":1,"x":2}', '{"x":NaN}', '{"x":Infinity}'):
            with self.assertRaises(E.Invalid):
                E.parsed(raw)


@unittest.skipUnless(os.name == 'posix', 'requires Linux private modes')
class PrivateJournalContracts(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.root.chmod(0o700)
        self.base = self.root / 'data'
        self.scratch = self.root / 'scratch'
        self.keys = self.root / 'keys'
        for p in (self.base, self.scratch, self.keys):
            p.mkdir(mode=0o700)
        for name, value in (('BASE', self.base), ('SCRATCH', self.scratch), ('KEYS', self.keys)):
            patcher = patch.object(E, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        patcher = patch.object(E.subprocess, 'run', return_value=type('Mount', (), {'returncode': 0, 'stdout': 'tmpfs\n'})())
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_private_write_is_new_only_and_links_rejected(self):
        target = self.root / 'private.json'
        E.write_new(target, {'value': 'synthetic-private'})
        with self.assertRaises(FileExistsError):
            E.write_new(target, {})
        self.assertEqual(E.read_json(target), {'value': 'synthetic-private'})
        target.chmod(0o644)
        with self.assertRaises(E.Invalid):
            E.read_json(target)
        target.chmod(0o600)
        alias = self.root / 'alias'
        alias.symlink_to(target)
        with self.assertRaises(E.Invalid):
            E.read_json(alias)
        alias.unlink()
        os.link(target, alias)
        with self.assertRaises(E.Invalid):
            E.read_json(target)

    def test_runner_pins_catalog_search_path_and_utc_for_raw_schema(self):
        # pg_get_constraintdef/pg_get_expr change qualification with search_path.
        # The real registration pins pg_catalog output, including in read-only
        # preflight sessions before an explicit transaction SET is possible.
        runner = E.Runner(self.root / 'commands')
        response = type('Reply', (), {'returncode': 0, 'stdout': b'{}\n', 'stderr': b''})()
        with patch.object(E.subprocess, 'run', return_value=response) as invoked:
            runner.sql(E.names('final')['platform']['database'], 'SELECT jsonb_build_object();')
        argv = invoked.call_args.args[0]
        options = next(value for value in argv if value.startswith('PGOPTIONS='))
        self.assertIn('-c search_path=pg_catalog', options)
        self.assertIn('-c timezone=UTC', options)
        self.assertIn('-c default_transaction_read_only=on', options)
        self.assertIn('/usr/bin/env', argv)
        self.assertIn('-i', argv)

    def test_prepare_failure_never_creates_epoch_directory(self):
        target = self.root / 'target.json'
        E.write_new(target, receipt('final'))
        state, source = state_source()
        envs = {}
        for service, env in source.items():
            path = self.root / (service + '.env')
            E.write_new(path, ''.join(k + '=' + v + '\n' for k, v in env.items()).encode())
            envs[service] = str(path)
        config = {'generation': 'final', 'target_receipt_file': str(target), 'source_env_files': envs}
        class Fake:
            def __init__(self, directory):
                pass
        with patch.object(E, 'inspect', side_effect=E.Invalid('TARGET_NOT_RESTORED')):
            with self.assertRaisesRegex(E.Invalid, 'TARGET_NOT_RESTORED'):
                E.prepare(config, Fake)
        self.assertFalse((self.base / 'epoch-final').exists())

    def test_prepare_create_new_and_failed_activation_cannot_repeat_increment(self):
        state, source = state_source()
        target = self.root / 'target.json'
        E.write_new(target, receipt('final'))
        envs = {}
        for service, env in source.items():
            path = self.root / (service + '.env')
            E.write_new(path, ''.join(k + '=' + v + '\n' for k, v in env.items()).encode())
            envs[service] = str(path)
        config = {'format': 'saas-uuid-u06-epoch-input/1', 'generation': 'final', 'target_receipt_file': str(target), 'source_env_files': envs}
        calls = []
        class Fake:
            def __init__(self, directory):
                E.new_dir(directory)
            def sql(self, database, sql, write=False):
                calls.append((database, sql, write))
                if 'UPDATE public.users SET session_version' in sql:
                    raise E.Invalid('SIMULATED_PLATFORM_TRANSACTION_ROLLBACK')
                return {'roles_created': 4}
        observed = {'tables': ['_sqlx_migrations', 'oauth_clients', 'users', 'notifications'], 'schema_material': []}
        with patch.object(E, 'inspect', return_value=observed), patch.object(E, 'cluster_proof', return_value={'roles': []}), patch.object(E, 'platform_state', return_value=state):
            report = E.prepare(config, Fake)
            self.assertEqual(report['status'], 'PREPARED_NO_DATABASE_MUTATION')
            journal = self.base / 'epoch-final'
            self.assertEqual(len(E.read_json(journal / 'client-secrets.private.json')['clients']), 12)
            with self.assertRaises(E.Invalid):
                E.prepare(config, Fake)
            with patch.object(E, 'validate_config', side_effect=lambda value: value):
                with self.assertRaisesRegex(E.Invalid, 'SIMULATED_PLATFORM_TRANSACTION_ROLLBACK'):
                    E.activate(journal, Fake)
                prior_calls = len(calls)
                with self.assertRaises(FileExistsError):
                    E.activate(journal, Fake)
                self.assertEqual(len(calls), prior_calls)
            self.assertFalse((journal / 'activate/complete.json').exists())
            self.assertEqual(sum('UPDATE public.users SET session_version' in sql for _, sql, _ in calls), 1)
            self.assertFalse(any(' LOGIN;' in sql for _, sql, _ in calls))


if __name__ == '__main__':
    unittest.main()
