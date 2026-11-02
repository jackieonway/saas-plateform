"""Owned Phase 25 business processes; importing this module performs no I/O.

The root controller uses Task.spawn for three namespace-bound supervisors.
Each supervisor starts exactly one attested binary as the task owner, records a
separate private lease, and pins its own child with pidfd until final cleanup.
No database, Docker, build, restart or gateway operation is implemented here.
"""
import argparse
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import select
import signal
import subprocess
import time
from urllib.parse import unquote, urlsplit


ENVIRONMENT = 'phase25-business-environment.json'
MANIFEST = 'phase25-business-manifest.json'
LEASE = 'phase25-business-lease.json'
SCRIPT = 'phase25-business-runtime.py'
RELATIVE_SCRIPT = 'saas-plateform/backend/tests/unified-auth-https/' + SCRIPT
SPECS = {
    'stores': {'binary': 'stores-api', 'prefix': 'STORES', 'port': 23031,
               'healthPath': '/stores-api/health/ready', 'databaseKey': 'STORES_DATABASE_URL',
               'code': 'stores', 'pairs': (('store', 'read'), ('store', 'create'), ('store', 'update'),
                   ('store-staff', 'read'), ('store-daily-report', 'read'),
                   ('store-daily-report', 'create'), ('store-daily-report', 'update'))},
    'orders': {'binary': 'work-orders-api', 'prefix': 'WORK_ORDERS', 'port': 23004,
               'healthPath': '/work-orders-api/health/ready', 'databaseKey': 'WORK_ORDERS_DATABASE_URL',
               'code': 'work-orders', 'pairs': (('ticket', 'read'), ('ticket', 'export'),
                   ('ticket', 'create'), ('ticket', 'update'), ('ticket', 'delete'), ('notification', 'publish'))},
    'notification': {'binary': 'saas-notification-ws', 'prefix': 'NOTIFICATION', 'port': 23003,
                     'healthPath': '/health/ready', 'databaseKey': 'DATABASE_URL'},
}
SUPERVISOR_FAILURE_CODES = frozenset('PHASE25_BUSINESS_' + code for code in (
    'FAILED', 'INTERRUPTED', 'SUPERVISOR_NAMESPACE_INVALID', 'FROZEN_RUNTIME_REQUIRED',
    'FROZEN_RUNTIME_DIGEST_INVALID', 'CHILD_LEASE_EXISTS', 'CHILD_START_BINDING_INVALID',
    'CHILD_BINARY_CHANGED', 'CHILD_EXITED', 'CHILD_STOP_TIMEOUT', 'LEASE_CHANGED',
    'PIDFD_REQUIRED', 'NAMESPACE_ENDED'))
