"""Owned offline restore orchestration. Only an environment owner invokes it.

This script never starts applications, changes HBA, rotates DB roles or drops a
database. Child restore commands create only their exact quarantined targets.
Elapsed time covers this cycle, not the complete fault-to-new-business RTO.
"""
import argparse
import contextlib
import datetime
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
OPS = HERE.parents[1] / "ops/identity-recovery"
sys.path.insert(0, str(OPS))
import core


EXPECTED = {
    "restore-new": "RESTORED_QUARANTINED",
    "reconcile": "RECONCILED_QUARANTINED",
    "adapter-rearm": "PHASE23_RECOVERY_REARM_FILES_READY",
    "rearm": "REARMED_QUARANTINED",
    "gate": "GATE_PASSED_QUARANTINED",
}
SAFE = {"status", "sequence", "databaseCount", "catalogsEqual", "materialsEqual",
        "oldCredentialsInvalidated", "fencedInventorySha256", "users", "clients", "services",
        "memberships", "runtimePasswords", "materials", "sourceUnchanged", "executesDatabaseWrites",
        "opensTraffic", "expectedConnectionLimits"}


def require(value, code):
    core.need(value, "CYCLE_" + code)


def source_fenced(config):
    core.guard(config)
    prefix = config["databases"]["platform"]["owner"].removesuffix("_migration_owner")
    roles = (("platform", prefix + "_identity_runtime"),
             ("platform", prefix + "_platform_runtime"),
             ("portal", config["databases"]["portal"]["owner"].removesuffix("_owner") + "_runtime"))
    predicates = ["NOT has_database_privilege(" + core.literal(role) + "," +
                  core.literal(config["databases"][kind]["source"]) + ",'CONNECT')" for kind, role in roles]
    require(core.Pg(config, "postgres").query("SELECT (" + " AND ".join(predicates) + ")::boolean") == "t",
            "SOURCE_RUNTIME_CONNECT_NOT_FENCED")


def invoke(argv):
    """Do not forward child stderr/stdout exceptions; credentials may be there."""
    process = None
    try:
        # A caught signal must not raise between successful OS spawn and the
        # STORE that gives us the child PID. Deferring the handler (not the mask)
        # leaves the executed child's termination signals normally unblocked.
        with deferred_launch_signals():
            process = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                       start_new_session=True)
        stdout, _ = process.communicate(timeout=1200)
    except subprocess.TimeoutExpired:
        terminate_owned_group(process)
        raise core.RecoveryError("RECOVERY_CYCLE_CHILD_TIMEOUT_OR_UNAVAILABLE") from None
    except OSError:
        terminate_owned_group(process)
        raise core.RecoveryError("RECOVERY_CYCLE_CHILD_TIMEOUT_OR_UNAVAILABLE") from None
    except BaseException:
        # User/owner cancellation must not leave an independently sessioned CLI
        # (or its pg_restore/psql child) racing the environment-owner cleanup.
        terminate_owned_group(process)
        raise
    require(process.returncode == 0, "CHILD_REJECTED")
    require(0 < len(stdout) <= core.MAX_JSON, "CHILD_RESPONSE_SIZE_INVALID")
    value = core.decode(stdout)
    require(type(value) is dict, "CHILD_RESPONSE_INVALID")
    return value


@contextlib.contextmanager
def deferred_launch_signals():
    pending = False
    def defer(_signum, _frame):
        nonlocal pending
        pending = True
    original = {sig: signal.getsignal(sig) for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP)}
    try:
        for sig in original:
            signal.signal(sig, defer)
        yield
    finally:
        for sig, handler in original.items():
            signal.signal(sig, handler)
        if pending:
            raise KeyboardInterrupt


def terminate_owned_group(process):
    if process is None:
        return
    # Block repeat termination signals while reaping; restore the caller's mask
    # afterward so cancellation is not swallowed. Only this new child PGID is
    # addressed, never the separately supervised PostgreSQL server's group.
    blocked = {signal.SIGTERM, signal.SIGINT, signal.SIGHUP}
    previous = signal.pthread_sigmask(signal.SIG_BLOCK, blocked)
    try:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.communicate(timeout=5)
    finally:
        signal.pthread_sigmask(signal.SIG_SETMASK, previous)


@contextlib.contextmanager
def interrupt_signals():
    def interrupt(_signum, _frame):
        raise KeyboardInterrupt
    original = {sig: signal.getsignal(sig) for sig in (signal.SIGTERM, signal.SIGHUP)}
    try:
        for sig in original:
            signal.signal(sig, interrupt)
        yield
    finally:
        for sig, handler in original.items():
            signal.signal(sig, handler)


def safe_result(step, value):
    require(value.get("status") == EXPECTED[step], "UNEXPECTED_CHILD_STATUS")
    result = {key: value[key] for key in SAFE if key in value}
    if "requirements" in value:
        requirements = value["requirements"]
        result["eligibleCounts"] = {key: len(requirements.get("eligible_" + key, []))
                                    for key in ("users", "clients", "memberships")}
        result["eligibleCounts"]["services"] = sum(map(len, requirements.get("eligible_services", {}).values()))
    require(result.get("opensTraffic", False) is False, "CHILD_OPENED_TRAFFIC")
    return result


