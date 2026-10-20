# Phase Execution Result — Phase 20

状态：`COMPLETED`。三条台账验收及Phase20范围内必需门禁全部通过，同一r2候选的scoped PostgreSQL 18/18步骤、完整Linux fast 53/53步骤和原ELF Node-free runtime 12/12检查全部PASS。fast实际778 PASS／0 FAIL／0 skip；PG父测试158 PASS，加独立lib56／doc1后该层实际215 PASS，不能把与fast重复执行的lib56重复解释成不同测试。managed审计为 `PHASE20_GATES_PASS`、27项检查全部PASS、绑定179份原始产物。独立开发观察、原始native失败和平台strict债务保持原状态，不混入最终r2计数。日期2026-10-17（Asia/Shanghai），原始UTC ISO时间保持原值。

## Executed Phase

Phase20：迁移 Rust 工单提醒到平台通知API。依赖Phase17、18、19已完成。本次范围为独立提醒POST、收件人选择、当前ticket/read及notification/publish授权、平台回执校验，以及保证事件身份不漂移所需的0003持久提醒意图和安全接管。本次只完成Phase20，下一建议Phase21／NOT_STARTED；不实施前端恢复、完整Socket.IO联合、门店消费者、通用框架、镜像发行或生产切流。

## Execution Summary

`POST /work-orders-api/tickets/{id}/remind` 仅接收 `{idempotencyKey}`；事件键为1–128个ASCII `[A-Za-z0-9._:-]`，区别于创建header和requestId。一次当前机密客户端内省／应用Bearer context同时检查ticket:read、独立ticket/read grant、notification:publish、独立notification/publish grant及组织硬上限。每次新请求和恢复均重新授权，先限制tenant、当前read范围及未软删除工单，收件人为处理人或无分配时的owner，目标组织为工单组织。提醒不修改工单、version或创建receipt。

新增0003的 `work_order_reminder_intents` 按tenant／发布者organization／可信application UUID／发布user／key唯一，保存ticket、目标组织与用户、完整类型化平台正文和SHA-256。该表禁止更新／删除，无TTL、级联删除、token、发送队列或本地成功cache。短scope内 `FOR SHARE` 事务先claim或核对意图，确认COMMIT并释放业务连接／锁后，每个传入请求最多以当前应用Bearer向正式平台通知API发出一次POST。同域同键更换工单、有效收件目标或正文409，即使首次平台发布根本未提交；固定INFO模板只含不可变编号和详情链接，不因工单标题／状态／version编辑变化。

意图COMMIT未确认，或总deadline在尝试平台POST前耗尽，返回503 `REMINDER_PREPARATION_UNKNOWN`，明确本请求零次平台POST。下一次若已有intent必须遵循它；若确无持久intent，可按当前快照首次claim，不能声称未提交的准备已经永久固定旧收件人。确认intent并尝试POST后遇网络、超时、5xx、无效200或期限到期，返回503 `REMINDER_RESULT_UNKNOWN`；可能已发布，保留同键及已固定完整正文，用当前有效凭据显式恢复，不自动换键、补发或直接写平台通知表。

平台明确400／401／403／404／409分别投影为400 VALIDATION_ERROR、401 INVALID_APP_TOKEN、403 DATA_DENIED、404 RECIPIENT_NOT_FOUND、409 IDEMPOTENCY_CONFLICT；其他不确定发布失败保持结果未知。成功200只投影UUID publicationId、recipientCount=1、deliveryCount=1、布尔duplicate和RFC3339 createdAt。受控HTTP记录与实际平台publication／接收者真实session收件箱分别验证；实时服务未运行、用户离线或无Toast不改变持久成功，删除delivery后同publication重放不复活它。

## Deliverables

