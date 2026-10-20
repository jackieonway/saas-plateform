# Phase 30：授权查询链与 Rust 能力边界验收

日期：2026-10-19（Asia/Shanghai）。结论：**COMPLETED**，五项原验收均 PASS。仅执行 Phase 30；未执行 Phase 31、外部镜像发行或生产部署。台账授权本阶段实施、验收和终态同步，故摘要、详情、决策及下一指针一并回填。

## 来源与范围

- HEAD：`32956c4532bbd6eff53584fd0f5e9bb6e188437c`。HEAD 仅标识提交起点，实际代码包括既有未提交工作；未提交、重置或覆盖其他阶段内容。
- 原生产输入 851 项，SHA-256：`300df64b7a9cb1fcff4aa3371f8a657e0149cf0ad10cb8c4bbc9905124012456`。只读副本为 `target/phase-30/source-before`。
- 最终候选 862 项、45 个变化路径、无删除，SHA-256：`178ffd6ed48417df5769fc9126c5afad4013e9d2aa2f5f631d20a3c1b7c98b43`。逐文件记录见 [inputs](phase-30-inputs.json)。文档及被既有 source filter 排除的公开 env 示例另按最终输出核验，不能声称该摘要覆盖所有仓库文件。
- 正式运行：`target/phase-30/formal-r1/result.json`，分类 `ADOPTABLE_CANDIDATE_GATE`，31/31 步 PASS；执行快照与宿主源码前后均一致。证据索引、实际 argv、工具版本、日志 SHA、ELF 和清理见 [runtime](phase-30-runtime.json)。这是一组 Phase30 后端门禁，不冒称完整 fast/browser/delivery 矩阵。
- Rust 1.97.0、Node 24.18.0、npm 11.16.0；Linux 工具镜像固定为 `sha256:1912f7302185ed2c1af4df155d0d2784886c87984266ed981325575e415d71dc`。PostgreSQL 使用自有私网、无宿主端口、tmpfs 与真实 SQL 夹具；TCP 探测只证明传输可达，不作为数据库成功证据。
- 冻结旧平台 ELF：SHA-256 `32eaabd58b6ea6e16e61a617469735b722c10dcf8f25c5fa17fb30612f8d9393`。它来自原生产字节加两个测试覆盖文件的 baseline-r4，生产构建排除测试模块。正式运行另外编译当前平台 ELF，并实际启动；两者身份在 runtime 中分别记录，未把缓存旧 ELF 当作新产物。

## 实现与协议决策

平台仍为一个 crate。窄 library 只导出 paging、应用绑定/成员组织上限纯策略和有界 snapshot 编排；HTTP、SQL、数据库行、凭据、context 渲染留在 executable 的私有 use case/store。没有新增依赖、迁移、共享数据库模型、根 workspace 或通用 repository。

`POST /api/apps/{appCode}/authorize` 验证机密客户端 Basic、application_id、应用 token 的签名/受众/身份/有效期/会话、登记的 resource/action、permission 和独立身份绑定 grant；响应前重新验证客户端凭据。wrapper 为 `contractVersion=1.0`、introspection 和原 1.0 context。两消费者继续独立验证响应，不进行失败自动回退；缺操作权限仍为 `OPERATION_DENIED`，缺独立数据授权仍为 `DATA_DENIED`，避免把上游 403 全部错误归类。

context 用一个聚合 SELECT 取得一致 MVCC 投影，再通过新的独立语句读取完整投影、SHA-256 revision 和 token/session 状态。内部 revision 包含绑定及运行元数据，公开 contextVersion 保留原算法；二者都不是认证凭据。变化最多重建一次，连续变化或预算耗尽失败关闭。没有在旧 REPEATABLE READ 快照中伪做新检查，也没有跨请求正向缓存。未引入全局版本行/触发器，避免为所有授权写入增加串行锁点。

成员查找复用请求内投影，在结果返回前复核最新授权；目标组织及目标应用可达性也再次读取。工单与门店保留实际共用的受控成员 HTTP/DTO、用途登记、组织硬上限及各自的短期 witness。两方的错误模型、领域规则和业务 SQL 独立，当前不足以证明引入公共 crate 的收益。冻结契约见 [协议说明](../application-authorization-contract.md) 与 [实际消费者差异](../stores-member-contract.md)。

分页保留非法语法的既有默认值，合法正数 page 封顶 1,000,000，size 保持 1..200/default 10，offset 使用 checked arithmetic。i64::MAX 不再溢出；超出 i64 解析范围仍按原非法输入默认 page=1。成员接口保留自身严格 400 边界。

## 实际验证

正式 31 步包含 **153 个实际 Rust 测试通过，0 FAIL、0 ignored**；具名检查是父测试内部语义证据，另列，不叠加为 Rust 测试数。

