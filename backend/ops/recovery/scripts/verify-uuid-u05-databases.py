#!/usr/bin/env python3
"""Audit fixed U04 targets after U05 writes; never mutate a database.

Default/--check only validates local inputs and prepares the SQL. --execute is
an explicit operator action after business/browser acceptance has finished.
Each database has its own bounded REPEATABLE READ READ ONLY transaction; these
are not claimed to be one coordinated cross-database snapshot.
"""

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
from urllib.parse import unquote, urlsplit


BACKEND = Path(__file__).resolve().parents[3]
WORKSPACE = BACKEND.parents[1]
VALIDATION = BACKEND / "docs/identity-sso/validation"
MATERIALS = Path("/var/lib/saas-uuid-u05-materials-20261024")
KINDS = ("platform", "stores", "work_orders")
TARGETS = {
    "platform": ("qiankun_platform_uuid7_u04_20261024", 107, 64, 12),
    "stores": ("qiankun_stores_uuid7_u04_20261024", 23, 2, 3),
    "work_orders": ("qiankun_orders_uuid7_u04_20261024", 17, 1, 4),
}
SOURCES = (
    "qiankun_platform_preview_20261024",
    "qiankun_stores_preview_20261024",
    "qiankun_orders_preview_20261024",
)
OLD_ROLES = ("qiankun_preview_20261024", "qiankun_notification_preview_20261024")
SCHEMA_FILES = {
    "platform": BACKEND / "src/database/schema_material.sql",
    "stores": WORKSPACE / "saas-stores/backend/src/imported_schema.sql",
    "work_orders": WORKSPACE / "saas-work-orders/backend/src/imported_schema.sql",
}
MIGRATION_DIRS = {
    "platform": BACKEND / "migrations",
    "stores": WORKSPACE / "saas-stores/backend/migrations",
    "work_orders": WORKSPACE / "saas-work-orders/backend/migrations",
}


class Invalid(Exception):
    """Only fixed, non-sensitive audit categories may reach stderr."""


def require(condition, code):
    if not condition:
        raise Invalid(code)


def unique(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "DUPLICATE_JSON_KEY")
        result[key] = value
    return result


def parsed(raw):
    return json.loads(raw, object_pairs_hook=unique,
                      parse_constant=lambda _: (_ for _ in ()).throw(Invalid("INVALID_JSON_NUMBER")))


def canonical(value):
    # serde_json::Value uses sorted object keys; preserve array order, every
    # original field and internal OID-derived names. No lossy normalization.
    return json.dumps(value, sort_keys=True, ensure_ascii=False,
                      separators=(",", ":"), allow_nan=False).encode("utf-8")


def digest(value):
    return hashlib.sha256(canonical(value)).hexdigest()


def private_json(path):
    require(os.name == "posix" and os.geteuid() == 0, "WSL_ROOT_REQUIRED")
    require(path.is_absolute() and all(not p.is_symlink() for p in (path, *path.parents)), "PRIVATE_PATH_INVALID")
    info = path.stat()
    require(stat.S_ISREG(info.st_mode) and info.st_uid == 0 and info.st_nlink == 1
            and stat.S_IMODE(info.st_mode) == 0o600 and info.st_size <= 4 * 1024 * 1024,
            "PRIVATE_FILE_INVALID")
    return parsed(path.read_text(encoding="utf-8-sig"))


def literal(value):
    require(isinstance(value, str) and "\x00" not in value, "SQL_LITERAL_INVALID")
    return "'" + value.replace("'", "''") + "'"