- [Rust提醒编排](../../business-apps/work-orders/backend/src/reminders.rs)、[类型化输入／固定正文／回执](../../business-apps/work-orders/backend/src/reminders/dto.rs)、[不可变意图claim](../../business-apps/work-orders/backend/src/reminders/store.rs)，以及授权／HTTP接线。
- 新增 [0003_reminder_intents.sql](../../business-apps/work-orders/backend/migrations/0003_reminder_intents.sql)，三版本Node SHA-256／SQLx SHA-384双历史接管、完整catalog启动校验和运行期ready检查；0001／0002及Cargo依赖不变。
- [受控HTTP／真实业务PG提醒回归](../../business-apps/work-orders/backend/tests/reminder_contracts.rs)、[实际平台／独立业务库联合](../../backend/src/phase20_work_orders_tests.rs)，及[精确CI注册](../../scripts/ci/manifest.mjs)／[具名结果解析](../../scripts/ci/work-orders-reminder-live-results.mjs)。
- 7份接入文档：根README、架构、工程基线、应用README、后端README、Rust工单说明和data-grant管理。说明独立发布授权、两种unknown、真实收件箱、跨库竞态及0003回退边界，明确旧Node handler无新intent保证。
- [正式inputs](phase-20-inputs.json)、[正式boundaries](phase-20-boundaries.json)、[正式运行索引](phase-20-runtime.json)与[台账更新建议](phase-20-ledger-update.md)关联target原始运行、日志、driver、原ELF和managed产物，保留各自来源和终态。

## Self-Check

以下保留台账验收原文，结论采用最终r2的实际受控HTTP／PG和真实平台联合证据。

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 同事件重试只一条持久通知；工单version不因提醒改变。 | PASS | reminder12验证稳定原文、同key并发、单publication、响应丢失／超时恢复及version不变；live20父1／named12通过真实平台publication／delivery、同事件回执及业务进程重启恢复。 |
| 无读权限/发布权限/数据grant、越界收件人均拒绝；未知结果不自动换键重发。 | PASS | read／publish permission和两类grant逐项撤销拒绝，SELF／组织硬上限／有效直接成员矩阵通过；preparation proof明确COMMIT拒绝时publication_posts=0，durable intent在平台未提交后换目标仍409，unknown proof记录同key／原文／publication_count=1／no_automatic_retry。 |
| 平台收件箱可核对结果，实时服务未就绪不被误判为持久发布失败。 | PASS | live20用接收者真实平台session按publicationId核对APP通知与组织收件箱，实时服务未启动仍返回持久成功；删除delivery后同publication重放不补建，工单version保持。 |

## Contracts and Race Boundaries

| Boundary | Implemented and verified behavior |
| --- | --- |
| 当前身份与双授权 | 同一次intro/context建立read及notification/publish断言；当前read／publish及软删除检查先于intent恢复，不用历史意图绕过撤权。 |
| 准备持久化边界 | 本地COMMIT未确认时不得POST；确无intent的下次请求可首次claim当前快照。未落库准备不能承诺固定旧目标。 |
| 不可变事件身份 | 五元域与平台一致，publisher-org区别于target-org；完整原文／hash与ticket／有效target核对，损坏503且不发上游。 |
| 一次入站最多一次POST | 意图提交和连接释放后只调用一次正式API，无自动重试、换键或成员预查，不缓存发布成功。 |
| 平台历史回执 | 当前发布者授权和目标scope仍须成立；已提交相同publication不重新枚举recipient、不重建被删除delivery。 |
| 跨库竞态 | 意图提交后工单仍可修改／软删；平台200已验证后不重新读工单把成功改判失败。两库不原子。 |
| 0003与回退 | 不修改0001／0002或增加依赖。原Phase19 strict baseline拒绝0003；保留intent／receipt／publication并用兼容构建，不能删数据／历史或改checksum回退。 |
| 旧Node差异 | 迁移runner应用0003不意味着handler采用intent；切回旧提醒不能被称为保持Phase20事件固定语义。 |
| 阶段边界 | 不改前端未知结果恢复、不运行完整Socket.IO联合、不交付或部署生产镜像、不切流。 |

## Source Provenance and Development Observations

正式采用候选为r2，捕获 `2026-10-17T05:10:56.421Z`，711 files／283条AST resolved edges／42个explicit required输入／44个registered test文件，SHA-256 `4f9a2cb80ece51c86e07a86bdafb98ca765fc2498a30ae5f64e7ce08cec3f449`。source revision `32956c4532bbd6eff53584fd0f5e9bb6e188437c` 是参考提交，实际受测字节由候选文件清单与摘要绑定；capture closure PASS、hostSharedDriftAtCapture=[]，最终PG／fast／raw执行来源也均稳定、hostDrift=[]。selector `scripts/ci/provenance.mjs` SHA `c7620d698d1eca22a7efbad0bbb402952d97b75e9560282e7d710c7edd8c7fc8`；捕获driver freeze-source SHA `0d136b4a7aa4e1e1e7192dbf117a864d3421ba9dc0d3203397646fdb53d49642`、source-closure SHA `b328fd2afe3ffd3bc45ac8bc2ebe91e71d1e53ee52680fd7a33f20871d5e827b`。输入见 [r2 capture](../../target/phase-20/inputs-candidate-r2.json)及[正式输入](phase-20-inputs.json)，来源检查与实际执行分别记录。

