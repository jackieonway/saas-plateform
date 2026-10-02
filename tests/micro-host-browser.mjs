/**
 * Phase 8 integration: the real Vite app, MicroAppContainer, HTML loader and sandbox.
 * APIs and the probe micro applications are intercepted in a new browser context.
 * No database, account credentials or running application services are required.
 * Run with Node 24; PLAYWRIGHT_MODULE and BROWSER_EXECUTABLE may override existing runtimes.
 */
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { existsSync } from "node:fs"
import { createRequire } from "node:module"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

const require = createRequire(import.meta.url)
const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const bundledNode = path.join(os.homedir(), ".cache/codex-runtimes/codex-primary-runtime/dependencies/node")
const playwrightPath = process.env.PLAYWRIGHT_MODULE || path.join(bundledNode, "node_modules/playwright")
const { chromium } = require(playwrightPath)
assert(Number(process.versions.node.split(".")[0]) >= 20, "Run this script with Node 24 (Vite requires Node 20+).")
const browserExecutable = process.env.BROWSER_EXECUTABLE || [chromium.executablePath(),
  path.join(process.env["ProgramFiles(x86)"] || "C:/Program Files (x86)", "Microsoft/Edge/Application/msedge.exe"),
  path.join(process.env.ProgramFiles || "C:/Program Files", "Microsoft/Edge/Application/msedge.exe"),
].find((candidate) => existsSync(candidate))
assert(browserExecutable && existsSync(browserExecutable), "Existing Chromium or Edge is required; this test does not install browsers.")
// Explicit IPv6 bypasses this Windows host's filtered IPv4/localhost loopback path.
const bindHost = process.env.MICRO_HOST_BIND || "::1"

const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const redact = (value) => String(value).replace(/test-(?:session-[AB]|application-\d+|app…)/g, "[mock-token]")
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

const applications = [...["probe", "probe-static"].map((code) => ({
  id: `application-${code}`, code, name: code === "probe" ? "上下文验收应用" : "重挂验收应用",
  icon: "Boxes", basePath: `/micro/${code}`, entryType: "HTML", entryUrl: `/__phase8/${code}/`,
})), ...[
  { code: "vue-workshop", name: "真实 Vue 验收应用", folder: "vue" },
  { code: "legacy-jquery", name: "真实 jQuery 验收应用", folder: "jquery" },
].map(({ code, name, folder }) => ({ id: `application-${code}`, code, name, icon: "Boxes",
  basePath: `/micro/${code}`, entryType: "HTML", entryUrl: `/sub-apps/${folder}/index.html` }))]
const user = {
  id: "user-browser-test", username: "browser-test", realName: "浏览器验收用户",
  email: "", phone: "", avatar: "", gender: "UNKNOWN", isSuperAdmin: false,
}
function profile(group, apps = applications) {
  return {
    user, tenant: { id: `tenant-${group}`, name: `租户 ${group}`, code: group },
    organization: { id: `org-${group}`, name: `组织 ${group}`, path: `/org-${group}/` },
    applications: apps,
    memberships: ["A", "B"].map((value) => ({
      organizationId: `org-${value}`, organizationName: `组织 ${value}`, organizationPath: `/org-${value}/`,
      tenantId: `tenant-${value}`, tenantName: `租户 ${value}`,
    })),
  }
}
function menu(appCode, label) {
  return [{ id: `menu-${appCode}`, parentId: null, type: "MENU", title: label,
    path: `/micro/${appCode}/detail`, icon: "Boxes", permission: "probe:read",
    sortOrder: 1, visible: true, children: [] }]
}

