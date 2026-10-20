# 关键管理变更与事务审计

最近更新：2026-10-16（Asia/Shanghai）。本文描述 Phase 13 的实现契约；实际运行、故障注入、失败与重跑结果见 [Phase 13 验证记录](validation/phase-13.md)。公开接口、功能权限和既有 action 码保持原有含义。

## 写入边界

关键管理操作完成输入和权限校验，开启或复用业务事务，在事务内读取必要快照、写业务状态、调用 `audit::write_in_tx` 或显式应用归属的 `write_in_tx_for_app`，最后提交。密码计算沿用受控异步服务；联系人初始化等需要事务状态的流程仍在原事务内等待该服务。审计插入错误传回调用方，业务写入随整个事务回滚。权限缓存只在提交成功后失效。

这项保证针对数据库事务：成功落库的关键变更及其审计同时存在，失败的同一事务不留下审计成功记录。它不把网络响应视为数据库提交证明，也不把所有日志或全部 HTTP 请求改成一个原子事件。

| 管理范围 | 对象与保留的 action | 事务及摘要 |
| --- | --- | --- |
| 角色、菜单、部门 | `CREATE_ROLE` / `UPDATE_ROLE` / `DELETE_ROLE` / `ROLE_MENUS`；`CREATE_MENU` / `UPDATE_MENU` / `DELETE_MENU`；`CREATE_DEPARTMENT` / `UPDATE_DEPARTMENT` / `DELETE_DEPARTMENT` | 对象创建、更新、删除，角色菜单和角色成员集合替换同事务；记录对象 ID、名称、必要属性和规范集合的数量/摘要 |
| 用户、成员、用户角色 | `CREATE_USER` / `UPDATE_USER` / `DELETE_USER` / `RESET_PASSWORD` | 用户资料、启停、批量启停、成员增删/调整、角色分配及密码重置同事务；跨租户用户变更按涉及的目标租户各记录一条，避免租户误读其它范围；不承诺每个 HTTP 请求只产生一行 |
| 租户与组织 | `CREATE_TENANT` / `UPDATE_TENANT` / `DELETE_TENANT` / `TENANT_AUTH`；`CREATE_ORGANIZATION` / `UPDATE_ORGANIZATION` / `DELETE_ORGANIZATION` / `ORG_AUTH`；`INITIALIZE_ADMINISTRATOR` | 创建同时覆盖自动组织、受管管理员角色及联系人成员初始化；授权级联清理和受管角色同步共用事务；创建/删除/初始化保留必要关联数量，普通配置变更只描述自身字段 |
| OAuth 客户端及管理撤销 | `CREATE_CLIENT` / `UPDATE_CLIENT` / `DELETE_CLIENT` / `TOKEN_REVOKED` / `KICK_SESSIONS` / `DEVICE_DENY` / `OAUTH_PURGE` | 客户端管理/secret 重置、凭据撤销、踢出会话、设备拒绝与手工历史清理同事务；凭据使用内部行 ID 与状态/数量，secret 只在原协议允许的响应中交付 |
| 会话与授权决定 | `CONTEXT_SWITCH` / `LOGOUT` / `UPDATE_PASSWORD` / `REVOKE_SESSION`；`CONSENT_APPROVE` / `CONSENT_DENY`；`DEVICE_APPROVE` / `DEVICE_DENY`；应用 `TOKEN_ISSUED` 与撤销 | 会话变更、改密撤销窗口、同意请求消费、设备批准/拒绝、应用 token 写入/撤销与审计共用相应事务；普通 OAuth 签发继续遵守 [Phase 11 契约](oauth-issuance.md) |
| 应用及平台设置 | `CREATE_APPLICATION` / `UPDATE_APPLICATION` / `DELETE_APPLICATION` / `UPDATE_PLATFORM_SETTINGS` | 应用入口/运行模式等配置，以及平台品牌、登录页展示和平台租户名称与审计一起提交；编辑正文和敏感配置不直接放入审计详情 |
| 静态版本与部署 | `REGISTER_APPLICATION_RELEASE` / `ADOPT_APPLICATION_RELEASE_ARTIFACT` / `PREFLIGHT_APPLICATION_RELEASE` / `ACTIVATE_APPLICATION_RELEASE` / `ROLLBACK_APPLICATION_RELEASE` | 登记/旧产物采用、预检结果、active/previous 指针各与自身审计同事务；应用归属明确绑定。预检与启用/回滚是独立事务，见下文 |
| 业务通知持久发布 | 原有通知发布审计及持久通知行 | 保留 publication、收件行、审计和 PostgreSQL `NOTIFY` 同事务的既有语义、发布幂等键及重放结果，不借 Phase 13 更改公开通知协议 |

## 快照、秘密及可追踪性

统一管理详情采用 `{objectType, objectId, before, after}`。创建前值和删除后值为 `null`。对象 ID 与必要名称明确保留；状态、期限、版本、授权数量及规范排序后的关联 ID 或 SHA-256 摘要按操作选取。等长的菜单/成员替换不能仅靠数量判断为相同。

删除前锁定目标并取得必要身份快照。依赖数据库补充租户或组织名称的删除，在目标存在时先插入审计，再执行删除；其它删除使用已取得的快照在同一事务中记录。`tenant_name`、`organization_name` 等历史名称在目标存在时填充，详情保留删除对象自己的 ID/名称。审计不是指向现存对象的级联子记录，删除后仍可追踪。

