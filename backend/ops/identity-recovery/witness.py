"""Bounded authenticated deny snapshots outside the database restore set.

The caller owns the HMAC key and keeps this directory outside the restore set.
CURRENT and immutable chained records reject an old signed witness (including
an old CURRENT while newer records remain). Rolling back *all* external state
requires an independently retained minimum_sequence; HMAC alone cannot detect
that attack. This module does not claim remote or disaster-level durability.

Disk operations require POSIX openat/O_NOFOLLOW, private owner-only storage,
file fsync, directory fsync and an advisory single-writer lock. Unsupported
platforms fail closed. Memory encoding/verification is platform independent.
Errors contain fixed categories only; never stringify caller data or secrets.
"""
import contextlib
import hashlib
import hmac
import json
import os
import re
import secrets
import stat
import uuid
from urllib.parse import urlsplit


DOMAIN = b"saas-identity-recovery-witness/1\x00"
CURRENT_DOMAIN = b"saas-identity-recovery-current/1\x00"
FORMAT = "saas-identity-recovery-witness/1"
CURRENT_FORMAT = "saas-identity-recovery-current/1"
MAX_BYTES = 8 * 1024 * 1024
MAX_CHAIN_BYTES = 32 * 1024 * 1024
MAX_RECORDS = 1024
MAX_ROWS = 20000
MAX_VERSION = (1 << 63) - 2
ZERO = "0" * 64
HEX = re.compile(r"[0-9a-f]{64}\Z")
RECORD = re.compile(r"witness-([0-9]{8})\.json\Z")
LABEL = re.compile(r"[a-z][a-z0-9_]{0,63}\Z")
FAMILIES = {
    "users": "id status is_super_admin session_version",
    "memberships": "id user_id organization_id member_type status identity_version",
    "tenants": "id status expires_at",
    "organizations": "id tenant_id parent_id status expires_at",
    "oidc_clients": "id purpose allowed_member_types redirect_uris post_logout_redirect_uris refresh_enabled status access_policy_version",
    "oauth_clients": "id client_id subject_mode allowed_member_types status access_policy_version application_id",
    "applications": "id allowed_member_types status access_policy_version",
    "roles": "id status",
    "user_roles": "user_id role_id organization_id",
    "role_menus": "role_id menu_id",
    "membership_data_grants": "id membership_id application_id resource_code action_code scope_type",
    "tenant_applications": "tenant_id application_id",
    "tenant_menus": "tenant_id menu_id",
    "organization_applications": "organization_id application_id",
    "organization_menus": "organization_id menu_id",
    "identity_account_tombstones": "user_id operation_id retired_at",
}
FAMILIES = {name: frozenset(columns.split()) for name, columns in FAMILIES.items()}


class WitnessError(RuntimeError):
    """A fixed, safe failure category suitable for a public receipt."""


def require(condition, category):
    if not condition:
        raise WitnessError(category)


def _keys(value, expected):
    require(type(value) is dict and set(value) == set(expected), "WITNESS_FIELDS_INVALID")


def _integer(value, minimum=0, maximum=MAX_VERSION):
    require(type(value) is int and minimum <= value <= maximum, "WITNESS_INTEGER_INVALID")


def _uuid(value):
    require(type(value) is str and len(value) == 36, "WITNESS_UUID_INVALID")
    try:
        parsed = uuid.UUID(value)
    except (ValueError, AttributeError):
        raise WitnessError("WITNESS_UUID_INVALID") from None
    require(str(parsed) == value and parsed.version == 7
            and parsed.variant == uuid.RFC_4122, "WITNESS_UUID_INVALID")
    return value


def _digest(value):
    require(type(value) is str and HEX.fullmatch(value), "WITNESS_DIGEST_INVALID")


