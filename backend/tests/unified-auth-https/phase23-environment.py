"""Explicit root controls for ONE owned Phase 23 environment; no import-time I/O.

prepare --snapshot LABEL; start; stop-writers; start-writers;
select-tools --snapshot LABEL --build-snapshot ORIGINAL --hba-restored;
profile --ttl-seconds 30|300|600; quarantine-source;
rotate-db-passwords --passwords PRIVATE --environment PRIVATE;
use-target --environment PRIVATE --gate-receipt PRIVATE --recovery-config PRIVATE;
open-target; restart-rp; restart-collector; status;
cleanup [--hba-restored]. Actual browser/DR, HBA and proxy ownership stay with caller.
No command accepts a secret on argv. No Windows 24445 or protocol/demo 23103 starts.
"""
import argparse
import contextlib
import hashlib
import json
import os
from pathlib import Path
import pwd
import re
import select
import shutil
import signal
import stat
import subprocess
import sys
import time
from urllib.parse import unquote, urlsplit
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parent))
import phase23_cluster_binding as cluster_binding

TASK = Path('/tmp/rust-saas-unified-auth-20261028')
WORKSPACE = Path('/mnt/d/WorkSpace/rust/rust-saas')
ADMIN = 'postgres://postgres@127.0.0.1:55494/postgres'
SERVICES = {'identity': 'saas-identity', 'platform': 'saas-platform-backend', 'portal': 'saas-customer-portal-backend'}
DAEMONS = {**SERVICES, 'identity-peer': 'saas-identity'}
ROLES = ('identity_runtime', 'platform_runtime', 'portal_runtime', 'identity_owner',
         'platform_owner', 'bridge_owner', 'migration_owner', 'portal_owner')
RP_PREFIX = 'saas-examples/oidc-rp/'
RP_FILES = {'package.json', 'package-lock.json', 'server.mjs', 'machine.mjs', 'scripts/check.mjs',
            'src/app.mjs', 'src/backchannel.mjs', 'src/config.mjs', 'src/oidc.mjs', 'src/store.mjs',
            'src/transport.mjs', 'public/app.js', 'public/index.html', 'public/style.css'}
PHASE25_RP_TEST_FILES = {'tests/capacity.test.mjs', 'tests/deployment.test.mjs',
                         'tests/oidc.test.mjs', 'tests/support.mjs'}


class EnvironmentFailure(RuntimeError):
    """Only this module's explicit guard codes may reach CLI diagnostics."""
    def __init__(self, code):
        self.code = 'PHASE23_ENV_' + code if type(code) is str and re.fullmatch(r'[A-Z0-9_]+', code) else None
        super().__init__(self.code or 'PHASE23_ENVIRONMENT_FAILED')


def safe_failure_code(error):
    if isinstance(error, EnvironmentFailure) and type(error.code) is str \
            and re.fullmatch(r'PHASE23_ENV_[A-Z0-9_]+', error.code):
        return error.code
    return 'PHASE23_ENVIRONMENT_FAILED'


def need(value, code):
    if not value:
        raise EnvironmentFailure(code)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def literal(value):
    return "'" + str(value).replace("'", "''") + "'"


def start_time(pid):
    return Path('/proc', str(pid), 'stat').read_text().rsplit(')', 1)[1].split()[19]


def compiled_input(relative):
    prefixes = ('saas-identity/src/', 'saas-identity/static/', 'saas-plateform/backend/src/',
        'saas-plateform/backend/static/', 'saas-plateform/backend/assets/', 'saas-plateform/backend/migrations/',
        'saas-plateform/backend/crates/', 'saas-plateform/backend/ops/recovery/src/',
        'saas-customer-portal/backend/src/', 'saas-customer-portal/backend/migrations/',
        'saas-plateform/frontend/', 'saas-customer-portal/frontend/',
        'saas-stores/backend/', 'saas-stores/frontend/',
        'saas-work-orders/backend/', 'saas-work-orders/frontend/',
        'saas-notification/src/', 'saas-notification/tests/',
        'saas-plateform/backend/tools/platform-admin/', RP_PREFIX)
    if relative.startswith(prefixes):
        return True
    if relative.startswith('saas-plateform/backend/tests/unified-auth-https/phase23-db-auth-probe/'):
        return Path(relative).suffix == '.rs' or Path(relative).name in ('Cargo.toml', 'Cargo.lock')
    return (Path(relative).parent.as_posix() in ('saas-identity', 'saas-notification', 'saas-plateform/backend', 'saas-customer-portal/backend')
            and Path(relative).name in ('Cargo.toml', 'Cargo.lock', 'build.rs', 'rust-toolchain.toml', 'rust-toolchain'))


def run(command, *, environment=None, input=None, timeout=45):
    result = subprocess.run(list(map(str, command)), input=input, capture_output=True,
                            env=environment, timeout=timeout)
    need(result.returncode == 0, 'SUBPROCESS_FAILED')
    return result.stdout


@contextlib.contextmanager
def spawn_signals(defer=False):
    pending = False
    def interrupt(_signum, _frame):
        nonlocal pending
        if defer:
            pending = True
        else:
            raise KeyboardInterrupt
    original = {sig: signal.getsignal(sig) for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP)}
    try:
        for sig in original:
            signal.signal(sig, interrupt)
        yield
    finally:
        for sig, handler in original.items():
            signal.signal(sig, handler)
        if pending:
            raise KeyboardInterrupt


def reap_spawn_group(process):
    """Only an unreaped child created here with start_new_session=True."""
    # Do not poll()/wait() before the final group signal: keeping the leader
    # unreaped reserves its PID/PGID while descendants are terminated.
    for signum in (signal.SIGCONT, signal.SIGTERM):
        try:
            os.killpg(process.pid, signum)
        except ProcessLookupError:
            pass
    deadline = time.monotonic() + 5
    while os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT) is None:
        if time.monotonic() >= deadline:
            break
        time.sleep(.05)
    try:
        # The leader may already be a zombie; its children still need the
        # final group signal before wait() releases this reserved PGID.
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait(timeout=3)
    deadline = time.monotonic() + 3
    while True:
        try:
            os.killpg(process.pid, 0)
        except ProcessLookupError:
            return
        need(time.monotonic() < deadline, 'SPAWN_GROUP_REMAINS')
        time.sleep(.05)


