-- Application identity / permissions remain in applications and the existing grant tables.
-- Releases pin an immutable version directory. Only preflight state can be updated.
ALTER TABLE applications ADD COLUMN runtime_mode VARCHAR(10) NOT NULL DEFAULT 'IFRAME'
    CHECK (runtime_mode IN ('IFRAME', 'TRUSTED'));
-- Platform operators explicitly approve TRUSTED through the management API.

CREATE FUNCTION revoke_app_tokens_on_runtime_mode_change() RETURNS trigger AS $$
BEGIN
    IF OLD.runtime_mode IS DISTINCT FROM NEW.runtime_mode THEN
        UPDATE oauth_tokens SET revoked = true
        WHERE token_use = 'app' AND application_id = NEW.id AND revoked = false;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER trg_app_token_runtime_mode_revoke
    AFTER UPDATE OF runtime_mode ON applications FOR EACH ROW
    EXECUTE FUNCTION revoke_app_tokens_on_runtime_mode_change();

CREATE TABLE application_releases (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    environment VARCHAR(32) NOT NULL CHECK (environment ~ '^[a-z0-9][a-z0-9_-]{0,31}$'),
    version VARCHAR(64) NOT NULL CHECK (version ~ '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$' AND version NOT LIKE '%..%'),
    entry_type VARCHAR(10) NOT NULL CHECK (entry_type IN ('HTML', 'ESM')),
    entry_url VARCHAR(2000) NOT NULL,
    health_url VARCHAR(2000),
    resources JSONB NOT NULL DEFAULT '[]' CHECK (jsonb_typeof(resources) = 'array'),
    validation_status VARCHAR(10) NOT NULL DEFAULT 'DRAFT' CHECK (validation_status IN ('DRAFT', 'PASSED', 'FAILED')),
    validation_result JSONB NOT NULL DEFAULT '{}',
    checked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_by UUID,
    UNIQUE (application_id, environment, version),
    UNIQUE (application_id, environment, id),
    CHECK ((validation_status = 'DRAFT' AND checked_at IS NULL) OR
           (validation_status IN ('PASSED', 'FAILED') AND checked_at IS NOT NULL))
);

CREATE TABLE application_deployments (
    application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    environment VARCHAR(32) NOT NULL,
    active_release_id UUID NOT NULL,
    previous_release_id UUID,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by UUID,
    PRIMARY KEY (application_id, environment),
    FOREIGN KEY (application_id, environment, active_release_id)
        REFERENCES application_releases(application_id, environment, id),
    FOREIGN KEY (application_id, environment, previous_release_id)
        REFERENCES application_releases(application_id, environment, id),
    CHECK (previous_release_id IS NULL OR previous_release_id <> active_release_id)
);

CREATE FUNCTION protect_application_release_definition() RETURNS trigger AS $$
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
$$ LANGUAGE plpgsql;
CREATE TRIGGER application_release_definition_immutable
    BEFORE UPDATE ON application_releases FOR EACH ROW
    EXECUTE FUNCTION protect_application_release_definition();

-- Compatibility: legacy entry_url remains until an operator registers, preflights and
-- activates a version. No migration writes to tenant, organization or role grants.
