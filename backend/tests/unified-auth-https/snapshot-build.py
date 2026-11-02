"""Copy a closed source revision briefly, then build without locking live work."""

import argparse
import datetime
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import time


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


FRONTEND_ROOTS = {
    "saas-plateform/frontend", "saas-customer-portal/frontend",
    "saas-stores/frontend", "saas-work-orders/frontend", "saas-examples/websocket",
    "saas-examples/micro-vue", "saas-examples/micro-jquery", "saas-examples/report-center",
}


def source_files(workspace, include_portal, include_all_projects=False):
    directories = [
        "saas-identity/src", "saas-identity/static", "saas-identity/tests",
        "saas-notification/src", "saas-notification/tests",
        "saas-plateform/backend/src", "saas-plateform/backend/static",
        "saas-plateform/backend/assets", "saas-plateform/backend/migrations",
        "saas-plateform/backend/crates", "saas-plateform/backend/ops/recovery/src",
        "saas-plateform/backend/ops/recovery/tests",
        "saas-plateform/backend/nginx", "saas-plateform/backend/docs/examples",
        "saas-plateform/backend/tools/platform-admin/src",
        "saas-plateform/backend/tools/platform-admin/manifests",
        "saas-plateform/backend/tools/platform-admin/schemas",
        "saas-plateform/backend/tools/platform-admin/tests",
        "saas-plateform/backend/ops/identity-deployment",
        "saas-plateform/backend/ops/identity-recovery",
        "saas-plateform/backend/tests",
        "saas-plateform/frontend/dist", "saas-plateform/frontend/src",
        "saas-plateform/frontend/public", "saas-plateform/frontend/scripts",
        "saas-plateform/frontend/vendor",
        "saas-plateform/frontend/tests",
        "saas-examples/oidc-rp/src", "saas-examples/oidc-rp/public",
        "saas-examples/oidc-rp/scripts", "saas-examples/oidc-rp/tests",
    ]
    roots = ["saas-identity", "saas-notification", "saas-plateform/backend",
             "saas-plateform/backend/ops/recovery",
             "saas-plateform/backend/tools/platform-admin"]
    if include_portal or include_all_projects:
        directories += ["saas-customer-portal/backend/src", "saas-customer-portal/backend/migrations",
                        "saas-customer-portal/backend/tests",
                        "saas-customer-portal/frontend/dist", "saas-customer-portal/frontend/src",
                        "saas-customer-portal/frontend/public", "saas-customer-portal/frontend/tests"]
        roots += ["saas-customer-portal/backend"]
    if include_all_projects:
        for project in ("saas-stores", "saas-work-orders"):
            directories += [project + "/backend/" + name for name in
                            ("src", "migrations", "tests", "vendor", "manifests")]
            directories += [project + "/frontend/" + name for name in
                            ("src", "public", "tests", "vendor", "scripts", "deploy", "dist")]
            roots.append(project + "/backend")
        # SDK tests inspect both current generated outputs and immutable archives.
        # The full source root remains closed while excluding dependencies below.
        directories += ["saas-micro-app-sdk/" + name for name in
                        ("src", "contracts", "compat", "generated", "tools", "scripts", "tests", "releases")]
        for example in ("websocket", "micro-vue", "micro-jquery", "report-center"):
            directories += ["saas-examples/" + example + "/" + name for name in
                            ("src", "public", "tests", "vendor", "scripts")]
        directories += ["saas-plateform/frontend/deploy", "saas-examples/scripts", "saas-examples/tests",
                        "saas-examples/websocket/dist", "saas-examples/websocket/backend/src",
                        "saas-examples/websocket/backend/tests"]
        roots += ["saas-examples", "saas-examples/websocket/backend"]
    files = set()
    forbidden = {".git", "target", "node_modules", "__pycache__", ".browsers"}
    for relative in directories:
        directory = workspace / relative
        if not directory.exists():
            continue
        for current, subdirectories, names in os.walk(directory, followlinks=False):
            subdirectories[:] = [name for name in subdirectories if name not in forbidden
                                 and not pathlib.Path(current, name).is_symlink()]
            for name in names:
                candidate = pathlib.Path(current, name)
                if (candidate.is_symlink() or name.startswith(".env")
                        or candidate.suffix.lower() in {".pem", ".key", ".p12", ".pfx"}):
                    continue
                if relative.endswith("identity-deployment") and candidate.suffix != ".sql":
                    continue
                if (relative.endswith("identity-recovery")
                        and candidate.suffix not in {".py", ".sql", ".md"}):
                    continue
                files.add(candidate.relative_to(workspace))
    for relative in roots:
        for name in ("Cargo.toml", "Cargo.lock", "build.rs", "rust-toolchain.toml", "rust-toolchain",
                     "catalog.sql", "config.example.json", "config.env.example", ".env.test.example",
                     "compose.yml", "compose.host-nginx.yml", "compose.env.example",
                     "compose.host-nginx.env.example", "compose.notification.yml",
                     "compose.notification-ca.yml", "compose.notification.env.example",
                     "compose.host-nginx.notification.yml", "compose.monitoring.yml",
                     "dependencies.lock.json", "config.example", "policy.example.json", "Dockerfile", ".dockerignore"):
            candidate = workspace / relative / name
            if candidate.is_file() and not candidate.is_symlink():
                files.add(candidate.relative_to(workspace))
    asset_roots = ["saas-plateform/frontend", "saas-examples/oidc-rp"]
    if include_portal or include_all_projects:
        asset_roots.append("saas-customer-portal/frontend")
    if include_all_projects:
        asset_roots += ["saas-stores/frontend", "saas-work-orders/frontend", "saas-micro-app-sdk", "saas-examples"]
        asset_roots += ["saas-examples/" + name for name in
                        ("websocket", "micro-vue", "micro-jquery", "report-center")]
    for relative in asset_roots:
        for name in ("package.json", "package-lock.json", "tsconfig.json", "vite.config.ts",
                     "vite.config.mjs", "eslint.config.js", "index.html", "server.mjs", "machine.mjs",
                     "frontend.tsx", ".node-version", ".nvmrc", "index.mjs", "index.d.mts", "release.json",
                     "Dockerfile", ".dockerignore"):
            candidate = workspace / relative / name
            if candidate.is_file() and not candidate.is_symlink():
                files.add(candidate.relative_to(workspace))
        # Source provenance includes repository npm settings. Copy only simple
        # public boolean/numeric controls, never registry credentials or URLs.
        npm_config = workspace / relative / ".npmrc"
        if npm_config.is_file() and not npm_config.is_symlink():
            lines = [line.strip() for line in npm_config.read_text().splitlines()
                     if line.strip() and not line.lstrip().startswith(("#", ";"))]
            if not all(re.fullmatch(r"[a-z][a-z-]*=(?:true|false|[0-9]+)", line)
                       and not re.search(r"auth|token|password|username|key|cert", line, re.I) for line in lines):
                raise SystemExit("Only public numeric or boolean project npm settings may be frozen")
            files.add(npm_config.relative_to(workspace))
        for pattern in ("tsconfig.*.json", "*.test.cjs", "smoke-test.cjs"):
            for candidate in (workspace / relative).glob(pattern):
                if candidate.is_file() and not candidate.is_symlink():
                    files.add(candidate.relative_to(workspace))
    return sorted(files)


