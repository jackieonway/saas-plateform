#!/usr/bin/env python3
"""Prepare one authorized temporary SELF notification capability, without IO to services.

Only --execute writes a new private output directory. The original policy,
configuration-rekey evidence, environment and launch plan are never overwritten.
No database, HTTP, subprocess or service-control API is used by this helper.
The prepared platform launch command is data for the owning operator, not run.
"""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import stat
import sys

POLICY_SHA = "32a7b7552902f904029125948ed5e32cb2b6bdcabc7828df20be9fcf24c43a0a"
SUMMARY_SHA = "30655e179971d2de959a73e328d47b2351c840ce0248819e3cfb8b675f7b197a"
MANIFEST_SHA = "ebac0beadde0cdf7bd02695c9f2125d90b16355f8cec8943042fe73e22d0bc76"
BINARY_SHA = "45049fa89b9272920448e49a5ac3af9e78d59c741dc6b65d68cfe9d33f8e03f0"
AUTHORIZATION = "User explicitly authorized one temporary SELF notification grant for this test, revoke after validation"
CAPABILITY = {"resource": "notification", "action": "publish", "scopes": ["SELF"]}
MAX_BYTES = 16 * 1024 * 1024


class Rejected(Exception):
    """All diagnostics are fixed identifiers, never source values."""


def require(condition, code):
    if not condition:
        raise Rejected(code)


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def checked_path(value, exists=True):
    require(sys.platform == "linux" and os.geteuid() == 0, "LINUX_ROOT_REQUIRED")
    require(isinstance(value, str) and value.startswith("/") and "\\" not in value and "\0" not in value
            and all(part not in ("", ".", "..") for part in value.split("/")[1:]), "NORMALIZED_PATH_REQUIRED")
    path = Path(value)
    for component in ([path] if exists else []) + list(path.parents):
        info = component.lstat()
        require(not stat.S_ISLNK(info.st_mode), "SYMLINK_REJECTED")
    return path


def private_bytes(path, limit=MAX_BYTES):
    path = checked_path(str(path))
    info = path.stat()
    require(stat.S_ISREG(info.st_mode) and info.st_uid == 0 and info.st_nlink == 1
            and stat.S_IMODE(info.st_mode) == 0o600 and info.st_size <= limit, "PRIVATE_INPUT_INVALID")
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(fd, "rb") as handle:
        opened = os.fstat(handle.fileno())
        require(opened.st_dev == info.st_dev and opened.st_ino == info.st_ino
                and opened.st_uid == 0 and opened.st_nlink == 1
                and stat.S_IMODE(opened.st_mode) == 0o600, "PRIVATE_OPEN_CHANGED")
        raw = handle.read(limit + 1)
    require(len(raw) <= limit, "PRIVATE_INPUT_TOO_LARGE")
    return raw


def unique(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "JSON_DUPLICATE_KEY")
        result[key] = value
    return result


def parsed(raw):
    try:
        return json.loads(raw.decode("utf-8-sig"), object_pairs_hook=unique,
                          parse_constant=lambda _: (_ for _ in ()).throw(Rejected("JSON_NUMBER_INVALID")))
    except (UnicodeError, ValueError):
        raise Rejected("JSON_INVALID") from None


