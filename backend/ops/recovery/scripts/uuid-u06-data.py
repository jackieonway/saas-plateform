#!/usr/bin/env python3
"""Closed local U06 typed snapshot/delta engine; no services or admission changes.

Rows are opaque PostgreSQL row_to_json TEXT inside an outer JSON envelope. Python
never decodes a business row, so numeric lexemes and duplicate JSON keys survive.
Only authenticated packages are consumed; every write is followed by a full
independent PostgreSQL reread before commit. Cross-database commits are separate:
any error leaves the whole deployment quarantined for the operator.
"""
import argparse
from collections import deque
import hashlib
import json
import os
from pathlib import Path
import re
import select
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import uuid

FORMAT = "saas-uuid-u06-data/1"
REG_FORMAT = "saas-uuid-u06-registration/1"
KINDS = ("platform", "stores", "work_orders")
BACKEND = Path(__file__).resolve().parents[3]
WORKSPACE = BACKEND.parents[1]
SCHEMA_SQL = {
    "platform": BACKEND / "src/database/schema_material.sql",
    "stores": WORKSPACE / "saas-stores/backend/src/imported_schema.sql",
    "work_orders": WORKSPACE / "saas-work-orders/backend/src/imported_schema.sql",
}
SCHEMAS = {
    "platform": "a11665c8c4a4282bf406a54cb61b82105fc9ea2c077b1ff717c9947eed47e3af",
    "stores": "2d989b86b44b7f597aef31bfadf5b3eae3b697364e2fd5030cb32fc676cda550",
    "work_orders": "b8a502cb0e3bcfdd7ed4c00fdfbbf21af1f527feddb5b61bebc4f13859732a3e",
}
NAMES = {
    "source": ("qiankun_platform_uuid7_u04_20261024", "qiankun_stores_uuid7_u04_20261024", "qiankun_orders_uuid7_u04_20261024"),
    "final": ("qiankun_platform_uuid7_final_20261024", "qiankun_stores_uuid7_final_20261024", "qiankun_orders_uuid7_final_20261024"),
    "recovery": ("qiankun_platform_uuid7_recovery_20261024", "qiankun_stores_uuid7_recovery_20261024", "qiankun_orders_uuid7_recovery_20261024"),
}
OWNERS = {
    "source": ("qiankun_platform_uuid7_u04_owner_20261024", "qiankun_stores_uuid7_u04_owner_20261024", "qiankun_orders_uuid7_u04_owner_20261024"),
    "final": ("qiankun_platform_uuid7_final_owner_20261024", "qiankun_stores_uuid7_final_owner_20261024", "qiankun_orders_uuid7_final_owner_20261024"),
    "recovery": ("qiankun_platform_uuid7_recovery_owner_20261024", "qiankun_stores_uuid7_recovery_owner_20261024", "qiankun_orders_uuid7_recovery_owner_20261024"),
}
RUNTIMES = {
    "source": tuple("qiankun_" + service + "_uuid7_u05_runtime_20261024" for service in ("platform", "stores", "orders", "notification")),
    "final": tuple("qiankun_" + service + "_uuid7_u06_final_runtime_20261024" for service in ("platform", "stores", "orders", "notification")),
    "recovery": tuple("qiankun_" + service + "_uuid7_u06_recovery_runtime_20261024" for service in ("platform", "stores", "orders", "notification")),
}
SCHEMA_QUERY_SHA256 = "204b5d744b099f2eda02fa363f5c54d3379da75753ae4673474017e6fb51ade2"
TABLES = {
    "platform": "_sqlx_migrations application_deployments application_release_artifacts application_release_operations application_releases application_url_entries applications audit_logs departments managed_deployment_bindings managed_deployment_jobs managed_image_artifacts membership_data_grants memberships menus notification_prefs notification_publications notifications oauth_authorization_codes oauth_authorization_requests oauth_clients oauth_device_codes oauth_tokens organization_applications organization_menus organizations platform_session_refresh_tokens platform_sessions platform_settings region_versions regions role_menus roles tenant_applications tenant_menus tenants user_roles users zip_distribution_jobs zip_distribution_nodes".split(),
    "stores": "_sqlx_migrations _stores_schema_contract _stores_schema_contract_v2 _stores_schema_contract_v3 store_audit_events store_daily_reports store_staff stores".split(),
    "work_orders": "_sqlx_migrations work_order_counters work_order_create_idempotency work_order_migrations work_order_reminder_intents work_orders".split(),
}
CREDENTIALS = {"platform": set("oauth_authorization_codes oauth_authorization_requests oauth_device_codes oauth_tokens platform_session_refresh_tokens platform_sessions".split()), "stores": set(), "work_orders": set()}
METADATA = {"platform": {"_sqlx_migrations"}, "stores": set("_sqlx_migrations _stores_schema_contract _stores_schema_contract_v2 _stores_schema_contract_v3".split()), "work_orders": {"_sqlx_migrations", "work_order_migrations"}}
APPEND_ONLY = {"platform": set("audit_logs application_release_artifacts notification_publications".split()), "stores": {"store_audit_events"}, "work_orders": {"work_order_create_idempotency", "work_order_reminder_intents"}}
MIGRATIONS = {"platform": 12, "stores": 3, "work_orders": 4}
CODEC_BINARY = "/usr/local/libexec/saas-recovery-u06-codec"
HEX = re.compile(r"[0-9a-f]{64}\Z")


class Invalid(Exception):
    """Only static error categories may be printed; DB errors remain private."""


def require(value, code):
    if not value:
        raise Invalid(code)


def unique(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "DUPLICATE_ENVELOPE_KEY")
        result[key] = value
    return result


def parsed(raw):
    try:
        return json.loads(raw, object_pairs_hook=unique, parse_constant=lambda _: (_ for _ in ()).throw(Invalid("NONFINITE_ENVELOPE_NUMBER")))
    except (ValueError, UnicodeError) as error:
        raise Invalid("INVALID_ENVELOPE_JSON") from error


def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def value_sha(value):
    return hashlib.sha256(canonical(value)).hexdigest()


def checked_path(raw):
    require(isinstance(raw, str) and raw.startswith("/") and "\\" not in raw and "\0" not in raw, "ABSOLUTE_LINUX_PATH_REQUIRED")
    require(all(p not in ("", ".", "..") for p in raw.split("/")[1:]), "NORMALIZED_PATH_REQUIRED")
    path = Path(raw)
    require(not any(p.is_symlink() for p in (path, *path.parents)), "SYMLINK_FORBIDDEN")
    return path


def private_file(raw, limit=None):
    path = checked_path(str(raw))
    item = path.stat()
    require(stat.S_ISREG(item.st_mode) and item.st_uid == os.geteuid() and stat.S_IMODE(item.st_mode) == 0o600 and item.st_nlink == 1, "PRIVATE_FILE_MODE_INVALID")
    require(limit is None or item.st_size <= limit, "PRIVATE_FILE_TOO_LARGE")
    return path


def private_dir(raw, tmpfs=False):
    path = checked_path(str(raw))
    item = path.stat()
    require(stat.S_ISDIR(item.st_mode) and item.st_uid == os.geteuid() and stat.S_IMODE(item.st_mode) == 0o700, "PRIVATE_DIRECTORY_INVALID")
    if tmpfs:
        result = subprocess.run(["findmnt", "-n", "-o", "FSTYPE", "--target", str(path)], capture_output=True, text=True, check=False)
        require(result.returncode == 0 and result.stdout.strip() == "tmpfs", "TMPFS_REQUIRED")
    return path


def new_file(raw):
    path = checked_path(str(raw))
    private_dir(str(path.parent))
    require(not path.exists(), "OUTPUT_ALREADY_EXISTS")
    return path


def write_json(path, value):
    with Path(path).open("xb") as stream:
        stream.write(canonical(value) + b"\n")
        stream.flush()
        os.fsync(stream.fileno())


def literal(value):
    require(isinstance(value, str) and "\0" not in value, "INVALID_SQL_LITERAL")
    return "'" + value.replace("'", "''") + "'"


def ident(value):
    require(isinstance(value, str) and bool(re.fullmatch(r"[a-zA-Z_][a-zA-Z_0-9]*", value)), "INVALID_SQL_IDENTIFIER")
    return '"' + value + '"'


def relation(table):
    return 'public.' + ident(table)


def psql_argv(database):
    return ["/usr/sbin/runuser", "-u", "postgres", "--", "/usr/bin/psql", "-X", "-qAt", "--no-password", "-v", "ON_ERROR_STOP=1", "-h", "/run/postgresql", "-p", "5432", "-U", "postgres", "-d", database, "--file=-"]


