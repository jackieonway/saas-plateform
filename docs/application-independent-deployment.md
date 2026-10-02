# 应用独立版本打包与部署

应用和授权使用稳定的 `applications.id`；静态版本文件、预检记录和环境部署指针分别管理。发布和回滚只切换应用入口，不重建宿主镜像，也不自动授予租户、组织或角色权限。完整管理 API、来源限制与预检诊断见[独立发布 API](./application-release-api.md)。

## 运行方式与接入前提

新建和迁移后的应用默认 `IFRAME`，只可激活 HTML 版本。独立 HTML 页面通过[iframe 桥接 SDK](../examples/micro-app-sdk/iframe-bridge.js)接收授权上下文并处理续期、更新与卸载，调用 `DanteMicroIframe.connect` 时明确配置应用编码、允许宿主 origin 和生命周期；授权字段见[MicroProps 接入说明](./micro-app-context-props.md)，接入方式见[微前端接入指南](./microfrontend-integration.md)。平台自有、已经审查的应用可由平台管理员在应用编辑表单显式选择 `TRUSTED`，此时支持在宿主执行 HTML 生命周期或 ESM 模块。应用编码、入口来源或仓库示例名称都不会自动赋予信任。

Vue、jQuery 现有示例导出宿主 HTML 生命周期，使用这些现有产物时须先由管理员明确标记自有可信；若保持默认 iframe，需要把子应用改为独立渲染页面并使用 iframe SDK。业务后端仍需校验应用 token 受众、组织、权限和数据授权，见[校验示例](../examples/app-backend/authorize.mjs)。

## 不可覆盖的版本打包

先由子应用自己的构建流程生成浏览器可运行的静态目录。HTML、脚本和样式使用相对资源地址；例如 `./assets/main.js`，不要引用开发服务器的 `/src/` 或共享的 `/latest/` 路径。将入口和全部动态 import、CSS 依赖所需文件放入构建目录，然后执行：

```powershell
node scripts/package-microapp.mjs --app example --environment development --version 1.0.0 --input path/to/example/dist
```

默认输出为：

```text
deploy/micro-apps/
└── example/
    └── versions/
        └── 1.0.0/
            ├── index.html
            ├── assets/...
            └── release.json
```

`release.json` 是可直接发送到 `POST /api/applications/{id}/releases` 的请求体，不含应用授权、运行方式或客户端密钥。生成的入口为 `/sub-apps/example/versions/1.0.0/index.html`，资源清单包含构建目录中除入口以外的所有文件，并按入口目录生成相对 URL。JS/MJS/CJS 登记为 `SCRIPT`，CSS 为 `STYLE`，其它静态文件为 `ASSET`；文件名会进行 URL 编码。

可选参数如下：

| 参数 | 用途 |
| --- | --- |
| `--output-root /srv/dante/micro-apps` | 把独立版本发布到指定存储根目录 |
| `--entry-type ESM --entry assets/entry.mjs` | 指定可信 ESM 入口；缺省 ESM 入口为 `entry.mjs` |
| `--entry pages/index.html` | 指定构建目录内的 HTML 入口 |
| `--health-url /micro-services/example/health` | 增加服务端 HTTP 健康检查 |

打包器拒绝已有版本目录或同名文件、输入和输出中的符号链接/junction、路径穿越、`.env` 族及常见凭据配置文件、源与目标相互包含。版本目录以独占创建方式保留，失败时仅清理本次新建目录，既有版本不受影响。根目录的 `release.json` 为保留名称。

当前 API 的预检限制也用于打包检查：单文件最多 4 MiB、总文件最多 16 MiB，资源清单最多 48 项，所有文件均非空。一个版本支持一个 HTML 入口；其它 HTML 文件不能当作 ASSET 预检，因此多入口构建需先整理成单入口交付目录。打包不执行 JavaScript，不检查服务器 MIME、CORS 或完整的运行时行为，这些仍由服务端预检和浏览器验收覆盖。

同一存储根下，一个应用和版本只对应一份文件。将同一内容提升到其它环境时，复制请求体到操作人员的临时文件，改 `environment` 并注册，继续引用同一版本目录；不要修改归档或以相同版本号重新打包不同内容。写入账户和发布工具须遵守不可覆盖约束，网关账户只能读取；只读挂载和数据库版本约束不能阻止拥有存储写权限的人手工改文件。

