/**
 * Phase 10 notification acceptance: real Vite/React, Chromium and Socket.IO.
 * HTTP inbox/profile data and Socket.IO authorization payloads are isolated fixtures;
 * these checks do not connect to PostgreSQL or claim backend authorization coverage.
 * Node 24: node tests/notification-frontend-browser.mjs [recovery|events|switch|unauthorized|muted|dnd|mutations|palette|publish|publish_self]
 * NOTIFICATION_BROWSER_SCENARIO selects a local diagnostic scenario; the mandatory runner always runs all.
 */
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import http from "node:http"
import net from "node:net"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { launchBrowser, socketServer } from "./browser/runtime.mjs"

const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const Server = socketServer
const bindHost = process.env.NOTIFICATION_BROWSER_BIND || (process.platform === "win32" ? "::1" : "127.0.0.1")
const hostUrl = (port) => `http://${bindHost.includes(":") ? `[${bindHost}]` : bindHost}:${port}`
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const deferred = () => {
  let resolve
  const promise = new Promise((done) => { resolve = done })
  return { promise, resolve }
}
const freePort = () => new Promise((resolve, reject) => {
  const server = net.createServer()
  server.on("error", reject)
  server.listen(0, bindHost, () => {
    const port = server.address().port
    server.close((error) => error ? reject(error) : resolve(port))
  })
})
async function waitState(predicate, message, timeout = 10_000) {
  const deadline = Date.now() + timeout
  while (!predicate() && Date.now() < deadline) await pause(35)
  assert(predicate(), message)
}

const systemApplication = { id: "notification-test-system", code: "system", name: "系统管理", icon: "Bell", basePath: "/system" }
const user = { id: "notification-user", username: "notification-browser", realName: "通知浏览器用户",
  email: "", phone: "", avatar: "", gender: "UNKNOWN", isSuperAdmin: false }
const owner = (group) => ({ userId: group === "C" ? "other-notification-user" : user.id,
  tenantId: `notification-tenant-${group}`, organizationId: `notification-org-${group}` })
const profile = (group) => ({ user: { ...user, id: owner(group).userId },
  tenant: { id: owner(group).tenantId, name: `租户 ${group}`, code: group, isPlatform: false },
  organization: { id: owner(group).organizationId, name: `组织 ${group}`, path: `/notification-org-${group}/` },
  applications: [systemApplication], memberships: ["A", "B"].map((value) => ({
    organizationId: owner(value).organizationId, organizationName: `组织 ${value}`, organizationPath: `/notification-org-${value}/`,
    tenantId: owner(value).tenantId, tenantName: `租户 ${value}`,
  })),
})
const notification = (group, title = `离线通知 ${group}`, id = `historical-${group}`) => ({
  ...owner(group), id, sourceKind: "APP", applicationId: systemApplication.id, publicationId: `publication-${id}`,
  applicationName: systemApplication.name, type: "INFO", title, content: "数据库收件箱夹具", link: "",
  isRead: false, readAt: null, createdAt: new Date().toISOString(),
})
// Match the public Socket.IO projection; HTTP-only source/read/applicationName fields are absent.
const realtime = (group, id, title, extra = {}) => ({ contractVersion: "1.0", ...owner(group), id,
  applicationId: systemApplication.id, publicationId: `publication-${id}`, type: "INFO", title,
  content: "在线事件夹具", link: "/system/notifications", createdAt: new Date().toISOString(), ...extra })
const defaultPrefs = { mutedTypes: [], dndEnabled: false, dndStart: "22:00", dndEnd: "08:00" }
const authenticated = (group) => ({ contractVersion: "1.0", ...owner(group), expiresAt: new Date(Date.now() + 60_000).toISOString() })

let browser, vite, origin, io, socketHttp
let startupOutput = ""
let nextFixtureId = 0
const tokenOwners = new Map()
const serverErrors = []

