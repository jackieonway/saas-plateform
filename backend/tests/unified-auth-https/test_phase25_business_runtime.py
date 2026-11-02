"""Ownership/diagnostic regressions; no service child, socket, DB or Docker."""
import ast
import base64
import contextlib
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import re
import tempfile
import uuid
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch


loader = importlib.util.spec_from_file_location('phase25_business_runtime',
    Path(__file__).with_name('phase25-business-runtime.py'))
runtime = importlib.util.module_from_spec(loader)
loader.loader.exec_module(runtime)


def fixture():
    token='0123456789abcdef0123456789abcdef'
    base_database='unified_auth_https_'+token
    role_prefix='uah25_'+token[-12:]
    fixture_id='019c0000-0000-7000-8000-000000000001'
    task = SimpleNamespace(state_path=Path('/owned/task/phase23-environment-state.json'),
        state={'mode': 'SOURCE', 'prepared': True, 'snapshot': 'phase25-tools-r2',
               'buildSnapshot': 'phase25-apps-r1', 'processes': {}}, cluster='12345', uid=1000, gid=1000)
    source = {'ids': {'deployment': 'owned-v7-id'},
        'platform': {'DATA_GRANT_POLICY': 'owned-policy'},
        'identity': {'CONTEXT_PLATFORM_CA_FILE': '/owned/task/ca.pem'}}
    resources = {'clusterId': '12345', 'deploymentId': 'owned-v7-id',
                 'source': {'database': base_database}}
    snapshot = {'includeAllProjects': True}
    attestation = {'snapshotLabel': 'phase25-apps-r1', 'snapshotSha256': 'a' * 64,
                   'binaries': {spec['binary']: 'b' * 64 for spec in runtime.SPECS.values()}}
    value = {'contractVersion': 'unified-auth-phase25-business-environment/1',
        'clusterId': '12345', 'deploymentId': 'owned-v7-id', 'baseDatabase': base_database,
        'snapshot': 'phase25-apps-r1', 'snapshotSha256': 'a' * 64,
        'platformOverrides': {'DATA_GRANT_POLICY': 'owned-policy'}, 'services': {}}
    manifest = {'contractVersion': 'unified-auth-phase25-business-manifest/1',
        'clusterId': '12345', 'deploymentId': 'owned-v7-id', 'baseDatabase': base_database,
        'snapshot': 'phase25-apps-r1', 'snapshotSha256': 'a' * 64, 'services': {},
        'prepared':True,'baseEnvironmentUpdated':True,'fixtureId':fixture_id,
        'rolePrefix':role_prefix,'baseRolePrefix':'uah_'+token[-12:],
        'baseMarker':'identity-deployment/1/owned-v7-id'}
    for name, spec in runtime.SPECS.items():
        database=base_database if name=='notification' else 'unified_auth_phase25_'+name+'_'+token
        role=role_prefix+'_'+name+'_runtime'
        environment = {spec['prefix'] + '_HOST': '127.0.0.1',
            spec['prefix'] + '_PORT': str(spec['port']),
            spec['databaseKey']: 'postgres://'+role+':private-canary@127.0.0.1:55494/'+database}
        if name == 'notification':
            environment.update({'PLATFORM_API_BASE_URL': 'https://console.saas.test:24444',
                'NOTIFICATION_ENV': 'test', 'NOTIFICATION_SOCKET_PATH': '/socket.io/',
                'NOTIFICATION_ALLOWED_ORIGINS':'https://console.saas.test',
                'NOTIFICATION_PLATFORM_CA_FILE': '/owned/task/ca.pem'})
        else:
            environment.update({'PLATFORM_API_ORIGIN': 'https://console.saas.test:24444',
                'PLATFORM_IDENTITY_ISSUER': 'https://auth.saas.test',
                'PLATFORM_AUTHORIZATION_MODE': 'combined' if name == 'stores' else 'canonical',
                spec['prefix']+'_ALLOWED_ORIGINS':'https://console.saas.test',
                'PLATFORM_CLIENT_ID':'phase25-'+name+'-'+fixture_id[-12:],
                'PLATFORM_CLIENT_SECRET':'c'*64,
                'PLATFORM_RESOURCE_SCOPE_BINDINGS': json.dumps({r+':'+a:spec['code']+':'+r+':'+a for r,a in spec['pairs']}),
                'PLATFORM_CA_CERT_FILE': '/owned/task/ca.pem'})
        value['services'][name] = {'binary': spec['binary'], 'host': '127.0.0.1',
            'port': spec['port'], 'healthPath': spec['healthPath'], 'environment': environment}
        manifest['services'][name] = {'binarySha256': 'b' * 64,'binary':spec['binary'],
            'host':'127.0.0.1','port':spec['port'],'healthPath':spec['healthPath'],
            'database':database,'runtime':role}
        if name!='notification':
            manifest['services'][name].update({'owner':role_prefix+'_'+name+'_owner',
                'marker':'unified-auth-phase25-business/1/owned-v7-id/'+name})
    raw = json.dumps(value).encode()
    manifest['environmentSha256'] = runtime.sha(raw)
    return task, value, manifest, raw, source, resources, snapshot, attestation


