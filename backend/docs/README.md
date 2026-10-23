# 平台文档导航

本文档集根据工作区实现维护。[qiankun 实施与验证](qiankun-audit.md)记录 2026-10-23 的迁移验收快照，区分本地候选和正式上线；后续门店组织模型的现行契约以本文导航中的授权与组织目录说明为准，历史候选不代表后续源码已重新发布。

## 开发与应用接入

| 文档 | 内容 |
| --- | --- |
| [架构](architecture.md) | 六仓库边界、宿主加载、身份与业务数据流 |
| [开发指南](development.md) | 环境、启动、代理、SDK 和日常检查 |
| [微应用接入](micro-application-onboarding.md) | qiankun 产物、登记、URL/版本入口、访问范围 |
| [按环境配置 URL](environment-url-entries.md) | 可变入口、revision 和版本覆盖规则 |
| [静态代理](micro-url-proxy.md) | `/micro-proxy/` 的资源解析和转发边界 |
| [独立业务交付](independent-business-delivery.md) | 平台登记、业务后端与前端的部署分工 |
| [工程验证](engineering-baseline.md) | CI、数据库和浏览器测试的实际门禁 |

## 身份、权限与通知

| 文档 | 内容 |
| --- | --- |
| [应用授权契约](application-authorization-contract.md) | 应用 token、combined/legacy 服务端授权与数据范围 |
| [应用组织目录](application-organization-directory.md) | STORE 门店身份、总部范围与新鲜组织资料 |
| [OAuth](oauth-issuance.md) | 授权码、PKCE、设备码、内省和撤销 |
| [数据授权管理](data-grant-management.md) | 服务端策略、差分预览/提交、并发与审计 |
| [数据授权操作](data-grant-user-guide.md) | 管理界面流程和范围解释 |
| [实时通知](realtime-protocol.md) | HTTP 持久化、Socket.IO 提示和补拉 |
| [管理审计](management-audit.md) | 事务、变更摘要、查询与导出 |
| [登录保护](login-protection.md) | 限流、密码校验预算和配置 |

## 部署与运维

| 文档 | 内容 |
| --- | --- |
| [部署](deployment.md) | 平台 Compose、独立网络、网关和配置 |
| [接入已有 Nginx](existing-nginx.md) | 宿主机 Nginx 的路由、TLS、CSP 和业务代理 |
| [ZIP 分发](zip-distribution-runbook.md) | 制品存储、代理、节点分发和激活 |
| [升级与保留](upgrade-and-retention.md) | 数据库迁移、版本指针、回滚和清理 |
| [网关工具](../nginx/README.md) | 配置生成、来源策略和静态资源路径 |
| [管理 CLI](../tools/platform-admin/README.md) | bootstrap、release、grants、artifact 和 retention |
| [镜像部署代理](../tools/deployment-agent/README.md) | 镜像操作边界和节点部署 |
| [制品代理](../tools/artifact-agent/README.md) | 固定版本静态资源落盘 |
| [运维工具](../ops/README.md) | 监控与恢复能力及当前限制 |

## 规划与历史

[支付设计](pay/README.md)和[支付实施台账](pay/task-ledger.md)是未实施规划，不是现有支付服务的操作手册。SDK 的公共 API 与生成发布方式由独立 SDK 仓库维护；本平台说明只描述消费和集成。

已删除旧迁移分析、阶段实施记录、旧日期审计、拆仓验收和重复项目总览。迁移验收结论记录在 `qiankun-audit.md`；现行行为以对应功能文档和源码为准。数据库旧迁移、固定 SDK 1 发布归档和历史制品仍可作为历史证据，不能据其目录存在推断当前运行方式。
