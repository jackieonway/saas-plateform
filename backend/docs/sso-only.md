# SSO 唯一登录

访问平台后通过 `/auth/login` 发起 code + PKCE 流程，认证中心完成账号登录与租户／员工身份选择，再回调 `/auth/callback`。平台在同一事务中消费一次性选择结果并建立员工上下文及 HttpOnly、Secure、SameSite Cookie；业务 Bearer 只留在内存。

`GET /auth/config` 固定返回 `{"mode":"BFF"}`。平台不存在本地密码表单、账号选择 ticket、公开平台 refresh 或 LEGACY 回退。旧 `/api/auth/login`、`/api/auth/context`、`/api/auth/logout` 返回 404；OAuth password grant 被拒绝。SSO 不可用时失败关闭。

## 必需配置

准确的 HTTPS `PUBLIC_APP_URL`、`BFF_OIDC_ISSUER`、已登记 `BFF_OIDC_CLIENT_ID`、独立 `BFF_OIDC_CLIENT_SECRET`、精确的 `BFF_OIDC_REDIRECT_URI=${PUBLIC_APP_URL}/auth/callback`、外部 `BFF_TOKEN_KEY_FILE` 和必要的 `BFF_CA_CERT_FILE`。平台和认证中心使用不同主机名。

密钥文件为 `{"activeKid":"a","keys":[{"kid":"a","key":"<32 随机字节的无填充 base64url>"}]}`。密钥、RP secret、内部服务凭据和 JWT signing key 不能复用或提交源码。

独立身份部署另需 `IDENTITY_REMOTE_ENABLED=true`、私网 `IDENTITY_SERVICE_URL`、`IDENTITY_SERVICE_CA_FILE`，以及分别用于 ACCOUNT_COMMAND 和统一客户端管理 AUTH_ADMIN 的服务登记。`CONTEXT_SOURCE_SERVICE_CLIENT_ID` / `CONTEXT_SOURCE_SERVICE_CLIENT_SECRET` 必须登记为 `CONTEXT_SELECTION_SOURCE`，且只允许本平台统一客户端；该凭据用于私有账号证明，与 RP、账号命令和管理凭据分开。平台进程不挂载公开签发 provider，认证中心提供唯一实际认证。

`BFF_CONTEXT_RENEWAL_TTL_SECONDS` 只设置平台内部 BFF／业务上下文的续期上限，默认 604800 秒，范围 1..7776000；实际截止仍取原来源的绝对期限与该上限的较早值。它不配置身份服务 OAuth Access Token／refresh 的生命周期，也不开放浏览器 refresh 接口。公开 OAuth 凭据期限由身份服务及具体客户端授权政策管理。

公开 `OIDC_ISSUER` / `BFF_OIDC_ISSUER` 固定为同一 HTTPS issuer；Compose 的 `IDENTITY_ISSUER` 映射到这两个变量。私网地址独立用于 `/internal/identity/auth/account-proof` 与 `/internal/identity/auth/context-interactions`，通过身份私网 CA 校验证书；公开网关继续拒绝 `/internal/*`。发现文档和浏览器授权使用 canonical `/oauth/*`，在线平台登录只申请 `openid profile`，不申请 `offline_access`。

公开 opaque `sub` 仅用于核对 ID Token、UserInfo 和受认证账号证明的一致性。平台内部账号 UUID、SSO session、grant 和代际只取自私有 `platform-account-proof/1`，不会从 `sub` 解析。上游 access/refresh 凭据仅在服务端加密存储；来源撤销、账号/客户端代际变化或未知 refresh 结果使本地会话失败关闭。

`BFF_ENABLED`、`BFF_LEGACY_LOGIN_ENABLED`、`BFF_LEGACY_HUMAN_CLIENTS` 均为已删除配置，不能通过 true/false 切换登录方式。新数据库仅执行 [0001 全新基线](fresh-installation.md)，没有旧客户端或旧会话的升级清理流程。

## 注销与签名通知

浏览器仍以同源 Cookie、CSRF 和原版本调用 `POST /auth/logout`。`scope=PLATFORM`
对应 canonical `CURRENT_APP`，只撤销原 RP grant 及其本地派生会话；`scope=GLOBAL`
对应当前账号的全部 SSO/人员 grant，二者均保留同意记录。服务端使用自己的 RP
Basic 凭据向固定 issuer 的 `POST /oauth/logout` 提交原凭据和原 operation。
未知结果先查同一 `/oauth/logout/operations/{operationId}`，只有明确 `NOT_FOUND`
才提交相同命令，不换 token、账号或 operation。已经确认的本地回执不重复远端操作。

ID Token 的公开 opaque `sid` 和 `sub` 仅持久化在服务端，和私有账号/SSO UUID
分开。refresh 不能更换原 sid/sub。登记的 back-channel 地址为
`${PUBLIC_APP_URL}/auth/backchannel-logout`；网关须允许该准确公开 POST。
接收器独立验证固定 issuer/JWKS 的 RS256、`typ=logout+jwt`、准确 RP audience、
原 sid/sub、事件及短期限，不要求浏览器 Cookie 或 CSRF，也不使用已撤销 access
token 再认证通知。重复 jti 返回 `409 BACKCHANNEL_REPLAY`。

接收器记录持久来源撤销摘要，和 callback/refresh 使用相同的本地事务锁：即使通知
先于本地会话写入，该旧 sid 也不能后来建立或恢复会话。迟到通知和注销回包只影响
原来源；新的浏览器来源保持有效，旧请求不能清除新 Cookie 或将其报告为已退出。
本地检查点不代表跨服务网络与独立数据库构成原子事务。目的核验、可靠投递和
不支持通知时的短会话合同见[登出与投递合同](designs/unified-auth/phase-20-logout-delivery-contract.md)。

## 验证

检查重定向只能到固定 issuer；state、nonce、PKCE、Cookie、CSRF、回调来源与版本均有效；选择成员、刷新、切换、跨标签页恢复及注销保留身份隔离。故障时不显示备用登录入口。机器客户端和 SSO 派生的微应用授权按自己的合同校验，不借用浏览器 Cookie。
