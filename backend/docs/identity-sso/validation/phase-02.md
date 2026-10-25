# Phase 2：凭据身份与版本绑定验收

日期：2026-10-24。状态：**COMPLETED／PASS**。六项验收标准均已通过，台账指针推进 Phase 3。本文件保留首轮失败、修复和分组重跑事实；客户与第二身份创建继续关闭。

## 实际交付

- `0014_credential_identity_binding.sql`：平台及 OAuth 凭据的账号／成员／身份／客户端／应用版本、明确来源注册、权威 SQL 守卫、精确成员撤销及策略／账号变更的同事务失效。旧凭据不补绑。
- session、登录选择票据、context／refresh、OAuth 全 grant、设备批准、userinfo／内省及应用 token 的证明链，见 [绑定说明](../phase-02-credential-binding.md)。
- 应用及 OAuth 客户端准入管理 API／DTO／CLI／seed，见 [策略 API](../phase-02-policy-api.md)。
- 新增身份绑定、策略保存、来源／成员快照、权威故障及真实 HTTP 父证明链测试；旧正常签发 fixture 显式保存证明，历史无绑定负例保持拒绝。

未追加收缩迁移，未开放真实客户／第二身份，未改历史 migration 或固定 SDK 发行字节。预览库与 UUID 专项部署没有被本阶段读取、迁移、停止或切流。

## 测试设施与已执行结果

独立新建 PostgreSQL 17.6 容器 `identity-sso-01a16e22-pg`，测试只向该集群新建并清理自有数据库。数据库 URL 通过私有进程环境传递，不读取业务 `.env`，不记录密码、原始 token 或 Cookie。

Linux 使用已有 `saas-ci:phase04-tools` 的 Rust／Cargo 1.97.0，源码只读挂载；从旧编译缓存复制到本任务独立 cargo／runtime 卷。为要求 loopback 的旧测试共享自有 PG 容器网络命名空间，以 `127.0.0.1` 连接该测试集群，不放宽测试目标守卫。源码内 `target/` 报告目录另挂本任务独立可写卷。

| 检查 | 实际结果 |
| --- | --- |
| 空库依序执行 0001～0014 | PASS；独立 smoke 数据库已删除 |
| Windows `cargo clippy --locked --all-targets -- -D warnings` | PASS；首轮类型复杂度和字面 Some lint 已修复后重跑 |
| Linux `cargo fmt --all -- --check` | PASS |
| Linux `cargo check --locked --offline --all-targets` | PASS |
| Linux `cargo clippy --locked --offline --all-targets -- -D warnings` | PASS |
| 首轮 `identity_binding_phase02` | 5 PASS／1 FAIL，8.32 秒；失败为过期 fixture 违反 `expires_at > issued_at`；修复后六项在最终完整数据库运行全部通过 |
| 策略 API 两项数据库合同 | 2／2 PASS，6.50 秒；包含并发冲突、真实审计失败及静默跳过回滚 |
| CLI Linux `fmt/check/clippy` | PASS |
| CLI Linux 全部测试，串行 | 58／58 PASS；25 单元＋6 策略 HTTP＋13 制品＋2 保留＋12 安全 |
| CLI `cargo doc --locked --offline --no-deps` | PASS |
| 首轮 Linux 普通后端测试 | library 9 PASS；binary 162 PASS／1 FAIL／151 ignored，失败是静态文件测试报告目录只读；调整可写报告挂载后完整通过 |
| 最新 Linux 普通后端测试 | PASS：9 library＋163 binary＋13 外部测试，共 185；binary 151 个 ignored 另执行；32.68 秒为 binary 测试时间 |
| 最新 Linux 全套 `fmt/check/clippy/test/doc` | PASS；构建已包含迁移目录追踪，测试报告使用独立可写卷 |
| 首轮 Linux 全量 ignored | 135 PASS／15 FAIL，共 150，615.88 秒；具体修复与最终重跑分开记录 |
| 修正后 Linux 全量 ignored | 150 PASS／1 FAIL，共 151，576.14 秒；唯一失败为旧并发 fixture 被新增账号锁提前串行，未观察到原应用锁屏障 |
| 最终管理审计专项 | 最新 fixture 1／1 PASS，4.36 秒；全部 16 组审计 raise／silent skip、回滚／重试及真实锁屏障通过；最新 `fmt` 和全目标 Clippy 再次通过 |
| 固定 SDK／仓库边界合同 | PASS：明确 Node 24.22.1 下 7／7；平台 `sdk:check` 通过；SOURCE 13 项、manifest 12 项摘要及 3.0.0 的 20 个生成产物验证通过 |