// Events contain authorization metadata and a token-presence flag; token values are never logged.
function probeHtml(withUpdate) {
  return `<!doctype html><html><head><title>Phase 8 probe</title></head><body>
  <script>(function () {
    const events = window.__phase8Events;
    const control = window.__phase8Control;
    let node, current, instanceId;
    function record(kind, props) {
      const required = ['contractVersion','contextVersion','contextEpoch','expiresAt','tokenExpiresAt',
        'signal','token','appCode','basePath','routePath','user','tenant','organization','roles',
        'permissions','dataGrants','menus','theme','onNavigate','onNotify'];
      const entry = { kind, instanceId, at: Date.now(), contractVersion: props.contractVersion,
        contextVersion: props.contextVersion, contextEpoch: props.contextEpoch,
        organizationId: props.organization.id, tenantId: props.tenant.id, userId: props.user.id,
        tokenPresent: typeof props.token === 'string' && props.token.length > 0,
        tokenChanged: Boolean(current && current.token !== props.token),
        hasSignal: props.signal instanceof AbortSignal, signalAborted: props.signal.aborted,
        missing: required.filter((key) => !(key in props)),
        expiresAt: props.expiresAt, tokenExpiresAt: props.tokenExpiresAt,
        roleCodes: props.roles.map((role) => role.code), permissions: props.permissions,
        dataGrants: props.dataGrants, menuTitles: props.menus.map((item) => item.title),
        theme: props.theme, routePath: props.routePath, appCode: props.appCode };
      events.push(entry);
      current = props;
      control.navigate = props.onNavigate;
      control.notify = props.onNotify;
      control.signal = props.signal;
      if (node) node.innerHTML = '<div data-testid="phase8-probe" data-org="' + props.organization.id + '">' + props.organization.name + '</div>';
    }
    const lifecycle = { name: 'phase8-probe', mount: function (container, props) {
      node = container;
      instanceId = ++control.nextId;
      record('mount', props);
    }, unmount: function () {
      events.push({ kind: 'unmount-start', instanceId, organizationId: current.organization.id, appCode: current.appCode, at: Date.now() });
      return new Promise(function (resolve) { setTimeout(function () {
        if (node) node.replaceChildren();
        events.push({ kind: 'unmount-end', instanceId, organizationId: current.organization.id, appCode: current.appCode, at: Date.now() });
        resolve();
      }, 60); });
    } };
    ${withUpdate ? "lifecycle.update = function (props) { record('update', props); };" : ""}
    window.__DANTE_MICRO_APP__ = lifecycle;
  })();</script></body></html>`
}

function externalProbeHtml(withUpdate, asyncScript) {
  return probeHtml(withUpdate).replace(/<script>[\s\S]*?<\/script>/,
    `<script${asyncScript ? " async" : ""} src="./app.js"></script>`)
}

function probeJavaScript(withUpdate, label) {
  const script = probeHtml(withUpdate).match(/<script>([\s\S]*?)<\/script>/)[1]
  return script.replace("const events = window.__phase8Events;",
    `const events = window.__phase8Events; events.push({ kind: 'script-executed', label: ${JSON.stringify(label)} });`)
}

let vite
let browser
let startupOutput = ""
let origin

async function startServer() {
  const port = await freePort()
  origin = `http://${bindHost.includes(":") ? `[${bindHost}]` : bindHost}:${port}`
  vite = spawn(process.execPath, [path.join(workspace, "frontend/node_modules/vite/bin/vite.js"),
    "--host", bindHost, "--port", String(port), "--strictPort"], {
    cwd: path.join(workspace, "frontend"), windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
  })
  vite.stdout.on("data", (chunk) => { startupOutput = (startupOutput + chunk).slice(-4000) })
  vite.stderr.on("data", (chunk) => { startupOutput = (startupOutput + chunk).slice(-4000) })
  let spawnError
  vite.on("error", (error) => { spawnError = error })
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    if (spawnError) throw spawnError
    if (vite.exitCode !== null) throw new Error(`Vite stopped before readiness: ${startupOutput}`)
    // Chromium navigation below proves HTTP readiness. Node loopback probes are
    // filtered on some Windows hosts even when the browser can reach the service.
    if (startupOutput.includes("ready in")) return
    await pause(200)
  }
  throw new Error(`Temporary Vite server did not become ready: ${startupOutput}`)
}

