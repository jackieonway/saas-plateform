# Phase 12 执行与验证证据

日期：2026-10-16（Asia/Shanghai）。阶段：Phase 12，限制登录内存和密码计算成本。状态：**COMPLETED**。Phase 11 完成状态已采纳，Phase 12 终态按用户明确要求写回台账。

## Executed Phase

用户明确要求“更新为完成并执行下一阶段任务”，随后要求完成后标记台账。已把 Phase 11 完成结果采纳到台账，并仅执行下一阶段 Phase 12，依赖 Phase 4 已完成。遵循 task-ledger-executor、台账全局约束及 `D:/WorkSpace/rust/AGENTS.md`。Phase 13 未执行。

Git HEAD 保持 `32956c4532bbd6eff53584fd0f5e9bb6e188437c`，此前未提交工作保留。正式冻结为 [phase-12-inputs.json](phase-12-inputs.json) 的 **505 文件**，SHA-256 `9b016ace67d06993c31f31de8f622db1b52ed21548b9c0f9a84951bc5821684c`。相较 Phase 11 记录的 502 文件，20 路径变化、3 文件新增、无删除；见 [phase-12-boundaries.json](phase-12-boundaries.json)。源码 SHA 与工作树摘要分别记录，不能把 HEAD 当作未提交实施内容的摘要。

## Execution Summary

- 新增共享容量的来源准入与账号失败存储，按单调时钟和固定 TTL 回收，饱和拒绝新身份，不驱逐活跃预算。固定长度摘要键、过期索引与每次最多 32 条的清理批次控制单次操作成本。
- 所有 HTTP 密码路径复用同一异步 PasswordService，运行槽和总入场槽分别限制并发与队列；超载或截止返回 429/Retry-After。任务进入 blocking 计算后，即使调用者取消或返回超时，也继续持有两份许可直到实际结束。
- 登录/OAuth password 在查账号前消耗来源预算，输入长度有上限。共享 phone/email 匹配保留原业务语义，整次候选验证共用一个截止时间，超时不返回部分 ticket。
- 代理来源只根据真实 socket peer 与精确信任表解析；从 XFF 右侧剥离可信代理，删除原始转发头，handler 和审计只接收规范化身份。
- 异步改密结束后，在用户锁内重新比对已证明的旧 hash，冲突返回 409，避免等待计算期间覆盖并发重置。创建、重置和联系人初始化复用同一预算，既有业务和授权边界保留。

## Deliverables

| 交付 | 文件与边界 |
| --- | --- |
| 有界登录状态 | [login_limits.rs](../../backend/src/login_limits.rs)，AppState clone 共享同一 Arc，非全局 OnceLock |
| 密码服务 | [password.rs](../../backend/src/util/password.rs)，两份 permit 随 blocking closure 持有；同步 helper 仅保留启动 seed/测试 |
| 可信来源 | [client_ip.rs](../../backend/src/client_ip.rs)、main 的原中间件窄委托，保留其余 CORS/requestId 行为 |
| 请求接入 | auth/users/oauth2；onboarding 及 tenants/organizations 的既有调用链传入窄 PasswordContext |
| 配置 | [config.rs](../../backend/src/config.rs)、[AppState](../../backend/src/state.rs)，数值范围在启动时拒绝非法值 |
| 真实 HTTP/PG 回归 | [login_protection_tests.rs](../../backend/src/login_protection_tests.rs)，5 项，显式 TEST_DATABASE_URL 与自有数据库清理 |
| 协议与部署预算 | [登录保护说明](../login-protection.md)，根/架构/工程基线和 CI README 已同步 |
| 验证与状态 | 本报告、inputs/boundaries/runtime 索引；原台账按用户明确授权同步终态 |

全调用链盘点发现联系人管理员初始化也在 HTTP 路径 hash；实施前已把 onboarding 和 tenants/organizations 的必要参数传递补入 Phase 12 Scope。没有新增业务功能、迁移、第三方依赖或 crate，没有顺带实施 Phase 13 审计迁移或 Phase 28 事务优化。

0001/0002/0003、普通 OAuth issuance 模块、app_token.rs、Cargo manifest/lock、源码快照 allowlist、前端/SDK/业务消费者受管输入均不变。session.rs 的 psr_ 实现逐行等价，仅将已无生产调用的独立 revoke_all 包装器标为测试可用；生产继续组合既有 revoke_all_in_tx，不改 psr_ 协议。

## Actual Verification

正式 PostgreSQL 与 fast 使用同一上述冻结快照，必需检查全部通过；完整原始证据、命令与资源校验索引为 [phase-12-runtime.json](phase-12-runtime.json)。

