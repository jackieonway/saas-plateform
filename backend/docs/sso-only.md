# SSO 唯一登录

访问平台后通过 `/auth/login` 发起 code + PKCE 流程，认证中心完成账号登录并回调 `/auth/callback`。平台建立 HttpOnly、Secure、SameSite Cookie，会话中明确选择当前员工成员；业务 Bearer 只留在内存。

`GET /auth/config` 固定返回 `{"mode":"BFF"}`。平台不存在本地密码表单、账号选择 ticket、公开平台 refresh 或 LEGACY 回退。旧 `/api/auth/login`、`/api/auth/context`、`/api/auth/logout` 返回 404；OAuth password grant 被拒绝。SSO 不可用时失败关闭。

## 必需配置

准确的 HTTPS `PUBLIC_APP_URL`、`BFF_OIDC_ISSUER`、已登记 `BFF_OIDC_CLIENT_ID`、独立 `BFF_OIDC_CLIENT_SECRET`、精确的 `BFF_OIDC_REDIRECT_URI=${PUBLIC_APP_URL}/auth/callback`、外部 `BFF_TOKEN_KEY_FILE` 和必要的 `BFF_CA_CERT_FILE`。平台和认证中心使用不同主机名。

密钥文件为 `{"activeKid":"a","keys":[{"kid":"a","key":"<32 随机字节的无填充 base64url>"}]}`。密钥、RP secret、内部服务凭据和 JWT signing key 不能复用或提交源码。

独立身份部署另需 `IDENTITY_REMOTE_ENABLED=true`、准确身份服务 HTTPS URL、CA，以及分别用于 ACCOUNT_COMMAND 和 OIDC_ADMIN 的服务登记。平台进程关闭内嵌 SSO/OIDC provider，认证中心提供实际认证。

`BFF_ENABLED`、`BFF_LEGACY_LOGIN_ENABLED`、`BFF_LEGACY_HUMAN_CLIENTS` 均为已删除配置，不能通过 true/false 切换登录方式。新数据库仅执行 [0001 全新基线](fresh-installation.md)，没有旧客户端或旧会话的升级清理流程。

## 验证

检查重定向只能到固定 issuer；state、nonce、PKCE、Cookie、CSRF、回调来源与版本均有效；选择成员、刷新、切换、跨标签页恢复及注销保留身份隔离。故障时不显示备用登录入口。机器客户端和 SSO 派生的微应用授权按自己的合同校验，不借用浏览器 Cookie。
