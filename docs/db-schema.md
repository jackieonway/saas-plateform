# Dante Cloud — PostgreSQL 数据库设计

> 连接串: `postgres://dante@127.0.0.1:5432/dante_cloud` (本地 trust 认证;docker 部署时带密码)
> 使用 SQLx 迁移: `backend/migrations/0001_init.sql`(初始结构)及后续增量迁移；地点字段由 `0027_tenant_organization_location.sql` 增加。
> 所有主键 UUID (gen_random_uuid()),时间 timestamptz。

## 实体关系

```
applications 1─N menus (自引用树 parent_id)
tenants 1─N organizations (自引用多层级树 parent_id)
organizations 1─N departments (自引用树)
users N─N organizations (memberships, 含 department)
tenants N─N applications (tenant_applications 授权)
tenants N─N menus (tenant_menus 授权)
organizations N─N applications (organization_applications 授权, ⊆ 租户授权)
organizations N─N menus (organization_menus 授权, ⊆ 租户授权)
roles (scope: organization_id 必填) N─N menus (role_menus, ⊆ 组织授权)
users N─N roles (user_roles, organization_id 记录授权上下文)
oauth_clients / oauth_tokens / oauth_device_codes / oauth_authorization_codes / oauth_authorization_requests
audit_logs
```

## 表结构 (完整 SQL 由后端代理实现,此处为权威定义)

