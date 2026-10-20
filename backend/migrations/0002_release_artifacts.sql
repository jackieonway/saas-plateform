-- ZIP/tar.gz artifacts are shared by every environment of an application version.
-- Existing URL registrations remain unbound until a verified upload adopts them.
CREATE TABLE public.application_release_artifacts (
    application_id uuid NOT NULL REFERENCES public.applications(id) ON DELETE CASCADE,
    version character varying(64) NOT NULL,
    artifact_digest text NOT NULL CHECK (artifact_digest ~ '^[0-9a-f]{64}$'),
    manifest jsonb NOT NULL CHECK (jsonb_typeof(manifest) = 'object'),
    created_at timestamp with time zone NOT NULL DEFAULT now(),
    PRIMARY KEY (application_id, version),
    UNIQUE (application_id, version, artifact_digest),
    CHECK (version ~ '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$' AND version NOT LIKE '%..%')
);

ALTER TABLE public.application_releases ADD COLUMN artifact_digest text;
ALTER TABLE public.application_releases ADD CONSTRAINT application_release_artifact_reference
    FOREIGN KEY (application_id, version, artifact_digest)
    REFERENCES public.application_release_artifacts(application_id, version, artifact_digest);

CREATE FUNCTION public.protect_application_release_artifact()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW IS DISTINCT FROM OLD THEN
        RAISE EXCEPTION 'application release artifact is immutable' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER application_release_artifact_immutable BEFORE UPDATE
    ON public.application_release_artifacts FOR EACH ROW
    EXECUTE FUNCTION public.protect_application_release_artifact();

CREATE OR REPLACE FUNCTION public.protect_application_release_definition()
RETURNS trigger LANGUAGE plpgsql AS $$
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
