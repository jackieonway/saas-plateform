-- Branding is global platform configuration. Tenant identity stays in tenants,
-- so renaming the platform tenant preserves every UUID and existing relation.
DO $$
DECLARE
    platform_id UUID;
BEGIN
    SELECT id INTO platform_id FROM tenants WHERE is_platform;
    IF platform_id IS NOT NULL THEN
        IF EXISTS (SELECT 1 FROM tenants WHERE code = 'admin-tenant' AND id <> platform_id) THEN
            RAISE EXCEPTION 'PLATFORM_TENANT_CODE_CONFLICT: admin-tenant is already used by a non-platform tenant; resolve that tenant code before upgrading';
        END IF;
        UPDATE tenants SET code = 'admin-tenant',
            name = CASE WHEN name = 'Dante 云平台' THEN '平台管理租户' ELSE name END,
            updated_at = now()
        WHERE id = platform_id;
    END IF;
END $$;

CREATE TABLE platform_settings (
    singleton BOOLEAN PRIMARY KEY DEFAULT true CHECK (singleton),
    platform_name VARCHAR(100) NOT NULL DEFAULT 'Dante Cloud',
    platform_description VARCHAR(300) NOT NULL DEFAULT '多租户权限管理平台',
    logo_url VARCHAR(2048) NOT NULL DEFAULT '',
    login_title VARCHAR(100) NOT NULL DEFAULT '欢迎回来',
    login_description VARCHAR(300) NOT NULL DEFAULT '管理您的租户与应用权限',
    footer_text VARCHAR(500) NOT NULL DEFAULT '© 2026 Dante Cloud · Powered by Rust & React',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
COMMENT ON TABLE platform_settings IS '平台全局品牌与登录页配置；唯一单行记录';
COMMENT ON COLUMN platform_settings.singleton IS '固定为 true，保证最多一条平台设置';
COMMENT ON COLUMN platform_settings.platform_name IS '平台展示名称';
COMMENT ON COLUMN platform_settings.platform_description IS '平台展示描述';
COMMENT ON COLUMN platform_settings.logo_url IS '平台标识的站内路径或 HTTP/HTTPS 地址';
COMMENT ON COLUMN platform_settings.login_title IS '登录页标题';
COMMENT ON COLUMN platform_settings.login_description IS '登录页说明';
COMMENT ON COLUMN platform_settings.footer_text IS '登录页页脚文案';
COMMENT ON COLUMN platform_settings.updated_at IS '最后修改时间';
INSERT INTO platform_settings (singleton) VALUES (true) ON CONFLICT DO NOTHING;

UPDATE oauth_clients SET client_name = 'Web 控制台', updated_at = now()
    WHERE client_id = 'dante-web' AND client_name = 'Dante Web 控制台';

-- Existing installations get a settings menu without rerunning initialization.
-- Empty databases create their platform, system application and menu in seed.rs.
DO $$
DECLARE
    system_app UUID;
    settings_menu UUID;
    settings_parent UUID;
BEGIN
    SELECT id INTO system_app FROM applications WHERE code = 'system';
    IF system_app IS NULL THEN
        RETURN;
    END IF;
    SELECT id INTO settings_parent FROM menus
        WHERE application_id = system_app AND type = 'DIR' AND title = '平台配置'
        ORDER BY created_at, id LIMIT 1;
    SELECT id INTO settings_menu FROM menus
        WHERE application_id = system_app AND path = '/system/settings'
        ORDER BY created_at, id LIMIT 1;
    IF settings_menu IS NULL THEN
        INSERT INTO menus (application_id, parent_id, type, title, path, component, icon, permission, sort_order, visible)
        VALUES (system_app, settings_parent, 'MENU', '平台设置', '/system/settings', '', 'Settings', 'system:settings', 3, true)
        RETURNING id INTO settings_menu;
    ELSE
        UPDATE menus SET permission = 'system:settings', updated_at = now() WHERE id = settings_menu;
    END IF;

    INSERT INTO tenant_applications (tenant_id, application_id)
        SELECT id, system_app FROM tenants WHERE is_platform
        ON CONFLICT DO NOTHING;
    INSERT INTO organization_applications (organization_id, application_id)
        SELECT o.id, system_app FROM organizations o JOIN tenants t ON t.id = o.tenant_id WHERE t.is_platform
        ON CONFLICT DO NOTHING;
    INSERT INTO tenant_menus (tenant_id, menu_id)
        SELECT t.id, m.id FROM tenants t CROSS JOIN menus m
        WHERE t.is_platform AND (m.id = settings_menu OR m.id = settings_parent)
        ON CONFLICT DO NOTHING;
    INSERT INTO organization_menus (organization_id, menu_id)
        SELECT o.id, m.id FROM organizations o JOIN tenants t ON t.id = o.tenant_id CROSS JOIN menus m
        WHERE t.is_platform AND (m.id = settings_menu OR m.id = settings_parent)
        ON CONFLICT DO NOTHING;
    INSERT INTO role_menus (role_id, menu_id)
        SELECT r.id, m.id FROM roles r JOIN tenants t ON t.id = r.tenant_id CROSS JOIN menus m
        WHERE t.is_platform AND r.built_in AND r.code = 'super_admin'
        AND (m.id = settings_menu OR m.id = settings_parent)
        ON CONFLICT DO NOTHING;
END $$;