租户/组织删除在事务内将原有 `fk_organizations_parent_same_tenant` 与 `fk_roles_organization_same_tenant` 两个 `DEFERRABLE` 复合约束暂缓到整图级联结束，再显式切回 `IMMEDIATE` 校验后提交。两者与已有单列 `ON DELETE CASCADE` 配套；立即检查可能在子组织/角色级联尚未完成时拒绝合法删除。约束和历史迁移不改写，最终租户/组织一致性仍必须满足；不会靠提前删除成员或跳过审计绕过此边界。

不记录密码、密码 hash、JWT/refresh token、设备码、授权码、client secret、联系人初始化返回的初始密码或完整请求正文。联系人、说明和地址等字段不作为原文进入管理详情；需要区分这类字段的真实更新时，只对明确选择的配置字段计算稳定摘要。普通 tenant/org 配置操作的前后比较只使用本行锁保护的配置快照，不把同时发生的独立成员/角色变化归到本次请求。

审计保留操作者 ID/用户名、必要租户/组织身份快照、规范来源 IP、受限 User-Agent 和 requestId。requestId 用于关联一次调用的日志与多个事务/目标范围，**不作为全局幂等键**；复用 requestId 不允许压掉另一次真实变更。

## 重试与重复调用

相同值的配置 PUT、相同规范集合的授权 PUT、已初始化管理员和已终结的撤销/退出等操作，根据各端点已有状态判断和比较规则返回，不为没有实质变化的重复调用新增相同含义的变更事件。忽略 `updated_at` 之类技术时间戳，集合排序后比较；授权的级联清理和管理员角色补齐发生真实改变时，仍需要审计。

审计故障导致事务回滚后，重试以数据库当前状态重新校验并执行，最终只保留成功提交的事件。创建端点没有由本次变更引入通用幂等键，仍按现有唯一约束与协议处理重复创建。密码/secret 轮换是新的凭据操作，不能靠相同 requestId 自动去重或恢复上次明文响应。

提交确认丢失或响应中断可能发生在数据库已经提交之后。调用方先重读目标状态与可访问的审计；无法确认结果时明确按结果未知处理。不得将连接错误解释为已经回滚，也不得自动重复不可恢复的一次性凭据操作。ZIP 登记继续使用已有产物摘要、稳定版本与 storage 对账，详见 [ZIP 恢复说明](../deploy/zip-recovery.md)。

## 独立预检与授权并发

网络/文件预检在短数据库事务之外运行，随后把该次检查结果及 `PREFLIGHT_APPLICATION_RELEASE` 事件一起提交。启用/回滚再在独立事务内锁定应用并检查版本和运行模式，修改部署指针并写自己的事件。后一个事务失败时，先前成功的预检记录仍保留；同一 requestId 出现预检和部署两条记录是两个明确动作，不是重复审计。预检每次代表新的实际检查观察。

普通更新和集合管理优先使用 `FOR NO KEY UPDATE`，同目标写序列化且兼容外键 `KEY SHARE`；删除使用更强行锁。组织授权 PUT 在事务内按租户→组织顺序取锁，等待后重新检查租户应用/菜单范围，与租户授权撤销共用父锁。角色成员/用户角色写入沿既有用户锁顺序协调。这些局部规则不等于全部身份与授权管理已具备全局可串行化隔离；全面授权并发边界继续由台账对应阶段验收。

## 诊断事件及失败观测

| 分类 | 当前行为 |
| --- | --- |
| 登录尝试及失败诊断 | `LOGIN` / `LOGIN_FAILED` 等使用原有 best-effort 审计；登录保护/失败预算的资源边界按 [登录保护](login-protection.md) 执行 |
| 通知个人读取、删除及偏好 | 原有已读/删除/偏好日志保留 best-effort；与可靠通知持久发布区分 |
| HTTP、健康、指标及维护诊断 | 使用原有 tracing/指标/维护路径；不为普通读取创建关键管理事务 |
| 设备自然过期 | 维持既有过期状态维护和 409 分类；不是成功 approve/deny 管理操作，不能写成成功批准事件 |

统一 audit helper 插入失败时记录 `audit_write_failed` 及可用 requestId，并累加进程生命周期内的饱和计数 `platform_audit_write_failures_total`。计数同时覆盖使用统一 helper 的事务和 best-effort 写入失败，不带用户/租户/对象等动态 label，经已有受保护 metrics 端点导出；进程重启会重置，不能当作持久化全历史总量。普通 OAuth 的 Phase 11 签发路径仍有自身事务 SQL 审计和写入行数校验，不声称所有历史签发写入都经过这项统一计数。

本阶段管理操作没有需要可靠外发的外部消息，因此不新增外部消息总线或 outbox。通知的持久 publication/收件行仍是权威数据，事务 `NOTIFY` 是读取提示。未来新增必须可靠投递的外部副作用，应先在业务事务内持久化 outbox，再由独立投递器重试，不能在 commit 后补写审计并称之为原子。

## 验证入口

隔离 PostgreSQL 的 `management_audit_tests` 覆盖审计插入失败回滚、成功与重试、删除快照、秘密不落审计、集合替换及已有通知/发布事务边界。正式选择以 [CI manifest](../scripts/ci/manifest.mjs) 为准；当前清单还纳入既有通知/成员十项，规划十组 64 项；总数须由实际 runner 输出确认。操作清单、运行结果及尚存限制以 [Phase 13](validation/phase-13.md) 记录为准，不由本文的实现描述推导为全部验收通过。
