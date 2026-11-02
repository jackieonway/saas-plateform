# 运维工具

当前运维内容覆盖数据库权限、监控、备份和恢复。系统从[全新基线](../docs/fresh-installation.md)安装；旧 UUID 搬迁、预览库冻结、成员收缩和阶段台账工具已退役。

- [身份数据库 owner/ACL](identity-deployment/README.md)：共享平台库中的账号与业务权限隔离。
- [监控](monitoring/README.md)：探针、指标、告警和受限运维入口。
- [快照备份与核验](recovery/README.md)：平台／门店／工单三库的加密归档、远端完整性和年龄核验；通用 restore 已关闭。
- [身份感知恢复](identity-recovery/README.md)：共享认证库与客户门户库的唯一 canonical 恢复流程，包含最新 deny 见证、隔离与 gate；不直接恢复门店／工单业务库。

运维必须指定实际数据库、角色、来源和凭据，记录当前 schema/二进制/配置版本。数据库 readiness、在线身份 proof 和业务授权分别验证，不能用本地投影代替不可用的认证中心。

工具只处理明确范围的资源；凭据不写日志、命令行、业务清单或提交文件。删除数据、停止服务和清理资源遵守工作区 AGENTS.md 及用户的本次授权。
