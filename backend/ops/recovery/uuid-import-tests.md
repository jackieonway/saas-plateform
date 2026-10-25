# UUID-U04 导入器实现验证记录

日期：2026-10-24。此记录只覆盖恢复工具的实现门禁与独立测试集群；实际 U04 三目标导入、重建和恢复验收另见身份专项 validation，不能由本记录代替。

| 环境／命令 | 实际结果 |
| --- | --- |
| Windows `cargo test --locked --offline --all-targets` | 153 passed，0 failed，0 ignored |
| Linux `cargo test --locked --offline --all-targets` | 161 passed，0 failed，2 ignored |
| `cargo fmt --all -- --check` | 通过，exit 0 |
| Windows `cargo clippy --locked --offline --all-targets -- -D warnings` | 通过，exit 0 |
| Linux 同一 clippy 命令 | 通过，exit 0 |
| Linux `cargo build --release --locked --offline` | 通过，exit 0 |
| `write_private_preflight_sql_for_operator` 显式诊断运行 | 1 passed，生成 18 个私有 SQL 文件，未连接数据库 |
| 独立 PostgreSQL 17.6 实际 fence SQL 回归 | 4 项通过，未访问业务库 |
| 独立 PostgreSQL 17.6 原生 `Session` 与真实迁移检查 | 1 显式 ignored 测试通过，覆盖三库，未访问业务库 |
| 独立 PostgreSQL 17.6 组织／部门层级数据 COPY／INSERT 回归 | 原逻辑在首个组织逐行回读复现 early EOF；修复后两表各 2 行完整回读，两个循环 UPDATE 均被原触发器拒绝 |

Linux 两个默认 ignored 项分别为 SQL 诊断导出和独立 PG17 原生交互回归。SQL 诊断将真实生成的 BEGIN／fence／metadata／catalog／ROLLBACK SQL 导出至操作者明确提供的私有 tmpfs 新目录；它不连接 PostgreSQL，必须显式设置 `UUID_IMPORT_SQL_DIR` 并使用 `--ignored --exact --nocapture` 运行。合并诊断脚本强制 `READ COMMITTED READ ONLY`；其中 `transaction_valid` 因导入正常要求 READ WRITE 而为 false，是只读探针的预期区别。各分段也单独生成，便于操作者保留私有 stdout／stderr。

实际查询回归使用 [test-uuid-import-fence.py](scripts/test-uuid-import-fence.py)，输入上述真实 Rust 生成的 `platform-02-fence.sql`。脚本创建独立、无网络、不发布端口、数据目录为 tmpfs 的 `postgres:17.6-bookworm` 容器。它不挂载业务卷；完成后检查完整容器 ID、随机标签与 network mode，再仅删除自建容器。

本次独立集群实际包含 41 个 toast relation、167 个 index、1 个 sequence 和 1 个非 postgres LOGIN。四项真实验证为：

1. 干净目标的全 catalog 扫描成功，owner／ACL 围栏通过、写角色计数为零；没有把 toast／index 误传给 sequence privilege 函数。
2. 给无目标 CONNECT 的测试 LOGIN 角色授予普通表 INSERT，fence 返回 `bad_data_writers=1`、`acl_valid=false`。
3. 撤销表权限，授予 sequence USAGE，fence 同样拒绝。
4. 撤销 sequence 权限，fence 恢复干净状态。

类型专用 privilege 函数通过 `CASE WHEN` 在对应 relation kind 内执行。不能用 `WHERE kind='S' AND has_sequence_privilege(...)` 代替：优化器可重排 AND，导致非 sequence 上抛错。table privilege 查询使用相同的类型门控。sequence 的 `acldefault` 类型使用小写 `s`。

脚本第二个可选参数为当前 Linux 编译输出中的原生库测试可执行文件路径，必须匹配 `/runtime/uuid-recovery-target/debug/deps/saas_recovery-<hash>`。提供后，脚本只从只读 runtime 辅助容器复制该测试文件与三个真实 migration-only 二进制至隔离 PG17 容器；PG17 容器本身没有业务卷、网络或已发布端口。三库分别完成真实 12／3／4 项迁移，然后执行 `isolated_pg17_sql_without_newline_and_migration_contracts`。该回归使用导入器真实 `Session::sql`，逐库验证：

1. 第一条 SQL 没有物理换行，在 stdin 保持开启、尚未排入第二条 SQL 时收到 JSON 响应。没有通过关闭 stdin 或随后命令掩盖分帧缺陷。
2. 在生产 `begin_sql` 设置的 `pg_catalog` 路径下执行真实 fence、metadata 查询与严格校验、完整 catalog 查询；工单两份迁移历史和门店三代 schema 合同均通过。
3. 执行真实 `finish_sql` 并收到 `business_verified`；metadata 与 finish 后的 `search_path` 均精确恢复 `pg_catalog`，最后 ROLLBACK 并正常结束原生进程。

该原生回归另在平台真实迁移事务内建立合成租户，以及组织／部门各两级合成行，先在 savepoint 内获取完整行 JSON，再回滚这些行，使用生产 CSV 编码、COPY staging 与完整 `insert_verify_sql` 导入。四行按 ordinal 逐字段回读一致，含带反斜线、引号和换行的文本；每表回读后 `search_path` 精确为 `pg_catalog`。随后真实 UPDATE 尝试在两张表制造循环，须由原组织／部门触发器抛出对应错误。全部合成数据最终随事务 ROLLBACK，fixture 不连接真实三库。

`import-05` 的私有 PostgreSQL 日志将 native early EOF 定位为 `reject_organization_cycle()` 在 `pg_catalog` 路径下无法解析未限定的 `organizations`。上述新回归已在修复前复现相同首行回读失败，修复后通过。生产修复只在每表 INSERT 前设置 `pg_catalog,public`，语句后立即恢复 `pg_catalog`；保留历史函数定义、全部正常触发器以及最终 schema／owner／ACL 检查。`public` 已由正式迁移 schema、owner、CREATE 权限与无其他 backend 的准入检查约束。

`psql --file=-` 需要完整物理行，中央 `Session::sql` 仅在缺失末尾 LF 时补齐；COPY payload 仍通过独立 `send`，不会改变数据。门店历史 fingerprint 使用 `pg_get_*` 的 deparse 输出，须与真实 migrator 保持 `public,pg_catalog` 上下文；两处合同验证仅在固定查询期间设置该路径，再恢复 `pg_catalog`。未放宽 schema hash、owner、ACL、迁移摘要或 fingerprint 对账。

PG18 的 NOT NULL `contype='n'` 与既有 c／p／u／f 同等参与精确源目标约束对账。单测覆盖完全匹配通过，任一侧缺失以及名称／列／定义变化拒绝；未 validated、非布尔 validated 和未知类型即使两侧相同也拒绝。没有忽略 PG18 NOT NULL 约束。

先前只覆盖查询交互的 Linux release 为 `f679596024021544d21290088d6ab5a46769aa38790065203300131c828ca830`。补充上述数据与触发器回归后，再次通过表列 Windows／Linux tests、clippy、fmt 和 Linux release 构建，交付操作者的修复版为：

```text
SHA256
71802231f06f75c64b77bac62b284a3cf7a02b7882820266bf134334933883d4
```

此独立回归覆盖原生分帧、真实三服务迁移的查询交互、准入和最终合同检查，以及合成组织／部门数据 COPY／INSERT 与层级触发器，尚未验证认证包与真实目标 schema hash 的绑定。完整导入仍需在登记的新 U04 三库中验证行 projection proof、FK、触发器、序列、重复拒绝以及失败恢复；独立集群成功不构成 U04 阶段完成声明。