APP_LOG_MAX_BYTES = 128 * 1024
APP_LOG_MAX_LINE = 8192
NATIVE_LOG_MAX_BYTES = 32 * 1024 * 1024
APP_SERVICE_CATEGORIES = {
    'platform authorization configuration is required': 'PLATFORM_CONFIGURATION',
    'platform authorization client initialization failed': 'PLATFORM_CLIENT_INITIALIZATION',
    'business database connection unavailable': 'DATABASE_UNAVAILABLE',
    'business database baseline verification failed': 'DATABASE_BASELINE_VERIFICATION',
    'business database startup budget exhausted': 'DATABASE_STARTUP_TIMEOUT',
    'HTTP listener unavailable': 'HTTP_LISTENER',
    'shutdown signal registration failed': 'SHUTDOWN_SIGNAL',
    'graceful shutdown budget exhausted': 'SHUTDOWN_TIMEOUT',
}
APP_CONFIG_CATEGORIES = {
    'metrics token must be a bounded non-whitespace credential': 'CONFIG_METRICS_TOKEN',
    'database TLS configuration contains an unsupported SSL mode': 'CONFIG_DATABASE_TLS',
    'PLATFORM_API_ORIGIN must be an exact HTTPS origin (development permits loopback HTTP)': 'CONFIG_PLATFORM_ORIGIN',
    'PLATFORM_CLIENT_ID must identify the application APP_RESOURCE service': 'CONFIG_PLATFORM_CLIENT_ID',
    'PLATFORM_CLIENT_SECRET must contain a valid confidential client secret': 'CONFIG_PLATFORM_CLIENT_SECRET',
    'PLATFORM_CA_CERT_FILE must identify a valid PEM certificate file': 'CONFIG_PLATFORM_CA',
}
APP_FAILURE_CATEGORIES = {
    'stores': {
        'stores_service_failed': {**APP_SERVICE_CATEGORIES,
            'imported business database admission failed': 'IMPORTED_DATABASE_ADMISSION'},
        'stores_configuration_failed': {**APP_CONFIG_CATEGORIES,
            'PLATFORM_AUTHORIZATION_MODE must be combined': 'CONFIG_AUTHORIZATION_MODE',
            'PLATFORM_IDENTITY_ISSUER must be an exact HTTPS origin (development permits loopback HTTP)': 'CONFIG_IDENTITY_ISSUER',
            'PLATFORM_RESOURCE_SCOPE_BINDINGS must map supported operations to reviewed OAuth scopes': 'CONFIG_RESOURCE_SCOPE_BINDINGS',
            'STORES_DATABASE_URL must explicitly identify the independent PostgreSQL database': 'CONFIG_DATABASE_URL',
            'STORES_DATABASE_INIT_MODE must be seed or imported; import requires stores-migrate --import': 'CONFIG_DATABASE_INIT_MODE',
            'STORES imported database receipt configuration is invalid': 'CONFIG_IMPORT_RECEIPT',
            'STORES_HOST must be a numeric bind address': 'CONFIG_HOST',
            'STORES_ALLOWED_ORIGINS must explicitly contain exact trusted origins; opaque null origins are not supported': 'CONFIG_ALLOWED_ORIGINS'},
    },
    'orders': {
        'work_orders_service_failed': APP_SERVICE_CATEGORIES,
        'work_orders_configuration_failed': {**APP_CONFIG_CATEGORIES,
            'WORK_ORDERS_DATABASE_INIT_MODE must be seed, import or imported': 'CONFIG_DATABASE_INIT_MODE',
            'IMPORT_REQUIRES_MIGRATE_ONLY: use work-orders-migrate --import': 'CONFIG_IMPORT_COMMAND',
            'IMPORTED_DATABASE_RECEIPT_INVALID: expected database, import marker and schema SHA256 are required only in imported mode': 'CONFIG_IMPORT_RECEIPT',
            'IMPORTED_DATABASE_TARGET_MISMATCH: database URL must match WORK_ORDERS_EXPECTED_DATABASE': 'CONFIG_IMPORT_TARGET',
            'PLATFORM_AUTHORIZATION_MODE must be canonical or absent': 'CONFIG_AUTHORIZATION_MODE',
            'PLATFORM_IDENTITY_ISSUER must be an exact HTTPS issuer origin': 'CONFIG_IDENTITY_ISSUER',
            'PLATFORM_RESOURCE_SCOPE_BINDINGS must map known operations to unique explicit OAuth scopes': 'CONFIG_RESOURCE_SCOPE_BINDINGS',
            'WORK_ORDERS_DATABASE_URL must explicitly identify the independent PostgreSQL database': 'CONFIG_DATABASE_URL',
            'WORK_ORDERS_HOST must be a numeric bind address': 'CONFIG_HOST',
            'WORK_ORDERS_ALLOWED_ORIGINS must contain exact HTTP(S) host origins; opaque null and wildcards are forbidden': 'CONFIG_ALLOWED_ORIGINS'},
    },
}
SAFE_APP_FAILURE_CODES = frozenset({'PHASE25_APP_DIAGNOSTIC_UNAVAILABLE'} | {
    'PHASE25_APP_' + code for events in APP_FAILURE_CATEGORIES.values()
    for categories in events.values() for code in categories.values()})


class RuntimeFailure(RuntimeError):
    def __init__(self, code):
        self.code = ('PHASE25_BUSINESS_' + code if type(code) is str
                     and re.fullmatch(r'[A-Z0-9_]+', code) else 'PHASE25_BUSINESS_FAILED')
        super().__init__(self.code)


class NativeLogCleanupFailure(RuntimeFailure):
    def __init__(self, code, audit):
        super().__init__(code)
        self.audit = audit


def need(value, code):
    if not value:
        raise RuntimeFailure(code)


def safe_failure(error):
    return error.code if isinstance(error, RuntimeFailure) else 'PHASE25_BUSINESS_FAILED'


def failure_result(error):
    result = {'status': 'FAIL', 'failureCode': safe_failure(error)}
    code = getattr(error, 'supervisorFailureCode', None)
    if type(code) is str and code in SUPERVISOR_FAILURE_CODES:
        result['supervisorFailureCode'] = code
    service = getattr(error, 'failedService', None)
    app_code = getattr(error, 'safeAppFailureCode', None)
    if type(service) is str and service in SPECS:
        result['service'] = service
        if type(app_code) is str and app_code in SAFE_APP_FAILURE_CODES:
            result['safeAppFailureCode'] = app_code
    if isinstance(error, NativeLogCleanupFailure):
        result['nativeLogAudit'] = error.audit
    return result


def app_failure_bytes(raw, name):
    unknown = 'PHASE25_APP_DIAGNOSTIC_UNAVAILABLE'
    events = APP_FAILURE_CATEGORIES.get(name)
    if events is None or type(raw) is not bytes or len(raw) > APP_LOG_MAX_BYTES:
        return unknown
    for line in reversed(raw.splitlines(keepends=True)):
        if not line.endswith(b'\n') or not 0 < len(line) <= APP_LOG_MAX_LINE:
            continue
        try:
            value = decode(line)
        except RuntimeFailure:
            continue
        fields = value.get('fields') if type(value) is dict else None
        if type(fields) is not dict or set(fields) != {'event', 'category'}:
            continue
        event, category = fields['event'], fields['category']
        if type(event) is str and type(category) is str and category in events.get(event, {}):
            return 'PHASE25_APP_' + events[event][category]
    return unknown


def app_failure_code(task, name):
    """Read a protected bounded log; only exact production categories escape.

    Never parse raw stderr text, reasons, source chains, suffixes or numeric
    configuration messages. Unknown, oversized or changing logs stay generic.
    """
    unknown = 'PHASE25_APP_DIAGNOSTIC_UNAVAILABLE'
    if name not in APP_FAILURE_CATEGORIES:
        return unknown
    try:
        raw = task.read(task.state_path.parent / ('phase25-business-' + name + '.log'),
                        maximum=APP_LOG_MAX_BYTES, empty_ok=True)
        return app_failure_bytes(raw, name)
    except (OSError, RuntimeError, TypeError, ValueError, KeyError):
        pass
    return unknown


