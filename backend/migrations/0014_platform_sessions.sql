-- 平台 session 持久化与统一撤销版本；旧 session JWT 不带 sid/ver，部署后须重新登录。
ALTER TABLE users
    ADD COLUMN session_version BIGINT NOT NULL DEFAULT 0
    CHECK (session_version >= 0);

CREATE TABLE platform_sessions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    ip VARCHAR(50) NOT NULL DEFAULT '',
    user_agent TEXT NOT NULL DEFAULT '',
    CHECK (expires_at > issued_at)
);

CREATE INDEX idx_platform_sessions_user_active
    ON platform_sessions (user_id, expires_at DESC)
    WHERE revoked_at IS NULL;

CREATE INDEX idx_platform_sessions_organization_active
    ON platform_sessions (organization_id, expires_at DESC)
    WHERE revoked_at IS NULL;
