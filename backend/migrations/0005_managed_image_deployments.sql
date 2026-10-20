-- Optional managed static-image deployment. Existing URL registration stays unchanged.
CREATE TABLE managed_image_artifacts (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    application_id uuid NOT NULL REFERENCES applications(id),
    version text NOT NULL,
    target_id text NOT NULL,
    template_id text NOT NULL,
    image_ref text NOT NULL,
    policy_revision text NOT NULL,
    public_url text NOT NULL,
    state text NOT NULL DEFAULT 'PENDING' CHECK(state IN ('PENDING','READY','CLEANING','REMOVED')),
    image_id text,
    route_sha256 text,
    container_name text,
    ever_ready boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE(application_id,version,target_id),
    UNIQUE(public_url)
);
CREATE TABLE managed_deployment_bindings (
    release_id uuid PRIMARY KEY REFERENCES application_releases(id),
    artifact_id uuid NOT NULL REFERENCES managed_image_artifacts(id)
);
CREATE TABLE managed_deployment_jobs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    artifact_id uuid NOT NULL REFERENCES managed_image_artifacts(id),
    release_id uuid NOT NULL REFERENCES application_releases(id),
    request_id uuid NOT NULL,
    payload_sha256 text NOT NULL,
    operation text NOT NULL CHECK(operation IN ('DEPLOY','CLEANUP')),
    state text NOT NULL DEFAULT 'QUEUED' CHECK(state IN ('QUEUED','RUNNING','READY','FAILED','CLEANED')),
    attempt integer NOT NULL DEFAULT 0 CHECK(attempt >= 0),
    nonce uuid,
    lease_until timestamptz,
    receipt jsonb,
    error_code text,
    created_by uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE(request_id)
);
CREATE UNIQUE INDEX managed_deployment_one_worker ON managed_deployment_jobs(artifact_id)
    WHERE state IN ('QUEUED','RUNNING');
CREATE INDEX managed_deployment_queue ON managed_deployment_jobs(state,created_at);
CREATE INDEX managed_deployment_releases ON managed_deployment_jobs(release_id);
