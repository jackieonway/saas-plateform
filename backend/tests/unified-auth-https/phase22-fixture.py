"""Owner-only Phase 22 load inputs; never executes load or starts an environment.

prepare: before services, register isolated clients/sources and private overrides.
codes: after health, create the one-use 60-second code batch, then run CODE first.
terminal: once, create a synthetic canonical terminal security source/job for alerts.
The environment owner must run prepare.py --cleanup in finally on every outcome.
"""

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import stat
import sys
import uuid
from urllib.parse import urlparse

from cryptography.hazmat.primitives.kdf.argon2 import Argon2id

from prepare import literal, new_id, psql
import phase23_cluster_binding as cluster_binding


TASK = Path("/tmp/rust-saas-unified-auth-20261028")
ISSUER = "https://auth.saas.test"
CONTRACT = "unified-auth-phase22-fixture/1"
ROLE_SUFFIXES = ("migration_owner", "identity_owner", "platform_owner", "bridge_owner",
                 "identity_runtime", "platform_runtime")


def reject(code):
    raise RuntimeError("PHASE22_" + code)


def read_regular(path, private=True, maximum=2 * 1024 * 1024):
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        info = os.fstat(descriptor)
        if (not stat.S_ISREG(info.st_mode) or info.st_size < 1 or info.st_size > maximum
                or (private and (info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o600))):
            reject("PRIVATE_INPUT_REJECTED")
        with os.fdopen(descriptor, "rb", closefd=False) as source:
            data = source.read(maximum + 1)
        if len(data) != info.st_size:
            reject("PRIVATE_INPUT_CHANGED")
        return data
    finally:
        os.close(descriptor)


def write_private(path, value, replace=False):
    """Exclusive mode-0600 creation; safe atomic replacement only for our manifest."""
    data = (json.dumps(value, indent=2) + "\n").encode() if not isinstance(value, bytes) else value
    if path.parent != TASK or not path.name.startswith("phase22-"):
        reject("OUTPUT_PATH_REJECTED")
    temporary = TASK / ("phase22-tmp-" + secrets.token_hex(12)) if replace else path
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "wb", closefd=False) as output:
            output.write(data)
            output.flush()
            os.fsync(descriptor)
        if replace:
            read_regular(path)
            os.replace(temporary, path)
    finally:
        os.close(descriptor)
        if replace and temporary.exists():
            temporary.unlink()


def digest(value):
    return hashlib.sha256(value.encode()).hexdigest()


def canonical_id(value):
    parsed = uuid.UUID(value)
    if parsed.version != 7 or str(parsed) != value:
        reject("IDENTIFIER_REJECTED")
    return value


def array(values, kind="text"):
    return "ARRAY[" + ",".join(literal(value) for value in values) + "]::" + kind + "[]"


