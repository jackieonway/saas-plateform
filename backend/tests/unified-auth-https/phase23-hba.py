"""Root-only, journaled SCRAM rules for one bound Phase 23 PostgreSQL cluster.

install|restore|status|cleanup-receipt --task-dir TASK --deployment-id UUIDV7
exec --task-dir TASK --deployment-id UUIDV7 --command PROGRAM [ARG ...]

The Python installed_hba() context manager restores on body/installation failure.
Only this task's 55494 cluster and exact four DB / three LOGIN-role pairs change.
No credentials, HBA bytes, SQL output or child exceptions are printed.
"""
import argparse
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import stat
import subprocess
import sys
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parent))
import phase23_cluster_binding as cluster_binding

ROLES = ('identity_runtime', 'platform_runtime', 'portal_runtime', 'identity_owner',
         'platform_owner', 'bridge_owner', 'migration_owner', 'portal_owner')
FORMAT = 'unified-auth-phase23-hba-journal/1'
MAXIMUM = 1024 * 1024


def need(value, code):
    if not value:
        raise RuntimeError('PHASE23_HBA_' + code)


def sha(value):
    return hashlib.sha256(value).hexdigest()


def literal(value):
    need(type(value) is str and '\x00' not in value, 'SQL_LITERAL_INVALID')
    return "'" + value.replace("'", "''") + "'"


def metadata(info):
    return {'uid': info.st_uid, 'gid': info.st_gid, 'mode': stat.S_IMODE(info.st_mode),
            'atimeNs': info.st_atime_ns, 'mtimeNs': info.st_mtime_ns}


def stable_metadata(value):
    # PostgreSQL or an independent SHA256 reader can advance atime without
    # changing the file. Initial restoration includes it; retained receipts pin
    # ownership, permissions, mtime and exact bytes, not subsequent read access.
    return {key: item for key, item in value.items() if key != 'atimeNs'}


def pg(binding, sql, database='postgres'):
    need(re.fullmatch(r'[a-z][a-z0-9_]{0,62}', database), 'DATABASE_IDENTIFIER_INVALID')
    environment = {'PATH': '/usr/lib/postgresql/18/bin:/usr/bin:/bin', 'PGHOST': '127.0.0.1',
        'PGPORT': '55494', 'PGUSER': 'postgres', 'PGDATABASE': database, 'PGCONNECT_TIMEOUT': '5',
        'PGAPPNAME': 'phase23-owned-hba', 'PGOPTIONS': '-c log_statement=none -c log_min_error_statement=panic'}
    query = ("DO $$ BEGIN IF NOT (" + cluster_binding.sql_guard(binding)
             + ") THEN RAISE EXCEPTION 'PHASE23_HBA_CLUSTER'; END IF; END $$;\n" + sql)
    try:
        result = subprocess.run(['/usr/lib/postgresql/18/bin/psql', '-X', '-qAt', '-v', 'ON_ERROR_STOP=1',
            '-v', 'VERBOSITY=sqlstate'], input=query.encode(), env=environment,
            capture_output=True, timeout=15)
    except (OSError, subprocess.TimeoutExpired):
        raise RuntimeError('PHASE23_HBA_DATABASE_UNAVAILABLE') from None
    need(result.returncode == 0 and len(result.stdout) <= MAXIMUM, 'DATABASE_QUERY_FAILED')
    try:
        return json.loads(result.stdout)
    except (ValueError, UnicodeDecodeError):
        raise RuntimeError('PHASE23_HBA_DATABASE_RESULT_INVALID') from None


