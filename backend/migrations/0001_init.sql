-- Fresh installation baseline. Existing installations must be recreated; no historical adoption.
-- PostgreSQL 17+, application-generated UUIDv7, SSO-only console.
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;

CREATE EXTENSION IF NOT EXISTS pg_trgm WITH SCHEMA public;

CREATE FUNCTION public.application_effective_entry(app_id uuid, env text) RETURNS TABLE(runtime_mode text, entry_type text, entry_url text, version text, id uuid, source text, proxy_enabled boolean)
    LANGUAGE sql STABLE
    AS $$
    WITH effective AS (
      SELECT a.code,a.base_path,a.runtime_mode::TEXT,
        CASE WHEN u.application_id IS NOT NULL THEN 'HTML' ELSE COALESCE(r.entry_type,a.entry_type)::TEXT END AS entry_type,
        CASE WHEN rtrim(COALESCE(a.base_path,''),'/') <> '/micro/' || a.code THEN ''
        WHEN u.application_id IS NOT NULL AND u.proxy_enabled THEN
          '/micro-proxy/' || a.code || '/' || u.revision::TEXT || '/' ||
          CASE WHEN strpos(u.entry_url,'?') > 0 THEN '?' || substring(u.entry_url from strpos(u.entry_url,'?') + 1) ELSE '' END
        WHEN u.application_id IS NOT NULL THEN u.entry_url ELSE COALESCE(r.entry_url,a.entry_url,'')::TEXT END AS entry_url,
        CASE WHEN u.application_id IS NULL THEN r.version::TEXT END AS version,
        CASE WHEN u.application_id IS NOT NULL THEN u.revision ELSE r.id END AS id,
        CASE WHEN u.application_id IS NOT NULL THEN 'URL' WHEN r.id IS NOT NULL THEN 'RELEASE' ELSE 'LEGACY' END AS source,
        COALESCE(u.proxy_enabled,FALSE) AS proxy_enabled
      FROM applications a
        LEFT JOIN application_url_entries u ON u.application_id=a.id AND u.environment=env
          AND a.entry_type='HTML' AND a.runtime_mode='TRUSTED'
          AND rtrim(COALESCE(a.base_path,''),'/')='/micro/' || a.code
        LEFT JOIN application_deployments d ON d.application_id=a.id AND d.environment=env
          AND rtrim(COALESCE(a.base_path,''),'/')='/micro/' || a.code
        LEFT JOIN application_releases r ON r.application_id=a.id AND r.environment=d.environment AND r.id=d.active_release_id
      WHERE a.id=app_id
    )
    SELECT e.runtime_mode,e.entry_type,
      CASE WHEN rtrim(COALESCE(e.base_path,''),'/')='/micro/' || e.code AND (e.runtime_mode<>'TRUSTED' OR e.entry_type<>'HTML') THEN '' ELSE e.entry_url END,
      e.version,e.id,e.source,e.proxy_enabled
    FROM effective e
$$;

CREATE FUNCTION public.bff_oidc_proof_active(subject_ref uuid, sid_ref uuid, grant_ref uuid, account_generation bigint, client_ref uuid, policy_generation bigint, expected_issuer text) RETURNS boolean
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'public'
    AS $$
    SELECT EXISTS(SELECT 1 FROM public.oidc_grants g
        JOIN public.oidc_clients c ON c.id=g.client_id
        JOIN public.sso_sessions s ON s.id=g.session_id AND s.user_id=g.user_id
        JOIN public.users u ON u.id=g.user_id
        JOIN public.oidc_provider_settings p ON p.singleton AND p.issuer=expected_issuer
        WHERE g.id=grant_ref AND g.user_id=subject_ref AND g.session_id=sid_ref
          AND g.client_id=client_ref AND g.account_version=account_generation
          AND g.client_policy_version=policy_generation AND g.revoked_at IS NULL
          AND g.expires_at>clock_timestamp() AND s.revoked_at IS NULL
          AND s.expires_at>clock_timestamp() AND s.account_version=account_generation
          AND u.status='ENABLED' AND u.session_version=account_generation
          AND c.status='ENABLED' AND c.access_policy_version=policy_generation
          AND c.purpose='PLATFORM_CONSOLE' AND c.allowed_member_types=ARRAY['EMPLOYEE']::text[])
$$;

CREATE FUNCTION public.bff_session_active(session_ref uuid) RETURNS boolean
    LANGUAGE sql STABLE
    SET search_path TO 'pg_catalog', 'public'
    AS $$
    SELECT EXISTS(SELECT 1 FROM public.bff_sessions b
        JOIN public.console_browsers browser ON browser.id=b.browser_id
          AND browser.current_session_id=b.id AND browser.expires_at>clock_timestamp()
        WHERE b.id=session_ref AND public.is_uuid_v7(session_ref)
          AND b.revoked_at IS NULL AND b.expires_at>clock_timestamp()
          AND NOT b.refresh_uncertain AND b.logout_request_id IS NULL
          AND public.bff_oidc_proof_active(b.user_id,b.global_session_id,b.oidc_grant_id,
            b.account_version,b.oidc_client_id,b.oidc_client_policy_version,b.issuer))
$$;

CREATE FUNCTION public.bump_session_version_on_user_disable() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF OLD.status = 'ENABLED' AND NEW.status <> 'ENABLED' THEN
        NEW.session_version := OLD.session_version + 1;
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION public.customer_portal_account_source_active(access_digest text, expected_issuer text, client_ref uuid, grant_ref uuid, sid_ref uuid, subject_ref uuid, account_generation bigint, policy_generation bigint, authentication_time bigint, source_deadline timestamp with time zone) RETURNS boolean
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'public'
    AS $$
    SELECT EXISTS(SELECT 1 FROM public.oidc_access_tokens tok
        JOIN public.oidc_grants g ON g.id=tok.grant_id
        JOIN public.oidc_clients c ON c.id=g.client_id
        JOIN public.sso_sessions s ON s.id=g.session_id AND s.user_id=g.user_id
        JOIN public.users u ON u.id=g.user_id
        JOIN public.oidc_provider_settings p ON p.singleton AND p.issuer=expected_issuer
        WHERE tok.token_hash=access_digest AND tok.revoked_at IS NULL
          AND tok.expires_at>clock_timestamp() AND g.id=grant_ref AND g.client_id=client_ref
          AND g.session_id=sid_ref AND g.user_id=subject_ref
          AND g.account_version=account_generation AND g.client_policy_version=policy_generation
          AND floor(extract(epoch FROM g.auth_time))::bigint=authentication_time
          AND g.revoked_at IS NULL AND g.expires_at>clock_timestamp()
          AND 'openid'=ANY(g.scope) AND 'profile'=ANY(g.scope)
          AND s.revoked_at IS NULL AND s.expires_at>clock_timestamp()
          AND s.account_version=account_generation AND u.status='ENABLED'
          AND u.session_version=account_generation AND c.status='ENABLED'
          AND c.access_policy_version=policy_generation AND c.purpose='CUSTOMER_PORTAL'
          AND c.allowed_member_types=ARRAY['CUSTOMER']::text[]
          AND LEAST(tok.expires_at,g.expires_at,s.expires_at)=source_deadline
          AND source_deadline>clock_timestamp())
$$;

CREATE FUNCTION public.identity_binding_active(subject_id uuid, tenant_ref uuid, organization_ref uuid, account_generation bigint, membership_ref uuid, relationship_type text, identity_generation bigint) RETURNS boolean
    LANGUAGE sql STABLE
    SET search_path TO 'pg_catalog', 'public'
    AS $$
    SELECT EXISTS (
        SELECT 1 FROM public.users u
        JOIN public.memberships m ON m.user_id=u.id AND m.id=membership_ref
        JOIN public.organizations o ON o.id=m.organization_id
        JOIN public.tenants t ON t.id=o.tenant_id
        WHERE u.id=subject_id AND u.status='ENABLED'
          AND public.is_uuid_v7(subject_id) AND public.is_uuid_v7(tenant_ref)
          AND public.is_uuid_v7(organization_ref) AND public.is_uuid_v7(membership_ref)
          AND u.session_version=account_generation AND account_generation >= 0
          AND m.organization_id=organization_ref AND o.tenant_id=tenant_ref
          AND m.member_type=relationship_type AND m.status='ENABLED'
          AND m.identity_version=identity_generation AND identity_generation >= 1
          AND t.status='ENABLED'
          AND (t.expires_at IS NULL OR t.expires_at>clock_timestamp())
          AND public.org_chain_active(o.id)
          AND (public.org_chain_expires_at(o.id) IS NULL
               OR public.org_chain_expires_at(o.id)>clock_timestamp()))
$$;

CREATE FUNCTION public.identity_lock_oidc_source(p_client_id text, p_issuer text) RETURNS boolean
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN
    PERFORM 1 FROM public.oidc_clients c WHERE c.id::text=p_client_id
      AND public.is_uuid_v7(c.id)
      AND c.purpose='PLATFORM_CONSOLE' AND c.allowed_member_types=ARRAY['EMPLOYEE']::text[]
      AND EXISTS(SELECT 1 FROM public.oidc_provider_settings p WHERE p.singleton AND p.issuer=p_issuer)
    FOR SHARE OF c;
    RETURN FOUND;
END $$;

CREATE FUNCTION public.identity_oidc_command_intent_active(operation_ref uuid, service_ref uuid, digest_ref text, method_ref text, path_ref text) RETURNS boolean
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'public'
    AS $$
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

CREATE FUNCTION public.is_uuid_v7(value uuid) RETURNS boolean
    LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE
    SET search_path TO 'pg_catalog'
    AS $$ SELECT (get_byte(uuid_send(value), 6) >> 4) = 7
    AND (get_byte(uuid_send(value), 8) & 192) = 128 $$;

CREATE FUNCTION public.oauth_client_grant_set(grants text[]) RETURNS text[]
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path TO 'pg_catalog'
    AS $$
    -- Match Rust str::trim (Unicode White_Space), including tabs/newlines.
    -- A different trim contract could disguise a runtime machine grant as USER.
    SELECT COALESCE(array_agg(DISTINCT grant_name ORDER BY grant_name)
        FILTER (WHERE grant_name <> ''), ARRAY[]::text[])
    FROM (
        SELECT btrim(raw_grant,
            U&'\0009\000A\000B\000C\000D\0020\0085\00A0\1680\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A\2028\2029\202F\205F\3000') AS grant_name
        FROM unnest(grants) AS parsed(raw_grant)
    ) AS normalized
$$;

CREATE FUNCTION public.oidc_console_source_active(expected_issuer text, client_ref text, policy_version bigint, member_type text) RETURNS boolean
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'public'
    AS $$
    SELECT EXISTS(SELECT 1 FROM public.oidc_clients c
        JOIN public.oidc_provider_settings p ON p.singleton AND p.issuer=expected_issuer
        WHERE c.id::text=client_ref AND c.status='ENABLED'
          AND c.access_policy_version=policy_version AND c.purpose='PLATFORM_CONSOLE'
          AND member_type='EMPLOYEE' AND c.allowed_member_types=ARRAY['EMPLOYEE']::text[])
$$;

CREATE FUNCTION public.oidc_key_material_immutable() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN
    IF (NEW.kid,NEW.n,NEW.e) IS DISTINCT FROM (OLD.kid,OLD.n,OLD.e) THEN
        RAISE EXCEPTION 'OIDC kid and public material are immutable' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;

CREATE FUNCTION public.org_chain_active(target_org uuid) RETURNS boolean
    LANGUAGE sql STABLE
    AS $$
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
$$;

CREATE FUNCTION public.org_chain_expires_at(target_org uuid) RETURNS timestamp with time zone
    LANGUAGE sql STABLE
    AS $$
    WITH RECURSIVE chain AS (
        SELECT id, parent_id, expires_at
        FROM organizations WHERE id = target_org
        UNION ALL
        SELECT parent.id, parent.parent_id, parent.expires_at
        FROM chain child JOIN organizations parent ON parent.id = child.parent_id
    )
    SELECT min(expires_at) FROM chain;
$$;

CREATE FUNCTION public.org_path(org uuid) RETURNS text
    LANGUAGE sql IMMUTABLE
    AS $$
  WITH RECURSIVE up AS (
    SELECT id, name, parent_id, 1 AS d FROM organizations WHERE id = org
    UNION ALL
    SELECT o.id, o.name, o.parent_id, up.d + 1 FROM organizations o JOIN up ON o.id = up.parent_id
  )
  SELECT COALESCE(string_agg(name, ' / ' ORDER BY d DESC), '') FROM up;
$$;

CREATE FUNCTION public.platform_account_command_intent_active(operation_ref uuid, target_ref uuid, account_generation bigint, digest_ref text, kind_ref text) RETURNS boolean
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'public'
    AS $$
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

CREATE FUNCTION public.platform_password_context_active(subject_ref uuid, session_ref uuid, tenant_ref uuid, organization_ref uuid, account_generation bigint) RETURNS boolean
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'public'
    AS $$
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

CREATE FUNCTION public.platform_session_active(session_ref uuid) RETURNS boolean
    LANGUAGE sql STABLE
    SET search_path TO 'pg_catalog', 'public'
    AS $$
    SELECT EXISTS (
        SELECT 1 FROM public.platform_sessions s
        JOIN public.bff_sessions b ON b.id=s.bff_session_id
        WHERE s.id=session_ref AND public.is_uuid_v7(session_ref)
          AND s.revoked_at IS NULL AND s.expires_at>clock_timestamp()
          AND public.platform_source_active(s.origin_issuer,s.origin_client_ref,
              s.origin_client_policy_version,s.member_type)
          AND public.bff_session_active(b.id) AND b.user_id=s.user_id
          AND b.account_version=s.account_version AND b.selected_session_id=s.id
          AND b.version=s.bff_context_version AND b.issuer=s.origin_issuer
          AND b.oidc_client_id::text=s.origin_client_ref
          AND b.oidc_client_policy_version=s.origin_client_policy_version
          AND public.identity_binding_active(s.user_id,s.tenant_id,s.organization_id,
              s.account_version,s.membership_id,s.member_type,s.identity_version)
    )
$$;

CREATE FUNCTION public.platform_source_active(expected_issuer text, client_ref text, policy_version bigint, member_type text) RETURNS boolean
    LANGUAGE sql STABLE
    SET search_path TO 'pg_catalog', 'public'
    AS $$
    -- Keep the narrow identity-owned proof port; the platform runtime must not
    -- gain direct SELECT on authentication-owned clients or provider settings.
    SELECT public.oidc_console_source_active(expected_issuer,client_ref,policy_version,member_type)
$$;

CREATE FUNCTION public.protect_application_identity_policy() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN
    IF NEW.access_policy_version < OLD.access_policy_version THEN
        RAISE EXCEPTION 'application access policy version cannot decrease'
            USING ERRCODE = '23514', CONSTRAINT = 'applications_access_policy_version_monotonic';
    END IF;
    IF (NEW.allowed_member_types, NEW.status) IS DISTINCT FROM
       (OLD.allowed_member_types, OLD.status) THEN
        NEW.access_policy_version := GREATEST(NEW.access_policy_version, OLD.access_policy_version + 1);
        NEW.updated_at := now();
    END IF;
    RETURN NEW;
END $$;

CREATE FUNCTION public.protect_application_release_artifact() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF NEW IS DISTINCT FROM OLD THEN
        RAISE EXCEPTION 'application release artifact is immutable' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION public.protect_application_release_definition() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF ROW(OLD.id, OLD.application_id, OLD.environment, OLD.version, OLD.entry_type,
           OLD.entry_url, OLD.health_url, OLD.resources, OLD.created_at, OLD.created_by)
       IS DISTINCT FROM
       ROW(NEW.id, NEW.application_id, NEW.environment, NEW.version, NEW.entry_type,
           NEW.entry_url, NEW.health_url, NEW.resources, NEW.created_at, NEW.created_by)
       OR (OLD.artifact_digest IS NOT NULL AND NEW.artifact_digest IS DISTINCT FROM OLD.artifact_digest)
    THEN
        RAISE EXCEPTION 'application release definition is immutable' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION public.protect_legacy_console_origin() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN
    IF OLD.client_id = 'platform-console-legacy' THEN
        IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION 'legacy console origin cannot be removed'
                USING ERRCODE='23514';
        END IF;
        IF NEW.client_id <> OLD.client_id OR NEW.subject_mode <> 'USER'
           OR NEW.allowed_member_types <> ARRAY['EMPLOYEE']::text[]
           OR NEW.application_id IS NOT NULL OR NEW.client_type <> 'PUBLIC'
           OR NEW.grant_types <> 'authorization_code' OR NOT NEW.require_pkce
           OR COALESCE(NEW.redirect_uris,'') <> '' THEN
            RAISE EXCEPTION 'legacy console origin must remain employee-only'
                USING ERRCODE='23514';
        END IF;
    END IF;
    IF TG_OP='DELETE' THEN RETURN OLD; END IF;
    RETURN NEW;
END $$;

CREATE FUNCTION public.protect_membership_identity() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN
    IF NEW.id IS DISTINCT FROM OLD.id
       OR NEW.user_id IS DISTINCT FROM OLD.user_id
       OR NEW.organization_id IS DISTINCT FROM OLD.organization_id
       OR NEW.member_type IS DISTINCT FROM OLD.member_type THEN
        RAISE EXCEPTION 'membership identity and ownership are immutable'
            USING ERRCODE = '23514', CONSTRAINT = 'memberships_identity_immutable';
    END IF;
    IF NEW.identity_version < OLD.identity_version
       OR (NEW.status IS DISTINCT FROM OLD.status
           AND NEW.identity_version <= OLD.identity_version) THEN
        RAISE EXCEPTION 'membership status changes require a newer identity version'
            USING ERRCODE = '23514', CONSTRAINT = 'memberships_identity_version_monotonic';
    END IF;
    IF NEW IS DISTINCT FROM OLD THEN
        NEW.updated_at := now();
    END IF;
    RETURN NEW;
END $$;

CREATE FUNCTION public.protect_notification_member_pref_identity() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN
    IF NEW.membership_id IS DISTINCT FROM OLD.membership_id
       OR NEW.identity_version IS DISTINCT FROM OLD.identity_version THEN
        RAISE EXCEPTION 'notification preference identity is immutable'
            USING ERRCODE='23514', CONSTRAINT='notification_member_prefs_identity_immutable';
    END IF;
    RETURN NEW;