async function startServers() {
  socketHttp = http.createServer((request, response) => { response.writeHead(404); response.end() })
  io = new Server(socketHttp, { path: "/socket.io/", transports: ["websocket", "polling"] })
  io.on("connection", (socket) => {
    socket.on("authenticate", ({ token } = {}) => {
      const entry = tokenOwners.get(token)
      if (!entry) { serverErrors.push("Unexpected fixture session"); socket.disconnect(true); return }
      const { state, group } = entry
      const record = { socket, group, authenticated: false, disconnected: false,
        origin: socket.handshake.headers.origin, transport: socket.conn.transport.name }
      state.connections.push(record)
      record.acknowledge = () => { record.authenticated = true; socket.emit("authenticated", authenticated(group)) }
      socket.on("disconnect", (reason) => { record.disconnected = true; record.reason = reason })
      if (state.authMode === "reject" && group === "A") socket.emit("unauthorized", { code: "SESSION_INVALID", message: "实时通知会话已失效" })
      else if (state.authMode !== "hold" || group !== "A") record.acknowledge()
    })
  })
  await new Promise((resolve, reject) => {
    socketHttp.once("error", reject)
    socketHttp.listen(0, bindHost, resolve)
  })
  const port = await freePort()
  origin = hostUrl(port)
  vite = spawn(process.execPath, [path.join(workspace, "node_modules/vite/bin/vite.js"),
    "--host", bindHost, "--port", String(port), "--strictPort"], {
    cwd: workspace, windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env, NOTIFICATION_DEV_PROXY_TARGET: hostUrl(socketHttp.address().port) },
  })
  vite.stdout.on("data", (chunk) => { startupOutput = (startupOutput + chunk).slice(-4000) })
  vite.stderr.on("data", (chunk) => { startupOutput = (startupOutput + chunk).slice(-4000) })
  let spawnError
  vite.on("error", (error) => { spawnError = error })
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    if (spawnError) throw spawnError
    if (vite.exitCode !== null) throw new Error(`Vite stopped before readiness: ${startupOutput}`)
    // Chromium navigation proves readiness; IPv4/localhost probes can be filtered on Windows.
    if (startupOutput.includes("ready in")) return
    await pause(200)
  }
  throw new Error(`Vite did not become ready: ${startupOutput}`)
}

