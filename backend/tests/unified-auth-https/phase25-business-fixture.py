"""Prepare/retire only the owned Phase 25 business-service fixtures.

No import-time I/O. The root environment owner invokes prepare after the base
SOURCE fixture is prepared and before its namespace starts. This CLI never
starts application services or creates Docker resources. All credentials stay
in task-owner mode-0600 files; child/SQL diagnostics are never printed.
"""
import argparse
import contextlib
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import signal
import subprocess
import sys
import uuid

from prepare import literal, new_id

TASK = Path('/tmp/rust-saas-unified-auth-20261028')
MANIFEST = 'phase25-business-manifest.json'
ENVIRONMENT = 'phase25-business-environment.json'
LOCK = 'phase25-business-prepare.lock'
LEASES = ('phase25-business-lease.json', 'phase25-business-stores.pid',
          'phase25-business-orders.pid', 'phase25-business-notification.pid')
HOSTILE_NAME = '<img src=x onerror=window.__phase25_xss=1>'
SPECS = {
    'stores': {'binary': 'stores-api', 'migrator': 'stores-migrate', 'port': 23031,
               'healthPath': '/stores-api/health/ready', 'prefix': 'STORES',
               'code': 'stores', 'pairs': (('store', 'read'), ('store', 'create'),
               ('store', 'update'), ('store-staff', 'read'), ('store-daily-report', 'read'),
               ('store-daily-report', 'create'), ('store-daily-report', 'update'))},
    'orders': {'binary': 'work-orders-api', 'migrator': 'work-orders-migrate', 'port': 23004,
               'healthPath': '/work-orders-api/health/ready', 'prefix': 'WORK_ORDERS',
               'code': 'work-orders', 'pairs': (('ticket', 'read'), ('ticket', 'export'),
               ('ticket', 'create'), ('ticket', 'update'), ('ticket', 'delete'),
               ('notification', 'publish'))},
    'notification': {'binary': 'saas-notification-ws', 'port': 23003,
                     'healthPath': '/health/ready'},
}


class Failure(RuntimeError):
    def __init__(self, code):
        self.code = 'PHASE25_BUSINESS_' + code
        super().__init__(self.code)


def need(value, code):
    if not value:
        raise Failure(code)


def sha(data):
    return hashlib.sha256(data).hexdigest()


@contextlib.contextmanager
def defer_interrupts():
    """Finish a private-file commit before the outer finally handles a signal."""
    pending = False
    def defer(_signum, _frame):
        nonlocal pending
        pending = True
    originals = {sig: signal.getsignal(sig) for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP)}
    try:
        for sig in originals:
            signal.signal(sig, defer)
        yield
    finally:
        for sig, handler in originals.items():
            signal.signal(sig, handler)
        if pending:
            raise KeyboardInterrupt


def write_private(task, path, value, replace=False):
    with defer_interrupts():
        task.write(path, value, replace=replace)


def canonical_id(value):
    try:
        item = uuid.UUID(value)
        return type(value) is str and str(item) == value and item.version == 7 and item.variant == uuid.RFC_4122
    except (ValueError, AttributeError, TypeError):
        return False