END $$;

CREATE FUNCTION public.protect_notification_recipient() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN
    IF NEW.id IS DISTINCT FROM OLD.id OR NEW.user_id IS DISTINCT FROM OLD.user_id
       OR NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
       OR NEW.organization_id IS DISTINCT FROM OLD.organization_id
       OR NEW.application_id IS DISTINCT FROM OLD.application_id
       OR NEW.publication_id IS DISTINCT FROM OLD.publication_id
       OR NEW.source_kind IS DISTINCT FROM OLD.source_kind
       OR NEW.recipient_membership_id IS DISTINCT FROM OLD.recipient_membership_id
       OR NEW.recipient_member_type IS DISTINCT FROM OLD.recipient_member_type
       OR NEW.recipient_identity_version IS DISTINCT FROM OLD.recipient_identity_version THEN
        RAISE EXCEPTION 'notification ownership and recipient proof are immutable'
            USING ERRCODE='23514', CONSTRAINT='notifications_recipient_immutable';
    END IF;
    RETURN NEW;
END $$;

CREATE FUNCTION public.protect_oauth_client_identity_policy() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN
    IF NEW.subject_mode = 'LEGACY_MIXED' THEN
        IF TG_OP = 'INSERT' THEN
            RAISE EXCEPTION 'new legacy mixed clients are prohibited'
                USING ERRCODE = '23514', CONSTRAINT = 'oauth_clients_no_new_legacy_mixed';
        ELSIF OLD.subject_mode <> 'LEGACY_MIXED' THEN
            RAISE EXCEPTION 'entering legacy mixed mode is prohibited'
                USING ERRCODE = '23514', CONSTRAINT = 'oauth_clients_no_new_legacy_mixed';
        END IF;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF NEW.access_policy_version < OLD.access_policy_version THEN
            RAISE EXCEPTION 'client access policy version cannot decrease'
                USING ERRCODE = '23514', CONSTRAINT = 'oauth_clients_access_policy_version_monotonic';
        END IF;
        IF (NEW.allowed_member_types, NEW.subject_mode, NEW.status, NEW.grant_types,
            NEW.application_id, NEW.client_type, NEW.require_pkce, NEW.scopes,
            NEW.redirect_uris, NEW.access_token_validity, NEW.refresh_token_validity,
            NEW.client_secret_hash) IS DISTINCT FROM
           (OLD.allowed_member_types, OLD.subject_mode, OLD.status, OLD.grant_types,
            OLD.application_id, OLD.client_type, OLD.require_pkce, OLD.scopes,
            OLD.redirect_uris, OLD.access_token_validity, OLD.refresh_token_validity,
            OLD.client_secret_hash) THEN
            NEW.access_policy_version := GREATEST(NEW.access_policy_version, OLD.access_policy_version + 1);
            NEW.updated_at := now();
        END IF;
    END IF;
    RETURN NEW;
END $$;

CREATE FUNCTION public.reject_department_cycle() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
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
$$;

CREATE FUNCTION public.reject_notification_publication_update() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF NEW IS DISTINCT FROM OLD THEN
        RAISE EXCEPTION 'notification publications are immutable' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END
$$;

CREATE FUNCTION public.reject_organization_cycle() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
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
$$;

CREATE FUNCTION public.revoke_app_tokens_on_app_code_change() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF OLD.code IS DISTINCT FROM NEW.code THEN
        UPDATE oauth_tokens SET revoked = true
        WHERE token_use = 'app' AND application_id = NEW.id AND revoked = false;
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION public.revoke_app_tokens_on_application_disable() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF OLD.status = 'ENABLED' AND NEW.status <> 'ENABLED' THEN
        UPDATE oauth_tokens SET revoked = true
        WHERE token_use = 'app' AND application_id = NEW.id AND revoked = false;
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION public.revoke_app_tokens_on_client_config_change() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
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
$$;

CREATE FUNCTION public.revoke_app_tokens_on_client_disable() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF OLD.status = 'ENABLED' AND NEW.status <> 'ENABLED' THEN
        UPDATE oauth_tokens SET revoked = true
        WHERE token_use = 'app' AND client_id = NEW.client_id AND revoked = false;
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION public.revoke_app_tokens_on_org_grant_delete() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    UPDATE oauth_tokens SET revoked = true
    WHERE token_use = 'app' AND organization_id = OLD.organization_id
      AND application_id = OLD.application_id AND revoked = false;
    RETURN OLD;
END;
$$;

CREATE FUNCTION public.revoke_app_tokens_on_runtime_mode_change() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF OLD.runtime_mode IS DISTINCT FROM NEW.runtime_mode THEN
        UPDATE oauth_tokens SET revoked = true
        WHERE token_use = 'app' AND application_id = NEW.id AND revoked = false;
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION public.revoke_app_tokens_on_tenant_grant_delete() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    UPDATE oauth_tokens SET revoked = true
    WHERE token_use = 'app' AND tenant_id = OLD.tenant_id
      AND application_id = OLD.application_id AND revoked = false;
    RETURN OLD;
END;
$$;

CREATE FUNCTION public.revoke_application_policy_credentials(application_ref uuid) RETURNS void
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public'
    AS $$
DECLARE registration public.applications%ROWTYPE;
BEGIN
    SELECT * INTO registration FROM public.applications WHERE id=application_ref;
    IF NOT FOUND THEN RETURN; END IF;
    -- Application admission does not revoke an unrelated platform/SSO session.
    UPDATE public.oauth_tokens SET revoked=true
        WHERE application_id=application_ref AND NOT revoked
        AND (application_policy_version IS DISTINCT FROM registration.access_policy_version
             OR registration.status<>'ENABLED');
    UPDATE public.oauth_authorization_codes SET consumed=true
        WHERE application_id=application_ref AND NOT consumed
        AND (application_policy_version IS DISTINCT FROM registration.access_policy_version
             OR registration.status<>'ENABLED');
    UPDATE public.oauth_authorization_requests SET consumed=true
        WHERE application_id=application_ref AND NOT consumed
        AND (application_policy_version IS DISTINCT FROM registration.access_policy_version
             OR registration.status<>'ENABLED');
    UPDATE public.oauth_device_codes SET status='EXPIRED'
        WHERE application_id=application_ref AND status IN ('PENDING','APPROVED')
        AND (application_policy_version IS DISTINCT FROM registration.access_policy_version
             OR registration.status<>'ENABLED');
END $$;

CREATE FUNCTION public.revoke_changed_application_policy() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN
    IF NEW.access_policy_version IS DISTINCT FROM OLD.access_policy_version THEN
        PERFORM public.revoke_application_policy_credentials(NEW.id);
    END IF;
    RETURN NEW;
END $$;

CREATE FUNCTION public.revoke_changed_client_policy() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN
    IF NEW.access_policy_version IS DISTINCT FROM OLD.access_policy_version THEN
        PERFORM public.revoke_client_policy_credentials(NEW.id);
    END IF;
    RETURN NEW;
END $$;

CREATE FUNCTION public.revoke_client_policy_credentials(client_ref uuid) RETURNS void
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public'
    AS $$
DECLARE registration public.oauth_clients%ROWTYPE;
BEGIN
    SELECT * INTO registration FROM public.oauth_clients WHERE id=client_ref;
    IF NOT FOUND THEN RETURN; END IF;
    UPDATE public.platform_sessions SET revoked_at=clock_timestamp()
        WHERE origin_client_ref=registration.client_id AND revoked_at IS NULL
        AND (origin_client_policy_version IS DISTINCT FROM registration.access_policy_version
             OR registration.status<>'ENABLED'
             OR NOT (member_type=ANY(registration.allowed_member_types)));
    UPDATE public.platform_session_refresh_tokens r SET consumed_at=clock_timestamp()
        FROM public.platform_sessions s WHERE r.session_id=s.id
        AND s.origin_client_ref=registration.client_id AND s.revoked_at IS NOT NULL
        AND r.consumed_at IS NULL;
    UPDATE public.oauth_tokens SET revoked=true WHERE NOT revoked AND (
        (client_id=registration.client_id
         AND (client_policy_version IS DISTINCT FROM registration.access_policy_version
              OR registration.status<>'ENABLED'))
        OR (origin_client_ref=registration.client_id
            AND (origin_client_policy_version IS DISTINCT FROM registration.access_policy_version
                 OR registration.status<>'ENABLED')));
    UPDATE public.oauth_authorization_codes SET consumed=true
        WHERE client_id=registration.client_id AND NOT consumed
        AND (client_policy_version IS DISTINCT FROM registration.access_policy_version
             OR registration.status<>'ENABLED');
    UPDATE public.oauth_authorization_requests SET consumed=true
        WHERE client_id=registration.client_id AND NOT consumed
        AND (client_policy_version IS DISTINCT FROM registration.access_policy_version
             OR registration.status<>'ENABLED');
    UPDATE public.oauth_device_codes SET status='EXPIRED'
        WHERE client_id=registration.client_id AND status IN ('PENDING','APPROVED')
        AND (client_policy_version IS DISTINCT FROM registration.access_policy_version
             OR registration.status<>'ENABLED');
END $$;

CREATE FUNCTION public.revoke_credentials_on_account_generation() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'public'
    AS $$
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

CREATE FUNCTION public.revoke_credentials_on_identity_update() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN
    IF NEW.identity_version IS DISTINCT FROM OLD.identity_version
       OR NEW.status IS DISTINCT FROM OLD.status THEN
        PERFORM public.revoke_membership_credentials(OLD.id);
    END IF;
    RETURN NEW;
END $$;

CREATE FUNCTION public.revoke_membership_credentials(membership_ref uuid) RETURNS void
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN
    UPDATE public.platform_sessions SET revoked_at=clock_timestamp()
        WHERE membership_id=membership_ref AND revoked_at IS NULL;
    UPDATE public.platform_session_refresh_tokens r SET consumed_at=clock_timestamp()
        FROM public.platform_sessions s WHERE r.session_id=s.id
        AND s.membership_id=membership_ref AND r.consumed_at IS NULL;
    UPDATE public.oauth_tokens SET revoked=true
        WHERE membership_id=membership_ref AND NOT revoked;
    UPDATE public.oauth_authorization_codes SET consumed=true
        WHERE membership_id=membership_ref AND NOT consumed;
    UPDATE public.oauth_device_codes SET status='EXPIRED'
        WHERE membership_id=membership_ref AND status IN ('PENDING','APPROVED');
END $$;

CREATE FUNCTION public.revoke_oidc_on_client_policy() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN
    IF NEW.access_policy_version < OLD.access_policy_version THEN
        RAISE EXCEPTION 'OIDC client policy generation cannot decrease' USING ERRCODE='23514';
    END IF;
    IF (NEW.purpose,NEW.allowed_member_types,NEW.redirect_uris,
        NEW.post_logout_redirect_uris,NEW.refresh_enabled,NEW.status)
       IS DISTINCT FROM
       (OLD.purpose,OLD.allowed_member_types,OLD.redirect_uris,
        OLD.post_logout_redirect_uris,OLD.refresh_enabled,OLD.status)
       AND NEW.access_policy_version <= OLD.access_policy_version THEN
        RAISE EXCEPTION 'OIDC client policy requires a new generation' USING ERRCODE='23514';
    END IF;
    IF NEW.access_policy_version IS DISTINCT FROM OLD.access_policy_version
       OR NEW.status <> 'ENABLED' THEN
        UPDATE public.oidc_grants SET revoked_at=clock_timestamp()
        WHERE client_id=NEW.id AND revoked_at IS NULL;
    END IF;
    RETURN NEW;
END $$;

CREATE FUNCTION public.revoke_oidc_on_sso_revocation() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN
    IF NEW.revoked_at IS NOT NULL AND OLD.revoked_at IS NULL THEN
        UPDATE public.oidc_grants SET revoked_at=clock_timestamp()
        WHERE session_id=NEW.id AND revoked_at IS NULL;
    END IF;
    RETURN NEW;
END $$;

CREATE FUNCTION public.revoke_sessions_on_membership_delete() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN
    PERFORM public.revoke_membership_credentials(OLD.id);
    RETURN OLD;
END $$;

CREATE FUNCTION public.revoke_sessions_on_user_disable() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'public'
    AS $$
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

CREATE FUNCTION public.revoke_sso_on_account_generation() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN
    IF NEW.session_version IS DISTINCT FROM OLD.session_version
       OR (OLD.status='ENABLED' AND NEW.status<>'ENABLED') THEN
        UPDATE public.sso_sessions SET revoked_at=clock_timestamp()
        WHERE user_id=NEW.id AND revoked_at IS NULL;
    END IF;
    RETURN NEW;
END $$;

CREATE FUNCTION public.sync_sessions_on_organization_state() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
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
$$;

CREATE FUNCTION public.sync_sessions_on_tenant_state() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
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
$$;

CREATE FUNCTION public.valid_member_type_set(member_types text[], allow_empty boolean) RETURNS boolean
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path TO 'pg_catalog'
    AS $$
    SELECT COALESCE(member_types = ARRAY['EMPLOYEE']::text[]
        OR member_types = ARRAY['CUSTOMER']::text[]
        OR member_types = ARRAY['EMPLOYEE', 'CUSTOMER']::text[]
        OR (allow_empty AND member_types = ARRAY[]::text[]), false)
$$;

SET default_table_access_method = heap;

CREATE TABLE public.application_deployments (
    application_id uuid NOT NULL,
    environment character varying(32) NOT NULL,
    active_release_id uuid NOT NULL,
    previous_release_id uuid,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_by uuid,
    CONSTRAINT application_deployments_check CHECK (((previous_release_id IS NULL) OR (previous_release_id <> active_release_id))),
    CONSTRAINT uuid_v7_active_release_id CHECK (public.is_uuid_v7(active_release_id)),
    CONSTRAINT uuid_v7_application_id CHECK (public.is_uuid_v7(application_id)),
    CONSTRAINT uuid_v7_previous_release_id CHECK (public.is_uuid_v7(previous_release_id)),
    CONSTRAINT uuid_v7_updated_by CHECK (public.is_uuid_v7(updated_by))
);

CREATE TABLE public.application_release_artifacts (
    application_id uuid NOT NULL,
    version character varying(64) NOT NULL,
    artifact_digest text NOT NULL,
    manifest jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT application_release_artifacts_artifact_digest_check CHECK ((artifact_digest ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT application_release_artifacts_manifest_check CHECK ((jsonb_typeof(manifest) = 'object'::text)),
    CONSTRAINT application_release_artifacts_version_check CHECK ((((version)::text ~ '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$'::text) AND ((version)::text !~~ '%..%'::text))),
    CONSTRAINT uuid_v7_application_id CHECK (public.is_uuid_v7(application_id))
);

CREATE TABLE public.application_release_operations (
    id uuid NOT NULL,
    application_id uuid NOT NULL,
    kind text NOT NULL,
    started_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT application_release_operations_kind_check CHECK ((kind = ANY (ARRAY['REGISTER'::text, 'UPLOAD'::text, 'PREFLIGHT'::text, 'ACTIVATE'::text, 'ROLLBACK'::text]))),
    CONSTRAINT uuid_v7_application_id CHECK (public.is_uuid_v7(application_id)),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id))
);

CREATE TABLE public.application_releases (
    id uuid NOT NULL,
    application_id uuid NOT NULL,
    environment character varying(32) NOT NULL,
    version character varying(64) NOT NULL,
    entry_type character varying(10) NOT NULL,
    entry_url character varying(2000) NOT NULL,
    health_url character varying(2000),
    resources jsonb DEFAULT '[]'::jsonb NOT NULL,
    validation_status character varying(10) DEFAULT 'DRAFT'::character varying NOT NULL,
    validation_result jsonb DEFAULT '{}'::jsonb NOT NULL,
    checked_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    created_by uuid,
    artifact_digest text,
    CONSTRAINT application_releases_check CHECK (((((validation_status)::text = 'DRAFT'::text) AND (checked_at IS NULL)) OR (((validation_status)::text = ANY (ARRAY[('PASSED'::character varying)::text, ('FAILED'::character varying)::text])) AND (checked_at IS NOT NULL)))),
    CONSTRAINT application_releases_environment_check CHECK (((environment)::text ~ '^[a-z0-9][a-z0-9_-]{0,31}$'::text)),
    CONSTRAINT application_releases_resources_check CHECK ((jsonb_typeof(resources) = 'array'::text)),
    CONSTRAINT application_releases_validation_status_check CHECK (((validation_status)::text = ANY (ARRAY[('DRAFT'::character varying)::text, ('PASSED'::character varying)::text, ('FAILED'::character varying)::text]))),
    CONSTRAINT application_releases_version_check CHECK ((((version)::text ~ '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$'::text) AND ((version)::text !~~ '%..%'::text))),
    CONSTRAINT uuid_v7_application_id CHECK (public.is_uuid_v7(application_id)),
    CONSTRAINT uuid_v7_created_by CHECK (public.is_uuid_v7(created_by)),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id))
);

CREATE TABLE public.application_url_entries (
    application_id uuid NOT NULL,
    environment character varying(32) NOT NULL,
    entry_url text NOT NULL,
    proxy_enabled boolean DEFAULT true NOT NULL,
    revision uuid NOT NULL,
    updated_by uuid,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT application_url_entries_entry_url_check CHECK (((length(entry_url) >= 1) AND (length(entry_url) <= 2000))),
    CONSTRAINT application_url_entries_environment_check CHECK (((environment)::text ~ '^[a-z0-9][a-z0-9_-]{0,31}$'::text)),
    CONSTRAINT uuid_v7_application_id CHECK (public.is_uuid_v7(application_id)),
    CONSTRAINT uuid_v7_revision CHECK (public.is_uuid_v7(revision)),
    CONSTRAINT uuid_v7_updated_by CHECK (public.is_uuid_v7(updated_by))
);