async function fixture(options = {}) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 960 } })
  const page = await context.newPage()
  page.setDefaultTimeout(12_000)
  const errors = []
  page.on("pageerror", (error) => errors.push(error.message))
  const pending = new Set()
  const failed = []
  const consoleErrors = []
  const scripts = []
  page.on("console", (message) => {
    if (message.type() === "error") consoleErrors.push(redact(message.text()))
  })
  page.on("response", (response) => {
    const pathname = new URL(response.url()).pathname
    if (pathname === "/src/main.tsx" || pathname.startsWith("/@")) scripts.push({ pathname,
      status: response.status(), contentType: response.headers()["content-type"] })
  })
  page.on("request", (request) => pending.add(new URL(request.url()).pathname))
  page.on("requestfinished", (request) => pending.delete(new URL(request.url()).pathname))
  page.on("requestfailed", (request) => {
    const pathname = new URL(request.url()).pathname
    pending.delete(pathname)
    failed.push({ pathname, error: request.failure()?.errorText })
  })
  const state = { version: "v1", label: "原菜单", issueCount: 0, contextCount: 0,
    held: null, release: null, holdNextA: false, ...options }
  const leases = new Map()
  await context.addInitScript(() => {
    localStorage.setItem("dante-token", "test-session-A")
    localStorage.setItem("dante-theme", "light")
    window.__phase8Events = []
    window.__phase8Control = { nextId: 0 }
  })
  await context.routeWebSocket("**/*XTransformPort=3003*", (socket) => socket.close())
  await context.route("**/*XTransformPort=3003*", (route) => route.abort())
  await context.route("**/__phase8/**", async (route) => {
    const isStatic = new URL(route.request().url()).pathname.includes("probe-static")
    if (new URL(route.request().url()).pathname.endsWith("/app.js")) {
      const delayed = state.holdFirstScript && !state.scriptHeld
      if (delayed) {
        state.scriptHeld = true
        state.scriptRelease = deferred()
        await state.scriptRelease.promise
      }
      return route.fulfill({ status: 200, contentType: "text/javascript",
        body: probeJavaScript(!isStatic, delayed ? "cancelled-script" : "current-script") }).catch((error) => {
        if (!/closed|disposed|Invalid Interception|Target|cancelled/i.test(error.message)) throw error
      })
    }
    return route.fulfill({ status: 200, contentType: "text/html", body: state.externalScript
      ? externalProbeHtml(!isStatic, state.asyncScript) : probeHtml(!isStatic) })
  })
  const fulfill = (route, value, status = 200) => route.fulfill({
    status, contentType: "application/json", body: JSON.stringify(value),
  }).catch((error) => {
    if (!/closed|disposed|Invalid Interception|Target|cancelled/i.test(error.message)) throw error
  })
  await context.route((url) => url.origin === origin && url.pathname.startsWith("/api/"), async (route) => {
    const request = route.request()
    const url = new URL(request.url())
    const bearer = request.headers().authorization?.replace(/^Bearer /, "")
    const group = bearer === "test-session-B" ? "B" : "A"
    if (url.pathname === "/api/__phase8-delayed401") {
      state.pending401 = deferred()
      state.held401 = true
      await state.pending401.promise
      return fulfill(route, { message: "旧上下文会话被撤销", code: "UNAUTHORIZED" }, 401)
    }
    if (url.pathname === "/api/auth/profile") return fulfill(route,
      profile(group, group === "B" && state.noAppsB ? [] : applications))
    if (url.pathname === "/api/auth/menus") {
      const app = applications.find((value) => value.id === url.searchParams.get("applicationId")) ?? applications[0]
      if (state.holdMenusApp === app.id && !state.heldMenus) {
        state.heldMenus = true
        state.menusRelease = deferred()
        await state.menusRelease.promise
      }
      return fulfill(route, menu(app.code, state.label))
    }
    if (url.pathname === "/api/auth/permissions") return fulfill(route, { permissions: ["probe:read"], roles: [] })
    if (url.pathname === "/api/external/menus") {
      const lease = leases.get(bearer)
      assert(lease, "Real micro applications must call APIs with their application token.")
      return fulfill(route, { application: { code: lease.appCode, name: "验收应用" },
        client: { tokenType: "user", username: user.username }, menus: menu(lease.appCode, state.label) })
    }
    if (url.pathname === "/api/external/roles") {
      const lease = leases.get(bearer)
      assert(lease, "Real micro applications must call APIs with their application token.")
      return fulfill(route, { application: { code: lease.appCode, name: "验收应用" }, organizations: [{
        id: `org-${lease.group}`, name: `组织 ${lease.group}`, tenantName: `租户 ${lease.group}`,
        roles: [{ id: "role-reader", code: "reader", name: "读取者" }],
      }] })
    }
    if (url.pathname === "/api/oauth/session-token") {
      if (state.failAt === "issue") return fulfill(route, { message: "应用授权已撤销", code: "FORBIDDEN" }, state.failStatus)
      const appCode = request.postDataJSON().appCode
      const token = `test-application-${++state.issueCount}`
      leases.set(token, { group, appCode })
      return fulfill(route, { accessToken: token, expiresIn: 4, tokenType: "Bearer", audience: appCode })
    }
    if (/^\/api\/apps\/[^/]+\/context$/.test(url.pathname)) {
      state.contextCount++
      if (state.failAt === "context") return fulfill(route, { message: "应用授权已撤销", code: "FORBIDDEN" }, state.failStatus)
      const lease = leases.get(bearer)
      assert(lease, "Application context must receive an application token, not the platform session.")
      const currentProfile = profile(lease.group)
      const response = {
        contractVersion: "1.0", contextVersion: `${lease.group}-${state.version}`,
        expiresAt: new Date(Date.now() + 3500).toISOString(), appCode: lease.appCode,
        application: { id: `application-${lease.appCode}`, name: "浏览器验收应用" },
        user: { id: user.id, username: user.username, displayName: user.realName },
        tenant: currentProfile.tenant, organization: currentProfile.organization,
        roles: [{ id: "role-reader", code: "reader", name: "读取者" }], permissions: ["probe:read"],
        dataGrants: [{ resource: "probe", action: "read", scope: "SELF", subjectUserId: user.id,
          tenantId: currentProfile.tenant.id, organizationIds: [] }],
        menus: menu(lease.appCode, state.label),
      }
      if (lease.group === "A" && state.holdNextA) {
        state.holdNextA = false
        state.held = { response }
        state.release = deferred()
        await state.release.promise
      }
      return fulfill(route, response)
    }
    if (url.pathname.startsWith("/api/monitor/")) return fulfill(route, { message: "无监控授权", code: "FORBIDDEN" }, 403)
    if (/prefs|preferences/.test(url.pathname)) return fulfill(route, { mutedTypes: [], dndEnabled: false, dndStart: "22:00", dndEnd: "08:00" })
    if (url.pathname.includes("unread")) return fulfill(route, { count: 0 })
    return fulfill(route, { records: [], total: 0, page: 1, size: 10 })
  })
  const documentResponse = await page.goto(`${origin}/micro/probe`, { waitUntil: "domcontentloaded" })
  state.documentStatus = documentResponse?.status()
  state.documentType = documentResponse?.headers()["content-type"]
  return { context, page, state, errors, pending, failed, consoleErrors, scripts, close: async () => {
    state.release?.resolve()
    state.scriptRelease?.resolve()
    state.menusRelease?.resolve()
    state.pending401?.resolve()
    await context.close()
  } }
}

