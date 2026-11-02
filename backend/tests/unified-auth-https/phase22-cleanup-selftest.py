"""Replay four Phase 22 finally cases entirely against an in-memory filesystem.

Reads the runner source and extracts only main()'s finally AST. No main(),
subprocess, Docker, database, listener, real signal or filesystem write executes.
Run after the live capacity window: python3 phase22-cleanup-selftest.py
"""
import argparse
import ast
import fnmatch
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import subprocess
import types


class MemoryFilesystem:
    def __init__(self):
        self.nodes = {}

    def path(self, value):
        return MemoryPath(self, str(value))

    def directory(self, value):
        self.nodes[str(value)] = ('directory', b'')
        return self.path(value)

    def file(self, value, content):
        self.nodes[str(value)] = ('file', content.encode() if isinstance(content, str) else content)
        return self.path(value)


class MemoryPath:
    def __init__(self, filesystem, value):
        self.fs = filesystem
        self.value = str(PurePosixPath(value))

    def __str__(self):
        return self.value

    def __eq__(self, other):
        return isinstance(other, MemoryPath) and self.fs is other.fs and self.value == other.value

    def __hash__(self):
        return hash((id(self.fs), self.value))

    def __truediv__(self, name):
        return self.fs.path(PurePosixPath(self.value) / str(name))

    @property
    def name(self):
        return PurePosixPath(self.value).name

    @property
    def suffix(self):
        return PurePosixPath(self.value).suffix

    @property
    def parent(self):
        return self.fs.path(PurePosixPath(self.value).parent)

    def resolve(self, **_options):
        return self

    def exists(self):
        return self.value in self.fs.nodes

    def is_symlink(self):
        return self.exists() and self.fs.nodes[self.value][0] == 'symlink'

    def is_file(self):
        return self.exists() and self.fs.nodes[self.value][0] == 'file'

    def is_dir(self):
        return self.exists() and self.fs.nodes[self.value][0] == 'directory'

    def stat(self):
        if not self.exists():
            raise FileNotFoundError('MOCK_ABSENT')
        return types.SimpleNamespace(st_uid=1000, st_size=len(self.fs.nodes[self.value][1]))

    def read_bytes(self):
        if not self.is_file():
            raise FileNotFoundError('MOCK_NOT_FILE')
        return self.fs.nodes[self.value][1]

    def read_text(self):
        return self.read_bytes().decode()

    def write_text(self, content):
        self.fs.file(self.value, content)
        return len(content)

    def unlink(self):
        if not self.exists():
            raise FileNotFoundError('MOCK_ABSENT')
        del self.fs.nodes[self.value]

    def iterdir(self):
        return iter([self.fs.path(value) for value in self.fs.nodes if str(PurePosixPath(value).parent) == self.value])

    def glob(self, pattern):
        return iter([target for target in self.iterdir() if fnmatch.fnmatch(target.name, pattern)])


def runner_finally(source):
    module = ast.parse(source)
    main = next(node for node in module.body if isinstance(node, ast.FunctionDef) and node.name == 'main')
    body = next(node for node in main.body if isinstance(node, ast.Try)).finalbody
    tree = ast.Module(body=body, type_ignores=[])
    # The replay contains no imports and receives only our process/OS mocks.
    if any(isinstance(node, (ast.Import, ast.ImportFrom)) for node in ast.walk(tree)):
        raise RuntimeError('SELFTEST_FINALLY_IMPORT_REJECTED')
    return compile(tree, '<phase22-cleanup-memory-mock>', 'exec'), hashlib.sha256(
        ast.dump(tree, include_attributes=False).encode()).hexdigest()


