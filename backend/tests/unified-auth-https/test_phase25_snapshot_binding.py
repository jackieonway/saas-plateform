"""Real file/manifest binding regression; no DB, processes, sockets or Docker."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

loader = importlib.util.spec_from_file_location('phase25_snapshot_binding',
    Path(__file__).with_name('phase23-environment.py'))
environment = importlib.util.module_from_spec(loader)
loader.loader.exec_module(environment)


class SnapshotBindingTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='phase25-snapshot-binding-pure-', dir='/tmp')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.root.chmod(0o700)
        patched = patch.object(environment, 'TASK', self.root)
        patched.start()
        self.addCleanup(patched.stop)
        self.task = environment.Task.__new__(environment.Task)
        self.task.uid, self.task.gid = os.getuid(), os.getgid()
        self.task.state = None
        self.compiled = 'saas-identity/src/lib.rs'
        self.frontend = 'saas-plateform/frontend/dist/index.html'
        self.helper = 'saas-plateform/backend/tests/unified-auth-https/operational-helper.py'
        self.binaries = {*environment.SERVICES.values(), 'platform-migrate', 'portal-migrate',
            'phase23-db-auth-probe', 'stores-api', 'stores-migrate', 'work-orders-api',
            'work-orders-migrate', 'saas-notification-ws', 'saas-platform-admin'}

    def write(self, path, raw):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(raw)
        path.chmod(0o600)

    def fixture(self, phase=25, missing=(), extra=(), tools_changes=None):
        self.build = f'phase{phase}-apps-test'
        self.tools = f'phase{phase}-tools-test'
        rp = environment.RP_FILES | (environment.PHASE25_RP_TEST_FILES if phase == 25 else set())
        rp = (rp - set(missing)) | set(extra)
        inputs = {environment.RP_PREFIX + name: b'PUBLIC_RP_FIXTURE ' + name.encode()
                  for name in rp}
        inputs.update({self.compiled: b'COMPILED_SOURCE', self.frontend: b'FROZEN_DIST',
                       self.helper: b'ORIGINAL_OPERATIONS'})
        manifests = {}
        for label in (self.build, self.tools):
            values = {**inputs, **(tools_changes or {})} if label == self.tools else inputs
            hashes = {}
            for relative, raw in values.items():
                self.write(self.root / ('source-snapshot-' + label) / relative, raw)
                hashes[relative] = hashlib.sha256(raw).hexdigest()
            manifest = {'inputs': hashes, 'snapshotSha256': environment.sha(
                json.dumps(hashes, sort_keys=True).encode()), 'includePortal': True,
                'includeAllProjects': phase == 25, 'completeFrontendOutputs': phase == 25}
            manifests[label] = manifest
            self.write(self.root / ('snapshot-' + label + '.json'), json.dumps(manifest).encode())
        binaries = {}
        for name in self.binaries:
            raw = b'ATTESTED_BINARY_FIXTURE ' + name.encode()
            self.write(self.root / 'bin' / name, raw)
            binaries[name] = environment.sha(raw)
        attestation = {**manifests[self.build], 'snapshotLabel': self.build,
            'builtServices': ['identity', 'platform', 'portal', 'stores', 'work-orders', 'notification', 'cli'],
            'binaries': binaries}
        self.write(self.root / ('build-attestation-' + self.build + '.json'),
                   json.dumps(attestation).encode())

    def validate(self):
        return self.task.snapshot(self.tools, build_label=self.build)

    def test_full_candidate_accepts_exact_eighteen_rp_files_and_changed_operations(self):
        self.fixture(tools_changes={self.helper: b'CURRENT_OPERATIONS'})
        snapshot, manifest, attestation = self.validate()
        self.assertEqual(snapshot.name, 'source-snapshot-' + self.tools)
        self.assertEqual(attestation['snapshotLabel'], self.build)
        self.assertNotEqual(manifest['snapshotSha256'], attestation['snapshotSha256'])

    def test_phase23_exact_fourteen_file_contract_is_preserved(self):
        self.fixture(phase=23)
        self.validate()

    def test_missing_production_rp_file_is_rejected(self):
        self.fixture(missing={'src/oidc.mjs'})
        with self.assertRaisesRegex(RuntimeError, 'RP_SNAPSHOT_INPUTS_INVALID'):
            self.validate()

    def test_missing_full_candidate_rp_test_is_rejected(self):
        self.fixture(missing={'tests/capacity.test.mjs'})
        with self.assertRaisesRegex(RuntimeError, 'RP_SNAPSHOT_INPUTS_INVALID'):
            self.validate()

    def test_unrecorded_rp_program_or_test_is_rejected(self):
        self.fixture(extra={'tests/unapproved.test.mjs'})
        with self.assertRaisesRegex(RuntimeError, 'RP_SNAPSHOT_INPUTS_INVALID'):
            self.validate()

    def test_phase23_does_not_accept_phase25_inventory(self):
        self.fixture(phase=23, extra=environment.PHASE25_RP_TEST_FILES)
        with self.assertRaisesRegex(RuntimeError, 'RP_SNAPSHOT_INPUTS_INVALID'):
            self.validate()

    def test_current_source_drift_cannot_reuse_original_binary(self):
        self.fixture(tools_changes={self.compiled: b'CHANGED_COMPILED_SOURCE'})
        with self.assertRaisesRegex(RuntimeError, 'COMPILED_APPLICATION_INPUT_DRIFT'):
            self.validate()

    def test_live_frontend_cannot_replace_frozen_candidate_dist(self):
        self.fixture(tools_changes={self.frontend: b'LIVE_DIST'})
        with self.assertRaisesRegex(RuntimeError, 'COMPILED_APPLICATION_INPUT_DRIFT'):
            self.validate()

    def test_changed_business_binary_is_rejected(self):
        self.fixture()
        self.write(self.root / 'bin/stores-api', b'UNATTESTED_BINARY')
        with self.assertRaisesRegex(RuntimeError, 'PHASE25_BINARY_ATTESTATION_INVALID'):
            self.validate()


if __name__ == '__main__':
    unittest.main()
