ALTER TABLE region_versions ADD COLUMN deleted_at TIMESTAMPTZ;
COMMENT ON COLUMN region_versions.deleted_at IS '版本删除时间；保留墓碑防止内置版本重启后重新导入';
