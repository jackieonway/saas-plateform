-- 0008 菜单外链支持
-- 需求: 菜单支持外链, 可配置「新窗口打开」(BLANK) 或「内部嵌入」(SELF)。
-- 兼容设计: link_url 空串 = 非外链(存量菜单不受影响, 前端按原路由行为跳转);
--           link_target 仅 BLANK/SELF 两值, 后端写入前校验非法值回退 BLANK。
-- 幂等设计: ADD COLUMN IF NOT EXISTS, 语句可重复执行不报错(与 0006 同风格)。

-- 1) 外链地址: 空 = 非外链; 非空时后端约束必须 http(s):// 开头, 长度上限 500
ALTER TABLE menus ADD COLUMN IF NOT EXISTS link_url VARCHAR(500) NOT NULL DEFAULT '';

-- 2) 打开方式: BLANK = 新窗口打开, SELF = 内部嵌入(/embed 路由 iframe)
ALTER TABLE menus ADD COLUMN IF NOT EXISTS link_target VARCHAR(10) NOT NULL DEFAULT 'BLANK';
