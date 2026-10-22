# 平台浏览器检查

使用 Node 24.18.0、npm 11.16.0。先在 `frontend` 安装锁定依赖，再安装独立浏览器工具；工具固定使用 Playwright 1.64.1 和 Socket.IO 4.8.4，不进入生产依赖。

在 `frontend` 执行：

```powershell
npm ci
npm ci --prefix tests/browser
node tests/browser/install.mjs chromium
node tests/browser/run.mjs --browser chromium
```

完整入口依次运行通知、平台设置、应用导航、地区位置、实体类型、部门上下文、路由权限和可信 HTML 沙箱共八套检查。也可以使用 `package.json` 中的 `test:*:browser`、`test:auth:routes` 逐项运行。缺失浏览器或脚本失败会令入口失败。

`--browser` 支持 `chromium`、`chrome`、`msedge`、`firefox`、`webkit`。Chrome 和 Edge 使用对应产品通道，需要本机已安装；不会自动换成 Chromium。下载浏览器存放在本目录忽略的 `.browsers` 中。Linux 可在安装命令追加 `--with-deps` 准备系统依赖。

这些脚本启动真实 Vite/React 页面并拦截 HTTP 接口；通知还使用隔离 Socket.IO 服务。它们不启动业务仓库，不连接 PostgreSQL。通过仅证明所执行浏览器与 UI fixture 下的行为，数据库授权、真实部署与生产版本兼容性需要单独验证。

`npm run test:micro:sandbox:browser` 使用真实 DOM、MutationObserver 和生产 loader 验证同步应用资源清理、宿主懒加载资源保留、嵌套归属及脚本异常后的作用域恢复。该专项验证可信模式的资源记账，不将可信代码执行当作浏览器安全隔离。

微应用简易接入的专项入口为 `npm run test:applications:browser`，可通过 `SAAS_TEST_BROWSER=msedge` 或 `chrome` 选择已安装的浏览器。它使用隔离 API fixture 验证创建、模块选择、发布资源自动发现、激活与授权的忙碌状态、缺项提示和移动端布局；真实授权事务另由 PostgreSQL 专项覆盖。

当前测试从仓库历史提交 `120c211` 的根 `tests` 中恢复，并将所有前端源码、固定 SDK 和工具依赖路径调整为本仓库 `frontend` 内路径。未恢复旧跨仓库调度与已退役的联合验收脚本。