class Session:
    diagnostic_dir = None
    diagnostic_files = []

    def __init__(self, database, scratch, write=False, isolation="REPEATABLE READ"):
        self.database = database
        self.error = tempfile.TemporaryFile(dir=scratch)
        env = {k: v for k, v in os.environ.items() if not k.startswith("PG")}
        env.update({"LC_ALL": "C.UTF-8", "PGPASSFILE": "/dev/null", "PGCONNECT_TIMEOUT": "10", "PGOPTIONS": "-c default_transaction_read_only=on"})
        self.proc = subprocess.Popen(psql_argv(database), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.error, env=env, bufsize=0)
        os.set_blocking(self.proc.stdin.fileno(), False)
        self.buffer = b""
        self.lines = deque()
        self.deadline = time.monotonic() + 1800
        require(isolation in ("REPEATABLE READ", "READ COMMITTED"), "SESSION_ISOLATION_INVALID")
        try:
            self.send("BEGIN ISOLATION LEVEL " + isolation + " " + ("READ WRITE" if write else "READ ONLY") + ";\nSET LOCAL search_path=pg_catalog;\nSET LOCAL timezone='UTC';\nSET LOCAL datestyle='ISO, YMD';\nSET LOCAL intervalstyle='postgres';\nSET LOCAL extra_float_digits=3;\nSET LOCAL bytea_output='hex';\nSET LOCAL standard_conforming_strings=on;\nSET LOCAL row_security=off;\nSET LOCAL statement_timeout='180s';\nSET LOCAL lock_timeout='3s';\nSET LOCAL session_replication_role='origin';")
            self.query("SELECT 'ready';")
        except Exception:
            self.abort()
            raise

    def send(self, sql):
        try:
            pending = memoryview((sql + "\n").encode())
            while pending:
                remaining = self.deadline - time.monotonic()
                require(remaining > 0 and select.select([], [self.proc.stdin], [], min(remaining, 190))[1], "DATABASE_WRITE_IO_TIMEOUT")
                try:
                    written = os.write(self.proc.stdin.fileno(), pending[:65536])
                except BlockingIOError:
                    continue
                require(written > 0, "DATABASE_STDIN_FAILED")
                pending = pending[written:]
        except (BrokenPipeError, OSError) as error:
            raise Invalid("DATABASE_SESSION_FAILED") from error

    def rows(self, sql):
        marker = "u06_boundary_" + uuid.uuid4().hex
        self.send(sql + "\n\\echo " + marker)
        while True:
            require(time.monotonic() < self.deadline, "DATABASE_SESSION_TIMEOUT")
            while not self.lines:
                remaining = self.deadline - time.monotonic()
                require(remaining > 0 and select.select([self.proc.stdout], [], [], min(remaining, 190))[0], "DATABASE_IO_TIMEOUT")
                chunk = os.read(self.proc.stdout.fileno(), 1024 * 1024)
                require(bool(chunk), "DATABASE_QUERY_FAILED")
                pieces = (self.buffer + chunk).split(b"\n")
                self.buffer = pieces.pop()
                self.lines.extend(pieces)
                require(len(self.buffer) <= 64 * 1024 * 1024 and all(len(line) <= 64 * 1024 * 1024 for line in pieces), "DATABASE_ROW_TOO_LARGE")
            line = self.lines.popleft().decode()
            if line == marker:
                break
            yield line

    def query(self, sql):
        return list(self.rows(sql))

    def one(self, sql):
        lines = self.query(sql)
        require(len(lines) == 1, "QUERY_FRAMING_INVALID")
        return parsed(lines[0])

    def copy(self, table, rows):
        self.send("COPY " + table + " FROM STDIN;")
        for row in rows:
            self.send(row.replace("\\", "\\\\").replace("\t", "\\t").replace("\n", "\\n").replace("\r", "\\r"))
        self.send("\\.")
        self.query("SELECT 'copied';")

    def close(self, commit=False):
        if self.proc.poll() is None:
            self.query("COMMIT;" if commit else "ROLLBACK;")
            self.proc.stdin.close()
            require(self.proc.wait(timeout=10) == 0, "DATABASE_SESSION_CLOSE_FAILED")
        self.error.close()

    def abort(self):
        if self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait()
        if not self.error.closed:
            self.error.flush()
            if self.error.tell() and self.diagnostic_dir is not None:
                directory = checked_path(str(self.diagnostic_dir))
                if not directory.exists():
                    directory.mkdir(mode=0o700)
                private_dir(str(directory))
                target = directory / (self.database + "-" + uuid.uuid4().hex + ".stderr")
                self.error.seek(0)
                with target.open("xb") as stream:
                    shutil.copyfileobj(self.error, stream)
                    stream.flush()
                    os.fsync(stream.fileno())
                self.diagnostic_files.append({"path": str(target), "bytes": target.stat().st_size, "sha256": sha(target)})
            self.error.close()


CATALOG = """SELECT COALESCE(jsonb_agg(jsonb_build_object(
 'name',c.relname,'columns',(SELECT jsonb_agg(jsonb_build_object('name',a.attname,'generated',a.attgenerated,'identity',a.attidentity,'json_type',CASE WHEN a.atttypid='json'::regtype THEN 'json' WHEN a.atttypid='jsonb'::regtype THEN 'jsonb' ELSE '' END) ORDER BY a.attnum) FROM pg_attribute a WHERE a.attrelid=c.oid AND a.attnum>0 AND NOT a.attisdropped),
 'pk',COALESCE((SELECT jsonb_agg(a.attname ORDER BY k.n) FROM pg_index i CROSS JOIN LATERAL unnest(i.indkey) WITH ORDINALITY k(attnum,n) JOIN pg_attribute a ON a.attrelid=c.oid AND a.attnum=k.attnum WHERE i.indrelid=c.oid AND i.indisprimary),'[]'::jsonb),
 'dependencies',COALESCE((SELECT jsonb_agg(DISTINCT rc.relname ORDER BY rc.relname) FROM pg_constraint f JOIN pg_class rc ON rc.oid=f.confrelid JOIN pg_namespace rn ON rn.oid=rc.relnamespace WHERE f.conrelid=c.oid AND f.contype='f' AND f.confrelid<>f.conrelid AND rn.nspname='public'),'[]'::jsonb),
 'foreign_keys_valid',(SELECT COALESCE(bool_and(f.convalidated),true) FROM pg_constraint f WHERE f.conrelid=c.oid AND f.contype='f'),
 'triggers_enabled',(SELECT COALESCE(bool_and(t.tgenabled IN ('O','A')),true) FROM pg_trigger t WHERE t.tgrelid=c.oid)
 ) ORDER BY c.relname),'[]'::jsonb) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind IN ('r','p','f');"""


def migration_rows(session):
    return session.one("SELECT COALESCE(jsonb_agg(jsonb_build_object('version',version,'description',description,'checksum',encode(checksum,'hex'),'success',success,'installed_on_present',installed_on IS NOT NULL,'execution_time_valid',execution_time>=-1) ORDER BY version),'[]'::jsonb) FROM public._sqlx_migrations;")


def semantic_material(material):
    normalized = []
    for original in material:
        item = dict(original)
        if item.get("kind") == "trigger" and item.get("internal"):
            name = item.get("name", "")
            match = re.fullmatch(r"RI_ConstraintTrigger_([ac])_[0-9]+", name)
            require(match is not None and item["definition"].count(name) == 1, "UNKNOWN_INTERNAL_TRIGGER_CONTRACT")
            replacement = "RI_ConstraintTrigger_" + match[1] + "_[OID]"
            item["name"] = replacement
            item["definition"] = item["definition"].replace(name, replacement, 1)
        normalized.append(item)
    return sorted(normalized, key=canonical)