class InputTests(unittest.TestCase):
    def invoke(self, parts):
        return runtime.validate(*parts)

    def test_valid_plan_does_not_mutate_input_and_uses_disjoint_environment_copies(self):
        parts = fixture(); original = copy.deepcopy(parts[1:])
        with patch.dict(os.environ, {'LD_PRELOAD': 'parent-secret', 'HTTP_PROXY': 'parent-proxy'}):
            plan = self.invoke(parts)
        self.assertEqual(parts[1:], original)
        self.assertEqual(set(plan), {'stores', 'orders', 'notification'})
        plan['stores']['environment']['STORES_PORT'] = 'different'
        self.assertEqual(parts[1]['services']['stores']['environment']['STORES_PORT'], '23031')
        self.assertNotIn('HTTP_PROXY', plan['orders']['environment'])
        self.assertNotIn('LD_PRELOAD', plan['stores']['environment'])

    def test_current_tools_source_and_full_build_are_mandatory(self):
        for change in ({'mode': 'RESTORE'}, {'prepared': False}, {'snapshot': 'phase23-tools-r2'},
                       {'buildSnapshot': 'phase23-apps-r1'}):
            with self.subTest(change=change), self.assertRaises(runtime.RuntimeFailure):
                parts = fixture(); parts[0].state.update(change); self.invoke(parts)
        parts = fixture(); parts[6]['includeAllProjects'] = False
        with self.assertRaisesRegex(runtime.RuntimeFailure, 'FULL_CANDIDATE_BINDING_INVALID'):
            self.invoke(parts)

    def test_environment_bytes_are_exact(self):
        parts = list(fixture()); parts[3] += b' '
        with self.assertRaisesRegex(runtime.RuntimeFailure, 'ENVIRONMENT_DIGEST_INVALID'):
            self.invoke(parts)

    def test_contract_cluster_deployment_database_and_policy_mismatches_fail(self):
        cases = [('contractVersion', 'untrusted'), ('clusterId', '999'), ('deploymentId', 'different'),
                 ('baseDatabase', 'business-real'), ('snapshot', 'phase25-apps-different'),
                 ('snapshotSha256', 'f' * 64), ('platformOverrides', {'DATA_GRANT_POLICY': 'different'})]
        for key, value in cases:
            with self.subTest(key=key), self.assertRaises(runtime.RuntimeFailure):
                parts = fixture(); parts[1][key] = value; self.invoke(parts)

    def test_missing_extra_and_changed_service_mappings_fail(self):
        for name in runtime.SPECS:
            for key, value in [('binary', '../wrong'), ('port', True), ('port', 1234),
                               ('host', '0.0.0.0'), ('healthPath', '/unowned')]:
                with self.subTest(name=name, key=key, value=value), self.assertRaises(runtime.RuntimeFailure):
                    parts = fixture(); parts[1]['services'][name][key] = value; self.invoke(parts)
        parts = fixture();parts[1]['services']['extra'] = {}
        with self.assertRaisesRegex(runtime.RuntimeFailure, 'SERVICE_SET_INVALID'):self.invoke(parts)
        parts = fixture();del parts[2]['services']['orders']
        with self.assertRaisesRegex(runtime.RuntimeFailure, 'SERVICE_SET_INVALID'):self.invoke(parts)

    def test_attested_binaries_cannot_drift(self):
        for name in runtime.SPECS:
            parts=fixture();parts[2]['services'][name]['binarySha256']='f'*64
            with self.subTest(name=name),self.assertRaisesRegex(runtime.RuntimeFailure,'BINARY_ATTESTATION_INVALID'):
                self.invoke(parts)

    def test_ambient_injection_and_invalid_environment_are_rejected(self):
        for key,value in [('PATH','private-canary'),('LD_PRELOAD','private-canary'),
                          ('PYTHONPATH','private-canary'),('HTTP_PROXY','private-canary'),
                          ('NODE_OPTIONS','private-canary'),('bad-key','x'),('A',b'bytes'),('A','nul\0canary')]:
            parts=fixture();parts[1]['services']['stores']['environment'][key]=value
            with self.subTest(key=key),self.assertRaises(runtime.RuntimeFailure):self.invoke(parts)

    def test_database_scope_host_role_query_and_cross_service_values_are_bound(self):
        for url in ['postgres://uah25_stores:secret@remote:55494/unified_auth_phase25_stores',
                    'postgres://uah25_stores:secret@127.0.0.1:5432/unified_auth_phase25_stores',
                    'postgres://wrong:secret@127.0.0.1:55494/unified_auth_phase25_stores',
                    'postgres://uah25_stores:secret@127.0.0.1:55494/production',
                    'postgres://uah25_stores:secret@127.0.0.1:55494/unified_auth_phase25_stores?host=remote']:
            parts=fixture();parts[1]['services']['stores']['environment']['STORES_DATABASE_URL']=url
            with self.subTest(url=url),self.assertRaisesRegex(runtime.RuntimeFailure,'DATABASE_BINDING_INVALID'):
                self.invoke(parts)
        parts=fixture();parts[1]['services']['orders']['environment']['DATABASE_URL']='private-canary'
        with self.assertRaisesRegex(runtime.RuntimeFailure,'CROSS_SERVICE_DATABASE_REJECTED'):self.invoke(parts)

    def test_private_authority_ca_and_canonical_modes_are_exact(self):
        changes=[('stores','PLATFORM_API_ORIGIN','https://foreign.test'),
            ('orders','PLATFORM_AUTHORIZATION_MODE','legacy'),
            ('stores','PLATFORM_IDENTITY_ISSUER','https://console.saas.test'),
            ('notification','NOTIFICATION_SOCKET_PATH','/wrong/'),
            ('notification','NOTIFICATION_ENV','development'),
            ('notification','NOTIFICATION_PLATFORM_CA_FILE','/outside/private-canary')]
        for name,key,value in changes:
            parts=fixture();parts[1]['services'][name]['environment'][key]=value
            with self.subTest(name=name,key=key),self.assertRaises(runtime.RuntimeFailure):self.invoke(parts)

    def test_consistent_foreign_database_and_role_cannot_be_adopted(self):
        parts=fixture();owner=parts[2]['services']['stores'];owner['database']='existing_business';owner['runtime']='existing_owner'
        parts[1]['services']['stores']['environment']['STORES_DATABASE_URL']='postgres://existing_owner:private-canary@127.0.0.1:55494/existing_business'
        raw=json.dumps(parts[1]).encode();parts=list(parts);parts[3]=raw;parts[2]['environmentSha256']=runtime.sha(raw)
        with self.assertRaisesRegex(runtime.RuntimeFailure,'MANIFEST_SERVICE_SCOPE_INVALID'):self.invoke(parts)

    def test_incomplete_preparation_and_exact_client_resource_bindings_fail(self):
        for key in ('prepared','baseEnvironmentUpdated'):
            parts=fixture();parts[2][key]=False
            with self.subTest(key=key),self.assertRaisesRegex(runtime.RuntimeFailure,'PREPARATION_INCOMPLETE'):self.invoke(parts)
        for key,value in [('PLATFORM_CLIENT_ID','phase25-foreign'),('PLATFORM_RESOURCE_SCOPE_BINDINGS','{}'),
                          ('STORES_ALLOWED_ORIGINS','https://foreign.test')]:
            parts=fixture();parts[1]['services']['stores']['environment'][key]=value
            with self.subTest(key=key),self.assertRaises(runtime.RuntimeFailure):self.invoke(parts)

    def test_actual_fixture_plan_and_environment_match_runtime_contract_without_io(self):
        # Execute the actual pure planning functions with explicit local ports.
        # Importing prepare would unnecessarily require cryptography on Windows.
        path=Path(__file__).with_name('phase25-business-fixture.py')
        tree=ast.parse(path.read_text(encoding='utf-8-sig'))
        functions={'need','canonical_id','names_for','validate_manifest','plan','environment_for','ui_environment','sha'}
        selected=[node for node in tree.body if isinstance(node,ast.FunctionDef) and node.name in functions
            or isinstance(node,ast.Assign) and any(isinstance(t,ast.Name) and t.id in {'HOSTILE_NAME','SPECS'} for t in node.targets)]
        counter=[100]
        def new_id():
            counter[0]+=1
            return '019c0000-0000-7000-8000-'+f'{counter[0]:012x}'
        namespace={'json':json,'re':re,'uuid':uuid,'hashlib':hashlib,'new_id':new_id,
                   'literal':lambda text:"'"+text.replace("'","''")+"'"}
        exec(compile(ast.Module(body=selected,type_ignores=[]),str(path),'exec'),namespace)
        fixture_module=SimpleNamespace(**namespace)
        task,_,_,_,source,resources,snapshot,attestation=fixture()
        ids={'deployment':'019c0000-0000-7000-8000-000000000011',
             'organization':'019c0000-0000-7000-8000-000000000012',
             'customer':'019c0000-0000-7000-8000-000000000013',
             'secondOrganization':'019c0000-0000-7000-8000-000000000014'}
        source['ids']=ids
        source['platform']['DATA_GRANT_POLICY']='{"applications":[],"managementScopes":[]}'
        resources['deploymentId']=ids['deployment']
        base={'database':resources['source']['database'],
              'rolePrefix':'uah_'+resources['source']['database'][-12:]}
        attestation['binaries'].update({'stores-migrate':'c'*64,'work-orders-migrate':'d'*64})
        task.pg=Mock(return_value='Owned original organization')
        with patch.object(runtime.subprocess,'Popen') as popen,patch.object(runtime.subprocess,'run') as run:
            manifest=fixture_module.plan(task,base,source,attestation)
            credentials={name:{'databasePassword':'e'*64,'metrics':'f'*64,
                'serviceId':'phase25-'+name+'-'+manifest['fixtureId'][-12:],'serviceSecret':'a'*64}
                for name in runtime.SPECS}
            value=fixture_module.environment_for(manifest,source,credentials)
            source['platform']['DATA_GRANT_POLICY']=manifest['basePolicyApplied']
            manifest['prepared']=manifest['baseEnvironmentUpdated']=True
            raw=json.dumps(value).encode();manifest['environmentSha256']=runtime.sha(raw)
            plan=runtime.validate(task,value,manifest,raw,source,resources,snapshot,attestation)
            self.assertEqual(set(plan),set(runtime.SPECS))
            popen.assert_not_called();run.assert_not_called()
        task.pg.assert_called_once()

    def test_json_duplicate_and_malformed_values_never_echo_private_input(self):
        for raw in [b'{"secret":"private-canary","secret":"private-other"}',b'private-canary',b'\xff']:
            with self.subTest(raw=raw),self.assertRaises(runtime.RuntimeFailure) as error:runtime.decode(raw)
            self.assertNotIn('private',str(error.exception))
        self.assertEqual(runtime.safe_failure(OSError('private-canary')),'PHASE25_BUSINESS_FAILED')
        self.assertEqual(runtime.safe_failure(runtime.RuntimeFailure('FIXED')),'PHASE25_BUSINESS_FIXED')


