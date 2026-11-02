"""Adversarial planner checks; no database, process, socket or container I/O."""
import copy
import unittest

import inventory as inv
import reconcile as rec
from test_inventory import sample, ID, TIME, LATER


EPOCH = "019a1ec0-0000-7000-8000-000000000099"
OTHER = "019a1ec0-0000-7000-8000-000000000002"


def baseline():
    value = inv.empty_inventory()
    value["databases"]["platform"]["auth_provider_settings"] = [sample(
        "platform", "auth_provider_settings", singleton=True)]
    value["databases"]["platform"]["identity_deployment_state"] = [sample(
        "platform", "identity_deployment_state", singleton=True, schema_version=1)]
    value["databases"]["platform"]["auth_signing_keys"] = [sample(
        "platform", "auth_signing_keys", kid="key-A", active=True, activated_at=TIME)]
    return value


def put(value, table, row, kind="platform"):
    value["databases"][kind][table].append(row)
    return row


def client():
    return sample("platform", "auth_clients", client_type="CONFIDENTIAL", runtime_kind="WEB_BFF",
                  subject_mode="USER", context_mode="ACCOUNT_ONLY", token_endpoint_auth_method="client_secret_basic",
                  grant_types=["authorization_code"], redirect_uris=["https://original.test/callback"],
                  logout_mode="SHORT_SESSION_ONLINE_CHECK")


def raw_sample(kind, table, **overrides):
    projected = sample(kind, table)
    result = {}
    for name, field in inv.TABLES[kind][table].fields.items():
        result[field.source] = (None if projected[name] is None else
                               {"safe": "original"} if field.pg_type == "jsonb" else
                               "original-private-fixture" if field.redacted else projected[name])
    result.update(overrides)
    return result


def registration_fixture():
    before = baseline()
    put(before, "auth_clients", dict(client(), context_mode="PLATFORM_CONTEXT", allowed_member_types=["EMPLOYEE"]))
    put(before, "applications", sample("platform", "applications", allowed_member_types=["EMPLOYEE"]))
    put(before, "context_selection_services", sample("platform", "context_selection_services", id=OTHER,
        purpose="CONTEXT_SELECTION_CONSUMER", allowed_auth_clients=[ID]))
    put(before, "context_selection_registrations", sample("platform", "context_selection_registrations",
        client_id=ID, client_registry="AUTH", member_type="EMPLOYEE", enabled=True,
        client_policy_version=1, application_id=ID, application_policy_version=1, consumer_service_id=OTHER))
    return before, copy.deepcopy(before)


