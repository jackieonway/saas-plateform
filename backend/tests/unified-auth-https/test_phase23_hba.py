"""Pure root file-journal tests. PG queries/reloads are mocked; no real HBA/DB."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('phase23_hba', Path(__file__).with_name('phase23-hba.py'))
hba = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(hba)


@unittest.skipUnless(os.name == 'posix' and os.getuid() == 0, 'root-only journal filesystem tests')
class HbaTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='phase23-hba-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.root.chmod(0o700)
        self.data = self.root / 'data'
        self.data.mkdir(mode=0o700)
        self.path = self.data / 'pg_hba.conf'
        self.original = b'# owned fixture\nhost all all 127.0.0.1/32 trust\nhost all all 127.0.0.2/32 trust\n'
        self.path.write_bytes(self.original)
        self.path.chmod(0o600)
        os.utime(self.path, ns=(1000000000, 2000000000))
        self.original_stat = hba.metadata(self.path.stat())
        self.deployment = '019a221c-e019-7000-8000-000000000001'
        self.name = 'unified_auth_https_' + '0' * 32
        self.prefix = 'uah_' + '0' * 12
        self.binding = {'taskUid': 0, 'systemIdentifier': '7701548153213487842',
                        'dataDirectory': str(self.data)}
        self.manifest = {'database': self.name, 'rolePrefix': self.prefix,
            'clusterId': self.binding['systemIdentifier'], 'portal': {
                'database': 'unified_auth_portal_' + '0' * 32,
                'owner': self.prefix + '_portal_owner', 'runtime': self.prefix + '_portal_runtime',
                'marker': 'unified-auth-https-portal/1/' + self.deployment, 'initializationPending': False}}
        self.controller = hba.Hba(self.root, self.deployment, journal_parent=self.root)
        self.guarded = (self.binding, self.manifest, 'a' * 64, self.path)
        self.guard = patch.object(hba.Hba, 'guard', return_value=self.guarded)
        self.guard.start()
        self.addCleanup(self.guard.stop)
        self.pg = patch.object(hba, 'pg', return_value={'reloaded': True, 'errors': 0})
        self.pg_mock = self.pg.start()
        self.addCleanup(self.pg.stop)

    def test_install_prepends_only_exact_two_scram_rules(self):
        result = self.controller.install()
        installed = self.path.read_bytes()
        rules = [line.split() for line in installed.splitlines() if line.startswith(b'host ')][:2]
        self.assertEqual(len(rules), 2)
        self.assertEqual(rules[0], [b'host', (self.name + ',unified_auth_restore_' + '0' * 32).encode(),
            (self.prefix + '_identity_runtime,' + self.prefix + '_platform_runtime').encode(),
            b'127.0.0.2/32', b'scram-sha-256'])
        self.assertEqual(rules[1], [b'host', ('unified_auth_portal_' + '0' * 32 +
            ',unified_auth_restore_portal_' + '0' * 32).encode(),
            (self.prefix + '_portal_runtime').encode(), b'127.0.0.2/32', b'scram-sha-256'])
        self.assertTrue(installed.endswith(self.original))
        self.assertEqual(result['port'], 55494)
        self.assertEqual(self.controller.journal.stat().st_mode & 0o777, 0o700)
        self.assertEqual(self.controller.journal.stat().st_uid, 0)
        self.assertEqual(set(p.name for p in self.controller.journal.iterdir()),
                         {'LOCK', 'journal.json', 'original.hba'})
        self.controller.restore()
        self.controller.cleanup_receipt()

    def test_restore_exact_original_bytes_stat_and_cleanup_receipt(self):
        self.controller.install()
        result = self.controller.restore()
        self.assertEqual(hba.metadata(self.path.stat()), self.original_stat)
        self.assertEqual(self.path.read_bytes(), self.original)
        self.assertTrue(result['receiptRetained'])
        self.assertTrue(result['originalBytesAndStatRestored'])
        self.assertEqual(set(p.name for p in self.controller.journal.iterdir()), {'LOCK', 'journal.json'})
        self.assertEqual(self.controller.restore()['finalSha256'], result['finalSha256'])
        self.assertTrue(self.controller.cleanup_receipt()['journalRemoved'])
        self.assertFalse(self.controller.journal.exists())

    def test_context_body_failure_still_restores(self):
        with self.assertRaisesRegex(RuntimeError, 'BODY_FAILED'):
            with hba.installed_hba(self.root, self.deployment, journal_parent=self.root):
                self.assertNotEqual(self.path.read_bytes(), self.original)
                raise RuntimeError('BODY_FAILED')
        self.assertEqual(self.path.read_bytes(), self.original)
        self.assertEqual(self.controller.status()['status'], 'PHASE23_HBA_RESTORED')
        self.controller.cleanup_receipt()

    def test_parse_or_reload_failure_rolls_back_own_change(self):
        self.pg_mock.side_effect = [{'reloaded': True, 'errors': 1}, {'reloaded': True, 'errors': 0}]
        with self.assertRaisesRegex(RuntimeError, 'RELOAD_OR_PARSE_FAILED'):
            self.controller.install()
        self.assertEqual(self.path.read_bytes(), self.original)
        self.assertEqual(self.controller.status()['status'], 'PHASE23_HBA_RESTORED')
        self.controller.cleanup_receipt()

    def test_interruption_after_replace_before_state_save_restores(self):
        replace = self.controller.replace_hba
        first = True
        def interrupted(*args):
            nonlocal first
            replace(*args)
            if first:
                first = False
                raise KeyboardInterrupt
        with patch.object(self.controller, 'replace_hba', side_effect=interrupted):
            with self.assertRaises(KeyboardInterrupt):
                self.controller.install()
        self.assertEqual(self.path.read_bytes(), self.original)
        self.assertEqual(self.controller.status()['status'], 'PHASE23_HBA_RESTORED')
        self.controller.cleanup_receipt()

    def test_external_change_is_preserved_and_journal_retained(self):
        self.controller.install()
        external = self.path.read_bytes() + b'# external owned edit\n'
        self.path.write_bytes(external)
        with self.assertRaisesRegex(RuntimeError, 'EXTERNAL_CHANGE_PRESERVED'):
            self.controller.restore()
        self.assertEqual(self.path.read_bytes(), external)
        self.assertTrue((self.controller.journal / 'original.hba').exists())

    def test_duplicate_install_cannot_clobber_active_journal(self):
        self.controller.install()
        before = (self.controller.journal / 'journal.json').read_bytes()
        installed = self.path.read_bytes()
        with self.assertRaisesRegex(RuntimeError, 'PREEXISTING_PHASE23_RULES'):
            self.controller.install()
        self.assertEqual((self.controller.journal / 'journal.json').read_bytes(), before)
        self.assertEqual(self.path.read_bytes(), installed)
        self.controller.restore()
        self.controller.cleanup_receipt()

    def test_context_install_failure_cannot_restore_another_callers_journal(self):
        self.controller.install()
        installed = self.path.read_bytes()
        with self.assertRaisesRegex(RuntimeError, 'PREEXISTING_PHASE23_RULES'):
            with hba.installed_hba(self.root, self.deployment, journal_parent=self.root):
                self.fail('must not reach body')
        self.assertEqual(self.path.read_bytes(), installed)
        self.assertEqual(self.controller.status()['state'], 'INSTALLED')
        self.controller.restore()
        self.controller.cleanup_receipt()

    def test_failed_restore_reload_can_be_retried_without_widening_rules(self):
        self.controller.install()
        self.pg_mock.return_value = {'reloaded': False, 'errors': 0}
        with self.assertRaisesRegex(RuntimeError, 'RELOAD_OR_PARSE_FAILED'):
            self.controller.restore()
        self.assertEqual(self.path.read_bytes(), self.original)
        self.assertTrue(self.controller.journal.exists())
        self.pg_mock.return_value = {'reloaded': True, 'errors': 0}
        self.controller.restore()
        self.controller.cleanup_receipt()
        self.assertFalse(self.controller.journal.exists())

    def test_absent_journal_cannot_be_reported_as_successful_restore(self):
        with self.assertRaises(FileNotFoundError):
            self.controller.restore()
        self.assertEqual(self.controller.status()['status'], 'PHASE23_HBA_JOURNAL_ABSENT')

    def test_repeated_restore_and_status_accept_only_original_stable_stat(self):
        self.controller.install()
        self.controller.restore()
        # Access time may advance when PostgreSQL or an independent reader reads.
        os.utime(self.path, ns=(self.original_stat['atimeNs'] + 100, self.original_stat['mtimeNs']))
        self.assertEqual(self.controller.restore()['status'], 'PHASE23_HBA_RESTORED')
        self.assertEqual(self.controller.status()['status'], 'PHASE23_HBA_RESTORED')
        os.utime(self.path, ns=(self.original_stat['atimeNs'], self.original_stat['mtimeNs'] + 100))
        with self.assertRaisesRegex(RuntimeError, 'RESTORED_RECEIPT_ACTUAL_CHANGED'):
            self.controller.restore()
        self.assertTrue(self.controller.journal.exists())

    def test_restored_receipt_rejects_changed_bytes_or_permissions(self):
        self.controller.install()
        self.controller.restore()
        self.path.chmod(0o640)
        with self.assertRaisesRegex(RuntimeError, 'RESTORED_RECEIPT_ACTUAL_CHANGED'):
            self.controller.status()
        self.path.chmod(0o600)
        self.path.write_bytes(self.original + b'# changed\n')
        with self.assertRaisesRegex(RuntimeError, 'RESTORED_RECEIPT_ACTUAL_CHANGED'):
            self.controller.cleanup_receipt()
        self.assertTrue(self.controller.journal.exists())

    def test_cleanup_requires_preexisting_verified_restored_receipt(self):
        self.controller.install()
        installed = self.path.read_bytes()
        with self.assertRaisesRegex(RuntimeError, 'RESTORATION_RECEIPT_REQUIRED'):
            self.controller.cleanup_receipt()
        self.assertEqual(self.path.read_bytes(), installed)
        self.controller.restore()
        self.controller.cleanup_receipt()

    def test_journal_original_tampering_is_rejected(self):
        self.controller.install()
        (self.controller.journal / 'original.hba').write_bytes(b'changed')
        installed = self.path.read_bytes()
        with self.assertRaisesRegex(RuntimeError, 'JOURNAL_ORIGINAL_CHANGED'):
            self.controller.restore()
        self.assertEqual(self.path.read_bytes(), installed)

    def test_changed_manifest_keeps_installed_hba_and_journal(self):
        self.controller.install()
        installed = self.path.read_bytes()
        with patch.object(hba.Hba, 'guard', return_value=(self.binding, self.manifest, 'b' * 64, self.path)):
            with self.assertRaisesRegex(RuntimeError, 'JOURNAL_ACTUAL_BINDING_CHANGED'):
                self.controller.restore()
        self.assertEqual(self.path.read_bytes(), installed)
        self.assertTrue(self.controller.journal.exists())

    def test_nonroot_cannot_create_journal(self):
        with patch.object(hba.os, 'getuid', return_value=1000):
            with self.assertRaisesRegex(RuntimeError, 'ROOT_REQUIRED'):
                hba.Hba(self.root, self.deployment, journal_parent=self.root)
        self.assertFalse(self.controller.journal.exists())

    def test_install_guard_failure_precedes_hba_read_and_journal_creation(self):
        with patch.object(hba.Hba, 'guard', side_effect=RuntimeError('PHASE23_HBA_DEPLOYMENT_ROLE_BINDING_CHANGED')), \
             patch.object(hba, 'read_regular') as read:
            with self.assertRaisesRegex(RuntimeError, 'DEPLOYMENT_ROLE_BINDING_CHANGED'):
                self.controller.install()
            read.assert_not_called()
        self.assertEqual(self.path.read_bytes(), self.original)
        self.assertFalse(self.controller.journal.exists())

    def test_symlink_hba_is_rejected_before_mutation(self):
        elsewhere = self.data / 'elsewhere'
        elsewhere.write_bytes(b'outside')
        self.path.unlink()
        self.path.symlink_to(elsewhere)
        with self.assertRaisesRegex(RuntimeError, 'FILE_INVALID'):
            self.controller.install()
        self.assertEqual(elsewhere.read_bytes(), b'outside')
        self.assertFalse(self.controller.journal.exists())

    def test_cli_does_not_print_exception_or_hba_bytes(self):
        error = io.StringIO()
        with patch.object(hba.Hba, 'status', side_effect=RuntimeError('raw secret HBA content')), \
             contextlib.redirect_stderr(error):
            self.assertEqual(hba.main(['status', '--task-dir', str(self.root), '--deployment-id', self.deployment]), 2)
        self.assertNotIn('raw secret', error.getvalue())
        self.assertEqual(json.loads(error.getvalue())['code'], 'PHASE23_HBA_REJECTED')


class GuardTests(unittest.TestCase):
    def setUp(self):
        self.controller = hba.Hba.__new__(hba.Hba)
        self.controller.deployment = '019a221c-e019-7000-8000-000000000001'
        self.name = 'unified_auth_https_' + '0' * 32
        self.prefix = 'uah_' + '0' * 12
        self.binding = {'systemIdentifier': '7701548153213487842', 'dataDirectory': '/tmp/owned/data'}
        self.manifest = {'database': self.name, 'rolePrefix': self.prefix, 'portal': {'database': 'unified_auth_portal_' + '0' * 32}}
        self.databases = {self.name: (self.prefix + '_migration_owner', 'identity-deployment/1/' + self.controller.deployment),
            self.manifest['portal']['database']: (self.prefix + '_portal_owner', 'unified-auth-https-portal/1/' + self.controller.deployment)}
        self.actual = {'cluster': self.binding['systemIdentifier'], 'data': self.binding['dataDirectory'],
            'hba': self.binding['dataDirectory'] + '/pg_hba.conf', 'database': 'postgres', 'user': 'postgres',
            'host': '127.0.0.1', 'port': 55494, 'memberships': 0,
            'roles': [[self.prefix + '_' + suffix, False, False, False, False, False, False, suffix.endswith('_runtime')]
                      for suffix in sorted(hba.ROLES)],
            'databases': {name: list(value) for name, value in self.databases.items()}}
        self.state = {'deployment_id': self.controller.deployment,
            **{field: self.prefix + '_' + suffix for field, suffix in (
                ('migration_owner', 'migration_owner'), ('identity_owner', 'identity_owner'),
                ('platform_owner', 'platform_owner'), ('projection_owner', 'identity_owner'),
                ('revocation_bridge_owner', 'bridge_owner'), ('identity_runtime', 'identity_runtime'),
                ('platform_runtime', 'platform_runtime'))}}
        self.patch = patch.object(self.controller, 'binding', return_value=(self.binding, self.manifest, 'a' * 64, self.databases))
        self.patch.start()
        self.addCleanup(self.patch.stop)

    def call(self):
        with patch.object(hba, 'pg', side_effect=[self.actual, self.state]):
            return self.controller.guard()

    def test_exact_eight_roles_and_source_owners_pass(self):
        self.assertEqual(self.call()[-1], Path(self.actual['hba']))

    def test_projection_owner_matches_canonical_provision_not_revocation_bridge(self):
        provision = Path(__file__).resolve().parents[2] / 'ops/identity-deployment/provision.sql'
        source = provision.read_text()
        statement = hba.re.search(r'INSERT INTO public\.identity_deployment_state\(([^)]*)\)\s*'
                                  r'VALUES\(([^;]*)\);', source, hba.re.S)
        self.assertIsNotNone(statement, 'canonical deployment state provision statement required')
        columns = [item.strip() for item in statement.group(1).split(',')]
        values = [item.strip() for item in statement.group(2).split(',')]
        self.assertEqual(len(columns), len(values))
        assignments = dict(zip(columns, values))
        self.assertEqual(assignments['projection_owner'], 'identity_owner')
        self.assertEqual(assignments['revocation_bridge_owner'], 'bridge_owner')
        self.state['projection_owner'] = self.prefix + '_' + assignments['projection_owner']
        self.state['revocation_bridge_owner'] = self.prefix + '_' + assignments['revocation_bridge_owner']
        self.call()
        self.state['projection_owner'] = self.prefix + '_bridge_owner'
        with self.assertRaisesRegex(RuntimeError, 'DEPLOYMENT_ROLE_BINDING_CHANGED'):
            self.call()

    def test_changed_actual_system_identifier_or_directory_or_port_rejects(self):
        for field, value in [('cluster', '7701548153213487843'), ('data', '/tmp/foreign/data'),
                             ('hba', '/etc/postgresql/pg_hba.conf'), ('port', 5432)]:
            before = self.actual[field]
            self.actual[field] = value
            with self.subTest(field=field), self.assertRaisesRegex(RuntimeError, 'ACTUAL_TARGET_CHANGED'):
                self.call()
            self.actual[field] = before

    def test_extra_or_privileged_or_login_owner_role_rejects(self):
        self.actual['roles'].append([self.prefix + '_foreign'] + [False] * 7)
        with self.assertRaisesRegex(RuntimeError, 'ROLE_SET_CHANGED'):
            self.call()
        self.actual['roles'].pop()
        self.actual['roles'][0][1] = True
        with self.assertRaisesRegex(RuntimeError, 'ROLE_POWERS_CHANGED'):
            self.call()
        self.actual['roles'][0][1] = False
        self.actual['roles'][0][-1] = True
        with self.assertRaisesRegex(RuntimeError, 'ROLE_POWERS_CHANGED'):
            self.call()

    def test_foreign_database_comment_and_deployment_mapping_reject(self):
        self.actual['databases'][self.name][1] = 'foreign'
        with self.assertRaisesRegex(RuntimeError, 'DATABASE_OWNERSHIP_CHANGED'):
            self.call()
        self.actual['databases'][self.name] = list(self.databases[self.name])
        self.state['deployment_id'] = '019a221c-e019-7000-8000-000000000002'
        with self.assertRaisesRegex(RuntimeError, 'DEPLOYMENT_ROLE_BINDING_CHANGED'):
            self.call()


if __name__ == '__main__':
    unittest.main()
