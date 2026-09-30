-- 0010 对外 API: OAuth 客户端关联应用
-- 需求: 第三方服务 (OAuth 客户端, 如 client_credentials 模式的 api-service) 获取 access token 后,
--       可拉取其「关联应用」的菜单树与角色授权数据 (GET /api/external/menus, GET /api/external/roles)。
-- 幂等设计: ADD COLUMN IF NOT EXISTS, 语句可重复执行不报错 (与 0006/0008/0009 同风格)。

-- 1) 客户端增加应用关联 (可空: 未关联应用的客户端调用对外 API 返回 404 提示配置;
--    应用被删除时置 NULL, 不连带删除客户端)
ALTER TABLE oauth_clients ADD COLUMN IF NOT EXISTS application_id UUID REFERENCES applications(id) ON DELETE SET NULL;

-- 2) 关联查询索引
CREATE INDEX IF NOT EXISTS idx_oauth_clients_application ON oauth_clients(application_id);

-- 3) 演示关联: API 服务客户端 (client_credentials) → athena 数据分析应用
UPDATE oauth_clients SET application_id = (SELECT id FROM applications WHERE code = 'athena')
WHERE client_id = 'api-service';
