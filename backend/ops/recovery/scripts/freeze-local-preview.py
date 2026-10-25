#!/usr/bin/env python3
"""Freeze the six reviewed WSL preview writers, retaining private evidence.

Default: read-only preflight; a new private /dev/shm report is the only output.
To freeze and seal, run as root in the local WSL distribution:

  python3 freeze-local-preview.py --execute \
    --recovery-bin /absolute/path/saas-recovery \
    --cipher-output /existing/private/directory/new-freeze.enc

No database names, roles, Docker endpoint, or writer names are configurable.
After an interrupted stop stage, --execute --resume-stage /dev/shm/original-stage
reuses its recorded process evidence and key; it never starts a replacement.
Failures retain private evidence and maintenance state; there is no automatic
restart, LOGIN restoration, database deletion, or remote connection. The key is
created in a separate private tmpfs directory and is never put in the archive.
"""

from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import secrets
import shutil
import stat
import subprocess
import sys
import tarfile
import time


WRITERS = (
    "qiankun-platform-preview-20261024",
    "qiankun-stores-preview-20261024",
    "codex-qiankun-orders-preview-api-20261024",
    "qiankun-preview-notification-20261024",
    "qiankun-preview-examples-oauth-20261024",
    "qiankun-preview-examples-chat-20261024",
)
STOP_ORDER = (WRITERS[4], WRITERS[5], WRITERS[2], WRITERS[1], WRITERS[3], WRITERS[0])
DATABASES = (
    ("platform", "qiankun_platform_preview_20261024"),
    ("stores", "qiankun_stores_preview_20261024"),
    ("work_orders", "qiankun_orders_preview_20261024"),
)
ROLES = ("qiankun_preview_20261024", "qiankun_notification_preview_20261024")
SERVICE_EXE = dict(zip(WRITERS, (
    "saas-platform-backend", "stores-api", "work-orders-api",
    "saas-notification-ws", "oauth-playground-api", "saas-chat-reference",
)))
SERVICE_LABEL = dict(zip(WRITERS, ("platform", "stores", "orders", "notification", "oauth", "chat")))
PROFILE = "uuid-local-preview"
SOCKET = "/var/run/postgresql"
DOCKER_SOCKET = "unix:///var/run/docker.sock"
TMPFS = Path("/dev/shm")
RESERVE_BYTES = 64 * 1024 * 1024
CACHE_NAMES = {".git", "node_modules", "target", ".cargo", "__pycache__"}
PATH_ENV_KEYS = {
    "APP_ENV_FILE", "ENV_FILE", "CONFIG_FILE", "CONFIG_PATH", "CONFIG_DIR",
    "ARTIFACT_ROOT", "ARTIFACT_STORAGE_ROOT", "RELEASE_ROOT", "RELEASES_ROOT",
    "STATIC_ROOT", "STATIC_DIR", "PUBLIC_DIR", "FRONTEND_ROOT", "FRONTEND_DIR",
    "RUNTIME_ROOT", "RUNTIME_DIR", "SIGNING_KEY_FILE", "PRIVATE_KEY_FILE",
    "MANAGED_DEPLOYMENT_CONFIG_FILE", "ZIP_DISTRIBUTION_CONFIG_FILE",
}
BASE_ENV = {
    "PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LANG": "C.UTF-8",
    "LC_ALL": "C.UTF-8", "HOME": "/nonexistent",
}


