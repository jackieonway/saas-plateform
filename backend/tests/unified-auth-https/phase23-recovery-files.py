"""Pure protected-file adapter for the owned canonical recovery rehearsal.

No database, application, proxy, signal or network operation is performed here.
All private reads/writes use the frozen recovery core's anchored mode-0600 API.
Original fixture files and the cleanup manifest are never overwritten.
"""
import argparse
import base64
import copy
import hashlib
import os
from pathlib import Path
import re
import secrets
import sys
import uuid
from urllib.parse import quote, unquote, urlsplit, urlunsplit

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "ops/identity-recovery"))
import core
import inventory

sys.path.insert(0, str(Path(__file__).resolve().parent))
import phase23_cluster_binding as cluster_binding


ISSUER = "https://auth.saas.test"
RP_ORIGIN = "https://customer.saas.test:24446"
LABELS = frozenset(("environment", "signing", "bff", "portal", "context", "invitation",
                    "tls_ca", "tls_certificate", "tls_key", "third_secret", "offline_secret",
                    "machine_secret", "resource_secret"))
SERVICE_TABLES = ("identity_service_clients", "context_selection_services", "platform_resource_services")

# container, secret variable, ID variable, current authoritative table, lookup
# column, expected public purpose, independently recorded metadata ID.
SERVICE_BINDINGS = (
    ("identity", "CONTEXT_PLATFORM_SERVICE_SECRET", "CONTEXT_PLATFORM_SERVICE_ID",
     "context_selection_services", "id", "CONTEXT_SELECTION_AUTHORITY", "selectionService"),
    ("platform", "IDENTITY_SERVICE_CLIENT_SECRET", "IDENTITY_SERVICE_CLIENT_ID",
     "identity_service_clients", "id", "ACCOUNT_COMMAND", "accountService"),
    ("platform", "CONTEXT_SOURCE_SERVICE_CLIENT_SECRET", "CONTEXT_SOURCE_SERVICE_CLIENT_ID",
     "identity_service_clients", "id", "CONTEXT_SELECTION_SOURCE", "sourceService"),
    ("platform", "IDENTITY_REVIEW_SERVICE_CLIENT_SECRET", "IDENTITY_REVIEW_SERVICE_CLIENT_ID",
     "identity_service_clients", "id", "AUTH_REVIEW", "reviewService"),
    ("platform", "IDENTITY_INVITATION_SERVICE_CLIENT_SECRET", "IDENTITY_INVITATION_SERVICE_CLIENT_ID",
     "identity_service_clients", "id", "AUTH_INVITATION", "invitationService"),
    ("platform", "IDENTITY_AUTH_ADMIN_SERVICE_CLIENT_SECRET", "IDENTITY_AUTH_ADMIN_SERVICE_CLIENT_ID",
     "identity_service_clients", "id", "AUTH_ADMIN", "authAdminService"),
    ("platform", "IDENTITY_KEY_ADMIN_SERVICE_CLIENT_SECRET", "IDENTITY_KEY_ADMIN_SERVICE_CLIENT_ID",
     "identity_service_clients", "id", "KEY_ADMIN", "keyAdminService"),
    ("platform", "CUSTOMER_AUTHORITY_SERVICE_CLIENT_SECRET", "CUSTOMER_AUTHORITY_SERVICE_CLIENT_ID",
     "identity_service_clients", "id", "CUSTOMER_ACCESS_PROOF", "customerAuthorityService"),
    ("portal", "CUSTOMER_SOURCE_SERVICE_CLIENT_SECRET", "CUSTOMER_SOURCE_SERVICE_CLIENT_ID",
     "identity_service_clients", "id", "CONTEXT_SELECTION_SOURCE", "portalSourceService"),
    ("portal", "CUSTOMER_RESOURCE_CLIENT_SECRET", "CUSTOMER_RESOURCE_CLIENT_ID",
     "platform_resource_services", "service_id", "CUSTOMER_PROFILE", "profileClient"),
    ("portal", "CONTEXT_SELECTION_CONSUMER_SECRET", "CONTEXT_SELECTION_CONSUMER_ID",
     "context_selection_services", "id", "CONTEXT_SELECTION_CONSUMER", "portalConsumerService"),
)
CA_BINDINGS = (("identity", "CONTEXT_PLATFORM_CA_FILE"), ("identity", "AUTH_DELIVERY_CA_FILE"),
               ("platform", "IDENTITY_SERVICE_CA_FILE"), ("platform", "BFF_CA_CERT_FILE"),
               ("portal", "OIDC_CA_FILE"), ("portal", "IDENTITY_SERVICE_CA_FILE"),
               ("portal", "CUSTOMER_RESOURCE_CA_FILE"))
