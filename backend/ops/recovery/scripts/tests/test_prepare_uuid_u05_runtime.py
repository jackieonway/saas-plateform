"""Offline contract tests; no business database or existing runtime is accessed."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("u05_runtime", Path(__file__).parents[1] / "prepare-uuid-u05-runtime.py")
RUNTIME = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUNTIME)
MARKER = "saas-uuid-import/1/019a1163-0000-7000-8000-000000000001/" + "a" * 64


@unittest.skipUnless(os.name == "posix", "requires real Linux modes and ownership")
class RuntimeContract(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.output = self.root / "new-runtime"
        self.counter = 0
        self.targets = {
            service: {"database": database, "schema_sha256": schema, "import_marker": MARKER, "runtime_role": f"qiankun_{service}_uuid7_u05_runtime_20261024"}
            for service, (database, schema) in RUNTIME.TARGETS.items()
        }
        self.manifest = {
            "format": RUNTIME.FORMAT, "profile": "uuid-local-preview", "output_dir": str(self.output),
            "images": {"runtime": {"name": "saas-ci:phase04-tools", "id": "sha256:" + "a" * 64}, "gateway": {"name": "nginx:1.28.0-bookworm", "id": "sha256:" + "b" * 64}},
            "receipt_file": self.private("receipts.json", {"format": "saas-uuid-u05-imported-receipts/1", "targets": self.targets}),
            "services": {},
        }
        self.binary = self.root / "synthetic-binary"
        self.binary.write_bytes(b"isolated executable fixture bytes")
        self.binary.chmod(0o700)
        binary_hash = hashlib.sha256(self.binary.read_bytes()).hexdigest()
        for service in RUNTIME.SERVICES:
            env = {}
            for key in RUNTIME.INPUT_KEYS[service]:
                if key == RUNTIME.DB_KEYS.get(service):
                    owner = service if service != "notification" else "platform"
                    role = self.targets[owner]["runtime_role"] if service != "notification" else "qiankun_notification_uuid7_u05_runtime_20261024"
                    env[key] = f"postgresql://{role}:synthetic-password@127.0.0.1:5432/{self.targets[owner]['database']}"
                elif key.endswith(("SECRET", "TOKEN")):
                    self.counter += 1
                    env[key] = f"synthetic_isolated_secret_{self.counter:040d}"
                else:
                    env[key] = "opaque-oauth-client-" + service
            self.manifest["services"][service] = {"env_file": self.private(service + ".json", env), "binary": {"path": "/usr/local/libexec/" + RUNTIME.FROZEN[service][0], "sha256": binary_hash}}
        self.manifest["policy_file"] = self.private("data-grant-policy.json", {"applications": [], "managementScopes": []})
        policy = Path(self.manifest["policy_file"]).read_bytes()
        self.manifest["policy_summary_file"] = self.private("summary.json", {
            "format": "saas-uuid-config-rekey/1", "status": "CONFIGURATION_REKEY_COMPLETE", "batch_id": MARKER.split("/")[2], "converted_manifest_sha256": "a" * 64,
            "fresh_login_required": True, "old_signed_credentials_reusable": False,
            "outputs": [{"owner": "data-grant-policy", "file": "data-grant-policy.json", "target_sha256": hashlib.sha256(policy).hexdigest(), "target_bytes": len(policy), "reusable": True}],
        })
        self.artifacts = []
        for app in RUNTIME.APPS:
            directory = self.root / ("dist-" + app)
            directory.mkdir()
            contents = {"index.html": "<!doctype html><p>isolated fixture</p>"}
            if app == "oauth-playground":
                contents["callback.html"] = "<!doctype html><p>callback</p>"
            if app in ("stores", "work_orders"):
                contents["build-info.json"] = json.dumps({"sdkVersion": "3.0.0", "version": "1.0.1-uuid-u05"})
            files = []
            for name, text in contents.items():
                raw = text.encode()
                (directory / name).write_bytes(raw)
                files.append({"path": name, "bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()})
            self.artifacts.append({"app": app, "root": str(directory), "version": "1.0.1-uuid-u05", "files": files})
        self.frontend = {"format": "saas-uuid-u05-frontends/1", "origin": RUNTIME.ORIGIN, "sdk_version": "3.0.0", "artifacts": self.artifacts}
        self.manifest["frontend_manifest"] = self.private("frontends.json", self.frontend)

    def private(self, name, value):
        path = self.root / name
        path.write_text(json.dumps(value), encoding="utf-8")
        path.chmod(0o600)
        return str(path)

    def run_manifest(self, prepare=False):
        path = self.private("manifest.json", self.manifest)
        with patch.object(RUNTIME, "OUTPUT", self.output), patch.object(RUNTIME, "binaries", return_value={key: self.binary for key in RUNTIME.SERVICES}):
            return RUNTIME.execute(path, prepare)

    def test_complete_check_is_read_only_then_prepare_creates_isolated_materials(self):
        self.assertEqual(self.run_manifest()["status"], "CHECKED_NOT_STARTED")
        self.assertFalse(self.output.exists())
        with patch.dict(os.environ, {"JWT_SECRET": "ambient-canary", "PGPASSWORD": "ambient-canary"}):
            self.assertEqual(self.run_manifest(True)["status"], "PREPARED_NOT_STARTED")
        self.assertEqual(stat.S_IMODE(self.output.stat().st_mode), 0o700)
        env = (self.output / "env/platform.env").read_text()
        self.assertIn("PLATFORM_DATABASE_INIT_MODE=imported\n", env)
        self.assertIn("AUTO_CREATE_DATABASE=false\n", env)
        self.assertNotIn("ambient-canary", env)
        self.assertNotIn("BOOTSTRAP_ADMIN", env)
        for service in RUNTIME.SERVICES:
            self.assertEqual(stat.S_IMODE((self.output / "env" / (service + ".env")).stat().st_mode), 0o600)
            self.assertEqual(list((self.output / "state" / service).iterdir()), [])
        plan = json.loads((self.output / "launch-plan.json").read_text())
        self.assertFalse(plan["automatic_start"])
        self.assertEqual(len(plan["commands"]), 7)
        chat = next(item for item in plan["commands"] if item["service"] == "examples_chat")
        self.assertIn("--interactive", chat["argv"])
        self.assertTrue(chat["stdin_open"])
        for command in plan["commands"]:
            self.assertNotIn("--privileged", command["argv"])
            self.assertNotIn("--env-file", command["argv"])
            self.assertNotIn("synthetic_isolated_secret", json.dumps(command))
        self.assertIn("os.execve(binary,[str(binary)],env)", (self.output / "runner.py").read_text())
        with self.assertRaises(RUNTIME.Invalid):
            self.run_manifest(True)

    def test_public_static_modes_remain_readable_with_private_umask(self):
        previous = os.umask(0o077)
        try:
            self.run_manifest(True)
        finally:
            os.umask(previous)
        for item in (self.output / "static", self.output / "static/platform"):
            self.assertEqual(stat.S_IMODE(item.stat().st_mode), 0o755)
        self.assertEqual(stat.S_IMODE((self.output / "static/platform/index.html").stat().st_mode), 0o644)
        self.assertEqual(stat.S_IMODE((self.output / "nginx.conf").stat().st_mode), 0o644)

    def test_private_modes_symlinks_duplicate_keys_and_unknown_fields_fail(self):
        path = Path(self.manifest["services"]["platform"]["env_file"])
        path.chmod(0o644)
        with self.assertRaises(RUNTIME.Invalid):
            self.run_manifest()
        path.chmod(0o600)
        link = self.root / "linked.json"
        link.symlink_to(path)
        with self.assertRaises(RUNTIME.Invalid):
            RUNTIME.private_bytes(str(link))
        with self.assertRaises(RUNTIME.Invalid):
            RUNTIME.parsed('{"x":1,"x":2}')
        with self.assertRaises(RUNTIME.Invalid):
            RUNTIME.parsed('{"x":NaN}')
        self.manifest["source_preview_state"] = "forbidden"
        with self.assertRaises(RUNTIME.Invalid):
            self.run_manifest()

    def test_old_target_wrong_epoch_owner_and_hidden_connection_options_fail(self):
        original = copy.deepcopy(self.targets)
        for field, value in (("database", "qiankun_platform_preview_20261024"), ("schema_sha256", "0" * 64), ("runtime_role", "qiankun_platform_uuid7_u04_owner_20261024"), ("import_marker", MARKER.replace("-7000-", "-4000-"))):
            changed = copy.deepcopy(original)
            changed["platform"][field] = value
            with self.assertRaises((RUNTIME.Invalid, ValueError)):
                RUNTIME.receipts({"format": "saas-uuid-u05-imported-receipts/1", "targets": changed})
        role = self.targets["platform"]["runtime_role"]
        database = self.targets["platform"]["database"]
        for url in (f"postgres://{role}:x@10.0.0.1:5432/{database}", f"postgres://postgres:x@127.0.0.1:5432/{database}", f"postgres://{role}:x@127.0.0.1:5432/{database}?options=-crole=postgres"):
            with self.assertRaises(RUNTIME.Invalid):
                RUNTIME.validate_database(url, database, role)

    def test_old_state_seed_env_policy_digest_and_stale_frontend_are_rejected(self):
        path = self.manifest["services"]["platform"]["env_file"]
        env = json.loads(Path(path).read_text())
        env["BOOTSTRAP_ADMIN_PASSWORD"] = "never-load-old-state"
        self.private("platform.json", env)
        with self.assertRaises(RUNTIME.Invalid):
            self.run_manifest()
        del env["BOOTSTRAP_ADMIN_PASSWORD"]
        self.private("platform.json", env)
        Path(self.manifest["policy_file"]).write_text('{"applications":[],"managementScopes":[],"unknown":true}')
        with self.assertRaises(RUNTIME.Invalid):
            self.run_manifest()
        item = self.frontend["artifacts"][0]
        (Path(item["root"]) / "extra.js").write_text("stale or unreviewed bytes")
        with self.assertRaises(RUNTIME.Invalid):
            RUNTIME.frontends(self.frontend)

    def test_nginx_routes_keep_api_prefixes_private_routes_and_oauth_isolation(self):
        config = RUNTIME.nginx_configuration({item["app"]: item for item in self.artifacts})
        self.assertIn("listen 127.0.0.1:13000", config)
        self.assertIn("location /stores-api/ { proxy_pass http://127.0.0.1:13010; }", config)
        self.assertIn("location /work-orders-api/ { proxy_pass http://127.0.0.1:13011; }", config)
        self.assertIn("rewrite ^/oauth-playground-api/(.*)$ /api/$1 break;", config)
        self.assertIn('proxy_set_header Authorization "";', config)
        self.assertIn("proxy_set_header Cookie $oauth_cookies;", config)
        self.assertIn("location = /callback", config)
        self.assertIn("/sub/oauth-playground/", config)
        self.assertIn("try_files $uri $uri/index.html =404", config)
        self.assertIn("location ^~ /stores-api/internal/ { return 404; }", config)
        self.assertIn("proxy_set_header Upgrade $http_upgrade;", config)
        self.assertNotIn("proxy_pass http://127.0.0.1:13008", config)
        self.assertIn('"PLATFORM_AUTHORIZATION_MODE": "combined"', repr(RUNTIME.fixed_environment("stores", self.targets["stores"])).replace("'", '"'))


if __name__ == "__main__":
    unittest.main()
