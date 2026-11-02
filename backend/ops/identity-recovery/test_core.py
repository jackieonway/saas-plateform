"""File/crypto/guard regressions. No database, socket, Docker or application."""
import contextlib
import copy
import io
import json
import os
from pathlib import Path
import tempfile
import subprocess
import sys
import unittest
from unittest.mock import patch

import core
import inventory


class CoreTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="phase23-core-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.root.chmod(0o700)
        self.config = {"workDirectory": self.root}
        self.material = {name: bytes([index]) * 32 for index, name in enumerate(
            ("encryption", "mac", "witness"), 1)}
        self.binding = {"task": "owned", "cluster": "fixed"}

    def test_private_mode_symlink_and_no_clobber(self):
        path = self.root / "data"
        core.private_write(path, b"protected")
        self.assertEqual(core.private_read(path), b"protected")
        with self.assertRaises(core.RecoveryError):
            core.private_write(path, b"replacement")
        path.chmod(0o644)
        with self.assertRaises(core.RecoveryError):
            core.private_read(path)
        path.chmod(0o600)
        link = self.root / "link"
        link.symlink_to(path)
        with self.assertRaises(core.RecoveryError):
            core.private_read(link)
        link.unlink()
        core.private_write(path, b"new", replace=True)
        self.assertEqual(core.private_read(path), b"new")

    def test_parent_symlink_is_rejected(self):
        directory = self.root / "directory"
        directory.mkdir(mode=0o700)
        alias = self.root / "alias"
        alias.symlink_to(directory, target_is_directory=True)
        with self.assertRaises(core.RecoveryError):
            core.private_write(alias / "value", b"secret")
        self.assertFalse((directory / "value").exists())

    def test_ancestor_swap_cannot_escape_write_or_read(self):
        original = self.root / "parent"
        original.mkdir(mode=0o700)
        outside = self.root / "outside"
        outside.mkdir(mode=0o700)
        core.private_write(original / "value", b"original")
        core.private_write(outside / "value", b"outside-secret")
        safe = core.safe_path
        def raced(path, root=None):
            result = safe(path, root)
            original.rename(self.root / "renamed")
            original.symlink_to(outside, target_is_directory=True)
            return result
        with patch.object(core, "safe_path", side_effect=raced):
            with self.assertRaises(core.RecoveryError):
                core.private_write(original / "new", b"must-not-escape")
        self.assertFalse((outside / "new").exists())
        original.unlink()
        (self.root / "renamed").rename(original)
        with patch.object(core, "safe_path", side_effect=raced):
            with self.assertRaises(core.RecoveryError):
                core.private_read(original / "value")
        self.assertEqual((outside / "value").read_bytes(), b"outside-secret")

    def test_fifo_is_rejected_before_it_can_block_cleanup(self):
        fifo = self.root / "damaged-artifact"
        os.mkfifo(fifo, 0o600)
        program = ("import sys; sys.path.insert(0,sys.argv[1]); import core; "
                   "\ntry: core.private_read(sys.argv[2])"
                   "\nexcept core.RecoveryError: raise SystemExit(7)"
                   "\nraise SystemExit(0)")
        result = subprocess.run([sys.executable, "-B", "-c", program,
                                 str(Path(core.__file__).resolve().parent), str(fifo)],
                                capture_output=True, timeout=2, check=False)
        self.assertEqual(result.returncode, 7)
        self.assertEqual(result.stdout, b"")

    def test_duplicate_and_nonfinite_json(self):
        for value in (b'{"x":1,"x":2}', b'{"x":NaN}', b'{"x":Infinity}'):
            with self.assertRaises(core.RecoveryError):
                core.decode(value)

    def test_authenticated_metadata_rejects_tamper_and_wrong_key(self):
        path = self.root / "manifest"
        core.authenticated_write(path, {"sequence": 7}, self.material["mac"], "manifest")
        self.assertEqual(core.authenticated_read(path, self.material["mac"], "manifest"), {"sequence": 7})
        with self.assertRaises(core.RecoveryError):
            core.authenticated_read(path, self.material["witness"], "manifest")
        with self.assertRaises(core.RecoveryError):
            core.authenticated_read(path, self.material["mac"], "floor")
        value = json.loads(path.read_bytes())
        value["document"]["sequence"] = 6
        core.private_write(path, value, replace=True)
        with self.assertRaises(core.RecoveryError):
            core.authenticated_read(path, self.material["mac"], "manifest")

    def test_aead_binds_kind_task_and_plaintext(self):
        secret = b"private-key-cookie-credential-PHC"
        record = core.encrypt(self.config, self.material, "materials", secret, self.binding)
        ciphertext = (self.root / record["filename"]).read_bytes()
        self.assertNotIn(secret, ciphertext)
        self.assertEqual(core.decrypt(self.config, self.material, "materials", record, self.binding), secret)
        for binding in ({"task": "another", "cluster": "fixed"}, {}):
            with self.assertRaises(core.RecoveryError):
                core.decrypt(self.config, self.material, "materials", record, binding)
        bad = copy.deepcopy(record)
        bad["plaintextBytes"] += 1
        with self.assertRaises(core.RecoveryError):
            core.decrypt(self.config, self.material, "materials", bad, self.binding)
        value = bytes([ciphertext[0] ^ 1]) + ciphertext[1:]
        core.private_write(self.root / record["filename"], value, replace=True)
        with self.assertRaises(core.RecoveryError):
            core.decrypt(self.config, self.material, "materials", record, self.binding)

    def test_argv_never_contains_connection_password(self):
        password = "never-in-argv"
        path = self.root / "admin.json"
        core.private_write(path, {"PGHOST": "127.0.0.1", "PGPORT": "55494", "PGUSER": "postgres",
                                  "PGDATABASE": "postgres", "PGPASSWORD": password})
        pg = core.Pg({"adminConnectionFile": path, "pgBin": "/usr/lib/postgresql/18/bin",
                      "workDirectory": self.root}, "phase23_owned")
        with patch.object(core, "execute", return_value=b"t") as call:
            self.assertEqual(pg.query("SELECT true"), "t")
            argv, env, data = call.call_args.args
            self.assertNotIn(password, " ".join(argv))
            self.assertEqual(env["PGPASSWORD"], password)
            self.assertEqual(data, b"SELECT true")
            self.assertIn("log_statement=none", env["PGOPTIONS"])
            self.assertIn("log_min_error_statement=panic", env["PGOPTIONS"])
            self.assertIn("log_error_verbosity=terse", env["PGOPTIONS"])

    def test_external_floor_requires_independent_sequence(self):
        with self.assertRaisesRegex(core.RecoveryError, "EXTERNAL_MINIMUM_SEQUENCE_REQUIRED"):
            core.latest_witness({}, {}, {}, 0)

    def _loadable_config(self):
        directory = tempfile.TemporaryDirectory(prefix="rust-saas-unified-auth-config-", dir="/tmp")
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        root.chmod(0o700)
        core.private_write(root / "OWNER", (root.name + "-pg55494").encode())
        task_id = "019a221c-e019-7000-8000-000000000001"
        token = "0" * 32
        value = {"format": core.FORMAT, "taskId": task_id, "clusterId": "7701548153213487842",
                 "issuer": "https://auth.saas.test", "taskRoot": str(root),
                 "pgBin": "/usr/lib/postgresql/18/bin", "databases": {},
                 "privateMaterials": {"signing": str(root / "signing")}}
        for field in ("workDirectory", "witnessDirectory", "floorFile", "keyFile", "adminConnectionFile"):
            value[field] = str(root / field)
        for kind, source, target in (("platform", "unified_auth_https_", "unified_auth_restore_"),
                                     ("portal", "unified_auth_portal_", "unified_auth_restore_portal_")):
            value["databases"][kind] = {"source": source + token, "target": target + token,
                "owner": "owned_role", "sourceMarker": "owned/" + task_id,
                "targetMarker": "phase23-recovery/2/" + task_id,
                "connectionFile": str(root / (kind + "-connection"))}
        return root / "config.json", value

    def test_private_config_accepts_new_cluster_and_exact_uint64_boundaries(self):
        path, value = self._loadable_config()
        for cluster_id in ("1", "7701548153213487842", "18446744073709551615"):
            with self.subTest(cluster_id=cluster_id):
                value["clusterId"] = cluster_id
                core.private_write(path, value, replace=path.exists())
                self.assertEqual(core.load_config(path)["clusterId"], cluster_id)

    def test_private_config_rejects_noncanonical_or_out_of_range_cluster_id(self):
        path, value = self._loadable_config()
        for cluster_id in (None, True, 7701548153213487842, 1.0, "", "0", "00", "01", "-1", "+1",
                           " 1", "1 ", "1\n", "\u0661", "18446744073709551616", "1" * 21):
            with self.subTest(cluster_id=cluster_id):
                value["clusterId"] = cluster_id
                core.private_write(path, value, replace=path.exists())
                with self.assertRaisesRegex(core.RecoveryError, "CLUSTER_ID_INVALID"):
                    core.load_config(path)

    def test_valid_config_cannot_bypass_observed_cluster_binding(self):
        with patch.object(core, "Pg") as pg, patch.object(core, "database_record") as record:
            pg.return_value.query.return_value = "7701548153213487843"
            with self.assertRaisesRegex(core.RecoveryError, "CLUSTER_MISMATCH"):
                core.guard({"clusterId": "7701548153213487842", "databases": {}})
            pg.return_value.query.assert_called_once_with("SELECT system_identifier::text FROM pg_control_system()")
            record.assert_not_called()

    def test_catalog_or_acl_change_rejected_even_without_data_change(self):
        config = {"databases": {"platform": {"target": "phase23_target", "targetMarker": "owned"}}}
        saved = {"roles": {"owned": True}, "catalogs": {"platform": {"trigger": "ENABLED", "unique": True}},
                 "databases": {"platform": {"owner": "owner", "acl": []}}}
        with patch.object(core, "role_facts", return_value=saved["roles"]), patch.object(core, "Pg") as pg:
            pg.return_value.json.return_value = {"trigger": "DISABLED", "unique": True}
            with self.assertRaisesRegex(core.RecoveryError, "CATALOG_OR_ACL_CHANGED"):
                core.assert_catalogs(config, saved)
            pg.return_value.json.return_value = saved["catalogs"]["platform"]
            with patch.object(core, "database_record", return_value={"owner": "owner", "acl": ["extra"], "comment": "owned"}):
                with self.assertRaisesRegex(core.RecoveryError, "DATABASE_ACL_CHANGED"):
                    core.assert_catalogs(config, saved)

    def test_catalog_includes_partial_unique_fences(self):
        for sql in ("pg_get_indexdef", "indisunique", "indisvalid", "indisready", "indpred", "indexprs",
                    "pg_get_functiondef", "tgenabled", "default_acl"):
            self.assertIn(sql, core.CATALOG_SQL)

    def test_role_escalation_rejects_before_unchanged_catalog(self):
        saved = {"roles": {"runtime_superuser": False}, "catalogs": {}, "databases": {}}
        with patch.object(core, "role_facts", return_value={"runtime_superuser": True}), \
             patch.object(core, "Pg") as pg:
            with self.assertRaisesRegex(core.RecoveryError, "ROLE_POWERS_OR_MEMBERSHIP_CHANGED"):
                core.assert_catalogs({"databases": {}}, saved)
            pg.assert_not_called()

    def test_rearm_obeys_actual_membership_generation_trigger(self):
        migration = Path(__file__).resolve().parents[2] / "migrations/0001_init.sql"
        source = migration.read_text()
        self.assertIn("membership status changes require a newer identity version", source)
        self.assertIn("AND NEW.identity_version <= OLD.identity_version", source)
        statement = core.membership_rearm_sql("019a221c-e019-7000-8000-000000000001")
        self.assertIn("status='ENABLED',identity_version=identity_version+1", statement)
        self.assertIn("WHERE id=", statement)

    def registration_state(self):
        from test_reconcile import registration_fixture, EPOCH, ID, OTHER
        import reconcile
        before, current = registration_fixture()
        requirements = reconcile.plan(before, current, EPOCH)["requirements"]
        actual = copy.deepcopy(current)
        actual["databases"]["platform"]["context_selection_registrations"] = [requirements["registration_bindings"][ID].copy()]
        actual["databases"]["platform"]["auth_clients"][0]["policy_version"] = requirements["client_floors"][ID]
        saved = {"inventory": before}
        latest = {"payload": {"inventory": current}}
        receipt = {"requirements": requirements,
                   "rearmed": {"clients": [ID], "services": {"context_selection_services": [OTHER]}}}
        return saved, latest, receipt, actual

    def test_registration_gate_accepts_only_exact_selected_binding_and_generated_floor(self):
        values = self.registration_state()
        core.assert_registration_targets(*values)
        for field, changed in (("client_policy_version", 1), ("client_policy_version", 3),
                               ("member_type", "CUSTOMER"), ("application_policy_version", 2),
                               ("client_registry", "OIDC"), ("consumer_service_id", None),
                               ("enabled", False)):
            mutated = copy.deepcopy(values)
            mutated[3]["databases"]["platform"]["context_selection_registrations"][0][field] = changed
            with self.assertRaisesRegex(core.RecoveryError, "REGISTRATION_BINDING_OR_GENERATION_CHANGED"):
                core.assert_registration_targets(*mutated)
        for selected in ("clients", "services"):
            mutated = copy.deepcopy(values)
            mutated[2]["rearmed"][selected] = [] if selected == "clients" else {}
            with self.assertRaisesRegex(core.RecoveryError, "REGISTRATION_BINDING_OR_GENERATION_CHANGED"):
                core.assert_registration_targets(*mutated)

    def test_registration_gate_rejects_extra_rows_and_client_ahead_of_exact_registration_floor(self):
        values = self.registration_state()
        values[3]["databases"]["platform"]["auth_clients"][0]["policy_version"] += 1
        with self.assertRaisesRegex(core.RecoveryError, "REGISTRATION_BINDING_OR_GENERATION_CHANGED"):
            core.assert_registration_targets(*values)
        values = self.registration_state()
        values[3]["databases"]["platform"]["applications"][0]["access_policy_version"] += 1
        with self.assertRaisesRegex(core.RecoveryError, "REGISTRATION_BINDING_OR_GENERATION_CHANGED"):
            core.assert_registration_targets(*values)
        values = self.registration_state()
        extra = dict(values[3]["databases"]["platform"]["context_selection_registrations"][0],
                     client_id="019a1ec0-0000-7000-8000-000000000003")
        values[3]["databases"]["platform"]["context_selection_registrations"].append(extra)
        with self.assertRaisesRegex(core.RecoveryError, "REGISTRATION_BINDING_OR_GENERATION_CHANGED"):
            core.assert_registration_targets(*values)

    def test_start_gate_checks_registration_semantics_even_if_receipt_hash_matches_wrong_state(self):
        saved, latest, receipt, actual = self.registration_state()
        actual["databases"]["platform"]["context_selection_registrations"][0]["client_policy_version"] = 1
        config = {"workDirectory": self.root, "databases": {"platform": {"target": "p"}, "portal": {"target": "q"}}}
        floor = {"sequence": 2}
        receipt.update(checkpointSha256=core.sha(saved), floorSha256=core.sha(floor), sequence=2,
                       targets={"platform": "p", "portal": "q"}, fencedInventorySha256=core.sha(actual),
                       materialPaths={}, materialDigests={})
        with patch.object(core, "checkpoint", return_value=saved), \
             patch.object(core, "latest_witness", return_value=(latest, floor)), \
             patch.object(core, "authenticated_read", return_value=receipt), patch.object(core, "guard"), \
             patch.object(core, "assert_catalogs"), patch.object(core, "material_digests", return_value={}), \
             patch.object(core, "inventory_capture", return_value=actual):
            with self.assertRaisesRegex(core.RecoveryError, "REGISTRATION_BINDING_OR_GENERATION_CHANGED"):
                core.gate(config, self.material, 2)

    def test_new_secrets_cannot_reuse_another_direction_old_secret(self):
        previous = {"auth_client_credentials": [{"secret_digest": core.sha(b"R" * 32)}],
                    "identity_service_clients": [{"secret_hash": core.sha(b"S" * 32)}],
                    "context_selection_services": [], "platform_resource_services": []}
        for request in (
                {"users": [], "clients": [], "services": [{"table": "identity_service_clients", "id": "service", "secret": "R" * 32}]},
                {"users": [], "clients": [{"id": "client", "secret": "S" * 32}], "services": []}):
            with self.assertRaisesRegex(core.RecoveryError, "OLD_SECRET_REUSED"):
                core.fresh_secret_digests(request, [previous])
        request = {"users": [], "clients": [{"id": "client", "secret": "N" * 32}],
                   "services": [{"table": "identity_service_clients", "id": "service", "secret": "N" * 32}]}
        with self.assertRaisesRegex(core.RecoveryError, "CROSS_PURPOSE_SECRET_REUSE"):
            core.fresh_secret_digests(request, [previous])
        request["services"][0]["secret"] = "T" * 32
        self.assertEqual(len(core.fresh_secret_digests(request, [previous])), 2)

    def test_intermediate_secret_denial_survives_overwrite_and_delete(self):
        checkpoint_secret, intermediate, latest_secret = "A" * 32, "B" * 32, "C" * 32
        def snapshot(secret):
            return {"auth_client_credentials": [], "identity_service_clients": [{"secret_hash": core.sha(secret.encode())}],
                    "context_selection_services": [], "platform_resource_services": []}
        request = {"users": [], "clients": [{"id": "client", "secret": intermediate}], "services": []}
        # The old endpoint-only comparison misses B; authenticated history adds
        # its public double digest even after the corresponding row was deleted.
        self.assertEqual(len(core.fresh_secret_digests(request, [snapshot(checkpoint_secret), snapshot(latest_secret)])), 1)
        with self.assertRaisesRegex(core.RecoveryError, "OLD_SECRET_REUSED"):
            core.fresh_secret_digests(request, [snapshot(checkpoint_secret), snapshot(latest_secret)],
                                      {inventory.digest(core.sha(intermediate.encode()))})

    def test_cli_never_prints_exception_payload(self):
        import recovery
        secret = "postgres://raw-password-and-PHC"
        output = io.StringIO()
        with patch("sys.argv", ["recovery.py", "init", "--config", "/private/config"]), \
             patch.object(core, "load_config", side_effect=RuntimeError(secret)), \
             contextlib.redirect_stderr(output):
            self.assertEqual(recovery.main(), 2)
        self.assertNotIn(secret, output.getvalue())
        self.assertEqual(json.loads(output.getvalue())["code"], "RECOVERY_REJECTED")

    def _captured(self):
        """Exercise real file/AEAD/witness/floor orchestration with no DB."""
        work = self.root / "backup"
        work.mkdir(mode=0o700)
        witness = self.root / "witness"
        witness.mkdir(mode=0o700)
        source_material = self.root / "source-material"
        core.private_write(source_material, b"fixture-signing-private-material")
        task_id = "019a221c-e019-7000-8000-000000000001"
        token = "0" * 32
        config = {"taskId": task_id, "clusterId": "7701548153213487841", "issuer": "https://auth.saas.test",
                  "workDirectory": work, "witnessDirectory": witness, "floorFile": self.root / "floor",
                  "privateMaterials": {"signing": source_material}, "databases": {
                      kind: {"source": prefix + token, "target": "unified_auth_restore_" + kind + "_" + token,
                             "sourceMarker": "owned/" + task_id, "targetMarker": "restore/" + task_id,
                             "owner": "owned_role"}
                      for kind, prefix in (("platform", "unified_auth_https_"), ("portal", "unified_auth_portal_"))}}
        empty = inventory.empty_inventory()
        fake = unittest.mock.MagicMock()
        fake.tool.return_value = b"PGDMP-real-orchestration-fixture"
        def query(sql):
            if sql == core.CATALOG_SQL:
                return {"schema": "current"}
            return {"catalog": {}, "tables": empty["databases"]["platform"]}
        fake.json.side_effect = query
        records = {kind: {"owner": "owned_role", "acl": [], "connections": -1} for kind in config["databases"]}
        with patch.object(core, "guard", return_value=records), patch.object(core, "Pg", return_value=fake), \
             patch.object(core, "role_facts", return_value={"owned": True}), \
             patch.object(core, "inventory_capture", return_value=empty), \
             patch.object(inventory, "project_database", side_effect=lambda kind, raw: empty["databases"][kind]):
            result = core.capture(config, self.material)
            self.assertEqual(result["sequence"], 1)
            result = core.capture_deny(config, self.material, 1)
            self.assertEqual(result["sequence"], 2)
        return config

    def test_driver_uses_authenticated_witness_and_independent_sequence(self):
        config = self._captured()
        saved = core.checkpoint(config, self.material)
        latest, floor = core.latest_witness(config, self.material, saved, 2)
        self.assertEqual(latest["sequence"], 2)
        self.assertIsNotNone(floor["facts"])
        self.assertEqual(core.status(config, self.material, 2)["status"], "VERIFIED")
        old = copy.deepcopy(floor)
        old["sequence"] = 1
        core.authenticated_write(config["floorFile"], old, self.material["mac"], "floor", replace=True)
        with self.assertRaisesRegex(core.RecoveryError, "EXTERNAL_FLOOR_ROLLBACK"):
            core.latest_witness(config, self.material, saved, 2)

    def test_corrupt_archive_rejected_before_any_create_database(self):
        config = self._captured()
        saved = core.checkpoint(config, self.material)
        path = config["workDirectory"] / saved["archives"]["platform"]["filename"]
        value = core.private_read(path)
        core.private_write(path, bytes([value[0] ^ 1]) + value[1:], replace=True)
        with patch.object(core, "guard"), patch.object(core, "Pg") as pg:
            with self.assertRaisesRegex(core.RecoveryError, "ARCHIVE_DIGEST_MISMATCH"):
                core.restore_new(config, self.material, 2)
            pg.return_value.query.assert_not_called()


if __name__ == "__main__":
    unittest.main()