def validate_metadata(session, kind, migrations, cfg):
    count = MIGRATIONS[kind]
    require([m["version"] for m in migrations] == list(range(1, count + 1)), "MIGRATION_VERSION_SET_MISMATCH")
    directories = {"platform": BACKEND / "migrations", "stores": WORKSPACE / "saas-stores/backend/migrations", "work_orders": WORKSPACE / "saas-work-orders/backend/migrations"}
    paths = []
    for row in migrations:
        candidates = list(directories[kind].glob(f"{row['version']:04}_*.sql"))
        require(len(candidates) == 1, "MIGRATION_SOURCE_AMBIGUOUS")
        path = candidates[0]
        paths.append(path)
        require(row["checksum"] == hashlib.sha384(path.read_bytes()).hexdigest() and row["description"] == path.stem[5:].replace("_", " ") and row["success"] and row["installed_on_present"] and row["execution_time_valid"], "CANONICAL_MIGRATION_METADATA_MISMATCH")
    legacy = []
    if kind == "work_orders":
        legacy = session.one("SELECT COALESCE(jsonb_agg(jsonb_build_object('version',version,'checksum',checksum,'applied_at_present',applied_at IS NOT NULL) ORDER BY version),'[]'::jsonb) FROM public.work_order_migrations;")
        require(len(legacy) == count, "LEGACY_MIGRATION_COUNT_MISMATCH")
        for row, path in zip(legacy, paths):
            require(row["version"] == path.name and row["checksum"] == hashlib.sha256(path.read_bytes()).hexdigest() and row["applied_at_present"], "LEGACY_MIGRATION_METADATA_MISMATCH")
    if kind == "stores":
        session.query("SET LOCAL search_path=public,pg_catalog;")
        valid = session.one("SELECT to_json(count(*)=1 AND COALESCE(bool_and(v3.fingerprint=public.stores_schema_fingerprint_v3() AND v3.legacy_fingerprint=v2.fingerprint AND v2.legacy_fingerprint=v1.fingerprint),false)) FROM public._stores_schema_contract_v3 v3 JOIN public._stores_schema_contract_v2 v2 USING(singleton) JOIN public._stores_schema_contract v1 USING(singleton) WHERE v3.singleton;")
        session.query("SET LOCAL search_path=pg_catalog;")
        require(valid is True, "STORES_SCHEMA_CONTRACT_INVALID")
    return legacy


def inspect(session, db, cfg):
    identity = session.one("SELECT jsonb_build_object('name',current_database(),'oid',(SELECT oid::bigint FROM pg_database WHERE datname=current_database()),'owner',(SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname=current_database()),'owner_oid',(SELECT datdba::bigint FROM pg_database WHERE datname=current_database()),'server_version_num',current_setting('server_version_num')::int,'system_identifier',(SELECT system_identifier::text FROM pg_control_system()),'user',current_user,'session_user',session_user,'local',inet_server_addr() IS NULL,'prepared',(SELECT count(*) FROM pg_prepared_xacts WHERE database=current_database()));")
    require(all(identity[k] == db[k] for k in ("name", "oid", "owner", "owner_oid")), "DATABASE_IDENTITY_MISMATCH")
    require(identity["user"] == identity["session_user"] == "postgres" and identity["local"] and identity["prepared"] == 0, "MAINTENANCE_IDENTITY_INVALID")
    require(identity["system_identifier"] == cfg["reg"]["system_identifier"] and identity["server_version_num"] >= 170000, "CLUSTER_IDENTITY_MISMATCH")
    tables = session.one(CATALOG)
    require(all(t["foreign_keys_valid"] and t["triggers_enabled"] for t in tables), "DISABLED_CONSTRAINT_OR_TRIGGER")
    names = [t["name"] for t in tables]
    kind = db["kind"]
    if cfg["fixture"]:
        contract = cfg["reg"]["fixture_contracts"][kind]
        require(names == contract["tables"], "TABLE_COVERAGE_MISMATCH")
        material = session.one(SCHEMA_SQL[kind].read_text())
        semantic = value_sha(semantic_material(material))
        require(semantic == contract["schema_sha256"], "FIXTURE_SCHEMA_SEMANTIC_MISMATCH")
        expected = value_sha(material)
    else:
        require(names == TABLES[kind], "TABLE_COVERAGE_MISMATCH")
        material = session.one(SCHEMA_SQL[kind].read_text())
        expected = db["expected_schema_sha256"]
        semantic = value_sha(semantic_material(material))
        require(semantic == db["semantic_schema_sha256"], "SCHEMA_SEMANTIC_PIN_MISMATCH")
    require(value_sha(material) == expected, "RAW_SCHEMA_MISMATCH")
    migrations = migration_rows(session)
    expected_count = cfg["reg"]["fixture_contracts"][kind]["migration_count"] if cfg["fixture"] else MIGRATIONS[kind]
    require(len(migrations) == expected_count and all(m["success"] for m in migrations), "MIGRATION_HISTORY_MISMATCH")
    require(len({m["version"] for m in migrations}) == len(migrations), "MIGRATION_HISTORY_DUPLICATE")
    legacy = [] if cfg["fixture"] else validate_metadata(session, kind, migrations, cfg)
    return {"identity": identity, "tables": tables, "schema_sha256": expected, "semantic_schema_sha256": semantic, "migrations": migrations, "legacy_migrations": legacy}


def policies(cfg, kind):
    if cfg["fixture"]:
        spec = cfg["reg"]["fixture_contracts"][kind]
        return set(spec["credentials"]), set(spec["metadata"]), set(spec["append_only"])
    return CREDENTIALS[kind], METADATA[kind], APPEND_ONLY[kind]


def admission(cfg, group):
    databases = cfg["reg"]["groups"][group]
    names = ",".join(literal(d["name"]) for d in databases)
    protected = ",".join(literal(r) for r in sorted(set(cfg["reg"]["fenced_roles"][group] + [d["owner"] for d in databases])))
    session = Session("postgres", cfg["work"], isolation="READ COMMITTED")
    try:
        proof = session.one("""SELECT jsonb_build_object(
 'system_identifier',(SELECT system_identifier::text FROM pg_control_system()),
 'maintenance_database',current_database(),'user',current_user,'session_user',session_user,'local',inet_server_addr() IS NULL,
 'isolation',current_setting('transaction_isolation'),'transaction_read_only',current_setting('transaction_read_only')::boolean,'default_read_only',current_setting('default_transaction_read_only')::boolean,
 'databases',(SELECT jsonb_agg(jsonb_build_object('name',d.datname,'oid',d.oid::bigint,'owner',pg_get_userbyid(d.datdba),'owner_oid',d.datdba::bigint,'readonly',EXISTS(SELECT 1 FROM pg_db_role_setting s WHERE s.setdatabase=d.oid AND s.setrole=0 AND s.setconfig @> ARRAY['default_transaction_read_only=on']),'backends',(SELECT count(*) FROM pg_stat_activity a WHERE a.datid=d.oid),'prepared',(SELECT count(*) FROM pg_prepared_xacts p WHERE p.database=d.datname)) ORDER BY d.datname) FROM pg_database d WHERE d.datname IN (""" + names + ")),'roles',(SELECT jsonb_agg(jsonb_build_object('name',r.rolname,'oid',r.oid::bigint,'login',r.rolcanlogin,'superuser',r.rolsuper,'createrole',r.rolcreaterole,'privileged_set',EXISTS(SELECT 1 FROM pg_roles p WHERE (p.rolsuper OR p.rolcreaterole OR p.rolname IN (" + protected + ")) AND pg_has_role(r.oid,p.oid,'SET')),'can_admin_source_roles',EXISTS(SELECT 1 FROM pg_auth_members e JOIN pg_roles p ON p.oid=e.roleid WHERE p.rolname IN (" + protected + ") AND e.admin_option AND pg_has_role(r.oid,e.member,'SET')),'connect',EXISTS(SELECT 1 FROM pg_database d WHERE d.datname IN (" + names + ") AND has_database_privilege(r.oid,d.oid,'CONNECT'))) ORDER BY r.rolname) FROM pg_roles r));")
        require(proof["system_identifier"] == cfg["reg"]["system_identifier"], "ADMISSION_CLUSTER_MISMATCH")
        require(proof["maintenance_database"] == "postgres" and proof["user"] == proof["session_user"] == "postgres" and proof["local"] and proof["isolation"] == "read committed" and proof["transaction_read_only"] and proof["default_read_only"], "FRESH_MAINTENANCE_CONTEXT_INVALID")
        actual = proof["databases"] or []
        require(len(actual) == len(databases), "ADMISSION_DATABASE_MISSING")
        expected = {d["name"]: d for d in databases}
        for db in actual:
            require(all(db[k] == expected[db["name"]][k] for k in ("name", "oid", "owner", "owner_oid")), "ADMISSION_IDENTITY_MISMATCH")
            require(db["readonly"] and db["backends"] == db["prepared"] == 0, "DATABASE_NOT_QUIESCENT")
        roles = {r["name"]: r for r in proof["roles"]}
        for role in cfg["reg"]["fenced_roles"][group] + [d["owner"] for d in databases]:
            if role not in roles:
                # Epoch preparation creates these exact future roles itself.
                # Their names are already in SET/admin predicates above and
                # the frozen registration; absence cannot protect a new name.
                require(not cfg["fixture"] and group in ("final", "recovery") and role in RUNTIMES[group], "REGISTERED_PROTECTED_ROLE_MISSING")
                continue
            require(not roles[role]["login"] and not roles[role]["superuser"], "RUNTIME_ROLE_NOT_FENCED")
        require("postgres" in roles and roles["postgres"]["login"] and roles["postgres"]["superuser"], "MAINTENANCE_ROLE_MISSING")
        require(not any(r["name"] != "postgres" and r["login"] and (r["connect"] or r["superuser"] or r["createrole"] or r["privileged_set"] or r["can_admin_source_roles"]) for r in proof["roles"]), "OTHER_LOGIN_CAN_WRITE_OR_ESCALATE")
        session.close()
        return proof
    except Exception:
        session.abort()
        raise


