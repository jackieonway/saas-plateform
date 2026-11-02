"""Diagnostic redaction and strict transport contracts; no live service claim."""
import importlib.util
import json
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('phase25_business_diagnostic_test', HERE / 'phase25-business-diagnostic.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class DiagnosticTests(unittest.TestCase):
    def test_private_values_cannot_escape_response_shape(self):
        secret = 'PRIVATE_RESPONSE_CANARY'
        value = dict(contractVersion=secret, code=secret, message=secret, token=secret,
            context=dict(permissions=['store:read'], dataGrants=[dict(resource='store', action='read', private=secret)]),
            principal=dict(userId=secret), data=dict(records=[dict(name=secret)], contextVersion=secret))
        rendered = json.dumps(module.shape(503, value))
        self.assertNotIn(secret, rendered)
        self.assertEqual(module.shape(503, value)['fixedCode'], 'UNRECOGNIZED_OR_ABSENT')

    def test_contract_counts_and_version_match_are_explicit_booleans(self):
        value = dict(contractVersion='resource-directory/1', directory='organizations', data=dict(
            contractVersion='1.0', contextVersion='private-hash', page=1, size=50,
            records=[dict(organizationType='STORE', status='ENABLED')]))
        facts = module.shape(200, value, authorization_context_version='private-hash')
        self.assertTrue(facts['directoryContextVersionMatchesAuthorization'])
        self.assertTrue(facts['everyRecordEnabledStore'])
        self.assertEqual(facts['recordCount'], 1)
        self.assertNotIn('private-hash', json.dumps(facts))

    def test_transport_keeps_both_service_secret_and_original_carrier_off_argv(self):
        secret, token = 'PRIVATE_SERVICE_SECRET', 'PRIVATE_ORIGINAL_CARRIER'
        task = SimpleNamespace(ns_command=lambda command: ['owned-namespace', *command])
        with patch.object(module.subprocess, 'run', return_value=SimpleNamespace(returncode=0, stdout=b'{"code":"FORBIDDEN"}\n403')) as run:
            response = module.request(task, '/owned/ca.pem', '/api/apps/stores/authorize',
                form=dict(token=token, resource='store', action='read'), basic=('owned-service', secret))
        command = run.call_args.args[0]
        self.assertIn('--cacert', command)
        self.assertNotIn('-k', command)
        self.assertNotIn('--insecure', command)
        self.assertNotIn(secret, ' '.join(command))
        self.assertNotIn(token, ' '.join(command))
        self.assertIn(secret.encode(), run.call_args.kwargs['input'])
        self.assertIn(token.encode(), run.call_args.kwargs['input'])
        self.assertEqual(response['status'], 403)

    def test_transport_failure_does_not_return_stderr(self):
        task = SimpleNamespace(ns_command=lambda command: command)
        with patch.object(module.subprocess, 'run', return_value=SimpleNamespace(returncode=60, stdout=b'', stderr=b'PRIVATE_TLS_ERROR')):
            response = module.request(task, '/owned/ca.pem', '/stores-api/stores', bearer='PRIVATE_CARRIER')
        self.assertEqual(response, dict(status=0, value=None, transportFailed=True))


if __name__ == '__main__':
    unittest.main()
