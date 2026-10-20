# Phase Execution Result — Phase 18

状态：`COMPLETED`。四条台账验收及 Phase18 范围内必需门禁通过，证据采用 [正式运行索引](phase-18-runtime.json) 的 `PHASE18_GATES_PASS` 语义。相同 691 文件候选的 scoped PostgreSQL 14 步全部通过，完整 fast 子级 52/52 步通过、745 PASS/0 FAIL/0 skip；两个外层 runner 因资源查询 filter 写法错误仍保留 `FAIL`，独立更正查询确认资源已清理，不能拼称一次整体执行从未失败。客户端日期 2026-10-17（Asia/Shanghai），原始 UTC ISO 时间保持原值。

## Executed Phase

Phase18：实现 Rust 工单写入、幂等和短事务。依赖 Phase6、13、17 已完成。本次交付严格兼容 DTO、创建幂等、编辑/状态/软删除、版本竞争、不可变归属和请求总 deadline；沿用 Phase17 每请求平台授权。本次只完成 Phase18，下一阶段建议为 Phase19 / `NOT_STARTED`，不提前实施成员目录、提醒或生产切流。其他聊天已采纳结果及并行修改分别保留。

## Execution Summary

创建通过 `Idempotency-Key` 请求头使用 tenant/org/user/create/key 五元唯一域；对验证后的 title trim、description 缺省空、priority 缺省 NORMAL、assignee 省略/null 归一的固定字段序列计算 SHA-256。工单、编号计数器与原始 201 receipt 在同一事务中提交；同键同规范正文重放原始 201，异正文返回 409。重放前必须通过当前 create 功能/数据授权；历史 receipt 不代表当前 detail 或当前未删除状态。

PATCH/DELETE 必须提供 version；锁内校验当前版本、组织/范围及授权期限。身份、归属、编号、显示快照与时间字段不能由客户端覆盖；状态流保留既有转换，删除采用软删除。外部平台授权在获取业务 pool 连接和事务之前完成；同一事务不等待平台 HTTP。HTTP 总期限为 15 秒，DB acquire/statement/lock 各有独立预算；COMMIT、提交后 expiry 或请求中止出现不确定结果时，创建提示以原 key/正文恢复，PATCH/DELETE 提示读回版本核验，不自动重试。

Phase19 真实成员校验接入前，所有显式非 null assignee 返回 400；编辑省略保留原值，null 清除仍受本地组织 scope 限制。运行路径没有允许全部成员的替身，也没有行锁内的成员 HTTP 请求。

同 [691 项正式输入](phase-18-inputs.json)、SHA-256 `9bea11c1855bdea5126fec3503e6b22d69e87a0d00a31f5f86b4187e8825ffec` 的 Linux scoped PG、完整 fast、原 ELF runtime 补验均有实际结果。原平台 74、迁移 21、受控 read 13、受控 write 14、原 live17 1 个 Rust test/13 个 named checks、新 live18 1 个 Rust test/12 个 named checks 全部零失败/跳过；完整工单 lib43、doctest1、rustdoc 与 ELF build 通过。lib43 已包含在 fast 的 745 中，不重复累计成更大的测试总数。

## Deliverables

- [Rust 写入用例](../../business-apps/work-orders/backend/src/writes.rs)、[严格 DTO 与规范摘要](../../business-apps/work-orders/backend/src/writes/dto.rs)、[HTTP/CORS/总期限](../../business-apps/work-orders/backend/src/http.rs) 及 [五 action 授权 policy](../../business-apps/work-orders/backend/src/authorization/policy.rs)。
- [新增 0002 幂等迁移](../../business-apps/work-orders/backend/migrations/0002_create_idempotency.sql)、[迁移目录](../../business-apps/work-orders/backend/src/migrations/catalog.rs) 及 [21 项迁移回归](../../business-apps/work-orders/backend/tests/migrations.rs)；旧 0001 原字节保留。
- [14 项真实业务 PG + 受控 HTTP 写入回归](../../business-apps/work-orders/backend/tests/write_contracts.rs)，以及 [真实平台联合 harness](../../backend/src/phase18_work_orders_tests.rs)。原 [Phase17 live harness](../../backend/src/phase17_work_orders_tests.rs) 和 read13 保持独立回归。
- [Rust 工单操作说明](../work-orders-rust-baseline.md)、组件/根 README、架构及工程基线同步，真实成员分配与提醒仍归后续阶段。
- [正式输入](phase-18-inputs.json)、[边界报告](phase-18-boundaries.json)、[运行索引](phase-18-runtime.json)、本报告与 [台账更新建议](phase-18-ledger-update.md)。[最终 managed 审计](../../target/phase-18/managed-audit.json)状态为`PHASE18_GATES_PASS`，27项检查全部PASS，绑定116份原始证据摘要。

