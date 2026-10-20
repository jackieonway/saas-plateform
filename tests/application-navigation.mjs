import assert from "node:assert/strict"
import test from "node:test"
import fs from "node:fs"
import path from "node:path"
import vm from "node:vm"
import { createRequire } from "node:module"
import { fileURLToPath } from "node:url"

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const require = createRequire(path.join(root, "frontend/package.json"))
const ts = require("typescript")
const source = fs.readFileSync(path.join(root, "frontend/src/lib/application-navigation.ts"), "utf8")
const code = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText
const exports = {}
new vm.Script(`(function(exports){${code}\n})`).runInNewContext()(exports)
const { createApplicationNavigation, createNavigationSequence, firstApplicationPath } = exports
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no }); return { promise, resolve, reject } }
const menu = (path, extra = {}) => ({ type: "MENU", visible: true, path, ...extra })

function fixture() {
  const apps = ["A", "B"].map(id => ({ id, basePath: `/micro/${id}` }))
  const state = { token: "token", epoch: 1, switching: false, identity: "user:tenant:org", currentApplicationId: null, applications: apps }
  const requests = [], navigations = [], failures = []
  const sequence = createNavigationSequence()
  const dependencies = {
    session: () => ({ ...state }), select: id => { state.currentApplicationId = id },
    menus: id => { const request = deferred(); requests.push({ id, ...request }); return request.promise },
    allowed: value => value.startsWith("/micro/"), navigate: value => navigations.push(value), failed: () => failures.push(true),
  }
  const select = createApplicationNavigation(dependencies, sequence)
  return { state, requests, navigations, failures, select, sequence,
    anotherConsumer: () => createApplicationNavigation(dependencies, sequence) }
}

