-- Identity expansion only. The user/organization unique key intentionally
-- remains until the independently validated Phase 6 contraction migration.
ALTER TABLE public.memberships
    ADD COLUMN member_type varchar(20) NOT NULL DEFAULT 'EMPLOYEE',
    ADD COLUMN status varchar(20) NOT NULL DEFAULT 'ENABLED',
    ADD COLUMN identity_version bigint NOT NULL DEFAULT 1,
    ADD COLUMN updated_at timestamptz NOT NULL DEFAULT now(),
    ADD CONSTRAINT memberships_member_type_check
        CHECK (member_type IN ('EMPLOYEE', 'CUSTOMER')),
    ADD CONSTRAINT memberships_status_check
        CHECK (status IN ('ENABLED', 'DISABLED')),
    ADD CONSTRAINT memberships_identity_version_check CHECK (identity_version >= 1),
    ADD CONSTRAINT memberships_customer_profile_check
        CHECK (member_type <> 'CUSTOMER'
               OR (department_id IS NULL AND COALESCE(position, '') = '')),
    ADD CONSTRAINT memberships_user_organization_type_key
        UNIQUE (user_id, organization_id, member_type);

CREATE FUNCTION public.protect_membership_identity() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
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

CREATE TRIGGER trg_membership_identity_immutable
BEFORE UPDATE ON public.memberships FOR EACH ROW
EXECUTE FUNCTION public.protect_membership_identity();

-- Phase 1 keeps every customer issuance entry disabled. Existing unbound
-- platform sessions therefore represent employees only. Deleting a customer
-- fixture must never revoke that account's employee session. Phase 2 replaces
-- this temporary employee compatibility branch with exact membership binding.
CREATE OR REPLACE FUNCTION public.revoke_sessions_on_membership_delete()
RETURNS trigger LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
BEGIN
    IF OLD.member_type = 'EMPLOYEE' THEN
        UPDATE public.platform_sessions SET revoked_at = now()
        WHERE user_id = OLD.user_id AND organization_id = OLD.organization_id
          AND revoked_at IS NULL;
        UPDATE public.platform_session_refresh_tokens r SET consumed_at = now()
        FROM public.platform_sessions s
        WHERE r.session_id = s.id AND s.user_id = OLD.user_id
          AND s.organization_id = OLD.organization_id AND r.consumed_at IS NULL;
        UPDATE public.oauth_tokens SET revoked = true
        WHERE user_id = OLD.user_id AND organization_id = OLD.organization_id
          AND NOT revoked;
        UPDATE public.oauth_authorization_codes SET consumed = true
        WHERE user_id = OLD.user_id AND organization_id = OLD.organization_id
          AND NOT consumed;
        UPDATE public.oauth_device_codes SET status = 'EXPIRED'
        WHERE user_id = OLD.user_id AND organization_id = OLD.organization_id
          AND status IN ('PENDING', 'APPROVED');
    END IF;
    RETURN OLD;
END $$;

-- Canonical order is EMPLOYEE, CUSTOMER. Exact array equality rejects NULL
-- elements, duplicate elements, extra dimensions and nonstandard lower bounds.
CREATE FUNCTION public.valid_member_type_set(member_types text[], allow_empty boolean)
RETURNS boolean LANGUAGE sql IMMUTABLE PARALLEL SAFE
SET search_path = pg_catalog AS $$
    SELECT COALESCE(member_types = ARRAY['EMPLOYEE']::text[]
        OR member_types = ARRAY['CUSTOMER']::text[]
        OR member_types = ARRAY['EMPLOYEE', 'CUSTOMER']::text[]
        OR (allow_empty AND member_types = ARRAY[]::text[]), false)
$$;

ALTER TABLE public.applications
    ADD COLUMN allowed_member_types text[] NOT NULL DEFAULT ARRAY['EMPLOYEE']::text[],
    ADD COLUMN access_policy_version bigint NOT NULL DEFAULT 1,
    ADD CONSTRAINT applications_allowed_member_types_check
        CHECK (public.valid_member_type_set(allowed_member_types, false)),
    ADD CONSTRAINT applications_access_policy_version_check
        CHECK (access_policy_version >= 1);

