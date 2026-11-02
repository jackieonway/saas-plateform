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
| `/auth`、`/api` | 保留路径转平台 API；`/auth` 属于平台 BFF Cookie owner，不能交给静态宿主 |
| `/micro-proxy/` | 保留完整 URL 转平台 API，关闭升级；由后端核对已登记静态入口 |
| `/micro/<code>` | 宿主 SPA 导航 |
| `/sub-apps/<app>/versions/<version>/` | 匹配 `versionRoutes` 后剥离前缀转静态服务 |
| `/sub-apps/` | 其余路径交给 `zipStatic`，未配置时 404 |
| `/work-orders-api`、`/stores-api`、`/socket.io` | 转已启用的业务或通知上游 |

内部、指标、开发服务器等命名空间受门禁保护。`/micro-proxy/` 不能落入宿主 SPA；宿主镜像对该路径固定返回 404。版本 URL 必须长期对应同一份字节。

平台 BFF 的精确公开来源必须与 `publicOrigin` 相同；网关保留 Cookie、Origin 和上游 Set-Cookie，并将 Host 固定为该来源。访问日志仅记录不含查询串的 `$uri` 与状态。`/auth` 单独屏蔽原始 Nginx 错误上下文，避免上游故障把 callback code/state 查询写入错误日志；通过安全访问状态和后端固定错误类别诊断。认证中心的独立域名路由由身份服务部署阶段配置。

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

## 认证权威与退役路径

认证中心的独立 HTTPS issuer 负责唯一公开 `/oauth/*`。本平台网关对旧 `/oauth`、`/oidc`、`/api/oauth`、`/api/oidc`、旧演练、callback、device 和 consent 入口固定返回 404；相关旧配置字段被拒绝。标准第三方 RP 在自己的 HTTPS 来源运行，见[独立示例](../../../saas-examples/oidc-rp/README.md)。

## 检查

在平台 `backend` 执行策略测试：

```sh
node --test tests/nginx-render-policy.mjs
```

Linux 已安装 Nginx/OpenSSL 时可增加临时真实 HTTPS 验证：

```sh
PLATFORM_TEST_NGINX=/usr/sbin/nginx PLATFORM_TEST_OPENSSL=/usr/bin/openssl node --test tests/nginx-render-policy.mjs
```

便携 Nginx 可用 `PLATFORM_TEST_NGINX_MIME_TYPES` 指定已存在的 `mime.types` 绝对路径，无需安装系统包或启动系统服务。测试使用自有随机回环端口；PID、临时目录和启动日志都位于测试目录。

该验证从实际网关样例开始，仅替换自有随机监听、证书和测试上游。Node 请求显式信任临时证书并验证 `localhost` 主机名，另以缺少该信任和错误主机名两个负对照确认 TLS 在请求到达上游前拒绝。验证还检查真实宿主 HTTPS CSP、上游旧策略替换、BFF Cookie/来源代理、回调日志脱敏、公开应用 context 转发，以及旧签发和私有服务入口拒绝。

嵌套浏览器测试由 `PLATFORM_TEST_BROWSER_PACKAGE` 指向 Playwright `index.mjs` 的绝对路径启用，`PLAYWRIGHT_BROWSERS_PATH` 指向已准备的 Chromium。浏览器必须具备对该次临时证书的有效信任；Node 的显式 CA 设置不会传给浏览器。当前自签名夹具不会自动建立浏览器信任，且没有 TLS 绕过选项，因此仅提供浏览器包不足以形成浏览器通过证据。未准备受信任的浏览器环境时，保持该项未运行并单独报告；不要为此修改用户全局信任库。该测试检查真实 qiankun HTML/UMD 挂载、更新和卸载，以及未批准来源被 `connect-src` 拒绝。

可选测试使用临时证书与回环上游，不连接部署服务器；进程和文件在结束后清理。上线前仍须在实际镜像执行 `nginx -t`，用真实域名验收 TLS、最终 CSP、qiankun 生命周期、API、MIME、缺资源 404 和 Cookie 隔离。具体部署见[平台部署](../docs/deployment.md)和[已有 Nginx](../docs/existing-nginx.md)。

私有 `POST /api/apps/{code}/authorize` 由独立 APP_RESOURCE 服务调用，公开网关固定返回 404；浏览器 `/api/apps/session-token` 和应用 context 继续转发。业务 API 的 `PLATFORM_API_ORIGIN` 与通知服务的 `PLATFORM_API_BASE_URL` 必须使用另行配置的私有 HTTPS listener，且证书主机名、CA 和精确路径可达性经过核对。当前公开网关模板不创建该私有 listener；配置缺口属于发布前置，不能改用公开 origin 绕过隔离。
