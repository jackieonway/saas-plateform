"""Actual task-PG proof for read-only runtime admission; always owns tiny DB.

No application/production database is used. Passwords exist only in this
process and the runtime psql environment. CLI diagnostics are fixed categories.
"""
import argparse
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import time
import uuid

import phase23_cluster_binding as binding

TASK = Path('/tmp/rust-saas-unified-auth-20261028')
PSQL = '/usr/lib/postgresql/18/bin/psql'


def need(value, label):
    if not value:
        raise RuntimeError('PHASE25_DDL_PROBE_' + label)


def literal(value):
    return "'" + str(value).replace("'", "''") + "'"


def execute(sql, *, database='postgres', username='postgres', password=None, check=True):
    env = {'PATH': '/usr/lib/postgresql/18/bin:/usr/bin:/bin', 'PGHOST': '127.0.0.1', 'PGPORT': '55494',
        'PGUSER': username, 'PGDATABASE': database, 'PGSSLMODE': 'prefer',
        'PGOPTIONS': (('-c log_statement=none -c log_min_error_statement=panic ' if username == 'postgres' else '')
                      + '-c statement_timeout=5000 -c lock_timeout=3000')}
    if password:
        env['PGPASSWORD'] = password
    result = subprocess.run([PSQL, '-X', '-qAt', '-v', 'ON_ERROR_STOP=1', '-v', 'VERBOSITY=sqlstate'],
        input=sql.encode(), env=env, capture_output=True, timeout=12)
    if check:
        if result.returncode != 0:
            states = re.findall(rb'(?:ERROR|FATAL):\s+([0-9A-Z]{5})', result.stderr)
            raise RuntimeError('PHASE25_DDL_PROBE_SQL_FAILED_' + (states[-1].decode() if states else 'UNKNOWN'))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    need(args.output.parent.resolve() == Path('/mnt/d/WorkSpace/rust/rust-saas/saas-plateform/backend/docs/designs/unified-auth/validation')
         and re.fullmatch(r'phase-25-readonly-startup-[a-z0-9-]+\.json', args.output.name)
         and not args.output.exists() and not args.output.is_symlink(), 'OUTPUT_REJECTED')
    owned = binding.load_binding(TASK)
    need(os.getuid() == owned['taskUid'], 'UNPRIVILEGED_OWNER_REQUIRED')
    token = uuid.uuid4().hex
    database = 'p25ddl_' + token
    owner, runtime = 'p25ddl_' + token[-16:] + '_owner', 'p25ddl_' + token[-16:] + '_runtime'
    marker = 'unified-auth-phase25-readonly-startup/1/' + token
    password = secrets.token_hex(32)
    guard = "DO $$ BEGIN IF NOT (" + binding.sql_guard(owned) + ") THEN RAISE EXCEPTION 'PROBE_CLUSTER'; END IF; END $$;\n"
    created_roles = created_database = False
    report = {'contractVersion': 'unified-auth-phase25-readonly-startup-probe/1', 'result': 'FAIL',
        'startedAtEpoch': time.time(), 'evidenceLayer': 'ACTUAL_POSTGRESQL_LOGIN_ROLE',
        'databasePrefix': 'p25ddl_', 'dedicatedDatabasesCreated': 0, 'dedicatedRolesCreated': 0,
        'dockerContainersCreated': 0, 'dockerContainersReused': 0,
        'formalOrSharedApplicationDatabasesModified': False}
    try:
        report['stage'] = 'FRESH_NAMES'
        empty = execute(guard + 'SELECT NOT EXISTS(SELECT 1 FROM pg_database WHERE datname=' + literal(database)
            + ') AND NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname IN(' + literal(owner) + ',' + literal(runtime) + '));')
        need(empty.stdout.strip() == b't', 'FRESH_NAMES_REQUIRED')
        report['stage'] = 'CREATE_ROLES'
        execute(guard + 'BEGIN; CREATE ROLE "' + owner + '" NOLOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE; '
            + 'COMMENT ON ROLE "' + owner + '" IS ' + literal(marker) + '; CREATE ROLE "' + runtime
            + '" LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD ' + literal(password)
            + '; COMMENT ON ROLE "' + runtime + '" IS ' + literal(marker) + '; COMMIT;')
        created_roles = True
        report['dedicatedRolesCreated'] = 2
        report['stage'] = 'CREATE_DATABASE'
        execute(guard + 'CREATE DATABASE "' + database + '" OWNER "' + owner + '";')
        created_database = True
        report['dedicatedDatabasesCreated'] = 1
        report['stage'] = 'DATABASE_MARKER'
        execute(guard + 'COMMENT ON DATABASE "' + database + '" IS ' + literal(marker) + ';')
        report['stage'] = 'PREPARE_HISTORY_AND_FENCE'
        execute(guard + 'BEGIN; ALTER SCHEMA public OWNER TO "' + owner + '"; REVOKE ALL ON DATABASE "'
            + database + '" FROM PUBLIC; REVOKE ALL ON SCHEMA public FROM PUBLIC; GRANT CONNECT ON DATABASE "'
            + database + '" TO "' + runtime + '"; GRANT USAGE ON SCHEMA public TO "' + runtime
            + '"; SET LOCAL ROLE "' + owner + '"; CREATE TABLE public._sqlx_migrations '
            + '(version BIGINT PRIMARY KEY,description TEXT NOT NULL,installed_on TIMESTAMPTZ NOT NULL DEFAULT now(),'
            + 'success BOOLEAN NOT NULL,checksum BYTEA NOT NULL,execution_time BIGINT NOT NULL); RESET ROLE; '
            + 'GRANT SELECT ON public._sqlx_migrations TO "' + runtime + '"; COMMIT;', database=database)
        report['stage'] = 'RESTRICTED_LOGIN'
        login = execute("SELECT current_user=" + literal(runtime) + ' AND current_database()=' + literal(database)
            + " AND NOT has_schema_privilege(current_user,'public','CREATE') AND NOT has_database_privilege(current_user,"
            + literal(database) + ",'CREATE,TEMP') AND NOT pg_has_role(current_user," + literal(owner)
            + ",'MEMBER') AND has_table_privilege(current_user,'public._sqlx_migrations','SELECT') "
            + "AND NOT has_table_privilege(current_user,'public._sqlx_migrations','INSERT,UPDATE,DELETE');",
            database=database, username=runtime, password=password)
        need(login.stdout.strip() == b't', 'ROLE_FENCE_FAILED')
        report['actualRestrictedLoginVerified'] = True
        report['stage'] = 'READ_HISTORY'
        report['existingHistoryReadStatus'] = execute('SELECT count(*) FROM public._sqlx_migrations;',
            database=database, username=runtime, password=password).stdout.strip() == b'0'
        report['stage'] = 'ENSURE_MIGRATIONS_TABLE'
        attempted = execute("SET search_path=public,pg_catalog,pg_temp; CREATE TABLE IF NOT EXISTS _sqlx_migrations "
            '(version BIGINT PRIMARY KEY,description TEXT NOT NULL,installed_on TIMESTAMPTZ NOT NULL DEFAULT now(),'
            'success BOOLEAN NOT NULL,checksum BYTEA NOT NULL,execution_time BIGINT NOT NULL);',
            database=database, username=runtime, password=password, check=False)
        matches = re.findall(rb'ERROR:\s+([0-9A-Z]{5})', attempted.stderr)
        report['createIfNotExistsExistingHistoryExitCode'] = attempted.returncode
        report['createIfNotExistsExistingHistorySqlstates'] = [value.decode() for value in matches]
        report['schemaCreatePermissionRefused'] = attempted.returncode != 0 and b'42501' in matches
        report['existingHistoryTableRemainsReadable'] = execute('SELECT count(*) FROM public._sqlx_migrations;',
            database=database, username=runtime, password=password).stdout.strip() == b'0'
        report['result'] = 'PASS'
    except Exception as error:
        report['failureCode'] = str(error) if re.fullmatch(r'PHASE25_DDL_PROBE_[A-Z0-9_]+', str(error)) else 'PHASE25_DDL_PROBE_FAILED'
    finally:
        failures = []
        if created_database:
            try:
                actual = execute(guard + "SELECT pg_get_userbyid(datdba)||'|'||COALESCE(shobj_description(oid,'pg_database'),'') "
                    'FROM pg_database WHERE datname=' + literal(database) + ';').stdout.strip().decode()
                need(actual == owner + '|' + marker, 'CLEANUP_DATABASE_BINDING_CHANGED')
                execute(guard + 'SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname=' + literal(database) + ';')
                execute(guard + 'DROP DATABASE "' + database + '";')
            except Exception:
                failures.append('OWNED_DATABASE_CLEANUP_FAILED')
        if created_roles:
            for role in (runtime, owner):
                try:
                    actual = execute(guard + "SELECT COALESCE(shobj_description(oid,'pg_authid'),'') FROM pg_roles WHERE rolname="
                        + literal(role) + ';').stdout.strip().decode()
                    need(actual == marker, 'CLEANUP_ROLE_BINDING_CHANGED')
                    execute(guard + 'DROP ROLE "' + role + '";')
                except Exception:
                    failures.append('OWNED_ROLE_CLEANUP_FAILED')
        try:
            remaining = execute(guard + "SELECT json_build_object('databases', (SELECT count(*) FROM pg_database WHERE datname="
                + literal(database) + "),'roles',(SELECT count(*) FROM pg_roles WHERE rolname IN("
                + literal(owner) + ',' + literal(runtime) + ')))::text;')
            report['cleanup'] = {**json.loads(remaining.stdout), 'failures': failures}
        except Exception:
            report['cleanup'] = {'failures': [*failures, 'OWNED_RESOURCE_AUDIT_FAILED']}
        if failures or report['cleanup'].get('databases', -1) or report['cleanup'].get('roles', -1):
            report['result'] = 'FAIL'
        report['finishedAtEpoch'] = time.time()
        with args.output.open('x', encoding='utf-8') as output:
            json.dump(report, output, indent=2)
            output.write('\n')
    print(json.dumps({'result': report['result'], 'schemaCreatePermissionRefused': report.get('schemaCreatePermissionRefused'),
        'sqlstates': report.get('createIfNotExistsExistingHistorySqlstates'), 'cleanup': report['cleanup']}))
    return 0 if report['result'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
