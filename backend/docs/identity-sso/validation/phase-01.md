# Phase 1：成员 schema 与显式生命周期 API 验收

日期：2026-10-24。状态：**COMPLETED／PASS**。Phase 1 六项标准、真实原因审计及适用回归均已验证，执行指针推进 Phase 2（NOT_STARTED）。本文保留首轮失败、修复及分组重跑事实，不将多轮验证描述为一次全绿；状态与[主台账](../task-ledger.md)一致。

## 交付范围

- [0013 扩展迁移](../../../migrations/0013_membership_identity_policy.sql)：成员类型／状态／版本、不可变身份、应用与客户端准入及策略版本；保留旧二元唯一并新增三元唯一，没有 Phase 6 收缩迁移。
- [内部身份模型](../../../src/identity_policy.rs)与[生命周期模块](../../../src/membership_lifecycle/mod.rs)：创建／绑定、读取、独立启停、删除、新增第二身份与显式替换，事务审计、并发版本和具体成员定位。
- [API 与配置说明](../phase-01-api.md)、[成员写路径及二元歧义清单](../phase-01-membership-path-audit.md)、[空库 UUID v7 基线](fresh-v7-baseline.md)。
- 旧用户批量维护、联系人管理员、角色资格及 seed 固定员工语义，三元冲突处理；全局账号管理范围仍覆盖全部类型。
- 旧人员登录、平台 session／refresh、普通用户 OAuth、应用凭据资格的最小 ENABLED EMPLOYEE 守卫，保证扩展期客户不能取得员工型旧凭据。

没有新增第三方依赖、客户角色、客户门户、OIDC 或独立服务工程，没有修改历史 migration／固定 SDK 发行字节。测试只使用独立 PostgreSQL 17 集群新建 fixture；预览库和生产库未被本阶段读取、迁移或切流。

## 核心行为与安全边界

三个创建／生命周期开关默认关闭，且 Phase 1 只允许显式隔离 `APP_ENV=test` 开启。所有新增写接口使用 `system:user` 权限、UUID v7 输入、真实账号／组织／租户与管理范围；绑定客户须确认账号归属并记录来源。所有新增写请求必填真实 `reason`，trim 后为 1～500 个 Unicode 字符且无控制字符，原子保存于审计；缺失／非法时无成员、凭据或审计变更。旧无类型请求不能夹带类型或启停字段，批量操作不删客户，也不隐式启用停用员工。

类型、账号、组织、成员 ID 不能原地改变；同组织同类型重复和版本冲突为 409。第二身份保留源关系；替换只停用源、使用独立目标 ID。目标已存在须核对目标版本，目标停用须显式启用；不自动恢复其旧成员数据授权或会话。永久员工退出清理不再受其他有效员工支持的角色，其他组织合法角色／授权／会话保留。

事务按稳定顺序锁操作者及目标账号，再锁具体成员；不在账号锁后强锁组织／租户父行。锁等待后及提交前使用同一事务重验具体 session、有效员工、实时权限与组织／租户范围指纹；到期比较采用 clock_timestamp。审计失败回滚成员、版本及兼容凭据撤销。当前上下文 EMPLOYEE 的自启停／删除／替换明确拒绝，防止自身资格改变被误当并发授权变更。

**撤销阶段边界**：现有凭据尚无完整成员版本绑定，本阶段仅有员工可通过旧入口签发。员工生命周期撤销其账号／组织适用的旧 session、refresh、OAuth token、code 与 device；客户生命周期不误撤员工。该 EMPLOYEE_COMPATIBILITY 模式有审计记录；精确 membership／account／client／app 版本绑定及客户 context 发放由 Phase 2～6 继续实现，不将本阶段的临时兼容分支描述为完整双身份凭据链。

## 实际执行

Windows 环境为 Rust／Cargo 1.97.0、Node 24.22.1；数据库为独立 `postgres:17.6-bookworm` 测试容器。所有 URL 和凭据仅通过私有进程环境传递，不读取业务 `.env`，不在证据中保留密码或 token。

Linux 复核在 WSL Docker 的 `saas-ci:phase04-tools` 中使用 Rust／Cargo 1.97.0、Node 24.18.0，数据库只连接上述隔离测试集群；源码只读挂载。构建缓存使用本阶段独立 runtime／cargo 卷，生产安全测试的 `target/production-security-tests` 报告另挂独立可写卷；未改写 Windows target 或预览 runtime。CLI 从当前源码重新构建，并通过 `PHASE25_CLI_BIN` 显式选择 Linux 可执行文件；Cargo 离线复用依赖缓存，测试 binary 实际重新编译。

