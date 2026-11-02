"""Pure protected-file adapter regression tests; no DB, sockets or services."""
import base64
import copy
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


loader = importlib.util.spec_from_file_location("phase23_recovery_files", Path(__file__).with_name("phase23-recovery-files.py"))
adapter = importlib.util.module_from_spec(loader)
loader.loader.exec_module(adapter)
core = adapter.core


class FilesTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="rust-saas-unified-auth-files-", dir="/tmp")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.root.chmod(0o700)
        binding = patch.object(adapter.cluster_binding, 'load_binding',
            return_value={'systemIdentifier': '18446744073709551614', 'dataDirectory': str(self.root / 'data')})
        binding.start()
        self.addCleanup(binding.stop)
        self.counter = 0
        core.private_write(self.root / "OWNER", (self.root.name + "-pg55494").encode())
        self.source, self.manifest, self.tls = self.fixture()
        self.env_path, self.manifest_path = self.root / "source.json", self.root / "manifest.json"
        self.tls_path, self.config_path = self.root / "tls.json", self.root / "config.json"
        for path, value in ((self.env_path, self.source), (self.manifest_path, self.manifest), (self.tls_path, self.tls)):
            core.private_write(path, value)
        self.initial = {path: core.private_read(path) for path in self.root.iterdir() if path.is_file()}

    def object_id(self):
        self.counter += 1
        return "019a1234-0000-7000-8000-" + format(self.counter, "012x")

    def write(self, name, value):
        path = self.root / name
        core.private_write(path, value)
        return str(path)

    def fixture(self):
        names = {entry[-1] for entry in adapter.SERVICE_BINDINGS if entry[-1] != "keyAdminService"}
        names |= {"deployment", "client", "portalClient", "admin", "employee", "customer",
                  "adminMember", "employeeMember", "customerMember", "applicant", "reviewer", "publisher"}
        ids = {key: self.object_id() for key in sorted(names)}
        database = "unified_auth_https_" + self.object_id().replace("-", "")
        prefix = "uah_" + database[-12:]
        task_id = ids["deployment"]
        portal_name = "unified_auth_portal_" + database[-32:]
        manifest = {"database": database, "rolePrefix": prefix, "clusterId": '18446744073709551614',
                    "portal": {"database": portal_name, "owner": prefix + "_portal_owner",
                               "runtime": prefix + "_portal_runtime", "marker": "unified-auth-https-portal/1/" + task_id,
                               "initializationPending": False}}
        tls = {name: self.write("source-" + name, ("-----BEGIN " + ("PRIVATE KEY" if name == "key" else "CERTIFICATE")
                    + "-----\nPURE_FILE_FIXTURE_" + name).encode()) for name in ("ca", "certificate", "key")}
        signing = self.write("source-signing.json", {"keys": [{"kid": name, "n": "public", "e": "AQAB",
                          "privateKeyPem": "pure-file-test-private-material"}
                          for name in ("owned-https", "phase23-b", "phase23-c", "phase23-d")]})
        ring = lambda name: self.write("source-" + name + ".json", {"activeKid": "source", "keys": [
            {"kid": "source", "key": base64.urlsafe_b64encode(bytes([self.counter % 128]) * 32).rstrip(b"=").decode()}]})
        source = {"ids": ids, "identity": {"OIDC_ISSUER": adapter.ISSUER, "OIDC_KEYSET_PATH": signing,
            "SSO_TOTP_ENCRYPTION_KEY": "source-totp-" + "1" * 40,
            "AUTH_INVITATION_KEYSET_PATH": self.write("source-invitation.json", {
                "purpose": "saas-identity/auth-invitation/1", "activeVersion": 7,
                "keys": [{"version": 7, "key": base64.urlsafe_b64encode(b"i" * 32).rstrip(b"=").decode()}]}),
            "CONTEXT_SELECTION_KEY_FILE": ring("context"), "IDENTITY_METRICS_TOKEN": "source-metrics-" + "2" * 40},
          "platform": {"BFF_OIDC_ISSUER": adapter.ISSUER, "BFF_OIDC_CLIENT_ID": ids["client"],
            "BFF_OIDC_CLIENT_SECRET": "source-console-" + "3" * 40, "JWT_SECRET": "source-jwt-" + "4" * 40,
            "BFF_TOKEN_KEY_FILE": ring("bff")},
          "portal": {"OIDC_ISSUER": adapter.ISSUER, "OIDC_CLIENT_ID": ids["portalClient"],
            "OIDC_CLIENT_SECRET": "source-portal-" + "5" * 40, "PORTAL_TOKEN_KEY_FILE": ring("portal")},
          "accounts": {name: {"username": "fixture-" + name, "password": "source-password-" + name + "7!"}
                       for name in ("admin", "employee", "customer", "applicant", "reviewer", "publisher")},
          "phase19": {"contractVersion": "unified-auth-phase19/1", "enabled": True},
          "phase20": {"contractVersion": "unified-auth-phase20/1", "enabled": True,
                      "recipientRef": "owned-recipient", "targets": {},
                      "collector": {"controlUrl": "http://127.0.0.1:23443", "controlSecret": "source-collector-" + "6" * 40}}}
        for container in ("identity", "platform", "portal"):
            name = portal_name if container == "portal" else database
            source[container]["DATABASE_URL"] = ("postgres://" + prefix + "_" + container
                + "_runtime:source-db-" + container + "-password@127.0.0.1:55494/" + name)
        key_admin = self.object_id()
        for index, (container, secret, field, _, lookup, _, metadata) in enumerate(adapter.SERVICE_BINDINGS):
            source[container][secret] = "source-service-" + str(index) + "-" + "a" * 40
            source[container][field] = "public-profile-id" if lookup == "service_id" else (
                key_admin if metadata == "keyAdminService" else ids[metadata])
        for container, field in adapter.CA_BINDINGS:
            source[container][field] = tls["ca"]
        standard_ids = {key: self.object_id() for key in ("user", "otherUser", "thirdClient", "offlineClient",
                                                        "machineClient", "resourceClient")}
        public = {key: "p21-public-" + key for key in ("third", "offline", "machine", "resource")}
        files = {key: self.write("source-" + key + "-secret", ("source-standard-" + key + "-" + "b" * 40).encode())
                 for key in public}
        rp = {key: {"OIDC_ISSUER": adapter.ISSUER, "OIDC_CLIENT_ID": public[key],
                    "OIDC_CLIENT_SECRET_FILE": files[key], "RP_ORIGIN": adapter.RP_ORIGIN,
                    "RP_BIND_HOST": "127.0.0.1", "RP_BIND_PORT": "23102", "NODE_EXTRA_CA_CERTS": tls["ca"],
                    "OIDC_SCOPES": "openid profile" + (" offline_access" if key == "offline" else "")}
              for key in ("third", "offline")}
        source["phase21"] = {"contractVersion": "unified-auth-phase21/1", "ids": standard_ids,
            "accounts": {key: {"username": "fixture-p21-" + key, "password": "source-p21-password-" + key + "7!"}
                         for key in ("user", "otherUser")}, "publicClients": public, "rp": rp,
            "machine": {"issuer": adapter.ISSUER, "clientId": public["machine"], "secretFile": files["machine"],
                        "scope": "demo.read", "resource": adapter.RP_ORIGIN + "/demo-resource"},
            "resource": {"clientId": public["resource"], "secretFile": files["resource"]}}
        source["phase23"] = {"contractVersion": "unified-auth-phase23/1", "ids": {
            "keyAdminService": key_admin, "retireMember": self.object_id(),
            "markerOrganization": self.object_id()},
            "signingKids": {"a": "owned-https", "b": "phase23-b", "c": "phase23-c", "d": "phase23-d"},
            "rpOrigin": adapter.RP_ORIGIN, "normalTokenTtlSeconds": 300,
            "keyRetirementRequestedProfileSeconds": 30, "longCarrierRequestedProfileSeconds": 600}
        return source, manifest, tls

    def prepare(self):
        result = adapter.prepare(self.root, self.env_path, self.manifest_path, self.config_path, self.tls_path)
        return result, core.load_config(self.config_path)

    def rearm_inputs(self):
        _, config = self.prepare()
        tables = {name: [] for name in ("users", "auth_clients", "memberships", *adapter.SERVICE_TABLES)}
        users = []
        for name in ("admin", "employee", "customer"):
            object_id = self.source["ids"][name]
            users.append(object_id)
            tables["users"].append({"id": object_id, "username_sha256": adapter.inventory.digest(self.source["accounts"][name]["username"])})
            tables["memberships"].append({"id": self.source["ids"][name + "Member"], "user_id": object_id})
        object_id = self.source["phase21"]["ids"]["user"]
        users.append(object_id)
        tables["users"].append({"id": object_id, "username_sha256": adapter.inventory.digest(self.source["phase21"]["accounts"]["user"]["username"])})
        clients = [self.source["ids"][key] for key in ("client", "portalClient")]
        tables["auth_clients"] = [{"id": item, "client_id": item, "client_type": "CONFIDENTIAL"} for item in clients]
        for key in ("third", "offline", "machine", "resource"):
            object_id = self.source["phase21"]["ids"][key + "Client"]
            clients.append(object_id)
            tables["auth_clients"].append({"id": object_id, "client_id": self.source["phase21"]["publicClients"][key], "client_type": "CONFIDENTIAL"})
        services = {table: [] for table in adapter.SERVICE_TABLES}
        for container, _, field, table, lookup, purpose, metadata in adapter.SERVICE_BINDINGS:
            object_id = self.source["phase23"]["ids"][metadata] if metadata == "keyAdminService" else self.source["ids"][metadata]
            tables[table].append({"id": object_id, lookup: self.source[container][field], "purpose": purpose})
            services[table].append(object_id)
        selection = {"users": users, "clients": clients, "memberships": [row["id"] for row in tables["memberships"]], "services": services}
        requirements = {"eligible_users": users, "eligible_clients": clients,
                        "eligible_memberships": selection["memberships"], "eligible_services": services}
        saved = {"inventory": {"databases": {"platform": copy.deepcopy(tables)}}}
        latest = {"payload": {"inventory": {"databases": {"platform": tables}}}}
        restored = {label: core.private_read(path) for label, path in config["privateMaterials"].items()}
        return config, selection, saved, latest, requirements, restored

    def assemble(self, values):
        config, selection, saved, latest, requirements, restored = values
        return adapter.assemble_rearm(config, self.source, selection, saved, latest, requirements,
            self.root / "target-env.json", self.root / "rearm.json", restored)

    def assert_sources_unchanged(self):
        self.assertTrue(all(core.private_read(path) == data for path, data in self.initial.items()))

    def test_prepare_binds_owned_exact_targets_and_thirteen_materials(self):
        result, config = self.prepare()
        self.assertEqual(result["materials"], 13)
        self.assertEqual(set(config["privateMaterials"]), adapter.LABELS)
        self.assertTrue(config["databases"]["platform"]["target"].startswith("unified_auth_restore_"))
        self.assertEqual(core.decode(core.private_read(config["adminConnectionFile"]))["PGDATABASE"], "postgres")
        self.assert_sources_unchanged()

    def test_retired_ttl_environment_override_rejects_before_first_write(self):
        for container in ("identity", "platform", "portal"):
            for field in ("OIDC_CODE_TTL_SECONDS", "OIDC_TOKEN_TTL_SECONDS", "OIDC_REFRESH_TTL_SECONDS"):
                with self.subTest(container=container, field=field):
                    source = copy.deepcopy(self.source)
                    source[container][field] = "600"
                    core.private_write(self.env_path, source, replace=True)
                    before = set(self.root.iterdir())
                    with self.assertRaisesRegex(core.RecoveryError, "RETIRED_TTL_OVERRIDE"):
                        adapter.prepare(self.root, self.env_path, self.manifest_path, self.config_path, self.tls_path)
                    self.assertEqual(set(self.root.iterdir()), before)

    def test_unknown_nested_and_process_secrets_reject_before_first_write(self):
        for mutate in (lambda source: source["platform"].update(UNRECOGNIZED_SERVICE_SECRET="must-not-copy"),
                       lambda source: source["phase20"]["collector"].update(anotherSecret="must-not-copy")):
            source = copy.deepcopy(self.source)
            mutate(source)
            core.private_write(self.env_path, source, replace=True)
            before = set(self.root.iterdir())
            with self.assertRaises(core.RecoveryError):
                adapter.prepare(self.root, self.env_path, self.manifest_path, self.config_path, self.tls_path)
            self.assertEqual(set(self.root.iterdir()), before)

    def test_prepare_rejects_inexact_manifest_ca_and_weak_private_mode(self):
        core.private_write(self.manifest_path, {**self.manifest, "extra": True}, replace=True)
        with self.assertRaises(core.RecoveryError):
            self.prepare()
        core.private_write(self.manifest_path, self.manifest, replace=True)
        Path(self.tls["ca"]).chmod(0o644)
        with self.assertRaises(core.RecoveryError):
            self.prepare()

    def test_rearm_rotates_all_credentials_keeps_real_restored_signing_tls(self):
        values = self.rearm_inputs()
        result = self.assemble(values)
        target = core.decode(core.private_read(self.root / "target-env.json"))
        request = core.decode(core.private_read(self.root / "rearm.json"))
        self.assertEqual((result["users"], result["clients"], result["services"], result["runtimePasswords"]), (4, 6, 11, 3))
        self.assertNotIn("OIDC_TOKEN_TTL_SECONDS", target["identity"])
        self.assertEqual(set(target["accounts"]), {"admin", "employee", "customer"})
        self.assertEqual(set(target["phase21"]["accounts"]), {"user"})
        runtime_path = target["phase23"]["runtimePasswordsFile"]
        runtime_bytes = core.private_read(runtime_path)
        self.assertEqual(core.sha(runtime_bytes), target["phase23"]["runtimePasswordsSha256"])
        passwords = core.decode(runtime_bytes)
        fresh_values = [entry["password"] for entry in request["users"]]
        fresh_values += [entry["secret"] for entry in request["clients"] + request["services"]]
        fresh_values += [entry["password"] for entry in passwords.values()]
        fresh_values += [target["identity"]["SSO_TOTP_ENCRYPTION_KEY"], target["identity"]["IDENTITY_METRICS_TOKEN"],
                         target["platform"]["JWT_SECRET"], target["phase20"]["collector"]["controlSecret"]]
        self.assertEqual(len(fresh_values), len(set(fresh_values)))
        for container in ("identity", "platform", "portal"):
            self.assertIn(passwords[container]["password"], target[container]["DATABASE_URL"])
            self.assertIn("unified_auth_restore_", target[container]["DATABASE_URL"])
        for label in ("signing", "tls_ca", "tls_certificate", "tls_key"):
            self.assertEqual(core.private_read(request["materials"][label]), values[-1][label])
        invitation = core.decode(core.private_read(request["materials"]["invitation"]))
        self.assertEqual(invitation["activeVersion"], 8)
        original_invitation = core.decode(values[-1]["invitation"])
        self.assertEqual(invitation["keys"][:-1], original_invitation["keys"])
        self.assertEqual(len(invitation["keys"]), 2)
        self.assertEqual(invitation["keys"][-1]["version"], invitation["activeVersion"])
        self.assertNotIn(invitation["keys"][-1]["key"], {item["key"] for item in original_invitation["keys"]})
        for label in adapter.RING_BINDINGS:
            ring = core.decode(core.private_read(request["materials"][label]))
            self.assertEqual(len(ring["keys"]), 1)
            self.assertNotEqual(ring["activeKid"], "source")
        self.assertNotIn("source-password-", json.dumps(target))
        self.assertNotIn("source-service-", json.dumps(target))
        self.assertNotIn("source-standard-", json.dumps(target))
        self.assert_sources_unchanged()

    def test_invitation_rearm_retains_exact_history_and_changes_only_active_generation(self):
        values = self.rearm_inputs()
        historical = {"purpose": "saas-identity/auth-invitation/1", "activeVersion": 4,
                      "keys": [{"version": version, "key": adapter.b64(bytes([version]) * 32)}
                               for version in (1, 4, 7)]}
        values[-1]["invitation"] = core.canonical(historical)
        closed = [{"status": "CONSUMED", "key_version": 4}, {"status": "REVOKED", "key_version": 7}]
        for tables in (values[2]["inventory"]["databases"]["platform"],
                       values[3]["payload"]["inventory"]["databases"]["platform"]):
            tables["auth_invitations"] = copy.deepcopy(closed)
        saved, latest, selection = copy.deepcopy(values[2]), copy.deepcopy(values[3]), copy.deepcopy(values[1])
        with patch.object(core, "execute", side_effect=AssertionError("no DB, HTTP or subprocess permitted")):
            self.assemble(values)
        request = core.decode(core.private_read(self.root / "rearm.json"))
        result = core.decode(core.private_read(request["materials"]["invitation"]))
        self.assertEqual(result["purpose"], historical["purpose"])
        self.assertEqual(result["keys"][:-1], historical["keys"])
        self.assertEqual(result["activeVersion"], 8)
        self.assertEqual(result["keys"][-1]["version"], 8)
        self.assertNotIn(result["keys"][-1]["key"], {item["key"] for item in historical["keys"]})
        self.assertEqual((values[2], values[3], values[1]), (saved, latest, selection))
        self.assertEqual(len(request["services"]), 11)
        self.assertEqual({row["table"] for row in request["services"]}, set(adapter.SERVICE_TABLES))
        self.assert_sources_unchanged()

    def test_invitation_fifteen_historical_versions_fill_the_existing_sixteen_version_limit(self):
        values = self.rearm_inputs()
        old = {"purpose": "saas-identity/auth-invitation/1", "activeVersion": 15,
               "keys": [{"version": version, "key": adapter.b64(bytes([version]) * 32)}
                        for version in range(1, 16)]}
        values[-1]["invitation"] = core.canonical(old)
        self.assemble(values)
        request = core.decode(core.private_read(self.root / "rearm.json"))
        result = core.decode(core.private_read(request["materials"]["invitation"]))
        self.assertEqual(result["keys"][:-1], old["keys"])
        self.assertEqual((len(result["keys"]), result["activeVersion"], result["keys"][-1]["version"]),
                         (16, 16, 16))

    def test_invitation_full_keyset_fails_before_randomness_or_output_without_discarding_history(self):
        values = self.rearm_inputs()
        old = {"purpose": "saas-identity/auth-invitation/1", "activeVersion": 16,
               "keys": [{"version": version, "key": adapter.b64(bytes([version]) * 32)}
                        for version in range(1, 17)]}
        values[-1]["invitation"] = core.canonical(old)
        before = set(self.root.iterdir())
        with patch.object(adapter.secrets, "token_hex", side_effect=AssertionError("must fail before generation")):
            with self.assertRaisesRegex(core.RecoveryError, "^RECOVERY_FILES_INVITATION_KEY_CAPACITY_EXHAUSTED$"):
                self.assemble(values)
        self.assertEqual(set(self.root.iterdir()), before)
        self.assertEqual(core.decode(values[-1]["invitation"]), old)
        self.assert_sources_unchanged()

    def test_invitation_history_is_reserved_against_new_credential_and_active_key_reuse(self):
        values = self.rearm_inputs()
        old = core.decode(values[-1]["invitation"])
        old_hex = base64.urlsafe_b64decode(old["keys"][0]["key"] + "=").hex()
        sequence = iter([old_hex, *[format(index, "064x") for index in range(1, 65)]])
        with patch.object(adapter.secrets, "token_hex", side_effect=lambda _size: next(sequence)):
            self.assemble(values)
        request = core.decode(core.private_read(self.root / "rearm.json"))
        result = core.decode(core.private_read(request["materials"]["invitation"]))
        self.assertEqual(result["keys"][:-1], old["keys"])
        self.assertNotEqual(result["keys"][-1]["key"], old["keys"][0]["key"])
        self.assertTrue(all(old_hex not in row["password"] for row in request["users"]))
        self.assertTrue(all(row["secret"] != old_hex for row in request["clients"] + request["services"]))

    def test_invalid_invitation_history_cannot_be_preserved_into_a_new_runtime(self):
        values = self.rearm_inputs()
        original = core.decode(values[-1]["invitation"])
        invalid = []
        for mutate in (
            lambda value: value.update(activeVersion=99),
            lambda value: value["keys"].append(copy.deepcopy(value["keys"][0])),
            lambda value: value["keys"].append({"version": 8, "key": value["keys"][0]["key"]}),
            lambda value: value["keys"][0].update(key=adapter.b64(bytes(32))),
            lambda value: value["keys"][0].update(key="invalid"),
            lambda value: value["keys"][0].update(version=True),
            lambda value: value["keys"][0].update(extra="must-not-copy"),
        ):
            value = copy.deepcopy(original)
            mutate(value)
            invalid.append(value)
        before = set(self.root.iterdir())
        for value in invalid:
            with self.subTest(kind=list(value["keys"][0])):
                values[-1]["invitation"] = core.canonical(value)
                with self.assertRaises(core.RecoveryError):
                    self.assemble(values)
                self.assertEqual(set(self.root.iterdir()), before)

    def test_rearm_uses_restored_bytes_when_original_private_key_is_unavailable(self):
        values = self.rearm_inputs()
        original_signing = Path(values[0]["privateMaterials"]["signing"])
        original_signing.unlink()
        self.assemble(values)
        request = core.decode(core.private_read(self.root / "rearm.json"))
        self.assertEqual(core.private_read(request["materials"]["signing"]), values[-1]["signing"])

    def authenticated_adapter_inputs(self):
        values = self.rearm_inputs()
        config, selection, saved, latest, requirements, restored = values
        core.private_directory(config["workDirectory"], create=True)
        core.private_directory(config["workDirectory"] / "restored-materials", create=True)
        for label, data in restored.items():
            core.private_write(config["workDirectory"] / "restored-materials" / (label + ".material"), data)
        saved.update(archives={"materials": {"authenticated": "fixture"}}, binding={"owned": "fixture"})
        floor = {"sequence": 7, "authenticated": "external-fixture-floor"}
        material = {"mac": b"m" * 32, "encryption": b"e" * 32, "witness": b"w" * 32}
        receipt = {"format": core.FORMAT, "checkpointSha256": core.sha(saved), "sequence": 7,
                   "floorSha256": core.sha(floor), "requirements": requirements,
                   "targets": {kind: database["target"] for kind, database in config["databases"].items()}}
        core.authenticated_write(config["workDirectory"] / "start.json", receipt, material["mac"], "start")
        selection_file = self.root / "selection.json"
        core.private_write(selection_file, selection)
        archived = core.canonical({label: base64.b64encode(data).decode() for label, data in restored.items()})
        return values, floor, material, archived, selection_file

    def test_actual_adapter_calls_authenticated_readers_and_never_database(self):
        values, floor, material, archived, selection = self.authenticated_adapter_inputs()
        Path(values[0]["privateMaterials"]["signing"]).unlink()
        with patch.object(core, "keys", return_value=material), \
             patch.object(core, "checkpoint", return_value=values[2]), \
             patch.object(core, "latest_witness", return_value=(values[3], floor)) as witness, \
             patch.object(core, "decrypt", return_value=archived), \
             patch.object(core, "Pg", side_effect=AssertionError("database forbidden")), \
             patch.object(core, "execute", side_effect=AssertionError("subprocess forbidden")):
            result = adapter.rearm(self.config_path, self.env_path, selection,
                self.root / "target-env.json", self.root / "rearm.json", 7)
        self.assertEqual(result["users"], 4)
        self.assertEqual(witness.call_args.args[-1], 7)
        request = core.decode(core.private_read(self.root / "rearm.json"))
        self.assertEqual(core.private_read(request["materials"]["signing"]), values[-1]["signing"])

    def test_tampered_recovered_material_rejected_before_creating_outputs(self):
        values, floor, material, archived, selection = self.authenticated_adapter_inputs()
        core.private_write(values[0]["workDirectory"] / "restored-materials/signing.material", b"tampered", replace=True)
        before = set(self.root.iterdir())
        with patch.object(core, "keys", return_value=material), \
             patch.object(core, "checkpoint", return_value=values[2]), \
             patch.object(core, "latest_witness", return_value=(values[3], floor)), \
             patch.object(core, "decrypt", return_value=archived), \
             self.assertRaisesRegex(core.RecoveryError, "RESTORED_MATERIAL_CHANGED"):
            adapter.rearm(self.config_path, self.env_path, selection,
                self.root / "target-env.json", self.root / "rearm.json", 7)
        self.assertEqual(set(self.root.iterdir()), before)

    def test_authenticated_start_must_bind_the_latest_external_floor(self):
        values, floor, material, archived, selection = self.authenticated_adapter_inputs()
        floor["sequence"] = 8
        before = set(self.root.iterdir())
        with patch.object(core, "keys", return_value=material), \
             patch.object(core, "checkpoint", return_value=values[2]), \
             patch.object(core, "latest_witness", return_value=(values[3], floor)), \
             patch.object(core, "decrypt", return_value=archived), \
             self.assertRaisesRegex(core.RecoveryError, "START_REQUIREMENTS_UNBOUND"):
            adapter.rearm(self.config_path, self.env_path, selection,
                self.root / "target-env.json", self.root / "rearm.json", 7)
        self.assertEqual(set(self.root.iterdir()), before)

    def test_selection_ineligible_and_duplicate_ids_reject_before_output(self):
        values = self.rearm_inputs()
        for candidate in (self.object_id(), values[1]["users"][0]):
            values[1]["users"].append(candidate)
            before = set(self.root.iterdir())
            with self.assertRaises(core.RecoveryError):
                self.assemble(values)
            self.assertEqual(set(self.root.iterdir()), before)
            values[1]["users"].pop()

    def test_public_and_internal_client_id_confusion_rejects(self):
        values = self.rearm_inputs()
        values[3]["payload"]["inventory"]["databases"]["platform"]["auth_clients"][-1]["client_id"] = self.source["phase21"]["ids"]["resourceClient"]
        before = set(self.root.iterdir())
        with self.assertRaisesRegex(core.RecoveryError, "MAPPING_INVALID"):
            self.assemble(values)
        self.assertEqual(set(self.root.iterdir()), before)

    def test_unselected_required_runtime_service_rejects_no_old_secret_fallback(self):
        values = self.rearm_inputs()
        values[1]["services"]["identity_service_clients"].pop()
        before = set(self.root.iterdir())
        with self.assertRaisesRegex(core.RecoveryError, "RUNTIME_SERVICE_SELECTION_INCOMPLETE"):
            self.assemble(values)
        self.assertEqual(set(self.root.iterdir()), before)

    def test_source_environment_and_restored_material_substitution_rejects(self):
        values = self.rearm_inputs()
        self.source["accounts"]["admin"]["password"] = "substituted-source-password"
        before = set(self.root.iterdir())
        with self.assertRaisesRegex(core.RecoveryError, "RESTORED_ENVIRONMENT_MISMATCH"):
            self.assemble(values)
        self.assertEqual(set(self.root.iterdir()), before)

    def test_outputs_do_not_clobber_and_clean_owned_partial_files(self):
        output = adapter.Outputs(self.root, (self.env_path,))
        with self.assertRaises(core.RecoveryError):
            output.add(self.env_path, b"must-not-overwrite")
        a, b = self.root / "planned-a", self.root / "planned-b"
        output.add(a, b"first-owned")
        output.add(b, b"second-owned")
        real = core.private_write
        def failure(path, value, replace=False):
            if path == b:
                raise core.RecoveryError("INJECTED_WRITE_FAILURE")
            return real(path, value, replace)
        with patch.object(core, "private_write", side_effect=failure), self.assertRaises(core.RecoveryError):
            output.commit()
        self.assertFalse(a.exists())
        self.assertFalse(b.exists())
        self.assert_sources_unchanged()

    def test_symlink_source_rejected_and_stdout_contains_only_fixed_metadata(self):
        alias = self.root / "alias.json"
        alias.symlink_to(self.env_path)
        with self.assertRaises(core.RecoveryError):
            adapter.prepare(self.root, alias, self.manifest_path, self.config_path, self.tls_path)
        result, _ = self.prepare()
        encoded = json.dumps(result)
        self.assertNotIn(str(self.root), encoded)
        self.assertNotIn("source-", encoded)
        self.assertFalse(any(type(value) is str and "password" in value for value in result.values()))


if __name__ == "__main__":
    unittest.main()
