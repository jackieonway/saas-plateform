# Phase Execution Result

本阶段执行结论为 `COMPLETED`。r4 三项实际运行、五条原文验收、最终来源／边界、资源与专属 cache 清理均核验通过；本文件提供由统一流程同步台账的完整建议。

## Executed Phase

Phase 26: 建立简单门店 Rust API 与独立业务库。

## Execution Summary

已交付并实际验证独立 stores Rust crate／Cargo.lock、SQLx schema、门店资料／员工关联／日报与精确汇总、逐请求应用授权、受控成员校验、SQL scope 与版本／不可变归属边界，以及必要文档和 CI／真实平台联合验证。

最终候选为 [inputs-candidate-r4.json](../../target/phase-26/inputs-candidate-r4.json)，sourceRevision 为 `32956c4532bbd6eff53584fd0f5e9bb6e188437c`，inputsSha256 为 `2a00d4ecf05558d1b6019f73707160c216e1e4f5fd18f24ea95ee8752dfee5eb`，757个输入、59条闭包边；selectorSha256 为 `bb30381c1dfc59c100d261114174a4d6cfda5c4e30dca59280fc3af517d5df80`。r4 manifest 文件 SHA256 为 `bd0bec6e0d7c655fe0098f03f53b6a900fc0a79aa6427e7f5534f89b970a1608`。不以旧候选或 HEAD 替代最终工作树字节。

[r4 初始边界](../../target/phase-26/boundaries-initial-r4.json)及[最终边界](../../target/phase-26/boundaries-final-r4.json)均 PASS，43项变化、41项 owned 输入、hostDrift 为空，109份旧正式证据与发布前进行中台账保持。最终边界文件 SHA256 为 `0cba89cd698ef375ed262ca01771f7705b172fdb7cf86bf759b71ee84a392c3a`。前轮 r3 SQL 原 PASS 保持，不继承为 r4 正式验收。日报 audit INSERT 零行缺陷已修为必须恰好插入一行，否则业务事务回滚；修复后 reports.rs SHA256 为 `fc4a273ae1c2bc0b5c3c4632fbc128be207aed0f6b795ee6e72a82105690b55e`。r4 live 原日志实际确认 HTTP POST／PATCH 两次零审计行均回滚，移除 trigger 后正常请求各生成恰好一条 request-bound audit。

## Deliverables

- 独立 `business-apps/stores/backend` 的薄入口／窄 lib、配置、授权／成员 HTTP 适配、三张业务表与追加式审计、SQLx 迁移及真实 PG 回归。
- 明确 API DTO／状态／错误、金额与日期、版本及 unknown 读回契约，配置与策略示例，以及8份必要文档。
- 平台真实普通用户、三个独立数据库／专属非超级账号、两份业务 ELF 的联合 fixture，stores SQL／live runner 与严格 parser、fast／workflow 注册。
- 正式输出路径为 `docs/validation/phase-26.md`、`phase-26-inputs.json`、`phase-26-boundaries.json`、`phase-26-runtime.json`、`phase-26-ledger-update.md`，由 root 统一发布并同步台账；原始证据与完整指纹保留。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 独立cargo build/test，空隔离库初始化；门店、员工关联、每日记录真实API和汇总可用。 | PASS | 同候选完整fast61步／894项、门店六项strict门禁及54项library、SQL parent1／named8、live parent1／named16全部通过，覆盖空库／重复初始化与真实CRUD／汇总。 |
| 自己/本店/下属门店及平级/跨租户/组织硬上限按SQL执行，功能权限或grant缺失均拒绝。 | PASS | 真实SELF／ORG／SUBTREE／TENANT、平级／跨租户／组织硬上限及permission／grant独立拒绝均通过，scope在各资源SQL中执行。 |
| 工单token调用门店、stores token调用工单、客户端错绑、撤销成员/授权均拒绝。 | PASS | 真实平台／两份业务ELF／三库联合完成双向跨应用token、客户端绑定与下一请求撤销负向，source proof一致。 |
| 负责人/员工由平台受控成员协议校验，过期/停用/越界成员不能关联；没有平台身份表直连。 | PASS | 受控成员目录与负责人／员工关联、失效／过期／越界拒绝均通过，账号独立／非超级用户及跨库CONNECT拒绝已核对。 |
| 归属不可跨租户组织编辑；同店同日并发/重复提交不重复记录或计入汇总，版本冲突、十进制精度与Asia/Shanghai日期边界有回归；日志/错误不泄漏秘密。 | PASS | SQL8项及live16项实际覆盖不可变归属、并发唯一／精确汇总、版本／金额／上海日期／安全输出；HTTP零审计行POST／PATCH回滚与恢复后各audit1均实证。 |

