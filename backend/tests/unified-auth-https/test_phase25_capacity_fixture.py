"""Pure mock coverage of the current Phase22 fixture; no PG or runtime I/O.

The real fixture main and cluster-binding validator run against in-memory files,
metadata and SQL responses. Filesystem writes, subprocesses and sockets are
forbidden. Run in WSL with python3 -B to avoid import bytecode output.
"""
import contextlib
import copy
import importlib.util
import io
import json
import os
from pathlib import Path
import socket
import stat
import subprocess
import sys
from types import SimpleNamespace
import unittest
from unittest import mock


HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location('phase25_capacity_fixture_target', HERE / 'phase22-fixture.py')
FIXTURE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FIXTURE)
TASK = FIXTURE.TASK
CLUSTER = '18446744073709551614'  # Synthetic, never a live system identifier.
STALE = '7701548153213487841'
TOKEN = '0123456789abcdef0123456789abcdef'
NAME = 'unified_auth_https_' + TOKEN
PREFIX = 'uah_' + TOKEN[-12:]
ADMIN = 'postgres://postgres@127.0.0.1:55494/postgres'
DATABASE = ADMIN.rsplit('/', 1)[0] + '/' + NAME
DEPLOYMENT = '00000000-0000-7000-8000-000000000001'
CA = TASK / 'mock-phase25-ca.pem'


def identifier(number):
    return '00000000-0000-7000-8000-' + str(number).zfill(12)


def baseline():
    binding = {'contractVersion': FIXTURE.cluster_binding.CONTRACT, 'taskDir': str(TASK),
        'ownerMarker': TASK.name + '-pg55494', 'taskDevice': 10, 'taskInode': 20,
        'ownerDevice': 10, 'ownerInode': 21, 'taskUid': 1000, 'taskGid': 1000,
        'systemIdentifier': CLUSTER, 'dataDirectory': str(TASK / 'data'),
        'host': '127.0.0.1', 'port': 55494, 'adminDatabase': 'postgres'}
    manifest = {'database': NAME, 'rolePrefix': PREFIX, 'clusterId': CLUSTER}
    environment = {'ids': {'deployment': DEPLOYMENT, 'resource': identifier(2),
                          'openid': identifier(3), 'profile': identifier(4)},
        'identity': {'DATABASE_URL': 'postgres://' + PREFIX + '_identity_runtime@127.0.0.1:55494/' + NAME,
                     'APP_ENV': 'test', 'OIDC_ISSUER': FIXTURE.ISSUER,
                     'CONTEXT_PLATFORM_CA_FILE': str(CA)}}
    return binding, manifest, environment


def ready_fixture():
    ids = {key: identifier(index) for index, key in enumerate((
        'user', 'session', 'userApp', 'userClient', 'group', 'consent', 'sector',
        'machineApp', 'machineClient', 'machineResource', 'machineScope',
        'machineGrant', 'machineAccess'), start=10)}
    return {'contractVersion': FIXTURE.CONTRACT, 'state': 'READY', 'ids': ids,
        'binding': {'database': NAME, 'rolePrefix': PREFIX, 'deployment': DEPLOYMENT, 'clusterId': CLUSTER},
        'counts': {'codeSources': 2, 'refreshSources': 8}, 'codes': {'state': 'NOT_ISSUED', 'ttlSeconds': 60},
        'terminal': None, 'files': {'codes': str(TASK / 'phase22-code-sources.json')},
        'user': {'redirectUri': 'https://rp.example.test/callback',
                 'scopeIds': [identifier(3), identifier(4)]}}


