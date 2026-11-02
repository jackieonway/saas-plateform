"""Owned actual recovery probe without browser, HBA, tokens or proxy changes.

Reuses the bound task PG and app attestation, creates one environment, then
always runs both cleanup stages. Component evidence only; this never claims
the full HTTPS old-carrier / key lifecycle acceptance matrix passed.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import pwd
import re
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import phase23_cluster_binding as cluster_binding

TASK = Path('/tmp/rust-saas-unified-auth-20261028')
WORKSPACE = Path('/mnt/d/WorkSpace/rust/rust-saas')
OPS_RELATIVE = Path('saas-plateform/backend/ops/identity-recovery')

PRIVATE_CALL = r'''
import json,pathlib,sys
ops=pathlib.Path(sys.argv[1]); root=pathlib.Path(sys.argv[2]); operation=sys.argv[3]
sys.path.insert(0,str(ops))
stage='load-config'
try:
 import core
 config=core.load_config(root/'phase23-recovery-config.json')
 stage=operation
 if operation=='catalog-privilege-negative':
  material=core.keys(config); saved=core.checkpoint(config,material)
  core.guard(config,True); core.assert_catalogs(config,saved)
  database=config['databases']['platform']['target']
  prefix=config['databases']['platform']['owner'].removesuffix('_migration_owner')
  # Keep this negative mutation orthogonal to every original users column ACL
  # by selecting the independent portal runtime, which has none on this table.
  target=core.Pg(config,database); runtime=core.identifier(prefix+'_portal_runtime')
  owner=core.identifier(prefix+'_identity_owner')
  try:
   target.query('SET ROLE '+owner+'; GRANT SELECT ON public.users TO '+runtime+'; RESET ROLE;')
   rejected=False
   try: core.assert_catalogs(config,saved)
   except core.RecoveryError: rejected=True
   core.need(rejected,'PROBE_REAL_ACL_DRIFT_NOT_REJECTED')
  finally:
   target.query('SET ROLE '+owner+'; REVOKE SELECT ON public.users FROM '+runtime+'; RESET ROLE;')
  core.assert_catalogs(config,saved)
  print(json.dumps({'status':'PASS','stage':stage,'effectiveDefaultAclAccepted':True,
                    'realExtraTablePrivilegeRejected':True,'originalEffectiveAclRestored':True}))
  raise SystemExit(0)
 if operation=='catalog-contract':
  import inventory
  differences=[]
  for kind,database in config['databases'].items():
   raw=core.Pg(config,database['source']).json(inventory.capture_sql(kind))
   for table,spec in inventory.TABLES[kind].items():
    expected={field.source:{'type':field.pg_type,'nullable':field.nullable} for field in spec.fields.values()}
    actual=raw['catalog'][table]
    for column in sorted(set(expected)|set(actual['columns'])):
     if expected.get(column)!=actual['columns'].get(column):
      differences.append({'databaseKind':kind,'table':table,'column':column,
                          'expected':expected.get(column),'actual':actual['columns'].get(column)})
    if actual['pk']!=list(spec.pk):
     differences.append({'databaseKind':kind,'table':table,'expectedPk':list(spec.pk),'actualPk':actual['pk']})
  print(json.dumps({'status':'PASS','stage':stage,'catalogDifferences':differences}))
  raise SystemExit(0)
 if operation=='init': result=core.initialize(config)
 else:
  material=core.keys(config)
  if operation=='capture': result=core.capture(config,material)
  else: result={'capture-deny':core.capture_deny,'restore-new':core.restore_new,
                'reconcile':core.reconcile,'gate':core.gate}[operation](config,material,int(sys.argv[4]))
 safe={key:value for key,value in result.items() if key in ('status','sequence','databaseCount',
  'catalogsEqual','materialsEqual','oldCredentialsInvalidated','opensTraffic','archiveBytes')}
 print(json.dumps({'status':'PASS','stage':stage,'result':safe}))
except BaseException as error:
 if isinstance(error,SystemExit) and error.code==0: raise
 errors=[]; candidate=error
 for _ in range(4):
  if candidate is None: break
  frames=[]; current=candidate.__traceback__
  while current is not None:
   path=pathlib.Path(current.tb_frame.f_code.co_filename)
   if path.is_absolute() and path.is_relative_to(ops):
    frames.append({'module':str(path.relative_to(ops)),'function':current.tb_frame.f_code.co_name,'line':current.tb_lineno})
   current=current.tb_next
  errors.append({'exceptionClass':type(candidate).__name__,'frames':frames})
  candidate=candidate.__context__
 diagnostic={}
 if operation in ('restore-new','catalog-privilege-negative'):
  try:
   saved=core.checkpoint(config,core.keys(config)); differences=[]
   for kind,database in config['databases'].items():
    if core.Pg(config,'postgres').query('SELECT count(*) FROM pg_database WHERE datname='+core.literal(database['target']))=='0': continue
    actual=core.Pg(config,database['target']).json(core.CATALOG_SQL)
    for group,expected in saved['catalogs'][kind].items():
     observed=actual[group]
     if observed==expected: continue
     count=2 if group in ('columns','constraints','indexes','triggers') else 3 if group=='default_acl' else 1
     before={core.canonical(row[:count]):row for row in expected or []}
     after={core.canonical(row[:count]):row for row in observed or []}
     for key in sorted(set(before)|set(after)):
      if before.get(key)==after.get(key): continue
      entry={'databaseKind':kind,'group':group,'object':core.decode(key),
             'expectedSha256':core.sha(core.canonical(before.get(key))),
             'actualSha256':core.sha(core.canonical(after.get(key)))}
      if group in ('schemas','relations','default_acl'):
       entry.update(expected=before.get(key),actual=after.get(key))
      differences.append(entry)
   diagnostic['catalogDifferences']=differences
  except BaseException as extra:
   diagnostic['diagnosticExceptionClass']=type(extra).__name__
 print(json.dumps({'status':'FAIL','stage':stage,'errors':errors,**diagnostic}))
 raise SystemExit(2)
'''


def invoke(argv):
    result = subprocess.run(list(map(str, argv)), stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=240)
    try:
        value = json.loads(result.stdout)
    except (ValueError, UnicodeDecodeError):
        value = {'status': 'FAIL', 'code': 'UNSTRUCTURED_CHILD_FAILURE'}
    if result.returncode != 0:
        return False, value
    return True, value


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--snapshot', required=True)
    parser.add_argument('--build-snapshot', required=True)
    parser.add_argument('--workspace-ops', action='store_true', help='Explicitly probe current repair inputs, not frozen ops')
    parser.add_argument('--through-restore', action='store_true')
    parser.add_argument('--file-negatives', action='store_true',
                        help='Run real authenticated-file negatives with read-only PG preflight before any restore')
    args = parser.parse_args(argv)
    assert os.getuid() == 0
    assert re.fullmatch(r'phase23-tools-r[1-9][0-9]*', args.snapshot)
    assert re.fullmatch(r'phase23-apps-r[1-9][0-9]*', args.build_snapshot)
    binding = cluster_binding.load_binding(TASK)
    snapshot = TASK / ('source-snapshot-' + args.snapshot)
    tools = snapshot / 'saas-plateform/backend/tests/unified-auth-https'
    ops = (WORKSPACE if args.workspace_ops else snapshot) / OPS_RELATIVE
    helper = tools / 'phase23-environment.py'
    owner = pwd.getpwuid(binding['taskUid']).pw_name
    report = {'contractVersion': 'phase23-actual-recovery-probe/1', 'result': 'FAIL',
        'validationLayer': 'ACTUAL_OWNED_POSTGRES_COMPONENT_PROBE_NO_BROWSER_OR_OLD_CARRIERS',
        'snapshot': args.snapshot, 'buildSnapshot': args.build_snapshot,
        'opsInputs': 'WORKSPACE_REPAIR' if args.workspace_ops else 'FROZEN_SNAPSHOT',
        'opsSha256': {path.name: hashlib.sha256(path.read_bytes()).hexdigest()
                       for path in ops.glob('*.py') if not path.name.startswith('test_')},
        'containersCreated': 0, 'containersReused': 0, 'hbaChanged': False, 'steps': [], 'cleanup': []}
    def step(name, command, report_key=None):
        passed, value = invoke(command)
        report['steps'].append({'step': name, 'passed': passed, 'result': value})
        if report_key is not None:
            report[report_key] = value
        if not passed:
            raise RuntimeError('PROBE_CHILD_FAILED')
        return value
    def environment(action, *more):
        return [sys.executable, '-B', helper, action, '--task-dir', TASK, *more]
    def operation(action, sequence=0):
        return ['runuser', '-u', owner, '--', sys.executable, '-B', '-c', PRIVATE_CALL,
                ops, TASK, action, str(sequence)]
    def file_negative(stage, sequence):
        return ['runuser', '-u', owner, '--', sys.executable, '-B', ops / 'negative_probe.py',
                '--stage', stage, '--config', TASK / 'phase23-recovery-config.json',
                '--minimum-sequence', str(sequence)]
    try:
        step('prepare', environment('prepare', '--snapshot', args.snapshot, '--build-snapshot', args.build_snapshot))
        step('start', environment('start'))
        step('stop-writers', environment('stop-writers'))
        step('adapter-prepare', ['runuser', '-u', owner, '--', sys.executable, '-B', tools / 'phase23-recovery-files.py',
            'prepare', '--task-dir', TASK, '--environment', TASK / 'phase23-source-environment.json',
            '--manifest', TASK / 'https-database.json', '--output-config', TASK / 'phase23-recovery-config.json',
            '--tls-materials', TASK / 'phase23-tls-materials.json'])
        step('init', operation('init'))
        step('catalog-contract', operation('catalog-contract'))
        checkpoint = step('capture', operation('capture'))
        if args.file_negatives or args.through_restore:
            sequence = checkpoint['result']['sequence']
            if args.file_negatives:
                step('file-negative-save-baseline', file_negative('save-baseline', sequence))
            deny = step('capture-deny', operation('capture-deny', sequence))
            sequence = deny['result']['sequence']
            if args.file_negatives:
                step('file-negative-probe', file_negative('probe', sequence), 'recoveryNegatives')
        if args.through_restore:
            step('quarantine-source', environment('quarantine-source'))
            for action in ('restore-new', 'catalog-privilege-negative', 'reconcile', 'gate'):
                step(action, operation(action, sequence))
        report['result'] = 'PASS'
    except BaseException as error:
        report['exceptionClass'] = type(error).__name__
    finally:
        for action, extra in (('cleanup', ()), ('cleanup', ('--hba-restored',))):
            try:
                passed, value = invoke(environment(action, *extra))
                report['cleanup'].append({'passed': passed, 'result': value})
                # A just-restored task DB may briefly host an autovacuum worker.
                # Reuse the exact ownership-guarded cleanup; never terminate the
                # shared server or relax its zero-connection ownership fence.
                if extra and not passed and value.get('processesRemaining') == 0:
                    for _ in range(3):
                        time.sleep(1)
                        passed, value = invoke(environment(action, *extra))
                        report['cleanup'].append({'passed': passed, 'result': value})
                        if passed:
                            break
                if not passed:
                    report['result'] = 'FAIL'
            except BaseException as error:
                report['cleanup'].append({'passed': False, 'exceptionClass': type(error).__name__})
                report['result'] = 'FAIL'
    print(json.dumps(report, sort_keys=True))
    return 0 if report['result'] == 'PASS' else 2


if __name__ == '__main__':
    raise SystemExit(main())
