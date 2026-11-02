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

CREATE FUNCTION public.auth_user_grant_source_active(subject_ref uuid, sid_ref uuid, grant_ref uuid, account_generation bigint, client_ref uuid, policy_generation bigint, expected_issuer text, expected_purpose text, expected_member_type text) RETURNS boolean
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'public'
    AS $$
    SELECT EXISTS(SELECT 1 FROM public.auth_grants g
        JOIN public.auth_clients c ON c.id=g.client_id
        JOIN public.auth_applications a ON a.application_id=c.application_id
        JOIN public.sso_sessions s ON s.id=g.source_session_id AND s.user_id=g.user_id
        JOIN public.users u ON u.id=g.user_id
        JOIN public.auth_provider_settings p ON p.singleton AND p.issuer=expected_issuer
        JOIN public.auth_consents consent ON consent.id=g.consent_id AND consent.user_id=g.user_id
        JOIN public.auth_consent_groups cg ON cg.id=consent.group_id
        JOIN public.auth_consent_group_clients gc ON gc.group_id=cg.id AND gc.client_id=c.id
        JOIN public.auth_context_interactions context ON context.transaction_id=g.context_ref
        JOIN public.auth_authorization_requests request ON request.id=context.authorization_request_id
        WHERE g.id=grant_ref AND g.user_id=subject_ref AND g.source_session_id=sid_ref
          AND g.client_id=client_ref AND g.account_version=account_generation
          AND g.client_policy_version=policy_generation AND g.revoked_at IS NULL
          AND g.subject_mode='USER' AND g.context_mode='PLATFORM_CONTEXT'
          AND g.expires_at>clock_timestamp() AND s.revoked_at IS NULL
          AND s.expires_at>clock_timestamp() AND s.account_version=account_generation
          AND u.status='ENABLED' AND u.session_version=account_generation
          AND c.status='ENABLED' AND c.policy_version=policy_generation AND c.client_id=c.id::text
          AND c.subject_mode='USER' AND c.context_mode='PLATFORM_CONTEXT'
          AND c.client_type='CONFIDENTIAL' AND c.runtime_kind='WEB_BFF'
          AND c.allowed_member_types=ARRAY[expected_member_type]::text[]
          AND ((expected_purpose='PLATFORM_CONSOLE' AND expected_member_type='EMPLOYEE')
            OR (expected_purpose='CUSTOMER_PORTAL' AND expected_member_type='CUSTOMER'))
          AND a.status='APPROVED' AND a.purpose=expected_purpose
          AND a.projection_version=g.application_projection_version
          AND consent.status='ACTIVE' AND consent.revoked_at IS NULL
          AND consent.generation=g.consent_generation AND consent.group_version=g.consent_group_version
          AND cg.status='ACTIVE' AND cg.version=g.consent_group_version AND cg.purpose=a.purpose
          AND context.grant_id=g.id AND context.client_id=c.id
          AND context.client_policy_version=g.client_policy_version
          AND context.application_id=c.application_id
          AND context.application_projection_version=g.application_projection_version
          AND context.selected_membership_id IS NOT NULL AND context.delivered_at IS NOT NULL
          AND context.credential_ciphertext IS NULL AND request.status='CONSUMED'
          AND request.client_id=c.id AND request.user_id=g.user_id
          AND request.source_session_id=g.source_session_id AND request.account_version=g.account_version
          AND request.client_policy_version=g.client_policy_version
          AND request.application_projection_version=g.application_projection_version
          AND request.auth_time=g.auth_time
          AND EXISTS(SELECT 1 FROM public.auth_refresh_families family WHERE family.grant_id=g.id
              AND family.revoked_at IS NULL AND family.absolute_expires_at>clock_timestamp()
              AND family.idle_expires_at>clock_timestamp())
          AND NOT EXISTS(SELECT 1 FROM unnest(g.scope_ids) scope_ref
              LEFT JOIN public.auth_grant_scopes gs ON gs.grant_id=g.id AND gs.scope_id=scope_ref
              LEFT JOIN public.auth_scopes scope ON scope.id=gs.scope_id AND scope.resource_id=gs.resource_id
              LEFT JOIN public.auth_resources resource ON resource.id=scope.resource_id
              LEFT JOIN public.auth_client_permissions permission ON permission.client_id=c.id
                  AND permission.resource_id=resource.id AND permission.scope_id=scope.id
              LEFT JOIN public.auth_consent_scopes approved ON approved.consent_id=consent.id
                  AND approved.resource_id=resource.id AND approved.scope_id=scope.id
              WHERE scope.id IS NULL OR scope.status<>'ACTIVE' OR resource.status<>'ACTIVE'
                OR permission.scope_id IS NULL OR approved.scope_id IS NULL
                OR (permission.expires_at IS NOT NULL AND permission.expires_at<=clock_timestamp())
                OR NOT (scope.scope_name=ANY(a.approved_scopes))
                OR NOT (resource.audience=ANY(g.audiences)))
          AND (SELECT count(DISTINCT scope.scope_name) FROM public.auth_grant_scopes gs
              JOIN public.auth_scopes scope ON scope.id=gs.scope_id AND scope.resource_id=gs.resource_id
              JOIN public.auth_resources resource ON resource.id=scope.resource_id
              WHERE gs.grant_id=g.id AND scope.id=ANY(g.scope_ids)
                AND resource.resource_key='userinfo' AND scope.scope_name IN ('openid','profile'))=2
          AND NOT EXISTS(SELECT 1 FROM public.auth_scopes scope
              WHERE scope.id=ANY(g.scope_ids) AND scope.scope_name='offline_access'))
$$;

CREATE FUNCTION public.bff_auth_proof_active(subject_ref uuid, sid_ref uuid, grant_ref uuid, account_generation bigint, client_ref uuid, policy_generation bigint, expected_issuer text) RETURNS boolean
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'public'
    AS $$
    SELECT public.auth_user_grant_source_active(subject_ref,sid_ref,grant_ref,account_generation,
        client_ref,policy_generation,expected_issuer,'PLATFORM_CONSOLE','EMPLOYEE')
$$;

-- This narrow projection binds a local employee context to the member chosen
-- for its original canonical grant without exposing identity tables to platform.
CREATE FUNCTION public.bff_context_member_matches(grant_ref uuid, member_ref uuid) RETURNS boolean
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'public'
    AS $$
    SELECT EXISTS(SELECT 1 FROM public.auth_grants g
        JOIN public.auth_context_interactions context ON context.transaction_id=g.context_ref
        WHERE g.id=grant_ref AND g.subject_mode='USER' AND g.context_mode='PLATFORM_CONTEXT'
          AND context.grant_id=g.id AND context.client_id=g.client_id
          AND context.selected_membership_id=member_ref AND context.delivered_at IS NOT NULL
          AND context.credential_ciphertext IS NULL)
$$;

-- Credential separation can be checked across owners without revealing hashes.
CREATE FUNCTION public.identity_credential_digest_in_use(digest_ref text) RETURNS boolean
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'public'
    AS $$
    SELECT COALESCE(digest_ref ~ '^[0-9a-f]{64}$',false) AND (
        EXISTS(SELECT 1 FROM public.auth_client_credentials credential WHERE credential.secret_digest=digest_ref)
        OR EXISTS(SELECT 1 FROM public.identity_service_clients service WHERE service.secret_hash=digest_ref))
$$;

-- The identity runtime can authenticate services but cannot edit their secrets.
-- This narrow owner lock fences revocation during one source-proof transaction.
CREATE FUNCTION public.identity_context_source_service_active(service_ref uuid, expected_digest text, client_ref uuid) RETURNS boolean
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN
    PERFORM 1 FROM public.identity_service_clients service
      WHERE service.id=service_ref AND service.status='ENABLED'
        AND service.purpose='CONTEXT_SELECTION_SOURCE' AND service.secret_hash=expected_digest
        AND client_ref=ANY(service.allowed_auth_clients)
      FOR SHARE OF service;
    IF NOT FOUND THEN RETURN false; END IF;
    RETURN EXISTS(SELECT 1 FROM public.identity_service_clients service
      WHERE service.id=service_ref AND service.status='ENABLED'
        AND service.purpose='CONTEXT_SELECTION_SOURCE' AND service.secret_hash=expected_digest
        AND client_ref=ANY(service.allowed_auth_clients));
