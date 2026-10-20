# Phase Execution Result — Phase 17

状态：`COMPLETED`。四条行为验收及Phase17范围内全部执行门禁PASS，[managed审计](../../target/phase-17/managed-audit.json)为PHASE17_GATES_PASS、14项检查全PASS。完整共享fast保留FAIL（50步48PASS/2FAIL，测试700PASS/1FAIL/0skip），两失败属于冻结候选的并行CLI内容；范围内结果和不同宿主源的外部补验分别采用，不拼称一次全绿。客户端日期**2026-10-17（Asia/Shanghai）**，原始UTC ISO时间保持原值。

## Executed Phase

Phase 17：实现 Rust 工单授权及只读/导出契约。

依赖Phase4、6、11、16已完成。本次复用独立Rust工单crate，实现逐请求Basic内省与同一应用Bearer context、严格授权DTO/policy、列表/count/detail/筛选/分页/CSV，以及requestId、Origin:null CORS和错误映射。保留内省、不跨请求缓存成功授权，不实现写入/提醒/成员，不新增后台SDK或参考crate，不切生产流量。本次只推进Phase17，保留并行其他阶段已采纳结果及修改，其产出不归入本阶段。

## Execution Summary

Rust[授权边界](../../business-apps/work-orders/backend/src/authorization.rs)每请求使用私有机密客户端Basic调用`/oauth/introspect`，再以原应用Bearer读取`/api/apps/work-orders/context`。固定work-orders受众，检查active、Bearer类型、到期与user/tenant/org一致；功能permission与当前直接成员身份绑定的数据grant分别校验。所有读取共同叠加tenant、组织硬上限、对应grant及未删除条件，SELF按owner判断。列表count/records与CSV容量count/数据采用只读一致快照，查询及DB等待后复验有效期。

同674项不可变候选`59b0f8d12db41b566e42f59d0aeba38f2afe72c547a18ca3501dabb53a654028`的PG summary **9步全部PASS**：平台74、原迁移15、新read13均零失败/跳过，真正平台联合为**1项Rust harness、13个named checks**全PASS。额外完整Linux lib34、doctest1、ELF build通过。原外层runner仍为**FAIL**：成功完成容器内测试后，发现宿主共享`manifest.mjs`漂移，raw尚未创建；不能覆盖该FAIL。随后从同一候选原ELF另行补验rawruntime11项PASS，来源和清理分别保留，未重跑PG或拼称一次整体成功。

代表查询基线已归档8份实际EXPLAIN，保留SQL、原参数、计划与buffers，未新增索引。独立工单fmt/check/alltarget全feature strictClippy/lib34/doc五个fast必需项均PASS，ci-policy63项PASS。完整共享fast的contracts和platform-admin-fmt失败、Windows历史失败及当前宿主平台Clippy53错误分别保留；新平台harness没有诊断。阶段完成采用“Phase17范围内门禁通过、共享外部失败披露”的证据模型，原摘要和候选不改写。

## Deliverables

- [授权入口](../../business-apps/work-orders/backend/src/authorization.rs)、[有界HTTP客户端](../../business-apps/work-orders/backend/src/authorization/client.rs)、[身份与scope policy](../../business-apps/work-orders/backend/src/authorization/policy.rs)、[授权单元回归](../../business-apps/work-orders/backend/src/authorization/tests.rs)。
- [统一只读用例](../../business-apps/work-orders/backend/src/reads.rs)、[参数/分页解析](../../business-apps/work-orders/backend/src/reads/query.rs)、[CSV编码](../../business-apps/work-orders/backend/src/reads/csv.rs)与[HTTP/CORS/requestId](../../business-apps/work-orders/backend/src/http.rs)。
- [owned业务PG+controlled HTTP13项](../../business-apps/work-orders/backend/tests/read_contracts.rs)、[真正平台联合harness](../../backend/src/phase17_work_orders_tests.rs)及[8份代表查询计划归档](phase-17-query-plans.json)。
- [旧Node后台授权参考退役说明](../../examples/app-backend/README.md)与`authorize.mjs`顶部注释；原函数体及测试保留，规范参考指向实际Rust工单实现，无新增参考crate。旧JS7项仅为兼容证据。
- [Rust工单操作说明](../work-orders-rust-baseline.md)、根/组件README、架构和工程基线同步。已实现GET只读/CSV与后续写入/提醒/成员分开，历史Phase16证据保留。
- [正式输入](phase-17-inputs.json)、[边界记录](phase-17-boundaries.json)、[正式运行索引](phase-17-runtime.json)及本报告；结构化建议的最终位置为[Phase17台账更新](phase-17-ledger-update.md)。运行索引采用PHASE17_GATES_PASS语义，不表示全共享fast通过。

