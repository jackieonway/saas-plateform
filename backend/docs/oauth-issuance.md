# OAuth 与凭据生命周期

平台提供 /oauth/authorize、/token、/userinfo、/introspect、/revoke、/device/authorization 和发现文档；对应路由在 src/routes/oauth2.rs，核心签发逻辑在 src/oauth_issuance.rs。

## 凭据用途

平台管理会话、应用 Bearer 和普通 OAuth token 有不同受众与权限。业务 API 必须验证应用绑定、用户/租户/组织和当前有效状态，不能把任意可解码 JWT 当作业务授权。

授权码、refresh 和设备码使用受控单次消费。发行、消费与关键审计在事务中处理；撤销、用户状态、成员关系与会话状态按当前请求重新检查。机密客户端 secret 只在后端或私有环境中保存。

## 客户端接入

1. 在管理界面创建匹配用途的客户端，配置真实 grant、scope、应用绑定和准确回调 URI。
2. 浏览器通过授权入口，后端交换 code，PKCE 参数与发起会话绑定。
3. 根据 token 类型调用资源端点；机器 client_credentials 不自动拥有用户身份。
4. 轮换、撤销和退出按正式端点处理，不只删除浏览器本地值。

普通 OAuth、平台会话刷新和应用令牌不能互换恢复协议。历史 password 仅对实际允许的客户端使用，不作为新业务登录方案；OAuth 演练示例默认禁用该流程。

## 提交未知与回退

连接断开或提交回执丢失不能证明数据库回滚。普通 OAuth 没有重新取回已签发 token 原文的对账接口；结果未知的 refresh 不应自动重复消费，应停止使用未知轮换结果并重新授权。

已执行的凭据迁移不回写旧 SQL，也不删除迁移历史。旧 issuer 与新存储/单次消费约束可能不兼容，回退需要兼容当前历史的构建或隔离恢复；不能通过重新写回 JWT 原文绕过约束。

开发与回调来源见 [开发指南](development.md)，数据库恢复后的旧凭据失效见 [恢复指南](../ops/recovery/README.md)。
