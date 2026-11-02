"""Pure service dispatch checks; no processes, sockets or database resources."""
import copy
import hashlib
import importlib.util
from pathlib import Path
import unittest
import tempfile


loader = importlib.util.spec_from_file_location(
    "phase23_service_instances", Path(__file__).with_name("run-service.py"))
runner = importlib.util.module_from_spec(loader)
loader.loader.exec_module(runner)


class ServiceInstancesTests(unittest.TestCase):
    def setUp(self):
        self.task = Path("/tmp/rust-saas-unified-auth-instance-pure")
        self.configuration = {
            "identity": {"BIND_ADDR": "127.0.0.1:23441", "DATABASE_URL": "private-source",
                         "OIDC_KEYSET_PATH": "/owned/keys", "AUTH_DELIVERY_WORKER_ENABLED": "true"},
            "platform": {"BIND_ADDR": "127.0.0.1:23001", "DATABASE_URL": "private-platform"},
            "portal": {"BIND_ADDR": "127.0.0.1:23101", "DATABASE_URL": "private-portal"},
        }

    def test_peer_is_independent_process_with_same_authority_and_runtime(self):
        original = copy.deepcopy(self.configuration)
        primary = runner.service_instance(self.task, self.configuration, "identity")
        peer = runner.service_instance(self.task, self.configuration, "identity-peer")
        self.assertEqual(primary[0], peer[0])
        self.assertEqual(peer[0], self.task / "bin/saas-identity")
        self.assertEqual(primary[1]["BIND_ADDR"], "127.0.0.1:23441")
        self.assertEqual(peer[1]["BIND_ADDR"], "127.0.0.1:23442")
        self.assertEqual({key: value for key, value in primary[1].items() if key != "BIND_ADDR"},
                         {key: value for key, value in peer[1].items() if key != "BIND_ADDR"})
        self.assertEqual(primary[2], self.task / "service-identity.pid")
        self.assertEqual(peer[2], self.task / "service-identity-peer.pid")
        self.assertEqual(primary[3], self.task / "service-identity.log")
        self.assertEqual(peer[3], self.task / "service-identity-peer.log")
        self.assertEqual(self.configuration, original)
        peer[1]["DATABASE_URL"] = "changed-only-in-peer-copy"
        self.assertEqual(self.configuration, original)
        self.assertEqual(primary[1]["DATABASE_URL"], "private-source")

    def test_existing_services_keep_their_configurations_and_binary_mapping(self):
        for name, executable in (("identity", "saas-identity"),
                                 ("platform", "saas-platform-backend"),
                                 ("portal", "saas-customer-portal-backend")):
            with self.subTest(service=name):
                binary, environment, record, log = runner.service_instance(
                    self.task, self.configuration, name)
                self.assertEqual(binary, self.task / "bin" / executable)
                self.assertEqual(environment, {**self.configuration[name],
                                               "PATH": "/usr/lib/postgresql/18/bin:/usr/bin:/bin"})
                self.assertEqual(record, self.task / ("service-" + name + ".pid"))
                self.assertEqual(log, self.task / ("service-" + name + ".log"))

    def test_peer_never_uses_an_independent_configuration_or_input_bind(self):
        self.configuration["identity-peer"] = {"DATABASE_URL": "unrelated-private-source",
                                               "BIND_ADDR": "0.0.0.0:9999"}
        self.configuration["identity"]["BIND_ADDR"] = "0.0.0.0:1234"
        _, peer, _, _ = runner.service_instance(self.task, self.configuration, "identity-peer")
        self.assertEqual(peer["BIND_ADDR"], "127.0.0.1:23442")
        self.assertEqual(peer["DATABASE_URL"], "private-source")

    def test_invalid_instance_and_environment_reject_without_private_values(self):
        for name in ("identity-secondary", "../../outside", ""):
            with self.subTest(instance=name), self.assertRaisesRegex(
                    SystemExit, "^Invalid task service instance$"):
                runner.service_instance(self.task, self.configuration, name)
        for invalid in (None, [], "private-source", {"SECRET": b"private-source"}, {1: "private-source"}):
            for name in ("identity", "identity-peer"):
                with self.subTest(invalid_type=type(invalid).__name__, instance=name):
                    with self.assertRaisesRegex(SystemExit, "^Invalid private service environment$"):
                        runner.service_instance(self.task, {"identity": invalid}, name)

    def test_configured_keyset_digest_is_captured_from_original_bytes(self):
        with tempfile.TemporaryDirectory(prefix="phase23-instance-keyset-pure-") as temporary:
            keyset = Path(temporary) / "signing.json"
            original = b'{"fixture":"private-source"}\n'
            keyset.write_bytes(original)
            environment = {"OIDC_KEYSET_PATH": str(keyset)}
            first = runner.keyset_record(environment, "identity")
            self.assertEqual(first, runner.keyset_record(environment, "identity-peer"))
            self.assertEqual(first, {"keysetPath": str(keyset),
                                     "configuredKeysetSha256": hashlib.sha256(original).hexdigest()})
            keyset.write_bytes(b"changed")
            self.assertEqual(first["configuredKeysetSha256"], hashlib.sha256(original).hexdigest())
            self.assertNotEqual(first, runner.keyset_record(environment, "identity-peer"))
            self.assertNotIn("loadedKeysetSha256", first)
            for instance in ("platform", "portal"):
                self.assertEqual(runner.keyset_record(environment, instance), {})
            keyset.unlink()
            with self.assertRaisesRegex(SystemExit, "^Task signing keyset unavailable$"):
                runner.keyset_record(environment, "identity-peer")


if __name__ == "__main__":
    unittest.main()
