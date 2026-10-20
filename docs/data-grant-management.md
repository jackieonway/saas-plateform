# 受控数据 grant 管理 API

Phase14提供正式API，按直接成员、应用、资源与操作查询和差量维护data grants；Phase15增加 `/system/data-grants` 管理页面和受限目标选择。Phase19在服务端应用策略增加可选 `memberLookups`，用于普通应用Bearer的窄成员查找，不新增成员grant或assign动作，不改变公开capabilities DTO。Phase20工单提醒同时使用既有ticket/read和独立notification/publish授权。功能权限、数据范围和grant管理权限分别校验；普通业务用户使用成员／业务／提醒接口不需要grant管理权。页面操作见[普通工单用户授权指南](data-grant-user-guide.md)，历史结果见 [Phase14](validation/phase-14.md)、[Phase15](validation/phase-15.md)，成员接入与提醒的实际结果由 [Phase19](validation/phase-19.md)、[Phase20](validation/phase-20.md)报告记录，不预记执行通过或生产部署。

## 管理权限与目标边界

每个目标必须同时指定 `tenantId`、`organizationId`、`userId`、`membershipId`、`applicationId` 五个非空 UUID。服务端核对完整关系：用户有效，membership 是该用户在该组织的有效直接成员，组织归属指定租户，租户/组织及祖先有效，应用启用且获得目标租户和组织的应用授权。角色继承或另一组织的成员关系不能代替该目标。

管理页面通过 `GET /api/data-grants/options` 先列出可管理应用，再搜索该应用下可查询的有效直接成员。选择项由同一 grant 读取边界生成，不要求额外的用户目录或应用管理权限，也不因此授予这些权限；范围外目标不会作为选项。每次快照、预览和保存仍独立复验五个 ID，不能把曾经出现的下拉项当作永久授权。

既有 `GET /api/users/{id}/memberships` 每个成员项返回 `membershipId`，保留原 tenant/org 等字段。该接口仍要求原有 `system:user` 权限与组织范围；它不是管理页面选择目标的前置接口，grant 管理权不隐式附送用户目录读权。

| 操作 | 操作者必须满足的条件 |
| --- | --- |
| GET 查询 | `system:data-grant:read`，目标在实际会话组织子树与策略 `read` 正向范围的交集内；实际平台根按其真实管理上下文判断 |
| POST 预览 | 同时满足查询与写入两条链：独立读/写权限、实际平台根上下文、目标同时在策略 read/write 范围内 |
| PATCH 差量保存 | `system:data-grant:write`、实际平台根上下文，目标在策略 `write` 正向范围内；不隐式要求或授予 read 权限 |

平台根可以管理策略明确允许的业务租户，不能把目标租户硬性等同于操作者所在的平台租户。普通组织管理员即使能管理用户、角色或菜单，也没有隐式 grant 写权；自行构造 PATCH 仍被拒绝。`tenant.isPlatform`、全局超管标志或前端传入的 `platformContext` 不能代替实际平台根会话检查。

可写能力来自服务端策略，独立于操作者自己的业务 grants。不能把操作者持有的 SELF/ORG/TENANT 解释为可转授上限，也不对 scope 枚举排序推导权限；每个资源/操作允许哪些 scope 都必须明确登记。未登记时拒绝，没有默认 TENANT，也没有首版委派或审批规则。

## 服务端策略与采用

`DATA_GRANT_POLICY` 是启动时读取的严格 JSON，不是请求参数或浏览器配置。未设置时等价于：

```json
{"applications":[],"managementScopes":[]}
```

两个数组及既有必需嵌套字段必须完整；每个application另可省略 `memberLookups`，其默认值为空数组，不默认开启成员查找。未知字段、无效类型、空白配置、nil UUID、重复应用／能力／范围根或未知scope均拒绝启动。应用最多64个，每应用能力最多128个，管理范围根最多128个；resource/action各为 `[a-z][a-z0-9_-]{0,99}`，scope为精确大写SELF、ORG、ORG_SUBTREE、TENANT，每个capability明确列出1–4个不重复scope。

以下 UUID 全是说明用的虚构值；采用时必须替换为现有对象并核对精确关系。例子只允许一个业务组织及其有效子树中，ticket/read 的 SELF 或 ORG 范围：

