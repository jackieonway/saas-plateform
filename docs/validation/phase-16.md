# Phase Execution Result — Phase 16

状态：**COMPLETED**。三项验收及全部必需门禁已通过，最终604项冻结输入为`ab0a4b25c8618a53f9ef46ec53a8d37fddbb154ff777abaf1171533519260ebd`。运行、原始日志及清理来源见[运行索引](phase-16-runtime.json)；外部并发变化和一处验证后文档更正分别保留，不覆盖原始冻结证据。

## Executed Phase

Phase 16：建立 Rust 工单服务并接管业务库基线。

依赖Phase1、4、6已完成。本次范围为独立crate、配置/错误/连接池、旧SHA-256历史与SQLx记录安全接管、live/ready和最小HTTP骨架。不实现完整CRUD、导出、平台授权、成员、提醒或UI，不将业务表并入平台库，不切换生产流量，不执行Phase17及后续。保留旧Node及旧SQL作为兼容输入。

执行终态：`COMPLETED`。本次已采纳Phase16结果，仅推进下一执行指针至Phase17 `NOT_STARTED`，保留并行Phase22/24已采纳结果。

## Execution Summary

当前实现将独立Rust crate置于现有[工单backend目录](../../business-apps/work-orders/backend/Cargo.toml)，保留自己的Cargo.lock、薄main和窄lib，不新增根workspace、不依赖平台实现crate。配置只读取明确的`WORK_ORDERS_DATABASE_URL`及Rust配置项，不自动加载`.env`，不回退使用平台`DATABASE_URL`。初始化完成前不创建HTTP监听器，错误以固定类别脱敏。

迁移使用旧Node相同的事务advisory锁。旧`0001_work_orders.sql`保持不可变SHA-256；在同事务随机参考schema内执行原始DDL，比较目标真实catalog、索引/约束、函数/trigger等，完全匹配后才记录SQLx原生SHA-384历史。旧库接管不重放目标CREATE，不改历史工单、UUID、归属、version、计数器或原迁移时间。校验/写入/参考schema清理在同事务提交，失败回滚且不开放ready。

**最终结果**：同604项候选的Linux fast **40步、668通过、0失败、0跳过**；PG门禁6步PASS，平台74项与独立工单15项均0失败/跳过；Linuxdoctest1项、实际Node-free ELF11项PASS。业务fixture有26条正常清理标记及恰好1条连接失败SETUP清理标记，全部确认删除owned库/关闭pool。Windowsfmt、all-targets/all-features strict Clippy、13单元及1doctest通过且逐文件与final等价。独立managed总审计PASS，原始来源、driver/freeze摘要、库/pool和容器/网络/卷空清单、WSL lease退出0均已核对。早轮r1/r2/r3各自状态与来源单列，不借其它聊天结果补足证据。

## Deliverables

- [独立Cargo crate](../../business-apps/work-orders/backend/Cargo.toml)和[锁文件](../../business-apps/work-orders/backend/Cargo.lock)，与原Node兼容文件同目录共存。
- [薄入口](../../business-apps/work-orders/backend/src/main.rs)、[窄lib与生命周期](../../business-apps/work-orders/backend/src/lib.rs)、[配置](../../business-apps/work-orders/backend/src/config.rs)、[错误](../../business-apps/work-orders/backend/src/error.rs)、[HTTP骨架](../../business-apps/work-orders/backend/src/http.rs)、[迁移/接管](../../business-apps/work-orders/backend/src/migrations.rs)、[catalog边界](../../business-apps/work-orders/backend/src/migrations/catalog.rs)。
- [配置模板](../../business-apps/work-orders/backend/config.rust.env.example)、[Rust基线操作说明](../work-orders-rust-baseline.md)、[backend兼容说明](../../business-apps/work-orders/backend/README.md)。
- [真实PG迁移测试](../../business-apps/work-orders/backend/tests/migrations.rs)，当前声明15项，默认ignored，必须经owned临时PG驱动执行；[受管业务PG入口](../../scripts/ci/work-orders-postgres.mjs)。
- Node-free直接ELF验证驱动位于`target/phase-16/raw-runtime.mjs`；[最终runtime报告](../../target/phase-16/linux-postgres-r1/raw-runtime-result.json)11项实际PASS，对应最终604项输入和独立Linux ELF，不使用早轮二进制摘要。
- 证据入口：[最终输入快照](phase-16-inputs.json)、[边界记录](phase-16-boundaries.json)、[运行索引](phase-16-runtime.json)、[结构化台账更新](phase-16-ledger-update.md)。运行索引关联92份原始证据，不改写旧轮次FAIL或WIP语义。