def supervisor_exited(task, name):
    """Retain only an allowlisted fixed code, never a supervisor's raw output."""
    error = RuntimeFailure('SUPERVISOR_EXITED')
    error.failedService = service_name(name)
    error.safeAppFailureCode = app_failure_code(task, name)
    try:
        raw = task.read(task.state_path.parent / ('phase23-business-' + service_name(name) + '.log'),
                        maximum=8192, empty_ok=True)
        for line in reversed(raw.splitlines()):
            try:
                value = json.loads(line)
            except (ValueError, UnicodeError):
                continue
            if (type(value) is dict and set(value) == {'status', 'failureCode'}
                    and value['status'] == 'FAIL' and value['failureCode'] in SUPERVISOR_FAILURE_CODES):
                error.supervisorFailureCode = value['failureCode']
                break
    except (OSError, RuntimeError, TypeError, ValueError):
        pass
    return error


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        need(key not in result, 'DUPLICATE_JSON_FIELD')
        result[key] = value
    return result


def decode(raw):
    try:
        return json.loads(raw, object_pairs_hook=unique_object)
    except RuntimeFailure:
        raise
    except (ValueError, TypeError, UnicodeError):
        raise RuntimeFailure('JSON_INVALID') from None


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def base_module():
    # Deliberately lazy: pure component tests can import on Windows without pwd.
    path = Path(__file__).with_name('phase23-environment.py')
    loader = importlib.util.spec_from_file_location('phase25_business_base', path)
    module = importlib.util.module_from_spec(loader)
    loader.loader.exec_module(module)
    return module


def service_name(name):
    need(name in SPECS, 'SERVICE_INVALID')
    return name


def native_path(task, name):
    service_name(name)
    return task.state_path.parent / ('phase25-business-' + name + '.pid')