class ProcessTests(unittest.TestCase):
    def setUp(self):
        self.task=fixture()[0]
        self.task.digest=Mock(return_value='b'*64)
        self.record={'contractVersion':'unified-auth-phase25-business-child/1','service':'stores',
            'pid':123456,'startTime':'777','uid':1000,'namespace':'mnt:[777]',
            'binary':str(self.task.state_path.parent/'bin/stores-api'),'binarySha256':'b'*64}
        self.stack=contextlib.ExitStack();self.addCleanup(self.stack.close)
        self.open=self.stack.enter_context(patch.object(runtime.os,'pidfd_open',return_value=99,create=True))
        self.stack.enter_context(patch.object(runtime.signal,'SIGKILL',9,create=True))
        self.send=self.stack.enter_context(patch.object(runtime.signal,'pidfd_send_signal',create=True))
        self.close=self.stack.enter_context(patch.object(runtime.os,'close'))
        self.ident=self.stack.enter_context(patch.object(runtime,'process_identity',return_value=dict(self.record)))
        self.ready=self.stack.enter_context(patch.object(runtime.select,'select',return_value=([99],[],[])))

    def test_stop_uses_original_pidfd_and_closes_it(self):
        runtime.stop_native(self.task,self.record,'stores')
        self.open.assert_called_once_with(123456)
        self.send.assert_called_once_with(99,signal.SIGTERM)
        self.close.assert_called_once_with(99)
        self.assertEqual(self.ident.call_count,2)

    def test_pid_reuse_before_pin_never_opens_or_signals(self):
        for field,value in [('startTime','888'),('namespace','mnt:[888]'),('binary','/unrelated'),('uid',0)]:
            self.ident.return_value={**self.record,field:value}
            with self.subTest(field=field),self.assertRaisesRegex(runtime.RuntimeFailure,'CHILD_IDENTITY_CHANGED'):
                runtime.stop_native(self.task,self.record,'stores')
        self.open.assert_not_called();self.send.assert_not_called()

    def test_pid_reuse_between_check_and_pin_never_signals_new_process(self):
        self.ident.side_effect=[self.record,{**self.record,'startTime':'888'}]
        with self.assertRaisesRegex(runtime.RuntimeFailure,'CHILD_IDENTITY_CHANGED'):
            runtime.stop_native(self.task,self.record,'stores')
        self.send.assert_not_called();self.close.assert_called_once_with(99)

    def test_exited_original_is_idempotent(self):
        self.ident.side_effect=FileNotFoundError
        runtime.stop_native(self.task,self.record,'stores')
        runtime.stop_native(self.task,self.record,'stores')
        self.open.assert_not_called();self.send.assert_not_called()

    def test_process_exit_during_pin_is_safe(self):
        self.open.side_effect=ProcessLookupError
        runtime.stop_native(self.task,self.record,'stores')
        self.send.assert_not_called();self.close.assert_not_called()

    def test_changed_binary_does_not_signal_or_adopt(self):
        self.task.digest.return_value='c'*64
        with self.assertRaisesRegex(runtime.RuntimeFailure,'CHILD_BINARY_CHANGED'):
            runtime.stop_native(self.task,self.record,'stores')
        self.send.assert_not_called();self.close.assert_called_once_with(99)

    def test_termination_is_bounded_then_kills_only_the_same_descriptor(self):
        self.ready.side_effect=[([],[],[]),([99],[],[])]
        runtime.stop_descriptor(99)
        self.assertEqual([call.args for call in self.send.call_args_list],[(99,signal.SIGTERM),(99,signal.SIGKILL)])
        self.assertEqual([call.args[-1] for call in self.ready.call_args_list],[1.5,.8])

    def test_cleanup_timeout_and_permission_errors_are_not_claimed_clean(self):
        self.ready.return_value=([],[],[])
        with self.assertRaisesRegex(runtime.RuntimeFailure,'CHILD_STOP_TIMEOUT'):runtime.stop_descriptor(99)
        self.send.side_effect=PermissionError('private-canary')
        with self.assertRaises(PermissionError):runtime.stop_descriptor(99)

    def test_esrch_original_descriptor_is_already_stopped(self):
        self.send.side_effect=ProcessLookupError
        runtime.stop_descriptor(99)
        self.ready.assert_not_called()