const events = (page) => page.evaluate(() => window.__phase8Events)
const waitEvent = (page, criteria) => page.waitForFunction((expected) =>
  window.__phase8Events.some((entry) => Object.entries(expected).every(([key, value]) => entry[key] === value)), criteria)
async function waitState(predicate, message) {
  const deadline = Date.now() + 8000
  while (!predicate() && Date.now() < deadline) await pause(50)
  assert(predicate(), message)
}
async function withFixture(options, run) {
  const test = await fixture(options)
  try {
    await run(test)
    assert.deepEqual(test.errors, [], "Browser must not produce uncaught errors.")
  } catch (error) {
    console.error("Browser failure diagnostic:", {
      pathname: new URL(test.page.url()).pathname,
      tokenIssueRequests: test.state.issueCount, contextRequests: test.state.contextCount,
      documentStatus: test.state.documentStatus, documentType: test.state.documentType,
      errors: test.errors.map(redact), body: redact((await test.page.locator("body").innerText()).slice(0, 4000)),
      html: redact((await test.page.content()).slice(0, 2000)),
      pending: [...test.pending], failed: test.failed, viteOutput: startupOutput,
      consoleErrors: test.consoleErrors, scripts: test.scripts,
      events: await events(test.page),
    })
    throw error
  } finally { await test.close() }
}