## Self-Check

以下四条验收原文保持，逐项以本轮真实执行证据判定。

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 同键同正文并发只一条记录，commit后丢响应按原键恢复；异正文409。 | PASS | write14 以 barrier 启动 8 并发，验证一条工单/一条 receipt/编号仅增长一次、规范正文重放及异正文409；专用 HTTP proxy 完整取得上游201后断下游，再以原键恢复。live18 再真实验证8并发、规范化重放、取得201 headers后丢弃body的恢复；receipt写入故障同时回滚工单和结果。 |
| 新组织/已失权重试不能泄漏旧结果；版本竞争仅一方成功，删除后列表详情导出不可见。 | PASS | write14 与 live18 覆盖五元域、跨组织/撤权重试、PATCH与DELETE版本竞争、合法/非法状态转换和删除后列表/detail/CSV不可见；live18用正式root preview/PATCH撤销create grant，旧key不能返回旧结果。 |
| DB锁/连接不随外部HTTP长等；总deadline与语句/锁预算可验证。 | PASS | write14 gate阻塞context时业务pool唯一连接仍idle且可获取，行锁`FOR UPDATE NOWAIT`成功；acquire/statement/lock超时分别验证回滚。15秒总期限实测15008ms，`late_commit=false`，连接可再取、原key重试只创建一次；等待中授权过期回滚通过。 |
| 未接入Phase19真实成员校验时，非空assignee明确拒绝；无分配/错误成员不会借测试替身绕过授权。 | PASS | DTO/授权纯测试、write14与live18真实执行显式非null assignee返回400；受控目录计数验证零调用，未配置成员放行替身。省略保留、null清除仍受组织范围限制，处理人不扩大SELF。 |

## Contract and Resource Bounds

| Boundary | Implemented behavior |
| --- | --- |
| 创建 key 与摘要 | key 为1–128个 ASCII 字母数字或`._:-`；保持旧JSON字段白名单，验证/规范化后固定字段摘要。五元唯一域隔离组织和用户，提醒幂等不复用为创建幂等。 |
| 原子与重放 | 工单、编号计数器与原始201 receipt同事务；当前create授权先于receipt；异正文409。历史receipt返回首次创建结果，当前状态必须另读detail。 |
| DTO 与版本 | 拒绝未知/重复字段、非对象JSON、非法UTF8/NUL、超限内容和身份覆盖；title按Unicode校验。PATCH/DELETE要求合法version，行锁后比较，竞争只允许一次提交。 |
| 状态与删除 | 保留待处理→处理中→已解决、已解决→处理中及同状态更新；非法转换400。软删除仍保留历史记录，所有读取继续叠加未删除谓词。 |
| 成员 | Phase19前所有显式非null assignee为400；省略保留、null清除受组织scope限制；不启动允许所有成员的运行stub。 |
| 外部HTTP与事务 | 平台内省/context在业务pool/事务前完成；锁内和等待后校验范围/版本/授权期限，提交前后检查新鲜度。受控gate证明外部HTTP等待期间pool/行锁可用。 |
| 请求与DB预算 | HTTP总期限15秒；DB acquire/statement/lock默认各5000ms且独立可配；平台授权默认总预算3000ms。真实PG超时/取消回归通过，不以代码常量替代行为验收。 |
| 不确定提交 | COMMIT失败、提交后expiry和写入总超时返回503恢复指导；create保留原key/正文，PATCH/DELETE先读回当前版本；中止不等于未提交，不自动重试非幂等写入。 |
| HTTP/CORS | requestId生成/传播、Origin:null和错误CORS保持；创建预检允许Idempotency-Key；响应no-store，不返回secret。 |
| 未实施 | 不实现成员目录、提醒、生产切流或工单镜像交付；live/ready不代表调用方获平台授权。 |

## Separate Runtime Evidence Layers

