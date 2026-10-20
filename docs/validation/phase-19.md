# Phase Execution Result — Phase 19

状态：`COMPLETED`。三条台账验收及Phase19范围内必需门禁全部通过，[managed审计](../../target/phase-19/managed-audit.json)为`PHASE19_GATES_PASS`、26项检查全部PASS，绑定268份原始证据摘要。采用同一r3候选的scoped PostgreSQL 16/16、完整fast 53/53和raw runtime 12/12实际结果；fast为765 PASS/0 FAIL/0 skip。r1受控取消、r2三份终态和native开发观察独立保留，不混入r3计数。客户端日期2026-10-17（Asia/Shanghai），原始UTC ISO时间保持原值。

## Executed Phase

Phase19：迁移 Rust 工单成员目录与分配校验。依赖Phase14、17、18已完成。本次实现assignees搜索分页、单成员验证、创建/编辑分配和平台内部窄成员用途声明；不直接查询平台成员表、不因处理人关系扩大SELF、不构造无第二消费者的通用框架。本次只完成Phase19，下一建议Phase20 / `NOT_STARTED`，不提前实施提醒、前端提交恢复、门店消费者、生产切流或部署。

## Execution Summary

Rust `GET /work-orders-api/assignees`保留既有搜索/分页和最小投影；创建/实际更换处理人使用当前普通用户应用Bearer调用真实平台members HTTP。服务继续逐请求confidential introspection/context，分别检查create/update功能权限、独立data grant和组织硬上限；目标必须是启用的有效直接成员，名称快照来自平台响应，不接受调用方覆盖。SELF始终按owner及组织上限，不因处理人关系取得额外读取、编辑、删除或导出权限。

平台在每个application的私有`DATA_GRANT_POLICY`增加可选`memberLookups:[{resource,action}]`，省略默认空；pair必须登记于同应用capabilities。members路由按登记用途校验，去掉散布的ticket特判；工单明确登记ticket/create、ticket/update。公开capability仍为resource/action/scopes三字段，不增加公开assign操作或成员grant，私有策略不进入公开context wire。本阶段只有工单消费者和平台窄策略，不提前宣称门店或公共跨应用能力已完成。

更新先用完整update scope非锁预读组织/version/oldAssignee，释放连接后在事务外请求成员接口；随后BEGIN/FOR UPDATE，复核组织、版本和oldAssignee，否则409 `VERSION_CONFLICT`。新成员结果是不可由写入调用方伪造的内部断言，绑定actor/tenant/current-org/action/target-org/user及可信名称。单调TTL从成员HTTP开始，取`min(PLATFORM_AUTH_TIMEOUT_MS,3000ms)`，响应读取与之后的pool/行锁等待都消耗同一窗口，不能在响应或取锁时续期。外部HTTP超时503 `PLATFORM_UNAVAILABLE`；已取得断言在提交前过期503 `MEMBER_VALIDATION_EXPIRED`并回滚；提交后不确定沿用CREATE_RESULT_UNKNOWN/WRITE_RESULT_UNKNOWN503及原键恢复/读回版本指导。

省略assignee或显式提交同一UUID保留历史分配/名称快照，不重查旧成员，也不额外要求目录scope。真正clear仍须目标组织范围，不请求新成员；真正非空change才验证直接成员。创建继续使用Phase18五元幂等域与当前create授权；同key/同hash已提交receipt恢复不重查历史member，禁用、移除、改名或目录暂不可达不会把历史恢复重新解释为新分配。原201不证明当前detail或成员仍有效。

平台库和业务库不共享原子事务。短TTL、版本和锁内身份复核限定陈旧结果窗口，不能保证最后一次平台校验与业务COMMIT之间绝无撤权或成员失效，也不自动清除已有分配。

## Deliverables