async function fixture(options = {}) {
  const id = ++nextFixtureId
  const context = await browser.newContext({ viewport: { width: 1440, height: 960 } })
  const page = await context.newPage()
  page.setDefaultTimeout(12_000)
  const state = { connections: [], requests: [], held: [], holds: new Set(), count: { A: 2, B: 8, C: 3 },
    inbox: { A: [notification("A")], B: [notification("B")], C: [notification("C")] },
    prefs: { A: { ...defaultPrefs }, B: { ...defaultPrefs }, C: { ...defaultPrefs } },
    authMode: "auto", canPublish: false, directoryRead: true, startPath: "/dashboard", publishFailures: 0, publishRequests: [], ...options }
  const tokens = Object.fromEntries(["A", "B", "C"].map((group) => [group, `test-notification-session-${id}-${group}`]))
  for (const group of Object.keys(tokens)) tokenOwners.set(tokens[group], { state, group })
  const errors = []
  const consoleErrors = []
  const failedRequests = []
  page.on("pageerror", (error) => errors.push(error.message))
  page.on("console", (message) => { if (message.type() === "error") consoleErrors.push(message.text()) })
  page.on("requestfailed", (request) => failedRequests.push({ pathname: new URL(request.url()).pathname, error: request.failure()?.errorText }))
  await context.addInitScript(({ token }) => {
    if (!localStorage.getItem("platform-session-v1")) localStorage.setItem("platform-session-v1", JSON.stringify({ schema: 1, version: 1, sessionId: crypto.randomUUID(), status: "active", token: token, refreshToken: null }))
    localStorage.setItem("platform-theme", "light")
    window.__notificationUi = { swings: 0 }
    const observed = new WeakSet()
    new MutationObserver((changes) => {
      for (const change of changes) for (const node of change.addedNodes) {
        if (!(node instanceof Element)) continue
        const bells = [...node.querySelectorAll("svg.lucide-bell.bell-swing")]
        if (node.matches("svg.lucide-bell.bell-swing")) bells.push(node)
        for (const bell of bells) if (!observed.has(bell)) { observed.add(bell); window.__notificationUi.swings++ }
      }
    }).observe(document, { childList: true, subtree: true })
  }, { token: tokens.A })
  // Expose the real QueryClient for cache assertions without replacing its behavior.
  await context.route("**/src/main.tsx", async (route) => {
    const response = await route.fetch()
    const body = await response.text()
    assert.match(body, /const queryClient\s*=/, "The real host QueryClient must be present.")
    await route.fulfill({ response, body: body + `\nwindow.__notificationQueryClient = queryClient;
      window.__notificationSuccesses = [];
      queryClient.getQueryCache().subscribe((event) => {
        if (event.query.queryKey[0] === 'notifications' && event.query.state.status === 'success')
          window.__notificationSuccesses.push({ key: event.query.queryKey, data: event.query.state.data });
      });\n` })
  })
  const fulfill = (route, value, status = 200) => route.fulfill({ status,
    contentType: "application/json", body: JSON.stringify(value) }).catch((error) => {
    if (!/closed|disposed|Invalid Interception|Target|cancelled/i.test(error.message)) throw error
  })
  await context.route((url) => url.origin === origin && url.pathname.startsWith("/api/"), async (route) => {
    const request = route.request(), url = new URL(request.url())
    if (url.pathname === '/api/platform/branding') return fulfill(route, { platformName: '通知验收平台',
      platformDescription: '隔离浏览器测试', logoUrl: '', loginTitle: '登录', loginDescription: '', footerText: '' })
    const group = Object.keys(tokens).find((candidate) => request.headers().authorization === `Bearer ${tokens[candidate]}`)
    assert(group, "Every fixture API request must carry a captured platform session.")
    const record = { group, pathname: url.pathname, query: url.search, method: request.method(),
      body: request.postData() ? request.postDataJSON() : undefined }
    state.requests.push(record)
    let value
    if (url.pathname === "/api/auth/profile") value = profile(group)
    else if (url.pathname === "/api/auth/permissions") value = { permissions: ["system:notification", ...(state.canPublish ? ["notification:publish"] : []),
      ...(state.canPublish && state.directoryRead ? ["system:user", "system:organization"] : [])], roles: [] }
    else if (url.pathname === "/api/search") value = { users: [], tenants: [], organizations: [], roles: [] }
    else if (url.pathname === "/api/organizations/tree") {
      if (!state.directoryRead) return fulfill(route, { code: "FORBIDDEN", message: "无组织目录权限" }, 403)
      assert.equal(url.searchParams.get("tenantId"), owner(group).tenantId, "Target organization query must use the captured tenant.")
      value = [{ id: owner(group).organizationId, name: `接收组织 ${group}`, children: [] }]
    }
    else if (url.pathname === "/api/users") {
      if (!state.directoryRead) return fulfill(route, { code: "FORBIDDEN", message: "无用户目录权限" }, 403)
      assert.equal(url.searchParams.get("tenantId"), owner(group).tenantId)
      assert.equal(url.searchParams.get("organizationId"), owner(group).organizationId, "USER selection must explicitly bind its receiving organization.")
      value = { records: [{ id: `recipient-${group}`, username: `receiver-${group}`, realName: `接收用户 ${group}` }], total: 1, page: 1, size: 100 }
    }
    else if (url.pathname === "/api/auth/menus") value = [{ id: "notification-menu", parentId: null, type: "MENU", title: "通知中心",
      path: "/system/notifications", icon: "Bell", permission: "system:notification", sortOrder: 1, visible: true, children: [] }]
    else if (url.pathname === "/api/notifications/unread-count") value = { count: state.count[group] }
    else if (url.pathname === "/api/notifications/prefs") value = state.prefs[group]
    else if (url.pathname === "/api/notifications" && request.method() === "GET") value = {
      records: state.inbox[group], total: state.inbox[group].length, page: 1, size: Number(url.searchParams.get("size")) || 5 }
    else if (url.pathname === "/api/notifications" && request.method() === "POST") {
      state.publishRequests.push(record)
      if (state.publishFailures > 0) {
        state.publishFailures--
        return fulfill(route, { code: "SERVICE_UNAVAILABLE", message: "发布结果尚未确认" }, 503)
      }
      value = { sent: 1, publicationId: `fixture-publication-${state.publishRequests.length}`, recipientCount: 1,
        deliveryCount: 1, duplicate: state.publishRequests.length === 2, createdAt: new Date().toISOString() }
    }
    else if (url.pathname === "/api/notifications/read-all") value = { updated: 88 }
    else if (/^\/api\/notifications\/[^/]+\/read$/.test(url.pathname)) value = { ok: true }
    else if (url.pathname.startsWith("/api/monitor/")) return fulfill(route, { code: "FORBIDDEN", message: "无监控授权" }, 403)
    else value = { records: [], total: 0, page: 1, size: 10 }
    const holdKey = `${group}:${url.pathname}`
    if (state.holds.has(holdKey)) {
      const release = deferred()
      const captured = structuredClone(value)
      state.held.push({ ...record, release })
      await release.promise
      return fulfill(route, captured)
    }
    return fulfill(route, value)
  })
  try {
    const response = await page.goto(`${origin}${state.startPath}`, { waitUntil: "domcontentloaded" })
    assert.equal(response.status(), 200, "Vite must serve the real host document.")
    await page.waitForFunction(() => !!window.__notificationQueryClient)
    await waitState(() => state.connections.some((entry) => entry.group === "A"), "The real browser must send authentication over Socket.IO.")
  } catch (error) {
    console.error("Notification fixture startup diagnostic:", { errors, consoleErrors, failedRequests,
      body: (await page.locator("body").innerText()).slice(0, 3500), viteOutput: startupOutput })
    await context.close()
    for (const token of Object.values(tokens)) tokenOwners.delete(token)
    throw error
  }
  const test = { page, context, state, tokens, errors, consoleErrors }
  test.close = async () => {
    for (const held of state.held) held.release.resolve()
    await context.close()
    for (const token of Object.values(tokens)) tokenOwners.delete(token)
  }
  return test
}

