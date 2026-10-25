#!/usr/bin/env python3
"""Isolated real-PostgreSQL fixtures for the UUID-U06 data engine.

Only databases created by this run, named uuid_u06_fixture_<random>_<case>, are
accessible to fixture SQL and eligible for cleanup. Business databases, roles,
services and HTTP endpoints are never accessed. Output is a new private folder.
--ddl-only validates fixture data and guards without claiming engine coverage.
Full runs use the protected native codec and the engine's separate CLI process.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import runpy
import secrets
import shutil
import signal
import stat
import subprocess
import sys
import uuid

PREFIX = "uuid_u06_fixture_"
ID1 = "019a1163-0000-7000-8000-000000000001"
ID2 = "019a1163-0000-7000-8000-000000000002"
ID3 = "019a1163-0000-7000-8000-000000000003"
CHILD1 = "019a1163-0000-7000-8000-000000000011"
CHILD2 = "019a1163-0000-7000-8000-000000000012"
CHILD3 = "019a1163-0000-7000-8000-000000000013"
AUDIT1 = "019a1163-0000-7000-8000-000000000021"
AUDIT2 = "019a1163-0000-7000-8000-000000000022"
CREDENTIAL = "019a1163-0000-7000-8000-000000000031"
BIG = "123456789012345678901234567890123456789012345678901234567890.1234567890123456789012345678901234567890"
UPDATED_BIG = "987654321098765432109876543210987654321098765432109876543210.9876543210987654321098765432109876543210"
JSON_DOCUMENT = ' \n {"duplicate":1,"duplicate":2,"huge":900719925474099312345678901234567890,"exact":1.2300}\t '
NEW_JSON_DOCUMENT = '\t{"duplicate":3,"duplicate":4,"huge":900719925474099312345678901234567891,"exact":2.3400}\n '
JSON_NULL_DOCUMENT = ' \nnull\t '
NATIVE_CODEC_SHA = "03eda003367dd98c3745c17fcdb86517e74c01ad0cbae565901ad4101456e64a"

SCHEMA = """
CREATE TABLE public.fixture_records (
 id uuid PRIMARY KEY CHECK (substring(id::text,15,1)='7' AND substring(id::text,20,1) IN ('8','9','a','b')),
 serial_value bigint GENERATED ALWAYS AS IDENTITY (START WITH 9007199254740993) UNIQUE,
 amount numeric(100,40) NOT NULL,
 raw_json json NOT NULL,
 canonical_json jsonb NOT NULL,
 nullable_json json,
 occurred_at timestamptz NOT NULL,
 payload bytea NOT NULL,
 version integer NOT NULL DEFAULT 1,
 doubled numeric(101,40) GENERATED ALWAYS AS (amount * 2) STORED
);
CREATE TABLE public.fixture_children (
 id uuid PRIMARY KEY,
 parent_id uuid NOT NULL REFERENCES public.fixture_records(id),
 large_integer bigint NOT NULL,
 note text NOT NULL
);
CREATE TABLE public.audit_logs (
 id uuid PRIMARY KEY,
 aggregate_id uuid NOT NULL REFERENCES public.fixture_records(id),
 event text NOT NULL,
 created_at timestamptz NOT NULL
);
CREATE FUNCTION public.fixture_audit_immutable() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN RAISE EXCEPTION 'fixture append-only audit' USING ERRCODE='23514'; END
$$;
CREATE TRIGGER fixture_audit_append_only BEFORE UPDATE OR DELETE ON public.audit_logs
FOR EACH ROW EXECUTE FUNCTION public.fixture_audit_immutable();
CREATE TABLE public.platform_sessions (id uuid PRIMARY KEY, token text NOT NULL, expires_at timestamptz NOT NULL);
CREATE SEQUENCE public.fixture_sequence START WITH 10;
CREATE SEQUENCE public.fixture_unused_sequence START WITH 333;
CREATE TABLE public._sqlx_migrations(version bigint PRIMARY KEY, description text NOT NULL,
 checksum bytea NOT NULL, success boolean NOT NULL,
 installed_on timestamptz NOT NULL DEFAULT '2031-01-01T00:00:00Z', execution_time bigint NOT NULL DEFAULT 1);
