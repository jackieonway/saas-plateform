# 数据授权管理接口

数据授权决定某个业务资源/动作可作用于哪些数据，与角色菜单 permission 分开。能够进入应用或看到按钮，并不自动获得读取/修改所有数据的权限。

## 三层边界

- 平台应用策略：DATA_GRANT_POLICY 的 applications 声明应用 UUID、资源、动作、允许范围及 memberLookups。
- 管理边界：managementScopes 声明管理者可维护的租户、组织、是否含后代及 read/write 能力。
- 业务目标：实际授予某个租户、组织、当前有效直接 membership 与应用，不从组织祖先继承成员的数据授权。

默认空策略拒绝。声明策略不会创建菜单、用户、成员关系或实际授权。修改配置须合并真实已有内容，不能用示例覆盖其他应用。

## 管理接口

| 方法与路径 | 用途 |
| --- | --- |
| GET /api/data-grants/options | 在当前管理范围内发现合法目标与选项 |
| GET /api/data-grants | 读取完整目标的版本和授权快照 |
| POST /api/data-grants/preview | 预览拟提交变更与范围 |
| PATCH /api/data-grants | 带当前版本执行变更 |

管理接口使用平台管理会话。读取/预览与写入分别要求 system:data-grant:read、system:data-grant:write，并同时满足实际 DATA_GRANT_POLICY 管理边界。字段定义见 src/data_grants 的 Target、ChangeRequest 和 OptionsQuery，路由在 src/routes/data_grants.rs。

## 正确变更顺序

1. 通过管理界面或 options 选定租户、组织、用户直接成员关系和应用。
2. 读取当前快照及版本，选取已登记资源/动作和允许的数据范围。
3. 预览并核对期望影响，再提交同一目标与版本。
4. 409 时重读并重新决策，不把旧快照直接覆盖新值；网络结果未知时先查询确认。
5. 使用普通业务用户重新取上下文并执行真实列表、详情与写入验证。

合法变更与审计在受控事务中完成；审计失败不会留下部分授权。成员关系真正删除/重建后其身份发生变化，不能沿用旧 membership 授权快照。

<a id="member-lookups"></a>
## 受控成员查找

memberLookups 必须匹配该应用的 capability 资源/动作。工单使用 ticket/create、ticket/update；门店使用自己的登记对。业务 Bearer 只能按当前授权、组织硬上限查有效直接成员，不获得通用目录管理权。接口规则见 [应用授权协议](application-authorization-contract.md)。

<a id="notification-reminders"></a>
## 工单通知提醒

提醒同时要求 ticket/read 与 notification/publish 的功能权限和数据授权，使用正式通知发布 API。业务意图和平台发布不是跨库事务，未知结果必须复用原事件恢复，不能换键补发。

## 运维配置

生产 Compose 将 DATA_GRANT_POLICY 从私有环境文件注入 API。保留 JSON 的正确引号与精确 UUID；进程启动会校验策略。完整部署见 [部署指南](deployment.md)，普通用户配置顺序见 [用户指南](data-grant-user-guide.md)。
