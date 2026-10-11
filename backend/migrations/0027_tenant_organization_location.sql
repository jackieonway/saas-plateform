-- Optional address and WGS-84 coordinates for tenant and organization markers.
ALTER TABLE tenants
  ADD COLUMN province VARCHAR(100) NOT NULL DEFAULT '',
  ADD COLUMN city VARCHAR(100) NOT NULL DEFAULT '',
  ADD COLUMN district VARCHAR(100) NOT NULL DEFAULT '',
  ADD COLUMN address VARCHAR(300) NOT NULL DEFAULT '',
  ADD COLUMN longitude DOUBLE PRECISION,
  ADD COLUMN latitude DOUBLE PRECISION,
  ADD CONSTRAINT ck_tenant_coordinates CHECK (
    (longitude IS NULL AND latitude IS NULL) OR
    (longitude IS NOT NULL AND latitude IS NOT NULL AND
     longitude BETWEEN -180 AND 180 AND latitude BETWEEN -90 AND 90)
  );

ALTER TABLE organizations
  ADD COLUMN province VARCHAR(100) NOT NULL DEFAULT '',
  ADD COLUMN city VARCHAR(100) NOT NULL DEFAULT '',
  ADD COLUMN district VARCHAR(100) NOT NULL DEFAULT '',
  ADD COLUMN address VARCHAR(300) NOT NULL DEFAULT '',
  ADD COLUMN longitude DOUBLE PRECISION,
  ADD COLUMN latitude DOUBLE PRECISION,
  ADD CONSTRAINT ck_organization_coordinates CHECK (
    (longitude IS NULL AND latitude IS NULL) OR
    (longitude IS NOT NULL AND latitude IS NOT NULL AND
     longitude BETWEEN -180 AND 180 AND latitude BETWEEN -90 AND 90)
  );
