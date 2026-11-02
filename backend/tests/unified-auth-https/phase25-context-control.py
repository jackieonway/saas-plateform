"""Exact owned SOURCE fixtures for the real Phase25 host context runner.

No import-time I/O or arbitrary SQL input. Only a new employee membership,
work-orders grants in the pre-existing second organization, one explicit
read-only detail-page MENU and their exact links are added. Existing rows/ENV
are never overwritten. Immutable
security and business facts remain until the whole owned database is removed.
"""
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import secrets
import sys
import time
import uuid

TASK = Path('/tmp/rust-saas-unified-auth-20261028')
PREFIX = 'saas-plateform/backend/tests/unified-auth-https/'
MARKER = 'phase25-context-manifest.json'
CONTRACT = 'unified-auth-phase25-context-fixture/1'
ACTIONS = frozenset(('prepare', 'inspect', 'cleanup'))
OPERATIONS = (('ticket', 'read'), ('ticket', 'export'), ('ticket', 'create'),
              ('ticket', 'update'), ('ticket', 'delete'), ('notification', 'publish'))
UUID7 = re.compile(r'[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}')


def need(value, code):
    if not value:
        raise RuntimeError('PHASE25_CONTEXT_' + code)


def literal(value):
    return "'" + str(value).replace("'", "''") + "'"


def new_id():
    value = ((int(time.time() * 1000) << 80) | (7 << 76)
             | (secrets.randbits(12) << 64) | (2 << 62) | secrets.randbits(62))
    return str(uuid.UUID(int=value))


def validate_manifest(value, ids, business, cluster, snapshot):
    need(type(value) is dict and set(value) == {'contractVersion', 'clusterId', 'database',
         'deploymentId', 'toolsSnapshot', 'fixtureId', 'member', 'role', 'grants',
         'user', 'tenant', 'organization', 'firstOrganization', 'sourceRole', 'detailMenu',
         'application', 'menus', 'prepared', 'cleaned'},
         'MANIFEST_FIELDS_INVALID')
    need(value['contractVersion'] == CONTRACT and value['clusterId'] == cluster
         and value['database'] == business['baseDatabase']
         and value['deploymentId'] == ids['deployment'] and value['toolsSnapshot'] == snapshot,
         'MANIFEST_BINDING_INVALID')
    expected = {'user': ids['employee'], 'tenant': ids['tenant'], 'organization': ids['secondOrganization'],
                'firstOrganization': ids['organization'], 'sourceRole': business['ids']['orders']['role'],
                'application': business['ids']['orders']['application'],
                'menus': business['ids']['orders']['menus']}
    need(all(value[key] == item for key, item in expected.items()), 'MANIFEST_SCOPE_INVALID')
    owned = [value['fixtureId'], value['member'], value['role'], value['detailMenu'], *value['grants']]
    need(len(value['grants']) == len(OPERATIONS) and len(owned) == len(set(owned))
         and all(type(item) is str and UUID7.fullmatch(item) for item in owned), 'MANIFEST_IDENTIFIERS_INVALID')
    need(all(type(value[key]) is bool for key in ('prepared', 'cleaned'))
         and not (value['prepared'] and value['cleaned']), 'MANIFEST_STATE_INVALID')
    return value


def binding_guard(value):
    return ("DO $binding$ BEGIN IF (SELECT shobj_description(oid,'pg_database') FROM pg_database WHERE datname=current_database()) IS DISTINCT FROM "
        + literal('identity-deployment/1/' + value['deploymentId'])
        + ' OR (SELECT count(*) FROM users WHERE id=' + literal(value['user']) + ")<>1"
        + ' OR (SELECT count(*) FROM organizations WHERE id=' + literal(value['organization'])
        + ' AND tenant_id=' + literal(value['tenant']) + ")<>1"
        + ' OR (SELECT count(*) FROM applications WHERE id=' + literal(value['application'])
        + " AND code='work-orders' AND entry_url='https://console.saas.test/sub/work-orders/index.html')<>1"
        + ' OR (SELECT count(*) FROM roles WHERE id=' + literal(value['sourceRole'])
        + ' AND tenant_id=' + literal(value['tenant']) + ' AND organization_id=' + literal(value['firstOrganization'])
        + " AND code='phase25-orders')<>1"
        + " THEN RAISE EXCEPTION 'CONTEXT_FIXTURE_BINDING'; END IF; END $binding$;")