def table_sql(table, predicate=None, lock=False):
    keys = table["pk"]
    require(bool(keys), "RETAINED_TABLE_WITHOUT_PRIMARY_KEY")
    key = "ARRAY[" + ",".join("t." + ident(c) + "::text" for c in keys) + "]"
    order = ",".join("t." + ident(c) for c in keys)
    json_columns = [c["name"] for c in table["columns"] if c["json_type"]]
    texts = "json_build_object(" + ",".join(literal(c) + ",t." + ident(c) + "::text" for c in json_columns) + ")" if json_columns else "'{}'::json"
    return "SELECT json_build_object('key'," + key + ",'row_text',row_to_json(t)::text,'json_texts'," + texts + ") FROM ONLY " + relation(table["name"]) + " t" + (" WHERE " + predicate if predicate else "") + " ORDER BY " + order + (" FOR UPDATE" if lock else "") + ";"


def stream_table(session, table, path):
    count = 0
    with Path(path).open("xb") as stream:
        for line in session.rows(table_sql(table)):
            envelope = parsed(line)
            require(set(envelope) == {"key", "row_text", "json_texts"} and isinstance(envelope["key"], list) and all(isinstance(v, str) for v in envelope["key"]) and isinstance(envelope["row_text"], str) and isinstance(envelope["json_texts"], dict) and all(v is None or isinstance(v, str) for v in envelope["json_texts"].values()), "ROW_ENVELOPE_INVALID")
            stream.write(line.encode() + b"\n")
            count += 1
    return {"name": table["name"], "file": Path(path).name, "rows": count, "bytes": Path(path).stat().st_size, "sha256": sha(path)}


def sequences(session):
    names = session.one("SELECT COALESCE(jsonb_agg(c.relname ORDER BY c.relname),'[]'::jsonb) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind='S';")
    return [{"name": name, **session.one("SELECT jsonb_build_object('last_value',last_value::text,'is_called',is_called) FROM " + relation(name) + ";")} for name in names]


def excluded_dump(session, table, path):
    # Metadata/credential evidence may have no PK; their exact opaque rows are
    # still sealed. They are never candidates for restore/delta/replay.
    if table["pk"]:
        return stream_table(session, table, path)
    count = 0
    with Path(path).open("xb") as stream:
        for line in session.rows("SELECT json_build_object('key',ARRAY[]::text[],'row_text',row_to_json(t)::text) FROM ONLY " + relation(table["name"]) + " t ORDER BY row_to_json(t)::text COLLATE \"C\";"):
            stream.write(line.encode() + b"\n")
            count += 1
    return {"name": table["name"], "file": Path(path).name, "rows": count, "bytes": Path(path).stat().st_size, "sha256": sha(path)}


def codec(cfg, mode, inp, out, purpose, expected=None):
    batch = purpose.split("/")
    require(len(batch) == 4 and batch[:2] == ["saas-uuid-preview-freeze", "1"] and re.fullmatch(r"[a-zA-Z0-9_-]+", batch[2]) is not None and re.fullmatch(r"u06-[a-zA-Z0-9_-]+", batch[3]) is not None, "CODEC_PURPOSE_INVALID")
    # Existing evidence codec binds component 2 as batch. Its logical name is
    # component 3; no arbitrary command or shell syntax is configuration data.
    bridge_dir = Path(cfg["scratch"]) / batch[2]
    created = not bridge_dir.exists()
    if created:
        bridge_dir.mkdir(mode=0o700)
    private_dir(str(bridge_dir), tmpfs=True)
    bridge = bridge_dir / ("u06-data-" + uuid.uuid4().hex + ".tar")
    if mode == "seal":
        with Path(inp).open("rb") as source, bridge.open("xb") as target:
            shutil.copyfileobj(source, target)
    dto = {"profile": "uuid-local-preview" if mode == "seal" else "uuid-local-u06", "input": str(bridge if mode == "seal" else inp), "output": str(out if mode == "seal" else bridge), "key_file": cfg["codec"]["key_file"], "purpose": purpose}
    if mode == "open":
        dto["expected_sha256"] = expected
    config = Path(cfg["work"]) / ("codec-" + uuid.uuid4().hex + ".json")
    write_json(config, dto)
    try:
        result = subprocess.run([cfg["codec"]["binary"], "uuid-seal-evidence" if mode == "seal" else "uuid-open-evidence", str(config)], capture_output=True, timeout=180, check=False)
        require(result.returncode == 0, "AUTHENTICATED_CODEC_FAILED")
        if mode == "open":
            private_file(str(bridge))
            require(not Path(out).exists(), "CODEC_OUTPUT_ALREADY_EXISTS")
            bridge.rename(out)
        private_file(str(out))
    finally:
        if bridge.exists():
            bridge.unlink()
        if created and not any(bridge_dir.iterdir()):
            bridge_dir.rmdir()


def package_seal(cfg, directory, manifest):
    write_json(directory / "manifest.json", manifest)
    out = cfg["output"]
    target = new_file(out["path"])
    plain = Path(cfg["work"]) / "payload.tar"
    with tarfile.open(plain, "x", format=tarfile.USTAR_FORMAT) as archive:
        for path in sorted(directory.iterdir()):
            info = archive.gettarinfo(str(path), arcname=path.name)
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            info.mode = 0o600
            with path.open("rb") as stream:
                archive.addfile(info, stream)
    checksum = sha(plain)
    codec(cfg, "seal", plain, target, out["purpose"])
    return {"path": str(target), "purpose": out["purpose"], "sha256": checksum, "encrypted_sha256": sha(target), "bytes": target.stat().st_size, "manifest_sha256": value_sha(manifest)}


def package_open(cfg, spec, label):
    require(isinstance(spec, dict) and set(spec) == {"path", "purpose", "sha256"}, "PACKAGE_REFERENCE_INVALID")
    require(bool(HEX.fullmatch(spec["sha256"])), "PACKAGE_HASH_INVALID")
    inp = private_file(spec["path"])
    plain = Path(cfg["work"]) / (label + ".tar")
    codec(cfg, "open", inp, plain, spec["purpose"], spec["sha256"])
    require(sha(plain) == spec["sha256"], "AUTHENTICATED_PLAINTEXT_MISMATCH")
    directory = Path(cfg["work"]) / label
    directory.mkdir(mode=0o700)
    with tarfile.open(plain, "r:") as archive:
        seen = set()
        for member in archive:
            require(member.isfile() and re.fullmatch(r"[a-zA-Z_0-9.-]+", member.name) is not None and member.name not in seen and member.size <= 2**32, "ARCHIVE_MEMBER_INVALID")
            seen.add(member.name)
            with archive.extractfile(member) as source, (directory / member.name).open("xb") as target:
                shutil.copyfileobj(source, target)
    manifest = parsed((directory / "manifest.json").read_bytes())
    require(manifest["format"] == FORMAT and manifest["registration_sha256"] == cfg["registration_sha256"], "PACKAGE_REGISTRATION_MISMATCH")
    files = manifest["files"]
    require(seen == {"manifest.json", *(f["file"] for f in files)}, "PACKAGE_FILE_COVERAGE_MISMATCH")
    for item in files:
        path = directory / item["file"]
        require(path.stat().st_size == item["bytes"] and sha(path) == item["sha256"], "PACKAGE_FILE_DIGEST_MISMATCH")
        with path.open("rb") as stream:
            require(sum(1 for _ in stream) == item["rows"], "PACKAGE_FILE_ROWCOUNT_MISMATCH")
    if manifest["type"] == "snapshot":
        validate_snapshot_manifest(cfg, manifest)
    else:
        require(manifest["type"] == "delta", "PACKAGE_TYPE_INVALID")
    return directory, manifest