METRICS_KEYS = frozenset(("IDENTITY_METRICS_TOKEN", "AUTH_METRICS_TOKEN", "METRICS_TOKEN"))
RING_BINDINGS = {"bff": ("platform", "BFF_TOKEN_KEY_FILE"),
                 "portal": ("portal", "PORTAL_TOKEN_KEY_FILE"),
                 "context": ("identity", "CONTEXT_SELECTION_KEY_FILE")}


def need(condition, code):
    core.need(condition, "FILES_" + code)


def uid(value):
    try:
        parsed = uuid.UUID(value) if type(value) is str else None
    except (ValueError, TypeError, AttributeError):
        parsed = None
    need(parsed is not None and parsed.version == 7 and str(parsed) == value, "UUID_INVALID")
    return value


def path_in(root, value):
    return core.safe_path(value, root)


def read_json(root, value):
    return core.decode(core.private_read(path_in(root, value)))


def owner_root(value):
    need(os.name == "posix" and hasattr(os, "O_NOFOLLOW"), "POSIX_REQUIRED")
    root = core.safe_path(value)
    need(root.parent == Path("/tmp") and root.name.startswith("rust-saas-unified-auth-"), "TASK_ROOT_INVALID")
    core.private_directory(root)
    need(core.private_read(root / "OWNER", 256).decode().strip() == root.name + "-pg55494", "OWNER_INVALID")
    return root


def text_secret(value):
    need(type(value) is str and 12 <= len(value.encode()) <= 2048 and "\0" not in value, "SECRET_INVALID")
    return value


def connection(value, role, database):
    need(type(value) is str, "CONNECTION_INVALID")
    try:
        parsed = urlsplit(value)
        valid = (parsed.scheme in ("postgres", "postgresql") and parsed.hostname == "127.0.0.1"
            and parsed.port == 55494 and unquote(parsed.username or "") == role
            and unquote(parsed.path) == "/" + database and not parsed.query and not parsed.fragment
            and parsed.password is not None)
    except ValueError:
        valid = False
    need(valid, "CONNECTION_INVALID")
    password = text_secret(unquote(parsed.password))
    return {"PGHOST": "127.0.0.1", "PGPORT": "55494", "PGUSER": role,
            "PGDATABASE": database, "PGPASSWORD": password}


def sensitive_paths(value, prefix=()):
    result = set()
    if type(value) is dict:
        for key, child in value.items():
            need(type(key) is str, "FIELD_INVALID")
            current = prefix + (key,)
            if re.search(r"secret|password|private.?key|token|keyset|key_file|_key$", key, re.I):
                result.add(current)
            result.update(sensitive_paths(child, current))
    elif type(value) is list:
        for index, child in enumerate(value):
            result.update(sensitive_paths(child, prefix + (str(index),)))
    return result