def canonical(value):
    """Canonical UTF-8 JSON, with no floats, duplicate keys or non-JSON values."""
    def visit(item, depth=0):
        require(depth <= 10, "WITNESS_DEPTH_EXCEEDED")
        if type(item) is dict:
            require(len(item) <= MAX_ROWS, "WITNESS_OBJECT_TOO_LARGE")
            for key, child in item.items():
                require(type(key) is str and len(key) <= 256, "WITNESS_KEY_INVALID")
                visit(child, depth + 1)
        elif type(item) is list:
            require(len(item) <= MAX_ROWS, "WITNESS_ARRAY_TOO_LARGE")
            for child in item:
                visit(child, depth + 1)
        else:
            require(item is None or type(item) in (str, bool, int), "WITNESS_VALUE_INVALID")
            if type(item) is str:
                require(len(item) <= 2048 and all(ord(c) >= 32 for c in item), "WITNESS_TEXT_INVALID")
            elif type(item) is int:
                _integer(item)
    visit(value)
    try:
        raw = json.dumps(value, sort_keys=True, separators=(",", ":"),
                         ensure_ascii=False, allow_nan=False).encode("utf-8")
    except (ValueError, UnicodeError, RecursionError):
        raise WitnessError("WITNESS_JSON_INVALID") from None
    require(len(raw) <= MAX_BYTES, "WITNESS_BYTES_EXCEEDED")
    return raw


def _parse(raw):
    require(type(raw) is bytes and len(raw) <= MAX_BYTES, "WITNESS_BYTES_EXCEEDED")
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, "WITNESS_DUPLICATE_KEY")
            result[key] = value
        return result
    try:
        value = json.loads(raw, object_pairs_hook=pairs)
    except (ValueError, UnicodeError, RecursionError):
        raise WitnessError("WITNESS_JSON_INVALID") from None
    require(canonical(value) == raw, "WITNESS_NONCANONICAL_JSON")
    return value


def make_binding(task_id, cluster_id, checkpoint_sha256, databases,
                 issuer="https://auth.saas.test"):
    """Bind one exact task checkpoint, cluster and two original database names."""
    _uuid(task_id)
    require(type(cluster_id) is str and re.fullmatch(r"[1-9][0-9]{0,19}", cluster_id),
            "WITNESS_CLUSTER_INVALID")
    _digest(checkpoint_sha256)
    require(issuer == "https://auth.saas.test", "WITNESS_ISSUER_INVALID")
    _keys(databases, ("platform", "portal"))
    for kind, database in databases.items():
        _keys(database, ("name", "marker"))
        require(database["name"] == "identity_phase13_" + kind + "_" + task_id.replace("-", ""),
                "WITNESS_DATABASE_INVALID")
        marker = database["marker"]
        require(type(marker) is str and 1 <= len(marker) <= 256 and task_id in marker
                and all(32 <= ord(c) <= 126 for c in marker), "WITNESS_MARKER_INVALID")
    return _parse(canonical({"task_id": task_id, "cluster_id": cluster_id,
                             "checkpoint_sha256": checkpoint_sha256,
                             "databases": databases, "issuer": issuer}))


def _binding(value):
    _keys(value, ("task_id", "cluster_id", "checkpoint_sha256", "databases", "issuer"))
    return make_binding(**value)