def run_cycle(args):
    config = core.load_config(args.config)
    root = Path(config["taskRoot"])
    require(type(args.minimum_sequence) is int and args.minimum_sequence >= 2, "INDEPENDENT_FLOOR_REQUIRED")
    paths = {name: core.safe_path(getattr(args, name), root) for name in
             ("config", "environment", "selection", "output_env", "output_rearm", "output")}
    require(len(set(paths.values())) == len(paths), "INPUT_OUTPUT_COLLISION")
    for name in ("output_env", "output_rearm", "output"):
        require(not paths[name].exists(), "OUTPUT_ALREADY_EXISTS")
    inputs = {name: core.sha(core.private_read(paths[name])) for name in ("config", "environment", "selection")}
    material = core.keys(config)
    saved = core.checkpoint(config, material)
    _, floor = core.latest_witness(config, material, saved, args.minimum_sequence)
    source_fenced(config)
    common = ["--config", str(paths["config"]), "--minimum-sequence", str(args.minimum_sequence)]
    driver = [sys.executable, "-B", str(OPS / "recovery.py")]
    commands = (
        ("restore-new", [*driver, "restore-new", *common]),
        ("reconcile", [*driver, "reconcile", *common]),
        ("adapter-rearm", [sys.executable, "-B", str(HERE / "phase23-recovery-files.py"), "rearm",
                           *common, "--environment", str(paths["environment"]), "--selection", str(paths["selection"]),
                           "--output-env", str(paths["output_env"]), "--output-rearm", str(paths["output_rearm"])]),
        ("rearm", [*driver, "rearm", *common, "--rearm-file", str(paths["output_rearm"])]),
        ("gate", [*driver, "gate", *common]),
    )
    started = time.monotonic()
    receipt = {"contractVersion": "phase-23-recovery-cycle/1", "status": "IN_PROGRESS",
               "validationLayer": "ACTUAL_TASK_OWNED_POSTGRES_RESTORE_AND_OFFLINE_GATE",
               "startedAtUtc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
               "checkpointSha256": core.sha(saved), "minimumSequence": args.minimum_sequence,
               "authenticatedSequence": floor["sequence"], "inputSha256": inputs,
               "stages": [], "opensTraffic": False, "completeRtoClaim": False}
    step = "preflight"
    interruption = None
    try:
        for step, command in commands:
            for name, expected in inputs.items():
                require(core.sha(core.private_read(paths[name])) == expected, "INPUT_CHANGED")
            begin = time.monotonic()
            result = safe_result(step, invoke(command))
            receipt["stages"].append({"step": step, "elapsedMilliseconds": round((time.monotonic() - begin) * 1000),
                                      "result": result})
        receipt["status"] = "GATE_PASSED_QUARANTINED"
        receipt["newEnvironmentSha256"] = core.sha(core.private_read(paths["output_env"]))
        receipt["newRearmSha256"] = core.sha(core.private_read(paths["output_rearm"]))
    except BaseException as error:
        receipt["status"] = "QUARANTINED"
        receipt["failedStep"] = step
        receipt["databaseWritesMayHaveOccurred"] = True
        category = str(error) if isinstance(error, core.RecoveryError) else "RECOVERY_CYCLE_REJECTED"
        if isinstance(error, KeyboardInterrupt):
            interruption = error
            category = "RECOVERY_CYCLE_INTERRUPTED"
        receipt["code"] = category if re.fullmatch(r"RECOVERY_[A-Z0-9_]{1,100}", category) else "RECOVERY_CYCLE_REJECTED"
    receipt["elapsedMilliseconds"] = round((time.monotonic() - started) * 1000)
    receipt["finishedAtUtc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    core.private_write(paths["output"], receipt)
    if interruption is not None:
        raise interruption
    return receipt


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("config", "environment", "selection", "output-env", "output-rearm", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--minimum-sequence", type=int, required=True)
    args = parser.parse_args(argv)
    with interrupt_signals():
        try:
            receipt = run_cycle(args)
            print(json.dumps({"status": receipt["status"], "stageCount": len(receipt["stages"]),
                              "elapsedMilliseconds": receipt["elapsedMilliseconds"], "opensTraffic": False,
                              "completeRtoClaim": False}, sort_keys=True))
            return 0 if receipt["status"] == "GATE_PASSED_QUARANTINED" else 2
        except KeyboardInterrupt:
            print('{"status":"QUARANTINED","code":"RECOVERY_CYCLE_INTERRUPTED"}', file=sys.stderr)
            return 130
        except Exception:
            print('{"status":"QUARANTINED","code":"RECOVERY_CYCLE_PREFLIGHT_REJECTED"}', file=sys.stderr)
            return 2


if __name__ == "__main__":
    raise SystemExit(main())
