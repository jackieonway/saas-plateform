"""Pure dual-identity supervisor regression; no process, files, DB or sockets."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

loader = importlib.util.spec_from_file_location('phase23_environment_instances',
    Path(__file__).with_name('phase23-environment.py'))
environment = importlib.util.module_from_spec(loader)
loader.loader.exec_module(environment)


class InstanceTests(unittest.TestCase):
    def task(self):
        task = Mock(uid=1000)
        task.state = {'prepared': True, 'mode': 'SOURCE', 'sourceQuarantined': False,
                      'targetOpen': False, 'profiles': []}
        task.json.return_value = {'identity': {'OIDC_KEYSET_PATH': str(environment.TASK / 'keys.json')}}
        return task

    def test_writers_start_four_daemons_with_only_three_runtime_roles(self):
        task = self.task()
        proof = {'independentProcesses': 2, 'distinctPids': True}
        with patch.object(environment, 'start_service') as start, \
             patch.object(environment, 'identity_instances', return_value=proof):
            result = environment.start_writers(task, None)
        self.assertEqual([call.args[1] for call in start.call_args_list],
                         ['identity', 'identity-peer', 'platform', 'portal'])
        self.assertEqual(set(environment.SERVICES), {'identity', 'platform', 'portal'})
        self.assertEqual(result['daemonInstances'], 4)
        self.assertEqual(result['services'], 3)
        self.assertEqual(result['identityInstances'], proof)
        self.assertEqual([call.args for call in task.health.call_args_list], [
            ('https://auth.saas.test:24444', '/health/ready'),
            ('https://auth.saas.test', '/healthz'),
            ('https://console.saas.test', '/auth/config'),
            ('https://customer.saas.test', '/auth/config')])
        self.assertEqual(result['readiness']['identityPublicLiveness'], 200)

    def test_cli_only_reports_local_guard_codes_and_never_arbitrary_exception_text(self):
        with self.assertRaises(environment.EnvironmentFailure) as caught:
            environment.need(False, 'SERVICE_HEALTH_FAILED')
        self.assertEqual(environment.safe_failure_code(caught.exception), 'PHASE23_ENV_SERVICE_HEALTH_FAILED')
        for error in (RuntimeError('PHASE23_ENV_PRIVATE_CANARY'), ValueError('PRIVATE_CANARY'),
                      environment.EnvironmentFailure('BAD_CODE PRIVATE_CANARY'),
                      environment.EnvironmentFailure('BAD_CODE\nPRIVATE_CANARY')):
            self.assertEqual(environment.safe_failure_code(error), 'PHASE23_ENVIRONMENT_FAILED')

    def test_profile_restarts_both_identity_processes_and_records_proof(self):
        task = self.task()
        with patch.object(environment, 'start_service') as start, \
             patch.object(environment, 'identity_instances', return_value={'distinctPids': True}):
            result = environment.profile(task, SimpleNamespace(ttl_seconds=600))
        self.assertEqual([call.args[0] for call in task.stop_service.call_args_list],
                         ['identity', 'identity-peer'])
        self.assertEqual([call.args[1] for call in start.call_args_list], ['identity', 'identity-peer'])
        self.assertNotIn('OIDC_TOKEN_TTL_SECONDS', task.json.return_value['identity'])
        self.assertEqual(result['canonicalTokenMaxTtlSeconds'], 300)
        self.assertEqual(result['requestedProfileSeconds'], 600)
        self.assertTrue(result['identityInstances']['distinctPids'])
        self.assertEqual(task.state['profiles'][0]['requestedProfileSeconds'], 600)
        self.assertEqual(task.state['profiles'][0]['canonicalTokenMaxTtlSeconds'], 300)
        self.assertNotIn('ttlSeconds', result)

    def test_stop_writers_stops_peer_and_quarantine_refuses_surviving_peer(self):
        task = self.task()
        result = environment.stop_writers(task, None)
        self.assertEqual({call.args[0] for call in task.stop_service.call_args_list}, set(environment.DAEMONS))
        self.assertEqual(result['databaseConnections'], 0)
        task.service.side_effect = lambda name: object() if name == 'identity-peer' else None
        with self.assertRaisesRegex(RuntimeError, 'WRITERS_NOT_STOPPED'):
            environment.quarantine_source(task, None)
        task.pg.assert_not_called()

    def test_independent_pid_same_binary_and_configured_keyset_required(self):
        task = self.task()
        binary = str(environment.TASK / 'bin/saas-identity')
        keyset = task.json.return_value['identity']['OIDC_KEYSET_PATH']
        records = {name: {'pid': pid, 'binary': binary, 'namespace': 'mnt:[1]', 'startTime': '42',
            'keysetPath': keyset, 'configuredKeysetSha256': 'a' * 64}
            for name, pid in (('identity', 100), ('identity-peer', 101))}
        task.service.side_effect = lambda name: (None, records[name])
        task.digest.side_effect = lambda filename: 'a' * 64 if filename == keyset else 'b' * 64
        def readlink(path):
            return binary if str(path).endswith('/exe') else 'mnt:[1]'
        with patch.object(environment, 'start_time', return_value='42'), \
             patch.object(environment.os, 'readlink', side_effect=readlink), \
             patch.object(Path, 'stat', return_value=SimpleNamespace(st_uid=1000)):
            result = environment.identity_instances(task)
            self.assertEqual(result['independentProcesses'], 2)
            for field, value in (('pid', 100), ('binary', binary + '-foreign'),
                                 ('configuredKeysetSha256', 'c' * 64), ('keysetPath', '/tmp/foreign')):
                old = records['identity-peer'][field]
                records['identity-peer'][field] = value
                with self.subTest(field=field), self.assertRaisesRegex(RuntimeError, 'IDENTITY_INSTANCES_NOT_INDEPENDENT'):
                    environment.identity_instances(task)
                records['identity-peer'][field] = old


if __name__ == '__main__':
    unittest.main()
