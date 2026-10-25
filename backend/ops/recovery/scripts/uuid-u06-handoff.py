#!/usr/bin/env python3
"""Reconcile an isolated U06 receiver, then bind its import marker and epoch receipt."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sys

SPEC = importlib.util.spec_from_file_location("u06_data_handoff", Path(__file__).with_name("uuid-u06-data.py"))
D = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(D)
BASE = Path("/var/lib/saas-uuid-u06-20261024")


def handoff(path, generation):
    D.require(generation in ("final", "recovery"), "CLOSED_RECEIVER_GENERATION_REQUIRED")
    raw = D.private_file(path).read_bytes()
    value = D.parsed(raw)
    D.require(value.get("profile") == "uuid-local-u06" and value.get("group") == generation,
              "CLOSED_RECEIVER_PROFILE_REQUIRED")
    directory = BASE / ("handoff-" + generation)
    D.private_dir(str(BASE))
    D.require(not directory.exists() and not directory.is_symlink(), "NEW_HANDOFF_REQUIRED")
    directory.mkdir(mode=0o700)
    D.require(value.get("report") == str(directory / "verification.json"), "FIXED_VERIFICATION_REPORT_REQUIRED")
    cfg = None
    try:
        cfg = D.configuration(path)
        result = D.verify(cfg)
        D.require(result["full_reconciliation"] == "passed" and len(result["databases"]) == 3,
                  "COMPLETE_LIVE_RECONCILIATION_REQUIRED")
        D.write_json(cfg["report"], result)
        D.write_json(directory / "verification-config.json", value)
        D.admission(cfg, generation)
        databases = cfg["reg"]["groups"][generation]
        # The registered local PostgreSQL 18 host supplies the import batch ID.
        session = D.Session(databases[0]["name"], cfg["work"])
        try:
            batch = session.one("SELECT to_json(uuidv7()::text);")
            session.close()
        except Exception:
            session.abort()
            raise
        parsed_batch = D.uuid.UUID(batch)
        D.require(str(parsed_batch) == batch and parsed_batch.version == 7
                  and parsed_batch.variant == D.uuid.RFC_4122, "V7_IMPORT_BATCH_REQUIRED")
        marker = "saas-uuid-import/1/" + batch + "/" + cfg["input"]["sha256"]
        D.write_json(directory / "marker-intent.json", {
            "generation": generation, "marker": marker,
            "registration_sha256": cfg["registration_sha256"],
            "verification_sha256": hashlib.sha256(Path(cfg["report"]).read_bytes()).hexdigest(),
            "input": cfg["input"], "targets": databases,
        })
        targets = {}
        for db in databases:
            D.admission(cfg, generation)
            session = D.Session(db["name"], cfg["work"], write=True)
            try:
                D.inspect(session, db, cfg)
                # Only fresh receivers may acquire a marker. Replays remain isolated
                # until the final T1 reconciliation above has succeeded for all DBs.
                guard = ("DO $u06_handoff$ BEGIN IF current_user<>'postgres' OR session_user<>'postgres' "
                         "OR inet_server_addr() IS NOT NULL OR current_database()<>" + D.literal(db["name"]) +
                         " OR NOT EXISTS(SELECT FROM pg_database WHERE oid=" + str(db["oid"]) +
                         " AND datname=current_database() AND datdba=" + str(db["owner_oid"]) +
                         " AND shobj_description(oid,'pg_database') IS NULL) THEN RAISE EXCEPTION "
                         "'U06 receiver identity or marker changed'; END IF; END; $u06_handoff$;")
                session.query(guard)
                session.query("COMMENT ON DATABASE " + D.ident(db["name"]) + " IS " + D.literal(marker) + ";")
                D.require(session.one("SELECT to_json(shobj_description(oid,'pg_database')) FROM pg_database "
                                      "WHERE datname=current_database();") == marker, "MARKER_READBACK_FAILED")
                session.close(commit=True)
            except Exception:
                session.abort()
                raise
            targets[db["kind"]] = {
                "database": db["name"], "database_oid": db["oid"], "owner": db["owner"],
                "owner_oid": db["owner_oid"], "schema_sha256": db["expected_schema_sha256"],
                "import_marker": marker,
            }
        D.admission(cfg, generation)
        receipt = {"format": "saas-uuid-u06-target-receipts/1", "generation": generation, "targets": targets}
        D.write_json(directory / "target-receipt.json", receipt)
        print(json.dumps({"status": "LIVE_RECONCILED_RECEIVERS_MARKED", "generation": generation,
                          "databases": 3, "rows": sum(d["rows"] for d in result["databases"]),
                          "credential_tables_empty": 6, "services_opened": False}))
    except Exception:
        D.write_json(directory / "failed.json", {"status": "FAILED_KEEP_RECEIVERS_ISOLATED"})
        raise
    finally:
        if cfg:
            shutil.rmtree(cfg["work"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--generation", choices=("final", "recovery"), required=True)
    parser.add_argument("--config", required=True)
    args = parser.parse_args()
    D.require(sys.platform == "linux" and os.geteuid() == 0, "WSL_ROOT_REQUIRED")
    os.umask(0o077)
    handoff(args.config, args.generation)


if __name__ == "__main__":
    try:
        main()
    except D.Invalid as error:
        print("U06_HANDOFF_REJECTED:" + str(error), file=sys.stderr)
        sys.exit(1)
    except Exception:
        print("U06_HANDOFF_REJECTED:PRIVATE_OPERATION_FAILED_KEEP_ISOLATED", file=sys.stderr)
        sys.exit(1)
