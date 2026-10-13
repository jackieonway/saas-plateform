-- Existing records use the company classification; new root organizations derive
-- their classification from the tenant when created by the API.
ALTER TABLE tenants
    ADD COLUMN tenant_type VARCHAR(30) NOT NULL DEFAULT 'COMPANY',
    ADD CONSTRAINT ck_tenant_type CHECK (
        tenant_type IN ('PERSONAL', 'COMPANY', 'PUBLIC_INSTITUTION', 'GOVERNMENT', 'OTHER')
    );

ALTER TABLE organizations
    ADD COLUMN organization_type VARCHAR(30) NOT NULL DEFAULT 'COMPANY',
    ADD CONSTRAINT ck_organization_type CHECK (
        organization_type IN ('COMPANY', 'SUBSIDIARY', 'BRANCH', 'PUBLIC_INSTITUTION', 'GOVERNMENT', 'OTHER')
    );
