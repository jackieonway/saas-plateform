# 平台部署与 qiankun 静态入口

本文面向 Linux Docker Engine 与 Compose v2。平台仓库包含宿主 React 前端、Rust API 和 Nginx 网关；工单、门店、通知及微应用静态资源各自部署。当前浏览器微应用使用 qiankun，应用登记使用 `HTML`，信任模式为 `TRUSTED`。已有宿主机 Nginx 时，改用[现有 Nginx 部署](existing-nginx.md)。

## 1. 确定入口和资源位置

| 路径 | 处理方 | 部署要求 |
| --- | --- | --- |
| `/micro/<code>` | 宿主 SPA 导航 | 宿主静态服务支持合法导航回退 |
| `/micro-proxy/<code>/<revision>/...` | 平台 Rust API | 网关保留原路径转给 backend；用于已登记且启用代理的环境 URL |
| `/sub-apps/<code>/versions/<version>/...` | 独立静态镜像或 ZIP 节点 | 精确版本路由、真实文件和缺资源 404 |
| `/work-orders-api`、`/stores-api` | 独立业务 API | 配置各自上游、身份和授权 |

环境 URL 可指向独立 HTTPS 静态站点，经平台代理提供同源 HTML/JS/CSS。关闭代理时，浏览器需直接访问上游，上游还须满足 CORS 和宿主 CSP。生产入口、预检和代理都使用服务端 `MICRO_ALLOWED_ORIGINS` 中的准确 HTTPS 来源。直接 URL 配置优先于同环境发布指针，见[环境 URL 入口](environment-url-entries.md)。

固定版本发布支持 URL 登记、ZIP 安装和受管静态镜像。这些是 qiankun 产物的交付方式，静态包仍须符合 HTML/UMD 生命周期要求。基础 Compose 将 `MICRO_APPS_DIR` 设为空，不挂载版本卷、不启动静态节点或部署 worker，因此仅启动平台不会启用 ZIP 和受管镜像部署。

## 2. 构建并记录镜像

在平台仓库根执行，替换真实提交与构建标识：

```sh
docker build -t saas-platform-api:local backend
docker build --build-arg SOURCE_REVISION=<完整提交SHA> --build-arg HOST_BUILD_ID=<构建标识> -t saas-platform-web:local frontend
docker build -t saas-platform-gateway:local backend/nginx
```

本地标签用于构建检查；正式发布记录批准的 `registry/repository@sha256:<摘要>` 和 PostgreSQL 镜像摘要。当前 CI 构建镜像，不推送 registry 或部署服务器。工具版本与检查命令见[工程基线](engineering-baseline.md)。

## 3. 网络和私有环境文件

选择不冲突的受控 bridge 网络；已有网络先核对后复用：

```sh
docker network create --driver bridge --subnet 172.30.0.0/24 platform-edge
```

将 [compose.env.example](../compose.env.example) 复制到源码外，例如 `/srv/saas/private/platform.env`，只允许运维用户读取。

| 配置 | 内容 |
| --- | --- |
| `POSTGRES_IMAGE`、`PLATFORM_IMAGE`、`FRONTEND_IMAGE`、`GATEWAY_IMAGE` | 批准的四个镜像摘要 |
| `POSTGRES_USER`、`POSTGRES_DB`、`POSTGRES_PASSWORD` | 平台专属数据库与凭据 |
| `PLATFORM_DATABASE_URL` | 使用 `postgres:5432` 的容器内连接串，密码按 URL 编码 |
| `JWT_SECRET`、`PLATFORM_METRICS_TOKEN` | 独立随机凭据 |
| `BOOTSTRAP_ADMIN_USERNAME`、`BOOTSTRAP_ADMIN_PASSWORD` | 初始管理身份 |
| `PUBLIC_APP_URL` | 无尾部斜线的准确公开 HTTPS 来源 |
| `MICRO_ALLOWED_ORIGINS` | 准确静态上游来源白名单，未填时使用公开来源 |
| `EDGE_NETWORK_NAME`、`GATEWAY_PRIVATE_IP` | 已有网络和网关固定容器 IP |
| `GATEWAY_CONFIG_FILE` | 实际 Nginx 配置绝对路径 |
| `TLS_CERTIFICATE_FILE`、`TLS_PRIVATE_KEY_FILE` | 真实证书链和私钥绝对路径 |
| `DATA_GRANT_POLICY` | 已审阅的业务能力及管理范围，空策略拒绝 |

