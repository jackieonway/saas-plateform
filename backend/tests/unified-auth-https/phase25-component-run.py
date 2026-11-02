"""Record current frozen native component suites without touching runtime fixtures.

Use the already owned PostgreSQL cluster, no Docker or HBA mutation. Tests create
and remove their own UUID-named databases. Reports keep hashes and fixed test
names, never raw output or database credentials.
"""
import argparse
import contextlib
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import time
from urllib.parse import unquote, urlsplit
import uuid


TASK = Path('/tmp/rust-saas-unified-auth-20261028')
WORKSPACE = Path('/mnt/d/WorkSpace/rust/rust-saas')
VALIDATION = WORKSPACE / 'saas-plateform/backend/docs/designs/unified-auth/validation'
CARGO = '/home/jackie/.local/rust-1.97/bin/cargo'


def sha(data):
    return hashlib.sha256(data).hexdigest()


def utc():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def file_matches(path, digest):
    try:
        return path.is_file() and sha(path.read_bytes()) == digest
    except OSError:
        return False


def snapshot_inputs(snapshot, inputs):
    """Verify cross-project include_str!, vendor, tools and static inputs too."""
    mismatches = []
    for name, digest in inputs.items():
        relative = Path(name)
        assert not relative.is_absolute() and '..' not in relative.parts
        entry = snapshot / relative
        try:
            matched = entry.resolve() == entry and entry.is_file() and sha(entry.read_bytes()) == digest
        except OSError:
            matched = False
        if not matched:
            mismatches.append(name)
    return mismatches


def group_members(group, floor):
    """A new session can contain only the processes created by this Popen."""
    members = []
    for entry in Path('/proc').iterdir():
        if not entry.name.isdecimal():
            continue
        try:
            fields = (entry / 'stat').read_text().rsplit(')', 1)[1].split()
            if int(fields[2]) == group and int(fields[3]) == group and int(fields[19]) >= floor:
                members.append((int(entry.name), fields[19], fields[0]))
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            pass
    return members


def stop_group(group, floor):
    """Pin each original child with pidfd; never signal a reused numeric PID."""
    signalled = set()
    for signum, seconds in ((signal.SIGTERM, 2), (signal.SIGKILL, 2)):
        deadline = time.monotonic() + seconds
        while True:
            active = [item for item in group_members(group, floor) if item[2] != 'Z']
            if not active:
                return {'status': 'PASS', 'originalProcessesSignalled': len(signalled), 'activeProcessesRemaining': 0}
            for pid, started, _state in active:
                try:
                    descriptor = os.pidfd_open(pid)
                except ProcessLookupError:
                    continue
                try:
                    if any(item[0] == pid and item[1] == started for item in group_members(group, floor)):
                        signal.pidfd_send_signal(descriptor, signum)
                        signalled.add((pid, started))
                except ProcessLookupError:
                    pass
                finally:
                    os.close(descriptor)
            if time.monotonic() >= deadline:
                break
            time.sleep(.05)
    return {'status': 'FAIL', 'originalProcessesSignalled': len(signalled),
            'activeProcessesRemaining': sum(item[2] != 'Z' for item in group_members(group, floor))}


@contextlib.contextmanager
def interrupt_cleanup():
    def interrupted(_signum, _frame):
        raise KeyboardInterrupt
    previous = {item: signal.getsignal(item) for item in (signal.SIGTERM, signal.SIGHUP)}
    try:
        for item in previous:
            signal.signal(item, interrupted)
        yield
    finally:
        for item, handler in previous.items():
            signal.signal(item, handler)


def run_suite(command, *, cwd, env, timeout):
    floor = int(float(Path('/proc/uptime').read_text().split()[0]) * os.sysconf('SC_CLK_TCK')) - 1
    process = subprocess.Popen(command, cwd=cwd, env=env, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, start_new_session=True)
    raw = b''
    timed_out = False
    interrupted = False
    try:
        with interrupt_cleanup():
            try:
                output, errors = process.communicate(timeout=timeout)
                raw = output + errors
            except subprocess.TimeoutExpired as error:
                raw = (error.stdout or b'') + (error.stderr or b'')
                timed_out = True
            except KeyboardInterrupt:
                interrupted = True
    finally:
        cleanup = stop_group(process.pid, floor)
        cleanup.update(processGroup=process.pid, processStartTickFloor=floor)
        cleanup['interrupted'] = interrupted
        if process.poll() is None:
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                cleanup['status'] = 'FAIL'
        # Bounded drain after reaping the original group, including inherited pipes.
        try:
            output, errors = process.communicate(timeout=5)
            raw = output + errors
        except subprocess.TimeoutExpired:
            cleanup['status'] = 'FAIL'
            cleanup['inheritedPipeDrainFailed'] = True
    return process.returncode, raw, timed_out, cleanup