## Self-Check

四条台账验收原文保持。以下行为标准及Phase17相关fast必需项均PASS；完整共享fast中两项外部失败按原值单列，不据其不同源补验改写原报告。

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| SELF/ORG/ORG_SUBTREE/TENANT及组织上限、其他租户、上级平级、无grant、错误受众均真实验证。 | PASS | owned业务PG+controlled HTTP13项实际执行四scope/上限/跨tenant/祖先平级/空grant/错误aud；真正平台main+Rust业务进程双库harness又实证SELF/ORG/subtree、非根TENANT平台降级、平台根TENANT及错误受众。 |
| 平台不可达503、失效401、缺授权403、不可见404；客户端secret不进前端/错误。 | PASS | read13及lib34覆盖状态/协议/body/redirect/timeout和脱敏；真正联合证明缺grant、撤grant/退出/删除成员下一请求失权、平台不可达拒绝。raw独立补验401/503/CORS，secret留在Rust私有配置，公开错误不含凭据。 |
| CSV范围与列表一致，注入/容量/编码回归通过；OPTIONS及错误响应CORS正确。 | PASS | controlled HTTP+真实PG核对列表/count/detail/CSV集合、1000行/4MiB容量、BOM/CRLF/UTF8/引号/公式和控制前缀；OPTIONS及成功/401/403/404/503错误CORS和requestId通过。真正联合各scope亦核对列表/count/detail/export。 |
| 极大page稳定拒绝/受限而非溢出；列表/count/导出同范围，计划及读取成本有可复核基线。 | PASS | 最大page1000000/size100产生checked offset99999900，范围1602时稳定空页且total1602，超限/非法数字拒绝；24,008行/8份实际EXPLAIN、SQL/参数及cost/buffers完整归档。计划为代表组件基线，不是逐次HTTP trace或生产SLA。 |

## Contract and Resource Bounds

| Boundary | Implemented behavior |
| --- | --- |
| 身份与实时授权 | 私有`PLATFORM_API_ORIGIN/CLIENT_ID/CLIENT_SECRET`；每请求Basic内省后同Bearer context，固定work-orders，active/aud/type/exp及身份一致，permission和grant独立，不正向跨请求缓存。 |
| 平台HTTP预算 | 总deadline默认3000ms、范围100–10000；connect最多1秒；并发默认32、范围1–128，许可覆盖两次调用及响应body，饱和立即503。单响应最多256KiB，须JSON类型，不跟redirect、不使用系统proxy；可选CA最多1MiB。 |
| origin与TLS | 生产精确HTTPS、正常证书与主机名验证；非生产只允许显式loopback HTTP，拒绝userinfo/path/query/fragment。真正联合使用development loopback HTTP，未声明TLS部署验收。 |
| 上下文 | contractVersion1.0、work-orders appCode、64hex contextVersion、当前有效且最长30秒、与内省身份和expiry一致；对应grant唯一、tenant/subject绑定，organizationIds与ceiling明确校验。 |
| SQL与freshness | tenant、deleted_at IS NULL、组织ceiling和scope参数化共同下推；SELF按owner，ORG当前组织，subtree明确IDs，TENANT仍叠加ceiling。用户organizationId只能进一步收窄。平台调用发生在业务只读事务前，DB等待和返回前复验到期。 |
| 查询解析 | query最多8192bytes，q最多200Unicode字符；page1–1000000、size1–100，默认1/20，offset最大99999900。拒绝未知/重复/身份覆盖字段、非法UTF8/NUL/UUID/enum/数字；搜索转义通配符和反斜杠。 |
| CSV | 独立ticket:export与ticket/export；最多1000行、4MiB，预算含BOM/UTF8/引号展开/CRLF；超限完整拒绝，不截断成功。所有人类文本快照统一引用/引号转义/公式与控制前缀中和。 |
| HTTP/CORS | 合法UUID requestId生成/传播，Origin:null默认允许、额外origin精确配置、无cookie credentials；OPTIONS校验方法/header，各类响应统一附允许origin的CORS。HTTP总预算15秒，no-store。 |
| 未实施 | 写入、提醒、assignees仍拒绝；live/ready仅描述进程/业务库，不保证调用方平台授权。未改旧0001、未切流、未新增生产容器交付。 |