class Task:
    def __init__(self, require_root=True):
        need(os.name == 'posix' and hasattr(os, 'O_NOFOLLOW') and TASK.resolve(strict=True) == TASK
             and not TASK.is_symlink(), 'OWNED_LINUX_TASK_REQUIRED')
        owner_path = TASK / 'OWNER'
        owner = owner_path.lstat()
        need(stat.S_ISREG(owner.st_mode) and owner.st_nlink == 1 and owner.st_uid != 0
             and stat.S_IMODE(owner.st_mode) == 0o600 and stat.S_IMODE(TASK.stat().st_mode) == 0o700
             and TASK.stat().st_uid == owner.st_uid, 'TASK_OWNER_INVALID')
        need(owner_path.read_bytes() == (TASK.name + '-pg55494\n').encode(), 'TASK_MARKER_INVALID')
        need(os.getuid() == 0 if require_root else os.getuid() == owner.st_uid, 'CALLER_UID_INVALID')
        self.uid, self.gid = owner.st_uid, owner.st_gid
        self.cluster_binding = cluster_binding.load_binding(TASK)
        self.cluster = self.cluster_binding['systemIdentifier']
        self.username = pwd.getpwuid(self.uid).pw_name
        self.state_path = TASK / 'phase23-environment-state.json'
        self.state = self.json(self.state_path) if self.state_path.exists() else None
        self.handles = []

    def checked(self, path, *, private=True, maximum=64 * 1024 * 1024, empty_ok=False):
        path = Path(path)
        need(path.is_absolute() and path.is_relative_to(TASK) and '..' not in path.parts
             and path != TASK, 'PRIVATE_PATH_INVALID')
        current = TASK
        for component in path.relative_to(TASK).parts:
            current /= component
            need(not current.is_symlink(), 'SYMLINK_REJECTED')
        info = path.lstat()
        need(stat.S_ISREG(info.st_mode) and info.st_nlink == 1 and (info.st_size > 0 or empty_ok or not private) and info.st_size <= maximum
             and info.st_uid in (0, self.uid) and not info.st_mode & 0o022, 'FILE_INVALID')
        if private:
            need(info.st_uid == self.uid and stat.S_IMODE(info.st_mode) == 0o600, 'PRIVATE_FILE_INVALID')
        return path

    def read(self, path, **kwargs):
        path = self.checked(path, **kwargs)
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        try:
            before = os.fstat(fd)
            data = b''
            while len(data) < before.st_size + 1:
                chunk = os.read(fd, min(65536, before.st_size + 1 - len(data)))
                if not chunk:
                    break
                data += chunk
            after = os.fstat(fd)
            need(len(data) == before.st_size and (before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns)
                 == (after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns), 'FILE_CHANGED')
            return data
        finally:
            os.close(fd)

    def json(self, path, **kwargs):
        return json.loads(self.read(path, **kwargs))

    def digest(self, path, maximum=2 * 1024 * 1024 * 1024):
        path = self.checked(path, private=False, maximum=maximum)
        result = hashlib.sha256()
        with path.open('rb') as source:
            before = os.fstat(source.fileno())
            while chunk := source.read(1024 * 1024):
                result.update(chunk)
            after = os.fstat(source.fileno())
        need((before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns)
             == (after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns), 'DIGEST_INPUT_CHANGED')
        return result.hexdigest()

    def write(self, path, value, replace=False):
        path = Path(path)
        need(path.parent == TASK or (path.parent.is_relative_to(TASK) and path.parent.resolve() == path.parent), 'WRITE_PATH_INVALID')
        need(not path.is_symlink(), 'WRITE_SYMLINK_REJECTED')
        if replace:
            self.checked(path)
        data = value if isinstance(value, bytes) else json.dumps(value, indent=2).encode() + b'\n'
        temporary = path.with_name('.phase23-write-' + os.urandom(8).hex()) if replace else path
        fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        try:
            os.fchmod(fd, 0o600)
            if os.getuid() == 0:
                os.fchown(fd, self.uid, self.gid)
            with os.fdopen(fd, 'wb', closefd=False) as output:
                output.write(data)
                output.flush()
                os.fsync(fd)
            if replace:
                os.replace(temporary, path)
        finally:
            os.close(fd)
            if replace and temporary.exists():
                temporary.unlink()

    def save(self):
        self.write(self.state_path, self.state, replace=self.state_path.exists())

    def user(self, command, **kwargs):
        environment = {'PATH': '/usr/lib/postgresql/18/bin:/usr/local/bin:/usr/bin:/bin',
                       'HOME': str(TASK), 'LANG': 'C.UTF-8',
                       'PYTHONDONTWRITEBYTECODE': '1',
                       'PGOPTIONS': '-c log_statement=none -c log_min_error_statement=panic'}
        return run(['/usr/sbin/runuser', '-u', self.username, '--', *map(str, command)], environment=environment, **kwargs)

    def pg(self, statement, database='postgres'):
        need(re.fullmatch(r'[a-z][a-z0-9_]{0,62}', database), 'DATABASE_IDENTIFIER_INVALID')
        environment = {'PATH': '/usr/lib/postgresql/18/bin:/usr/bin:/bin', 'PGHOST': '127.0.0.1',
                       'PGPORT': '55494', 'PGUSER': 'postgres', 'PGDATABASE': database,
                       'PGSSLMODE': 'prefer', 'PGAPPNAME': 'phase23-owned-environment',
                       'PGOPTIONS': '-c log_statement=none -c log_min_error_statement=panic'}
        raw = run(['/usr/lib/postgresql/18/bin/psql', '-X', '-qAt', '-v', 'ON_ERROR_STOP=1',
                   '-v', 'VERBOSITY=sqlstate'], environment=environment,
                  input=("DO $$ BEGIN IF NOT (" + cluster_binding.sql_guard(self.cluster_binding)
                         + ") THEN RAISE EXCEPTION 'PHASE23_CLUSTER'; END IF; END $$;\n"
                         + statement).encode(), timeout=30)
        return raw.decode().strip()

    def snapshot(self, label=None, all_inputs=True, build_label=None):
        label = label or self.state['snapshot']
        build_label = build_label or (self.state or {}).get('buildSnapshot', label)
        need(re.fullmatch(r'phase(?:23|25)-[a-z0-9-]+', label), 'SNAPSHOT_LABEL_INVALID')
        need(re.fullmatch(r'phase(?:23|25)-[a-z0-9-]+', build_label), 'BUILD_SNAPSHOT_LABEL_INVALID')
        snapshot = TASK / ('source-snapshot-' + label)
        need(snapshot.is_dir() and not snapshot.is_symlink() and snapshot.resolve() == snapshot, 'SNAPSHOT_PATH_INVALID')
        manifest = self.json(TASK / ('snapshot-' + label + '.json'), private=False)
        build_snapshot = TASK / ('source-snapshot-' + build_label)
        need(build_snapshot.is_dir() and not build_snapshot.is_symlink() and build_snapshot.resolve() == build_snapshot, 'BUILD_SNAPSHOT_PATH_INVALID')
        build_manifest = self.json(TASK / ('snapshot-' + build_label + '.json'), private=False)
        attestation = self.json(TASK / ('build-attestation-' + build_label + '.json'), private=False)
        inputs = manifest['inputs']
        build_inputs = build_manifest['inputs']
        need(type(inputs) is dict and inputs and attestation['inputs'] == build_inputs
             and manifest['includePortal'] is True and build_manifest['includePortal'] is True
             and manifest['snapshotSha256'] == sha(json.dumps(inputs, sort_keys=True).encode())
             and build_manifest['snapshotSha256'] == sha(json.dumps(build_inputs, sort_keys=True).encode())
             and attestation['snapshotSha256'] == build_manifest['snapshotSha256']
             and attestation['snapshotLabel'] == build_label
             and {'identity', 'platform', 'portal'} <= set(attestation['builtServices']), 'SNAPSHOT_ATTESTATION_INVALID')
        if label.startswith('phase25-') or build_label.startswith('phase25-'):
            need(label.startswith('phase25-') and build_label.startswith('phase25-')
                 and manifest.get('includeAllProjects') is True
                 and build_manifest.get('includeAllProjects') is True
                 and manifest.get('completeFrontendOutputs') is True
                 and build_manifest.get('completeFrontendOutputs') is True
                 and {'identity', 'platform', 'portal', 'stores', 'work-orders', 'notification', 'cli'}
                     <= set(attestation['builtServices']), 'PHASE25_FULL_CANDIDATE_REQUIRED')
            for name in ('stores-api', 'stores-migrate', 'work-orders-api', 'work-orders-migrate',
                         'saas-notification-ws', 'saas-platform-admin'):
                need(name in attestation['binaries']
                     and self.digest(TASK / 'bin' / name) == attestation['binaries'][name],
                     'PHASE25_BINARY_ATTESTATION_INVALID')
        need({name: digest for name, digest in inputs.items() if compiled_input(name)}
             == {name: digest for name, digest in build_inputs.items() if compiled_input(name)}, 'COMPILED_APPLICATION_INPUT_DRIFT')
        for relative, expected in build_inputs.items():
            if compiled_input(relative):
                need(self.digest(build_snapshot / relative) == expected, 'COMPILED_SNAPSHOT_INPUT_CHANGED')
        rp = {name.removeprefix(RP_PREFIX) for name in inputs if name.startswith(RP_PREFIX)}
        expected_rp = RP_FILES | PHASE25_RP_TEST_FILES if label.startswith('phase25-') else RP_FILES
        need(rp == expected_rp, 'RP_SNAPSHOT_INPUTS_INVALID')
        for relative, expected in inputs.items():
            need(not Path(relative).is_absolute() and '..' not in Path(relative).parts
                 and re.fullmatch(r'[0-9a-f]{64}', expected), 'SNAPSHOT_INPUT_INVALID')
            if all_inputs or relative.startswith(RP_PREFIX):
                need(self.digest(snapshot / relative) == expected, 'SNAPSHOT_INPUT_CHANGED')
        for name in (*SERVICES.values(), 'platform-migrate', 'portal-migrate', 'phase23-db-auth-probe'):
            need(name in attestation['binaries'] and self.digest(TASK / 'bin' / name) == attestation['binaries'][name], 'BINARY_ATTESTATION_INVALID')
        return snapshot, manifest, attestation

    def tools(self):
        return TASK / ('source-snapshot-' + self.state['snapshot']) / 'saas-plateform/backend/tests/unified-auth-https'

    def namespace(self):
        value = self.json(TASK / 'namespace.pid', private=False)
        need(type(value.get('pid')) is int and value['pid'] > 1
             and re.fullmatch(r'mnt:\[[0-9]+\]', value.get('namespace', ''))
             and start_time(value['pid']) == value['startTime']
             and os.readlink('/proc/' + str(value['pid']) + '/ns/mnt') == value['namespace']
             and str(TASK / 'environment-controller.sh').encode() in Path('/proc', str(value['pid']), 'cmdline').read_bytes(),
             'NAMESPACE_RECORD_INVALID')
        return value

    def open_log(self, path):
        path = Path(path)
        need(path.parent == TASK, 'LOG_PATH_INVALID')
        try:
            fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_APPEND | os.O_NOFOLLOW, 0o600)
        except FileExistsError:
            self.checked(path, empty_ok=True)
            fd = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_NOFOLLOW | os.O_NONBLOCK)
        else:
            try:
                # Only a newly created inode may have its permissions or owner set.
                os.fchmod(fd, 0o600)
                if os.getuid() == 0:
                    os.fchown(fd, self.uid, self.gid)
            except BaseException:
                os.close(fd)
                raise
        try:
            self.checked(path, empty_ok=True)
            opened, named = os.fstat(fd), path.lstat()
            need((opened.st_dev, opened.st_ino) == (named.st_dev, named.st_ino)
                 and stat.S_ISREG(opened.st_mode) and opened.st_nlink == 1
                 and opened.st_uid == self.uid and stat.S_IMODE(opened.st_mode) == 0o600,
                 'LOG_FILE_CHANGED')
            return fd
        except BaseException:
            os.close(fd)
            raise

    def spawn(self, key, command, script, uid):
        need(re.fullmatch(r'[a-z][a-z0-9-]{0,40}', key), 'SPAWN_KEY_INVALID')
        need(key not in self.state['processes'] or not self.alive(self.state['processes'][key]), 'PROCESS_ALREADY_RUNNING')
        journal_path = TASK / ('phase23-spawn-' + key + '.json')
        need(not journal_path.exists() and not journal_path.is_symlink(), 'SPAWN_CLEANUP_REQUIRED')
        log_path = TASK / ('phase23-' + key + '.log')
        fd = self.open_log(log_path)
        environment = {'PATH': '/usr/lib/postgresql/18/bin:/usr/local/bin:/usr/bin:/bin',
                       'TASK_GATEWAY_SCRIPT': 'phase23-gateway.py', 'HOME': str(TASK), 'LANG': 'C.UTF-8',
                       'PYTHONDONTWRITEBYTECODE': '1'}
        process, journal = None, None
        previous = self.state['processes'].get(key)
        try:
            with spawn_signals():
                try:
                    # Delay caught signals across Popen's return/assignment,
                    # without masking signals inherited by the executed child.
                    with spawn_signals(defer=True):
                        process = subprocess.Popen(list(map(str, command)), env=environment, stdout=fd,
                                                   stderr=subprocess.STDOUT, start_new_session=True)
                    journal = {'contractVersion': 'phase23-spawn-ownership/1', 'key': key,
                               'pid': process.pid, 'pgid': process.pid, 'startTime': start_time(process.pid),
                               'script': str(script), 'uid': uid, 'cleanupRequired': True,
                               'namespace': os.readlink('/proc/' + str(process.pid) + '/ns/mnt')}
                    self.write(journal_path, journal)
                    deadline = time.monotonic() + 5
                    while True:
                        need(os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT) is None,
                             'PROCESS_START_FAILED')
                        proc = Path('/proc', str(process.pid))
                        if str(script).encode() in (proc / 'cmdline').read_bytes() and proc.stat().st_uid == uid:
                            value = {'pid': process.pid, 'startTime': journal['startTime'], 'script': str(script),
                                     'uid': uid, 'namespace': os.readlink(proc / 'ns/mnt'), 'pgid': process.pid}
                            self.state['processes'][key] = value
                            self.save()
                            need(self.json(journal_path) == journal, 'SPAWN_JOURNAL_CHANGED')
                            journal_path.unlink()
                            return
                        need(time.monotonic() < deadline, 'PROCESS_RECORD_NOT_READY')
                        time.sleep(.05)
                except BaseException:
                    # Repeated cancellation cannot interrupt rollback halfway.
                    with spawn_signals(defer=True):
                        if process is not None:
                            try:
                                reap_spawn_group(process)
                            except BaseException:
                                if journal is None:
                                    journal = {'contractVersion': 'phase23-spawn-ownership/1', 'key': key,
                                        'pid': process.pid, 'pgid': process.pid, 'startTime': None,
                                        'script': str(script), 'uid': uid, 'cleanupRequired': True, 'namespace': None}
                                    try:
                                        journal['startTime'] = start_time(process.pid)
                                        journal['namespace'] = os.readlink('/proc/' + str(process.pid) + '/ns/mnt')
                                    except BaseException:
                                        pass
                                # A separately protected lease survives a failed
                                # state save; an unknown start time stays blocked.
                                self.state.setdefault('pendingSpawns', {})[key] = journal
                                persisted = False
                                try:
                                    if journal_path.exists():
                                        need(self.json(journal_path) == journal, 'SPAWN_JOURNAL_CHANGED')
                                    else:
                                        self.write(journal_path, journal)
                                    persisted = True
                                except BaseException:
                                    pass
                                try:
                                    self.save()
                                    persisted = True
                                except BaseException:
                                    pass
                                need(persisted, 'SPAWN_OWNERSHIP_PERSIST_FAILED')
                                raise EnvironmentFailure('SPAWN_CLEANUP_REQUIRED') from None
                            if previous is None:
                                self.state['processes'].pop(key, None)
                            else:
                                self.state['processes'][key] = previous
                            if journal_path.exists():
                                need(self.json(journal_path) == journal, 'SPAWN_JOURNAL_CHANGED')
                                journal_path.unlink()
                            try:
                                self.save()
                            except BaseException:
                                # Any stale state record now names a dead child;
                                # preserving the original failure is sufficient.
                                pass
                    raise
        finally:
            os.close(fd)

    def alive(self, value):
        proc = Path('/proc', str(value['pid']))
        try:
            if not (proc / 'exe').exists():
                return False
            need(start_time(value['pid']) == value['startTime'] and proc.stat().st_uid == value['uid']
                 and os.readlink(proc / 'ns/mnt') == value['namespace']
                 and value['script'].encode() in (proc / 'cmdline').read_bytes(), 'PROCESS_IDENTITY_CHANGED')
            return True
        except FileNotFoundError:
            return False

    def stop_spawn_group(self, value):
        """Stop only a recorded new session, pinning each member with pidfd."""
        leader = value['pid']
        need(type(leader) is int and leader > 1 and value.get('pgid') == leader
             and re.fullmatch(r'[0-9]+', value.get('startTime') or '')
             and hasattr(os, 'pidfd_open') and hasattr(signal, 'pidfd_send_signal'), 'SPAWN_GROUP_RECORD_INVALID')
        descriptors = {}
        namespace = value.get('namespace')
        anchored = False
        def collect(termination_signal):
            nonlocal anchored, namespace
            anchored = False
            members = []
            for entry in Path('/proc').iterdir():
                if not entry.name.isdigit():
                    continue
                try:
                    fields = (entry / 'stat').read_text().rsplit(')', 1)[1].split()
                    if int(fields[2]) != leader:
                        continue
                    if fields[0] == 'Z':
                        continue
                    need(int(fields[3]) == leader and entry.stat().st_uid in (0, self.uid), 'SPAWN_GROUP_IDENTITY_CHANGED')
                    current_namespace = os.readlink(entry / 'ns/mnt')
                    if int(entry.name) == leader:
                        need(fields[19] == value['startTime'], 'SPAWN_GROUP_LEADER_CHANGED')
                        namespace = current_namespace
                        anchored = True
                    members.append((entry, fields[19], current_namespace))
                except (FileNotFoundError, ProcessLookupError):
                    continue
            if not members:
                return 0
            # After an exited leader, a surviving member must still carry the
            # task's private namespace and a task path before we adopt its PID.
            if not anchored:
                pinned_alive = descriptors and len(select.select(list(descriptors.values()), [], [], 0)[0]) < len(descriptors)
                need(pinned_alive or any(str(TASK).encode() in (entry / 'cmdline').read_bytes()
                     for entry, _, _ in members), 'SPAWN_GROUP_UNANCHORED')
            for entry, started, current_namespace in members:
                need(current_namespace == namespace and int(started) >= int(value['startTime']),
                     'SPAWN_GROUP_NAMESPACE_CHANGED')
                identity = (int(entry.name), started)
                if identity in descriptors:
                    continue
                try:
                    fd = os.pidfd_open(identity[0])
                    try:
                        need(start_time(identity[0]) == started
                             and os.getpgid(identity[0]) == leader and os.getsid(identity[0]) == leader
                             and os.readlink(entry / 'ns/mnt') == namespace,
                             'SPAWN_GROUP_MEMBER_CHANGED')
                    except BaseException:
                        os.close(fd)
                        raise
                    descriptors[identity] = fd
                    for signum in (signal.SIGCONT, termination_signal):
                        try:
                            signal.pidfd_send_signal(fd, signum)
                        except ProcessLookupError:
                            pass
                except (FileNotFoundError, ProcessLookupError):
                    continue
            return len(members)
        try:
            for termination_signal, seconds in ((signal.SIGTERM, 5), (signal.SIGKILL, 3)):
                for fd in descriptors.values():
                    try:
                        signal.pidfd_send_signal(fd, termination_signal)
                    except ProcessLookupError:
                        pass
                deadline = time.monotonic() + seconds
                while True:
                    collect(termination_signal)
                    exited = set(select.select(list(descriptors.values()), [], [], .05)[0])
                    if len(exited) == len(descriptors):
                        # The prior inventory may predate a final fork. Once
                        # every known member has exited, enumerate the group
                        # again and include any newly visible native child.
                        remaining = collect(termination_signal)
                        exited = set(select.select(list(descriptors.values()), [], [], 0)[0])
                        if remaining == 0 and len(exited) == len(descriptors):
                            return
                    if time.monotonic() >= deadline:
                        break
            need(False, 'SPAWN_GROUP_REMAINS')
        finally:
            for fd in descriptors.values():
                os.close(fd)

    def stop_pending_spawns(self):
        pending = self.state.get('pendingSpawns', {}) if self.state else {}
        need(type(pending) is dict, 'SPAWN_JOURNAL_INVALID')
        records = {key: (value, None) for key, value in pending.items()}
        for path in sorted(TASK.glob('phase23-spawn-*.json')):
            value = self.json(path)
            key = value.get('key')
            need(key not in records or records[key][0] == value, 'SPAWN_JOURNAL_CHANGED')
            records[key] = (value, path)
        for key, (value, path) in records.items():
            need(value.get('contractVersion') == 'phase23-spawn-ownership/1'
                 and isinstance(key, str) and re.fullmatch(r'[a-z][a-z0-9-]{0,40}', key)
                 and value.get('key') == key
                 and (path is None or path.name == 'phase23-spawn-' + key + '.json')
                 and value.get('cleanupRequired') is True, 'SPAWN_JOURNAL_INVALID')
        for key, (value, path) in records.items():
            self.stop_spawn_group(value)
            if path is not None:
                need(self.json(path) == value, 'SPAWN_JOURNAL_CHANGED')
                path.unlink()
            pending.pop(key, None)
            self.save()

    def stop_process(self, key):
        value = self.state['processes'].get(key)
        if value is None:
            return
        if 'pgid' in value:
            self.stop_spawn_group(value)
        elif self.alive(value):
            os.kill(value['pid'], signal.SIGCONT)
            os.kill(value['pid'], signal.SIGTERM)
            deadline = time.monotonic() + 8
            while self.alive(value) and time.monotonic() < deadline:
                time.sleep(.1)
            if self.alive(value):
                os.kill(value['pid'], signal.SIGKILL)
                deadline = time.monotonic() + 3
                while self.alive(value) and time.monotonic() < deadline:
                    time.sleep(.1)
            need(not self.alive(value), 'PROCESS_REMAINS')
        self.state['processes'].pop(key, None)
        self.save()

    def service(self, service):
        path = TASK / ('service-' + service + '.pid')
        if not path.exists():
            return None
        record = self.json(path)
        need(service in DAEMONS and record['binary'] == str(TASK / 'bin' / DAEMONS[service])
             and type(record['pid']) is int and record['pid'] > 1
             and re.fullmatch(r'mnt:\[[0-9]+\]', record['namespace']), 'SERVICE_RECORD_INVALID')
        return path, record

    def stop_service(self, service):
        value = self.service(service)
        if value:
            path, record = value
            proc = Path('/proc', str(record['pid']))
            if (proc / 'exe').exists():
                need(os.readlink(proc / 'exe') == record['binary'] and start_time(record['pid']) == record['startTime']
                     and os.readlink(proc / 'ns/mnt') == record['namespace'], 'SERVICE_IDENTITY_CHANGED')
                os.kill(record['pid'], signal.SIGCONT)
                os.kill(record['pid'], signal.SIGTERM)
                deadline = time.monotonic() + 8
                while (proc / 'exe').exists() and time.monotonic() < deadline:
                    need(start_time(record['pid']) == record['startTime'], 'SERVICE_PID_REUSED')
                    time.sleep(.1)
                if (proc / 'exe').exists():
                    need(start_time(record['pid']) == record['startTime']
                         and os.readlink(proc / 'exe') == record['binary'], 'SERVICE_IDENTITY_CHANGED')
                    os.kill(record['pid'], signal.SIGKILL)
            self.stop_process('service-' + service)
            deadline = time.monotonic() + 2
            while path.exists() and time.monotonic() < deadline:
                time.sleep(.1)
            if path.exists():
                need(self.json(path) == record and not (proc / 'exe').exists(), 'SERVICE_RECORD_CHANGED')
                path.unlink()
        else:
            self.stop_process('service-' + service)

    def stop_auxiliary(self, kind):
        names = {'rp': 'phase23-rp.pid', 'collector': 'collector.pid', 'scram': 'phase23-scram-proxy.pid'}
        path = TASK / names[kind]
        native = self.json(path) if path.exists() else None
        tracked = self.state['processes'].get(kind)
        need(native is None or tracked is not None, 'UNTRACKED_AUXILIARY_RECORD')
        if native is not None:
            need(type(native.get('pid')) is int and native['pid'] > 1
                 and re.fullmatch(r'[0-9]+', native.get('startTime', '')), 'AUXILIARY_RECORD_INVALID')
            if kind != 'rp':
                need(native['pid'] == tracked['pid'] and native['startTime'] == tracked['startTime'], 'AUXILIARY_NATIVE_BINDING_INVALID')
            else:
                need(native.get('contractVersion') == 'unified-auth-phase23-rp-process/1'
                     and native.get('namespace') == tracked['namespace']
                     and native.get('script') == str(TASK / 'phase23-rp-source/server.mjs'), 'RP_CHILD_BINDING_INVALID')
        self.stop_process(kind)
        if native is not None:
            proc = Path('/proc', str(native['pid']))
            if (proc / 'exe').exists():
                need(kind == 'rp' and start_time(native['pid']) == native['startTime']
                     and os.readlink(proc / 'ns/mnt') == native['namespace']
                     and os.readlink(proc / 'exe') == native['binary']
                     and native['script'].encode() in (proc / 'cmdline').read_bytes(), 'AUXILIARY_CHILD_IDENTITY_CHANGED')
                os.kill(native['pid'], signal.SIGTERM)
                deadline = time.monotonic() + 5
                while (proc / 'exe').exists() and time.monotonic() < deadline:
                    need(start_time(native['pid']) == native['startTime'], 'AUXILIARY_PID_REUSED')
                    time.sleep(.1)
                need(not (proc / 'exe').exists(), 'RP_CHILD_REMAINS')
            if path.exists():
                need(self.json(path) == native, 'AUXILIARY_RECORD_CHANGED')
                path.unlink()

    def ns_command(self, command):
        namespace = self.namespace()
        return ['nsenter', '-t', str(namespace['pid']), '-m', '--', 'setpriv', '--reuid=' + str(self.uid),
                '--regid=' + str(self.gid), '--init-groups', *map(str, command)]

    def health(self, origin, endpoint, expected=None):
        environment = self.json(TASK / 'https-environment.json')
        ca = Path(environment['identity']['CONTEXT_PLATFORM_CA_FILE'])
        self.checked(ca)
        from urllib.parse import urlsplit
        parsed = urlsplit(origin)
        resolve = parsed.hostname + ':' + str(parsed.port or 443) + ':127.0.0.2'
        deadline = time.monotonic() + 20
        while True:
            result = subprocess.run(['curl', '--noproxy', '*', '--cacert', str(ca), '--resolve', resolve,
                '--silent', '--max-time', '2', '--output', '/dev/null', '--write-out', '%{http_code}',
                origin + endpoint], capture_output=True)
            if result.returncode == 0 and result.stdout == b'200':
                return
            need(time.monotonic() < deadline, expected or 'SERVICE_HEALTH_FAILED')
            time.sleep(.1)

    def resources(self):
        manifest = self.json(TASK / 'https-database.json')
        if (TASK / 'phase23-resources.json').exists():
            value = self.json(TASK / 'phase23-resources.json')
        else:
            need(self.state and not self.state['prepared'], 'RESOURCE_RECORD_MISSING')
            environment = self.json(TASK / 'https-environment.json')
            deployment = environment['ids']['deployment']
            value = {'contractVersion': 'unified-auth-phase23-resources/1', 'clusterId': self.cluster,
                'deploymentId': deployment, 'source': manifest, 'targets': {kind: {
                    'database': 'unified_auth_restore_' + ('portal_' if kind == 'portal' else '') + manifest['database'][-32:],
                    'owner': manifest['rolePrefix'] + ('_portal_owner' if kind == 'portal' else '_migration_owner'),
                    'marker': 'phase23-recovery/2/' + deployment} for kind in ('platform', 'portal')}}
        need(value['contractVersion'] == 'unified-auth-phase23-resources/1' and value['clusterId'] == self.cluster
             and value['source'] == manifest and manifest['clusterId'] == self.cluster, 'RESOURCE_BINDING_INVALID')
        name, prefix = manifest['database'], manifest['rolePrefix']
        need(re.fullmatch(r'unified_auth_https_[0-9a-f]{32}', name) and prefix == 'uah_' + name[-12:], 'SOURCE_NAME_INVALID')
        deployment = str(uuid.UUID(value['deploymentId']))
        need(uuid.UUID(deployment).version == 7 and deployment == value['deploymentId'], 'DEPLOYMENT_INVALID')
        need(manifest['portal'] == {'database': 'unified_auth_portal_' + name[-32:], 'owner': prefix + '_portal_owner',
             'runtime': prefix + '_portal_runtime', 'marker': 'unified-auth-https-portal/1/' + deployment,
             'initializationPending': False}, 'PORTAL_BINDING_INVALID')
        expected = {kind: {'database': 'unified_auth_restore_' + ('portal_' if kind == 'portal' else '') + name[-32:],
            'owner': prefix + ('_portal_owner' if kind == 'portal' else '_migration_owner'),
            'marker': 'phase23-recovery/2/' + deployment} for kind in ('platform', 'portal')}
        need(value['targets'] == expected, 'TARGET_BINDING_INVALID')
        return value

    def empty_preparation_resources(self):
        """A failed pre-fixture prepare may have no ownership manifest at all.

        Absence is accepted only after an actual guarded query proves this
        dedicated cluster has no nondefault DB and no uah_* role. Unknown
        resources are preserved; this path never infers names or issues DROP.
        """
        need(self.state is not None and self.state.get('prepared') is False
             and not (TASK / 'https-database.json').exists()
             and not (TASK / 'phase23-resources.json').exists(), 'EMPTY_PREPARATION_NOT_PROVEN')
        observed = json.loads(self.pg("SELECT jsonb_build_object('nonDefaultDatabases',"
            "(SELECT count(*) FROM pg_database WHERE datname NOT IN('postgres','template0','template1')),"
            "'taskRoles',(SELECT count(*) FROM pg_roles WHERE starts_with(rolname,'uah_')))"))
        need(type(observed) is dict and set(observed) == {'nonDefaultDatabases', 'taskRoles'}
             and all(type(value) is int and value == 0 for value in observed.values()),
             'EMPTY_PREPARATION_UNKNOWN_RESOURCES')
        return {'databasesRemaining': 0, 'rolesRemaining': 0,
                'emptyPreparationVerifiedByGuardedPostgres': True}

    def pairs(self, mode):
        resources = self.resources()
        if mode == 'SOURCE':
            shared = resources['source']['database']
            prefix = resources['source']['rolePrefix']
            return {'platform': {'database': shared, 'owner': prefix + '_migration_owner',
                                'marker': 'identity-deployment/1/' + resources['deploymentId']},
                    'portal': {key: resources['source']['portal'][key] for key in ('database', 'owner', 'marker')}}
        need(mode == 'RESTORE', 'MODE_INVALID')
        return resources['targets']

    def guard_databases(self, mode, quiet=False, absent_ok=False):
        records = {}
        for kind, expected in self.pairs(mode).items():
            raw = self.pg("SELECT jsonb_build_object('owner',pg_get_userbyid(datdba),'marker',shobj_description(oid,'pg_database'),"
                "'limit',datconnlimit,'size',pg_database_size(oid))::text FROM pg_database WHERE datname=" + literal(expected['database']))
            if not raw and absent_ok:
                continue
            need(raw, 'DATABASE_MISSING')
            record = json.loads(raw)
            need(record['owner'] == expected['owner'] and record['marker'] == expected['marker'], 'DATABASE_OWNER_OR_MARKER_INVALID')
            if quiet:
                need(self.pg('SELECT count(*) FROM pg_stat_activity WHERE datname=' + literal(expected['database'])) == '0', 'WRITERS_REMAIN')
            records[kind] = record
        return records

    def guard_roles(self):
        prefix = self.resources()['source']['rolePrefix']
        names = [prefix + '_' + suffix for suffix in ROLES]
        sql_names = 'ARRAY[' + ','.join(map(literal, names)) + ']'
        runtime_names = 'ARRAY[' + ','.join(literal(prefix + '_' + kind + '_runtime') for kind in SERVICES) + ']'
        check = "SELECT ((SELECT count(*) FROM pg_roles WHERE rolname=ANY(" + sql_names + "))=8 AND NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname=ANY(" + sql_names + ") AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls OR rolinherit OR rolcanlogin<>(rolname=ANY(" + runtime_names + ")))) AND NOT EXISTS(SELECT 1 FROM pg_auth_members membership JOIN pg_roles role_row ON role_row.oid IN(membership.roleid,membership.member) WHERE role_row.rolname=ANY(" + sql_names + ")))::text"
        need(self.pg(check) == 'true', 'ROLE_CLOSURE_INVALID')

    def environment_binding(self, environment, mode):
        resources = self.resources()
        prefix = resources['source']['rolePrefix']
        need(environment['ids']['deployment'] == resources['deploymentId']
             and environment['phase23']['contractVersion'] == 'unified-auth-phase23/1', 'ENVIRONMENT_DEPLOYMENT_INVALID')
        pairs = self.pairs(mode)
        for service in SERVICES:
            parsed = urlsplit(environment[service]['DATABASE_URL'])
            name = pairs['portal' if service == 'portal' else 'platform']['database']
            need(parsed.scheme == 'postgres' and parsed.hostname == '127.0.0.1' and parsed.port == 55494
                 and parsed.path == '/' + name and unquote(parsed.username or '') == prefix + '_' + service + '_runtime'
                 and not parsed.query and not parsed.fragment and parsed.password, 'RUNTIME_DATABASE_BINDING_INVALID')
        need(environment['phase21']['rp']['third']['RP_ORIGIN'] == 'https://customer.saas.test:24446', 'RP_ORIGIN_INVALID')

    def core(self, action, config, sequence, environment=None, environment_sha=None):
        self.checked(config)
        source = self.tools().parents[1] / 'ops/identity-recovery/recovery.py'
        if environment is not None:
            # The supplied candidate must be the exact authenticated rearm
            # material, not merely a same-database JSON assembled by the caller.
            code = """import sys
from pathlib import Path
sys.path.insert(0,sys.argv[1])
import core
config=core.load_config(Path(sys.argv[2])); material=core.keys(config)
receipt=core.authenticated_read(config['workDirectory']/'start.json',material['mac'],'start')
core.need('rearmed' in receipt and receipt['materialPaths']['environment']==sys.argv[3]
 and receipt['materialDigests']['environment']==sys.argv[5]
 and core.sha(core.private_read(Path(sys.argv[3])))==sys.argv[5],'ENVIRONMENT_NOT_GATED_MATERIAL')
print(core.canonical(core.gate(config,material,int(sys.argv[4]))).decode())
"""
            raw = self.user(['python3', '-B', '-c', code, source.parent, config,
                             environment, str(sequence), environment_sha], timeout=180)
            return json.loads(raw)
        raw = self.user(['python3', '-B', source, action, '--config', config,
                         '--minimum-sequence', str(sequence)], timeout=180)
        return json.loads(raw)


