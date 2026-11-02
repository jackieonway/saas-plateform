"""Deny-only SQL planning for the canonical recovery inventory.

This module never connects to PostgreSQL. The offline driver must bind both
databases, stop every writer and worker, authenticate the checkpoint/witness and
run each returned list in one transaction with triggers enabled. SQL containing
encrypted-delta receipt material is private input: never log or serialize a plan.
Only ``requirements`` is suitable for a protected, non-secret start receipt.
"""
from datetime import datetime, timedelta, timezone
import json
import re
import uuid

from inventory import TABLES, project_row, row_key, validate_inventory


class ReconcileError(RuntimeError):
    """A fixed failure category; never includes SQL, rows or secret material."""


def require(condition, category):
    if not condition:
        raise ReconcileError("RECOVERY_" + category)


def identifier(value):
    require(isinstance(value, str) and re.fullmatch(r"[a-z_][a-z0-9_]{0,62}", value),
            "IDENTIFIER_INVALID")
    return '"' + value + '"'


def literal(value, pg_type=None):
    """SQL literals, never shell escaping. Values are independently typed first."""
    if value is None:
        return "NULL"
    if type(value) is bool:
        return "true" if value else "false"
    if type(value) is int:
        require(-(1 << 63) < value < (1 << 63), "INTEGER_INVALID")
        return str(value)
    if pg_type in ("json", "jsonb"):
        value = json.dumps(value, sort_keys=True, separators=(",", ":"),
                           ensure_ascii=False, allow_nan=False)
        return literal(value) + "::" + pg_type
    if isinstance(value, list):
        element = {"_text": "text", "_uuid": "uuid", "_varchar": "varchar"}.get(pg_type)
        require(element is not None, "ARRAY_TYPE_INVALID")
        return "ARRAY[" + ",".join(literal(item) for item in value) + "]::" + element + "[]"
    require(isinstance(value, str) and "\x00" not in value and len(value) <= 2 * 1024 * 1024,
            "LITERAL_INVALID")
    return "E'" + value.replace("\\", "\\\\").replace("'", "''") + "'"


def key(kind, table, row):
    return tuple(row[column] for column in TABLES[kind][table].pk)


def indexed(kind, table, rows):
    result = {key(kind, table, row): row for row in rows}
    require(len(result) == len(rows), "DUPLICATE_FACT")
    return result


def where(kind, table, row):
    spec = TABLES[kind][table]
    return " AND ".join(identifier(column) + " IS NOT DISTINCT FROM "
                        + literal(row[column], spec.fields[column].pg_type)
                        for column in spec.pk)


def update(kind, table, row, assignments):
    """A suppressed trigger cannot silently turn an exact write into success."""
    sql = "UPDATE public." + identifier(table) + " SET " + assignments + " WHERE " + where(kind, table, row)
    return "WITH changed AS (" + sql + " RETURNING 1) SELECT 1/((count(*)=1)::int) FROM changed;"


def bulk(table, assignments, condition="true"):
    return "UPDATE public." + identifier(table) + " SET " + assignments + " WHERE " + condition + ";"


def greater(old, new, field, increment=0):
    maximum = max(old.get(field) or 0, (new or {}).get(field) or 0,
                  getattr(old, "_highwater", {}).get(field, 0),
                  getattr(new, "_highwater", {}).get(field, 0))
    require(type(maximum) is int and maximum + increment < (1 << 63) - 1,
            "GENERATION_EXHAUSTED")
    return maximum + increment


class _RecoveryRow(dict):
    """Internal, immutable-by-convention row metadata; never persisted as fields."""


def _with_highwater(value, external_highwater):
    floors = external_highwater or {}
    require(type(floors) is dict and len(floors) <= 20000, "HIGHWATER_INVALID")
    for reference, floor in floors.items():
        require(type(reference) is str and len(reference) <= 2048
                and re.match(r"^(platform|portal)/[a-z_][a-z0-9_]{0,62}/", reference)
                and all(ord(character) >= 32 for character in reference)
                and type(floor) is int and 0 <= floor <= (1 << 63) - 2,
                "HIGHWATER_INVALID")
    result = dict(value, databases={})
    for kind, tables in value["databases"].items():
        result["databases"][kind] = {}
        for table, rows in tables.items():
            spec = TABLES[kind][table]
            enriched = []
            for row in rows:
                item = _RecoveryRow(row)
                prefix = kind + "/" + table + "/" + row_key(spec, row) + "/"
                item._highwater = {field: floors[prefix + field] for field in spec.fields if prefix + field in floors}
                enriched.append(item)
            result["databases"][kind][table] = enriched
    return result


def _key_window(row):
    raw = row["last_signed_exp"]
    floor = getattr(row, "_highwater", {}).get("last_signed_exp")
    if floor is None:
        return raw
    try:
        stamp = datetime(1970, 1, 1, tzinfo=timezone.utc) + timedelta(microseconds=floor)
    except (OverflowError, ValueError):
        raise ReconcileError("RECOVERY_KEY_WINDOW_INVALID") from None
    return later(raw, stamp.isoformat())


def later(a, b):
    values = [value for value in (a, b) if value is not None]
    return max(values, key=lambda item: datetime.fromisoformat(item.replace("Z", "+00:00"))) if values else None


def semantic_equal(a, b, ignored=()):
    if b is None:
        return False
    return {k: v for k, v in a.items() if k not in ignored} == {
        k: v for k, v in b.items() if k not in ignored}


RECEIPTS = (
    "identity_account_command_receipts", "identity_auth_command_receipts",
    "identity_consent_command_receipts", "auth_mfa_recovery_receipts",
    "auth_mfa_recovery_approvals", "platform_auth_review_operations",
    "auth_rp_sessions", "auth_logout_command_receipts",
    "auth_invitation_activation_receipts", "identity_audit_logs", "audit_logs",
)
SOURCE_FACTS = ("identity_account_tombstones", "identity_account_revocation_facts")
TERMINAL = frozenset(("DELIVERED", "TERMINAL", "DEGRADED"))
JOB_STATE = ("state", "available_at", "attempt_count", "lease_id", "lease_until", "last_jti",
             "last_error_code", "delivered_at", "terminal_at")
POLICY_IGNORE = frozenset(("updated_at", "projected_at", "version", "policy_version",
                           "projection_version", "access_policy_version", "credential_version"))
SET_POLICIES = ("auth_client_permissions", "auth_consent_group_clients", "auth_consent_scopes",
                "membership_data_grants", "user_roles", "role_menus", "tenant_applications",
                "tenant_menus", "organization_applications", "organization_menus",
                "platform_auth_application_operators")
STATUS_POLICIES = {
    "applications": ("status", "ENABLED", "DISABLED", "access_policy_version"),
    "auth_applications": ("status", "APPROVED", "SUSPENDED", "projection_version"),
    "auth_application_admissions": ("status", "APPROVED", "SUSPENDED", "version"),
    "auth_resources": ("status", "ACTIVE", "SUSPENDED", "version"),
    "auth_scopes": ("status", "ACTIVE", "SUSPENDED", "version"),
    "auth_consent_groups": ("status", "ACTIVE", "SUSPENDED", "version"),
    "auth_subject_sectors": ("status", "ACTIVE", "RETIRED", "version"),
    "platform_auth_partners": ("status", "ACTIVE", "SUSPENDED", "version"),
    "platform_auth_application_requests": ("status", None, "SUSPENDED", "version"),
    "platform_machine_delegations": ("status", "ENABLED", "DISABLED", "version"),
    "tenants": ("status", "ENABLED", "DISABLED", None),
    "organizations": ("status", "ENABLED", "DISABLED", None),
    "roles": ("status", "ENABLED", "DISABLED", None),
    "menus": ("status", "ENABLED", "DISABLED", None),
    "context_selection_registrations": ("enabled", True, False, None),
    "platform_resource_bindings": ("enabled", True, False, None),
}


