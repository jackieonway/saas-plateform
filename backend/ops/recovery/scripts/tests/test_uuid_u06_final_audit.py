"""Offline final-audit contracts and opt-in disposable PostgreSQL scan fixtures."""
import contextlib
import copy
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('u06_final_audit', Path(__file__).parents[1] / 'uuid-u06-final-audit.py')
A = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(A)
V7 = '019a1163-0000-7000-8000-000000000001'
V7B = '019a1163-0000-7000-8000-000000000002'
V4 = '019a1163-0000-4000-8000-000000000003'


def registration():
    groups = {}
    for number, group in enumerate(A.D.NAMES):
        groups[group] = [{'kind': kind, 'name': A.D.NAMES[group][i], 'owner': A.D.OWNERS[group][i], 'oid': 100 + number * 10 + i,
                          'owner_oid': 200 + number * 10 + i, 'expected_schema_sha256': 'a' * 64,
                          'semantic_schema_sha256': str(i + 1) * 64} for i, kind in enumerate(A.D.KINDS)]
    return {'format': A.D.REG_FORMAT, 'system_identifier': '123456789', 'groups': groups,
            'fenced_roles': {g: list(roles) for g, roles in A.D.RUNTIMES.items()}}


def sequence():
    return {'name': 'audit_id_seq', 'schema': 'public', 'owner': 'owner', 'owner_oid': 4, 'start': '1', 'increment': '1', 'min': '1',
            'max': '9223372036854775807', 'cache': '1', 'cycle': False, 'owned_schema': 'public', 'owned_table': 'audit', 'owned_column': 'id', 'dependency': 'a'}


def bindings():
    external = {'client_id': 'opaque-disabled', 'client_type': 'CONFIDENTIAL', 'status': 'DISABLED', 'redirect_uris': 'https://external.invalid/callback'}
    return {'external_client': external, 'external_client_sha256': A.D.value_sha(external),
            'journals': {g: {'roles': {name: 400 + i for i, name in enumerate(A.D.RUNTIMES[g])}} for g in ('final', 'recovery')}}


