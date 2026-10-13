ALTER TABLE tenants
    ADD COLUMN contact_name VARCHAR(100) NOT NULL DEFAULT '',
    ADD COLUMN contact_phone VARCHAR(30) NOT NULL DEFAULT '',
    ADD COLUMN contact_email VARCHAR(100) NOT NULL DEFAULT '';

ALTER TABLE organizations
    ADD COLUMN contact_name VARCHAR(100) NOT NULL DEFAULT '',
    ADD COLUMN contact_phone VARCHAR(30) NOT NULL DEFAULT '',
    ADD COLUMN contact_email VARCHAR(100) NOT NULL DEFAULT '';

-- Email is the canonical username when a contact has no telephone number.
ALTER TABLE users ALTER COLUMN username TYPE VARCHAR(100);
ALTER TABLE audit_logs ALTER COLUMN username TYPE VARCHAR(100);
CREATE INDEX idx_users_login_email ON users (lower(email)) WHERE email <> '';
CREATE INDEX idx_users_login_phone ON users ((regexp_replace(phone, '[[:space:]()\-]', '', 'g'))) WHERE phone <> '';
