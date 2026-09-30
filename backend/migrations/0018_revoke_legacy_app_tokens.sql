-- 旧 /api/oauth/session-token 未带 audience/jti，无法安全用于独立应用校验。
-- 关联应用的用户 access token 在升级时统一撤销；用户重新签发 5 分钟应用令牌。
UPDATE oauth_tokens tok SET revoked = true
FROM oauth_clients client
WHERE tok.client_id = client.client_id
  AND client.application_id IS NOT NULL
  AND tok.user_id IS NOT NULL
  AND tok.token_type = 'access'
  AND tok.token_use = 'oauth'
  AND tok.revoked = false;
