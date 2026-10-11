-- Upgrade databases initialized with the first region import draft.
DELETE FROM regions a USING regions b
WHERE a.version=b.version AND a.code=b.code AND a.source_id>b.source_id;
CREATE UNIQUE INDEX IF NOT EXISTS idx_regions_version_code_unique ON regions(version,code);
