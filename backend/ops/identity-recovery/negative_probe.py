#!/usr/bin/env python3
"""Authenticated-file negatives with read-only PostgreSQL preflight.

No command in this module creates/restores a database, changes HBA, writes SQL,
starts services or removes a directory. The environment owner runs this only
while both sources are stopped and both restore targets are still absent.
"""
import argparse
import base64
import contextlib
import hmac
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import sys

import core
import witness


FORMAT = "identity-recovery-negative-probe/1"
LAYER = "REAL_AUTHENTICATED_FILE_LOADERS_AND_READONLY_PG_PREFLIGHT"
BASELINE_NAMES = ("floor", "current", "record")
CASE_NAMES = ("ciphertext-tamper", "manifest-tamper", "wrong-encryption-key", "mixed-source-binding",
              "signed-old-floor", "signed-old-current", "signed-old-record", "missing-chain", "extra-chain")


class ProbeFailure(core.RecoveryError):
    def __init__(self, category, cases):
        super().__init__(category)
        self.cases = cases


def reject(code):
    raise core.RecoveryError("RECOVERY_NEGATIVE_" + code)


def baseline_directory(config):
    return config["workDirectory"] / "negative-baseline"


def present_bytes(path):
    """Distinguish genuine absence from unsafe links, modes and I/O errors."""
    with core.parent_descriptor(path) as (parent, name):
        try:
            os.stat(name, dir_fd=parent, follow_symlinks=False)
        except FileNotFoundError:
            return None
        return core.read_at(parent, name, core.MAX_FILE)


def same_bytes(left, right):
    if left is None or right is None:
        return left is right
    return len(left) == len(right) and hmac.compare_digest(core.sha(left), core.sha(right))


def change_exact(path, expected, replacement):
    """Compare-and-change only an exact file; ancestor resolution is anchored."""
    with core.parent_descriptor(path) as (parent, name):
        try:
            os.stat(name, dir_fd=parent, follow_symlinks=False)
        except FileNotFoundError:
            actual = None
        else:
            actual = core.read_at(parent, name, core.MAX_FILE)
        if not same_bytes(actual, expected):
            reject("CONCURRENT_FILE_CHANGE")
        if replacement is None:
            if actual is not None:
                os.unlink(name, dir_fd=parent)
                os.fsync(parent)
            return
        temporary = ".negative-change-" + secrets.token_hex(12)
        descriptor = None
        published = False
        try:
            descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                                 0o600, dir_fd=parent)
            with os.fdopen(descriptor, "wb", closefd=False) as output:
                output.write(replacement)
                output.flush()
                os.fsync(descriptor)
            if expected is None:
                # Atomic no-replace publication; never clobber a concurrent file.
                os.link(temporary, name, src_dir_fd=parent, dst_dir_fd=parent, follow_symlinks=False)
                os.unlink(temporary, dir_fd=parent)
            else:
                # The environment owner serializes all artifact writers. Check
                # again immediately before replace; a conflicting writer stops
                # the probe instead of being silently reset in finally.
                if not same_bytes(core.read_at(parent, name, core.MAX_FILE), expected):
                    reject("CONCURRENT_FILE_CHANGE")
                os.replace(temporary, name, src_dir_fd=parent, dst_dir_fd=parent)
            published = True
            os.fsync(parent)
        finally:
            if descriptor is not None:
                os.close(descriptor)
            if not published:
                try:
                    os.unlink(temporary, dir_fd=parent)
                except FileNotFoundError:
                    pass


@contextlib.contextmanager
def mutation(config, name, path, replacement):
    """Keep a private before-image until exact restoration has succeeded.

    A conflict retains the journal and the conflicting file. It does not retry,
    restore a whole directory, or overwrite a different writer's data.
    """
    before = present_bytes(path)
    journal = baseline_directory(config) / ("probe-journal-" + name + ".json")
    content = core.canonical({"format": FORMAT, "case": name, "path": str(path),
        "before": None if before is None else base64.b64encode(before).decode(),
        "expectedAfterSha256": None if replacement is None else core.sha(replacement)})
    core.private_write(journal, content)
    try:
        change_exact(path, before, replacement)
        yield
    finally:
        actual = present_bytes(path)
        if same_bytes(actual, replacement):
            change_exact(path, replacement, before)
        elif not same_bytes(actual, before):
            reject("CONCURRENT_FILE_CHANGE")
        # A concurrent change above raises before this unlink. The before-image
        # remains protected for the environment owner's explicit investigation.
        change_exact(journal, content, None)


