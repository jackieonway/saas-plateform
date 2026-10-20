# Phase Execution Result

本阶段执行结论为 `COMPLETED`。最终 r4 的完整 fast、隔离 PostgreSQL、真实平台联合三项均实际通过，最终来源／边界、资源与专属 cache 清理及 managed 证据采集已经核对。以下五条验收保持台账原文。

## Executed Phase

Phase 26: 建立简单门店 Rust API 与独立业务库。

Phase26 的执行结论为 `COMPLETED`，前置 Phase4／6／14／17／19 均已完成。本阶段只交付第二真实业务消费者的后端与必要 CI／真实平台联合验证；门店前端及双消费者浏览器实证属于 Phase27。Phase27 的顺序执行授权已收到，先完成并核验 Phase26 终态，再推进 Phase27，不需要用户重新提出请求。

## Execution Summary

当前源码提供独立 `stores-api` Rust crate、锁文件、配置和 SQLx 迁移，覆盖门店资料、员工关联、每日营业记录与数据库精确汇总。薄入口负责装配及进程生命周期，私有模块承载业务、SQL、平台 HTTP 适配与安全错误；不依赖平台或工单实现 crate，不增加公共后台 crate。

受保护请求逐次以 stores 机密客户端 Basic 内省当前应用 token，再以同一应用 Bearer 请求固定 stores context。校验应用、用户、tenant、当前组织、有效期、精确 permission 与独立 data grant。所有 list／count／detail／mutation／summary 的 SQL 同时包含 tenant、context 组织硬上限、grant 与资源自身 owner 条件；不以 manager、关联员工或另一个资源的 SELF 扩大授权。staff／report 按自己的资源动作检查目标 store，不暗加 store/read。

门店 owner 固定为创建人，manager 是独立的受控成员；staff owner 为经验证的被关联员工，createdBy 是操作者；日报 owner 为录入人。新 manager／staff 通过既有平台成员窄 HTTP 验证目标组织中的有效直接成员。成员见证绑定操作者、tenant、当前组织、resource／action、目标组织与用户，从 HTTP 开始使用不超过 `min(PLATFORM_AUTH_TIMEOUT_MS,3000ms)` 的绝对单调截止；等待池和业务锁消耗原窗口，锁内复核 version／旧 manager／归属与见证，不能续期。

三张业务表以复合外键维持 tenant／store／organization 一致，触发器拒绝 identity／owner／创建时间、关联 user 与日报营业日变化，业务 version 精确递增。业务变更与只含安全元数据的追加式审计同事务提交。独立审阅又发现日报 audit INSERT 影响零行时原实现未拒绝，已补充必须恰好插入一行的检查；否则返回安全数据库错误并回滚业务事务。r4 真实 HTTP POST／PATCH 零审计行回归确认两次业务变更均回滚，移除受控 trigger 后正常请求各留下恰好一条 request-bound audit。当前修复后 reports.rs SHA256 为 `fc4a273ae1c2bc0b5c3c4632fbc128be207aed0f6b795ee6e72a82105690b55e`。

迁移通过 detach 的专用物理连接运行，只有该连接临时使用 public-first search_path；成功和失败均关闭连接，取消也不把 advisory lock 或临时 search_path 返回业务池。业务池继续 catalog-first。readiness 核对 SQLx 历史及 catalog 指纹，拒绝 checksum 或 schema 漂移。

日报使用 `NUMERIC(18,2)`，API 金额始终为非负十进制字符串，拒绝浮点 JSON、指数与隐式舍入。SUM 在 PostgreSQL numeric 中执行，以文本规范化输出，可超过单笔金额上限。营业日为真实 `YYYY-MM-DD`，有效年份 0001–9999；省略时由服务端 UTC＋08:00 计算 Asia/Shanghai 当天，允许有效历史与未来日期。同 tenant／store／营业日唯一，重复及并发冲突返回409，不生成重复记录或重复汇总。

HTTP 提供固定安全错误、UUID requestId、no-store、64KiB JSON 限制及精确 Origin CORS。`Origin:null` 仍须应用 Bearer；无 Cookie 身份和凭据 CORS。没有 Idempotency-Key／receipt 协议；写入结果未知时必须在当前有效授权下读回核对，不自动重发 POST／PATCH／DELETE。

