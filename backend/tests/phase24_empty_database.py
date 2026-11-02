"""Verify the retired baseline in one owned empty database, with final cleanup.

No Docker, existing database reset, HBA change or application process is used.
The task PostgreSQL and its protected admin URL must already belong to this run.
"""

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
from urllib.parse import urlsplit


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--task-dir", type=Path, required=True)
    parser.add_argument("--snapshot", required=True)
    parser.add_argument("--migration-bin", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    task = args.task_dir
    assert task.resolve(strict=True) == task and task.parent == Path("/tmp")
    assert task.name.startswith("rust-saas-unified-auth-")
    assert (task / "OWNER").read_text().strip() == task.name + "-pg55494"
    assert re.fullmatch(r"phase24-[a-z0-9-]+", args.snapshot)
    assert args.migration_bin.resolve(strict=True).is_relative_to(
        Path("/home/jackie/.cache/rust-saas-unified-auth-20261029/target-linux"))
    snapshot = task / ("source-snapshot-" + args.snapshot)
    backend = snapshot / "saas-plateform/backend"
    manifest = json.loads((task / ("snapshot-" + args.snapshot + ".json")).read_text())
    for relative in ("migrations/0001_init.sql", "ops/identity-deployment/create-roles.sql",
                     "ops/identity-deployment/provision.sql", "ops/identity-deployment/verify.sql"):
        assert hashlib.sha256((backend / relative).read_bytes()).hexdigest() == manifest[
            "inputs"]["saas-plateform/backend/" + relative]
    helper_path = backend / "tests/unified-auth-https/prepare.py"
    sys.path.insert(0, str(helper_path.parent))
    spec = importlib.util.spec_from_file_location("phase24_prepare", helper_path)
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    admin_url = (task / "admin-url").read_text().strip()
    parsed = urlsplit(admin_url)
    assert parsed.hostname == "127.0.0.1" and parsed.port == 55494
    assert parsed.username == "postgres" and parsed.path == "/postgres"
    binding = json.loads((task / "phase23-cluster-binding.json").read_text())
    assert binding["dataDirectory"] == str(task / "data")
    cluster = helper.psql(admin_url, "SELECT system_identifier FROM pg_control_system()")
    assert cluster == binding["systemIdentifier"]
    deployment = helper.new_id()
    database = "unified_auth_https_" + deployment.replace("-", "")
    prefix = "uah_" + database[-12:]
    owned = {"database": database, "rolePrefix": prefix, "clusterId": cluster}
    url = admin_url.rsplit("/", 1)[0] + "/" + database
    result = {"status": "FAIL", "validationLevel": "ACTUAL_EMPTY_POSTGRESQL",
              "snapshot": args.snapshot, "snapshotSha256": manifest["snapshotSha256"],
              "migrationSha256": manifest["inputs"]["saas-plateform/backend/migrations/0001_init.sql"],
              "migrationBinarySha256": hashlib.sha256(args.migration_bin.read_bytes()).hexdigest(),
              "dockerCreated": 0, "dockerReused": 0, "checks": [], "cleanup": {}}
    created = False

    def check(name, condition):
        assert condition, name
        result["checks"].append({"name": name, "status": "PASS"})

    def run_runtime(role, statement):
        env = {key: value for key, value in os.environ.items() if not key.upper().startswith("PG")}
        env.update(PGHOST="127.0.0.1", PGPORT="55494", PGDATABASE=database, PGUSER=role)
        return subprocess.run(["psql", "-X", "-At", "-v", "ON_ERROR_STOP=1", "-v",
                               "VERBOSITY=sqlstate"], input=statement, capture_output=True,
                              text=True, env=env, timeout=30)

    try:
        check("fresh_database_absent", helper.psql(admin_url,
              "SELECT NOT EXISTS(SELECT 1 FROM pg_database WHERE datname=" + helper.literal(database) + ")") == "t")
        check("fresh_roles_absent", helper.psql(admin_url,
              "SELECT NOT EXISTS(SELECT 1 FROM pg_roles WHERE left(rolname," + str(len(prefix) + 1) + ")="
              + helper.literal(prefix + "_") + ")") == "t")
        helper.psql(admin_url, 'CREATE DATABASE "' + database + '";')
        created = True
        helper.psql(admin_url, 'COMMENT ON DATABASE "' + database + '" IS ' +
                    helper.literal("identity-deployment/1/" + deployment))
        env = {key: value for key, value in os.environ.items()
               if key not in ("DATABASE_URL", "SQLX_OFFLINE") and not key.upper().startswith("PG")}
        env["DATABASE_URL"] = url
        for name in ("fresh_migration", "repeat_same_migration"):
            migration = subprocess.run([str(args.migration_bin)], env=env,
                                       capture_output=True, text=True, timeout=60)
            check(name, migration.returncode == 0)
        helper.psql(url, "INSERT INTO public.auth_provider_settings(singleton,issuer,created_at) "
                    "VALUES(true,'https://auth.saas.test',clock_timestamp())")
        values = {"expected_database": database, "deployment_id": deployment,
                  "issuer": "https://auth.saas.test", "role_prefix": prefix}
        for filename in ("create-roles.sql", "provision.sql", "verify.sql"):
            sql = (backend / "ops/identity-deployment" / filename).read_text()
            for key, value in values.items():
                sql = sql.replace(":'" + key + "'", helper.literal(value))
            helper.psql(url, sql)
            check(filename, True)
        forbidden = ["oauth_clients", "oauth_tokens", "oauth_authorization_codes",
                     "oauth_authorization_requests", "oauth_device_codes", "oidc_clients",
                     "oidc_consents", "oidc_authorization_requests", "oidc_authorization_codes",
                     "oidc_grants", "oidc_access_tokens", "oidc_refresh_tokens", "oidc_signing_keys",
                     "oidc_provider_settings", "oidc_context_clients", "oidc_context_interactions",
                     "oidc_account_logout_receipts", "identity_oidc_admin_receipts",
                     "platform_oidc_command_intents"]
        check("legacy_relations_absent", all(helper.psql(url,
              "SELECT to_regclass('public." + name + "') IS NULL") == "t" for name in forbidden))
        check("sqlx_single_successful_baseline", helper.psql(url,
              "SELECT count(*)=1 AND bool_and(version=1 AND success) FROM public._sqlx_migrations") == "t")
        result["tableCount"] = int(helper.psql(url,
            "SELECT count(*) FROM information_schema.tables WHERE table_schema='public' AND table_type='BASE TABLE'"))
        for kind, own, blocked in (("identity", "auth_clients", "memberships"),
                                   ("platform", "memberships", "auth_client_credentials")):
            role = prefix + "_" + kind + "_runtime"
            ready = run_runtime(role, "SELECT current_user=session_user AND "
                                "public.identity_phase20_schema_ready(); SELECT count(*) FROM public." + own)
            check(kind + "_login_readiness_and_owned_read", ready.returncode == 0 and ready.stdout.splitlines()[0] == "t")
            for label, sql in (("cross_owner_denied", "SELECT * FROM public." + blocked),
                               ("ddl_denied", "CREATE TABLE public.phase24_forbidden(id int)"),
                               ("owner_role_denied", 'SET ROLE "' + prefix + "_" + kind + '_owner"')):
                denied = run_runtime(role, sql)
                check(kind + "_" + label, denied.returncode != 0 and "42501" in denied.stderr)
        # Recovery declarations are current inputs, separately attested from the
        # immutable migration binary and baseline snapshot.
        inventory_path = Path(__file__).resolve().parents[1] / "ops/identity-recovery/inventory.py"
        result["inventorySha256"] = hashlib.sha256(inventory_path.read_bytes()).hexdigest()
        inv_spec = importlib.util.spec_from_file_location("phase24_inventory", inventory_path)
        inventory = importlib.util.module_from_spec(inv_spec)
        sys.modules[inv_spec.name] = inventory
        inv_spec.loader.exec_module(inventory)
        # Compare every declared recovery table with the actual catalog.
        for name, declaration in inventory.TABLES["platform"].items():
            actual = helper.psql(url, "SELECT column_name||':'||udt_name||':'||"
                "CASE WHEN is_nullable='NO' THEN '!' ELSE '?' END FROM information_schema.columns "
                "WHERE table_schema='public' AND table_name=" + helper.literal(name) + " ORDER BY ordinal_position")
            expected = {column.source + ":" + column.pg_type + ":" + ("?" if column.nullable else "!")
                        for column in declaration.fields.values()}
            check("inventory_" + name, set(actual.splitlines()) == expected)
        result["status"] = "PASS"
    except Exception as error:
        # Only fixed labels / SQLSTATE reach the permanent evidence.
        result["errorType"] = type(error).__name__
        detail = str(error)
        if re.fullmatch(r"[A-Za-z0-9_. -]+(?:\(SQLSTATE [A-Z0-9]{5}\))?", detail):
            result["error"] = detail
        raise
    finally:
        try:
            if created:
                helper.cleanup(admin_url, owned)
            result["cleanup"] = {"databaseAbsent": helper.psql(admin_url,
                "SELECT NOT EXISTS(SELECT 1 FROM pg_database WHERE datname=" + helper.literal(database) + ")") == "t",
                "rolesAbsent": helper.psql(admin_url,
                "SELECT NOT EXISTS(SELECT 1 FROM pg_roles WHERE left(rolname," + str(len(prefix) + 1) + ")="
                + helper.literal(prefix + "_") + ")") == "t"}
        except Exception:
            result["status"] = "FAIL"
            result["cleanup"] = {"status": "REQUIRES_OWNER_RECONCILIATION"}
            raise
        finally:
            args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"status": result["status"], "checks": len(result["checks"]),
                      "cleanup": result["cleanup"]}))


if __name__ == "__main__":
    main()
