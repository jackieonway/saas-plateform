-- Keep existing integrations in their current compatibility mode. The managed
-- client API enables this explicitly by default for newly created PUBLIC
-- clients; operators can migrate existing clients after updating their caller.
ALTER TABLE oauth_clients ADD COLUMN require_pkce boolean NOT NULL DEFAULT false;

COMMENT ON COLUMN oauth_clients.require_pkce IS
    'Authorization-code grants require RFC 7636 S256 when true; false preserves legacy optional PKCE. Managed new PUBLIC clients default to true.';
