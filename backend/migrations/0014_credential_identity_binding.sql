-- No historical credential is upgraded from current account/member state.
-- Nullable columns retain old rows for audit; every reader rejects unbound rows.
ALTER TABLE public.platform_sessions
    ADD COLUMN account_version bigint CHECK (account_version >= 0),
    ADD COLUMN membership_id uuid CHECK (public.is_uuid_v7(membership_id)),
    ADD COLUMN member_type text CHECK (member_type IN ('EMPLOYEE','CUSTOMER')),
    ADD COLUMN identity_version bigint CHECK (identity_version >= 1),
    ADD COLUMN origin_issuer text,
    ADD COLUMN origin_client_ref text,
    ADD COLUMN origin_client_policy_version bigint CHECK (origin_client_policy_version >= 1);

ALTER TABLE public.oauth_tokens
    ADD COLUMN account_version bigint CHECK (account_version >= 0),
    ADD COLUMN membership_id uuid CHECK (public.is_uuid_v7(membership_id)),
    ADD COLUMN member_type text CHECK (member_type IN ('EMPLOYEE','CUSTOMER')),
    ADD COLUMN identity_version bigint CHECK (identity_version >= 1),
    ADD COLUMN client_policy_version bigint CHECK (client_policy_version >= 1),
    ADD COLUMN application_policy_version bigint CHECK (application_policy_version >= 1),
    ADD COLUMN origin_issuer text,
    ADD COLUMN origin_client_ref text,
    ADD COLUMN origin_client_policy_version bigint CHECK (origin_client_policy_version >= 1),
    ADD COLUMN application_service_client_ref text;

ALTER TABLE public.oauth_authorization_codes
    ADD COLUMN account_version bigint CHECK (account_version >= 0),
    ADD COLUMN membership_id uuid CHECK (public.is_uuid_v7(membership_id)),
    ADD COLUMN member_type text CHECK (member_type IN ('EMPLOYEE','CUSTOMER')),
    ADD COLUMN identity_version bigint CHECK (identity_version >= 1),
    ADD COLUMN client_policy_version bigint CHECK (client_policy_version >= 1),
    ADD COLUMN application_id uuid CHECK (public.is_uuid_v7(application_id)),
    ADD COLUMN application_policy_version bigint CHECK (application_policy_version >= 1),
    ADD COLUMN origin_issuer text,
    ADD COLUMN origin_client_ref text,
    ADD COLUMN origin_client_policy_version bigint CHECK (origin_client_policy_version >= 1),
    ADD COLUMN platform_session_id uuid CHECK (public.is_uuid_v7(platform_session_id));

ALTER TABLE public.oauth_device_codes
    ADD COLUMN account_version bigint CHECK (account_version >= 0),
    ADD COLUMN membership_id uuid CHECK (public.is_uuid_v7(membership_id)),
    ADD COLUMN member_type text CHECK (member_type IN ('EMPLOYEE','CUSTOMER')),
    ADD COLUMN identity_version bigint CHECK (identity_version >= 1),
    ADD COLUMN client_policy_version bigint CHECK (client_policy_version >= 1),
    ADD COLUMN application_id uuid CHECK (public.is_uuid_v7(application_id)),
    ADD COLUMN application_policy_version bigint CHECK (application_policy_version >= 1),
    ADD COLUMN origin_issuer text,
    ADD COLUMN origin_client_ref text,
    ADD COLUMN origin_client_policy_version bigint CHECK (origin_client_policy_version >= 1),
    ADD COLUMN platform_session_id uuid CHECK (public.is_uuid_v7(platform_session_id));

ALTER TABLE public.oauth_authorization_requests
    ADD COLUMN client_policy_version bigint CHECK (client_policy_version >= 1),
    ADD COLUMN application_id uuid CHECK (public.is_uuid_v7(application_id)),
    ADD COLUMN application_policy_version bigint CHECK (application_policy_version >= 1);

-- A real, server-selected registration for the retained password console.
-- Empty redirects prevent this registration being used for browser OAuth.
-- The explicit UUID is stable schema seed data, not a secret or runtime issuer.
INSERT INTO public.oauth_clients
    (id,client_id,client_name,description,client_type,grant_types,scopes,
     redirect_uris,require_pkce,subject_mode,allowed_member_types)
VALUES ('019a174e-0000-7000-8000-000000000001','platform-console-legacy',
        'Legacy platform console','Server-selected employee console origin',
        'PUBLIC','authorization_code','profile','',true,'USER',ARRAY['EMPLOYEE']::text[]);

CREATE FUNCTION public.protect_legacy_console_origin() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
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
CREATE TRIGGER trg_protect_legacy_console_origin
BEFORE UPDATE OR DELETE ON public.oauth_clients FOR EACH ROW
EXECUTE FUNCTION public.protect_legacy_console_origin();

CREATE FUNCTION public.identity_binding_active(
    subject_id uuid, tenant_ref uuid, organization_ref uuid, account_generation bigint,
    membership_ref uuid, relationship_type text, identity_generation bigint)
RETURNS boolean LANGUAGE sql STABLE SET search_path = pg_catalog, public AS $$
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

