#!/usr/bin/env python3
"""Offline identity-aware recovery primitives; imports never perform I/O.

Only task-owned, explicitly bound databases are accepted by the rehearsal.
Production operators must provide an independently durable current witness and
explicit reset authorization; a local checkpoint does not establish PITR/SLOs.
"""
import base64
import datetime
import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import stat
import subprocess
import time
import urllib.parse
import unicodedata
import uuid


DOMAIN = b"saas-identity/dual-db-recovery-backup/1\x00"
ITERATIONS = 210000
MAX_FILE = 512 * 1024 * 1024
MAX_VERSION = 9223372036854775806
IDENTIFIER = re.compile(r"[A-Za-z_][A-Za-z0-9_]{0,62}\Z")
SHA = re.compile(r"[0-9a-f]{64}\Z")
ISSUER = "https://auth.saas.test"
# The task cluster's bridge address reaches its existing SCRAM rule. Loopback
# administration is retained; runtime password acceptance is proved separately.
TASK_PG_HOSTS = frozenset(("127.0.0.1", "localhost", "::1", "172.20.0.2"))

INVENTORY = {
    "users": ("id,status,is_super_admin,session_version", "id"),
    "memberships": ("id,user_id,organization_id,member_type,status,identity_version", "id"),
    "tenants": ("id,status,expires_at", "id"),
    "organizations": ("id,tenant_id,parent_id,status,expires_at", "id"),
    "oidc_clients": ("id,purpose,allowed_member_types,redirect_uris,post_logout_redirect_uris,refresh_enabled,status,access_policy_version", "id"),
    "oauth_clients": ("id,client_id,subject_mode,allowed_member_types,status,access_policy_version,application_id", "id"),
    "applications": ("id,allowed_member_types,status,access_policy_version", "id"),
    "roles": ("id,status", "id"),
    "user_roles": ("user_id,role_id,organization_id", "user_id,role_id"),
    "role_menus": ("role_id,menu_id", "role_id,menu_id"),
    "membership_data_grants": ("id,membership_id,application_id,resource_code,action_code,scope_type", "id"),
    "tenant_applications": ("tenant_id,application_id", "tenant_id,application_id"),
    "organization_applications": ("organization_id,application_id", "organization_id,application_id"),
    "tenant_menus": ("tenant_id,menu_id", "tenant_id,menu_id"),
    "organization_menus": ("organization_id,menu_id", "organization_id,menu_id"),
    "identity_account_tombstones": ("user_id,operation_id,retired_at", "user_id"),
}

INVALIDATION = {
    "sso_sessions": "UPDATE public.sso_sessions SET revoked_at=clock_timestamp() WHERE revoked_at IS NULL",
    "sso_login_challenges": "UPDATE public.sso_login_challenges SET consumed_at=clock_timestamp() WHERE consumed_at IS NULL",
    "oidc_authorization_requests": "UPDATE public.oidc_authorization_requests SET consumed_at=clock_timestamp() WHERE consumed_at IS NULL",
    "oidc_authorization_codes": "UPDATE public.oidc_authorization_codes SET consumed_at=clock_timestamp() WHERE consumed_at IS NULL",
    "oidc_grants": "UPDATE public.oidc_grants SET revoked_at=clock_timestamp() WHERE revoked_at IS NULL",
    "oidc_access_tokens": "UPDATE public.oidc_access_tokens SET revoked_at=clock_timestamp() WHERE revoked_at IS NULL",
    "oidc_refresh_tokens": "UPDATE public.oidc_refresh_tokens SET revoked_at=clock_timestamp() WHERE revoked_at IS NULL",
    "platform_sessions": "UPDATE public.platform_sessions SET revoked_at=clock_timestamp() WHERE revoked_at IS NULL",
    "platform_session_refresh_tokens": "UPDATE public.platform_session_refresh_tokens SET consumed_at=clock_timestamp() WHERE consumed_at IS NULL",
    "oauth_tokens": "UPDATE public.oauth_tokens SET revoked=true WHERE NOT revoked",
    "oauth_authorization_codes": "UPDATE public.oauth_authorization_codes SET consumed=true WHERE NOT consumed",
    "oauth_authorization_requests": "UPDATE public.oauth_authorization_requests SET consumed=true WHERE NOT consumed",
    "oauth_device_codes": "UPDATE public.oauth_device_codes SET status='EXPIRED' WHERE status IN ('PENDING','APPROVED')",
    "bff_sessions": "UPDATE public.bff_sessions SET revoked_at=clock_timestamp() WHERE revoked_at IS NULL",
    "customer_portal_contexts": "UPDATE public.customer_portal_contexts SET revoked_at=clock_timestamp() WHERE revoked_at IS NULL",
    "console_login_flows": "UPDATE public.console_login_flows SET consumed_at=clock_timestamp() WHERE consumed_at IS NULL",
    "console_browsers": "UPDATE public.console_browsers SET current_session_id=NULL,generation=generation+1",
    "platform_account_command_intents": "UPDATE public.platform_account_command_intents SET state='ABANDONED' WHERE state IN ('PENDING_SECRET','UNKNOWN')",
    "platform_oidc_command_intents": "DELETE FROM public.platform_oidc_command_intents",
}
PORTAL_INVALIDATION = {
    "portal_sessions": "UPDATE public.portal_sessions SET revoked_at=clock_timestamp() WHERE revoked_at IS NULL",
    "portal_login_flows": "UPDATE public.portal_login_flows SET consumed_at=clock_timestamp() WHERE consumed_at IS NULL",
    "portal_browsers": "UPDATE public.portal_browsers SET current_session_id=NULL,generation=generation+1",
}


class RecoveryError(Exception):
    """Only a fixed category is allowed across the private/report boundary."""


def require(condition, category):
    if not condition:
        raise RecoveryError(category)


def identifier(value):
    require(isinstance(value, str) and IDENTIFIER.fullmatch(value), "RECOVERY_IDENTIFIER_INVALID")
    return '"' + value + '"'


def canonical_v7(value):
    try:
        parsed = uuid.UUID(value)
    except (ValueError, TypeError, AttributeError):
        raise RecoveryError("RECOVERY_UUID_INVALID") from None
    require(parsed.version == 7 and parsed.variant == uuid.RFC_4122 and str(parsed) == value,
            "RECOVERY_UUID_INVALID")
    return value