def validate(task, value, manifest, raw, source, resources, snapshot, attestation):
    """Pure cross-document checks. File ownership/read checks stay with Task."""
    state = task.state
    need(type(state) is dict and state.get('mode') == 'SOURCE' and state.get('prepared') is True
         and re.fullmatch(r'phase25-tools-[a-z0-9-]+', state.get('snapshot', '')),
         'PREPARED_SOURCE_TOOLS_REQUIRED')
    need(type(value) is dict and value.get('contractVersion') == 'unified-auth-phase25-business-environment/1'
         and type(manifest) is dict
         and manifest.get('contractVersion') == 'unified-auth-phase25-business-manifest/1', 'CONTRACT_INVALID')
    need(manifest.get('prepared') is True and manifest.get('baseEnvironmentUpdated') is True,
         'BUSINESS_PREPARATION_INCOMPLETE')
    need(value.get('clusterId') == manifest.get('clusterId') == task.cluster == resources['clusterId']
         and value.get('deploymentId') == manifest.get('deploymentId') == resources['deploymentId']
         == source['ids']['deployment']
         and value.get('baseDatabase') == manifest.get('baseDatabase') == resources['source']['database'],
         'DEPLOYMENT_BINDING_INVALID')
    base_database = value['baseDatabase']
    need(type(base_database) is str and re.fullmatch(r'unified_auth_https_[0-9a-f]{32}', base_database),
         'BASE_DATABASE_SCOPE_INVALID')
    token = base_database[-32:]
    role_prefix = 'uah25_' + token[-12:]
    need(manifest.get('rolePrefix') == role_prefix
         and manifest.get('baseRolePrefix') == 'uah_' + token[-12:]
         and manifest.get('baseMarker') == 'identity-deployment/1/' + value['deploymentId']
         and type(manifest.get('fixtureId')) is str
         and re.fullmatch(r'[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}',
                          manifest['fixtureId']), 'MANIFEST_SCOPE_INVALID')
    build = state.get('buildSnapshot', state['snapshot'])
    need(build.startswith('phase25-') and value.get('snapshot') == manifest.get('snapshot') == build
         and value.get('snapshotSha256') == manifest.get('snapshotSha256') == attestation['snapshotSha256']
         and attestation['snapshotLabel'] == build
         and snapshot.get('includeAllProjects') is True, 'FULL_CANDIDATE_BINDING_INVALID')
    need(manifest.get('environmentSha256') == sha(raw), 'ENVIRONMENT_DIGEST_INVALID')
    need(type(value.get('services')) is dict and set(value['services']) == set(SPECS)
         and type(manifest.get('services')) is dict and set(manifest['services']) == set(SPECS),
         'SERVICE_SET_INVALID')
    need(value.get('platformOverrides') == {'DATA_GRANT_POLICY': source['platform']['DATA_GRANT_POLICY']},
         'PLATFORM_POLICY_DRIFT')
    planned = {}
    for name, spec in SPECS.items():
        record, owner = value['services'][name], manifest['services'][name]
        need(type(record) is dict and type(owner) is dict, 'SERVICE_ENDPOINT_INVALID')
        expected_database = base_database if name == 'notification' else 'unified_auth_phase25_' + name + '_' + token
        need(owner.get('database') == expected_database and owner.get('runtime') == role_prefix + '_' + name + '_runtime'
             and owner.get('binary') == spec['binary'] and owner.get('host') == '127.0.0.1'
             and owner.get('port') == spec['port'] and owner.get('healthPath') == spec['healthPath'],
             'MANIFEST_SERVICE_SCOPE_INVALID')
        if name != 'notification':
            need(owner.get('owner') == role_prefix + '_' + name + '_owner'
                 and owner.get('marker') == 'unified-auth-phase25-business/1/' + value['deploymentId'] + '/' + name,
                 'MANIFEST_DATABASE_SCOPE_INVALID')
        need(type(record) is dict and type(owner) is dict
             and record.get('binary') == spec['binary']
             and record.get('host') == '127.0.0.1' and type(record.get('port')) is int
             and record['port'] == spec['port'] and record.get('healthPath') == spec['healthPath'],
             'SERVICE_ENDPOINT_INVALID')
        expected_sha = attestation['binaries'].get(spec['binary'])
        need(type(expected_sha) is str and re.fullmatch(r'[0-9a-f]{64}', expected_sha)
             and owner.get('binarySha256') == expected_sha, 'BINARY_ATTESTATION_INVALID')
        environment = record.get('environment')
        need(type(environment) is dict and 1 <= len(environment) <= 100
             and all(type(k) is str and re.fullmatch(r'[A-Z][A-Z0-9_]{0,80}', k)
                     and type(v) is str and len(v) <= 1024 * 1024 and '\0' not in v
                     for k, v in environment.items()), 'SERVICE_ENVIRONMENT_INVALID')
        forbidden = {'PATH', 'HOME', 'ENV', 'BASH_ENV', 'CDPATH', 'IFS', 'GCONV_PATH',
                     'LOCPATH', 'NLSPATH', 'SHELLOPTS', 'NODE_OPTIONS', 'RUSTC_WRAPPER'}
        need(not any(k in forbidden or k.startswith(('LD_', 'DYLD_', 'PYTHON'))
                     or 'PROXY' in k for k in environment), 'ENVIRONMENT_OVERRIDE_REJECTED')
        prefix = spec['prefix']
        need(environment.get(prefix + '_HOST') == '127.0.0.1'
             and environment.get(prefix + '_PORT') == str(spec['port']), 'ENVIRONMENT_BIND_INVALID')
        database_keys = {'DATABASE_URL', 'STORES_DATABASE_URL', 'WORK_ORDERS_DATABASE_URL'}
        need(database_keys.intersection(environment) == {spec['databaseKey']}, 'CROSS_SERVICE_DATABASE_REJECTED')
        try:
            database = urlsplit(environment[spec['databaseKey']])
            good_db = (database.scheme in ('postgres', 'postgresql') and database.hostname == '127.0.0.1'
                       and database.port == 55494 and database.path == '/' + owner['database']
                       and unquote(database.username or '') == owner['runtime'] and database.password
                       and not database.query and not database.fragment)
        except (KeyError, TypeError, ValueError):
            good_db = False
        need(good_db, 'DATABASE_BINDING_INVALID')
        if name == 'notification':
            need(environment.get('PLATFORM_API_BASE_URL') == 'https://console.saas.test:24444'
                 and environment.get('NOTIFICATION_SOCKET_PATH') == '/socket.io/'
                 and environment.get('NOTIFICATION_ENV') == 'test'
                 and environment.get('NOTIFICATION_ALLOWED_ORIGINS') == 'https://console.saas.test',
                 'AUTHORITY_BINDING_INVALID')
            ca = environment.get('NOTIFICATION_PLATFORM_CA_FILE')
        else:
            need(environment.get('PLATFORM_API_ORIGIN') == 'https://console.saas.test:24444'
                 and environment.get('PLATFORM_IDENTITY_ISSUER') == 'https://auth.saas.test'
                 and environment.get('PLATFORM_AUTHORIZATION_MODE') == ('combined' if name == 'stores' else 'canonical')
                 and environment.get(prefix + '_ALLOWED_ORIGINS') == 'https://console.saas.test'
                 and environment.get('PLATFORM_CLIENT_ID') == 'phase25-' + name + '-' + manifest['fixtureId'][-12:]
                 and type(environment.get('PLATFORM_CLIENT_SECRET')) is str
                 and re.fullmatch(r'[0-9a-f]{64}', environment['PLATFORM_CLIENT_SECRET']), 'AUTHORITY_BINDING_INVALID')
            need(decode(environment.get('PLATFORM_RESOURCE_SCOPE_BINDINGS')) == {
                resource + ':' + action: spec['code'] + ':' + resource + ':' + action
                for resource, action in spec['pairs']}, 'RESOURCE_SCOPE_BINDING_INVALID')
            ca = environment.get('PLATFORM_CA_CERT_FILE')
        need(ca == source['identity']['CONTEXT_PLATFORM_CA_FILE'], 'CA_BINDING_INVALID')
        planned[name] = {**record, 'environment': {**environment,
            'PATH': '/usr/lib/postgresql/18/bin:/usr/bin:/bin',
            'HOME': str(task.state_path.parent), 'LANG': 'C.UTF-8'}, 'binarySha256': expected_sha}
    return planned


