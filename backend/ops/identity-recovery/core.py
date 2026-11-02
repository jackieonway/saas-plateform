"""Canonical, offline recovery. No import-time I/O and no legacy fallback.

The environment owner stops every writer and retains the independently observed
minimum witness sequence. This module never changes HBA, stops PG, drops a source
database, prints SQL/credentials, or starts an application.
"""
import base64
import contextlib
import datetime
import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import secrets
import stat
import subprocess
import time
import uuid

from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.argon2 import Argon2id
from cryptography.exceptions import InvalidKey

FORMAT = "identity-recovery/2"
MAX_FILE = 512 * 1024 * 1024
MAX_JSON = 32 * 1024 * 1024
IDENT = re.compile(r"[a-z][a-z0-9_]{0,62}\Z")
HEX = re.compile(r"[0-9a-f]{64}\Z")
DOMAIN = b"saas-identity/canonical-recovery/2\0"


class RecoveryError(RuntimeError):
    """Only fixed, non-sensitive error categories cross the CLI boundary."""


def need(condition, code):
    if not condition:
        raise RecoveryError("RECOVERY_" + code)


def canonical(value):
    try:
        return json.dumps(value, sort_keys=True, separators=(",", ":"),
                          ensure_ascii=True, allow_nan=False).encode("ascii")
    except (ValueError, TypeError, OverflowError):
        raise RecoveryError("RECOVERY_JSON_INVALID") from None


def pairs(values):
    result = {}
    for key, value in values:
        need(key not in result, "DUPLICATE_FIELD")
        result[key] = value
    return result


def decode(data):
    need(len(data) <= MAX_JSON, "JSON_TOO_LARGE")
    try:
        return json.loads(data, object_pairs_hook=pairs,
                          parse_constant=lambda _: (_ for _ in ()).throw(ValueError()))
    except (UnicodeError, ValueError, RecursionError):
        raise RecoveryError("RECOVERY_JSON_INVALID") from None


def sha(value):
    return hashlib.sha256(value if isinstance(value, bytes) else canonical(value)).hexdigest()


def exact(value, names):
    need(type(value) is dict and set(value) == set(names), "FIELDS_INVALID")


def identifier(value):
    need(type(value) is str and IDENT.fullmatch(value), "IDENTIFIER_INVALID")
    return '"' + value + '"'


def literal(value):
    if value is None:
        return "NULL"
    if type(value) is bool:
        return "true" if value else "false"
    if type(value) is int:
        need(0 <= value < 2 ** 63 - 1, "INTEGER_INVALID")
        return str(value)
    need(type(value) is str and len(value) <= MAX_JSON and "\0" not in value,
         "LITERAL_INVALID")
    return "E'" + value.replace("\\", "\\\\").replace("'", "''") + "'"


def safe_path(path, root=None):
    value = Path(path)
    need(value.is_absolute() and not any(part in (".", "..") for part in value.parts), "PATH_INVALID")
    current = Path(value.anchor)
    for part in value.parts[1:]:
        current /= part
        if current.exists() or current.is_symlink():
            need(not current.is_symlink(), "SYMLINK_REJECTED")
    if root is not None:
        need(value != root and value.is_relative_to(root), "PATH_OUTSIDE_TASK")
    return value


