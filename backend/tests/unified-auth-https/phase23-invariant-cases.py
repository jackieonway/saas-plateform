"""Read-only, authenticated old-row invariants after the owned recovery.

No listener, writer, HBA, application start, traffic gate or import-time I/O.
Current applications may create new rows: only authenticated historical carrier
IDs are checked for closure. Runtime/catalog boundaries are checked separately.
Run after target readiness and old-carrier negatives, before any fresh positive
login, consent or MFA enrollment. Those later positive cases are a separate layer.
"""
import argparse
import base64
import contextlib
import json
from pathlib import Path
import re
import sys
from urllib.parse import unquote, urlsplit

OPS = Path(__file__).resolve().parents[2] / "ops/identity-recovery"
sys.path.insert(0, str(OPS))
import core
import inventory
import reconcile


BASE_PG = core.Pg
LEGACY = ("oidc_clients", "oauth_clients", "oidc_grants", "oidc_access_tokens",
          "oidc_refresh_tokens", "oidc_authorization_requests", "oidc_authorization_codes",
          "oauth_tokens", "oauth_authorization_requests", "oauth_authorization_codes", "oauth_device_codes")
REVOKED = {"platform": ("auth_grants", "auth_access_tokens", "auth_refresh_families",
    "auth_refresh_tokens", "sso_sessions", "platform_sessions", "customer_portal_contexts",
    "auth_client_credentials"), "portal": ()}
CONSUMED = {"platform": ("auth_authorization_codes", "auth_refresh_tokens", "sso_login_challenges",
    "console_login_flows", "platform_session_refresh_tokens", "auth_totp_recovery_codes"),
    "portal": ("portal_login_flows",)}
COLLECTIBLE = frozenset(("sso_login_challenges", "console_login_flows", "portal_login_flows"))


def require(value, code):
    core.need(value, "INVARIANT_" + code)


class ReadOnlyPg(BASE_PG):
    def __init__(self, config, database):
        super().__init__(config, database)
        self.env["PGOPTIONS"] += " -c default_transaction_read_only=on"

    def query(self, sql):
        # Server enforcement applies to every query, including reused core
        # guards and SECURITY DEFINER calls; connection defaults also deny writes.
        return super().query("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\n" +
                             sql.rstrip().rstrip(";") + ";\nROLLBACK;")


@contextlib.contextmanager
def readonly_queries():
    previous = core.Pg
    core.Pg = ReadOnlyPg
    try:
        yield
    finally:
        core.Pg = previous


def index(kind, table, rows):
    return {tuple(row[name] for name in inventory.TABLES[kind][table].pk): row for row in rows}


def historical_rows(history, kind, table):
    result = {}
    for envelope in history:
        result.update(index(kind, table, envelope["document"]["payload"]["inventory"]["databases"][kind][table]))
    return result