def literal(value):
    if value is None:
        return "NULL"
    if type(value) is bool:
        return "true" if value else "false"
    if type(value) is int:
        require(0 <= value <= MAX_VERSION, "RECOVERY_INTEGER_INVALID")
        return str(value)
    if isinstance(value, list):
        require(len(value) <= 1000 and all(isinstance(item, str) for item in value), "RECOVERY_ARRAY_INVALID")
        return "ARRAY[" + ",".join(literal(item) for item in value) + "]::text[]"
    require(isinstance(value, str) and len(value) <= 16384 and "\x00" not in value,
            "RECOVERY_LITERAL_INVALID")
    # Explicit E strings remain safe even if a role changed the server's
    # standard_conforming_strings setting. SQL text is never shell text.
    return "E'" + value.replace("\\", "\\\\").replace("'", "''") + "'"


def canonical(value):
    return json.dumps(value, ensure_ascii=True, sort_keys=True, allow_nan=False,
                      separators=(",", ":")).encode("ascii")


def sha(value):
    return hashlib.sha256(canonical(value)).hexdigest()


def private_directory(path):
    path = Path(path)
    info = path.lstat()
    require(stat.S_ISDIR(info.st_mode) and not path.is_symlink()
            and path.resolve(strict=True) == path and stat.S_IMODE(info.st_mode) == 0o700,
            "RECOVERY_PRIVATE_DIRECTORY_INVALID")
    return path


def private_read(path, maximum=MAX_FILE):
    path = Path(path)
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and not path.is_symlink() and path.resolve(strict=True) == path
            and stat.S_IMODE(info.st_mode) == 0o600 and 0 <= info.st_size <= maximum,
            "RECOVERY_PRIVATE_FILE_INVALID")
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(descriptor, "rb") as stream:
        value = stream.read(maximum + 1)
    require(len(value) <= maximum, "RECOVERY_PRIVATE_FILE_TOO_LARGE")
    return value


def private_write(path, value, replace=False):
    path = Path(path)
    private_directory(path.parent)
    require(isinstance(value, bytes) and len(value) <= MAX_FILE, "RECOVERY_PRIVATE_WRITE_INVALID")
    target = path.parent / ("." + path.name + "." + secrets.token_hex(8)) if replace else path
    descriptor = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "wb") as output:
        output.write(value)
        output.flush()
        os.fsync(output.fileno())
    if replace:
        if path.exists():
            private_read(path)
        os.replace(target, path)
    descriptor = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def remove_raw(path, directory):
    """Cleanup never reads a dump or refuses it merely for being oversized."""
    path, directory = Path(path), private_directory(directory)
    require(path.parent == directory, "RECOVERY_RAW_CLEANUP_PARENT_INVALID")
    try:
        info = path.lstat()
    except FileNotFoundError:
        return
    require(stat.S_ISREG(info.st_mode) and not path.is_symlink()
            and stat.S_IMODE(info.st_mode) == 0o600
            and (not hasattr(os, "getuid") or info.st_uid == os.getuid()),
            "RECOVERY_RAW_CLEANUP_PATH_INVALID")
    path.unlink()
    require(not path.exists(), "RECOVERY_RAW_CLEANUP_FAILED")


def inventory(db):
    output = {}
    db.query("SET TIME ZONE 'UTC'")
    db.query("SET DateStyle='ISO,YMD'")
    for family, (columns, ordering) in INVENTORY.items():
        rows = db.query("SELECT row_to_json(s)::text FROM (SELECT " + columns
                        + " FROM public." + family + " ORDER BY " + ordering + ")s")
        output[family] = [json.loads(row[0]) for row in rows]
        require(len(rows) <= 100000, "RECOVERY_INVENTORY_TOO_LARGE")
    return output


def counted(db, statement):
    require(statement.startswith(("UPDATE public.", "DELETE FROM public.", "INSERT INTO public.")),
            "RECOVERY_MUTATION_INVALID")
    return int(db.query("WITH affected AS (" + statement + " RETURNING 1) SELECT count(*)::text FROM affected")[0][0])


def fence_and_invalidate(platform, portal, latest, recovery_epoch):
    """Never combine disabling with the external generation fence statement."""
    canonical_v7(recovery_epoch)
    current = inventory(platform)
    highwater = latest["account_highwater"]
    require(all(row["id"] in highwater for row in current["users"]), "RECOVERY_HIGHWATER_INCOMPLETE")
    floors = {}
    platform.query("BEGIN")
    try:
        invalid_passwords = counted(platform, "UPDATE public.users SET password_hash="
            + literal("!recovery-disabled:" + recovery_epoch) + ",status='DISABLED',updated_at=clock_timestamp()")
        require(invalid_passwords == len(current["users"]), "RECOVERY_PASSWORD_INVALIDATION_SUPPRESSED")
        for row in current["users"]:
            floor = max(row["session_version"], highwater[row["id"]]) + 1
            require(floor <= MAX_VERSION, "RECOVERY_GENERATION_EXHAUSTED")
            # The first disable may itself increment OLD+1. Compare it as well.
            actual = int(platform.query("SELECT session_version::text FROM public.users WHERE id=" + literal(row["id"]))[0][0])
            floor = max(floor, actual + 1)
            require(floor <= MAX_VERSION, "RECOVERY_GENERATION_EXHAUSTED")
            require(counted(platform, "UPDATE public.users SET session_version=" + str(floor)
                    + " WHERE id=" + literal(row["id"])) == 1, "RECOVERY_FENCE_WRITE_SUPPRESSED")
            floors[row["id"]] = floor
        counts = {name: counted(platform, statement) for name, statement in INVALIDATION.items()}
        require(platform.query("SELECT count(*)::text FROM public.users WHERE status<>'DISABLED' OR password_hash<>"
                + literal("!recovery-disabled:" + recovery_epoch)) == [["0"]], "RECOVERY_PASSWORD_INVALIDATION_INCOMPLETE")
        platform.query("COMMIT")
    except BaseException:
        platform.query("ROLLBACK")
        raise
    portal.query("BEGIN")
    try:
        portal_counts = {name: counted(portal, statement) for name, statement in PORTAL_INVALIDATION.items()}
        portal.query("COMMIT")
    except BaseException:
        portal.query("ROLLBACK")
        raise
    assert_no_live_credentials(platform, portal)
    return {"passwords_invalidated": invalid_passwords, "account_floors": floors,
            "platform_credentials": counts, "portal_credentials": portal_counts}