def guard_expression(name, prefix, deployment, cluster):
    roles = [prefix + "_" + suffix for suffix in ROLE_SUFFIXES]
    identity_tables = ["users", "sso_sessions", "auth_applications", "auth_clients",
                       "auth_client_credentials", "auth_client_permissions", "auth_resources", "auth_scopes",
                       "auth_consent_groups", "auth_consent_group_clients", "auth_consents", "auth_consent_scopes",
                       "auth_subject_sectors", "auth_subject_links", "auth_authorization_requests", "auth_authorization_codes",
                       "auth_grants", "auth_grant_scopes", "auth_access_tokens", "auth_access_token_scopes",
                       "auth_refresh_families", "auth_refresh_tokens", "auth_rp_sessions", "auth_delivery_targets",
                       "auth_delivery_jobs", "auth_security_notification_outbox"]
    marker = "identity-deployment/1/" + deployment
    return f"""current_database()={literal(name)}
      AND ({cluster_binding.sql_guard(cluster)})
      AND EXISTS(SELECT 1 FROM pg_database WHERE datname=current_database()
        AND pg_get_userbyid(datdba)={literal(prefix+'_migration_owner')}
        AND shobj_description(oid,'pg_database')={literal(marker)})
      AND EXISTS(SELECT 1 FROM pg_namespace WHERE nspname='public'
        AND pg_get_userbyid(nspowner)={literal(prefix+'_migration_owner')})
      AND (SELECT count(*) FROM identity_deployment_state)=1
      AND EXISTS(SELECT 1 FROM identity_deployment_state WHERE singleton
        AND deployment_id={literal(deployment)}::uuid AND schema_version=1 AND issuer={literal(ISSUER)}
        AND migration_owner={literal(prefix+'_migration_owner')}
        AND identity_owner={literal(prefix+'_identity_owner')}
        AND platform_owner={literal(prefix+'_platform_owner')}
        AND revocation_bridge_owner={literal(prefix+'_bridge_owner')}
        AND projection_owner={literal(prefix+'_identity_owner')}
        AND identity_runtime={literal(prefix+'_identity_runtime')}
        AND platform_runtime={literal(prefix+'_platform_runtime')})
      AND (SELECT count(*) FROM pg_roles WHERE rolname=ANY({array(roles)}))=6
      AND NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname=ANY({array(roles)})
        AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls OR rolinherit
          OR rolcanlogin<>(rolname IN({literal(prefix+'_identity_runtime')},{literal(prefix+'_platform_runtime')}))))
      AND NOT EXISTS(SELECT 1 FROM pg_auth_members membership JOIN pg_roles role
        ON role.oid IN(membership.roleid,membership.member) WHERE role.rolname=ANY({array(roles)}))
      AND (SELECT count(*) FROM _sqlx_migrations WHERE version=1 AND success)=1
      AND NOT EXISTS(SELECT 1 FROM _sqlx_migrations WHERE version<>1 OR NOT success)
      AND NOT EXISTS(SELECT 1 FROM unnest({array(identity_tables)}) expected(name)
        WHERE NOT EXISTS(SELECT 1 FROM pg_class relation JOIN pg_namespace namespace
          ON namespace.oid=relation.relnamespace WHERE namespace.nspname='public'
          AND relation.relname=expected.name AND relation.relkind='r'
          AND pg_get_userbyid(relation.relowner)={literal(prefix+'_identity_owner')}))"""


def context(args):
    if (os.name != "posix" or not hasattr(os, "O_NOFOLLOW")
            or args.task_dir.resolve(strict=True) != TASK or args.task_dir.is_symlink()):
        reject("OWNED_LINUX_TASK_REQUIRED")
    marker = read_regular(TASK / "OWNER", private=False, maximum=256).decode().strip()
    if marker != TASK.name + "-pg55494":
        reject("TASK_MARKER_REJECTED")
    cluster = cluster_binding.load_binding(TASK)
    admin = urlparse(args.admin_url)
    if (admin.scheme not in ("postgres", "postgresql") or admin.username != "postgres"
            or admin.password is not None or admin.hostname != "127.0.0.1" or admin.port != 55494
            or admin.path != "/postgres" or admin.params or admin.query or admin.fragment):
        reject("ADMIN_TARGET_REJECTED")
    manifest = json.loads(read_regular(TASK / "https-database.json"))
    environment = json.loads(read_regular(TASK / "https-environment.json"))
    name, prefix = manifest["database"], manifest["rolePrefix"]
    deployment = canonical_id(environment["ids"]["deployment"])
    if (not re.fullmatch(r"unified_auth_https_[0-9a-f]{32}", name)
            or prefix != "uah_" + name[-12:] or manifest["clusterId"] != cluster["systemIdentifier"]
            or "phase21" in environment):
        reject("FRESH_CANONICAL_MANIFEST_REQUIRED")
    runtime = urlparse(environment["identity"]["DATABASE_URL"])
    if (runtime.scheme not in ("postgres", "postgresql") or runtime.hostname != "127.0.0.1"
            or runtime.port != 55494 or runtime.path != "/" + name or runtime.query or runtime.fragment
            or runtime.username != prefix + "_identity_runtime"
            or environment["identity"].get("APP_ENV") != "test"
            or environment["identity"].get("OIDC_ISSUER") != ISSUER):
        reject("RUNTIME_BINDING_REJECTED")
    database_url = admin._replace(path="/" + name).geturl()
    guard = guard_expression(name, prefix, deployment, cluster)
    if psql(database_url, "SELECT (" + guard + ")::text") != "true":
        reject("DATABASE_OR_ROLE_OWNERSHIP_REJECTED")
    return environment, database_url, guard, {"database": name, "rolePrefix": prefix,
        "deployment": deployment, "clusterId": cluster["systemIdentifier"]}