class FreezeError(Exception):
    """Static public error; command output is retained only in private evidence."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise FreezeError(message)


def now_ms() -> int:
    return time.time_ns() // 1_000_000


def normalized(path: Path) -> Path:
    require(path.is_absolute() and ".." not in path.parts and "." not in path.parts,
            "absolute normalized paths required")
    for part in reversed((path, *path.parents)):
        if part.exists() or part.is_symlink():
            require(not part.is_symlink(), "output symlink paths rejected")
    return path


def metadata(path: Path, follow: bool = False) -> dict:
    value = path.stat() if follow else path.lstat()
    return {
        "device": value.st_dev, "inode": value.st_ino,
        "uid": value.st_uid, "gid": value.st_gid,
        "mode": oct(stat.S_IMODE(value.st_mode)), "bytes": value.st_size,
        "mtime_ns": value.st_mtime_ns,
    }


def private(path: Path, directory: bool = False) -> None:
    normalized(path)
    value = path.lstat()
    require(value.st_uid == os.geteuid(), "private path owner mismatch")
    require(stat.S_IMODE(value.st_mode) == (0o700 if directory else 0o600),
            "private path permissions mismatch")
    require(stat.S_ISDIR(value.st_mode) if directory else stat.S_ISREG(value.st_mode),
            "private regular file or directory required")


def require_tmpfs(path: Path) -> None:
    # Linux statfs starts with a signed native long f_type. An oversized aligned
    # buffer avoids depending on the architecture-specific remainder of statfs.
    info = (ctypes.c_long * 128)()
    libc = ctypes.CDLL(None, use_errno=True)
    require(libc.statfs(os.fsencode(path), ctypes.byref(info)) == 0,
            "tmpfs inspection failed")
    require(info[0] == 0x01021994, "private staging must be tmpfs")


def new_directory(path: Path) -> Path:
    normalized(path)
    path.mkdir(mode=0o700)
    private(path, directory=True)
    return path


def reserve_bytes(count: int) -> None:
    require(shutil.disk_usage(TMPFS).free >= count + RESERVE_BYTES,
            "insufficient private tmpfs space; evidence retained")


def write_private(path: Path, value: bytes) -> None:
    private(path.parent, directory=True)
    reserve_bytes(len(value))
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW
    with os.fdopen(os.open(path, flags, 0o600), "wb") as output:
        output.write(value)
        output.flush()
        os.fsync(output.fileno())
    private(path)


def write_json(path: Path, value: object) -> None:
    write_private(path, (json.dumps(value, ensure_ascii=True, indent=2) + "\n").encode())


def digest(path: Path) -> str:
    result = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            result.update(chunk)
    return result.hexdigest()


def copy_regular(source: Path, target: Path) -> dict:
    before = source.stat()
    require(stat.S_ISREG(before.st_mode), "snapshot source is not a regular file")
    reserve_bytes(before.st_size)
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW
    checksum = hashlib.sha256()
    with source.open("rb") as reader, os.fdopen(os.open(target, flags, 0o600), "wb") as writer:
        opened = os.fstat(reader.fileno())
        require((before.st_dev, before.st_ino) == (opened.st_dev, opened.st_ino),
                "snapshot source identity changed")
        while chunk := reader.read(1024 * 1024):
            writer.write(chunk)
            checksum.update(chunk)
        writer.flush()
        os.fsync(writer.fileno())
        after = os.fstat(reader.fileno())
    current = source.stat()
    require((opened.st_dev, opened.st_ino, opened.st_size, opened.st_mtime_ns)
            == (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns)
            == (current.st_dev, current.st_ino, current.st_size, current.st_mtime_ns),
            "snapshot source changed during capture")
    private(target)
    return {"source": str(source), "source_metadata": metadata(source, follow=True),
            "sha256": checksum.hexdigest(), "captured_bytes": target.stat().st_size}


class Runner:
    def __init__(self, logs: Path):
        self.logs = logs
        self.sequence = 0
        self.writer_ids: dict[str, str] = {}
        self.docker = shutil.which("docker", path=BASE_ENV["PATH"])
        self.psql = shutil.which("psql", path=BASE_ENV["PATH"])
        self.runuser = shutil.which("runuser", path=BASE_ENV["PATH"])
        require(bool(self.docker and self.psql and self.runuser), "required local tools unavailable")

    def run(self, command: list[str], label: str, data: bytes | None = None,
            timeout: int = 120, env: dict | None = None) -> bytes:
        self.sequence += 1
        name = f"{self.sequence:04d}-{label}"
        try:
            result = subprocess.run(command, input=data, stdout=subprocess.PIPE,
                                    stderr=subprocess.PIPE, env=env or BASE_ENV,
                                    timeout=timeout, check=False)
        except subprocess.TimeoutExpired as error:
            write_private(self.logs / f"{name}.stdout", error.stdout or b"")
            write_private(self.logs / f"{name}.stderr", error.stderr or b"")
            raise FreezeError("local command timed out; inspect private evidence") from None
        write_private(self.logs / f"{name}.stdout", result.stdout)
        write_private(self.logs / f"{name}.stderr", result.stderr)
        write_json(self.logs / f"{name}.result.json", {"returncode": result.returncode})
        require(result.returncode == 0, "local command failed; inspect private evidence")
        return result.stdout

    def docker_command(self, args: list[str], label: str, timeout: int = 120) -> bytes:
        return self.run([self.docker, "--host", DOCKER_SOCKET, *args], label, timeout=timeout)

    def inspect(self, label: str) -> list[dict]:
        targets = [self.writer(name) for name in WRITERS] if self.writer_ids else list(WRITERS)
        raw = self.docker_command(["inspect", *targets], label)
        value = json.loads(raw)
        require(len(value) == len(WRITERS)
                and {item["Name"].removeprefix("/") for item in value} == set(WRITERS),
                "exact reviewed writer inventory required")
        found = {}
        for item in value:
            name = item["Name"].removeprefix("/")
            identifier = item["Id"]
            require(bool(re.fullmatch("[0-9a-f]{64}", identifier)), "full Docker identity required")
            labels = item["Config"].get("Labels") or {}
            labelled = (labels.get("owner") == "codex-local-runtime-20261024"
                        and labels.get("app") == "qiankun-preview"
                        and labels.get("service") == SERVICE_LABEL[name])
            legacy_notification = (name == WRITERS[3] and identifier.startswith("49ce6d1fe7c1")
                                   and item["Config"]["Image"] == "saas-ci:phase04-tools")
            require(labelled or legacy_notification, "reviewed preview ownership gate failed")
            require(not self.writer_ids or self.writer_ids[name] == identifier,
                    "preview container was replaced during freeze")
            found[name] = identifier
        if not self.writer_ids:
            self.writer_ids = found
        return value

    def writer(self, name: str) -> str:
        require(name in WRITERS and name in self.writer_ids, "unpinned preview writer rejected")
        return self.writer_ids[name]

    def sql(self, query: str, label: str, mutate: bool = False,
            database: str = DATABASES[0][1]) -> object:
        # env -i after runuser prevents hidden libpq service/password/host settings.
        options = "-c statement_timeout=30000 -c lock_timeout=10000 "
        options += "-c default_transaction_read_only=" + ("off" if mutate else "on")
        require(database in {name for _, name in DATABASES}, "non-source database rejected")
        command = [self.runuser, "-u", "postgres", "--", "/usr/bin/env", "-i",
                   "PATH=" + BASE_ENV["PATH"], "LC_ALL=C.UTF-8", "PGCONNECT_TIMEOUT=10",
                   "PGAPPNAME=saas-uuid-preview-freeze", "PSQL_HISTORY=/dev/null",
                   "PGOPTIONS=" + options, self.psql, "-X", "-q", "-A", "-t",
                   "--no-password", "-v", "ON_ERROR_STOP=1", "-h", SOCKET, "-p", "5432",
                   "-U", "postgres", "-d", database, "-f", "-"]
        raw = self.run(command, label, query.encode(), timeout=180)
        if mutate:
            return {"completed": True}
        try:
            return json.loads(raw)
        except (ValueError, UnicodeDecodeError):
            raise FreezeError("invalid private database evidence") from None


def database_inventory(runner: Runner, label: str) -> dict:
    names = ",".join("'" + name + "'" for _, name in DATABASES)
    roles = ",".join("'" + name + "'" for name in ROLES)
    return runner.sql(f"""
SELECT json_build_object(
  'current_user', current_user, 'database', current_database(),
  'local_socket', inet_server_addr() IS NULL,
  'port', current_setting('port')::integer,
  'admin_superuser', (SELECT rolsuper FROM pg_roles WHERE rolname=current_user),
  'databases', (SELECT json_agg(json_build_object(
      'name',d.datname,'owner',r.rolname,'owner_superuser',r.rolsuper,
      'settings',COALESCE((SELECT s.setconfig FROM pg_db_role_setting s
                  WHERE s.setdatabase=d.oid AND s.setrole=0),ARRAY[]::text[])))
      FROM pg_database d JOIN pg_roles r ON r.oid=d.datdba WHERE d.datname IN ({names})),
  'roles', (SELECT json_agg(json_build_object('name',rolname,'superuser',rolsuper,
                    'login',rolcanlogin)) FROM pg_roles WHERE rolname IN ({roles})),
  'connect_roles', (SELECT COALESCE(json_agg(DISTINCT r.rolname),'[]'::json)
      FROM pg_database d JOIN pg_roles r ON r.rolcanlogin AND NOT r.rolsuper
      WHERE d.datname IN ({names}) AND has_database_privilege(r.oid,d.oid,'CONNECT')),
  'prepared_transactions', (SELECT count(*) FROM pg_prepared_xacts WHERE database IN ({names})),
  'backends', (SELECT COALESCE(json_agg(json_build_object(
      'pid',pid,'database',datname,'user',usename,'client_addr',client_addr,
      'application_name',application_name,'state',state,'backend_type',backend_type)),'[]'::json)
      FROM pg_stat_activity WHERE datname IN ({names}) AND pid<>pg_backend_pid()));