def _raw_original(kind, table, row, raw_latest):
    require(raw_latest is not None and kind in raw_latest and table in raw_latest[kind],
            "MISSING_ORIGINAL_FACT")
    matches = [raw for raw in raw_latest[kind][table] if all(
        raw.get(column) == row[column] for column in TABLES[kind][table].pk)]
    require(len(matches) == 1, "MISSING_ORIGINAL_FACT")
    raw = matches[0]
    require(project_row(kind, table, raw) == row, "ORIGINAL_FACT_DIGEST_MISMATCH")
    return raw


def _insert(kind, table, row, raw_latest=None):
    spec = TABLES[kind][table]
    if any(field.redacted for field in spec.fields.values()):
        raw = _raw_original(kind, table, row, raw_latest)
    else:
        raw = {field.source: row[name] for name, field in spec.fields.items()}
    columns = [field.source for field in spec.fields.values()]
    values = [literal(raw[field.source], field.pg_type) for field in spec.fields.values()]
    return ("INSERT INTO public." + identifier(table) + "(" + ",".join(map(identifier, columns))
            + ") VALUES(" + ",".join(values) + ");")


def _fk_exists(before, table, column, value):
    if value is None:
        return True
    return any(row[column] == value for row in before[table])


def _original_references(before, table, row):
    """Only sources already present in the checkpoint can support new facts.

    In particular, a post-checkpoint grant/user is never recreated to make a
    receipt insert pass. Trigger checks remain the final authority at execution.
    """
    relations = {
        "identity_account_command_receipts": (("target_user_id", "users", "id"),),
        "identity_auth_command_receipts": (("service_id", "identity_service_clients", "id"),),
        "identity_consent_command_receipts": (("user_id", "users", "id"),
                                               ("consent_id", "auth_consents", "id")),
        "auth_mfa_recovery_receipts": (("target_user_id", "users", "id"),),
        "auth_mfa_recovery_approvals": (("approver_user_id", "users", "id"),
                                        ("operation_id", "auth_mfa_recovery_receipts", "operation_id")),
        "auth_rp_sessions": (("grant_id", "auth_grants", "id"), ("client_id", "auth_clients", "id"),
                             ("user_id", "users", "id"), ("source_session_id", "sso_sessions", "id")),
        "auth_logout_command_receipts": (("grant_id", "auth_grants", "id"),
                                          ("grant_id", "auth_rp_sessions", "grant_id"),
                                          ("client_id", "auth_clients", "id"), ("user_id", "users", "id")),
        "auth_invitation_activation_receipts": (("invitation_id", "auth_invitations", "id"),
                                                 ("verification_id", "auth_invitation_verifications", "id"),
                                                 ("user_id", "users", "id"),
                                                 ("delivery_target_id", "auth_delivery_targets", "id")),
        "platform_auth_review_operations": (("actor_user_id", "users", "id"),
                                             ("request_id", "platform_auth_application_requests", "id")),
        "auth_subject_links": (("user_id", "users", "id"), ("sector_id", "auth_subject_sectors", "id")),
        "auth_consent_revocation_outbox": (("consent_id", "auth_consents", "id"),
                                           ("user_id", "users", "id"), ("group_id", "auth_consent_groups", "id"),
                                           ("client_id", "auth_clients", "id")),
        "auth_delivery_jobs": (("target_id", "auth_delivery_targets", "id"),
                                ("grant_id", "auth_grants", "id"),
                                ("invitation_id", "auth_invitations", "id"),
                                ("security_event_id", "auth_security_notification_outbox", "id")),
    }
    for source, target, target_column in relations.get(table, ()):
        require(_fk_exists(before, target, target_column, row[source]), "MISSING_ORIGINAL_SOURCE")
    if table == "auth_rp_sessions":
        original = next(item for item in before["auth_grants"] if item["id"] == row["grant_id"])
        require(original["subject_mode"] == "USER" and original["revoked_at"] is None,
                "MISSING_ACTIVE_ORIGINAL_SOURCE")
    if table == "auth_logout_command_receipts":
        access = any(item["grant_id"] == row["grant_id"] and item["token_digest"] == row["token_digest"]
                     for item in before["auth_access_tokens"])
        families = {item["id"] for item in before["auth_refresh_families"] if item["grant_id"] == row["grant_id"]}
        refresh = any(item["family_id"] in families and item["token_digest"] == row["token_digest"]
                      for item in before["auth_refresh_tokens"])
        require(access or refresh, "MISSING_ORIGINAL_TOKEN_SOURCE")


def _merge_originals(before, latest, sql, raw_latest):
    # These receipts are imported before any source is revoked. No UPDATE is
    # emitted for an existing immutable row. All post-checkpoint raw facts must
    # match their authenticated, secret-free inventory projection.
    for table in RECEIPTS:
        old = indexed("platform", table, before[table])
        for row in latest[table]:
            original = old.get(key("platform", table, row))
            if original is not None:
                require(original == row, "IMMUTABLE_RECEIPT_CHANGED")
                continue
            _original_references(before, table, row)
            sql.append(_insert("platform", table, row, raw_latest))
    for table in SOURCE_FACTS:
        old = indexed("platform", table, before[table])
        for row in latest[table]:
            original = old.get(key("platform", table, row))
            if original is not None:
                require(original == row, "IMMUTABLE_SOURCE_FACT_CHANGED")
            else:
                sql.append(_insert("platform", table, row, raw_latest))
    old_links = indexed("platform", "auth_subject_links", before["auth_subject_links"])
    for row in latest["auth_subject_links"]:
        if key("platform", "auth_subject_links", row) not in old_links:
            _original_references(before, "auth_subject_links", row)
            sql.append(_insert("platform", "auth_subject_links", row, raw_latest))
    old_selections = indexed("platform", "context_selections", before["context_selections"])
    for row in latest["context_selections"]:
        original = old_selections.get(key("platform", "context_selections", row))
        if row["consumed_at"] is not None:
            require(original is not None, "MISSING_COMMITTED_SELECTION")
            if original["consumed_at"] is None:
                raw = _raw_original("platform", "context_selections", row, raw_latest)
                sql.append(update("platform", "context_selections", original,
                                  "consumed_at=" + literal(raw["consumed_at"]) + ",result="
                                  + literal(raw["result"], "jsonb") + ",result_ciphertext="
                                  + literal(raw["result_ciphertext"]) + ",credential_ciphertext=NULL"))
            else:
                require(all(original[column] == row[column] for column in
                            ("consumed_at", "result_sha256", "result_ciphertext_sha256")),
                        "COMMITTED_SELECTION_CHANGED")
    if latest["audit_logs"]:
        sql.append("SELECT setval(pg_get_serial_sequence('public.audit_logs','id'),"
                   "GREATEST(COALESCE((SELECT max(id) FROM public.audit_logs),1),"
                   "(SELECT last_value FROM public.audit_logs_id_seq)),true);")


def _merge_backchannel(kind, before, latest, sql):
    prefix = "platform" if kind == "platform" else "portal"
    for suffix, unique in (("sources", ("issuer", "client_id", "sid_digest")),
                           ("receipts", ("issuer", "client_id", "jti_digest"))):
        table = prefix + "_backchannel_logout_" + suffix
        old = indexed(kind, table, before[table])
        by_source = {tuple(row[c] for c in unique): row for row in before[table]}
        for row in latest[table]:
            original = old.get(key(kind, table, row))
            collision = by_source.get(tuple(row[c] for c in unique))
            require(collision is None or collision == row, "BACKCHANNEL_SOURCE_COLLISION")
            if original is not None:
                require(original == row, "BACKCHANNEL_FACT_CHANGED")
            else:
                sql.append(_insert(kind, table, row))


