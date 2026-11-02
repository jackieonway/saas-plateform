# 统一 OAuth / OIDC 签发与来源校验

`saas-identity` 是唯一客户端登记和 OAuth/OIDC 签发权威。管理平台与客户门户分别作为独立 BFF，人员与机器使用同一注册簿中的不同客户端。平台不挂载公开 OAuth 签发、设备授权或旧客户端登记接口；认证中心不可用时失败关闭。

## 唯一公开协议

Discovery 位于准确 HTTPS issuer 的 `/.well-known/openid-configuration`。身份服务提供 `/oauth/authorize`、`/oauth/token`、`/oauth/userinfo`、`/oauth/introspect`、`/oauth/revoke`、`/oauth/jwks` 与受控 `/oauth/logout`。客户端以 Discovery 为准，并验证 issuer、CA、精确回调、state、nonce 和 S256。当前不提供 password、implicit 或设备授权。

USER 客户端使用 code + S256，按登记选择 `ACCOUNT_ONLY` 或 `PLATFORM_CONTEXT`；后者还限定员工／客户及业务应用资格。MACHINE 客户端单独登记为 CONFIDENTIAL、SERVICE、NONE，只允许受批 resource/scope 的 `client_credentials`，没有人员成员或 ID Token。ID Token 不用于业务 API。

公开 `sub` 是受审 sector 下稳定的 opaque 标识，客户端不解析为内部 UUID。默认 Access Token 为 opaque 并绑定精确 resource、audience、scope、原 grant、同意及来源代际。在线 refresh 不超过原 SSO/业务来源绝对期限；离线授权只适用于另行批准的 ACCOUNT_ONLY，不能靠请求 `offline_access` 自行取得。refresh 轮换和重用拒绝不修复旧来源。

## 统一客户端管理

`/system/oauth/clients` 使用唯一注册簿，可按应用、合作方、环境、人员／机器和状态筛选。平台 `GET /api/identity/clients` 代理身份私有 `/internal/identity/auth/clients`，分页使用 `limit=1..100`、`offset=0..100000`，默认 50/0，返回 `{records,page:{limit,offset,total}}`，不再提供旧数组合同或双标签客户端列表。

客户端命令使用独立 `AUTH_ADMIN` 服务身份，配置 `IDENTITY_AUTH_ADMIN_SERVICE_CLIENT_ID` / `IDENTITY_AUTH_ADMIN_SERVICE_CLIENT_SECRET`。应用投影另用 `APPLICATION_PROJECTION`；审核、邀请及密钥使用各自用途。所有写入复核当前员工、细分权限、来源、近期 TOTP、期望版本和原 operation。提交结果未知时查询原 operation，不换编号重复创建或轮换。secret 摘要持久化，明文只在获准创建／轮换返回中短暂展示。

## 密钥管理

平台 `/api/identity/keys/publish`、`/activate`、`/{kid}` 和 `/{kid}/revoke` 分别调用私有 `/internal/identity/auth/keys/*`；原操作查询使用 `/api/identity/key-operations/{id}`。配置独立 `IDENTITY_KEY_ADMIN_SERVICE_CLIENT_ID` / `IDENTITY_KEY_ADMIN_SERVICE_CLIENT_SECRET`，身份服务登记用途必须为 `KEY_ADMIN`。该身份不复用客户端管理、应用投影、账号命令或 RP secret。管理端还要求 `system:oauth:keys` 和近期真实 TOTP；公钥发布与私钥部署分别进行。完整步骤见[签名密钥轮换](../../../saas-identity/docs/signing-key-rotation.md)。

## 平台内部能力和资源服务

宿主通过 `POST /api/apps/session-token` 获取绑定原 BFF 来源和具体应用 audience 的短期 `app_session`。它有独立 token_use、摘要账本和验证器，不能用作公开 OAuth grant，也不提供外部客户端注册或 refresh。具体接口见[应用授权合同](application-authorization-contract.md)。

业务 API 使用平台独立 `APP_RESOURCE` 服务凭据调用私有资源授权，身份的 canonical opaque Token 与内部 `app_session` 分别验证。身份、成员、组织、客户端、应用或服务范围收缩后拒绝原凭据；统一权威失联不会切回平台签发。第三方标准接入示例为[独立 OIDC RP 与机器 CLI](../../../saas-examples/oidc-rp/README.md)。
