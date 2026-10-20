# Vue 静态微应用

独立的 Vite 工程，Node 24.18.0 / npm 11.16.0。Vite 精确锁定为 8.3.4（MIT）；现有 Vue 依赖精确锁定为 3.5.43（MIT）。`package-lock.json` 是本工程的安装入口，不读取宿主的 `node_modules`。

```sh
cd mini-services/micro-vue
npm ci
npm run dev
npm test
npm run build
npm run preview
```

开发和预览默认只监听 `127.0.0.1:3005`，占用端口即失败。可用 `npm run dev -- --host 0.0.0.0 --port 3105` 显式修改监听地址和端口。预览前先构建；结束时使用当前终端的 Ctrl+C，根目录的受控开发入口也可管理本工程。

根 `index.html` 是唯一 HTML 入口，`public/` 保存 classic `app.js`、框架依赖 `vendor/vue.global.prod.js` 和登记版本 **SDK 1.0.0** 的 `vendor/micro-sdk.js`。Vite 将它们逐字节复制到 `dist/`，无需在生产运行 Node。此示例保留现有宿主 TRUSTED 沙箱中的 `window.__MICRO_APP__` 兼容生命周期；默认 opaque iframe 应用另需 iframe SDK 握手，本次没有改变运行模式或 iframe 权限。不能将这些兼容脚本改为 `type="module"`；开发页去掉 Vite 的 ESM/HMR 客户端，修改后通过宿主重新加载微应用。

发布时将整个 `dist/` 放在固定版本目录，例如 `/sub-apps/vue/versions/build-001/`，入口为该目录中的 `index.html`。三个脚本 URL 始终相对入口解析，旧版本目录应继续保留。Vite dev/preview 使用 `mpa` 模式，缺少的 JS、CSS、图片、HTML 和路由均为真实 404。正式静态服务器也必须使用精确文件路由与 404；不能将未知路径改写到入口页。

```sh
npm run preview -- --base /sub-apps/vue/versions/build-001/
```

只向静态目录发布 `dist/`，不要发布仓库、`.env` 或 `node_modules`。前端环境变量仅允许 `MICRO_PUBLIC_` 前缀的公开配置，示例未使用任何环境变量替换；应用 token 和完整 MicroProps 2.0 由宿主运行时传入。开发/预览开放静态资源匿名 CORS，业务 API 的认证和 CORS 由 Rust 服务负责。

`npm test` 运行现有上下文、取消、卸载、迟到响应测试，以及实际启动本工程 Vite dev、构建到临时目录、在固定版本前缀下启动 preview 的 HTTP 测试。测试核对 classic 资源内容与所有缺资源 404，并在结束时关闭监听、删除自身临时目录。此测试不代表真实浏览器、数据库、Docker 或生产 Nginx 已验收。


## SDK 版本与适配

本应用的 classic 生命周期使用 `window.MicroSdk.createContextClient`。`mount` 创建客户端，`update` 传入新的宿主 props，`unmount` 清理客户端；原有框架状态和路由/主题增量更新方式保持不变。应用只消费宿主传入的应用 token、epoch、signal 与到期时间，不读平台存储，也不自行刷新令牌。实际消费版本同时记录在生命周期 `sdkVersion` 与挂载容器的 `data-sdk-version`。

主题和路由变化仅在完整授权、凭据、期限、部署版本和宿主 signal 均相同时复用在途请求。示例外层与 SDK 使用同一个授权签名 helper；即使 contextVersion 不变，grant/菜单/身份/组织上限变化也会取消旧请求并清除展示结果。签名只保存在内存中，不写日志或存储。当前 1.0.0 仍为仓库候选，本次同步不改变 wire 版本或已有 legacy 字节。

SDK 产物来自仓库 `examples/micro-app-sdk` 的首个登记版本 1.0.0，保持 context 1.0、MicroProps/iframe 2.0。vendor 是生成文件；升级时在仓库根执行 `npm run sdk:sync`，并用 `npm run sdk:check` 核对所有登记副本。不要只编辑本应用 vendor。独立发布必须重新构建新应用版本，已发布的旧静态目录继续保留以支持回滚。

业务请求使用 SDK 强制 `credentials: 'omit'`、当前 Bearer 和 UUID `X-Request-Id`，JSON 响应上限为 1 MiB。HTTP 401、403、409、网络与协议错误可分别识别，错误只使用固定文本与安全请求编号，不回显远端消息或 token。前端展示不代替业务后端逐请求授权；本适配没有扩大组织数据边界或改变 TRUSTED/IFRAME 运行模式。浏览器支持区间及完整能力要求见 [SDK 接入说明](../../examples/micro-app-sdk/README.md)。