## Self-Check

保持台账三条验收原文。以下行为标准均据最终真实PG/runtime证据PASS；最终fast与独立总审计也已完成。

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 空库首启、同库再启、已有迁移记录与工单数据的首启、两个实例并发启动分别通过。 | PASS | 最终独立业务PG15/0/0，覆盖空库/重复、旧数据接管、fresh/legacy并发及Node/Rust共锁。完整suite清理26条正常+恰好1条SETUP标记全true，旧数据/计数器未改变；r2的14/1FAIL单列保留。 |
| 结构/索引/trigger/checksum不符时停止，迁移失败不开放ready；计数器和历史工单不变。 | PASS | 最终PG15项证明结构/索引/trigger/checksum/dirty/未知历史拒绝和失败回滚；直接ELF启动deadline内始终未开放ready，退出1，释放锁后重启ready200，前后历史/工单/计数器快照hash相同。fixture连接失败cleanup与静态脱敏修正已在该候选。 |
| Rust二进制独立启动，运行环境无Node依赖。 | PASS | 最终实际ELF11项PASS，实际进程PID1为/work-orders-api、UID10001，node/nodejs不存在、ldd无缺失；DB断开ready503/live200、恢复ready200，SIGTERM退出0，runtime owned库删除且cleanupErrors空。 |

## Baseline Adoption Contract

旧不可变SQL为[0001_work_orders.sql](../../business-apps/work-orders/backend/migrations/0001_work_orders.sql)，SHA-256：`a2e4224febd195fdd663a8954c3be4b76a8a06338521056c6809ebaa57d04090`。

共享旧锁为`pg_advisory_xact_lock(hashtextextended('work-orders-business-migrations',11))`。获得锁后重新读取当前状态，避免多个实例根据等待前快照接管。接受空库、精确旧`work_order_migrations`状态或两套合法历史已接管状态；非空无历史、未知/部分/dirty记录、checksum或结构不符均停止，不能自动伪造SQLx记录、删除旧记录或重建目标表。

参考schema按当前PG版本构造原始DDL，再比较表/列/类型/默认值/约束、索引及有效性、不可变身份函数源码与安全配置、trigger启用状态、RLS/policy、rules及继承。所有权/ACL属于部署策略，不声明固定schema指纹覆盖它们。迁移账号需要独立业务库建schema/表/函数权限；参考schema在同事务内删除，失败全部回滚。其它非系统schema永久关系也会触发专用业务库检查拒绝，不能将该机制用于平台共享库。

旧Node输入起点捕获见[legacy输入清单](../../target/phase-16/legacy-inputs.json)，共18项。最终[边界记录](phase-16-boundaries.json)已确认：只有backend README随Rust说明改变，其余17项（包括旧SQL、Node可执行源码、package/lock及原测试）逐字节保留。原Node兼容初始化只作为测试驱动，Node-free运行另行证明。

独立Cargo.lock含**221个第三方package**，仍采用既有Axum/Tokio/SQLx/Serde/UUID/tracing/SHA2/URL依赖家族。与平台冻结lock比较，**20个精确版本/source/checksum元组不在该平台锁图中**，不能描述为零新版本或所有依赖均相同。完整差异清单见[边界记录的dependencies](phase-16-boundaries.json)，离线`cargo metadata --locked --offline --format-version 1`生成的[license清单](../../target/phase-16/dependency-license-inventory.json)确认221项均有声明license，缺失0项。该清单仅是元数据和锁图核对，不宣称已完成漏洞、安全或全部许可证条款审计；本阶段没有为此改锁或新增平台实现依赖。

## Health, Configuration and Runtime Limits