def psql(database, queries):
    require(database in (*SOURCES, *(v[0] for v in TARGETS.values()), "postgres"), "FIXED_DATABASE_REQUIRED")
    env = {k: v for k, v in os.environ.items() if not k.startswith("PG")}
    env.update({"LC_ALL": "C.UTF-8", "PGPASSFILE": "/dev/null", "PGCONNECT_TIMEOUT": "10"})
    sql = """BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL search_path=pg_catalog;
SET LOCAL timezone='UTC';
SET LOCAL row_security=off;
SET LOCAL statement_timeout='120s';
SET LOCAL lock_timeout='3s';
""" + "\n".join(queries) + "\nROLLBACK;\n"
    result = subprocess.run(
        ["/usr/sbin/runuser", "-u", "postgres", "--", "/usr/bin/psql", "-X", "-qAt",
         "--no-password", "-v", "ON_ERROR_STOP=1", "-h", "/run/postgresql", "-p", "5432",
         "-U", "postgres", "-d", database, "--file=-"],
        input=sql, text=True, capture_output=True, env=env, timeout=150,
    )
    require(result.returncode == 0, "READ_ONLY_QUERY_FAILED")
    require(len(result.stdout.encode()) <= 16 * 1024 * 1024, "AUDIT_RESULT_TOO_LARGE")
    lines = result.stdout.splitlines()
    require(len(lines) == len(queries), "AUDIT_RESULT_FRAMING_INVALID")
    return [parsed(line) for line in lines]


def source_gate_sql():
    names = ",".join(map(literal, SOURCES))
    roles = ",".join(map(literal, OLD_ROLES))
    return f"""SELECT jsonb_build_object(
 'identity_valid',current_user='postgres' AND session_user='postgres' AND inet_server_addr() IS NULL,
 'transaction_read_only',current_setting('transaction_read_only')='on',
 'isolation',current_setting('transaction_isolation'),
 'source_roles',(SELECT jsonb_agg(jsonb_build_object('name',rolname,'login',rolcanlogin,'superuser',rolsuper) ORDER BY rolname)
   FROM pg_roles WHERE rolname IN ({roles})),
 'sources',(SELECT jsonb_agg(jsonb_build_object('database',d.datname,'default_read_only',EXISTS(
   SELECT 1 FROM pg_db_role_setting s WHERE s.setdatabase=d.oid AND s.setrole=0
   AND s.setconfig @> ARRAY['default_transaction_read_only=on']),
   'active_backends',(SELECT count(*) FROM pg_stat_activity a WHERE a.datid=d.oid),
   'prepared_transactions',(SELECT count(*) FROM pg_prepared_xacts p WHERE p.database=d.datname)) ORDER BY d.datname)
   FROM pg_database d WHERE d.datname IN ({names})));"""


