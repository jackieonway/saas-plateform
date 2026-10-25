"""Run only in an explicitly isolated Linux container; never uses a database."""
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import stat
import subprocess
import unittest


@unittest.skipUnless(os.environ.get("UUID_U06_CODEC_ISOLATED_FIXTURE") == "true",
                     "requires an isolated container with private U06 roots")
class EvidenceCliTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.binary = Path("/usr/local/libexec/saas-recovery-u06-codec")
        expected = os.environ["UUID_U06_CODEC_SHA256"]
        assert len(expected) == 64
        assert hashlib.sha256(cls.binary.read_bytes()).hexdigest() == expected

    def private(self, path, body):
        raw = json.dumps(body).encode() if isinstance(body, dict) else body
        with path.open("xb") as handle:
            os.fchmod(handle.fileno(), 0o600)
            handle.write(raw)

    def call(self, operation, body):
        config = self.output_dir / ("config-" + secrets.token_hex(8) + ".json")
        self.private(config, body)
        return subprocess.run([str(self.binary), operation, str(config)],
                              capture_output=True, text=True, timeout=30)

    def setUp(self):
        batch = "codec-fixture-" + secrets.token_hex(8)
        input_root = Path("/var/lib/saas-uuid-u06-20261024")
        output_root = Path("/dev/shm/saas-uuid-u06-20261024")
        for root in (input_root, output_root):
            root.mkdir(mode=0o700, exist_ok=True)
            info = root.lstat()
            self.assertTrue(stat.S_ISDIR(info.st_mode))
            self.assertEqual(stat.S_IMODE(info.st_mode), 0o700)
            self.assertEqual(info.st_uid, os.geteuid())
        self.input_dir = input_root / batch
        self.output_dir = output_root / batch
        self.input_dir.mkdir(mode=0o700)
        self.output_dir.mkdir(mode=0o700)
        self.plain = b"U06 codec private fixture\n" * 50_000
        self.source = self.output_dir / "original"
        self.key = self.output_dir / "key"
        self.cipher = self.input_dir / "snapshot.enc"
        self.output = self.output_dir / "opened"
        self.private(self.source, self.plain)
        self.key_bytes = os.urandom(32)
        self.private(self.key, self.key_bytes)
        purpose = f"saas-uuid-preview-freeze/1/{batch}/u06-cli"
        sealed = self.call("uuid-seal-evidence", dict(profile="uuid-local-preview",
            input=str(self.source), output=str(self.cipher), key_file=str(self.key), purpose=purpose))
        self.assertEqual(sealed.returncode, 0, sealed.stderr)
        report = json.loads(sealed.stdout)
        self.assertEqual(report["sha256"], hashlib.sha256(self.plain).hexdigest())
        self.config = dict(profile="uuid-local-u06", input=str(self.cipher),
            output=str(self.output), key_file=str(self.key), purpose=purpose,
            expected_sha256=report["sha256"])

    def tearDown(self):
        for owned in (self.input_dir, self.output_dir):
            shutil.rmtree(owned)

    def test_protected_binary_roundtrip_and_no_overwrite(self):
        opened = self.call("uuid-open-evidence", self.config)
        self.assertEqual(opened.returncode, 0, opened.stderr)
        report = json.loads(opened.stdout)
        self.assertEqual(report["status"], "PRIVATE_U06_EVIDENCE_OPENED")
        self.assertEqual(report["frames"], 2)
        self.assertEqual(report["plaintext_bytes"], len(self.plain))
        self.assertEqual(self.output.read_bytes(), self.plain)
        info = self.output.lstat()
        self.assertEqual(stat.S_IMODE(info.st_mode), 0o600)
        self.assertEqual(info.st_nlink, 1)
        refused = self.call("uuid-open-evidence", self.config)
        self.assertEqual(refused.returncode, 1)
        self.assertEqual(self.output.lstat().st_ino, info.st_ino)
        self.assertEqual(self.output.read_bytes(), self.plain)

    def test_authentication_failures_publish_nothing_and_clear_staging(self):
        original_cipher = self.cipher.read_bytes()
        for variant in ("truncated", "footer", "trailing", "key", "purpose", "digest"):
            with self.subTest(variant=variant):
                config = dict(self.config)
                changed = original_cipher
                self.key.write_bytes(self.key_bytes)
                if variant == "truncated":
                    changed = original_cipher[:-1]
                elif variant == "footer":
                    changed = original_cipher[:-1] + bytes([original_cipher[-1] ^ 1])
                elif variant == "trailing":
                    changed += b"x"
                elif variant == "key":
                    self.key.write_bytes(os.urandom(32))
                elif variant == "purpose":
                    config["purpose"] = config["purpose"].replace("u06-cli", "u06-other")
                elif variant == "digest":
                    config["expected_sha256"] = "0" * 64
                self.cipher.write_bytes(changed)
                result = self.call("uuid-open-evidence", config)
                self.assertEqual(result.returncode, 1)
                self.assertEqual(result.stdout, "")
                self.assertNotIn(config["purpose"], result.stderr)
                self.assertFalse(self.output.exists())
                self.assertFalse(list(self.output_dir.glob("recovery-*")))

    def test_existing_symlink_and_target_bytes_are_preserved(self):
        self.output.symlink_to(self.source)
        before = self.output.lstat()
        result = self.call("uuid-open-evidence", self.config)
        self.assertEqual(result.returncode, 1)
        after = self.output.lstat()
        self.assertTrue(stat.S_ISLNK(after.st_mode))
        self.assertEqual((before.st_dev, before.st_ino), (after.st_dev, after.st_ino))
        self.assertEqual(self.output.readlink(), self.source)
        self.assertEqual(self.source.read_bytes(), self.plain)


if __name__ == "__main__":
    unittest.main()