def count_sql(value):
    member, role, org, app = (literal(value[key]) for key in ('member', 'role', 'organization', 'application'))
    menus = ','.join(literal(item) for item in value['menus'])
    grants = ','.join(literal(item) for item in value['grants'])
    return ("SELECT json_build_object('members',(SELECT count(*) FROM memberships WHERE id=" + member
        + "),'roles',(SELECT count(*) FROM roles WHERE id=" + role
        + "),'userRoles',(SELECT count(*) FROM user_roles WHERE role_id=" + role
        + "),'roleMenus',(SELECT count(*) FROM role_menus WHERE role_id=" + role
        + "),'dataGrants',(SELECT count(*) FROM membership_data_grants WHERE id IN(" + grants
        + ")),'organizationApplications',(SELECT count(*) FROM organization_applications WHERE organization_id="
        + org + ' AND application_id=' + app
        + "),'organizationMenus',(SELECT count(*) FROM organization_menus WHERE organization_id=" + org
        + ' AND menu_id IN(' + menus + ")),'detailMenus',(SELECT count(*) FROM menus WHERE id="
        + literal(value['detailMenu']) + "),'detailTenantLinks',(SELECT count(*) FROM tenant_menus WHERE menu_id="
        + literal(value['detailMenu']) + "),'detailOrganizationLinks',(SELECT count(*) FROM organization_menus WHERE menu_id="
        + literal(value['detailMenu']) + "),'detailRoleLinks',(SELECT count(*) FROM role_menus WHERE menu_id="
        + literal(value['detailMenu']) + "))::text;")


def prepare_sql(value):
    member, role, user, tenant, org, app = (literal(value[key]) for key in
        ('member', 'role', 'user', 'tenant', 'organization', 'application'))
    menus = ','.join(literal(item) for item in value['menus'])
    statements = [binding_guard(value),
        "DO $fresh$ BEGIN IF EXISTS(SELECT 1 FROM memberships WHERE user_id=" + user
        + ' AND organization_id=' + org + " AND member_type='EMPLOYEE')"
        + ' OR EXISTS(SELECT 1 FROM memberships WHERE id=' + member + ')'
        + ' OR EXISTS(SELECT 1 FROM roles WHERE id=' + role + ')'
        + ' OR EXISTS(SELECT 1 FROM menus WHERE id=' + literal(value['detailMenu']) + ')'
        + ' OR EXISTS(SELECT 1 FROM organization_applications WHERE organization_id=' + org
        + ' AND application_id=' + app + ')'
        + ' OR EXISTS(SELECT 1 FROM organization_menus WHERE organization_id=' + org
        + ' AND menu_id IN(' + menus + '))'
        + " THEN RAISE EXCEPTION 'CONTEXT_FIXTURE_NOT_FRESH'; END IF; END $fresh$;",
        f"INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES({member},{user},{org},'EMPLOYEE');",
        f'INSERT INTO organization_applications(organization_id,application_id) VALUES({org},{app});',
        f"INSERT INTO roles(id,tenant_id,organization_id,name,code) VALUES({role},{tenant},{org},'Phase25 context switch',"
        + literal('p25-context-' + value['fixtureId'][-12:]) + ');',
        f'INSERT INTO user_roles(user_id,role_id,organization_id) VALUES({user},{role},{org});']
    for menu in value['menus']:
        statements += [f'INSERT INTO organization_menus(organization_id,menu_id) VALUES({org},{literal(menu)});',
                       f'INSERT INTO role_menus(role_id,menu_id) VALUES({role},{literal(menu)});']
    # The real host deliberately requires an explicit detail-page MENU; a
    # root MENU grants no implicit descendant navigation. This new invisible
    # route belongs only to this fixture, with its existing read permission.
    detail = literal(value['detailMenu'])
    statements += [f"INSERT INTO menus(id,application_id,parent_id,type,title,path,permission,visible,link_target) VALUES({detail},{app},"
        + literal(value['menus'][0]) + ",'MENU','Phase25 owned ticket detail','/micro/work-orders/tickets/:id','ticket:read',false,'MICRO');",
        f'INSERT INTO tenant_menus(tenant_id,menu_id) VALUES({tenant},{detail});',
        f'INSERT INTO organization_menus(organization_id,menu_id) VALUES({org},{detail}),('
        + literal(value['firstOrganization']) + f',{detail});',
        f'INSERT INTO role_menus(role_id,menu_id) VALUES({role},{detail}),('
        + literal(value['sourceRole']) + f',{detail});']
    for grant, (resource, action) in zip(value['grants'], OPERATIONS):
        statements.append('INSERT INTO membership_data_grants(id,membership_id,application_id,resource_code,action_code,scope_type) VALUES('
            + ','.join((literal(grant), member, app, literal(resource), literal(action), "'ORG_SUBTREE'")) + ');')
    return 'BEGIN;\n' + '\n'.join(statements) + '\n' + count_sql(value) + '\nCOMMIT;'


