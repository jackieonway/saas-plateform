# Phase 11 执行与验证证据

日期：2026-10-16（Asia/Shanghai）。阶段：Phase 11，修复普通 OAuth 原子签发与实时失权。状态：**COMPLETED**。原台账未直接修改，采纳更新见末尾说明。

## Executed Phase

用户明确要求按台账执行 Phase 11。台账指针 11、详情 NOT_STARTED 与汇总一致，依赖 Phase 4、6 均已 COMPLETED。适用规则为用户提供的 task-ledger-executor 及 `D:/WorkSpace/rust/AGENTS.md`。本阶段只处理普通 OAuth 状态机、共享用户有效性、增量迁移与必要回归，不实施 Phase 12/13/28 等后续工作。

Git HEAD 为 `32956c4532bbd6eff53584fd0f5e9bb6e188437c`，包含此前各阶段的大量未提交工作。最终实际输入为 [phase-11-inputs.json](phase-11-inputs.json) 的 502 文件，SHA-256 `162fb6705bd9768264d0d14581477ecba56d915da8cbaaa208cc50e75dbc0ac2`；HEAD 不是本阶段全部工作内容的摘要。与 Phase 10 记录的 499 文件比较，恰好 12 路径变化，其中 3 文件新增，无移除。逐项边界核验见 [phase-11-boundaries.json](phase-11-boundaries.json)。

## Execution Summary

授权码、普通 refresh 与已批准设备码均在同一事务内完成条件消费、完整 access/refresh 写入和 TOKEN_ISSUED 审计。遵循 user → client → 一次性凭据的锁序，锁内重新读取客户端和凭据，严格检查消费、token 插入和签发审计插入的受影响行数。SQL 异常或触发器跳过写入都会使整个签发回滚。

普通 userinfo、introspect、refresh 及其它用户 grant 签发复用同一有效性契约：启用用户、当前直接成员、匹配租户与组织、租户及完整组织祖先启用且未到期、客户端启用。机器身份保留无用户和无组织上下文的独立规则。userinfo 继续严格验证 JWT；普通 introspect 保留按数据库 token 摘要识别 opaque access 的兼容契约。

另补齐 consent 请求消费与授权码插入的事务，防止写入失败烧毁请求；password grant 在获取用户锁后复核已验证的密码 hash，阻止等锁期间密码重置后凭旧密码签发；管理踢出在同一用户优先事务内撤销平台 sessions 与 OAuth token，消除两个提交间的刷新窗口。

## Deliverables

- [窄签发模块](../../backend/src/oauth_issuance.rs)、[OAuth 路由](../../backend/src/routes/oauth2.rs)与[踢出事务](../../backend/src/routes/oauth_admin.rs)。没有新增第三方依赖、crate 或根 workspace。
- [0003 增量迁移](../../backend/migrations/0003_oauth_issuance.sql)：既有行 storage_version=1，新行默认 2；新普通 OAuth access 使用随机 UUID v4 jti，access/refresh 仅保存摘要和必要元数据，jwt 均为 NULL，取消旧 hash 冲突 upsert。
- [10 项真实 PostgreSQL 专项](../../backend/src/oauth_issuance_tests.rs)，以及 CI 新增该组、既有 psr_ 4 项和 fast 的有效期单元入口。
- [协议、状态、升级与回退说明](../oauth-issuance.md)，根 README、现状架构、工程基线及 CI README 已同步。
- [建议台账更新](phase-11-ledger-update.md)。原台账保持未直接覆盖，原因见本页末尾。

0001/0002 校验和、session.rs 的 psr_ 协议、app_token.rs、Cargo manifest/lock、CI 快照 allowlist，以及前端/SDK/业务消费者的受管输入文件均与 Phase 10 记录逐项相同。测试环境使用新创建的隔离 PostgreSQL 服务和独立测试数据库，没有读取私有 `.env` 或生产库。

## Actual Verification

正式 PostgreSQL 与 fast 使用同一最终冻结快照，各自 launcher/报告退出 0，源文件在宿主、干净快照和容器执行前后逐项不变。完整索引见 [phase-11-runtime.json](phase-11-runtime.json)，不能把首轮 WIP 或 Windows 默认 ignored 计入正式通过数量。

