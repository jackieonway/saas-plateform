"""Canonical witness checks. No PostgreSQL, Docker or services are started.

Memory checks run on every platform. Real anchored-file tests require POSIX and
use only unittest's uniquely owned temporary directory, removed on every exit.
"""

import copy
import hashlib
import hmac
import os
from pathlib import Path
import shutil
import stat
import tempfile
import unittest
from unittest import mock

import inventory
import witness


TASK = "019aa31b-6112-7000-8000-000000000001"
EPOCH = "019aa31b-6112-7000-8000-000000000002"
SOURCE = "019aa31b611270008000000000000003"
KEY = bytes(range(1, 33))
REF = 'platform/users/["019aa31b-6112-7000-8000-000000000099"]/session_version'


def binding(checkpoint="a" * 64):
    return witness.make_binding(TASK, "7701548153213487841", checkpoint, {
        "platform": {"name": "unified_auth_https_" + SOURCE,
                     "marker": "identity-deployment/1/" + TASK},
        "portal": {"name": "unified_auth_portal_" + SOURCE,
                   "marker": "portal-deployment/1/" + TASK},
    })


def payload(previous=None, time=100):
    return witness.make_payload(inventory.empty_inventory(), EPOCH, time, {"business": "b" * 64}, previous)


def source_inventory():
    value = inventory.empty_inventory()
    value["databases"]["platform"]["platform_backchannel_logout_sources"] = [{
        "id": TASK, "issuer": "https://auth.saas.test", "client_id": EPOCH,
        "sid_digest": "a" * 64, "sub_digest": "b" * 64, "revoked_at": "2026-10-28T00:00:00+00:00",
    }]
    value["databases"]["portal"]["portal_backchannel_logout_receipts"] = [{
        "id": TASK, "issuer": "https://auth.saas.test", "client_id": EPOCH,
        "jti_digest": "c" * 64, "issued_at": "2026-10-28T00:00:00+00:00",
        "expires_at": "2026-10-28T00:02:00+00:00", "received_at": "2026-10-28T00:00:01+00:00",
    }]
    return value


def service_inventory(secret_label):
    value = inventory.empty_inventory()
    if secret_label is not None:
        value["databases"]["platform"]["identity_service_clients"] = [{
            "id": TASK, "secret_hash_sha256": inventory.digest(hashlib.sha256(secret_label).hexdigest()),
            "purpose": "AUTH_ADMIN", "status": "ENABLED", "created_at": "2026-10-28T00:00:00+00:00",
            "allowed_auth_clients": [EPOCH], "allowed_auth_resources": [],
        }]
    return value


def fixed_error(test, action, category=None):
    with test.assertRaises(witness.WitnessError) as caught:
        action()
    value = str(caught.exception)
    test.assertRegex(value, r"^WITNESS_[A-Z0-9_]+$")
    if category is not None:
        test.assertEqual(value, category)