END;
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
          AND public.bff_auth_proof_active(b.user_id,b.global_session_id,b.oidc_grant_id,
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
    SELECT public.auth_user_grant_source_active(subject_ref,sid_ref,grant_ref,account_generation,
        client_ref,policy_generation,expected_issuer,'CUSTOMER_PORTAL','CUSTOMER')
      AND EXISTS(SELECT 1 FROM public.auth_access_tokens tok
        JOIN public.auth_grants g ON g.id=tok.grant_id
        JOIN public.sso_sessions s ON s.id=g.source_session_id AND s.user_id=g.user_id
        WHERE tok.token_digest=access_digest AND tok.revoked_at IS NULL
          AND tok.expires_at>clock_timestamp() AND g.id=grant_ref AND g.client_id=client_ref
          AND g.source_session_id=sid_ref AND g.user_id=subject_ref
          AND g.account_version=account_generation AND g.client_policy_version=policy_generation
          AND floor(extract(epoch FROM g.auth_time))::bigint=authentication_time
          AND (SELECT count(DISTINCT scope.scope_name) FROM public.auth_access_token_scopes token_scope
              JOIN public.auth_scopes scope ON scope.id=token_scope.scope_id AND scope.resource_id=token_scope.resource_id
              JOIN public.auth_resources resource ON resource.id=scope.resource_id
              WHERE token_scope.token_id=tok.id AND scope.id=ANY(g.scope_ids)
                AND resource.resource_key='userinfo' AND scope.scope_name IN ('openid','profile'))=2
          AND NOT EXISTS(SELECT 1 FROM public.auth_access_token_scopes token_scope
              WHERE token_scope.token_id=tok.id AND NOT (token_scope.scope_id=ANY(g.scope_ids)))
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

CREATE FUNCTION public.identity_lock_auth_source(p_client_id text, p_issuer text) RETURNS boolean
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN
    -- Lock the application before its client, matching identity policy writes.
    PERFORM 1 FROM public.auth_applications a JOIN public.auth_clients c ON c.application_id=a.application_id
      WHERE c.id::text=p_client_id AND c.client_id=p_client_id
        AND a.status='APPROVED' AND a.purpose='PLATFORM_CONSOLE'
      FOR SHARE OF a;
    IF NOT FOUND THEN RETURN false; END IF;
    PERFORM 1 FROM public.auth_clients c WHERE c.id::text=p_client_id AND c.client_id=p_client_id
      AND public.is_uuid_v7(c.id)
      AND c.status='ENABLED' AND c.subject_mode='USER' AND c.context_mode='PLATFORM_CONTEXT'
      AND c.client_type='CONFIDENTIAL' AND c.runtime_kind='WEB_BFF'
      AND c.allowed_member_types=ARRAY['EMPLOYEE']::text[]
      AND EXISTS(SELECT 1 FROM public.auth_provider_settings p WHERE p.singleton AND p.issuer=p_issuer)
    FOR SHARE OF c;
    IF NOT FOUND THEN RETURN false; END IF;
    -- Registering another group member advances this row. Hold its generation
    -- until the caller's final proof and local session commit have completed.
    PERFORM 1 FROM public.auth_consent_groups cg
      JOIN public.auth_consent_group_clients gc ON gc.group_id=cg.id
      JOIN public.auth_clients c ON c.id=gc.client_id
      WHERE c.id::text=p_client_id AND c.client_id=p_client_id AND cg.status='ACTIVE'
      FOR SHARE OF cg;
    RETURN FOUND;
END $$;

CREATE FUNCTION public.identity_admin_assurance(subject_ref uuid, sid_ref uuid, grant_ref uuid, account_generation bigint, client_ref uuid, policy_generation bigint, expected_issuer text)
RETURNS TABLE(totp_enabled boolean,mfa_method text,mfa_verified_at timestamptz)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path TO 'pg_catalog','public' AS $$
    SELECT EXISTS(SELECT 1 FROM public.auth_totp_factors factor WHERE factor.user_id=subject_ref),
        s.mfa_method,s.mfa_verified_at
    FROM public.sso_sessions s WHERE s.id=sid_ref AND s.user_id=subject_ref
      AND public.bff_auth_proof_active(subject_ref,sid_ref,grant_ref,account_generation,client_ref,policy_generation,expected_issuer)
$$;

CREATE FUNCTION public.platform_identity_management_actor_active(actor_ref uuid, session_ref uuid, permission_ref text, require_recent boolean, production boolean)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path TO 'pg_catalog','public' AS $$
DECLARE actor record; assurance record; allowed boolean;
BEGIN
    IF permission_ref<>ALL(ARRAY['system:oauth:view','system:oauth:register','system:oauth:review','system:oauth:publish','system:oauth:rotate','system:oauth:disable','system:oauth:scopes','system:oauth:invite','system:oauth:keys','system:oauth:assign']) THEN RETURN false; END IF;
    SELECT s.tenant_id,s.organization_id,u.is_super_admin,b.user_id,b.global_session_id,
        b.oidc_grant_id,b.account_version,b.oidc_client_id,b.oidc_client_policy_version,b.issuer INTO actor
    FROM public.platform_sessions s JOIN public.users u ON u.id=s.user_id
      JOIN public.bff_sessions b ON b.id=s.bff_session_id
      JOIN public.tenants tenant ON tenant.id=s.tenant_id AND tenant.is_platform
      JOIN public.organizations org ON org.id=s.organization_id AND org.tenant_id=tenant.id AND org.parent_id IS NULL
    WHERE s.id=session_ref AND s.user_id=actor_ref AND s.member_type='EMPLOYEE'
      AND public.platform_session_active(s.id) FOR SHARE OF u;
    IF NOT FOUND THEN RETURN false; END IF;
    SELECT EXISTS(SELECT 1 FROM public.menus menu
      JOIN public.applications app ON app.id=menu.application_id AND app.status='ENABLED' AND 'EMPLOYEE'=ANY(app.allowed_member_types)
      JOIN public.tenant_menus tm ON tm.menu_id=menu.id AND tm.tenant_id=actor.tenant_id
      JOIN public.organization_menus om ON om.menu_id=menu.id AND om.organization_id=actor.organization_id
      JOIN public.tenant_applications ta ON ta.application_id=app.id AND ta.tenant_id=actor.tenant_id
      JOIN public.organization_applications oa ON oa.application_id=app.id AND oa.organization_id=actor.organization_id
      WHERE menu.status='ENABLED' AND menu.permission=permission_ref
        AND (actor.is_super_admin OR EXISTS(SELECT 1 FROM public.user_roles ur
          JOIN public.roles role ON role.id=ur.role_id AND role.status='ENABLED'
            AND role.tenant_id=actor.tenant_id AND role.organization_id=actor.organization_id
          JOIN public.role_menus rm ON rm.role_id=role.id AND rm.menu_id=menu.id WHERE ur.user_id=actor_ref))) INTO allowed;
    IF NOT allowed THEN RETURN false; END IF;
    IF NOT require_recent AND NOT production THEN RETURN true; END IF;
    SELECT proof.* INTO assurance FROM public.identity_admin_assurance(actor.user_id,actor.global_session_id,actor.oidc_grant_id,
      actor.account_version,actor.oidc_client_id,actor.oidc_client_policy_version,actor.issuer) proof;
    IF NOT FOUND OR NOT assurance.totp_enabled THEN RETURN false; END IF;
    IF require_recent AND (assurance.mfa_method IS DISTINCT FROM 'totp' OR assurance.mfa_verified_at IS NULL
      OR assurance.mfa_verified_at>clock_timestamp() OR assurance.mfa_verified_at<clock_timestamp()-interval '5 minutes') THEN RETURN false; END IF;
    RETURN true;
END $$;

CREATE FUNCTION public.platform_identity_management_intent_actor_active(operation_ref uuid) RETURNS boolean
LANGUAGE sql VOLATILE SECURITY DEFINER SET search_path TO 'pg_catalog','public' AS $$
    SELECT EXISTS(SELECT 1 FROM public.platform_auth_command_intents intent WHERE intent.id=operation_ref
      AND CASE
        WHEN intent.path LIKE '%/invitations%' THEN intent.management_permission='system:oauth:invite'
        WHEN intent.path LIKE '%/review/applications/%' THEN intent.management_permission IN ('system:oauth:review','system:oauth:publish','system:oauth:disable')
        WHEN intent.path LIKE '%/credentials/revoke' THEN intent.management_permission='system:oauth:disable'
        WHEN intent.path LIKE '%/credentials' THEN intent.management_permission='system:oauth:rotate'
        WHEN intent.path LIKE '%/keys/%' THEN intent.management_permission='system:oauth:keys'
        WHEN intent.path LIKE '%/consent-groups%' OR intent.path LIKE '%/auth/applications/%' THEN intent.management_permission='system:oauth:scopes'
        WHEN intent.method='PATCH' THEN intent.management_permission IN ('system:oauth:scopes','system:oauth:disable')
        ELSE intent.management_permission='system:oauth:register'
      END
      AND public.platform_identity_management_actor_active(intent.actor_user_id,intent.session_id,intent.management_permission,true,false)
      AND (intent.application_ref IS NULL
        OR EXISTS(SELECT 1 FROM public.users actor WHERE actor.id=intent.actor_user_id AND actor.is_super_admin)
        OR public.platform_identity_management_actor_active(intent.actor_user_id,intent.session_id,'system:oauth:review',false,false)
        OR public.platform_identity_management_actor_active(intent.actor_user_id,intent.session_id,'system:oauth:assign',false,false)
        OR EXISTS(SELECT 1 FROM public.platform_auth_application_operators operator
          JOIN public.platform_auth_applications app USING(application_id)
          JOIN public.platform_auth_partners partner ON partner.id=app.partner_id AND partner.status='ACTIVE'
          WHERE operator.application_id=intent.application_ref AND operator.user_id=intent.actor_user_id)))
$$;

CREATE FUNCTION public.identity_auth_command_intent_active(operation_ref uuid, service_ref uuid, digest_ref text, method_ref text, path_ref text) RETURNS boolean
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'public'
    AS $$
BEGIN
    PERFORM 1 FROM public.platform_auth_command_intents intent
    WHERE intent.id=operation_ref AND intent.service_client_id=service_ref
      AND intent.request_digest=digest_ref AND intent.method=method_ref AND intent.path=path_ref
      AND intent.accepted_at<=pg_catalog.clock_timestamp()
      AND intent.accepted_at>pg_catalog.clock_timestamp()-interval '60 seconds'
    FOR SHARE OF intent;
    IF NOT FOUND OR NOT public.platform_identity_management_intent_actor_active(operation_ref) THEN RETURN false; END IF;
    -- A lock-only wait may leave the original tuple unchanged. Sample the
    -- deadline again in a new statement after acquiring that lock.
    RETURN EXISTS(SELECT 1 FROM public.platform_auth_command_intents intent
      WHERE intent.id=operation_ref AND intent.service_client_id=service_ref
        AND intent.request_digest=digest_ref AND intent.method=method_ref AND intent.path=path_ref
        AND intent.accepted_at<=pg_catalog.clock_timestamp()
        AND intent.accepted_at>pg_catalog.clock_timestamp()-interval '60 seconds'
        AND public.platform_identity_management_intent_actor_active(operation_ref));
END $$;

CREATE FUNCTION public.is_uuid_v7(value uuid) RETURNS boolean
    LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE
    SET search_path TO 'pg_catalog'
    AS $$ SELECT (get_byte(uuid_send(value), 6) >> 4) = 7
    AND (get_byte(uuid_send(value), 8) & 192) = 128 $$;

CREATE FUNCTION public.auth_console_source_active(expected_issuer text, client_ref text, policy_version bigint, member_type text) RETURNS boolean
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'public'
    AS $$
    SELECT EXISTS(SELECT 1 FROM public.auth_clients c
        JOIN public.auth_applications a ON a.application_id=c.application_id
        JOIN public.auth_provider_settings p ON p.singleton AND p.issuer=expected_issuer
        WHERE c.id::text=client_ref AND c.client_id=client_ref AND c.status='ENABLED'
          AND c.policy_version=policy_version AND a.status='APPROVED' AND a.purpose='PLATFORM_CONSOLE'
          AND c.subject_mode='USER' AND c.context_mode='PLATFORM_CONTEXT'
          AND c.client_type='CONFIDENTIAL' AND c.runtime_kind='WEB_BFF'
          AND member_type='EMPLOYEE' AND c.allowed_member_types=ARRAY['EMPLOYEE']::text[])
$$;

CREATE FUNCTION public.auth_key_material_immutable() RETURNS trigger
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
          AND public.bff_context_member_matches(b.oidc_grant_id,s.membership_id)
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
    SELECT public.auth_console_source_active(expected_issuer,client_ref,policy_version,member_type)
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

CREATE FUNCTION public.revoke_credentials_on_account_generation() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path TO 'pg_catalog', 'public'
    AS $$
DECLARE revoked_sessions bigint;
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
        INSERT INTO public.identity_account_revocation_facts AS fact(user_id,account_version,platform_session_count,created_at)
        VALUES(NEW.id,NEW.session_version,revoked_sessions,pg_catalog.clock_timestamp())
        ON CONFLICT(user_id,account_version) DO UPDATE SET
            platform_session_count=fact.platform_session_count+EXCLUDED.platform_session_count;
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
        INSERT INTO public.identity_account_revocation_facts AS fact(user_id,account_version,platform_session_count,created_at)
        VALUES(NEW.id,NEW.session_version,revoked_sessions,pg_catalog.clock_timestamp())
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
    context_selection_required boolean NOT NULL DEFAULT false,
    switch_session_id uuid CHECK (switch_session_id IS NULL OR public.is_uuid_v7(switch_session_id)),
    switch_version bigint CHECK (switch_version>=1),
    CONSTRAINT console_login_flows_switch_binding_check CHECK ((switch_session_id IS NULL)=(switch_version IS NULL)),
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
    created_at timestamp with time zone NOT NULL,
    CONSTRAINT identity_account_revocation_facts_account_version_check CHECK ((account_version >= 0)),
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

CREATE TABLE public.identity_auth_command_receipts (
    operation_id uuid NOT NULL,
    service_id uuid NOT NULL,
    request_digest text NOT NULL,
    service_secret_hash text NOT NULL,
    method text NOT NULL,
    path text NOT NULL,
    result_json jsonb NOT NULL,
    returns_secret boolean NOT NULL,
    created_at timestamp with time zone NOT NULL,
    CONSTRAINT identity_auth_command_receipts_method_check CHECK ((method = ANY (ARRAY['POST'::text, 'PATCH'::text, 'DELETE'::text, 'PUT'::text]))),
    CONSTRAINT identity_auth_command_receipts_operation_id_check CHECK (public.is_uuid_v7(operation_id)),
    CONSTRAINT identity_auth_command_receipts_path_check CHECK ((path ~ '^/internal/identity/auth/keys(/[A-Za-z0-9_-]+){0,2}$'::text OR path ~ '^/internal/identity/auth/applications/[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$'::text OR path ~ '^/internal/identity/auth/clients(/[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}(/credentials(/revoke)?)?)?$'::text OR path ~ '^/internal/identity/auth/consent-groups(/[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/clients)?$'::text OR path ~ '^/internal/identity/auth/invitations(/[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/revoke)?$'::text OR path ~ '^/internal/identity/auth/review/applications/[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/(test|production)$'::text)),
    CONSTRAINT identity_auth_command_receipts_request_digest_check CHECK ((request_digest ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT identity_auth_command_receipts_result_json_check CHECK ((jsonb_typeof(result_json) = 'object'::text)),
    CONSTRAINT identity_auth_command_receipts_service_id_check CHECK (public.is_uuid_v7(service_id)),
    CONSTRAINT identity_auth_command_receipts_service_secret_hash_check CHECK ((service_secret_hash ~ '^[0-9a-f]{64}$'::text))
);

CREATE TABLE public.identity_service_clients (
    id uuid NOT NULL,
    secret_hash text NOT NULL,
    purpose text NOT NULL,
    status text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    CONSTRAINT identity_service_clients_id_check CHECK (public.is_uuid_v7(id)),
    CONSTRAINT identity_service_clients_purpose_check CHECK ((purpose = ANY (ARRAY['ACCOUNT_COMMAND'::text, 'CUSTOMER_ACCESS_PROOF'::text, 'KEY_ADMIN'::text, 'APPLICATION_PROJECTION'::text, 'AUTH_ADMIN'::text, 'CONTEXT_SELECTION_SOURCE'::text, 'AUTH_INVITATION'::text, 'AUTH_INVITATION_CONSUME'::text, 'AUTH_INVITATION_DELIVERY'::text, 'AUTH_REVIEW'::text]))),
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

CREATE TABLE public.auth_provider_settings (
    singleton boolean NOT NULL,
    issuer text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    CONSTRAINT auth_provider_settings_singleton_check CHECK (singleton)
);

CREATE TABLE public.auth_signing_keys (
    kid text NOT NULL,
    n text NOT NULL,
    e text NOT NULL,
    published_at timestamp with time zone NOT NULL,
    active boolean DEFAULT false NOT NULL,
    activated_at timestamp with time zone,
    retired_at timestamp with time zone,
    revoked_at timestamp with time zone,
    last_signed_exp timestamp with time zone,
    CONSTRAINT auth_signing_keys_e_check CHECK ((e = 'AQAB'::text)),
    CONSTRAINT auth_signing_keys_kid_check CHECK ((kid ~ '^[A-Za-z0-9_-]{1,80}$'::text))
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

CREATE TABLE public.platform_auth_command_intents (
    id uuid NOT NULL,
    actor_user_id uuid NOT NULL,
    tenant_id uuid NOT NULL,
    organization_id uuid NOT NULL,
    session_id uuid NOT NULL,
    service_client_id uuid NOT NULL,
    request_digest text NOT NULL,
    method text NOT NULL,
    path text NOT NULL,
    management_permission text NOT NULL DEFAULT 'system:oauth:view'
        CHECK (management_permission IN ('system:oauth:view','system:oauth:register','system:oauth:review','system:oauth:publish','system:oauth:rotate','system:oauth:disable','system:oauth:scopes','system:oauth:invite','system:oauth:keys','system:oauth:assign')),
    application_ref uuid CHECK (public.is_uuid_v7(application_ref)),
    accepted_at timestamp with time zone NOT NULL,
    CONSTRAINT platform_auth_command_intents_actor_user_id_check CHECK (public.is_uuid_v7(actor_user_id)),
    CONSTRAINT platform_auth_command_intents_id_check CHECK (public.is_uuid_v7(id)),
    CONSTRAINT platform_auth_command_intents_method_check CHECK ((method = ANY (ARRAY['POST'::text, 'PATCH'::text, 'DELETE'::text, 'PUT'::text]))),
    CONSTRAINT platform_auth_command_intents_organization_id_check CHECK (public.is_uuid_v7(organization_id)),
    CONSTRAINT platform_auth_command_intents_path_check CHECK ((path ~ '^/internal/identity/auth/keys(/[A-Za-z0-9_-]+){0,2}$'::text OR path ~ '^/internal/identity/auth/applications/[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$'::text OR path ~ '^/internal/identity/auth/clients(/[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}(/credentials(/revoke)?)?)?$'::text OR path ~ '^/internal/identity/auth/consent-groups(/[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/clients)?$'::text OR path ~ '^/internal/identity/auth/invitations(/[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/revoke)?$'::text OR path ~ '^/internal/identity/auth/review/applications/[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/(test|production)$'::text)),
    CONSTRAINT platform_auth_command_intents_request_digest_check CHECK ((request_digest ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT platform_auth_command_intents_service_client_id_check CHECK (public.is_uuid_v7(service_client_id)),
    CONSTRAINT platform_auth_command_intents_session_id_check CHECK (public.is_uuid_v7(session_id)),
    CONSTRAINT platform_auth_command_intents_tenant_id_check CHECK (public.is_uuid_v7(tenant_id))
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
    mfa_verified_at timestamp with time zone,
    mfa_method text,
    expires_at timestamp with time zone NOT NULL,
    revoked_at timestamp with time zone,
    CONSTRAINT sso_sessions_account_version_check CHECK ((account_version >= 0)),
    CONSTRAINT sso_sessions_check CHECK ((expires_at > authenticated_at)),
    CONSTRAINT sso_sessions_csrf_hash_check CHECK ((csrf_hash ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT sso_sessions_id_check CHECK (public.is_uuid_v7(id)),
    CONSTRAINT sso_sessions_mfa_method_check CHECK (mfa_method IS NULL OR mfa_method IN ('totp','recovery_code')),
    CONSTRAINT sso_sessions_mfa_proof_check CHECK ((mfa_verified_at IS NULL) = (mfa_method IS NULL)),
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
    session_version bigint DEFAULT 1 NOT NULL,
    CONSTRAINT users_session_version_check CHECK ((session_version >= 1)),
    CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id))
);

-- Canonical identity model. Platform application/partner identifiers are
-- opaque projections; no FK grants identity runtime access to business records.
-- Platform-owned manual partner intake. A partner is never a business tenant.
CREATE TABLE public.platform_auth_partners (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    name text NOT NULL CHECK (length(btrim(name)) BETWEEN 1 AND 120),
    contact_name text NOT NULL CHECK (length(btrim(contact_name)) BETWEEN 1 AND 80),
    contact_channel text NOT NULL CHECK (length(btrim(contact_channel)) BETWEEN 1 AND 256),
    subject_reference uuid NOT NULL CHECK (public.is_uuid_v7(subject_reference)),
    status text NOT NULL DEFAULT 'ACTIVE' CHECK (status IN ('ACTIVE','SUSPENDED','RETIRED')),
    created_by uuid NOT NULL CHECK (public.is_uuid_v7(created_by)),
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE public.platform_auth_applications (
    application_id uuid PRIMARY KEY CHECK (public.is_uuid_v7(application_id)),
    partner_id uuid NOT NULL REFERENCES public.platform_auth_partners(id),
    display_name text NOT NULL CHECK (length(btrim(display_name)) BETWEEN 1 AND 120),
    created_by uuid NOT NULL CHECK (public.is_uuid_v7(created_by)),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE public.platform_auth_application_operators (
    application_id uuid NOT NULL REFERENCES public.platform_auth_applications(application_id),
    user_id uuid NOT NULL CHECK (public.is_uuid_v7(user_id)),
    assigned_by uuid NOT NULL CHECK (public.is_uuid_v7(assigned_by)),
    assigned_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY(application_id,user_id)
);
CREATE TABLE public.platform_auth_application_requests (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    application_id uuid NOT NULL REFERENCES public.platform_auth_applications(application_id),
    environment text NOT NULL CHECK (environment IN ('test','production')),
    manifest jsonb NOT NULL CHECK (jsonb_typeof(manifest)='object'),
    requested_by uuid NOT NULL CHECK (public.is_uuid_v7(requested_by)),
    submitted_by uuid NOT NULL CHECK (public.is_uuid_v7(submitted_by)),
    status text NOT NULL DEFAULT 'SUBMITTED' CHECK (status IN ('SUBMITTED','RETURNED','TEST_APPROVED','TEST_ACCEPTED','PRODUCTION_APPROVED','ENABLED','SUSPENDED')),
    reviewed_by uuid CHECK (public.is_uuid_v7(reviewed_by)),
    published_by uuid CHECK (public.is_uuid_v7(published_by)),
    reviewed_contact_reference uuid CHECK (public.is_uuid_v7(reviewed_contact_reference)),
    reviewed_domain_reference uuid CHECK (public.is_uuid_v7(reviewed_domain_reference)),
    test_evidence_reference uuid CHECK (public.is_uuid_v7(test_evidence_reference)),
    reason text NOT NULL DEFAULT '',
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1),
    synced_version bigint NOT NULL DEFAULT 0 CHECK (synced_version >= 0 AND synced_version <= version),
    identity_projection_version bigint CHECK (identity_projection_version >= 1),
    identity_admission_version bigint CHECK (identity_admission_version >= 1),
    remote_operation_id uuid CHECK (public.is_uuid_v7(remote_operation_id)),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(application_id,environment),
    CHECK (reviewed_by IS NULL OR (reviewed_by<>requested_by AND reviewed_by<>submitted_by)),
    CHECK (published_by IS NULL OR (published_by<>requested_by AND published_by<>submitted_by)),
    CHECK ((status NOT IN ('TEST_APPROVED','TEST_ACCEPTED') OR environment='test')
        AND (status NOT IN ('PRODUCTION_APPROVED','ENABLED') OR environment='production'))
);
-- Only the deployment authority may populate real production responsibility.
-- Runtime receives SELECT only; a missing/default row keeps production closed.
CREATE TABLE public.platform_auth_release_bindings (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    enabled boolean NOT NULL DEFAULT false,
    reviewer_user_ids uuid[] NOT NULL DEFAULT ARRAY[]::uuid[],
    publisher_user_ids uuid[] NOT NULL DEFAULT ARRAY[]::uuid[],
    policy_reference text NOT NULL DEFAULT '',
    operations_reference text NOT NULL DEFAULT '',
    verified_at timestamptz,
    CHECK (NOT enabled OR (cardinality(reviewer_user_ids)>0 AND cardinality(publisher_user_ids)>0
        AND length(btrim(policy_reference)) BETWEEN 1 AND 500
        AND length(btrim(operations_reference)) BETWEEN 1 AND 500 AND verified_at IS NOT NULL))
);
INSERT INTO public.platform_auth_release_bindings(singleton) VALUES(true);
CREATE TABLE public.platform_auth_review_operations (
    operation_id uuid PRIMARY KEY CHECK (public.is_uuid_v7(operation_id)),
    actor_user_id uuid NOT NULL CHECK (public.is_uuid_v7(actor_user_id)),
    actor_session_id uuid NOT NULL CHECK (public.is_uuid_v7(actor_session_id)),
    request_digest text NOT NULL CHECK (request_digest ~ '^[0-9a-f]{64}$'),
    action text NOT NULL CHECK (length(action) BETWEEN 1 AND 80),
    management_permission text NOT NULL,
    application_id uuid CHECK (public.is_uuid_v7(application_id)),
    request_id uuid REFERENCES public.platform_auth_application_requests(id),
    remote_required boolean NOT NULL DEFAULT false,
    result_json jsonb NOT NULL CHECK (jsonb_typeof(result_json)='object'),
    remote_result jsonb CHECK (remote_result IS NULL OR jsonb_typeof(remote_result)='object'),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    remote_applied_at timestamptz
);

CREATE TABLE public.auth_applications (
    application_id uuid PRIMARY KEY CHECK (public.is_uuid_v7(application_id)),
    partner_id uuid CHECK (public.is_uuid_v7(partner_id)),
    ownership text NOT NULL CHECK (ownership IN ('FIRST_PARTY','THIRD_PARTY')),
    status text NOT NULL CHECK (status IN ('DRAFT','PENDING_REVIEW','APPROVED','SUSPENDED','RETIRED')),
    display_name text NOT NULL CHECK (length(display_name) BETWEEN 1 AND 200),
    purpose text NOT NULL CHECK (length(purpose) BETWEEN 1 AND 2000),
    approved_scopes text[] NOT NULL DEFAULT ARRAY[]::text[],
    projection_version bigint NOT NULL DEFAULT 1 CHECK (projection_version >= 1),
    projected_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK ((ownership = 'THIRD_PARTY') = (partner_id IS NOT NULL))
);

CREATE TABLE public.auth_application_admissions (
    application_id uuid NOT NULL REFERENCES public.auth_applications(application_id),
    environment text NOT NULL CHECK (environment IN ('test','production')),
    status text NOT NULL CHECK (status IN ('APPROVED','SUSPENDED')),
    redirect_uris text[] NOT NULL,
    post_logout_redirect_uris text[] NOT NULL DEFAULT ARRAY[]::text[],
    scopes text[] NOT NULL CHECK (scopes <@ ARRAY['openid','profile']::text[] AND 'openid'=ANY(scopes)),
    requested_by uuid NOT NULL CHECK (public.is_uuid_v7(requested_by)),
    reviewed_by uuid NOT NULL CHECK (public.is_uuid_v7(reviewed_by) AND reviewed_by<>requested_by),
    published_by uuid CHECK (public.is_uuid_v7(published_by) AND published_by<>requested_by),
    contact_reference uuid NOT NULL CHECK (public.is_uuid_v7(contact_reference)),
    domain_reference uuid NOT NULL CHECK (public.is_uuid_v7(domain_reference)),
    review_reference uuid NOT NULL CHECK (public.is_uuid_v7(review_reference)),
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY(application_id,environment),
    CHECK (environment<>'production' OR status<>'APPROVED' OR published_by IS NOT NULL)
);

CREATE TABLE public.auth_resources (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    resource_key text NOT NULL UNIQUE CHECK (resource_key ~ '^[a-z][a-z0-9._-]{0,63}$'),
    audience text NOT NULL UNIQUE CHECK (length(audience) BETWEEN 1 AND 500),
    status text NOT NULL CHECK (status IN ('ACTIVE','SUSPENDED','RETIRED')),
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1)
);

CREATE TABLE public.auth_scopes (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    resource_id uuid NOT NULL REFERENCES public.auth_resources(id),
    scope_name text NOT NULL CHECK (scope_name ~ '^[a-z][a-z0-9._:-]{0,127}$'),
    description text NOT NULL DEFAULT '',
    status text NOT NULL CHECK (status IN ('ACTIVE','SUSPENDED','RETIRED')),
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1),
    UNIQUE (resource_id, scope_name),
    UNIQUE (id, resource_id)
);

CREATE TABLE public.auth_clients (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    client_id text NOT NULL UNIQUE CHECK (length(client_id) BETWEEN 1 AND 100),
    display_name text NOT NULL CHECK (length(btrim(display_name)) BETWEEN 1 AND 120),
    application_id uuid NOT NULL REFERENCES public.auth_applications(application_id),
    environment text NOT NULL CHECK (environment ~ '^[a-z0-9][a-z0-9_-]{0,31}$'),
    client_type text NOT NULL CHECK (client_type IN ('CONFIDENTIAL','PUBLIC')),
    runtime_kind text NOT NULL CHECK (runtime_kind IN ('WEB_BFF','SPA','NATIVE','SERVICE','DEVICE')),
    subject_mode text NOT NULL CHECK (subject_mode IN ('USER','MACHINE')),
    context_mode text CHECK (context_mode IN ('ACCOUNT_ONLY','PLATFORM_CONTEXT','NONE')),
    allowed_member_types text[] NOT NULL DEFAULT ARRAY[]::text[],
    grant_types text[] NOT NULL,
    token_endpoint_auth_method text NOT NULL CHECK (token_endpoint_auth_method IN ('client_secret_basic','none')),
    redirect_uris text[] NOT NULL DEFAULT ARRAY[]::text[],
    post_logout_redirect_uris text[] NOT NULL DEFAULT ARRAY[]::text[],
    status text NOT NULL CHECK (status IN ('ENABLED','DISABLED','RETIRED')),
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1),
    policy_version bigint NOT NULL DEFAULT 1 CHECK (policy_version >= 1),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK (
        (subject_mode = 'MACHINE' AND context_mode = 'NONE' AND cardinality(allowed_member_types) = 0
            AND client_type = 'CONFIDENTIAL' AND grant_types = ARRAY['client_credentials']::text[])
        OR (subject_mode = 'USER' AND context_mode = 'ACCOUNT_ONLY' AND cardinality(allowed_member_types) = 0
            AND grant_types <@ ARRAY['authorization_code','refresh_token']::text[])
        OR (subject_mode = 'USER' AND context_mode = 'PLATFORM_CONTEXT'
            AND cardinality(allowed_member_types) = 1
            AND runtime_kind = 'WEB_BFF' AND client_type = 'CONFIDENTIAL'
            AND allowed_member_types <@ ARRAY['EMPLOYEE','CUSTOMER']::text[]
            AND grant_types <@ ARRAY['authorization_code','refresh_token']::text[])
    ),
    CHECK ((client_type = 'PUBLIC' AND token_endpoint_auth_method = 'none'
                AND NOT ('client_credentials' = ANY(grant_types)))
        OR (client_type = 'CONFIDENTIAL' AND token_endpoint_auth_method = 'client_secret_basic')),
    CHECK (cardinality(grant_types) > 0),
    CHECK (('authorization_code' = ANY(grant_types)) = (cardinality(redirect_uris) > 0)),
    UNIQUE (id, application_id, environment),
    UNIQUE (id, application_id, environment, subject_mode, context_mode),
    UNIQUE (id, client_type)
);

CREATE TABLE public.auth_client_credentials (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    client_id uuid NOT NULL,
    client_type text NOT NULL DEFAULT 'CONFIDENTIAL' CHECK (client_type = 'CONFIDENTIAL'),
    credential_version bigint NOT NULL CHECK (credential_version >= 1),
    secret_digest text CHECK (secret_digest IS NULL OR length(secret_digest) BETWEEN 32 AND 512),
    public_key_thumbprint text CHECK (public_key_thumbprint IS NULL OR length(public_key_thumbprint) BETWEEN 16 AND 256),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    expires_at timestamptz,
    revoked_at timestamptz,
    CHECK ((secret_digest IS NOT NULL) <> (public_key_thumbprint IS NOT NULL)),
    CHECK (expires_at IS NULL OR expires_at > created_at),
    UNIQUE (client_id, credential_version),
    FOREIGN KEY (client_id, client_type) REFERENCES public.auth_clients(id, client_type)
);
CREATE UNIQUE INDEX auth_client_credentials_one_current_idx
    ON public.auth_client_credentials(client_id)
    WHERE revoked_at IS NULL AND expires_at IS NULL;
CREATE INDEX auth_client_credentials_digest_idx ON public.auth_client_credentials(secret_digest)
    WHERE secret_digest IS NOT NULL;

CREATE TABLE public.auth_client_permissions (
    client_id uuid NOT NULL REFERENCES public.auth_clients(id),
    resource_id uuid NOT NULL REFERENCES public.auth_resources(id),
    scope_id uuid NOT NULL,
    expires_at timestamptz,
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1),
    PRIMARY KEY (client_id, resource_id, scope_id),
    FOREIGN KEY (scope_id, resource_id) REFERENCES public.auth_scopes(id, resource_id)
);

CREATE TABLE public.auth_consent_groups (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    application_id uuid NOT NULL REFERENCES public.auth_applications(application_id),
    environment text NOT NULL CHECK (environment ~ '^[a-z0-9][a-z0-9_-]{0,31}$'),
    display_name text NOT NULL CHECK (length(btrim(display_name)) BETWEEN 1 AND 120),
    purpose text NOT NULL CHECK (length(purpose) BETWEEN 1 AND 2000),
    subject_mode text NOT NULL DEFAULT 'USER' CHECK (subject_mode = 'USER'),
    context_mode text NOT NULL CHECK (context_mode IN ('ACCOUNT_ONLY','PLATFORM_CONTEXT')),
    status text NOT NULL CHECK (status IN ('ACTIVE','SUSPENDED','RETIRED')),
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1),
    UNIQUE (id, application_id, environment, subject_mode, context_mode)
);

CREATE TABLE public.auth_consent_group_clients (
    group_id uuid NOT NULL,
    client_id uuid NOT NULL UNIQUE,
    application_id uuid NOT NULL,
    environment text NOT NULL,
    subject_mode text NOT NULL DEFAULT 'USER' CHECK (subject_mode = 'USER'),
    context_mode text NOT NULL CHECK (context_mode IN ('ACCOUNT_ONLY','PLATFORM_CONTEXT')),
    PRIMARY KEY (group_id, client_id),
    FOREIGN KEY (group_id, application_id, environment, subject_mode, context_mode)
        REFERENCES public.auth_consent_groups(id, application_id, environment, subject_mode, context_mode),
    FOREIGN KEY (client_id, application_id, environment, subject_mode, context_mode)
        REFERENCES public.auth_clients(id, application_id, environment, subject_mode, context_mode)
);

CREATE TABLE public.auth_consents (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    user_id uuid NOT NULL,
    group_id uuid NOT NULL REFERENCES public.auth_consent_groups(id),
    status text NOT NULL CHECK (status IN ('ACTIVE','REVOKED')),
    generation bigint NOT NULL DEFAULT 1 CHECK (generation >= 1),
    group_version bigint NOT NULL DEFAULT 1 CHECK (group_version >= 1),
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1),
    authorized_at timestamptz,
    revoked_at timestamptz,
    display_deleted_at timestamptz,
    UNIQUE (user_id, group_id),
    UNIQUE (id, generation),
    CHECK (display_deleted_at IS NULL OR status = 'REVOKED'),
    CHECK ((status = 'ACTIVE' AND authorized_at IS NOT NULL AND revoked_at IS NULL)
        OR (status = 'REVOKED' AND authorized_at IS NOT NULL AND revoked_at IS NOT NULL))
);

CREATE TABLE public.auth_consent_scopes (
    consent_id uuid NOT NULL REFERENCES public.auth_consents(id),
    resource_id uuid NOT NULL REFERENCES public.auth_resources(id),
    scope_id uuid NOT NULL,
    granted_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (consent_id, resource_id, scope_id),
    FOREIGN KEY (scope_id, resource_id) REFERENCES public.auth_scopes(id, resource_id)
);
CREATE INDEX auth_consents_user_status_idx ON public.auth_consents(user_id, status, authorized_at DESC);
CREATE INDEX auth_consent_scopes_consent_idx ON public.auth_consent_scopes(consent_id, resource_id, scope_id);

-- User-visible consent rows can be hidden after revocation, but this event is
-- retained as the bounded-delivery source for Phase 20. It contains no token.
CREATE TABLE public.auth_consent_revocation_outbox (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    consent_id uuid NOT NULL REFERENCES public.auth_consents(id),
    user_id uuid NOT NULL,
    group_id uuid NOT NULL REFERENCES public.auth_consent_groups(id),
    client_id uuid NOT NULL REFERENCES public.auth_clients(id),
    consent_generation bigint NOT NULL CHECK (consent_generation >= 1),
    event_kind text NOT NULL CHECK (event_kind IN ('CONSENT_REVOKED')),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    available_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    delivered_at timestamptz,
    attempt_count integer NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    last_error_code text CHECK (last_error_code IS NULL OR last_error_code ~ '^[A-Z0-9_]{1,64}$'),
    UNIQUE (consent_id, consent_generation, client_id, event_kind)
);

-- Browser-triggered authorization management is idempotent across transport
-- failures. The receipt contains only the result and authority metadata.
CREATE TABLE public.identity_consent_command_receipts (
    operation_id uuid PRIMARY KEY CHECK (public.is_uuid_v7(operation_id)),
    user_id uuid NOT NULL,
    consent_id uuid NOT NULL REFERENCES public.auth_consents(id),
    request_digest text NOT NULL CHECK (request_digest ~ '^[0-9a-f]{64}$'),
    action text NOT NULL CHECK (action IN ('REVOKE','DISPLAY_DELETE')),
    expected_version bigint NOT NULL CHECK (expected_version >= 1),
    result_json jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX auth_consent_revocation_outbox_pending_idx
    ON public.auth_consent_revocation_outbox(available_at, created_at, id)
    WHERE delivered_at IS NULL;

CREATE TABLE public.auth_subject_sectors (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    application_id uuid NOT NULL REFERENCES public.auth_applications(application_id),
    environment text NOT NULL CHECK (environment ~ '^[a-z0-9][a-z0-9_-]{0,31}$'),
    status text NOT NULL CHECK (status IN ('ACTIVE','RETIRED')),
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1),
    UNIQUE (application_id, environment)
);

CREATE TABLE public.auth_subject_links (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    user_id uuid NOT NULL,
    sector_id uuid NOT NULL REFERENCES public.auth_subject_sectors(id),
    subject text NOT NULL CHECK (length(subject) BETWEEN 22 AND 256),
    retired_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (user_id, sector_id),
    UNIQUE (sector_id, subject)
);

CREATE TABLE public.auth_authorization_requests (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    client_id uuid NOT NULL REFERENCES public.auth_clients(id),
    user_id uuid,
    source_session_id uuid,
    account_version bigint CHECK (account_version IS NULL OR account_version >= 1),
    auth_time timestamptz,
    client_policy_version bigint NOT NULL CHECK (client_policy_version >= 1),
    application_projection_version bigint NOT NULL CHECK (application_projection_version >= 1),
    redirect_uri text NOT NULL,
    state_digest text NOT NULL CHECK (length(state_digest) = 64),
    state_value text NOT NULL CHECK (length(state_value) BETWEEN 1 AND 512),
    nonce_digest text CHECK (nonce_digest IS NULL OR length(nonce_digest) = 64),
    nonce_value text NOT NULL CHECK (length(nonce_value) BETWEEN 8 AND 512),
    code_challenge text NOT NULL CHECK (code_challenge ~ '^[A-Za-z0-9_-]{43}$'),
    code_challenge_method text NOT NULL CHECK (code_challenge_method = 'S256'),
    scope_ids uuid[] NOT NULL CHECK (cardinality(scope_ids) BETWEEN 1 AND 16),
    browser_digest text NOT NULL CHECK (browser_digest ~ '^[0-9a-f]{64}$'),
    csrf_digest text NOT NULL CHECK (csrf_digest ~ '^[0-9a-f]{64}$'),
    force_login boolean NOT NULL DEFAULT false,
    force_consent boolean NOT NULL DEFAULT false,
    max_age_seconds bigint CHECK (max_age_seconds IS NULL OR max_age_seconds BETWEEN 0 AND 86400),
    prior_session_id uuid,
    status text NOT NULL CHECK (status IN ('PENDING','AUTHORIZED','DENIED','EXPIRED','CONSUMED')),
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz,
    CHECK (expires_at > created_at),
    CHECK ((source_session_id IS NULL AND account_version IS NULL AND auth_time IS NULL)
        OR (source_session_id IS NOT NULL AND account_version IS NOT NULL AND auth_time IS NOT NULL))
);

CREATE TABLE public.auth_authorization_codes (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    request_id uuid NOT NULL UNIQUE REFERENCES public.auth_authorization_requests(id),
    client_id uuid NOT NULL REFERENCES public.auth_clients(id),
    user_id uuid NOT NULL,
    consent_id uuid NOT NULL REFERENCES public.auth_consents(id),
    consent_generation bigint NOT NULL CHECK (consent_generation >= 1),
    consent_group_version bigint NOT NULL CHECK (consent_group_version >= 1),
    code_digest text NOT NULL UNIQUE CHECK (length(code_digest) = 64),
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK (expires_at > created_at)
);

-- Canonical-client context selection is a one-time, backend-only extension
-- of the unified authorization request.
CREATE TABLE public.auth_context_interactions (
    transaction_id uuid PRIMARY KEY CHECK (public.is_uuid_v7(transaction_id)),
    request_hash text NOT NULL UNIQUE CHECK (request_hash ~ '^[0-9a-f]{64}$'),
    client_id uuid NOT NULL REFERENCES public.auth_clients(id),
    client_policy_version bigint NOT NULL CHECK (client_policy_version >= 1),
    application_id uuid NOT NULL REFERENCES public.auth_applications(application_id),
    application_projection_version bigint NOT NULL CHECK (application_projection_version >= 1),
    redirect_uri text NOT NULL,
    state_digest text NOT NULL CHECK (state_digest ~ '^[0-9a-f]{64}$'),
    nonce_digest text NOT NULL CHECK (nonce_digest ~ '^[0-9a-f]{64}$'),
    code_challenge text NOT NULL CHECK (code_challenge ~ '^[A-Za-z0-9_-]{43}$'),
    intent jsonb NOT NULL CHECK (jsonb_typeof(intent)='object'),
    authorization_request_id uuid UNIQUE REFERENCES public.auth_authorization_requests(id) ON DELETE CASCADE,
    source_digest text UNIQUE CHECK (source_digest IS NULL OR source_digest ~ '^[0-9a-f]{64}$'),
    selected_membership_id uuid CHECK (selected_membership_id IS NULL OR public.is_uuid_v7(selected_membership_id)),
    credential_ciphertext text,
    selection_expires_at timestamptz,
    grant_id uuid UNIQUE,
    delivered_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    expires_at timestamptz NOT NULL CHECK (expires_at > created_at),
    CHECK (source_digest IS NULL OR authorization_request_id IS NOT NULL),
    CHECK ((selected_membership_id IS NULL) = (selection_expires_at IS NULL)),
    CHECK (selected_membership_id IS NULL OR
        (source_digest IS NOT NULL AND selection_expires_at <= expires_at)),
    CHECK (grant_id IS NULL OR selected_membership_id IS NOT NULL),
    CHECK (delivered_at IS NULL OR grant_id IS NOT NULL),
    CHECK ((credential_ciphertext IS NOT NULL) =
        (selected_membership_id IS NOT NULL AND delivered_at IS NULL)),
    CHECK (delivered_at IS NULL OR selection_expires_at IS NOT NULL)
);
CREATE INDEX auth_context_interactions_expiration_idx
    ON public.auth_context_interactions(expires_at);

CREATE FUNCTION public.auth_context_interaction_binding_immutable() RETURNS trigger
LANGUAGE plpgsql SET search_path TO 'pg_catalog','public' AS $$
BEGIN
    IF (NEW.transaction_id,NEW.request_hash,NEW.client_id,NEW.client_policy_version,
        NEW.application_id,NEW.application_projection_version,NEW.redirect_uri,
        NEW.state_digest,NEW.nonce_digest,NEW.code_challenge,NEW.intent,NEW.created_at)
       IS DISTINCT FROM
       (OLD.transaction_id,OLD.request_hash,OLD.client_id,OLD.client_policy_version,
        OLD.application_id,OLD.application_projection_version,OLD.redirect_uri,
        OLD.state_digest,OLD.nonce_digest,OLD.code_challenge,OLD.intent,OLD.created_at)
       OR NEW.expires_at > OLD.expires_at
       OR (OLD.selected_membership_id IS NOT NULL AND
           (NEW.selected_membership_id,NEW.selection_expires_at) IS DISTINCT FROM
           (OLD.selected_membership_id,OLD.selection_expires_at))
       OR (OLD.grant_id IS NOT NULL AND NEW.grant_id IS DISTINCT FROM OLD.grant_id)
       OR (OLD.delivered_at IS NOT NULL AND NEW.delivered_at IS DISTINCT FROM OLD.delivered_at) THEN
        RAISE EXCEPTION 'AUTH_CONTEXT_INTERACTION_BINDING_IMMUTABLE';
    END IF;
    IF NEW.authorization_request_id IS DISTINCT FROM OLD.authorization_request_id
       AND OLD.authorization_request_id IS NOT NULL
       AND (OLD.selected_membership_id IS NOT NULL OR NEW.source_digest IS NOT NULL) THEN
        RAISE EXCEPTION 'AUTH_CONTEXT_INTERACTION_FLOW_IMMUTABLE';
    END IF;
    IF OLD.source_digest IS NOT NULL AND NEW.source_digest IS DISTINCT FROM OLD.source_digest
       AND NOT (NEW.source_digest IS NULL
           AND NEW.authorization_request_id IS DISTINCT FROM OLD.authorization_request_id
           AND OLD.selected_membership_id IS NULL) THEN
        RAISE EXCEPTION 'AUTH_CONTEXT_INTERACTION_SOURCE_IMMUTABLE';
    END IF;
    IF OLD.credential_ciphertext IS NOT NULL AND NEW.credential_ciphertext IS DISTINCT FROM OLD.credential_ciphertext
       AND NOT (NEW.credential_ciphertext IS NULL AND NEW.delivered_at IS NOT NULL) THEN
        RAISE EXCEPTION 'AUTH_CONTEXT_INTERACTION_CREDENTIAL_IMMUTABLE';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER auth_context_interaction_binding_immutable
BEFORE UPDATE ON public.auth_context_interactions
FOR EACH ROW EXECUTE FUNCTION public.auth_context_interaction_binding_immutable();

CREATE TABLE public.auth_grants (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    client_id uuid NOT NULL REFERENCES public.auth_clients(id),
    subject_mode text NOT NULL CHECK (subject_mode IN ('USER','MACHINE')),
    user_id uuid,
    consent_id uuid REFERENCES public.auth_consents(id),
    consent_generation bigint CHECK (consent_generation IS NULL OR consent_generation >= 1),
    consent_group_version bigint CHECK (consent_group_version IS NULL OR consent_group_version >= 1),
    source_session_id uuid,
    context_mode text NOT NULL CHECK (context_mode IN ('ACCOUNT_ONLY','PLATFORM_CONTEXT','NONE')),
    context_ref uuid CHECK (public.is_uuid_v7(context_ref)),
    account_version bigint CHECK (account_version IS NULL OR account_version >= 0),
    client_policy_version bigint NOT NULL CHECK (client_policy_version >= 1),
    application_projection_version bigint NOT NULL CHECK (application_projection_version >= 1),
    credential_version bigint CHECK (credential_version IS NULL OR credential_version >= 1),
    scope_ids uuid[] NOT NULL,
    audiences text[] NOT NULL,
    auth_time timestamptz,
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1),
    CHECK ((subject_mode = 'USER' AND user_id IS NOT NULL AND consent_id IS NOT NULL
            AND consent_generation IS NOT NULL AND consent_group_version IS NOT NULL AND context_mode <> 'NONE'
            AND credential_version IS NULL)
        OR (subject_mode = 'MACHINE' AND user_id IS NULL AND consent_id IS NULL AND consent_generation IS NULL
            AND consent_group_version IS NULL AND source_session_id IS NULL AND context_mode = 'NONE' AND context_ref IS NULL
            AND account_version IS NULL AND auth_time IS NULL AND credential_version IS NOT NULL)),
    CHECK ((context_mode = 'PLATFORM_CONTEXT') = (context_ref IS NOT NULL)),
    CHECK (expires_at > created_at),
    FOREIGN KEY (client_id, credential_version)
        REFERENCES public.auth_client_credentials(client_id, credential_version),
    FOREIGN KEY (context_ref) REFERENCES public.auth_context_interactions(transaction_id),
    UNIQUE (id, context_ref)
);
CREATE UNIQUE INDEX auth_grants_context_ref_unique_idx
    ON public.auth_grants(context_ref) WHERE context_ref IS NOT NULL;
ALTER TABLE public.auth_context_interactions
    ADD CONSTRAINT auth_context_interactions_grant_fk
        FOREIGN KEY (grant_id, transaction_id) REFERENCES public.auth_grants(id, context_ref);

CREATE TABLE public.auth_grant_scopes (
    grant_id uuid NOT NULL REFERENCES public.auth_grants(id),
    resource_id uuid NOT NULL REFERENCES public.auth_resources(id),
    scope_id uuid NOT NULL,
    PRIMARY KEY (grant_id, resource_id, scope_id),
    FOREIGN KEY (scope_id, resource_id) REFERENCES public.auth_scopes(id, resource_id)
);

CREATE TABLE public.auth_access_tokens (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    grant_id uuid NOT NULL REFERENCES public.auth_grants(id),
    token_digest text NOT NULL UNIQUE CHECK (length(token_digest) = 64),
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK (expires_at > created_at)
);

CREATE TABLE public.auth_access_token_scopes (
    token_id uuid NOT NULL REFERENCES public.auth_access_tokens(id),
    resource_id uuid NOT NULL REFERENCES public.auth_resources(id),
    scope_id uuid NOT NULL,
    PRIMARY KEY (token_id, resource_id, scope_id),
    FOREIGN KEY (scope_id, resource_id) REFERENCES public.auth_scopes(id, resource_id)
);

CREATE TABLE public.auth_refresh_families (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    grant_id uuid NOT NULL REFERENCES public.auth_grants(id),
    absolute_expires_at timestamptz NOT NULL,
    idle_expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1),
    CHECK (idle_expires_at <= absolute_expires_at),
    CHECK (absolute_expires_at > created_at AND idle_expires_at > created_at)
);
CREATE INDEX auth_refresh_families_grant_idx ON public.auth_refresh_families(grant_id);

CREATE TABLE public.auth_refresh_tokens (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    family_id uuid NOT NULL REFERENCES public.auth_refresh_families(id),
    token_digest text NOT NULL UNIQUE CHECK (length(token_digest) = 64),
    generation bigint NOT NULL CHECK (generation >= 1),
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz,
    revoked_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (family_id, generation),
    CHECK (expires_at > created_at)
);

-- TOTP factors are encrypted by the identity process with a dedicated key.
-- Pending enrollment material expires and is never returned after creation.
CREATE TABLE public.auth_totp_enrollments (
    user_id uuid PRIMARY KEY,
    secret_ciphertext bytea NOT NULL CHECK (octet_length(secret_ciphertext) BETWEEN 28 AND 128),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    expires_at timestamptz NOT NULL,
    CHECK (expires_at > created_at)
);
CREATE TABLE public.auth_totp_factors (
    user_id uuid PRIMARY KEY,
    secret_ciphertext bytea NOT NULL CHECK (octet_length(secret_ciphertext) BETWEEN 28 AND 128),
    enabled_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    last_accepted_step bigint NOT NULL DEFAULT 0 CHECK (last_accepted_step >= 0),
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1)
);
CREATE TABLE public.auth_totp_recovery_codes (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    user_id uuid NOT NULL,
    code_digest text NOT NULL CHECK (code_digest ~ '^[0-9a-f]{64}$'),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    consumed_at timestamptz,
    UNIQUE (user_id, code_digest)
);
CREATE INDEX auth_totp_recovery_codes_available_idx
    ON public.auth_totp_recovery_codes(user_id, created_at, id) WHERE consumed_at IS NULL;
