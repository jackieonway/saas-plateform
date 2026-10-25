-- One canonical migration chain; runtime services never migrate or seed.
-- Cluster roles and object ownership are assigned by the explicitly controlled
-- ops/identity-deployment/provision.sql job after this migration commits.

CREATE TABLE public.identity_deployment_state (
    singleton boolean PRIMARY KEY CHECK (singleton),
    deployment_id uuid NOT NULL CHECK (public.is_uuid_v7(deployment_id)),
    issuer text NOT NULL,
    schema_version bigint NOT NULL CHECK (schema_version=20),
    migration_owner name NOT NULL,
    identity_owner name NOT NULL,
    platform_owner name NOT NULL,
    revocation_bridge_owner name NOT NULL,
    projection_owner name NOT NULL,
    identity_runtime name NOT NULL,
    platform_runtime name NOT NULL,
    ready_at timestamptz NOT NULL
);
COMMENT ON TABLE public.identity_deployment_state IS
    'Migration-job-owned sealed deployment marker; runtime must match current_user, fixed issuer and schema version. Not a production readiness claim.';

CREATE TABLE public.identity_service_clients (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    secret_hash text NOT NULL UNIQUE CHECK (secret_hash ~ '^[0-9a-f]{64}$'),
    purpose text NOT NULL CHECK (purpose IN ('ACCOUNT_COMMAND','CUSTOMER_ACCESS_PROOF','OIDC_ADMIN')),
    status text NOT NULL CHECK (status IN ('ENABLED','DISABLED')),
    created_at timestamptz NOT NULL
);
COMMENT ON TABLE public.identity_service_clients IS
    'Authentication-owned single-purpose internal callers. Runtime reads hash-only registration; provisioning owner alone writes.';