""", label)


def validate_database_inventory(value: dict, frozen: bool = False, allow_fenced: bool = False) -> None:
    require(value.get("current_user") == "postgres" and value.get("admin_superuser") is True
            and value.get("local_socket") is True and value.get("port") == 5432
            and value.get("database") == DATABASES[0][1], "local postgres admin identity mismatch")
    databases = value.get("databases") or []
    require({item["name"] for item in databases} == {name for _, name in DATABASES}
            and len(databases) == 3, "source database inventory mismatch")
    require(all(item["owner"] in ROLES and item["owner_superuser"] is False for item in databases),
            "source database ownership mismatch")
    roles = value.get("roles") or []
    require({item["name"] for item in roles} == set(ROLES) and len(roles) == 2
            and all(item["superuser"] is False for item in roles), "service role privilege mismatch")
    require(value.get("prepared_transactions") == 0, "prepared source transaction prevents freeze")
    require(set(value.get("connect_roles") or []) <= set(ROLES),
            "unexpected non-superuser source login role")
    if frozen:
        require(all(item["login"] is False for item in roles), "service NOLOGIN verification failed")
        require(all("default_transaction_read_only=on" in item["settings"] for item in databases),
                "source read-only default verification failed")
        require(not value.get("backends"), "source database still has active backends")
    else:
        require(allow_fenced or all(item["login"] is True for item in roles),
                "source roles already fenced; inspect previous maintenance evidence")
        require(all(item.get("user") in (*ROLES, "postgres") for item in value.get("backends", [])),
                "unexpected source backend role")


def parse_environment(raw: bytes) -> dict[str, str]:
    result = {}
    for item in raw.split(b"\0"):
        if b"=" in item:
            key, value = item.split(b"=", 1)
            result[key.decode("utf-8", "surrogateescape")] = value.decode("utf-8", "surrogateescape")
    return result


def process_start(pid: int) -> str:
    # comm can contain spaces and closing parentheses; fields after its final ')'
    # begin with field 3. starttime is field 22.
    raw = Path(f"/proc/{pid}/stat").read_text()
    return raw[raw.rfind(")") + 2:].split()[19]


def container_path(pid: int, value: str, cwd: str = "/") -> Path:
    path = PurePosixPath(value)
    if not path.is_absolute():
        path = PurePosixPath(cwd) / path
    require(".." not in path.parts, "container material path traversal rejected")
    return Path(f"/proc/{pid}/root") / str(path).lstrip("/")


def capture_processes(runner: Runner, containers: list[dict], root: Path,
                      copy_executables: bool) -> tuple[list[dict], list[tuple[str, Path]]]:
    records, extra_sources = [], []
    for container in containers:
        name = container["Name"].removeprefix("/")
        location = new_directory(root / name)
        top = runner.docker_command(["top", runner.writer(name), "-eo", "pid,ppid,comm"], "top-" + name)
        rows = top.decode().splitlines()[1:]
        rust_count, node_count = 0, 0
        process_rows = []
        for row in rows:
            fields = row.split(None, 2)
            require(len(fields) == 3 and fields[0].isdigit(), "unexpected docker top format")
            pid = int(fields[0])
            proc = Path(f"/proc/{pid}")
            start = process_start(pid)
            exe = os.readlink(proc / "exe")
            basename = PurePosixPath(exe.removesuffix(" (deleted)")).name
            rust = basename == SERVICE_EXE[name]
            node = basename in {"node", "nodejs"}
            details = {"host_pid": pid, "host_ppid": int(fields[1]), "comm": fields[2],
                       "starttime_ticks": start, "exe_link": exe,
                       "executable_metadata": metadata(proc / "exe", follow=True),
                       "service_rust": rust, "node": node}
            process_rows.append(details)
            if not (rust or node):
                continue
            rust_count += int(rust)
            node_count += int(node)
            destination = new_directory(location / str(pid))
            cmdline = (proc / "cmdline").read_bytes()
            environ = (proc / "environ").read_bytes()
            write_private(destination / "cmdline.bin", cmdline)
            write_private(destination / "environ.bin", environ)
            write_private(destination / "stat.txt", (proc / "stat").read_bytes())
            write_private(destination / "cgroup.txt", (proc / "cgroup").read_bytes())
            cwd = os.readlink(proc / "cwd")
            details["cwd"] = cwd
            details["argv"] = [value.decode("utf-8", "surrogateescape")
                               for value in cmdline.split(b"\0") if value]
            environment = parse_environment(environ)
            details["path_environment"] = {key: value for key, value in environment.items()
                                           if key in PATH_ENV_KEYS
                                           or key.endswith(("_ENV_FILE", "_CONFIG_FILE"))}
            if copy_executables:
                details["executable_copy"] = copy_regular(proc / "exe", destination / "executable.bin")
            for key, value in details["path_environment"].items():
                if not value:
                    continue
                require(not "://" in value, "path environment unexpectedly contains URL")
                source = container_path(pid, value, cwd)
                require(source.exists(), "configured material path is missing")
                extra_sources.append((f"{name}-{pid}-{key}", source))
            if node:
                require(not cwd.endswith(" (deleted)"), "Node workspace has been deleted")
                extra_sources.append((f"{name}-{pid}-node-workspace", container_path(pid, cwd)))
                scripts = [value for value in details["argv"][1:]
                           if value.endswith((".js", ".mjs", ".cjs", ".json"))]
                require(bool(scripts), "Node script entry not identified")
                for index, value in enumerate(scripts):
                    source = container_path(pid, value, cwd)
                    require(source.is_file(), "Node script or configuration is missing")
                    extra_sources.append((f"{name}-{pid}-node-script-{index}", source))
            require(process_start(pid) == start
                    and metadata(proc / "exe", follow=True) == details["executable_metadata"],
                    "service process identity changed during capture")
        if name == WRITERS[2]:
            require(node_count == 1 and rust_count in {0, 1},
                    "actual orders Node service process not identified")
        else:
            require(rust_count == 1, "exact actual Rust service process not identified")
        records.append({"container": name, "init_host_pid": container["State"]["Pid"],
                        "processes": process_rows})
    return records, extra_sources


def verify_running_generation(runner: Runner, before: list[dict], processes: list[dict]) -> None:
    current = runner.inspect("inspect-before-mutation")
    old = {item["Name"]: item for item in before}
    for item in current:
        prior = old[item["Name"]]
        require(item["Id"] == prior["Id"] and item["State"].get("Running") is True
                and item["State"]["Pid"] == prior["State"]["Pid"]
                and item["State"]["StartedAt"] == prior["State"]["StartedAt"]
                and item["RestartCount"] == prior["RestartCount"],
                "writer generation changed before freeze")
    for container in processes:
        top = runner.docker_command(["top", runner.writer(container["container"]), "-eo", "pid,ppid,comm"],
                                    "top-recheck-" + container["container"])
        pids = {int(row.split(None, 1)[0]) for row in top.decode().splitlines()[1:]}
        require(pids == {row["host_pid"] for row in container["processes"]},
                "writer process inventory changed before freeze")
        for row in container["processes"]:
            if row["service_rust"] or row["node"]:
                proc = Path(f"/proc/{row['host_pid']}")
                require(process_start(row["host_pid"]) == row["starttime_ticks"]
                        and metadata(proc / "exe", follow=True) == row["executable_metadata"],
                        "writer executable identity changed before freeze")


def mount_sources(containers: list[dict]) -> tuple[list[dict], list[tuple[str, Path]]]:
    inventory, sources = [], []
    seen = set()
    for container in containers:
        name = container["Name"].removeprefix("/")
        for index, mount in enumerate(container.get("Mounts", [])):
            source = Path(mount["Source"])
            entry = {"container": name, **mount}
            require(source.is_absolute() and source.exists(), "mounted source is unavailable")
            entry["source_metadata"] = metadata(source, follow=True)
            destination_parts = PurePosixPath(mount["Destination"]).parts
            if "cache" in destination_parts or any(cache_name(part) for part in destination_parts):
                entry["capture"] = "build-cache-excluded"
            elif (source.stat().st_dev, source.stat().st_ino) in seen:
                entry["capture"] = "same-mounted-source-already-scheduled"
            else:
                require(not str(source).startswith(("/var/lib/postgresql", "/var/lib/docker/overlay2")),
                        "unreviewed mounted filesystem rejected")
                entry["capture"] = "snapshot-non-cache-materials"
                seen.add((source.stat().st_dev, source.stat().st_ino))
                sources.append((f"{name}-mount-{index}", source))
            inventory.append(entry)
    return inventory, sources


def cache_name(name: str) -> bool:
    return name in CACHE_NAMES or name.endswith(("-target", "_target"))


def root_is_cache(source: Path) -> bool:
    return source.is_dir() and any(cache_name(part) for part in source.parts)


def source_is_cache(label: str, source: Path) -> bool:
    return label.startswith("cache-root-") or root_is_cache(source)


def snapshot_source(source: Path, target: Path, allowed_roots: list[Path],
                    exclusions: list[dict], copies: list[dict], ancestors: frozenset = frozenset()) -> None:
    value = source.lstat()
    if stat.S_ISLNK(value.st_mode):
        resolved = source.resolve(strict=True)
        require(any(resolved == root or resolved.is_relative_to(root) for root in allowed_roots),
                "material symlink leaves reviewed source roots")
        exclusions.append({"source": str(source), "symlink": os.readlink(source),
                           "captured_resolved_target": str(resolved)})
        return snapshot_source(resolved, target, allowed_roots, exclusions, copies, ancestors)
    if stat.S_ISREG(value.st_mode):
        copies.append({"archive_path": str(target), **copy_regular(source, target)})
    elif stat.S_ISDIR(value.st_mode):
        identity = (value.st_dev, value.st_ino)
        require(identity not in ancestors, "material directory cycle rejected")
        new_directory(target)
        for child in sorted(source.iterdir()):
            if cache_name(child.name):
                exclusions.append({"source": str(child), "reason": "rebuildable-cache",
                                   "metadata": metadata(child)})
                continue
            snapshot_source(child, target / child.name, allowed_roots, exclusions, copies,
                            ancestors | {identity})
    else:
        exclusions.append({"source": str(source), "reason": "special-file-not-material",
                           "metadata": metadata(source)})


def resolve_live_sources(sources: list[tuple[str, Path]], mounts: list[dict]) -> list[tuple[str, Path]]:
    """Replace /proc/PID/root mounted paths before the process disappears."""
    result = []
    for label, source in sources:
        parts = source.parts
        require(len(parts) >= 5 and parts[1] == "proc" and parts[3] == "root",
                "unexpected process material path")
        container_value = PurePosixPath("/", *parts[4:])
        name = next((name for name in WRITERS if label.startswith(name + "-")), None)
        matches = [mount for mount in mounts if mount["container"] == name
                   and (container_value == PurePosixPath(mount["Destination"])
                        or container_value.is_relative_to(PurePosixPath(mount["Destination"])))]
        if matches:
            mount = max(matches, key=lambda item: len(PurePosixPath(item["Destination"]).parts))
            relative = container_value.relative_to(PurePosixPath(mount["Destination"]))
            mapped = Path(mount["Source"]) / relative
            cache_directory = (mapped.is_dir()
                               and ("cache" in container_value.parts
                                    or any(cache_name(part) for part in container_value.parts)))
            result.append((("cache-root-" if cache_directory else "") + label, mapped))
        else:
            # Unmounted image files must be copied while /proc/PID/root exists.
            result.append(("live-image-" + label, source))
    return result


def capture_sources(sources: list[tuple[str, Path]], root: Path) -> dict:
    exclusions, copies, scheduled, seen = [], [], [], set()
    for label, source in sources:
        require(source.exists(), "material source disappeared")
        if source_is_cache(label, source):
            exclusions.append({"source": str(source), "label": label,
                               "reason": "rebuildable-cache-root", "metadata": metadata(source)})
            continue
        identity = (source.stat().st_dev, source.stat().st_ino)
        if identity in seen:
            exclusions.append({"source": str(source), "label": label, "reason": "same-source-captured"})
            continue
        seen.add(identity)
        scheduled.append((label, source))
    allowed = [source.resolve(strict=True) for _, source in scheduled]
    for label, source in scheduled:
        snapshot_source(source, root / label, allowed, exclusions, copies)
    return {"sources": [{"label": label, "path": str(source)} for label, source in scheduled],
            "copies": copies, "exclusions": exclusions}


def estimate_materials(sources: list[tuple[str, Path]], evidence: Path) -> dict:
    """Fail before STOP if both the private snapshots and tar will not fit."""
    allowed = [source.resolve(strict=True) for label, source in sources if not source_is_cache(label, source)]
    payload, entries = 0, 0
    seen_roots = set()

    def visit(source: Path, ancestors: frozenset = frozenset()) -> None:
        nonlocal payload, entries
        value = source.lstat()
        entries += 1
        if stat.S_ISLNK(value.st_mode):
            resolved = source.resolve(strict=True)
            require(any(resolved == root or resolved.is_relative_to(root) for root in allowed),
                    "material symlink leaves reviewed source roots")
            visit(resolved, ancestors)
        elif stat.S_ISREG(value.st_mode):
            payload += value.st_size
        elif stat.S_ISDIR(value.st_mode):
            identity = (value.st_dev, value.st_ino)
            require(identity not in ancestors, "material directory cycle rejected")
            for child in source.iterdir():
                if not cache_name(child.name):
                    visit(child, ancestors | {identity})

    for label, source in sources:
        if source_is_cache(label, source):
            continue
        identity = (source.stat().st_dev, source.stat().st_ino)
        if identity not in seen_roots:
            seen_roots.add(identity)
            visit(source)
    existing = sum(path.stat().st_size for path in evidence.rglob("*") if path.is_file())
    required = 2 * payload + entries * 8192 + existing + 32 * 1024 * 1024
    reserve_bytes(required)
    return {"pending_material_bytes": payload, "pending_entries": entries,
            "existing_evidence_bytes": existing, "additional_tmpfs_budget_bytes": required,
            "available_tmpfs_bytes": shutil.disk_usage(TMPFS).free}


def fresh_effect_counts(runner: Runner, registry: dict) -> list[dict]:
    """Count actual frozen task/effect tables; old inventory is never reused."""
    registered = {}
    for database in registry["databases"]:
        tables = {(item["schema"], item["table"]) for item in database.get("tables", [])
                  if item.get("retention") == "TASK_RECONCILE"}
        tables |= {(item["schema"], item["table"]) for item in database.get("columns", [])
                   if item.get("retention") == "TASK_RECONCILE"}
        registered[database["kind"]] = tables
    require(set(registered) == {kind for kind, _ in DATABASES}, "effect registry database mismatch")
    counts = []
    for kind, name in DATABASES:
        actual = runner.sql("""SELECT COALESCE(json_agg(json_build_object('schema',n.nspname,
            'table',c.relname)),'[]'::json) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
            WHERE c.relkind IN ('r','p') AND n.nspname='public';""",
            "effect-table-inventory-" + kind, database=name)
        current = {(item["schema"], item["table"]) for item in actual}
        required = registered[kind]
        require(required <= current, "registered task table missing from frozen source")
        effects = {table for table in current if (
            table[1] in {"notifications", "notification_publications", "work_order_reminder_intents"}
            or re.search(r"(?:^|_)(?:outbox|deliveries|delivery|installation|installations)(?:_|$)", table[1]))}
        for schema, table in sorted(required | effects):
            require(bool(re.fullmatch(r"[a-z_][a-z0-9_]*", schema))
                    and bool(re.fullmatch(r"[a-z_][a-z0-9_]*", table)), "unreviewed effect table identifier")
            count = runner.sql(f'SELECT json_build_object(\'count\',count(*)) FROM "{schema}"."{table}";',
                               "effect-count-" + kind + "-" + table, database=name)["count"]
            require(isinstance(count, int) and count >= 0, "invalid frozen effect count")
            counts.append({"kind": kind, "database": name, "schema": schema, "table": table,
                           "rows": count, "task_reconcile": (schema, table) in required})
    return counts


def file_inventory(root: Path) -> list[dict]:
    result = []
    for path in sorted(root.rglob("*")):
        if path.is_dir():
            private(path, directory=True)
        else:
            private(path)
            result.append({"path": str(path.relative_to(root)), "sha256": digest(path),
                           "metadata": metadata(path)})
    return result


def make_archive(stage: Path, evidence: Path, original_stage: Path | None = None) -> Path:
    archive = stage / "freeze-materials.private.tar"
    required_bytes = sum(path.stat().st_size for path in evidence.rglob("*") if path.is_file())
    if original_stage is not None:
        required_bytes += sum(path.stat().st_size for path in (original_stage / "evidence").rglob("*")
                              if path.is_file())
    reserve_bytes(required_bytes + 16 * 1024 * 1024)
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW
    with os.fdopen(os.open(archive, flags, 0o600), "wb") as output:
        with tarfile.open(fileobj=output, mode="w|", format=tarfile.PAX_FORMAT) as tar:
            tar.add(evidence, arcname="evidence", recursive=True)
            tar.add(stage / "freeze-evidence.private.json", arcname="freeze-evidence.private.json")
            if original_stage is not None:
                tar.add(original_stage / "evidence", arcname="original/evidence", recursive=True)
                if (original_stage / "failure.private.json").exists():
                    tar.add(original_stage / "failure.private.json", arcname="original/failure.private.json")
        output.flush()
        os.fsync(output.fileno())
    private(archive)
    return archive


def quiescence(captured_ms: int, cipher_sha256: str) -> dict:
    require(len(cipher_sha256) == 64 and all(char in "0123456789abcdef" for char in cipher_sha256),
            "actual ciphertext digest required")
    current = now_ms()
    require(0 < captured_ms < (1 << 48) and 0 <= current - captured_ms <= 3_600_000,
            "freeze evidence expired before sealing")
    return {"format": "saas-uuid-quiescence/1", "profile": PROFILE,
            "writes_quiesced": True,
            "databases": [{"kind": kind, "name": name} for kind, name in DATABASES],
            "stopped_writers": list(WRITERS), "captured_ms": captured_ms,
            "writer_evidence_sha256": cipher_sha256}


def verify_stopped(containers: list[dict]) -> None:
    require(all(item["State"].get("Running") is False
                and item["State"].get("Paused") is False
                and item["State"].get("Pid") == 0
                and item["HostConfig"]["RestartPolicy"]["Name"] == "no"
                for item in containers), "writer stop or restart-policy verification failed")


def validate_archived_writers(before: list[dict]) -> dict[str, str]:
    require(len(before) == len(WRITERS)
            and {item["Name"].removeprefix("/") for item in before} == set(WRITERS),
            "exact archived writer inventory required")
    result = {}
    for item in before:
        name = item["Name"].removeprefix("/")
        identifier = item["Id"]
        require(bool(re.fullmatch("[0-9a-f]{64}", identifier)), "full archived Docker identity required")
        labels = item["Config"].get("Labels") or {}
        labelled = (labels.get("owner") == "codex-local-runtime-20261024"
                    and labels.get("app") == "qiankun-preview"
                    and labels.get("service") == SERVICE_LABEL[name])
        historical = (name == WRITERS[3] and identifier.startswith("49ce6d1fe7c1")
                      and item["Config"]["Image"] == "saas-ci:phase04-tools")
        require(labelled or historical, "archived preview ownership gate failed")
        result[name] = identifier
    return result


def verify_frozen_writers(runner: Runner, before: list[dict], processes: list[dict],
                         evidence: Path, label: str) -> dict:
    """An AutoRemove absence is separate evidence, never a synthetic inspect."""
    expected = validate_archived_writers(before)
    require(runner.writer_ids == expected, "frozen writer identity pin mismatch")
    process_by_name = {item["container"]: item for item in processes}
    require(set(process_by_name) == set(WRITERS), "original process evidence incomplete")
    present, removed = [], []
    for original in before:
        name = original["Name"].removeprefix("/")
        identifier = runner.writer(name)
        ids = runner.docker_command(["ps", "-a", "--no-trunc", "--filter", "id=" + identifier,
                                     "--format", "{{.ID}}"], label + "-id-" + name).decode().split()
        require(not ids or ids == [identifier], "unexpected Docker identity query result")
        if ids:
            actual = json.loads(runner.docker_command(["inspect", identifier], label + "-inspect-" + name))
            require(len(actual) == 1 and actual[0]["Id"] == identifier
                    and actual[0]["Name"].removeprefix("/") == name,
                    "frozen preview container identity mismatch")
            verify_stopped(actual)
            present.append(actual[0])
        else:
            require(original["HostConfig"].get("AutoRemove") is True,
                    "non-AutoRemove writer disappeared; manual investigation required")
            same_name = runner.docker_command(["ps", "-a", "--no-trunc", "--filter",
                "name=^/" + name + "$", "--format", "{{.ID}}"], label + "-name-" + name)
            require(not same_name.strip(), "removed preview name has been recreated")
            original_processes = process_by_name[name]["processes"]
            pids = [item["host_pid"] for item in original_processes]
            require(bool(pids) and all(isinstance(pid, int) and pid > 0 for pid in pids),
                    "original AutoRemove process evidence missing")
            require(all(not Path(f"/proc/{pid}").exists() for pid in pids),
                    "original AutoRemove PID exists or was reused; absence cannot be proved")
            removed.append({"kind": "RemovalVerified", "name": name, "original_id": identifier,
                            "original_auto_remove": True, "full_id_absent": True,
                            "same_name_absent": True, "original_host_pids_absent": pids,
                            "checked_ms": now_ms()})
    require(len(present) + len(removed) == len(WRITERS), "incomplete frozen writer proof")
    proof = {"present_inspected_ids": [item["Id"] for item in present], "removals": removed,
             "checked_ms": now_ms()}
    write_json(evidence / (label + ".private.json"), proof)
    return proof


def validate_options(options: argparse.Namespace) -> None:
    require(sys.platform == "linux" and "microsoft" in os.uname().release.lower(),
            "this script only supports the local WSL Linux environment")
    require(os.geteuid() == 0, "run in WSL as root to inspect actual service processes")
    require_tmpfs(TMPFS)
    if options.resume_stage:
        require(options.execute and bool(options.recovery_bin) and bool(options.cipher_output),
                "resume requires explicit execute, recovery binary and new ciphertext output")
        resumed = normalized(Path(options.resume_stage))
        require(resumed.parent == TMPFS and resumed.name.startswith("uuid-preview-freeze-")
                and not resumed.name.endswith("-key"), "exact original tmpfs freeze stage required")
        private(resumed, directory=True)
        require_tmpfs(resumed)
    require(bool(options.recovery_bin) == bool(options.cipher_output),
            "recovery binary and new ciphertext output must be supplied together")
    if options.recovery_bin:
        binary = normalized(Path(options.recovery_bin))
        require(binary.is_file() and os.access(binary, os.X_OK), "explicit recovery binary unavailable")
        output = normalized(Path(options.cipher_output))
        require(not output.exists() and not output.is_symlink(), "ciphertext output must be new")
        private(output.parent, directory=True)
        require(not output.is_relative_to(TMPFS),
                "ciphertext must be retained separately from tmpfs plaintext and key")


def run(options: argparse.Namespace, stage: Path) -> dict:
    evidence = new_directory(stage / "evidence")
    runner = Runner(new_directory(evidence / "commands"))
    before = runner.inspect("inspect-before")
    require(all(item["State"].get("Running") is True and not item["State"].get("Paused")
                for item in before), "all six reviewed preview writers must be running for initial freeze")
    db_before = database_inventory(runner, "database-before")
    validate_database_inventory(db_before)
    registry_source = Path(__file__).resolve().parents[1] / "uuid-references-v2.json"
    copy_regular(registry_source, evidence / "uuid-references-v2.json")
    registry = json.loads((evidence / "uuid-references-v2.json").read_bytes())
    effect_counts_before = fresh_effect_counts(runner, registry)
    write_json(evidence / "effect-counts-before.private.json", effect_counts_before)
    require(all(item["rows"] == 0 for item in effect_counts_before),
            "source task or external effect records require reconciliation")
    mounts, mounted_sources = mount_sources(before)
    write_json(evidence / "mounted-materials.private.json", mounts)
    processes, extra = capture_processes(runner, before,
                                         new_directory(evidence / "processes"), options.execute)
    write_json(evidence / "processes.private.json", processes)
    write_json(evidence / "preflight.private.json", {"databases": db_before,
               "restart_policies": {item["Name"]: item["HostConfig"]["RestartPolicy"] for item in before},
               "stop_order": STOP_ORDER, "tmpfs_available_bytes": shutil.disk_usage(TMPFS).free,
               "external_effect_configuration": [
                   {"container": container["container"], "host_pid": process["host_pid"],
                    "managed_deployment_configured": bool(process.get("path_environment", {}).get("MANAGED_DEPLOYMENT_CONFIG_FILE")),
                    "zip_distribution_configured": bool(process.get("path_environment", {}).get("ZIP_DISTRIBUTION_CONFIG_FILE"))}
                   for container in processes for process in container["processes"]
                   if process["service_rust"] or process["node"]],
               "recovery_binary": metadata(Path(options.recovery_bin)) if options.recovery_bin else None,
               "recovery_binary_sha256": digest(Path(options.recovery_bin)) if options.recovery_bin else None})
    extra = resolve_live_sources(extra, mounts)
    budget = estimate_materials(mounted_sources + extra, evidence)
    if not options.execute:
        pending_executables = sum(process["executable_metadata"]["bytes"] for container in processes
                                  for process in container["processes"]
                                  if process["service_rust"] or process["node"])
        reserve_bytes(budget["additional_tmpfs_budget_bytes"] + 2 * pending_executables)
        budget["uncopied_executable_bytes"] = pending_executables
    write_json(evidence / "capacity-preflight.private.json", budget)
    if not options.execute:
        return {"status": "PREFLIGHT_ONLY", "writers": 6, "databases": 3,
                "private_stage": str(stage), "services_changed": False}

    key_directory = new_directory(TMPFS / (stage.name + "-key"))
    key_file = key_directory / "key.bin"
    write_private(key_file, secrets.token_bytes(32))
    require(not key_file.is_relative_to(stage), "key must be separate from evidence")

    live_sources = [(label, source) for label, source in extra if label.startswith("live-image-")]
    live_materials = capture_sources(live_sources, new_directory(evidence / "image-materials"))
    write_json(evidence / "image-materials.private.json", live_materials)
    verify_running_generation(runner, before, processes)

    # Mutation begins only after process evidence, source ownership and private
    # output/key preflight succeed. No compensation here restarts old binaries.
    write_json(evidence / "mutation-started.private.json", {"captured_ms": now_ms()})
    updates, stopped = [], []
    for name in WRITERS:
        runner.docker_command(["update", "--restart=no", runner.writer(name)], "restart-no-" + name)
        updates.append(name)
    for name in STOP_ORDER:
        runner.docker_command(["stop", "--time", "60", runner.writer(name)], "stop-" + name, timeout=100)
        stopped.append(name)
    stop_proof = verify_frozen_writers(runner, before, processes, evidence, "writers-stopped")

    # All identifiers below are compile-time allowlists, never operator strings.
    role_sql = "\n".join(f'ALTER ROLE "{role}" NOLOGIN;' for role in ROLES)
    runner.sql("BEGIN;\n" + role_sql + "\nCOMMIT;\n", "roles-nologin", mutate=True)
    names = ",".join("'" + name + "'" for _, name in DATABASES)
    runner.sql("SELECT pg_terminate_backend(pid) FROM pg_stat_activity "
               f"WHERE datname IN ({names}) AND pid<>pg_backend_pid();\n",
               "terminate-source-backends", mutate=True)
    defaults = "\n".join(f'ALTER DATABASE "{name}" SET default_transaction_read_only=on;'
                         for _, name in DATABASES)
    runner.sql(defaults + "\n", "database-readonly-defaults", mutate=True)
    db_after = database_inventory(runner, "database-frozen")
    validate_database_inventory(db_after, frozen=True)
    effect_counts_after = fresh_effect_counts(runner, registry)
    write_json(evidence / "effect-counts-frozen.private.json", effect_counts_after)
    require(all(item["rows"] == 0 for item in effect_counts_after),
            "frozen task or external effect records require reconciliation; maintenance retained")
    captured = now_ms()
    require(captured >= 0, "invalid local freeze time")
    write_json(evidence / "barriers.private.json", {"captured_ms": captured,
               "restart_disabled": updates, "stopped_in_order": stopped,
               "databases_before": db_before, "databases_after": db_after})

    frozen_sources = mounted_sources + [(label, source) for label, source in extra
                                        if not label.startswith("live-image-")]
    materials = capture_sources(frozen_sources, new_directory(evidence / "mounted-snapshots"))
    write_json(evidence / "mounted-snapshots.private.json", materials)
    verify_frozen_writers(runner, before, processes, evidence, "writers-final")
    validate_database_inventory(database_inventory(runner, "database-final"), frozen=True)
    write_json(stage / "freeze-evidence.private.json", {
        "format": "saas-uuid-preview-freeze-evidence/1", "profile": PROFILE,
        "captured_ms": captured, "stop_results": {
            name: ("AUTO_REMOVED_VERIFIED" if name in {item["name"] for item in stop_proof["removals"]}
                   else "STOPPED_RESTART_NO") for name in WRITERS},
        "stage_permissions": metadata(stage), "key_directory_permissions": metadata(key_directory),
        "key_permissions": metadata(key_file), "key_archive_excluded": True,
        "database_barriers_verified": True, "files": file_inventory(evidence),
    })
    archive = make_archive(stage, evidence)
    if not options.recovery_bin:
        # An unsealed staging area is deliberately not an exporter assertion.
        return {"status": "FROZEN_PRIVATE_UNSEALED", "private_stage": str(stage),
                "private_key_file": str(key_file), "quiescence_created": False}

    config = stage / "seal-evidence.config.json"
    output = Path(options.cipher_output)
    write_json(config, {"profile": PROFILE, "input": str(archive), "output": str(output),
                       "key_file": str(key_file), "purpose": "saas-uuid-preview-freeze/1/" + stage.name})
    sealed_raw = runner.run([str(Path(options.recovery_bin)), "uuid-seal-evidence", str(config)],
                            "seal-evidence", timeout=1800)
    sealed = json.loads(sealed_raw)
    private(output)
    require(sealed.get("status") == "PRIVATE_EVIDENCE_SEALED"
            and sealed.get("sha256") == digest(archive)
            and sealed.get("plaintext_bytes") == archive.stat().st_size,
            "authenticated archive sealing did not match private input")
    cipher_sha256 = digest(output)
    verify_frozen_writers(runner, before, processes, evidence, "writers-after-seal")
    validate_database_inventory(database_inventory(runner, "database-after-seal"), frozen=True)
    assertion = quiescence(captured, cipher_sha256)
    write_json(stage / "quiescence.json", assertion)
    write_json(stage / "freeze-status.private.json", {
        "status": "FROZEN_SEALED", "archive_sha256": digest(archive),
        "cipher_sha256": cipher_sha256, "cipher_permissions": metadata(output),
        "seal_result": sealed, "quiescence_sha256": digest(stage / "quiescence.json"),
    })
    return {"status": "FROZEN_SEALED", "writers": 6, "databases": 3,
            "private_stage": str(stage), "private_key_file": str(key_file),
            "writer_evidence_sha256": cipher_sha256, "quiescence_created": True}


def read_private_json(path: Path) -> object:
    private(path)
    return json.loads(path.read_bytes())


def sources_from_process_records(processes: list[dict]) -> list[tuple[str, Path]]:
    result = []
    for container in processes:
        name = container["container"]
        require(name in WRITERS, "unreviewed archived process container")
        for process in container["processes"]:
            if not (process["service_rust"] or process["node"]):
                continue
            pid = process["host_pid"]
            cwd = process["cwd"]
            for key, value in process.get("path_environment", {}).items():
                if value:
                    result.append((f"{name}-{pid}-{key}", container_path(pid, value, cwd)))
            if process["node"]:
                result.append((f"{name}-{pid}-node-workspace", container_path(pid, cwd)))
                scripts = [value for value in process["argv"][1:]
                           if value.endswith((".js", ".mjs", ".cjs", ".json"))]
                for index, value in enumerate(scripts):
                    result.append((f"{name}-{pid}-node-script-{index}", container_path(pid, value, cwd)))
    return result


def resume(options: argparse.Namespace, original: Path, attempt: Path) -> dict:
    """Continue the recorded stop-stage failure, never repeat a running preflight."""
    require(not (original / "quiescence.json").exists()
            and not (original / "freeze-status.private.json").exists(),
            "original stage already has a final assertion")
    old_evidence = original / "evidence"
    private(old_evidence, directory=True)
    originals = list((old_evidence / "commands").glob("*-inspect-before.stdout"))
    require(len(originals) == 1, "one original before-inspect record required")
    before = read_private_json(originals[0])
    identities = validate_archived_writers(before)
    processes = read_private_json(old_evidence / "processes.private.json")
    archived_mounts = read_private_json(old_evidence / "mounted-materials.private.json")
    previous_preflight = read_private_json(old_evidence / "preflight.private.json")
    read_private_json(old_evidence / "mutation-started.private.json")
    registry = read_private_json(old_evidence / "uuid-references-v2.json")
    image_materials = read_private_json(old_evidence / "image-materials.private.json")
    require({item["container"] for item in processes} == set(WRITERS),
            "original service process records incomplete")
    for container in processes:
        for process in container["processes"]:
            if process["service_rust"] or process["node"]:
                pid = process["host_pid"]
                require(isinstance(pid, int) and pid > 0, "invalid archived service PID")
                executable = old_evidence / "processes" / container["container"] / str(pid) / "executable.bin"
                private(executable)
                copied = process.get("executable_copy") or {}
                require(copied.get("sha256") == digest(executable)
                        and copied.get("captured_bytes") == executable.stat().st_size,
                        "original actual executable copy failed integrity verification")
    key_directory = original.parent / (original.name + "-key")
    private(key_directory, directory=True)
    require_tmpfs(key_directory)
    key_file = key_directory / "key.bin"
    private(key_file)
    require(key_file.stat().st_size == 32, "original separate key must contain 32 bytes")
    evidence = new_directory(attempt / "evidence")
    runner = Runner(new_directory(evidence / "commands"))
    runner.writer_ids = identities
    initial_proof = verify_frozen_writers(runner, before, processes, evidence, "resume-writers-before")
    current = database_inventory(runner, "resume-database-before")
    validate_database_inventory(current, allow_fenced=True)
    old_owners = {item["name"]: item["owner"] for item in previous_preflight["databases"]["databases"]}
    require({item["name"]: item["owner"] for item in current["databases"]} == old_owners,
            "source database owner changed since original freeze")
    mounts, mounted_sources = mount_sources(before)
    def mounted_identity(items: list[dict]) -> list[dict]:
        return [{**{key: value for key, value in item.items() if key != "source_metadata"},
                 "source_identity": {key: item["source_metadata"][key] for key in ("device", "inode")}}
                for item in items]
    require(mounted_identity(mounts) == mounted_identity(archived_mounts),
            "mounted source identity changed since original freeze")
    extra = resolve_live_sources(sources_from_process_records(processes), mounts)
    image_labels = {item["label"] for item in image_materials.get("sources", [])}
    image_labels |= {item.get("label") for item in image_materials.get("exclusions", [])}
    require(all(label in image_labels for label, _ in extra if label.startswith("live-image-")),
            "unmounted image configuration was not captured before original process exited")
    sources = mounted_sources + [(label, source) for label, source in extra
                                  if not label.startswith("live-image-")]
    budget = estimate_materials(sources, old_evidence)
    write_json(evidence / "resume-preflight.private.json", {"original_stage": str(original),
               "original_inspect_sha256": digest(originals[0]), "writer_ids": identities,
               "database_before": current, "mounted_sources_revalidated": mounts, "capacity": budget,
               "original_key_reused": True, "recovery_binary_sha256": digest(Path(options.recovery_bin))})

    role_sql = "\n".join(f'ALTER ROLE "{role}" NOLOGIN;' for role in ROLES)
    runner.sql("BEGIN;\n" + role_sql + "\nCOMMIT;\n", "resume-roles-nologin", mutate=True)
    names = ",".join("'" + name + "'" for _, name in DATABASES)
    runner.sql("SELECT pg_terminate_backend(pid) FROM pg_stat_activity "
               f"WHERE datname IN ({names}) AND pid<>pg_backend_pid();\n",
               "resume-terminate-source-backends", mutate=True)
    runner.sql("\n".join(f'ALTER DATABASE "{name}" SET default_transaction_read_only=on;'
                          for _, name in DATABASES) + "\n", "resume-database-readonly", mutate=True)
    frozen = database_inventory(runner, "resume-database-frozen")
    validate_database_inventory(frozen, frozen=True)
    counts = fresh_effect_counts(runner, registry)
    write_json(evidence / "effect-counts-frozen.private.json", counts)
    require(all(item["rows"] == 0 for item in counts),
            "frozen task or external effect records require reconciliation; maintenance retained")
    captured = now_ms()
    materials = capture_sources(sources, new_directory(evidence / "mounted-snapshots"))
    write_json(evidence / "mounted-snapshots.private.json", materials)
    verify_frozen_writers(runner, before, processes, evidence, "resume-writers-final")
    validate_database_inventory(database_inventory(runner, "resume-database-final"), frozen=True)
    write_json(attempt / "freeze-evidence.private.json", {
        "format": "saas-uuid-preview-freeze-evidence/1", "profile": PROFILE,
        "resumed_original_stage": str(original), "captured_ms": captured,
        "writer_proof": initial_proof, "database_barriers_verified": True,
        "stage_permissions": metadata(attempt), "original_stage_permissions": metadata(original),
        "key_directory_permissions": metadata(key_directory), "key_permissions": metadata(key_file),
        "key_archive_excluded": True, "original_files": file_inventory(old_evidence),
        "files": file_inventory(evidence)})
    archive = make_archive(attempt, evidence, original_stage=original)
    config = attempt / "seal-evidence.config.json"
    output = Path(options.cipher_output)
    write_json(config, {"profile": PROFILE, "input": str(archive), "output": str(output),
                       "key_file": str(key_file), "purpose": "saas-uuid-preview-freeze/1/" + original.name})
    sealed = json.loads(runner.run([str(Path(options.recovery_bin)), "uuid-seal-evidence", str(config)],
                                  "resume-seal-evidence", timeout=1800))
    private(output)
    require(sealed.get("status") == "PRIVATE_EVIDENCE_SEALED" and sealed.get("sha256") == digest(archive)
            and sealed.get("plaintext_bytes") == archive.stat().st_size,
            "authenticated resume archive sealing did not match private input")
    cipher_sha256 = digest(output)
    verify_frozen_writers(runner, before, processes, evidence, "resume-writers-after-seal")
    validate_database_inventory(database_inventory(runner, "resume-database-after-seal"), frozen=True)
    write_json(original / "quiescence.json", quiescence(captured, cipher_sha256))
    write_json(original / "freeze-status.private.json", {
        "status": "FROZEN_SEALED", "resumed_at": str(attempt), "cipher_sha256": cipher_sha256,
        "cipher_permissions": metadata(output), "seal_result": sealed,
        "quiescence_sha256": digest(original / "quiescence.json")})
    return {"status": "FROZEN_SEALED", "writers": 6, "databases": 3,
            "private_stage": str(original), "private_resume_stage": str(attempt),
            "private_key_file": str(key_file), "writer_evidence_sha256": cipher_sha256,
            "quiescence_created": True}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--execute", action="store_true", help="explicitly freeze the reviewed local writers")
    parser.add_argument("--recovery-bin", help="absolute local recovery binary with uuid-seal-evidence")
    parser.add_argument("--cipher-output", help="new absolute cipher path in an existing owned 0700 directory")
    parser.add_argument("--resume-stage", help="original owned /dev/shm freeze stage after a stop-stage failure")
    options = parser.parse_args()
    stage, failure_stage = None, None
    os.umask(0o077)
    try:
        validate_options(options)
        tag = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime()) + "-" + secrets.token_hex(8)
        if options.resume_stage:
            stage = Path(options.resume_stage)
            failure_stage = new_directory(stage / ("resume-" + tag))
            result = resume(options, stage, failure_stage)
        else:
            stage = new_directory(TMPFS / ("uuid-preview-freeze-" + tag))
            failure_stage = stage
            result = run(options, stage)
        print(json.dumps(result, sort_keys=True))
        return 0
    except (Exception, KeyboardInterrupt) as error:
        # Neither subprocess output nor OS exceptions are safe for public stderr.
        if failure_stage is not None:
            try:
                write_json(failure_stage / "failure.private.json", {"error_type": type(error).__name__,
                           "error": str(error), "captured_ms": now_ms(),
                           "automatic_restart": False, "evidence_retained": True})
            except Exception:
                pass
        message = str(error) if isinstance(error, FreezeError) else "freeze failed; inspect private evidence"
        print(json.dumps({"status": "FAILED", "error": message,
                          "private_stage": str(stage) if stage else None,
                          "private_attempt": str(failure_stage) if failure_stage else None,
                          "automatic_restart": False}), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