def _delivery(before, latest, sql, raw_latest):
    old_events = indexed("platform", "auth_consent_revocation_outbox", before["auth_consent_revocation_outbox"])
    for row in latest["auth_consent_revocation_outbox"]:
        if key("platform", "auth_consent_revocation_outbox", row) in old_events:
            continue
        _original_references(before, "auth_consent_revocation_outbox", row)
        # Inserting a new consent event while original grants are still active
        # avoids generating a replacement job with a different created_at.
        matched = [grant for grant in before["auth_grants"] if grant["consent_id"] == row["consent_id"]
                   and grant["user_id"] == row["user_id"] and grant["client_id"] == row["client_id"]]
        rp_grants = {item["grant_id"] for item in before["auth_rp_sessions"]}
        job_grants = {item["grant_id"] for item in before["auth_delivery_jobs"] if item["grant_id"] is not None}
        require(all(grant["revoked_at"] is None or grant["id"] not in rp_grants
                    or grant["id"] in job_grants for grant in matched),
                "ORIGINAL_OUTBOX_EXPANSION_UNSAFE")
        sql.append(_insert("platform", "auth_consent_revocation_outbox", row, raw_latest))
    old_security = indexed("platform", "auth_security_notification_outbox", before["auth_security_notification_outbox"])
    require(all(key("platform", "auth_security_notification_outbox", row) in old_security
                for row in latest["auth_security_notification_outbox"]), "MISSING_SECURITY_OUTBOX_ORIGINAL")
    for table in ("auth_consent_revocation_outbox", "auth_security_notification_outbox"):
        old = indexed("platform", table, before[table])
        for row in latest[table]:
            original = old.get(key("platform", table, row))
            if original is None:
                continue
            immutable = TABLES["platform"][table].immutable
            require(all(original[column] == row[column] for column in immutable), "OUTBOX_SOURCE_CHANGED")
            require(row["attempt_count"] >= original["attempt_count"], "ATTEMPT_REGRESSION")
            columns = ("attempt_count", "available_at", "delivered_at", "last_error_code", "expanded_at",
                       "terminal_at", "delivery_state")
            sql.append(update("platform", table, original, ",".join(
                identifier(column) + "=" + literal(greater(original, row, column) if column == "attempt_count" else row[column],
                                                  TABLES["platform"][table].fields[column].pg_type)
                for column in columns)))
    old_jobs = indexed("platform", "auth_delivery_jobs", before["auth_delivery_jobs"])
    current_jobs = indexed("platform", "auth_delivery_jobs", latest["auth_delivery_jobs"])
    event_ids = {item["id"] for item in before["auth_consent_revocation_outbox"]}
    event_ids.update(item["id"] for item in latest["auth_consent_revocation_outbox"])
    for row in latest["auth_delivery_jobs"]:
        original = old_jobs.get(key("platform", "auth_delivery_jobs", row))
        if original is None:
            _original_references(before, "auth_delivery_jobs", row)
            require(row["consent_event_id"] is None or row["consent_event_id"] in event_ids,
                    "MISSING_ORIGINAL_SOURCE")
            sql.append(_insert("platform", "auth_delivery_jobs", row, raw_latest))
            continue
        immutable = set(TABLES["platform"]["auth_delivery_jobs"].immutable) - {"consent_event_id"}
        require(all(original[column] == row[column] for column in immutable), "DELIVERY_BINDING_CHANGED")
        require(original["consent_event_id"] is None or original["consent_event_id"] == row["consent_event_id"],
                "DELIVERY_BINDING_CHANGED")
        require(row["attempt_count"] >= original["attempt_count"], "ATTEMPT_REGRESSION")
        require(greater(original, row, "max_attempts") == row["max_attempts"], "DELIVERY_BUDGET_CHANGED")
        if original["state"] in TERMINAL:
            require(all(original[column] == row[column] for column in JOB_STATE), "DELIVERY_TERMINAL_CHANGED")
            require(greater(original, row, "attempt_count") == row["attempt_count"], "DELIVERY_TERMINAL_CHANGED")
            if original["consent_event_id"] != row["consent_event_id"]:
                sql.append(update("platform", "auth_delivery_jobs", original,
                                  "consent_event_id=" + literal(row["consent_event_id"])))
            continue
        columns = JOB_STATE + ("consent_event_id",)
        sql.append(update("platform", "auth_delivery_jobs", original, ",".join(
            identifier(column) + "=" + literal(greater(original, row, column) if column == "attempt_count" else row[column],
                                              TABLES["platform"]["auth_delivery_jobs"].fields[column].pg_type)
            for column in columns)))
    for original in before["auth_delivery_jobs"]:
        if key("platform", "auth_delivery_jobs", original) in current_jobs:
            continue
        attempts = greater(original, None, "attempt_count")
        require(attempts <= original["max_attempts"]
                and greater(original, None, "max_attempts") == original["max_attempts"], "DELIVERY_BUDGET_CHANGED")
        if original["state"] in TERMINAL:
            require(attempts == original["attempt_count"], "DELIVERY_TERMINAL_CHANGED")
        else:
            sql.append(update("platform", "auth_delivery_jobs", original, "attempt_count=" + str(attempts)))
    # INSERT/UPDATE job triggers recompute source summaries. Reapply the
    # authenticated original summary only after all original job states have
    # been imported, before the separate recovery quarantine terminates pending
    # work. This preserves historical completed timestamps and attempt budgets.
    for table in ("auth_consent_revocation_outbox", "auth_security_notification_outbox"):
        for row in latest[table]:
            columns = ("attempt_count", "available_at", "delivered_at", "last_error_code", "expanded_at",
                       "terminal_at", "delivery_state")
            sql.append(update("platform", table, row, ",".join(
                identifier(column) + "=" + literal(greater(row, None, column) if column == "attempt_count" else row[column],
                                                  TABLES["platform"][table].fields[column].pg_type)
                for column in columns)))


def _policies(before, latest, sql, requirements):
    """Unchanged policies can remain available; ambiguous authority is disabled.

    No absent/new policy is recreated. Set-like grant rows are intersected by
    exact original ID and semantic payload; a changed row is deleted rather than
    guessed narrower. This also handles deleted members without FK cascades.
    """
    for table in SET_POLICIES:
        current = indexed("platform", table, latest[table])
        for row in before[table]:
            new = current.get(key("platform", table, row))
            if not semantic_equal(row, new, POLICY_IGNORE):
                sql.append("DELETE FROM public." + identifier(table) + " WHERE " + where("platform", table, row) + ";")
            elif "version" in row:
                sql.append(update("platform", table, row, "version=" + str(greater(row, new, "version"))))
    for table, (field, enabled, disabled, version) in STATUS_POLICIES.items():
        if table == "context_selection_registrations":
            # A matching registration is still fenced to an old client policy.
            # Explicit rearm, including its consumer, is the only enable path.
            sql.append(bulk(table, "enabled=false"))
            continue
        current = indexed("platform", table, latest[table])
        for row in before[table]:
            new = current.get(key("platform", table, row))
            safe = semantic_equal(row, new, POLICY_IGNORE) and (enabled is None or row[field] == enabled)
            if safe:
                if version:
                    sql.append(update("platform", table, row, identifier(version) + "="
                                      + str(greater(row, new, version))))
                continue
            value = row[field] if row[field] in ("RETIRED", "DISABLED", "SUSPENDED") else disabled
            assignments = identifier(field) + "=" + literal(value)
            if version:
                assignments += "," + identifier(version) + "=" + str(greater(row, new, version, 1))
            sql.append(update("platform", table, row, assignments))
    current = indexed("platform", "platform_auth_applications", latest["platform_auth_applications"])
    for row in before["platform_auth_applications"]:
        if not semantic_equal(row, current.get((row["application_id"],))):
            sql.append("UPDATE public.auth_applications SET status=CASE WHEN status='RETIRED' THEN status ELSE 'SUSPENDED' END,"
                       "projection_version=projection_version+1 WHERE application_id=" + literal(row["application_id"]) + ";")
            sql.append("UPDATE public.applications SET status='DISABLED',access_policy_version=access_policy_version+1 WHERE id="
                       + literal(row["application_id"]) + ";")
    sql.append(bulk("platform_auth_release_bindings", "enabled=false"))
    # Client identity/callback/scope mutations require fresh trusted review.
    current = indexed("platform", "auth_clients", latest["auth_clients"])
    for row in before["auth_clients"]:
        new = current.get(key("platform", "auth_clients", row))
        safe = semantic_equal(row, new, POLICY_IGNORE) and row["status"] == "ENABLED"
        if safe:
            requirements["eligible_clients"].append(row["id"])
        status = row["status"] if row["status"] == "RETIRED" else "ENABLED" if safe else "DISABLED"
        assignments = "status=" + literal(status) + ",version=" + str(greater(row, new, "version", 1))
        assignments += ",policy_version=" + str(greater(row, new, "policy_version", 1))
        sql.append(update("platform", "auth_clients", row, assignments))
        requirements["client_floors"][row["id"]] = greater(row, new, "policy_version", 1)
    # Every old service credential is disabled; explicit rearm must rotate it.
    for table in ("identity_service_clients", "context_selection_services", "platform_resource_services"):
        current = indexed("platform", table, latest[table])
        for row in before[table]:
            new = current.get(key("platform", table, row))
            if semantic_equal(row, new, POLICY_IGNORE | {"secret_hash_sha256"}) and row["status"] == "ENABLED":
                requirements["eligible_services"].setdefault(table, []).append(row["id"])
            assignments = "status='DISABLED'"
            if "credential_version" in row:
                assignments += ",credential_version=" + str(greater(row, new, "credential_version", 1))
            sql.append(update("platform", table, row, assignments))
    current = indexed("platform", "auth_delivery_targets", latest["auth_delivery_targets"])
    for row in before["auth_delivery_targets"]:
        new = current.get((row["id"],))
        safe = semantic_equal(row, new, POLICY_IGNORE) and row["status"] == "ACTIVE"
        status = "ACTIVE" if safe else "REVOKED"
        sql.append(update("platform", "auth_delivery_targets", row,
                          "status=" + literal(status) + ",version="
                          + str(greater(row, new, "version", 0 if safe else 1))))


