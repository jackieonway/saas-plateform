"""Restore-new, owner/ACL-preserving recovery of the two owned business DBs.

The root caller stops business services first. Core identity/platform, original
business DBs/roles and HBA are never modified. stdout is one redacted JSON result.
"""
import argparse
import copy
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import re
import select
import signal
import subprocess
import time
from urllib.parse import urlsplit, urlunsplit

TASK = Path('/tmp/rust-saas-unified-auth-20261028')
JOURNAL = 'phase25-business-recovery-journal.json'
CARRIERS = 'phase25-business-recovery-carriers.json'
CONTRACT = 'unified-auth-phase25-business-recovery-journal/1'
KINDS = ('stores', 'orders')
PREFIX = 'saas-plateform/backend/tests/unified-auth-https/'


def need(value, code):
    if not value:
        raise RuntimeError('PHASE25_BUSINESS_RECOVERY_' + code)


def safe_code(error):
    label = str(error)
    return label if re.fullmatch(r'PHASE25_BUSINESS_RECOVERY_[A-Z0-9_]+', label) else 'PHASE25_BUSINESS_RECOVERY_FAILED'


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def literal(value):
    return "'" + str(value).replace("'", "''") + "'"


def ident(value):
    need(type(value) is str and re.fullmatch(r'[a-z][a-z0-9_]{0,62}', value), 'IDENTIFIER_REJECTED')
    return '"' + value + '"'


def sha(value):
    return hashlib.sha256(value).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()


def utility(task, command):
    """setpriv execs the single serial PG tool, so run's kill/wait owns its PID.

    runuser may fork a PAM child. A timeout or caught TERM must instead kill and
    reap the actual pg_dump/pg_restore process before backup cleanup continues.
    Neither command uses parallel workers.
    """
    need(command and str(command[0]) in ('/usr/lib/postgresql/18/bin/pg_dump', '/usr/lib/postgresql/18/bin/pg_restore')
         and not any(str(value) in ('-j', '--jobs', '--no-owner', '--no-acl', '--role', '--create', '--clean')
                     or str(value).startswith(('--jobs=', '--role=')) for value in command), 'PG_TOOL_ARGUMENTS_REJECTED')
    result = subprocess.run(['/usr/bin/setpriv', '--reuid=' + str(task.uid), '--regid=' + str(task.gid),
        '--init-groups', *map(str, command)], capture_output=True, timeout=45,
        env={'PATH': '/usr/lib/postgresql/18/bin:/usr/bin:/bin', 'HOME': str(TASK), 'LANG': 'C.UTF-8',
             'PGOPTIONS': '-c log_statement=none -c log_min_error_statement=panic'})
    need(result.returncode == 0, 'PG_TOOL_FAILED')
    return result.stdout


def database_acl_plan(target, owner, rows):
    """Preserve current fixture grantors exactly; unsupported chains fail closed."""
    ident(target)
    ident(owner)
    need(type(rows) is list and rows, 'DATABASE_ACL_REQUIRED')
    grantees = {owner, 'PUBLIC'}
    for row in rows:
        need(type(row) is dict and set(row) == {'grantor', 'grantee', 'privilege', 'grantable'}
             and row['grantor'] == owner and row['privilege'] in ('CREATE', 'CONNECT', 'TEMPORARY')
             and type(row['grantable']) is bool, 'DATABASE_ACL_GRANTOR_OR_PRIVILEGE_REJECTED')
        if row['grantee'] != 'PUBLIC':
            ident(row['grantee'])
        grantees.add(row['grantee'])
    statements = ['REVOKE ALL PRIVILEGES ON DATABASE ' + ident(target) + ' FROM '
        + ('PUBLIC' if role == 'PUBLIC' else ident(role)) for role in sorted(grantees)]
    statements.append('SET LOCAL ROLE ' + ident(owner))
    for row in rows:
        statements.append('GRANT ' + row['privilege'] + ' ON DATABASE ' + ident(target) + ' TO '
            + ('PUBLIC' if row['grantee'] == 'PUBLIC' else ident(row['grantee']))
            + (' WITH GRANT OPTION' if row['grantable'] else ''))
    return 'BEGIN;\n' + ';\n'.join(statements) + '; RESET ROLE; COMMIT;'


