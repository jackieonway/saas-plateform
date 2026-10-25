#!/usr/bin/env python3
"""Fixed U06 final/recovery runtime control; no arbitrary launch argv or restart.

--check-code is offline. --check validates already prepared runtime materials.
--start, --capture and --freeze each require a fresh private evidence directory.
Only an explicit --start or --freeze mutates Docker or runtime role admission.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import runpy
import socket
import stat
import subprocess
import sys
import time

SCRIPT = Path(__file__).resolve()
C = runpy.run_path(str(SCRIPT.with_name("uuid-u06-control.py")))
P = runpy.run_path(str(SCRIPT.with_name("prepare-uuid-u06-runtime.py")))
E = runpy.run_path(str(SCRIPT.with_name("prepare-uuid-u06-epoch.py")))
require, Invalid = C["require"], C["Invalid"]
FORMAT = "saas-uuid-u06-switch/1"
BASE = Path("/var/lib/saas-uuid-u06-switch-20261024")
PORTS = (13000, 13001, 13002, 13007, 13008, 13010, 13011)
SERVICES = (*P["SERVICES"], "gateway")
HELPERS = {name: C["sha_file"](SCRIPT.with_name(name)) for name in
           ("uuid-u06-control.py", "prepare-uuid-u06-runtime.py", "prepare-uuid-u06-epoch.py",
            "prepare-uuid-u05-runtime.py", "verify-uuid-u05-databases.py", "uuid-u06-data.py")}


def fixed_name(generation, service):
    require(generation in P["GENERATIONS"] and service in SERVICES, "FIXED_RUNTIME_IDENTITY_REQUIRED")
    return f"qiankun-uuid-u06-{generation}-{service.replace('_', '-')}-20261024"


def private_json(path):
    C["private"](path.parent, True)
    C["private"](path)
    require(path.stat().st_size <= 16 * 1024 * 1024, "PRIVATE_RECEIPT_TOO_LARGE")
    return C["AUDIT"]["parsed"](path.read_bytes())


def checked_plan(root, manifest, saved):
    generation = manifest["generation"]
    require(generation in P["GENERATIONS"] and root == P["BASE"] / generation, "FIXED_LAUNCH_ROOT_REQUIRED")
    require(manifest["images"] == {key: {"name": value[0], "id": value[1]} for key, value in P["IMAGES"].items()}, "FIXED_LAUNCH_IMAGES_REQUIRED")
    require(set(manifest["services"]) == set(P["SERVICES"])
            and all(manifest["services"][s]["binary"]["sha256"] == C["RUNTIME_BINARIES"][s] for s in P["SERVICES"]), "FIXED_LAUNCH_BINARIES_REQUIRED")
    plan = P["launch_plan"](root, manifest)
    require(saved == plan, "SAVED_LAUNCH_PLAN_CHANGED")
    return plan


def material(generation):
    require(generation in P["GENERATIONS"], "FIXED_GENERATION_REQUIRED")
    root = P["BASE"] / generation
    C["private"](P["BASE"], True)
    C["private"](root, True)
    prepared = private_json(root / "prepared.json")
    P["fields"](prepared, ("format", "status", "generation", "services", "frontends", "credential_epoch_sha256", "receipt_sha256", "source_sessions_loaded", "state_rebuilt"))
    require(prepared["format"] == P["FORMAT"] and prepared["status"] == "PREPARED_NOT_STARTED"
            and prepared["generation"] == generation and prepared["services"] == 6 and prepared["frontends"] == 8
            and prepared["source_sessions_loaded"] is False and prepared["state_rebuilt"] is False, "PREPARED_RUNTIME_REQUIRED")
    receipt, _ = P["checked_json"](str(root / "imported-receipts.json"), prepared["receipt_sha256"])
    P["receipts"](receipt, generation)
    epoch, _ = P["checked_json"](str(root / "credential-epoch.json"), prepared["credential_epoch_sha256"])
    require(receipt["epoch_sha256"] == prepared["credential_epoch_sha256"], "RUNTIME_EPOCH_COMMITMENT_MISMATCH")
    P["validate_epoch"](epoch, generation, P["predecessor_epoch"]() if generation == "recovery" else None)
    manifest = private_json(root / "input-manifest.json")
    P["fields"](manifest, ("format", "profile", "generation", "output_dir", "images", "receipt_file", "receipt_sha256", "policy_file", "policy_sha256", "frontend_manifest", "frontend_manifest_sha256", "services"))
    require(manifest["format"] == P["FORMAT"] and manifest["profile"] == "uuid-local-preview"
            and manifest["generation"] == generation and manifest["output_dir"] == str(root)
            and manifest["receipt_sha256"] == prepared["receipt_sha256"], "RUNTIME_MANIFEST_GENERATION_INVALID")
    require(manifest["images"] == {key: {"name": value[0], "id": value[1]} for key, value in P["IMAGES"].items()}, "RUNTIME_IMAGES_INVALID")
    P["fields"](manifest["services"], P["SERVICES"])
    P["binaries"](manifest)
    epoch_dir = E["BASE"] / ("epoch-" + generation)
    journal, target_receipt = E["load_journal"](str(epoch_dir))
    activated = private_json(epoch_dir / "activate" / "complete.json")
    require(activated["status"] == "ACTIVE_EPOCH_ROLES_NOLOGIN" and activated["generation"] == generation
            and activated["epoch_sha256"] == journal["epoch_sha256"] == prepared["credential_epoch_sha256"], "ACTIVATED_EPOCH_REQUIRED")
    require(set(activated["roles"]) == set(E["runtime_roles"](generation).values())
            and all(type(v) is int and v > 0 for v in activated["roles"].values())
            and len(set(activated["roles"].values())) == 4, "ACTIVATED_RUNTIME_ROLE_OIDS_INVALID")
    for service in P["SERVICES"]:
        expected_file = epoch_dir / (service + ".json")
        require(manifest["services"][service]["env_file"] == str(expected_file)
                and C["sha_file"](expected_file) == journal["service_env_sha256"][service], "PREPARED_EPOCH_ENV_CHANGED")
    environments = P["service_environments"](manifest, receipt, P["policy"](manifest), epoch)
    fingerprints = []
    for service in P["SERVICES"]:
        binary = root / "bin" / service
        info = binary.lstat()
        require(not binary.is_symlink() and stat.S_ISREG(info.st_mode) and info.st_uid == 0 and info.st_nlink == 1
                and stat.S_IMODE(info.st_mode) == 0o700 and C["sha_file"](binary) == C["RUNTIME_BINARIES"][service], "PREPARED_BINARY_CHANGED")
        env_file = root / "env" / (service + ".env")
        C["private"](env_file)
        expected = "".join(k + "=" + v + "\n" for k, v in sorted(environments[service].items())).encode()
        require(env_file.read_bytes() == expected, "PREPARED_RUNTIME_ENV_CHANGED")
        C["private"](root / "state" / service, True)
        fingerprints.append({"service": service, "binary_sha256": C["sha_file"](binary), "env_sha256": C["sha_file"](env_file)})
    frontend = private_json(root / "frontends.json")
    frontend_input, _ = P["checked_json"](manifest["frontend_manifest"], manifest["frontend_manifest_sha256"], 16 * 1024 * 1024)
    require(frontend == frontend_input, "PREPARED_FRONTEND_DESCRIPTOR_CHANGED")
    artifacts = P["reviewed_frontends"](frontend)
    expected_files = {app + "/" + f["path"]: f for app, a in artifacts.items() for f in a["files"]}
    actual_files = set()
    for path in (root / "static").rglob("*"):
        require(not path.is_symlink(), "PREPARED_STATIC_SYMLINK")
        if path.is_file():
            relative = path.relative_to(root / "static").as_posix()
            require(relative in expected_files and path.stat().st_uid == 0 and stat.S_IMODE(path.stat().st_mode) == 0o644
                    and path.stat().st_size == expected_files[relative]["bytes"]
                    and C["sha_file"](path) == expected_files[relative]["sha256"], "PREPARED_FRONTEND_CHANGED")
            actual_files.add(relative)
        else:
            require(path.is_dir() and path.stat().st_uid == 0 and stat.S_IMODE(path.stat().st_mode) == 0o755, "PREPARED_STATIC_DIRECTORY_INVALID")
    require(actual_files == set(expected_files), "PREPARED_FRONTEND_FILE_SET_CHANGED")
    for file, expected in (("runner.py", P["RUNNER"].encode()), ("empty.env", b"")):
        C["private"](root / file)
        require((root / file).read_bytes() == expected, "PREPARED_RUNNER_OR_EMPTY_ENV_CHANGED")
    nginx = root / "nginx.conf"
    require(not nginx.is_symlink() and nginx.stat().st_uid == 0 and stat.S_IMODE(nginx.stat().st_mode) == 0o644
            and nginx.read_text() == P["U05"].nginx_configuration(artifacts).replace(r'return 200 "u05\n";', r'return 200 "u06\n";'), "PREPARED_NGINX_CHANGED")
    # Reconstruct from fixed code/constants; saved JSON is only compared, never executed.
    plan = checked_plan(root, manifest, private_json(root / "launch-plan.json"))
    commitment = {"generation": generation, "prepared_sha256": C["sha_file"](root / "prepared.json"),
                  "manifest_sha256": C["sha_file"](root / "input-manifest.json"), "launch_plan_sha256": C["AUDIT"]["digest"](plan),
                  "nginx_sha256": C["sha_file"](nginx), "runner_sha256": C["sha_file"](root / "runner.py"),
                  "static": C["mount_content"](root / "static"), "services": fingerprints,
                  "epoch_journal_sha256": C["sha_file"](epoch_dir / "prepared.json"),
                  "epoch_activation_sha256": C["sha_file"](epoch_dir / "activate" / "complete.json")}
    return {"root": root, "receipt": receipt, "target_receipt": target_receipt, "roles": activated["roles"],
            "plan": plan, "commitment": commitment}


class ReadonlyRunner:
    def sql(self, database, statement):
        result = subprocess.run(C["pg_command"](database), input="SET search_path=pg_catalog; SET timezone='UTC';\n" + statement,
                                capture_output=True, text=True, env=C["environment"](), timeout=150)
        require(result.returncode == 0 and len(result.stdout.splitlines()) == 1, "READONLY_RUNTIME_PROOF_FAILED")
        return C["AUDIT"]["parsed"](result.stdout.strip())


def database_proof(generation, prepared, sealed, login, idle):
    require(C["cluster"]() == sealed["cluster"], "SWITCH_CLUSTER_GENERATION_CHANGED")
    runner, targets = ReadonlyRunner(), []
    for kind in C["KINDS"]:
        known = next(t for t in sealed["targets"] if t["set"] == generation and t["kind"] == kind)
        target = prepared["target_receipt"]["targets"][kind]
        require((target["database"], target["database_oid"], target["owner"], target["owner_oid"], target["schema_sha256"])
                == (known["name"], known["oid"], known["owner"], known["owner_oid"], known["raw_schema_sha256"]), "EPOCH_SEALED_TARGET_MISMATCH")
        require(prepared["receipt"]["targets"][kind]["import_marker"] == target["import_marker"], "RUNTIME_IMPORT_MARKER_MISMATCH")
        value = runner.sql(known["name"], "BEGIN READ ONLY; " + E["identity_sql"]() + " COMMIT;")
        require(all(value[k] == target[k] for k in ("database", "database_oid", "owner", "owner_oid", "import_marker"))
                and value["maintenance"] and value["owner_nologin"] and value["owner_safe"] and value["database_readonly"]
                and not value["public_connect"] and value["prepared"] == 0 and (not idle or value["backends"] == 0), "SWITCH_DATABASE_IDENTITY_OR_FENCE_CHANGED")
        material_value = C["sql"](C["AUDIT"]["SCHEMA_FILES"][kind].read_text(), known["name"])[0]
        require(C["AUDIT"]["digest"](material_value) == known["raw_schema_sha256"]
                and C["DATA"]["value_sha"](C["DATA"]["semantic_material"](material_value)) == known["semantic_schema_sha256"], "SWITCH_SCHEMA_CHANGED")
        history = C["sql"]("SELECT jsonb_agg(jsonb_build_object('version',version,'success',success,'checksum_sha384',encode(checksum,'hex')) ORDER BY version) FROM public._sqlx_migrations;", known["name"])[0]
        require(C["AUDIT"]["digest"](history) == C["HISTORY_SHA256"][kind], "SWITCH_MIGRATION_HISTORY_CHANGED")
        require(value["tables"] == C["DATA"]["TABLES"][kind], "SWITCH_TABLE_SET_CHANGED")
        E["privilege_proof"](runner, kind, generation, value["tables"])
        targets.append(value)
    roles = E["cluster_proof"](runner, generation, prepared["roles"], login)["roles"]
    closure = C["source_admission"](targets, roles, not login)
    # U05 and the inactive U06 generation may never retain a LOGIN runtime role.
    other_names = set(C["AUDIT"]["runtime_roles"](C["AUDIT"]["load_public_inputs"]()[0]).values())
    other_names.update(E["runtime_roles"]("recovery" if generation == "final" else "final").values())
    require(not any(r["name"] in other_names and r["login"] for r in closure["roles"]), "OTHER_GENERATION_RUNTIME_LOGIN_ENABLED")
    return {"targets": targets, "roles": roles, "closure": closure}


def all_containers():
    identifiers = C["docker"](["ps", "-aq"]).split()
    require(all(re.fullmatch(r"[0-9a-f]{12,64}", i) for i in identifiers), "DOCKER_INVENTORY_INVALID")
    return C["AUDIT"]["parsed"](C["docker"](["inspect", *identifiers])) if identifiers else []


def mutually_exclusive(generation, own_allowed):
    own, others = [], []
    for item in all_containers():
        name = item["Name"].removeprefix("/")
        labels = item["Config"].get("Labels") or {}
        managed = name.startswith("qiankun-uuid-u05-") or name.startswith("qiankun-uuid-u06-") or labels.get("saas.uuid-stage") in ("U05", "U06")
        if not managed:
            continue
        if name in {fixed_name(generation, s) for s in SERVICES}:
            own.append(item)
        else:
            require(not item["State"]["Running"] and not item["State"]["Paused"] and not item["State"]["Restarting"], "OTHER_GENERATION_CONTAINER_RUNNING")
            others.append({"id": item["Id"], "name": name, "running": False})
    require(own_allowed or not own, "FRESH_GENERATION_CONTAINER_NAMES_REQUIRED")
    return others


def container_record(generation, service, prepared, running=True, verify_process=True):
    name = fixed_name(generation, service)
    item = C["AUDIT"]["parsed"](C["docker"](["inspect", name]))[0]
    root, labels = prepared["root"], item["Config"].get("Labels") or {}
    gateway = service == "gateway"
    expected_image = P["IMAGES"]["gateway" if gateway else "runtime"][1]
    expected_labels = {"saas.uuid-stage": "U06", "saas.uuid-generation": generation,
                       "saas.uuid-runtime": "20261024", "saas.uuid-service": service}
    require(item["Name"] == "/" + name and re.fullmatch(r"[0-9a-f]{64}", item["Id"])
            and item["Image"] == expected_image and all(labels.get(k) == v for k, v in expected_labels.items())
            and (running is None or item["State"]["Running"] is running) and not item["State"]["Paused"] and not item["State"]["Restarting"]
            and item["HostConfig"]["NetworkMode"] == "host" and item["HostConfig"]["ReadonlyRootfs"]
            and item["HostConfig"]["Init"] and item["HostConfig"]["CapDrop"] == ["ALL"]
            and "no-new-privileges" in item["HostConfig"]["SecurityOpt"]
            and item["HostConfig"]["RestartPolicy"]["Name"] == "no" and not item["HostConfig"]["AutoRemove"]
            and item["HostConfig"]["LogConfig"]["Type"] == "none"
            and item["HostConfig"]["Tmpfs"] == {"/tmp": "rw,noexec,nosuid,nodev,size=64m"}
            and not item["HostConfig"]["Privileged"] and not item["HostConfig"].get("CapAdd")
            and not item["HostConfig"].get("Devices") and not item["HostConfig"].get("DeviceRequests")
            and not item["HostConfig"].get("Binds") and item["HostConfig"]["PidMode"] == ""
            and item["Config"]["OpenStdin"] is (service == "examples_chat") and not item["Config"]["Tty"], "U06_CONTAINER_POLICY_CHANGED")
    required = {"/srv/frontends": (str(root / "static"), False), "/etc/nginx/nginx.conf": (str(root / "nginx.conf"), False)} if gateway else {
        "/run/u06/service": (str(root / "bin" / service), False), "/run/u06/service.env": (str(root / "env" / (service + ".env")), False),
        "/run/u06/runner.py": (str(root / "runner.py"), False), "/run/u06/empty.env": (str(root / "empty.env"), False),
        "/run/u06/state": (str(root / "state" / service), True)}
    mounts = sorted(({k: m[k] for k in ("Type", "Source", "Destination", "RW")} for m in item["Mounts"]), key=lambda m: m["Destination"])
    require(len(mounts) == len(required) and {m["Destination"] for m in mounts} == set(required)
            and all(m["Type"] == "bind" and (m["Source"], m["RW"]) == required[m["Destination"]] for m in mounts), "U06_CONTAINER_MOUNTS_CHANGED")
    require(item["Config"]["Entrypoint"] == (["/usr/sbin/nginx"] if gateway else ["/usr/bin/python3"])
            and item["Config"]["Cmd"] == (["-g", "daemon off;"] if gateway else ["-I", "/run/u06/runner.py", C["RUNTIME_BINARIES"][service]])
            and item["Config"]["User"] == ("101:101" if gateway else ""), "U06_CONTAINER_ENTRYPOINT_CHANGED")
    proof = {"service": service, "name": name, "id": item["Id"], "image": item["Image"], "mounts": mounts,
             "running": item["State"]["Running"], "pid": item["State"]["Pid"], "container_started_at": item["State"]["StartedAt"],
             "restart_count": item["RestartCount"], "config_sha256": C["AUDIT"]["digest"](item["Config"]),
             "host_config_sha256": C["AUDIT"]["digest"](item["HostConfig"]), "processes": []}
    if item["State"]["Running"] and verify_process:
        pending = [proof["pid"]]
        for pid in pending:
            proof["processes"].append(C["process_proof"](pid))
            pending.extend(int(v) for v in Path(f"/proc/{pid}/task/{pid}/children").read_text().split())
            require(len(pending) <= 10, "U06_PROCESS_TREE_UNEXPECTED")
        if not gateway:
            require(sum(p["exe_sha256"] == C["RUNTIME_BINARIES"][service] for p in proof["processes"]) == 1, "U06_RUNNING_BINARY_CHANGED")
    elif not item["State"]["Running"]:
        require(proof["pid"] == 0, "U06_STOPPED_CONTAINER_HAS_PID")
    return proof


class Phase:
    def __init__(self, directory, action, generation):
        require(directory.parent == BASE and not directory.exists() and not directory.is_symlink(), "FRESH_SWITCH_DIRECTORY_REQUIRED")
        directory.mkdir(mode=0o700)
        C["fsync_dir"](BASE)
        self.directory, self.started = directory, C["now"]()
        self.script_sha256 = C["sha_file"](SCRIPT)
        self.journal = os.open(directory / "events.ndjson", os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        os.fsync(self.journal)
        C["write_new"](directory / "intent.json", {"format": FORMAT, "action": action, "generation": generation,
                        "started_at_utc": self.started, "script_sha256": self.script_sha256, "helpers": HELPERS})

    def event(self, operation, **fields):
        raw = json.dumps({"at_utc": C["now"](), "operation": operation, **fields}, sort_keys=True).encode() + b"\n"
        require(os.write(self.journal, raw) == len(raw), "SWITCH_JOURNAL_SHORT_WRITE")
        os.fsync(self.journal)

    def close(self):
        os.close(self.journal)


def code_guard(phase):
    require(C["sha_file"](SCRIPT) == phase.script_sha256
            and all(C["sha_file"](SCRIPT.with_name(name)) == digest for name, digest in HELPERS.items()), "SWITCH_CODE_CHANGED")


def fence_roles(prepared, phase):
    statements = ["BEGIN READ WRITE;"]
    for name, oid in sorted(prepared["roles"].items()):
        statements += [C["role_guard"]({"name": name, "oid": oid}), "ALTER ROLE " + C["ident"](name) + " NOLOGIN;"]
    statements.append("COMMIT;")
    phase.event("exact_runtime_nologin_intent", roles=4)
    C["sql"]("\n".join(statements), readonly=False, phase=phase)


def quarantine_roles(prepared, phase):
    """Exact identity guard permits an already NOLOGIN role during cleanup."""
    statements = ["BEGIN READ WRITE;"]
    for name, oid in sorted(prepared["roles"].items()):
        require(type(oid) is int and oid > 0, "QUARANTINE_ROLE_OID_INVALID")
        statements += [f"DO $guard$ BEGIN IF NOT EXISTS(SELECT FROM pg_roles WHERE oid={oid} "
                       f"AND rolname={C['literal'](name)} AND NOT (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls)) "
                       "THEN RAISE EXCEPTION 'quarantine runtime role identity changed'; END IF; END; $guard$;",
                       "ALTER ROLE " + C["ident"](name) + " NOLOGIN;"]
    statements.append("COMMIT;")
    phase.event("quarantine_exact_runtime_nologin_intent", roles=4)
    C["sql"]("\n".join(statements), readonly=False, phase=phase)


def quarantine_generation(generation, prepared, sealed, phase, known_records, attempted):
    """Every cleanup step is attempted independently; incomplete closure is explicit."""
    results = []

    def step(operation, function, **identity):
        phase.event("quarantine_step_intent", step=operation, **identity)
        try:
            function()
            result = {"step": operation, "status": "PASSED", **identity}
        except BaseException as error:
            result = {"step": operation, "status": "FAILED", "category": type(error).__name__, **identity}
        results.append(result)
        phase.event("quarantine_step_result", **result)

    def cluster_guard():
        require(C["cluster"]() == sealed["cluster"], "QUARANTINE_CLUSTER_CHANGED")

    def fence():
        cluster_guard()
        quarantine_roles(prepared, phase)

    step("runtime_nologin", fence)
    candidates = {record["service"]: {"service": record["service"], "id": record["id"]} for record in known_records}
    for service in attempted:
        if service not in candidates:
            def discover(service=service):
                # An interrupted docker run may have created its fixed fresh name
                # without returning an ID. Inventory failure is never treated as absence.
                found = [item for item in all_containers() if item["Name"] == "/" + fixed_name(generation, service)]
                require(len(found) <= 1, "QUARANTINE_DUPLICATE_NAME")
                if found:
                    checked = container_record(generation, service, prepared, running=None, verify_process=False)
                    require(checked["id"] == found[0]["Id"], "QUARANTINE_DISCOVERED_CONTAINER_CHANGED")
                    candidates[service] = {"service": service, "id": checked["id"]}
            step("discover_attempted_id", discover, service=service)
    for service, record in reversed(list(candidates.items())):
        def stop(service=service, record=record):
            identifier = record["id"]
            require(re.fullmatch(r"[0-9a-f]{64}", identifier), "QUARANTINE_CONTAINER_ID_INVALID")
            current = C["AUDIT"]["parsed"](C["docker"](["inspect", identifier]))[0]
            labels = current["Config"].get("Labels") or {}
            require(current["Id"] == identifier and current["Name"] == "/" + fixed_name(generation, service)
                    and current["Image"] == P["IMAGES"]["gateway" if service == "gateway" else "runtime"][1]
                    and all(labels.get(k) == v for k, v in {"saas.uuid-stage": "U06", "saas.uuid-generation": generation,
                            "saas.uuid-runtime": "20261024", "saas.uuid-service": service}.items()), "QUARANTINE_CONTAINER_IDENTITY_CHANGED")
            C["docker"](["stop", "--time", "10", identifier])
            after = C["AUDIT"]["parsed"](C["docker"](["inspect", identifier]))[0]
            require(after["Id"] == identifier and not after["State"]["Running"] and not after["State"]["Paused"]
                    and not after["State"]["Restarting"] and after["State"]["Pid"] == 0, "QUARANTINE_CONTAINER_STILL_RUNNING")
        step("stop_exact_id", stop, service=service, container_id=record["id"])
    for target in prepared["target_receipt"]["targets"].values():
        def idle(target=target):
            cluster_guard()
            C["target_idle"]({"name": target["database"], "oid": target["database_oid"], "owner_oid": target["owner_oid"]}, phase, terminate=True)
        step("target_idle", idle, database=target["database"], database_oid=target["database_oid"])
    step("fresh_database_nologin_idle_closure", lambda: database_proof(generation, prepared, sealed, False, True))
    step("old_u05_sources_still_fenced", lambda: C["source_generation"](True))

    def stopped_inventory():
        mutually_exclusive(generation, True)
        require(not any(item["Name"] in {"/" + fixed_name(generation, s) for s in SERVICES}
                        and (item["State"]["Running"] or item["State"]["Paused"] or item["State"]["Restarting"])
                        for item in all_containers()), "QUARANTINE_GENERATION_STILL_RUNNING")

    step("all_generations_stopped", stopped_inventory)
    complete = all(result["status"] == "PASSED" for result in results)
    report = {"format": FORMAT, "generation": generation, "recorded_at_utc": C["now"](),
              "status": "QUARANTINED_NOLOGIN_IDLE" if complete else "FAILED_CLEANUP_INCOMPLETE", "steps": results}
    C["write_new"](phase.directory / "quarantine.json", report)
    return report["status"]


def publish(phase, value):
    code_guard(phase)
    receipt = {"format": FORMAT, "script_sha256": phase.script_sha256, "helpers": HELPERS,
               "recorded_at_utc": C["now"](), "started_at_utc": phase.started, **value}
    digest = C["write_new"](phase.directory / "receipt.json", receipt)
    phase.event("receipt_published", status=receipt["status"], receipt_sha256=digest)
    print(json.dumps({"status": receipt["status"], "generation": receipt["generation"], "receipt": str(phase.directory / "receipt.json"), "receipt_sha256": digest}, separators=(",", ":")))


def run(args):
    require(sys.platform == "linux" and os.geteuid() == 0, "WSL_ROOT_REQUIRED")
    if args.check_code:
        print(json.dumps({"status": "OFFLINE_CODE_READY", "helpers": HELPERS, "generations": list(P["GENERATIONS"])}))
        return
    previous = private_json(args.receipt) if args.freeze else None
    if previous:
        require(previous["format"] == FORMAT and previous["status"] in ("CAPTURED_RUNNING", "STARTED_NOT_HEALTH_VERIFIED")
                and previous["script_sha256"] == C["sha_file"](SCRIPT) and previous["helpers"] == HELPERS, "FIXED_SWITCH_CAPTURE_REQUIRED")
    generation = previous["generation"] if previous else args.generation
    prepared = material(generation)
    if args.check:
        print(json.dumps({"status": "PREPARED_MATERIAL_CHECKED_NO_DATABASE_OR_DOCKER_ACCESS", "generation": generation,
                          "material_sha256": C["AUDIT"]["digest"](prepared["commitment"])}))
        return
    sealed_path = Path(previous["sealed_receipt"]) if previous else args.sealed_receipt
    sealed, sealed_sha = C["read_receipt"](sealed_path)
    require(sealed["stage"] == "SEALED", "SEALED_SIX_TARGET_RECEIPT_REQUIRED")
    if previous:
        require(sealed_sha == previous["sealed_receipt_sha256"] and prepared["commitment"] == previous["material"], "SWITCH_CAPTURE_MATERIAL_CHANGED")
    if not BASE.exists():
        BASE.mkdir(mode=0o700)
        C["fsync_dir"](BASE.parent)
    C["private"](BASE, True)
    require(args.output is not None, "FRESH_SWITCH_OUTPUT_REQUIRED")
    phase = Phase(args.output, "freeze" if args.freeze else "start" if args.start else "capture", generation)
    lock, created, attempted, freeze_records, mutation_started = None, [], [], [], False
    try:
        lock = C["Advisory"](phase)
        lock.process.stdin.write("SELECT pg_try_advisory_lock(823140620261025);\n")
        lock.process.stdin.flush()
        require(lock.process.stdout.readline().strip() == "t", "EPOCH_ADMISSION_LOCK_BUSY")
        proof = database_proof(generation, prepared, sealed, True, bool(args.start))
        C["source_generation"](True)
        others = mutually_exclusive(generation, not args.start)
        common = {"generation": generation, "cluster": sealed["cluster"], "sealed_receipt": str(sealed_path),
                  "sealed_receipt_sha256": sealed_sha, "material": prepared["commitment"], "other_generations_stopped": others}
        if args.start:
            sockets = []
            try:
                for port in PORTS:
                    handle = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
                    sockets.append(handle)
                    handle.bind(("127.0.0.1", port))
            finally:
                for handle in sockets:
                    handle.close()
            require(all(not any((prepared["root"] / "state" / service).iterdir()) for service in P["SERVICES"]), "FRESH_EMPTY_RUNTIME_STATE_REQUIRED")
            for command in prepared["plan"]["commands"]:
                code_guard(phase)
                lock.alive()
                mutually_exclusive(generation, True)
                service = command["service"]
                phase.event("fixed_container_start_intent", service=service, launch_plan_sha256=C["AUDIT"]["digest"](prepared["plan"]))
                mutation_started = True
                attempted.append(service)
                identifier = C["docker"](command["argv"][1:]).strip()
                require(re.fullmatch(r"[0-9a-f]{64}", identifier), "STARTED_CONTAINER_ID_INVALID")
                created.append({"service": service, "id": identifier})
                deadline = time.monotonic() + 30
                while True:
                    try:
                        observed = container_record(generation, service, prepared)
                        require(observed["id"] == identifier, "STARTED_CONTAINER_ID_CHANGED")
                        break
                    except (Invalid, FileNotFoundError, ProcessLookupError):
                        require(time.monotonic() < deadline, "STARTED_CONTAINER_NOT_READY")
                        time.sleep(0.1)
                phase.event("fixed_container_started", service=service, container_id=identifier)
            records = [container_record(generation, s, prepared) for s in SERVICES]
            proof = database_proof(generation, prepared, sealed, True, False)
            publish(phase, {**common, "status": "STARTED_NOT_HEALTH_VERIFIED", "containers": records, "database_proof": proof})
        elif args.capture:
            records = [container_record(generation, s, prepared) for s in SERVICES]
            publish(phase, {**common, "status": "CAPTURED_RUNNING", "containers": records, "database_proof": proof})
        else:
            records = [container_record(generation, s, prepared) for s in SERVICES]
            require(records == previous["containers"] and proof["roles"] == previous["database_proof"]["roles"], "CAPTURED_RUNNING_GENERATION_CHANGED")
            freeze_records = records
            started, clock = C["now"](), time.monotonic()
            phase.event("downtime_started", started_at_utc=started, budget_seconds=600)
            phase.event("freeze_mutation_intent", exact_containers=[r["id"] for r in records], exact_roles=prepared["roles"])
            mutation_started = True
            by_service = {record["service"]: record for record in records}
            for service in C["STOP_ORDER"]:
                code_guard(phase)
                lock.alive()
                require(C["cluster"]() == sealed["cluster"], "CLUSTER_CHANGED_DURING_STOP")
                require(container_record(generation, service, prepared) == by_service[service], "RUNTIME_CHANGED_DURING_STOP")
                phase.event("stop_exact_container_intent", service=service, container_id=by_service[service]["id"])
                C["docker"](["stop", "--time", "30", by_service[service]["id"]])
            fence_roles(prepared, phase)
            for target in proof["targets"]:
                C["target_idle"]({"name": target["database"], "oid": target["database_oid"], "owner_oid": target["owner_oid"]}, phase, terminate=True)
            after = database_proof(generation, prepared, sealed, False, True)
            stopped = [container_record(generation, s, prepared, False) for s in SERVICES]
            strip = lambda record: {k: v for k, v in record.items() if k not in ("running", "pid", "processes")}
            require(list(map(strip, stopped)) == list(map(strip, records)), "STOPPED_RUNTIME_GENERATION_CHANGED")
            C["source_generation"](True)
            mutually_exclusive(generation, True)
            publish(phase, {**common, "status": "FROZEN_NOLOGIN_IDLE", "containers": stopped, "database_proof": after,
                            "downtime_started_at_utc": started, "freeze_elapsed_seconds": time.monotonic() - clock})
    except BaseException:
        cleanup = "NO_RUNTIME_MUTATION_ATTEMPTED"
        try:
            if mutation_started:
                cleanup = "FAILED_CLEANUP_INCOMPLETE"
                try:
                    cleanup = quarantine_generation(generation, prepared, sealed, phase, freeze_records or created, attempted)
                except BaseException as cleanup_error:
                    phase.event("quarantine_evidence_incomplete", category=type(cleanup_error).__name__)
        finally:
            C["write_new"](phase.directory / "failure.json", {"format": FORMAT,
                            "status": "FAILED_CLEANUP_INCOMPLETE" if cleanup == "FAILED_CLEANUP_INCOMPLETE" else "FAILED_RETAIN_PRIVATE_EVIDENCE", "generation": generation,
                            "recorded_at_utc": C["now"](), "cleanup": cleanup,
                            "automatic_source_restart": False, "automatic_container_or_database_removal": False})
        raise
    finally:
        if lock:
            lock.close()
        phase.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_mutually_exclusive_group(required=True)
    for name in ("check-code", "check", "start", "capture", "freeze"):
        actions.add_argument("--" + name, action="store_true")
    parser.add_argument("--generation", choices=P["GENERATIONS"])
    parser.add_argument("--sealed-receipt", type=Path)
    parser.add_argument("--receipt", type=Path)
    parser.add_argument("--output", type=Path)
    try:
        run(parser.parse_args())
    except BaseException as error:
        if isinstance(error, SystemExit):
            raise
        print(str(error) if isinstance(error, Invalid) else "U06_SWITCH_FAILED_RETAIN_PRIVATE_EVIDENCE", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
