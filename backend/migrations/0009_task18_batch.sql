-- 0009 Task 18 批量变更: 租户/组织过期时间 + 审计 user-agent + 根组织唯一约束
-- 需求来源: 用户 6 条需求中的 1/3/6

-- 1) 租户与组织增加过期时间 (NULL = 永不过期; 到期后租户下所有组织禁止登录)
ALTER TABLE tenants ADD COLUMN IF NOT EXISTS expires_at TIMESTAMPTZ NULL;
ALTER TABLE organizations ADD COLUMN IF NOT EXISTS expires_at TIMESTAMPTZ NULL;

-- 2) 审计日志记录请求 user-agent (存量行补空串兼容; 应用层负责写入)
ALTER TABLE audit_logs ADD COLUMN IF NOT EXISTS user_agent VARCHAR(500) NOT NULL DEFAULT '';

-- 3) 一个租户只能有一个根组织: 部分唯一索引 (parent_id IS NULL 的组织在租户内唯一)
--    当前存量数据: 每租户各 1 个根组织, 索引可直接创建成功
CREATE UNIQUE INDEX IF NOT EXISTS uq_org_root_per_tenant ON organizations (tenant_id) WHERE parent_id IS NULL;