def db_guard(task, name, owner, marker):
    raw = task.pg("SELECT json_build_object('owner',pg_get_userbyid(datdba),'marker',shobj_description(oid,'pg_database'))::text "
        'FROM pg_database WHERE datname=' + literal(name))
    need(raw and json.loads(raw) == {'owner': owner, 'marker': marker}, 'DATABASE_OWNERSHIP_CHANGED')


def acl(task, database):
    return json.loads(task.pg("SELECT COALESCE(json_agg(json_build_object('grantor',pg_get_userbyid(x.grantor),"
        "'grantee',CASE WHEN x.grantee=0 THEN 'PUBLIC' ELSE pg_get_userbyid(x.grantee) END,"
        "'privilege',x.privilege_type,'grantable',x.is_grantable) ORDER BY x.grantor,x.grantee,x.privilege_type,x.is_grantable),'[]'::json) "
        "FROM pg_database d CROSS JOIN LATERAL aclexplode(COALESCE(d.datacl,acldefault('d',d.datdba))) x WHERE d.datname=" + literal(database)))


def snapshot(task, database, kind):
    fingerprint = 'stores' if kind == 'stores' else 'work_orders'
    tables = json.loads(task.pg("SELECT COALESCE(json_agg(c.relname ORDER BY c.relname),'[]'::json)::text FROM pg_class c "
        "JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind IN('r','p')", database))
    rows = {}
    for table in tables:
        raw = task.pg("SELECT json_build_object('rows',count(*),'digest',md5(COALESCE(string_agg(to_jsonb(row)::text,E'\\n' "
            "ORDER BY to_jsonb(row)::text),'')))::text FROM public." + ident(table) + ' row', database)
        rows[table] = json.loads(raw)
    schema = task.pg('SELECT public.' + ident(fingerprint + '_schema_fingerprint') + '()', database)
    history = task.pg("SELECT json_agg(to_jsonb(history) ORDER BY version)::text FROM _sqlx_migrations history", database)
    objects = task.pg("SELECT jsonb_agg(item ORDER BY item::text)::text FROM ("
        "SELECT jsonb_build_object('kind','schema','name',n.nspname,'owner',pg_get_userbyid(n.nspowner),'acl',"
        "(SELECT jsonb_agg(jsonb_build_array(pg_get_userbyid(a.grantor),CASE WHEN a.grantee=0 THEN 'PUBLIC' ELSE pg_get_userbyid(a.grantee) END,a.privilege_type,a.is_grantable) ORDER BY a.grantor,a.grantee,a.privilege_type) FROM aclexplode(COALESCE(n.nspacl,acldefault('n',n.nspowner))) a)) item "
        "FROM pg_namespace n WHERE n.nspname='public' UNION ALL "
        "SELECT jsonb_build_object('kind','table','name',c.relname,'owner',pg_get_userbyid(c.relowner),'acl',"
        "(SELECT jsonb_agg(jsonb_build_array(pg_get_userbyid(a.grantor),CASE WHEN a.grantee=0 THEN 'PUBLIC' ELSE pg_get_userbyid(a.grantee) END,a.privilege_type,a.is_grantable) ORDER BY a.grantor,a.grantee,a.privilege_type) FROM aclexplode(COALESCE(c.relacl,acldefault('r',c.relowner))) a)) "
        "FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind IN('r','p') UNION ALL "
        "SELECT jsonb_build_object('kind','function','name',p.proname||'('||pg_get_function_identity_arguments(p.oid)||')','owner',pg_get_userbyid(p.proowner),'acl',"
        "(SELECT jsonb_agg(jsonb_build_array(pg_get_userbyid(a.grantor),CASE WHEN a.grantee=0 THEN 'PUBLIC' ELSE pg_get_userbyid(a.grantee) END,a.privilege_type,a.is_grantable) ORDER BY a.grantor,a.grantee,a.privilege_type) FROM aclexplode(COALESCE(p.proacl,acldefault('f',p.proowner))) a)) "
        "FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public') owned_objects", database)
    return {'rows': rows, 'schemaFingerprint': schema, 'historySha256': sha(history.encode()),
        'objectOwnerAclSha256': sha(objects.encode()), 'databaseAcl': acl(task, database)}