r1候选711 files，SHA `4f8b4558d8e0bafdb13981ff7e10f18f8dac0de671a0838dda84804d5d2fd91e`，capture时间 `2026-10-17T05:06:39.337Z`，仅捕获、未dispatch正式Linux门禁；因此没有r1 outer FAIL或取消运行。平台live测试的私有tuple alias修复后重新捕获r2。不能把Phase19的取消历史套入本轮候选记录。

以下为各自来源上的开发观察，单独保留，不计入最终r2的门禁数量：

| Observation | Actual result and limits | Original evidence |
| --- | --- | --- |
| 开发迁移／提醒readiness | 源711 files／SHA `b14139efb7e2f0878d5b81d6866abab80a0f39a305a682a048a61a106ca874f0`；migration27、controlled reminder12均PASS／0FAIL／0skip，4个严格proof全部true，12个case assertionsOk／databaseRemoved／poolClosed均true。outer PASS／exit0，executionSourceUnchanged=true；hostDrift实际为 `.github/README.md` 与 `scripts/ci/README.md` 两路径。cleanupErrors=[]、unexpectedLeaseExit=false、lease已停止exit0，四种正确owner资源查询均exit0且空。此为DEVELOPMENT_READINESS，fullPostgresGateExecuted=false，既非完整PG也非最终r2。 | [outer](../../target/phase-20/linux-readiness-r1/linux-runner-result.json)、[summary](../../target/phase-20/linux-readiness-r1/reports/postgres/summary.json)、[migration](../../target/phase-20/linux-readiness-r1/reports/postgres/work-orders-postgres/postgres-result.json)、[reminder](../../target/phase-20/linux-readiness-r1/reports/postgres/work-orders-reminder-postgres/postgres-result.json)。 |
| Native CI policy readiness | 100PASS／0FAIL／0skip，exit0、sourceStable=true，before／after均为 `4f8b4558d8e0bafdb13981ff7e10f18f8dac0de671a0838dda84804d5d2fd91e`；该观察在tuple alias修复前，不能替代r2完整fast。 | [ci-readiness-r1](../../target/phase-20/ci-readiness-r1.json)与其stdout／stderr。 |
| Native work-orders fmt／check | fmt PASS／exit0／sourceStable=true；check r1 FAIL／101，测试JSON直接序列化Uuid导致E0277，改成to_string且不增依赖后check r2 PASS／0。r1 sourceStable=false、记录漂移 `backend/src/phase20_work_orders_tests.rs`；r2 sourceStable=true。 | [fmt](../../target/phase-20/native-r1-fmt.json)、[check-r1](../../target/phase-20/native-r1-check.json)、[check-r2](../../target/phase-20/native-r2-check.json)，原stderr／stdout保留。 |
| Native work-orders library | 56项中54PASS／2FAIL／0ignored，wrapper FAIL／101，sourceStable=true。失败原名为 `authorization::tests::fresh_basic_introspection_then_bearer_context_are_never_positive_cached` 与 `authorization::tests::upstream_status_body_redirect_json_and_capacity_fail_closed`，均为已有HTTP授权测试；本轮未证实网络根因，不称Native全部通过。 | [library](../../target/phase-20/native-r1-library.json)及原stdout／stderr。 |
| Native work-orders strict Clippy | 独立工单crate all-targets／all-features、`-D warnings` PASS／exit0／sourceStable=true。该局部结论不代表整个平台strict通过。 | [work-orders Clippy](../../target/phase-20/native-r1-clippy.json)。 |
| 全平台strict Clippy | r1 FAIL／101／54errors，含Phase20 live新type_complexity1；只引入私有tuple alias后r2仍FAIL／101／53errors，phase20 live与main归属errors均0，sourceStable=true。两轮JSON、JSONL、stderr和wrapper原样保留，不把局部修复写成整个平台strict PASS。 | [platform-r1](../../target/phase-20/platform-clippy-r1.json)、[platform-r2](../../target/phase-20/platform-clippy-r2.json)。 |

## Adopted r2 Evidence

