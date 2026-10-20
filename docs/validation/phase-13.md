# Phase 13 执行与验证证据

日期：2026-10-16（Asia/Shanghai）。阶段：Phase 13，使关键管理变更与审计同事务。状态：**COMPLETED**。用户明确要求执行本阶段，终态按此前“完成后标记台账”的要求已采纳。

## Executed Phase

Phase 4、11 已完成，本次仅实施 Phase 13。遵循 task-ledger-executor、[架构优化台账](../architecture-optimization-ledger.md) 和 `D:/WorkSpace/rust/AGENTS.md`，保留此前未提交工作。当前指针已同步为 **Phase 14 / NOT_STARTED**。

Git HEAD 保持 `32956c4532bbd6eff53584fd0f5e9bb6e188437c`；最终 r2 冻结 [phase-13-inputs.json](phase-13-inputs.json) 为 **506 文件**、SHA-256 `582d2759e38c06f62ea609bb9b0debfe634e3415cde6617cb604f401bccec9ab`。相较 Phase 12，**26 路径变化、1 文件新增、无删除**；HEAD 不能替代未提交工作树内容摘要。正式 PG、fast、host 静态检查及资源校验均绑定该最终快照，见 [范围核验](phase-13-boundaries.json) 和 [运行索引](phase-13-runtime.json)。

## Execution Summary

- 公共事务审计直接使用主事务，SQL 异常及 BEFORE INSERT trigger 的 RETURN NULL 均作为失败；必须插入恰好一行，调用方 await? 后才允许提交。best-effort 仅保留明确分类的登录及个人通知诊断。
- 角色/菜单/部门、用户及成员/角色/状态/密码、租户/组织及管理员初始化、OAuth 客户端/撤销/授权决定、应用/平台设置和发布管理均已接入。删除前取必要身份，关键对象的名称快照不依赖删除后补查。
- 详情采用明确选择的 objectType/objectId/before/after，集合使用排序 ID 或数量与 SHA-256，正文只标记变化字段或摘要。没有密码、密码 hash、JWT、refresh token、client secret、设备/授权码和通知正文。
- 同值/同集合重试无重复变更事件。requestId 负责相关性，不是全局幂等键；多目标租户用户变更按既有可见性每租户各一条，任意一条失败回滚全部。
- 发布网络预检保持事务外执行，诊断结果和其审计独立提交；active/previous 指针与自身审计共用主事务。ZIP 已安装不可变字节、未知 commit 对账、注册幂等与通知 publication/delivery/NOTIFY 的原事务语义保留。
- 统一 helper 的审计失败累加进程生命周期饱和计数 `platform_audit_write_failures_total`，无动态标签；日志保留 requestId。普通 OAuth Phase 11 签发保留自己的严格事务 SQL，不虚称全部历史审计都经过新计数。

## Deliverables

| 交付 | 实现与边界 |
| --- | --- |
| 事务审计接口/安全摘要 | [audit.rs](../../backend/src/util/audit.rs)，AuditInfo 字段兼容，新增 write_in_tx / write_in_tx_for_app / change_detail |
| 关键操作清单/采用规则 | [management-audit.md](../management-audit.md)，列出 action、事务、删除快照、重试、诊断及外发边界 |
| 管理/授权完整调用链 | roles/users/tenants/organizations/auth/menus/departments/oauth_admin/applications/application_releases/oauth2/device/platform_settings；既有业务与授权接口保持 |
| 平台设置与通知 | 平台品牌/登录页展示及平台租户名称共同提交；通知原 publication、收件行、审计、NOTIFY 和幂等重放维持 |
| 失败观测 | [observability.rs](../../backend/src/observability.rs) 的常量内存饱和 counter，经既有受保护指标入口导出，进程重启重置 |
| 真实故障及并发回归 | [management_audit_tests.rs](../../backend/src/management_audit_tests.rs)，八个真实 HTTP/PG 测试；原通知/成员十项补入必需清单 |
| 现状/执行状态 | README、architecture、engineering-baseline、CI README 和原台账已同步；[完整已采纳更新](phase-13-ledger-update.md) |

没有新增迁移、第三方依赖、crate、工作区或前端/SDK/业务消费者改动。0001/0002/0003 校验和、普通 OAuth issuer 存储/消费模块、app token 协议模块、Phase 12 密码池/限流/代理配置与实现均不变。session.rs 只删除已无调用的 autocommit revoke_one 包装器，psr 实现不变；onboarding 只将一处父组织行锁改为兼容 FK KEY SHARE 的 NO KEY UPDATE。

## Actual Verification

正式最终结果只使用 r2；各早轮失败及不同源码结果独立保留，不与最终通过相加。

