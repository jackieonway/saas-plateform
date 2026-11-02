"""Root-created nonsecret binding for a newly created owned PostgreSQL cluster.

Importing this module does not read files or connect to PostgreSQL. All runtime
consumers call load_binding explicitly; every SQL operation still checks the
actual system identifier and data_directory, rather than trusting a manifest.
"""
import argparse
import json
import os
from pathlib import Path
import re
import stat
import subprocess


NAME = 'phase23-cluster-binding.json'
CONTRACT = 'unified-auth-phase23-owned-cluster/1'
FIELDS = frozenset(('contractVersion', 'taskDir', 'ownerMarker', 'taskDevice', 'taskInode',
    'ownerDevice', 'ownerInode', 'taskUid', 'taskGid', 'systemIdentifier', 'dataDirectory',
    'host', 'port', 'adminDatabase'))


def need(condition, code):
    if not condition:
        raise RuntimeError('PHASE23_CLUSTER_BINDING_' + code)


def identifier(value):
    need(type(value) is str and re.fullmatch(r'[1-9][0-9]{0,19}', value) is not None
         and int(value) <= 18446744073709551615, 'SYSTEM_IDENTIFIER_INVALID')
    return value


def protected_read(path, *, uid, mode, maximum):
    before = path.lstat()
    need(stat.S_ISREG(before.st_mode) and before.st_uid == uid and before.st_nlink == 1
         and stat.S_IMODE(before.st_mode) == mode and 0 < before.st_size <= maximum,
         'FILE_INVALID')
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        opened = os.fstat(fd)
        need((before.st_dev, before.st_ino) == (opened.st_dev, opened.st_ino), 'FILE_CHANGED')
        data = os.read(fd, maximum + 1)
        after = os.fstat(fd)
        need(len(data) == opened.st_size and (opened.st_dev, opened.st_ino, opened.st_size,
             opened.st_mtime_ns, opened.st_ctime_ns) == (after.st_dev, after.st_ino,
             after.st_size, after.st_mtime_ns, after.st_ctime_ns), 'FILE_CHANGED')
        return data, opened
    finally:
        os.close(fd)


def owned_root(value):
    need(os.name == 'posix' and hasattr(os, 'O_NOFOLLOW'), 'POSIX_REQUIRED')
    root = Path(value)
    need(root.is_absolute() and root.parent == Path('/tmp')
         and re.fullmatch(r'rust-saas-unified-auth-[A-Za-z0-9_-]+', root.name)
         and root.resolve(strict=True) == root and not root.is_symlink(), 'TASK_INVALID')
    details = root.lstat()
    need(stat.S_ISDIR(details.st_mode) and details.st_uid != 0
         and stat.S_IMODE(details.st_mode) == 0o700, 'TASK_OWNER_INVALID')
    owner, owner_stat = protected_read(root / 'OWNER', uid=details.st_uid, mode=0o600, maximum=256)
    need(owner == (root.name + '-pg55494\n').encode(), 'OWNER_MARKER_INVALID')
    data = root / 'data'
    info = data.lstat()
    need(stat.S_ISDIR(info.st_mode) and info.st_uid == details.st_uid
         and stat.S_IMODE(info.st_mode) == 0o700 and data.resolve(strict=True) == data
         and not data.is_symlink(), 'DATA_DIRECTORY_INVALID')
    return root, details, owner_stat, owner.decode().strip()


def validate_binding(value, root, details, owner_stat, marker):
    need(type(value) is dict and set(value) == FIELDS, 'SCHEMA_INVALID')
    identifier(value['systemIdentifier'])
    expected = {'contractVersion': CONTRACT, 'taskDir': str(root), 'ownerMarker': marker,
        'taskDevice': details.st_dev, 'taskInode': details.st_ino,
        'ownerDevice': owner_stat.st_dev, 'ownerInode': owner_stat.st_ino,
        'taskUid': details.st_uid, 'taskGid': details.st_gid,
        'dataDirectory': str(root / 'data'), 'host': '127.0.0.1', 'port': 55494,
        'adminDatabase': 'postgres'}
    need(all(type(value[key]) is type(item) and value[key] == item
             for key, item in expected.items()), 'OWNERSHIP_CHANGED')
    return value


