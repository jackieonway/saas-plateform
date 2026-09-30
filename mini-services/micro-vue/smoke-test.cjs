// 运行时判定: 沙箱执行 app.js + Vue 桩, 验证组件结构与 lifecycle.update 增量更新
const fs = require("node:fs")
const vm = require("node:vm")

const code = fs.readFileSync("public/app.js", "utf8")

let capturedComponent = null
const vueStub = {
  version: "3.5.13-stub",
  h: function (tag, props, children) {
    return { tag, props, children } // 最小 vnode 桩
  },
  reactive: function (o) {
    return o // 测试环境直通(真实 Vue 由响应式代理驱动重渲染)
  },
  createApp: function (component) {
    capturedComponent = component
    return {
      mount() {},
      unmount() {},
    }
  },
}

const fakeWindow = { __DANTE_MICRO_APP__: undefined }
Object.defineProperty(fakeWindow, "__DANTE_MICRO_APP__", {
  set(v) { this._lifec = v },
  get() { return this._lifec },
  configurable: true,
})
fakeWindow._lifec = undefined

const sandbox = new Proxy(
  { window: fakeWindow, Vue: vueStub, console },
  {
    get(t, k) {
      if (k === "window") return fakeWindow
      if (k === "Vue") return vueStub
      if (k === "console") return console
      if (k in t) return t[k]
      return globalThis[k]
    },
    set(t, k, v) { t[k] = v; return true },
    has() { return true },
  }
)

vm.createContext(sandbox)
vm.runInContext(code, sandbox)

const lifec = fakeWindow._lifec || fakeWindow.__DANTE_MICRO_APP__
console.log("lifecycle 暴露:", !!lifec, lifec && Object.keys(lifec))
if (!lifec || typeof lifec.mount !== "function" || typeof lifec.unmount !== "function" || typeof lifec.update !== "function") {
  console.log("FAIL: 生命周期契约不完整(需 mount/unmount/update)")
  process.exit(1)
}
const baseProps = {
  appCode: "vue-workshop",
  basePath: "/micro/vue-workshop",
  routePath: "/micro/vue-workshop",
  token: "stub-token",
  user: { id: "1", username: "admin", displayName: "Admin" },
  theme: "light",
  menus: [],
}
const fakeContainer = { innerHTML: "" }
lifec.mount(fakeContainer, baseProps)
console.log("capturedComponent:", !!capturedComponent)
if (capturedComponent) {
  const m = capturedComponent.methods || {}
  console.log("methods keys:", Object.keys(m).join(", "))
  console.log("renderOverview:", typeof m.renderOverview)
  console.log("renderSecurity:", typeof m.renderSecurity)
  console.log("render:", typeof capturedComponent.render)
  if (typeof capturedComponent.render !== "function") {
    console.log("FAIL: render 缺失")
    process.exit(1)
  }
  const data = capturedComponent.data()
  if (!data.p || typeof data.p !== "object") {
    console.log("FAIL: data().p(响应式 propsState)缺失")
    process.exit(1)
  }
  const makeCtx = () => ({
    ...m,
    ...data,
    view: "overview",
    routePath: data.p.routePath || "(空)",
    tokenPreview: data.p.token ? data.p.token.slice(0, 8) + "…" : "",
    seconds: 0,
    menus: { loading: false, fired: false, status: "", ok: false, message: "", data: null },
    roles: { loading: false, fired: false, status: "", ok: false, message: "", data: null },
    notifyLog: [],
    _timer: null,
  })
  try {
    const vnode = capturedComponent.render.call(makeCtx())
    console.log("render(overview) 调用成功, vnode.tag =", vnode && vnode.tag)
    const ctxSec = makeCtx()
    ctxSec.view = "security"
    const vnode2 = capturedComponent.render.call(ctxSec)
    console.log("render(security) 调用成功, vnode2.tag =", vnode2 && vnode2.tag)
  } catch (e) {
    console.log("render 调用失败:", e.message)
    process.exit(1)
  }

  // ---- update 契约验证: 增量写入 propsState, 组件数据引用同一对象 ----
  lifec.update({ ...baseProps, routePath: "/micro/vue-workshop/security", theme: "dark" })
  if (data.p.routePath !== "/micro/vue-workshop/security" || data.p.theme !== "dark") {
    console.log("FAIL: update 未写入 propsState(routePath/theme)", data.p.routePath, data.p.theme)
    process.exit(1)
  }
  if (data.p.updates !== 1) {
    console.log("FAIL: update 计数错误", data.p.updates)
    process.exit(1)
  }
  console.log("update 契约: propsState.routePath =", data.p.routePath, "· updates =", data.p.updates, "· lastUpdateAt =", data.p.lastUpdateAt || "(未记录)")

  lifec.unmount()
  console.log("unmount 调用成功")
  console.log("ALL GREEN")
}
