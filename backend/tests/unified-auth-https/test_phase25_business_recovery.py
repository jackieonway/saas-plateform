"""Pure fail-closed ACL, carrier and cleanup contracts; no DR PASS claim."""
import copy
import importlib.util
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('phase25_business_recovery_test', HERE / 'phase25-business-recovery.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class RecoveryTests(unittest.TestCase):
    def test_database_acl_replay_preserves_owner_grantor_and_grant_option(self):
        rows = [dict(grantor='owned_owner', grantee='owned_owner', privilege='CREATE', grantable=False),
            dict(grantor='owned_owner', grantee='owned_runtime', privilege='CONNECT', grantable=True)]
        sql = module.database_acl_plan('new_target', 'owned_owner', rows)
        self.assertIn('REVOKE ALL PRIVILEGES ON DATABASE "new_target" FROM PUBLIC', sql)
        self.assertIn('SET LOCAL ROLE "owned_owner"', sql)
        self.assertIn('GRANT CONNECT ON DATABASE "new_target" TO "owned_runtime" WITH GRANT OPTION', sql)
        self.assertNotIn('source_database', sql)
        self.assertTrue(sql.endswith('RESET ROLE; COMMIT;'))

    def test_unknown_grantors_or_privileges_cannot_be_silently_discarded(self):
        row = dict(grantor='owned_owner', grantee='owned_runtime', privilege='CONNECT', grantable=False)
        for change in ({'grantor': 'another_grantor'}, {'privilege': 'ALL'}, {'grantable': 'true'}, {'grantee': 'runtime; DROP DATABASE source'}):
            with self.subTest(change=change), self.assertRaises(RuntimeError):
                module.database_acl_plan('new_target', 'owned_owner', [{**row, **change}])

    def test_acl_plan_refuses_empty_or_injectable_target(self):
        with self.assertRaises(RuntimeError):
            module.database_acl_plan('new_target', 'owned_owner', [])
        with self.assertRaises(RuntimeError):
            module.database_acl_plan('source; DROP DATABASE other', 'owned_owner', [])

    def carriers(self):
        business = dict(clusterId='1', deploymentId='owned', fixtureId='fixture', baseDatabase='base')
        row = dict(value='private-test-carrier', sha256=module.sha(b'private-test-carrier'), expiresAtMs=2e12,
            sourceResponseSha256='0' * 64)
        value = dict(contractVersion='unified-auth-phase25-business-recovery-carriers/1', **business,
            snapshot='phase25-tools-r4', carriers=dict(stores=copy.deepcopy(row), orders=copy.deepcopy(row)))
        return business, value

    def test_exact_carrier_fixture_source_and_digest_are_mandatory(self):
        business, value = self.carriers()
        module.validate_carriers(None, value, business, snapshot_label='phase25-tools-r4')
        for field in ('clusterId', 'deploymentId', 'fixtureId', 'baseDatabase', 'snapshot'):
            changed = copy.deepcopy(value)
            changed[field] = 'foreign'
            with self.subTest(field=field), self.assertRaises(RuntimeError):
                module.validate_carriers(None, changed, business, snapshot_label='phase25-tools-r4')
        changed = copy.deepcopy(value)
        changed['carriers']['stores']['value'] = 'changed-private-carrier'
        with self.assertRaises(RuntimeError):
            module.validate_carriers(None, changed, business)

    def test_infinite_deadline_or_extra_material_is_rejected(self):
        business, value = self.carriers()
        for field, invalid in [('expiresAtMs', float('nan')), ('expiresAtMs', float('inf')), ('sourceResponseSha256', 'not-a-digest')]:
            changed = copy.deepcopy(value)
            changed['carriers']['stores'][field] = invalid
            with self.subTest(field=field, invalid=invalid), self.assertRaises(RuntimeError):
                module.validate_carriers(None, changed, business)
        changed = copy.deepcopy(value)
        changed['carriers']['stores']['unexpected'] = True
        with self.assertRaises(RuntimeError):
            module.validate_carriers(None, changed, business)

    def test_early_cleanup_without_journal_or_handoff_is_idempotent_and_does_not_read_core(self):
        def forbidden():
            raise AssertionError('CORE_TASK_MUST_NOT_BE_LOADED')
        with TemporaryDirectory(prefix='phase25-recovery-cleanup-test-') as directory, patch.object(module, 'TASK', Path(directory)):
            first = module.cleanup(SimpleNamespace(Task=forbidden))
            second = module.cleanup(SimpleNamespace(Task=forbidden))
        self.assertEqual(first, second)
        self.assertEqual(first['result'], 'PASS')
        self.assertEqual(first['privateFilesRemoved'], 0)

    def test_unknown_journal_is_preserved_instead_of_adopted(self):
        with TemporaryDirectory(prefix='phase25-recovery-journal-test-') as directory, patch.object(module, 'TASK', Path(directory)):
            journal = Path(directory) / module.JOURNAL
            journal.write_text('unrecognized protected record')
            fake = SimpleNamespace(cluster='1', json=lambda _path: {'contractVersion': 'foreign'})
            with self.assertRaisesRegex(RuntimeError, 'CLEANUP_JOURNAL_INVALID'):
                module.cleanup(None, fake)
            self.assertTrue(journal.exists())

    def test_pg_tools_use_direct_exec_setpriv_and_no_parallel_or_acl_bypass(self):
        task = SimpleNamespace(uid=1000, gid=1000)
        with patch.object(module.subprocess, 'run', return_value=SimpleNamespace(returncode=0, stdout=b'ok')) as run:
            self.assertEqual(module.utility(task, ['/usr/lib/postgresql/18/bin/pg_dump', '--format=custom', 'owned']), b'ok')
        command = run.call_args.args[0]
        self.assertEqual(command[:5], ['/usr/bin/setpriv', '--reuid=1000', '--regid=1000', '--init-groups', '/usr/lib/postgresql/18/bin/pg_dump'])
        self.assertNotIn('runuser', command)
        self.assertEqual(run.call_args.kwargs['timeout'], 45)
        for option in ('--no-owner', '--no-acl', '--jobs=2', '--role=other'):
            with self.subTest(option=option), self.assertRaises(RuntimeError):
                module.utility(task, ['/usr/lib/postgresql/18/bin/pg_restore', option])

    def test_dangling_backup_or_log_link_is_preserved_and_blocks_cleanup(self):
        import json
        for kind in ('directory', 'log'):
            with self.subTest(kind=kind), TemporaryDirectory(prefix='phase25-dangling-guard-') as directory, patch.object(module, 'TASK', Path(directory)):
                root = Path(directory)
                fixture = '019b0000-0000-7000-8000-000000000001'
                deployment = '019b0000-0000-7000-8000-000000000002'
                token = fixture.replace('-', '')
                base_database = 'unified_auth_https_' + 'a' * 32
                journal = dict(contractVersion=module.CONTRACT, clusterId='1', fixtureId=fixture,
                    deploymentId=deployment, fixtureToken=token, baseDatabase=base_database, snapshot='phase25-tools-r4',
                    directory=str(root / ('phase25-business-recovery-' + token)), processes={}, pendingSpawns={}, targets={})
                for name in module.KINDS:
                    journal['targets'][name] = dict(database='unified_auth_p25restore_' + name + '_' + token,
                        sourceDatabase='unified_auth_phase25_' + name + '_' + 'a' * 32,
                        owner='uah25_' + 'a' * 12 + '_' + name + '_owner',
                        sourceMarker='unified-auth-phase25-business/1/' + deployment + '/' + name,
                        marker='unified-auth-phase25-business-recovery/1/' + fixture + '/' + name)
                (root / module.JOURNAL).write_text(json.dumps(journal))
                link = Path(journal['directory']) if kind == 'directory' else root / 'phase25-business-recovery-stores.log'
                try:
                    link.symlink_to(root / 'nonexistent-target', target_is_directory=kind == 'directory')
                except OSError:
                    self.skipTest('Native symlink creation requires the Linux test invocation')
                class Fake:
                    cluster = '1'
                    uid = 1000
                    def json(self, path):
                        return json.loads(path.read_text())
                    def pg(self, sql):
                        if sql.startswith('SELECT EXISTS'):
                            return 'f'
                        if sql.startswith('SELECT count(*)'):
                            return '0'
                        raise AssertionError('UNEXPECTED_DATABASE_WRITE')
                    def stop_pending_spawns(self):
                        pass
                    def stop_process(self, key):
                        pass
                    def checked(self, path, **_options):
                        if path.is_symlink():
                            raise RuntimeError('PROTECTED_SYMLINK_REJECTED')
                        return path
                report = module.cleanup(SimpleNamespace(Task=Fake), Fake())
                self.assertEqual(report['result'], 'FAIL')
                self.assertTrue(link.is_symlink())
                self.assertTrue((root / module.JOURNAL).exists())


if __name__ == '__main__':
    unittest.main()