def exercise(*, stage='prepare', cluster=CLUSTER, manifest_cluster=None, binding_changes=None,
             actual_cluster=None, actual_directory=None, ownership=True, task=TASK,
             owner_marker=None, binding_owner_marker=None, binding_read_error=None,
             fixture_cluster=None, drift_before_transaction=False, extra_args=()):
    """Exercise production control flow with every external effect trapped."""
    binding, manifest, environment = baseline()
    binding['systemIdentifier'] = cluster
    binding.update(binding_changes or {})
    manifest['clusterId'] = cluster if manifest_cluster is None else manifest_cluster
    fixture = ready_fixture()
    fixture['binding']['clusterId'] = cluster if fixture_cluster is None else fixture_cluster
    trace = {'reads': [], 'queries': [], 'writes': [], 'transactions': [], 'mutations': [],
             'protectedReads': [], 'hashParameters': [], 'output': '', 'error': None}
    root_info = SimpleNamespace(st_mode=stat.S_IFDIR | 0o700, st_uid=1000, st_gid=1000,
                                st_dev=10, st_ino=20)
    data_info = SimpleNamespace(st_mode=stat.S_IFDIR | 0o700, st_uid=1000, st_gid=1000,
                                st_dev=10, st_ino=22)
    owner_info = SimpleNamespace(st_dev=10, st_ino=21)
    owner = (TASK.name + '-pg55494\n') if owner_marker is None else owner_marker
    bound_owner = owner if binding_owner_marker is None else binding_owner_marker
    store = {TASK / 'OWNER': owner.encode(), TASK / 'https-database.json': json.dumps(manifest).encode(),
             TASK / 'https-environment.json': json.dumps(environment).encode(),
             TASK / 'phase22-fixture.json': json.dumps(fixture).encode(),
             CA: b'-----BEGIN CERTIFICATE-----\nPURE_MOCK_CERTIFICATE\n-----END CERTIFICATE-----\n'}
    sequence = 100

    def next_id():
        nonlocal sequence
        sequence += 1
        return identifier(sequence)

    def random_hex(length):
        nonlocal sequence
        sequence += 1
        return format(sequence, 'x').zfill(length * 2)

    def read_regular(path, **_kwargs):
        trace['reads'].append(str(path))
        if path not in store:
            raise AssertionError('UNEXPECTED_PRIVATE_READ')
        return store[path]

    def protected_read(path, **kwargs):
        trace['protectedReads'].append((str(path), kwargs))
        if path == TASK / 'OWNER':
            if kwargs != {'uid': 1000, 'mode': 0o600, 'maximum': 256}:
                raise AssertionError('OWNER_PROTECTION_CHANGED')
            return bound_owner.encode(), owner_info
        if path == TASK / FIXTURE.cluster_binding.NAME:
            if kwargs != {'uid': 0, 'mode': 0o644, 'maximum': 4096}:
                raise AssertionError('ROOT_BINDING_PROTECTION_CHANGED')
            if binding_read_error is not None:
                raise RuntimeError(binding_read_error)
            return json.dumps(binding).encode(), SimpleNamespace()
        raise AssertionError('UNEXPECTED_PROTECTED_READ')

    def lstat(path):
        if path == TASK:
            return root_info
        if path == TASK / 'data':
            return data_info
        raise AssertionError('UNEXPECTED_STAT')

    def write_private(path, value, replace=False):
        trace['writes'].append((str(path), copy.deepcopy(value), replace))

    def sql_matches(statement, *, transaction=False):
        expected_cluster = cluster if actual_cluster is None else actual_cluster
        if transaction and drift_before_transaction:
            expected_cluster = STALE if cluster != STALE else CLUSTER
        expected_directory = str(TASK / 'data') if actual_directory is None else actual_directory
        return (ownership and "system_identifier::text FROM pg_control_system())='" + expected_cluster + "'" in statement
                and "current_setting('data_directory')='" + expected_directory + "'" in statement)

    def psql(database, statement):
        if database != DATABASE:
            raise AssertionError('UNEXPECTED_DATABASE_TARGET')
        if statement.startswith('SELECT '):
            trace['queries'].append(statement)
            if 'identity_deployment_state' in statement:
                return 'true' if sql_matches(statement) else 'false'
            if statement.startswith('SELECT count(*) FROM auth_authorization_requests '):
                return '0'
            if statement.startswith('SELECT (NOT EXISTS(SELECT 1 FROM auth_grants)'):
                return 'true'
            if statement.startswith('SELECT (EXISTS(SELECT 1 FROM auth_resources '):
                return 'true'
            raise AssertionError('UNEXPECTED_QUERY')
        if statement.startswith('BEGIN;'):
            trace['transactions'].append(statement)
            if not sql_matches(statement, transaction=True):
                raise RuntimeError('PHASE22_OWNER_GUARD')
            trace['mutations'].append(statement)
            return ''
        raise AssertionError('UNEXPECTED_SQL')

    class FakeArgon:
        def __init__(self, **kwargs):
            trace['hashParameters'].append(kwargs)

        def derive_phc_encoded(self, _password):
            return 'PURE_MOCK_PHC_NOT_A_PASSWORD_HASH'

    def forbidden(*_args, **_kwargs):
        raise AssertionError('REAL_FILESYSTEM_PROCESS_OR_NETWORK_IO_FORBIDDEN')

    output = io.StringIO()
    with contextlib.ExitStack() as stack:
        stack.enter_context(mock.patch.object(Path, 'resolve', lambda path, **_kwargs: path))
        stack.enter_context(mock.patch.object(Path, 'is_symlink', return_value=False))
        stack.enter_context(mock.patch.object(Path, 'lstat', lstat))
        stack.enter_context(mock.patch.object(Path, 'glob', side_effect=lambda *_args: iter(())))
        for method in ('open', 'write_text', 'write_bytes', 'mkdir', 'unlink', 'rename', 'replace'):
            stack.enter_context(mock.patch.object(Path, method, side_effect=forbidden))
        for method in ('open', 'mkdir', 'unlink', 'replace', 'rename'):
            stack.enter_context(mock.patch.object(os, method, side_effect=forbidden))
        stack.enter_context(mock.patch('builtins.open', side_effect=forbidden))
        stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=forbidden))
        stack.enter_context(mock.patch.object(subprocess, 'Popen', side_effect=forbidden))
        stack.enter_context(mock.patch.object(socket, 'socket', side_effect=forbidden))
        stack.enter_context(mock.patch.object(socket, 'create_connection', side_effect=forbidden))
        stack.enter_context(mock.patch.object(FIXTURE, 'read_regular', side_effect=read_regular))
        stack.enter_context(mock.patch.object(FIXTURE, 'write_private', side_effect=write_private))
        stack.enter_context(mock.patch.object(FIXTURE.cluster_binding, 'protected_read', side_effect=protected_read))
        stack.enter_context(mock.patch.object(FIXTURE, 'psql', side_effect=psql))
        stack.enter_context(mock.patch.object(FIXTURE, 'new_id', side_effect=next_id))
        stack.enter_context(mock.patch.object(FIXTURE, 'Argon2id', FakeArgon))
        stack.enter_context(mock.patch.object(FIXTURE.secrets, 'token_hex', side_effect=random_hex))
        stack.enter_context(mock.patch.object(FIXTURE.secrets, 'token_urlsafe', return_value='PURE_MOCK_PASSWORD'))
        stack.enter_context(mock.patch.object(FIXTURE.secrets, 'token_bytes', side_effect=lambda count: b'x' * count))
        stack.enter_context(mock.patch.object(FIXTURE.os, 'urandom', side_effect=lambda count: b'x' * count))
        argv = ['phase22-fixture.py', '--task-dir', str(task), '--admin-url', ADMIN, '--stage', stage, *extra_args]
        stack.enter_context(mock.patch.object(sys, 'argv', argv))
        stack.enter_context(contextlib.redirect_stdout(output))
        try:
            FIXTURE.main()
        except Exception as error:
            trace['error'] = error
    trace['output'] = output.getvalue()
    return trace


