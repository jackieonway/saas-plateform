#!/usr/bin/env python3
"""Run U06 CLI against isolated databases initialized by frozen real migrators.

Only this run's uuid_u06_fixture_ databases and NOLOGIN owners are touched.
All business rows are invented fixtures. No source business database, service,
HTTP endpoint or credential policy is accessed. Evidence stays private.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import runpy
import secrets
import shutil
import signal
import stat
import subprocess
import sys
from urllib.parse import urlencode
import uuid

HERE = Path(__file__).resolve().parent
SUPPORT = runpy.run_path(str(HERE / "uuid-u06-data-fixture.py"))
Fixture = SUPPORT["Fixture"]
Rejected = SUPPORT["Rejected"]
require = SUPPORT["require"]
literal = SUPPORT["literal"]
identifier = SUPPORT["identifier"]
write_new = SUPPORT["write_new"]
KINDS = ("platform", "stores", "work_orders")
MIGRATORS = {
    "platform": ("/usr/local/libexec/saas-platform-migrate-u04", "c8fb69f437f059a682c681bd27f58e5b4d9eee6c9bd2dcf51effa77597827aa1", "DATABASE_URL", 12, 40),
    "stores": ("/usr/local/libexec/saas-stores-migrate-u04", "9d6a64285e0367ccb3e23f55c073c975758638e648394fc9ea25d4ce48b444dc", "STORES_DATABASE_URL", 3, 8),
    "work_orders": ("/usr/local/libexec/saas-orders-migrate-u04", "7d5af8de46af8a8fc3e56fb2f0fa26fb001f78700a97e4ac1bf4786ee3c3d2bf", "WORK_ORDERS_DATABASE_URL", 4, 6),
}
METADATA = {"platform": ["_sqlx_migrations"], "stores": ["_sqlx_migrations", "_stores_schema_contract", "_stores_schema_contract_v2", "_stores_schema_contract_v3"], "work_orders": ["_sqlx_migrations", "work_order_migrations"]}
CREDENTIALS = {"platform": "oauth_authorization_codes oauth_authorization_requests oauth_device_codes oauth_tokens platform_session_refresh_tokens platform_sessions".split(), "stores": [], "work_orders": []}
APPEND_ONLY = {"platform": "audit_logs application_release_artifacts notification_publications".split(), "stores": ["store_audit_events"], "work_orders": ["work_order_create_idempotency", "work_order_reminder_intents"]}

# These rows are fictional and deliberately make child IDs sort before parents.
SEED = {
    "platform": """
INSERT INTO public.tenants(id,name,code,created_at,updated_at) VALUES ('019a2a00-0000-7000-8000-000000000100','UUID fixture tenant','uuid-fixture-tenant','2026-10-24T00:00:00.123456Z','2026-10-24T00:00:00.123456Z');
INSERT INTO public.organizations(id,tenant_id,parent_id,name,code,organization_type,created_at,updated_at) VALUES ('019a2a00-0000-7000-8000-000000000002','019a2a00-0000-7000-8000-000000000100',NULL,'Fixture root','fixture-root','COMPANY','2026-10-24T00:00:00.123456Z','2026-10-24T00:00:00.123456Z');
INSERT INTO public.organizations(id,tenant_id,parent_id,name,code,organization_type,created_at,updated_at) VALUES ('019a2a00-0000-7000-8000-000000000001','019a2a00-0000-7000-8000-000000000100','019a2a00-0000-7000-8000-000000000002','Fixture store organization','fixture-store','STORE','2026-10-24T00:00:00.123456Z','2026-10-24T00:00:00.123456Z');
INSERT INTO public.departments(id,organization_id,parent_id,name,code,created_at,updated_at) VALUES ('019a2a00-0000-7000-8000-000000000012','019a2a00-0000-7000-8000-000000000001',NULL,'Fixture department root','fixture-dept-root','2026-10-24T00:00:00.123456Z','2026-10-24T00:00:00.123456Z');
INSERT INTO public.departments(id,organization_id,parent_id,name,code,created_at,updated_at) VALUES ('019a2a00-0000-7000-8000-000000000011','019a2a00-0000-7000-8000-000000000001','019a2a00-0000-7000-8000-000000000012','Fixture department child','fixture-dept-child','2026-10-24T00:00:00.123456Z','2026-10-24T00:00:00.123456Z');
INSERT INTO public.departments(id,organization_id,parent_id,name,code,created_at,updated_at) VALUES ('019a2a00-0000-7000-8000-000000000013','019a2a00-0000-7000-8000-000000000001','019a2a00-0000-7000-8000-000000000012','Fixture removable department','fixture-dept-delete','2026-10-24T00:00:00.123456Z','2026-10-24T00:00:00.123456Z');
INSERT INTO public.applications(id,code,name,base_path,entry_type,runtime_mode,created_at,updated_at) VALUES ('019a2a00-0000-7000-8000-000000000300','uuid-fixture','UUID fixture app','/micro/uuid-fixture','HTML','TRUSTED','2026-10-24T00:00:00.123456Z','2026-10-24T00:00:00.123456Z');
INSERT INTO public.application_releases(id,application_id,environment,version,entry_type,entry_url,resources,validation_status,validation_result,checked_at,created_at) VALUES ('019a2a00-0000-7000-8000-000000000310','019a2a00-0000-7000-8000-000000000300','fixture','1.0.0','HTML','https://fixture.invalid/1.0.0/','[]'::jsonb,'PASSED','{"fixture":true}'::jsonb,'2026-10-24T00:00:00.234567Z','2026-10-24T00:00:00.123456Z');
""",
    "stores": """
