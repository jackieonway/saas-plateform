-- 应用专用 access token 与签发会话、应用受众和唯一令牌标识绑定。
ALTER TABLE oauth_tokens
    ADD COLUMN token_use VARCHAR(20) NOT NULL DEFAULT 'oauth',
    ADD COLUMN audience VARCHAR(100),
    ADD COLUMN jti UUID,
    ADD COLUMN application_id UUID REFERENCES applications(id) ON DELETE CASCADE,
    ADD COLUMN platform_session_id UUID REFERENCES platform_sessions(id) ON DELETE CASCADE;

ALTER TABLE oauth_tokens
    ADD CONSTRAINT ck_oauth_tokens_app_binding
    CHECK (token_use <> 'app' OR (
        token_type = 'access' AND user_id IS NOT NULL AND tenant_id IS NOT NULL
        AND organization_id IS NOT NULL AND audience IS NOT NULL AND jti IS NOT NULL
        AND application_id IS NOT NULL AND platform_session_id IS NOT NULL
    ));

CREATE UNIQUE INDEX uq_oauth_tokens_jti
    ON oauth_tokens (jti) WHERE jti IS NOT NULL;

CREATE INDEX idx_oauth_tokens_app_active
    ON oauth_tokens (application_id, expires_at DESC)
    WHERE token_use = 'app' AND revoked = false;