def validate_snapshot_manifest(cfg, manifest):
    require(manifest["group"] in NAMES, "SNAPSHOT_GROUP_INVALID")
    registered = cfg["reg"]["groups"][manifest["group"]]
    require([d["kind"] for d in manifest["databases"]] == [d["kind"] for d in registered], "SNAPSHOT_DATABASE_COVERAGE_INVALID")
    expected_files = set()
    for observed, db in zip(manifest["databases"], registered):
        kind = db["kind"]
        require(all(observed["identity"][k] == db[k] for k in ("name", "oid", "owner", "owner_oid")), "SNAPSHOT_DATABASE_IDENTITY_INVALID")
        names = cfg["reg"]["fixture_contracts"][kind]["tables"] if cfg["fixture"] else TABLES[kind]
        require([t["name"] for t in observed["tables"]] == names, "SNAPSHOT_CATALOG_TABLE_COVERAGE_INVALID")
        if not cfg["fixture"]:
            require(observed["schema_sha256"] == db["expected_schema_sha256"] and observed["semantic_schema_sha256"] == db["semantic_schema_sha256"], "SNAPSHOT_SCHEMA_PINS_INVALID")
        credentials, metadata, _ = policies(cfg, kind)
        for name in names:
            expected_files.add((kind, name))
            items = [f for f in manifest["files"] if f["kind"] == kind and f["name"] == name]
            require(len(items) == 1, "SNAPSHOT_TABLE_FILE_DUPLICATED_OR_MISSING")
            item = items[0]
            retention = "credential_excluded" if name in credentials else "metadata_rebuilt" if name in metadata else "retained"
            require(item["retention"] == retention and item["file"] == kind + "." + name + ".ndjson" and isinstance(item["rows"], int) and item["rows"] >= 0, "SNAPSHOT_TABLE_POLICY_INVALID")
    require({(f["kind"], f["name"]) for f in manifest["files"]} == expected_files and len(manifest["files"]) == len(expected_files), "SNAPSHOT_COMPLETE_FILE_COVERAGE_INVALID")
    require(len(manifest["admission_observations"]) == len(registered) * 2 and all(p == manifest["admission_before"] == manifest["admission_after"] for p in manifest["admission_observations"]), "SNAPSHOT_FRESH_ADMISSION_INVALID")


def snapshot(cfg):
    group = cfg["group"]
    before = admission(cfg, group)
    directory = Path(cfg["work"]) / "snapshot"
    directory.mkdir(mode=0o700)
    manifest = {"format": FORMAT, "type": "snapshot", "registration_sha256": cfg["registration_sha256"], "group": group, "captured_ns": time.time_ns(), "databases": [], "files": [], "admission_before": before, "admission_observations": []}
    for db in cfg["reg"]["groups"][group]:
        fresh_before = admission(cfg, group)
        require(fresh_before == before, "FRESH_ADMISSION_CHANGED")
        session = Session(db["name"], cfg["work"])
        try:
            observed = inspect(session, db, cfg)
            initial_sequences = sequences(session)
            kind = db["kind"]
            credentials, metadata, _ = policies(cfg, kind)
            observed["kind"] = kind
            observed["sequences"] = initial_sequences
            observed["snapshot"] = session.one("SELECT jsonb_build_object('txid_snapshot',pg_current_snapshot()::text,'lsn',pg_current_wal_lsn()::text);")
            for table in observed["tables"]:
                filename = kind + "." + table["name"] + ".ndjson"
                item = excluded_dump(session, table, directory / filename) if table["name"] in credentials | metadata else stream_table(session, table, directory / filename)
                item.update({"kind": kind, "retention": "credential_excluded" if table["name"] in credentials else "metadata_rebuilt" if table["name"] in metadata else "retained"})
                manifest["files"].append(item)
            require(sequences(session) == initial_sequences, "SOURCE_SEQUENCE_CHANGED")
            require(inspect(session, db, cfg) == {k: observed[k] for k in ("identity", "tables", "schema_sha256", "semantic_schema_sha256", "migrations", "legacy_migrations")}, "SOURCE_CATALOG_CHANGED")
            session.close()
            fresh_after = admission(cfg, group)
            require(fresh_after == before, "FRESH_ADMISSION_CHANGED")
            manifest["admission_observations"].extend([fresh_before, fresh_after])
            manifest["databases"].append(observed)
        except Exception:
            session.abort()
            raise
    after = admission(cfg, group)
    require(before == after, "FRESH_ADMISSION_CHANGED")
    manifest["admission_after"] = after
    receipt = package_seal(cfg, directory, manifest)
    return {"operation": "snapshot", "artifact": receipt, "tables": len(manifest["files"]), "rows": sum(f["rows"] for f in manifest["files"]), "retained_rows": sum(f["rows"] for f in manifest["files"] if f["retention"] == "retained"), "databases": [database_report(d, manifest, d["sequences"]) for d in manifest["databases"]], "admission": "passed"}


def ordered_tables(tables, retained):
    pending = {t["name"]: set(t["dependencies"]) & retained for t in tables if t["name"] in retained}
    ordered = []
    while pending:
        ready = sorted(name for name, deps in pending.items() if not deps)
        require(bool(ready), "CROSS_TABLE_FOREIGN_KEY_CYCLE")
        for name in ready:
            ordered.append(name)
            del pending[name]
        for deps in pending.values():
            deps.difference_update(ready)
    return ordered


def opaque_rows(path):
    with Path(path).open(encoding="utf-8") as stream:
        for line in stream:
            item = parsed(line)
            require(set(item) == {"key", "row_text", "json_texts"} and isinstance(item["row_text"], str) and isinstance(item["key"], list) and all(isinstance(x, str) for x in item["key"]) and isinstance(item["json_texts"], dict) and all(v is None or isinstance(v, str) for v in item["json_texts"].values()), "INVALID_ROW_ENVELOPE")
            yield item


def load_stage(session, rows):
    session.query("TRUNCATE pg_temp.u06_stage;")
    session.copy("pg_temp.u06_stage(envelope)", (canonical(r).decode() for r in rows))


def stage_record(table):
    return "(SELECT json_populate_record(NULL::" + relation(table["name"]) + ",((envelope::json)->>'row_text')::json) r,(envelope::jsonb)->'json_texts' json_texts FROM pg_temp.u06_stage) v"


def stage_value(column):
    value = "(v.r)." + ident(column["name"])
    if column["json_type"]:
        value = "(v.json_texts->>" + literal(column["name"]) + ")::" + column["json_type"]
    return value


def insert_stage(session, table):
    columns = [c for c in table["columns"] if not c["generated"]]
    cols = ",".join(ident(c["name"]) for c in columns)
    values = ",".join(stage_value(c) for c in columns)
    business_statement(session, "INSERT INTO " + relation(table["name"]) + " (" + cols + ") OVERRIDING SYSTEM VALUE SELECT " + values + " FROM " + stage_record(table) + ";")


def business_statement(session, sql):
    # Frozen owner trigger bodies use unqualified public relations. Public is
    # fenced against untrusted CREATE; pg_catalog remains first. Keep this
    # scope around the single reviewed business DML statement, then reset it.
    session.query("SET LOCAL search_path=pg_catalog,public;")
    session.query(sql)
    session.query("SET LOCAL search_path=pg_catalog;")


def file_map(manifest, kind):
    return {f["name"]: f for f in manifest["files"] if f["kind"] == kind and f["retention"] == "retained"}


def database_report(observed, manifest, sequence_state):
    files = file_map(manifest, observed["kind"])
    return {"kind": observed["kind"], "database": observed["identity"]["name"], "database_oid": observed["identity"]["oid"], "owner_oid": observed["identity"]["owner_oid"], "raw_schema_sha256": observed["schema_sha256"], "semantic_schema_sha256": observed["semantic_schema_sha256"], "migration_projection_sha256": value_sha({"sqlx": observed["migrations"], "legacy": observed["legacy_migrations"]}), "rows": sum(f["rows"] for f in files.values()), "table_proofs": [{k: f[k] for k in ("name", "rows", "bytes", "sha256")} for f in files.values()], "sequences": sequence_state, "full_row_reconciliation": "passed"}