| 类别 | 实际结果 |
| --- | --- |
| 公共 library 集成测试 | 4 PASS：分页、应用绑定、组织上限/SELF、快照重建/撤销/持续变化 |
| 新鲜夹具性能测试 | 1 PASS；combined/legacy 各独立数据库生命周期，保留完整原始时延数组 |
| 原平台 ELF 对照与当前 ELF | 1 PASS；两真实应用、四种 scope、身份/菜单/grant/硬上限/contextVersion、已提交 grant 撤销、role/user/org/tenant/app/token/session/client 失效、错误受众/凭据、公开 hash 拒绝、极大分页 |
| 独立业务 crate 单元测试 | 工单 57 PASS，门店 55 PASS；包含 combined transport、版本/形状拒绝、错误码与不自动回退 |
| 真实三库/三进程联合 | combined 1 父测试/16 具名检查 PASS；legacy 1 父测试/16 具名检查 PASS；独立账号跨库连接被拒 |
| 工单真实平台回归 | 只读 1/13、写入幂等 1/12、成员分配 1/10、提醒恢复 1/12，均 PASS；含平台失联、当前 grant/会话/成员失效及 HTTP 与业务锁边界 |
| 新增 CI 协议步骤 | 1 PASS，独立验证实际结果、预算及数据库/角色清理标记；已进入 postgres 计划 |
| 选定平台单元 | config 7、公共契约 9、成员策略 1、data_grants 11，均 PASS |
| 静态/构建/文档 | 当前平台 ELF、两业务 ELF、平台 all-target check、两业务 fmt/严格 Clippy/doc、平台窄库严格 Clippy、平台 doc 均 PASS |
| Windows 补充政策检查 | 205 PASS / 0 FAIL / 0 skip；CI 与部署政策，日志 `target/phase-30/ci-policy-r2.tap` |

旧/新 context 对照仅剔除按请求时刻生成的 expiresAt，并按 ID 归一化菜单同级顺序。原 `tree::build_tree` 遍历 HashMap，顺序本就不稳定；对照仍检查每个节点、字段、层级及完全相同的 contextVersion，没有删除菜单语义断言。

确定性交错通过“读取快照 → 提交独立 grant 删除 → 新语句 revalidate”证明已提交变化可见；未用 sleep 猜撤权时机。用户、组织、租户、应用及客户端变更会触发现有持久撤销，恢复 ENABLED 不恢复旧 token，测试重新登录取得新令牌后才做正向测量。

预算实际证明包含：授权并发槽饱和 503；数据库 statement timeout `57014`、lock timeout `55P03`、连接池获取超时及恢复。当前平台真实 ELF 使用 DB_LOCK_TIMEOUT_MS=100，持有 memberships 排他锁时返回 `503/SERVICE_UNAVAILABLE`，释放后两应用恢复成功。HTTP 截止不被解释为 SQL 立即取消；SQL 有独立服务端期限，瞬时无锁等待也不是永不等待的保证。

公共策略测试进入 fast；`scripts/ci/phase30-postgres.mjs` 进入 postgres，使用精确 Rust 选择和真实结果/清理校验。未重新执行整个前端、浏览器和镜像交付矩阵；本轮没有修改前端或发行产物。

## 同负载成本与默认路径

同一工具/PG 镜像、正常用户、两个分别登记的应用及机密客户端，每应用 4 个 worker × 20 次授权，授权预热 5 次、成员预热 3 次，随后成员查询 20 次，池上限 4。旧生产代码采用 baseline-r4；当前 combined 与当前 legacy 各用新鲜夹具。原始数组、方法和 provenance 见 [performance](phase-30-performance.json)。

| 应用/路径 | HTTP/80 次授权 | PG 调用（含嵌套） | SQL 执行累计 ms | 总耗时 ms | p95/p99 ms | 成员 PG 调用/20次 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 门店原生产 | 160 | 1520 | 205.46 | 917.98 | 133.10 / 314.59 | 680 |
| 门店当前 combined | 80 | 1200 | 320.43 | 1157.49 | 122.45 / 448.52 | 540 |
| 门店当前 legacy（默认） | 160 | 1440 | 353.46 | 989.03 | 219.11 / 350.26 | 540 |
| 工单原生产 | 160 | 1520 | 182.99 | 399.86 | 21.61 / 25.60 | 680 |
| 工单当前 combined（默认） | 80 | 1200 | 252.93 | 332.13 | 17.92 / 18.52 | 540 |
| 工单当前 legacy | 160 | 1440 | 283.10 | 379.68 | 20.50 / 21.27 | 540 |

各组观察到的池峰值均为 4、数据库锁等待者峰值均为 0（5ms 采样）。combined 的 HTTP 数下降 50%，PG 调用数下降 21.05%；成员调用数下降 20.59%。当前 legacy 的授权 PG 调用数下降 5.26%。`pg_stat_statements.track=all` 包括嵌套函数，累计执行时间不能当作独占 CPU 时间；本轮该数值上升，不宣称 SQL 执行时间或 CPU 全面改善。