| 检查 | 已取得结果 | 证据 |
| --- | --- | --- |
| Linux PostgreSQL 正式门禁 | **PASS：8 组 / 46 项 / 0 FAIL / 0 skip** | `target/phase-12/linux-postgres-r1/reports/postgres/postgres/postgres-result.json` |
| 新登录保护 HTTP/PG | **PASS：5/5**，加强后的累计截止与改密CAS真实锁等待均通过 | 同上目录 `login_protection_tests.log`，5次自有库 assertions/database/pool 清理均 true |
| Phase 11 原有回归 | **PASS：原7组41全部保留并通过** | OAuth10、psr4、授权隔离/app3、安全8、配置管理4、实体6、ZIP6 |
| Linux fast 正式门禁 | **PASS：27/27 步，零必需失败/跳过** | `target/phase-12/linux-fast-r1/reports/fast/summary.json` |
| fast 细分计数 | 契约362、部署36、政策52、dev/Vite19、Rust63，共532项通过 | Rust包含config4、limiter8、password8、clientIP6及原archive16/maintenance3/contract9/OAuth1/transport8；不把安装/工具/build步骤计作测试 |
| Windows 全目标/全 feature check | PASS，exit 0；仅既有 Config::from_env dead_code 警告 | `target/phase-12/static-final/rust-check.log` |
| Windows 默认测试 | **86 PASS / 0 FAIL / 73 ignored**，exit 0 | `windows-default-tests.log`；移除 DATABASE_URL/TEST_DATABASE_URL，ignored 不计正式 PG 通过 |
| Rust 文档 | PASS，exit 0；2 个既有 rustdoc 警告 | `rust-doc.log` |
| 新模块/专项与配置格式 | PASS | `new-module-format.log`，5 个文件实际 rustfmt --check |
| 阶段 diff 空白 | PASS | `phase-diff-whitespace.log` |
| 全仓严格 Clippy | FAIL，exit 101，38 个既有诊断 | `strict-clippy.log`；不关闭检查、不伪标通过 |
| 全仓 fmt check | FAIL，exit 1，保留既有格式债务 | `strict-fmt.log`，不批量格式化无关代码 |

上述 host 检查前后实际输入都为同一 9b016ace…，没有源漂移。prefreeze 曾因传入共享密码服务使 onboarding 两个函数超过参数数目而新增两项 Clippy 诊断，已用窄 PasswordContext 组合恢复原参数数量；最终恢复原有 38 项，新模块及专项没有新增 Clippy 诊断。早期集成中 LoginLimits 尚未包 Arc 时的 Clone 编译错误已修复；最终 AppState 克隆明确共享预算。

### 容量与行为回归口径

| 场景 | 实际断言 |
| --- | --- |
| 随机账号/IP | 100,000 个随机用户名/来源循环；配置 cap=128 时主表与过期索引每步均≤128，最终恰好128/128 |
| TTL/时钟 | 正常前进、回拨钳制、极大 u64::MAX 跳变；过期两表同步回收，活跃身份不被随机输入驱逐 |
| 来源并发与副本语义 | 64 线程争同来源只允许配置3次；Arc clone 共享，独立存储得到独立预算，重建存储清空。该单元证明进程内语义，不声称实际多副本共享配额 |
| 密码硬上限 | 可控 Condvar/oneshot 闸门证明 active/queued/admitted 上限；排队取消、排队截止、运行取消和执行截止后许可释放时机正确 |
| HTTP 饱和 | cap=16 时15个不存在账号进入真实查询/失败审计，再1000个随机账号全部429且不再增加审计；来源伪造无法换桶，OAuth共用来源预算 |
| 输入边界 | 默认321字节账号、1025字节密码拒绝；配置允许1024账号以及Unicode邮箱小写扩张时不误触内部限流key长度边界 |
| 共享 phone/email | 成功匹配不会惩罚其它密码不符账号；真正失败仍受输入及稳定 user-id 预算限制 |
| 累计验证截止 | 64个共享alias、复用默认hash，按3次真实验证校准且不提高默认总预算；必须已有1..63个proof完成后429，无返回ticket/memberships及持久OAuth凭据 |
| 混合密码负载 | current_thread Tokio 下24个 login/OAuth password/change/create 请求，观察真实Argon2 active后请求health/profile；响应均在2秒测试截止内，明确执行过载429并排空许可 |
| 密码生命周期与CAS | create/change/reset/OAuth正例，平台session/psr/app失效保持；改密完成异步计算后实际用户锁等待期间模拟管理员reset，409且不覆盖新hash/版本、不写成功审计 |

新增服务继续用 Argon2id v19/m19456/t2/p1，没有降低成本。健康检查的2秒是回归测试截止，不是生产延迟承诺。这里测量/断言的是预算和行为，没有给出吞吐、RSS 或任意历史 PHC 参数的容量 SLA。

累计验证超时保证没有返回部分凭据；既有按候选记录的 last_login_at/LOGIN 审计可能已经写入，本阶段不把登录流程描述为全部 SQL 原子回滚。全面管理审计语义与事务仍属 Phase 13。

## 先失败与修正记录

WIP r1：505 文件，SHA `58978bb0c39c163fdbd0cb798210abe196659d21457813d64e10eb5d94793d3f`，新 PG 5 项为 **4 PASS / 1 FAIL / 0 skip**，exit 1。失败是测试假定 reset 必须自动撤销存量普通 OAuth access；既有契约只持久撤销平台会话，普通 OAuth 无 session_version 绑定。本阶段未擅自改变普通撤销政策，最终测试明确验证平台session、psr_、app失效、旧密码不能新签普通OAuth，并断言存量普通access维持兼容，显式 revoke 后失效。

