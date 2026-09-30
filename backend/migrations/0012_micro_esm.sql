-- 0012 微前端补强: base_path 冲突修复 + ESM 通道第一方子应用种子
-- 背景:
--   ① 0011 把两个微应用 base_path 都种成 '/micro', 导致主应用 AppRail 切换时
--      「pathname.startsWith(basePath + '/')」前缀误判, 从 /micro/vue-workshop 切
--      legacy-jquery 时被判为「已在该应用」而拒绝导航(侧栏已切/主区域不动)。
--      修复: 每个微应用独占自己的前缀(与 athena/hermes 同款约定)。
--      不改 0011 已应用文件(sqlx 校验和), 用 UPDATE 修正(新旧库均幂等)。
--   ② ESM 通道(第一方直挂, 无沙箱)此前无实际使用方: 新增 report-center 种子,
--      entry_url 指向主应用 Vite dev server 的模块 /src/micro-apps/report-center.tsx。
-- 幂等设计: UPDATE 天然幂等 + INSERT 按 code/title 幂等(与 0011 同风格)。

-- 1) base_path 冲突修复
UPDATE applications SET base_path = '/micro/vue-workshop'  WHERE code = 'vue-workshop';
UPDATE applications SET base_path = '/micro/legacy-jquery' WHERE code = 'legacy-jquery';

-- 2) 报表中心应用(第一方 ESM 通道, 按幂等插入; sort_order 取最大 + 1)
INSERT INTO applications (code, name, icon, base_path, entry_type, entry_url, description, sort_order)
SELECT 'report-center', '报表中心', 'BarChart3', '/micro/report-center', 'ESM', '/src/micro-apps/report-center.tsx',
       '微前端演示: 第一方 ESM 通道子应用(共享运行时·无沙箱)',
       (SELECT COALESCE(MAX(sort_order), 0) + 1 FROM applications)
WHERE NOT EXISTS (SELECT 1 FROM applications WHERE code = 'report-center');

-- 3) 种子菜单 (link_target='MICRO', path 填 /micro/report-center[/子路径])
INSERT INTO menus (application_id, parent_id, type, title, path, icon, component, permission, sort_order, visible, link_url, link_target)
SELECT a.id, NULL, 'MENU', '数据总览', '/micro/report-center', '', '', '', 1, true, '', 'MICRO'
FROM applications a
WHERE a.code = 'report-center'
  AND NOT EXISTS (SELECT 1 FROM menus m WHERE m.application_id = a.id AND m.title = '数据总览');

INSERT INTO menus (application_id, parent_id, type, title, path, icon, component, permission, sort_order, visible, link_url, link_target)
SELECT a.id, NULL, 'MENU', '共享运行时', '/micro/report-center/shared', '', '', '', 2, true, '', 'MICRO'
FROM applications a
WHERE a.code = 'report-center'
  AND NOT EXISTS (SELECT 1 FROM menus m WHERE m.application_id = a.id AND m.title = '共享运行时');

-- 4) 角色与授权链绑定 (照抄 0011 手法: super_admin 角色 + dante 租户 + 总公司)
INSERT INTO role_menus (role_id, menu_id)
SELECT r.id, m.id
FROM roles r
JOIN organizations o ON o.id = r.organization_id
JOIN tenants t ON t.id = o.tenant_id
JOIN menus m ON m.application_id IN (SELECT id FROM applications WHERE code IN ('vue-workshop', 'legacy-jquery', 'report-center'))
WHERE t.code = 'dante' AND o.name = '总公司' AND r.code = 'super_admin'
ON CONFLICT DO NOTHING;

INSERT INTO tenant_applications (tenant_id, application_id)
SELECT t.id, a.id
FROM tenants t
JOIN applications a ON a.code IN ('vue-workshop', 'legacy-jquery', 'report-center')
WHERE t.code = 'dante'
ON CONFLICT DO NOTHING;

INSERT INTO tenant_menus (tenant_id, menu_id)
SELECT t.id, m.id
FROM tenants t
JOIN menus m ON m.application_id IN (SELECT id FROM applications WHERE code IN ('vue-workshop', 'legacy-jquery', 'report-center'))
WHERE t.code = 'dante'
ON CONFLICT DO NOTHING;

INSERT INTO organization_applications (organization_id, application_id)
SELECT o.id, a.id
FROM organizations o
JOIN applications a ON a.code IN ('vue-workshop', 'legacy-jquery', 'report-center')
WHERE o.tenant_id = (SELECT id FROM tenants WHERE code = 'dante') AND o.name = '总公司'
ON CONFLICT DO NOTHING;

INSERT INTO organization_menus (organization_id, menu_id)
SELECT o.id, m.id
FROM organizations o
JOIN menus m ON m.application_id IN (SELECT id FROM applications WHERE code IN ('vue-workshop', 'legacy-jquery', 'report-center'))
WHERE o.tenant_id = (SELECT id FROM tenants WHERE code = 'dante') AND o.name = '总公司'
ON CONFLICT DO NOTHING;

-- 5) OAuth 外壳客户端 (会话签发用, 与 0011 两壳同规格)
INSERT INTO oauth_clients (client_id, client_secret_hash, client_name, description, client_type,
                           redirect_uris, grant_types, scopes, access_token_validity, refresh_token_validity, status, application_id)
SELECT 'micro-report-shell', encode(sha256('micro-report-shell-secret'::bytea), 'hex'), '微前端外壳·报表',
       '微前端 ESM 通道子应用会话令牌签发客户端 (report-center)', 'CONFIDENTIAL', '',
       'authorization_code,client_credentials', 'profile', 7200, 604800, 'ENABLED', a.id
FROM applications a
WHERE a.code = 'report-center'
ON CONFLICT (client_id) DO NOTHING;