def run_case(code, scenario, phase=22):
    filesystem = MemoryFilesystem()
    root = filesystem.directory('/owned/phase22-cleanup-memory-mock')
    tools = filesystem.directory(root / 'phase22-tools')
    filesystem.file(tools / 'public.py', 'pass')
    source = filesystem.directory(root / 'source')
    validation = filesystem.directory(root / 'reports')
    cluster = '18446744073709551614'  # Synthetic identity; no real cluster is used.
    report_prefix = 'phase-22' if phase == 22 else 'phase-25-capacity'
    runtime_name = 'phase22-runtime.json' if phase == 22 else 'phase25-capacity-runtime.json'
    manifest = {'database': 'unified_auth_https_' + 'a' * 32,
                'rolePrefix': 'uah_' + 'a' * 12, 'clusterId': cluster}
    filesystem.file(root / 'https-database.json', json.dumps(manifest))
    filesystem.file(root / 'phase22-secret.json', json.dumps({'password': 'MEMORY_ONLY_SELFTEST_PRIVATE_CANARY'}))
    if scenario == 'control-failure':
        filesystem.file(root / 'namespace.pid', '{}')
    if scenario == 'malformed-pid':
        filesystem.file(root / 'service-identity.pid', 'invalid')
    calls = []

    def control(*arguments):
        calls.append(('control', arguments))
        if scenario == 'control-failure':
            raise subprocess.TimeoutExpired('MOCK_BOUNDED_CONTROL', 1)
        return types.SimpleNamespace(returncode=0)

    def user(command, **_options):
        assert command[:2] == ['python3', '-B'] and command[2] == source / 'prepare.py'
        calls.append(('cleanup', True))
        if scenario == 'database-failure':
            raise RuntimeError('MOCK_DATABASE_CLEANUP_FAILED')
        (root / 'https-database.json').unlink()

    def output(command, **_options):
        if command[0] == 'docker':
            calls.append(('df', True))
            return 'MOCK_DOCKER_DF_NO_ENGINE_CALLED'
        if command[0] == 'df':
            calls.append(('filesystem-df', True))
            return 'MOCK_FILESYSTEM_DF_NO_COMMAND_EXECUTED'
        assert command[0] == 'psql'
        if 'pg_database_size' in command[-1]:
            return '1'
        return cluster + ('|1|6' if scenario == 'database-failure' else '|0|0')

    def forbidden(*_arguments, **_options):
        raise AssertionError('UNEXPECTED_OS_OR_PROCESS_OPERATION')

    namespace = dict(TASK=root, VALIDATION=validation, ADMIN='MOCK_ADMIN_TARGET', source=source,
        tools=tools, owner=types.SimpleNamespace(st_uid=1000), before_files=set(), report={'result': 'PASS'},
        cluster=cluster, owned_cluster={'systemIdentifier': cluster, 'dataDirectory': str(root / 'data')},
        cluster_binding=types.SimpleNamespace(sql_guard=lambda _value: 'TRUE'),
        runtime_name=runtime_name, report_prefix=report_prefix,
        processes=[], control=control, user=user,
        subprocess=types.SimpleNamespace(check_output=output, DEVNULL=None, TimeoutExpired=subprocess.TimeoutExpired),
        Path=filesystem.path, os=types.SimpleNamespace(kill=forbidden, readlink=forbidden),
        json=json, re=re, time=types.SimpleNamespace(monotonic=lambda: 0, sleep=forbidden),
        hashlib=hashlib, print=lambda *_arguments: None)
    exited = None
    try:
        exec(code, namespace)
    except SystemExit as failure:
        exited = failure.code
    result = json.loads((validation / (report_prefix + '-cleanup.json')).read_text())
    assert not (validation / ('phase-22-cleanup.json' if phase == 25 else 'phase-25-capacity-cleanup.json')).exists()
    expected = 'PASS' if scenario == 'success' else 'FAILED'
    assert result['result'] == expected
    assert exited == (None if scenario == 'success' else 1)
    assert any(name == 'df' for name, _details in calls)
    assert any(name == 'cleanup' for name, _details in calls)
    assert not (root / 'phase22-secret.json').exists()
    assert result['logRedaction']['leaks'] == 0
    if scenario == 'success':
        assert result['ownedDatabaseAndRolesRemoved'] is True
        assert result['remainingLeaseRecords'] == 0 and result['cleanupErrors'] == []
    else:
        assert result['remainingLeaseRecords'] == 1 and result['cleanupErrors']
        if scenario == 'database-failure':
            assert (root / 'https-database.json').exists()
            assert result['ownedDatabaseAndRolesRemoved'] is False
        else:
            assert result['ownedDatabaseAndRolesRemoved'] is True
    return {'scenario': scenario, 'result': 'PASS', 'runnerExpectedResult': expected,
            'independentCleanupContinued': True, 'residualOwnershipPreserved': scenario != 'success'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runner', type=Path, default=Path(__file__).with_name('phase22-run.py'))
    args = parser.parse_args()
    result = {'contractVersion': 'identity-phase22-cleanup-selftest/1', 'result': 'FAILED',
              'realProcessesStarted': 0, 'realSignalsSent': 0, 'databaseConnections': 0,
              'dockerCommandsExecuted': 0, 'realFilesystemMutations': 0, 'cases': []}
    try:
        data = args.runner.read_bytes()
        code, ast_sha = runner_finally(data.decode())
        result['source'] = {'file': 'phase22-run.py', 'sha256': hashlib.sha256(data).hexdigest(),
                            'finallyAstSha256': ast_sha}
        for scenario in ('success', 'control-failure', 'database-failure', 'malformed-pid'):
            result['cases'].append(run_case(code, scenario))
        result['result'] = 'PASS'
    except BaseException:
        result['failure'] = 'MEMORY_MOCK_SELFTEST_FAILED'
    print(json.dumps(result, sort_keys=True))
    return 0 if result['result'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