def assert_no_live_credentials(platform, portal):
    conditions = {
        "sso_sessions": "revoked_at IS NULL", "sso_login_challenges": "consumed_at IS NULL",
        "oidc_authorization_requests": "consumed_at IS NULL", "oidc_authorization_codes": "consumed_at IS NULL",
        "oidc_grants": "revoked_at IS NULL", "oidc_access_tokens": "revoked_at IS NULL", "oidc_refresh_tokens": "revoked_at IS NULL",
        "platform_sessions": "revoked_at IS NULL", "platform_session_refresh_tokens": "consumed_at IS NULL",
        "oauth_tokens": "NOT revoked", "oauth_authorization_codes": "NOT consumed", "oauth_authorization_requests": "NOT consumed",
        "oauth_device_codes": "status IN ('PENDING','APPROVED')", "bff_sessions": "revoked_at IS NULL",
        "customer_portal_contexts": "revoked_at IS NULL", "console_login_flows": "consumed_at IS NULL",
        "console_browsers": "current_session_id IS NOT NULL",
        "platform_account_command_intents": "state IN ('PENDING_SECRET','UNKNOWN')", "platform_oidc_command_intents": "true",
    }
    for family, condition in conditions.items():
        require(platform.query("SELECT count(*)::text FROM public." + family + " WHERE " + condition) == [["0"]],
                "RECOVERY_LIVE_" + family.upper() + "_REMAINS")
    for family, condition in {"portal_sessions": "revoked_at IS NULL", "portal_login_flows": "consumed_at IS NULL",
                              "portal_browsers": "current_session_id IS NOT NULL"}.items():
        require(portal.query("SELECT count(*)::text FROM public." + family + " WHERE " + condition) == [["0"]],
                "RECOVERY_LIVE_" + family.upper() + "_REMAINS")


def earlier_expiry(left, right):
    for item in (left, right):
        if item is not None:
            try:
                parsed = datetime.datetime.fromisoformat(item.replace("Z", "+00:00"))
                require(parsed.tzinfo is not None, "RECOVERY_EXPIRY_INVALID")
            except (TypeError, ValueError, AttributeError):
                raise RecoveryError("RECOVERY_EXPIRY_INVALID") from None
    if left is None:
        return right
    if right is None:
        return left
    try:
        a, b = (datetime.datetime.fromisoformat(item.replace("Z", "+00:00")) for item in (left, right))
        require(a.tzinfo is not None and b.tzinfo is not None, "RECOVERY_EXPIRY_INVALID")
    except (TypeError, ValueError):
        raise RecoveryError("RECOVERY_EXPIRY_INVALID") from None
    return left if a <= b else right


def deny_plan(restored, latest):
    """Pure deny-only plan. It never inserts post-checkpoint permissions."""
    mutations, counts = [], {}

    def add(family, sql):
        mutations.append(sql)
        counts[family] = counts.get(family, 0) + 1

    for family in ("tenants", "organizations", "roles"):
        current = {row["id"]: row for row in latest[family]}
        for old in restored[family]:
            new = current.get(old["id"])
            enabled = new is not None and old["status"] == new["status"] == "ENABLED"
            if family == "organizations" and new is not None:
                enabled = enabled and (old["tenant_id"], old["parent_id"]) == (new["tenant_id"], new["parent_id"])
            sets = {"status": "ENABLED" if enabled else "DISABLED"}
            if family != "roles":
                sets["expires_at"] = earlier_expiry(old["expires_at"], new["expires_at"]) if new else old["expires_at"]
            add(family, "UPDATE public." + family + " SET " + ",".join(key + "=" + literal(value) for key, value in sets.items())
                + " WHERE id=" + literal(old["id"]))
    current = {row["id"]: row for row in latest["memberships"]}
    for old in restored["memberships"]:
        new = current.get(old["id"])
        if new is None:
            add("memberships_deleted", "DELETE FROM public.memberships WHERE id=" + literal(old["id"]))
            continue
        require(all(old[key] == new[key] for key in ("user_id", "organization_id", "member_type")),
                "RECOVERY_MEMBER_IDENTITY_MISMATCH")
        enabled = old["status"] == new["status"] == "ENABLED"
        changed = old["status"] != new["status"] or old["identity_version"] != new["identity_version"]
        version = max(old["identity_version"], new["identity_version"]) + (1 if changed else 0)
        require(version <= MAX_VERSION, "RECOVERY_MEMBER_GENERATION_EXHAUSTED")
        add("memberships", "UPDATE public.memberships SET status=" + literal("ENABLED" if enabled else "DISABLED")
            + ",identity_version=" + str(version) + " WHERE id=" + literal(old["id"]))
    for family in ("applications", "oidc_clients", "oauth_clients"):
        current = {row["id"]: row for row in latest[family]}
        for old in restored[family]:
            new = current.get(old["id"])
            enabled = new is not None and old["status"] == new["status"] == "ENABLED"
            # The schema's valid_member_type_set requires this exact order;
            # lexical sorting would put CUSTOMER first and fail its CHECK.
            allowed = [member for member in ("EMPLOYEE", "CUSTOMER")
                       if new and member in old["allowed_member_types"] and member in new["allowed_member_types"]]
            machine = family == "oauth_clients" and old["subject_mode"] == "MACHINE"
            if not allowed and not machine:
                enabled = False
            sets = {"allowed_member_types": allowed if allowed or machine else old["allowed_member_types"],
                    "status": "ENABLED" if enabled else "DISABLED"}
            if family == "oauth_clients" and new is not None:
                # The legacy policy version also covers scopes/grants/redirects
                # outside this narrow snapshot. A changed generation must be
                # disabled rather than restoring an unobserved wider policy.
                if (old["client_id"], old["subject_mode"], old["application_id"], old["access_policy_version"]) != (new["client_id"], new["subject_mode"], new["application_id"], new["access_policy_version"]):
                    sets["status"] = "DISABLED"
            if family == "oidc_clients":
                redirects = sorted(set(old["redirect_uris"]) & set(new["redirect_uris"])) if new else []
                sets["redirect_uris"] = redirects or old["redirect_uris"]
                sets["post_logout_redirect_uris"] = sorted(set(old["post_logout_redirect_uris"]) & set(new["post_logout_redirect_uris"])) if new else []
                sets["refresh_enabled"] = bool(new and old["refresh_enabled"] and new["refresh_enabled"])
                if not redirects or new is not None and old["purpose"] != new["purpose"]:
                    sets["status"] = "DISABLED"
            version = max(old["access_policy_version"], new["access_policy_version"] if new else 0) + 1
            require(version <= MAX_VERSION, "RECOVERY_POLICY_GENERATION_EXHAUSTED")
            sets["access_policy_version"] = version
            add(family, "UPDATE public." + family + " SET " + ",".join(key + "=" + literal(value) for key, value in sets.items())
                + " WHERE id=" + literal(old["id"]))
    links = ("user_roles", "role_menus", "membership_data_grants", "tenant_applications", "organization_applications", "tenant_menus", "organization_menus")
    for family in links:
        retained = {canonical(row) for row in latest[family]}
        for old in restored[family]:
            if canonical(old) not in retained:
                where = " AND ".join(key + "=" + literal(value) for key, value in old.items())
                add(family, "DELETE FROM public." + family + " WHERE " + where)
    latest_users = {row["id"]: row for row in latest["users"]}
    for old in restored["users"]:
        new = latest_users.get(old["id"])
        add("super_admin", "UPDATE public.users SET is_super_admin="
            + literal(bool(new and old["is_super_admin"] and new["is_super_admin"])) + " WHERE id=" + literal(old["id"]))
    existing = {row["user_id"]: row for row in restored["identity_account_tombstones"]}
    source_ids = {row["id"] for row in restored["users"]}
    for row in latest["identity_account_tombstones"]:
        if row["user_id"] not in source_ids:
            continue
        if row["user_id"] in existing:
            require(existing[row["user_id"]] == row, "RECOVERY_RETIREMENT_TOMBSTONE_CONFLICT")
        else:
            add("tombstones", "INSERT INTO public.identity_account_tombstones(user_id,operation_id,retired_at) VALUES("
                + ",".join(literal(row[key]) for key in ("user_id", "operation_id", "retired_at")) + ")")
    # Delete dependent grant rows before their membership. The FK cascades
    # otherwise remove a planned row early and mask exact-one write checks.
    mutations = [sql for sql in mutations if not sql.startswith("DELETE FROM public.memberships WHERE")]
    mutations.extend("DELETE FROM public.memberships WHERE id=" + literal(old["id"])
        for old in restored["memberships"] if old["id"] not in {row["id"] for row in latest["memberships"]})
    return mutations, counts