| Endpoint / limit | 当前实现含义与验收边界 |
| --- | --- |
| `GET /work-orders-api/health/live` | 进程存活；返回`scope:process`，不表示DB或平台授权可用。 |
| `GET /work-orders-api/health/ready`、`/health`别名 | 初始化后校验业务库精确历史、必需表/列与启用身份trigger；scope为`business-database-schema`。运行期检查比启动完整catalog检查窄，200不证明所有schema属性或平台授权。 |
| `/tickets`及子路由、`/assignees` | 返回503 `BUSINESS_API_UNAVAILABLE`，不提供未实现业务数据或授权结果；未知路径JSON404。 |
| 连接与执行 | max connections默认10、范围1–30；连接获取/语句/锁等待默认各5秒、范围100–30000ms；空闲事务15秒；固定search_path。 |
| 生命周期 | 总启动默认30秒、范围1–300秒；正常启动完成前不监听；退出排空默认10秒、范围100–60000ms；ready查询默认2秒、范围100–10000ms。 |
| 配置与数据库TLS | URI必须明确host/user/独立业务database，拒绝未知或重复语义参数；不自动加载`.env`；TLS按URI sslmode，未指定时SQLx prefer，verify-full保留证书/主机名验证。此配置能力不等于本轮TLS部署实测。 |
| 错误与日志 | 公开错误为`{code,message}`，requestId为合法UUID；日志固定路由类别，不记录URL/query、请求体、token、数据库URL或完整source chain。静态脱敏与内含敏感底层source的回归在最终Windows/Linux单元通过，实际启动失败日志核查不含数据库URL或密码。 |

独立运行使用固定Debian bookworm-slim runtime镜像`sha256:97049525bbb2470c8694cd9cab09d92d5784c937a9de8d2dbe1aa28baed2074d`，不挂载工作树、不发布host端口、UID10001直接启动`/work-orders-api`，测试工具与探针在单独tools容器含Node。[最终实际报告](../../target/phase-16/linux-postgres-r1/raw-runtime-result.json)已证明该配置运行；runtime根文件系统可写，不能据此称为生产只读镜像交付，本阶段不新增生产容器路线。最终ELF为x86-64（machine62），53,559,984 bytes，sha256 `558623abc2a1f9979157363adb97efac8e485742abd2b2ced726fad6cbc05898`；早轮r3的不同ELF摘要不代替它。

## Actual Validation Results