-- Existing grant contracts are classified without adding a grant or permitting
-- CUSTOMER. New writers must explicitly register machine purpose themselves.
CREATE FUNCTION public.oauth_client_grant_set(grants text[]) RETURNS text[]
LANGUAGE sql IMMUTABLE PARALLEL SAFE SET search_path = pg_catalog AS $$
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

ALTER TABLE public.oauth_clients
    ADD COLUMN subject_mode varchar(20) NOT NULL DEFAULT 'USER',
    ADD COLUMN allowed_member_types text[] NOT NULL DEFAULT ARRAY['EMPLOYEE']::text[],
    ADD COLUMN access_policy_version bigint NOT NULL DEFAULT 1;

UPDATE public.oauth_clients
SET subject_mode = CASE
        WHEN public.oauth_client_grant_set(string_to_array(grant_types, ','))
             = ARRAY['client_credentials']::text[] THEN 'MACHINE'
        WHEN 'client_credentials' = ANY(public.oauth_client_grant_set(string_to_array(grant_types, ',')))
             THEN 'LEGACY_MIXED'
        ELSE 'USER' END,
    allowed_member_types = CASE
        WHEN public.oauth_client_grant_set(string_to_array(grant_types, ','))
             = ARRAY['client_credentials']::text[] THEN ARRAY[]::text[]
        ELSE ARRAY['EMPLOYEE']::text[] END;

ALTER TABLE public.oauth_clients
    ADD CONSTRAINT oauth_clients_subject_mode_check
        CHECK (subject_mode IN ('USER', 'MACHINE', 'LEGACY_MIXED')),
    ADD CONSTRAINT oauth_clients_access_policy_version_check
        CHECK (access_policy_version >= 1),
    ADD CONSTRAINT oauth_clients_allowed_member_types_check
        CHECK (public.valid_member_type_set(allowed_member_types, subject_mode = 'MACHINE')
               AND ((subject_mode = 'MACHINE') = (cardinality(allowed_member_types) = 0))),
    ADD CONSTRAINT oauth_clients_subject_grants_check CHECK (
        (subject_mode = 'USER'
         AND NOT ('client_credentials' = ANY(public.oauth_client_grant_set(string_to_array(grant_types, ',')))))
        OR (subject_mode = 'MACHINE' AND client_type = 'CONFIDENTIAL'
            AND public.oauth_client_grant_set(string_to_array(grant_types, ','))
                = ARRAY['client_credentials']::text[])
        OR (subject_mode = 'LEGACY_MIXED' AND client_type = 'CONFIDENTIAL'
            AND 'client_credentials' = ANY(public.oauth_client_grant_set(string_to_array(grant_types, ',')))
            AND cardinality(public.oauth_client_grant_set(string_to_array(grant_types, ','))) > 1));

CREATE FUNCTION public.protect_oauth_client_identity_policy() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
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

CREATE TRIGGER trg_oauth_client_identity_policy
BEFORE INSERT OR UPDATE ON public.oauth_clients FOR EACH ROW
EXECUTE FUNCTION public.protect_oauth_client_identity_policy();

CREATE FUNCTION public.protect_application_identity_policy() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
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

CREATE TRIGGER trg_application_identity_policy
BEFORE UPDATE ON public.applications FOR EACH ROW
EXECUTE FUNCTION public.protect_application_identity_policy();

COMMENT ON COLUMN public.memberships.member_type IS
    'Immutable relationship type; a second identity needs a different membership row.';
COMMENT ON COLUMN public.memberships.identity_version IS
    'Monotonic identity generation; lifecycle status changes increment it transactionally.';
COMMENT ON COLUMN public.oauth_clients.subject_mode IS
    'USER: nonempty member types; MACHINE: empty member types and machine grant only; LEGACY_MIXED: pre-expansion registered clients only.';
COMMENT ON COLUMN public.oauth_clients.allowed_member_types IS
    'Canonical member admission set. Empty is permitted only for MACHINE, never a user wildcard.';
COMMENT ON COLUMN public.applications.allowed_member_types IS
    'Nonempty canonical member admission set; defaults to EMPLOYEE without CUSTOMER widening.';