class SupervisorTests(unittest.TestCase):
    def setUp(self):
        parts=fixture();self.task=parts[0];self.files={}
        self.plan=runtime.validate(*parts)
        self.binding={'contractVersion':'unified-auth-phase25-business-processes/1',
            'environmentSha256':'d'*64,'manifestSha256':'e'*64}
        self.task.namespace=Mock(return_value={'namespace':'mnt:[777]'})
        self.task.open_log=Mock(return_value=77)
        self.task.digest=Mock(return_value='b'*64)
        self.task.read=Mock(side_effect=lambda path:json.dumps(self.files[Path(path)]).encode())
        self.task.write=Mock(side_effect=lambda path,value:self.files.__setitem__(Path(path),copy.deepcopy(value)))
        self.base=SimpleNamespace(spawn_signals=lambda **kw:contextlib.nullcontext(),start_time=Mock(return_value='888'))
        self.stack=contextlib.ExitStack();self.addCleanup(self.stack.close)
        self.namespace=self.stack.enter_context(patch.object(runtime,'supervisor_namespace',return_value='mnt:[777]'))
        self.stack.enter_context(patch.object(runtime,'configuration',return_value=(self.plan,self.binding)))
        self.stack.enter_context(patch.object(Path,'exists',lambda path:Path(path) in self.files or path.name=='namespace.pid'))
        self.stack.enter_context(patch.object(Path,'is_symlink',return_value=False))
        self.stack.enter_context(patch.object(Path,'unlink',lambda path:self.files.pop(Path(path),None)))
        self.stack.enter_context(patch.object(runtime.os,'getuid',return_value=1000,create=True))
        self.stack.enter_context(patch.object(runtime.os,'readlink',return_value='mnt:[777]'))
        self.stack.enter_context(patch.object(runtime.signal,'SIGHUP',1,create=True))
        self.signals=self.stack.enter_context(patch.object(runtime.signal,'signal'))
        self.stack.enter_context(patch.object(runtime.signal,'pidfd_send_signal',create=True))
        self.pin=self.stack.enter_context(patch.object(runtime.os,'pidfd_open',return_value=99,create=True))
        self.close=self.stack.enter_context(patch.object(runtime.os,'close'))
        self.child=Mock(pid=123456)
        self.popen=self.stack.enter_context(patch.object(runtime.subprocess,'Popen',return_value=self.child))
        self.ident=self.stack.enter_context(patch.object(runtime,'process_identity',return_value={
            'pid':123456,'startTime':'777','uid':1000,'namespace':'mnt:[777]',
            'binary':str(self.task.state_path.parent/'bin/stores-api')}))
        self.select=self.stack.enter_context(patch.object(runtime.select,'select',return_value=([99],[],[])))
        self.stop=self.stack.enter_context(patch.object(runtime,'stop_descriptor'))

    def test_child_exit_runs_pinned_cleanup_and_deletes_only_own_lease(self):
        with self.assertRaisesRegex(runtime.RuntimeFailure,'CHILD_EXITED'):
            runtime.supervise(self.task,self.base,'stores')
        self.assertEqual(self.files,{})
        self.stop.assert_called_once_with(99)
        self.child.wait.assert_called_once_with(timeout=.5)
        self.child.poll.assert_not_called()
        self.child.terminate.assert_not_called();self.child.kill.assert_not_called()
        self.assertEqual(self.popen.call_args.args[0],[str(self.task.state_path.parent/'bin/stores-api')])
        self.assertIs(self.popen.call_args.kwargs['shell'],False)
        self.assertNotIn('private-canary',' '.join(self.popen.call_args.args[0]))
        self.assertEqual(set(c.args[0] for c in self.close.call_args_list),{77,99})

    def test_popen_error_does_not_signal_any_unowned_process(self):
        self.popen.side_effect=OSError('private-canary')
        with self.assertRaises(OSError):runtime.supervise(self.task,self.base,'stores')
        self.pin.assert_not_called();self.stop.assert_not_called()
        self.close.assert_called_once_with(77)

    def test_signal_on_popen_handoff_still_pins_and_reaps_the_created_child(self):
        @contextlib.contextmanager
        def handoff(**kwargs):
            yield
            raise KeyboardInterrupt
        self.base.spawn_signals=handoff
        with self.assertRaises(KeyboardInterrupt):runtime.supervise(self.task,self.base,'stores')
        self.pin.assert_called_once_with(123456)
        self.stop.assert_called_once_with(99)
        self.child.wait.assert_called_once_with(timeout=.5)
        self.assertEqual(self.files,{})

    def test_namespace_mismatch_is_rejected_before_popen(self):
        self.namespace.return_value='mnt:[888]'
        with self.assertRaisesRegex(runtime.RuntimeFailure,'SUPERVISOR_NAMESPACE_INVALID'):
            runtime.supervise(self.task,self.base,'stores')
        self.popen.assert_not_called()

    def test_changed_child_identity_rolls_back_original_pinned_child_before_lease(self):
        self.ident.return_value['uid']=0
        with self.assertRaisesRegex(runtime.RuntimeFailure,'CHILD_START_BINDING_INVALID'):
            runtime.supervise(self.task,self.base,'stores')
        self.stop.assert_called_once_with(99)
        self.task.write.assert_not_called()
        self.assertEqual(self.files,{})

    def test_failed_lease_write_still_reaps_original_child(self):
        self.task.write.side_effect=OSError('private-canary')
        with self.assertRaises(OSError):runtime.supervise(self.task,self.base,'stores')
        self.stop.assert_called_once_with(99);self.child.wait.assert_called_once_with(timeout=.5)
        self.assertEqual(self.files,{})

    def test_failed_child_stop_preserves_private_lease_for_root_recovery(self):
        self.stop.side_effect=runtime.RuntimeFailure('CHILD_STOP_TIMEOUT')
        with self.assertRaisesRegex(runtime.RuntimeFailure,'CHILD_STOP_TIMEOUT'):
            runtime.supervise(self.task,self.base,'stores')
        self.assertIn(runtime.native_path(self.task,'stores'),self.files)
        self.child.wait.assert_not_called()
        self.assertEqual(set(c.args[0] for c in self.close.call_args_list),{77,99})

    def test_changed_lease_cannot_be_deleted_even_after_original_child_stops(self):
        def altered(*args):
            self.files[runtime.native_path(self.task,'stores')]={'foreign':'private-canary'}
            return ([99],[],[])
        self.select.side_effect=altered
        with self.assertRaisesRegex(runtime.RuntimeFailure,'LEASE_CHANGED'):
            runtime.supervise(self.task,self.base,'stores')
        self.stop.assert_called_once_with(99)
        self.assertEqual(self.files[runtime.native_path(self.task,'stores')],{'foreign':'private-canary'})


