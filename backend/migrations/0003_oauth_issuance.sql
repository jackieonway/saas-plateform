-- Version 1 rows predate atomic ordinary OAuth issuance. Preserve them, their
-- revocation state and legacy JWT metadata for the documented compatibility
-- window; no live credential or replay evidence is deleted here.
ALTER TABLE oauth_tokens ADD COLUMN storage_version smallint NOT NULL DEFAULT 1;
ALTER TABLE oauth_tokens ALTER COLUMN storage_version SET DEFAULT 2;

ALTER TABLE oauth_tokens ADD CONSTRAINT ck_oauth_tokens_storage_version
    CHECK (storage_version IN (1, 2));

-- NOT VALID avoids scanning legacy data. The version marker is essential:
-- PostgreSQL also checks a NOT VALID constraint when an old row is updated,
-- including refresh consumption or revocation. Legacy rows must remain writable.
ALTER TABLE oauth_tokens ADD CONSTRAINT ck_oauth_tokens_oauth_no_plaintext
    CHECK (token_use <> 'oauth' OR storage_version = 1 OR jwt IS NULL) NOT VALID;

COMMENT ON COLUMN oauth_tokens.storage_version IS
    '1: preserved legacy metadata; 2: newly issued token, JWT plaintext prohibited for ordinary OAuth';
COMMENT ON COLUMN oauth_tokens.jwt IS
    'Deprecated legacy ordinary OAuth plaintext; new issuance stores NULL. Preserve old rows until an approved cleanup window.';
