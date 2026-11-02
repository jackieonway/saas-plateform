"""Pure Linux log ownership regression tests; no DB, sockets or services.

Run: python3 -B -m unittest discover -s <this-directory> -p test_phase23_environment_logs.py -v
"""
import importlib.util
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import Mock, patch


loader = importlib.util.spec_from_file_location(
    'phase23_environment_logs', Path(__file__).with_name('phase23-environment.py'))
environment = importlib.util.module_from_spec(loader)
loader.loader.exec_module(environment)


@unittest.skipUnless(os.name == 'posix' and hasattr(os, 'O_NOFOLLOW'), 'Linux file semantics required')
class LogTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='phase23-env-log-pure-', dir='/tmp')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.root.chmod(0o700)
        self.task = environment.Task.__new__(environment.Task)
        self.task.uid = 65534 if os.getuid() == 0 else os.getuid()
        self.task.gid = 65534 if os.getuid() == 0 else os.getgid()
        self.task.state = {'processes': {}}
        task_patch = patch.object(environment, 'TASK', self.root)
        task_patch.start()
        self.addCleanup(task_patch.stop)
        self.path = self.root / 'service-identity.log'

    def owned_file(self, path=None, content=b'prior-log\n', mode=0o600):
        path = path or self.path
        path.write_bytes(content)
        path.chmod(mode)
        if os.getuid() == 0:
            os.chown(path, self.task.uid, self.task.gid)
        return path

    def assert_private(self, path):
        details = path.lstat()
        self.assertTrue(stat.S_ISREG(details.st_mode))
        self.assertEqual((details.st_uid, stat.S_IMODE(details.st_mode), details.st_nlink),
                         (self.task.uid, 0o600, 1))

    def test_umask_0022_all_native_logs_private_before_spawn_and_native_append(self):
        self.task.service = Mock(side_effect=[None, (self.root / 'service-identity.pid', {})])
        self.task.tools = Mock(return_value=self.root / 'tools')

        def spawn(*_args):
            for service in environment.DAEMONS:
                path = self.root / ('service-' + service + '.log')
                self.assert_private(path)
                self.assertEqual(path.read_bytes(), b'')
                # Reproduce the unchanged shared runner's separate append open.
                with path.open('ab', buffering=0) as log:
                    log.write(b'native-output\n')
                self.assert_private(path)

        self.task.spawn = Mock(side_effect=spawn)
        previous = os.umask(0o022)
        try:
            environment.start_service(self.task, 'identity')
        finally:
            os.umask(previous)
        self.task.spawn.assert_called_once()
        for service in environment.DAEMONS:
            path = self.root / ('service-' + service + '.log')
            self.assertEqual(path.read_bytes(), b'native-output\n')
            self.assert_private(path)

    def test_good_empty_and_nonempty_logs_preserve_inode_and_append(self):
        for content in (b'', b'prior-log\n'):
            with self.subTest(empty=not content):
                self.owned_file(content=content)
                before = self.path.lstat()
                with (patch.object(environment.os, 'fchmod', side_effect=AssertionError('existing chmod')),
                      patch.object(environment.os, 'fchown', side_effect=AssertionError('existing chown'))):
                    fd = self.task.open_log(self.path)
                try:
                    os.write(fd, b'appended\n')
                finally:
                    os.close(fd)
                self.assertEqual(self.path.read_bytes(), content + b'appended\n')
                self.assertEqual(self.path.lstat().st_ino, before.st_ino)
                self.assert_private(self.path)
                self.path.unlink()

    def test_existing_0644_is_rejected_unchanged_before_any_spawn(self):
        self.owned_file(mode=0o644)
        before = self.path.lstat()
        self.task.spawn = Mock()
        self.task.service = Mock()
        with (patch.object(environment.os, 'fchmod', side_effect=AssertionError('foreign chmod')),
              patch.object(environment.os, 'fchown', side_effect=AssertionError('foreign chown'))):
            with self.assertRaisesRegex(RuntimeError, 'PRIVATE_FILE_INVALID'):
                environment.start_service(self.task, 'identity')
        self.assertEqual(self.path.read_bytes(), b'prior-log\n')
        self.assertEqual((self.path.lstat().st_uid, self.path.lstat().st_mode, self.path.lstat().st_ino),
                         (before.st_uid, before.st_mode, before.st_ino))
        self.task.spawn.assert_not_called()
        self.task.service.assert_not_called()

    @unittest.skipUnless(os.getuid() == 0, 'root is required to make an alien-owner fixture')
    def test_root_owned_0600_is_rejected_without_chown(self):
        self.path.write_bytes(b'root-private-log\n')
        self.path.chmod(0o600)
        with (patch.object(environment.os, 'fchmod', side_effect=AssertionError('root chmod')),
              patch.object(environment.os, 'fchown', side_effect=AssertionError('root chown'))):
            with self.assertRaisesRegex(RuntimeError, 'PRIVATE_FILE_INVALID'):
                self.task.open_log(self.path)
        self.assertEqual((self.path.stat().st_uid, self.path.read_bytes()), (0, b'root-private-log\n'))

    def test_symlink_and_dangling_symlink_rejected_without_target_changes(self):
        target = self.owned_file(self.root / 'target.log')
        before = target.stat()
        for destination in (target, self.root / 'absent-target'):
            with self.subTest(dangling=not destination.exists()):
                self.path.symlink_to(destination)
                with self.assertRaisesRegex(RuntimeError, 'SYMLINK_REJECTED'):
                    self.task.open_log(self.path)
                self.assertTrue(self.path.is_symlink())
                self.path.unlink()
        self.assertEqual(target.read_bytes(), b'prior-log\n')
        self.assertEqual((target.stat().st_uid, target.stat().st_mode), (before.st_uid, before.st_mode))
        self.assertFalse((self.root / 'absent-target').exists())

    def test_hardlink_rejected(self):
        target = self.owned_file(self.root / 'target.log')
        os.link(target, self.path)
        with self.assertRaisesRegex(RuntimeError, 'FILE_INVALID'):
            self.task.open_log(self.path)
        self.assertEqual(target.read_bytes(), b'prior-log\n')
        self.assertEqual(target.stat().st_nlink, 2)

    def test_fifo_rejected_without_open_or_block(self):
        os.mkfifo(self.path, 0o600)
        with self.assertRaisesRegex(RuntimeError, 'FILE_INVALID'):
            self.task.open_log(self.path)
        self.assertTrue(stat.S_ISFIFO(self.path.lstat().st_mode))

    def test_directory_rejected(self):
        self.path.mkdir(mode=0o700)
        with self.assertRaisesRegex(RuntimeError, 'FILE_INVALID'):
            self.task.open_log(self.path)
        self.assertTrue(self.path.is_dir())

    def test_outside_task_path_rejected_without_creation(self):
        with self.assertRaisesRegex(RuntimeError, 'LOG_PATH_INVALID'):
            self.task.open_log(self.root / 'missing' / 'nested.log')
        self.assertFalse((self.root / 'missing').exists())

    def test_inode_replacement_after_open_is_rejected(self):
        self.owned_file()
        original_open = os.open

        def replacing_open(path, flags, *args, **kwargs):
            fd = original_open(path, flags, *args, **kwargs)
            if flags & os.O_APPEND and not flags & os.O_CREAT:
                self.path.rename(self.root / 'old.log')
                self.owned_file(content=b'replacement\n')
            return fd

        with patch.object(environment.os, 'open', side_effect=replacing_open):
            with self.assertRaisesRegex(RuntimeError, 'LOG_FILE_CHANGED'):
                self.task.open_log(self.path)
        self.assertEqual(self.path.read_bytes(), b'replacement\n')
        self.assertEqual((self.root / 'old.log').read_bytes(), b'prior-log\n')


if __name__ == '__main__':
    unittest.main()