def audit_sql(kind, roles):
    inventory = (BACKEND / "ops/recovery/src/uuid_inventory/catalog.rs").read_text()
    # Reuse the already reviewed U01 recursive scalar/domain/array expansion,
    # including format(%I) identifiers and ONLY semantics for ordinary tables.
    prefix = "WITH RECURSIVE\n" + inventory.split("WITH RECURSIVE\n", 1)[1].split("table_counts AS MATERIALIZED (", 1)[0]
    specs = ",".join("(" + literal(k) + "," + literal(v) + ")" for k, v in roles.items())
    tasks = "'{}'::jsonb"
    if kind == "platform":
        tasks = "jsonb_build_object(" + ",".join(
            literal(t) + ",(SELECT count(*) FROM ONLY public." + t + ")"
            for t in ("application_release_operations", "managed_deployment_jobs", "zip_distribution_jobs", "zip_distribution_nodes")) + ")"
    return prefix + f"""
uuid_counts AS MATERIALIZED (
 SELECT c.*,x.payload::jsonb AS counts FROM uuid_columns c
 CROSS JOIN LATERAL XMLTABLE('/table/row' PASSING query_to_xml(format($scan$
   WITH vals AS MATERIALIZED (%s)
   SELECT jsonb_build_object('non_null',count(*) FILTER(WHERE value IS NOT NULL),
    'invalid',count(*) FILTER(WHERE value IS NOT NULL AND
      ((get_byte(uuid_send(value),6)>>4)<>7 OR (get_byte(uuid_send(value),8)&192)<>128))) AS payload
   FROM vals
 $scan$,c.value_query),true,false,'') COLUMNS payload text PATH 'payload') x
), db AS (SELECT * FROM pg_database WHERE datname=current_database()),
protected_roles AS (
 SELECT datdba AS oid FROM db UNION SELECT nspowner FROM pg_namespace WHERE oid IN (SELECT oid FROM user_schemas)
 UNION SELECT relowner FROM pg_class WHERE relnamespace IN (SELECT oid FROM user_schemas)
 UNION SELECT proowner FROM pg_proc WHERE pronamespace IN (SELECT oid FROM user_schemas)
 UNION SELECT typowner FROM pg_type WHERE typnamespace IN (SELECT oid FROM user_schemas)
),
role_specs(service,role_name) AS (VALUES {specs}),
role_matrix AS (
 SELECT spec.service,jsonb_build_object('service',spec.service,'exists',r.oid IS NOT NULL,
  'login',r.rolcanlogin,'unprivileged',NOT(r.rolsuper OR r.rolcreatedb OR r.rolcreaterole OR r.rolreplication OR r.rolbypassrls),
  'connect',has_database_privilege(r.oid,db.oid,'CONNECT'),
  'per_database_read_only',(SELECT split_part(v,'=',2) FROM pg_db_role_setting s CROSS JOIN LATERAL unnest(s.setconfig) v
    WHERE s.setdatabase=db.oid AND s.setrole=r.oid AND v LIKE 'default_transaction_read_only=%'),
  'configured_read_only',(SELECT split_part(v,'=',2) FROM pg_db_role_setting s CROSS JOIN LATERAL unnest(s.setconfig) v
    WHERE ((s.setdatabase=db.oid AND s.setrole IN (0,r.oid)) OR (s.setdatabase=0 AND s.setrole=r.oid))
    AND v LIKE 'default_transaction_read_only=%'
    ORDER BY (s.setrole=r.oid) DESC,(s.setdatabase=db.oid) DESC LIMIT 1),
  'select_tables',(SELECT count(*) FROM tables c
    WHERE c.schema_name='public' AND has_table_privilege(r.oid,c.oid,'SELECT')),
  'writable_tables',(SELECT count(*) FROM tables c
    WHERE c.schema_name='public' AND has_table_privilege(r.oid,c.oid,'INSERT,UPDATE,DELETE,TRUNCATE')),
  'full_dml_tables',(SELECT count(*) FROM tables c
    WHERE c.schema_name='public' AND has_table_privilege(r.oid,c.oid,'INSERT')
    AND has_table_privilege(r.oid,c.oid,'UPDATE') AND has_table_privilege(r.oid,c.oid,'DELETE')),
  'ddl_or_owner_reachable',
    EXISTS(SELECT 1 FROM protected_roles p WHERE pg_has_role(r.oid,p.oid,'SET') OR pg_has_role(r.oid,p.oid,'USAGE'))
    OR EXISTS(SELECT 1 FROM pg_auth_members m WHERE m.admin_option
      AND (pg_has_role(r.oid,m.member,'SET') OR pg_has_role(r.oid,m.member,'USAGE')))
    OR EXISTS(SELECT 1 FROM pg_roles p WHERE (pg_has_role(r.oid,p.oid,'SET') OR pg_has_role(r.oid,p.oid,'USAGE'))
      AND (p.rolsuper OR p.rolcreatedb OR p.rolcreaterole OR p.rolreplication OR p.rolbypassrls
        OR has_database_privilege(p.oid,db.oid,'CREATE,TEMPORARY')
        OR EXISTS(SELECT 1 FROM pg_namespace n WHERE n.nspname !~ '^pg_(toast_)?temp_'
          AND has_schema_privilege(p.oid,n.oid,'CREATE'))))
 ) AS proof FROM role_specs spec LEFT JOIN pg_roles r ON r.rolname=spec.role_name CROSS JOIN db
)
SELECT jsonb_build_object(
 'database',current_database(),'database_oid',db.oid::bigint,'owner_oid',db.datdba::bigint,
 'identity_valid',current_user='postgres' AND session_user='postgres' AND inet_server_addr() IS NULL,
 'transaction_read_only',current_setting('transaction_read_only')='on',
 'isolation',current_setting('transaction_isolation'),'server_version_num',current_setting('server_version_num')::int,
 'default_read_only',EXISTS(SELECT 1 FROM pg_db_role_setting s WHERE s.setdatabase=db.oid AND s.setrole=0
    AND s.setconfig @> ARRAY['default_transaction_read_only=on']),
 'owner_nologin',NOT owner.rolcanlogin,'owner_non_superuser',NOT owner.rolsuper,
 'marker',shobj_description(db.oid,'pg_database'),
 'uuid_columns',COALESCE((SELECT jsonb_agg(jsonb_build_object('schema',schema_name,'table',table_name,
    'column',column_name,'array_layers',array_layers,'domains',domains,'has_default',default_expression IS NOT NULL,
    'non_null',counts->'non_null','invalid',counts->'invalid') ORDER BY schema_name,table_name,attnum) FROM uuid_counts),'[]'),
 'foreign_keys',(SELECT count(*) FROM pg_constraint c JOIN pg_namespace n ON n.oid=c.connamespace WHERE n.nspname='public' AND c.contype='f'),
 'unvalidated_foreign_keys',(SELECT count(*) FROM pg_constraint c JOIN pg_namespace n ON n.oid=c.connamespace WHERE n.nspname='public' AND c.contype='f' AND NOT c.convalidated),
 'disabled_triggers',(SELECT count(*) FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND t.tgenabled<>'O'),
 'history',(SELECT jsonb_agg(jsonb_build_object('version',version,'success',success,'checksum',encode(checksum,'hex')) ORDER BY version) FROM public._sqlx_migrations),
 'runtime_roles',(SELECT jsonb_agg(proof ORDER BY service) FROM role_matrix),
 'task_rows',{tasks}
) FROM db JOIN pg_roles owner ON owner.oid=db.datdba;"""