CREATE TABLE public.auth_mfa_recovery_receipts (
    operation_id uuid PRIMARY KEY CHECK (public.is_uuid_v7(operation_id)),
    target_user_id uuid NOT NULL,
    request_digest text NOT NULL CHECK (request_digest ~ '^[0-9a-f]{64}$'),
    result_json jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE public.auth_mfa_recovery_approvals (
    operation_id uuid NOT NULL REFERENCES public.auth_mfa_recovery_receipts(operation_id),
    approver_user_id uuid NOT NULL,
    primary_authenticated_at timestamptz NOT NULL,
    mfa_verified_at timestamptz NOT NULL,
    approval_digest text NOT NULL CHECK (approval_digest ~ '^[0-9a-f]{64}$'),
    PRIMARY KEY (operation_id, approver_user_id)
);
CREATE TABLE public.auth_security_notification_outbox (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    user_id uuid NOT NULL,
    event_kind text NOT NULL CHECK (event_kind IN ('MFA_ENABLED','MFA_DISABLED','MFA_RECOVERED')),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    available_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    delivered_at timestamptz,
    attempt_count integer NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    last_error_code text CHECK (last_error_code IS NULL OR last_error_code ~ '^[A-Z0-9_]{1,64}$')
);
CREATE INDEX auth_security_notification_outbox_pending_idx
    ON public.auth_security_notification_outbox(available_at, created_at, id)
    WHERE delivered_at IS NULL;

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

ALTER TABLE ONLY public.identity_auth_command_receipts
    ADD CONSTRAINT identity_auth_command_receipts_pkey PRIMARY KEY (operation_id);

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

ALTER TABLE ONLY public.auth_provider_settings
    ADD CONSTRAINT auth_provider_settings_pkey PRIMARY KEY (singleton);

ALTER TABLE ONLY public.auth_signing_keys
    ADD CONSTRAINT auth_signing_keys_pkey PRIMARY KEY (kid);

ALTER TABLE ONLY public.organization_applications
    ADD CONSTRAINT organization_applications_pkey PRIMARY KEY (organization_id, application_id);

ALTER TABLE ONLY public.organization_menus
    ADD CONSTRAINT organization_menus_pkey PRIMARY KEY (organization_id, menu_id);

ALTER TABLE ONLY public.organizations
    ADD CONSTRAINT organizations_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.platform_account_command_intents
    ADD CONSTRAINT platform_account_command_intents_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.platform_auth_command_intents
    ADD CONSTRAINT platform_auth_command_intents_pkey PRIMARY KEY (id);

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

CREATE UNIQUE INDEX idx_auth_one_active_key ON public.auth_signing_keys USING btree ((true)) WHERE active;

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

CREATE INDEX idx_user_roles_org ON public.user_roles USING btree (organization_id);

CREATE INDEX idx_user_roles_role ON public.user_roles USING btree (role_id);

CREATE INDEX idx_users_login_email ON public.users USING btree (lower((email)::text)) WHERE ((email)::text <> ''::text);

CREATE INDEX idx_users_login_phone ON public.users USING btree (regexp_replace((phone)::text, '[[:space:]()\-]'::text, ''::text, 'g'::text)) WHERE ((phone)::text <> ''::text);

CREATE UNIQUE INDEX managed_deployment_one_worker ON public.managed_deployment_jobs USING btree (artifact_id) WHERE (state = ANY (ARRAY['QUEUED'::text, 'RUNNING'::text]));

CREATE INDEX managed_deployment_queue ON public.managed_deployment_jobs USING btree (state, created_at);

CREATE INDEX managed_deployment_releases ON public.managed_deployment_jobs USING btree (release_id);

CREATE UNIQUE INDEX uniq_tenants_platform ON public.tenants USING btree (is_platform) WHERE is_platform;

CREATE UNIQUE INDEX uq_departments_id_organization ON public.departments USING btree (id, organization_id);

CREATE UNIQUE INDEX uq_org_root_per_tenant ON public.organizations USING btree (tenant_id) WHERE (parent_id IS NULL);

CREATE UNIQUE INDEX uq_organizations_id_tenant ON public.organizations USING btree (id, tenant_id);

CREATE INDEX zip_distribution_application ON public.zip_distribution_jobs USING btree (application_id, created_at);

CREATE INDEX zip_distribution_public_root ON public.zip_distribution_jobs USING btree (public_root);

CREATE INDEX zip_distribution_queue ON public.zip_distribution_jobs USING btree (state, created_at);

CREATE TRIGGER application_release_artifact_immutable BEFORE UPDATE ON public.application_release_artifacts FOR EACH ROW EXECUTE FUNCTION public.protect_application_release_artifact();

CREATE TRIGGER application_release_definition_immutable BEFORE UPDATE ON public.application_releases FOR EACH ROW EXECUTE FUNCTION public.protect_application_release_definition();

CREATE TRIGGER trg_account_credential_generation AFTER UPDATE ON public.users FOR EACH ROW EXECUTE FUNCTION public.revoke_credentials_on_account_generation();

CREATE TRIGGER trg_account_sso_generation AFTER UPDATE ON public.users FOR EACH ROW EXECUTE FUNCTION public.revoke_sso_on_account_generation();

CREATE TRIGGER trg_application_identity_policy BEFORE UPDATE ON public.applications FOR EACH ROW EXECUTE FUNCTION public.protect_application_identity_policy();

CREATE TRIGGER trg_departments_no_cycle BEFORE INSERT OR UPDATE OF parent_id ON public.departments FOR EACH ROW EXECUTE FUNCTION public.reject_department_cycle();

CREATE TRIGGER trg_membership_credential_generation AFTER UPDATE ON public.memberships FOR EACH ROW EXECUTE FUNCTION public.revoke_credentials_on_identity_update();

CREATE TRIGGER trg_membership_identity_immutable BEFORE UPDATE ON public.memberships FOR EACH ROW EXECUTE FUNCTION public.protect_membership_identity();

CREATE TRIGGER trg_membership_session_revoke AFTER DELETE ON public.memberships FOR EACH ROW EXECUTE FUNCTION public.revoke_sessions_on_membership_delete();

CREATE TRIGGER trg_notification_member_pref_identity BEFORE UPDATE ON public.notification_member_prefs FOR EACH ROW EXECUTE FUNCTION public.protect_notification_member_pref_identity();

CREATE TRIGGER trg_notification_publication_immutable BEFORE UPDATE ON public.notification_publications FOR EACH ROW EXECUTE FUNCTION public.reject_notification_publication_update();

CREATE TRIGGER trg_notification_recipient_immutable BEFORE UPDATE ON public.notifications FOR EACH ROW EXECUTE FUNCTION public.protect_notification_recipient();

CREATE TRIGGER trg_auth_key_material BEFORE UPDATE ON public.auth_signing_keys FOR EACH ROW EXECUTE FUNCTION public.auth_key_material_immutable();

CREATE TRIGGER trg_organization_session_state AFTER UPDATE OF status, expires_at ON public.organizations FOR EACH ROW EXECUTE FUNCTION public.sync_sessions_on_organization_state();

CREATE TRIGGER trg_organizations_no_cycle BEFORE INSERT OR UPDATE OF parent_id ON public.organizations FOR EACH ROW EXECUTE FUNCTION public.reject_organization_cycle();

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

ALTER TABLE ONLY public.identity_auth_command_receipts
    ADD CONSTRAINT identity_auth_command_receipts_service_id_fkey FOREIGN KEY (service_id) REFERENCES public.identity_service_clients(id);

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

REVOKE ALL ON FUNCTION public.bff_auth_proof_active(subject_ref uuid, sid_ref uuid, grant_ref uuid, account_generation bigint, client_ref uuid, policy_generation bigint, expected_issuer text) FROM PUBLIC;
REVOKE ALL ON FUNCTION public.auth_user_grant_source_active(subject_ref uuid, sid_ref uuid, grant_ref uuid, account_generation bigint, client_ref uuid, policy_generation bigint, expected_issuer text, expected_purpose text, expected_member_type text) FROM PUBLIC;
REVOKE ALL ON FUNCTION public.bff_context_member_matches(grant_ref uuid, member_ref uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION public.identity_credential_digest_in_use(digest_ref text) FROM PUBLIC;
REVOKE ALL ON FUNCTION public.identity_context_source_service_active(service_ref uuid, expected_digest text, client_ref uuid) FROM PUBLIC;

REVOKE ALL ON FUNCTION public.customer_portal_account_source_active(access_digest text, expected_issuer text, client_ref uuid, grant_ref uuid, sid_ref uuid, subject_ref uuid, account_generation bigint, policy_generation bigint, authentication_time bigint, source_deadline timestamp with time zone) FROM PUBLIC;

REVOKE ALL ON FUNCTION public.identity_lock_auth_source(p_client_id text, p_issuer text) FROM PUBLIC;

REVOKE ALL ON FUNCTION public.identity_auth_command_intent_active(operation_ref uuid, service_ref uuid, digest_ref text, method_ref text, path_ref text) FROM PUBLIC;

REVOKE ALL ON FUNCTION public.auth_console_source_active(expected_issuer text, client_ref text, policy_version bigint, member_type text) FROM PUBLIC;

REVOKE ALL ON FUNCTION public.platform_account_command_intent_active(operation_ref uuid, target_ref uuid, account_generation bigint, digest_ref text, kind_ref text) FROM PUBLIC;

REVOKE ALL ON FUNCTION public.platform_password_context_active(subject_ref uuid, session_ref uuid, tenant_ref uuid, organization_ref uuid, account_generation bigint) FROM PUBLIC;

REVOKE ALL ON FUNCTION public.revoke_credentials_on_account_generation() FROM PUBLIC;

REVOKE ALL ON FUNCTION public.revoke_sessions_on_user_disable() FROM PUBLIC;

SET check_function_bodies = true;
SELECT pg_catalog.set_config('search_path','public,pg_catalog,pg_temp',false);

-- Required public platform branding; no accounts, sessions or business fixtures.
INSERT INTO public.platform_settings(singleton) VALUES(true);

-- Platform-owned selection credentials are short-lived; consumed source facts
-- remain durable. Identity runtime accesses this capability only through HTTP.
CREATE TABLE public.context_selections (
    transaction_id uuid PRIMARY KEY CHECK (public.is_uuid_v7(transaction_id)),
    interaction_id uuid NOT NULL UNIQUE CHECK (public.is_uuid_v7(interaction_id)),
    client_id uuid NOT NULL CHECK (public.is_uuid_v7(client_id)),
    consumer_service_id uuid CHECK (public.is_uuid_v7(consumer_service_id)),
    credential_hash text NOT NULL UNIQUE CHECK (credential_hash ~ '^[0-9a-f]{64}$'),
    credential_ciphertext text,
    selection jsonb NOT NULL CHECK (jsonb_typeof(selection)='object'),
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz,
    result jsonb,
    CHECK (expires_at>created_at AND expires_at<=created_at+interval '120 seconds'),
    CHECK ((consumed_at IS NULL)=(result IS NULL)),
    CHECK ((consumed_at IS NULL)=(credential_ciphertext IS NOT NULL)),
    CHECK (consumed_at IS NULL OR (consumed_at>=created_at AND consumed_at<expires_at)),
    CHECK (result IS NULL OR jsonb_typeof(result)='object'),
    CHECK ((selection->'binding'->>'transactionId') IS NOT NULL
        AND selection->'binding'->>'transactionId'=transaction_id::text),
    CHECK ((selection->'binding'->'source'->>'interactionId') IS NOT NULL
        AND selection->'binding'->'source'->>'interactionId'=interaction_id::text),
    CHECK ((selection->'binding'->'registration'->>'clientId') IS NOT NULL
        AND selection->'binding'->'registration'->>'clientId'=client_id::text)
);
CREATE INDEX context_selections_expiration ON public.context_selections(expires_at);
ALTER TABLE public.context_selections ADD COLUMN result_ciphertext text;
CREATE UNIQUE INDEX context_selections_switch_once ON public.context_selections(client_id,(selection#>>'{binding,intent,sessionId}'),(selection#>>'{binding,intent,expectedVersion}'))
    WHERE consumed_at IS NOT NULL AND selection#>>'{binding,intent,kind}'='SWITCH';
ALTER TABLE public.customer_portal_contexts ADD COLUMN bff_session_id uuid CHECK (bff_session_id IS NULL OR public.is_uuid_v7(bff_session_id));
ALTER TABLE public.customer_portal_contexts ADD COLUMN bff_context_version bigint CHECK (bff_context_version>=1);

CREATE FUNCTION public.context_selection_binding_immutable() RETURNS trigger
LANGUAGE plpgsql SET search_path TO 'pg_catalog','public' AS $$
BEGIN
    IF (NEW.transaction_id,NEW.interaction_id,NEW.client_id,NEW.consumer_service_id,
        NEW.credential_hash,NEW.selection,NEW.created_at,NEW.expires_at)
       IS DISTINCT FROM
       (OLD.transaction_id,OLD.interaction_id,OLD.client_id,OLD.consumer_service_id,
        OLD.credential_hash,OLD.selection,OLD.created_at,OLD.expires_at)
       OR (OLD.consumed_at IS NOT NULL AND
           (NEW.consumed_at,NEW.result,NEW.result_ciphertext) IS DISTINCT FROM (OLD.consumed_at,OLD.result,OLD.result_ciphertext)) THEN
        RAISE EXCEPTION 'CONTEXT_SELECTION_BINDING_IMMUTABLE';
    END IF;
    IF OLD.credential_ciphertext IS DISTINCT FROM NEW.credential_ciphertext
       AND NOT (OLD.consumed_at IS NULL AND NEW.consumed_at IS NOT NULL
           AND NEW.credential_ciphertext IS NULL) THEN
        RAISE EXCEPTION 'CONTEXT_SELECTION_CREDENTIAL_IMMUTABLE';
    END IF;
    IF OLD.consumed_at IS NOT NULL AND NEW.credential_ciphertext IS NOT NULL THEN
        RAISE EXCEPTION 'CONTEXT_SELECTION_CREDENTIAL_ALREADY_CONSUMED';
    END IF;
    IF OLD.resource_revoked_at IS NOT NULL AND NEW.resource_revoked_at IS DISTINCT FROM OLD.resource_revoked_at THEN
        RAISE EXCEPTION 'CONTEXT_SELECTION_RESOURCE_REVOCATION_IMMUTABLE';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER context_selection_binding_immutable
BEFORE UPDATE ON public.context_selections
FOR EACH ROW EXECUTE FUNCTION public.context_selection_binding_immutable();
ALTER TABLE public.identity_service_clients ADD COLUMN allowed_auth_clients uuid[] NOT NULL DEFAULT '{}';

CREATE TABLE public.context_selection_services (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    secret_hash text NOT NULL UNIQUE CHECK (secret_hash ~ '^[0-9a-f]{64}$'),
    purpose text NOT NULL CHECK (purpose IN ('CONTEXT_SELECTION_AUTHORITY','CONTEXT_SELECTION_CONSUMER')),
    allowed_auth_clients uuid[] NOT NULL DEFAULT '{}',
    status text NOT NULL CHECK (status IN ('ENABLED','DISABLED')),
    CHECK (cardinality(allowed_auth_clients)>0)
);
CREATE TABLE public.context_selection_registrations (
    client_id uuid PRIMARY KEY CHECK (public.is_uuid_v7(client_id)),
    client_registry text NOT NULL DEFAULT 'AUTH' CHECK (client_registry='AUTH'),
    client_policy_version bigint NOT NULL CHECK (client_policy_version>=1),
    member_type text NOT NULL CHECK (member_type IN ('EMPLOYEE','CUSTOMER')),
    application_id uuid NOT NULL REFERENCES public.applications(id),
    application_policy_version bigint NOT NULL CHECK (application_policy_version>=1),
    consumer_service_id uuid REFERENCES public.context_selection_services(id),
    enabled boolean NOT NULL DEFAULT true,
    CHECK ((application_id IS NULL)=(application_policy_version IS NULL)),
    CHECK (application_id IS NOT NULL),
    CHECK (member_type<>'CUSTOMER' OR (application_id IS NOT NULL AND consumer_service_id IS NOT NULL))
);
ALTER TABLE public.auth_consents ADD CONSTRAINT auth_consents_user_fk FOREIGN KEY(user_id) REFERENCES public.users(id);
ALTER TABLE public.auth_consent_revocation_outbox ADD CONSTRAINT auth_consent_revocation_outbox_user_fk FOREIGN KEY(user_id) REFERENCES public.users(id);
ALTER TABLE public.identity_consent_command_receipts ADD CONSTRAINT identity_consent_command_receipts_user_fk FOREIGN KEY(user_id) REFERENCES public.users(id);
ALTER TABLE public.auth_subject_links ADD CONSTRAINT auth_subject_links_user_fk FOREIGN KEY(user_id) REFERENCES public.users(id);
ALTER TABLE public.auth_authorization_requests ADD CONSTRAINT auth_authorization_requests_user_fk FOREIGN KEY(user_id) REFERENCES public.users(id);
ALTER TABLE public.auth_authorization_requests ADD CONSTRAINT auth_authorization_requests_source_session_fk FOREIGN KEY(source_session_id) REFERENCES public.sso_sessions(id);
ALTER TABLE public.auth_authorization_requests ADD CONSTRAINT auth_authorization_requests_prior_session_fk FOREIGN KEY(prior_session_id) REFERENCES public.sso_sessions(id);
ALTER TABLE public.auth_authorization_codes ADD CONSTRAINT auth_authorization_codes_user_fk FOREIGN KEY(user_id) REFERENCES public.users(id);
ALTER TABLE public.auth_grants ADD CONSTRAINT auth_grants_user_fk FOREIGN KEY(user_id) REFERENCES public.users(id);
ALTER TABLE public.auth_grants ADD CONSTRAINT auth_grants_source_session_fk FOREIGN KEY(source_session_id) REFERENCES public.sso_sessions(id);
ALTER TABLE public.auth_totp_enrollments ADD CONSTRAINT auth_totp_enrollments_user_fk FOREIGN KEY(user_id) REFERENCES public.users(id);
ALTER TABLE public.auth_totp_factors ADD CONSTRAINT auth_totp_factors_user_fk FOREIGN KEY(user_id) REFERENCES public.users(id);
ALTER TABLE public.auth_totp_recovery_codes ADD CONSTRAINT auth_totp_recovery_codes_user_fk FOREIGN KEY(user_id) REFERENCES public.users(id);
ALTER TABLE public.auth_mfa_recovery_receipts ADD CONSTRAINT auth_mfa_recovery_receipts_target_user_fk FOREIGN KEY(target_user_id) REFERENCES public.users(id);
ALTER TABLE public.auth_mfa_recovery_approvals ADD CONSTRAINT auth_mfa_recovery_approvals_approver_user_fk FOREIGN KEY(approver_user_id) REFERENCES public.users(id);
ALTER TABLE public.auth_security_notification_outbox ADD CONSTRAINT auth_security_notification_outbox_user_fk FOREIGN KEY(user_id) REFERENCES public.users(id);

-- Account invitations are identity-owned claims, never public registration.
CREATE TABLE public.auth_invitations (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    service_id uuid NOT NULL REFERENCES public.identity_service_clients(id),
    operation_id uuid NOT NULL UNIQUE CHECK (public.is_uuid_v7(operation_id)),
    request_digest text NOT NULL CHECK (request_digest ~ '^[0-9a-f]{64}$'),
    recipient_ref text NOT NULL CHECK (octet_length(recipient_ref) BETWEEN 1 AND 256 AND recipient_ref !~ '[[:cntrl:]]'),
    purpose text NOT NULL CHECK (purpose='ACCOUNT_ONBOARDING'),
    actor_user_id uuid NOT NULL CHECK (public.is_uuid_v7(actor_user_id)),
    actor_session_id uuid NOT NULL CHECK (public.is_uuid_v7(actor_session_id)),
    key_version bigint NOT NULL CHECK (key_version>0),
    token_digest text NOT NULL UNIQUE CHECK (token_digest ~ '^[0-9a-f]{64}$'),
    status text NOT NULL CHECK (status IN ('ACTIVE','REVOKED','CONSUMED')),
    version bigint NOT NULL CHECK (version>0),
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL CHECK (expires_at=created_at+interval '24 hours'),
    revoked_at timestamptz,
    consumed_at timestamptz,
    consumed_by_service_id uuid REFERENCES public.identity_service_clients(id),
    verification_ref uuid UNIQUE CHECK (verification_ref IS NULL OR public.is_uuid_v7(verification_ref)),
    activated_account_id uuid REFERENCES public.users(id),
    activation_operation_id uuid UNIQUE CHECK (activation_operation_id IS NULL OR public.is_uuid_v7(activation_operation_id)),
    CHECK (id=operation_id),
    CHECK ((status='ACTIVE' AND revoked_at IS NULL AND consumed_at IS NULL AND consumed_by_service_id IS NULL AND verification_ref IS NULL AND activated_account_id IS NULL AND activation_operation_id IS NULL)
        OR (status='REVOKED' AND revoked_at IS NOT NULL AND revoked_at>=created_at AND consumed_at IS NULL AND consumed_by_service_id IS NULL AND verification_ref IS NULL AND activated_account_id IS NULL AND activation_operation_id IS NULL)
        OR (status='CONSUMED' AND revoked_at IS NULL AND consumed_at IS NOT NULL AND consumed_at>=created_at AND consumed_by_service_id IS NULL AND verification_ref IS NOT NULL AND activated_account_id IS NOT NULL AND activation_operation_id IS NOT NULL))
);
CREATE INDEX auth_invitations_created_idx ON public.auth_invitations(created_at DESC,id DESC);

ALTER TABLE public.platform_auth_partners ADD CONSTRAINT platform_auth_partners_creator_fk FOREIGN KEY(created_by) REFERENCES public.users(id);
ALTER TABLE public.platform_auth_applications ADD CONSTRAINT platform_auth_applications_creator_fk FOREIGN KEY(created_by) REFERENCES public.users(id);
ALTER TABLE public.platform_auth_application_operators ADD CONSTRAINT platform_auth_application_operators_user_fk FOREIGN KEY(user_id) REFERENCES public.users(id);
ALTER TABLE public.platform_auth_application_operators ADD CONSTRAINT platform_auth_application_operators_assigner_fk FOREIGN KEY(assigned_by) REFERENCES public.users(id);
ALTER TABLE public.platform_auth_application_requests ADD CONSTRAINT platform_auth_application_requests_requester_fk FOREIGN KEY(requested_by) REFERENCES public.users(id);
ALTER TABLE public.platform_auth_application_requests ADD CONSTRAINT platform_auth_application_requests_submitter_fk FOREIGN KEY(submitted_by) REFERENCES public.users(id);
ALTER TABLE public.platform_auth_application_requests ADD CONSTRAINT platform_auth_application_requests_reviewer_fk FOREIGN KEY(reviewed_by) REFERENCES public.users(id);
ALTER TABLE public.platform_auth_application_requests ADD CONSTRAINT platform_auth_application_requests_publisher_fk FOREIGN KEY(published_by) REFERENCES public.users(id);
ALTER TABLE public.platform_auth_review_operations ADD CONSTRAINT platform_auth_review_operations_actor_fk FOREIGN KEY(actor_user_id) REFERENCES public.users(id);

CREATE FUNCTION public.identity_auth_review_intent_active(operation_ref uuid,service_ref uuid,digest_ref text,method_ref text,path_ref text,body_ref jsonb)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path TO 'pg_catalog','public' AS $$
DECLARE requested record; expected jsonb; actor_ref uuid; session_ref uuid;
BEGIN
    IF method_ref<>'PUT' OR NOT public.identity_auth_command_intent_active(operation_ref,service_ref,digest_ref,method_ref,path_ref) THEN RETURN false; END IF;
    actor_ref:=(body_ref->>'actorUserId')::uuid; session_ref:=(body_ref->>'actorSessionId')::uuid;
    SELECT request.*,app.partner_id,app.created_by AS application_creator INTO requested
      FROM public.platform_auth_application_requests request JOIN public.platform_auth_applications app USING(application_id)
      JOIN public.platform_auth_partners partner ON partner.id=app.partner_id AND partner.status='ACTIVE'
      WHERE request.id=(body_ref->>'requestId')::uuid AND request.version=(body_ref->>'requestVersion')::bigint
        AND request.remote_operation_id=operation_ref AND request.status IN ('TEST_APPROVED','TEST_ACCEPTED','ENABLED','SUSPENDED')
        AND path_ref='/internal/identity/auth/review/applications/'||request.application_id::text||'/'||request.environment
      FOR SHARE OF request;
    IF NOT FOUND OR requested.reviewed_by IS NULL OR requested.reviewed_contact_reference IS NULL OR requested.reviewed_domain_reference IS NULL THEN RETURN false; END IF;
    IF NOT EXISTS(SELECT 1 FROM public.platform_auth_command_intents intent WHERE intent.id=operation_ref
        AND intent.actor_user_id=actor_ref AND intent.session_id=session_ref
        AND intent.management_permission=CASE WHEN requested.status='SUSPENDED' THEN 'system:oauth:disable'
            WHEN requested.environment='production' THEN 'system:oauth:publish' ELSE 'system:oauth:review' END) THEN RETURN false; END IF;
    IF requested.status<>'SUSPENDED' AND (actor_ref IN(requested.requested_by,requested.submitted_by,requested.application_creator)
        OR requested.reviewed_by IN(requested.requested_by,requested.submitted_by,requested.application_creator)) THEN RETURN false; END IF;
    IF requested.environment='production' AND requested.status<>'SUSPENDED' THEN
        IF requested.status<>'ENABLED' OR requested.published_by IS DISTINCT FROM actor_ref
          OR NOT EXISTS(SELECT 1 FROM public.platform_auth_release_bindings binding WHERE binding.singleton AND binding.enabled
            AND binding.verified_at<=clock_timestamp() AND requested.reviewed_by=ANY(binding.reviewer_user_ids)
            AND actor_ref=ANY(binding.publisher_user_ids)
            AND NOT EXISTS(SELECT 1 FROM unnest(binding.reviewer_user_ids||binding.publisher_user_ids) AS responsible(user_id)
              LEFT JOIN public.users subject ON subject.id=responsible.user_id WHERE subject.id IS NULL OR subject.status<>'ENABLED'))
          OR NOT EXISTS(SELECT 1 FROM public.platform_auth_application_requests tested WHERE tested.application_id=requested.application_id
            AND tested.environment='test' AND tested.status='TEST_ACCEPTED' AND tested.synced_version IN(tested.version-1,tested.version)
            AND tested.identity_projection_version IS NOT NULL AND tested.identity_admission_version IS NOT NULL
            AND tested.test_evidence_reference IS NOT NULL) THEN RETURN false; END IF;
    END IF;
    expected:=jsonb_build_object('applicationId',requested.application_id,'environment',requested.environment,
      'requestId',requested.id,'requestVersion',requested.version,'actorUserId',actor_ref,'actorSessionId',session_ref,
      'partnerId',requested.partner_id,'manifest',requested.manifest,'requestedBy',requested.requested_by,
      'reviewedBy',requested.reviewed_by,'publishedBy',requested.published_by,
      'contactReference',requested.reviewed_contact_reference,'domainReference',requested.reviewed_domain_reference,
      'status',CASE WHEN requested.status='SUSPENDED' THEN 'SUSPENDED' ELSE 'APPROVED' END);
    RETURN body_ref-ARRAY['expectedProjectionVersion','expectedAdmissionVersion']=expected
      AND requested.reviewed_contact_reference::text=requested.manifest->>'contactReference'
      AND requested.reviewed_domain_reference::text=requested.manifest->>'domainEvidenceReference'
      AND public.identity_auth_command_intent_active(operation_ref,service_ref,digest_ref,method_ref,path_ref);
END $$;

CREATE FUNCTION public.identity_auth_invitation_intent_active(
    operation_ref uuid,service_ref uuid,digest_ref text,method_ref text,path_ref text,actor_ref uuid,session_ref uuid
) RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER
SET search_path TO pg_catalog,public AS $$
BEGIN
    IF method_ref<>'POST'
       OR path_ref !~ '^/internal/identity/auth/invitations(/[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/revoke)?$'
       OR NOT public.identity_auth_command_intent_active(operation_ref,service_ref,digest_ref,method_ref,path_ref) THEN
        RETURN false;
    END IF;
    RETURN EXISTS(SELECT 1 FROM public.platform_auth_command_intents intent
        WHERE intent.id=operation_ref AND intent.service_client_id=service_ref
          AND intent.request_digest=digest_ref AND intent.method=method_ref AND intent.path=path_ref
          AND intent.actor_user_id=actor_ref AND intent.session_id=session_ref
          AND intent.management_permission='system:oauth:invite'
          AND public.platform_identity_management_actor_active(actor_ref,session_ref,'system:oauth:invite',true,false));
END $$;

-- Phase 15 internal capabilities and resource consumers are not OAuth clients.
-- These declarations follow the canonical business primary keys deliberately.
CREATE TABLE public.platform_resource_services (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    service_id text NOT NULL UNIQUE CHECK (service_id ~ '^[A-Za-z0-9][A-Za-z0-9._-]{0,99}$'),
    secret_hash text NOT NULL UNIQUE CHECK (secret_hash ~ '^[0-9a-f]{64}$'),
    credential_version bigint NOT NULL DEFAULT 1 CHECK (credential_version>=1),
    purpose text NOT NULL CHECK (purpose IN ('APP_RESOURCE','CUSTOMER_PROFILE')),
    application_id uuid NOT NULL REFERENCES public.applications(id),
    status text NOT NULL DEFAULT 'ENABLED' CHECK (status IN ('ENABLED','DISABLED')),
    allowed_member_types text[] NOT NULL DEFAULT ARRAY[]::text[]
        CHECK (public.valid_member_type_set(allowed_member_types,true)),
    allowed_operations text[] NOT NULL CHECK (cardinality(allowed_operations) BETWEEN 1 AND 128),
    audience text NOT NULL CHECK (length(audience) BETWEEN 1 AND 500 AND audience !~ '[[:cntrl:]]'),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK (purpose<>'CUSTOMER_PROFILE' OR (allowed_member_types=ARRAY['CUSTOMER']::text[]
        AND allowed_operations=ARRAY['customer:profile']::text[]))
);
CREATE FUNCTION public.platform_resource_service_generation() RETURNS trigger
LANGUAGE plpgsql SET search_path TO pg_catalog,public AS $$
BEGIN
    IF NEW.id IS DISTINCT FROM OLD.id OR NEW.created_at IS DISTINCT FROM OLD.created_at
       OR NEW.credential_version<OLD.credential_version THEN
        RAISE EXCEPTION 'PLATFORM_RESOURCE_SERVICE_BINDING_INVALID' USING ERRCODE='23514';
    END IF;
    IF (NEW.service_id,NEW.secret_hash,NEW.purpose,NEW.application_id,NEW.status,
        NEW.allowed_member_types,NEW.allowed_operations,NEW.audience)
       IS DISTINCT FROM
       (OLD.service_id,OLD.secret_hash,OLD.purpose,OLD.application_id,OLD.status,
        OLD.allowed_member_types,OLD.allowed_operations,OLD.audience) THEN
        NEW.credential_version:=GREATEST(NEW.credential_version,OLD.credential_version+1);
    END IF;
    NEW.updated_at:=clock_timestamp();
    RETURN NEW;
END $$;
CREATE TRIGGER platform_resource_service_generation
BEFORE UPDATE ON public.platform_resource_services
FOR EACH ROW EXECUTE FUNCTION public.platform_resource_service_generation();
CREATE FUNCTION public.platform_resource_service_binding() RETURNS trigger
LANGUAGE plpgsql SET search_path TO pg_catalog,public AS $$
BEGIN
    IF NEW.status='ENABLED' AND NOT EXISTS(SELECT 1 FROM public.applications application
        WHERE application.id=NEW.application_id AND application.code=NEW.audience) THEN
        RAISE EXCEPTION 'PLATFORM_RESOURCE_SERVICE_AUDIENCE_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER platform_resource_service_binding BEFORE INSERT OR UPDATE ON public.platform_resource_services
FOR EACH ROW EXECUTE FUNCTION public.platform_resource_service_binding();

CREATE TABLE public.platform_app_capabilities (
    token_hash text PRIMARY KEY CHECK (token_hash ~ '^[0-9a-f]{64}$'),
    id uuid NOT NULL UNIQUE CHECK (public.is_uuid_v7(id)),
    jti uuid NOT NULL UNIQUE CHECK (public.is_uuid_v7(jti)),
    user_id uuid NOT NULL REFERENCES public.users(id),
    tenant_id uuid NOT NULL REFERENCES public.tenants(id),
    organization_id uuid NOT NULL REFERENCES public.organizations(id),
    application_id uuid NOT NULL REFERENCES public.applications(id),
    audience text NOT NULL CHECK (length(audience) BETWEEN 1 AND 100),
    scope text NOT NULL DEFAULT 'app:access' CHECK (scope='app:access'),
    platform_session_id uuid NOT NULL REFERENCES public.platform_sessions(id),
    -- A derived capability must not block permanent member retirement. Missing
    -- ledger rows reject the original jti; audit and source facts remain separate.
    membership_id uuid NOT NULL REFERENCES public.memberships(id) ON DELETE CASCADE,
    member_type text NOT NULL CHECK (member_type IN ('EMPLOYEE','CUSTOMER')),
    identity_version bigint NOT NULL CHECK (identity_version>=1),
    account_version bigint NOT NULL CHECK (account_version>=1),
    application_policy_version bigint NOT NULL CHECK (application_policy_version>=1),
    origin_issuer text NOT NULL CHECK (origin_issuer ~ '^https://[^/?#[:space:]]+$'),
    origin_client_ref text NOT NULL CHECK (length(origin_client_ref) BETWEEN 1 AND 100),
    origin_client_policy_version bigint NOT NULL CHECK (origin_client_policy_version>=1),
    issued_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    revoked boolean NOT NULL DEFAULT false,
    CHECK (id=jti),
    CHECK (expires_at>issued_at AND expires_at<=issued_at+interval '300 seconds')
);
CREATE INDEX platform_app_capabilities_session_idx ON public.platform_app_capabilities(platform_session_id);
CREATE INDEX platform_app_capabilities_expiration_idx ON public.platform_app_capabilities(expires_at);
CREATE FUNCTION public.platform_app_capability_immutable() RETURNS trigger
LANGUAGE plpgsql SET search_path TO pg_catalog,public AS $$
BEGIN
    IF (NEW.token_hash,NEW.id,NEW.jti,NEW.user_id,NEW.tenant_id,NEW.organization_id,NEW.application_id,
        NEW.audience,NEW.scope,NEW.platform_session_id,NEW.membership_id,NEW.member_type,
        NEW.identity_version,NEW.account_version,NEW.application_policy_version,NEW.origin_issuer,
        NEW.origin_client_ref,NEW.origin_client_policy_version,NEW.issued_at,NEW.expires_at)
       IS DISTINCT FROM
       (OLD.token_hash,OLD.id,OLD.jti,OLD.user_id,OLD.tenant_id,OLD.organization_id,OLD.application_id,
        OLD.audience,OLD.scope,OLD.platform_session_id,OLD.membership_id,OLD.member_type,
        OLD.identity_version,OLD.account_version,OLD.application_policy_version,OLD.origin_issuer,
        OLD.origin_client_ref,OLD.origin_client_policy_version,OLD.issued_at,OLD.expires_at)
       OR (OLD.revoked AND NOT NEW.revoked) THEN
        RAISE EXCEPTION 'PLATFORM_APP_CAPABILITY_BINDING_IMMUTABLE' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER platform_app_capability_immutable BEFORE UPDATE ON public.platform_app_capabilities
FOR EACH ROW EXECUTE FUNCTION public.platform_app_capability_immutable();

CREATE FUNCTION public.platform_app_capability_revoke_grant() RETURNS trigger
LANGUAGE plpgsql SET search_path TO pg_catalog,public AS $$
BEGIN
    IF TG_TABLE_NAME='tenant_applications' THEN
        UPDATE public.platform_app_capabilities SET revoked=true
          WHERE application_id=OLD.application_id AND tenant_id=OLD.tenant_id AND NOT revoked;
        UPDATE public.context_selections SET resource_revoked_at=clock_timestamp()
          WHERE resource_revoked_at IS NULL
            AND selection#>>'{binding,registration,application,id}'=OLD.application_id::text
            AND selection#>>'{member,tenantId}'=OLD.tenant_id::text;
    ELSE
        UPDATE public.platform_app_capabilities SET revoked=true
          WHERE application_id=OLD.application_id AND organization_id=OLD.organization_id AND NOT revoked;
        UPDATE public.context_selections SET resource_revoked_at=clock_timestamp()
          WHERE resource_revoked_at IS NULL
            AND selection#>>'{binding,registration,application,id}'=OLD.application_id::text
            AND selection#>>'{member,organizationId}'=OLD.organization_id::text;
    END IF;
    RETURN OLD;
END $$;
CREATE TRIGGER platform_app_capability_tenant_grant_revoke AFTER DELETE ON public.tenant_applications
FOR EACH ROW EXECUTE FUNCTION public.platform_app_capability_revoke_grant();
CREATE TRIGGER platform_app_capability_org_grant_revoke AFTER DELETE ON public.organization_applications
FOR EACH ROW EXECUTE FUNCTION public.platform_app_capability_revoke_grant();
CREATE FUNCTION public.platform_app_capability_revoke_application() RETURNS trigger
LANGUAGE plpgsql SET search_path TO pg_catalog,public AS $$
BEGIN
    IF (NEW.status,NEW.code,NEW.runtime_mode,NEW.allowed_member_types,NEW.access_policy_version)
       IS DISTINCT FROM (OLD.status,OLD.code,OLD.runtime_mode,OLD.allowed_member_types,OLD.access_policy_version) THEN
        UPDATE public.platform_app_capabilities SET revoked=true WHERE application_id=NEW.id AND NOT revoked;
        UPDATE public.context_selections SET resource_revoked_at=clock_timestamp()
          WHERE resource_revoked_at IS NULL
            AND selection#>>'{binding,registration,application,id}'=NEW.id::text;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER platform_app_capability_application_revoke AFTER UPDATE ON public.applications
FOR EACH ROW EXECUTE FUNCTION public.platform_app_capability_revoke_application();

CREATE FUNCTION public.platform_app_capability_active(expected_hash text) RETURNS boolean
LANGUAGE sql STABLE SET search_path TO pg_catalog,public AS $$
    SELECT EXISTS(SELECT 1 FROM public.platform_app_capabilities capability
      JOIN public.platform_sessions parent ON parent.id=capability.platform_session_id
      JOIN public.bff_sessions source_bff ON source_bff.id=parent.bff_session_id
      JOIN public.applications application ON application.id=capability.application_id
      JOIN public.users account ON account.id=capability.user_id
      JOIN public.memberships member ON member.id=capability.membership_id
      JOIN public.organizations organization ON organization.id=capability.organization_id
      JOIN public.tenants tenant ON tenant.id=capability.tenant_id
      JOIN public.tenant_applications tenant_app ON tenant_app.tenant_id=capability.tenant_id
        AND tenant_app.application_id=capability.application_id
      JOIN public.organization_applications org_app ON org_app.organization_id=capability.organization_id
        AND org_app.application_id=capability.application_id
      WHERE capability.token_hash=expected_hash AND NOT capability.revoked
        AND capability.issued_at<=clock_timestamp() AND capability.expires_at>clock_timestamp()
        AND capability.expires_at<=capability.issued_at+interval '300 seconds'
        AND capability.scope='app:access' AND capability.id=capability.jti
        AND public.platform_session_active(parent.id)
        AND parent.user_id=capability.user_id AND parent.tenant_id=capability.tenant_id
        AND parent.organization_id=capability.organization_id AND parent.membership_id=capability.membership_id
        AND parent.member_type=capability.member_type AND parent.identity_version=capability.identity_version
        AND parent.account_version=capability.account_version AND capability.expires_at<=parent.expires_at
        AND capability.expires_at<=source_bff.expires_at
        AND parent.origin_issuer=capability.origin_issuer AND parent.origin_client_ref=capability.origin_client_ref
        AND parent.origin_client_policy_version=capability.origin_client_policy_version
        AND application.status='ENABLED' AND application.code=capability.audience
        AND application.access_policy_version=capability.application_policy_version
        AND capability.member_type=ANY(application.allowed_member_types)
        AND account.status='ENABLED' AND account.session_version=capability.account_version
        AND member.user_id=capability.user_id AND member.organization_id=capability.organization_id
        AND member.member_type=capability.member_type AND member.identity_version=capability.identity_version
        AND member.status='ENABLED' AND organization.tenant_id=capability.tenant_id
        AND public.org_chain_active(organization.id)
        AND (public.org_chain_expires_at(organization.id) IS NULL OR public.org_chain_expires_at(organization.id)>clock_timestamp())
        AND tenant.status='ENABLED' AND (tenant.expires_at IS NULL OR tenant.expires_at>clock_timestamp()))
$$;

CREATE TABLE public.platform_resource_bindings (
    application_id uuid NOT NULL REFERENCES public.applications(id),
    resource_code text NOT NULL CHECK (resource_code ~ '^[a-z][a-z0-9._-]{0,63}$'),
    action_code text NOT NULL CHECK (action_code ~ '^[a-z][a-z0-9._-]{0,63}$'),
    canonical_resource text NOT NULL CHECK (canonical_resource ~ '^[a-z][a-z0-9._-]{0,63}$'),
    audience text NOT NULL CHECK (length(audience) BETWEEN 1 AND 500 AND audience !~ '[[:cntrl:]]'),
    oauth_scope text NOT NULL CHECK (oauth_scope ~ '^[a-z][a-z0-9._:-]{0,127}$'),
    enabled boolean NOT NULL DEFAULT true,
    PRIMARY KEY(application_id,resource_code,action_code)
);
CREATE TABLE public.platform_machine_delegations (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    client_id uuid NOT NULL CHECK (public.is_uuid_v7(client_id)),
    source_application_id uuid NOT NULL CHECK (public.is_uuid_v7(source_application_id)),
    credential_version bigint NOT NULL CHECK (credential_version>=1),
    application_id uuid NOT NULL REFERENCES public.applications(id),
    tenant_id uuid NOT NULL REFERENCES public.tenants(id),
    organization_id uuid REFERENCES public.organizations(id),
    resource text NOT NULL CHECK (resource ~ '^[a-z][a-z0-9._-]{0,63}$'),
    action text NOT NULL CHECK (action ~ '^[a-z][a-z0-9._-]{0,63}$'),
    scope_type text NOT NULL CHECK (scope_type IN ('ORG','ORG_SUBTREE','TENANT')),
    organization_ids uuid[] NOT NULL DEFAULT ARRAY[]::uuid[] CHECK (cardinality(organization_ids)<=1000),
    version bigint NOT NULL DEFAULT 1 CHECK (version>=1),
    status text NOT NULL DEFAULT 'ENABLED' CHECK (status IN ('ENABLED','DISABLED')),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    expires_at timestamptz,
    CHECK (expires_at IS NULL OR expires_at>created_at),
    CHECK ((scope_type='TENANT' AND organization_id IS NULL AND cardinality(organization_ids)=0)
        OR (scope_type='ORG' AND organization_id IS NOT NULL AND organization_ids=ARRAY[organization_id])
        OR (scope_type='ORG_SUBTREE' AND organization_id IS NOT NULL AND cardinality(organization_ids)>0
            AND organization_id=ANY(organization_ids)))
);
CREATE INDEX platform_machine_delegations_source_idx ON public.platform_machine_delegations
    (client_id,source_application_id,credential_version,application_id,resource,action);
CREATE FUNCTION public.platform_machine_delegation_generation() RETURNS trigger
LANGUAGE plpgsql SET search_path TO pg_catalog,public AS $$
BEGIN
    IF NEW.id IS DISTINCT FROM OLD.id OR NEW.created_at IS DISTINCT FROM OLD.created_at
       OR NEW.version<OLD.version THEN
        RAISE EXCEPTION 'PLATFORM_MACHINE_DELEGATION_BINDING_INVALID' USING ERRCODE='23514';
    END IF;
    IF (NEW.client_id,NEW.source_application_id,NEW.credential_version,NEW.application_id,NEW.tenant_id,
        NEW.organization_id,NEW.resource,NEW.action,NEW.scope_type,NEW.organization_ids,NEW.status,NEW.expires_at)
       IS DISTINCT FROM
       (OLD.client_id,OLD.source_application_id,OLD.credential_version,OLD.application_id,OLD.tenant_id,
        OLD.organization_id,OLD.resource,OLD.action,OLD.scope_type,OLD.organization_ids,OLD.status,OLD.expires_at) THEN
        NEW.version:=GREATEST(NEW.version,OLD.version+1);
    END IF;
    NEW.updated_at:=clock_timestamp();
    RETURN NEW;
END $$;
CREATE TRIGGER platform_machine_delegation_generation BEFORE UPDATE ON public.platform_machine_delegations
FOR EACH ROW EXECUTE FUNCTION public.platform_machine_delegation_generation();

CREATE FUNCTION public.platform_machine_scope_valid(tenant_ref uuid, organization_ref uuid, scope_ref text, allowed_organizations uuid[]) RETURNS boolean
LANGUAGE sql STABLE SET search_path TO pg_catalog,public AS $$
    WITH RECURSIVE descendants AS (
      SELECT id FROM public.organizations WHERE id=organization_ref AND tenant_id=tenant_ref
      UNION
      SELECT child.id FROM public.organizations child JOIN descendants parent ON child.parent_id=parent.id
        WHERE child.tenant_id=tenant_ref
    ), live_descendants AS (
      SELECT d.id FROM descendants d WHERE public.org_chain_active(d.id)
        AND (public.org_chain_expires_at(d.id) IS NULL OR public.org_chain_expires_at(d.id)>clock_timestamp())
    )
    SELECT EXISTS(SELECT 1 FROM public.tenants tenant WHERE tenant.id=tenant_ref
        AND tenant.status='ENABLED' AND (tenant.expires_at IS NULL OR tenant.expires_at>clock_timestamp()))
      AND CASE scope_ref
        WHEN 'TENANT' THEN organization_ref IS NULL AND cardinality(allowed_organizations)=0
        WHEN 'ORG' THEN allowed_organizations=ARRAY[organization_ref]
          AND EXISTS(SELECT 1 FROM live_descendants WHERE id=organization_ref)
        WHEN 'ORG_SUBTREE' THEN EXISTS(SELECT 1 FROM live_descendants WHERE id=organization_ref)
          AND (SELECT array_agg(id ORDER BY id) FROM live_descendants)
            =(SELECT array_agg(id ORDER BY id) FROM unnest(allowed_organizations) AS entries(id))
        ELSE false END
$$;
CREATE FUNCTION public.platform_machine_delegation_scope_check() RETURNS trigger
LANGUAGE plpgsql SET search_path TO pg_catalog,public AS $$
BEGIN
    IF NEW.status='ENABLED' AND NOT public.platform_machine_scope_valid(
        NEW.tenant_id,NEW.organization_id,NEW.scope_type,NEW.organization_ids) THEN
        RAISE EXCEPTION 'PLATFORM_MACHINE_DELEGATION_SCOPE_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER platform_machine_delegation_scope_check
BEFORE INSERT OR UPDATE ON public.platform_machine_delegations
FOR EACH ROW EXECUTE FUNCTION public.platform_machine_delegation_scope_check();

ALTER TABLE public.identity_service_clients ADD COLUMN allowed_auth_resources uuid[] NOT NULL DEFAULT ARRAY[]::uuid[];
ALTER TABLE public.identity_service_clients DROP CONSTRAINT identity_service_clients_purpose_check;
ALTER TABLE public.identity_service_clients ADD CONSTRAINT identity_service_clients_purpose_check
    CHECK (purpose IN ('ACCOUNT_COMMAND','CUSTOMER_ACCESS_PROOF','KEY_ADMIN','APPLICATION_PROJECTION',
        'AUTH_ADMIN','CONTEXT_SELECTION_SOURCE','AUTH_INVITATION','AUTH_INVITATION_CONSUME',
        'AUTH_INVITATION_DELIVERY','AUTH_SECURITY','AUTH_REVIEW','RESOURCE_ACCESS_PROOF'));
REVOKE ALL ON FUNCTION public.platform_app_capability_active(expected_hash text) FROM PUBLIC;
REVOKE ALL ON FUNCTION public.platform_machine_scope_valid(tenant_ref uuid, organization_ref uuid, scope_ref text, allowed_organizations uuid[]) FROM PUBLIC;

ALTER TABLE public.context_selections ADD COLUMN resource_revoked_at timestamptz;
CREATE FUNCTION public.platform_context_registration_resource_revoke() RETURNS trigger
LANGUAGE plpgsql SET search_path TO pg_catalog,public AS $$
DECLARE target uuid;
BEGIN
    IF TG_OP='UPDATE' AND (NEW.client_id,NEW.client_registry,NEW.client_policy_version,NEW.member_type,
        NEW.application_id,NEW.application_policy_version,NEW.consumer_service_id,NEW.enabled)
       IS NOT DISTINCT FROM (OLD.client_id,OLD.client_registry,OLD.client_policy_version,OLD.member_type,
        OLD.application_id,OLD.application_policy_version,OLD.consumer_service_id,OLD.enabled) THEN
        RETURN NEW;
    END IF;
    target:=OLD.client_id;
    UPDATE public.context_selections SET resource_revoked_at=clock_timestamp()
      WHERE client_id=target AND resource_revoked_at IS NULL;
    IF TG_OP='DELETE' THEN RETURN OLD; END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER platform_context_registration_resource_revoke
AFTER UPDATE OR DELETE ON public.context_selection_registrations
FOR EACH ROW EXECUTE FUNCTION public.platform_context_registration_resource_revoke();

-- Consumed selections retain the immutable source and switch fence after the
-- short-lived selection credential expires. Runtime cleanup may remove only
-- selections that were never consumed.
CREATE FUNCTION public.context_selection_consumed_delete_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path TO pg_catalog,public AS $$
BEGIN
    IF OLD.consumed_at IS NOT NULL THEN
        RAISE EXCEPTION 'CONTEXT_SELECTION_CONSUMED_FACT_REQUIRED' USING ERRCODE='23514';
    END IF;
    RETURN OLD;
END $$;
CREATE TRIGGER context_selection_consumed_delete_guard
BEFORE DELETE ON public.context_selections
FOR EACH ROW EXECUTE FUNCTION public.context_selection_consumed_delete_guard();

-- Public RP session identifiers are opaque and separate from private SSO IDs.
ALTER TABLE public.bff_sessions
    ADD COLUMN oidc_public_sid text NOT NULL CHECK (oidc_public_sid ~ '^[0-9a-f]{64}$'),
    ADD COLUMN oidc_public_sub text NOT NULL CHECK (octet_length(oidc_public_sub) BETWEEN 1 AND 256 AND oidc_public_sub ~ '^[!-~]+$'),
    ADD COLUMN logout_confirmed_at timestamptz,
    ADD CONSTRAINT bff_logout_confirmation_binding CHECK (logout_confirmed_at IS NULL OR logout_request_id IS NOT NULL);
CREATE FUNCTION public.bff_public_source_immutable() RETURNS trigger
LANGUAGE plpgsql SET search_path TO pg_catalog,public AS $$
BEGIN
    IF (NEW.oidc_public_sid,NEW.oidc_public_sub) IS DISTINCT FROM (OLD.oidc_public_sid,OLD.oidc_public_sub) THEN
        RAISE EXCEPTION 'BFF_PUBLIC_SOURCE_IMMUTABLE' USING ERRCODE='23514';
    END IF;
    IF (OLD.logout_request_id IS NOT NULL AND (NEW.logout_request_id,NEW.logout_scope) IS DISTINCT FROM (OLD.logout_request_id,OLD.logout_scope))
       OR (OLD.logout_confirmed_at IS NOT NULL AND NEW.logout_confirmed_at IS DISTINCT FROM OLD.logout_confirmed_at) THEN
        RAISE EXCEPTION 'BFF_LOGOUT_RECEIPT_IMMUTABLE' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER bff_public_source_immutable BEFORE UPDATE ON public.bff_sessions
FOR EACH ROW EXECUTE FUNCTION public.bff_public_source_immutable();
CREATE TABLE public.platform_backchannel_logout_receipts (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    issuer text NOT NULL CHECK (issuer ~ '^https://[^/?#[:space:]]+$'),
    client_id uuid NOT NULL CHECK (public.is_uuid_v7(client_id)),
    jti_digest text NOT NULL CHECK (jti_digest ~ '^[0-9a-f]{64}$'),
    issued_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL CHECK (expires_at>issued_at),
    received_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(issuer,client_id,jti_digest)
);
CREATE INDEX platform_backchannel_logout_receipts_expiry ON public.platform_backchannel_logout_receipts(expires_at);
CREATE TABLE public.platform_backchannel_logout_sources (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    issuer text NOT NULL CHECK (issuer ~ '^https://[^/?#[:space:]]+$'),
    client_id uuid NOT NULL CHECK (public.is_uuid_v7(client_id)),
    sid_digest text NOT NULL CHECK (sid_digest ~ '^[0-9a-f]{64}$'),
    sub_digest text NOT NULL CHECK (sub_digest ~ '^[0-9a-f]{64}$'),
    revoked_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(issuer,client_id,sid_digest)
);

-- Phase 20 canonical RP logout and reliable, purpose-separated delivery facts.
CREATE TABLE public.auth_rp_sessions (
    grant_id uuid PRIMARY KEY REFERENCES public.auth_grants(id),
    client_id uuid NOT NULL REFERENCES public.auth_clients(id),
    user_id uuid NOT NULL REFERENCES public.users(id),
    source_session_id uuid NOT NULL REFERENCES public.sso_sessions(id),
    public_sid text NOT NULL CHECK (public_sid ~ '^[0-9a-f]{64}$'),
    public_sub text NOT NULL CHECK (octet_length(public_sub) BETWEEN 1 AND 256 AND public_sub ~ '^[!-~]+$'),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(client_id,public_sid)
);
CREATE TABLE public.auth_logout_command_receipts (
    operation_id uuid PRIMARY KEY CHECK (public.is_uuid_v7(operation_id)),
    client_id uuid NOT NULL REFERENCES public.auth_clients(id),
    grant_id uuid NOT NULL REFERENCES public.auth_grants(id),
    user_id uuid NOT NULL REFERENCES public.users(id),
    token_digest text NOT NULL CHECK (token_digest ~ '^[0-9a-f]{64}$'),
    scope text NOT NULL CHECK (scope IN('CURRENT_APP','GLOBAL')),
    public_sid text NOT NULL CHECK (public_sid ~ '^[0-9a-f]{64}$'),
    public_sub text NOT NULL CHECK (octet_length(public_sub) BETWEEN 1 AND 256 AND public_sub ~ '^[!-~]+$'),
    request_digest text NOT NULL CHECK (request_digest ~ '^[0-9a-f]{64}$'),
    result_json jsonb NOT NULL CHECK (jsonb_typeof(result_json)='object'),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE public.auth_delivery_targets (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    purpose text NOT NULL CHECK (purpose IN('BACK_CHANNEL','VERIFIED_CHANNEL')),
    status text NOT NULL CHECK (status IN('ACTIVE','REVOKED')),
    client_id uuid REFERENCES public.auth_clients(id),
    recipient_ref text CHECK (recipient_ref IS NULL OR (octet_length(recipient_ref) BETWEEN 1 AND 256 AND recipient_ref !~ '[[:cntrl:]]')),
    channel_purpose text CHECK (channel_purpose IN('ACCOUNT_ONBOARDING','SECURITY')),
    url text NOT NULL CHECK (octet_length(url) BETWEEN 1 AND 2048 AND url ~ '^https://[^/?#[:space:]@]+(/[^?#[:space:]]*)?$'),
    allowed_ips text[] NOT NULL CHECK (cardinality(allowed_ips) BETWEEN 1 AND 16 AND array_position(allowed_ips,NULL) IS NULL),
    version bigint NOT NULL DEFAULT 1 CHECK (version>=1),
    verified_reference uuid NOT NULL CHECK (public.is_uuid_v7(verified_reference)),
    verified_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL CHECK (expires_at>verified_at),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK ((purpose='BACK_CHANNEL' AND client_id IS NOT NULL AND recipient_ref IS NULL AND channel_purpose IS NULL)
        OR (purpose='VERIFIED_CHANNEL' AND client_id IS NULL AND recipient_ref IS NOT NULL AND channel_purpose IS NOT NULL))
);
ALTER TABLE public.auth_clients
    ADD COLUMN logout_mode text NOT NULL DEFAULT 'SHORT_SESSION_ONLINE_CHECK' CHECK (logout_mode IN('SHORT_SESSION_ONLINE_CHECK','BACK_CHANNEL')),
    ADD COLUMN backchannel_logout_uri text CHECK (backchannel_logout_uri IS NULL OR (octet_length(backchannel_logout_uri)<=2048 AND backchannel_logout_uri ~ '^https://[^/?#[:space:]@]+(/[^?#[:space:]]*)?$')),
    ADD COLUMN backchannel_target_id uuid REFERENCES public.auth_delivery_targets(id),
    ADD CONSTRAINT auth_client_logout_binding CHECK ((logout_mode='SHORT_SESSION_ONLINE_CHECK' AND backchannel_logout_uri IS NULL AND backchannel_target_id IS NULL)
        OR (logout_mode='BACK_CHANNEL' AND backchannel_logout_uri IS NOT NULL AND backchannel_target_id IS NOT NULL));
ALTER TABLE public.auth_application_admissions
    ADD COLUMN logout_mode text NOT NULL DEFAULT 'SHORT_SESSION_ONLINE_CHECK' CHECK (logout_mode IN('SHORT_SESSION_ONLINE_CHECK','BACK_CHANNEL')),
    ADD COLUMN backchannel_logout_uri text,
    ADD CONSTRAINT auth_admission_logout_binding CHECK ((logout_mode='SHORT_SESSION_ONLINE_CHECK' AND backchannel_logout_uri IS NULL)
        OR (logout_mode='BACK_CHANNEL' AND backchannel_logout_uri IS NOT NULL AND octet_length(backchannel_logout_uri) BETWEEN 1 AND 2048 AND backchannel_logout_uri ~ '^https://[^/?#[:space:]@]+(/[^?#[:space:]]*)?$'));
CREATE TABLE public.auth_delivery_jobs (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    kind text NOT NULL CHECK (kind IN('LOGOUT','INVITATION','SECURITY')),
    event_key text NOT NULL UNIQUE CHECK (event_key ~ '^[0-9a-f]{64}$'),
    target_id uuid REFERENCES public.auth_delivery_targets(id),
    target_version bigint CHECK (target_version>=1),
    grant_id uuid REFERENCES public.auth_grants(id),
    invitation_id uuid REFERENCES public.auth_invitations(id),
    security_event_id uuid REFERENCES public.auth_security_notification_outbox(id),
    consent_event_id uuid REFERENCES public.auth_consent_revocation_outbox(id),
    state text NOT NULL DEFAULT 'PENDING' CHECK (state IN('PENDING','LEASED','DELIVERED','TERMINAL','DEGRADED')),
    available_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    attempt_count integer NOT NULL DEFAULT 0 CHECK (attempt_count>=0 AND attempt_count<=max_attempts),
    max_attempts integer NOT NULL DEFAULT 8 CHECK (max_attempts BETWEEN 1 AND 8),
    lease_id uuid CHECK (lease_id IS NULL OR public.is_uuid_v7(lease_id)),
    lease_until timestamptz,
    last_jti uuid CHECK (last_jti IS NULL OR public.is_uuid_v7(last_jti)),
    last_error_code text CHECK (last_error_code IS NULL OR last_error_code ~ '^[A-Z0-9_]{1,64}$'),
    delivered_at timestamptz,
    terminal_at timestamptz,
    CHECK ((target_id IS NULL)=(target_version IS NULL)),
    CHECK (state='DEGRADED' OR target_id IS NOT NULL),
    CHECK ((state='LEASED')=(lease_id IS NOT NULL) AND (lease_id IS NULL)=(lease_until IS NULL)),
    CHECK ((kind='LOGOUT' AND grant_id IS NOT NULL AND invitation_id IS NULL AND security_event_id IS NULL)
        OR (kind='INVITATION' AND grant_id IS NULL AND invitation_id IS NOT NULL AND security_event_id IS NULL AND consent_event_id IS NULL)
        OR (kind='SECURITY' AND grant_id IS NULL AND invitation_id IS NULL AND security_event_id IS NOT NULL AND consent_event_id IS NULL))
);
CREATE INDEX auth_delivery_jobs_pending ON public.auth_delivery_jobs(available_at,created_at,id) WHERE state IN('PENDING','LEASED');
ALTER TABLE public.auth_consent_revocation_outbox
    ADD COLUMN expanded_at timestamptz,
    ADD COLUMN terminal_at timestamptz,
    ADD COLUMN delivery_state text NOT NULL DEFAULT 'PENDING' CHECK (delivery_state IN('PENDING','EXPANDED','DELIVERED','TERMINAL','DEGRADED'));
ALTER TABLE public.auth_security_notification_outbox
    ADD COLUMN expanded_at timestamptz,
    ADD COLUMN terminal_at timestamptz,
    ADD COLUMN delivery_state text NOT NULL DEFAULT 'PENDING' CHECK (delivery_state IN('PENDING','EXPANDED','DELIVERED','TERMINAL','DEGRADED'));

CREATE TABLE public.auth_invitation_verifications (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    invitation_id uuid NOT NULL REFERENCES public.auth_invitations(id),
    recipient_ref text NOT NULL CHECK (octet_length(recipient_ref) BETWEEN 1 AND 256 AND recipient_ref !~ '[[:cntrl:]]'),
    purpose text NOT NULL CHECK (purpose='ACCOUNT_ONBOARDING'),
    delivery_target_id uuid NOT NULL REFERENCES public.auth_delivery_targets(id),
    delivery_target_version bigint NOT NULL CHECK (delivery_target_version>=1),
    verified_reference uuid NOT NULL CHECK (public.is_uuid_v7(verified_reference)),
    verified_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL CHECK (expires_at>verified_at),
    status text NOT NULL CHECK (status IN('ACTIVE','REVOKED')),
    version bigint NOT NULL DEFAULT 1 CHECK (version>=1),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE public.auth_invitation_activation_receipts (
    operation_id uuid PRIMARY KEY CHECK (public.is_uuid_v7(operation_id)),
    invitation_id uuid NOT NULL UNIQUE REFERENCES public.auth_invitations(id),
    verification_id uuid NOT NULL UNIQUE REFERENCES public.auth_invitation_verifications(id),
    user_id uuid NOT NULL UNIQUE REFERENCES public.users(id),
    invitation_version bigint NOT NULL CHECK (invitation_version>=1),
    verification_version bigint NOT NULL CHECK (verification_version>=1),
    delivery_target_id uuid NOT NULL REFERENCES public.auth_delivery_targets(id),
    delivery_target_version bigint NOT NULL CHECK (delivery_target_version>=1),
    request_digest text NOT NULL CHECK (request_digest ~ '^[0-9a-f]{64}$'),
    key_version bigint NOT NULL CHECK (key_version>=1),
    result_json jsonb NOT NULL CHECK (jsonb_typeof(result_json)='object'),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(operation_id,invitation_id,user_id,verification_id)
);
ALTER TABLE public.auth_invitations
    ADD CONSTRAINT auth_invitation_verification_fk FOREIGN KEY(verification_ref) REFERENCES public.auth_invitation_verifications(id),
    ADD CONSTRAINT auth_invitation_activation_tuple_fk FOREIGN KEY(activation_operation_id,id,activated_account_id,verification_ref)
        REFERENCES public.auth_invitation_activation_receipts(operation_id,invitation_id,user_id,verification_id);

CREATE FUNCTION public.auth_phase20_receipt_immutable() RETURNS trigger
LANGUAGE plpgsql SET search_path TO pg_catalog,public AS $$
BEGIN
    RAISE EXCEPTION 'AUTH_ORIGINAL_RECEIPT_IMMUTABLE' USING ERRCODE='23514';
END $$;
CREATE TRIGGER auth_rp_session_immutable BEFORE UPDATE ON public.auth_rp_sessions FOR EACH ROW EXECUTE FUNCTION public.auth_phase20_receipt_immutable();
CREATE TRIGGER auth_logout_receipt_immutable BEFORE UPDATE ON public.auth_logout_command_receipts FOR EACH ROW EXECUTE FUNCTION public.auth_phase20_receipt_immutable();
CREATE TRIGGER auth_activation_receipt_immutable BEFORE UPDATE ON public.auth_invitation_activation_receipts FOR EACH ROW EXECUTE FUNCTION public.auth_phase20_receipt_immutable();
CREATE FUNCTION public.auth_rp_session_source_check() RETURNS trigger
LANGUAGE plpgsql SET search_path TO pg_catalog,public AS $$
BEGIN
    IF NOT EXISTS(SELECT 1 FROM public.auth_grants grant_row WHERE grant_row.id=NEW.grant_id
        AND grant_row.subject_mode='USER' AND grant_row.revoked_at IS NULL
        AND grant_row.client_id=NEW.client_id AND grant_row.user_id=NEW.user_id AND grant_row.source_session_id=NEW.source_session_id) THEN
        RAISE EXCEPTION 'AUTH_RP_SESSION_SOURCE_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER auth_rp_session_source_check BEFORE INSERT ON public.auth_rp_sessions FOR EACH ROW EXECUTE FUNCTION public.auth_rp_session_source_check();

CREATE FUNCTION public.auth_delivery_target_generation() RETURNS trigger
LANGUAGE plpgsql SET search_path TO pg_catalog,public AS $$
BEGIN
    IF NEW.id IS DISTINCT FROM OLD.id OR NEW.created_at IS DISTINCT FROM OLD.created_at OR NEW.version<OLD.version THEN
        RAISE EXCEPTION 'AUTH_DELIVERY_TARGET_IMMUTABLE' USING ERRCODE='23514';
    END IF;
    IF (NEW.purpose,NEW.status,NEW.client_id,NEW.recipient_ref,NEW.channel_purpose,NEW.url,NEW.allowed_ips,
        NEW.verified_reference,NEW.verified_at,NEW.expires_at) IS DISTINCT FROM
       (OLD.purpose,OLD.status,OLD.client_id,OLD.recipient_ref,OLD.channel_purpose,OLD.url,OLD.allowed_ips,
        OLD.verified_reference,OLD.verified_at,OLD.expires_at) THEN
        NEW.version:=GREATEST(NEW.version,OLD.version+1);
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER auth_delivery_target_generation BEFORE UPDATE ON public.auth_delivery_targets FOR EACH ROW EXECUTE FUNCTION public.auth_delivery_target_generation();
CREATE FUNCTION public.auth_client_logout_target_check() RETURNS trigger
LANGUAGE plpgsql SET search_path TO pg_catalog,public AS $$
BEGIN
    IF TG_OP='UPDATE' AND NEW.status=OLD.status AND NEW.policy_version>OLD.policy_version
       AND (to_jsonb(NEW)-ARRAY['version','policy_version','updated_at'])
         =(to_jsonb(OLD)-ARRAY['version','policy_version','updated_at']) THEN
        RETURN NEW; -- Emergency credential revocation is also pure shrink.
    END IF;
    IF TG_OP='UPDATE' AND NEW.status IN('DISABLED','RETIRED') AND NEW.status IS DISTINCT FROM OLD.status
       AND (to_jsonb(NEW)-ARRAY['status','version','policy_version','updated_at'])
         =(to_jsonb(OLD)-ARRAY['status','version','policy_version','updated_at']) THEN
        RETURN NEW; -- Pure authority shrink must not depend on a live endpoint.
    END IF;
    IF NEW.logout_mode='BACK_CHANNEL' AND (NEW.subject_mode<>'USER' OR NEW.client_type<>'CONFIDENTIAL'
      OR NOT EXISTS(SELECT 1 FROM public.auth_delivery_targets target WHERE target.id=NEW.backchannel_target_id
        AND target.purpose='BACK_CHANNEL' AND target.client_id=NEW.id AND target.status='ACTIVE'
        AND target.url=NEW.backchannel_logout_uri AND target.verified_at<=clock_timestamp() AND target.expires_at>clock_timestamp())
      OR NOT EXISTS(SELECT 1 FROM public.auth_applications application WHERE application.application_id=NEW.application_id
        AND (application.ownership='FIRST_PARTY' OR EXISTS(SELECT 1 FROM public.auth_application_admissions admission
          WHERE admission.application_id=NEW.application_id AND admission.environment=NEW.environment
            AND admission.status='APPROVED' AND admission.logout_mode='BACK_CHANNEL'
            AND admission.backchannel_logout_uri=NEW.backchannel_logout_uri)))) THEN
        RAISE EXCEPTION 'AUTH_CLIENT_LOGOUT_TARGET_UNVERIFIED' USING ERRCODE='23514';
    END IF;
    IF TG_OP='UPDATE' AND (NEW.logout_mode,NEW.backchannel_logout_uri,NEW.backchannel_target_id)
       IS DISTINCT FROM (OLD.logout_mode,OLD.backchannel_logout_uri,OLD.backchannel_target_id) THEN
        NEW.policy_version:=GREATEST(NEW.policy_version,OLD.policy_version+1);
        NEW.version:=GREATEST(NEW.version,OLD.version+1);
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER auth_client_logout_target_check BEFORE INSERT OR UPDATE ON public.auth_clients FOR EACH ROW EXECUTE FUNCTION public.auth_client_logout_target_check();
CREATE FUNCTION public.auth_logout_receipt_source_check() RETURNS trigger
LANGUAGE plpgsql SET search_path TO pg_catalog,public AS $$
BEGIN
    IF NOT EXISTS(SELECT 1 FROM public.auth_rp_sessions rp WHERE rp.grant_id=NEW.grant_id AND rp.client_id=NEW.client_id
        AND rp.user_id=NEW.user_id AND rp.public_sid=NEW.public_sid AND rp.public_sub=NEW.public_sub)
       OR NOT (EXISTS(SELECT 1 FROM public.auth_access_tokens token WHERE token.grant_id=NEW.grant_id AND token.token_digest=NEW.token_digest)
         OR EXISTS(SELECT 1 FROM public.auth_refresh_tokens token JOIN public.auth_refresh_families family ON family.id=token.family_id
           WHERE family.grant_id=NEW.grant_id AND token.token_digest=NEW.token_digest)) THEN
        RAISE EXCEPTION 'AUTH_LOGOUT_RECEIPT_SOURCE_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER auth_logout_receipt_source_check BEFORE INSERT ON public.auth_logout_command_receipts FOR EACH ROW EXECUTE FUNCTION public.auth_logout_receipt_source_check();
CREATE FUNCTION public.auth_invitation_activation_source_check() RETURNS trigger
LANGUAGE plpgsql SET search_path TO pg_catalog,public AS $$
BEGIN
    IF NOT EXISTS(SELECT 1 FROM public.auth_invitations invitation
      JOIN public.auth_invitation_verifications verification ON verification.id=NEW.verification_id
        AND verification.invitation_id=invitation.id AND verification.recipient_ref=invitation.recipient_ref AND verification.purpose=invitation.purpose
      JOIN public.auth_delivery_targets target ON target.id=verification.delivery_target_id
      JOIN public.auth_delivery_jobs job ON job.invitation_id=invitation.id AND job.kind='INVITATION'
        AND job.target_id=target.id AND job.target_version=target.version
      WHERE invitation.id=NEW.invitation_id AND invitation.version=NEW.invitation_version AND invitation.status='ACTIVE'
        AND invitation.expires_at>clock_timestamp() AND verification.status='ACTIVE' AND verification.version=NEW.verification_version
        AND verification.verified_at<=clock_timestamp() AND verification.expires_at>clock_timestamp()
        AND verification.delivery_target_id=NEW.delivery_target_id AND verification.delivery_target_version=NEW.delivery_target_version
        AND target.version=NEW.delivery_target_version AND target.status='ACTIVE' AND target.purpose='VERIFIED_CHANNEL'
        AND target.channel_purpose='ACCOUNT_ONBOARDING' AND target.recipient_ref=invitation.recipient_ref
        AND target.verified_at<=clock_timestamp() AND target.expires_at>clock_timestamp()) THEN
        RAISE EXCEPTION 'AUTH_INVITATION_ACTIVATION_SOURCE_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER auth_invitation_activation_source_check BEFORE INSERT ON public.auth_invitation_activation_receipts FOR EACH ROW EXECUTE FUNCTION public.auth_invitation_activation_source_check();

-- Trigger-created jobs retain the source UUIDv7 time and a purpose-separated
-- event digest tail. There is no v4/default generator or new extension.
CREATE FUNCTION public.auth_delivery_derived_id(source_ref uuid,event_digest text) RETURNS uuid
LANGUAGE sql IMMUTABLE STRICT SET search_path TO pg_catalog,public AS $$
    SELECT CASE WHEN public.is_uuid_v7(source_ref) AND event_digest ~ '^[0-9a-f]{64}$' THEN
      (substr(replace(source_ref::text,'-',''),1,12)||'7'||substr(event_digest,1,3)
       ||substr('89ab',(strpos('0123456789abcdef',substr(event_digest,4,1))-1)%4+1,1)||substr(event_digest,5,15))::uuid END
$$;
CREATE FUNCTION public.auth_enqueue_logout(grant_ref uuid,consent_ref uuid) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path TO pg_catalog,public AS $$
DECLARE source record; destination record; event_ref text;
BEGIN
    SELECT rp.*,client.logout_mode,client.backchannel_target_id,client.backchannel_logout_uri INTO source
      FROM public.auth_rp_sessions rp JOIN public.auth_grants grant_row ON grant_row.id=rp.grant_id
      JOIN public.auth_clients client ON client.id=rp.client_id
      WHERE rp.grant_id=grant_ref AND grant_row.subject_mode='USER' AND grant_row.revoked_at IS NOT NULL;
    IF NOT FOUND THEN RETURN; END IF;
    event_ref:=encode(sha256(convert_to('logout/1'||chr(10)||grant_ref::text,'UTF8')),'hex');
    SELECT target.* INTO destination FROM public.auth_delivery_targets target
      WHERE source.logout_mode='BACK_CHANNEL' AND target.id=source.backchannel_target_id
        AND target.client_id=source.client_id AND target.purpose='BACK_CHANNEL' AND target.status='ACTIVE'
        AND target.url=source.backchannel_logout_uri AND target.verified_at<=clock_timestamp() AND target.expires_at>clock_timestamp();
    IF FOUND THEN
        INSERT INTO public.auth_delivery_jobs(id,kind,event_key,target_id,target_version,grant_id,consent_event_id)
          VALUES(public.auth_delivery_derived_id(grant_ref,event_ref),'LOGOUT',event_ref,destination.id,destination.version,grant_ref,consent_ref)
          ON CONFLICT(event_key) DO UPDATE SET consent_event_id=COALESCE(auth_delivery_jobs.consent_event_id,EXCLUDED.consent_event_id);
    ELSE
        INSERT INTO public.auth_delivery_jobs(id,kind,event_key,grant_id,consent_event_id,state,last_error_code,terminal_at)
          VALUES(public.auth_delivery_derived_id(grant_ref,event_ref),'LOGOUT',event_ref,grant_ref,consent_ref,'DEGRADED','SHORT_SESSION_CONTRACT',clock_timestamp())
          ON CONFLICT(event_key) DO UPDATE SET consent_event_id=COALESCE(auth_delivery_jobs.consent_event_id,EXCLUDED.consent_event_id);
    END IF;
END $$;
CREATE FUNCTION public.auth_grant_revoked_delivery() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path TO pg_catalog,public AS $$
BEGIN
    IF OLD.revoked_at IS NULL AND NEW.revoked_at IS NOT NULL AND NEW.subject_mode='USER' THEN
        PERFORM public.auth_enqueue_logout(NEW.id,NULL);
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER auth_grant_revoked_delivery AFTER UPDATE OF revoked_at ON public.auth_grants FOR EACH ROW EXECUTE FUNCTION public.auth_grant_revoked_delivery();
CREATE FUNCTION public.auth_sso_revocation_delivery() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path TO pg_catalog,public AS $$
BEGIN
    IF OLD.revoked_at IS NULL AND NEW.revoked_at IS NOT NULL THEN
        UPDATE public.auth_grants SET revoked_at=clock_timestamp() WHERE subject_mode='USER' AND source_session_id=NEW.id AND revoked_at IS NULL;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER auth_sso_revocation_delivery AFTER UPDATE OF revoked_at ON public.sso_sessions FOR EACH ROW EXECUTE FUNCTION public.auth_sso_revocation_delivery();
CREATE FUNCTION public.auth_account_revocation_delivery() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path TO pg_catalog,public AS $$
BEGIN
    IF NEW.status IS DISTINCT FROM OLD.status OR NEW.session_version IS DISTINCT FROM OLD.session_version THEN
        UPDATE public.sso_sessions SET revoked_at=clock_timestamp() WHERE user_id=NEW.id AND revoked_at IS NULL;
        UPDATE public.auth_grants SET revoked_at=clock_timestamp() WHERE subject_mode='USER' AND user_id=NEW.id AND revoked_at IS NULL;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER auth_account_revocation_delivery AFTER UPDATE OF status,session_version ON public.users FOR EACH ROW EXECUTE FUNCTION public.auth_account_revocation_delivery();
CREATE FUNCTION public.auth_client_revocation_delivery() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path TO pg_catalog,public AS $$
BEGIN
    IF NEW.policy_version IS DISTINCT FROM OLD.policy_version OR NEW.status<>'ENABLED' THEN
        UPDATE public.auth_grants SET revoked_at=clock_timestamp() WHERE client_id=NEW.id AND revoked_at IS NULL
          AND (NEW.status<>'ENABLED' OR client_policy_version<>NEW.policy_version);
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER auth_client_revocation_delivery AFTER UPDATE OF status,policy_version ON public.auth_clients FOR EACH ROW EXECUTE FUNCTION public.auth_client_revocation_delivery();

CREATE FUNCTION public.auth_delivery_job_binding() RETURNS trigger
LANGUAGE plpgsql SET search_path TO pg_catalog,public AS $$
BEGIN
    IF TG_OP='UPDATE' THEN
        IF (NEW.id,NEW.kind,NEW.event_key,NEW.target_id,NEW.target_version,NEW.grant_id,NEW.invitation_id,
            NEW.security_event_id,NEW.created_at,NEW.max_attempts) IS DISTINCT FROM
           (OLD.id,OLD.kind,OLD.event_key,OLD.target_id,OLD.target_version,OLD.grant_id,OLD.invitation_id,
            OLD.security_event_id,OLD.created_at,OLD.max_attempts)
           OR NEW.attempt_count<OLD.attempt_count
           OR (OLD.consent_event_id IS NOT NULL AND NEW.consent_event_id IS DISTINCT FROM OLD.consent_event_id)
           OR (OLD.state IN('DELIVERED','TERMINAL','DEGRADED') AND
             (NEW.state,NEW.available_at,NEW.attempt_count,NEW.lease_id,NEW.lease_until,NEW.last_jti,NEW.last_error_code,NEW.delivered_at,NEW.terminal_at)
                IS DISTINCT FROM
             (OLD.state,OLD.available_at,OLD.attempt_count,OLD.lease_id,OLD.lease_until,OLD.last_jti,OLD.last_error_code,OLD.delivered_at,OLD.terminal_at)) THEN
            RAISE EXCEPTION 'AUTH_DELIVERY_JOB_BINDING_IMMUTABLE' USING ERRCODE='23514';
        END IF;
    END IF;
    IF NEW.consent_event_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM public.auth_grants grant_row
      JOIN public.auth_consent_revocation_outbox event ON event.id=NEW.consent_event_id
        AND event.consent_id=grant_row.consent_id AND event.user_id=grant_row.user_id AND event.client_id=grant_row.client_id
      WHERE grant_row.id=NEW.grant_id) THEN
        RAISE EXCEPTION 'AUTH_DELIVERY_CONSENT_SOURCE_INVALID' USING ERRCODE='23514';
    END IF;
    IF TG_OP='INSERT' AND NEW.target_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM public.auth_delivery_targets target
      WHERE target.id=NEW.target_id AND target.version=NEW.target_version AND target.status='ACTIVE'
        AND target.verified_at<=clock_timestamp() AND target.expires_at>clock_timestamp()
        AND ((NEW.kind='LOGOUT' AND target.purpose='BACK_CHANNEL' AND EXISTS(SELECT 1 FROM public.auth_grants grant_row
            JOIN public.auth_rp_sessions rp ON rp.grant_id=grant_row.id
            WHERE grant_row.id=NEW.grant_id AND target.client_id=grant_row.client_id))
          OR (NEW.kind='INVITATION' AND target.purpose='VERIFIED_CHANNEL' AND target.channel_purpose='ACCOUNT_ONBOARDING'
            AND EXISTS(SELECT 1 FROM public.auth_invitations invitation WHERE invitation.id=NEW.invitation_id AND invitation.recipient_ref=target.recipient_ref))
          OR (NEW.kind='SECURITY' AND target.purpose='VERIFIED_CHANNEL' AND target.channel_purpose='SECURITY'
            AND EXISTS(SELECT 1 FROM public.auth_security_notification_outbox event WHERE event.id=NEW.security_event_id AND event.user_id::text=target.recipient_ref)))) THEN
        RAISE EXCEPTION 'AUTH_DELIVERY_TARGET_SOURCE_INVALID' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER auth_delivery_job_binding BEFORE INSERT OR UPDATE ON public.auth_delivery_jobs FOR EACH ROW EXECUTE FUNCTION public.auth_delivery_job_binding();

CREATE FUNCTION public.auth_consent_delivery_expand() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path TO pg_catalog,public AS $$
DECLARE grant_ref uuid;
BEGIN
    FOR grant_ref IN SELECT grant_row.id FROM public.auth_grants grant_row WHERE grant_row.consent_id=NEW.consent_id
      AND grant_row.client_id=NEW.client_id AND grant_row.user_id=NEW.user_id AND grant_row.subject_mode='USER' AND grant_row.revoked_at IS NOT NULL LOOP
        PERFORM public.auth_enqueue_logout(grant_ref,NEW.id);
    END LOOP;
    UPDATE public.auth_consent_revocation_outbox SET expanded_at=clock_timestamp(),
      delivery_state=CASE WHEN EXISTS(SELECT 1 FROM public.auth_delivery_jobs WHERE consent_event_id=NEW.id) THEN delivery_state ELSE 'DEGRADED' END,
      terminal_at=CASE WHEN EXISTS(SELECT 1 FROM public.auth_delivery_jobs WHERE consent_event_id=NEW.id) THEN terminal_at ELSE clock_timestamp() END
      WHERE id=NEW.id;
    RETURN NEW;
END $$;
CREATE TRIGGER auth_consent_delivery_expand AFTER INSERT ON public.auth_consent_revocation_outbox FOR EACH ROW EXECUTE FUNCTION public.auth_consent_delivery_expand();
CREATE FUNCTION public.auth_security_delivery_expand() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path TO pg_catalog,public AS $$
DECLARE destination record; event_ref text; queued boolean:=false;
BEGIN
    FOR destination IN SELECT target.* FROM public.auth_delivery_targets target WHERE target.purpose='VERIFIED_CHANNEL'
      AND target.channel_purpose='SECURITY' AND target.recipient_ref=NEW.user_id::text AND target.status='ACTIVE'
      AND target.verified_at<=clock_timestamp() AND target.expires_at>clock_timestamp() LOOP
        queued:=true;
        event_ref:=encode(sha256(convert_to('security/1'||chr(10)||NEW.id::text||chr(10)||destination.id::text,'UTF8')),'hex');
        INSERT INTO public.auth_delivery_jobs(id,kind,event_key,target_id,target_version,security_event_id)
          VALUES(public.auth_delivery_derived_id(NEW.id,event_ref),'SECURITY',event_ref,destination.id,destination.version,NEW.id)
          ON CONFLICT(event_key) DO NOTHING;
    END LOOP;
    IF NOT queued THEN
        event_ref:=encode(sha256(convert_to('security/1'||chr(10)||NEW.id::text||chr(10)||'none','UTF8')),'hex');
        INSERT INTO public.auth_delivery_jobs(id,kind,event_key,security_event_id,state,last_error_code,terminal_at)
          VALUES(public.auth_delivery_derived_id(NEW.id,event_ref),'SECURITY',event_ref,NEW.id,'DEGRADED','VERIFIED_CHANNEL_MISSING',clock_timestamp())
          ON CONFLICT(event_key) DO NOTHING;
    END IF;
    UPDATE public.auth_security_notification_outbox SET expanded_at=clock_timestamp(),delivery_state=CASE WHEN queued THEN 'EXPANDED' ELSE 'DEGRADED' END,
      terminal_at=CASE WHEN queued THEN NULL ELSE clock_timestamp() END WHERE id=NEW.id;
    RETURN NEW;
END $$;
CREATE TRIGGER auth_security_delivery_expand AFTER INSERT ON public.auth_security_notification_outbox FOR EACH ROW EXECUTE FUNCTION public.auth_security_delivery_expand();
CREATE FUNCTION public.auth_delivery_source_summary() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path TO pg_catalog,public AS $$
DECLARE all_delivered boolean; all_finished boolean; any_terminal boolean; result_state text;
BEGIN
    IF NEW.security_event_id IS NULL AND NEW.consent_event_id IS NULL THEN RETURN NEW; END IF;
    SELECT bool_and(state='DELIVERED'),bool_and(state IN('DELIVERED','TERMINAL','DEGRADED')),bool_or(state='TERMINAL')
      INTO all_delivered,all_finished,any_terminal FROM public.auth_delivery_jobs
      WHERE (NEW.security_event_id IS NOT NULL AND security_event_id=NEW.security_event_id)
         OR (NEW.consent_event_id IS NOT NULL AND consent_event_id=NEW.consent_event_id);
    result_state:=CASE WHEN all_delivered THEN 'DELIVERED' WHEN all_finished AND any_terminal THEN 'TERMINAL'
      WHEN all_finished THEN 'DEGRADED' ELSE 'EXPANDED' END;
    IF NEW.security_event_id IS NOT NULL THEN
        UPDATE public.auth_security_notification_outbox SET delivery_state=result_state,
          delivered_at=CASE WHEN all_delivered THEN COALESCE(delivered_at,clock_timestamp()) ELSE delivered_at END,
          terminal_at=CASE WHEN all_finished THEN COALESCE(terminal_at,clock_timestamp()) ELSE terminal_at END
          WHERE id=NEW.security_event_id;
    END IF;
    IF NEW.consent_event_id IS NOT NULL THEN
        UPDATE public.auth_consent_revocation_outbox SET delivery_state=result_state,
          delivered_at=CASE WHEN all_delivered THEN COALESCE(delivered_at,clock_timestamp()) ELSE delivered_at END,
          terminal_at=CASE WHEN all_finished THEN COALESCE(terminal_at,clock_timestamp()) ELSE terminal_at END
          WHERE id=NEW.consent_event_id;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER auth_delivery_source_summary AFTER INSERT OR UPDATE ON public.auth_delivery_jobs FOR EACH ROW EXECUTE FUNCTION public.auth_delivery_source_summary();

CREATE FUNCTION public.auth_invitation_verification_generation() RETURNS trigger
LANGUAGE plpgsql SET search_path TO pg_catalog,public AS $$
BEGIN
    IF (NEW.id,NEW.invitation_id,NEW.recipient_ref,NEW.purpose,NEW.delivery_target_id,NEW.created_at)
      IS DISTINCT FROM (OLD.id,OLD.invitation_id,OLD.recipient_ref,OLD.purpose,OLD.delivery_target_id,OLD.created_at) OR NEW.version<OLD.version THEN
        RAISE EXCEPTION 'AUTH_INVITATION_VERIFICATION_IMMUTABLE' USING ERRCODE='23514';
    END IF;
    IF (NEW.delivery_target_version,NEW.verified_reference,NEW.verified_at,NEW.expires_at,NEW.status) IS DISTINCT FROM
       (OLD.delivery_target_version,OLD.verified_reference,OLD.verified_at,OLD.expires_at,OLD.status) THEN NEW.version:=GREATEST(NEW.version,OLD.version+1); END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER auth_invitation_verification_generation BEFORE UPDATE ON public.auth_invitation_verifications FOR EACH ROW EXECUTE FUNCTION public.auth_invitation_verification_generation();
CREATE FUNCTION public.auth_invitation_original_binding() RETURNS trigger
LANGUAGE plpgsql SET search_path TO pg_catalog,public AS $$
BEGIN
    IF (NEW.id,NEW.service_id,NEW.operation_id,NEW.request_digest,NEW.recipient_ref,NEW.purpose,NEW.actor_user_id,
        NEW.actor_session_id,NEW.key_version,NEW.token_digest,NEW.created_at,NEW.expires_at) IS DISTINCT FROM
       (OLD.id,OLD.service_id,OLD.operation_id,OLD.request_digest,OLD.recipient_ref,OLD.purpose,OLD.actor_user_id,
        OLD.actor_session_id,OLD.key_version,OLD.token_digest,OLD.created_at,OLD.expires_at)
       OR NEW.version<OLD.version OR (OLD.status<>'ACTIVE' AND NEW IS DISTINCT FROM OLD) THEN
        RAISE EXCEPTION 'AUTH_INVITATION_ORIGINAL_BINDING_IMMUTABLE' USING ERRCODE='23514';
    END IF;
    IF NEW.status IS DISTINCT FROM OLD.status THEN NEW.version:=GREATEST(NEW.version,OLD.version+1); END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER auth_invitation_original_binding BEFORE UPDATE ON public.auth_invitations FOR EACH ROW EXECUTE FUNCTION public.auth_invitation_original_binding();

CREATE FUNCTION public.auth_delivery_source_immutable() RETURNS trigger
LANGUAGE plpgsql SET search_path TO pg_catalog,public AS $$
BEGIN
    IF TG_TABLE_NAME='auth_consent_revocation_outbox' THEN
        IF (NEW.id,NEW.consent_id,NEW.user_id,NEW.group_id,NEW.client_id,NEW.consent_generation,NEW.event_kind,NEW.created_at)
          IS DISTINCT FROM (OLD.id,OLD.consent_id,OLD.user_id,OLD.group_id,OLD.client_id,OLD.consent_generation,OLD.event_kind,OLD.created_at) THEN
            RAISE EXCEPTION 'AUTH_CONSENT_DELIVERY_SOURCE_IMMUTABLE' USING ERRCODE='23514';
        END IF;
    ELSIF (NEW.id,NEW.user_id,NEW.event_kind,NEW.created_at) IS DISTINCT FROM (OLD.id,OLD.user_id,OLD.event_kind,OLD.created_at) THEN
        RAISE EXCEPTION 'AUTH_SECURITY_DELIVERY_SOURCE_IMMUTABLE' USING ERRCODE='23514';
    END IF;
    IF NEW.attempt_count<OLD.attempt_count THEN RAISE EXCEPTION 'AUTH_DELIVERY_ATTEMPT_REGRESSION' USING ERRCODE='23514'; END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER auth_consent_delivery_source_immutable BEFORE UPDATE ON public.auth_consent_revocation_outbox FOR EACH ROW EXECUTE FUNCTION public.auth_delivery_source_immutable();
CREATE TRIGGER auth_security_delivery_source_immutable BEFORE UPDATE ON public.auth_security_notification_outbox FOR EACH ROW EXECUTE FUNCTION public.auth_delivery_source_immutable();

-- Readonly runtime facts use exact owner locks; neither fence grants writes.
CREATE FUNCTION public.identity_delivery_target_active(target_id uuid,expected_version bigint,expected_purpose text,client_ref uuid,recipient_ref text,channel_purpose text) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path TO pg_catalog,public AS $$
BEGIN
    PERFORM 1 FROM public.auth_delivery_targets target WHERE target.id=$1 AND target.version=$2 AND target.purpose=$3
      AND target.client_id IS NOT DISTINCT FROM $4 AND target.recipient_ref IS NOT DISTINCT FROM $5 AND target.channel_purpose IS NOT DISTINCT FROM $6
      AND target.status='ACTIVE' AND target.verified_at<=clock_timestamp() AND target.expires_at>clock_timestamp() FOR SHARE OF target;
    IF NOT FOUND THEN RETURN false; END IF;
    RETURN EXISTS(SELECT 1 FROM public.auth_delivery_targets target WHERE target.id=$1 AND target.version=$2 AND target.purpose=$3
      AND target.client_id IS NOT DISTINCT FROM $4 AND target.recipient_ref IS NOT DISTINCT FROM $5 AND target.channel_purpose IS NOT DISTINCT FROM $6
      AND target.status='ACTIVE' AND target.verified_at<=clock_timestamp() AND target.expires_at>clock_timestamp());
END $$;
CREATE FUNCTION public.identity_invitation_verification_active(verification_id uuid,expected_verification_version bigint,invitation_id uuid,target_id uuid,target_version bigint,recipient_ref text,purpose text) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path TO pg_catalog,public AS $$
BEGIN
    IF NOT public.identity_delivery_target_active($4,$5,'VERIFIED_CHANNEL',NULL,$6,$7) THEN RETURN false; END IF;
    PERFORM 1 FROM public.auth_invitation_verifications verification WHERE verification.id=$1 AND verification.version=$2
      AND verification.invitation_id=$3 AND verification.delivery_target_id=$4 AND verification.delivery_target_version=$5
      AND verification.recipient_ref=$6 AND verification.purpose=$7 AND verification.status='ACTIVE'
      AND verification.verified_at<=clock_timestamp() AND verification.expires_at>clock_timestamp() FOR SHARE OF verification;
    IF NOT FOUND THEN RETURN false; END IF;
    RETURN EXISTS(SELECT 1 FROM public.auth_invitation_verifications verification
      JOIN public.auth_delivery_targets target ON target.id=verification.delivery_target_id
      JOIN public.auth_invitations invitation ON invitation.id=verification.invitation_id
      JOIN public.auth_delivery_jobs job ON job.invitation_id=invitation.id AND job.kind='INVITATION'
        AND job.target_id=target.id AND job.target_version=target.version
      WHERE verification.id=$1 AND verification.version=$2 AND verification.invitation_id=$3
        AND verification.delivery_target_id=$4 AND verification.delivery_target_version=$5 AND verification.recipient_ref=$6 AND verification.purpose=$7
        AND verification.status='ACTIVE' AND verification.verified_at<=clock_timestamp() AND verification.expires_at>clock_timestamp()
        AND target.version=$5 AND target.status='ACTIVE' AND target.purpose='VERIFIED_CHANNEL'
        AND target.recipient_ref=$6 AND target.channel_purpose=$7 AND target.verified_at<=clock_timestamp() AND target.expires_at>clock_timestamp()
        AND invitation.recipient_ref=$6 AND invitation.purpose=$7 AND invitation.status='ACTIVE' AND invitation.expires_at>clock_timestamp());
END $$;

-- Catalog-only closure check: neither runtime gains access to the other's rows.
-- The canonical fresh baseline also rejects any reintroduced legacy authority.
CREATE FUNCTION public.identity_phase20_schema_ready() RETURNS boolean
LANGUAGE sql STABLE SET search_path TO pg_catalog,public AS $$
    WITH auth_objects(name,required) AS (VALUES
      ('auth_rp_sessions',ARRAY['SELECT','INSERT']),('auth_logout_command_receipts',ARRAY['SELECT','INSERT']),
      ('auth_delivery_targets',ARRAY['SELECT']),('auth_invitation_verifications',ARRAY['SELECT']),
      ('auth_invitation_activation_receipts',ARRAY['SELECT','INSERT']),('auth_delivery_jobs',ARRAY['SELECT','INSERT','UPDATE']),
      ('auth_consent_revocation_outbox',ARRAY['SELECT','INSERT','UPDATE']),('auth_security_notification_outbox',ARRAY['SELECT','INSERT','UPDATE'])
    ), internal_functions(name) AS (VALUES
      ('auth_phase20_receipt_immutable'),('auth_rp_session_source_check'),('auth_delivery_target_generation'),
      ('auth_client_logout_target_check'),('auth_logout_receipt_source_check'),('auth_invitation_activation_source_check'),
      ('auth_delivery_derived_id'),('auth_enqueue_logout'),('auth_grant_revoked_delivery'),('auth_sso_revocation_delivery'),
      ('auth_account_revocation_delivery'),('auth_client_revocation_delivery'),('auth_delivery_job_binding'),('auth_consent_delivery_expand'),
      ('auth_security_delivery_expand'),('auth_delivery_source_summary'),('auth_invitation_verification_generation'),('auth_delivery_source_immutable'),('auth_invitation_original_binding')
    )
    SELECT EXISTS(SELECT 1 FROM public.identity_deployment_state deployment WHERE deployment.singleton
      AND NOT EXISTS(SELECT 1 FROM pg_class relation JOIN pg_namespace namespace ON namespace.oid=relation.relnamespace
        WHERE namespace.nspname='public' AND relation.relkind IN ('r','p','v','m','f')
          AND (relation.relname ~ '^(oauth_|oidc_)'
            OR relation.relname IN ('identity_oidc_admin_receipts','platform_oidc_command_intents')))
      AND NOT EXISTS(SELECT 1 FROM unnest(ARRAY['auth_provider_settings','auth_signing_keys']) object(name)
        WHERE to_regclass('public.'||object.name) IS NULL
          OR (SELECT relowner FROM pg_class WHERE oid=to_regclass('public.'||object.name))
            IS DISTINCT FROM (SELECT oid FROM pg_roles WHERE rolname=deployment.identity_owner)
          OR EXISTS(SELECT 1 FROM unnest(ARRAY['SELECT','INSERT','UPDATE','DELETE']) permission(name)
            WHERE NOT has_table_privilege(deployment.identity_runtime,'public.'||object.name,permission.name))
          OR has_any_column_privilege(deployment.platform_runtime,'public.'||object.name,'SELECT,INSERT,UPDATE,REFERENCES')
          OR has_table_privilege(deployment.platform_runtime,'public.'||object.name,'DELETE,TRUNCATE,TRIGGER'))
      AND EXISTS(SELECT 1 FROM pg_class receipt WHERE receipt.oid=to_regclass('public.identity_auth_command_receipts')
        AND receipt.relowner=(SELECT oid FROM pg_roles WHERE rolname=deployment.identity_owner)
        AND has_table_privilege(deployment.identity_runtime,receipt.oid,'SELECT')
        AND has_table_privilege(deployment.identity_runtime,receipt.oid,'INSERT')
        AND NOT has_any_column_privilege(deployment.identity_runtime,receipt.oid,'UPDATE,REFERENCES')
        AND NOT has_table_privilege(deployment.identity_runtime,receipt.oid,'DELETE,TRUNCATE,TRIGGER')
        AND NOT has_any_column_privilege(deployment.platform_runtime,receipt.oid,'SELECT,INSERT,UPDATE,REFERENCES')
        AND NOT has_table_privilege(deployment.platform_runtime,receipt.oid,'DELETE,TRUNCATE,TRIGGER'))
      AND EXISTS(SELECT 1 FROM pg_class intent WHERE intent.oid=to_regclass('public.platform_auth_command_intents')
        AND intent.relowner=(SELECT oid FROM pg_roles WHERE rolname=deployment.platform_owner)
        AND has_table_privilege(deployment.platform_runtime,intent.oid,'SELECT')
        AND has_table_privilege(deployment.platform_runtime,intent.oid,'INSERT')
        AND NOT has_any_column_privilege(deployment.platform_runtime,intent.oid,'UPDATE,REFERENCES')
        AND NOT has_table_privilege(deployment.platform_runtime,intent.oid,'DELETE,TRUNCATE,TRIGGER')
        AND NOT has_any_column_privilege(deployment.identity_runtime,intent.oid,'SELECT,INSERT,UPDATE,REFERENCES')
        AND NOT has_table_privilege(deployment.identity_runtime,intent.oid,'DELETE,TRUNCATE,TRIGGER'))
      AND NOT EXISTS(SELECT 1 FROM auth_objects object WHERE to_regclass('public.'||object.name) IS NULL
        OR (SELECT relowner FROM pg_class WHERE oid=to_regclass('public.'||object.name))
           IS DISTINCT FROM (SELECT oid FROM pg_roles WHERE rolname=deployment.identity_owner)
        OR EXISTS(SELECT 1 FROM unnest(object.required) permission(name) WHERE NOT has_table_privilege(deployment.identity_runtime,'public.'||object.name,permission.name))
        OR has_any_column_privilege(deployment.identity_runtime,'public.'||object.name,'REFERENCES')
        OR has_table_privilege(deployment.identity_runtime,'public.'||object.name,'DELETE,TRUNCATE,TRIGGER')
        OR (NOT 'INSERT'=ANY(object.required) AND has_any_column_privilege(deployment.identity_runtime,'public.'||object.name,'INSERT'))
        OR (NOT 'UPDATE'=ANY(object.required) AND has_any_column_privilege(deployment.identity_runtime,'public.'||object.name,'UPDATE'))
        OR has_any_column_privilege(deployment.platform_runtime,'public.'||object.name,'SELECT,INSERT,UPDATE,REFERENCES')
        OR has_table_privilege(deployment.platform_runtime,'public.'||object.name,'DELETE,TRUNCATE,TRIGGER'))
      AND NOT EXISTS(SELECT 1 FROM unnest(ARRAY['platform_backchannel_logout_receipts','platform_backchannel_logout_sources']) object(name)
        WHERE to_regclass('public.'||object.name) IS NULL
          OR (SELECT relowner FROM pg_class WHERE oid=to_regclass('public.'||object.name))
            IS DISTINCT FROM (SELECT oid FROM pg_roles WHERE rolname=deployment.platform_owner)
          OR NOT has_table_privilege(deployment.platform_runtime,'public.'||object.name,'SELECT')
          OR NOT has_table_privilege(deployment.platform_runtime,'public.'||object.name,'INSERT')
          OR has_any_column_privilege(deployment.platform_runtime,'public.'||object.name,'UPDATE,REFERENCES')
          OR has_table_privilege(deployment.platform_runtime,'public.'||object.name,'DELETE,TRUNCATE,TRIGGER')
          OR has_any_column_privilege(deployment.identity_runtime,'public.'||object.name,'SELECT,INSERT,UPDATE,REFERENCES')
          OR has_table_privilege(deployment.identity_runtime,'public.'||object.name,'DELETE,TRUNCATE,TRIGGER'))
      AND NOT EXISTS(SELECT 1 FROM internal_functions object WHERE NOT EXISTS(SELECT 1 FROM pg_proc procedure JOIN pg_namespace namespace ON namespace.oid=procedure.pronamespace
        WHERE namespace.nspname='public' AND procedure.proname=object.name
          AND procedure.proowner=(SELECT oid FROM pg_roles WHERE rolname=deployment.identity_owner)
          AND NOT has_function_privilege(deployment.identity_runtime,procedure.oid,'EXECUTE')
          AND NOT has_function_privilege(deployment.platform_runtime,procedure.oid,'EXECUTE')))
      AND NOT EXISTS(SELECT 1 FROM unnest(ARRAY['public.identity_delivery_target_active(uuid,bigint,text,uuid,text,text)',
        'public.identity_invitation_verification_active(uuid,bigint,uuid,uuid,bigint,text,text)']) fence(signature)
        WHERE to_regprocedure(fence.signature) IS NULL
          OR (SELECT proowner FROM pg_proc WHERE oid=to_regprocedure(fence.signature)) IS DISTINCT FROM (SELECT oid FROM pg_roles WHERE rolname=deployment.identity_owner)
          OR NOT has_function_privilege(deployment.identity_runtime,to_regprocedure(fence.signature),'EXECUTE')
          OR has_function_privilege(deployment.platform_runtime,to_regprocedure(fence.signature),'EXECUTE'))
      AND (SELECT count(*) FROM pg_attribute WHERE attrelid='public.bff_sessions'::regclass AND attname IN('oidc_public_sid','oidc_public_sub')
        AND atttypid='text'::regtype AND attnotnull AND NOT attisdropped)=2
      AND EXISTS(SELECT 1 FROM pg_trigger WHERE tgrelid='public.bff_sessions'::regclass AND tgname='bff_public_source_immutable' AND tgenabled='O' AND NOT tgisinternal))
$$;
