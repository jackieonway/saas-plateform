"""Pure mock regression of the real Phase23 fixture main; no runtime I/O.

Real private reads/writes, PostgreSQL, RSA generation, subprocesses and generic
filesystem opens/mutations are replaced or forbidden. These tests demonstrate
ordering and task binding only, never actual PG, key generation or deployment.
Run with python3 -B phase23-fixture-selftest.py to avoid import bytecode output.
"""
import contextlib
import copy
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import types
import unittest
from unittest import mock
from urllib.parse import urlsplit


HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location('phase23_fixture_selftest_target', HERE / 'phase23-fixture.py')
FIXTURE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FIXTURE)
TASK = FIXTURE.TASK
CLUSTER = '18446744073709551614'  # Synthetic identity, never a live cluster.
TOKEN = '0123456789abcdef0123456789abcdef'
NAME = 'unified_auth_https_' + TOKEN
PREFIX = 'uah_' + TOKEN[-12:]
DEPLOYMENT = '00000000-0000-7000-8000-000000000001'
ADMIN = 'postgres://postgres@127.0.0.1:55494/postgres'
SIGNING = TASK / 'phase23-mock-signing.json'


def baseline():
    portal = {'database': 'unified_auth_portal_' + TOKEN, 'owner': PREFIX + '_portal_owner',
              'runtime': PREFIX + '_portal_runtime', 'marker': 'unified-auth-https-portal/1/' + DEPLOYMENT,
              'initializationPending': False}
    manifest = {'database': NAME, 'rolePrefix': PREFIX, 'clusterId': CLUSTER, 'portal': portal}
    environment = {'ids': {'deployment': DEPLOYMENT, 'organization': '00000000-0000-7000-8000-000000000002',
                          'tenant': '00000000-0000-7000-8000-000000000004',
                          'secondOrganization': '00000000-0000-7000-8000-000000000005',
                          'application': '00000000-0000-7000-8000-000000000006',
                          'admin': '00000000-0000-7000-8000-000000000007',
                          'adminMember': '00000000-0000-7000-8000-000000000008'},
        'phase21': {'ids': {'otherUser': '00000000-0000-7000-8000-000000000003'},
                    'rp': {'third': {'mockPublic': 'only'}}, 'machine': {'mockPublic': 'only'},
                    'resource': {'mockPublic': 'only'}}}
    for kind in ('identity', 'platform', 'portal'):
        name = portal['database'] if kind == 'portal' else NAME
        environment[kind] = {'DATABASE_URL': 'postgres://' + PREFIX + '_' + kind
                             + '_runtime:mock-old-password@127.0.0.1:55494/' + name}
    environment['identity']['OIDC_KEYSET_PATH'] = str(SIGNING)
    signing = {'keys': [{'kid': 'owned-https', 'n': 'PURE_MOCK_PUBLIC', 'e': 'AQAB',
                         'privateKeyPem': 'PURE_MOCK_PRIVATE_NOT_RSA_MATERIAL'}]}
    return manifest, environment, signing