def configuration(task):
    directory = task.state_path.parent
    raw = task.read(directory / ENVIRONMENT)
    manifest_raw = task.read(directory / MANIFEST)
    value, manifest = decode(raw), decode(manifest_raw)
    lock = directory / 'phase25-business-prepare.lock'
    need(not lock.exists() and not lock.is_symlink(), 'BUSINESS_PREPARATION_INCOMPLETE')
    source = task.json(directory / 'https-environment.json')
    snapshot_path, snapshot, attestation = task.snapshot(all_inputs=False)
    need(Path(__file__).resolve() == snapshot_path / RELATIVE_SCRIPT, 'FROZEN_RUNTIME_REQUIRED')
    need(snapshot['inputs'].get(RELATIVE_SCRIPT) == sha(Path(__file__).read_bytes()),
         'FROZEN_RUNTIME_DIGEST_INVALID')
    task.environment_binding(source, 'SOURCE')
    services = validate(task, value, manifest, raw, source, task.resources(), snapshot, attestation)
    for name, service in services.items():
        binary = directory / 'bin' / SPECS[name]['binary']
        need(task.digest(binary) == service['binarySha256'], 'BINARY_DIGEST_INVALID')
        task.checked(Path(service['environment'][
            'NOTIFICATION_PLATFORM_CA_FILE' if name == 'notification' else 'PLATFORM_CA_CERT_FILE']))
    return services, {'contractVersion': 'unified-auth-phase25-business-processes/1',
        'clusterId': task.cluster, 'deploymentId': value['deploymentId'], 'snapshot': value['snapshot'],
        'snapshotSha256': value['snapshotSha256'], 'environmentSha256': sha(raw),
        'manifestSha256': sha(manifest_raw), 'runtimeSha256': sha(Path(__file__).read_bytes())}


def process_identity(pid):
    proc = Path('/proc', str(pid))
    fields = (proc / 'stat').read_text().rsplit(')', 1)[1].split()
    return {'pid': pid, 'startTime': fields[19], 'uid': proc.stat().st_uid,
            'namespace': os.readlink(proc / 'ns/mnt'), 'binary': os.readlink(proc / 'exe')}


def same_process(record, actual):
    return all(record.get(key) == actual.get(key)
               for key in ('pid', 'startTime', 'uid', 'namespace', 'binary'))


def require_pidfd():
    need(hasattr(os, 'pidfd_open') and hasattr(signal, 'pidfd_send_signal'), 'PIDFD_REQUIRED')


def stop_descriptor(fd):
    """Only a pinned original process; never signal a numeric PID."""
    for signum, timeout in ((signal.SIGTERM, 1.5), (signal.SIGKILL, .8)):
        try:
            signal.pidfd_send_signal(fd, signum)
        except ProcessLookupError:
            return
        if select.select([fd], [], [], timeout)[0]:
            return
    raise RuntimeFailure('CHILD_STOP_TIMEOUT')


def stop_native(task, record, name):
    """Recover one orphan lease, validating before AND after pidfd_open."""
    require_pidfd()
    directory = task.state_path.parent
    need(record.get('contractVersion') == 'unified-auth-phase25-business-child/1'
         and record.get('service') == service_name(name)
         and record.get('binary') == str(directory / 'bin' / SPECS[name]['binary'])
         and record.get('uid') == task.uid and type(record.get('pid')) is int and record['pid'] > 1
         and re.fullmatch(r'[0-9]+', record.get('startTime', ''))
         and re.fullmatch(r'mnt:\[[0-9]+\]', record.get('namespace', '')), 'CHILD_RECORD_INVALID')
    try:
        before = process_identity(record['pid'])
    except (FileNotFoundError, ProcessLookupError):
        return
    need(same_process(record, before), 'CHILD_IDENTITY_CHANGED')
    try:
        fd = os.pidfd_open(record['pid'])
    except ProcessLookupError:
        return
    try:
        try:
            after = process_identity(record['pid'])
        except (FileNotFoundError, ProcessLookupError):
            return
        need(same_process(record, after), 'CHILD_IDENTITY_CHANGED')
        need(task.digest(Path(record['binary'])) == record.get('binarySha256'), 'CHILD_BINARY_CHANGED')
        stop_descriptor(fd)
    finally:
        os.close(fd)


def remove_lease(task, path, record):
    if path.exists() or path.is_symlink():
        need(decode(task.read(path)) == record, 'LEASE_CHANGED')
        path.unlink()


def supervisor_namespace(task, base):
    """The owner can inspect the root controller identity, but not its ns link.

    Root validated that link before nsenter. The child binds its own namespace
    to the same protected lease, exact controller argv and unreused root PID.
    """
    directory = task.state_path.parent
    try:
        record = task.json(directory / 'namespace.pid', private=False)
        identity = task.read(directory / 'namespace.identity', private=False).decode().strip()
        need(type(record) is dict and set(record) == {'pid', 'startTime', 'namespace'}
             and type(record['pid']) is int and record['pid'] > 1
             and type(record['startTime']) is str and re.fullmatch(r'[0-9]+', record['startTime'])
             and type(record['namespace']) is str and re.fullmatch(r'mnt:\[[0-9]+\]', record['namespace'])
             and identity == record['namespace'], 'SUPERVISOR_NAMESPACE_INVALID')
        proc = Path('/proc', str(record['pid']))
        need(proc.stat().st_uid == 0 and base.start_time(record['pid']) == record['startTime']
             and str(directory / 'environment-controller.sh').encode()
                 in (proc / 'cmdline').read_bytes().split(b'\0')
             and task.json(directory / 'namespace.pid', private=False) == record,
             'SUPERVISOR_NAMESPACE_INVALID')
        return record['namespace']
    except (OSError, KeyError, TypeError, ValueError, UnicodeError):
        raise RuntimeFailure('SUPERVISOR_NAMESPACE_INVALID') from None