def evaluate(saved, latest, history, target, receipt, validation_profile="full"):
    require(validation_profile in ("full", "recovery"), "VALIDATION_PROFILE_INVALID")
    """Pure projection comparison. Failure codes never contain a row or ID."""
    inventory.validate_inventory(target)
    before = saved["inventory"]["databases"]
    current = latest["payload"]["inventory"]["databases"]
    requirements, selected = receipt["requirements"], receipt["rearmed"]
    counts = {"oldCarrierRows": 0, "absentPostCheckpointRows": 0, "versionFloors": 0,
              "immutableFacts": 0, "consentsClosed": 0, "deliveryRows": 0, "quarantinedPrincipals": 0,
              "garbageCollectedRows": 0, "policyRows": 0}

    def old(kind, table):
        initial = index(kind, table, before[kind][table])
        historical = historical_rows(history, kind, table)
        actual = index(kind, table, target["databases"][kind][table])
        require(table in COLLECTIBLE or initial.keys() <= actual.keys(), "CHECKPOINT_ROW_MISSING")
        for key, original in historical.items():
            if key not in actual:
                require(key not in initial or table in COLLECTIBLE, "CHECKPOINT_ROW_MISSING")
                counts["garbageCollectedRows" if key in initial else "absentPostCheckpointRows"] += 1
                continue
            counts["oldCarrierRows"] += 1
            yield original, actual[key]

    for kind in ("platform", "portal"):
        for table in REVOKED[kind]:
            for _, row in old(kind, table):
                require(row["revoked_at"] is not None, "OLD_CARRIER_LIVE")
        for table in CONSUMED[kind]:
            for _, row in old(kind, table):
                require(row["consumed_at"] is not None, "OLD_ONE_TIME_LIVE")
        session, browser = (("bff_sessions", "console_browsers") if kind == "platform"
                            else ("portal_sessions", "portal_browsers"))
        for original, row in old(kind, session):
            require(row["revoked_at"] is not None and row["refresh_uncertain"]
                    and row["refresh_lease_id"] is None and row["refresh_started_at"] is None,
                    "OLD_BFF_LIVE")
            fields = (("selected_session_id", "platform_refresh_ciphertext_sha256") if kind == "platform" else
                      ("membership_id", "organization_id", "identity_version", "customer_ciphertext_sha256",
                       "customer_expires_at", "pending_selection_ciphertext_sha256", "context_lease_id", "context_started_at"))
            require(all(row[field] is None for field in fields), "OLD_BFF_POINTER_LIVE")
            reference = (index(kind, session, current[kind][session]).get(tuple(row[field] for field in
                         inventory.TABLES[kind][session].pk)) or index(kind, session, before[kind][session]).get(
                         tuple(row[field] for field in inventory.TABLES[kind][session].pk)) or original)
            require(all(row[field] == reference[field] for field in inventory.TABLES[kind][session].immutable)
                    and all(row[field] == reference[field] for field in
                            ("logout_request_id", "logout_scope", "logout_confirmed_at")), "OLD_BFF_SOURCE_CHANGED")
        for _, row in old(kind, browser):
            require(row["current_session_id"] is None, "OLD_BROWSER_POINTER_LIVE")
    for _, row in old("platform", "platform_app_capabilities"):
        require(row["revoked"], "OLD_CAPABILITY_LIVE")
    for _, row in old("platform", "context_selections"):
        require(row["resource_revoked_at"] is not None, "OLD_SELECTION_LIVE")
    for _, row in old("platform", "auth_authorization_requests"):
        require(row["status"] in ("EXPIRED", "CONSUMED") and row["consumed_at"] is not None,
                "OLD_AUTHORIZATION_REQUEST_LIVE")
    for table in ("auth_totp_enrollments", "auth_totp_factors", "platform_auth_command_intents"):
        historic = historical_rows(history, "platform", table)
        actual = index("platform", table, target["databases"]["platform"][table])
        require(not historic.keys() & actual.keys(), "OLD_PENDING_AUTHORITY_LIVE")
    for original, row in old("platform", "platform_account_command_intents"):
        if original["state"] in ("PENDING_SECRET", "UNKNOWN"):
            require(row["state"] == "ABANDONED", "OLD_UNKNOWN_INTENT_LIVE")

    target_hwm = inventory.highwater(target)
    for reference, floor in latest["payload"]["highwater"].items():
        if reference in target_hwm:
            require(target_hwm[reference] >= floor, "HISTORICAL_VERSION_REGRESSED")
            counts["versionFloors"] += 1
    platform = target["databases"]["platform"]
    for table, field, selected_name, floors in (
            ("users", "session_version", "users", "user_floors"),
            ("memberships", "identity_version", "memberships", "membership_floors")):
        actual = index("platform", table, platform[table])
        originals = index("platform", table, before["platform"][table])
        for object_id, floor in requirements[floors].items():
            row = actual.get((object_id,))
            require(row is not None and row[field] >= floor and row["status"] ==
                    ("ENABLED" if object_id in selected[selected_name] else "DISABLED"),
                    "PRINCIPAL_QUARANTINE_CHANGED")
            ignored = {"status", field, "updated_at"}
            if table == "users":
                ignored.update(("password_hash_sha256", "last_login_at"))
                previous_hashes = {old["password_hash_sha256"] for envelope in history for old in
                                  envelope["document"]["payload"]["inventory"]["databases"]["platform"]["users"]
                                  if old["id"] == object_id}
                require(row["password_hash_sha256"] not in previous_hashes, "OLD_ACCOUNT_PASSWORD_HASH_RESTORED")
            require(reconcile.semantic_equal(originals.get((object_id,)), row, ignored), "PRINCIPAL_AUTHORITY_PAYLOAD_CHANGED")
            counts["quarantinedPrincipals"] += 1
    clients = index("platform", "auth_clients", platform["auth_clients"])
    for original in before["platform"]["auth_clients"]:
        row = clients.get((original["id"],))
        expected = ("RETIRED" if original["status"] == "RETIRED" else "ENABLED"
                    if original["id"] in requirements["eligible_clients"] else "DISABLED")
        require(row is not None and row["status"] == expected
                and row["policy_version"] >= requirements["client_floors"][original["id"]], "CLIENT_POLICY_REGRESSED")
        require(reconcile.semantic_equal(original, row, reconcile.POLICY_IGNORE | {"status"}), "CLIENT_AUTHORITY_PAYLOAD_CHANGED")
    denied = {row["secret_digest_sha256"] for envelope in history for row in
              envelope["document"]["payload"]["inventory"]["databases"]["platform"]["auth_client_credentials"]
              if row["secret_digest_sha256"] is not None}
    historical_versions = {}
    for envelope in history:
        for row in envelope["document"]["payload"]["inventory"]["databases"]["platform"]["auth_client_credentials"]:
            historical_versions[row["client_id"]] = max(historical_versions.get(row["client_id"], 0), row["credential_version"])
        for table in ("identity_service_clients", "context_selection_services", "platform_resource_services"):
            denied.update(row["secret_hash_sha256"] for row in
                          envelope["document"]["payload"]["inventory"]["databases"]["platform"][table])
    require(all(requirements["credential_floors"].get(row["id"], -1) >= historical_versions.get(row["id"], 0)
                for row in before["platform"]["auth_clients"]), "AUTHENTICATED_CREDENTIAL_FLOOR_STALE")
    for client_id in selected["clients"]:
        require(any(row["client_id"] == client_id and row["revoked_at"] is None
                    and row["secret_digest_sha256"] is not None and row["secret_digest_sha256"] not in denied
                    and row["credential_version"] > requirements["credential_floors"][client_id]
                    for row in platform["auth_client_credentials"]), "FRESH_CLIENT_CREDENTIAL_MISSING")
    for table in ("identity_service_clients", "context_selection_services", "platform_resource_services"):
        actual = index("platform", table, platform[table])
        for original in before["platform"][table]:
            row = actual.get((original["id"],))
            enabled = original["id"] in selected["services"].get(table, [])
            require(row is not None and row["status"] == ("ENABLED" if enabled else "DISABLED")
                    and (not enabled or row["secret_hash_sha256"] not in denied), "SERVICE_CREDENTIAL_NOT_ROTATED")
            require(reconcile.semantic_equal(original, row, reconcile.POLICY_IGNORE | {"status", "secret_hash_sha256"}),
                    "SERVICE_AUTHORITY_PAYLOAD_CHANGED")

    for original, row in old("platform", "auth_consents"):
        require(row["status"] == "REVOKED" and row["revoked_at"] is not None
                and row["generation"] >= requirements["consent_floors"].get(original["id"], original["generation"])
                and (original["display_deleted_at"] is None or row["display_deleted_at"] is not None), "OLD_CONSENT_REVIVED")
        counts["consentsClosed"] += 1
    for table in ("auth_invitations", "auth_invitation_verifications"):
        for original, row in old("platform", table):
            require(row["status"] in (("REVOKED", "CONSUMED") if table == "auth_invitations" else ("REVOKED",)),
                    "OLD_INVITATION_LIVE")
            if table == "auth_invitations" and original["status"] == "CONSUMED":
                require(row["status"] == "CONSUMED" and all(row[field] == original[field] for field in
                        ("activation_operation_id", "activated_account_id", "verification_ref", "consumed_at")),
                        "ACTIVATION_RESULT_CHANGED")
    tombstones = {row["user_id"] for row in current["platform"]["identity_account_tombstones"]}
    users = index("platform", "users", platform["users"])
    require(all((object_id,) not in users or users[(object_id,)]["status"] == "DISABLED" for object_id in tombstones),
            "RETIRED_USER_REVIVED")
    links = index("platform", "auth_subject_links", platform["auth_subject_links"])
    for key, original in index("platform", "auth_subject_links", current["platform"]["auth_subject_links"]).items():
        row = links.get(key)
        require(row is not None and all(row[field] == original[field]
                for field in inventory.TABLES["platform"]["auth_subject_links"].immutable)
                and ((original["retired_at"] is None and original["user_id"] not in tombstones)
                     or row["retired_at"] is not None), "SUBJECT_DENIAL_REGRESSED")
    for table in reconcile.SET_POLICIES:
        after = index("platform", table, current["platform"][table])
        expected = {key: row for key, row in index("platform", table, before["platform"][table]).items()
                    if reconcile.semantic_equal(row, after.get(key), reconcile.POLICY_IGNORE)}
        actual = index("platform", table, platform[table])
        require(actual.keys() == expected.keys() and all(reconcile.semantic_equal(row, actual[key], reconcile.POLICY_IGNORE)
                for key, row in expected.items()), "POLICY_INTERSECTION_CHANGED")
    bindings = index("platform", "platform_auth_applications", current["platform"]["platform_auth_applications"])
    changed_apps = {row["application_id"] for row in before["platform"]["platform_auth_applications"]
                    if not reconcile.semantic_equal(row, bindings.get((row["application_id"],)))}
    for table, (field, enabled, disabled, _) in reconcile.STATUS_POLICIES.items():
        if table == "context_selection_registrations":
            continue
        after = index("platform", table, current["platform"][table])
        actual = index("platform", table, platform[table])
        for key, original in index("platform", table, before["platform"][table]).items():
            new = after.get(key)
            safe = reconcile.semantic_equal(original, new, reconcile.POLICY_IGNORE) and (enabled is None or original[field] == enabled)
            expected = original[field] if safe or original[field] in ("RETIRED", "DISABLED", "SUSPENDED") else disabled
            if table in ("applications", "auth_applications") and original.get("id", original.get("application_id")) in changed_apps:
                expected = "DISABLED" if table == "applications" else "RETIRED" if original[field] == "RETIRED" else "SUSPENDED"
            row = actual.get(key)
            require(row is not None and row[field] == expected and
                    reconcile.semantic_equal(original, row, reconcile.POLICY_IGNORE | {field}), "POLICY_AUTHORITY_PAYLOAD_CHANGED")
            counts["policyRows"] += 1
    expected_registrations = reconcile.registration_targets(before["platform"], current["platform"],
        requirements, selected["clients"], selected["services"])
    require(index("platform", "context_selection_registrations", platform["context_selection_registrations"])
            == expected_registrations, "REGISTRATION_BINDING_OR_GENERATION_CHANGED")
    applications = index("platform", "applications", platform["applications"])
    require(all(not row["enabled"] or (clients.get((row["client_id"],), {}).get("policy_version")
                == row["client_policy_version"]
                and applications.get((row["application_id"],), {}).get("status") == "ENABLED"
                and applications[(row["application_id"],)]["access_policy_version"] == row["application_policy_version"]
                and row["member_type"] in applications[(row["application_id"],)]["allowed_member_types"])
                for row in expected_registrations.values()),
            "REGISTRATION_BINDING_OR_GENERATION_CHANGED")
    counts["policyRows"] += len(expected_registrations)
    durable = list(reconcile.RECEIPTS + reconcile.SOURCE_FACTS)
    for kind, prefix in (("platform", "platform"), ("portal", "portal")):
        tables = durable if kind == "platform" else []
        for table in [*tables, prefix + "_backchannel_logout_sources", prefix + "_backchannel_logout_receipts"]:
            actual = index(kind, table, target["databases"][kind][table])
            for key, original in index(kind, table, current[kind][table]).items():
                require(actual.get(key) == original, "IMMUTABLE_FACT_CHANGED_OR_MISSING")
                counts["immutableFacts"] += 1
    for table in ("auth_delivery_jobs", "auth_consent_revocation_outbox", "auth_security_notification_outbox"):
        actual = index("platform", table, platform[table])
        for key, original in index("platform", table, current["platform"][table]).items():
            row = actual.get(key)
            require(row is not None and row["attempt_count"] >= original["attempt_count"], "DELIVERY_FACT_MISSING_OR_REGRESSED")
            if table == "auth_delivery_jobs":
                require(all(row[field] == original[field] for field in inventory.TABLES["platform"][table].immutable)
                        and row["state"] in reconcile.TERMINAL, "DELIVERY_JOB_REOPENED_OR_CHANGED")
                if original["state"] in reconcile.TERMINAL:
                    require(all(row[field] == original[field] for field in reconcile.JOB_STATE), "DELIVERY_TERMINAL_CHANGED")
            elif original["delivery_state"] in reconcile.TERMINAL:
                require(row["delivery_state"] == original["delivery_state"]
                        and all(original[field] is None or row[field] == original[field]
                                for field in ("delivered_at", "terminal_at")), "DELIVERY_TERMINAL_CHANGED")
            counts["deliveryRows"] += 1
    keys = {row["kid"]: row for row in platform["auth_signing_keys"]}
    if validation_profile == "full":
        require(set(("owned-https", "phase23-b", "phase23-c", "phase23-d")) <= keys.keys(), "KEY_FACT_MISSING")
        require(keys["owned-https"]["retired_at"] is not None and not keys["owned-https"]["active"]
                and all(keys[kid]["revoked_at"] is not None and not keys[kid]["active"]
                        for kid in ("phase23-b", "phase23-c"))
                and keys["phase23-d"]["active"] and keys["phase23-d"]["retired_at"] is None
                and keys["phase23-d"]["revoked_at"] is None and sum(row["active"] for row in keys.values()) == 1,
                "KEY_LIFECYCLE_REGRESSED")
    else:
        original_keys = {row["kid"]: row for row in current["platform"]["auth_signing_keys"]}
        require(set(("owned-https", "phase23-c", "phase23-d")) <= keys.keys()
                and set(keys) == set(original_keys), "KEY_FACT_MISSING")
        require(keys["phase23-c"]["revoked_at"] is not None and not keys["phase23-c"]["active"]
                and keys["phase23-d"]["active"] and keys["phase23-d"]["retired_at"] is None
                and keys["phase23-d"]["revoked_at"] is None
                and sum(row["active"] for row in keys.values()) == 1, "KEY_RECOVERY_PROFILE_REGRESSED")
        # Recovery preparation does not exercise A retirement or B lifecycle.
        # Preserve their authenticated facts; never turn their absence into a
        # lifecycle PASS, and keep every other old-carrier invariant above.
        for kid in keys.keys() - {"phase23-c", "phase23-d"}:
            require(keys[kid] == original_keys[kid], "KEY_UNEXERCISED_FACT_CHANGED")
    return counts


