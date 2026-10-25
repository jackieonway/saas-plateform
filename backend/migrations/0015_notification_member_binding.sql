-- Existing deliveries/publications remain archived with NULL member proofs.
-- Never infer their original identity from today's memberships or versions.
ALTER TABLE public.notifications
    ADD COLUMN recipient_membership_id uuid CHECK (public.is_uuid_v7(recipient_membership_id)),
    ADD COLUMN recipient_member_type text,
    ADD COLUMN recipient_identity_version bigint,
    ADD CONSTRAINT notifications_recipient_proof_check CHECK (
        (recipient_membership_id IS NULL AND recipient_member_type IS NULL
         AND recipient_identity_version IS NULL)
        OR (recipient_membership_id IS NOT NULL AND recipient_member_type IS NOT NULL
            AND recipient_identity_version IS NOT NULL
            AND recipient_member_type IN ('EMPLOYEE','CUSTOMER')
            AND recipient_identity_version >= 1));

-- A membership may be removed and later recreated with another ID. Keep the
-- immutable original soft reference, without a membership FK or cascade.
CREATE FUNCTION public.protect_notification_recipient() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
BEGIN
    IF NEW.id IS DISTINCT FROM OLD.id OR NEW.user_id IS DISTINCT FROM OLD.user_id
       OR NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
       OR NEW.organization_id IS DISTINCT FROM OLD.organization_id
       OR NEW.application_id IS DISTINCT FROM OLD.application_id
       OR NEW.publication_id IS DISTINCT FROM OLD.publication_id
       OR NEW.source_kind IS DISTINCT FROM OLD.source_kind
       OR NEW.recipient_membership_id IS DISTINCT FROM OLD.recipient_membership_id
       OR NEW.recipient_member_type IS DISTINCT FROM OLD.recipient_member_type
       OR NEW.recipient_identity_version IS DISTINCT FROM OLD.recipient_identity_version THEN
        RAISE EXCEPTION 'notification ownership and recipient proof are immutable'
            USING ERRCODE='23514', CONSTRAINT='notifications_recipient_immutable';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER trg_notification_recipient_immutable
BEFORE UPDATE ON public.notifications FOR EACH ROW
EXECUTE FUNCTION public.protect_notification_recipient();

ALTER TABLE public.notifications
    DROP CONSTRAINT uq_notification_publication_delivery,
    ADD CONSTRAINT uq_notification_publication_delivery
        UNIQUE(publication_id,recipient_membership_id,recipient_identity_version);
CREATE INDEX idx_notifications_member_inbox ON public.notifications
    (recipient_membership_id,recipient_identity_version,is_read,created_at DESC,id)
    WHERE recipient_membership_id IS NOT NULL;

ALTER TABLE public.notification_publications
    ADD COLUMN publisher_membership_id uuid CHECK (public.is_uuid_v7(publisher_membership_id)),
    ADD COLUMN publisher_member_type text,
    ADD COLUMN publisher_identity_version bigint,
    ADD CONSTRAINT notification_publications_publisher_proof_check CHECK (
        (publisher_membership_id IS NULL AND publisher_member_type IS NULL
         AND publisher_identity_version IS NULL)
        OR (publisher_membership_id IS NOT NULL AND publisher_member_type IS NOT NULL
            AND publisher_identity_version IS NOT NULL
            AND publisher_member_type IN ('EMPLOYEE','CUSTOMER')
            AND publisher_identity_version >= 1)),
    DROP CONSTRAINT notification_publications_tenant_id_publisher_organization__key,
    ADD CONSTRAINT uq_notification_publication_member_request
        UNIQUE(tenant_id,publisher_organization_id,application_id,publisher_user_id,
               publisher_membership_id,publisher_identity_version,idempotency_key);
-- The existing publication BEFORE UPDATE trigger protects all columns,
-- including the new proof and NULL legacy proofs, against later rebinding.

-- Preferences are per immutable member generation. Retain notification_prefs
-- as legacy data; no user-level preferences are copied into a new identity.
CREATE TABLE public.notification_member_prefs (
    membership_id uuid NOT NULL CHECK (public.is_uuid_v7(membership_id)),
    identity_version bigint NOT NULL CHECK (identity_version >= 1),
    dnd_enabled boolean NOT NULL DEFAULT false,
    dnd_start varchar(5) NOT NULL DEFAULT '22:00',
    dnd_end varchar(5) NOT NULL DEFAULT '07:00',
    muted_types text NOT NULL DEFAULT '',
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY(membership_id,identity_version)
);
CREATE FUNCTION public.protect_notification_member_pref_identity() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
BEGIN
    IF NEW.membership_id IS DISTINCT FROM OLD.membership_id
       OR NEW.identity_version IS DISTINCT FROM OLD.identity_version THEN
        RAISE EXCEPTION 'notification preference identity is immutable'
            USING ERRCODE='23514', CONSTRAINT='notification_member_prefs_identity_immutable';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER trg_notification_member_pref_identity
BEFORE UPDATE ON public.notification_member_prefs FOR EACH ROW
EXECUTE FUNCTION public.protect_notification_member_pref_identity();