原 WIP 的1ms总截止仅证明首次验证超时；最终改为64alias、实测校准和已完成proof数量断言，检验累计截止而非每候选重置。WIP source、原5结果、失败日志、driver/freeze和清理保留于 `target/phase-12/linux-login-wip-r1`，不改写旧结果，也不和最终通过相加。

## 默认预算、兼容与限制

默认2个执行许可、8个等待许可，排队250ms、执行等待10秒；登录来源/账号共享4096条、600秒固定窗口，每来源60次准入、用户失败5次，输入320字节，候选验证总预算10秒。LOGIN_MAX_ENTRIES最小3保证正常单账号能预约source/input/user三条。当前Compose为单API；N副本各有自己的预算，总执行/等待、条目和可分散来源预算按N倍评估，重启清空计数。没有共享限流模式，也没有新增Redis或自动安装生产共享网关规则。

数据库旧 PHC 的参数保持兼容，单任务内存由 PHC 决定；本轮只覆盖项目默认参数。超时不会终止CPU计算，已有任务保留许可直到结束。队列容量和运行容量控制任务数，不能把它们直接当成任意PHC的RSS硬限制。

TRUSTED_PROXY_IPS仅列实际受控代理的精确地址；缺配置时按实际peer计数，经过同一代理的请求可能共用预算。多跳XFF从右向左剥离，坏链/重复链/超2048字节或32跳回落peer；无ConnectInfo为unknown共享桶。IPv4-mapped IPv6归一，原始Forwarded/XFF清除。不用任意客户端头作为限流身份。

生产密码强度、普通OAuth存储/消费、平台psr_和应用token协议保留；改密/重置撤平台会话不等于自动撤所有普通OAuth。未执行数据库迁移、生产操作、外部镜像发行或后续阶段。

## 工具与命令

Node24.18.0/npm11.16.0/Rust1.97.0；WSL Ubuntu本地Docker29.1.3；固定工具镜像 `sha256:42e1b7fb5fea569fba2abada59030185286c9d642643cb7fa63ed7a5715b87b7`，PG17.6镜像 `sha256:8745eeb23be1055bd80c88e509a5247110cce0f2496c7492d165f3d0fc1975cd`。默认PATH的Node16未用于验收。

```text
node target/phase-12/run-postgres-linux.mjs docs/validation/phase-12-inputs.json D:/WorkSpace/rust/saas/rust-saas/target/phase-12/linux-postgres-r1 full
node target/phase-12/run-postgres-linux.mjs docs/validation/phase-12-inputs.json D:/WorkSpace/rust/saas/rust-saas/target/phase-12/linux-fast-r1 fast
node target/phase-12/run-static-checks.mjs
node target/phase-12/check-boundaries.mjs
```

正式PG由manifest精确选择8组；每组 `cargo test --manifest-path backend/Cargo.toml --locked --bin saas-platform-backend -j 2 <module> -- --ignored --test-threads=1 --nocapture`。测试只用launcher新建的私网tmpfs PG，无宿主端口，不挂生产数据/宿主源码/Docker socket。资源带随机saas.ci.owner与saas.phase12.owner，仅按owner清理，保留日志和工具镜像，不执行全局prune。所有资源/lease/source核验见最终runtime索引。

正式PG owner为`ci-1792137875174-61a3d142`，fast为`ci-1792137875177-952b264e`。两次运行的宿主创建前、干净快照和容器内前后输入都为505项/9b016ace…，退出0。各自cleanupErrors为空、owner容器/网络审计PASS、WSL lease正常停止并退出0；最终所有Phase12标签的容器/网络/卷空查及driver/freeze副本哈希见runner最终强校验。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 大量随机用户名/IP不超配置上限，TTL可回收，时钟跳变可测 | PASS | 100k键/128容量、双索引一致、TTL/回拨/极大前跳、64并发和真实未知账号HTTP饱和均在最终门禁通过 |
| 并行Argon2和排队上限可断言，明确过载，普通API不被同步计算拖住 | PASS | 密码8单元证明双许可/取消/截止/入场前复制；24混合HTTP与current_thread健康/认证读通过，默认Argon2未降低 |
| 多实例预算/重启行为写清；需要共享预算时按实际模式实现验收，不虚称全局 | PASS | 当前单API模板明确进程级预算，独立存储/重建语义实测，N副本按N倍评估；本阶段不提供或宣称共享配额模式，未来实际共享需求另验 |

本阶段无剩余实施或必需验收项。所有37阶段的汇总/详情与下一指针在最终归档时重新核对；Phase13未启动。严格Clippy/fmt既有债务仍为实际FAIL，不能把阶段完成表述成全仓全部检查清零。

## Suggested Task Ledger Update

本次用户已明确授权更新台账。Phase 11 的完成记录已采纳；Phase 12 的汇总、详情三项勾选、证据、决策、变更记录和下一指针已同步为 **Phase 13 / NOT_STARTED**。未自动执行下一阶段。完整已采纳更新摘要见 [phase-12-ledger-update.md](phase-12-ledger-update.md)。
