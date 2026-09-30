-- 0011 微前端 (仿乾坤) 后端支撑
-- 需求: 主应用以 HTML 入口加载任意技术栈子应用 (Vue 3 / jQuery 遗留系统等)。
--       applications 增加 entry_type ('ESM' | 'HTML') 与 entry_url (HTML 通道 = 子应用 HTML 入口,
--       如 /sub/vue/; ESM 通道 = 模块 URL)。menus.link_target 新增语义枚举 'MICRO'
--       (微前端容器路由, menu.path 填 /micro/{appCode}, link_url 留空) — 列宽 VARCHAR(10) 足够, 无需改列。
-- 幂等设计: ADD COLUMN IF NOT EXISTS + ON CONFLICT DO NOTHING / WHERE NOT EXISTS (与 0006/0008/0010 同风格)。

-- 1) 应用入口类型: 'ESM' = 原生 ESM 模块通道, 'HTML' = 微前端 HTML 入口通道
ALTER TABLE applications ADD COLUMN IF NOT EXISTS entry_type VARCHAR(10) NOT NULL DEFAULT 'ESM';

-- 2) 应用入口地址: HTML 通道 = 子应用 HTML 入口(如 /sub/vue/); ESM 通道 = 模块 URL; 空串 = 未配置
ALTER TABLE applications ADD COLUMN IF NOT EXISTS entry_url VARCHAR(500) NOT NULL DEFAULT '';

-- 3) 种子应用两行 (按 code 幂等; sort_order 取现有应用最大值 + 1 / 再 + 1)
INSERT INTO applications (code, name, icon, base_path, entry_type, entry_url, description, sort_order)
SELECT 'vue-workshop', 'Vue 工作台', 'Sparkles', '/micro', 'HTML', '/sub/vue/',
       '微前端演示: Vue 3 子应用(HTML 入口+沙箱)',
       (SELECT COALESCE(MAX(sort_order), 0) + 1 FROM applications)
WHERE NOT EXISTS (SELECT 1 FROM applications WHERE code = 'vue-workshop');

INSERT INTO applications (code, name, icon, base_path, entry_type, entry_url, description, sort_order)
SELECT 'legacy-jquery', '遗留工单(jQuery)', 'Wrench', '/micro', 'HTML', '/sub/jquery/',
       '微前端演示: jQuery 遗留系统子应用',
       (SELECT COALESCE(MAX(sort_order), 0) + 1 FROM applications)
WHERE NOT EXISTS (SELECT 1 FROM applications WHERE code = 'legacy-jquery');

-- 4) 种子菜单 (按 application_id + title 幂等; link_target='MICRO', link_url 留空, path 填 /micro/{appCode})
INSERT INTO menus (application_id, parent_id, type, title, path, icon, component, permission, sort_order, visible, link_url, link_target)
SELECT a.id, NULL, 'MENU', '总览', '/micro/vue-workshop', '', '', '', 1, true, '', 'MICRO'
FROM applications a
WHERE a.code = 'vue-workshop'
  AND NOT EXISTS (SELECT 1 FROM menus m WHERE m.application_id = a.id AND m.title = '总览');

INSERT INTO menus (application_id, parent_id, type, title, path, icon, component, permission, sort_order, visible, link_url, link_target)
SELECT a.id, NULL, 'MENU', '权限与沙箱', '/micro/vue-workshop/security', '', '', '', 2, true, '', 'MICRO'
FROM applications a
WHERE a.code = 'vue-workshop'
  AND NOT EXISTS (SELECT 1 FROM menus m WHERE m.application_id = a.id AND m.title = '权限与沙箱');

INSERT INTO menus (application_id, parent_id, type, title, path, icon, component, permission, sort_order, visible, link_url, link_target)
SELECT a.id, NULL, 'MENU', '工单列表', '/micro/legacy-jquery', '', '', '', 1, true, '', 'MICRO'
FROM applications a
WHERE a.code = 'legacy-jquery'
  AND NOT EXISTS (SELECT 1 FROM menus m WHERE m.application_id = a.id AND m.title = '工单列表');

