"""Narrow task-owner mutations for the current Phase25 resource browser.

Only the pre-existing owned employee membership, owned stores application and
its exact read grant can be changed. No import-time I/O, environment writes,
new DB, role, process, container or DDL. Secrets are returned only to the parent
via its captured pipe in preflight; never print this command's raw output.
"""
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import sys

TASK = Path('/tmp/rust-saas-unified-auth-20261028')
PREFIX = 'saas-plateform/backend/tests/unified-auth-https/'
ACTIONS = frozenset(('preflight', 'remove-employee-store-read', 'restore-employee-store-read',
    'disable-stores', 'enable-stores', 'disable-employee', 'enable-employee', 'inspect',
    'capture-recovery', 'clear-recovery-capture'))
RECOVERY_CARRIERS = 'phase25-business-recovery-carriers.json'


def need(condition, code):
    if not condition:
        raise RuntimeError('PHASE25_RESOURCE_' + code)


def literal(value):
    return "'" + str(value).replace("'", "''") + "'"


def valid_grant(row, ids, business):
    need(type(row) is dict and set(row) == {'id', 'membership_id', 'application_id', 'resource_code',
         'action_code', 'scope_type', 'created_at', 'updated_at'}, 'GRANT_RESTORE_FIELDS_INVALID')
    expected = {'id': business['ids']['stores']['grants'][7], 'membership_id': ids['employeeMember'],
        'application_id': business['ids']['stores']['application'], 'resource_code': 'store',
        'action_code': 'read', 'scope_type': 'ORG_SUBTREE'}
    need(all(row.get(key) == value for key, value in expected.items()), 'GRANT_RESTORE_BINDING_INVALID')
    need(all(type(row[key]) is str and re.fullmatch(r'[0-9T: .+\-Z]{10,50}', row[key])
         for key in ('created_at', 'updated_at')), 'GRANT_RESTORE_TIME_INVALID')
    return row


def change_sql(action, ids, business, grant=None):
    """Pure command planner with exact fixture IDs, never a caller SQL port."""
    need(action in ACTIONS and action not in ('preflight', 'capture-recovery', 'clear-recovery-capture'), 'ACTION_INVALID')
    member, app = literal(ids['employeeMember']), literal(business['ids']['stores']['application'])
    grant_id = literal(business['ids']['stores']['grants'][7])
    base_guard = ("DO $binding$ BEGIN IF (SELECT count(*) FROM memberships WHERE id=" + member
        + ' AND user_id=' + literal(ids['employee']) + ' AND organization_id=' + literal(ids['organization'])
        + " AND member_type='EMPLOYEE')<>1 OR (SELECT count(*) FROM applications WHERE id=" + app
        + " AND code='stores')<>1 THEN RAISE EXCEPTION 'RESOURCE_BINDING_CHANGED'; END IF; END $binding$;")
    statements = []
    if action == 'remove-employee-store-read':
        statements.append('WITH removed AS (DELETE FROM membership_data_grants WHERE id=' + grant_id
            + ' AND membership_id=' + member + ' AND application_id=' + app
            + " AND resource_code='store' AND action_code='read' AND scope_type='ORG_SUBTREE' RETURNING *) "
            + 'SELECT row_to_json(removed)::text FROM removed')
    elif action == 'restore-employee-store-read':
        row = valid_grant(grant, ids, business)
        statements.append('INSERT INTO membership_data_grants SELECT * FROM json_populate_record('
            + 'NULL::membership_data_grants,' + literal(json.dumps(row)) + '::json)')
    elif action in ('disable-stores', 'enable-stores'):
        status = 'DISABLED' if action == 'disable-stores' else 'ENABLED'
        statements.append("UPDATE applications SET status=" + literal(status)
            + ',access_policy_version=access_policy_version+1 WHERE id=' + app + ' AND status<>' + literal(status))
    elif action in ('disable-employee', 'enable-employee'):
        status = 'DISABLED' if action == 'disable-employee' else 'ENABLED'
        statements.append('UPDATE memberships SET status=' + literal(status)
            + ',identity_version=identity_version+1 WHERE id=' + member + ' AND status<>' + literal(status))
    if action != 'remove-employee-store-read':
        statements.append("SELECT json_build_object('memberEnabled',member.status='ENABLED',"
            + "'applicationEnabled',app.status='ENABLED','memberGeneration',member.identity_version,"
            + "'applicationGeneration',app.access_policy_version,'dataGrantRestored',EXISTS(SELECT 1 FROM "
            + 'membership_data_grants WHERE id=' + grant_id + ' AND membership_id=' + member
            + ' AND application_id=' + app + " AND scope_type='ORG_SUBTREE'))::text FROM memberships member "
            + 'CROSS JOIN applications app WHERE member.id=' + member + ' AND app.id=' + app)
    return 'BEGIN;\n' + base_guard + '\n' + ';\n'.join(statements) + ';\nCOMMIT;'


