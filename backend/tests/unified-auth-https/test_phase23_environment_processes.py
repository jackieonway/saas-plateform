"""Owned-process regression tests; no DB, application, Docker or network.

Most cases are pure mocks. Three cases create temporary Python supervisor/child
processes in new sessions and always reap them in finally.
"""
import ast
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch


HERE = Path(__file__).resolve().parent
loader = importlib.util.spec_from_file_location('phase23_environment_processes', HERE / 'phase23-environment.py')
environment = importlib.util.module_from_spec(loader)
loader.loader.exec_module(environment)


class ProcessTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='phase23-env-process-pure-', dir='/tmp')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.root.chmod(0o700)
        task_patch = patch.object(environment, 'TASK', self.root)
        task_patch.start()
        self.addCleanup(task_patch.stop)
        self.task = environment.Task.__new__(environment.Task)
        self.task.uid, self.task.gid = os.getuid(), os.getgid()
        self.task.state_path = self.root / 'phase23-environment-state.json'
        self.task.state = {'processes': {'unrelated': {'pid': 876543}}}
        self.script = self.root / 'owned-supervisor.py'

    def mocked_spawn(self, *, mode='ready', launch_signal=None, cleanup_fails=False, wait_for_kill=False):
        process = Mock(pid=987654)
        process.poll.side_effect = AssertionError('leader must not be reaped before the group is signaled')
        process.wait.return_value = 0
        events = []
        state = {'term': False, 'killed': False}
        original_read, original_stat, original_link = Path.read_bytes, Path.stat, os.readlink
        def read_bytes(path):
            if str(path) == '/proc/987654/cmdline':
                if mode == 'proc-error':
                    raise OSError('private-proc-error')
                return str(self.script).encode() if mode != 'timeout' else b'still-in-wrapper'
            return original_read(path)
        def stat_path(path, *args, **kwargs):
            if str(path) == '/proc/987654':
                return SimpleNamespace(st_uid=self.task.uid)
            return original_stat(path, *args, **kwargs)
        def readlink(path):
            return 'mnt:[999999]' if str(path) == '/proc/987654/ns/mnt' else original_link(path)
        def killpg(pid, signum):
            self.assertEqual(pid, 987654)
            events.append(('signal', signum))
            if cleanup_fails and signum == signal.SIGTERM:
                raise PermissionError('mock-reap-denied')
            if signum == signal.SIGTERM:
                state['term'] = True
            if signum == signal.SIGKILL:
                state['killed'] = True
            if signum == 0:
                raise ProcessLookupError
        def waitid(*_args):
            if state['killed'] or (state['term'] and not wait_for_kill):
                return SimpleNamespace(si_pid=987654)
            return None
        def factory(*_args, **kwargs):
            self.assertTrue(kwargs['start_new_session'])
            if launch_signal:
                signal.getsignal(launch_signal)(launch_signal, None)
            return process
        process.wait.side_effect = lambda **_kwargs: events.append(('reaped', 987654)) or 0
        counter = iter(range(1000))
        stack = __import__('contextlib').ExitStack()
        stack.enter_context(patch.object(environment.subprocess, 'Popen', side_effect=factory))
        stack.enter_context(patch.object(environment.os, 'killpg', side_effect=killpg))
        stack.enter_context(patch.object(environment.os, 'waitid', side_effect=waitid))
        stack.enter_context(patch.object(environment.os, 'readlink', side_effect=readlink))
        stack.enter_context(patch.object(environment, 'start_time', return_value='12345'))
        stack.enter_context(patch.object(Path, 'read_bytes', read_bytes))
        stack.enter_context(patch.object(Path, 'stat', stat_path))
        stack.enter_context(patch.object(environment.time, 'monotonic', side_effect=lambda: next(counter)))
        stack.enter_context(patch.object(environment.time, 'sleep'))
        self.addCleanup(stack.close)
        self.spawn_patches = stack
        return process, events

    def invoke(self):
        self.task.spawn('namespace', ['owned-wrapper', str(self.root)], self.script, self.task.uid)

    def assert_reaped(self, process, events):
        process.wait.assert_called_once_with(timeout=3)
        self.assertIn(('signal', signal.SIGTERM), events)
        self.assertLess(events.index(('signal', signal.SIGKILL)), events.index(('reaped', 987654)))
        self.assertNotIn('namespace', self.task.state['processes'])
        self.assertEqual(self.task.state['processes']['unrelated'], {'pid': 876543})
        self.assertFalse(list(self.root.glob('phase23-spawn-*.json')))

    def test_success_commits_owned_pgid_without_termination_or_wait(self):
        process, events = self.mocked_spawn()
        self.invoke()
        self.assertEqual(events, [])
        process.wait.assert_not_called()
        self.assertEqual(self.task.state['processes']['namespace']['pgid'], 987654)
        self.assertEqual(self.task.json(self.task.state_path), self.task.state)
        self.assertFalse(list(self.root.glob('phase23-spawn-*.json')))

    def test_identity_timeout_reaps_unrecorded_wrapper(self):
        process, events = self.mocked_spawn(mode='timeout')
        with self.assertRaisesRegex(RuntimeError, 'PROCESS_RECORD_NOT_READY'):
            self.invoke()
        self.assert_reaped(process, events)

    def test_proc_read_exception_reaps_unrecorded_wrapper(self):
        process, events = self.mocked_spawn(mode='proc-error')
        with self.assertRaises(OSError):
            self.invoke()
        self.assert_reaped(process, events)

    def test_first_state_save_failure_rolls_back_process_and_partial_state(self):
        process, events = self.mocked_spawn()
        self.task.save = Mock(side_effect=[OSError('mock-state-write-denied'), None])
        with self.assertRaises(OSError):
            self.invoke()
        self.assert_reaped(process, events)
        self.assertEqual(self.task.save.call_count, 2)

    def test_launch_signal_before_popen_assignment_still_reaps_child(self):
        for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            with self.subTest(signal=signum):
                process, events = self.mocked_spawn(launch_signal=signum)
                with self.assertRaises(KeyboardInterrupt):
                    self.invoke()
                self.assert_reaped(process, events)
                # Close this iteration's patches before creating the next set.
                self.spawn_patches.close()

    def test_term_timeout_kills_before_reaping_reserved_leader(self):
        process, events = self.mocked_spawn(mode='timeout', wait_for_kill=True)
        with self.assertRaisesRegex(RuntimeError, 'PROCESS_RECORD_NOT_READY'):
            self.invoke()
        self.assert_reaped(process, events)

    def test_failed_reap_keeps_private_independent_ownership_lease(self):
        process, _events = self.mocked_spawn(mode='proc-error', cleanup_fails=True)
        with self.assertRaisesRegex(RuntimeError, 'SPAWN_CLEANUP_REQUIRED'):
            self.invoke()
        lease = self.root / 'phase23-spawn-namespace.json'
        value = self.task.json(lease)
        self.assertEqual((value['pid'], value['pgid'], value['startTime']), (987654, 987654, '12345'))
        self.assertTrue(value['cleanupRequired'])
        self.assertEqual(lease.stat().st_mode & 0o777, 0o600)
        process.wait.assert_not_called()
        self.task.stop_spawn_group = Mock(side_effect=RuntimeError('unavailable'))
        with self.assertRaises(RuntimeError):
            self.task.stop_pending_spawns()
        self.assertTrue(lease.exists())

    def test_launch_signal_and_failed_reap_also_leave_recoverable_lease(self):
        _process, _events = self.mocked_spawn(launch_signal=signal.SIGTERM, cleanup_fails=True)
        with self.assertRaisesRegex(RuntimeError, 'SPAWN_CLEANUP_REQUIRED'):
            self.invoke()
        value = self.task.json(self.root / 'phase23-spawn-namespace.json')
        self.assertEqual((value['pid'], value['startTime']), (987654, '12345'))

    def test_popen_failure_never_signals_other_groups(self):
        with (patch.object(environment.subprocess, 'Popen', side_effect=OSError('spawn-denied')),
              patch.object(environment.os, 'killpg') as kill):
            with self.assertRaises(OSError):
                self.invoke()
            kill.assert_not_called()
        self.assertEqual(self.task.state['processes'], {'unrelated': {'pid': 876543}})

    def test_pending_cleanup_removes_lease_only_after_success(self):
        value = {'contractVersion': 'phase23-spawn-ownership/1', 'key': 'namespace',
                 'pid': 987654, 'pgid': 987654, 'startTime': '12345', 'cleanupRequired': True}
        path = self.root / 'phase23-spawn-namespace.json'
        self.task.write(path, value)
        self.task.stop_spawn_group = Mock()
        self.task.stop_pending_spawns()
        self.task.stop_spawn_group.assert_called_once_with(value)
        self.assertFalse(path.exists())

    def test_failed_lease_write_falls_back_to_protected_state_ownership(self):
        _process, _events = self.mocked_spawn(mode='proc-error', cleanup_fails=True)
        original_write = self.task.write
        def write(path, value, **kwargs):
            if Path(path).name == 'phase23-spawn-namespace.json':
                raise OSError('mock-lease-path-unavailable')
            return original_write(path, value, **kwargs)
        self.task.write = write
        with self.assertRaisesRegex(RuntimeError, 'SPAWN_CLEANUP_REQUIRED'):
            self.invoke()
        value = self.task.json(self.task.state_path)['pendingSpawns']['namespace']
        self.assertEqual((value['pid'], value['startTime']), (987654, '12345'))
        self.assertFalse((self.root / 'phase23-spawn-namespace.json').exists())
        self.task.stop_spawn_group = Mock()
        self.task.stop_pending_spawns()
        self.task.stop_spawn_group.assert_called_once_with(value)
        self.assertEqual(self.task.json(self.task.state_path)['pendingSpawns'], {})

    def group_late_fork(self, *, during_kill=False, never_exits=False):
        leader, late = 987654, 987655
        live, descriptors, sent = {leader}, {}, []
        state = {'forked': False, 'termRequested': False, 'inventories': 0}
        original_iterdir, original_read, original_stat = Path.iterdir, Path.read_text, Path.stat
        original_bytes, original_close = Path.read_bytes, os.close
        value = {'pid': leader, 'pgid': leader, 'startTime': '12345', 'uid': self.task.uid,
                 'script': str(self.script), 'namespace': 'mnt:[999999]'}
        self.task.state['processes']['namespace'] = value
        def iterdir(path):
            if str(path) == '/proc':
                state['inventories'] += 1
                return iter(Path('/proc', str(pid)) for pid in sorted(live))
            return original_iterdir(path)
        def read_text(path, *args, **kwargs):
            if path.parent.parent == Path('/proc') and path.name == 'stat' and path.parent.name.isdigit():
                pid = int(path.parent.name)
                if pid not in live:
                    raise FileNotFoundError
                fields = ['S', '1', str(leader), str(leader)] + ['0'] * 15 + [str(12345 + pid - leader)]
                return str(pid) + ' (owned) ' + ' '.join(fields)
            return original_read(path, *args, **kwargs)
        def stat_path(path, *args, **kwargs):
            if path.parent == Path('/proc') and path.name.isdigit():
                return SimpleNamespace(st_uid=self.task.uid)
            return original_stat(path, *args, **kwargs)
        def read_bytes(path):
            if path.parent.parent == Path('/proc') and path.name == 'cmdline':
                return (str(self.root) + '/owned-child').encode()
            return original_bytes(path)
        def pidfd_open(pid):
            fd = pid + 10000
            descriptors[fd] = pid
            return fd
        def send(fd, signum):
            pid = descriptors[fd]
            sent.append((pid, signum))
            if never_exits:
                return
            if pid == leader and signum == signal.SIGTERM:
                state['termRequested'] = True
            if pid == late and signum in (signal.SIGTERM, signal.SIGKILL):
                live.discard(pid)
            if pid == leader and signum == signal.SIGKILL:
                # The last native child can become visible only after the
                # original group inventory, during KILL escalation itself.
                live.discard(leader)
                live.add(late)
                state['forked'] = True
        def select_fds(readers, *_args):
            if not during_kill and state['termRequested'] and not state['forked'] and not never_exits:
                # The directory snapshot has completed. Parent forks its
                # native child and exits before known pidfds are checked.
                live.discard(leader)
                live.add(late)
                state['forked'] = True
            return ([fd for fd in readers if descriptors[fd] not in live], [], [])
        counter = iter(range(1000))
        with (__import__('contextlib').ExitStack() as stack):
            stack.enter_context(patch.object(Path, 'iterdir', iterdir))
            stack.enter_context(patch.object(Path, 'read_text', read_text))
            stack.enter_context(patch.object(Path, 'stat', stat_path))
            stack.enter_context(patch.object(Path, 'read_bytes', read_bytes))
            stack.enter_context(patch.object(environment.os, 'readlink', return_value='mnt:[999999]'))
            stack.enter_context(patch.object(environment.os, 'pidfd_open', side_effect=pidfd_open))
            stack.enter_context(patch.object(environment.os, 'getpgid', return_value=leader))
            stack.enter_context(patch.object(environment.os, 'getsid', return_value=leader))
            stack.enter_context(patch.object(environment, 'start_time', side_effect=lambda pid: str(12345 + pid - leader)))
            stack.enter_context(patch.object(environment.signal, 'pidfd_send_signal', side_effect=send))
            stack.enter_context(patch.object(environment.select, 'select', side_effect=select_fds))
            stack.enter_context(patch.object(environment.time, 'monotonic', side_effect=lambda: next(counter)))
            stack.enter_context(patch.object(environment.os, 'close', side_effect=lambda fd: None if fd in descriptors else original_close(fd)))
            if never_exits:
                with self.assertRaisesRegex(RuntimeError, 'SPAWN_GROUP_REMAINS'):
                    self.task.stop_process('namespace')
                self.assertIn('namespace', self.task.state['processes'])
            else:
                self.task.stop_process('namespace')
                self.assertNotIn('namespace', self.task.state['processes'])
        return live, sent, state

    def test_group_final_inventory_collects_child_forked_after_prior_snapshot(self):
        live, sent, state = self.group_late_fork()
        self.assertFalse(live)
        self.assertIn((987655, signal.SIGTERM), sent)
        self.assertGreaterEqual(state['inventories'], 3)

    def test_group_kill_wait_collects_late_native_child(self):
        live, sent, _state = self.group_late_fork(during_kill=True)
        self.assertFalse(live)
        self.assertIn((987655, signal.SIGKILL), sent)

    def test_group_that_does_not_converge_keeps_state_and_fails_within_deadline(self):
        live, _sent, state = self.group_late_fork(never_exits=True)
        self.assertEqual(live, {987654})
        self.assertLess(state['inventories'], 20)

    def rp_launch(self, process, signum, *, repeated_signal=False):
        # Compile the actual launch/finally block, avoiding unrelated frozen
        # artifact setup while still executing the source under regression.
        tree = ast.parse((HERE / 'phase23-run-rp.py').read_text())
        main = next(node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name == 'main')
        index = next(i for i, node in enumerate(main.body) if isinstance(node, ast.Assign)
                     and any(isinstance(target, ast.Name) and target.id == 'record' for target in node.targets))
        names = ('task', 'node', 'source', 'child_environment', 'expected_namespace', 'namespace', 'manifest')
        function = ast.FunctionDef(name='launch', args=ast.arguments(posonlyargs=[], args=[ast.arg(arg=name) for name in names],
            kwonlyargs=[], kw_defaults=[], defaults=[]), body=main.body[index:], decorator_list=[])
        namespace = {'control': environment, 'TASK': self.root, 'os': os, 'signal': signal,
                     'subprocess': subprocess, 'time': time}
        exec(compile(ast.fix_missing_locations(ast.Module(body=[function], type_ignores=[])), '<actual-rp-launch>', 'exec'), namespace)
        original_signals = {sig: signal.getsignal(sig) for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)}
        def factory(*_args, **_kwargs):
            signal.getsignal(signum)(signum, None)
            return process
        if repeated_signal:
            process.terminate.side_effect = lambda: signal.getsignal(signum)(signum, None)
        try:
            with (patch.object(subprocess, 'Popen', side_effect=factory),
                  patch.object(environment, 'start_time', return_value='12345'), patch('builtins.print')):
                namespace['launch'](self.task, self.root / 'node', self.root / 'rp', {}, 'mnt:[999999]', {},
                                    {'snapshotSha256': 'a' * 64})
        finally:
            for sig, handler in original_signals.items():
                signal.signal(sig, handler)

    def test_rp_launch_signal_before_child_assignment_reaps_native_child(self):
        process = Mock(pid=987655)
        process.poll.return_value = None
        with self.assertRaises(KeyboardInterrupt):
            self.rp_launch(process, signal.SIGTERM)
        process.terminate.assert_called_once()
        process.wait.assert_called_once_with(timeout=5)
        self.assertFalse((self.root / 'phase23-rp.pid').exists())

    def test_rp_repeated_signal_during_finally_cannot_interrupt_reaping(self):
        process = Mock(pid=987655)
        process.poll.return_value = None
        with self.assertRaises(KeyboardInterrupt):
            self.rp_launch(process, signal.SIGINT, repeated_signal=True)
        process.wait.assert_called_once_with(timeout=5)
        self.assertFalse((self.root / 'phase23-rp.pid').exists())


