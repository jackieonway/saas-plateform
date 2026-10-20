# Phase 35 执行与验收

状态：COMPLETED。范围：本机隔离双节点 ZIP 无共享存储分发。989项当前源码，SHA-256：`9e81138c1e72311325acb86c2c2cb56ea27495bf0ebe7dd1dd73a99eaede8f10`；HEAD：`32956c4532bbd6eff53584fd0f5e9bb6e188437c`。

平台只安全解包一次，独立agent接收闭合规范文件/摘要，通过节点私有暂存和Linux RENAME_NOREPLACE发布。所有节点回执、实际磁盘文件及每个节点HTTPS静态字节均核验后，才允许公共预检和显式激活/回滚；事务门禁检查配置与新鲜证据。旧版本资源保留。

## Self-Check

| Acceptance Criterion | Result | Notes |
|---|---|---|
| 缺文件/错摘要/离线不能激活，幂等重试不覆盖异内容 | PASS | 实际双节点故障，公共403/预检与激活409；正常节点inode保持，损坏和有效不同摘要均拒绝覆盖 |
| 认证/TLS/白名单有效，未完成节点不接候选流量，旧版本可回退 | PASS | 正常CA/主机名；错误CA/主机名/token拒绝；auth_request阻止不完整候选；v2激活/v1回退及旧chunk实证 |
| 独立Rust组件/协议/任务UI与恢复证据 | PASS | 15项多节点检查，三浏览器，真实丢失POST响应后相同requestId/单任务，审计回滚和lease恢复 |
| 既有功能及历史证据保留 | PASS | fast 82步/1133通过，17项原发布回归；174份证据和旧迁移1–5摘要不变 |

## 验证证据

- 完整执行：`target/phase-35/implementation/final-r2/result.json`；fast：`target/phase-35/implementation/final-r2/fast-public/summary.json`。
- 运行汇总：[runtime](phase-35-implementation-runtime.json)、[inputs](phase-35-implementation-inputs.json)、[preservation](phase-35-implementation-preservation.json)。
- 独立crate和协议实际执行fmt、test、严格Clippy及rustdoc；平台/前端构建，原发布/ZIP/升级数据库回归17/17。
- 本机二进制已复核ELF Machine 62和SHA-256。owner容器、网络、缓存、私有临时文件均清理；没有生产操作。
- 早期失败/开发轮次记录在runtime的priorAttempts，不作为正式通过依据。

## Risks / Notes

仅两个本机逻辑节点及各自tmpfs，不外推物理跨机/生产可靠性。agent不操作Docker，无自动远程删除功能。原后端整体fmt/Clippy债务保持；新crate和协议严格检查已通过。配置、TLS边界、权限和操作恢复见[runbook](../zip-distribution-runbook.md)。

## Suggested Task Ledger Update

见[完整建议](phase-35-implementation-ledger-update.md)。35 COMPLETED，36保持COMPLETED，37未触发。