def registration_bindings(before, latest, requirements):
    """Exact unchanged AUTH bindings that can follow a selected client rearm."""
    result = {}
    current = indexed("platform", "context_selection_registrations", latest["context_selection_registrations"])
    clients = [indexed("platform", "auth_clients", tables["auth_clients"]) for tables in (before, latest)]
    apps = [indexed("platform", "applications", tables["applications"]) for tables in (before, latest)]
    consumers = indexed("platform", "context_selection_services", before["context_selection_services"])
    projected = indexed("platform", "platform_auth_applications", latest["platform_auth_applications"])
    changed_apps = {row["application_id"] for row in before["platform_auth_applications"]
                    if not semantic_equal(row, projected.get((row["application_id"],)))}
    for row in before["context_selection_registrations"]:
        client_id, application = row["client_id"], row["application_id"]
        if (not row["enabled"] or row["client_registry"] != "AUTH" or application is None
                or row != current.get((client_id,)) or client_id not in requirements["eligible_clients"]
                or application in changed_apps):
            continue
        bound = True
        for registered, applications in zip(clients, apps):
            client, app = registered.get((client_id,)), applications.get((application,))
            bound = bound and (client is not None and app is not None
                and client["subject_mode"] == "USER" and client["context_mode"] == "PLATFORM_CONTEXT"
                and client["client_type"] == "CONFIDENTIAL" and client["application_id"] == application
                and client["allowed_member_types"] == [row["member_type"]]
                and client["policy_version"] == row["client_policy_version"]
                and app["status"] == "ENABLED" and app["access_policy_version"] == row["application_policy_version"]
                and requirements.get("registration_application_floors", {}).get(application)
                    == row["application_policy_version"]
                and row["member_type"] in app["allowed_member_types"])
        if not bound or not semantic_equal(apps[0][(application,)], apps[1][(application,)], POLICY_IGNORE):
            continue
        consumer_id = row["consumer_service_id"]
        if consumer_id is not None:
            consumer = consumers.get((consumer_id,))
            if (consumer_id not in requirements["eligible_services"].get("context_selection_services", [])
                    or consumer is None or consumer["purpose"] != "CONTEXT_SELECTION_CONSUMER"
                    or client_id not in consumer["allowed_auth_clients"]):
                continue
        result[client_id] = dict(row, enabled=True,
            client_policy_version=requirements["client_floors"][client_id])
    return result


def registration_targets(before, latest, requirements, selected_clients=(), selected_services=None):
    """Only selected unchanged client/consumer tuples may use the exact new floor."""
    bindings = registration_bindings(before, latest, requirements)
    require(requirements.get("registration_bindings", {}) == bindings, "REGISTRATION_REQUIREMENTS_CHANGED")
    consumers = set((selected_services or {}).get("context_selection_services", ()))
    selected = set(selected_clients)
    result = {}
    for row in before["context_selection_registrations"]:
        client_id = row["client_id"]
        expected = dict(row, enabled=False)
        binding = bindings.get(client_id)
        if (binding is not None and client_id in selected
                and (binding["consumer_service_id"] is None or binding["consumer_service_id"] in consumers)):
            expected = dict(binding)
        result[(client_id,)] = expected
    return result


def _bff_lifecycle(kind, table, before, latest, sql):
    current = indexed(kind, table, latest[table])
    for row in before[table]:
        new = current.get(key(kind, table, row))
        if new is None:
            continue
        require(all(row[column] == new[column] for column in TABLES[kind][table].immutable),
                "BFF_SOURCE_CHANGED")
        for column in ("logout_request_id", "logout_scope", "logout_confirmed_at"):
            require(row[column] is None or row[column] == new[column], "BFF_LOGOUT_RECEIPT_CHANGED")
        assignments = ",".join(identifier(column) + "=" + literal(new[column])
                               for column in ("logout_request_id", "logout_scope", "logout_confirmed_at"))
        sql.append(update(kind, table, row, assignments))


def _browser_floor(kind, table, before, latest, sql):
    current = indexed(kind, table, latest[table])
    for row in before[table]:
        new = current.get(key(kind, table, row))
        sql.append(update(kind, table, row, "current_session_id=NULL,generation="
                          + str(greater(row, new, "generation", 1))))


def _keys(before, latest, sql, requirements):
    require(len(before["auth_provider_settings"]) == 1 and before["auth_provider_settings"][0]["singleton"]
            and before["auth_provider_settings"] == latest["auth_provider_settings"], "ISSUER_CHANGED")
    current = indexed("platform", "auth_signing_keys", latest["auth_signing_keys"])
    sql.append(bulk("auth_signing_keys", "active=false", "active"))
    for row in before["auth_signing_keys"]:
        new = current.get((row["kid"],))
        require(new is not None and all(row[column] == new[column] for column in ("kid", "n", "e", "published_at")),
                "KEY_MATERIAL_CHANGED_OR_OMITTED")
        retired, revoked = later(row["retired_at"], new["retired_at"]), later(row["revoked_at"], new["revoked_at"])
        assignments = "active=false,retired_at=" + literal(retired) + ",revoked_at=" + literal(revoked)
        assignments += ",activated_at=" + literal(later(row["activated_at"], new["activated_at"]))
        assignments += ",last_signed_exp=" + literal(later(_key_window(row), _key_window(new)))
        sql.append(update("platform", "auth_signing_keys", row, assignments))
        if new["active"] and retired is None and revoked is None:
            require(requirements["active_kid"] is None, "MULTIPLE_ACTIVE_KEYS")
            requirements["active_kid"] = row["kid"]
    # A key absent from the checkpoint may be imported as a public *inactive*
    # denial fact. Private material is never recovered from an inventory.
    old = {row["kid"] for row in before["auth_signing_keys"]}
    for row in latest["auth_signing_keys"]:
        if row["kid"] not in old:
            value = dict(row, active=False)
            sql.append(_insert("platform", "auth_signing_keys", value))
            require(not row["active"], "ACTIVE_KEY_NOT_IN_CHECKPOINT")
    require(requirements["active_kid"] is not None, "NO_SAFE_ACTIVE_KEY")
    sql.append("UPDATE public.auth_signing_keys SET active=true WHERE kid=" + literal(requirements["active_kid"]) + ";")


