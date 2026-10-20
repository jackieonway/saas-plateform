# Suggested Task Ledger Update — Phase 19

客户端日期2026-10-17（Asia/Shanghai），UTC原始时间保持。采用Phase19 `COMPLETED`：三条验收及范围内必需门禁通过，[managed审计](../../target/phase-19/managed-audit.json)为PHASE19_GATES_PASS、26检查全部PASS、绑定268原始证据摘要。采用r3单一来源，PG16/16、fast53/53且765/0/0、raw12/12全部PASS；r1取消、r2三份终态与native混合观察独立保留，不改原结果。本文件为结构化建议，不直接覆盖台账。

## Phase Status Update

Phase19：`COMPLETED`，三项checkbox全部完成；下一建议Phase20“迁移 Rust 工单提醒到平台通知API”/`NOT_STARTED`。同步摘要表、阶段详情、Notes及按本轮用户授权调整的执行指针，保留其他聊天已采纳状态与修改，不自动实施下一阶段。

## Output Summary

- Rust assignees搜索分页、单成员验证、创建/实际更换分配完成；普通应用Bearer调用平台HTTP，不直接查平台成员表，名称来自平台可信响应。
- 每app私有memberLookups缺省空、pair必须匹配同app capabilities，公开capability resource/action/scopes wire不变；未声明用途拒绝，无新公开assign grant，无通用公共框架。
- scoped非锁预读取组织/version/oldAssignee后释放连接，成员HTTP在pool/事务/行锁前；FOR UPDATE后复核，不一致409 VERSION_CONFLICT。
- 成员断言绑定actor/tenant/current-org/action/target-org/user和名称，单调TTL从成员请求开始取min(platform timeout,3s)，响应及pool/锁等待均消耗窗口，不续期。HTTPtimeout503 PLATFORM_UNAVAILABLE，提交前断言过期503 MEMBER_VALIDATION_EXPIRED回滚，提交后未知结果延续原key恢复/读回版本。
- 同UUID与省略保留历史分配/名称，不查旧成员、不扩目录scope；真正clear要求目标组织范围，真正非空change额外验证有效直接成员。SELF始终owner+组织上限，assignee不带来额外数据访问。
- 已提交创建receipt恢复仍要求当前create授权，但不查历史member；原201不代表当前detail或成员仍有效。平台与业务库没有跨库原子事务，短TTL/版本限定窗口，不保证撤销与COMMIT全序。

## Completed Items

