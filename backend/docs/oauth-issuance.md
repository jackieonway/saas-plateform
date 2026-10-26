# OAuth 签发与来源校验

账号登录由 SSO/OIDC 提供。平台 OAuth 保留应用授权码、设备授权、已登记 refresh 与机器 `client_credentials`，不接受 password grant 或公开 `psr_` 平台刷新。

授权码和设备批准来自当前有效员工上下文。签发与兑换保存具体账号/成员代际、客户端与应用策略版本、原平台会话来源；等待锁和提交前重新核对原始权限。刷新不能扩大 scope，重放及失效来源拒绝，不通过读取当前代际修复旧授权。

人员和机器客户端按明确用途登记；机器用途只允许 CONFIDENTIAL client_credentials，不包含成员身份，也不发放未经登记的 refresh。Public 客户端不能获得机器能力。发现文档不声明 password grant。

签发、消费一次性凭据、写入 token pair 与审计处于同一受控事务。数据库只保存摘要和必要来源证明；访问 token 使用独立 jti，撤销一个 token 不误伤其他有效授权。账号、成员、组织、客户端和应用变更按相应作用域实时拒绝旧凭据。

应用 API 的 userinfo、introspect、业务 context 和通知 proof 都须检查当前来源，JWT 签名并非独立授权。来源不可用返回失败，客户端不得自动改用另一认证协议。