## Separate Runtime Evidence Layers

**受控HTTP+真实业务PG**：[read13结果](../../target/phase-17/linux-postgres-wip-r2/reports/postgres/work-orders-read-postgres/postgres-result.json)为13通过/0失败/0跳过。SQL真实执行，平台HTTP为精确可控fixture，可测试错误身份、过期、capacity、JSON/status/redirect、CORS和SQL集合。它不等同于真实平台成功调用。

**真正平台联合**：[live结果](../../target/phase-17/linux-postgres-wip-r2/reports/postgres/work-orders-live-postgres/postgres-result.json)为1个Rust test、13个named checks。真实平台Axum/router、真实机密客户端内省/token/context与独立Rust工单子进程、双库实际执行；transport为explicit-development-loopback-http。13项为：real-confidential-introspection；missing-grant-denied；self-scope-list-count-detail-export；org-scope-list-count-detail-export；subtree-scope-list-count-detail-export；nonroot-tenant-platform-downgrade；platform-root-tenant-scope；wrong-audience-denied；grant-removal-next-request；logout-next-request；membership-removal-next-request；platform-unreachable-fail-closed；loopback-http-explicit-development。双库assertions/database_removed/pool_closed全true，业务进程stopped=true；named13不混计为13项Rust tests。

**Node-free raw ELF**：[独立补验](../../target/phase-17/linux-runtime-r1/supplement-result.json)及[raw报告](../../target/phase-17/linux-runtime-r1/raw-runtime-result.json)11项PASS。使用原容器编译的同一ELF，SHA-256 `7155530bb405c655813c9edb6cfef132509cc8a007631298a03120f854974f15`、65,460,864bytes、x86-64；PID1为/work-orders-api，UID10001、无node/nodejs、ldd无缺失、无host ports/工作树mounts。Debian rootfs可写，不称生产只读镜像。平台目标故意不可达，验证缺Bearer401、伪Bearer503+CORS、PGdown ready503/live200及恢复、SIGTERM0、lock截止启动失败不开放ready、历史/counter保持与重试。它只证明failclosed，不证明成功平台读取；成功调用由真正联合承担。

**旧JS兼容**：已退役helper7项通过，仅证明原fixture兼容，没有新Node服务或参考crate，也不是Rust证据。

## Query-Plan Baseline

[计划归档](phase-17-query-plans.json)与原read日志8个JSON逐项核对，保存原SQL/参数对象、完整plan、日志行号/hash，并单独标明由冻结fixture恢复的有序bind值。总24,008行；target tenant19207含1deleted、active19206；组织ceiling active1602。PostgreSQL17.6，冻结674项同上。执行和planning buffers分别使用根节点值，不累加子节点；未输出planning buffers保留未输出。