CLI 首轮发现两个旧应用 mock 缺少新准入字段，补齐合法员工默认与版本后原断言通过。并行制品锁测试另发生一次 `WouldBlock`，在 `--test-threads=1` 下完整 58 项通过。没有删减锁、冲突或安全检查。

广泛后端回归发现的应用签发授权顺序／404、无权账号成员选择／403和审计故障／500语义均按原合同修复；另有测试设施 loopback 与可写报告挂载问题。全部 **151 个数据库测试均有实际通过结果**：最终全量通过的 150 项，加仅调整并发 fixture 后重建并定点通过的管理审计一项。最后一项让两个明确授权管理员分别执行请求，避免账号锁先串行而遮蔽待验收的应用锁；原应用锁屏障、404／409、无成功审计和回滚断言均保留。该轮之后没有再修改生产 Rust 或 SQL，不将分组重跑描述为一次全绿。

新增迁移暴露了 SQLx 嵌入迁移的热缓存问题：隔离迁移命令曾成功运行但实际只包含 1～13，测试宏已包含 14。新增 `build.rs` 明确追踪整个 migrations 目录，强制库与 binary 重建；历史升级验证进一步比较完整有序版本集合。当前 UUID 检查也从只数命名约束强化为真实 UUID 列集合与已验证单列 v7 CHECK 覆盖集合相等，精确应为 116（原 107＋新增 9）。不修改历史迁移 checksum。

OAuth 定点在最终重建前实测 22／23 PASS，51.78 秒；唯一 v4 消费 guard 缺失修复后，在最终全量运行 **23／23 通过**。核心 SQL 守卫也核验 v7。真实 HTTP parent proof 测试覆盖 code／device 批准、父注销与来源策略撤销、独立 password grant 保留。

SDK 合同测试原固定断言仍写 2.0.0，而已验收的活动 vendor 是 3.0.0；只更新当前版本断言与自动化说明，继续固定为 3.0.0，不动态接纳任意版本。SDK vendor、公共生成 DTO 和历史 2.0.0 发行字节均未改写；Context 1.0／MicroProps 2.0／Notification 1.0 保持原义。首次直接 `node` 命中本机旧工具链，不支持 `--test`；改用显式 Node 24.22.1 后执行完整合同。

## 验收自检

| 台账标准 | 结果 | 通过证据 |
| --- | --- | --- |
| 成员停用、移除、替换后所有关联凭据失效 | PASS | 六项身份绑定专项、四项原生命周期和 OAuth／session 回归 |
| 同组织两种身份及其他组织互不误撤；伪造绑定拒绝 | PASS | 双向在线、停用→启用新代有效且旧代全部拒绝、删除客户保留新员工及其他组织凭据、七类错配拒绝 |
| 无绑定旧凭据重认证，切换不改成员类型 | PASS | 13→14 旧格式／NULL fixture、旧票据拒绝、明确成员选择与普通 context 切换回归 |
| 权威故障 503 且业务未执行 | PASS | 关闭权威池的 HTTP probe 计数／审计零写入、session 503 专项及固定 SDK 合同 |
| 客户端／应用策略变更精确失效 | PASS | 独立双 client／app grant、code／device／refresh、真实父 session 注销和来源收紧、其他 grant 保留 |
| 策略列表／读取／保存／并发／收紧及来源分离 | PASS | 两项 API 数据库合同、CLI 六项 HTTP 合同、来源与 service client 分别落库并由真实应用 token 检验 |

## Suggested Task Ledger Update

Phase 2：COMPLETED。下一阶段：Phase 3 员工授权、目录及双向授权入口隔离。本轮已将真实完成状态回写主台账；用户要求完成全部任务，后续仍按依赖顺序逐阶段验收，不在 Phase 2 停止。

结束本阶段时只读查询本任务测试集群，仅剩 `postgres` 维护库；所有自有 fixture 数据库已删除。独立编译／报告卷和测试 PG 留作后续阶段继续验收，不涉及预览实例或业务数据。客户／第二身份／真实生命周期写入口的安全门禁保持原限制，Phase 6 前不发布收缩迁移，不宣称 OIDC、客户门户、独立部署或上线完成。
