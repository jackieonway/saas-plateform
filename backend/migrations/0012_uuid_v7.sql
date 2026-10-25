-- Application-owned UUIDv7 IDs. PostgreSQL 17 compatible; no SQL generator.
-- Apply to an empty/rekeyed target. Existing v4 data deliberately fails atomically.
-- Historical migrations/checksums stay unchanged. Every writer supplies its ID.
CREATE FUNCTION public.is_uuid_v7(value uuid) RETURNS boolean
LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE
SET search_path = pg_catalog
AS $$ SELECT (get_byte(uuid_send(value), 6) >> 4) = 7
    AND (get_byte(uuid_send(value), 8) & 192) = 128 $$;

ALTER TABLE public."application_deployments"
    ALTER COLUMN "active_release_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_active_release_id" CHECK (public.is_uuid_v7("active_release_id")),
    ALTER COLUMN "application_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_application_id" CHECK (public.is_uuid_v7("application_id")),
    ALTER COLUMN "previous_release_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_previous_release_id" CHECK (public.is_uuid_v7("previous_release_id")),
    ALTER COLUMN "updated_by" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_updated_by" CHECK (public.is_uuid_v7("updated_by"));

ALTER TABLE public."application_release_artifacts"
    ALTER COLUMN "application_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_application_id" CHECK (public.is_uuid_v7("application_id"));

ALTER TABLE public."application_release_operations"
    ALTER COLUMN "application_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_application_id" CHECK (public.is_uuid_v7("application_id")),
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id"));

ALTER TABLE public."application_releases"
    ALTER COLUMN "application_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_application_id" CHECK (public.is_uuid_v7("application_id")),
    ALTER COLUMN "created_by" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_created_by" CHECK (public.is_uuid_v7("created_by")),
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id"));

ALTER TABLE public."application_url_entries"
    ALTER COLUMN "application_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_application_id" CHECK (public.is_uuid_v7("application_id")),
    ALTER COLUMN "revision" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_revision" CHECK (public.is_uuid_v7("revision")),
    ALTER COLUMN "updated_by" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_updated_by" CHECK (public.is_uuid_v7("updated_by"));

ALTER TABLE public."applications"
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id"));

ALTER TABLE public."audit_logs"
    ALTER COLUMN "application_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_application_id" CHECK (public.is_uuid_v7("application_id")),
    ALTER COLUMN "organization_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_organization_id" CHECK (public.is_uuid_v7("organization_id")),
    ALTER COLUMN "request_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_request_id" CHECK (public.is_uuid_v7("request_id")),
    ALTER COLUMN "tenant_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_tenant_id" CHECK (public.is_uuid_v7("tenant_id")),
    ALTER COLUMN "user_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_user_id" CHECK (public.is_uuid_v7("user_id"));

ALTER TABLE public."departments"
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id")),
    ALTER COLUMN "organization_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_organization_id" CHECK (public.is_uuid_v7("organization_id")),
    ALTER COLUMN "parent_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_parent_id" CHECK (public.is_uuid_v7("parent_id"));

ALTER TABLE public."managed_deployment_bindings"
    ALTER COLUMN "artifact_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_artifact_id" CHECK (public.is_uuid_v7("artifact_id")),
    ALTER COLUMN "release_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_release_id" CHECK (public.is_uuid_v7("release_id"));

ALTER TABLE public."managed_deployment_jobs"
    ALTER COLUMN "artifact_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_artifact_id" CHECK (public.is_uuid_v7("artifact_id")),
    ALTER COLUMN "created_by" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_created_by" CHECK (public.is_uuid_v7("created_by")),
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id")),
    ALTER COLUMN "nonce" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_nonce" CHECK (public.is_uuid_v7("nonce")),
    ALTER COLUMN "release_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_release_id" CHECK (public.is_uuid_v7("release_id")),
    ALTER COLUMN "request_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_request_id" CHECK (public.is_uuid_v7("request_id"));