## Deliverables

- [独立 stores crate](../../business-apps/stores/backend/Cargo.toml)、[锁文件](../../business-apps/stores/backend/Cargo.lock)、薄入口与私有业务／授权模块。
- [SQLx 迁移](../../business-apps/stores/backend/migrations/0001_stores.sql)、[迁移与 schema 校验](../../business-apps/stores/backend/src/migrations.rs)、[真实 PostgreSQL 契约](../../business-apps/stores/backend/tests/postgres_contracts.rs)。
- [HTTP 明确契约](../../business-apps/stores/backend/API.md)、[后端 README](../../business-apps/stores/backend/README.md)、[配置模板](../../business-apps/stores/backend/config.example)、[受控策略示例](../../business-apps/stores/backend/policy.example.json)与门店／根 README、架构、工程基线的必要同步。
- [真实平台联合 fixture](../../backend/src/phase26_stores_tests.rs)及独立 stores SQL／live CI runner、严格结果 parser、fast／workflow／source selector 注册。
- [最终候选来源 r4](../../target/phase-26/inputs-candidate-r4.json)、[最终边界](../../target/phase-26/boundaries-final-r4.json)与[managed 采集](../../target/phase-26/managed-audit-r4.json)。前轮 r3 因后续日报零审计行修复不再采用；r4 三项门禁、最终边界和清理均已实际通过。正式 inputs／boundaries／runtime／报告／ledger-update 由统一发布流程产生，原始 target 证据与完整指纹保留。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 独立cargo build/test，空隔离库初始化；门店、员工关联、每日记录真实API和汇总可用。 | PASS | 同候选 fast61步／894项通过，门店独立 strict fmt／all-targets check／Clippy／54项 library／doc／locked build 六步通过；SQL parent1／named8 与 live parent1／named16 证明空库／重复初始化及真实 CRUD／汇总。 |
| 自己/本店/下属门店及平级/跨租户/组织硬上限按SQL执行，功能权限或grant缺失均拒绝。 | PASS | 真实 live 分别核对 SELF owner、ORG、ORG_SUBTREE、TENANT与组织硬上限及平级／跨租户；permission和grant独立负向通过。范围进入 list／count／detail／write／summary SQL，manager／staff不扩大其他资源SELF。 |
| 工单token调用门店、stores token调用工单、客户端错绑、撤销成员/授权均拒绝。 | PASS | 同源真实平台、两份实际业务 ELF、三库普通用户联合完成双向跨应用token／机密客户端绑定负向与下一请求成员／授权撤销；未使用mock替代这些链路。 |
| 负责人/员工由平台受控成员协议校验，过期/停用/越界成员不能关联；没有平台身份表直连。 | PASS | 真实成员目录、负责人／员工关联及失效／过期／越界拒绝均通过；门店只用既有受控成员HTTP。三库账号独立且非超级用户，跨库CONNECT严格拒绝，grant仅经正式平台HTTP配置。 |
| 归属不可跨租户组织编辑；同店同日并发/重复提交不重复记录或计入汇总，版本冲突、十进制精度与Asia/Shanghai日期边界有回归；日志/错误不泄漏秘密。 | PASS | SQL8项及live16项覆盖不可变归属、同日并发唯一／不重复汇总、版本、定点精度／大额SUM与上海零点边界、安全输出。另实际HTTP POST／PATCH零审计行均回滚，恢复后各audit1；三项无失败／跳过且资源／cache清理核验通过。 |

## Verification Layers and Source Provenance

最终候选为 `target/phase-26/inputs-candidate-r4.json`，evidenceClass 为 `ADOPTABLE_CANDIDATE`；正式运行的 evidenceClass 为 `ADOPTABLE_CANDIDATE_GATE`。日报零审计行修复及真实 HTTP 回归均包含在此757项来源中。sourceRevision 是 Git HEAD；实际未提交工作树字节由完整 inputsSha256 与逐文件清单识别，不从旧阶段继承测试状态。

