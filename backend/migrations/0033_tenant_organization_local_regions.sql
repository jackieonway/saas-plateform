-- Complete the tenant and organization location path with township and village.
-- Empty defaults preserve existing rows and clients that only provide province/city/district.
ALTER TABLE tenants
  ADD COLUMN township VARCHAR(128) NOT NULL DEFAULT '',
  ADD COLUMN village VARCHAR(128) NOT NULL DEFAULT '';

ALTER TABLE organizations
  ADD COLUMN township VARCHAR(128) NOT NULL DEFAULT '',
  ADD COLUMN village VARCHAR(128) NOT NULL DEFAULT '';

COMMENT ON COLUMN tenants.township IS '乡镇、街道地区名称';
COMMENT ON COLUMN tenants.village IS '村、社区地区名称';
COMMENT ON COLUMN organizations.township IS '乡镇、街道地区名称';
COMMENT ON COLUMN organizations.village IS '村、社区地区名称';
