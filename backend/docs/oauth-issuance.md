# 统一 OAuth / OIDC 签发与来源校验

`saas-identity` 是唯一客户端登记和 OAuth/OIDC 签发权威。管理平台与客户门户分别作为独立 BFF，人员与机器使用同一注册簿中的不同客户端。平台不挂载公开 OAuth 签发、设备授权或旧客户端登记接口；认证中心不可用时失败关闭。

## 唯一公开协议

Discovery 位于准确 HTTPS issuer 的 `/.well-known/openid-configuration`。身份服务提供 `/oauth/authorize`、`/oauth/token`、`/oauth/userinfo`、`/oauth/introspect`、`/oauth/revoke`、`/oauth/jwks` 与受控 `/oauth/logout`。客户端以 Discovery 为准，并验证 issuer、CA、精确回调、state、nonce 和 S256。当前不提供 password、implicit 或设备授权。

USER 客户端使用 code + S256，按登记选择 `ACCOUNT_ONLY` 或 `PLATFORM_CONTEXT`；后者还限定员工／客户及业务应用资格。MACHINE 客户端单独登记为 CONFIDENTIAL、SERVICE、NONE，只允许受批 resource/scope 的 `client_credentials`，没有人员成员或 ID Token。ID Token 不用于业务 API。

公开 `sub` 是受审 sector 下稳定的 opaque 标识，客户端不解析为内部 UUID。默认 Access Token 为 opaque 并绑定精确 resource、audience、scope、原 grant、同意及来源代际。在线 refresh 不超过原 SSO/业务来源绝对期限；离线授权只适用于另行批准的 ACCOUNT_ONLY，不能靠请求 `offline_access` 自行取得。refresh 轮换和重用拒绝不修复旧来源。

## 客户端管理

`/system/oauth/clients` 使用唯一注册簿，可按应用、环境、人员／机器和状态筛选。平台 `GET /api/identity/clients` 代理身份私有 `/internal/identity/auth/clients`，分页使用 `limit=1..100`、`offset=0..100000`，默认 50/0，返回 `{records,page:{limit,offset,total}}`，不再提供旧数组合同或双标签客户端列表。

客户端命令使用独立 `AUTH_ADMIN` 服务身份，配置 `IDENTITY_AUTH_ADMIN_SERVICE_CLIENT_ID` / `IDENTITY_AUTH_ADMIN_SERVICE_CLIENT_SECRET`。应用投影另用 `APPLICATION_PROJECTION`，为客户端登记提供应用资料。所有写入复核当前员工、细分权限、来源、近期 TOTP、期望版本和原 operation。提交结果未知时查询原 operation，不换编号重复创建或轮换。secret 摘要持久化，明文只在获准创建／轮换返回中短暂展示。

新建 USER 客户端未指定历史分组时，认证中心在创建事务中自动建立独立的同意组，不复用其他客户端的用户授权；MACHINE 客户端不创建用户同意组。编辑保留现有客户端的原有绑定。

### 授权和令牌设置

客户端的 `grantTypes` 只允许授权码、刷新令牌及客户端凭据；USER 使用授权码 + PKCE，可选择刷新令牌，MACHINE 仅使用客户端凭据。隐式、密码模式不提供选项。客户端用途和授权类型创建后固定。

`autoAuthorize` 默认 false。仅自有 USER 客户端可启用自动授权，认证中心仍校验当前账号、客户端、应用、scope 和来源。明确的 `prompt=consent`、首次离线访问、已撤销授权及共享同意组不会被自动批准；员工身份选择仍照常执行。自动批准持久化同意记录，并记录 `AUTH_CONSENT_AUTO_APPROVE` 审计。

| 客户端字段 | 含义 | 自定义范围 |
| --- | --- | --- |
| `accessTokenTtlSeconds` | 用户 Access Token 有效期 | 60～3600 秒 |
| `refreshTokenTtlSeconds` | 刷新令牌空闲有效期 | 300～604800 秒 |
| `clientTokenTtlSeconds` | 机器客户端 Access Token 有效期 | 60～3600 秒 |

三个值为 null 时继承认证中心全局配置，接口同时返回 `tokenDefaults`。用户客户端不能设置机器令牌期限，机器客户端不能设置用户令牌、刷新期限或自动授权；没有 refresh grant 时不能设置刷新期限。有效期不得使用负数、0 或无限值，实际签发还取原授权、SSO 来源、refresh family 绝对期限及凭据有效期等适用边界的较早值。刷新令牌策略固定为 `ROTATE`，不允许延用旧令牌。

认证中心的全局配置为 `OIDC_ACCESS_TOKEN_TTL_SECONDS`（默认 300）、`OIDC_REFRESH_TOKEN_IDLE_TTL_SECONDS`（1800）、`OIDC_OFFLINE_REFRESH_TOKEN_IDLE_TTL_SECONDS`（86400）和 `OIDC_CLIENT_TOKEN_TTL_SECONDS`（300）；单位均为秒，取值范围与上表对应。客户端策略变更会推进原 policyVersion 并失效原来源授权，不能用于延长旧 grant。

平台只保留客户端列表、登记、编辑、启停、退役以及凭据轮换／撤销。合作方接入、账号邀请、同意组管理、签名密钥管理已移除；对应的 `/api/identity/review/*`、`/invitations*`、`/invitation-operations/*`、`/consent-groups*`、应用同意组接口、`/keys/*` 和 `/key-operations/*` 不再挂载。OAuth 签名、JWKS、用户同意和来源验证仍由认证中心内部执行，不构成平台管理入口。

## 平台内部能力和资源服务

宿主通过 `POST /api/apps/session-token` 获取绑定原 BFF 来源和具体应用 audience 的短期 `app_session`。它有独立 token_use、摘要账本和验证器，不能用作公开 OAuth grant，也不提供外部客户端注册或 refresh。具体接口见[应用授权合同](application-authorization-contract.md)。

业务 API 使用平台独立 `APP_RESOURCE` 服务凭据调用私有资源授权，身份的 canonical opaque Token 与内部 `app_session` 分别验证。身份、成员、组织、客户端、应用或服务范围收缩后拒绝原凭据；统一权威失联不会切回平台签发。第三方标准接入示例为[独立 OIDC RP 与机器 CLI](../../../saas-examples/oidc-rp/README.md)。
