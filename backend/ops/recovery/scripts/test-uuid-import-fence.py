#!/usr/bin/env python3
"""Run real fences and optional native preflight in a disposable offline PG17."""
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import sys
import time

IMAGE = "postgres:17.6-bookworm"
DATABASE = "qiankun_platform_uuid7_u04_20261024"
OWNER = "qiankun_platform_uuid7_u04_owner_20261024"
LABEL = "saas.uuid-import-fence-fixture"
VOLUME = "qiankun-preview-runtime-20261024"


def main():
    directory = Path(sys.argv[1]).resolve(strict=True)
    fence = (directory / "platform-02-fence.sql").read_text(encoding="utf-8")
    if "has_sequence_privilege" not in fence or "CASE WHEN" not in fence:
        raise RuntimeError("actual generated CASE-protected fence query required")
    token = secrets.token_hex(12)
    container = None
    helper = None

    def call(args, sql=None, checked=True):
        result = subprocess.run(args, input=sql, text=True, capture_output=True, timeout=45)
        if checked and result.returncode:
            fd = os.open(directory / f"fixture-{token}.stderr", os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(fd, "w", encoding="utf-8") as handle:
                handle.write(result.stderr)
            raise RuntimeError("isolated fixture native command failed; private stderr retained")
        return result

    def sql(database, text):
        return call(["docker", "exec", "-i", container, "psql", "-X", "-qAt", "-v", "ON_ERROR_STOP=1", "-U", "postgres", "-d", database, "--file=-"], text).stdout.strip()

    def observe():
        result = json.loads(sql(DATABASE, "BEGIN ISOLATION LEVEL READ COMMITTED READ WRITE;\nSET LOCAL search_path=pg_catalog;\n" + fence + "ROLLBACK;\n"))
        if result["database"] != DATABASE:
            raise RuntimeError("fixture connection identity mismatch")
        return result

    def native_preflight():
        nonlocal helper
        if len(sys.argv) != 3:
            return False
        test_binary = sys.argv[2]
        if not re.fullmatch(r"/runtime/uuid-recovery-target/debug/deps/saas_recovery-[0-9a-f]+", test_binary):
            raise RuntimeError("explicit native fixture test binary required")
        helper = call(["docker", "create", "--network", "none", "--label", f"{LABEL}={token}", "-v", VOLUME + ":/runtime:ro", "saas-ci:phase04-tools", "true"]).stdout.strip()
        if not re.fullmatch(r"[0-9a-f]{64}", helper):
            raise RuntimeError("invalid fixture copy helper identity")

        def copy_runtime(path, name):
            local = directory / f"fixture-{token}-{name}"
            call(["docker", "cp", helper + ":" + path, str(local)])
            local.chmod(0o700)
            remote = "/tmp/" + name
            call(["docker", "cp", str(local), container + ":" + remote])
            call(["docker", "exec", container, "chmod", "755", remote])
            local.unlink()
            return remote

        cases = [
            ("platform", DATABASE, OWNER, "/runtime/target/debug/platform-migrate", "DATABASE_URL"),
            ("stores", "qiankun_stores_uuid7_u04_20261024", "qiankun_stores_uuid7_u04_owner_20261024", "/runtime/stores-u02-target/debug/stores-migrate", "STORES_DATABASE_URL"),
            ("work_orders", "qiankun_orders_uuid7_u04_20261024", "qiankun_orders_uuid7_u04_owner_20261024", "/runtime/orders-u02-target/debug/work-orders-migrate", "WORK_ORDERS_DATABASE_URL"),
        ]
        for kind, database, owner, binary, env_name in cases:
            if kind != "platform":
                sql("postgres", f"CREATE ROLE {owner} NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;\nCREATE DATABASE {database} OWNER {owner} TEMPLATE template0;\nREVOKE ALL ON DATABASE {database} FROM PUBLIC;\n")
            sql("postgres", f"ALTER DATABASE {database} SET default_transaction_read_only=off;")
            remote = copy_runtime(binary, kind + "-migrate")
            url = "postgresql://postgres@localhost:5432/" + database + "?host=/var/run/postgresql&options[role]=" + owner
            call(["docker", "exec", "-u", "postgres", container, "env", "-i", env_name + "=" + url, remote, "--import"])
            sql("postgres", f"ALTER DATABASE {database} SET default_transaction_read_only=on;")
        remote = copy_runtime(test_binary, "uuid-import-native-tests")
        result = call(["docker", "exec", "-u", "postgres", container, "env", "-i", "PATH=/usr/lib/postgresql/17/bin:/usr/bin:/bin", "UUID_IMPORT_PG17_FIXTURE=isolated-offline-fixture-v1", remote, "uuid_rekey::import::session::tests::isolated_pg17_sql_without_newline_and_migration_contracts", "--ignored", "--exact", "--nocapture"])
        if "1 passed; 0 failed" not in result.stdout:
            raise RuntimeError("native interactive PG17 fixture did not pass")
        return True

    try:
        container = call(["docker", "run", "-d", "--rm", "--network", "none", "--label", f"{LABEL}={token}", "--name", f"uuid-import-fence-{token}", "--tmpfs", "/var/lib/postgresql/data:rw", "-e", "POSTGRES_HOST_AUTH_METHOD=trust", IMAGE, "postgres", "-c", "cluster_name=uuid-import-fixture-v1"]).stdout.strip()
        if not re.fullmatch(r"[0-9a-f]{64}", container):
            raise RuntimeError("invalid fixture container identity")
        for _ in range(50):
            initialized = "PostgreSQL init process complete" in call(["docker", "logs", container], checked=False).stdout
            if initialized and call(["docker", "exec", container, "pg_isready", "-q", "-U", "postgres"], checked=False).returncode == 0:
                break
            time.sleep(0.2)
        else:
            raise RuntimeError("isolated PG17 fixture did not become ready")
        version = int(sql("postgres", "SHOW server_version_num;"))
        if not 170000 <= version < 180000:
            raise RuntimeError("PG17 fixture version mismatch")
        sql("postgres", f"CREATE ROLE {OWNER} NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;\nCREATE ROLE fixture_visitor LOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;\nCREATE DATABASE {DATABASE} OWNER {OWNER} TEMPLATE template0;\nREVOKE ALL ON DATABASE {DATABASE} FROM PUBLIC;\n")
        sql(DATABASE, f"SET ROLE {OWNER};\nCREATE TABLE public.fence_fixture(id bigint PRIMARY KEY,payload text);\nALTER TABLE public.fence_fixture ALTER COLUMN payload SET STORAGE EXTERNAL;\nINSERT INTO public.fence_fixture VALUES(1,repeat('isolated-fixture',1000));\nCREATE INDEX fence_fixture_extra_index ON public.fence_fixture((id+1));\nCREATE SEQUENCE public.fence_fixture_seq;\nCREATE FUNCTION public.fence_fixture_fn() RETURNS integer LANGUAGE sql IMMUTABLE AS $$ SELECT 1 $$;\nRESET ROLE;\nALTER DATABASE {DATABASE} SET default_transaction_read_only=on;\n")
        shapes = json.loads(sql(DATABASE, "SELECT jsonb_build_object('toast',(SELECT count(*) FROM pg_class WHERE relkind='t'),'indexes',(SELECT count(*) FROM pg_class WHERE relkind='i'),'sequences',(SELECT count(*) FROM pg_class WHERE relkind='S'),'other_login',(SELECT count(*) FROM pg_roles WHERE rolcanlogin AND rolname<>'postgres'));"))
        if any(shapes[key] < 1 for key in ("toast", "indexes", "sequences", "other_login")):
            raise RuntimeError("fixture lacks required relation types or alternate LOGIN")
        clean = observe()
        for key in ("actor_valid", "local", "transaction_valid", "owner_valid", "database_valid", "object_ownership_valid", "acl_valid", "default_read_only"):
            if clean[key] is not True:
                raise RuntimeError("clean fixture admission failed")
        if any(clean[key] != 0 for key in ("bad_data_writers", "bad_logins", "other_backends", "prepared_transactions")) or clean["comment"] is not None:
            raise RuntimeError("clean fixture counts or epoch invalid")
        sql(DATABASE, "BEGIN READ WRITE; GRANT INSERT ON public.fence_fixture TO fixture_visitor; COMMIT;")
        table_write = observe()
        if table_write["bad_data_writers"] != 1 or table_write["acl_valid"] is not False:
            raise RuntimeError("unregistered table writer was accepted")
        sql(DATABASE, "BEGIN READ WRITE; REVOKE INSERT ON public.fence_fixture FROM fixture_visitor; GRANT USAGE ON SEQUENCE public.fence_fixture_seq TO fixture_visitor; COMMIT;")
        sequence_write = observe()
        if sequence_write["bad_data_writers"] != 1 or sequence_write["acl_valid"] is not False:
            raise RuntimeError("unregistered sequence writer was accepted")
        sql(DATABASE, "BEGIN READ WRITE; REVOKE USAGE ON SEQUENCE public.fence_fixture_seq FROM fixture_visitor; COMMIT;")
        restored = observe()
        if restored["bad_data_writers"] != 0 or restored["acl_valid"] is not True:
            raise RuntimeError("fixture privilege cleanup did not restore the fence")
        native = native_preflight()
        print(json.dumps({"status": "PASS", "postgres_major": 17, "checks": 4, "typed_relation_guards": True, "relation_shape_counts": shapes, "native_open_stdin_without_lf": native, "actual_migration_sql_smoke_databases": 3 if native else 0, "business_databases_accessed": 0}))
    finally:
        if helper and re.fullmatch(r"[0-9a-f]{64}", helper):
            state = json.loads(call(["docker", "inspect", helper]).stdout)[0]
            if state["Id"] != helper or state["Config"]["Labels"].get(LABEL) != token or state["HostConfig"]["NetworkMode"] != "none":
                raise RuntimeError("fixture helper cleanup identity check failed")
            call(["docker", "rm", "-f", helper])
        if container and re.fullmatch(r"[0-9a-f]{64}", container):
            state = json.loads(call(["docker", "inspect", container]).stdout)[0]
            if state["Id"] != container or state["Config"]["Labels"].get(LABEL) != token or state["HostConfig"]["NetworkMode"] != "none":
                raise RuntimeError("fixture cleanup identity check failed")
            call(["docker", "rm", "-f", container])


if __name__ == "__main__":
    main()
