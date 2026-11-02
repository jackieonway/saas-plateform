"""Pure ownership/foreign-target rejection tests; no DB, service or Docker."""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

HERE = Path(__file__).resolve().parent
loader = importlib.util.spec_from_file_location('phase25_context_control_test', HERE / 'phase25-context-control.py')
control = importlib.util.module_from_spec(loader)
loader.loader.exec_module(control)


def identifier(number):
    return '019d0000-0000-7000-8000-' + format(number, '012x')


class ContextControlTests(unittest.TestCase):
    def setUp(self):
        self.ids = {'deployment': identifier(1), 'employee': identifier(2),
            'tenant': identifier(3), 'secondOrganization': identifier(4), 'organization': identifier(24)}
        self.business = {'baseDatabase': 'unified_auth_https_' + 'a' * 32,
            'ids': {'orders': {'application': identifier(5), 'menus': [identifier(i) for i in range(6, 13)], 'role': identifier(25)}}}
        self.value = {'contractVersion': control.CONTRACT, 'clusterId': '123456789',
            'database': self.business['baseDatabase'], 'deploymentId': self.ids['deployment'],
            'toolsSnapshot': 'phase25-tools-r4', 'fixtureId': identifier(13),
            'member': identifier(14), 'role': identifier(15), 'grants': [identifier(i) for i in range(16, 22)],
            'user': self.ids['employee'], 'tenant': self.ids['tenant'],
            'organization': self.ids['secondOrganization'],
            'firstOrganization': self.ids['organization'], 'sourceRole': self.business['ids']['orders']['role'],
            'detailMenu': identifier(26),
            'application': self.business['ids']['orders']['application'],
            'menus': self.business['ids']['orders']['menus'], 'prepared': False, 'cleaned': False}

    def validate(self, value):
        return control.validate_manifest(value, self.ids, self.business, '123456789', 'phase25-tools-r4')

    def test_only_bound_cluster_database_deployment_and_snapshot_are_accepted(self):
        self.assertIs(self.validate(self.value), self.value)
        for field in ('clusterId', 'database', 'deploymentId', 'toolsSnapshot', 'contractVersion'):
            value = copy.deepcopy(self.value)
            value[field] = 'foreign-or-unapproved'
            with self.subTest(field=field), self.assertRaisesRegex(RuntimeError, 'MANIFEST_BINDING_INVALID'):
                self.validate(value)

    def test_scope_cannot_be_redirected_to_existing_admin_or_other_application(self):
        for field in ('user', 'tenant', 'organization', 'firstOrganization', 'sourceRole', 'application', 'menus'):
            value = copy.deepcopy(self.value)
            value[field] = [identifier(99)] if field == 'menus' else identifier(99)
            with self.subTest(field=field), self.assertRaisesRegex(RuntimeError, 'MANIFEST_SCOPE_INVALID'):
                self.validate(value)

    def test_unknown_or_missing_manifest_fields_reject_before_sql_planning(self):
        for value in ({**self.value, 'sql': 'DELETE FROM users'},
                      {key: item for key, item in self.value.items() if key != 'member'}):
            with self.assertRaisesRegex(RuntimeError, 'MANIFEST_FIELDS_INVALID'):
                self.validate(value)

    def test_only_unique_uuid7_owned_identifiers_can_be_cleaned(self):
        for field in ('fixtureId', 'member', 'role', 'detailMenu'):
            for invalid in (identifier(16), '019d0000-0000-4000-8000-000000000099', "x'); DELETE FROM users; --"):
                value = copy.deepcopy(self.value)
                value[field] = invalid
                with self.subTest(field=field, invalid=invalid), self.assertRaisesRegex(RuntimeError, 'MANIFEST_IDENTIFIERS_INVALID'):
                    self.validate(value)
        value = copy.deepcopy(self.value)
        value['grants'][-1] = value['grants'][0]
        with self.assertRaisesRegex(RuntimeError, 'MANIFEST_IDENTIFIERS_INVALID'):
            self.validate(value)

    def test_extra_or_missing_data_grant_cannot_expand_cleanup_scope(self):
        for grants in (self.value['grants'][:-1], self.value['grants'] + [identifier(23)]):
            value = {**self.value, 'grants': grants}
            with self.assertRaisesRegex(RuntimeError, 'MANIFEST_IDENTIFIERS_INVALID'):
                self.validate(value)

    def test_invalid_cleanup_states_are_rejected(self):
        for state in ({'prepared': 'true'}, {'cleaned': 1}, {'prepared': True, 'cleaned': True}):
            with self.subTest(state=state), self.assertRaisesRegex(RuntimeError, 'MANIFEST_STATE_INVALID'):
                self.validate({**self.value, **state})

    def test_unknown_action_has_no_task_or_database_side_effect(self):
        value = subprocess.run([sys.executable, '-B', str(HERE / 'phase25-context-control.py')],
            input=json.dumps({'action': 'DELETE FROM users'}), text=True, capture_output=True, timeout=10)
        self.assertEqual(value.returncode, 1)
        self.assertEqual(value.stdout, '')
        self.assertEqual(value.stderr, 'PHASE25_CONTEXT_REQUEST_INVALID')

    def test_client_sql_and_target_fields_are_not_an_accepted_control_port(self):
        value = subprocess.run([sys.executable, '-B', str(HERE / 'phase25-context-control.py')],
            input=json.dumps({'action': 'cleanup', 'database': 'existing_business', 'sql': 'DROP DATABASE existing_business'}),
            text=True, capture_output=True, timeout=10)
        self.assertEqual(value.returncode, 1)
        self.assertEqual(value.stdout, '')
        self.assertEqual(value.stderr, 'PHASE25_CONTEXT_REQUEST_INVALID')

    def test_committed_cleanup_removes_marker_before_next_deployment_in_same_task(self):
        with tempfile.TemporaryDirectory(prefix='phase25-context-marker-') as directory:
            root = Path(directory)
            marker = root / control.MARKER
            marker.write_text(json.dumps(self.value), encoding='utf-8')
            task = SimpleNamespace(checked=Mock(return_value=marker))
            with patch.object(control, 'TASK', root):
                control.retire_marker(task, marker)
            task.checked.assert_called_once_with(marker)
            self.assertFalse(marker.exists(), 'a completed prior deployment must not poison the next SOURCE fixture')
            # A new deployment can exclusively create its own marker instead
            # of failing validation against the previous database/deployment.
            with marker.open('x', encoding='utf-8') as output:
                json.dump({**self.value, 'deploymentId': identifier(99)}, output)
            self.assertEqual(json.loads(marker.read_text(encoding='utf-8'))['deploymentId'], identifier(99))

    def test_marker_retirement_cannot_delete_another_task_file(self):
        with tempfile.TemporaryDirectory(prefix='phase25-context-marker-') as directory:
            root = Path(directory)
            foreign = root / 'foreign-manifest.json'
            foreign.write_text('preserve', encoding='utf-8')
            task = SimpleNamespace(checked=Mock())
            with patch.object(control, 'TASK', root), self.assertRaisesRegex(RuntimeError, 'CLEANUP_MARKER_SCOPE_INVALID'):
                control.retire_marker(task, foreign)
            task.checked.assert_not_called()
            self.assertEqual(foreign.read_text(encoding='utf-8'), 'preserve')


if __name__ == '__main__':
    unittest.main()