工单本地总耗时和尾延迟改善，默认采用 combined。门店 combined 总耗时与尾延迟收益不足，保留 **legacy 默认**；显式 combined 已验协议等价，可用于后续受控容量复测。legacy 保留两次 HTTP 协议，仍使用新的快照复核，不表示回退到旧 SQL 实现。门店当前 legacy 相对旧生产也有本轮耗时回归，已经披露，不能把减少调用数包装成全面提速。

串行预热不能完全消除其余并发连接/计划的冷启动，首个门店样本尾部明显；每模式一次本地样本不是统计结论、生产 SLO 或 100 会话/10万记录容量证明。更大负载仍属于后续容量阶段。

## 自检

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 工单/门店真实使用同一受控成员协议，未登记资源动作、应用token互换、客户端错绑、平级/跨租户均拒绝。 | PASS | 真实两个独立 Rust 进程及双模式联合，原 ELF 对照；用途、凭据、受众、组织拒绝均有实际检查 |
| 双应用新旧授权路径在身份/菜单/grant/组织硬上限/失效/错误语义等价，不能只测同一应用换appCode。 | PASS | 两个独立登记应用、不同机密客户端；原 ELF/context 与四种 scope 对照及两业务消费者回归，保留错误码区别 |
| 同负载授权HTTP/SQL/锁/池预算有实际前后对比，收益不足保留旧路径并解释，不牺牲撤权。 | PASS | 原生产/当前 combined/当前 legacy 实测及原始数组；门店保留 legacy 默认，累计执行时间和冷尾回归明确披露 |
| contextVersion不能作认证凭据，公开API收敛且依赖单向，公共测试验证行为。 | PASS | hash 作为 token 被拒，窄 lib 公共 API 4 测试，HTTP/SQL 私有、无共享 DB 模型/公共业务 crate |
| paging极大page不溢出500，稳定边界契约及回归进入CI。 | PASS | 公共边界测试、真实 `/api/users?page=i64::MAX&size=200` 返回 200；fast 与 postgres 已接入 |

## 失败、既有债务与清理

- baseline-r1：非 Git 快照缺显式 revision，未执行实际测试；r2：工具镜像没有 pg_isready，未冒充 SQL readiness。修正后 baseline-r3 冷样本、r4 相同串行预热样本 PASS，均保留。
- development-check-r1：提取后的 permissions 局部变量缺失，已修复；r2 公共测试与 all-target check PASS。
- r3/r5：菜单 HashMap 顺序导致对照失败，保留原失败，改为保留所有语义的规范化比较。
- r4：cap-drop 环境不能 chmod docker cp 得来的异主文件；复制为容器内自有文件后设置执行权限，未增加 capability。
- r6：测试恢复状态后错误地复用已被触发器撤销的 token；改为真实重新登录，不修改撤销机制。
- r7：工单 combined/legacy 403 错误映射回归；分离 response kind，受限读取结构化拒绝，修复后完整回归通过。
- r8：30 步开发 PASS，但期间宿主继续完成默认模式和最终可执行文件验证代码；它不冒充最终候选。正式采纳仅 formal-r1 的冻结输入及 31 步。
- 全平台 fmt 在原冻结代码与当前代码都存在既有失败，日志 `baseline-platform-fmt.log`、`platform-fmt-r2.log`；未重排无关模块。全平台严格 Clippy 未作为清洁门禁采纳，历史债务仍在；本阶段窄库和两个业务 crate 严格 Clippy 均实际 PASS。
- 正式容器、PostgreSQL 与网络全部按 owner 清理，资源查询为空；本阶段专属 cache volume 在核对 owner 标签和零使用容器后删除，并以成功的 volume ls 空查确认。没有触碰其他缓存。
- 130 份先前正式证据逐文件 SHA 保持；全部既有迁移/Cargo.lock/生成公共 Rust 契约保持。其他 36 个阶段详情保持，Phase31 仅为下一指针，不执行。

## 回退与交接

先升级平台以提供 authorize，再为工单启用 combined。回退到旧平台前显式设置两应用 `PLATFORM_AUTHORIZATION_MODE=legacy`；combined 不会在服务错误时自动重放旧协议。门店默认已经是 legacy。无新增数据库迁移，现有 schema/锁文件/公开 context 1.0 均保持。

最后授权/成员检查与业务 COMMIT 之间仍有跨库竞态，继承既有绝对截止和提交前验证，不承诺跨库原子撤权。生产域名/TLS、发行、实际部署及生产容量未在本阶段执行。

台账同步内容及下一阶段建议见 [ledger update](phase-30-ledger-update.md)。下一阶段为 Phase31「交付受支持升级与 dry-run 保留策略」，需要新的执行请求。
