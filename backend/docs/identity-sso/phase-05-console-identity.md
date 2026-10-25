# Phase 5：控制台成员证明与会话隔离合同

本文说明本阶段管理端 API 与前端会话处理的安全合同和调用方式。当前实施状态及实际运行结果以[任务台账](task-ledger.md)和阶段验收记录为准；本文不代替 PostgreSQL、浏览器或构建门禁的通过证据。

本阶段延用平台控制台的员工入口。展示同账号的客户关系不授予客户控制台访问权；客户最小 API、独立入口及正式创建门禁属于 Phase 6 及后续阶段。固定 SDK、Context 1、MicroProps 2、Notification 1 的发布合同与历史副本不随管理端 DTO 扩展修改。

## 当前成员证明

`POST /api/auth/context` 和 `GET /api/auth/profile` 在响应顶层返回：

| 字段 | JSON 类型 | 含义 |
| --- | --- | --- |
| `membershipId` | UUID v7 字符串 | 服务器会话绑定的具体成员 ID。 |
| `memberType` | 字符串 | 实际成员类型；当前控制台成功响应为 `EMPLOYEE`。 |
| `identityVersion` | 规范十进制字符串 | 具体成员的身份代际，取值为 `1`～`9223372036854775807`。 |
| `accountVersion` | 规范十进制字符串 | 全局账号代际，取值为 `0`～`9223372036854775807`。 |

`user.id`、`tenant.id`、`organization.id` 继续分别提供账号、租户和组织 ID。前端从这七个服务器字段构造完整身份；没有另设 `contextIdentity` 响应包装。

版本字符串不接受小数、指数、正号或前导零。前端不把它们转为 JavaScript `number`，因此大于安全整数范围的合法 `i64` 仍能保持精确。这里的公开管理端版本格式与通知服务内部整数 proof／hint 合同不同；也不修改成员生命周期管理 API 原有的数字版本字段。

`context` 先校验账号、来源客户端策略与具体成员，再把所选成员及版本写入服务器 session。`profile` 从当前有效员工 session 读取证明，并在资料读取后复验同一证明；身份或账号状态在读取期间变化时不返回旧资料。客户端不能用 JWT 中的账号／组织、旧缓存或当前成员目录补齐缺失的服务器证明。

## 登录与可选成员列表

`POST /api/auth/login` 密码验证成功后，响应继续使用 `ticket`、`memberships`、`accountCount`。`GET /api/auth/profile` 也返回本人 `memberships`。每条选择项包含真实成员数据：

| 字段 | 含义 |
| --- | --- |
| `userId`、`membershipId` | 该关系真实绑定的账号及成员 ID，均为 UUID v7。 |
| `memberType`、`status` | 该行实际的 `EMPLOYEE`／`CUSTOMER` 和 `ENABLED`／`DISABLED`。 |
| `identityVersion` | 该成员实际代际的规范十进制字符串。 |
| `tenantId`、`tenantName` | 所属租户。 |
| `organizationId`、`organizationName`、`organizationPath` | 所属组织。 |
| `departmentId`、`departmentName` | 员工部门；客户关系不携带员工部门。 |
| `consoleSelectable` | 该关系当前是否可通过此控制台入口选择。 |
| `selectionDisabledReason` | 可选时为 `null`；客户为 `CLIENT_MEMBER_TYPE_NOT_ALLOWED`，已停用员工为 `MEMBERSHIP_DISABLED`。 |

登录选择项另带 `username`、`realName`，便于共享电话或 Email 命中多个已验证账号时明确选择；系统不会据此合并账号。列表仅列出当前租户及组织链有效的本人关系；停用员工和客户关系可以显示各自状态及不可选原因。

展示列表与登录票据授权分别受控。只有启用员工成员会进入签名 ticket 的具体成员／版本快照，客户及停用员工的展示项不会成为可签发上下文的证明。客户专属账号、没有有效员工成员的账号不能通过旧控制台登录；开通生命周期测试开关也不会改变这一点。

## 显式选择与普通切换

