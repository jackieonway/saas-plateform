"""Phase19 frozen-source gate regression; only dedicated temporary files."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

loader = importlib.util.spec_from_file_location('prepare_frozen_source_target', Path(__file__).with_name('prepare.py'))
prepare = importlib.util.module_from_spec(loader)
loader.loader.exec_module(prepare)


class FrozenSourceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='phase23-frozen-source-pure-', dir='/tmp')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.task = self.root / 'task'
        self.task.mkdir(mode=0o700)
        self.label = 'phase23-tools-r5'
        self.backend = self.task / ('source-snapshot-' + self.label) / 'saas-plateform/backend'
        self.required = ('Cargo.toml', 'migrations/0001_init.sql', 'ops/identity-deployment/create-roles.sql',
            'ops/identity-deployment/provision.sql', 'ops/identity-deployment/verify.sql',
            'tests/unified-auth-https/prepare.py')
        for relative in self.required:
            path = self.backend / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('PURE_FROZEN_FILE_' + relative)
            path.chmod(0o600)
        self.inputs = {'saas-plateform/backend/' + relative:
            hashlib.sha256((self.backend / relative).read_bytes()).hexdigest() for relative in self.required}
        self.manifest = self.task / ('snapshot-' + self.label + '.json')
        self.write_manifest()

    def write_manifest(self, digest=None):
        self.manifest.write_text(json.dumps({'copiedAt': 'PURE_TEST', 'includePortal': True,
            'inputs': self.inputs, 'snapshotSha256': digest or hashlib.sha256(json.dumps(self.inputs, sort_keys=True).encode()).hexdigest()}))
        self.manifest.chmod(0o600)

    def test_explicit_snapshot_accepts_prepare_script_in_the_same_snapshot(self):
        with patch.object(prepare, '__file__', str(self.backend / 'tests/unified-auth-https/prepare.py')):
            self.assertEqual(prepare.resolve_backend_source(self.task, self.backend, True), self.backend)
            with self.assertRaisesRegex(RuntimeError, 'explicit owned frozen'):
                prepare.resolve_backend_source(self.task, None, True)

    def test_live_workspace_and_arbitrary_task_backend_are_rejected(self):
        live = self.root / 'workspace/saas-plateform/backend'
        for relative in self.required:
            path = live / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('PURE_LIVE_SOURCE')
        with patch.object(prepare, '__file__', str(live / 'tests/unified-auth-https/prepare.py')):
            with self.assertRaisesRegex(RuntimeError, 'explicit owned frozen'):
                prepare.resolve_backend_source(self.task, live, True)
        arbitrary = self.task / 'arbitrary/saas-plateform/backend'
        arbitrary.parent.mkdir(parents=True)
        self.backend.rename(arbitrary)
        with self.assertRaisesRegex(RuntimeError, 'exact owned snapshot layout'):
            prepare.resolve_backend_source(self.task, arbitrary, True)

    def test_missing_manifest_whole_manifest_tamper_and_source_drift_are_rejected(self):
        self.manifest.unlink()
        with self.assertRaises(FileNotFoundError):
            prepare.resolve_backend_source(self.task, self.backend, True)
        self.write_manifest(digest='0' * 64)
        with self.assertRaisesRegex(RuntimeError, 'manifest is invalid'):
            prepare.resolve_backend_source(self.task, self.backend, True)
        self.write_manifest()
        sql = self.backend / 'ops/identity-deployment/provision.sql'
        sql.write_text('PURE_CHANGED_ROLE_SQL')
        with self.assertRaisesRegex(RuntimeError, 'snapshot input changed'):
            prepare.resolve_backend_source(self.task, self.backend, True)
        self.assertEqual(sql.read_text(), 'PURE_CHANGED_ROLE_SQL')

    def test_required_role_sql_missing_and_link_aliases_are_rejected(self):
        alias = self.task / 'backend-alias'
        alias.symlink_to(self.backend, target_is_directory=True)
        with self.assertRaisesRegex(RuntimeError, 'explicit owned frozen'):
            prepare.resolve_backend_source(self.task, alias, True)
        self.inputs.pop('saas-plateform/backend/ops/identity-deployment/verify.sql')
        self.write_manifest()
        with self.assertRaisesRegex(RuntimeError, 'inputs are incomplete'):
            prepare.resolve_backend_source(self.task, self.backend, True)

    def test_manifest_or_source_public_modes_fail_closed(self):
        self.manifest.chmod(0o644)
        with self.assertRaisesRegex(RuntimeError, 'FILE_INVALID'):
            prepare.resolve_backend_source(self.task, self.backend, True)
        self.manifest.chmod(0o600)
        (self.backend / 'ops/identity-deployment/create-roles.sql').chmod(0o644)
        with self.assertRaisesRegex(RuntimeError, 'FILE_INVALID'):
            prepare.resolve_backend_source(self.task, self.backend, True)


if __name__ == '__main__':
    unittest.main()