## Caddy 与 Docker Compose

`frontend/Dockerfile` 使用[deploy/Caddyfile](../deploy/Caddyfile)。Compose 把 `./deploy/micro-apps` 只读挂载到 web 容器的 `/micro-apps`。独立团队可在 `.env.docker` 设置 `MICRO_APPS_DIR=/srv/dante/micro-apps`，让打包器的 `--output-root` 使用同一宿主机目录。因此独立子应用版本不进入宿主前端镜像；首次更新网关配置需要重建 web 镜像，后续新增版本只需写入挂载目录，无需再次构建宿主。

```powershell
# 第一次安装版本网关配置
docker compose up -d --build

# 已安装版本网关后，发布新应用文件
node scripts/package-microapp.mjs --app example --environment development --version 1.0.1 --input path/to/example/dist
```

网关先按 `/sub-apps/<应用>/versions/<版本>/*` 从 `/micro-apps` 读取版本文件；存量 `/sub-apps/*` 继续读取镜像的 `/srv/sub-apps`，迁移期旧入口仍可使用。两个微应用路径均无 SPA fallback：不存在的 HTML、JS、CSS 返回 404；只有宿主路由会回退到宿主 `index.html`。HTML 和目录首页使用 `Cache-Control: no-cache`，成功的版本静态资源使用 `public, max-age=31536000, immutable`，错误返回 `no-store`。保留历史版本文件，使尚未刷新 profile 的用户继续访问原版本资源。

版本目录是公开静态构建产物，Caddy / Nginx 仅为这些目录返回 `Access-Control-Allow-Origin: *`，不允许凭据。这使默认 iframe 的 opaque origin（模块请求的 `Origin: null`）能够加载 Vite 等工具输出的原生 module，同时支持来源已被平台允许的可信跨源静态 fetch。业务 API、OAuth、宿主和迁移期未版本化路径不增加此通用 CORS 头；业务请求依然使用应用 token 与后端授权。版本产物中不能包含服务器密钥、`.env` 或私有业务数据。

Compose 缺省为本地 `development`：后端在容器网络通过 `MICRO_GATEWAY_ORIGIN=http://web` 访问相对入口，允许来源也为 `http://web`。宿主浏览器按公开站点访问相同相对路径。若入口指向独立服务或 CDN，需要配置它们的精确 origin；浏览器可访问的 `localhost` 在后端容器中不会指向外部子应用服务。

生产站点必须在网关或前置入口终止 TLS，并让后端可访问公开 HTTPS origin。当前仓库 Caddy 容器配置监听 80，可放在 TLS 入口之后。若直接由 Caddy 管理 HTTPS，须将 `:80` 改为真实域名、删除全局 `auto_https off`，在 Compose 的 web `ports` 增加 `443:443`（需要 HTTP/3 时另映射 UDP 443），并增加 `/data` 与 `/config` 的持久卷保存证书和配置状态；根级 `volumes` 中也声明这两个卷，避免重建容器丢失证书。仅替换域名不会开启本文件已禁用的自动 HTTPS。

无论由 Caddy 或前置入口终止 TLS，在 Git 忽略的 `.env.docker` 写入：

```dotenv
APPLICATION_ENVIRONMENT=production
PUBLIC_APP_URL=https://saas.example.com
MICRO_GATEWAY_ORIGIN=https://saas.example.com
MICRO_ALLOWED_ORIGINS=https://saas.example.com,https://cdn.example.com
```

然后执行 `docker compose --env-file .env.docker up -d --build`。在生产模式或注册 `production` 版本时，每一步预检 URL 都必须为 HTTPS；网关和允许列表必须是无路径、无凭据的精确 origin。`TRUSTED` 的跨源入口、脚本和样式还需要 CDN 返回匹配宿主的 CORS 头。不要把任意 URL 来源自动加入允许列表。

## Nginx 独立服务

[deploy/nginx.conf](../deploy/nginx.conf) 是可独立校验的 Nginx 示例。替换站点域名、后端上游与文件根目录，并在生产增加证书/TLS 配置。示例使用：

| 路径 | 目录 |
| --- | --- |
| 宿主 SPA、迁移期旧 `/sub-apps/*` | `/srv/dante/frontend/dist` |
| `/sub-apps/<应用>/versions/<版本>/*` | `/srv/dante/micro-apps/<应用>/versions/<版本>/*` |