| Layer | Actual result | Evidence |
| --- | --- | --- |
| Windows新增定向纯/局部HTTP | 四命令退出0，9 PASS/0 FAIL/0 ignored：DTO6、授权2、创建CORS1，六输入前后稳定；不是完整lib/PG。 | [windows-pure.json](../../target/phase-18/windows-pure.json)及其四份日志。 |
| 迁移真实PG | 21 PASS/0 FAIL/0 skip，旧Node单历史、SQLx/双历史、空库/接管和增量0002实际执行。 | [migration21](../../target/phase-18/linux-work-orders-r1/reports/postgres/work-orders-postgres/postgres-result.json)。 |
| 受控平台HTTP+业务PG | 原read13和新write14全部实际通过；平台响应为精确受控fixture，业务SQL真实执行。 | [read13](../../target/phase-18/linux-work-orders-r1/reports/postgres/work-orders-read-postgres/postgres-result.json)、[write14](../../target/phase-18/linux-work-orders-r1/reports/postgres/work-orders-write-postgres/postgres-result.json)。 |
| 真平台联合Phase17 | 独立1个Rust test、13个named checks通过，原scope/撤权/失效/不可达读取回归保持。 | [live17](../../target/phase-18/linux-work-orders-r1/reports/postgres/work-orders-live-postgres/postgres-result.json)。 |
| 真平台联合Phase18 | 独立1个Rust test、12个named checks通过；真实密码login/context/app-token、confidential introspection、正式root grants，以及独立Rust业务进程/双owned库。 | [live18](../../target/phase-18/linux-work-orders-r1/reports/postgres/work-orders-write-live-postgres/postgres-result.json)。 |
| 原平台回归 | 所选74项全部通过、零失败/跳过。 | [platform74](../../target/phase-18/linux-work-orders-r1/reports/postgres/platform-postgres/postgres-result.json)。 |
| scopedPG执行子级 | 14/14步骤PASS，含以上六组与lib43/doctest1/rustdoc/ELF build，均对应同691候选。 | [PG summary](../../target/phase-18/linux-work-orders-r1/reports/postgres/summary.json)。 |
| 完整共享fast子级 | 52/52步骤PASS，745 PASS/0 FAIL/0 skip；contracts355、CIpolicy84及工单fmt/check/strictClippy/lib43/doc均通过。745已含lib43，不再相加。 | [fast summary](../../target/phase-18/linux-fast-r1/reports/fast/summary.json)。 |
| 原ELF raw runtime补验 | 12checks全部PASS，UID10001、无Node/hostports/workspace mounts；PGdown/recover、启动锁/期限、历史/receipt/data/counter保持与SIGTERM通过。 | [supplement](../../target/phase-18/linux-runtime-r1/supplement-result.json)、[raw runtime](../../target/phase-18/linux-runtime-r1/raw-runtime-result.json)。 |

受控write14的响应恢复通过专用HTTP proxy完整取得业务上游201后断开下游；live18的恢复在取得201响应头后丢弃未读body。两者分别证明对应的客户端恢复边界，都不声称模拟了PostgreSQL COMMIT ACK网络丢失。代码对COMMIT未知结果提供恢复语义，不把这两个HTTP场景当作数据库传输故障证明。

真平台联合transport是`explicit-development-loopback-http`，不构成生产TLS部署验收。raw runtime故意使隔离loopback HTTPS平台不可达，只证明读写401/503 fail-closed、健康/故障/生命周期与数据保持；真实平台写入成功由live18证明，二者不混计。

本轮PostgreSQL选择范围为Phase18/工单及所选原平台74，不重复其他聊天负责的Phase23 notification live，`fullPostgresGateExecuted=false`。这里的scopedPG通过不能扩写为全部PostgreSQL套件通过。

## Source, Migration, Dependencies, and Artifacts

- [正式输入](phase-18-inputs.json)对应`target/phase-18/source-candidate-r1`：691文件，SHA-256 `9bea11c1855bdea5126fec3503e6b22d69e87a0d00a31f5f86b4187e8825ffec`，捕获于`2026-10-17T02:35:23.298Z`，closure PASS/251条resolved edges。两次Linux执行均记录`executionSourceUnchanged=true`。
- [正式边界](phase-18-boundaries.json)状态`BOUNDARIES_PASS`，15项检查通过；28项必需输入与候选字节等价，17项legacy保留，原Phase17 harness和旧Node兼容输入不被新写入实现替换。
- 旧0001 SHA-256 `a2e4224febd195fdd663a8954c3be4b76a8a06338521056c6809ebaa57d04090`；新0002 SHA-256 `70700874ffd778b9c9c6921614fe54efcad1512b57b5a1fd8373ad56d8f4541c`。迁移21证明实际增量兼容，未改写旧0001。
- 工单Cargo manifest/lock与Phase17完全相同，253个第三方依赖身份保持，新增/移除均0。复用原锁文件及依赖库存，不声明执行了新的漏洞、license或法律审计；未新增参考crate或平台实现依赖。
- 同源原ELF为66,927,640bytes，SHA-256 `ebc0d7a13fb567121e9945a46f5cdb1a7c35cae310161551caf0c6e23de1ad52`。raw直接复制该二进制进入owned运行容器，rootfs可写；无工单应用镜像交付，不能称只读根文件系统。
- 两个CI README在freeze后补充21/read13/write14/live17/live18统计说明：`.github/README.md`、`scripts/ci/README.md`。它们是记录在边界中的文档字节漂移，两个outer均记`hostWholeTreeEquivalent=false`；冻结候选及受测产品源保持，不声称当前整个宿主目录与691文件逐字节相同。
- 外部driver、原报告和补验driver分别归档/摘要。冻结产品输入、当前宿主观察、外部补验来源分开记录，不回写原候选或原FAIL。