def own_task(base, journal):
    task = base.Task()
    task.state_path = TASK / JOURNAL
    task.state = journal
    return task


def validate_carriers(task, value, business, *, snapshot_label=None):
    need(type(value) is dict and value.get('contractVersion') == 'unified-auth-phase25-business-recovery-carriers/1'
         and all(value.get(key) == business[key] for key in ('clusterId', 'deploymentId', 'fixtureId', 'baseDatabase'))
         and (snapshot_label is None or value.get('snapshot') == snapshot_label)
         and type(value.get('carriers')) is dict and set(value['carriers']) == set(KINDS), 'CARRIER_BINDING_CHANGED')
    for item in value['carriers'].values():
        need(type(item) is dict and set(item) == {'value', 'sha256', 'expiresAtMs', 'sourceResponseSha256'}
             and type(item['value']) is str and 1 <= len(item['value']) <= 16384
             and item['sha256'] == sha(item['value'].encode()) and re.fullmatch(r'[0-9a-f]{64}', item['sourceResponseSha256'])
             and type(item['expiresAtMs']) in (int, float) and math.isfinite(item['expiresAtMs'])
             and item['expiresAtMs'] > 0, 'CARRIER_MATERIAL_CHANGED')


def validate_cleanup_journal(task, journal):
    uuid7 = r'[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}'
    need(journal.get('contractVersion') == CONTRACT and journal.get('clusterId') == task.cluster
         and re.fullmatch(uuid7, journal.get('fixtureId', '')) and re.fullmatch(uuid7, journal.get('deploymentId', ''))
         and journal.get('fixtureToken') == journal['fixtureId'].replace('-', '')
         and re.fullmatch(r'unified_auth_https_[0-9a-f]{32}', journal.get('baseDatabase', ''))
         and re.fullmatch(r'phase25-tools-r[1-9][0-9]*', journal.get('snapshot', ''))
         and journal.get('directory') == str(TASK / ('phase25-business-recovery-' + journal['fixtureToken']))
         and set(journal.get('targets', {})) == set(KINDS), 'CLEANUP_JOURNAL_INVALID')
    for kind in KINDS:
        row = journal['targets'][kind]
        need(row.get('sourceDatabase') == 'unified_auth_phase25_' + kind + '_' + journal['baseDatabase'][-32:]
             and row.get('owner') == 'uah25_' + journal['baseDatabase'][-12:] + '_' + kind + '_owner'
             and row.get('sourceMarker') == 'unified-auth-phase25-business/1/' + journal['deploymentId'] + '/' + kind,
             'CLEANUP_SOURCE_BINDING_CHANGED')
    script = str(TASK / ('source-snapshot-' + journal['snapshot']) / PREFIX / 'phase25-business-recovery.py')
    for field in ('processes', 'pendingSpawns'):
        values = journal.get(field, {})
        need(type(values) is dict and set(values) <= {'business-recovery-' + kind for kind in KINDS}
             and all(value.get('script') == script and value.get('uid') == task.uid for value in values.values()),
             'CLEANUP_PROCESS_SCOPE_CHANGED')


