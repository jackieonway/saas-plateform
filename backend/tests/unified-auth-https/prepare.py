"""Prepare an owned, fresh database for the real HTTPS authentication suite.

Run in the task's WSL environment. No existing business database is accepted.
The caller owns the short-lived directory and invokes cleanup after services stop.
Passwords and keys are written only to mode-0600 files inside that directory.
"""

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import time
from urllib.parse import urlparse
import uuid

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.hazmat.primitives.kdf.argon2 import Argon2id

import phase23_cluster_binding as cluster_binding


PHASE19_PERMISSIONS = ("view", "register", "review", "publish", "rotate",
                       "disable", "scopes", "invite", "keys", "assign")
PHASE19_ROLES = {
    "applicant": ("view", "register", "assign"),
    "reviewer": ("view", "review", "invite"),
    "publisher": ("view", "publish", "disable"),
}
MIGRATION_ERROR_CODES = frozenset(('MIGRATION_ARGUMENTS_INVALID', 'MIGRATION_SEED_NOT_SUPPORTED',
    'MIGRATION_DATABASE_URL_INVALID', 'MIGRATION_AMBIENT_PG_CONFIGURATION_UNSUPPORTED',
    'MIGRATION_DATABASE_CONNECTION_FAILED', 'MIGRATION_DATABASE_INITIALIZATION_FAILED',
    'MIGRATION_DATABASE_INITIALIZATION_TIMED_OUT', 'MIGRATION_DATABASE_CLOSE_TIMED_OUT'))


def migration_environment(database_url):
    # Product migrators require an explicit target and reject every ambient
    # libpq option. Do not remove PGOPTIONS from the caller's psql environment.
    return {**{key: value for key, value in os.environ.items() if not key.upper().startswith('PG')},
            'DATABASE_URL': database_url}


def migration_failure(result, portal=False):
    # Product migration diagnostics have fixed public codes. Never include raw
    # stderr, a SQL statement, a URL or a merely MIGRATION-prefixed unknown token.
    raw = result.stderr[:65536]
    text = raw.decode('ascii', errors='ignore') if isinstance(raw, bytes) else raw
    codes = re.findall(r'(?<![A-Z0-9_])MIGRATION_[A-Z_]+(?![A-Z0-9_])', text)
    code = next((value for value in codes if value in MIGRATION_ERROR_CODES), None)
    return RuntimeError('current ' + ('portal ' if portal else '') + 'migration command failed'
                        + (' (' + code + ')' if code else ''))


def resolve_backend_source(task, requested, require_frozen):
    """Bind Phase19 source to an explicit task snapshot, including frozen callers.

    A script executing from that same snapshot is valid. Its own parent path
    cannot identify a live workspace: frozen copies have the same layout.
    """
    script_backend = Path(__file__).resolve().parents[2]
    backend = requested.resolve(strict=True) if requested else script_backend
    if backend != script_backend and not backend.is_relative_to(task):
        raise RuntimeError('backend source must be the workspace or an owned snapshot')
    if not (backend / 'Cargo.toml').is_file() or not (backend / 'ops/identity-deployment/provision.sql').is_file():
        raise RuntimeError('complete backend source snapshot required')
    if not require_frozen:
        return backend
    if requested is None or not requested.is_absolute() or requested != backend or not backend.is_relative_to(task):
        raise RuntimeError('Phase 19 requires an explicit owned frozen backend source')
    parts = backend.relative_to(task).parts
    match = re.fullmatch(r'source-snapshot-([A-Za-z0-9_-]+)', parts[0]) if len(parts) == 3 else None
    if match is None or parts[1:] != ('saas-plateform', 'backend'):
        raise RuntimeError('Phase 19 backend must use an exact owned snapshot layout')
    snapshot = task / parts[0]
    manifest_path = task / ('snapshot-' + match.group(1) + '.json')
    if snapshot.resolve(strict=True) != snapshot or manifest_path.resolve(strict=True) != manifest_path:
        raise RuntimeError('Phase 19 source snapshot links are rejected')
    owner = task.stat().st_uid
    raw, _ = cluster_binding.protected_read(manifest_path, uid=owner, mode=0o600, maximum=8 * 1024 * 1024)
    manifest = json.loads(raw)
    inputs = manifest.get('inputs') if type(manifest) is dict else None
    if (type(inputs) is not dict or not inputs or any(type(name) is not str
            or Path(name).is_absolute() or '..' in Path(name).parts or type(digest) is not str
            or re.fullmatch(r'[0-9a-f]{64}', digest) is None for name, digest in inputs.items())
            or manifest.get('snapshotSha256') != hashlib.sha256(json.dumps(inputs, sort_keys=True).encode()).hexdigest()):
        raise RuntimeError('Phase 19 source snapshot manifest is invalid')
    required = ('Cargo.toml', 'migrations/0001_init.sql', 'ops/identity-deployment/create-roles.sql',
        'ops/identity-deployment/provision.sql', 'ops/identity-deployment/verify.sql',
        'tests/unified-auth-https/prepare.py')
    for relative in required:
        filename = backend / relative
        recorded = inputs.get('saas-plateform/backend/' + relative)
        if filename.resolve(strict=True) != filename or recorded is None:
            raise RuntimeError('Phase 19 source snapshot inputs are incomplete')
        data, _ = cluster_binding.protected_read(filename, uid=owner, mode=0o600, maximum=8 * 1024 * 1024)
        if hashlib.sha256(data).hexdigest() != recorded:
            raise RuntimeError('Phase 19 source snapshot input changed')
    return backend


def new_id():
    value = ((int(time.time() * 1000) << 80) | (7 << 76)
             | (secrets.randbits(12) << 64) | (2 << 62) | secrets.randbits(62))
    return str(uuid.UUID(int=value))


def literal(value):
    return "'" + str(value).replace("'", "''") + "'"


def b64(value):
    return base64.urlsafe_b64encode(value).decode().rstrip("=")