def apply_deny(db, restored, latest):
    plan, counts = deny_plan(restored, latest)
    db.query("BEGIN")
    try:
        for index, statement in enumerate(plan):
            try:
                require(counted(db, statement) == 1, "RECOVERY_DENY_WRITE_SUPPRESSED")
            except Exception as error:
                # Keep the failed operation reproducible without publishing
                # SQL literals, account data or an adapter's raw diagnostic.
                match = re.match(r"(UPDATE|DELETE FROM|INSERT INTO) public\.([a-z_]+)\b", statement)
                error.recovery_mutation = {"index": index, "operation": match[1], "family": match[2],
                    "statement_sha256": hashlib.sha256(statement.encode()).hexdigest()}
                raise
        db.query("COMMIT")
    except BaseException:
        db.query("ROLLBACK")
        raise
    return counts


def hash_password(password, known, executable=None):
    require(isinstance(password, str) and len(password) >= 12 and len(password.encode()) <= 1024
            and password.strip() == password and not any(unicodedata.category(char).startswith("C") for char in password),
            "RECOVERY_PASSWORD_POLICY_INVALID")
    known.append(password)
    executable = executable or shutil.which("argon2")
    require(executable is not None and Path(executable).is_absolute(), "RECOVERY_ARGON2_UNAVAILABLE")
    salt = secrets.token_hex(16)
    # Password is stdin only. The PHC is consumed in memory and never copied to
    # SafeCapture/report. 32MiB,t=2,p=1 is not weaker than the existing default.
    try:
        result = subprocess.run([executable, salt, "-id", "-v", "13", "-m", "15", "-t", "2", "-p", "1", "-l", "32", "-e"],
                                input=password.encode(), stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                timeout=30, check=False)
    except (OSError, subprocess.TimeoutExpired):
        raise RecoveryError("RECOVERY_ARGON2_EXECUTION_FAILED") from None
    require(result.returncode == 0 and not result.stderr and len(result.stdout) <= 1024,
            "RECOVERY_ARGON2_FAILED")
    try:
        phc = result.stdout.decode("ascii").strip()
    except UnicodeError:
        raise RecoveryError("RECOVERY_ARGON2_ENCODING_INVALID") from None
    require(re.fullmatch(r"\$argon2id\$v=19\$m=32768,t=2,p=1\$[A-Za-z0-9+/]+\$[A-Za-z0-9+/]+", phc),
            "RECOVERY_ARGON2_FORMAT_INVALID")
    known.append(phc)
    return phc


def reset_accounts(db, restored, latest, passwords, known):
    initial = {row["id"]: row for row in restored["users"]}
    final = {row["id"]: row for row in latest["users"]}
    retired = {row["user_id"] for row in latest["identity_account_tombstones"]}
    for user, password in passwords.items():
        canonical_v7(user)
        require(user in initial and user in final and user not in retired
                and initial[user]["status"] == final[user]["status"] == "ENABLED",
                "RECOVERY_RESET_ACCOUNT_NOT_ALLOWED")
        phc = hash_password(password, known)
        require(counted(db, "UPDATE public.users SET password_hash=" + literal(phc)
                + ",status='ENABLED',updated_at=clock_timestamp() WHERE id=" + literal(user)) == 1,
                "RECOVERY_RESET_WRITE_SUPPRESSED")
    return {"reset_accounts": len(passwords), "all_other_accounts_remain_disabled": True,
            "algorithm": "Argon2id", "version": 19, "memory_kib": 32768, "iterations": 2, "parallelism": 1}


def pg_environment(tools, work, options, database):
    require(options["host"] in TASK_PG_HOSTS, "RECOVERY_PG_TASK_HOST_REQUIRED")
    identifier(database)
    env = tools.clean_env(work)
    env.update(PGHOST=options["host"], PGPORT=str(options.get("port", 5432)), PGUSER=options["user"],
               PGPASSWORD=options.get("password", ""), PGDATABASE=database,
               PGSSLMODE=options.get("sslmode", "prefer"), PGPASSFILE="/dev/null", PGSERVICEFILE="/dev/null",
               PGCONNECT_TIMEOUT="10", PGCLIENTENCODING="UTF8", PGAPPNAME="identity-phase13-recovery",
               PGOPTIONS="-c statement_timeout=300000 -c lock_timeout=30000")
    return env


def mac_file(path, metadata, key):
    private_directory(path.parent)
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and not path.is_symlink() and stat.S_IMODE(info.st_mode) == 0o600
            and 0 <= info.st_size <= MAX_FILE, "RECOVERY_MAC_FILE_INVALID")
    digest = hmac.new(key, DOMAIN, hashlib.sha256)
    header = canonical(metadata)
    digest.update(len(header).to_bytes(8, "big"))
    digest.update(header)
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(descriptor, "rb") as source:
        for block in iter(lambda: source.read(65536), b""):
            digest.update(block)
    return digest.hexdigest()