def cleanup(base, task=None):
    report = {'result': 'PASS', 'restoreDatabasesRemaining': 0, 'processesRemaining': 0,
        'privateFilesRemoved': 0, 'privateBytesRemoved': 0, 'carrierFilesRemaining': 0, 'failures': []}
    path, carriers = TASK / JOURNAL, TASK / CARRIERS
    if not any(p.exists() or p.is_symlink() for p in (path, carriers)):
        return report
    task = task or base.Task()
    journal = task.json(path) if path.exists() or path.is_symlink() else None
    secret_candidates = []
    if carriers.exists() and not carriers.is_symlink():
        material = task.json(carriers)
        secret_candidates.extend(item.get('value') for item in material.get('carriers', {}).values() if type(item) is dict)
    business_environment = TASK / 'phase25-business-environment.json'
    if business_environment.exists():
        for service in task.json(business_environment).get('services', {}).values():
            for name, value in service.get('environment', {}).items():
                if type(value) is str and ('SECRET' in name or 'TOKEN' in name):
                    secret_candidates.append(value)
                if type(value) is str and name.endswith('DATABASE_URL'):
                    secret_candidates.append(urlsplit(value).password)
    report['nativeLogCanaryFindings'] = 0
    if journal:
        validate_cleanup_journal(task, journal)
        isolated = own_task(base, journal)
        try:
            isolated.stop_pending_spawns()
        except BaseException:
            report['failures'].append('PENDING_SPAWN_CLEANUP_FAILED')
        for kind in reversed(KINDS):
            lease = TASK / ('phase25-business-recovery-child-' + kind + '.json')
            try:
                isolated.stop_process('business-recovery-' + kind)
                if lease.exists() or lease.is_symlink():
                    native = task.json(lease)
                    runtime = load('phase25_recovery_runtime_cleanup', 'phase25-business-runtime.py')
                    expected_binary = str(TASK / 'bin' / ('stores-api' if kind == 'stores' else 'work-orders-api'))
                    need(native.get('binary') == expected_binary and native.get('uid') == task.uid
                         and type(native.get('pid')) is int and native['pid'] > 1
                         and re.fullmatch(r'[0-9]+', native.get('startTime', '')), 'CLEANUP_CHILD_LEASE_INVALID')
                    if not Path('/proc', str(native['pid'])).exists():
                        need(task.json(lease) == native, 'CLEANUP_CHILD_LEASE_CHANGED')
                        lease.unlink()
                        continue
                    record = runtime.process_identity(native['pid'])
                    need(record == {key: native[key] for key in record}, 'CLEANUP_CHILD_IDENTITY_CHANGED')
                    descriptor = os.pidfd_open(native['pid'])
                    try:
                        pinned = runtime.process_identity(native['pid'])
                        need(pinned == {key: native[key] for key in pinned}, 'CLEANUP_PINNED_CHILD_IDENTITY_CHANGED')
                        runtime.stop_descriptor(descriptor)
                    finally:
                        os.close(descriptor)
                    need(task.json(lease) == native, 'CLEANUP_CHILD_LEASE_CHANGED')
                    lease.unlink()
            except ProcessLookupError:
                if lease.exists():
                    need(task.json(lease) == native, 'CLEANUP_CHILD_LEASE_CHANGED')
                    lease.unlink()
            except BaseException:
                report['failures'].append('OWNED_PROCESS_CLEANUP_FAILED_' + kind.upper())
        journal = isolated.state
        for kind in reversed(KINDS):
            record = journal['targets'][kind]
            try:
                expected = 'unified_auth_p25restore_' + kind + '_' + journal['fixtureToken']
                need(record['database'] == expected and record['database'] != record['sourceDatabase']
                     and record['marker'] == 'unified-auth-phase25-business-recovery/1/' + journal['fixtureId'] + '/' + kind,
                     'CLEANUP_TARGET_SCOPE_CHANGED')
                existing = task.pg('SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname=' + literal(record['database']) + ')') == 't'
                if existing:
                    actual = json.loads(task.pg("SELECT json_build_object('owner',pg_get_userbyid(datdba),'marker',shobj_description(oid,'pg_database'))::text FROM pg_database WHERE datname=" + literal(record['database'])))
                    need(actual['owner'] == record['owner'] and (actual['marker'] == record['marker']
                         or (record.get('creationPending') is True and actual['marker'] is None)), 'CLEANUP_DATABASE_OWNERSHIP_CHANGED')
                    need(not journal.get('processes'), 'TARGET_WRITERS_REMAIN')
                    task.pg('SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname=' + literal(record['database']))
                    task.pg('DROP DATABASE ' + ident(record['database']))
            except BaseException:
                report['failures'].append('OWNED_DATABASE_CLEANUP_FAILED_' + kind.upper())
        directory = Path(journal['directory'])
        if directory.exists() or directory.is_symlink():
            try:
                need(directory.resolve() == directory and not directory.is_symlink() and directory.stat().st_uid == task.uid,
                     'CLEANUP_DIRECTORY_CHANGED')
                if not any(directory.iterdir()):
                    directory.rmdir()
                    directory = None
                if directory is None:
                    pass
                else:
                    allowed = {'OWNER', 'stores.dump', 'orders.dump'}
                    need(all(p.is_file() and not p.is_symlink() and p.name in allowed for p in directory.iterdir()), 'CLEANUP_DIRECTORY_CONTENT_CHANGED')
                    need(task.json(directory / 'OWNER') == {'contractVersion': CONTRACT, 'fixtureId': journal['fixtureId']}, 'CLEANUP_DIRECTORY_OWNER_CHANGED')
                    for item in directory.iterdir():
                        size = task.checked(item).stat().st_size
                        item.unlink()
                        report['privateFilesRemoved'] += 1
                        report['privateBytesRemoved'] += size
                    directory.rmdir()
            except BaseException:
                report['failures'].append('PRIVATE_BACKUP_CLEANUP_FAILED')
        for kind in KINDS:
            for filename in ('phase23-business-recovery-' + kind + '.log', 'phase25-business-recovery-' + kind + '.log'):
                log = TASK / filename
                if log.exists() or log.is_symlink():
                    try:
                        size = task.checked(log, empty_ok=True).stat().st_size
                        raw = task.read(log, empty_ok=True)
                        report['nativeLogCanaryFindings'] += sum(value.encode() in raw for value in secret_candidates if value)
                        log.unlink()
                        report['privateFilesRemoved'] += 1
                        report['privateBytesRemoved'] += size
                    except BaseException:
                        report['failures'].append('PRIVATE_LOG_CLEANUP_FAILED')
        report['processesRemaining'] = len(journal.get('processes', {})) + len(journal.get('pendingSpawns', {}))
        report['restoreDatabasesRemaining'] = int(task.pg('SELECT count(*) FROM pg_database WHERE datname=ANY(ARRAY['
            + ','.join(literal(row['database']) for row in journal['targets'].values()) + '])'))
    if carriers.exists() or carriers.is_symlink():
        try:
            value = task.json(carriers)
            business = journal if journal else task.json(TASK / 'phase25-business-manifest.json')
            validate_carriers(task, value, business, snapshot_label=journal['snapshot'] if journal else None)
            size = carriers.stat().st_size
            carriers.unlink()
            report['privateFilesRemoved'] += 1
            report['privateBytesRemoved'] += size
        except BaseException:
            report['failures'].append('PRIVATE_CARRIER_CLEANUP_FAILED')
    report['carrierFilesRemaining'] = int(carriers.exists() or carriers.is_symlink())
    if report['failures'] or report['restoreDatabasesRemaining'] or report['processesRemaining'] or report['carrierFilesRemaining'] or report.get('nativeLogCanaryFindings'):
        report['result'] = 'FAIL'
    elif journal and path.exists():
        path.unlink()
    return report