| Source field | Actual value |
| --- | --- |
| sourceRevision | `32956c4532bbd6eff53584fd0f5e9bb6e188437c` |
| inputsSha256 | `2a00d4ecf05558d1b6019f73707160c216e1e4f5fd18f24ea95ee8752dfee5eb` |
| candidate manifest file SHA256 | `bd0bec6e0d7c655fe0098f03f53b6a900fc0a79aa6427e7f5534f89b970a1608` |
| selectorSha256 | `bb30381c1dfc59c100d261114174a4d6cfda5c4e30dca59280fc3af517d5df80` |
| source input files／closure edges | 757／59 |
| stores Cargo.toml SHA256 | `3f8f092ccaf6ba6859473a7e5772c99846295c8344a94d830d61d166ddb4e3c7` |
| stores Cargo.lock SHA256 | `b16f71d8e85a4537f53b58232cefbb5fd27b805813ab360416948111306742c9` |
| stores 0001 migration SHA256 | `5637961903960ef759ae00d97d23913dccc7e2fef800a369872ab54ff06fb4d4` |

r4 初始及最终边界均实际为 PASS：43 项 changedFromStart、41 项 ownedExecutableInputs、hostDrift 为空，109 份旧正式证据及发布前进行中台账保持；最终四项边界检查通过。最终边界 artifactsSha256 为 `153e7f49ea8cbcb3285c65876ded53bf9aefe91c9fc0d345e2c4197d4158fdb3`，最终边界文件 SHA256 为 `0cba89cd698ef375ed262ca01771f7705b172fdb7cf86bf759b71ee84a392c3a`。

| Required run | Adopted actual status | Actual steps／counts | Required evidence |
| --- | --- | --- | --- |
| fast | PASS | child61步全部 PASS；894 tests／894 PASS／0 FAIL／0 SKIP／0取消／0待办项；1931 filtered 单列 | [outer 终态](../../target/phase-26/validation-runner/final-fast-r4/run.json)、[完整 child summary](../../target/phase-26/validation-runner/final-fast-r4/reports/fast/summary.json)，含每个实际 argv／exit／timeout及日志 |
| stores-postgres | PASS | 5个 managed steps；Rust parent 1 PASS／0 FAIL／0 SKIP，内部8项具名检查全部 PASS | [outer 终态](../../target/phase-26/validation-runner/final-stores-postgres-r4/run.json)、[实际 SQL 结果](../../target/phase-26/validation-runner/final-stores-postgres-r4/reports/stores-postgres/postgres-result.json)、[原始 SQL 日志](../../target/phase-26/validation-runner/final-stores-postgres-r4/reports/stores-postgres/stores-postgres-contracts.log) |
| stores-live-postgres | PASS | 7个 managed steps；Rust parent 1 PASS／0 FAIL／0 SKIP，内部16项具名检查全部 PASS | [outer 终态](../../target/phase-26/validation-runner/final-stores-live-postgres-r4/run.json)、[实际 live 结果](../../target/phase-26/validation-runner/final-stores-live-postgres-r4/reports/stores-live-postgres/postgres-result.json)、[原始 HTTP／业务日志](../../target/phase-26/validation-runner/final-stores-live-postgres-r4/reports/stores-live-postgres/phase26_stores_tests-real_platform_stores_and_cross_app_isolation.log) |

具名 SQL 与 live 检查数由冻结 parser 声明及实际输出共同确定。选中的单个 Rust test、内部具名业务检查、完整 fast 测试与 filtered 数分开列示，不相加冒充同一测试口径。live parent 原日志有199 filtered，它们不计执行 PASS 或 SKIP。原生 `--no-run` 只证明编译，不作为真实 PG 执行。

fast 的61步与894项来自本轮实际 summary，mandatoryFailureCount／mandatorySkipCount 均0，stepStatusCounts 为61 PASS／0 FAIL／0 SKIP／0 NOT_RUN。1931 filtered 是各选定 Rust 命令的排除数，不属于跳过或执行通过。实际覆盖包括独立 npm 安装、SDK drift、现有前端 build／lint／typecheck、契约378项、工单 Vite11项、部署单元38项、CI策略128项、dev／Vite20项、平台登记的选择性 Rust 单元、通知／工单门禁、门店六项门禁、OAuth Rust17项与平台CLI21项及TLS1项。门店 library 原始日志确认为54 PASS／0 FAIL／0 ignored／0 filtered。

