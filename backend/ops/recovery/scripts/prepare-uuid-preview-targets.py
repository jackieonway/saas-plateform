#!/usr/bin/env python3
"""Create only the fixed local UUID-U04 rehearsal targets, never a source DB.

Default: read-only preflight. --execute creates fresh NOLOGIN owners and empty
databases. After the service-owned migrate-only tools succeed, --seal makes the
targets persistently read-only for the controlled offline importer. This script
contains no drop, truncate, password, seed, worker or service-start operation.
"""
import argparse
import json
import os
import subprocess

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


def sql(statement):
    # Never inherit libpq URLs, passwords, service files or startup options.
    env = {key: value for key, value in os.environ.items() if not key.startswith("PG")}
    env.update({"LC_ALL": "C", "PGCONNECT_TIMEOUT": "10"})
    result = subprocess.run(
        ["/usr/bin/psql", "-X", "-Atq", "-v", "ON_ERROR_STOP=1", "-h",
         "/run/postgresql", "-p", "5432", "-U", "postgres", "-d", "postgres"],
        input=statement, capture_output=True, text=True, timeout=30, env=env,
    )
    if result.returncode:
        raise RuntimeError("fixed local target operation failed; retain and inspect the quarantined targets")
    return result.stdout.strip()


def literal(value):
    assert value.replace("_", "").isalnum() and value.isascii()
    return "'" + value + "'"


def values(items):
    return ",".join(literal(item) for item in items)


def preflight(require_targets=False):
    names = [target[1] for target in TARGETS]
    owners = [target[2] for target in TARGETS]
    if set(names) & set(SOURCES) or len(set(names + owners)) != 6:
        raise RuntimeError("target/source identity collision")
    proof = json.loads(sql(f"""
BEGIN READ ONLY;
SELECT json_build_object(
 'actor',current_user,'session_actor',session_user,
 'local_socket',inet_server_addr() IS NULL,
 'port',current_setting('port')::int,
 'server_version_num',current_setting('server_version_num')::int,
 'source_roles',(SELECT json_agg(json_build_object('name',rolname,'login',rolcanlogin,'super',rolsuper))
     FROM pg_roles WHERE rolname IN ({values(SOURCE_ROLES)})),
 'sources',(SELECT json_agg(json_build_object('name',d.datname,'readonly',EXISTS(
     SELECT FROM pg_db_role_setting s WHERE s.setdatabase=d.oid AND s.setrole=0
       AND 'default_transaction_read_only=on'=ANY(s.setconfig)),
     'backends',(SELECT count(*) FROM pg_stat_activity a WHERE a.datid=d.oid),
     'prepared',(SELECT count(*) FROM pg_prepared_xacts p WHERE p.database=d.datname)))
     FROM pg_database d WHERE d.datname IN ({values(SOURCES)})),
 'target_roles',(SELECT coalesce(json_agg(json_build_object('name',rolname,'login',rolcanlogin,'super',rolsuper)), '[]')
     FROM pg_roles WHERE rolname IN ({values(owners)})),
 'targets',(SELECT coalesce(json_agg(json_build_object('name',d.datname,'owner',r.rolname,
     'backends',(SELECT count(*) FROM pg_stat_activity a WHERE a.datid=d.oid),
     'prepared',(SELECT count(*) FROM pg_prepared_xacts p WHERE p.database=d.datname))), '[]')
     FROM pg_database d JOIN pg_roles r ON r.oid=d.datdba WHERE d.datname IN ({values(names)})));
COMMIT;
"""))
    if (proof["actor"] != "postgres" or proof["session_actor"] != "postgres"
            or not proof["local_socket"] or proof["port"] != 5432
            or proof["server_version_num"] < 170000):
        raise RuntimeError("fixed local postgres peer maintenance identity required")
    if (len(proof["source_roles"] or []) != 2 or any(
            role["login"] or role["super"] for role in proof["source_roles"])):
        raise RuntimeError("old source roles must remain NOLOGIN and non-superuser")
    if len(proof["sources"] or []) != 3 or any(
            not item["readonly"] or item["backends"] or item["prepared"]
            for item in proof["sources"]):
        raise RuntimeError("three source databases must remain fenced and idle")
    if not require_targets:
        if proof["target_roles"] or proof["targets"]:
            raise RuntimeError("fresh target names and owners required; existing objects are never reused")
    else:
        expected = {name: owner for _, name, owner in TARGETS}
        if (len(proof["targets"]) != 3 or len(proof["target_roles"]) != 3
                or any(role["login"] or role["super"] for role in proof["target_roles"])
                or any(item["owner"] != expected.get(item["name"])
                       or item["backends"] or item["prepared"] for item in proof["targets"])):
            raise RuntimeError("three exact idle targets with NOLOGIN owners required")
    return proof


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_mutually_exclusive_group()
    actions.add_argument("--execute", action="store_true")
    actions.add_argument("--seal", action="store_true")
    args = parser.parse_args()
    preflight(require_targets=args.seal)
    if args.execute:
        for _, name, owner in TARGETS:
            sql(f'CREATE ROLE "{owner}" NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;')
            # CREATE DATABASE cannot run inside an explicit transaction.
            sql(f'CREATE DATABASE "{name}" OWNER "{owner}" TEMPLATE template0 ENCODING \'UTF8\';')
            sql(f'REVOKE ALL ON DATABASE "{name}" FROM PUBLIC;')
        preflight(require_targets=True)
    elif args.seal:
        for _, name, _ in TARGETS:
            sql(f'ALTER DATABASE "{name}" SET default_transaction_read_only=on;')
        preflight(require_targets=True)
    print(json.dumps({"status": "TARGETS_READ_ONLY" if args.seal else
                      "FRESH_TARGETS_CREATED" if args.execute else "READ_ONLY_PREFLIGHT_PASSED",
                      "source_databases_preserved": True,
                      "targets": [{"kind": kind, "name": name, "owner": owner}
                                  for kind, name, owner in TARGETS]}, sort_keys=True))


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, subprocess.TimeoutExpired, json.JSONDecodeError) as error:
        raise SystemExit(str(error))