def compare_db(session, expected, directory, observed, cfg, label):
    kind = expected["kind"]
    require(observed["semantic_schema_sha256"] == expected["semantic_schema_sha256"] and observed["tables"] == expected["tables"] and observed["migrations"] == expected["migrations"] and observed["legacy_migrations"] == expected["legacy_migrations"], "TARGET_CONTRACT_MISMATCH")
    credentials, _, _ = policies(cfg, kind)
    for name in credentials:
        require(session.one("SELECT count(*)::text::json FROM " + relation(name) + ";") == 0, "TARGET_CREDENTIALS_NOT_EMPTY")
    files = file_map(cfg["comparison_manifest"], kind)
    for table in observed["tables"]:
        if table["name"] not in files:
            continue
        path = Path(cfg["work"]) / (label + "." + kind + "." + table["name"])
        actual = stream_table(session, table, path)
        expected_file = files[table["name"]]
        require(all(actual[k] == expected_file[k] for k in ("rows", "bytes", "sha256")), "TARGET_FULL_ROW_RECONCILIATION_FAILED")
        path.unlink()
    require(sequences(session) == expected["sequences"], "TARGET_SEQUENCE_RECONCILIATION_FAILED")


def restore(cfg):
    require(cfg["group"] in ("final", "recovery"), "SOURCE_WRITES_FORBIDDEN")
    directory, manifest = package_open(cfg, cfg["input"], "restore-input")
    require(manifest["type"] == "snapshot" and cfg["group"] != manifest["group"], "RESTORE_SOURCE_TARGET_MUST_DIFFER")
    admission(cfg, cfg["group"])
    cfg["comparison_manifest"] = manifest
    reports = []
    for db in cfg["reg"]["groups"][cfg["group"]]:
        kind = db["kind"]
        expected = next(x for x in manifest["databases"] if x["kind"] == kind)
        session = Session(db["name"], cfg["work"], write=True)
        try:
            observed = inspect(session, db, cfg)
            require(observed["tables"] == expected["tables"] and observed["migrations"] == expected["migrations"], "RESTORE_MIGRATIONS_OR_COLUMNS_MISMATCH")
            credentials, metadata, _ = policies(cfg, kind)
            for table in observed["tables"]:
                if table["name"] not in metadata:
                    count = session.one("SELECT count(*)::text::json FROM " + relation(table["name"]) + ";")
                    if kind == "platform" and table["name"] == "platform_settings" and count == 1:
                        replace_fresh_platform_default(session)
                    else:
                        require(count == 0, "RESTORE_TARGET_NOT_EMPTY")
            files = file_map(manifest, kind)
            order = ordered_tables(observed["tables"], set(files))
            table_specs = {t["name"]: t for t in observed["tables"]}
            session.query("CREATE TEMP TABLE u06_stage(envelope text NOT NULL) ON COMMIT DROP;")
            for name in order:
                # A legacy BEFORE INSERT FOR EACH STATEMENT trigger rejects
                # even an empty INSERT SELECT (store_staff). No source rows
                # means no business statement; the full reread still proves 0.
                if files[name]["rows"] == 0:
                    continue
                require(kind != "stores" or name != "store_staff", "NONEMPTY_LEGACY_STAFF_REQUIRES_OWNER_CONTRACT")
                load_stage(session, opaque_rows(directory / files[name]["file"]))
                insert_stage(session, table_specs[name])
            align_sequences(session, expected["sequences"])
            final_observed = inspect(session, db, cfg)
            compare_db(session, expected, directory, final_observed, cfg, "restored")
            final_observed["kind"] = kind
            proof = database_report(final_observed, manifest, sequences(session))
            session.close(commit=True)
            reports.append({**proof, "committed": True})
        except Exception:
            session.abort()
            raise
    admission(cfg, cfg["group"])
    return {"operation": "restore", "databases": reports, "full_reconciliation": "passed", "services_opened": False}


def replace_fresh_platform_default(session):
    # Migration 0001 deliberately creates this singleton. Only its complete
    # known defaults at that migration's transaction timestamp may be replaced;
    # any edited row makes the fresh-target admission fail closed.
    values = {"platform_name": "SaaS Platform", "platform_description": "多租户权限管理平台", "logo_url": "", "login_title": "欢迎回来", "login_description": "管理您的租户与应用权限", "footer_text": "© 2026 SaaS Platform · Powered by Rust & React", "primary_color": "#008577"}
    predicate = "singleton AND " + " AND ".join(ident(k) + "=" + literal(v) for k, v in values.items()) + " AND updated_at=(SELECT installed_on FROM public._sqlx_migrations WHERE version=1)"
    valid = session.one("SELECT to_json(count(*)=1 AND COALESCE(bool_and(" + predicate + "),false)) FROM public.platform_settings;")
    require(valid is True, "FRESH_PLATFORM_SETTINGS_DEFAULT_MISMATCH")
    business_statement(session, "DELETE FROM public.platform_settings WHERE " + predicate + ";")


def align_sequences(session, values):
    for item in values:
        require(re.fullmatch(r"-?[0-9]+", item["last_value"]) is not None and isinstance(item["is_called"], bool), "INVALID_SEQUENCE_STATE")
        session.query("SELECT setval(" + literal("public." + ident(item["name"])) + "::regclass," + item["last_value"] + "," + ("true" if item["is_called"] else "false") + ");")


def indexed_rows(path):
    result = {}
    for row in opaque_rows(path):
        key = tuple(row["key"])
        require(key not in result, "DUPLICATE_PRIMARY_KEY_ENVELOPE")
        result[key] = {"row_text": row["row_text"], "json_texts": row["json_texts"]}
    return result


def validate_delta_boundaries(cfg, before, after):
    require(before["type"] == after["type"] == "snapshot", "DELTA_SNAPSHOT_REQUIRED")
    require(before["group"] == after["group"] and (cfg["fixture"] or before["group"] == "final"), "DELTA_REQUIRES_SAME_FINAL_SOURCE")
    require(isinstance(before["captured_ns"], int) and isinstance(after["captured_ns"], int) and before["captured_ns"] < after["captured_ns"], "DELTA_TIME_DIRECTION_INVALID")
    require([d["kind"] for d in before["databases"]] == [d["kind"] for d in after["databases"]], "DELTA_DATABASE_ORDER_MISMATCH")
    for old, new in zip(before["databases"], after["databases"]):
        require(old["identity"] == new["identity"], "DELTA_SOURCE_GENERATION_CHANGED")
        require([s["name"] for s in old["sequences"]] == [s["name"] for s in new["sequences"]], "DELTA_SEQUENCE_SET_CHANGED")
        for a, b in zip(old["sequences"], new["sequences"]):
            require(int(a["last_value"]) <= int(b["last_value"]) and not (a["last_value"] == b["last_value"] and a["is_called"] and not b["is_called"]), "DELTA_SEQUENCE_WOULD_REWIND")


def delta(cfg):
    left_dir, before = package_open(cfg, cfg["before"], "delta-before")
    right_dir, after = package_open(cfg, cfg["after"], "delta-after")
    validate_delta_boundaries(cfg, before, after)
    directory = Path(cfg["work"]) / "delta"
    directory.mkdir(mode=0o700)
    manifest = {"format": FORMAT, "type": "delta", "registration_sha256": cfg["registration_sha256"], "before_manifest_sha256": value_sha(before), "after_manifest_sha256": value_sha(after), "before": cfg["before"], "after": cfg["after"], "files": [], "changes": []}
    for kind in KINDS if not cfg["fixture"] else sorted({d["kind"] for d in before["databases"]}):
        old = file_map(before, kind)
        new = file_map(after, kind)
        require(set(old) == set(new), "DELTA_TABLE_COVERAGE_MISMATCH")
        old_db = next(d for d in before["databases"] if d["kind"] == kind)
        new_db = next(d for d in after["databases"] if d["kind"] == kind)
        require(all(old_db[k] == new_db[k] for k in ("semantic_schema_sha256", "tables", "migrations", "legacy_migrations")), "DELTA_SCHEMA_CHANGED")
        _, _, append_only = policies(cfg, kind)
        for name in sorted(old):
            if old[name]["sha256"] == new[name]["sha256"]:
                continue
            left = indexed_rows(left_dir / old[name]["file"])
            right = indexed_rows(right_dir / new[name]["file"])
            filename = kind + "." + name + ".delta.ndjson"
            count = 0
            counts = {"insert": 0, "update": 0, "delete": 0}
            with (directory / filename).open("xb") as stream:
                for key in sorted(set(left) | set(right)):
                    a, b = left.get(key), right.get(key)
                    if a == b:
                        continue
                    operation = "insert" if a is None else "delete" if b is None else "update"
                    require(name not in append_only or operation == "insert", "APPEND_ONLY_ROW_CHANGED")
                    stream.write(canonical({"key": list(key), "before": a, "after": b, "operation": operation}) + b"\n")
                    count += 1
                    counts[operation] += 1
            item = {"kind": kind, "name": name, "file": filename, "rows": count, "bytes": (directory / filename).stat().st_size, "sha256": sha(directory / filename), "retention": "retained"}
            manifest["files"].append(item)
            manifest["changes"].append({"kind": kind, "table": name, **counts})
    return {"operation": "delta", "artifact": package_seal(cfg, directory, manifest), "changes": manifest["changes"], "row_changes": sum(f["rows"] for f in manifest["files"])}


