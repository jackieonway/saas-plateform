# 平台品牌与管理租户设置

平台管理员可以进入 **系统管理 → 平台配置 → 平台设置**（`/system/settings`），修改平台名称、平台简介、Logo 地址、登录标题、登录说明、页脚文案及平台租户名称。设置保存在 PostgreSQL 中，重启后继续生效。

平台名称用于登录页、导航栏、浏览器标题、OAuth 授权与设备页面、系统监控的服务名称，以及新账号的欢迎通知。登录说明、页脚和 Logo 从同一份配置读取。后台保存后当前页面立即更新；其他已打开的页面会在重新获取配置后更新。通知和审计记录中的历史文案保留创建时的内容。

平台租户编码固定为 `admin-tenant`，平台租户名称可以在此页面或租户管理的编辑弹窗中修改。名称直接更新 `tenants` 表中的平台租户记录，登录资料、组织选择及租户列表均使用该记录；租户 ID 和组织、用户、角色、授权关系保持连续。

`0032_platform_settings.sql` 为已有安装新增配置和菜单，并按 `is_platform` 标记迁移平台租户编码。旧默认名称 `Dante 云平台` 更新为 `平台管理租户`，已有自定义名称保留。如果普通租户已经占用 `admin-tenant`，迁移会失败，需要先为该普通租户选择其他编码后重试。升级须重新构建并启动后端，SQLx 将自动执行新迁移；不要修改已执行的历史迁移文件。

## 接口

| 接口 | 访问要求 | 内容 |
| --- | --- | --- |
| `GET /api/platform/branding` | 无需登录 | 公开的六个展示字段 |
| `GET /api/platform/settings` | 平台管理上下文和 `system:settings` 权限 | 展示字段、平台租户名称和固定编码 |
| `PUT /api/platform/settings` | 平台管理上下文和 `system:settings` 权限 | 原子保存展示字段和平台租户名称，并记录审计日志 |

公开字段是 `platformName`、`platformDescription`、`logoUrl`、`loginTitle`、`loginDescription`、`footerText`。管理接口额外返回 `platformTenantName` 和 `platformTenantCode`。保存时提交完整的七个可写字段，省略 `platformTenantCode`；服务端拒绝未知字段，租户管理接口也禁止把平台租户编码改成其他值。

```json
{
  "platformName": "企业管理平台",
  "platformDescription": "租户、组织与应用统一管理",
  "logoUrl": "/assets/company-logo.png",
  "loginTitle": "欢迎登录",
  "loginDescription": "使用企业账号管理您的租户与应用",
  "footerText": "© 2026 企业管理平台",
  "platformTenantName": "企业平台管理租户"
}
```

Logo 地址支持以 `/` 开头的同源资源地址及 HTTP/HTTPS 图片 URL；留空时使用内置图标。名称与文案按普通文本显示，非法地址、控制字符、空名称和超长字段会被拒绝，校验失败不会产生部分保存。

## 验证

前端运行 `npm run build`；后端运行 `cargo test --locked`。数据库回归测试通过 `TEST_DATABASE_URL` 或根目录的 `.env.test` 连接 PostgreSQL，并创建独立临时数据库，显式运行 `cargo test --locked platform_settings_tests -- --ignored`。浏览器回归由 `node tests/platform-settings-browser.mjs` 执行。