```json
{
  "applications": [
    {
      "applicationId": "50000000-0000-4000-8000-000000000001",
      "capabilities": [
        {"resource": "ticket", "action": "read", "scopes": ["SELF", "ORG"]}
      ]
    }
  ],
  "managementScopes": [
    {
      "tenantId": "10000000-0000-4000-8000-000000000001",
      "organizationId": "20000000-0000-4000-8000-000000000001",
      "includeDescendants": true,
      "read": true,
      "write": true
    }
  ]
}
```

`managementScopes` 是正向允许集合。不同根覆盖同一目标时取正向并集，某一项 `read:false`/`write:false` 不会否定另一项的正向允许；同一 tenant/org 根不能重复登记。根必须在运行时仍属于所列租户且有效，`includeDescendants:false` 只允许根本身。配置中的 UUID 或范围不会创建租户、组织、成员或应用。

每个提交变更的 resource/action 还必须同时存在于该应用 UUID 的 capability 清单和该应用中 `status=ENABLED`、`permission` 精确为 `resource:action` 的菜单登记。其它应用的同名权限不算，`visible` 或菜单类型不是这一登记条件。所选 scope 必须是该 capability 明列的可选值，菜单登记和服务端策略缺一不可。

真实配置模板位于 [Compose 模板](../deploy/compose.env.example)、[legacy Compose 模板](../deploy/compose.legacy.env.example) 和[隔离测试模板](../.env.test.example)，默认均为空拒绝。单行环境值保留外层单引号，例如 `DATA_GRANT_POLICY='{"applications":[],"managementScopes":[]}'`，便于 Compose env-file 和 POSIX shell 保留 JSON。[根 Compose](../docker-compose.yml) 将该变量显式传给 backend，未设置时使用空策略；显式设置空字符串则保留该值并由启动校验拒绝。原生服务从操作者明确注入的运行环境读取；不得假设仓库有 `backend/.env.example` 或把前端配置当成服务端策略。

策略只在启动时装载，没有热重载或策略写 API。版本摘要包含策略，但版本冲突不能替代部署同步：调整允许范围时应停止或排空旧策略实例、同步全部副本并重新启动，避免旧副本继续按旧配置授权。空策略按拒绝处理；跨副本同步是部署责任，不能把 hash 描述为全局配置锁。

<a id="member-lookups"></a>
## 应用成员查找声明与工单采用

`applications[].memberLookups` 是独立于公开capabilities DTO的服务端配置，形状为 `[{"resource":"ticket","action":"create"},{"resource":"ticket","action":"update"}]`。省略或空数组不开放成员查找；每应用最多128项，每个pair必须已在同一application的capabilities中登记，声明不能重复，也不能凭空引入resource/action。公开能力项仍只有 `resource/action/scopes` 三字段，grant编辑接口不新增member grant、assign操作或客户端开关。

以下虚构UUID示例登记工单create／update及其成员查找。采用时并入现有应用与管理范围，保留其他capabilities、memberLookups和managementScopes；不要用示例覆盖真实完整配置：

```json
{
  "applications": [
    {
      "applicationId": "50000000-0000-4000-8000-000000000001",
      "capabilities": [
        {"resource":"ticket","action":"create","scopes":["SELF","ORG"]},
        {"resource":"ticket","action":"update","scopes":["SELF","ORG"]}
      ],
      "memberLookups": [
        {"resource":"ticket","action":"create"},
        {"resource":"ticket","action":"update"}
      ]
    }
  ],
  "managementScopes": [
    {
      "tenantId":"10000000-0000-4000-8000-000000000001",
      "organizationId":"20000000-0000-4000-8000-000000000001",
      "includeDescendants":true,
      "read":true,
      "write":true
    }
  ]
}
```

声明不会创建应用、菜单、成员或grant。仍须登记对应应用的 `ticket:create`／`ticket:update` 功能菜单并明确授权，以及当前直接membership上的对应data grant。`managementScopes`继续约束grant管理者的维护边界；普通工单操作者查成员不因此需要 `system:data-grant:read/write` 或通用用户目录管理权限。平台生成的真实application UUID必须与工单机密客户端绑定的 `work-orders` 应用一致，不能用客户端名称代替。