门店六项的实际参数分别包含 fmt --all --check、check --all-targets --all-features、Clippy --all-targets --all-features -- -D warnings、test --lib --all-features、doc --no-deps --all-features、独立 locked binary build；其余参数与命令在 summary 逐项记录。这是本阶段实际 manifest 范围的完整 fast，不代表平台全仓既有 strict fmt／Clippy 债务已修复。

两个 PG run 实际工具为 Node24.18.0、npm11.16.0、rustc1.97.0／cargo1.97.0，Docker29.1.3。工具镜像实际 ID 为 `sha256:1912f7302185ed2c1af4df155d0d2784886c87984266ed981325575e415d71dc`，Linux amd64；PG17.6 实际镜像为 `sha256:8745eeb23be1055bd80c88e509a5247110cce0f2496c7492d165f3d0fc1975cd`，owned tmpfs 且无 host ports。工具容器实际 capDrop ALL、capAdd 空、no-new-privileges，未为测试放宽 capability。

| Actual live business ELF | Bytes | SHA256 | Verified format |
| --- | --- | --- | --- |
| [stores-api](../../target/phase-26/validation-runner/final-stores-live-postgres-r4/reports/artifacts/stores-api) | 67912520 | `e4335fc52fddd7772568a794c36682cfd3ff78515bad076bc0b37349da24868f` | ELF64／machine62 |
| [work-orders-api](../../target/phase-26/validation-runner/final-stores-live-postgres-r4/reports/artifacts/work-orders-api) | 67902896 | `94d132037a7c4f7815f0fa9ff6ec4d1508cba337014cb12723f26b33e953fd3e` | ELF64／machine62 |

这些是 live 实际构建并执行后留存的原始业务 ELF，已独立读头及核对字节摘要。managed 采集状态为 `PHASE26_GATES_PASS`，9项检查全部 PASS；公开 artifacts 为207项，共141158275 bytes，artifactsSha256 为 `d0f791fd8fa05cc7abd81d6a6932cf70bffa6b83ef93f73a419b3b1b93003d5c`，managed-audit-r4.json 文件 SHA256 为 `b4ff20903e89bc2a27e0bbf609675119c325d43576c4c37c8817bb99d73df957`。207只指 managed artifacts 清单，不冒充后续正式 runtime 追加发布文件后的总数。

## Actual HTTP, Database and Audit Evidence

r4 live 使用真实平台入口、两个独立业务服务和三个专属非超级数据库账号，通过明确的本地开发 loopback HTTP 执行普通用户业务与授权负向。transport 原记录为 `explicit-development-loopback-http`，不将其描述为 TLS、浏览器或 Phase27 opaque iframe 验收。grant 写入全部经正式平台 HTTP，SQL只用于受控测试准备与结果断言；跨数据库 CONNECT 以严格拒绝断言核对。

实际数据库断言为 stores6行、staff4行、daily11行、grant54行、grant audit15行，distinctAccounts／nonSuperuser／crossDatabaseConnectDenied／grantWritesOnlyViaHttp 全部 true。16项具名检查覆盖真实登录／机密内省、CRUD／汇总、SELF／ORG／SUBTREE／TENANT及硬上限、permission与grant独立拒绝、双应用token／客户端绑定、下一请求撤权、受控成员与失效／过期／越界、不可变归属、日报并发唯一、版本／金额／上海日期、安全输出。

原始 live 日志第20行保存 `STORES_LIVE_AUDIT_ZERO_PROOF`：createRolledBack、updateRolledBack、triggerRemoved 均 true，移除 trigger 后 createRetryAuditCount 与 updateRetryAuditCount 各为1，且分别绑定实际 requestId。此项属于当前16项业务检查内的实际回归证明，不额外增加具名测试计数。

## Development Findings and Original Failures

开发观察有自己的来源与终态，不进入最终三个 adopted runs 的计数；不同快照的 PASS 不替代候选 r3 门禁。

