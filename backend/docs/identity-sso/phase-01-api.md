# Phase 1：成员生命周期 API 与扩展迁移

本阶段实现管理端接口与 schema 扩展，不开放正式客户入口、客户凭据签发或 SSO。主执行范围见[任务台账](task-ledger.md)，写路径审查见[成员路径清单](phase-01-membership-path-audit.md)，实际验证见[阶段证据](validation/phase-01.md)。

## 配置与部署边界

| 配置 | 默认值 | Phase 1 的作用 |
| --- | --- | --- |
| `MEMBERSHIP_LIFECYCLE_ENABLED` | `false` | 控制所有新增生命周期写接口，包含新增员工、状态、删除和身份组合操作。 |
| `CUSTOMER_MEMBERSHIPS_ENABLED` | `false` | 额外控制新增客户关系及转换目标为客户的操作。 |
| `DUAL_MEMBERSHIPS_ENABLED` | `false` | 额外控制同账号同组织第二身份；旧员工批量维护和联系人初始化也受此门禁。 |

Phase 1 验收时，任何一个开关为 true 都要求显式 `APP_ENV=test` 且不处于 production 应用环境；当时普通 development／production 开启会在配置加载时失败。三个开关当前仍默认关闭，隔离 test 保留 fixture 兼容分支；其他环境须按[Phase 6 独立收缩与启用合同](phase-06-rollout.md)成对提供环境绑定的 receipt 与 manifest 摘要，并通过服务启动验证。客户或双身份开关仍要求生命周期开关。[Phase 6](validation/phase-06.md)已通过任务自有 development 收缩、启动与 HTTP 验收，生产未启用；不能以修改环境名称在真实业务库抢先开放。

本阶段仅允许隔离 fixture 使用客户写接口。旧平台登录、context、session／refresh、普通用户 OAuth 和应用凭据的资格证明统一限定为有效员工，客户不能借旧入口签发员工凭据。成员与凭据的完整版本绑定由 Phase 2 继续实现。

## Schema

[0013_membership_identity_policy.sql](../../migrations/0013_membership_identity_policy.sql) 是追加的扩展迁移，保留已有 migration 字节：

- `memberships` 增加 member_type、status、identity_version、updated_at。成员类型为 EMPLOYEE／CUSTOMER，状态为 ENABLED／DISABLED，版本至少为 1。
- 保留原 `UNIQUE(user_id, organization_id)`，另加 `UNIQUE(user_id, organization_id, member_type)`。本阶段不发布删除旧约束的收缩迁移；真实双行创建仍会被原约束阻止。Phase 6 才执行独立收缩。
- 成员 ID、账号、组织、类型不能原地改写；版本不能倒退，状态改变必须递增版本。客户不能携带员工部门或岗位。
- 应用增加非空 allowed_member_types 与 access_policy_version；客户端另有 subject_mode。成员集合规范顺序为 EMPLOYEE、CUSTOMER，拒绝未知值、NULL、重复及异常数组形状。
- 新 USER client 默认仅 EMPLOYEE，不得包含机器 grant；MACHINE 必须为 CONFIDENTIAL、仅 client_credentials 且成员集合为空。空集合不代表人员通配。扩展升级只保留迁移前真实存在的 LEGACY_MIXED，禁止新建或转入该模式。
- 准入相关配置变更递增策略版本；旧管理接口暂不提供成员策略编辑，后续 Phase 2 完成管理 API 与全部撤销消费。

删除触发器的 Phase 1 兼容分支只处理现有员工凭据：删除 EMPLOYEE 时撤销对应账号／组织的 session、平台 refresh、OAuth token、code 与 device；删除 CUSTOMER 不触碰员工凭据。由于旧凭据尚未绑定具体成员，本阶段不宣称客户 token 的精确撤销已经实现；客户签发继续被关闭。

## 通用合同

所有接口要求有效员工管理会话及 `system:user` 权限。组织必须真实存在，属于允许管理的租户／组织子树；平台根上下文沿用已有跨租户管理规则。账号、组织、部门及 Path ID 都要求 RFC variant 的 UUID v7。

成员响应采用 camelCase，字段为 id、userId、organizationId、memberType、status、identityVersion、departmentId、position、updatedAt，不附加该账号另一身份的角色或授权。

写操作按稳定顺序锁定操作者和目标账号，再按 ID 锁成员；不在账号锁后强锁组织／租户，避免与既有联系人初始化反序。事务中在锁等待后和提交前重验具体操作者 session、有效员工资格、实时权限、租户／组织链、到期时间与状态指纹。并发撤权、停用、到期或范围变化导致拒绝并回滚。

所有新生命周期写请求都必须提供 `reason`，说明创建、启停、删除、新增第二身份或替换的实际原因。去除首尾空白后须为 1～500 个字符，不得包含控制字符；缺失或非法值返回 400。操作名称不能代替变更原因。

