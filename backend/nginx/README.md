# Nginx 网关与静态版本服务

本目录包含配置渲染器、独立 HTTPS 网关镜像、可选 ZIP 静态镜像和多节点 ZIP 门禁片段。网关负责公开路由，qiankun 在宿主浏览器运行；微应用资源来自已登记环境 URL 或不可变版本地址。

## 生成与构建

按 [config.example.json](config.example.json) 创建私有 JSON，在平台 `backend` 执行：

```sh
node nginx/render.mjs /srv/saas/private/gateway.json /srv/saas/config/nginx.conf
docker build -t saas-platform-gateway:local nginx
```

输出父目录须已存在。渲染器拒绝未知字段及非法来源、上游。重配先生成候选文件，审阅后替换实际挂载。网关以 `101:101` 运行，默认监听容器 `8080/8443`；外部端口由 Compose 配置，证书、私钥、配置以只读文件挂载。

## 配置字段

| 字段 | 作用 |
| --- | --- |
| `kind` | 使用 `gateway` |
| `publicOrigin` | 无路径和尾部斜线的准确 HTTPS 来源 |
| `backend`、`frontend` | 默认 `backend:3001`、`frontend-web:8080` |
| `notification`、`workOrders`、`stores` | 可选通知、工单、门店上游，关闭时保持 404 |
| `oauthPlayground`、`oauthPlaygroundStatic` | 独立 OAuth 演练 API 和静态服务 |
| `zipStatic` | ZIP 静态上游，保留 `/sub-apps/` 完整路径 |
| `versionRoutes` | `app`、`version`、`upstream` 组成的精确版本路由 |
| `allowedServices`、`allowedUpstreamIps` | 私网 DNS 服务名及额外准确 IP；提供数组会替换默认集合 |
| `trustedProxyIps` | 网关前可信代理的准确 IP |
| `applicationOrigins`、`imageOrigins` | CSP 使用的准确应用和图片来源 |
| `trustedHtmlEval` | 宿主默认 `true`，为 qiankun CSP 加入 `'unsafe-eval'`；静态传输服务默认 `false` |

上游只接受校验允许的 `host:port`。网关样例和宿主策略默认启用 `trustedHtmlEval`，支持 qiankun 2 的 HTML/UMD 求值；配置保留显式 `true`。这是执行管理员批准代码的信任声明，子应用与宿主共享浏览器信任边界。设置 `false` 会阻止 qiankun，仅适用于明确不运行微应用的宿主。

求值许可不扩展 `script-src` 的 URL 来源，也不允许内联脚本；qiankun 抓取外部 HTML/JS/CSS 仍受 `connect-src` 和准确的 `applicationOrigins` 限制。禁止以 `https:` 或通配符代替来源审阅，生产还需配置静态资源 CORS、API 来源和最终响应 CSP。

## 路径职责

| 公开路径 | 行为 |
| --- | --- |
| `/api`、`/oauth` | 保留路径转平台 API |
| `/micro-proxy/` | 保留完整 URL 转平台 API，关闭升级；由后端核对已登记静态入口 |
| `/micro/<code>` | 宿主 SPA 导航 |
| `/sub-apps/<app>/versions/<version>/` | 匹配 `versionRoutes` 后剥离前缀转静态服务 |
| `/sub-apps/` | 其余路径交给 `zipStatic`，未配置时 404 |
| `/work-orders-api`、`/stores-api`、`/socket.io` | 转已启用的业务或通知上游 |

内部、指标、开发服务器等命名空间受门禁保护。`/micro-proxy/` 不能落入宿主 SPA；宿主镜像对该路径固定返回 404。版本 URL 必须长期对应同一份字节。

以下片段可加入完整网关 JSON，还需设置真实 `publicOrigin` 和 `trustedHtmlEval: true`：