def psql(url, sql):
    options = {}
    parsed = urlparse(url)
    if parsed.username != 'postgres':
        # Only the fixed real-LOGIN identity probe uses a runtime connection.
        # Privileged PGOPTIONS belong to the admin writes; a runtime cannot set
        # their SUSET GUCs and must never receive arbitrary SQL or secret writes.
        role = parsed.username or ''
        match = re.fullmatch(r'uah_([0-9a-f]{12})_(identity|platform|portal)_runtime', role)
        database = parsed.path.removeprefix('/')
        expected_kind = 'portal' if match and match.group(2) == 'portal' else 'https'
        if (match is None or parsed.scheme not in ('postgres', 'postgresql')
                or parsed.hostname != '127.0.0.1' or parsed.port != 55494
                or parsed.password is None or parsed.query or parsed.fragment
                or re.fullmatch('unified_auth_' + expected_kind + r'_[0-9a-f]{32}', database) is None
                or database[-12:] != match.group(1)
                or sql != 'SELECT current_user=session_user AND current_user=' + literal(role)):
            raise RuntimeError('owned HTTPS runtime connection only permits its fixed identity probe')
        options['env'] = {key: value for key, value in os.environ.items() if not key.upper().startswith('PG')}
        options['env']['PGOPTIONS'] = '-c default_transaction_read_only=on'
    result = subprocess.run(
        ["psql", "-X", "-v", "ON_ERROR_STOP=1", "-v", "VERBOSITY=sqlstate", "-At", url],
        input=sql, text=True, capture_output=True, timeout=45, **options)
    if result.returncode:
        # PostgreSQL diagnostics can contain a query with credential material.
        code = re.search(r"(?:ERROR|FATAL):\s+([A-Z0-9]{5})\b", result.stderr)
        raise RuntimeError("owned HTTPS fixture SQL failed" +
                           (" (SQLSTATE " + code.group(1) + ")" if code else ""))
    return result.stdout.strip()


def save(path, value):
    path.write_text(json.dumps(value, indent=2), encoding="utf-8")
    path.chmod(0o600)


def cleanup(admin_url, manifest):
    name, prefix = manifest["database"], manifest["rolePrefix"]
    if not re.fullmatch(r"unified_auth_https_[0-9a-f]{32}", name):
        raise RuntimeError("invalid owned database name")
    if not re.fullmatch(r"uah_[0-9a-f]{12}", prefix):
        raise RuntimeError("invalid owned role prefix")
    if prefix != "uah_" + name[-12:]:
        raise RuntimeError("owned role prefix does not match database")
    actual = psql(admin_url, "SELECT system_identifier FROM pg_control_system()")
    if actual != manifest["clusterId"]:
        raise RuntimeError("owned cluster identity changed")
    portal = manifest.get("portal")
    if portal is not None:
        expected_portal = "unified_auth_portal_" + name.removeprefix("unified_auth_https_")
        if (portal.get("database") != expected_portal
                or portal.get("owner") != prefix + "_portal_owner"
                or portal.get("runtime") != prefix + "_portal_runtime"
                or not re.fullmatch(r"unified-auth-https-portal/1/[0-9a-f-]{36}", portal.get("marker", ""))):
            raise RuntimeError("invalid owned portal resource manifest")
        try:
            deployment = uuid.UUID(portal["marker"].rsplit("/", 1)[1])
        except ValueError as error:
            raise RuntimeError("invalid portal deployment marker") from error
        if deployment.version != 7 or str(deployment) != portal["marker"].rsplit("/", 1)[1]:
            raise RuntimeError("portal deployment marker must use canonical UUIDv7")
        rows = psql(admin_url, "SELECT COALESCE(shobj_description(oid,'pg_database'),'') || '|' || "
                    "pg_get_userbyid(datdba) FROM pg_database WHERE datname=" + literal(expected_portal))
        if rows:
            comment, owner = rows.rsplit("|", 1)
            if (comment != portal["marker"]
                    and not (not comment and portal.get("initializationPending") is True)):
                raise RuntimeError("portal database ownership marker mismatch")
            if owner not in (urlparse(admin_url).username, portal["owner"]):
                raise RuntimeError("portal database owner mismatch")
            psql(admin_url, f'DROP DATABASE "{expected_portal}" WITH (FORCE);')
        for role in (portal["runtime"], portal["owner"]):
            comment = psql(admin_url, "SELECT COALESCE(shobj_description(oid,'pg_authid'),'') "
                           "FROM pg_roles WHERE rolname=" + literal(role))
            present = psql(admin_url, "SELECT EXISTS(SELECT 1 FROM pg_roles WHERE rolname=" + literal(role) + ")")
            if present == "t" and comment != portal["marker"]:
                raise RuntimeError("portal role ownership marker mismatch")
            psql(admin_url, f'DROP ROLE IF EXISTS "{role}";')
    psql(admin_url, f'DROP DATABASE IF EXISTS "{name}" WITH (FORCE);')
    for suffix in ("identity_runtime", "platform_runtime", "identity_owner",
                   "platform_owner", "bridge_owner", "migration_owner"):
        psql(admin_url, f'DROP ROLE IF EXISTS "{prefix}_{suffix}";')