CREATE TABLE public.applications (
    id uuid NOT NULL,
    code character varying(50) NOT NULL,
    name character varying(100) NOT NULL,
    icon character varying(100) DEFAULT 'LayoutGrid'::character varying,
    base_path character varying(200) DEFAULT ''::character varying,
    description text DEFAULT ''::text,
    sort_order integer DEFAULT 0 NOT NULL,
    status character varying(20) DEFAULT 'ENABLED'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    entry_type character varying(10) DEFAULT 'HTML'::character varying NOT NULL,
    entry_url character varying(500) DEFAULT ''::character varying NOT NULL,
    runtime_mode character varying(10) DEFAULT 'TRUSTED'::character varying NOT NULL,
    allowed_member_types text[] DEFAULT ARRAY['EMPLOYEE'::text] NOT NULL,
    access_policy_version bigint DEFAULT 1 NOT NULL,
    CONSTRAINT applications_access_policy_version_check CHECK ((access_policy_version >= 1)),
    CONSTRAINT applications_allowed_member_types_check CHECK (public.valid_member_type_set(allowed_member_types, false)),
    CONSTRAINT applications_runtime_mode_check CHECK ((((status)::text <> 'ENABLED'::text) OR (((entry_type)::text = 'HTML'::text) AND ((runtime_mode)::text = 'TRUSTED'::text)))),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id))
);

CREATE TABLE public.audit_logs (
    id bigint NOT NULL,
    user_id uuid,
    username character varying(100) DEFAULT ''::character varying,
    tenant_id uuid,
    tenant_name character varying(100) DEFAULT ''::character varying,
    organization_id uuid,
    organization_name character varying(100) DEFAULT ''::character varying,
    action character varying(50) NOT NULL,
    detail text DEFAULT ''::text,
    ip character varying(50) DEFAULT ''::character varying,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    user_agent character varying(500) DEFAULT ''::character varying NOT NULL,
    request_id uuid,
    application_id uuid,
    CONSTRAINT uuid_v7_application_id CHECK (public.is_uuid_v7(application_id)),
    CONSTRAINT uuid_v7_organization_id CHECK (public.is_uuid_v7(organization_id)),
    CONSTRAINT uuid_v7_request_id CHECK (public.is_uuid_v7(request_id)),
    CONSTRAINT uuid_v7_tenant_id CHECK (public.is_uuid_v7(tenant_id)),
    CONSTRAINT uuid_v7_user_id CHECK (public.is_uuid_v7(user_id))
);

CREATE SEQUENCE public.audit_logs_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.audit_logs_id_seq OWNED BY public.audit_logs.id;

CREATE TABLE public.bff_sessions (
    id uuid NOT NULL,
    browser_id uuid NOT NULL,
    token_hash text NOT NULL,
    csrf_hash text NOT NULL,
    user_id uuid NOT NULL,
    account_version bigint NOT NULL,
    global_session_id uuid NOT NULL,
    oidc_client_id uuid NOT NULL,
    oidc_grant_id uuid NOT NULL,
    oidc_client_policy_version bigint NOT NULL,
    issuer text NOT NULL,
    auth_time bigint NOT NULL,
    nonce_hash text NOT NULL,
    access_ciphertext text NOT NULL,
    refresh_ciphertext text NOT NULL,
    access_expires_at timestamp with time zone NOT NULL,
    version bigint NOT NULL,
    selected_session_id uuid,
    platform_refresh_ciphertext text,
    refresh_lease_id uuid,
    refresh_started_at timestamp with time zone,
    refresh_uncertain boolean DEFAULT false NOT NULL,
    logout_request_id uuid,
    logout_scope text,
    created_at timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    revoked_at timestamp with time zone,
    CONSTRAINT bff_sessions_account_version_check CHECK ((account_version >= 0)),
    CONSTRAINT bff_sessions_auth_time_check CHECK ((auth_time > 0)),
    CONSTRAINT bff_sessions_browser_id_check CHECK (public.is_uuid_v7(browser_id)),
    CONSTRAINT bff_sessions_check CHECK ((expires_at > created_at)),
    CONSTRAINT bff_sessions_check1 CHECK (((logout_request_id IS NULL) = (logout_scope IS NULL))),
    CONSTRAINT bff_sessions_check2 CHECK (((refresh_lease_id IS NULL) = (refresh_started_at IS NULL))),
    CONSTRAINT bff_sessions_csrf_hash_check CHECK ((csrf_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT bff_sessions_global_session_id_check CHECK (public.is_uuid_v7(global_session_id)),
    CONSTRAINT bff_sessions_id_check CHECK (public.is_uuid_v7(id)),
    CONSTRAINT bff_sessions_logout_request_id_check CHECK (((logout_request_id IS NULL) OR public.is_uuid_v7(logout_request_id))),
    CONSTRAINT bff_sessions_logout_scope_check CHECK ((logout_scope = ANY (ARRAY['GLOBAL'::text, 'PLATFORM'::text]))),
    CONSTRAINT bff_sessions_nonce_hash_check CHECK ((nonce_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT bff_sessions_oidc_client_id_check CHECK (public.is_uuid_v7(oidc_client_id)),
    CONSTRAINT bff_sessions_oidc_client_policy_version_check CHECK ((oidc_client_policy_version >= 1)),
    CONSTRAINT bff_sessions_oidc_grant_id_check CHECK (public.is_uuid_v7(oidc_grant_id)),
    CONSTRAINT bff_sessions_refresh_lease_id_check CHECK (((refresh_lease_id IS NULL) OR public.is_uuid_v7(refresh_lease_id))),
    CONSTRAINT bff_sessions_selected_session_id_check CHECK (((selected_session_id IS NULL) OR public.is_uuid_v7(selected_session_id))),
    CONSTRAINT bff_sessions_token_hash_check CHECK ((token_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT bff_sessions_user_id_check CHECK (public.is_uuid_v7(user_id)),
    CONSTRAINT bff_sessions_version_check CHECK ((version >= 1))
);

CREATE TABLE public.console_browsers (
    id uuid NOT NULL,
    token_hash text NOT NULL,
    generation bigint NOT NULL,
    current_session_id uuid,
    created_at timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    CONSTRAINT console_browsers_current_session_id_check CHECK (((current_session_id IS NULL) OR public.is_uuid_v7(current_session_id))),
    CONSTRAINT console_browsers_generation_check CHECK ((generation >= 0)),
    CONSTRAINT console_browsers_id_check CHECK (public.is_uuid_v7(id)),
    CONSTRAINT console_browsers_token_hash_check CHECK ((token_hash ~ '^[0-9a-f]{64}$'::text))
);

CREATE TABLE public.console_login_flows (
    id uuid NOT NULL,
    browser_id uuid NOT NULL,
    browser_generation bigint NOT NULL,
    state_hash text NOT NULL,
    nonce_hash text NOT NULL,
    verifier_ciphertext text NOT NULL,
    return_path text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    consumed_at timestamp with time zone,
    CONSTRAINT console_login_flows_browser_generation_check CHECK ((browser_generation >= 0)),
    CONSTRAINT console_login_flows_browser_id_check CHECK (public.is_uuid_v7(browser_id)),
    CONSTRAINT console_login_flows_check CHECK ((expires_at > created_at)),
    CONSTRAINT console_login_flows_id_check CHECK (public.is_uuid_v7(id)),
    CONSTRAINT console_login_flows_nonce_hash_check CHECK ((nonce_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT console_login_flows_state_hash_check CHECK ((state_hash ~ '^[0-9a-f]{64}$'::text))
);

CREATE TABLE public.customer_portal_contexts (
    id uuid NOT NULL,
    token_hash text NOT NULL,
    issuer text NOT NULL,
    oidc_client_id uuid NOT NULL,
    oidc_grant_id uuid NOT NULL,
    global_session_id uuid NOT NULL,
    user_id uuid NOT NULL,
    account_version bigint NOT NULL,
    oidc_client_policy_version bigint NOT NULL,
    auth_time bigint NOT NULL,
    source_expires_at timestamp with time zone NOT NULL,
    profile_client_ref uuid NOT NULL,
    profile_client_policy_version bigint NOT NULL,
    application_id uuid NOT NULL,
    application_policy_version bigint NOT NULL,
    tenant_id uuid NOT NULL,
    organization_id uuid NOT NULL,
    membership_id uuid NOT NULL,
    member_type text NOT NULL,
    identity_version bigint NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    revoked_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT customer_portal_contexts_account_version_check CHECK ((account_version >= 0)),
    CONSTRAINT customer_portal_contexts_application_id_check CHECK (public.is_uuid_v7(application_id)),
    CONSTRAINT customer_portal_contexts_application_policy_version_check CHECK ((application_policy_version >= 1)),
    CONSTRAINT customer_portal_contexts_auth_time_check CHECK ((auth_time > 0)),
    CONSTRAINT customer_portal_contexts_check CHECK ((expires_at <= source_expires_at)),
    CONSTRAINT customer_portal_contexts_global_session_id_check CHECK (public.is_uuid_v7(global_session_id)),
    CONSTRAINT customer_portal_contexts_id_check CHECK (public.is_uuid_v7(id)),
    CONSTRAINT customer_portal_contexts_identity_version_check CHECK ((identity_version >= 1)),
    CONSTRAINT customer_portal_contexts_member_type_check CHECK ((member_type = 'CUSTOMER'::text)),
    CONSTRAINT customer_portal_contexts_membership_id_check CHECK (public.is_uuid_v7(membership_id)),
    CONSTRAINT customer_portal_contexts_oidc_client_id_check CHECK (public.is_uuid_v7(oidc_client_id)),
    CONSTRAINT customer_portal_contexts_oidc_client_policy_version_check CHECK ((oidc_client_policy_version >= 1)),
    CONSTRAINT customer_portal_contexts_oidc_grant_id_check CHECK (public.is_uuid_v7(oidc_grant_id)),
    CONSTRAINT customer_portal_contexts_organization_id_check CHECK (public.is_uuid_v7(organization_id)),
    CONSTRAINT customer_portal_contexts_profile_client_policy_version_check CHECK ((profile_client_policy_version >= 1)),
    CONSTRAINT customer_portal_contexts_profile_client_ref_check CHECK (public.is_uuid_v7(profile_client_ref)),
    CONSTRAINT customer_portal_contexts_tenant_id_check CHECK (public.is_uuid_v7(tenant_id)),
    CONSTRAINT customer_portal_contexts_token_hash_check CHECK ((token_hash ~ '^[a-f0-9]{64}$'::text)),
    CONSTRAINT customer_portal_contexts_user_id_check CHECK (public.is_uuid_v7(user_id))
);

CREATE TABLE public.departments (
    id uuid NOT NULL,
    organization_id uuid NOT NULL,
    parent_id uuid,
    name character varying(100) NOT NULL,
    code character varying(50) DEFAULT ''::character varying,
    sort_order integer DEFAULT 0 NOT NULL,
    description text DEFAULT ''::text,
    status character varying(20) DEFAULT 'ENABLED'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id)),
    CONSTRAINT uuid_v7_organization_id CHECK (public.is_uuid_v7(organization_id)),
    CONSTRAINT uuid_v7_parent_id CHECK (public.is_uuid_v7(parent_id))
);

CREATE TABLE public.identity_account_command_receipts (
    operation_id uuid NOT NULL,
    service_id uuid NOT NULL,
    target_user_id uuid NOT NULL,
    canonical_digest text NOT NULL,
    state text NOT NULL,
    before_version bigint NOT NULL,
    after_version bigint NOT NULL,
    revoked_sessions bigint NOT NULL,
    revoked_tokens bigint NOT NULL,
    created_at timestamp with time zone NOT NULL,
    CONSTRAINT identity_account_command_receipts_after_version_check CHECK ((after_version >= 0)),
    CONSTRAINT identity_account_command_receipts_before_version_check CHECK ((before_version >= 0)),
    CONSTRAINT identity_account_command_receipts_canonical_digest_check CHECK ((canonical_digest ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT identity_account_command_receipts_operation_id_check CHECK (public.is_uuid_v7(operation_id)),
    CONSTRAINT identity_account_command_receipts_revoked_sessions_check CHECK ((revoked_sessions >= 0)),
    CONSTRAINT identity_account_command_receipts_revoked_tokens_check CHECK ((revoked_tokens >= 0)),
    CONSTRAINT identity_account_command_receipts_service_id_check CHECK (public.is_uuid_v7(service_id)),
    CONSTRAINT identity_account_command_receipts_state_check CHECK ((state = 'COMPLETED'::text)),
    CONSTRAINT identity_account_command_receipts_target_user_id_check CHECK (public.is_uuid_v7(target_user_id))
);

CREATE TABLE public.identity_account_revocation_facts (
    user_id uuid NOT NULL,
    account_version bigint NOT NULL,
    platform_session_count bigint CONSTRAINT identity_account_revocation_fac_platform_session_count_not_null NOT NULL,
    oauth_token_count bigint NOT NULL,
    created_at timestamp with time zone NOT NULL,
    CONSTRAINT identity_account_revocation_facts_account_version_check CHECK ((account_version >= 0)),
    CONSTRAINT identity_account_revocation_facts_oauth_token_count_check CHECK ((oauth_token_count >= 0)),
    CONSTRAINT identity_account_revocation_facts_platform_session_count_check CHECK ((platform_session_count >= 0)),
    CONSTRAINT identity_account_revocation_facts_user_id_check CHECK (public.is_uuid_v7(user_id))
);

CREATE TABLE public.identity_account_tombstones (
    user_id uuid NOT NULL,
    operation_id uuid NOT NULL,
    retired_at timestamp with time zone NOT NULL,
    CONSTRAINT identity_account_tombstones_operation_id_check CHECK (public.is_uuid_v7(operation_id)),
    CONSTRAINT identity_account_tombstones_user_id_check CHECK (public.is_uuid_v7(user_id))
);

CREATE TABLE public.identity_audit_logs (
    id uuid NOT NULL,
    user_id uuid,
    username text NOT NULL,
    action text NOT NULL,
    detail text NOT NULL,
    ip text NOT NULL,
    user_agent text NOT NULL,
    created_at timestamp with time zone DEFAULT clock_timestamp() NOT NULL,
    CONSTRAINT identity_audit_logs_id_check CHECK (public.is_uuid_v7(id)),
    CONSTRAINT identity_audit_logs_user_id_check CHECK (((user_id IS NULL) OR public.is_uuid_v7(user_id)))
);

CREATE TABLE public.identity_deployment_state (
    singleton boolean NOT NULL,
    deployment_id uuid NOT NULL,
    issuer text NOT NULL,
    schema_version bigint NOT NULL,
    migration_owner name NOT NULL,
    identity_owner name NOT NULL,
    platform_owner name NOT NULL,
    revocation_bridge_owner name NOT NULL,
    projection_owner name NOT NULL,
    identity_runtime name NOT NULL,
    platform_runtime name NOT NULL,
    ready_at timestamp with time zone NOT NULL,
    CONSTRAINT identity_deployment_state_deployment_id_check CHECK (public.is_uuid_v7(deployment_id)),
    CONSTRAINT identity_deployment_state_schema_version_check CHECK ((schema_version = 1)),
    CONSTRAINT identity_deployment_state_singleton_check CHECK (singleton)
);

CREATE TABLE public.identity_oidc_admin_receipts (
    operation_id uuid NOT NULL,
    service_id uuid NOT NULL,
    request_digest text NOT NULL,
    service_secret_hash text NOT NULL,
    method text NOT NULL,
    path text NOT NULL,
    result_json jsonb NOT NULL,
    returns_secret boolean NOT NULL,
    created_at timestamp with time zone NOT NULL,
    CONSTRAINT identity_oidc_admin_receipts_method_check CHECK ((method = ANY (ARRAY['POST'::text, 'PATCH'::text, 'DELETE'::text]))),
    CONSTRAINT identity_oidc_admin_receipts_operation_id_check CHECK (public.is_uuid_v7(operation_id)),
    CONSTRAINT identity_oidc_admin_receipts_path_check CHECK ((path ~ '^/internal/identity/oidc/(clients|keys)(/[A-Za-z0-9_-]+){0,2}$'::text)),
    CONSTRAINT identity_oidc_admin_receipts_request_digest_check CHECK ((request_digest ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT identity_oidc_admin_receipts_result_json_check CHECK ((jsonb_typeof(result_json) = 'object'::text)),
    CONSTRAINT identity_oidc_admin_receipts_service_id_check CHECK (public.is_uuid_v7(service_id)),
    CONSTRAINT identity_oidc_admin_receipts_service_secret_hash_check CHECK ((service_secret_hash ~ '^[0-9a-f]{64}$'::text))
);

CREATE TABLE public.identity_service_clients (
    id uuid NOT NULL,
    secret_hash text NOT NULL,
    purpose text NOT NULL,
    status text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    CONSTRAINT identity_service_clients_id_check CHECK (public.is_uuid_v7(id)),
    CONSTRAINT identity_service_clients_purpose_check CHECK ((purpose = ANY (ARRAY['ACCOUNT_COMMAND'::text, 'CUSTOMER_ACCESS_PROOF'::text, 'OIDC_ADMIN'::text]))),
    CONSTRAINT identity_service_clients_secret_hash_check CHECK ((secret_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT identity_service_clients_status_check CHECK ((status = ANY (ARRAY['ENABLED'::text, 'DISABLED'::text])))
);

CREATE TABLE public.managed_deployment_bindings (
    release_id uuid NOT NULL,
    artifact_id uuid NOT NULL,
    CONSTRAINT uuid_v7_artifact_id CHECK (public.is_uuid_v7(artifact_id)),
    CONSTRAINT uuid_v7_release_id CHECK (public.is_uuid_v7(release_id))
);

CREATE TABLE public.managed_deployment_jobs (
    id uuid NOT NULL,
    artifact_id uuid NOT NULL,
    release_id uuid NOT NULL,
    request_id uuid NOT NULL,
    payload_sha256 text NOT NULL,
    operation text NOT NULL,
    state text DEFAULT 'QUEUED'::text NOT NULL,
    attempt integer DEFAULT 0 NOT NULL,
    nonce uuid,
    lease_until timestamp with time zone,
    receipt jsonb,
    error_code text,
    created_by uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT managed_deployment_jobs_attempt_check CHECK ((attempt >= 0)),
    CONSTRAINT managed_deployment_jobs_operation_check CHECK ((operation = ANY (ARRAY['DEPLOY'::text, 'CLEANUP'::text]))),
    CONSTRAINT managed_deployment_jobs_state_check CHECK ((state = ANY (ARRAY['QUEUED'::text, 'RUNNING'::text, 'READY'::text, 'FAILED'::text, 'CLEANED'::text]))),
    CONSTRAINT uuid_v7_artifact_id CHECK (public.is_uuid_v7(artifact_id)),
    CONSTRAINT uuid_v7_created_by CHECK (public.is_uuid_v7(created_by)),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id)),
    CONSTRAINT uuid_v7_nonce CHECK (public.is_uuid_v7(nonce)),
    CONSTRAINT uuid_v7_release_id CHECK (public.is_uuid_v7(release_id)),
    CONSTRAINT uuid_v7_request_id CHECK (public.is_uuid_v7(request_id))
);

CREATE TABLE public.managed_image_artifacts (
    id uuid NOT NULL,
    application_id uuid NOT NULL,
    version text NOT NULL,
    target_id text NOT NULL,
    template_id text NOT NULL,
    image_ref text NOT NULL,
    policy_revision text NOT NULL,
    public_url text NOT NULL,
    state text DEFAULT 'PENDING'::text NOT NULL,
    image_id text,
    route_sha256 text,
    container_name text,
    ever_ready boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT managed_image_artifacts_state_check CHECK ((state = ANY (ARRAY['PENDING'::text, 'READY'::text, 'CLEANING'::text, 'REMOVED'::text]))),
    CONSTRAINT uuid_v7_application_id CHECK (public.is_uuid_v7(application_id)),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id))
);

CREATE TABLE public.membership_data_grants (
    id uuid NOT NULL,
    membership_id uuid NOT NULL,
    application_id uuid NOT NULL,
    resource_code character varying(100) NOT NULL,
    action_code character varying(100) NOT NULL,
    scope_type character varying(20) NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT ck_membership_data_grants_resource CHECK (((length(TRIM(BOTH FROM resource_code)) > 0) AND (length(TRIM(BOTH FROM action_code)) > 0))),
    CONSTRAINT ck_membership_data_grants_scope CHECK (((scope_type)::text = ANY (ARRAY[('SELF'::character varying)::text, ('ORG'::character varying)::text, ('ORG_SUBTREE'::character varying)::text, ('TENANT'::character varying)::text]))),
    CONSTRAINT uuid_v7_application_id CHECK (public.is_uuid_v7(application_id)),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id)),
    CONSTRAINT uuid_v7_membership_id CHECK (public.is_uuid_v7(membership_id))
);

CREATE TABLE public.memberships (
    id uuid NOT NULL,
    user_id uuid NOT NULL,
    organization_id uuid NOT NULL,
    department_id uuid,
    "position" character varying(50) DEFAULT ''::character varying,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    member_type character varying(20) DEFAULT 'EMPLOYEE'::character varying NOT NULL,
    status character varying(20) DEFAULT 'ENABLED'::character varying NOT NULL,
    identity_version bigint DEFAULT 1 NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT memberships_customer_profile_check CHECK ((((member_type)::text <> 'CUSTOMER'::text) OR ((department_id IS NULL) AND ((COALESCE("position", ''::character varying))::text = ''::text)))),
    CONSTRAINT memberships_identity_version_check CHECK ((identity_version >= 1)),
    CONSTRAINT memberships_member_type_check CHECK (((member_type)::text = ANY ((ARRAY['EMPLOYEE'::character varying, 'CUSTOMER'::character varying])::text[]))),
    CONSTRAINT memberships_status_check CHECK (((status)::text = ANY ((ARRAY['ENABLED'::character varying, 'DISABLED'::character varying])::text[]))),
    CONSTRAINT uuid_v7_department_id CHECK (public.is_uuid_v7(department_id)),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id)),
    CONSTRAINT uuid_v7_organization_id CHECK (public.is_uuid_v7(organization_id)),
    CONSTRAINT uuid_v7_user_id CHECK (public.is_uuid_v7(user_id))
);

CREATE TABLE public.menus (
    id uuid NOT NULL,
    application_id uuid NOT NULL,
    parent_id uuid,
    type character varying(20) DEFAULT 'MENU'::character varying NOT NULL,
    title character varying(100) NOT NULL,
    path character varying(200) DEFAULT ''::character varying,
    component character varying(200) DEFAULT ''::character varying,
    icon character varying(100) DEFAULT ''::character varying,
    permission character varying(100) DEFAULT ''::character varying,
    sort_order integer DEFAULT 0 NOT NULL,
    visible boolean DEFAULT true NOT NULL,
    status character varying(20) DEFAULT 'ENABLED'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    link_url character varying(500) DEFAULT ''::character varying NOT NULL,
    link_target character varying(10) DEFAULT 'BLANK'::character varying NOT NULL,
    CONSTRAINT uuid_v7_application_id CHECK (public.is_uuid_v7(application_id)),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id)),
    CONSTRAINT uuid_v7_parent_id CHECK (public.is_uuid_v7(parent_id))
);

CREATE TABLE public.notification_member_prefs (
    membership_id uuid NOT NULL,
    identity_version bigint NOT NULL,
    dnd_enabled boolean DEFAULT false NOT NULL,
    dnd_start character varying(5) DEFAULT '22:00'::character varying NOT NULL,
    dnd_end character varying(5) DEFAULT '07:00'::character varying NOT NULL,
    muted_types text DEFAULT ''::text NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT notification_member_prefs_identity_version_check CHECK ((identity_version >= 1)),
    CONSTRAINT notification_member_prefs_membership_id_check CHECK (public.is_uuid_v7(membership_id))
);

CREATE TABLE public.notification_prefs (
    user_id uuid NOT NULL,
    dnd_enabled boolean DEFAULT false NOT NULL,
    dnd_start character varying(5) DEFAULT '22:00'::character varying NOT NULL,
    dnd_end character varying(5) DEFAULT '07:00'::character varying NOT NULL,
    muted_types text DEFAULT ''::text NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT uuid_v7_user_id CHECK (public.is_uuid_v7(user_id))
);

CREATE TABLE public.notification_publications (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    publisher_organization_id uuid NOT NULL,
    application_id uuid NOT NULL,
    publisher_user_id uuid NOT NULL,
    publisher_username character varying(50) NOT NULL,
    application_name character varying(200) NOT NULL,
    idempotency_key character varying(128) NOT NULL,
    request_hash character(64) NOT NULL,
    request_body jsonb NOT NULL,
    recipient_count integer NOT NULL,
    delivery_count integer NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    publisher_membership_id uuid,
    publisher_member_type text,
    publisher_identity_version bigint,
    CONSTRAINT notification_publications_check CHECK ((delivery_count >= recipient_count)),
    CONSTRAINT notification_publications_publisher_membership_id_check CHECK (public.is_uuid_v7(publisher_membership_id)),
    CONSTRAINT notification_publications_publisher_proof_check CHECK ((((publisher_membership_id IS NULL) AND (publisher_member_type IS NULL) AND (publisher_identity_version IS NULL)) OR ((publisher_membership_id IS NOT NULL) AND (publisher_member_type IS NOT NULL) AND (publisher_identity_version IS NOT NULL) AND (publisher_member_type = ANY (ARRAY['EMPLOYEE'::text, 'CUSTOMER'::text])) AND (publisher_identity_version >= 1)))),
    CONSTRAINT notification_publications_recipient_count_check CHECK ((recipient_count >= 0)),
    CONSTRAINT notification_publications_request_body_check CHECK ((jsonb_typeof(request_body) = 'object'::text)),
    CONSTRAINT uuid_v7_application_id CHECK (public.is_uuid_v7(application_id)),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id)),
    CONSTRAINT uuid_v7_publisher_organization_id CHECK (public.is_uuid_v7(publisher_organization_id)),
    CONSTRAINT uuid_v7_publisher_user_id CHECK (public.is_uuid_v7(publisher_user_id)),
    CONSTRAINT uuid_v7_tenant_id CHECK (public.is_uuid_v7(tenant_id))
);