业务服务以当前普通应用Bearer调用 `GET /api/apps/{appCode}/members` 和 `/members/{userId}`，查询包含已登记的resource/action及目标organizationId；列表另可带q/page/size。平台核对当前应用、功能permission、该action的data grant、组织硬上限、目标组织及有效直接成员；不从浏览器接受可信身份，也不让业务服务直查平台SQL。工单仅使用ticket/create或ticket/update：SELF／ORG只能查当前组织，ORG_SUBTREE／TENANT仍与硬上限取交集，处理人不扩大SELF的owner范围。

工单更新先按业务scope非锁定预读，再释放业务连接做平台成员校验，之后进入本地事务并锁定复核组织／version／旧assignee；不一致409 `VERSION_CONFLICT`。真正改换或清除分配须满足目录的目标组织scope；新非空分配必须是该组织当前有效直接成员。同UUID或省略更新保留既有名称快照，不重查旧成员。已提交的同key／同hash创建结果仍需当前create授权，但重放不重新验证历史处理人，原201仅是创建历史。

成员校验从该HTTP请求开始以单调时钟计时，有效窗口为 `min(PLATFORM_AUTH_TIMEOUT_MS,3000ms)`，响应读取与后续DB等待也消耗窗口，不在响应或取得锁后续期。成员HTTP本身超时503 `PLATFORM_UNAVAILABLE`；已取得的成员结果在提交前过期503 `MEMBER_VALIDATION_EXPIRED`并回滚，同时继续复核context期限。COMMIT后才发现context／成员期限过期时返回503 `CREATE_RESULT_UNKNOWN`或 `WRITE_RESULT_UNKNOWN`，按原创建键／正文或读回版本对账，不能认定回滚。该窗口不会把平台库与业务库变成原子事务：最后一次平台验证到业务COMMIT之间仍可能撤销成员或权限，后续新请求重新校验；不承诺跨库强一致或自动撤销已保存分配。

Phase19自身不增加schema或依赖，0001／0002及已有数据保持。其成员能力回退至18会恢复所有显式非null assignee的400，也不能重放带非空处理人的创建正文；已有分配和receipt必须保留。Phase20升级0003后还须使用接受新历史与不可变intent的兼容构建，不能直接使用原Phase18／19二进制，先核对在途结果。若同时回退平台至不认识 `memberLookups` 的旧严格解析器，须先审阅配置兼容并移除该新字段、同步副本；旧成员路由边界不等价于新空声明。完整回退见Rust工单说明，这里不执行实际部署或回退。

<a id="notification-reminders"></a>
## 工单提醒的独立发布授权

Rust工单 `POST /work-orders-api/tickets/{id}/remind` 从同一次当前应用授权上下文检查两条独立链：`ticket:read` 功能permission与 `ticket/read` data grant决定是否可读取当前未删除工单，`notification:publish` 与 `notification/publish` 决定是否可向目标用户及组织发布。历史intent或publication都不能替代当前授权。提醒不增加OAuth scope或机器身份，也不使用 `ticket:publish`、成员目录grant或管理者session发布。

要通过正式grant管理页面/API维护提醒所需范围，应在实际work-orders应用的既有capabilities中合并所需pair。例如以下仅是待合并的capability片段，scopes须按实际审批范围列出：

```json
[
  {"resource":"ticket","action":"read","scopes":["SELF","ORG"]},
  {"resource":"notification","action":"publish","scopes":["SELF","ORG"]}
]
```

保留同应用其他capabilities、memberLookups及managementScopes，不能用这个片段覆盖完整策略。登记能力不自动创建菜单、grant或业务授权；还须给当前普通成员显式授予该应用的 `ticket:read` 与 `notification:publish` 功能菜单，并在其当前组织的直接membership分别配置两个data grant。发布者不因此需要 `system:data-grant:read/write` 或 `system:audit`；这些是管理与审计权限。

两类grant各自与tenant和上下文组织硬上限取交集。read的SELF按工单owner限制，处理人不扩权；publish的SELF只允许向当前用户、当前组织发布，ORG限当前组织，ORG_SUBTREE／TENANT仍遵守有效组织范围及普通业务组织TENANT收窄规则。提醒目标固定为工单组织内的处理人，无处理人时为owner；有read权不保证能向该目标发布。新事件的收件资格由正式平台通知API检查，不能用曾出现在成员目录或前端候选中的UUID代替。

