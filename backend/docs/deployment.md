# 平台 Docker / Compose 部署指南

如果已有宿主机 Nginx，使用 [现有 Nginx 方案](existing-nginx.md) 与 compose.host-nginx.yml，不启动本页默认的 gateway 容器。下面说明仓库自带网关的部署入口。

## 部署组成

平台 backend/compose.yml 使用已经构建的四个镜像：postgres、backend、frontend-web、gateway。只有网关公开 HTTP/TLS；API 与静态前端在受控 edge 网络，数据库在内部 database 网络。业务应用和通知需要分别部署、显式配置路由。

以下部署命令面向 Linux Docker Engine 与 Compose v2。开发源码构建使用 Node 24.18.0、npm 11.16.0、Rust 1.97.0；服务器运行镜像时不需要这些构建工具。

## 1. 构建并记录镜像

在平台仓库根执行，提交与构建标识替换为真实值：

```sh
docker build -t saas-platform-api:local backend
docker build --build-arg SOURCE_REVISION=<完整提交SHA> --build-arg HOST_BUILD_ID=<构建标识> -t saas-platform-web:local frontend
docker build -t saas-platform-gateway:local backend/nginx
```

本地标签用于构建检查。正式交付将审阅的镜像上传到自己的 registry，记录 registry/repository@sha256 摘要，并选定批准的 PostgreSQL 镜像。当前 CI 只构建，不自动推送或部署。

## 2. 准备网络和私有文件

选一个不冲突的 Docker 私网子网；以下仅为与模板一致的例子：

```sh
docker network create --driver bridge --subnet 172.30.0.0/24 platform-edge
```

已有网络直接核对，不重复创建。选择未占用的网关地址，例如 172.30.0.10。将 backend/compose.env.example 的副本保存到源码之外，例如 /srv/saas/private/platform.env，限制为部署用户可读并填写：

| 配置 | 要求 |
| --- | --- |
| POSTGRES_IMAGE、PLATFORM_IMAGE、FRONTEND_IMAGE、GATEWAY_IMAGE | 批准的不可变镜像摘要 |
| POSTGRES_USER、POSTGRES_DB、POSTGRES_PASSWORD | 平台专用数据库与凭据 |
| PLATFORM_DATABASE_URL | 指向 Compose 内 postgres:5432，与上项一致，密码需 URL 编码 |
| JWT_SECRET、PLATFORM_METRICS_TOKEN | 独立随机值，不共用 |
| BOOTSTRAP_ADMIN_USERNAME、BOOTSTRAP_ADMIN_PASSWORD | 初始管理身份 |
| PUBLIC_APP_URL | 精确公开 HTTPS 来源，含非默认端口时必须写入 |
| EDGE_NETWORK_NAME、GATEWAY_PRIVATE_IP | 实际网络与固定网关地址 |
| GATEWAY_CONFIG_FILE | 生成后 Nginx 配置的绝对路径 |
| TLS_CERTIFICATE_FILE、TLS_PRIVATE_KEY_FILE | 实际证书链和私钥的绝对路径 |
| DATA_GRANT_POLICY | 已审阅的应用能力和管理边界；默认空表示拒绝 |

容器 Nginx 用户为 101，证书和配置必须可读。不要把本地数据库 URL 中的 localhost 带入容器；容器内 localhost 指向容器自身。

## 3. 生成网关配置

按 backend/nginx/config.example.json 创建私有配置，例如 /srv/saas/private/gateway.json，设置与 PUBLIC_APP_URL 相同的 publicOrigin。默认上游 backend:3001、frontend-web:8080；通知、工单、ZIP 与版本路由默认未启用。

在平台 backend 执行：

```sh
node nginx/render.mjs /srv/saas/private/gateway.json /srv/saas/config/nginx.conf
```

先准备可写输出目录。renderer 对配置及输出有校验；重配时先生成新的候选文件，核对后替换受挂载文件。应用 CSP、允许来源与上游都必须随实际接入设置，不能只开放端口。

## 4. 校验并启动

仍在 backend 执行：

```sh
docker compose --env-file /srv/saas/private/platform.env -f compose.yml config --quiet
docker compose --env-file /srv/saas/private/platform.env -f compose.yml up -d
docker compose --env-file /srv/saas/private/platform.env -f compose.yml ps
docker compose --env-file /srv/saas/private/platform.env -f compose.yml logs --tail 100 backend gateway
```

确认数据库、API 和静态前端健康，使用真实域名验证证书、登录、API、静态资源与缺资源 404。语法检查通过不等于服务就绪。

## 5. 可选服务与业务入口

- 工单、门店各用自己的 backend/compose.yml，配置相同受控 edge 网络及平台 HTTPS 身份服务。其容器和数据库由各自 Compose 管理。
- 通知使用 compose.notification.yml 与独立私有环境文件，按需合并 --profile notification；先让平台 HTTPS 可用，再启用通知。网关不能依赖通知就绪形成探针循环。
- 私有 CA 使用 compose.notification-ca.yml，提供真实证书路径，不关闭 TLS 校验。
- 监控使用 compose.monitoring.yml 与 --profile monitoring，见 [监控说明](../ops/monitoring/README.md)。

平台网关支持 workOrders、stores 和 notification 上游，但示例 JSON 未展示全部字段。业务静态版本使用 versionRoutes，并把对应 web 服务加入 allowedServices。可用配置见 [网关说明](../nginx/README.md)，挂到同一网络不会自动生成路由。

## 升级、回退和数据保护

升级前保存当前镜像摘要、配置、迁移状态及可验证备份；先在隔离环境验证新镜像。替换环境文件中的目标摘要，按相同 Compose 命令更新，并检查就绪、登录和业务操作。保留旧静态资源，避免已打开页面的旧 chunk 404。

静态回退与数据库恢复是两个操作。数据库不做自动 down migration，旧 API 镜像必须兼容当前 schema 和迁移历史；不兼容时通过隔离恢复流程处理。不要删除数据库卷作为升级步骤。备份恢复和停写要求见 [恢复指南](../ops/recovery/README.md)。
