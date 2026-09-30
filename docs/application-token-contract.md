# 应用专用令牌接入契约（Phase 5）

独立部署的微应用无需放入宿主的 `sub-app` 目录。应用前端可由自己的域名或静态站点发布；宿主用 `appCode` 请求平台的应用令牌，再通过微应用 props 传入。业务后端必须独立校验令牌，不能只信任浏览器传来的用户、租户、组织或菜单字段。

## 注册与签发

1. 在 `applications` 注册唯一 `code`，保持应用 `ENABLED`；为该应用绑定一个已启用的 `CONFIDENTIAL` OAuth 客户端。客户端密钥只保存在业务后端的密钥管理或环境配置中，不放入前端包、props、URL 或日志。
2. 分别为租户和当前组织授权该应用（`tenant_applications`、`organization_applications`）。用户必须持有有效平台 session，并直接加入当前组织。
3. 宿主调用 `POST /api/oauth/session-token`，请求体 `{"appCode":"your-app"}`，以当前平台 session Bearer 认证。只有上述全部条件满足才返回 `201`：`{"accessToken":"...","tokenType":"Bearer","expiresIn":300,"audience":"your-app"}`。实际 `expiresIn` 不超过 300 秒，且不会超过客户端配置或平台 session 的剩余寿命。应用未授权、禁用或未关联可用机密客户端时返回 404。

专用令牌 JWT 包含 `sub`（用户）、`tenant`、`org`、`cid`、`aud`（应用 code）、唯一 `jti`、`iat`、`exp`。`oauth_tokens` 只保存该令牌的 SHA-256 哈希、应用/会话绑定和撤销元数据，`jwt` 列为空。多次签发产生不同 `jti`，可以逐个吊销并审计。

## 业务后端校验

业务后端每次请求以自己的机密客户端凭据调用平台 `POST /oauth/introspect`（`application/x-www-form-urlencoded`，参数 `client_id`、`client_secret`、`token`；正式环境使用 HTTPS）。只接受 `active: true` 且 `aud` 等于本应用 code、`tenant_id`/`organization_id` 与所访问资源归属一致的响应。响应还包含 `sub`、`jti`、`scope`、`exp`、`client_id`。A 应用的令牌由 B 应用客户端内省时返回 `{"active":false}`。刷新令牌也不会被该资源内省接口当作访问令牌。

平台内省会检查令牌签名、哈希库记录、有效期、撤销标记、机密客户端及应用状态、用户和直接成员状态、租户/组织祖先状态、两级应用授权及签发时的平台 session。平台会话登出、踢出、改密、成员移除或授权撤销后，下一次内省立即返回无效；业务后端不缓存正向内省结果。业务后端不持有平台 JWT 签名密钥。

`/oauth/userinfo` 和现有 `/api/external/*` 对专用令牌使用同样的实时有效性检查。原有 OAuth 令牌的基础流程仍可用，但非专用令牌内省只允许签发它的客户端调用。

## 撤销与升级

- `POST /oauth/revoke` 由令牌所属客户端撤销单个令牌；管理端令牌吊销与应用/客户端禁用、租户/组织应用授权删除也会使相关专用令牌失效。重新启用或授权不会复活旧令牌。
- `0016_application_token_binding.sql` 增加用途、受众、`jti`、应用和平台会话绑定；`0017_application_token_revocation.sql` 增加授权/状态变更的撤销触发器。
- 旧 `/api/oauth/session-token` 生成的用户 access token 没有 `aud/jti`，`0018_revoke_legacy_app_tokens.sql` 在升级时撤销关联应用客户端的旧用户 access token，宿主需要重新签发。`client_credentials` 服务令牌不受这次迁移影响。此迁移不修改历史文件，也不记录明文令牌。
- `0019_application_token_config_revocation.sql` 在应用 code 或客户端的应用绑定、类型、scope、access token TTL 变更时撤销已签发的应用令牌；恢复原配置不会恢复旧令牌。配置变更后宿主需重新签发。

## 验证

在隔离 PostgreSQL 17.6 上，`backend/src/scope_tests.rs` 的 `application_token_audience_and_revocation` 覆盖两租户、两应用正负向签发、双向受众内省、过期与手动撤销、客户端/应用停用、应用 code 与客户端 TTL 变更及恢复、两级授权移除及重新授权、平台会话登出、成员移除、`userinfo`/外部菜单兼容及原有 `client_credentials` 流程。临时库模拟旧用户令牌与服务令牌，`0018` 只撤销前者。
