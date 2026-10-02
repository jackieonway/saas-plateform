# 应用授权上下文契约（Phase 6）

## 边界

角色只决定菜单与操作权限码。数据权限单独授予用户在当前组织的直接成员关系，不从角色或菜单推导。业务 API 必须同时验证操作权限和对应资源操作的数据授权；缺少任一项即拒绝。平台与业务后端都不能信任浏览器传入的租户、组织、用户或资源归属。

独立数据授权使用 `membership_data_grants`。每条记录绑定 `membership_id`、`application_id`、`resource_code`、`action_code` 和一个 `scope_type`。相同成员、应用、资源、操作最多有一条授权。删除成员会级联删除数据授权；角色或菜单变更不会改写数据授权。没有显式记录时默认为拒绝。策略管理 API 和批量配置界面不在 Phase 6 范围内；在接入业务应用前需由受控的管理流程写入授权，并记录操作者和变更原因。

## 查询接口

`GET /api/apps/{appCode}/context` 仅接受 Phase 5 应用专用 Bearer token。路径中的 `appCode` 必须等于 token 的独立受众。平台逐请求校验令牌、平台会话、直接成员、租户/组织/应用状态与两级应用授权；错误受众、撤销或失效令牌返回拒绝，不返回上下文。普通 OAuth `client_credentials` token 不能调用此接口。

响应契约版本 `1.0`：

```json
{
  "contractVersion": "1.0",
  "contextVersion": "sha256-hex-of-effective-context",
  "expiresAt": "2026-10-02T10:00:00Z",
  "appCode": "example",
  "application": { "id": "...", "name": "Example" },
  "user": { "id": "...", "username": "...", "displayName": "..." },
  "tenant": { "id": "...", "code": "...", "name": "..." },
  "organization": { "id": "...", "name": "...", "path": "..." },
  "roles": [],
  "permissions": ["order:read"],
  "dataGrants": [{
    "resource": "order", "action": "read", "scope": "ORG_SUBTREE",
    "subjectUserId": "...", "tenantId": "...", "organizationIds": ["..."]
  }],
  "menus": []
}
```

`permissions` 来自当前用户有效祖先角色菜单与租户、当前组织菜单授权的交集，菜单展示和平台操作权限使用同一计算器。`roles` 只列出对当前应用的有效菜单有贡献的角色。`dataGrants` 独立读取当前组织成员记录，按资源与操作输出。`contextVersion` 是当前有效内容的稳定哈希；授权、菜单、角色或身份数据变化会改变它。`expiresAt` 不晚于应用 token 到期时间和本次计算后 30 秒。调用方不得将过期上下文作为业务授权依据；业务后端应逐请求内省应用 token 并获取当前上下文，不缓存正向授权。平台不使用原有的 30 秒进程内权限缓存，跨实例在下一次请求读取数据库变更。

## 数据范围

| scope | 服务端边界 |
| --- | --- |
| `SELF` | `subjectUserId`；业务资源必须声明并校验可验证的用户归属字段。 |
| `ORG` | 当前组织 ID，且记录租户 ID 必须等于当前租户。 |
| `ORG_SUBTREE` | 当前组织及其有效子孙组织 ID，始终限当前租户；祖先角色不会改变范围起点。 |
| `TENANT` | 当前租户 ID。 |

`organizationIds` 仅在 `ORG` 与 `ORG_SUBTREE` 下填充。应用必须在服务端把资源的 `tenant_id`、`organization_id`、`owner_user_id` 等实际字段映射到上述边界；没有可靠归属字段时拒绝对应操作。映射与业务查询的落地属于 Phase 7/11。不能把 `dataGrants` 当作操作权限，也不能用一个资源的授权访问另一个资源。

## 部署与升级

`0020_membership_data_grants.sql` 新建独立授权表，不给现有成员自动添加数据权限。升级后业务数据访问默认为拒绝，须先配置实际业务资源的数据授权。现有平台菜单和 OAuth 服务令牌不因本迁移改变。平台与应用应通过 HTTPS 访问，客户端密钥仍只在业务后端保存。