INSERT INTO public._sqlx_migrations(version,description,checksum,success)
 VALUES(1,'isolated U06 fixture',decode(repeat('ab',48),'hex'),true);
"""

class Rejected(Exception):
    """Fixed identifiers only; subprocess stderr stays in a private file."""


def require(condition, code):
    if not condition:
        raise Rejected(code)


def literal(value):
    require(isinstance(value, str) and "\0" not in value, "SQL_LITERAL_INVALID")
    return "'" + value.replace("'", "''") + "'"


def identifier(value):
    require(isinstance(value, str) and re.fullmatch(r"[a-z][a-z0-9_]{0,62}", value), "SQL_IDENTIFIER_INVALID")
    return '"' + value + '"'


def write_new(path, raw):
    if isinstance(raw, str):
        raw = raw.encode()
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    os.fchmod(fd, 0o600)
    with os.fdopen(fd, "wb") as handle:
        handle.write(raw)
        handle.flush()
        os.fsync(handle.fileno())


def private_directory(value):
    require(sys.platform == "linux" and os.geteuid() == 0, "LINUX_ROOT_REQUIRED")
    require(value.startswith("/") and "\\" not in value and "\0" not in value and
            all(part not in ("", ".", "..") for part in value.split("/")[1:]), "ABSOLUTE_OUTPUT_REQUIRED")
    result = Path(value)
    for component in result.parents:
        require(not component.is_symlink(), "OUTPUT_SYMLINK_REJECTED")
    require(not result.exists(), "NEW_OUTPUT_REQUIRED")
    parent = result.parent.stat()
    require(stat.S_ISDIR(parent.st_mode) and parent.st_uid == 0 and stat.S_IMODE(parent.st_mode) == 0o700,
            "OUTPUT_PARENT_PRIVATE_REQUIRED")
    result.mkdir(mode=0o700)
    return result


class Fixture:
    def __init__(self, output):
        self.output = output
        self.run_id = secrets.token_hex(8)
        self.created = {}
        self.roles = {}
        self.calls = 0
        self.checks = []
        self.stage = "INITIALIZATION"

    def sql(self, database, statement, *, admin=False, accepted_failure=False):
        require((database == "postgres" and admin) or database in self.created, "UNREGISTERED_DATABASE_REJECTED")
        if database != "postgres":
            require(database.startswith(PREFIX + self.run_id + "_"), "FIXTURE_DATABASE_PREFIX_REQUIRED")
        self.calls += 1
        result = subprocess.run([
            "/usr/sbin/runuser", "-u", "postgres", "--", "/usr/bin/env", "-i", "PATH=/usr/bin:/bin",
            "LC_ALL=C", "PGCONNECT_TIMEOUT=5", "/usr/bin/psql", "-X", "-Atq", "-v", "ON_ERROR_STOP=1",
            "-h", "/run/postgresql", "-p", "5432", "-U", "postgres", "-d", database, "--file=-",
        ], input=statement, text=True, capture_output=True, timeout=45)
        if result.returncode and not accepted_failure:
            write_new(self.output / f"sql-error-{self.calls:04}.private.txt", result.stderr)
            raise Rejected("FIXTURE_SQL_FAILED")
        return result

    def json(self, database, statement, *, admin=False):
        result = self.sql(database, statement, admin=admin)
        try:
            return json.loads(result.stdout.strip())
        except ValueError:
            raise Rejected("FIXTURE_JSON_QUERY_FAILED") from None

    def create(self, case):
        self.stage = "FIXTURE_DATABASE_CREATION"
        require(re.fullmatch(r"[a-z][a-z0-9_]{0,15}", case), "FIXTURE_CASE_INVALID")
        database = PREFIX + self.run_id + "_" + case
        require(database not in self.created and len(database) <= 63, "FRESH_FIXTURE_DATABASE_REQUIRED")
        comment = "saas-uuid-u06-fixture/1/" + self.run_id + "/" + case
        owner = database + "_owner"
        exists = self.sql("postgres", "SELECT count(*) FROM pg_database WHERE datname=" + literal(database) + ";", admin=True)
        require(exists.stdout.strip() == "0", "FIXTURE_DATABASE_ALREADY_EXISTS")
        self.sql("postgres", "CREATE ROLE " + identifier(owner) + " NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;", admin=True)
        # Count a created-but-unverified owner as a cleanup failure instead of
        # silently omitting it if its OID query fails. Never delete without OID.
        self.roles[owner] = None
        owner_info = self.json("postgres", "SELECT json_build_object('oid',oid::bigint,'login',rolcanlogin) FROM pg_roles WHERE rolname=" + literal(owner) + ";", admin=True)
        self.roles[owner] = owner_info
        self.sql("postgres", "CREATE DATABASE " + identifier(database) + " OWNER " + identifier(owner) + " TEMPLATE template0;", admin=True)
        # Register immediately so a later fixture failure still cleans up only our database.
        self.created[database] = {"oid": None, "comment": comment, "owner": owner, "owner_oid": owner_info["oid"]}
        self.sql("postgres", "COMMENT ON DATABASE " + identifier(database) + " IS " + literal(comment) + ";", admin=True)
        metadata = self.json("postgres", "SELECT json_build_object('oid',oid::bigint,'comment',pg_catalog.shobj_description(oid,'pg_database')) FROM pg_database WHERE datname=" + literal(database) + ";", admin=True)
        require(metadata["comment"] == comment and isinstance(metadata["oid"], int), "FIXTURE_IDENTITY_INVALID")
        self.created[database]["oid"] = metadata["oid"]
        self.sql("postgres", "REVOKE ALL ON DATABASE " + identifier(database) + " FROM PUBLIC; ALTER DATABASE " + identifier(database) + " SET default_transaction_read_only=on;", admin=True)
        return database

    def baseline(self, database, seeded=True):
        prefix = "BEGIN READ WRITE;\nSET LOCAL ROLE " + identifier(self.created[database]["owner"]) + ";\n" + SCHEMA
        seed = f"""
