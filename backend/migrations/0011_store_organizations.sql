-- Store identity belongs to the platform organization directory. Business
-- services keep only their own extension data and never join this database.
ALTER TABLE organizations DROP CONSTRAINT ck_organization_type;
ALTER TABLE organizations ADD CONSTRAINT ck_organization_type CHECK (
  organization_type IN ('COMPANY','SUBSIDIARY','BRANCH','STORE','PUBLIC_INSTITUTION','GOVERNMENT','OTHER')
);