| Case | Planning ms | Execution ms | Scan / existing index | Root execution shared hits | Planning shared hits |
| --- | ---: | ---: | --- | ---: | ---: |
| org_first_page | 0.344 | 6.456 | Bitmap Heap/Index，work_orders_tenant_org_active | 734 | 99 |
| org_maximum_offset | 0.476 | 8.574 | Bitmap Heap/Index，work_orders_tenant_org_active | 740 | 77 |
| org_count | 0.119 | 1.322 | Bitmap Heap/Index，work_orders_tenant_org_active | 734 | 3 |
| org_literal_search | 0.571 | 4.880 | Bitmap Heap/Index，work_orders_tenant_org_active | 734 | 未输出 |
| org_export_capacity | 0.318 | 2.709 | Bitmap Heap/Index，work_orders_tenant_org_active | 734 | 9 |
| tenant_first_page | 0.096 | 40.713 | Seq Scan | 706 | 未输出 |
| tenant_count | 0.143 | 7.824 | Seq Scan | 706 | 3 |
| self_first_page | 0.114 | 0.095 | Index Scan，work_orders_tenant_owner_active | 6 | 未输出 |

组织ceiling的首屏扫描1602行后top-N heapsort35kB；最大offset仍扫描自身1602行并quicksort858kB，返回0行，不证明生产亿级匹配行恒定成本。字面搜索过滤1585行、quicksort29kB；export容量limit1001前扫描1602行、quicksort459kB。tenant顺扫19206活动行、过滤4802，首屏top-N36kB；SELF现有owner索引首屏只读取20行、6个shared hits。样本执行0.095–40.713ms，planning0.096–0.571ms；这些是一次受控观测，不是分位数、吞吐或SLA。

这些SQL的投影/UTC转换、排序、参数类型与生产读取组件对应，org命名case主测organization ceiling的ANY谓词，不是每个grant全路径、每次HTTP请求自动捕获的SQL trace。原tenant/SELF/ceiling组件成本可复核，但未覆盖所有grant/filter/组织列表规模/租户分布。**本阶段未新增索引**，保留work_orders_tenant_org_active与work_orders_tenant_owner_active；本样本足以交付现有计划基线，不推出通用索引建议、全文检索或分区。

## Actual Validation Results

