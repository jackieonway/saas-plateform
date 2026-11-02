"""Run one task binary in its verified private mount namespace."""

import argparse
import hashlib
import json
import os
import pathlib
import signal
import subprocess
import time


INSTANCES = {
    "identity": ("identity", "saas-identity"),
    "identity-peer": ("identity", "saas-identity"),
    "platform": ("platform", "saas-platform-backend"),
    "portal": ("portal", "saas-customer-portal-backend"),
}


def service_instance(task, configuration, instance):
    """Resolve one process without creating a new logical runtime identity."""
    if instance not in INSTANCES:
        raise SystemExit("Invalid task service instance")
    service, executable = INSTANCES[instance]
    environment = configuration.get(service)
    if not isinstance(environment, dict) or not all(
            isinstance(key, str) and isinstance(value, str) for key, value in environment.items()):
        raise SystemExit("Invalid private service environment")
    environment = dict(environment)
    if instance == "identity-peer":
        environment["BIND_ADDR"] = "127.0.0.1:23442"
    environment["PATH"] = "/usr/lib/postgresql/18/bin:/usr/bin:/bin"
    return (task / "bin" / executable, environment,
            task / ("service-" + instance + ".pid"),
            task / ("service-" + instance + ".log"))


def keyset_record(environment, instance):
    """Attest configured bytes; actual signing must prove process loading."""
    if instance not in ("identity", "identity-peer") or "OIDC_KEYSET_PATH" not in environment:
        return {}
    keyset = pathlib.Path(environment["OIDC_KEYSET_PATH"])
    try:
        content = keyset.read_bytes()
    except OSError:
        raise SystemExit("Task signing keyset unavailable") from None
    return {"keysetPath": str(keyset),
            "configuredKeysetSha256": hashlib.sha256(content).hexdigest()}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--task-dir", type=pathlib.Path, required=True)
    parser.add_argument("service", choices=tuple(INSTANCES))
    args = parser.parse_args()
    task = args.task_dir.resolve()
    if task.parent != pathlib.Path("/tmp") or not task.name.startswith("rust-saas-unified-auth-"):
        raise SystemExit("Task directory is outside the dedicated temporary scope")
    if not (task / "OWNER").is_file():
        raise SystemExit("Task ownership marker is required")
    expected_namespace = (task / "namespace.identity").read_text().strip()
    if os.readlink("/proc/self/ns/mnt") != expected_namespace:
        raise SystemExit("Service must run in the owned mount namespace")
    configuration = json.loads((task / "https-environment.json").read_text())
    binary, environment, record, log_path = service_instance(task, configuration, args.service)
    if not binary.is_file():
        raise SystemExit("Task binary is unavailable")
    if record.exists():
        raise SystemExit("Task service already has an ownership record")
    configured_keyset = keyset_record(environment, args.service)

    def stop(*_args):
        raise SystemExit(0)

    for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        signal.signal(signum, stop)
    child = None
    with log_path.open("ab", buffering=0) as log:
        try:
            child = subprocess.Popen([str(binary)], cwd=task, env=environment,
                                     stdout=log, stderr=subprocess.STDOUT)
            start_time = pathlib.Path("/proc/" + str(child.pid) + "/stat").read_text().rsplit(")", 1)[1].split()[19]
            record.write_text(json.dumps({
                "supervisor": os.getpid(), "pid": child.pid, "startTime": start_time,
                "binary": str(binary), "namespace": expected_namespace,
                **configured_keyset,
            }))
            record.chmod(0o600)
            print("TASK_SERVICE_STARTED " + args.service + " pid=" + str(child.pid), flush=True)
            while child.poll() is None and (task / "namespace.pid").exists() and (task / "OWNER").exists():
                time.sleep(1)
            if child.poll() is not None:
                raise SystemExit(child.returncode)
        finally:
            if child is not None and child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=3)
            record.unlink(missing_ok=True)
            print("TASK_SERVICE_STOPPED " + args.service, flush=True)


if __name__ == "__main__":
    main()
