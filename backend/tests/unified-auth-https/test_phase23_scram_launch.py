"""Pure root/unprivileged probe launch test; no probe or DB is started."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

loader = importlib.util.spec_from_file_location('phase23_scram_launch_target',
    Path(__file__).with_name('phase23-scram-cases.py'))
scram = importlib.util.module_from_spec(loader)
loader.loader.exec_module(scram)


class ScramLaunchTests(unittest.TestCase):
    def test_root_launch_uses_absolute_runuser_and_all_launches_keep_runtime_options_clean(self):
        task = SimpleNamespace(username='owned-user')
        command = ['/owned/bin/probe', '--input', '/owned/input.json', '--output', '/owned/output.json']
        for uid in (0, 1000):
            with self.subTest(uid=uid), patch.object(scram.os, 'getuid', return_value=uid), \
                 patch.object(scram.subprocess, 'run', return_value=SimpleNamespace(
                    returncode=0, stdout=b'DB_AUTH_PROBE_PASS\n', stderr=b'')) as run:
                self.assertEqual(scram.run_probe(task, Path(command[0]), Path(command[2]), Path(command[4])), 0)
                expected = ['/usr/sbin/runuser', '-u', task.username, '--', *command] if uid == 0 else command
                self.assertEqual(run.call_args.args[0], expected)
                self.assertFalse(any(name.upper().startswith('PG') for name in run.call_args.kwargs['env']))


if __name__ == '__main__':
    unittest.main()