def validate_source(root, source, check_materials=True):
    core.exact(source, ("identity", "platform", "portal", "accounts", "ids",
                        "phase19", "phase20", "phase21", "phase23"))
    for container in ("identity", "platform", "portal"):
        need(type(source[container]) is dict and all(type(key) is str and type(value) is str
             for key, value in source[container].items()), "PROCESS_ENV_INVALID")
    retired_ttl_fields = {"OIDC_CODE_TTL_SECONDS", "OIDC_TOKEN_TTL_SECONDS", "OIDC_REFRESH_TTL_SECONDS"}
    need(all(not (retired_ttl_fields & source[container].keys())
             for container in ("identity", "platform", "portal")), "RETIRED_TTL_OVERRIDE")
    core.exact(source["phase19"], ("contractVersion", "enabled"))
    need(source["phase19"] == {"contractVersion": "unified-auth-phase19/1", "enabled": True}, "PHASE19_CONTRACT_INVALID")
    core.exact(source["phase20"], ("contractVersion", "enabled", "recipientRef", "targets", "collector"))
    core.exact(source["phase20"]["collector"], ("controlUrl", "controlSecret"))
    need(source["phase20"]["contractVersion"] == "unified-auth-phase20/1" and source["phase20"]["enabled"] is True,
         "PHASE20_CONTRACT_INVALID")
    ids, standard, phase = source["ids"], source["phase21"], source["phase23"]
    need(type(ids) is dict and all(type(key) is str for key in ids), "IDS_INVALID")
    for value in ids.values():
        uid(value)
    uid(ids["deployment"])
    core.exact(source["accounts"], ("admin", "employee", "customer", "applicant", "reviewer", "publisher"))
    need(all(name in ids for name in source["accounts"]), "ACCOUNT_ID_MAPPING_MISSING")
    core.exact(standard, ("contractVersion", "ids", "accounts", "publicClients", "rp", "machine", "resource"))
    need(standard["contractVersion"] == "unified-auth-phase21/1", "STANDARD_CONTRACT_INVALID")
    for value in standard["ids"].values():
        uid(value)
    core.exact(standard["publicClients"], ("third", "offline", "machine", "resource"))
    core.exact(standard["rp"], ("third", "offline"))
    core.exact(standard["accounts"], ("user", "otherUser"))
    for key, account in {**source["accounts"], **standard["accounts"]}.items():
        core.exact(account, ("username", "password"))
        need(type(account["username"]) is str and 1 <= len(account["username"]) <= 100, "ACCOUNT_INVALID")
        text_secret(account["password"])
    core.exact(phase, ("contractVersion", "ids", "signingKids", "rpOrigin", "normalTokenTtlSeconds",
                       "keyRetirementRequestedProfileSeconds", "longCarrierRequestedProfileSeconds"))
    need(phase["contractVersion"] == "unified-auth-phase23/1" and phase["rpOrigin"] == RP_ORIGIN,
         "PHASE23_CONTRACT_INVALID")
    core.exact(phase["ids"], ("keyAdminService", "retireMember", "markerOrganization"))
    for value in phase["ids"].values():
        uid(value)
    core.exact(phase["signingKids"], ("a", "b", "c", "d"))
    need(phase["signingKids"] == {"a": "owned-https", "b": "phase23-b", "c": "phase23-c", "d": "phase23-d"},
         "SIGNING_METADATA_INVALID")
    need(type(phase["normalTokenTtlSeconds"]) is int and phase["normalTokenTtlSeconds"] == 300
         and phase["keyRetirementRequestedProfileSeconds"] == 30 and phase["longCarrierRequestedProfileSeconds"] == 600,
         "TTL_POLICY_INVALID")
    need(source["identity"]["OIDC_ISSUER"] == ISSUER
         and source["platform"]["BFF_OIDC_ISSUER"] == ISSUER
         and source["portal"]["OIDC_ISSUER"] == ISSUER, "ISSUER_INVALID")
    for key in ("third", "offline"):
        rp = standard["rp"][key]
        core.exact(rp, ("OIDC_ISSUER", "OIDC_CLIENT_ID", "OIDC_CLIENT_SECRET_FILE", "RP_ORIGIN",
                        "RP_BIND_HOST", "RP_BIND_PORT", "NODE_EXTRA_CA_CERTS", "OIDC_SCOPES"))
        need(rp["OIDC_ISSUER"] == ISSUER and rp["OIDC_CLIENT_ID"] == standard["publicClients"][key]
             and rp["RP_ORIGIN"] == RP_ORIGIN and rp["RP_BIND_HOST"] == "127.0.0.1"
             and rp["RP_BIND_PORT"] == "23102", "RP_BINDING_INVALID")
    core.exact(standard["machine"], ("issuer", "clientId", "secretFile", "scope", "resource"))
    core.exact(standard["resource"], ("clientId", "secretFile"))
    need(standard["machine"]["issuer"] == ISSUER and standard["machine"]["resource"] == RP_ORIGIN + "/demo-resource"
         and standard["machine"]["scope"] == "demo.read"
         and standard["machine"]["clientId"] == standard["publicClients"]["machine"]
         and standard["resource"]["clientId"] == standard["publicClients"]["resource"], "MACHINE_BINDING_INVALID")
    expected = set()
    for container, secret, id_field, _, lookup, _, metadata_id in SERVICE_BINDINGS:
        text_secret(source[container][secret])
        need(type(source[container][id_field]) is str, "SERVICE_MAPPING_MISSING")
        expected_id = phase["ids"][metadata_id] if metadata_id == "keyAdminService" else ids[metadata_id]
        if lookup == "id":
            need(source[container][id_field] == expected_id, "SERVICE_METADATA_MAPPING_INVALID")
        expected.add((container, secret))
    need(source["platform"]["BFF_OIDC_CLIENT_ID"] == ids["client"]
         and source["portal"]["OIDC_CLIENT_ID"] == ids["portalClient"], "CLIENT_METADATA_MAPPING_INVALID")
    for container, key in (("identity", "SSO_TOTP_ENCRYPTION_KEY"), ("platform", "JWT_SECRET"),
                           ("platform", "BFF_OIDC_CLIENT_SECRET"), ("portal", "OIDC_CLIENT_SECRET")):
        text_secret(source[container][key])
        expected.add((container, key))
    for container in ("identity", "platform", "portal"):
        for key in METRICS_KEYS & source[container].keys():
            text_secret(source[container][key])
            expected.add((container, key))
    expected |= {("identity", "OIDC_KEYSET_PATH"),
                 ("identity", "AUTH_INVITATION_KEYSET_PATH"), ("phase20", "collector", "controlSecret"),
                 ("phase23", "normalTokenTtlSeconds")}
    expected |= {(container, field) for container, field in RING_BINDINGS.values()}
    expected |= {("accounts", key, "password") for key in source["accounts"]}
    expected |= {("phase21", "accounts", key, "password") for key in standard["accounts"]}
    expected |= {("phase21", "rp", key, "OIDC_CLIENT_SECRET_FILE") for key in ("third", "offline")}
    expected |= {("phase21", key, "secretFile") for key in ("machine", "resource")}
    need(sensitive_paths(source) == expected, "UNKNOWN_SECRET_FIELD")
    text_secret(source["phase20"]["collector"]["controlSecret"])
    if check_materials:
        for path in source_material_paths(root, source).values():
            core.private_read(path)
    return source


