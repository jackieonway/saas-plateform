"""Closed candidate inputs and explicit frontend output boundaries; no services."""

import importlib.util
import pathlib
import tempfile
import unittest


loader = importlib.util.spec_from_file_location("snapshot_build", pathlib.Path(__file__).with_name("snapshot-build.py"))
snapshot = importlib.util.module_from_spec(loader)
loader.loader.exec_module(snapshot)


class CandidateSnapshotTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="phase25-snapshot-pure-")
        self.addCleanup(temporary.cleanup)
        self.root = pathlib.Path(temporary.name)

    def file(self, relative, content="PURE_SOURCE"):
        target = self.root / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content)
        return target

    def test_full_candidate_closes_business_vendor_migrations_and_contract_inputs(self):
        required = (
            "saas-stores/backend/vendor/operational-telemetry/Cargo.toml",
            "saas-stores/backend/migrations/0001_init.sql",
            "saas-work-orders/backend/src/bin/work-orders-migrate.rs",
            "saas-work-orders/backend/tests/current.rs",
            "saas-plateform/backend/tests/public.rs",
            "saas-plateform/backend/tests/fixtures/micro-app-protocol/protocol.rs",
            "saas-plateform/frontend/deploy/tests/static-image-policy.mjs",
            "saas-plateform/backend/tools/platform-admin/manifests/clients.json",
            "saas-plateform/backend/tools/platform-admin/schemas/client.json",
            "saas-plateform/backend/tools/platform-admin/tests/cli.rs",
            "saas-micro-app-sdk/index.mjs",
            "saas-micro-app-sdk/release.json",
            "saas-micro-app-sdk/releases/3.0.0/protocol.rs",
            "saas-examples/oidc-rp/tests/protocol.test.mjs",
            "saas-examples/websocket/frontend.tsx",
            "saas-examples/websocket/backend/Cargo.lock",
            "saas-examples/dependencies.lock.json",
            "saas-examples/.node-version",
            "saas-examples/scripts/build-provenance.mjs",
            "saas-examples/micro-vue/smoke-test.cjs",
            "saas-customer-portal/backend/migrations/0001_init.sql",
        )
        for name in required:
            self.file(name)
        actual = {name.as_posix() for name in snapshot.source_files(self.root, False, True)}
        self.assertTrue(set(required).issubset(actual))

    def test_inputs_exclude_dependency_trees_and_private_material(self):
        allowed = self.file("saas-micro-app-sdk/src/client.mjs")
        for name in ("saas-micro-app-sdk/src/node_modules/module/index.mjs",
                     "saas-micro-app-sdk/src/.env", "saas-micro-app-sdk/src/key.pem",
                     "saas-micro-app-sdk/unspecified-secret.conf"):
            self.file(name)
        (allowed.parent / "link.mjs").symlink_to(allowed)
        actual = snapshot.source_files(self.root, False, True)
        self.assertEqual(actual, [allowed.relative_to(self.root)])

    def test_public_project_npm_controls_are_frozen_but_credentials_are_rejected(self):
        config = self.file("saas-stores/frontend/.npmrc", "engine-strict=true\n")
        self.assertIn(config.relative_to(self.root), snapshot.source_files(self.root, False, True))
        config.write_text("//registry.invalid/:_authToken=PURE_FAKE_TOKEN\n")
        with self.assertRaisesRegex(SystemExit, "public numeric or boolean"):
            snapshot.source_files(self.root, False, True)

    def test_frontend_override_requires_supported_unique_completed_real_output(self):
        output = self.root / "build"
        self.file("build/index.html", "PURE_BUILD")
        self.file("build/assets/app.js")
        key = "saas-plateform/frontend"
        selected = snapshot.frontend_outputs([key + "=" + str(output)])
        self.assertEqual(selected[key][1], [pathlib.Path("assets/app.js"), pathlib.Path("index.html")])
        for values in (["unknown=" + str(output)], [key + "=relative"],
                       [key + "=" + str(output), key + "=" + str(output)]):
            with self.assertRaises(SystemExit):
                snapshot.frontend_outputs(values)
        (output / "nested-link").symlink_to(self.root, target_is_directory=True)
        with self.assertRaisesRegex(SystemExit, "links are rejected"):
            snapshot.frontend_outputs([key + "=" + str(output)])

    def test_frontend_override_rejects_secrets_without_mutating_output(self):
        self.file("build/index.html")
        forbidden = self.file("build/.env.local", "PURE_FAKE_SECRET")
        with self.assertRaisesRegex(SystemExit, "forbidden private file"):
            snapshot.frontend_outputs(["saas-stores/frontend=" + str(self.root / "build")])
        self.assertEqual(forbidden.read_text(), "PURE_FAKE_SECRET")


if __name__ == "__main__":
    unittest.main()
