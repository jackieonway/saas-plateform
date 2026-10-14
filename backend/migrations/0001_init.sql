-- SaaS Platform unified initial schema for PostgreSQL 17+.
-- Empty databases only. SQLx records this baseline as version 1.
-- Accounts, tenants and authorization grants are initialized by src/seed.rs.

-- Optional notification search acceleration.
DO $$ BEGIN
  CREATE EXTENSION IF NOT EXISTS pg_trgm;
EXCEPTION WHEN OTHERS THEN
  RAISE NOTICE 'pg_trgm unavailable; notification search uses ordinary indexes';
END $$;

-- Tables: final columns and defaults.
CREATE SEQUENCE public."audit_logs_id_seq" AS bigint START WITH 1 INCREMENT BY 1 MINVALUE 1 MAXVALUE 9223372036854775807 CACHE 1 NO CYCLE;
CREATE TABLE public."application_deployments" (
  "application_id" uuid NOT NULL,
  "environment" character varying(32) NOT NULL,
  "active_release_id" uuid NOT NULL,
  "previous_release_id" uuid,
  "updated_at" timestamp with time zone DEFAULT now() NOT NULL,
  "updated_by" uuid
);

CREATE TABLE public."application_releases" (
  "id" uuid DEFAULT gen_random_uuid() NOT NULL,
  "application_id" uuid NOT NULL,
  "environment" character varying(32) NOT NULL,
  "version" character varying(64) NOT NULL,
  "entry_type" character varying(10) NOT NULL,
  "entry_url" character varying(2000) NOT NULL,
  "health_url" character varying(2000),
  "resources" jsonb DEFAULT '[]'::jsonb NOT NULL,
  "validation_status" character varying(10) DEFAULT 'DRAFT'::character varying NOT NULL,
  "validation_result" jsonb DEFAULT '{}'::jsonb NOT NULL,
  "checked_at" timestamp with time zone,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL,
  "created_by" uuid
);

CREATE TABLE public."applications" (
  "id" uuid DEFAULT gen_random_uuid() NOT NULL,
  "code" character varying(50) NOT NULL,
  "name" character varying(100) NOT NULL,
  "icon" character varying(100) DEFAULT 'LayoutGrid'::character varying,
  "base_path" character varying(200) DEFAULT ''::character varying,
  "description" text DEFAULT ''::text,
  "sort_order" integer DEFAULT 0 NOT NULL,
  "status" character varying(20) DEFAULT 'ENABLED'::character varying NOT NULL,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL,
  "updated_at" timestamp with time zone DEFAULT now() NOT NULL,
  "entry_type" character varying(10) DEFAULT 'ESM'::character varying NOT NULL,
  "entry_url" character varying(500) DEFAULT ''::character varying NOT NULL,
  "runtime_mode" character varying(10) DEFAULT 'IFRAME'::character varying NOT NULL
);

CREATE TABLE public."audit_logs" (
  "id" bigint DEFAULT nextval('audit_logs_id_seq'::regclass) NOT NULL,
  "user_id" uuid,
  "username" character varying(100) DEFAULT ''::character varying,
  "tenant_id" uuid,
  "tenant_name" character varying(100) DEFAULT ''::character varying,
  "organization_id" uuid,
  "organization_name" character varying(100) DEFAULT ''::character varying,
  "action" character varying(50) NOT NULL,
  "detail" text DEFAULT ''::text,
  "ip" character varying(50) DEFAULT ''::character varying,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL,
  "user_agent" character varying(500) DEFAULT ''::character varying NOT NULL,
  "request_id" uuid,
  "application_id" uuid
);

