-- Explicit platform-membership-rollout operation; never a SQLx migration.
-- The caller holds a transaction advisory lock and memberships ACCESS EXCLUSIVE,
-- validates the exact binary key, dependent FKs, triple key and immutable trigger,
-- and verifies the inventory/gate artifacts before executing this statement.
-- RESTRICT is intentional: foreign-key dependencies must be retired explicitly.
ALTER TABLE public.memberships
    DROP CONSTRAINT memberships_user_id_organization_id_key RESTRICT;