- 最终r3冻结700files/267 AST edges/33显式required closure输入，SHA-256 `52ebb7a5904b82fa94f5f16b69e9cad924b8afcea4ae60df75fa1423a101741e`，capture `2026-10-17T04:04:01.019Z`；PG/fast/raw均独立完整执行，源前后不变、hostDrift=[]、exit0。
- scopedPG outer/child PASS、16/16步骤：platform76（原74+成员策略2）、migration21、read13、write14、member12全部零失败/跳过；live17/live18/live19各1个Rust test，named分别13/12/10；完整工单lib50、doctest1、rustdoc、ELF build通过。
- 完整共享fast outer/child PASS、53/53步骤、765PASS/0FAIL/0skip；contracts355、CIpolicy93与工单fmt/check/alltarget/allfeature strictClippy/lib50/doc通过。765已包含lib50，不重复加总；scopedPG不是全部PostgreSQL套件，不重复并行notification-live。
- member12每case为单owned业务库+controlled平台HTTP，覆盖目录/shape/权限scope、启用直接成员/禁用/间接/无成员、创建receipt和四态更新、SELF、HTTP错误/超时及竞态。清理逐case assertions_ok/database_removed/pool_closed=true。
- gate实证业务唯一连接idle1、同工单NOWAIT可锁；测试500ms成员窗口被HTTP消耗200ms后锁等待过期、context仍28s有效，回滚证明TTL从请求开始且不重置。组织预读变异拒绝、version不变、assignment不写入、immutable trigger重新启用。
- live19以真实密码login/context/app-token/confidential introspection、明确memberLookups、正式root preview/PATCH与用户/成员管理API完成10named；两库/pool/独立业务进程清理全true。middleware只延迟真实handler，3045ms成员timeout无部分写入；组织/assignee故障变异409。
- 双向分配证明SELF仍按owner，不能因assignee读取/编辑/删除/导出他人工单；本人child旧记录同UUID保留不查member，真正clear/change仍受目录scope。禁用后的同值历史编辑、成员移除后的receipt恢复不重查成员；撤销当前create grant后原key拒绝。
- 同r3原ELF67,361,248bytes/SHA-256 `3ae2bbb630a9b79a4b0fa469d4502b2f37b2dc47ba70db48f370a08f483f3a5c`，raw12/12PASS：UID10001、无Node/hostports/workspace mounts、rootfs可写。健康/PGdown恢复、双版本历史/receipt/data/counter、启动期限/锁释放重试、SIGTERM及读写fail-closed通过；平台故意不可达，真实成功分配由live19证明，不交付生产镜像或只读rootfs。
- r3 PG/fast/raw三组各4正确ownerlabel资源查询exit0/空/PASS，cleanupErrors=[]、lease停止exit0，raw unexpectedLeaseExit=false、sourceAndBinaryUnchanged=true；冻结源、driver、原ELF和资源argv分别归档。
- fresh正式boundary为BOUNDARIES_PASS、18检查，56owned/16旧Node/7演进fixtures/7文档+1公共模板/253继承依赖；host drift及采集期间host change均0。原schema、Cargo manifest/lock和0001/0002字节保持，无新增依赖或crate，不声明新漏洞/license审计。
- managed PHASE19_GATES_PASS、26PASS/268原证据；audit SHA `57d2ddf9163b56940ef7da07d32988b869f0f75cd60b85e8b367f7f6bc83f2cc`，artifact-list SHA `6ad70c16056756f0744ba241ba6d94f7137876318f236daafc7bce7833150c34`；正式boundary SHA `d24e86c7c8bf568a2d6c80e14c92d5ce763043f051207c9b07b348a53fa1e520`。268仅为managed绑定证据数，正式运行索引以其自身产物清单为准。
- r1受控取消FAIL/143及app_members hostguard、dispatch和四空查/cleanup/lease原证据保留；originalOuterFailures只此取消轮次。retainedRuns恰r2 PG PASS、fast FAIL、runtime PASS三份，不填补r3采用计数。
- retained r2源700/de1bf0d0e2c108f36319bc43ed611faeb304a72d7673a0ef0007cb680239ff93：PG16通过、raw12通过，完整fast child/outer FAIL、53steps52/1、764/1/0，唯一ci-policy93=92/1。mock写死74对manifest实际76，改为动态sum后native完整readiness-r4 93/93通过、700预像稳定，再冻结r3重跑全部门禁；没有拼接r2 PG/raw与r3 fast。r2三组资源/来源亦各自清理稳定，原结果不改写。
- native-development-audit为MIXED，13收集检查/29证据摘要。Windows工单50=48PASS/2HTTP FAIL，directory6/新DTO1为48中的子集；网络原因未证实。平台policy8、工单fmt/strict专项通过，早期定向readiness20/0、26/1、28/0分别保留。
- 当前host平台strict首轮54errors含新增items_after_test_module1，仅移动test mod末尾修复；次轮53errors、8owned文件0errors，两轮json/log保留。工单crate严格通过不称整个平台strict通过，不把所有尝试描述为从未失败。
- r2继承的PHASE18控制台前缀仅stdout标签，结构化scope/输入/suites属Phase19且driver运行期间不改；不冒充历史Phase18证据。所有来源、限制及原报告由[正式运行索引](phase-19-runtime.json)、[报告](phase-19.md)、[输入](phase-19-inputs.json)、[边界](phase-19-boundaries.json)关联。

## Remaining Items

Phase19范围内剩余：**无**。三项验收及范围内必需门禁全部通过。已披露r1取消、retained r2 fast失败、native HTTP失败、整个平台strict债务与跨库竞态边界；这些原始状态不由阶段完成结论改写。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 搜索/分页保持既有契约；禁用/越界/无成员或缺分配授权不能作为处理人。 | PASS | r3 member12/live19真实分页/搜索/投影、有效直接成员、禁用/移除/间接/跨组织及正式撤grant通过，platform76新增成员策略回归和policy8支持私有声明边界。 |
| 成员接口超时不拉长同工单行锁；记录预读/锁定组织不匹配拒绝。 | PASS | r3 gate/唯一pool/NOWAIT、HTTPtimeout不变、500ms请求起算断言被HTTP与锁等待消耗后回滚、组织及assignee预读变异409通过；故障事务恢复immutable guard。 |
| SELF仍按owner及组织上限执行，分配关系不带来额外数据访问。 | PASS | r3双向分配后对他人owner的列表/detail/PATCH/DELETE/CSV仍不可见；本人child旧记录同值快照保留不查成员，实际clear/change仍受组织目录scope限制。 |