def read_regular(path, uid, maximum=MAXIMUM):
    before = path.lstat()
    need(stat.S_ISREG(before.st_mode) and before.st_uid == uid and before.st_nlink == 1
         and not stat.S_IMODE(before.st_mode) & 0o022 and before.st_size <= maximum, 'FILE_INVALID')
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK
                         | getattr(os, 'O_NOATIME', 0))
    try:
        opened = os.fstat(descriptor)
        need((before.st_dev, before.st_ino) == (opened.st_dev, opened.st_ino), 'FILE_CHANGED')
        value = os.read(descriptor, maximum + 1)
        after = os.fstat(descriptor)
        need(len(value) == opened.st_size and (opened.st_dev, opened.st_ino, opened.st_size,
             opened.st_mtime_ns, opened.st_ctime_ns) == (after.st_dev, after.st_ino,
             after.st_size, after.st_mtime_ns, after.st_ctime_ns), 'FILE_CHANGED')
        return value, opened
    finally:
        os.close(descriptor)


def write_new(path, value):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        os.fchmod(descriptor, 0o600)
        os.fchown(descriptor, 0, 0)
        with os.fdopen(descriptor, 'wb', closefd=False) as output:
            output.write(value)
            output.flush()
            os.fsync(descriptor)
    finally:
        os.close(descriptor)


