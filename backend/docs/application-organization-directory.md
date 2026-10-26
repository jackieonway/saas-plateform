# 应用受控组织目录

门店身份由平台组织管理维护，`organizationType=STORE` 表示门店组织。平台负责名称、编码、层级、地址、联系方式、组织状态和成员；门店应用只维护营业扩展、日报和统计。全新 `0001_init.sql` 基线直接建立 STORE 类型及当前组织约束。

## 边界与用途登记

组织管理的 `/api/organizations/tree` 需要平台会话和 `system:organization`，包含管理资料，不能用作应用业务目录。本接口使用应用专用 Bearer token，不扩充 SDK 2 的 MicroProps，也不把平台管理会话或客户端 secret 交给微应用。

在 `DATA_GRANT_POLICY.applications[]` 增加独立的 `organizationLookups`，默认空、未登记即拒绝。示例片段应放在真实应用策略中：

```json
{
  "applicationId": "11111111-1111-4111-8111-111111111111",
  "capabilities": [{"resource":"store","action":"read","scopes":["ORG","ORG_SUBTREE"]}],
  "organizationLookups": [{"resource":"store","action":"read","organizationType":"STORE"}]
}
```

每项必须引用同一应用的 capability，资源动作对不能重复，最多 128 项；当前只允许 `organizationType=STORE`。它与 `memberLookups` 相互独立，开放一种目录不会自动开放另一种目录。策略的 scopes 是明确允许值，不具有隐含大小顺序。

门店应用按实际使用登记 `store/read|create|update`、`store-staff/read`、`store-daily-report/read|create|update`；每个接口必须使用正在执行的资源动作查询，不能用 store/read 的目录证明执行日报写入。这里只登记查询用途，不自动给任何用户增加 permission 或数据授权。

## HTTP 契约

```http
GET /api/apps/stores/organizations?resource=store&action=read&page=1&size=20&q=门店
Authorization: Bearer <stores-application-token>

GET /api/apps/stores/organizations/<organization-id>?resource=store&action=update
Authorization: Bearer <stores-application-token>
```

resource/action 必填。列表 page 默认 1、范围 1–1000000；size 默认 20、范围 1–50；q 在修剪后最多 100 字，按名称或编码的字面子串搜索。单项不接受分页或搜索。未知字段、tenantId、organizationType 等调用方范围覆盖参数会被拒绝；目录固定为用途已登记的 STORE。

单项及列表每条 record 形状如下，UUID 和时间仅为示例：

```json
{
  "contractVersion": "1.0",
  "contextVersion": "<current-authorization-context-version>",
  "id": "22222222-2222-4222-8222-222222222222",
  "tenantId": "33333333-3333-4333-8333-333333333333",
  "parentId": "44444444-4444-4444-8444-444444444444",
  "name": "示例门店",
  "code": "STORE-A",
  "organizationType": "STORE",
  "status": "ENABLED",
  "path": "总部 / 示例门店",
  "province": "浙江省",
  "city": "杭州市",
  "district": "西湖区",
  "township": "",
  "village": "",
  "address": "示例地址",
  "contactName": "门店联系人员",
  "contactPhone": "",
  "contactEmail": "",
  "revision": "<64-lowercase-hex-directory-row-revision>",
  "expiresAt": "2026-10-24T00:00:00.000Z"
}
```

列表 envelope 为 `contractVersion`、`contextVersion`、`expiresAt`、`records`、`total`、`page`、`size`。parentId 可为 null。目录包括尚未建立营业扩展的 STORE 组织，平台不读取门店业务数据库。

最多计入 10000 个匹配组织；超限要求收窄查询，不静默截断。单次响应最多 1 MiB。这里不返回角色、成员清单、菜单授权、管理计数或组织版本写入能力。

## 每次请求的授权与新鲜度

请求同时要求当前应用 permission、当前用户直接成员关系上的同资源动作 dataGrant、同租户、组织硬边界和已登记的组织目录用途。目标 STORE 及完整祖先链必须启用且未到期，租户与目标组织都必须授权当前应用。

总部可以在自己的组织上下文通过 ORG_SUBTREE 授权读取下属 STORE，无须加入每家门店或切换到门店会话。SELF 的列表发现目标及 ORG 目标仅为当前组织。SELF 单项可解析已知组织在 grant organizationIds 与硬上限交集中的元数据，以保留本人拥有的有效子组织业务记录；这不证明业务 owner，也不允许根据元数据创建子组织营业档案或选择跨组织联系人。业务服务仍需将 tenant、组织上限、dataGrant 和 SELF owner 写入自己的 SQL 条件，新增档案、联系人及成员选择另执行当前组织目标约束。

平台先组装目录投影，再独立重验应用授权上下文和目录投影。目标改类型、禁用、到期、撤销应用授权或目录元数据变化，不会继续返回原结果。中途变化导致无法证明一致时返回 503；不存在、非 STORE 或越界目标统一 404。

`expiresAt` 不晚于请求开始后 3 秒、上下文期限、应用 token 期限、目标租户及组织祖先期限。业务调用方必须从 HTTP 开始计绝对窗口，包含响应传输及后续数据库/锁等待；不能在收到响应时续期，不能正向缓存到下一业务请求。

业务授权和 lookup 的 `contextVersion` 必须一致；否则原 SQL 可能仍使用变更前的数据范围，应终止并重新取得整套授权。分页收集须保持同一 contextVersion，并取所有页最早到期时间。统计和总数必须先与有效 STORE 集合相交再计算，不能先聚合后隐藏。跨平台/业务库不存在全局事务，短期证明不承诺跨库原子撤销。

组织改类型或删除后，营业扩展和日报历史由业务服务保留并停止通过当前门店接口读写；不跨服务直连数据库或建立跨库外键。业务历史归档访问需要单独定义明确权限。

## 验证

`src/app_organizations_tests.rs` 在显式 loopback `TEST_DATABASE_URL` 上创建和删除随机隔离数据库，验证 HQ 子树、跨租户/兄弟隔离、独立用途、SELF、类型/状态/期限与应用授权撤销。`src/app_organizations.rs` 和策略模块另有纯单元测试。测试不读取 `.env`，不使用业务 preview 库作为测试数据表。

```powershell
cargo test --locked app_organizations
cargo test --locked organization_lookups
cargo test --locked app_organizations_tests -- --ignored
```

完整功能授权模型见[应用授权契约](application-authorization-contract.md)，策略管理见[独立数据授权](data-grant-management.md)。