def runtime_bindings(config, source, target):
    roles = []
    for slot in ("identity", "platform", "portal"):
        kind = "portal" if slot == "portal" else "platform"
        old, new = urlsplit(source[slot]["DATABASE_URL"]), urlsplit(target[slot]["DATABASE_URL"])
        require(old.scheme in ("postgres", "postgresql") and new.scheme == old.scheme
                and old.hostname == new.hostname == "127.0.0.1" and old.port == new.port == 55494
                and old.path == "/" + config["databases"][kind]["source"]
                and new.path == "/" + config["databases"][kind]["target"]
                and old.username == new.username and not old.query and not new.query
                and not old.fragment and not new.fragment, "RUNTIME_BINDING_INVALID")
        role = unquote(old.username or "")
        core.identifier(role)
        owner = config["databases"][kind]["owner"]
        expected = (owner.removesuffix("_owner") + "_runtime" if slot == "portal" else
                    owner.removesuffix("_migration_owner") + "_" + slot + "_runtime")
        require(role == expected, "RUNTIME_ROLE_INVALID")
        roles.append((kind, role))
    predicates = ["NOT has_database_privilege(" + core.literal(role) + "," +
                  core.literal(config["databases"][kind]["source"]) + ",'CONNECT')" for kind, role in roles]
    require(core.Pg(config, "postgres").query("SELECT (" + " AND ".join(predicates) + ")::boolean;") == "t",
            "SOURCE_RUNTIME_NOT_FENCED")