def validate_payload(payload):
    """Only complete, typed public security projections may enter a witness."""
    _keys(payload, ("recovery_epoch", "captured_at_ms", "account_highwater", "inventory", "business_markers"))
    _uuid(payload["recovery_epoch"])
    _integer(payload["captured_at_ms"], 1)
    highwater = payload["account_highwater"]
    require(type(highwater) is dict and len(highwater) <= MAX_ROWS, "WITNESS_HIGHWATER_INVALID")
    for account, version in highwater.items():
        _uuid(account)
        _integer(version)
    inventory = payload["inventory"]
    _keys(inventory, FAMILIES)
    total = 0
    for family, rows in inventory.items():
        require(type(rows) is list and len(rows) <= MAX_ROWS, "WITNESS_ROWS_INVALID")
        total += len(rows)
        require(total <= MAX_ROWS, "WITNESS_ROWS_EXCEEDED")
        identities = set()
        for row in rows:
            _keys(row, FAMILIES[family])
            for key, value in row.items():
                if key == "client_id":
                    require(type(value) is str and re.fullmatch(r"[A-Za-z0-9_.:-]{1,100}", value),
                            "WITNESS_CLIENT_ID_INVALID")
                elif key == "id" or key.endswith("_id"):
                    if value is None and ((key == "parent_id" and family == "organizations")
                                          or (key == "application_id" and family == "oauth_clients")):
                        continue
                    _uuid(value)
                elif key in ("session_version", "identity_version", "access_policy_version"):
                    _integer(value, 0 if key == "session_version" else 1)
                elif key in ("is_super_admin", "refresh_enabled"):
                    require(type(value) is bool, "WITNESS_BOOLEAN_INVALID")
                elif key == "status":
                    require(value in ("ENABLED", "DISABLED"), "WITNESS_STATUS_INVALID")
                elif key == "member_type":
                    require(value in ("EMPLOYEE", "CUSTOMER"), "WITNESS_MEMBER_TYPE_INVALID")
                elif key == "subject_mode":
                    require(value in ("MACHINE", "USER", "LEGACY_MIXED"), "WITNESS_SUBJECT_MODE_INVALID")
                elif key == "purpose":
                    require(value in ("PLATFORM_CONSOLE", "CUSTOMER_PORTAL", "ACCOUNT"), "WITNESS_PURPOSE_INVALID")
                elif key == "allowed_member_types":
                    require(type(value) is list and len(value) <= 2
                            and all(type(item) is str and item in ("EMPLOYEE", "CUSTOMER") for item in value)
                            and len(set(value)) == len(value),
                            "WITNESS_MEMBER_TYPES_INVALID")
                    require((len(value) == 0) == (family == "oauth_clients" and row["subject_mode"] == "MACHINE"),
                            "WITNESS_MEMBER_TYPES_INVALID")
                elif key in ("redirect_uris", "post_logout_redirect_uris"):
                    require(type(value) is list and len(value) <= 16, "WITNESS_REDIRECT_INVALID")
                    for uri in value:
                        require(type(uri) is str and len(uri) <= 2048, "WITNESS_REDIRECT_INVALID")
                        try:
                            parsed = urlsplit(uri)
                            valid = parsed.scheme == "https" and parsed.hostname is not None and not (
                                parsed.username or parsed.password or parsed.fragment or parsed.query)
                        except ValueError:
                            valid = False
                        require(valid, "WITNESS_REDIRECT_INVALID")
                elif key in ("expires_at", "retired_at"):
                    require((value is None and key == "expires_at") or (type(value) is str
                            and re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}[T ][0-9:.+Z-]{8,40}", value)),
                            "WITNESS_TIMESTAMP_INVALID")
                else:
                    require(type(value) is str and re.fullmatch(r"[A-Za-z0-9_.:* /-]{1,100}", value),
                            "WITNESS_POLICY_TEXT_INVALID")
            identity_keys = ({"id"} if "id" in row else {"user_id"}
                             if family == "identity_account_tombstones" else set(row))
            identity = canonical({key: row[key] for key in identity_keys})
            require(identity not in identities, "WITNESS_DUPLICATE_IDENTITY")
            identities.add(identity)
            if family == "users":
                require(row["id"] in highwater and highwater[row["id"]] >= row["session_version"],
                        "WITNESS_HIGHWATER_BELOW_ACCOUNT")
    markers = payload["business_markers"]
    require(type(markers) is dict and len(markers) <= 64, "WITNESS_MARKERS_INVALID")
    for label, digest in markers.items():
        require(type(label) is str and LABEL.fullmatch(label), "WITNESS_LABEL_INVALID")
        _digest(digest)
    canonical(payload)
    return payload


def _tag(key, domain, document):
    require(type(key) is bytes and len(key) == 32, "WITNESS_KEY_INVALID")
    return hmac.new(key, domain + canonical(document), hashlib.sha256).hexdigest()


def seal(key, binding, payload, sequence, previous_sha256=ZERO):
    """Build an authenticated in-memory record; append controls its sequence."""
    _integer(sequence, 1, MAX_RECORDS)
    _digest(previous_sha256)
    require((sequence == 1) == (previous_sha256 == ZERO), "WITNESS_PREVIOUS_INVALID")
    document = {"format": FORMAT, "binding": _binding(binding), "sequence": sequence,
                "previous_sha256": previous_sha256, "payload": validate_payload(payload)}
    return _parse(canonical({"document": document, "tag": _tag(key, DOMAIN, document)}))


def verify(key, binding, envelope, minimum_sequence=0):
    """Authenticate one record and its checkpoint binding, without disk state."""
    _integer(minimum_sequence, 0, MAX_RECORDS)
    _keys(envelope, ("document", "tag"))
    _digest(envelope["tag"])
    document = envelope["document"]
    expected = _tag(key, DOMAIN, document)
    require(hmac.compare_digest(envelope["tag"], expected), "WITNESS_AUTHENTICATION_FAILED")
    _keys(document, ("format", "binding", "sequence", "previous_sha256", "payload"))
    require(document["format"] == FORMAT and _binding(document["binding"]) == _binding(binding),
            "WITNESS_BINDING_MISMATCH")
    _integer(document["sequence"], max(1, minimum_sequence), MAX_RECORDS)
    _digest(document["previous_sha256"])
    require((document["sequence"] == 1) == (document["previous_sha256"] == ZERO), "WITNESS_PREVIOUS_INVALID")
    validate_payload(document["payload"])
    return _parse(canonical(envelope))