class Hba:
    def __init__(self, task_dir, deployment_id, *, journal_parent=Path('/run')):
        need(os.name == 'posix' and os.getuid() == 0, 'ROOT_REQUIRED')
        deployment = uuid.UUID(deployment_id)
        need(deployment.version == 7 and str(deployment) == deployment_id, 'DEPLOYMENT_INVALID')
        self.task_dir, self.deployment = Path(task_dir), deployment_id
        self.journal = Path(journal_parent) / ('rust-saas-phase23-hba-' + deployment_id)
        need(self.journal.parent.resolve(strict=True) == self.journal.parent, 'JOURNAL_PARENT_INVALID')
        parent = self.journal.parent.stat()
        need(parent.st_uid == 0 and not stat.S_IMODE(parent.st_mode) & 0o002,
             'JOURNAL_PARENT_INVALID')
        self.lock = None
        self.installed_by_me = False

    def binding(self):
        binding = cluster_binding.load_binding(self.task_dir)
        raw, _ = cluster_binding.protected_read(self.task_dir / 'https-database.json',
            uid=binding['taskUid'], mode=0o600, maximum=8192)
        try:
            manifest = json.loads(raw)
        except (ValueError, UnicodeDecodeError):
            raise RuntimeError('PHASE23_HBA_MANIFEST_INVALID') from None
        need(type(manifest) is dict and set(manifest) == {'database', 'rolePrefix', 'clusterId', 'portal'},
             'MANIFEST_INVALID')
        name, prefix = manifest['database'], manifest['rolePrefix']
        need(type(name) is str and re.fullmatch(r'unified_auth_https_[0-9a-f]{32}', name)
             and prefix == 'uah_' + name[-12:] and manifest['clusterId'] == binding['systemIdentifier'],
             'MANIFEST_BINDING_INVALID')
        portal = {'database': 'unified_auth_portal_' + name[-32:], 'owner': prefix + '_portal_owner',
            'runtime': prefix + '_portal_runtime', 'marker': 'unified-auth-https-portal/1/' + self.deployment,
            'initializationPending': False}
        need(manifest['portal'] == portal, 'PORTAL_BINDING_INVALID')
        targets = {'unified_auth_restore_' + name[-32:]: (prefix + '_migration_owner',
                       'phase23-recovery/2/' + self.deployment),
                   'unified_auth_restore_portal_' + name[-32:]: (prefix + '_portal_owner',
                       'phase23-recovery/2/' + self.deployment)}
        databases = {name: (prefix + '_migration_owner', 'identity-deployment/1/' + self.deployment),
                     portal['database']: (portal['owner'], portal['marker']), **targets}
        return binding, manifest, sha(raw), databases

    def guard(self):
        binding, manifest, digest, databases = self.binding()
        prefix = manifest['rolePrefix']
        actual = pg(binding, "SELECT jsonb_build_object('cluster',system_identifier::text,'data',"
            "current_setting('data_directory'),'hba',current_setting('hba_file'),'database',current_database(),"
            "'user',current_user,'host',host(inet_server_addr()),'port',inet_server_port(),'roles',"
            "(SELECT jsonb_agg(jsonb_build_array(rolname,rolsuper,rolcreatedb,rolcreaterole,rolinherit,rolreplication,"
            "rolbypassrls,rolcanlogin) ORDER BY rolname) FROM pg_roles WHERE starts_with(rolname,"
            + literal(prefix + '_') + ")),'memberships',(SELECT count(*) FROM pg_auth_members m JOIN pg_roles r "
            "ON r.oid IN(m.roleid,m.member) WHERE starts_with(r.rolname," + literal(prefix + '_') + ")),"
            "'databases',(SELECT COALESCE(jsonb_object_agg(datname,jsonb_build_array(pg_get_userbyid(datdba),"
            "shobj_description(oid,'pg_database'))),'{}'::jsonb) FROM pg_database WHERE datname=ANY(ARRAY["
            + ','.join(map(literal, databases)) + "]))) FROM pg_control_system()")
        need(actual.get('cluster') == binding['systemIdentifier'] and actual.get('data') == binding['dataDirectory']
             and actual.get('hba') == str(Path(binding['dataDirectory']) / 'pg_hba.conf')
             and actual.get('database') == 'postgres' and actual.get('user') == 'postgres'
             and actual.get('host') == '127.0.0.1' and actual.get('port') == 55494, 'ACTUAL_TARGET_CHANGED')
        expected = sorted(prefix + '_' + suffix for suffix in ROLES)
        roles = actual.get('roles')
        need(type(roles) is list and len(roles) == 8 and [row[0] for row in roles] == expected
             and actual.get('memberships') == 0, 'ROLE_SET_CHANGED')
        for role in roles:
            need(len(role) == 8 and all(type(flag) is bool for flag in role[1:])
                 and not any(role[1:7]) and role[7] == role[0].endswith('_runtime'), 'ROLE_POWERS_CHANGED')
        observed = actual.get('databases')
        need(type(observed) is dict and manifest['database'] in observed and manifest['portal']['database'] in observed
             and set(observed) <= set(databases)
             and all(value == list(databases[name]) for name, value in observed.items()), 'DATABASE_OWNERSHIP_CHANGED')
        state = pg(binding, "SELECT to_jsonb(d) FROM identity_deployment_state d WHERE singleton", manifest['database'])
        expected_state = {'deployment_id': self.deployment, 'migration_owner': prefix + '_migration_owner',
            'identity_owner': prefix + '_identity_owner', 'platform_owner': prefix + '_platform_owner',
            'projection_owner': prefix + '_identity_owner', 'revocation_bridge_owner': prefix + '_bridge_owner',
            'identity_runtime': prefix + '_identity_runtime', 'platform_runtime': prefix + '_platform_runtime'}
        need(type(state) is dict and all(state.get(key) == value for key, value in expected_state.items()),
             'DEPLOYMENT_ROLE_BINDING_CHANGED')
        return binding, manifest, digest, Path(actual['hba'])

    def rules(self, manifest):
        shared, portal, prefix = manifest['database'], manifest['portal']['database'], manifest['rolePrefix']
        block = ('# BEGIN phase23 owned SCRAM ' + self.deployment + '\n'
            'host ' + shared + ',unified_auth_restore_' + shared[-32:] + ' '
            + prefix + '_identity_runtime,' + prefix + '_platform_runtime 127.0.0.2/32 scram-sha-256\n'
            'host ' + portal + ',unified_auth_restore_portal_' + shared[-32:] + ' '
            + prefix + '_portal_runtime 127.0.0.2/32 scram-sha-256\n'
            '# END phase23 owned SCRAM ' + self.deployment + '\n')
        return block.encode('ascii')

    def acquire(self, create=False):
        if create:
            os.mkdir(self.journal, 0o700)
            os.chmod(self.journal, 0o700)
            write_new(self.journal / 'LOCK', b'phase23-owned-hba\n')
        details = self.journal.lstat()
        need(stat.S_ISDIR(details.st_mode) and details.st_uid == 0 and stat.S_IMODE(details.st_mode) == 0o700
             and not self.journal.is_symlink(), 'JOURNAL_OWNERSHIP_CHANGED')
        raw, info = read_regular(self.journal / 'LOCK', 0)
        need(raw == b'phase23-owned-hba\n' and stat.S_IMODE(info.st_mode) == 0o600, 'JOURNAL_LOCK_INVALID')
        self.lock = os.open(self.journal / 'LOCK', os.O_RDONLY | os.O_NOFOLLOW)
        try:
            fcntl.flock(self.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BaseException:
            os.close(self.lock)
            self.lock = None
            raise

    def release(self):
        if self.lock is not None:
            os.close(self.lock)
            self.lock = None

    def save(self, value, first=False):
        raw = (json.dumps(value, sort_keys=True) + '\n').encode()
        if first:
            write_new(self.journal / 'journal.json', raw)
        else:
            temporary = self.journal / ('journal-' + uuid.uuid4().hex)
            write_new(temporary, raw)
            os.replace(temporary, self.journal / 'journal.json')
        descriptor = os.open(self.journal, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)

    def load(self):
        raw, info = read_regular(self.journal / 'journal.json', 0)
        need(stat.S_IMODE(info.st_mode) == 0o600, 'JOURNAL_MODE_CHANGED')
        value = json.loads(raw)
        need(value['contractVersion'] == FORMAT and value['taskDir'] == str(self.task_dir)
             and value['deploymentId'] == self.deployment, 'JOURNAL_BINDING_CHANGED')
        original = None
        if value['state'] != 'RESTORED' or (self.journal / 'original.hba').exists():
            original, original_info = read_regular(self.journal / 'original.hba', 0)
            need(stat.S_IMODE(original_info.st_mode) == 0o600 and sha(original) == value['originalSha256'],
                 'JOURNAL_ORIGINAL_CHANGED')
        return value, original

    def replace_hba(self, path, expected, replacement, preserved, binding):
        # Revalidate the actual PG target immediately before any file mutation.
        current_binding, _, _, current_path = self.guard()
        need(current_binding == binding and current_path == path, 'ACTUAL_TARGET_CHANGED')
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        temporary = '.phase23-hba-' + uuid.uuid4().hex
        try:
            before, info = read_regular(path, binding['taskUid'])
            need(before == expected, 'EXTERNAL_CHANGE_PRESERVED')
            descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                                 0o600, dir_fd=directory)
            try:
                with os.fdopen(descriptor, 'wb', closefd=False) as output:
                    output.write(replacement)
                    output.flush()
                    os.fchown(descriptor, preserved['uid'], preserved['gid'])
                    os.fchmod(descriptor, preserved['mode'])
                    os.utime(descriptor, ns=(preserved['atimeNs'], preserved['mtimeNs']))
                    os.fsync(descriptor)
            finally:
                os.close(descriptor)
            again, current = read_regular(path, binding['taskUid'])
            need(again == expected and (current.st_dev, current.st_ino, current.st_ctime_ns) ==
                 (info.st_dev, info.st_ino, info.st_ctime_ns), 'EXTERNAL_CHANGE_PRESERVED')
            os.replace(temporary, path.name, src_dir_fd=directory, dst_dir_fd=directory)
            os.fsync(directory)
        finally:
            try:
                os.unlink(temporary, dir_fd=directory)
            except FileNotFoundError:
                pass
            os.close(directory)

    def reload(self, binding):
        result = pg(binding, "SELECT jsonb_build_object('reloaded',pg_reload_conf(),'errors',"
                    "(SELECT count(*) FROM pg_hba_file_rules WHERE error IS NOT NULL))")
        need(result == {'reloaded': True, 'errors': 0}, 'RELOAD_OR_PARSE_FAILED')

    def install(self):
        binding, manifest, digest, path = self.guard()
        original, info = read_regular(path, binding['taskUid'])
        need(b'# BEGIN phase23 owned SCRAM ' not in original, 'PREEXISTING_PHASE23_RULES')
        installed = self.rules(manifest) + original
        self.acquire(create=True)
        try:
            write_new(self.journal / 'original.hba', original)
            value = {'contractVersion': FORMAT, 'taskDir': str(self.task_dir), 'deploymentId': self.deployment,
                'clusterBinding': binding, 'manifestSha256': digest, 'hba': str(path), 'originalStat': metadata(info),
                'originalSha256': sha(original), 'installedSha256': sha(installed), 'state': 'PREPARED'}
            self.save(value, first=True)
            try:
                self.replace_hba(path, original, installed, value['originalStat'], binding)
                value['state'] = 'INSTALLED'
                self.save(value)
                self.reload(binding)
                self.installed_by_me = True
                return self.result(value, 'PHASE23_HBA_INSTALLED')
            except BaseException:
                self.restore_locked()
                raise
        finally:
            self.release()

    def result(self, value, status):
        return {'status': status, 'journal': str(self.journal), 'originalSha256': value['originalSha256'],
                'installedSha256': value['installedSha256'], 'ruleCount': 2,
                'sourceAddress': '127.0.0.2/32', 'port': 55494}

    def restore_locked(self):
        value, original = self.load()
        binding, manifest, digest, path = self.guard()
        need(binding == value['clusterBinding'] and digest == value['manifestSha256'] and str(path) == value['hba'],
             'JOURNAL_ACTUAL_BINDING_CHANGED')
        if value['state'] == 'RESTORED':
            return self.verify_restored(value, path, binding)
        installed = self.rules(manifest) + original
        need(sha(installed) == value['installedSha256'], 'JOURNAL_INSTALLED_CHANGED')
        current, _ = read_regular(path, binding['taskUid'])
        if current == installed:
            value['state'] = 'RESTORING'
            self.save(value)
            self.replace_hba(path, installed, original, value['originalStat'], binding)
        else:
            need(current == original and value['state'] in ('PREPARED', 'RESTORING', 'RESTORED'),
                 'EXTERNAL_CHANGE_PRESERVED')
        self.reload(binding)
        # Reading the bytes may update atime. Restore the saved timestamps only
        # after byte verification, then verify stat without another file read.
        actual, _ = read_regular(path, binding['taskUid'])
        need(actual == original, 'RESTORED_BYTES_CHANGED')
        os.utime(path, ns=(value['originalStat']['atimeNs'], value['originalStat']['mtimeNs']), follow_symlinks=False)
        need(metadata(path.lstat()) == value['originalStat'], 'RESTORED_STAT_CHANGED')
        value['state'] = 'RESTORED'
        value['finalSha256'] = sha(actual)
        value['finalStat'] = metadata(path.lstat())
        self.save(value)
        (self.journal / 'original.hba').unlink()
        return self.verify_restored(value, path, binding)

    def verify_restored(self, value, path, binding):
        need(value['state'] == 'RESTORED' and value.get('finalSha256') == value['originalSha256']
             and value.get('finalStat') == value['originalStat'], 'RESTORATION_RECEIPT_INVALID')
        actual, info = read_regular(path, binding['taskUid'])
        need(sha(actual) == value['finalSha256']
             and stable_metadata(metadata(info)) == stable_metadata(value['finalStat']),
             'RESTORED_RECEIPT_ACTUAL_CHANGED')
        return {**self.result(value, 'PHASE23_HBA_RESTORED'), 'receiptRetained': True,
                'originalBytesAndStatRestored': True, 'finalSha256': value['finalSha256'],
                'finalStat': value['finalStat']}

    def restore(self):
        self.acquire()
        try:
            return self.restore_locked()
        finally:
            self.release()

    def status(self):
        if not self.journal.exists():
            return {'status': 'PHASE23_HBA_JOURNAL_ABSENT', 'journal': str(self.journal)}
        self.acquire()
        try:
            value, _ = self.load()
            binding, _, digest, path = self.guard()
            need(binding == value['clusterBinding'] and digest == value['manifestSha256'] and str(path) == value['hba'],
                 'JOURNAL_ACTUAL_BINDING_CHANGED')
            if value['state'] == 'RESTORED':
                return self.verify_restored(value, path, binding)
            return {**self.result(value, 'PHASE23_HBA_JOURNALED'), 'state': value['state']}
        finally:
            self.release()

    def cleanup_receipt(self):
        """Call after saving restore proof, before the environment drops its DBs."""
        self.acquire()
        try:
            value, _ = self.load()
            need(value['state'] == 'RESTORED', 'RESTORATION_RECEIPT_REQUIRED')
            result = self.restore_locked()
            need(result['receiptRetained'], 'RESTORATION_RECEIPT_REQUIRED')
            need(set(item.name for item in self.journal.iterdir()) <= {'LOCK', 'original.hba', 'journal.json'},
                 'JOURNAL_UNKNOWN_FILES')
            for name in ('original.hba', 'journal.json', 'LOCK'):
                try:
                    (self.journal / name).unlink()
                except FileNotFoundError:
                    need(name == 'original.hba', 'JOURNAL_FILE_MISSING')
            self.journal.rmdir()
            return {**result, 'status': 'PHASE23_HBA_RECEIPT_CLEANED', 'receiptRetained': False,
                    'journalRemoved': True}
        finally:
            self.release()