@unittest.skipUnless(hasattr(os, 'pidfd_open') and hasattr(signal, 'pidfd_send_signal'), 'Linux pidfd required')
class TemporaryPythonGroupTests(unittest.TestCase):
    setUp = ProcessTests.setUp
    # These cases use only short-lived Python sleep children, not applications.
    def exercise_group(self, key, ignore_term=False):
        supervisor = self.root / 'temporary-python-supervisor.py'
        child_code = ('import signal,time,sys,pathlib; '
                      + ('signal.signal(signal.SIGTERM,signal.SIG_IGN); ' if ignore_term else '')
                      + 'pathlib.Path(sys.argv[1]).write_text("ready"); time.sleep(60)')
        supervisor.write_text('import subprocess,sys,time,json,pathlib\n'
            + 'child=subprocess.Popen([sys.executable,"-c",' + repr(child_code) + ',sys.argv[1]+"/ready"])\n'
            + 'pathlib.Path(sys.argv[1]+"/children.json").write_text(json.dumps({"pid":child.pid}))\n'
            + 'time.sleep(60)\n')
        process, peer = None, None
        child_pid, child_start = None, None
        try:
            with environment.spawn_signals(defer=True):
                process = subprocess.Popen([sys.executable, '-B', str(supervisor), str(self.root)],
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
                peer = subprocess.Popen([sys.executable, '-B', '-c', 'import time;time.sleep(60)'],
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
            deadline = time.monotonic() + 5
            while not (self.root / 'ready').exists():
                self.assertIsNone(process.poll())
                self.assertLess(time.monotonic(), deadline)
                time.sleep(.01)
            child_pid = json.loads((self.root / 'children.json').read_text())['pid']
            child_start = environment.start_time(child_pid)
            self.task.state['processes'][key] = {'pid': process.pid, 'pgid': process.pid,
                'startTime': environment.start_time(process.pid), 'script': str(supervisor), 'uid': os.getuid(),
                'namespace': os.readlink('/proc/' + str(process.pid) + '/ns/mnt')}
            self.assertFalse((self.root / ('service-' + key + '.pid')).exists())
            self.task.stop_process(key)
            process.wait(timeout=3)
            self.assertFalse(Path('/proc', str(child_pid), 'exe').exists())
            self.assertIsNone(peer.poll())
            self.assertNotIn(key, self.task.state['processes'])
        finally:
            cleanup_errors = []
            for owned in (process, peer):
                if owned is None:
                    continue
                try:
                    with environment.spawn_signals(defer=True):
                        if owned.poll() is None:
                            try:
                                os.killpg(owned.pid, signal.SIGKILL)
                            except ProcessLookupError:
                                pass
                        owned.wait(timeout=3)
                except BaseException as error:
                    cleanup_errors.append(error)
            try:
                if child_pid and Path('/proc', str(child_pid), 'exe').exists():
                    # An identified temporary child's PID is checked before this
                    # fallback; production services are never addressed here.
                    fd = os.pidfd_open(child_pid)
                    try:
                        if (environment.start_time(child_pid) == child_start
                                and str(self.root).encode() in Path('/proc', str(child_pid), 'cmdline').read_bytes()):
                            signal.pidfd_send_signal(fd, signal.SIGKILL)
                    finally:
                        os.close(fd)
            except BaseException as error:
                cleanup_errors.append(error)
            if cleanup_errors:
                raise RuntimeError('temporary Python process cleanup failed') from cleanup_errors[0]

    def test_temporary_python_service_group_without_native_record(self):
        self.exercise_group('service-identity')

    def test_temporary_python_rp_group_kills_stubborn_child_and_preserves_peer(self):
        self.exercise_group('rp', ignore_term=True)

    def test_temporary_python_spawn_save_failure_reaps_new_group(self):
        supervisor = self.root / 'temporary-python-launch-failure.py'
        child_code = 'import pathlib,sys,time;pathlib.Path(sys.argv[1]).write_text("ready");time.sleep(60)'
        supervisor.write_text('import subprocess,sys,time,json,pathlib\n'
            + 'child=subprocess.Popen([sys.executable,"-c",' + repr(child_code) + ',sys.argv[1]+"/ready"])\n'
            + 'pathlib.Path(sys.argv[1]+"/children.json").write_text(json.dumps({"pid":child.pid}))\n'
            + 'time.sleep(60)\n')
        original_factory, original_save = subprocess.Popen, self.task.save
        spawned = []
        child_pid, child_start = None, None
        def factory(*args, **kwargs):
            process = original_factory(*args, **kwargs)
            spawned.append(process)
            return process
        calls = 0
        def save():
            nonlocal calls, child_pid, child_start
            calls += 1
            if calls == 1:
                deadline = time.monotonic() + 5
                while not all((self.root / name).exists() for name in ('ready', 'children.json')):
                    self.assertLess(time.monotonic(), deadline)
                    time.sleep(.01)
                child_pid = json.loads((self.root / 'children.json').read_text())['pid']
                child_start = environment.start_time(child_pid)
                raise OSError('injected-first-save-failure')
            original_save()
        self.task.save = save
        try:
            with patch.object(environment.subprocess, 'Popen', side_effect=factory):
                with self.assertRaisesRegex(OSError, 'injected-first-save-failure'):
                    self.task.spawn('namespace', [sys.executable, '-B', str(supervisor), str(self.root)],
                                    supervisor, os.getuid())
            self.assertEqual(len(spawned), 1)
            self.assertIsNotNone(spawned[0].returncode)
            child = json.loads((self.root / 'children.json').read_text())['pid']
            self.assertFalse(Path('/proc', str(child), 'exe').exists())
            self.assertFalse(list(self.root.glob('phase23-spawn-*.json')))
        finally:
            cleanup_errors = []
            for process in spawned:
                try:
                    with environment.spawn_signals(defer=True):
                        if process.returncode is None:
                            try:
                                os.killpg(process.pid, signal.SIGKILL)
                            except ProcessLookupError:
                                pass
                        process.wait(timeout=3)
                except BaseException as error:
                    cleanup_errors.append(error)
            try:
                if child_pid and Path('/proc', str(child_pid), 'exe').exists():
                    fd = os.pidfd_open(child_pid)
                    try:
                        if (environment.start_time(child_pid) == child_start
                                and str(self.root).encode() in Path('/proc', str(child_pid), 'cmdline').read_bytes()):
                            signal.pidfd_send_signal(fd, signal.SIGKILL)
                    finally:
                        os.close(fd)
            except BaseException as error:
                cleanup_errors.append(error)
            if cleanup_errors:
                raise RuntimeError('temporary Python launch-failure cleanup failed') from cleanup_errors[0]


if __name__ == '__main__':
    unittest.main()