def transact(database_url, guard, statements):
    check = "DO $guard$ BEGIN IF NOT (" + guard + ") THEN RAISE EXCEPTION 'PHASE22_OWNER_GUARD'; END IF; END $guard$"
    psql(database_url, "BEGIN; SELECT pg_advisory_xact_lock(2200261028);\n" + check + ";\n"
         + ";\n".join(statements) + ";\nCOMMIT;")


def fixture_manifest(binding):
    fixture = json.loads(read_regular(TASK / "phase22-fixture.json"))
    if (fixture.get("contractVersion") != CONTRACT or fixture.get("binding") != binding
            or fixture.get("state") != "READY"):
        reject("FIXTURE_BINDING_REJECTED")
    for value in fixture["ids"].values():
        canonical_id(value)
    return fixture


def prepare_inputs(args, environment, database_url, guard, binding):
    if (any(TASK.glob("service-*.pid")) or any(TASK.glob("phase22-*.json"))
            or psql(database_url, "SELECT (NOT EXISTS(SELECT 1 FROM auth_grants) AND NOT EXISTS(SELECT 1 FROM auth_authorization_codes))::text") != "true"):
        reject("UNUSED_PRESTART_FIXTURE_REQUIRED")
    ids = {name: new_id() for name in ("user", "session", "userApp", "userClient", "group", "consent", "sector",
                                      "machineApp", "machineClient", "machineResource", "machineScope",
                                      "machineGrant", "machineAccess")}
    q = {key: literal(value) for key, value in ids.items()}
    root = environment["ids"]
    scopes = [canonical_id(root["openid"]), canonical_id(root["profile"])]
    resource = canonical_id(root["resource"])
    if psql(database_url, f"SELECT (EXISTS(SELECT 1 FROM auth_resources WHERE id={literal(resource)} AND resource_key='userinfo' AND audience={literal(ISSUER+'/oauth/userinfo')} AND status='ACTIVE') AND EXISTS(SELECT 1 FROM auth_scopes WHERE id={literal(scopes[0])} AND resource_id={literal(resource)} AND scope_name='openid' AND status='ACTIVE') AND EXISTS(SELECT 1 FROM auth_scopes WHERE id={literal(scopes[1])} AND resource_id={literal(resource)} AND scope_name='profile' AND status='ACTIVE'))::text") != "true":
        reject("ACCOUNT_SCOPE_BINDING_REJECTED")
    secret = {key: secrets.token_hex(32) for key in ("user", "machine", "metrics", "session", "machineAccess")}
    password = "Owned-P22-" + secrets.token_urlsafe(18) + "9!"
    csrf = digest("sso-session-csrf-v1:" + secret["session"])
    sub = base64.urlsafe_b64encode(secrets.token_bytes(32)).decode().rstrip("=")
    public = {key: "p22-" + key + "-" + ids[key + "Client"][-12:] for key in ("user", "machine")}
    callback, audience = "https://rp.example.test/callback", ISSUER + "/phase22-capacity-resource"
    # This owned input deliberately costs more than the normal account hash.
    # It exercises execution/queue observation, without changing hash policy.
    phc = Argon2id(salt=os.urandom(16), length=32, iterations=3, lanes=1, memory_cost=65536).derive_phc_encoded(password.encode())
    paths = {key: str(TASK / name) for key, name in {
        "userClient": "phase22-user-client.json", "machineClient": "phase22-machine-client.json",
        "codes": "phase22-code-sources.json", "refresh": "phase22-refresh-sources.json",
        "introspection": "phase22-introspection-sources.json", "ca": "phase22-ca.pem"}.items()}
    fixture = {"contractVersion": CONTRACT, "binding": binding, "state": "PREPARING", "ids": ids,
        "provenance": {"codes": "OWNER_SQL_FIXTURE_60_SECOND_CODES_NOT_BROWSER_AUTHORIZATION",
                       "refresh": "OWNER_SQL_CANONICAL_GRANT_AND_FAMILY_FIXTURES",
                       "introspection": "OWNER_SQL_MACHINE_TOKEN_REPLACE_WITH_REAL_CLIENT_CREDENTIALS_IF_DESIRED"},
        "identityOverrides": {"AUTH_SOURCE_REQUESTS": "1000000", "AUTH_CLIENT_REQUESTS": "1000000",
            "AUTH_RATE_MAX_ENTRIES": "4096", "AUTH_RATE_WINDOW_SECONDS": "60", "TRUSTED_PROXY_IPS": "127.0.0.1",
            "IDENTITY_METRICS_TOKEN": secret["metrics"], "IDENTITY_DB_POOL_MAX_CONNECTIONS": "4",
            "IDENTITY_DB_ACQUIRE_TIMEOUT_MS": "300", "IDENTITY_DB_STATEMENT_TIMEOUT_MS": "300",
            "IDENTITY_DB_LOCK_TIMEOUT_MS": "300", "PASSWORD_CONCURRENCY": "1", "PASSWORD_QUEUE_CAPACITY": "1",
            "PASSWORD_QUEUE_TIMEOUT_MS": "100", "PASSWORD_EXECUTION_TIMEOUT_MS": "100",
            "LOGIN_VERIFY_BUDGET_MS": "300"},
        "files": paths, "account": {"username": "phase22-owned-user", "password": password,
            "cookie": "__Host-saas_sso=" + secret["session"], "csrf": csrf},
        "passwordProbe": {"username": "phase22-owned-user", "password": password,
                          "phcParameters": {"m": 65536, "t": 3, "p": 1}},
        "user": {"clientId": public["user"], "scope": "openid profile", "redirectUri": callback,
                 "scopeIds": scopes, "resourceId": resource, "subject": sub},
        "machine": {"clientId": public["machine"], "scope": "phase22.read", "resource": audience},
        "counts": {"codeSources": args.code_sources, "refreshSources": args.refresh_sources},
        "codes": {"state": "NOT_ISSUED", "ttlSeconds": 60}, "terminal": None,
        "cleanup": {"privateFilesGlob": "phase22-*", "databaseAndRoles": "prepare.py --cleanup with the original manifest"}}
    write_private(TASK / "phase22-fixture.json", fixture)
    statements = [
        f"INSERT INTO users(id,username,password_hash,status,session_version) VALUES({q['user']},'phase22-owned-user',{literal(phc)},'ENABLED',1)",
        f"INSERT INTO sso_sessions(id,user_id,token_hash,csrf_hash,account_version,authenticated_at,expires_at) VALUES({q['session']},{q['user']},{literal(digest(secret['session']))},{literal(digest(csrf))},1,statement_timestamp(),statement_timestamp()+interval '30 minutes')",
        f"INSERT INTO auth_resources(id,resource_key,audience,status) VALUES({q['machineResource']},'phase22-capacity',{literal(audience)},'ACTIVE')",
        f"INSERT INTO auth_scopes(id,resource_id,scope_name,status) VALUES({q['machineScope']},{q['machineResource']},'phase22.read','ACTIVE')",
    ]
    for kind in ("user", "machine"):
        machine = kind == "machine"
        app, client = q[kind + "App"], q[kind + "Client"]
        names = ["phase22.read"] if machine else ["openid", "profile"]
        runtime, subject, mode = ("SERVICE", "MACHINE", "NONE") if machine else ("WEB_BFF", "USER", "ACCOUNT_ONLY")
        statements += [
            f"INSERT INTO auth_applications(application_id,ownership,status,display_name,purpose,approved_scopes) VALUES({app},'FIRST_PARTY','APPROVED','Phase 22 {kind}','EXPLICIT_OWNED_CAPACITY_INPUT',{array(names)})",
            f"INSERT INTO auth_clients(id,client_id,display_name,application_id,environment,client_type,runtime_kind,subject_mode,context_mode,grant_types,token_endpoint_auth_method,redirect_uris,status) VALUES({client},{literal(public[kind])},'Phase 22 {kind}',{app},'test','CONFIDENTIAL',{literal(runtime)},{literal(subject)},{literal(mode)},{array(['client_credentials'] if machine else ['authorization_code','refresh_token'])},'client_secret_basic',{array([] if machine else [callback])},'ENABLED')",
            f"INSERT INTO auth_client_credentials(id,client_id,credential_version,secret_digest) VALUES({literal(new_id())},{client},1,{literal(digest(secret[kind]))})",
        ]
        pairs = [(ids["machineResource"], ids["machineScope"])] if machine else [(resource, scope) for scope in scopes]
        for resource_id, scope_id in pairs:
            statements.append(f"INSERT INTO auth_client_permissions(client_id,resource_id,scope_id) VALUES({client},{literal(resource_id)},{literal(scope_id)})")
    statements += [
        f"INSERT INTO auth_consent_groups(id,application_id,environment,display_name,purpose,context_mode,status) VALUES({q['group']},{q['userApp']},'test','Phase 22 input consent','EXPLICIT_OWNED_CAPACITY_INPUT','ACCOUNT_ONLY','ACTIVE')",
        f"INSERT INTO auth_consent_group_clients(group_id,client_id,application_id,environment,context_mode) VALUES({q['group']},{q['userClient']},{q['userApp']},'test','ACCOUNT_ONLY')",
        f"INSERT INTO auth_consents(id,user_id,group_id,status,generation,group_version,version,authorized_at) VALUES({q['consent']},{q['user']},{q['group']},'ACTIVE',1,1,1,statement_timestamp())",
        f"INSERT INTO auth_consent_scopes(consent_id,resource_id,scope_id) SELECT {q['consent']},resource_id,id FROM auth_scopes WHERE id=ANY({array(scopes,'uuid')})",
        f"INSERT INTO auth_subject_sectors(id,application_id,environment,status) VALUES({q['sector']},{q['userApp']},'test','ACTIVE')",
        f"INSERT INTO auth_subject_links(id,user_id,sector_id,subject) VALUES({literal(new_id())},{q['user']},{q['sector']},{literal(sub)})",
    ]
    refresh_tokens, source_grants = [], []
    for _ in range(args.refresh_sources):
        grant, access, family, token = (new_id() for _ in range(4))
        raw, access_raw, sid = (secrets.token_hex(32) for _ in range(3))
        source_grants.append({"grantId": grant, "accessToken": access_raw, "refreshFamilyId": family, "sid": sid})
        refresh_tokens.append(raw)
        statements += [
            f"INSERT INTO auth_grants(id,client_id,subject_mode,user_id,consent_id,consent_generation,consent_group_version,source_session_id,context_mode,account_version,client_policy_version,application_projection_version,scope_ids,audiences,auth_time,expires_at) SELECT {literal(grant)},{q['userClient']},'USER',{q['user']},{q['consent']},1,1,id,'ACCOUNT_ONLY',1,1,1,{array(scopes,'uuid')},ARRAY[{literal(ISSUER+'/oauth/userinfo')}],authenticated_at,expires_at FROM sso_sessions WHERE id={q['session']}",
            f"INSERT INTO auth_grant_scopes(grant_id,resource_id,scope_id) SELECT {literal(grant)},resource_id,id FROM auth_scopes WHERE id=ANY({array(scopes,'uuid')})",
            f"INSERT INTO auth_rp_sessions(grant_id,client_id,user_id,source_session_id,public_sid,public_sub) VALUES({literal(grant)},{q['userClient']},{q['user']},{q['session']},{literal(sid)},{literal(sub)})",
            f"INSERT INTO auth_access_tokens(id,grant_id,token_digest,expires_at) VALUES({literal(access)},{literal(grant)},{literal(digest(access_raw))},statement_timestamp()+interval '5 minutes')",
            f"INSERT INTO auth_access_token_scopes(token_id,resource_id,scope_id) SELECT {literal(access)},resource_id,id FROM auth_scopes WHERE id=ANY({array(scopes,'uuid')})",
            f"INSERT INTO auth_refresh_families(id,grant_id,absolute_expires_at,idle_expires_at) SELECT {literal(family)},id,expires_at,expires_at FROM auth_grants WHERE id={literal(grant)}",
            f"INSERT INTO auth_refresh_tokens(id,family_id,token_digest,generation,expires_at) SELECT {literal(token)},id,{literal(digest(raw))},1,idle_expires_at FROM auth_refresh_families WHERE id={literal(family)}",
        ]
    statements += [
        f"INSERT INTO auth_grants(id,client_id,subject_mode,context_mode,client_policy_version,application_projection_version,credential_version,scope_ids,audiences,expires_at) VALUES({q['machineGrant']},{q['machineClient']},'MACHINE','NONE',1,1,1,ARRAY[{q['machineScope']}::uuid],ARRAY[{literal(audience)}],statement_timestamp()+interval '5 minutes')",
        f"INSERT INTO auth_grant_scopes(grant_id,resource_id,scope_id) VALUES({q['machineGrant']},{q['machineResource']},{q['machineScope']})",
        f"INSERT INTO auth_access_tokens(id,grant_id,token_digest,expires_at) SELECT {q['machineAccess']},id,{literal(digest(secret['machineAccess']))},expires_at FROM auth_grants WHERE id={q['machineGrant']}",
        f"INSERT INTO auth_access_token_scopes(token_id,resource_id,scope_id) VALUES({q['machineAccess']},{q['machineResource']},{q['machineScope']})",
    ]
    transact(database_url, guard, statements)
    ca = read_regular(Path(environment["identity"]["CONTEXT_PLATFORM_CA_FILE"]), private=False, maximum=1024 * 1024)
    if b"-----BEGIN CERTIFICATE-----" not in ca or b"PRIVATE KEY" in ca:
        reject("CA_CERTIFICATE_REJECTED")
    write_private(Path(paths["ca"]), ca)
    for kind in ("user", "machine"):
        write_private(Path(paths[kind + "Client"]), {"clientId": public[kind], "clientSecret": secret[kind]})
    write_private(Path(paths["refresh"]), {"refreshTokens": refresh_tokens})
    write_private(Path(paths["introspection"]), {"tokens": [secret["machineAccess"]]})
    fixture.update({"state": "READY", "refreshSources": source_grants})
    write_private(TASK / "phase22-fixture.json", fixture, replace=True)
    print("PHASE22_INPUTS_READY code_batch_pending=true refresh_sources=" + str(args.refresh_sources))