def fixture_name(kind, name):
    """Observe only component fixture prefixes; root HTTPS resources are excluded.

    These are a leak detector, never an authorization to delete a database/role.
    Any residual requires the coordinator's exact ownership-based cleanup.
    """
    if kind == 'databases':
        prefixes = ('identity_phase', 'identity_p04_', 'authorization_isolation_test_',
            'entity_creation_test_', 'notification_test_', 'organization_subtree_reporting_test_',
            'platform_settings_test_', 'production_security_test_', 'department_organizations_test_',
            'auth_review_phase19_', 'release_test_', 'context_http_', 'context_selection_',
            'phase26_stores_sql_', 'work_orders_phase')
        return name.startswith(prefixes)
    return bool(re.fullmatch(r'(?:id02|id12|id15h|i23k)_[a-f0-9]+_[a-z_]+', name)
                or re.fullmatch(r'p20_invite_[a-f0-9]{32}', name))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('service', choices=('identity', 'platform', 'portal', 'stores', 'orders', 'notification'))
    parser.add_argument('--snapshot', default='phase25-apps-r2')
    parser.add_argument('--selector', help='One exact existing Rust test name; no full-suite claim')
    parser.add_argument('--test-target', help='One existing Cargo integration test target')
    args = parser.parse_args()
    assert re.fullmatch(r'phase25-[a-z0-9-]+', args.snapshot)
    assert args.selector is None or re.fullmatch(r'[A-Za-z_][A-Za-z0-9_:]*', args.selector)
    assert args.test_target is None or re.fullmatch(r'[A-Za-z_][A-Za-z0-9_-]*', args.test_target)
    assert os.getuid() != 0
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import phase23_cluster_binding as binding_helper
    binding = binding_helper.load_binding(TASK)
    owner = (TASK / 'OWNER').stat()
    admin_raw, _ = binding_helper.protected_read(TASK / 'admin-url', uid=owner.st_uid,
                                                mode=0o600, maximum=4096)
    admin = admin_raw.decode().strip()
    endpoint = urlsplit(admin)
    assert endpoint.scheme in ('postgres', 'postgresql') and endpoint.hostname == '127.0.0.1'
    assert endpoint.port == 55494 and endpoint.username == 'postgres' and endpoint.path == '/postgres'
    assert not endpoint.query and not endpoint.fragment
    snapshot = TASK / ('source-snapshot-' + args.snapshot)
    manifest_raw = (TASK / ('snapshot-' + args.snapshot + '.json')).read_bytes()
    manifest = json.loads(manifest_raw)
    project = {'identity': 'saas-identity', 'platform': 'saas-plateform/backend',
               'portal': 'saas-customer-portal/backend',
               'stores': 'saas-stores/backend', 'orders': 'saas-work-orders/backend',
               'notification': 'saas-notification'}[args.service]
    assert snapshot.resolve() == snapshot and snapshot.is_dir() and snapshot.stat().st_uid == owner.st_uid
    assert not snapshot_inputs(snapshot, manifest['inputs']), 'SNAPSHOT_INPUTS_CHANGED'
    inputs = {name: digest for name, digest in manifest['inputs'].items()
              if name.startswith(project + '/') and '/dist/' not in name
              and '/node_modules/' not in name}
    assert inputs and all(sha((snapshot / name).read_bytes()) == digest for name, digest in inputs.items())
    env = {name: os.environ[name] for name in ('HOME', 'USER', 'LOGNAME') if name in os.environ}
    env.update(PATH='/home/jackie/.local/rust-1.97/bin:/usr/local/bin:/usr/bin:/bin',
               CARGO_TARGET_DIR=str((TASK / 'target-linux').resolve()),
               CARGO_HOME='/home/jackie/.cache/rust-saas-unified-auth-20261029/cargo-home',
               CARGO_INCREMENTAL='0', SQLX_OFFLINE='true', RUST_BACKTRACE='0',
               PYTHONDONTWRITEBYTECODE='1', TEST_DATABASE_URL=admin,
               CONTEXT_SELECTION_TEST_ADMIN_URL=admin,
               CONTEXT_SELECTION_TEST_CLUSTER_ID=binding['systemIdentifier'],
               MICRO_PROXY_TEST_NODE=str(TASK / 'phase21-node-runtime/bin/node'),
               STORES_TEST_NODE=str(TASK / 'phase21-node-runtime/bin/node'),
               WORK_ORDERS_TEST_NODE=str(TASK / 'phase21-node-runtime/bin/node'),
               IDENTITY_PHASE04_NOTIFICATION_BINARY=str(TASK / 'bin/saas-notification-ws'),
               PHASE25_CLI_BIN=str(TASK / 'bin/saas-platform-admin'))
    env['IDENTITY_PHASE12_TLS_TESTS'] = 'TASK_OWNED'
    markers = {'07': 'consent', '08': 'oauth', '09': 'oauth', '10': 'refresh', '11': 'machine', '12': 'context'}
    for phase in ('01', '02', '03', '04', '05', '07', '08', '09', '10', '11', '12', '19', '20', '22', '23', '24'):
        env['IDENTITY_PHASE' + phase + '_TEST_DATABASE_URL'] = admin
        env['IDENTITY_PHASE' + phase + '_TEST_CLUSTER_ID'] = binding['systemIdentifier']
        if phase in markers:
            env['IDENTITY_PHASE' + phase + '_TEST_MARKER'] = 'identity-phase' + phase + '-' + markers[phase] + '/task-owned/1/phase25-components'
    env.update(CI_TEST_POSTGRES_OWNER='ci-phase25-components',
               STORES_TEST_LOCAL_POSTGRES_OWNER='ci-phase25-components@127.0.0.1:55494',
               WORK_ORDERS_TEST_LOCAL_POSTGRES_OWNER='ci-phase25-components@127.0.0.1:55494')
    attestation_raw = (TASK / ('build-attestation-' + args.snapshot + '.json')).read_bytes()
    attestation = json.loads(attestation_raw)
    assert attestation['snapshotLabel'] == args.snapshot
    assert attestation['snapshotSha256'] == manifest['snapshotSha256']
    auxiliary = {}
    for name in ('saas-platform-admin', 'saas-notification-ws'):
        binary = TASK / 'bin' / name
        digest = sha(binary.read_bytes())
        assert digest == attestation['binaries'][name], 'AUXILIARY_BINARY_CHANGED'
        auxiliary[name] = {'path': str(binary), 'sha256': digest}
    node = TASK / 'phase21-node-runtime/bin/node'
    node_raw = (TASK / 'phase24-node-runtime-attestation.json').read_bytes()
    node_attestation = json.loads(node_raw)
    assert sha(node.read_bytes()) == node_attestation['binarySha256'], 'NODE_BINARY_CHANGED'
    auxiliary['node'] = {'path': str(node), 'sha256': node_attestation['binarySha256'],
                         'attestationSha256': sha(node_raw)}
    def pg(statement):
        pg_env = {'PATH': '/usr/bin:/bin', 'PGHOST': '127.0.0.1', 'PGPORT': '55494',
                  'PGUSER': 'postgres', 'PGDATABASE': 'postgres', 'PGCONNECT_TIMEOUT': '5',
                  'PGOPTIONS': '-c log_statement=none -c log_min_error_statement=panic'}
        if endpoint.password:
            pg_env['PGPASSWORD'] = unquote(endpoint.password)
        result = subprocess.run(['psql', '-X', '-A', '-t', '-v', 'ON_ERROR_STOP=1'],
                                input=statement.encode(), capture_output=True,
                                env=pg_env, timeout=20)
        assert result.returncode == 0, 'OWNED_PG_QUERY_FAILED'
        return result.stdout.decode().strip()
    def resources():
        return {'databases': json.loads(pg("SELECT coalesce(json_agg(datname ORDER BY datname),'[]') FROM pg_database")),
                'roles': json.loads(pg("SELECT coalesce(json_agg(rolname ORDER BY rolname),'[]') FROM pg_roles"))}
    assert pg('SELECT ' + binding_helper.sql_guard(binding)) == 't', 'ACTUAL_CLUSTER_CHANGED'
    before = resources()
    temporary = TASK / ('phase25-component-' + args.service + '-' + uuid.uuid4().hex)
    temporary.mkdir(mode=0o700)
    env.update({name: str(temporary) for name in ('TMPDIR', 'TMP', 'TEMP')})
    commands = ([['--lib'], ['--bin', 'saas-identity']] if args.service == 'identity'
                else [['--all-targets']])
    if args.test_target:
        commands = [['--test', args.test_target]]
    report = {'contractVersion': 'unified-auth-phase25-component-suite/1', 'phase': 25,
              'service': args.service, 'snapshot': args.snapshot,
              'exactTestSelector': args.selector, 'integrationTestTarget': args.test_target,
              'fullServiceSuiteClaimed': args.selector is None and args.test_target is None,
              'snapshotSha256': manifest['snapshotSha256'], 'manifestSha256': sha(manifest_raw),
              'sourceInputs': inputs, 'startedAtUtc': utc(), 'status': 'FAIL',
              'runnerSha256': sha(Path(__file__).read_bytes()),
              'allSnapshotInputsVerified': len(manifest['inputs']),
              'auxiliaryBinaries': auxiliary, 'buildAttestationSha256': sha(attestation_raw),
              'layers': ['RUST_COMPONENT', 'EXPLICIT_TASK_OWNED_POSTGRESQL_ROUTER'],
              'clusterSystemIdentifier': binding['systemIdentifier'],
              'clusterBindingSha256': sha((TASK / binding_helper.NAME).read_bytes()),
              'dockerCreated': 0, 'dockerReused': 0, 'hbaWrites': 0, 'runs': [],
              'limits': ['No browser, external client, actual service HTTPS or release acceptance claim.',
                         'Any ignored, skipped or failed test remains explicit. Test PostgreSQL schemas may be narrow fixtures.']}
    try:
        for selection in commands:
            command = [CARGO, 'test', '--offline', '--locked', '-j2', *selection,
                       *([args.selector] if args.selector else []),
                       '--', '--include-ignored', '--test-threads=1', *(['--exact'] if args.selector else [])]
            started = time.monotonic()
            started_at = utc()
            try:
                code, raw, timed_out, process_cleanup = run_suite(command, cwd=snapshot / project,
                                                                env=env, timeout=1800)
            except KeyboardInterrupt:
                report['interrupted'] = True
                report['status'] = 'FAIL'
                break
            text = raw.decode(errors='replace')
            cases = [{'name': name, 'result': result.upper()} for name, result in re.findall(
                r'^test ([A-Za-z0-9_:]+) \.\.\. (ok|FAILED|ignored)', text, re.M)]
            summaries = [{'result': result, 'passed': int(passed), 'failed': int(failed),
                          'ignored': int(ignored), 'filtered': int(filtered)}
                         for result, passed, failed, ignored, filtered in re.findall(
                r'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; \d+ measured; (\d+) filtered out', text)]
            executables = []
            for value in re.findall(r'Running [^\n]+ \(([^)]+)\)', text):
                executable = Path(value)
                if not executable.is_absolute():
                    executable = (snapshot / project / executable).resolve()
                if executable.is_file() and executable.is_relative_to(Path(env['CARGO_TARGET_DIR'])):
                    executables.append({'pathRelativeToTarget': executable.relative_to(Path(env['CARGO_TARGET_DIR'])).as_posix(),
                                        'sha256': sha(executable.read_bytes())})
            report['runs'].append({'command': command, 'startedAtUtc': started_at, 'returnCode': code,
                                   'durationSeconds': round(time.monotonic()-started, 3),
                                   'rawOutputSha256': sha(raw), 'cases': cases, 'summaries': summaries,
                                   'testExecutables': executables,
                                   'processCleanup': process_cleanup,
                                   'timedOut': timed_out,
                                   'compilerErrorCodes': sorted(set(re.findall(r'error\[(E[0-9]+)\]', text)))})
            report['status'] = 'PASS' if all(run['returnCode'] == 0 and run['summaries']
                and run['processCleanup']['status'] == 'PASS' and not run['timedOut']
                and not run['processCleanup'].get('interrupted')
                and not any(summary['failed'] or summary['ignored'] for summary in run['summaries'])
                for run in report['runs']) else 'FAIL'
            # Never retain a raw log: tests can contain private fixture material.
            if code != 0:
                report['failureTestNames'] = [case['name'] for case in cases if case['result'] == 'FAILED']
                break
    finally:
        try:
            after = resources()
            report['resourceObservation'] = {kind: {'added': sorted(set(after[kind])-set(before[kind])),
                                                   'removed': sorted(set(before[kind])-set(after[kind]))}
                                             for kind in before}
        except Exception:
            report['status'] = 'FAIL'
            report['resourceObservationFailure'] = 'OWNED_RESOURCE_POSTCHECK_UNAVAILABLE'
            report['resourceObservation'] = {kind: {'added': [], 'removed': []} for kind in before}
        report['sourceMismatchAfter'] = snapshot_inputs(snapshot, manifest['inputs'])
        report['auxiliaryMismatchAfter'] = [name for name, value in auxiliary.items()
            if not file_matches(Path(value['path']), value['sha256'])]
        report['finishedAtUtc'] = utc()
        report['totals'] = {name: sum(summary[name] for run in report['runs'] for summary in run['summaries'])
                            for name in ('passed', 'failed', 'ignored', 'filtered')}
        residuals = {kind: [name for name in report['resourceObservation'][kind]['added'] if fixture_name(kind, name)]
                     for kind in before}
        temporary_count = 0
        temporary_bytes = 0
        try:
            assert not any(run['processCleanup']['status'] != 'PASS' for run in report['runs']), 'PROCESS_GROUP_REQUIRES_RECOVERY'
            assert temporary.resolve() == temporary and temporary.parent == TASK and temporary.stat().st_uid == owner.st_uid
            temporary_entries = list(temporary.rglob('*'))
            temporary_count = sum(path.is_file() or path.is_symlink() for path in temporary_entries)
            temporary_bytes = sum(path.lstat().st_size for path in temporary_entries if path.is_file() and not path.is_symlink())
            shutil.rmtree(temporary)
        except Exception:
            report['status'] = 'FAIL'
            report['temporaryCleanupFailure'] = 'OWNED_TEMPORARY_DIRECTORY_CLEANUP_FAILED'
        report['cleanup'] = {'rawLogsRetained': 0, 'temporaryDirectoriesCreatedByRunner': 1,
                             'temporaryRuntimeDirectory': str(temporary),
                             'temporaryRuntimeDirectoryRemoved': not temporary.exists(),
                             'remainingTemporaryFilesRemoved': temporary_count, 'remainingTemporaryLogicalBytesRemoved': temporary_bytes,
                             'testFixturesSelfClean': not any(residuals.values()) and 'resourceObservationFailure' not in report,
                             'componentFixtureResiduals': residuals,
                             'globalObservationMayIncludeConcurrentRootFixtures': True,
                             'databaseDeletionByThisRunner': False}
        if (report['sourceMismatchAfter'] or report['auxiliaryMismatchAfter'] or any(residuals.values()) or not report['totals']['passed']
                or 'temporaryCleanupFailure' in report or 'resourceObservationFailure' in report
                or any(run['processCleanup']['status'] != 'PASS' for run in report['runs'])):
            report['status'] = 'FAIL'
        suffix = ('-selected-' + sha(args.selector.encode())[:12]) if args.selector else (
            '-integration-' + args.test_target if args.test_target else '')
        output = VALIDATION / ('phase-25-' + args.service + suffix + '-components.json')
        if output.exists():
            previous = output.read_bytes()
            backup = output.with_name(output.stem + '-attempt-' + sha(previous)[:12] + '.json')
            if not backup.exists():
                backup.write_bytes(previous)
        output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n')
        print(json.dumps({'status': report['status'], 'service': args.service,
                          'totals': report['totals'], 'output': str(output),
                          'resources': report['resourceObservation']}))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