def main():
    raw = sys.stdin.buffer.read(32769)
    need(0 < len(raw) <= 32768, 'CONTROL_BUDGET_INVALID')
    request = json.loads(raw)
    need(type(request) is dict and set(request) <= {'action', 'grant', 'carriers'} and request.get('action') in ACTIONS,
         'CONTROL_REQUEST_INVALID')
    spec = importlib.util.spec_from_file_location('phase25_resource_task', Path(__file__).with_name('phase23-environment.py'))
    base = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(base)
    task = base.Task(require_root=False)
    need(task.state and task.state['prepared'] and task.state['mode'] == 'SOURCE'
         and not task.state.get('sourceQuarantined')
         and re.fullmatch(r'phase25-tools-r[1-9][0-9]*', task.state['snapshot']), 'OWNED_SOURCE_REQUIRED')
    snapshot = TASK / ('source-snapshot-' + task.state['snapshot'])
    need(Path(__file__).resolve() == snapshot / PREFIX / 'phase25-resource-control.py', 'FROZEN_CONTROL_REQUIRED')
    manifest = task.json(TASK / ('snapshot-' + task.state['snapshot'] + '.json'))
    need(manifest['includeAllProjects'] and manifest['completeFrontendOutputs']
         and manifest['snapshotSha256'] == hashlib.sha256(json.dumps(manifest['inputs'], sort_keys=True).encode()).hexdigest(),
         'FULL_CANDIDATE_REQUIRED')
    names = ('phase25-resource-control.py', 'phase25-resource-browser.mjs', 'phase25-resource-cases.mjs',
             'phase25-business-http.mjs')
    for name in names:
        need(task.digest(snapshot / PREFIX / name) == manifest['inputs'][PREFIX + name], 'FROZEN_TOOL_CHANGED')
    environment = task.json(TASK / 'https-environment.json')
    task.environment_binding(environment, 'SOURCE')
    need(environment['ids'] == task.json(TASK / 'phase23-source-environment.json')['ids'], 'ORIGINAL_IDENTITIES_CHANGED')
    business = task.json(TASK / 'phase25-business-manifest.json')
    fixture_spec = importlib.util.spec_from_file_location('phase25_resource_fixture', Path(__file__).with_name('phase25-business-fixture.py'))
    fixture = importlib.util.module_from_spec(fixture_spec)
    fixture_spec.loader.exec_module(fixture)
    fixture.validate_manifest(task, business)
    need(business['prepared'] and business['toolsSnapshot'] == task.state['snapshot']
         and business['clusterId'] == task.cluster and business['deploymentId'] == environment['ids']['deployment']
         and business['baseDatabase'] == task.json(TASK / 'https-database.json')['database'], 'BUSINESS_BINDING_REQUIRED')
    task.guard_databases('SOURCE')
    ids = environment['ids']
    action = request['action']
    guard = ("DO $guard$ BEGIN IF (SELECT shobj_description(oid,'pg_database') FROM pg_database WHERE datname=current_database())<>"
        + literal(business['baseMarker']) + " THEN RAISE EXCEPTION 'RESOURCE_DATABASE_CHANGED'; END IF; END $guard$;\n")
    if action == 'clear-recovery-capture':
        path = TASK / RECOVERY_CARRIERS
        if path.exists():
            value = task.json(path)
            need(value.get('contractVersion') == 'unified-auth-phase25-business-recovery-carriers/1'
                 and value.get('deploymentId') == business['deploymentId'] and value.get('fixtureId') == business['fixtureId']
                 and value.get('snapshot') == task.state['snapshot'], 'RECOVERY_CAPTURE_BINDING_CHANGED')
            path.unlink()
        result = {'status': 'RECOVERY_CAPTURE_CLEARED', 'remainingCarrierFiles': 0}
    elif action == 'capture-recovery':
        path = TASK / RECOVERY_CARRIERS
        need(not path.exists() and not path.is_symlink(), 'RECOVERY_CAPTURE_ALREADY_EXISTS')
        supplied = request.get('carriers')
        need(type(supplied) is dict and set(supplied) == {'stores', 'orders'}, 'RECOVERY_CARRIERS_INVALID')
        carriers = {}
        for kind in ('stores', 'orders'):
            entry = supplied[kind]
            need(type(entry) is dict and set(entry) == {'value', 'sourceResponseSha256'}
                 and re.fullmatch(r'[0-9a-f]{64}', entry['sourceResponseSha256']), 'RECOVERY_SOURCE_RESPONSE_INVALID')
            token = entry['value']
            need(type(token) is str and 1 <= len(token) <= 16384 and all(33 <= ord(char) <= 126 for char in token),
                 'RECOVERY_CARRIER_INVALID')
            digest = hashlib.sha256(token.encode()).hexdigest()
            code = 'stores' if kind == 'stores' else 'work-orders'
            observed = task.pg(guard + "SELECT json_build_object('expiresAtMs',extract(epoch FROM expires_at)*1000,"
                "'remainingMs',extract(epoch FROM expires_at-clock_timestamp())*1000)::text FROM platform_app_capabilities "
                'WHERE token_hash=' + literal(digest) + ' AND user_id=' + literal(ids['employee'])
                + ' AND membership_id=' + literal(ids['employeeMember']) + ' AND application_id='
                + literal(business['ids'][kind]['application']) + ' AND audience=' + literal(code)
                + ' AND platform_app_capability_active(token_hash);', business['baseDatabase'])
            need(observed, 'RECOVERY_CARRIER_NOT_CURRENT')
            deadline = json.loads(observed)
            need(deadline['remainingMs'] >= 60000, 'RECOVERY_CARRIER_WINDOW_TOO_SHORT')
            carriers[kind] = {'value': token, 'sha256': digest, 'expiresAtMs': deadline['expiresAtMs'],
                'sourceResponseSha256': entry['sourceResponseSha256']}
        value = {'contractVersion': 'unified-auth-phase25-business-recovery-carriers/1',
            **{key: business[key] for key in ('clusterId', 'deploymentId', 'fixtureId', 'baseDatabase')},
            'snapshot': task.state['snapshot'], 'buildSnapshot': task.state['buildSnapshot'],
            'buildSnapshotSha256': business['snapshotSha256'], 'carriers': carriers}
        task.write(path, value)
        result = {'status': 'RECOVERY_CAPTURED', 'actualCurrentEmployeeCarriers': 2,
            'minimumRemainingMs': min(value['expiresAtMs'] for value in carriers.values())
                - float(task.pg('SELECT extract(epoch FROM clock_timestamp())*1000;')),
            'rawCarriersPersistedOnlyInOwned0600File': True}
    elif action == 'preflight':
        facts = json.loads(task.pg(guard + change_sql('inspect', ids, business), business['baseDatabase']))
        need(facts['memberEnabled'] and facts['applicationEnabled'] and facts['dataGrantRestored'], 'FRESH_RESOURCE_POLICY_REQUIRED')
        configurations = environment['phase21']
        secrets = {key: task.read(Path(configurations['rp'][key]['OIDC_CLIENT_SECRET_FILE'])).decode().strip()
            for key in ('third',)}
        secrets['machine'] = task.read(Path(configurations['machine']['secretFile'])).decode().strip()
        result = {'environment': environment, 'business': business, 'secrets': secrets, 'initialPolicy': facts,
            'sourceSnapshot': task.state['snapshot'], 'snapshotSha256': manifest['snapshotSha256'],
            'scriptSha256': {name: manifest['inputs'][PREFIX + name] for name in names}}
    else:
        result = json.loads(task.pg(guard + change_sql(action, ids, business, request.get('grant')), business['baseDatabase']))
    sys.stdout.write(json.dumps(result))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        label = str(error)
        sys.stderr.write(label if re.fullmatch(r'PHASE25_RESOURCE_[A-Z0-9_]+', label) else 'PHASE25_RESOURCE_CONTROL_FAILED')
        raise SystemExit(1) from None
