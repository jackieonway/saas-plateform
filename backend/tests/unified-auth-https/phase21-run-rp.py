"""Run the delivered Node RP with public configuration in the owned namespace."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--task-dir", type=Path, required=True)
    args = parser.parse_args()
    root = args.task_dir.resolve(strict=True)
    if root != Path("/tmp/rust-saas-unified-auth-20261028") or not (root / "OWNER").is_file():
        raise SystemExit("Owned Phase 21 task required")
    expected_namespace = (root / "namespace.identity").read_text().strip()
    if os.readlink("/proc/self/ns/mnt") != expected_namespace:
        raise SystemExit("Owned mount namespace required")
    configuration = json.loads((root / "https-environment.json").read_text())
    phase = configuration["phase21"]
    if phase["contractVersion"] != "unified-auth-phase21/1":
        raise SystemExit("Exact Phase 21 fixture required")
    source = root / "phase21-rp-source"
    attestation = json.loads((root / "phase21-rp-attestation.json").read_text())
    for relative, expected in attestation["inputs"].items():
        if hashlib.sha256((source / relative).read_bytes()).hexdigest() != expected:
            raise SystemExit("Frozen RP source changed")
    node = root / "phase21-node-runtime/node-v24.22.1-linux-x64/bin/node"
    environment = {**phase["rp"]["third"], "PATH": str(node.parent) + ":/usr/bin:/bin",
                   "HOME": str(root), "LANG": "C.UTF-8"}
    # No database URL, private service credential or owner proof reaches the RP.
    assert not any("DATABASE" in key or "SERVICE" in key for key in environment)
    record = root / "phase21-rp.pid"
    if record.exists():
        raise SystemExit("Phase 21 RP already has an ownership record")
    owner = (root / "OWNER").stat()
    if os.getuid() == 0:
        os.setgid(owner.st_gid)
        os.setuid(owner.st_uid)

    def stop(*_args):
        raise SystemExit(0)

    for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        signal.signal(signum, stop)
    child = None
    with (root / "phase21-rp.log").open("ab", buffering=0) as log:
        try:
            child = subprocess.Popen([str(node), str(source / "server.mjs")], cwd=source,
                                     env=environment, stdout=log, stderr=subprocess.STDOUT)
            start = Path("/proc/" + str(child.pid) + "/stat").read_text().rsplit(")", 1)[1].split()[19]
            record.write_text(json.dumps({"supervisor": os.getpid(), "pid": child.pid,
                "startTime": start, "source": str(source), "namespace": expected_namespace}))
            record.chmod(0o600)
            print("PHASE21_LINUX_STANDARD_RP_STARTED", flush=True)
            while child.poll() is None and (root / "namespace.pid").exists():
                time.sleep(0.5)
            if child.poll() is not None:
                raise SystemExit(child.returncode)
        finally:
            if child is not None and child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=3)
            record.unlink(missing_ok=True)
            print("PHASE21_LINUX_STANDARD_RP_STOPPED", flush=True)


if __name__ == "__main__":
    main()