def key_predicate(table, key, alias="t"):
    require(len(key) == len(table["pk"]) and all(isinstance(v, str) for v in key), "DELTA_KEY_INVALID")
    # Keys are the canonical native PostgreSQL text captured from the source.
    # Values remain literals; no caller-controlled SQL type/expression is used.
    return " AND ".join(alias + "." + ident(c) + "::text=" + literal(v) for c, v in zip(table["pk"], key))


def advance_reviewed_stores_version(session, table, before, after, predicate, kind, schema_pin):
    if kind != "stores" or table["name"] not in ("stores", "store_daily_reports"):
        return 0
    require(schema_pin == "03dd8e2b4eb64e937937cfe037525567ee9c0d0296ca88a65a00e39da152d759", "STORES_VERSION_GUARD_SCHEMA_NOT_REVIEWED")
    projection = "FROM (SELECT json_populate_record(NULL::" + relation(table["name"]) + "," + literal(before["row_text"]) + "::json) r) a CROSS JOIN (SELECT json_populate_record(NULL::" + relation(table["name"]) + "," + literal(after["row_text"]) + "::json) r) b"
    versions = session.one("SELECT json_build_object('before',(a.r).version,'after',(b.r).version) " + projection + ";")
    require(type(versions["before"]) is int and type(versions["after"]) is int and versions["after"] - versions["before"] in (1, 2), "STORES_VERSION_GAP_NOT_SUPPORTED")
    if versions["after"] == versions["before"] + 2:
        # Both reviewed guards only reject identity/version changes. They have
        # no history/effect trigger. This uncommitted version-only step allows
        # the final exact after-image UPDATE to satisfy the +1 guard as well.
        business_statement(session, "UPDATE " + relation(table["name"]) + " t SET version=version+1 WHERE " + predicate + " AND version=" + str(versions["before"]) + ";")
        return 1
    return 0


def apply_row(session, table, item, kind=None, schema_pin=None):
    require(set(item) == {"key", "before", "after", "operation"}, "DELTA_ROW_FIELDS_INVALID")
    before, after, op = item["before"], item["after"], item["operation"]
    require(op in ("insert", "update", "delete") and (before is None) == (op == "insert") and (after is None) == (op == "delete"), "DELTA_OPERATION_INVALID")
    predicate = key_predicate(table, item["key"])
    actual = [parsed(line) for line in session.query(table_sql(table, predicate, lock=True))]
    actual = [{"row_text": r["row_text"], "json_texts": r["json_texts"]} for r in actual]
    require(actual == ([] if before is None else [before]), "DELTA_BEFORE_IMAGE_MISMATCH")
    if op == "delete":
        business_statement(session, "DELETE FROM " + relation(table["name"]) + " t WHERE " + predicate + ";")
        return
    load_stage(session, [{"key": item["key"], **after}])
    if op == "insert":
        insert_stage(session, table)
    else:
        advance_reviewed_stores_version(session, table, before, after, predicate, kind, schema_pin)
        identities = [c["name"] for c in table["columns"] if c["identity"]]
        if identities:
            check = " AND ".join("(a.r)." + ident(c) + " IS NOT DISTINCT FROM (b.r)." + ident(c) for c in identities)
            require(session.one("SELECT to_json(" + check + ") FROM (SELECT json_populate_record(NULL::" + relation(table["name"]) + "," + literal(before["row_text"]) + "::json) r) a CROSS JOIN (SELECT json_populate_record(NULL::" + relation(table["name"]) + "," + literal(after["row_text"]) + "::json) r) b;") is True, "IDENTITY_FIELD_CHANGE_REJECTED")
        columns = [c for c in table["columns"] if not c["generated"] and not c["identity"] and c["name"] not in table["pk"]]
        require(bool(columns), "UPDATE_WITHOUT_WRITABLE_FIELDS")
        assignment = ",".join(ident(c["name"]) + "=" + stage_value(c) for c in columns)
        business_statement(session, "UPDATE " + relation(table["name"]) + " t SET " + assignment + " FROM " + stage_record(table) + " WHERE " + predicate + ";")


def replay(cfg):
    require(cfg["group"] == "recovery", "REPLAY_REQUIRES_RECOVERY_TARGET")
    resume = cfg.get("resume_after_kinds", [])
    require(resume in ([], ["platform"]), "REPLAY_RESUME_PREFIX_INVALID")
    if resume:
        require([d["kind"] for d in cfg["reg"]["groups"][cfg["group"]]] == list(KINDS), "REPLAY_RESUME_REQUIRES_THREE_KINDS")
    delta_dir, changes = package_open(cfg, cfg["input"], "replay-delta")
    require(changes["type"] == "delta", "DELTA_PACKAGE_REQUIRED")
    before_dir, before = package_open(cfg, changes["before"], "replay-before")
    after_dir, after = package_open(cfg, changes["after"], "replay-after")
    validate_delta_boundaries(cfg, before, after)
    require(value_sha(before) == changes["before_manifest_sha256"] and value_sha(after) == changes["after_manifest_sha256"], "DELTA_BOUNDARY_MISMATCH")
    admission(cfg, cfg["group"])
    reports = []
    for db in cfg["reg"]["groups"][cfg["group"]]:
        kind = db["kind"]
        previous = next(d for d in before["databases"] if d["kind"] == kind)
        expected = next(d for d in after["databases"] if d["kind"] == kind)
        session = Session(db["name"], cfg["work"], write=kind not in resume)
        try:
            if kind in resume:
                cfg["comparison_manifest"] = after
                observed = inspect(session, db, cfg)
                try:
                    compare_db(session, expected, after_dir, observed, cfg, "resumed-t1")
                except Invalid as error:
                    raise Invalid("REPLAY_RESUME_TARGET_NOT_AT_T1") from error
                observed["kind"] = kind
                proof = database_report(observed, after, sequences(session))
                session.close()
                reports.append({**proof, "resumed_verified_t1": True, "business_dml_statements": 0, "committed": False})
                continue
            cfg["comparison_manifest"] = before
            observed = inspect(session, db, cfg)
            compare_db(session, previous, before_dir, observed, cfg, "before-replay")
            session.query("CREATE TEMP TABLE u06_stage(envelope text NOT NULL) ON COMMIT DROP;")
            files = file_map(changes, kind)
            retained = set(file_map(before, kind))
            order = ordered_tables(observed["tables"], retained)
            tables = {t["name"]: t for t in observed["tables"]}
            _, _, append_only = policies(cfg, kind)
            actions = {}
            for name, file in files.items():
                with (delta_dir / file["file"]).open() as stream:
                    actions[name] = [parsed(line) for line in stream]
                require(name not in append_only or all(x["operation"] == "insert" for x in actions[name]), "APPEND_ONLY_DELTA_REJECTED")
            # Deletes go from children to parents; inserts/updates from parents
            # to children. Unsupported cross-table dependency changes fail FK
            # checks with triggers fully enabled and never open the target.
            for name in reversed(order):
                for item in actions.get(name, []):
                    if item["operation"] == "delete":
                        apply_row(session, tables[name], item, kind, observed["semantic_schema_sha256"])
            for name in order:
                for item in actions.get(name, []):
                    if item["operation"] != "delete":
                        apply_row(session, tables[name], item, kind, observed["semantic_schema_sha256"])
            align_sequences(session, expected["sequences"])
            cfg["comparison_manifest"] = after
            final_observed = inspect(session, db, cfg)
            compare_db(session, expected, after_dir, final_observed, cfg, "after-replay")
            final_observed["kind"] = kind
            proof = database_report(final_observed, after, sequences(session))
            session.close(commit=True)
            reports.append({**proof, "changes": sum(len(a) for a in actions.values()), "committed": True})
        except Exception:
            session.abort()
            raise
    admission(cfg, cfg["group"])
    return {"operation": "replay", "databases": reports, "full_reconciliation": "passed", "services_opened": False}


