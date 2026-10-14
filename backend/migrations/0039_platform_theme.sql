ALTER TABLE platform_settings
    ADD COLUMN primary_color VARCHAR(7) NOT NULL DEFAULT '#008577'
        CHECK (primary_color ~ '^#[0-9A-Fa-f]{6}$');

COMMENT ON COLUMN platform_settings.primary_color IS '平台主题主色（#RRGGBB）；前端生成兼顾对比度的明暗主题与导航配色';