def supervise(kind):
    base = load('phase25_recovery_supervisor_base', 'phase23-environment.py')
    runtime = load('phase25_recovery_supervisor_runtime', 'phase25-business-runtime.py')
    task = base.Task(require_root=False)
    journal = task.json(TASK / JOURNAL)
    need(journal.get('contractVersion') == CONTRACT and kind in KINDS, 'SUPERVISOR_BINDING_INVALID')
    services, _ = runtime.configuration(task)
    namespace = runtime.supervisor_namespace(task, base)
    need(os.readlink('/proc/self/ns/mnt') == namespace, 'SUPERVISOR_NAMESPACE_CHANGED')
    source = services[kind]
    environment = copy.deepcopy(source['environment'])
    key = 'STORES_DATABASE_URL' if kind == 'stores' else 'WORK_ORDERS_DATABASE_URL'
    url = urlsplit(environment[key])
    environment[key] = urlunsplit((url.scheme, url.netloc, '/' + journal['targets'][kind]['database'], '', ''))
    binary = TASK / 'bin' / source['binary']
    lease = TASK / ('phase25-business-recovery-child-' + kind + '.json')
    need(not lease.exists() and not lease.is_symlink(), 'SUPERVISOR_LEASE_EXISTS')
    fd = task.open_log(TASK / ('phase25-business-recovery-' + kind + '.log'))
    child = descriptor = record = None
    def interrupt(*_args):
        raise SystemExit(0)
    handlers = {sig: signal.signal(sig, interrupt) for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)}
    try:
        with base.spawn_signals(defer=True):
            child = subprocess.Popen([str(binary)], env=environment, cwd=TASK, stdout=fd, stderr=subprocess.STDOUT)
        descriptor = os.pidfd_open(child.pid)
        record = runtime.process_identity(child.pid)
        need(record['binary'] == str(binary) and record['uid'] == task.uid and record['namespace'] == namespace,
             'SUPERVISOR_CHILD_CHANGED')
        task.write(lease, record)
        while not select.select([descriptor], [], [], .2)[0]:
            need((TASK / 'namespace.pid').exists(), 'SUPERVISOR_NAMESPACE_ENDED')
        raise RuntimeError('PHASE25_BUSINESS_RECOVERY_CHILD_EXITED')
    finally:
        for sig in handlers:
            signal.signal(sig, signal.SIG_IGN)
        try:
            if child is not None:
                if descriptor is None:
                    descriptor = os.pidfd_open(child.pid)
                runtime.stop_descriptor(descriptor)
                child.wait(timeout=.5)
            if record is not None and lease.exists():
                need(task.json(lease) == record, 'SUPERVISOR_LEASE_CHANGED')
                lease.unlink()
        finally:
            if descriptor is not None:
                os.close(descriptor)
            os.close(fd)


