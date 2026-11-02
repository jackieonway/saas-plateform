"""Public routing and real raw-socket relays; no DB, namespace or Docker."""
import importlib.util
from pathlib import Path
import socket
import tempfile
import threading
import time
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location('phase25_gateway_test', Path(__file__).with_name('gateway.py'))
gateway = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gateway)


class PublicRouteTests(unittest.TestCase):
    def test_root_namespace_gateway_uses_root_task_guard_instead_of_task_user_mode(self):
        with tempfile.TemporaryDirectory(prefix='rust-saas-phase25-gateway-owner-') as directory:
            task = Path(directory)
            (task / 'phase25-business-manifest.json').write_text('{}')
            owner = SimpleNamespace(state={'prepared': True, 'mode': 'SOURCE', 'snapshot': 'phase25-tools-r4'},
                snapshot=Mock(), environment_binding=Mock(), cluster='1234567890123')
            business = {'prepared': True, 'toolsSnapshot': 'phase25-tools-r4', 'baseDatabase': 'owned_fixture',
                'deploymentId': 'public-deployment-fixture', 'clusterId': owner.cluster}
            owner.json = lambda name: (business if name.name == 'phase25-business-manifest.json'
                else {'ids': {'deployment': business['deploymentId']}} if name.name == 'https-environment.json'
                else {'database': business['baseDatabase']})
            task_constructor = Mock(return_value=owner)
            modules = [SimpleNamespace(Task=task_constructor), SimpleNamespace(validate_manifest=lambda _owner, value: value)]
            loader = SimpleNamespace(exec_module=Mock())
            with patch.object(gateway.importlib.util, 'spec_from_file_location', return_value=SimpleNamespace(loader=loader)), \
                    patch.object(gateway.importlib.util, 'module_from_spec', side_effect=modules):
                self.assertTrue(gateway.phase25_business_enabled(task, task / 'source-snapshot-phase25-tools-r4'))
            task_constructor.assert_called_once_with()
            owner.snapshot.assert_called_once_with(all_inputs=False)
            owner.environment_binding.assert_called_once_with({'ids': {'deployment': business['deploymentId']}}, 'SOURCE')
            owner.state['sourceQuarantined'] = True
            with patch.object(gateway.importlib.util, 'spec_from_file_location', return_value=SimpleNamespace(loader=loader)), \
                    patch.object(gateway.importlib.util, 'module_from_spec', return_value=modules[0]):
                with self.assertRaisesRegex(ValueError, 'SOURCE_BINDING_REQUIRED'):
                    gateway.phase25_business_enabled(task, task / 'source-snapshot-phase25-tools-r4')

    def test_only_current_public_console_routes_reach_fixed_business_ports(self):
        for path, port in [('/stores-api/stores', 23031), ('/work-orders-api/tickets', 23004), ('/socket.io/', 23003)]:
            self.assertEqual(gateway.public_business_target('GET', 'console.saas.test', path,
                'EIO=4&transport=polling', False, True), port)
            for host, private, enabled in [('auth.saas.test', False, True), ('customer.saas.test', False, True),
                                           ('console.saas.test', True, True), ('console.saas.test', False, False)]:
                self.assertIsNone(gateway.public_business_target('GET', host, path, '', private, enabled))
        for path in ['/internal/identity/metrics', '/api/internal/notifications/context-proof',
                     '/api/apps/stores/authorize', '/socket.io/../internal', '/socket.io-forged/',
                     '/stores-api/../../internal', '/stores-api/%2e%2e/internal', '/stores-api/%252e%252e/internal',
                     '/stores-api/\\internal', '/work-orders-api/%2finternal']:
            self.assertIsNone(gateway.public_business_target('GET', 'console.saas.test', path, '', False, True))

    def test_static_files_are_exact_current_bundle_files_without_spa_or_escape_fallback(self):
        with tempfile.TemporaryDirectory(prefix='rust-saas-phase25-gateway-static-') as directory:
            root = Path(directory)
            for app in ('stores', 'work-orders'):
                base = root / ('saas-' + app) / 'frontend/dist'
                base.mkdir(parents=True)
                (base / 'index.html').write_text('public fixture ' + app)
                (base / 'entry.js').write_text('public fixture module')
                self.assertEqual(gateway.business_static_target(root, '/sub/' + app + '/'), (base / 'index.html').resolve())
                self.assertEqual(gateway.business_static_target(root, '/sub/' + app + '/entry.js'), (base / 'entry.js').resolve())
            for path in ['/sub/other/index.html', '/sub/work-orders/not-found.js',
                         '/sub/work-orders/../index.html', '/sub/work-orders/%2e%2e/index.html',
                         '/sub/work-orders/\\outside', '/internal/private.json']:
                self.assertIsNone(gateway.business_static_target(root, path))


