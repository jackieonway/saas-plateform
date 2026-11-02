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
- [SSO 登录保护](login-protection.md)、[统一 OAuth/OIDC 与客户端管理](oauth-issuance.md)
- [应用授权合同](application-authorization-contract.md)
- [合作方人工接入、近期 TOTP 与受控邀请](designs/unified-auth/phase-19-partner-onboarding.md)
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
- [签名密钥轮换](../../../saas-identity/docs/signing-key-rotation.md)、[统一身份隔离恢复](../ops/identity-recovery/README.md)
- [HTTPS 验证驱动与证据范围](../tests/unified-auth-https/README.md)

## 证据边界

本索引只提供当前操作指南、配置和可执行入口。设计台账与阶段报告保留用于追溯，不列为当前版本验收结论。发布候选应针对当时完整源码重新构建，并独立记录数据库、HTTPS、浏览器、标准客户端及清理结果。