`memberLookups`只控制已声明ticket/create、ticket/update的成员辅助入口，不授予notification/publish。提醒不新增成员查找声明或成员预查；平台已经持久发布的同正文重试，在当前发布者身份、发布授权和目标范围成立时恢复原回执，不重新枚举当前收件人，也不补建已删除的收件箱delivery。工单仍先检查当前read／publish与软删除状态，不能凭回执恢复已失去访问的工单。

提醒键、持久intent与准备／发布未知结果的完整边界见 [Rust工单说明](work-orders-rust-baseline.md#reminder-recovery)。管理配置不执行真实授权或生产切流。

## 安装独立管理权限

Phase 14 的两个隐藏 BUTTON `system:data-grant:read` 和 `system:data-grant:write` 继续保留在系统“授权管理”菜单下，不搬移已有按钮或改变其 UUID。Phase 15 在系统应用下新增独立、可见的根级 MENU，路径 `/system/data-grants`，permission 为 `system:data-grant:read`。页面同时要求独立读取权限与当前获授权菜单中的精确 MENU 路由证据；只有隐藏读按钮或父路径不会自动得到页面入口。

新空库 seed 仅按初始化规则将这些菜单明确授予平台根的管理员角色、平台租户和平台根组织，不自动授予所有角色或业务用户，也不自动创建业务 data grants。

已有安装不会通过迁移自动扩权。实际平台根管理员使用既有菜单管理 API 先查询系统应用、原“授权管理”父菜单及读写按钮，保留已有 UUID 和授权集合。仅在对象缺失时创建所需按钮；Phase 15 的新页面 MENU 需显式注册并授权。例如页面入口的 `POST /api/menus` 正文为：

```json
{
  "applicationId": "实际系统应用UUID",
  "parentId": null,
  "type": "MENU",
  "title": "数据授权",
  "path": "/system/data-grants",
  "permission": "system:data-grant:read",
  "visible": true
}
```

缺少 Phase 14 读按钮时，在原“授权管理”父菜单下创建：

```json
{
  "applicationId": "实际系统应用UUID",
  "parentId": "实际授权管理菜单UUID",
  "type": "BUTTON",
  "title": "查询数据授权",
  "permission": "system:data-grant:read",
  "visible": false
}
```

写按钮独立使用 `system:data-grant:write`，保持隐藏。以上占位字符串必须换成真实 UUID，不能原样发送。随后通过既有租户授权 `PUT /api/tenants/{id}/authorizations`、组织授权 `PUT /api/organizations/{id}/authorizations`、角色菜单 `PUT /api/roles/{id}/menus` 及用户角色 API，分别授权新 MENU 和需要的读写按钮给指定管理角色和成员；保留原有集合，不用只含新菜单的请求覆盖其它授权。业务应用的功能 permission 仍单独配置，grant 管理入口和两个按钮不代表 ticket/read 等业务权限。

## 管理页面与受限目标选择

页面位于 `/system/data-grants`。具备独立读取权限时，“授权管理”和“用户管理”提供相应跳转入口；它们不附送进入这两个原页面所需的权限。用户管理的 `userId` 深链只是筛选条件，必须与服务器返回的有效候选项匹配，不能直接构造目标或预选范围外成员。

先“选择数据授权应用”，再使用“搜索成员”和分页选择精确的直接成员；应用和成员都不自动预选。页面展示已存 `scope`、目标运行时 `effectiveScope` 和 `organizationIds`，能力与可选 scope 来自服务端当前登记。组织管理员或仅有读权限的管理者可以在边界内查看；编辑还同时要求实际平台根、独立 write 权限及服务端确认的目标可写状态。前端隐藏写入口不替代服务端检查。

每个可编辑 resource/action 可选择“不授权”或该 capability 明列的 scope。pair 未登记或已存 scope 已不在当前允许列表的历史行，标为“历史授权 · 当前不可编辑”，保留显示且不进入本次差量。通过页面清理前，须由有权运维受控恢复该 pair 及已存 scope 的必要登记，再预览删除；API 删除的登记规则见后文。空 grant 默认拒绝，不会因选择应用、查看页面或分配角色自动补齐。

改动先点“预览变更”，核对 `before`、`after` 和 `changes` 后再点“确认保存”，页面不自动保存。确认提交使用刚才预览的精确 delta 与版本；编辑、切换目标或权限变化都会使旧预览失效。没有差量时不发送预览或保存请求。保存成功显示本次响应的安全 requestId，目标业务下一受保护新请求按新授权执行；切换会话、组织或目标后，旧请求不能把回执或草稿填入新目标。

409 时页面提示“授权版本已变化，请重新读取并审阅”；网络错误、5xx 或无法确认保存回执时提示“保存结果待确认，请重新读取授权”。两者都须先“重新读取授权”，审阅当前状态后重新编辑、预览，不能直接重试旧提交。403 退出可写状态，清除草稿和预览，并隐藏旧快照、编辑和提交；手动重新读取且 GET 获准后才恢复只读快照，当前编辑器仍保持禁止写入，不能继续使用旧快照。

保存成功后的“查询本次审计”仅提供给同时具备 `system:audit` 的管理者，在页面内按 requestId、目标 application、目标 tenant 和 `DATA_GRANT_CHANGE` 精确查询。grant 读取/写入权限不等于审计权限。真实变更应能关联同事务记录；同值保存属于 no-op，没有新增审计时不能仅凭空列表推断请求失败。requestId 来自成功响应头 `x-request-id`，用于追踪，不能当幂等键。

### 目标选择接口

`GET /api/data-grants/options` 使用当前平台 session Bearer，要求独立 read 权限及有效操作者上下文。服务端只在 Phase 14 相同的策略 read 与实际会话组织子树交集内，返回应用及有效直接成员；不调用或授予通用用户目录/应用管理权限，不写 grant 或审计。

| 可选查询参数 | 规则 |
| --- | --- |
| `applicationId` | 非空 UUID；未选择时只返回应用选项，`records:[]`、`total:0` |
| `userId` | 非空 UUID；将应用和成员选项限制到该用户的可读有效成员，用于用户页面深链 |
| `keyword` | 去除首尾空白后最多 100 个 Unicode 字符且不超过 400 bytes；按 username/displayName 作大小写不敏感的字面子串搜索，影响成员列表 |
| `page` | 默认 1，范围 1–1000000 |
| `size` | 默认 20，范围 1–50 |

未知参数、无效 UUID 或超限值返回 400。`applications` 尊重 `userId`，不随 `keyword` 改变；成员按选定应用查询。指定未登记或当前不可读的应用不会泄露其名称，也不会返回该应用的成员；不能将空列表当成授权申请已完成。发现时的管理组织展开最多 10000 项，回复同样受 1 MiB JSON 预算限制，超限返回 400。

回复结构如下，UUID 均为说明用虚构值：

```json
{
  "applications": [
    {"id": "50000000-0000-4000-8000-000000000001", "code": "work-orders", "name": "工单管理"}
  ],
  "records": [
    {
      "target": {
        "tenantId": "10000000-0000-4000-8000-000000000001",
        "organizationId": "20000000-0000-4000-8000-000000000001",
        "userId": "30000000-0000-4000-8000-000000000001",
        "membershipId": "40000000-0000-4000-8000-000000000001",
        "applicationId": "50000000-0000-4000-8000-000000000001"
      },
      "username": "example-member",
      "displayName": "示例成员",
      "tenantName": "示例业务租户",
      "organizationName": "示例业务组织",
      "canWrite": true
    }
  ],
  "total": 1,
  "page": 1,
  "size": 20
}
```

`records[].canWrite` 是服务端按实际平台根、独立 write 和策略 write 得出的可写提示。页面同时检查当前实际平台根与 write；该提示不授予权限、不预留后续操作资格，快照/预览/PATCH 仍按请求时状态重新校验。Phase 14 的 Snapshot、Preview 和变更正文保持不变，没有把 `canWrite` 加入这些 DTO。

## 查询、预览和保存

所有端点使用当前平台 session Bearer。GET 严格要求以下五个查询参数：

```http
GET /api/data-grants?tenantId=10000000-0000-4000-8000-000000000001&organizationId=20000000-0000-4000-8000-000000000001&userId=30000000-0000-4000-8000-000000000001&membershipId=40000000-0000-4000-8000-000000000001&applicationId=50000000-0000-4000-8000-000000000001
Authorization: Bearer <当前平台session>
```

GET 和成功 PATCH 返回 Snapshot，结构为：

```json
{
  "target": {
    "tenantId": "10000000-0000-4000-8000-000000000001",
    "organizationId": "20000000-0000-4000-8000-000000000001",
    "userId": "30000000-0000-4000-8000-000000000001",
    "membershipId": "40000000-0000-4000-8000-000000000001",
    "applicationId": "50000000-0000-4000-8000-000000000001"
  },
  "version": "<服务端返回的64字符小写十六进制摘要>",
  "grants": [
    {
      "id": "60000000-0000-4000-8000-000000000001",
      "resource": "ticket",
      "action": "read",
      "scope": "SELF",
      "effectiveScope": "SELF",
      "organizationIds": ["20000000-0000-4000-8000-000000000001"]
    }
  ],
  "capabilities": [
    {"resource": "ticket", "action": "read", "scopes": ["ORG", "SELF"]}
  ]
}
```

版本占位文本只说明形状，调用时必须原样使用本次 GET/PATCH 返回的真实 64 字符小写十六进制值。列表顺序是服务端规范化结果，不能以 scope 数组位置推导大小关系。

`scope` 是已存值；`effectiveScope` 与 `organizationIds` 按目标成员的运行上下文计算，延续现有 app_context 的组织硬上限。普通业务组织的 TENANT 会有效收窄为 ORG_SUBTREE，SELF 仍须叠加 owner 条件与组织上限；不会因操作者是平台根而把目标用户的业务范围放大。业务接口仍需同时检查功能 permission、该 resource/action 的数据 grant 和 SQL 租户/组织/owner 条件。

GET显示当前最多128条stored grants，包括历史上未登记进当前管理策略的行；capabilities／managementScopes不追溯删除已有grant，未触及历史项保留原UUID。Phase19的memberLookups另行控制普通应用的成员辅助入口，不因此新增或撤销grant行。首版所有变更键仍须当前capability及菜单登记，包括remove；清理历史未登记pair时先受控恢复必要登记，再经正式删除，不能直接移除任意pair。

POST `/api/data-grants/preview` 和 PATCH `/api/data-grants` 接受相同的严格差量正文：

```json
{
  "target": {
    "tenantId": "10000000-0000-4000-8000-000000000001",
    "organizationId": "20000000-0000-4000-8000-000000000001",
    "userId": "30000000-0000-4000-8000-000000000001",
    "membershipId": "40000000-0000-4000-8000-000000000001",
    "applicationId": "50000000-0000-4000-8000-000000000001"
  },
  "expectedVersion": "<最近快照的真实version>",
  "upsert": [{"resource": "ticket", "action": "read", "scope": "ORG"}],
  "remove": []
}
```

差量总计最多 128 个操作，至少一项非空；resource/action 键不能重复，也不能同时出现在 upsert 和 remove 中。目标已存及合并后的 grant 总数各最多 128，组织展开最多 10000 项，JSON 回复最多 1 MiB；超限返回 400，写入在提交前完成预算检查并回滚。未知字段、缺字段、无效 UUID、scope、版本格式或未登记的 pair 都会拒绝。upsert 修改已有 grant 的 scope 时保留 grant UUID；未修改成员和关联记录的 UUID 保持，remove 只删除所列 pair。

预览回复为 `{target,version,before,after,changes}`：before/after 为上述 GrantView 数组，`changes` 是 `{added:GrantView[],updated:GrantView[],removed:GrantView[]}`，不是计数。仅预览中尚未落库的新增条目 `id:null`，成功 PATCH 后返回真实 grant UUID。预览不写 grant 或变更审计，不提交后续写入，也不预留锁；看到预览成功后仍需单独 PATCH。

`expectedVersion` 是并发条件，不是授权凭据。版本覆盖目标 grant 及其有效组织硬边界、应用/启用菜单登记与服务端策略，不包含操作者身份、session 或权限；不同获授权管理者可以交接同一目标版本，每次请求仍独立复验当前操作者。服务端在等待锁后重新读取当前目标、权限、策略与版本；过期值返回 409，应重新 GET，核对其它人的变更并重新预览，不能把旧 delta 无条件覆盖新状态。PATCH 只要求写链，持有版本不会绕过平台根、独立权限或目标策略检查。推荐交互流程采用同时具备读/写权的操作者执行“GET → 预览 → PATCH”。

## 事务、即时生效和错误处理

写入按用户 UUID 排序锁定需要的用户，再锁目标 membership，与成员维护的用户优先顺序一致；多个 grant writer 对同一成员串行。所有必要身份、范围、能力和版本检查在等待后使用新鲜读取。父组织/租户状态、菜单或应用授权的并发撤销以最后复核点为线性化边界；这不是对所有相关表持全局锁的承诺，下一次受保护请求继续检查最新数据库状态。

真实差量与 `DATA_GRANT_CHANGE` 审计同事务提交，审计显式记录目标业务 tenant/org/application、操作者与 requestId，以及必要的前后摘要；审计插入失败会回滚 grant 变更。同值 upsert 或删除本已不存在的合法 pair 属于 no-op，不新增变更审计。requestId 用于追踪，不是幂等键。

成功更新后，目标应用下一次受保护新请求从数据库读取新的 grant/context；不通过长授权缓存延长撤销窗口。平台失联时仍按已有 fail-closed 业务接入契约处理。管理页面消费相同快照、预览和 PATCH 契约，普通业务接口继续检查各自功能权限和数据范围。

| HTTP / code | 含义与处理 |
| --- | --- |
| 400 `BAD_REQUEST` | 严格 JSON/查询格式、未知字段、重复 pair、非法 scope/版本、未登记能力或超限；修正请求后重发 |
| 401 `UNAUTHORIZED` | 平台 session 无效或失效，重新认证 |
| 403 `FORBIDDEN` | 缺独立权限、写入非实际平台根、超显式范围、目标应用未授权/停用或当前用户/租户/组织等状态失效；不能靠修改 expectedVersion 绕过 |
| 404 `NOT_FOUND` | 目标用户/成员/租户/组织缺失或五个目标 UUID 绑定不匹配；不把其它上下文的 membership 当成当前成员 |
| 409 `CONFLICT` | 快照版本已变化，重新查询并审阅差量 |
| 500 `SERVER_ERROR` | 服务端明确的数据库或审计失败会回滚写事务；浏览器遇到 5xx 仍按结果待确认处理，重新读取后核对，保留请求编号 |

提交回执或响应丢失时，不根据网络错误推断回滚。具备查询权限的操作者重新 GET 同一目标，核对已存 rows/UUID/version，再决定下一次差量；不盲目替换新版本值重放旧意图。服务端不会返回 token/secret 或完整策略配置作为诊断。

## 工单工具与阶段边界

Phase 25 已退役旧工单 Node 管理入口与 SQL 草案输出，公开 manifest 保留为审阅输入。[独立 Rust CLI](../tools/platform-admin/README.md) 的 grants-plan 默认离线，grants-apply 必须显式 --apply，并通过同一正式管理 API 的权限、策略、版本和事务审计。CLI 仅增量授予缺项，同 resource/action 的 scope 冲突拒绝；scope 更新和撤销使用上述管理页面或 API另行审阅。角色菜单配置不会自动补齐数据范围。

角色可按既有组织祖先链提供功能菜单权限，并继续受当前租户和组织应用/菜单授权上限约束；data grant 只绑定当前组织的有效直接 membership，祖先组织的 grant 不沿角色继承。用户切换到另一组织时，即使功能菜单相同，也必须核对该组织直接成员上的 grant。

Phase14交付grant变更契约，Phase15提供管理页面和普通用户授权闭环；Phase25已交付独立Rust管理CLI。Phase19在普通应用Bearer成员契约上增加显式memberLookups与Rust工单分配预校验，Phase20独立提醒消费既有notification/publish授权；运行结果和限制分别由 [Phase19](validation/phase-19.md)、[Phase20](validation/phase-20.md)报告记录。本文示例不执行真实变更、不自动授权，不以配置／源码存在代替实际验证。