def load_task():
    path = Path(__file__).resolve().with_name('phase23-environment.py')
    spec = importlib.util.spec_from_file_location('phase25_owned_base_environment', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.Task(require_root=True)


def shared_guard(manifest):
    return ("DO $guard$ BEGIN IF current_database()<>" + literal(manifest['baseDatabase'])
        + " OR NOT EXISTS(SELECT 1 FROM pg_database WHERE datname=current_database() AND "
        + "pg_get_userbyid(datdba)=" + literal(manifest['baseRolePrefix'] + '_migration_owner')
        + " AND shobj_description(oid,'pg_database')=" + literal(manifest['baseMarker'])
        + ") THEN RAISE EXCEPTION 'PHASE25_SHARED_BINDING'; END IF; END $guard$;\n")


def assert_no_business_leases(task):
    need(not any((TASK / name).exists() or (TASK / name).is_symlink() for name in LEASES), 'RUNTIME_LEASE_REMAINS')
    need(not any(key.startswith('business-') for key in (task.state or {}).get('processes', {})), 'RUNTIME_PROCESS_REMAINS')


def names_for(base_database):
    need(type(base_database) is str and re.fullmatch(r'unified_auth_https_[0-9a-f]{32}', base_database), 'BASE_NAME_INVALID')
    token = base_database[-32:]
    prefix = 'uah25_' + token[-12:]
    return prefix, {kind: 'unified_auth_phase25_' + kind + '_' + token for kind in ('stores', 'orders')}


def validate_manifest(task, value):
    need(type(value) is dict and value.get('contractVersion') == 'unified-auth-phase25-business-manifest/1'
         and value.get('clusterId') == task.cluster and canonical_id(value.get('deploymentId'))
         and canonical_id(value.get('fixtureId')), 'MANIFEST_BINDING_INVALID')
    prefix, databases = names_for(value['baseDatabase'])
    need(value['baseRolePrefix'] == 'uah_' + value['baseDatabase'][-12:]
         and value['baseMarker'] == 'identity-deployment/1/' + value['deploymentId']
         and value['rolePrefix'] == prefix and set(value['services']) == set(SPECS), 'MANIFEST_SCOPE_INVALID')
    for kind, spec in SPECS.items():
        item = value['services'][kind]
        expected_db = value['baseDatabase'] if kind == 'notification' else databases[kind]
        need(item['database'] == expected_db and item['runtime'] == prefix + '_' + kind + '_runtime'
             and item['binary'] == spec['binary'] and item['port'] == spec['port']
             and re.fullmatch(r'[0-9a-f]{64}', item['binarySha256']), 'SERVICE_SCOPE_INVALID')
        if kind != 'notification':
            need(item['owner'] == prefix + '_' + kind + '_owner'
                 and item['marker'] == 'unified-auth-phase25-business/1/' + value['deploymentId'] + '/' + kind,
                 'DATABASE_SCOPE_INVALID')
    need(set(value['ids']) == {'stores', 'orders', 'storeOrganization', 'customerSecondMember'}
         and canonical_id(value['ids']['storeOrganization']) and canonical_id(value['ids']['customerSecondMember']),
         'IDS_INVALID')
    for kind in ('stores', 'orders'):
        item = value['ids'][kind]
        need(set(item) == {'application', 'service', 'resource', 'role', 'menus', 'scopes', 'grants'}
             and all(canonical_id(item[key]) for key in ('application', 'service', 'resource', 'role'))
             and len(item['menus']) == len(SPECS[kind]['pairs']) + 1
             and len(item['scopes']) == len(SPECS[kind]['pairs'])
             and len(item['grants']) == 2 * len(SPECS[kind]['pairs'])
             and all(canonical_id(x) for key in ('menus', 'scopes', 'grants') for x in item[key]), 'APP_IDS_INVALID')
    ui = value['ui']
    need(canonical_id(ui['organizationId']) and canonical_id(ui['customerId'])
         and canonical_id(ui['secondOrganizationId']) and ui['hostileName'] == HOSTILE_NAME
         and type(ui['originalName']) is str and len(ui['originalName']) <= 100, 'UI_SCOPE_INVALID')
    return value


def base_ready(task):
    assert_no_business_leases(task)
    need(task.state and task.state.get('mode') == 'SOURCE' and task.state.get('prepared') is True
         and not task.state.get('sourceQuarantined') and not task.state.get('processes')
         and not (TASK / 'namespace.pid').exists() and not (TASK / 'gateway.pid').exists()
         and not any(TASK.glob('service-*.pid')), 'FRESH_BASE_REQUIRED')
    base = task.json(TASK / 'https-database.json')
    environment = task.json(TASK / 'https-environment.json')
    resources = task.resources()
    need(resources['source'] == base and base['clusterId'] == task.cluster
         and resources['deploymentId'] == environment['ids']['deployment']
         and environment.get('phase21') and environment.get('phase23')
         and not environment.get('phase25Ui'), 'SOURCE_BINDING_INVALID')
    task.guard_databases('SOURCE', quiet=True)
    snapshot, source, attestation = task.snapshot()
    need(task.state['snapshot'].startswith('phase25-') and task.state['buildSnapshot'].startswith('phase25-'),
         'CURRENT_FULL_CANDIDATE_REQUIRED')
    return base, environment, snapshot, source, attestation


def sql_array(values, kind='text'):
    return 'ARRAY[' + ','.join(literal(value) for value in values) + ']::' + kind + '[]'


def plan(task, base, environment, attestation):
    prefix, databases = names_for(base['database'])
    ids = {kind: {'application': new_id(), 'service': new_id(), 'resource': new_id(), 'role': new_id(),
        'menus': [new_id() for _ in range(len(SPECS[kind]['pairs']) + 1)],
        'scopes': [new_id() for _ in SPECS[kind]['pairs']],
        'grants': [new_id() for _ in range(2 * len(SPECS[kind]['pairs']))]} for kind in ('stores', 'orders')}
    ids.update({'storeOrganization': new_id(), 'customerSecondMember': new_id()})
    services = {}
    for kind, spec in SPECS.items():
        services[kind] = {'database': base['database'] if kind == 'notification' else databases[kind],
            'runtime': prefix + '_' + kind + '_runtime', 'binary': spec['binary'],
            'binarySha256': attestation['binaries'][spec['binary']], 'host': '127.0.0.1', 'port': spec['port'],
            'healthPath': spec['healthPath']}
        if kind != 'notification':
            services[kind].update({'owner': prefix + '_' + kind + '_owner',
                'marker': 'unified-auth-phase25-business/1/' + environment['ids']['deployment'] + '/' + kind,
                'creationPending': True, 'migratorSha256': attestation['binaries'][spec['migrator']]})
    name = task.pg('SELECT name FROM organizations WHERE id=' + literal(environment['ids']['organization']), base['database'])
    need(name and name != HOSTILE_NAME, 'ORIGINAL_ORGANIZATION_NAME_REQUIRED')
    old_policy = environment['platform'].get('DATA_GRANT_POLICY')
    policy = json.loads(old_policy) if old_policy is not None else {'applications': [], 'managementScopes': []}
    need(type(policy) is dict and set(policy) == {'applications', 'managementScopes'}
         and type(policy['applications']) is list and type(policy['managementScopes']) is list, 'POLICY_INVALID')
    for kind in ('stores', 'orders'):
        pairs = SPECS[kind]['pairs']
        policy['applications'].append({'applicationId': ids[kind]['application'], 'capabilities': [
            {'resource': resource, 'action': action, 'scopes': ['ORG_SUBTREE']} for resource, action in pairs],
            'memberLookups': [{'resource': resource, 'action': action} for resource, action in pairs
                if (kind == 'stores' and (resource, action) in (('store', 'create'), ('store', 'update'), ('store-staff', 'read')))
                or (kind == 'orders' and resource == 'ticket')],
            'organizationLookups': [{'resource': resource, 'action': action, 'organizationType': 'STORE'}
                for resource, action in pairs] if kind == 'stores' else []})
    manifest = {'contractVersion': 'unified-auth-phase25-business-manifest/1', 'clusterId': task.cluster,
        'fixtureId': new_id(), 'deploymentId': environment['ids']['deployment'], 'baseDatabase': base['database'],
        'baseRolePrefix': base['rolePrefix'], 'baseMarker': 'identity-deployment/1/' + environment['ids']['deployment'],
        'rolePrefix': prefix, 'snapshot': task.state['buildSnapshot'], 'snapshotSha256': attestation['snapshotSha256'],
        'toolsSnapshot': task.state['snapshot'], 'services': services, 'ids': ids,
        'ui': {'organizationId': environment['ids']['organization'], 'originalName': name, 'hostileName': HOSTILE_NAME,
               'customerId': environment['ids']['customer'], 'secondOrganizationId': environment['ids']['secondOrganization'],
               'restored': False}, 'basePolicyBefore': old_policy,
        'basePolicyApplied': json.dumps(policy, separators=(',', ':')), 'sharedCommitPending': False,
        'baseEnvironmentUpdated': False, 'prepared': False, 'environmentSha256': None,
        'sharedRowsDisposition': 'DISABLE_PRESERVE_IMMUTABLE_FACTS_UNTIL_BASE_DATABASE_DROP'}
    return validate_manifest(task, manifest)


def shared_sql(manifest, environment, credentials):
    ids, root = manifest['ids'], environment['ids']
    tenant, organization = literal(root['tenant']), literal(root['organization'])
    statements = [shared_guard(manifest),
        "DO $fresh$ BEGIN IF EXISTS(SELECT 1 FROM applications WHERE code IN('stores','work-orders')) "
        "OR EXISTS(SELECT 1 FROM auth_grants) OR EXISTS(SELECT 1 FROM memberships WHERE user_id="
        + literal(root['customer']) + ' AND organization_id=' + literal(root['secondOrganization'])
        + ") THEN RAISE EXCEPTION 'PHASE25_FRESH_SHARED_REQUIRED'; END IF; END $fresh$",
        'INSERT INTO organizations(id,tenant_id,parent_id,name,organization_type) VALUES('
        + literal(ids['storeOrganization']) + ',' + tenant + ',' + organization + ",'Phase25 owned STORE','STORE')",
        'INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES('
        + literal(ids['customerSecondMember']) + ',' + literal(root['customer']) + ','
        + literal(root['secondOrganization']) + ",'CUSTOMER')",
        'UPDATE organizations SET name=' + literal(HOSTILE_NAME) + ' WHERE id=' + organization
        + ' AND name=' + literal(manifest['ui']['originalName'])]
    for kind in ('stores', 'orders'):
        item, spec = ids[kind], SPECS[kind]
        app, role, resource = (literal(item[k]) for k in ('application', 'role', 'resource'))
        operations = [a + ':' + b for a, b in spec['pairs']]
        scopes = [spec['code'] + ':' + operation for operation in operations]
        statements += [
            f"INSERT INTO applications(id,code,name,base_path,entry_url,allowed_member_types) VALUES({app},{literal(spec['code'])},{literal('Phase25 owned '+kind)},{literal('/micro/'+spec['code'])},{literal('https://console.saas.test/sub/'+spec['code']+'/index.html')},ARRAY['EMPLOYEE'])",
            f'INSERT INTO tenant_applications(tenant_id,application_id) VALUES({tenant},{app})',
            f'INSERT INTO organization_applications(organization_id,application_id) VALUES({organization},{app})',
            f"INSERT INTO roles(id,tenant_id,organization_id,name,code) VALUES({role},{tenant},{organization},{literal('Phase25 owned '+kind)},{literal('phase25-'+kind)})",
            f"INSERT INTO platform_resource_services(id,service_id,secret_hash,purpose,application_id,audience,allowed_member_types,allowed_operations) VALUES({literal(item['service'])},{literal(credentials[kind]['serviceId'])},{literal(sha(credentials[kind]['serviceSecret'].encode()))},'APP_RESOURCE',{app},{literal(spec['code'])},ARRAY['EMPLOYEE'],{sql_array(operations)})",
            f"INSERT INTO auth_resources(id,resource_key,audience,status) VALUES({resource},{literal(spec['code'])},{literal(spec['code'])},'ACTIVE')",
        ]
        for account in ('admin', 'employee'):
            statements.append(f"INSERT INTO user_roles(user_id,role_id,organization_id) VALUES({literal(root[account])},{role},{organization})")
        for index, menu in enumerate(item['menus']):
            permission = '' if index == 0 else operations[index - 1]
            parent = 'NULL' if index == 0 else literal(item['menus'][0])
            statements += [
                f"INSERT INTO menus(id,application_id,parent_id,type,title,path,permission,link_target) VALUES({literal(menu)},{app},{parent},{literal('MENU' if index==0 else 'BUTTON')},{literal('Phase25 '+(permission or kind))},{literal('/micro/'+spec['code'] if index==0 else '')},{literal(permission)},'MICRO')",
                f'INSERT INTO tenant_menus(tenant_id,menu_id) VALUES({tenant},{literal(menu)})',
                f'INSERT INTO organization_menus(organization_id,menu_id) VALUES({organization},{literal(menu)})',
                f'INSERT INTO role_menus(role_id,menu_id) VALUES({role},{literal(menu)})',
            ]
        for index, (operation_resource, action) in enumerate(spec['pairs']):
            statements += [
                f"INSERT INTO auth_scopes(id,resource_id,scope_name,status) VALUES({literal(item['scopes'][index])},{resource},{literal(scopes[index])},'ACTIVE')",
                f"INSERT INTO platform_resource_bindings(application_id,resource_code,action_code,canonical_resource,audience,oauth_scope) VALUES({app},{literal(operation_resource)},{literal(action)},{literal(spec['code'])},{literal(spec['code'])},{literal(scopes[index])})",
            ]
            for offset, account in enumerate(('admin', 'employee')):
                grant = item['grants'][offset * len(spec['pairs']) + index]
                statements.append(f"INSERT INTO membership_data_grants(id,membership_id,application_id,resource_code,action_code,scope_type) VALUES({literal(grant)},{literal(root[account+'Member'])},{app},{literal(operation_resource)},{literal(action)},'ORG_SUBTREE')")
    return 'BEGIN;\n' + ';\n'.join(statements) + ';\nCOMMIT;'


def create_roles(task, manifest, credentials):
    marker = 'unified-auth-phase25-business/1/' + manifest['deploymentId']
    roles = []
    for kind, item in manifest['services'].items():
        if kind != 'notification':
            roles += [f'CREATE ROLE "{item["owner"]}" NOLOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS',
                      f'COMMENT ON ROLE "{item["owner"]}" IS {literal(marker)}']
        roles += [f'CREATE ROLE "{item["runtime"]}" LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD {literal(credentials[kind]["databasePassword"])}',
                  f'COMMENT ON ROLE "{item["runtime"]}" IS {literal(marker)}']
    task.pg("BEGIN; SET LOCAL password_encryption='scram-sha-256';\n" + ';\n'.join(roles) + '; COMMIT;')


def migrate_business(task, manifest, kind):
    item, spec = manifest['services'][kind], SPECS[kind]
    name, owner, runtime = item['database'], item['owner'], item['runtime']
    task.pg(f'CREATE DATABASE "{name}" OWNER "{owner}";')
    task.pg(f'COMMENT ON DATABASE "{name}" IS {literal(item["marker"])}; REVOKE ALL ON DATABASE "{name}" FROM PUBLIC;')
    item['creationPending'] = False
    write_private(task, TASK / MANIFEST, manifest, replace=True)
    migration_url = f'postgresql://postgres@localhost:55494/{name}?host={TASK}&options[role]={owner}'
    binary = TASK / 'bin' / spec['migrator']
    need(task.digest(binary) == item['migratorSha256'], 'MIGRATOR_CHANGED')
    child_env = {'PATH': '/usr/lib/postgresql/18/bin:/usr/bin:/bin', 'HOME': str(TASK), 'LANG': 'C.UTF-8',
                 spec['prefix'] + '_DATABASE_URL': migration_url}
    need(not any(key.upper().startswith('PG') for key in child_env), 'MIGRATOR_AMBIENT_PG_REJECTED')
    result = subprocess.run(['/usr/bin/setpriv', '--reuid=' + str(task.uid), '--regid=' + str(task.gid),
        '--init-groups', str(binary)], capture_output=True, env=child_env, timeout=330)
    need(result.returncode == 0, 'CURRENT_MIGRATION_FAILED')
    # The normal startup path verifies the current SQLx/fingerprint contract.
    # Its restricted runtime can read that history, but cannot change it or DDL.
    writable = ('stores', 'store_daily_reports') if kind == 'stores' else (
        'work_orders', 'work_order_counters', 'work_order_create_idempotency', 'work_order_reminder_intents')
    statements = [f'ALTER SCHEMA public OWNER TO "{owner}"',
        'REVOKE ALL ON SCHEMA public FROM PUBLIC', 'REVOKE ALL ON ALL TABLES IN SCHEMA public FROM PUBLIC',
        'REVOKE ALL ON ALL FUNCTIONS IN SCHEMA public FROM PUBLIC',
        f'GRANT CONNECT ON DATABASE "{name}" TO "{runtime}"', f'GRANT USAGE ON SCHEMA public TO "{runtime}"',
        f'GRANT SELECT ON ALL TABLES IN SCHEMA public TO "{runtime}"',
        f'GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA public TO "{runtime}"']
    statements.extend(f'GRANT INSERT,UPDATE,DELETE ON public.{table} TO "{runtime}"' for table in writable)
    if kind == 'stores':
        statements.append(f'GRANT INSERT ON public.store_audit_events TO "{runtime}"')
    statements += [f'ALTER DEFAULT PRIVILEGES FOR ROLE "{owner}" IN SCHEMA public REVOKE ALL ON TABLES FROM PUBLIC',
                   f'ALTER DEFAULT PRIVILEGES FOR ROLE "{owner}" IN SCHEMA public REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC']
    task.pg('BEGIN;\n' + ';\n'.join(statements) + '; COMMIT;', name)
    valid = task.pg("SELECT count(*)=1 AND bool_and(version=1 AND success) FROM _sqlx_migrations", name)
    need(valid == 't', 'CURRENT_HISTORY_INVALID')
    isolated = task.pg('SELECT NOT has_database_privilege(' + literal(runtime) + ",'" + name + "','CREATE,TEMP')"
        + ' AND NOT has_schema_privilege(' + literal(runtime) + ",'public','CREATE')"
        + ' AND NOT pg_has_role(' + literal(runtime) + ',' + literal(owner) + ",'MEMBER')"
        + ' AND NOT has_table_privilege(' + literal(runtime) + ",'public._sqlx_migrations','INSERT,UPDATE,DELETE')"
        + ' AND NOT has_database_privilege(' + literal(runtime) + ',' + literal(manifest['baseDatabase']) + ",'CONNECT')", name)
    need(isolated == 't', 'RUNTIME_ISOLATION_FAILED')


def environment_for(manifest, base, credentials):
    ca = base['identity']['CONTEXT_PLATFORM_CA_FILE']
    result = {'contractVersion': 'unified-auth-phase25-business-environment/1',
        **{key: manifest[key] for key in ('clusterId', 'deploymentId', 'baseDatabase', 'snapshot', 'snapshotSha256')},
        'services': {}, 'ids': manifest['ids'], 'phase25Ui': ui_environment(manifest),
        'platformOverrides': {'DATA_GRANT_POLICY': manifest['basePolicyApplied']}}
    for kind, spec in SPECS.items():
        item, secret = manifest['services'][kind], credentials[kind]
        database_url = 'postgresql://' + item['runtime'] + ':' + secret['databasePassword'] + '@127.0.0.1:55494/' + item['database']
        if kind == 'notification':
            values = {'DATABASE_URL': database_url, 'NOTIFICATION_ENV': 'test', 'RUST_LOG': 'info',
                'PLATFORM_API_BASE_URL': 'https://console.saas.test:24444', 'NOTIFICATION_PLATFORM_CA_FILE': ca,
                'NOTIFICATION_ALLOWED_ORIGINS': 'https://console.saas.test', 'NOTIFICATION_HOST': '127.0.0.1',
                'NOTIFICATION_PORT': str(spec['port']), 'METRICS_TOKEN': secret['metrics'],
                'NOTIFICATION_SOCKET_PATH': '/socket.io/'}
        else:
            prefix = spec['prefix']
            values = {'NODE_ENV': 'production', 'RUST_LOG': 'info', prefix + '_DATABASE_URL': database_url,
                prefix + '_HOST': '127.0.0.1', prefix + '_PORT': str(spec['port']),
                prefix + '_ALLOWED_ORIGINS': 'https://console.saas.test', prefix + '_METRICS_TOKEN': secret['metrics'],
                'PLATFORM_API_ORIGIN': 'https://console.saas.test:24444', 'PLATFORM_CA_CERT_FILE': ca,
                'PLATFORM_CLIENT_ID': secret['serviceId'], 'PLATFORM_CLIENT_SECRET': secret['serviceSecret'],
                'PLATFORM_AUTHORIZATION_MODE': 'combined' if kind == 'stores' else 'canonical',
                'PLATFORM_IDENTITY_ISSUER': 'https://auth.saas.test',
                'PLATFORM_RESOURCE_SCOPE_BINDINGS': json.dumps({resource + ':' + action: spec['code'] + ':' + resource + ':' + action
                    for resource, action in spec['pairs']}, separators=(',', ':'))}
        result['services'][kind] = {key: item[key] for key in ('binary', 'host', 'port', 'healthPath')}
        result['services'][kind]['environment'] = values
    return result


def ui_environment(manifest):
    return {'contractVersion': 'unified-auth-phase25-ui-fixture/1', 'organizationId': manifest['ui']['organizationId'],
        'hostileName': HOSTILE_NAME, 'customerSecondMember': manifest['ids']['customerSecondMember'],
        'secondOrganizationId': manifest['ui']['secondOrganizationId']}


def prepare(task):
    need(not any((TASK / name).exists() or (TASK / name).is_symlink() for name in (MANIFEST, ENVIRONMENT, LOCK)), 'EMPTY_PREPARATION_REQUIRED')
    base, environment, _snapshot, _source, attestation = base_ready(task)
    manifest = plan(task, base, environment, attestation)
    names = [item['database'] for kind, item in manifest['services'].items() if kind != 'notification']
    occupied = task.pg('SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname=ANY(' + sql_array(names)
        + ')) OR EXISTS(SELECT 1 FROM pg_roles WHERE starts_with(rolname,' + literal(manifest['rolePrefix'] + '_') + '))')
    need(occupied == 'f', 'NEW_RESOURCES_ALREADY_EXIST')
    try:
        write_private(task, TASK / LOCK, {'contractVersion': 'unified-auth-phase25-business-preparation/1', 'fixtureId': manifest['fixtureId']})
        write_private(task, TASK / MANIFEST, manifest)
        credentials = {kind: {'databasePassword': secrets.token_hex(32), 'metrics': secrets.token_hex(32),
            'serviceId': 'phase25-' + kind + '-' + manifest['fixtureId'][-12:], 'serviceSecret': secrets.token_hex(32)} for kind in SPECS}
        create_roles(task, manifest, credentials)
        for kind in ('stores', 'orders'):
            migrate_business(task, manifest, kind)
        # A role granted only notification row reads cannot become a platform or
        # identity caller. LISTEN itself requires no table write privilege.
        notification = manifest['services']['notification']['runtime']
        task.pg(shared_guard(manifest) + f'GRANT CONNECT ON DATABASE "{manifest["baseDatabase"]}" TO "{notification}"; '
            + f'GRANT USAGE ON SCHEMA public TO "{notification}"; GRANT SELECT ON public.notifications TO "{notification}";', manifest['baseDatabase'])
        manifest['sharedCommitPending'] = True
        write_private(task, TASK / MANIFEST, manifest, replace=True)
        task.pg(shared_sql(manifest, environment, credentials), manifest['baseDatabase'])
        business = environment_for(manifest, environment, credentials)
        manifest['environmentSha256'] = sha(json.dumps(business, indent=2).encode() + b'\n')
        write_private(task, TASK / MANIFEST, manifest, replace=True)
        write_private(task, TASK / ENVIRONMENT, business)
        need(manifest['environmentSha256'] == sha(task.read(TASK / ENVIRONMENT)), 'ENVIRONMENT_WRITE_CHANGED')
        # Preserve all unrelated base fields. Only these two exact additions
        # belong to this fixture and are later restored with compare-and-set.
        current = task.json(TASK / 'https-environment.json')
        need(current == environment, 'BASE_ENVIRONMENT_CHANGED')
        current['platform']['DATA_GRANT_POLICY'] = manifest['basePolicyApplied']
        current['phase25Ui'] = ui_environment(manifest)
        # Persist the intent first; atomic replacement leaves either the original
        # or exactly the applied value for compare-and-set cleanup.
        manifest['baseEnvironmentUpdated'] = True
        write_private(task, TASK / MANIFEST, manifest, replace=True)
        write_private(task, TASK / 'https-environment.json', current, replace=True)
        manifest['prepared'] = True
        write_private(task, TASK / MANIFEST, manifest, replace=True)
        return {'status': 'PHASE25_BUSINESS_PREPARED', 'databasesCreated': 2, 'rolesCreated': 5,
            'servicesConfigured': 3, 'servicesStarted': 0, 'containersCreated': 0}
    except BaseException:
        # A retained manifest is the recovery journal if exact cleanup fails.
        if (TASK / MANIFEST).exists() and task.json(TASK / MANIFEST).get('fixtureId') == manifest['fixtureId']:
            cleanup(task, allow_preparing=True)
        raise
    finally:
        if (TASK / LOCK).exists():
            record = task.json(TASK / LOCK)
            if record.get('fixtureId') == manifest['fixtureId']:
                (TASK / LOCK).unlink()


def restore_ui(task, manifest):
    if manifest['ui']['restored']:
        return
    ui = manifest['ui']
    sql = shared_guard(manifest) + "DO $ui$ BEGIN IF EXISTS(SELECT 1 FROM organizations WHERE id="
    sql += literal(ui['organizationId']) + ' AND name NOT IN(' + literal(ui['originalName']) + ',' + literal(HOSTILE_NAME)
    sql += ")) OR EXISTS(SELECT 1 FROM memberships WHERE id=" + literal(manifest['ids']['customerSecondMember'])
    sql += ' AND (user_id<>' + literal(ui['customerId']) + ' OR organization_id<>' + literal(ui['secondOrganizationId'])
    sql += " OR member_type<>'CUSTOMER')) THEN RAISE EXCEPTION 'PHASE25_UI_CHANGED'; END IF; END $ui$;\n"
    sql += 'UPDATE organizations SET name=' + literal(ui['originalName']) + ' WHERE id=' + literal(ui['organizationId'])
    sql += ' AND name=' + literal(HOSTILE_NAME) + '; DELETE FROM memberships WHERE id=' + literal(manifest['ids']['customerSecondMember']) + ';'
    task.pg('BEGIN;\n' + sql + '\nCOMMIT;', manifest['baseDatabase'])
    base = task.json(TASK / 'https-environment.json')
    if 'phase25Ui' in base:
        need(base['phase25Ui'] == ui_environment(manifest), 'UI_ENVIRONMENT_CHANGED')
        base.pop('phase25Ui')
        write_private(task, TASK / 'https-environment.json', base, replace=True)
    manifest['ui']['restored'] = True
    write_private(task, TASK / MANIFEST, manifest, replace=True)


def cleanup(task, allow_preparing=False, ui_only=False):
    assert_no_business_leases(task) if not ui_only else None
    if not (TASK / MANIFEST).exists():
        need(not ui_only, 'UI_MANIFEST_REQUIRED')
        unknown = task.pg("SELECT EXISTS(SELECT 1 FROM pg_database WHERE starts_with(datname,'unified_auth_phase25_')) OR EXISTS(SELECT 1 FROM pg_roles WHERE starts_with(rolname,'uah25_'))")
        need(unknown == 'f' and not any((TASK / name).exists() or (TASK / name).is_symlink()
             for name in (ENVIRONMENT, LOCK, MANIFEST)), 'UNKNOWN_RESOURCES_WITHOUT_MANIFEST')
        return {'status': 'PHASE25_BUSINESS_CLEANUP_NOOP', 'databasesRemaining': 0, 'rolesRemaining': 0}
    manifest = validate_manifest(task, task.json(TASK / MANIFEST))
    need(allow_preparing or not (TASK / LOCK).exists(), 'PREPARATION_ACTIVE')
    base = task.json(TASK / 'https-database.json')
    need(base['database'] == manifest['baseDatabase'] and base['rolePrefix'] == manifest['baseRolePrefix']
         and base['clusterId'] == manifest['clusterId'], 'BASE_MANIFEST_CHANGED')
    task.pg(shared_guard(manifest), manifest['baseDatabase'])
    restore_ui(task, manifest)
    if ui_only:
        return {'status': 'PHASE25_BUSINESS_UI_RESTORED', 'originalNameRestored': True, 'addedCustomerMemberRemoved': True}
    # Retire exact shared fixtures. Immutable notification/audit facts remain in
    # this owned base database and are removed by the base database's final DROP.
    if manifest['sharedCommitPending']:
        statements = [shared_guard(manifest)]
        for kind in ('stores', 'orders'):
            item, spec = manifest['ids'][kind], SPECS[kind]
            statements += [f"UPDATE platform_resource_services SET status='DISABLED' WHERE id={literal(item['service'])} AND application_id={literal(item['application'])} AND purpose='APP_RESOURCE'",
                f"UPDATE applications SET status='DISABLED' WHERE id={literal(item['application'])} AND code={literal(spec['code'])}",
                f"UPDATE auth_resources SET status='SUSPENDED' WHERE id={literal(item['resource'])} AND resource_key={literal(spec['code'])}",
                'DELETE FROM membership_data_grants WHERE id=ANY(' + sql_array(item['grants'], 'uuid') + ')',
                'DELETE FROM user_roles WHERE role_id=' + literal(item['role']),
                "UPDATE roles SET status='DISABLED' WHERE id=" + literal(item['role'])]
        task.pg('BEGIN;\n' + ';\n'.join(statements) + '; COMMIT;', manifest['baseDatabase'])
    current = task.json(TASK / 'https-environment.json')
    if manifest['baseEnvironmentUpdated']:
        observed = current['platform'].get('DATA_GRANT_POLICY')
        need(observed in (manifest['basePolicyApplied'], manifest['basePolicyBefore']), 'BASE_POLICY_CHANGED')
        if observed == manifest['basePolicyApplied']:
            if manifest['basePolicyBefore'] is None:
                current['platform'].pop('DATA_GRANT_POLICY')
            else:
                current['platform']['DATA_GRANT_POLICY'] = manifest['basePolicyBefore']
            write_private(task, TASK / 'https-environment.json', current, replace=True)
        manifest['baseEnvironmentUpdated'] = False
        write_private(task, TASK / MANIFEST, manifest, replace=True)
    marker = 'unified-auth-phase25-business/1/' + manifest['deploymentId']
    for kind, item in manifest['services'].items():
        roles = [item['runtime']] + ([item['owner']] if kind != 'notification' else [])
        for role in roles:
            observed = task.pg("SELECT json_build_object('marker',COALESCE(shobj_description(oid,'pg_authid'),'')) FROM pg_roles WHERE rolname=" + literal(role))
            need(not observed or json.loads(observed)['marker'] == marker, 'ROLE_MARKER_CHANGED')
        if kind != 'notification':
            record = task.pg("SELECT json_build_object('owner',pg_get_userbyid(datdba),'marker',COALESCE(shobj_description(oid,'pg_database'),'')) FROM pg_database WHERE datname=" + literal(item['database']))
            if record:
                actual = json.loads(record)
                need(actual['owner'] == item['owner'] and (actual['marker'] == item['marker']
                     or (item['creationPending'] and actual['marker'] == '')), 'DATABASE_MARKER_CHANGED')
                need(task.pg('SELECT count(*) FROM pg_stat_activity WHERE datname=' + literal(item['database'])) == '0', 'BUSINESS_DATABASE_CONNECTIONS_REMAIN')
                task.pg('DROP DATABASE "' + item['database'] + '";')
        else:
            role = item['runtime']
            if task.pg('SELECT EXISTS(SELECT 1 FROM pg_roles WHERE rolname=' + literal(role) + ')') == 't':
                task.pg(shared_guard(manifest) + f'REVOKE SELECT ON public.notifications FROM "{role}"; REVOKE USAGE ON SCHEMA public FROM "{role}"; REVOKE CONNECT ON DATABASE "{manifest["baseDatabase"]}" FROM "{role}";', manifest['baseDatabase'])
        for role in roles:
            # No DROP OWNED: unexpected dependencies must fail and preserve the
            # journal instead of revoking unknown grants in unrelated databases.
            task.pg('DROP ROLE IF EXISTS "' + role + '";')
    if (TASK / ENVIRONMENT).exists():
        need(sha(task.read(TASK / ENVIRONMENT)) == manifest['environmentSha256'], 'BUSINESS_ENVIRONMENT_CHANGED')
        (TASK / ENVIRONMENT).unlink()
    need(task.json(TASK / MANIFEST)['fixtureId'] == manifest['fixtureId'], 'MANIFEST_CHANGED')
    (TASK / MANIFEST).unlink()
    return {'status': 'PHASE25_BUSINESS_CLEANED', 'databasesRemoved': 2, 'rolesRemoved': 5,
        'sharedFixturesDisabled': True, 'sharedImmutableFactsAwaitOwnedBaseDatabaseDrop': True,
        'originalNameRestored': True, 'addedCustomerMemberRemoved': True, 'containersCreated': 0}


@contextlib.contextmanager
def interruption_cleanup():
    def stop(_signum, _frame):
        raise KeyboardInterrupt
    originals = {sig: signal.getsignal(sig) for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP)}
    try:
        for sig in originals:
            signal.signal(sig, stop)
        yield
    finally:
        for sig, handler in originals.items():
            signal.signal(sig, handler)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('action', choices=('prepare', 'cleanup', 'cleanup-ui'))
    parser.add_argument('--task-dir', type=Path, required=True)
    args = parser.parse_args()
    need(args.task_dir == TASK and args.task_dir.is_absolute(), 'EXACT_TASK_REQUIRED')
    task = load_task()
    with interruption_cleanup():
        result = prepare(task) if args.action == 'prepare' else cleanup(task, ui_only=args.action == 'cleanup-ui')
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    try:
        main()
    except (Exception, KeyboardInterrupt) as error:
        code = error.code if isinstance(error, Failure) else 'PHASE25_BUSINESS_FIXTURE_REJECTED'
        print(code, file=sys.stderr)
        raise SystemExit(130 if isinstance(error, KeyboardInterrupt) else 1) from None
