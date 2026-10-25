#!/usr/bin/env python3
"""Validate and materialize the fixed local U05 rehearsal; never launch services."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import sys
from urllib.parse import unquote, urlsplit
import uuid

FORMAT = "saas-uuid-u05-runtime/1"
OUTPUT = Path("/var/lib/saas-uuid-u05-runtime-20261024")
ORIGIN = "http://127.0.0.1:13000"
PLATFORM = "http://127.0.0.1:13001"
SERVICES = ("platform", "stores", "work_orders", "notification", "examples_oauth", "examples_chat")
APPS = ("platform", "stores", "work_orders", "vue-workshop", "legacy-jquery", "report-center", "oauth-playground", "websocket-chat")
TARGETS = {
    "platform": ("qiankun_platform_uuid7_u04_20261024", "a11665c8c4a4282bf406a54cb61b82105fc9ea2c077b1ff717c9947eed47e3af"),
    "stores": ("qiankun_stores_uuid7_u04_20261024", "2d989b86b44b7f597aef31bfadf5b3eae3b697364e2fd5030cb32fc676cda550"),
    "work_orders": ("qiankun_orders_uuid7_u04_20261024", "b8a502cb0e3bcfdd7ed4c00fdfbbf21af1f527feddb5b61bebc4f13859732a3e"),
}
FROZEN = {
    "platform": ("saas-uuid-u05-platform", "45049fa89b9272920448e49a5ac3af9e78d59c741dc6b65d68cfe9d33f8e03f0"),
    "stores": ("saas-uuid-u05-stores", "87aa8b97132a9a3d73afa7b7d2a1b1e5b341cbe0891d81a5dd9b2c89413ca119"),
    "work_orders": ("saas-uuid-u05-orders", "998eedd23231103d572a8d88345c1992edea7fea77cc221b19a5742b825e01cc"),
    "notification": ("saas-uuid-u05-notification", "2e84eafa51408112be8de6d940732832b48ba4ca5ae7d7962fd6dd08f1aee6ea"),
    "examples_oauth": ("saas-uuid-u05-oauth", "fb36627dc4441b3ed4c5acbd60b7e53cec66fcb884aeb21d16c273a08525c02f"),
    "examples_chat": ("saas-uuid-u05-chat", "0f5378497cd1b1371a8a74e73a3713137cff1069d77db8d74e4b17a3c4452874"),
}
DB_KEYS = {"platform": "DATABASE_URL", "stores": "STORES_DATABASE_URL", "work_orders": "WORK_ORDERS_DATABASE_URL", "notification": "DATABASE_URL"}
INPUT_KEYS = {
    "platform": {"DATABASE_URL", "JWT_SECRET", "METRICS_TOKEN"},
    "stores": {"STORES_DATABASE_URL", "PLATFORM_CLIENT_ID", "PLATFORM_CLIENT_SECRET", "STORES_METRICS_TOKEN"},
    "work_orders": {"WORK_ORDERS_DATABASE_URL", "PLATFORM_CLIENT_ID", "PLATFORM_CLIENT_SECRET", "WORK_ORDERS_METRICS_TOKEN"},
    "notification": {"DATABASE_URL", "METRICS_TOKEN"},
    "examples_oauth": {"OAUTH_AUTH_CLIENT_ID", "OAUTH_AUTH_CLIENT_SECRET", "OAUTH_DEVICE_CLIENT_ID", "OAUTH_CC_CLIENT_ID", "OAUTH_CC_CLIENT_SECRET"},
    "examples_chat": set(),
}


class Invalid(Exception):
    """Only static diagnostic text is allowed to reach stdout/stderr."""


def require(condition, message):
    if not condition:
        raise Invalid(message)


def fields(value, names):
    require(isinstance(value, dict) and set(value) == set(names), "configuration field set invalid")


def unique(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate JSON key")
        result[key] = value
    return result


def parsed(raw):
    try:
        return json.loads(raw, object_pairs_hook=unique, parse_constant=lambda _: (_ for _ in ()).throw(Invalid("invalid JSON number")))
    except (ValueError, UnicodeError) as error:
        raise Invalid("invalid JSON input") from error


def path_checked(value):
    require(isinstance(value, str) and value.startswith("/") and "\\" not in value and "\x00" not in value, "normalized absolute Linux path required")
    require(all(part not in ("", ".", "..") for part in value.split("/")[1:]), "normalized absolute Linux path required")
    path = Path(value)
    for component in (path, *path.parents):
        require(not component.is_symlink(), "symbolic links are forbidden")
    return path


def private_bytes(value, limit=4 * 1024 * 1024):
    path = path_checked(value)
    info = path.stat()
    require(stat.S_ISREG(info.st_mode) and info.st_uid == os.geteuid() and stat.S_IMODE(info.st_mode) == 0o600 and info.st_nlink == 1, "private input ownership or mode invalid")
    require(info.st_size <= limit, "private input too large")
    with path.open("rb") as stream:
        raw = stream.read(limit + 1)
    require(len(raw) <= limit, "private input too large")
    return raw


def private_json(value, limit=4 * 1024 * 1024):
    return parsed(private_bytes(value, limit))


def sha_file(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def digest(value):
    require(isinstance(value, str) and re.fullmatch("[0-9a-f]{64}", value), "SHA256 invalid")
    return value


def marker_parts(value):
    require(isinstance(value, str), "import marker invalid")
    match = re.fullmatch(r"saas-uuid-import/1/([0-9a-f-]{36})/([0-9a-f]{64})", value)
    require(match is not None, "import marker invalid")
    batch = uuid.UUID(match[1])
    require(str(batch) == match[1] and batch.version == 7 and batch.variant == uuid.RFC_4122, "import marker batch invalid")
    return match[1], match[2]


def role_valid(role):
    return isinstance(role, str) and bool(re.fullmatch(r"qiankun_[a-z0-9_]*_u05_[a-z0-9_]*20261024", role)) and "owner" not in role


def validate_database(value, database, role=None):
    try:
        url = urlsplit(value)
        valid = url.scheme in ("postgres", "postgresql") and url.hostname == "127.0.0.1" and url.port == 5432 and unquote(url.path) == "/" + database and not url.query and not url.fragment and bool(url.password)
        username = unquote(url.username or "")
    except (ValueError, TypeError):
        raise Invalid("runtime database URL invalid") from None
    require(valid and role_valid(username) and (role is None or username == role), "runtime database target or role invalid")
    return username


def receipts(value):
    fields(value, ("format", "targets"))
    require(value["format"] == "saas-uuid-u05-imported-receipts/1", "receipt format invalid")
    fields(value["targets"], TARGETS)
    markers = set()
    roles = set()
    for service, (database, schema) in TARGETS.items():
        target = value["targets"][service]
        fields(target, ("database", "import_marker", "schema_sha256", "runtime_role"))
        require(target["database"] == database and target["schema_sha256"] == schema, "receipt target generation invalid")
        markers.add(marker_parts(target["import_marker"]))
        require(role_valid(target["runtime_role"]), "fresh runtime role required")
        roles.add(target["runtime_role"])
    require(len(markers) == 1 and len(roles) == 3, "receipt batch or role set invalid")
    return value["targets"]


def policy(manifest, targets):
    raw = private_bytes(manifest["policy_file"])
    value = parsed(raw)
    fields(value, ("applications", "managementScopes"))
    require(isinstance(value["applications"], list) and isinstance(value["managementScopes"], list), "policy collection invalid")
    # The authenticated config-rekey owner performs the complete closed DTO
    # validation. This handoff additionally binds its published exact output.
    summary = private_json(manifest["policy_summary_file"])
    batch, converted = marker_parts(targets["platform"]["import_marker"])
    require(summary.get("format") == "saas-uuid-config-rekey/1" and summary.get("status") == "CONFIGURATION_REKEY_COMPLETE" and summary.get("batch_id") == batch and summary.get("converted_manifest_sha256") == converted and summary.get("fresh_login_required") is True and summary.get("old_signed_credentials_reusable") is False, "configuration rekey handoff invalid")
    outputs = [item for item in summary.get("outputs", []) if item.get("owner") == "data-grant-policy"]
    require(len(outputs) == 1, "one policy owner output required")
    item = outputs[0]
    require(item.get("file") == Path(manifest["policy_file"]).name == "data-grant-policy.json" and item.get("target_sha256") == hashlib.sha256(raw).hexdigest() and item.get("target_bytes") == len(raw) and item.get("reusable") is True, "policy output digest mismatch")
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def fixed_environment(service, target=None):
    if service == "platform":
        result = {"APP_ENV": "development", "APPLICATION_ENVIRONMENT": "development", "AUTO_CREATE_DATABASE": "false", "BIND_ADDR": "127.0.0.1:13001", "PUBLIC_APP_URL": ORIGIN, "MICRO_GATEWAY_ORIGIN": ORIGIN, "MICRO_ALLOWED_ORIGINS": ORIGIN, "CORS_ALLOWED_ORIGINS": ORIGIN, "STATIC_DIR": "", "MICRO_APPS_DIR": "", "OAUTH_AUTO_PURGE": "false", "APP_ENV_FILE": "/run/u05/empty.env"}
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


def service_environments(manifest, targets, data_policy):
    result = {}
    secrets = set()
    roles = set()
    for service in SERVICES:
        source = manifest["services"][service]
        fields(source, ("binary", "env_file"))
        env = private_json(source["env_file"], 1024 * 1024)
        optional = {"OAUTH_DEVICE_CLIENT_SECRET"} if service == "examples_oauth" else set()
        require(isinstance(env, dict) and INPUT_KEYS[service] <= set(env) <= INPUT_KEYS[service] | optional, "private runtime env field set invalid")
        require(all(isinstance(value, str) and value and not any(ord(char) < 32 or ord(char) == 127 for char in value) for value in env.values()), "private runtime env value invalid")
        for key, value in env.items():
            if key.endswith(("SECRET", "TOKEN")):
                require(re.fullmatch(r"[A-Za-z0-9_-]{32,256}", value) and value not in secrets, "fresh independent URL-safe secret required")
                secrets.add(value)
        if service in DB_KEYS:
            target = targets[service if service != "notification" else "platform"]
            role = validate_database(env[DB_KEYS[service]], target["database"], target["runtime_role"] if service != "notification" else None)
            require(role not in roles and (service != "notification" or "notification" in role), "distinct runtime database roles required")
            roles.add(role)
        env.update(fixed_environment(service, targets.get(service)))
        if service == "platform":
            env["DATA_GRANT_POLICY"] = data_policy
        result[service] = env
    return result


def binaries(manifest):
    result = {}
    for service in SERVICES:
        binary = manifest["services"][service]["binary"]
        fields(binary, ("path", "sha256"))
        name, frozen_hash = FROZEN[service]
        require(binary["path"] == "/usr/local/libexec/" + name, "protected runtime binary path required")
        expected = digest(binary["sha256"])
        require(frozen_hash is None or expected == frozen_hash, "frozen runtime binary generation invalid")
        path = path_checked(binary["path"])
        info = path.stat()
        require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1 and info.st_uid == 0 and stat.S_IMODE(info.st_mode) & 0o022 == 0, "protected runtime binary ownership invalid")
        require(sha_file(path) == expected, "runtime binary digest mismatch")
        result[service] = path
    return result


def frontends(value):
    fields(value, ("format", "origin", "sdk_version", "artifacts"))
    require(value["format"] == "saas-uuid-u05-frontends/1" and value["origin"] == ORIGIN and value["sdk_version"] == "3.0.0", "reviewed SDK3 frontend manifest required")
    require(isinstance(value["artifacts"], list) and len(value["artifacts"]) == len(APPS), "frontend artifact count invalid")
    result = {}
    for artifact in value["artifacts"]:
        fields(artifact, ("app", "root", "version", "files"))
        app = artifact["app"]
        require(app in APPS and app not in result and isinstance(artifact["version"], str) and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,63}", artifact["version"]) and ".." not in artifact["version"], "frontend application or version invalid")
        root = path_checked(artifact["root"])
        require(root.is_dir(), "frontend directory missing")
        require(isinstance(artifact["files"], list) and 0 < len(artifact["files"]) <= 20000, "frontend file manifest invalid")
        listed = set()
        for item in artifact["files"]:
            fields(item, ("path", "bytes", "sha256"))
            relative = item["path"]
            require(isinstance(relative, str) and all(re.fullmatch(r"[A-Za-z0-9_@+.,()-]+", p) and p not in (".", "..") and not p.startswith(".") for p in relative.split("/")), "frontend relative path invalid")
            require(relative not in listed and not PurePosixPath(relative).is_absolute(), "duplicate frontend path")
            listed.add(relative)
            path = path_checked(str(root / relative))
            require(path.is_file() and type(item["bytes"]) is int and path.stat().st_size == item["bytes"] and sha_file(path) == digest(item["sha256"]), "frontend file digest mismatch")
        actual = set()
        for parent, directories, files in os.walk(root, followlinks=False):
            require(not any((Path(parent) / name).is_symlink() for name in directories + files), "frontend symbolic links forbidden")
            actual.update((Path(parent) / name).relative_to(root).as_posix() for name in files)
        require(actual == listed and "index.html" in listed and (app != "oauth-playground" or "callback.html" in listed), "frontend complete tree mismatch")
        if app in ("stores", "work_orders"):
            info = parsed((root / "build-info.json").read_bytes())
            require(info.get("sdkVersion") == "3.0.0" and info.get("version") == artifact["version"], "frontend build metadata generation invalid")
        result[app] = artifact
    return result


def nginx_configuration(artifacts):
    # Route semantics are from backend/nginx/policy.mjs and frontend/vite.config.ts.
    static = []
    aliases = {"stores": "stores", "work_orders": "work-orders", "vue-workshop": "vue", "legacy-jquery": "jquery", "report-center": "report-center", "websocket-chat": "websocket-chat", "oauth-playground": "oauth-playground"}
    for app, alias in aliases.items():
        public_app = "work-orders" if app == "work_orders" else app
        for prefix in (f"/sub/{alias}/", f"/sub-apps/{public_app}/versions/{artifacts[app]['version']}/"):
            static.append(f'''        location = {prefix[:-1]} {{ return 308 {prefix}$is_args$args; }}
        location ^~ {prefix} {{
            rewrite ^{re.escape(prefix)}(.*)$ /{app}/$1 break;
            root /srv/frontends;
            index index.html;
            try_files $uri $uri/index.html =404;
            limit_except GET {{ deny all; }}
            add_header Cache-Control no-store always;
            add_header X-Content-Type-Options nosniff always;
        }}''')
    private = []
    for prefix in ("/api/internal", "/internal", "/metrics", "/api/metrics", "/stores-api/internal", "/stores-api/metrics", "/work-orders-api/internal", "/work-orders-api/metrics", "/oauth-playground-api/internal", "/oauth-playground-api/metrics", "/socket.io/metrics"):
        private += [f"        location = {prefix} {{ return 404; }}", f"        location ^~ {prefix}/ {{ return 404; }}"]
    proxies = []
    for prefix, port in (("api", 13001), ("oauth", 13001), ("micro-proxy", 13001), ("micro-assets", 13001), ("micro-releases", 13001), ("releases", 13001), ("stores-api", 13010), ("work-orders-api", 13011)):
        for match in (f"= /{prefix}", f"/{prefix}/"):
            proxies.append(f"        location {match} {{ proxy_pass http://127.0.0.1:{port}; }}")
    return '''worker_processes 1;
pid /tmp/nginx.pid;
error_log /dev/stderr crit;
events { worker_connections 512; }
http {
    include /etc/nginx/mime.types;
    types { application/javascript mjs cjs; }
    default_type application/octet-stream;
    access_log off;
    server_tokens off;
    client_body_temp_path /tmp/client_body;
    proxy_temp_path /tmp/proxy;
    fastcgi_temp_path /tmp/fastcgi;
    uwsgi_temp_path /tmp/uwsgi;
    scgi_temp_path /tmp/scgi;
    map $http_upgrade $connection_upgrade { default upgrade; '' close; }
    map $http_cookie $oauth_cookie {
        default "";
        "~(?:^|;[[:space:]]*)oauth_playground=(?<oauth_value>[^;]*)" "oauth_playground=$oauth_value";
    }
    map $http_cookie $oauth_host_cookie {
        default "";
        "~(?:^|;[[:space:]]*)__Host-oauth_playground=(?<oauth_host_value>[^;]*)" "__Host-oauth_playground=$oauth_host_value";
    }
    map "$oauth_cookie|$oauth_host_cookie" $oauth_cookies {
        default "$oauth_cookie; $oauth_host_cookie";
        "|" ""; "~^[|]" $oauth_host_cookie; "~[|]$" $oauth_cookie;
    }
    map $http_accept $accept_html { default 0; ~*text/html 1; }
    server {
        listen 127.0.0.1:13000;
        server_name 127.0.0.1;
        root /srv/frontends/platform;
        index index.html;
        autoindex off;
        disable_symlinks on;
        client_max_body_size 32m;
        proxy_http_version 1.1;
        proxy_set_header Host $http_host;
        proxy_set_header X-Forwarded-Host $http_host;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header Origin $http_origin;
        proxy_set_header Connection "";
        proxy_connect_timeout 5s;
        proxy_read_timeout 65s;
        add_header Cache-Control no-store always;
        add_header X-Content-Type-Options nosniff always;
        add_header Referrer-Policy no-referrer always;
        add_header Content-Security-Policy "default-src 'none'; script-src 'self' 'unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self' http://127.0.0.1:13000 ws://127.0.0.1:13000 http://127.0.0.1:13008 ws://127.0.0.1:13008; frame-src 'self'; base-uri 'self'; object-src 'none'; frame-ancestors 'none'" always;
        location = /healthz { return 200 "u05\\n"; }
''' + "\n".join(private + proxies) + '''
        location = /socket.io { proxy_pass http://127.0.0.1:13002; }
        location /socket.io/ {
            proxy_set_header Upgrade $http_upgrade;
            proxy_set_header Connection $connection_upgrade;
            proxy_buffering off;
            proxy_read_timeout 75s;
            proxy_pass http://127.0.0.1:13002;
        }
        location = /oauth-playground-api { return 404; }
        location /oauth-playground-api/ {
            rewrite ^/oauth-playground-api/(.*)$ /api/$1 break;
            proxy_set_header Host $http_host;
            proxy_set_header X-Forwarded-Proto $scheme;
            proxy_set_header Origin $http_origin;
            proxy_set_header Authorization "";
            proxy_set_header Cookie $oauth_cookies;
            proxy_set_header Upgrade "";
            proxy_set_header Connection "";
            proxy_pass http://127.0.0.1:13007;
        }
        location = /callback {
            root /srv/frontends/oauth-playground;
            try_files /callback.html =404;
            limit_except GET { deny all; }
        }
''' + "\n".join(static) + '''
        location /sub/ { return 404; }
        location /sub-apps/ { return 404; }
        location ~ (?:^|/)\\. { return 404; }
        location ~* \\.[A-Za-z0-9]+$ { try_files $uri =404; }
        location / { try_files $uri $uri/ @navigation; }
        location @navigation {
            if ($accept_html = 0) { return 404; }
            try_files /index.html =404;
        }
    }
}
'''


RUNNER = '''#!/usr/bin/env python3
import hashlib, json, os, stat, sys
from pathlib import Path
try:
    binary=Path('/run/u05/service')
    with binary.open('rb') as handle:
        if hashlib.file_digest(handle,'sha256').hexdigest()!=sys.argv[1]: raise ValueError()
    env={}
    for line in Path('/run/u05/service.env').read_text().splitlines():
        key,sep,value=line.partition('=')
        if not sep or key in env: raise ValueError()
        env[key]=value
    env.update({'PATH':'/usr/bin:/bin','LANG':'C.UTF-8','HOME':'/run/u05/state'})
    for fd,name in ((1,'stdout.log'),(2,'stderr.log')):
        handle=os.open('/run/u05/state/'+name,os.O_WRONLY|os.O_CREAT|os.O_APPEND|os.O_NOFOLLOW,0o600)
        info=os.fstat(handle)
        if not stat.S_ISREG(info.st_mode) or stat.S_IMODE(info.st_mode)!=0o600: raise ValueError()
        os.dup2(handle,fd);os.close(handle)
    os.chdir('/run/u05/state')
    os.execve(binary,[str(binary)],env)
except Exception:
    print('U05 private runtime launch validation failed',file=sys.stderr)
    sys.exit(1)
'''


def write_new(path, value, mode=0o600):
    raw = value.encode() if isinstance(value, str) else value
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode)
    os.fchmod(fd, mode)
    with os.fdopen(fd, "wb") as handle:
        handle.write(raw)
        handle.flush()
        os.fsync(handle.fileno())
    require(path.read_bytes() == raw, "prepared output read-back failed")


def launch_plan(root, manifest):
    shared = ["docker", "run", "--detach", "--init", "--restart", "no", "--network", "host", "--read-only", "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--log-driver", "none", "--tmpfs", "/tmp:rw,noexec,nosuid,nodev,size=64m"]
    plans = []
    for service in SERVICES:
        command = shared + (["--interactive"] if service == "examples_chat" else [])
        command += ["--name", f"qiankun-uuid-u05-{service.replace('_','-')}-20261024", "--label", "saas.uuid-stage=U05", "--label", "saas.uuid-runtime=20261024", "--label", "saas.uuid-service=" + service]
        for source, target, readonly in ((root / "bin" / service, "/run/u05/service", True), (root / "env" / (service + ".env"), "/run/u05/service.env", True), (root / "runner.py", "/run/u05/runner.py", True), (root / "empty.env", "/run/u05/empty.env", True), (root / "state" / service, "/run/u05/state", False)):
            command += ["--mount", f"type=bind,source={source},target={target}" + (",readonly" if readonly else "")]
        command += ["--workdir", "/run/u05/state", "--entrypoint", "/usr/bin/python3", manifest["images"]["runtime"]["id"], "/run/u05/runner.py", manifest["services"][service]["binary"]["sha256"]]
        plans.append({"service": service, "argv": command, "stdin_open": service == "examples_chat"})
    gateway = shared + ["--user", "101:101", "--name", "qiankun-uuid-u05-gateway-20261024", "--label", "saas.uuid-stage=U05", "--label", "saas.uuid-runtime=20261024", "--label", "saas.uuid-service=gateway", "--mount", f"type=bind,source={root / 'static'},target=/srv/frontends,readonly", "--mount", f"type=bind,source={root / 'nginx.conf'},target=/etc/nginx/nginx.conf,readonly", "--entrypoint", "/usr/sbin/nginx", manifest["images"]["gateway"]["id"], "-g", "daemon off;"]
    plans.append({"service": "gateway", "argv": gateway, "stdin_open": False})
    return {"format": "saas-uuid-u05-launch-plan/1", "profile": "uuid-local-preview", "origin": ORIGIN, "automatic_start": False, "commands": plans}


def execute(manifest_path, prepare=False):
    require(sys.platform == "linux", "U05 preparation requires Linux private files")
    manifest = private_json(manifest_path)
    fields(manifest, ("format", "profile", "output_dir", "images", "receipt_file", "policy_file", "policy_summary_file", "frontend_manifest", "services"))
    require(manifest["format"] == FORMAT and manifest["profile"] == "uuid-local-preview", "runtime manifest profile invalid")
    root = path_checked(manifest["output_dir"])
    require(root == OUTPUT and not root.exists() and root.parent.is_dir(), "new fixed U05 runtime directory required")
    fields(manifest["images"], ("runtime", "gateway"))
    for key, name in (("runtime", "saas-ci:phase04-tools"), ("gateway", "nginx:1.28.0-bookworm")):
        image = manifest["images"][key]
        fields(image, ("name", "id"))
        require(image["name"] == name and isinstance(image["id"], str) and re.fullmatch("sha256:[0-9a-f]{64}", image["id"]), "pinned runtime image invalid")
    fields(manifest["services"], SERVICES)
    targets = receipts(private_json(manifest["receipt_file"]))
    data_policy = policy(manifest, targets)
    environments = service_environments(manifest, targets, data_policy)
    binary_paths = binaries(manifest)
    artifacts = frontends(private_json(manifest["frontend_manifest"], 16 * 1024 * 1024))
    if prepare:
        root.mkdir(mode=0o700)
        for name in ("bin", "env", "state"):
            (root / name).mkdir(mode=0o700)
        (root / "static").mkdir(mode=0o755)
        (root / "static").chmod(0o755)
        write_new(root / "runner.py", RUNNER)
        write_new(root / "empty.env", b"")
        for service in SERVICES:
            (root / "state" / service).mkdir(mode=0o700)
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
        write_new(root / "nginx.conf", nginx_configuration(artifacts), 0o644)
        write_new(root / "launch-plan.json", json.dumps(launch_plan(root, manifest), indent=2) + "\n")
        write_new(root / "input-manifest.json", json.dumps(manifest, indent=2) + "\n")
        write_new(root / "frontends.json", json.dumps({"format": "saas-uuid-u05-frontends/1", "origin": ORIGIN, "sdk_version": "3.0.0", "artifacts": list(artifacts.values())}, indent=2) + "\n")
        write_new(root / "prepared.json", json.dumps({"format": FORMAT, "status": "PREPARED_NOT_STARTED", "services": len(SERVICES), "frontends": len(APPS), "state_rebuilt": False, "source_sessions_loaded": False}, indent=2) + "\n")
    return {"status": "PREPARED_NOT_STARTED" if prepare else "CHECKED_NOT_STARTED", "services": len(SERVICES), "frontends": len(APPS), "database_connections": 0, "services_started": 0}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest")
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument("--check", action="store_true")
    action.add_argument("--prepare", action="store_true")
    args = parser.parse_args()
    try:
        print(json.dumps(execute(args.manifest, args.prepare)))
    except (Invalid, OSError, ValueError, TypeError, KeyError):
        print("U05 runtime preparation rejected; inspect the private inputs against the fixed contract", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
