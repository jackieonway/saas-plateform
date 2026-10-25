\set ON_ERROR_STOP on
\set ECHO none
\set QUIET on
-- Explicit administrative job, never a runtime or a second schema migrator.
-- Required psql variable: role_prefix. Passwords are provisioned separately by
-- the operator through a protected channel; newly created LOGIN roles have no
-- password and cannot authenticate with password authentication yet.
BEGIN;
CREATE TEMP TABLE identity_role_input(prefix text NOT NULL) ON COMMIT DROP;
INSERT INTO identity_role_input VALUES (:'role_prefix');
DO $roles$
DECLARE prefix text; suffix text; role_name text;
BEGIN
    SELECT input.prefix INTO prefix FROM pg_temp.identity_role_input input;
    IF prefix !~ '^[a-z][a-z0-9_]{5,31}$' THEN
        RAISE EXCEPTION 'IDENTITY_ROLE_PREFIX_INVALID';
    END IF;
    FOREACH suffix IN ARRAY ARRAY['_migration_owner','_identity_owner','_platform_owner','_bridge_owner','_identity_runtime','_platform_runtime'] LOOP
        role_name:=prefix||suffix;
        IF EXISTS(SELECT 1 FROM pg_catalog.pg_roles WHERE rolname=role_name) THEN
            RAISE EXCEPTION 'IDENTITY_ROLE_ALREADY_EXISTS';
        END IF;
        EXECUTE pg_catalog.format('CREATE ROLE %I %s NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS',
            role_name,CASE WHEN suffix IN ('_identity_runtime','_platform_runtime') THEN 'LOGIN PASSWORD NULL' ELSE 'NOLOGIN' END);
    END LOOP;
END $roles$;
COMMIT;
