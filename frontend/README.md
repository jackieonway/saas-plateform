# SaaS 平台前端

本目录是平台的 React 19、TypeScript、Vite 6 宿主，可以独立安装、检查、构建和制作静态镜像。独立微应用的运行入口为 `/micro/:appCode/*`，统一使用 qiankun 2.10.16 加载 HTML，并通过已固定的微应用 SDK 3.0.0 接收平台授权上下文。

## 运行边界

| 页面类型 | 当前实现 | 入口 |
| --- | --- | --- |
| 平台控制台、Athena、Hermes 等内置页面 | 宿主 React 路由，按页面懒加载 | `/system/*`、`/athena/*`、`/hermes/*` 等 |
| 独立微应用 | qiankun HTML 入口、可信运行模式 | `/micro/:appCode/*` |
| 菜单外链 | `SELF` 使用 `/embed` 中的 iframe，其他方式在新窗口打开 | 已授权菜单的精确 HTTP/HTTPS URL |

因此，“独立微应用加载统一为 qiankun”不代表控制台所有页面都已拆成微应用，也不代表外链嵌入功能已经移除。qiankun 使用自己的 JavaScript 沙箱和实验性样式隔离；子应用仍在宿主页面内执行，必须按可信代码管理。

OAuth 客户端页区分业务 OAuth 与认证中心统一登录 OIDC 登记，管理平台和客户门户可在后一个标签查看配置。业务列表使用后端分页，支持每页条数、前后翻页与跳页；删除末页记录后自动修正页码。[接口边界](../backend/docs/oauth-issuance.md)说明两个列表的数据来源。

公共 `DialogContent` 与 `AlertDialogContent` 默认限制在动态视口高度内并允许纵向滚动，覆盖各页面的长表单与确认弹窗；已有定制布局可继续显式覆盖滚动策略。OAuth 编辑框采用固定标题/操作栏与独立滚动表单，小窗口下始终能访问取消和保存。避免在具体页面重复创建无高度上限的固定弹窗。

Vue、jQuery 等正式示例源码位于同工作区独立的 `saas-examples` 仓库。宿主不再保存 `public/sub-apps` 示例副本，也不提供 `/sub-apps/<app>/index.html` 非版本别名；开发代理和正式登记入口分别按下文配置。宿主默认构建不会替独立仓库构建产物。架构与验收入口见[项目文档](../backend/docs/README.md)。

## 开发环境

仓库 `.nvmrc` 固定 Node 24.22.1，`package.json` 要求 Node `>=24.22.1 <25`、npm `>=11.16.0 <12`，并通过 `.npmrc` 开启引擎检查。先确认当前终端实际使用的版本：

```powershell
node --version
npm --version
npm ci --ignore-scripts --no-audit --no-fund
```

依赖通过 `package-lock.json` 固定。使用[平台开发指南](../backend/docs/development.md)启动 Rust 后端后，在本目录执行：

```powershell
npm run dev
```

访问 `http://127.0.0.1:3000`。开发服务严格占用 3000 端口，端口冲突会失败。`npm run preview` 只预览构建结果，不提供下面的开发代理，不能替代联调入口。

### 开发代理

`vite.config.ts` 只配置转发，不启动任何后端或业务前端。启动 Vite 前可以用环境变量覆盖目标：

```powershell
$env:BACKEND_PROXY_TARGET = 'http://127.0.0.1:3001'
npm run dev
```

| 浏览器路径 | 默认目标 | 环境变量 |
| --- | --- | --- |
| `/api`、`/oauth`、`/micro-proxy` | `http://127.0.0.1:3001` | `BACKEND_PROXY_TARGET` |
| `/work-orders-api` | `http://127.0.0.1:3004` | `WORK_ORDERS_DEV_PROXY_TARGET` |
| `/stores-api` | `http://127.0.0.1:3031` | `STORES_DEV_PROXY_TARGET` |
| `/oauth-playground-api`，重写为 `/api` | `http://127.0.0.1:3011` | `OAUTH_PLAYGROUND_DEV_PROXY_TARGET` |
| `/sub/oauth-playground/`、`/callback` | `http://127.0.0.1:3010` | `OAUTH_PLAYGROUND_STATIC_DEV_PROXY_TARGET` |
| `/sub/vue` | `http://127.0.0.1:3005` | `MICRO_VUE_DEV_PROXY_TARGET` |
| `/sub/jquery` | `http://127.0.0.1:3006` | `MICRO_JQUERY_DEV_PROXY_TARGET` |
| `/socket.io`，含 WebSocket Upgrade | `http://127.0.0.1:3003` | `NOTIFICATION_DEV_PROXY_TARGET` |

OAuth 演练代理会剥离平台 Authorization；API 只转发演练服务自己的允许 Cookie，静态代理不转发 Cookie。地图请求有独立公网代理并剥离平台凭据。外部访问 Vite 时，`DEV_ALLOWED_HOSTS` 接受逗号分隔的精确主机名，不接受 URL、端口或通配符。

版本静态文件从 `../deploy/micro-apps/<appCode>/versions/<version>/` 提供，也可以通过绝对路径 `MICRO_APPS_DEV_DIR` 指定存储目录。缺失目录不会阻止 Vite 启动；缺失文件和非法路径返回真实 404。生产环境由网关和 Rust 服务处理微应用资源，见[部署指南](../backend/docs/deployment.md)。

