"""Own and finally remove one native Phase 22 validation environment.

Run as WSL root only to create the private mount namespace. Application,
database fixture and Node work run as the task OWNER's unprivileged uid.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import pwd
import re
import shutil
import subprocess
import time

import phase23_cluster_binding as cluster_binding


TASK = Path('/tmp/rust-saas-unified-auth-20261028')
WORKSPACE = Path('/mnt/d/WorkSpace/rust/rust-saas')
VALIDATION = WORKSPACE / 'saas-plateform/backend/docs/designs/unified-auth/validation'
ADMIN = 'postgres://postgres@127.0.0.1:55494/postgres'


def options(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument('--phase', choices=(22, 25), type=int, default=22)
    parser.add_argument('--label')
    parser.add_argument('--build-label', help='Attestation snapshot when only verification tools changed')
    parser.add_argument('--tls-workspace', type=Path, default=WORKSPACE)
    parser.add_argument('--faults-only', action='store_true', help='Phase 22 only: reuse exact-binary component evidence')
    args = parser.parse_args(argv)
    args.label = args.label or ('phase22-final-r1' if args.phase == 22 else None)
    if (not args.label or not re.fullmatch('phase' + str(args.phase) + r'-[a-z0-9-]+', args.label)
            or (args.build_label is not None and not re.fullmatch(
                'phase' + str(args.phase) + r'-[a-z0-9-]+', args.build_label))):
        raise RuntimeError('PHASE22_SNAPSHOT_LABEL_REJECTED')
    if args.phase == 25 and args.faults_only:
        raise RuntimeError('PHASE22_PHASE25_REQUIRES_FRESH_CAPACITY')
    return args


def candidate(args):
    """Reuse the full candidate verifier; it reads only owned files and hashes."""
    snapshot = TASK / ('source-snapshot-' + args.label)
    if args.phase != 25:
        return snapshot, None
    spec = importlib.util.spec_from_file_location('phase25_capacity_candidate',
        Path(__file__).with_name('phase23-environment.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    verified, manifest, attestation = module.Task().snapshot(args.label,
        build_label=args.build_label or args.label)
    if verified != snapshot:
        raise RuntimeError('PHASE22_PHASE25_SNAPSHOT_REJECTED')
    return snapshot, {'snapshotLabel': args.label, 'snapshotSha256': manifest['snapshotSha256'],
        'buildSnapshotLabel': attestation['snapshotLabel'], 'buildSnapshotSha256': attestation['snapshotSha256'],
        'builtServices': attestation['builtServices'], 'binaries': attestation['binaries']}


def capacity_controller(data):
    """Select the metrics gateway in this copied controller, with drift refusal."""
    default = b'gateway_script=${TASK_GATEWAY_SCRIPT:-gateway.py}'
    allowed = b'case "$gateway_script" in gateway.py|phase23-gateway.py)'
    if data.count(default) != 1 or data.count(allowed) != 1:
        raise RuntimeError('PHASE22_GATEWAY_CONTROLLER_CONTRACT_CHANGED')
    return data.replace(default, b'gateway_script=phase22-gateway.py\nexport PYTHONDONTWRITEBYTECODE=1').replace(
        allowed, b'case "$gateway_script" in phase22-gateway.py)')


def runtime_report(filename, uid, phase):
    raw, _ = cluster_binding.protected_read(filename, uid=uid, mode=0o600, maximum=8 * 1024 * 1024)
    value = json.loads(raw)
    if (type(value) is not dict or value.get('phase') != phase
            or value.get('result') not in ('PASS', 'FAILED')
            or (phase == 25 and (value.get('freshCapacityRequired') is not True
                or value.get('capacityEvidence', {}).get('reusedCompletedComponentEvidence')))):
        raise RuntimeError('PHASE22_RUNTIME_REPORT_REJECTED')
    return raw, value


def main():
    args = options()
    if (os.getuid() != 0 or TASK.resolve(strict=True) != TASK
            or not (TASK / 'OWNER').read_text().startswith(TASK.name + '-pg')
            or not re.fullmatch(r'phase(?:22|25)-[a-z0-9-]+', args.label)):
        raise RuntimeError('PHASE22_OWNER_REQUIRED')
    owned_cluster = cluster_binding.load_binding(TASK)
    cluster = owned_cluster['systemIdentifier']
    if any((TASK / name).exists() for name in ('https-database.json', 'namespace.pid', 'gateway.pid')):
        raise RuntimeError('PHASE22_REQUIRES_EMPTY_TLS_LEASE')
    owner = (TASK / 'OWNER').stat()
    username = pwd.getpwuid(owner.st_uid).pw_name
    if owner.st_uid == 0:
        raise RuntimeError('PHASE22_UNPRIVILEGED_OWNER_REQUIRED')
    snapshot, attested = candidate(args)
    source = ((snapshot if args.phase == 25 else WORKSPACE)
              / 'saas-plateform/backend/tests/unified-auth-https')
    tls_workspace = args.tls_workspace.resolve(strict=True)
    if (tls_workspace != WORKSPACE and (not tls_workspace.is_relative_to(TASK)
            or tls_workspace != args.tls_workspace)):
        raise RuntimeError('PHASE22_TLS_WORKSPACE_REJECTED')
    report_prefix = 'phase-22' if args.phase == 22 else 'phase-25-capacity'
    runtime_name = 'phase22-runtime.json' if args.phase == 22 else 'phase25-capacity-runtime.json'
    if (TASK / runtime_name).exists() or (TASK / runtime_name).is_symlink():
        raise RuntimeError('PHASE22_FRESH_RUNTIME_OUTPUT_REQUIRED')
    tools = TASK / 'phase22-tools'
    if tools.is_symlink() or (tools.exists() and (not tools.is_dir()
            or tools.resolve() != TASK / 'phase22-tools'
            or tools.stat().st_uid not in (0, owner.st_uid)
            or tools.stat().st_mode & 0o777 != 0o700)):
        raise RuntimeError('PHASE22_PUBLIC_TOOL_DIRECTORY_REJECTED')
    tools.mkdir(mode=0o700, exist_ok=True)
    os.chown(tools, owner.st_uid, owner.st_gid)
    for name in ('prepare.py', 'phase23_cluster_binding.py', 'phase22-fixture.py', 'gateway.py', 'phase22-gateway.py',
                 'environment.sh', 'run-service.py', 'phase22-runtime.mjs', 'phase22-load.mjs', 'phase22-alerts.mjs'):
        target = tools / name
        if target.is_symlink() or (target.exists() and (not target.is_file()
                or target.stat().st_uid not in (0, owner.st_uid) or target.stat().st_nlink != 1)):
            raise RuntimeError('PHASE22_PUBLIC_TOOL_FILE_REJECTED')
        data = (source / name).read_bytes()
        if name == 'environment.sh':
            data = capacity_controller(data)
        target.write_bytes(data)
        os.chown(target, owner.st_uid, owner.st_gid)
        target.chmod(0o600)
    os.chown(tools, owner.st_uid, owner.st_gid)
    before_files = {p.name for p in TASK.iterdir() if p.is_file()}
    before_df = subprocess.check_output(['docker', 'system', 'df'], text=True)
    report = {'contractVersion': 'unified-auth-phase22-cleanup/1', 'result': 'FAILED',
              'dockerContext': subprocess.check_output(['docker', 'context', 'show'], text=True).strip(),
              'dockerContainersCreated': 0, 'dockerContainersReused': 0,
              'dockerBefore': before_df, 'ownedApplications': ['identity', 'platform'],
              'filesystemBefore': subprocess.check_output(['df', '-B1', str(TASK)], text=True, timeout=15),
              'retainedSharedPostgresCluster': cluster, 'phase': args.phase,
              'freshCapacityRequired': args.phase == 25, 'candidate': attested,
              'reportPrefix': report_prefix, 'runtimeName': runtime_name}
    processes = []
    def user(command, **kwargs):
        return subprocess.run(['runuser', '-u', username, '--', *map(str, command)],
                              check=True, capture_output=True, timeout=kwargs.pop('timeout', 60), **kwargs)
    def control(action, service=None):
        command = ['bash', str(tools / 'environment.sh'), action, '--task-dir', str(TASK)]
        if service:
            command += ['--service', service]
        return subprocess.run(command, capture_output=True, timeout=35)
    stage = 'prepare'
    try:
        user(['python3', '-B', source / 'prepare.py', '--task-dir', TASK, '--admin-url', ADMIN,
              '--cluster-id', cluster, '--migration-bin', TASK / 'bin/platform-migrate',
              '--backend-source', snapshot / 'saas-plateform/backend',
              '--tls-ca-file', tls_workspace / '.local-run/ca.pem'])
        stage = 'phase22-inputs'
        user(['python3', '-B', tools / 'phase22-fixture.py', '--task-dir', TASK, '--admin-url', ADMIN])
        environment_file = TASK / 'https-environment.json'
        environment = json.loads(environment_file.read_text())
        fixture = json.loads((TASK / 'phase22-fixture.json').read_text())
        if args.faults_only:
            baseline = TASK / 'phase22-capacity-baseline.json'
            shutil.copyfile(VALIDATION / 'phase-22-capacity.json', baseline)
            os.chown(baseline, owner.st_uid, owner.st_gid)
            baseline.chmod(0o600)
        environment['identity'].update(fixture['identityOverrides'])
        # Normal login uses the production default. The runtime switches to a
        # short pressure deadline only at its already required owned restart.
        environment['identity']['PASSWORD_EXECUTION_TIMEOUT_MS'] = '10000'
        environment['identity']['LOGIN_VERIFY_BUDGET_MS'] = '10000'
        environment['identity'].pop('RUST_LOG', None)  # Exercise the production INFO default.
        environment_file.write_text(json.dumps(environment))
        environment_file.chmod(0o600)
        # Startup must refuse two explicitly unsafe filters before binding.
        stage = 'startup-warning-gate'
        startup = []
        for filter_value in ('error', 'error,saas_identity::delivery::worker=warn,saas_identity::password=warn'):
            result = subprocess.run(['/usr/sbin/runuser', '-u', username, '--', str(TASK / 'bin/saas-identity')],
                env={'PATH': '/usr/local/bin:/usr/bin:/bin', 'RUST_LOG': filter_value},
                capture_output=True, timeout=10)
            startup.append(result.returncode != 0 and b'IDENTITY_REQUIRED_WARN_LOGGING_DISABLED' in result.stderr)
        if startup != [True, True]:
            raise RuntimeError('PHASE22_STARTUP_WARNING_GATE_FAILED')
        report['startupUnsafeFiltersRejected'] = 2
        stage = 'namespace-and-services'
        for name, command in [('namespace', ['bash', str(tools / 'environment.sh'), 'start', '--task-dir', str(TASK), '--workspace', str(tls_workspace), '--static-workspace', str(snapshot)])]:
            log = (TASK / ('phase22-' + name + '.log')).open('wb')
            processes.append((subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT), log))
        deadline = time.monotonic() + 10
        while not (TASK / 'namespace.pid').exists():
            if time.monotonic() > deadline:
                raise RuntimeError('PHASE22_NAMESPACE_NOT_READY')
            time.sleep(.1)
        for service in ('identity',):
            log = (TASK / ('phase22-' + service + '-supervisor.log')).open('wb')
            command = ['bash', str(tools / 'environment.sh'), 'run', '--task-dir', str(TASK), '--service', service]
            processes.append((subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT), log))
        deadline = time.monotonic() + 15
        while True:
            result = subprocess.run(['curl', '--noproxy', '*', '--cacert', environment['identity']['CONTEXT_PLATFORM_CA_FILE'], '--resolve', 'auth.saas.test:24444:127.0.0.2', '--silent', '--fail', '--max-time', '1', 'https://auth.saas.test:24444/health/ready'], capture_output=True)
            if result.returncode == 0:
                break
            if time.monotonic() > deadline:
                raise RuntimeError('PHASE22_IDENTITY_NOT_READY')
            time.sleep(.1)
        log = (TASK / 'phase22-platform-supervisor.log').open('wb')
        processes.append((subprocess.Popen(['bash', str(tools / 'environment.sh'), 'run', '--task-dir', str(TASK), '--service', 'platform'], stdout=log, stderr=subprocess.STDOUT), log))
        deadline = time.monotonic() + 15
        while True:
            result = subprocess.run(['curl', '--noproxy', '*', '--cacert', environment['identity']['CONTEXT_PLATFORM_CA_FILE'], '--resolve', 'console.saas.test:443:127.0.0.2', '--silent', '--fail', '--max-time', '1', 'https://console.saas.test/auth/config'], capture_output=True)
            if result.returncode == 0:
                break
            if time.monotonic() > deadline:
                raise RuntimeError('PHASE22_PLATFORM_NOT_READY')
            time.sleep(.1)
        namespace = json.loads((TASK / 'namespace.pid').read_text())
        stage = 'runtime'
        node = TASK / 'phase21-node-runtime/bin/node'
        command = ['nsenter', '-t', str(namespace['pid']), '-m', '--', 'setpriv', '--reuid='+str(owner.st_uid), '--regid='+str(owner.st_gid), '--init-groups', str(node), str(tools / 'phase22-runtime.mjs'), str(TASK / runtime_name)]
        if args.faults_only:
            command.append('--faults-only')
        elif args.phase == 25:
            command.append('--phase25')
        runtime = subprocess.run(command, capture_output=True, timeout=260)
        report['runtimeExitCode'] = runtime.returncode
        if (TASK / runtime_name).is_file():
            raw, completed = runtime_report(TASK / runtime_name, owner.st_uid, args.phase)
            (VALIDATION / (report_prefix + '-runtime.json')).write_bytes(raw)
        else:
            raise RuntimeError('PHASE22_RUNTIME_REPORT_MISSING')
        report['result'] = 'PASS' if runtime.returncode == 0 and completed['result'] == 'PASS' else 'FAILED'
    except Exception as error:
        report['failure'] = 'PHASE22_ENVIRONMENT_VALIDATION_FAILED'
        report['failureStage'] = stage
        report['failureType'] = type(error).__name__
        if re.fullmatch(r'PHASE22_[A-Z0-9_]+', str(error)):
            report['failureCode'] = str(error)
        if isinstance(error, subprocess.CalledProcessError):
            lines = (error.stderr or b'').decode(errors='replace').splitlines()
            tail = lines[-1] if lines else ''
            if re.fullmatch(r'(RuntimeError|ModuleNotFoundError|FileNotFoundError): [A-Za-z0-9 _.-]{1,100}', tail):
                report['safeFailure'] = tail
    finally:
        cleanup_errors = []
        def step(code, action):
            try:
                return action()
            except BaseException:
                # Cleanup failures must not expose subprocess SQL, credentials,
                # exception objects, or stop cleanup of independent resources.
                if code not in cleanup_errors:
                    cleanup_errors.append(code)
                report['result'] = 'FAILED'
                return None

        def private_path(target):
            if target.parent.resolve() != TASK or target.is_symlink():
                raise RuntimeError('OWNED_PRIVATE_PATH_REJECTED')
            if target.exists() and (not target.is_file() or target.stat().st_uid not in (0, owner.st_uid)):
                raise RuntimeError('OWNED_PRIVATE_FILE_REJECTED')
            return target

        def service_record(service):
            target = private_path(TASK / ('service-' + service + '.pid'))
            if not target.exists():
                return None
            value = json.loads(target.read_text())
            expected = str(TASK / 'bin' / ('saas-identity' if service == 'identity' else 'saas-platform-backend'))
            if (not isinstance(value.get('pid'), int) or value['pid'] < 2
                    or value.get('binary') != expected or not isinstance(value.get('startTime'), str)
                    or not re.fullmatch(r'mnt:\[[0-9]+\]', value.get('namespace', ''))):
                raise RuntimeError('OWNED_PROCESS_RECORD_REJECTED')
            return target, value

        def matches(value):
            proc = Path('/proc') / str(value['pid'])
            try:
                if not (proc / 'exe').exists():
                    return False
                if (os.readlink(proc / 'exe') != value['binary']
                        or os.readlink(proc / 'ns/mnt') != value['namespace']
                        or (proc / 'stat').read_text().rsplit(')', 1)[1].split()[19] != value['startTime']):
                    raise RuntimeError('OWNED_PROCESS_IDENTITY_CHANGED')
                return True
            except FileNotFoundError:
                return False

        def resume(service):
            record = service_record(service)
            if record and matches(record[1]):
                os.kill(record[1]['pid'], 18)  # SIGCONT, only the verified task binary.

        def controlled(action, service=None):
            result = control(action, service)
            if result.returncode != 0:
                raise RuntimeError('OWNED_CONTROL_FAILED')

        def fallback_stop(service):
            record = service_record(service)
            if record is None:
                return
            target, value = record
            if matches(value):
                os.kill(value['pid'], 18)
                os.kill(value['pid'], 15)
                deadline = time.monotonic() + 8
                while matches(value) and time.monotonic() < deadline:
                    time.sleep(.1)
                if matches(value):
                    os.kill(value['pid'], 9)
                    deadline = time.monotonic() + 3
                    while matches(value) and time.monotonic() < deadline:
                        time.sleep(.1)
                if matches(value):
                    raise RuntimeError('OWNED_SERVICE_REMAINS')
            # A stopped supervisor normally removes its record within 1 second.
            deadline = time.monotonic() + 2
            while target.exists() and time.monotonic() < deadline:
                time.sleep(.1)
            if target.exists():
                current = service_record(service)
                if current is None or current[1] != value or matches(value):
                    raise RuntimeError('OWNED_SERVICE_RECORD_CHANGED')
                private_path(target).unlink()

        # Each step runs even after an earlier controller/proc operation fails.
        for service in ('identity', 'platform'):
            step('RESUME_' + service.upper() + '_FAILED', lambda service=service: resume(service))
            if (TASK / 'namespace.pid').exists():
                step('STOP_' + service.upper() + '_FAILED', lambda service=service: controlled('stop-service', service))
            step('STOP_' + service.upper() + '_FALLBACK_FAILED', lambda service=service: fallback_stop(service))
        if (TASK / 'namespace.pid').exists():
            step('STOP_NAMESPACE_FAILED', lambda: controlled('stop'))
        for index, (process, log) in enumerate(processes):
            def wait_owned(process=process):
                try:
                    process.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=3)
            step('OWNED_PROCESS_WAIT_FAILED', wait_owned)
            step('OWNED_LOG_CLOSE_FAILED', log.close)

        database_removed = False
        manifest_path = TASK / 'https-database.json'
        if manifest_path.exists():
            def database_binding():
                value = json.loads(private_path(manifest_path).read_text())
                name, prefix = value['database'], value['rolePrefix']
                if (not re.fullmatch(r'unified_auth_https_[0-9a-f]{32}', name)
                        or prefix != 'uah_' + name[-12:] or value.get('clusterId') != cluster):
                    raise RuntimeError('OWNED_DATABASE_BINDING_REJECTED')
                return name, prefix
            binding = step('DATABASE_MANIFEST_REJECTED', database_binding)
            if binding:
                name, prefix = binding
                def owned_query(statement):
                    guarded = ('DO $$ BEGIN IF NOT (' + cluster_binding.sql_guard(owned_cluster)
                        + ") THEN RAISE EXCEPTION 'PHASE22_CLUSTER'; END IF; END $$;\n" + statement)
                    return subprocess.check_output(['psql', '-X', '-q', '-v', 'ON_ERROR_STOP=1', '-At', ADMIN,
                        '-c', guarded], text=True, stderr=subprocess.DEVNULL, timeout=15).strip()
                size = step('DATABASE_SIZE_PROBE_FAILED', lambda: owned_query("SELECT pg_database_size('" + name + "')"))
                if size is not None:
                    report['ownedDatabaseBytesBeforeDrop'] = size
                # common prepare depends on its original source-directory depth.
                step('DATABASE_ROLE_CLEANUP_FAILED', lambda: user(['python3', '-B', source / 'prepare.py',
                    '--task-dir', TASK, '--admin-url', ADMIN, '--cluster-id', cluster, '--cleanup']))
                roles = [prefix + '_' + suffix for suffix in ('identity_runtime', 'platform_runtime',
                    'identity_owner', 'platform_owner', 'bridge_owner', 'migration_owner')]
                query = ("SELECT (SELECT system_identifier::text FROM pg_control_system()) || '|' || "
                    "(SELECT count(*)::text FROM pg_database WHERE datname='" + name + "') || '|' || "
                    "(SELECT count(*)::text FROM pg_roles WHERE rolname IN (" + ','.join("'" + role + "'" for role in roles) + '))')
                counts = step('DATABASE_ROLE_VERIFY_FAILED', lambda: owned_query(query))
                report['ownedDatabaseAndRolesRemoved'] = database_removed = counts == cluster + '|0|0'
                if not database_removed:
                    step('DATABASE_OR_ROLE_REMAINS', lambda: (_ for _ in ()).throw(RuntimeError('RESIDUAL')))
        else:
            report['databaseManifestAbsent'] = True  # prepare may already have cleaned its failed initialization.
        # Compare private fixture material only in memory before deleting logs.
        candidates = set()
        def values(value, key=''):
            if isinstance(value, dict):
                for name, item in value.items():
                    values(item, name)
            elif isinstance(value, list):
                for item in value:
                    values(item, key)
            elif (isinstance(value, str) and len(value) >= 16 and not value.startswith(('/', 'http'))
                  and re.search(r'password|secret|token|csrf|cookie|verifier|^code$|privatekeypem|key$|username', key, re.I)):
                candidates.add(value.encode())
        scan_files = step('PRIVATE_INPUT_INVENTORY_FAILED', lambda: [target for target in TASK.iterdir()
            if target.is_file() and target.name.startswith(('phase22-', 'https-'))
            and target.suffix == '.json' and target.name != runtime_name]) or []
        for target in scan_files:
            step('PRIVATE_INPUT_SCAN_FAILED', lambda target=target: values(json.loads(private_path(target).read_text())))
        for name in ('phase22-metrics-token', 'phase22-collector-token'):
            target = TASK / name
            if target.exists():
                step('PRIVATE_TOKEN_SCAN_FAILED', lambda target=target: candidates.add(private_path(target).read_bytes().rstrip(b'\r\n')))
        logs = step('OWNED_LOG_INVENTORY_FAILED', lambda: [target for target in TASK.glob('*.log')
            if target.name == 'gateway.log' or target.name.startswith(('phase22-', 'service-'))]) or []
        log_bytes = []
        for target in logs:
            content = step('OWNED_LOG_SCAN_FAILED', lambda target=target: private_path(target).read_bytes())
            if content is not None:
                log_bytes.append(content)
        completed_runtime = locals().get('runtime')
        if completed_runtime is not None:
            log_bytes.extend(value for value in (completed_runtime.stdout, completed_runtime.stderr) if isinstance(value, bytes))
        report['logRedaction'] = {'files': len(logs), 'bytes': sum(map(len, log_bytes)),
            'privateCandidatesComparedInMemory': len(candidates),
            'leaks': sum(any(secret in data for data in log_bytes) for secret in candidates),
            'defaultInfoObserved': any(b'INFO' in data for data in log_bytes),
            'passwordWarningObserved': any(b'password_' in data and b'WARN' in data for data in log_bytes)}
        if report['logRedaction']['leaks']:
            report['result'] = 'FAILED'
        lease_names = {'https-database.json', 'namespace.pid', 'gateway.pid', 'service-identity.pid', 'service-platform.pid'}
        private_files = step('PRIVATE_CLEANUP_INVENTORY_FAILED', lambda: [p for p in TASK.iterdir()
            if p.is_file() and p.name not in before_files and p.name not in lease_names
            and (p.name.startswith(('https-', 'phase22-', 'service-')) or p.name == runtime_name
                or p.name in {'hosts', 'environment-controller.sh', 'namespace.stop'})]) or []
        # These known files can have been created before the baseline inventory.
        for name in ('https-environment.json', 'gateway.log', 'namespace.identity'):
            target = TASK / name
            if target.is_file() and target not in private_files:
                private_files.append(target)
        report['privateFilesRemoved'] = 0
        report['privateBytesRemoved'] = 0
        for target in private_files:
            def remove_private(target=target):
                checked = private_path(target)
                size = checked.stat().st_size
                checked.unlink()
                report['privateFilesRemoved'] += 1
                report['privateBytesRemoved'] += size
            step('PRIVATE_FILE_REMOVE_FAILED', remove_private)
        report['remainingLeaseRecords'] = sum((TASK / name).exists() for name in lease_names)
        if report['remainingLeaseRecords']:
            step('OWNED_LEASE_RECORDS_REMAIN', lambda: (_ for _ in ()).throw(RuntimeError('RESIDUAL')))
        report['remainingTrackedProcesses'] = sum(process.poll() is None for process, _log in processes)
        if report['remainingTrackedProcesses']:
            step('OWNED_TRACKED_PROCESSES_REMAIN', lambda: (_ for _ in ()).throw(RuntimeError('RESIDUAL')))
        report['dockerAfter'] = step('DOCKER_FINAL_DF_FAILED', lambda: subprocess.check_output(['docker', 'system', 'df'], text=True, timeout=15))
        report['filesystemAfter'] = step('FILESYSTEM_FINAL_DF_FAILED', lambda: subprocess.check_output(['df', '-B1', str(TASK)], text=True, timeout=15))
        report['retainedPublicTools'] = str(tools)
        report['toolsSha256'] = step('PUBLIC_TOOL_MANIFEST_FAILED', lambda: {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
            for p in tools.iterdir() if p.is_file() and not p.is_symlink()})
        report['cleanupErrors'] = cleanup_errors
        if cleanup_errors:
            report['result'] = 'FAILED'
        step('CLEANUP_REPORT_WRITE_FAILED', lambda: (VALIDATION / (report_prefix + '-cleanup.json')).write_text(json.dumps(report, indent=2)))
        print('PHASE22_NATIVE_ENVIRONMENT_' + report['result'])
        if report['result'] != 'PASS':
            raise SystemExit(1)


if __name__ == '__main__':
    main()