class SupervisorNamespaceTests(unittest.TestCase):
    def setUp(self):
        self.task=fixture()[0]
        self.record={'pid':123,'startTime':'777','namespace':'mnt:[777]'}
        self.task.json=Mock(side_effect=lambda *args,**kwargs:copy.deepcopy(self.record))
        self.task.read=Mock(return_value=b'mnt:[777]\n')
        self.task.namespace=Mock(side_effect=PermissionError('private-canary'))
        self.base=SimpleNamespace(start_time=Mock(return_value='777'))
        self.stack=contextlib.ExitStack();self.addCleanup(self.stack.close)
        self.stat=self.stack.enter_context(patch.object(Path,'stat',return_value=SimpleNamespace(st_uid=0)))
        self.cmdline=self.stack.enter_context(patch.object(Path,'read_bytes',return_value=
            b'bash\0/owned/task/environment-controller.sh\0inside\0'))
        self.link=self.stack.enter_context(patch.object(runtime.os,'readlink',side_effect=PermissionError('private-canary')))

    def test_owner_checks_root_lease_and_exact_argv_without_reading_root_namespace_link(self):
        self.assertEqual(runtime.supervisor_namespace(self.task,self.base),'mnt:[777]')
        self.task.namespace.assert_not_called();self.link.assert_not_called()
        self.base.start_time.assert_called_once_with(123)
        self.assertEqual(self.task.json.call_count,2)

    def test_reused_pid_nonroot_controller_foreign_argv_or_changed_lease_is_rejected(self):
        for kind in ('pid','uid','argv','lease'):
            with self.subTest(kind=kind):
                with contextlib.ExitStack() as local:
                    if kind=='pid':local.enter_context(patch.object(self.base,'start_time',return_value='888'))
                    if kind=='uid':local.enter_context(patch.object(Path,'stat',return_value=SimpleNamespace(st_uid=1000)))
                    if kind=='argv':local.enter_context(patch.object(Path,'read_bytes',return_value=b'bash\0/owned/task/environment-controller.sh-foreign\0'))
                    if kind=='lease':local.enter_context(patch.object(self.task,'json',side_effect=[self.record,{**self.record,'pid':124}]))
                    with self.assertRaisesRegex(runtime.RuntimeFailure,'SUPERVISOR_NAMESPACE_INVALID'):
                        runtime.supervisor_namespace(self.task,self.base)

    def test_missing_or_malformed_public_namespace_identity_has_only_fixed_diagnostics(self):
        for changes in ({'pid':True},{'pid':1},{'startTime':'bad'},{'namespace':'foreign'}, {'extra':'private-canary'}):
            with self.subTest(changes=changes),patch.object(self.task,'json',return_value={**self.record,**changes}),\
                    self.assertRaisesRegex(runtime.RuntimeFailure,'SUPERVISOR_NAMESPACE_INVALID'):
                runtime.supervisor_namespace(self.task,self.base)
        for raw in (b'mnt:[999]',b'\xff'):
            with self.subTest(raw=raw),patch.object(self.task,'read',return_value=raw),\
                    self.assertRaisesRegex(runtime.RuntimeFailure,'SUPERVISOR_NAMESPACE_INVALID'):
                runtime.supervisor_namespace(self.task,self.base)
        with patch.object(self.task,'read',side_effect=PermissionError('private-canary')),\
                self.assertRaises(runtime.RuntimeFailure) as error:
            runtime.supervisor_namespace(self.task,self.base)
        self.assertNotIn('private-canary',str(error.exception))


class SupervisorDiagnosticsTests(unittest.TestCase):
    def test_only_fixed_allowlisted_code_survives_parent_log_extraction(self):
        task=fixture()[0]
        task.read=Mock(return_value=b'raw-private-canary\n'+json.dumps({
            'status':'FAIL','failureCode':'PHASE25_BUSINESS_SUPERVISOR_NAMESPACE_INVALID'}).encode()+b'\n')
        result=runtime.failure_result(runtime.supervisor_exited(task,'stores'))
        self.assertEqual(result,{'status':'FAIL','failureCode':'PHASE25_BUSINESS_SUPERVISOR_EXITED',
            'supervisorFailureCode':'PHASE25_BUSINESS_SUPERVISOR_NAMESPACE_INVALID',
            'service':'stores','safeAppFailureCode':'PHASE25_APP_DIAGNOSTIC_UNAVAILABLE'})
        task.read.assert_any_call(task.state_path.parent/'phase23-business-stores.log',maximum=8192,empty_ok=True)
        self.assertNotIn('private-canary',json.dumps(result))

    def test_unknown_or_unreadable_output_remains_generic_and_does_not_escape(self):
        task=fixture()[0]
        for raw in (b'private-canary',b'{"status":"FAIL","failureCode":"PRIVATE_CANARY"}',
                    b'{"status":"FAIL","failureCode":[]}',
                    b'{"status":"FAIL","failureCode":"PHASE25_BUSINESS_FAILED","secret":"private-canary"}'):
            task.read=Mock(return_value=raw)
            with self.subTest(raw=raw):
                self.assertEqual(runtime.failure_result(runtime.supervisor_exited(task,'stores')),
                    {'status':'FAIL','failureCode':'PHASE25_BUSINESS_SUPERVISOR_EXITED',
                     'service':'stores','safeAppFailureCode':'PHASE25_APP_DIAGNOSTIC_UNAVAILABLE'})
        task.read=Mock(side_effect=PermissionError('private-canary'))
        self.assertEqual(runtime.failure_result(runtime.supervisor_exited(task,'stores')),
            {'status':'FAIL','failureCode':'PHASE25_BUSINESS_SUPERVISOR_EXITED',
             'service':'stores','safeAppFailureCode':'PHASE25_APP_DIAGNOSTIC_UNAVAILABLE'})


