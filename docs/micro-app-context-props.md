# 微应用上下文与生命周期契约（Phase 8）

宿主只有取得当前应用的短期 token 和有效授权上下文后才挂载。应用必须关联已启用的机密 OAuth 客户端，并同时获得当前租户和组织授权。客户端密钥只在业务后端使用。旧版“没有 token 仍挂载”的行为已移除。

## MicroProps 2.0

后端上下文契约保持 `1.0`，宿主映射为 `MicroProps.contractVersion = "2.0"`。完整 TypeScript 类型见 `frontend/src/components/micro/types.ts`。

| 字段 | 含义 |
| --- | --- |
| `appCode`, `basePath`, `routePath` | 应用和宿主路由上下文。 |
| `token`, `tokenExpiresAt` | 本应用独立受众的短期令牌及到期时间。 |
| `contextVersion`, `contextEpoch`, `expiresAt` | 后端内容版本、宿主单调推送序号和展示上下文到期时间。 |
| `user`, `tenant`, `organization` | 服务端确认的当前身份、租户和组织。 |
| `roles`, `permissions`, `menus` | 当前用户角色、操作权限码和可见菜单。 |
| `dataGrants` | 独立绑定当前组织成员关系的资源/操作数据授权；不从角色推导。 |
| `theme` | 宿主实际主题 `light` 或 `dark`。 |
| `signal` | 上下文替换、过期、组织切换和卸载时触发的 AbortSignal。 |
| `onNavigate`, `onNotify` | 宿主路由和界面 Toast；过期上下文的回调不再生效。 |

这些值用于界面展示和发送业务请求。业务后端须按[后端参考组件](../examples/app-backend/authorize.mjs)逐请求内省、取得当前上下文，分别验证操作权限和数据授权；浏览器 props 不能成为资源归属的权威来源。

## 请求取消与更新

宿主至多约 20 秒刷新一次上下文，使用上下文到期计时器立即取消旧展示工作；令牌在到期前续签。续期 token、权限、同数量菜单内容、身份、独立数据授权、主题或子路由变化都会触发 `update(props)`。没有 `update` 的应用会在旧生命周期清理完成后重挂。

组织切换开始时，宿主先清空旧 profile 并停止旧微应用，再发送组织切换请求。认证查询按上下文序号隔离，所有请求绑定取消信号；即使请求实现忽略取消，迟到结果也会因序号不符被丢弃。应用失去授权、令牌或会话失效时，宿主退出 `/micro/...` 路由并显示提示。网络错误时挂载停止，用户可重试。

微应用每次发起请求都应检查到期时间，并使用当前 token 和信号。需要同时保护 fetch 与响应 JSON 解析阶段，防止组织切换后旧结果回流。可直接在有构建流程的应用中使用 [`createContextClient`](../examples/micro-app-sdk/context-client.mjs)：

```js
import { createContextClient } from "./context-client.mjs"

let client
window.__DANTE_MICRO_APP__ = {
  name: "orders",
  async mount(container, props) {
    client = createContextClient(props)
    // 渲染当前 props。需要业务数据时：
    // const orders = await client.requestJson("/orders-api/orders")
  },
  async update(props) {
    client.update(props)
    // 更新当前组织、菜单、权限、主题和路由。
  },
  async unmount() {
    client.dispose()
    // 等待框架实例销毁，并移除自建计时器、监听器与订阅。
  },
}
```

SDK 会合并调用者信号与宿主取消信号，并在读取 JSON 后再次验证当前上下文。取消抛出 `AbortError`，应用应忽略该请求的界面结果。Vue/jQuery 无构建示例在各自 `app.js` 实现相同逻辑，并保留在 `frontend/public/sub-apps/` 的发布镜像。

## 生命周期与资源归属

`mount/update/unmount` 可以返回 Promise；Promise 必须在实际操作结束后完成。宿主串行执行这些操作，并在旧异步任务结束、完成最终 `unmount` 后再执行新应用脚本。每次挂载使用独立 DOM 节点，取消时先从页面移除旧节点，迟到挂载不会重新显示旧组织。

HTML 入口的脚本按声明顺序加载和执行，带 `async` 的脚本也会在挂载前完成，以保证卸载后的迟到脚本不再求值。加载超时和组织/应用切换的信号覆盖入口、资源下载和每段脚本执行。失败或取消后的入口与资源缓存同步换代，正常重挂可复用成功缓存。

HTML 沙箱回收代理注册的计时器、RAF、window 监听器以及动态样式/脚本。宿主通过 `sandbox.run` 包裹生命周期的同步调用，记录 document 监听器并保留真实 document 身份，兼容 jQuery 委托。`await` 之后自行创建的 document 资源仍需应用在信号取消或 `unmount` 中清理。MutationObserver 无法完全区分同帧宿主与应用的动态样式，因此应用优先把样式放在挂载容器里，并为自己添加的全局节点显式记账。HTML 沙箱适用于可信应用，信任和独立发布边界在 Phase 9 确定。

## 验证

使用 Node 24：

```bash
node --test tests/micro-context-runtime.mjs tests/micro-sandbox-lifecycle.mjs \
  mini-services/micro-vue/smoke-test.cjs mini-services/micro-jquery/smoke-test.cjs
node tests/micro-host-browser.mjs
```

前一组验证上下文请求乱序、续期、到期、异步卸载、完整 props 签名、SDK 和示例清理；后一组启动临时 Vite，使用隔离的 API 响应在真实浏览器验证宿主流程。浏览器测试支持 `PLAYWRIGHT_MODULE`、`BROWSER_EXECUTABLE`、`MICRO_HOST_BIND`；本机默认绑定 IPv6 `::1`。它不修改数据库，也不读取测试凭据。真实后端授权语义由 Phase 5～7 的 PostgreSQL 回归承担。