@contextlib.contextmanager
def parent_descriptor(path):
    """Resolve every ancestor through O_NOFOLLOW directory descriptors.

    An ancestor swapped after validation cannot redirect read/create/replace to
    another directory; all effects use the retained directory inode.
    """
    path = Path(path)
    need(path.is_absolute() and path.name and not any(part in (".", "..") for part in path.parts),
         "PATH_INVALID")
    fd = os.open(path.anchor, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for part in path.parts[1:-1]:
            next_fd = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = next_fd
        yield fd, path.name
    finally:
        os.close(fd)


def read_at(parent, name, maximum):
    # A FIFO must not block before fstat can reject it as a non-regular artifact.
    fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
    try:
        info = os.fstat(fd)
        need(stat.S_ISREG(info.st_mode) and info.st_nlink == 1 and info.st_uid == os.getuid()
             and stat.S_IMODE(info.st_mode) == 0o600 and 0 < info.st_size <= maximum,
             "PRIVATE_FILE_INVALID")
        with os.fdopen(fd, "rb", closefd=False) as stream:
            data = stream.read(maximum + 1)
        need(len(data) == info.st_size, "PRIVATE_FILE_CHANGED")
        return data
    finally:
        os.close(fd)


def private_read(path, maximum=MAX_JSON):
    path = safe_path(path)
    try:
        with parent_descriptor(path) as (parent, name):
            return read_at(parent, name, maximum)
    except OSError:
        raise RecoveryError("RECOVERY_PRIVATE_FILE_IO") from None


def sync_dir(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def private_write(path, data, replace=False):
    path = safe_path(path)
    if not isinstance(data, bytes):
        data = canonical(data)
    need(0 < len(data) <= MAX_FILE, "OUTPUT_SIZE_INVALID")
    temporary = ".recovery-write-" + secrets.token_hex(12) if replace else path.name
    try:
        with parent_descriptor(path) as (parent, name):
            fd, created = None, False
            try:
                if replace:
                    read_at(parent, name, MAX_FILE)
                fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                             0o600, dir_fd=parent)
                created = True
                with os.fdopen(fd, "wb", closefd=False) as stream:
                    stream.write(data)
                    stream.flush()
                    os.fsync(fd)
                if replace:
                    os.replace(temporary, name, src_dir_fd=parent, dst_dir_fd=parent)
                    created = False
                os.fsync(parent)
            finally:
                if fd is not None:
                    os.close(fd)
                if replace and created:
                    os.unlink(temporary, dir_fd=parent)
    except OSError:
        raise RecoveryError("RECOVERY_PRIVATE_WRITE_FAILED") from None


def private_directory(path, create=False):
    path = safe_path(path)
    with parent_descriptor(path) as (parent, name):
        if create:
            os.mkdir(name, mode=0o700, dir_fd=parent)
            os.fsync(parent)
        fd = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
        try:
            info = os.fstat(fd)
            need(stat.S_ISDIR(info.st_mode) and stat.S_IMODE(info.st_mode) == 0o700
                 and info.st_uid == os.getuid(), "PRIVATE_DIRECTORY_INVALID")
        finally:
            os.close(fd)
    return path


def tag(key, label, value):
    return hmac.new(key, DOMAIN + label.encode("ascii") + b"\0" + canonical(value), hashlib.sha256).hexdigest()


def authenticated_write(path, value, key, label, replace=False):
    private_write(path, {"document": value, "tag": tag(key, label, value)}, replace)


def authenticated_read(path, key, label):
    envelope = decode(private_read(path))
    exact(envelope, ("document", "tag"))
    need(type(envelope["tag"]) is str and hmac.compare_digest(
        envelope["tag"], tag(key, label, envelope["document"])), "AUTHENTICATION_FAILED")
    return envelope["document"]


def execute(argv, env, data=None, timeout=300, maximum=MAX_JSON):
    try:
        result = subprocess.run(argv, input=data, env=env, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired):
        raise RecoveryError("RECOVERY_TOOL_UNAVAILABLE_OR_TIMEOUT") from None
    need(result.returncode == 0, "TOOL_FAILED")
    need(len(result.stdout) <= maximum, "TOOL_OUTPUT_TOO_LARGE")
    return result.stdout


class Pg:
    def __init__(self, config, database):
        self.config, self.database = config, database
        values = decode(private_read(config["adminConnectionFile"]))
        allowed = {"PGHOST", "PGPORT", "PGUSER", "PGDATABASE", "PGPASSWORD"}
        need(type(values) is dict and set(values) <= allowed and all(type(v) is str for v in values.values()),
             "CONNECTION_FIELDS_INVALID")
        need(values.get("PGHOST") == "127.0.0.1" and values.get("PGPORT") == "55494"
             and values.get("PGUSER") == "postgres" and values.get("PGDATABASE") == "postgres",
             "ADMIN_CONNECTION_REJECTED")
        self.env = {"PATH": config["pgBin"] + ":/usr/bin:/bin", "LANG": "C.UTF-8",
                    "HOME": str(config["workDirectory"]), "PGCONNECT_TIMEOUT": "5",
                    "PGAPPNAME": "identity-recovery-v2", "PGOPTIONS":
                    "-c statement_timeout=30000 -c lock_timeout=10000 -c log_statement=none "
                    "-c log_min_error_statement=panic -c log_error_verbosity=terse", **values}
        self.env["PGDATABASE"] = database

    def query(self, sql):
        data = execute([str(Path(self.config["pgBin"]) / "psql"), "-X", "-qAt", "-v",
                        "ON_ERROR_STOP=1", "-v", "VERBOSITY=sqlstate"], self.env, sql.encode())
        return data.decode("utf-8").strip()

    def json(self, sql):
        return decode(self.query(sql).encode())

    def tool(self, name, args, timeout=900):
        return execute([str(Path(self.config["pgBin"]) / name), *args], self.env,
                       timeout=timeout, maximum=MAX_FILE)


CONFIG_FIELDS = ("format", "taskId", "clusterId", "issuer", "taskRoot", "workDirectory",
                 "witnessDirectory", "floorFile", "keyFile", "adminConnectionFile", "databases",
                 "privateMaterials", "pgBin")


def load_config(path):
    need(os.name == "posix" and hasattr(os, "O_NOFOLLOW"), "POSIX_REQUIRED")
    config = decode(private_read(path))
    exact(config, CONFIG_FIELDS)
    need(config["format"] == FORMAT and config["issuer"] == "https://auth.saas.test", "BINDING_REJECTED")
    # The private, owner-bound configuration pins this rehearsal's actual PG
    # cluster. A historic fixture identifier would prevent a newly initialized
    # cluster from running; accepting an arbitrary JSON number would lose uint64
    # precision in callers. guard() still compares the exact observed identifier.
    cluster_id = config["clusterId"]
    need(type(cluster_id) is str and re.fullmatch(r"[1-9][0-9]{0,19}", cluster_id)
         and int(cluster_id) < (1 << 64), "CLUSTER_ID_INVALID")
    try:
        task_id = uuid.UUID(config["taskId"])
    except (ValueError, TypeError, AttributeError):
        raise RecoveryError("RECOVERY_TASK_ID_INVALID") from None
    need(task_id.version == 7 and str(task_id) == config["taskId"], "TASK_ID_INVALID")
    root = safe_path(config["taskRoot"])
    need(root.parent == Path("/tmp") and root.name.startswith("rust-saas-unified-auth-"),
         "TASK_ROOT_INVALID")
    private_directory(root)
    owner = root / "OWNER"
    need(owner.is_file() and not owner.is_symlink()
         and owner.read_text().strip() == root.name + "-pg55494", "TASK_OWNER_MISMATCH")
    for field in ("workDirectory", "witnessDirectory", "floorFile", "keyFile", "adminConnectionFile"):
        config[field] = safe_path(config[field], root)
    work, witness = config["workDirectory"], config["witnessDirectory"]
    need(work != witness and not work.is_relative_to(witness) and not witness.is_relative_to(work),
         "WITNESS_INSIDE_BACKUP")
    for field in ("floorFile", "keyFile"):
        need(not config[field].is_relative_to(work) and not config[field].is_relative_to(witness),
             "EXTERNAL_FLOOR_REQUIRED")
    need(config["pgBin"] == "/usr/lib/postgresql/18/bin", "PG_TOOLS_REJECTED")
    exact(config["databases"], ("platform", "portal"))
    names = []
    for kind, database in config["databases"].items():
        exact(database, ("source", "target", "owner", "sourceMarker", "targetMarker", "connectionFile"))
        for field in ("source", "target", "owner"):
            identifier(database[field])
        source_prefix = "unified_auth_https_" if kind == "platform" else "unified_auth_portal_"
        target_prefix = "unified_auth_restore_" if kind == "platform" else "unified_auth_restore_portal_"
        need(re.fullmatch(re.escape(source_prefix) + r"[0-9a-f]{32}", database["source"])
             and database["target"] == target_prefix + database["source"][-32:], "DATABASE_NAME_REJECTED")
        need(config["taskId"] in database["sourceMarker"] and database["targetMarker"] ==
             "phase23-recovery/2/" + config["taskId"], "DATABASE_MARKER_REJECTED")
        database["connectionFile"] = safe_path(database["connectionFile"], root)
        names += [database["source"], database["target"]]
    need(len(set(names)) == 4, "DATABASES_NOT_DISTINCT")
    need(type(config["privateMaterials"]) is dict and config["privateMaterials"], "MATERIALS_REQUIRED")
    for label, value in config["privateMaterials"].items():
        need(re.fullmatch(r"[a-z][a-z0-9_]{0,40}", label), "MATERIAL_LABEL_REJECTED")
        config["privateMaterials"][label] = safe_path(value, root)
        need(config["privateMaterials"][label] not in (config["keyFile"], config["floorFile"])
             and not config["privateMaterials"][label].is_relative_to(work)
             and not config["privateMaterials"][label].is_relative_to(witness), "EXTERNAL_KEYS_IN_BACKUP")
    return config


def keys(config):
    value = decode(private_read(config["keyFile"]))
    exact(value, ("format", "encryption", "mac", "witness"))
    need(value["format"] == FORMAT, "KEY_FORMAT_REJECTED")
    result = {}
    try:
        for name in ("encryption", "mac", "witness"):
            result[name] = base64.b64decode(value[name], validate=True)
            need(len(result[name]) == 32, "KEY_INVALID")
    except (ValueError, TypeError):
        raise RecoveryError("RECOVERY_KEY_INVALID") from None
    need(len(set(result.values())) == 3, "KEY_REUSE")
    return result


def initialize(config):
    need(not any(config[field].exists() for field in
                 ("workDirectory", "witnessDirectory", "floorFile", "keyFile")), "ALREADY_INITIALIZED")
    private_directory(config["workDirectory"], True)
    private_directory(config["witnessDirectory"], True)
    private_write(config["keyFile"], {"format": FORMAT, **{
        name: base64.b64encode(secrets.token_bytes(32)).decode() for name in ("encryption", "mac", "witness")}})
    return {"status": "INITIALIZED", "format": FORMAT}


CATALOG_SQL = """SELECT jsonb_build_object(
 'schemas',(SELECT jsonb_agg(jsonb_build_array(nspname,pg_get_userbyid(nspowner),nspacl::text) ORDER BY nspname) FROM pg_namespace WHERE nspname='public'),
 'relations',(SELECT jsonb_agg(jsonb_build_array(c.relname,c.relkind,pg_get_userbyid(c.relowner),COALESCE(c.relacl,pg_catalog.acldefault(CASE WHEN c.relkind='S' THEN 'S'::"char" ELSE 'r'::"char" END,c.relowner))::text,c.relrowsecurity,c.relforcerowsecurity) ORDER BY c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public'),
 'columns',(SELECT jsonb_agg(jsonb_build_array(c.relname,a.attname,format_type(a.atttypid,a.atttypmod),a.attnotnull,a.attacl::text,pg_get_expr(d.adbin,d.adrelid)) ORDER BY c.relname,a.attnum) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace JOIN pg_attribute a ON a.attrelid=c.oid LEFT JOIN pg_attrdef d ON d.adrelid=c.oid AND d.adnum=a.attnum WHERE n.nspname='public' AND a.attnum>0 AND NOT a.attisdropped),
 'functions',(SELECT jsonb_agg(jsonb_build_array(p.oid::regprocedure::text,pg_get_userbyid(p.proowner),p.proacl::text,p.prosecdef,p.proconfig,encode(sha256(convert_to(pg_get_functiondef(p.oid),'UTF8')),'hex')) ORDER BY p.oid::regprocedure::text) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public'),
 'indexes',(SELECT jsonb_agg(jsonb_build_array(c.relname,x.relname,pg_get_indexdef(i.indexrelid),i.indisunique,i.indisvalid,i.indisready,pg_get_expr(i.indpred,i.indrelid),pg_get_expr(i.indexprs,i.indrelid)) ORDER BY c.relname,x.relname) FROM pg_index i JOIN pg_class c ON c.oid=i.indrelid JOIN pg_class x ON x.oid=i.indexrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public'),
 'default_acl',(SELECT jsonb_agg(jsonb_build_array(pg_get_userbyid(d.defaclrole),n.nspname,d.defaclobjtype,d.defaclacl::text) ORDER BY pg_get_userbyid(d.defaclrole),n.nspname,d.defaclobjtype) FROM pg_default_acl d LEFT JOIN pg_namespace n ON n.oid=d.defaclnamespace),
 'constraints',(SELECT jsonb_agg(jsonb_build_array(c.relname,x.conname,pg_get_constraintdef(x.oid,true),x.convalidated) ORDER BY c.relname,x.conname) FROM pg_constraint x JOIN pg_class c ON c.oid=x.conrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public'),
 'triggers',(SELECT jsonb_agg(jsonb_build_array(c.relname,t.tgname,t.tgenabled,pg_get_triggerdef(t.oid,true)) ORDER BY c.relname,t.tgname) FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND NOT t.tgisinternal),
 'migrations',(SELECT jsonb_agg(jsonb_build_array(version,success,encode(checksum,'hex')) ORDER BY version) FROM _sqlx_migrations))::text"""


def database_record(config, name):
    return Pg(config, "postgres").json("""SELECT jsonb_build_object('name',datname,'owner',pg_get_userbyid(datdba),
        'comment',shobj_description(oid,'pg_database'),'connections',datconnlimit,
        'acl',(SELECT jsonb_agg(jsonb_build_array(pg_get_userbyid(grantor),CASE WHEN grantee=0 THEN 'PUBLIC' ELSE pg_get_userbyid(grantee) END,privilege_type,is_grantable) ORDER BY grantor::regrole::text,grantee::regrole::text,privilege_type,is_grantable) FROM aclexplode(COALESCE(datacl,acldefault('d',datdba)))))
        FROM pg_database WHERE datname=""" + literal(name))


def guard(config, target=False, quiet=True):
    admin = Pg(config, "postgres")
    need(admin.query("SELECT system_identifier::text FROM pg_control_system()") == config["clusterId"],
         "CLUSTER_MISMATCH")
    result = {}
    for kind, database in config["databases"].items():
        name = database["target" if target else "source"]
        record = database_record(config, name)
        need(type(record) is dict and record["owner"] == database["owner"] and record["comment"] ==
             database["targetMarker" if target else "sourceMarker"], "DATABASE_OWNERSHIP_MISMATCH")
        if quiet:
            need(admin.query("SELECT count(*) FROM pg_stat_activity WHERE datname=" + literal(name)) == "0",
                 "WRITERS_NOT_STOPPED")
        pg = Pg(config, name)
        need(pg.query("SELECT count(*)=1 AND bool_and(version=1 AND success) FROM _sqlx_migrations") == "t",
             "CANONICAL_MIGRATION_REQUIRED")
        if kind == "platform":
            need(pg.query("SELECT count(*)=1 AND bool_and(schema_version=1 AND issuer=" + literal(config["issuer"]) +
                          " AND deployment_id=" + literal(config["taskId"]) + "::uuid) FROM identity_deployment_state") == "t",
                 "DEPLOYMENT_MISMATCH")
        result[kind] = record
    return result


def inventory_capture(config, target=False):
    import inventory
    return inventory.capture(lambda kind, sql: Pg(config, config["databases"][kind][
        "target" if target else "source"]).json(sql))


def assert_catalogs(config, saved, target=True):
    need(role_facts(config) == saved["roles"], "ROLE_POWERS_OR_MEMBERSHIP_CHANGED")
    for kind, database in config["databases"].items():
        pg = Pg(config, database["target" if target else "source"])
        need(pg.json(CATALOG_SQL) == saved["catalogs"][kind], "CATALOG_OR_ACL_CHANGED")
        if target:
            record = database_record(config, database["target"])
            need(record["owner"] == saved["databases"][kind]["owner"]
                 and record["acl"] == saved["databases"][kind]["acl"]
                 and record["comment"] == database["targetMarker"], "DATABASE_ACL_CHANGED")
            need(record["connections"] in (0, saved["databases"][kind]["connections"]),
                 "DATABASE_CONNECTION_LIMIT_CHANGED")


def role_facts(config):
    """Pin task principals without serializing password verifiers."""
    platform = Pg(config, config["databases"]["platform"]["source"])
    state = platform.json("SELECT to_jsonb(d) FROM identity_deployment_state d WHERE singleton")
    fields = ("migration_owner", "identity_owner", "platform_owner", "projection_owner",
              "revocation_bridge_owner", "identity_runtime", "platform_runtime")
    names = {state[field] for field in fields}
    portal_owner = config["databases"]["portal"]["owner"]
    need(portal_owner.endswith("_portal_owner"), "TASK_ROLE_SET_INVALID")
    names.update((portal_owner, portal_owner[:-len("_owner")] + "_runtime"))
    need(len(names) == 8, "TASK_ROLE_SET_INVALID")
    owner_suffix = "_migration_owner"
    migration_owner = config["databases"]["platform"]["owner"]
    need(migration_owner.endswith(owner_suffix), "TASK_ROLE_SET_INVALID")
    prefix = migration_owner[:-len(owner_suffix)] + "_"
    for name in names:
        identifier(name)
        need(name.startswith(prefix), "TASK_ROLE_PREFIX_MISMATCH")
    names_sql = "ARRAY[" + ",".join(literal(name) for name in sorted(names)) + "]::text[]"
    facts = Pg(config, "postgres").json("SELECT jsonb_build_object('roles',"
        "(SELECT jsonb_agg(jsonb_build_array(rolname,rolsuper,rolcreatedb,rolcreaterole,rolinherit,"
        "rolreplication,rolbypassrls,rolcanlogin,rolvaliduntil,rolconnlimit,rolconfig,shobj_description(oid,'pg_authid')) ORDER BY rolname) "
        "FROM pg_roles WHERE rolname=ANY(" + names_sql + ")), 'memberships',"
        "(SELECT COALESCE(jsonb_agg(jsonb_build_array(pg_get_userbyid(roleid),pg_get_userbyid(member),"
        "pg_get_userbyid(grantor),admin_option,inherit_option,set_option) "
        "ORDER BY roleid,member),'[]'::jsonb) FROM pg_auth_members WHERE pg_get_userbyid(roleid)=ANY("
        + names_sql + ") OR pg_get_userbyid(member)=ANY(" + names_sql + ")))")
    need(len(facts["roles"] or []) == 8 and facts["memberships"] == [], "TASK_ROLE_SET_INVALID")
    for row in facts["roles"]:
        need(not any(row[1:7]) and row[7] == row[0].endswith("_runtime"), "TASK_ROLE_POWERS_INVALID")
    return facts


def encrypt(config, material, kind, plaintext, binding):
    nonce = secrets.token_bytes(12)
    aad = canonical({"binding": binding, "kind": kind, "format": FORMAT})
    ciphertext = nonce + AESGCM(material["encryption"]).encrypt(nonce, plaintext, aad)
    path = config["workDirectory"] / (kind + ".aesgcm")
    private_write(path, ciphertext)
    return {"filename": path.name, "sha256": sha(ciphertext), "bytes": len(ciphertext),
            "plaintextSha256": sha(plaintext), "plaintextBytes": len(plaintext)}


def decrypt(config, material, kind, record, binding):
    exact(record, ("filename", "sha256", "bytes", "plaintextSha256", "plaintextBytes"))
    need(record["filename"] == kind + ".aesgcm", "ARCHIVE_NAME_REJECTED")
    ciphertext = private_read(config["workDirectory"] / record["filename"], MAX_FILE)
    need(len(ciphertext) == record["bytes"] and sha(ciphertext) == record["sha256"], "ARCHIVE_DIGEST_MISMATCH")
    try:
        plaintext = AESGCM(material["encryption"]).decrypt(ciphertext[:12], ciphertext[12:],
            canonical({"binding": binding, "kind": kind, "format": FORMAT}))
    except Exception:
        raise RecoveryError("RECOVERY_ARCHIVE_AUTHENTICATION_FAILED") from None
    need(len(plaintext) == record["plaintextBytes"] and sha(plaintext) == record["plaintextSha256"],
         "PLAINTEXT_DIGEST_MISMATCH")
    return plaintext


def checkpoint(config, material):
    value = authenticated_read(config["workDirectory"] / "checkpoint.json", material["mac"], "checkpoint")
    exact(value, ("format", "binding", "createdAtMs", "recoveryPointMs", "databases", "catalogs", "roles", "inventory", "archives"))
    need(value["format"] == FORMAT and value["binding"] == binding(config), "CHECKPOINT_BINDING_MISMATCH")
    import inventory
    inventory.validate_inventory(value["inventory"])
    return value


def binding(config):
    return {"task_id": config["taskId"], "cluster_id": config["clusterId"], "issuer": config["issuer"],
            "databases": {kind: {"name": database["source"], "marker": database["sourceMarker"]}
                          for kind, database in config["databases"].items()}}


def witness_store(config, material, saved):
    from witness import WitnessStore
    return WitnessStore(config["witnessDirectory"], material["witness"],
                        dict(binding(config), checkpoint_sha256=sha(saved)))


def save_floor(config, material, saved, latest, replace=False, facts=None):
    sequence = latest["document"]["sequence"] if "document" in latest else latest["sequence"]
    document = latest["document"] if "document" in latest else latest
    floor = {"format": FORMAT, "checkpointSha256": sha(saved), "sequence": sequence,
             "witnessSha256": sha(document), "highwaterSha256": sha(document["payload"]["highwater"]),
             "facts": facts}
    authenticated_write(config["floorFile"], floor, material["mac"], "floor", replace)
    return floor


def latest_witness(config, material, saved, minimum_sequence):
    need(type(minimum_sequence) is int and minimum_sequence >= 1, "EXTERNAL_MINIMUM_SEQUENCE_REQUIRED")
    floor = authenticated_read(config["floorFile"], material["mac"], "floor")
    exact(floor, ("format", "checkpointSha256", "sequence", "witnessSha256", "highwaterSha256", "facts"))
    need(floor["format"] == FORMAT and floor["checkpointSha256"] == sha(saved)
         and type(floor["sequence"]) is int and floor["sequence"] >= minimum_sequence,
         "EXTERNAL_FLOOR_ROLLBACK")
    latest = witness_store(config, material, saved).load_latest(minimum_sequence=floor["sequence"])
    document = latest["document"] if "document" in latest else latest
    need(document["sequence"] == floor["sequence"] and sha(document) == floor["witnessSha256"]
         and sha(document["payload"]["highwater"]) == floor["highwaterSha256"], "EXTERNAL_FLOOR_MISMATCH")
    return document, floor


def historical_credentials(config, material, saved, latest, floor):
    """Retain overwritten/deleted secret denials and credential ownership.

    Public double hashes suffice to reject a known old secret. Complete verified
    rows retain client_id for a credential that disappeared from latest inventory.
    Never infer that ownership from an opaque highwater key alone.
    """
    history = witness_store(config, material, saved).load_history(minimum_sequence=floor["sequence"])
    need(history and history[-1]["document"]["sequence"] == floor["sequence"]
         and sha(history[-1]["document"]) == floor["witnessSha256"], "HISTORY_FLOOR_MISMATCH")
    denied, versions = set(), {}
    for envelope in history:
        tables = envelope["document"]["payload"]["inventory"]["databases"]["platform"]
        for row in tables["auth_client_credentials"]:
            if row["secret_digest_sha256"] is not None:
                denied.add(row["secret_digest_sha256"])
            versions[row["client_id"]] = max(versions.get(row["client_id"], 0), row["credential_version"])
        for table in ("identity_service_clients", "context_selection_services", "platform_resource_services"):
            denied.update(row["secret_hash_sha256"] for row in tables[table])
    known = {row["id"] for snapshot in (saved["inventory"], latest["payload"]["inventory"])
             for row in snapshot["databases"]["platform"]["auth_clients"]}
    return denied, {client: version for client, version in versions.items() if client in known}


def capture(config, material):
    recovery_point_ms = time.time_ns() // 1_000_000
    records = guard(config)
    import inventory
    saved_inventory, original_facts = inventory.empty_inventory(), {}
    for kind, database in config["databases"].items():
        raw = Pg(config, database["source"]).json(inventory.capture_sql(kind))
        saved_inventory["databases"][kind] = inventory.project_database(kind, raw)
        original_facts[kind] = raw["tables"]
    inventory.validate_inventory(saved_inventory)
    catalogs = {kind: Pg(config, database["source"]).json(CATALOG_SQL)
                for kind, database in config["databases"].items()}
    roles = role_facts(config)
    archives = {}
    archives["checkpoint-facts"] = encrypt(config, material, "checkpoint-facts", canonical(original_facts), binding(config))
    for kind, database in config["databases"].items():
        dump = Pg(config, database["source"]).tool("pg_dump", ["--format=custom", "--no-password", "--lock-wait-timeout=10s"])
        need(dump[:5] == b"PGDMP", "CUSTOM_DUMP_REQUIRED")
        archives[kind] = encrypt(config, material, kind, dump, binding(config))
    materials = {label: base64.b64encode(private_read(path, MAX_FILE)).decode()
                 for label, path in config["privateMaterials"].items()}
    archives["materials"] = encrypt(config, material, "materials", canonical(materials), binding(config))
    guard(config)
    need(inventory_capture(config) == saved_inventory, "SOURCE_CHANGED_DURING_CAPTURE")
    saved = {"format": FORMAT, "binding": binding(config), "createdAtMs": time.time_ns() // 1_000_000,
             "recoveryPointMs": recovery_point_ms,
             "databases": records, "catalogs": catalogs, "roles": roles,
             "inventory": saved_inventory, "archives": archives}
    authenticated_write(config["workDirectory"] / "checkpoint.json", saved, material["mac"], "checkpoint")
    from witness import make_payload
    payload = make_payload(saved_inventory, config["taskId"], saved["createdAtMs"], {})
    store = witness_store(config, material, saved)
    store.append(payload, expected_sequence=0)
    latest = store.load_latest(minimum_sequence=1)
    floor = save_floor(config, material, saved, latest)
    return {"status": "CHECKPOINT_CAPTURED", "checkpointSha256": sha(saved),
            "sequence": floor["sequence"], "archiveBytes": sum(value["bytes"] for value in archives.values()),
            "crossDatabaseAtomicSnapshotClaim": False, "pitrClaim": False}


def capture_deny(config, material, minimum_sequence):
    saved = checkpoint(config, material)
    previous, _ = latest_witness(config, material, saved, minimum_sequence)
    guard(config)
    import inventory
    latest_inventory = inventory.empty_inventory()
    original_facts = {}
    for kind, database in config["databases"].items():
        raw = Pg(config, database["source"]).json(inventory.capture_sql(kind))
        latest_inventory["databases"][kind] = inventory.project_database(kind, raw)
        original_facts[kind] = raw["tables"]
    inventory.validate_inventory(latest_inventory)
    from witness import make_payload
    payload = make_payload(latest_inventory, config["taskId"], time.time_ns() // 1_000_000, {},
                           previous=previous["payload"])
    next_sequence = previous["sequence"] + 1
    facts = encrypt(config, material, "deny-facts-%08d" % next_sequence,
                    canonical(original_facts), saved["binding"])
    guard(config)
    need(inventory_capture(config) == latest_inventory, "SOURCE_CHANGED_DURING_DENY_CAPTURE")
    store = witness_store(config, material, saved)
    store.append(payload, expected_sequence=previous["sequence"])
    latest = store.load_latest(minimum_sequence=next_sequence)
    floor = save_floor(config, material, saved, latest, replace=True, facts=facts)
    return {"status": "DENY_CAPTURED", "sequence": floor["sequence"],
            "witnessSha256": floor["witnessSha256"], "highwaterSha256": floor["highwaterSha256"]}


def restore_new(config, material, minimum_sequence):
    saved = checkpoint(config, material)
    latest_witness(config, material, saved, minimum_sequence)
    guard(config)
    admin = Pg(config, "postgres")
    # Authenticate every artifact and material before creating the first target.
    plaintext = {kind: decrypt(config, material, kind, record, saved["binding"])
                 for kind, record in saved["archives"].items()}
    for kind in ("platform", "portal"):
        need(plaintext[kind][:5] == b"PGDMP", "CUSTOM_DUMP_REQUIRED")
        database = config["databases"][kind]
        need(admin.query("SELECT count(*) FROM pg_database WHERE datname=" + literal(database["target"])) == "0",
             "TARGET_ALREADY_EXISTS")
    created = []
    try:
        for kind in ("platform", "portal"):
            database, record = config["databases"][kind], saved["databases"][kind]
            name = identifier(database["target"])
            admin.query("CREATE DATABASE " + name + " OWNER " + identifier(database["owner"]) + " TEMPLATE template0")
            created.append(kind)
            admin.query("COMMENT ON DATABASE " + name + " IS " + literal(database["targetMarker"]) +
                        "; REVOKE ALL ON DATABASE " + name + " FROM PUBLIC; ALTER DATABASE " + name + " CONNECTION LIMIT 0")
            for grantor, grantee, privilege, option in record["acl"]:
                need(privilege in ("CONNECT", "CREATE", "TEMPORARY"), "ACL_PRIVILEGE_INVALID")
                admin.query("SET ROLE " + identifier(grantor) + "; GRANT " + privilege + " ON DATABASE " + name +
                            " TO " + ("PUBLIC" if grantee == "PUBLIC" else identifier(grantee)) +
                            (" WITH GRANT OPTION" if option else "") + "; RESET ROLE")
            # Restore from stdin: no raw dump path or connection secret in argv.
            pg = Pg(config, database["target"])
            execute([str(Path(config["pgBin"]) / "pg_restore"), "--exit-on-error", "--single-transaction",
                     "--no-password", "--dbname=" + database["target"]], pg.env, plaintext[kind], timeout=900)
            need(pg.json(CATALOG_SQL) == saved["catalogs"][kind], "RESTORED_CATALOG_MISMATCH")
            actual = database_record(config, database["target"])
            need(actual["owner"] == record["owner"] and actual["acl"] == record["acl"]
                 and actual["comment"] == database["targetMarker"], "RESTORED_DATABASE_ACL_MISMATCH")
        need(inventory_capture(config, True) == saved["inventory"], "RESTORED_DATA_MISMATCH")
        directory = config["workDirectory"] / "restored-materials"
        private_directory(directory, True)
        materials = decode(plaintext["materials"])
        need(set(materials) == set(config["privateMaterials"]), "MATERIAL_INVENTORY_MISMATCH")
        for label, value in materials.items():
            private_write(directory / (label + ".material"), base64.b64decode(value, validate=True))
        authenticated_write(config["workDirectory"] / "restore.json", {
            "format": FORMAT, "checkpointSha256": sha(saved), "created": created,
            "restoredAtMs": time.time_ns() // 1_000_000}, material["mac"], "restore")
        return {"status": "RESTORED_QUARANTINED", "databaseCount": len(created), "catalogsEqual": True,
                "materialsEqual": True, "oldCredentialsInvalidated": False}
    except Exception:
        # Never drop anything here. Environment-owner finally inspects and removes
        # only these exact, commented, task-owned targets. The failed target stays
        # CONNECTION LIMIT 0 and no start receipt exists.
        raise RecoveryError("RECOVERY_RESTORE_QUARANTINED") from None


def reconcile(config, material, minimum_sequence):
    saved = checkpoint(config, material)
    latest, floor = latest_witness(config, material, saved, minimum_sequence)
    guard(config, True)
    assert_catalogs(config, saved)
    import reconcile as policy
    restored = inventory_capture(config, True)
    _, credential_floors = historical_credentials(config, material, saved, latest, floor)
    raw_latest = None
    if floor["facts"] is not None:
        raw_latest = decode(decrypt(config, material, "deny-facts-%08d" % floor["sequence"],
                                    floor["facts"], saved["binding"]))
    plan = policy.plan(restored, latest["payload"]["inventory"], config["taskId"], raw_latest=raw_latest,
                       external_highwater=latest["payload"]["highwater"],
                       external_credential_floors=credential_floors)
    for kind in ("platform", "portal"):
        pg = Pg(config, config["databases"][kind]["target"])
        pg.query("BEGIN; SET LOCAL statement_timeout='60s'; SET LOCAL lock_timeout='10s';\n" +
                 ";\n".join(plan[kind]) + ";\nCOMMIT;")
    verification = policy.verify_sql(restored, latest["payload"]["inventory"], config["taskId"],
                                     raw_latest=raw_latest, external_highwater=latest["payload"]["highwater"],
                                     external_credential_floors=credential_floors)
    for kind in ("platform", "portal"):
        need(Pg(config, config["databases"][kind]["target"]).query(verification[kind]) == "t",
             "INVALIDATION_INCOMPLETE")
    fenced = inventory_capture(config, True)
    receipt = {"format": FORMAT, "checkpointSha256": sha(saved), "sequence": floor["sequence"],
               "floorSha256": sha(floor), "fencedInventorySha256": sha(fenced), "requirements": plan["requirements"],
               "targets": {kind: database["target"] for kind, database in config["databases"].items()},
               "reconciledAtMs": time.time_ns() // 1_000_000}
    authenticated_write(config["workDirectory"] / "start.json", receipt, material["mac"], "start")
    return {"status": "RECONCILED_QUARANTINED", "sequence": floor["sequence"],
            "fencedInventorySha256": sha(fenced), "requirements": plan["requirements"]}


def gate(config, material, minimum_sequence):
    saved = checkpoint(config, material)
    latest, floor = latest_witness(config, material, saved, minimum_sequence)
    receipt = authenticated_read(config["workDirectory"] / "start.json", material["mac"], "start")
    guard(config, True)
    assert_catalogs(config, saved)
    if "rearmed" in receipt:
        need(material_digests(config, receipt["materialPaths"]) == receipt["materialDigests"],
             "REARM_MATERIAL_CHANGED")
    actual = inventory_capture(config, True)
    assert_registration_targets(saved, latest, receipt, actual)
    need(receipt["checkpointSha256"] == sha(saved) and receipt["floorSha256"] == sha(floor)
         and receipt["sequence"] == floor["sequence"] and receipt["targets"] == {
             kind: database["target"] for kind, database in config["databases"].items()}
         and receipt["fencedInventorySha256"] == sha(actual), "START_GATE_ROLLBACK")
    return {"status": "GATE_PASSED_QUARANTINED", "sequence": floor["sequence"],
            "fencedInventorySha256": receipt["fencedInventorySha256"], "opensTraffic": False,
            "expectedConnectionLimits": {database["target"]: saved["databases"][kind]["connections"]
                                         for kind, database in config["databases"].items()}}


def assert_registration_targets(saved, latest, receipt, actual):
    import reconcile as policy
    selected = receipt.get("rearmed", {})
    expected = policy.registration_targets(saved["inventory"]["databases"]["platform"],
        latest["payload"]["inventory"]["databases"]["platform"], receipt["requirements"],
        selected.get("clients", ()), selected.get("services", {}))
    observed = policy.indexed("platform", "context_selection_registrations",
        actual["databases"]["platform"]["context_selection_registrations"])
    need(observed == expected, "REGISTRATION_BINDING_OR_GENERATION_CHANGED")
    clients = policy.indexed("platform", "auth_clients", actual["databases"]["platform"]["auth_clients"])
    applications = policy.indexed("platform", "applications", actual["databases"]["platform"]["applications"])
    need(all(not row["enabled"] or (clients.get((row["client_id"],), {}).get("policy_version")
             == row["client_policy_version"] and clients[(row["client_id"],)]["status"] == "ENABLED"
             and applications.get((row["application_id"],), {}).get("status") == "ENABLED"
             and applications[(row["application_id"],)]["access_policy_version"] == row["application_policy_version"]
             and row["member_type"] in applications[(row["application_id"],)]["allowed_member_types"])
             for row in expected.values()),
         "REGISTRATION_BINDING_OR_GENERATION_CHANGED")


def status(config, material, minimum_sequence):
    saved = checkpoint(config, material)
    latest, floor = latest_witness(config, material, saved, minimum_sequence)
    return {"status": "VERIFIED", "sequence": floor["sequence"], "checkpointSha256": sha(saved),
            "witnessSha256": sha(latest), "sourceCheckpointAgeSeconds":
            max(0, (time.time_ns() // 1_000_000 - saved["recoveryPointMs"]) // 1000)}


def uuid7():
    """Generate a fresh internal UUIDv7 without an extra dependency."""
    value = ((time.time_ns() // 1_000_000) << 80) | (7 << 76) | (secrets.randbits(12) << 64)
    value |= (2 << 62) | secrets.randbits(62)
    return str(uuid.UUID(int=value))


def raw_facts(config, material, saved, floor):
    if floor["facts"] is None:
        return None
    value = decode(decrypt(config, material, "deny-facts-%08d" % floor["sequence"],
                           floor["facts"], saved["binding"]))
    exact(value, ("platform", "portal"))
    return value


def selected_ids(rows, fields, eligible):
    need(type(rows) is list and len(rows) <= 1000, "REARM_LIST_INVALID")
    seen = set()
    for row in rows:
        exact(row, fields)
        need(type(row["id"]) is str and row["id"] in eligible and row["id"] not in seen,
             "REARM_NOT_ELIGIBLE")
        seen.add(row["id"])
    return sorted(seen)


def material_digests(config, paths):
    need(type(paths) is dict and set(paths) == set(config["privateMaterials"]), "REARM_MATERIALS_INCOMPLETE")
    return {label: sha(private_read(safe_path(path, config["taskRoot"]), MAX_FILE))
            for label, path in paths.items()}


def fresh_secret_digests(request, snapshots, historical_secret_hashes=()):
    """Deny old secrets across *all* directions and never share a fresh one."""
    denied = set()
    for snapshot in snapshots:
        denied.update(row["secret_digest"] for row in snapshot["auth_client_credentials"]
                      if row["secret_digest"] is not None)
        for table in ("identity_service_clients", "context_selection_services", "platform_resource_services"):
            denied.update(row["secret_hash"] for row in snapshot[table])
    seen, result = set(), {}
    # User passwords also cannot equal a new confidential service/client secret.
    passwords = {sha(row["password"].encode()) for row in request["users"]
                 if type(row.get("password")) is str}
    for row in [*request["clients"], *request["services"]]:
        secret = row.get("secret")
        need(type(secret) is str and 32 <= len(secret) <= 1024, "NEW_SECRET_INVALID")
        digest = sha(secret.encode())
        need(digest not in denied, "OLD_SECRET_REUSED")
        # Inventory hashes the stored SHA256 hex value as canonical JSON. This
        # also rejects an intermediate overwritten service secret, without ever
        # retaining that raw secret in the witness or output.
        need(sha(digest) not in historical_secret_hashes, "OLD_SECRET_REUSED")
        need(digest not in seen and digest not in passwords, "CROSS_PURPOSE_SECRET_REUSE")
        seen.add(digest)
        result[(row.get("table", "auth_clients"), row["id"])] = digest
    return result


def membership_rearm_sql(object_id):
    # protect_membership_identity rejects a status transition unless the same
    # UPDATE supplies a newer generation. Quarantine's floor is not an exemption.
    return ("UPDATE public.memberships SET status='ENABLED',identity_version=identity_version+1 "
            "WHERE id=" + literal(object_id))


def rearm(config, material, minimum_sequence, rearm_file):
    """Explicit task-fixture recovery; never auto-enable an unknown principal.

    All old carriers remain closed. Only dual-snapshot eligible IDs receive new
    passwords/credentials. No service purpose, scope, owner, subject, role, issuer
    or resource binding is edited here. Production human recovery authorization
    and secret distribution are intentionally outside this isolated task CLI.
    """
    gate(config, material, minimum_sequence)
    saved = checkpoint(config, material)
    latest, floor = latest_witness(config, material, saved, minimum_sequence)
    receipt_path = config["workDirectory"] / "start.json"
    receipt = authenticated_read(receipt_path, material["mac"], "start")
    need("rearmed" not in receipt, "REARM_ALREADY_APPLIED")
    request = decode(private_read(safe_path(rearm_file, config["taskRoot"])))
    exact(request, ("users", "clients", "memberships", "services", "materials"))
    requirements = receipt["requirements"]
    users = selected_ids(request["users"], ("id", "password"), requirements["eligible_users"])
    clients = selected_ids(request["clients"], ("id", "secret"), requirements["eligible_clients"])
    need(type(request["memberships"]) is list and len(set(request["memberships"])) == len(request["memberships"])
         and set(request["memberships"]) <= set(requirements["eligible_memberships"]), "REARM_NOT_ELIGIBLE")
    memberships = sorted(request["memberships"])
    need(type(request["services"]) is list and len(request["services"]) <= 1000, "REARM_LIST_INVALID")
    services = {}
    for table in ("identity_service_clients", "context_selection_services", "platform_resource_services"):
        entries = [row for row in request["services"] if type(row) is dict and row.get("table") == table]
        services[table] = selected_ids(entries, ("id", "table", "secret"),
                                       requirements["eligible_services"].get(table, []))
    need(sum(map(len, services.values())) == len(request["services"]), "REARM_SERVICE_INVALID")
    new_materials = material_digests(config, request["materials"])
    raw = raw_facts(config, material, saved, floor)
    need(raw is not None, "LATEST_FACTS_REQUIRED_FOR_REARM")
    import inventory
    current = latest["payload"]["inventory"]
    for kind, tables in raw.items():
        need(set(tables) == set(inventory.TABLES[kind]), "RAW_FACTS_INCOMPLETE")
        for table, rows in tables.items():
            expected = current["databases"][kind][table]
            projected = [inventory.project_row(kind, table, row) for row in rows]
            spec = inventory.TABLES[kind][table]
            need(sorted(projected, key=lambda row: inventory.row_key(spec, row)) == expected,
                 "RAW_FACT_WITNESS_MISMATCH")
    source = raw["platform"]
    original_source = decode(decrypt(config, material, "checkpoint-facts",
                                     saved["archives"]["checkpoint-facts"], saved["binding"]))["platform"]
    historical_denials, credential_floors = historical_credentials(config, material, saved, latest, floor)
    fresh = fresh_secret_digests(request, (source, original_source), historical_denials)
    need(all(requirements["credential_floors"].get(client, -1) >= version
             for client, version in credential_floors.items()
             if client in requirements["credential_floors"]), "START_CREDENTIAL_FLOOR_REGRESSION")
    sql = []
    for entry in request["users"]:
        password = entry["password"]
        need(type(password) is str and 12 <= len(password.encode()) <= 1024, "NEW_PASSWORD_INVALID")
        originals = [next(row for row in rows["users"] if row["id"] == entry["id"])
                     for rows in (source, original_source)]
        for original in originals:
            try:
                Argon2id.verify_phc_encoded(password.encode(), original["password_hash"])
            except InvalidKey:
                pass
            except (ValueError, TypeError):
                raise RecoveryError("RECOVERY_ORIGINAL_PASSWORD_FORMAT_INVALID") from None
            else:
                raise RecoveryError("RECOVERY_OLD_PASSWORD_REUSED")
        phc = Argon2id(salt=secrets.token_bytes(16), length=32, iterations=2,
                      lanes=1, memory_cost=32768).derive_phc_encoded(password.encode())
        sql.append("UPDATE public.users SET password_hash=" + literal(phc) +
                   ",status='ENABLED',updated_at=clock_timestamp() WHERE id=" + literal(entry["id"]))
    for entry in request["clients"]:
        secret = entry["secret"]
        need(type(secret) is str and 32 <= len(secret) <= 1024, "NEW_SECRET_INVALID")
        digest = fresh[("auth_clients", entry["id"])]
        original = next(row for row in source["auth_clients"] if row["id"] == entry["id"])
        need(original["client_type"] == "CONFIDENTIAL", "REARM_PUBLIC_CLIENT_INVALID")
        version = requirements["credential_floors"][entry["id"]] + 1
        sql += ["UPDATE public.auth_clients SET status='ENABLED' WHERE id=" + literal(entry["id"]),
                "INSERT INTO public.auth_client_credentials(id,client_id,client_type,credential_version,secret_digest,expires_at) VALUES(" +
                literal(uuid7()) + "," + literal(entry["id"]) + ",'CONFIDENTIAL'," + str(version) + "," + literal(digest) +
                ",clock_timestamp()+interval '90 days')"]
    for entry in request["services"]:
        secret = entry["secret"]
        need(type(secret) is str and 32 <= len(secret) <= 1024, "NEW_SECRET_INVALID")
        table = entry["table"]
        digest = fresh[(table, entry["id"])]
        # platform_resource_services advances its credential generation in the
        # existing trigger when the digest changes; do not fight that trigger.
        sql.append("UPDATE public." + identifier(table) + " SET secret_hash=" + literal(digest)
                   + ",status='ENABLED' WHERE id=" + literal(entry["id"]))
    for object_id in memberships:
        original = next(row for row in source["memberships"] if row["id"] == object_id)
        need(original["user_id"] in users, "REARM_MEMBER_WITHOUT_USER")
        need(requirements["membership_floors"][object_id] < (1 << 63) - 2, "GENERATION_EXHAUSTED")
        sql.append(membership_rearm_sql(object_id))
    import reconcile as policy
    registrations = policy.registration_targets(saved["inventory"]["databases"]["platform"],
        current["databases"]["platform"], requirements, clients, services)
    for row in registrations.values():
        if row["enabled"]:
            sql.append(policy.update("platform", "context_selection_registrations", row,
                "enabled=true,client_policy_version=" + str(row["client_policy_version"])))
    Pg(config, config["databases"]["platform"]["target"]).query(
        "BEGIN; SET LOCAL statement_timeout='60s';\n" + ";\n".join(sql) + ";\nCOMMIT;")
    verification = policy.verify_sql(saved["inventory"], current, config["taskId"],
        allowed_rearmed_users=users, allowed_rearmed_clients=clients,
        allowed_rearmed_memberships=memberships, allowed_rearmed_services=services, raw_latest=raw,
        external_highwater=latest["payload"]["highwater"], external_credential_floors=credential_floors)
    for kind in ("platform", "portal"):
        need(Pg(config, config["databases"][kind]["target"]).query(verification[kind]) == "t",
             "REARM_INVALIDATION_INCOMPLETE")
    assert_catalogs(config, saved)
    receipt["rearmed"] = {"users": users, "clients": clients, "memberships": memberships, "services": services}
    receipt["materialDigests"] = new_materials
    receipt["materialPaths"] = request["materials"]
    receipt["fencedInventorySha256"] = sha(inventory_capture(config, True))
    authenticated_write(receipt_path, receipt, material["mac"], "start", replace=True)
    return {"status": "REARMED_QUARANTINED", "sequence": floor["sequence"], "users": len(users),
            "clients": len(clients), "services": sum(map(len, services.values())), "memberships": len(memberships),
            "opensTraffic": False}