class CapacityFixtureBindingTests(unittest.TestCase):
    def assert_rejected_without_effects(self, trace, code):
        self.assertIsInstance(trace['error'], RuntimeError)
        self.assertEqual(str(trace['error']), code)
        self.assertFalse(trace['writes'])
        self.assertFalse(trace['transactions'])
        self.assertFalse(trace['mutations'])
        self.assertFalse(trace['hashParameters'])
        self.assertEqual(trace['output'], '')

    def test_changed_cluster_prepares_with_original_defaults_and_generation_one(self):
        trace = exercise()
        self.assertIsNone(trace['error'])
        self.assertEqual(trace['output'], 'PHASE22_INPUTS_READY code_batch_pending=true refresh_sources=8\n')
        fixture = trace['writes'][-1][1]
        self.assertEqual(fixture['binding'], ready_fixture()['binding'])
        self.assertEqual(fixture['contractVersion'], 'unified-auth-phase22-fixture/1')
        self.assertEqual(fixture['state'], 'READY')
        self.assertEqual(fixture['counts'], {'codeSources': 1200, 'refreshSources': 8})
        self.assertEqual(fixture['codes'], {'state': 'NOT_ISSUED', 'ttlSeconds': 60})
        self.assertEqual(trace['hashParameters'], [{'salt': b'x' * 16, 'length': 32,
            'iterations': 3, 'lanes': 1, 'memory_cost': 65536}])
        statements = trace['mutations'][0].split(';\n')
        user = next(row for row in statements if row.startswith('INSERT INTO users('))
        session = next(row for row in statements if row.startswith('INSERT INTO sso_sessions('))
        self.assertTrue(user.endswith(",'ENABLED',1)"))
        self.assertIn(',1,statement_timestamp(),', session)
        user_grants = [row for row in statements if row.startswith('INSERT INTO auth_grants(') and "'USER'" in row]
        self.assertEqual(len(user_grants), 8)
        self.assertTrue(all(",1,1,id,'ACCOUNT_ONLY',1,1,1," in row for row in user_grants))

    def test_all_three_stages_guard_actual_cluster_and_directory_inside_transaction(self):
        for stage in ('prepare', 'codes', 'terminal'):
            with self.subTest(stage=stage):
                trace = exercise(stage=stage)
                self.assertIsNone(trace['error'])
                self.assertEqual(len(trace['mutations']), 1)
                transaction = trace['mutations'][0]
                self.assertIn(FIXTURE.cluster_binding.sql_guard(baseline()[0]), transaction)
                self.assertIn("current_database()='" + NAME + "'", transaction)
                self.assertIn('identity_deployment_state', transaction)
                self.assertIn("'identity-deployment/1/" + DEPLOYMENT + "'", transaction)
                self.assertIn('PHASE22_OWNER_GUARD', transaction)
                self.assertLess(transaction.index('PHASE22_OWNER_GUARD'), transaction.index('INSERT INTO '))
                self.assertNotIn(STALE, transaction)

    def test_stale_manifest_cluster_fails_before_queries_or_private_outputs(self):
        trace = exercise(manifest_cluster=STALE)
        self.assert_rejected_without_effects(trace, 'PHASE22_FRESH_CANONICAL_MANIFEST_REQUIRED')
        self.assertFalse(trace['queries'])

    def test_actual_cluster_directory_or_database_ownership_mismatch_fails_before_effects(self):
        for arguments in ({'actual_cluster': STALE}, {'actual_directory': '/tmp/foreign/data'}, {'ownership': False}):
            with self.subTest(arguments=arguments):
                trace = exercise(**arguments)
                self.assert_rejected_without_effects(trace, 'PHASE22_DATABASE_OR_ROLE_OWNERSHIP_REJECTED')
                self.assertEqual(len(trace['queries']), 1)

    def test_root_binding_changed_fields_and_invalid_identifier_fail_before_queries(self):
        for field, value in (('taskDir', '/tmp/foreign'), ('taskUid', 0), ('taskInode', 99),
                             ('ownerInode', 99), ('dataDirectory', '/tmp/foreign/data'), ('port', 5432)):
            with self.subTest(field=field):
                trace = exercise(binding_changes={field: value})
                self.assert_rejected_without_effects(trace, 'PHASE23_CLUSTER_BINDING_OWNERSHIP_CHANGED')
                self.assertFalse(trace['queries'])
        trace = exercise(binding_changes={'systemIdentifier': '18446744073709551616'})
        self.assert_rejected_without_effects(trace, 'PHASE23_CLUSTER_BINDING_SYSTEM_IDENTIFIER_INVALID')

    def test_task_path_and_both_owner_marker_checks_fail_before_queries(self):
        for arguments, code in (({'task': Path('/tmp/rust-saas-unified-auth-foreign')},
                                 'PHASE22_OWNED_LINUX_TASK_REQUIRED'),
                                ({'owner_marker': 'foreign-owner\n'}, 'PHASE22_TASK_MARKER_REJECTED'),
                                ({'binding_owner_marker': 'foreign-owner\n'},
                                 'PHASE23_CLUSTER_BINDING_OWNER_MARKER_INVALID')):
            with self.subTest(arguments=arguments):
                trace = exercise(**arguments)
                self.assert_rejected_without_effects(trace, code)
                self.assertFalse(trace['queries'])

    def test_root_protected_binding_read_failure_precedes_queries_and_outputs(self):
        trace = exercise(binding_read_error='PHASE23_CLUSTER_BINDING_FILE_INVALID')
        self.assert_rejected_without_effects(trace, 'PHASE23_CLUSTER_BINDING_FILE_INVALID')
        self.assertFalse(trace['queries'])
        self.assertEqual(trace['protectedReads'][-1], (str(TASK / FIXTURE.cluster_binding.NAME),
                         {'uid': 0, 'mode': 0o644, 'maximum': 4096}))

    def test_stale_saved_capacity_binding_cannot_issue_codes_or_terminal_source(self):
        for stage in ('codes', 'terminal'):
            with self.subTest(stage=stage):
                self.assert_rejected_without_effects(exercise(stage=stage, fixture_cluster=STALE),
                                                     'PHASE22_FIXTURE_BINDING_REJECTED')

    def test_cluster_drift_is_rechecked_by_every_transaction_before_database_mutation(self):
        for stage in ('prepare', 'codes', 'terminal'):
            with self.subTest(stage=stage):
                trace = exercise(stage=stage, drift_before_transaction=True)
                self.assertIsInstance(trace['error'], RuntimeError)
                self.assertEqual(str(trace['error']), 'PHASE22_OWNER_GUARD')
                self.assertEqual(len(trace['transactions']), 1)
                self.assertFalse(trace['mutations'])
                self.assertEqual(trace['output'], '')

    def test_original_capacity_count_bounds_still_reject_before_input_reads(self):
        for name, count in (('--code-sources', 749), ('--code-sources', 10001),
                            ('--refresh-sources', 0), ('--refresh-sources', 33)):
            with self.subTest(name=name, count=count):
                trace = exercise(extra_args=(name, str(count)))
                self.assert_rejected_without_effects(trace, 'PHASE22_INPUT_COUNT_REJECTED')
                self.assertFalse(trace['reads'])


if __name__ == '__main__':
    unittest.main()