INSERT INTO public.fixture_records(id,amount,raw_json,canonical_json,nullable_json,occurred_at,payload)
VALUES ({literal(ID1)},{BIG},{literal(JSON_DOCUMENT)}, '{{"nested":null,"integer":9007199254740993}}', NULL,
 '2031-01-01T15:01:02.123456+05:30',decode('0001ff805c0a0d','hex')),
 ({literal(ID2)},2.0000000000000000000000000000000000000001,'{{"other":true}}','null',{literal(JSON_NULL_DOCUMENT)},
 '2031-02-02T00:00:00.654321Z',decode('fe00','hex'));
INSERT INTO public.fixture_children VALUES
 ({literal(CHILD1)},{literal(ID1)},9007199254740993,'baseline retained'),
 ({literal(CHILD2)},{literal(ID2)},-9007199254740993,'baseline deleted later');
INSERT INTO public.audit_logs VALUES ({literal(AUDIT1)},{literal(ID1)},'BASELINE','2031-01-01T00:00:00Z');
INSERT INTO public.platform_sessions VALUES ({literal(CREDENTIAL)},'isolated-nonproduction-credential','2032-01-01T00:00:00Z');
SELECT nextval('public.fixture_sequence');
"""
        self.sql(database, prefix + (seed if seeded else "") + "COMMIT;\n")

    def mutate(self, database):
        self.sql(database, f"""BEGIN READ WRITE;