新前端登录选择及已登录切换均向 `POST /api/auth/context` 显式提交 `membershipId`。可以同时提交 `organizationId` 和 `userId`，服务器要求三者一致。以下请求中的占位符必须换成实际存在且经当前票据验证的 UUID v7：

```json
{
  "membershipId": "<selected-employee-membership-v7>",
  "organizationId": "<selected-organization-v7>",
  "userId": "<password-verified-account-v7>"
}
```

请求使用登录 ticket 或当前控制台 session 作为 Bearer。ticket 只能选择本次密码验证覆盖的账号和原成员代际；session 不能通过 `userId` 切换到其他全局账号。成员 ID 与组织不一致、选择同组织客户成员、成员停用或旧代际证明均拒绝，不会退回该账号同组织另一条员工记录。

为兼容旧调用方，只提交 `organizationId` 时仍可选择该账号在该组织唯一有效的员工关系；没有唯一员工资格就拒绝。新界面使用具体成员 ID，不依赖该兼容规则。请求未知字段和非法 UUID v7 返回 `400`。

普通选择不创建成员、不启用停用成员，也不修改成员类型或版本。成功切换创建新的服务器 session，并撤销旧 session；拒绝切换不会撤销原有效 session。刷新只轮换同一服务器会话的凭据，前端保留原已证明身份，随后 `profile` 仍须与当前证明一致。

## 只读生命周期能力接口

`GET /api/memberships/capabilities` 要求有效员工 session 和 `system:user` 权限。它在写开关全部关闭时仍允许有权限管理员读取，精确返回三个服务器配置值：

```json
{
  "lifecycleEnabled": false,
  "customerMembershipsEnabled": false,
  "dualMembershipsEnabled": false
}
```

| 字段 | 配置与界面用途 |
| --- | --- |
| `lifecycleEnabled` | `MEMBERSHIP_LIFECYCLE_ENABLED`；新增生命周期写接口的总门禁。 |
| `customerMembershipsEnabled` | `CUSTOMER_MEMBERSHIPS_ENABLED`；创建客户或以客户为目标的额外门禁。 |
| `dualMembershipsEnabled` | `DUAL_MEMBERSHIPS_ENABLED`；同账号同组织第二身份的额外门禁。 |

接口没有 `reason` 或 schema 就绪字段。前端依据各字段显示关闭原因，不能用构建环境、页面路由或本地存储把关闭值覆盖为启用。读取失败时不能推定允许写入。没有 `system:user` 的员工返回 `403`，客户或失效 session 无法通过员工提取器。

Phase 5 验收时，开关仅允许隔离 `APP_ENV=test` 且应用环境非 production 开启，双身份正向测试仅在专用目标 fixture 中显式移除旧约束。三个开关当前仍默认关闭；现行[Phase 6 独立收缩与启用合同](phase-06-rollout.md)保留该 fixture 分支，其他环境须提供环境绑定的 receipt 与 manifest 摘要并通过启动验证。能力响应只报告配置，不承诺旧二元唯一约束已经收缩。[Phase 6](validation/phase-06.md)已在任务自有 development 目标验收真实收缩和写入，生产未启用客户或第二身份。

## 生命周期操作与管理边界

管理界面的普通成员选择与身份关系管理是不同操作。员工和客户按各自真实成员行展示；客户行不提供员工角色、员工部门或岗位授权入口。旧 `/api/users/{id}/memberships` 批量维护仍只操作员工关系，不能用缺省类型覆盖客户。

新增关系使用 `POST /api/memberships` 并显式提交已有账号 `userId`、组织、类型及 `reason`。绑定客户还要求 `accountBindingConfirmed: true` 与 `bindingSource`，不能仅凭电话、Email 或知道账号 ID 推断归属。

保留源身份使用 `POST /api/memberships/{id}/second-identity`；停用源并切换关系使用 `POST /api/memberships/{id}/replace-identity`。目标已存在时必须提交匹配的 `expectedTargetIdentityVersion`；目标停用时还须显式 `enableTarget: true`。普通表单不能直接修改单行 `member_type`，也不能自动恢复旧会话或旧权限。独立启停使用具体成员的 `/status` 接口和 `expectedIdentityVersion`。

