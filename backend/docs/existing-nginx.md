# 使用已有宿主机 Nginx 部署

适用于 Linux 宿主机运行 Nginx、同机 Docker 运行平台和业务服务。已有 Nginx 提供公开 HTTPS，容器只向 `127.0.0.1` 发布端口，平台使用 `compose.host-nginx.yml`。

## 1. 平台容器

将 [compose.host-nginx.env.example](../compose.host-nginx.env.example) 复制到源码外私有文件，例如 `/srv/saas/private/platform.env`，填写镜像摘要、数据库、管理员、签名凭据、指标凭据、公开来源和数据授权策略。先创建或核对受控网络：

```sh
docker network create --driver bridge --subnet 172.30.0.0/24 platform-edge
docker network inspect platform-edge --format '{{(index .IPAM.Config 0).Gateway}}'
```

`HOST_NGINX_PROXY_IP` 填 API 实际收到的 Nginx 连接对端 IP，通常为 Docker bridge 网关，需按环境实测。它用于可信代理检查，不能填写访客地址或整个网段。

在平台 `backend` 执行：

```sh
docker compose --env-file /srv/saas/private/platform.env -f compose.host-nginx.yml config --quiet
docker compose --env-file /srv/saas/private/platform.env -f compose.host-nginx.yml up -d
docker compose --env-file /srv/saas/private/platform.env -f compose.host-nginx.yml ps
```

不与 `compose.yml` 合并，否则其 gateway 也会启动。默认回环端口如下；业务由各自仓库部署：

| 服务 | 宿主机地址 | 容器端口 |
| --- | --- | --- |
| 平台静态服务 | `127.0.0.1:18080` | `8080` |
| 平台 API | `127.0.0.1:13001` | `3001` |
| 工单 API / 静态服务 | `127.0.0.1:3004` / `127.0.0.1:3013` | `3004` / `8080` |
| 门店 API / 静态服务 | `127.0.0.1:3031` / `127.0.0.1:3014` | `3031` / `8080` |
| 可选通知 | `127.0.0.1:13003` | `3003` |

这些回环地址供宿主机 Nginx 使用。平台 API 容器内的 `127.0.0.1` 指 API 容器，不能直接作为业务静态代理上游；生产登记经真实 HTTPS 网关可访问的地址，并配置允许来源。

## 2. 配置 HTTPS 与微应用路径

使用[完整宿主 Nginx 示例](examples/host-nginx.conf.example)，放到 `http` 级包含目录，例如 `/etc/nginx/conf.d/saas.conf`。完整文件包含 `map` 和 `upstream`，不能整体放入已有 `server` 块。替换域名、证书链与私钥，避免同一域名存在重复生效站点。只启用已部署服务，其余 location 保持 404。

| 路径 | 规则 |
| --- | --- |
| `/api`、`/oauth` | 保留原路径转到平台 API |
| `/micro-proxy/` | 保留完整路径和查询到平台 API，禁止升级连接 |
| `/work-orders-api`、`/stores-api` | 保留前缀到独立业务 API |
| `/sub-apps/<code>/versions/<version>/` | 精确版本路由，剥离版本前缀到对应静态服务 |
| `/micro/<code>` | 宿主 SPA 导航 |

`/micro-proxy/` 必须被平台后端接管，不能落入宿主静态 fallback。代理只提供已登记的静态内容，业务 API 和 WebSocket 使用各自路由。配置见[环境 URL](environment-url-entries.md)和[静态代理](micro-url-proxy.md)。

当前 qiankun 使用 HTML 脚本求值，最终宿主 CSP 的 `script-src` 需要允许 `'unsafe-eval'`。完整示例已为宿主 location 设置该策略并隐藏上游 CSP，渲染器的宿主默认值与之相同。检查面板或其他代理是否额外添加更严格的 CSP。只登记管理员批准的可信 HTML/UMD；求值许可不代表可以加载任意来源，资源连接仍使用准确白名单，不放宽为通配符。显式 `trustedHtmlEval: false` 会阻止 qiankun，只有不运行微应用时才使用。

固定版本前缀须匹配前端 `APP_VERSION` 和平台注册版本。历史版本要保留独立旧静态服务或不可变版本树，不能全部指向一个滚动更新容器。缺失文件返回 404。

## 3. 检查并加载

```sh
sudo nginx -t
sudo systemctl reload nginx
```

先检查成功，再 reload。真实域名验收证书、登录、刷新、OAuth、API、qiankun 加载/卸载、业务操作、JS/CSS MIME、最终 CSP、缺资源 404 和后端审计来源 IP。

## 4. 可选通知

按 [compose.notification.env.example](../compose.notification.env.example) 准备私有配置和数据库身份，在 `backend` 执行：

```sh
docker compose --env-file /srv/saas/private/platform.env --env-file /srv/saas/private/notification.env -f compose.host-nginx.yml -f compose.notification.yml -f compose.host-nginx.notification.yml --profile notification config --quiet
docker compose --env-file /srv/saas/private/platform.env --env-file /srv/saas/private/notification.env -f compose.host-nginx.yml -f compose.notification.yml -f compose.host-nginx.notification.yml --profile notification up -d
```

私有 CA 按已有 overlay 配置信任。`/socket.io/` 转给回环通知端口，保留升级头和读取期限。来源、身份和授权仍由通知服务检查。

## 5. 可选 OAuth 演练

独立演练 API 与静态服务可分别使用回环上游 `127.0.0.1:3011` 和 `127.0.0.1:3010`。在私有 JSON 设置真实 `publicOrigin`、`backend: "127.0.0.1:13001"`、`frontend: "127.0.0.1:18080"`、`oauthPlayground` 和 `oauthPlaygroundStatic`，生成完整候选文件：

```sh
node nginx/render.mjs /srv/saas/private/oauth-gateway.json /srv/saas/config/oauth-gateway.review.conf
```

将生成的 OAuth `map` 及 location 合并到已有站点，核对重复定义与完整代理头。保留以下规则：

- `/oauth-playground-api` 及子路径改写为独立 API `/api`，`internal`、`metrics` 固定 404。
- API 仅发送准确匹配的 `oauth_playground`、`__Host-oauth_playground` Cookie，清空 Authorization 和升级头，保留 Origin 与 Set-Cookie。
- `/callback` 只允许 GET/HEAD 加载 `callback.html`，保留查询、关闭访问日志并禁止缓存；其他方法 405。
- `/sub/oauth-playground` 补斜线跳转，其子路径剥离前缀提供静态资源；静态上游不接收 Cookie 或 Authorization。

回调资产使用 `/sub/oauth-playground/assets/*`。演练 API 的 `OAUTH_PUBLIC_ORIGIN`、平台注册准确 redirect URI（公开来源加 `/callback`）和网关来源一致；qiankun 入口为公开来源加 `/sub/oauth-playground/`。使用真实构建产物，未配置的演练入口保持 404。

## 6. 已有 Nginx 也在容器内时

容器 Nginx 的回环地址指其自身。应加入受控 edge 网络，改用 `backend:3001`、`frontend-web:8080` 和业务 DNS，重新核对可信代理 IP、证书、CSP 和路由。渲染策略和临时 HTTPS 检查见[网关说明](../nginx/README.md)。
