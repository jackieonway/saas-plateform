# 前端微应用开发与接入手册

本文面向微应用开发者、业务后端开发者和平台管理员，说明如何将不同技术栈的前端接入当前 Dante Cloud 系统。从选型、开发、上下文与权限、路由、构建，到上传、授权、联调和上线，均以当前仓库实现为依据。

核对日期：2026-10-14。示例应用编码统一使用 `orders`，示例平台地址使用 `https://saas.example.com`，示例业务 API 使用 `/orders-api`。这些是开发示例，仓库并未预置 orders 应用和 orders API；实施时必须替换为实际配置。已有可运行业务参考是 `work-orders`。

## 目录

1. [接入方式与技术栈选型](#1-接入方式与技术栈选型)
2. [接入前准备](#2-接入前准备)
3. [HTML 微应用最小接入示例](#3-html-微应用最小接入示例)
4. [上下文与生命周期](#4-上下文与生命周期)
5. [各框架开发方法](#5-各框架开发方法)
6. [路由、主题与平台交互](#6-路由主题与平台交互)
7. [业务请求与后端授权](#7-业务请求与后端授权)
8. [静态资源与构建](#8-静态资源与构建)
9. [应用登记、上传与授权](#9-应用登记上传与授权)
10. [本地开发与联调](#10-本地开发与联调)
11. [生产部署与版本回滚](#11-生产部署与版本回滚)
12. [ESM 内部模块接入](#12-esm-内部模块接入)
13. [现有项目改造与特殊类型前端](#13-现有项目改造与特殊类型前端)
14. [验收清单与故障排查](#14-验收清单与故障排查)
15. [仓库参考资料](#15-仓库参考资料)

## 1. 接入方式与技术栈选型

### 1.1 当前支持的两条常规接入路径

| 比较项 | 独立 HTML 微应用 | 第一方 ESM 模块 |
| --- | --- | --- |
| 适用场景 | 独立业务系统、不同框架、独立版本发布 | 与平台紧密集成的内部模块 |
| 创建应用选择 | `HTML` | `ESM` |
| 运行方式 | `IFRAME`，隔离运行 | `TRUSTED`，在宿主页面直挂 |
| 交付内容 | 包含 `index.html` 的 ZIP 或 tar.gz | 宿主源码中的同名 `.tsx` 模块 |
| SDK/契约 | `DanteMicroIframe.connect` | 导出 `mount/update/unmount` 生命周期 |
| 框架与样式 | 自带运行时和样式 | 可以共享宿主 React 和 UI 模块 |
| 更新方式 | 上传、预检、激活独立版本 | 重新构建并发布宿主前端 |
| 是否上传 ESM ZIP | 不适用 | 当前上传接口不支持 |

**新建独立业务前端默认选择 HTML 微应用。** “HTML”表示入口是 HTML 页面，不表示只能使用原生 HTML，也不要求应用脚本只能使用 classic script。React/Vue/Vite 的标准模块产物可以运行在 iframe 中，前提是资源服务满足 MIME 和 CORS 要求。

当前系统使用自有容器、消息桥接和生命周期契约，不要求子应用安装 qiankun、Wujie 或 Module Federation。仅实现其它微前端框架的生命周期，不能自动接入本系统。

### 1.2 不同类型前端的推荐方式

| 类型 | 推荐方式 | 主要适配点 |
| --- | --- | --- |
| React + Vite / Webpack | HTML iframe | 创建 React root、更新上下文状态、销毁 root |
| Vue 3 + Vite | HTML iframe | 创建应用、响应式上下文、应用卸载 |
| Vue 2 / Vue CLI | HTML iframe | 创建实例、更新 observable 状态、销毁实例 |
| Angular | HTML iframe | 显式启动、注入上下文服务、销毁应用与订阅 |
| Svelte / Solid | HTML iframe | 上下文状态适配、框架 dispose/unmount |
| jQuery / 原生 JavaScript | HTML iframe | 容器渲染、事件委托、资源清理 |
| 平台内部 React 页面 | ESM | 宿主同名模块、共享运行时、生命周期 |
| Next.js / Nuxt / Astro | 静态 SPA 或整理后的单入口静态页面优先 | SSR 服务不能随静态上传包启动；多入口静态导出需改造 |
| uni-app H5 / 移动 H5 | HTML iframe | 构建 Web 产物，核对依赖的浏览器能力 |
| Web Components | HTML iframe | 创建自定义元素、传递上下文、清理事件 |
| 小程序、原生 Android/iOS | 单独客户端方案 | 不属于当前 Web 微应用容器可直接加载的产物 |

历史可信 HTML 入口仍有兼容机制，但不作为新应用常规接入方式；参见第 13 节。

## 2. 接入前准备

### 2.1 先确定应用标识和地址

| 配置 | orders 示例 | 用途 |
| --- | --- | --- |
| 应用编码 | `orders` | 应用记录、SDK 握手、token 受众的稳定标识 |
| 宿主挂载路径 | `/micro/orders` | 用户在平台访问的业务地址 |
| 业务子路由 | `/micro/orders/list` | 平台菜单与子应用页面映射 |
| 静态入口 | `/sub-apps/orders/versions/1.0.0/index.html` | 上传后由平台登记的资源地址 |
| API 前缀 | `/orders-api` | 业务后端地址，需要配置网关 |
| 允许的宿主 origin | `https://saas.example.com` | SDK 验证父页面的浏览器来源 |

建议编码使用小写英文、数字和连字符，例如 `work-orders`。SDK 的 `appCode`、应用管理编码、后端 token 受众必须一致。稳定编码不应随着版本号变化。

当前 iframe SDK 要求 `basePath` 为 `/micro/{appCode}`；新应用遵循这一约定。静态资源路径与用户路由是两类地址，不要混用。

### 2.2 团队需要准备的内容

- 前端：支持单入口静态构建，接入 SDK，实现生命周期和上下文更新。
- 业务后端：验证应用 token 和实际权限，按租户、组织及数据授权过滤资源。
- 平台管理员：创建应用、关联已启用机密 OAuth 客户端、配置 scope、菜单与授权。
- 部署人员：配置资源网关、业务 API 反代、HTTPS、CORS 和持久版本目录。

只显示上下文的页面可以没有业务 API，但平台仍要求有效应用 token、关联客户端与授权上下文；没有 token 时宿主不会挂载。

### 2.3 推荐代码结构

```text
orders-frontend/
├── index.html
├── package.json
├── vite.config.ts
├── public/
│   └── iframe-bridge.js
└── src/
    ├── main.tsx / main.ts / main.js
    ├── platform/          # 生命周期、上下文、路由适配、请求封装
    ├── pages/             # 业务页面
    ├── components/
    └── styles/
```

SDK 由仓库 [examples/micro-app-sdk/iframe-bridge.js](../examples/micro-app-sdk/iframe-bridge.js) 复制。记录所用契约版本；SDK 更新后重新构建并发布应用。它是 classic script，通过 `window.DanteMicroIframe` 提供能力，当前没有单独发布的 npm SDK 包。

## 3. HTML 微应用最小接入示例

以下示例只展示平台上下文，不调用假业务接口。将 SDK 与两个文件放在同一交付目录，即可通过平台上传后验证握手、主题、导航与卸载。

### 3.1 index.html

```html
<!doctype html>
<html lang="zh-CN">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>订单应用</title>
    <style>
      body { margin: 0; padding: 24px; font-family: sans-serif; }
      :root[data-theme="dark"] { color-scheme: dark; }
    </style>
  </head>
  <body>
    <div id="app">等待平台提供授权上下文…</div>
    <script src="./iframe-bridge.js"></script>
    <script src="./app.js"></script>
  </body>
</html>
```

### 3.2 app.js

```js
let container = null
let currentProps = null
let unbindAbort = () => {}

function render(props) {
  unbindAbort()
  currentProps = props
  document.documentElement.dataset.theme = props.theme
  const title = document.createElement("h1")
  title.textContent = "订单应用"
  const identity = document.createElement("p")
  identity.textContent = `${props.user.displayName} / ${props.organization.name}`
  const route = document.createElement("p")
  route.textContent = `当前路由：${props.routePath}`
  const listButton = document.createElement("button")
  listButton.textContent = "订单列表"
  listButton.onclick = () => props.onNavigate(`${props.basePath}/list`)
  container.replaceChildren(title, identity, route, listButton)

  const abort = () => {
    if (currentProps !== props) return
    currentProps = null
    container.replaceChildren(document.createTextNode("上下文已更新或失效，等待平台…"))
  }
  props.signal.addEventListener("abort", abort, { once: true })
  unbindAbort = () => props.signal.removeEventListener("abort", abort)
  if (props.signal.aborted) abort()
}

const bridge = window.DanteMicroIframe.connect({
  appCode: "orders",
  allowedHostOrigins: ["https://saas.example.com"],
  container: document.getElementById("app"),
  lifecycle: {
    mount(element, props) {
      container = element
      render(props)
    },
    update(props) {
      render(props)
    },
    unmount() {
      unbindAbort()
      currentProps = null
      container?.replaceChildren()
      container = null
    },
  },
})
```

允许来源填写实际宿主 origin；本地开发可增加 `http://localhost:3000`。origin 只包含协议、主机和可选端口，不包含路径或结尾 `/`。`localhost`、`127.0.0.1`、不同端口均是不同 origin。

直接打开子应用 `index.html` 时没有宿主消息，停留在等待页面是正常行为。不要因此在生产入口中生成模拟 token 或自动绕过握手。

## 4. 上下文与生命周期

### 4.1 MicroProps 2.0

完整类型见 [frontend/src/components/micro/types.ts](../frontend/src/components/micro/types.ts)。后端上下文接口契约为 `1.0`，宿主映射后传给前端的是 `MicroProps.contractVersion = "2.0"`；不要把两者当成版本冲突。

| 字段 | 含义与使用规则 |
| --- | --- |
| `appCode` | 当前应用编码 |
| `basePath` | 宿主挂载路径，如 `/micro/orders` |
| `routePath` | 宿主当前 pathname，包含 `basePath` |
| `user` | 用户 ID、用户名、显示名 |
| `tenant`、`organization` | 服务端确认的当前租户与组织 |
| `organizationScopeIds` | 组织范围上限；`null` 仅用于实际平台根上下文，不代表普通租户可跨租户 |
| `roles` | 当前用户的角色信息 |
| `permissions` | 操作权限码数组 |
| `dataGrants` | 当前组织成员关系的独立资源/操作数据授权 |
| `menus` | 当前应用对用户可见的菜单树 |
| `token`、`tokenExpiresAt` | 本应用专用短期 token 和到期时间 |
| `contextVersion` | 服务端授权内容版本 |
| `contextEpoch` | 宿主推送序号，用于区分旧上下文 |
| `expiresAt` | 授权展示上下文到期时间 |
| `deploymentVersion`、`entryVersionId` | 当前静态发布版本与记录 ID，可为空 |
| `theme` | 实际主题 `light` 或 `dark` |
| `signal` | 上下文替换、过期、组织切换或卸载时取消旧工作 |
| `onNavigate(path)` | 请求宿主切换本应用路由 |
| `onNotify(message, variant)` | 请求宿主显示 Toast |

iframe 接收到的是序列化 DTO。`signal` 和两个回调由 SDK 在子页面内创建；无需也不应该通过 `postMessage` 自己序列化这些对象。

### 4.2 生命周期职责

| 方法 | 必须性 | 职责 |
| --- | --- | --- |
| `mount(container, props)` | 必须 | 创建应用实例，接收初始上下文，渲染到给定容器 |
| `update(props)` | 可选，正式应用建议实现 | 替换最新上下文、同步路由与主题、处理授权变化 |
| `unmount()` | 必须 | 销毁实例，清理订阅、监听器、定时器和全局资源 |

生命周期可返回 Promise，必须在实际工作结束后完成。不要在其中等待永不结束的订阅或长期轮询；需要保证初始化失败和取消时可以结束。

SDK 串行执行生命周期，但新上下文到来时会立即取消旧 signal，不会等慢请求结束后才取消。宿主至多约每 20 秒刷新上下文，并在 token 到期前续期；主题、路由、token 和授权内容变化均可能触发更新。

没有 `update` 的应用收到更新时会卸载后重新挂载，输入、滚动位置等页面状态可能丢失。

### 4.3 上下文更新时的数据处理

每次更新都要替换 token、signal 和权限，不能仅在首次挂载时读取一次。

- 用户、租户或组织变化：清空旧业务列表、详情、请求缓存和涉及身份的草稿。
- 仅 token 续期：替换请求凭据，可保留仍有权限的页面输入。
- 权限或数据授权变化：重新评估按钮与可见数据，撤权后的旧数据不能继续展示。
- 主题或路由变化：同步显示，并保留合法的本地状态。
- signal 取消或上下文到期：停止请求和后台工作，隐藏旧授权视图。

React Query 等缓存应至少按应用、用户、租户、组织区分；权限或范围变化时还要失效相关查询。不要为了续期把 token 本身放入持久缓存键或日志。

### 4.4 iframe SDK 对外能力

| 方法 | 行为 |
| --- | --- |
| `bridge.current()` | 返回当前可用上下文，否则返回 `null` |
| `bridge.requestJson(url, options)` | 自动添加当前 Bearer token、取消信号、`credentials: "omit"`，解析后再次校验上下文 |
| `bridge.downloadCsv(file)` | 请求宿主按当前授权发起 CSV 下载 |
| `bridge.dispose()` | 停止消息连接、取消请求并安排卸载；当前返回值不是可等待的卸载 Promise |

子应用不要在自己的 `unmount` 中再次调用 `bridge.dispose()`。页面销毁由 SDK 和宿主处理，应用负责自身资源清理。

## 5. 各框架开发方法

本节代码为入口适配示例，业务组件由应用自行实现。HTML iframe 方案中，每个应用自己打包框架及依赖，不依赖宿主全局 React、Vue 或 UI 库。

### 5.1 React + Vite

在 `public/iframe-bridge.js` 放置 SDK，在 `index.html` 中先加载 SDK，再加载应用入口：

```html
<div id="app"></div>
<script src="./iframe-bridge.js"></script>
<script type="module" src="/src/main.tsx"></script>
```

这里 `/src/main.tsx` 是 Vite 源码入口，仅存在于源文件；上传的是 `vite build` 处理后的 `dist`，不能直接上传源码 HTML。

入口示例：

```tsx
import { createRoot, type Root } from "react-dom/client"
import { App } from "./App"
import "./styles.css"

let root: Root | null = null
let detachAbort = () => {}

function renderContext(props: MicroProps) {
  detachAbort()
  document.documentElement.dataset.theme = props.theme
  root!.render(<App platform={props} />)
  const abort = () => root?.render(<p>授权上下文已失效，等待平台…</p>)
  props.signal.addEventListener("abort", abort, { once: true })
  detachAbort = () => props.signal.removeEventListener("abort", abort)
  if (props.signal.aborted) abort()
}

window.DanteMicroIframe.connect({
  appCode: "orders",
  allowedHostOrigins: ["https://saas.example.com"],
  container: document.getElementById("app")!,
  lifecycle: {
    mount(container: HTMLElement, props: MicroProps) {
      root = createRoot(container)
      renderContext(props)
    },
    update: renderContext,
    unmount() {
      detachAbort()
      root?.unmount()
      root = null
    },
  },
})
```

`MicroProps` 按仓库完整类型建立本应用的类型契约；`window.DanteMicroIframe` 需要在应用 `.d.ts` 文件中声明，不能把这个片段当成包含所有类型声明的独立项目。独立应用宜保留自有类型文件并在 SDK 升级时核对，不要形成对宿主构建目录的运行时依赖。

`App` 内通过 Context 或状态容器向业务组件提供 `platform`。请求副作用应依赖当前上下文并清理旧任务；不要将旧 props 永久保存在模块常量中。开发时使用 StrictMode，需要保证 effect 的注册与清理可重复执行。

### 5.2 Vue 3 + Vite

SDK 加载方式与 React 相同，入口模块按 Vue 项目实际文件替换。推荐用 `shallowRef` 整体替换上下文，避免深度代理 signal 和其它平台对象：

```js
import { createApp, shallowRef } from "vue"
import App from "./App.vue"
import "./styles.css"

const platform = shallowRef(null)
let app = null
let detachAbort = () => {}

function apply(props) {
  detachAbort()
  document.documentElement.dataset.theme = props.theme
  platform.value = props
  const abort = () => { if (platform.value === props) platform.value = null }
  props.signal.addEventListener("abort", abort, { once: true })
  detachAbort = () => props.signal.removeEventListener("abort", abort)
  if (props.signal.aborted) abort()
}

window.DanteMicroIframe.connect({
  appCode: "orders",
  allowedHostOrigins: ["https://saas.example.com"],
  container: document.getElementById("app"),
  lifecycle: {
    mount(container, props) {
      apply(props)
      app = createApp(App)
      app.provide("platform", platform)
      app.mount(container)
    },
    update: apply,
    unmount() {
      detachAbort()
      app?.unmount()
      app = null
      platform.value = null
    },
  },
})
```

页面通过 `inject("platform")` 读取 ref，`null` 时显示等待或失效状态。watcher 发起的业务请求也要绑定当前 signal；应用卸载不会代替你清理所有外部库订阅。

仓库 [独立工单前端](../business-apps/work-orders/frontend/README.md) 使用 Vue 3、真实 API 和 classic IIFE 产物，可作为完整实现参考。

### 5.3 Vue 2 / Vue CLI

可使用 `Vue.observable({ props: null })` 或 Vuex 保存上下文。`mount` 创建 Vue 实例并挂载到容器内的专用节点，`update` 替换 `state.props`，`unmount` 调用 `$destroy()` 后清空容器。

Vue CLI 构建通常设置 `publicPath: "./"`，并确认实际产物没有指向根路径的资源。Vue 2 的插件、document 事件和全局总线要单独退订；收到 signal 取消后立即隐藏旧数据。

### 5.4 Angular

将默认自动 bootstrap 改为由 SDK 的 `mount` 启动。上下文用服务、Signal 或 BehaviorSubject 传递，保留 bootstrap 返回的 `ApplicationRef` 或 `NgModuleRef`。

```text
mount(container, props)
  → 在 container 中建立 Angular 根元素
  → 启动应用，取得应用引用
  → 把 props 推送到 PlatformContextService

update(props)
  → 替换上下文与取消信号
  → 同步内部路由、主题和权限

unmount()
  → 终止业务订阅与后台任务
  → destroy 应用引用
  → 清空本应用容器
```

Angular 不同版本的 bootstrap 和销毁 API 存在差异，按项目版本选择 `bootstrapApplication` 或模块 bootstrap，不要把 Vue/React 的销毁调用套到 Angular。资源使用版本目录可用的相对地址；不要将 HTML `base href` 设成宿主 `/micro/orders/` 后导致资源请求落到业务路由。Angular Router 的 LocationStrategy 需要单独适配为由平台驱动的内部路由，默认浏览器 history 不能自动同步父页面。

### 5.5 Svelte、Solid 与 Web Components

- Svelte 5 使用框架 `mount/unmount` API；旧版本按组件构造与 `$destroy()` 适配。通过 store 或组件 props 接收平台上下文。
- Solid 使用 `render` 创建视图并保存 disposer，通过 signal 更新上下文，卸载时调用 disposer。
- Web Components 创建元素并通过属性传入上下文；移除节点时在 `disconnectedCallback` 清理订阅，外部资源仍需显式回收。

不同框架的入口只是生命周期适配差异，上下文、业务请求和发布约束一致。

### 5.6 jQuery 与原生 JavaScript

可以直接采用第 3 节的结构。jQuery 使用容器级事件委托，并加事件命名空间：

```js
$(container).on("click.orders", "[data-action='refresh']", refresh)
// unmount 中执行：
$(container).off(".orders")
```

不要在每次 `update` 中重复注册相同监听器。事件处理读取最新上下文；HTML 拼接使用转义或 `.text()`，避免把用户输入和接口字段直接拼入 `innerHTML`。

## 6. 路由、主题与平台交互

### 6.1 宿主路由与内部路由

用户访问 `/micro/orders/list` 时，浏览器地址属于宿主；iframe 自身地址仍是静态版本入口。内部路径可按以下方式取得：

```js
function childPath(props) {
  return props.routePath.slice(props.basePath.length) || "/"
}

// 页面内跳转优先使用完整宿主路径，HTML iframe 与 ESM 均清晰可用。
props.onNavigate(`${props.basePath}/list`)
```

HTML iframe 导航仅允许本应用范围内的路径，不能用回调打开其它应用、外部 URL 或绕过平台菜单授权。菜单中的页面路径应与业务页面映射一致。

React Router 推荐使用内存路由；Vue Router 推荐 `createMemoryHistory()`。初始化后，在每次 `update` 将内部路由同步到 `childPath(props)`；用户操作通过 `onNavigate` 请求父页面跳转，再接收新的上下文。不要依赖 iframe 的 `window.history` 改变父页面 URL。

当前宿主通过 `useLocation().pathname` 提供 `routePath`，**没有在 MicroProps 中提供独立的 search/hash 字段**。导航回调接受的 URL 可以含 query/hash，但不能假设这些内容会随 props 完整回传。需要分享和刷新恢复的记录 ID，优先编码为 pathname，例如 `/micro/orders/detail/123`。

### 6.2 主题与 UI 样式

使用 `props.theme` 设置本应用的主题开关，例如 `data-theme`、`class="dark"` 或组件库主题配置。

iframe 不继承宿主 CSS。字体、图标、CSS reset 和组件库主题都应由子应用自行提供。可在应用中统一采用设计规范，但不能直接引用宿主 DOM 来读取样式。

### 6.3 通知

```js
props.onNotify("保存成功", "success")
```

支持 `default/success/error/warning/info`，用于界面 Toast。它不是持久站内通知接口，也不会自动向其他用户发送业务提醒。旧上下文回调失效后不能继续使用。

### 6.4 CSV 导出

从业务 API 获取已经由服务端过滤的 CSV，然后调用：

```js
await bridge.downloadCsv({
  resource: "order",
  filename: "订单.csv",
  content: csvText,
})
```

`csvText` 是业务 API 返回的文本；`requestJson` 只适用于 JSON，不直接用于解析 CSV。CSV 请求需按第 7 节保护文本读取阶段。

宿主要求 `order:export` 操作权限和同资源 `export` 数据 grant，并检查当前上下文。内容最多 4 MiB UTF-8，文件名使用安全的 `.csv` 名称。Promise 成功仅表示宿主已发起下载，不表示浏览器已经保存文件。

默认 iframe sandbox 为 `allow-scripts allow-forms`，没有下载权限；导出使用宿主代理，不通过给 iframe 增加 `allow-downloads` 来绕开授权。其它文件格式的下载需要另行实现平台能力，当前 CSV broker 不支持任意文件。

## 7. 业务请求与后端授权

### 7.1 使用 SDK 请求 JSON

在用户交互或应用挂载完成后调用真实业务接口：

```js
async function loadOrders() {
  const snapshot = bridge.current()
  if (!snapshot) return
  try {
    const result = await bridge.requestJson("/orders-api/orders")
    if (bridge.current() !== snapshot || snapshot.signal.aborted) return
    // 将 result 写入本应用状态；此处替换为真实 DTO 的处理。
  } catch (error) {
    if (error.name === "AbortError") return
    if (bridge.current() !== snapshot) return
    snapshot.onNotify("订单加载失败", "error")
  }
}
```

提交 JSON 时显式设置 `Content-Type`：

```js
await bridge.requestJson("/orders-api/orders", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({ title: "新订单" }),
})
```

当前 `requestJson` 总会解析 JSON；HTTP 204、文本响应不能直接用它。HTTP 非成功状态抛出通用 Error，**没有结构化的 `error.status` 属性**。需要区分 401、403、409 时，应用应建立自己的请求封装，而不是假设 SDK 已提供业务错误 DTO。

### 7.2 自定义请求封装

以下实现同时检查 fetch 前、响应体解析后以及当前上下文，适用于需要状态码的 JSON API：

```js
async function requestBusinessJson(url, options = {}) {
  const snapshot = bridge.current()
  const check = () => {
    if (!snapshot || snapshot.signal.aborted || bridge.current() !== snapshot ||
        Date.parse(snapshot.expiresAt) <= Date.now() ||
        Date.parse(snapshot.tokenExpiresAt) <= Date.now()) {
      throw new DOMException("上下文已失效", "AbortError")
    }
  }
  check()
  const headers = new Headers(options.headers)
  headers.set("Authorization", `Bearer ${snapshot.token}`)
  const response = await fetch(url, {
    ...options,
    headers,
    credentials: "omit",
    signal: options.signal
      ? AbortSignal.any([snapshot.signal, options.signal])
      : snapshot.signal,
  })
  check()
  const body = response.status === 204 ? null : await response.json()
  check()
  if (!response.ok) {
    const error = new Error(`业务请求失败：${response.status}`)
    error.status = response.status
    error.body = body
    throw error
  }
  return body
}
```

此例要求目标浏览器支持 `AbortSignal.any`；旧浏览器采用 SDK 内的 AbortController 与事件合并方式。API 返回非 JSON 错误页时，需额外根据 Content-Type 处理，不能把这个 JSON 示例当作任意接口封装。

CSV、上传和其它响应类型同样必须绑定 signal，并在 `response.text()`、`response.blob()` 或解析完成后再次检查上下文。

### 7.3 前端权限表达

```js
function canOperate(props, resource, action) {
  return props.permissions.includes(`${resource}:${action}`) &&
    props.dataGrants.some(grant =>
      grant.resource === resource && grant.action === action)
}
```

以上只是按钮显示示例，不实现记录级授权。权限码由实际业务设计决定，不是系统为每个应用自动生成的固定 CRUD 集合。涉及资源数据范围的操作，需要同时具备操作权限与对应 grant。

`roles` 不能直接推导数据范围；`dataGrants` 是独立授权。SELF、ORG、ORG_SUBTREE、TENANT 的最终解释和资源归属检查由后端完成，且受当前组织范围上限约束。前端收到 TENANT 也不能自行宣布可访问租户所有数据。

### 7.4 业务后端必须执行的校验

```text
浏览器携带应用 Bearer token
  → 业务后端用自身客户端凭据调用 /oauth/introspect
  → 验证 active、aud、用户、租户、组织
  → 使用应用 token 获取当前 /api/apps/{code}/context
  → 验证操作权限、dataGrants、组织范围上限
  → 按可信租户与组织范围查询业务库
  → 对记录归属和本次操作进行校验
```

参考 [authorize.mjs](../examples/app-backend/authorize.mjs)。该参考组件逐请求内省并读取当前上下文，不缓存正向授权结果。新增业务后端可用 Rust、Java、Go、Node 等实现同等语义。

创建记录时的租户、组织、创建人由后端从可信身份取得，不信任请求 body 中同名字段。列表、总数、详情、修改、删除、导出都要应用同样的范围约束。

应用 token 与平台 session 是不同凭据。不要读取宿主登录 token，不要把应用 token 放进 URL、持久存储、埋点或日志。客户端 secret 只能存在于业务后端。

### 7.5 CORS 和 opaque iframe

iframe 没有 `allow-same-origin`，浏览器把它视为 opaque origin。即使 API 地址与平台域名相同，请求也可能携带 `Origin: null`，不能按普通同源页面处理。

业务 API 需要：

- 支持无 Cookie 的 Bearer 请求，并验证每个受保护请求。
- 正确响应 OPTIONS，允许实际使用的 HTTP 方法。
- 允许 `Authorization`、`Content-Type` 以及业务使用的其它请求头。
- 对许可的 opaque 请求返回 `Access-Control-Allow-Origin: null`，或在适当的无凭据 API 设计中使用 `*`；返回精确动态来源时设置 `Vary: Origin`。
- 成功响应和错误响应都提供对应 CORS 头，避免 401/403 被浏览器表现成网络错误。

允许 `null` 本身不能识别合法微应用，因为其它 opaque 页面也会产生该来源。安全边界是应用 token、受众与后端授权，不是 CORS。

仓库开发代理只为特定已有接口提供处理，不会自动为新建 `/orders-api` 提供代理与预检。新增业务 API 需要配置自己的网关和 CORS。

## 8. 静态资源与构建

### 8.1 Vite 配置

保留原项目框架插件，设置相对资源基址：

```ts
import { defineConfig } from "vite"
// plugins 使用本项目的 React/Vue 等插件。
export default defineConfig({
  base: "./",
  build: {
    outDir: "dist",
    sourcemap: false,
  },
})
```

`base: "./"` 处理构建器管理的资源，不会自动改写业务代码中写死的 `fetch("/...")`、图片 URL 或第三方组件地址。分别检查所有硬编码路径。

`public` 文件通过相对 URL 引用；模块导入的图片、字体由构建器处理。API 的 `/orders-api` 是有意使用的网关根路径，不能为了资源相对化改成 `./orders-api`，否则请求会进入静态版本目录。

Webpack 使用适合项目加载方式的 `output.publicPath`（如 `auto` 或相对路径），Vue CLI 使用 `publicPath: "./"`；其它构建器同样核对最终产物。

### 8.2 上传目录要求

```text
dist/
├── index.html
├── iframe-bridge.js
└── assets/
    ├── main-<hash>.js
    ├── main-<hash>.css
    └── images-or-fonts...
```

- 包根目录包含 `index.html`；系统也支持压缩包内多包一层 `dist/`。
- 一个版本交付一个 HTML 入口；其它 HTML 不能作为普通 ASSET 通过现有预检。
- JS/CSS/图片/字体等必须随包完整交付，动态 import 的 chunk 也要包含。
- 不包含 `.env`、secret、私有业务数据、node_modules、源仓库或服务端配置。
- 非空文件，禁止路径穿越、链接、特殊文件和重复路径。
- 保留第三方依赖要求的许可文件。

当前限制：压缩包最多 20 MiB，解压文件总量最多 16 MiB，单文件最多 4 MiB，入口外资源最多 48 项。大型组件库、图片和代码分块可能触发限制；构建时控制体积、资源数量并确认每个 chunk 大小。

### 8.3 Windows 构建和 ZIP 示例

在子应用目录执行项目自己的构建命令，例如：

```powershell
npm ci
npm run build
Compress-Archive -Path ./dist/* -DestinationPath ./orders-1.0.0.zip
```

输出文件名选择尚不存在的新文件名；发布新内容使用新版本号。上传 `orders-1.0.0.zip`，不上传源码目录。仅当项目有 package-lock 时使用 `npm ci`，其它包管理器按项目锁文件执行对应安装命令。

## 9. 应用登记、上传与授权

### 9.1 创建应用

在“应用管理”新建应用，选择 `HTML`。快速接入开启时，只需填写名称和编码，系统生成 `/micro/{code}` 挂载路径、未发布占位入口和首页 MICRO 菜单。

快速接入不会自动创建 OAuth 客户端，也不会自动授予租户、组织或角色权限。应用必须关联已启用机密客户端，按业务需求配置 scope；密钥交给业务后端保存。

### 9.2 发布版本

进入应用的“发布版本”：

1. 选择正确环境，例如 `development` 或 `production`。
2. 填写新版本号，例如 `1.0.0`。
3. 上传 ZIP 或 tar.gz，执行“上传并登记”。
4. 查看自动 HTTP 预检结果；失败时按诊断修复网关、资源或构建包。
5. 状态为 `PASSED` 后点击“激活”。

上传不会自动激活。文件写入 `MICRO_APPS_DIR/<code>/versions/<version>/`；当前环境的部署指针决定加载哪一版本。

自动化上传接口为：

```text
POST /api/applications/{id}/releases/upload
  ?environment=development&version=1.0.0&entryType=HTML&format=zip
```

请求体是原始压缩包字节，不是 multipart 表单；tar.gz 对应 `format=tar.gz`。接口需要平台管理授权，具体响应和权限见 [发布 API](./application-release-api.md)。不要从子应用持有的业务 token 调用管理发布接口。

### 9.3 配置菜单

| 菜单 | path | linkTarget | linkUrl |
| --- | --- | --- | --- |
| 首页 | `/micro/orders` | `MICRO` | 留空 |
| 订单列表 | `/micro/orders/list` | `MICRO` | 留空 |

菜单 path 对应宿主路由，不填写 `/sub-apps/.../index.html`。按实际业务建立按钮菜单和权限码，并分配给相应角色；角色权限不能超出租户、组织已经获得的授权范围。

### 9.4 完成分层授权

依次核对：

1. 租户应用授权和菜单授权。
2. 当前组织应用授权和菜单授权。
3. 当前用户角色的菜单及操作权限。
4. 用户具有相应角色。
5. 当前组织成员关系具有业务所需独立数据授权。

`/api/auth/profile` 返回当前租户和组织允许使用的启用应用；`/api/auth/menus` 再按角色和菜单授权筛选导航。应用存在、包已激活，不等于用户能看到和进入。

### 9.5 用户访问时发生什么

```text
访问 /micro/orders/*
  → 宿主检查个人可用应用和菜单路由
  → 获取 orders 专用短期 token
  → 获取 /api/apps/orders/context
  → 核对运行方式、入口和有效授权
  → 加载 iframe 静态入口
  → SDK 验证允许宿主，建立专属 MessageChannel
  → mount；随后按变化 update；离开或失效时卸载
```

客户端关联缺失、授权失效、上下文过期或加载失败时，宿主停止挂载。不要在子应用实现“挂载失败就自己登录平台”的备用流程。

## 10. 本地开发与联调

### 10.1 页面开发与平台集成分开验证

独立运行开发服务器可以调试布局、组件与业务逻辑；完整 SDK 握手和权限只能在宿主中验证。

如需开发预览上下文，使用明确的开发入口和模拟服务，不放入生产启动路径；不使用真实 session、secret 或长期 token。上线前通过真实平台和真实业务后端验收。

### 10.2 推荐使用构建版本联调

1. 启动平台后端、数据库和宿主前端。
2. 将 SDK 的允许宿主配置设为实际开发 origin。
3. 构建子应用，并通过应用管理上传 `development` 版本。
4. 预检、激活，使用已授权用户从平台菜单进入。
5. 修改后生成新版本再上传，不覆盖已有版本文件。

宿主 Vite 默认从 `deploy/micro-apps` 服务版本静态资源；可通过绝对路径 `MICRO_APPS_DEV_DIR` 指定其它目录。后端写入目录必须和 Vite 读取目录对应，否则上传成功后预检会因资源不可达失败。

### 10.3 命令行打包兼容方式

在仓库根目录执行：

```powershell
node scripts/package-microapp.mjs --app orders --environment development --version 1.0.0 --input path/to/orders/dist
```

它生成 `deploy/micro-apps/orders/versions/1.0.0/` 和历史登记接口的 `release.json`。**该命令只打包，不自动登记数据库、预检或激活。** 常规开发优先使用管理页面上传；已有自动化集成按 [独立部署文档](./application-independent-deployment.md) 完成后续登记。

### 10.4 开发服务器与 HMR

HMR 对普通独立页面可用，但 iframe 中涉及 opaque origin、模块 CORS、WebSocket、宿主白名单与入口预检。当前新建 HTML 应用的常规流程是上传版本包，不能仅填一个 Vite dev URL 就视作完成接入。

如需开发服务器直连，需单独设计受控开发入口并配置资源及 WebSocket，不能改变生产隔离策略来让 HMR 工作。稳定的集成验收优先使用构建产物。

## 11. 生产部署与版本回滚

### 11.1 网关和存储

仓库提供 [Caddy](../deploy/Caddyfile) 和 [Nginx](../deploy/nginx.conf) 示例。生产部署需要：

- 为版本目录配置持久的 `MICRO_APPS_DIR`，后端可写、静态网关只读。
- 正确提供 HTML、JS、MJS、CSS、字体等 MIME。
- `/sub-apps/...` 缺文件严格返回 404，禁止回退成宿主 HTML。
- 宿主 `/micro/...` 刷新时回退到宿主 SPA 的 `index.html`。
- 版本公开静态资源提供匿名 CORS；模块请求可能携带 `Origin: null`。
- HTML 使用 `no-cache`，成功的不可变版本资源可长期缓存，错误响应不缓存。
- 单独配置业务 API 反代和 CORS，不把静态资源的通用 CORS 规则扩散到 OAuth 或平台全部接口。

### 11.2 平台生产来源配置

按实际部署配置：

```dotenv
APPLICATION_ENVIRONMENT=production
PUBLIC_APP_URL=https://saas.example.com
MICRO_GATEWAY_ORIGIN=https://saas.example.com
MICRO_ALLOWED_ORIGINS=https://saas.example.com
```

这些是平台部署参数。子应用 `allowedHostOrigins` 是另一项构建配置：前者控制服务端能检查哪些资源来源，后者验证父页面来源，两者不能互相代替。

生产预检要求 HTTPS。后端必须能访问配置的网关；容器中的 `localhost` 指向容器自身，不能用开发机浏览器能访问来证明后端也能访问。

当前 Caddy 示例的 TLS 开启、前置 HTTPS 和卷配置详见 [独立版本部署](./application-independent-deployment.md)，不能只替换域名就假定已有 HTTPS。

### 11.3 版本管理和回滚

同一应用和版本对应不可变文件，新内容使用新版本号。相同版本号跨环境上传完全相同文件时可复用已有目录，不同文件会被拒绝。

升级时上传新版本、预检、激活。回滚选择已有版本记录切换部署指针，保留旧目录供回滚和仍在运行的用户加载旧 chunk。应用授权保持稳定，发布动作不会自动改变租户、组织和角色权限。

版本切换不应被当作即时终止所有业务操作的机制；业务 API 仍通过 token、会话与实时授权控制访问。

## 12. ESM 内部模块接入

### 12.1 文件与注册规则

新增 `frontend/src/micro-apps/orders.tsx`。当前 [registry.ts](../frontend/src/micro-apps/registry.ts) 使用 `import.meta.glob("./*.tsx")` 将同目录 `.tsx` 模块包含进宿主构建，因此文件名必须与应用编码一致；当前自动发现不包括子目录或任意 `.ts/.vue` 文件。

新建应用时选择 `ESM`，宿主按编码关联已包含的模块。`/src/micro-apps/orders.tsx` 是模块注册标识，不是部署后向公网提供 TSX 源码的地址。

### 12.2 React 生命周期示例

```tsx
import { createRoot, type Root } from "react-dom/client"
import type { MicroLifecycle, MicroProps } from "@/components/micro/types"

let root: Root | null = null
let detachAbort = () => {}

function render(props: MicroProps) {
  detachAbort()
  root!.render(
    <section>
      <h1>订单模块</h1>
      <p>{props.user.displayName} / {props.organization.name}</p>
      <button onClick={() => props.onNavigate(`${props.basePath}/list`)}>
        订单列表
      </button>
    </section>,
  )
  const abort = () => root?.render(<p>授权上下文已失效</p>)
  props.signal.addEventListener("abort", abort, { once: true })
  detachAbort = () => props.signal.removeEventListener("abort", abort)
  if (props.signal.aborted) abort()
}

export const __DANTE_MICRO_APP__: MicroLifecycle = {
  name: "orders",
  mount(container, props) {
    root = createRoot(container)
    render(props)
  },
  update: render,
  unmount() {
    detachAbort()
    root?.unmount()
    root = null
  },
}
```

也可直接导出 `mount/unmount/update`，形状见 `MicroEsmModule`。ESM 模块不调用 iframe SDK；业务请求可使用 [context-client.mjs](../examples/micro-app-sdk/context-client.mjs) 的 `createContextClient`，并在更新时替换上下文、卸载时 dispose。

### 12.3 构建、样式和权限

完成模块后重新构建宿主 `frontend`，发布宿主前端，再在对应构建中创建/使用 ESM 应用。沿用相同的客户端关联、菜单、租户、组织、角色与数据授权要求。

模块可共享宿主 React、UI 组件及样式，但宿主上下文 Provider 不一定自动覆盖另一个 React root；需要的 Provider 在模块中显式配置。全局 CSS、全局事件和模块单例不会获得 iframe 隔离，必须避免污染宿主并在卸载时清理。

完整示例见 [report-center.tsx](../frontend/src/micro-apps/report-center.tsx)。ESM 模块变更随宿主发布，不能通过 HTML 版本上传按钮独立更新。

## 13. 现有项目改造与特殊类型前端

### 13.1 已有独立 SPA

1. 保留业务组件，增加平台适配层，移除生产入口的自动挂载。
2. 将创建与销毁应用移动到 SDK 生命周期。
3. 用平台上下文替代原先自行保存的登录身份；如仍需独立访问，明确区分入口与认证流程。
4. 请求封装改用应用 token、当前 signal 和迟到响应检查。
5. 路由改为宿主驱动，主题和权限在 update 中同步。
6. 清理根路径资源依赖，生成符合限制的静态包。
7. 配置业务后端授权、CORS、网关，再完成发布与分层授权。

不是只给现有 URL 外面套一个 iframe：未接入 SDK 的应用无法完成当前平台握手，也不会自动获得上下文。

### 13.2 Next.js / Nuxt / SSR

能够静态化并整理为单入口的应用可按 HTML 方式接入。Next.js 静态导出、Nuxt generate、Astro 多页输出通常会产生多个 HTML 和框架特定资源路径，不能假设原始导出目录直接满足当前上传规则。

需要 SSR、Server Actions、服务端中间件或 Node API 的项目，需要独立部署服务。当前 ZIP 上传不会安装 Node 依赖或启动 SSR 服务。外部服务 iframe 集成需通过受控的兼容集成路径验证入口、来源、SDK、CORS、路由和发布契约，不是管理页面上传的标准流程。

### 13.3 H5、浏览器存储和第三方能力

Web/H5 构建可以接入，但不能假设 iframe 中可使用 localStorage、IndexedDB、Service Worker、顶层跳转、弹窗、文件下载或原生桥接。默认 opaque sandbox 下这些能力可能不可用或受限。

认证信息保存在内存上下文；需要长期保存的业务偏好通过授权业务 API 保存。涉及支付跳转、摄像头、剪贴板、上传、第三方登录等能力，先用实际目标浏览器验证，并按产品需求建立受控宿主能力，不直接放宽 sandbox。

### 13.4 历史可信 HTML

历史自有可信代码可由平台管理员显式设置 `TRUSTED`，通过 `window.__DANTE_MICRO_APP__` 暴露生命周期。现有可信 HTML 加载器按 classic script 执行，预检拒绝原生 `type="module"` 和 modulepreload；普通 Vite 产物应使用 iframe HTML。

该方式在宿主页执行代码，资源归属和卸载要求不同，详见 [接入与上线指南](./microfrontend-integration.md)。不要把旧 `/sub/vue/`、`/sub/jquery/` 开发代理配置当成新应用生产发布规范。

## 14. 验收清单与故障排查

### 14.1 开发完成检查

- [ ] appCode 与平台应用编码、后端受众一致。
- [ ] SDK 为当前契约版本，显式配置正确宿主 origin。
- [ ] mount、update、unmount 完整；生命周期 Promise 可实际结束。
- [ ] 请求使用最新 token、signal，并检查解析后的上下文。
- [ ] 组织切换、授权变化、过期时旧数据立即停止展示。
- [ ] 权限按钮和数据范围分别处理，后端实现实际授权。
- [ ] 路由能从 props 恢复，前进/后退与平台菜单同步。
- [ ] 主题变化生效，事件和订阅没有重复注册。
- [ ] 上传目录只有单 HTML 入口与完整静态资源，符合体积/数量限制。
- [ ] 产物不含密钥、session、`.env` 或真实私有数据。

### 14.2 平台集成验收

| 场景 | 应验证的结果 |
| --- | --- |
| 未授权用户直接访问地址 | 不挂载应用，不返回越权业务数据 |
| 普通用户与管理员分别访问 | 菜单、按钮和数据范围符合各自权限 |
| 刷新深层路由 | 宿主 SPA 正常加载，子应用恢复对应页面 |
| 组织切换时旧请求未完成 | 旧请求取消；忽略取消的迟到响应也不能回写 |
| token 续期 | 使用新 token，不丢失仍合法的编辑状态 |
| 撤销应用、菜单或数据授权 | 停止非法展示或操作，业务 API 拒绝越权 |
| 业务 API 返回 401 | 停止旧授权请求和视图，不自行续签平台 session |
| 业务 API 返回 403 | 清理失去读取权限的数据，暂停被拒绝操作 |
| 业务 API 返回 409 | 按业务冲突规则处理，不误当作身份失效 |
| 多次进入和离开 | 无遗留计时器、重复订阅和跨组织缓存 |
| 激活新版本、回滚旧版本 | 正确加载目标入口，历史 chunk 可继续获取 |
| 资源缺失 | 返回真实 404，不把宿主 HTML 当作 JS |
| CSV 导出 | 服务端范围过滤与宿主权限校验均通过 |

### 14.3 常见问题

| 表现 | 优先检查 |
| --- | --- |
| 应用不存在于导航 | 租户/组织应用授权、菜单授权、用户角色、应用启用状态 |
| token/context 获取失败 | 应用关联客户端、客户端启用状态、scope、用户会话与应用授权 |
| Iframe handshake timed out | SDK 是否加载、appCode、allowedHostOrigins、脚本执行错误 |
| 页面一直等待上下文 | 是否从平台进入；直接打开入口不会自动挂载 |
| module script 被 CORS 拦截 | 版本静态服务是否允许 opaque `Origin: null` 匿名获取 |
| API 报 CORS 错误 | OPTIONS、Authorization 头、错误响应 CORS、业务网关配置 |
| JS 报 MIME 或 Unexpected token `<` | 请求是否返回宿主 HTML fallback；资源基址是否错误 |
| 动态 chunk 404 | 上传包是否包含 chunk，版本目录是否完整，是否误删历史版本 |
| 上传包被拒绝 | 单入口 HTML、文件数量、体积、空文件、敏感文件、重复版本 |
| 上传成功但 HTTP 预检失败 | 后端是否能访问网关，挂载目录、来源白名单、HTTPS 和 MIME |
| 切换组织后仍显示旧数据 | 是否处理 signal，是否检查迟到响应，缓存键是否隔离 |
| 输入频繁丢失 | 是否缺少 update，是否每次续期都重建应用或清空所有状态 |
| 页面内部跳转但父地址不变 | 是否直接操作 iframe history，是否调用 onNavigate |
| 查询参数无法恢复 | 当前 routePath 只有 pathname，应使用路径参数或扩展契约 |
| localStorage 抛 SecurityError | 默认 iframe 是 opaque origin，改用内存/服务端存储 |
| Toast 正常但没有站内通知 | onNotify 只显示 Toast，业务通知需独立后端接口 |
| ESM 提示模块未包含在构建中 | 文件名、`.tsx` 后缀、目录位置、宿主是否已重新构建发布 |
| CSV 下载被拒绝 | resource 对应 export 权限/dataGrant、当前上下文、文件名和容量 |

### 14.4 可用仓库验证命令

平台契约与现有示例测试，在仓库根目录使用 Node 24：

```powershell
node --test tests/micro-context-runtime.mjs tests/micro-sandbox-lifecycle.mjs tests/micro-iframe-protocol.mjs tests/package-microapp.mjs mini-services/micro-vue/smoke-test.cjs mini-services/micro-jquery/smoke-test.cjs
node --test tests/micro-release-gateways.mjs
node tests/micro-host-browser.mjs
```

浏览器测试需要相应浏览器运行环境，参数见 [上下文契约](./micro-app-context-props.md)。这些测试覆盖平台和已有示例，不会自动证明新业务应用的真实接口、数据库范围和业务流程已通过。新应用需完成自己的单元检查及真实服务端到端验收。

## 15. 仓库参考资料

| 文件 | 阅读目的 |
| --- | --- |
| [微前端接入与上线指南](./microfrontend-integration.md) | 平台接入链路、历史迁移和网关说明 |
| [上下文与生命周期契约](./micro-app-context-props.md) | MicroProps、消息桥、续期、取消与清理 |
| [独立版本打包与部署](./application-independent-deployment.md) | 上传限制、版本目录、容器与网关部署 |
| [独立发布 API](./application-release-api.md) | 管理接口、来源限制、预检和运行模式 |
| [iframe SDK](../examples/micro-app-sdk/iframe-bridge.js) | connect/current/requestJson/downloadCsv 的真实实现 |
| [可信模式请求 SDK](../examples/micro-app-sdk/context-client.mjs) | createContextClient 的上下文与请求取消 |
| [MicroProps 类型](../frontend/src/components/micro/types.ts) | 完整字段与生命周期类型 |
| [ESM 注册器](../frontend/src/micro-apps/registry.ts) | 当前宿主模块发现规则 |
| [ESM 报表中心](../frontend/src/micro-apps/report-center.tsx) | 共享运行时与生命周期完整示例 |
| [独立工单前端](../business-apps/work-orders/frontend/README.md) | Vue 3 真实业务、冲突处理、权限与导出 |
| [独立工单后端](../business-apps/work-orders/backend/README.md) | 独立业务 API 与平台授权对接 |
| [后端授权参考组件](../examples/app-backend/authorize.mjs) | 内省、受众、操作权限与记录范围校验 |
| [命令行打包工具](../scripts/package-microapp.mjs) | 兼容自动化的不可变版本打包 |

当本文建议与实际版本行为不同，先核对 SDK、宿主、后端和部署网关是否为同一套已发布契约；不要通过放宽授权、关闭取消或修改隔离模式来掩盖版本差异。