def _carriers(kind, before, latest, sql, requirements, epoch):
    if kind == "portal":
        _bff_lifecycle(kind, "portal_sessions", before, latest, sql)
        sql.append(bulk("portal_sessions", "revoked_at=COALESCE(revoked_at,clock_timestamp())"))
        _browser_floor(kind, "portal_browsers", before, latest, sql)
        sql.append(bulk("portal_login_flows", "consumed_at=COALESCE(consumed_at,clock_timestamp())"))
        sql.append(bulk("portal_sessions", "membership_id=NULL,organization_id=NULL,identity_version=NULL,"
                        "customer_ciphertext=NULL,customer_expires_at=NULL,customer_needs_renewal=false,"
                        "pending_selection_ciphertext=NULL,refresh_lease_id=NULL,refresh_started_at=NULL,"
                        "refresh_uncertain=true,context_lease_id=NULL,context_started_at=NULL"))
        current = indexed(kind, "portal_sessions", latest["portal_sessions"])
        for row in before["portal_sessions"]:
            sql.append(update(kind, "portal_sessions", row, "version="
                              + str(greater(row, current.get((row["id"],)), "version", 1))))
        return
    _bff_lifecycle(kind, "bff_sessions", before, latest, sql)
    # All USER, offline USER and MACHINE grants are revoked explicitly. SSO
    # triggers alone do not cover offline or machine grant families.
    for table in ("auth_grants", "auth_access_tokens", "auth_refresh_families", "auth_refresh_tokens",
                  "sso_sessions", "platform_sessions", "bff_sessions", "customer_portal_contexts"):
        sql.append(bulk(table, "revoked_at=COALESCE(revoked_at,clock_timestamp())"))
    for table in ("auth_grants", "auth_refresh_families"):
        current = indexed(kind, table, latest[table])
        for row in before[table]:
            sql.append(update(kind, table, row, "version="
                              + str(greater(row, current.get((row["id"],)), "version", 1))))
    sql.append(bulk("platform_app_capabilities", "revoked=true", "NOT revoked"))
    for table in ("auth_authorization_codes", "auth_refresh_tokens", "sso_login_challenges",
                  "console_login_flows", "platform_session_refresh_tokens"):
        sql.append(bulk(table, "consumed_at=COALESCE(consumed_at,clock_timestamp())"))
    sql.append(bulk("auth_authorization_requests", "status='EXPIRED',consumed_at=COALESCE(consumed_at,clock_timestamp())",
                    "status<>'CONSUMED'"))
    # Immutable selected/delivered facts stay intact. Parent authorization
    # requests are closed, hence a retained encrypted pending result is unusable.
    sql.append(bulk("context_selections", "resource_revoked_at=COALESCE(resource_revoked_at,clock_timestamp())"))
    _browser_floor(kind, "console_browsers", before, latest, sql)
    sql.append(bulk("bff_sessions", "selected_session_id=NULL,platform_refresh_ciphertext=NULL,"
                    "refresh_lease_id=NULL,refresh_started_at=NULL,refresh_uncertain=true"))
    current = indexed(kind, "bff_sessions", latest["bff_sessions"])
    for row in before["bff_sessions"]:
        sql.append(update(kind, "bff_sessions", row, "version="
                          + str(greater(row, current.get((row["id"],)), "version", 1))))
    sql.append(bulk("platform_account_command_intents", "state='ABANDONED'", "state IN('PENDING_SECRET','UNKNOWN')"))
    # Old intent acceptance windows must not execute after recovery. Completed
    # receipts remain in identity's separate immutable tables.
    sql.append("DELETE FROM public.platform_auth_command_intents;")
    sql.append(bulk("auth_client_credentials", "revoked_at=COALESCE(revoked_at,clock_timestamp())"))
    for row in before["auth_clients"]:
        requirements["credential_floors"][row["id"]] = max((greater(credential, None, "credential_version")
            for credential in before["auth_client_credentials"] + latest["auth_client_credentials"]
            if credential["client_id"] == row["id"]), default=0)
    for row in before["auth_consents"]:
        new = indexed("platform", "auth_consents", latest["auth_consents"]).get((row["id"],))
        assignments = "status='REVOKED',revoked_at=COALESCE(revoked_at,clock_timestamp()),generation="
        assignments += str(greater(row, new, "generation", 1)) + ",version=" + str(greater(row, new, "version", 1))
        assignments += ",group_version=" + str(greater(row, new, "group_version"))
        if new is not None and new["display_deleted_at"] is not None:
            assignments += ",display_deleted_at=" + literal(new["display_deleted_at"])
        sql.append(update("platform", "auth_consents", row, assignments))
        requirements["consent_floors"][row["id"]] = greater(row, new, "generation", 1)
    # Setting status and generation in one UPDATE would let the status trigger
    # replace the supplied floor. Use a fresh version above every witness fact
    # first, then disable in a separate statement; final expected floor includes
    # that trigger's increment. No intermediate state is externally committed.
    tombstones = {row["user_id"] for row in latest["identity_account_tombstones"]}
    current = indexed("platform", "users", latest["users"])
    for row in before["users"]:
        new = current.get((row["id"],))
        floor = greater(row, new, "session_version", 2)
        require(floor + (1 if row["status"] == "ENABLED" else 0) < (1 << 63) - 1,
                "GENERATION_EXHAUSTED")
        sql.append(update("platform", "users", row, "session_version=" + str(floor)))
        sql.append(update("platform", "users", row, "status='DISABLED',password_hash=" + literal("!recovery-disabled:" + epoch)))
        requirements["user_floors"][row["id"]] = floor + (1 if row["status"] == "ENABLED" else 0)
        if (new is not None and row["status"] == new["status"] == "ENABLED"
                and row["id"] not in tombstones and semantic_equal(row, new, POLICY_IGNORE | {"session_version", "last_login_at"})):
            requirements["eligible_users"].append(row["id"])
    current = indexed("platform", "memberships", latest["memberships"])
    for row in before["memberships"]:
        new = current.get((row["id"],))
        floor = greater(row, new, "identity_version", 1)
        sql.append(update("platform", "memberships", row, "status='DISABLED',identity_version=" + str(floor)))
        requirements["membership_floors"][row["id"]] = floor
        if (new is not None and row["status"] == new["status"] == "ENABLED"
                and row["user_id"] in requirements["eligible_users"]
                and semantic_equal(row, new, POLICY_IGNORE | {"identity_version"})):
            requirements["eligible_memberships"].append(row["id"])
    # A retired subject link is permanent. Do not retire mappings merely for
    # temporary quarantine; eligible restored users retain their stable sub.
    current = indexed("platform", "auth_subject_links", latest["auth_subject_links"])
    for row in before["auth_subject_links"]:
        new = current.get((row["id"],))
        require(new is not None, "SUBJECT_FACT_OMITTED")
        require(all(row[column] == new[column] for column in ("id", "user_id", "sector_id", "subject", "created_at")),
                "SUBJECT_BINDING_CHANGED")
        retired = later(row["retired_at"], new["retired_at"])
        if row["user_id"] in tombstones and retired is None:
            assignments = "retired_at=COALESCE(retired_at,clock_timestamp())"
        else:
            assignments = "retired_at=" + literal(retired)
        sql.append(update("platform", "auth_subject_links", row, assignments))
    # Recovery codes and pending enrollments never regain one-use authority.
    sql.append(bulk("auth_totp_recovery_codes", "consumed_at=COALESCE(consumed_at,clock_timestamp())"))
    sql.append("DELETE FROM public.auth_totp_enrollments;")
    sql.append("DELETE FROM public.auth_totp_factors;")
    current = indexed("platform", "auth_invitations", latest["auth_invitations"])
    for row in before["auth_invitations"]:
        new = current.get((row["id"],))
        if row["status"] != "ACTIVE":
            require(new is not None and row == new, "INVITATION_TERMINAL_CHANGED")
            continue
        # A completed activation cannot be fabricated: its original immutable
        # receipt and FK-bound account must already be present/imported.
        if new is not None and new["status"] == "CONSUMED":
            require(any(receipt["invitation_id"] == row["id"] for receipt in before["auth_invitation_activation_receipts"]),
                    "MISSING_ORIGINAL_ACTIVATION")
            columns = ("status", "version", "consumed_at", "verification_ref", "activated_account_id", "activation_operation_id")
            sql.append(update("platform", "auth_invitations", row, ",".join(
                identifier(column) + "=" + literal(new[column]) for column in columns)))
        else:
            sql.append(update("platform", "auth_invitations", row,
                              "status='REVOKED',revoked_at=clock_timestamp(),version=" + str(greater(row, new, "version", 1))))
    current = indexed("platform", "auth_invitation_verifications", latest["auth_invitation_verifications"])
    for row in before["auth_invitation_verifications"]:
        new = current.get((row["id"],))
        sql.append(update("platform", "auth_invitation_verifications", row,
                          "status='REVOKED',version=" + str(greater(row, new, "version", 1))))
    # Trigger-generated recovery logout jobs also become terminal. Never reset
    # a lease or budget to PENDING and never touch an existing terminal tuple.
    sql.append(bulk("auth_delivery_jobs", "state='TERMINAL',lease_id=NULL,lease_until=NULL,"
                    "last_error_code='RECOVERY_QUARANTINE',terminal_at=clock_timestamp()",
                    "state IN('PENDING','LEASED')"))


