"""Owner-only facts for one newly prepared Phase 21 interoperability database.

The delivered RP never imports this module or receives a database credential.
Run after prepare.py and before any service starts; use prepare.py --cleanup in
the caller's finally block, including when this extension fails.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
from urllib.parse import urlparse

from cryptography.hazmat.primitives.kdf.argon2 import Argon2id

from prepare import literal, new_id, psql, save
import phase23_cluster_binding as cluster_binding


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--task-dir", type=Path, required=True)
    parser.add_argument("--admin-url", required=True)
    parser.add_argument("--rp-origin", choices=("https://customer.saas.test", "https://customer.saas.test:24446"),
                        default="https://customer.saas.test")
    args = parser.parse_args()
    task = args.task_dir.resolve(strict=True)
    url = urlparse(args.admin_url)
    if (task != Path("/tmp/rust-saas-unified-auth-20261028")
            or not (task / "OWNER").is_file()
            or url.scheme not in ("postgres", "postgresql")
            or url.hostname != "127.0.0.1" or url.port != 55494
            or url.path != "/postgres" or url.query or url.fragment):
        raise RuntimeError("owned Phase 21 task and cluster required")
    manifest = json.loads((task / "https-database.json").read_text())
    environment = json.loads((task / "https-environment.json").read_text())
    binding = cluster_binding.load_binding(task)
    name, prefix = manifest["database"], manifest["rolePrefix"]
    if (not re.fullmatch(r"unified_auth_https_[0-9a-f]{32}", name)
            or prefix != "uah_" + name[-12:]
            or manifest["clusterId"] != binding['systemIdentifier']
            or "phase21" in environment
            or any(task.glob("service-*.pid")) or (task / "namespace.pid").exists()):
        raise RuntimeError("fresh unused Phase 21 fixture required")
    database_url = args.admin_url.rsplit("/", 1)[0] + "/" + name
    marker = "identity-deployment/1/" + environment["ids"]["deployment"]
    check = psql(database_url, "SELECT system_identifier::text||'|'||"
                 "shobj_description((SELECT oid FROM pg_database WHERE datname=current_database()),'pg_database')"
                 "||'|'||(SELECT count(*) FROM auth_grants)::text FROM pg_control_system() WHERE "
                 + cluster_binding.sql_guard(binding))
    if check != manifest["clusterId"] + "|" + marker + "|0":
        raise RuntimeError("owned empty database marker mismatch")
    ids = {key: new_id() for key in (
        "user", "otherUser", "partner", "requester", "reviewer", "contact", "domain", "review",
        "thirdApp", "thirdClient", "thirdGroup", "thirdSector", "thirdTarget", "thirdTargetVerification",
        "offlineApp", "offlineClient", "offlineGroup", "offlineSector", "offlineScope",
        "machineApp", "machineClient", "resourceClient", "demoResource", "demoRead", "demoAdmin")}
    q = {key: literal(value) for key, value in ids.items()}
    root_ids = environment["ids"]
    origin, issuer = args.rp_origin, "https://auth.saas.test"
    callback, receiver = origin + "/auth/callback", origin + "/auth/backchannel-logout"
    credentials = {key: secrets.token_hex(32) for key in ("third", "offline", "machine", "resource")}
    accounts = {}
    statements = []
    for key in ("user", "otherUser"):
        password = "Owned-P21-" + secrets.token_urlsafe(18) + "9!"
        username = "phase21-" + key.lower()
        phc = Argon2id(salt=os.urandom(16), length=32, iterations=2,
                       lanes=1, memory_cost=19456).derive_phc_encoded(password.encode())
        statements.append(f"INSERT INTO users(id,username,password_hash,real_name,session_version) "
                          f"VALUES({q[key]},{literal(username)},{literal(phc)},'Phase 21 pure account',1)")
        accounts[key] = {"username": username, "password": password}
    statements += [
        f"INSERT INTO auth_scopes(id,resource_id,scope_name,status) VALUES({q['offlineScope']},{literal(root_ids['resource'])},'offline_access','ACTIVE')",
        f"INSERT INTO auth_resources(id,resource_key,audience,status) VALUES({q['demoResource']},'p21-demo',{literal(origin+'/demo-resource')},'ACTIVE')",
        f"INSERT INTO auth_scopes(id,resource_id,scope_name,status) VALUES({q['demoRead']},{q['demoResource']},'demo.read','ACTIVE'),({q['demoAdmin']},{q['demoResource']},'demo.admin','ACTIVE')",
    ]
    for key, ownership, scopes in (("third", "THIRD_PARTY", ["openid", "profile"]),
                                   ("offline", "FIRST_PARTY", ["openid", "profile", "offline_access"]),
                                   ("machine", "FIRST_PARTY", ["demo.read"])):
        app_id = q[key + "App"]
        partner = q["partner"] if ownership == "THIRD_PARTY" else "NULL"
        array = "ARRAY[" + ",".join(literal(s) for s in scopes) + "]"
        statements.append(f"INSERT INTO auth_applications(application_id,partner_id,ownership,status,display_name,purpose,approved_scopes) "
                          f"VALUES({app_id},{partner},'{ownership}','APPROVED','Phase 21 {key}','EXPLICIT_OWNED_INTEROP_FIXTURE',{array})")
    statements.append(f"INSERT INTO auth_application_admissions(application_id,environment,status,redirect_uris,scopes,requested_by,reviewed_by,contact_reference,domain_reference,review_reference,logout_mode,backchannel_logout_uri) "
                      f"VALUES({q['thirdApp']},'test','APPROVED',ARRAY[{literal(callback)}],ARRAY['openid','profile'],{q['requester']},{q['reviewer']},{q['contact']},{q['domain']},{q['review']},'BACK_CHANNEL',{literal(receiver)})")
    public_clients = {}
    for key, app in (("third", "third"), ("offline", "offline"), ("machine", "machine"), ("resource", "machine")):
        public_id = "p21-" + key + "-" + ids[key + "Client"][-12:]
        public_clients[key] = public_id
        machine = key in ("machine", "resource")
        context, subject, runtime = ("NONE", "MACHINE", "SERVICE") if machine else ("ACCOUNT_ONLY", "USER", "WEB_BFF")
        grants = "ARRAY['client_credentials']" if machine else "ARRAY['authorization_code','refresh_token']"
        redirects = "ARRAY[]::text[]" if machine else "ARRAY[" + literal(callback) + "]"
        statements.append(f"INSERT INTO auth_clients(id,client_id,display_name,application_id,environment,client_type,runtime_kind,subject_mode,context_mode,grant_types,token_endpoint_auth_method,redirect_uris,status) "
                          f"VALUES({q[key+'Client']},{literal(public_id)},'Phase 21 {key}',{q[app+'App']},'test','CONFIDENTIAL','{runtime}','{subject}','{context}',{grants},'client_secret_basic',{redirects},'ENABLED')")
        digest = hashlib.sha256(credentials[key].encode()).hexdigest()
        statements.append(f"INSERT INTO auth_client_credentials(id,client_id,credential_version,secret_digest) VALUES({literal(new_id())},{q[key+'Client']},1,{literal(digest)})")
        permissions = [(ids["demoResource"], ids["demoRead"])] if machine else [
            (root_ids["resource"], root_ids["openid"]), (root_ids["resource"], root_ids["profile"])]
        if key == "offline":
            permissions.append((root_ids["resource"], ids["offlineScope"]))
        for resource, scope in permissions:
            statements.append(f"INSERT INTO auth_client_permissions(client_id,resource_id,scope_id) VALUES({q[key+'Client']},{literal(resource)},{literal(scope)})")
        if not machine:
            statements += [
                f"INSERT INTO auth_consent_groups(id,application_id,environment,display_name,purpose,context_mode,status) VALUES({q[key+'Group']},{q[key+'App']},'test','Phase 21 {key}','EXPLICIT_OWNED_INTEROP_FIXTURE','ACCOUNT_ONLY','ACTIVE')",
                f"INSERT INTO auth_consent_group_clients(group_id,client_id,application_id,environment,context_mode) VALUES({q[key+'Group']},{q[key+'Client']},{q[key+'App']},'test','ACCOUNT_ONLY')",
                f"INSERT INTO auth_subject_sectors(id,application_id,environment,status) VALUES({q[key+'Sector']},{q[key+'App']},'test','ACTIVE')",
            ]
    statements += [
        f"INSERT INTO auth_delivery_targets(id,purpose,status,client_id,url,allowed_ips,verified_reference,verified_at,expires_at) VALUES({q['thirdTarget']},'BACK_CHANNEL','ACTIVE',{q['thirdClient']},{literal(receiver)},ARRAY['127.0.0.2'],{q['thirdTargetVerification']},clock_timestamp()-interval '1 second',clock_timestamp()+interval '4 hours')",
        f"UPDATE auth_clients SET logout_mode='BACK_CHANNEL',backchannel_logout_uri={literal(receiver)},backchannel_target_id={q['thirdTarget']} WHERE id={q['thirdClient']}",
    ]
    psql(database_url, "BEGIN;\n" + ";\n".join(statements) + ";\nCOMMIT;")
    ca = environment["identity"]["CONTEXT_PLATFORM_CA_FILE"]
    # The RP is an additional recipient alongside the already prepared console,
    # portal and invitation collector; each still needs its exact TLS origin.
    delivery_origins = list(dict.fromkeys(filter(None,
        environment["identity"].get("AUTH_DELIVERY_TEST_ORIGINS", "").split(",") + [origin])))
    environment["identity"].update({"AUTH_DELIVERY_WORKER_ENABLED": "true",
        "AUTH_DELIVERY_CA_FILE": ca, "AUTH_DELIVERY_TEST_ORIGINS": ",".join(delivery_origins)})
    configs = {}
    for key in ("third", "offline"):
        secret_file = task / ("phase21-" + key + "-secret")
        secret_file.write_text(credentials[key], encoding="utf-8")
        secret_file.chmod(0o600)
        configs[key] = {"OIDC_ISSUER": issuer, "OIDC_CLIENT_ID": public_clients[key],
            "OIDC_CLIENT_SECRET_FILE": str(secret_file), "RP_ORIGIN": origin,
            "RP_BIND_HOST": "127.0.0.1", "RP_BIND_PORT": "23102" if origin.endswith(":24446") else "23101", "NODE_EXTRA_CA_CERTS": ca,
            "OIDC_SCOPES": "openid profile" + (" offline_access" if key == "offline" else "")}
    for key in ("machine", "resource"):
        secret_file = task / ("phase21-" + key + "-secret")
        secret_file.write_text(credentials[key], encoding="utf-8")
        secret_file.chmod(0o600)
    environment["phase21"] = {"contractVersion": "unified-auth-phase21/1", "ids": ids,
        "accounts": accounts, "publicClients": public_clients, "rp": configs,
        "machine": {"issuer": issuer, "clientId": public_clients["machine"],
            "secretFile": str(task / "phase21-machine-secret"), "scope": "demo.read",
            "resource": origin + "/demo-resource"},
        "resource": {"clientId": public_clients["resource"], "secretFile": str(task / "phase21-resource-secret")}}
    public = environment["phase21"]
    save(task / "phase21-public.json", {key: public[key] for key in ("rp", "machine", "resource")})
    save(task / "https-environment.json", environment)
    print("PHASE21_OWNED_FIXTURE_READY pure_accounts=2 third_party=openid_profile approved_offline=separate machine=separate")


if __name__ == "__main__":
    main()