| 检查 | 当前已取得结果 | 证据 |
| --- | --- | --- |
| Linux PostgreSQL 正式门禁 | **PASS：7 组 / 41 项 / 0 FAIL / 0 skip** | `target/phase-11/linux-postgres-r1/reports/postgres/postgres/postgres-result.json` |
| 普通 OAuth 专项 | **PASS：10/10**，消费/写入/审计故障及真实旧库升级全部执行 | 同上目录 `oauth_issuance_tests.log`，10 次自有库清理 assertions/database/pool 均 true |
| psr_ 与应用 token 回归 | **PASS：psr_ 4/4、authorization_isolation 3/3**，production_security 8/8 | 正式同一 PostgreSQL 组；另 platform_settings 4、entity_creation 6、ZIP registration 6 通过，总计 41 |
| Linux fast 正式门禁 | **PASS：24/24 步**，零必需失败/跳过 | `target/phase-11/linux-fast-r1/reports/fast/summary.json` |
| fast 细分 | 契约 362、部署单元 36、CI 政策 52、dev/Vite 19；Rust config 3、archive 16、maintenance 3、public contract 9、OAuth expiry 1、transport 8，全部 PASS | 各步原始 TAP/Cargo 日志和精确 counts 独立保留，不把工具/构建步骤计作测试数 |
| Windows 最终测试编译 | PASS，OAuth 10 函数编译成功，无警告 | `target/phase11/windows-no-run.log`、`windows-no-run-result.json` |
| Windows 默认测试 | 64 PASS / 0 FAIL / 68 ignored，exit 0 | `target/phase11/windows-default-tests.log`；数据库变量移除，ignored 不计数据库验收通过 |
| 最终全目标/全 feature Rust check | PASS，exit 0 | `target/phase-11/static-final/rust-check.log` |
| Rust 文档 | PASS，exit 0；2 个既有 rustdoc warning | `target/phase-11/static-final/rust-doc.log` |
| 新模块与专项测试格式、阶段 diff 空白 | PASS | `target/phase-11/static-final/result.json` |
| 全 crate 严格 Clippy | FAIL，exit 101；38 个既有 lint 错误，未关闭 lint | `target/phase-11/static-final/strict-clippy.log` |
| 全 crate fmt check | FAIL，exit 1；原有格式债务仍存在，未批量格式化无关代码 | `target/phase-11/static-final/strict-fmt.log` |

静态检查前后实际输入 SHA 均为上述最终 162fb670…，没有源文件漂移。Clippy 的 oauth2 诊断位于原有 check_oauth_rate 和原有 refresh scope 的 iter().any()，oauth_admin 诊断是原有列表元组类型；新 oauth_issuance 模块与专项测试没有新增 Clippy 诊断。文档警告位于原有 notifications.rs 正则注释和 extractors.rs 的尖括号说明。这些全仓债务与阶段必需门禁分别记录，不能宣称全仓所有检查均通过。

### 专项覆盖

| 场景 | 行为与实测要求 |
| --- | --- |
| 并发 refresh | 20 个同时请求，唯一成功；其余及重放 invalid_grant；最终只增加一套 token |
| 授权码与设备码 | 并发单次消费，重放拒绝；APPROVED 但已过期设备码同样拒绝 |
| 故障回滚 | 3 类凭据 × 8 阶段：消费异常/零行、access 异常/零行、refresh 异常/零行、TOKEN_ISSUED 审计异常/零行，共 24 子场景；移除故障后同一凭据能成功重试 |
| 实时失权 | 用户停用、租户停用/到期、当前组织停用/到期、祖先停用/到期、删除直接成员、停用客户端，共 9 类；检查 userinfo/introspect 及 refresh/code/device/password |
| 机器 token | 不受无关用户、成员、组织规则影响；可内省，不产生 userinfo 身份 |
| 独立撤销 | 并发相同 grant 的 access、jti 与摘要不同，单独撤销其中一个不影响另一个；随机 jti 使同秒签发也具有独立身份 |
| scope | refresh 不能超出原 grant 或客户端当前注册范围；省略 scope 也重新核对当前范围 |
| consent | 授权码 INSERT 异常或零行不消费请求，可重试且成功后仅一条 code |
| 密码等锁竞态 | 持有真实用户行锁，以 pg_stat_activity/pg_blocking_pids 观察已完成密码验证的请求阻塞，再提交密码修改；旧密码不能签发 |
| 真实旧库升级 | 在额外自有库先运行 0001/0002、写入无 jti 原文 JWT 和 refresh，再执行 0003；旧 token 可使用、刷新、更新撤销，历史原文不清除，新原文 INSERT 被 23514 拒绝 |

每个故障/状态场景使用独立 AppState，20 并发及重放不超过既有 30/min 限流。并发屏障与可观察 SQL 锁等待用于关键竞态，没有使用固定 sleep 掩盖结果。管理员 kick 与 refresh 的交错、客户端配置在锁等待期间变化、恢复启用后旧未撤销 token 恢复属于源码审查结论，本轮不把它们列作额外已实测场景。

## 先失败与兼容记录

首轮 WIP 冻结为 502 项，SHA `e9cbfc47c13f041e4cc6fb74dac765cc9c1cc404f8bc2e1e4a769a5fe2613e75`。其 OAuth 初稿 7 项为 6 PASS / 1 FAIL / 0 ignored，退出 1。失败是 legacy fixture 在迁移后的表内插入 JWT 原文却未显式设置旧格式版本，触发新明文禁止约束；最终 fixture 已标记 storage_version=1，并新增真实迁移前写入旧 token 的升级测试。原始失败、旧 7 数量、WIP driver 与快照保留在 `target/phase-11/linux-oauth-wip-r1`，不回填为最终 10，也不与最终通过相加。WIP 自有资源已清理，lease 退出 0。