```sql
CREATE TABLE tenants (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name VARCHAR(100) NOT NULL,
  code VARCHAR(50) NOT NULL UNIQUE,
  description TEXT DEFAULT '',
  province VARCHAR(100) NOT NULL DEFAULT '',
  city VARCHAR(100) NOT NULL DEFAULT '',
  district VARCHAR(100) NOT NULL DEFAULT '',
  address VARCHAR(300) NOT NULL DEFAULT '',
  longitude DOUBLE PRECISION,
  latitude DOUBLE PRECISION,
  status VARCHAR(20) NOT NULL DEFAULT 'ENABLED',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE applications (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  code VARCHAR(50) NOT NULL UNIQUE,
  name VARCHAR(100) NOT NULL,
  icon VARCHAR(100) DEFAULT 'LayoutGrid',
  base_path VARCHAR(200) DEFAULT '',
  description TEXT DEFAULT '',
  sort_order INT NOT NULL DEFAULT 0,
  status VARCHAR(20) NOT NULL DEFAULT 'ENABLED',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE menus (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
  parent_id UUID REFERENCES menus(id) ON DELETE CASCADE,
  type VARCHAR(20) NOT NULL DEFAULT 'MENU',        -- DIR | MENU | BUTTON
  title VARCHAR(100) NOT NULL,
  path VARCHAR(200) DEFAULT '',
  component VARCHAR(200) DEFAULT '',
  icon VARCHAR(100) DEFAULT '',
  permission VARCHAR(100) DEFAULT '',
  sort_order INT NOT NULL DEFAULT 0,
  visible BOOLEAN NOT NULL DEFAULT true,
  status VARCHAR(20) NOT NULL DEFAULT 'ENABLED',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_menus_app ON menus(application_id);

CREATE TABLE organizations (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
  parent_id UUID REFERENCES organizations(id) ON DELETE CASCADE,
  name VARCHAR(100) NOT NULL,
  code VARCHAR(50) DEFAULT '',
  sort_order INT NOT NULL DEFAULT 0,
  description TEXT DEFAULT '',
  province VARCHAR(100) NOT NULL DEFAULT '',
  city VARCHAR(100) NOT NULL DEFAULT '',
  district VARCHAR(100) NOT NULL DEFAULT '',
  address VARCHAR(300) NOT NULL DEFAULT '',
  longitude DOUBLE PRECISION,
  latitude DOUBLE PRECISION,
  status VARCHAR(20) NOT NULL DEFAULT 'ENABLED',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_orgs_tenant ON organizations(tenant_id);

CREATE TABLE departments (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  parent_id UUID REFERENCES departments(id) ON DELETE CASCADE,
  name VARCHAR(100) NOT NULL,
  code VARCHAR(50) DEFAULT '',
  sort_order INT NOT NULL DEFAULT 0,
  description TEXT DEFAULT '',
  status VARCHAR(20) NOT NULL DEFAULT 'ENABLED',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE users (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  username VARCHAR(50) NOT NULL UNIQUE,
  password_hash TEXT NOT NULL,
  real_name VARCHAR(100) DEFAULT '',
  email VARCHAR(100) DEFAULT '',
  phone VARCHAR(30) DEFAULT '',
  avatar TEXT DEFAULT '',
  gender VARCHAR(10) DEFAULT 'UNKNOWN',   -- MALE | FEMALE | UNKNOWN
  status VARCHAR(20) NOT NULL DEFAULT 'ENABLED',
  is_super_admin BOOLEAN NOT NULL DEFAULT false,
  last_login_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE memberships (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  department_id UUID REFERENCES departments(id) ON DELETE SET NULL,
  position VARCHAR(50) DEFAULT '',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(user_id, organization_id)
);

CREATE TABLE roles (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
  organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,  -- 角色 scope
  name VARCHAR(100) NOT NULL,
  code VARCHAR(50) DEFAULT '',
  description TEXT DEFAULT '',
  status VARCHAR(20) NOT NULL DEFAULT 'ENABLED',
  built_in BOOLEAN NOT NULL DEFAULT false,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(organization_id, name)
);

CREATE TABLE role_menus (
  role_id UUID NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
  menu_id UUID NOT NULL REFERENCES menus(id) ON DELETE CASCADE,
  PRIMARY KEY (role_id, menu_id)
);

CREATE TABLE user_roles (
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role_id UUID NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
  organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, role_id)
);

CREATE TABLE tenant_applications (
  tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
  application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (tenant_id, application_id)
);

CREATE TABLE tenant_menus (
  tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
  menu_id UUID NOT NULL REFERENCES menus(id) ON DELETE CASCADE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (tenant_id, menu_id)
);

CREATE TABLE organization_applications (
  organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  application_id UUID NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (organization_id, application_id)
);

CREATE TABLE organization_menus (
  organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  menu_id UUID NOT NULL REFERENCES menus(id) ON DELETE CASCADE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (organization_id, menu_id)
);

-- ===== OAuth2 =====
CREATE TABLE oauth_clients (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  client_id VARCHAR(100) NOT NULL UNIQUE,
  client_secret_hash TEXT,                       -- PUBLIC 客户端为 NULL
  client_name VARCHAR(200) NOT NULL,
  description TEXT DEFAULT '',
  client_type VARCHAR(20) NOT NULL DEFAULT 'CONFIDENTIAL',  -- CONFIDENTIAL | PUBLIC
  redirect_uris TEXT DEFAULT '',                 -- 逗号分隔
  grant_types TEXT DEFAULT 'authorization_code,refresh_token', -- 逗号分隔
  scopes TEXT DEFAULT 'profile',                 -- 逗号分隔
  access_token_validity INT NOT NULL DEFAULT 7200,
  refresh_token_validity INT NOT NULL DEFAULT 604800,
  status VARCHAR(20) NOT NULL DEFAULT 'ENABLED',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE oauth_authorization_requests (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  client_id VARCHAR(100) NOT NULL,
  redirect_uri TEXT NOT NULL,
  scope TEXT NOT NULL DEFAULT '',
  state TEXT DEFAULT '',
  code_challenge TEXT DEFAULT '',
  code_challenge_method VARCHAR(10) DEFAULT '',
  expires_at TIMESTAMPTZ NOT NULL,
  consumed BOOLEAN NOT NULL DEFAULT false,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE oauth_authorization_codes (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  code_hash TEXT NOT NULL UNIQUE,
  client_id VARCHAR(100) NOT NULL,
  user_id UUID NOT NULL,
  tenant_id UUID, organization_id UUID,
  redirect_uri TEXT NOT NULL,
  scope TEXT DEFAULT '',
  code_challenge TEXT DEFAULT '',
  code_challenge_method VARCHAR(10) DEFAULT '',
  expires_at TIMESTAMPTZ NOT NULL,
  consumed BOOLEAN NOT NULL DEFAULT false,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE oauth_tokens (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  token_hash TEXT NOT NULL UNIQUE,
  token_type VARCHAR(20) NOT NULL,               -- access | refresh
  jwt TEXT,                                      -- access 存 JWT 本体便于 introspect 快速返回
  user_id UUID,                                  -- client_credentials 为 NULL
  client_id VARCHAR(100) NOT NULL,
  tenant_id UUID, organization_id UUID,
  scope TEXT DEFAULT '',
  expires_at TIMESTAMPTZ NOT NULL,
  revoked BOOLEAN NOT NULL DEFAULT false,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_tokens_user ON oauth_tokens(user_id);

CREATE TABLE oauth_device_codes (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  device_code_hash TEXT NOT NULL UNIQUE,
  user_code VARCHAR(20) NOT NULL UNIQUE,
  client_id VARCHAR(100) NOT NULL,
  scope TEXT DEFAULT '',
  status VARCHAR(20) NOT NULL DEFAULT 'PENDING', -- PENDING | APPROVED | DENIED | EXPIRED
  user_id UUID, tenant_id UUID, organization_id UUID,
  interval_seconds INT NOT NULL DEFAULT 5,
  last_polled_at TIMESTAMPTZ,
  expires_at TIMESTAMPTZ NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE audit_logs (
  id BIGSERIAL PRIMARY KEY,
  user_id UUID, username VARCHAR(50) DEFAULT '',
  tenant_id UUID, tenant_name VARCHAR(100) DEFAULT '',
  organization_id UUID, organization_name VARCHAR(100) DEFAULT '',
  action VARCHAR(50) NOT NULL,
  detail TEXT DEFAULT '',
  ip VARCHAR(50) DEFAULT '',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_audit_created ON audit_logs(created_at DESC);
```