class WebSocketRelayTests(unittest.TestCase):
    def test_upgrade_header_has_total_deadline_even_when_peer_drips_bytes(self):
        left, right = socket.socketpair()
        stop = threading.Event()
        def drip():
            while not stop.wait(.02):
                try:
                    right.sendall(b'H')
                except OSError:
                    return
        writer = threading.Thread(target=drip)
        try:
            writer.start()
            started = time.monotonic()
            with self.assertRaises(TimeoutError):
                gateway.websocket_response(left, maximum_seconds=.06)
            self.assertLess(time.monotonic() - started, .5)
        finally:
            stop.set()
            left.close()
            right.close()
            writer.join(timeout=1)
            self.assertFalse(writer.is_alive())

    def test_upstream_101_headers_and_first_frame_remain_byte_exact(self):
        left, right = socket.socketpair()
        try:
            raw = (b'HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n'
                   b'Connection: Upgrade\r\nSec-WebSocket-Accept: actual-upstream-value\r\n\r\n')
            first_frame = b'\x81\x05hello'
            right.sendall(raw + first_frame)
            actual, status, headers = gateway.websocket_response(left)
            self.assertEqual(status, 101)
            self.assertEqual(actual, raw)
            self.assertEqual(headers['Sec-WebSocket-Accept'], 'actual-upstream-value')
            self.assertEqual(left.recv(1024), first_frame)
        finally:
            left.close()
            right.close()

    def test_invalid_upgrade_or_oversized_header_never_becomes_a_tunnel(self):
        for raw in [b'HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\n\r\n',
                    b'NOTHTTP 101 Wrong\r\n\r\n', b'X' * 65537]:
            left, right = socket.socketpair()
            try:
                left.settimeout(1)
                writer = threading.Thread(target=lambda: (right.sendall(raw), right.shutdown(socket.SHUT_WR)))
                writer.start()
                with self.assertRaises(ValueError):
                    gateway.websocket_response(left)
                writer.join(timeout=2)
                self.assertFalse(writer.is_alive())
            finally:
                left.close()
                right.close()

    def test_bidirectional_frame_bytes_are_relayed_and_peer_close_ends_the_relay(self):
        client, inbound = socket.socketpair()
        outbound, service = socket.socketpair()
        failures = []
        def run():
            try:
                gateway.relay_websocket(inbound, outbound, maximum_seconds=2, idle_seconds=1)
            except BaseException as error:
                failures.append(type(error).__name__)
        worker = threading.Thread(target=run)
        try:
            client.settimeout(2)
            service.settimeout(2)
            worker.start()
            masked_client_frame = b'\x81\x85\x01\x02\x03\x04masked-data'
            service_frame = b'\x81\x06server'
            client.sendall(masked_client_frame)
            self.assertEqual(service.recv(1024), masked_client_frame)
            service.sendall(service_frame)
            self.assertEqual(client.recv(1024), service_frame)
            client.shutdown(socket.SHUT_WR)
            worker.join(timeout=2)
            self.assertFalse(worker.is_alive())
            self.assertEqual(failures, [])
        finally:
            for channel in (client, inbound, outbound, service):
                channel.close()
            worker.join(timeout=2)

    def test_idle_and_total_byte_budgets_terminate_without_forwarding_excess(self):
        for budget in ('idle', 'bytes'):
            client, inbound = socket.socketpair()
            outbound, service = socket.socketpair()
            worker = threading.Thread(target=lambda: gateway.relay_websocket(inbound, outbound,
                maximum_seconds=1, idle_seconds=.05, maximum_bytes=4))
            try:
                worker.start()
                if budget == 'bytes':
                    client.sendall(b'12345')
                worker.join(timeout=1.5)
                self.assertFalse(worker.is_alive())
                service.settimeout(.05)
                with self.assertRaises(TimeoutError):
                    service.recv(1024)
            finally:
                for channel in (client, inbound, outbound, service):
                    channel.close()
                worker.join(timeout=2)


if __name__ == '__main__':
    unittest.main()