const requestCount = (state, group, pathname) => state.requests.filter((r) => r.group === group && r.pathname === pathname).length
const notificationRequestCount = (state, group) => state.requests.filter((r) => r.group === group && r.pathname.startsWith("/api/notifications")).length
const connection = (state, group = "A") => state.connections.findLast((entry) => entry.group === group && !entry.disconnected)
const unread = (page, count) => page.locator(`header button[aria-label="${count ? `${count} 条未读通知` : "通知"}"]`)
const uiState = (page) => page.evaluate(() => ({ ...window.__notificationUi, toastCount: document.querySelectorAll("[data-sonner-toast]").length }))
const cache = (page) => page.evaluate(() => window.__notificationQueryClient.getQueryCache().getAll()
  .filter((query) => query.queryKey[0] === "notifications").map((query) => ({ key: query.queryKey, data: query.state.data, status: query.state.status })))
async function ready(test, group = "A") {
  await waitState(() => connection(test.state, group)?.authenticated, "Socket.IO must confirm the captured owner.")
  await unread(test.page, test.state.count[group]).waitFor()
  await test.page.locator('header [aria-label="实时推送已连接"]').waitFor()
}
async function openBell(test, group = "A") {
  await unread(test.page, test.state.count[group]).click()
  await test.page.getByText(test.state.inbox[group][0].title, { exact: true }).waitFor()
}
async function settled(test) {
  await test.page.waitForFunction(() => window.__notificationQueryClient.getQueryCache().getAll()
    .filter((query) => query.queryKey[0] === "notifications").every((query) => query.state.fetchStatus !== "fetching"))
}
async function setToken(test, group) {
  await test.page.evaluate(async (token) => {
    const { useAuthStore } = await import("/src/stores/auth.ts")
    await useAuthStore.getState().setToken(token)
  }, test.tokens[group])
}
async function beginSwitch(test) {
  return test.page.evaluate(async () => {
    const { useAuthStore } = await import("/src/stores/auth.ts")
    const { getNotificationContext } = await import("/src/hooks/use-notification-context.ts")
    const scope = getNotificationContext(), token = useAuthStore.getState().token
    useAuthStore.getState().beginContextSwitch()
    return { signalAborted: scope.signal.aborted, contextGone: getNotificationContext() === null,
      switching: useAuthStore.getState().contextSwitching, tokenUnchanged: token === useAuthStore.getState().token }
  })
}
async function withFixture(options, run) {
  const test = await fixture(options)
  try {
    await run(test)
    assert.deepEqual(test.errors, [], "The real React page must have no uncaught errors.")
    assert(!test.consoleErrors.some((message) => /Encountered two children with the same key/.test(message)),
      "Context-bound notification dialogs must retain distinct React identities.")
  } catch (error) {
    console.error("Notification browser diagnostic:", { pathname: new URL(test.page.url()).pathname,
      errors: test.errors, consoleErrors: test.consoleErrors, body: (await test.page.locator("body").innerText()).slice(0, 3500),
      requests: test.state.requests, connections: test.state.connections.map(({ group, authenticated, disconnected, reason, transport }) =>
        ({ group, authenticated, disconnected, reason, transport })), cache: await cache(test.page),
      comboboxes: await test.page.locator('[role="combobox"]').evaluateAll((elements) => elements.map((element) => ({
        text: element.textContent, label: element.getAttribute("aria-label"), labelledBy: element.getAttribute("aria-labelledby"), html: element.outerHTML.slice(0, 450) }))), viteOutput: startupOutput })
    throw error
  } finally { await test.close() }
}