def save_baseline(config_path, minimum_sequence=1):
    if minimum_sequence != 1:
        reject("BASELINE_SEQUENCE_REQUIRED")
    config = core.load_config(config_path)
    material = core.keys(config)
    saved = core.checkpoint(config, material)
    document, floor = core.latest_witness(config, material, saved, 1)
    if floor["sequence"] != 1 or document["sequence"] != 1:
        reject("BASELINE_MUST_BE_FIRST_SEQUENCE")
    directory = baseline_directory(config)
    core.private_directory(directory, create=True)
    paths = {"floor": config["floorFile"], "current": config["witnessDirectory"] / "CURRENT",
             "record": config["witnessDirectory"] / "witness-00000001.json"}
    copies = {}
    for name, path in paths.items():
        raw = core.private_read(path)
        core.private_write(directory / (name + ".bin"), raw)
        copies[name] = {"sha256": core.sha(raw), "bytes": len(raw)}
    value = {"format": FORMAT, "checkpointSha256": core.sha(saved), "sequence": 1,
             "binding": core.binding(config), "copies": copies}
    core.authenticated_write(directory / "baseline.json", value, material["mac"], "negative-baseline")
    return {"status": "BASELINE_SAVED", "sequence": 1, "files": 3, "validationLayer": LAYER}


def load_baseline(config, material, saved):
    directory = baseline_directory(config)
    core.private_directory(directory)
    value = core.authenticated_read(directory / "baseline.json", material["mac"], "negative-baseline")
    core.exact(value, ("format", "checkpointSha256", "sequence", "binding", "copies"))
    if (value["format"] != FORMAT or value["sequence"] != 1 or value["binding"] != core.binding(config)
            or value["checkpointSha256"] != core.sha(saved) or set(value["copies"]) != set(BASELINE_NAMES)):
        reject("BASELINE_BINDING_INVALID")
    copies = {}
    for name in BASELINE_NAMES:
        core.exact(value["copies"][name], ("sha256", "bytes"))
        raw = core.private_read(directory / (name + ".bin"))
        if core.sha(raw) != value["copies"][name]["sha256"] or len(raw) != value["copies"][name]["bytes"]:
            reject("BASELINE_COPY_CHANGED")
        copies[name] = raw
    original_floor = core.authenticated_read(directory / "floor.bin", material["mac"], "floor")
    record = witness.verify(material["witness"], core.witness_store(config, material, saved).binding,
                            core.decode(copies["record"]), minimum_sequence=1)
    current = core.decode(copies["current"])
    core.exact(current, ("document", "tag"))
    head = current["document"]
    if (not hmac.compare_digest(current["tag"], witness._tag(material["witness"], witness.CURRENT_DOMAIN, head))
            or head["format"] != witness.CURRENT_FORMAT or head["sequence"] != 1
            or head["binding"] != record["document"]["binding"]
            or head["record_sha256"] != witness._sha(record)
            or record["document"]["sequence"] != 1 or original_floor["sequence"] != 1
            or original_floor["checkpointSha256"] != core.sha(saved)
            or original_floor["witnessSha256"] != core.sha(record["document"])):
        reject("BASELINE_SIGNATURE_INVALID")
    return copies


def readonly_preflight(config):
    # guard issues SELECT only and requires zero connections to each source.
    core.guard(config)
    sources = ",".join(core.literal(value["source"]) for value in config["databases"].values())
    targets = ",".join(core.literal(value["target"]) for value in config["databases"].values())
    actual = core.Pg(config, "postgres").json("SELECT jsonb_build_object("
        "'sourceConnectionCount',(SELECT count(*) FROM pg_stat_activity WHERE datname=ANY(ARRAY[" + sources + "])),"
        "'targetDatabaseCount',(SELECT count(*) FROM pg_database WHERE datname=ANY(ARRAY[" + targets + "])))")
    if actual != {"sourceConnectionCount": 0, "targetDatabaseCount": 0}:
        reject("QUIET_SOURCES_AND_ABSENT_TARGETS_REQUIRED")
    return actual