def source_material_paths(root, source):
    paths = {"signing": source["identity"]["OIDC_KEYSET_PATH"],
             "invitation": source["identity"]["AUTH_INVITATION_KEYSET_PATH"]}
    paths.update({label: source[container][field] for label, (container, field) in RING_BINDINGS.items()})
    paths.update({key + "_secret": source["phase21"]["rp"][key]["OIDC_CLIENT_SECRET_FILE"]
                  for key in ("third", "offline")})
    paths.update({key + "_secret": source["phase21"][key]["secretFile"] for key in ("machine", "resource")})
    return {label: path_in(root, path) for label, path in paths.items()}


class Outputs:
    """Validate the whole output set before the first exclusive creation."""
    def __init__(self, root, inputs):
        self.root, self.inputs, self.files = root, set(map(Path, inputs)), {}

    def add(self, path, value):
        path = path_in(self.root, path)
        need(path not in self.inputs and path not in self.files, "OUTPUT_COLLISION")
        # lstat-style existence, plus anchored creation below; no original file
        # is overwritten, even if another owner creates a path after this check.
        need(not path.exists() and not path.is_symlink(), "OUTPUT_EXISTS")
        self.files[path] = value if type(value) is bytes else core.canonical(value)
        return str(path)

    def commit(self):
        created = []
        try:
            for path, data in self.files.items():
                core.private_write(path, data)
                created.append(path)
        except Exception:
            cleanup_failed = False
            for path in reversed(created):
                try:
                    with core.parent_descriptor(path) as (parent, name):
                        need(core.read_at(parent, name, core.MAX_FILE) == self.files[path], "PARTIAL_OUTPUT_CHANGED")
                        os.unlink(name, dir_fd=parent)
                        os.fsync(parent)
                except Exception:
                    # Never delete a replacement. The fixed error below tells
                    # the owner that targeted final cleanup is still required.
                    cleanup_failed = True
            if cleanup_failed:
                raise core.RecoveryError("RECOVERY_FILES_PARTIAL_OUTPUT_CLEANUP_REQUIRED") from None
            raise