def prepare(task, args):
    need(task.state is None and not any((TASK / name).exists() for name in
         ('https-database.json', 'namespace.pid', 'gateway.pid')), 'EMPTY_LEASE_REQUIRED')
    snapshot, manifest, attestation = task.snapshot(args.snapshot, build_label=args.build_snapshot or args.snapshot)
    task.state = {'contractVersion': 'unified-auth-phase23-environment/1', 'snapshot': args.snapshot,
        'snapshotSha256': manifest['snapshotSha256'], 'buildSnapshot': args.build_snapshot or args.snapshot,
        'buildSnapshotSha256': attestation['snapshotSha256'], 'mode': 'SOURCE', 'processes': {}, 'profiles': [],
        'prepared': False, 'sourceQuarantined': False, 'targetOpen': False, 'gate': None,
        'filesystemBefore': run(['df', '-B1', TASK]).decode(), 'dockerBefore': run(['docker', 'system', 'df']).decode(),
        'dockerContext': run(['docker', 'context', 'show']).decode().strip(), 'containersCreated': 0, 'containersReused': 0}
    task.save()
    # Frozen public inputs become readable by the one unprivileged task owner.
    for relative in manifest['inputs']:
        path = snapshot / relative
        for directory in (path.parent, *path.parent.parents):
            if not directory.is_relative_to(snapshot):
                break
            os.chown(directory, task.uid, task.gid)
            directory.chmod(0o700)
        os.chown(path, task.uid, task.gid)
        path.chmod(0o600)
    build_snapshot = TASK / ('source-snapshot-' + task.state['buildSnapshot'])
    for relative in attestation['inputs']:
        if not compiled_input(relative):
            continue
        path = build_snapshot / relative
        for directory in (path.parent, *path.parent.parents):
            if not directory.is_relative_to(build_snapshot):
                break
            os.chown(directory, task.uid, task.gid)
            directory.chmod(0o700)
        os.chown(path, task.uid, task.gid)
        path.chmod(0o600)
    for name in {'snapshot-' + args.snapshot + '.json', 'snapshot-' + task.state['buildSnapshot'] + '.json',
                 'build-attestation-' + task.state['buildSnapshot'] + '.json'}:
        path = TASK / name
        os.chown(path, task.uid, task.gid)
        path.chmod(0o600)
    for name in (*SERVICES.values(), 'platform-migrate', 'portal-migrate', 'phase23-db-auth-probe'):
        path = TASK / 'bin' / name
        need(path.is_file() and not path.is_symlink(), 'FROZEN_BINARY_MISSING')
        os.chown(path, task.uid, task.gid)
        path.chmod(0o700)
    os.chown(TASK / 'bin', task.uid, task.gid)
    (TASK / 'bin').chmod(0o700)
    tls_workspace = TASK / 'phase23-gateway-workspace'
    need(not tls_workspace.exists(), 'TLS_WORKSPACE_EXISTS')
    (tls_workspace / '.local-run').mkdir(parents=True, mode=0o700)
    for directory in (tls_workspace, tls_workspace / '.local-run'):
        os.chown(directory, task.uid, task.gid)
        directory.chmod(0o700)
    mappings = {'ca': 'ca.pem', 'certificate': 'server.pem', 'key': 'server.key'}
    tls_paths = {}
    for kind, name in mappings.items():
        source = WORKSPACE / '.local-run' / name
        need(source.is_file() and not source.is_symlink() and source.stat().st_size < 1024 * 1024, 'TLS_SOURCE_INVALID')
        data = source.read_bytes()
        destination = TASK / ('phase23-' + name)
        task.write(destination, data)
        task.write(tls_workspace / '.local-run' / name, data)
        tls_paths[kind] = str(destination)
    task.write(TASK / 'phase23-tls-materials.json', tls_paths)
    tools = task.tools()
    task.user(['python3', '-B', tools / 'prepare.py', '--task-dir', TASK, '--admin-url', ADMIN,
        '--cluster-id', task.cluster, '--migration-bin', TASK / 'bin/platform-migrate', '--backend-source', snapshot / 'saas-plateform/backend',
        '--include-phase19', '--include-portal', '--include-phase20', '--portal-migration-bin', TASK / 'bin/portal-migrate',
        '--tls-ca-file', TASK / 'phase23-ca.pem'], timeout=120)
    task.user(['python3', '-B', tools / 'phase21-fixture.py', '--task-dir', TASK, '--admin-url', ADMIN,
               '--rp-origin', 'https://customer.saas.test:24446'], timeout=90)
    task.user(['python3', '-B', tools / 'phase23-fixture.py', '--task-dir', TASK, '--admin-url', ADMIN], timeout=90)
    task.resources()
    task.environment_binding(task.json(TASK / 'https-environment.json'), 'SOURCE')
    task.guard_databases('SOURCE', quiet=True)
    task.guard_roles()
    task.state['prepared'] = True
    task.save()
    return {'status': 'PHASE23_PREPARED', 'services': 3, 'containersCreated': 0,
            'compiledFromBuildSnapshot': True, 'opsExecutedFromSelectedSnapshot': True,
            'compiledApplicationInputsIdentical': True}


