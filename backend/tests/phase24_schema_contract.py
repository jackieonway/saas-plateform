"""Static closure checks for the canonical fresh-install authentication schema.

Run with ``python backend/tests/phase24_schema_contract.py`` from the repository.
These checks do not connect to PostgreSQL; the phase acceptance also requires an
actual empty-database migration, provisioning, readiness and runtime ACL test.
"""

from pathlib import Path
import re
import unittest


BACKEND = Path(__file__).resolve().parents[1]
SCHEMA = (BACKEND / "migrations/0001_init.sql").read_text(encoding="utf-8")
PROVISION = (BACKEND / "ops/identity-deployment/provision.sql").read_text(encoding="utf-8")
VERIFY = (BACKEND / "ops/identity-deployment/verify.sql").read_text(encoding="utf-8")


def without_comments(sql: str) -> str:
    return re.sub(r"--[^\n]*", "", sql)


class CanonicalSchemaContract(unittest.TestCase):
    def test_current_private_source_function_calls_exist_in_the_baseline(self):
        functions = set(re.findall(
            r"CREATE\s+(?:OR\s+REPLACE\s+)?FUNCTION\s+public\.([a-z_0-9]+)", SCHEMA, re.I
        ))
        missing = []
        for root in (BACKEND / "src", BACKEND.parents[1] / "saas-identity/src"):
            for path in root.rglob("*.rs"):
                if "test" in path.name or any("tests" in part for part in path.parts):
                    continue
                source = path.read_text(encoding="utf-8")
                calls = re.finditer(
                    r"(?:SELECT|FROM|JOIN)\s+(?:public\.)?"
                    r"((?:identity_|platform_|bff_|customer_|revoke_|auth_)[a-z_0-9]+)\s*\(",
                    source, re.I,
                )
                for call in calls:
                    if call.group(1) not in functions:
                        missing.append((str(path.relative_to(root)), call.group(1)))
        self.assertEqual(missing, [], "retired SQL helpers still have current callers")

    def test_all_public_relation_references_are_declared(self):
        sql = without_comments(SCHEMA)
        declared = set(re.findall(r"CREATE TABLE public\.([a-z_0-9]+)", sql))
        references = set(re.findall(
            r"(?:FROM|JOIN)\s+public\.([a-z_0-9]+)(?![a-z_0-9]|\s*\()", sql
        ))
        references.update(re.findall(
            r"(?:UPDATE|INSERT INTO|REFERENCES|ALTER TABLE(?: ONLY)?)\s+public\.([a-z_0-9]+)", sql
        ))
        # SQLx creates its own migration history before applying the baseline.
        self.assertEqual(references - declared - {"_sqlx_migrations"}, set())

    def test_every_trigger_uses_a_declared_function(self):
        functions = set(re.findall(r"CREATE FUNCTION public\.([a-z_0-9]+)", SCHEMA))
        targets = set(re.findall(r"EXECUTE FUNCTION public\.([a-z_0-9]+)\(", SCHEMA))
        self.assertEqual(targets - functions, set())

    def test_no_legacy_relation_or_registry_can_be_initialized(self):
        tables = set(re.findall(r"CREATE TABLE public\.([a-z_0-9]+)", SCHEMA))
        self.assertEqual({name for name in tables if name.startswith(("oauth_", "oidc_"))}, set())
        self.assertNotIn("identity_oidc_admin_receipts", tables)
        self.assertNotIn("platform_oidc_command_intents", tables)
        for sql in (SCHEMA, PROVISION, VERIFY):
            self.assertNotRegex(without_comments(sql), r"public\.(?:oauth_|oidc_)[a-z_0-9]+")
        self.assertNotIn("allowed_oidc_clients", SCHEMA)
        self.assertIn("DEFAULT 'AUTH' CHECK (client_registry='AUTH')", SCHEMA)
        self.assertNotIn("LEGACY_MIXED", SCHEMA)
        self.assertNotIn("oauth_token_count", SCHEMA)

    def test_retained_authority_and_command_stores_have_explicit_owners(self):
        for name in ("auth_signing_keys", "auth_provider_settings", "identity_auth_command_receipts"):
            self.assertIn(f"CREATE TABLE public.{name}", SCHEMA)
            self.assertIn(f"'{name}'", PROVISION)
        self.assertIn("CREATE TABLE public.platform_auth_command_intents", SCHEMA)
        self.assertIn("ELSIF item.relname='platform_auth_command_intents'", PROVISION)
        self.assertIn("CREATE FUNCTION public.identity_auth_command_intent_active", SCHEMA)
        self.assertIn("CREATE FUNCTION public.identity_lock_auth_source", SCHEMA)
        self.assertIn("CREATE FUNCTION public.bff_auth_proof_active", SCHEMA)
        self.assertIn("CREATE FUNCTION public.auth_console_source_active", SCHEMA)
        self.assertNotIn("/internal/identity/oidc/", SCHEMA)
        self.assertIn("/internal/identity/auth/keys", SCHEMA)
        self.assertIn("'KEY_ADMIN'", SCHEMA)


if __name__ == "__main__":
    unittest.main()