def authenticated_start(config, material, saved, floor):
    start = core.authenticated_read(config["workDirectory"] / "start.json", material["mac"], "start")
    require(start["checkpointSha256"] == core.sha(saved) and start["floorSha256"] == core.sha(floor)
            and start["sequence"] == floor["sequence"] and "rearmed" in start
            and start["targets"] == {kind: item["target"] for kind, item in config["databases"].items()},
            "REARM_RECEIPT_BINDING_INVALID")
    require(set(start["rearmed"]) == {"users", "clients", "memberships", "services"}, "REARM_SELECTION_INVALID")
    for key in ("users", "clients", "memberships"):
        values = start["rearmed"][key]
        require(type(values) is list and len(set(values)) == len(values)
                and set(values) <= set(start["requirements"]["eligible_" + key]), "REARM_SELECTION_INVALID")
    return start


def load_recovery_gate(config_file, minimum_sequence):
    """Private adapter API, not a formal-report object and never printed by CLI.

    IDs below are authenticated original references only. The browser owner may
    carry this subset through its private control pipe; only fixed conclusions
    belong in the formal report. This function does not connect to PostgreSQL.
    """
    require(type(minimum_sequence) is int and minimum_sequence >= 2, "INDEPENDENT_FLOOR_REQUIRED")
    config = core.load_config(config_file)
    material = core.keys(config)
    saved = core.checkpoint(config, material)
    latest, floor = core.latest_witness(config, material, saved, minimum_sequence)
    start = authenticated_start(config, material, saved, floor)
    requirements = start["requirements"]
    _, versions = core.historical_credentials(config, material, saved, latest, floor)
    require(all(requirements["credential_floors"].get(row["id"], -1) >= versions.get(row["id"], 0)
                for row in saved["inventory"]["databases"]["platform"]["auth_clients"]),
            "AUTHENTICATED_CREDENTIAL_FLOOR_STALE")
    return {"contractVersion": "phase-23-authenticated-recovery-gate/1",
            "authenticatedSequence": floor["sequence"], "checkpointSha256": core.sha(saved),
            "credentialFloors": requirements["credential_floors"], "membershipFloors": requirements["membership_floors"],
            "userFloors": requirements["user_floors"], "clientFloors": requirements["client_floors"],
            "rearmed": start["rearmed"]}


