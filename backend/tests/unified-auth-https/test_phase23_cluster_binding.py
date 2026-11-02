"""Owned file/identity tests; database query is mocked, no sockets or services."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

loader = importlib.util.spec_from_file_location('phase23_cluster_binding_test_target',
    Path(__file__).with_name('phase23_cluster_binding.py'))
binding = importlib.util.module_from_spec(loader)
loader.loader.exec_module(binding)


class IdentifierTests(unittest.TestCase):
    def test_uint64_canonical_postgres_identity(self):
        for value in ('1', '18446744073709551615', '9876543210123456789'):
            self.assertEqual(binding.identifier(value), value)
        for value in ('0', '01', '-1', '18446744073709551616', '', 1, True, '1\n', '1.0'):
            with self.subTest(value=value), self.assertRaisesRegex(RuntimeError, 'SYSTEM_IDENTIFIER_INVALID'):
                binding.identifier(value)


@unittest.skipUnless(os.name == 'posix' and os.getuid() == 0, 'Linux root file-ownership fixture required')
class ProtectedBindingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='rust-saas-unified-auth-binding-test-', dir='/tmp')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.root.chmod(0o700)
        os.chown(self.root, 65534, 65534)
        self.data = self.root / 'data'
        self.data.mkdir(mode=0o700)
        os.chown(self.data, 65534, 65534)
        self.owner = self.root / 'OWNER'
        self.owner.write_bytes((self.root.name + '-pg55494\n').encode())
        self.owner.chmod(0o600)
        os.chown(self.owner, 65534, 65534)
        self.actual = {'systemIdentifier': '9876543210123456789', 'dataDirectory': str(self.data),
            'database': 'postgres', 'user': 'postgres', 'host': '127.0.0.1', 'port': 55494}

    def initialize(self, actual=None, returncode=0):
        with patch.object(binding.subprocess, 'run', return_value=SimpleNamespace(
                returncode=returncode, stdout=json.dumps(actual or self.actual).encode())) as run:
            result = binding.initialize(self.root)
        self.assertEqual(run.call_count, 1)
        command = run.call_args
        self.assertEqual(command.kwargs['env']['PGHOST'], '127.0.0.1')
        self.assertEqual(command.kwargs['env']['PGPORT'], '55494')
        self.assertNotIn('PGPASSWORD', command.kwargs['env'])
        self.assertIn('pg_control_system()', command.kwargs['input'].decode())
        self.assertIn('host(inet_server_addr())', command.kwargs['input'].decode())
        return result

    def rewrite(self, value):
        path = self.root / binding.NAME
        path.write_text(json.dumps(value))
        path.chmod(0o644)

    def test_new_identity_anchored_to_root_owned_nonsecret_file(self):
        result = self.initialize()
        path = self.root / binding.NAME
        self.assertEqual((path.stat().st_uid, path.stat().st_mode & 0o777), (0, 0o644))
        self.assertEqual(binding.load_binding(self.root), result)
        guard = binding.sql_guard(result)
        self.assertIn(self.actual['systemIdentifier'], guard)
        self.assertIn("current_setting('data_directory')", guard)
        self.assertEqual(result['taskInode'], self.root.stat().st_ino)
        self.assertEqual(result['ownerInode'], self.owner.stat().st_ino)

    def test_root_creation_cannot_overwrite_prior_binding(self):
        original = self.initialize()
        with self.assertRaises(FileExistsError):
            self.initialize()
        self.assertEqual(binding.load_binding(self.root), original)

    def test_wrong_live_cluster_location_or_endpoint_refused_before_write(self):
        for field, value in (('dataDirectory', '/var/lib/postgresql/data'), ('database', 'other'),
                ('user', 'other'), ('host', '127.0.0.2'), ('port', 5432)):
            actual = {**self.actual, field: value}
            with self.subTest(field=field), self.assertRaisesRegex(RuntimeError, 'DATABASE_TARGET_INVALID'):
                self.initialize(actual)
            self.assertFalse((self.root / binding.NAME).exists())
        with self.assertRaisesRegex(RuntimeError, 'DATABASE_QUERY_FAILED'):
            self.initialize(returncode=1)
        self.assertFalse((self.root / binding.NAME).exists())

    def test_existing_owner_replacement_invalidates_binding(self):
        self.initialize()
        self.owner.rename(self.root / 'old-owner')
        self.owner.write_bytes((self.root.name + '-pg55494\n').encode())
        self.owner.chmod(0o600)
        os.chown(self.owner, 65534, 65534)
        with self.assertRaisesRegex(RuntimeError, 'OWNERSHIP_CHANGED'):
            binding.load_binding(self.root)

    def test_foreign_binding_fields_and_noncanonical_sysid_refused(self):
        original = self.initialize()
        for field, value in (('taskDir', '/tmp/foreign'), ('taskUid', 0), ('taskInode', 1),
                ('dataDirectory', '/tmp/foreign/data'), ('host', 'localhost'), ('port', True)):
            changed = {**original, field: value}
            self.rewrite(changed)
            with self.subTest(field=field), self.assertRaisesRegex(RuntimeError, 'OWNERSHIP_CHANGED'):
                binding.load_binding(self.root)
        self.rewrite({**original, 'systemIdentifier': '18446744073709551616'})
        with self.assertRaisesRegex(RuntimeError, 'SYSTEM_IDENTIFIER_INVALID'):
            binding.load_binding(self.root)
        self.rewrite({**original, 'extra': 'ignored-is-forbidden'})
        with self.assertRaisesRegex(RuntimeError, 'SCHEMA_INVALID'):
            binding.load_binding(self.root)

    def test_user_writable_wrong_owner_or_linked_binding_rejected(self):
        self.initialize()
        path = self.root / binding.NAME
        path.chmod(0o666)
        with self.assertRaisesRegex(RuntimeError, 'FILE_INVALID'):
            binding.load_binding(self.root)
        path.chmod(0o644)
        os.chown(path, 65534, 65534)
        with self.assertRaisesRegex(RuntimeError, 'FILE_INVALID'):
            binding.load_binding(self.root)
        os.chown(path, 0, 0)
        os.link(path, self.root / 'alias')
        with self.assertRaisesRegex(RuntimeError, 'FILE_INVALID'):
            binding.load_binding(self.root)

    def test_data_directory_symlink_or_wrong_owner_refused(self):
        self.initialize()
        os.chown(self.data, 0, 0)
        with self.assertRaisesRegex(RuntimeError, 'DATA_DIRECTORY_INVALID'):
            binding.load_binding(self.root)
        os.chown(self.data, 65534, 65534)
        self.data.rename(self.root / 'real-data')
        self.data.symlink_to(self.root / 'real-data', target_is_directory=True)
        with self.assertRaisesRegex(RuntimeError, 'DATA_DIRECTORY_INVALID'):
            binding.load_binding(self.root)


if __name__ == '__main__':
    unittest.main()
