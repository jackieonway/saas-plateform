-- The OIDC provider owns these tables; legacy OAuth remains platform-owned.
CREATE TABLE public.oidc_provider_settings (
    singleton boolean PRIMARY KEY CHECK (singleton),
    issuer text NOT NULL,
    created_at timestamptz NOT NULL
);

CREATE TABLE public.oidc_clients (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    purpose text NOT NULL CHECK (purpose IN ('PLATFORM_CONSOLE','CUSTOMER_PORTAL','ACCOUNT')),
    secret_hash text NOT NULL CHECK (secret_hash ~ '^[0-9a-f]{64}$'),
    allowed_member_types text[] NOT NULL CHECK (
        cardinality(allowed_member_types) BETWEEN 1 AND 2
        AND allowed_member_types <@ ARRAY['EMPLOYEE','CUSTOMER']::text[]),
    redirect_uris text[] NOT NULL CHECK (cardinality(redirect_uris) BETWEEN 1 AND 16),
    post_logout_redirect_uris text[] NOT NULL DEFAULT '{}',
    refresh_enabled boolean NOT NULL DEFAULT false,
    status text NOT NULL CHECK (status IN ('ENABLED','DISABLED')),
    access_policy_version bigint NOT NULL CHECK (access_policy_version >= 1),
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    CHECK (purpose <> 'PLATFORM_CONSOLE' OR allowed_member_types=ARRAY['EMPLOYEE']::text[]),
    CHECK (purpose <> 'CUSTOMER_PORTAL' OR allowed_member_types=ARRAY['CUSTOMER']::text[])
);

CREATE TABLE public.oidc_authorization_requests (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    browser_hash text NOT NULL CHECK (browser_hash ~ '^[0-9a-f]{64}$'),
    csrf_hash text NOT NULL CHECK (csrf_hash ~ '^[0-9a-f]{64}$'),
    client_id uuid NOT NULL REFERENCES public.oidc_clients(id) ON DELETE CASCADE
        CHECK (public.is_uuid_v7(client_id)),
    client_policy_version bigint NOT NULL CHECK (client_policy_version >= 1),
    redirect_uri text NOT NULL,
    scope text[] NOT NULL,
    state text NOT NULL,
    nonce text NOT NULL,
    code_challenge text NOT NULL CHECK (code_challenge ~ '^[A-Za-z0-9_-]{43}$'),
    force_login boolean NOT NULL DEFAULT false,
    max_age bigint CHECK (max_age >= 0),
    session_id uuid REFERENCES public.sso_sessions(id) ON DELETE CASCADE
        CHECK (session_id IS NULL OR public.is_uuid_v7(session_id)),
    user_id uuid REFERENCES public.users(id) ON DELETE CASCADE
        CHECK (user_id IS NULL OR public.is_uuid_v7(user_id)),
    account_version bigint CHECK (account_version >= 0),
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL CHECK (expires_at > created_at),
    consumed_at timestamptz,
    CHECK ((session_id IS NULL AND user_id IS NULL AND account_version IS NULL)
        OR (session_id IS NOT NULL AND user_id IS NOT NULL AND account_version IS NOT NULL))
);
CREATE INDEX idx_oidc_requests_live ON public.oidc_authorization_requests(expires_at)
    WHERE consumed_at IS NULL;

CREATE TABLE public.oidc_grants (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    client_id uuid NOT NULL REFERENCES public.oidc_clients(id) ON DELETE CASCADE
        CHECK (public.is_uuid_v7(client_id)),
    client_policy_version bigint NOT NULL CHECK (client_policy_version >= 1),
    session_id uuid NOT NULL REFERENCES public.sso_sessions(id) ON DELETE CASCADE
        CHECK (public.is_uuid_v7(session_id)),
    user_id uuid NOT NULL REFERENCES public.users(id) ON DELETE CASCADE
        CHECK (public.is_uuid_v7(user_id)),
    account_version bigint NOT NULL CHECK (account_version >= 0),
    scope text[] NOT NULL,
    nonce text NOT NULL,
    auth_time timestamptz NOT NULL,
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL CHECK (expires_at > created_at),
    revoked_at timestamptz
);
CREATE INDEX idx_oidc_grants_client ON public.oidc_grants(client_id);
CREATE INDEX idx_oidc_grants_session ON public.oidc_grants(session_id);

