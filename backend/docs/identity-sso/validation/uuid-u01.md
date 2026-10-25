# UUID-U01 验收记录

状态：COMPLETED。执行日期：2026-10-24。授权范围：本地预览三库；远程开发及生产未审计。此阶段没有生成主体映射、改变业务 ID、初始化数据库或切流。

交付的 `saas-recovery uuid-inventory` 使用独立私有配置，显式限制本地 socket／loopback 和三个 preview 源库；清除 libpq 环境参数，执行有超时的 `REPEATABLE READ READ ONLY`，核验实际库名和只读状态。catalog 包含表行数、UUID/domain/array、default、约束、全部有序 FK、索引、触发器、视图、函数、类型、策略、序列和 migration checksum。数据行 UUID 原值不输出。完整 catalog 只写入 tmpfs 上新建的 mode 0600 文件。

实际运行与脱敏摘要见 [uuid-u01-catalog-summary.json](uuid-u01-catalog-summary.json)。服务器为 WSL Ubuntu PostgreSQL 18.6，Unix socket `/var/run/postgresql`、端口 5432；`inet_server_addr/port` 在 Unix socket 连接下为 null。三个事务均返回 `read_only=on`、`repeatable read`、`row_security=off`。

| 源库 | 表 | UUID 列 | FK | 非空 UUID 单元格 | v4 默认 | v7 单元格 | nil |
|---|---:|---:|---:|---:|---:|---:|---:|
| qiankun_platform_preview_20261024 | 40 | 107 | 64 | 2706 | 23 | 0 | 0 |
| qiankun_stores_preview_20261024 | 7 | 23 | 2 | 28 | 0 | 0 | 0 |
| qiankun_orders_preview_20261024 | 6 | 17 | 1 | 0 | 0 | 0 | 0 |

所有非空单元格均为 version 4／RFC variant。单元格数量包含引用重复，不是独立实体数。当前没有 UUID domain 或数组；清点能够发现它们，并在无已审查映射合同的情况下拒绝登记成功。每库独立快照仅用于清点，不是跨库协调导出。

引用注册表 [uuid-references.json](../../../ops/recovery/uuid-references.json) 精确覆盖 147 个 UUID 列、53 张表、67 组有序 FK，另登记 45 组结构化路径及路由／协议／计数器。运行时比较表与列集合、全部有序 FK，拒绝复合 FK 被拆成多组、非 UUID FK 遗漏或目标变化。引用语义和每表保留见 [引用合同](../uuid-reference-contract.md)，导出、时间规则、签名重建及恢复栅栏见 [导出合同](../uuid-export-contract.md)。注册表 SHA-256：`89d874d1e978eec23ee715b3894bebeccbe79ff492f42089eadb42907658e9b2`。

[生成点注册表](../uuid-generation-registry.json) 区分 Rust 229 个符号引用中的 24 个生产用途／205 个测试引用，浏览器／SDK／Node 活动生成点 11 个、测试 38 个，以及历史 defaults、内嵌 SQL、固定发行、安全随机和版本相关断言。可维护 telemetry 库本体没有 UUID 生成，实际入口在各服务 HTTP adapter。历史字节和负例不算活业务发号。

验证实际执行：

```sh
cargo fmt --manifest-path backend/ops/recovery/Cargo.toml -- --check
cargo test --manifest-path backend/ops/recovery/Cargo.toml --locked --offline
cargo clippy --manifest-path backend/ops/recovery/Cargo.toml --all-targets --locked --offline -- -D warnings
cargo doc --manifest-path backend/ops/recovery/Cargo.toml --no-deps --locked --offline
saas-recovery uuid-inventory /absolute/private/inventory-final-config.json
```

Windows 和 Linux 上单测均为 10 passed；Windows clippy 与文档构建通过，Linux CLI 构建通过。真实命令的远程主机、普通 ext4 报告目录和已存在报告三种负面配置均退出 1。单元测试另外覆盖未登记列／表／UUID 数组类型、错误连接／只读状态、复合 FK 拆分及非 UUID FK 缺失。`git diff --check` 通过。

完整私有报告 SHA-256：`f8998835f4ded92d1b2b54318c51d7e6c9a8fca130ef7de45fced9a80baf0cff`。报告存在本机私有 tmpfs，重启后消失；需要长期保留时必须认证加密，仓库仅保留上述脱敏摘要。离线构建复用已有锁定依赖缓存，没有升级依赖。实际 PG17 新库迁移验收属于 U02，本阶段没有将 PG18 实跑冒充 PG17 实跑。

| Acceptance Criterion | Result | Evidence |
|---|---|---|
| 已审计环境有明确连接目标、只读证据、catalog 和时间 | PASS | 三库真实 CLI metadata、表行数与指纹；远程／生产列为未审计 |
| 自有持久主键／引用、生成点、外部入口有类型、owner 与处理规则 | PASS | 精确 catalog 门禁、两份机器注册表、结构化路径及协议合同；未知路径在 U03 逐记录失败关闭 |
| 导出合同覆盖主数据、凭据排除、业务不变量、指纹与受控恢复 | PASS | 导出合同及每表策略，PG17 基线保留 |

下一阶段 UUID-U02：新代码生成与输入校验、追加迁移及全部 INSERT／seed 适配。真实存量重编号和重建尚未开始。