def cleanup_sql(value):
    member, role, user, org, app = (literal(value[key]) for key in
        ('member', 'role', 'user', 'organization', 'application'))
    # The membership and role are created in the same atomic transaction as
    # the non-ID organization links. Their exact generated IDs/code must
    # still prove that transaction committed before deleting those links.
    # A failed prepare must never remove pre-existing second-org grants.
    statements = [binding_guard(value),
        'DO $owned$ BEGIN IF NOT EXISTS(SELECT 1 FROM memberships WHERE id=' + member
        + ' AND user_id=' + user + ' AND organization_id=' + org + " AND member_type='EMPLOYEE')"
        + ' OR NOT EXISTS(SELECT 1 FROM roles WHERE id=' + role + ' AND organization_id=' + org
        + ' AND code=' + literal('p25-context-' + value['fixtureId'][-12:])
        + ") THEN RAISE EXCEPTION 'CONTEXT_OWNED_TRANSACTION_UNPROVEN'; END IF; END $owned$;",
        # Deleting only the exact new MENU cascades its four owned grant
        # families, including links added to the existing first-org role.
        'DELETE FROM menus WHERE id=' + literal(value['detailMenu']) + ' AND application_id=' + app
        + ' AND parent_id=' + literal(value['menus'][0])
        + " AND type='MENU' AND path='/micro/work-orders/tickets/:id' AND permission='ticket:read';",
        'DELETE FROM membership_data_grants WHERE membership_id=' + member + ' AND application_id=' + app
        + ' AND id IN(' + ','.join(literal(item) for item in value['grants']) + ');',
        'DELETE FROM role_menus WHERE role_id=' + role + ' AND menu_id IN('
        + ','.join(literal(item) for item in value['menus']) + ');',
        f'DELETE FROM user_roles WHERE user_id={user} AND role_id={role} AND organization_id={org};',
        f'DELETE FROM roles WHERE id={role} AND organization_id={org} AND code='
        + literal('p25-context-' + value['fixtureId'][-12:]) + ';',
        f'DELETE FROM organization_menus WHERE organization_id={org} AND menu_id IN('
        + ','.join(literal(item) for item in value['menus']) + ');',
        f'DELETE FROM organization_applications WHERE organization_id={org} AND application_id={app};',
        f"DELETE FROM memberships WHERE id={member} AND user_id={user} AND organization_id={org} AND member_type='EMPLOYEE';"]
    return 'BEGIN;\n' + '\n'.join(statements) + '\n' + count_sql(value) + '\nCOMMIT;'


def retire_marker(task, marker):
    """Unlink only this protected marker after committed DB cleanup.

    If cleanup's reply is lost, a repeated request sees no marker and no
    fixture can have been created without one. Failed/unknown DB cleanup
    never reaches this step, so its original identifiers remain recoverable.
    """
    need(marker == TASK / MARKER, 'CLEANUP_MARKER_SCOPE_INVALID')
    task.checked(marker)
    marker.unlink()
    need(not marker.exists() and not marker.is_symlink(), 'CLEANUP_MARKER_REMAINS')