| 检查 | 最终实际结果 | 证据 |
| --- | --- | --- |
| Linux PostgreSQL | **10 组 / 64 PASS / 0 FAIL / 0 skip** | [postgres-result.json](../../target/phase-13/linux-postgres-r2/reports/postgres/postgres/postgres-result.json) |
| 原 Phase 12 选择 | **原八组 46 全部保留并通过** | manifest 条目及最终 collector 双重核对，OAuth10、psr4及登录保护5全部通过 |
| 新管理审计 | **8/8 PASS**，实际 **67 个矩阵场景 / 134 次故障 / 18 个 no-op** | [management_audit_tests.log](../../target/phase-13/linux-postgres-r2/reports/postgres/postgres/management_audit_tests.log)，只统计运行时安全 marker |
| 既有通知/成员完整组 | **10/10 PASS** | [notification_tests.log](../../target/phase-13/linux-postgres-r2/reports/postgres/postgres/notification_tests.log) |
| Linux fast | **27/27 步 PASS，532 项选定测试通过，零必需 skip** | [summary.json](../../target/phase-13/linux-fast-r2/reports/fast/summary.json) |
| Windows 默认测试 | **86 PASS / 0 FAIL / 81 ignored** | 下表原始 default 日志；移除 DATABASE_URL/TEST_DATABASE_URL，ignored 不充当 PG 通过 |
| 源码与运行资源 | **PASS** | [runner-final-verification.json](../../target/phase-13/runner-final-verification.json)，宿主/快照/容器前后同 SHA、随机 owner 清理、lease 退出0 |

| Host 命令 | 结果 | 日志 |
| --- | --- | --- |
| rust-check | PASS / exit 0 | [原始日志](../../target/phase-13/static-final-r2/rust-check.log) |
| windows-default-tests | PASS / exit 0 | [原始日志](../../target/phase-13/static-final-r2/windows-default-tests.log) |
| rust-doc | PASS / exit 0 | [原始日志](../../target/phase-13/static-final-r2/rust-doc.log) |
| strict-clippy | FAIL / exit 101 | [原始日志](../../target/phase-13/static-final-r2/strict-clippy.log) |
| strict-fmt | FAIL / exit 1 | [原始日志](../../target/phase-13/static-final-r2/strict-fmt.log) |
| new-module-format | PASS / exit 0 | [原始日志](../../target/phase-13/static-final-r2/new-module-format.log) |
| phase-diff-whitespace | PASS / exit 0 | [原始日志](../../target/phase-13/static-final-r2/phase-diff-whitespace.log) |

全目标/全 feature check、文档、新模块格式与最终 diff 检查均通过。全仓严格 Clippy 实际为既有 **38 项诊断 FAIL**，完整 fmt check 也保留既有格式债务，未关闭检查、未用 allow 压制、未批量格式化无关文件。本次测试初版新增的一项 type_complexity 已改为私有类型别名，最终新模块没有新增诊断。

### 故障与并发断言

- 每个矩阵场景用自有临时 PG trigger 注入 RAISE 或 RETURN NULL；请求失败时业务表和审计表快照不变，失败计数恰好加一；恢复后业务确实改变，action/requestId/actor/必要名称可追踪，成功不增加失败计数。
- 双租户用户变更特意在第一条审计已写入后让第二条失败，验证主写和第一条审计都回滚；成功同 requestId 两租户各一条，保留原可见性约定。
- 部门 CRUD、角色/用户绑定、等长菜单与成员/角色替换、同值/同集合/已初始化等 no-op、初始密码/密钥响应的秘密检查均通过。
- 组织授权请求实际等待父租户锁，待撤回授权提交后必须重新校验并拒绝写回；kick 实际等待用户锁，随后读取新提交 token 的计数并撤销，audit before 1 / after 0。
- 应用删除持有应用锁并等待客户端；客户端换关联请求必须先等待应用锁，释放屏障后删除成功、客户端保留而 binding 清空、旧 binding CAS 被拒绝，没有 deadlock 成功事件。屏障观察 pg_stat_activity、wait_event_type=Lock 和 pg_blocking_pids，不用固定 sleep 替代交错证据。
- 删除租户/组织后精确检查 tenant_name/organization_name、目标及相关子组织/角色/部门/成员消失；通知两入口实际投递且重复 key 不新增 publication/delivery/audit，RFC7009 重复、未知和 foreign-client 请求保持兼容。

### 级联外键修复与兼容

首轮真实组织删除暴露既有 schema 的配套单列 CASCADE 与复合 DEFERRABLE NO ACTION 的检查顺序。两个独立 PG 诊断均重现 SQLSTATE 23503；角色图指向 fk_roles_organization_same_tenant，完整子组织树还指向 fk_organizations_parent_same_tenant。只暂缓这两个原约束后，原 DELETE 与立即恢复检查成功，完整图子树和整租户级联均无残留。

组织/租户删除事务现在在审计后执行这两个指定约束 DEFERRED、原 DELETE、两个约束 IMMEDIATE，再 commit；不修改迁移，不通过提前清成员绕过规则，提交前仍验证最终完整性。原始证据：[角色图](../../target/phase-13/phase13-cascade-ab253d3606.log)、[完整图](../../target/phase-13/phase13-cascade-graph-a7e9e991.log)、[独立确认](../../target/phase-13/sql-delete-diagnostic-1792143197395/result.json)、[诊断资源元数据](../../target/phase-13/sql-cascade-diagnostic-resources.json)。最终 HTTP 级联回归保留管理员、孙组织、父子部门及关联部门成员。

