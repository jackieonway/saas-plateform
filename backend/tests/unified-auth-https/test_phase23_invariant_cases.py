"""Pure old-ID invariant/readonly boundaries; no PG, sockets, service or listener."""
import copy
import importlib.util
import io
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch
import contextlib

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[1] / "ops/identity-recovery"))
spec = importlib.util.spec_from_file_location("phase23_invariants", HERE / "phase23-invariant-cases.py")
cases = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cases)
import core
import inventory
import reconcile
import witness
from test_inventory import sample, ID, TIME
from test_reconcile import baseline, client, put, registration_fixture, EPOCH, OTHER


def fixture(key_profile="full"):
    before = baseline()
    before["databases"]["platform"]["auth_signing_keys"] = [
        sample("platform", "auth_signing_keys", kid="owned-https", active=False, retired_at=TIME),
        sample("platform", "auth_signing_keys", kid="phase23-b", active=False, revoked_at=TIME),
        sample("platform", "auth_signing_keys", kid="phase23-c", active=False),
        sample("platform", "auth_signing_keys", kid="phase23-d", active=True, activated_at=TIME)]
    if key_profile == "recovery":
        before["databases"]["platform"]["auth_signing_keys"] = [
            row for row in before["databases"]["platform"]["auth_signing_keys"]
            if row["kid"] != "phase23-b"]
        before["databases"]["platform"]["auth_signing_keys"][0]["retired_at"] = None
    put(before, "users", sample("platform", "users", is_super_admin=False))
    put(before, "memberships", sample("platform", "memberships"))
    put(before, "auth_clients", client())
    put(before, "auth_client_credentials", sample("platform", "auth_client_credentials", client_type="CONFIDENTIAL",
                                                  secret_digest_sha256="a" * 64))
    put(before, "auth_consents", sample("platform", "auth_consents"))
    put(before, "auth_grants", sample("platform", "auth_grants", subject_mode="MACHINE", context_mode="NONE", credential_version=1))
    put(before, "bff_sessions", sample("platform", "bff_sessions"))
    put(before, "console_browsers", sample("platform", "console_browsers"))
    put(before, "portal_sessions", sample("portal", "portal_sessions"), "portal")
    latest = copy.deepcopy(before)
    next(row for row in latest["databases"]["platform"]["auth_signing_keys"]
         if row["kid"] == "phase23-c")["revoked_at"] = TIME
    first = witness.make_payload(before, EPOCH, 1, {})
    last = witness.make_payload(latest, EPOCH, 2, {}, previous=first)
    history = [{"document": {"payload": first}}, {"document": {"payload": last}}]
    document = {"payload": last}
    requirements = reconcile.plan(before, latest, EPOCH)["requirements"]
    receipt = {"requirements": requirements, "rearmed": {"users": [], "clients": [], "memberships": [], "services": {}}}
    target = copy.deepcopy(latest)
    platform = target["databases"]["platform"]
    platform["users"][0].update(status="DISABLED", session_version=5, password_hash_sha256="b" * 64)
    platform["memberships"][0].update(status="DISABLED", identity_version=4)
    platform["auth_clients"][0].update(policy_version=3, version=3)
    platform["auth_client_credentials"][0]["revoked_at"] = TIME
    platform["auth_consents"][0].update(status="REVOKED", revoked_at=TIME, generation=2, version=2)
    platform["auth_grants"][0].update(revoked_at=TIME, version=2)
    platform["bff_sessions"][0].update(revoked_at=TIME, refresh_uncertain=True, version=2)
    platform["console_browsers"][0]["generation"] = 2
    target["databases"]["portal"]["portal_sessions"][0].update(revoked_at=TIME, refresh_uncertain=True, version=2)
    return {"inventory": before}, document, history, target, receipt