def supervise(task, base, name):
    require_pidfd()
    name = service_name(name)
    services, binding = configuration(task)
    namespace = supervisor_namespace(task, base)
    need(os.readlink('/proc/self/ns/mnt') == namespace and os.getuid() == task.uid, 'SUPERVISOR_NAMESPACE_INVALID')
    service = services[name]
    path = native_path(task, name)
    need(not path.exists() and not path.is_symlink(), 'CHILD_LEASE_EXISTS')
    binary = task.state_path.parent / 'bin' / service['binary']
    logfd = task.open_log(task.state_path.parent / ('phase25-business-' + name + '.log'))
    child, descriptor, record = None, None, None

    def interrupt(*_args):
        raise SystemExit(0)

    old_handlers = {sig: signal.signal(sig, interrupt) for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP)}
    try:
        with base.spawn_signals(defer=True):
            child = subprocess.Popen([str(binary)], cwd=task.state_path.parent,
                env=service['environment'], stdout=logfd, stderr=subprocess.STDOUT, shell=False)
        descriptor = os.pidfd_open(child.pid)
        actual = process_identity(child.pid)
        need(actual['uid'] == task.uid and actual['namespace'] == namespace
             and actual['binary'] == str(binary), 'CHILD_START_BINDING_INVALID')
        need(task.digest(binary) == service['binarySha256'], 'CHILD_BINARY_CHANGED')
        record = {'contractVersion': 'unified-auth-phase25-business-child/1', 'service': name,
                  **actual, 'binarySha256': service['binarySha256'], **binding,
                  'supervisorPid': os.getpid(), 'supervisorStartTime': base.start_time(os.getpid())}
        # The child-specific contract must remain distinct from aggregate binding.
        record['contractVersion'] = 'unified-auth-phase25-business-child/1'
        task.write(path, record)
        while True:
            exited = select.select([descriptor], [], [], .2)[0]
            need(not exited, 'CHILD_EXITED')
            need((task.state_path.parent / 'namespace.pid').exists(), 'NAMESPACE_ENDED')
    finally:
        # Root Task.stop_spawn_group gives supervisors five seconds. Each native
        # child gets one shared short TERM/KILL budget and lease removal follows.
        for sig in old_handlers:
            signal.signal(sig, signal.SIG_IGN)
        try:
            if child is not None:
                if descriptor is None:
                    # Popen has not been waited/polled: an unreaped child keeps
                    # its original numeric PID reserved across this pin.
                    try:
                        descriptor = os.pidfd_open(child.pid)
                    except ProcessLookupError:
                        pass
                if descriptor is not None:
                    stop_descriptor(descriptor)
                child.wait(timeout=.5)
            if record is not None:
                remove_lease(task, path, record)
        finally:
            if descriptor is not None:
                os.close(descriptor)
            os.close(logfd)
            for sig, handler in old_handlers.items():
                signal.signal(sig, handler)


def stop_one(task, name):
    path = native_path(task, name)
    errors = []
    record = None
    try:
        record = decode(task.read(path)) if path.exists() or path.is_symlink() else None
    except BaseException:
        errors.append('native')
    key = 'business-' + name
    try:
        pending = (task.state or {}).get('pendingSpawns', {})
        need(type(pending) is dict, 'PENDING_SPAWN_INVALID')
        journal_path = task.state_path.parent / ('phase23-spawn-' + key + '.json')
        journal = decode(task.read(journal_path)) if journal_path.exists() or journal_path.is_symlink() else None
        pending_record = pending.get(key)
        need(journal is None or pending_record is None or journal == pending_record, 'PENDING_SPAWN_CHANGED')
        incomplete = journal if journal is not None else pending_record
        if incomplete is not None:
            need(incomplete.get('contractVersion') == 'phase23-spawn-ownership/1'
                 and incomplete.get('key') == key and incomplete.get('cleanupRequired') is True
                 and incomplete.get('uid') == task.uid
                 and incomplete.get('script') == str(task.tools() / SCRIPT), 'PENDING_SPAWN_INVALID')
            task.stop_spawn_group(incomplete)
            if journal is not None:
                remove_lease(task, journal_path, journal)
            pending.pop(key, None)
            task.save()
    except BaseException:
        errors.append('pending')
    tracked = (task.state or {}).get('processes', {}).get(key)
    if tracked is not None:
        try:
            need(tracked.get('pgid') == tracked.get('pid')
                 and tracked.get('uid') == task.uid
                 and tracked.get('script') == str(task.tools() / SCRIPT), 'SUPERVISOR_RECORD_INVALID')
            task.stop_process(key)
        except BaseException:
            errors.append('tracked')
    if record is not None and (path.exists() or path.is_symlink()):
        try:
            stop_native(task, record, name)
            remove_lease(task, path, record)
        except BaseException:
            errors.append('native-stop')
    need(not errors, 'SERVICE_CLEANUP_REQUIRED')


