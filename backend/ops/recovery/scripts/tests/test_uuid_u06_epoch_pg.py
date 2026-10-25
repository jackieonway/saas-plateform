"""Opt-in, disposable PG17 SQL/ACL regression; no host sockets or service ports."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
import unittest

SPEC = importlib.util.spec_from_file_location('u06_epoch_pg', Path(__file__).parents[1] / 'prepare-uuid-u06-epoch.py')
E = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(E)
FIXTURE = importlib.util.spec_from_file_location('u06_epoch_pure', Path(__file__).with_name('test_prepare_uuid_u06_epoch.py'))
F = importlib.util.module_from_spec(FIXTURE)
FIXTURE.loader.exec_module(F)
IMAGE = 'postgres:17.6-bookworm'


@unittest.skipUnless(os.environ.get('UUID_U06_EPOCH_PG_FIXTURE') == 'isolated-offline-v1', 'explicit disposable offline fixture required')
class ActualPostgresContract(unittest.TestCase):
    def setUp(self):
        self.container = None
        self.temporary = tempfile.TemporaryDirectory(prefix='u06-epoch-pg-', dir='/dev/shm')
        self.addCleanup(self.temporary.cleanup)
        self.private = Path(self.temporary.name)
        self.private.chmod(0o700)
        self.sequence = 0
        self.call(['docker', 'image', 'inspect', IMAGE])  # Never pull or use a remote daemon.
        self.container = self.call(['docker', '--host', 'unix:///run/docker.sock', 'run', '--rm', '-d', '--network', 'none',
            '--label', 'saas.uuid-u06-epoch-fixture=isolated-offline-v1', '--tmpfs', '/var/lib/postgresql/data:rw',
            '-e', 'POSTGRES_HOST_AUTH_METHOD=trust', IMAGE, 'postgres', '-c', 'cluster_name=uuid-u06-epoch-offline-fixture']).decode().strip()
        self.assertRegex(self.container, r'\A[0-9a-f]{64}\Z')
        self.addCleanup(lambda: self.call(['docker', '--host', 'unix:///run/docker.sock', 'rm', '--force', self.container], checked=False))
        for _ in range(100):
            initialized = b'PostgreSQL init process complete' in self.call(['docker', '--host', 'unix:///run/docker.sock', 'logs', self.container])
            ready = self.call(['docker', '--host', 'unix:///run/docker.sock', 'exec', self.container, 'pg_isready', '-q', '-U', 'postgres'], checked=False, result=True)
            if initialized and ready.returncode == 0:
                break
            time.sleep(0.1)
        else:
            self.fail('isolated fixture startup failed; private diagnostics retained')

    def call(self, argv, sql=None, checked=True, result=False):
        require = E.require
        # Every Docker operation is fixed to the local socket. image inspection is
        # also explicitly local; an inherited DOCKER_HOST cannot select a remote.
        if argv[:3] == ['docker', 'image', 'inspect']:
            argv = ['docker', '--host', 'unix:///run/docker.sock', *argv[1:]]
        value = subprocess.run(argv, input=sql.encode() if sql is not None else None, capture_output=True,
                               timeout=60, env={'PATH': '/usr/sbin:/usr/bin:/sbin:/bin', 'LANG': 'C.UTF-8'}, check=False)
        if checked and value.returncode:
            self.sequence += 1
            E.write_new(self.private / f'failure-{self.sequence}.stderr', value.stderr)
            require(False, 'ISOLATED_PG_FIXTURE_COMMAND_FAILED')
        return value if result else value.stdout

    def sql(self, database, query, write=False):
        argv = ['docker', '--host', 'unix:///run/docker.sock', 'exec', '-i', self.container, 'env', '-i',
                'PATH=/usr/lib/postgresql/17/bin:/usr/bin:/bin', 'PGOPTIONS=-c default_transaction_read_only=' + ('off' if write else 'on'),
                'psql', '-X', '-qAt', '--no-password', '-v', 'ON_ERROR_STOP=1', '-U', 'postgres', '-d', database, '--file=-']
        raw = self.call(argv, query)
        lines = [line for line in raw.splitlines() if line]
        return E.parsed(lines[-1]) if lines else None

    def test_real_rotation_guard_acl_and_before_image_rollback(self):
        for generation in ('final', 'recovery'):
            for definition in E.names(generation).values():
                self.sql('postgres', 'CREATE ROLE ' + E.ident(definition['owner']) + ' NOLOGIN;\nCREATE DATABASE ' + E.ident(definition['database']) + ' OWNER ' + E.ident(definition['owner']) + ';\nREVOKE ALL ON DATABASE ' + E.ident(definition['database']) + ' FROM PUBLIC;\nALTER DATABASE ' + E.ident(definition['database']) + ' SET default_transaction_read_only=on;', True)
        for database in E.protected_databases():
            if '_uuid7_final_' in database or '_uuid7_recovery_' in database:
                continue
            self.sql('postgres', 'CREATE DATABASE ' + E.ident(database) + '; REVOKE ALL ON DATABASE ' + E.ident(database) + ' FROM PUBLIC;', True)
        fixed = E.names('final')
        platform = fixed['platform']['database']
        tables = {
            'platform': ['_sqlx_migrations', 'oauth_clients', 'users', 'applications', 'notifications', *E.CREDENTIALS],
            'stores': [*E.METADATA['stores'], 'stores'],
            'work_orders': [*E.METADATA['work_orders'], 'work_orders'],
        }
        self.sql(platform, 'CREATE TABLE public.oauth_clients(client_id text PRIMARY KEY,client_type text,client_secret_hash text,status text,redirect_uris text,description text,application_id text);\nCREATE TABLE public.users(id text PRIMARY KEY,session_version bigint,password_hash text,updated_at text);\nCREATE TABLE public.applications(id text,code text);', True)
        for kind, names in tables.items():
            existing = {'oauth_clients', 'users', 'applications'} if kind == 'platform' else set()
            for table in names:
                if table not in existing:
                    self.sql(fixed[kind]['database'], 'CREATE TABLE public.' + E.ident(table) + '(id integer);', True)
            self.sql(fixed[kind]['database'], 'CREATE SEQUENCE public.synthetic_counter; REVOKE CREATE ON SCHEMA public FROM PUBLIC;', True)
        state, source = F.state_source()
        for table, rows in (('oauth_clients', state['clients']), ('users', state['users']), ('applications', state['applications'])):
            self.sql(platform, 'INSERT INTO public.' + E.ident(table) + ' SELECT * FROM jsonb_populate_recordset(NULL::public.' + E.ident(table) + ',' + E.literal(E.canonical(rows).decode()) + '::jsonb);', True)
        initial = E.platform_state(self, platform)
        material_schema = self.sql(platform, E.SCHEMA_SQL['platform'].read_text(encoding='utf-8'))
        self.sql(platform, 'BEGIN;\n' + E.schema_guard('platform', material_schema) + "\nSELECT jsonb_build_object('schema_guard',true); COMMIT;", True)
        material = E.generate_material(initial, source, 'final')
        E.cluster_proof(self, 'final')
        self.sql('postgres', 'BEGIN;\n' + E.create_roles_sql('final', material['passwords']) + "\nSELECT jsonb_build_object('created',4); COMMIT;", True)
        for kind in E.KINDS:
            target = F.receipt('final')['targets'][kind]
            actual = self.sql(fixed[kind]['database'], 'SELECT jsonb_build_object(\'database_oid\',d.oid::bigint,\'owner_oid\',d.datdba::bigint) FROM pg_database d WHERE datname=current_database();')
            target.update(actual)
            self.sql('postgres', 'COMMENT ON DATABASE ' + E.ident(target['database']) + ' IS ' + E.literal(target['import_marker']) + ';', True)
            query = 'BEGIN; SET LOCAL standard_conforming_strings=on;\n' + E.mutation_guard(target) + '\n'
            if kind == 'platform':
                query += E.rotate_sql(initial, material['hashes']) + '\n'
            query += E.grants_sql(kind, 'final', tables[kind]) + "\nSELECT jsonb_build_object('committed',true); COMMIT;"
            self.sql(target['database'], query, True)
            E.privilege_proof(self, kind, 'final', tables[kind])
        after = E.platform_state(self, platform)
        self.assertEqual(after, E.rotate_expected(initial, material['hashes']))
        proof = E.cluster_proof(self, 'final', {}, False)
        self.assertEqual(len(proof['roles']), 4)
        self.assertTrue(all(not role['login'] for role in proof['roles']))
        # The original before-image is stale. A second transaction must roll back
        # before any user version or secret can be modified again.
        argv = ['docker', '--host', 'unix:///run/docker.sock', 'exec', '-i', self.container, 'env', '-i',
                'PATH=/usr/lib/postgresql/17/bin:/usr/bin:/bin', 'PGOPTIONS=-c default_transaction_read_only=off',
                'psql', '-X', '-qAt', '-v', 'ON_ERROR_STOP=1', '-U', 'postgres', '-d', platform, '--file=-']
        failed = self.call(argv, 'BEGIN;\n' + E.rotate_sql(initial, material['hashes']) + '\nCOMMIT;', checked=False, result=True)
        self.assertNotEqual(failed.returncode, 0)
        self.assertEqual(E.platform_state(self, platform), after)
        role_oids = {role['name']: role['oid'] for role in proof['roles']}
        self.sql('postgres', E.admission_sql(role_oids, True), True)
        E.cluster_proof(self, 'final', role_oids, True)
        self.sql('postgres', E.admission_sql(role_oids, False), True)
        E.cluster_proof(self, 'final', role_oids, False)


if __name__ == '__main__':
    unittest.main()