INSERT INTO public.stores(id,tenant_id,organization_id,owner_user_id,name,business_status,business_contact,version,created_at,updated_at) VALUES ('019a2a00-0000-7000-8000-000000000400','019a2a00-0000-7000-8000-000000000100','019a2a00-0000-7000-8000-000000000001','019a2a00-0000-7000-8000-000000000200','Fixture store historical name','OPEN','Fixture contact',1,'2026-10-24T00:00:00.123456Z','2026-10-24T00:00:00.123456Z');
INSERT INTO public.store_daily_reports(id,tenant_id,store_id,organization_id,owner_user_id,business_date,amount,note,version,created_at,updated_at) VALUES
('019a2a00-0000-7000-8000-000000000401','019a2a00-0000-7000-8000-000000000100','019a2a00-0000-7000-8000-000000000400','019a2a00-0000-7000-8000-000000000001','019a2a00-0000-7000-8000-000000000200','2026-10-22',9007199254740991.23,'Fixture keep/update',1,'2026-10-24T00:00:00.123456Z','2026-10-24T00:00:00.123456Z'),
('019a2a00-0000-7000-8000-000000000402','019a2a00-0000-7000-8000-000000000100','019a2a00-0000-7000-8000-000000000400','019a2a00-0000-7000-8000-000000000001','019a2a00-0000-7000-8000-000000000200','2026-10-23',0.01,'Fixture delete',1,'2026-10-24T00:00:00.123456Z','2026-10-24T00:00:00.123456Z');
INSERT INTO public.store_audit_events(id,tenant_id,organization_id,actor_user_id,resource_code,action_code,entity_id,request_id,created_at) VALUES ('019a2a00-0000-7000-8000-000000000403','019a2a00-0000-7000-8000-000000000100','019a2a00-0000-7000-8000-000000000001','019a2a00-0000-7000-8000-000000000200','store','create','019a2a00-0000-7000-8000-000000000400','019a2a00-0000-7000-8000-000000000490','2026-10-24T00:00:00.123456Z');
UPDATE public.stores SET business_contact='Fixture baseline profile 2',version=version+1 WHERE id='019a2a00-0000-7000-8000-000000000400';
UPDATE public.stores SET business_contact='Fixture baseline profile 3',version=version+1 WHERE id='019a2a00-0000-7000-8000-000000000400';
UPDATE public.store_daily_reports SET note='Fixture baseline report 2',version=version+1 WHERE id='019a2a00-0000-7000-8000-000000000401';
UPDATE public.store_daily_reports SET note='Fixture baseline report 3',version=version+1 WHERE id='019a2a00-0000-7000-8000-000000000401';
""",
    "work_orders": """
