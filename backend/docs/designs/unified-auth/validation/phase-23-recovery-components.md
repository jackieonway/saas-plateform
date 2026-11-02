# Phase 23 canonical 恢复组件验证

本报告只证明 Python／POSIX 文件组件，实际 PostgreSQL 备份恢复、旧凭据拒绝、新业务和清理必须另有当轮运行证据。精确输入摘要与实际执行记录见[组件 JSON](phase-23-recovery-components.json)，执行入口和契约见[当前恢复 runbook](../../../../ops/identity-recovery/README.md)。未修改总台账。

当前恢复入口已替换旧模型：不再使用 `schema_version=20`、旧 16 族 witness、旧 registry 凭据轮换或历史任务 driver fallback。typed inventory 明确覆盖共享库86张表、门户库5张表、948字段；实际 capture 会核对当前 `0001` 的列、类型、nullable 和PK，额外 canonical 表或结构漂移失败关闭。

运行 `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s saas-plateform/backend/ops/identity-recovery -p 'test_*.py'` 为106项，再执行 `python3 -m unittest discover -s saas-plateform/backend/tests/unified-auth-https -p 'test_phase23_recovery_*.py'` 为23项；两次均禁写bytecode，合计 **129/129 PASS，0失败、0跳过**：

| 组件 | 通过 | 覆盖 |
|---|---:|---|
| core | 18 | AEAD与元数据认证、checkpoint→witness→外部floor真实文件编排、独立序号回退、篡改archive在首次CREATE前拒绝、秘密不进argv／错误输出、祖先目录替换race、catalog／ACL／角色漂移、完整历史跨用途旧secret／新secret复用、真实成员状态触发器的代际合同、真实FIFO及时拒绝 |
| inventory | 22 | 完整安全族、真实catalog shape、秘密投影、类型／PK／UUIDv7／时间／代际、不变source与logout事实、key窗口、终态和次数、合法的一次consent关联 |
| reconcile | 29 | canonical全部旧载体关闭、双BFF未知结果和指针、source／JTI合并、准确原receipt、缺FK失败关闭、deny-only权限交集、key吊销、投递预算、完整历史高水位、已删除不同credentialID的client归属和明确rearm allowlist |
| witness | 25 | HMAC／checkpoint／issuer／cluster绑定、旧record／CURRENT／全目录回滚加外部序号、缺链／孤儿、存储上界、并发writer、符号／硬链接与mode、跨链高水位、删除对象floor保留、同一锁内读取完整已认证历史且不返回部分链 |
| negative probe | 12 | 真实status子进程／AEAD／文件链9类负向、真实原签名baseline、逐案finally全字节还原、超时和失败、并发冲突保留before-image、不把预存损坏当PASS；本组件PG只读预检为mock |
| file adapter | 14 | 精确13标签、认证requirements与独立floor、公开／内部ID映射、凭据与材料互异、从真正还原材料取签名／TLS、新TTL／runtime密码资产、未知秘密／越界／替换拒绝、源文件不变和部分写入清理 |
| cycle caller | 9 | PG／恢复子命令mock：固定顺序及status、源runtime CONNECT门禁、输入摘要不变、错误metadata脱敏、失败保持隔离、超时及中断只终止本次客户端进程组；独立Python子进程实测SIGTERM受控中断，不声称运行真实恢复或测得完整RTO |

独立复审在集成过程中发现六项P2，修复后源码复核全部闭合：关键步骤重核catalog/ACL、完整partial unique索引指纹、跨用途凭据复用、已删除对象历史floor、集群角色权限与membership，以及私有文件祖先TOCTOU。新增回归证明祖先在校验后被替换为symlink时，边界外没有文件产生；不是发生副作用后才返回错误。

最终对照当前迁移的自查另发现一项P2：成员从隔离状态重新启用时，必须在同一UPDATE推进 `identity_version`，否则真实 `protect_membership_identity` 触发器会拒绝。已最小修复为状态与版本同时更新，并增加当前迁移合同回归；该SQL的实际执行仍由后续真实PG演练验证。

root额外独立终审又发现两类P2并实际验证修复：完整历史中被覆盖的service secret仍须拒绝，且被删credential的版本必须保留原client归属；私有artifact为0600 FIFO时须在打开阶段避免阻塞。现rearm／plan／verify消费同一完整认证历史，最低新版本超过已删除的历史v17；真实FIFO负例在受控子进程界限内立即拒绝。累计八项独立P2及一项自查P2均已修复，最终源码摘要以JSON为准。

新增cycle的窄审另发现中断清理P2：独立会话子CLI不应在owner取消时遗留PG客户端。已使未完成调用在BaseException时清理自己的进程组并传播中断，SIGTERM/HUP受控转为中断，先写隔离receipt再返回130；补受控回归后，累计九项独立P2及一项自查P2已修复。

本轮没有连接数据库、运行应用、创建或复用容器；测试使用自动清理的专属临时目录，没有导出密码、token、PHC、私钥、Cookie或连接串。共享PG、其他阶段服务及既有业务库未操作。此处不声称完成实际恢复、SCRAM密码轮换、跨服务HTTPS、RTO/RPO目标或正式发布。