## 种子数据 (0002_seed.sql,密码 Argon2 哈希由 Rust 启动时 upsert 生成)

- **租户** dante「Dante 云平台」
- **应用** ①system 系统管理(LayoutGrid, /system) ②athena 数据分析(BarChart3, /athena) ③hermes 消息中心(MessageSquare, /hermes)
- **system 菜单树**:
  - 仪表盘 MENU /dashboard permission dashboard:view
  - 组织机构 DIR:租户管理(/system/tenants, system:tenant) + 授权按钮(BUTTON system:tenant:auth)、组织管理(/system/organizations, system:organization)、部门管理(/system/departments, system:department)、授权管理(/system/authorizations, system:authorization)
  - 用户与角色 DIR:用户管理(/system/users, system:user)、角色管理(/system/roles, system:role)
  - 平台配置 DIR:应用管理(/system/applications, system:application)、菜单管理(/system/menus, system:menu)
  - 开放平台 DIR:OAuth 客户端(/system/oauth/clients, system:oauth)、设备与会话(/system/oauth/tokens, system:oauth)、审计日志(/system/audit-logs, system:audit)、平台监控(/system/monitor, system:monitor)
- **athena 菜单树**: 数据概览(/athena/overview)、报表中心 DIR(销售报表 /athena/reports/sales、运营报表 /athena/reports/operations)、数据源管理(/athena/sources)
- **hermes 菜单树**: 收件箱(/hermes/inbox)、通知设置(/hermes/notifications)
- **组织树**(dante 租户): 总公司 → {华东区分公司 → {上海办事处, 杭州办事处}, 华北区分公司 → {北京办事处}}
- **部门**(总公司下): 产品部、研发部(→前端组、后端组)、人事部;华东区分公司下:交付一组
- **用户**:
  | 用户 | 密码 | 说明 |
  |---|---|---|
  | admin | admin123 | 超级管理员,membership=总公司,体验全部功能 |
  | zhangsan | Zs@12345 | 华东区分公司 组织管理员(角色:组织管理员) |
  | lisi | Ls@12345 | 总公司 普通用户 |
  | wangwu | Ww@12345 | 总公司 + 华北区分公司 双 membership(演示登录选择) |
  | tianqi | Tq@12345 | 上海办事处 普通用户 |
- **角色**(dante 租户):
  - 超级管理员(scope 总公司, built_in, 全菜单)
  - 组织管理员(scope 华东区分公司: 组织机构+用户与角色模块菜单+仪表盘)
  - 普通用户(scope 总公司: 仪表盘+各应用基础菜单)
- **授权链演示**: 租户 dante 授权全部 3 应用全部菜单;总公司授权 system+athena+hermes 全部;华东区分公司授权 athena+hermes(无 system);华北区分公司仅 hermes;上海/杭州/北京办事处:华东/华北同款或更少(athena 或 hermes)
- **OAuth 客户端**:
  | client_id | 类型 | 授权 | 用途 |
  |---|---|---|---|
  | dante-web | CONFIDENTIAL | authorization_code,refresh_token + PKCE | Web 应用演示,redirect_uri: https://oauthdebugger.com/callback,http://localhost:9000/callback |
  | smart-tv | PUBLIC | urn:ietf:params:oauth:grant-type:device_code,refresh_token | 智能电视等输入设备码设备 |
  | api-service | CONFIDENTIAL | client_credentials | 服务间调用 |
  | resource-portal | CONFIDENTIAL | authorization_code,password,refresh_token | 密码模式演示 |