async function verifyUpdatesAndSwitch() {
  await withFixture({}, async ({ page, state }) => {
    await waitEvent(page, { kind: "mount", appCode: "probe", organizationId: "org-A" })
    const initial = (await events(page)).find((entry) => entry.kind === "mount")
    assert.equal(initial.contractVersion, "2.0")
    assert.equal(initial.contextVersion, "A-v1")
    assert.equal(initial.tokenPresent, true)
    assert.equal(initial.hasSignal, true)
    assert.equal(initial.signalAborted, false)
    assert.deepEqual(initial.missing, [])
    assert.deepEqual(initial.roleCodes, ["reader"])
    assert.deepEqual(initial.permissions, ["probe:read"])
    assert.equal(initial.dataGrants[0].scope, "SELF")
    assert.equal(initial.tenantId, "tenant-A")
    assert.equal(initial.userId, user.id)
    assert(Date.parse(initial.expiresAt) > initial.at)
    assert(Date.parse(initial.tokenExpiresAt) > initial.at)

    await page.getByRole("button", { name: "切换到深色模式", exact: true }).click()
    await waitEvent(page, { kind: "update", theme: "dark" })
    await page.evaluate(() => window.__phase8Control.navigate("/micro/probe/detail"))
    await waitEvent(page, { kind: "update", routePath: "/micro/probe/detail" })
    state.version = "v2"
    state.label = "菜单改名但数量不变"
    await waitEvent(page, { kind: "update", contextVersion: "A-v2" })
    const menuUpdate = (await events(page)).find((entry) => entry.kind === "update" && entry.contextVersion === "A-v2")
    assert.deepEqual(menuUpdate.menuTitles, [state.label])
    assert.equal(menuUpdate.menuTitles.length, initial.menuTitles.length)
    await waitEvent(page, { kind: "update", tokenChanged: true })
    const beforeSwitch = await events(page)
    assert.equal(beforeSwitch.filter((entry) => entry.kind === "mount").length, 1, "All ordinary changes must use update.")
    assert.equal(beforeSwitch.filter((entry) => entry.kind === "unmount-start").length, 0)

    state.holdNextA = true
    await waitState(() => state.held, "An old application context response must be in flight before switching.")
    await page.evaluate(async () => {
      const { request } = await import("/src/lib/api.ts")
      window.__phase8Control.old401Result = request("/api/__phase8-delayed401").catch((error) => error.status)
    })
    await waitState(() => state.held401, "Old platform request must be pending before switching.")
    const oldSignalAborted = await page.evaluate(async () => {
      const { useAuthStore } = await import("/src/stores/auth.ts")
      const oldSignal = window.__phase8Control.signal
      useAuthStore.getState().beginContextSwitch()
      await new Promise((resolve) => setTimeout(resolve, 50))
      return oldSignal.aborted
    })
    assert.equal(oldSignalAborted, true, "Begin switching must immediately invalidate the old lifetime.")
    await page.waitForFunction(() => !document.querySelector('[data-testid="phase8-probe"][data-org="org-A"]'))
    state.pending401.resolve()
    assert.equal(await page.evaluate(() => window.__phase8Control.old401Result), 401)
    assert.equal(await page.evaluate(async () => {
      const { useAuthStore } = await import("/src/stores/auth.ts")
      return useAuthStore.getState().token === "test-session-A" && useAuthStore.getState().contextSwitching
    }), true, "Late 401 must not log out during switching while the platform token has not changed yet.")
    const afterBegin = (await events(page)).length
    await page.evaluate(async () => {
      const { useAuthStore } = await import("/src/stores/auth.ts")
      useAuthStore.getState().setToken("test-session-B")
    })
    await waitEvent(page, { kind: "mount", organizationId: "org-B" })
    state.release.resolve()
    await pause(200)
    const afterSwitch = await events(page)
    assert(!afterSwitch.slice(afterBegin).some((entry) => ["mount", "update"].includes(entry.kind) && entry.organizationId === "org-A"), "Late old context cannot flow back into the application.")
    assert.equal(await page.locator('[data-testid="phase8-probe"]').getAttribute("data-org"), "org-B")
    const oldEnd = afterSwitch.findIndex((entry) => entry.kind === "unmount-end" && entry.organizationId === "org-A")
    const newMount = afterSwitch.findIndex((entry) => entry.kind === "mount" && entry.organizationId === "org-B")
    assert(oldEnd >= 0 && oldEnd < newMount, "New organization mount waits for old asynchronous unmount.")
    console.log("PASS real host: complete props, theme/route/content update, renewal, immediate switch cleanup, late context/401 isolation")
  })
}