- [目录用例与内部成员断言](../../business-apps/work-orders/backend/src/directory.rs)、[query解析](../../business-apps/work-orders/backend/src/directory/query.rs)、[目录单元回归](../../business-apps/work-orders/backend/src/directory/tests.rs)。
- [有界成员客户端](../../business-apps/work-orders/backend/src/authorization/client.rs)、[授权编排](../../business-apps/work-orders/backend/src/authorization.rs)、[非锁预读及事务内复核](../../business-apps/work-orders/backend/src/writes.rs)、[兼容DTO](../../business-apps/work-orders/backend/src/writes/dto.rs)及[HTTP接线](../../business-apps/work-orders/backend/src/http.rs)。
- [平台私有成员用途策略](../../backend/src/data_grants/policy.rs)、[真实members路由](../../backend/src/routes/app_members.rs)，以及[配置说明](../data-grant-management.md#member-lookups)。
- [controlled HTTP + owned业务PG成员12项](../../business-apps/work-orders/backend/tests/member_contracts.rs)、[真正平台联合harness](../../backend/src/phase19_work_orders_tests.rs)和[精确live结果解析](../../scripts/ci/work-orders-member-live-results.mjs)。旧live17/live18保持独立，7项演进fixture与旧Node输入在边界中逐项记录。
- [操作、竞态、采用与回退说明](../work-orders-rust-baseline.md)、相关架构/README及公共模板同步；[正式输入](phase-19-inputs.json)、[正式边界](phase-19-boundaries.json)、[正式运行索引](phase-19-runtime.json)、本报告与[台账更新建议](phase-19-ledger-update.md)。运行索引关联原始证据，本报告不将managed的268份数量冒充运行索引自身的最终产物数量。

## Self-Check

三条台账验收原文保持，以下结论全部采用最终r3的实际结果。

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 搜索/分页保持既有契约；禁用/越界/无成员或缺分配授权不能作为处理人。 | PASS | member12真实业务PG+受控HTTP覆盖参数/响应校验、直接成员、禁用/间接/越界、grant和传输错误；live19真实密码/app-token/members验证两页集合、字面搜索、有效分配及正式禁用/移除/撤create或update grant。平台76含新增成员策略2项，policy8专项通过。 |
| 成员接口超时不拉长同工单行锁；记录预读/锁定组织不匹配拒绝。 | PASS | member12 gate证明业务唯一pool连接idle1、同工单NOWAIT可锁；HTTPtimeout后记录不变。500ms断言预算先被HTTP消耗200ms、context仍28秒有效，锁等待后断言过期回滚，未在锁内重查/续期。组织预读故障拒绝且guard恢复；live19再验证唯一pool、NOWAIT、3045ms外部timeout及组织/assignee变异409。 |
| SELF仍按owner及组织上限执行，分配关系不带来额外数据访问。 | PASS | member12 scope矩阵和live19双向分配后均验证assignee不能读取/编辑/删除/导出其他owner的SELF工单。当前SELF可读本人子组织旧记录，但同值仅保留历史快照、不查目录；实际clear/change仍受目标组织scope，不能借处理人关系扩权。 |

## Contracts and Race Boundaries

| Boundary | Implemented and verified behavior |
| --- | --- |
| 成员用途登记 | 每app可选memberLookups、缺省空；最多128个唯一pair，均须登记于同app capabilities。未声明resource/action拒绝，数据能力不是隐含的目录授权。 |
| 公开协议 | capability wire仍为resource/action/scopes；平台members的id/displayName/organizationId和业务投影userId/displayName/organizationId保持。不增加公开assign grant、业务环境变量或依赖。 |
| 目录参数 | action=create/update必需，organizationId默认当前组织；page默认1/上限1000000、size默认20/上限50；q原文先限100Unicode字符再trim、query最多8192bytes；未知/重复/非法UTF8/NUL/percent输入拒绝。搜索中的%和_保持字面含义。 |
| 候选和组织 | 有效启用直接成员，祖先成员不当作当前组织直接成员。应用/租户/组织有效；SELF/ORG目录仅当前组织，subtree/TENANT继续叠加组织硬上限。 |
| 更新四态 | 省略/同UUID保留锁定记录的旧分配和名称快照；null原本即null不产生分配变更；真正clear要求组织scope但不查新成员；真正非空change另外请求平台验证。 |
| 预读/锁定 | 预读使用完整update scope，不借read权限或assignee扩权；释放连接后HTTP，锁内再核对组织/version/oldAssignee，不匹配409。正常接口不允许修改归属。 |
| TTL | 成员请求开始时单调计时min(platform timeout,3秒)；响应读取、pool和行锁等待均计入，不得重新起算或在锁内补HTTP。提交前过期503回滚，提交后不确定用原结果恢复语义。 |
| 错误 | 单成员上游404→business404 MEMBER_NOT_FOUND；列表上游404→503 PLATFORM_UNAVAILABLE；权限/grant403→403 DATA_DENIED；成员用途400→400 VALIDATION_ERROR；外部HTTPtimeout503。解析/边界异常不回显secret。 |
| receipt | 当前create授权先于恢复；已有receipt不重查历史member；仅新创建执行有效直接成员校验。规范UUID进入正文摘要，名称/验证时间/TTL不进入幂等身份；省略/null旧摘要保持。 |
| 跨库竞态 | 平台与业务库没有原子提交；TTL和本地复核只能约束窗口，不能保证撤权与COMMIT全序，不自动抹掉历史分配。 |
| 总deadline | 保留Phase18的15秒请求总期限与独立DB acquire/statement/lock预算，不自动重试PATCH/DELETE；外部HTTP不占业务连接/事务/行锁。 |

## Adopted r3 Evidence

最终r3为700files、267条AST resolved edges、33项显式required closure输入，SHA-256 `52ebb7a5904b82fa94f5f16b69e9cad924b8afcea4ae60df75fa1423a101741e`，捕获时间`2026-10-17T04:04:01.019Z`。PG、fast、raw均以此来源独立完整执行并终态PASS/exit0，冻结执行源前后不变、hostDrift=[]；不借retained r2填补最终结果。

| Layer | Adopted actual result | Evidence |
| --- | --- | --- |
| 迁移PG | 21 PASS/0 FAIL/0 skip，0001/0002和历史接管回归保持。 | [migration21](../../target/phase-19/linux-work-orders-r3/reports/postgres/work-orders-postgres/postgres-result.json)。 |
| 受控读取/写入PG | 原read13、write14全部通过，零失败/跳过。 | [read13](../../target/phase-19/linux-work-orders-r3/reports/postgres/work-orders-read-postgres/postgres-result.json)、[write14](../../target/phase-19/linux-work-orders-r3/reports/postgres/work-orders-write-postgres/postgres-result.json)。 |
| 受控成员PG | member12全部通过，每case一个owned业务库、精确受控平台HTTP；SQL真实执行，不能称真平台成功。 | [member12](../../target/phase-19/linux-work-orders-r3/reports/postgres/work-orders-member-postgres/postgres-result.json)及原log的TTL/pool/锁/组织故障标记。 |
| 原live17 | 独立1个Rust test、13个named checks通过。 | [live17](../../target/phase-19/linux-work-orders-r3/reports/postgres/work-orders-live-postgres/postgres-result.json)。 |
| 原live18 | 独立1个Rust test、12个named checks通过，非法UUID负向替代Phase19前的全拒绝预期。 | [live18](../../target/phase-19/linux-work-orders-r3/reports/postgres/work-orders-write-live-postgres/postgres-result.json)。 |
| 新live19 | 独立1个Rust test、10个named checks通过，真实平台/业务双库+独立Rust子进程。 | [live19](../../target/phase-19/linux-work-orders-r3/reports/postgres/work-orders-member-live-postgres/postgres-result.json)。 |
| 平台回归 | platform76全部通过：原74+新增成员策略2，零失败/跳过。 | [platform76](../../target/phase-19/linux-work-orders-r3/reports/postgres/platform-postgres/postgres-result.json)。 |
| scopedPG整体 | outer/child PASS/exit0，16/16步骤；完整工单lib50、doctest1、rustdoc与ELF build通过。 | [PG summary](../../target/phase-19/linux-work-orders-r3/reports/postgres/summary.json)、[PG outer](../../target/phase-19/linux-work-orders-r3/linux-runner-result.json)。 |
| 完整共享fast | outer/child PASS/exit0，53/53步骤，765 PASS/0 FAIL/0 skip；contracts355、CIpolicy93和工单fmt/check/all-target/all-feature strictClippy/lib50/doc通过。765已经包含lib50，不重复加总。 | [fast summary](../../target/phase-19/linux-fast-r3/reports/fast/summary.json)、[fast outer](../../target/phase-19/linux-fast-r3/linux-runner-result.json)。 |
| 原ELF raw | 12/12checks通过，sourceAndBinaryUnchanged=true、hostDrift=[]、unexpectedLeaseExit=false。 | [supplement](../../target/phase-19/linux-runtime-r3/supplement-result.json)、[raw result](../../target/phase-19/linux-runtime-r3/raw-runtime-result.json)。 |

本轮是Phase19/工单scope及所选platform76的PostgreSQL验证，不重复其他聊天负责的notification-live，`fullPostgresGateExecuted=false`，不能称全部PostgreSQL套件通过。实际Rust test数与named checks分别记录，不把10/12/13个checks当成额外Rust测试重复累计。

## Real Platform, Fixtures, and Runtime Limits

live19使用真实密码login/context/app-token、confidential introspection、明确memberLookups及正式root grant preview/PATCH；bootstrap用户/直接成员/角色/client走真实管理API，没有SQL伪造session/grant。成员middleware gate只延迟真实handler，不构造成功响应。目录候选禁用/移除和调用方grant变化均经正式API发生。

预读/锁定身份竞态只在owned业务库故障事务中临时停用immutable trigger、注入、恢复trigger后COMMIT，gate释放前guard已恢复；这不是正常业务可修改组织/归属的产品能力，旧0001未变。controlled member12每case是单业务库+HTTP fixture，live才是真平台/业务双库+子进程，二者证据不混称。

同源r3原ELF为67,361,248bytes，SHA-256 `3ae2bbb630a9b79a4b0fa469d4502b2f37b2dc47ba70db48f370a08f483f3a5c`。raw真实运行UID10001，无Node、hostports或workspace mounts；rootfs可写，通过docker cp传入原ELF，没有交付工单生产应用镜像，也不宣称只读根文件系统。raw故意让平台不可达，证明健康/PGdown恢复、增量历史/receipt/data/counter保持、启动锁/期限、SIGTERM及读写fail-closed；成功真实成员/分配由live19证明。联合transport是显式development loopback HTTP，不是生产TLS或切流验收。

## Source, Boundary, Dependencies, and Cleanup

[最终边界](phase-19-boundaries.json)为BOUNDARIES_PASS，18项检查通过，56owned、16旧Node兼容输入、7演进fixtures、7文档+1公共模板；当次host drift及采集期间host change均0。33为闭包显式required项，不与requiredSharedInputs或owned集合混用。冻结700选择集与执行前后核验一致；这些检查不构成未选择工作区文件的完整外部审计。

Schema、Cargo manifest/lock与253第三方依赖身份保持，无新增/移除；旧0001 SHA `a2e4224febd195fdd663a8954c3be4b76a8a06338521056c6809ebaa57d04090`、0002 SHA `70700874ffd778b9c9c6921614fe54efcad1512b57b5a1fd8373ad56d8f4541c`保持。依赖结论继承相同锁文件与原库存，不声称新的漏洞/license/法律审计。

managed为26PASS、绑定268原始证据；audit SHA `57d2ddf9163b56940ef7da07d32988b869f0f75cd60b85e8b367f7f6bc83f2cc`，artifact-list SHA `6ad70c16056756f0744ba241ba6d94f7137876318f236daafc7bce7833150c34`。正式boundary SHA `d24e86c7c8bf568a2d6c80e14c92d5ce763043f051207c9b07b348a53fa1e520`。正式运行索引关联这些来源，不虚构其自身最终artifacts数量。

| Resource | Actual verification |
| --- | --- |
| controlled迁移/read/write/member/platform数据库与pool | suite原日志清理成功；member12逐case assertions_ok/database_removed/pool_closed=true。 |
| live17/live18/live19双库/pool/业务进程 | 各套独立cleanup通过；live19的MEMBER_LIVE_BUSINESS/PLATFORM三个布尔全true，PROCESS stopped=true。 |
| r3 PG/fast/raw owned容器及网络 | 三次各4条正确ownerlabel查询exit0、输出空、PASS；cleanupErrors=[]、lease停止exit0。 |
| raw业务库/存储/退出 | runtime库删除，remainingContainers/remainingNetworks空，owned tmpfs随容器删除；unexpectedLeaseExit=false，正常SIGTERM通过。 |
| 来源与产物 | 三组输入SHA均52ebb…1741e、执行源不变、hostDrift=[]；raw源/ELF不变。driver、原报告和查询argv各自归档。 |

## Retained Earlier Runs and Development Observations

以下原始结果保留独立来源，不充作r3采用计数，也不被本次完成状态改写。

| Earlier source | Preserved result and meaning |
| --- | --- |
| r1受控取消 | [dispatch](../../target/phase-19/dispatch-cancellation-r1.json)与[原outer](../../target/phase-19/linux-work-orders-r1/linux-runner-result.json)保留FAIL/143及app_members hostguard失败。原因是修复本阶段新strict lint后要求新不可变候选；仅停止核实owner容器，四资源空查PASS、cleanupErrors空、lease0。managed的originalOuterFailures仅此取消轮次。 |
| retained r2 PG | de1bf0d0e2c108f36319bc43ed611faeb304a72d7673a0ef0007cb680239ff93，outer/child PASS/exit0、16/16，76/21/13/14/12/三live/lib50/doc1等实际通过。保存[原PG结果](../../target/phase-19/linux-work-orders-r2/linux-runner-result.json)，不代替r3重验。 |
| retained r2 fast | [原outer](../../target/phase-19/linux-fast-r2/linux-runner-result.json)及[child](../../target/phase-19/linux-fast-r2/reports/fast/summary.json)均FAIL；53steps=52PASS/1FAIL、764PASS/1FAIL/0skip，唯一ci-policy93=92/1。tests/ci-runner.mjs把document.passed写死74，而新manifest合计76，mock预期与evaluate不符。实际PG业务不是该失败原因，但完整fast失败原样保留。 |
| retained r2 runtime | [原supplement](../../target/phase-19/linux-runtime-r2/supplement-result.json)12/12PASS、原ELF67,361,248bytes/3ae2…3a5c；r2三组源未变、hostDrift=[]、cleanupErrors=[]、各4空查PASS/lease0，raw unexpectedLeaseExit=false。原ELF哈希与r3相同不表示来源相同，两个候选分别构建/运行取证。 |

retainedRuns恰为r2 PG PASS、fast FAIL、runtime PASS三份。将聚合mock改为manifest动态sum后，完整native readiness-r4实际93/93通过、0fail/0skip，原argv及700输入预像前后同hash；随后重新冻结r3并完整重跑PG、fast、raw。没有把r2 PG/raw与r3 fast拼成一次成功。r2 driver的PHASE18纯stdout前缀是继承标签，实际结构化scope/输入/named suites属于Phase19；运行期间不改driver字节，不当作历史Phase18证据。

[native开发观察](../../target/phase-19/native-development-audit.json)为MIXED_DEVELOPMENT_OBSERVATIONS，13项收集检查、29份原证据摘要；忠实收集通过不等于产品门禁全部通过。Windows工单lib实际50项48PASS/2HTTP FAIL/0ignored，目录6与新增DTO1包含在48中；两个HTTP失败原log保留，网络原因没有被证实，不用Linux通过回写Windows为全通过。平台policy8专项通过，工单fmt/strict通过。早期定向readiness20/0、26/1、28/0分别保留，不与完整93项混计。

当前宿主平台strict首轮54errors含本阶段items_after_test_module新增1项，仅移test mod至末尾修复；[r2归属报告](../../target/phase-19/platform-clippy-r2.json)本轮53errors、8owned文件error0，两轮json/jsonl/stderr保留。最终工单crate strict通过不代表整个平台strict通过，也不声称所有编写/取消/候选尝试从未失败。

## Adoption, Rollback, and Scope Limits

采用时先为同应用合并明确ticket/create和ticket/update的memberLookups，保留既有能力与管理范围并按启动加载规则同步副本；核对普通用户功能权限、data grant和目标组织。接口可用不代表当前前端创建header/未知结果恢复流程已适配，提醒仍属于Phase20，未切生产流量。

回退原Phase18恢复显式非null assignee400、目录503。历史分配/receipt不删除，省略更新仍可保留快照；但带非空assignee的创建正文不能由原Phase18重放，不能宣称简单二进制回退仍保持新receipt恢复。先核对在途结果，必要时使用保留新成员/receipt契约的兼容构建，不删receipt、不改schema/checksum绕过。平台若回退到不认识memberLookups的旧严格解析器，需先移除新字段并同步旧配置，旧成员访问边界另审，不把删字段视为保持新的默认拒绝。

本阶段无新增业务开放问题。完成仅适用于Phase19工单消费者、平台窄策略及本轮真实范围；其他聊天已采纳结果和修改保留，Phase26/27/30的其他消费者/公共收敛、提醒/前端恢复/生产部署不提前认领。

## Suggested Task Ledger Update

采用[结构化更新](phase-19-ledger-update.md)：Phase19 `COMPLETED`，三条checkbox全部完成，本阶段无剩余项；下一建议Phase20“迁移 Rust 工单提醒到平台通知API”/`NOT_STARTED`。同步摘要表、阶段详情、Notes及按本轮用户授权调整的执行指针，保留其他聊天阶段结果，不自动执行下一阶段。正式运行结论采用PHASE19_GATES_PASS，retained与native混合观察继续按原状态归档。