def with_registration(selected_client=True, selected_consumer=True):
    saved, document, _, target, receipt = fixture()
    original, _ = registration_fixture()
    for value in (saved["inventory"], document["payload"]["inventory"], target):
        tables = value["databases"]["platform"]
        tables["auth_clients"][0].update(context_mode="PLATFORM_CONTEXT", allowed_member_types=["EMPLOYEE"])
        for table in ("applications", "context_selection_services", "context_selection_registrations"):
            tables[table] = copy.deepcopy(original["databases"]["platform"][table])
    before, latest = saved["inventory"], document["payload"]["inventory"]
    receipt["requirements"] = reconcile.plan(before, latest, EPOCH)["requirements"]
    receipt["rearmed"]["clients"] = [ID] if selected_client else []
    receipt["rearmed"]["services"] = {"context_selection_services": [OTHER]} if selected_consumer else {}
    platform = target["databases"]["platform"]
    floor = receipt["requirements"]["client_floors"][ID]
    platform["auth_clients"][0]["policy_version"] = floor
    platform["context_selection_services"][0].update(status="ENABLED" if selected_consumer else "DISABLED",
                                                    secret_hash_sha256="c" * 64)
    platform["context_selection_registrations"][0].update(enabled=selected_client and selected_consumer,
        client_policy_version=floor if selected_client and selected_consumer else 1)
    if selected_client:
        platform["auth_client_credentials"].append(sample("platform", "auth_client_credentials", id=OTHER,
            client_type="CONFIDENTIAL", credential_version=2, secret_digest_sha256="c" * 64))
    first = witness.make_payload(before, EPOCH, 1, {})
    last = witness.make_payload(latest, EPOCH, 2, {}, previous=first)
    history = [{"document": {"payload": first}}, {"document": {"payload": last}}]
    return saved, {"payload": last}, history, target, receipt