def native_log_canaries(task):
    """Protected actual service values; returned bytes must remain in memory."""
    filename = task.state_path.parent / ENVIRONMENT
    if not filename.exists() and not filename.is_symlink():
        return None
    raw = task.read(filename, maximum=1024 * 1024)
    value = decode(raw)
    need(type(value) is dict and value.get('contractVersion') == 'unified-auth-phase25-business-environment/1'
         and type(value.get('services')) is dict and set(value['services']) == set(SPECS),
         'NATIVE_LOG_ENVIRONMENT_INVALID')
    candidates = set()
    for name, spec in SPECS.items():
        entry = value['services'][name]
        need(type(entry) is dict and type(entry.get('environment')) is dict, 'NATIVE_LOG_ENVIRONMENT_INVALID')
        values = entry['environment']
        need(all(type(key) is str and type(item) is str for key, item in values.items()), 'NATIVE_LOG_ENVIRONMENT_INVALID')
        url = values.get(spec['databaseKey'])
        need(type(url) is str and url, 'NATIVE_LOG_ENVIRONMENT_INVALID')
        parsed = urlsplit(url)
        need(parsed.scheme in ('postgres', 'postgresql') and parsed.password, 'NATIVE_LOG_ENVIRONMENT_INVALID')
        candidates.update(item.encode() for item in (url, parsed.password, unquote(parsed.password)))
        for key, item in values.items():
            if item and re.search(r'secret|password|token', key, re.I):
                candidates.add(item.encode())
        if name != 'notification':
            client, secret = values.get('PLATFORM_CLIENT_ID'), values.get('PLATFORM_CLIENT_SECRET')
            need(type(client) is str and client and type(secret) is str and secret, 'NATIVE_LOG_ENVIRONMENT_INVALID')
            candidates.add(base64.b64encode((client + ':' + secret).encode()))
        metrics = values.get('METRICS_TOKEN' if name == 'notification' else spec['prefix'] + '_METRICS_TOKEN')
        need(type(metrics) is str and metrics, 'NATIVE_LOG_ENVIRONMENT_INVALID')
    return candidates


def cleanup_native_logs(task, *, allow_unscanned_legacy=False):
    """Called only after every owned business process is confirmed stopped.

    The explicit legacy option is for the already audited r1/r2 leftover only;
    normal stop refuses to erase native logs whose ENV has disappeared.
    """
    directory = task.state_path.parent
    paths = [(name, directory / ('phase25-business-' + name + '.log')) for name in SPECS]
    paths = [(name, path) for name, path in paths if path.exists() or path.is_symlink()]
    audit = {'status': 'PASS', 'nativeLogsFound': len(paths), 'nativeLogsRemoved': 0,
        'nativeLogBytesRemoved': 0, 'canaryScanComplete': not paths,
        'canaryCoverage': 'NOT_REQUIRED_NO_NATIVE_LOGS', 'privateCandidateCount': 0,
        'candidateFindings': 0 if not paths else None, 'rawLogContentsPersisted': False, 'logs': []}
    if not paths:
        return audit
    try:
        candidates = native_log_canaries(task)
        if candidates is None:
            audit.update({'canaryCoverage': 'UNAVAILABLE_ENVIRONMENT_ABSENT',
                          'status': 'PASS_WITH_LIMITED_CANARY_COVERAGE'})
        else:
            audit.update({'canaryCoverage': 'ACTUAL_PROTECTED_BUSINESS_ENVIRONMENT',
                'privateCandidateCount': len(candidates), 'candidateFindings': 0, 'canaryScanComplete': True})
        for name, path in paths:
            raw = task.read(path, maximum=NATIVE_LOG_MAX_BYTES, empty_ok=True)
            record = {'service': name, 'name': path.name, 'bytes': len(raw), 'sha256': sha(raw),
                      'safeAppFailureCode': app_failure_bytes(raw, name)}
            audit['logs'].append(record)
            if candidates is not None:
                audit['candidateFindings'] += sum(candidate in raw for candidate in candidates)
        if candidates is None and not allow_unscanned_legacy:
            raise RuntimeFailure('NATIVE_LOG_ENVIRONMENT_REQUIRED')
        if audit['candidateFindings']:
            raise RuntimeFailure('NATIVE_LOG_PRIVATE_CANDIDATE_FOUND')
        for (_, path), record in zip(paths, audit['logs']):
            task.checked(path, empty_ok=True, maximum=NATIVE_LOG_MAX_BYTES)
            need(task.digest(path, maximum=NATIVE_LOG_MAX_BYTES) == record['sha256'], 'NATIVE_LOG_CHANGED')
            path.unlink()
            audit['nativeLogsRemoved'] += 1
            audit['nativeLogBytesRemoved'] += record['bytes']
    except Exception as error:
        audit['status'] = 'FAIL'
        code = (error.code.removeprefix('PHASE25_BUSINESS_') if isinstance(error, RuntimeFailure)
                else 'NATIVE_LOG_AUDIT_OR_DELETE_FAILED')
        raise NativeLogCleanupFailure(code, audit) from None
    return audit