CREATE TABLE public.identity_audit_logs (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    user_id uuid CHECK (user_id IS NULL OR public.is_uuid_v7(user_id)),
    username text NOT NULL,
    action text NOT NULL,
    detail text NOT NULL,
    ip text NOT NULL,
    user_agent text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
COMMENT ON TABLE public.identity_audit_logs IS
    'Authentication audit only; no platform tenant, organization, application or authorization dependency.';

CREATE TABLE public.platform_account_command_intents (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    target_user_id uuid NOT NULL CHECK (public.is_uuid_v7(target_user_id)),
    actor_user_id uuid NOT NULL CHECK (public.is_uuid_v7(actor_user_id)),
    actor_tenant_id uuid NOT NULL CHECK (public.is_uuid_v7(actor_tenant_id)),
    actor_organization_id uuid NOT NULL CHECK (public.is_uuid_v7(actor_organization_id)),
    actor_session_id uuid NOT NULL CHECK (public.is_uuid_v7(actor_session_id)),
    expected_account_version bigint NOT NULL CHECK (expected_account_version>=0),
    canonical_digest text NOT NULL CHECK (canonical_digest ~ '^[0-9a-f]{64}$'),
    command_kind text NOT NULL CHECK (command_kind IN
        ('CREATE','UPDATE_PROFILE','SET_STATUS','RESET_PASSWORD','CHANGE_PASSWORD','REVOKE_ALL','RETIRE')),
    expires_at timestamptz NOT NULL,
    state text NOT NULL CHECK (state IN ('PENDING_SECRET','UNKNOWN','COMPLETED','ABANDONED','REJECTED')),
    followup jsonb NOT NULL CHECK (jsonb_typeof(followup)='object'),
    receipt jsonb CHECK (receipt IS NULL OR jsonb_typeof(receipt)='object'),
    created_at timestamptz NOT NULL,
    completed_at timestamptz,
    CHECK (expires_at>created_at),
    CHECK (state<>'COMPLETED' OR completed_at IS NOT NULL)
);
CREATE INDEX idx_platform_account_commands_target ON public.platform_account_command_intents(target_user_id,created_at);
COMMENT ON TABLE public.platform_account_command_intents IS
    'Platform-owned durable exact actor/target precondition and nonsecret followup only. No password, PHC, encrypted password, token or client secret is permitted by the writer protocol.';

CREATE TABLE public.identity_account_tombstones (
    user_id uuid PRIMARY KEY REFERENCES public.users(id) CHECK (public.is_uuid_v7(user_id)),
    operation_id uuid NOT NULL UNIQUE CHECK (public.is_uuid_v7(operation_id)),
    retired_at timestamptz NOT NULL
);
COMMENT ON TABLE public.identity_account_tombstones IS
    'Authentication-owned irreversible retirement marker. Runtime cannot DELETE users or cascade platform relations.';

CREATE TABLE public.identity_account_command_receipts (
    operation_id uuid PRIMARY KEY CHECK (public.is_uuid_v7(operation_id)),
    service_id uuid NOT NULL REFERENCES public.identity_service_clients(id) CHECK (public.is_uuid_v7(service_id)),
    target_user_id uuid NOT NULL REFERENCES public.users(id) CHECK (public.is_uuid_v7(target_user_id)),
    canonical_digest text NOT NULL CHECK (canonical_digest ~ '^[0-9a-f]{64}$'),
    state text NOT NULL CHECK (state='COMPLETED'),
    before_version bigint NOT NULL CHECK (before_version>=0),
    after_version bigint NOT NULL CHECK (after_version>=0),
    revoked_sessions bigint NOT NULL CHECK (revoked_sessions>=0),
    revoked_tokens bigint NOT NULL CHECK (revoked_tokens>=0),
    created_at timestamptz NOT NULL
);
COMMENT ON TABLE public.identity_account_command_receipts IS
    'Authentication-owned committed result; exact operation/service/digest binding. Contains no secret request payload.';

CREATE TABLE public.identity_account_revocation_facts (
    user_id uuid NOT NULL CHECK (public.is_uuid_v7(user_id)),
    account_version bigint NOT NULL CHECK (account_version>=0),
    platform_session_count bigint NOT NULL CHECK (platform_session_count>=0),
    oauth_token_count bigint NOT NULL CHECK (oauth_token_count>=0),
    created_at timestamptz NOT NULL,
    PRIMARY KEY(user_id,account_version)
);
COMMENT ON TABLE public.identity_account_revocation_facts IS
    'Authentication-owned exact generation transaction fact. Only invalidation bridge inserts/adds actual affected-row counts; identity runtime only reads.';

CREATE TABLE public.platform_oidc_command_intents (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    actor_user_id uuid NOT NULL CHECK (public.is_uuid_v7(actor_user_id)),
    tenant_id uuid NOT NULL CHECK (public.is_uuid_v7(tenant_id)),
    organization_id uuid NOT NULL CHECK (public.is_uuid_v7(organization_id)),
    session_id uuid NOT NULL CHECK (public.is_uuid_v7(session_id)),
    service_client_id uuid NOT NULL CHECK (public.is_uuid_v7(service_client_id)),
    request_digest text NOT NULL CHECK (request_digest ~ '^[0-9a-f]{64}$'),
    method text NOT NULL CHECK (method IN ('POST','PATCH','DELETE')),
    path text NOT NULL CHECK (path ~ '^/internal/identity/oidc/(clients|keys)(/[A-Za-z0-9_-]+){0,2}$'),
    accepted_at timestamptz NOT NULL
);
COMMENT ON TABLE public.platform_oidc_command_intents IS
    'Platform-owned immutable root/system:oauth acceptance point, service-bound digest only; no request body or secret.';

CREATE TABLE public.identity_oidc_admin_receipts (
    operation_id uuid PRIMARY KEY CHECK (public.is_uuid_v7(operation_id)),
    service_id uuid NOT NULL REFERENCES public.identity_service_clients(id) CHECK (public.is_uuid_v7(service_id)),
    request_digest text NOT NULL CHECK (request_digest ~ '^[0-9a-f]{64}$'),
    service_secret_hash text NOT NULL CHECK (service_secret_hash ~ '^[0-9a-f]{64}$'),
    method text NOT NULL CHECK (method IN ('POST','PATCH','DELETE')),
    path text NOT NULL CHECK (path ~ '^/internal/identity/oidc/(clients|keys)(/[A-Za-z0-9_-]+){0,2}$'),
    result_json jsonb NOT NULL CHECK (jsonb_typeof(result_json)='object'),
    returns_secret boolean NOT NULL,
    created_at timestamptz NOT NULL
);
COMMENT ON TABLE public.identity_oidc_admin_receipts IS
    'Authentication-owned service/key/digest/method/path-bound safe result only. Raw newly issued client secrets are never persisted.';

CREATE FUNCTION public.identity_oidc_command_intent_active(
    operation_ref uuid,service_ref uuid,digest_ref text,method_ref text,path_ref text)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER
SET search_path=pg_catalog,public AS $$
BEGIN
    PERFORM 1 FROM public.platform_oidc_command_intents intent
    WHERE intent.id=operation_ref AND intent.service_client_id=service_ref
      AND intent.request_digest=digest_ref AND intent.method=method_ref AND intent.path=path_ref
      AND intent.accepted_at<=pg_catalog.clock_timestamp()
      AND intent.accepted_at>pg_catalog.clock_timestamp()-interval '60 seconds'
    FOR SHARE OF intent;
    IF NOT FOUND THEN RETURN false; END IF;
    -- A lock-only wait may leave the original tuple unchanged. Sample the
    -- deadline again in a new statement after acquiring that lock.
    RETURN EXISTS(SELECT 1 FROM public.platform_oidc_command_intents intent
      WHERE intent.id=operation_ref AND intent.service_client_id=service_ref
        AND intent.request_digest=digest_ref AND intent.method=method_ref AND intent.path=path_ref
        AND intent.accepted_at<=pg_catalog.clock_timestamp()
        AND intent.accepted_at>pg_catalog.clock_timestamp()-interval '60 seconds');
END $$;
REVOKE ALL ON FUNCTION public.identity_oidc_command_intent_active(uuid,uuid,text,text,text) FROM PUBLIC;

CREATE FUNCTION public.platform_account_command_intent_active(
    operation_ref uuid,target_ref uuid,account_generation bigint,digest_ref text,kind_ref text)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER
SET search_path=pg_catalog,public AS $$
BEGIN
    PERFORM 1 FROM public.platform_account_command_intents intent
    WHERE intent.id=operation_ref AND intent.target_user_id=target_ref
      AND intent.expected_account_version=account_generation
      AND intent.canonical_digest=digest_ref AND intent.command_kind=kind_ref
      AND intent.state IN ('PENDING_SECRET','UNKNOWN')
      AND intent.expires_at>pg_catalog.clock_timestamp()
      AND intent.expires_at<=intent.created_at+interval '60 seconds'
    FOR SHARE OF intent;
    IF NOT FOUND THEN RETURN false; END IF;
    RETURN EXISTS(SELECT 1 FROM public.platform_account_command_intents intent
      WHERE intent.id=operation_ref AND intent.target_user_id=target_ref
        AND intent.expected_account_version=account_generation
        AND intent.canonical_digest=digest_ref AND intent.command_kind=kind_ref
        AND intent.state IN ('PENDING_SECRET','UNKNOWN')
        AND intent.expires_at>pg_catalog.clock_timestamp()
        AND intent.expires_at<=intent.created_at+interval '60 seconds');
END $$;
REVOKE ALL ON FUNCTION public.platform_account_command_intent_active(uuid,uuid,bigint,text,text) FROM PUBLIC;

-- This cross-owner permission is only a boolean, transaction-scoped personal
-- password precondition. Identity first locks users; the function then locks
-- the original exact platform session/member, so mutation cannot outlive a
-- member/session revocation that committed before its lock. No identity table
-- read privilege is granted to the platform owner beyond its safe users columns.
CREATE FUNCTION public.platform_password_context_active(
    subject_ref uuid,session_ref uuid,tenant_ref uuid,organization_ref uuid,account_generation bigint)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER
SET search_path=pg_catalog,public AS $$
DECLARE member_ref uuid;
BEGIN
    SELECT s.membership_id INTO member_ref FROM public.platform_sessions s
    WHERE s.id=session_ref AND s.user_id=subject_ref AND s.tenant_id=tenant_ref
      AND s.organization_id=organization_ref AND s.account_version=account_generation
      AND s.member_type='EMPLOYEE';
    IF NOT FOUND THEN RETURN false; END IF;
    PERFORM 1 FROM public.memberships m WHERE m.id=member_ref AND m.user_id=subject_ref
      AND m.organization_id=organization_ref AND m.member_type='EMPLOYEE' FOR SHARE OF m;
    IF NOT FOUND THEN RETURN false; END IF;
    -- Membership revocation updates its row before its trigger touches sessions.
    -- Lock in that same member -> session order to avoid an ABBA cycle.
    PERFORM 1 FROM public.platform_sessions s WHERE s.id=session_ref
      AND s.user_id=subject_ref AND s.tenant_id=tenant_ref AND s.organization_id=organization_ref
      AND s.account_version=account_generation AND s.membership_id=member_ref
      AND s.member_type='EMPLOYEE' FOR SHARE OF s;
    IF NOT FOUND THEN RETURN false; END IF;
    RETURN EXISTS(SELECT 1 FROM public.platform_sessions s WHERE s.id=session_ref
      AND s.user_id=subject_ref AND s.tenant_id=tenant_ref AND s.organization_id=organization_ref
      AND s.account_version=account_generation AND s.membership_id=member_ref
      AND s.member_type='EMPLOYEE' AND public.platform_session_active(s.id));
END $$;
REVOKE ALL ON FUNCTION public.platform_password_context_active(uuid,uuid,uuid,uuid,bigint) FROM PUBLIC;

-- These two functions are the only cross-owner invalidation bridge. Their
-- NOLOGIN owner receives SELECT plus UPDATE of invalidation fields only, no
-- platform INSERT/DELETE, no membership/role/policy writes and no role inheritance.
-- Their only INSERT is the authentication-owned, generation-bound count fact.
-- Trigger OLD/NEW supplies the target. Neither runtime gets direct EXECUTE.
CREATE OR REPLACE FUNCTION public.revoke_sessions_on_user_disable()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path=pg_catalog,public AS $$
DECLARE revoked_sessions bigint;
BEGIN
    IF OLD.status='ENABLED' AND NEW.status<>'ENABLED' THEN
        UPDATE public.platform_sessions SET revoked_at=pg_catalog.now()
        WHERE user_id=NEW.id AND revoked_at IS NULL;
        GET DIAGNOSTICS revoked_sessions=ROW_COUNT;
        INSERT INTO public.identity_account_revocation_facts AS fact(user_id,account_version,platform_session_count,oauth_token_count,created_at)
        VALUES(NEW.id,NEW.session_version,revoked_sessions,0,pg_catalog.clock_timestamp())
        ON CONFLICT(user_id,account_version) DO UPDATE SET
            platform_session_count=fact.platform_session_count+EXCLUDED.platform_session_count;
    END IF;
    RETURN NEW;
END $$;

CREATE OR REPLACE FUNCTION public.revoke_credentials_on_account_generation()
RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER
SET search_path=pg_catalog,public AS $$
DECLARE revoked_sessions bigint; revoked_tokens bigint;
BEGIN
    IF NEW.session_version<OLD.session_version THEN
        RAISE EXCEPTION 'account credential version cannot decrease' USING ERRCODE='23514';
    END IF;
    IF NEW.session_version IS DISTINCT FROM OLD.session_version
       OR (OLD.status='ENABLED' AND NEW.status<>'ENABLED') THEN
        UPDATE public.platform_sessions SET revoked_at=pg_catalog.clock_timestamp()
            WHERE user_id=NEW.id AND revoked_at IS NULL;
        GET DIAGNOSTICS revoked_sessions=ROW_COUNT;
        UPDATE public.platform_session_refresh_tokens r SET consumed_at=pg_catalog.clock_timestamp()
            FROM public.platform_sessions s WHERE r.session_id=s.id
            AND s.user_id=NEW.id AND r.consumed_at IS NULL;
        UPDATE public.oauth_tokens SET revoked=true WHERE user_id=NEW.id AND NOT revoked;
        GET DIAGNOSTICS revoked_tokens=ROW_COUNT;
        UPDATE public.oauth_authorization_codes SET consumed=true WHERE user_id=NEW.id AND NOT consumed;
        UPDATE public.oauth_device_codes SET status='EXPIRED'
            WHERE user_id=NEW.id AND status IN ('PENDING','APPROVED');
        INSERT INTO public.identity_account_revocation_facts AS fact(user_id,account_version,platform_session_count,oauth_token_count,created_at)
        VALUES(NEW.id,NEW.session_version,revoked_sessions,revoked_tokens,pg_catalog.clock_timestamp())
        ON CONFLICT(user_id,account_version) DO UPDATE SET
            platform_session_count=fact.platform_session_count+EXCLUDED.platform_session_count,
            oauth_token_count=fact.oauth_token_count+EXCLUDED.oauth_token_count;
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION public.revoke_sessions_on_user_disable() FROM PUBLIC;
REVOKE ALL ON FUNCTION public.revoke_credentials_on_account_generation() FROM PUBLIC;

-- Authentication-only boolean projections. No business table, password hash,
-- token string, role or platform administrator flag can be returned.
ALTER FUNCTION public.bff_oidc_proof_active(uuid,uuid,uuid,bigint,uuid,bigint,text)
    SECURITY DEFINER;
ALTER FUNCTION public.bff_oidc_proof_active(uuid,uuid,uuid,bigint,uuid,bigint,text)
    SET search_path=pg_catalog,public;
REVOKE ALL ON FUNCTION public.bff_oidc_proof_active(uuid,uuid,uuid,bigint,uuid,bigint,text) FROM PUBLIC;
ALTER FUNCTION public.customer_portal_account_source_active(text,text,uuid,uuid,uuid,uuid,bigint,bigint,bigint,timestamptz)
    SECURITY DEFINER;
ALTER FUNCTION public.customer_portal_account_source_active(text,text,uuid,uuid,uuid,uuid,bigint,bigint,bigint,timestamptz)
    SET search_path=pg_catalog,public;
REVOKE ALL ON FUNCTION public.customer_portal_account_source_active(text,text,uuid,uuid,uuid,uuid,bigint,bigint,bigint,timestamptz) FROM PUBLIC;

CREATE FUNCTION public.oidc_console_source_active(expected_issuer text,client_ref text,policy_version bigint,member_type text)
RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER
SET search_path=pg_catalog,public AS $$
    SELECT EXISTS(SELECT 1 FROM public.oidc_clients c
        JOIN public.oidc_provider_settings p ON p.singleton AND p.issuer=expected_issuer
        WHERE c.id::text=client_ref AND c.status='ENABLED'
          AND c.access_policy_version=policy_version AND c.purpose='PLATFORM_CONSOLE'
          AND member_type='EMPLOYEE' AND c.allowed_member_types=ARRAY['EMPLOYEE']::text[])
$$;
REVOKE ALL ON FUNCTION public.oidc_console_source_active(text,text,bigint,text) FROM PUBLIC;

CREATE FUNCTION public.identity_lock_oidc_source(p_client_id text,p_issuer text)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER
SET search_path=pg_catalog,public AS $$
BEGIN
    PERFORM 1 FROM public.oidc_clients c WHERE c.id::text=p_client_id
      AND public.is_uuid_v7(c.id)
      AND c.purpose='PLATFORM_CONSOLE' AND c.allowed_member_types=ARRAY['EMPLOYEE']::text[]
      AND EXISTS(SELECT 1 FROM public.oidc_provider_settings p WHERE p.singleton AND p.issuer=p_issuer)
    FOR SHARE OF c;
    RETURN FOUND;
END $$;
REVOKE ALL ON FUNCTION public.identity_lock_oidc_source(text,text) FROM PUBLIC;

-- Platform wrapper retains its own legacy policy and never gives its OAuth
-- table privileges to the authentication runtime or projection owner.
CREATE OR REPLACE FUNCTION public.platform_source_active(expected_issuer text,client_ref text,policy_version bigint,member_type text)
RETURNS boolean LANGUAGE sql STABLE SECURITY INVOKER
SET search_path=pg_catalog,public AS $$
    SELECT CASE WHEN expected_issuer='urn:saas:legacy-platform' THEN EXISTS(
        SELECT 1 FROM public.oauth_clients c WHERE c.client_id=client_ref
          AND c.status='ENABLED' AND c.subject_mode IN ('USER','LEGACY_MIXED')
          AND c.access_policy_version=policy_version AND member_type=ANY(c.allowed_member_types))
    ELSE public.oidc_console_source_active(expected_issuer,client_ref,policy_version,member_type) END
$$;

REVOKE ALL ON public.identity_deployment_state,public.identity_service_clients,public.identity_audit_logs FROM PUBLIC;