INSERT INTO public.work_order_counters(tenant_id,next_number) VALUES ('019a2a00-0000-7000-8000-000000000100',3);
INSERT INTO public.work_orders(id,number,tenant_id,organization_id,organization_name,owner_user_id,owner_display_name,title,description,priority,status,version,created_at,updated_at) VALUES
('019a2a00-0000-7000-8000-000000000500','WO-FIXTURE-1','019a2a00-0000-7000-8000-000000000100','019a2a00-0000-7000-8000-000000000001','Fixture store organization','019a2a00-0000-7000-8000-000000000200','Fixture owner','Fixture update ticket','Fixed fictional seed','NORMAL','OPEN',1,'2026-10-24T00:00:00.123456Z','2026-10-24T00:00:00.123456Z'),
('019a2a00-0000-7000-8000-000000000501','WO-FIXTURE-2','019a2a00-0000-7000-8000-000000000100','019a2a00-0000-7000-8000-000000000001','Fixture store organization','019a2a00-0000-7000-8000-000000000200','Fixture owner','Fixture physical delete ticket','No idempotency references','LOW','OPEN',1,'2026-10-24T00:00:00.123456Z','2026-10-24T00:00:00.123456Z');
INSERT INTO public.work_order_create_idempotency(tenant_id,organization_id,user_id,action,idempotency_key,request_hash,ticket_id,response_json,created_at) VALUES ('019a2a00-0000-7000-8000-000000000100','019a2a00-0000-7000-8000-000000000001','019a2a00-0000-7000-8000-000000000200','create','fixture:create:1',repeat('a',64),'019a2a00-0000-7000-8000-000000000500','{"id":"019a2a00-0000-7000-8000-000000000500","fixture":true}','2026-10-24T00:00:00.123456Z');
""",
}
DELTA = {
    "platform": """
UPDATE public.organizations SET name='Fixture store organization after',updated_at='2026-10-24T01:00:00.654321Z' WHERE id='019a2a00-0000-7000-8000-000000000001';
UPDATE public.departments SET name='Fixture department child after',updated_at='2026-10-24T01:00:00.654321Z' WHERE id='019a2a00-0000-7000-8000-000000000011';
DELETE FROM public.departments WHERE id='019a2a00-0000-7000-8000-000000000013';
INSERT INTO public.departments(id,organization_id,parent_id,name,code,created_at,updated_at) VALUES ('019a2a00-0000-7000-8000-000000000014','019a2a00-0000-7000-8000-000000000001','019a2a00-0000-7000-8000-000000000012','Fixture inserted department','fixture-dept-insert','2026-10-24T01:00:00.654321Z','2026-10-24T01:00:00.654321Z');
INSERT INTO public.application_releases(id,application_id,environment,version,entry_type,entry_url,resources,validation_status,validation_result,checked_at,created_at) VALUES ('019a2a00-0000-7000-8000-000000000311','019a2a00-0000-7000-8000-000000000300','fixture','1.0.1','HTML','https://fixture.invalid/1.0.1/','[]'::jsonb,'PASSED','{"fixture":true,"after":true}'::jsonb,'2026-10-24T01:00:00.765432Z','2026-10-24T01:00:00.654321Z');
""",
    "stores": """
UPDATE public.stores SET business_status='CLOSED',business_contact='Fixture after contact',version=version+1,updated_at='2026-10-24T01:00:00.654321Z' WHERE id='019a2a00-0000-7000-8000-000000000400';
UPDATE public.store_daily_reports SET amount=9007199254740992.34,note='Fixture updated precise amount',version=version+1,updated_at='2026-10-24T01:00:00.654321Z' WHERE id='019a2a00-0000-7000-8000-000000000401';
UPDATE public.stores SET business_contact='Fixture final profile 5',version=version+1,updated_at='2026-10-24T01:00:01.654321Z' WHERE id='019a2a00-0000-7000-8000-000000000400';
UPDATE public.store_daily_reports SET amount=9007199254740993.45,note='Fixture final precise report 5',version=version+1,updated_at='2026-10-24T01:00:01.654321Z' WHERE id='019a2a00-0000-7000-8000-000000000401';
DELETE FROM public.store_daily_reports WHERE id='019a2a00-0000-7000-8000-000000000402';
INSERT INTO public.store_daily_reports(id,tenant_id,store_id,organization_id,owner_user_id,business_date,amount,note,version,created_at,updated_at) VALUES ('019a2a00-0000-7000-8000-000000000404','019a2a00-0000-7000-8000-000000000100','019a2a00-0000-7000-8000-000000000400','019a2a00-0000-7000-8000-000000000001','019a2a00-0000-7000-8000-000000000200','2026-10-24',123.45,'Fixture inserted',1,'2026-10-24T01:00:00.654321Z','2026-10-24T01:00:00.654321Z');
INSERT INTO public.store_audit_events(id,tenant_id,organization_id,actor_user_id,resource_code,action_code,entity_id,request_id,created_at) VALUES ('019a2a00-0000-7000-8000-000000000405','019a2a00-0000-7000-8000-000000000100','019a2a00-0000-7000-8000-000000000001','019a2a00-0000-7000-8000-000000000200','store','update','019a2a00-0000-7000-8000-000000000400','019a2a00-0000-7000-8000-000000000491','2026-10-24T01:00:00.654321Z');
""",
    "work_orders": """
