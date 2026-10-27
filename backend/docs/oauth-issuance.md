# OAuth 签发与来源校验

账号登录由 SSO/OIDC 提供。平台 OAuth 保留应用授权码、设备授权、已登记 refresh 与机器 `client_credentials`，不接受 password grant 或公开 `psr_` 平台刷新。

## 客户端管理与分页

`/system/oauth/clients` 分为“业务 OAuth 客户端”和“统一登录 OIDC 客户端”。管理平台、客户门户使用认证中心登记的 `oidc_clients`，分别由 BFF 的客户端标识、secret 和精确回调部署配置引用；不硬编码在业务 OAuth 列表，也不要求在 `oauth_clients` 复制一份控制台客户端。统一登录标签通过平台受权限保护的 `/api/oidc/clients` 读取当前登记，可查看用途、允许身份、回调/退出返回地址、refresh 设置及策略版本，不返回 secret。

业务 OAuth 列表使用 `GET /api/oauth/clients?page=1&size=10`，返回 `{records,total,page,size}`。复用平台有界分页参数：page 至少 1、至多一百万，size 为 1–200、默认 10。按 `created_at,id` 稳定排序，并在同一只读可重复读事务中查询总数、页面和应用名称，避免并发变化导致元数据错配。不带 page/size 时保留数组响应，以兼容原有调用。分页和旧接口使用相同的平台上下文及 `system:oauth` 权限门禁，均不返回凭据。

前端支持每页 10/20/50 条、前后翻页和跳页；修改每页数量回到第一页，删除末页最后记录后恢复到有效页。统一登录登记接口仍按其既有合同返回 `records`，其标签内按本次完整登记结果分页展示。

授权码和设备批准来自当前有效员工上下文。签发与兑换保存具体账号/成员代际、客户端与应用策略版本、原平台会话来源；等待锁和提交前重新核对原始权限。刷新不能扩大 scope，重放及失效来源拒绝，不通过读取当前代际修复旧授权。

人员和机器客户端按明确用途登记；机器用途只允许 CONFIDENTIAL client_credentials，不包含成员身份，也不发放未经登记的 refresh。Public 客户端不能获得机器能力。发现文档不声明 password grant。

签发、消费一次性凭据、写入 token pair 与审计处于同一受控事务。数据库只保存摘要和必要来源证明；访问 token 使用独立 jti，撤销一个 token 不误伤其他有效授权。账号、成员、组织、客户端和应用变更按相应作用域实时拒绝旧凭据。

应用 API 的 userinfo、introspect、业务 context 和通知 proof 都须检查当前来源，JWT 签名并非独立授权。来源不可用返回失败，客户端不得自动改用另一认证协议。