def load_public_inputs():
    summary_path = VALIDATION / "uuid-u04-summary.json"
    startup_path = VALIDATION / "uuid-u05-startup-diagnostic.json"
    summary = parsed(summary_path.read_text())
    startup = parsed(startup_path.read_text())
    require(len(startup["cases"]) == 9 and all(c["receipt_schema"] == "PASS" for c in startup["cases"] if c["case"] == "correct"), "STARTUP_BASELINE_INVALID")
    expected = {v["kind"]: v for v in summary["current_targets"]}
    require(set(expected) == set(KINDS), "U04_TARGET_SET_INVALID")
    queries = {kind: path.read_text() for kind, path in SCHEMA_FILES.items()}
    require(len(set(queries.values())) == 1, "SERVICE_SCHEMA_QUERY_DISAGREEMENT")
    require(digest([{"b": 2, "a": 1}, {"b": 4, "a": 3}]) == "db1ae46d919a4bf5eb460249076679903197cf01faf6dbd1ecd3d849ff7c8b45", "TYPED_SCHEMA_CANONICALIZATION_INVALID")
    for kind, (name, columns, fks, history) in TARGETS.items():
        e = expected[kind]
        require(e["database"] == name and e["uuid_columns"] == columns and e["foreign_keys"] == fks
                and e["migrations"] == history and re.fullmatch(r"[0-9a-f]{64}", e["schema_sha256"]), "U04_BASELINE_INVALID")
    return expected, queries, {
        "u04_summary_sha256": hashlib.sha256(summary_path.read_bytes()).hexdigest(),
        "startup_diagnostic_sha256": hashlib.sha256(startup_path.read_bytes()).hexdigest(),
        "schema_query_sha256": hashlib.sha256(next(iter(queries.values())).encode()).hexdigest(),
    }