审计与业务写入在同一事务提交；审计失败也回滚版本、凭据撤销和成员变更。审计记录实际变更原因、来源、目标成员、操作者 context、绑定依据及兼容撤销范围，不记录原始 token 或密码。

当前上下文的 EMPLOYEE 不允许通过这些管理接口自行变更状态、删除或替换，返回 403；同账号其他组织、或同组织 CUSTOMER 关系不因此被禁止。账号全局管理仍沿用原有入口及规则。

## 接口

| 方法与路径 | 行为 |
| --- | --- |
| `GET /api/memberships` | 按管理范围分页；可用 userId、organizationId、tenantId、memberType、status 过滤，分页参数沿用 page／size。 |
| `GET /api/memberships/{id}` | 读取具体成员；越界与不存在均不返回目标资料。 |
| `POST /api/memberships` | 创建绑定已有账号的明确类型关系，不自动创建全局账号。同组织同类型重复返回 409。 |
| `PUT /api/memberships/{id}/status` | 显式启停具体成员，要求 expectedIdentityVersion；递增版本并撤销该关系适用的兼容凭据。 |
| `DELETE /api/memberships/{id}` | JSON body 指定 expectedIdentityVersion；先记录最终版本，再删除具体关系，清理不再被有效员工支持的角色。 |
| `POST /api/memberships/{id}/second-identity` | 创建或显式复用另一类型目标关系，保留源关系状态与版本。 |
| `POST /api/memberships/{id}/replace-identity` | 创建或显式复用目标，停用源并递增其版本；不原地改类型，也不把源历史数据改绑目标。 |

新接口成功返回 200；列表使用原有 records／total／page／size 包装，其他读取／创建／状态返回成员 DTO。第二身份与替换返回目标成员 DTO，并带 sourceMembershipId。删除返回 id、deleted、identityVersion。

### 创建与绑定

必填 userId、organizationId、memberType、reason；可选 departmentId、position。客户另须 `accountBindingConfirmed: true` 和非空单行 bindingSource（最多 200 字符），由有权限管理员显式确认并记录账号绑定依据。知道全局账号 ID 本身不构成授权；组织范围和管理权限仍独立检查。客户禁止部门与岗位，员工部门必须属于同一组织，岗位最多 50 字符。

示意请求，ID 占位符须换为实际存在的 v7 账号与组织：

```json
{
  "userId": "<existing-user-v7>",
  "organizationId": "<existing-organization-v7>",
  "memberType": "CUSTOMER",
  "reason": "为已核实账号开通本组织客户身份",
  "accountBindingConfirmed": true,
  "bindingSource": "管理员已核实的账号绑定依据"
}
```

### 独立启停和删除

状态 body 为 `{"status":"DISABLED","expectedIdentityVersion":1,"reason":"员工离职"}`；删除 body 为 `{"expectedIdentityVersion":1,"reason":"移除误绑定的组织关系"}`。版本不匹配返回 409；非法版本或原因返回 400。普通临时停用保留员工角色配置，但当前凭据与资格立即失效。重新启用须显式操作，不恢复旧会话。

### 新增第二身份与替换

body 必须包含 memberType、expectedIdentityVersion（源）、reason。目标已存在时，还必须提供匹配的 expectedTargetIdentityVersion。目标停用时，须明确 `enableTarget: true`；否则拒绝。目标不存在却提供 expectedTargetIdentityVersion 时返回 409，不能悄悄创建替代它。

目标为客户时仍须账号绑定确认与来源。目标已停用后通过组合流程启用，不恢复其旧成员授权或会话；永久退出的员工角色按仍然有效的其他员工关系及祖先支持集合清理，客户不提供员工角色支持。替换会清理源成员的数据授权，并保留其他组织仍受支持的员工角色和会话。

普通登录切换不是上述操作，不创建／启用成员，不修改 member_type。本阶段所有新增写接口默认关闭，双行正向验证仅在显式去除旧二元约束的专用目标 schema fixture 中进行。

## 旧员工入口与错误

`/api/users/{id}/memberships` 的无类型 GET／PUT 只处理 EMPLOYEE，保留客户行；已有 DISABLED 员工可更新资料但不隐式启用。旧请求夹带 memberType、status、identityVersion 或 membershipId 会被拒绝，避免借旧 DTO 绕过生命周期服务。全局账号范围检查仍覆盖全部成员类型，不能因为只管理员工就忽略账号的范围外客户关系。

默认关闭或缺权限为 403；参数／绑定依据非法为 400；目标不存在或越组织范围为 404；重复成员、版本或权限／组织状态并发变化为 409。失效 session 为 401，权威依赖不可用为 503；所有失败都不提交部分成员或授权变更。