最终只读查询测试集群 `pg_database`，除既有维护库 `postgres`／`uuid_u02_admin` 外没有剩余数据库，所有本阶段及回归 fixture 已清理。运行容器使用 `--rm`，测试连接私有环境文件已删除；保留的独立构建缓存与报告卷不包含测试数据库。

### Phase 1 数据库合同

```powershell
cargo test --locked --bin saas-platform-backend membership_lifecycle_phase01 -- --ignored --test-threads=1 --nocapture
```

Windows 专项 4／4 通过，17.53 秒；随后在 Windows、Linux 完整 ignored 运行中再次通过。最终补齐真实 reason 后从最新源码重新编译，Linux 专项 **4／4 通过，16.58 秒**：

| 测试 | 已通过的实际检查 |
| --- | --- |
| fresh_install_seed_v7 | 空库完整迁移、当前真实 seed、幂等、107 个 UUID 列／582 个非空 UUID 单元格／0 UUID 默认／64 外键／6 个隔离账号及非法输入 |
| existing_employee_upgrade | 0012 v7 基线后真实升级 0013，20 个 fixture UUID 单元格，员工 ID／角色／数据授权引用、双唯一约束及旧客户端分类保留 |
| contract_matrix | 数据库枚举／版本／准入约束、默认门禁、错误租户与范围、重复与并发 409、版本冲突、审计失败回滚、目标双行 fixture、第二身份／替换／复用、旧 bulk 与客户凭据拒绝；五种写操作的 30 个非法 reason 场景、真实 trim 原因审计及 500 Unicode 字符成功边界 |
| authority_rechecked_after_lock_wait | pg_blocking_pids 证明锁等待后分别撤 session／菜单权限、停用组织／租户、使组织／租户实时到期；覆盖创建与启停，无成员变更或成功审计 |

只有专用目标 schema fixture 删除旧二元唯一用于双行正向验证；默认扩展 schema 保留双约束。测试成功或失败后均核验自有数据库移除、池关闭，没有将预览库用于 fixture。

### 构建、消费者与广泛回归

| 检查 | 当前实测结果 |
| --- | --- |
| `cargo fmt --all -- --check` | PASS；最终 reason 修正后再次通过 |
| `cargo check --locked --all-targets` | PASS |
| `cargo clippy --locked --all-targets -- -D warnings` | PASS；最终 reason 与测试修正后的最新源码再次通过，24.40 秒 |
| `cargo test --locked --all-targets -- --test-threads=1` | PASS：9 library＋159 binary＋12 外部测试，共 180 非 ignored；140 数据库测试另显式执行 |
| `cargo doc --locked --no-deps` | PASS |
| SDK 3.0.0 | 20 个产物校验，30／30 tests；平台 sdk:check PASS |
| 门店／工单 `cargo test --locked --lib` | 77／77、71／71；未运行其 PostgreSQL ignored 合同 |
| 首轮平台完整 ignored 回归 | 128 PASS／12 FAIL，共 140；失败修复与跨平台复核见下文，不将首轮记为全绿 |
| 修正后 OAuth 模块 Windows 定点 | 22／22 PASS，保留原摘要、重放、撤销与升级断言 |
| Linux 全量 ignored 运行 | 132 PASS／8 FAIL，共 140，430.90 秒；8 项均在业务断言前因测试报告目录只读失败，见下文 |
| Linux 生产安全模块定点重跑 | 独立可写报告卷后 8／8 PASS，164.71 秒；保留真实 HTTP、即时撤销、配额及生产密码断言 |
| Linux 最新 reason 修正后专项 | 4／4 PASS，16.58 秒；包含六种锁等待屏障、所有原生命周期合同与新审计原因边界 |

完整数据库回归中的 **140 个测试均已有通过结果**：Linux 全量运行通过的 132 个，加报告挂载修正后重跑通过的 8 个。最终 reason 变更只影响新增生命周期 DTO／审计，已对最新源码重新编译并重跑该模块全部 4 项；未把之前 binary 的全量结果冒称为最后一轮源码的一次完整全绿运行。

Linux 在同一显式测试环境执行的 Cargo 命令为：

```sh
cargo test --locked --offline --bin saas-platform-backend -- --ignored --test-threads=1
cargo test --locked --offline --bin saas-platform-backend membership_lifecycle_phase01 -- --ignored --test-threads=1 --nocapture
```