def run(args):
    config = core.load_config(args.config)
    root = Path(config["taskRoot"])
    paths = {name: core.safe_path(getattr(args, name), root) for name in ("config", "source_env", "target_env", "output")}
    require(len(set(paths.values())) == 4, "PATH_COLLISION")
    require(type(args.minimum_sequence) is int and args.minimum_sequence >= 2, "INDEPENDENT_FLOOR_REQUIRED")
    source_bytes, target_bytes = core.private_read(paths["source_env"]), core.private_read(paths["target_env"])
    source, target_env = core.decode(source_bytes), core.decode(target_bytes)
    material = core.keys(config)
    saved = core.checkpoint(config, material)
    latest, floor = core.latest_witness(config, material, saved, args.minimum_sequence)
    history = core.witness_store(config, material, saved).load_history(minimum_sequence=floor["sequence"])
    require(history and core.sha(history[-1]["document"]) == floor["witnessSha256"], "HISTORY_FLOOR_MISMATCH")
    start = authenticated_start(config, material, saved, floor)
    require(start["materialPaths"]["environment"] == str(paths["target_env"])
            and start["materialDigests"]["environment"] == core.sha(target_bytes), "REARM_RECEIPT_BINDING_INVALID")
    materials = core.decode(core.decrypt(config, material, "materials", saved["archives"]["materials"], saved["binding"]))
    require(base64.b64decode(materials["environment"], validate=True) == source_bytes, "SOURCE_ENVIRONMENT_CHANGED")
    with readonly_queries():
        core.guard(config)
        core.guard(config, target=True, quiet=False)
        core.assert_catalogs(config, saved)
        runtime_bindings(config, source, target_env)
        captured = core.inventory_capture(config, target=True)
        counts = evaluate(saved, latest, history, captured, start,
                          getattr(args, "validation_profile", "full"))
        # Current fresh installs have no legacy authority relations.
        # Fail if this target has acquired a second protocol registry or carrier.
        for table in LEGACY:
            require(core.Pg(config, config["databases"]["platform"]["target"]).query(
                "SELECT to_regclass('public." + table + "') IS NOT NULL;") == "f", "LEGACY_STATE_PRESENT")
    result = {"contractVersion": "phase-23-readonly-invariants/1", "status": "PASS",
              "validationProfile": getattr(args, "validation_profile", "full"),
              "fullKeyLifecycleChecked": getattr(args, "validation_profile", "full") == "full",
              "validationLevel": "READ_ONLY_CURRENT_TARGET_OLD_ID_INVARIANTS", "counts": counts,
              "authenticatedSequence": floor["sequence"], "checkpointSha256": core.sha(saved),
              "sourceEnvironmentSha256": core.sha(source_bytes), "targetEnvironmentSha256": core.sha(target_bytes),
              "targetInventorySha256": core.sha(captured), "opensTraffic": False,
              "completeRtoClaim": False, "ignoresNewCarrierRows": True}
    core.private_write(paths["output"], result)
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("config", "source-env", "target-env", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--minimum-sequence", type=int, required=True)
    parser.add_argument("--validation-profile", choices=("full", "recovery"), default="full")
    args = parser.parse_args(argv)
    try:
        result = run(args)
        print(json.dumps({key: result[key] for key in ("status", "counts", "opensTraffic", "completeRtoClaim", "validationProfile", "fullKeyLifecycleChecked")}, sort_keys=True))
        return 0
    except Exception as error:
        code = str(error) if isinstance(error, core.RecoveryError) else "RECOVERY_INVARIANT_REJECTED"
        if not re.fullmatch(r"RECOVERY_[A-Z0-9_]{1,100}", code):
            code = "RECOVERY_INVARIANT_REJECTED"
        print(json.dumps({"status": "QUARANTINED", "code": code}), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
