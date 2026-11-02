"""Pure gateway routing/provenance checks; no sockets, services or databases."""
import importlib.util
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import unittest


loader = importlib.util.spec_from_file_location(
    "phase23_gateway_instances", Path(__file__).with_name("gateway.py"))
gateway = importlib.util.module_from_spec(loader)
loader.loader.exec_module(gateway)


class GatewayInstancesTests(unittest.TestCase):
    def test_private_context_source_has_no_retired_route_alias(self):
        self.assertTrue(gateway.identity_private_allowed(
            "POST", "/internal/identity/auth/context-selection/source"))
        self.assertFalse(gateway.identity_private_allowed(
            "POST", "/internal/identity/context-selection/source"))

    def request(self, routing, **changes):
        value = {"method": "POST", "host": "auth.saas.test", "path": "/oauth/token",
                 "query": "", "private": False}
        value.update(changes)
        return routing.select(**value)

    def test_default_keeps_single_identity_upstream_and_no_evidence_marker(self):
        routing = gateway.IdentityTokenUpstreams()
        self.assertIsNone(self.request(routing))
        self.assertIsNone(self.request(routing))
        self.assertEqual(gateway.proxy_response_headers([
            ("Content-Type", "application/json"), ("X-Task-Identity-Port", "23442")], set()),
            [("Content-Type", "application/json")])

    def test_only_exact_public_token_route_consumes_the_alternating_selection(self):
        routing = gateway.IdentityTokenUpstreams((23441, 23442))
        invalid = ({"method": "GET"}, {"method": "HEAD"}, {"host": "console.saas.test"},
                   {"host": "customer.saas.test"}, {"host": "auth.saas.test:24444"},
                   {"path": "/oauth/token/"}, {"path": "/oauth/jwks"},
                   {"path": "/oidc/token"}, {"query": "grant_type=authorization_code"},
                   {"private": True})
        for value in invalid:
            with self.subTest(value=value):
                self.assertIsNone(self.request(routing, **value))
        self.assertEqual(self.request(routing), 23441)
        for value in invalid:
            self.assertIsNone(self.request(routing, **value))
        self.assertEqual(self.request(routing), 23442)
        self.assertEqual(self.request(routing), 23441)

    def test_concurrent_requests_reserve_one_alternating_slot_each(self):
        routing = gateway.IdentityTokenUpstreams((23441, 23442))
        with ThreadPoolExecutor(max_workers=8) as pool:
            ports = list(pool.map(lambda _value: self.request(routing), range(100)))
        self.assertEqual(ports.count(23441), 50)
        self.assertEqual(ports.count(23442), 50)
        self.assertEqual(self.request(routing), 23441)

    def test_upstream_selection_cannot_be_configured_outside_owned_identity_instances(self):
        for ports in ((), (23441,), (23442, 23441), (23441, 23441), (23441, 5432),
                      (True, 23442), ("23441", "23442"), "23441,23442", 23441):
            with self.subTest(ports=ports), self.assertRaisesRegex(ValueError, "Invalid owned identity token upstreams"):
                gateway.IdentityTokenUpstreams(ports)

    def test_request_and_upstream_cannot_forge_or_duplicate_response_marker(self):
        headers = [("X-Task-Identity-Port", "23442"), ("x-task-identity-port", "5432"),
                   ("X-Forwarded-Proto", "http"), ("Authorization", "private-fixture"),
                   ("Content-Type", "application/json"), ("Connection", "close")]
        request = gateway.proxy_request_headers(headers, {"connection"})
        self.assertEqual(request, {"Authorization": "private-fixture", "Content-Type": "application/json"})
        for actual in (23441, 23442):
            with self.subTest(actual=actual):
                response = gateway.proxy_response_headers(headers + [("Content-Length", "123")],
                                                           {"connection"}, actual)
                markers = [(key, value) for key, value in response if key.lower() == "x-task-identity-port"]
                self.assertEqual(markers, [("X-Task-Identity-Port", str(actual))])
                self.assertNotIn(("Content-Length", "123"), response)
        for invalid in (True, "23441", 5432):
            with self.assertRaisesRegex(ValueError, "Invalid owned identity response upstream"):
                gateway.proxy_response_headers([], set(), invalid)


if __name__ == "__main__":
    unittest.main()
