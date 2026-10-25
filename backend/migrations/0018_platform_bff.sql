-- Platform BFF owns console_* and bff_sessions. The authentication owner keeps
-- oidc_account_logout_receipts and exposes read-only account proof below.
CREATE TABLE public.console_browsers (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    token_hash text NOT NULL UNIQUE CHECK (token_hash ~ '^[0-9a-f]{64}$'),
    generation bigint NOT NULL CHECK (generation >= 0),
    current_session_id uuid CHECK (current_session_id IS NULL OR public.is_uuid_v7(current_session_id)),
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL
);
CREATE TABLE public.console_login_flows (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    browser_id uuid NOT NULL REFERENCES public.console_browsers(id) ON DELETE CASCADE
        CHECK (public.is_uuid_v7(browser_id)),
    browser_generation bigint NOT NULL CHECK (browser_generation >= 0),
    state_hash text NOT NULL UNIQUE CHECK (state_hash ~ '^[0-9a-f]{64}$'),
    nonce_hash text NOT NULL CHECK (nonce_hash ~ '^[0-9a-f]{64}$'),
    verifier_ciphertext text NOT NULL,
    return_path text NOT NULL,
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL CHECK (expires_at>created_at),
    consumed_at timestamptz
);
CREATE INDEX idx_console_login_flows_expiry ON public.console_login_flows(expires_at);

CREATE TABLE public.bff_sessions (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    browser_id uuid NOT NULL REFERENCES public.console_browsers(id) ON DELETE CASCADE
        CHECK (public.is_uuid_v7(browser_id)),
    token_hash text NOT NULL UNIQUE CHECK (token_hash ~ '^[0-9a-f]{64}$'),
    csrf_hash text NOT NULL CHECK (csrf_hash ~ '^[0-9a-f]{64}$'),
    user_id uuid NOT NULL REFERENCES public.users(id) ON DELETE CASCADE CHECK (public.is_uuid_v7(user_id)),
    account_version bigint NOT NULL CHECK (account_version >= 0),
    global_session_id uuid NOT NULL CHECK (public.is_uuid_v7(global_session_id)),
    oidc_client_id uuid NOT NULL CHECK (public.is_uuid_v7(oidc_client_id)),
    oidc_grant_id uuid NOT NULL CHECK (public.is_uuid_v7(oidc_grant_id)),
    oidc_client_policy_version bigint NOT NULL CHECK (oidc_client_policy_version>=1),
    issuer text NOT NULL,
    auth_time bigint NOT NULL CHECK (auth_time>0),
    nonce_hash text NOT NULL CHECK (nonce_hash ~ '^[0-9a-f]{64}$'),
    access_ciphertext text NOT NULL,
    refresh_ciphertext text NOT NULL,
    access_expires_at timestamptz NOT NULL,
    version bigint NOT NULL CHECK (version>=1),
    selected_session_id uuid CHECK (selected_session_id IS NULL OR public.is_uuid_v7(selected_session_id)),
    platform_refresh_ciphertext text,
    refresh_lease_id uuid CHECK (refresh_lease_id IS NULL OR public.is_uuid_v7(refresh_lease_id)),
    refresh_started_at timestamptz,
    refresh_uncertain boolean NOT NULL DEFAULT false,
    logout_request_id uuid CHECK (logout_request_id IS NULL OR public.is_uuid_v7(logout_request_id)),
    logout_scope text CHECK (logout_scope IN ('GLOBAL','PLATFORM')),
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL CHECK (expires_at>created_at),
    revoked_at timestamptz,
    CHECK ((logout_request_id IS NULL)=(logout_scope IS NULL)),
    CHECK ((refresh_lease_id IS NULL)=(refresh_started_at IS NULL))
);
CREATE INDEX idx_bff_sessions_user ON public.bff_sessions(user_id);
CREATE INDEX idx_bff_sessions_global ON public.bff_sessions(global_session_id);
ALTER TABLE public.platform_sessions
    ADD COLUMN bff_session_id uuid REFERENCES public.bff_sessions(id) ON DELETE CASCADE
        CHECK (bff_session_id IS NULL OR public.is_uuid_v7(bff_session_id)),
    ADD COLUMN bff_context_version bigint CHECK (bff_context_version IS NULL OR bff_context_version>=1),
    ADD CONSTRAINT platform_bff_binding_pair CHECK ((bff_session_id IS NULL)=(bff_context_version IS NULL));
CREATE INDEX idx_platform_sessions_bff ON public.platform_sessions(bff_session_id);