def _sha(value):
    return hashlib.sha256(canonical(value)).hexdigest()


class WitnessStore:
    """Private append-only record chain plus authenticated durable CURRENT."""
    def __init__(self, directory, key, binding):
        self.directory = os.fspath(directory)
        self.binding = _binding(binding)
        _tag(key, DOMAIN, {})
        self.key = key

    @contextlib.contextmanager
    def _locked(self):
        require(os.name == "posix" and hasattr(os, "O_NOFOLLOW") and hasattr(os, "O_DIRECTORY"),
                "WITNESS_POSIX_STORAGE_REQUIRED")
        import fcntl
        require(type(self.directory) is str and self.directory.startswith("/")
                and "\\" not in self.directory and "\x00" not in self.directory
                and all(part not in ("", ".", "..") for part in self.directory[1:].split("/")),
                "WITNESS_PATH_INVALID")
        directory_fd = lock_fd = None
        try:
            directory_fd = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
            for part in self.directory[1:].split("/"):
                child_fd = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=directory_fd)
                os.close(directory_fd)
                directory_fd = child_fd
            info = os.fstat(directory_fd)
            require(info.st_uid == os.geteuid() and stat.S_IMODE(info.st_mode) == 0o700,
                    "WITNESS_DIRECTORY_PERMISSIONS_INVALID")
            lock_fd = os.open("LOCK", os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600, dir_fd=directory_fd)
            self._check_file(lock_fd)
            fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            yield directory_fd
        except OSError:
            raise WitnessError("WITNESS_STORAGE_UNAVAILABLE") from None
        finally:
            if lock_fd is not None:
                os.close(lock_fd)
            if directory_fd is not None:
                os.close(directory_fd)

    @staticmethod
    def _check_file(fd):
        info = os.fstat(fd)
        require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1 and info.st_uid == os.geteuid()
                and stat.S_IMODE(info.st_mode) == 0o600, "WITNESS_FILE_PERMISSIONS_INVALID")

    def _read(self, directory_fd, name):
        fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory_fd)
        try:
            self._check_file(fd)
            with os.fdopen(fd, "rb", closefd=False) as stream:
                raw = stream.read(MAX_BYTES + 1)
            return _parse(raw)
        finally:
            os.close(fd)

    def _write(self, directory_fd, name, value):
        raw = canonical(value)
        fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                     0o600, dir_fd=directory_fd)
        try:
            self._check_file(fd)
            with os.fdopen(fd, "wb", closefd=False) as stream:
                stream.write(raw)
                stream.flush()
                os.fsync(fd)
        finally:
            os.close(fd)
        os.fsync(directory_fd)

    def _latest(self, directory_fd, minimum_sequence):
        _integer(minimum_sequence, 0, MAX_RECORDS)
        names = os.listdir(directory_fd)
        records = sorted(name for name in names if RECORD.fullmatch(name))
        require(set(names) == set(records) | ({"LOCK", "CURRENT"} if "CURRENT" in names else {"LOCK"}),
                "WITNESS_UNEXPECTED_FILE")
        if "CURRENT" not in names:
            require(not records and minimum_sequence == 0, "WITNESS_CURRENT_MISSING")
            return None
        current = self._read(directory_fd, "CURRENT")
        _keys(current, ("document", "tag"))
        _digest(current["tag"])
        require(hmac.compare_digest(current["tag"], _tag(self.key, CURRENT_DOMAIN, current["document"])),
                "WITNESS_CURRENT_AUTHENTICATION_FAILED")
        head = current["document"]
        _keys(head, ("format", "binding", "sequence", "record_sha256"))
        require(head["format"] == CURRENT_FORMAT and _binding(head["binding"]) == self.binding,
                "WITNESS_CURRENT_BINDING_MISMATCH")
        _integer(head["sequence"], max(1, minimum_sequence), MAX_RECORDS)
        _digest(head["record_sha256"])
        require(records == ["witness-%08d.json" % index for index in range(1, head["sequence"] + 1)],
                "WITNESS_CHAIN_FILES_MISMATCH")
        previous, latest, total = ZERO, None, 0
        highwater = {}
        for index, name in enumerate(records, 1):
            latest = verify(self.key, self.binding, self._read(directory_fd, name))
            document = latest["document"]
            require(document["sequence"] == index and document["previous_sha256"] == previous,
                    "WITNESS_CHAIN_MISMATCH")
            current_highwater = document["payload"]["account_highwater"]
            require(all(account in current_highwater and current_highwater[account] >= version
                        for account, version in highwater.items()), "WITNESS_HIGHWATER_REGRESSION")
            highwater = current_highwater
            total += len(canonical(latest))
            require(total <= MAX_CHAIN_BYTES, "WITNESS_CHAIN_BYTES_EXCEEDED")
            previous = _sha(latest)
        require(previous == head["record_sha256"], "WITNESS_HEAD_MISMATCH")
        return latest

    def load_latest(self, minimum_sequence=0):
        """Authenticate all retained records and enforce the external floor."""
        with self._locked() as directory_fd:
            latest = self._latest(directory_fd, minimum_sequence)
            require(latest is not None, "WITNESS_CURRENT_MISSING")
            return latest

    def verify_candidate(self, envelope, minimum_sequence=0):
        """Reject a valid old record using the still-present durable CURRENT."""
        candidate = verify(self.key, self.binding, envelope, minimum_sequence)
        with self._locked() as directory_fd:
            latest = self._latest(directory_fd, minimum_sequence)
            require(latest is not None and _sha(latest) == _sha(candidate), "WITNESS_REPLAY_REJECTED")
        return candidate

    def append(self, payload, expected_sequence):
        """Append exactly CURRENT+1, preserving every previous account floor.

        A crash after record fsync and before CURRENT commit leaves an orphan
        and fails closed. No automatic healing can choose a weaker snapshot.
        """
        _integer(expected_sequence, 0, MAX_RECORDS - 1)
        payload = _parse(canonical(validate_payload(payload)))
        with self._locked() as directory_fd:
            latest = self._latest(directory_fd, expected_sequence)
            sequence = 0 if latest is None else latest["document"]["sequence"]
            require(sequence == expected_sequence, "WITNESS_SEQUENCE_CONFLICT")
            if latest is not None:
                old = latest["document"]["payload"]
                require(payload["captured_at_ms"] >= old["captured_at_ms"], "WITNESS_TIME_REGRESSION")
                require(all(account in payload["account_highwater"]
                            and payload["account_highwater"][account] >= version
                            for account, version in old["account_highwater"].items()),
                        "WITNESS_HIGHWATER_REGRESSION")
            envelope = seal(self.key, self.binding, payload, sequence + 1,
                            ZERO if latest is None else _sha(latest))
            new_size = len(canonical(envelope))
            old_size = sum(os.stat("witness-%08d.json" % index, dir_fd=directory_fd,
                                   follow_symlinks=False).st_size for index in range(1, sequence + 1))
            require(old_size + new_size <= MAX_CHAIN_BYTES, "WITNESS_CHAIN_BYTES_EXCEEDED")
            record_name = "witness-%08d.json" % (sequence + 1)
            record_temporary = "RECORD-" + secrets.token_hex(16) + ".tmp"
            self._write(directory_fd, record_temporary, envelope)
            # link is an atomic publish with no replacement, unlike rename on
            # POSIX. The temporary and new name share one inode only while the
            # exclusive writer lock is held; committed files have nlink == 1.
            os.link(record_temporary, record_name, src_dir_fd=directory_fd,
                    dst_dir_fd=directory_fd, follow_symlinks=False)
            os.unlink(record_temporary, dir_fd=directory_fd)
            os.fsync(directory_fd)
            document = {"format": CURRENT_FORMAT, "binding": self.binding,
                        "sequence": sequence + 1, "record_sha256": _sha(envelope)}
            current = {"document": document, "tag": _tag(self.key, CURRENT_DOMAIN, document)}
            temporary = "CURRENT-" + secrets.token_hex(16) + ".tmp"
            self._write(directory_fd, temporary, current)
            os.replace(temporary, "CURRENT", src_dir_fd=directory_fd, dst_dir_fd=directory_fd)
            os.fsync(directory_fd)
            return {"format": FORMAT, "sequence": sequence + 1, "sha256": _sha(envelope),
                    "payload_sha256": _sha(payload), "binding_sha256": _sha(self.binding)}