def start_service(task, service):
    need(service in DAEMONS, 'SERVICE_NAME_INVALID')
    # The shared runner opens a separate native log with its inherited umask.
    # Preflight every native log before starting any service, including restarts.
    for name in DAEMONS:
        fd = task.open_log(TASK / ('service-' + name + '.log'))
        os.close(fd)
    record = task.service(service)
    if record:
        tracked = task.state['processes'].get('service-' + service)
        need(tracked and task.alive(tracked), 'UNTRACKED_SERVICE')
        native = record[1]
        proc = Path('/proc', str(native['pid']))
        need(os.readlink(proc / 'exe') == native['binary'] and start_time(native['pid']) == native['startTime']
             and os.readlink(proc / 'ns/mnt') == native['namespace'] and proc.stat().st_uid == task.uid,
             'SERVICE_IDENTITY_CHANGED')
        return
    script = task.tools() / 'run-service.py'
    task.spawn('service-' + service, ['bash', task.tools() / 'environment.sh', 'run', '--task-dir', TASK,
               '--service', service], script, task.uid)
    deadline = time.monotonic() + 8
    while not task.service(service):
        need(time.monotonic() < deadline, 'SERVICE_RECORD_TIMEOUT')
        time.sleep(.1)


def identity_instances(task):
    records = {name: task.service(name) for name in ('identity', 'identity-peer')}
    need(all(records.values()), 'IDENTITY_INSTANCES_MISSING')
    primary, peer = (records[name][1] for name in ('identity', 'identity-peer'))
    keyset = task.json(TASK / 'https-environment.json')['identity']['OIDC_KEYSET_PATH']
    digest = task.digest(keyset)
    need(primary['pid'] != peer['pid'] and primary['binary'] == peer['binary']
         and primary['namespace'] == peer['namespace']
         and primary.get('configuredKeysetSha256') == peer.get('configuredKeysetSha256') == digest
         and primary.get('keysetPath') == peer.get('keysetPath') == keyset,
         'IDENTITY_INSTANCES_NOT_INDEPENDENT_OR_KEYSET_CHANGED')
    for record in (primary, peer):
        proc = Path('/proc', str(record['pid']))
        need(start_time(record['pid']) == record['startTime']
             and os.readlink(proc / 'exe') == record['binary']
             and os.readlink(proc / 'ns/mnt') == record['namespace']
             and proc.stat().st_uid == task.uid, 'IDENTITY_INSTANCE_CHANGED')
    return {'independentProcesses': 2, 'distinctPids': True,
            'sameBinarySha256': task.digest(primary['binary']), 'configuredKeysetSha256': digest}


