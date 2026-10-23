# OAuth 与应用令牌签发

平台同时提供普通 OAuth 授权、平台会话刷新和 qiankun 应用专用令牌。它们共用部分传输入口，但身份、存储用途和验证规则不同。当前实现位于 `src/routes/oauth2.rs`、`src/oauth_issuance.rs`、`src/session.rs`、`src/app_token.rs`；客户端管理位于 `src/routes/oauth_admin.rs`。

## 路由与用途

| 路由 | 用途 |
| --- | --- |
| `GET /oauth/authorize` | 授权码请求，进入平台同意流程 |
| `POST /oauth/token` | 表单提交授权码、刷新令牌、机器凭据、密码或设备码 |
| `GET /oauth/userinfo` | 验证访问令牌并返回对应身份信息 |
| `POST /oauth/introspect` | 经过机密客户端认证后查询令牌当前状态 |
| `POST /oauth/revoke` | 经过客户端认证后撤销属于该客户端的令牌 |
| `POST /oauth/device/authorization` | 发起设备授权 |
| `GET /oauth/.well-known/openid-configuration` | 当前端点、grant 和 PKCE 方法声明 |
| `GET /api/oauth/consent/{request_id}` | 平台会话读取待同意请求 |
| `POST /api/oauth/consent/{request_id}/decision` | 当前会话确认或拒绝请求 |
| `POST /api/oauth/session-token` | 当前平台会话换取应用专用令牌 |

`/oauth/token` 的普通 OAuth grant 为 `authorization_code`、`refresh_token`、`client_credentials`、`password`、`urn:ietf:params:oauth:grant-type:device_code`。客户端必须启用对应 grant。机密客户端验证密钥；公共客户端没有可保密的密钥，不能申请机器凭据或刷新无用户的机器凭据。

## 普通 OAuth 签发的一致性

普通用户 grant 必须同时具有 user、tenant、organization，且用户启用、直接成员关系存在、租户有效、组织到根的完整链启用且未到期、客户端启用。机器 grant 三个身份字段均为空，其主体为 client ID，不随无关用户或组织状态失效。

签发事务按用户、客户端、一次性凭据的顺序获取必要锁，等待后重查当前身份及客户端配置。授权码、设备码只能消费一次；刷新会消费旧刷新令牌，签发完整替换对。凭据消费、access 写入、可选 refresh 写入及相应事务审计必须一起提交，任一步骤失败全部回滚。

访问令牌使用带随机 `jti` 的 JWT；普通 OAuth 刷新令牌使用随机不透明字符串。新记录的 `token_use='oauth'`、`storage_version=2`、`jwt=NULL`，数据库只保存令牌摘要。客户端启用了 `refresh_token` 才附带刷新令牌。已知租户或祖先组织到期时间会进一步限制两个令牌的到期时间，后续禁用或撤销仍通过实时检查生效。

普通 token 响应使用 OAuth 命名：

```json
{
  "access_token": "<access-token>",
  "token_type": "Bearer",
  "expires_in": 3600,
  "scope": "read",
  "refresh_token": "<only-when-enabled>"
}
```

`expires_in` 来自当前有效期计算，不保证等于配置值。成功的 `/oauth/token` 响应带 `Cache-Control: no-store` 和 `Pragma: no-cache`。刷新请求可以收窄 scope，不能扩大 scope；空客户端 scope 按空集合处理。

## 授权码与 PKCE

客户端公开字段 `requirePkce` 对应数据库 `require_pkce`。通过管理接口创建的新公共客户端默认要求 S256；明确关闭的历史客户端保留可选 PKCE 行为。`requirePkce=true` 时，授权请求必须携带规范 S256 challenge，换码必须使用符合 RFC 7636 长度和字符规则的 verifier，禁止降级到 `plain`。

待同意请求和授权码兑换会重查当前 redirect、grant、scope 和 PKCE 策略。管理员收紧策略或移除 redirect 后，先前创建的请求不获得豁免。发现文档同时列出 S256 与 plain，具体客户端仍以其策略为准。

`migrations/0003_oauth_issuance.sql` 保留历史 token 元数据；`0007_oauth_pkce_policy.sql` 增加 PKCE 策略。旧普通 OAuth 无 `jti` 的令牌及旧明文字段属于受控存储兼容，不代表前端仍支持 iframe 或旧 SDK。不要通过修改历史迁移或直接清空令牌表来做 qiankun 文档清理。

## 平台会话刷新

当 `grant_type=refresh_token` 且 refresh token 属于平台会话格式时，`/oauth/token` 分派给 `session::refresh`。此分支禁止 OAuth `client_id`、`client_secret`、Basic 凭据和 scope 参数，返回新的平台会话 access/refresh 及 `refresh_expires_in`。平台刷新凭据不能转换成普通 OAuth grant。

## qiankun 应用专用令牌

```http
POST /api/oauth/session-token
Authorization: Bearer <platform-session-access-token>
Content-Type: application/json

{"appCode":"work-orders"}
```

平台要求应用启用、当前租户和所选组织均授权该应用，并选择关联该应用、已启用且创建时间最早的机密客户端。成功为 HTTP 201：

```json
{
  "accessToken": "<application-access-token>",
  "expiresIn": 299,
  "tokenType": "Bearer",
  "audience": "work-orders"
}
```

有效期取客户端 access TTL、300 秒和父平台会话剩余时间的最小值。记录使用 `token_use='app'`，绑定应用 audience、`jti` 和 `platform_session_id`，不签发应用 refresh token。后续通过 SDK 重新向壳请求短期 token。该接口是当前 qiankun 运行链路的一部分。

应用上下文、业务授权和通知发布均使用实时 `app_token::inspect`，校验令牌、父会话、成员关系和两级应用授权。契约详见[应用授权契约](application-authorization-contract.md)。应用内省要求调用方的应用绑定匹配，不能只凭 client ID 或 context hash 访问其他应用。

## 管理、限流与排查

客户端管理在 `/api/oauth/clients`；令牌管理在 `/api/oauth/tokens`，包括单令牌撤销、会话踢出和过期清理；设备码管理在 `/api/oauth/device-codes`。管理权限除 `system:oauth` 外还要求对应客户端或令牌菜单路径。客户端列表不返回密钥摘要，创建和重生成密钥时应由调用方安全接收一次性结果。

普通 OAuth 错误使用 `error`、`error_description`，受保护平台 API 使用 `code`、`message`。无效或未知 token 的内省通常为 `{ "active": false }`；撤销未知或不属于调用客户端的 token 不披露其存在。发生限流应遵循 `Retry-After`。

内省和机密授权相关配置：`OAUTH_INTROSPECTION_CLIENT_REQUESTS_PER_MINUTE=600`、`OAUTH_AUTH_FAILURE_REQUESTS_PER_MINUTE=30`、`OAUTH_INTROSPECTION_ATTEMPTS_PER_MINUTE=3000`。配置从服务端加载，不能由请求扩大预算。`OAUTH_AUTO_PURGE` 默认关闭。

关键验证位于 `src/oauth_issuance_tests.rs`、`src/platform_session_refresh_tests.rs`、`src/management_audit_tests.rs`：涵盖并发刷新单赢家、一次性消费、写入回滚、身份撤销、PKCE 收紧、旧存储升级和独立令牌撤销。数据库套件需隔离测试数据库；文档变更本身不等于这些集成测试已执行。
