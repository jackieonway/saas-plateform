# 微前端接入与上线指南

HTML 微应用采用[上传 ZIP 或 tar.gz 构建包](./application-independent-deployment.md)：服务端将静态文件置于 `/sub-apps/<code>/versions/<version>/`，由管理员预检、激活或回滚，宿主按当前环境的部署指针加载入口。ESM 模块随宿主前端构建和部署。两种方式都不会自动授予租户、组织或角色权限。管理接口与 HTTPS、来源、CORS 规则见[发布 API](./application-release-api.md)。

新建微应用选择 `ESM` 或 `HTML`，无需单独选择运行方式或入口。第一方 ESM 模块以应用编码命名，加入宿主构建后由编码自动关联并直挂运行，随宿主发布；报表中心使用此方式。系统管理和消息中心由宿主路由直接承载，也没有独立入口。HTML 应用上传 ZIP 或 tar.gz 静态包，系统解压并管理环境版本，通过 iframe SDK 接收平台上下文；Vue、jQuery 和独立工单使用此方式。以下可信 HTML 加载方式仅供历史集成迁移参考。

## 接入链路

### 快速接入（HTML 微应用）

新建应用时选择 `HTML` 并保持“快速接入”开启，只需填写名称和编码。系统按编码生成 `basePath=/micro/{code}`、未发布占位入口，并在同一数据库事务里创建首页 `MICRO` 菜单。创建后上传 ZIP 或 tar.gz，待预检通过再激活版本；随后按平台原有流程完成租户、组织、角色授权。关闭“快速接入”可自行填写挂载路径和菜单。

1. 管理员在应用管理创建应用。使用快速接入时填写名称、编码并选择 `HTML`；自定义模式可以指定 `basePath` 和菜单。
2. 上传包含 `index.html` 的 ZIP 或 tar.gz，选择环境与版本号，预检通过后激活。
3. 快速接入会自动创建首页菜单。自定义模式需要在菜单管理创建菜单，`path` 填 `/micro/{code}` 或其子路径，`linkTarget` 选 `MICRO`，`linkUrl` 留空。
4. 完成租户应用/菜单、组织应用/菜单、角色菜单授权，并为用户分配该角色。`/api/auth/profile` 只返回当前租户和组织均已授权的启用应用；`/api/auth/menus` 再按角色与菜单授权过滤导航。
5. 用户打开 `/micro/{code}/*`，`MicroAppContainer` 从个人应用列表读取入口配置，申请应用专用短期 token，再调用 `/api/apps/{code}/context` 获取完整上下文。只有两步成功才加载子应用；关联客户端缺失、授权失效或上下文过期时停止挂载。

快速接入只自动创建首页菜单，不会自动授予租户、组织和角色权限，也不会创建 OAuth 客户端。接入微应用时须关联已启用的机密客户端，确认所需 scope 与业务后端权限规则；客户端密钥只保存在业务后端。角色控制菜单和操作权限，数据范围按当前组织成员关系独立显式授权。

## 自有可信 HTML 子应用契约

本节仅适用于平台管理员显式设置 `runtimeMode=TRUSTED` 的自有代码。默认 iframe 页面应按[iframe 桥接 SDK](../examples/micro-app-sdk/iframe-bridge.js)接入。可信入口返回 HTML，脚本和样式使用相对入口的资源地址，例如 `./app.js`、`./assets/main.css`。不要依赖开发服务器根路径 `/src/...`。主应用用 `import-html-entry` 拉取可信 HTML 与资源，在自定义 `window` Proxy 上执行脚本，随后调用以下生命周期：

```js
window.__DANTE_MICRO_APP__ = {
  name: "example",
  async mount(container, props) {
    // 渲染到 container；MicroProps 2.0 有 token、租户/组织、角色、
    // 操作权限、独立 dataGrants、菜单、版本/到期时间和 AbortSignal。
  },
  async update(props) {
    // 可选：token、授权内容、主题、路由变化时更新；取消旧请求并接收新 signal。
  },
  async unmount() {
    // 清理框架实例、document 事件、外部订阅等，实际结束后再完成 Promise。
  },
}
```

`update` 可选；实现后切换子路径或主题可保留子应用状态。宿主会在组织切换前停止旧工作，并串行等待旧生命周期卸载；无 `update` 的应用在授权变化后重挂。所有业务请求需使用 `props.signal` 并拒绝迟到结果。完整字段、请求 SDK、续期与资源清理规则见[MicroProps 2.0 接入契约](./micro-app-context-props.md)。

## 迁移期未版本化同源静态目录

以下配置用于已有入口的兼容。新应用和新版本请使用[独立版本部署](./application-independent-deployment.md)及仓库的 [Caddy](../deploy/Caddyfile) / [Nginx](../deploy/nginx.conf) 示例，按版本目录保存文件、严格返回资源 404，并保留历史版本供回滚和已挂载用户使用。

以应用编码 `example` 为例：