def exercise(manifest=None, environment=None, signing=None, *, shared_marker=None,
             portal_marker=None, legacy_present=False, symlink=None):
    """Call the production main with every external effect trapped in memory."""
    defaults = baseline()
    manifest = copy.deepcopy(manifest if manifest is not None else defaults[0])
    environment = copy.deepcopy(environment if environment is not None else defaults[1])
    signing = copy.deepcopy(signing if signing is not None else defaults[2])
    trace = {'writes': [], 'queries': [], 'mutations': [], 'reads': [], 'rsa': 0, 'output': '', 'error': None}
    store = {str(TASK / 'https-database.json'): manifest,
             str(TASK / 'https-environment.json'): environment,
             environment['identity']['OIDC_KEYSET_PATH']: signing}
    sequence = 10

    def new_id():
        nonlocal sequence
        sequence += 1
        return '00000000-0000-7000-8000-' + str(sequence).zfill(12)

    def random_hex(count):
        nonlocal sequence
        sequence += 1
        return format(sequence, 'x').zfill(count * 2)

    def private_read(path, **_kwargs):
        trace['reads'].append(str(path))
        if str(path) not in store:
            raise AssertionError('UNEXPECTED_PRIVATE_READ')
        return json.dumps(copy.deepcopy(store[str(path)])).encode()

    def private_write(path, value, replace=False):
        trace['writes'].append({'path': str(path), 'value': copy.deepcopy(value), 'replace': replace})

    def psql(database, statement):
        if statement.startswith('SELECT '):
            trace['queries'].append((database, statement))
            if 'FROM pg_control_system()' in statement:
                if database == ADMIN.rsplit('/', 1)[0] + '/' + NAME:
                    return shared_marker if shared_marker is not None else CLUSTER + '|identity-deployment/1/' + DEPLOYMENT
                if database == ADMIN.rsplit('/', 1)[0] + '/unified_auth_portal_' + TOKEN:
                    return portal_marker if portal_marker is not None else CLUSTER + '|unified-auth-https-portal/1/' + DEPLOYMENT
                raise AssertionError('UNEXPECTED_MARKER_DATABASE')
            if statement.startswith("SELECT to_regclass('public."):
                return 't' if legacy_present else 'f'
            raise AssertionError('UNEXPECTED_READ_ONLY_SQL')
        if statement.startswith('BEGIN;'):
            trace['mutations'].append((database, statement))
            return ''
        raise AssertionError('UNEXPECTED_SQL')

    class FakeKey:
        def public_key(self):
            return types.SimpleNamespace(public_numbers=lambda: types.SimpleNamespace(n=(1 << 2047) + 17, e=65537))

        def private_bytes(self, *_args):
            return b'PURE_MOCK_PRIVATE_NOT_RSA_MATERIAL'

    def generate_private_key(**kwargs):
        if kwargs != {'public_exponent': 65537, 'key_size': 2048}:
            raise AssertionError('UNEXPECTED_RSA_REQUEST')
        trace['rsa'] += 1
        return FakeKey()

    def forbidden(*_args, **_kwargs):
        raise AssertionError('REAL_FILESYSTEM_OR_PROCESS_IO_FORBIDDEN')

    output = io.StringIO()
    with contextlib.ExitStack() as stack:
        stack.enter_context(mock.patch.object(Path, 'resolve', lambda path, **_kwargs: path))
        stack.enter_context(mock.patch.object(Path, 'glob', return_value=iter(())))
        stack.enter_context(mock.patch.object(Path, 'exists', return_value=False))
        stack.enter_context(mock.patch.object(Path, 'is_symlink', lambda path: path == symlink))
        for method in ('open', 'write_text', 'write_bytes', 'mkdir', 'unlink', 'rename', 'replace'):
            stack.enter_context(mock.patch.object(Path, method, side_effect=forbidden))
        for method in ('open', 'mkdir', 'unlink', 'replace', 'rename'):
            stack.enter_context(mock.patch.object(os, method, side_effect=forbidden))
        stack.enter_context(mock.patch('builtins.open', side_effect=forbidden))
        stack.enter_context(mock.patch.object(subprocess, 'run', side_effect=forbidden))
        stack.enter_context(mock.patch.object(subprocess, 'Popen', side_effect=forbidden))
        stack.enter_context(mock.patch.object(FIXTURE.os, 'getuid', return_value=1000))
        stack.enter_context(mock.patch.object(FIXTURE.cluster_binding, 'load_binding',
            return_value={'systemIdentifier': CLUSTER, 'dataDirectory': str(TASK / 'data')}))
        stack.enter_context(mock.patch.object(FIXTURE.core, 'private_directory', return_value=None))
        stack.enter_context(mock.patch.object(FIXTURE.core, 'private_read', side_effect=private_read))
        stack.enter_context(mock.patch.object(FIXTURE.core, 'private_write', side_effect=private_write))
        stack.enter_context(mock.patch.object(FIXTURE, 'psql', side_effect=psql))
        stack.enter_context(mock.patch.object(FIXTURE, 'new_id', side_effect=new_id))
        stack.enter_context(mock.patch.object(FIXTURE.secrets, 'token_hex', side_effect=random_hex))
        stack.enter_context(mock.patch.object(FIXTURE.rsa, 'generate_private_key', side_effect=generate_private_key))
        stack.enter_context(mock.patch.dict(os.environ, {}, clear=False))
        stack.enter_context(mock.patch.object(sys, 'argv', ['phase23-fixture.py', '--task-dir', str(TASK), '--admin-url', ADMIN]))
        stack.enter_context(contextlib.redirect_stdout(output))
        try:
            FIXTURE.main()
        except Exception as error:
            trace['error'] = error
    trace['output'] = output.getvalue()
    return trace


