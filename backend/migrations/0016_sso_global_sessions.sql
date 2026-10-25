-- Account authentication is independent of platform memberships and OAuth policy.
CREATE TABLE public.sso_sessions (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    user_id uuid NOT NULL REFERENCES public.users(id) ON DELETE CASCADE
        CHECK (public.is_uuid_v7(user_id)),
    token_hash text NOT NULL UNIQUE CHECK (token_hash ~ '^[0-9a-f]{64}$'),
    csrf_hash text NOT NULL CHECK (csrf_hash ~ '^[0-9a-f]{64}$'),
    account_version bigint NOT NULL CHECK (account_version >= 0),
    authenticated_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL CHECK (expires_at > authenticated_at),
    revoked_at timestamptz
);
CREATE INDEX idx_sso_sessions_account_active ON public.sso_sessions(user_id, expires_at)
    WHERE revoked_at IS NULL;

CREATE TABLE public.sso_login_challenges (
    id uuid PRIMARY KEY CHECK (public.is_uuid_v7(id)),
    token_hash text NOT NULL UNIQUE CHECK (token_hash ~ '^[0-9a-f]{64}$'),
    csrf_hash text NOT NULL CHECK (csrf_hash ~ '^[0-9a-f]{64}$'),
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL CHECK (expires_at > created_at),
    consumed_at timestamptz
);
CREATE INDEX idx_sso_login_challenges_expiry ON public.sso_login_challenges(expires_at);

-- Covers existing account-owner reset/kick/ban entry points, including legacy
-- commands without an SSO sid. Membership-only revocation never reaches here.
CREATE FUNCTION public.revoke_sso_on_account_generation() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
BEGIN
    IF NEW.session_version IS DISTINCT FROM OLD.session_version
       OR (OLD.status='ENABLED' AND NEW.status<>'ENABLED') THEN
        UPDATE public.sso_sessions SET revoked_at=clock_timestamp()
        WHERE user_id=NEW.id AND revoked_at IS NULL;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER trg_account_sso_generation
AFTER UPDATE ON public.users FOR EACH ROW
EXECUTE FUNCTION public.revoke_sso_on_account_generation();