async function verifyApplicationSelectionRace() {
  await withFixture({}, async ({ page, state }) => {
    await waitEvent(page, { kind: "mount", appCode: "probe" })
    const beforeExit = (await events(page)).filter((entry) => entry.kind === "mount").length
    await page.evaluate(() => window.__phase8Control.navigate("/dashboard"))
    await page.waitForURL("**/dashboard")
    await page.getByText("我的应用", { exact: true }).waitFor()
    assert.equal((await events(page)).filter((entry) => entry.kind === "mount").length, beforeExit,
      "Outgoing animated micro routes must not remount after leaving.")
    assert.equal(await page.getByText("当前组织无法访问此应用，已退出应用", { exact: true }).count(), 0,
      "Leaving a valid application must not produce an authorization-failure toast.")
    const rail = page.getByRole("complementary", { name: "应用导航", exact: true })
    state.holdMenusApp = "application-probe-static"
    await rail.getByRole("button", { name: "切换到应用 重挂验收应用", exact: true }).click()
    await waitState(() => state.heldMenus, "First selected application's menus must be delayed.")
    await rail.getByRole("button", { name: "切换到应用 上下文验收应用", exact: true }).click()
    await page.waitForURL("**/micro/probe/detail")
    state.menusRelease.resolve()
    await pause(200)
    assert.equal(new URL(page.url()).pathname, "/micro/probe/detail", "Late first application's menu result must not navigate away from the second selection.")
    assert.equal(await page.evaluate(async () => {
      const { useAuthStore } = await import("/src/stores/auth.ts")
      return useAuthStore.getState().currentApplicationId === "application-probe"
    }), true)
    console.log("PASS real application rail: rapid selection ignores late previous application's menus")
  })
}

async function verifyCancelledHtmlScripts() {
  for (const asyncScript of [false, true]) {
    await withFixture({ externalScript: true, asyncScript, holdFirstScript: true }, async ({ page, state }) => {
      await waitState(() => state.scriptHeld, "First external HTML application script must be in flight.")
      assert.equal((await events(page)).filter((entry) => entry.kind === "mount").length, 0)
      await page.evaluate(async () => {
        const { useAuthStore } = await import("/src/stores/auth.ts")
        useAuthStore.getState().beginContextSwitch()
        await new Promise((resolve) => setTimeout(resolve, 50))
        useAuthStore.getState().setToken("test-session-B")
      })
      await waitEvent(page, { kind: "mount", organizationId: "org-B" })
      state.scriptRelease.resolve()
      await pause(200)
      const history = await events(page)
      assert(history.some((entry) => entry.kind === "script-executed" && entry.label === "current-script"))
      assert(!history.some((entry) => entry.kind === "script-executed" && entry.label === "cancelled-script"), "Cancelled external scripts cannot create late side effects.")
      assert(!history.some((entry) => entry.kind === "mount" && entry.organizationId === "org-A"))
      assert.equal(await page.locator('[data-testid="phase8-probe"]').getAttribute("data-org"), "org-B")
      console.log(`PASS real HTML loader: cancelled ${asyncScript ? "async" : "ordinary"} script stays inert and the same entry URL mounts after retry`)
    })
  }
}

