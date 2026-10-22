-- Run with psql as the platform database administrator, against the intended
-- migrated platform database. This never runs automatically at service startup.
-- Provision the LOGIN password separately using psql \password notification_ws
-- (or the deployment secret manager); do not put it in this file/CLI arguments.
-- PostgreSQL LISTEN has no separate GRANT. This role may read only the columns
-- needed for scoped event reconstruction and has no platform identity access.
\set ON_ERROR_STOP on
BEGIN;
DO $$
BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'notification_ws') THEN
    CREATE ROLE notification_ws LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE
      NOINHERIT NOREPLICATION NOBYPASSRLS CONNECTION LIMIT 8;
  END IF;
  IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'notification_ws'
      AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
    OR EXISTS (SELECT FROM pg_auth_members m JOIN pg_roles r ON r.oid = m.member
      WHERE r.rolname = 'notification_ws') THEN
    RAISE EXCEPTION 'notification_ws must be a dedicated nonprivileged role without memberships';
  END IF;
END
$$;
ALTER ROLE notification_ws SET default_transaction_read_only = on;
ALTER ROLE notification_ws SET search_path = pg_catalog, public;
REVOKE ALL ON SCHEMA public FROM notification_ws;
REVOKE ALL ON ALL TABLES IN SCHEMA public FROM notification_ws;
REVOKE ALL ON ALL SEQUENCES IN SCHEMA public FROM notification_ws;
SELECT format('GRANT CONNECT ON DATABASE %I TO notification_ws', current_database()) \gexec
GRANT USAGE ON SCHEMA public TO notification_ws;
GRANT SELECT (id, user_id, tenant_id, organization_id, source_kind,
  application_id, publication_id, type, title, content, link, created_at)
  ON TABLE public.notifications TO notification_ws;
COMMIT;
-- Also audit PUBLIC grants in the actual database. REVOKE from a named role
-- cannot remove privileges inherited through PUBLIC; this script intentionally
-- does not change unrelated platform-wide policy. No default table privileges
-- are granted, so future migrations cannot silently widen this service's reads.