CREATE TABLE public.notifications (
    id uuid NOT NULL,
    user_id uuid NOT NULL,
    type character varying(20) DEFAULT 'INFO'::character varying NOT NULL,
    title character varying(200) NOT NULL,
    content text DEFAULT ''::text,
    link character varying(300) DEFAULT ''::character varying,
    is_read boolean DEFAULT false NOT NULL,
    read_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    source_kind character varying(12) DEFAULT 'LEGACY'::character varying NOT NULL,
    tenant_id uuid,
    organization_id uuid,
    application_id uuid,
    publication_id uuid,
    recipient_membership_id uuid,
    recipient_member_type text,
    recipient_identity_version bigint,
    CONSTRAINT ck_notification_context CHECK (((((source_kind)::text = 'LEGACY'::text) AND (tenant_id IS NULL) AND (organization_id IS NULL) AND (application_id IS NULL) AND (publication_id IS NULL)) OR (((source_kind)::text = 'APP'::text) AND (tenant_id IS NOT NULL) AND (organization_id IS NOT NULL) AND (application_id IS NOT NULL) AND (publication_id IS NOT NULL)) OR (((source_kind)::text = 'SYSTEM'::text) AND (tenant_id IS NOT NULL) AND (organization_id IS NOT NULL) AND (application_id IS NULL) AND (publication_id IS NULL)))),
    CONSTRAINT notifications_recipient_membership_id_check CHECK (public.is_uuid_v7(recipient_membership_id)),
    CONSTRAINT notifications_recipient_proof_check CHECK ((((recipient_membership_id IS NULL) AND (recipient_member_type IS NULL) AND (recipient_identity_version IS NULL)) OR ((recipient_membership_id IS NOT NULL) AND (recipient_member_type IS NOT NULL) AND (recipient_identity_version IS NOT NULL) AND (recipient_member_type = ANY (ARRAY['EMPLOYEE'::text, 'CUSTOMER'::text])) AND (recipient_identity_version >= 1)))),
    CONSTRAINT uuid_v7_application_id CHECK (public.is_uuid_v7(application_id)),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id)),
    CONSTRAINT uuid_v7_organization_id CHECK (public.is_uuid_v7(organization_id)),
    CONSTRAINT uuid_v7_publication_id CHECK (public.is_uuid_v7(publication_id)),
    CONSTRAINT uuid_v7_tenant_id CHECK (public.is_uuid_v7(tenant_id)),
    CONSTRAINT uuid_v7_user_id CHECK (public.is_uuid_v7(user_id))
);

CREATE TABLE public.oauth_authorization_codes (
    id uuid NOT NULL,
    code_hash text NOT NULL,
    client_id character varying(100) NOT NULL,
    user_id uuid NOT NULL,
    tenant_id uuid,
    organization_id uuid,
    redirect_uri text NOT NULL,
    scope text DEFAULT ''::text,
    code_challenge text DEFAULT ''::text,
    code_challenge_method character varying(10) DEFAULT ''::character varying,
    expires_at timestamp with time zone NOT NULL,
    consumed boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    account_version bigint,
    membership_id uuid,
    member_type text,
    identity_version bigint,
    client_policy_version bigint,
    application_id uuid,
    application_policy_version bigint,
    origin_issuer text,
    origin_client_ref text,
    origin_client_policy_version bigint,
    platform_session_id uuid,
    CONSTRAINT oauth_authorization_codes_account_version_check CHECK ((account_version >= 0)),
    CONSTRAINT oauth_authorization_codes_application_id_check CHECK (public.is_uuid_v7(application_id)),
    CONSTRAINT oauth_authorization_codes_application_policy_version_check CHECK ((application_policy_version >= 1)),
    CONSTRAINT oauth_authorization_codes_client_policy_version_check CHECK ((client_policy_version >= 1)),
    CONSTRAINT oauth_authorization_codes_identity_version_check CHECK ((identity_version >= 1)),
    CONSTRAINT oauth_authorization_codes_member_type_check CHECK ((member_type = ANY (ARRAY['EMPLOYEE'::text, 'CUSTOMER'::text]))),
    CONSTRAINT oauth_authorization_codes_membership_id_check CHECK (public.is_uuid_v7(membership_id)),
    CONSTRAINT oauth_authorization_codes_origin_client_policy_version_check CHECK ((origin_client_policy_version >= 1)),
    CONSTRAINT oauth_authorization_codes_platform_session_id_check CHECK (public.is_uuid_v7(platform_session_id)),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id)),
    CONSTRAINT uuid_v7_organization_id CHECK (public.is_uuid_v7(organization_id)),
    CONSTRAINT uuid_v7_tenant_id CHECK (public.is_uuid_v7(tenant_id)),
    CONSTRAINT uuid_v7_user_id CHECK (public.is_uuid_v7(user_id))
);

CREATE TABLE public.oauth_authorization_requests (
    id uuid NOT NULL,
    client_id character varying(100) NOT NULL,
    redirect_uri text NOT NULL,
    scope text DEFAULT ''::text NOT NULL,
    state text DEFAULT ''::text,
    code_challenge text DEFAULT ''::text,
    code_challenge_method character varying(10) DEFAULT ''::character varying,
    expires_at timestamp with time zone NOT NULL,
    consumed boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    client_policy_version bigint,
    application_id uuid,
    application_policy_version bigint,
    CONSTRAINT oauth_authorization_requests_application_id_check CHECK (public.is_uuid_v7(application_id)),
    CONSTRAINT oauth_authorization_requests_application_policy_version_check CHECK ((application_policy_version >= 1)),
    CONSTRAINT oauth_authorization_requests_client_policy_version_check CHECK ((client_policy_version >= 1)),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id))
);

CREATE TABLE public.oauth_clients (
    id uuid NOT NULL,
    client_id character varying(100) NOT NULL,
    client_secret_hash text,
    client_name character varying(200) NOT NULL,
    description text DEFAULT ''::text,
    client_type character varying(20) DEFAULT 'CONFIDENTIAL'::character varying NOT NULL,
    redirect_uris text DEFAULT ''::text,
    grant_types text DEFAULT 'authorization_code,refresh_token'::text,
    scopes text DEFAULT 'profile'::text,
    access_token_validity bigint DEFAULT 7200 NOT NULL,
    refresh_token_validity bigint DEFAULT 604800 NOT NULL,
    status character varying(20) DEFAULT 'ENABLED'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    application_id uuid,
    require_pkce boolean DEFAULT false NOT NULL,
    subject_mode character varying(20) DEFAULT 'USER'::character varying NOT NULL,
    allowed_member_types text[] DEFAULT ARRAY['EMPLOYEE'::text] NOT NULL,
    access_policy_version bigint DEFAULT 1 NOT NULL,
    CONSTRAINT oauth_clients_access_policy_version_check CHECK ((access_policy_version >= 1)),
    CONSTRAINT oauth_clients_allowed_member_types_check CHECK ((public.valid_member_type_set(allowed_member_types, ((subject_mode)::text = 'MACHINE'::text)) AND (((subject_mode)::text = 'MACHINE'::text) = (cardinality(allowed_member_types) = 0)))),
    CONSTRAINT oauth_clients_subject_grants_check CHECK (((((subject_mode)::text = 'USER'::text) AND (NOT ('client_credentials'::text = ANY (public.oauth_client_grant_set(string_to_array(grant_types, ','::text)))))) OR (((subject_mode)::text = 'MACHINE'::text) AND ((client_type)::text = 'CONFIDENTIAL'::text) AND (public.oauth_client_grant_set(string_to_array(grant_types, ','::text)) = ARRAY['client_credentials'::text])) OR (((subject_mode)::text = 'LEGACY_MIXED'::text) AND ((client_type)::text = 'CONFIDENTIAL'::text) AND ('client_credentials'::text = ANY (public.oauth_client_grant_set(string_to_array(grant_types, ','::text)))) AND (cardinality(public.oauth_client_grant_set(string_to_array(grant_types, ','::text))) > 1)))),
    CONSTRAINT oauth_clients_subject_mode_check CHECK (((subject_mode)::text = ANY ((ARRAY['USER'::character varying, 'MACHINE'::character varying, 'LEGACY_MIXED'::character varying])::text[]))),
    CONSTRAINT uuid_v7_application_id CHECK (public.is_uuid_v7(application_id)),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id))
);

CREATE TABLE public.oauth_device_codes (
    id uuid NOT NULL,
    device_code_hash text NOT NULL,
    user_code character varying(20) NOT NULL,
    client_id character varying(100) NOT NULL,
    scope text DEFAULT ''::text,
    status character varying(20) DEFAULT 'PENDING'::character varying NOT NULL,
    user_id uuid,
    tenant_id uuid,
    organization_id uuid,
    interval_seconds bigint DEFAULT 5 NOT NULL,
    last_polled_at timestamp with time zone,
    expires_at timestamp with time zone NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    account_version bigint,
    membership_id uuid,
    member_type text,
    identity_version bigint,
    client_policy_version bigint,
    application_id uuid,
    application_policy_version bigint,
    origin_issuer text,
    origin_client_ref text,
    origin_client_policy_version bigint,
    platform_session_id uuid,
    CONSTRAINT oauth_device_codes_account_version_check CHECK ((account_version >= 0)),
    CONSTRAINT oauth_device_codes_application_id_check CHECK (public.is_uuid_v7(application_id)),
    CONSTRAINT oauth_device_codes_application_policy_version_check CHECK ((application_policy_version >= 1)),
    CONSTRAINT oauth_device_codes_client_policy_version_check CHECK ((client_policy_version >= 1)),
    CONSTRAINT oauth_device_codes_identity_version_check CHECK ((identity_version >= 1)),
    CONSTRAINT oauth_device_codes_member_type_check CHECK ((member_type = ANY (ARRAY['EMPLOYEE'::text, 'CUSTOMER'::text]))),
    CONSTRAINT oauth_device_codes_membership_id_check CHECK (public.is_uuid_v7(membership_id)),
    CONSTRAINT oauth_device_codes_origin_client_policy_version_check CHECK ((origin_client_policy_version >= 1)),
    CONSTRAINT oauth_device_codes_platform_session_id_check CHECK (public.is_uuid_v7(platform_session_id)),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id)),
    CONSTRAINT uuid_v7_organization_id CHECK (public.is_uuid_v7(organization_id)),
    CONSTRAINT uuid_v7_tenant_id CHECK (public.is_uuid_v7(tenant_id)),
    CONSTRAINT uuid_v7_user_id CHECK (public.is_uuid_v7(user_id))
);