| Layer | Actual result | Evidence |
| --- | --- | --- |
| 冻结候选／闭包 | 711 files／283 edges／42 required／44 registered tests；来源稳定、hostDrift=[]。 | [正式inputs](phase-20-inputs.json)、[r2 capture](../../target/phase-20/inputs-candidate-r2.json)、三路原运行来源记录。 |
| Native开发观察 | MIXED，见上节原始结果 | fmt／check-r2／工单strict PASS；check-r1、lib54／2及平台strict54→53原失败保留，不以最终Linux改写。 |
| 完整Linux fast | outer／child PASS、exit0，53/53步骤PASS，778PASS／0FAIL／0skip。contracts355、CIpolicy100、独立工单fmt／check／all-target all-feature strict Clippy／lib56／rustdoc全部通过，lib56已含在778中。 | [outer](../../target/phase-20/linux-fast-r2/linux-runner-result.json)、[完整summary](../../target/phase-20/linux-fast-r2/reports/fast/summary.json)及53份原日志。 |
| Scoped PostgreSQL | outer／child PASS、exit0，18/18步骤。平台76、migration27、read13、write14、member12、reminder12、四个live父测试各1，共158父测试；另lib56、doc1及rustdoc／ELF build通过。全部0FAIL／0skip，按层合计215实际测试执行。fullPostgresGateExecuted=false，独立notification-live未选择且不借历史结果。 | [outer](../../target/phase-20/linux-work-orders-r2/linux-runner-result.json)、[完整summary](../../target/phase-20/linux-work-orders-r2/reports/postgres/summary.json)。 |
| 0003 migration | 27PASS／0FAIL／0skip。fresh／Node及SQLx合法v1-v3前缀、receipt与intent保持、Node-Rust并发重启、增量回滚／重试、intent不可变与application域、坏历史／结构拒绝均由真实PG验证。 | [migration27](../../target/phase-20/linux-work-orders-r2/reports/postgres/work-orders-postgres/postgres-result.json)及work-orders-migrations.log。 |
| 受控提醒合同 | 12PASS／0FAIL／0skip，四proof全部true：unknownResult、databaseRelease、recipientChange、preparation；12个case assertionsOk／databaseRemoved／poolClosed均true。唯一pool可用且同工单行锁可取得、POST等待响应时无业务锁；准备COMMIT失败publication_posts=0；持久intent使首次平台未提交后的目标变化仍冲突。 | [reminder12](../../target/phase-20/linux-work-orders-r2/reports/postgres/work-orders-reminder-postgres/postgres-result.json)及work-orders-reminder-contracts.log。 |
| 真实平台提醒联合 | live20父1PASS、12named全部PASS／0FAIL／0skip；真实密码login／app-token／confidential introspection、正式grant预览／变更、实际业务进程及两座独立库，核对publication／delivery／接收者session inbox。两库assertionsOk／databaseRemoved／poolClosed、processRestart.stopped和processCleanup.stopped全部true。 | [live20](../../target/phase-20/linux-work-orders-r2/reports/postgres/work-orders-reminder-live-postgres/postgres-result.json)及实际联合日志；transport明确development-loopback-http。 |
| 既有真实工单联合回归 | live17／18／19各1父测试PASS，named分别13／12／10全PASS；各平台／业务库／pool及进程清理全部true，不借用旧阶段运行。 | [live17](../../target/phase-20/linux-work-orders-r2/reports/postgres/work-orders-live-postgres/postgres-result.json)、[live18](../../target/phase-20/linux-work-orders-r2/reports/postgres/work-orders-write-live-postgres/postgres-result.json)、[live19](../../target/phase-20/linux-work-orders-r2/reports/postgres/work-orders-member-live-postgres/postgres-result.json)。 |
| 原ELF Node-free runtime | 同PG原ELF `67,913,136` bytes／SHA `8fb1a06578f379dc92b04942ebdace81c7cd09d2d11d5664665d431640a890cf`，12/12检查PASS。UID10001、无Node／host ports／workspace mounts、rootfs可写；ready／PGdown恢复、三版本双历史、receipt1／intent1／业务数据及counter保持、启动期限拒绝／锁释放重试、SIGTERM exit0。 | [raw12](../../target/phase-20/linux-runtime-r2/raw-runtime-result.json)、[outer supplement](../../target/phase-20/linux-runtime-r2/supplement-result.json)、[原ELF](../../target/phase-20/linux-work-orders-r2/reports/artifacts/work-orders-api)。 |
| Boundaries／managed | fresh正式boundary BOUNDARIES_PASS、8/8检查、65 owned inputs／2 CI fixtures／42 required，历史SQL／Cargo固定、hostDrift=[]。managed PHASE20_GATES_PASS、27/27检查、179份产物，SHA `e89e1386e674d0b3f2ec8ba188b5dfc669fd47ad0124c739f40ca6244ea6a026`；artifactsSha256为 `6951ff90587cd5883c34233f4735cf09b23287b4fb601279b58276d74b19172b`，它是产物清单摘要，不是额外文件。 | [boundary](phase-20-boundaries.json)、[managed](../../target/phase-20/managed-audit.json)。 |

