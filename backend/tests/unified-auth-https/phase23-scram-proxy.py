"""Task-only bounded byte forwarder. It never parses PostgreSQL or credentials.

Production: python3 phase23-scram-proxy.py --config /owned/config.json
Local echo tests only: python3 phase23-scram-proxy.py --self-test
"""
import argparse
import asyncio
import contextlib
from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
import signal
import socket
import stat
import sys


TASK = Path('/tmp/rust-saas-unified-auth-20261028')
PID_FILE = 'phase23-scram-proxy.pid'
FIXED = {'contractVersion': 'identity-scram-proxy-input/1', 'taskDir': str(TASK),
         'listenPort': 25495, 'maximumConnections': 8, 'backlog': 8,
         'maximumBytesPerConnection': 1048576, 'totalTimeoutMs': 10000,
         'idleTimeoutMs': 3000, 'chunkBytes': 65536}


def reject():
    raise RuntimeError('PHASE23_SCRAM_PROXY_FAILED')


def wsl_linux():
    return sys.platform == 'linux' and 'microsoft' in os.uname().release.lower()


def validate_config(value):
    if not isinstance(value, dict) or set(value) != set(FIXED):
        reject()
    if any(type(value[key]) is not type(expected) or value[key] != expected for key, expected in FIXED.items()):
        reject()
    return value


def process_start(pid):
    return Path('/proc', str(pid), 'stat').read_text().rsplit(')', 1)[1].split()[19]


def process_facts(pid):
    return process_start(pid), Path('/proc', str(pid), 'cmdline').read_bytes()[:16384]