ALTER TABLE public."managed_image_artifacts"
    ALTER COLUMN "application_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_application_id" CHECK (public.is_uuid_v7("application_id")),
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id"));

ALTER TABLE public."membership_data_grants"
    ALTER COLUMN "application_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_application_id" CHECK (public.is_uuid_v7("application_id")),
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id")),
    ALTER COLUMN "membership_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_membership_id" CHECK (public.is_uuid_v7("membership_id"));

ALTER TABLE public."memberships"
    ALTER COLUMN "department_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_department_id" CHECK (public.is_uuid_v7("department_id")),
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id")),
    ALTER COLUMN "organization_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_organization_id" CHECK (public.is_uuid_v7("organization_id")),
    ALTER COLUMN "user_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_user_id" CHECK (public.is_uuid_v7("user_id"));

ALTER TABLE public."menus"
    ALTER COLUMN "application_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_application_id" CHECK (public.is_uuid_v7("application_id")),
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id")),
    ALTER COLUMN "parent_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_parent_id" CHECK (public.is_uuid_v7("parent_id"));

ALTER TABLE public."notification_prefs"
    ALTER COLUMN "user_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_user_id" CHECK (public.is_uuid_v7("user_id"));

ALTER TABLE public."notification_publications"
    ALTER COLUMN "application_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_application_id" CHECK (public.is_uuid_v7("application_id")),
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id")),
    ALTER COLUMN "publisher_organization_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_publisher_organization_id" CHECK (public.is_uuid_v7("publisher_organization_id")),
    ALTER COLUMN "publisher_user_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_publisher_user_id" CHECK (public.is_uuid_v7("publisher_user_id")),
    ALTER COLUMN "tenant_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_tenant_id" CHECK (public.is_uuid_v7("tenant_id"));

ALTER TABLE public."notifications"
    ALTER COLUMN "application_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_application_id" CHECK (public.is_uuid_v7("application_id")),
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id")),
    ALTER COLUMN "organization_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_organization_id" CHECK (public.is_uuid_v7("organization_id")),
    ALTER COLUMN "publication_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_publication_id" CHECK (public.is_uuid_v7("publication_id")),
    ALTER COLUMN "tenant_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_tenant_id" CHECK (public.is_uuid_v7("tenant_id")),
    ALTER COLUMN "user_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_user_id" CHECK (public.is_uuid_v7("user_id"));

ALTER TABLE public."oauth_authorization_codes"
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id")),
    ALTER COLUMN "organization_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_organization_id" CHECK (public.is_uuid_v7("organization_id")),
    ALTER COLUMN "tenant_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_tenant_id" CHECK (public.is_uuid_v7("tenant_id")),
    ALTER COLUMN "user_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_user_id" CHECK (public.is_uuid_v7("user_id"));

ALTER TABLE public."oauth_authorization_requests"
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id"));

ALTER TABLE public."oauth_clients"
    ALTER COLUMN "application_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_application_id" CHECK (public.is_uuid_v7("application_id")),
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id"));

ALTER TABLE public."oauth_device_codes"
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id")),
    ALTER COLUMN "organization_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_organization_id" CHECK (public.is_uuid_v7("organization_id")),
    ALTER COLUMN "tenant_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_tenant_id" CHECK (public.is_uuid_v7("tenant_id")),
    ALTER COLUMN "user_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_user_id" CHECK (public.is_uuid_v7("user_id"));

ALTER TABLE public."oauth_tokens"
    ALTER COLUMN "application_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_application_id" CHECK (public.is_uuid_v7("application_id")),
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id")),
    ALTER COLUMN "jti" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_jti" CHECK (public.is_uuid_v7("jti")),
    ALTER COLUMN "organization_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_organization_id" CHECK (public.is_uuid_v7("organization_id")),
    ALTER COLUMN "platform_session_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_platform_session_id" CHECK (public.is_uuid_v7("platform_session_id")),
    ALTER COLUMN "tenant_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_tenant_id" CHECK (public.is_uuid_v7("tenant_id")),
    ALTER COLUMN "user_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_user_id" CHECK (public.is_uuid_v7("user_id"));

