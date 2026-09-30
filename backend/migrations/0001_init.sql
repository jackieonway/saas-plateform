-- Dante Cloud — 初始结构 (权威定义见 docs/db-schema.md)
-- 种子数据由 Rust (src/seed.rs) 在启动时写入 (密码需 Argon2 现算), 见 0002_seed.sql 注释

CREATE TABLE tenants (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name VARCHAR(100) NOT NULL,
  code VARCHAR(50) NOT NULL UNIQUE,
  description TEXT DEFAULT '',
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
  type VARCHAR(20) NOT NULL DEFAULT 'MENU',
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
CREATE INDEX idx_menus_parent ON menus(parent_id);

CREATE TABLE organizations (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
  parent_id UUID REFERENCES organizations(id) ON DELETE CASCADE,
  name VARCHAR(100) NOT NULL,
  code VARCHAR(50) DEFAULT '',
  sort_order INT NOT NULL DEFAULT 0,
  description TEXT DEFAULT '',
  status VARCHAR(20) NOT NULL DEFAULT 'ENABLED',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_orgs_tenant ON organizations(tenant_id);
CREATE INDEX idx_orgs_parent ON organizations(parent_id);

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
CREATE INDEX idx_departments_org ON departments(organization_id);
CREATE INDEX idx_departments_parent ON departments(parent_id);

CREATE TABLE users (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  username VARCHAR(50) NOT NULL UNIQUE,
  password_hash TEXT NOT NULL,
  real_name VARCHAR(100) DEFAULT '',
  email VARCHAR(100) DEFAULT '',
  phone VARCHAR(30) DEFAULT '',
  avatar TEXT DEFAULT '',
  gender VARCHAR(10) DEFAULT 'UNKNOWN',
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
CREATE INDEX idx_memberships_user ON memberships(user_id);
CREATE INDEX idx_memberships_org ON memberships(organization_id);

CREATE TABLE roles (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
  organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  name VARCHAR(100) NOT NULL,
  code VARCHAR(50) DEFAULT '',
  description TEXT DEFAULT '',
  status VARCHAR(20) NOT NULL DEFAULT 'ENABLED',
  built_in BOOLEAN NOT NULL DEFAULT false,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(organization_id, name)
);
CREATE INDEX idx_roles_org ON roles(organization_id);
CREATE INDEX idx_roles_tenant ON roles(tenant_id);

CREATE TABLE role_menus (
  role_id UUID NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
  menu_id UUID NOT NULL REFERENCES menus(id) ON DELETE CASCADE,
  PRIMARY KEY (role_id, menu_id)
);
CREATE INDEX idx_role_menus_menu ON role_menus(menu_id);

CREATE TABLE user_roles (
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role_id UUID NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
  organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, role_id)
);
CREATE INDEX idx_user_roles_role ON user_roles(role_id);
CREATE INDEX idx_user_roles_org ON user_roles(organization_id);

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
CREATE INDEX idx_tenant_menus_menu ON tenant_menus(menu_id);

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
CREATE INDEX idx_org_menus_menu ON organization_menus(menu_id);

-- ===== OAuth2 =====
CREATE TABLE oauth_clients (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  client_id VARCHAR(100) NOT NULL UNIQUE,
  client_secret_hash TEXT,
  client_name VARCHAR(200) NOT NULL,
  description TEXT DEFAULT '',
  client_type VARCHAR(20) NOT NULL DEFAULT 'CONFIDENTIAL',
  redirect_uris TEXT DEFAULT '',
  grant_types TEXT DEFAULT 'authorization_code,refresh_token',
  scopes TEXT DEFAULT 'profile',
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
  token_type VARCHAR(20) NOT NULL,
  jwt TEXT,
  user_id UUID,
  client_id VARCHAR(100) NOT NULL,
  tenant_id UUID, organization_id UUID,
  scope TEXT DEFAULT '',
  expires_at TIMESTAMPTZ NOT NULL,
  revoked BOOLEAN NOT NULL DEFAULT false,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_tokens_user ON oauth_tokens(user_id);
CREATE INDEX idx_tokens_client ON oauth_tokens(client_id);

CREATE TABLE oauth_device_codes (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  device_code_hash TEXT NOT NULL UNIQUE,
  user_code VARCHAR(20) NOT NULL UNIQUE,
  client_id VARCHAR(100) NOT NULL,
  scope TEXT DEFAULT '',
  status VARCHAR(20) NOT NULL DEFAULT 'PENDING',
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
CREATE INDEX idx_audit_action ON audit_logs(action);

-- 组织层级路径, 如 "总公司 / 华东区分公司 / 上海办事处"
CREATE OR REPLACE FUNCTION org_path(org uuid) RETURNS text AS $$
  WITH RECURSIVE up AS (
    SELECT id, name, parent_id, 1 AS d FROM organizations WHERE id = org
    UNION ALL
    SELECT o.id, o.name, o.parent_id, up.d + 1 FROM organizations o JOIN up ON o.id = up.parent_id
  )
  SELECT COALESCE(string_agg(name, ' / ' ORDER BY d DESC), '') FROM up;
$$ LANGUAGE sql IMMUTABLE;
