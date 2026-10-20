# 应用授权与受控成员协议

Phase 30 的接口用于平台与独立工单、门店 Rust API 之间的请求授权。业务数据库、领域模型和范围 SQL 仍由各应用持有；不引入共享数据库模型或公共业务 crate。

`POST /api/apps/{appCode}/authorize` 使用机密客户端 Basic 认证，提交 form 字段 `token`、`resource`、`action`。请求上限 32 KiB，拒绝多余字段。调用客户端的 application_id、令牌应用及 audience 必须一致；资源动作必须登记，且当前身份必须同时拥有操作 permission 与独立 data grant。响应为 `{contractVersion:"1.0",introspection:{...},context:{...}}`，context 保持原 1.0 字段与 contextVersion 算法。内省含 active、scope、client_id、token_type、sub、tenant_id、organization_id、aud、jti、exp；客户端继续独立验证身份、到期时间及完整 context，不依赖 iat。

工单与门店消费同一成员 HTTP 协议：`GET /api/apps/{appCode}/members` 和 `/members/{userId}`。Bearer 必须为当前应用令牌，用途由 resource/action 及 assignmentMembers 能力登记约束。查询携带 organizationId；列表接受 page 1..1,000,000、size 1..50 和最多 100 字符的 q；单项不接受分页、搜索。列表只返回有效直接成员的 id、displayName、organizationId、total、page、size。组织范围先取独立 grant，再与 context 的硬上限求交；越界组织使用既有 404，缺权限或 grant 使用 403。SELF 不允许替其他用户分配。工单与门店继续保留各自的成员 witness 与写入前检查。

每次 context 计算使用一条 PostgreSQL 聚合语句的一致 MVCC 快照。之后以新的独立 SQL 语句重新读取身份、角色、菜单、grant、组织范围与运行元数据，并重新验证 token/session 的撤销状态。内部 SHA-256 revision 仅检测这一次请求内的变化；变化最多重建一次，持续变化返回 503。成员查询在返回结果前再次复核；目标组织及目标应用授权也再次读取。机密客户端 secret/status/application 另在响应前重新检查。所有检查均不使用跨请求正向缓存，公开 contextVersion 不能代替 token 或 Basic 凭据。

平台纯策略库只导出分页、应用绑定与组织上限策略，以及有界快照编排；SQL、HTTP、数据库行与凭据留在私有模块。两个消费者已有相同成员协议，但错误模型、领域授权和部署仍独立，当前没有足够收益引入公共 crate 或 workspace。

平台预算参数：DB_MAX_CONNECTIONS 默认 10（1..128）；DB_ACQUIRE_TIMEOUT_MS 默认 5000（100..30000）；DB_STATEMENT_TIMEOUT_MS 默认 5000（100..60000）；DB_LOCK_TIMEOUT_MS 默认 5000（100..30000）；APP_AUTHORIZATION_TIMEOUT_MS 默认 5000（100..30000）；APP_AUTHORIZATION_MAX_CONCURRENT 默认 32（1..128）。数据库连接初始化设置 statement_timeout 与 lock_timeout；HTTP 超时不能解释为数据库立即取消，SQL 有自己的服务端期限。授权并发槽满时返回 503，不排无限队列。

两业务 API 使用 PLATFORM_AUTHORIZATION_MODE=combined 或 legacy，未知值拒绝启动；combined 没有隐式失败回退。legacy 仍先 Basic 内省，验证后再请求 Bearer context，用于显式回滚。工单默认 combined；门店因当前同负载总耗时及尾延迟收益不足，保留 legacy 默认。两种模式均需通过同一业务与成员协议回归；本轮只验证本地协议与预算，不代表生产切换。

平台通用分页保持既有非法输入默认值，正整数 page 上限 1,000,000，size 1..200，offset 使用 checked arithmetic。i64::MAX 被封顶，超出 i64 的语法输入沿用默认 page=1；成员协议保留自身严格 400 边界。