def runtime_roles(expected):
    receipt = private_json(MATERIALS / "imported-receipts.private.json")
    roles = {}
    def input_role(service, key, database):
        values = private_json(MATERIALS / (service + "-input.private.json"))
        url = urlsplit(values[key])
        require(url.scheme in ("postgres", "postgresql") and url.hostname == "127.0.0.1"
                and url.port == 5432 and unquote(url.path) == "/" + database
                and not url.query and not url.fragment and bool(url.password), "RUNTIME_DATABASE_URL_INVALID")
        return unquote(url.username or "")
    database_keys = {"platform": "DATABASE_URL", "stores": "STORES_DATABASE_URL", "work_orders": "WORK_ORDERS_DATABASE_URL"}
    for kind in KINDS:
        r = receipt["targets"][kind]
        require(r["database"] == expected[kind]["database"] and r["schema_sha256"] == expected[kind]["schema_sha256"]
                and r["import_marker"] == expected[kind]["marker"], "PRIVATE_RECEIPT_MISMATCH")
        roles[kind] = r["runtime_role"]
        require(input_role(kind, database_keys[kind], expected[kind]["database"]) == roles[kind], "RUNTIME_ROLE_RECEIPT_MISMATCH")
    roles["notification"] = input_role("notification", "DATABASE_URL", TARGETS["platform"][0])
    require(len(set(roles.values())) == 4 and all(re.fullmatch(r"qiankun_[a-z0-9_]*_u05_[a-z0-9_]*20261024", r)
            and "owner" not in r for r in roles.values()), "RUNTIME_ROLE_SET_INVALID")
    return roles


def check_history(kind, history):
    count = TARGETS[kind][3]
    require(isinstance(history, list) and [r["version"] for r in history] == list(range(1, count + 1))
            and all(r["success"] is True for r in history), "MIGRATION_HISTORY_INVALID")
    for row in history:
        files = list(MIGRATION_DIRS[kind].glob(f"{row['version']:04}_*.sql"))
        require(len(files) == 1, "MIGRATION_SOURCE_AMBIGUOUS")
        allowed = {hashlib.sha384(files[0].read_bytes()).hexdigest()}
        if kind == "platform" and row["version"] == 1:
            declaration = (BACKEND / "src/database.rs").read_text().split("const LEGACY_INITIAL_CHECKSUMS:", 1)[1].split("];", 1)[0]
            allowed.update(re.findall(r'"([0-9a-f]{96})"', declaration))
        require(row["checksum"] in allowed, "MIGRATION_CHECKSUM_MISMATCH")