## 早轮失败记录

| 轮次/输入 | 实际结果与原因 | 处理及证据 |
| --- | --- | --- |
| WIP r1 / 506项 / 95a433e9… | 管理审计 **7 PASS / 1 FAIL / 0 skip**；合法已初始化组织 DELETE 重试400 | 独立 PG 确认上述真实复合外键顺序缺陷，作事务内窄修；[原结果](../../target/phase-13/linux-audit-wip-r1/reports/postgres/audit-result.json) 与原日志不改写 |
| 完整候选 r1 / 506项 / 128d9732… | fast27 PASS，PG **63 PASS / 1 FAIL / 0 skip**；第二 TENANT_AUTH 夹具本是无变更增量请求，返回200符合 no-op 合约 | 使用两个自有非管理菜单，并显式 old:false/new:true；前后 SQL 确认同数量但 ID 集不同，不改生产 API；[原 PG 结果](../../target/phase-13/linux-postgres-r1/reports/postgres/postgres/postgres-result.json) 保留 |
| 同候选 host r1 | check/default/doc/新格式 PASS；diff FAIL2：本次移除旧包装器后多留 EOF 空行 | 修正尾空行并清理一条过时测试整合注释；[原 static 结果](../../target/phase-13/static-final/result.json) 保留，重新冻结后完整重跑 r2，而非继承不同字节的通过 |

所有 r1/WIP 都保存 driver 与输入原字节、各自前后源校验和自有资源清理，不把 FAIL 重写为 PASS。原 Phase 12 证据保留执行时点含义，其指向后来修改文件的哈希不递归解释为本轮证据。

## 工具、命令与资源

Node 24.18.0/npm 11.16.0、Rust 1.97.0；WSL Linux Docker 29.1.3、正式 PostgreSQL 17.6 与固定工具镜像沿用项目受管环境。默认 PATH Node16 未用于本轮验收。

```text
node target/phase-13/run-postgres-linux.mjs docs/validation/phase-13-inputs.json D:/WorkSpace/rust/saas/rust-saas/target/phase-13/linux-postgres-r2 full
node target/phase-13/run-postgres-linux.mjs docs/validation/phase-13-inputs.json D:/WorkSpace/rust/saas/rust-saas/target/phase-13/linux-fast-r2 fast
node target/phase-13/run-static-checks-r2.mjs
node target/phase-13/check-boundaries.mjs
node target/phase-13/collect-runner-evidence.mjs
node target/phase-13/update-ledger-complete.mjs
node target/phase-13/write-report.mjs
node target/phase-13/collect-final.mjs
```

PG 由 manifest 精确选择十组，显式 --ignored --test-threads=1，失败计数 exact+1 在串行选择内断言。新测试只接受显式 TEST_DATABASE_URL；运行只用 launcher 新建私网 tmpfs PG，无宿主端口、生产数据、源码 bind 或 Docker socket。最终各 owner、saas.phase13.owner、saas.phase13.cascade、phase13 名称及两个精确诊断容器均有只读空查；所有 WSL lease 正常停止并退出0，无全局 prune。工具镜像/日志作为可复核缓存保留。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 故障注入审计插入失败时，关键管理变更回滚或存在可靠事务事件 | PASS | 最终8组管理审计的67个真实操作场景，RAISE/RETURN NULL、第二租户审计、业务/审计全表快照、失败计数与恢复重试均通过 |
| 成功操作恰有可追踪审计，无 token/secret；通知/发布现有事务语义不退化 | PASS | requestId/actor/删除名称、18个no-op、集合等长替换、秘密名单、3类真实锁屏障及通知/发布幂等/独立预检通过；既有通知十项和原46项全部保留 |

## Risks / Notes

无剩余实施或必需验收项。上述原子性针对数据库主事务；预检、技术时间戳、诊断、超时和结果未知按 [管理审计规则](../management-audit.md) 的明确边界处理。没有通用管理 POST 幂等或跨全部授权路径的全局可串行化承诺。联系人初始化仍沿原事务等待受控密码服务，全面事务缩短属于后续阶段。

审计失败 counter 进程重启归零；普通 OAuth 自有严格 SQL 审计单列。没有可靠外发的新需求，因此没有引入 outbox；未来需要外部投递时应先同事务持久化事件再重试。全仓 Clippy/fmt 既有 FAIL 与默认测试 ignored 都不计为必需 PG 通过。没有生产数据库操作、外部发布或 Phase14 实施。

## Suggested Task Ledger Update

用户已明确授权完成后更新台账。本次 Phase13 汇总、详情、两项验收、证据、决策、变更记录均已采纳 **COMPLETED**，当前指针 **Phase14 / NOT_STARTED**；所有37阶段汇总/详情一致。完整结构化更新见 [phase-13-ledger-update.md](phase-13-ledger-update.md)。