三路采用运行的源、原ELF及driver均保持绑定：PG／fast的executionSourceUnchanged=true，raw的sourceAndBinaryUnchanged=true，hostDrift均[]。每路均完成phase owner与generic CI owner各自container／network四项资源查询，命令使用精确 `label=`、exit0且结果为空；cleanupErrors=[]、unexpectedLeaseExit=false，WSL前台lease已停止exit0。raw在自有可写transport容器中直接复制原ELF，没有构建或发布业务生产镜像；其平台故意不可达，只证明授权失败关闭，成功提醒由真实live20证明。

managed保留development readiness一份PASS运行、两份retained candidate（r1未dispatch；development候选关联自己的实际运行）、6份native／Node开发观察和两份平台Clippy失败。originalOuterFailures=[]；这只表示本轮没有失败的外层运行，不能抹掉已披露的native与平台strict失败。179为managed绑定产物数，不应冒充正式runtime索引自身的产物数量。

## Actual Commands and Environment

最终Linux使用Node24.18.0、npm11.16.0、rustc1.97.0和Docker29.1.3；raw PostgreSQL17.6。PG与fast由本轮归档 `run-linux.mjs` 及各自snapshot／preflight执行，raw由归档supplement／raw-runtime driver复用PG原ELF。各子报告保存实际argv、exit、timedOut与完整日志；关键实际子命令为：

```text
cargo test --manifest-path business-apps/work-orders/backend/Cargo.toml --locked --test migrations -j 2 -- --ignored --test-threads=1 --nocapture
cargo test --manifest-path business-apps/work-orders/backend/Cargo.toml --locked --test reminder_contracts -j 2 -- --ignored --test-threads=1 --nocapture
cargo test --manifest-path backend/Cargo.toml --locked --bin saas-platform-backend -j 2 phase20_work_orders_tests::real_platform_reminder_publication_and_recovery -- --ignored --test-threads=1 --nocapture
cargo clippy --locked --manifest-path business-apps/work-orders/backend/Cargo.toml --all-targets --all-features -j 2 -- -D warnings
```

显式ignored选择按本轮manifest和资源ownership执行，不把默认跳过算通过。完整fast包含既有Socket.IO单元／兼容回归，但不因此将本台账全局 `realtime_protocol_result` 从NOT_EVALUATED改成已联合验收，也不把它当Phase23／34完整联合结果。

## Risks / Notes

Phase20范围内无剩余实施或必需验收项。跨库身份／工单变化窗口、未持久准备边界、平台历史回执不重新枚举recipient，以及旧Node差异均为已说明的契约限制。0003升级后不能直接退旧严格Rust二进制；须保留intent／receipt／platform publication，对账在途结果并采用兼容构建。前端现有创建header／恢复仍归Phase21。

本轮只验正式HTTP持久发布与收件箱；真实联合使用显式开发loopback HTTP，不宣称生产HTTPS、完整Socket.IO、现网、registry、部署或切流验收。原生HTTP两失败根因未证实，全平台strict仍53errors；局部工单strict和最终范围内门禁通过不改变这些观察。未选择notification-live，不称完整postgres gate；全局realtime_protocol_result仍NOT_EVALUATED。

## Suggested Task Ledger Update

Phase20更新为COMPLETED，三条验收checkbox全部完成；Expected Outputs引用正式报告与inputs／boundaries／runtime证据。同步摘要表、阶段Notes、执行指针及Change Log，保留其他聊天已采纳状态和修改。具体结构化建议见 [ledger-update](phase-20-ledger-update.md)。本次在Phase20终止，下一建议Phase21“交付 Vue + Vite 工单前端与未知结果恢复”／NOT_STARTED，须新明确请求，不自动实施。