UPDATE public.fixture_records SET amount={UPDATED_BIG},raw_json={literal(NEW_JSON_DOCUMENT)},
 occurred_at='2033-03-03T03:03:03.333333Z',payload=decode('00ff1234','hex'),version=version+1 WHERE id={literal(ID1)};
DELETE FROM public.fixture_children WHERE id={literal(CHILD2)};
DELETE FROM public.fixture_records WHERE id={literal(ID2)};
INSERT INTO public.fixture_records(id,amount,raw_json,canonical_json,nullable_json,occurred_at,payload)
 VALUES ({literal(ID3)},3.0000000000000000000000000000000000000003,'{{"insert":true}}','{{"insert":true}}',NULL,
 '2033-03-03T03:03:03.333334Z',decode('00ee','hex'));
INSERT INTO public.fixture_children VALUES ({literal(CHILD3)},{literal(ID3)},9007199254740994,'inserted child');
INSERT INTO public.audit_logs VALUES ({literal(AUDIT2)},{literal(ID1)},'AFTER_SNAPSHOT','2033-03-03T03:03:03.333335Z');
INSERT INTO public.platform_sessions VALUES ('019a1163-0000-7000-8000-000000000032','second-isolated-credential','2034-01-01T00:00:00Z');
SELECT nextval('public.fixture_sequence');
SELECT nextval('public.fixture_sequence');
COMMIT;
""")

    def projection(self, database):
        # Cast each special type to its PostgreSQL text before building JSON:
        # Python must never parse NUMERIC, large bigint or duplicate-key JSON.
        return self.json(database, """BEGIN READ ONLY;
SELECT json_build_object(
 'records',(SELECT json_agg(x ORDER BY id) FROM (
  SELECT id::text,serial_value::text,amount::text,raw_json::text,
   canonical_json::text,nullable_json::text,nullable_json IS NULL AS sql_null,
   to_char(occurred_at AT TIME ZONE 'UTC','YYYY-MM-DD HH24:MI:SS.US') AS occurred_at,
   encode(payload,'hex') AS payload,version,doubled::text FROM public.fixture_records) x),
 'children',(SELECT json_agg(x ORDER BY id) FROM (SELECT id::text,parent_id::text,large_integer::text,note FROM public.fixture_children) x),
 'audit',(SELECT json_agg(x ORDER BY id) FROM (SELECT id::text,aggregate_id::text,event,created_at::text FROM public.audit_logs) x),
 'sequence',(SELECT json_build_object('lastValue',last_value::text,'called',is_called) FROM public.fixture_sequence),
 'unusedSequence',(SELECT json_build_object('lastValue',last_value::text,'called',is_called) FROM public.fixture_unused_sequence),
 'identitySequence',(SELECT json_build_object('lastValue',last_value::text,'called',is_called) FROM public.fixture_records_serial_value_seq));
