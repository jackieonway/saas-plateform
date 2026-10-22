# 平台文档

本项目是六仓库体系中的主平台，提供身份、授权、应用管理和微应用宿主。前端、后端分别安装和构建，业务应用通过网络接入。

| 要做的事 | 阅读入口 |
| --- | --- |
| 了解仓库、服务和数据库关系 | [整体架构](architecture.md) |
| 在本地开发和联调 | [开发指南](development.md) |
| 构建镜像和部署平台 | [部署指南](deployment.md) |
| 接入已经安装的宿主机 Nginx | [现有 Nginx 配置](existing-nginx.md) |
| 运行测试和理解 CI | [工程与验证](engineering-baseline.md) |
| 接入独立业务应用 | [业务接入](independent-business-delivery.md)、[授权协议](application-authorization-contract.md) |
| 给用户配置权限 | [用户授权指南](data-grant-user-guide.md)、[管理接口](data-grant-management.md) |
| 排查登录和凭据问题 | [登录保护](login-protection.md)、[OAuth](oauth-issuance.md)、[审计](management-audit.md) |
| 运行实时通知 | [通知协议](realtime-protocol.md)及独立通知仓库 README |
| 管理应用版本 | [升级与保留](upgrade-and-retention.md)、[ZIP 分发](zip-distribution-runbook.md) |
| 健康监控、备份和恢复 | [运维入口](../ops/README.md) |
| 查看已执行和未执行的检查 | [验证状态](repository-split-validation.md) |
| 了解后续支付规划 | [支付方案](pay/README.md)、[支付台账](pay/task-ledger.md) |

命令所在目录会在正文中明确说明。命令、接口字段和状态码保留原名称，说明文字使用中文。源码、迁移和配置解析器是接口及运行参数的最终依据。
