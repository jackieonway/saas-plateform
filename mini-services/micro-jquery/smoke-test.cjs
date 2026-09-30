// jQuery 子应用冒烟: vm + $ 桩, 验证生命周期与 mount/unmount
const fs = require("node:fs")
const vm = require("node:vm")

const code = fs.readFileSync("public/app.js", "utf8")

// 极简 jQuery 桩: $(sel) 返回可链式对象
function makeJqStub() {
  const el = {
    _listeners: {},
    on(ev, fn) { (this._listeners[ev] ||= []).push(fn); return this },
    off() { this._listeners = {}; return this },
    text(v) { return this },
    html(v) { return this },
    append() { return this },
    empty() { return this },
    each() { return this },
    attr() { return this },
    querySelector() { return makeJqStub() },
    querySelectorAll() { return [] },
    getAttribute() { return "" },
    setAttribute() { return this },
    getElementsByTagName() { return [] },
    getElementsByClassName() { return [] },
    addEventListener() { return this },
    removeEventListener() { return this },
    appendChild() { return this },
    removeChild() { return this },
    insertBefore() { return this },
    cloneNode() { return makeJqStub() },
    style: {},
    className: "",
    id: "",
    val() { return "" },
    find() { return makeJqStub() },
    addClass() { return this },
    removeClass() { return this },
    click() { return this },
    keyup() { return this },
    trigger() { return this },
  }
  return el
}
const jqStub = function (sel) { return makeJqStub() }
jqStub.ajax = function () {}
jqStub.fn = {}

const fakeWindow = { __DANTE_MICRO_APP__: undefined, jQuery: jqStub, $: jqStub,
  setInterval: () => 1, clearInterval: () => {}, setTimeout: () => 2, clearTimeout: () => {},
  addEventListener: () => {}, removeEventListener: () => {} }
Object.defineProperty(fakeWindow, "__DANTE_MICRO_APP__", {
  set(v) { this._lifec = v },
  get() { return this._lifec },
  configurable: true,
})
fakeWindow._lifec = undefined

const docStub = {
  getElementById() { return makeJqStub() },
  createElement() { return makeJqStub() },
  querySelector() { return makeJqStub() },
  querySelectorAll() { return [makeJqStub()] },
  addEventListener() {},
  removeEventListener() {},
  title: "",
  head: makeJqStub(),
  body: makeJqStub(),
}

const sandbox = new Proxy(
  { window: fakeWindow, document: docStub, jQuery: jqStub, $: jqStub, console },
  {
    get(t, k) {
      if (k === "window") return fakeWindow
      if (k === "document") return docStub
      if (k === "jQuery" || k === "$") return jqStub
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

const lifec = fakeWindow._lifec
console.log("lifecycle 暴露:", !!lifec, lifec && Object.keys(lifec))
if (!lifec || typeof lifec.mount !== "function" || typeof lifec.unmount !== "function") {
  console.log("FAIL: 契约不完整")
  process.exit(1)
}
const container = { innerHTML: "" }
try {
  const r = lifec.mount(container, {
    appCode: "legacy-jquery",
    basePath: "/micro/legacy-jquery",
    routePath: "/micro/legacy-jquery",
    token: "stub-token",
    user: { id: "1", username: "admin", displayName: "Admin" },
    theme: "light",
    menus: [],
  })
  Promise.resolve(r).then(() => {
    console.log("mount 成功(Promise resolve)")
    return lifec.unmount()
  }).then(() => {
    console.log("unmount 成功(Promise resolve)")
    console.log("ALL GREEN")
  }).catch((e) => { console.log("mount/unmount 失败:", e.message); process.exit(1) })
} catch (e) {
  console.log("mount 同步抛错:", e.message)
  process.exit(1)
}