## Actual Verification Summary

| Required run | Actual status | Steps／tests／named checks |
| --- | --- | --- |
| fast | PASS | child61步全部PASS，894 tests／894 PASS／0 FAIL／0 SKIP／0取消／0待办项，1931 filtered单列；[实际完整summary](../../target/phase-26/validation-runner/final-fast-r4/reports/fast/summary.json) |
| stores-postgres | PASS | 5 managed steps，Rust parent1 PASS／0 FAIL／0 SKIP，内部8项具名检查 PASS；[实际结果](../../target/phase-26/validation-runner/final-stores-postgres-r4/reports/stores-postgres/postgres-result.json) |
| stores-live-postgres | PASS | 7 managed steps，Rust parent1 PASS／0 FAIL／0 SKIP，内部16项具名检查 PASS；[实际结果](../../target/phase-26/validation-runner/final-stores-live-postgres-r4/reports/stores-live-postgres/postgres-result.json) |

最终计数只取这三个 fresh adopted runs。fast61步均exit0，mandatoryFailureCount／mandatorySkipCount为0；1931 filtered是选择性Rust命令排除数，单列而非执行PASS或SKIP。门店六步实际执行strict fmt、all-targets／all-features check和Clippy -D warnings、54项library、doc与locked binary build。既有CLI21项／TLS1项、OAuth Rust17项及其他manifest步骤也实际通过；范围按实际steps记录，不外推平台全仓strict债务已修复。

PG 单个 Rust test 与内部具名检查分列，live parent 的199 filtered也不计执行PASS或SKIP。三个 run 的源前后均为最终 r4 SHA，executionSourceUnchanged／hostSnapshotUnchanged为true，无hostDrift／cleanupErrors；各自containers／networks双空查询共6项，三个lease均ready／stopped／exit0。SQL owned库／池和live三库／三角色／池／三个服务进程清理布尔值全部true。

实际 live 数据为 stores6／staff4／daily11／grant54／grant audit15，账号独立／非超级用户／跨库 CONNECT 拒绝／grant只经HTTP均 true。transport 是明确的开发 loopback HTTP，未声称浏览器、TLS或Phase27验证。实际工具为 Node24.18.0／npm11.16.0／Rust1.97.0／Docker29.1.3／PG17.6；工具镜像 ID 为 `sha256:1912f7302185ed2c1af4df155d0d2784886c87984266ed981325575e415d71dc`，PG 镜像为 `sha256:8745eeb23be1055bd80c88e509a5247110cce0f2496c7492d165f3d0fc1975cd`。

两个实际 live ELF 均为64位／machine62：[stores-api](../../target/phase-26/validation-runner/final-stores-live-postgres-r4/reports/artifacts/stores-api) 67912520 bytes，SHA256 `e4335fc52fddd7772568a794c36682cfd3ff78515bad076bc0b37349da24868f`；[work-orders-api](../../target/phase-26/validation-runner/final-stores-live-postgres-r4/reports/artifacts/work-orders-api) 67902896 bytes，SHA256 `94d132037a7c4f7815f0fa9ff6ec4d1508cba337014cb12723f26b33e953fd3e`。

[专属cache清理](../../target/phase-26/validation-runner/cache-cleanup-final-r4.json)实际PASS，ownerVerified／volumeRemoved为true，attachedContainers／remainingOwnedVolumes为空，独立lease ready／stopped／exit0；文件SHA256 `fdc44caa1ae198084c9c7697feaa5b60cb6bd0a5068ccd49cd4ff868934211b3`。[managed采集](../../target/phase-26/managed-audit-r4.json)状态 `PHASE26_GATES_PASS`，9项检查全PASS，207 artifacts／141158275 bytes，artifactsSha256 `d0f791fd8fa05cc7abd81d6a6932cf70bffa6b83ef93f73a419b3b1b93003d5c`，其文件SHA256 `b4ff20903e89bc2a27e0bbf609675119c325d43576c4c37c8817bb99d73df957`。207为managed清单口径，不冒充正式runtime追加发布文件后的总数。