def load_binding(task_dir):
    root, details, owner_stat, marker = owned_root(task_dir)
    raw, _ = protected_read(root / NAME, uid=0, mode=0o644, maximum=4096)
    try:
        value = json.loads(raw)
    except (ValueError, UnicodeDecodeError):
        raise RuntimeError('PHASE23_CLUSTER_BINDING_JSON_INVALID') from None
    return validate_binding(value, root, details, owner_stat, marker)


def sql_guard(binding):
    """An admin-only predicate; callers still guard database names and markers."""
    cluster = identifier(binding['systemIdentifier'])
    directory = binding['dataDirectory']
    need(type(directory) is str and re.fullmatch(r'/tmp/rust-saas-unified-auth-[A-Za-z0-9_-]+/data', directory),
         'DATA_DIRECTORY_INVALID')
    return ("(SELECT system_identifier::text FROM pg_control_system())='" + cluster
            + "' AND current_setting('data_directory')='" + directory + "'")


def initialize(task_dir):
    need(os.getuid() == 0, 'ROOT_INITIALIZER_REQUIRED')
    root, details, owner_stat, marker = owned_root(task_dir)
    environment = {'PATH': '/usr/lib/postgresql/18/bin:/usr/bin:/bin', 'PGHOST': '127.0.0.1',
        'PGPORT': '55494', 'PGUSER': 'postgres', 'PGDATABASE': 'postgres', 'PGSSLMODE': 'prefer',
        'PGAPPNAME': 'phase23-owned-cluster-binding', 'PGCONNECT_TIMEOUT': '5',
        'PGOPTIONS': '-c log_statement=none -c log_min_error_statement=panic'}
    query = ("SELECT json_build_object('systemIdentifier',system_identifier::text,"
        "'dataDirectory',current_setting('data_directory'),'database',current_database(),"
        "'user',current_user,'host',host(inet_server_addr()),'port',inet_server_port()) FROM pg_control_system()")
    result = subprocess.run(['/usr/lib/postgresql/18/bin/psql', '-X', '-qAt', '-v', 'ON_ERROR_STOP=1'],
        input=query.encode(), capture_output=True, env=environment, timeout=10)
    need(result.returncode == 0, 'DATABASE_QUERY_FAILED')
    try:
        actual = json.loads(result.stdout)
    except (ValueError, UnicodeDecodeError):
        raise RuntimeError('PHASE23_CLUSTER_BINDING_DATABASE_RESULT_INVALID') from None
    need(type(actual) is dict and actual.get('dataDirectory') == str(root / 'data')
         and actual.get('database') == 'postgres' and actual.get('user') == 'postgres'
         and actual.get('host') == '127.0.0.1' and actual.get('port') == 55494,
         'DATABASE_TARGET_INVALID')
    value = {'contractVersion': CONTRACT, 'taskDir': str(root), 'ownerMarker': marker,
        'taskDevice': details.st_dev, 'taskInode': details.st_ino,
        'ownerDevice': owner_stat.st_dev, 'ownerInode': owner_stat.st_ino,
        'taskUid': details.st_uid, 'taskGid': details.st_gid,
        'systemIdentifier': identifier(actual['systemIdentifier']), 'dataDirectory': str(root / 'data'),
        'host': '127.0.0.1', 'port': 55494, 'adminDatabase': 'postgres'}
    # Recheck the original directory and marker identities after querying PG.
    current = owned_root(root)
    validate_binding(value, *current)
    raw = (json.dumps(value, indent=2) + '\n').encode()
    fd = os.open(root / NAME, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o644)
    try:
        os.fchmod(fd, 0o644)
        os.fchown(fd, 0, 0)
        with os.fdopen(fd, 'wb', closefd=False) as output:
            output.write(raw)
            output.flush()
            os.fsync(fd)
    finally:
        os.close(fd)
    return load_binding(root)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('action', choices=('initialize', 'show'))
    parser.add_argument('--task-dir', type=Path, required=True)
    args = parser.parse_args()
    try:
        binding = initialize(args.task_dir) if args.action == 'initialize' else load_binding(args.task_dir)
        print(json.dumps({'status': 'PHASE23_OWNED_CLUSTER_BOUND', **binding}))
    except Exception as error:
        print(str(error) if isinstance(error, RuntimeError) else 'PHASE23_CLUSTER_BINDING_REJECTED')
        raise SystemExit(1) from None