> 种子里用户密码哈希:后端 `main.rs` 启动时若 users 表为空则执行种子(密码用 Argon2 现算);SQL 迁移只建表。种子逻辑集中在 `src/seed.rs`,幂等(存在即跳过)。dev 默认 client secrets: dante-web=dante-web-secret, api-service=api-service-secret, resource-portal=resource-portal-secret。

## 0013 结构约束补充

后续迁移 [`0013_tenant_relationship_integrity.sql`](../backend/migrations/0013_tenant_relationship_integrity.sql) 在现有表上增加同租户组织父节点、同组织部门父节点、成员部门归属、角色租户/组织归属的复合外键，并用触发器阻止组织和部门树形成环路。存量库执行前请先运行[只读关系审计](./sql/tenant-relationship-audit.sql)，按[迁移说明](./tenant-relationship-migration.md)处理异常；本节不改变上面的初始结构 SQL。

## 0014～0015 平台会话补充

[`0014_platform_sessions.sql`](../backend/migrations/0014_platform_sessions.sql) 为 `users` 增加 `session_version`，并建立 `platform_sessions`（会话 ID、用户、租户、组织、签发/到期/撤销时间、IP、UA）。[`0015_platform_session_invalidation.sql`](../backend/migrations/0015_platform_session_invalidation.sql) 增加组织祖先链有效性函数及成员删除、用户禁用、租户/组织状态与到期时间变更的会话撤销或到期收紧触发器。旧迁移保持原样；详细语义见[平台会话生命周期](./platform-session-lifecycle.md)。

## 0016～0019 应用令牌补充

[`0016_application_token_binding.sql`](../backend/migrations/0016_application_token_binding.sql) 为 `oauth_tokens` 增加 `token_use`、`audience`、唯一 `jti`、`application_id` 和 `platform_session_id`；专用应用令牌均需完整绑定，明文 `jwt` 列保持空值。[`0017_application_token_revocation.sql`](../backend/migrations/0017_application_token_revocation.sql) 在两级应用授权删除及应用/客户端禁用时撤销旧令牌。[`0018_revoke_legacy_app_tokens.sql`](../backend/migrations/0018_revoke_legacy_app_tokens.sql) 撤销缺少受众声明的历史应用用户 access token；服务 `client_credentials` 令牌保留。[`0019_application_token_config_revocation.sql`](../backend/migrations/0019_application_token_config_revocation.sql) 在应用 code 或客户端绑定、类型、scope、TTL 变更时撤销旧应用令牌，配置恢复不会复活旧令牌。接入语义见[应用令牌契约](./application-token-contract.md)。

## 0020 独立数据授权

[`0020_membership_data_grants.sql`](../backend/migrations/0020_membership_data_grants.sql) 创建 `membership_data_grants`：按成员关系、应用、资源和操作显式授予 `SELF/ORG/ORG_SUBTREE/TENANT` 范围，缺失即拒绝。数据授权不引用角色，删除成员时级联删除。授权上下文与业务资源边界见[应用授权上下文契约](./app-authorization-context.md)。
# 地区数据版本与初始化

`region_versions` 保存版本标识、显示名称和复制来源；`regions` 保存地区编码、父级编码、名称、拼音、层级、类型等字段。`regions(version, code)` 唯一，`source_id` 保留源记录主键。建表与字段备注见 `0028_region_catalog.sql`，唯一索引与数据修正见 `0029_region_code_unique.sql`、`0030_region_catalog_comment.sql`。

内置 `2023` 版来自 `docs/t_region.sql`。源文件有 688,939 行，其中 23,016 行的编码和业务字段完全重复；`scripts/prepare-region-seed.py` 去重后生成 665,923 行的 `backend/assets/regions-2023.copy.gz`。后端启动时在事务中流式导入一次；后续启动不会覆盖后台维护过的数据。新版本可创建空版本或复制现有版本，复制出的版本需要自行核对并维护，不能视为新的官方区划数据。