CREATE TABLE public.oauth_tokens (
    id uuid NOT NULL,
    token_hash text NOT NULL,
    token_type character varying(20) NOT NULL,
    jwt text,
    user_id uuid,
    client_id character varying(100) NOT NULL,
    tenant_id uuid,
    organization_id uuid,
    scope text DEFAULT ''::text,
    expires_at timestamp with time zone NOT NULL,
    revoked boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    token_use character varying(20) DEFAULT 'oauth'::character varying NOT NULL,
    audience character varying(100),
    jti uuid,
    application_id uuid,
    platform_session_id uuid,
    storage_version smallint DEFAULT 2 NOT NULL,
    account_version bigint,
    membership_id uuid,
    member_type text,
    identity_version bigint,
    client_policy_version bigint,
    application_policy_version bigint,
    origin_issuer text,
    origin_client_ref text,
    origin_client_policy_version bigint,
    application_service_client_ref text,
    CONSTRAINT ck_oauth_tokens_app_binding CHECK ((((token_use)::text <> 'app'::text) OR (((token_type)::text = 'access'::text) AND (user_id IS NOT NULL) AND (tenant_id IS NOT NULL) AND (organization_id IS NOT NULL) AND (audience IS NOT NULL) AND (jti IS NOT NULL) AND (application_id IS NOT NULL) AND (platform_session_id IS NOT NULL)))),
    CONSTRAINT ck_oauth_tokens_storage_version CHECK ((storage_version = ANY (ARRAY[1, 2]))),
    CONSTRAINT oauth_tokens_account_version_check CHECK ((account_version >= 0)),
    CONSTRAINT oauth_tokens_application_policy_version_check CHECK ((application_policy_version >= 1)),
    CONSTRAINT oauth_tokens_client_policy_version_check CHECK ((client_policy_version >= 1)),
    CONSTRAINT oauth_tokens_identity_version_check CHECK ((identity_version >= 1)),
    CONSTRAINT oauth_tokens_member_type_check CHECK ((member_type = ANY (ARRAY['EMPLOYEE'::text, 'CUSTOMER'::text]))),
    CONSTRAINT oauth_tokens_membership_id_check CHECK (public.is_uuid_v7(membership_id)),
    CONSTRAINT oauth_tokens_origin_client_policy_version_check CHECK ((origin_client_policy_version >= 1)),
    CONSTRAINT uuid_v7_application_id CHECK (public.is_uuid_v7(application_id)),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id)),
    CONSTRAINT uuid_v7_jti CHECK (public.is_uuid_v7(jti)),
    CONSTRAINT uuid_v7_organization_id CHECK (public.is_uuid_v7(organization_id)),
    CONSTRAINT uuid_v7_platform_session_id CHECK (public.is_uuid_v7(platform_session_id)),
    CONSTRAINT uuid_v7_tenant_id CHECK (public.is_uuid_v7(tenant_id)),
    CONSTRAINT uuid_v7_user_id CHECK (public.is_uuid_v7(user_id))
);

CREATE TABLE public.oidc_access_tokens (
    id uuid NOT NULL,
    grant_id uuid NOT NULL,
    token_hash text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    revoked_at timestamp with time zone,
    CONSTRAINT oidc_access_tokens_check CHECK ((expires_at > created_at)),
    CONSTRAINT oidc_access_tokens_grant_id_check CHECK (public.is_uuid_v7(grant_id)),
    CONSTRAINT oidc_access_tokens_id_check CHECK (public.is_uuid_v7(id)),
    CONSTRAINT oidc_access_tokens_token_hash_check CHECK ((token_hash ~ '^[0-9a-f]{64}$'::text))
);

