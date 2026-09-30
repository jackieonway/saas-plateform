-- 0007 平台租户体系
-- 需求: 全库有且仅有一个「平台租户」(is_platform = true), 平台租户可管理所有租户,
-- 其余租户只能操作自己租户内的数据。

-- 1) 租户表增加平台标记 (默认 false, 新建租户一律非平台)
ALTER TABLE tenants ADD COLUMN is_platform BOOLEAN NOT NULL DEFAULT false;

-- 2) 部分唯一索引: 全库至多一个平台租户 (并发兜底)
CREATE UNIQUE INDEX uniq_tenants_platform ON tenants (is_platform) WHERE is_platform;

-- 3) 存量数据标记: 种子租户 dante 为平台租户;
--    兜底: 若该库无种子租户且尚无平台租户, 则最早创建的租户充当平台租户。
UPDATE tenants SET is_platform = true WHERE code = 'dante';
UPDATE tenants SET is_platform = true
WHERE NOT EXISTS (SELECT 1 FROM tenants WHERE is_platform)
  AND id = (SELECT id FROM tenants ORDER BY created_at, id LIMIT 1);
