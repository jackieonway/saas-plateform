"""Pure narrow-mutation guard tests; no database or service runtime evidence."""
import copy
import importlib.util
from pathlib import Path
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('phase25_resource_control_test', HERE / 'phase25-resource-control.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ResourceControlTests(unittest.TestCase):
    def setUp(self):
        self.ids = {'employeeMember': '019b0000-0000-7000-8000-000000000001',
            'employee': '019b0000-0000-7000-8000-000000000002',
            'organization': '019b0000-0000-7000-8000-000000000003'}
        self.business = {'ids': {'stores': {'application': '019b0000-0000-7000-8000-000000000004',
            'grants': ['019b0000-0000-7000-8000-0000000000%02d' % index for index in range(14)]}}}
        self.grant = {'id': self.business['ids']['stores']['grants'][7],
            'membership_id': self.ids['employeeMember'],
            'application_id': self.business['ids']['stores']['application'], 'resource_code': 'store',
            'action_code': 'read', 'scope_type': 'ORG_SUBTREE',
            'created_at': '2026-10-29T10:00:00+00:00', 'updated_at': '2026-10-29T10:00:00+00:00'}

    def test_only_exact_fixture_read_grant_can_be_restored(self):
        self.assertEqual(module.valid_grant(self.grant, self.ids, self.business), self.grant)
        for key in ('id', 'membership_id', 'application_id', 'resource_code', 'action_code', 'scope_type'):
            value = copy.deepcopy(self.grant)
            value[key] = 'foreign-or-unapproved'
            with self.subTest(field=key), self.assertRaisesRegex(RuntimeError, 'GRANT_RESTORE_BINDING_INVALID'):
                module.valid_grant(value, self.ids, self.business)

    def test_unknown_fields_and_timestamp_sql_injection_are_rejected(self):
        value = {**self.grant, 'owner': 'unexpected'}
        with self.assertRaisesRegex(RuntimeError, 'GRANT_RESTORE_FIELDS_INVALID'):
            module.valid_grant(value, self.ids, self.business)
        value = {**self.grant, 'created_at': "2026-10-29'); DROP TABLE users; --"}
        with self.assertRaisesRegex(RuntimeError, 'GRANT_RESTORE_TIME_INVALID'):
            module.valid_grant(value, self.ids, self.business)

    def test_data_grant_remove_is_exactly_bounded_to_employee_app_action_and_scope(self):
        sql = module.change_sql('remove-employee-store-read', self.ids, self.business)
        for value in (self.grant['id'], self.ids['employeeMember'], self.ids['employee'],
                      self.ids['organization'], self.grant['application_id']):
            self.assertIn(value, sql)
        self.assertIn("resource_code='store' AND action_code='read' AND scope_type='ORG_SUBTREE'", sql)
        self.assertIn('RETURNING *', sql)
        self.assertNotIn('DELETE FROM memberships', sql)

    def test_reenable_uses_monotonic_generation_and_never_revives_capability_rows(self):
        for action, generation in [('enable-stores', 'access_policy_version'), ('enable-employee', 'identity_version')]:
            sql = module.change_sql(action, self.ids, self.business)
            self.assertIn(generation + '=' + generation + '+1', sql)
            self.assertNotIn('platform_app_capabilities', sql)
            self.assertNotIn('revoked=false', sql)

    def test_restore_preserves_captured_timestamps_with_typed_record_insertion(self):
        sql = module.change_sql('restore-employee-store-read', self.ids, self.business, self.grant)
        self.assertIn('json_populate_record(NULL::membership_data_grants,', sql)
        self.assertIn(self.grant['created_at'], sql)
        self.assertNotIn('ON CONFLICT', sql)

    def test_unknown_action_cannot_be_a_general_sql_port(self):
        for action in ('DROP DATABASE business', 'restore-admin', 'disable-orders', 'preflight'):
            with self.subTest(action=action), self.assertRaisesRegex(RuntimeError, 'ACTION_INVALID'):
                module.change_sql(action, self.ids, self.business)


if __name__ == '__main__':
    unittest.main()