async function verifyRealVueAndJQuery() {
  await withFixture({}, async ({ page, state }) => {
    await waitEvent(page, { kind: "mount", appCode: "probe" })
    await page.evaluate(() => window.__phase8Control.navigate("/micro/vue-workshop"))
    const zone = page.locator(".micro-mount-zone")
    await zone.locator(".mf-vue-root").waitFor()
    const expectedAuthorization = ["租户 A", "组织 A", "读取者 (reader)", "probe:read", "独立数据授权", "probe:read · SELF"]
    await page.waitForFunction((expected) => {
      const node = document.querySelector(".mf-vue-root")
      return node && expected.every((text) => node.textContent.includes(text))
    }, expectedAuthorization)
    await page.getByTestId("micro-status-bar").getByText("运行中", { exact: true }).waitFor()
    await page.evaluate(() => { window.__phase8RealRoot = document.querySelector(".mf-vue-root") })
    const vueExpiry = await zone.locator(".mf-vue-kv").filter({ hasText: "tokenExpiresAt" }).innerText()
    state.version = "vue-v2"
    await zone.locator(".mf-vue-kv").filter({ hasText: "contextVersion" }).getByText("A-vue-v2", { exact: true }).waitFor()
    await page.waitForFunction((oldValue) => {
      const row = [...document.querySelectorAll(".mf-vue-kv")].find((item) => item.textContent.includes("tokenExpiresAt"))
      return row && row.innerText !== oldValue
    }, vueExpiry)
    assert.equal(await page.evaluate(() => window.__phase8RealRoot === document.querySelector(".mf-vue-root")), true,
      "Real Vue must keep its root DOM through context and token renewal updates.")
    await zone.getByRole("button", { name: "GET /api/external/menus", exact: true }).click()
    await zone.getByText("HTTP 200", { exact: false }).first().waitFor()

    state.version = "jquery-v1"
    const rail = page.getByRole("complementary", { name: "应用导航", exact: true })
    await rail.getByRole("button", { name: "切换到应用 真实 jQuery 验收应用", exact: true }).click()
    await zone.locator(".mf-jq-root").waitFor()
    assert.equal(await page.locator(".mf-vue-root").count(), 0)
    await page.waitForFunction((expected) => {
      const node = document.querySelector(".mf-jq-root")
      return node && expected.every((text) => node.textContent.includes(text))
    }, expectedAuthorization)
    await page.getByTestId("micro-status-bar").getByText("运行中", { exact: true }).waitFor()
    await page.evaluate(() => { window.__phase8RealRoot = document.querySelector(".mf-jq-root") })
    const jqExpiry = await zone.locator("#mf-jq-context li").filter({ hasText: "令牌到期:" }).innerText()
    state.version = "jquery-v2"
    await zone.locator("#mf-jq-context").getByText("A-jquery-v2", { exact: true }).waitFor()
    await page.waitForFunction((oldValue) => {
      const row = [...document.querySelectorAll("#mf-jq-context li")].find((item) => item.textContent.includes("令牌到期:"))
      return row && row.innerText !== oldValue
    }, jqExpiry)
    assert.equal(await page.evaluate(() => window.__phase8RealRoot === document.querySelector(".mf-jq-root")), true,
      "Real jQuery must keep its root DOM through context and token renewal updates.")
    await zone.locator("#mf-jq-btn-menus").click()
    await zone.locator("#mf-jq-menu-status").getByText(/HTTP 200/).waitFor()
    await rail.getByRole("button", { name: "切换到应用 上下文验收应用", exact: true }).click()
    await page.locator('[data-testid="phase8-probe"]').waitFor()
    assert.equal(await page.locator(".mf-jq-root").count(), 0)
    assert.equal(await page.locator(".mf-vue-root").count(), 0)
    console.log("PASS real Vue/jQuery vendors: full authorization display, renewal updates, API token use, clean entry/exit")
  })
}