CREATE TABLE public."departments" (
  "id" uuid DEFAULT gen_random_uuid() NOT NULL,
  "organization_id" uuid NOT NULL,
  "parent_id" uuid,
  "name" character varying(100) NOT NULL,
  "code" character varying(50) DEFAULT ''::character varying,
  "sort_order" integer DEFAULT 0 NOT NULL,
  "description" text DEFAULT ''::text,
  "status" character varying(20) DEFAULT 'ENABLED'::character varying NOT NULL,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL,
  "updated_at" timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public."membership_data_grants" (
  "id" uuid DEFAULT gen_random_uuid() NOT NULL,
  "membership_id" uuid NOT NULL,
  "application_id" uuid NOT NULL,
  "resource_code" character varying(100) NOT NULL,
  "action_code" character varying(100) NOT NULL,
  "scope_type" character varying(20) NOT NULL,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL,
  "updated_at" timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public."memberships" (
  "id" uuid DEFAULT gen_random_uuid() NOT NULL,
  "user_id" uuid NOT NULL,
  "organization_id" uuid NOT NULL,
  "department_id" uuid,
  "position" character varying(50) DEFAULT ''::character varying,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public."menus" (
  "id" uuid DEFAULT gen_random_uuid() NOT NULL,
  "application_id" uuid NOT NULL,
  "parent_id" uuid,
  "type" character varying(20) DEFAULT 'MENU'::character varying NOT NULL,
  "title" character varying(100) NOT NULL,
  "path" character varying(200) DEFAULT ''::character varying,
  "component" character varying(200) DEFAULT ''::character varying,
  "icon" character varying(100) DEFAULT ''::character varying,
  "permission" character varying(100) DEFAULT ''::character varying,
  "sort_order" integer DEFAULT 0 NOT NULL,
  "visible" boolean DEFAULT true NOT NULL,
  "status" character varying(20) DEFAULT 'ENABLED'::character varying NOT NULL,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL,
  "updated_at" timestamp with time zone DEFAULT now() NOT NULL,
  "link_url" character varying(500) DEFAULT ''::character varying NOT NULL,
  "link_target" character varying(10) DEFAULT 'BLANK'::character varying NOT NULL
);

CREATE TABLE public."notification_prefs" (
  "user_id" uuid NOT NULL,
  "dnd_enabled" boolean DEFAULT false NOT NULL,
  "dnd_start" character varying(5) DEFAULT '22:00'::character varying NOT NULL,
  "dnd_end" character varying(5) DEFAULT '07:00'::character varying NOT NULL,
  "muted_types" text DEFAULT ''::text NOT NULL,
  "updated_at" timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public."notification_publications" (
  "id" uuid DEFAULT gen_random_uuid() NOT NULL,
  "tenant_id" uuid NOT NULL,
  "publisher_organization_id" uuid NOT NULL,
  "application_id" uuid NOT NULL,
  "publisher_user_id" uuid NOT NULL,
  "publisher_username" character varying(50) NOT NULL,
  "application_name" character varying(200) NOT NULL,
  "idempotency_key" character varying(128) NOT NULL,
  "request_hash" character(64) NOT NULL,
  "request_body" jsonb NOT NULL,
  "recipient_count" integer NOT NULL,
  "delivery_count" integer NOT NULL,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public."notifications" (
  "id" uuid DEFAULT gen_random_uuid() NOT NULL,
  "user_id" uuid NOT NULL,
  "type" character varying(20) DEFAULT 'INFO'::character varying NOT NULL,
  "title" character varying(200) NOT NULL,
  "content" text DEFAULT ''::text,
  "link" character varying(300) DEFAULT ''::character varying,
  "is_read" boolean DEFAULT false NOT NULL,
  "read_at" timestamp with time zone,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL,
  "source_kind" character varying(12) DEFAULT 'LEGACY'::character varying NOT NULL,
  "tenant_id" uuid,
  "organization_id" uuid,
  "application_id" uuid,
  "publication_id" uuid
);

CREATE TABLE public."oauth_authorization_codes" (
  "id" uuid DEFAULT gen_random_uuid() NOT NULL,
  "code_hash" text NOT NULL,
  "client_id" character varying(100) NOT NULL,
  "user_id" uuid NOT NULL,
  "tenant_id" uuid,
  "organization_id" uuid,
  "redirect_uri" text NOT NULL,
  "scope" text DEFAULT ''::text,
  "code_challenge" text DEFAULT ''::text,
  "code_challenge_method" character varying(10) DEFAULT ''::character varying,
  "expires_at" timestamp with time zone NOT NULL,
  "consumed" boolean DEFAULT false NOT NULL,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public."oauth_authorization_requests" (
  "id" uuid DEFAULT gen_random_uuid() NOT NULL,
  "client_id" character varying(100) NOT NULL,
  "redirect_uri" text NOT NULL,
  "scope" text DEFAULT ''::text NOT NULL,
  "state" text DEFAULT ''::text,
  "code_challenge" text DEFAULT ''::text,
  "code_challenge_method" character varying(10) DEFAULT ''::character varying,
  "expires_at" timestamp with time zone NOT NULL,
  "consumed" boolean DEFAULT false NOT NULL,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public."oauth_clients" (
  "id" uuid DEFAULT gen_random_uuid() NOT NULL,
  "client_id" character varying(100) NOT NULL,
  "client_secret_hash" text,
  "client_name" character varying(200) NOT NULL,
  "description" text DEFAULT ''::text,
  "client_type" character varying(20) DEFAULT 'CONFIDENTIAL'::character varying NOT NULL,
  "redirect_uris" text DEFAULT ''::text,
  "grant_types" text DEFAULT 'authorization_code,refresh_token'::text,
  "scopes" text DEFAULT 'profile'::text,
  "access_token_validity" bigint DEFAULT 7200 NOT NULL,
  "refresh_token_validity" bigint DEFAULT 604800 NOT NULL,
  "status" character varying(20) DEFAULT 'ENABLED'::character varying NOT NULL,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL,
  "updated_at" timestamp with time zone DEFAULT now() NOT NULL,
  "application_id" uuid
);

CREATE TABLE public."oauth_device_codes" (
  "id" uuid DEFAULT gen_random_uuid() NOT NULL,
  "device_code_hash" text NOT NULL,
  "user_code" character varying(20) NOT NULL,
  "client_id" character varying(100) NOT NULL,
  "scope" text DEFAULT ''::text,
  "status" character varying(20) DEFAULT 'PENDING'::character varying NOT NULL,
  "user_id" uuid,
  "tenant_id" uuid,
  "organization_id" uuid,
  "interval_seconds" bigint DEFAULT 5 NOT NULL,
  "last_polled_at" timestamp with time zone,
  "expires_at" timestamp with time zone NOT NULL,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public."oauth_tokens" (
  "id" uuid DEFAULT gen_random_uuid() NOT NULL,
  "token_hash" text NOT NULL,
  "token_type" character varying(20) NOT NULL,
  "jwt" text,
  "user_id" uuid,
  "client_id" character varying(100) NOT NULL,
  "tenant_id" uuid,
  "organization_id" uuid,
  "scope" text DEFAULT ''::text,
  "expires_at" timestamp with time zone NOT NULL,
  "revoked" boolean DEFAULT false NOT NULL,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL,
  "token_use" character varying(20) DEFAULT 'oauth'::character varying NOT NULL,
  "audience" character varying(100),
  "jti" uuid,
  "application_id" uuid,
  "platform_session_id" uuid
);

CREATE TABLE public."organization_applications" (
  "organization_id" uuid NOT NULL,
  "application_id" uuid NOT NULL,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public."organization_menus" (
  "organization_id" uuid NOT NULL,
  "menu_id" uuid NOT NULL,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public."organizations" (
  "id" uuid DEFAULT gen_random_uuid() NOT NULL,
  "tenant_id" uuid NOT NULL,
  "parent_id" uuid,
  "name" character varying(100) NOT NULL,
  "code" character varying(50) DEFAULT ''::character varying,
  "sort_order" integer DEFAULT 0 NOT NULL,
  "description" text DEFAULT ''::text,
  "status" character varying(20) DEFAULT 'ENABLED'::character varying NOT NULL,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL,
  "updated_at" timestamp with time zone DEFAULT now() NOT NULL,
  "expires_at" timestamp with time zone,
  "province" character varying(100) DEFAULT ''::character varying NOT NULL,
  "city" character varying(100) DEFAULT ''::character varying NOT NULL,
  "district" character varying(100) DEFAULT ''::character varying NOT NULL,
  "address" character varying(300) DEFAULT ''::character varying NOT NULL,
  "longitude" double precision,
  "latitude" double precision,
  "township" character varying(128) DEFAULT ''::character varying NOT NULL,
  "village" character varying(128) DEFAULT ''::character varying NOT NULL,
  "organization_type" character varying(30) DEFAULT 'COMPANY'::character varying NOT NULL,
  "contact_name" character varying(100) DEFAULT ''::character varying NOT NULL,
  "contact_phone" character varying(30) DEFAULT ''::character varying NOT NULL,
  "contact_email" character varying(100) DEFAULT ''::character varying NOT NULL
);

CREATE TABLE public."platform_session_refresh_tokens" (
  "id" uuid DEFAULT gen_random_uuid() NOT NULL,
  "session_id" uuid NOT NULL,
  "token_hash" character varying(64) NOT NULL,
  "session_version" bigint NOT NULL,
  "issued_at" timestamp with time zone DEFAULT now() NOT NULL,
  "expires_at" timestamp with time zone NOT NULL,
  "consumed_at" timestamp with time zone
);

CREATE TABLE public."platform_sessions" (
  "id" uuid DEFAULT gen_random_uuid() NOT NULL,
  "user_id" uuid NOT NULL,
  "tenant_id" uuid NOT NULL,
  "organization_id" uuid NOT NULL,
  "issued_at" timestamp with time zone DEFAULT now() NOT NULL,
  "expires_at" timestamp with time zone NOT NULL,
  "revoked_at" timestamp with time zone,
  "ip" character varying(50) DEFAULT ''::character varying NOT NULL,
  "user_agent" text DEFAULT ''::text NOT NULL
);

CREATE TABLE public."platform_settings" (
  "singleton" boolean DEFAULT true NOT NULL,
  "platform_name" character varying(100) DEFAULT 'SaaS Platform'::character varying NOT NULL,
  "platform_description" character varying(300) DEFAULT '多租户权限管理平台'::character varying NOT NULL,
  "logo_url" character varying(2048) DEFAULT ''::character varying NOT NULL,
  "login_title" character varying(100) DEFAULT '欢迎回来'::character varying NOT NULL,
  "login_description" character varying(300) DEFAULT '管理您的租户与应用权限'::character varying NOT NULL,
  "footer_text" character varying(500) DEFAULT '© 2026 SaaS Platform · Powered by Rust & React'::character varying NOT NULL,
  "updated_at" timestamp with time zone DEFAULT now() NOT NULL,
  "primary_color" character varying(7) DEFAULT '#008577'::character varying NOT NULL
);

CREATE TABLE public."region_versions" (
  "version" character varying(20) NOT NULL,
  "label" character varying(100) NOT NULL,
  "source_version" character varying(20),
  "imported" boolean DEFAULT false NOT NULL,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL,
  "updated_at" timestamp with time zone DEFAULT now() NOT NULL,
  "deleted_at" timestamp with time zone
);

CREATE TABLE public."regions" (
  "version" character varying(20) NOT NULL,
  "source_id" character varying(32) NOT NULL,
  "code" character varying(12) NOT NULL,
  "parent_code" character varying(12) DEFAULT '0'::character varying NOT NULL,
  "name" character varying(128) NOT NULL,
  "name_pinyin" character varying(512) NOT NULL,
  "name_short" character varying(64) NOT NULL,
  "level" integer DEFAULT 1 NOT NULL,
  "type" character varying(12),
  "type_name" character varying(32),
  "post_code" character varying(512),
  "source_url" character varying(512),
  "created_at" timestamp with time zone DEFAULT now() NOT NULL,
  "updated_at" timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public."role_menus" (
  "role_id" uuid NOT NULL,
  "menu_id" uuid NOT NULL
);

CREATE TABLE public."roles" (
  "id" uuid DEFAULT gen_random_uuid() NOT NULL,
  "tenant_id" uuid NOT NULL,
  "organization_id" uuid NOT NULL,
  "name" character varying(100) NOT NULL,
  "code" character varying(50) DEFAULT ''::character varying,
  "description" text DEFAULT ''::text,
  "status" character varying(20) DEFAULT 'ENABLED'::character varying NOT NULL,
  "built_in" boolean DEFAULT false NOT NULL,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL,
  "updated_at" timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public."tenant_applications" (
  "tenant_id" uuid NOT NULL,
  "application_id" uuid NOT NULL,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public."tenant_menus" (
  "tenant_id" uuid NOT NULL,
  "menu_id" uuid NOT NULL,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public."tenants" (
  "id" uuid DEFAULT gen_random_uuid() NOT NULL,
  "name" character varying(100) NOT NULL,
  "code" character varying(50) NOT NULL,
  "description" text DEFAULT ''::text,
  "status" character varying(20) DEFAULT 'ENABLED'::character varying NOT NULL,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL,
  "updated_at" timestamp with time zone DEFAULT now() NOT NULL,
  "is_platform" boolean DEFAULT false NOT NULL,
  "expires_at" timestamp with time zone,
  "province" character varying(100) DEFAULT ''::character varying NOT NULL,
  "city" character varying(100) DEFAULT ''::character varying NOT NULL,
  "district" character varying(100) DEFAULT ''::character varying NOT NULL,
  "address" character varying(300) DEFAULT ''::character varying NOT NULL,
  "longitude" double precision,
  "latitude" double precision,
  "township" character varying(128) DEFAULT ''::character varying NOT NULL,
  "village" character varying(128) DEFAULT ''::character varying NOT NULL,
  "tenant_type" character varying(30) DEFAULT 'COMPANY'::character varying NOT NULL,
  "contact_name" character varying(100) DEFAULT ''::character varying NOT NULL,
  "contact_phone" character varying(30) DEFAULT ''::character varying NOT NULL,
  "contact_email" character varying(100) DEFAULT ''::character varying NOT NULL
);

CREATE TABLE public."user_roles" (
  "user_id" uuid NOT NULL,
  "role_id" uuid NOT NULL,
  "organization_id" uuid NOT NULL,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public."users" (
  "id" uuid DEFAULT gen_random_uuid() NOT NULL,
  "username" character varying(100) NOT NULL,
  "password_hash" text NOT NULL,
  "real_name" character varying(100) DEFAULT ''::character varying,
  "email" character varying(100) DEFAULT ''::character varying,
  "phone" character varying(30) DEFAULT ''::character varying,
  "avatar" text DEFAULT ''::text,
  "gender" character varying(10) DEFAULT 'UNKNOWN'::character varying,
  "status" character varying(20) DEFAULT 'ENABLED'::character varying NOT NULL,
  "is_super_admin" boolean DEFAULT false NOT NULL,
  "last_login_at" timestamp with time zone,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL,
  "updated_at" timestamp with time zone DEFAULT now() NOT NULL,
  "session_version" bigint DEFAULT 0 NOT NULL
);

-- Database functions and integrity triggers.
ALTER SEQUENCE public."audit_logs_id_seq" OWNED BY public."audit_logs"."id";
CREATE OR REPLACE FUNCTION public.bump_session_version_on_user_disable()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
BEGIN
    IF OLD.status = 'ENABLED' AND NEW.status <> 'ENABLED' THEN
        NEW.session_version := OLD.session_version + 1;
    END IF;
    RETURN NEW;
END;
$function$;

CREATE OR REPLACE FUNCTION public.org_chain_active(target_org uuid)
 RETURNS boolean
 LANGUAGE sql
 STABLE
AS $function$
    WITH RECURSIVE chain AS (
        SELECT id, parent_id, status, expires_at
        FROM organizations WHERE id = target_org
        UNION ALL
        SELECT parent.id, parent.parent_id, parent.status, parent.expires_at
        FROM chain child JOIN organizations parent ON parent.id = child.parent_id
    )
    SELECT COALESCE(bool_and(status = 'ENABLED'
                             AND (expires_at IS NULL OR expires_at > now())), false)
    FROM chain;
$function$;

CREATE OR REPLACE FUNCTION public.org_chain_expires_at(target_org uuid)
 RETURNS timestamp with time zone
 LANGUAGE sql
 STABLE
AS $function$
    WITH RECURSIVE chain AS (
        SELECT id, parent_id, expires_at
        FROM organizations WHERE id = target_org
        UNION ALL
        SELECT parent.id, parent.parent_id, parent.expires_at
        FROM chain child JOIN organizations parent ON parent.id = child.parent_id
    )
    SELECT min(expires_at) FROM chain;
$function$;

CREATE OR REPLACE FUNCTION public.org_path(org uuid)
 RETURNS text
 LANGUAGE sql
 IMMUTABLE
AS $function$
  WITH RECURSIVE up AS (
    SELECT id, name, parent_id, 1 AS d FROM organizations WHERE id = org
    UNION ALL
    SELECT o.id, o.name, o.parent_id, up.d + 1 FROM organizations o JOIN up ON o.id = up.parent_id
  )
  SELECT COALESCE(string_agg(name, ' / ' ORDER BY d DESC), '') FROM up;
$function$;

CREATE OR REPLACE FUNCTION public.protect_application_release_definition()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
BEGIN
    IF ROW(OLD.id, OLD.application_id, OLD.environment, OLD.version, OLD.entry_type,
           OLD.entry_url, OLD.health_url, OLD.resources, OLD.created_at, OLD.created_by)
       IS DISTINCT FROM
       ROW(NEW.id, NEW.application_id, NEW.environment, NEW.version, NEW.entry_type,
           NEW.entry_url, NEW.health_url, NEW.resources, NEW.created_at, NEW.created_by) THEN
        RAISE EXCEPTION 'application release definition is immutable' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$function$;

CREATE OR REPLACE FUNCTION public.reject_department_cycle()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
BEGIN
    IF NEW.parent_id IS NULL THEN
        RETURN NEW;
    END IF;
    -- 同组织部门层级变更串行化。
    PERFORM pg_advisory_xact_lock(hashtextextended(NEW.organization_id::text, 1));
    IF NEW.parent_id = NEW.id THEN
        RAISE EXCEPTION 'department cannot be its own parent';
    END IF;
    IF EXISTS (
        WITH RECURSIVE ancestors AS (
            SELECT d.id, d.parent_id, ARRAY[d.id] AS path
            FROM departments d WHERE d.id = NEW.parent_id
            UNION ALL
            SELECT parent.id, parent.parent_id, ancestors.path || parent.id
            FROM ancestors JOIN departments parent ON parent.id = ancestors.parent_id
            WHERE NOT parent.id = ANY(ancestors.path)
        )
        SELECT 1 FROM ancestors WHERE id = NEW.id
    ) THEN
        RAISE EXCEPTION 'department hierarchy cannot contain a cycle';
    END IF;
    RETURN NEW;
END
$function$;

CREATE OR REPLACE FUNCTION public.reject_notification_publication_update()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
BEGIN
    IF NEW IS DISTINCT FROM OLD THEN
        RAISE EXCEPTION 'notification publications are immutable' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END
$function$;

CREATE OR REPLACE FUNCTION public.reject_organization_cycle()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
BEGIN
    IF NEW.parent_id IS NULL THEN
        RETURN NEW;
    END IF;
    -- 同租户层级变更串行化，防并发事务各自通过检查后共同形成环路。
    PERFORM pg_advisory_xact_lock(hashtextextended(NEW.tenant_id::text, 0));
    IF NEW.parent_id = NEW.id THEN
        RAISE EXCEPTION 'organization cannot be its own parent';
    END IF;
    IF EXISTS (
        WITH RECURSIVE ancestors AS (
            SELECT o.id, o.parent_id, ARRAY[o.id] AS path
            FROM organizations o WHERE o.id = NEW.parent_id
            UNION ALL
            SELECT parent.id, parent.parent_id, ancestors.path || parent.id
            FROM ancestors JOIN organizations parent ON parent.id = ancestors.parent_id
            WHERE NOT parent.id = ANY(ancestors.path)
        )
        SELECT 1 FROM ancestors WHERE id = NEW.id
    ) THEN
        RAISE EXCEPTION 'organization hierarchy cannot contain a cycle';
    END IF;
    RETURN NEW;
END
$function$;

CREATE OR REPLACE FUNCTION public.revoke_app_tokens_on_app_code_change()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
BEGIN
    IF OLD.code IS DISTINCT FROM NEW.code THEN
        UPDATE oauth_tokens SET revoked = true
        WHERE token_use = 'app' AND application_id = NEW.id AND revoked = false;
    END IF;
    RETURN NEW;
END;
$function$;

CREATE OR REPLACE FUNCTION public.revoke_app_tokens_on_application_disable()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
BEGIN
    IF OLD.status = 'ENABLED' AND NEW.status <> 'ENABLED' THEN
        UPDATE oauth_tokens SET revoked = true
        WHERE token_use = 'app' AND application_id = NEW.id AND revoked = false;
    END IF;
    RETURN NEW;
END;
$function$;

CREATE OR REPLACE FUNCTION public.revoke_app_tokens_on_client_config_change()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
BEGIN
    IF OLD.application_id IS DISTINCT FROM NEW.application_id
       OR OLD.client_type IS DISTINCT FROM NEW.client_type
       OR OLD.scopes IS DISTINCT FROM NEW.scopes
       OR OLD.access_token_validity IS DISTINCT FROM NEW.access_token_validity THEN
        UPDATE oauth_tokens SET revoked = true
        WHERE token_use = 'app' AND client_id = NEW.client_id AND revoked = false;
    END IF;
    RETURN NEW;
END;
$function$;

CREATE OR REPLACE FUNCTION public.revoke_app_tokens_on_client_disable()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
BEGIN
    IF OLD.status = 'ENABLED' AND NEW.status <> 'ENABLED' THEN
        UPDATE oauth_tokens SET revoked = true
        WHERE token_use = 'app' AND client_id = NEW.client_id AND revoked = false;
    END IF;
    RETURN NEW;
END;
$function$;

CREATE OR REPLACE FUNCTION public.revoke_app_tokens_on_org_grant_delete()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
BEGIN
    UPDATE oauth_tokens SET revoked = true
    WHERE token_use = 'app' AND organization_id = OLD.organization_id
      AND application_id = OLD.application_id AND revoked = false;
    RETURN OLD;
END;
$function$;

CREATE OR REPLACE FUNCTION public.revoke_app_tokens_on_runtime_mode_change()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
BEGIN
    IF OLD.runtime_mode IS DISTINCT FROM NEW.runtime_mode THEN
        UPDATE oauth_tokens SET revoked = true
        WHERE token_use = 'app' AND application_id = NEW.id AND revoked = false;
    END IF;
    RETURN NEW;
END;
$function$;

CREATE OR REPLACE FUNCTION public.revoke_app_tokens_on_tenant_grant_delete()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
BEGIN
    UPDATE oauth_tokens SET revoked = true
    WHERE token_use = 'app' AND tenant_id = OLD.tenant_id
      AND application_id = OLD.application_id AND revoked = false;
    RETURN OLD;
END;
$function$;

CREATE OR REPLACE FUNCTION public.revoke_sessions_on_membership_delete()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
BEGIN
    UPDATE platform_sessions SET revoked_at = now()
    WHERE user_id = OLD.user_id AND organization_id = OLD.organization_id
      AND revoked_at IS NULL;
    RETURN OLD;
END;
$function$;

CREATE OR REPLACE FUNCTION public.revoke_sessions_on_user_disable()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
BEGIN
    IF OLD.status = 'ENABLED' AND NEW.status <> 'ENABLED' THEN
        UPDATE platform_sessions SET revoked_at = now()
        WHERE user_id = NEW.id AND revoked_at IS NULL;
    END IF;
    RETURN NEW;
END;
$function$;

CREATE OR REPLACE FUNCTION public.sync_sessions_on_organization_state()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
BEGIN
    IF NEW.status <> 'ENABLED' OR (NEW.expires_at IS NOT NULL AND NEW.expires_at <= now()) THEN
        WITH RECURSIVE descendants AS (
            SELECT NEW.id AS id
            UNION ALL
            SELECT child.id FROM organizations child
            JOIN descendants parent ON child.parent_id = parent.id
        )
        UPDATE platform_sessions SET revoked_at = now()
        WHERE organization_id IN (SELECT id FROM descendants) AND revoked_at IS NULL;
    ELSIF NEW.expires_at IS NOT NULL THEN
        WITH RECURSIVE descendants AS (
            SELECT NEW.id AS id
            UNION ALL
            SELECT child.id FROM organizations child
            JOIN descendants parent ON child.parent_id = parent.id
        )
        UPDATE platform_sessions SET expires_at = LEAST(expires_at, NEW.expires_at)
        WHERE organization_id IN (SELECT id FROM descendants) AND revoked_at IS NULL;
    END IF;
    RETURN NEW;
END;
$function$;

CREATE OR REPLACE FUNCTION public.sync_sessions_on_tenant_state()
 RETURNS trigger
 LANGUAGE plpgsql
AS $function$
BEGIN
    IF NEW.status <> 'ENABLED' OR (NEW.expires_at IS NOT NULL AND NEW.expires_at <= now()) THEN
        UPDATE platform_sessions SET revoked_at = now()
        WHERE tenant_id = NEW.id AND revoked_at IS NULL;
    ELSIF NEW.expires_at IS NOT NULL THEN
        UPDATE platform_sessions SET expires_at = LEAST(expires_at, NEW.expires_at)
        WHERE tenant_id = NEW.id AND revoked_at IS NULL;
    END IF;
    RETURN NEW;
END;
$function$;

-- Primary keys, unique constraints and checks.
ALTER TABLE public."application_deployments" ADD CONSTRAINT "application_deployments_check" CHECK (((previous_release_id IS NULL) OR (previous_release_id <> active_release_id)));
ALTER TABLE public."application_deployments" ADD CONSTRAINT "application_deployments_pkey" PRIMARY KEY (application_id, environment);
ALTER TABLE public."application_releases" ADD CONSTRAINT "application_releases_application_id_environment_id_key" UNIQUE (application_id, environment, id);
ALTER TABLE public."application_releases" ADD CONSTRAINT "application_releases_application_id_environment_version_key" UNIQUE (application_id, environment, version);
ALTER TABLE public."application_releases" ADD CONSTRAINT "application_releases_check" CHECK (((((validation_status)::text = 'DRAFT'::text) AND (checked_at IS NULL)) OR (((validation_status)::text = ANY (ARRAY[('PASSED'::character varying)::text, ('FAILED'::character varying)::text])) AND (checked_at IS NOT NULL))));
ALTER TABLE public."application_releases" ADD CONSTRAINT "application_releases_entry_type_check" CHECK (((entry_type)::text = ANY (ARRAY[('HTML'::character varying)::text, ('ESM'::character varying)::text])));
ALTER TABLE public."application_releases" ADD CONSTRAINT "application_releases_environment_check" CHECK (((environment)::text ~ '^[a-z0-9][a-z0-9_-]{0,31}$'::text));
ALTER TABLE public."application_releases" ADD CONSTRAINT "application_releases_pkey" PRIMARY KEY (id);
ALTER TABLE public."application_releases" ADD CONSTRAINT "application_releases_resources_check" CHECK ((jsonb_typeof(resources) = 'array'::text));
ALTER TABLE public."application_releases" ADD CONSTRAINT "application_releases_validation_status_check" CHECK (((validation_status)::text = ANY (ARRAY[('DRAFT'::character varying)::text, ('PASSED'::character varying)::text, ('FAILED'::character varying)::text])));
ALTER TABLE public."application_releases" ADD CONSTRAINT "application_releases_version_check" CHECK ((((version)::text ~ '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$'::text) AND ((version)::text !~~ '%..%'::text)));
ALTER TABLE public."applications" ADD CONSTRAINT "applications_code_key" UNIQUE (code);
ALTER TABLE public."applications" ADD CONSTRAINT "applications_pkey" PRIMARY KEY (id);
ALTER TABLE public."applications" ADD CONSTRAINT "applications_runtime_mode_check" CHECK (((runtime_mode)::text = ANY (ARRAY[('IFRAME'::character varying)::text, ('TRUSTED'::character varying)::text])));
ALTER TABLE public."audit_logs" ADD CONSTRAINT "audit_logs_pkey" PRIMARY KEY (id);
ALTER TABLE public."departments" ADD CONSTRAINT "departments_pkey" PRIMARY KEY (id);
ALTER TABLE public."membership_data_grants" ADD CONSTRAINT "ck_membership_data_grants_resource" CHECK (((length(TRIM(BOTH FROM resource_code)) > 0) AND (length(TRIM(BOTH FROM action_code)) > 0)));
ALTER TABLE public."membership_data_grants" ADD CONSTRAINT "ck_membership_data_grants_scope" CHECK (((scope_type)::text = ANY (ARRAY[('SELF'::character varying)::text, ('ORG'::character varying)::text, ('ORG_SUBTREE'::character varying)::text, ('TENANT'::character varying)::text])));
ALTER TABLE public."membership_data_grants" ADD CONSTRAINT "membership_data_grants_pkey" PRIMARY KEY (id);
ALTER TABLE public."membership_data_grants" ADD CONSTRAINT "uq_membership_data_grants" UNIQUE (membership_id, application_id, resource_code, action_code);
ALTER TABLE public."memberships" ADD CONSTRAINT "memberships_pkey" PRIMARY KEY (id);
ALTER TABLE public."memberships" ADD CONSTRAINT "memberships_user_id_organization_id_key" UNIQUE (user_id, organization_id);
ALTER TABLE public."menus" ADD CONSTRAINT "menus_pkey" PRIMARY KEY (id);
ALTER TABLE public."notification_prefs" ADD CONSTRAINT "notification_prefs_pkey" PRIMARY KEY (user_id);
ALTER TABLE public."notification_publications" ADD CONSTRAINT "notification_publications_check" CHECK ((delivery_count >= recipient_count));
ALTER TABLE public."notification_publications" ADD CONSTRAINT "notification_publications_id_tenant_id_application_id_key" UNIQUE (id, tenant_id, application_id);
ALTER TABLE public."notification_publications" ADD CONSTRAINT "notification_publications_pkey" PRIMARY KEY (id);
ALTER TABLE public."notification_publications" ADD CONSTRAINT "notification_publications_recipient_count_check" CHECK ((recipient_count >= 0));
ALTER TABLE public."notification_publications" ADD CONSTRAINT "notification_publications_request_body_check" CHECK ((jsonb_typeof(request_body) = 'object'::text));
ALTER TABLE public."notification_publications" ADD CONSTRAINT "notification_publications_tenant_id_publisher_organization__key" UNIQUE (tenant_id, publisher_organization_id, application_id, publisher_user_id, idempotency_key);
ALTER TABLE public."notifications" ADD CONSTRAINT "ck_notification_context" CHECK (((((source_kind)::text = 'LEGACY'::text) AND (tenant_id IS NULL) AND (organization_id IS NULL) AND (application_id IS NULL) AND (publication_id IS NULL)) OR (((source_kind)::text = 'APP'::text) AND (tenant_id IS NOT NULL) AND (organization_id IS NOT NULL) AND (application_id IS NOT NULL) AND (publication_id IS NOT NULL)) OR (((source_kind)::text = 'SYSTEM'::text) AND (tenant_id IS NOT NULL) AND (organization_id IS NOT NULL) AND (application_id IS NULL) AND (publication_id IS NULL))));
ALTER TABLE public."notifications" ADD CONSTRAINT "notifications_pkey" PRIMARY KEY (id);
ALTER TABLE public."notifications" ADD CONSTRAINT "uq_notification_publication_delivery" UNIQUE (publication_id, organization_id, user_id);
ALTER TABLE public."oauth_authorization_codes" ADD CONSTRAINT "oauth_authorization_codes_code_hash_key" UNIQUE (code_hash);
ALTER TABLE public."oauth_authorization_codes" ADD CONSTRAINT "oauth_authorization_codes_pkey" PRIMARY KEY (id);
ALTER TABLE public."oauth_authorization_requests" ADD CONSTRAINT "oauth_authorization_requests_pkey" PRIMARY KEY (id);
ALTER TABLE public."oauth_clients" ADD CONSTRAINT "oauth_clients_client_id_key" UNIQUE (client_id);
ALTER TABLE public."oauth_clients" ADD CONSTRAINT "oauth_clients_pkey" PRIMARY KEY (id);
ALTER TABLE public."oauth_device_codes" ADD CONSTRAINT "oauth_device_codes_device_code_hash_key" UNIQUE (device_code_hash);
ALTER TABLE public."oauth_device_codes" ADD CONSTRAINT "oauth_device_codes_pkey" PRIMARY KEY (id);
ALTER TABLE public."oauth_device_codes" ADD CONSTRAINT "oauth_device_codes_user_code_key" UNIQUE (user_code);
ALTER TABLE public."oauth_tokens" ADD CONSTRAINT "ck_oauth_tokens_app_binding" CHECK ((((token_use)::text <> 'app'::text) OR (((token_type)::text = 'access'::text) AND (user_id IS NOT NULL) AND (tenant_id IS NOT NULL) AND (organization_id IS NOT NULL) AND (audience IS NOT NULL) AND (jti IS NOT NULL) AND (application_id IS NOT NULL) AND (platform_session_id IS NOT NULL))));
ALTER TABLE public."oauth_tokens" ADD CONSTRAINT "oauth_tokens_pkey" PRIMARY KEY (id);
ALTER TABLE public."oauth_tokens" ADD CONSTRAINT "oauth_tokens_token_hash_key" UNIQUE (token_hash);
ALTER TABLE public."organization_applications" ADD CONSTRAINT "organization_applications_pkey" PRIMARY KEY (organization_id, application_id);
ALTER TABLE public."organization_menus" ADD CONSTRAINT "organization_menus_pkey" PRIMARY KEY (organization_id, menu_id);
ALTER TABLE public."organizations" ADD CONSTRAINT "ck_organization_coordinates" CHECK ((((longitude IS NULL) AND (latitude IS NULL)) OR ((longitude IS NOT NULL) AND (latitude IS NOT NULL) AND ((longitude >= ('-180'::integer)::double precision) AND (longitude <= (180)::double precision)) AND ((latitude >= ('-90'::integer)::double precision) AND (latitude <= (90)::double precision)))));
ALTER TABLE public."organizations" ADD CONSTRAINT "ck_organization_type" CHECK (((organization_type)::text = ANY (ARRAY[('COMPANY'::character varying)::text, ('SUBSIDIARY'::character varying)::text, ('BRANCH'::character varying)::text, ('PUBLIC_INSTITUTION'::character varying)::text, ('GOVERNMENT'::character varying)::text, ('OTHER'::character varying)::text])));
ALTER TABLE public."organizations" ADD CONSTRAINT "organizations_pkey" PRIMARY KEY (id);
ALTER TABLE public."platform_session_refresh_tokens" ADD CONSTRAINT "platform_session_refresh_tokens_check" CHECK ((expires_at > issued_at));
ALTER TABLE public."platform_session_refresh_tokens" ADD CONSTRAINT "platform_session_refresh_tokens_pkey" PRIMARY KEY (id);
ALTER TABLE public."platform_session_refresh_tokens" ADD CONSTRAINT "platform_session_refresh_tokens_session_version_check" CHECK ((session_version >= 0));
ALTER TABLE public."platform_session_refresh_tokens" ADD CONSTRAINT "platform_session_refresh_tokens_token_hash_key" UNIQUE (token_hash);
ALTER TABLE public."platform_sessions" ADD CONSTRAINT "platform_sessions_check" CHECK ((expires_at > issued_at));
ALTER TABLE public."platform_sessions" ADD CONSTRAINT "platform_sessions_pkey" PRIMARY KEY (id);
ALTER TABLE public."platform_settings" ADD CONSTRAINT "platform_settings_pkey" PRIMARY KEY (singleton);
ALTER TABLE public."platform_settings" ADD CONSTRAINT "platform_settings_primary_color_check" CHECK (((primary_color)::text ~ '^#[0-9A-Fa-f]{6}$'::text));
ALTER TABLE public."platform_settings" ADD CONSTRAINT "platform_settings_singleton_check" CHECK (singleton);
ALTER TABLE public."region_versions" ADD CONSTRAINT "region_versions_pkey" PRIMARY KEY (version);
ALTER TABLE public."regions" ADD CONSTRAINT "regions_pkey" PRIMARY KEY (version, source_id);
ALTER TABLE public."role_menus" ADD CONSTRAINT "role_menus_pkey" PRIMARY KEY (role_id, menu_id);
ALTER TABLE public."roles" ADD CONSTRAINT "roles_organization_id_name_key" UNIQUE (organization_id, name);
ALTER TABLE public."roles" ADD CONSTRAINT "roles_pkey" PRIMARY KEY (id);
ALTER TABLE public."tenant_applications" ADD CONSTRAINT "tenant_applications_pkey" PRIMARY KEY (tenant_id, application_id);
ALTER TABLE public."tenant_menus" ADD CONSTRAINT "tenant_menus_pkey" PRIMARY KEY (tenant_id, menu_id);
ALTER TABLE public."tenants" ADD CONSTRAINT "ck_tenant_coordinates" CHECK ((((longitude IS NULL) AND (latitude IS NULL)) OR ((longitude IS NOT NULL) AND (latitude IS NOT NULL) AND ((longitude >= ('-180'::integer)::double precision) AND (longitude <= (180)::double precision)) AND ((latitude >= ('-90'::integer)::double precision) AND (latitude <= (90)::double precision)))));
ALTER TABLE public."tenants" ADD CONSTRAINT "ck_tenant_type" CHECK (((tenant_type)::text = ANY (ARRAY[('PERSONAL'::character varying)::text, ('COMPANY'::character varying)::text, ('PUBLIC_INSTITUTION'::character varying)::text, ('GOVERNMENT'::character varying)::text, ('OTHER'::character varying)::text])));
ALTER TABLE public."tenants" ADD CONSTRAINT "tenants_code_key" UNIQUE (code);
ALTER TABLE public."tenants" ADD CONSTRAINT "tenants_pkey" PRIMARY KEY (id);
ALTER TABLE public."user_roles" ADD CONSTRAINT "user_roles_pkey" PRIMARY KEY (user_id, role_id);
ALTER TABLE public."users" ADD CONSTRAINT "users_pkey" PRIMARY KEY (id);
ALTER TABLE public."users" ADD CONSTRAINT "users_session_version_check" CHECK ((session_version >= 0));
ALTER TABLE public."users" ADD CONSTRAINT "users_username_key" UNIQUE (username);

-- Query and relationship indexes.
CREATE INDEX idx_audit_action ON public.audit_logs USING btree (action);
CREATE INDEX idx_audit_application_time ON public.audit_logs USING btree (application_id, created_at DESC) WHERE (application_id IS NOT NULL);
CREATE INDEX idx_audit_created ON public.audit_logs USING btree (created_at DESC);
CREATE INDEX idx_audit_request_id ON public.audit_logs USING btree (request_id) WHERE (request_id IS NOT NULL);
CREATE INDEX idx_departments_org ON public.departments USING btree (organization_id);
CREATE INDEX idx_departments_parent ON public.departments USING btree (parent_id);
CREATE INDEX idx_membership_data_grants_context ON public.membership_data_grants USING btree (membership_id, application_id);
CREATE INDEX idx_memberships_org ON public.memberships USING btree (organization_id);
CREATE INDEX idx_memberships_user ON public.memberships USING btree (user_id);
CREATE INDEX idx_menus_app ON public.menus USING btree (application_id);
CREATE INDEX idx_menus_parent ON public.menus USING btree (parent_id);
DO $$ BEGIN
  CREATE INDEX idx_notifications_content_trgm ON public.notifications USING gin (content gin_trgm_ops);
EXCEPTION WHEN OTHERS THEN
  RAISE NOTICE 'optional trigram index unavailable';
END $$;
CREATE INDEX idx_notifications_context_inbox ON public.notifications USING btree (user_id, tenant_id, organization_id, is_read, created_at DESC, id) WHERE ((source_kind)::text = ANY (ARRAY[('APP'::character varying)::text, ('SYSTEM'::character varying)::text]));
DO $$ BEGIN
  CREATE INDEX idx_notifications_title_trgm ON public.notifications USING gin (title gin_trgm_ops);
EXCEPTION WHEN OTHERS THEN
  RAISE NOTICE 'optional trigram index unavailable';
END $$;
CREATE INDEX idx_notifications_user_created ON public.notifications USING btree (user_id, created_at DESC);
CREATE INDEX idx_notifications_user_read ON public.notifications USING btree (user_id, is_read);
CREATE INDEX idx_notifications_user_read_created ON public.notifications USING btree (user_id, is_read, created_at DESC);
CREATE INDEX idx_oauth_clients_application ON public.oauth_clients USING btree (application_id);
CREATE INDEX idx_oauth_tokens_app_active ON public.oauth_tokens USING btree (application_id, expires_at DESC) WHERE (((token_use)::text = 'app'::text) AND (revoked = false));
CREATE INDEX idx_org_menus_menu ON public.organization_menus USING btree (menu_id);
CREATE INDEX idx_orgs_parent ON public.organizations USING btree (parent_id);
CREATE INDEX idx_orgs_tenant ON public.organizations USING btree (tenant_id);
CREATE INDEX idx_platform_session_refresh_tokens_session ON public.platform_session_refresh_tokens USING btree (session_id, expires_at DESC);
CREATE INDEX idx_platform_sessions_organization_active ON public.platform_sessions USING btree (organization_id, expires_at DESC) WHERE (revoked_at IS NULL);
CREATE INDEX idx_platform_sessions_user_active ON public.platform_sessions USING btree (user_id, expires_at DESC) WHERE (revoked_at IS NULL);
CREATE INDEX idx_regions_code ON public.regions USING btree (version, code);
CREATE INDEX idx_regions_name ON public.regions USING btree (version, name);
CREATE INDEX idx_regions_parent ON public.regions USING btree (version, parent_code, code);
CREATE UNIQUE INDEX idx_regions_version_code_unique ON public.regions USING btree (version, code);
CREATE INDEX idx_role_menus_menu ON public.role_menus USING btree (menu_id);
CREATE INDEX idx_roles_org ON public.roles USING btree (organization_id);
CREATE INDEX idx_roles_tenant ON public.roles USING btree (tenant_id);
CREATE INDEX idx_tenant_menus_menu ON public.tenant_menus USING btree (menu_id);
CREATE INDEX idx_tokens_client ON public.oauth_tokens USING btree (client_id);
CREATE INDEX idx_tokens_user ON public.oauth_tokens USING btree (user_id);
CREATE INDEX idx_user_roles_org ON public.user_roles USING btree (organization_id);
CREATE INDEX idx_user_roles_role ON public.user_roles USING btree (role_id);
CREATE INDEX idx_users_login_email ON public.users USING btree (lower((email)::text)) WHERE ((email)::text <> ''::text);
CREATE INDEX idx_users_login_phone ON public.users USING btree (regexp_replace((phone)::text, '[[:space:]()\-]'::text, ''::text, 'g'::text)) WHERE ((phone)::text <> ''::text);
CREATE UNIQUE INDEX uniq_tenants_platform ON public.tenants USING btree (is_platform) WHERE is_platform;
CREATE UNIQUE INDEX uq_departments_id_organization ON public.departments USING btree (id, organization_id);
CREATE UNIQUE INDEX uq_oauth_tokens_jti ON public.oauth_tokens USING btree (jti) WHERE (jti IS NOT NULL);
CREATE UNIQUE INDEX uq_org_root_per_tenant ON public.organizations USING btree (tenant_id) WHERE (parent_id IS NULL);
CREATE UNIQUE INDEX uq_organizations_id_tenant ON public.organizations USING btree (id, tenant_id);

-- Foreign keys, including tenant and organization boundaries.
ALTER TABLE public."application_deployments" ADD CONSTRAINT "application_deployments_application_id_environment_active__fkey" FOREIGN KEY (application_id, environment, active_release_id) REFERENCES application_releases(application_id, environment, id);
ALTER TABLE public."application_deployments" ADD CONSTRAINT "application_deployments_application_id_environment_previou_fkey" FOREIGN KEY (application_id, environment, previous_release_id) REFERENCES application_releases(application_id, environment, id);
ALTER TABLE public."application_deployments" ADD CONSTRAINT "application_deployments_application_id_fkey" FOREIGN KEY (application_id) REFERENCES applications(id) ON DELETE CASCADE;
ALTER TABLE public."application_releases" ADD CONSTRAINT "application_releases_application_id_fkey" FOREIGN KEY (application_id) REFERENCES applications(id) ON DELETE CASCADE;
ALTER TABLE public."departments" ADD CONSTRAINT "departments_organization_id_fkey" FOREIGN KEY (organization_id) REFERENCES organizations(id) ON DELETE CASCADE;
ALTER TABLE public."departments" ADD CONSTRAINT "departments_parent_id_fkey" FOREIGN KEY (parent_id) REFERENCES departments(id) ON DELETE CASCADE;
ALTER TABLE public."departments" ADD CONSTRAINT "fk_departments_parent_same_organization" FOREIGN KEY (parent_id, organization_id) REFERENCES departments(id, organization_id) DEFERRABLE;
ALTER TABLE public."membership_data_grants" ADD CONSTRAINT "membership_data_grants_application_id_fkey" FOREIGN KEY (application_id) REFERENCES applications(id) ON DELETE CASCADE;
ALTER TABLE public."membership_data_grants" ADD CONSTRAINT "membership_data_grants_membership_id_fkey" FOREIGN KEY (membership_id) REFERENCES memberships(id) ON DELETE CASCADE;
ALTER TABLE public."memberships" ADD CONSTRAINT "fk_memberships_department_same_organization" FOREIGN KEY (department_id, organization_id) REFERENCES departments(id, organization_id) DEFERRABLE;
ALTER TABLE public."memberships" ADD CONSTRAINT "memberships_department_id_fkey" FOREIGN KEY (department_id) REFERENCES departments(id) ON DELETE SET NULL;
ALTER TABLE public."memberships" ADD CONSTRAINT "memberships_organization_id_fkey" FOREIGN KEY (organization_id) REFERENCES organizations(id) ON DELETE CASCADE;
ALTER TABLE public."memberships" ADD CONSTRAINT "memberships_user_id_fkey" FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE public."menus" ADD CONSTRAINT "menus_application_id_fkey" FOREIGN KEY (application_id) REFERENCES applications(id) ON DELETE CASCADE;
ALTER TABLE public."menus" ADD CONSTRAINT "menus_parent_id_fkey" FOREIGN KEY (parent_id) REFERENCES menus(id) ON DELETE CASCADE;
ALTER TABLE public."notification_prefs" ADD CONSTRAINT "notification_prefs_user_id_fkey" FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE public."notification_publications" ADD CONSTRAINT "notification_publications_application_id_fkey" FOREIGN KEY (application_id) REFERENCES applications(id) ON DELETE CASCADE;
ALTER TABLE public."notification_publications" ADD CONSTRAINT "notification_publications_publisher_organization_id_tenant_fkey" FOREIGN KEY (publisher_organization_id, tenant_id) REFERENCES organizations(id, tenant_id) ON DELETE CASCADE;
ALTER TABLE public."notification_publications" ADD CONSTRAINT "notification_publications_tenant_id_fkey" FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE;
ALTER TABLE public."notifications" ADD CONSTRAINT "fk_notification_organization_tenant" FOREIGN KEY (organization_id, tenant_id) REFERENCES organizations(id, tenant_id) ON DELETE CASCADE;
ALTER TABLE public."notifications" ADD CONSTRAINT "fk_notification_publication_context" FOREIGN KEY (publication_id, tenant_id, application_id) REFERENCES notification_publications(id, tenant_id, application_id) ON DELETE CASCADE;
ALTER TABLE public."notifications" ADD CONSTRAINT "notifications_application_id_fkey" FOREIGN KEY (application_id) REFERENCES applications(id) ON DELETE CASCADE;
ALTER TABLE public."notifications" ADD CONSTRAINT "notifications_tenant_id_fkey" FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE;
ALTER TABLE public."notifications" ADD CONSTRAINT "notifications_user_id_fkey" FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE public."oauth_clients" ADD CONSTRAINT "oauth_clients_application_id_fkey" FOREIGN KEY (application_id) REFERENCES applications(id) ON DELETE SET NULL;
ALTER TABLE public."oauth_tokens" ADD CONSTRAINT "oauth_tokens_application_id_fkey" FOREIGN KEY (application_id) REFERENCES applications(id) ON DELETE CASCADE;
ALTER TABLE public."oauth_tokens" ADD CONSTRAINT "oauth_tokens_platform_session_id_fkey" FOREIGN KEY (platform_session_id) REFERENCES platform_sessions(id) ON DELETE CASCADE;
ALTER TABLE public."organization_applications" ADD CONSTRAINT "organization_applications_application_id_fkey" FOREIGN KEY (application_id) REFERENCES applications(id) ON DELETE CASCADE;
ALTER TABLE public."organization_applications" ADD CONSTRAINT "organization_applications_organization_id_fkey" FOREIGN KEY (organization_id) REFERENCES organizations(id) ON DELETE CASCADE;
ALTER TABLE public."organization_menus" ADD CONSTRAINT "organization_menus_menu_id_fkey" FOREIGN KEY (menu_id) REFERENCES menus(id) ON DELETE CASCADE;
ALTER TABLE public."organization_menus" ADD CONSTRAINT "organization_menus_organization_id_fkey" FOREIGN KEY (organization_id) REFERENCES organizations(id) ON DELETE CASCADE;
ALTER TABLE public."organizations" ADD CONSTRAINT "fk_organizations_parent_same_tenant" FOREIGN KEY (parent_id, tenant_id) REFERENCES organizations(id, tenant_id) DEFERRABLE;
ALTER TABLE public."organizations" ADD CONSTRAINT "organizations_parent_id_fkey" FOREIGN KEY (parent_id) REFERENCES organizations(id) ON DELETE CASCADE;
ALTER TABLE public."organizations" ADD CONSTRAINT "organizations_tenant_id_fkey" FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE;
ALTER TABLE public."platform_session_refresh_tokens" ADD CONSTRAINT "platform_session_refresh_tokens_session_id_fkey" FOREIGN KEY (session_id) REFERENCES platform_sessions(id) ON DELETE CASCADE;
ALTER TABLE public."platform_sessions" ADD CONSTRAINT "platform_sessions_organization_id_fkey" FOREIGN KEY (organization_id) REFERENCES organizations(id) ON DELETE CASCADE;
ALTER TABLE public."platform_sessions" ADD CONSTRAINT "platform_sessions_tenant_id_fkey" FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE;
ALTER TABLE public."platform_sessions" ADD CONSTRAINT "platform_sessions_user_id_fkey" FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE public."regions" ADD CONSTRAINT "regions_version_fkey" FOREIGN KEY (version) REFERENCES region_versions(version) ON DELETE CASCADE;
ALTER TABLE public."role_menus" ADD CONSTRAINT "role_menus_menu_id_fkey" FOREIGN KEY (menu_id) REFERENCES menus(id) ON DELETE CASCADE;
ALTER TABLE public."role_menus" ADD CONSTRAINT "role_menus_role_id_fkey" FOREIGN KEY (role_id) REFERENCES roles(id) ON DELETE CASCADE;
ALTER TABLE public."roles" ADD CONSTRAINT "fk_roles_organization_same_tenant" FOREIGN KEY (organization_id, tenant_id) REFERENCES organizations(id, tenant_id) DEFERRABLE;
ALTER TABLE public."roles" ADD CONSTRAINT "roles_organization_id_fkey" FOREIGN KEY (organization_id) REFERENCES organizations(id) ON DELETE CASCADE;
ALTER TABLE public."roles" ADD CONSTRAINT "roles_tenant_id_fkey" FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE;
ALTER TABLE public."tenant_applications" ADD CONSTRAINT "tenant_applications_application_id_fkey" FOREIGN KEY (application_id) REFERENCES applications(id) ON DELETE CASCADE;
ALTER TABLE public."tenant_applications" ADD CONSTRAINT "tenant_applications_tenant_id_fkey" FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE;
ALTER TABLE public."tenant_menus" ADD CONSTRAINT "tenant_menus_menu_id_fkey" FOREIGN KEY (menu_id) REFERENCES menus(id) ON DELETE CASCADE;
ALTER TABLE public."tenant_menus" ADD CONSTRAINT "tenant_menus_tenant_id_fkey" FOREIGN KEY (tenant_id) REFERENCES tenants(id) ON DELETE CASCADE;
ALTER TABLE public."user_roles" ADD CONSTRAINT "user_roles_organization_id_fkey" FOREIGN KEY (organization_id) REFERENCES organizations(id) ON DELETE CASCADE;
ALTER TABLE public."user_roles" ADD CONSTRAINT "user_roles_role_id_fkey" FOREIGN KEY (role_id) REFERENCES roles(id) ON DELETE CASCADE;
ALTER TABLE public."user_roles" ADD CONSTRAINT "user_roles_user_id_fkey" FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;

-- Triggers.
CREATE TRIGGER application_release_definition_immutable BEFORE UPDATE ON public.application_releases FOR EACH ROW EXECUTE FUNCTION protect_application_release_definition();
CREATE TRIGGER trg_app_token_application_revoke AFTER UPDATE OF status ON public.applications FOR EACH ROW EXECUTE FUNCTION revoke_app_tokens_on_application_disable();
CREATE TRIGGER trg_app_token_code_revoke AFTER UPDATE OF code ON public.applications FOR EACH ROW EXECUTE FUNCTION revoke_app_tokens_on_app_code_change();
CREATE TRIGGER trg_app_token_runtime_mode_revoke AFTER UPDATE OF runtime_mode ON public.applications FOR EACH ROW EXECUTE FUNCTION revoke_app_tokens_on_runtime_mode_change();
CREATE TRIGGER trg_departments_no_cycle BEFORE INSERT OR UPDATE OF parent_id ON public.departments FOR EACH ROW EXECUTE FUNCTION reject_department_cycle();
CREATE TRIGGER trg_membership_session_revoke AFTER DELETE ON public.memberships FOR EACH ROW EXECUTE FUNCTION revoke_sessions_on_membership_delete();
CREATE TRIGGER trg_notification_publication_immutable BEFORE UPDATE ON public.notification_publications FOR EACH ROW EXECUTE FUNCTION reject_notification_publication_update();
CREATE TRIGGER trg_app_token_client_config_revoke AFTER UPDATE OF application_id, client_type, scopes, access_token_validity ON public.oauth_clients FOR EACH ROW EXECUTE FUNCTION revoke_app_tokens_on_client_config_change();
CREATE TRIGGER trg_app_token_client_revoke AFTER UPDATE OF status ON public.oauth_clients FOR EACH ROW EXECUTE FUNCTION revoke_app_tokens_on_client_disable();
CREATE TRIGGER trg_app_token_org_grant_revoke AFTER DELETE ON public.organization_applications FOR EACH ROW EXECUTE FUNCTION revoke_app_tokens_on_org_grant_delete();
CREATE TRIGGER trg_organization_session_state AFTER UPDATE OF status, expires_at ON public.organizations FOR EACH ROW EXECUTE FUNCTION sync_sessions_on_organization_state();
CREATE TRIGGER trg_organizations_no_cycle BEFORE INSERT OR UPDATE OF parent_id ON public.organizations FOR EACH ROW EXECUTE FUNCTION reject_organization_cycle();
CREATE TRIGGER trg_app_token_tenant_grant_revoke AFTER DELETE ON public.tenant_applications FOR EACH ROW EXECUTE FUNCTION revoke_app_tokens_on_tenant_grant_delete();
CREATE TRIGGER trg_tenant_session_state AFTER UPDATE OF status, expires_at ON public.tenants FOR EACH ROW EXECUTE FUNCTION sync_sessions_on_tenant_state();
CREATE TRIGGER trg_user_session_state AFTER UPDATE OF status ON public.users FOR EACH ROW EXECUTE FUNCTION revoke_sessions_on_user_disable();
CREATE TRIGGER trg_user_session_version BEFORE UPDATE OF status ON public.users FOR EACH ROW EXECUTE FUNCTION bump_session_version_on_user_disable();

-- Table and column documentation.
COMMENT ON COLUMN public."organizations"."township" IS '乡镇、街道地区名称';
COMMENT ON COLUMN public."organizations"."village" IS '村、社区地区名称';
COMMENT ON TABLE public."platform_settings" IS '平台全局品牌与登录页配置；唯一单行记录';
COMMENT ON COLUMN public."platform_settings"."singleton" IS '固定为 true，保证最多一条平台设置';
COMMENT ON COLUMN public."platform_settings"."platform_name" IS '平台展示名称';
COMMENT ON COLUMN public."platform_settings"."platform_description" IS '平台展示描述';
COMMENT ON COLUMN public."platform_settings"."logo_url" IS '平台标识的站内路径或 HTTP/HTTPS 地址';
COMMENT ON COLUMN public."platform_settings"."login_title" IS '登录页标题';
COMMENT ON COLUMN public."platform_settings"."login_description" IS '登录页说明';
COMMENT ON COLUMN public."platform_settings"."footer_text" IS '登录页页脚文案';
COMMENT ON COLUMN public."platform_settings"."updated_at" IS '最后修改时间';
COMMENT ON COLUMN public."platform_settings"."primary_color" IS '平台主题主色（#RRGGBB）；前端生成兼顾对比度的明暗主题与导航配色';
COMMENT ON TABLE public."region_versions" IS '地区数据版本';
COMMENT ON COLUMN public."region_versions"."version" IS '版本标识，如2023';
COMMENT ON COLUMN public."region_versions"."label" IS '版本名称，如2023版';
COMMENT ON COLUMN public."region_versions"."source_version" IS '复制来源版本';
COMMENT ON COLUMN public."region_versions"."imported" IS '是否已完成内置数据导入';
COMMENT ON COLUMN public."region_versions"."created_at" IS '创建时间';
COMMENT ON COLUMN public."region_versions"."updated_at" IS '更新时间';
COMMENT ON COLUMN public."region_versions"."deleted_at" IS '版本删除时间；保留墓碑防止内置版本重启后重新导入';
COMMENT ON TABLE public."regions" IS '按版本维护的地区记录；导入时对源文件完全重复的编码去重';
COMMENT ON COLUMN public."regions"."version" IS '地区数据版本';
COMMENT ON COLUMN public."regions"."source_id" IS '原始主键或新建记录主键';
COMMENT ON COLUMN public."regions"."code" IS '地区编码';
COMMENT ON COLUMN public."regions"."parent_code" IS '父级编码';
COMMENT ON COLUMN public."regions"."name" IS '地区名称';
COMMENT ON COLUMN public."regions"."name_pinyin" IS '地区拼音';
COMMENT ON COLUMN public."regions"."name_short" IS '地区拼音简称';
COMMENT ON COLUMN public."regions"."level" IS '层级';
COMMENT ON COLUMN public."regions"."type" IS '地区类型';
COMMENT ON COLUMN public."regions"."type_name" IS '地区类型名称 111:主城区 112:城乡结合区 121:镇中心区 122:镇乡结合区 123:特殊区域 210:乡中心区 220:村庄';
COMMENT ON COLUMN public."regions"."post_code" IS '邮编';
COMMENT ON COLUMN public."regions"."source_url" IS '获取地区链接';
COMMENT ON COLUMN public."regions"."created_at" IS '创建时间';
COMMENT ON COLUMN public."regions"."updated_at" IS '更新时间';
COMMENT ON COLUMN public."tenants"."township" IS '乡镇、街道地区名称';
COMMENT ON COLUMN public."tenants"."village" IS '村、社区地区名称';

-- Built-in micro applications; identities are created per installation.
INSERT INTO public.applications ("code", "name", "icon", "base_path", "description", "sort_order", "status", "entry_type", "entry_url", "runtime_mode") VALUES ('vue-workshop', 'Vue 工作台', 'Sparkles', '/micro/vue-workshop', '微前端演示: Vue 3 子应用(HTML 入口+沙箱)', '1', 'ENABLED', 'HTML', '/sub/vue/', 'IFRAME');
INSERT INTO public.applications ("code", "name", "icon", "base_path", "description", "sort_order", "status", "entry_type", "entry_url", "runtime_mode") VALUES ('legacy-jquery', '遗留工单(jQuery)', 'Wrench', '/micro/legacy-jquery', '微前端演示: jQuery 遗留系统子应用', '2', 'ENABLED', 'HTML', '/sub/jquery/', 'IFRAME');
INSERT INTO public.applications ("code", "name", "icon", "base_path", "description", "sort_order", "status", "entry_type", "entry_url", "runtime_mode") VALUES ('report-center', '报表中心', 'BarChart3', '/micro/report-center', '第一方 ESM 报表中心；随宿主构建并直挂', '3', 'ENABLED', 'ESM', '/src/micro-apps/report-center.tsx', 'TRUSTED');
INSERT INTO public.menus (application_id, "parent_id", "type", "title", "path", "component", "icon", "permission", "sort_order", "visible", "status", "link_url", "link_target") SELECT id, NULL, 'MENU', '工单列表', '/micro/legacy-jquery', '', '', '', '1', 'true', 'ENABLED', '', 'MICRO' FROM public.applications WHERE code = 'legacy-jquery';
INSERT INTO public.menus (application_id, "parent_id", "type", "title", "path", "component", "icon", "permission", "sort_order", "visible", "status", "link_url", "link_target") SELECT id, NULL, 'MENU', '数据总览', '/micro/report-center', '', '', '', '1', 'true', 'ENABLED', '', 'MICRO' FROM public.applications WHERE code = 'report-center';
INSERT INTO public.menus (application_id, "parent_id", "type", "title", "path", "component", "icon", "permission", "sort_order", "visible", "status", "link_url", "link_target") SELECT id, NULL, 'MENU', '共享运行时', '/micro/report-center/shared', '', '', '', '2', 'true', 'ENABLED', '', 'MICRO' FROM public.applications WHERE code = 'report-center';
INSERT INTO public.menus (application_id, "parent_id", "type", "title", "path", "component", "icon", "permission", "sort_order", "visible", "status", "link_url", "link_target") SELECT id, NULL, 'MENU', '总览', '/micro/vue-workshop', '', '', '', '1', 'true', 'ENABLED', '', 'MICRO' FROM public.applications WHERE code = 'vue-workshop';
INSERT INTO public.menus (application_id, "parent_id", "type", "title", "path", "component", "icon", "permission", "sort_order", "visible", "status", "link_url", "link_target") SELECT id, NULL, 'MENU', '权限与沙箱', '/micro/vue-workshop/security', '', '', '', '2', 'true', 'ENABLED', '', 'MICRO' FROM public.applications WHERE code = 'vue-workshop';

-- One global branding record; tenant identity remains separate.
INSERT INTO public.platform_settings (singleton) VALUES (true);