CREATE FUNCTION public.platform_session_active(session_ref uuid)
RETURNS boolean LANGUAGE sql STABLE SET search_path = pg_catalog, public AS $$
    SELECT EXISTS (
        SELECT 1 FROM public.platform_sessions s
        JOIN public.oauth_clients c ON c.client_id=s.origin_client_ref
        WHERE s.id=session_ref AND public.is_uuid_v7(session_ref) AND s.revoked_at IS NULL
          AND s.expires_at>clock_timestamp()
          AND s.origin_issuer='urn:saas:legacy-platform'
          AND c.status='ENABLED' AND c.subject_mode IN ('USER','LEGACY_MIXED')
          AND c.access_policy_version=s.origin_client_policy_version
          AND s.member_type=ANY(c.allowed_member_types)
          AND public.identity_binding_active(s.user_id,s.tenant_id,s.organization_id,
                s.account_version,s.membership_id,s.member_type,s.identity_version))
$$;

-- Keep tombstone references instead of a membership FK: deleting a relation
-- must not erase the proof needed to identify and retire its credentials.
CREATE INDEX idx_platform_sessions_membership ON public.platform_sessions(membership_id);
CREATE INDEX idx_oauth_tokens_membership ON public.oauth_tokens(membership_id);
CREATE INDEX idx_oauth_codes_membership ON public.oauth_authorization_codes(membership_id);
CREATE INDEX idx_oauth_device_membership ON public.oauth_device_codes(membership_id);
CREATE INDEX idx_platform_sessions_origin ON public.platform_sessions(origin_client_ref);

CREATE FUNCTION public.revoke_membership_credentials(membership_ref uuid) RETURNS void
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
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

CREATE OR REPLACE FUNCTION public.revoke_sessions_on_membership_delete()
RETURNS trigger LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
BEGIN
    PERFORM public.revoke_membership_credentials(OLD.id);
    RETURN OLD;
END $$;

CREATE FUNCTION public.revoke_credentials_on_identity_update() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
BEGIN
    IF NEW.identity_version IS DISTINCT FROM OLD.identity_version
       OR NEW.status IS DISTINCT FROM OLD.status THEN
        PERFORM public.revoke_membership_credentials(OLD.id);
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER trg_membership_credential_generation
AFTER UPDATE ON public.memberships FOR EACH ROW
EXECUTE FUNCTION public.revoke_credentials_on_identity_update();

CREATE FUNCTION public.revoke_credentials_on_account_generation() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
BEGIN
    IF NEW.session_version < OLD.session_version THEN
        RAISE EXCEPTION 'account credential version cannot decrease' USING ERRCODE='23514';
    END IF;
    IF NEW.session_version IS DISTINCT FROM OLD.session_version
       OR (OLD.status='ENABLED' AND NEW.status<>'ENABLED') THEN
        UPDATE public.platform_sessions SET revoked_at=clock_timestamp()
            WHERE user_id=NEW.id AND revoked_at IS NULL;
        UPDATE public.platform_session_refresh_tokens r SET consumed_at=clock_timestamp()
            FROM public.platform_sessions s WHERE r.session_id=s.id
            AND s.user_id=NEW.id AND r.consumed_at IS NULL;
        UPDATE public.oauth_tokens SET revoked=true WHERE user_id=NEW.id AND NOT revoked;
        UPDATE public.oauth_authorization_codes SET consumed=true
            WHERE user_id=NEW.id AND NOT consumed;
        UPDATE public.oauth_device_codes SET status='EXPIRED'
            WHERE user_id=NEW.id AND status IN ('PENDING','APPROVED');
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER trg_account_credential_generation
AFTER UPDATE ON public.users FOR EACH ROW
EXECUTE FUNCTION public.revoke_credentials_on_account_generation();

CREATE FUNCTION public.revoke_client_policy_credentials(client_ref uuid) RETURNS void
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
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

CREATE FUNCTION public.revoke_application_policy_credentials(application_ref uuid) RETURNS void
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
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

CREATE FUNCTION public.revoke_changed_client_policy() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
BEGIN
    IF NEW.access_policy_version IS DISTINCT FROM OLD.access_policy_version THEN
        PERFORM public.revoke_client_policy_credentials(NEW.id);
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER trg_client_policy_credential_generation
AFTER UPDATE ON public.oauth_clients FOR EACH ROW
EXECUTE FUNCTION public.revoke_changed_client_policy();

CREATE FUNCTION public.revoke_changed_application_policy() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
BEGIN
    IF NEW.access_policy_version IS DISTINCT FROM OLD.access_policy_version THEN
        PERFORM public.revoke_application_policy_credentials(NEW.id);
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER trg_application_policy_credential_generation
AFTER UPDATE ON public.applications FOR EACH ROW
EXECUTE FUNCTION public.revoke_changed_application_policy();

-- Existing rows have no provable signing-time binding. Never backfill them.
UPDATE public.platform_sessions SET revoked_at=clock_timestamp() WHERE revoked_at IS NULL;
UPDATE public.platform_session_refresh_tokens SET consumed_at=clock_timestamp() WHERE consumed_at IS NULL;
UPDATE public.oauth_tokens SET revoked=true WHERE NOT revoked;
UPDATE public.oauth_authorization_codes SET consumed=true WHERE NOT consumed;
UPDATE public.oauth_authorization_requests SET consumed=true WHERE NOT consumed;
UPDATE public.oauth_device_codes SET status='EXPIRED' WHERE status IN ('PENDING','APPROVED');