def start_writers(task, _args):
    need(task.state['prepared'] and ((task.state['mode'] == 'SOURCE' and not task.state['sourceQuarantined'])
         or (task.state['mode'] == 'RESTORE' and task.state['targetOpen'])), 'TRAFFIC_GATE_CLOSED')
    task.namespace()
    task.snapshot(all_inputs=False)
    task.environment_binding(task.json(TASK / 'https-environment.json'), task.state['mode'])
    task.guard_databases(task.state['mode'])
    task.guard_roles()
    start_service(task, 'identity')
    start_service(task, 'identity-peer')
    task.health('https://auth.saas.test:24444', '/health/ready')
    task.health('https://auth.saas.test', '/healthz')
    start_service(task, 'platform')
    task.health('https://console.saas.test', '/auth/config')
    start_service(task, 'portal')
    task.health('https://customer.saas.test', '/auth/config')
    task.state['lastVerifiedReadiness'] = {'identityPrivateReady': 200, 'identityPublicLiveness': 200,
                                         'platformAuthConfig': 200, 'portalAuthConfig': 200}
    task.state['identityInstances'] = identity_instances(task)
    task.save()
    return {'status': 'PHASE23_WRITERS_READY', 'services': 3, 'daemonInstances': 4, 'mode': task.state['mode'],
            'identityInstances': task.state['identityInstances'],
            'readiness': task.state['lastVerifiedReadiness']}


def auxiliary(task, kind):
    if kind == 'rp':
        script = task.tools() / 'phase23-run-rp.py'
        command = ['python3', '-B', script, '--task-dir', TASK]
    elif kind == 'collector':
        script = task.tools() / 'collector.py'
        command = ['python3', '-B', script, '--task-dir', TASK]
    else:
        script = task.tools() / 'phase23-scram-proxy.py'
        config = TASK / 'phase23-scram-proxy-input.json'
        value = {'contractVersion': 'identity-scram-proxy-input/1', 'taskDir': str(TASK), 'listenPort': 25495,
                 'maximumConnections': 8, 'backlog': 8, 'maximumBytesPerConnection': 1048576,
                 'totalTimeoutMs': 10000, 'idleTimeoutMs': 3000, 'chunkBytes': 65536}
        if config.exists():
            need(task.json(config) == value, 'SCRAM_CONFIG_CHANGED')
        else:
            task.write(config, value)
        command = ['python3', '-B', script, '--config', config]
    task.spawn(kind, task.ns_command(command), script, task.uid)
    if kind == 'collector':
        task.state['collectorMode'] = task.state['mode']
        task.save()
    expected = {'rp': 'phase23-rp.pid', 'collector': 'collector.pid', 'scram': 'phase23-scram-proxy.pid'}[kind]
    deadline = time.monotonic() + (30 if kind == 'rp' else 8)
    while not (TASK / expected).exists():
        need(task.alive(task.state['processes'][kind]) and time.monotonic() < deadline, 'AUXILIARY_NOT_READY')
        time.sleep(.1)


