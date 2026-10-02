-- Preserve unowned historical rows as LEGACY; new deliveries always carry explicit context.
CREATE TABLE notification_publications (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    publisher_organization_id UUID NOT NULL,
    application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    publisher_user_id UUID NOT NULL,
    publisher_username VARCHAR(50) NOT NULL,
    application_name VARCHAR(200) NOT NULL,
    idempotency_key VARCHAR(128) NOT NULL,
    request_hash CHAR(64) NOT NULL,
    request_body JSONB NOT NULL CHECK (jsonb_typeof(request_body) = 'object'),
    recipient_count INTEGER NOT NULL CHECK (recipient_count >= 0),
    delivery_count INTEGER NOT NULL CHECK (delivery_count >= recipient_count),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, publisher_organization_id, application_id, publisher_user_id, idempotency_key),
    UNIQUE (id, tenant_id, application_id),
    FOREIGN KEY (publisher_organization_id, tenant_id)
        REFERENCES organizations(id, tenant_id) ON DELETE CASCADE
);

CREATE FUNCTION reject_notification_publication_update() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW IS DISTINCT FROM OLD THEN
        RAISE EXCEPTION 'notification publications are immutable' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END
$$;
CREATE TRIGGER trg_notification_publication_immutable
    BEFORE UPDATE ON notification_publications
    FOR EACH ROW EXECUTE FUNCTION reject_notification_publication_update();

ALTER TABLE notifications
    ADD COLUMN source_kind VARCHAR(12) NOT NULL DEFAULT 'LEGACY',
    ADD COLUMN tenant_id UUID REFERENCES tenants(id) ON DELETE CASCADE,
    ADD COLUMN organization_id UUID,
    ADD COLUMN application_id UUID REFERENCES applications(id) ON DELETE CASCADE,
    ADD COLUMN publication_id UUID,
    ADD CONSTRAINT ck_notification_context CHECK (
        (source_kind = 'LEGACY' AND tenant_id IS NULL AND organization_id IS NULL
         AND application_id IS NULL AND publication_id IS NULL)
        OR (source_kind = 'APP' AND tenant_id IS NOT NULL AND organization_id IS NOT NULL
         AND application_id IS NOT NULL AND publication_id IS NOT NULL)
        OR (source_kind = 'SYSTEM' AND tenant_id IS NOT NULL AND organization_id IS NOT NULL
         AND application_id IS NULL AND publication_id IS NULL)
    ),
    ADD CONSTRAINT fk_notification_organization_tenant
        FOREIGN KEY (organization_id, tenant_id)
        REFERENCES organizations(id, tenant_id) ON DELETE CASCADE,
    ADD CONSTRAINT fk_notification_publication_context
        FOREIGN KEY (publication_id, tenant_id, application_id)
        REFERENCES notification_publications(id, tenant_id, application_id) ON DELETE CASCADE,
    ADD CONSTRAINT uq_notification_publication_delivery
        UNIQUE (publication_id, organization_id, user_id);

CREATE INDEX idx_notifications_context_inbox
    ON notifications(user_id, tenant_id, organization_id, is_read, created_at DESC, id)
    WHERE source_kind IN ('APP', 'SYSTEM');