const scenarios = {
  recovery: async () => withFixture({ authMode: "hold" }, async (test) => {
    const { page, state } = test
    await unread(page, 2).waitFor()
    await openBell(test)
    const endpoints = ["/api/notifications", "/api/notifications/unread-count", "/api/notifications/prefs"]
    const baseline = endpoints.map((endpoint) => requestCount(state, "A", endpoint))
    connection(state).acknowledge()
    await ready(test)
    await waitState(() => endpoints.every((endpoint, i) => requestCount(state, "A", endpoint) > baseline[i]), "Authentication must immediately refresh list, count and preferences.")
    assert.deepEqual(await uiState(page), { swings: 0, toastCount: 0 }, "Historical inbox recovery must not animate or toast.")
    state.authMode = "auto"
    state.count.A = 4
    state.inbox.A = [notification("A", "断线期间持久通知")]
    const beforeReconnect = endpoints.map((endpoint) => requestCount(state, "A", endpoint))
    const oldConnection = connection(state)
    oldConnection.socket.conn.close()
    await waitState(() => connection(state)?.authenticated && connection(state) !== oldConnection, "Transport loss must reconnect and reauthenticate.")
    await waitState(() => endpoints.every((endpoint, i) => requestCount(state, "A", endpoint) > beforeReconnect[i]), "Reauthentication must immediately refresh every database view.")
    await unread(page, 4).waitFor()
    await page.getByText("断线期间持久通知", { exact: true }).waitFor()
    assert.deepEqual(await uiState(page), { swings: 0, toastCount: 0 }, "Reconnect recovery must not replay historical Toasts.")
    assert.equal(connection(state).origin, origin)
  }),
  events: async () => withFixture({}, async (test) => {
    const { page, state } = test
    await ready(test)
    await openBell(test)
    await settled(test)
    const before = notificationRequestCount(state, "A")
    for (const extra of [{ contractVersion: "9.0" }, { userId: "wrong-user" }, { tenantId: "wrong-tenant" }, { organizationId: "wrong-org" }]) {
      connection(state).socket.emit("notification", realtime("A", `wrong-${Object.keys(extra)[0]}`, "非法归属事件", extra))
    }
    await pause(500)
    assert.equal(notificationRequestCount(state, "A"), before, "Wrong contract or owner must not trigger an inbox refresh.")
    assert.deepEqual(await uiState(page), { swings: 0, toastCount: 0 })
    state.count.A = 11
    state.inbox.A = [notification("A", "实时事件对应数据库通知")]
    connection(state).socket.emit("notification", realtime("A", "live-dedup", "正确实时通知"))
    await unread(page, 11).waitFor()
    await page.locator("[data-sonner-toast]").filter({ hasText: "正确实时通知" }).waitFor()
    await page.getByText("实时事件对应数据库通知", { exact: true }).waitFor()
    await settled(test)
    assert.deepEqual(await uiState(page), { swings: 1, toastCount: 1 }, "A valid event refreshes database count and produces one animation and Toast.")
    const afterFirst = notificationRequestCount(state, "A")
    connection(state).socket.emit("notification", realtime("A", "live-dedup", "同ID不同标题仍不能重复提醒"))
    await pause(500)
    assert.equal(notificationRequestCount(state, "A"), afterFirst, "Duplicate event ID must not repeat refreshes.")
    assert.deepEqual(await uiState(page), { swings: 1, toastCount: 1 })
  }),
  switch: async () => withFixture({}, async (test) => {
    const { page, state } = test
    await ready(test)
    await openBell(test)
    const oldConnection = connection(state)
    state.count.A = 444
    state.inbox.A = [notification("A", "迟到旧组织HTTP")]
    for (const endpoint of ["/api/notifications", "/api/notifications/unread-count"]) state.holds.add(`A:${endpoint}`)
    await page.evaluate(() => { void window.__notificationQueryClient.invalidateQueries({ queryKey: ["notifications"] }) })
    await waitState(() => state.held.length >= 2, "Old list and count responses must be held during switching.")
    assert.deepEqual(await beginSwitch(test), { signalAborted: true, contextGone: true, switching: true, tokenUnchanged: true })
    await waitState(() => oldConnection.disconnected, "The old socket must disconnect while the original token is still retained.")
    oldConnection.socket.emit("authenticated", authenticated("A"))
    oldConnection.socket.emit("notification", realtime("A", "late-A", "迟到旧组织Socket"))
    assert.equal(await page.getByLabel("实时推送已连接", { exact: true }).count(), 0)
    await setToken(test, "B")
    await ready(test, "B")
    await openBell(test, "B")
    for (const held of state.held) held.release.resolve()
    await pause(550)
    await unread(page, 8).waitFor()
    assert.equal(await page.getByText("迟到旧组织HTTP", { exact: true }).count(), 0)
    assert.equal(await page.getByText("迟到旧组织Socket", { exact: true }).count(), 0)
    assert.deepEqual(await uiState(page), { swings: 0, toastCount: 0 })
    const snapshots = await page.evaluate(() => window.__notificationSuccesses)
    assert(!snapshots.some(({ data }) => data?.count === 444 || data?.records?.some((item) => item.title === "迟到旧组织HTTP")), "Aborted old HTTP responses must never become successful notification cache data.")
    assert((await cache(page)).filter(({ key, status }) => key[3] === owner("B").organizationId && status === "success")
      .every(({ key }) => key[1] === owner("B").userId && key[2] === owner("B").tenantId && typeof key[4] === "number"))
  }),
  unauthorized: async () => withFixture({ authMode: "reject" }, async (test) => {
    const { page, state } = test
    await waitState(() => state.connections.some((entry) => entry.disconnected), "Unauthorized must close the current socket.")
    const attempts = state.connections.length
    await pause(2600)
    assert.equal(state.connections.length, attempts, "Rejected authentication must stop reconnect attempts until token changes.")
    assert.equal(await page.getByLabel("实时推送已连接", { exact: true }).count(), 0)
    await unread(page, 2).waitFor()
    await setToken(test, "B")
    await ready(test, "B")
    assert.equal(connection(state, "B").group, "B", "A fresh token must establish a fresh connection.")
  }),
  muted: async () => withFixture({ prefs: { A: { ...defaultPrefs, mutedTypes: ["WARNING"] }, B: { ...defaultPrefs }, C: { ...defaultPrefs } } }, async (test) => {
    await verifySilentPreference(test, "WARNING", "静音类型消息")
  }),
  dnd: async () => withFixture({ prefs: { A: { ...defaultPrefs, dndEnabled: true, dndStart: "00:00", dndEnd: "00:00" }, B: { ...defaultPrefs }, C: { ...defaultPrefs } } }, async (test) => {
    await verifySilentPreference(test, "INFO", "免打扰消息")
    await test.page.getByLabel("免打扰中 00:00-00:00", { exact: true }).waitFor()
  }),
  mutations: async () => withFixture({}, async (test) => {
    const { page, state } = test
    await ready(test)
    await openBell(test)
    state.holds.add("A:/api/notifications/read-all")
    state.holds.add("A:/api/notifications/historical-A/read")
    await page.getByRole("button", { name: "全部已读", exact: true }).click()
    await page.getByRole("button").filter({ hasText: "离线通知 A" }).click()
    await waitState(() => state.held.length === 2, "Read-all and read-one must both be held with their original session.")
    await beginSwitch(test)
    await setToken(test, "B")
    await ready(test, "B")
    await openBell(test, "B")
    await settled(test)
    const before = notificationRequestCount(state, "B")
    for (const held of state.held) held.release.resolve()
    await pause(550)
    assert.equal(notificationRequestCount(state, "B"), before, "Old mutation completion must not invalidate the new context.")
    assert.equal(await page.locator("[data-sonner-toast]").filter({ hasText: "已读 88 条" }).count(), 0)
    await unread(page, 8).waitFor()
    assert.equal(new URL(page.url()).pathname, "/dashboard")
  }),
  palette: async () => withFixture({}, async (test) => {
    const { page, state } = test
    await ready(test)
    state.holds.add("A:/api/notifications")
    await page.keyboard.press("Control+k")
    await page.getByRole("combobox").fill("离线")
    await waitState(() => state.held.some((entry) => entry.query.includes("q=")), "The old palette search response must be pending.")
    await beginSwitch(test)
    await setToken(test, "B")
    await ready(test, "B")
    await page.keyboard.press("Control+k")
    await page.getByRole("combobox").fill("离线")
    await page.getByRole("option").filter({ hasText: "离线通知 B" }).waitFor()
    for (const held of state.held) held.release.resolve()
    await pause(450)
    assert.equal(await page.getByRole("option").filter({ hasText: "离线通知 A" }).count(), 0)
    assert((await cache(page)).some(({ key, status }) => key[3] === owner("B").organizationId && key[5] === "palette" && status === "success"))
    await setToken(test, "C")
    await ready(test, "C")
    await page.keyboard.press("Control+k")
    await page.getByRole("combobox").fill("离线")
    await page.getByRole("option").filter({ hasText: "离线通知 C" }).waitFor()
    assert.equal(await page.getByRole("option").filter({ hasText: "离线通知 B" }).count(), 0)
    assert((await cache(page)).some(({ key, status }) => key[1] === owner("C").userId && key[2] === owner("C").tenantId && key[3] === owner("C").organizationId && key[5] === "palette" && status === "success"))
  }),
  publish: async () => withFixture({ canPublish: true, startPath: "/system/notifications", publishFailures: 1 }, async (test) => {
    const { page, state } = test
    await ready(test)
    let dialog = await openPublish(test)
    assert.equal(await dialog.getByRole("button", { name: "发送", exact: true }).isDisabled(), true, "An explicit application and valid target are required.")
    await selectPublishApplication(page, dialog)
    await selectPublishOrganization(page, dialog, "A")
    await dialog.getByRole("combobox", { name: "选择用户", exact: true }).click()
    await page.getByRole("option", { name: "接收用户 A(receiver-A)", exact: true }).click()
    const firstKey = await dialog.getByLabel("幂等键", { exact: true }).inputValue()
    await dialog.getByPlaceholder("一句话说明通知主题").fill("  用户发布通知  ")
    await dialog.getByPlaceholder("通知正文,支持换行").fill("  第一行\n第二行  ")
    await dialog.getByPlaceholder("/micro/system/…", { exact: true }).fill(" /micro/system/record/1 ")
    await dialog.getByRole("button", { name: "发送", exact: true }).click()
    await dialog.getByRole("button", { name: "重试发送", exact: true }).waitFor()
    const firstBody = { applicationId: systemApplication.id, idempotencyKey: firstKey,
      target: { kind: "USER", userId: "recipient-A", organizationId: owner("A").organizationId },
      title: "用户发布通知", content: "第一行\n第二行", type: "INFO", link: "/micro/system/record/1" }
    assert.deepEqual(state.publishRequests[0].body, firstBody)
    assert.equal(state.publishRequests[0].group, "A")
    assert.equal(await dialog.getByLabel("幂等键", { exact: true }).isDisabled(), true)
    assert.equal(await dialog.getByRole("combobox", { name: "发布应用", exact: true }).isDisabled(), true)
    assert.equal(await dialog.getByPlaceholder("一句话说明通知主题").isDisabled(), true)
    assert.equal(await dialog.getByPlaceholder("通知正文,支持换行").isDisabled(), true)
    for (const radio of await dialog.getByRole("radio").all()) assert.equal(await radio.isDisabled(), true)
    await dialog.getByRole("button", { name: "重试发送", exact: true }).click()
    await page.locator("[data-sonner-toast]").filter({ hasText: "通知发布已确认，共 1 位用户、1 条投递" }).waitFor()
    await dialog.waitFor({ state: "hidden" })
    assert.deepEqual(state.publishRequests[1].body, firstBody, "An unknown 503 result must retry the identical body and idempotency key.")

    dialog = await openPublish(test)
    await selectPublishApplication(page, dialog)
    await dialog.getByText("组织成员", { exact: true }).click()
    await selectPublishOrganization(page, dialog, "A")
    const organizationKey = await dialog.getByLabel("幂等键", { exact: true }).inputValue()
    assert.notEqual(organizationKey, firstKey, "A confirmed publication resets the next idempotency key.")
    await dialog.getByPlaceholder("一句话说明通知主题").fill("组织发布通知")
    await dialog.getByPlaceholder("通知正文,支持换行").fill("只投递直接成员")
    await dialog.getByRole("button", { name: "发送", exact: true }).click()
    await dialog.waitFor({ state: "hidden" })
    assert.deepEqual(state.publishRequests[2].body, { applicationId: systemApplication.id, idempotencyKey: organizationKey,
      target: { kind: "ORGANIZATION", organizationId: owner("A").organizationId }, title: "组织发布通知", content: "只投递直接成员", type: "INFO" })

    dialog = await openPublish(test)
    await selectPublishApplication(page, dialog)
    await dialog.getByText("租户全员", { exact: true }).click()
    await dialog.getByText("租户 A", { exact: true }).waitFor()
    const tenantKey = await dialog.getByLabel("幂等键", { exact: true }).inputValue()
    await dialog.getByPlaceholder("一句话说明通知主题").fill("租户发布通知")
    await dialog.getByRole("button", { name: "发送", exact: true }).click()
    await dialog.waitFor({ state: "hidden" })
    assert.deepEqual(state.publishRequests[3].body, { applicationId: systemApplication.id, idempotencyKey: tenantKey,
      target: { kind: "TENANT" }, title: "租户发布通知", content: "", type: "INFO" })

    dialog = await openPublish(test)
    await selectPublishApplication(page, dialog)
    await dialog.getByText("租户全员", { exact: true }).click()
    await dialog.getByPlaceholder("一句话说明通知主题").fill("旧组织未发送草稿")
    const oldDraftKey = await dialog.getByLabel("幂等键", { exact: true }).inputValue()
    await beginSwitch(test)
    await dialog.waitFor({ state: "hidden" })
    await setToken(test, "B")
    await ready(test, "B")
    dialog = await openPublish(test)
    assert.equal(await dialog.getByPlaceholder("一句话说明通知主题").inputValue(), "", "Changing context must discard the old publish draft.")
    assert.notEqual(await dialog.getByLabel("幂等键", { exact: true }).inputValue(), oldDraftKey)
    await dialog.getByRole("combobox", { name: "发布应用", exact: true }).getByText("选择当前可用应用", { exact: true }).waitFor()
    await selectPublishApplication(page, dialog)
    await dialog.getByText("租户全员", { exact: true }).click()
    await dialog.getByText("租户 B", { exact: true }).waitFor()
    await dialog.getByPlaceholder("一句话说明通知主题").fill("新组织发布通知")
    const newKey = await dialog.getByLabel("幂等键", { exact: true }).inputValue()
    await dialog.getByRole("button", { name: "发送", exact: true }).click()
    await dialog.waitFor({ state: "hidden" })
    assert.equal(state.publishRequests[4].group, "B", "The new dialog must use its captured new platform session.")
    assert.deepEqual(state.publishRequests[4].body, { applicationId: systemApplication.id, idempotencyKey: newKey,
      target: { kind: "TENANT" }, title: "新组织发布通知", content: "", type: "INFO" })
    assert.equal(state.publishRequests.length, 5)
  }),
  publish_self: async () => withFixture({ canPublish: true, directoryRead: false, startPath: "/system/notifications" }, async (test) => {
    const { page, state } = test
    await ready(test)
    const dialog = await openPublish(test)
    await selectPublishApplication(page, dialog)
    await selectPublishOrganization(page, dialog, "A", "组织 A")
    await dialog.getByRole("combobox", { name: "选择用户", exact: true }).click()
    await page.getByRole("option", { name: "通知浏览器用户(notification-browser)", exact: true }).click()
    const idempotencyKey = await dialog.getByLabel("幂等键", { exact: true }).inputValue()
    await dialog.getByPlaceholder("一句话说明通知主题").fill("显式选择本人和当前组织")
    await dialog.getByRole("button", { name: "发送", exact: true }).click()
    await dialog.waitFor({ state: "hidden" })
    assert.equal(state.publishRequests.length, 1)
    assert.equal(state.publishRequests[0].group, "A")
    assert.deepEqual(state.publishRequests[0].body, { applicationId: systemApplication.id, idempotencyKey,
      target: { kind: "USER", userId: user.id, organizationId: owner("A").organizationId },
      title: "显式选择本人和当前组织", content: "", type: "INFO" })
    assert(!state.requests.some(({ pathname }) => pathname === "/api/users" || pathname === "/api/organizations/tree"),
      "A publisher without directory permissions must use verified profile candidates without enumerating management directories.")
    // This checks UI selection/serialization only; fixtures do not grant backend data scope.
  }),
}

