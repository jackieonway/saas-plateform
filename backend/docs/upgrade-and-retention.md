# 升级、回退与版本保留

## 升级流程

先记录当前程序镜像摘要、配置和数据库迁移状态，并验证备份可取回。隔离环境核对新后端与当前 schema、旧前端协议兼容后，再按 [部署指南](deployment.md)更新镜像。

新增迁移只通过明确版本执行，不重写已应用 SQL/checksum，不自动 down migration。旧程序的迁移器可能拒绝更高版本历史，数据库可读不代表旧镜像可无条件重启。

应用静态发布使用登记、预检、激活和回滚接口。回滚会重新预检，不只是改浏览器缓存。保留已打开旧页所需的 hash 资源，镜像仍在 registry 不代表其历史静态 URL 在线可访问。

## 只读保留预览

GET /api/applications/{id}/releases/retention-preview 仅供符合权限的平台根管理上下文使用。minimumAgeDays 为 30–3650，keepLatest 为 5–1000；CLI 提供 retention-preview 命令。

默认候选保留期为 30 天或最近 5 版，保护取并集。还保护 active、previous、跨环境引用、共享 URL/摘要、未来时间、外部所有权和在途操作。接口跨环境读取引用，不把环境参数当成可安全删除的隔离边界。

返回 applyAvailable:false、deletionPermitted:false。当前没有此预览的 apply/delete 接口，未保护条目也只是需要人工核对，不能据结果直接删除。

## 在途任务与数据分类

发布操作保留持久保护，未知结果、取消或依赖失败时不能靠 TTL 自动解除。需核对实际任务及审计；正常结束只释放本次任务对应保护。

审计、通知、业务软删除、会话、幂等 receipt 和提醒 intent 有不同领域约束，不能套用静态版本预览的天数。在线保留不能代替备份频率、RPO/RTO 或恢复演练。工具入口见 [管理 CLI](../tools/platform-admin/README.md)，数据恢复见 [恢复指南](../ops/recovery/README.md)。
