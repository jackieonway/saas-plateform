#!/usr/bin/env python3
"""Record or recreate only the fixed, unopened UUID-U04 rehearsal targets.

Default is read-only catalog/Docker observation, writing one new private receipt:
  recreate-uuid-preview-targets.py --output /private/record.json \
      --assert-u04-never-opened
To recreate that exact sealed group, preserving owners and every source:
  recreate-uuid-preview-targets.py --execute --receipt /private/record.json \
      --output /private/recreated.json --assert-u04-never-opened

The assertion comes from the trusted maintenance operator. Current PostgreSQL
catalogs cannot prove that a role was never temporarily enabled in the past.
This is NOT recovery after opening service traffic or new business writes.
After recreation the targets are CREATED_UNMIGRATED, default writable, with
NOLOGIN owners and PUBLIC admission revoked. Run real migrations, then seal.
No source alteration, role deletion, FORCE, data replay, or service start exists.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import time
import uuid

TARGETS = (
    ("platform", "qiankun_platform_uuid7_u04_20261024", "qiankun_platform_uuid7_u04_owner_20261024"),
    ("stores", "qiankun_stores_uuid7_u04_20261024", "qiankun_stores_uuid7_u04_owner_20261024"),
    ("work_orders", "qiankun_orders_uuid7_u04_20261024", "qiankun_orders_uuid7_u04_owner_20261024"),
)
SOURCES = (
    "qiankun_platform_preview_20261024",
    "qiankun_stores_preview_20261024",
    "qiankun_orders_preview_20261024",
)
SOURCE_ROLES = ("qiankun_preview_20261024", "qiankun_notification_preview_20261024")
WRITERS = (
    "qiankun-platform-preview-20261024", "qiankun-stores-preview-20261024",
    "codex-qiankun-orders-preview-api-20261024", "qiankun-preview-notification-20261024",
    "qiankun-preview-examples-oauth-20261024", "qiankun-preview-examples-chat-20261024",
)
AUTO_REMOVED = "qiankun-preview-notification-20261024"
FORMAT = "saas-uuid-u04-target-recreation/1"
SEALED = "SEALED_U04_TARGETS_RECORDED"
UNMIGRATED = "CREATED_UNMIGRATED"
POLICY = "fixed-unopened-local-preview-u04/1"
ADVISORY_LOCK = 823140420261024
LIMIT = 4 * 1024 * 1024


class RecreateError(Exception):
    """Static public errors; full identity receipts remain private."""


def require(condition, message):
    if not condition:
        raise RecreateError(message)


def strict_pairs(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate receipt JSON key")
        result[key] = value
    return result


def decode(raw):
    try:
        return json.loads(raw, object_pairs_hook=strict_pairs,
                          parse_constant=lambda _: (_ for _ in ()).throw(RecreateError("invalid JSON number")))
    except (ValueError, UnicodeError) as error:
        raise RecreateError("invalid private receipt JSON") from error


def exact(value, keys):
    require(type(value) is dict and set(value) == set(keys), "incomplete or extended private evidence")


def oid(value):
    return type(value) is int and 0 < value <= 0xFFFFFFFF


def literal(value):
    return "'" + value.replace("'", "''") + "'"


def values(items):
    return ",".join(literal(item) for item in items)


def marker(value):
    if value is None:
        return True
    if type(value) is not str:
        return False
    match = re.fullmatch(r"saas-uuid-import/1/([0-9a-f-]{36})/([0-9a-f]{64})", value)
    if not match:
        return False
    try:
        batch = uuid.UUID(match[1])
    except ValueError:
        return False
    return str(batch) == match[1] and batch.version == 7 and batch.variant == uuid.RFC_4122


def canonical_path(raw):
    require(type(raw) is str and raw.startswith("/") and not raw.startswith("//")
            and all(part not in ("", ".", "..") for part in raw.split("/")[1:]),
            "absolute canonical Linux path required")
    path = Path(raw)
    for parent in reversed((path, *path.parents)):
        try:
            metadata = parent.lstat()
        except FileNotFoundError:
            continue
        require(not stat.S_ISLNK(metadata.st_mode), "receipt symlink path refused")
    return path


def private(path, directory=False):
    metadata = path.lstat()
    require(metadata.st_uid == os.geteuid()
            and stat.S_IMODE(metadata.st_mode) == (0o700 if directory else 0o600)
            and (stat.S_ISDIR(metadata.st_mode) if directory else stat.S_ISREG(metadata.st_mode)),
            "owned private regular receipt or directory required")
    return metadata


def read_receipt(path):
    private(path.parent, directory=True)
    before = private(path)
    require(before.st_size <= LIMIT, "receipt size limit exceeded")
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(descriptor, "rb") as source:
        opened = os.fstat(source.fileno())
        require((before.st_dev, before.st_ino) == (opened.st_dev, opened.st_ino), "receipt identity changed")
        raw = source.read(LIMIT + 1)
        after = os.fstat(source.fileno())
    require(len(raw) <= LIMIT and (opened.st_size, opened.st_mtime_ns) == (after.st_size, after.st_mtime_ns),
            "receipt changed during read")
    return decode(raw), hashlib.sha256(raw).hexdigest()


def reserve_output(path):
    private(path.parent, directory=True)
    require(not path.exists() and not path.is_symlink(), "new receipt output required")
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    os.fchmod(descriptor, 0o600)
    private(path)
    return descriptor


def finish_output(path, descriptor, receipt):
    raw = (json.dumps(receipt, sort_keys=True, ensure_ascii=True, indent=2) + "\n").encode()
    require(len(raw) <= LIMIT, "receipt output size limit exceeded")
    with os.fdopen(descriptor, "wb") as output:
        current = path.lstat()
        opened = os.fstat(output.fileno())
        require((current.st_dev, current.st_ino) == (opened.st_dev, opened.st_ino), "new receipt inode changed")
        output.write(raw)
        output.flush()
        os.fsync(output.fileno())
    private(path)
    directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)
    return hashlib.sha256(raw).hexdigest()


def catalog_select():
    target_names = [name for _, name, _ in TARGETS]
    owners = [owner for _, _, owner in TARGETS]
    databases = values((*SOURCES, *target_names))
    privileged = values((*SOURCE_ROLES, *owners))
    return f"""SELECT jsonb_build_object(
 'actor',current_user,'session_actor',session_user,
 'actor_oid',(SELECT oid::bigint FROM pg_roles WHERE rolname=current_user),
 'maintenance_oid',(SELECT oid::bigint FROM pg_database WHERE datname=current_database()),
 'maintenance_database',current_database(),
 'system_identifier',(SELECT system_identifier::text FROM pg_control_system()),
 'local_socket',inet_server_addr() IS NULL AND inet_server_port() IS NULL,
 'port',current_setting('port')::int,'server_version_num',current_setting('server_version_num')::int,
 'readonly_observer',current_setting('transaction_read_only')::boolean,
 'isolation',current_setting('transaction_isolation'),
 'roles',(SELECT COALESCE(jsonb_agg(jsonb_build_object(
   'name',r.rolname,'oid',r.oid::bigint,'login',r.rolcanlogin,'superuser',r.rolsuper,
   'create_db',r.rolcreatedb,'create_role',r.rolcreaterole,'replication',r.rolreplication,'bypass_rls',r.rolbypassrls,
   'privileged_set',EXISTS(SELECT 1 FROM pg_roles p WHERE
       (p.rolname IN ({privileged},'pg_execute_server_program','pg_write_server_files','pg_read_server_files')
        OR p.rolsuper OR p.rolcreaterole) AND pg_has_role(r.oid,p.oid,'SET')),
   'can_admin_owners',EXISTS(SELECT 1 FROM pg_auth_members m JOIN pg_roles p ON p.oid=m.roleid
       WHERE p.rolname IN ({privileged}) AND m.admin_option AND pg_has_role(r.oid,m.member,'SET')),
   'connect',(SELECT jsonb_agg(jsonb_build_object('name',d.datname,'oid',d.oid::bigint,
       'allowed',has_database_privilege(r.oid,d.oid,'CONNECT')) ORDER BY d.datname::text COLLATE "C")
       FROM pg_database d WHERE d.datname IN ({databases}))) ORDER BY r.rolname::text COLLATE "C"),'[]'::jsonb)
       FROM pg_roles r),
 'databases',(SELECT COALESCE(jsonb_agg(jsonb_build_object(
   'name',d.datname,'oid',d.oid::bigint,'owner',owner.rolname,'owner_oid',owner.oid::bigint,
   'encoding',pg_encoding_to_char(d.encoding),'collate',d.datcollate,'ctype',d.datctype,
   'allow_connections',d.datallowconn,'is_template',d.datistemplate,
   'readonly',EXISTS(SELECT 1 FROM pg_db_role_setting s WHERE s.setdatabase=d.oid AND s.setrole=0
       AND s.setconfig @> ARRAY['default_transaction_read_only=on']),
   'public_connect',EXISTS(SELECT 1 FROM aclexplode(COALESCE(d.datacl,acldefault('d',d.datdba))) a
       WHERE a.grantee=0 AND a.privilege_type='CONNECT'),
   'backends',(SELECT count(*) FROM pg_stat_activity a WHERE a.datid=d.oid),
   'prepared',(SELECT count(*) FROM pg_prepared_xacts p WHERE p.database=d.datname),
   'comment',shobj_description(d.oid,'pg_database')) ORDER BY d.datname::text COLLATE "C"),'[]'::jsonb)
       FROM pg_database d JOIN pg_roles owner ON owner.oid=d.datdba WHERE d.datname IN ({databases})))"""


class Runner:
    def sql(self, statement, readonly=True):
        env = {"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LC_ALL": "C", "LANG": "C.UTF-8",
               "PGPASSFILE": "/dev/null", "PGCONNECT_TIMEOUT": "10",
               "PGOPTIONS": "-c default_transaction_read_only=" + ("on" if readonly else "off")
               + " -c statement_timeout=30000 -c lock_timeout=3000"}
        command = ["/usr/bin/psql", "-X", "-Atq", "--no-password", "-v", "ON_ERROR_STOP=1",
                   "-h", "/run/postgresql", "-p", "5432", "-U", "postgres", "-d", "postgres", "--file=-"]
        if os.geteuid() == 0:
            command = ["/usr/sbin/runuser", "-u", "postgres", "--", *command]
        try:
            result = subprocess.run(command, input=statement, text=True, capture_output=True,
                                    timeout=180, env=env)
        except (OSError, subprocess.TimeoutExpired) as error:
            raise RecreateError("fixed local maintenance command unavailable or timed out") from error
        require(result.returncode == 0, "target recreation failed; keep the entire group quarantined")
        return result.stdout.strip()

    def observe(self):
        return decode(self.sql("BEGIN ISOLATION LEVEL READ COMMITTED READ ONLY;\n"
                               "SET LOCAL search_path=pg_catalog;\n" + catalog_select() + ";\nCOMMIT;\n"))

    def docker(self, arguments):
        try:
            result = subprocess.run(["/usr/bin/docker", "--host", "unix:///run/docker.sock", *arguments],
                                    capture_output=True, text=True, timeout=30,
                                    env={"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LC_ALL": "C"})
        except (OSError, subprocess.TimeoutExpired) as error:
            raise RecreateError("fixed local Docker observation unavailable") from error
        require(result.returncode == 0, "fixed local Docker observation failed")
        return result.stdout

    def writers(self):
        names = {}
        for line in self.docker(["ps", "--all", "--no-trunc", "--format", "{{.ID}}\t{{.Names}}"] ).splitlines():
            parts = line.split("\t")
            require(len(parts) == 2 and re.fullmatch(r"[0-9a-f]{64}", parts[0]) is not None
                    and parts[1] not in names, "unexpected Docker inventory identity")
            names[parts[1]] = parts[0]
        result = []
        template = ('{"id":{{json .Id}},"name":{{json .Name}},"running":{{json .State.Running}},'
                    '"paused":{{json .State.Paused}},"restarting":{{json .State.Restarting}},'
                    '"pid":{{json .State.Pid}},"restart_policy":{{json .HostConfig.RestartPolicy.Name}}}')
        for name in WRITERS:
            if name not in names:
                require(name == AUTO_REMOVED, "a reviewed stopped writer disappeared")
                result.append({"name": name, "present": False, "id": None, "state": None})
                continue
            state = decode(self.docker(["container", "inspect", "--format", template, names[name]]))
            result.append({"name": name, "present": True, "id": names[name], "state": state})
        validate_writers(result)
        return result


def validate_writers(writers):
    require(type(writers) is list and len(writers) == len(WRITERS), "complete stopped-writer proof required")
    for expected, writer in zip(WRITERS, writers):
        exact(writer, ("name", "present", "id", "state"))
        require(writer["name"] == expected and type(writer["present"]) is bool, "writer identity mismatch")
        if not writer["present"]:
            require(expected == AUTO_REMOVED and writer["id"] is None and writer["state"] is None,
                    "only the reviewed auto-removed notification writer may be absent")
            continue
        state = writer["state"]
        exact(state, ("id", "name", "running", "paused", "restarting", "pid", "restart_policy"))
        require(type(writer["id"]) is str and re.fullmatch(r"[0-9a-f]{64}", writer["id"]) is not None
                and state["id"] == writer["id"] and state["name"] == "/" + expected
                and state["running"] is False and state["paused"] is False and state["restarting"] is False
                and type(state["pid"]) is int and state["pid"] == 0 and state["restart_policy"] == "no",
                "all reviewed writer containers must remain stopped without restart")


def validate_proof(proof, sealed=True):
    exact(proof, ("actor", "session_actor", "actor_oid", "maintenance_oid", "maintenance_database",
                  "system_identifier", "local_socket", "port", "server_version_num", "readonly_observer",
                  "isolation", "roles", "databases"))
    require(proof["actor"] == proof["session_actor"] == "postgres" and oid(proof["actor_oid"])
            and oid(proof["maintenance_oid"]) and proof["maintenance_database"] == "postgres"
            and type(proof["system_identifier"]) is str and re.fullmatch(r"[0-9]{1,20}", proof["system_identifier"]) is not None
            and 0 < int(proof["system_identifier"]) <= 0xFFFFFFFFFFFFFFFF
            and proof["local_socket"] is True and type(proof["port"]) is int and proof["port"] == 5432
            and type(proof["server_version_num"]) is int and proof["server_version_num"] >= 170000
            and proof["readonly_observer"] is True and proof["isolation"] == "read committed",
            "fixed local read-only postgres maintenance identity required")
    roles, role_oids = {}, set()
    require(type(proof["roles"]) is list, "complete role proof required")
    for role in proof["roles"]:
        exact(role, ("name", "oid", "login", "superuser", "create_db", "create_role", "replication", "bypass_rls",
                     "privileged_set", "can_admin_owners", "connect"))
        require(type(role["name"]) is str and 0 < len(role["name"]) <= 63 and role["name"] not in roles
                and oid(role["oid"]) and role["oid"] not in role_oids
                and all(type(role[key]) is bool for key in ("login", "superuser", "create_db", "create_role", "replication",
                                                            "bypass_rls", "privileged_set", "can_admin_owners")),
                "invalid role identity or flags")
        roles[role["name"]] = role
        role_oids.add(role["oid"])
    require("postgres" in roles and roles["postgres"]["oid"] == proof["actor_oid"]
            and roles["postgres"]["superuser"] and roles["postgres"]["login"], "postgres administrator identity mismatch")
    for name in (*SOURCE_ROLES, *(owner for _, _, owner in TARGETS)):
        require(name in roles and not roles[name]["login"] and not roles[name]["superuser"], "source and target owners must remain NOLOGIN")
    for _, _, owner in TARGETS:
        require(not any(roles[owner][key] for key in ("create_db", "create_role", "replication", "bypass_rls")),
                "target owner has an unexpected administrative privilege")
    require(type(proof["databases"]) is list and len(proof["databases"]) == 6, "six fixed source/target databases required")
    databases, database_oids = {}, set()
    target_owners = {name: owner for _, name, owner in TARGETS}
    for database in proof["databases"]:
        exact(database, ("name", "oid", "owner", "owner_oid", "encoding", "collate", "ctype", "allow_connections",
                         "is_template", "readonly", "public_connect", "backends", "prepared", "comment"))
        name = database["name"]
        require(type(name) is str and name in (*SOURCES, *target_owners) and name not in databases
                and oid(database["oid"]) and database["oid"] not in database_oids
                and database["oid"] != proof["maintenance_oid"] and type(database["owner"]) is str and database["owner"] in roles
                and database["owner_oid"] == roles[database["owner"]]["oid"]
                and database["encoding"] == "UTF8" and all(type(database[k]) is str and database[k] for k in ("collate", "ctype"))
                and database["allow_connections"] is True and database["is_template"] is False
                and type(database["readonly"]) is bool and type(database["public_connect"]) is bool
                and type(database["backends"]) is int and database["backends"] == 0
                and type(database["prepared"]) is int and database["prepared"] == 0,
                "database identity, admission, or idle state mismatch")
        if name in SOURCES:
            require(database["owner"] == SOURCE_ROLES[0] and database["readonly"], "source must stay frozen with its original owner")
        else:
            require(database["owner"] == target_owners[name] and not database["public_connect"]
                    and database["readonly"] is sealed and marker(database["comment"]), "sealed rehearsal target required")
            if not sealed:
                require(database["comment"] is None, "newly recreated target must be unmigrated and unmarked")
        databases[name] = database
        database_oids.add(database["oid"])
    require(set(databases) == set((*SOURCES, *target_owners)), "fixed database set mismatch")
    for name, role in roles.items():
        require(type(role["connect"]) is list and len(role["connect"]) == 6, "complete effective CONNECT proof required")
        connects = {}
        for connect in role["connect"]:
            exact(connect, ("name", "oid", "allowed"))
            require(type(connect["name"]) is str and connect["name"] in databases and connect["name"] not in connects
                    and connect["oid"] == databases[connect["name"]]["oid"] and type(connect["allowed"]) is bool,
                    "CONNECT proof identity mismatch")
            connects[connect["name"]] = connect["allowed"]
        if name != "postgres" and role["login"]:
            require(not any(connects.values()) and not any(role[key] for key in
                    ("superuser", "create_role", "privileged_set", "can_admin_owners")), "another login can enter or administer the frozen group")
        if name == "postgres":
            require(all(connects.values()), "maintenance postgres cannot reach the exact group")
    return databases


def receipt_document(proof, writers, asserted, tool_hash, status=SEALED, parent=None):
    return {"format": FORMAT, "profile": "uuid-local-preview", "policy": POLICY, "status": status,
            "captured_ms": time.time_ns() // 1_000_000, "tool_sha256": tool_hash,
            "trusted_maintenance": {"phase": "UUID-U04", "never_opened_asserted": asserted,
                                    "no_new_business_writes_asserted": asserted,
                                    "catalog_does_not_prove_historical_nonopening": True},
            "parent_receipt_sha256": parent, "source_and_target_proof": proof, "stopped_writers": writers}


def validate_receipt(receipt, current, writers, tool_hash):
    exact(receipt, ("format", "profile", "policy", "status", "captured_ms", "tool_sha256", "trusted_maintenance",
                    "parent_receipt_sha256", "source_and_target_proof", "stopped_writers"))
    require(receipt["format"] == FORMAT and receipt["profile"] == "uuid-local-preview" and receipt["policy"] == POLICY
            and receipt["status"] == SEALED and receipt["tool_sha256"] == tool_hash
            and type(receipt["captured_ms"]) is int and 0 < receipt["captured_ms"] <= time.time_ns() // 1_000_000,
            "eligible original sealed U04 receipt required")
    exact(receipt["trusted_maintenance"], ("phase", "never_opened_asserted", "no_new_business_writes_asserted",
                                           "catalog_does_not_prove_historical_nonopening"))
    require(receipt["trusted_maintenance"] == {"phase": "UUID-U04", "never_opened_asserted": True,
                "no_new_business_writes_asserted": True, "catalog_does_not_prove_historical_nonopening": True}
            and receipt["parent_receipt_sha256"] is None, "explicit unopened U04 maintenance assertion required")
    original = receipt["source_and_target_proof"]
    old_databases = validate_proof(original)
    validate_writers(receipt["stopped_writers"])
    require(receipt["stopped_writers"] == writers, "stopped writer generation changed since recording")
    current_databases = validate_proof(current)
    normalized = json.loads(json.dumps(current))
    for database in normalized["databases"]:
        old = old_databases[database["name"]]
        require((database["oid"], database["owner"], database["owner_oid"])
                == (old["oid"], old["owner"], old["owner_oid"]), "recorded database or owner OID changed")
        if database["name"] not in SOURCES:
            require(old["comment"] is None or database["comment"] == old["comment"], "target import marker changed since recording")
            database["comment"] = old["comment"]
    require(normalized == original and set(current_databases) == set(old_databases), "complete recorded maintenance identities changed")


def recreation_sql(current):
    # A session advisory lock serializes this tool. The complete identity guard
    # is re-read after acquiring it, before any DROP. Trusted postgres must not
    # run concurrent DDL; DROP DATABASE has no atomic OID-qualified form.
    expected = literal(json.dumps(current, sort_keys=True, ensure_ascii=True, separators=(",", ":")))
    sql = [f"SELECT pg_advisory_lock({ADVISORY_LOCK});", "BEGIN ISOLATION LEVEL READ COMMITTED READ ONLY;",
           "SET LOCAL search_path=pg_catalog;", f"SELECT ({catalog_select()}) = {expected}::jsonb AS exact_group \\gset",
           "COMMIT;", "\\if :exact_group"]
    for _, name, _ in TARGETS:
        sql.append(f'DROP DATABASE "{name}";')
    for _, name, owner in TARGETS:
        sql.extend([f'CREATE DATABASE "{name}" OWNER "{owner}" TEMPLATE template0 ENCODING \'UTF8\';',
                    f'REVOKE ALL ON DATABASE "{name}" FROM PUBLIC;',
                    f'ALTER DATABASE "{name}" SET default_transaction_read_only=off;'])
    sql.extend([f"SELECT pg_advisory_unlock({ADVISORY_LOCK});", "\\else", "\\quit 4", "\\endif"])
    return "\n".join(sql) + "\n"


def validate_recreated(before, after):
    old = validate_proof(before)
    new = validate_proof(after, sealed=False)
    require(before["system_identifier"] == after["system_identifier"] and before["actor_oid"] == after["actor_oid"]
            and before["maintenance_oid"] == after["maintenance_oid"], "maintenance instance changed during recreation")
    for name in SOURCES:
        require(old[name] == new[name], "source proof changed during target recreation")
    old_roles = {role["name"]: role for role in before["roles"]}
    new_roles = {role["name"]: role for role in after["roles"]}
    require(set(old_roles) == set(new_roles), "role set changed during recreation")
    for name, original in old_roles.items():
        normalized = json.loads(json.dumps(new_roles[name]))
        # Target database OIDs necessarily change; all role identities, flags,
        # and effective source/target admission decisions must remain identical.
        for connect in normalized["connect"]:
            connect["oid"] = old[connect["name"]]["oid"]
        require(normalized == original, "role identities or admission changed during recreation")
    for _, name, owner in TARGETS:
        require(new[name]["oid"] not in {item["oid"] for item in old.values()}
                and new[name]["owner"] == owner and new[name]["owner_oid"] == old[name]["owner_oid"]
                and all(new[name][field] == old[name][field] for field in ("encoding", "collate", "ctype")),
                "recreated target or retained owner identity mismatch")


def main(argv=None, runner=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--receipt", help="existing private sealed-group receipt, required only with --execute")
    parser.add_argument("--output", required=True, help="new absolute file in an existing owned mode-0700 directory")
    parser.add_argument("--assert-u04-never-opened", action="store_true",
                        help="trusted operator asserts no service opening or new business writes occurred")
    args = parser.parse_args(argv)
    require(sys.platform == "linux", "recreation receipts require local Linux maintenance")
    require(bool(args.receipt) == args.execute, "existing receipt is required exactly for explicit execution")
    require(not args.execute or args.assert_u04_never_opened, "execution requires explicit unopened U04 assertion")
    require(not (set(name for _, name, _ in TARGETS) & set(SOURCES)), "fixed source/target collision")
    output = canonical_path(args.output)
    private(output.parent, directory=True)
    require(not output.exists() and not output.is_symlink(), "new receipt output required")
    old_receipt, parent_hash = (None, None)
    if args.receipt:
        input_path = canonical_path(args.receipt)
        require(input_path != output, "new receipt must differ from its input")
        old_receipt, parent_hash = read_receipt(input_path)
    tool_hash = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    runner = runner or Runner()
    before = runner.observe()
    validate_proof(before)
    writers = runner.writers()
    validate_writers(writers)
    if args.execute:
        validate_receipt(old_receipt, before, writers, tool_hash)
    status = SEALED if args.assert_u04_never_opened else "OBSERVED_ONLY_NOT_DESTRUCTIVE_INPUT"
    receipt = receipt_document(before, writers, args.assert_u04_never_opened, tool_hash, status)
    descriptor = reserve_output(output)
    try:
        if args.execute:
            runner.sql(recreation_sql(before), readonly=False)
            after = runner.observe()
            validate_recreated(before, after)
            after_writers = runner.writers()
            require(after_writers == writers, "stopped writers changed during recreation")
            receipt = receipt_document(after, after_writers, True, tool_hash, UNMIGRATED, parent_hash)
        owned_descriptor, descriptor = descriptor, None
        receipt_hash = finish_output(output, owned_descriptor, receipt)
    except Exception:
        if descriptor is not None:
            failure = receipt_document(before, writers, args.assert_u04_never_opened, tool_hash,
                                       "FAILED_GROUP_REMAINS_QUARANTINED", parent_hash)
            owned_descriptor, descriptor = descriptor, None
            finish_output(output, owned_descriptor, failure)
        raise
    print(json.dumps({"status": receipt["status"], "receipt_sha256": receipt_hash, "targets": 3,
                      "source_databases_preserved": True, "roles_preserved": True}, sort_keys=True))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (RecreateError, OSError, ValueError) as error:
        raise SystemExit(str(error) if isinstance(error, RecreateError) else "private target recreation failed; retain the quarantined group")
