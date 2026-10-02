# 对外 API 用户视图迁移（Phase 7）

`GET /api/external/menus` 和 `GET /api/external/roles` 保留原路径，但从完整应用配置视图收敛为当前用户视图，只接受 Phase 5 应用专用令牌。菜单来自当前用户在当前组织的有效角色与租户/组织授权交集；角色响应只含当前组织内对当前应用有贡献的角色，不再枚举其他组织。失效或撤销令牌返回 401，错误应用受众返回 403。旧 `client_credentials` 服务令牌访问这两个用户视图返回 403；该令牌的 `/oauth/introspect` 等 OAuth 基础用途仍可用。

新接入应直接调用 `GET /api/apps/{appCode}/context`，获取版本化菜单、操作权限与独立 `dataGrants`。旧路径仅为现有 Vue/jQuery 演示保留包装形状，不能用于同步应用全量菜单或角色管理配置。管理端需要配置列表时使用平台管理 API，并持有相应的平台操作权限和租户作用域；业务后端不能用服务令牌绕过用户视图。

业务后端参考组件位于 [`examples/app-backend/authorize.mjs`](../examples/app-backend/authorize.mjs)。它逐请求以内省接口验证令牌，再获取当前上下文，分别核对操作权限和资源数据授权，并要求资源查询带认证租户条件。客户端密钥仅在服务端配置中使用，不能进入浏览器包、props 或日志。`SELF` 的归属字段必须由各业务资源声明并在服务端校验；示例订单使用 `owner_user_id`。对应测试位于 [`authorize.test.mjs`](../examples/app-backend/authorize.test.mjs)。

升级顺序：先部署 Phase 5/6 后端和数据库迁移，再更新业务后端按上下文接口校验，然后部署本阶段的旧路径收敛。仍依赖全量 `/api/external/roles` 的报表或演示页面需改为当前组织视图；宿主与示例微应用的完整 props 和页面迁移属于 Phase 8。
