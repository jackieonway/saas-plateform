-- Optional no-shared-storage delivery. Existing ZIP identities remain immutable.
CREATE TABLE zip_distribution_jobs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    application_id uuid NOT NULL REFERENCES applications(id),
    release_id uuid NOT NULL REFERENCES application_releases(id),
    group_id text NOT NULL,
    public_root text NOT NULL,
    artifact_digest text NOT NULL CHECK(artifact_digest ~ '^[0-9a-f]{64}$'),
    manifest jsonb NOT NULL,
    policy_revision text NOT NULL,
    policy jsonb NOT NULL,
    request_id uuid NOT NULL UNIQUE,
    payload_sha256 text NOT NULL,
    state text NOT NULL DEFAULT 'QUEUED' CHECK(state IN ('QUEUED','RUNNING','READY','FAILED')),
    attempt integer NOT NULL DEFAULT 0 CHECK(attempt>=0),
    nonce uuid,
    lease_until timestamptz,
    verification_nonce uuid,
    verified_at timestamptz,
    ever_ready boolean NOT NULL DEFAULT false,
    error_code text,
    created_by uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE(release_id)
);
CREATE TABLE zip_distribution_nodes (
    job_id uuid NOT NULL REFERENCES zip_distribution_jobs(id),
    node_id text NOT NULL,
    state text NOT NULL DEFAULT 'PENDING' CHECK(state IN ('PENDING','READY','FAILED')),
    attempt integer NOT NULL DEFAULT 0,
    receipt jsonb,
    error_code text,
    verified_at timestamptz,
    PRIMARY KEY(job_id,node_id)
);
CREATE INDEX zip_distribution_queue ON zip_distribution_jobs(state,created_at);
CREATE INDEX zip_distribution_application ON zip_distribution_jobs(application_id,created_at);
CREATE INDEX zip_distribution_public_root ON zip_distribution_jobs(public_root);