| Observation | Preserved actual outcome | Interpretation |
| --- | --- | --- |
| [Windows library r1](../../target/phase-26/auth-local-r1/cargo-test-lib.log)、[r2](../../target/phase-26/auth-local-r2/cargo-test-lib.log) | 原结果均为 FAIL，exit101；r1为43 PASS／9 FAIL，r2为45 PASS／9 FAIL，回环 HTTP 相关测试失败。 | 原生探针中 std TCP 连接超时；修正版探针的 direct reqwest 也超时。首版探针另有无 Tokio reactor 的自身 panic，不能把该 panic 当作网络根因。未证明是应用代理缺失，不弱化测试或把本机 suite 改写为通过。 |
| [早轮 PG 编译 r2](../../target/phase-26/auth-local-r2/cargo-postgres-no-run.log) | FAIL，exit101，Executor 生命周期泛化不足。 | 编译失败单列，未执行 PG 测试。 |
| [早轮 PG 编译 r3](../../target/phase-26/auth-local-r3/cargo-postgres-no-run.log)、[Clippy r3](../../target/phase-26/auth-local-r3/cargo-clippy.log) | 原 FAIL 保持；分别包含 Send／Acquire 泛化错误、test module 后存在 items 的严格 lint。 | 随后修复专用迁移连接的 run_direct 调用与目录模块顺序；r4 编译／Clippy 开发结果仍按自己的来源保存，不混入正式验收。 |
| [真实 live r1](../../target/phase-26/validation-runner/live-r1/run.json)、[原始日志](../../target/phase-26/validation-runner/live-r1/reports/stores-live.log) | outer 与 stores-live 步骤 FAIL，exit101；原日志为 HTTP status200 expected404。 | 开发记录确认原fixture对集合端点的状态预期过严，修正断言后重跑；保留原差异及 assertionsOk=false，不误记为迁移失败。原日志另记三个业务／平台进程停止、三库及角色删除、池关闭。 |
| 候选控制 r1／r2 | 未采用；控制脚本允许范围清单遗漏被拒绝。 | 涉及 .github README、postgres／test 清单；源码 SHA 与 r3 相同也不抹除原拒绝。r3 修正了控制允许范围，后又因日报零审计行修复停用，不能把任一旧候选写成最终采用。 |
| [前轮 r3 SQL](../../target/phase-26/validation-runner/final-stores-postgres-r3/run.json) | 原终态 PASS，sourceProof 前后不变，资源空查询及 lease 原结果保留；未采用。 | 该轮使用 `f18a10afbc80ac8394706d580def0f9789bdd21dbbc88b192f6f1d8e345f3ce1`，早于日报零审计行修复与新增真实 HTTP 回归。其 PASS 不改写，但不能代替 r4 正式三项。 |

[development-index](../../target/phase-26/development-index.json) 共19项观察，其中6项原始 status 为 FAIL；包含修复后的 live-audit-r1 开发回归。其完整文件 SHA256 为 `90d110b1cea84222211095690608e9da20eb17f65b486446c192730e0d542e8c`，登记的所有文件路径已确认存在。开发 check／units／sql／live-r2、局部 CI 与编译结果保留各自原始报告，不继承为正式 PASS；最终三项只采用 r4 fresh runs 的实际日志与终态。

## Cleanup and Publication Boundary

r4 fast／SQL／live 各自执行两个按 owner 标签限定的精确资源查询，containers／networks 均为空，共6项空查询；三个 WSL lease 均 ready／stopped／exit0。三 run 的 sourceProof 前后均为 `2a00d4ecf05558d1b6019f73707160c216e1e4f5fd18f24ea95ee8752dfee5eb`，executionSourceUnchanged／hostSnapshotUnchanged 均 true，hostDrift 与 cleanupErrors 均为空。SQL fixture 的 assertionsOk／databaseRemoved／poolClosed 全 true；live 的平台／门店／工单三库、角色、池清理及三个服务进程停止均 true。

