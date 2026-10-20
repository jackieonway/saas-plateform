# 后台授权参考（已退役）

`authorize.mjs` 自 Phase 17 起退役，仅保留其既有行为和 `authorize.test.mjs` 作为历史兼容测试输入。它不是可部署服务，也不是继续维护的后台 SDK；新业务后台应使用 Rust，并以工单服务的实际授权实现和平台 HTTP 契约为参考。

## Rust 接入参考

阅读顺序如下：

1. [业务后端接入与授权边界](../../docs/architecture.md#backend-integration)：平台应用 Bearer、机密客户端内省、当前上下文、功能权限和独立数据 grant 的职责。
2. [Rust 工单授权实现](../../business-apps/work-orders/backend/src/authorization.rs)：以服务端当前身份校验受众、有效期、用户／租户／组织一致性、grant 身份绑定及组织硬上限。
3. [Rust 工单服务源码](../../business-apps/work-orders/backend/src/)和[集成测试](../../business-apps/work-orders/backend/tests/)：平台请求的超时／响应容量限制，列表、count、详情和 CSV 共同使用的数据库范围，以及错误和 CORS 边界。
4. [工单构建、配置与数据库基线](../../docs/work-orders-rust-baseline.md)及 [Phase 17 验证记录](../../docs/validation/phase-17.md)：独立运行条件、实际验证结果和限制。测试声明不能替代对应阶段的执行证据。

工单模块是业务服务内部实现，不是供其他服务直接依赖的公共 crate。其他业务服务应独立实现自己的业务规则，通过平台公开 HTTPS API 获取授权；不要共享平台数据库模型、引入平台 JWT 签名密钥或把浏览器提交的身份当作可信来源。

每个受保护请求先以服务端机密客户端 Basic 调用 `/oauth/introspect`，再以同一应用 Bearer 获取 `/api/apps/{appCode}/context`。功能 permission 与 resource/action data grant 必须分别满足；tenant、组织硬上限与 grant 范围必须落实到 SQL。SELF 按业务记录的可信归属人判断，其他角色或处理人身份不隐式扩大范围。无法证明当前授权时停止业务操作。授权调用合并与跨请求正向缓存不属于本阶段。

## 保留的兼容测试

从仓库根目录使用项目指定的 Node 24 执行：

```sh
node --test examples/app-backend/authorize.test.mjs
```

该命令只运行内存／HTTP fixture 契约，不启动 Node 后台，不访问真实平台或 PostgreSQL，也不证明 Rust 工单接口已通过验收。旧 helper 的输入容忍度与完整业务服务不同，不能将它复制为新的授权边界。当前构建和 CI 保留该测试用于发现历史兼容变化；新授权行为及其门禁以 Rust 服务为准。