async function verifySerialRemount() {
  await withFixture({}, async ({ page, state }) => {
    await waitEvent(page, { kind: "mount", appCode: "probe" })
    await page.evaluate(() => window.__phase8Control.navigate("/micro/probe-static"))
    await waitEvent(page, { kind: "mount", appCode: "probe-static" })
    state.version = "v2"
    state.label = "无 update 的新菜单"
    await waitEvent(page, { kind: "mount", appCode: "probe-static", contextVersion: "A-v2" })
    const history = await events(page)
    const mounts = history.filter((entry) => entry.kind === "mount" && entry.appCode === "probe-static")
    assert(mounts.length >= 2)
    const retiring = mounts[0]
    const start = history.findIndex((entry) => entry.kind === "unmount-start" && entry.instanceId === retiring.instanceId)
    const end = history.findIndex((entry) => entry.kind === "unmount-end" && entry.instanceId === retiring.instanceId)
    const next = history.findIndex((entry) => entry.kind === "mount" && entry.instanceId === mounts[1].instanceId)
    assert(start >= 0 && start < end && end < next, "No-update applications must unmount completely before remount.")
    assert(!history.some((entry) => entry.kind === "update" && entry.appCode === "probe-static"))
    console.log("PASS real host: applications without update remount in strict lifecycle order")
  })
}

async function verifyAuthorizationFailures() {
  for (const failStatus of [401, 403, 404]) {
    await withFixture({ failAt: "context", failStatus }, async ({ page }) => {
      await page.waitForURL("**/dashboard")
      await page.getByText("当前应用授权已失效，已退出应用", { exact: true }).waitFor()
      assert.equal((await events(page)).filter((entry) => entry.kind === "mount").length, 0)
      assert.equal(await page.evaluate(() => localStorage.getItem("dante-token") !== null), true,
        "Application authorization errors must preserve the independently valid platform session.")
      console.log(`PASS real host: application context ${failStatus} exits micro route with an explicit toast`)
    })
  }
  await withFixture({ failAt: "issue", failStatus: 403 }, async ({ page }) => {
    await page.waitForURL("**/dashboard")
    await page.getByText("当前应用授权已失效，已退出应用", { exact: true }).waitFor()
    assert.equal((await events(page)).filter((entry) => entry.kind === "mount").length, 0)
    console.log("PASS real host: application token issuance failure exits micro route")
  })
  await withFixture({ noAppsB: true }, async ({ page }) => {
    await waitEvent(page, { kind: "mount", appCode: "probe" })
    await page.evaluate(async () => {
      const { useAuthStore } = await import("/src/stores/auth.ts")
      useAuthStore.getState().beginContextSwitch()
      useAuthStore.getState().setToken("test-session-B")
    })
    await page.waitForURL("**/dashboard")
    await page.getByText("当前组织无法访问此应用，已退出应用", { exact: true }).waitFor()
    assert.equal(await page.locator('[data-testid="phase8-probe"]').count(), 0)
    console.log("PASS real host: unavailable application after organization switch exits micro route")
  })
}

try {
  await startServer()
  browser = await chromium.launch({ headless: true, executablePath: browserExecutable, args: ["--no-proxy-server"] })
  const groups = { updates: verifyUpdatesAndSwitch, remount: verifySerialRemount,
    selection: verifyApplicationSelectionRace, scripts: verifyCancelledHtmlScripts,
    vendors: verifyRealVueAndJQuery, failures: verifyAuthorizationFailures }
  const selected = process.argv[2]
  assert(!selected || selected in groups, "Unknown browser scenario group.")
  for (const [name, run] of Object.entries(groups)) if (!selected || selected === name) await run()
  console.log(`Phase 8 browser integration passed (${selected || "all 11 scenarios"}); APIs and credentials were isolated.`)
} finally {
  await browser?.close()
  if (vite && vite.exitCode === null) {
    vite.kill()
    await Promise.race([new Promise((resolve) => vite.once("exit", resolve)), pause(5000)])
    if (vite.exitCode === null) vite.kill("SIGKILL")
  }
}
