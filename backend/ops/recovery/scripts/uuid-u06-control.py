#!/usr/bin/env python3
"""Fixed local U06 control plane: fresh targets, real migrations and U05 freeze.

No runtime generation, source DROP/TRUNCATE, seed, restore or service restart.
Every phase uses a fresh private directory with fsynced intent before mutation.
Run --check first. --freeze-u05 is reserved for root's explicit stop-window call.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import inspect
import json
import os
from pathlib import Path
import re
import runpy
import signal
import stat
import subprocess
import sys
import time
from urllib.parse import urlencode

FORMAT = "saas-uuid-u06-control/1"
BASE = Path("/var/lib/saas-uuid-u06-control-20261024")
RUNTIME = Path("/var/lib/saas-uuid-u05-runtime-20261024")
LOCK = 823140620261024
SCRIPT = Path(__file__).resolve()
AUDIT = runpy.run_path(str(SCRIPT.with_name("verify-uuid-u05-databases.py")))
DATA = runpy.run_path(str(SCRIPT.with_name("uuid-u06-data.py")))
SEMANTIC_HELPER_SHA256 = hashlib.sha256("\n".join(inspect.getsource(DATA[n]) for n in
    ("canonical", "value_sha", "semantic_material")).encode()).hexdigest()
KINDS = ("platform", "stores", "work_orders")
TARGETS = tuple({"set": target_set, "kind": kind,
                 "name": f"qiankun_{stem}_uuid7_{target_set}_20261024",
                 "owner": f"qiankun_{stem}_uuid7_{target_set}_owner_20261024"}
                for target_set in ("final", "recovery")
                for kind, stem in zip(KINDS, ("platform", "stores", "orders")))
MIGRATORS = {
    "platform": ("/usr/local/libexec/saas-platform-migrate-u04", "c8fb69f437f059a682c681bd27f58e5b4d9eee6c9bd2dcf51effa77597827aa1", "DATABASE_URL"),
    "stores": ("/usr/local/libexec/saas-stores-migrate-u04", "9d6a64285e0367ccb3e23f55c073c975758638e648394fc9ea25d4ce48b444dc", "STORES_DATABASE_URL"),
    "work_orders": ("/usr/local/libexec/saas-orders-migrate-u04", "7d5af8de46af8a8fc3e56fb2f0fa26fb001f78700a97e4ac1bf4786ee3c3d2bf", "WORK_ORDERS_DATABASE_URL"),
}
HISTORY_SHA256 = {
    "platform": "80706cce23384465caf0e66a2a3056b4b003f6ee1ea47ea1d8095e985efbb2c3",
    "stores": "0ee5122f007f1b19e75e5fa0f8974f7da3944522473619bfdfdaf91b62b3b0e7",
    "work_orders": "ceb4a5cd8aa674a201bd98f4e2831b89e20b40c91b38bbca4a35eac9c39c479a",
}
WRITERS = {
    "gateway": "qiankun-uuid-u05-gateway-r2-20261024",
    **{service: "qiankun-uuid-u05-" + service.replace("_", "-") + "-20261024"
       for service in ("platform", "stores", "work_orders", "notification", "examples_oauth", "examples_chat")},
}
STOP_ORDER = ("gateway", "examples_oauth", "examples_chat", "work_orders", "stores", "notification", "platform")
RUNTIME_IMAGE = "sha256:42e1b7fb5fea569fba2abada59030185286c9d642643cb7fa63ed7a5715b87b7"
GATEWAY_IMAGE = "sha256:3b03dfd827515a07ccf4946000f0bbdd2a4fb6aca7499249fcb6b1f9726823d6"
RUNTIME_BINARIES = {
    "platform": "45049fa89b9272920448e49a5ac3af9e78d59c741dc6b65d68cfe9d33f8e03f0",
    "stores": "87aa8b97132a9a3d73afa7b7d2a1b1e5b341cbe0891d81a5dd9b2c89413ca119",
    "work_orders": "998eedd23231103d572a8d88345c1992edea7fea77cc221b19a5742b825e01cc",
    "notification": "2e84eafa51408112be8de6d940732832b48ba4ca5ae7d7962fd6dd08f1aee6ea",
    "examples_oauth": "fb36627dc4441b3ed4c5acbd60b7e53cec66fcb884aeb21d16c273a08525c02f",
    "examples_chat": "0f5378497cd1b1371a8a74e73a3713137cff1069d77db8d74e4b17a3c4452874",
}


class Invalid(Exception):
    """Static public category; command output is retained privately."""


def require(value, code):
    if not value:
        raise Invalid(code)


def now():
    return datetime.now(timezone.utc).isoformat()


def sha_file(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def literal(value):
    return "'" + value.replace("'", "''") + "'"


def ident(value):
    require(bool(re.fullmatch(r"[a-z_][a-z0-9_]{0,62}", value)), "IDENTIFIER_INVALID")
    return '"' + value + '"'


def private(path, directory=False):
    require(path.is_absolute() and ".." not in path.parts
            and all(not p.is_symlink() for p in (path, *path.parents)), "PRIVATE_PATH_INVALID")
    s = path.lstat()
    require(s.st_uid == 0 and stat.S_IMODE(s.st_mode) == (0o700 if directory else 0o600)
            and (stat.S_ISDIR(s.st_mode) if directory else stat.S_ISREG(s.st_mode) and s.st_nlink == 1), "PRIVATE_METADATA_INVALID")


def fsync_dir(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def write_new(path, value):
    raw = json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2).encode() + b"\n"
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as out:
        os.fchmod(out.fileno(), 0o600)
        out.write(raw)
        out.flush()
        os.fsync(out.fileno())
    fsync_dir(path.parent)
    require(path.read_bytes() == raw, "PRIVATE_WRITE_READBACK_FAILED")
    return hashlib.sha256(raw).hexdigest()


def read_receipt(path):
    private(path.parent, True)
    private(path)
    raw = path.read_bytes()
    require(len(raw) <= 16 * 1024 * 1024, "RECEIPT_TOO_LARGE")
    value = AUDIT["parsed"](raw)
    require(value.get("format") == FORMAT and value.get("stage") in ("PREPARED", "CREATED", "MIGRATED", "SEALED"), "RECEIPT_STAGE_INVALID")
    require(value.get("script_sha256") == sha_file(SCRIPT), "CONTROL_SCRIPT_CHANGED")
    require(value.get("semantic_helper_sha256") == SEMANTIC_HELPER_SHA256, "SEMANTIC_HELPER_CHANGED")
    require(value.get("target_definitions") == list(TARGETS), "FIXED_TARGET_SET_MISMATCH")
    validate_targets(value["stage"], value.get("targets"))
    return value, hashlib.sha256(raw).hexdigest()


def validate_targets(stage, targets):
    require(isinstance(targets, list), "RECEIPT_TARGET_LIST_INVALID")
    if stage == "PREPARED":
        require(targets == [], "PREPARED_TARGETS_NOT_EMPTY")
        return
    require(len(targets) == len(TARGETS), "RECEIPT_TARGET_COUNT_INVALID")
    for target, known in zip(TARGETS, targets):
        require(isinstance(known, dict) and all(known.get(k) == v for k, v in target.items()), "RECEIPT_TARGET_IDENTITY_INVALID")
        require(all(type(known.get(k)) is int and 0 < known[k] <= 4294967295 for k in ("oid", "owner_oid")), "RECEIPT_TARGET_OID_INVALID")
        if stage in ("MIGRATED", "SEALED", "FROZEN_U05_V7_SOURCES"):
            require(all(isinstance(known.get(k), str) and re.fullmatch(r"[0-9a-f]{64}", known[k])
                        for k in ("raw_schema_sha256", "semantic_schema_sha256")), "RECEIPT_TARGET_SCHEMA_INVALID")
    require(len({t["oid"] for t in targets}) == len(TARGETS)
            and len({t["owner_oid"] for t in targets}) == len(TARGETS), "RECEIPT_TARGET_OIDS_DUPLICATED")


class Phase:
    def __init__(self, directory, action, previous, previous_sha):
        require(directory.parent == BASE and not directory.exists() and not directory.is_symlink(), "FRESH_PHASE_DIRECTORY_REQUIRED")
        directory.mkdir(mode=0o700)
        directory.chmod(0o700)
        fsync_dir(BASE)
        self.directory = directory
        self.previous = previous
        self.previous_sha = previous_sha
        self.action = action
        self.started = now()
        self.script_sha256 = sha_file(SCRIPT)
        self.journal = os.open(directory / "events.ndjson", os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        os.fchmod(self.journal, 0o600)
        os.fsync(self.journal)
        write_new(directory / "intent.json", {"format": FORMAT, "action": action, "started_at_utc": self.started,
                  "previous_receipt_sha256": previous_sha, "script_sha256": self.script_sha256, "target_definitions": TARGETS,
                  "semantic_helper_sha256": SEMANTIC_HELPER_SHA256,
                  "database_mutation_started": False, "source_freeze_only_when_action_is_freeze_u05": True})

    def event(self, operation, **fields):
        raw = json.dumps({"at_utc": now(), "operation": operation, **fields}, sort_keys=True).encode() + b"\n"
        os.write(self.journal, raw)
        os.fsync(self.journal)

    def close(self):
        os.close(self.journal)


def environment(readonly=True):
    return {"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8",
            "PGPASSFILE": "/dev/null", "PGCONNECT_TIMEOUT": "10",
            "PGOPTIONS": "-c default_transaction_read_only=" + ("on" if readonly else "off")
            + " -c statement_timeout=120000 -c lock_timeout=3000"}


def pg_command(database):
    allowed = {"postgres", *(t["name"] for t in TARGETS), *(v[0] for v in AUDIT["TARGETS"].values())}
    require(database in allowed, "FIXED_DATABASE_REQUIRED")
    return ["/usr/sbin/runuser", "-u", "postgres", "--", "/usr/bin/psql", "-X", "-qAt", "--no-password",
            "-v", "ON_ERROR_STOP=1", "-h", "/run/postgresql", "-p", "5432", "-U", "postgres", "-d", database, "--file=-"]


def sql(statement, database="postgres", readonly=True, phase=None):
    prefix = "BEGIN ISOLATION LEVEL READ COMMITTED READ ONLY;\nSET LOCAL search_path=pg_catalog;\nSET LOCAL timezone='UTC';\n" if readonly else "SET search_path=pg_catalog;\nSET timezone='UTC';\n"
    result = subprocess.run(pg_command(database), input=prefix + statement + ("\nROLLBACK;\n" if readonly else "\n"),
                            text=True, capture_output=True, env=environment(readonly), timeout=150)
    if result.returncode:
        if phase:
            write_new(phase.directory / ("sql-failure-" + str(time.time_ns()) + ".json"), {"stdout": result.stdout, "stderr": result.stderr, "exit": result.returncode})
        raise Invalid("CONTROL_SQL_FAILED_RETAIN_QUARANTINE")
    return [AUDIT["parsed"](line) for line in result.stdout.splitlines()]


class Advisory:
    def __init__(self, phase):
        self.stderr = (phase.directory / "advisory.stderr").open("xb")
        os.chmod(phase.directory / "advisory.stderr", 0o600)
        self.process = subprocess.Popen(pg_command("postgres"), stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=self.stderr, text=True, env=environment())
        self.process.stdin.write(f"SELECT pg_try_advisory_lock({LOCK});\n")
        self.process.stdin.flush()
        require(self.process.stdout.readline().strip() == "t", "U06_CONTROL_LOCK_BUSY")

    def alive(self):
        require(self.process.poll() is None, "CONTROL_LOCK_SESSION_LOST")

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
        self.stderr.close()


def cluster():
    value = sql("""SELECT jsonb_build_object('system_identifier',(SELECT system_identifier::text FROM pg_control_system()),
     'postmaster_start',pg_postmaster_start_time()::text,'server_version_num',current_setting('server_version_num')::int,
     'data_directory',current_setting('data_directory'),'port',current_setting('port')::int,
     'actor_valid',current_user='postgres' AND session_user='postgres' AND inet_server_addr() IS NULL,
     'postmaster_pid_file',pg_read_file('postmaster.pid',0,256));""")[0]
    require(value["actor_valid"] is True and value["port"] == 5432 and value["server_version_num"] == 180006, "FIXED_PG18_PEER_REQUIRED")
    lines = value.pop("postmaster_pid_file").splitlines()
    pid = int(lines[0])
    require(lines[1] == value["data_directory"], "POSTMASTER_PID_DIRECTORY_MISMATCH")
    text = Path(f"/proc/{pid}/stat").read_text()
    value.update({"postmaster_pid": pid, "postmaster_start_ticks": text.rsplit(")", 1)[1].split()[19], "pidfile_started_epoch": lines[2]})
    return value


def binaries():
    result = {}
    for kind, (raw, expected, _) in MIGRATORS.items():
        path = Path(raw)
        s = path.lstat()
        require(not path.is_symlink() and stat.S_ISREG(s.st_mode) and s.st_uid == s.st_gid == 0
                and stat.S_IMODE(s.st_mode) == 0o755 and s.st_nlink == 1 and sha_file(path) == expected, "FROZEN_MIGRATOR_IDENTITY_INVALID")
        result[kind] = {"path": raw, "sha256": expected, "bytes": s.st_size, "inode": s.st_ino, "device": s.st_dev}
    return result


def new_names():
    dbs = ",".join(literal(t["name"]) for t in TARGETS)
    owners = ",".join(literal(t["owner"]) for t in TARGETS)
    return sql(f"""SELECT jsonb_build_object(
      'databases',(SELECT COALESCE(jsonb_agg(jsonb_build_object('name',datname,'oid',oid::bigint,'owner_oid',datdba::bigint) ORDER BY datname),'[]') FROM pg_database WHERE datname IN ({dbs})),
      'roles',(SELECT COALESCE(jsonb_agg(jsonb_build_object('name',rolname,'oid',oid::bigint,'login',rolcanlogin,'superuser',rolsuper,
       'createdb',rolcreatedb,'createrole',rolcreaterole,'replication',rolreplication,'bypassrls',rolbypassrls) ORDER BY rolname),'[]') FROM pg_roles WHERE rolname IN ({owners})));""")[0]


def source_generation(frozen=False, phase=None):
    expected, schemas, bindings = AUDIT["load_public_inputs"]()
    roles = AUDIT["runtime_roles"](expected)
    sources = []
    for kind in KINDS:
        name = expected[kind]["database"]
        metadata, material = sql("""SELECT jsonb_build_object('database',current_database(),'oid',d.oid::bigint,'owner',r.rolname,'owner_oid',d.datdba::bigint,
          'owner_nologin',NOT r.rolcanlogin,'owner_non_superuser',NOT r.rolsuper,
          'default_read_only',EXISTS(SELECT 1 FROM pg_db_role_setting s WHERE s.setdatabase=d.oid AND s.setrole=0 AND s.setconfig @> ARRAY['default_transaction_read_only=on']),
          'marker',shobj_description(d.oid,'pg_database'),
          'history',(SELECT jsonb_agg(jsonb_build_object('version',version,'success',success,'checksum',encode(checksum,'hex')) ORDER BY version) FROM public._sqlx_migrations),
          'backends',(SELECT count(*) FROM pg_stat_activity WHERE datid=d.oid AND pid<>pg_backend_pid()),
          'prepared',(SELECT count(*) FROM pg_prepared_xacts WHERE database=d.datname)) FROM pg_database d JOIN pg_roles r ON r.oid=d.datdba WHERE d.datname=current_database();\n""" + schemas[kind], name)
        e = expected[kind]
        require(metadata["oid"] == e["database_oid"] and metadata["owner_oid"] == e["owner_oid"] and metadata["owner"] == e["owner"] and metadata["marker"] == e["marker"]
                and metadata["owner_nologin"] and metadata["owner_non_superuser"] and metadata["default_read_only"]
                and metadata["prepared"] == 0 and AUDIT["digest"](material) == e["schema_sha256"], "U05_SOURCE_GENERATION_CHANGED")
        AUDIT["check_history"](kind, metadata["history"])
        if frozen:
            require(metadata["backends"] == 0, "U05_SOURCE_NOT_IDLE")
        history = [{"version": r["version"], "success": r["success"], "checksum_sha384": r["checksum"]} for r in metadata["history"]]
        require(AUDIT["digest"](history) == HISTORY_SHA256[kind], "SOURCE_CANONICAL_HISTORY_CHANGED")
        material_file = None
        if phase:
            material_file = str(phase.directory / ("schema-source-" + kind + ".json"))
            write_new(Path(material_file), material)
        sources.append({"kind": kind, **metadata, "schema_sha256": e["schema_sha256"], "schema_material_file": material_file,
                        "semantic_schema_sha256": DATA["value_sha"](DATA["semantic_material"](material)),
                        "migration_history": history, "migration_history_sha256": HISTORY_SHA256[kind],
                        "registration": {"kind": kind, "name": name, "oid": metadata["oid"], "owner": metadata["owner"],
                                         "owner_oid": metadata["owner_oid"], "expected_schema_sha256": e["schema_sha256"],
                                         "semantic_schema_sha256": DATA["value_sha"](DATA["semantic_material"](material))}})
    role_list = ",".join(literal(v) for v in roles.values())
    role_proof = sql(f"""SELECT jsonb_agg(jsonb_build_object('name',r.rolname,'oid',r.oid::bigint,'login',r.rolcanlogin,
      'superuser',r.rolsuper,'createdb',r.rolcreatedb,'createrole',r.rolcreaterole,'replication',r.rolreplication,'bypassrls',r.rolbypassrls,
      'settings',COALESCE((SELECT jsonb_agg(jsonb_build_object('database_oid',setdatabase::bigint,'config',setconfig) ORDER BY setdatabase) FROM pg_db_role_setting s WHERE s.setrole=r.oid),'[]')) ORDER BY r.rolname)
      FROM pg_roles r WHERE rolname IN ({role_list});""")[0]
    require(len(role_proof) == 4 and all(not any(r[k] for k in ("superuser", "createdb", "createrole", "replication", "bypassrls"))
            and r["login"] is (not frozen) for r in role_proof), "U05_RUNTIME_ROLE_STATE_INVALID")
    legacy = AUDIT["psql"]("postgres", [AUDIT["source_gate_sql"]()])[0]
    require(legacy["identity_valid"] and legacy["transaction_read_only"] and len(legacy["sources"]) == 3
            and len(legacy["source_roles"]) == 2
            and all(s["default_read_only"] and s["active_backends"] == s["prepared_transactions"] == 0 for s in legacy["sources"])
            and all(not r["login"] and not r["superuser"] for r in legacy["source_roles"]), "LEGACY_V4_FENCE_CHANGED")
    admission = source_admission(sources, role_proof, frozen)
    return {"sources": sources, "runtime_roles": role_proof, "legacy_fences": legacy, "bindings": bindings,
            "admission": admission}


def source_admission(sources, runtime_roles, frozen):
    """Fresh pg_roles closure, including indirect SET and admin-option paths."""
    names = ",".join(literal(s["database"]) for s in sources)
    protected = ",".join(literal(s["owner"]) for s in sources) + "," + ",".join(literal(r["name"]) for r in runtime_roles)
    proof = sql("""SELECT jsonb_build_object('maintenance_database',current_database(),'user',current_user,
      'session_user',session_user,'local',inet_server_addr() IS NULL,'isolation',current_setting('transaction_isolation'),
      'transaction_read_only',current_setting('transaction_read_only')::boolean,
      'default_read_only',current_setting('default_transaction_read_only')::boolean,
      'roles',(SELECT jsonb_agg(jsonb_build_object('name',r.rolname,'oid',r.oid::bigint,'login',r.rolcanlogin,
      'superuser',r.rolsuper,'createrole',r.rolcreaterole,
      'privileged_set',EXISTS(SELECT 1 FROM pg_roles p WHERE (p.rolsuper OR p.rolcreaterole OR p.rolname IN (""" + protected + ")) AND pg_has_role(r.oid,p.oid,'SET'))," +
      "'privileged_set_other',EXISTS(SELECT 1 FROM pg_roles p WHERE p.oid<>r.oid AND (p.rolsuper OR p.rolcreaterole OR p.rolname IN (" + protected + ")) AND pg_has_role(r.oid,p.oid,'SET'))," +
      "'can_admin_source_roles',EXISTS(SELECT 1 FROM pg_auth_members e JOIN pg_roles p ON p.oid=e.roleid WHERE p.rolname IN (" + protected + ") AND e.admin_option AND pg_has_role(r.oid,e.member,'SET'))," +
      "'connect',EXISTS(SELECT 1 FROM pg_database d WHERE d.datname IN (" + names + ") AND has_database_privilege(r.oid,d.oid,'CONNECT'))) ORDER BY r.rolname) FROM pg_roles r));")[0]
    require(proof["maintenance_database"] == "postgres" and proof["user"] == proof["session_user"] == "postgres"
            and proof["local"] and proof["isolation"] == "read committed" and proof["transaction_read_only"]
            and proof["default_read_only"], "SOURCE_ADMISSION_MAINTENANCE_CONTEXT_INVALID")
    roles = {r["name"]: r for r in proof["roles"]}
    require("postgres" in roles and roles["postgres"]["login"] and roles["postgres"]["superuser"], "SOURCE_MAINTENANCE_ROLE_MISSING")
    expected_runtime = {r["name"]: r["oid"] for r in runtime_roles}
    for role, oid in expected_runtime.items():
        observed = roles.get(role)
        require(observed and observed["oid"] == oid and observed["login"] is (not frozen)
                and not any(observed[k] for k in ("superuser", "createrole", "privileged_set_other", "can_admin_source_roles")), "SOURCE_RUNTIME_ROLE_ESCALATION")
    require(not any(r["name"] != "postgres" and r["login"] and (frozen or r["name"] not in expected_runtime)
                    and any(r[k] for k in ("connect", "superuser", "createrole", "privileged_set", "can_admin_source_roles"))
                    for r in proof["roles"]), "OTHER_LOGIN_CAN_WRITE_OR_ESCALATE_SOURCE")
    return proof


def role_guard(role):
    require(type(role["oid"]) is int and role["oid"] > 0, "RUNTIME_ROLE_OID_INVALID")
    ident(role["name"])
    return (f"DO $guard$ BEGIN IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE oid={role['oid']} "
            f"AND rolname={literal(role['name'])} AND rolcanlogin AND NOT "
            "(rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls)) "
            "THEN RAISE EXCEPTION 'runtime role identity changed'; END IF; END; $guard$;")


def target_catalog(target, migrated=False, sealed=False, phase=None):
    name, owner = target["name"], target["owner"]
    meta = sql(f"""SELECT jsonb_build_object('name',d.datname,'oid',d.oid::bigint,'owner',r.rolname,'owner_oid',r.oid::bigint,
      'owner_valid',NOT(r.rolcanlogin OR r.rolsuper OR r.rolcreatedb OR r.rolcreaterole OR r.rolreplication OR r.rolbypassrls),
      'owner_memberships',(SELECT count(*) FROM pg_auth_members WHERE member=r.oid OR roleid=r.oid),
      'allow_connections',d.datallowconn,'encoding',pg_encoding_to_char(d.encoding),
      'default_read_only',EXISTS(SELECT 1 FROM pg_db_role_setting s WHERE s.setdatabase=d.oid AND s.setrole=0 AND s.setconfig @> ARRAY['default_transaction_read_only=on']),
      'public_database_acl_entries',(SELECT count(*) FROM aclexplode(COALESCE(d.datacl,acldefault('d',d.datdba))) a WHERE a.grantee=0),
      'unexpected_database_acl_entries',(SELECT count(*) FROM aclexplode(COALESCE(d.datacl,acldefault('d',d.datdba))) a WHERE a.grantee NOT IN (d.datdba,(SELECT oid FROM pg_roles WHERE rolname='postgres'))),
      'database_acl',d.datacl,'backends',(SELECT count(*) FROM pg_stat_activity WHERE datid=d.oid),
      'prepared',(SELECT count(*) FROM pg_prepared_xacts WHERE database=d.datname),
      'bad_logins',(SELECT count(*) FROM pg_roles l WHERE l.rolcanlogin AND l.rolname<>'postgres' AND
        (l.rolsuper OR l.rolcreaterole OR has_database_privilege(l.oid,d.oid,'CONNECT') OR pg_has_role(l.oid,r.oid,'SET') OR
         EXISTS(SELECT 1 FROM pg_auth_members m WHERE m.roleid=r.oid AND m.admin_option AND pg_has_role(l.oid,m.member,'SET')))))
      FROM pg_database d JOIN pg_roles r ON r.oid=d.datdba WHERE d.datname={literal(name)};""")[0]
    require(meta["owner"] == owner and meta["owner_valid"] and meta["owner_memberships"] == 0 and meta["allow_connections"]
            and meta["encoding"] == "UTF8" and meta["public_database_acl_entries"] == meta["unexpected_database_acl_entries"] == 0
            and meta["backends"] == meta["prepared"] == meta["bad_logins"] == 0
            and (meta["default_read_only"] if sealed else not meta["default_read_only"]), "NEW_TARGET_ADMISSION_FAILED")
    objects = sql(f"""WITH tbl AS MATERIALIZED (SELECT c.oid,c.relname,c.relowner,c.relacl FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind IN ('r','p'))
      SELECT jsonb_build_object('schema_owner_valid',(SELECT nspowner={meta['owner_oid']} FROM pg_namespace WHERE nspname='public'),
      'public_schema_acl_entries',(SELECT count(*) FROM pg_namespace n CROSS JOIN LATERAL aclexplode(COALESCE(n.nspacl,acldefault('n',n.nspowner))) a WHERE n.nspname='public' AND a.grantee=0),
      'table_owner_mismatches',(SELECT count(*) FROM tbl WHERE relowner<>{meta['owner_oid']}),
      'relation_owner_mismatches',(SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind IN ('r','p','i','I','S','v','m','f') AND c.relowner<>{meta['owner_oid']}),
      'function_owner_mismatches',(SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public' AND p.proowner<>{meta['owner_oid']} AND NOT EXISTS(SELECT 1 FROM pg_depend e WHERE e.classid='pg_proc'::regclass AND e.objid=p.oid AND e.deptype='e')),
      'tables',COALESCE((SELECT jsonb_agg(jsonb_build_object('table',t.relname,'owner_oid',t.relowner::bigint,'acl',t.relacl,'rows',x.rows) ORDER BY t.relname)
       FROM tbl t CROSS JOIN LATERAL XMLTABLE('/table/row' PASSING query_to_xml(format('SELECT count(*) AS rows FROM ONLY public.%I',t.relname),true,false,'') COLUMNS rows bigint PATH 'rows') x),'[]'));
    """, name)[0]
    require(objects["schema_owner_valid"] and objects["public_schema_acl_entries"] == 0
            and all(objects[k] == 0 for k in ("table_owner_mismatches", "relation_owner_mismatches", "function_owner_mismatches")), "NEW_TARGET_OBJECT_OWNER_FAILED")
    meta.update(objects)
    if not migrated:
        require(not objects["tables"], "NEW_TARGET_NOT_EMPTY")
        return {**target, **meta}
    material = sql(AUDIT["SCHEMA_FILES"][target["kind"]].read_text(), name)[0]
    history = sql("SELECT jsonb_agg(jsonb_build_object('version',version,'success',success,'checksum',encode(checksum,'hex')) ORDER BY version) FROM public._sqlx_migrations;", name)[0]
    AUDIT["check_history"](target["kind"], history)
    metadata = {"_sqlx_migrations", "_stores_schema_contract", "_stores_schema_contract_v2", "_stores_schema_contract_v3", "work_order_migrations"}
    for table in objects["tables"]:
        if table["table"] not in metadata:
            require(table["rows"] == int(target["kind"] == "platform" and table["table"] == "platform_settings"), "NEW_TARGET_SEED_OR_BUSINESS_ROWS_PRESENT")
        require(table["acl"] is None, "NEW_TARGET_UNEXPECTED_TABLE_ACL")
    material_file = None
    if phase:
        material_file = str(phase.directory / ("schema-" + name + ".json"))
        write_new(Path(material_file), material)
    canonical_history = [{"version": r["version"], "success": r["success"], "checksum_sha384": r["checksum"]} for r in history]
    require(AUDIT["digest"](canonical_history) == HISTORY_SHA256[target["kind"]], "NEW_TARGET_CANONICAL_HISTORY_MISMATCH")
    schema_sha256 = AUDIT["digest"](material)
    semantic_sha256 = DATA["value_sha"](DATA["semantic_material"](material))
    return {**target, **meta, "history": history, "history_sha256": AUDIT["digest"](history),
            "migration_history": canonical_history, "migration_history_sha256": HISTORY_SHA256[target["kind"]],
            "raw_schema_sha256": schema_sha256, "semantic_schema_sha256": semantic_sha256,
            "schema_material_items": len(material), "schema_material_file": material_file,
            "registration": {"kind": target["kind"], "name": name, "oid": meta["oid"], "owner": owner,
                             "owner_oid": meta["owner_oid"], "expected_schema_sha256": schema_sha256,
                             "semantic_schema_sha256": semantic_sha256}}


def group_members(pgid):
    members = []
    for path in Path("/proc").iterdir():
        if path.name.isdigit():
            try:
                fields = (path / "stat").read_text().rsplit(")", 1)[1].split()
                if int(fields[2]) == pgid:
                    members.append(int(path.name))
            except (FileNotFoundError, ProcessLookupError):
                pass
    return members


def close_process_group(process):
    # The runuser process and its migration descendants share this new session.
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait(timeout=10)
    deadline = time.monotonic() + 10
    while group_members(process.pid):
        try:
            while os.waitpid(-process.pid, os.WNOHANG)[0]:
                pass
        except ChildProcessError:
            pass
        require(time.monotonic() < deadline, "MIGRATOR_PROCESS_GROUP_NOT_REAPED")
        time.sleep(0.05)


def target_idle(known, phase, terminate=False):
    """Only the created generation can be inspected or terminated on failure."""
    def observe():
        proof = sql(f"""SELECT jsonb_build_object('identity_valid',EXISTS(SELECT 1 FROM pg_database
         WHERE oid={known['oid']} AND datname={literal(known['name'])} AND datdba={known['owner_oid']}),
         'backends',(SELECT count(*) FROM pg_stat_activity WHERE datid={known['oid']}),
         'prepared',(SELECT count(*) FROM pg_prepared_xacts WHERE database={literal(known['name'])}));""")[0]
        require(proof["identity_valid"] and proof["prepared"] == 0, "MIGRATOR_TARGET_GENERATION_CHANGED")
        return proof
    proof = observe()
    if terminate and proof["backends"]:
        phase.event("migration_cleanup_terminate_new_target_intent", target=known["name"], database_oid=known["oid"])
        # Recheck the exact OID/name/owner in the same statement as termination.
        proof_terminated = sql(f"""SELECT jsonb_build_object('terminated',COALESCE(bool_and(pg_terminate_backend(a.pid)),true))
          FROM pg_stat_activity a JOIN pg_database d ON d.oid=a.datid
          WHERE d.oid={known['oid']} AND d.datname={literal(known['name'])} AND d.datdba={known['owner_oid']};""",
          readonly=False, phase=phase)[0]
        require(proof_terminated["terminated"], "MIGRATOR_BACKEND_TERMINATION_FAILED")
    deadline = time.monotonic() + 10
    while proof["backends"]:
        require(time.monotonic() < deadline, "MIGRATOR_TARGET_BACKENDS_REMAIN")
        time.sleep(0.1)
        proof = observe()
    phase.event("migration_target_idle", target=known["name"], database_oid=known["oid"], backends=0, prepared=0)


def migrate_target(target, known, phase):
    binary, expected_sha, key = MIGRATORS[target["kind"]]
    require(sha_file(binary) == expected_sha, "FROZEN_MIGRATOR_CHANGED")
    url = "postgresql://postgres@localhost:5432/" + target["name"] + "?" + urlencode({"host": "/run/postgresql", "options[role]": target["owner"], "sslmode": "disable"})
    env = {"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8", key: url}
    phase.event("migration_intent", target=target["name"], binary_sha256=expected_sha)
    process, problem = None, None
    with (phase.directory / (target["name"] + ".stdout")).open("xb") as out, (phase.directory / (target["name"] + ".stderr")).open("xb") as err:
        os.fchmod(out.fileno(), 0o600)
        os.fchmod(err.fileno(), 0o600)
        try:
            process = subprocess.Popen(["/usr/sbin/runuser", "-u", "postgres", "--", binary, "--import"],
                                       env=env, stdout=out, stderr=err, start_new_session=True)
            try:
                process.wait(timeout=150)
            except subprocess.TimeoutExpired:
                raise Invalid("MIGRATOR_TIMEOUT_RETAIN_NEW_TARGET_QUARANTINE") from None
            require(process.returncode == 0 and sha_file(binary) == expected_sha, "MIGRATOR_FAILED_RETAIN_NEW_TARGET_QUARANTINE")
        except BaseException as error:
            problem = error
        finally:
            cleanup_problem = None
            try:
                if process is not None:
                    close_process_group(process)
            except BaseException as error:
                cleanup_problem = error
            finally:
                try:
                    out.flush()
                    err.flush()
                    os.fsync(out.fileno())
                    os.fsync(err.fileno())
                    fsync_dir(phase.directory)
                finally:
                    target_idle(known, phase, terminate=problem is not None or cleanup_problem is not None)
            if cleanup_problem is not None:
                raise cleanup_problem
        phase.event("migration_process_group_reaped", target=target["name"], exit_code=process.returncode if process else None,
                    logs_fsynced=True, backends=0)
    if problem is not None:
        raise problem


def docker(arguments):
    result = subprocess.run(["/usr/bin/docker", "--host", "unix:///var/run/docker.sock", *arguments], capture_output=True, text=True,
                            env={"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LC_ALL": "C.UTF-8"}, timeout=90)
    require(result.returncode == 0, "FIXED_DOCKER_OPERATION_FAILED")
    return result.stdout


def process_proof(pid):
    text = Path(f"/proc/{pid}/stat").read_text()
    executable = Path(f"/proc/{pid}/exe")
    info = executable.stat()
    return {"pid": pid, "start_ticks": text.rsplit(")", 1)[1].split()[19], "exe_sha256": sha_file(executable),
            "cmdline_sha256": sha_file(f"/proc/{pid}/cmdline"), "environment_sha256": sha_file(f"/proc/{pid}/environ"),
            "exe_device": info.st_dev, "exe_inode": info.st_ino, "exe_bytes": info.st_size}


def mount_content(path):
    path = Path(path)
    require(not any(p.is_symlink() for p in (path, *path.parents)), "U05_GATEWAY_SYMLINK_FORBIDDEN")
    if path.is_file():
        return {"source": str(path), "kind": "file", "sha256": sha_file(path), "bytes": path.stat().st_size}
    require(path.is_dir(), "U05_GATEWAY_MOUNT_TYPE_INVALID")
    material = []
    for item in sorted(path.rglob("*")):
        require(not item.is_symlink(), "U05_GATEWAY_SYMLINK_FORBIDDEN")
        relative = item.relative_to(path).as_posix()
        if item.is_dir():
            material.append({"path": relative, "kind": "directory"})
        else:
            require(item.is_file(), "U05_GATEWAY_SPECIAL_FILE_FORBIDDEN")
            material.append({"path": relative, "kind": "file", "bytes": item.stat().st_size, "sha256": sha_file(item)})
    return {"source": str(path), "kind": "directory", "sha256": AUDIT["digest"](material), "items": len(material)}


def writers(running=True, services=None):
    result = []
    manifest = AUDIT["private_json"](AUDIT["MATERIALS"] / "runtime-manifest.private.json")
    for service, name in WRITERS.items():
        if services is not None and service not in services:
            continue
        c = AUDIT["parsed"](docker(["inspect", name]))[0]
        label = c["Config"].get("Labels") or {}
        require(c["Name"] == "/" + name and re.fullmatch(r"[0-9a-f]{64}", c["Id"])
                and label.get("saas.uuid-stage") == "U05" and label.get("saas.uuid-runtime") == "20261024"
                and label.get("saas.uuid-service") == service and c["HostConfig"]["NetworkMode"] == "host"
                and c["HostConfig"]["ReadonlyRootfs"] and "ALL" in (c["HostConfig"].get("CapDrop") or [])
                and c["HostConfig"]["RestartPolicy"]["Name"] == "no" and not c["HostConfig"]["AutoRemove"]
                and c["State"]["Running"] is running and not c["State"]["Paused"] and not c["State"]["Restarting"]
                and c["Image"] == (GATEWAY_IMAGE if service == "gateway" else RUNTIME_IMAGE), "U05_WRITER_OWNERSHIP_OR_STATE_INVALID")
        mounts = sorted(({k: v[k] for k in ("Type", "Source", "Destination", "RW")} for v in c["Mounts"]), key=lambda x: x["Destination"])
        record = {"service": service, "name": name, "id": c["Id"], "image": c["Image"], "mounts": mounts,
                  "running": running, "pid": c["State"]["Pid"], "processes": [],
                  "container_started_at": c["State"]["StartedAt"], "restart_count": c["RestartCount"],
                  "config_sha256": AUDIT["digest"](c["Config"]), "host_config_sha256": AUDIT["digest"](c["HostConfig"])}
        if service != "gateway":
            required = {"/run/u05/service": (str(RUNTIME / "bin" / service), False),
                        "/run/u05/service.env": (str(RUNTIME / "env" / (service + ".env")), False),
                        "/run/u05/runner.py": (str(RUNTIME / "runner.py"), False),
                        "/run/u05/empty.env": (str(RUNTIME / "empty.env"), False),
                        "/run/u05/state": (str(RUNTIME / "state" / service), True)}
            binds = {x["Destination"]: x for x in mounts if x["Type"] == "bind"}
            require(len(mounts) == len(required) and set(binds) == set(required)
                    and all((binds[k]["Source"], binds[k]["RW"]) == v for k, v in required.items()), "U05_WRITER_MOUNTS_CHANGED")
            record["binary_sha256"] = RUNTIME_BINARIES[service]
            require(manifest["services"][service]["binary"]["sha256"] == record["binary_sha256"]
                    and sha_file(RUNTIME / "bin" / service) == record["binary_sha256"], "U05_RUNTIME_BINARY_CHANGED")
            record["environment_file_sha256"] = sha_file(RUNTIME / "env" / (service + ".env"))
            record["runner_sha256"] = sha_file(RUNTIME / "runner.py")
            record["empty_environment_sha256"] = sha_file(RUNTIME / "empty.env")
        else:
            required = {"/srv/frontends": str(RUNTIME / "static"),
                        "/srv/frontends/platform": "/var/lib/saas-uuid-u05-materials-20261024/host-u05-r2/static",
                        "/etc/nginx/nginx.conf": str(RUNTIME / "nginx.conf")}
            require(len(mounts) == 3 and {x["Destination"] for x in mounts} == set(required)
                    and all(x["Type"] == "bind" and not x["RW"] and x["Source"] == required[x["Destination"]] for x in mounts), "U05_GATEWAY_MOUNTS_INVALID")
            record["mount_file_hashes"] = [mount_content(x["Source"]) for x in mounts]
        if running:
            pending = [record["pid"]]
            for pid in pending:
                record["processes"].append(process_proof(pid))
                pending.extend(int(n) for n in Path(f"/proc/{pid}/task/{pid}/children").read_text().split())
                require(len(pending) <= 8, "U05_WRITER_PROCESS_TREE_UNEXPECTED")
            if service != "gateway":
                require(sum(p["exe_sha256"] == record["binary_sha256"] for p in record["processes"]) == 1, "U05_ACTUAL_EXECUTABLE_MISMATCH")
        else:
            require(record["pid"] == 0, "STOPPED_WRITER_HAS_PID")
        result.append(record)
    return result


def guard(receipt, lock):
    lock.alive()
    require(sha_file(SCRIPT) == receipt["script_sha256"]
            and SEMANTIC_HELPER_SHA256 == receipt["semantic_helper_sha256"], "CONTROL_CODE_CHANGED_DURING_PHASE")
    require(cluster() == receipt["cluster"], "POSTMASTER_OR_CLUSTER_GENERATION_CHANGED")
    current = source_generation()
    require([(s["oid"], s["owner_oid"], s["schema_sha256"], s["history"]) for s in current["sources"]]
            == [(s["oid"], s["owner_oid"], s["schema_sha256"], s["history"]) for s in receipt["source_generation"]["sources"]], "SOURCE_GENERATION_CHANGED")
    require(current["runtime_roles"] == receipt["source_generation"]["runtime_roles"], "SOURCE_RUNTIME_ROLE_GENERATION_CHANGED")


def finish(phase, stage, base, targets, **extra):
    require(sha_file(SCRIPT) == phase.script_sha256, "CONTROL_CODE_CHANGED_BEFORE_RECEIPT")
    validate_targets(stage, targets)
    if stage in ("MIGRATED", "SEALED", "FROZEN_U05_V7_SOURCES"):
        for kind in KINDS:
            source = next(s for s in base["source_generation"]["sources"] if s["kind"] == kind)
            require(all(t["semantic_schema_sha256"] == source["semantic_schema_sha256"]
                        for t in targets if t["kind"] == kind), "SOURCE_FINAL_RECOVERY_SEMANTIC_SCHEMA_DIFFERENCE")
    receipt = {"format": FORMAT, "stage": stage, "recorded_at_utc": now(), "started_at_utc": phase.started,
               "script_sha256": sha_file(SCRIPT), "previous_receipt_sha256": phase.previous_sha,
               "semantic_helper_sha256": SEMANTIC_HELPER_SHA256,
               "target_definitions": TARGETS, **base, "targets": targets, **extra}
    output = phase.directory / "receipt.json"
    digest = write_new(output, receipt)
    phase.event("receipt_published", stage=stage, receipt_sha256=digest)
    print(json.dumps({"status": stage, "receipt": str(output), "receipt_sha256": digest,
                      "source_databases_never_dropped_or_truncated": True}, separators=(",", ":")))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    for option in ("check", "prepare", "create", "migrate", "seal", "freeze-u05"):
        group.add_argument("--" + option, action="store_true")
    parser.add_argument("--receipt", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    require(sys.platform == "linux" and os.geteuid() == 0, "WSL_ROOT_REQUIRED")
    require(len({t["name"] for t in TARGETS} | {t["owner"] for t in TARGETS}) == 12
            and not ({t["name"] for t in TARGETS} & {v[0] for v in AUDIT["TARGETS"].values()}), "TARGET_SOURCE_COLLISION")
    frozen_binaries = binaries()
    if args.check:
        print(json.dumps({"status": "STATIC_READY_NO_DATABASE_CONNECTION", "target_databases": 6,
                          "migrators": frozen_binaries, "migration_counts_per_set": [12, 3, 4]}, separators=(",", ":")))
        return
    if not BASE.exists():
        BASE.mkdir(mode=0o700)
        fsync_dir(BASE.parent)
    private(BASE, True)
    require(args.output is not None and args.output.is_absolute(), "NEW_OUTPUT_DIRECTORY_REQUIRED")
    action = next(k for k in ("prepare", "create", "migrate", "seal", "freeze_u05") if getattr(args, k))
    previous, previous_sha = (None, None) if action == "prepare" else read_receipt(args.receipt)
    expected_stage = {"create": "PREPARED", "migrate": "CREATED", "seal": "MIGRATED", "freeze_u05": "SEALED"}
    if action != "prepare":
        require(previous["stage"] == expected_stage[action], "WRONG_PHASE_RECEIPT")
    phase = Phase(args.output, action, previous, previous_sha)
    lock = None
    try:
        lock = Advisory(phase)
        if action == "prepare":
            require(new_names() == {"databases": [], "roles": []}, "UNKNOWN_EXISTING_TARGET_OR_OWNER")
            base = {"cluster": cluster(), "source_generation": source_generation(phase=phase), "migrators": frozen_binaries, "writers": writers()}
            # Parse and execute the exact SELECT-only DO guards before downtime.
            sql("\n".join(role_guard(r) for r in base["source_generation"]["runtime_roles"]), phase=phase)
            phase.event("actual_runtime_role_guards_validated_read_only", roles=4)
            finish(phase, "PREPARED", base, [])
            return
        guard(previous, lock)
        require(previous["migrators"] == frozen_binaries, "MIGRATOR_METADATA_CHANGED")
        base = {key: previous[key] for key in ("cluster", "source_generation", "migrators", "writers")}
        targets = []
        if action == "create":
            require(new_names() == {"databases": [], "roles": []}, "UNKNOWN_EXISTING_TARGET_OR_OWNER")
            for target in TARGETS:
                guard(previous, lock)
                name, owner = ident(target["name"]), ident(target["owner"])
                phase.event("mutation_intent", operation_kind="CREATE_FIXED_ROLE_AND_DATABASE", target=target["name"])
                sql(f"CREATE ROLE {owner} NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;", readonly=False, phase=phase)
                sql(f"CREATE DATABASE {name} OWNER {owner} TEMPLATE template0 ENCODING 'UTF8' ALLOW_CONNECTIONS false;", readonly=False, phase=phase)
                sql(f"REVOKE ALL ON DATABASE {name} FROM PUBLIC;\nALTER DATABASE {name} ALLOW_CONNECTIONS true;", readonly=False, phase=phase)
                sql(f"ALTER SCHEMA public OWNER TO {owner};\nREVOKE ALL ON SCHEMA public FROM PUBLIC;", target["name"], readonly=False, phase=phase)
                targets.append(target_catalog(target))
                phase.event("created_and_verified", target=target["name"], database_oid=targets[-1]["oid"], owner_oid=targets[-1]["owner_oid"])
            finish(phase, "CREATED", base, targets)
        elif action in ("migrate", "seal"):
            for target, known in zip(TARGETS, previous["targets"]):
                guard(previous, lock)
                current = target_catalog(target, migrated=action == "seal", sealed=False)
                require(current["oid"] == known["oid"] and current["owner_oid"] == known["owner_oid"], "NEW_TARGET_GENERATION_CHANGED")
                if action == "migrate":
                    migrate_target(target, known, phase)
                else:
                    require(current["raw_schema_sha256"] == known["raw_schema_sha256"] and current["history"] == known["history"], "NEW_TARGET_SCHEMA_CHANGED_BEFORE_SEAL")
                    phase.event("seal_intent", target=target["name"])
                    sql(f"ALTER DATABASE {ident(target['name'])} SET default_transaction_read_only=on;", readonly=False, phase=phase)
                targets.append(target_catalog(target, migrated=True, sealed=action == "seal", phase=phase))
                source = next(s for s in base["source_generation"]["sources"] if s["kind"] == target["kind"])
                require(targets[-1]["semantic_schema_sha256"] == source["semantic_schema_sha256"], "NEW_TARGET_SEMANTIC_SCHEMA_DIFFERENCE")
                phase.event("target_verified", target=target["name"], raw_schema_sha256=targets[-1]["raw_schema_sha256"])
            finish(phase, "MIGRATED" if action == "migrate" else "SEALED", base, targets)
        else:
            # This action is intentionally separate. Precreation never stops U05.
            observed = writers()
            require(observed == previous["writers"], "U05_WRITER_GENERATION_CHANGED")
            write_new(phase.directory / "writers-before.json", observed)
            fresh_before = source_generation()
            require(fresh_before["runtime_roles"] == previous["source_generation"]["runtime_roles"], "RUNTIME_ROLE_GENERATION_CHANGED_BEFORE_STOP")
            write_new(phase.directory / "source-before.json", fresh_before)
            downtime_started, downtime_clock = now(), time.monotonic()
            phase.event("downtime_started", budget_seconds=600, operator_selected_window=True, started_at_utc=downtime_started)
            by_service = {w["service"]: w for w in observed}
            for service in STOP_ORDER:
                lock.alive()
                require(cluster() == previous["cluster"], "POSTMASTER_GENERATION_CHANGED_DURING_FREEZE")
                writer = by_service[service]
                require(writers(services=[service]) == [writer], "WRITER_GENERATION_CHANGED_DURING_STOP")
                phase.event("stop_intent", service=service, container_id=writer["id"])
                docker(["stop", "--time", "30", writer["id"]])
                phase.event("stopped", service=service, container_id=writer["id"])
            roles = fresh_before["runtime_roles"]
            require([(r["name"],r["oid"],r["settings"]) for r in roles]
                    == [(r["name"],r["oid"],r["settings"]) for r in previous["source_generation"]["runtime_roles"]], "RUNTIME_ROLE_GENERATION_CHANGED")
            statements = ["BEGIN READ WRITE;"]
            for role in roles:
                statements.append(role_guard(role))
                statements.append(f"ALTER ROLE {ident(role['name'])} NOLOGIN;")
            statements.append("COMMIT;")
            phase.event("runtime_nologin_intent", roles=4)
            sql("\n".join(statements), readonly=False, phase=phase)
            source_oids = ",".join(str(s["oid"]) for s in fresh_before["sources"])
            phase.event("terminate_fixed_u05_backends_intent")
            sql(f"SELECT jsonb_build_object('terminated',COALESCE(bool_and(pg_terminate_backend(pid)),true)) FROM pg_stat_activity WHERE datid IN ({source_oids}) AND pid<>pg_backend_pid();", readonly=False, phase=phase)
            stopped = writers(False)
            require([{k: v for k, v in w.items() if k not in ("running", "pid", "processes")} for w in stopped]
                    == [{k: v for k, v in w.items() if k not in ("running", "pid", "processes")} for w in observed], "STOPPED_WRITER_GENERATION_CHANGED")
            after = source_generation(True)
            require([(r["name"],r["oid"],r["settings"]) for r in after["runtime_roles"]]
                    == [(r["name"],r["oid"],r["settings"]) for r in roles], "ROLE_WRITE_OVERRIDES_CHANGED")
            write_new(phase.directory / "writers-after.json", stopped)
            write_new(phase.directory / "source-after.json", after)
            finish(phase, "FROZEN_U05_V7_SOURCES", base, previous["targets"], frozen_source_generation=after,
                   stopped_writers=stopped, downtime_started_at_utc=downtime_started,
                   freeze_elapsed_seconds=time.monotonic() - downtime_clock,
                   freeze_effects="API/gateway stopped; source LOGIN/CONNECT/SET/admin closure passed; sources idle/default-read-only; original v4 untouched")
    except Exception as error:
        phase.event("failed_quarantined", category=str(error) if isinstance(error, Invalid) else "UNEXPECTED_CONTROL_FAILURE")
        write_new(phase.directory / "failure.json", {"format": FORMAT, "status": "FAILED_RETAIN_QUARANTINE", "action": action,
                  "recorded_at_utc": now(), "category": str(error) if isinstance(error, Invalid) else "UNEXPECTED_CONTROL_FAILURE",
                  "automatic_source_restart": False, "automatic_database_drop": False})
        raise
    finally:
        if lock:
            lock.close()
        phase.close()


if __name__ == "__main__":
    try:
        main()
    except Invalid as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
    except Exception:
        print("U06_CONTROL_FAILED_RETAIN_PRIVATE_EVIDENCE", file=sys.stderr)
        sys.exit(1)
