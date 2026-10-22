# 业务应用授权协议

平台是身份与授权的权威来源，业务服务独立保存领域数据。业务 API 不信任浏览器传入的 tenant、organization、owner 或角色。

## 请求模式

| 模式 | 调用链 | 当前业务默认 |
| --- | --- | --- |
| combined | POST /api/apps/{appCode}/authorize | 工单 |
| legacy | Basic 内省 /oauth/introspect，再用应用 Bearer 获取 /api/apps/{appCode}/context | 门店 |

PLATFORM_AUTHORIZATION_MODE 只接受 combined 或 legacy，失败不自动降级。切换模式需在业务服务进程或实际 Compose override 中显式配置，不能仅修改未被容器透传的宿主变量。

combined 使用机密客户端 Basic，表单提交 token、resource、action；正文上限 32 KiB，未知字段拒绝。响应含 contractVersion、introspection 和 context。客户端应用绑定、令牌受众、当前用户/租户/组织、有效期、permission 和数据授权必须一致。

## 成员目录

GET /api/apps/{appCode}/members 与 /members/{userId} 使用普通应用 Bearer。resource、action、organizationId 必填；列表可用 q、page、size，page 为 1–1000000，size 为 1–50，q 最多 100 字符且不能包含空字符（NUL），非法输入返回 400；单成员接口不接受分页和搜索。

平台策略必须显式登记对应 memberLookups，调用者还需具备对应操作权限与数据授权。目录只返回受控组织内有效直接成员，不开放平台用户管理权限，也不临时创造 capability。

## 业务端执行要求

1. 逐请求获得当前授权，不把 contextVersion 当成凭据或跨请求正向缓存。
2. 在每个列表、计数、详情和写入 SQL 中落实租户、组织硬上限与资源动作范围。
3. SELF 仍以业务 owner 为准；分配处理人或负责人不扩大 SELF 范围。
4. 成员 HTTP 不持有业务连接或行锁；短事务中复核 version、组织、旧值及预校验期限。
5. COMMIT 后结果未知时按业务幂等键或读回版本确认，不声称超时即回滚。

平台默认 APP_AUTHORIZATION_TIMEOUT_MS=5000、APP_AUTHORIZATION_MAX_CONCURRENT=32，受限配置在 src/config.rs 校验。饱和返回 503，不无限排队；平台与业务数据库间不具备跨库原子提交。
