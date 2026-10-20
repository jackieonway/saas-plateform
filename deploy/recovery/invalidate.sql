BEGIN;
SET LOCAL lock_timeout = '10s';
UPDATE users SET session_version = session_version + 1, updated_at = now();
UPDATE platform_sessions SET revoked_at = now() WHERE revoked_at IS NULL;
UPDATE platform_session_refresh_tokens SET consumed_at = now() WHERE consumed_at IS NULL;
UPDATE oauth_tokens SET revoked = true WHERE revoked = false;
UPDATE oauth_authorization_codes SET consumed = true WHERE consumed = false;
UPDATE oauth_authorization_requests SET consumed = true WHERE consumed = false;
UPDATE oauth_device_codes SET status = 'EXPIRED', expires_at = LEAST(expires_at,now());
INSERT INTO audit_logs(action,detail,username) VALUES
('RECOVERY_RESET','Offline isolated restore: all prior credentials invalidated before traffic acceptance','recovery-operator');
COMMIT;