CREATE TABLE public.oidc_account_logout_receipts (
    id uuid NOT NULL,
    client_id uuid NOT NULL,
    grant_id uuid NOT NULL,
    user_id uuid NOT NULL,
    token_hash text NOT NULL,
    account_version bigint NOT NULL,
    created_at timestamp with time zone NOT NULL,
    CONSTRAINT oidc_account_logout_receipts_account_version_check CHECK ((account_version >= 0)),
    CONSTRAINT oidc_account_logout_receipts_client_id_check CHECK (public.is_uuid_v7(client_id)),
    CONSTRAINT oidc_account_logout_receipts_grant_id_check CHECK (public.is_uuid_v7(grant_id)),
    CONSTRAINT oidc_account_logout_receipts_id_check CHECK (public.is_uuid_v7(id)),
    CONSTRAINT oidc_account_logout_receipts_token_hash_check CHECK ((token_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT oidc_account_logout_receipts_user_id_check CHECK (public.is_uuid_v7(user_id))
);

CREATE TABLE public.oidc_authorization_codes (
    id uuid NOT NULL,
    grant_id uuid NOT NULL,
    token_hash text NOT NULL,
    redirect_uri text NOT NULL,
    code_challenge text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    consumed_at timestamp with time zone,
    CONSTRAINT oidc_authorization_codes_check CHECK ((expires_at > created_at)),
    CONSTRAINT oidc_authorization_codes_code_challenge_check CHECK ((code_challenge ~ '^[A-Za-z0-9_-]{43}$'::text)),
    CONSTRAINT oidc_authorization_codes_grant_id_check CHECK (public.is_uuid_v7(grant_id)),
    CONSTRAINT oidc_authorization_codes_id_check CHECK (public.is_uuid_v7(id)),
    CONSTRAINT oidc_authorization_codes_token_hash_check CHECK ((token_hash ~ '^[0-9a-f]{64}$'::text))
);

CREATE TABLE public.oidc_authorization_requests (
    id uuid NOT NULL,
    browser_hash text NOT NULL,
    csrf_hash text NOT NULL,
    client_id uuid NOT NULL,
    client_policy_version bigint NOT NULL,
    redirect_uri text NOT NULL,
    scope text[] NOT NULL,
    state text NOT NULL,
    nonce text NOT NULL,
    code_challenge text NOT NULL,
    force_login boolean DEFAULT false NOT NULL,
    max_age bigint,
    session_id uuid,
    user_id uuid,
    account_version bigint,
    created_at timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    consumed_at timestamp with time zone,
    CONSTRAINT oidc_authorization_requests_account_version_check CHECK ((account_version >= 0)),
    CONSTRAINT oidc_authorization_requests_browser_hash_check CHECK ((browser_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT oidc_authorization_requests_check CHECK ((expires_at > created_at)),
    CONSTRAINT oidc_authorization_requests_check1 CHECK ((((session_id IS NULL) AND (user_id IS NULL) AND (account_version IS NULL)) OR ((session_id IS NOT NULL) AND (user_id IS NOT NULL) AND (account_version IS NOT NULL)))),
    CONSTRAINT oidc_authorization_requests_client_id_check CHECK (public.is_uuid_v7(client_id)),
    CONSTRAINT oidc_authorization_requests_client_policy_version_check CHECK ((client_policy_version >= 1)),
    CONSTRAINT oidc_authorization_requests_code_challenge_check CHECK ((code_challenge ~ '^[A-Za-z0-9_-]{43}$'::text)),
    CONSTRAINT oidc_authorization_requests_csrf_hash_check CHECK ((csrf_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT oidc_authorization_requests_id_check CHECK (public.is_uuid_v7(id)),
    CONSTRAINT oidc_authorization_requests_max_age_check CHECK ((max_age >= 0)),
    CONSTRAINT oidc_authorization_requests_session_id_check CHECK (((session_id IS NULL) OR public.is_uuid_v7(session_id))),
    CONSTRAINT oidc_authorization_requests_user_id_check CHECK (((user_id IS NULL) OR public.is_uuid_v7(user_id)))
);

CREATE TABLE public.oidc_clients (
    id uuid NOT NULL,
    name text NOT NULL,
    purpose text NOT NULL,
    secret_hash text NOT NULL,
    allowed_member_types text[] NOT NULL,
    redirect_uris text[] NOT NULL,
    post_logout_redirect_uris text[] DEFAULT '{}'::text[] NOT NULL,
    refresh_enabled boolean DEFAULT false NOT NULL,
    status text NOT NULL,
    access_policy_version bigint NOT NULL,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    CONSTRAINT oidc_clients_access_policy_version_check CHECK ((access_policy_version >= 1)),
    CONSTRAINT oidc_clients_allowed_member_types_check CHECK ((((cardinality(allowed_member_types) >= 1) AND (cardinality(allowed_member_types) <= 2)) AND (allowed_member_types <@ ARRAY['EMPLOYEE'::text, 'CUSTOMER'::text]))),
    CONSTRAINT oidc_clients_check CHECK (((purpose <> 'PLATFORM_CONSOLE'::text) OR (allowed_member_types = ARRAY['EMPLOYEE'::text]))),
    CONSTRAINT oidc_clients_check1 CHECK (((purpose <> 'CUSTOMER_PORTAL'::text) OR (allowed_member_types = ARRAY['CUSTOMER'::text]))),
    CONSTRAINT oidc_clients_id_check CHECK (public.is_uuid_v7(id)),
    CONSTRAINT oidc_clients_name_check CHECK (((length(name) >= 1) AND (length(name) <= 120))),
    CONSTRAINT oidc_clients_purpose_check CHECK ((purpose = ANY (ARRAY['PLATFORM_CONSOLE'::text, 'CUSTOMER_PORTAL'::text, 'ACCOUNT'::text]))),
    CONSTRAINT oidc_clients_redirect_uris_check CHECK (((cardinality(redirect_uris) >= 1) AND (cardinality(redirect_uris) <= 16))),
    CONSTRAINT oidc_clients_secret_hash_check CHECK ((secret_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT oidc_clients_status_check CHECK ((status = ANY (ARRAY['ENABLED'::text, 'DISABLED'::text])))
);

CREATE TABLE public.oidc_grants (
    id uuid NOT NULL,
    client_id uuid NOT NULL,
    client_policy_version bigint NOT NULL,
    session_id uuid NOT NULL,
    user_id uuid NOT NULL,
    account_version bigint NOT NULL,
    scope text[] NOT NULL,
    nonce text NOT NULL,
    auth_time timestamp with time zone NOT NULL,
    created_at timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    revoked_at timestamp with time zone,
    CONSTRAINT oidc_grants_account_version_check CHECK ((account_version >= 0)),
    CONSTRAINT oidc_grants_check CHECK ((expires_at > created_at)),
    CONSTRAINT oidc_grants_client_id_check CHECK (public.is_uuid_v7(client_id)),
    CONSTRAINT oidc_grants_client_policy_version_check CHECK ((client_policy_version >= 1)),
    CONSTRAINT oidc_grants_id_check CHECK (public.is_uuid_v7(id)),
    CONSTRAINT oidc_grants_session_id_check CHECK (public.is_uuid_v7(session_id)),
    CONSTRAINT oidc_grants_user_id_check CHECK (public.is_uuid_v7(user_id))
);

CREATE TABLE public.oidc_provider_settings (
    singleton boolean NOT NULL,
    issuer text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    CONSTRAINT oidc_provider_settings_singleton_check CHECK (singleton)
);

CREATE TABLE public.oidc_refresh_tokens (
    id uuid NOT NULL,
    grant_id uuid NOT NULL,
    token_hash text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    used_at timestamp with time zone,
    revoked_at timestamp with time zone,
    CONSTRAINT oidc_refresh_tokens_check CHECK ((expires_at > created_at)),
    CONSTRAINT oidc_refresh_tokens_grant_id_check CHECK (public.is_uuid_v7(grant_id)),
    CONSTRAINT oidc_refresh_tokens_id_check CHECK (public.is_uuid_v7(id)),
    CONSTRAINT oidc_refresh_tokens_token_hash_check CHECK ((token_hash ~ '^[0-9a-f]{64}$'::text))
);

CREATE TABLE public.oidc_signing_keys (
    kid text NOT NULL,
    n text NOT NULL,
    e text NOT NULL,
    published_at timestamp with time zone NOT NULL,
    active boolean DEFAULT false NOT NULL,
    activated_at timestamp with time zone,
    retired_at timestamp with time zone,
    revoked_at timestamp with time zone,
    last_signed_exp timestamp with time zone,
    CONSTRAINT oidc_signing_keys_e_check CHECK ((e = 'AQAB'::text)),
    CONSTRAINT oidc_signing_keys_kid_check CHECK ((kid ~ '^[A-Za-z0-9_-]{1,80}$'::text))
);

CREATE TABLE public.organization_applications (
    organization_id uuid NOT NULL,
    application_id uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT uuid_v7_application_id CHECK (public.is_uuid_v7(application_id)),
    CONSTRAINT uuid_v7_organization_id CHECK (public.is_uuid_v7(organization_id))
);

CREATE TABLE public.organization_menus (
    organization_id uuid NOT NULL,
    menu_id uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT uuid_v7_menu_id CHECK (public.is_uuid_v7(menu_id)),
    CONSTRAINT uuid_v7_organization_id CHECK (public.is_uuid_v7(organization_id))
);

CREATE TABLE public.organizations (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    parent_id uuid,
    name character varying(100) NOT NULL,
    code character varying(50) DEFAULT ''::character varying,
    sort_order integer DEFAULT 0 NOT NULL,
    description text DEFAULT ''::text,
    status character varying(20) DEFAULT 'ENABLED'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    expires_at timestamp with time zone,
    province character varying(100) DEFAULT ''::character varying NOT NULL,
    city character varying(100) DEFAULT ''::character varying NOT NULL,
    district character varying(100) DEFAULT ''::character varying NOT NULL,
    address character varying(300) DEFAULT ''::character varying NOT NULL,
    longitude double precision,
    latitude double precision,
    township character varying(128) DEFAULT ''::character varying NOT NULL,
    village character varying(128) DEFAULT ''::character varying NOT NULL,
    organization_type character varying(30) DEFAULT 'COMPANY'::character varying NOT NULL,
    contact_name character varying(100) DEFAULT ''::character varying NOT NULL,
    contact_phone character varying(30) DEFAULT ''::character varying NOT NULL,
    contact_email character varying(100) DEFAULT ''::character varying NOT NULL,
    CONSTRAINT ck_organization_coordinates CHECK ((((longitude IS NULL) AND (latitude IS NULL)) OR ((longitude IS NOT NULL) AND (latitude IS NOT NULL) AND ((longitude >= ('-180'::integer)::double precision) AND (longitude <= (180)::double precision)) AND ((latitude >= ('-90'::integer)::double precision) AND (latitude <= (90)::double precision))))),
    CONSTRAINT ck_organization_type CHECK (((organization_type)::text = ANY ((ARRAY['COMPANY'::character varying, 'SUBSIDIARY'::character varying, 'BRANCH'::character varying, 'STORE'::character varying, 'PUBLIC_INSTITUTION'::character varying, 'GOVERNMENT'::character varying, 'OTHER'::character varying])::text[]))),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id)),
    CONSTRAINT uuid_v7_parent_id CHECK (public.is_uuid_v7(parent_id)),
    CONSTRAINT uuid_v7_tenant_id CHECK (public.is_uuid_v7(tenant_id))
);

CREATE TABLE public.platform_account_command_intents (
    id uuid NOT NULL,
    target_user_id uuid NOT NULL,
    actor_user_id uuid NOT NULL,
    actor_tenant_id uuid NOT NULL,
    actor_organization_id uuid NOT NULL,
    actor_session_id uuid NOT NULL,
    expected_account_version bigint CONSTRAINT platform_account_command_inte_expected_account_version_not_null NOT NULL,
    canonical_digest text NOT NULL,
    command_kind text NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    state text NOT NULL,
    followup jsonb NOT NULL,
    receipt jsonb,
    created_at timestamp with time zone NOT NULL,
    completed_at timestamp with time zone,
    CONSTRAINT platform_account_command_intents_actor_organization_id_check CHECK (public.is_uuid_v7(actor_organization_id)),
    CONSTRAINT platform_account_command_intents_actor_session_id_check CHECK (public.is_uuid_v7(actor_session_id)),
    CONSTRAINT platform_account_command_intents_actor_tenant_id_check CHECK (public.is_uuid_v7(actor_tenant_id)),
    CONSTRAINT platform_account_command_intents_actor_user_id_check CHECK (public.is_uuid_v7(actor_user_id)),
    CONSTRAINT platform_account_command_intents_canonical_digest_check CHECK ((canonical_digest ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT platform_account_command_intents_check CHECK ((expires_at > created_at)),
    CONSTRAINT platform_account_command_intents_check1 CHECK (((state <> 'COMPLETED'::text) OR (completed_at IS NOT NULL))),
    CONSTRAINT platform_account_command_intents_command_kind_check CHECK ((command_kind = ANY (ARRAY['CREATE'::text, 'UPDATE_PROFILE'::text, 'SET_STATUS'::text, 'RESET_PASSWORD'::text, 'CHANGE_PASSWORD'::text, 'REVOKE_ALL'::text, 'RETIRE'::text]))),
    CONSTRAINT platform_account_command_intents_expected_account_version_check CHECK ((expected_account_version >= 0)),
    CONSTRAINT platform_account_command_intents_followup_check CHECK ((jsonb_typeof(followup) = 'object'::text)),
    CONSTRAINT platform_account_command_intents_id_check CHECK (public.is_uuid_v7(id)),
    CONSTRAINT platform_account_command_intents_receipt_check CHECK (((receipt IS NULL) OR (jsonb_typeof(receipt) = 'object'::text))),
    CONSTRAINT platform_account_command_intents_state_check CHECK ((state = ANY (ARRAY['PENDING_SECRET'::text, 'UNKNOWN'::text, 'COMPLETED'::text, 'ABANDONED'::text, 'REJECTED'::text]))),
    CONSTRAINT platform_account_command_intents_target_user_id_check CHECK (public.is_uuid_v7(target_user_id))
);

CREATE TABLE public.platform_oidc_command_intents (
    id uuid NOT NULL,
    actor_user_id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    organization_id uuid NOT NULL,
    session_id uuid NOT NULL,
    service_client_id uuid NOT NULL,
    request_digest text NOT NULL,
    method text NOT NULL,
    path text NOT NULL,
    accepted_at timestamp with time zone NOT NULL,
    CONSTRAINT platform_oidc_command_intents_actor_user_id_check CHECK (public.is_uuid_v7(actor_user_id)),
    CONSTRAINT platform_oidc_command_intents_id_check CHECK (public.is_uuid_v7(id)),
    CONSTRAINT platform_oidc_command_intents_method_check CHECK ((method = ANY (ARRAY['POST'::text, 'PATCH'::text, 'DELETE'::text]))),
    CONSTRAINT platform_oidc_command_intents_organization_id_check CHECK (public.is_uuid_v7(organization_id)),
    CONSTRAINT platform_oidc_command_intents_path_check CHECK ((path ~ '^/internal/identity/oidc/(clients|keys)(/[A-Za-z0-9_-]+){0,2}$'::text)),
    CONSTRAINT platform_oidc_command_intents_request_digest_check CHECK ((request_digest ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT platform_oidc_command_intents_service_client_id_check CHECK (public.is_uuid_v7(service_client_id)),
    CONSTRAINT platform_oidc_command_intents_session_id_check CHECK (public.is_uuid_v7(session_id)),
    CONSTRAINT platform_oidc_command_intents_tenant_id_check CHECK (public.is_uuid_v7(tenant_id))
);

CREATE TABLE public.platform_session_refresh_tokens (
    id uuid NOT NULL,
    session_id uuid NOT NULL,
    token_hash character varying(64) NOT NULL,
    session_version bigint NOT NULL,
    issued_at timestamp with time zone DEFAULT now() NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    consumed_at timestamp with time zone,
    CONSTRAINT platform_session_refresh_tokens_check CHECK ((expires_at > issued_at)),
    CONSTRAINT platform_session_refresh_tokens_session_version_check CHECK ((session_version >= 0)),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id)),
    CONSTRAINT uuid_v7_session_id CHECK (public.is_uuid_v7(session_id))
);

CREATE TABLE public.platform_sessions (
    id uuid NOT NULL,
    user_id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    organization_id uuid NOT NULL,
    issued_at timestamp with time zone DEFAULT now() NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    revoked_at timestamp with time zone,
    ip character varying(50) DEFAULT ''::character varying NOT NULL,
    user_agent text DEFAULT ''::text NOT NULL,
    account_version bigint,
    membership_id uuid,
    member_type text,
    identity_version bigint,
    origin_issuer text,
    origin_client_ref text,
    origin_client_policy_version bigint,
    bff_session_id uuid,
    bff_context_version bigint,
    CONSTRAINT platform_bff_binding_pair CHECK (((bff_session_id IS NULL) = (bff_context_version IS NULL))),
    CONSTRAINT platform_sessions_account_version_check CHECK ((account_version >= 0)),
    CONSTRAINT platform_sessions_bff_context_version_check CHECK (((bff_context_version IS NULL) OR (bff_context_version >= 1))),
    CONSTRAINT platform_sessions_bff_session_id_check CHECK (((bff_session_id IS NULL) OR public.is_uuid_v7(bff_session_id))),
    CONSTRAINT platform_sessions_check CHECK ((expires_at > issued_at)),
    CONSTRAINT platform_sessions_identity_version_check CHECK ((identity_version >= 1)),
    CONSTRAINT platform_sessions_member_type_check CHECK ((member_type = ANY (ARRAY['EMPLOYEE'::text, 'CUSTOMER'::text]))),
    CONSTRAINT platform_sessions_membership_id_check CHECK (public.is_uuid_v7(membership_id)),
    CONSTRAINT platform_sessions_origin_client_policy_version_check CHECK ((origin_client_policy_version >= 1)),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id)),
    CONSTRAINT uuid_v7_organization_id CHECK (public.is_uuid_v7(organization_id)),
    CONSTRAINT uuid_v7_tenant_id CHECK (public.is_uuid_v7(tenant_id)),
    CONSTRAINT uuid_v7_user_id CHECK (public.is_uuid_v7(user_id))
);

CREATE TABLE public.platform_settings (
    singleton boolean DEFAULT true NOT NULL,
    platform_name character varying(100) DEFAULT 'SaaS Platform'::character varying NOT NULL,
    platform_description character varying(300) DEFAULT '多租户权限管理平台'::character varying NOT NULL,
    logo_url character varying(2048) DEFAULT ''::character varying NOT NULL,
    login_title character varying(100) DEFAULT '欢迎回来'::character varying NOT NULL,
    login_description character varying(300) DEFAULT '管理您的租户与应用权限'::character varying NOT NULL,
    footer_text character varying(500) DEFAULT '© 2026 SaaS Platform · Powered by Rust & React'::character varying NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    primary_color character varying(7) DEFAULT '#008577'::character varying NOT NULL,
    CONSTRAINT platform_settings_primary_color_check CHECK (((primary_color)::text ~ '^#[0-9A-Fa-f]{6}$'::text)),
    CONSTRAINT platform_settings_singleton_check CHECK (singleton)
);

CREATE TABLE public.region_versions (
    version character varying(20) NOT NULL,
    label character varying(100) NOT NULL,
    source_version character varying(20),
    imported boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    deleted_at timestamp with time zone
);

CREATE TABLE public.regions (
    version character varying(20) NOT NULL,
    source_id character varying(32) NOT NULL,
    code character varying(12) NOT NULL,
    parent_code character varying(12) DEFAULT '0'::character varying NOT NULL,
    name character varying(128) NOT NULL,
    name_pinyin character varying(512) NOT NULL,
    name_short character varying(64) NOT NULL,
    level integer DEFAULT 1 NOT NULL,
    type character varying(12),
    type_name character varying(32),
    post_code character varying(512),
    source_url character varying(512),
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE public.role_menus (
    role_id uuid NOT NULL,
    menu_id uuid NOT NULL,
    CONSTRAINT uuid_v7_menu_id CHECK (public.is_uuid_v7(menu_id)),
    CONSTRAINT uuid_v7_role_id CHECK (public.is_uuid_v7(role_id))
);

CREATE TABLE public.roles (
    id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    organization_id uuid NOT NULL,
    name character varying(100) NOT NULL,
    code character varying(50) DEFAULT ''::character varying,
    description text DEFAULT ''::text,
    status character varying(20) DEFAULT 'ENABLED'::character varying NOT NULL,
    built_in boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id)),
    CONSTRAINT uuid_v7_organization_id CHECK (public.is_uuid_v7(organization_id)),
    CONSTRAINT uuid_v7_tenant_id CHECK (public.is_uuid_v7(tenant_id))
);

CREATE TABLE public.sso_login_challenges (
    id uuid NOT NULL,
    token_hash text NOT NULL,
    csrf_hash text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    consumed_at timestamp with time zone,
    CONSTRAINT sso_login_challenges_check CHECK ((expires_at > created_at)),
    CONSTRAINT sso_login_challenges_csrf_hash_check CHECK ((csrf_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT sso_login_challenges_id_check CHECK (public.is_uuid_v7(id)),
    CONSTRAINT sso_login_challenges_token_hash_check CHECK ((token_hash ~ '^[0-9a-f]{64}$'::text))
);

CREATE TABLE public.sso_sessions (
    id uuid NOT NULL,
    user_id uuid NOT NULL,
    token_hash text NOT NULL,
    csrf_hash text NOT NULL,
    account_version bigint NOT NULL,
    authenticated_at timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    revoked_at timestamp with time zone,
    CONSTRAINT sso_sessions_account_version_check CHECK ((account_version >= 0)),
    CONSTRAINT sso_sessions_check CHECK ((expires_at > authenticated_at)),
    CONSTRAINT sso_sessions_csrf_hash_check CHECK ((csrf_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT sso_sessions_id_check CHECK (public.is_uuid_v7(id)),
    CONSTRAINT sso_sessions_token_hash_check CHECK ((token_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT sso_sessions_user_id_check CHECK (public.is_uuid_v7(user_id))
);

CREATE TABLE public.tenant_applications (
    tenant_id uuid NOT NULL,
    application_id uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT uuid_v7_application_id CHECK (public.is_uuid_v7(application_id)),
    CONSTRAINT uuid_v7_tenant_id CHECK (public.is_uuid_v7(tenant_id))
);

CREATE TABLE public.tenant_menus (
    tenant_id uuid NOT NULL,
    menu_id uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT uuid_v7_menu_id CHECK (public.is_uuid_v7(menu_id)),
    CONSTRAINT uuid_v7_tenant_id CHECK (public.is_uuid_v7(tenant_id))
);

CREATE TABLE public.tenants (
    id uuid NOT NULL,
    name character varying(100) NOT NULL,
    code character varying(50) NOT NULL,
    description text DEFAULT ''::text,
    status character varying(20) DEFAULT 'ENABLED'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    is_platform boolean DEFAULT false NOT NULL,
    expires_at timestamp with time zone,
    province character varying(100) DEFAULT ''::character varying NOT NULL,
    city character varying(100) DEFAULT ''::character varying NOT NULL,
    district character varying(100) DEFAULT ''::character varying NOT NULL,
    address character varying(300) DEFAULT ''::character varying NOT NULL,
    longitude double precision,
    latitude double precision,
    township character varying(128) DEFAULT ''::character varying NOT NULL,
    village character varying(128) DEFAULT ''::character varying NOT NULL,
    tenant_type character varying(30) DEFAULT 'COMPANY'::character varying NOT NULL,
    contact_name character varying(100) DEFAULT ''::character varying NOT NULL,
    contact_phone character varying(30) DEFAULT ''::character varying NOT NULL,
    contact_email character varying(100) DEFAULT ''::character varying NOT NULL,
    CONSTRAINT ck_tenant_coordinates CHECK ((((longitude IS NULL) AND (latitude IS NULL)) OR ((longitude IS NOT NULL) AND (latitude IS NOT NULL) AND ((longitude >= ('-180'::integer)::double precision) AND (longitude <= (180)::double precision)) AND ((latitude >= ('-90'::integer)::double precision) AND (latitude <= (90)::double precision))))),
    CONSTRAINT ck_tenant_type CHECK (((tenant_type)::text = ANY (ARRAY[('PERSONAL'::character varying)::text, ('COMPANY'::character varying)::text, ('PUBLIC_INSTITUTION'::character varying)::text, ('GOVERNMENT'::character varying)::text, ('OTHER'::character varying)::text]))),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id))
);

CREATE TABLE public.user_roles (
    user_id uuid NOT NULL,
    role_id uuid NOT NULL,
    organization_id uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT uuid_v7_organization_id CHECK (public.is_uuid_v7(organization_id)),
    CONSTRAINT uuid_v7_role_id CHECK (public.is_uuid_v7(role_id)),
    CONSTRAINT uuid_v7_user_id CHECK (public.is_uuid_v7(user_id))
);

CREATE TABLE public.users (
    id uuid NOT NULL,
    username character varying(100) NOT NULL,
    password_hash text NOT NULL,
    real_name character varying(100) DEFAULT ''::character varying,
    email character varying(100) DEFAULT ''::character varying,
    phone character varying(30) DEFAULT ''::character varying,
    avatar text DEFAULT ''::text,
    gender character varying(10) DEFAULT 'UNKNOWN'::character varying,
    status character varying(20) DEFAULT 'ENABLED'::character varying NOT NULL,
    is_super_admin boolean DEFAULT false NOT NULL,
    last_login_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    session_version bigint DEFAULT 0 NOT NULL,
    CONSTRAINT users_session_version_check CHECK ((session_version >= 0)),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id))
);

CREATE TABLE public.zip_distribution_jobs (
    id uuid NOT NULL,
    application_id uuid NOT NULL,
    release_id uuid NOT NULL,
    group_id text NOT NULL,
    public_root text NOT NULL,
    artifact_digest text NOT NULL,
    manifest jsonb NOT NULL,
    policy_revision text NOT NULL,
    policy jsonb NOT NULL,
    request_id uuid NOT NULL,
    payload_sha256 text NOT NULL,
    state text DEFAULT 'QUEUED'::text NOT NULL,
    attempt integer DEFAULT 0 NOT NULL,
    nonce uuid,
    lease_until timestamp with time zone,
    verification_nonce uuid,
    verified_at timestamp with time zone,
    ever_ready boolean DEFAULT false NOT NULL,
    error_code text,
    created_by uuid NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT uuid_v7_application_id CHECK (public.is_uuid_v7(application_id)),
    CONSTRAINT uuid_v7_created_by CHECK (public.is_uuid_v7(created_by)),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id)),
    CONSTRAINT uuid_v7_nonce CHECK (public.is_uuid_v7(nonce)),
    CONSTRAINT uuid_v7_release_id CHECK (public.is_uuid_v7(release_id)),
    CONSTRAINT uuid_v7_request_id CHECK (public.is_uuid_v7(request_id)),
    CONSTRAINT uuid_v7_verification_nonce CHECK (public.is_uuid_v7(verification_nonce)),
    CONSTRAINT zip_distribution_jobs_artifact_digest_check CHECK ((artifact_digest ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT zip_distribution_jobs_attempt_check CHECK ((attempt >= 0)),
    CONSTRAINT zip_distribution_jobs_state_check CHECK ((state = ANY (ARRAY['QUEUED'::text, 'RUNNING'::text, 'READY'::text, 'FAILED'::text])))
);

CREATE TABLE public.zip_distribution_nodes (
    job_id uuid NOT NULL,
    node_id text NOT NULL,
    state text DEFAULT 'PENDING'::text NOT NULL,
    attempt integer DEFAULT 0 NOT NULL,
    receipt jsonb,
    error_code text,
    verified_at timestamp with time zone,
    CONSTRAINT uuid_v7_job_id CHECK (public.is_uuid_v7(job_id)),
    CONSTRAINT zip_distribution_nodes_state_check CHECK ((state = ANY (ARRAY['PENDING'::text, 'READY'::text, 'FAILED'::text])))
);

ALTER TABLE ONLY public.audit_logs ALTER COLUMN id SET DEFAULT nextval('public.audit_logs_id_seq'::regclass);

ALTER TABLE ONLY public.application_deployments
    ADD CONSTRAINT application_deployments_pkey PRIMARY KEY (application_id, environment);

ALTER TABLE ONLY public.application_release_artifacts
    ADD CONSTRAINT application_release_artifacts_application_id_version_artifa_key UNIQUE (application_id, version, artifact_digest);

ALTER TABLE ONLY public.application_release_artifacts
    ADD CONSTRAINT application_release_artifacts_pkey PRIMARY KEY (application_id, version);

ALTER TABLE ONLY public.application_release_operations
    ADD CONSTRAINT application_release_operations_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.application_releases
    ADD CONSTRAINT application_releases_application_id_environment_id_key UNIQUE (application_id, environment, id);

ALTER TABLE ONLY public.application_releases
    ADD CONSTRAINT application_releases_application_id_environment_version_key UNIQUE (application_id, environment, version);

ALTER TABLE public.application_releases
    ADD CONSTRAINT application_releases_entry_type_check CHECK (((entry_type)::text = 'HTML'::text)) NOT VALID;

ALTER TABLE ONLY public.application_releases
    ADD CONSTRAINT application_releases_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.application_url_entries
    ADD CONSTRAINT application_url_entries_pkey PRIMARY KEY (application_id, environment);

ALTER TABLE ONLY public.applications
    ADD CONSTRAINT applications_code_key UNIQUE (code);

ALTER TABLE ONLY public.applications
    ADD CONSTRAINT applications_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.audit_logs
    ADD CONSTRAINT audit_logs_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.bff_sessions
    ADD CONSTRAINT bff_sessions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.bff_sessions
    ADD CONSTRAINT bff_sessions_token_hash_key UNIQUE (token_hash);

ALTER TABLE public.oauth_tokens
    ADD CONSTRAINT ck_oauth_tokens_oauth_no_plaintext CHECK ((((token_use)::text <> 'oauth'::text) OR (storage_version = 1) OR (jwt IS NULL))) NOT VALID;

ALTER TABLE ONLY public.console_browsers
    ADD CONSTRAINT console_browsers_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.console_browsers
    ADD CONSTRAINT console_browsers_token_hash_key UNIQUE (token_hash);

ALTER TABLE ONLY public.console_login_flows
    ADD CONSTRAINT console_login_flows_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.console_login_flows
    ADD CONSTRAINT console_login_flows_state_hash_key UNIQUE (state_hash);

ALTER TABLE ONLY public.customer_portal_contexts
    ADD CONSTRAINT customer_portal_contexts_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.customer_portal_contexts
    ADD CONSTRAINT customer_portal_contexts_token_hash_key UNIQUE (token_hash);

ALTER TABLE ONLY public.departments
    ADD CONSTRAINT departments_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.identity_account_command_receipts
    ADD CONSTRAINT identity_account_command_receipts_pkey PRIMARY KEY (operation_id);

ALTER TABLE ONLY public.identity_account_revocation_facts
    ADD CONSTRAINT identity_account_revocation_facts_pkey PRIMARY KEY (user_id, account_version);

ALTER TABLE ONLY public.identity_account_tombstones
    ADD CONSTRAINT identity_account_tombstones_operation_id_key UNIQUE (operation_id);

ALTER TABLE ONLY public.identity_account_tombstones
    ADD CONSTRAINT identity_account_tombstones_pkey PRIMARY KEY (user_id);

ALTER TABLE ONLY public.identity_audit_logs
    ADD CONSTRAINT identity_audit_logs_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.identity_deployment_state
    ADD CONSTRAINT identity_deployment_state_pkey PRIMARY KEY (singleton);

ALTER TABLE ONLY public.identity_oidc_admin_receipts
    ADD CONSTRAINT identity_oidc_admin_receipts_pkey PRIMARY KEY (operation_id);

ALTER TABLE ONLY public.identity_service_clients
    ADD CONSTRAINT identity_service_clients_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.identity_service_clients
    ADD CONSTRAINT identity_service_clients_secret_hash_key UNIQUE (secret_hash);

ALTER TABLE ONLY public.managed_deployment_bindings
    ADD CONSTRAINT managed_deployment_bindings_pkey PRIMARY KEY (release_id);

ALTER TABLE ONLY public.managed_deployment_jobs
    ADD CONSTRAINT managed_deployment_jobs_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.managed_deployment_jobs
    ADD CONSTRAINT managed_deployment_jobs_request_id_key UNIQUE (request_id);

ALTER TABLE ONLY public.managed_image_artifacts
    ADD CONSTRAINT managed_image_artifacts_application_id_version_target_id_key UNIQUE (application_id, version, target_id);

ALTER TABLE ONLY public.managed_image_artifacts
    ADD CONSTRAINT managed_image_artifacts_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.managed_image_artifacts
    ADD CONSTRAINT managed_image_artifacts_public_url_key UNIQUE (public_url);

ALTER TABLE ONLY public.membership_data_grants
    ADD CONSTRAINT membership_data_grants_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.memberships
    ADD CONSTRAINT memberships_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.memberships
    ADD CONSTRAINT memberships_user_organization_type_key UNIQUE (user_id, organization_id, member_type);

ALTER TABLE ONLY public.menus
    ADD CONSTRAINT menus_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.notification_member_prefs
    ADD CONSTRAINT notification_member_prefs_pkey PRIMARY KEY (membership_id, identity_version);

ALTER TABLE ONLY public.notification_prefs
    ADD CONSTRAINT notification_prefs_pkey PRIMARY KEY (user_id);

ALTER TABLE ONLY public.notification_publications
    ADD CONSTRAINT notification_publications_id_tenant_id_application_id_key UNIQUE (id, tenant_id, application_id);

ALTER TABLE ONLY public.notification_publications
    ADD CONSTRAINT notification_publications_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.notifications
    ADD CONSTRAINT notifications_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.oauth_authorization_codes
    ADD CONSTRAINT oauth_authorization_codes_code_hash_key UNIQUE (code_hash);

ALTER TABLE ONLY public.oauth_authorization_codes
    ADD CONSTRAINT oauth_authorization_codes_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.oauth_authorization_requests
    ADD CONSTRAINT oauth_authorization_requests_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.oauth_clients
    ADD CONSTRAINT oauth_clients_client_id_key UNIQUE (client_id);

ALTER TABLE ONLY public.oauth_clients
    ADD CONSTRAINT oauth_clients_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.oauth_device_codes
    ADD CONSTRAINT oauth_device_codes_device_code_hash_key UNIQUE (device_code_hash);

ALTER TABLE ONLY public.oauth_device_codes
    ADD CONSTRAINT oauth_device_codes_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.oauth_device_codes
    ADD CONSTRAINT oauth_device_codes_user_code_key UNIQUE (user_code);

ALTER TABLE ONLY public.oauth_tokens
    ADD CONSTRAINT oauth_tokens_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.oauth_tokens
    ADD CONSTRAINT oauth_tokens_token_hash_key UNIQUE (token_hash);

ALTER TABLE ONLY public.oidc_access_tokens
    ADD CONSTRAINT oidc_access_tokens_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.oidc_access_tokens
    ADD CONSTRAINT oidc_access_tokens_token_hash_key UNIQUE (token_hash);

ALTER TABLE ONLY public.oidc_account_logout_receipts
    ADD CONSTRAINT oidc_account_logout_receipts_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.oidc_authorization_codes
    ADD CONSTRAINT oidc_authorization_codes_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.oidc_authorization_codes
    ADD CONSTRAINT oidc_authorization_codes_token_hash_key UNIQUE (token_hash);

ALTER TABLE ONLY public.oidc_authorization_requests
    ADD CONSTRAINT oidc_authorization_requests_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.oidc_clients
    ADD CONSTRAINT oidc_clients_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.oidc_grants
    ADD CONSTRAINT oidc_grants_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.oidc_provider_settings
    ADD CONSTRAINT oidc_provider_settings_pkey PRIMARY KEY (singleton);

ALTER TABLE ONLY public.oidc_refresh_tokens
    ADD CONSTRAINT oidc_refresh_tokens_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.oidc_refresh_tokens
    ADD CONSTRAINT oidc_refresh_tokens_token_hash_key UNIQUE (token_hash);

ALTER TABLE ONLY public.oidc_signing_keys
    ADD CONSTRAINT oidc_signing_keys_pkey PRIMARY KEY (kid);

ALTER TABLE ONLY public.organization_applications
    ADD CONSTRAINT organization_applications_pkey PRIMARY KEY (organization_id, application_id);

ALTER TABLE ONLY public.organization_menus
    ADD CONSTRAINT organization_menus_pkey PRIMARY KEY (organization_id, menu_id);

ALTER TABLE ONLY public.organizations
    ADD CONSTRAINT organizations_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.platform_account_command_intents
    ADD CONSTRAINT platform_account_command_intents_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.platform_oidc_command_intents
    ADD CONSTRAINT platform_oidc_command_intents_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.platform_session_refresh_tokens
    ADD CONSTRAINT platform_session_refresh_tokens_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.platform_session_refresh_tokens
    ADD CONSTRAINT platform_session_refresh_tokens_token_hash_key UNIQUE (token_hash);

ALTER TABLE ONLY public.platform_sessions
    ADD CONSTRAINT platform_sessions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.platform_settings
    ADD CONSTRAINT platform_settings_pkey PRIMARY KEY (singleton);

ALTER TABLE ONLY public.region_versions
    ADD CONSTRAINT region_versions_pkey PRIMARY KEY (version);

ALTER TABLE ONLY public.regions
    ADD CONSTRAINT regions_pkey PRIMARY KEY (version, source_id);

ALTER TABLE ONLY public.role_menus
    ADD CONSTRAINT role_menus_pkey PRIMARY KEY (role_id, menu_id);

ALTER TABLE ONLY public.roles
    ADD CONSTRAINT roles_organization_id_name_key UNIQUE (organization_id, name);

ALTER TABLE ONLY public.roles
    ADD CONSTRAINT roles_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.sso_login_challenges
    ADD CONSTRAINT sso_login_challenges_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.sso_login_challenges
    ADD CONSTRAINT sso_login_challenges_token_hash_key UNIQUE (token_hash);

ALTER TABLE ONLY public.sso_sessions
    ADD CONSTRAINT sso_sessions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.sso_sessions
    ADD CONSTRAINT sso_sessions_token_hash_key UNIQUE (token_hash);

ALTER TABLE ONLY public.tenant_applications
    ADD CONSTRAINT tenant_applications_pkey PRIMARY KEY (tenant_id, application_id);

ALTER TABLE ONLY public.tenant_menus
    ADD CONSTRAINT tenant_menus_pkey PRIMARY KEY (tenant_id, menu_id);

ALTER TABLE ONLY public.tenants
    ADD CONSTRAINT tenants_code_key UNIQUE (code);

ALTER TABLE ONLY public.tenants
    ADD CONSTRAINT tenants_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.membership_data_grants
    ADD CONSTRAINT uq_membership_data_grants UNIQUE (membership_id, application_id, resource_code, action_code);

ALTER TABLE ONLY public.notifications
    ADD CONSTRAINT uq_notification_publication_delivery UNIQUE (publication_id, recipient_membership_id, recipient_identity_version);

ALTER TABLE ONLY public.notification_publications
    ADD CONSTRAINT uq_notification_publication_member_request UNIQUE (tenant_id, publisher_organization_id, application_id, publisher_user_id, publisher_membership_id, publisher_identity_version, idempotency_key);

ALTER TABLE ONLY public.user_roles
    ADD CONSTRAINT user_roles_pkey PRIMARY KEY (user_id, role_id);

ALTER TABLE ONLY public.users
    ADD CONSTRAINT users_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.users
    ADD CONSTRAINT users_username_key UNIQUE (username);

ALTER TABLE ONLY public.zip_distribution_jobs
    ADD CONSTRAINT zip_distribution_jobs_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.zip_distribution_jobs
    ADD CONSTRAINT zip_distribution_jobs_release_id_key UNIQUE (release_id);

ALTER TABLE ONLY public.zip_distribution_jobs
    ADD CONSTRAINT zip_distribution_jobs_request_id_key UNIQUE (request_id);

ALTER TABLE ONLY public.zip_distribution_nodes
    ADD CONSTRAINT zip_distribution_nodes_pkey PRIMARY KEY (job_id, node_id);

CREATE INDEX application_release_operations_pending ON public.application_release_operations USING btree (application_id);

CREATE INDEX idx_audit_action ON public.audit_logs USING btree (action);

CREATE INDEX idx_audit_application_time ON public.audit_logs USING btree (application_id, created_at DESC) WHERE (application_id IS NOT NULL);

CREATE INDEX idx_audit_created ON public.audit_logs USING btree (created_at DESC);

CREATE INDEX idx_audit_request_id ON public.audit_logs USING btree (request_id) WHERE (request_id IS NOT NULL);

CREATE INDEX idx_bff_sessions_global ON public.bff_sessions USING btree (global_session_id);

CREATE INDEX idx_bff_sessions_user ON public.bff_sessions USING btree (user_id);

CREATE INDEX idx_console_login_flows_expiry ON public.console_login_flows USING btree (expires_at);

CREATE INDEX idx_customer_portal_contexts_subject ON public.customer_portal_contexts USING btree (user_id, membership_id);

CREATE INDEX idx_departments_org ON public.departments USING btree (organization_id);

CREATE INDEX idx_departments_parent ON public.departments USING btree (parent_id);

CREATE INDEX idx_membership_data_grants_context ON public.membership_data_grants USING btree (membership_id, application_id);

CREATE INDEX idx_memberships_org ON public.memberships USING btree (organization_id);

CREATE INDEX idx_memberships_user ON public.memberships USING btree (user_id);

CREATE INDEX idx_menus_app ON public.menus USING btree (application_id);

CREATE INDEX idx_menus_parent ON public.menus USING btree (parent_id);

CREATE INDEX idx_notifications_content_trgm ON public.notifications USING gin (content public.gin_trgm_ops);

CREATE INDEX idx_notifications_context_inbox ON public.notifications USING btree (user_id, tenant_id, organization_id, is_read, created_at DESC, id) WHERE ((source_kind)::text = ANY (ARRAY[('APP'::character varying)::text, ('SYSTEM'::character varying)::text]));

CREATE INDEX idx_notifications_member_inbox ON public.notifications USING btree (recipient_membership_id, recipient_identity_version, is_read, created_at DESC, id) WHERE (recipient_membership_id IS NOT NULL);

CREATE INDEX idx_notifications_title_trgm ON public.notifications USING gin (title public.gin_trgm_ops);

CREATE INDEX idx_notifications_user_created ON public.notifications USING btree (user_id, created_at DESC);

CREATE INDEX idx_notifications_user_read ON public.notifications USING btree (user_id, is_read);

CREATE INDEX idx_notifications_user_read_created ON public.notifications USING btree (user_id, is_read, created_at DESC);

CREATE INDEX idx_oauth_clients_application ON public.oauth_clients USING btree (application_id);

CREATE INDEX idx_oauth_codes_membership ON public.oauth_authorization_codes USING btree (membership_id);

CREATE INDEX idx_oauth_device_membership ON public.oauth_device_codes USING btree (membership_id);

CREATE INDEX idx_oauth_tokens_app_active ON public.oauth_tokens USING btree (application_id, expires_at DESC) WHERE (((token_use)::text = 'app'::text) AND (revoked = false));

CREATE INDEX idx_oauth_tokens_membership ON public.oauth_tokens USING btree (membership_id);

CREATE INDEX idx_oidc_access_grant ON public.oidc_access_tokens USING btree (grant_id);

CREATE INDEX idx_oidc_account_logout_receipts_user ON public.oidc_account_logout_receipts USING btree (user_id);

CREATE INDEX idx_oidc_grants_client ON public.oidc_grants USING btree (client_id);

CREATE INDEX idx_oidc_grants_session ON public.oidc_grants USING btree (session_id);

CREATE UNIQUE INDEX idx_oidc_one_active_key ON public.oidc_signing_keys USING btree ((true)) WHERE active;

CREATE INDEX idx_oidc_refresh_grant ON public.oidc_refresh_tokens USING btree (grant_id);

CREATE INDEX idx_oidc_requests_live ON public.oidc_authorization_requests USING btree (expires_at) WHERE (consumed_at IS NULL);

CREATE INDEX idx_org_menus_menu ON public.organization_menus USING btree (menu_id);

CREATE INDEX idx_orgs_parent ON public.organizations USING btree (parent_id);

CREATE INDEX idx_orgs_tenant ON public.organizations USING btree (tenant_id);

CREATE INDEX idx_platform_account_commands_target ON public.platform_account_command_intents USING btree (target_user_id, created_at);

CREATE INDEX idx_platform_session_refresh_tokens_session ON public.platform_session_refresh_tokens USING btree (session_id, expires_at DESC);

CREATE INDEX idx_platform_sessions_bff ON public.platform_sessions USING btree (bff_session_id);

CREATE INDEX idx_platform_sessions_membership ON public.platform_sessions USING btree (membership_id);

CREATE INDEX idx_platform_sessions_organization_active ON public.platform_sessions USING btree (organization_id, expires_at DESC) WHERE (revoked_at IS NULL);

CREATE INDEX idx_platform_sessions_origin ON public.platform_sessions USING btree (origin_client_ref);

CREATE INDEX idx_platform_sessions_user_active ON public.platform_sessions USING btree (user_id, expires_at DESC) WHERE (revoked_at IS NULL);

CREATE INDEX idx_regions_code ON public.regions USING btree (version, code);

CREATE INDEX idx_regions_name ON public.regions USING btree (version, name);

CREATE INDEX idx_regions_parent ON public.regions USING btree (version, parent_code, code);

CREATE UNIQUE INDEX idx_regions_version_code_unique ON public.regions USING btree (version, code);

CREATE INDEX idx_role_menus_menu ON public.role_menus USING btree (menu_id);

CREATE INDEX idx_roles_org ON public.roles USING btree (organization_id);

CREATE INDEX idx_roles_tenant ON public.roles USING btree (tenant_id);

CREATE INDEX idx_sso_login_challenges_expiry ON public.sso_login_challenges USING btree (expires_at);

CREATE INDEX idx_sso_sessions_account_active ON public.sso_sessions USING btree (user_id, expires_at) WHERE (revoked_at IS NULL);

CREATE INDEX idx_tenant_menus_menu ON public.tenant_menus USING btree (menu_id);

CREATE INDEX idx_tokens_client ON public.oauth_tokens USING btree (client_id);

CREATE INDEX idx_tokens_user ON public.oauth_tokens USING btree (user_id);

CREATE INDEX idx_user_roles_org ON public.user_roles USING btree (organization_id);

CREATE INDEX idx_user_roles_role ON public.user_roles USING btree (role_id);

CREATE INDEX idx_users_login_email ON public.users USING btree (lower((email)::text)) WHERE ((email)::text <> ''::text);

CREATE INDEX idx_users_login_phone ON public.users USING btree (regexp_replace((phone)::text, '[[:space:]()\-]'::text, ''::text, 'g'::text)) WHERE ((phone)::text <> ''::text);

CREATE UNIQUE INDEX managed_deployment_one_worker ON public.managed_deployment_jobs USING btree (artifact_id) WHERE (state = ANY (ARRAY['QUEUED'::text, 'RUNNING'::text]));

CREATE INDEX managed_deployment_queue ON public.managed_deployment_jobs USING btree (state, created_at);

CREATE INDEX managed_deployment_releases ON public.managed_deployment_jobs USING btree (release_id);

CREATE UNIQUE INDEX uniq_tenants_platform ON public.tenants USING btree (is_platform) WHERE is_platform;

CREATE UNIQUE INDEX uq_departments_id_organization ON public.departments USING btree (id, organization_id);

CREATE UNIQUE INDEX uq_oauth_tokens_jti ON public.oauth_tokens USING btree (jti) WHERE (jti IS NOT NULL);

CREATE UNIQUE INDEX uq_org_root_per_tenant ON public.organizations USING btree (tenant_id) WHERE (parent_id IS NULL);

CREATE UNIQUE INDEX uq_organizations_id_tenant ON public.organizations USING btree (id, tenant_id);

CREATE INDEX zip_distribution_application ON public.zip_distribution_jobs USING btree (application_id, created_at);

CREATE INDEX zip_distribution_public_root ON public.zip_distribution_jobs USING btree (public_root);

CREATE INDEX zip_distribution_queue ON public.zip_distribution_jobs USING btree (state, created_at);

CREATE TRIGGER application_release_artifact_immutable BEFORE UPDATE ON public.application_release_artifacts FOR EACH ROW EXECUTE FUNCTION public.protect_application_release_artifact();

CREATE TRIGGER application_release_definition_immutable BEFORE UPDATE ON public.application_releases FOR EACH ROW EXECUTE FUNCTION public.protect_application_release_definition();

CREATE TRIGGER trg_account_credential_generation AFTER UPDATE ON public.users FOR EACH ROW EXECUTE FUNCTION public.revoke_credentials_on_account_generation();

CREATE TRIGGER trg_account_sso_generation AFTER UPDATE ON public.users FOR EACH ROW EXECUTE FUNCTION public.revoke_sso_on_account_generation();

CREATE TRIGGER trg_app_token_application_revoke AFTER UPDATE OF status ON public.applications FOR EACH ROW EXECUTE FUNCTION public.revoke_app_tokens_on_application_disable();

CREATE TRIGGER trg_app_token_client_config_revoke AFTER UPDATE OF application_id, client_type, scopes, access_token_validity ON public.oauth_clients FOR EACH ROW EXECUTE FUNCTION public.revoke_app_tokens_on_client_config_change();

CREATE TRIGGER trg_app_token_client_revoke AFTER UPDATE OF status ON public.oauth_clients FOR EACH ROW EXECUTE FUNCTION public.revoke_app_tokens_on_client_disable();

CREATE TRIGGER trg_app_token_code_revoke AFTER UPDATE OF code ON public.applications FOR EACH ROW EXECUTE FUNCTION public.revoke_app_tokens_on_app_code_change();

CREATE TRIGGER trg_app_token_org_grant_revoke AFTER DELETE ON public.organization_applications FOR EACH ROW EXECUTE FUNCTION public.revoke_app_tokens_on_org_grant_delete();

CREATE TRIGGER trg_app_token_runtime_mode_revoke AFTER UPDATE OF runtime_mode ON public.applications FOR EACH ROW EXECUTE FUNCTION public.revoke_app_tokens_on_runtime_mode_change();

CREATE TRIGGER trg_app_token_tenant_grant_revoke AFTER DELETE ON public.tenant_applications FOR EACH ROW EXECUTE FUNCTION public.revoke_app_tokens_on_tenant_grant_delete();

CREATE TRIGGER trg_application_identity_policy BEFORE UPDATE ON public.applications FOR EACH ROW EXECUTE FUNCTION public.protect_application_identity_policy();

CREATE TRIGGER trg_application_policy_credential_generation AFTER UPDATE ON public.applications FOR EACH ROW EXECUTE FUNCTION public.revoke_changed_application_policy();

CREATE TRIGGER trg_client_policy_credential_generation AFTER UPDATE ON public.oauth_clients FOR EACH ROW EXECUTE FUNCTION public.revoke_changed_client_policy();

CREATE TRIGGER trg_departments_no_cycle BEFORE INSERT OR UPDATE OF parent_id ON public.departments FOR EACH ROW EXECUTE FUNCTION public.reject_department_cycle();

CREATE TRIGGER trg_membership_credential_generation AFTER UPDATE ON public.memberships FOR EACH ROW EXECUTE FUNCTION public.revoke_credentials_on_identity_update();

CREATE TRIGGER trg_membership_identity_immutable BEFORE UPDATE ON public.memberships FOR EACH ROW EXECUTE FUNCTION public.protect_membership_identity();

CREATE TRIGGER trg_membership_session_revoke AFTER DELETE ON public.memberships FOR EACH ROW EXECUTE FUNCTION public.revoke_sessions_on_membership_delete();

CREATE TRIGGER trg_notification_member_pref_identity BEFORE UPDATE ON public.notification_member_prefs FOR EACH ROW EXECUTE FUNCTION public.protect_notification_member_pref_identity();

CREATE TRIGGER trg_notification_publication_immutable BEFORE UPDATE ON public.notification_publications FOR EACH ROW EXECUTE FUNCTION public.reject_notification_publication_update();

CREATE TRIGGER trg_notification_recipient_immutable BEFORE UPDATE ON public.notifications FOR EACH ROW EXECUTE FUNCTION public.protect_notification_recipient();

CREATE TRIGGER trg_oauth_client_identity_policy BEFORE INSERT OR UPDATE ON public.oauth_clients FOR EACH ROW EXECUTE FUNCTION public.protect_oauth_client_identity_policy();

CREATE TRIGGER trg_oidc_client_policy AFTER UPDATE ON public.oidc_clients FOR EACH ROW EXECUTE FUNCTION public.revoke_oidc_on_client_policy();

CREATE TRIGGER trg_oidc_key_material BEFORE UPDATE ON public.oidc_signing_keys FOR EACH ROW EXECUTE FUNCTION public.oidc_key_material_immutable();

CREATE TRIGGER trg_oidc_sso_revocation AFTER UPDATE ON public.sso_sessions FOR EACH ROW EXECUTE FUNCTION public.revoke_oidc_on_sso_revocation();

CREATE TRIGGER trg_organization_session_state AFTER UPDATE OF status, expires_at ON public.organizations FOR EACH ROW EXECUTE FUNCTION public.sync_sessions_on_organization_state();

CREATE TRIGGER trg_organizations_no_cycle BEFORE INSERT OR UPDATE OF parent_id ON public.organizations FOR EACH ROW EXECUTE FUNCTION public.reject_organization_cycle();

CREATE TRIGGER trg_protect_legacy_console_origin BEFORE DELETE OR UPDATE ON public.oauth_clients FOR EACH ROW EXECUTE FUNCTION public.protect_legacy_console_origin();

CREATE TRIGGER trg_tenant_session_state AFTER UPDATE OF status, expires_at ON public.tenants FOR EACH ROW EXECUTE FUNCTION public.sync_sessions_on_tenant_state();

CREATE TRIGGER trg_user_session_state AFTER UPDATE OF status ON public.users FOR EACH ROW EXECUTE FUNCTION public.revoke_sessions_on_user_disable();

CREATE TRIGGER trg_user_session_version BEFORE UPDATE OF status ON public.users FOR EACH ROW EXECUTE FUNCTION public.bump_session_version_on_user_disable();

ALTER TABLE ONLY public.application_deployments
    ADD CONSTRAINT application_deployments_application_id_environment_active__fkey FOREIGN KEY (application_id, environment, active_release_id) REFERENCES public.application_releases(application_id, environment, id);

ALTER TABLE ONLY public.application_deployments
    ADD CONSTRAINT application_deployments_application_id_environment_previou_fkey FOREIGN KEY (application_id, environment, previous_release_id) REFERENCES public.application_releases(application_id, environment, id);

ALTER TABLE ONLY public.application_deployments
    ADD CONSTRAINT application_deployments_application_id_fkey FOREIGN KEY (application_id) REFERENCES public.applications(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.application_releases
    ADD CONSTRAINT application_release_artifact_reference FOREIGN KEY (application_id, version, artifact_digest) REFERENCES public.application_release_artifacts(application_id, version, artifact_digest);

ALTER TABLE ONLY public.application_release_artifacts
    ADD CONSTRAINT application_release_artifacts_application_id_fkey FOREIGN KEY (application_id) REFERENCES public.applications(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.application_release_operations
    ADD CONSTRAINT application_release_operations_application_id_fkey FOREIGN KEY (application_id) REFERENCES public.applications(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.application_releases
    ADD CONSTRAINT application_releases_application_id_fkey FOREIGN KEY (application_id) REFERENCES public.applications(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.application_url_entries
    ADD CONSTRAINT application_url_entries_application_id_fkey FOREIGN KEY (application_id) REFERENCES public.applications(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.application_url_entries
    ADD CONSTRAINT application_url_entries_updated_by_fkey FOREIGN KEY (updated_by) REFERENCES public.users(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.bff_sessions
    ADD CONSTRAINT bff_sessions_browser_id_fkey FOREIGN KEY (browser_id) REFERENCES public.console_browsers(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.bff_sessions
    ADD CONSTRAINT bff_sessions_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.console_login_flows
    ADD CONSTRAINT console_login_flows_browser_id_fkey FOREIGN KEY (browser_id) REFERENCES public.console_browsers(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.departments
    ADD CONSTRAINT departments_organization_id_fkey FOREIGN KEY (organization_id) REFERENCES public.organizations(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.departments
    ADD CONSTRAINT departments_parent_id_fkey FOREIGN KEY (parent_id) REFERENCES public.departments(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.departments
    ADD CONSTRAINT fk_departments_parent_same_organization FOREIGN KEY (parent_id, organization_id) REFERENCES public.departments(id, organization_id) DEFERRABLE;

ALTER TABLE ONLY public.memberships
    ADD CONSTRAINT fk_memberships_department_same_organization FOREIGN KEY (department_id, organization_id) REFERENCES public.departments(id, organization_id) DEFERRABLE;

ALTER TABLE ONLY public.notifications
    ADD CONSTRAINT fk_notification_organization_tenant FOREIGN KEY (organization_id, tenant_id) REFERENCES public.organizations(id, tenant_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.notifications
    ADD CONSTRAINT fk_notification_publication_context FOREIGN KEY (publication_id, tenant_id, application_id) REFERENCES public.notification_publications(id, tenant_id, application_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.organizations
    ADD CONSTRAINT fk_organizations_parent_same_tenant FOREIGN KEY (parent_id, tenant_id) REFERENCES public.organizations(id, tenant_id) DEFERRABLE;

ALTER TABLE ONLY public.roles
    ADD CONSTRAINT fk_roles_organization_same_tenant FOREIGN KEY (organization_id, tenant_id) REFERENCES public.organizations(id, tenant_id) DEFERRABLE;

ALTER TABLE ONLY public.identity_account_command_receipts
    ADD CONSTRAINT identity_account_command_receipts_service_id_fkey FOREIGN KEY (service_id) REFERENCES public.identity_service_clients(id);

ALTER TABLE ONLY public.identity_account_command_receipts
    ADD CONSTRAINT identity_account_command_receipts_target_user_id_fkey FOREIGN KEY (target_user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.identity_account_tombstones
    ADD CONSTRAINT identity_account_tombstones_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id);

ALTER TABLE ONLY public.identity_oidc_admin_receipts
    ADD CONSTRAINT identity_oidc_admin_receipts_service_id_fkey FOREIGN KEY (service_id) REFERENCES public.identity_service_clients(id);

ALTER TABLE ONLY public.managed_deployment_bindings
    ADD CONSTRAINT managed_deployment_bindings_artifact_id_fkey FOREIGN KEY (artifact_id) REFERENCES public.managed_image_artifacts(id);

ALTER TABLE ONLY public.managed_deployment_bindings
    ADD CONSTRAINT managed_deployment_bindings_release_id_fkey FOREIGN KEY (release_id) REFERENCES public.application_releases(id);

ALTER TABLE ONLY public.managed_deployment_jobs
    ADD CONSTRAINT managed_deployment_jobs_artifact_id_fkey FOREIGN KEY (artifact_id) REFERENCES public.managed_image_artifacts(id);

ALTER TABLE ONLY public.managed_deployment_jobs
    ADD CONSTRAINT managed_deployment_jobs_release_id_fkey FOREIGN KEY (release_id) REFERENCES public.application_releases(id);

ALTER TABLE ONLY public.managed_image_artifacts
    ADD CONSTRAINT managed_image_artifacts_application_id_fkey FOREIGN KEY (application_id) REFERENCES public.applications(id);

ALTER TABLE ONLY public.membership_data_grants
    ADD CONSTRAINT membership_data_grants_application_id_fkey FOREIGN KEY (application_id) REFERENCES public.applications(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.membership_data_grants
    ADD CONSTRAINT membership_data_grants_membership_id_fkey FOREIGN KEY (membership_id) REFERENCES public.memberships(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.memberships
    ADD CONSTRAINT memberships_department_id_fkey FOREIGN KEY (department_id) REFERENCES public.departments(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.memberships
    ADD CONSTRAINT memberships_organization_id_fkey FOREIGN KEY (organization_id) REFERENCES public.organizations(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.memberships
    ADD CONSTRAINT memberships_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.menus
    ADD CONSTRAINT menus_application_id_fkey FOREIGN KEY (application_id) REFERENCES public.applications(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.menus
    ADD CONSTRAINT menus_parent_id_fkey FOREIGN KEY (parent_id) REFERENCES public.menus(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.notification_prefs
    ADD CONSTRAINT notification_prefs_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.notification_publications
    ADD CONSTRAINT notification_publications_application_id_fkey FOREIGN KEY (application_id) REFERENCES public.applications(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.notification_publications
    ADD CONSTRAINT notification_publications_publisher_organization_id_tenant_fkey FOREIGN KEY (publisher_organization_id, tenant_id) REFERENCES public.organizations(id, tenant_id) ON DELETE CASCADE;

ALTER TABLE ONLY public.notification_publications
    ADD CONSTRAINT notification_publications_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES public.tenants(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.notifications
    ADD CONSTRAINT notifications_application_id_fkey FOREIGN KEY (application_id) REFERENCES public.applications(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.notifications
    ADD CONSTRAINT notifications_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES public.tenants(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.notifications
    ADD CONSTRAINT notifications_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.oauth_clients
    ADD CONSTRAINT oauth_clients_application_id_fkey FOREIGN KEY (application_id) REFERENCES public.applications(id) ON DELETE SET NULL;

ALTER TABLE ONLY public.oauth_tokens
    ADD CONSTRAINT oauth_tokens_application_id_fkey FOREIGN KEY (application_id) REFERENCES public.applications(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.oauth_tokens
    ADD CONSTRAINT oauth_tokens_platform_session_id_fkey FOREIGN KEY (platform_session_id) REFERENCES public.platform_sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.oidc_access_tokens
    ADD CONSTRAINT oidc_access_tokens_grant_id_fkey FOREIGN KEY (grant_id) REFERENCES public.oidc_grants(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.oidc_account_logout_receipts
    ADD CONSTRAINT oidc_account_logout_receipts_client_id_fkey FOREIGN KEY (client_id) REFERENCES public.oidc_clients(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.oidc_account_logout_receipts
    ADD CONSTRAINT oidc_account_logout_receipts_grant_id_fkey FOREIGN KEY (grant_id) REFERENCES public.oidc_grants(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.oidc_account_logout_receipts
    ADD CONSTRAINT oidc_account_logout_receipts_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.oidc_authorization_codes
    ADD CONSTRAINT oidc_authorization_codes_grant_id_fkey FOREIGN KEY (grant_id) REFERENCES public.oidc_grants(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.oidc_authorization_requests
    ADD CONSTRAINT oidc_authorization_requests_client_id_fkey FOREIGN KEY (client_id) REFERENCES public.oidc_clients(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.oidc_authorization_requests
    ADD CONSTRAINT oidc_authorization_requests_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sso_sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.oidc_authorization_requests
    ADD CONSTRAINT oidc_authorization_requests_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.oidc_grants
    ADD CONSTRAINT oidc_grants_client_id_fkey FOREIGN KEY (client_id) REFERENCES public.oidc_clients(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.oidc_grants
    ADD CONSTRAINT oidc_grants_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.sso_sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.oidc_grants
    ADD CONSTRAINT oidc_grants_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.oidc_refresh_tokens
    ADD CONSTRAINT oidc_refresh_tokens_grant_id_fkey FOREIGN KEY (grant_id) REFERENCES public.oidc_grants(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.organization_applications
    ADD CONSTRAINT organization_applications_application_id_fkey FOREIGN KEY (application_id) REFERENCES public.applications(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.organization_applications
    ADD CONSTRAINT organization_applications_organization_id_fkey FOREIGN KEY (organization_id) REFERENCES public.organizations(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.organization_menus
    ADD CONSTRAINT organization_menus_menu_id_fkey FOREIGN KEY (menu_id) REFERENCES public.menus(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.organization_menus
    ADD CONSTRAINT organization_menus_organization_id_fkey FOREIGN KEY (organization_id) REFERENCES public.organizations(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.organizations
    ADD CONSTRAINT organizations_parent_id_fkey FOREIGN KEY (parent_id) REFERENCES public.organizations(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.organizations
    ADD CONSTRAINT organizations_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES public.tenants(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.platform_session_refresh_tokens
    ADD CONSTRAINT platform_session_refresh_tokens_session_id_fkey FOREIGN KEY (session_id) REFERENCES public.platform_sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.platform_sessions
    ADD CONSTRAINT platform_sessions_bff_session_id_fkey FOREIGN KEY (bff_session_id) REFERENCES public.bff_sessions(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.platform_sessions
    ADD CONSTRAINT platform_sessions_organization_id_fkey FOREIGN KEY (organization_id) REFERENCES public.organizations(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.platform_sessions
    ADD CONSTRAINT platform_sessions_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES public.tenants(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.platform_sessions
    ADD CONSTRAINT platform_sessions_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.regions
    ADD CONSTRAINT regions_version_fkey FOREIGN KEY (version) REFERENCES public.region_versions(version) ON DELETE CASCADE;

ALTER TABLE ONLY public.role_menus
    ADD CONSTRAINT role_menus_menu_id_fkey FOREIGN KEY (menu_id) REFERENCES public.menus(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.role_menus
    ADD CONSTRAINT role_menus_role_id_fkey FOREIGN KEY (role_id) REFERENCES public.roles(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.roles
    ADD CONSTRAINT roles_organization_id_fkey FOREIGN KEY (organization_id) REFERENCES public.organizations(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.roles
    ADD CONSTRAINT roles_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES public.tenants(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.sso_sessions
    ADD CONSTRAINT sso_sessions_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.tenant_applications
    ADD CONSTRAINT tenant_applications_application_id_fkey FOREIGN KEY (application_id) REFERENCES public.applications(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.tenant_applications
    ADD CONSTRAINT tenant_applications_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES public.tenants(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.tenant_menus
    ADD CONSTRAINT tenant_menus_menu_id_fkey FOREIGN KEY (menu_id) REFERENCES public.menus(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.tenant_menus
    ADD CONSTRAINT tenant_menus_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES public.tenants(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.user_roles
    ADD CONSTRAINT user_roles_organization_id_fkey FOREIGN KEY (organization_id) REFERENCES public.organizations(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.user_roles
    ADD CONSTRAINT user_roles_role_id_fkey FOREIGN KEY (role_id) REFERENCES public.roles(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.user_roles
    ADD CONSTRAINT user_roles_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;

ALTER TABLE ONLY public.zip_distribution_jobs
    ADD CONSTRAINT zip_distribution_jobs_application_id_fkey FOREIGN KEY (application_id) REFERENCES public.applications(id);

ALTER TABLE ONLY public.zip_distribution_jobs
    ADD CONSTRAINT zip_distribution_jobs_release_id_fkey FOREIGN KEY (release_id) REFERENCES public.application_releases(id);

ALTER TABLE ONLY public.zip_distribution_nodes
    ADD CONSTRAINT zip_distribution_nodes_job_id_fkey FOREIGN KEY (job_id) REFERENCES public.zip_distribution_jobs(id);

REVOKE ALL ON FUNCTION public.bff_oidc_proof_active(subject_ref uuid, sid_ref uuid, grant_ref uuid, account_generation bigint, client_ref uuid, policy_generation bigint, expected_issuer text) FROM PUBLIC;

REVOKE ALL ON FUNCTION public.customer_portal_account_source_active(access_digest text, expected_issuer text, client_ref uuid, grant_ref uuid, sid_ref uuid, subject_ref uuid, account_generation bigint, policy_generation bigint, authentication_time bigint, source_deadline timestamp with time zone) FROM PUBLIC;

REVOKE ALL ON FUNCTION public.identity_lock_oidc_source(p_client_id text, p_issuer text) FROM PUBLIC;

REVOKE ALL ON FUNCTION public.identity_oidc_command_intent_active(operation_ref uuid, service_ref uuid, digest_ref text, method_ref text, path_ref text) FROM PUBLIC;

REVOKE ALL ON FUNCTION public.oidc_console_source_active(expected_issuer text, client_ref text, policy_version bigint, member_type text) FROM PUBLIC;

REVOKE ALL ON FUNCTION public.platform_account_command_intent_active(operation_ref uuid, target_ref uuid, account_generation bigint, digest_ref text, kind_ref text) FROM PUBLIC;

REVOKE ALL ON FUNCTION public.platform_password_context_active(subject_ref uuid, session_ref uuid, tenant_ref uuid, organization_ref uuid, account_generation bigint) FROM PUBLIC;

REVOKE ALL ON FUNCTION public.revoke_credentials_on_account_generation() FROM PUBLIC;

REVOKE ALL ON FUNCTION public.revoke_sessions_on_user_disable() FROM PUBLIC;

SET check_function_bodies = true;
SELECT pg_catalog.set_config('search_path','public,pg_catalog,pg_temp',false);

-- Required public platform branding; no accounts, sessions or business fixtures.
INSERT INTO public.platform_settings(singleton) VALUES(true);