def main():
    request = json.loads(sys.stdin.buffer.read(1025))
    need(type(request) is dict and set(request) == {'action'} and request['action'] in ACTIONS, 'REQUEST_INVALID')
    loader = importlib.util.spec_from_file_location('phase25_context_task', Path(__file__).with_name('phase23-environment.py'))
    base = importlib.util.module_from_spec(loader)
    loader.loader.exec_module(base)
    task = base.Task(require_root=False)
    need(task.state and task.state['prepared'] and task.state['mode'] == 'SOURCE'
         and not task.state.get('sourceQuarantined')
         and re.fullmatch(r'phase25-tools-r[1-9][0-9]*', task.state['snapshot']), 'OWNED_SOURCE_REQUIRED')
    label = task.state['snapshot']
    snapshot = TASK / ('source-snapshot-' + label)
    need(Path(__file__).resolve() == snapshot / PREFIX / 'phase25-context-control.py', 'FROZEN_CONTROL_REQUIRED')
    source = task.json(TASK / ('snapshot-' + label + '.json'))
    need(source['includeAllProjects'] and source['completeFrontendOutputs']
         and source['snapshotSha256'] == hashlib.sha256(json.dumps(source['inputs'], sort_keys=True).encode()).hexdigest(),
         'FULL_CANDIDATE_REQUIRED')
    names = ('phase25-context-control.py', 'phase25-context-browser.mjs', 'phase25-business-http.mjs')
    for name in names:
        need(task.digest(snapshot / PREFIX / name) == source['inputs'][PREFIX + name], 'FROZEN_TOOL_CHANGED')
    environment = task.json(TASK / 'https-environment.json')
    task.environment_binding(environment, 'SOURCE')
    need(environment['ids'] == task.json(TASK / 'phase23-source-environment.json')['ids'], 'ORIGINAL_IDENTITIES_CHANGED')
    business = task.json(TASK / 'phase25-business-manifest.json')
    fixture_loader = importlib.util.spec_from_file_location('phase25_context_fixture', Path(__file__).with_name('phase25-business-fixture.py'))
    fixture = importlib.util.module_from_spec(fixture_loader)
    fixture_loader.loader.exec_module(fixture)
    fixture.validate_manifest(task, business)
    need(business['prepared'] and business['toolsSnapshot'] == label, 'BUSINESS_BINDING_REQUIRED')
    task.guard_databases('SOURCE')
    ids = environment['ids']
    marker = TASK / MARKER
    action = request['action']
    if action == 'cleanup' and not marker.exists():
        sys.stdout.write(json.dumps({'fixtureRowsRemoved': True, 'manifestAbsent': True,
            'existingRowsOverwritten': False}))
        return
    previous = task.json(marker) if marker.exists() else None
    if previous is not None:
        validate_manifest(previous, ids, business, task.cluster, label)
    if action == 'prepare' and (previous is None or previous['cleaned']):
        value = {'contractVersion': CONTRACT, 'clusterId': task.cluster, 'database': business['baseDatabase'],
            'deploymentId': ids['deployment'], 'toolsSnapshot': label, 'fixtureId': new_id(),
            'member': new_id(), 'role': new_id(), 'grants': [new_id() for _ in OPERATIONS],
            'detailMenu': new_id(), 'firstOrganization': ids['organization'],
            'sourceRole': business['ids']['orders']['role'],
            'user': ids['employee'], 'tenant': ids['tenant'], 'organization': ids['secondOrganization'],
            'application': business['ids']['orders']['application'], 'menus': business['ids']['orders']['menus'],
            'prepared': False, 'cleaned': False}
        validate_manifest(value, ids, business, task.cluster, label)
        task.write(marker, value, replace=previous is not None)
    need(marker.exists(), 'OWNED_MANIFEST_REQUIRED')
    value = validate_manifest(task.json(marker), ids, business, task.cluster, label)
    if action == 'prepare':
        need(not value['prepared'] and not value['cleaned'], 'FIXTURE_ALREADY_USED')
        counts = json.loads(task.pg(prepare_sql(value), value['database']))
        need(counts == {'members': 1, 'roles': 1, 'userRoles': 1, 'roleMenus': len(value['menus']) + 1,
             'dataGrants': len(OPERATIONS), 'organizationApplications': 1, 'organizationMenus': len(value['menus']),
             'detailMenus': 1, 'detailTenantLinks': 1, 'detailOrganizationLinks': 2, 'detailRoleLinks': 2},
             'PREPARATION_INCOMPLETE')
        value['prepared'] = True
        task.write(marker, value, replace=True)
        result = {'environment': environment, 'business': business, 'fixture': value,
            'snapshotSha256': source['snapshotSha256'],
            'scriptSha256': {name: source['inputs'][PREFIX + name] for name in names}}
    elif action == 'cleanup':
        before = json.loads(task.pg(binding_guard(value) + '\n' + count_sql(value), value['database']))
        owned_keys = ('members', 'roles', 'userRoles', 'roleMenus', 'dataGrants',
                      'detailMenus', 'detailTenantLinks', 'detailOrganizationLinks', 'detailRoleLinks')
        no_owned_transaction = all(before[key] == 0 for key in owned_keys)
        if no_owned_transaction:
            # Prepare either rolled back or cleanup already committed. Do not
            # touch organization links whose ownership is no longer proven.
            counts = before
        else:
            counts = json.loads(task.pg(cleanup_sql(value), value['database']))
            need(all(item == 0 for item in counts.values()), 'FIXTURE_ROWS_REMAIN')
        retire_marker(task, marker)
        result = {'fixtureRowsRemoved': True, 'remaining': counts, 'existingRowsOverwritten': False,
                  'noOwnedTransactionLinksLeftUntouched': no_owned_transaction,
                  'privateManifestRemoved': True,
                  'securityAndBusinessFactsRetainedUntilOwnedDatabaseCleanup': True}
    else:
        result = json.loads(task.pg(binding_guard(value) + '\n' + count_sql(value), value['database']))
    sys.stdout.write(json.dumps(result))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        code = str(error)
        sys.stderr.write(code if re.fullmatch(r'PHASE25_CONTEXT_[A-Z0-9_]+', code) else 'PHASE25_CONTEXT_CONTROL_FAILED')
        raise SystemExit(1) from None