-- 5) 角色与授权链绑定 (照抄 0002 种子里 athena 应用的绑定手法):
--    超管角色 role_menus + 租户授权 tenant_applications/tenant_menus + 组织授权 organization_applications/organization_menus
--    (dante 平台租户 / 总公司 / super_admin 内置角色 → 登录 admin 后侧栏可见这两个应用与菜单)

INSERT INTO role_menus (role_id, menu_id)
SELECT r.id, m.id
FROM roles r
JOIN organizations o ON o.id = r.organization_id
JOIN tenants t ON t.id = o.tenant_id
JOIN menus m ON m.application_id IN (SELECT id FROM applications WHERE code IN ('vue-workshop', 'legacy-jquery'))
WHERE t.code = 'dante' AND o.name = '总公司' AND r.code = 'super_admin'
ON CONFLICT DO NOTHING;

INSERT INTO tenant_applications (tenant_id, application_id)
SELECT t.id, a.id
FROM tenants t
JOIN applications a ON a.code IN ('vue-workshop', 'legacy-jquery')
WHERE t.code = 'dante'
ON CONFLICT DO NOTHING;

INSERT INTO tenant_menus (tenant_id, menu_id)
SELECT t.id, m.id
FROM tenants t
JOIN menus m ON m.application_id IN (SELECT id FROM applications WHERE code IN ('vue-workshop', 'legacy-jquery'))
WHERE t.code = 'dante'
ON CONFLICT DO NOTHING;

INSERT INTO organization_applications (organization_id, application_id)
SELECT o.id, a.id
FROM organizations o
JOIN applications a ON a.code IN ('vue-workshop', 'legacy-jquery')
WHERE o.tenant_id = (SELECT id FROM tenants WHERE code = 'dante') AND o.name = '总公司'
ON CONFLICT DO NOTHING;

INSERT INTO organization_menus (organization_id, menu_id)
SELECT o.id, m.id
FROM organizations o
JOIN menus m ON m.application_id IN (SELECT id FROM applications WHERE code IN ('vue-workshop', 'legacy-jquery'))
WHERE o.tenant_id = (SELECT id FROM tenants WHERE code = 'dante') AND o.name = '总公司'
ON CONFLICT DO NOTHING;

-- 6) OAuth 客户端种子两行 (按 client_id 幂等, 参照 0010 的应用关联写法):
--    微前端外壳客户端 — 走会话签发 (POST /api/oauth/session-token), 不走客户端认证, secret 仅填合法默认哈希。
--    grant_types 含 client_credentials (参考现有 api-service 行), 不含 refresh_token (会话签发只出一条 access token)。
INSERT INTO oauth_clients (client_id, client_secret_hash, client_name, description, client_type,
                           redirect_uris, grant_types, scopes, access_token_validity, refresh_token_validity, status, application_id)
SELECT 'micro-vue-shell', encode(sha256('micro-vue-shell-secret'::bytea), 'hex'), '微前端外壳·Vue',
       '微前端 HTML 入口子应用会话令牌签发客户端 (vue-workshop)', 'CONFIDENTIAL', '',
       'authorization_code,client_credentials', 'profile', 7200, 604800, 'ENABLED', a.id
FROM applications a
WHERE a.code = 'vue-workshop'
ON CONFLICT (client_id) DO NOTHING;

INSERT INTO oauth_clients (client_id, client_secret_hash, client_name, description, client_type,
                           redirect_uris, grant_types, scopes, access_token_validity, refresh_token_validity, status, application_id)
SELECT 'micro-jquery-shell', encode(sha256('micro-jquery-shell-secret'::bytea), 'hex'), '微前端外壳·jQuery',
       '微前端 HTML 入口子应用会话令牌签发客户端 (legacy-jquery)', 'CONFIDENTIAL', '',
       'authorization_code,client_credentials', 'profile', 7200, 604800, 'ENABLED', a.id
FROM applications a
WHERE a.code = 'legacy-jquery'
ON CONFLICT (client_id) DO NOTHING;