class AuditContracts(unittest.TestCase):
    def test_registration_rejects_cross_generation_oid_collision_and_unknown_fields(self):
        self.assertEqual(A.validate_registration(registration())['format'], A.D.REG_FORMAT)
        for variant in ('name', 'oid', 'owner_oid', 'hash', 'extra'):
            reg = registration()
            if variant == 'name':
                reg['groups']['recovery'][0]['name'] = reg['groups']['final'][0]['name']
            elif variant in ('oid', 'owner_oid'):
                reg['groups']['final'][0][variant] = reg['groups']['source'][0][variant]
            elif variant == 'hash':
                reg['groups']['final'][0]['semantic_schema_sha256'] = 'f' * 64
            else:
                reg['extra'] = 1
            with self.assertRaises(A.Invalid):
                A.validate_registration(reg)

    def test_uuid_requires_both_version_and_variant_for_every_nonnull_value(self):
        good = [{'counts': {'non_null': 10, 'v7': 10, 'rfc_variant': 10, 'invalid': 0}}]
        self.assertEqual(A.validate_uuid(good, 1)['v7_uuid_values'], 10)
        for key in ('v7', 'rfc_variant', 'invalid'):
            bad = copy.deepcopy(good)
            bad[0]['counts'][key] = 1
            with self.assertRaises(A.Invalid):
                A.validate_uuid(bad, 1)
        with self.assertRaises(A.Invalid):
            A.validate_uuid(good, 2)

    def test_composite_fk_uses_ordered_pairs_and_match_null_semantics(self):
        fk = {'schema': 'public', 'table': 'child', 'parent_schema': 'public', 'parent_table': 'parent', 'match': 's', 'validated': True,
              'columns': [{'child': 'foreign_a', 'parent': 'key_a'}, {'child': 'foreign_b', 'parent': 'key_b'}]}
        sql = A.orphan_sql(fk)
        self.assertIn('p."key_a"=c."foreign_a" AND p."key_b"=c."foreign_b"', sql)
        self.assertIn('c."foreign_a" IS NOT NULL AND c."foreign_b" IS NOT NULL', sql)
        self.assertIn('NOT EXISTS', sql)
        self.assertNotIn('NOT IN', sql)
        fk['match'] = 'f'
        self.assertIn('NOT(c."foreign_a" IS NULL AND c."foreign_b" IS NULL)', A.orphan_sql(fk))
        fk['validated'] = False
        with self.assertRaises(A.Invalid):
            A.orphan_sql(fk)

    def test_sequence_bounds_ownership_collision_and_exhaustion_are_checked(self):
        definition = sequence()
        good = {'last_value': '9007199254740993', 'is_called': True}
        self.assertTrue(A.validate_sequence(definition, good, 'owner', {'min': '1', 'max': '9007199254740993'}, 4)['legal'])
        for value, bounds, owner in (({'last_value': '0', 'is_called': True}, {'min': '1', 'max': '1'}, 'owner'),
                                     ({'last_value': '1', 'is_called': False}, {'min': '1', 'max': '1'}, 'owner'),
                                     ({'last_value': '9223372036854775807', 'is_called': True}, {'min': '1', 'max': '1'}, 'owner'),
                                     (good, {'min': '1', 'max': '9007199254740993'}, 'other-owner')):
            with self.assertRaises(A.Invalid):
                A.validate_sequence(definition, value, owner, bounds, 4)

    def run_fake_audit(self, wrong_external=False):
        reg = registration()
        db_lookup = {d['name']: d['kind'] for group in reg['groups'].values() for d in group}
        sessions = []
        class Session:
            def __init__(self, name, scratch, write=False):
                assert not write
                self.kind = db_lookup[name]
                self.closed = False
                sessions.append(self)
            def one(self, sql):
                if sql == A.FK_CATALOG:
                    return [{'schema': 'public', 'table': 'child', 'parent_schema': 'public', 'parent_table': 'parent', 'match': 's', 'validated': True,
                             'columns': [{'child': 'parent_id', 'parent': 'id'}]}] * A.EXPECTED[self.kind]['foreign_keys']
                if 'FROM scans;' in sql:
                    return [{'counts': {'non_null': 1, 'v7': 1, 'rfc_variant': 1, 'invalid': 0}}] * A.EXPECTED[self.kind]['uuid_columns']
                return 0
            def close(self):
                self.closed = True
            def abort(self):
                self.closed = True
        def inspect(session, db, cfg):
            return {'tables': A.D.TABLES[db['kind']]}
        epoch = bindings()
        baseline = {**epoch['external_client'], **({'client_id': 'wrong-local-client'} if wrong_external else {})}
        with patch.object(A.D, 'Session', Session), patch.object(A.D, 'inspect', side_effect=inspect), \
             patch.object(A.D, 'admission', side_effect=lambda cfg, group: {'group': group}) as admission, \
             patch.object(A, 'active_recovery_gate', return_value={'active': True}) as active, \
             patch.object(A, 'old_v4_gate', return_value={'old': True}), patch.object(A, 'sequence_audit', return_value=[]), \
             patch.object(A, 'recovery_acl_audit', return_value={'acl': True}), \
             patch.object(A, 'business_audit', side_effect=lambda session, kind: {'disabled_client': baseline} if kind == 'platform' else {}):
            result = A.run_audit({'reg': reg, 'fixture': False, 'work': '/unused/offline', 'epoch_bindings': epoch})
        return result, admission, active, sessions

    def test_active_recovery_never_uses_idle_admission_and_every_session_closes(self):
        result, admission, active, sessions = self.run_fake_audit()
        self.assertEqual(result['status'], 'PASS')
        self.assertEqual([call.args[1] for call in admission.call_args_list], ['source', 'source', 'final', 'final'])
        self.assertEqual(active.call_count, 2)
        self.assertEqual(len(sessions), 9)
        self.assertTrue(all(s.closed for s in sessions))
        for group in result['groups'].values():
            self.assertEqual(group['totals']['tables'], 54)
            self.assertEqual(group['totals']['uuid_columns'], 147)
            self.assertEqual(group['totals']['foreign_keys'], 67)

    def test_same_wrong_disabled_client_in_all_generations_cannot_replace_historical_fence(self):
        with self.assertRaisesRegex(A.Invalid, 'EXTERNAL_CLIENT_IDENTITY_OR_FENCE_CHANGED'):
            self.run_fake_audit(wrong_external=True)

    def test_existing_epoch_journals_bind_external_client_and_runtime_oids_without_db(self):
        reg, epoch, files = registration(), bindings(), {}
        helper_sha = 'f' * 64
        for generation in ('final', 'recovery'):
            directory = A.BASE / ('epoch-' + generation)
            receipt = {'format': 'saas-uuid-u06-target-receipts/1', 'generation': generation, 'targets': {
                db['kind']: {'database': db['name'], 'database_oid': db['oid'], 'owner': db['owner'], 'owner_oid': db['owner_oid'],
                             'schema_sha256': db['expected_schema_sha256']} for db in reg['groups'][generation]}}
            before = {'clients': [{**epoch['external_client'], 'client_secret_hash': 'private-secret-hash'}]
                      + [{'client_id': 'opaque-' + str(i), 'client_type': 'CONFIDENTIAL', 'status': 'ENABLED', 'redirect_uris': []} for i in range(11)]}
            prepared = {'format': 'saas-uuid-u06-epoch-journal/1', 'generation': generation, 'status': 'PREPARED', 'script_sha256': helper_sha,
                        'target_receipt_sha256': A.D.value_sha(receipt), 'before_platform_sha256': A.D.value_sha(before), 'epoch_sha256': 'a' * 64}
            activated = {'status': 'ACTIVE_EPOCH_ROLES_NOLOGIN', 'generation': generation, 'epoch_sha256': 'a' * 64,
                         'before_platform_sha256': prepared['before_platform_sha256'], 'roles': epoch['journals'][generation]['roles']}
            for name, value in (('prepared.json', prepared), ('target-receipt.json', receipt), ('activate/complete.json', activated),
                                ('before-platform.private.json', before), ('enable-login/complete.json', {'status': 'RUNTIME_LOGIN_ENABLED', 'generation': generation, 'roles': activated['roles']})):
                files[directory / name] = A.D.canonical(value) + b'\n'
        with patch.object(A, 'read_private', side_effect=lambda path: files[path]), patch.object(A.D, 'sha', return_value=helper_sha), \
             patch.object(A.D, 'Session', side_effect=AssertionError('offline binding opened DB')):
            result = A.load_epoch_bindings(reg)
            self.assertEqual(result['external_client'], epoch['external_client'])
            self.assertNotIn('private-secret-hash', repr(result))
            files[A.BASE / 'epoch-final/before-platform.private.json'] += b' '
            self.assertEqual(A.load_epoch_bindings(reg)['external_client_sha256'], epoch['external_client_sha256'])
            changed = A.D.parsed(files[A.BASE / 'epoch-final/before-platform.private.json'])
            changed['clients'][0]['client_id'] = 'wrong-local-client'
            files[A.BASE / 'epoch-final/before-platform.private.json'] = A.D.canonical(changed)
            with self.assertRaisesRegex(A.Invalid, 'HISTORICAL_PLATFORM_BEFORE_IMAGE_CHANGED'):
                A.load_epoch_bindings(reg)

    def test_active_roles_are_oid_bound_and_all_other_login_bypasses_fail_closed(self):
        databases = (*A.OLD_DATABASES, *(name for group in A.D.NAMES.values() for name in group))
        expected = bindings()['journals']['recovery']['roles']
        roles = [{'name': 'postgres', 'oid': 10, 'login': True, 'superuser': True, 'createrole': True, 'safe': False, 'memberships': 0,
                  'privileged_set': True, 'admin_protected': False, 'connect': dict.fromkeys(databases, True)}]
        for i, name in enumerate(A.D.RUNTIMES['recovery']):
            own = A.D.NAMES['recovery'][i if i < 3 else 0]
            roles.append({'name': name, 'oid': expected[name], 'login': True, 'superuser': False, 'createrole': False, 'safe': True, 'memberships': 0,
                          'privileged_set': True, 'admin_protected': False, 'connect': {db: db == own for db in databases}})
        good = {'roles': roles, 'public_admission': dict.fromkeys(databases, False)}
        A.validate_recovery_roles(good, expected, databases)
        for problem in ('oid', 'membership', 'cross_database', 'missing_database', 'public', 'other_connect', 'other_set', 'other_admin', 'other_superuser', 'other_createrole'):
            proof = copy.deepcopy(good)
            if problem == 'oid':
                proof['roles'][1]['oid'] += 1
            elif problem == 'membership':
                proof['roles'][1]['memberships'] = 1
            elif problem == 'cross_database':
                proof['roles'][1]['connect'][A.OLD_DATABASES[0]] = True
            elif problem == 'missing_database':
                del proof['roles'][1]['connect'][A.OLD_DATABASES[0]]
            elif problem == 'public':
                proof['public_admission'][A.OLD_DATABASES[0]] = True
            else:
                other = {'name': 'unexpected-login', 'login': True, 'superuser': False, 'createrole': False, 'safe': True, 'privileged_set': False,
                         'admin_protected': False, 'connect': dict.fromkeys(databases, False)}
                if problem == 'other_connect':
                    other['connect'][A.OLD_DATABASES[0]] = True
                elif problem in ('other_set', 'other_admin'):
                    other['privileged_set' if problem == 'other_set' else 'admin_protected'] = True
                else:
                    other[problem.removeprefix('other_')] = True
                proof['roles'].append(other)
            with self.assertRaises(A.Invalid, msg=problem):
                A.validate_recovery_roles(proof, expected, databases)

    def test_createdb_only_unrelated_role_has_no_existing_database_write_or_escalation_path(self):
        roles = [{'name': 'postgres', 'login': True, 'superuser': True, 'createrole': True},
                 {'name': 'isolated-fixture-creator', 'login': True, 'superuser': False, 'createrole': False,
                  'safe': False, 'privileged_set': False, 'admin_protected': False, 'connect': {'protected': False}}]
        A.deny_other_logins(roles)
        for key in ('privileged_set', 'admin_protected', 'superuser', 'createrole'):
            bad = copy.deepcopy(roles)
            bad[1][key] = True
            with self.assertRaises(A.Invalid):
                A.deny_other_logins(bad)

    def test_business_fences_reject_additional_disabled_clients_or_publish_grants(self):
        class Session:
            def __init__(self, disabled=1, grants=0):
                self.disabled, self.grants = disabled, grants
            def one(self, sql):
                if 'FROM public.oauth_clients' in sql:
                    return [{'client_id': str(i), 'client_type': 'CONFIDENTIAL', 'status': 'DISABLED' if i < self.disabled else 'ENABLED', 'redirect_uris': 'https://external.invalid'} for i in range(12)]
                if 'resource_code=' in sql:
                    return self.grants
                return {table: 0 for table in A.D.TABLES['platform']}
        self.assertEqual(A.business_audit(Session(), 'platform')['notification_publish_grants'], 0)
        for session in (Session(disabled=2), Session(grants=1)):
            with self.assertRaises(A.Invalid):
                A.business_audit(session, 'platform')


