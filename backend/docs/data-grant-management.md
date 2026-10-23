# 独立数据授权管理接口

本文说明平台如何管理“某个直接成员在某个应用中，对某项资源动作具有什么数据范围”。实现分为 HTTP 边界 `src/routes/data_grants.rs`、用例 `src/data_grants.rs`、配置策略 `src/data_grants/policy.rs` 和存储适配器 `src/data_grants/store.rs`，发现接口在 `src/data_grants/discovery.rs`。

## 授权模型

数据授权存储在 `membership_data_grants`，绑定直接 `membership_id` 和 `application_id`，资源动作对在该目标中唯一。角色菜单决定操作权限，成员数据授权决定数据范围，二者彼此独立。平台超级管理员身份、祖先角色和业务应用中的某项数据授权都不能替代管理权限或管理边界。

管理读取要求 `system:data-grant:read`；变更要求 `system:data-grant:write` 且操作者位于平台租户根组织。预览同时要求读与写。所有操作还要落入启动配置明确列出的 `managementScopes`，非平台根读取另外限于本租户和当前组织子树。请求执行中会重新读取当前账户、会话、权限、目标成员及应用状态。

## 服务端策略

`DATA_GRANT_POLICY` 是严格 JSON，未配置时默认为空 allowlist。示例 UUID 必须替换为本环境真实 ID：

```json
{
  "applications": [{
    "applicationId": "11111111-1111-4111-8111-111111111111",
    "capabilities": [{
      "resource": "ticket",
      "action": "assign",
      "scopes": ["SELF", "ORG", "ORG_SUBTREE"]
    }],
    "memberLookups": [{"resource": "ticket", "action": "assign"}]
  }],
  "managementScopes": [{
    "tenantId": "22222222-2222-4222-8222-222222222222",
    "organizationId": "33333333-3333-4333-8333-333333333333",
    "includeDescendants": true,
    "read": true,
    "write": true
  }]
}
```

两个顶层数组都必须存在；未知字段、重复应用、重复资源动作、重复 scope、重复租户组织边界和空 UUID 会被拒绝。资源及动作匹配 `[a-z][a-z0-9_-]{0,99}`。scope 只接受 `SELF`、`ORG`、`ORG_SUBTREE`、`TENANT`，没有隐含大小顺序。

`capabilities` 还必须与目标应用当前有效、启用的菜单权限登记相交；只在环境变量写一个资源动作不能绕过应用登记。`memberLookups` 可省略，默认不开放成员分配用途；它只能引用同一应用已登记的 capability，具体接口见[应用授权契约](application-authorization-contract.md)。

配置上限为 4 MiB、64 个应用、每应用 128 个 capability 和 128 个 member lookup、128 个管理边界。策略在进程启动时解析；修改后需要重启相关平台进程。

## 发现可管理目标

`GET /api/data-grants/options` 使用平台 Bearer 会话，接受：

| 参数 | 规则 |
| --- | --- |
| `applicationId` | 可选应用 UUID；未选应用时用于获取可见应用列表 |
| `userId` | 可选用户 UUID |
| `keyword` | 用户名或显示名搜索，修剪空白后最多 100 字、400 字节 |
| `page` | 默认 1，范围 1–1000000 |
| `size` | 默认 20，范围 1–50 |

响应为 `applications`、`records`、`total`、`page`、`size`。每个成员选项带完整 `target`、用户名、显示名、租户名、组织名及 `canWrite`。`target` 包含 `tenantId`、`organizationId`、`userId`、`membershipId`、`applicationId`。未提供 `applicationId` 时不返回成员页。选项只用于发现，不是后续读写的授权凭证。

## 查询、预览和变更

所有接口使用平台 Bearer 会话：

| 方法与路径 | 请求 | 成功结果 |
| --- | --- | --- |
| `GET /api/data-grants` | 五个 target 字段作为 query | `target`、`version`、`grants`、`capabilities` |
| `POST /api/data-grants/preview` | JSON 差量请求 | `target`、`version`、`before`、`after`、`changes` |
| `PATCH /api/data-grants` | 同一 JSON 差量请求 | 保存后的完整快照 |

```json
{
  "target": {
    "tenantId": "22222222-2222-4222-8222-222222222222",
    "organizationId": "33333333-3333-4333-8333-333333333333",
    "userId": "44444444-4444-4444-8444-444444444444",
    "membershipId": "55555555-5555-4555-8555-555555555555",
    "applicationId": "11111111-1111-4111-8111-111111111111"
  },
  "expectedVersion": "<copy-the-64-character-version-from-GET>",
  "upsert": [{"resource": "ticket", "action": "assign", "scope": "ORG"}],
  "remove": []
}
```

示例中的 version 占位符不能直接提交。`expectedVersion` 必须是查询返回的 64 位小写十六进制字符串。新增和修改共用 `upsert`，删除使用只含资源动作的 `remove`；两数组合计 1–128 项，同一资源动作不能重复或同时出现在两数组。

快照中的 grant 含 `id`、`resource`、`action`、配置 `scope`、实际 `effectiveScope`、`organizationIds`。新增项在预览中的 `id=null`，持久化后有真实 ID；保留和更新原项保持 ID，成员关系也不被重建。除目标为平台租户根组织外，配置 `TENANT` 会投影为 `ORG_SUBTREE`；`SELF` 仍受目标组织子树上限约束。

版本覆盖目标、当前 grants、服务端策略和运行时授权边界。预览不落库，协议允许直接 PATCH，但 PATCH 执行同等校验。并发修改、成员变化或授权边界变化返回 409，调用方必须重读再决定；不能盲目重发旧版本。

## 事务、审计和限制

变更先锁定目标及必要身份，形成差量计划，检查容量，再执行增改删。等待数据库锁后会重查实际权限、目标和边界，最终 grants 必须与计划一致。变更与 `DATA_GRANT_CHANGE` 审计在一个事务提交；审计抛错或触发器静默跳过插入都会导致整个差量回滚。完全无变化的差量返回当前快照，不生成变更审计。

当前授权最多 128 项，组织发现和有效范围最多 10000 项，序列化响应最多 1 MiB。400 表示参数、能力或容量不合法；401 表示会话失效；403 表示管理权限或显式边界不足；404 表示目标不存在或不可用；409 表示快照冲突；数据库或授权证明不可用可能返回 503。

## 回归依据

`src/data_grant_tests.rs` 覆盖权限隔离、边界、严格请求、预览、差量 ID 保持、并发版本及审计回滚；`src/data_grant_discovery_tests.rs` 覆盖发现范围与锁等待；`src/data_grants/policy.rs` 内的单元测试覆盖配置 schema 和规范化。集成套件要求隔离 PostgreSQL 环境，具体运行方式见[开发文档](development.md)。管理页面操作步骤见[数据授权使用指南](data-grant-user-guide.md)。