def _credential_history(before, latest, requirements, external_credential_floors):
    """Recover credential->client ownership from authenticated witness history.

    Per-row highwater cannot recover that relationship after a credential row
    has disappeared from both checkpoint and latest inventory. The driver must
    aggregate *every authenticated history inventory*, keyed by original client.
    Unknown historical clients never create or authorize checkpoint objects.
    """
    if external_credential_floors is None:
        return
    require(type(external_credential_floors) is dict and len(external_credential_floors) <= 20000,
            "CREDENTIAL_HISTORY_INVALID")
    for client_id, floor in external_credential_floors.items():
        try:
            parsed = uuid.UUID(client_id) if type(client_id) is str else None
        except (ValueError, AttributeError):
            parsed = None
        require(parsed is not None and parsed.version == 7 and parsed.variant == uuid.RFC_4122
                and str(parsed) == client_id and type(floor) is int and 0 <= floor <= (1 << 63) - 2,
                "CREDENTIAL_HISTORY_INVALID")
    known = {row["id"] for row in before["auth_clients"]}
    for client_id in known:
        visible = max((greater(row, None, "credential_version")
                       for row in before["auth_client_credentials"] + latest["auth_client_credentials"]
                       if row["client_id"] == client_id), default=0)
        historical = external_credential_floors.get(client_id, visible)
        require(historical >= visible, "CREDENTIAL_HISTORY_REGRESSION")
        requirements["credential_floors"][client_id] = max(
            requirements["credential_floors"].get(client_id, 0), historical, visible)


def plan(restored, latest, recovery_epoch, raw_latest=None, external_highwater=None,
         external_credential_floors=None):
    """Return private SQL plus non-secret requirements for a quarantined restore."""
    validate_inventory(restored)
    validate_inventory(latest)
    require(restored["schema_sha256"] == latest["schema_sha256"], "SCHEMA_CONTRACT_CHANGED")
    try:
        parsed = uuid.UUID(recovery_epoch)
    except (ValueError, TypeError, AttributeError):
        raise ReconcileError("RECOVERY_EPOCH_INVALID") from None
    require(parsed.version == 7 and str(parsed) == recovery_epoch, "EPOCH_INVALID")
    before = _with_highwater(restored, external_highwater)["databases"]
    current = _with_highwater(latest, external_highwater)["databases"]
    deployment = before["platform"]["identity_deployment_state"]
    require(len(deployment) == 1 and deployment[0]["singleton"] and deployment[0]["schema_version"] == 1
            and deployment == current["platform"]["identity_deployment_state"], "DEPLOYMENT_CHANGED")
    requirements = {"format": "canonical-recovery-requirements/1", "epoch": recovery_epoch,
                    "eligible_users": [], "eligible_clients": [], "eligible_memberships": [],
                    "eligible_services": {}, "user_floors": {}, "membership_floors": {},
                    "client_floors": {}, "credential_floors": {}, "consent_floors": {}, "active_kid": None,
                    "registration_bindings": {}, "registration_application_floors": {}}
    result = {"platform": [], "portal": [], "requirements": requirements}
    _merge_originals(before["platform"], current["platform"], result["platform"], raw_latest)
    _delivery(before["platform"], current["platform"], result["platform"], raw_latest)
    for kind in ("platform", "portal"):
        _merge_backchannel(kind, before[kind], current[kind], result[kind])
    _keys(before["platform"], current["platform"], result["platform"], requirements)
    applications = [indexed("platform", "applications", tables["platform"]["applications"])
                    for tables in (before, current)]
    for row in before["platform"]["context_selection_registrations"]:
        app = row["application_id"]
        if (app,) in applications[0] and (app,) in applications[1]:
            requirements["registration_application_floors"][app] = greater(
                applications[0][(app,)], applications[1][(app,)], "access_policy_version")
    _policies(before["platform"], current["platform"], result["platform"], requirements)
    requirements["registration_bindings"] = registration_bindings(before["platform"], current["platform"], requirements)
    for kind in ("platform", "portal"):
        _carriers(kind, before[kind], current[kind], result[kind], requirements, recovery_epoch)
    _credential_history(before["platform"], current["platform"], requirements, external_credential_floors)
    return result