ALTER TABLE public."organization_applications"
    ALTER COLUMN "application_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_application_id" CHECK (public.is_uuid_v7("application_id")),
    ALTER COLUMN "organization_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_organization_id" CHECK (public.is_uuid_v7("organization_id"));

ALTER TABLE public."organization_menus"
    ALTER COLUMN "menu_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_menu_id" CHECK (public.is_uuid_v7("menu_id")),
    ALTER COLUMN "organization_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_organization_id" CHECK (public.is_uuid_v7("organization_id"));

ALTER TABLE public."organizations"
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id")),
    ALTER COLUMN "parent_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_parent_id" CHECK (public.is_uuid_v7("parent_id")),
    ALTER COLUMN "tenant_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_tenant_id" CHECK (public.is_uuid_v7("tenant_id"));

ALTER TABLE public."platform_session_refresh_tokens"
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id")),
    ALTER COLUMN "session_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_session_id" CHECK (public.is_uuid_v7("session_id"));

ALTER TABLE public."platform_sessions"
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id")),
    ALTER COLUMN "organization_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_organization_id" CHECK (public.is_uuid_v7("organization_id")),
    ALTER COLUMN "tenant_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_tenant_id" CHECK (public.is_uuid_v7("tenant_id")),
    ALTER COLUMN "user_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_user_id" CHECK (public.is_uuid_v7("user_id"));

ALTER TABLE public."role_menus"
    ALTER COLUMN "menu_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_menu_id" CHECK (public.is_uuid_v7("menu_id")),
    ALTER COLUMN "role_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_role_id" CHECK (public.is_uuid_v7("role_id"));

ALTER TABLE public."roles"
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id")),
    ALTER COLUMN "organization_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_organization_id" CHECK (public.is_uuid_v7("organization_id")),
    ALTER COLUMN "tenant_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_tenant_id" CHECK (public.is_uuid_v7("tenant_id"));

ALTER TABLE public."tenant_applications"
    ALTER COLUMN "application_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_application_id" CHECK (public.is_uuid_v7("application_id")),
    ALTER COLUMN "tenant_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_tenant_id" CHECK (public.is_uuid_v7("tenant_id"));

ALTER TABLE public."tenant_menus"
    ALTER COLUMN "menu_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_menu_id" CHECK (public.is_uuid_v7("menu_id")),
    ALTER COLUMN "tenant_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_tenant_id" CHECK (public.is_uuid_v7("tenant_id"));

ALTER TABLE public."tenants"
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id"));

ALTER TABLE public."user_roles"
    ALTER COLUMN "organization_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_organization_id" CHECK (public.is_uuid_v7("organization_id")),
    ALTER COLUMN "role_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_role_id" CHECK (public.is_uuid_v7("role_id")),
    ALTER COLUMN "user_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_user_id" CHECK (public.is_uuid_v7("user_id"));

ALTER TABLE public."users"
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id"));

ALTER TABLE public."zip_distribution_jobs"
    ALTER COLUMN "application_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_application_id" CHECK (public.is_uuid_v7("application_id")),
    ALTER COLUMN "created_by" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_created_by" CHECK (public.is_uuid_v7("created_by")),
    ALTER COLUMN "id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_id" CHECK (public.is_uuid_v7("id")),
    ALTER COLUMN "nonce" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_nonce" CHECK (public.is_uuid_v7("nonce")),
    ALTER COLUMN "release_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_release_id" CHECK (public.is_uuid_v7("release_id")),
    ALTER COLUMN "request_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_request_id" CHECK (public.is_uuid_v7("request_id")),
    ALTER COLUMN "verification_nonce" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_verification_nonce" CHECK (public.is_uuid_v7("verification_nonce"));

ALTER TABLE public."zip_distribution_nodes"
    ALTER COLUMN "job_id" DROP DEFAULT,
    ADD CONSTRAINT "uuid_v7_job_id" CHECK (public.is_uuid_v7("job_id"));
