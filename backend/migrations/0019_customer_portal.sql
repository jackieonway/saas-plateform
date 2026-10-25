-- Platform-owned derived customer contexts. Authentication stores, secrets and
-- upstream tokens remain with their owners; source IDs are immutable proofs.
CREATE TABLE public.customer_portal_contexts (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    token_hash text NOT NULL UNIQUE CHECK (token_hash ~ '^[a-f0-9]{64}$'),
    issuer text NOT NULL,
    oidc_client_id uuid NOT NULL CHECK (public.is_uuid_v7(oidc_client_id)),
    oidc_grant_id uuid NOT NULL CHECK (public.is_uuid_v7(oidc_grant_id)),
    global_session_id uuid NOT NULL CHECK (public.is_uuid_v7(global_session_id)),
    user_id uuid NOT NULL CHECK (public.is_uuid_v7(user_id)),
    account_version bigint NOT NULL CHECK (account_version>=0),
    oidc_client_policy_version bigint NOT NULL CHECK (oidc_client_policy_version>=1),
    auth_time bigint NOT NULL CHECK (auth_time>0),
    source_expires_at timestamptz NOT NULL,
    profile_client_ref uuid NOT NULL CHECK (public.is_uuid_v7(profile_client_ref)),
    profile_client_policy_version bigint NOT NULL CHECK (profile_client_policy_version>=1),
    application_id uuid NOT NULL CHECK (public.is_uuid_v7(application_id)),
    application_policy_version bigint NOT NULL CHECK (application_policy_version>=1),
    tenant_id uuid NOT NULL CHECK (public.is_uuid_v7(tenant_id)),
    organization_id uuid NOT NULL CHECK (public.is_uuid_v7(organization_id)),
    membership_id uuid NOT NULL CHECK (public.is_uuid_v7(membership_id)),
    member_type text NOT NULL CHECK (member_type='CUSTOMER'),
    identity_version bigint NOT NULL CHECK (identity_version>=1),
    expires_at timestamptz NOT NULL CHECK (expires_at<=source_expires_at),
    revoked_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX idx_customer_portal_contexts_subject ON public.customer_portal_contexts(user_id,membership_id);
COMMENT ON TABLE public.customer_portal_contexts IS
    'Platform-owned customer-profile/1 contexts; source references have no cross-owner foreign keys.';

-- Authentication-owner read projection for the temporary cohost boundary. It
-- only rejects a previously verified access binding; true NEVER replaces the
-- consumer's two online AccountProofPort calls. No credential is returned and
-- only a one-way digest is accepted. SECURITY INVOKER adds no table privileges.
-- Phase 12 MUST assign this projection/authentication tables to the authority
-- owner and revoke platform access to raw authentication/credential tables.
-- The final checkpoint may remain a narrowly granted, independently audited
-- boolean projection (with hardened SECURITY DEFINER/search_path if needed),
-- or gain an equivalent internal network adapter. Either option still requires
-- online HTTP account proof first and must preserve exact original-source denial.
CREATE FUNCTION public.customer_portal_account_source_active(
    access_digest text,expected_issuer text,client_ref uuid,grant_ref uuid,
    sid_ref uuid,subject_ref uuid,account_generation bigint,policy_generation bigint,
    authentication_time bigint,source_deadline timestamptz)
RETURNS boolean LANGUAGE sql STABLE SECURITY INVOKER
SET search_path=pg_catalog,public AS $$
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
REVOKE ALL ON FUNCTION public.customer_portal_account_source_active(
    text,text,uuid,uuid,uuid,uuid,bigint,bigint,bigint,timestamptz) FROM PUBLIC;
COMMENT ON FUNCTION public.customer_portal_account_source_active(
    text,text,uuid,uuid,uuid,uuid,bigint,bigint,bigint,timestamptz) IS
    'Authentication-owner deny-only cohost projection. Non-owner runtime needs an explicit narrow EXECUTE grant; Phase12 must verify owner/grants and remove raw authentication-table access.';
