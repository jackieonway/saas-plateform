# 身份新库 UUID v7 基线验收

日期：2026-10-24。状态：**PASS**。本证据仅对应身份新功能的空库／隔离升级基线，不代表独立预览三库 UUID 保数据专项已经完成，也不代表 Phase 2～13 已实施。

## 实际环境与边界

- Windows Rust／Cargo 1.97.0，Node 24.22.1。
- Linux 交叉复核使用 WSL Docker 的 `saas-ci:phase04-tools`、Rust／Cargo 1.97.0、Node 24.18.0；源码只读挂载，构建缓存与测试报告使用本阶段独立卷，实际结果见 [Phase 1 验收](phase-01.md)。
- 使用已核实的独立测试容器 `saas-uuid-u02-pg17-20261024`，镜像 `postgres:17.6-bookworm`，测试连接为 loopback 的 55437 端口。
- 管理连接仅从显式 `IDENTITY_PHASE01_TEST_DATABASE_URL` 读取，数据库须为 postgres；不读取 `.env` 或业务／预览库配置。凭据通过私有进程环境传递，未写入源码、日志或本文。
- 每次真实创建 `identity_phase01_contract_<uuid-v7>` 临时库；只清理该测试持有且名称校验通过的库。成功与失败清理日志均核实数据库已移除、连接池已关闭。
- 没有从预览库读取或迁入账号、成员、业务记录、旧 ID 映射及凭据；没有启动或切换真实服务。

## 两条实际验收路径

### A. 空库完整安装与当前真实 seed

测试：`membership_lifecycle_phase01_fresh_install_seed_v7`。

从空库真实执行 SQLx 历史迁移链至 `0013_membership_identity_policy.sql`，随后运行当前 `seed::run`，再次运行检查幂等。当前 seed 显式写入成员类型和客户端主体模式，要求最新扩展 schema；没有伪造其在 0012 直接运行的结果。

| 检查 | 实测结果 |
| --- | --- |
| UUID 列 | 107 |
| 非空 UUID 单元格 | 582，包含主键及外键重复引用，不是实体数 |
| UUID 默认值 | 0 |
| UUID version／variant 与列约束 | v7／RFC variant；全部目标列检查通过 |
| 外键 | 64 个，真实迁移建立并验证 |
| 隔离 seed 账号 | 6 个；仅为测试用演示初始化，不是生产用户 |
| 初始凭据 | 空库没有已签发 session／refresh／OAuth 凭据 |
| seed 再运行 | ID、成员、客户端与授权快照不变 |
| 默认身份与准入 | 成员为 ENABLED EMPLOYEE／版本 1；应用与 USER client 默认仅员工，MACHINE 为空成员集合 |
| 负例 | 非 v7／错误 variant、遗漏应用 ID、FK／自然键异常被拒绝 |

该测试已在本阶段专项与全量数据库回归中实际通过，包含 Windows 与 Linux 复核；不是仅引用旧 UUID 专项的历史结果。

### B. 0012 基线到 0013 的隔离升级

测试：`membership_lifecycle_phase01_existing_employee_upgrade`。

从空库真实执行至 0012，验证当时 v7 列与默认值，并插入明确的员工、角色、成员数据授权及 USER／MACHINE／LEGACY_MIXED 升级 fixture；再真实执行 0013。

- 升级前后均为 107 个 UUID 列，UUID 默认值为 0；fixture 有 20 个非空 UUID 单元格、64 个外键。
- 员工 ID 和角色／成员数据授权引用保持不变，成员扩展字段有安全默认值。
- 原二元唯一约束与新增三元唯一约束同时存在，没有发布 Phase 6 收缩迁移。
- 原纯机器 client 分类为 MACHINE／空成员集合，纯人员 client 为 USER／EMPLOYEE；迁移前实际混合 fixture 保留 LEGACY_MIXED／EMPLOYEE，不增加客户准入。
- 新建混合客户端、非法成员类型／状态／版本和非法数组由数据库合同测试拒绝。

## 活动消费者与输入合同

| 范围与命令 | 实际结果 | 限制 |
| --- | --- | --- |
| SDK 3.0.0：`npm run check` | 20 个生成／发行产物校验通过 | 历史 1.0.0／2.0.0 字节保留 |
| SDK：`npm test` | 30／30 | 含 v7 生成、回拨、typed context、variant 与请求 ID |
| 平台前端：`npm run sdk:check` | Pinned SDK artifacts verified | 只验证当前固定消费产物，不代表全部门户 UI 已完成 |
| 门店：`cargo test --locked --lib` | 77 passed，0 failed，0 ignored | 非 PostgreSQL 合同；使用受控 Node HTTP fixture |
| 工单：`cargo test --locked --lib` | 71 passed，0 failed，0 ignored | 非 PostgreSQL 合同；使用受控 Node HTTP fixture |

消费者验证显式移除了子进程数据库 URL，未运行门店／工单 PostgreSQL ignored 套件。具体路径审查与工具入口见[成员路径清单](../phase-01-membership-path-audit.md)。v7 的 HTTP／DTO 非法输入及旧客户凭据入口拒绝另由 Phase 1 contract_matrix 实际验证。

## 复现入口与结论

在平台 backend 下，通过私有环境显式提供独立测试集群管理连接后运行：

```powershell
cargo test --locked --bin saas-platform-backend membership_lifecycle_phase01_fresh_install_seed_v7 -- --ignored --nocapture
cargo test --locked --bin saas-platform-backend membership_lifecycle_phase01_existing_employee_upgrade -- --ignored --nocapture
```

本新库具备真实空库初始化、当前 seed、v7 输入／存储、引用完整性、扩展升级与活动消费者证据。历史账号分类、旧 ID 映射、跨库保数据导入及外部旧主体协同在此范围不适用；UUID-U03～U06 的真实状态仍由其独立台账维护。

该基线作为身份 Phase 1 的已验收输入，阶段 API、事务撤销及创建门禁的完成情况见[Phase 1 验收](phase-01.md)。
