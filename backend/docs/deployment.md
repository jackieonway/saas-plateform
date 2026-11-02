# 部署

平台、身份服务、客户门户、通知服务和业务 API 分别构建、配置及发布。使用各自目录作为 Docker build context；不从相邻仓库复制运行时代码。镜像使用经批准的不可变摘要，密钥与私有配置存放在源码外。

## 全新数据库

先执行[全新安装](fresh-installation.md)。平台/身份共库，门店、工单和客户门户独立建库。每个库只使用当前 `0001` 基线；不采用旧迁移历史，也不将旧数据自动导入新库。

平台 canonical migration job 完成结构和显式 bootstrap，随后按[角色交接](../ops/identity-deployment/README.md)建立 migration、identity、platform、bridge owner 和独立 runtime 权限。身份与平台 runtime 不执行 DDL、seed 或权限授予。

初始化结构不等于登记账号、RP、服务客户端、业务应用和授权。首次部署应明确提供管理员、固定 issuer、精确 RP callback、外部密钥及各用途的独立凭据，再验证 readiness。

## 运行配置

平台只支持 SSO/BFF，必需配置见[SSO](sso-only.md)。`IDENTITY_REMOTE_ENABLED=true` 的平台关闭内嵌 provider，通过校验证书的 HTTPS 调用独立身份服务。认证失败或上游不可用不回退。

`compose.yml` 用于独立 edge 网关，`compose.host-nginx.yml` 用于已有宿主 Nginx。复制相应 `compose*.env.example` 到受保护的配置路径，填写镜像摘要、数据库、issuer/RP/CA/keyring、网关来源、网络和独立指标密钥。示例不能直接作为生产凭据。

已有 Nginx 的来源、CSP、宿主资源、微应用代理、OIDC 与通知路由见[网关说明](existing-nginx.md)。认证主机、平台主机和客户门户的 callback/私有 API 边界不能混用。

通知服务使用受限数据库角色和平台明确 authority，业务服务使用自己的 APP_RESOURCE 私有服务凭据和准确允许来源。PostgreSQL TLS 与 HTTP TLS 分别配置，不能据 HTTPS 推断数据库连接已强制校验证书。

## 发布与恢复

先验证数据库/角色/issuer readiness，再验证平台 → SSO → 回调 → 成员选择、客户 RP 隔离、撤销、503、应用授权和通知。成功编译、fixture 或旧报告不能替代该部署的验收。

应用版本的切换、回滚与保留见[版本保留](upgrade-and-retention.md)；备份与恢复见[运维](../ops/README.md)。恢复只接受当前 schema 和同一逻辑数据库集合，恢复后重置会话/凭据并重新核验，不用历史 UUID 搬迁工具。

## 私有资源服务入口

资源后端的 `PLATFORM_API_ORIGIN` 必须指向显式配置的私有 HTTPS listener，同时路由其准确 `POST /api/apps/{code}/authorize` 与 `/internal/platform/apps/{code}/*` 目录合同。通知服务的 `PLATFORM_API_BASE_URL` 同样使用能提供 `/api/internal/notifications/context-proof` 的私有入口。它们不是浏览器的公开平台 origin；公开网关拒绝这些服务端口。

可以使用同一证书主机名加独立私有端口，但端口、监听地址、网络允许范围、精确方法／路径、CA 挂载及每个服务的来源配置都必须由部署责任人核实。任务 fixture 的 `https://console.saas.test:24444` 只说明一种隔离验证布置，不代表现有生产入口。缺少实际私有 listener 或 CA 时列为发布候选／正式切换前置，不将变量指向公开 443 作为替代。