def issue_codes(database_url, guard, binding):
    fixture = fixture_manifest(binding)
    client = canonical_id(fixture["ids"]["userClient"])
    if (fixture["codes"]["state"] != "NOT_ISSUED"
            or psql(database_url, "SELECT count(*) FROM auth_authorization_requests WHERE client_id=" + literal(client)) != "0"):
        reject("CODE_BATCH_ALREADY_ISSUED")
    fixture["codes"]["state"] = "SEEDING"
    write_private(TASK / "phase22-fixture.json", fixture, replace=True)
    q = {key: literal(value) for key, value in fixture["ids"].items()}
    entries, statements = [], []
    for _ in range(fixture["counts"]["codeSources"]):
        flow, code_id = new_id(), new_id()
        code, verifier, state, nonce = (secrets.token_hex(32) for _ in range(4))
        challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).decode().rstrip("=")
        entries.append({"code": code, "verifier": verifier, "redirectUri": fixture["user"]["redirectUri"]})
        statements += [
            f"INSERT INTO auth_authorization_requests(id,client_id,user_id,source_session_id,account_version,auth_time,client_policy_version,application_projection_version,redirect_uri,state_digest,state_value,nonce_digest,nonce_value,code_challenge,code_challenge_method,scope_ids,browser_digest,csrf_digest,status,expires_at) SELECT {literal(flow)},{q['userClient']},{q['user']},id,account_version,authenticated_at,1,1,{literal(fixture['user']['redirectUri'])},{literal(digest(state))},{literal(state)},{literal(digest(nonce))},{literal(nonce)},{literal(challenge)},'S256',{array(fixture['user']['scopeIds'],'uuid')},{literal(secrets.token_hex(32))},{literal(secrets.token_hex(32))},'AUTHORIZED',LEAST(expires_at,statement_timestamp()+interval '5 minutes') FROM sso_sessions WHERE id={q['session']} AND revoked_at IS NULL AND expires_at>clock_timestamp()",
            f"INSERT INTO auth_authorization_codes(id,request_id,client_id,user_id,consent_id,consent_generation,consent_group_version,code_digest,expires_at) SELECT {literal(code_id)},id,{q['userClient']},{q['user']},{q['consent']},1,1,{literal(digest(code))},LEAST(expires_at,statement_timestamp()+interval '60 seconds') FROM auth_authorization_requests WHERE id={literal(flow)} AND expires_at>clock_timestamp()",
        ]
    transact(database_url, guard, statements)
    write_private(Path(fixture["files"]["codes"]), {"codes": entries})
    fixture["codes"]["state"] = "ISSUED_ONCE"
    write_private(TASK / "phase22-fixture.json", fixture, replace=True)
    print("PHASE22_CODE_BATCH_READY ttl_seconds=60 sources=" + str(len(entries)))