早轮 Windows 编译发生在测试增补过程中，曾有未使用测试 import 警告；最终 no-run 已无这些警告。最终验收以同一 162fb670… 快照下的正式 PG/fast 报告为准。

## 工具、命令与资源

固定工具为 Node v24.18.0、npm 11.16.0、Rust 1.97.0；本机默认 Node 16 不用于验收。WSL Ubuntu 使用本地 Docker 29.1.3，工具镜像 `sha256:42e1b7fb5fea569fba2abada59030185286c9d642643cb7fa63ed7a5715b87b7`，PostgreSQL 17.6 镜像 `sha256:8745eeb23be1055bd80c88e509a5247110cce0f2496c7492d165f3d0fc1975cd`。

正式命令：

```text
node target/phase-11/run-postgres-linux.mjs docs/validation/phase-11-inputs.json D:/WorkSpace/rust/saas/rust-saas/target/phase-11/linux-postgres-r1 full
node target/phase-11/run-postgres-linux.mjs docs/validation/phase-11-inputs.json D:/WorkSpace/rust/saas/rust-saas/target/phase-11/linux-fast-r1 fast
node target/phase-11/run-static-checks.mjs
node target/phase-11/check-boundaries.mjs
```

PG 必需命令由 `scripts/ci/postgres.mjs` 对每组执行 `cargo test --manifest-path backend/Cargo.toml --locked --bin saas-platform-backend -j 2 <module> -- --ignored --test-threads=1 --nocapture`，严格核对清单数量，缺报告、零测试、失败或必需 skip 均不能通过。

容器资源带随机 `saas.ci.owner` 和本阶段 `saas.phase11.owner`，数据库使用 tmpfs 与私网，不发布宿主端口，不挂宿主源码、生产数据或 Docker socket。launcher 结束核验并移除自有容器/网络，保留原始报告和工具镜像，不执行全局 prune。正式 PG owner 为 `ci-1792135296605-00d05edd`，fast 为 `ci-1792135296605-e8d1bf08`；各自 cleanupErrors 为空、Phase11 owner 容器/网络空查 PASS，WSL lease 均正常停止且退出 0，执行前后 source SHA 相同。最终资源总查和 driver/hash 核对记录在 runtime 索引。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 同一普通 refresh 至少 20 并发兑换仅一成功；重放 invalid_grant | PASS | 真正发起 20 个并发 HTTP 请求，核对唯一成功和 token 数量；正式 OAuth 10 项之一 |
| 消费或新 token 写入失败不烧毁凭据/不产生半套 token；授权码设备码单次消费 | PASS | 三凭据 × 八故障阶段全部执行；恢复后重试、并发单次消费和 consent 失败回滚通过 |
| 停用/到期/删成员后相关用户 token 不能续发或返回有效身份 | PASS | 九类状态、三个身份/刷新端点和其它用户 grant 实测通过；client_credentials 独立正例通过 |
| 新 token 只存摘要/元数据，同秒不同 token 可独立撤销；psr_/app 回归通过 | PASS | 随机 jti、NULL jwt、独立撤销及真实旧库升级通过；psr_ 4、应用授权隔离 3 与安全 8 均在正式同快照执行 |

所有本阶段验收项 PASS，没有可继续执行的 Phase 11 剩余项。严格 Clippy/fmt 的既有全仓债务保留为明确限制，没有以阶段完成冒称其通过。

## Risks / Notes

- 本阶段采用动态实时状态规则。恢复启用或重新加入原直接成员后，尚未撤销/消费且未到期的普通凭据可以恢复有效；显式撤销和已消费状态不会恢复。没有自动增加全量身份事件永久撤销政策。
- 旧原文与无 jti token 经兼容版本保留，未来清理须有正式窗口；没有清库、改旧迁移或改 SQLx 历史。0003 后旧 issuer 不能混跑或作为直接回退构建，新约束下回退必须保留新存储协议和状态机。
- 事务错误可以回滚；网络超时/提交回执丢失无法证明未提交，不能自动重耗未知 refresh；没有返回旧 token 原文的幂等恢复接口，应重新授权。普通 refresh 保留现有按客户端 TTL 续发语义，psr_ 的绝对截止规则不改。
- 未访问生产库、推送镜像、部署外部环境或执行后续阶段；浏览器完整矩阵和生产运维验收不在本次后端变更的必需范围。

## Suggested Task Ledger Update

用户所提供的 [task-ledger-executor 技能](C:/Users/MECHREVO/.codex/skills/skills/task-ledger-executor/SKILL.md) 要求：`Do not directly overwrite the ledger unless the user explicitly asks.` 本次请求为执行 Phase 11，没有明确要求直接覆盖台账，因此原台账未修改；状态、摘要、决策及指针的完整建议替换块在 [phase-11-ledger-update.md](phase-11-ledger-update.md)。这一区分也避免将执行证据和未采纳的台账建议混为一谈。
