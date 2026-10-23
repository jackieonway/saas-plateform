# 独立业务应用接入与交付

工单和门店拥有各自的前端、Rust API、迁移、平台契约、锁文件和 CI。平台提供身份、菜单、功能权限、数据授权与 qiankun 宿主，业务仓库自行构建和部署。

仅需接入前端时，从[微应用接入](micro-application-onboarding.md)开始。本文覆盖同时部署业务 API 的交付过程。

## 1. 准备业务和身份

1. 在所属业务仓库安装锁定依赖，运行其检查并构建前后端，记录源提交、构建标识和镜像摘要。
2. 为业务准备专属 PostgreSQL 数据库、运行账号和迁移执行条件。平台 Compose 不创建业务数据库。
3. 在平台创建机密 OAuth 客户端，将生成的 client ID 和一次性 secret 保存到业务 API 私有环境中。
4. 登记应用与绑定已有客户端，核对业务 `backend/manifests` 的菜单、资源动作和平台协议。
5. 在平台 `DATA_GRANT_POLICY` 中审阅应用 capability、成员查找和管理范围；空策略拒绝业务数据访问。
6. 为用户当前直接成员配置功能权限和数据授权，实测读写、组织切换和撤权。

管理工具的 `bootstrap` 只补缺项，不能迁移或覆盖已有应用。所有 ID 使用平台真实 UUID。接口约束见[应用授权契约](application-authorization-contract.md)与[管理 CLI](../tools/platform-admin/README.md)。

## 2. 构建 qiankun 前端

新接入应用使用 `HTML`、`TRUSTED`，宿主导航为 `/micro/<code>`。前端构建须输出包含 `bootstrap`、`mount`、`unmount` 的 qiankun HTML/UMD 产物，SDK 使用所属仓库锁定的 2.0.0 交付。SDK、宿主上下文和业务鉴权分别承担不同责任，接入细节见[微应用接入](micro-application-onboarding.md)。

工单和门店静态 Dockerfile 接收 `APP_CODE`、`APP_VERSION`、`HOST_ORIGIN`、`API_BASE` 等构建参数，取值按各仓库实际构建入口填写。公开配置仅包含版本、准确宿主来源和业务 API 地址。客户端 secret、数据库 URL、平台 session 不进入 HTML、JS、镜像公开层或 URL。

静态站点必须返回正确 MIME，真实文件缺失返回 404，并保留入口所需的完整 JS/CSS/图片/字体。不能用源代码开发服务器或宿主 SPA 回退代替正式产物。

## 3. 选择一种环境入口

| 方式 | 适用情况 | 运维责任 |
| --- | --- | --- |
| 环境 URL + 默认代理 | 每个环境维护一个现行静态站点 | 平台 API 能经允许来源访问上游；网关将 `/micro-proxy/` 原样转给 API |
| 环境 URL + 关闭代理 | 浏览器直连静态站点 | 上游 CORS、宿主 CSP、证书和资源 URL 全部可达 |
| 固定版本 URL | 需要版本登记、预检、激活和回滚 | 保留 `/sub-apps/<code>/versions/<version>/` 的不可变字节和全部旧资源 |
| ZIP 安装或多节点分发 | 由平台接收静态构建包 | 显式配置可写存储、静态网关及必要的节点门禁 |
| 受管静态镜像 | 固定镜像仓库和目标由 worker 部署 | 平台目标配置、Docker worker、真实网关 route include 和保留策略 |

环境 URL 优先于同环境发布指针，更新会改变代理 revision。它不是一次不可变版本发布；恢复到版本指针前需删除该环境 URL 配置并验收实际入口。[环境 URL](environment-url-entries.md)、[ZIP 分发](zip-distribution-runbook.md)和[部署代理](../tools/deployment-agent/README.md)分别说明配置。

## 4. 部署 API、网关与授权

业务 `backend/compose.yml` 管理其数据库、API 和静态前端。接入同一受控 edge 网络不会自动生成路由，仍需配置 `/work-orders-api` 或 `/stores-api` 上游。业务 API 的平台 HTTPS 来源、客户端凭据、精确宿主来源和可选 CA 按业务文档设置。

平台 `MICRO_ALLOWED_ORIGINS` 控制服务端抓取静态来源，生产仅使用 HTTPS。宿主 CSP 要满足当前 qiankun 脚本求值要求；仓库网关渲染 JSON 显式设置 `trustedHtmlEval: true`，并核查最终响应。普通业务 API 仍执行自己的身份、成员状态、功能权限和数据范围检查。

## 5. 交付验收

使用真实平台账户完成以下检查并保存证据：

- 首次打开、切换页面、卸载重进和浏览器刷新可用，微应用不会遗留定时器或覆盖宿主路由。
- 列表、详情、创建、修改、成员目录及业务报表遵循当前直接成员和数据范围。
- 切组织、撤权、禁用成员后，前端状态更新且服务端拒绝越权操作。
- JS/CSS MIME 正确、缺资源 404，静态网络请求不携带平台身份凭据。
- 工单额外验证提醒、收件箱及通知授权；未部署通知时按业务行为处理。
- 固定版本切换与回滚后，旧版本 URL 和已打开页面所需资源仍可取回。

升级保留已引用的资源，不修改已执行迁移 SQL/checksum。未知发布或写入结果按原意图查询，不能通过换键、覆盖版本或删目录强行重试。恢复边界见[升级与保留](upgrade-and-retention.md)及[三库恢复](../ops/recovery/README.md)。