@unittest.skipUnless(os.environ.get('UUID_U06_FINAL_AUDIT_PG_FIXTURE') == 'isolated-offline-v1', 'explicit disposable offline audit fixture required')
class ActualAuditSql(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location('audit_container_fixture', Path(__file__).with_name('test_uuid_u06_epoch_pg.py'))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        self.fixture = module.ActualPostgresContract()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.sql = self.fixture.sql

    def test_recursive_uuid_composite_orphans_and_owned_sequences(self):
        self.sql('postgres', "CREATE TABLE public.parent(a uuid,b integer,ordinal bigserial,PRIMARY KEY(a,b)); CREATE TABLE public.child(id uuid PRIMARY KEY,parent_uuid uuid,parent_number integer,FOREIGN KEY(parent_uuid,parent_number) REFERENCES public.parent(a,b)); CREATE DOMAIN public.uuid_array AS uuid[]; CREATE TABLE public.arrays(id uuid PRIMARY KEY,values public.uuid_array);", True)
        self.sql('postgres', "INSERT INTO public.parent(a,b) VALUES('" + V7 + "',7); INSERT INTO public.child VALUES('" + V7B + "','" + V7 + "',7); INSERT INTO public.child VALUES('" + V7 + "',NULL,7); INSERT INTO public.arrays VALUES('" + V7 + "',ARRAY['" + V7 + "'::uuid,NULL,'" + V7B + "'::uuid]);", True)
        fixture = self
        class Session:
            def one(self, sql):
                return fixture.sql('postgres', sql)
        session = Session()
        scans = session.one(A.uuid_sql())
        self.assertEqual(A.validate_uuid(scans, 5)['non_null_uuid_values'], 7)
        self.assertTrue(any(s['array_layers'] == 1 and s['domains'] for s in scans))
        keys = session.one(A.FK_CATALOG)
        self.assertEqual(len(keys), 1)
        self.assertEqual(session.one(A.orphan_sql(keys[0])), 0)
        sequences = A.sequence_audit(session, 'postgres')
        self.assertEqual(len(sequences), 1)
        self.assertEqual(sequences[0]['owned_column'], 'ordinal')
        self.sql('postgres', "SET session_replication_role=replica; INSERT INTO public.child VALUES('" + V4 + "','" + V7 + "',999); SET session_replication_role=origin; INSERT INTO public.arrays VALUES('" + V7B + "',ARRAY['" + V4 + "'::uuid]);", True)
        self.assertEqual(session.one(A.orphan_sql(keys[0])), 1)
        with self.assertRaises(A.Invalid):
            A.validate_uuid(session.one(A.uuid_sql()), 5)

    def test_live_acl_queries_reject_column_sequence_and_original_database_login_bypasses(self):
        runtime = dict(zip((*A.D.KINDS, 'notification'), A.D.RUNTIMES['recovery']))
        for name in runtime.values():
            self.sql('postgres', 'CREATE ROLE ' + A.D.ident(name) + ' LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS;', True)
        tables = ['_sqlx_migrations', 'business']
        self.sql('postgres', 'CREATE TABLE public._sqlx_migrations(id integer); CREATE TABLE public.business(id bigserial); REVOKE ALL ON DATABASE postgres FROM PUBLIC; REVOKE CREATE ON SCHEMA public FROM PUBLIC;', True)
        store = A.D.ident(runtime['stores'])
        self.sql('postgres', 'GRANT CONNECT ON DATABASE postgres TO ' + store + '; GRANT USAGE ON SCHEMA public TO ' + store
                 + '; GRANT SELECT ON ALL TABLES IN SCHEMA public TO ' + store + '; GRANT INSERT,UPDATE,DELETE ON public.business TO ' + store
                 + '; GRANT USAGE,SELECT ON ALL SEQUENCES IN SCHEMA public TO ' + store + '; ALTER ROLE ' + store + ' IN DATABASE postgres SET default_transaction_read_only=off;', True)
        expected = self.sql('postgres', "SELECT jsonb_object_agg(rolname,oid::bigint) FROM pg_roles WHERE rolname IN (" + ','.join(A.D.literal(r) for r in runtime.values()) + ');')
        fixture = self
        class Session:
            def __init__(self, *args, **kwargs):
                pass
            def one(self, sql):
                return fixture.sql('postgres', sql)
            def close(self):
                pass
            def abort(self):
                pass
        session = Session()
        with patch.dict(A.D.TABLES, {'stores': tables}):
            self.assertEqual(len(A.recovery_acl_audit(session, 'stores', expected)), 4)
            forbidden = [('GRANT SELECT(id) ON public.business TO ' + A.D.ident(runtime['notification']), 'REVOKE SELECT(id) ON public.business FROM ' + A.D.ident(runtime['notification'])),
                         ('GRANT UPDATE ON SEQUENCE public.business_id_seq TO ' + store, 'REVOKE UPDATE ON SEQUENCE public.business_id_seq FROM ' + store),
                         ('GRANT UPDATE(id) ON public._sqlx_migrations TO ' + store, 'REVOKE UPDATE(id) ON public._sqlx_migrations FROM ' + store),
                         ('GRANT SELECT ON public.business TO ' + store + ' WITH GRANT OPTION', 'REVOKE GRANT OPTION FOR SELECT ON public.business FROM ' + store),
                         ('GRANT MAINTAIN ON public.business TO ' + store, 'REVOKE MAINTAIN ON public.business FROM ' + store)]
            for grant, revoke in forbidden:
                self.sql('postgres', grant + ';', True)
                with self.assertRaises(A.Invalid, msg=grant):
                    A.recovery_acl_audit(session, 'stores', expected)
                self.sql('postgres', revoke + ';', True)
            A.recovery_acl_audit(session, 'stores', expected)
        for name in A.OLD_ROLES:
            self.sql('postgres', 'CREATE ROLE ' + A.D.ident(name) + ' NOLOGIN;', True)
        for name in A.OLD_DATABASES:
            self.sql('postgres', 'CREATE DATABASE ' + A.D.ident(name) + ' OWNER ' + A.D.ident(A.OLD_ROLES[0])
                     + '; REVOKE ALL ON DATABASE ' + A.D.ident(name) + ' FROM PUBLIC; ALTER DATABASE ' + A.D.ident(name) + ' SET default_transaction_read_only=on;', True)
        cfg = {'reg': {'system_identifier': self.sql('postgres', 'SELECT to_json(system_identifier::text) FROM pg_control_system();')}, 'work': '/unused/offline'}
        with patch.object(A.D, 'Session', Session):
            A.old_v4_gate(cfg)
            self.sql('postgres', 'CREATE ROLE unexpected_audit_login LOGIN CREATEDB;', True)
            A.old_v4_gate(cfg)
            self.sql('postgres', 'GRANT CONNECT ON DATABASE ' + A.D.ident(A.OLD_DATABASES[0]) + ' TO unexpected_audit_login;', True)
            with self.assertRaisesRegex(A.Invalid, 'OTHER_LOGIN_CAN_WRITE_OR_ESCALATE'):
                A.old_v4_gate(cfg)
            self.sql('postgres', 'REVOKE CONNECT ON DATABASE ' + A.D.ident(A.OLD_DATABASES[0]) + ' FROM unexpected_audit_login; GRANT ' + A.D.ident(A.OLD_ROLES[0]) + ' TO unexpected_audit_login WITH SET FALSE;', True)
            with self.assertRaisesRegex(A.Invalid, 'OTHER_LOGIN_CAN_WRITE_OR_ESCALATE'):
                A.old_v4_gate(cfg)
            self.sql('postgres', 'REVOKE ' + A.D.ident(A.OLD_ROLES[0]) + ' FROM unexpected_audit_login;', True)
            A.old_v4_gate(cfg)


if __name__ == '__main__':
    unittest.main()