class AppFailureDiagnosticsTests(unittest.TestCase):
    def setUp(self):
        self.task=fixture()[0]
        self.unknown='PHASE25_APP_DIAGNOSTIC_UNAVAILABLE'

    def line(self,event='stores_service_failed',category='business database baseline verification failed',**fields):
        return json.dumps({'timestamp':'PUBLIC_TEST_TIMESTAMP','level':'ERROR',
            'fields':{'event':event,'category':category,**fields}}).encode()+b'\n'

    def read(self,raw,service='stores'):
        self.task.read=Mock(return_value=raw)
        return runtime.app_failure_code(self.task,service)

    def test_configuration_and_service_categories_produce_only_fixed_enum_codes(self):
        self.assertEqual(self.read(self.line()),'PHASE25_APP_DATABASE_BASELINE_VERIFICATION')
        self.assertEqual(self.read(self.line(event='stores_configuration_failed',
            category='PLATFORM_AUTHORIZATION_MODE must be combined')),'PHASE25_APP_CONFIG_AUTHORIZATION_MODE')
        self.assertEqual(self.read(self.line(event='work_orders_service_failed',
            category='business database connection unavailable'),'orders'),'PHASE25_APP_DATABASE_UNAVAILABLE')
        self.task.read.assert_called_once_with(self.task.state_path.parent/'phase25-business-orders.log',
            maximum=runtime.APP_LOG_MAX_BYTES,empty_ok=True)

    def test_allowlisted_strings_are_actual_current_production_error_literals(self):
        workspace=Path(__file__).resolve().parents[4]
        for service,repository in [('stores','saas-stores'),('orders','saas-work-orders')]:
            error=(workspace/repository/'backend/src/error.rs').read_text(encoding='utf-8-sig')
            main=(workspace/repository/'backend/src/main.rs').read_text(encoding='utf-8-sig')
            for event,categories in runtime.APP_FAILURE_CATEGORIES[service].items():
                self.assertIn('"'+event+'"',main)
                for category in categories:self.assertIn('"'+category+'"',error)

    def test_malicious_or_secret_categories_cannot_escape_or_match_by_substring(self):
        private='postgresql://private-worker:private-canary@private-host/private-db'
        for raw in [self.line(category=private),self.line(category='business database baseline verification failed '+private),
                    self.line(event='private-canary'),self.line(category=['business database baseline verification failed']),
                    self.line(secret=private),self.line(event='work_orders_service_failed'),
                    private.encode()+b'\n',b'{"fields":{"event":"stores_service_failed","category":"private-canary",'
                    b'"category":"business database baseline verification failed"}}\n']:
            with self.subTest(raw=raw):
                code=self.read(raw)
                self.assertEqual(code,self.unknown)
                self.assertNotIn('private',code)
        self.assertEqual(self.read(self.line(),'notification'),self.unknown)

    def test_bounded_complete_json_records_only_and_latest_matching_category(self):
        for raw in [self.line()[:-1],self.line()+b'X'*runtime.APP_LOG_MAX_BYTES,
                    self.line()+b'\xff\n'+b'X'*(runtime.APP_LOG_MAX_BYTES+1),
                    self.line()+b'X'*(runtime.APP_LOG_MAX_LINE+1)+b'\n']:
            with self.subTest(length=len(raw)):
                expected=self.unknown if len(raw)>runtime.APP_LOG_MAX_BYTES or raw==self.line()[:-1] else 'PHASE25_APP_DATABASE_BASELINE_VERIFICATION'
                self.assertEqual(self.read(raw),expected)
        long_record=self.line(category='business database baseline verification failed',padding='X'*runtime.APP_LOG_MAX_LINE)
        self.assertEqual(self.read(long_record),self.unknown)
        self.assertEqual(self.read(self.line()+self.line(category='HTTP listener unavailable')),'PHASE25_APP_HTTP_LISTENER')

    def test_changed_unreadable_or_oversized_protected_log_is_not_retried_or_exposed(self):
        for error in [RuntimeError('FILE_CHANGED private-canary'),PermissionError('private-canary'),
                      FileNotFoundError('private-canary'),RuntimeError('FILE_INVALID private-canary')]:
            self.task.read=Mock(side_effect=error)
            with self.subTest(error=type(error).__name__):
                self.assertEqual(runtime.app_failure_code(self.task,'stores'),self.unknown)
                self.task.read.assert_called_once()

    @unittest.skipUnless(os.name=='posix','native protected read uses POSIX owner checks')
    def test_actual_protected_reader_rejects_a_log_mutated_during_the_read(self):
        base=runtime.base_module()
        with tempfile.TemporaryDirectory(prefix='phase25-native-log-change-') as directory:
            root=Path(directory);root.chmod(0o700)
            filename=root/'phase25-business-stores.log'
            filename.write_bytes(self.line());filename.chmod(0o600)
            task=base.Task.__new__(base.Task)
            task.uid,task.gid=os.getuid(),os.getgid();task.state_path=root/'state.json'
            original=os.read;mutated=False
            def changing(fd,size):
                nonlocal mutated
                value=original(fd,size)
                if not mutated:
                    mutated=True
                    with filename.open('ab') as output:output.write(b'private-canary\n')
                return value
            with patch.object(base,'TASK',root),patch.object(base.os,'read',side_effect=changing):
                self.assertEqual(runtime.app_failure_code(task,'stores'),self.unknown)
            self.assertTrue(mutated)

    def test_output_enum_and_service_cannot_be_injected_through_exception_attributes(self):
        error=runtime.RuntimeFailure('SUPERVISOR_EXITED')
        error.failedService='postgresql://private-canary'
        error.safeAppFailureCode='PHASE25_APP_PRIVATE_CANARY'
        self.assertEqual(runtime.failure_result(error),{'status':'FAIL','failureCode':'PHASE25_BUSINESS_SUPERVISOR_EXITED'})
        error.failedService='orders'
        self.assertEqual(runtime.failure_result(error),{'status':'FAIL','failureCode':'PHASE25_BUSINESS_SUPERVISOR_EXITED','service':'orders'})


