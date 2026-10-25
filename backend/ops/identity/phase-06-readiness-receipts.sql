-- Owned by the explicit rollout operation, outside SQLx migration history.
-- No runtime route writes this relation. Operators must keep its owner separate
-- from the service role and grant the service SELECT when using split roles.
CREATE TABLE public.identity_membership_readiness_receipts (
    id uuid PRIMARY KEY CONSTRAINT uuid_v7_id CHECK (public.is_uuid_v7(id)),
    environment text NOT NULL CHECK (environment IN ('development', 'production')),
    scope text NOT NULL CHECK (scope IN ('TASK_OWNED', 'PRODUCTION')),
    database_name text NOT NULL,
    database_oid bigint NOT NULL,
    cluster_system_identifier text NOT NULL,
    platform_binary_sha256 text NOT NULL CHECK (platform_binary_sha256 ~ '^[0-9a-f]{64}$'),
    manifest_sha256 text NOT NULL CHECK (manifest_sha256 ~ '^[0-9a-f]{64}$'),
    migration_sha256 text NOT NULL CHECK (migration_sha256 ~ '^[0-9a-f]{64}$'),
    schema_sha256 text NOT NULL CHECK (schema_sha256 ~ '^[0-9a-f]{64}$'),
    applied_at timestamptz NOT NULL DEFAULT now(),
    inventory jsonb NOT NULL,
    gates jsonb NOT NULL,
    CHECK ((environment = 'production') = (scope = 'PRODUCTION'))
);
REVOKE ALL ON TABLE public.identity_membership_readiness_receipts FROM PUBLIC;
COMMENT ON TABLE public.identity_membership_readiness_receipts IS
    'Phase 6 explicit rollout receipts; owner=platform-membership-rollout operator; not an SQLx migration or automatic production approval.';