def prepare_portal_database(admin_url, shared_name, portal, migration_bin, password):
    """Initialize only the planned empty portal DB, then seal owner/runtime ACLs."""
    name, owner, runtime, marker = (portal[key] for key in ("database", "owner", "runtime", "marker"))
    shared_prefix = owner.removesuffix("_portal_owner")
    psql(admin_url, f'''BEGIN;
CREATE ROLE "{owner}" NOLOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
CREATE ROLE "{runtime}" LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD {literal(password)};
COMMENT ON ROLE "{owner}" IS {literal(marker)};
COMMENT ON ROLE "{runtime}" IS {literal(marker)};
COMMIT;
CREATE DATABASE "{name}";
COMMENT ON DATABASE "{name}" IS {literal(marker)};
''')
    url = admin_url.rsplit("/", 1)[0] + "/" + name
    result = subprocess.run([str(migration_bin)], env=migration_environment(url),
                            capture_output=True, timeout=60)
    if result.returncode:
        raise migration_failure(result, portal=True)
    psql(url, f'''BEGIN;
ALTER DATABASE "{name}" OWNER TO "{owner}";
ALTER SCHEMA public OWNER TO "{owner}";
REVOKE ALL ON DATABASE "{name}" FROM PUBLIC;
REVOKE ALL ON SCHEMA public FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA public FROM PUBLIC;
REVOKE ALL ON ALL FUNCTIONS IN SCHEMA public FROM PUBLIC;
GRANT CONNECT ON DATABASE "{name}" TO "{runtime}";
GRANT USAGE ON SCHEMA public TO "{runtime}";
ALTER TABLE public._sqlx_migrations OWNER TO "{owner}";
ALTER TABLE public.portal_browsers OWNER TO "{owner}";
ALTER TABLE public.portal_login_flows OWNER TO "{owner}";
ALTER TABLE public.portal_sessions OWNER TO "{owner}";
ALTER TABLE public.portal_backchannel_logout_receipts OWNER TO "{owner}";
ALTER TABLE public.portal_backchannel_logout_sources OWNER TO "{owner}";
ALTER FUNCTION public.is_uuid_v7(uuid) OWNER TO "{owner}";
ALTER FUNCTION public.portal_public_source_immutable() OWNER TO "{owner}";
GRANT SELECT,INSERT,UPDATE,DELETE ON public.portal_browsers,public.portal_login_flows,public.portal_sessions TO "{runtime}";
GRANT SELECT,INSERT ON public.portal_backchannel_logout_receipts,public.portal_backchannel_logout_sources TO "{runtime}";
GRANT EXECUTE ON FUNCTION public.is_uuid_v7(uuid) TO "{runtime}";
ALTER DEFAULT PRIVILEGES FOR ROLE "{owner}" IN SCHEMA public REVOKE ALL ON TABLES FROM PUBLIC;
ALTER DEFAULT PRIVILEGES FOR ROLE "{owner}" IN SCHEMA public REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC;
COMMIT;
''')
    verified = psql(url, f'''SELECT
    (SELECT array_agg(tablename::text ORDER BY tablename) FROM pg_tables WHERE schemaname='public')
       =ARRAY['_sqlx_migrations','portal_backchannel_logout_receipts','portal_backchannel_logout_sources','portal_browsers','portal_login_flows','portal_sessions']::text[]
    AND (SELECT count(*) FROM information_schema.columns WHERE table_schema='public' AND udt_name='uuid')=20
    AND NOT has_database_privilege({literal(runtime)},{literal(shared_name)},'CONNECT')
    AND NOT has_database_privilege({literal(shared_prefix + '_identity_runtime')},current_database(),'CONNECT')
    AND NOT has_database_privilege({literal(shared_prefix + '_platform_runtime')},current_database(),'CONNECT')
    AND NOT has_database_privilege({literal(runtime)},current_database(),'CREATE,TEMP')
    AND NOT has_schema_privilege({literal(runtime)},'public','CREATE')
    AND NOT has_table_privilege({literal(runtime)},'public._sqlx_migrations','SELECT,INSERT,UPDATE,DELETE')
    AND NOT pg_has_role({literal(runtime)},{literal(owner)},'MEMBER')
    AND has_function_privilege({literal(runtime)},'public.is_uuid_v7(uuid)','EXECUTE')
    AND NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname={literal(runtime)}
       AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls OR rolinherit))
    AND NOT EXISTS(SELECT 1 FROM pg_auth_members m JOIN pg_roles r ON r.oid=m.member
       WHERE r.rolname={literal(runtime)})
    AND NOT EXISTS(SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
       WHERE n.nspname='public' AND pg_get_userbyid(c.relowner)={literal(runtime)})
    AND NOT EXISTS(SELECT 1 FROM unnest(ARRAY['portal_browsers','portal_login_flows','portal_sessions']) t(name)
       CROSS JOIN unnest(ARRAY['SELECT','INSERT','UPDATE','DELETE']) p(privilege)
       WHERE NOT has_table_privilege({literal(runtime)},'public.'||t.name,p.privilege))
    AND NOT EXISTS(SELECT 1 FROM unnest(ARRAY['portal_backchannel_logout_receipts','portal_backchannel_logout_sources']) t(name)
       CROSS JOIN unnest(ARRAY['SELECT','INSERT']) p(privilege)
       WHERE NOT has_table_privilege({literal(runtime)},'public.'||t.name,p.privilege))
    AND NOT EXISTS(SELECT 1 FROM unnest(ARRAY['portal_backchannel_logout_receipts','portal_backchannel_logout_sources']) t(name)
       CROSS JOIN unnest(ARRAY['UPDATE','DELETE']) p(privilege)
       WHERE has_table_privilege({literal(runtime)},'public.'||t.name,p.privilege));
''')
    if verified != "t":
        raise RuntimeError("portal runtime isolation verification failed")
    parsed = urlparse(url)
    runtime_url = parsed._replace(netloc=f"{runtime}:{password}@{parsed.hostname}:{parsed.port}").geturl()
    if psql(runtime_url, "SELECT current_user=session_user AND current_user=" + literal(runtime)) != "t":
        raise RuntimeError("portal runtime requires its real LOGIN identity")
    return runtime_url


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--task-dir", type=Path, required=True)
    parser.add_argument("--admin-url", required=True)
    parser.add_argument("--cluster-id", required=True)
    parser.add_argument("--migration-bin", type=Path)
    parser.add_argument("--backend-source", type=Path,
                        help="Frozen backend source supplying matching role SQL")
    parser.add_argument("--tls-ca-file", type=Path,
                        help="Explicit public CA bundle when executing this script from a frozen snapshot")
    parser.add_argument("--include-portal", action="store_true")
    parser.add_argument("--include-phase19", action="store_true",
                        help="Add isolated review/invitation services and distinct human roles")
    parser.add_argument("--include-phase20", action="store_true",
                        help="Add owner-verified RP delivery targets and the isolated collector")
    parser.add_argument("--portal-migration-bin", type=Path)
    parser.add_argument("--cleanup", action="store_true")
    args = parser.parse_args()
    url = urlparse(args.admin_url)
    if (url.scheme not in ("postgres", "postgresql") or url.hostname != "127.0.0.1"
            or url.port != 55494 or url.path != "/postgres" or url.query or url.fragment):
        raise RuntimeError("explicit owned loopback cluster required")
    task = args.task_dir.resolve(strict=True)
    if not task.is_relative_to(Path("/tmp/rust-saas-unified-auth-20261028")):
        raise RuntimeError("task directory outside owned environment")
    marker = task / "https-database.json"
    if args.cleanup:
        cleanup(args.admin_url, json.loads(marker.read_text()))
        marker.unlink()
        print("HTTPS_FIXTURE_DATABASE_AND_ROLES_REMOVED")
        return
    if marker.exists() or not args.migration_bin or not args.migration_bin.is_file():
        raise RuntimeError("fresh fixture and current migration binary required")
    if args.include_portal and (not args.portal_migration_bin or not args.portal_migration_bin.is_file()):
        raise RuntimeError("current portal migration binary required")
    if args.include_phase20 and not (args.include_phase19 and args.include_portal):
        raise RuntimeError("Phase 20 requires explicit Phase 19 and portal fixture options")
    actual = psql(args.admin_url, "SELECT system_identifier FROM pg_control_system()")
    if actual != args.cluster_id:
        raise RuntimeError("owned cluster identity mismatch")
    backend = resolve_backend_source(task, args.backend_source, args.include_phase19)
    token = uuid.UUID(new_id()).hex
    name, prefix = f"unified_auth_https_{token}", f"uah_{token[-12:]}"
    occupied = psql(args.admin_url,
        "SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname=" + literal(name)
        + ") OR EXISTS(SELECT 1 FROM pg_roles WHERE left(rolname," + str(len(prefix) + 1)
        + ")=" + literal(prefix + "_") + ")")
    if occupied != "f":
        raise RuntimeError("task resource identifiers must be unused")
    if args.include_portal:
        portal_name = f"unified_auth_portal_{token}"
        if psql(args.admin_url, "SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname="
                + literal(portal_name) + ")") != "f":
            raise RuntimeError("portal database identifier must be unused")
    manifest = {"database": name, "rolePrefix": prefix, "clusterId": actual}
    save(marker, manifest)
    database_url = args.admin_url.rsplit("/", 1)[0] + "/" + name
    try:
        psql(args.admin_url, f'CREATE DATABASE "{name}";')
        result = subprocess.run([str(args.migration_bin)], env=migration_environment(database_url),
            capture_output=True, timeout=60)
        if result.returncode:
            raise migration_failure(result)
        if args.include_portal and psql(database_url,
                "SELECT to_regclass('public.platform_resource_services') IS NOT NULL") != "t":
            raise RuntimeError("portal fixture requires the current Phase 15+ resource-service baseline")
        if args.include_phase19 and psql(database_url,
                "SELECT to_regclass('public.auth_invitations') IS NOT NULL "
                "AND to_regclass('public.platform_auth_review_operations') IS NOT NULL "
                "AND to_regprocedure('public.identity_admin_assurance(uuid,uuid,uuid,bigint,uuid,bigint,text)') IS NOT NULL "
                "AND to_regprocedure('public.identity_auth_review_intent_active(uuid,uuid,text,text,text,jsonb)') IS NOT NULL") != "t":
            raise RuntimeError("Phase 19 fixture requires the current review, invitation and assurance baseline")
        if args.include_phase20 and psql(database_url,
                "SELECT to_regclass('public.auth_delivery_targets') IS NOT NULL "
                "AND to_regclass('public.auth_delivery_jobs') IS NOT NULL "
                "AND to_regclass('public.auth_rp_sessions') IS NOT NULL "
                "AND to_regclass('public.auth_invitation_verifications') IS NOT NULL") != "t":
            raise RuntimeError("Phase 20 requires current canonical logout and trusted delivery facts")
        ids = {key: new_id() for key in ("tenant", "organization", "secondOrganization", "adminMenu",
            "application", "client", "group", "resource", "openid", "profile", "sector",
            "admin", "employee", "customer", "adminMember", "employeeMember", "customerMember",
            "sourceService", "accountService", "selectionService", "deployment")}
        if args.include_phase19:
            phase19_ids = ("oauthMenu", "reviewService", "invitationService", "authAdminService")
            phase19_ids += tuple(account + suffix for account in PHASE19_ROLES
                                 for suffix in ("", "Member", "Role"))
            phase19_ids += tuple("oauth" + permission.title() + "Menu"
                                 for permission in PHASE19_PERMISSIONS)
            ids.update({key: new_id() for key in phase19_ids})
        if args.include_portal:
            ids.update({key: new_id() for key in ("portalApplication", "portalClient", "portalGroup",
                "portalSector", "portalSourceService", "customerAuthorityService", "portalConsumerService",
                "profileClient", "adminCustomerMember")})
            manifest["portal"] = {"database": portal_name, "owner": prefix + "_portal_owner",
                "runtime": prefix + "_portal_runtime", "marker": "unified-auth-https-portal/1/" + ids["deployment"],
                "initializationPending": True}
            save(marker, manifest)
        if args.include_phase20:
            ids.update({key: new_id() for key in ("consoleBackchannelTarget", "portalBackchannelTarget",
                "invitationChannel", "consoleTargetVerification", "portalTargetVerification", "channelVerification")})
        secret = {key: secrets.token_hex(32) for key in
                  ("client", "source", "account", "selection", "jwt")}
        if args.include_phase19:
            secret.update({key: secrets.token_hex(32) for key in ("review", "invitation", "authAdmin")})
        if args.include_portal:
            secret.update({key: secrets.token_hex(32) for key in ("portalClient", "portalSource",
                "customerAuthority", "portalConsumer", "profile", "portalDatabase")})
        if args.include_phase20:
            secret["collectorControl"] = secrets.token_hex(32)
        password = "Owned-Https-" + secrets.token_urlsafe(18) + "9!"
        phc = Argon2id(salt=os.urandom(16), length=32, iterations=2,
                       lanes=1, memory_cost=19456).derive_phc_encoded(password.encode())
        account_types = [("admin", "EMPLOYEE"), ("employee", "EMPLOYEE"), ("customer", "CUSTOMER")]
        account_passwords = {account: password for account, _ in account_types}
        account_hashes = {account: phc for account, _ in account_types}
        if args.include_phase19:
            for account in PHASE19_ROLES:
                actor_password = "Owned-Https-" + secrets.token_urlsafe(18) + "9!"
                account_types.append((account, "EMPLOYEE"))
                account_passwords[account] = actor_password
                account_hashes[account] = Argon2id(salt=os.urandom(16), length=32, iterations=2,
                    lanes=1, memory_cost=19456).derive_phc_encoded(actor_password.encode())
        q = {key: literal(value) for key, value in ids.items()}
        statements = [
            f"INSERT INTO tenants(id,name,code,is_platform) VALUES({q['tenant']},'HTTPS owned tenant','https-owned',true)",
            f"INSERT INTO organizations(id,tenant_id,name) VALUES({q['organization']},{q['tenant']},'HTTPS headquarters')",
            f"INSERT INTO organizations(id,tenant_id,parent_id,name) VALUES({q['secondOrganization']},{q['tenant']},{q['organization']},'HTTPS branch')",
            f"INSERT INTO applications(id,code,name,allowed_member_types) VALUES({q['application']},'https-console','HTTPS console',ARRAY['EMPLOYEE'])",
            f"INSERT INTO tenant_applications(tenant_id,application_id) VALUES({q['tenant']},{q['application']})",
            f"INSERT INTO organization_applications(organization_id,application_id) VALUES({q['organization']},{q['application']}),({q['secondOrganization']},{q['application']})",
            f"INSERT INTO menus(id,application_id,type,title,permission,path) VALUES({q['adminMenu']},{q['application']},'MENU','HTTPS account administration','system:user','/system/users')",
            f"INSERT INTO tenant_menus(tenant_id,menu_id) VALUES({q['tenant']},{q['adminMenu']})",
            f"INSERT INTO organization_menus(organization_id,menu_id) VALUES({q['organization']},{q['adminMenu']}),({q['secondOrganization']},{q['adminMenu']})",
            f"INSERT INTO auth_applications(application_id,ownership,status,display_name,purpose,approved_scopes) VALUES({q['application']},'FIRST_PARTY','APPROVED','HTTPS console','PLATFORM_CONSOLE',ARRAY['openid','profile'])",
            f"INSERT INTO auth_resources(id,resource_key,audience,status) VALUES({q['resource']},'userinfo','https://auth.saas.test/oauth/userinfo','ACTIVE')",
            f"INSERT INTO auth_scopes(id,resource_id,scope_name,status) VALUES({q['openid']},{q['resource']},'openid','ACTIVE'),({q['profile']},{q['resource']},'profile','ACTIVE')",
            f"INSERT INTO auth_clients(id,client_id,display_name,application_id,environment,client_type,runtime_kind,subject_mode,context_mode,allowed_member_types,grant_types,token_endpoint_auth_method,redirect_uris,post_logout_redirect_uris,status) VALUES({q['client']},{q['client']},'HTTPS console',{q['application']},'test','CONFIDENTIAL','WEB_BFF','USER','PLATFORM_CONTEXT',ARRAY['EMPLOYEE'],ARRAY['authorization_code','refresh_token'],'client_secret_basic',ARRAY['https://console.saas.test/auth/callback'],ARRAY[]::text[],'ENABLED')",
            f"INSERT INTO auth_client_credentials(id,client_id,credential_version,secret_digest) VALUES({literal(new_id())},{q['client']},1,{literal(hashlib.sha256(secret['client'].encode()).hexdigest())})",
            f"INSERT INTO auth_client_permissions(client_id,resource_id,scope_id) VALUES({q['client']},{q['resource']},{q['openid']}),({q['client']},{q['resource']},{q['profile']})",
            f"INSERT INTO auth_consent_groups(id,application_id,environment,display_name,purpose,context_mode,status) VALUES({q['group']},{q['application']},'test','HTTPS console consent','PLATFORM_CONSOLE','PLATFORM_CONTEXT','ACTIVE')",
            f"INSERT INTO auth_consent_group_clients(group_id,client_id,application_id,environment,context_mode) VALUES({q['group']},{q['client']},{q['application']},'test','PLATFORM_CONTEXT')",
            f"INSERT INTO auth_subject_sectors(id,application_id,environment,status) VALUES({q['sector']},{q['application']},'test','ACTIVE')",
            f"INSERT INTO identity_service_clients(id,secret_hash,purpose,status,allowed_auth_clients,created_at) VALUES({q['sourceService']},{literal(hashlib.sha256(secret['source'].encode()).hexdigest())},'CONTEXT_SELECTION_SOURCE','ENABLED',ARRAY[{q['client']}::uuid],clock_timestamp()),({q['accountService']},{literal(hashlib.sha256(secret['account'].encode()).hexdigest())},'ACCOUNT_COMMAND','ENABLED',ARRAY[]::uuid[],clock_timestamp())",
            f"INSERT INTO context_selection_services(id,secret_hash,purpose,allowed_auth_clients,status) VALUES({q['selectionService']},{literal(hashlib.sha256(secret['selection'].encode()).hexdigest())},'CONTEXT_SELECTION_AUTHORITY',ARRAY[{q['client']}::uuid],'ENABLED')",
            f"INSERT INTO context_selection_registrations(client_id,client_registry,client_policy_version,member_type,application_id,application_policy_version) VALUES({q['client']},'AUTH',1,'EMPLOYEE',{q['application']},1)",
            "INSERT INTO auth_provider_settings(singleton,issuer,created_at) VALUES(true,'https://auth.saas.test',clock_timestamp())",
        ]
        if args.include_portal:
            # A private profile service is independent of the canonical RP and
            # cannot issue OAuth tokens or read another resource.
            profile_public = "https-profile-" + uuid.UUID(ids["profileClient"]).hex
            statements += [
                f"INSERT INTO applications(id,code,name,base_path,allowed_member_types) VALUES({q['portalApplication']},'https-customer','HTTPS customer portal','/customer',ARRAY['CUSTOMER'])",
                f"INSERT INTO tenant_applications(tenant_id,application_id) VALUES({q['tenant']},{q['portalApplication']})",
                f"INSERT INTO organization_applications(organization_id,application_id) VALUES({q['organization']},{q['portalApplication']}),({q['secondOrganization']},{q['portalApplication']})",
                f"INSERT INTO auth_applications(application_id,ownership,status,display_name,purpose,approved_scopes) VALUES({q['portalApplication']},'FIRST_PARTY','APPROVED','HTTPS customer portal','CUSTOMER_PORTAL',ARRAY['openid','profile'])",
                f"INSERT INTO auth_clients(id,client_id,display_name,application_id,environment,client_type,runtime_kind,subject_mode,context_mode,allowed_member_types,grant_types,token_endpoint_auth_method,redirect_uris,post_logout_redirect_uris,status) VALUES({q['portalClient']},{q['portalClient']},'HTTPS customer portal',{q['portalApplication']},'test','CONFIDENTIAL','WEB_BFF','USER','PLATFORM_CONTEXT',ARRAY['CUSTOMER'],ARRAY['authorization_code','refresh_token'],'client_secret_basic',ARRAY['https://customer.saas.test/auth/callback'],ARRAY[]::text[],'ENABLED')",
                f"INSERT INTO auth_client_credentials(id,client_id,credential_version,secret_digest) VALUES({literal(new_id())},{q['portalClient']},1,{literal(hashlib.sha256(secret['portalClient'].encode()).hexdigest())})",
                f"INSERT INTO auth_client_permissions(client_id,resource_id,scope_id) VALUES({q['portalClient']},{q['resource']},{q['openid']}),({q['portalClient']},{q['resource']},{q['profile']})",
                f"INSERT INTO auth_consent_groups(id,application_id,environment,display_name,purpose,context_mode,status) VALUES({q['portalGroup']},{q['portalApplication']},'test','HTTPS customer consent','CUSTOMER_PORTAL','PLATFORM_CONTEXT','ACTIVE')",
                f"INSERT INTO auth_consent_group_clients(group_id,client_id,application_id,environment,context_mode) VALUES({q['portalGroup']},{q['portalClient']},{q['portalApplication']},'test','PLATFORM_CONTEXT')",
                f"INSERT INTO auth_subject_sectors(id,application_id,environment,status) VALUES({q['portalSector']},{q['portalApplication']},'test','ACTIVE')",
                f"INSERT INTO identity_service_clients(id,secret_hash,purpose,status,allowed_auth_clients,created_at) VALUES({q['portalSourceService']},{literal(hashlib.sha256(secret['portalSource'].encode()).hexdigest())},'CONTEXT_SELECTION_SOURCE','ENABLED',ARRAY[{q['portalClient']}::uuid],clock_timestamp()),({q['customerAuthorityService']},{literal(hashlib.sha256(secret['customerAuthority'].encode()).hexdigest())},'CUSTOMER_ACCESS_PROOF','ENABLED',ARRAY[{q['portalClient']}::uuid],clock_timestamp())",
                f"UPDATE identity_service_clients SET allowed_auth_clients=ARRAY[{q['client']}::uuid,{q['portalClient']}::uuid] WHERE id={q['sourceService']}",
                f"UPDATE context_selection_services SET allowed_auth_clients=ARRAY[{q['client']}::uuid,{q['portalClient']}::uuid] WHERE id={q['selectionService']}",
                f"INSERT INTO context_selection_services(id,secret_hash,purpose,allowed_auth_clients,status) VALUES({q['portalConsumerService']},{literal(hashlib.sha256(secret['portalConsumer'].encode()).hexdigest())},'CONTEXT_SELECTION_CONSUMER',ARRAY[{q['portalClient']}::uuid],'ENABLED')",
                f"INSERT INTO context_selection_registrations(client_id,client_registry,client_policy_version,member_type,application_id,application_policy_version,consumer_service_id) VALUES({q['portalClient']},'AUTH',1,'CUSTOMER',{q['portalApplication']},1,{q['portalConsumerService']})",
                f"INSERT INTO platform_resource_services(id,service_id,secret_hash,purpose,application_id,audience,allowed_member_types,allowed_operations,status) SELECT {q['profileClient']},{literal(profile_public)},{literal(hashlib.sha256(secret['profile'].encode()).hexdigest())},'CUSTOMER_PROFILE',{q['portalApplication']},a.code,ARRAY['CUSTOMER'],ARRAY['customer:profile'],'ENABLED' FROM applications a WHERE a.id={q['portalApplication']}",
            ]
        for account, member_type in account_types:
            statements += [
                f"INSERT INTO users(id,username,password_hash,real_name,is_super_admin,session_version) VALUES({q[account]},{literal('https-' + account)},{literal(account_hashes[account])},{literal('HTTPS ' + account)},{str(account == 'admin').lower()},1)",
                f"INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES({q[account + 'Member']},{q[account]},{q['organization']},{literal(member_type)})",
            ]
        if args.include_phase19:
            statements += [
                f"INSERT INTO menus(id,application_id,type,title,permission,path) VALUES({q['oauthMenu']},{q['application']},'MENU','HTTPS OAuth management','system:oauth','/system/oauth/clients')",
                f"INSERT INTO tenant_menus(tenant_id,menu_id) VALUES({q['tenant']},{q['oauthMenu']})",
                f"INSERT INTO organization_menus(organization_id,menu_id) VALUES({q['organization']},{q['oauthMenu']})",
            ]
            for permission in PHASE19_PERMISSIONS:
                menu = q["oauth" + permission.title() + "Menu"]
                full_permission = literal("system:oauth:" + permission)
                statements += [
                    f"INSERT INTO menus(id,application_id,parent_id,type,title,permission) VALUES({menu},{q['application']},{q['oauthMenu']},'BUTTON',{full_permission},{full_permission})",
                    f"INSERT INTO tenant_menus(tenant_id,menu_id) VALUES({q['tenant']},{menu})",
                    f"INSERT INTO organization_menus(organization_id,menu_id) VALUES({q['organization']},{menu})",
                ]
            for account, permissions in PHASE19_ROLES.items():
                role = q[account + "Role"]
                statements += [
                    f"INSERT INTO roles(id,tenant_id,organization_id,name,code) VALUES({role},{q['tenant']},{q['organization']},{literal('HTTPS ' + account)},{literal('https-phase19-' + account)})",
                    f"INSERT INTO user_roles(user_id,role_id,organization_id) VALUES({q[account]},{role},{q['organization']})",
                    f"INSERT INTO role_menus(role_id,menu_id) VALUES({role},{q['oauthMenu']})",
                ]
                for permission in permissions:
                    statements.append(f"INSERT INTO role_menus(role_id,menu_id) VALUES({role},{q['oauth' + permission.title() + 'Menu']})")
            for service, credential, purpose in (("reviewService", "review", "AUTH_REVIEW"),
                    ("invitationService", "invitation", "AUTH_INVITATION"),
                    ("authAdminService", "authAdmin", "AUTH_ADMIN")):
                statements.append(f"INSERT INTO identity_service_clients(id,secret_hash,purpose,status,created_at) VALUES({q[service]},{literal(hashlib.sha256(secret[credential].encode()).hexdigest())},{literal(purpose)},'ENABLED',clock_timestamp())")
        if args.include_portal:
            statements.append(
                f"INSERT INTO memberships(id,user_id,organization_id,member_type) VALUES({q['adminCustomerMember']},{q['admin']},{q['organization']},'CUSTOMER')")
        if args.include_phase20:
            for client, target, verification, origin in (
                    ("client", "consoleBackchannelTarget", "consoleTargetVerification", "https://console.saas.test"),
                    ("portalClient", "portalBackchannelTarget", "portalTargetVerification", "https://customer.saas.test")):
                endpoint = origin + "/auth/backchannel-logout"
                statements += [
                    f"INSERT INTO auth_delivery_targets(id,purpose,status,client_id,url,allowed_ips,verified_reference,verified_at,expires_at) VALUES({q[target]},'BACK_CHANNEL','ACTIVE',{q[client]},{literal(endpoint)},ARRAY['127.0.0.2'],{q[verification]},clock_timestamp()-interval '1 second',clock_timestamp()+interval '4 hours')",
                    f"UPDATE auth_clients SET logout_mode='BACK_CHANNEL',backchannel_logout_uri={literal(endpoint)},backchannel_target_id={q[target]} WHERE id={q[client]}",
                    f"UPDATE context_selection_registrations SET client_policy_version=(SELECT policy_version FROM auth_clients WHERE id={q[client]}) WHERE client_id={q[client]}",
                ]
            statements.append(
                f"INSERT INTO auth_delivery_targets(id,purpose,status,recipient_ref,channel_purpose,url,allowed_ips,verified_reference,verified_at,expires_at) VALUES({q['invitationChannel']},'VERIFIED_CHANNEL','ACTIVE','phase20:owned:recipient','ACCOUNT_ONBOARDING','https://auth.saas.test:24444/internal/task-delivery/invitations',ARRAY['127.0.0.2'],{q['channelVerification']},clock_timestamp()-interval '1 second',clock_timestamp()+interval '4 hours')")
        key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
        public = key.public_key().public_numbers()
        modulus = b64(public.n.to_bytes(256, "big"))
        save(task / "https-signing.json", {"keys": [{"kid": "owned-https", "n": modulus,
            "e": "AQAB", "privateKeyPem": key.private_bytes(serialization.Encoding.PEM,
            serialization.PrivateFormat.PKCS8, serialization.NoEncryption()).decode()}]})
        statements.append("INSERT INTO auth_signing_keys(kid,n,e,published_at,active,activated_at) "
                          f"VALUES('owned-https',{literal(modulus)},'AQAB',clock_timestamp(),true,clock_timestamp())")
        psql(database_url, ";\n".join(statements) + ";")
        psql(database_url, f'COMMENT ON DATABASE "{name}" IS '
             + literal("identity-deployment/1/" + ids["deployment"]) + ";")
        values = {"expected_database": name, "role_prefix": prefix,
                  "deployment_id": ids["deployment"], "issuer": "https://auth.saas.test"}
        for filename in ("create-roles.sql", "provision.sql", "verify.sql"):
            source = (backend / "ops" / "identity-deployment" / filename).read_text()
            for key_name, value in values.items():
                source = source.replace(":'" + key_name + "'", literal(value))
            psql(database_url, source)
        portal_database_url = None
        if args.include_portal:
            portal_database_url = prepare_portal_database(args.admin_url, name, manifest["portal"],
                args.portal_migration_bin, secret["portalDatabase"])
            manifest["portal"]["initializationPending"] = False
            save(marker, manifest)
        for filename in ("https-bff-keys.json", "https-context-keys.json"):
            save(task / filename, {"activeKid": "owned", "keys": [
                {"kid": "owned", "key": b64(os.urandom(32))}]})
        if args.include_phase19:
            save(task / "https-invitation-keys.json", {"purpose": "saas-identity/auth-invitation/1",
                "activeVersion": 1, "keys": [{"version": 1, "key": b64(os.urandom(32))}]})
        if args.include_portal:
            save(task / "https-portal-keys.json", {"activeKid": "portal-owned", "keys": [
                {"kid": "portal-owned", "key": b64(os.urandom(32))}]})
        ca = str(args.tls_ca_file.resolve(strict=True) if args.tls_ca_file
                 else Path(__file__).resolve().parents[4] / ".local-run" / "ca.pem")
        identity = {"APP_ENV": "test", "BIND_ADDR": "127.0.0.1:23441", "RUST_LOG": "info",
            "DATABASE_URL": database_url.replace("postgres@", prefix + "_identity_runtime@"),
            "SSO_ENABLED": "true", "OIDC_ENABLED": "true", "SSO_ORIGIN": "https://auth.saas.test",
            "OIDC_ISSUER": "https://auth.saas.test", "OIDC_KEYSET_PATH": str(task / "https-signing.json"),
            "SSO_TOTP_ENCRYPTION_KEY": secrets.token_hex(32),
            "CONTEXT_PLATFORM_ORIGIN": "https://console.saas.test:24444",
            "CONTEXT_PLATFORM_SERVICE_ID": ids["selectionService"],
            "CONTEXT_PLATFORM_SERVICE_SECRET": secret["selection"], "CONTEXT_PLATFORM_CA_FILE": ca,
            "CONTEXT_SELECTION_KEY_FILE": str(task / "https-context-keys.json")}
        platform = {"APP_ENV": "test", "BIND_ADDR": "127.0.0.1:23001",
            "DATABASE_URL": database_url.replace("postgres@", prefix + "_platform_runtime@"),
            "PUBLIC_APP_URL": "https://console.saas.test", "JWT_SECRET": secret["jwt"],
            "IDENTITY_REMOTE_ENABLED": "true", "OIDC_ISSUER": "https://auth.saas.test",
            "IDENTITY_SERVICE_URL": "https://auth.saas.test:24444", "IDENTITY_SERVICE_CA_FILE": ca,
            "IDENTITY_SERVICE_CLIENT_ID": ids["accountService"],
            "IDENTITY_SERVICE_CLIENT_SECRET": secret["account"],
            "CONTEXT_SOURCE_SERVICE_CLIENT_ID": ids["sourceService"],
            "CONTEXT_SOURCE_SERVICE_CLIENT_SECRET": secret["source"],
            "BFF_OIDC_ISSUER": "https://auth.saas.test", "BFF_OIDC_CLIENT_ID": ids["client"],
            "BFF_OIDC_CLIENT_SECRET": secret["client"],
            "BFF_OIDC_REDIRECT_URI": "https://console.saas.test/auth/callback", "BFF_CA_CERT_FILE": ca,
            "BFF_TOKEN_KEY_FILE": str(task / "https-bff-keys.json"),
            "TRUSTED_PROXY_IPS": "127.0.0.1,127.0.0.2"}
        environment = {"identity": identity, "platform": platform,
            "accounts": {key: {"username": "https-" + key, "password": account_passwords[key]}
                         for key, _ in account_types}, "ids": ids}
        if args.include_phase19:
            identity["AUTH_INVITATION_KEYSET_PATH"] = str(task / "https-invitation-keys.json")
            for service, credential, variable in (("reviewService", "review", "REVIEW"),
                    ("invitationService", "invitation", "INVITATION"),
                    ("authAdminService", "authAdmin", "AUTH_ADMIN")):
                platform["IDENTITY_" + variable + "_SERVICE_CLIENT_ID"] = ids[service]
                platform["IDENTITY_" + variable + "_SERVICE_CLIENT_SECRET"] = secret[credential]
            environment["phase19"] = {"contractVersion": "unified-auth-phase19/1", "enabled": True}
        if args.include_phase20:
            identity.update({"AUTH_DELIVERY_WORKER_ENABLED": "true", "AUTH_DELIVERY_CA_FILE": ca,
                "AUTH_DELIVERY_TEST_ORIGINS": "https://console.saas.test,https://customer.saas.test,https://auth.saas.test:24444"})
            environment["phase20"] = {"contractVersion": "unified-auth-phase20/1", "enabled": True,
                "recipientRef": "phase20:owned:recipient", "targets": {
                    "consoleBackchannel": ids["consoleBackchannelTarget"],
                    "portalBackchannel": ids["portalBackchannelTarget"],
                    "invitationChannel": ids["invitationChannel"]},
                "collector": {"controlUrl": "http://127.0.0.1:23443", "controlSecret": secret["collectorControl"]}}
        if args.include_portal:
            identity["CUSTOMER_PORTAL_OIDC_CLIENT_ID"] = ids["portalClient"]
            platform.update({"CUSTOMER_PORTAL_PORT_ENABLED": "true",
                "CUSTOMER_PORTAL_OIDC_CLIENT_ID": ids["portalClient"],
                "CUSTOMER_PROFILE_CLIENT_REF": ids["profileClient"],
                "CUSTOMER_PROFILE_APPLICATION_ID": ids["portalApplication"],
                "CUSTOMER_AUTHORITY_SERVICE_CLIENT_ID": ids["customerAuthorityService"],
                "CUSTOMER_AUTHORITY_SERVICE_CLIENT_SECRET": secret["customerAuthority"]})
            environment["portal"] = {"DATABASE_URL": portal_database_url,
                "BIND_ADDR": "127.0.0.1:23101", "CUSTOMER_PORTAL_ENABLED": "true",
                "PUBLIC_CUSTOMER_URL": "https://customer.saas.test", "OIDC_ISSUER": "https://auth.saas.test",
                "OIDC_CLIENT_ID": ids["portalClient"], "OIDC_CLIENT_SECRET": secret["portalClient"],
                "OIDC_REDIRECT_URI": "https://customer.saas.test/auth/callback", "OIDC_CA_FILE": ca,
                "IDENTITY_SERVICE_URL": "https://auth.saas.test:24444", "IDENTITY_SERVICE_CA_FILE": ca,
                "CUSTOMER_SOURCE_SERVICE_CLIENT_ID": ids["portalSourceService"],
                "CUSTOMER_SOURCE_SERVICE_CLIENT_SECRET": secret["portalSource"],
                "CUSTOMER_RESOURCE_URL": "https://console.saas.test:24444", "CUSTOMER_RESOURCE_CA_FILE": ca,
                "CUSTOMER_RESOURCE_CLIENT_ID": profile_public, "CUSTOMER_RESOURCE_CLIENT_SECRET": secret["profile"],
                "PORTAL_TOKEN_KEY_FILE": str(task / "https-portal-keys.json"),
                "CONTEXT_SELECTION_ENABLED": "true", "CONTEXT_SELECTION_CONSUMER_ID": ids["portalConsumerService"],
                "CONTEXT_SELECTION_CONSUMER_SECRET": secret["portalConsumer"]}
        save(task / "https-environment.json", environment)
        print("HTTPS_FIXTURE_PREPARED")
    except BaseException:
        cleanup(args.admin_url, manifest)
        marker.unlink(missing_ok=True)
        raise


if __name__ == "__main__":
    main()