[专属 cache 清理](../../target/phase-26/validation-runner/cache-cleanup-final-r4.json)实际 PASS：核验 owner 后删除 `phase26-validation-cargo-20261018`，ownerVerified／volumeRemoved 为 true，attachedContainers／remainingOwnedVolumes 均为空，其独立 WSL lease 为 ready／stopped／exit0。该452 bytes报告 SHA256 为 `fdc44caa1ae198084c9c7697feaa5b60cb6bd0a5068ccd49cd4ff868934211b3`。PG run 中为有界重跑暂留 cache 的原记录保持；最终清理由此独立报告证明，未改变其他全局缓存。

managed 采集已重新核对 candidate 物理文件集合、own-selector、闭包、宿主／冻结字节、最终边界、原始 PG 日志解析及证据指纹，并保护全部109份旧正式文件。其他36阶段和全局实时缓存保持；台账终态由 root 统一生成、审阅与应用，Phase26 完成快照仅推进27 NOT_STARTED。正式发布后冻结本阶段原始证据和两份定稿文本，后续 Phase27 不回写这些文件。

## Risks / Notes

- 平台成员／grant 最后检查与业务 COMMIT 之间仍有跨库竞态；有界成员见证不提供跨数据库事务或自动撤销已保存关联。
- 网络断开、取消或 unknown 错误不能证明未提交；必须先授权读回核对，没有通用幂等 receipt 保证。
- policy.example.json 是含明确占位应用 UUID 的待合并策略片段，不自动建立应用、菜单、角色或 grant；普通业务用户无需 grant 管理权限。
- 完整 fast 只按本轮实际 steps 与日志陈述覆盖范围；门店独立 strict fmt／Clippy／build／library／doc 的结果不能外推为平台全仓既有 strict fmt／Clippy 债务已经修复。
- 一组织一门店是示例业务约束，不固化到平台模型。当前阶段未实施前端、支付／POS／库存／排班／审批、公共 crate、镜像发行或生产部署。

## Suggested Task Ledger Update

### Phase Status Update

Phase 26: COMPLETED。

### Output Summary

- 独立 stores-api／Cargo.lock、业务 schema、真实门店／员工／日报／精确汇总 API，以及受控授权和成员适配已落地。
- r4 为757项／59闭包边，完整 inputsSha256 为 `2a00d4ecf05558d1b6019f73707160c216e1e4f5fd18f24ea95ee8752dfee5eb`。完整fast61步／894项、SQL5步／parent1／named8、live7步／parent1／named16全部实际 PASS；最终边界、资源／lease与专属cache清理通过。

### Completed Items

- 独立后端、迁移／schema、DTO／错误／日期金额／恢复约定与必要文档交付。
- 五条台账原文验收全部 PASS，三项同源运行、54项门店library、真实三库／两业务ELF与受控成员／授权负向完成。
- 日报零审计行回滚回归、最终边界、资源／lease／专属cache清理及207项managed采集完成；原生网络、早轮编译及live失败分列保留。

### Remaining Items

- Phase26 业务实施与验收无余项。后续门店前端及双消费者浏览器实证归 Phase27，按已收到的顺序授权启动。

### New Decisions

| Decision | Reason | Impact |
| --- | --- | --- |
| 第二业务保持独立 stores crate／业务库／专属账号，复用现有受控平台 HTTP。 | 用真实消费者验证契约，不提前抽取公共后台框架。 | Phase30 使用两个消费者差异再收敛。 |
| 日报重复提交返回409，未知提交先读回。 | 一店一日唯一约束保证汇总不重复，当前无新 receipt 协议。 | 调用方不得自动重放 mutation。 |
| 成员见证从 HTTP 开始使用绝对单调截止。 | 锁等待不能延长旧成员校验有效期。 | 明确保留最后检查至 COMMIT 的跨库竞态。 |

### New Open Questions

| Question | Impact | Suggested Owner |
| --- | --- | --- |
| 无新增阻塞问题。 | 本阶段五项验收与必需门禁已全部通过。 | root |

### Next Recommended Phase

Phase 27: 交付门店 Vite 微应用与双消费者契约实证。该阶段已收到顺序执行授权；在 Phase26 COMPLETED 且终态验证通过后启动，Phase26 完成快照中的 Phase27 保持 NOT_STARTED。