class InvariantTests(unittest.TestCase):
    def test_exact_registration_rearm_preserves_all_other_old_source_invariants(self):
        for client_selected, consumer_selected in ((True, True), (True, False), (False, True), (False, False)):
            values = with_registration(client_selected, consumer_selected)
            self.assertGreater(cases.evaluate(*values)["policyRows"], 0)

    def test_registration_rearm_cannot_use_old_extra_or_unselected_generation_or_binding(self):
        for field, value in (("client_policy_version", 1), ("client_policy_version", 3),
                             ("member_type", "CUSTOMER"), ("application_policy_version", 2),
                             ("consumer_service_id", None)):
            values = with_registration()
            values[3]["databases"]["platform"]["context_selection_registrations"][0][field] = value
            with self.assertRaisesRegex(core.RecoveryError, "REGISTRATION_BINDING_OR_GENERATION_CHANGED"):
                cases.evaluate(*values)
        for selected in ((False, True), (True, False)):
            values = with_registration(*selected)
            values[3]["databases"]["platform"]["context_selection_registrations"][0].update(enabled=True,
                client_policy_version=values[4]["requirements"]["client_floors"][ID])
            with self.assertRaisesRegex(core.RecoveryError, "REGISTRATION_BINDING_OR_GENERATION_CHANGED"):
                cases.evaluate(*values)
        values = with_registration()
        values[3]["databases"]["platform"]["auth_clients"][0]["policy_version"] += 1
        with self.assertRaisesRegex(core.RecoveryError, "REGISTRATION_BINDING_OR_GENERATION_CHANGED"):
            cases.evaluate(*values)
        values = with_registration()
        values[3]["databases"]["platform"]["context_selection_registrations"].append(dict(
            values[3]["databases"]["platform"]["context_selection_registrations"][0], client_id=OTHER))
        with self.assertRaisesRegex(core.RecoveryError, "REGISTRATION_BINDING_OR_GENERATION_CHANGED"):
            cases.evaluate(*values)

    def test_recovery_profile_preserves_all_old_invariants_without_claiming_a_b_lifecycle(self):
        values = fixture("recovery")
        self.assertGreater(cases.evaluate(*values, validation_profile="recovery")["oldCarrierRows"], 0)
        with self.assertRaisesRegex(core.RecoveryError, "KEY_FACT_MISSING"):
            cases.evaluate(*values)
        values[3]["databases"]["platform"]["auth_grants"][0]["revoked_at"] = None
        with self.assertRaisesRegex(core.RecoveryError, "OLD_CARRIER_LIVE"):
            cases.evaluate(*values, validation_profile="recovery")

    def test_recovery_profile_cannot_revive_c_or_change_unexercised_a_facts(self):
        for kid, changes, code in (("phase23-c", {"revoked_at": None}, "KEY_RECOVERY_PROFILE_REGRESSED"),
                                   ("owned-https", {"retired_at": TIME}, "KEY_UNEXERCISED_FACT_CHANGED")):
            values = fixture("recovery")
            next(row for row in values[3]["databases"]["platform"]["auth_signing_keys"]
                 if row["kid"] == kid).update(changes)
            with self.assertRaisesRegex(core.RecoveryError, code):
                cases.evaluate(*values, validation_profile="recovery")
        with self.assertRaisesRegex(core.RecoveryError, "VALIDATION_PROFILE_INVALID"):
            cases.evaluate(*fixture(), validation_profile="skip")

    def test_closed_old_rows_pass_and_new_live_carrier_does_not_create_global_empty_assertion(self):
        values = fixture()
        result = cases.evaluate(*values)
        self.assertGreater(result["oldCarrierRows"], 0)
        fresh = sample("platform", "auth_grants", id=OTHER, subject_mode="MACHINE", context_mode="NONE", credential_version=1)
        values[3]["databases"]["platform"]["auth_grants"].append(fresh)
        self.assertEqual(cases.evaluate(*values), result)

    def test_old_grant_and_bff_unknown_source_cannot_become_live(self):
        for table, change, code in (("auth_grants", {"revoked_at": None}, "OLD_CARRIER_LIVE"),
                                    ("bff_sessions", {"refresh_uncertain": False}, "OLD_BFF_LIVE"),
                                    ("bff_sessions", {"selected_session_id": OTHER}, "OLD_BFF_POINTER_LIVE")):
            values = fixture()
            values[3]["databases"]["platform"][table][0].update(change)
            with self.assertRaisesRegex(core.RecoveryError, code):
                cases.evaluate(*values)

    def test_authenticated_floor_and_nonselected_member_user_cannot_regress(self):
        for table, change, code in (("users", {"session_version": 0}, "HISTORICAL_VERSION_REGRESSED"),
                                    ("memberships", {"status": "ENABLED"}, "PRINCIPAL_QUARANTINE_CHANGED"),
                                    ("users", {"status": "ENABLED"}, "PRINCIPAL_QUARANTINE_CHANGED")):
            values = fixture()
            values[3]["databases"]["platform"][table][0].update(change)
            with self.assertRaisesRegex((core.RecoveryError, inventory.InventoryError), code):
                cases.evaluate(*values)

    def test_hidden_consent_and_key_revocation_are_not_restored(self):
        values = fixture()
        values[3]["databases"]["platform"]["auth_consents"][0].update(status="ACTIVE", revoked_at=None)
        with self.assertRaisesRegex(core.RecoveryError, "OLD_CONSENT_REVIVED"):
            cases.evaluate(*values)
        values = fixture()
        values[3]["databases"]["platform"]["auth_signing_keys"][2]["revoked_at"] = None
        with self.assertRaisesRegex(core.RecoveryError, "KEY_LIFECYCLE_REGRESSED"):
            cases.evaluate(*values)

    def test_immutable_invitation_receipt_disappearance_is_rejected(self):
        values = fixture()
        row = sample("platform", "auth_invitation_activation_receipts")
        values[1]["payload"]["inventory"]["databases"]["platform"]["auth_invitation_activation_receipts"] = [row]
        with self.assertRaisesRegex(core.RecoveryError, "IMMUTABLE_FACT_CHANGED_OR_MISSING"):
            cases.evaluate(*values)

    def test_selected_client_cannot_reuse_an_old_secret_or_a_deleted_history_generation(self):
        values = fixture()
        values[4]["rearmed"]["clients"] = [ID]
        values[4]["requirements"]["credential_floors"][ID] = 17
        values[3]["databases"]["platform"]["auth_client_credentials"].append(sample(
            "platform", "auth_client_credentials", id=OTHER, client_type="CONFIDENTIAL", credential_version=2,
            secret_digest_sha256="b" * 64))
        with self.assertRaisesRegex(core.RecoveryError, "FRESH_CLIENT_CREDENTIAL_MISSING"):
            cases.evaluate(*values)
        values[3]["databases"]["platform"]["auth_client_credentials"][-1]["credential_version"] = 18
        cases.evaluate(*values)
        values[3]["databases"]["platform"]["auth_client_credentials"][-1]["secret_digest_sha256"] = "a" * 64
        with self.assertRaisesRegex(core.RecoveryError, "FRESH_CLIENT_CREDENTIAL_MISSING"):
            cases.evaluate(*values)

    def test_every_pg_query_has_server_enforced_readonly_transaction_and_default(self):
        config = {"pgBin": "/usr/bin", "workDirectory": Path("/tmp/unused"), "adminConnectionFile": Path("/tmp/unused")}
        connection = {"PGHOST": "127.0.0.1", "PGPORT": "55494", "PGUSER": "postgres", "PGDATABASE": "postgres"}
        with patch.object(core, "private_read", return_value=json.dumps(connection).encode()), \
             patch.object(core, "execute", return_value=b"t") as execute:
            pg = cases.ReadOnlyPg(config, "owned_target")
            self.assertEqual(pg.query("SELECT true;"), "t")
        self.assertIn("default_transaction_read_only=on", execute.call_args.args[1]["PGOPTIONS"])
        sql = execute.call_args.args[2].decode()
        self.assertTrue(sql.startswith("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;"))
        self.assertTrue(sql.endswith("ROLLBACK;"))
        original = core.Pg
        with cases.readonly_queries():
            self.assertIs(core.Pg, cases.ReadOnlyPg)
        self.assertIs(core.Pg, original)

    def test_private_gate_refuses_cross_floor_before_returning_original_ids(self):
        config = {"workDirectory": Path("/owned"), "databases": {"platform": {"target": "p"}, "portal": {"target": "q"}}}
        saved = {"inventory": {}}
        floor = {"sequence": 4}
        start = {"checkpointSha256": core.sha(saved), "floorSha256": "0" * 64, "sequence": 4,
                 "targets": {"platform": "p", "portal": "q"}, "rearmed": {}}
        with patch.object(core, "authenticated_read", return_value=start):
            with self.assertRaisesRegex(core.RecoveryError, "REARM_RECEIPT_BINDING_INVALID"):
                cases.authenticated_start(config, {"mac": b"x" * 32}, saved, floor)

    def test_private_gate_keeps_deleted_member_and_credential_floors_without_any_pg_connection(self):
        config = {"workDirectory": Path("/owned"), "databases": {"platform": {"target": "p"}, "portal": {"target": "q"}}}
        saved = {"inventory": {"databases": {"platform": {"auth_clients": [{"id": ID}]}}}}
        floor = {"sequence": 4}
        selected = {"users": [], "clients": [], "memberships": [], "services": {}}
        requirements = {"eligible_users": [], "eligible_clients": [], "eligible_memberships": [],
                        "user_floors": {ID: 23}, "membership_floors": {OTHER: 31},
                        "credential_floors": {ID: 17}, "client_floors": {ID: 24}}
        start = {"checkpointSha256": core.sha(saved), "floorSha256": core.sha(floor), "sequence": 4,
                 "targets": {"platform": "p", "portal": "q"}, "rearmed": selected, "requirements": requirements}
        with patch.object(core, "load_config", return_value=config), patch.object(core, "keys", return_value={"mac": b"x" * 32}), \
             patch.object(core, "checkpoint", return_value=saved), patch.object(core, "latest_witness", return_value=({}, floor)), \
             patch.object(core, "historical_credentials", return_value=(set(), {ID: 17})), \
             patch.object(core, "authenticated_read", return_value=start), patch.object(core, "Pg") as pg:
            private = cases.load_recovery_gate("/owned/config", 4)
        pg.assert_not_called()
        self.assertEqual(private["credentialFloors"][ID], 17)
        self.assertEqual(private["membershipFloors"][OTHER], 31)
        self.assertEqual(private["userFloors"][ID], 23)
        self.assertEqual(private["rearmed"]["memberships"], [])

    def test_authenticated_start_cannot_understate_a_deleted_history_credential(self):
        values = fixture()
        deleted = sample("platform", "auth_client_credentials", id=OTHER, client_type="CONFIDENTIAL", credential_version=17)
        middle = copy.deepcopy(values[0]["inventory"])
        middle["databases"]["platform"]["auth_client_credentials"].append(deleted)
        values[2].insert(1, {"document": {"payload": {"inventory": middle}}})
        with self.assertRaisesRegex(core.RecoveryError, "AUTHENTICATED_CREDENTIAL_FLOOR_STALE"):
            cases.evaluate(*values)

    def test_selected_user_superadmin_or_client_redirect_drift_is_not_a_valid_recovery(self):
        values = fixture()
        values[4]["rearmed"]["users"] = [ID]
        values[3]["databases"]["platform"]["users"][0].update(status="ENABLED", is_super_admin=True)
        with self.assertRaisesRegex(core.RecoveryError, "PRINCIPAL_AUTHORITY_PAYLOAD_CHANGED"):
            cases.evaluate(*values)
        values = fixture()
        values[3]["databases"]["platform"]["auth_clients"][0]["redirect_uris"] = ["https://changed.test/callback"]
        with self.assertRaisesRegex(core.RecoveryError, "CLIENT_AUTHORITY_PAYLOAD_CHANGED"):
            cases.evaluate(*values)

    def test_normal_old_primary_negative_can_gc_consumed_challenges(self):
        values = fixture()
        challenge = sample("platform", "sso_login_challenges", consumed_at=TIME)
        values[0]["inventory"]["databases"]["platform"]["sso_login_challenges"] = [challenge]
        for envelope in values[2]:
            envelope["document"]["payload"]["inventory"]["databases"]["platform"]["sso_login_challenges"] = [challenge]
        values[3]["databases"]["platform"]["sso_login_challenges"] = [sample("platform", "sso_login_challenges", id=OTHER)]
        self.assertEqual(cases.evaluate(*values)["garbageCollectedRows"], 1)

    def test_mid_history_bff_deleted_from_latest_uses_checkpoint_original_operation(self):
        values = fixture()
        middle = copy.deepcopy(values[0]["inventory"])
        middle["databases"]["platform"]["bff_sessions"][0].update(logout_request_id=OTHER, logout_scope="GLOBAL", logout_confirmed_at=TIME)
        last = copy.deepcopy(values[1]["payload"]["inventory"])
        last["databases"]["platform"]["bff_sessions"] = []
        values[1]["payload"]["inventory"] = last
        values[2].insert(1, {"document": {"payload": {"inventory": middle}}})
        values[2][-1]["document"]["payload"]["inventory"] = last
        cases.evaluate(*values)

    def test_revoked_invitation_and_retired_subject_facts_do_not_regain_authority(self):
        values = fixture()
        invitation = sample("platform", "auth_invitations", status="ACTIVE", purpose="ACCOUNT_ONBOARDING")
        for snapshot in (values[0]["inventory"], values[1]["payload"]["inventory"], values[3]):
            snapshot["databases"]["platform"]["auth_invitations"] = [copy.deepcopy(invitation)]
        with self.assertRaisesRegex(core.RecoveryError, "OLD_INVITATION_LIVE"):
            cases.evaluate(*values)
        values = fixture()
        link = sample("platform", "auth_subject_links", retired_at=TIME)
        values[1]["payload"]["inventory"]["databases"]["platform"]["auth_subject_links"] = [link]
        values[3]["databases"]["platform"]["auth_subject_links"] = [{**link, "retired_at": None}]
        with self.assertRaisesRegex(core.RecoveryError, "SUBJECT_DENIAL_REGRESSED"):
            cases.evaluate(*values)

    def test_cli_never_prints_database_exception_or_private_request(self):
        canary = "PRIVATE-PASSWORD-PHC-SQL-ORIGINAL-ID"
        output = io.StringIO()
        with patch.object(cases, "run", side_effect=RuntimeError(canary)), contextlib.redirect_stderr(output):
            status = cases.main(["--config", "/owned/c", "--source-env", "/owned/s", "--target-env", "/owned/t",
                                 "--output", "/owned/o", "--minimum-sequence", "2"])
        self.assertEqual(status, 2)
        self.assertNotIn(canary, output.getvalue())
        self.assertEqual(json.loads(output.getvalue())["code"], "RECOVERY_INVARIANT_REJECTED")


if __name__ == "__main__":
    unittest.main()