@unittest.skipUnless(os.name=='posix','native log cleanup uses POSIX owner checks')
class NativeLogCleanupTests(unittest.TestCase):
    def setUp(self):
        temporary=tempfile.TemporaryDirectory(prefix='phase25-native-log-cleanup-')
        self.addCleanup(temporary.cleanup)
        self.root=Path(temporary.name);self.root.chmod(0o700)
        self.base=runtime.base_module()
        patched=patch.object(self.base,'TASK',self.root);patched.start();self.addCleanup(patched.stop)
        self.task=self.base.Task.__new__(self.base.Task)
        self.task.uid,self.task.gid=os.getuid(),os.getgid()
        self.task.state_path=self.root/'state.json';self.task.state=None
        self.environment=fixture()[1]
        for name,spec in runtime.SPECS.items():
            values=self.environment['services'][name]['environment']
            values['METRICS_TOKEN' if name=='notification' else spec['prefix']+'_METRICS_TOKEN']='private-metrics-'+name
            if name!='notification':
                values['PLATFORM_CLIENT_ID']='public-'+name
                values['PLATFORM_CLIENT_SECRET']='private-platform-'+name
        self.environment['services']['stores']['environment']['STORES_DATABASE_URL']='postgres://worker:encoded%2Fpassword%40secret@127.0.0.1:55494/owned'

    def write(self,path,raw):
        path.write_bytes(raw);path.chmod(0o600)

    def prepare(self):
        self.write(self.root/runtime.ENVIRONMENT,json.dumps(self.environment).encode())
        for name in runtime.SPECS:
            raw=json.dumps({'fields':{'event':'stores_service_failed' if name=='stores' else 'safe_info',
                'category':'business database baseline verification failed'}}).encode()+b'\n'
            self.write(self.root/('phase25-business-'+name+'.log'),raw)

    def test_all_real_canaries_scanned_before_exact_logs_deleted_and_repeat_is_noop(self):
        self.prepare()
        unrelated=self.root/'phase25-business-unrelated.log';self.write(unrelated,b'UNRELATED_PRESERVE')
        result=runtime.stop(self.task);audit=result['nativeLogAudit']
        self.assertEqual(audit['nativeLogsFound'],3);self.assertEqual(audit['nativeLogsRemoved'],3)
        self.assertEqual(audit['candidateFindings'],0);self.assertTrue(audit['canaryScanComplete'])
        self.assertEqual(audit['canaryCoverage'],'ACTUAL_PROTECTED_BUSINESS_ENVIRONMENT')
        self.assertGreater(audit['privateCandidateCount'],0)
        self.assertEqual(unrelated.read_bytes(),b'UNRELATED_PRESERVE')
        self.assertTrue((self.root/runtime.ENVIRONMENT).exists())
        self.assertEqual(runtime.stop(self.task)['nativeLogAudit']['nativeLogsRemoved'],0)
        rendered=json.dumps(result)
        for values in (entry['environment'] for entry in self.environment['services'].values()):
            for key,value in values.items():
                if re.search(r'secret|password|token|database_url',key,re.I):self.assertNotIn(value,rendered)
        self.assertEqual(audit['logs'][0]['safeAppFailureCode'],'PHASE25_APP_DATABASE_BASELINE_VERIFICATION')

    def test_full_url_encoded_and_decoded_password_metrics_secret_and_basic_are_in_memory_only(self):
        self.prepare();values=self.environment['services']['stores']['environment']
        candidates=[values['STORES_DATABASE_URL'],'encoded%2Fpassword%40secret','encoded/password@secret',
            values['STORES_METRICS_TOKEN'],values['PLATFORM_CLIENT_SECRET'],
            base64.b64encode((values['PLATFORM_CLIENT_ID']+':'+values['PLATFORM_CLIENT_SECRET']).encode()).decode()]
        for value in candidates:
            self.prepare();self.write(self.root/'phase25-business-stores.log',value.encode()+b'\n')
            with self.subTest(candidateKind=candidates.index(value)),self.assertRaises(runtime.NativeLogCleanupFailure) as failure:
                runtime.stop(self.task)
            audit=failure.exception.audit
            self.assertGreater(audit['candidateFindings'],0);self.assertEqual(audit['nativeLogsRemoved'],0)
            self.assertEqual(len(list(self.root.glob('phase25-business-*.log'))),3)
            self.assertNotIn(value,json.dumps(runtime.failure_result(failure.exception)))
            self.assertTrue((self.root/runtime.ENVIRONMENT).exists())

    def test_changed_log_is_not_deleted_after_the_scan(self):
        self.prepare();self.task.digest=Mock(return_value='0'*64)
        with self.assertRaisesRegex(runtime.NativeLogCleanupFailure,'NATIVE_LOG_CHANGED') as failure:
            runtime.stop(self.task)
        self.assertEqual(failure.exception.audit['nativeLogsRemoved'],0)
        self.assertEqual(len(list(self.root.glob('phase25-business-*.log'))),3)

    def test_missing_environment_is_safe_noop_without_logs_and_refuses_unscanned_normal_delete(self):
        self.assertEqual(runtime.stop(self.task)['nativeLogAudit']['nativeLogsFound'],0)
        filename=self.root/'phase25-business-stores.log'
        self.write(filename,json.dumps({'fields':{'event':'stores_service_failed',
            'category':'business database baseline verification failed'}}).encode()+b'\n')
        with self.assertRaisesRegex(runtime.NativeLogCleanupFailure,'NATIVE_LOG_ENVIRONMENT_REQUIRED') as failure:
            runtime.stop(self.task)
        self.assertTrue(filename.exists());self.assertFalse(failure.exception.audit['canaryScanComplete'])
        self.assertIsNone(failure.exception.audit['candidateFindings'])
        legacy=runtime.stop(self.task,allow_unscanned_legacy=True)['nativeLogAudit']
        self.assertEqual(legacy['status'],'PASS_WITH_LIMITED_CANARY_COVERAGE')
        self.assertEqual(legacy['nativeLogsRemoved'],1);self.assertFalse(legacy['canaryScanComplete'])
        self.assertIsNone(legacy['candidateFindings']);self.assertFalse(filename.exists())

    def test_malformed_environment_or_unprotected_log_does_not_erase_any_native_log(self):
        self.prepare();environment=self.root/runtime.ENVIRONMENT
        self.write(environment,b'{"private":"private-canary"}')
        with self.assertRaisesRegex(runtime.NativeLogCleanupFailure,'NATIVE_LOG_ENVIRONMENT_INVALID'):
            runtime.stop(self.task)
        self.assertEqual(len(list(self.root.glob('phase25-business-*.log'))),3)
        self.prepare();filename=self.root/'phase25-business-stores.log';filename.chmod(0o644)
        with self.assertRaises(runtime.NativeLogCleanupFailure):runtime.stop(self.task)
        self.assertEqual(len(list(self.root.glob('phase25-business-*.log'))),3)

    def test_failed_process_cleanup_never_reaches_log_read_or_delete(self):
        self.prepare()
        with patch.object(runtime,'stop_one',side_effect=runtime.RuntimeFailure('SERVICE_CLEANUP_REQUIRED')),\
                patch.object(runtime,'cleanup_native_logs') as cleanup,\
                self.assertRaisesRegex(runtime.RuntimeFailure,'CLEANUP_REQUIRED'):
            runtime.stop(self.task)
        cleanup.assert_not_called()
        self.assertEqual(len(list(self.root.glob('phase25-business-*.log'))),3)