def encrypted_file(tools, run_job, work, raw, destination, passphrase, key, binding, decrypt=False):
    env = tools.clean_env(work)
    env["IDENTITY_RECOVERY_BACKUP_PASSPHRASE"] = passphrase
    openssl = shutil.which("openssl", path=env["PATH"])
    require(openssl is not None, "RECOVERY_OPENSSL_UNAVAILABLE")
    if decrypt:
        sidecar = json.loads(private_read(destination.with_suffix(destination.suffix + ".auth"), 1048576))
        require(sidecar["metadata"]["binding"] == binding
                and hmac.compare_digest(mac_file(destination, sidecar["metadata"], key), sidecar["tag"]),
                "RECOVERY_BACKUP_AUTHENTICATION_FAILED")
        source, output = destination, raw
    else:
        source, output = raw, destination
    private_write(output, b"")
    argv = [openssl, "enc", "-aes-256-cbc", "-salt", "-pbkdf2", "-iter", str(ITERATIONS), "-md", "sha256",
            "-pass", "env:IDENTITY_RECOVERY_BACKUP_PASSPHRASE", "-in", str(source), "-out", str(output)]
    if decrypt:
        argv.append("-d")
    job = checked_job(run_job, ("recovery-decrypt-" if decrypt else "recovery-encrypt-") + destination.stem, argv, env, 120)
    if decrypt:
        require(tools.file_sha(raw) == sidecar["metadata"]["raw_sha256"]
                and raw.stat().st_size == sidecar["metadata"]["raw_bytes"], "RECOVERY_DECRYPTED_HASH_MISMATCH")
    else:
        metadata = {"binding": binding, "raw_sha256": tools.file_sha(raw), "raw_bytes": raw.stat().st_size,
                    "encrypted_bytes": destination.stat().st_size, "cipher": "AES-256-CBC",
                    "kdf": "PBKDF2-HMAC-SHA256", "iterations": ITERATIONS}
        private_write(destination.with_suffix(destination.suffix + ".auth"), canonical({"metadata": metadata,
                      "tag": mac_file(destination, metadata, key)}))
    return {"encrypted_sha256": tools.file_sha(destination), "encrypted_bytes": destination.stat().st_size,
            "authenticated_before_decryption": decrypt, "job": job}


def database_record(admin, name, catalog):
    identifier(name)
    rows = admin.query("SELECT r.rolname,COALESCE(shobj_description(d.oid,'pg_database'),'') FROM pg_database d JOIN pg_roles r ON r.oid=d.datdba WHERE d.datname=" + literal(name))
    require(len(rows) == 1, "RECOVERY_DATABASE_BINDING_MISSING")
    return {"name": name, "owner": rows[0][0], "comment": rows[0][1], "acl": catalog.database_acl(admin, name)}


def verify_task_binding(task_id, cluster, names, markers):
    canonical_v7(task_id)
    require(cluster == "7700230159816609830" and set(names) == set(markers) == {"platform", "portal"},
            "RECOVERY_TASK_CLUSTER_OR_DATABASE_SET_INVALID")
    for kind, name in names.items():
        require(name == "identity_phase13_" + kind + "_" + uuid.UUID(task_id).hex,
                "RECOVERY_TASK_DATABASE_NAME_INVALID")
        expected = "identity-deployment/1/" + task_id if kind == "platform" else "identity_sso_phase13_portal_owned:" + task_id
        require(markers[kind] == expected, "RECOVERY_TASK_DATABASE_MARKER_INVALID")


def role_url(raw, password):
    try:
        parsed = urllib.parse.urlsplit(raw)
        port = parsed.port or 5432
    except ValueError:
        raise RecoveryError("RECOVERY_ROLE_URL_INVALID") from None
    require(parsed.scheme in {"postgres", "postgresql"} and parsed.hostname in TASK_PG_HOSTS
            and parsed.username is not None and parsed.password is not None and not parsed.fragment,
            "RECOVERY_ROLE_URL_INVALID")
    try:
        options = urllib.parse.parse_qsl(parsed.query, keep_blank_values=True, strict_parsing=True)
    except ValueError:
        raise RecoveryError("RECOVERY_ROLE_URL_QUERY_INVALID") from None
    require(len(options) <= 1 and all(key == "sslmode" and value in {"disable", "prefer", "require"} for key, value in options),
            "RECOVERY_ROLE_URL_QUERY_INVALID")
    identifier(urllib.parse.unquote(parsed.path.removeprefix("/")))
    role = urllib.parse.unquote(parsed.username)
    identifier(role)
    host = "[" + parsed.hostname + "]" if ":" in parsed.hostname else parsed.hostname
    return urllib.parse.urlunsplit(parsed._replace(netloc=urllib.parse.quote(role, safe="") + ":"
        + urllib.parse.quote(password, safe="") + "@" + host + ":" + str(port)))


PORTAL_PROJECTIONS = {
    "portal_browsers": "SELECT id::text,generation::text,COALESCE(current_session_id::text,''),created_at::text,expires_at::text FROM public.portal_browsers ORDER BY id",
    "portal_login_flows": "SELECT id::text,browser_id::text,browser_generation::text,created_at::text,expires_at::text,(consumed_at IS NOT NULL)::text FROM public.portal_login_flows ORDER BY id",
    "portal_sessions": "SELECT id::text,user_id::text,account_version::text,global_session_id::text,oidc_grant_id::text,version::text,COALESCE(membership_id::text,''),COALESCE(identity_version::text,''),(revoked_at IS NOT NULL)::text FROM public.portal_sessions ORDER BY id",
}