```json
{
  "workOrders": "work-orders-api:3004",
  "stores": "stores-api:3031",
  "allowedServices": ["backend", "frontend-web", "work-orders-api", "stores-api", "work-orders-web", "stores-web"],
  "versionRoutes": [
    {"app":"work-orders","version":"1.0.0","upstream":"work-orders-web:8080"},
    {"app":"stores","version":"1.0.0","upstream":"stores-web:8080"}
  ]
}
```

通知等其他服务启用时也须保留在 `allowedServices` 中。历史版本使用独立服务或不可变存储，不能全部指向新版本容器。

## ZIP 与受管镜像

`Dockerfile.micro-static` 使用 [micro-static.example.json](micro-static.example.json) 渲染的配置，只服务已有版本文件，缺资源 404。该服务先剥离公开 `/sub-apps/` 前缀，`microRoot` 内直接保存 `<app>/versions/<version>/`，应映射到平台 `MICRO_APPS_DIR` 或节点 `rootDirectory` 的实际版本根。它传输静态文件，求值发生在宿主文档中，所以自身样例和默认 CSP 保持 `trustedHtmlEval: false`。该服务不安装或激活版本，基础 Compose 不启动它或挂载版本卷。

多节点 ZIP 使用 [zip-distribution.conf.example](zip-distribution.conf.example) 的 `auth_request` 门禁和固定 HTTPS 节点，见[分发指南](../docs/zip-distribution-runbook.md)。门禁不用于受管镜像路径。

受管镜像 worker 写出 location include 并检查、reload Nginx。网关必须显式挂载路由目录并在正确的 HTTPS `server` 中 include；基础 Compose 和渲染器不会自动完成。手写 `versionRoutes` 与 worker 不能重复管理同一前缀，见[部署代理](../tools/deployment-agent/README.md)。

## OAuth 演练

可选 API 将 `/oauth-playground-api/*` 改写到 `/api/*`，保留查询、Origin 和上游 Set-Cookie，仅发送 `oauth_playground`、`__Host-oauth_playground` Cookie，清空 Authorization 和升级头。`internal`、`metrics` 固定 404。

`/callback` 仅 GET/HEAD 加载 `callback.html`，保留查询、禁止缓存并关闭访问日志；`/sub/oauth-playground` 补尾斜线后跳转，其子路径剥离前缀提供静态资源。静态请求不发送 Cookie 或 Authorization；未配置时对应路径保持 404。

回调资源使用 `/sub/oauth-playground/assets/*`，qiankun 入口为公开来源加 `/sub/oauth-playground/`。API `OAUTH_PUBLIC_ORIGIN`、平台注册 `/callback` 和网关来源一致。网关不启动演练服务或创建平台注册记录。

## 检查

在平台 `backend` 执行策略测试：

```sh
node --test tests/nginx-render-policy.mjs
```

Linux 已安装 Nginx/OpenSSL 时可增加临时真实 HTTPS 验证：

```sh
PLATFORM_TEST_NGINX=/usr/sbin/nginx PLATFORM_TEST_OPENSSL=/usr/bin/openssl node --test tests/nginx-render-policy.mjs
```

该验证从实际网关样例开始，仅替换自有随机监听、证书和测试上游。它检查真实宿主 HTTPS CSP、上游旧策略替换和 OAuth 转发/凭据隔离。若已安装 Playwright 及其 Chromium，额外设置 `PLATFORM_TEST_BROWSER_PACKAGE` 为 Playwright `index.mjs` 的绝对路径、`PLAYWRIGHT_BROWSERS_PATH` 为已准备的浏览器目录，即可运行嵌套的真实 qiankun HTML/UMD 挂载、更新和卸载测试；测试还检查未批准来源被 `connect-src` 拒绝。

可选测试使用临时证书与回环上游，不连接部署服务器；进程和文件在结束后清理。上线前仍须在实际镜像执行 `nginx -t`，用真实域名验收 TLS、最终 CSP、qiankun 生命周期、API、MIME、缺资源 404 和 Cookie 隔离。具体部署见[平台部署](../docs/deployment.md)和[已有 Nginx](../docs/existing-nginx.md)。