| Gate / attempt | Actual result | Evidence / interpretation |
| --- | --- | --- |
| Windows全lib早轮 | FAIL；31项25通过/6失败 | 4个根路由冲突已修；2个auth测试Windows loopbackTCP阻断，原失败保留，不把lib34的Linux结果回写为Windows通过。 |
| Windows授权专项 | 7通过/2失败/0ignored，UNAVAILABLE | [TCP记录](../../target/phase-17/windows-tcp-readiness.json)：127.0.0.1/::1 bind成功、直接Tokio500ms及reqwest1秒connect超时；未放宽timeout、未skip。Linux完整lib34另行实跑。 |
| 资源前WIP预检r1/r2 | 原FAIL保留；resourcesCreated空/runtimeRun=false | [r1](../../target/phase-17/preflight-failure-r1.json)、[r2副本](../../target/phase-17/preflight-failure-r2.json)：并行notification变化导致拒绝；source-wip-r2未采纳，完整source-wip-r1为执行副本。 |
| r2容器内PG | PASS；summary9步，平台74/迁移15/read13/live1均0fail/skip | [summary](../../target/phase-17/linux-postgres-wip-r2/reports/postgres/summary.json)与各子result；真平台1harness内13 named单列。 |
| r2额外独立crate | lib34/0/0、doc1/0/0、ELF build PASS | [lib](../../target/phase-17/linux-postgres-wip-r2/reports/artifacts/library-tests-result.json)、[doc](../../target/phase-17/linux-postgres-wip-r2/reports/artifacts/doctest-result.json)、[build](../../target/phase-17/linux-postgres-wip-r2/reports/artifacts/build-result.json)。 |
| r2外层runner | **FAIL保留**，exitCode0但driverError为宿主manifest漂移 | [原runner](../../target/phase-17/linux-postgres-wip-r2/linux-runner-result.json)：执行副本674项before/after不变；容器/网络空、cleanupErrors空、lease0；raw未创建。不能将容器PASS覆盖原外层FAIL。 |
| 同源raw补验 | PASS；11项，同ELF/source不变 | [supplement](../../target/phase-17/linux-runtime-r1/supplement-result.json)，runtime库删除、cleanupErrors空、容器/网络空、lease0；原r2 FAIL不改写。 |
| 边界及依赖 | BOUNDARIES_PASS，19 checks PASS | [boundaries](phase-17-boundaries.json)：23独占输入等价、17旧Node/SQL保持；原221依赖身份保留、新32合计253，license253项声明齐全；shared宿主漂移分列。 |
| Phase17独立工单fast门禁 | fmt/check/strictClippy/lib34/doc五步PASS，ci-policy63PASS | [完整fast原summary](../../target/phase-17/linux-fast-r1/reports/fast/summary.json)中work-orders40–44步全部退出0；check与Clippy含all-targets/all-features。范围内从原日志派生，不是额外整套重跑；不宣称平台strict通过。 |
| Phase17范围内fast派生报告 | PHASE17_SCOPED_FAST_PASS；9个选中步骤、137测试通过/0失败/0跳过 | [focused结果](../../target/phase-17/focused-fast-result.json)：lib34+ci-policy63+原contracts兼容40（旧helper7/CORS1/frontend client19/app13），从原summary及log成功行验证；137已包含lib34，不再次累计。derivationOnly=true、testsReexecuted=false，保留原全fast50步FAIL。 |
| 当前宿主全平台strict Clippy | **FAIL，exit101，53 error，新harness0诊断** | [JSON原输出](../../target/phase-17/platform-clippy-current.jsonl)、[stderr](../../target/phase-17/platform-clippy-current.stderr.log)；命令为backend --locked --offline --all-targets --all-features --message-format=json -- -D warnings，current host并非674候选。不能把53全部称为Phase16旧38。 |
| 当前宿主共享CI定向兼容 | PASS；6/6，exit0，10项相关输入前后稳定 | [原日志重析](../../target/phase-17/current-policy-reconciled.json)：node --test scripts/ci/work-orders.test.mjs；原collector只识别TAP而Node输出spec的ℹ计数，原current-policy.json的FAIL保留，重析同一未改日志而非重跑。只证明当前宿主共享策略兼容，不替代674候选/PG/fast。 |
| 完整共享fast | **FAIL保留**；50步48PASS/2FAIL，测试700PASS/1FAIL/0skip | [原summary](../../target/phase-17/linux-fast-r1/reports/fast/summary.json)：contracts为354PASS/1FAIL，失败于tests/platform-admin-contract.mjs:120读取当时selector未复制的business-apps/work-orders/tests/platform-e2e.mjs；platform-admin-fmt因冻结tools/platform-admin/src/lib.rs三处assert格式而exit1。两项是并行CLI输入，不是Phase17授权/读取行为失败。 |
| 外部两失败的当前宿主补验 | PASS；contract5/5及工具crate fmt退出0，相关输入稳定 | [不同源重验](../../target/phase-17/current-external-recheck.json)：执行当前宿主字节，未改源码/原674候选，未重跑全fast。不能把该5PASS回填进原700/1，亦不能把原两失败改PASS；仅说明当前宿主的外部集成检查已通过。 |
| 最终managed审计/运行索引 | PHASE17_GATES_PASS；14项检查全PASS | [managed原报告](../../target/phase-17/managed-audit.json)、[正式运行索引](phase-17-runtime.json)：同674候选PG/lib/doc/原ELF、归档driver、资源/库/pool/进程及lease清理均核对；原r2外层FAIL和fullFastFAIL保留，当前宿主策略6及external5+fmt不同源补验分列。状态表示Phase17范围内通过，不表示共享全fast绿。 |

## Source, Dependencies and Concurrent Work