1. 将子应用构建产物放在主站的 `/sub-apps/example/`，确保浏览器可取得 `/sub-apps/example/index.html` 及其相对资源。若沿用当前 Compose/Caddy 架构，可在前端构建前放入 `frontend/public/sub-apps/example/`；Vite 构建时会复制到 `dist/sub-apps/example/`。也可在 Caddy 中为 `/sub-apps/example/*` 挂载独立静态目录。
2. 在应用管理设置 `code=example`、`basePath=/micro/example`、`entryType=HTML`、`entryUrl=/sub-apps/example/index.html`。该 URL 是浏览器可访问的地址，不是容器内文件路径。
3. 在菜单管理添加 `path=/micro/example`、`linkTarget=MICRO` 的菜单，并完成租户、组织、角色及用户授权。
4. 发布前检查入口、JS、CSS 都返回正确内容类型和资源文件；直刷 `/micro/example` 应由主站返回 SPA 的 `index.html`，随后容器再加载子应用入口。

子应用独立部署时，可让网关把同源前缀 `/sub-apps/example/*` 反代到它的服务。这样主应用仍使用 `entryUrl=/sub-apps/example/index.html`，无需处理跨域抓取 HTML、脚本、样式的 CORS 问题。资源基址和接口地址仍要按该前缀配置。

### Nginx 示例：子应用静态文件随主站发布

假设前端构建产物位于 `/srv/dante/frontend/dist`，其中已有 `sub-apps/example/index.html` 和其相对 JS/CSS 文件，Rust 后端监听本机 3001。生产站点应配置 HTTPS；下面只展示与路径有关的 server 块：

```nginx
server {
    listen 80;
    server_name dante.example.com;
    root /srv/dante/frontend/dist;
    index index.html;

    location ^~ /api/ {
        proxy_pass http://127.0.0.1:3001;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
    location ^~ /oauth/ {
        proxy_pass http://127.0.0.1:3001;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    # 子应用资源必须按真实文件返回；缺失文件应为 404，不能回退成主应用 HTML。
    location ^~ /sub-apps/ {
        try_files $uri =404;
    }

    # /micro/example 是主应用 React Router 路由，刷新时返回主应用 index.html。
    location / {
        try_files $uri $uri/ /index.html;
    }
}
```

`proxy_pass` 这里刻意不带末尾 `/`，以原样保留 `/api/...`、`/oauth/...` 路径。子应用文件放在 `root` 对应的 `dist/sub-apps/example/` 后，数据库 `entry_url` 配置为 `/sub-apps/example/index.html`，菜单 `path` 配置为 `/micro/example`。

### Nginx 示例：子应用单独服务

若子应用进程或静态服务在本机 3005 提供 `/index.html` 及相对资源，用下面的 location **替换**上例的 `/sub-apps/` 静态 location；每个子应用可使用独立前缀和上游。

```nginx
location ^~ /sub-apps/example/ {
    proxy_pass http://127.0.0.1:3005/;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-Proto $scheme;
}
```

此处 `proxy_pass` 末尾的 `/` 用来剥掉 `/sub-apps/example/` 前缀：浏览器请求 `/sub-apps/example/app.js`，上游收到 `/app.js`。子应用 HTML、脚本和样式仍须使用相对资源路径，避免请求错误地落到主站根路径。若上游本身已经服务 `/sub-apps/example/` 前缀，则去掉 `proxy_pass` 的末尾 `/`。

## 当前仓库示例的上线差距

- Vue 和 jQuery 的旧迁移入口 `/sub/vue/`、`/sub/jquery/` 依赖开发代理，当前使用已接入 iframe SDK 的 HTML 版本并从独立目录激活。已执行的旧迁移不要直接修改。
- `report-center` 使用宿主构建的 `/src/micro-apps/report-center.tsx` ESM 入口，直挂运行并展示共享 React 运行时；更新它需要重新构建宿主前端。
- `frontend/Dockerfile` 使用 `frontend/package-lock.json` 与 `npm ci`。首次启用版本网关需实际构建、发布宿主镜像；随后新版本写入独立只读挂载即可，无需重新构建宿主镜像。
- 存量 `frontend/dist/sub-apps/` 中的 Vue、jQuery 示例仍可用于迁移兼容，旧镜像不会自动得到新的网关路由。独立版本使用 Compose 的 `MICRO_APPS_DIR` 挂载目录，须验证网关文件、缓存、MIME 与严格 404 行为。
- Phase 5～8 已补齐应用 token 的租户/组织授权、受众、平台会话绑定和撤销检查，以及完整上下文、续期和生命周期。生产业务后端仍须接入[校验组件](../examples/app-backend/authorize.mjs)，不能只依赖前端菜单或数据授权展示。

## 上线核验

- 管理员与普通用户分别检查应用可见范围、菜单和按钮权限；未授权用户不能通过直接输入 `/micro/{code}` 进入子应用。
- 在浏览器网络面板确认子应用入口及所有 JS/CSS 返回真实文件，不是主站 SPA 回退的 `index.html`；确认无跨域和 MIME 错误。
- 检查主题切换、子路径跳转、重复进入/离开、异常重试和卸载后的事件/定时器清理。
- 若子应用调用后端接口，验证令牌缺失、过期、撤销和越权场景，并确认服务端根据 token 做授权，不能只依赖前端 `menus` 属性。
