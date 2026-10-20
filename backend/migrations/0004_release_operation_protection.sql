-- Expand-only. Pending records deliberately have no expiry: a crash or unknown
-- outcome must retain protection until explicit operational reconciliation.
CREATE TABLE application_release_operations (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    application_id uuid NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    kind text NOT NULL CHECK (kind IN ('REGISTER','UPLOAD','PREFLIGHT','ACTIVATE','ROLLBACK')),
    started_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX application_release_operations_pending
    ON application_release_operations(application_id);