def start(task, args):
    need(task.state['prepared'] and not (TASK / 'namespace.pid').exists(), 'FRESH_NAMESPACE_REQUIRED')
    task.snapshot()
    task.spawn('namespace', ['bash', task.tools() / 'environment.sh', 'start', '--task-dir', TASK,
        '--workspace', TASK / 'phase23-gateway-workspace', '--static-workspace', TASK / ('source-snapshot-' + task.state['snapshot'])],
        TASK / 'environment-controller.sh', 0)
    deadline = time.monotonic() + 10
    while not all((TASK / name).exists() for name in ('namespace.pid', 'gateway.pid')):
        need(time.monotonic() < deadline and task.alive(task.state['processes']['namespace']), 'NAMESPACE_START_TIMEOUT')
        time.sleep(.1)
    task.namespace()
    gateway_pid = int((TASK / 'gateway.pid').read_text())
    task.state['processes']['gateway'] = {'pid': gateway_pid, 'startTime': start_time(gateway_pid), 'uid': 0,
        'namespace': task.namespace()['namespace'], 'script': str(task.tools() / 'phase23-gateway.py')}
    task.save()
    gateway_log = TASK / 'gateway.log'
    if gateway_log.exists():
        task.checked(gateway_log, private=False, empty_ok=True)
        os.chown(gateway_log, task.uid, task.gid)
        gateway_log.chmod(0o600)
    auxiliary(task, 'scram')
    auxiliary(task, 'collector')
    start_writers(task, args)
    auxiliary(task, 'rp')
    task.health('https://customer.saas.test:24446', '/')
    return {'status': 'PHASE23_ENVIRONMENT_READY', 'services': 3, 'standardRp': True, 'protocolStarted': False,
            'readiness': {**task.state['lastVerifiedReadiness'], 'standardRp': 200}}


def select_tools(task, args):
    """Select changed tools while retaining the same stopped, prepared source."""
    need(args.hba_restored is True, 'TOOLS_HBA_CONFIRMATION_REQUIRED')
    need(task.state is not None and task.state.get('prepared') is True
         and task.state.get('mode') == 'SOURCE' and task.state.get('sourceQuarantined') is False
         and task.state.get('targetOpen') is False and task.state.get('gate') is None,
         'TOOLS_PREPARED_SOURCE_REQUIRED')
    need(type(args.snapshot) is str and re.fullmatch(r'phase(?:23|25)-tools-r[1-9][0-9]*', args.snapshot)
         and args.snapshot != task.state['snapshot'], 'TOOLS_NEW_SNAPSHOT_REQUIRED')
    need(args.build_snapshot == task.state.get('buildSnapshot') and args.build_snapshot is not None,
         'TOOLS_ORIGINAL_BUILD_REQUIRED')
    def quiescent():
        need(type(task.state.get('processes')) is dict and not task.state['processes']
             and type(task.state.get('pendingSpawns', {})) is dict and not task.state.get('pendingSpawns', {})
             and not any(TASK.glob('phase23-spawn-*.json')), 'TOOLS_PENDING_PROCESSES')
        leases = ('namespace.pid', 'namespace.identity', 'gateway.pid', 'phase23-rp.pid', 'collector.pid',
                  'phase23-scram-proxy.pid', 'browser-proxy.pid',
                  *('service-' + name + '.pid' for name in DAEMONS))
        need(not any((TASK / name).exists() or (TASK / name).is_symlink() for name in leases),
             'TOOLS_LEASES_REMAIN')
    quiescent()
    task.guard_databases('SOURCE', quiet=True)
    need(task.guard_databases('RESTORE', quiet=True, absent_ok=True) == {}, 'TOOLS_TARGET_ALREADY_EXISTS')
    task.guard_roles()
    task.environment_binding(task.json(TASK / 'https-environment.json'), 'SOURCE')
    snapshot, manifest, _ = task.snapshot(args.snapshot, build_label=args.build_snapshot)
    # Later unprivileged consumers must be able to read the selected code. Only
    # public snapshot permissions are inspected; no config or secret is changed.
    directories = {snapshot}
    for relative in manifest['inputs']:
        filename = snapshot / relative
        info = filename.lstat()
        need(info.st_uid == task.uid and not info.st_mode & 0o022 and info.st_mode & 0o400,
             'TOOLS_SNAPSHOT_NOT_OWNER_READABLE')
        for directory in filename.parents:
            if not directory.is_relative_to(snapshot):
                break
            directories.add(directory)
    for directory in directories:
        info = directory.lstat()
        need(stat.S_ISDIR(info.st_mode) and info.st_uid == task.uid
             and not info.st_mode & 0o022 and info.st_mode & 0o500 == 0o500,
             'TOOLS_SNAPSHOT_NOT_OWNER_TRAVERSABLE')
    quiescent()
    previous = task.state['snapshot']
    task.state['snapshot'] = args.snapshot
    task.state['snapshotSha256'] = manifest['snapshotSha256']
    task.save()
    return {'status': 'PHASE23_TOOLS_SELECTED', 'previousSnapshot': previous,
            'snapshot': args.snapshot, 'snapshotSha256': manifest['snapshotSha256'],
            'buildSnapshot': task.state['buildSnapshot'], 'buildSnapshotUnchanged': True,
            'sourceDatabaseAndSecretsUnchanged': True, 'rootHbaRestorationConfirmed': True}


def stop_writers(task, _args):
    errors = []
    for service in DAEMONS:
        try:
            task.stop_service(service)
        except Exception:
            errors.append('STOP_' + service.upper())
    need(not errors, 'WRITER_STOP_INCOMPLETE')
    task.guard_databases(task.state['mode'], quiet=True)
    task.state['lastVerifiedReadiness'] = None
    task.save()
    return {'status': 'PHASE23_WRITERS_STOPPED', 'databaseConnections': 0, 'standardRpRetained': True}


def profile(task, args):
    need(args.ttl_seconds in (30, 300, 600), 'PROFILE_INVALID')
    for service in ('identity', 'identity-peer'):
        task.stop_service(service)
    path = TASK / 'https-environment.json'
    environment = task.json(path)
    task.environment_binding(environment, task.state['mode'])
    # Canonical token TTL is fixed at 300 seconds; the old profile knob is retired.
    task.write(path, environment, replace=True)
    if task.state['mode'] == 'SOURCE':
        task.write(TASK / 'phase23-source-environment.json', environment, replace=True)
    start_service(task, 'identity')
    start_service(task, 'identity-peer')
    task.health('https://auth.saas.test:24444', '/health/ready')
    instances = identity_instances(task)
    task.state['identityInstances'] = instances
    task.state['profiles'].append({'requestedProfileSeconds': args.ttl_seconds,
                                   'canonicalTokenMaxTtlSeconds': 300, 'loadedAfterRestart': True,
                                   'identityInstances': instances})
    task.save()
    return {'status': 'PHASE23_PROFILE_RESTARTED', 'requestedProfileSeconds': args.ttl_seconds,
            'canonicalTokenMaxTtlSeconds': 300,
            'loadedAfterRestart': True, 'identityInstances': instances}


def source_closed(task):
    task.guard_databases('SOURCE')
    resources = task.resources()
    runtimes = [resources['source']['rolePrefix'] + '_' + kind + '_runtime' for kind in SERVICES]
    for expected in task.pairs('SOURCE').values():
        checks = ' AND '.join('NOT has_database_privilege(' + literal(role) + ',' + literal(expected['database']) + ",'CONNECT')" for role in runtimes)
        checks += " AND NOT EXISTS(SELECT 1 FROM pg_database database_row CROSS JOIN LATERAL aclexplode(COALESCE(database_row.datacl,acldefault('d',database_row.datdba))) acl WHERE database_row.datname=" + literal(expected['database']) + " AND acl.grantee=0 AND acl.privilege_type='CONNECT')"
        need(task.pg('SELECT (' + checks + ')::text') == 'true', 'SOURCE_CONNECT_STILL_OPEN')


def quarantine_source(task, _args):
    need(not any(task.service(service) for service in DAEMONS), 'WRITERS_NOT_STOPPED')
    task.guard_databases('SOURCE', quiet=True)
    task.guard_roles()
    prefix = task.resources()['source']['rolePrefix']
    grantees = ['PUBLIC'] + ['"' + prefix + '_' + kind + '_runtime"' for kind in SERVICES]
    task.pg('BEGIN;\n' + '\n'.join('REVOKE CONNECT ON DATABASE "' + value['database'] + '" FROM ' + ','.join(grantees) + ';'
             for value in task.pairs('SOURCE').values()) + '\nCOMMIT;')
    source_closed(task)
    task.state['sourceQuarantined'] = True
    task.save()
    return {'status': 'PHASE23_SOURCE_QUARANTINED', 'databaseConnections': 0}


def rotate_passwords(task, args):
    need(not any(task.service(service) for service in DAEMONS), 'WRITERS_NOT_STOPPED')
    source_closed(task)
    task.guard_databases('RESTORE', quiet=True)
    task.guard_roles()
    raw = task.read(args.passwords)
    target = task.json(args.environment)
    task.environment_binding(target, 'RESTORE')
    need(target['phase23']['runtimePasswordsFile'] == str(args.passwords)
         and target['phase23']['runtimePasswordsSha256'] == sha(raw), 'PASSWORD_MATERIAL_BINDING_INVALID')
    values = json.loads(raw)
    need(set(values) == set(SERVICES), 'PASSWORD_MAP_INVALID')
    prefix = task.resources()['source']['rolePrefix']
    seen, statements = set(), []
    source = task.json(TASK / 'phase23-source-environment.json')
    old = {unquote(urlsplit(source[kind]['DATABASE_URL']).password or '') for kind in SERVICES}
    for kind in SERVICES:
        row = values[kind]
        need(set(row) == {'role', 'password'} and row['role'] == prefix + '_' + kind + '_runtime'
             and isinstance(row['password'], str) and 32 <= len(row['password']) <= 1024
             and all(33 <= ord(char) <= 126 for char in row['password'])
             and row['password'] not in seen | old
             and unquote(urlsplit(target[kind]['DATABASE_URL']).password or '') == row['password'], 'PASSWORD_MAP_INVALID')
        seen.add(row['password'])
        statements.append('ALTER ROLE "' + row['role'] + '" PASSWORD ' + literal(row['password']) + ';')
    task.pg("BEGIN; SET LOCAL password_encryption='scram-sha-256';\n" + '\n'.join(statements) + '\nCOMMIT;')
    task.state['runtimePasswordsRotated'] = True
    task.save()
    return {'status': 'PHASE23_RUNTIME_PASSWORDS_ROTATED', 'roles': 3, 'realScramProbeStillRequired': True}