CREATE TABLE public.oidc_account_logout_receipts (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    client_id uuid NOT NULL REFERENCES public.oidc_clients(id) ON DELETE CASCADE CHECK (public.is_uuid_v7(client_id)),
    grant_id uuid NOT NULL REFERENCES public.oidc_grants(id) ON DELETE CASCADE CHECK (public.is_uuid_v7(grant_id)),
    user_id uuid NOT NULL REFERENCES public.users(id) ON DELETE CASCADE CHECK (public.is_uuid_v7(user_id)),
    token_hash text NOT NULL CHECK (token_hash ~ '^[0-9a-f]{64}$'),
    account_version bigint NOT NULL CHECK (account_version>=0),
    created_at timestamptz NOT NULL
);
CREATE INDEX idx_oidc_account_logout_receipts_user ON public.oidc_account_logout_receipts(user_id);

-- Explicit same-database authentication read projection. Phase 12 must give it
-- a single owner and replace cross-service table access with the internal port.
CREATE FUNCTION public.bff_oidc_proof_active(subject_ref uuid,sid_ref uuid,grant_ref uuid,
    account_generation bigint,client_ref uuid,policy_generation bigint,expected_issuer text)
RETURNS boolean LANGUAGE sql STABLE SET search_path=pg_catalog,public AS $$
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
LANGUAGE sql STABLE SET search_path=pg_catalog,public AS $$
    SELECT EXISTS(SELECT 1 FROM public.bff_sessions b
        JOIN public.console_browsers browser ON browser.id=b.browser_id
          AND browser.current_session_id=b.id AND browser.expires_at>clock_timestamp()
        WHERE b.id=session_ref AND public.is_uuid_v7(session_ref)
          AND b.revoked_at IS NULL AND b.expires_at>clock_timestamp()
          AND NOT b.refresh_uncertain AND b.logout_request_id IS NULL
          AND public.bff_oidc_proof_active(b.user_id,b.global_session_id,b.oidc_grant_id,
            b.account_version,b.oidc_client_id,b.oidc_client_policy_version,b.issuer))
$$;

CREATE FUNCTION public.platform_source_active(expected_issuer text,client_ref text,policy_version bigint,member_type text)
RETURNS boolean LANGUAGE sql STABLE SET search_path=pg_catalog,public AS $$
    SELECT CASE WHEN expected_issuer='urn:saas:legacy-platform' THEN EXISTS(
        SELECT 1 FROM public.oauth_clients c WHERE c.client_id=client_ref
          AND c.status='ENABLED' AND c.subject_mode IN ('USER','LEGACY_MIXED')
          AND c.access_policy_version=policy_version AND member_type=ANY(c.allowed_member_types))
    ELSE EXISTS(SELECT 1 FROM public.oidc_clients c JOIN public.oidc_provider_settings p
        ON p.singleton AND p.issuer=expected_issuer WHERE c.id::text=client_ref
          AND c.status='ENABLED' AND c.access_policy_version=policy_version
          AND c.purpose='PLATFORM_CONSOLE' AND member_type='EMPLOYEE'
          AND c.allowed_member_types=ARRAY['EMPLOYEE']::text[]) END
$$;

CREATE OR REPLACE FUNCTION public.platform_session_active(session_ref uuid)
RETURNS boolean LANGUAGE sql STABLE SET search_path=pg_catalog,public AS $$
    SELECT EXISTS(SELECT 1 FROM public.platform_sessions s
        WHERE s.id=session_ref AND public.is_uuid_v7(session_ref) AND s.revoked_at IS NULL
          AND s.expires_at>clock_timestamp()
          AND public.platform_source_active(s.origin_issuer,s.origin_client_ref,
            s.origin_client_policy_version,s.member_type)
          AND ((s.bff_session_id IS NULL AND s.origin_issuer='urn:saas:legacy-platform')
            OR EXISTS(SELECT 1 FROM public.bff_sessions b WHERE b.id=s.bff_session_id
                AND public.bff_session_active(b.id) AND b.user_id=s.user_id
                AND b.account_version=s.account_version AND b.selected_session_id=s.id
                AND b.version=s.bff_context_version AND b.issuer=s.origin_issuer
                AND b.oidc_client_id::text=s.origin_client_ref
                AND b.oidc_client_policy_version=s.origin_client_policy_version))
          AND public.identity_binding_active(s.user_id,s.tenant_id,s.organization_id,
            s.account_version,s.membership_id,s.member_type,s.identity_version))
$$;