def prepare(task_dir, environment_file, manifest_file, output_config, tls_materials_file):
    root = owner_root(task_dir)
    binding = cluster_binding.load_binding(root)
    cluster = binding['systemIdentifier']
    environment_path, manifest_path = path_in(root, environment_file), path_in(root, manifest_file)
    source = validate_source(root, read_json(root, environment_path))
    manifest = read_json(root, manifest_path)
    core.exact(manifest, ("database", "rolePrefix", "clusterId", "portal"))
    database, prefix = manifest["database"], manifest["rolePrefix"]
    need(type(database) is str and re.fullmatch(r"unified_auth_https_[0-9a-f]{32}", database)
         and prefix == "uah_" + database[-12:] and manifest["clusterId"] == cluster, "MANIFEST_INVALID")
    portal = manifest["portal"]
    core.exact(portal, ("database", "owner", "runtime", "marker", "initializationPending"))
    task_id = uid(source["ids"]["deployment"])
    need(portal == {"database": "unified_auth_portal_" + database[-32:], "owner": prefix + "_portal_owner",
        "runtime": prefix + "_portal_runtime", "marker": "unified-auth-https-portal/1/" + task_id,
        "initializationPending": False}, "PORTAL_MANIFEST_INVALID")
    connections = {kind: connection(source[kind]["DATABASE_URL"], prefix + "_" + kind + "_runtime",
                    portal["database"] if kind == "portal" else database)
                   for kind in ("identity", "platform", "portal")}
    tls = read_json(root, tls_materials_file)
    core.exact(tls, ("ca", "certificate", "key"))
    tls_paths = {label: path_in(root, path) for label, path in tls.items()}
    tls_bytes = {label: core.private_read(path) for label, path in tls_paths.items()}
    need(all(data.startswith(b"-----BEGIN ") for data in tls_bytes.values()), "TLS_MATERIAL_INVALID")
    for container, field in CA_BINDINGS:
        need(core.private_read(path_in(root, source[container][field])) == tls_bytes["ca"], "CA_MAPPING_CHANGED")
    for rp in source["phase21"]["rp"].values():
        need(core.private_read(path_in(root, rp["NODE_EXTRA_CA_CERTS"])) == tls_bytes["ca"], "CA_MAPPING_CHANGED")
    materials = {"environment": str(environment_path), **{label: str(path)
                 for label, path in source_material_paths(root, source).items()},
                 **{"tls_" + label: str(path) for label, path in tls_paths.items()}}
    need(set(materials) == LABELS, "MATERIAL_LABELS_INVALID")
    output = Outputs(root, (environment_path, manifest_path, Path(tls_materials_file), *map(Path, materials.values())))
    admin_file = output.add(root / "phase23-admin-connection.json", {"PGHOST": "127.0.0.1",
        "PGPORT": "55494", "PGUSER": "postgres", "PGDATABASE": "postgres"})
    connection_files = {kind: output.add(root / ("phase23-" + kind + "-connection.json"), connections[kind])
                        for kind in ("platform", "portal")}
    config = {"format": "identity-recovery/2", "taskId": task_id, "clusterId": cluster, "issuer": ISSUER,
        "taskRoot": str(root), "workDirectory": str(root / "phase23-recovery-private"),
        "witnessDirectory": str(root / "phase23-recovery-witness"),
        "floorFile": str(root / "phase23-recovery-floor.json"), "keyFile": str(root / "phase23-recovery-keys.json"),
        "adminConnectionFile": admin_file, "pgBin": "/usr/lib/postgresql/18/bin", "privateMaterials": materials,
        "databases": {"platform": {"source": database, "target": "unified_auth_restore_" + database[-32:],
            "owner": prefix + "_migration_owner", "sourceMarker": "identity-deployment/1/" + task_id,
            "targetMarker": "phase23-recovery/2/" + task_id, "connectionFile": connection_files["platform"]},
          "portal": {"source": portal["database"], "target": "unified_auth_restore_portal_" + database[-32:],
            "owner": portal["owner"], "sourceMarker": portal["marker"],
            "targetMarker": "phase23-recovery/2/" + task_id, "connectionFile": connection_files["portal"]}}}
    output.add(output_config, config)
    output.commit()
    return {"status": "PHASE23_RECOVERY_FILES_PREPARED", "databases": 2, "runtimeConnections": 2,
            "materials": len(materials), "sourceUnchanged": True}


def index(rows, field, value):
    matches = [row for row in rows if row[field] == value]
    need(len(matches) == 1, "AUTHORITATIVE_MAPPING_INVALID")
    return matches[0]


def selection_ids(value, eligible):
    need(type(value) is list and all(type(item) is str for item in value)
         and len(value) == len(set(value)) and set(value) <= set(eligible), "SELECTION_NOT_ELIGIBLE")
    for item in value:
        uid(item)
    return sorted(value)


class Fresh:
    def __init__(self):
        self.seen = set()

    def secret(self):
        for _ in range(8):
            value = secrets.token_hex(32)
            if value not in self.seen:
                self.seen.add(value)
                return value
        need(False, "RANDOM_COLLISION")

    def key(self):
        # All keys and credentials use the same uniqueness set and independent
        # 32-byte random draws; none is derived from a user's identity.
        return bytes.fromhex(self.secret())


def b64(value):
    return base64.urlsafe_b64encode(value).rstrip(b"=").decode()


