# 平台文档

以下文档描述当前源码和全新数据库基线。旧升级链、迁移台账、阶段验收与历史环境记录不作为当前运行说明。

## 初始化与部署

- [全新安装与数据库边界](fresh-installation.md)
- [架构与数据归属](architecture.md)
- [开发](development.md)、[部署](deployment.md)、[已有 Nginx](existing-nginx.md)
- [SSO 唯一登录](sso-only.md)、[身份角色与权限交接](../ops/identity-deployment/README.md)
- [工具链与验证](engineering-baseline.md)

## 身份、授权与业务

- [成员与访问控制](identity-and-access.md)
- [SSO 登录保护](login-protection.md)、[OAuth 签发](oauth-issuance.md)
- [应用授权合同](application-authorization-contract.md)
- [组织目录](application-organization-directory.md)
- [数据授权管理](data-grant-management.md)、[使用指南](data-grant-user-guide.md)
- [管理审计](management-audit.md)、[通知协议](realtime-protocol.md)

## 应用交付与运维

- [微应用接入](micro-application-onboarding.md)
- [环境 URL](environment-url-entries.md)、[资源代理](micro-url-proxy.md)
- [独立业务交付](independent-business-delivery.md)
- [ZIP 分发](zip-distribution-runbook.md)、[版本保留](upgrade-and-retention.md)
- [管理 CLI](../tools/platform-admin/README.md)、[部署代理](../tools/deployment-agent/README.md)、[制品代理](../tools/artifact-agent/README.md)
- [运维工具](../ops/README.md)