def use_target(task, args):
    need(not any(task.service(service) for service in DAEMONS), 'WRITERS_NOT_STOPPED')
    source_closed(task)
    task.guard_databases('SOURCE', quiet=True)
    records = task.guard_databases('RESTORE', quiet=True)
    task.guard_roles()
    need(all(record['limit'] == 0 for record in records.values()), 'TARGET_NOT_QUARANTINED')
    candidate = task.read(args.environment)
    environment = json.loads(candidate)
    task.environment_binding(environment, 'RESTORE')
    receipt = task.json(args.gate_receipt)
    need(receipt['status'] == 'GATE_PASSED_QUARANTINED' and receipt['opensTraffic'] is False
         and type(receipt['sequence']) is int and receipt['sequence'] >= 1, 'GATE_RECEIPT_INVALID')
    fresh = task.core('gate', args.recovery_config, receipt['sequence'], args.environment, sha(candidate))
    need(fresh == receipt and set(receipt['expectedConnectionLimits']) == {value['database'] for value in task.pairs('RESTORE').values()}
         and all(limit == -1 for limit in receipt['expectedConnectionLimits'].values()), 'GATE_RECEIPT_CHANGED')
    task.write(TASK / 'https-environment.json', candidate, replace=True)
    public = {key: environment['phase21'][key] for key in ('rp', 'machine', 'resource')}
    task.write(TASK / 'phase23-public.json', public, replace=True)
    task.state.update({'mode': 'RESTORE', 'gate': receipt, 'targetEnvironmentSha256': sha(candidate), 'targetOpen': False})
    task.save()
    return {'status': 'PHASE23_TARGET_SELECTED_QUARANTINED', 'standardRpRestarted': False, 'collectorRestarted': False}


def open_target(task, _args):
    need(task.state['mode'] == 'RESTORE' and task.state['gate'], 'TARGET_GATE_MISSING')
    source_closed(task)
    task.guard_roles()
    records = task.guard_databases('RESTORE', quiet=True)
    if task.state['targetOpen']:
        need(all(value['limit'] == -1 for value in records.values()), 'TARGET_LIMIT_CHANGED')
    else:
        need(all(value['limit'] == 0 for value in records.values()), 'TARGET_LIMIT_CHANGED')
        task.pg('BEGIN;\n' + '\n'.join('ALTER DATABASE "' + name + '" CONNECTION LIMIT -1;'
                 for name in task.state['gate']['expectedConnectionLimits']) + '\nCOMMIT;')
        need(all(value['limit'] == -1 for value in task.guard_databases('RESTORE', quiet=True).values()), 'TARGET_OPEN_FAILED')
        task.state['targetOpen'] = True
        task.save()
    return {'status': 'PHASE23_TARGET_OPENED', 'databaseLimitsRestored': 2, 'aclChanged': False}


def restart_auxiliary(task, args):
    kind = 'rp' if args.action == 'restart-rp' else 'collector'
    task.stop_auxiliary(kind)
    native = TASK / ('phase23-rp.pid' if kind == 'rp' else 'collector.pid')
    deadline = time.monotonic() + 3
    while native.exists() and time.monotonic() < deadline:
        time.sleep(.1)
    need(not native.exists(), 'AUXILIARY_LEASE_REMAINS')
    auxiliary(task, kind)
    if kind == 'rp':
        task.health('https://customer.saas.test:24446', '/')
    return {'status': 'PHASE23_' + kind.upper() + '_RESTARTED', 'oldCarrierProofClaimed': False}


def status(task, _args):
    result = {'status': 'PHASE23_STATUS', 'prepared': bool(task.state and task.state['prepared']),
              'mode': task.state['mode'] if task.state else None, 'containersCreated': 0}
    if task.state:
        result['runningTrackedProcesses'] = sum(task.alive(value) for value in task.state['processes'].values())
        result['coreServiceRecords'] = sum(bool(task.service(service)) for service in DAEMONS)
        result['identityInstances'] = task.state.get('identityInstances')
        result['targetOpen'] = task.state['targetOpen']
        result['lastVerifiedReadiness'] = task.state.get('lastVerifiedReadiness')
    return result