报告挂载修正后的生产安全模块直接运行同一已构建 binary，参数为 `production_security_tests --ignored --test-threads=1`；它只改变容器测试报告目录的可写性，没有修改源码、放宽断言或增加 HTTP 超时。

### 发现、修复与复核

1. 独立代码审查发现旧入口可把客户关系当普通成员、父实体锁顺序倒置及锁等待后操作者／实时到期未重验；已补最小员工守卫及事务内实时指纹，4 项新合同全部通过。
2. 核心套件首次运行 2 PASS／2 FAIL，定位为共享菜单投影 COALESCE 无别名、按列名读取权限失败；修复为既有 MenuRow 投影索引并加说明后 4／4 通过。
3. 完整 ignored 首轮 128／140 通过。5 个 OAuth fixture 仍假设新机器客户端拥有 refresh 或在旧 schema 直接调用新代码；按独立 USER／MACHINE 与真实旧混合迁移路径修正后整个 OAuth 模块 22／22 通过。配额测试的 PUBLIC fixture 也改为合法 USER，而非非法 MACHINE→PUBLIC。
4. 两项迁移回归分别把新增列误判为旧行改变、在只执行 v10 后调用要求扩展字段的当前 API；已保留所有旧字段多重集／checksum 严格比较、增加新字段默认断言，并通过真实迁移器升到当前头再调用当前接口。
5. Windows 下 CLI 及生产真实 HTTP fixture 出现 loopback operation timed out；当前 CLI 重新构建后仍复现，配额 fixture 修正后也表现为同类超时。没有增加超时、关闭断言或把失败写成通过，改用独立 Linux 构建和相同业务断言复核。
6. Linux 全量执行通过 132 项，包括 CLI 真实往返、两项迁移回归和全部 OAuth。生产安全模块 8 项报 `Read-only file system (os error 30)`，原因是其 helper 在 `CARGO_MANIFEST_DIR/target` 写测试清理报告；此前只将 `CARGO_TARGET_DIR` 指向可写卷，源码 target 仍只读。单独挂载本阶段报告卷后原 binary 的整个生产安全模块 8／8 通过，证明无需修改业务实现或 HTTP 断言。
7. 最终审查发现审计 reason 曾等于 action，不能说明实际变更原因。四类 DTO／五种写操作已要求真实 reason，先校验再开启事务；30 个非法输入逐项断言 400 且快照不变，所有成功生命周期写审计精确匹配 trim 后原因，另验证 500 字符边界。最新源码 clippy、fmt 与四项数据库专项均通过。

## Self-Check

| Acceptance Criterion | Result | Notes |
| --- | --- | --- |
| 空库基线、隔离升级、员工 ID／授权、双约束、非法值与重复并发 | PASS | fresh／upgrade／contract_matrix；真实迁移，不写伪迁移记录 |
| 应用／客户端非法集合与版本、新混合禁止、安全默认员工 | PASS | DB 约束、模型与既有客户端创建兼容；机器集合仅表示不适用，策略管理 API 留 Phase 2 |
| 生命周期、目标存在／停用、租户／范围、并发、类型不可变 | PASS | contract_matrix；账号绑定来源及事务审计 |
| 所有客户／第二行写入路径受开关保护，正式启停／替换关闭 | PASS | 三项配置默认关闭、非 test 开启失败、用户 bulk／onboarding／seed 与路径审查 |
| 旧无类型仅员工、停用不自动启用、客户不被 bulk 覆盖 | PASS | 默认扩展拒第二行；专用双行 fixture 验证仅客户时可独立新增员工 |
| 二元键／外键／写入／触发器清单与三元冲突，收缩留 Phase 6 | PASS | 无引用二元唯一的 FK；历史迁移保持原字节；后续读取按 Phase 2～4 继续 |
| 真实原因审计及失败无副作用 | PASS | 五种操作 30 个负例、500 字符边界、trim 后原因精确匹配；审计与变更同事务 |
| 广泛回归与跨平台 HTTP 复核 | PASS | 180 非 ignored、140 数据库用例均有通过结果；Linux 132＋8 报告卷修正重跑，最新 reason 专项 4／4 |

## 后续阶段

下一阶段为 Phase 2 的 context 与凭据版本绑定、精确局部撤销及客户端／应用策略 API。客户与第二身份仍不在正式环境开放，完整安全基线至 Phase 6 才能启用；SSO／OIDC／门户仍为后续阶段。