def frontend_outputs(values):
    outputs = {}
    for value in values:
        root_name, separator, raw_path = value.partition("=")
        if not separator or root_name not in FRONTEND_ROOTS or root_name in outputs:
            raise SystemExit("A unique supported frontend root and explicit build directory are required")
        directory = pathlib.Path(raw_path)
        if not directory.is_absolute() or directory.is_symlink():
            raise SystemExit("Frontend build outputs must be absolute real directories")
        directory = directory.resolve(strict=True)
        if not directory.is_dir() or not (directory / "index.html").is_file():
            raise SystemExit("A completed frontend build with index.html is required")
        entries = []
        for current, subdirectories, names in os.walk(directory, followlinks=False):
            for name in subdirectories + names:
                candidate = pathlib.Path(current, name)
                if candidate.is_symlink():
                    raise SystemExit("Frontend build output links are rejected")
                if candidate.is_file():
                    if name.startswith(".env") or candidate.suffix.lower() in {".pem", ".key", ".p12", ".pfx"}:
                        raise SystemExit("Frontend build output contains a forbidden private file")
                    entries.append(candidate.relative_to(directory))
        outputs[root_name] = (directory, sorted(entries))
    return outputs


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("copy", "build"))
    parser.add_argument("--task-dir", type=pathlib.Path, required=True)
    parser.add_argument("--label", required=True)
    parser.add_argument("--workspace", type=pathlib.Path)
    parser.add_argument("--include-portal", action="store_true")
    parser.add_argument("--include-all-projects", action="store_true",
                        help="Freeze all eight repositories and build current business services, notification and CLI")
    parser.add_argument("--frontend-output", action="append", default=[], metavar="PROJECT=ABSOLUTE_DIRECTORY",
                        help="Copy a newly built private output into the snapshot's PROJECT/dist without changing live dist")
    parser.add_argument("--require-frontend-outputs", action="store_true",
                        help="Require explicit new build outputs for all eight frontends in the final candidate")
    parser.add_argument("--rust-bin", type=pathlib.Path)
    parser.add_argument("--cargo-home", type=pathlib.Path)
    parser.add_argument("--pg-bin", type=pathlib.Path,
                        help="Attest the exact PostgreSQL tools used by an offline recovery rehearsal")
    parser.add_argument("--service", choices=("identity", "platform", "portal", "stores", "work-orders", "notification", "cli"),
                        help="Build one component; the attestation covers only its binaries")
    args = parser.parse_args()
    task = args.task_dir.resolve()
    if (task.parent != pathlib.Path("/tmp") or not task.name.startswith("rust-saas-unified-auth-")
            or not (task / "OWNER").is_file() or not re.fullmatch(r"[a-zA-Z0-9_-]+", args.label)):
        raise SystemExit("An owned task directory and safe snapshot label are required")
    snapshot = task / ("source-snapshot-" + args.label)
    manifest_file = task / ("snapshot-" + args.label + ".json")
    if args.mode == "copy":
        if args.workspace is None or snapshot.exists() or manifest_file.exists():
            raise SystemExit("An explicit source workspace and a new snapshot label are required")
        workspace = args.workspace.resolve(strict=True)
        output_directories = frontend_outputs(args.frontend_output)
        if args.require_frontend_outputs and (not args.include_all_projects or set(output_directories) != FRONTEND_ROOTS):
            raise SystemExit("The final full candidate requires all eight explicit frontend build outputs")
        snapshot.mkdir(mode=0o700)
        inputs = {}
        for relative in source_files(workspace, args.include_portal, args.include_all_projects):
            if any(relative.is_relative_to(pathlib.Path(name) / "dist") for name in output_directories):
                continue
            source, destination = workspace / relative, snapshot / relative
            data = source.read_bytes()
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)
            # Frozen-source readers require exact private-file permissions,
            # independent of the caller's inherited umask.
            destination.chmod(0o600)
            inputs[relative.as_posix()] = hashlib.sha256(data).hexdigest()
        output_manifests = {}
        for name, (directory, entries) in output_directories.items():
            output_inputs = {}
            for relative in entries:
                data = (directory / relative).read_bytes()
                target = pathlib.Path(name) / "dist" / relative
                destination = snapshot / target
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(data)
                destination.chmod(0o600)
                inputs[target.as_posix()] = hashlib.sha256(data).hexdigest()
                output_inputs[relative.as_posix()] = inputs[target.as_posix()]
            output_manifests[name] = {"inputs": output_inputs,
                "sha256": hashlib.sha256(json.dumps(output_inputs, sort_keys=True).encode()).hexdigest()}
        manifest = {
            "copiedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(),
            "includePortal": args.include_portal or args.include_all_projects,
            "includeAllProjects": args.include_all_projects,
            "completeFrontendOutputs": set(output_directories) == FRONTEND_ROOTS,
            "frontendOutputs": output_manifests, "inputs": inputs,
            "snapshotSha256": hashlib.sha256(json.dumps(inputs, sort_keys=True).encode()).hexdigest(),
        }
        manifest_file.write_text(json.dumps(manifest, indent=2))
        manifest_file.chmod(0o600)
        print("TASK_SOURCE_SNAPSHOT_READY " + args.label + " sha256=" + manifest["snapshotSha256"])
        return

    if args.rust_bin is None or args.cargo_home is None:
        raise SystemExit("Explicit reusable Rust binary and Cargo cache directories are required")
    manifest = json.loads(manifest_file.read_text())
    for relative, expected in manifest["inputs"].items():
        if digest(snapshot / relative) != expected:
            raise SystemExit("Snapshot input changed before compilation")
    environment = {
        "HOME": os.environ.get("HOME", "/tmp"), "LANG": "C.UTF-8",
        "PATH": str(args.rust_bin.resolve(strict=True)) + ":/usr/local/bin:/usr/bin:/bin",
        "CARGO_HOME": str(args.cargo_home.resolve(strict=True)),
        "CARGO_TARGET_DIR": str(task / "target-linux"),
        # Each immutable snapshot has a new source path. Its local incremental
        # state will not help the next snapshot; shared dependency outputs do.
        "CARGO_INCREMENTAL": "0",
    }
    toolchain = {
        name: subprocess.check_output([name, "--version"], env=environment, text=True).strip()
        for name in ("rustc", "cargo")
    }
    commands = [
        ("identity", "saas-identity", ("saas-identity",)),
        ("platform", "saas-plateform/backend", ("saas-platform-backend", "platform-migrate")),
    ]
    if manifest["includePortal"]:
        commands.append(("portal", "saas-customer-portal/backend", ("saas-customer-portal-backend", "portal-migrate")))
    if manifest.get("includeAllProjects", False):
        commands += [
            ("stores", "saas-stores/backend", ("stores-api", "stores-migrate")),
            ("work-orders", "saas-work-orders/backend", ("work-orders-api", "work-orders-migrate")),
            ("notification", "saas-notification", ("saas-notification-ws",)),
            ("cli", "saas-plateform/backend/tools/platform-admin", ("saas-platform-admin",)),
        ]
    if args.service is not None:
        commands = [command for command in commands if command[0] == args.service]
        if not commands:
            raise SystemExit("Requested component is not included in the snapshot")
    binaries = {}
    build_commands = []
    for _service, relative, names in commands:
        command = ["cargo", "build", "--offline", "--locked", "-j", "2"]
        for name in names:
            command += ["--bin", name]
        started_at = datetime.datetime.now(datetime.timezone.utc).isoformat()
        started = time.monotonic()
        subprocess.run(command, cwd=snapshot / relative, env=environment, check=True)
        build_commands.append({"component": _service, "directory": relative, "command": command,
            "startedAt": started_at, "durationSeconds": round(time.monotonic() - started, 3), "exitCode": 0})
        for name in names:
            destination = task / "bin" / name
            destination.parent.mkdir(exist_ok=True)
            pending = destination.with_suffix(".next")
            shutil.copyfile(task / "target-linux/debug" / name, pending)
            pending.chmod(0o700)
            pending.replace(destination)
            binaries[name] = digest(destination)
    auxiliary = []
    probe = snapshot / "saas-plateform/backend/tests/unified-auth-https/phase23-db-auth-probe"
    if args.service is None and (probe / "Cargo.toml").is_file():
        command = ["cargo", "build", "--offline", "--locked", "-j", "2", "--bin", "phase23-db-auth-probe"]
        started_at = datetime.datetime.now(datetime.timezone.utc).isoformat()
        started = time.monotonic()
        subprocess.run(command, cwd=probe, env=environment, check=True)
        build_commands.append({"component": "db-auth-probe", "directory": probe.relative_to(snapshot).as_posix(),
            "command": command, "startedAt": started_at,
            "durationSeconds": round(time.monotonic() - started, 3), "exitCode": 0})
        destination = task / "bin/phase23-db-auth-probe"
        pending = destination.with_suffix(".next")
        shutil.copyfile(task / "target-linux/debug/phase23-db-auth-probe", pending)
        pending.chmod(0o700)
        pending.replace(destination)
        binaries[destination.name] = digest(destination)
        auxiliary.append(destination.name)
    for relative, expected in manifest["inputs"].items():
        if digest(snapshot / relative) != expected:
            raise SystemExit("Snapshot input changed during compilation")
    result = {
        "builtAt": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "snapshotLabel": args.label, "snapshotSha256": manifest["snapshotSha256"],
        "inputs": manifest["inputs"], "binaries": binaries, "toolchain": toolchain,
        "incremental": False, "builtServices": [command[0] for command in commands],
        "profile": "dev", "offline": True, "locked": True, "buildCommands": build_commands,
        "includeAllProjects": manifest.get("includeAllProjects", False),
        "completeFrontendOutputs": manifest.get("completeFrontendOutputs", False),
        "frontendOutputs": manifest.get("frontendOutputs", {}),
        "builtAuxiliaryBinaries": auxiliary,
        "python": {"executable": sys.executable, "version": sys.version,
                   "sha256": digest(pathlib.Path(sys.executable).resolve(strict=True))},
    }
    if args.pg_bin is not None:
        pg_bin = args.pg_bin.resolve(strict=True)
        result["recoveryTools"] = {}
        for name in ("pg_dump", "pg_restore", "psql"):
            executable = pg_bin / name
            result["recoveryTools"][name] = {
                "path": str(executable), "sha256": digest(executable),
                "version": subprocess.check_output([str(executable), "--version"], text=True).strip(),
            }
        openssl = pathlib.Path(shutil.which("openssl") or "/unavailable").resolve(strict=True)
        result["recoveryTools"]["openssl"] = {"path": str(openssl), "sha256": digest(openssl),
            "version": subprocess.check_output([str(openssl), "version"], text=True).strip()}
    for key in ("parentSnapshotSha256", "derivationPatchSha256", "changedFiles"):
        if key in manifest:
            result[key] = manifest[key]
    attestation = task / ("build-attestation-" + args.label + ".json")
    attestation.write_text(json.dumps(result, indent=2))
    attestation.chmod(0o600)
    print("TASK_SNAPSHOT_BINARIES_READY " + args.label + " sha256=" + manifest["snapshotSha256"])
    for name, sha256 in binaries.items():
        print(name + " " + sha256)


if __name__ == "__main__":
    main()