class FixtureBoundaryTests(unittest.TestCase):
    def assert_rejected_without_effects(self, trace, code):
        self.assertIsInstance(trace['error'], RuntimeError)
        self.assertEqual(str(trace['error']), code)
        self.assertFalse(trace['writes'], 'private output happened before binding rejection')
        self.assertFalse(trace['mutations'], 'role/fixture SQL mutation happened before binding rejection')
        self.assertEqual(trace['rsa'], 0)
        self.assertEqual(trace['output'], '')

    def test_external_readable_owned_https_keyset_is_rejected_before_any_write_or_alter(self):
        manifest, environment, signing = baseline()
        outside = '/tmp/unrelated-project/private-signing.json'
        environment['identity']['OIDC_KEYSET_PATH'] = outside
        trace = exercise(manifest, environment, signing)
        self.assert_rejected_without_effects(trace, 'RECOVERY_PATH_OUTSIDE_TASK')
        self.assertNotIn(outside, trace['reads'])

    def test_each_runtime_wrong_database_path_is_rejected_before_any_write_or_alter(self):
        for kind in ('identity', 'platform', 'portal'):
            with self.subTest(kind=kind):
                manifest, environment, signing = baseline()
                parsed = urlsplit(environment[kind]['DATABASE_URL'])
                environment[kind]['DATABASE_URL'] = parsed._replace(path='/other_project').geturl()
                self.assert_rejected_without_effects(exercise(manifest, environment, signing),
                    'PHASE23_RUNTIME_ROLE_OR_DATABASE_INVALID')

    def test_every_runtime_endpoint_role_scheme_query_and_fragment_is_bound(self):
        for kind in ('identity', 'platform', 'portal'):
            for change in ('role', 'host', 'port', 'scheme', 'query', 'fragment'):
                with self.subTest(kind=kind, change=change):
                    manifest, environment, signing = baseline()
                    parsed = urlsplit(environment[kind]['DATABASE_URL'])
                    if change == 'role':
                        parsed = parsed._replace(netloc='another_runtime:mock@127.0.0.1:55494')
                    elif change == 'host':
                        parsed = parsed._replace(netloc=parsed.netloc.replace('127.0.0.1', '192.0.2.1'))
                    elif change == 'port':
                        parsed = parsed._replace(netloc=parsed.netloc.replace('55494', '5432'))
                    else:
                        parsed = parsed._replace(**{change: {'scheme': 'postgresql', 'query': 'options=other', 'fragment': 'other'}[change]})
                    environment[kind]['DATABASE_URL'] = parsed.geturl()
                    self.assert_rejected_without_effects(exercise(manifest, environment, signing),
                        'PHASE23_RUNTIME_ROLE_OR_DATABASE_INVALID')

    def test_portal_manifest_all_five_fields_and_exact_shape_are_bound_before_effects(self):
        for field in ('database', 'owner', 'runtime', 'marker', 'initializationPending', 'unexpected'):
            with self.subTest(field=field):
                manifest, environment, signing = baseline()
                manifest['portal'][field] = True if field == 'initializationPending' else 'unrelated'
                self.assert_rejected_without_effects(exercise(manifest, environment, signing), 'PHASE23_PORTAL_BINDING_INVALID')

    def test_both_real_marker_queries_must_match_the_owned_cluster_and_database(self):
        for field, code in (('shared_marker', 'PHASE23_DATABASE_MARKER_INVALID'),
                            ('portal_marker', 'PHASE23_PORTAL_MARKER_INVALID')):
            for value in ('foreign-cluster|foreign-marker', CLUSTER + '|foreign-marker'):
                with self.subTest(field=field, marker=value):
                    self.assert_rejected_without_effects(exercise(**{field: value}), code)

    def test_symlinked_key_parent_is_rejected_by_real_safe_path_before_effects(self):
        manifest, environment, signing = baseline()
        parent = TASK / 'mock-alias'
        environment['identity']['OIDC_KEYSET_PATH'] = str(parent / 'signing.json')
        self.assert_rejected_without_effects(exercise(manifest, environment, signing, symlink=parent), 'RECOVERY_SYMLINK_REJECTED')

    def test_initial_kid_shape_and_legacy_empty_checks_precede_all_effects(self):
        manifest, environment, signing = baseline()
        signing['keys'][0]['kid'] = 'another-key'
        self.assert_rejected_without_effects(exercise(manifest, environment, signing), 'PHASE23_INITIAL_SIGNING_MATERIAL_INVALID')
        self.assert_rejected_without_effects(exercise(legacy_present=True), 'PHASE23_LEGACY_STATE_PRESENT')

    def test_legal_fake_inputs_reach_ready_and_capture_only_owned_outputs(self):
        trace = exercise()
        self.assertIsNone(trace['error'])
        self.assertEqual(trace['output'], 'PHASE23_OWNED_FIXTURE_READY keys=4 runtime_passwords=3 legacy_state=empty\n')
        self.assertEqual(trace['rsa'], 3)
        self.assertEqual(len(trace['writes']), 9)
        self.assertEqual(len(trace['mutations']), 1)
        self.assertEqual(trace['mutations'][0][1].count('ALTER ROLE '), 3)
        self.assertTrue(all(Path(row['path']).is_relative_to(TASK) for row in trace['writes']))
        by_path = {row['path']: row for row in trace['writes']}
        signing = by_path[str(SIGNING)]
        self.assertTrue(signing['replace'])
        self.assertEqual([key['kid'] for key in signing['value']['keys']], ['owned-https', 'phase23-b', 'phase23-c', 'phase23-d'])
        environment = by_path[str(TASK / 'https-environment.json')]['value']
        self.assertNotIn('OIDC_CODE_TTL_SECONDS', environment['identity'])
        self.assertEqual(environment['platform']['MEMBERSHIP_LIFECYCLE_ENABLED'], 'true')
        self.assertEqual(environment['phase23']['signingKids'], {'a': 'owned-https', 'b': 'phase23-b', 'c': 'phase23-c', 'd': 'phase23-d'})
        marker_reads = [database for database, statement in trace['queries'] if 'FROM pg_control_system()' in statement]
        self.assertEqual(marker_reads, [ADMIN.rsplit('/', 1)[0] + '/' + NAME,
                                      ADMIN.rsplit('/', 1)[0] + '/unified_auth_portal_' + TOKEN])

    def test_organization_marker_uses_explicit_role_and_exact_application_tenant_context_caps(self):
        trace = exercise()
        self.assertIsNone(trace['error'])
        statement = trace['mutations'][0][1]
        scope = baseline()[1]['ids']
        self.assertIn('PHASE23_ORGANIZATION_MARKER_SCOPE_INVALID', statement)
        self.assertIn("app.code='https-console'", statement)
        self.assertIn("member.member_type='EMPLOYEE'", statement)
        self.assertIn("'EMPLOYEE'=ANY(app.allowed_member_types)", statement)
        self.assertIn('target.parent_id=selected.id', statement)
        self.assertIn('selected.parent_id IS NULL', statement)
        for alias, key in (('tenant', 'tenant'), ('selected', 'organization'), ('target', 'secondOrganization'),
                           ('app', 'application'), ('member', 'adminMember')):
            self.assertIn(alias + '.id=' + FIXTURE.literal(scope[key]) + '::uuid', statement)
        self.assertEqual(statement.count("'system:organization'"), 1)
        self.assertEqual(statement.count('INSERT INTO menus('), 1)
        self.assertEqual(statement.count('INSERT INTO tenant_menus('), 1)
        self.assertEqual(statement.count('INSERT INTO organization_menus('), 1)
        self.assertEqual(statement.count('INSERT INTO roles('), 1)
        self.assertEqual(statement.count('INSERT INTO role_menus('), 1)
        self.assertIn('INSERT INTO user_roles(user_id,role_id,organization_id) VALUES(' + FIXTURE.literal(scope['admin']), statement)
        self.assertNotIn('UPDATE users', statement)
        self.assertNotIn('is_super_admin', statement)
        self.assertNotIn('CREATE OR REPLACE FUNCTION', statement)
        self.assertNotIn('GRANT ALL', statement)

    def test_rpo_marker_is_an_owner_seeded_child_without_any_member(self):
        trace = exercise()
        self.assertIsNone(trace['error'])
        environment = next(row['value'] for row in trace['writes']
                           if row['path'] == str(TASK / 'https-environment.json'))
        scope = environment['ids']
        marker = environment['phase23']['ids']['markerOrganization']
        self.assertNotIn(marker, scope.values())
        self.assertNotEqual(marker, scope['organization'])
        self.assertNotEqual(marker, scope['secondOrganization'])
        seed = FIXTURE.organization_marker_seed(environment, marker)
        self.assertIn('INSERT INTO organizations(id,tenant_id,parent_id,name) VALUES('
                      + FIXTURE.literal(marker) + '::uuid,' + FIXTURE.literal(scope['tenant'])
                      + '::uuid,' + FIXTURE.literal(scope['organization']) + '::uuid,', seed[0])
        self.assertIn('PHASE23_MARKER_ORGANIZATION_NOT_ISOLATED', seed[1])
        self.assertIn('OR EXISTS(SELECT 1 FROM memberships WHERE organization_id='
                      + FIXTURE.literal(marker) + '::uuid)', seed[1])
        statement = trace['mutations'][0][1]
        self.assertTrue(all(fragment in statement for fragment in seed))
        for fragment in statement.split(';'):
            if 'INSERT INTO memberships(' in fragment:
                self.assertNotIn(marker, fragment)
        self.assertNotIn('INSERT INTO organization_applications', '\n'.join(seed))
        self.assertNotIn('GRANT ', '\n'.join(seed))
        self.assertLess(statement.index(seed[0]), statement.index("'system:organization'"))

    def test_rpo_marker_rejects_existing_root_child_and_malformed_identifiers(self):
        environment = baseline()[1]
        for value in [environment['ids']['organization'], environment['ids']['secondOrganization'],
                      environment['ids']['tenant'], None, 'untrusted-identifier', 1]:
            with self.subTest(marker=value):
                with self.assertRaisesRegex(RuntimeError,
                        '^PHASE23_MARKER_ORGANIZATION_BINDING_INVALID$'):
                    FIXTURE.organization_marker_seed(environment, value)

    def test_organization_marker_missing_or_foreign_identifier_shape_rejects_before_outputs(self):
        for name in ('tenant', 'organization', 'secondOrganization', 'application', 'admin', 'adminMember'):
            manifest, environment, signing = baseline()
            environment['ids'][name] = 'untrusted-identifier'
            with self.subTest(identifier=name):
                self.assert_rejected_without_effects(exercise(manifest, environment, signing),
                    'PHASE23_ORGANIZATION_MARKER_BINDING_INVALID')


if __name__ == '__main__':
    unittest.main()