UPDATE public.work_orders SET title='Fixture after ticket',description='After image',priority='HIGH',status='IN_PROGRESS',version=version+1,updated_at='2026-10-24T01:00:00.654321Z' WHERE id='019a2a00-0000-7000-8000-000000000500';
DELETE FROM public.work_orders WHERE id='019a2a00-0000-7000-8000-000000000501';
INSERT INTO public.work_orders(id,number,tenant_id,organization_id,organization_name,owner_user_id,owner_display_name,title,description,priority,status,version,created_at,updated_at) VALUES ('019a2a00-0000-7000-8000-000000000502','WO-FIXTURE-3','019a2a00-0000-7000-8000-000000000100','019a2a00-0000-7000-8000-000000000001','Fixture store organization','019a2a00-0000-7000-8000-000000000200','Fixture owner','Fixture inserted ticket','New fictional ticket','NORMAL','OPEN',1,'2026-10-24T01:00:00.654321Z','2026-10-24T01:00:00.654321Z');
UPDATE public.work_order_counters SET next_number=4 WHERE tenant_id='019a2a00-0000-7000-8000-000000000100';
INSERT INTO public.work_order_create_idempotency(tenant_id,organization_id,user_id,action,idempotency_key,request_hash,ticket_id,response_json,created_at) VALUES ('019a2a00-0000-7000-8000-000000000100','019a2a00-0000-7000-8000-000000000001','019a2a00-0000-7000-8000-000000000200','create','fixture:create:3',repeat('b',64),'019a2a00-0000-7000-8000-000000000502','{"id":"019a2a00-0000-7000-8000-000000000502","fixture":true}','2026-10-24T01:00:00.654321Z');
""",
}


def canonical(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode()


def digest(value):
    return hashlib.sha256(canonical(value)).hexdigest()


def migrate(fixture, kind, database):
    fixture.stage = "REAL_MIGRATOR_" + kind.upper()
    raw, expected, key, count, tables = MIGRATORS[kind]
    binary = Path(raw)
    info = binary.stat()
    require(binary.is_file() and not binary.is_symlink() and info.st_uid == 0
            and not stat.S_IMODE(info.st_mode) & 0o022 and hashlib.sha256(binary.read_bytes()).hexdigest() == expected,
            "FROZEN_MIGRATOR_IDENTITY_CHANGED")
    owner = fixture.created[database]["owner"]
    fixture.sql(database, "BEGIN READ WRITE; ALTER SCHEMA public OWNER TO " + identifier(owner)
                + "; REVOKE ALL ON SCHEMA public FROM PUBLIC; COMMIT;")
    fixture.sql("postgres", "ALTER DATABASE " + identifier(database) + " SET default_transaction_read_only=off;", admin=True)
    url = "postgresql://postgres@localhost:5432/" + database + "?" + urlencode({"host": "/run/postgresql", "options[role]": owner, "sslmode": "disable"})
    env = {"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8", key: url}
    try:
        process = subprocess.Popen(["/usr/sbin/runuser", "-u", "postgres", "--", raw, "--import"],
                                   env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        try:
            out, err = process.communicate(timeout=170)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            out, err = process.communicate(timeout=10)
            write_new(fixture.output / (database + ".migrate.stdout.private.txt"), out)
            write_new(fixture.output / (database + ".migrate.stderr.private.txt"), err)
            raise Rejected("FROZEN_REAL_MIGRATOR_TIMEOUT") from None
        write_new(fixture.output / (database + ".migrate.stdout.private.txt"), out)
        write_new(fixture.output / (database + ".migrate.stderr.private.txt"), err)
        require(process.returncode == 0 and hashlib.sha256(binary.read_bytes()).hexdigest() == expected,
                "FROZEN_REAL_MIGRATOR_FAILED")
    finally:
        fixture.sql("postgres", "ALTER DATABASE " + identifier(database) + " SET default_transaction_read_only=on;", admin=True)
    history = fixture.json(database, "BEGIN READ ONLY; SELECT json_agg(version ORDER BY version) FROM public._sqlx_migrations; COMMIT;")
    require(history == list(range(1, count + 1)), "REAL_MIGRATION_HISTORY_MISMATCH")
    fixture.checks.append("real-migrator-" + kind + "-" + database.rsplit("_", 1)[1])


def tables(fixture, database):
    return fixture.json(database, "BEGIN READ ONLY; SELECT json_agg(c.relname ORDER BY c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind IN ('r','p'); COMMIT;")


def projection(fixture, database, kind, names):
    retained = [n for n in names if n not in METADATA[kind] + CREDENTIALS[kind]]
    parts = ["SELECT " + literal(n) + " name, COALESCE(json_agg(row_to_json(t)::text ORDER BY row_to_json(t)::text),'[]') rows FROM public." + identifier(n) + " t" for n in retained]
    return fixture.json(database, "BEGIN READ ONLY; SET LOCAL timezone='UTC'; SET LOCAL datestyle='ISO,YMD'; SELECT json_object_agg(name,rows) FROM (" + " UNION ALL ".join(parts) + ") all_rows; COMMIT;")


def tuple_projection(fixture, database, kind, names):
    # PostgreSQL tuple identity changes on UPDATE, even when values are equal.
    # Compare every retained platform row around resume to prove no such writes.
    retained = [n for n in names if n not in METADATA[kind] + CREDENTIALS[kind]]
    parts = ["SELECT " + literal(n) + " name, COALESCE(json_agg(json_build_object('row',row_to_json(t)::text,'xmin',t.xmin::text,'ctid',t.ctid::text) ORDER BY row_to_json(t)::text),'[]') rows FROM public." + identifier(n) + " t" for n in retained]
    return fixture.json(database, "BEGIN READ ONLY; SET LOCAL timezone='UTC'; SET LOCAL datestyle='ISO,YMD'; SELECT json_object_agg(name,rows) FROM (" + " UNION ALL ".join(parts) + ") all_rows; COMMIT;")


def versions(fixture, database):
    return fixture.json(database, "BEGIN READ ONLY; SELECT json_build_object('store',(SELECT version FROM public.stores WHERE id='019a2a00-0000-7000-8000-000000000400'),'report',(SELECT version FROM public.store_daily_reports WHERE id='019a2a00-0000-7000-8000-000000000401')); COMMIT;")


def sequence_projection(fixture, database):
    names = fixture.json(database, "BEGIN READ ONLY; SELECT COALESCE(json_agg(c.relname ORDER BY c.relname),'[]') FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind='S'; COMMIT;")
    if not names:
        return []
    parts = ["SELECT " + literal(n) + " name,last_value::text,is_called FROM public." + identifier(n) for n in names]
    return fixture.json(database, "BEGIN READ ONLY; SELECT json_agg(t ORDER BY name) FROM (" + " UNION ALL ".join(parts) + ") t; COMMIT;")


def run(fixture, engine, initialize_only=False):
    engine_sha = hashlib.sha256(engine.read_bytes()).hexdigest()
    semantic = runpy.run_path(str(engine))["semantic_material"]
    groups = {g: [] for g in ("source", "final", "recovery")}
    lookup = {}
    for group in groups:
        for kind, stem in zip(KINDS, ("p", "s", "w")):
            database = fixture.create(stem + "_" + group)
            migrate(fixture, kind, database)
            meta = fixture.created[database]
            groups[group].append({"kind": kind, "name": database, "oid": meta["oid"], "owner": meta["owner"], "owner_oid": meta["owner_oid"]})
            lookup[(group, kind)] = database
    names = {kind: tables(fixture, lookup[("source", kind)]) for kind in KINDS}
    require(all(len(names[k]) == MIGRATORS[k][4] for k in KINDS), "REAL_54_TABLE_COVERAGE_REQUIRED")
    contracts = {}
    schema_paths = {"platform": HERE.parents[2] / "src/database/schema_material.sql",
                    "stores": HERE.parents[4] / "saas-stores/backend/src/imported_schema.sql",
                    "work_orders": HERE.parents[4] / "saas-work-orders/backend/src/imported_schema.sql"}
    for kind in KINDS:
        schema = fixture.json(lookup[("source", kind)], "BEGIN READ ONLY; SET LOCAL search_path=pg_catalog; " + schema_paths[kind].read_text() + " COMMIT;")
        contracts[kind] = {"tables": names[kind], "schema_sha256": digest(semantic(schema)), "migration_count": MIGRATORS[kind][3], "metadata": METADATA[kind], "credentials": CREDENTIALS[kind], "append_only": APPEND_ONLY[kind]}
        require(SEED[kind].strip() and DELTA[kind].strip(), "REAL_FIXTURE_SEED_IMPLEMENTATION_REQUIRED")
        database = lookup[("source", kind)]
        fixture.sql(database, "BEGIN READ WRITE; SET LOCAL search_path=pg_catalog,public; " + SEED[kind] + " COMMIT;")
    fixture.checks.append("54-real-tables-seeded-with-invented-data")
    before_projection = {k: projection(fixture, lookup[("source", k)], k, names[k]) for k in KINDS}
    before_versions = versions(fixture, lookup[("source", "stores")])
    require(before_versions == {"store": 3, "report": 3}, "REAL_STORES_T0_VERSION_3_REQUIRED")
    if initialize_only:
        return {"engine_sha256": engine_sha, "tableCounts": {k: len(names[k]) for k in KINDS}, "initializeOnly": True}
    system_id = fixture.sql("postgres", "BEGIN READ ONLY; SELECT system_identifier FROM pg_control_system(); COMMIT;", admin=True).stdout.strip()
    registration = {"format": "saas-uuid-u06-registration/1", "system_identifier": system_id, "groups": groups,
                    "fenced_roles": {g: [] for g in groups}, "fixture_contracts": contracts}
    reg_path = fixture.output / "registration.private.json"
    write_new(reg_path, canonical(registration))
    codec = Path("/usr/local/libexec/saas-recovery-u06-codec")
    require(hashlib.sha256(codec.read_bytes()).hexdigest() == SUPPORT["NATIVE_CODEC_SHA"], "NATIVE_CODEC_IDENTITY_MISMATCH")
    key = fixture.output / "fixture.key"
    write_new(key, secrets.token_bytes(32))
    batch = str(uuid.uuid7())
    scratch = Path("/dev/shm/saas-uuid-u06-20261024") / batch
    cipher = Path("/var/lib/saas-uuid-u06-20261024") / batch
    for directory in (scratch, cipher):
        root = directory.parent
        require(root.is_dir() and not root.is_symlink() and root.stat().st_uid == 0
                and stat.S_IMODE(root.stat().st_mode) == 0o700, "PRIVATE_CODEC_ROOT_REQUIRED")
    cipher.mkdir(mode=0o700)
    def call(operation, label, group=None, package=False, reject=None, **inputs):
        require(hashlib.sha256(engine.read_bytes()).hexdigest() == engine_sha, "ENGINE_CHANGED_DURING_FIXTURE")
        fixture.stage = "ENGINE_" + label.upper().replace("-", "_")
        report = fixture.output / (label + ".report.private.json")
        config = {"format": "saas-uuid-u06-data/1", "profile": "uuid-u06-fixture", "registration": str(reg_path),
                  "scratch": str(scratch), "codec": {"binary": str(codec), "sha256": SUPPORT["NATIVE_CODEC_SHA"], "key_file": str(key)}, "report": str(report), **inputs}
        if group: config["group"] = group
        if package: config["output"] = {"path": str(cipher / (label + ".enc")), "purpose": "saas-uuid-preview-freeze/1/" + batch + "/u06-real-" + label}
        path = fixture.output / (label + ".config.private.json")
        write_new(path, canonical(config))
        process = subprocess.Popen([sys.executable, str(engine), "--" + operation, str(path)],
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
        try:
            out, err = process.communicate(timeout=240)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            out, err = process.communicate(timeout=10)
            write_new(fixture.output / (label + ".stdout.private.txt"), out)
            write_new(fixture.output / (label + ".stderr.private.txt"), err)
            raise Rejected("REAL_ENGINE_TIMEOUT") from None
        write_new(fixture.output / (label + ".stdout.private.txt"), out)
        write_new(fixture.output / (label + ".stderr.private.txt"), err)
        if reject:
            require(process.returncode != 0 and err.strip() == "U06_DATA_REJECTED:" + reject, "REAL_NEGATIVE_REJECTION_MISMATCH")
            fixture.checks.append(label + "-rejected")
            return None
        require(process.returncode == 0 and report.is_file(), "REAL_ENGINE_POSITIVE_FAILED")
        value = json.loads(report.read_bytes())
        fixture.checks.append(label)
        return {k: value["artifact"][k] for k in ("path", "purpose", "sha256")} if package else value
    scratch_owned = False
    try:
        scratch.mkdir(mode=0o700)
        scratch_owned = True
        before = call("snapshot", "snapshot-before", group="source", package=True)
        call("restore", "restore-final", group="final", input=before)
        call("verify", "verify-final-baseline", group="final", input=before)
        call("restore", "restore-recovery", group="recovery", input=before)
        for group in ("final", "recovery"):
            require({k: projection(fixture, lookup[(group, k)], k, names[k]) for k in KINDS} == before_projection, "REAL_BASELINE_PROJECTION_MISMATCH")
        fixture.checks.append("real-baseline-full-independent-projection")
        for kind in KINDS:
            fixture.sql(lookup[("source", kind)], "BEGIN READ WRITE; SET LOCAL search_path=pg_catalog,public; " + DELTA[kind] + " COMMIT;")
        after_versions = versions(fixture, lookup[("source", "stores")])
        require(after_versions == {"store": 5, "report": 5}, "REAL_STORES_T1_VERSION_5_REQUIRED")
        fixture.checks.append("stores-and-report-two-legal-patches-version-3-to-5")
        after_projection = {k: projection(fixture, lookup[("source", k)], k, names[k]) for k in KINDS}
        require(all(after_projection[k] != before_projection[k] for k in KINDS), "REAL_DELTA_FIXTURE_DID_NOT_CHANGE")
        after = call("snapshot", "snapshot-after", group="source", package=True)
        changes = call("delta", "delta", before=before, after=after, package=True)
        def recovery_projection():
            return {k: projection(fixture, lookup[("recovery", k)], k, names[k]) for k in KINDS}
        def platform_state():
            database = lookup[("recovery", "platform")]
            return {"logical": projection(fixture, database, "platform", names["platform"]),
                    "tuples": tuple_projection(fixture, database, "platform", names["platform"]),
                    "sequences": sequence_projection(fixture, database)}
        original_platform = platform_state()
        call("replay", "resume-uncommitted-platform", group="recovery", input=changes,
             resume_after_kinds=["platform"], reject="REPLAY_RESUME_TARGET_NOT_AT_T1")
        require(recovery_projection() == before_projection and platform_state() == original_platform,
                "UNCOMMITTED_RESUME_REJECTION_MUTATED_TARGET")
        recovery_stores = lookup[("recovery", "stores")]
        fixture.sql(recovery_stores, """BEGIN READ WRITE;