def stop(task, *, allow_unscanned_legacy=False):
    failures = []
    for name in reversed(SPECS):
        try:
            stop_one(task, name)
        except BaseException:
            failures.append(name)
    need(not failures, 'CLEANUP_REQUIRED')
    path = task.state_path.parent / LEASE
    if path.exists() or path.is_symlink():
        record = decode(task.read(path))
        need(record.get('contractVersion') == 'unified-auth-phase25-business-processes/1'
             and set(record.get('services', {})) == set(SPECS), 'AGGREGATE_LEASE_INVALID')
        remove_lease(task, path, record)
    return {'status': 'PHASE25_BUSINESS_STOPPED', 'servicesRemaining': 0,
            'nativeLogAudit': cleanup_native_logs(task, allow_unscanned_legacy=allow_unscanned_legacy)}


def start(task):
    services, binding = configuration(task)
    directory = task.state_path.parent
    need(not (directory / LEASE).exists() and not (directory / LEASE).is_symlink(), 'AGGREGATE_LEASE_EXISTS')
    need(all(not native_path(task, name).exists() and not native_path(task, name).is_symlink()
             and 'business-' + name not in task.state['processes']
             and 'business-' + name not in task.state.get('pendingSpawns', {})
             and not (directory / ('phase23-spawn-business-' + name + '.json')).exists()
             and not (directory / ('phase23-spawn-business-' + name + '.json')).is_symlink()
             for name in SPECS), 'BUSINESS_ALREADY_REGISTERED')
    namespace = task.namespace()
    script = task.tools() / SCRIPT
    started = []
    try:
        for name in SPECS:
            # Record before spawn: Task.spawn owns any partial launch journal.
            started.append(name)
            task.spawn('business-' + name, task.ns_command([
                'python3', '-B', script, '--task-dir', directory, 'supervise', '--service', name]), script, task.uid)
            deadline = time.monotonic() + 20
            while not native_path(task, name).exists():
                if not task.alive(task.state['processes']['business-' + name]):
                    raise supervisor_exited(task, name)
                need(time.monotonic() < deadline, 'CHILD_RECORD_TIMEOUT')
                time.sleep(.1)
            record = decode(task.read(native_path(task, name)))
            need(record.get('namespace') == namespace['namespace']
                 and record.get('environmentSha256') == binding['environmentSha256']
                 and record.get('manifestSha256') == binding['manifestSha256']
                 and record.get('binarySha256') == services[name]['binarySha256']
                 and same_process(record, process_identity(record['pid'])), 'CHILD_RECORD_BINDING_INVALID')
            deadline = time.monotonic() + 20
            spec = SPECS[name]
            while True:
                result = subprocess.run(task.ns_command(['curl', '--noproxy', '*', '--silent',
                    '--max-time', '1', '--output', '/dev/null', '--write-out', '%{http_code}',
                    'http://127.0.0.1:' + str(spec['port']) + spec['healthPath']]),
                    capture_output=True, env={'PATH': '/usr/bin:/bin'}, timeout=3)
                if result.returncode == 0 and result.stdout == b'200':
                    break
                if not task.alive(task.state['processes']['business-' + name]):
                    raise supervisor_exited(task, name)
                need(time.monotonic() < deadline, 'READINESS_FAILED')
                time.sleep(.1)
        task.write(directory / LEASE, {**binding,
            'services': {name: decode(task.read(native_path(task, name))) for name in SPECS}})
    except BaseException as error:
        if started and isinstance(error, Exception):
            error.failedService = started[-1]
            error.safeAppFailureCode = app_failure_code(task, started[-1])
        failures = []
        for name in reversed(started):
            try:
                stop_one(task, name)
            except BaseException:
                failures.append(name)
        need(not failures, 'START_ROLLBACK_REQUIRED')
        raise
    return {'status': 'PHASE25_BUSINESS_READY', 'services': 3,
            'readiness': {name: 200 for name in SPECS}, 'environmentSha256': binding['environmentSha256']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--task-dir', type=Path, required=True)
    parser.add_argument('action', choices=('start', 'stop', 'status', 'supervise'))
    parser.add_argument('--service', choices=tuple(SPECS))
    args = parser.parse_args()
    try:
        base = base_module()
        need(args.task_dir == base.TASK, 'TASK_ARGUMENT_INVALID')
        task = base.Task(require_root=args.action != 'supervise')
        if args.action == 'start':
            result = start(task)
        elif args.action == 'stop':
            result = stop(task)
        elif args.action == 'supervise':
            need(args.service is not None, 'SERVICE_INVALID')
            supervise(task, base, args.service)
            return
        else:
            result = {'status': 'PHASE25_BUSINESS_STATUS', 'trackedSupervisors':
                sum('business-' + name in task.state['processes'] for name in SPECS),
                'nativeLeases': sum(native_path(task, name).exists() for name in SPECS),
                'aggregateLease': (task.state_path.parent / LEASE).exists()}
        print(json.dumps(result))
    except KeyboardInterrupt:
        print(json.dumps({'status': 'FAIL', 'failureCode': 'PHASE25_BUSINESS_INTERRUPTED'}))
        raise SystemExit(1) from None
    except Exception as error:
        print(json.dumps(failure_result(error)))
        raise SystemExit(1) from None


if __name__ == '__main__':
    main()