def invitation_history(raw):
    """Keep authenticated original-operation verification keys, never drop one."""
    invitation = core.decode(raw)
    core.exact(invitation, ("purpose", "activeVersion", "keys"))
    need(invitation["purpose"] == "saas-identity/auth-invitation/1"
         and type(invitation["activeVersion"]) is int
         and type(invitation["keys"]) is list and 1 <= len(invitation["keys"]) <= 16,
         "INVITATION_KEY_INVALID")
    versions, materials = set(), set()
    for item in invitation["keys"]:
        core.exact(item, ("version", "key"))
        version, key = item["version"], item["key"]
        need(type(version) is int and 1 <= version < (1 << 63) - 2 and version not in versions
             and type(key) is str and re.fullmatch(r"[A-Za-z0-9_-]{43}", key),
             "INVITATION_KEY_INVALID")
        material = base64.urlsafe_b64decode(key + "=")
        need(len(material) == 32 and any(material) and b64(material) == key
             and material not in materials, "INVITATION_KEY_INVALID")
        versions.add(version)
        materials.add(material)
    need(invitation["activeVersion"] in versions, "INVITATION_VERSION_INVALID")
    # The service permits at most 16 versions. Original receipt validation still
    # needs historical keys, so capacity exhaustion must not silently retire one.
    need(len(invitation["keys"]) < 16, "INVITATION_KEY_CAPACITY_EXHAUSTED")
    return invitation, materials


