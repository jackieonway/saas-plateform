-- Existing rows keep NULL correlation; historical details are never guessed.
ALTER TABLE audit_logs ADD COLUMN request_id UUID;
ALTER TABLE audit_logs ADD COLUMN application_id UUID;
-- Intentionally no FK: deleting an application must not remove its audit identity.
CREATE INDEX idx_audit_request_id ON audit_logs(request_id) WHERE request_id IS NOT NULL;
CREATE INDEX idx_audit_application_time ON audit_logs(application_id, created_at DESC)
    WHERE application_id IS NOT NULL;