版本路径通过正则 alias 直接读取真实文件，缺文件返回 404；`/api/` 与 `/oauth/` 原样反代至 Rust 后端。缓存规则与 Caddy 相同，错误响应不标记 immutable。相对 `include mime.types` 按主配置目录解析：把 MIME 表放到示例配置同目录，或将 include 改为已安装 MIME 表的绝对路径。执行 `nginx -t -p /etc/nginx/ -c /绝对路径/deploy/nginx.conf` 后按现有部署流程重载。

示例显式补齐 `.mjs` / `.cjs` 的 JavaScript MIME，避免默认 `entry.mjs` 被当作 `application/octet-stream` 而预检失败。入口和脚本仍须是浏览器可执行的构建代码；仅设置 MIME 不会把 Node 专用的 CommonJS 模块转换为浏览器 ESM。

子应用也可放在其它独立静态服务或 CDN。保持版本目录不可覆盖、HTML 重新验证、版本资源长期缓存、缺失资源严格 404，并配置正确 MIME；将绝对 HTTPS 入口和允许 origin 配置到平台。HTML 中同源脚本和样式必须保留在版本目录，跨源资源也须满足平台来源列表和适用的 CORS 检查。

## 登记、人工预检、发布与回滚

1. 登记应用身份、菜单和 OAuth 关联，再完成租户、组织、角色授权。发布动作不会代替授权。
2. 打包并部署新的版本目录。检查浏览器和平台后端都可访问相同入口与资源。
3. 在应用卡片选择“发布版本”，选择对应环境并登记请求体；也可通过平台 session Bearer 调用注册 API，发送 `release.json`。新版本状态是 DRAFT，不自动联网或激活。
4. 手工预检，逐项检查状态、MIME、字节、健康检查、来源及 CORS。HTTP 200 可能表示 FAILED，必须看 `validationStatus`。
5. 预检 PASSED 后人工激活。服务器会再检；失败保留当前部署并保存诊断。
6. 浏览器刷新 profile 后按版本 ID 卸载旧实例、挂载新入口。已挂载旧用户仍访问旧版本目录，不删除旧目录。
7. 回滚会重新验证 `previousReleaseId` 指向的上一部署，再交换指针；也可手工激活任意历史 PASSED 版本。iframe 应用不能回滚 ESM，停用应用不能切换部署。

## 真实 Vue 版本示例与验证

仓库 Vue 的 `public` 是实际 HTML、应用脚本与已固定的 Vue vendor 文件。以下两条命令生成两个独立、不可覆盖的版本目录：

```powershell
node scripts/package-microapp.mjs --app vue --environment development --version 1.0.0 --input mini-services/micro-vue/public
node scripts/package-microapp.mjs --app vue --environment development --version 1.0.1 --input mini-services/micro-vue/public
```

两个 `release.json` 各含 `app.js` 与 `vendor/vue.global.prod.js`，分别指向 `/sub-apps/vue/versions/1.0.0/index.html` 和 `/sub-apps/vue/versions/1.0.1/index.html`。上述命令用相同源码检验独立版本归档和指针切换；真实代码变更应先由子应用构建，再给新版本号打包。运行现有 Vue 示例时，管理员须明确标记 TRUSTED。

打包器的真实文件测试不使用既有输出：

```powershell
node --test tests/package-microapp.mjs
```

网关测试使用便携 Caddy/Nginx 的路径创建临时配置和本地端口，覆盖真实 HTTP 文件/MIME/缓存、微资源 404、旧入口、宿主 fallback 与 API/OAuth 反代；只关闭自己启动的进程：

```powershell
$env:CADDY_BIN = '绝对路径/caddy.exe'
$env:NGINX_BIN = '绝对路径/nginx.exe'
node --test tests/micro-release-gateways.mjs
```

工具路径未配置且本地默认便携工具不存在时，对应网关测试准确跳过；这不代表网关已被验证。可在各自可用平台单独运行，例如 Linux / WSL 使用 `NGINX_BIN=/usr/sbin/nginx node --test --test-name-pattern=Nginx tests/micro-release-gateways.mjs`，Caddy 用同样的测试名称筛选。部署还应执行实际镜像构建、Compose 配置检查和业务应用浏览器验收。验证命令与发布权限细节见[发布 API](./application-release-api.md)。
