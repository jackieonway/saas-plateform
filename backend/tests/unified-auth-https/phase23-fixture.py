"""Owner-only additions to a fresh, canonical three-service recovery fixture.

No application imports this file. Secret material is written through the same
anchored mode-0600 primitives as the recovery tool and never printed.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import sys
from urllib.parse import urlsplit

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric import rsa

from prepare import b64, literal, new_id, psql
import phase23_cluster_binding as cluster_binding

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'ops/identity-recovery'))
import core

TASK = Path('/tmp/rust-saas-unified-auth-20261028')


def organization_marker_grants(environment):
    """Grant the real organization-edit permission only in the owned console.

    Product authorization still proves the selected employee session, role,
    tenant/organization application caps and menu caps. No administrator flag,
    cross-tenant grant or product authorization function is changed here.
    """
    scope = environment['ids']
    names = ('tenant', 'organization', 'secondOrganization', 'application', 'admin', 'adminMember')
    if any(not isinstance(scope.get(name), str) or not re.fullmatch(
            r'[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}', scope[name])
            for name in names):
        raise RuntimeError('PHASE23_ORGANIZATION_MARKER_BINDING_INVALID')
    q = {name: literal(scope[name]) + '::uuid' for name in names}
    menu, role = literal(new_id()) + '::uuid', literal(new_id()) + '::uuid'
    return [
        "DO $marker$ BEGIN IF NOT EXISTS(SELECT 1 FROM tenants tenant "
        "JOIN organizations selected ON selected.tenant_id=tenant.id "
        "JOIN organizations target ON target.tenant_id=tenant.id AND target.parent_id=selected.id "
        "JOIN memberships member ON member.organization_id=selected.id "
        "JOIN users actor ON actor.id=member.user_id "
        "JOIN tenant_applications tenant_app ON tenant_app.tenant_id=tenant.id "
        "JOIN organization_applications org_app ON org_app.organization_id=selected.id "
        "AND org_app.application_id=tenant_app.application_id "
        "JOIN applications app ON app.id=org_app.application_id "
        f"WHERE tenant.id={q['tenant']} AND tenant.is_platform AND tenant.status='ENABLED' "
        f"AND selected.id={q['organization']} AND selected.parent_id IS NULL AND selected.status='ENABLED' "
        f"AND target.id={q['secondOrganization']} AND target.status='ENABLED' "
        f"AND member.id={q['adminMember']} AND member.user_id={q['admin']} "
        "AND member.member_type='EMPLOYEE' AND member.status='ENABLED' AND actor.status='ENABLED' "
        f"AND app.id={q['application']} AND app.code='https-console' AND app.status='ENABLED' "
        "AND 'EMPLOYEE'=ANY(app.allowed_member_types)) THEN "
        "RAISE EXCEPTION 'PHASE23_ORGANIZATION_MARKER_SCOPE_INVALID'; END IF; END $marker$",
        "INSERT INTO menus(id,application_id,type,title,permission,path) VALUES("
        + menu + ',' + q['application'] + ",'MENU','Owned Phase23 organization marker',"
        "'system:organization','/system/organizations')",
        "INSERT INTO tenant_menus(tenant_id,menu_id) VALUES(" + q['tenant'] + ',' + menu + ')',
        "INSERT INTO organization_menus(organization_id,menu_id) VALUES(" + q['organization'] + ',' + menu + ')',
        "INSERT INTO roles(id,tenant_id,organization_id,name,code,status) VALUES("
        + role + ',' + q['tenant'] + ',' + q['organization']
        + ",'Owned Phase23 organization marker','phase23-organization-marker','ENABLED')",
        "INSERT INTO user_roles(user_id,role_id,organization_id) VALUES("
        + q['admin'] + ',' + role + ',' + q['organization'] + ')',
        "INSERT INTO role_menus(role_id,menu_id) VALUES(" + role + ',' + menu + ')',
    ]


def organization_marker_seed(environment, marker_organization):
    """Keep the RPO mutation outside every customer context candidate."""
    scope = environment['ids']
    if (not isinstance(marker_organization, str) or not re.fullmatch(
            r'[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}', marker_organization)
            or marker_organization in scope.values()):
        raise RuntimeError('PHASE23_MARKER_ORGANIZATION_BINDING_INVALID')
    marker = literal(marker_organization) + '::uuid'
    tenant = literal(scope['tenant']) + '::uuid'
    parent = literal(scope['organization']) + '::uuid'
    return [
        "INSERT INTO organizations(id,tenant_id,parent_id,name) VALUES("
        + marker + ',' + tenant + ',' + parent + ",'Owned Phase23 RPO marker')",
        "DO $marker$ BEGIN IF NOT EXISTS(SELECT 1 FROM organizations marker "
        f"WHERE marker.id={marker} AND marker.tenant_id={tenant} AND marker.parent_id={parent} "
        "AND marker.status='ENABLED') OR EXISTS(SELECT 1 FROM memberships "
        f"WHERE organization_id={marker}) THEN "
        "RAISE EXCEPTION 'PHASE23_MARKER_ORGANIZATION_NOT_ISOLATED'; END IF; END $marker$",
    ]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--task-dir', type=Path, required=True)
    parser.add_argument('--admin-url', required=True)
    args = parser.parse_args()
    if (args.task_dir.resolve(strict=True) != TASK or os.getuid() == 0
            or args.admin_url != 'postgres://postgres@127.0.0.1:55494/postgres'
            or any(TASK.glob('service-*.pid')) or (TASK / 'namespace.pid').exists()):
        raise RuntimeError('PHASE23_FRESH_OWNER_REQUIRED')
    core.private_directory(TASK)
    binding = cluster_binding.load_binding(TASK)
    cluster = binding['systemIdentifier']
    os.environ['PGOPTIONS'] = '-c log_statement=none -c log_min_error_statement=panic'
    manifest = json.loads(core.private_read(TASK / 'https-database.json'))
    environment = json.loads(core.private_read(TASK / 'https-environment.json'))
    name, prefix = manifest['database'], manifest['rolePrefix']
    deployment = environment['ids']['deployment']
    if (not re.fullmatch(r'unified_auth_https_[0-9a-f]{32}', name)
            or prefix != 'uah_' + name[-12:] or manifest['clusterId'] != cluster
            or 'phase23' in environment or not environment.get('portal')
            or not environment.get('phase21')):
        raise RuntimeError('PHASE23_SOURCE_BINDING_INVALID')
    database = args.admin_url.rsplit('/', 1)[0] + '/' + name
    marker = 'identity-deployment/1/' + deployment
    portal = manifest.get('portal')
    if portal != {'database': 'unified_auth_portal_' + name[-32:],
                  'owner': prefix + '_portal_owner', 'runtime': prefix + '_portal_runtime',
                  'marker': 'unified-auth-https-portal/1/' + deployment,
                  'initializationPending': False}:
        raise RuntimeError('PHASE23_PORTAL_BINDING_INVALID')
    check = psql(database, "SELECT system_identifier::text||'|'||shobj_description("
        "(SELECT oid FROM pg_database WHERE datname=current_database()),'pg_database') "
        "FROM pg_control_system() WHERE " + cluster_binding.sql_guard(binding))
    if check != cluster + '|' + marker:
        raise RuntimeError('PHASE23_DATABASE_MARKER_INVALID')
    portal_check = psql(args.admin_url.rsplit('/', 1)[0] + '/' + portal['database'],
        "SELECT system_identifier::text||'|'||shobj_description("
        "(SELECT oid FROM pg_database WHERE datname=current_database()),'pg_database') "
        "FROM pg_control_system() WHERE " + cluster_binding.sql_guard(binding))
    if portal_check != cluster + '|' + portal['marker']:
        raise RuntimeError('PHASE23_PORTAL_MARKER_INVALID')
    # Bind every mutable path and every runtime connection before the first
    # private output, role-password change or signing-material replacement.
    signing_path = core.safe_path(environment['identity']['OIDC_KEYSET_PATH'], TASK)
    signing = json.loads(core.private_read(signing_path))
    if len(signing['keys']) != 1 or signing['keys'][0]['kid'] != 'owned-https':
        raise RuntimeError('PHASE23_INITIAL_SIGNING_MATERIAL_INVALID')
    for kind, suffix in (('identity', 'identity_runtime'), ('platform', 'platform_runtime'), ('portal', 'portal_runtime')):
        parsed = urlsplit(environment[kind]['DATABASE_URL'])
        expected_database = manifest['portal']['database'] if kind == 'portal' else name
        if (parsed.scheme != 'postgres' or parsed.username != prefix + '_' + suffix
                or parsed.hostname != '127.0.0.1' or parsed.port != 55494
                or parsed.path != '/' + expected_database or parsed.query or parsed.fragment):
            raise RuntimeError('PHASE23_RUNTIME_ROLE_OR_DATABASE_INVALID')
    legacy = ('oidc_clients', 'oauth_clients', 'oidc_grants', 'oidc_access_tokens',
              'oidc_refresh_tokens', 'oidc_authorization_requests', 'oidc_authorization_codes',
              'oauth_tokens', 'oauth_authorization_requests', 'oauth_authorization_codes', 'oauth_device_codes')
    if any(psql(database, "SELECT to_regclass('public." + table + "') IS NOT NULL") != 'f' for table in legacy):
        raise RuntimeError('PHASE23_LEGACY_STATE_PRESENT')
    marker_grants = organization_marker_grants(environment)
    ids = {'keyAdminService': new_id(), 'retireMember': new_id(), 'markerOrganization': new_id()}
    marker_seed = organization_marker_seed(environment, ids['markerOrganization'])
    admin_secret = secrets.token_hex(32)
    statements = [*marker_seed, *marker_grants,
        "INSERT INTO identity_service_clients(id,secret_hash,purpose,status,created_at) VALUES("
        + literal(ids['keyAdminService']) + ',' + literal(hashlib.sha256(admin_secret.encode()).hexdigest())
        + ",'KEY_ADMIN','ENABLED',clock_timestamp())",
        "INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES("
        + literal(ids['retireMember']) + ',' + literal(environment['phase21']['ids']['otherUser'])
        + ',' + literal(environment['ids']['organization']) + ",'EMPLOYEE')",
    ]
    passwords = {}
    for kind, suffix in (('identity', 'identity_runtime'), ('platform', 'platform_runtime'), ('portal', 'portal_runtime')):
        role = prefix + '_' + suffix
        password = secrets.token_hex(32)
        passwords[kind] = {'role': role, 'passwordFile': str(TASK / ('phase23-source-' + kind + '-password'))}
        core.private_write(Path(passwords[kind]['passwordFile']), password.encode())
        statements.append('ALTER ROLE "' + role + '" PASSWORD ' + literal(password))
        current = environment[kind]['DATABASE_URL']
        environment[kind]['DATABASE_URL'] = 'postgres://' + role + ':' + password + '@' + current.split('@', 1)[1]
    psql(database, 'BEGIN; SET LOCAL password_encryption=\'scram-sha-256\';\n'
         + ';\n'.join(statements) + '; COMMIT;')
    for kid in ('phase23-b', 'phase23-c', 'phase23-d'):
        key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
        public = key.public_key().public_numbers()
        signing['keys'].append({'kid': kid, 'n': b64(public.n.to_bytes(256, 'big')), 'e': 'AQAB',
            'privateKeyPem': key.private_bytes(serialization.Encoding.PEM,
                serialization.PrivateFormat.PKCS8, serialization.NoEncryption()).decode()})
    core.private_write(signing_path, signing, replace=True)
    environment['identity'].update({'TRUSTED_PROXY_IPS': '127.0.0.1',
        'IDENTITY_METRICS_TOKEN': secrets.token_hex(32), 'AUTH_SOURCE_REQUESTS': '100000',
        'AUTH_CLIENT_REQUESTS': '100000', 'IDENTITY_DB_POOL_MAX_CONNECTIONS': '8',
        'AUTH_DELIVERY_TEST_ORIGINS': 'https://console.saas.test,https://customer.saas.test,https://auth.saas.test:24444,https://customer.saas.test:24446'})
    environment['platform']['DB_MAX_CONNECTIONS'] = '8'
    environment['platform']['MEMBERSHIP_LIFECYCLE_ENABLED'] = 'true'
    environment['platform']['IDENTITY_KEY_ADMIN_SERVICE_CLIENT_ID'] = ids['keyAdminService']
    environment['platform']['IDENTITY_KEY_ADMIN_SERVICE_CLIENT_SECRET'] = admin_secret
    environment['phase23'] = {'contractVersion': 'unified-auth-phase23/1', 'ids': ids,
        'signingKids': {'a': 'owned-https', 'b': 'phase23-b', 'c': 'phase23-c', 'd': 'phase23-d'},
        'rpOrigin': 'https://customer.saas.test:24446', 'normalTokenTtlSeconds': 300,
        'keyRetirementRequestedProfileSeconds': 30, 'longCarrierRequestedProfileSeconds': 600}
    core.private_write(TASK / 'phase23-source-db-passwords.json', passwords)
    core.private_write(TASK / 'https-environment.json', environment, replace=True)
    core.private_write(TASK / 'phase23-source-environment.json', environment)
    core.private_write(TASK / 'phase23-public.json',
        {key: environment['phase21'][key] for key in ('rp', 'machine', 'resource')})
    resources = {'contractVersion': 'unified-auth-phase23-resources/1', 'clusterId': cluster,
        'deploymentId': deployment, 'source': manifest, 'targets': {
            'platform': {'database': name.replace('unified_auth_https_', 'unified_auth_restore_'),
                         'owner': prefix + '_migration_owner', 'marker': 'phase23-recovery/2/' + deployment},
            'portal': {'database': name.replace('unified_auth_https_', 'unified_auth_restore_portal_'),
                       'owner': prefix + '_portal_owner', 'marker': 'phase23-recovery/2/' + deployment}},
        'rolesSharedOnlyWithinThisTask': True, 'legacyTablesVerifiedEmpty': list(legacy)}
    core.private_write(TASK / 'phase23-resources.json', resources)
    print('PHASE23_OWNED_FIXTURE_READY keys=4 runtime_passwords=3 legacy_state=empty')


if __name__ == '__main__':
    try:
        main()
    except Exception:
        print('PHASE23_FIXTURE_REJECTED', file=sys.stderr)
        raise SystemExit(1) from None