HEAD为`32956c4532bbd6eff53584fd0f5e9bb6e188437c`；[正式输入](phase-17-inputs.json)保留同674项`59b0f8d12db41b566e42f59d0aeba38f2afe72c547a18ca3501dabb53a654028`，不是当前宿主重算后替换的候选。r2容器源与raw补验源/ELF前后不变。原外层runner因共享宿主scripts/ci/manifest.mjs变化失败，后续host还有notification/CLI等并发变化；[边界](phase-17-boundaries.json)将23独占输入字节等价、共享注册和外部漂移分别表达，不声称整个当前宿主或其余全部文件同候选。

17项旧Node可执行/锁/测试及旧0001保持；旧SQL SHA-256仍为`a2e4224febd195fdd663a8954c3be4b76a8a06338521056c6809ebaa57d04090`。旧参考只增加顶部退役注释，函数尾与原兼容测试不变。[依赖清单](../../target/phase-17/dependency-inventory.json)保留Phase16原221package身份并新增32，共253，253项license元数据有声明。reqwest用于有界HTTPS，chrono验证RFC3339到期，serde_json转为生产依赖；无平台实现依赖或根workspace。仅核对锁身份与声明，不称完整漏洞/安全/许可证兼容或法律合规审计。

当前宿主全平台Clippy53项分布于app_token、scope_tests、config、region_seed、releases/routes等，新phase17_work_orders_tests.rs无诊断。先前live代理含自身1项的39诊断输出是已修正前验，不能覆盖当前53或称平台strict全部通过；命令组合和并发源码不同，不与旧38机械相减归因。

并行其他阶段已采纳结果与修改保持。共享CI完整执行中可能包括这些输入和额外回归，其成功不代表Phase17实施了它们，也不覆盖其正式阶段状态。本次终态同步仅修改17并推进18，不擅自校准全局实时缓存或重写其它阶段记录。

完整fast冻结了当时的并行CLI契约与格式状态。其一项契约依赖未被当时selector选入的联合测试文件，另一项工具crate存在三处assert格式差异；范围外源码未为本阶段门禁而修改。范围内派生报告只核对原9个必需步骤和137个成功测试（lib34、policy63、兼容40），没有重新执行或覆盖失败。当前宿主分别contract5/5、fmt退出0且相关输入稳定，但源码不同，因此保留原50步48/2、700/1的真实失败，不把针对性补验拼成相同候选一次全PASS。

## Risks / Notes

- 真平台成功联合为显式开发loopback HTTP，raw的loopback HTTPS故意不可达；不声称本轮完成生产TLS、部署、容量或完整浏览器旅程验收。
- 两次平台调用不合并、不正向跨请求缓存；授权deadline/body/并发有界，DB结果返回前复验到期。Phase30未来授权快照优化未提前实施。
- SELF按owner，grant与功能权限独立，组织ceiling始终叠加；处理人、前端UUID、CORS null和可见菜单都不增加服务端权限。
- Windows历史失败、平台strict53和原r2外层FAIL如实保留；Linuxlib/PG/raw成功各自独立表述。PG未因宿主并发漂移而无谓重跑，补验只复用同674候选与原ELF。
- 完整共享fast两项外部失败同样保留；Phase17相关必需项全PASS是本阶段完成依据，当前宿主外部补验不替代冻结候选来源，不宣称整仓全部门禁通过。
- 计划是代表性组件成本，不保证最大offset在大规模生产数据下恒定，也不构成SLA。没有新索引、全文或分区。
- 不改旧0001，不实现写入/提醒/assignees，不新增参考crate，不切流；后续Phase18需新的明确请求。

## Suggested Task Ledger Update

见[结构化更新](phase-17-ledger-update.md)。本次采用Phase17 `COMPLETED`，范围内剩余无；完整共享fast与平台strict失败作为明确限制保留。下一阶段Phase18“实现 Rust 工单写入、幂等和短事务”，`NOT_STARTED`，本次不实施。