def json_bytes(value):
    return (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode()


def write_new(path, raw):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    os.fchmod(fd, 0o600)
    with os.fdopen(fd, "wb") as handle:
        handle.write(raw)
        handle.flush()
        os.fsync(handle.fileno())
    require(private_bytes(path) == raw, "OUTPUT_READBACK_FAILED")


def binary_digest(path):
    path = checked_path(str(path))
    info = path.stat()
    require(stat.S_ISREG(info.st_mode) and info.st_uid == 0, "FROZEN_BINARY_INVALID")
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def prepare(args):
    materials = checked_path(args.materials)
    runtime = checked_path(args.runtime)
    output = checked_path(args.output, exists=False)
    require(materials.is_dir() and runtime.is_dir() and output.parent == materials,
            "OUTPUT_MATERIALS_BOUNDARY_REQUIRED")
    require(not output.exists(), "NEW_OUTPUT_REQUIRED")
    require(args.authorization_note == AUTHORIZATION, "EXPLICIT_AUTHORIZATION_NOTE_REQUIRED")
    parent_info = materials.stat()
    require(parent_info.st_uid == 0 and stat.S_IMODE(parent_info.st_mode) == 0o700,
            "MATERIALS_PRIVATE_DIRECTORY_REQUIRED")

    baseline_paths = {
        "policy": materials / "data-grant-policy.json",
        "policyCopy": materials / "policy-data-grant-policy.json",
        "summary": materials / "policy-summary.json",
        "manifest": materials / "policy-manifest.uuid7",
    }
    originals = {key: private_bytes(filename) for key, filename in baseline_paths.items()}
    expected = {"policy": POLICY_SHA, "policyCopy": POLICY_SHA, "summary": SUMMARY_SHA, "manifest": MANIFEST_SHA}
    require(all(digest(originals[key]) == expected[key] for key in expected), "BASELINE_DIGEST_MISMATCH")
    base = parsed(originals["policy"])
    handoff = parsed(private_bytes(materials / "credential-handoff.private.json"))
    require(isinstance(handoff, dict) and isinstance(handoff.get("applications"), list), "HANDOFF_APPLICATIONS_REQUIRED")
    matches = [item for item in handoff["applications"] if isinstance(item, dict) and item.get("code") == "work-orders"]
    require(len(matches) == 1 and isinstance(matches[0].get("applicationId"), str), "WORK_ORDERS_HANDOFF_UNIQUE_REQUIRED")
    app_id = matches[0]["applicationId"]
    require(isinstance(base, dict) and set(base) == {"applications", "managementScopes"}
            and isinstance(base["applications"], list), "CLOSED_POLICY_REQUIRED")
    indexed = [(index, app) for index, app in enumerate(base["applications"])
               if isinstance(app, dict) and app.get("applicationId") == app_id]
    require(len(indexed) == 1, "WORK_ORDERS_BASE_POLICY_UNIQUE_REQUIRED")
    index, application = indexed[0]
    require(isinstance(application.get("capabilities"), list) and all(isinstance(item, dict)
            for item in application["capabilities"]), "CAPABILITY_ARRAY_REQUIRED")
    require(not any(item.get("resource") == "notification" and item.get("action") == "publish"
                    for item in application["capabilities"]), "NOTIFICATION_CAPABILITY_ALREADY_PRESENT")
    extension = copy.deepcopy(base)
    extension["applications"][index]["capabilities"].append(copy.deepcopy(CAPABILITY))
    reverted = copy.deepcopy(extension)
    removed = reverted["applications"][index]["capabilities"].pop()
    require(removed == CAPABILITY and reverted == base, "SINGLE_CAPABILITY_DIFF_REQUIRED")
    require(extension["managementScopes"] == base["managementScopes"], "MANAGEMENT_BOUNDARIES_CHANGED")
    for original_app, new_app in zip(base["applications"], extension["applications"]):
        require(original_app.get("memberLookups") == new_app.get("memberLookups")
                and original_app.get("organizationLookups") == new_app.get("organizationLookups"), "LOOKUPS_CHANGED")
    policy_raw = json_bytes(extension)

    base_env_path = runtime / "env" / "platform.env"
    env_raw = private_bytes(base_env_path)
    env_lines = env_raw.splitlines(keepends=True)
    policy_lines = [i for i, line in enumerate(env_lines) if line.startswith(b"DATA_GRANT_POLICY=")]
    require(len(policy_lines) == 1, "ONE_POLICY_ENV_LINE_REQUIRED")
    env_index = policy_lines[0]
    old_line = env_lines[env_index]
    line_ending = b"\r\n" if old_line.endswith(b"\r\n") else b"\n" if old_line.endswith(b"\n") else b""
    original_value = old_line[len(b"DATA_GRANT_POLICY="):]
    if line_ending:
        original_value = original_value[:-len(line_ending)]
    require(parsed(original_value) == base, "BASE_ENV_POLICY_MISMATCH")
    compact = json.dumps(extension, ensure_ascii=False, separators=(",", ":")).encode()
    new_lines = list(env_lines)
    new_lines[env_index] = b"DATA_GRANT_POLICY=" + compact + line_ending
    new_env_raw = b"".join(new_lines)
    require(all(before == after for i, (before, after) in enumerate(zip(env_lines, new_lines)) if i != env_index)
            and len(env_lines) == len(new_lines), "OTHER_ENV_BYTES_CHANGED")

    frozen = checked_path("/usr/local/libexec/saas-uuid-u05-platform")
    require(binary_digest(frozen) == BINARY_SHA and binary_digest(runtime / "bin" / "platform") == BINARY_SHA,
            "FROZEN_PLATFORM_DIGEST_MISMATCH")
    plan_path = runtime / "launch-plan.json"
    plan_raw = private_bytes(plan_path)
    plan = parsed(plan_raw)
    require(isinstance(plan, dict) and plan.get("format") == "saas-uuid-u05-launch-plan/1"
            and plan.get("automatic_start") is False and isinstance(plan.get("commands"), list), "LAUNCH_PLAN_INVALID")
    platform_commands = [item for item in plan["commands"] if isinstance(item, dict) and item.get("service") == "platform"]
    require(len(platform_commands) == 1, "PLATFORM_COMMAND_UNIQUE_REQUIRED")
    command = copy.deepcopy(platform_commands[0])
    argv = command.get("argv")
    require(isinstance(argv, list) and all(isinstance(item, str) for item in argv)
            and argv[:2] == ["docker", "run"] and argv[-1] == BINARY_SHA, "PLATFORM_ARGV_INVALID")
    old_mount = f"type=bind,source={base_env_path},target=/run/u05/service.env,readonly"
    require(argv.count(old_mount) == 1, "PLATFORM_ENV_MOUNT_UNIQUE_REQUIRED")
    mount_index = argv.index(old_mount)
    require(mount_index > 0 and argv[mount_index - 1] == "--mount", "PLATFORM_ENV_MOUNT_FLAG_REQUIRED")
    argv[mount_index] = f"type=bind,source={output / 'platform.env'},target=/run/u05/service.env,readonly"
    old_command = platform_commands[0]
    restored_command = copy.deepcopy(command)
    restored_command["argv"][mount_index] = old_mount
    require(restored_command == old_command, "LAUNCH_ONLY_ENV_MOUNT_MAY_CHANGE")
    candidate = {"format": "saas-uuid-u05-notification-platform-launch/1", "automatic_start": False, **command}
    launch_raw = json_bytes(candidate)
    metadata = {
        "format": "saas-uuid-u05-notification-scope-extension/1", "status": "PREPARED",
        "authorizationNote": AUTHORIZATION, "applicationCode": "work-orders",
        "temporary": True, "revokeGrantAfterValidation": True,
        "servicesStarted": 0, "httpRequests": 0, "databaseWrites": 0,
        "baseline": {key: {"path": str(baseline_paths[key]), "sha256": expected[key]} for key in expected},
        "policy": {"path": str(output / "data-grant-policy.private.json"), "sha256": digest(policy_raw)},
        "environment": {"basePath": str(base_env_path), "baseSha256": digest(env_raw),
                        "candidatePath": str(output / "platform.env"), "candidateSha256": digest(new_env_raw),
                        "changedKeys": ["DATA_GRANT_POLICY"], "otherBytesUnchanged": True},
        "diff": {"addedCapabilities": [CAPABILITY], "removedCapabilities": 0, "changedExistingCapabilities": 0,
                 "managementScopesUnchanged": True, "memberLookupsUnchanged": True,
                 "organizationLookupsUnchanged": True, "reverseSingleAppendEqualsBase": True},
        "frozenPlatformSha256": BINARY_SHA,
        "launch": {"basePath": str(plan_path), "baseSha256": digest(plan_raw),
                   "candidatePath": str(output / "launch-platform.private.json"), "candidateSha256": digest(launch_raw),
                   "changedArguments": 1, "onlyEnvironmentMountSourceChanged": True},
    }
    if args.execute:
        output.mkdir(mode=0o700)
        write_new(output / "data-grant-policy.private.json", policy_raw)
        write_new(output / "platform.env", new_env_raw)
        write_new(output / "launch-platform.private.json", launch_raw)
        require(all(private_bytes(baseline_paths[key]) == originals[key] for key in originals)
                and private_bytes(base_env_path) == env_raw and private_bytes(plan_path) == plan_raw,
                "BASELINE_CHANGED_DURING_PREPARATION")
        write_new(output / "extension.private.json", json_bytes(metadata))
    return {"status": "PREPARED" if args.execute else "CHECKED_NO_WRITES", "addedCapabilities": 1,
            "scope": "SELF", "httpRequests": 0, "databaseWrites": 0, "servicesStarted": 0,
            "policySha256": digest(policy_raw), "envSha256": digest(new_env_raw), "output": str(output)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--materials", required=True)
    parser.add_argument("--runtime", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--authorization-note", required=True)
    parser.add_argument("--execute", action="store_true")
    args = parser.parse_args()
    try:
        print(json.dumps(prepare(args), separators=(",", ":")))
    except Rejected as error:
        print(json.dumps({"status": "REJECTED", "error": str(error)}), file=sys.stderr)
        return 1
    except Exception:
        print(json.dumps({"status": "REJECTED", "error": "PREPARATION_OPERATION_FAILED"}), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