COMMIT;
""")

    def smoke(self, source):
        self.stage = "FIXTURE_DDL_AND_TYPES"
        baseline = self.projection(source)
        record = next(row for row in baseline["records"] if row["id"] == ID1)
        require(record["amount"] == BIG and record["raw_json"] == JSON_DOCUMENT
                and record["serial_value"] == "9007199254740993"
                and record["payload"] == "0001ff805c0a0d" and record["sql_null"] is True,
                "EXACT_NATIVE_TYPE_BASELINE_FAILED")
        other = next(row for row in baseline["records"] if row["id"] == ID2)
        require(other["nullable_json"] == JSON_NULL_DOCUMENT and other["sql_null"] is False,
                "JSON_NULL_SQL_NULL_FIXTURE_FAILED")
        denied = self.sql(source, "BEGIN READ WRITE; UPDATE public.audit_logs SET event='forbidden'; COMMIT;", accepted_failure=True)
        require(denied.returncode != 0 and self.projection(source) == baseline, "APPEND_ONLY_AUDIT_GUARD_FAILED")
        denied = self.sql(source, "BEGIN READ WRITE; DELETE FROM public.fixture_records WHERE id=" + literal(ID2) + "; COMMIT;", accepted_failure=True)
        require(denied.returncode != 0 and self.projection(source) == baseline, "FOREIGN_KEY_GUARD_FAILED")
        self.checks.append("native-types-json-null-fk-and-append-only-fixture")
        return baseline

    def cleanup(self):
        failures = 0
        for database, expected in reversed(list(self.created.items())):
            try:
                require(database.startswith(PREFIX + self.run_id + "_"),
                        "CLEANUP_FIXTURE_IDENTITY_REQUIRED")
                observed = self.json("postgres", "SELECT json_build_object('oid',oid::bigint,'comment',pg_catalog.shobj_description(oid,'pg_database')) FROM pg_database WHERE datname=" + literal(database) + ";", admin=True)
                require(observed["comment"] == expected["comment"] and
                        (expected["oid"] is None or observed["oid"] == expected["oid"]),
                        "CLEANUP_FIXTURE_IDENTITY_CHANGED")
                self.sql("postgres", "DROP DATABASE " + identifier(database) + " WITH (FORCE);", admin=True)
            except Exception:
                failures += 1
        for owner, expected in reversed(list(self.roles.items())):
            try:
                require(owner.startswith(PREFIX + self.run_id + "_"), "CLEANUP_OWNER_PREFIX_REQUIRED")
                observed = self.json("postgres", "SELECT json_build_object('oid',oid::bigint,'login',rolcanlogin) FROM pg_roles WHERE rolname=" + literal(owner) + ";", admin=True)
                require(observed == expected and observed["login"] is False, "CLEANUP_OWNER_IDENTITY_CHANGED")
                self.sql("postgres", "DROP ROLE " + identifier(owner) + ";", admin=True)
            except Exception:
                failures += 1
        return failures


def run_engine_cases(fixture, args, source, baseline):
    fixture.stage = "ENGINE_INTERFACE_SETUP"
    engine = Path(args.engine)
    require(engine.is_file(), "ENGINE_SCRIPT_REQUIRED")
    key_file = fixture.output / "fixture.key"
    write_new(key_file, secrets.token_bytes(32))
    batch = str(uuid.uuid7()) if hasattr(uuid, "uuid7") else "019a1163-0000-7000-8000-" + secrets.token_hex(6)
    plain_root = Path("/dev/shm/saas-uuid-u06-20261024")
    cipher_root = Path("/var/lib/saas-uuid-u06-20261024")
    for root in [plain_root, cipher_root]:
        require(root.is_dir() and not root.is_symlink() and root.stat().st_uid == 0
                and stat.S_IMODE(root.stat().st_mode) == 0o700, "NATIVE_CODEC_PRIVATE_ROOT_REQUIRED")
    codec = Path("/usr/local/libexec/saas-recovery-u06-codec")
    info = codec.stat()
    require(codec.is_file() and not codec.is_symlink() and info.st_uid == 0
            and not (stat.S_IMODE(info.st_mode) & 0o022)
            and hashlib.sha256(codec.read_bytes()).hexdigest() == NATIVE_CODEC_SHA,
            "NATIVE_CODEC_IDENTITY_MISMATCH")
    scratch = plain_root / batch
    scratch.mkdir(mode=0o700)
    cipher_directory = cipher_root / batch
    cipher_directory.mkdir(mode=0o700)
    final = fixture.create("final")
    recovery = fixture.create("recovery")
    fixture.baseline(final, seeded=False)
    fixture.baseline(recovery, seeded=False)
    schema_sql = Path(__file__).resolve().parents[3] / "src/database/schema_material.sql"
    schema_query = schema_sql.read_text()
    catalog = fixture.json(source, "BEGIN READ ONLY; SET LOCAL search_path=pg_catalog; SET LOCAL timezone='UTC'; "
                           "SET LOCAL datestyle='ISO, YMD'; SET LOCAL intervalstyle='postgres'; "
                           "SET LOCAL extra_float_digits=3; SET LOCAL bytea_output='hex'; "
                           "SET LOCAL standard_conforming_strings=on; SET LOCAL row_security=off; "
                           "SET LOCAL session_replication_role='origin';\n" + schema_query + "\nCOMMIT;")
    # Only the pure schema normalizer is reused. Every data operation below is
    # still a separate CLI process consuming authenticated encrypted packages.
    semantic_material = runpy.run_path(str(engine))["semantic_material"]
    semantic_catalog = semantic_material(catalog)
    names = sorted(["_sqlx_migrations", "audit_logs", "fixture_children", "fixture_records", "platform_sessions"])
    def canonical(value):
        return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()
    def sha(raw):
        return hashlib.sha256(raw).hexdigest()
    system_id = fixture.sql("postgres", "BEGIN READ ONLY; SELECT system_identifier FROM pg_control_system(); COMMIT;", admin=True).stdout.strip()
    groups = {"source": source, "final": final, "recovery": recovery}
    registration = {"format": "saas-uuid-u06-registration/1", "system_identifier": system_id,
        "groups": {group: [{"kind": "platform", "name": name, "oid": fixture.created[name]["oid"],
                             "owner": fixture.created[name]["owner"], "owner_oid": fixture.created[name]["owner_oid"]}]
                   for group, name in groups.items()}, "fenced_roles": {group: [] for group in groups},
        "fixture_contracts": {"platform": {"tables": names,
            "schema_sha256": sha(canonical(semantic_catalog)), "migration_count": 1,
            "credentials": ["platform_sessions"], "metadata": ["_sqlx_migrations"], "append_only": ["audit_logs"]}}}
    registration_file = fixture.output / "registration.private.json"
    write_new(registration_file, canonical(registration) + b"\n")
    specification = {"binary": str(codec), "sha256": sha(codec.read_bytes()), "key_file": str(key_file)}
    call_number = 0
    def call(operation, label, *, group=None, input_spec=None, before=None, after=None, package=False, reject=False):
        nonlocal call_number
        call_number += 1
        fixture.stage = "ENGINE_" + label.upper().replace("-", "_")
        report_path = fixture.output / (label + ".report.private.json")
        cfg = {"format": "saas-uuid-u06-data/1", "profile": "uuid-u06-fixture",
               "registration": str(registration_file), "scratch": str(scratch), "codec": specification,
               "report": str(report_path)}
        if group is not None: cfg["group"] = group
        if input_spec is not None: cfg["input"] = input_spec
        if before is not None: cfg["before"] = before
        if after is not None: cfg["after"] = after
        if package: cfg["output"] = {"path": str(cipher_directory / (label + ".enc")),
                                     "purpose": "saas-uuid-preview-freeze/1/" + batch + "/u06-" + label}
        config_path = fixture.output / (label + ".config.private.json")
        write_new(config_path, canonical(cfg) + b"\n")
        process = subprocess.Popen([sys.executable, str(engine), "--" + operation, str(config_path)],
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
        try:
            out, err = process.communicate(timeout=180)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            out, err = process.communicate(timeout=10)
            write_new(fixture.output / (label + ".stdout.private.txt"), out)
            write_new(fixture.output / (label + ".stderr.private.txt"), err)
            raise Rejected("ENGINE_FIXTURE_TIMEOUT") from None
        write_new(fixture.output / (label + ".stdout.private.txt"), out)
        write_new(fixture.output / (label + ".stderr.private.txt"), err)
        if reject:
            require(process.returncode != 0 and "U06_DATA_REJECTED:" in err, "ENGINE_NEGATIVE_NOT_REJECTED")
            category = err.strip().removeprefix("U06_DATA_REJECTED:")
            require(category in reject, "ENGINE_NEGATIVE_WRONG_REJECTION")
            fixture.checks.append(label + "-rejected")
            return None
        require(process.returncode == 0 and report_path.is_file(), "ENGINE_POSITIVE_FAILED")
        report = json.loads(report_path.read_bytes())
        fixture.checks.append(label)
        if package:
            artifact = report["artifact"]
            return {key: artifact[key] for key in ["path", "purpose", "sha256"]}
        return report
    try:
        before = call("snapshot", "snapshot-before", group="source", package=True)
        call("restore", "source-restore-forbidden", group="source", input_spec=before,
             reject={"SOURCE_WRITES_FORBIDDEN"})
        require(fixture.projection(source) == baseline, "SOURCE_RESTORE_REJECTION_MUTATED_DATABASE")
        call("restore", "restore-baseline-final", group="final", input_spec=before)
        require(fixture.projection(final) == baseline, "BASELINE_RESTORE_EXACT_VALUES_FAILED")
        require(fixture.sql(final, "BEGIN READ ONLY; SELECT count(*) FROM public.platform_sessions; COMMIT;").stdout.strip() == "0",
                "CREDENTIAL_EXCLUSION_FAILED")
        call("verify", "verify-final-baseline", group="final", input_spec=before)
        tampered = cipher_directory / "snapshot-tampered.enc"
        damaged = bytearray(Path(before["path"]).read_bytes())
        damaged[-1] ^= 1
        write_new(tampered, damaged)
        tamper_spec = {**before, "path": str(tampered)}
        recovery_before = fixture.projection(recovery)
        call("restore", "ciphertext-tamper", group="recovery", input_spec=tamper_spec,
             reject={"AUTHENTICATED_CODEC_FAILED"})
        require(fixture.projection(recovery) == recovery_before, "TAMPER_REJECTION_MUTATED_TARGET")
        call("restore", "restore-baseline-recovery", group="recovery", input_spec=before)
        fixture.mutate(source)
        expected_after = fixture.projection(source)
        after = call("snapshot", "snapshot-after", group="source", package=True)
        changes = call("delta", "delta", before=before, after=after, package=True)
        call("delta", "reverse-time-delta", before=after, after=before, package=True,
             reject={"DELTA_TIME_DIRECTION_INVALID"})
        fixture.sql(source, "BEGIN READ WRITE; SELECT setval('public.fixture_sequence',11,true); COMMIT;")
        rewind = call("snapshot", "snapshot-sequence-rewind", group="source", package=True)
        call("delta", "sequence-rewind-delta", before=after, after=rewind, package=True,
             reject={"DELTA_SEQUENCE_WOULD_REWIND"})
        fixture.sql(source, "BEGIN READ WRITE; SELECT setval('public.fixture_sequence',12,true); COMMIT;")
        require(fixture.projection(source) == expected_after, "SEQUENCE_FIXTURE_RESET_FAILED")
        call("replay", "source-replay-forbidden", group="source", input_spec=changes,
             reject={"REPLAY_REQUIRES_RECOVERY_TARGET"})
        require(fixture.projection(source) == expected_after, "SOURCE_REPLAY_REJECTION_MUTATED_DATABASE")
        call("replay", "final-target-replay-forbidden", group="final", input_spec=changes,
             reject={"REPLAY_REQUIRES_RECOVERY_TARGET"})
        require(fixture.projection(final) == baseline, "FINAL_REPLAY_REJECTION_MUTATED_TARGET")
        fixture.sql(recovery, "BEGIN READ WRITE; UPDATE public.fixture_records SET version=99 WHERE id=" + literal(ID1) + "; COMMIT;")
        conflict_before = fixture.projection(recovery)
        call("replay", "before-image-conflict", group="recovery", input_spec=changes,
             reject={"TARGET_FULL_ROW_RECONCILIATION_FAILED", "DELTA_BEFORE_IMAGE_MISMATCH"})
        require(fixture.projection(recovery) == conflict_before, "BEFORE_IMAGE_REJECTION_MUTATED_TARGET")
        fixture.sql(recovery, "BEGIN READ WRITE; UPDATE public.fixture_records SET version=1 WHERE id=" + literal(ID1) + "; COMMIT;")
        require(fixture.projection(recovery) == baseline, "CONFLICT_FIXTURE_BASELINE_RESET_FAILED")
        call("replay", "replay-recovery", group="recovery", input_spec=changes)
        call("verify", "verify-recovery", group="recovery", input_spec=after)
        require(fixture.projection(recovery) == expected_after, "RECOVERY_EXACT_VALUES_FAILED")
        final_before_repeat = fixture.projection(recovery)
        call("replay", "already-replayed", group="recovery", input_spec=changes,
             reject={"TARGET_FULL_ROW_RECONCILIATION_FAILED", "DELTA_BEFORE_IMAGE_MISMATCH"})
        require(fixture.projection(recovery) == final_before_repeat, "REPEATED_REPLAY_MUTATED_TARGET")
        fixture.checks.append("full-native-projection-numeric-json-timestamp-bytea-generated-identity-sequences")
    finally:
        require(scratch.parent == Path("/dev/shm/saas-uuid-u06-20261024") and scratch.name == batch
                and not scratch.is_symlink(), "SCRATCH_CLEANUP_BOUNDARY_REQUIRED")
        shutil.rmtree(scratch)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True)
    parser.add_argument("--engine", default=str(Path(__file__).with_name("uuid-u06-data.py")))
    parser.add_argument("--ddl-only", action="store_true")
    parser.add_argument("--native-codec", action="store_true", help="Compatibility alias; full runs always use the protected native codec")
    args = parser.parse_args()
    output = private_directory(args.output)
    fixture = Fixture(output)
    report = {"format": "saas-uuid-u06-data-fixture/1", "status": "FAIL", "businessDatabasesAccessed": 0,
              "codec": "unused-ddl-only" if args.ddl_only else "protected-native"}
    try:
        version = fixture.sql("postgres", "BEGIN READ ONLY; SHOW server_version_num; COMMIT;", admin=True).stdout.strip()
        require(version.isdigit() and int(version) >= 170000, "POSTGRES_17_PLUS_REQUIRED")
        source = fixture.create("source")
        fixture.baseline(source)
        baseline = fixture.smoke(source)
        if args.ddl_only:
            fixture.mutate(source)
            after = fixture.projection(source)
            require(after != baseline and len(after["records"]) == 2 and len(after["audit"]) == 2,
                    "INSERT_UPDATE_DELETE_FIXTURE_FAILED")
            fixture.checks.append("real-insert-update-delete-and-sequences")
            report["status"] = "DDL_ONLY"
        else:
            run_engine_cases(fixture, args, source, baseline)
            report["status"] = "PASS"
        report["postgresMajor"] = int(version) // 10000
    except Rejected as error:
        report["error"] = str(error)
    except Exception:
        report["error"] = "FIXTURE_OPERATION_FAILED"
    finally:
        cleanup_failures = fixture.cleanup()
        report.update({"stage": fixture.stage, "checks": fixture.checks, "sqlCalls": fixture.calls,
                       "fixtureDatabasesCreated": len(fixture.created), "fixtureOwnersCreated": len(fixture.roles),
                       "cleanupFailures": cleanup_failures})
        if cleanup_failures:
            if "error" in report:
                report["primaryError"] = report["error"]
            report["status"] = "FAIL"
            report["error"] = "FIXTURE_CLEANUP_FAILED"
        write_new(output / "summary.private.json", json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, separators=(",", ":")))
    return 0 if report["status"] in ("PASS", "DDL_ONLY") else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Rejected as error:
        print(json.dumps({"status": "FAIL", "error": str(error)}), file=sys.stderr)
        raise SystemExit(1)
    except Exception:
        print(json.dumps({"status": "FAIL", "error": "FIXTURE_OPERATION_FAILED"}), file=sys.stderr)
        raise SystemExit(1)