## 微应用加载与授权

实现集中在 `src/components/micro/`：

1. `MicroAppContainer.tsx` 从当前用户 profile 的已授权应用中发现应用，等待所选应用与路由一致；普通用户不需要读取应用管理接口。
2. 宿主向 `/api/oauth/session-token` 申请应用受众令牌，再向 `/api/apps/:appCode/context` 获取完整上下文。可执行入口以该实时上下文为准。
3. `entry.ts` 仅接受 `entryType=HTML`、`runtimeMode=TRUSTED`；旧入口显示重新配置提示。加载每个异步阶段都重新检查入口、授权和取消信号。
4. `qiankun.ts` 使用 `loadMicroApp` 和标准生命周期。子应用从 qiankun props 的 `platform` 获取严格的 MicroProps DTO，从 `services` 获取宿主能力，从 `container` 获取本次挂载节点。
5. 授权变化走子应用可选 `update`；没有 `update` 时串行卸载并重新挂载。切换组织、离开页面、授权失效和到期会撤销旧 signal，迟到结果不能进入新上下文。

MicroProps 使用活动 SDK `3.0.0`、契约 `2.0`，包括应用令牌、用户、租户、组织、组织范围、角色、权限、数据授权、菜单、上下文版本/序号、到期时间、路由、主题及通知/导航回调。`routePath` 完整保留 React Router 的 pathname、search、hash；只变化 query 或 hash 也会更新子应用。菜单授权仍单独按 pathname 归一化判断，query/hash 内容不能授予其他路径访问权。

新应用交付 HTML 与经典脚本/UMD，按平台接入合同导出 qiankun `bootstrap`、`mount`、`update`、`unmount` 四个生命周期；运行时仍为缺少 `update` 的既有应用保留上文所述重挂回退。`script[type=module]` 会被明确拒绝。资源加载禁用预取，剥离 Authorization/Cookie 并使用 `credentials: omit`；业务 API 必须通过应用令牌和 SDK 客户端访问。`services.downloadCsv` 另行检查有效上下文、导出权限、数据授权、文件名、大小和频率。

`runtime.ts` 对同页可信生命周期串行编排，单阶段默认期限 10 秒。生命周期超时或无法证明成功卸载时，页面运行域被锁定，必须刷新整个宿主页面恢复；取消信号和超时不能终止已经执行的 JavaScript。

接入时可按环境配置直接 URL，默认启用平台同域代理；也可以登记版本、检查资源并激活部署。直接 URL 覆盖该环境的版本入口，移除后恢复已激活版本。旧应用重新配置时保持停用，待代码适配完成再启用。菜单、租户、组织、角色授权仍是独立前提，保存入口并不代表用户已有访问权。完整步骤见[微应用接入](../backend/docs/micro-application-onboarding.md)。

## 检查与构建

在本目录执行与 CI 前端任务相同的入口：

```powershell
npm run sdk:check
npm run qiankun:check
npm run lint
npm run typecheck
npm test
npm run build
```

`sdk:check` 校验 `vendor/micro-app-sdk/SOURCE.json`、发行 manifest、文件集合、长度和 SHA-256，拒绝本地漂移。升级 SDK 应在 SDK 仓库生成并交付完整发行物，不能手改 vendor 文件通过检查。跨端 Rust DTO 一致性由仓库 `.github/tests/sdk-consumer-contract.mjs` 检查。

`qiankun:check` 检查活动源码、public 和当前 SDK，拒绝旧微应用桥接、SDK 1 与宿主内置微应用副本。`build` 在生成 dist 后自动对产物再次检查。HTML 通过固定开发依赖 parse5 解析真实 script 元素及属性实体，避免注释和脚本字符串干扰；解码后的 type 按浏览器规则精确匹配 module，不做空白归一化。宿主自己的 Vite ESM 入口合法，子应用 HTML 不允许 module 脚本；独立 SDK 历史 releases 不属于活动交付扫描范围。

`npm test` 执行 package 中明确列出的 Node 契约、组件 fixture、上下文/会话、qiankun 生命周期和静态交付测试；它不等于真实浏览器、PostgreSQL 或生产部署验收。`build` 先执行 TypeScript 构建检查再生成 `dist/`。浏览器工具使用独立依赖与显式入口，见[浏览器检查](tests/browser/README.md)。

## 目录导航

| 路径 | 用途 |
| --- | --- |
| `src/api/` | 平台 HTTP API 和 DTO |
| `src/components/micro/` | qiankun、授权快照和宿主服务 |
| `src/components/layout/` | 应用选择、菜单、权限守卫和宿主布局 |
| `src/pages/` | 宿主页面及应用接入/发布管理 |
| `vendor/micro-app-sdk/` | 已固定、可校验的 SDK 发行物 |
| `tests/` | 契约、组件和浏览器场景 |
| `deploy/` | 静态镜像、缓存策略和历史资源工具 |

生产镜像使用本目录作为构建上下文，由非特权 Nginx 在 8080 提供宿主静态文件，健康路径为 `/healthz`。镜像构建、网关分工和旧 hash 资源保留见[静态交付](deploy/README.md)；CI 实际门禁见[持续集成说明](../.github/README.md)。