def assemble_rearm(config, source, selection, saved, latest, requirements, output_environment, output_rearm,
                   restored_materials):
    """Pure mapping/planning; writes only after all eligibility checks succeed."""
    root = owner_root(config["taskRoot"])
    validate_source(root, source, check_materials=False)
    need(type(restored_materials) is dict and set(restored_materials) == LABELS
         and all(type(value) is bytes and value for value in restored_materials.values()), "RESTORED_MATERIALS_REQUIRED")
    need(core.decode(restored_materials["environment"]) == source, "RESTORED_ENVIRONMENT_MISMATCH")
    core.exact(selection, ("users", "clients", "memberships", "services"))
    core.exact(selection["services"], SERVICE_TABLES)
    users = selection_ids(selection["users"], requirements["eligible_users"])
    clients = selection_ids(selection["clients"], requirements["eligible_clients"])
    memberships = selection_ids(selection["memberships"], requirements["eligible_memberships"])
    services = {table: selection_ids(selection["services"][table], requirements["eligible_services"].get(table, []))
                for table in SERVICE_TABLES}
    current = latest["payload"]["inventory"]["databases"]["platform"]
    original = saved["inventory"]["databases"]["platform"]
    result, fresh = copy.deepcopy(source), Fresh()
    invitation, historical_invitation_material = invitation_history(restored_materials["invitation"])
    # All original invitations are already CONSUMED/REVOKED by reconciliation.
    # Their nonactive keys remain only for exact original-operation recovery;
    # no newly generated credential or active key may reuse those bytes.
    fresh.seen.update(material.hex() for material in historical_invitation_material)
    inputs = [Path(path) for path in config["privateMaterials"].values()]
    output = Outputs(root, inputs)
    request = {"users": [], "clients": [], "memberships": memberships, "services": [], "materials": {}}
    accounts = [(source["ids"][name], ("accounts", name), source["accounts"][name])
                for name in ("admin", "employee", "customer")]
    accounts += [(source["phase21"]["ids"]["user"], ("phase21", "accounts", "user"),
                  source["phase21"]["accounts"]["user"])]
    need(set(users) <= {object_id for object_id, _, _ in accounts}, "ACCOUNT_MAPPING_MISSING")
    result["accounts"], result["phase21"]["accounts"] = {}, {}
    for object_id, location, account in accounts:
        if object_id not in users:
            continue
        for tables in (original, current):
            row = index(tables["users"], "id", object_id)
            need(row["username_sha256"] == inventory.digest(account["username"]), "USERNAME_MAPPING_INVALID")
        password = "Owned-Recovery-" + fresh.secret() + "9!"
        request["users"].append({"id": object_id, "password": password})
        container = result["accounts"] if location[0] == "accounts" else result["phase21"]["accounts"]
        container[location[-1]] = {"username": account["username"], "password": password}
    for object_id in memberships:
        member = index(current["memberships"], "id", object_id)
        need(member["user_id"] in users, "MEMBERSHIP_WITHOUT_USER")
    client_slots = [(source["platform"]["BFF_OIDC_CLIENT_ID"], source["ids"]["client"], "platform"),
                    (source["portal"]["OIDC_CLIENT_ID"], source["ids"]["portalClient"], "portal")]
    client_slots += [(source["phase21"]["publicClients"][key], source["phase21"]["ids"][key + "Client"], key)
                     for key in ("third", "offline", "machine", "resource")]
    need(set(clients) == {object_id for _, object_id, _ in client_slots}, "RUNTIME_CLIENT_SELECTION_INCOMPLETE")
    for public, object_id, slot in client_slots:
        for tables in (original, current):
            row = index(tables["auth_clients"], "client_id", public)
            need(row["id"] == object_id and row["client_type"] == "CONFIDENTIAL", "CLIENT_ID_MAPPING_INVALID")
        secret = fresh.secret()
        request["clients"].append({"id": object_id, "secret": secret})
        if slot == "platform":
            result[slot]["BFF_OIDC_CLIENT_SECRET"] = secret
        elif slot == "portal":
            result[slot]["OIDC_CLIENT_SECRET"] = secret
        else:
            secret_file = output.add(root / ("phase23-rearm-" + slot + "-secret"), secret.encode())
            request["materials"][slot + "_secret"] = secret_file
            if slot in ("third", "offline"):
                result["phase21"]["rp"][slot]["OIDC_CLIENT_SECRET_FILE"] = secret_file
            else:
                result["phase21"][slot]["secretFile"] = secret_file
    bound_services = {table: set() for table in SERVICE_TABLES}
    shared_secrets = {}
    for container, secret_field, id_field, table, lookup, purpose, metadata_id in SERVICE_BINDINGS:
        expected_id = source["phase23"]["ids"][metadata_id] if metadata_id == "keyAdminService" else source["ids"][metadata_id]
        for tables in (original, current):
            row = index(tables[table], lookup, source[container][id_field])
            need(row["id"] == expected_id and row["purpose"] == purpose, "SERVICE_ID_MAPPING_INVALID")
        need(expected_id in services[table], "RUNTIME_SERVICE_SELECTION_INCOMPLETE")
        bound_services[table].add(expected_id)
        key = (table, expected_id)
        if key not in shared_secrets:
            shared_secrets[key] = fresh.secret()
            request["services"].append({"table": table, "id": expected_id, "secret": shared_secrets[key]})
        result[container][secret_field] = shared_secrets[key]
    need(all(bound_services[table] == set(services[table]) for table in SERVICE_TABLES), "SERVICE_MAPPING_MISSING")
    for label, (container, field) in RING_BINDINGS.items():
        kid = "recovery-" + label + "-" + core.uuid7().replace("-", "")
        path = output.add(root / ("phase23-rearm-" + label + "-keys.json"),
                          {"activeKid": kid, "keys": [{"kid": kid, "key": b64(fresh.key())}]})
        result[container][field] = path
        request["materials"][label] = path
    version = max(item["version"] for item in invitation["keys"]) + 1
    invitation_file = output.add(root / "phase23-rearm-invitation-keys.json", {
        "purpose": invitation["purpose"], "activeVersion": version,
        "keys": [*copy.deepcopy(invitation["keys"]), {"version": version, "key": b64(fresh.key())}]})
    result["identity"]["AUTH_INVITATION_KEYSET_PATH"] = invitation_file
    request["materials"]["invitation"] = invitation_file
    result["identity"]["SSO_TOTP_ENCRYPTION_KEY"] = fresh.secret()
    result["platform"]["JWT_SECRET"] = fresh.secret()
    for container in ("identity", "platform", "portal"):
        for key in METRICS_KEYS & result[container].keys():
            result[container][key] = fresh.secret()
    result["phase20"]["collector"]["controlSecret"] = fresh.secret()
    for label in ("signing", "tls_ca", "tls_certificate", "tls_key"):
        path = output.add(root / ("phase23-rearm-" + label + ".material"),
                          restored_materials[label])
        request["materials"][label] = path
    result["identity"]["OIDC_KEYSET_PATH"] = request["materials"]["signing"]
    for container, field in CA_BINDINGS:
        result[container][field] = request["materials"]["tls_ca"]
    for rp in result["phase21"]["rp"].values():
        rp["NODE_EXTRA_CA_CERTS"] = request["materials"]["tls_ca"]
    runtime_passwords = {}
    for container in ("identity", "platform", "portal"):
        kind = "portal" if container == "portal" else "platform"
        parsed = urlsplit(source[container]["DATABASE_URL"])
        old = connection(source[container]["DATABASE_URL"], unquote(parsed.username), config["databases"][kind]["source"])
        password = fresh.secret()
        runtime_passwords[container] = {"role": old["PGUSER"], "password": password}
        result[container]["DATABASE_URL"] = urlunsplit((parsed.scheme,
            quote(old["PGUSER"], safe="") + ":" + quote(password, safe="") + "@127.0.0.1:55494",
            "/" + config["databases"][kind]["target"], "", ""))
    runtime_bytes = core.canonical(runtime_passwords)
    result["phase23"]["runtimePasswordsFile"] = output.add(root / "phase23-rearm-runtime-passwords.json", runtime_bytes)
    result["phase23"]["runtimePasswordsSha256"] = core.sha(runtime_bytes)
    request["materials"]["environment"] = output.add(output_environment, result)
    need(set(request["materials"]) == set(config["privateMaterials"]) == LABELS, "MATERIAL_LABELS_INVALID")
    output.add(output_rearm, request)
    output.commit()
    return {"status": "PHASE23_RECOVERY_REARM_FILES_READY", "users": len(users), "clients": len(clients),
            "services": len(request["services"]), "memberships": len(memberships), "runtimePasswords": 3,
            "materials": len(request["materials"]), "sourceUnchanged": True, "executesDatabaseWrites": False}