def rejection_category(error):
    category = str(error) if isinstance(error, core.RecoveryError) else "RECOVERY_REJECTED"
    return category if re.fullmatch(r"RECOVERY_[A-Z0-9_]+", category) else "RECOVERY_REJECTED"


def status_rejection(config_path, minimum_sequence):
    """Run the real CLI status path; it authenticates files and never uses PG."""
    result = subprocess.run([sys.executable, "-B", str(Path(__file__).with_name("recovery.py")),
        "status", "--config", str(config_path), "--minimum-sequence", str(minimum_sequence)],
        env={"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "PYTHONDONTWRITEBYTECODE": "1"},
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30, check=False)
    if result.returncode != 2 or result.stdout or len(result.stderr) > 4096:
        reject("EXPECTED_CLI_REJECTION_MISSING")
    value = core.decode(result.stderr)
    core.exact(value, ("status", "code"))
    if value["status"] != "QUARANTINED" or not re.fullmatch(r"RECOVERY_[A-Z0-9_]+", value["code"]):
        reject("UNSAFE_CLI_REJECTION")
    return value["code"]


def load_archives(config_path, minimum_sequence):
    """Use restore-new's actual authenticated precreation loaders, without SQL."""
    config = core.load_config(config_path)
    material = core.keys(config)
    saved = core.checkpoint(config, material)
    _, floor = core.latest_witness(config, material, saved, minimum_sequence)
    for kind, record in saved["archives"].items():
        core.decrypt(config, material, kind, record, saved["binding"])
    if floor["facts"] is not None:
        core.decrypt(config, material, "deny-facts-%08d" % floor["sequence"], floor["facts"], saved["binding"])


def archive_rejection(config_path, minimum_sequence):
    try:
        load_archives(config_path, minimum_sequence)
    except Exception as error:
        return rejection_category(error)
    reject("EXPECTED_ARCHIVE_REJECTION_MISSING")


def run_probe(config_path, minimum_sequence, preflight=readonly_preflight):
    if type(minimum_sequence) is not int or minimum_sequence < 2:
        reject("LATER_INDEPENDENT_SEQUENCE_REQUIRED")
    config = core.load_config(config_path)
    material = core.keys(config)
    saved = core.checkpoint(config, material)
    latest, floor = core.latest_witness(config, material, saved, minimum_sequence)
    if latest["sequence"] < 2 or latest["sequence"] >= witness.MAX_RECORDS:
        reject("LATER_SEQUENCE_REQUIRED")
    baseline = load_baseline(config, material, saved)
    # A pre-existing corrupt backup cannot become an apparently successful
    # negative. Prove all original artifacts decrypt before altering any file.
    load_archives(config_path, minimum_sequence)
    observed_sequence = floor["sequence"]
    checkpoint_path = config["workDirectory"] / "checkpoint.json"
    manifest = core.decode(core.private_read(checkpoint_path))
    manifest["document"]["createdAtMs"] += 1
    key_file = core.decode(core.private_read(config["keyFile"]))
    other_keys = {base64.b64decode(key_file[name]) for name in ("encryption", "mac", "witness")}
    altered_key = secrets.token_bytes(32)
    while altered_key in other_keys:
        altered_key = secrets.token_bytes(32)
    key_file["encryption"] = base64.b64encode(altered_key).decode()
    mixed = core.decode(core.private_read(config_path))
    suffix = mixed["databases"]["platform"]["source"][-32:]
    altered_suffix = suffix[:-1] + ("0" if suffix[-1] != "0" else "1")
    mixed["databases"]["platform"]["source"] = "unified_auth_https_" + altered_suffix
    mixed["databases"]["portal"]["source"] = "unified_auth_portal_" + altered_suffix
    mixed["databases"]["platform"]["target"] = "unified_auth_restore_" + altered_suffix
    mixed["databases"]["portal"]["target"] = "unified_auth_restore_portal_" + altered_suffix
    ciphertext_path = config["workDirectory"] / saved["archives"]["platform"]["filename"]
    raw_ciphertext = core.private_read(ciphertext_path, core.MAX_FILE)
    changed_ciphertext = bytes([raw_ciphertext[0] ^ 1]) + raw_ciphertext[1:]
    record_path = config["witnessDirectory"] / ("witness-%08d.json" % observed_sequence)
    cases = (
        ("ciphertext-tamper", ciphertext_path, changed_ciphertext, archive_rejection, "RECOVERY_ARCHIVE_DIGEST_MISMATCH"),
        ("manifest-tamper", checkpoint_path, core.canonical(manifest), status_rejection, "RECOVERY_AUTHENTICATION_FAILED"),
        ("wrong-encryption-key", config["keyFile"], core.canonical(key_file), archive_rejection, "RECOVERY_ARCHIVE_AUTHENTICATION_FAILED"),
        ("mixed-source-binding", Path(config_path), core.canonical(mixed), status_rejection, "RECOVERY_CHECKPOINT_BINDING_MISMATCH"),
        ("signed-old-floor", config["floorFile"], baseline["floor"], status_rejection, "RECOVERY_EXTERNAL_FLOOR_ROLLBACK"),
        ("signed-old-current", config["witnessDirectory"] / "CURRENT", baseline["current"], status_rejection, "RECOVERY_REJECTED"),
        ("signed-old-record", record_path, baseline["record"], status_rejection, "RECOVERY_REJECTED"),
        ("missing-chain", config["witnessDirectory"] / "witness-00000001.json", None, status_rejection, "RECOVERY_REJECTED"),
        ("extra-chain", config["witnessDirectory"] / ("witness-%08d.json" % (observed_sequence + 1)),
         baseline["record"], status_rejection, "RECOVERY_REJECTED"),
    )
    results = []
    for name, path, replacement, loader, expected in cases:
        before = after = None
        original = present_bytes(path)
        try:
            before = preflight(config)
            try:
                with mutation(config, name, path, replacement):
                    code = loader(config_path, minimum_sequence)
                    if code != expected:
                        reject("UNEXPECTED_REJECTION_CATEGORY")
            finally:
                after = preflight(config)
            # Re-authenticate original config/key material and every archive
            # after restoration, rather than trusting cached key bytes.
            recovered_config = core.load_config(config_path)
            recovered_material = core.keys(recovered_config)
            if recovered_config != config or recovered_material != material:
                reject("RESTORATION_MISMATCH")
            recovered, current_floor = core.latest_witness(config, material, core.checkpoint(config, material), minimum_sequence)
            if recovered != latest or current_floor != floor:
                reject("RESTORATION_MISMATCH")
            load_archives(config_path, minimum_sequence)
            results.append({"case": name, "status": "PASS", "code": code,
                            "preflightBefore": before, "preflightAfter": after, "originalBytesRestored": True})
        except Exception as error:
            try:
                restored = same_bytes(present_bytes(path), original)
            except Exception:
                restored = False
            category = rejection_category(error)
            results.append({"case": name, "status": "FAIL", "code": category,
                            "preflightBefore": before, "preflightAfter": after, "originalBytesRestored": restored})
            raise ProbeFailure(category, results) from None
    return {"format": FORMAT, "status": "PASS", "sequence": observed_sequence,
            "validationLayer": LAYER, "restoreSqlExecuted": False, "databaseWrites": False,
            "cases": results}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stage", required=True, choices=("save-baseline", "probe"))
    parser.add_argument("--config", required=True)
    parser.add_argument("--minimum-sequence", required=True, type=int)
    args = parser.parse_args()
    try:
        result = (save_baseline(args.config, args.minimum_sequence) if args.stage == "save-baseline"
                  else run_probe(args.config, args.minimum_sequence))
        print(json.dumps(result, sort_keys=True))
        return 0
    except Exception as error:
        print(json.dumps({"format": FORMAT, "status": "FAILED", "code": rejection_category(error),
                          "validationLayer": LAYER, "restoreSqlExecuted": False,
                          "cases": error.cases if isinstance(error, ProbeFailure) else [],
                          "recoveryJournalMayRemain": True}), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