class MemoryTests(unittest.TestCase):
    def test_canonical_v2_roundtrip_binds_whole_inventory(self):
        value = payload()
        sealed = witness.seal(KEY, binding(), value, 1)
        self.assertEqual(witness.verify(KEY, binding(), sealed)["document"]["payload"], value)
        self.assertEqual(sealed["document"]["format"], "saas-identity-recovery-witness/2")
        self.assertNotIn("account_highwater", value)
        self.assertEqual(set(value["inventory"]["databases"]), {"platform", "portal"})

    def test_wrong_key_or_changed_payload_fails_authentication(self):
        original = witness.seal(KEY, binding(), payload(), 1)
        fixed_error(self, lambda: witness.verify(bytes([42]) * 32, binding(), original),
                    "WITNESS_AUTHENTICATION_FAILED")
        altered = copy.deepcopy(original)
        altered["document"]["payload"]["business_markers"]["business"] = "c" * 64
        fixed_error(self, lambda: witness.verify(KEY, binding(), altered), "WITNESS_AUTHENTICATION_FAILED")

    def test_checkpoint_cluster_issuer_and_task_are_bound(self):
        sealed = witness.seal(KEY, binding(), payload(), 1)
        fixed_error(self, lambda: witness.verify(KEY, binding("d" * 64), sealed), "WITNESS_BINDING_MISMATCH")
        changed = binding()
        changed["cluster_id"] = "7701548153213487842"
        fixed_error(self, lambda: witness.verify(KEY, changed, sealed), "WITNESS_BINDING_MISMATCH")
        fixed_error(self, lambda: witness.make_binding(TASK, "7701548153213487841", "a" * 64,
                                                       binding()["databases"], issuer="https://evil.invalid"))
        mixed = binding()["databases"]
        mixed["portal"]["name"] = "unified_auth_portal_" + "f" * 32
        fixed_error(self, lambda: witness.make_binding(TASK, "7701548153213487841", "a" * 64, mixed),
                    "WITNESS_DATABASE_PAIR_MISMATCH")

    def test_v1_format_is_not_a_fallback(self):
        sealed = witness.seal(KEY, binding(), payload(), 1)
        sealed["document"]["format"] = "saas-identity-recovery-witness/1"
        sealed["tag"] = hmac.new(KEY, witness.DOMAIN + witness.canonical(sealed["document"]),
                                 hashlib.sha256).hexdigest()
        fixed_error(self, lambda: witness.verify(KEY, binding(), sealed), "WITNESS_BINDING_MISMATCH")

    def test_duplicate_json_and_extra_missing_safety_family_reject(self):
        fixed_error(self, lambda: witness._parse(b'{"a":1,"a":2}'))
        for operation in ("missing", "extra"):
            value = payload()
            tables = value["inventory"]["databases"]["platform"]
            if operation == "missing":
                del tables[next(iter(tables))]
            else:
                tables["unknown_auth_authority"] = []
            fixed_error(self, lambda: witness.seal(KEY, binding(), value, 1), "WITNESS_INVENTORY_INVALID")

    def test_versions_boolean_values_bad_keys_and_plain_error_text_reject(self):
        for version in (True, -1, 1 << 63):
            value = payload()
            value["highwater"] = {REF: version}
            fixed_error(self, lambda: witness.validate_payload(value), "WITNESS_INTEGER_INVALID")
        value = payload()
        value["highwater"] = {"secret-DO-NOT-ECHO": 1}
        fixed_error(self, lambda: witness.validate_payload(value), "WITNESS_HIGHWATER_REFERENCE_INVALID")
        fixed_error(self, lambda: witness.seal(b"secret-DO-NOT-ECHO", binding(), payload(), 1),
                    "WITNESS_KEY_INVALID")

    def test_floor_of_deleted_objects_is_retained(self):
        previous = payload()
        previous["highwater"] = {REF: 9}
        next_value = payload(previous, time=101)
        self.assertEqual(next_value["highwater"][REF], 9)
        fixed_error(self, lambda: witness._advance(previous, dict(next_value, highwater={})),
                    "WITNESS_HIGHWATER_REGRESSION")
        fixed_error(self, lambda: witness._advance(previous, dict(next_value, highwater={REF: 8})),
                    "WITNESS_HIGHWATER_REGRESSION")

    def test_epoch_and_capture_time_do_not_regress(self):
        previous = payload()
        fixed_error(self, lambda: payload(previous, time=99), "WITNESS_TIME_REGRESSION")
        fixed_error(self, lambda: witness.make_payload(inventory.empty_inventory(), TASK, 101, {}, previous),
                    "WITNESS_EPOCH_MISMATCH")

    def test_candidate_external_minimum_sequence_is_required(self):
        sealed = witness.seal(KEY, binding(), payload(), 1)
        fixed_error(self, lambda: witness.verify(KEY, binding(), sealed, minimum_sequence=2))

    def test_encoder_supports_typed_business_text_and_signed_sort_positions(self):
        value = {"sort_order": -20, "text": "line one\nline two\t中文"}
        self.assertEqual(witness._parse(witness.canonical(value)), value)
        fixed_error(self, lambda: witness.canonical({"unsafe": float("nan")}))

    def test_real_typed_rp_source_and_replay_facts_cannot_be_lost_or_rebound(self):
        source = source_inventory()
        previous = witness.make_payload(source, EPOCH, 100, {})
        for kind, table, field in (
                ("platform", "platform_backchannel_logout_sources", "sub_digest"),
                ("portal", "portal_backchannel_logout_receipts", "jti_digest")):
            changed = copy.deepcopy(source)
            changed["databases"][kind][table][0][field] = "d" * 64
            fixed_error(self, lambda: witness.make_payload(changed, EPOCH, 101, {}, previous),
                        "WITNESS_INVENTORY_INVALID")
            changed = copy.deepcopy(source)
            changed["databases"][kind][table].clear()
            fixed_error(self, lambda: witness.make_payload(changed, EPOCH, 101, {}, previous),
                        "WITNESS_INVENTORY_INVALID")

    def test_real_key_window_and_emergency_revocation_cannot_roll_back(self):
        value = inventory.empty_inventory()
        value["databases"]["platform"]["auth_signing_keys"] = [{
            "kid": "recovery-fixture", "n": "public-modulus-fixture", "e": "AQAB",
            "published_at": "2026-10-28T00:00:00+00:00", "active": False,
            "activated_at": "2026-10-28T00:00:01+00:00", "retired_at": None,
            "revoked_at": "2026-10-28T00:01:00+00:00", "last_signed_exp": "2026-10-28T00:05:00+00:00",
        }]
        previous = witness.make_payload(value, EPOCH, 100, {})
        for key, altered in (("revoked_at", None), ("last_signed_exp", "2026-10-28T00:04:00+00:00"),
                             ("n", "other-material")):
            changed = copy.deepcopy(value)
            changed["databases"]["platform"]["auth_signing_keys"][0][key] = altered
            fixed_error(self, lambda: witness.make_payload(changed, EPOCH, 101, {}, previous),
                        "WITNESS_INVENTORY_INVALID")