## New Decisions

| Decision | Reason | Impact |
| --- | --- | --- |
| 私有memberLookups显式登记/默认空/绑定同app capability | 成员辅助不是任意data grant的隐含权力，平台无需散布ticket特判 | 公开wire保持，无新assign grant；本阶段仅工单消费者/窄策略。 |
| 省略/同UUID保留历史快照，实际change/clear另验组织 | 历史成员变化不能阻断无关编辑，也不能用兼容请求扩SELF目录 | 同值不查member；实际非空change验证有效直接成员，clear无需新成员HTTP。 |
| 已提交receipt优先恢复并要求当前create授权 | 历史恢复不是新分配，成员变化不应破坏恢复，当前失权仍必须拒绝 | 原201不代表当前detail，摘要不包含名称/验证时间/TTL。 |
| scoped非锁预读→释放连接→HTTP→锁内组织/version/assignee复核 | 慢平台请求不能占业务连接/行锁，锁前计划不能覆盖不同记录身份 | mismatch409，test故障注入只在owned库且guard恢复后释放gate。 |
| 单调TTL从成员请求开始取min(platform timeout,3s) | 响应和后续等待都使结果陈旧，不能取锁时重新计时 | 过期回滚503，提交后未知结果按键/版本恢复；不承诺跨库原子性。 |
| 原Phase18回退不保证新非空正文receipt恢复 | 旧DTO先拒绝非null assignee | 核对在途结果，必要时兼容构建；不删receipt、不改schema/checksum绕过。 |
| r3全门禁同源重验，r1/r2/native分别保留 | 聚合mock失败必须修复并完整重验，不能跨来源拼绿 | 采用700/52ebb…1741e的PG16/fast53/765/0/0/raw12；retained状态按原样保存。 |
| 工单、平台strict与实际PG范围分开 | 局部门禁通过不等于整个平台严格通过或全部PostgreSQL执行 | 当前平台53errors、未选notification-live等限制明确保留。 |

## New Open Questions

无新增业务开放问题。平台与业务库跨库竞态边界、同值历史和receipt恢复语义已明确；成员其他消费者/公共收敛、提醒、前端恢复、生产部署按后续阶段处理，不因本阶段完成外推。

## Next Recommended Phase

Phase20：迁移 Rust 工单提醒到平台通知API，`NOT_STARTED`。本次只完成Phase19并按用户授权同步台账，保留其他聊天阶段结果与修改，不自动实施下一阶段。

## Ledger Evidence Note

2026-10-17完成Phase19。Rust目录/分配使用普通应用Bearer、私有memberLookups默认空且同app capability绑定，公开wire不变；有效直接成员/组织/权限/grant独立校验，SELF始终owner。scoped非锁预读后释放连接，member HTTP后锁内组织/version/assignee复核，断言从请求开始min(platform timeout,3s)，跨库不原子。同UUID/省略保留历史，receipt原key恢复不重查member但仍需当前create授权；原Phase18不能重放新非空创建正文，回退需兼容方案。最终r3 700files/267AST/33required/SHA52ebb7a5904b82fa94f5f16b69e9cad924b8afcea4ae60df75fa1423a101741e，PG outer/child16/16：platform76/migration21/read13/write14/member12/live17 1+13/live18 1+12/live19 1+10、lib50/doc1/rustdoc/build全部PASS；完整fast outer/child53/53、765/0/0（含lib50）、contracts355/CIpolicy93；同原ELF67361248bytes/3ae2bbb630a9b79a4b0fa469d4502b2f37b2dc47ba70db48f370a08f483f3a5c的raw12通过，UID10001/无Node端口挂载/rootfs可写。三组源/ELF稳定、hostDrift[]、cleanupErrors[]、各4空查PASS/lease0。managed26PASS绑定268证据，boundary18PASS/56owned/16旧Node/7fixture/7md+1template/253继承依赖。r1取消143/hostguard、retained r2 PG PASS/fast FAIL(53步52/1、764/1/0、ci-policy92/1)/raw PASS、native48/2与strict54→53/owned0等各保留；mock动态sum后native93/93再r3同源全重验，不拼绿、不称所有尝试无失败。结论PHASE19_GATES_PASS，三checkbox完成；不称全平台strict/全PG/生产镜像，只完成19，下一20 NOT_STARTED。