class ControllerTests(unittest.TestCase):
    def setUp(self):
        parts=fixture();self.task=parts[0];self.files={};self.events=[]
        self.plan=runtime.validate(*parts)
        self.binding={'contractVersion':'unified-auth-phase25-business-processes/1',
            'environmentSha256':'d'*64,'manifestSha256':'e'*64}
        self.task.tools=Mock(return_value=Path('/owned/task/source-snapshot-phase25-tools-r2/saas-plateform/backend/tests/unified-auth-https'))
        self.task.namespace=Mock(return_value={'pid':123,'namespace':'mnt:[777]'})
        self.task.ns_command=Mock(side_effect=lambda command:['nsenter','setpriv',*map(str,command)])
        self.task.alive=Mock(return_value=True)
        self.task.read=Mock(side_effect=lambda path:json.dumps(self.files[Path(path)]).encode())
        self.task.write=Mock(side_effect=lambda path,value:self.files.__setitem__(Path(path),copy.deepcopy(value)))
        self.task.spawn=Mock(side_effect=self.spawn)
        self.task.stop_process=Mock(side_effect=lambda key:self.task.state['processes'].pop(key,None))
        self.task.stop_spawn_group=Mock()
        self.task.save=Mock()
        self.stack=contextlib.ExitStack();self.addCleanup(self.stack.close)
        self.stack.enter_context(patch.object(Path,'exists',lambda path:Path(path) in self.files))
        self.stack.enter_context(patch.object(Path,'is_symlink',return_value=False))
        self.stack.enter_context(patch.object(Path,'unlink',lambda path:self.files.pop(Path(path),None)))
        self.stack.enter_context(patch.object(runtime,'configuration',return_value=(self.plan,self.binding)))
        self.ident=self.stack.enter_context(patch.object(runtime,'process_identity',side_effect=self.identity))
        self.native=self.stack.enter_context(patch.object(runtime,'stop_native'))
        self.wire=self.stack.enter_context(patch.object(runtime.subprocess,'run',return_value=SimpleNamespace(returncode=0,stdout=b'200')))

    def identity(self,pid):
        return next(v for k,v in self.files.items() if k.suffix=='.pid' and v['pid']==pid)

    def spawn(self,key,command,script,uid):
        name=key.removeprefix('business-');self.events.append(('spawn',name))
        self.task.state['processes'][key]={'pid':123456,'pgid':123456,'uid':uid,'script':str(script)}
        self.files[runtime.native_path(self.task,name)]={'pid':123456+len(self.events),
            'startTime':'777','uid':1000,'namespace':'mnt:[777]',
            'binary':str(self.task.state_path.parent/'bin'/runtime.SPECS[name]['binary']),
            'binarySha256':'b'*64,'environmentSha256':'d'*64,'manifestSha256':'e'*64}

    def test_start_spawns_three_fixed_owner_supervisors_and_private_health_only(self):
        result=runtime.start(self.task)
        self.assertEqual(result['services'],3)
        self.assertIn(self.task.state_path.parent/runtime.LEASE,self.files)
        self.assertEqual(self.task.spawn.call_count,3)
        for call,name in zip(self.task.spawn.call_args_list,runtime.SPECS):
            self.assertEqual(call.args[0],'business-'+name)
            self.assertIn('--service',call.args[1]);self.assertEqual(call.args[1][-1],name)
            self.assertEqual(call.args[3],1000)
            self.assertNotIn('private-canary',' '.join(map(str,call.args)))
        self.assertTrue(all('http://127.0.0.1:' in ' '.join(c.args[0]) for c in self.wire.call_args_list))

    def test_partial_start_rolls_back_first_second_and_third_spawn(self):
        for index in (1,2,3):
            self.files.clear();self.events.clear();self.task.state['processes'].clear()
            calls=0
            def failing(*args):
                nonlocal calls
                calls+=1
                if calls==index:raise OSError('private-canary')
                return self.spawn(*args)
            self.task.spawn.side_effect=failing
            with self.subTest(index=index),self.assertRaises(OSError):runtime.start(self.task)
            self.assertEqual(self.task.state['processes'],{})
            self.assertEqual(self.files,{})

    def test_changed_lease_binding_rolls_back_before_health(self):
        original=self.task.spawn.side_effect
        def corrupt(*args):
            original(*args)
            self.files[runtime.native_path(self.task,'stores')]['environmentSha256']='wrong'
        self.task.spawn.side_effect=corrupt
        with self.assertRaisesRegex(runtime.RuntimeFailure,'CHILD_RECORD_BINDING_INVALID'):runtime.start(self.task)
        self.wire.assert_not_called();self.assertEqual(self.files,{})

    def test_dead_supervisor_keeps_only_fixed_failure_code_and_rolls_back_group(self):
        log=self.task.state_path.parent/'phase23-business-stores.log'
        self.files[log]={'status':'FAIL','failureCode':'PHASE25_BUSINESS_SUPERVISOR_NAMESPACE_INVALID'}
        self.files[self.task.state_path.parent/'phase25-business-stores.log']={
            'fields':{'event':'stores_service_failed','category':'business database baseline verification failed'}}
        def dead(key,command,script,uid):
            self.task.state['processes'][key]={'pid':123456,'pgid':123456,'uid':uid,'script':str(script)}
        self.task.spawn.side_effect=dead
        self.task.alive.return_value=False
        self.task.read.side_effect=lambda path,**kwargs:json.dumps(self.files[Path(path)]).encode()+b'\n'
        with self.assertRaises(runtime.RuntimeFailure) as failure:
            runtime.start(self.task)
        self.assertEqual(runtime.failure_result(failure.exception),{'status':'FAIL',
            'failureCode':'PHASE25_BUSINESS_SUPERVISOR_EXITED',
            'supervisorFailureCode':'PHASE25_BUSINESS_SUPERVISOR_NAMESPACE_INVALID',
            'service':'stores','safeAppFailureCode':'PHASE25_APP_DATABASE_BASELINE_VERIFICATION'})
        self.assertEqual(self.task.state['processes'],{})
        self.task.stop_process.assert_called_once_with('business-stores')
        self.wire.assert_not_called()

    def test_stop_is_idempotent_and_uses_existing_group_controller(self):
        runtime.start(self.task);runtime.stop(self.task);runtime.stop(self.task)
        self.assertEqual(self.files,{})
        self.assertEqual(self.task.state['processes'],{})
        self.assertEqual([c.args[0] for c in self.task.stop_process.call_args_list],
                         ['business-notification','business-orders','business-stores'])

    def test_stale_supervisor_without_pgid_is_not_killed(self):
        self.task.state['processes']['business-stores']={'pid':123456,'uid':1000,'script':str(self.task.tools()/runtime.SCRIPT)}
        with self.assertRaisesRegex(runtime.RuntimeFailure,'CLEANUP_REQUIRED'):runtime.stop(self.task)
        self.task.stop_process.assert_not_called()

    def test_pending_only_business_group_is_cleaned_without_touching_other_groups(self):
        key='business-stores'
        record={'contractVersion':'phase23-spawn-ownership/1','key':key,'cleanupRequired':True,
            'uid':1000,'script':str(self.task.tools()/runtime.SCRIPT),'pid':123456,'pgid':123456,'startTime':'777'}
        unrelated={'key':'unrelated'}
        self.task.state['pendingSpawns']={key:record,'unrelated':unrelated}
        path=self.task.state_path.parent/('phase23-spawn-'+key+'.json');self.files[path]=record
        runtime.stop(self.task)
        self.task.stop_spawn_group.assert_called_once_with(record)
        self.assertEqual(self.task.state['pendingSpawns'],{'unrelated':unrelated})
        self.assertEqual(self.files,{})

    def test_failed_pending_cleanup_keeps_journal_and_never_claims_stopped(self):
        key='business-orders'
        record={'contractVersion':'phase23-spawn-ownership/1','key':key,'cleanupRequired':True,
            'uid':1000,'script':str(self.task.tools()/runtime.SCRIPT),'pid':123456,'pgid':123456,'startTime':'777'}
        path=self.task.state_path.parent/('phase23-spawn-'+key+'.json');self.files[path]=record
        self.task.stop_spawn_group.side_effect=PermissionError('private-canary')
        with self.assertRaisesRegex(runtime.RuntimeFailure,'CLEANUP_REQUIRED'):runtime.stop(self.task)
        self.assertEqual(self.files[path],record)

    def test_bad_native_lease_does_not_block_trusted_group_cleanup(self):
        key='business-stores';path=runtime.native_path(self.task,'stores')
        self.files[path]={'malformed':'private-canary'}
        self.task.state['processes'][key]={'pid':123456,'pgid':123456,'uid':1000,
            'script':str(self.task.tools()/runtime.SCRIPT)}
        original=self.task.read.side_effect
        def broken(candidate):
            if Path(candidate)==path:return b'private-canary truncated JSON'
            return original(candidate)
        self.task.read.side_effect=broken
        with self.assertRaisesRegex(runtime.RuntimeFailure,'CLEANUP_REQUIRED'):runtime.stop(self.task)
        self.task.stop_process.assert_called_once_with(key)
        self.native.assert_not_called()
        self.assertIn(path,self.files)
        self.assertNotIn(key,self.task.state['processes'])

    def test_changed_lease_is_retained_and_not_deleted(self):
        path=runtime.native_path(self.task,'stores');self.files[path]={'secret':'private-canary'}
        with self.assertRaisesRegex(runtime.RuntimeFailure,'LEASE_CHANGED'):
            runtime.remove_lease(self.task,path,{'secret':'different'})
        self.assertEqual(self.files[path],{'secret':'private-canary'})

    def test_no_duplicate_start_or_extra_service(self):
        runtime.start(self.task)
        with self.assertRaisesRegex(runtime.RuntimeFailure,'AGGREGATE_LEASE_EXISTS'):runtime.start(self.task)
        self.assertEqual(self.task.spawn.call_count,3)
        for name in ('work-orders','../stores','/bin/sh','extra'):
            with self.subTest(name=name),self.assertRaisesRegex(runtime.RuntimeFailure,'SERVICE_INVALID'):
                runtime.native_path(self.task,name)


if __name__=='__main__':
    unittest.main()