def verify(cfg):
    directory, manifest = package_open(cfg, cfg["input"], "verify-input")
    require(manifest["type"] == "snapshot", "VERIFY_SNAPSHOT_REQUIRED")
    admission(cfg, cfg["group"])
    cfg["comparison_manifest"] = manifest
    reports = []
    for db in cfg["reg"]["groups"][cfg["group"]]:
        session = Session(db["name"], cfg["work"])
        try:
            expected = next(d for d in manifest["databases"] if d["kind"] == db["kind"])
            observed = inspect(session, db, cfg)
            compare_db(session, expected, directory, observed, cfg, "verified")
            observed["kind"] = db["kind"]
            reports.append(database_report(observed, manifest, sequences(session)))
            session.close()
        except Exception:
            session.abort()
            raise
    admission(cfg, cfg["group"])
    return {"operation": "verify", "full_reconciliation": "passed", "tables": sum(f["retention"] == "retained" for f in manifest["files"]), "databases": reports, "services_opened": False}


def configuration(path):
    require(os.name == "posix" and os.geteuid() == 0, "WSL_ROOT_REQUIRED")
    os.umask(0o077)
    cfg = parsed(private_file(path, 1024 * 1024).read_bytes())
    allowed = {"format", "profile", "registration", "scratch", "codec", "group", "input", "before", "after", "output", "report", "resume_after_kinds"}
    require(isinstance(cfg, dict) and set(cfg) <= allowed and {"format", "profile", "registration", "scratch", "codec", "report"} <= set(cfg), "CONFIGURATION_FIELDS_INVALID")
    require(cfg["format"] == FORMAT and cfg["profile"] in ("uuid-local-u06", "uuid-u06-fixture"), "PROFILE_INVALID")
    cfg["fixture"] = cfg["profile"] == "uuid-u06-fixture"
    reg_path = private_file(cfg["registration"], 1024 * 1024)
    cfg["reg"] = parsed(reg_path.read_bytes())
    cfg["registration_sha256"] = sha(reg_path)
    reg = cfg["reg"]
    require(set(reg) == ({"format", "system_identifier", "groups", "fenced_roles", "fixture_contracts"} if cfg["fixture"] else {"format", "system_identifier", "groups", "fenced_roles"}), "REGISTRATION_FIELDS_INVALID")
    require(reg["format"] == REG_FORMAT and re.fullmatch(r"[0-9]+", reg["system_identifier"]) is not None and set(reg["groups"]) == set(NAMES), "REGISTRATION_INVALID")
    all_oids = set()
    all_owner_oids = set()
    for group, databases in reg["groups"].items():
        require(0 < len(databases) <= 3 and (cfg["fixture"] or len(databases) == 3), "REGISTRATION_DATABASE_COUNT_INVALID")
        require(len({d["kind"] for d in databases}) == len(databases), "REGISTRATION_DUPLICATE_KIND")
        for i, db in enumerate(databases):
            required_fields = {"kind", "name", "oid", "owner", "owner_oid"}
            require(set(db) == (required_fields if cfg["fixture"] else required_fields | {"expected_schema_sha256", "semantic_schema_sha256"}) and db["kind"] in KINDS, "REGISTRATION_DATABASE_FIELDS_INVALID")
            require(db["oid"] not in all_oids and isinstance(db["oid"], int) and db["oid"] > 0 and isinstance(db["owner_oid"], int) and db["owner_oid"] > 0, "REGISTRATION_OID_INVALID")
            all_oids.add(db["oid"])
            require(db["owner_oid"] not in all_owner_oids, "REGISTRATION_OWNER_GENERATIONS_NOT_ISOLATED")
            all_owner_oids.add(db["owner_oid"])
            if cfg["fixture"]:
                require(re.fullmatch(r"uuid_u06_fixture_[a-z0-9_]+", db["name"]) is not None and re.fullmatch(r"uuid_u06_fixture_[a-z0-9_]+", db["owner"]) is not None, "FIXTURE_IDENTITY_INVALID")
            else:
                require(db["kind"] == KINDS[i] and db["name"] == NAMES[group][i] and db["owner"] == OWNERS[group][i], "CLOSED_TARGET_NAME_MISMATCH")
                require(bool(HEX.fullmatch(db["expected_schema_sha256"])) and bool(HEX.fullmatch(db["semantic_schema_sha256"])) and (group != "source" or db["expected_schema_sha256"] == SCHEMAS[db["kind"]]), "SCHEMA_PIN_INVALID")
        require(group in reg["fenced_roles"] and isinstance(reg["fenced_roles"][group], list), "FENCED_ROLE_REGISTRATION_MISSING")
        if not cfg["fixture"]:
            require(len(reg["fenced_roles"][group]) == 4 and set(reg["fenced_roles"][group]) == set(RUNTIMES[group]), "CLOSED_RUNTIME_ROLE_SET_REQUIRED")
    if not cfg["fixture"]:
        require(all(len({reg["groups"][group][i]["semantic_schema_sha256"] for group in NAMES}) == 1 for i in range(3)), "REGISTERED_SCHEMA_GENERATIONS_NOT_EQUIVALENT")
    else:
        require(len({tuple(d["kind"] for d in reg["groups"][group]) for group in NAMES}) == 1, "FIXTURE_KIND_SET_MISMATCH")
        for contract in reg["fixture_contracts"].values():
            require(set(contract) == {"tables", "schema_sha256", "migration_count", "credentials", "metadata", "append_only"}, "FIXTURE_CONTRACT_FIELDS_INVALID")
    scratch = private_dir(cfg["scratch"], tmpfs=True)
    require(all(hashlib.sha256(path.read_text().encode()).hexdigest() == SCHEMA_QUERY_SHA256 for path in SCHEMA_SQL.values()), "SCHEMA_QUERY_SOURCE_CHANGED")
    codec_spec = cfg["codec"]
    require(set(codec_spec) == {"binary", "sha256", "key_file"} and bool(HEX.fullmatch(codec_spec["sha256"])), "CODEC_SPEC_INVALID")
    binary = checked_path(codec_spec["binary"])
    info = binary.stat()
    require(stat.S_ISREG(info.st_mode) and info.st_uid == 0 and info.st_nlink == 1 and not (info.st_mode & 0o022) and sha(binary) == codec_spec["sha256"], "CODEC_IDENTITY_INVALID")
    require(str(binary) == CODEC_BINARY, "CLOSED_CODEC_REQUIRED")
    require(private_file(codec_spec["key_file"]).stat().st_size == 32, "KEY_SIZE_INVALID")
    new_file(cfg["report"])
    if "group" in cfg:
        require(cfg["group"] in NAMES, "GROUP_INVALID")
    cfg["work"] = tempfile.mkdtemp(prefix="u06-data-", dir=scratch)
    return cfg


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    for name in ("snapshot", "restore", "delta", "replay", "verify"):
        group.add_argument("--" + name, metavar="PRIVATE_CONFIG")
    args = parser.parse_args()
    operation, path = next((k, v) for k, v in vars(args).items() if v is not None)
    cfg = None
    start = time.monotonic()
    try:
        cfg = configuration(path)
        require(operation == "replay" or "resume_after_kinds" not in cfg, "RESUME_ONLY_ALLOWED_FOR_REPLAY")
        Session.diagnostic_dir = Path(cfg["report"]).parent / (Path(cfg["report"]).stem + "-diagnostics")
        Session.diagnostic_files = []
        result = globals()[operation](cfg)
        result.update({"format": FORMAT, "elapsed_seconds": round(time.monotonic() - start, 3)})
        write_json(cfg["report"], result)
        print(json.dumps({k: v for k, v in result.items() if k != "artifact"}, sort_keys=True))
        return 0
    except Invalid as error:
        if cfg is not None:
            write_json(cfg["report"], {"format": FORMAT, "operation": operation, "status": "REJECTED", "code": str(error), "diagnostics": Session.diagnostic_files, "elapsed_seconds": round(time.monotonic() - start, 3)})
        print("U06_DATA_REJECTED:" + str(error), file=sys.stderr)
        return 1
    except Exception:
        if cfg is not None and not Path(cfg["report"]).exists():
            write_json(cfg["report"], {"format": FORMAT, "operation": operation, "status": "REJECTED", "code": "PRIVATE_OPERATION_FAILED", "diagnostics": Session.diagnostic_files, "elapsed_seconds": round(time.monotonic() - start, 3)})
        print("U06_DATA_REJECTED:PRIVATE_OPERATION_FAILED", file=sys.stderr)
        return 1
    finally:
        if cfg is not None:
            shutil.rmtree(cfg["work"], ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
