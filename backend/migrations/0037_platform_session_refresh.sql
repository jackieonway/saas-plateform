-- 平台刷新令牌独立于 OAuth 应用令牌，只持久化摘要，绑定既有会话与撤销版本。
CREATE TABLE platform_session_refresh_tokens (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id UUID NOT NULL REFERENCES platform_sessions(id) ON DELETE CASCADE,
    token_hash VARCHAR(64) NOT NULL UNIQUE,
    session_version BIGINT NOT NULL CHECK (session_version >= 0),
    issued_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    CHECK (expires_at > issued_at)
);

CREATE INDEX idx_platform_session_refresh_tokens_session
    ON platform_session_refresh_tokens (session_id, expires_at DESC);