test("application selection A then B ignores A late menu success", async () => {
  const f = fixture(), a = f.select("A", "/dashboard"), b = f.select("B", "/dashboard")
  f.requests[1].resolve([menu("/micro/B/list")]); await b
  f.requests[0].resolve([menu("/micro/A/list")]); await a
  assert.deepEqual(f.navigations, ["/micro/B/list"])
})
test("application selection A then B then A rejects the first A transaction", async () => {
  const f = fixture(), a = f.select("A", "/dashboard"), b = f.select("B", "/dashboard"), secondA = f.select("A", "/dashboard")
  f.requests[2].resolve([menu("/micro/A/new")]); await secondA
  f.requests[0].resolve([menu("/micro/A/old")]); f.requests[1].resolve([menu("/micro/B/list")]); await Promise.all([a, b])
  assert.deepEqual(f.navigations, ["/micro/A/new"])
})
test("direct navigation and actual layout departure cancel pending selection while app stays A", async () => {
  const f = fixture(), pending = f.select("A", "/dashboard")
  f.sequence.invalidate(); f.requests[0].resolve([menu("/micro/A/list")]); await pending
  assert.deepEqual(f.navigations, [])

  const layoutPath = path.join(root, "frontend/src/components/layout/AppLayout.tsx")
  const ast = ts.createSourceFile(layoutPath, fs.readFileSync(layoutPath, "utf8"), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  let layoutEffect
  const find = node => { if (ts.isCallExpression(node) && node.expression.getText(ast) === "React.useLayoutEffect") layoutEffect = node; ts.forEachChild(node, find) }
  find(ast); assert(layoutEffect, "The actual layout must register its selection lifetime")
  const effectCode = ts.transpileModule(`export const setup = ${layoutEffect.arguments[0].getText(ast)}`, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText
  const departure = fixture(), effectExports = {}
  new vm.Script(`(function(exports,invalidateApplicationSelection){${effectCode}\n})`).runInNewContext()(effectExports, () => departure.sequence.invalidate())
  const cleanup = effectExports.setup()
  const departingSelection = departure.select("A", "/dashboard")
  if (typeof cleanup === "function") cleanup()
  departure.requests[0].resolve([menu("/micro/A/list")]); await departingSelection
  assert.deepEqual(departure.navigations, [], "Leaving AppLayout must retire its pending selection")
})
test("rail and dashboard transactions share generation across separate navigator instances", async () => {
  const f = fixture(), rail = f.anotherConsumer()
  const dashboardA = f.select("A", "/dashboard"), railB = rail("B", "/dashboard"), railA = rail("A", "/dashboard")
  f.requests[2].resolve([menu("/micro/A/final")]); await railA
  f.requests[0].resolve([menu("/micro/A/stale")]); f.requests[1].resolve([menu("/micro/B/stale")]); await Promise.all([dashboardA, railB])
  assert.deepEqual(f.navigations, ["/micro/A/final"])
})
test("old micro route effects cannot overwrite a newer selection while its menus are pending", async () => {
  const f = fixture()
  f.state.currentApplicationId = "A"
  const selectedB = f.select("B", "/micro/A/detail")
  const containerPath = path.join(root, "frontend/src/components/micro/MicroAppContainer.tsx")
  const ast = ts.createSourceFile(containerPath, fs.readFileSync(containerPath, "utf8"), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  // Replay actual route synchronization callbacks as delayed passive effects
  // while a newer application selection is waiting for its menus.
  const routeEffects = []
  const find = node => {
    if (ts.isCallExpression(node) && node.expression.getText(ast) === "React.useEffect" &&
        node.arguments[0]?.getText(ast).includes("setCurrentApplication(")) routeEffects.push(node.arguments[0])
    ts.forEachChild(node, find)
  }
  find(ast)
  for (const callback of routeEffects) {
    const code = ts.transpileModule(`export const effect = ${callback.getText(ast)}`, {
      compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
    }).outputText
    const captured = {}
    new vm.Script(`(function(exports){${code}\n})`).runInNewContext({ activeRoute: true, app: { id: "A" }, switching: false,
      setCurrentApplication: id => { f.state.currentApplicationId = id } })(captured)
    captured.effect()
  }
  f.requests[0].resolve([menu("/micro/B/list")]); await selectedB
  assert.equal(f.state.currentApplicationId, "B", "The old route must not reclaim a newer application intent")
  assert.deepEqual(f.navigations, ["/micro/B/list"])

  const layoutPath = path.join(root, "frontend/src/components/layout/AppLayout.tsx")
  const layoutAst = ts.createSourceFile(layoutPath, fs.readFileSync(layoutPath, "utf8"), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  let inference
  const findInference = node => {
    if (ts.isCallExpression(node) && node.expression.getText(layoutAst) === "React.useEffect" &&
        node.arguments[0]?.getText(layoutAst).includes("inferredPathRef")) inference = node.arguments[0]
    ts.forEachChild(node, findInference)
  }
  findInference(layoutAst); assert(inference, "The layout owns deep-link application inference")
  const inferredSelections = [], layoutContext = { contextEpoch: 1, pathname: "/micro/A/detail", inferredPathRef: { current: null },
    profile: { applications: [{ id: "B", basePath: "/micro/B" }] }, currentApplicationId: "B",
    setCurrentApplication: id => { inferredSelections.push(id); layoutContext.currentApplicationId = id } }
  const inferred = {}, inferenceCode = ts.transpileModule(`export const infer = ${inference.getText(layoutAst)}`, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText
  new vm.Script(`(function(exports){${inferenceCode}\n})`).runInNewContext(layoutContext)(inferred)
  inferred.infer()
  layoutContext.profile = { applications: [{ id: "A", basePath: "/micro/A" }, { id: "B", basePath: "/micro/B" }] }
  inferred.infer()
  assert.deepEqual(inferredSelections, ["A"], "A same-epoch newly authorized deep link must acquire its route application")
  layoutContext.currentApplicationId = "B" // A newer manual selection must survive an ordinary profile poll.
  inferred.infer(); assert.deepEqual(inferredSelections, ["A"])
  layoutContext.profile = { applications: [] }; inferred.infer()
  layoutContext.profile = { applications: [{ id: "A", basePath: "/micro/A" }] }; inferred.infer()
  assert.deepEqual(inferredSelections, ["A", "A"], "Regranting the same owner after an empty profile must restore its deep link")
})
for (const field of ["epoch", "identity", "switching", "applications"]) test(`selection rejects changed auth ${field}`, async () => {
  const f = fixture(), pending = f.select("A", "/dashboard")
  f.state[field] = { epoch: 2, identity: "user:tenant:otherOrg", switching: true, applications: [] }[field]
  f.requests[0].resolve([menu("/micro/A/list")]); await pending
  assert.deepEqual(f.navigations, [])
})
test("legitimate token rotation within the same authorization epoch preserves selection", async () => {
  const f = fixture(), pending = f.select("A", "/dashboard")
  f.state.token = "rotated-token"
  f.requests[0].resolve([menu("/micro/A/list")]); await pending
  assert.deepEqual(f.navigations, ["/micro/A/list"])
})
test("logout before a menu response prevents navigation", async () => {
  const f = fixture(), pending = f.select("A", "/dashboard")
  f.state.token = null
  f.requests[0].resolve([menu("/micro/A/list")]); await pending
  assert.deepEqual(f.navigations, [])
})
test("a superseded selection failure cannot toast or redirect", async () => {
  const f = fixture(), a = f.select("A", "/dashboard"), b = f.select("B", "/dashboard")
  f.requests[0].reject(new Error("old failure")); await a
  assert.deepEqual(f.failures, []); assert.deepEqual(f.navigations, [])
  f.requests[1].reject(new Error("current failure")); await b
  assert.deepEqual(f.failures, [true])
})
test("landing selection ignores directories, buttons, links, invisible menus and patterns", () => {
  const nodes = [menu("/dir", { type: "DIR" }), menu("/button", { type: "BUTTON" }), menu("/link", { linkUrl: "https://example.test" }),
    menu("/hidden", { visible: false }), menu("/tickets/:id"), menu("/tickets/*"), menu("/denied"), menu("/allowed")]
  assert.equal(firstApplicationPath(nodes, value => value !== "/denied"), "/allowed")
  assert.equal(firstApplicationPath([menu("/tickets/:id")], () => true), null)
})

function hookFixture({ landing, permissionFailure = false }) {
  const app = { id: "selected", code: "orders", basePath: landing.startsWith("/micro/") ? "/micro/orders" : "/system" }
  const state = { token: "token", contextEpoch: 1, contextSwitching: false, currentApplicationId: null,
    profile: { user: { id: "u" }, tenant: { id: "t" }, organization: { id: "o" }, platformContext: false, applications: [app] },
    setCurrentApplication: (id) => { state.currentApplicationId = id } }
  const useAuthStore = selector => selector(state); useAuthStore.getState = () => state
  const calls = [], navigations = [], cache = new Map()
  const keys = { menus: (id, epoch) => ["auth-menus", id, epoch], permissions: epoch => ["permissions", epoch] }
  const queryClient = {
    fetchQuery: async options => {
      const key = JSON.stringify(options.queryKey); calls.push(options.kind)
      if (options.kind === "permissions" && permissionFailure) { cache.set(key, { status: "error" }); throw new Error("permissions 503") }
      const data = options.kind === "menus" ? [menu(landing)] : { permissions: ["system:user"] }
      cache.set(key, { status: "success", data }); return data
    },
    getQueryState: key => cache.get(JSON.stringify(key)), getQueryData: key => cache.get(JSON.stringify(key))?.data,
  }
  const stubs = {
    react: { useMemo: fn => fn(), useCallback: fn => fn },
    "react-router-dom": { ...require("react-router-dom"), useNavigate: () => path => navigations.push(path) },
    "@tanstack/react-query": { useQueryClient: () => queryClient }, sonner: { toast: { error() {} } },
    "@/stores/auth": { useAuthStore }, "@/hooks/use-authorized-paths": { useAuthorizedPaths: () => ({}) },
    "@/lib/auth-queries": {
      captureAuthSession: () => ({ epoch: state.contextEpoch, identity: "u:t:o" }), authQueryKeys: keys,
      authMenusQueryOptions: (id, session) => ({ kind: "menus", queryKey: keys.menus(id, session.epoch) }),
      authPermissionsQueryOptions: session => ({ kind: "permissions", queryKey: keys.permissions(session.epoch) }),
    },
  }
  const loaded = new Map()
  function load(name) {
    if (stubs[name]) return stubs[name]
    if (!name.startsWith("@/")) return require(name)
    if (loaded.has(name)) return loaded.get(name)
    const source = fs.readFileSync(path.join(root, "frontend/src", `${name.slice(2)}.ts`), "utf8")
    const code = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText
    const exports = {}; loaded.set(name, exports)
    new vm.Script(`(function(require,exports){${code}\n})`).runInNewContext({ URL, URLSearchParams, DOMException,
      window: { location: { origin: "https://host.example" } } })(load, exports)
    return exports
  }
  return { ...load("@/hooks/use-application-navigation").useApplicationNavigation(), calls, navigations }
}

test("real navigation hook enters explicit micro MENU without calling failing host permissions", async () => {
  const f = hookFixture({ landing: "/micro/orders/list", permissionFailure: true })
  await f.selectApplication("selected", "/dashboard")
  assert.deepEqual(f.calls, ["menus"]); assert.deepEqual(f.navigations, ["/micro/orders/list"])
  assert.equal(f.navigateTo("/\\elsewhere.example/profile"), false, "Internal navigation cannot reinterpret an external URL as a personal route")
  assert.deepEqual(f.navigations, ["/micro/orders/list"])
})
test("real navigation hook refuses host MENU when its required permissions fail", async () => {
  const f = hookFixture({ landing: "/system/users", permissionFailure: true })
  await f.selectApplication("selected", "/dashboard")
  assert.deepEqual(f.calls, ["menus", "permissions"]); assert.deepEqual(f.navigations, [])
})
test("real navigation hook uses host MENU and permission evidence from the same scoped cache", async () => {
  const f = hookFixture({ landing: "/system/users" })
  await f.selectApplication("selected", "/dashboard")
  assert.deepEqual(f.calls, ["menus", "permissions"]); assert.deepEqual(f.navigations, ["/system/users"])
})