| Run / gate | 已确认实际结果 | Evidence / scope |
| --- | --- | --- |
| runtime与工具预检 | readiness-only PASS，非服务验收 | [runtime预检](../../target/phase-16/runtime-preflight-r1.json)：Rust1.97.0、Node24.18.0、npm11.16.0、PG17.6；Debian12/glibc2.36无Node；[复制/权限预检](../../target/phase-16/copy-runtime-preflight-r2.json)PASS。 |
| CI policy前验 | 命令exit0；未冻结readiness-only | [前验result](../../target/phase-16/ci-policy-preflight-r1/result.json)，不借其计数声称最终CI/fast通过。 |
| r1业务门禁 | FAIL；执行前源摘要不一致；业务PG15项未运行 | [r1 runner](../../target/phase-16/linux-business-wip-r1/linux-runner-result.json)；inputs603项`b1b1b68d…`；cleanupErrors含无法验证尚未建立network归属，后续独立resourceAudit空与lease exit0；该原始错误保留，最终[总资源审计](../../target/phase-16/managed-final-audit-r1.json)空清单另列。 |
| r2独立业务PG | FAIL；14通过/1失败/0跳过；非平台74项 | [原PG结果](../../target/phase-16/linux-business-wip-r2/reports/postgres/work-orders-postgres/postgres-result.json)、[原日志](../../target/phase-16/linux-business-wip-r2/reports/postgres/work-orders-postgres/work-orders-migrations.log)。 |
| r2 driver/冻结状态 | SUPERSEDED；不能作终态验收 | [安全修复说明](../../target/phase-16/linux-business-wip-r2/driver-safety-superseded.json)、[runner](../../target/phase-16/linux-business-wip-r2/linux-runner-result.json)；raw runtime cleanup改为仅删除本次成功CREATE的owned库，driver hash变化拒绝在旧冻结下运行。 |
| r3业务PG/直接ELF（WIP） | PASS；PG15/0/0，Node-free运行11项PASS，runner exit0、cleanupErrors空 | [r3 runner](../../target/phase-16/linux-business-wip-r3/linux-runner-result.json)、[PG15结果](../../target/phase-16/linux-business-wip-r3/reports/postgres/work-orders-postgres/postgres-result.json)、[实际runtime](../../target/phase-16/linux-business-wip-r3/raw-runtime-result.json)。执行副本604项`45724601…`不变；hostDrift为OAuth render-edge/delivery及ci delivery三文件，finalHostSourceUnchanged=false。该WIP不替代后续修正后的最终门禁。 |
| 最终输入与边界 | PASS；604项`ab0a4b25c8618a53f9ef46ec53a8d37fddbb154ff777abaf1171533519260ebd` | [输入](phase-16-inputs.json)、[边界](phase-16-boundaries.json)：20项Phase16声明输入，17项旧兼容文件保持，8项辅助文档/模板，91项观测变化；这些计数对应不同集合不直接相加。Windows所验owned源码与final逐文件等价。 |
| 最终Windows独立crate | PASS；fmt、strict Clippy all-targets/all-features、13单元、1doctest均通过 | [Windows结果](../../target/phase-16/windows-final-r1/result.json)；均--locked/--offline（fmt无locked参数），0失败/忽略，输入与final逐文件等价。 |
| 最终完整PG门禁 | PASS；6步，平台74+独立业务15项均0失败/跳过 | [PG summary](../../target/phase-16/linux-postgres-r1/reports/postgres/summary.json)、[平台74](../../target/phase-16/linux-postgres-r1/reports/postgres/postgres/postgres-result.json)、[工单15](../../target/phase-16/linux-postgres-r1/reports/postgres/work-orders-postgres/postgres-result.json)。业务[原日志](../../target/phase-16/linux-postgres-r1/reports/postgres/work-orders-postgres/work-orders-migrations.log)26条正常fixture marker及恰好1SETUP marker均成功；本次明确执行平台回归，不继承旧PASS。 |
| 最终Node-free直接ELF | PASS；11项，0失败，runtime库删除、cleanupErrors空 | [实际报告](../../target/phase-16/linux-postgres-r1/raw-runtime-result.json)；ELF/PID1/UID、无Node、ldd、DB故障恢复、启动无ready、历史保持和SIGTERM均实际核对。 |
| 最终Linuxdoctest | PASS；1通过/0失败/0忽略 | [doctest结果](../../target/phase-16/linux-postgres-r1/reports/artifacts/doctest-result.json)，与同604项输入关联。 |
| 最终Linux fast/相关CI门禁 | PASS；40步、668通过、0失败、0跳过 | [fast summary](../../target/phase-16/linux-fast-r1/reports/fast/summary.json)、[runner](../../target/phase-16/linux-fast-r1/linux-runner-result.json)：Phase16独立crate fmt/check/strictClippy/13单元/doc五个必需gate均PASS；共享CI所选额外回归按实际scope保留，不宣称本次实施其它阶段。 |
| 最终源边界/归档/清理 | PASS；执行副本604项前后一致，原始driver和freeze归档摘要通过 | [managed审计](../../target/phase-16/managed-final-audit-r1.json)：两个runner均PASS、cleanupErrors空，独立inventory的容器/网络/卷全空，WSL lease ready/stopped且exit0；随机库删除、pool关闭与runtime清理来源已核对。 |
| 验证后单文档补充 | PASS；600/604冻结输入仍与当前host相同；1文档更正+3外部漂移分列 | [文档补充](../../target/phase-16/final-document-supplement.json)、[正式边界](phase-16-boundaries.json)：Phase16所有可执行及共享CI可执行字节保持，CI README仅替换固定步数描述；不宣称其余603项host全等。 |

## Source Snapshots and Concurrent Workspace Changes

HEAD为`32956c4532bbd6eff53584fd0f5e9bb6e188437c`，实际工作树内容按输入manifest。最初观测542项`3fdf4a523e50ec449f15642f736c4364f804002db9e044a2318fbcefc6a34a7c`；WIP r1为603项`b1b1b68d84c173b913fdac095331f5e1814d54ed693d15b9887027e9bb42b283`；WIP r2为604项`e170ff2e19242dd5787ee77575cf789f7f438b7a0d0be48ee047a71e7b162c07`；WIP r3为604项`45724601f2ac82cba798a4b7da06594dad00c23a8192d248c88d2bfba6d1aa31`；最终候选604项为`ab0a4b25c8618a53f9ef46ec53a8d37fddbb154ff777abaf1171533519260ebd`，执行副本为`target/phase-16/source-final-r1`。各尝试结果不可互换，两个最终Linux runner执行副本均前后不变，host漂移另行记录。