async function openPublish(test) {
  await test.page.getByRole("button", { name: "发送通知", exact: true }).click()
  const dialog = test.page.getByRole("dialog", { name: "发送通知", exact: true })
  await dialog.waitFor()
  return dialog
}
async function selectPublishApplication(page, dialog) {
  await dialog.getByRole("combobox", { name: "发布应用", exact: true }).click()
  await page.getByRole("option", { name: "系统管理（system）", exact: true }).click()
}
async function selectPublishOrganization(page, dialog, group, label = `接收组织 ${group}`) {
  await dialog.locator('button[role="combobox"]').filter({ hasText: "选择目标组织" }).click()
  await page.getByRole("button", { name: label, exact: true }).click()
}

async function verifySilentPreference(test, type, title) {
  const { page, state } = test
  await waitState(() => connection(state)?.authenticated, "Socket must authenticate before the preference event.")
  await unread(page, 2).waitFor()
  await waitState(() => requestCount(state, "A", "/api/notifications/prefs") > 0, "Display preferences must load.")
  await openBell(test)
  await settled(test)
  state.count.A = 7
  state.inbox.A = [notification("A", title)]
  connection(state).socket.emit("notification", realtime("A", `silent-${type}`, title, { type }))
  await unread(page, 7).waitFor()
  await page.getByText(title, { exact: true }).waitFor()
  assert.deepEqual(await uiState(page), { swings: 1, toastCount: 0 }, "Display preferences suppress Toasts while the database list/count and bell update.")
}

let passed = 0
try {
  const selector = process.argv[2] || process.env.NOTIFICATION_BROWSER_SCENARIO || "all"
  assert(selector === "all" || Object.hasOwn(scenarios, selector), `Unknown scenario: ${selector}`)
  await startServers()
  browser = await launchBrowser()
  for (const [name, run] of Object.entries(scenarios)) {
    if (selector !== "all" && selector !== name) continue
    await run()
    console.log(`PASS notification browser: ${name}`)
    passed++
  }
  assert.deepEqual(serverErrors, [])
  console.log(`Notification frontend browser acceptance: ${passed}/${selector === "all" ? Object.keys(scenarios).length : 1} passed (isolated HTTP/Socket.IO fixtures; no PostgreSQL).`)
} finally {
  await browser?.close()
  if (vite && vite.exitCode === null) {
    vite.kill()
    await Promise.race([new Promise((resolve) => vite.once("exit", resolve)), pause(5000)])
    if (vite.exitCode === null) vite.kill("SIGKILL")
  }
  if (io) await new Promise((resolve) => io.close(resolve))
}