def cleanup(task, args):
    errors = []
    report = {'status': 'PHASE23_CLEANUP', 'result': 'PASS', 'awaitHba': not args.hba_restored,
              'containersCreated': 0, 'containersReused': 0, 'errors': errors, 'processesRemaining': 0,
              'databasesRemoved': 0, 'rolesRemoved': 0, 'privateFilesRemoved': 0, 'privateBytesRemoved': 0}
    def step(code, action):
        try:
            return action()
        except BaseException:
            errors.append(code)
            report['result'] = 'FAIL'
            return None
    ownership_evidence = {}
    for name in ('https-environment.json', 'phase23-source-environment.json', 'https-database.json',
                 'phase23-resources.json'):
        path = TASK / name
        if path.exists():
            raw = step('OWNERSHIP_EVIDENCE_READ', lambda path=path: task.read(path))
            if raw is not None:
                ownership_evidence[name] = raw
    collector_candidates = set()
    step('STOP_PENDING_SPAWNS', task.stop_pending_spawns)
    report['processesRemaining'] += len(list(TASK.glob('phase23-spawn-*.json')))
    report['processesRemaining'] += len(task.state.get('pendingSpawns', {})) if task.state else 0
    if task.state:
        for service in DAEMONS:
            step('STOP_' + service.upper(), lambda service=service: task.stop_service(service))
        if task.state['processes'].get('collector'):
            def collector_canaries():
                import http.client
                mode = task.state.get('collectorMode', 'SOURCE')
                environment = task.json(TASK / ('phase23-source-environment.json' if mode == 'SOURCE' else 'https-environment.json'))
                task.guard_databases(mode)
                database = task.pairs(mode)['platform']['database']
                references = task.pg('SELECT id::text FROM auth_invitations ORDER BY created_at DESC,id LIMIT 32', database).splitlines()
                compared = 0
                for reference in references:
                    need(str(uuid.UUID(reference)) == reference and uuid.UUID(reference).version == 7, 'INVITATION_ID_INVALID')
                    connection = http.client.HTTPConnection('127.0.0.1', 23443, timeout=2)
                    try:
                        connection.request('POST', '/control/peek', json.dumps({'kind': 'INVITATION', 'invitationId': reference}),
                            {'Content-Type': 'application/json', 'X-Task-Control-Key': environment['phase20']['collector']['controlSecret']})
                        response = connection.getresponse()
                        data = response.read(65537)
                        need(len(data) <= 65536 and response.status in (200, 404), 'COLLECTOR_CANARY_READ_FAILED')
                        if response.status == 200:
                            token = json.loads(data)['message']['token']
                            need(isinstance(token, str) and 1 <= len(token) <= 8192, 'COLLECTOR_CANARY_INVALID')
                            collector_candidates.add(token.encode())
                            compared += 1
                    finally:
                        connection.close()
                report['collectorPrivateMessagesComparedOnlyInMemory'] = compared
                log_paths = [path for path in TASK.iterdir() if path.is_file() and path.suffix == '.log'
                             and 'hba' not in path.name.lower()
                             and (path.name.startswith(('phase23-', 'service-')) or path.name == 'gateway.log')]
                findings = 0
                total = 0
                for path in log_paths:
                    data = task.read(path, maximum=32 * 1024 * 1024, empty_ok=True)
                    total += len(data)
                    need(total <= 64 * 1024 * 1024, 'LOG_SCAN_BUDGET_EXCEEDED')
                    findings += sum(token in data for token in collector_candidates)
                task.state['collectorCanaryScan'] = {'messagesComparedOnlyInMemory': compared,
                    'logFiles': len(log_paths), 'logBytes': total, 'candidateFindings': findings}
                task.save()
                need(findings == 0, 'COLLECTOR_PRIVATE_CANARY_FOUND')
            step('COLLECTOR_MEMORY_CANARY_SCAN', collector_canaries)
        for kind in ('rp', 'collector', 'scram'):
            step('STOP_' + kind.upper(), lambda kind=kind: task.stop_auxiliary(kind))
        if (TASK / 'namespace.pid').exists():
            def stop_namespace():
                task.namespace()
                environment = {'PATH': '/usr/local/bin:/usr/bin:/bin', 'TASK_GATEWAY_SCRIPT': 'phase23-gateway.py'}
                run(['bash', task.tools() / 'environment.sh', 'stop', '--task-dir', TASK], environment=environment, timeout=35)
            step('STOP_NAMESPACE', stop_namespace)
        for kind in ('gateway', 'namespace'):
            step('STOP_' + kind.upper(), lambda kind=kind: task.stop_process(kind))
        for value in task.state['processes'].values():
            alive = step('PROCESS_IDENTITY_CHECK', lambda value=value: task.alive(value))
            report['processesRemaining'] += int(alive is not False)
    if args.hba_restored and not report['processesRemaining']:
        # The root HBA owner acknowledges restoration; this wrapper never reads,
        # changes, restores or deletes its HBA journal or private ownership files.
        empty_preparation = (task.state is not None and task.state.get('prepared') is False
            and not (TASK / 'https-database.json').exists()
            and not (TASK / 'phase23-resources.json').exists())
        if empty_preparation:
            resources = None
            empty = step('EMPTY_PREPARATION_RESOURCE_GUARD', task.empty_preparation_resources)
            if empty is not None:
                report.update(empty)
        else:
            resources = step('RESOURCE_GUARD', task.resources)
        if resources:
            pairs = {'RESTORE': task.pairs('RESTORE'), 'SOURCE': task.pairs('SOURCE')}
            for mode in ('RESTORE', 'SOURCE'):
                for kind, expected in pairs[mode].items():
                    def drop_database(mode=mode, kind=kind, expected=expected):
                        records = task.guard_databases(mode, quiet=True, absent_ok=True)
                        if kind not in records:
                            return
                        report.setdefault('databaseBytesBeforeDrop', []).append(records[kind]['size'])
                        task.pg('DROP DATABASE "' + expected['database'] + '" WITH (FORCE);')
                        need(task.pg('SELECT count(*) FROM pg_database WHERE datname=' + literal(expected['database'])) == '0', 'DROP_DATABASE_FAILED')
                        report['databasesRemoved'] += 1
                    step('DROP_' + mode + '_' + kind.upper(), drop_database)
            all_names = [value['database'] for mode in pairs.values() for value in mode.values()]
            databases_left = step('DATABASE_RESIDUAL_CHECK', lambda: task.pg('SELECT count(*) FROM pg_database WHERE datname=ANY(ARRAY[' + ','.join(map(literal, all_names)) + '])'))
            report['databasesRemaining'] = int(databases_left) if databases_left is not None else -1
            if databases_left == '0':
                prefix = resources['source']['rolePrefix']
                for suffix in ROLES:
                    role = prefix + '_' + suffix
                    def drop_role(role=role):
                        record = task.pg("SELECT jsonb_build_object('super',rolsuper,'createDb',rolcreatedb,'createRole',rolcreaterole,'replication',rolreplication,'bypass',rolbypassrls,'inherit',rolinherit)::text FROM pg_roles WHERE rolname=" + literal(role))
                        if not record:
                            return
                        need(not any(json.loads(record).values()) and task.pg('SELECT count(*) FROM pg_auth_members membership JOIN pg_roles role_row ON role_row.oid IN(membership.roleid,membership.member) WHERE role_row.rolname=' + literal(role)) == '0', 'ROLE_CLOSURE_CHANGED')
                        task.pg('DROP ROLE "' + role + '";')
                        report['rolesRemoved'] += 1
                    step('DROP_ROLE_' + suffix.upper(), drop_role)
                roles_left = step('ROLE_RESIDUAL_CHECK', lambda: task.pg('SELECT count(*) FROM pg_roles WHERE rolname=ANY(ARRAY[' + ','.join(literal(prefix + '_' + suffix) for suffix in ROLES) + '])'))
                report['rolesRemaining'] = int(roles_left) if roles_left is not None else -1
        if not errors:
            # Secret candidates remain in memory, never in this safe report.
            candidates, logs = set(collector_candidates), []
            def values(value, key=''):
                if isinstance(value, dict):
                    for name, item in value.items():
                        values(item, name)
                elif isinstance(value, list):
                    for item in value:
                        values(item, key)
                elif isinstance(value, str) and len(value) >= 16 and re.search(r'password|secret|token|csrf|cookie|verifier|^code$|privatekeypem|key$|username', key, re.I):
                    candidates.add(value.encode())
            prefixes = ('phase23-', 'https-', 'phase21-third-secret', 'phase21-offline-secret', 'phase21-machine-secret', 'phase21-resource-secret')
            evidence_names = {'phase23-environment-state.json', 'phase23-resources.json', 'https-database.json',
                              'https-environment.json', 'phase23-source-environment.json'}
            files = [path for path in TASK.iterdir() if path.is_file() and (path.name.startswith(prefixes) or path.name == 'phase21-public.json')
                     and path.name != cluster_binding.NAME
                     and 'hba' not in path.name.lower() and not path.name.startswith('phase23-environment-cleanup')]
            for path in files:
                if path.suffix == '.json':
                    step('PRIVATE_JSON_SCAN', lambda path=path: values(task.json(path)))
                elif 'password' in path.name or path.name.endswith('-secret'):
                    step('PRIVATE_SECRET_SCAN', lambda path=path: candidates.add(task.read(path).strip()))
            log_paths = [path for path in TASK.iterdir() if path.is_file() and path.suffix == '.log'
                         and 'hba' not in path.name.lower()
                         and (path.name.startswith(('phase23-', 'service-')) or path.name == 'gateway.log')]
            for path in log_paths:
                data = step('LOG_SCAN', lambda path=path: task.read(path, maximum=32 * 1024 * 1024, empty_ok=True))
                if data is not None:
                    logs.append(data)
            if sum(map(len, logs)) > 64 * 1024 * 1024:
                errors.append('LOG_SCAN_BUDGET_EXCEEDED')
            report['logRedaction'] = {'files': len(logs), 'candidatesComparedInMemory': len(candidates),
                                     'candidateFindings': sum(any(secret in data for data in logs) for secret in candidates)}
            if report['logRedaction']['candidateFindings']:
                errors.append('LOG_PRIVATE_CANDIDATE_FOUND')
            if not errors:
                directories = ('phase23-gateway-workspace', 'phase23-rp-source', 'phase23-recovery-private', 'phase23-recovery-witness')
                for name in directories:
                    directory = TASK / name
                    def remove_directory(directory=directory):
                        if not directory.exists():
                            return
                        need(directory.is_dir() and not directory.is_symlink() and directory.resolve().parent == TASK
                             and directory.stat().st_uid == task.uid, 'PRIVATE_DIRECTORY_GUARD')
                        for root, subdirs, names in os.walk(directory, followlinks=False):
                            for child in subdirs:
                                entry = Path(root) / child
                                if entry.is_symlink():
                                    need(entry == TASK / 'phase23-rp-source/node_modules'
                                         and entry.resolve() == (TASK / 'phase21-rp-source/node_modules').resolve(), 'UNEXPECTED_DIRECTORY_LINK')
                            for child in names:
                                entry = Path(root) / child
                                need(not entry.is_symlink() and entry.stat().st_uid == task.uid, 'PRIVATE_DIRECTORY_FILE_GUARD')
                        shutil.rmtree(directory)
                    step('REMOVE_PRIVATE_DIRECTORY', remove_directory)
                for path in set(files + log_paths + [TASK / 'hosts', TASK / 'environment-controller.sh', TASK / 'namespace.stop', TASK / 'collector.stop']):
                    if not path.exists():
                        continue
                    if path.name in evidence_names:
                        continue
                    def remove_file(path=path):
                        task.checked(path, private=path.name not in ('hosts', 'environment-controller.sh', 'namespace.stop', 'collector.stop'), empty_ok=True)
                        size = path.stat().st_size
                        path.unlink()
                        report['privateFilesRemoved'] += 1
                        report['privateBytesRemoved'] += size
                    step('REMOVE_PRIVATE_FILE', remove_file)
    report['awaitHba'] = not args.hba_restored
    if report['processesRemaining'] or (args.hba_restored and (report.get('databasesRemaining', -1) != 0 or report.get('rolesRemaining', -1) != 0)):
        errors.append('RESIDUAL_RESOURCES')
    report['result'] = 'FAIL' if errors else ('AWAIT_HBA' if report['awaitHba'] else 'PASS')
    report['dockerBefore'] = task.state.get('dockerBefore') if task.state else None
    report['filesystemBefore'] = task.state.get('filesystemBefore') if task.state else None
    report['dockerAfter'] = step('DOCKER_DF', lambda: run(['docker', 'system', 'df']).decode())
    report['filesystemAfter'] = step('FILESYSTEM_DF', lambda: run(['df', '-B1', TASK]).decode())
    report['retainedSharedPostgresAndPublicBuildInputs'] = True
    report['windowsWslVdiskCompactionClaimed'] = False
    report['rootHbaJournalRemovedByThisWrapper'] = False
    report['collectorCanaryScan'] = task.state.get('collectorCanaryScan') if task.state else None
    lease_names = ('namespace.pid', 'gateway.pid', 'phase23-rp.pid', 'collector.pid', 'phase23-scram-proxy.pid',
                   *('service-' + name + '.pid' for name in DAEMONS))
    report['remainingLeaseRecords'] = sum((TASK / name).exists() for name in lease_names)
    if report['remainingLeaseRecords']:
        errors.append('LEASE_RECORDS_REMAIN')
    if not errors and args.hba_restored:
        for name in ('https-environment.json', 'phase23-source-environment.json', 'https-database.json',
                     'phase23-resources.json', 'phase23-environment-state.json'):
            path = TASK / name
            if path.exists():
                step('REMOVE_FINAL_OWNERSHIP_EVIDENCE', lambda path=path: (task.checked(path), path.unlink()))
    if errors:
        # Retain exact protected evidence after any partial final-file deletion.
        # Never overwrite an existing record or repair a changed ownership fact.
        for name, raw in ownership_evidence.items():
            if not (TASK / name).exists():
                step('RETAIN_FAILURE_OWNERSHIP_RECORD', lambda name=name, raw=raw: task.write(TASK / name, raw))
        if task.state and not task.state_path.exists():
            step('RETAIN_FAILURE_STATE_RECORD', task.save)
    report['result'] = 'FAIL' if errors else ('AWAIT_HBA' if report['awaitHba'] else 'PASS')
    task.write(TASK / 'phase23-environment-cleanup.json', report, replace=(TASK / 'phase23-environment-cleanup.json').exists())
    return report


ACTIONS = {'prepare': prepare, 'start': start, 'select-tools': select_tools,
           'stop-writers': stop_writers, 'start-writers': start_writers,
           'profile': profile, 'use-target': use_target, 'quarantine-source': quarantine_source,
           'open-target': open_target, 'rotate-db-passwords': rotate_passwords,
           'restart-rp': restart_auxiliary, 'restart-collector': restart_auxiliary, 'cleanup': cleanup, 'status': status}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=ACTIONS)
    parser.add_argument('--task-dir', type=Path, required=True)
    parser.add_argument('--snapshot')
    parser.add_argument('--build-snapshot', help='App/probe binary attestation snapshot; selected ops snapshot may differ only outside compiled closure')
    parser.add_argument('--ttl-seconds', type=int)
    parser.add_argument('--environment', type=Path)
    parser.add_argument('--passwords', type=Path)
    parser.add_argument('--gate-receipt', type=Path)
    parser.add_argument('--recovery-config', type=Path)
    parser.add_argument('--hba-restored', action='store_true', help='Root HBA owner confirms its own finally restoration; wrapper never touches HBA')
    args = parser.parse_args()
    need(args.task_dir == TASK, 'TASK_ARGUMENT_INVALID')
    task = Task()
    if args.action != 'prepare':
        need(task.state is not None and task.state['contractVersion'] == 'unified-auth-phase23-environment/1', 'ENVIRONMENT_RECORD_REQUIRED')
    result = ACTIONS[args.action](task, args)
    print(json.dumps(result, sort_keys=True))
    return 1 if result.get('result') == 'FAIL' else 0


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except Exception as error:
        print(safe_failure_code(error), file=sys.stderr)
        raise SystemExit(1) from None