def verify_sql(restored, latest, recovery_epoch, allowed_rearmed_users=(),
               allowed_rearmed_clients=(), allowed_rearmed_memberships=(),
               allowed_rearmed_services=None, raw_latest=None, external_highwater=None,
               external_credential_floors=None):
    """Boolean guards; old-ID carriers stay denied even after explicit rearm.

    Re-enabling a policy is not credential rotation. The caller must separately
    prove old/new passwords and secrets at their actual normal endpoints.
    """
    recovery = plan(restored, latest, recovery_epoch, raw_latest, external_highwater,
                    external_credential_floors)
    requirements = recovery["requirements"]
    for allowed, eligible in ((allowed_rearmed_users, requirements["eligible_users"]),
                              (allowed_rearmed_clients, requirements["eligible_clients"]),
                              (allowed_rearmed_memberships, requirements["eligible_memberships"])):
        require(set(allowed) <= set(eligible), "REARM_NOT_ELIGIBLE")
    allowed_rearmed_services = allowed_rearmed_services or {}
    require(set(allowed_rearmed_services) <= set(requirements["eligible_services"]), "REARM_SERVICE_INVALID")
    for table, allowed in allowed_rearmed_services.items():
        require(set(allowed) <= set(requirements["eligible_services"].get(table, ())), "REARM_NOT_ELIGIBLE")
    predicates = {"platform": [], "portal": []}
    before = _with_highwater(restored, external_highwater)["databases"]
    latest = _with_highwater(latest, external_highwater)
    for kind in ("platform", "portal"):
        tables = (("portal_sessions", "revoked_at IS NOT NULL AND membership_id IS NULL AND organization_id IS NULL "
                   "AND identity_version IS NULL AND customer_ciphertext IS NULL AND customer_expires_at IS NULL "
                   "AND pending_selection_ciphertext IS NULL AND refresh_uncertain AND refresh_lease_id IS NULL "
                   "AND context_lease_id IS NULL"),
                  ("portal_login_flows", "consumed_at IS NOT NULL"),
                  ("portal_browsers", "current_session_id IS NULL")) if kind == "portal" else (
            ("auth_grants", "revoked_at IS NOT NULL"), ("auth_access_tokens", "revoked_at IS NOT NULL"),
            ("auth_refresh_families", "revoked_at IS NOT NULL"), ("auth_refresh_tokens", "revoked_at IS NOT NULL"),
            ("auth_authorization_codes", "consumed_at IS NOT NULL"), ("sso_sessions", "revoked_at IS NOT NULL"),
            ("bff_sessions", "revoked_at IS NOT NULL AND selected_session_id IS NULL AND platform_refresh_ciphertext IS NULL "
             "AND refresh_uncertain AND refresh_lease_id IS NULL"),
            ("platform_sessions", "revoked_at IS NOT NULL"), ("platform_session_refresh_tokens", "consumed_at IS NOT NULL"),
            ("customer_portal_contexts", "revoked_at IS NOT NULL"), ("platform_app_capabilities", "revoked"),
            ("console_login_flows", "consumed_at IS NOT NULL"), ("console_browsers", "current_session_id IS NULL"),
            ("context_selections", "resource_revoked_at IS NOT NULL"),
            ("auth_client_credentials", "revoked_at IS NOT NULL"),
            ("auth_totp_recovery_codes", "consumed_at IS NOT NULL"))
        for table, condition in tables:
            for row in before[kind][table]:
                predicates[kind].append("EXISTS(SELECT 1 FROM public." + identifier(table)
                                        + " WHERE " + where(kind, table, row) + " AND " + condition + ")")
        prefix = "platform" if kind == "platform" else "portal"
        for suffix in ("sources", "receipts"):
            table = prefix + "_backchannel_logout_" + suffix
            for row in latest["databases"][kind][table]:
                spec = TABLES[kind][table]
                exact = " AND ".join(identifier(column) + " IS NOT DISTINCT FROM "
                                     + literal(row[column], field.pg_type) for column, field in spec.fields.items())
                predicates[kind].append("EXISTS(SELECT 1 FROM public." + identifier(table) + " WHERE " + exact + ")")
        browser = "console_browsers" if kind == "platform" else "portal_browsers"
        new_browsers = indexed(kind, browser, latest["databases"][kind][browser])
        for row in before[kind][browser]:
            floor = greater(row, new_browsers.get((row["id"],)), "generation", 1)
            predicates[kind].append("EXISTS(SELECT 1 FROM public." + identifier(browser) + " WHERE "
                                    + where(kind, browser, row) + " AND generation>=" + str(floor) + ")")
    for table, floors, field, allowed in (
            ("users", requirements["user_floors"], "session_version", set(allowed_rearmed_users)),
            ("memberships", requirements["membership_floors"], "identity_version", set(allowed_rearmed_memberships))):
        for object_id, floor in floors.items():
            status = "ENABLED" if object_id in allowed else "DISABLED"
            predicates["platform"].append("EXISTS(SELECT 1 FROM public." + identifier(table) + " WHERE id="
                                          + literal(object_id) + " AND " + field + ">=" + str(floor)
                                          + " AND status=" + literal(status) + ")")
    for table in ("identity_service_clients", "context_selection_services", "platform_resource_services"):
        allowed = set(allowed_rearmed_services.get(table, ()))
        for row in before["platform"][table]:
            predicates["platform"].append("EXISTS(SELECT 1 FROM public." + identifier(table) + " WHERE "
                                          + where("platform", table, row) + " AND status="
                                          + literal("ENABLED" if row["id"] in allowed else "DISABLED") + ")")
    # Required generation floors are checked independently of status. A future
    # explicit rearm cannot restore the old generation while setting ENABLED.
    for row in before["platform"]["auth_clients"]:
        object_id = row["id"]
        new = indexed("platform", "auth_clients", latest["databases"]["platform"]["auth_clients"]).get((object_id,))
        safe = object_id in requirements["eligible_clients"]
        status = row["status"] if row["status"] == "RETIRED" else "ENABLED" if safe else "DISABLED"
        predicates["platform"].append("EXISTS(SELECT 1 FROM auth_clients WHERE id=" + literal(object_id)
                                      + " AND status=" + literal(status) + " AND policy_version>="
                                      + str(requirements["client_floors"][object_id])
                                      + " AND version>=" + str(greater(row, new, "version", 1)) + ")")
        if object_id in allowed_rearmed_clients and row["client_type"] == "CONFIDENTIAL":
            floor = requirements["credential_floors"][object_id]
            require(floor < (1 << 63) - 2, "GENERATION_EXHAUSTED")
            predicates["platform"].append("EXISTS(SELECT 1 FROM auth_client_credentials WHERE client_id="
                                          + literal(object_id) + " AND client_type='CONFIDENTIAL'"
                                          + " AND revoked_at IS NULL AND secret_digest IS NOT NULL"
                                          + " AND (expires_at IS NULL OR expires_at>clock_timestamp())"
                                          + " AND credential_version>" + str(floor) + ")")
    for row in before["platform"]["auth_consents"]:
        new = indexed("platform", "auth_consents", latest["databases"]["platform"]["auth_consents"]).get((row["id"],))
        condition = "status='REVOKED' AND revoked_at IS NOT NULL AND generation>="
        condition += str(requirements["consent_floors"][row["id"]])
        condition += " AND version>=" + str(greater(row, new, "version", 1))
        condition += " AND group_version>=" + str(greater(row, new, "group_version"))
        if new is not None and new["display_deleted_at"] is not None:
            condition += " AND display_deleted_at IS NOT NULL"
        predicates["platform"].append("EXISTS(SELECT 1 FROM auth_consents WHERE id=" + literal(row["id"])
                                      + " AND " + condition + ")")
    for table in ("auth_grants", "auth_refresh_families"):
        current = indexed("platform", table, latest["databases"]["platform"][table])
        for row in before["platform"][table]:
            predicates["platform"].append("EXISTS(SELECT 1 FROM public." + identifier(table) + " WHERE "
                                          + where("platform", table, row) + " AND version>="
                                          + str(greater(row, current.get((row["id"],)), "version", 1)) + ")")
    for kind, table in (("platform", "bff_sessions"), ("portal", "portal_sessions")):
        current = indexed(kind, table, latest["databases"][kind][table])
        for row in before[kind][table]:
            new = current.get((row["id"],))
            conditions = ["version>=" + str(greater(row, new, "version", 1))]
            source = new or row
            for column in TABLES[kind][table].immutable:
                field = TABLES[kind][table].fields[column]
                if not field.redacted:
                    conditions.append(identifier(column) + " IS NOT DISTINCT FROM " + literal(source[column], field.pg_type))
            for column in ("logout_request_id", "logout_scope", "logout_confirmed_at"):
                conditions.append(identifier(column) + " IS NOT DISTINCT FROM " + literal(source[column]))
            predicates[kind].append("EXISTS(SELECT 1 FROM public." + identifier(table) + " WHERE "
                                    + where(kind, table, row) + " AND " + " AND ".join(conditions) + ")")
    for table in ("auth_delivery_jobs", "auth_consent_revocation_outbox", "auth_security_notification_outbox"):
        for row in latest["databases"]["platform"][table]:
            spec = TABLES["platform"][table]
            conditions = ["attempt_count>=" + str(greater(row, None, "attempt_count"))]
            for column in spec.immutable:
                if column in ("consent_event_id",) or spec.fields[column].redacted:
                    continue
                conditions.append(identifier(column) + " IS NOT DISTINCT FROM "
                                  + literal(row[column], spec.fields[column].pg_type))
            if table == "auth_delivery_jobs":
                if row["consent_event_id"] is not None:
                    conditions.append("consent_event_id=" + literal(row["consent_event_id"]))
                if row["state"] in TERMINAL:
                    conditions.extend(identifier(column) + " IS NOT DISTINCT FROM "
                                      + literal(row[column], spec.fields[column].pg_type) for column in JOB_STATE)
                else:
                    conditions.append("state IN('TERMINAL','DEGRADED') AND lease_id IS NULL AND lease_until IS NULL")
            elif row["delivery_state"] in TERMINAL:
                conditions.append("delivery_state=" + literal(row["delivery_state"]))
                if row["delivered_at"] is not None:
                    conditions.append("delivered_at IS NOT NULL")
                if row["terminal_at"] is not None:
                    conditions.append("terminal_at IS NOT NULL")
            predicates["platform"].append("EXISTS(SELECT 1 FROM public." + identifier(table) + " WHERE "
                                          + where("platform", table, row) + " AND " + " AND ".join(conditions) + ")")
    current_jobs = indexed("platform", "auth_delivery_jobs", latest["databases"]["platform"]["auth_delivery_jobs"])
    for row in before["platform"]["auth_delivery_jobs"]:
        new = current_jobs.get((row["id"],))
        predicates["platform"].append("EXISTS(SELECT 1 FROM auth_delivery_jobs WHERE id=" + literal(row["id"])
                                      + " AND attempt_count>=" + str(greater(row, new, "attempt_count"))
                                      + " AND max_attempts=" + str(row["max_attempts"])
                                      + " AND state IN('DELIVERED','TERMINAL','DEGRADED'))")
    # The live set may only contain the authenticated intersection; a missing
    # DELETE or a concurrent added role/permission cannot pass the start gate.
    for table in SET_POLICIES:
        current = indexed("platform", table, latest["databases"]["platform"][table])
        kept = []
        for row in before["platform"][table]:
            new = current.get(key("platform", table, row))
            predicate = where("platform", table, row)
            if semantic_equal(row, new, POLICY_IGNORE):
                kept.append(predicate)
                conditions = [predicate]
                for column, field in TABLES["platform"][table].fields.items():
                    if column in POLICY_IGNORE or field.redacted:
                        continue
                    conditions.append(identifier(column) + " IS NOT DISTINCT FROM " + literal(row[column], field.pg_type))
                if "version" in row:
                    conditions.append("version>=" + str(greater(row, new, "version")))
                predicates["platform"].append("EXISTS(SELECT 1 FROM public." + identifier(table) + " WHERE "
                                              + " AND ".join(conditions) + ")")
            else:
                predicates["platform"].append("NOT EXISTS(SELECT 1 FROM public." + identifier(table) + " WHERE "
                                              + predicate + ")")
        allowed = " OR ".join("(" + predicate + ")" for predicate in kept) or "false"
        predicates["platform"].append("NOT EXISTS(SELECT 1 FROM public." + identifier(table)
                                      + " WHERE NOT(" + allowed + "))")
    app_bindings = indexed("platform", "platform_auth_applications", latest["databases"]["platform"]["platform_auth_applications"])
    changed_apps = {row["application_id"] for row in before["platform"]["platform_auth_applications"]
                    if not semantic_equal(row, app_bindings.get((row["application_id"],)))}
    for table, (field, enabled, disabled, version) in STATUS_POLICIES.items():
        if table == "context_selection_registrations":
            continue
        current = indexed("platform", table, latest["databases"]["platform"][table])
        for row in before["platform"][table]:
            new = current.get(key("platform", table, row))
            safe = semantic_equal(row, new, POLICY_IGNORE) and (enabled is None or row[field] == enabled)
            status = row[field] if safe or row[field] in ("RETIRED", "DISABLED", "SUSPENDED") else disabled
            if table in ("applications", "auth_applications") and row.get("id", row.get("application_id")) in changed_apps:
                status = "DISABLED" if table == "applications" else "RETIRED" if row[field] == "RETIRED" else "SUSPENDED"
            conditions = [where("platform", table, row), identifier(field) + " IS NOT DISTINCT FROM " + literal(status)]
            if version:
                conditions.append(identifier(version) + ">=" + str(greater(row, new, version, 0 if safe else 1)))
            predicates["platform"].append("EXISTS(SELECT 1 FROM public." + identifier(table) + " WHERE "
                                          + " AND ".join(conditions) + ")")
    expected_registrations = registration_targets(before["platform"], latest["databases"]["platform"], requirements,
                                                  allowed_rearmed_clients, allowed_rearmed_services)
    registration_predicates = []
    for row in expected_registrations.values():
        spec = TABLES["platform"]["context_selection_registrations"]
        exact = " AND ".join(identifier(column) + " IS NOT DISTINCT FROM " + literal(row[column], field.pg_type)
                             for column, field in spec.fields.items())
        registration_predicates.append("(" + exact + ")")
        predicates["platform"].append("EXISTS(SELECT 1 FROM public.context_selection_registrations WHERE " + exact + ")")
        if row["enabled"]:
            predicates["platform"].append("EXISTS(SELECT 1 FROM public.auth_clients WHERE id="
                + literal(row["client_id"]) + " AND status='ENABLED' AND policy_version="
                + str(row["client_policy_version"]) + ")")
            predicates["platform"].append("EXISTS(SELECT 1 FROM public.applications WHERE id="
                + literal(row["application_id"]) + " AND status='ENABLED' AND access_policy_version="
                + str(row["application_policy_version"]) + " AND " + literal(row["member_type"])
                + "=ANY(allowed_member_types))")
    predicates["platform"].append("NOT EXISTS(SELECT 1 FROM public.context_selection_registrations WHERE NOT("
                                  + (" OR ".join(registration_predicates) or "false") + "))")
    # Denial targets remain denied even though the DB trigger alone permits a
    # REVOKED->ACTIVE update. No policy guess or missing object is re-enabled.
    current = indexed("platform", "auth_delivery_targets", latest["databases"]["platform"]["auth_delivery_targets"])
    for row in before["platform"]["auth_delivery_targets"]:
        new = current.get((row["id"],))
        safe = semantic_equal(row, new, POLICY_IGNORE) and row["status"] == "ACTIVE"
        predicates["platform"].append("EXISTS(SELECT 1 FROM auth_delivery_targets WHERE id=" + literal(row["id"])
                                      + " AND status=" + literal("ACTIVE" if safe else "REVOKED")
                                      + " AND version>=" + str(greater(row, new, "version", 0 if safe else 1)) + ")")
    for table in RECEIPTS + SOURCE_FACTS:
        for row in latest["databases"]["platform"][table]:
            spec = TABLES["platform"][table]
            conditions = []
            original_raw = None
            if raw_latest is not None and table in raw_latest.get("platform", {}):
                original_raw = _raw_original("platform", table, row, raw_latest)
            for column, field in spec.fields.items():
                if field.redacted:
                    # Exact original raw values are only in the protected
                    # encrypted delta; no general JSON hash is guessed in SQL.
                    if original_raw is not None:
                        conditions.append(identifier(field.source) + " IS NOT DISTINCT FROM "
                                          + literal(original_raw[field.source], field.pg_type))
                else:
                    conditions.append(identifier(column) + " IS NOT DISTINCT FROM "
                                      + literal(row[column], field.pg_type))
            predicates["platform"].append("EXISTS(SELECT 1 FROM public." + identifier(table) + " WHERE "
                                          + " AND ".join(conditions) + ")")
    predicates["platform"].extend((
        "NOT EXISTS(SELECT 1 FROM auth_delivery_jobs WHERE state IN('PENDING','LEASED'))",
        "NOT EXISTS(SELECT 1 FROM platform_account_command_intents WHERE state IN('PENDING_SECRET','UNKNOWN'))",
        "NOT EXISTS(SELECT 1 FROM platform_auth_command_intents)",
        "NOT EXISTS(SELECT 1 FROM auth_invitations WHERE status='ACTIVE')",
        "NOT EXISTS(SELECT 1 FROM auth_invitation_verifications WHERE status='ACTIVE')",
        "NOT EXISTS(SELECT 1 FROM auth_totp_enrollments)",
        "NOT EXISTS(SELECT 1 FROM auth_totp_factors)",
        "NOT EXISTS(SELECT 1 FROM platform_auth_release_bindings WHERE enabled)",
        "(SELECT count(*) FROM auth_signing_keys WHERE active AND retired_at IS NULL AND revoked_at IS NULL)=1",
        "EXISTS(SELECT 1 FROM auth_signing_keys WHERE active AND kid=" + literal(requirements["active_kid"]) + ")",
    ))
    for row in latest["databases"]["platform"]["auth_signing_keys"]:
        condition = "kid=" + literal(row["kid"]) + " AND n=" + literal(row["n"]) + " AND e=" + literal(row["e"])
        if row["revoked_at"] is not None:
            condition += " AND NOT active AND revoked_at IS NOT NULL"
        if row["retired_at"] is not None:
            condition += " AND NOT active AND retired_at IS NOT NULL"
        if _key_window(row) is not None:
            condition += " AND last_signed_exp>=" + literal(_key_window(row))
        predicates["platform"].append("EXISTS(SELECT 1 FROM auth_signing_keys WHERE " + condition + ")")
    return {kind: "SELECT (" + " AND ".join(clauses or ["true"]) + ")::boolean;"
            for kind, clauses in predicates.items()}