@unittest.skipUnless(os.name == "posix" and hasattr(os, "O_NOFOLLOW"), "POSIX anchored filesystem required")
class FileTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="canonical-witness-v2-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.directory = self.root / "witness"
        self.directory.mkdir(mode=0o700)
        self.store = witness.WitnessStore(self.directory, KEY, binding())

    def append(self, count=2):
        previous = None
        records = []
        for index in range(count):
            previous = payload(previous, time=100 + index)
            self.store.append(previous, expected_sequence=index)
            records.append(self.store.load_latest(index + 1))
        return records

    def test_chain_roundtrip_permissions_and_compare_exchange(self):
        records = self.append()
        self.assertEqual(self.store.load_latest(2), records[-1])
        for path in self.directory.iterdir():
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
            self.assertEqual(path.stat().st_nlink, 1)
        fixed_error(self, lambda: self.store.append(payload(time=103), expected_sequence=1),
                    "WITNESS_SEQUENCE_CONFLICT")

    def test_authenticated_history_retains_intermediate_secret_projection_after_delete(self):
        previous = None
        labels = (b"A" * 32, b"B" * 32, b"C" * 32, None)
        for index, label in enumerate(labels):
            previous = witness.make_payload(service_inventory(label), EPOCH, 100 + index, {}, previous)
            self.store.append(previous, expected_sequence=index)
        with mock.patch.object(self.store, "_locked", wraps=self.store._locked) as locked, \
             mock.patch.object(self.store, "_read", wraps=self.store._read) as reads:
            history = self.store.load_history(minimum_sequence=4)
        self.assertEqual(locked.call_count, 1)
        self.assertEqual(reads.call_count, 5)  # authenticated CURRENT and four records, once each
        self.assertEqual([entry["document"]["sequence"] for entry in history], [1, 2, 3, 4])
        rows = [entry["document"]["payload"]["inventory"]["databases"]["platform"]["identity_service_clients"]
                for entry in history]
        self.assertEqual([row[0]["secret_hash_sha256"] for row in rows[:3]],
                         [inventory.digest(hashlib.sha256(label).hexdigest()) for label in labels[:3]])
        self.assertEqual(rows[-1], [])
        self.assertEqual(self.store.load_latest(4), history[-1])
        for label in labels[:3]:
            self.assertNotIn(label.decode(), witness.canonical(history).decode())
        original_b = rows[1][0]["secret_hash_sha256"]
        rows[1][0]["secret_hash_sha256"] = "f" * 64
        reread = self.store.load_history(4)
        self.assertEqual(reread[1]["document"]["payload"]["inventory"]["databases"]["platform"]
                         ["identity_service_clients"][0]["secret_hash_sha256"], original_b)

    def test_history_rejects_complete_directory_rollback_with_external_floor(self):
        self.append(1)
        current_one = (self.directory / "CURRENT").read_bytes()
        self.store.append(payload(time=101), 1)
        (self.directory / "CURRENT").write_bytes(current_one)
        (self.directory / "witness-00000002.json").unlink()
        self.assertEqual(len(self.store.load_history(1)), 1)
        fixed_error(self, lambda: self.store.load_history(minimum_sequence=2))

    def test_history_never_returns_a_partially_verified_or_incomplete_chain(self):
        self.append()
        path = self.directory / "witness-00000002.json"
        raw = path.read_bytes()
        path.write_bytes(raw.replace(b'"captured_at_ms":101', b'"captured_at_ms":102'))
        fixed_error(self, lambda: self.store.load_history(2), "WITNESS_AUTHENTICATION_FAILED")
        path.write_bytes(raw)
        (self.directory / "witness-00000001.json").unlink()
        fixed_error(self, lambda: self.store.load_history(2), "WITNESS_CHAIN_FILES_MISMATCH")

    def test_authenticated_old_record_replay_and_old_current_reject(self):
        first = self.append(1)[0]
        old_current = (self.directory / "CURRENT").read_bytes()
        self.store.append(payload(first["document"]["payload"], time=101), 1)
        fixed_error(self, lambda: self.store.verify_candidate(first), "WITNESS_REPLAY_REJECTED")
        (self.directory / "CURRENT").write_bytes(old_current)
        fixed_error(self, lambda: self.store.load_latest(), "WITNESS_CHAIN_FILES_MISMATCH")

    def test_whole_directory_rollback_is_caught_by_independent_floor(self):
        first = self.append(1)[0]
        old_current = (self.directory / "CURRENT").read_bytes()
        self.store.append(payload(first["document"]["payload"], time=101), 1)
        (self.directory / "CURRENT").write_bytes(old_current)
        (self.directory / "witness-00000002.json").unlink()
        # Cryptography alone cannot distinguish this complete valid old set.
        self.assertEqual(self.store.load_latest(1), first)
        fixed_error(self, lambda: self.store.load_latest(minimum_sequence=2))

    def test_missing_record_or_orphan_is_not_automatically_healed(self):
        self.append()
        (self.directory / "witness-00000001.json").unlink()
        fixed_error(self, lambda: self.store.load_latest(), "WITNESS_CHAIN_FILES_MISMATCH")
        orphan = self.directory / "CURRENT-orphan.tmp"
        orphan.touch(mode=0o600)
        fixed_error(self, lambda: self.store.load_latest(), "WITNESS_UNEXPECTED_FILE")

    def test_current_and_record_modification_reject_without_payload_echo(self):
        self.append(1)
        path = self.directory / "witness-00000001.json"
        raw = path.read_bytes()
        path.write_bytes(raw.replace(b'"captured_at_ms":100', b'"captured_at_ms":101'))
        fixed_error(self, lambda: self.store.load_latest(), "WITNESS_AUTHENTICATION_FAILED")

    def test_highwater_regression_rejects_before_committed_record(self):
        first = payload()
        first["highwater"] = {REF: 8}
        self.store.append(first, 0)
        smaller = payload(time=101)
        fixed_error(self, lambda: self.store.append(smaller, 1), "WITNESS_HIGHWATER_REGRESSION")
        self.assertEqual(self.store.load_latest()["document"]["sequence"], 1)
        self.assertFalse((self.directory / "witness-00000002.json").exists())

    def test_signed_invalid_chain_transition_is_rechecked_on_full_load(self):
        first = payload()
        first["highwater"] = {REF: 8}
        self.store.append(first, 0)
        # Simulate a caller holding the MAC key signing an internally valid but
        # regressed second snapshot. A complete read still rejects the chain.
        with self.store._locked() as descriptor:
            old = self.store._latest(descriptor, 1)
            second = witness.seal(KEY, binding(), payload(time=101), 2, witness._sha(old))
            self.store._write(descriptor, "witness-00000002.json", second)
            head = {"format": witness.CURRENT_FORMAT, "binding": binding(), "sequence": 2,
                    "record_sha256": witness._sha(second)}
            (self.directory / "CURRENT").unlink()
            self.store._write(descriptor, "CURRENT", {"document": head,
                "tag": witness._tag(KEY, witness.CURRENT_DOMAIN, head)})
        fixed_error(self, lambda: self.store.load_latest(), "WITNESS_HIGHWATER_REGRESSION")

    def test_symlink_hardlink_and_public_modes_fail_closed(self):
        self.append(1)
        current = self.directory / "CURRENT"
        saved = self.root / "saved"
        shutil.copyfile(current, saved)
        saved.chmod(0o600)
        current.unlink()
        current.symlink_to(saved)
        fixed_error(self, lambda: self.store.load_latest(), "WITNESS_STORAGE_UNAVAILABLE")
        current.unlink()
        os.link(saved, current)
        fixed_error(self, lambda: self.store.load_latest(), "WITNESS_FILE_PERMISSIONS_INVALID")
        current.unlink()
        shutil.copyfile(saved, current)
        current.chmod(0o644)
        fixed_error(self, lambda: self.store.load_latest(), "WITNESS_FILE_PERMISSIONS_INVALID")
        current.chmod(0o600)
        self.directory.chmod(0o755)
        fixed_error(self, lambda: self.store.load_latest(), "WITNESS_DIRECTORY_PERMISSIONS_INVALID")

    def test_symlinked_ancestor_and_concurrent_writer_fail_closed(self):
        import fcntl
        self.append(1)
        link = self.root / "alias"
        link.symlink_to(self.directory, target_is_directory=True)
        alias = witness.WitnessStore(link, KEY, binding())
        fixed_error(self, lambda: alias.load_latest(), "WITNESS_STORAGE_UNAVAILABLE")
        descriptor = os.open(self.directory / "LOCK", os.O_RDWR)
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            fixed_error(self, lambda: self.store.load_latest(), "WITNESS_STORAGE_UNAVAILABLE")
        finally:
            os.close(descriptor)

    def test_bounded_storage_refuses_growth_without_new_record(self):
        self.append(1)
        with mock.patch.object(witness, "MAX_CHAIN_BYTES", 1):
            fixed_error(self, lambda: self.store.append(payload(time=101), 1), "WITNESS_CHAIN_BYTES_EXCEEDED")
        self.assertFalse((self.directory / "witness-00000002.json").exists())


if __name__ == "__main__":
    unittest.main()