def check_target(kind, observed, material, expected):
    name, columns, fks, count = TARGETS[kind]
    e = expected[kind]
    require(observed["database"] == name and observed["database_oid"] == e["database_oid"]
            and observed["owner_oid"] == e["owner_oid"] and observed["marker"] == e["marker"], "TARGET_IDENTITY_OR_MARKER_MISMATCH")
    require(observed["identity_valid"] is True and observed["transaction_read_only"] is True
            and observed["isolation"] == "repeatable read" and observed["server_version_num"] == 180006,
            "READ_ONLY_LOCAL_TRANSACTION_INVALID")
    require(observed["default_read_only"] is True and observed["owner_nologin"] is True
            and observed["owner_non_superuser"] is True, "TARGET_DEFAULT_OR_OWNER_CHANGED")
    require(isinstance(material, list) and all(isinstance(v, dict) for v in material), "TYPED_SCHEMA_ARRAY_INVALID")
    schema_digest = digest(material)
    require(schema_digest == e["schema_sha256"], "RAW_SCHEMA_HASH_MISMATCH")
    scans = observed["uuid_columns"]
    require(len(scans) == columns and len({(s["schema"], s["table"], s["column"]) for s in scans}) == columns,
            "UUID_COLUMN_SET_INVALID")
    require(all(s["schema"] == "public" and s["invalid"] == 0 and s["has_default"] is False for s in scans), "UUID_VERSION_VARIANT_OR_DEFAULT_INVALID")
    require(observed["foreign_keys"] == fks and observed["unvalidated_foreign_keys"] == 0
            and observed["disabled_triggers"] == 0, "FOREIGN_KEY_OR_TRIGGER_CHANGED")
    check_history(kind, observed["history"])
    require(len(observed["runtime_roles"]) == 4
            and {r["service"] for r in observed["runtime_roles"]} == {*KINDS, "notification"},
            "RUNTIME_ROLE_MATRIX_INCOMPLETE")
    for role in observed["runtime_roles"]:
        require(role["exists"] is True and role["login"] is True and role["unprivileged"] is True, "RUNTIME_PRINCIPAL_INVALID")
        owner = role["service"] == kind
        reader = kind == "platform" and role["service"] == "notification"
        require(role["connect"] is (owner or reader) and role["ddl_or_owner_reachable"] is False, "RUNTIME_ROLE_TARGET_OR_DDL_INVALID")
        if owner:
            require(role["per_database_read_only"] == "off" and role["configured_read_only"] == "off"
                    and role["full_dml_tables"] > 0, "BUSINESS_ROLE_NOT_EXPLICITLY_WRITABLE")
        else:
            require(role["writable_tables"] == 0, "UNEXPECTED_CROSS_DATABASE_WRITER")
        if reader:
            require(role["configured_read_only"] == "on" and role["select_tables"] > 0, "NOTIFICATION_NOT_READ_ONLY")
    require(all(value == 0 for value in observed["task_rows"].values()), "EXTERNAL_TASK_EFFECTS_REQUIRE_RECONCILIATION")
    return {
        "service": kind, "database": name, "database_oid_matches_u04": True,
        "owner_oid_matches_u04": True, "owner_nologin": True, "owner_non_superuser": True,
        "database_default_read_only": True, "import_marker_matches_u04": True,
        "raw_schema_sha256": schema_digest, "raw_schema_items": len(material),
        "uuid_columns": len(scans), "uuid_array_columns": sum(s["array_layers"] > 0 for s in scans),
        "uuid_domain_columns": sum(bool(s["domains"]) for s in scans),
        "nonnull_uuid_values": sum(s["non_null"] for s in scans), "invalid_uuid_values": 0,
        "uuid_defaults": 0, "uuid_column_scans": scans, "foreign_keys": fks,
        "unvalidated_foreign_keys": 0, "disabled_triggers": 0, "migration_count": count,
        "migration_versions": [r["version"] for r in observed["history"]],
        "migration_checksums_match_formal_source_or_exact_legacy_allowlist": True,
        "migration_history_sha256": digest(observed["history"]),
        "runtime_role_matrix": observed["runtime_roles"], "task_rows": observed["task_rows"],
        "transaction": "REPEATABLE READ READ ONLY", "search_path": "pg_catalog",
        "server_version_num": observed["server_version_num"],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    action = parser.add_mutually_exclusive_group()
    action.add_argument("--check", action="store_true", help="static preparation only (default)")
    action.add_argument("--execute", action="store_true", help="run only after root ends U05 business/browser writes")
    parser.add_argument("--output", type=Path, default=VALIDATION / "uuid-u05-database-audit.json")
    args = parser.parse_args()
    expected, schemas, bindings = load_public_inputs()
    # Exercise plan construction without private inputs, PostgreSQL or writes.
    sample_roles = {k: "qiankun_" + k + "_u05_runtime_20261024" for k in (*KINDS, "notification")}
    for kind in KINDS:
        require("uuid_counts AS MATERIALIZED" in audit_sql(kind, sample_roles), "UUID_SCAN_PLAN_INVALID")
    if not args.execute:
        print(json.dumps({"status": "STATIC_READY_NO_DATABASE_CONNECTION", "uuid_columns": 147,
                          "foreign_keys": 67, "migration_counts": [12, 3, 4], **bindings}, separators=(",", ":")))
        return
    require(os.name == "posix" and os.geteuid() == 0, "WSL_ROOT_REQUIRED")
    require(args.output.parent.resolve() == VALIDATION.resolve() and not args.output.exists()
            and not args.output.is_symlink(), "FRESH_VALIDATION_OUTPUT_REQUIRED")
    roles = runtime_roles(expected)
    source = psql("postgres", [source_gate_sql()])[0]
    require(source["identity_valid"] is True and source["transaction_read_only"] is True
            and source["isolation"] == "repeatable read", "SOURCE_GATE_TRANSACTION_INVALID")
    require(len(source["source_roles"]) == 2 and all(not r["login"] and not r["superuser"] for r in source["source_roles"]), "OLD_ROLE_FENCE_CHANGED")
    require(len(source["sources"]) == 3 and all(d["default_read_only"] is True and d["active_backends"] == 0
            and d["prepared_transactions"] == 0 for d in source["sources"]), "SOURCE_FENCE_CHANGED")
    reports = []
    for kind in KINDS:
        observed, material = psql(TARGETS[kind][0], [audit_sql(kind, roles), schemas[kind]])
        reports.append(check_target(kind, observed, material, expected))
    report = {
        "format": "saas-uuid-u05-database-audit/1", "status": "PASS",
        "recorded_at_utc": datetime.now(timezone.utc).isoformat(),
        "scope": "Fixed local U04 targets after U05 business/browser writes; source fences observed through maintenance catalog",
        "method": "runuser postgres; psql -X with no inherited PG settings; four bounded REPEATABLE READ READ ONLY transactions; ROLLBACK",
        "database_writes": 0, "business_uuid_or_row_values_disclosed": False,
        "inputs": bindings, "source_fences": source, "targets": reports,
        "totals": {"uuid_columns": sum(t["uuid_columns"] for t in reports),
                   "nonnull_uuid_values": sum(t["nonnull_uuid_values"] for t in reports),
                   "invalid_uuid_values": 0, "foreign_keys": sum(t["foreign_keys"] for t in reports),
                   "unvalidated_foreign_keys": 0, "disabled_triggers": 0},
        "limitations": [
            "Each database has a separate read-only snapshot, not a coordinated cross-database snapshot.",
            "Database defaults remain read-only, while the three dedicated business LOGIN roles intentionally have per-database read_only=off and effective DML. Targets are not claimed to reject every writer.",
            "Runtime role permissions and configured defaults are catalog observations; no test write is made by this script.",
            "Schema digests preserve the complete ordered service JSON material, including internal trigger names; they do not include ACLs, which are observed separately in the role matrix.",
            "Task counts of zero establish the database task-table observation; external agent effects are not contacted or inferred.",
            "The source gate repeats the named old-role/default-read-only/idle/prepared checks only; it does not claim to repeat the complete U03 export admission observer.",
            "This audit alone does not prove browser behavior, credential rejection or completion of U05/U06."
        ],
    }
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(args.output, flags, 0o644)
    with os.fdopen(descriptor, "wb") as stream:
        stream.write(json.dumps(report, ensure_ascii=False, indent=2).encode() + b"\n")
        stream.flush()
        os.fsync(stream.fileno())
    print(json.dumps({"status": "PASS", "totals": report["totals"],
                      "output_sha256": hashlib.sha256(args.output.read_bytes()).hexdigest()}, separators=(",", ":")))


if __name__ == "__main__":
    try:
        main()
    except Invalid as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
    except Exception:
        print("DATABASE_AUDIT_PREPARATION_OR_EXECUTION_FAILED", file=sys.stderr)
        sys.exit(1)