## Original Runner Failures and Cleanup

[原PG外层](../../target/phase-18/linux-work-orders-r1/linux-runner-result.json)和[原fast外层](../../target/phase-18/linux-fast-r1/linux-runner-result.json)均仍为`FAIL`。子级测试已完成后，target下新driver的Docker资源空查询漏写`label=`前缀，daemon返回`invalid filter 'saas.phase18.owner'`。原FAIL和原错误保留；不是子级业务/契约测试失败，也不能把后续补查询覆盖成原runner PASS。

随后独立执行[PG更正资源审计](../../target/phase-18/resource-audit-work-orders-r1-complete.json)和[fast更正资源审计](../../target/phase-18/resource-audit-fast-r1-complete.json)。各用四条带正确`label=...`的argv查询：phase-owner与CI-owner各查containers/networks，全部exit0且输出空；各WSL lease停止、exit0。两个报告引用原outer摘要，保留原错误，证明清理而不重跑测试。

| Resource | Verified result | Evidence |
| --- | --- | --- |
| 迁移/read/write/platform测试数据库与pool | 对应用例cleanup标记通过；write14逐例`assertions_ok/database_removed/pool_closed=true`。 | 各PG suite原始日志及运行索引。 |
| Phase17 live双库/pool/业务进程 | 三类cleanup全部通过。 | live17结构化结果及原日志。 |
| Phase18 live双库/pool/业务进程 | 两库`assertionsOk/databaseRemoved/poolClosed=true`，`processCleanup.stopped=true`。 | live18的三类WRITE_LIVE cleanup标记由严格parser核对。 |
| scopedPG与fast owned容器/网络 | 独立更正审计各4查询exit0/输出空，lease停止且exit0；原outer FAIL不覆盖。 | 两份resource-audit-*-complete.json和归档driver。 |
| raw业务库/容器/网络 | runtime库删除，剩余containers/networks空，cleanupErrors空，lease停止且exit0。PG使用owned tmpfs随容器删除。 | supplement/raw-runtime原始结果。 |
| 冻结源与原ELF | PG/fast执行源未变，raw的sourceAndBinaryUnchanged=true；两README drift单列。 | 两outer、正式边界和supplement结果。 |

## Development Failures and Current Host Observations

[编写期记录](../../target/phase-18/development-failures.json)保留freeze前两类失败：新授权测试漏导入Uuid导致早轮编译3errors，初版Patch手写Default被Clippy的derivable_impls拦截；均在冻结前修复，后续Windows9和最终候选完整lib/strict门禁通过。该文件是回顾记录，不是原始stderr副本，也不猜测当时源码摘要。

最终收集器的首次本地预检把边界检查项的`result`误读为`status`，失败保存在[预检r0](../../target/phase-18/managed-audit-preflight-failure-r0.json)和[原r0收集器](../../target/phase-18/collect-managed-audit.r0.mjs)。修正本地断言后重新收集得到27项PASS/116份证据绑定；没有重跑测试或修改原测试报告，原预检失败保持可查。

[本轮当前宿主平台strict Clippy归属](../../target/phase-18/platform-clippy-current.json)在`2026-10-17T02:36:11.706Z`实测exit101、53errors；Phase18新harness为0errors/0diagnostics，受检文件前后摘要相同。此处是当前宿主整个平台观察，与冻结候选工单crate strict通过分列；不借新文件零诊断声称整个平台全通过。

## Risks / Notes

- 完成结论是Phase18范围内验收和必需门禁通过。两个原outer因资源查询错误仍为FAIL；更正资源审计与raw分别采用，不拼称一次整体从未失败。
- 全共享fast本轮子级52/52实际通过；原平台74是所选PG回归，notification-live等未选套件不在本轮结论中。其他聊天的阶段结果和变化只按边界保留，不归入Phase18产出。
- 历史201 receipt不代表当前记录仍存在或未被修改；未知PATCH/DELETE先读回核验，不能盲目自动重试。
- Phase19前所有显式非null assignee继续拒绝。未实施提醒、成员目录、生产切流或TLS部署验收；raw rootfs可写，未交付应用镜像。

## Suggested Task Ledger Update

采用[结构化更新](phase-18-ledger-update.md)：Phase18 `COMPLETED`，四条验收checkbox全部完成，本阶段无剩余项；下一建议Phase19“迁移 Rust 工单成员目录与分配校验”/`NOT_STARTED`。同步摘要表、阶段详情、Notes和按本轮用户授权调整的执行指针，同时保留其他聊天已采纳状态及修改。正式索引使用`PHASE18_GATES_PASS`，明确保存原outer FAIL、补审计和上述范围限制，不自动实施下一阶段。