def rearm(config_file, environment_file, selection_file, output_environment, output_rearm, minimum_sequence):
    config = core.load_config(config_file)
    root = owner_root(config["taskRoot"])
    material = core.keys(config)
    saved = core.checkpoint(config, material)
    latest, floor = core.latest_witness(config, material, saved, minimum_sequence)
    receipt = core.authenticated_read(config["workDirectory"] / "start.json", material["mac"], "start")
    need(receipt["checkpointSha256"] == core.sha(saved) and receipt["sequence"] == floor["sequence"]
         and receipt["floorSha256"] == core.sha(floor) and receipt["format"] == core.FORMAT
         and receipt["targets"] == {kind: database["target"] for kind, database in config["databases"].items()}
         and "rearmed" not in receipt, "START_REQUIREMENTS_UNBOUND")
    # Every source material is compared with authenticated checkpoint bytes.
    # This stops a changed source environment from substituting ID/secret maps.
    archived = core.decode(core.decrypt(config, material, "materials", saved["archives"]["materials"], saved["binding"]))
    need(type(archived) is dict and set(archived) == set(config["privateMaterials"]) == LABELS, "MATERIAL_LABELS_INVALID")
    restored_materials = {}
    for label in config["privateMaterials"]:
        try:
            expected = base64.b64decode(archived[label], validate=True)
        except (ValueError, TypeError):
            need(False, "ARCHIVED_MATERIAL_INVALID")
        restored = core.private_read(config["workDirectory"] / "restored-materials" / (label + ".material"))
        need(restored == expected, "RESTORED_MATERIAL_CHANGED")
        restored_materials[label] = restored
    environment_path = path_in(root, environment_file)
    need(environment_path == config["privateMaterials"]["environment"], "ENVIRONMENT_ASSET_MISMATCH")
    environment_bytes = core.private_read(environment_path)
    need(environment_bytes == restored_materials["environment"], "SOURCE_ENVIRONMENT_CHANGED")
    source = core.decode(restored_materials["environment"])
    selection = read_json(root, selection_file)
    need(type(minimum_sequence) is int and minimum_sequence >= 1, "EXTERNAL_MINIMUM_SEQUENCE_REQUIRED")
    return assemble_rearm(config, source, selection, saved, latest, receipt["requirements"],
                          output_environment, output_rearm, restored_materials)


def main(argv=None):
    parser = argparse.ArgumentParser(description="Protected-file-only Phase23 recovery adapter; no database or services")
    sub = parser.add_subparsers(dest="operation", required=True)
    before = sub.add_parser("prepare")
    before.add_argument("--task-dir", type=Path, required=True)
    before.add_argument("--environment", type=Path, required=True)
    before.add_argument("--manifest", type=Path, required=True)
    before.add_argument("--output-config", type=Path, required=True)
    before.add_argument("--tls-materials", type=Path, required=True)
    after = sub.add_parser("rearm")
    after.add_argument("--config", type=Path, required=True)
    after.add_argument("--environment", type=Path, required=True)
    after.add_argument("--selection", type=Path, required=True)
    after.add_argument("--output-env", type=Path, required=True)
    after.add_argument("--output-rearm", type=Path, required=True)
    after.add_argument("--minimum-sequence", type=int, required=True)
    args = parser.parse_args(argv)
    if args.operation == "prepare":
        result = prepare(args.task_dir, args.environment, args.manifest, args.output_config, args.tls_materials)
    else:
        result = rearm(args.config, args.environment, args.selection, args.output_env, args.output_rearm,
                       args.minimum_sequence)
    print(core.canonical(result).decode())


if __name__ == "__main__":
    try:
        main()
    except Exception:
        print("PHASE23_RECOVERY_FILES_REJECTED", file=sys.stderr)
        raise SystemExit(1) from None