class TaskLease:
    """Anchored task files plus unchanged common mount-namespace PID lease.

    A non-root process can read the root controller's stat/cmdline but may not
    read /proc/<root>/ns/mnt; compare the current namespace to the sealed lease.
    """
    def __init__(self, task, facts=process_facts):
        self.task, self.facts, self.fd = Path(task), facts, None
        try:
            if (self.task.parent != Path('/tmp') or self.task.is_symlink()
                    or self.task.resolve(strict=True) != self.task):
                reject()
            self.fd = os.open(self.task, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
            info = os.fstat(self.fd)
            if info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o700:
                reject()
            self.owner_bytes = self.read('OWNER', 256, private=False)
            if self.owner_bytes.decode().strip() != self.task.name + '-pg55494':
                reject()
            self.lease_bytes = self.read('namespace.pid', 4096, private=False)
            self.lease = json.loads(self.lease_bytes)
            if (set(self.lease) != {'pid', 'startTime', 'namespace'} or type(self.lease['pid']) is not int
                    or self.lease['pid'] < 2 or not re.fullmatch(r'[0-9]+', self.lease['startTime'])
                    or not re.fullmatch(r'mnt:\[[0-9]+\]', self.lease['namespace'])):
                reject()
            self.identity_bytes = self.read('namespace.identity', 256, private=False)
            if not self.valid():
                reject()
        except BaseException:
            self.close()
            reject()

    def read(self, name, maximum, private=True):
        if Path(name).name != name or name in ('.', '..'):
            reject()
        descriptor = os.open(name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=self.fd)
        try:
            info = os.fstat(descriptor)
            if (not stat.S_ISREG(info.st_mode) or info.st_size < 1 or info.st_size > maximum
                    or info.st_uid not in (0, os.getuid()) or info.st_mode & 0o022
                    or (private and (info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o600))):
                reject()
            value = b''
            while len(value) <= maximum:
                chunk = os.read(descriptor, min(65536, maximum + 1 - len(value)))
                if not chunk:
                    break
                value += chunk
            if len(value) != info.st_size:
                reject()
            return value
        finally:
            os.close(descriptor)

    def valid(self):
        try:
            if (self.read('OWNER', 256, private=False) != self.owner_bytes
                    or self.read('namespace.pid', 4096, private=False) != self.lease_bytes
                    or self.read('namespace.identity', 256, private=False) != self.identity_bytes
                    or self.identity_bytes.decode().strip() != self.lease['namespace']
                    or os.readlink('/proc/self/ns/mnt') != self.lease['namespace']):
                return False
            started, command = self.facts(self.lease['pid'])
            return (started == self.lease['startTime']
                    and str(self.task / 'environment-controller.sh').encode() in command)
        except BaseException:
            return False

    def create_record(self):
        raw = json.dumps({'contractVersion': 'identity-scram-proxy-process/1', 'pid': os.getpid(),
            'startTime': process_start(os.getpid()), 'namespace': self.lease['namespace'],
            'leasePid': self.lease['pid'], 'leaseStartTime': self.lease['startTime'],
            'script': str(Path(__file__).resolve()), 'listenPort': 25495}, sort_keys=True).encode()
        descriptor = os.open(PID_FILE, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=self.fd)
        try:
            os.fchmod(descriptor, 0o600)
            with os.fdopen(descriptor, 'wb', closefd=False) as stream:
                stream.write(raw)
                stream.flush()
                os.fsync(descriptor)
            os.fsync(self.fd)
        except BaseException:
            created = os.fstat(descriptor)
            current = os.stat(PID_FILE, dir_fd=self.fd, follow_symlinks=False)
            if (created.st_dev, created.st_ino) == (current.st_dev, current.st_ino):
                os.unlink(PID_FILE, dir_fd=self.fd)
                os.fsync(self.fd)
            raise
        finally:
            os.close(descriptor)
        return raw

    def remove_record(self, original):
        try:
            current = self.read(PID_FILE, 4096)
        except FileNotFoundError:
            return
        if current != original:
            reject()
        os.unlink(PID_FILE, dir_fd=self.fd)
        os.fsync(self.fd)

    def close(self):
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None


@dataclass(frozen=True)
class Limits:
    connections: int = 8
    backlog: int = 8
    byte_limit: int = 1048576
    total: float = 10
    idle: float = 3
    chunk: int = 65536


class BoundedProxy:
    """Internal test constructor permits an ephemeral echo target, never CLI.

    Bytes are transient read/write chunks only. Neither exceptions, addresses,
    bytes, per-connection metadata nor protocol material are logged.
    """
    def __init__(self, port=25495, target_port=55494, limits=Limits(), lease=lambda: True):
        self.port, self.target_port, self.limits, self.lease = port, target_port, limits, lease
        self.server = None
        self.tasks, self.writers = set(), set()
        self.closing = False
        self.counts = dict(accepted=0, capacity=0, idle=0, total=0, byte_limit=0, unavailable=0, peak=0)

    async def start(self):
        if not self.lease():
            reject()
        self.server = await asyncio.start_server(self.accept, '127.0.0.1', self.port,
            family=socket.AF_INET, backlog=self.limits.backlog, limit=self.limits.chunk // 2,
            reuse_address=False, start_serving=True)
        self.port = self.server.sockets[0].getsockname()[1]

    def accept(self, reader, writer):
        if self.closing or not self.lease() or len(self.tasks) >= self.limits.connections:
            self.counts['capacity'] += 1
            writer.close()
            writer.transport.abort()
            return
        self.writers.add(writer)
        writer.transport.set_write_buffer_limits(high=self.limits.chunk, low=self.limits.chunk // 2)
        task = asyncio.create_task(self.forward(reader, writer))
        self.tasks.add(task)
        self.counts['accepted'] += 1
        self.counts['peak'] = max(self.counts['peak'], len(self.tasks))
        def done(completed):
            self.tasks.discard(completed)
            with contextlib.suppress(BaseException):
                completed.exception()
        task.add_done_callback(done)

    async def forward(self, client_reader, client_writer):
        upstream_writer, jobs = None, []
        loop = asyncio.get_running_loop()
        state = {'bytes': 0, 'activity': loop.time()}
        async def pipe(reader, writer):
            while True:
                data = await reader.read(self.limits.chunk)
                if not data:
                    if writer.can_write_eof():
                        writer.write_eof()
                        await writer.drain()
                    return 'EOF'
                if state['bytes'] + len(data) > self.limits.byte_limit:
                    return 'BYTE_LIMIT'
                state['bytes'] += len(data)
                state['activity'] = loop.time()
                writer.write(data)
                await writer.drain()
                state['activity'] = loop.time()
                if state['bytes'] == self.limits.byte_limit:
                    return 'BYTE_LIMIT'
        async def idle():
            while True:
                remaining = state['activity'] + self.limits.idle - loop.time()
                if remaining <= 0:
                    return 'IDLE'
                await asyncio.sleep(remaining)
        try:
            async with asyncio.timeout(self.limits.total):
                try:
                    upstream_reader, upstream_writer = await asyncio.wait_for(asyncio.open_connection(
                        '127.0.0.1', self.target_port, family=socket.AF_INET,
                        local_addr=('127.0.0.2', 0), limit=self.limits.chunk // 2), self.limits.idle)
                except TimeoutError:
                    self.counts['unavailable'] += 1
                    return
                self.writers.add(upstream_writer)
                upstream_writer.transport.set_write_buffer_limits(high=self.limits.chunk, low=self.limits.chunk // 2)
                jobs = [asyncio.create_task(pipe(client_reader, upstream_writer)),
                        asyncio.create_task(pipe(upstream_reader, client_writer)), asyncio.create_task(idle())]
                pending = set(jobs)
                while pending:
                    completed, pending = await asyncio.wait(pending, return_when=asyncio.FIRST_COMPLETED)
                    for job in completed:
                        outcome = job.result()
                        if outcome == 'IDLE':
                            self.counts['idle'] += 1
                            return
                        if outcome == 'BYTE_LIMIT':
                            self.counts['byte_limit'] += 1
                            return
                    # Both byte directions ended cleanly; the watchdog has no work.
                    if all(job.done() for job in jobs[:2]):
                        return
        except TimeoutError:
            self.counts['total'] += 1
        except asyncio.CancelledError:
            pass
        except Exception:
            self.counts['unavailable'] += 1
        finally:
            for job in jobs:
                job.cancel()
            await asyncio.gather(*jobs, return_exceptions=True)
            for writer in (client_writer, upstream_writer):
                if writer is not None:
                    writer.close()
                    writer.transport.abort()
                    self.writers.discard(writer)
            # abort closes the sockets without waiting for a peer or a drain.
            await asyncio.sleep(0)

    async def close(self):
        if self.closing:
            return
        self.closing = True
        if self.server is not None:
            self.server.close()
        for writer in tuple(self.writers):
            writer.close()
            writer.transport.abort()
        for task in tuple(self.tasks):
            task.cancel()
        await asyncio.gather(*tuple(self.tasks), return_exceptions=True)
        if self.server is not None:
            await self.server.wait_closed()
        self.tasks.clear()
        self.writers.clear()


async def watch_lease(lease, stopped, interval=0.1):
    while lease():
        try:
            await asyncio.wait_for(stopped.wait(), interval)
            return
        except TimeoutError:
            pass
    stopped.set()


async def run_config(filename):
    if not wsl_linux() or os.getuid() == 0:
        reject()
    path = Path(filename)
    if not path.is_absolute() or path.parent != TASK or path.name in ('.', '..'):
        reject()
    guard, proxy, original = TaskLease(TASK), None, None
    stopped = asyncio.Event()
    loop = asyncio.get_running_loop()
    monitor = None
    try:
        validate_config(json.loads(guard.read(path.name, 4096)))
        try:
            guard.read(PID_FILE, 4096)
            reject()
        except FileNotFoundError:
            pass
        proxy = BoundedProxy(lease=guard.valid)
        await proxy.start()
        original = guard.create_record()
        for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            loop.add_signal_handler(signum, stopped.set)
        monitor = asyncio.create_task(watch_lease(guard.valid, stopped))
        print('PHASE23_SCRAM_PROXY_READY', flush=True)
        await stopped.wait()
    finally:
        if monitor is not None:
            monitor.cancel()
            await asyncio.gather(monitor, return_exceptions=True)
        try:
            if proxy is not None:
                await proxy.close()
        finally:
            try:
                if original is not None:
                    guard.remove_record(original)
            finally:
                guard.close()
                for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
                    loop.remove_signal_handler(signum)
    print('PHASE23_SCRAM_PROXY_STOPPED', flush=True)


async def self_tests():
    if not wsl_linux():
        reject()
    import tempfile
    cases = []
    async def until(predicate):
        async with asyncio.timeout(1):
            while not predicate():
                await asyncio.sleep(.001)
    class Echo:
        def __init__(self):
            self.tasks, self.writers, self.sources = set(), set(), []
        async def start(self):
            def accept(reader, writer):
                self.sources.append(writer.get_extra_info('peername')[0] == '127.0.0.2')
                self.writers.add(writer)
                task = asyncio.create_task(self.echo(reader, writer))
                self.tasks.add(task)
                task.add_done_callback(self.tasks.discard)
            self.server = await asyncio.start_server(accept, '127.0.0.1', 0)
            self.port = self.server.sockets[0].getsockname()[1]
        async def echo(self, reader, writer):
            try:
                while data := await reader.read(65536):
                    writer.write(data)
                    await writer.drain()
            except (OSError, asyncio.CancelledError):
                pass
            finally:
                writer.close()
                writer.transport.abort()
                self.writers.discard(writer)
        async def close(self):
            self.server.close()
            for writer in tuple(self.writers):
                writer.close()
                writer.transport.abort()
            for task in tuple(self.tasks):
                task.cancel()
            await asyncio.gather(*tuple(self.tasks), return_exceptions=True)
            await self.server.wait_closed()
    async def setup(**options):
        echo = Echo()
        await echo.start()
        proxy = BoundedProxy(port=0, target_port=echo.port, **options)
        try:
            await proxy.start()
        except BaseException:
            await echo.close()
            raise
        return echo, proxy
    async def finish(echo, proxy, clients=()):
        for writer in clients:
            writer.close()
            writer.transport.abort()
        await proxy.close()
        await echo.close()
        assert not proxy.tasks and not proxy.writers and not proxy.server.is_serving()
    # No actual PostgreSQL port, task lease or production listener is touched.
    echo, proxy = await setup()
    writers = []
    try:
        reader, writer = await asyncio.open_connection('127.0.0.1', proxy.port)
        writers.append(writer)
        writer.write(b'KNOWN_ECHO_CANARY')
        await writer.drain()
        assert await asyncio.wait_for(reader.readexactly(17), 1) == b'KNOWN_ECHO_CANARY'
        writer.write_eof()
        assert await asyncio.wait_for(reader.read(), 1) == b''
        assert echo.sources == [True]
        cases.append('transparent_echo_source_bind_and_half_close')
    finally:
        await finish(echo, proxy, writers)
    echo, proxy = await setup()
    writers, readers = [], []
    try:
        for _ in range(8):
            reader, writer = await asyncio.open_connection('127.0.0.1', proxy.port)
            readers.append(reader)
            writers.append(writer)
        await until(lambda: len(proxy.tasks) == 8 and len(echo.sources) == 8)
        reader, writer = await asyncio.open_connection('127.0.0.1', proxy.port)
        writers.append(writer)
        with contextlib.suppress(ConnectionResetError):
            assert await asyncio.wait_for(reader.read(), 1) == b''
        await until(lambda: proxy.counts['capacity'] == 1)
        assert proxy.counts['peak'] == 8 and len(echo.sources) == 8
        # Close the proxy while all eight clients are still open, not after
        # client teardown has already removed its connections.
        await asyncio.wait_for(proxy.close(), 1)
        for reader in readers:
            assert await asyncio.wait_for(reader.read(), 1) == b''
        cases.append('eight_connection_hard_capacity_and_close_all')
    finally:
        await finish(echo, proxy, writers)
    for condition, limits in [('idle', Limits(total=.5, idle=.04)), ('byte_limit', Limits(byte_limit=32))]:
        echo, proxy = await setup(limits=limits)
        writers = []
        try:
            reader, writer = await asyncio.open_connection('127.0.0.1', proxy.port)
            writers.append(writer)
            if condition == 'byte_limit':
                writer.write(b'x' * 33)
                await writer.drain()
            with contextlib.suppress(ConnectionResetError):
                await asyncio.wait_for(reader.read(), 1)
            await until(lambda: proxy.counts[condition] == 1)
            cases.append(condition + '_hard_limit')
        finally:
            await finish(echo, proxy, writers)
    echo, proxy = await setup(limits=Limits(total=.15, idle=.05))
    writers = []
    try:
        reader, writer = await asyncio.open_connection('127.0.0.1', proxy.port)
        writers.append(writer)
        async with asyncio.timeout(1):
            while True:
                with contextlib.suppress(ConnectionResetError, BrokenPipeError):
                    writer.write(b'x')
                    await writer.drain()
                if not await reader.read(1):
                    break
                await asyncio.sleep(.01)
        assert proxy.counts['total'] == 1
        cases.append('total_deadline_despite_continuous_activity')
    finally:
        await finish(echo, proxy, writers)
    with tempfile.TemporaryDirectory(prefix='phase23-proxy-selftest-') as directory:
        task = Path(directory)
        task.chmod(0o700)
        def write(name, value, mode=0o600):
            target = task / name
            target.write_text(value)
            target.chmod(mode)
        namespace = os.readlink('/proc/self/ns/mnt')
        write('OWNER', task.name + '-pg55494')
        write('namespace.pid', json.dumps({'pid': os.getpid(), 'startTime': process_start(os.getpid()), 'namespace': namespace}), 0o644)
        write('namespace.identity', namespace, 0o644)
        actual_start = process_start(os.getpid())
        process_state = {'start': actual_start}
        facts = lambda _pid: (process_state['start'], str(task / 'environment-controller.sh').encode())
        guard = TaskLease(task, facts)
        try:
            process_state['start'] = str(int(actual_start) + 1)
            assert not guard.valid()
            process_state['start'] = actual_start
            assert guard.valid()
            original = guard.create_record()
            assert stat.S_IMODE((task / PID_FILE).stat().st_mode) == 0o600
            write(PID_FILE, 'DIFFERENT_RECORD')
            try:
                guard.remove_record(original)
                raise AssertionError('REPLACED_PID_RECORD_NOT_REJECTED')
            except RuntimeError:
                assert (task / PID_FILE).read_text() == 'DIFFERENT_RECORD'
            write(PID_FILE, original.decode())
            guard.remove_record(original)
            write('config.json', json.dumps(FIXED))
            validate_config(json.loads(guard.read('config.json', 4096)))
            write('config.json', json.dumps(FIXED), 0o644)
            try:
                guard.read('config.json', 4096)
                raise AssertionError('PRIVATE_MODE_NOT_REJECTED')
            except RuntimeError:
                pass
            (task / 'link.json').symlink_to(task / 'config.json')
            try:
                guard.read('link.json', 4096)
                raise AssertionError('SYMLINK_NOT_REJECTED')
            except OSError:
                pass
            echo, proxy = await setup(lease=guard.valid)
            writers = []
            stopped = asyncio.Event()
            watcher = asyncio.create_task(watch_lease(guard.valid, stopped, .005))
            try:
                reader, writer = await asyncio.open_connection('127.0.0.1', proxy.port)
                writers.append(writer)
                await until(lambda: len(proxy.tasks) == 1)
                (task / 'namespace.pid').unlink()
                assert not guard.valid()
                await asyncio.wait_for(stopped.wait(), .5)
                await proxy.close()
                assert await asyncio.wait_for(reader.read(), .5) == b''
            finally:
                watcher.cancel()
                await asyncio.gather(watcher, return_exceptions=True)
                await finish(echo, proxy, writers)
            cases.append('private_files_pid_record_and_lease_loss')
        finally:
            guard.close()
    for modified in (dict(FIXED, targetPort=1), dict(FIXED, listenPort=0), dict(FIXED, maximumConnections=9)):
        try:
            validate_config(modified)
            raise AssertionError('NONFIXED_CONFIG_NOT_REJECTED')
        except RuntimeError:
            pass
    cases.append('fixed_config_no_arbitrary_forwarding')
    return {'contractVersion': 'identity-scram-proxy-selftests/1', 'result': 'PASS',
            'passed': len(cases), 'cases': cases, 'realPostgresConnections': 0,
            'hbaChanged': False, 'temporaryListenersConnectionsAndFilesCleaned': True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--config')
    mode.add_argument('--self-test', action='store_true')
    args = parser.parse_args()
    try:
        if args.self_test:
            print(json.dumps(asyncio.run(self_tests()), sort_keys=True))
        else:
            asyncio.run(run_config(args.config))
        return 0
    except BaseException:
        # Never print a traceback, exception message, address or stream content.
        print('PHASE23_SCRAM_PROXY_FAILED', file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
