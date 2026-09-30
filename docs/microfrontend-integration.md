# 微前端接入与上线指南

本文依据当前实现整理。主应用支持 `HTML` 和 `ESM` 两种入口；独立开发、独立发布的子应用优先使用 `HTML`。`ESM` 是与主应用共享运行时的第一方模块通道。

## 接入链路

### 快速接入（HTML 微应用）

新建应用时选择 `HTML` 并保持“快速接入”开启，只需填写名称和编码。系统按编码生成 `basePath=/micro/{code}`、`entryUrl=/sub-apps/{code}/index.html`，并在同一数据库事务里创建首页 `MICRO` 菜单。接着按平台原有流程完成租户、组织、角色授权。关闭“快速接入”可自行填写入口和菜单；即使直接调用创建 API，HTML 类型留空的路径与入口也会采用相同约定。

将已经构建好的子应用文件同步到主站静态目录：

```bash
npm run micro:sync -- example ./path/to/example/dist
```

命令要求源目录包含 `index.html`，把内容复制到 `frontend/public/sub-apps/example/`。随后重新构建主站，或将该目录作为独立静态目录挂载到生产网关。若后续采用独立服务反代，则在应用管理关闭“快速接入”，填写自定义入口地址即可。

1. 管理员在应用管理创建应用。使用快速接入时填写名称、编码并选择 `HTML`；自定义模式可以指定 `basePath` 和 `entryUrl`。
2. 快速接入会自动创建首页菜单。自定义模式需要在菜单管理创建菜单，`path` 填 `/micro/{code}` 或其子路径，`linkTarget` 选 `MICRO`，`linkUrl` 留空。
3. 完成租户应用/菜单、组织应用/菜单、角色菜单授权，并为用户分配该角色。`/api/auth/profile` 只返回当前租户和组织均已授权的启用应用；`/api/auth/menus` 再按角色与菜单授权过滤导航。
4. 用户打开 `/micro/{code}/*`，`MicroAppContainer` 从个人应用列表读取入口配置，获取该应用菜单，尝试申请应用关联 OAuth 客户端的 access token，再加载子应用。没有关联客户端时，token 为空但挂载继续。

快速接入只自动创建首页菜单，不会自动授予租户、组织和角色权限，也不会创建 OAuth 客户端。子应用需要调用 `/api/external/*` 等 OAuth 资源接口时，再给应用关联启用的 OAuth 客户端，并确认所需 scope 和后端权限规则。

## HTML 子应用契约

入口返回 HTML，脚本和样式使用相对入口的资源地址，例如 `./app.js`、`./assets/main.css`。不要依赖开发服务器根路径 `/src/...`。主应用用 `import-html-entry` 拉取 HTML 与资源，在自定义 `window` Proxy 上执行脚本，随后调用以下生命周期：

```js
window.__DANTE_MICRO_APP__ = {
  name: "example",
  async mount(container, props) {
    // 渲染到 container；props 有 appCode、basePath、routePath、
    // token?、user、theme、menus、onNavigate、onNotify
  },
  async update(props) {
    // 可选：路由、主题、菜单变化时增量更新
  },
  async unmount() {
    // 清理框架实例、document 事件、外部订阅等
  },
}
```

`update` 可选；实现后切换子路径或主题可保留子应用状态。`unmount` 必须清理自身在 `document` 上绑定的监听器。宿主沙箱会回收经代理注册的 window 定时器和监听器，以及部分动态插入的样式/脚本节点，但它不是安全隔离边界：子应用脚本可访问真实 `document`，也会得到注入的 token。只接入可信代码；不可信应用使用独立域名的 iframe/外链模式。

## 推荐的生产部署方式：同源静态目录

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

- Vue 和 jQuery 子应用源码已复制到 `frontend/public/sub-apps/vue/`、`jquery/`；但迁移 `0011_micro_frontend.sql` 插入的数据库入口仍为 `/sub/vue/`、`/sub/jquery/`，依赖 Vite 开发代理和 3005/3006 端口。上线前应将现有数据库的 `entry_url` 更新为 `/sub-apps/vue/index.html`、`/sub-apps/jquery/index.html`，并重新构建前端。已执行的旧迁移不要直接修改；可使用新迁移或管理界面更新。
- `report-center` 示例是 `ESM`，迁移中的入口为 `/src/micro-apps/report-center.tsx`。该地址只适用于 Vite 开发服务器；生产环境需把模块显式打包为浏览器可加载的 ESM 文件并改写入口，或改造为 HTML 子应用。
- `frontend/Dockerfile` 已改为使用 `frontend/package-lock.json` 与 `npm ci`；发布时仍需实际执行镜像构建，并确认子应用已同步到 `frontend/public/sub-apps/`。
- 本地重建后的 `frontend/dist/sub-apps/` 已包含 Vue、jQuery 示例；已运行的旧镜像不会自动获得新文件，仍需重新构建/发布或挂载静态目录。
- 当前 `/api/oauth/session-token` 仅要求有效 session，并按 `appCode` 查找应用和关联客户端，未再核对该用户在当前租户、组织是否获该应用授权。前端容器只从授权应用列表加载，但不能替代后端授权；生产使用注入 token 前应补上服务端授权检查。

## 上线核验

- 管理员与普通用户分别检查应用可见范围、菜单和按钮权限；未授权用户不能通过直接输入 `/micro/{code}` 进入子应用。
- 在浏览器网络面板确认子应用入口及所有 JS/CSS 返回真实文件，不是主站 SPA 回退的 `index.html`；确认无跨域和 MIME 错误。
- 检查主题切换、子路径跳转、重复进入/离开、异常重试和卸载后的事件/定时器清理。
- 若子应用调用后端接口，验证令牌缺失、过期、撤销和越权场景，并确认服务端根据 token 做授权，不能只依赖前端 `menus` 属性。