同一工作区还有并发聊天写入。观察范围包括OAuth相关`mini-services/oauth-playground/{backend,frontend,deploy,tests}`、`tests/phase24-oauth-browser.mjs`、`backend/src/oauth_playground_ui_tests.rs`，以及Socket.IO相关`tests/phase22-socketio.mjs`、`tests/fixtures/socketio-rust`、`examples/websocket`、平台public_contract等。这里只登记观察到/主执行者报告的漂移，不认定这些外部工作已完成，也不将其交付或验收归入Phase16。

最终boundaries已以observed-start和final逐文件区分Phase16声明文件与外部变化，共91项观测变化，未将外部工作计作本阶段交付。最终PG执行副本604项前后相同，host另观测`mini-services/oauth-playground/frontend/src/api.js`、其`frontend/tests/api.test.mjs`及`tests/phase24-oauth-browser.mjs`漂移，`finalHostSourceUnchanged=false`如实保留。不能据执行副本不变声称603项其余host输入全部相同；其它聊天已写入文件不能被本阶段回退覆盖。

其他聊天已在台账将Phase22/24记为COMPLETED；本次只推进Phase16→17，不改写这些详情或Phase22的SOCKET_IO_COMPATIBLE结论。旧CurrentPointer内`realtime_protocol_result: NOT_EVALUATED`缓存按Phase22 Notes要求保留，不能据此覆盖其独立实测，也不声称全局实时元数据已经一致。CI README固定“33步”是共享CI追加后滞留的文档文字，验证后仅替换为“合计步数以本次summary为准”，其hash由`2d7e404e…`变为`ad5c49b6…`，完整前后字节摘要见[单文档补充记录](../../target/phase-16/final-document-supplement.json)。该记录确认600项冻结输入与当前host完全相同，1处文档更正和3处外部OAuth漂移分别列出；17项legacy及本阶段所有可执行/共享CI执行字节不变，不把新的host摘要冒充已测候选。

## Earlier Failures and Repairs

- r1源在并发修改中偏离reviewed freeze，driver在创建业务门禁前拒绝。旧result保留FAIL及cleanupErrors，不把后续空resourceAudit覆盖成原runner无错误。
- r2`column_shape_drift_rejected`原始数据保持断言比较了包含新增`unexpected:null`列的`row_to_json`，与篡改前对象结构不同；计数器仍42。这一fixture断言失败不证明迁移写坏数据，也不等于整套通过。最终修复后的15项全量运行单独记录为PASS，原FAIL不改写。
- r2运行期间发现raw runtime清理必须只删除本次成功CREATE的数据库；安全修复优先于driver冻结，原driver另存，hash变化使该轮SUPERSEDED，不能为通过而继续使用旧不安全清理。
- r3之后完成fixture连接失败时owned库/pool清理和静态错误脱敏日志修正，并冻结604项最终候选；Windows检查、最终PG/ELF、fast40/668和managed总审计均通过。早轮WIP成功不用于代替已重跑的正式结果。

## Risks / Notes

- 业务库必须独立，迁移账号需适当DDL权限。接管不是备份或生产恢复工具；先在隔离副本预演。生产连接、域名/TLS、容量与真实流量均不在本次验收。
- 不兼容/部分/dirty历史应停止并报告，不能改旧SQL/checksum、删除迁移记录、重建旧库或自动回退到Node掩盖失败。提交结果未知时下次启动重读两套真实历史。
- 运行期ready检查有明确窄范围；不能据ready200推断平台依赖、完整API、全部catalog属性或旧Node health语义通过。业务端点当前503是阶段边界的一部分。
- 测试/构建tools或旧Node兼容测试可以含Node，实际Rust运行环境需单独证明无Node；不将预检镜像无Node当成service运行结论。
- 外部并发修改和历史未提交内容必须保留；不擅自认定外部聊天阶段状态。阶段完成后建议Phase17，但本次不实施。

## Suggested Task Ledger Update

见[结构化台账更新](phase-16-ledger-update.md)。本次已采纳Phase16 `COMPLETED`，阶段剩余无。下一阶段为Phase17“实现 Rust 工单授权及只读/导出契约”，`NOT_STARTED`，需新的明确指令。