Compose 固定传入生产环境，并将 `MICRO_GATEWAY_ORIGIN` 设为 `PUBLIC_APP_URL`。只有网关公开端口；API、静态服务使用 edge 网络，数据库使用内部 database 网络。容器内 `localhost` 指容器本身。

私有 CA 需要额外 overlay 显式传入 `MICRO_CA_CERT_FILE` 并只读挂载证书。仅向 env 文件增加一个 Compose 未引用的变量，不会将它传给容器。Nginx 容器用户为 `101:101`，配置与证书须可读。

## 4. 生成 qiankun 网关

按 [nginx/config.example.json](../nginx/config.example.json) 准备私有 JSON。样例和网关策略默认启用 `trustedHtmlEval`，支持当前 qiankun 2 的 HTML/UMD 求值。它表示执行管理员批准的可信代码，子应用与宿主共享浏览器信任边界；仅登记审阅过的产物和准确来源。配置中保留显式声明：

```json
{
  "kind": "gateway",
  "publicOrigin": "https://platform.example.com",
  "backend": "backend:3001",
  "frontend": "frontend-web:8080",
  "trustedHtmlEval": true
}
```

在 `backend` 执行，输出父目录须预先存在：

```sh
node nginx/render.mjs /srv/saas/private/gateway.json /srv/saas/config/nginx.conf
```

渲染器校验准确来源、上游和字段；求值许可不扩展脚本 URL 白名单，不启用内联脚本，外部资源抓取仍限定于准确的 `applicationOrigins`。`/micro-proxy/` 已有后端专用转发；`versionRoutes`、`zipStatic` 默认关闭。设置 `trustedHtmlEval: false` 会关闭 qiankun 所需的求值，只适用于明确不运行微应用的宿主。重配时生成候选文件，审阅后替换实际挂载文件，并在真实镜像中执行 `nginx -t`。字段和静态路由见[网关说明](../nginx/README.md)。

## 5. 启动和验收

在 `backend` 执行：

```sh
docker compose --env-file /srv/saas/private/platform.env -f compose.yml config --quiet
docker compose --env-file /srv/saas/private/platform.env -f compose.yml up -d
docker compose --env-file /srv/saas/private/platform.env -f compose.yml ps
docker compose --env-file /srv/saas/private/platform.env -f compose.yml logs --tail 100 backend gateway
```

服务健康后，用真实 HTTPS 域名检查登录、刷新、宿主深链、API、最终 CSP、qiankun 加载/切换/卸载、组织切换和撤权。缺失 JS/CSS 必须返回 404，代理入口不能返回宿主 SPA。静态资源请求不得携带平台 token 或客户端 secret。

## 6. 可选能力

| 能力 | 需额外完成 |
| --- | --- |
| 工单、门店 | 各自 Compose、数据库、平台客户端、授权、API 路由和静态入口；见[独立业务交付](independent-business-delivery.md) |
| 通知 | `compose.notification.yml`、私有通知配置与 `--profile notification`；私有 CA 使用 `compose.notification-ca.yml` |
| 监控 | `compose.monitoring.yml` 和私有配置；见[监控指南](../ops/monitoring/README.md) |
| 同机 ZIP | backend 挂载已有可写目录并配置 `MICRO_APPS_DIR`；独立 micro-static 只读提供版本树，网关配置 `zipStatic` |
| ZIP 多节点 | 平台分发配置、节点 TLS/凭据及网关就绪门禁；见[分发指南](zip-distribution-runbook.md) |
| 受管静态镜像 | `DEPLOYMENT_CONFIG_FILE`、独立 worker、路由目录挂载及网关 include；见[部署代理](../tools/deployment-agent/README.md) |

受管镜像需要网关实际包含 worker 写出的 location 文件；基础 Compose 和渲染器不会自动挂载或 include。对同一应用及公开来源，不同时配置 ZIP 分发组和受管镜像模板。

## 7. 升级和恢复

先保存镜像摘要、配置、数据库迁移和入口状态，验证备份可从远端取回，在隔离环境验收后再更新镜像。继续提供已打开页面所需的旧静态资源。数据库不会自动 down migration，旧 API 镜像须兼容当前 schema 和迁移历史。

直接 URL、发布指针、静态文件和数据库具有不同恢复步骤，见[升级与保留](upgrade-and-retention.md)。三库恢复仍使用固定 pilot 材料协议，其成功结果不能证明任意 qiankun 发布材料齐全，见[恢复指南](../ops/recovery/README.md)。