CREATE TABLE public.oidc_authorization_codes (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    grant_id uuid NOT NULL REFERENCES public.oidc_grants(id) ON DELETE CASCADE
        CHECK (public.is_uuid_v7(grant_id)),
    token_hash text NOT NULL UNIQUE CHECK (token_hash ~ '^[0-9a-f]{64}$'),
    redirect_uri text NOT NULL,
    code_challenge text NOT NULL CHECK (code_challenge ~ '^[A-Za-z0-9_-]{43}$'),
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL CHECK (expires_at > created_at),
    consumed_at timestamptz
);
CREATE TABLE public.oidc_access_tokens (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    grant_id uuid NOT NULL REFERENCES public.oidc_grants(id) ON DELETE CASCADE
        CHECK (public.is_uuid_v7(grant_id)),
    token_hash text NOT NULL UNIQUE CHECK (token_hash ~ '^[0-9a-f]{64}$'),
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL CHECK (expires_at > created_at),
    revoked_at timestamptz
);
CREATE TABLE public.oidc_refresh_tokens (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    grant_id uuid NOT NULL REFERENCES public.oidc_grants(id) ON DELETE CASCADE
        CHECK (public.is_uuid_v7(grant_id)),
    token_hash text NOT NULL UNIQUE CHECK (token_hash ~ '^[0-9a-f]{64}$'),
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL CHECK (expires_at > created_at),
    used_at timestamptz,
    revoked_at timestamptz
);
CREATE INDEX idx_oidc_access_grant ON public.oidc_access_tokens(grant_id);
CREATE INDEX idx_oidc_refresh_grant ON public.oidc_refresh_tokens(grant_id);

-- Private material is provisioned outside the database. Publication precedes
-- activation; retired/revoked kids cannot be reused for different material.
CREATE TABLE public.oidc_signing_keys (
    kid text PRIMARY KEY CHECK (kid ~ '^[A-Za-z0-9_-]{1,80}$'),
    n text NOT NULL,
    e text NOT NULL CHECK (e='AQAB'),
    published_at timestamptz NOT NULL,
    active boolean NOT NULL DEFAULT false,
    activated_at timestamptz,
    retired_at timestamptz,
    revoked_at timestamptz,
    last_signed_exp timestamptz
);
CREATE UNIQUE INDEX idx_oidc_one_active_key ON public.oidc_signing_keys((true))
    WHERE active;

-- Client policy is a single-owner generation. No membership lookup or global
-- account change is involved. The read path also checks the current generation.
CREATE FUNCTION public.revoke_oidc_on_client_policy() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
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
CREATE TRIGGER trg_oidc_client_policy AFTER UPDATE ON public.oidc_clients
FOR EACH ROW EXECUTE FUNCTION public.revoke_oidc_on_client_policy();

CREATE FUNCTION public.revoke_oidc_on_sso_revocation() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
BEGIN
    IF NEW.revoked_at IS NOT NULL AND OLD.revoked_at IS NULL THEN
        UPDATE public.oidc_grants SET revoked_at=clock_timestamp()
        WHERE session_id=NEW.id AND revoked_at IS NULL;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER trg_oidc_sso_revocation AFTER UPDATE ON public.sso_sessions
FOR EACH ROW EXECUTE FUNCTION public.revoke_oidc_on_sso_revocation();

CREATE FUNCTION public.oidc_key_material_immutable() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
BEGIN
    IF (NEW.kid,NEW.n,NEW.e) IS DISTINCT FROM (OLD.kid,OLD.n,OLD.e) THEN
        RAISE EXCEPTION 'OIDC kid and public material are immutable' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER trg_oidc_key_material BEFORE UPDATE ON public.oidc_signing_keys
FOR EACH ROW EXECUTE FUNCTION public.oidc_key_material_immutable();
