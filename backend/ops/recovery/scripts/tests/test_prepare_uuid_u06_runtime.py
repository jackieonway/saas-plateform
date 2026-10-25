"""Offline U06 contracts. Synthetic files/processes only; no DB, Docker or ports."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("u06_runtime", Path(__file__).parents[1] / "prepare-uuid-u06-runtime.py")
R = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(R)
MARKER = "saas-uuid-import/1/019a1163-0000-7000-8000-000000000001/" + "a" * 64


def sha(value):
    return hashlib.sha256(value).hexdigest()


def secret(index):
    return "isolated_synthetic_secret_" + str(index).zfill(40)


def descriptor(frontend):
    return {"format": frontend["format"], "origin": frontend["origin"], "sdk_version": frontend["sdk_version"], "artifacts": [{"app": a["app"], "version": a["version"], "files": sorted(a["files"], key=lambda f: f["path"])} for a in sorted(frontend["artifacts"], key=lambda a: a["app"])]}


@unittest.skipUnless(sys.platform == "linux" and os.geteuid() == 0, "requires Linux root ownership and mode semantics")
class RuntimeContract(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.base = self.root / "generations"
        self.binary = self.root / "synthetic-binary"
        self.binary.write_bytes(b"isolated non-executable fixture")
        self.binary.chmod(0o700)
        self.source_clients = {"opaque-client-" + str(i): sha(secret(i).encode()) for i in range(12)}
        self.source_jwt = sha(secret(99).encode())
        self.policy = {"applications": [], "managementScopes": []}
        self.frontend = {"format": "saas-uuid-u05-frontends/1", "origin": R.ORIGIN, "sdk_version": "3.0.0", "artifacts": []}
        roots = {}
        for app in R.APPS:
            directory = self.root / ("dist-" + app)
            directory.mkdir()
            roots[app] = str(directory)
            version = "host-u05-r2" if app == "platform" else "2.0.1-uuid-u05" if app == "work_orders" else "1.0.1-uuid-u05"
            contents = {"index.html": b"<p>isolated fixture</p>", "assets/app.js": b"window.fixture=true;"}
            if app == "oauth-playground":
                contents["callback.html"] = b"<p>isolated callback</p>"
            if app in ("stores", "work_orders"):
                contents["build-info.json"] = json.dumps({"sdkVersion": "3.0.0", "version": version}).encode()
            files = []
            for name, raw in contents.items():
                path = directory / name
                path.parent.mkdir(exist_ok=True)
                path.write_bytes(raw)
                files.append({"path": name, "bytes": len(raw), "sha256": sha(raw)})
            self.frontend["artifacts"].append({"app": app, "root": str(directory), "version": version, "files": files})
        patches = {
            "BASE": self.base,
            "SOURCE_IDS_SHA256": sha(R.canonical(sorted(self.source_clients))),
            "SOURCE_CLIENT_HASHES_SHA256": sha(R.canonical(self.source_clients)),
            "SOURCE_JWT_SHA256": self.source_jwt,
            "POLICY_SHA256": sha(R.canonical(self.policy)),
            "FRONTEND_ROOTS": roots,
            "FRONTEND_DESCRIPTOR_SHA256": sha(R.canonical(descriptor(self.frontend))),
        }
        for key, value in patches.items():
            active = patch.object(R, key, value)
            active.start()
            self.addCleanup(active.stop)
        active = patch.object(R, "binaries", return_value={service: self.binary for service in R.SERVICES})
        active.start()
        self.addCleanup(active.stop)
        self.manifest, self.receipt, self.epoch = self.make_generation("final")

    def private(self, name, value):
        path = self.root / name
        path.write_text(json.dumps(value), encoding="utf-8")
        path.chmod(0o600)
        return str(path)

    def make_generation(self, generation, predecessor=None):
        offset = 100 if generation == "final" else 200
        previous = self.source_clients if predecessor is None else {item["client_id"]: item["new_secret_hash"] for item in predecessor["confidential_clients"]}
        epoch = {
            "format": "saas-uuid-u06-credential-epoch/1", "generation": generation,
            "jwt_secret_sha256": sha(secret(offset + 99).encode()),
            "previous_jwt_secret_sha256": self.source_jwt if predecessor is None else predecessor["jwt_secret_sha256"],
            "confidential_clients": [{"client_id": identifier, "previous_secret_hash": previous[identifier], "new_secret_hash": sha(secret(offset + i).encode())} for i, identifier in enumerate(self.source_clients)],
            "secrets_fresh": True,
        }
        epoch_path = self.private(generation + "-epoch.json", epoch)
        receipt = {
            "format": "saas-uuid-u06-imported-receipts/1", "generation": generation,
            "targets": {service: {**names, "schema_sha256": str(i + 1) * 64, "import_marker": MARKER, "migration_versions": R.MIGRATIONS[service][0], "migration_history_sha256": R.MIGRATIONS[service][1]} for i, (service, names) in enumerate(R.target_names(generation).items())},
            "notification_runtime_role": R.notification_role(generation), "epoch_file": epoch_path, "epoch_sha256": sha(Path(epoch_path).read_bytes()),
        }
        receipt_path = self.private(generation + "-receipts.json", receipt)
        policy_path = self.private(generation + "-policy.json", self.policy)
        frontend_path = self.private(generation + "-frontends.json", self.frontend)
        envs = {
            "platform": {"JWT_SECRET": secret(offset + 99), "METRICS_TOKEN": secret(offset + 300)},
            "stores": {"PLATFORM_CLIENT_ID": "opaque-client-0", "PLATFORM_CLIENT_SECRET": secret(offset), "STORES_METRICS_TOKEN": secret(offset + 301)},
            "work_orders": {"PLATFORM_CLIENT_ID": "opaque-client-1", "PLATFORM_CLIENT_SECRET": secret(offset + 1), "WORK_ORDERS_METRICS_TOKEN": secret(offset + 302)},
            "notification": {"METRICS_TOKEN": secret(offset + 303)},
            "examples_oauth": {"OAUTH_AUTH_CLIENT_ID": "opaque-client-2", "OAUTH_AUTH_CLIENT_SECRET": secret(offset + 2), "OAUTH_DEVICE_CLIENT_ID": "preserved-public-client", "OAUTH_CC_CLIENT_ID": "opaque-client-3", "OAUTH_CC_CLIENT_SECRET": secret(offset + 3)},
            "examples_chat": {},
        }
        for i, (service, key) in enumerate(R.DB_KEYS.items()):
            target = receipt["targets"][service if service != "notification" else "platform"]
            role = target["runtime_role"] if service != "notification" else receipt["notification_runtime_role"]
            envs[service][key] = f"postgresql://{role}:{secret(offset + 400 + i)}@127.0.0.1:5432/{target['database']}"
        manifest = {
            "format": R.FORMAT, "profile": "uuid-local-preview", "generation": generation, "output_dir": str(self.base / generation),
            "images": {key: {"name": value[0], "id": value[1]} for key, value in R.IMAGES.items()},
            "receipt_file": receipt_path, "receipt_sha256": sha(Path(receipt_path).read_bytes()),
            "policy_file": policy_path, "policy_sha256": sha(Path(policy_path).read_bytes()),
            "frontend_manifest": frontend_path, "frontend_manifest_sha256": sha(Path(frontend_path).read_bytes()),
            "services": {service: {"binary": {"path": "/usr/local/libexec/" + R.U05.FROZEN[service][0], "sha256": sha(self.binary.read_bytes())}, "env_file": self.private(generation + "-" + service + ".json", envs[service])} for service in R.SERVICES},
        }
        return manifest, receipt, epoch

    def run_manifest(self, prepare=False, manifest=None):
        return R.execute(self.private("manifest.json", manifest or self.manifest), prepare)

    def rewrite_receipt(self):
        path = self.manifest["receipt_file"]
        self.private(Path(path).name, self.receipt)
        self.manifest["receipt_sha256"] = sha(Path(path).read_bytes())

    def test_check_read_only_then_both_generations_prepare_with_fresh_empty_state(self):
        self.assertEqual(self.run_manifest()["status"], "CHECKED_NOT_STARTED")
        self.assertFalse(self.base.exists())
        previous_umask = os.umask(0o077)
        try:
            self.assertEqual(self.run_manifest(True)["services_started"], 0)
            recovery, _, _ = self.make_generation("recovery", self.epoch)
            self.assertEqual(self.run_manifest(False, recovery)["database_connections"], 0)
            self.run_manifest(True, recovery)
        finally:
            os.umask(previous_umask)
        self.assertEqual(stat.S_IMODE(self.base.stat().st_mode), 0o700)
        for generation in R.GENERATIONS:
            root = self.base / generation
            self.assertEqual(stat.S_IMODE(root.stat().st_mode), 0o700)
            for service in R.SERVICES:
                self.assertEqual(list((root / "state" / service).iterdir()), [])
                self.assertEqual(stat.S_IMODE((root / "env" / (service + ".env")).stat().st_mode), 0o600)
            self.assertEqual(stat.S_IMODE((root / "static" / "platform" / "assets").stat().st_mode), 0o755)
            self.assertEqual(stat.S_IMODE((root / "static" / "platform" / "index.html").stat().st_mode), 0o644)
            self.assertEqual(stat.S_IMODE((root / "nginx.conf").stat().st_mode), 0o644)
            self.assertEqual(len(json.loads((root / "launch-plan.json").read_text())["commands"]), 7)
        with self.assertRaises(R.Invalid):
            self.run_manifest(True)

    def test_receipts_bind_exact_generation_roles_migrations_and_same_batch(self):
        changes = [("database", "qiankun_platform_uuid7_u04_20261024"), ("owner_role", "qiankun_platform_uuid7_recovery_owner_20261024"), ("runtime_role", "qiankun_platform_uuid7_u06_recovery_runtime_20261024"), ("migration_versions", list(range(1, 14))), ("migration_history_sha256", "0" * 64), ("import_marker", MARKER.replace("000001/", "000002/")), ("schema_sha256", "bad")]
        for field, value in changes:
            receipt = copy.deepcopy(self.receipt)
            receipt["targets"]["platform"][field] = value
            with self.subTest(field=field), self.assertRaises(R.Invalid):
                R.receipts(receipt, "final")
        receipt = copy.deepcopy(self.receipt)
        receipt["notification_runtime_role"] = receipt["targets"]["platform"]["runtime_role"]
        with self.assertRaises(R.Invalid):
            R.receipts(receipt, "final")
        with self.assertRaises(R.Invalid):
            R.receipts(self.receipt, "recovery")

    def test_epoch_rejects_id_changes_secret_reuse_and_wrong_source_commitments(self):
        for mutate in (
            lambda e: e["confidential_clients"][0].update(client_id="changed-opaque-id"),
            lambda e: e["confidential_clients"][0].update(new_secret_hash=e["confidential_clients"][1]["new_secret_hash"]),
            lambda e: e["confidential_clients"][0].update(new_secret_hash=e["confidential_clients"][1]["previous_secret_hash"]),
            lambda e: e["confidential_clients"][0].update(previous_secret_hash="e" * 64),
            lambda e: e.update(jwt_secret_sha256=e["previous_jwt_secret_sha256"]),
            lambda e: e.update(previous_jwt_secret_sha256="f" * 64),
            lambda e: e.update(secrets_fresh=False),
        ):
            epoch = copy.deepcopy(self.epoch)
            mutate(epoch)
            with self.assertRaises(R.Invalid):
                R.validate_epoch(epoch, "final")

    def test_recovery_requires_prepared_final_and_rejects_original_secret_reuse(self):
        recovery, _, epoch = self.make_generation("recovery", self.epoch)
        with self.assertRaises(OSError):
            self.run_manifest(False, recovery)
        self.run_manifest(True)
        self.assertEqual(self.run_manifest(False, recovery)["generation"], "recovery")
        for mutate in (
            lambda e: e.update(previous_jwt_secret_sha256=self.source_jwt),
            lambda e: e["confidential_clients"][0].update(previous_secret_hash=self.source_clients["opaque-client-0"]),
            lambda e: e["confidential_clients"][0].update(new_secret_hash=self.source_clients["opaque-client-0"]),
            lambda e: e.update(jwt_secret_sha256=self.source_jwt),
        ):
            wrong = copy.deepcopy(epoch)
            mutate(wrong)
            with self.assertRaises(R.Invalid):
                R.validate_epoch(wrong, "recovery", self.epoch)
        (self.base / "final" / "credential-epoch.json").write_text("{}")
        with self.assertRaises(R.Invalid):
            self.run_manifest(False, recovery)

    def test_recovery_rechecks_final_receipt_hash_and_epoch_commitment(self):
        recovery, _, _ = self.make_generation("recovery", self.epoch)
        self.run_manifest(True)
        root = self.base / "final"
        receipt_path = root / "imported-receipts.json"
        receipt = json.loads(receipt_path.read_text())
        receipt["epoch_sha256"] = "0" * 64
        receipt_path.write_text(json.dumps(receipt))
        with self.assertRaises(R.Invalid):
            self.run_manifest(False, recovery)
        prepared_path = root / "prepared.json"
        prepared = json.loads(prepared_path.read_text())
        prepared["receipt_sha256"] = sha(receipt_path.read_bytes())
        prepared_path.write_text(json.dumps(prepared))
        with self.assertRaises(R.Invalid):
            self.run_manifest(False, recovery)

    def test_env_secret_receipt_binding_db_identity_and_closed_fields(self):
        path = Path(self.manifest["services"]["stores"]["env_file"])
        original = json.loads(path.read_text())
        for key, value in (("PLATFORM_CLIENT_SECRET", secret(999)), ("PLATFORM_CLIENT_ID", "other-client"), ("BOOTSTRAP_ADMIN_PASSWORD", secret(999)), ("STORES_DATABASE_URL", original["STORES_DATABASE_URL"] + "?options=-crole=postgres"), ("STORES_METRICS_TOKEN", secret(100))):
            env = {**original, key: value}
            self.private(path.name, env)
            with self.subTest(key=key), self.assertRaises(R.Invalid):
                self.run_manifest()
        self.private(path.name, original)
        self.receipt["targets"]["platform"]["schema_sha256"] = "f" * 64
        # Even a valid fresh schema digest must be bound by the reviewed raw receipt.
        self.private(Path(self.manifest["receipt_file"]).name, self.receipt)
        with self.assertRaises(R.Invalid):
            self.run_manifest()

    def test_policy_and_frontend_raw_hashes_are_bound(self):
        for key in ("policy_sha256", "frontend_manifest_sha256", "receipt_sha256"):
            manifest = copy.deepcopy(self.manifest)
            manifest[key] = "0" * 64
            with self.subTest(key=key), self.assertRaises(R.Invalid):
                self.run_manifest(manifest=manifest)
        changed = {"applications": [{"temporary": "notification-grant"}], "managementScopes": []}
        self.private(Path(self.manifest["policy_file"]).name, changed)
        self.manifest["policy_sha256"] = sha(Path(self.manifest["policy_file"]).read_bytes())
        with self.assertRaises(R.Invalid):
            self.run_manifest()

    def test_frontend_rejects_r1_changed_sdk_versions_missing_extra_and_symlink(self):
        for change in ("version", "root", "bytes", "missing", "extra", "symlink"):
            with self.subTest(change=change):
                frontend = copy.deepcopy(self.frontend)
                artifact = frontend["artifacts"][0]
                path = Path(artifact["root"]) / "assets" / "app.js"
                original = path.read_bytes()
                extra = Path(artifact["root"]) / "extra.js"
                if change == "version":
                    artifact["version"] = "host-u05-r1"
                elif change == "root":
                    other = self.root / "unreviewed-root"
                    shutil.copytree(artifact["root"], other)
                    artifact["root"] = str(other)
                elif change == "bytes":
                    path.write_bytes(b"modified SDK bytes")
                    item = next(f for f in artifact["files"] if f["path"] == "assets/app.js")
                    item.update(bytes=path.stat().st_size, sha256=sha(path.read_bytes()))
                elif change == "missing":
                    path.unlink()
                elif change == "extra":
                    extra.write_text("extra")
                else:
                    path.unlink()
                    path.symlink_to(self.binary)
                try:
                    with self.assertRaises(R.Invalid):
                        R.reviewed_frontends(frontend)
                finally:
                    if path.is_symlink() or not path.exists():
                        path.unlink(missing_ok=True)
                    path.write_bytes(original)
                    extra.unlink(missing_ok=True)

    def test_private_inputs_reject_modes_owners_hardlinks_symlinks_duplicates_nonroot(self):
        path = Path(self.manifest["services"]["platform"]["env_file"])
        path.chmod(0o644)
        with self.assertRaises(R.Invalid):
            self.run_manifest()
        path.chmod(0o600)
        os.chown(path, 65534, 65534)
        with self.assertRaises(R.Invalid):
            self.run_manifest()
        os.chown(path, 0, 0)
        linked = self.root / "linked.json"
        os.link(path, linked)
        with self.assertRaises(R.Invalid):
            self.run_manifest()
        linked.unlink()
        linked.symlink_to(path)
        with self.assertRaises(R.Invalid):
            R.private_bytes(str(linked))
        with self.assertRaises(R.Invalid):
            R.parsed('{"generation":"final","generation":"recovery"}')
        with self.assertRaises(R.Invalid):
            R.parsed('{"number":NaN}')
        with patch.object(R.os, "geteuid", return_value=1000), self.assertRaises(R.Invalid):
            self.run_manifest()

    def test_private_input_parent_must_be_root_owned_and_0700(self):
        parent = self.root / "public-parent"
        parent.mkdir(mode=0o755)
        path = parent / "private.json"
        path.write_text("{}")
        path.chmod(0o600)
        with self.assertRaises(R.Invalid):
            R.private_json(str(path))
        parent.chmod(0o700)
        self.assertEqual(R.private_json(str(path)), {})
        os.chown(parent, 65534, 65534)
        with self.assertRaises(R.Invalid):
            R.private_json(str(path))
        os.chown(parent, 0, 0)

    def test_known_image_name_cannot_replace_frozen_image_id(self):
        for key in R.IMAGES:
            manifest = copy.deepcopy(self.manifest)
            manifest["images"][key]["id"] = "sha256:" + "0" * 64
            with self.subTest(image=key), self.assertRaises(R.Invalid):
                self.run_manifest(manifest=manifest)

    def test_manifest_generation_output_base_mode_unknown_fields_and_old_format_rejected(self):
        for key, value in (("generation", "u05"), ("output_dir", str(self.base / "recovery")), ("format", "saas-uuid-u05-runtime/1"), ("policy_summary_file", "forbidden")):
            manifest = {**self.manifest, key: value}
            with self.subTest(key=key), self.assertRaises(R.Invalid):
                self.run_manifest(manifest=manifest)
        self.base.mkdir(mode=0o755)
        with self.assertRaises(R.Invalid):
            self.run_manifest()

    def test_launch_plan_has_fixed_images_generation_mounts_and_isolated_python(self):
        self.run_manifest(True)
        root = self.base / "final"
        plan = json.loads((root / "launch-plan.json").read_text())
        self.assertFalse(plan["automatic_start"])
        self.assertTrue(plan["requires_no_other_generation_running"])
        self.assertEqual(len(plan["commands"]), 7)
        for command in plan["commands"]:
            argv = command["argv"]
            self.assertIn("--read-only", argv)
            self.assertIn("ALL", argv)
            self.assertIn("no-new-privileges", argv)
            self.assertIn("saas.uuid-generation=final", argv)
            self.assertNotIn("--env-file", argv)
            self.assertNotIn("--privileged", argv)
            self.assertNotIn("isolated_synthetic_secret", json.dumps(argv))
            self.assertNotIn(str(self.base / "recovery"), json.dumps(argv))
            for i, item in enumerate(argv):
                if item == "--mount":
                    self.assertTrue(argv[i + 1].startswith("type=bind,source=" + str(root) + "/"))
            if command["service"] != "gateway":
                image_index = argv.index(self.manifest["images"]["runtime"]["id"])
                self.assertEqual(argv[image_index + 1:image_index + 3], ["-I", "/run/u06/runner.py"])
        chat = next(c for c in plan["commands"] if c["service"] == "examples_chat")
        self.assertIn("--interactive", chat["argv"])
        self.assertTrue(chat["stdin_open"])
        env = (root / "env/platform.env").read_text()
        self.assertIn("APP_ENV_FILE=/run/u06/empty.env\n", env)
        self.assertIn("PLATFORM_DATABASE_INIT_MODE=imported\n", env)
        self.assertIn("AUTO_CREATE_DATABASE=false\n", env)

    def test_actual_isolated_runner_drops_ambient_env_and_rejects_modified_binary(self):
        isolated = self.root / "runner-fixture"
        isolated.mkdir(mode=0o700)
        (isolated / "state").mkdir(mode=0o700)
        binary = isolated / "service"
        binary.write_text("#!/usr/bin/python3\nimport json,os\nprint(json.dumps(dict(os.environ),sort_keys=True))\n")
        binary.chmod(0o700)
        config = isolated / "service.env"
        config.write_text("SYNTHETIC_ALLOWED=prepared-value\n")
        config.chmod(0o600)
        runner = isolated / "runner.py"
        runner.write_text(R.RUNNER.replace("/run/u06", str(isolated)))
        runner.chmod(0o600)
        canaries = {"PGPASSWORD": "ambient-canary", "PGOPTIONS": "ambient-canary", "JWT_SECRET": "ambient-canary", "PYTHONPATH": "/nonexistent-canary", "PYTHONSTARTUP": "/nonexistent-canary", "BOOTSTRAP_ADMIN_PASSWORD": "ambient-canary", "UNRELATED": "ambient-canary"}
        done = subprocess.run([sys.executable, "-I", str(runner), sha(binary.read_bytes())], env={**os.environ, **canaries}, capture_output=True, timeout=10)
        self.assertEqual(done.returncode, 0, done.stderr.decode())
        env = json.loads((isolated / "state/stdout.log").read_text())
        self.assertEqual(env["SYNTHETIC_ALLOWED"], "prepared-value")
        self.assertTrue(set(canaries).isdisjoint(env))
        self.assertEqual(env["HOME"], str(isolated / "state"))
        self.assertEqual(stat.S_IMODE((isolated / "state/stdout.log").stat().st_mode), 0o600)
        done = subprocess.run([sys.executable, "-I", str(runner), "0" * 64], capture_output=True, timeout=10)
        self.assertEqual(done.returncode, 1)
        self.assertIn(b"U06 private runtime launch validation failed", done.stderr)

    def test_nginx_preserves_existing_route_contract(self):
        config = R.U05.nginx_configuration({a["app"]: a for a in self.frontend["artifacts"]})
        for expected in ("listen 127.0.0.1:13000", "location /stores-api/ { proxy_pass http://127.0.0.1:13010; }", "location /work-orders-api/ { proxy_pass http://127.0.0.1:13011; }", "rewrite ^/oauth-playground-api/(.*)$ /api/$1 break;", 'proxy_set_header Authorization "";', "proxy_set_header Cookie $oauth_cookies;", "location = /callback", "try_files $uri $uri/index.html =404", "location ^~ /stores-api/internal/ { return 404; }", "proxy_set_header Upgrade $http_upgrade;"):
            self.assertIn(expected, config)
        self.assertNotIn("proxy_pass http://127.0.0.1:13008", config)
        self.assertEqual(R.fixed_environment("examples_chat")["CHAT_BIND_ADDR"], "127.0.0.1:13008")

    def test_generated_nginx_only_changes_health_text_preserving_u05_release_routes(self):
        self.run_manifest(True)
        config = (self.base / "final/nginx.conf").read_text()
        expected = R.U05.nginx_configuration({a["app"]: a for a in self.frontend["artifacts"]})
        self.assertEqual(config, expected.replace(r'return 200 "u05\n";', r'return 200 "u06\n";'))
        self.assertIn(r'return 200 "u06\n";', config)
        self.assertIn("/sub-apps/stores/versions/1.0.1-uuid-u05/", config)
        self.assertIn("/sub-apps/work-orders/versions/2.0.1-uuid-u05/", config)


class FrozenContract(unittest.TestCase):
    def test_production_constants_match_accepted_r2_public_descriptor(self):
        report = Path(__file__).parents[4] / "docs/identity-sso/validation/uuid-u05-frontend-builds-r2.json"
        frontend = json.loads(report.read_text(encoding="utf-8"))
        self.assertEqual(sha(R.canonical(descriptor(frontend))), R.FRONTEND_DESCRIPTOR_SHA256)
        self.assertEqual(len(frontend["artifacts"]), 8)
        self.assertEqual(sum(len(a["files"]) for a in frontend["artifacts"]), 253)


if __name__ == "__main__":
    unittest.main()