Windows native library 原网络 FAIL、r2／r3 的 Executor／Send／Acquire 编译 FAIL、r3 严格 Clippy FAIL，以及 live-r1 的 HTTP200 expected404 FAIL 各自保留原来源；修复后的开发观察和未采用 r1／r2 控制范围拒绝也分列，不用于继承正式 PASS。[development-index](../../target/phase-26/development-index.json) 当前19项，其中6项原 FAIL，包含独立 live-audit-r1 开发观察；完整 SHA256 为 `90d110b1cea84222211095690608e9da20eb17f65b486446c192730e0d542e8c`。完整失败与限制说明见配套报告的 Development Findings and Original Failures。

## Risks / Notes

- 成员见证有绝对单调截止，平台最后检查至业务 COMMIT 仍有跨库竞态；不承诺自动撤销已保存关联。
- unknown 写入先在当前有效授权下读回；无 Idempotency-Key／receipt 协议，日报重复／并发为409。
- 一组织一门店只是业务示例；manager／staff 身份不扩大其他资源 SELF。
- 完整 fast 只按实际 steps 覆盖范围报告；门店独立 strict 检查通过不等于平台全仓既有 strict fmt／Clippy 债务已修复。Windows r1的43 PASS／9 FAIL及r2的45 PASS／9 FAIL保持原观察。
- 只实施 Phase26 后端及必要验证，没有前端、公共 crate、支付／POS／复杂库存或生产部署。Phase27 已收到授权，待26终态核验后顺序启动。

## Suggested Task Ledger Update

### Phase Status Update

Phase 26: COMPLETED。

同步范围为 Phase26 详情／五原文验收、阶段摘要、当前执行指针、相关决策与变更记录。保护其他36阶段、109份历史正式证据与全局实时缓存。本次 Phase26 完成快照仍保留授权记录 `[26]`，只推进 Phase27 NOT_STARTED；Phase27 的已接收授权在后续独立启动时记录。正式发布后冻结本阶段全部原始证据及两份定稿文本，Phase27不回写。

### Output Summary

- 独立 stores-api／专属库账号、资料／员工／一店一营业日日报／数据库精确汇总与安全审计。
- 逐请求 Basic 内省＋应用 Bearer context、精确 permission＋grant、SQL tenant／组织硬上限／scope、受控直接成员与有界见证。
- 最终 r4 完整来源为 `2a00d4ecf05558d1b6019f73707160c216e1e4f5fd18f24ea95ee8752dfee5eb`；fast61步／894项、SQL5步／parent1／named8、live7步／parent1／named16全部实际PASS，最终边界／资源／lease／专属cache与managed采集通过。

### Completed Items

- 独立后端、迁移／schema检查、DTO与安全错误／恢复约定及配置文档交付。
- 五原验收全PASS，同源真实PG及平台三库／两业务ELF、受控成员／授权边界、日报并发／精度／日期与审计回滚完成。
- 项目CI及门店独立六项门禁完成，最终来源／边界、清理／专属cache、207项managed证据与原失败分别归档。

### Remaining Items

- Phase26业务实施与验收无余项。Phase27门店前端／双消费者浏览器实证属于后续阶段，按已收到的顺序授权启动。

### New Decisions

| Decision | Reason | Impact |
| --- | --- | --- |
| stores 使用独立 crate、库和专属账号，只调用既有受控平台 HTTP。 | 验证工单以外的第二业务消费者，保持依赖单向与身份边界。 | 不提前实施 Phase30 通用框架。 |
| 日报 tenant／store／businessDate 唯一，金额十进制字符串与 SQL numeric SUM。 | 防止并发重复与浮点误差，明确上海营业日。 | 重复为409；修改必须显式带 version。 |
| 新成员关联先 HTTP、后持业务锁，见证不续期；业务与安全审计同事务。 | 限定陈旧成员结果和锁占用，审计失败不能留下业务变更。 | 保留跨库最后检查至提交竞态；unknown 必须先读回。 |
| Phase26 终态先核验，再执行已授权 Phase27。 | 用户已提出顺序阶段请求，27依赖26完成。 | 不要求用户重发请求，不在26完成快照中冒记27已实施。 |

### New Open Questions

| Question | Impact | Suggested Owner |
| --- | --- | --- |
| 无新增阻塞问题。 | 本阶段五原验收与必需门禁已全部通过。 | root |

### Next Recommended Phase

Phase 27: 交付门店 Vite 微应用与双消费者契约实证。已收到顺序执行授权；Phase26 完成并通过终态核验后，以独立阶段启动，承接真实 stores 后端及两个消费者的协议差异。