def recover(base):
    task = base.Task()
    runtime = load('phase25_recovery_runtime', 'phase25-business-runtime.py')
    fixture = load('phase25_recovery_fixture', 'phase25-business-fixture.py')
    need(task.state and task.state.get('prepared') and task.state.get('mode') == 'SOURCE'
         and not task.state.get('sourceQuarantined'), 'LIVE_SOURCE_REQUIRED')
    services, bound = runtime.configuration(task)
    business = task.json(TASK / 'phase25-business-manifest.json')
    fixture.validate_manifest(task, business)
    fixture.assert_no_business_leases(task)
    need(not any('business-' + kind in task.state['processes'] for kind in runtime.SPECS), 'BUSINESS_WRITERS_REMAIN')
    snapshot_path, manifest, _ = task.snapshot(all_inputs=False)
    need(Path(__file__).resolve() == snapshot_path / PREFIX / 'phase25-business-recovery.py'
         and task.digest(Path(__file__)) == manifest['inputs'][PREFIX + 'phase25-business-recovery.py'], 'FROZEN_RECOVERY_REQUIRED')
    carriers = task.json(TASK / CARRIERS)
    validate_carriers(task, carriers, business, snapshot_label=task.state['snapshot'])
    need(carriers.get('buildSnapshot') == task.state['buildSnapshot']
         and carriers.get('buildSnapshotSha256') == business['snapshotSha256'], 'CARRIER_BUILD_CHANGED')
    token = business['fixtureId'].replace('-', '')
    directory = TASK / ('phase25-business-recovery-' + token)
    need(not directory.exists() and not directory.is_symlink() and not (TASK / JOURNAL).exists(), 'EMPTY_RECOVERY_LEASE_REQUIRED')
    journal = {'contractVersion': CONTRACT, **{key: business[key] for key in ('clusterId', 'deploymentId', 'fixtureId', 'baseDatabase')},
        'fixtureToken': token, 'directory': str(directory), 'snapshot': task.state['snapshot'],
        'buildSnapshot': task.state['buildSnapshot'], 'buildSnapshotSha256': business['snapshotSha256'],
        'processes': {}, 'pendingSpawns': {}, 'targets': {}}
    for kind in KINDS:
        original = business['services'][kind]
        db_guard(task, original['database'], original['owner'], original['marker'])
        need(task.pg('SELECT count(*) FROM pg_stat_activity WHERE datname=' + literal(original['database'])) == '0', 'SOURCE_DATABASE_WRITERS_REMAIN')
        target = 'unified_auth_p25restore_' + kind + '_' + token
        need(task.pg('SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname=' + literal(target) + ')') == 'f', 'RESTORE_TARGET_ALREADY_EXISTS')
        journal['targets'][kind] = {'database': target, 'sourceDatabase': original['database'], 'owner': original['owner'],
            'sourceMarker': original['marker'], 'marker': 'unified-auth-phase25-business-recovery/1/' + business['fixtureId'] + '/' + kind,
            'creationPending': True}
    isolated = own_task(base, journal)
    isolated.save()
    directory.mkdir(mode=0o700)
    os.chown(directory, task.uid, task.gid)
    task.write(directory / 'OWNER', {'contractVersion': CONTRACT, 'fixtureId': business['fixtureId']})
    started = time.monotonic()
    report = {'contractVersion': 'unified-auth-phase25-business-recovery/1', 'result': 'FAIL',
        'sourceSnapshot': task.state['snapshot'], 'buildSnapshot': task.state['buildSnapshot'],
        'buildSnapshotSha256': business['snapshotSha256'], 'dockerContainersCreated': 0, 'dockerContainersReused': 0,
        'sourceDatabasesModified': False, 'sourceRolesModified': False, 'coreOrHbaModified': False,
        'restoreOwnerAclPreserved': True, 'roleRemappingUsed': False, 'cases': [], 'fixtureRpoSeconds': 0,
        'formalReportContainsRawCredentials': False,
        'rtoScope': 'Dump start through both restored authenticated business reads; excludes capture, stopping writers, preflight, journal/directory setup and final cleanup',
        'rpoScope': 'Both owned source databases quiescent, dump/restore observations equal; no production WAL/SLA claim'}
    source_snapshots = {}
    try:
        for kind in KINDS:
            row = journal['targets'][kind]
            source_snapshots[kind] = snapshot(task, row['sourceDatabase'], kind)
            database_acl_plan(row['database'], row['owner'], source_snapshots[kind]['databaseAcl'])
            archive = directory / (kind + '.dump')
            utility(task, ['/usr/lib/postgresql/18/bin/pg_dump', '--format=custom', '--file', archive,
                '--host', '127.0.0.1', '--port', '55494', '--username', 'postgres', '--no-password', row['sourceDatabase']])
            need(archive.is_file() and archive.stat().st_size > 0, 'BACKUP_ARCHIVE_MISSING')
            archive.chmod(0o600)
            task.pg('CREATE DATABASE ' + ident(row['database']) + ' OWNER ' + ident(row['owner']))
            task.pg('COMMENT ON DATABASE ' + ident(row['database']) + ' IS ' + literal(row['marker']))
            row['creationPending'] = False
            isolated.save()
            utility(task, ['/usr/lib/postgresql/18/bin/pg_restore', '--host', '127.0.0.1', '--port', '55494',
                '--username', 'postgres', '--no-password', '--exit-on-error', '--dbname', row['database'], archive])
            task.pg(database_acl_plan(row['database'], row['owner'], source_snapshots[kind]['databaseAcl']))
            restored = snapshot(task, row['database'], kind)
            need(restored == source_snapshots[kind], 'RESTORED_DATA_SCHEMA_OWNER_OR_ACL_CHANGED')
            need(snapshot(task, row['sourceDatabase'], kind) == source_snapshots[kind], 'SOURCE_CHANGED_DURING_BACKUP')
            script = task.tools() / 'phase25-business-recovery.py'
            isolated.spawn('business-recovery-' + kind, task.ns_command(['python3', '-B', script, '--task-dir', TASK, '--supervise', kind]), script, task.uid)
            deadline = time.monotonic() + 15
            ready = False
            while time.monotonic() < deadline:
                response = subprocess.run(task.ns_command(['curl', '--noproxy', '*', '--silent', '--max-time', '1', '--output', '/dev/null',
                    '--write-out', '%{http_code}', 'http://127.0.0.1:' + str(services[kind]['port']) + services[kind]['healthPath']]), capture_output=True, timeout=3)
                if response.returncode == 0 and response.stdout == b'200':
                    ready = True
                    break
                need(isolated.alive(isolated.state['processes']['business-recovery-' + kind]), 'RESTORED_RUNTIME_EXITED')
                time.sleep(.1)
            need(ready, 'RESTORED_RUNTIME_NOT_READY')
            item = carriers['carriers'][kind]
            ca = services[kind]['environment']['PLATFORM_CA_CERT_FILE']
            endpoint = '/stores-api/stores' if kind == 'stores' else '/work-orders-api/tickets'
            command = task.ns_command(['curl', '--noproxy', '*', '--silent', '--show-error', '--max-time', '15', '--cacert', ca,
                '--config', '-', '--write-out', '\n%{http_code}', 'https://console.saas.test:24447' + endpoint])
            response = subprocess.run(command, input=('header = "Authorization: Bearer ' + item['value'] + '"\n').encode(),
                capture_output=True, timeout=18)
            need(response.returncode == 0 and response.stdout.rsplit(b'\n', 1)[-1] == b'200', 'RESTORED_AUTHENTICATED_READ_REJECTED')
            body = json.loads(response.stdout.rsplit(b'\n', 1)[0])
            need(sha(canonical(body)) == item['sourceResponseSha256'], 'RESTORED_AUTHENTICATED_DATA_CHANGED')
            now = float(task.pg('SELECT extract(epoch FROM clock_timestamp())*1000'))
            need(now < item['expiresAtMs'], 'ORIGINAL_CARRIER_EXPIRED')
            report['cases'].append({'kind': kind, 'result': 'PASS', 'tableCount': len(restored['rows']),
                'businessAndHistoryRowsSha256': sha(canonical(restored['rows'])), 'schemaFingerprint': restored['schemaFingerprint'],
                'historySha256': restored['historySha256'], 'objectOwnerAclSha256': restored['objectOwnerAclSha256'],
                'databaseAclGrantorPreserved': True, 'actualRestrictedRuntimeReady': 200,
                'originalRealCarrierAuthenticatedRead': 200, 'originalSourceResponseDigestMatched': True,
                'carrierRemainingMs': round(item['expiresAtMs'] - now, 3)})
        report['fixtureRtoSeconds'] = round(time.monotonic() - started, 3)
        for kind in KINDS:
            need(snapshot(task, journal['targets'][kind]['sourceDatabase'], kind) == source_snapshots[kind], 'SOURCE_CHANGED_AFTER_RECOVERY')
        report['result'] = 'PASS'
    except BaseException as error:
        report['failureCode'] = safe_code(error)
    finally:
        findings = 0
        for kind in KINDS:
            for filename in ('phase23-business-recovery-' + kind + '.log', 'phase25-business-recovery-' + kind + '.log'):
                path = TASK / filename
                if path.exists():
                    raw = task.read(path, empty_ok=True)
                    candidates = [carriers['carriers'][name]['value'] for name in KINDS]
                    candidates += [urlsplit(services[name]['environment']['STORES_DATABASE_URL' if name == 'stores' else 'WORK_ORDERS_DATABASE_URL']).password for name in KINDS]
                    findings += sum(value.encode() in raw for value in candidates if value)
        report['nativeLogCanaryFindings'] = findings
        with base.spawn_signals(defer=True):
            report['cleanup'] = cleanup(base, task)
        if findings or report['cleanup']['result'] != 'PASS':
            report['result'] = 'FAIL'
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--task-dir', type=Path, required=True)
    parser.add_argument('--cleanup-only', action='store_true')
    parser.add_argument('--supervise', choices=KINDS)
    args = parser.parse_args()
    need(args.task_dir == TASK, 'EXACT_TASK_REQUIRED')
    if args.supervise:
        supervise(args.supervise)
        return 0
    base = load('phase25_business_recovery_base', 'phase23-environment.py')
    try:
        with base.spawn_signals():
            report = cleanup(base) if args.cleanup_only else recover(base)
    except BaseException as error:
        report = {'contractVersion': 'unified-auth-phase25-business-recovery/1', 'result': 'FAIL', 'failureCode': safe_code(error)}
        try:
            with base.spawn_signals(defer=True):
                report['cleanup'] = cleanup(base)
        except BaseException:
            report['cleanup'] = {'result': 'FAIL', 'failures': ['OWNED_CLEANUP_REQUIRES_RECONCILIATION']}
    print(json.dumps(report, sort_keys=True))
    return 0 if report['result'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
