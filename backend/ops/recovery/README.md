# 当前快照备份与核验

`saas-recovery` 仅提供 `backup`、`backup-new`、`verify`、`age`。它使用绝对路径的私有配置、受保护的源库服务凭据、限时 PostgreSQL 工具和加密不可变对象。配置字段见 [config.example.json](config.example.json) 与 [src/config.rs](src/config.rs)。

通用 `restore` 命令已关闭。调用时立即返回固定错误 `CANONICAL_IDENTITY_RECOVERY_REQUIRED`，发生在配置读取、锁／临时目录创建、SQL、子进程、远端下载和监控事件之前。工具中已删除旧凭据失效 SQL、建库、`pg_restore` 和恢复制品写入路径。调用旧命令不会自动转发到其他工具。

## 备份与核验

在本目录运行：

```bash
cargo test --offline --locked
cargo run --offline --locked -- backup /absolute/private/config.json
cargo run --offline --locked -- verify /absolute/private/config.json
cargo run --offline --locked -- age /absolute/private/config.json
```

`backup` 要求先按实际部署协调写入静止，再显式设置 `BACKUP_WRITES_QUIESCED=true`；该环境值只是操作方声明，不能证明多个数据库具有原子快照。`backup-new` 保留同一备份流程并创建唯一对象，`verify` 从远端重新取回和认证备份，`age` 另外核对当前 pilot dump profile 的年龄限制。日志仅输出固定错误和安全状态。

通用备份集仍包含平台共享库、门店库和工单库。客户门户独立库、身份签名私钥、加密密钥和最新撤销事实不能从这个三库归档合同中推定齐备。核验加密对象、条目散列或 catalog 成功，也不构成可安全恢复 canonical 身份系统的证据。

配置已移除 `admin_service`、各库 `target`／`owner` 和 `restored_artifacts`。仅备份和核验不需要恢复管理员或恢复目标。旧配置字段由严格 JSON 解析拒绝；应按当前示例重建配置，不能依靠静默兼容。身份私有数据的备份读权限仍由实际数据库 owner 按明确任务配置，不扩充业务 runtime 权限。

## 恢复边界

共享认证库与客户门户库的 canonical 身份恢复，唯一当前流程是[身份感知恢复工具](../identity-recovery/README.md)：它要求明确目标与集群绑定、受认证的数据库和密钥材料、恢复集外最新 deny 见证、代际合并、隔离重置、gate 和重新验收。通用工具的 `verify` 不替代该流程，也不能把旧归档作为可直接开流的身份恢复集。

门店、工单业务备份继续保留。它们的实际恢复需要单独受控的业务库流程、明确目标授权及当前 schema／业务验证；`identity-recovery` 不直接恢复这些业务库。门户同样不能仅通过销毁会话表替代对所需身份与业务事实的核对。

本目录保留备份 catalog 捕获、bundle 加密与路径完整性检查，以及平台初始化测试仍引用的 `src/uuid_inventory/catalog.rs`。旧 UUID inventory/export/transform/import/config-rekey 命令和通用恢复执行入口均不再提供。当前 dump profile 不宣称持续 WAL/PITR；不得更改存储指纹签认漂移或将 UUID 重编号当作恢复步骤。
