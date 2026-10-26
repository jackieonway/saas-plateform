# 当前系统的备份与隔离恢复

`saas-recovery` 提供 `backup`、`backup-new`、`verify`、`age`、`restore`，使用绝对路径的私有配置与受保护服务凭据。旧 UUID inventory/export/transform/import/config-rekey 命令已移除。

在本目录运行 `cargo test --locked`、`cargo run --locked -- <operation> /absolute/private/config.json`。配置字段见 `config.example.json` 和 `src/config.rs`。工具使用有期限的本机 PostgreSQL 工具和加密的不可变备份，日志只输出经过清理的状态。

备份应覆盖同一逻辑时点的平台共享库、门店库和工单库；客户门户会话库需要单独纳入部署备份策略或明确销毁重建。备份包含各库当前 SQLx 版本 1/checksum 和完整结构清单，不接管旧迁移历史。

恢复目标保持隔离，核对原 schema、checksum、owner、引用和当前业务合同。恢复后的账号/客户端/会话需按身份感知策略撤销并重新验收；不能把备份解密或目录对比成功当作已经切换生产。

当前 dump profile 的保留窗口与恢复预算以配置及实现为准，不宣称持续 WAL/PITR。不要修改存储指纹来签认漂移，也不要把 UUID 重编号作为恢复步骤。