完整请求及错误合同沿用[Phase 1 成员 API](phase-01-api.md)。应用和人类客户端身份集合分别沿用[Phase 2 策略管理 API](phase-02-policy-api.md)；客户端成员类型集合不能扩大应用授权，机器客户端保留独立主体用途。

## 缓存、请求与标签页

前端身份命名空间使用完整有序元组：

```text
[userId, tenantId, organizationId, membershipId, memberType, identityVersion, accountVersion]
```

`context-identity.ts` 校验四个 UUID v7、已知成员类型和精确版本字符串；控制台提交及恢复还要求成员类型为员工。登录选择项另用账号、租户、组织、具体成员、类型和身份版本构造选择键，同组织员工与客户不会共用选择项键。

共享会话记录升级为 `platform-session-v2`，保存完整服务器证明和由其构造的 `contextKey`。原 v1 记录或拆分旧 token 缺少具体成员证明，要求重新认证，不能按当前账号／组织猜测后自动刷新。浏览器本地的逻辑 session ID、记录递增 `version` 与服务器 `accountVersion` 各有职责，不互相替代。

登录、刷新、切换和注销继续通过同一个 Web Locks 协调入口。跨标签页广播只用于提示记录版本变化，页面以持久会话记录为准；无法确认的消费或切换结果进入 `unknown` 并停止受保护请求。存储或 Web Locks 不可用时显示协调错误，不能在各标签页独立继续消费凭据。

身份变化使旧请求生命周期取消、上下文 epoch 递增，并清理旧 profile、菜单、权限、选中应用及通知状态。请求在发送前和响应体读取后核验逻辑会话、完整身份及 epoch；旧标签页、旧回调和晚返回结果不能写入新身份。React Query 授权键、通知上下文、应用选择缓存、最近页面／搜索与侧栏菜单折叠均纳入完整成员命名空间，刷新同一有效身份不凭空创建另一成员命名空间。

应用或客户端准入策略成功保存后，当前窗口依据保存前捕获的 session fence 调用 `retireAuthorizationProjection`：同步取消旧请求、清除 profile／当前应用、递增 epoch，重新挂载并读取授权投影。它不通过伪造 token 刷新或变更成员关系清理页面。该本地动作不代表所有标签页都立即收到策略广播；其他页面的后续请求仍由服务端准入和凭据版本检查约束，页面依据既有授权重读机制收敛。

`profile` 与当前 session 证明不一致时，前端不会接受它覆盖当前身份。私人页面还随逻辑会话／epoch 重新挂载，避免旧表单草稿遗留在另一身份；服务端逐请求授权仍是最终访问边界。

## 验证范围

[identity_ui_tests.rs](../../src/identity_ui_tests.rs) 是使用显式 `IDENTITY_PHASE05_TEST_DATABASE_URL` 的 PostgreSQL HTTP 集成用例。它只创建、迁移及清理本轮自有 `identity_phase05_*` 数据库，分别检查服务器证明、ticket 范围、精确成员选择、普通切换不变成员快照、原会话撤销、实际角色权限和配置门禁。有效客户 session 由明确允许客户的 fixture 来源证明，用于验证员工 API 拒绝真实客户身份；它不开放生产客户入口。

前端模型及会话测试检查版本解析、命名空间、持久记录、跨标签页协调和旧响应失效。真实源码 UI 的浏览器 fixture 可以证明组件、缓存和请求生命周期在受控响应下的表现；模拟的 HTTP 成功不能代替 PostgreSQL 事务、凭据撤销或真实权限检查结果。反过来，Rust HTTP 测试也不证明页面点击、弹窗校验或真实浏览器多标签页行为。

实际命令、运行结果、失败与重跑过程、fixture 边界及清理证据应分别写入阶段验收记录。仅存在测试源码、通过静态检查或普通测试跳过了 `#[ignore]`，均不能作为数据库／浏览器专项已通过的结论。