def data_fingerprints(db, queries, catalog):
    db.query("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
    try:
        db.query("SET LOCAL TIME ZONE 'UTC'")
        db.query("SET LOCAL DateStyle='ISO,YMD'")
        result = {name: catalog.digest_rows(db.query(query)) for name, query in queries.items()}
        db.query("COMMIT")
        return result
    except BaseException:
        db.query("ROLLBACK")
        raise


def checked_job(run_job, label, argv, env, timeout):
    result = run_job(label, argv, env, timeout=timeout)
    require(isinstance(result, dict) and result.get("exit_code") == 0,
            "RECOVERY_EXTERNAL_JOB_FAILED")
    require(all(isinstance(result.get(name), dict) and result[name].get("reader_complete") is True
                and result[name].get("credential_pattern_categories") == [] for name in ("stdout", "stderr")),
            "RECOVERY_EXTERNAL_JOB_SCAN_FAILED")
    return {key: result[key] for key in ("exit_code", "stdout", "stderr")}


def capture_checkpoint(ctx, catalog):
    tools, work = ctx["tools"], Path(ctx["work"])
    task_id = canonical_v7(work.name)
    verify_task_binding(task_id, ctx["cluster_id"], ctx["names"], ctx["markers"])
    directory = work / "recovery-private"
    directory.mkdir(mode=0o700)
    private_directory(directory)
    passphrase, mac_key = secrets.token_urlsafe(48), secrets.token_bytes(32)
    ctx["known"].extend((passphrase, base64.urlsafe_b64encode(mac_key).decode().rstrip("=")))
    private_write(directory / "backup-key.json", canonical({"passphrase": passphrase,
        "mac_key": base64.b64encode(mac_key).decode(), "task_id": task_id}))
    records, catalog_fingerprints, projections, source_inventory = {}, {}, {}, None
    admin = tools.Database(ctx["admin_options"])
    try:
        require(tools.facts(admin)["cluster_system_identifier"] == ctx["cluster_id"], "RECOVERY_CLUSTER_MISMATCH")
        for kind in ("platform", "portal"):
            name = ctx["names"][kind]
            records[kind] = database_record(admin, name, catalog)
            require(records[kind]["comment"] == ctx["markers"][kind], "RECOVERY_DATABASE_MARKER_MISMATCH")
            db = tools.Database(ctx[kind + "_options"])
            try:
                if kind == "platform":
                    deployment = db.query("SELECT migration_owner::text,issuer,schema_version::text FROM public.identity_deployment_state WHERE singleton")
                    require(deployment == [[records[kind]["owner"], ISSUER, "20"]], "RECOVERY_MIGRATION_OWNER_BINDING_INVALID")
                catalog_fingerprints[kind] = catalog.projections(db, catalog.CATALOG_QUERIES)
                queries = catalog.PROJECTIONS if kind == "platform" else PORTAL_PROJECTIONS
                projections[kind] = data_fingerprints(db, queries, catalog)
                if kind == "platform":
                    source_inventory = inventory(db)
            finally:
                db.close()
    finally:
        admin.close()
    binding = {"task_id": task_id, "cluster_id": ctx["cluster_id"], "issuer": ISSUER,
               "databases": {kind: {"name": record["name"], "marker": record["comment"]} for kind, record in records.items()},
               "database_records_sha256": sha(records), "catalog_sha256": sha(catalog_fingerprints),
               "projection_sha256": sha(projections), "checkpoint_ms": time.time_ns() // 1000000}
    jobs, archives, raw_paths = {}, {}, []
    try:
        for kind in ("platform", "portal"):
            raw = directory / (kind + ".dump")
            raw_paths.append(raw)
            private_write(raw, b"")
            env = pg_environment(tools, work, ctx[kind + "_options"], ctx["names"][kind])
            executable = shutil.which("pg_dump", path=env["PATH"])
            require(executable is not None, "RECOVERY_PG_DUMP_UNAVAILABLE")
            jobs[kind] = checked_job(ctx["run_job"], "recovery-dump-" + kind,
                [executable, "--format=custom", "--no-password", "--file=" + str(raw), "--lock-wait-timeout=30s"], env, 300)
            require(private_read(raw, MAX_FILE)[:5] == b"PGDMP", "RECOVERY_CUSTOM_DUMP_REQUIRED")
            archive = directory / (kind + ".dump.enc")
            archives[kind] = archive
            encrypted_file(tools, ctx["run_job"], work, raw, archive, passphrase, mac_key, dict(binding, kind=kind))
        materials = {}
        for label, path in ctx["private_materials"].items():
            require(re.fullmatch(r"[a-z][a-z0-9_]{0,40}", label), "RECOVERY_MATERIAL_LABEL_INVALID")
            materials[label] = {"filename": Path(path).name, "bytes_base64": base64.b64encode(private_read(Path(path), 8 * 1024 * 1024)).decode()}
        raw = directory / "materials.json"
        raw_paths.append(raw)
        private_write(raw, canonical({"materials": materials, "process_configuration": ctx["envs"]}))
        archives["materials"] = directory / "materials.enc"
        encrypted_file(tools, ctx["run_job"], work, raw, archives["materials"], passphrase, mac_key, dict(binding, kind="materials"))
        # Authenticate a real modified ciphertext, without decrypting it.
        original = archives["materials"]
        altered = directory / "tampered.enc"
        raw_paths.append(altered)
        value = private_read(original)
        private_write(altered, bytes([value[0] ^ 1]) + value[1:])
        sidecar = json.loads(private_read(original.with_suffix(".enc.auth")))
        require(not hmac.compare_digest(mac_file(altered, sidecar["metadata"], mac_key), sidecar["tag"]),
                "RECOVERY_CIPHERTEXT_TAMPER_ACCEPTED")
    finally:
        for path in raw_paths:
            if path.exists():
                remove_raw(path, directory)
    manifest = {"binding": binding, "records": records, "catalog": catalog_fingerprints,
                "projections": projections, "inventory": source_inventory,
                "archives": {key: {"filename": path.name, "sha256": tools.file_sha(path)} for key, path in archives.items()}}
    private_write(directory / "manifest.json", canonical(manifest))
    # The manifest itself is bound into an authentication sidecar. Witness
    # checkpoint binding uses this digest and cannot be exchanged across tasks.
    digest = sha(manifest)
    private_write(directory / "manifest.auth", canonical({"sha256": digest,
        "tag": hmac.new(mac_key, DOMAIN + b"manifest\x00" + canonical(manifest), hashlib.sha256).hexdigest()}))
    return {"private_directory": directory, "passphrase": passphrase, "mac_key": mac_key,
            "manifest": manifest, "checkpoint_sha256": digest,
            "report": {"scope": "TASK_OWNED_DUAL_DATABASE_CHECKPOINT", "status": "PASS", "checkpoint_sha256": digest,
                "checkpoint_ms": binding["checkpoint_ms"], "databases": list(records),
                "archives": manifest["archives"], "catalog": catalog_fingerprints,
                "data": projections, "material_labels": sorted(materials), "dump_jobs": jobs,
                "ciphertext_tamper_rejected": True, "raw_dump_files_removed": True,
                "cross_database_atomic_snapshot_claim": False, "pitr_claim": False}}


def restore_checkpoint(ctx, checkpoint, catalog, report):
    """Drop/recreate exact task databases only after caller stopped all services."""
    tools, work = ctx["tools"], Path(ctx["work"])
    manifest = checkpoint["manifest"]
    directory = private_directory(checkpoint["private_directory"])
    authentication = json.loads(private_read(directory / "manifest.auth"))
    require(sha(manifest) == authentication["sha256"] == checkpoint["checkpoint_sha256"]
            and hmac.compare_digest(authentication["tag"], hmac.new(checkpoint["mac_key"], DOMAIN + b"manifest\x00" + canonical(manifest), hashlib.sha256).hexdigest()),
            "RECOVERY_MANIFEST_AUTHENTICATION_FAILED")
    admin = tools.Database(ctx["admin_options"])
    raw_paths = []
    report["databases"] = {}
    try:
        require(tools.facts(admin)["cluster_system_identifier"] == ctx["cluster_id"], "RECOVERY_CLUSTER_MISMATCH")
        # Authenticate and decrypt every required artifact, and validate both
        # current targets, before destroying either source database.
        for kind in ("platform", "portal", "materials"):
            raw = directory / ("restored-materials.json" if kind == "materials" else "restore-" + kind + ".dump")
            raw_paths.append(raw)
            archive = directory / manifest["archives"][kind]["filename"]
            require(tools.file_sha(archive) == manifest["archives"][kind]["sha256"], "RECOVERY_ARCHIVE_HASH_MISMATCH")
            encrypted_file(tools, ctx["run_job"], work, raw, archive, checkpoint["passphrase"], checkpoint["mac_key"], dict(manifest["binding"], kind=kind), decrypt=True)
            if kind != "materials":
                require(private_read(raw)[:5] == b"PGDMP", "RECOVERY_AUTHENTICATED_DUMP_FORMAT_INVALID")
        materials = json.loads(private_read(directory / "restored-materials.json"))
        require(materials["process_configuration"] == ctx["envs"], "RECOVERY_PROCESS_CONFIGURATION_MISMATCH")
        for kind in ("platform", "portal"):
            record, name = manifest["records"][kind], ctx["names"][kind]
            require(record["name"] == name and database_record(admin, name, catalog) == record,
                    "RECOVERY_LIVE_DATABASE_OWNER_OR_MARKER_CHANGED")
            require(admin.query("SELECT count(*)::text FROM pg_stat_activity WHERE datname=" + literal(name)) == [["0"]],
                    "RECOVERY_DATABASE_STILL_ACTIVE")
        for kind in ("platform", "portal"):
            record = manifest["records"][kind]
            name = ctx["names"][kind]
            require(record["name"] == name and database_record(admin, name, catalog) == record,
                    "RECOVERY_LIVE_DATABASE_OWNER_OR_MARKER_CHANGED")
            require(admin.query("SELECT count(*)::text FROM pg_stat_activity WHERE datname=" + literal(name)) == [["0"]],
                    "RECOVERY_DATABASE_STILL_ACTIVE")
            admin.query("DROP DATABASE " + identifier(name))
            admin.query("CREATE DATABASE " + identifier(name) + " OWNER " + identifier(record["owner"]) + " TEMPLATE template0")
            admin.query("COMMENT ON DATABASE " + identifier(name) + " IS " + literal(record["comment"]))
            catalog.restore_database_acl(admin, name, record["acl"])
            raw = directory / ("restore-" + kind + ".dump")
            db = tools.Database(ctx[kind + "_options"])
            try:
                require(db.query("SELECT count(*)::text FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public'") == [["0"]]
                        and db.query("SELECT count(*)::text FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public'") == [["0"]],
                        "RECOVERY_FRESH_PUBLIC_NOT_EMPTY")
            finally:
                db.close()
            env = pg_environment(tools, work, ctx[kind + "_options"], name)
            executable = shutil.which("pg_restore", path=env["PATH"])
            require(executable is not None, "RECOVERY_PG_RESTORE_UNAVAILABLE")
            job = checked_job(ctx["run_job"], "recovery-restore-" + kind,
                [executable, "--no-password", "--exit-on-error", "--single-transaction", "--dbname=" + name, str(raw)], env, 300)
            db = tools.Database(ctx[kind + "_options"])
            try:
                actual_catalog = catalog.projections(db, catalog.CATALOG_QUERIES)
                actual_data = data_fingerprints(db, catalog.PROJECTIONS if kind == "platform" else PORTAL_PROJECTIONS, catalog)
            finally:
                db.close()
            record_equal = database_record(admin, name, catalog) == record
            report["databases"][kind] = {"restore_job": job, "catalog": actual_catalog,
                "catalog_equal": actual_catalog == manifest["catalog"][kind],
                "database_owner_comment_acl_equal": record_equal,
                "safe_data_equal": actual_data == manifest["projections"][kind],
                "differing_catalog_families": [key for key in actual_catalog if actual_catalog[key] != manifest["catalog"][kind][key]]}
            require(record_equal and actual_catalog == manifest["catalog"][kind], "RECOVERY_OWNER_OR_ACL_MISMATCH")
            require(actual_data == manifest["projections"][kind], "RECOVERY_RESTORED_DATA_MISMATCH")
        restored_directory = directory / "restored-materials"
        restored_directory.mkdir(mode=0o700)
        restored_paths = {}
        for label, item in materials["materials"].items():
            path = restored_directory / (label + "-" + item["filename"])
            private_write(path, base64.b64decode(item["bytes_base64"], validate=True))
            require(private_read(path) == private_read(Path(ctx["private_materials"][label])), "RECOVERY_LIBRARY_MATERIAL_MISMATCH")
            restored_paths[label] = path
        report["library_materials_restored_equal"] = sorted(restored_paths)
        report["original_process_configuration_authenticated"] = materials["process_configuration"] == ctx["envs"]
        require(report["original_process_configuration_authenticated"], "RECOVERY_PROCESS_CONFIGURATION_MISMATCH")
        return restored_paths
    finally:
        admin.close()
        for path in raw_paths:
            if path.exists():
                remove_raw(path, directory)
        report["raw_restore_files_removed"] = all(not path.exists() for path in raw_paths)


def rotate_credentials(ctx, db, restored_paths):
    """Rotate every restored secret-bearing registration; return safe counts."""
    envs, known = ctx["envs"], ctx["known"]
    counts = {"services": 0, "oidc_rps": 0, "legacy_clients": 0, "database_runtimes": 0, "bff_keyrings": 0}
    for label, registration in ctx["services"].items():
        secret = secrets.token_hex(32)
        known.append(secret)
        require(counted(db, "UPDATE public.identity_service_clients SET secret_hash=" + literal(hashlib.sha256(secret.encode()).hexdigest())
                + " WHERE id=" + literal(registration["id"])) == 1, "RECOVERY_SERVICE_ROTATION_FAILED")
        registration["secret"] = secret
        counts["services"] += 1
        fields = {"account": "IDENTITY_SERVICE_CLIENT_SECRET", "customer": "CUSTOMER_AUTHORITY_SERVICE_CLIENT_SECRET", "admin": "IDENTITY_ADMIN_SERVICE_CLIENT_SECRET"}
        require(label in fields, "RECOVERY_SERVICE_PURPOSE_INVALID")
        envs["platform"][fields[label]] = secret
    expected_services = {registration["id"] for registration in ctx["services"].values()}
    for row in db.query("SELECT id::text FROM public.identity_service_clients ORDER BY id"):
        if row[0] not in expected_services:
            secret = secrets.token_hex(32)
            known.append(secret)
            require(counted(db, "UPDATE public.identity_service_clients SET status='DISABLED',secret_hash="
                + literal(hashlib.sha256(secret.encode()).hexdigest()) + " WHERE id=" + literal(row[0])) == 1,
                "RECOVERY_EXTRA_SERVICE_ROTATION_FAILED")
            counts["services"] += 1
    for label in ("console", "portal"):
        registration = ctx[label]
        secret = secrets.token_hex(32)
        known.append(secret)
        require(counted(db, "UPDATE public.oidc_clients SET access_policy_version=access_policy_version+1,secret_hash=" + literal(hashlib.sha256(secret.encode()).hexdigest())
                + " WHERE id=" + literal(registration["clientId"])) == 1, "RECOVERY_RP_ROTATION_FAILED")
        registration["clientSecret"] = secret
        envs["platform" if label == "console" else "portal"]["BFF_OIDC_CLIENT_SECRET" if label == "console" else "OIDC_CLIENT_SECRET"] = secret
        counts["oidc_rps"] += 1
    # Reprovisioning only the two active RPs must not leave another restored
    # confidential OIDC registration authenticatable with its old secret.
    active_ids = {ctx[label]["clientId"] for label in ("console", "portal")}
    for row in db.query("SELECT id::text FROM public.oidc_clients ORDER BY id"):
        if row[0] not in active_ids:
            secret = secrets.token_hex(32)
            known.append(secret)
            require(counted(db, "UPDATE public.oidc_clients SET access_policy_version=access_policy_version+1,secret_hash="
                + literal(hashlib.sha256(secret.encode()).hexdigest()) + " WHERE id=" + literal(row[0])) == 1,
                "RECOVERY_EXTRA_RP_ROTATION_FAILED")
    for row in db.query("SELECT id::text FROM public.oauth_clients WHERE client_type='CONFIDENTIAL' ORDER BY id"):
        secret = secrets.token_hex(32)
        known.append(secret)
        require(counted(db, "UPDATE public.oauth_clients SET client_secret_hash=" + literal(hashlib.sha256(secret.encode()).hexdigest())
                + " WHERE id=" + literal(row[0])) == 1, "RECOVERY_LEGACY_ROTATION_FAILED")
        if row[0] == ctx["profile"]["id"]:
            ctx["profile"]["clientSecret"] = secret
            envs["portal"]["CUSTOMER_RESOURCE_CLIENT_SECRET"] = secret
        ctx.setdefault("recovery_rotated_legacy", {})[row[0]] = secret
        counts["legacy_clients"] += 1
    admin = ctx["tools"].Database(ctx["admin_options"])
    try:
        runtime_labels = ("identity", "platform", "portal") + (("notification",) if "notification" in envs else ())
        for label in runtime_labels:
            old = envs[label]["DATABASE_URL"]
            ctx.setdefault("recovery_old_database_urls", {})[label] = old
            role = urllib.parse.unquote(urllib.parse.urlsplit(old).username)
            # Dedicated runtime names are pinned by the copied deployment and
            # caller task role markers; no other cluster login is modified.
            expected = db.query("SELECT identity_runtime::text,platform_runtime::text FROM public.identity_deployment_state WHERE singleton")[0]
            task_hex = uuid.UUID(Path(ctx["work"]).name).hex
            expected_role = expected[0] if label == "identity" else expected[1] if label == "platform" else "identity_p13_portal_" + task_hex if label == "portal" else "p13_notification_" + task_hex
            require(role == expected_role, "RECOVERY_DATABASE_ROLE_BINDING_INVALID")
            password = secrets.token_urlsafe(36)
            known.append(password)
            admin.query("BEGIN")
            try:
                admin.query("SET LOCAL log_statement='none'")
                admin.query("SET LOCAL log_min_error_statement='panic'")
                admin.query("ALTER ROLE " + identifier(role) + " PASSWORD " + literal(password))
                admin.query("COMMIT")
            except BaseException:
                admin.query("ROLLBACK")
                raise
            envs[label]["DATABASE_URL"] = role_url(old, password)
            known.append(envs[label]["DATABASE_URL"])
            counts["database_runtimes"] += 1
    finally:
        admin.close()
    new_jwt = secrets.token_urlsafe(48)
    known.append(new_jwt)
    envs["platform"]["JWT_SECRET"] = new_jwt
    if "notification" in envs:
        metrics = secrets.token_urlsafe(48)
        known.append(metrics)
        envs["notification"]["METRICS_TOKEN"] = metrics
    for label, key in (("console", "BFF_TOKEN_KEY_FILE"), ("portal", "PORTAL_TOKEN_KEY_FILE")):
        raw = secrets.token_bytes(32)
        encoded = base64.urlsafe_b64encode(raw).decode().rstrip("=")
        known.append(encoded)
        kid = label + "-recovery-" + secrets.token_hex(8)
        path = restored_paths[label + "_keyring"].parent / (label + "-rotated-keyring.json")
        private_write(path, canonical({"activeKid": kid, "keys": [{"kid": kid, "key": encoded}]}))
        envs["platform" if label == "console" else "portal"][key] = str(path)
        counts["bff_keyrings"] += 1
    envs["identity"]["OIDC_KEYSET_PATH"] = str(restored_paths["keyset"])
    return counts


def assert_start_allowed(ctx, store, receipt_file, key):
    """Caller invokes this immediately before the restored runtime restart."""
    envelope = json.loads(private_read(Path(receipt_file), 1048576))
    require(set(envelope) == {"receipt", "tag"}, "RECOVERY_START_RECEIPT_INVALID")
    receipt = envelope["receipt"]
    require(receipt["task_id"] == Path(ctx["work"]).name and receipt["issuer"] == ISSUER
            and hmac.compare_digest(envelope["tag"], hmac.new(key,
                b"saas-identity/recovery-start-receipt/1\x00" + canonical(receipt), hashlib.sha256).hexdigest()),
            "RECOVERY_START_RECEIPT_AUTHENTICATION_FAILED")
    latest = store.load_latest(minimum_sequence=receipt["witness_sequence"])
    witness_digest = hashlib.sha256(json.dumps(latest, sort_keys=True, ensure_ascii=False,
        separators=(",", ":"), allow_nan=False).encode("utf-8")).hexdigest()
    require(witness_digest == receipt["witness_sha256"], "RECOVERY_START_WITNESS_MISMATCH")
    db = ctx["tools"].Database(ctx["platform_options"])
    try:
        actual = inventory(db)
        require(sha(actual) == receipt["fenced_inventory_sha256"], "RECOVERY_START_DATABASE_STATE_CHANGED")
        highwater = latest["document"]["payload"]["account_highwater"]
        require(all(row["session_version"] > highwater[row["id"]] for row in actual["users"]),
                "RECOVERY_START_ACCOUNT_FLOOR_ROLLED_BACK")
    finally:
        db.close()
    return {"status": "PASS", "witness_sequence": receipt["witness_sequence"],
            "account_floor_verified": True, "database_state_verified": True}