INSERT INTO public.store_daily_reports(id,tenant_id,store_id,organization_id,owner_user_id,business_date,amount,note,version,created_at,updated_at)
VALUES ('019a2a00-0000-7000-8000-000000000499','019a2a00-0000-7000-8000-000000000100','019a2a00-0000-7000-8000-000000000400','019a2a00-0000-7000-8000-000000000001','019a2a00-0000-7000-8000-000000000200','2026-10-25',0.01,'Fixture partial-commit conflict',1,'2026-10-24T00:00:00.123456Z','2026-10-24T00:00:00.123456Z');
COMMIT;""")
        conflict_stores = projection(fixture, recovery_stores, "stores", names["stores"])
        call("replay", "partial-commit-stores-conflict", group="recovery", input=changes,
             reject="TARGET_FULL_ROW_RECONCILIATION_FAILED")
        expected_partial = {"platform": after_projection["platform"], "stores": conflict_stores,
                            "work_orders": before_projection["work_orders"]}
        require(recovery_projection() == expected_partial, "REAL_PARTIAL_COMMIT_BOUNDARY_FAILED")
        fixture.checks.append("platform-committed-t1-before-stores-conflict")
        fixture.sql(recovery_stores, "BEGIN READ WRITE; DELETE FROM public.store_daily_reports WHERE id='019a2a00-0000-7000-8000-000000000499'; COMMIT;")
        expected_partial["stores"] = before_projection["stores"]
        require(recovery_projection() == expected_partial, "PARTIAL_CONFLICT_FIXTURE_RESET_FAILED")
        committed_platform = platform_state()
        for label, resume, rejection in [
            ("partial-ordinary-replay", None, "TARGET_FULL_ROW_RECONCILIATION_FAILED"),
            ("resume-nonprefix", ["stores"], "REPLAY_RESUME_PREFIX_INVALID"),
            ("resume-unsupported-extra-kind", ["platform", "stores"], "REPLAY_RESUME_PREFIX_INVALID"),
        ]:
            options = {} if resume is None else {"resume_after_kinds": resume}
            call("replay", label, group="recovery", input=changes, reject=rejection, **options)
            require(recovery_projection() == expected_partial and platform_state() == committed_platform,
                    "INVALID_RESUME_MUTATED_TARGET")
        resume_report = call("replay", "replay-recovery-resume-platform", group="recovery", input=changes,
                             resume_after_kinds=["platform"])
        platform_report = next(d for d in resume_report["databases"] if d["kind"] == "platform")
        require(platform_report["resumed_verified_t1"] is True and platform_report["business_dml_statements"] == 0
                and platform_report["committed"] is False, "RESUMED_PLATFORM_REPORT_NOT_READONLY")
        require(platform_state() == committed_platform, "RESUMED_PLATFORM_TUPLES_OR_SEQUENCES_CHANGED")
        require(versions(fixture, recovery_stores) == after_versions, "RESUMED_STORES_VERSION_5_REQUIRED")
        fixture.checks.append("resume-platform-t1-readonly-tuple-and-sequence-proof")
        call("verify", "verify-recovery", group="recovery", input=after)
        require({k: projection(fixture, lookup[("recovery", k)], k, names[k]) for k in KINDS} == after_projection, "REAL_AFTER_PROJECTION_MISMATCH")
        call("replay", "already-replayed", group="recovery", input=changes, reject="TARGET_FULL_ROW_RECONCILIATION_FAILED")
        require({k: projection(fixture, lookup[("recovery", k)], k, names[k]) for k in KINDS} == after_projection, "REAL_REPEAT_REJECTION_MUTATED_TARGET")
        fixture.checks.append("real-delta-full-independent-projection")
        require(hashlib.sha256(engine.read_bytes()).hexdigest() == engine_sha, "ENGINE_CHANGED_DURING_FIXTURE")
        return {"engine_sha256": engine_sha, "tableCounts": {k: len(names[k]) for k in KINDS},
                "seedProjectionSha256": digest(before_projection), "afterProjectionSha256": digest(after_projection),
                "storesVersionsBefore": before_versions, "storesVersionsAfter": after_versions,
                "partialCommitResume": "passed", "resumedPlatformTupleAndSequenceSha256": digest(committed_platform)}
    finally:
        if scratch_owned:
            require(scratch.parent == Path("/dev/shm/saas-uuid-u06-20261024") and scratch.name == batch and not scratch.is_symlink(), "SCRATCH_CLEANUP_BOUNDARY_REQUIRED")
            shutil.rmtree(scratch)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True)
    parser.add_argument("--engine", default=str(HERE / "uuid-u06-data.py"))
    parser.add_argument("--initialize-only", action="store_true")
    args = parser.parse_args()
    fixture = Fixture(SUPPORT["private_directory"](args.output))
    report = {"format": "saas-uuid-u06-real-schema-fixture/1", "status": "FAIL", "businessDatabasesAccessed": 0,
              "sourceBusinessRowsUsed": 0, "codec": "protected-native", "migrators": {k: MIGRATORS[k][1] for k in KINDS}}
    try:
        report.update(run(fixture, Path(args.engine), args.initialize_only))
        report["status"] = "INITIALIZE_ONLY" if args.initialize_only else "PASS"
    except Rejected as error:
        report["error"] = str(error)
    except Exception:
        report["error"] = "REAL_SCHEMA_FIXTURE_OPERATION_FAILED"
    finally:
        failures = fixture.cleanup()
        report.update({"stage": fixture.stage, "checks": fixture.checks, "sqlCalls": fixture.calls,
                       "fixtureDatabasesCreated": len(fixture.created), "fixtureOwnersCreated": len(fixture.roles), "cleanupFailures": failures})
        if failures:
            report["status"] = "FAIL"
            report["error"] = "REAL_SCHEMA_FIXTURE_CLEANUP_FAILED"
        write_new(fixture.output / "summary.private.json", json.dumps(report, indent=2))
    print(json.dumps(report, separators=(",", ":")))
    return 0 if report["status"] in ("PASS", "INITIALIZE_ONLY") else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Rejected as error:
        print(json.dumps({"status": "FAIL", "error": str(error)}), file=sys.stderr)
        raise SystemExit(1)
