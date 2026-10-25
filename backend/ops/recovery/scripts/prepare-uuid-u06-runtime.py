#!/usr/bin/env python3
"""Prepare fixed U06 final/recovery materials offline; never execute a launch plan."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import stat
import sys
from urllib.parse import unquote, urlsplit

_SPEC = importlib.util.spec_from_file_location("uuid_u05_runtime_helpers", Path(__file__).with_name("prepare-uuid-u05-runtime.py"))
U05 = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(U05)
Invalid = U05.Invalid
require, fields = U05.require, U05.fields
path_checked = U05.path_checked
parsed, sha_file, digest, marker_parts, write_new = U05.parsed, U05.sha_file, U05.digest, U05.marker_parts, U05.write_new
binaries = U05.binaries
SERVICES, APPS, INPUT_KEYS, DB_KEYS = U05.SERVICES, U05.APPS, U05.INPUT_KEYS, U05.DB_KEYS
ORIGIN, PLATFORM = U05.ORIGIN, U05.PLATFORM
FORMAT = "saas-uuid-u06-runtime/1"
BASE = Path("/var/lib/saas-uuid-u06-runtime-20261024")
GENERATIONS = ("final", "recovery")
IMAGES = {
    "runtime": ("saas-ci:phase04-tools", "sha256:42e1b7fb5fea569fba2abada59030185286c9d642643cb7fa63ed7a5715b87b7"),
    "gateway": ("nginx:1.28.0-bookworm", "sha256:3b03dfd827515a07ccf4946000f0bbdd2a4fb6aca7499249fcb6b1f9726823d6"),
}
MIGRATIONS = {
    "platform": (list(range(1, 13)), "80706cce23384465caf0e66a2a3056b4b003f6ee1ea47ea1d8095e985efbb2c3"),
    "stores": (list(range(1, 4)), "0ee5122f007f1b19e75e5fa0f8974f7da3944522473619bfdfdaf91b62b3b0e7"),
    "work_orders": (list(range(1, 5)), "ceb4a5cd8aa674a201bd98f4e2831b89e20b40c91b38bbca4a35eac9c39c479a"),
}
FRONTEND_DESCRIPTOR_SHA256 = "eb14516b7c2894b6b793e77a1cb90a49121a884d10fbf13f8f29ce99f95f210a"
FRONTEND_ROOTS = {
    app: "/var/lib/saas-uuid-u05-materials-20261024/host-u05-r2/static" if app == "platform" else "/var/lib/saas-uuid-u05-runtime-20261024/static/" + app
    for app in APPS
}
POLICY_SHA256 = "a4742919d165d32a13fb439b3e4f213b13c260f76f47806261d54ad600622563"
SOURCE_IDS_SHA256 = "62e3d050ce92fa06526496b1a4fe5c0a8f02809ae6840621bdc250f1857a44ea"
SOURCE_CLIENT_HASHES_SHA256 = "5fc73d7ea1d6b1953cf574f8a66a4a60bcf7d73b357cb94230e1c771367f3cdc"
SOURCE_JWT_SHA256 = "1dd908d57bb4ca67f8a7cf07419587d8f262f16c8892af9307b3303b4d483737"


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()


def hashed(value):
    return hashlib.sha256(value).hexdigest()


def private_bytes(value, limit=4 * 1024 * 1024):
    path = path_checked(value)
    private_directory(path.parent)
    return U05.private_bytes(value, limit)


def private_json(value, limit=4 * 1024 * 1024):
    return parsed(private_bytes(value, limit))


def target_names(generation):
    require(generation in GENERATIONS, "U06 generation invalid")
    return {
        service: {
            "database": f"qiankun_{short}_uuid7_{generation}_20261024",
            "owner_role": f"qiankun_{short}_uuid7_{generation}_owner_20261024",
            "runtime_role": f"qiankun_{short}_uuid7_u06_{generation}_runtime_20261024",
        }
        for service, short in (("platform", "platform"), ("stores", "stores"), ("work_orders", "orders"))
    }


def notification_role(generation):
    return f"qiankun_notification_uuid7_u06_{generation}_runtime_20261024"


def checked_json(path, expected, limit=4 * 1024 * 1024):
    raw = private_bytes(path, limit)
    require(hashed(raw) == digest(expected), "private input digest mismatch")
    return parsed(raw), raw


def receipts(value, generation):
    fields(value, ("format", "generation", "targets", "notification_runtime_role", "epoch_file", "epoch_sha256"))
    require(value["format"] == "saas-uuid-u06-imported-receipts/1" and value["generation"] == generation, "U06 receipt generation invalid")
    expected = target_names(generation)
    fields(value["targets"], expected)
    markers = set()
    for service, names in expected.items():
        target = value["targets"][service]
        fields(target, (*names, "schema_sha256", "import_marker", "migration_versions", "migration_history_sha256"))
        require(all(target[key] == name for key, name in names.items()), "U06 receipt database or role invalid")
        digest(target["schema_sha256"])
        versions, history = MIGRATIONS[service]
        require(target["migration_versions"] == versions and all(type(v) is int for v in target["migration_versions"]) and target["migration_history_sha256"] == history, "frozen migration history required")
        markers.add(marker_parts(target["import_marker"]))
    require(len(markers) == 1, "one imported batch required")
    require(value["notification_runtime_role"] == notification_role(generation), "U06 notification role invalid")
    path_checked(value["epoch_file"])
    digest(value["epoch_sha256"])
    return value["targets"]


def epoch_parts(value):
    fields(value, ("format", "generation", "jwt_secret_sha256", "previous_jwt_secret_sha256", "confidential_clients", "secrets_fresh"))
    require(value["format"] == "saas-uuid-u06-credential-epoch/1" and value["generation"] in GENERATIONS and value["secrets_fresh"] is True, "credential epoch invalid")
    current_jwt = digest(value["jwt_secret_sha256"])
    previous_jwt = digest(value["previous_jwt_secret_sha256"])
    clients = value["confidential_clients"]
    require(isinstance(clients, list) and len(clients) == 12, "twelve confidential clients required")
    previous, current = {}, {}
    for item in clients:
        fields(item, ("client_id", "previous_secret_hash", "new_secret_hash"))
        identifier = item["client_id"]
        require(isinstance(identifier, str) and identifier and len(identifier) <= 256 and not any(ord(c) < 32 or ord(c) == 127 for c in identifier) and identifier not in previous, "opaque client ID set invalid")
        previous[identifier] = digest(item["previous_secret_hash"])
        current[identifier] = digest(item["new_secret_hash"])
    require(hashed(canonical(sorted(previous))) == SOURCE_IDS_SHA256, "source client ID set changed")
    require(len(set(previous.values())) == len(previous) and len(set(current.values())) == len(current), "independent client secrets required")
    require(set(previous.values()).isdisjoint(current.values()) and current_jwt not in {previous_jwt, *previous.values(), *current.values()} and previous_jwt not in current.values(), "credential epoch reuses secrets")
    return previous, current, previous_jwt, current_jwt


def validate_epoch(value, generation, predecessor=None):
    previous, current, old_jwt, new_jwt = epoch_parts(value)
    require(value["generation"] == generation, "credential epoch generation invalid")
    if generation == "final":
        require(hashed(canonical(previous)) == SOURCE_CLIENT_HASHES_SHA256 and old_jwt == SOURCE_JWT_SHA256, "final source credential commitment invalid")
    else:
        require(predecessor is not None, "prepared final epoch required")
        original, final, original_jwt, final_jwt = validate_epoch(predecessor, "final")
        require(previous == final and old_jwt == final_jwt, "recovery predecessor epoch mismatch")
        require(set(current.values()).isdisjoint(original.values()) and original_jwt not in current.values() and new_jwt not in {*original.values(), original_jwt}, "recovery reuses original secrets")
    return previous, current, old_jwt, new_jwt


def predecessor_epoch():
    root = path_checked(str(BASE / "final"))
    private_directory(root)
    prepared = private_json(str(root / "prepared.json"))
    fields(prepared, ("format", "status", "generation", "services", "frontends", "credential_epoch_sha256", "receipt_sha256", "source_sessions_loaded", "state_rebuilt"))
    require(prepared["format"] == FORMAT and prepared["status"] == "PREPARED_NOT_STARTED" and prepared["generation"] == "final" and prepared["services"] == 6 and prepared["frontends"] == 8 and prepared["source_sessions_loaded"] is False and prepared["state_rebuilt"] is False, "prepared final generation required")
    receipt, _ = checked_json(str(root / "imported-receipts.json"), prepared["receipt_sha256"])
    receipts(receipt, "final")
    require(receipt["epoch_sha256"] == prepared["credential_epoch_sha256"], "final receipt epoch commitment mismatch")
    epoch, _ = checked_json(str(root / "credential-epoch.json"), prepared["credential_epoch_sha256"])
    return epoch


def validate_database(value, database, role):
    try:
        url = urlsplit(value)
        password = unquote(url.password or "")
        valid = url.scheme in ("postgres", "postgresql") and url.hostname == "127.0.0.1" and url.port == 5432 and unquote(url.path) == "/" + database and not url.query and not url.fragment and unquote(url.username or "") == role
    except (ValueError, TypeError):
        raise Invalid("runtime database URL invalid") from None
    require(valid and re.fullmatch(r"[A-Za-z0-9_-]{32,256}", password), "runtime database target or secret invalid")
    return hashed(password.encode())


def policy(manifest):
    value, _ = checked_json(manifest["policy_file"], manifest["policy_sha256"])
    fields(value, ("applications", "managementScopes"))
    require(hashed(canonical(value)) == POLICY_SHA256, "original reviewed policy required")
    return json.dumps(value, ensure_ascii=True, separators=(",", ":"))


def reviewed_frontends(value):
    artifacts = U05.frontends(value)
    require(all(artifacts[app]["root"] == FRONTEND_ROOTS[app] for app in APPS), "reviewed frontend source roots required")
    descriptor = {"format": value["format"], "origin": value["origin"], "sdk_version": value["sdk_version"], "artifacts": [{"app": a["app"], "version": a["version"], "files": sorted(a["files"], key=lambda f: f["path"])} for a in sorted(artifacts.values(), key=lambda a: a["app"])]}
    require(hashed(canonical(descriptor)) == FRONTEND_DESCRIPTOR_SHA256, "accepted r2 frontend bytes required")
    return artifacts


def fixed_environment(service, target=None):
    if service == "platform":
        result = {"APP_ENV": "development", "APPLICATION_ENVIRONMENT": "development", "AUTO_CREATE_DATABASE": "false", "BIND_ADDR": "127.0.0.1:13001", "PUBLIC_APP_URL": ORIGIN, "MICRO_GATEWAY_ORIGIN": ORIGIN, "MICRO_ALLOWED_ORIGINS": ORIGIN, "CORS_ALLOWED_ORIGINS": ORIGIN, "STATIC_DIR": "", "MICRO_APPS_DIR": "", "OAUTH_AUTO_PURGE": "false", "APP_ENV_FILE": "/run/u06/empty.env"}
        prefix = "PLATFORM"
    elif service in ("stores", "work_orders"):
        prefix = "STORES" if service == "stores" else "WORK_ORDERS"
        result = {"NODE_ENV": "development", prefix + "_HOST": "127.0.0.1", prefix + "_PORT": "13010" if service == "stores" else "13011", prefix + "_ALLOWED_ORIGINS": ORIGIN, "PLATFORM_API_ORIGIN": PLATFORM, "PLATFORM_AUTHORIZATION_MODE": "combined"}
    elif service == "notification":
        return {"NOTIFICATION_ENV": "development", "NOTIFICATION_HOST": "127.0.0.1", "NOTIFICATION_PORT": "13002", "NOTIFICATION_SOCKET_PATH": "/socket.io/", "NOTIFICATION_ALLOWED_ORIGINS": ORIGIN, "PLATFORM_API_BASE_URL": PLATFORM}
    elif service == "examples_oauth":
        return {"OAUTH_PUBLIC_ORIGIN": ORIGIN, "OAUTH_PLATFORM_ORIGIN": ORIGIN, "OAUTH_BIND_ADDR": "127.0.0.1:13007", "OAUTH_ALLOW_INSECURE_LOOPBACK": "true", "OAUTH_PASSWORD_ENABLED": "false", "OAUTH_SCOPE": "profile"}
    else:
        return {"CHAT_BIND_ADDR": "127.0.0.1:13008", "CHAT_ALLOWED_ORIGINS": ORIGIN, "CHAT_ALLOW_SANDBOX_ORIGIN": "false"}
    result.update({prefix + "_DATABASE_INIT_MODE": "imported", prefix + "_EXPECTED_DATABASE": target["database"], prefix + "_IMPORT_MARKER": target["import_marker"], prefix + "_EXPECTED_SCHEMA_SHA256": target["schema_sha256"]})
    return result


def service_environments(manifest, receipt, data_policy, epoch):
    _, client_hashes, _, jwt_hash = epoch_parts(epoch)
    result, secret_hashes = {}, set()
    epoch_hashes = set(client_hashes.values()) | {jwt_hash}
    old_hashes = {item["previous_secret_hash"] for item in epoch["confidential_clients"]} | {epoch["previous_jwt_secret_sha256"]}
    for service in SERVICES:
        source = manifest["services"][service]
        fields(source, ("binary", "env_file"))
        env = private_json(source["env_file"], 1024 * 1024)
        optional = {"OAUTH_DEVICE_CLIENT_SECRET"} if service == "examples_oauth" else set()
        require(isinstance(env, dict) and INPUT_KEYS[service] <= set(env) <= INPUT_KEYS[service] | optional, "private runtime env field set invalid")
        require(all(isinstance(v, str) and v and not any(ord(c) < 32 or ord(c) == 127 for c in v) for v in env.values()), "private runtime env value invalid")
        for key, value in env.items():
            if key.endswith(("SECRET", "TOKEN")):
                secret_hash = hashed(value.encode())
                require(re.fullmatch(r"[A-Za-z0-9_-]{32,256}", value) and secret_hash not in secret_hashes | old_hashes, "fresh independent URL-safe secret required")
                secret_hashes.add(secret_hash)
                if key == "JWT_SECRET":
                    require(secret_hash == jwt_hash, "JWT epoch mismatch")
                elif key.endswith("CLIENT_SECRET"):
                    identifier = env[key[:-6] + "ID"]
                    require(client_hashes.get(identifier) == secret_hash, "client secret epoch mismatch")
                else:
                    require(secret_hash not in epoch_hashes, "metrics token must be independent")
        if service in DB_KEYS:
            target = receipt["targets"][service if service != "notification" else "platform"]
            role = target["runtime_role"] if service != "notification" else receipt["notification_runtime_role"]
            password_hash = validate_database(env[DB_KEYS[service]], target["database"], role)
            require(password_hash not in secret_hashes | old_hashes | epoch_hashes, "independent database password required")
            secret_hashes.add(password_hash)
        env.update(fixed_environment(service, receipt["targets"].get(service)))
        if service == "platform":
            env["DATA_GRANT_POLICY"] = data_policy
        result[service] = env
    return result


# No executable or argv can be supplied to the runner. -I is also fixed in the
# Docker plan so ambient PYTHONPATH/PYTHONSTARTUP cannot affect Python startup.
RUNNER = '''#!/usr/bin/env python3
import hashlib, os, re, stat, sys
from pathlib import Path
try:
    if len(sys.argv)!=2 or not re.fullmatch('[0-9a-f]{64}',sys.argv[1]): raise ValueError()
    binary=Path('/run/u06/service')
    info=binary.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_uid!=0 or info.st_nlink!=1 or stat.S_IMODE(info.st_mode)&0o022: raise ValueError()
    with binary.open('rb') as handle:
        if hashlib.file_digest(handle,'sha256').hexdigest()!=sys.argv[1]: raise ValueError()
    config=Path('/run/u06/service.env')
    info=config.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_uid!=0 or info.st_nlink!=1 or stat.S_IMODE(info.st_mode)!=0o600: raise ValueError()
    env={}
    for line in config.read_text().splitlines():
        key,sep,value=line.partition('=')
        if not sep or not re.fullmatch('[A-Z][A-Z0-9_]*',key) or key in env: raise ValueError()
        env[key]=value
    env.update({'PATH':'/usr/bin:/bin','LANG':'C.UTF-8','HOME':'/run/u06/state'})
    state=Path('/run/u06/state')
    info=state.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid!=0 or stat.S_IMODE(info.st_mode)!=0o700: raise ValueError()
    for fd,name in ((1,'stdout.log'),(2,'stderr.log')):
        handle=os.open(state/name,os.O_WRONLY|os.O_CREAT|os.O_APPEND|os.O_NOFOLLOW,0o600)
        info=os.fstat(handle)
        if not stat.S_ISREG(info.st_mode) or info.st_uid!=0 or info.st_nlink!=1 or stat.S_IMODE(info.st_mode)!=0o600: raise ValueError()
        os.dup2(handle,fd);os.close(handle)
    os.chdir(state)
    os.execve(binary,[str(binary)],env)
except Exception:
    print('U06 private runtime launch validation failed',file=sys.stderr)
    sys.exit(1)
'''


def private_directory(path):
    info = path.stat()
    require(stat.S_ISDIR(info.st_mode) and info.st_uid == 0 and stat.S_IMODE(info.st_mode) == 0o700, "root owned private runtime directory required")


def launch_plan(root, manifest):
    generation = manifest["generation"]
    shared = ["docker", "run", "--detach", "--init", "--restart", "no", "--network", "host", "--read-only", "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--log-driver", "none", "--tmpfs", "/tmp:rw,noexec,nosuid,nodev,size=64m"]
    plans = []
    def identity(service):
        return ["--name", f"qiankun-uuid-u06-{generation}-{service.replace('_', '-')}-20261024", "--label", "saas.uuid-stage=U06", "--label", "saas.uuid-generation=" + generation, "--label", "saas.uuid-runtime=20261024", "--label", "saas.uuid-service=" + service]
    for service in SERVICES:
        command = shared + (["--interactive"] if service == "examples_chat" else []) + identity(service)
        for source, target, readonly in ((root / "bin" / service, "/run/u06/service", True), (root / "env" / (service + ".env"), "/run/u06/service.env", True), (root / "runner.py", "/run/u06/runner.py", True), (root / "empty.env", "/run/u06/empty.env", True), (root / "state" / service, "/run/u06/state", False)):
            command += ["--mount", f"type=bind,source={source},target={target}" + (",readonly" if readonly else "")]
        command += ["--workdir", "/run/u06/state", "--entrypoint", "/usr/bin/python3", manifest["images"]["runtime"]["id"], "-I", "/run/u06/runner.py", manifest["services"][service]["binary"]["sha256"]]
        plans.append({"service": service, "argv": command, "stdin_open": service == "examples_chat"})
    gateway = shared + ["--user", "101:101"] + identity("gateway") + ["--mount", f"type=bind,source={root / 'static'},target=/srv/frontends,readonly", "--mount", f"type=bind,source={root / 'nginx.conf'},target=/etc/nginx/nginx.conf,readonly", "--entrypoint", "/usr/sbin/nginx", manifest["images"]["gateway"]["id"], "-g", "daemon off;"]
    plans.append({"service": "gateway", "argv": gateway, "stdin_open": False})
    return {"format": "saas-uuid-u06-launch-plan/1", "profile": "uuid-local-preview", "generation": generation, "origin": ORIGIN, "automatic_start": False, "requires_no_other_generation_running": True, "commands": plans}


def execute(manifest_path, prepare=False):
    require(sys.platform == "linux" and os.geteuid() == 0, "U06 preparation requires Linux root")
    manifest = private_json(manifest_path)
    fields(manifest, ("format", "profile", "generation", "output_dir", "images", "receipt_file", "receipt_sha256", "policy_file", "policy_sha256", "frontend_manifest", "frontend_manifest_sha256", "services"))
    generation = manifest["generation"]
    require(manifest["format"] == FORMAT and manifest["profile"] == "uuid-local-preview" and generation in GENERATIONS, "U06 manifest profile or generation invalid")
    root = path_checked(manifest["output_dir"])
    require(root == BASE / generation and not root.exists(), "new fixed U06 generation directory required")
    path_checked(str(BASE))
    if BASE.exists():
        private_directory(BASE)
    else:
        require(BASE.parent.is_dir(), "U06 base parent required")
    fields(manifest["images"], ("runtime", "gateway"))
    for key, (name, image_id) in IMAGES.items():
        image = manifest["images"][key]
        fields(image, ("name", "id"))
        require(image["name"] == name and image["id"] == image_id, "pinned runtime image invalid")
    fields(manifest["services"], SERVICES)
    receipt, receipt_raw = checked_json(manifest["receipt_file"], manifest["receipt_sha256"])
    receipts(receipt, generation)
    epoch, epoch_raw = checked_json(receipt["epoch_file"], receipt["epoch_sha256"])
    validate_epoch(epoch, generation, predecessor_epoch() if generation == "recovery" else None)
    data_policy = policy(manifest)
    environments = service_environments(manifest, receipt, data_policy, epoch)
    binary_paths = binaries(manifest)
    frontend, _ = checked_json(manifest["frontend_manifest"], manifest["frontend_manifest_sha256"], 16 * 1024 * 1024)
    artifacts = reviewed_frontends(frontend)
    if prepare:
        if not BASE.exists():
            BASE.mkdir(mode=0o700)
            BASE.chmod(0o700)
        private_directory(BASE)
        root.mkdir(mode=0o700)
        root.chmod(0o700)
        for name in ("bin", "env", "state"):
            (root / name).mkdir(mode=0o700)
            (root / name).chmod(0o700)
        (root / "static").mkdir(mode=0o755)
        (root / "static").chmod(0o755)
        write_new(root / "runner.py", RUNNER)
        write_new(root / "empty.env", b"")
        write_new(root / "credential-epoch.json", epoch_raw)
        write_new(root / "imported-receipts.json", receipt_raw)
        for service in SERVICES:
            (root / "state" / service).mkdir(mode=0o700)
            (root / "state" / service).chmod(0o700)
            destination = root / "bin" / service
            with binary_paths[service].open("rb") as source, destination.open("xb") as target:
                shutil.copyfileobj(source, target, 1024 * 1024)
            destination.chmod(0o700)
            require(sha_file(destination) == manifest["services"][service]["binary"]["sha256"], "copied binary digest mismatch")
            write_new(root / "env" / (service + ".env"), "".join(key + "=" + value + "\n" for key, value in sorted(environments[service].items())))
        for app, artifact in artifacts.items():
            for item in artifact["files"]:
                destination = root / "static" / app / item["path"]
                destination.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
                directory = destination.parent
                while directory != root / "static":
                    directory.chmod(0o755)
                    directory = directory.parent
                with (Path(artifact["root"]) / item["path"]).open("rb") as source, destination.open("xb") as target:
                    shutil.copyfileobj(source, target, 1024 * 1024)
                destination.chmod(0o644)
                require(sha_file(destination) == item["sha256"], "copied frontend digest mismatch")
        write_new(root / "nginx.conf", U05.nginx_configuration(artifacts).replace(r'return 200 "u05\n";', r'return 200 "u06\n";'), 0o644)
        write_new(root / "launch-plan.json", json.dumps(launch_plan(root, manifest), indent=2) + "\n")
        write_new(root / "input-manifest.json", json.dumps(manifest, indent=2) + "\n")
        write_new(root / "frontends.json", json.dumps(frontend, indent=2) + "\n")
        prepared = {"format": FORMAT, "status": "PREPARED_NOT_STARTED", "generation": generation, "services": len(SERVICES), "frontends": len(APPS), "credential_epoch_sha256": hashed(epoch_raw), "receipt_sha256": hashed(receipt_raw), "source_sessions_loaded": False, "state_rebuilt": False}
        write_new(root / "prepared.json", json.dumps(prepared, indent=2) + "\n")
    return {"status": "PREPARED_NOT_STARTED" if prepare else "CHECKED_NOT_STARTED", "generation": generation, "services": len(SERVICES), "frontends": len(APPS), "database_connections": 0, "services_started": 0}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest")
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument("--check", action="store_true")
    action.add_argument("--prepare", action="store_true")
    args = parser.parse_args()
    try:
        print(json.dumps(execute(args.manifest, args.prepare)))
    except (Invalid, OSError, ValueError, TypeError, KeyError, OverflowError):
        print("U06 runtime preparation rejected; inspect private inputs against the fixed contract", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