@contextlib.contextmanager
def installed_hba(task_dir, deployment_id, **kwargs):
    controller = Hba(task_dir, deployment_id, **kwargs)
    try:
        controller.install()
        yield controller
    finally:
        # Cover the signal boundary between install returning and yield; never
        # restore a preexisting journal belonging to a different caller.
        if controller.installed_by_me:
            controller.restore()


@contextlib.contextmanager
def interrupts():
    previous = {}
    def stop(signum, frame):
        raise KeyboardInterrupt
    try:
        for signum in (signal.SIGTERM, signal.SIGHUP):
            previous[signum] = signal.signal(signum, stop)
        yield
    finally:
        for signum, handler in previous.items():
            signal.signal(signum, handler)


def run_command(command):
    need(bool(command), 'COMMAND_REQUIRED')
    process = subprocess.Popen(command, start_new_session=True)
    try:
        return process.wait()
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=5)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('install', 'restore', 'status', 'cleanup-receipt', 'exec'))
    parser.add_argument('--task-dir', type=Path, required=True)
    parser.add_argument('--deployment-id', required=True)
    parser.add_argument('--command', nargs=argparse.REMAINDER)
    args = parser.parse_args(argv)
    try:
        with interrupts():
            if args.action == 'exec':
                with installed_hba(args.task_dir, args.deployment_id):
                    return run_command(args.command)
            need(not args.command, 'COMMAND_ONLY_FOR_EXEC')
            result = getattr(Hba(args.task_dir, args.deployment_id), args.action.replace('-', '_'))()
            print(json.dumps(result, sort_keys=True))
            return 0
    except BaseException as error:
        code = str(error) if isinstance(error, RuntimeError) else 'PHASE23_HBA_REJECTED'
        if not re.fullmatch(r'PHASE23_(?:HBA|CLUSTER_BINDING)_[A-Z0-9_]+', code):
            code = 'PHASE23_HBA_REJECTED'
        print(json.dumps({'status': 'FAIL_JOURNAL_PRESERVED_IF_PRESENT', 'code': code}), file=sys.stderr)
        return 130 if isinstance(error, KeyboardInterrupt) else 2


if __name__ == '__main__':
    raise SystemExit(main())