def terminal_source(database_url, guard, binding):
    fixture = fixture_manifest(binding)
    if fixture["terminal"] is not None:
        reject("TERMINAL_SOURCE_ALREADY_REQUESTED")
    event, target, verification = new_id(), new_id(), new_id()
    user = canonical_id(fixture["ids"]["user"])
    fixture["terminal"] = {"state": "SEEDING", "eventId": event, "targetId": target,
        "provenance": "OWNER_SEEDED_SYNTHETIC_MFA_NOTIFICATION_AND_TERMINAL_FAILURE_NOT_WORKER_EXECUTION"}
    write_private(TASK / "phase22-fixture.json", fixture, replace=True)
    statements = [
        f"INSERT INTO auth_delivery_targets(id,purpose,status,recipient_ref,channel_purpose,url,allowed_ips,verified_reference,verified_at,expires_at) VALUES({literal(target)},'VERIFIED_CHANNEL','ACTIVE',{literal(user)},'SECURITY','https://auth.saas.test/phase22-never-delivered',ARRAY['127.0.0.1'],{literal(verification)},clock_timestamp()-interval '1 second',clock_timestamp()+interval '1 hour')",
        f"INSERT INTO auth_security_notification_outbox(id,user_id,event_kind) VALUES({literal(event)},{literal(user)},'MFA_ENABLED')",
        f"UPDATE auth_delivery_jobs SET state='TERMINAL',terminal_at=clock_timestamp(),last_error_code='DELIVERY_HTTP_REJECTED' WHERE security_event_id={literal(event)} AND target_id={literal(target)} AND state='PENDING'",
        f"DO $terminal$ BEGIN IF (SELECT count(*) FROM auth_delivery_jobs WHERE security_event_id={literal(event)} AND target_id={literal(target)} AND state='TERMINAL')<>1 OR NOT EXISTS(SELECT 1 FROM auth_security_notification_outbox WHERE id={literal(event)} AND delivery_state='TERMINAL') THEN RAISE EXCEPTION 'PHASE22_TERMINAL_SOURCE_MISSING'; END IF; END $terminal$",
    ]
    transact(database_url, guard, statements)
    fixture["terminal"]["state"] = "CREATED_ONCE_IMMUTABLE"
    write_private(TASK / "phase22-fixture.json", fixture, replace=True)
    print("PHASE22_TERMINAL_SOURCE_READY owner_seeded=true worker_execution_claim=false")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--task-dir", type=Path, required=True)
    parser.add_argument("--admin-url", required=True)
    parser.add_argument("--stage", choices=("prepare", "codes", "terminal"), default="prepare")
    parser.add_argument("--code-sources", type=int, default=1200)
    parser.add_argument("--refresh-sources", type=int, default=8)
    args = parser.parse_args()
    if not 750 <= args.code_sources <= 10000 or not 1 <= args.refresh_sources <= 32:
        reject("INPUT_COUNT_REJECTED")
    environment, database_url, guard, binding = context(args)
    if args.stage == "prepare":
        prepare_inputs(args, environment, database_url, guard, binding)
    elif args.stage == "codes":
        issue_codes(database_url, guard, binding)
    else:
        terminal_source(database_url, guard, binding)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        # Never print a URL, SQL statement, token, account ID, or traceback.
        message = str(error)
        print(message if re.fullmatch(r"PHASE22_[A-Z0-9_]+", message) else "PHASE22_FIXTURE_FAILED", file=sys.stderr)
        raise SystemExit(1) from None