class ReconcileTests(unittest.TestCase):
    def test_registrations_start_disabled_and_only_exact_selected_client_consumer_can_advance(self):
        before, latest = registration_fixture()
        result = rec.plan(before, latest, EPOCH)
        requirements = result["requirements"]
        self.assertIn('UPDATE public."context_selection_registrations" SET enabled=false', "\n".join(result["platform"]))
        binding = requirements["registration_bindings"][ID]
        self.assertEqual(binding, dict(before["databases"]["platform"]["context_selection_registrations"][0],
                                       client_policy_version=2))
        for clients, services, enabled in (
                ([], {}, False), ([ID], {}, False),
                ([], {"context_selection_services": [OTHER]}, False),
                ([ID], {"context_selection_services": [OTHER]}, True)):
            expected = rec.registration_targets(before["databases"]["platform"], latest["databases"]["platform"],
                                                requirements, clients, services)[(ID,)]
            self.assertEqual(expected["enabled"], enabled)
            self.assertEqual(expected["client_policy_version"], 2 if enabled else 1)

    def test_changed_disabled_stale_or_ineligible_registration_bindings_stay_quarantined(self):
        for field, value in (("enabled", False), ("client_registry", "OIDC"), ("member_type", "CUSTOMER"),
                             ("application_id", OTHER), ("application_policy_version", 2),
                             ("consumer_service_id", ID), ("client_policy_version", 2)):
            before, latest = registration_fixture()
            latest["databases"]["platform"]["context_selection_registrations"][0][field] = value
            self.assertEqual(rec.plan(before, latest, EPOCH)["requirements"]["registration_bindings"], {})
        for table, field, value in (("auth_clients", "policy_version", 2),
                                    ("applications", "access_policy_version", 2),
                                    ("applications", "status", "DISABLED"),
                                    ("context_selection_services", "purpose", "CONTEXT_SELECTION_AUTHORITY"),
                                    ("context_selection_services", "allowed_auth_clients", [])):
            before, _ = registration_fixture()
            before["databases"]["platform"][table][0][field] = value
            self.assertEqual(rec.plan(before, copy.deepcopy(before), EPOCH)["requirements"]["registration_bindings"], {})

    def test_registration_verification_pins_full_tuple_exact_client_floor_and_no_extra_rows(self):
        before, latest = registration_fixture()
        sql = rec.verify_sql(before, latest, EPOCH, allowed_rearmed_clients=[ID],
            allowed_rearmed_services={"context_selection_services": [OTHER]})["platform"]
        self.assertIn('"client_policy_version" IS NOT DISTINCT FROM 2', sql)
        self.assertIn('"member_type" IS NOT DISTINCT FROM E\'EMPLOYEE\'', sql)
        self.assertIn('"enabled" IS NOT DISTINCT FROM true', sql)
        self.assertIn("AND status='ENABLED' AND policy_version=2)", sql)
        self.assertIn("NOT EXISTS(SELECT 1 FROM public.context_selection_registrations WHERE NOT(", sql)
        requirements = rec.plan(before, latest, EPOCH)["requirements"]
        requirements["registration_bindings"][ID]["client_policy_version"] = 3
        with self.assertRaisesRegex(rec.ReconcileError, "REGISTRATION_REQUIREMENTS_CHANGED"):
            rec.registration_targets(before["databases"]["platform"], latest["databases"]["platform"],
                                     requirements, [ID], {"context_selection_services": [OTHER]})

    def test_employee_registration_without_consumer_still_requires_explicit_client_selection(self):
        before, _ = registration_fixture()
        before["databases"]["platform"]["context_selection_registrations"][0]["consumer_service_id"] = None
        latest = copy.deepcopy(before)
        requirements = rec.plan(before, latest, EPOCH)["requirements"]
        for clients, enabled in (([], False), ([ID], True)):
            row = rec.registration_targets(before["databases"]["platform"], latest["databases"]["platform"],
                                           requirements, clients)[(ID,)]
            self.assertEqual(row["enabled"], enabled)

    def test_registration_cannot_rebind_behind_authenticated_application_highwater(self):
        before, latest = registration_fixture()
        floor = inv.highwater(before)
        reference = next(key for key in floor if "/applications/" in key and key.endswith("/access_policy_version"))
        floor[reference] = 9
        requirements = rec.plan(before, latest, EPOCH, external_highwater=floor)["requirements"]
        self.assertEqual(requirements["registration_application_floors"][ID], 9)
        self.assertEqual(requirements["registration_bindings"], {})
        target = rec.registration_targets(before["databases"]["platform"], latest["databases"]["platform"],
                                         requirements, [ID], {"context_selection_services": [OTHER]})[(ID,)]
        self.assertFalse(target["enabled"])
        self.assertEqual(target["application_policy_version"], 1)

    def reject(self, before, latest, category, **kwargs):
        with self.assertRaisesRegex(rec.ReconcileError, "^RECOVERY_" + category + "$"):
            rec.plan(before, latest, EPOCH, **kwargs)

    def test_canonical_all_old_carriers_are_explicitly_closed(self):
        before = baseline()
        result = rec.plan(before, copy.deepcopy(before), EPOCH)
        sql = "\n".join(result["platform"])
        for table in ("auth_grants", "auth_access_tokens", "auth_refresh_families", "auth_refresh_tokens",
                      "sso_sessions", "platform_sessions", "bff_sessions", "customer_portal_contexts"):
            self.assertIn('UPDATE public."' + table + '" SET revoked_at=', sql)
        self.assertIn("revoked=true", sql)
        self.assertIn("refresh_uncertain=true", sql)
        self.assertIn("DELETE FROM public.auth_totp_factors;", sql)
        self.assertNotIn("session_replication_role", sql)
        self.assertNotIn("DISABLE TRIGGER", sql)
        self.assertNotIn("oidc_grants", sql)
        portal = "\n".join(result["portal"])
        self.assertLess(portal.index('UPDATE public."portal_sessions" SET revoked_at'),
                        portal.index("membership_id=NULL"))
        for field in ("organization_id=NULL", "identity_version=NULL", "customer_ciphertext=NULL",
                      "customer_expires_at=NULL", "pending_selection_ciphertext=NULL"):
            self.assertIn(field, portal)

    def test_account_floor_precedes_disable_and_survives_status_trigger(self):
        before = baseline()
        put(before, "users", sample("platform", "users", session_version=2))
        latest = copy.deepcopy(before)
        latest["databases"]["platform"]["users"][0]["session_version"] = 19
        result = rec.plan(before, latest, EPOCH)
        self.assertEqual(result["requirements"]["user_floors"][ID], 22)
        self.assertIn(ID, result["requirements"]["eligible_users"])
        sql = "\n".join(result["platform"])
        self.assertLess(sql.index("session_version=21"), sql.index("password_hash=E'!recovery-disabled:"))

    def test_retired_account_is_never_eligible(self):
        before = baseline()
        put(before, "users", sample("platform", "users"))
        latest = copy.deepcopy(before)
        put(latest, "identity_account_tombstones", sample("platform", "identity_account_tombstones", retired_at=TIME))
        result = rec.plan(before, latest, EPOCH)
        self.assertNotIn(ID, result["requirements"]["eligible_users"])
        self.assertTrue(any('INSERT INTO public."identity_account_tombstones"' in sql for sql in result["platform"]))

    def test_unknown_refresh_and_browser_generation_do_not_roll_back(self):
        before = baseline()
        for kind, session, browser in (("platform", "bff_sessions", "console_browsers"),
                                        ("portal", "portal_sessions", "portal_browsers")):
            put(before, session, sample(kind, session, refresh_uncertain=True,
                                       refresh_lease_id=OTHER, refresh_started_at=TIME), kind)
            put(before, browser, sample(kind, browser, generation=3, current_session_id=ID), kind)
        latest = copy.deepcopy(before)
        for kind, session, browser in (("platform", "bff_sessions", "console_browsers"),
                                        ("portal", "portal_sessions", "portal_browsers")):
            latest["databases"][kind][browser][0]["generation"] = 17
            latest["databases"][kind][session][0]["version"] = 9
        result = rec.plan(before, latest, EPOCH)
        for kind in ("platform", "portal"):
            self.assertIn("generation=18", "\n".join(result[kind]))
            self.assertIn("version=10", "\n".join(result[kind]))
            self.assertIn("refresh_uncertain=true", "\n".join(result[kind]))

    def test_post_checkpoint_backchannel_sources_and_jtis_are_inserted_without_fk(self):
        before = baseline()
        latest = copy.deepcopy(before)
        for kind, prefix in (("platform", "platform"), ("portal", "portal")):
            for suffix in ("sources", "receipts"):
                table = prefix + "_backchannel_logout_" + suffix
                put(latest, table, sample(kind, table), kind)
        result = rec.plan(before, latest, EPOCH)
        for kind, prefix in (("platform", "platform"), ("portal", "portal")):
            sql = "\n".join(result[kind])
            self.assertIn('INSERT INTO public."' + prefix + '_backchannel_logout_sources"', sql)
            self.assertIn('INSERT INTO public."' + prefix + '_backchannel_logout_receipts"', sql)
            self.assertNotIn("DELETE FROM public." + prefix + "_backchannel", sql)

    def test_same_source_different_subject_refuses_merge(self):
        before = baseline()
        table = "platform_backchannel_logout_sources"
        put(before, table, sample("platform", table))
        latest = copy.deepcopy(before)
        latest["databases"]["platform"][table][0]["sub_digest"] = "d" * 64
        self.reject(before, latest, "BACKCHANNEL_SOURCE_COLLISION")

    def test_post_backup_key_revocation_and_window_are_preserved(self):
        before = baseline()
        put(before, "auth_signing_keys", sample("platform", "auth_signing_keys", kid="key-B"))
        latest = copy.deepcopy(before)
        keys = latest["databases"]["platform"]["auth_signing_keys"]
        keys[0].update(active=False, revoked_at=TIME, last_signed_exp=LATER)
        keys[1].update(active=True, activated_at=TIME)
        result = rec.plan(before, latest, EPOCH)
        self.assertEqual(result["requirements"]["active_kid"], "key-B")
        sql = "\n".join(result["platform"])
        self.assertIn("revoked_at=E'" + TIME, sql)
        self.assertIn("last_signed_exp=E'" + LATER, sql)
        self.assertIn("SET active=true WHERE kid=E'key-B'", sql)

    def test_no_safe_active_key_or_changed_public_material_is_closed(self):
        before = baseline()
        latest = copy.deepcopy(before)
        latest["databases"]["platform"]["auth_signing_keys"][0].update(active=False, revoked_at=TIME)
        self.reject(before, latest, "NO_SAFE_ACTIVE_KEY")
        latest = copy.deepcopy(before)
        latest["databases"]["platform"]["auth_signing_keys"][0]["n"] = "changed-public-key"
        self.reject(before, latest, "KEY_MATERIAL_CHANGED_OR_OMITTED")

    def test_missing_original_receipt_source_cannot_be_manufactured(self):
        before = baseline()
        latest = copy.deepcopy(before)
        receipt = sample("platform", "auth_logout_command_receipts")
        put(latest, "auth_logout_command_receipts", receipt)
        self.reject(before, latest, "MISSING_ORIGINAL_SOURCE", raw_latest={"platform": {"auth_logout_command_receipts": []}})

    def test_new_consent_receipt_is_imported_before_old_carriers_are_revoked(self):
        before = baseline()
        put(before, "users", sample("platform", "users"))
        put(before, "auth_consents", sample("platform", "auth_consents"))
        latest = copy.deepcopy(before)
        raw = raw_sample("platform", "identity_consent_command_receipts", action="REVOKE")
        receipt = inv.project_row("platform", "identity_consent_command_receipts", raw)
        put(latest, "identity_consent_command_receipts", receipt)
        result = rec.plan(before, latest, EPOCH,
                          raw_latest={"platform": {"identity_consent_command_receipts": [raw]}})
        sql = "\n".join(result["platform"])
        self.assertLess(sql.index('INSERT INTO public."identity_consent_command_receipts"'),
                        sql.index('UPDATE public."auth_grants"'))
        self.assertNotIn("result_json_sha256", sql)

    def test_raw_receipt_must_match_authenticated_projection(self):
        before = baseline()
        put(before, "users", sample("platform", "users"))
        put(before, "auth_consents", sample("platform", "auth_consents"))
        latest = copy.deepcopy(before)
        raw = raw_sample("platform", "identity_consent_command_receipts", action="REVOKE")
        put(latest, "identity_consent_command_receipts", inv.project_row("platform", "identity_consent_command_receipts", raw))
        raw["result_json"] = {"safe": "tampered"}
        self.reject(before, latest, "ORIGINAL_FACT_DIGEST_MISMATCH",
                    raw_latest={"platform": {"identity_consent_command_receipts": [raw]}})

    def test_existing_immutable_receipt_cannot_be_updated(self):
        before = baseline()
        put(before, "identity_consent_command_receipts", sample("platform", "identity_consent_command_receipts", action="REVOKE"))
        latest = copy.deepcopy(before)
        latest["databases"]["platform"]["identity_consent_command_receipts"][0]["result_json_sha256"] = "b" * 64
        self.reject(before, latest, "IMMUTABLE_RECEIPT_CHANGED")

    def test_delivery_budget_and_terminal_tuple_cannot_regress(self):
        before = baseline()
        put(before, "auth_delivery_jobs", sample("platform", "auth_delivery_jobs", state="LEASED", attempt_count=1,
                                                  target_id=ID, target_version=1, grant_id=ID,
                                                  lease_id=OTHER, lease_until=LATER))
        latest = copy.deepcopy(before)
        latest["databases"]["platform"]["auth_delivery_jobs"][0].update(
            state="TERMINAL", attempt_count=3, terminal_at=TIME, last_error_code="FIXTURE_FAILED",
            lease_id=None, lease_until=None)
        result = rec.plan(before, latest, EPOCH)
        sql = "\n".join(result["platform"])
        self.assertIn('"attempt_count"=3', sql)
        self.assertIn("state IN('PENDING','LEASED')", sql)
        self.assertNotIn("attempt_count=0", sql)
        closed = copy.deepcopy(latest)
        reopened = copy.deepcopy(closed)
        reopened["databases"]["platform"]["auth_delivery_jobs"][0]["state"] = "PENDING"
        self.reject(closed, reopened, "DELIVERY_TERMINAL_CHANGED")

    def test_changed_client_policy_is_disabled_and_never_rearmed_implicitly(self):
        before = baseline()
        put(before, "auth_clients", client())
        latest = copy.deepcopy(before)
        latest["databases"]["platform"]["auth_clients"][0]["redirect_uris"] = ["https://different.test/callback"]
        result = rec.plan(before, latest, EPOCH)
        self.assertNotIn(ID, result["requirements"]["eligible_clients"])
        self.assertIn("status=E'DISABLED'", "\n".join(result["platform"]))

    def test_service_rotation_eligibility_uses_public_policy_and_disables_old_secret(self):
        before = baseline()
        put(before, "identity_service_clients", sample("platform", "identity_service_clients"))
        latest = copy.deepcopy(before)
        latest["databases"]["platform"]["identity_service_clients"][0]["secret_hash_sha256"] = "b" * 64
        result = rec.plan(before, latest, EPOCH)
        self.assertEqual(result["requirements"]["eligible_services"], {"identity_service_clients": [ID]})
        self.assertIn("status='DISABLED'", "\n".join(result["platform"]))

    def test_credential_version_floor_includes_new_postcheckpoint_versions(self):
        before = baseline()
        put(before, "auth_clients", client())
        latest = copy.deepcopy(before)
        put(latest, "auth_client_credentials", sample("platform", "auth_client_credentials",
                                                      client_type="CONFIDENTIAL", credential_version=17))
        result = rec.plan(before, latest, EPOCH)
        self.assertEqual(result["requirements"]["credential_floors"][ID], 17)

    def test_deny_only_set_intersection_does_not_insert_new_permissions(self):
        before = baseline()
        put(before, "auth_client_permissions", sample("platform", "auth_client_permissions"))
        latest = copy.deepcopy(before)
        latest["databases"]["platform"]["auth_client_permissions"] = []
        result = rec.plan(before, latest, EPOCH)
        self.assertIn('DELETE FROM public."auth_client_permissions"', "\n".join(result["platform"]))
        self.assertNotIn('INSERT INTO public."auth_client_permissions"', "\n".join(result["platform"]))

    def test_rearm_allowlist_is_checked_against_eligible_ids(self):
        value = baseline()
        with self.assertRaisesRegex(rec.ReconcileError, "^RECOVERY_REARM_NOT_ELIGIBLE$"):
            rec.verify_sql(value, value, EPOCH, allowed_rearmed_users=[OTHER])

    def test_sql_strings_are_escaped_without_shell_or_dollar_quoting(self):
        self.assertEqual(rec.literal("a'b\\c"), "E'a''b\\\\c'")
        self.assertIn("$(never-executed)", rec.literal("$(never-executed)"))
        self.assertEqual(rec.literal([], "_uuid"), "ARRAY[]::uuid[]")
        with self.assertRaises(rec.ReconcileError):
            rec.identifier("bad; DROP DATABASE other")

    def test_verification_checks_floor_old_sources_and_terminal_delivery(self):
        before = baseline()
        put(before, "users", sample("platform", "users"))
        put(before, "auth_clients", client())
        put(before, "auth_consents", sample("platform", "auth_consents"))
        put(before, "bff_sessions", sample("platform", "bff_sessions"))
        put(before, "console_browsers", sample("platform", "console_browsers"))
        put(before, "portal_sessions", sample("portal", "portal_sessions"), "portal")
        latest = copy.deepcopy(before)
        latest["databases"]["platform"]["users"][0]["session_version"] = 11
        latest["databases"]["platform"]["auth_consents"][0].update(generation=7, version=9, display_deleted_at=TIME,
                                                                    status="REVOKED", revoked_at=TIME)
        for kind, prefix in (("platform", "platform"), ("portal", "portal")):
            put(latest, prefix + "_backchannel_logout_sources", sample(kind, prefix + "_backchannel_logout_sources"), kind)
        result = rec.verify_sql(before, latest, EPOCH)
        sql = result["platform"]
        self.assertIn("session_version>=14", sql)
        self.assertIn("generation>=8", sql)
        self.assertIn("version>=10", sql)
        self.assertIn("display_deleted_at IS NOT NULL", sql)
        self.assertIn("selected_session_id IS NULL", sql)
        self.assertIn('"platform_backchannel_logout_sources"', sql)
        self.assertIn("state IN('PENDING','LEASED')", sql)
        self.assertIn("NOT EXISTS(SELECT 1 FROM auth_totp_factors)", sql)
        self.assertIn("pending_selection_ciphertext IS NULL", result["portal"])
        self.assertIn('"portal_backchannel_logout_sources"', result["portal"])

    def test_changed_target_is_denied_despite_database_allowing_reactivation(self):
        before = baseline()
        put(before, "auth_delivery_targets", sample("platform", "auth_delivery_targets", purpose="BACK_CHANNEL",
                                                     status="ACTIVE", version=1))
        latest = copy.deepcopy(before)
        latest["databases"]["platform"]["auth_delivery_targets"][0].update(status="REVOKED", version=17)
        result = rec.plan(before, latest, EPOCH)
        self.assertIn("status=E'REVOKED',version=18", "\n".join(result["platform"]))
        sql = rec.verify_sql(before, latest, EPOCH)["platform"]
        self.assertIn("status=E'REVOKED' AND version>=18", sql)

    def test_a_new_security_source_without_exact_replay_order_blocks(self):
        before = baseline()
        latest = copy.deepcopy(before)
        put(latest, "auth_security_notification_outbox", sample("platform", "auth_security_notification_outbox",
                                                               event_kind="MFA_ENABLED", delivery_state="PENDING"))
        self.reject(before, latest, "MISSING_SECURITY_OUTBOX_ORIGINAL")

    def test_three_snapshot_deleted_user_and_membership_use_external_history(self):
        before = baseline()
        put(before, "users", sample("platform", "users", session_version=1))
        put(before, "memberships", sample("platform", "memberships", identity_version=1))
        middle = copy.deepcopy(before)
        middle["databases"]["platform"]["users"][0]["session_version"] = 20
        middle["databases"]["platform"]["memberships"][0]["identity_version"] = 30
        floor = inv.highwater(middle)
        latest = copy.deepcopy(middle)
        latest["databases"]["platform"]["users"] = []
        latest["databases"]["platform"]["memberships"] = []
        result = rec.plan(before, latest, EPOCH, external_highwater=floor)
        self.assertEqual(result["requirements"]["user_floors"][ID], 23)
        self.assertEqual(result["requirements"]["membership_floors"][ID], 31)
        self.assertEqual(result["requirements"]["eligible_users"], [])
        self.assertEqual(result["requirements"]["eligible_memberships"], [])
        query = rec.verify_sql(before, latest, EPOCH, external_highwater=floor)["platform"]
        self.assertIn("session_version>=23", query)
        self.assertIn("identity_version>=31", query)
        self.assertIn("status=E'DISABLED'", query)

    def test_deleted_browser_consent_client_and_credential_keep_external_floors(self):
        before = baseline()
        put(before, "console_browsers", sample("platform", "console_browsers", generation=1))
        put(before, "auth_consents", sample("platform", "auth_consents"))
        put(before, "auth_clients", client())
        put(before, "auth_client_credentials", sample("platform", "auth_client_credentials", client_type="CONFIDENTIAL"))
        middle = copy.deepcopy(before)
        middle["databases"]["platform"]["console_browsers"][0]["generation"] = 20
        middle["databases"]["platform"]["auth_consents"][0].update(generation=21, version=22, group_version=23)
        middle["databases"]["platform"]["auth_clients"][0].update(policy_version=24, version=25)
        middle["databases"]["platform"]["auth_client_credentials"][0]["credential_version"] = 26
        floor = inv.highwater(middle)
        latest = copy.deepcopy(middle)
        for table in ("console_browsers", "auth_consents", "auth_clients", "auth_client_credentials"):
            latest["databases"]["platform"][table] = []
        result = rec.plan(before, latest, EPOCH, external_highwater=floor)
        self.assertEqual(result["requirements"]["consent_floors"][ID], 22)
        self.assertEqual(result["requirements"]["client_floors"][ID], 25)
        self.assertEqual(result["requirements"]["credential_floors"][ID], 26)
        self.assertIn("generation=21", "\n".join(result["platform"]))
        query = rec.verify_sql(before, latest, EPOCH, external_highwater=floor)["platform"]
        self.assertIn("generation>=21", query)
        self.assertIn("policy_version>=25", query)
        self.assertIn("group_version>=23", query)

    def test_external_key_window_and_deleted_job_attempt_budget_are_not_reset(self):
        before = baseline()
        put(before, "auth_delivery_jobs", sample("platform", "auth_delivery_jobs", state="PENDING", attempt_count=1))
        middle = copy.deepcopy(before)
        middle["databases"]["platform"]["auth_signing_keys"][0]["last_signed_exp"] = LATER
        middle["databases"]["platform"]["auth_delivery_jobs"][0]["attempt_count"] = 4
        floor = inv.highwater(middle)
        latest = copy.deepcopy(before)
        latest["databases"]["platform"]["auth_delivery_jobs"] = []
        result = rec.plan(before, latest, EPOCH, external_highwater=floor)
        sql = "\n".join(result["platform"])
        self.assertIn("attempt_count=4", sql)
        self.assertIn("last_signed_exp=E'" + LATER, sql)
        query = rec.verify_sql(before, latest, EPOCH, external_highwater=floor)["platform"]
        self.assertIn("last_signed_exp>=E'" + LATER, query)

    def test_external_highwater_is_typed_and_cannot_exhaust_or_inject(self):
        before = baseline()
        put(before, "users", sample("platform", "users"))
        floor = inv.highwater(before)
        reference = next(item for item in floor if item.endswith("/session_version"))
        floor[reference] = (1 << 63) - 2
        self.reject(before, before, "GENERATION_EXHAUSTED", external_highwater=floor)
        self.reject(before, before, "HIGHWATER_INVALID", external_highwater={"bad\nref": 1})

    def test_deleted_distinct_credential_id_keeps_full_history_client_floor(self):
        before = baseline()
        put(before, "auth_clients", client())
        put(before, "auth_client_credentials", sample("platform", "auth_client_credentials",
                                                      client_type="CONFIDENTIAL", credential_version=1))
        middle = copy.deepcopy(before)
        put(middle, "auth_client_credentials", sample("platform", "auth_client_credentials", id=OTHER,
                                                      client_type="CONFIDENTIAL", credential_version=17))
        latest = copy.deepcopy(before)
        # The authenticated history aggregator retains the original client_id
        # relationship; the generic per-row floor by itself cannot infer it.
        client_floors = {}
        for snapshot in (before, middle, latest):
            for row in snapshot["databases"]["platform"]["auth_client_credentials"]:
                client_floors[row["client_id"]] = max(client_floors.get(row["client_id"], 0), row["credential_version"])
        result = rec.plan(before, latest, EPOCH, external_highwater=inv.highwater(middle),
                          external_credential_floors=client_floors)
        self.assertEqual(result["requirements"]["credential_floors"][ID], 17)
        query = rec.verify_sql(before, latest, EPOCH, allowed_rearmed_clients=[ID],
                               external_highwater=inv.highwater(middle), external_credential_floors=client_floors)["platform"]
        self.assertIn("credential_version>17", query)
        self.assertIn("revoked_at IS NULL AND secret_digest IS NOT NULL", query)
        self.assertIn('"auth_client_credentials" WHERE "id" IS NOT DISTINCT FROM', query)
        self.assertIn("AND revoked_at IS NOT NULL", query)

    def test_client_without_checkpoint_credential_still_uses_deleted_history_id(self):
        before = baseline()
        put(before, "auth_clients", client())
        result = rec.plan(before, before, EPOCH, external_credential_floors={ID: 17})
        self.assertEqual(result["requirements"]["credential_floors"][ID], 17)
        query = rec.verify_sql(before, before, EPOCH, allowed_rearmed_clients=[ID],
                               external_credential_floors={ID: 17})["platform"]
        self.assertIn("credential_version>17", query)

    def test_credential_history_is_typed_does_not_rearm_unknown_and_never_lowers(self):
        before = baseline()
        put(before, "auth_clients", client())
        put(before, "auth_client_credentials", sample("platform", "auth_client_credentials",
                                                      client_type="CONFIDENTIAL", credential_version=5))
        for invalid in ({"not-a-uuid": 17}, {ID: True}, {ID: -1}, {ID: 1.5}, {ID: 1 << 63}):
            self.reject(before, before, "CREDENTIAL_HISTORY_INVALID", external_credential_floors=invalid)
        self.reject(before, before, "CREDENTIAL_HISTORY_REGRESSION", external_credential_floors={ID: 4})
        result = rec.plan(before, before, EPOCH, external_credential_floors={ID: 17, OTHER: 999})
        self.assertEqual(result["requirements"]["credential_floors"], {ID: 17})
        self.assertNotIn(OTHER, result["requirements"]["eligible_clients"])
        with self.assertRaisesRegex(rec.ReconcileError, "^RECOVERY_REARM_NOT_ELIGIBLE$"):
            rec.verify_sql(before, before, EPOCH, allowed_rearmed_clients=[OTHER],
                           external_credential_floors={ID: 17, OTHER: 999})


if __name__ == "__main__":
    unittest.main()
