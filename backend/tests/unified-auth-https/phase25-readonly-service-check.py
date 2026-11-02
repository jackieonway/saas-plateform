"""Real restricted-LOGIN startup checks for current stores/work-order binaries.

Owns one fresh database and two marked roles, closes every child in finally and
then removes only that exact fixture. No Docker or other application DB use.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import signal
import socket
import subprocess
import time
import urllib.error
import urllib.request
import uuid

import phase23_cluster_binding as binding

spec = importlib.util.spec_from_file_location('phase25_owned_ddl_probe', Path(__file__).with_name('phase25-readonly-startup-probe.py'))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)
TASK, execute, literal, need = probe.TASK, probe.execute, probe.literal, probe.need


def digest(path):
    result = hashlib.sha256()
    with path.open('rb') as stream:
        while data := stream.read(1024 * 1024):
            result.update(data)
    return result.hexdigest()


def source_inputs(root):
    paths = [root / 'Cargo.toml', root / 'Cargo.lock']
    for name in ('src', 'migrations', 'vendor'):
        paths.extend(path for path in (root / name).rglob('*') if path.is_file() and path.suffix in ('.rs', '.sql', '.toml', '.lock'))
    return {path.relative_to(root).as_posix(): digest(path) for path in sorted(set(paths))}


def run_service(binary, kind, database, runtime, password, *, ready):
    prefix = 'STORES' if kind == 'stores' else 'WORK_ORDERS'
    route = '/stores-api' if kind == 'stores' else '/work-orders-api'
    with socket.socket() as reserved:
        reserved.bind(('127.0.0.1', 0))
        port = reserved.getsockname()[1]
    secret = secrets.token_hex(32)
    env = {'PATH': '/usr/bin:/bin', 'NODE_ENV': 'test', prefix + '_DATABASE_URL':
        'postgresql://' + runtime + ':' + password + '@127.0.0.1:55494/' + database,
        prefix + '_HOST': '127.0.0.1', prefix + '_PORT': str(port),
        prefix + '_ALLOWED_ORIGINS': 'http://127.0.0.1:3000', prefix + '_STARTUP_TIMEOUT_MS': '5000',
        'PLATFORM_API_ORIGIN': 'http://127.0.0.1:9', 'PLATFORM_CLIENT_ID': 'owned-startup-probe',
        'PLATFORM_CLIENT_SECRET': secret,
        'PLATFORM_AUTHORIZATION_MODE': 'combined' if kind == 'stores' else 'canonical'}
    process = subprocess.Popen([str(binary)], env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    result = {'expectedReady': ready, 'readinessObserved': False}
    try:
        deadline = time.monotonic() + 8
        if ready:
            while time.monotonic() < deadline and process.poll() is None:
                try:
                    with urllib.request.urlopen('http://127.0.0.1:' + str(port) + route + '/health/ready', timeout=.5) as response:
                        if response.status == 200:
                            result['readinessObserved'] = True
                            break
                except (urllib.error.URLError, TimeoutError, OSError):
                    pass
                time.sleep(.05)
            need(result['readinessObserved'], 'RESTRICTED_RUNTIME_READY_FAILED')
        else:
            process.wait(timeout=8)
            need(process.returncode != 0, 'INVALID_BASELINE_ACCEPTED')
            result['rejectedWithNonzeroExit'] = True
    finally:
        if process.poll() is None:
            process.send_signal(signal.SIGTERM)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        log = process.communicate(timeout=2)[0]
        result['childStoppedAndReaped'] = process.poll() is not None
        result['privateLogBytesScanned'] = len(log)
        result['secretFindings'] = sum(value.encode() in log for value in (password, secret))
        need(result['secretFindings'] == 0, 'PRIVATE_RUNTIME_LOG_LEAK')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--kind', choices=('stores', 'orders'), required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--migration-bin', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    validation = Path('/mnt/d/WorkSpace/rust/rust-saas/saas-plateform/backend/docs/designs/unified-auth/validation')
    need(args.output.parent.resolve() == validation and not args.output.exists() and not args.output.is_symlink()
         and re.fullmatch(r'phase-25-readonly-startup-[a-z0-9-]+\.json', args.output.name), 'OUTPUT_REJECTED')
    for binary in (args.binary, args.migration_bin):
        need(binary.is_file() and binary.resolve() == binary and not binary.is_symlink(), 'BINARY_REJECTED')
    owned = binding.load_binding(TASK)
    need(os.getuid() == owned['taskUid'], 'UNPRIVILEGED_OWNER_REQUIRED')
    prefix = 'STORES' if args.kind == 'stores' else 'WORK_ORDERS'
    fingerprint = 'stores' if args.kind == 'stores' else 'work_orders'
    root = Path('/mnt/d/WorkSpace/rust/rust-saas') / ('saas-stores' if args.kind == 'stores' else 'saas-work-orders') / 'backend'
    token = uuid.uuid4().hex
    database = 'p25startup_' + args.kind + '_' + token
    owner, runtime = 'p25startup_' + token[-16:] + '_owner', 'p25startup_' + token[-16:] + '_runtime'
    marker = 'unified-auth-phase25-readonly-service/1/' + token
    password = secrets.token_hex(32)
    guard = "DO $$ BEGIN IF NOT (" + binding.sql_guard(owned) + ") THEN RAISE EXCEPTION 'PROBE_CLUSTER'; END IF; END $$;\n"
    created_roles = created_database = False
    report = {'contractVersion': 'unified-auth-phase25-readonly-service/1', 'result': 'FAIL',
        'kind': args.kind, 'evidenceLayer': 'ACTUAL_POSTGRESQL_LOGIN_ROLE_AND_NATIVE_HTTP',
        'startedAtEpoch': time.time(), 'binarySha256': digest(args.binary), 'migratorSha256': digest(args.migration_bin),
        'compileInputs': source_inputs(root), 'checks': {}, 'dockerContainersCreated': 0, 'dockerContainersReused': 0,
        'dedicatedDatabasesCreated': 0, 'dedicatedRolesCreated': 0,
        'formalOrSharedApplicationDatabasesModified': False, 'httpsOrWholeCandidateAcceptanceClaimed': False}
    try:
        report['stage'] = 'CREATE_OWNED_EMPTY_DATABASE_AND_ROLES'
        fresh = execute(guard + 'SELECT NOT EXISTS(SELECT 1 FROM pg_database WHERE datname=' + literal(database)
            + ') AND NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname IN(' + literal(owner) + ',' + literal(runtime) + '));')
        need(fresh.stdout.strip() == b't', 'FRESH_NAMES_REQUIRED')
        execute(guard + 'BEGIN; CREATE ROLE "' + owner + '" NOLOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE; '
            + 'COMMENT ON ROLE "' + owner + '" IS ' + literal(marker) + '; CREATE ROLE "' + runtime
            + '" LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE PASSWORD ' + literal(password)
            + '; COMMENT ON ROLE "' + runtime + '" IS ' + literal(marker) + '; COMMIT;')
        created_roles = True
        report['dedicatedRolesCreated'] = 2
        execute(guard + 'CREATE DATABASE "' + database + '" OWNER "' + owner + '";')
        created_database = True
        report['dedicatedDatabasesCreated'] = 1
        execute(guard + 'COMMENT ON DATABASE "' + database + '" IS ' + literal(marker) + ';')
        execute(guard + 'ALTER SCHEMA public OWNER TO "' + owner + '"; REVOKE ALL ON DATABASE "' + database
            + '" FROM PUBLIC; REVOKE ALL ON SCHEMA public FROM PUBLIC; GRANT CONNECT ON DATABASE "' + database
            + '" TO "' + runtime + '"; GRANT USAGE ON SCHEMA public TO "' + runtime + '";', database=database)
        report['stage'] = 'EMPTY_DATABASE_MUST_FAIL_WITHOUT_DDL'
        report['checks']['emptyDatabase'] = run_service(args.binary, args.kind, database, runtime, password, ready=False)
        empty = execute(guard + "SELECT count(*)=0 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' "
            "AND c.relkind IN('r','p','v','m','S');", database=database)
        need(empty.stdout.strip() == b't', 'EMPTY_RUNTIME_CREATED_SCHEMA')
        report['checks']['emptyDatabase']['schemaObjectsCreated'] = 0

        report['stage'] = 'OFFLINE_MIGRATOR_PROVISIONS_CANONICAL_BASELINE'
        url = 'postgresql://postgres@localhost:55494/' + database + '?host=' + str(TASK) + '&options[role]=' + owner
        migrated = subprocess.run([str(args.migration_bin)], env={'PATH': '/usr/bin:/bin', prefix + '_DATABASE_URL': url},
            capture_output=True, timeout=45)
        need(migrated.returncode == 0, 'OFFLINE_MIGRATOR_FAILED')
        report['checks']['offlineMigrator'] = {'exitCode': 0, 'actualOwnerDelegation': True}
        execute(guard + 'REVOKE ALL ON ALL TABLES IN SCHEMA public FROM PUBLIC; REVOKE ALL ON ALL FUNCTIONS IN SCHEMA public FROM PUBLIC; '
            + 'GRANT SELECT ON ALL TABLES IN SCHEMA public TO "' + runtime + '"; GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA public TO "'
            + runtime + '";', database=database)
        fence = execute("SELECT current_user=" + literal(runtime) + ' AND current_database()=' + literal(database)
            + " AND NOT has_schema_privilege(current_user,'public','CREATE') AND NOT has_database_privilege(current_user,"
            + literal(database) + ",'CREATE,TEMP') AND NOT pg_has_role(current_user," + literal(owner)
            + ",'MEMBER') AND has_table_privilege(current_user,'public._sqlx_migrations','SELECT') "
            + "AND NOT has_table_privilege(current_user,'public._sqlx_migrations','INSERT,UPDATE,DELETE');",
            database=database, username=runtime, password=password)
        need(fence.stdout.strip() == b't', 'ROLE_FENCE_FAILED')
        report['actualRestrictedLoginVerified'] = True

        report['stage'] = 'NORMAL_API_READONLY_READY'
        history_sql = ("SELECT jsonb_build_object('history',(SELECT jsonb_agg(to_jsonb(history) ORDER BY version) FROM "
            "_sqlx_migrations history),'schema',public." + fingerprint + "_schema_fingerprint())::text;")
        before_runtime = execute(guard + history_sql, database=database).stdout.strip()
        report['checks']['currentBaseline'] = run_service(args.binary, args.kind, database, runtime, password, ready=True)
        after_runtime = execute(guard + history_sql, database=database).stdout.strip()
        need(before_runtime == after_runtime, 'READONLY_STARTUP_CHANGED_BASELINE')
        report['checks']['currentBaseline']['historyAndSchemaUnchanged'] = True
        report['checks']['currentBaseline']['historyAndSchemaObservationSha256'] = hashlib.sha256(before_runtime).hexdigest()
        report['stage'] = 'CHECKSUM_MISMATCH_MUST_FAIL'
        checksum = execute(guard + "SELECT encode(checksum,'hex') FROM _sqlx_migrations WHERE version=1;", database=database).stdout.strip().decode()
        need(re.fullmatch(r'[0-9a-f]{96}', checksum), 'CANONICAL_CHECKSUM_REQUIRED')
        execute(guard + "UPDATE _sqlx_migrations SET checksum=decode('00','hex') WHERE version=1;", database=database)
        report['checks']['wrongChecksum'] = run_service(args.binary, args.kind, database, runtime, password, ready=False)
        execute(guard + 'UPDATE _sqlx_migrations SET checksum=decode(' + literal(checksum) + ",'hex') WHERE version=1;", database=database)

        report['stage'] = 'SCHEMA_FINGERPRINT_MISMATCH_MUST_FAIL'
        stored = execute(guard + 'SELECT fingerprint FROM _' + fingerprint + '_schema_contract WHERE singleton;', database=database).stdout.strip().decode()
        need(re.fullmatch(r'[0-9a-f]{32}', stored), 'SCHEMA_FINGERPRINT_REQUIRED')
        execute(guard + 'UPDATE _' + fingerprint + '_schema_contract SET fingerprint=' + literal('0' * 32) + ' WHERE singleton;', database=database)
        report['checks']['wrongFingerprint'] = run_service(args.binary, args.kind, database, runtime, password, ready=False)
        execute(guard + 'UPDATE _' + fingerprint + '_schema_contract SET fingerprint=' + literal(stored) + ' WHERE singleton;', database=database)

        report['stage'] = 'MISSING_HISTORY_MUST_FAIL_WITHOUT_RECREATE'
        execute(guard + 'DROP TABLE _sqlx_migrations;', database=database)
        report['checks']['missingHistory'] = run_service(args.binary, args.kind, database, runtime, password, ready=False)
        absent = execute(guard + "SELECT to_regclass('public._sqlx_migrations') IS NULL;", database=database)
        need(absent.stdout.strip() == b't', 'RUNTIME_RECREATED_HISTORY')
        report['checks']['missingHistory']['historyRecreated'] = False
        report['result'] = 'PASS'
    except Exception as error:
        report['failureCode'] = str(error) if re.fullmatch(r'PHASE25_DDL_PROBE_[A-Z0-9_]+', str(error)) else 'PHASE25_DDL_PROBE_SERVICE_CHECK_FAILED'
    finally:
        failures = []
        if created_database:
            try:
                actual = execute(guard + "SELECT pg_get_userbyid(datdba)||'|'||COALESCE(shobj_description(oid,'pg_database'),'') FROM pg_database WHERE datname="
                    + literal(database) + ';').stdout.strip().decode()
                need(actual == owner + '|' + marker, 'CLEANUP_DATABASE_BINDING_CHANGED')
                execute(guard + 'SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname=' + literal(database) + ';')
                execute(guard + 'DROP DATABASE "' + database + '";')
            except Exception:
                failures.append('OWNED_DATABASE_CLEANUP_FAILED')
        if created_roles:
            for role in (runtime, owner):
                try:
                    actual = execute(guard + "SELECT COALESCE(shobj_description(oid,'pg_authid'),'') FROM pg_roles WHERE rolname=" + literal(role) + ';').stdout.strip().decode()
                    need(actual == marker, 'CLEANUP_ROLE_BINDING_CHANGED')
                    execute(guard + 'DROP ROLE "' + role + '";')
                except Exception:
                    failures.append('OWNED_ROLE_CLEANUP_FAILED')
        remaining = execute(guard + "SELECT json_build_object('databases',(SELECT count(*) FROM pg_database WHERE datname="
            + literal(database) + "),'roles',(SELECT count(*) FROM pg_roles WHERE rolname IN(" + literal(owner) + ',' + literal(runtime) + ')))::text;')
        report['cleanup'] = {**json.loads(remaining.stdout), 'failures': failures,
            'runtimePrivateFilesCreated': 0, 'logsRetained': False, 'sharedPostgresRetained': True}
        if failures or report['cleanup']['databases'] or report['cleanup']['roles']:
            report['result'] = 'FAIL'
        report['finishedAtEpoch'] = time.time()
        with args.output.open('x', encoding='utf-8') as output:
            json.dump(report, output, indent=2)
            output.write('\n')
    print(json.dumps({'kind': args.kind, 'result': report['result'], 'stage': report['stage'],
        'checksCompleted': len(report['checks']), 'failureCode': report.get('failureCode'), 'cleanup': report['cleanup']}))
    return 0 if report['result'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
