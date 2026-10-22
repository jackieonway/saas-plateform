/** Real AppLayout regression: denied routes never mount or fetch business data. */
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import net from "node:net"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { launchBrowser } from "./browser/runtime.mjs"

const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const bindHost = process.env.ROUTE_PERMISSIONS_BROWSER_BIND || (process.platform === "win32" ? "::1" : "127.0.0.1")
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const freePort = () => new Promise((resolve, reject) => {
  const server = net.createServer().on("error", reject)
  server.listen(0, bindHost, () => {
    const port = server.address().port
    server.close((error) => error ? reject(error) : resolve(port))
  })
})
async function waitState(predicate, message, timeout = 12_000) {
  const deadline = Date.now() + timeout
  while (!predicate() && Date.now() < deadline) await pause(25)
  assert(predicate(), message)
}
let browser, vite, origin, startup = "", count = 0
async function startServer() {
  const port = await freePort()
  origin = `http://${bindHost.includes(":") ? `[${bindHost}]` : bindHost}:${port}`
  vite = spawn(process.execPath, [path.join(workspace, "node_modules/vite/bin/vite.js"),
    "--host", bindHost, "--port", String(port), "--strictPort"], {
    cwd: workspace, windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
  })
  vite.stdout.on("data", (chunk) => { startup = (startup + chunk).slice(-4000) })
  vite.stderr.on("data", (chunk) => { startup = (startup + chunk).slice(-4000) })
  let spawnError
  vite.on("error", (error) => { spawnError = error })
  await waitState(() => {
    if (spawnError) throw spawnError
    if (vite.exitCode !== null) throw new Error(`Vite stopped: ${startup}`)
    return startup.includes("ready in")
  }, "Vite did not become ready", 30_000)
}

const profile = {
  user: { id: "guard-user", username: "guard", realName: "权限验收", email: "guard@example.test",
    phone: "", gender: "UNKNOWN", avatar: "", isSuperAdmin: true },
  tenant: { id: "guard-tenant", name: "权限验收租户", code: "guard-tenant", isPlatform: true },
  organization: { id: "guard-org", name: "权限验收组织", path: "权限验收组织" },
  applications: [{ id: "guard-system", code: "system", name: "系统管理", icon: "LayoutGrid", basePath: "/system" }],
  memberships: [{ userId: "guard-user", tenantId: "guard-tenant", tenantName: "权限验收租户",
    organizationId: "guard-org", organizationName: "权限验收组织", organizationPath: "权限验收组织" }],
}
const tenant = { id: "private-tenant", name: "先前租户的私有数据", code: "private-tenant", tenantType: "COMPANY",
  contactName: "联系人", contactPhone: "13800138000", contactEmail: "contact@example.test", status: "ENABLED",
  isPlatform: false, description: "", organizationCount: 1, userCount: 1, authorizedAppCount: 1,
  createdAt: "2026-10-13T00:00:00Z", expiresAt: null }
const sidebarMenus = [{ id: "fold-root", parentId: null, type: "DIR", title: "组织机构", path: null, icon: "Building2",
  permission: null, sortOrder: 1, visible: true, children: [{ id: "fold-child", parentId: "fold-root", type: "DIR",
    title: "组织维护", path: null, icon: "Network", permission: null, sortOrder: 1, visible: true,
    children: [{ id: "fold-leaf", parentId: "fold-child", type: "MENU", title: "组织管理", path: "/system/organizations",
      icon: "Network", permission: "system:organization", sortOrder: 1, visible: true }] }] }]
const microApp = { id: "guard-micro-app", code: "guardmicro", name: "菜单权限验收微应用", icon: "Boxes",
  basePath: "/micro/guardmicro", runtimeMode: "TRUSTED", entryType: "HTML", entryUrl: "/__guardmicro/index.html" }
const microMenu = (suffix = "") => ({ id: `guard-micro-menu-${suffix}`, parentId: null, type: "MENU", title: "已授权微应用菜单",
  path: `/micro/guardmicro${suffix}`, icon: "Boxes", permission: "guardmicro:read", sortOrder: 1, visible: true })
const businessPath = (pathname) => /^\/api\/(tenants|organizations|departments|applications|menus|users|roles|regions|oauth|audit-logs|monitor)(\/|$)/.test(pathname) || pathname === "/api/platform/settings"

async function fixture(pathname, options = {}) {
  const context = await browser.newContext({ viewport: options.viewport ?? { width: 1440, height: 1000 } })
  const page = await context.newPage()
  page.setDefaultTimeout(12_000)
  const state = { requests: [], errors: [], permissions: options.permissions ?? [], permissionError: !!options.permissionError,
    externalRequests: [], menus: options.menus ?? (options.allowTenants ? [{ id: "guard-tenant-menu", parentId: null,
      type: "MENU", title: "租户管理", path: "/system/tenants", icon: "Building2", permission: "system:tenant", sortOrder: 1, visible: true }] : []),
    profile: structuredClone(profile), entries: 0, contextRequests: 0,
    microContextMenus: options.microContextMenus ?? options.menus ?? [] }
  state.profile.tenant.isPlatform = options.isPlatform ?? true
  if (options.platformContext !== undefined) state.profile.platformContext = options.platformContext
  if (options.microApp) state.profile.applications.push(microApp)
  let releasePermissions
  const permissionGate = options.loading ? new Promise((resolve) => { releasePermissions = resolve }) : Promise.resolve()
  state.releasePermissions = () => releasePermissions?.()
  let releaseMenus
  const menusGate = options.loadingMenus ? new Promise((resolve) => { releaseMenus = resolve }) : Promise.resolve()
  state.releaseMenus = () => releaseMenus?.()
  page.on("pageerror", (error) => state.errors.push(error.message))
  await context.addInitScript((authenticated) => {
    if (authenticated) if (!localStorage.getItem("platform-session-v1")) localStorage.setItem("platform-session-v1", JSON.stringify({ schema: 1, version: 1, sessionId: crypto.randomUUID(), status: "active", token: "guard-session", refreshToken: null }))
    localStorage.setItem("platform-theme", "light")
  }, options.authenticated !== false)
  await context.routeWebSocket(() => true, () => {})
  await context.route((url) => url.origin === origin && url.pathname === "/src/main.tsx", async (route) => {
    const response = await route.fetch()
    const body = (await response.text()).replace("ReactDOM.createRoot", "import { authStore as guardAuthStore } from '/src/stores/auth.ts';\nwindow.__permissionAuth = guardAuthStore;\nReactDOM.createRoot")
    assert.match(body, /window\.__permissionAuth/)
    await route.fulfill({ response, body })
  })
  const fulfill = async (route, value, status = 200) => {
    try { await route.fulfill({ status, contentType: "application/json", body: JSON.stringify(value) }) }
    catch (error) { if (!/closed|disposed|Invalid Interception|Target|cancelled/i.test(error.message)) throw error }
  }
  await context.route("https://*.example.test/**", async (route) => {
    state.externalRequests.push(route.request().url())
    await route.fulfill({ status: 200, contentType: "text/html", body: "<p>Authorized external fixture</p>" })
  })
  await context.route((url) => url.origin === origin && url.pathname === microApp.entryUrl, async (route) => {
    state.entries++
    await route.fulfill({ status: 200, contentType: "text/html", body: `<!doctype html><html><body><script>
      var active;
      window.__MICRO_APP__ = { name: "guard-menu-probe",
        mount: function(container, props) { active = container; container.innerHTML = '<p data-testid="guard-micro-mounted">' + props.routePath + '</p>'; },
        update: function(props) { active.querySelector('[data-testid="guard-micro-mounted"]').textContent = props.routePath; },
        unmount: function() { if (active) active.replaceChildren(); }
      };
    </script></body></html>` })
  })
  await context.route((url) => url.origin === origin && url.pathname.startsWith("/api/"), async (route) => {
    const request = route.request(), url = new URL(request.url())
    state.requests.push({ pathname: url.pathname, method: request.method(), bearer: request.headers().authorization })
    if (url.pathname === "/api/platform/branding") return fulfill(route, { platformName: "权限验收平台",
      platformDescription: "隔离测试", logoUrl: "", loginTitle: "欢迎回来", loginDescription: "登录", footerText: "" })
    if (url.pathname === "/api/auth/profile") return fulfill(route, state.profile)
    if (url.pathname === "/api/auth/menus") {
      if (options.microApp && url.searchParams.get("applicationId") !== microApp.id) return fulfill(route, { menus: [] })
      await menusGate
      return fulfill(route, options.menusError ? { message: "菜单授权服务不可用" } : { menus: state.menus }, options.menusError ? 503 : 200)
    }
    if (url.pathname === "/api/oauth/session-token" && options.microApp) return fulfill(route,
      { accessToken: "guard-micro-token", tokenType: "Bearer", expiresIn: 60, audience: microApp.code })
    if (url.pathname === `/api/apps/${microApp.code}/context` && options.microApp) {
      state.contextRequests++
      return fulfill(route, { contractVersion: "1.0", contextVersion: `guard-${state.contextRequests}`,
        expiresAt: new Date(Date.now() + (options.microContextTtlMs ?? 60_000)).toISOString(), appCode: microApp.code,
        application: { id: microApp.id, name: microApp.name, runtimeMode: "TRUSTED",
          entryType: microApp.entryType, entryUrl: microApp.entryUrl, deploymentVersion: null, entryVersionId: null },
        user: { id: state.profile.user.id, username: state.profile.user.username, displayName: state.profile.user.realName },
        tenant: state.profile.tenant, organization: state.profile.organization, roles: [], permissions: ["guardmicro:read"],
        dataGrants: [], menus: state.microContextMenus })
    }
    if (url.pathname === "/api/auth/permissions") {
      await permissionGate
      return fulfill(route, state.permissionError ? { message: "权限服务不可用" } : { permissions: state.permissions, roles: [] }, state.permissionError ? 503 : 200)
    }
    if (url.pathname === "/api/notifications/prefs") return fulfill(route, { dndEnabled: false, dndStart: null, dndEnd: null, mutedTypes: [] })
    if (url.pathname === "/api/notifications/unread-count") return fulfill(route, { count: 0 })
    if (url.pathname === "/api/notifications/socket-context") return fulfill(route, { userId: profile.user.id,
      tenantId: profile.tenant.id, organizationId: profile.organization.id, sessionVersion: 0 })
    if (url.pathname === "/api/notifications" || url.pathname === "/api/auth/login-history") return fulfill(route, { records: [], total: 0, page: 1, size: 10 })
    if (url.pathname === "/api/auth/my-sessions") return fulfill(route, { sessions: [], summary: { active: 0, revoked: 0 } })
    if (url.pathname === "/api/tenants/options" && options.scopedManagement) return fulfill(route, [state.profile.tenant])
    if (url.pathname === "/api/organizations/tree" && options.scopedManagement) return fulfill(route, [{
      id: state.profile.organization.id, name: "当前组织", code: "current-org", tenantId: state.profile.tenant.id,
      parentId: null, organizationType: "COMPANY", status: "ENABLED", sortOrder: 1, description: "", children: [{
        id: "guard-descendant-org", name: "下级组织", code: "descendant-org", tenantId: state.profile.tenant.id,
        parentId: state.profile.organization.id, organizationType: "BRANCH", status: "ENABLED", sortOrder: 1, children: [],
      }],
    }])
    if (url.pathname === "/api/audit-logs" && options.scopedManagement) return fulfill(route, { records: [], total: 0, page: 1, size: 10 })
    if (url.pathname === "/api/monitor/tenant-insights" && options.scopedManagement) return fulfill(route, { insights: [] })
    if (url.pathname === "/api/tenants" && options.allowTenants) return fulfill(route, { records: [tenant], total: 1, page: 1, size: 10 })
    return fulfill(route, { message: `Business page unexpectedly requested ${url.pathname}` }, 403)
  })
  try {
    await page.goto(`${origin}${pathname}`, { waitUntil: "domcontentloaded" })
    return { page, state, context }
  } catch (error) { state.releasePermissions(); state.releaseMenus(); await context.close(); throw error }
}

async function check(name, pathname, options, run) {
  if (process.env.ROUTE_PERMISSIONS_FILTER && !new RegExp(process.env.ROUTE_PERMISSIONS_FILTER).test(name)) return
  const f = await fixture(pathname, options)
  try {
    await run(f)
    assert.deepEqual(f.state.errors, [], "Production AppLayout has no uncaught browser errors")
    count++
    console.log(`PASS ${name}`)
  } catch (error) {
    console.error({ name, requests: f.state.requests, errors: f.state.errors,
      body: await f.page.locator("body").innerText().catch(() => "") })
    throw error
  } finally { f.state.releasePermissions(); f.state.releaseMenus(); await f.context.close() }
}
function noBusinessRequests(state) {
  assert.deepEqual(state.requests.filter((request) => businessPath(request.pathname)), [],
    "Permission checks must prevent business-page requests, even for global superadmins")
}

try {
  await startServer()
  browser = await launchBrowser()
  const deniedPaths = ["/system/tenants", "/system/settings", "/system/applications", "/system/menus",
    "/system/organizations", "/system/departments", "/system/authorizations", "/system/users", "/system/roles",
    "/system/regions", "/system/oauth/clients", "/system/oauth/tokens", "/system/audit-logs", "/system/monitor",
    "/athena/overview", "/athena/reports/sales", "/athena/reports/operations", "/athena/sources",
    "/hermes/inbox", "/hermes/notifications", "/SYSTEM/TENANTS", "/system/%74enants", "/system/tenants/"]
  for (const pathname of deniedPaths) {
    await check(`denied direct route ${pathname}`, pathname, {}, async ({ page, state }) => {
      await page.getByText("无访问权限", { exact: true }).waitFor()
      noBusinessRequests(state)
    })
  }
  await check("authenticated dashboard personal workbench needs no application MENU", "/dashboard", {}, async ({ page, state }) => {
    await page.getByText("我的应用", { exact: true }).waitFor()
    assert.equal(await page.getByText("无访问权限", { exact: true }).count(), 0)
    // The monitor API independently denies its data; only the personal fallback mounts.
    assert(state.requests.some((request) => request.pathname === "/api/monitor/overview"))
    assert.equal(state.requests.filter((request) => businessPath(request.pathname)).every((request) =>
      request.pathname === "/api/monitor/overview"), true)
  })
  await check("unauthenticated deep link redirects before business mount", "/system/tenants", { authenticated: false }, async ({ page, state }) => {
    await page.waitForURL(/\/login\?return=/)
    await page.getByLabel("登录账号").waitFor()
    noBusinessRequests(state)
    assert.equal(state.requests.some((request) => request.pathname.startsWith("/api/auth/")), false)
  })
  await check("loading permissions prevent data access", "/system/tenants", { loading: true }, async ({ page, state }) => {
    await page.getByRole("status", { name: "正在验证访问权限", exact: true }).waitFor()
    noBusinessRequests(state)
    state.releasePermissions()
    await page.getByText("无访问权限", { exact: true }).waitFor()
    noBusinessRequests(state)
  })
  await check("permission errors fail closed with retry", "/system/tenants", { permissionError: true }, async ({ page, state }) => {
    await page.getByText("访问权限验证失败，请重试", { exact: true }).waitFor()
    noBusinessRequests(state)
    state.permissionError = false
    await page.getByRole("button", { name: "重试", exact: true }).click()
    await page.getByText("无访问权限", { exact: true }).waitFor()
    noBusinessRequests(state)
  })
  await check("authorized route mounts with its effective permission", "/system/tenants", { permissions: ["system:tenant"], allowTenants: true }, async ({ page, state }) => {
    await page.getByText(tenant.name, { exact: true }).waitFor()
    assert.equal(state.requests.some((request) => request.pathname === "/api/tenants"), true)
    assert.equal(await page.getByText("无访问权限", { exact: true }).count(), 0)
  })
  for (const [grantedPath, deniedPath] of [["/system/oauth/clients", "/system/oauth/tokens"], ["/system/oauth/tokens", "/system/oauth/clients"]]) {
    await check(`shared permission code cannot grant unauthorized sibling host menu ${deniedPath}`, deniedPath, {
      permissions: ["system:oauth"], menus: [{ id: "guard-oauth-menu", parentId: null, type: "MENU", title: "OAuth 授权菜单",
        path: grantedPath, icon: "Key", permission: "system:oauth", sortOrder: 1, visible: true }],
    }, async ({ page, state }) => {
      await page.getByText("无访问权限", { exact: true }).waitFor()
      noBusinessRequests(state)
    })
  }
  for (const [path, permission] of [["/system/settings", "system:settings"], ["/system/applications", "system:application"],
    ["/system/menus", "system:menu"], ["/system/oauth/clients", "system:oauth"], ["/system/oauth/tokens", "system:oauth"]]) {
    await check(`platform child capability false denies global page ${path} before data requests`, path, {
      isPlatform: true, platformContext: false, permissions: [permission], menus: [{ id: "global-granted-menu", parentId: null,
        type: "MENU", title: "已授权平台功能", path, icon: "Settings", permission, sortOrder: 1, visible: true }],
    }, async ({ page, state }) => {
      await page.getByText("无访问权限", { exact: true }).waitFor()
      noBusinessRequests(state)
      assert.equal(state.profile.user.isSuperAdmin, true)
      assert.equal(state.profile.tenant.isPlatform, true)
    })
  }
  await check("platform child capability false keeps tenant administration read only", "/system/tenants", {
    platformContext: false, permissions: ["system:tenant"], allowTenants: true,
  }, async ({ page, state }) => {
    await page.getByText(tenant.name, { exact: true }).waitFor()
    assert.equal(await page.getByRole("button", { name: "新建租户", exact: true }).count(), 0)
    assert.equal(await page.getByRole("button", { name: `编辑 ${tenant.name}`, exact: true }).count(), 0)
    assert.equal(await page.getByRole("button", { name: "初始化首位管理员", exact: true }).count(), 0)
    assert.equal(state.requests.filter((request) => businessPath(request.pathname)).every((request) => request.pathname === "/api/tenants"), true)
  })
  await check("platform child capability false hides tenant authorization tab", "/system/authorizations", {
    platformContext: false, scopedManagement: true, permissions: ["system:authorization", "system:tenant"],
    menus: [{ id: "org-authorization-menu", parentId: null, type: "MENU", title: "授权管理", path: "/system/authorizations",
      icon: "Shield", permission: "system:authorization", sortOrder: 1, visible: true }],
  }, async ({ page, state }) => {
    await page.getByRole("heading", { name: "授权管理", exact: true }).waitFor()
    await waitState(() => state.requests.some((request) => request.pathname === "/api/organizations/tree"), "Organization selection uses scoped server tree")
    assert.equal(await page.getByRole("tab", { name: "租户授权", exact: true }).count(), 0)
    assert.equal(state.requests.some((request) => /^\/api\/tenants\/[^/]+\/authorizations/.test(request.pathname)), false)
  })
  await check("platform child ignores organization URL outside scoped server tree", "/system/authorizations?organizationId=ungranted-ancestor", {
    platformContext: false, scopedManagement: true, permissions: ["system:authorization", "system:tenant"],
    menus: [{ id: "org-authorization-menu", parentId: null, type: "MENU", title: "授权管理", path: "/system/authorizations",
      icon: "Shield", permission: "system:authorization", sortOrder: 1, visible: true }],
  }, async ({ page, state }) => {
    await page.getByRole("heading", { name: "授权管理", exact: true }).waitFor()
    await waitState(() => state.requests.some((request) => request.pathname === "/api/organizations/tree"), "Scope is checked against current organization tree")
    assert.equal(state.requests.some((request) => request.pathname.includes("ungranted-ancestor")), false,
      "A URL selection outside current org/descendants must not create an authorization editor request")
  })
  await check("platform child capability false avoids cross tenant audit options", "/system/audit-logs", {
    platformContext: false, scopedManagement: true, permissions: ["system:audit"],
    menus: [{ id: "audit-scope-menu", parentId: null, type: "MENU", title: "审计日志", path: "/system/audit-logs",
      icon: "FileText", permission: "system:audit", sortOrder: 1, visible: true }],
  }, async ({ page, state }) => {
    await page.getByRole("heading", { name: "审计日志", exact: true }).waitFor()
    await waitState(() => state.requests.some((request) => request.pathname === "/api/audit-logs"), "Scoped audit rows may still load")
    assert.equal(state.requests.some((request) => request.pathname === "/api/tenants/options"), false)
    assert.equal(await page.getByRole("columnheader", { name: "租户", exact: true }).count(), 0)
  })
  await check("platform child capability false shows scoped monitor without global system API", "/system/monitor", {
    platformContext: false, scopedManagement: true, permissions: ["system:monitor"],
    menus: [{ id: "monitor-scope-menu", parentId: null, type: "MENU", title: "平台监控", path: "/system/monitor",
      icon: "Activity", permission: "system:monitor", sortOrder: 1, visible: true }],
  }, async ({ page, state }) => {
    await page.getByRole("heading", { name: "平台监控", exact: true }).waitFor()
    await waitState(() => state.requests.some((request) => request.pathname === "/api/monitor/tenant-insights"), "Scoped monitor insights may still load")
    assert.equal(await page.getByRole("tab", { name: "平台状态", exact: true }).count(), 0)
    assert.equal(state.requests.some((request) => request.pathname === "/api/monitor/system"), false)
  })
  await check("context change removes old data and permissions before remount", "/system/tenants", { permissions: ["system:tenant"], allowTenants: true }, async ({ page, state }) => {
    await page.getByText(tenant.name, { exact: true }).waitFor()
    await page.evaluate(() => window.__permissionAuth.getState().beginContextSwitch())
    await page.getByText("正在切换组织...", { exact: true }).waitFor()
    assert.equal(await page.getByText(tenant.name, { exact: true }).count(), 0)
    const before = state.requests.filter((request) => request.pathname === "/api/tenants").length
    state.permissions = []
    await page.evaluate(() => window.__permissionAuth.getState().setSession("new-guard-session", "new-guard-refresh"))
    await page.getByText("无访问权限", { exact: true }).waitFor()
    assert.equal(await page.getByText(tenant.name, { exact: true }).count(), 0)
    assert.equal(state.requests.filter((request) => request.pathname === "/api/tenants").length, before)
  })
  await check("personal profile remains available without app permissions", "/profile", {}, async ({ page, state }) => {
    await page.getByRole("heading", { name: "个人中心", exact: true }).waitFor()
    noBusinessRequests(state)
    assert.equal(await page.getByText("无访问权限", { exact: true }).count(), 0)
  })
  await check("ungranted external URL does not mount an iframe", "/embed?url=https%3A%2F%2Fungranted.example.test%2Fprivate", {}, async ({ page, state }) => {
    await page.getByText("无访问权限", { exact: true }).waitFor()
    assert.deepEqual(state.externalRequests, [])
    assert.equal(await page.locator("iframe").count(), 0)
  })
  await check("external directory grant does not substitute for an authorized menu", "/embed?url=https%3A%2F%2Fauthorized.example.test%2Fembed", {
    menus: [{ id: "external-directory", parentId: null, type: "DIR", title: "外链目录", path: null, icon: null,
      permission: null, sortOrder: 1, visible: true, linkUrl: "https://authorized.example.test/embed", linkTarget: "SELF" }],
  }, async ({ page, state }) => {
    await page.getByText("无访问权限", { exact: true }).waitFor()
    assert.deepEqual(state.externalRequests, [])
    assert.equal(await page.locator("iframe").count(), 0)
  })
  await check("external iframe only mounts an effective menu URL", "/embed?url=https%3A%2F%2Fauthorized.example.test%2Fembed&title=授权外链", {
    menus: [{ id: "external-menu", parentId: null, type: "MENU", title: "授权外链", path: null, icon: null,
      permission: null, sortOrder: 1, visible: true, linkUrl: "https://authorized.example.test/embed", linkTarget: "SELF" }],
  }, async ({ page, state }) => {
    await page.getByRole("heading", { name: "授权外链", exact: true }).waitFor()
    await waitState(() => state.externalRequests.length === 1, "Authorized URL must mount once")
    assert.equal(await page.locator("iframe").getAttribute("src"), "https://authorized.example.test/embed")
  })
  await check("micro ungranted app cannot load entry for a global administrator", "/micro/guardmicro", {}, async ({ page, state }) => {
    await page.getByText("无访问权限", { exact: true }).waitFor()
    assert.equal(state.entries, 0)
    assert.equal(state.contextRequests, 0)
    noBusinessRequests(state)
  })
  for (const isPlatform of [true, false]) {
    await check(`micro no menus deny ${isPlatform ? "platform" : "ordinary"} organization global admin`, "/micro/guardmicro", {
      microApp: true, isPlatform,
    }, async ({ page, state }) => {
      await page.getByText("无访问权限", { exact: true }).waitFor()
      assert.equal(state.entries, 0)
      assert.equal(state.contextRequests, 0)
      noBusinessRequests(state)
    })
  }
  for (const suffix of ["/private", "/public/private"]) {
    await check(`micro base and parent menu do not grant unlisted route ${suffix}`, `/micro/guardmicro${suffix}`, {
      microApp: true, menus: [microMenu(), microMenu("/public")],
    }, async ({ page, state }) => {
      await page.getByText("无访问权限", { exact: true }).waitFor()
      assert.equal(state.entries, 0)
      assert.equal(state.contextRequests, 0)
      noBusinessRequests(state)
    })
  }
  await check("micro root redirects to first explicitly authorized menu before entry load", "/micro/guardmicro", {
    microApp: true, menus: [microMenu("/public")],
  }, async ({ page, state }) => {
    await page.waitForURL("**/micro/guardmicro/public")
    await page.getByTestId("guard-micro-mounted").waitFor()
    assert.equal(await page.getByTestId("guard-micro-mounted").innerText(), "/micro/guardmicro/public")
    assert.equal(state.entries, 1)
  })
  for (const [pattern, route] of [["/orders/:id", "/orders/123"], ["/reports/*", "/reports/2026/month"]]) {
    await check(`micro explicit detail pattern ${pattern} permits declared route`, `/micro/guardmicro${route}`, {
      microApp: true, menus: [microMenu(pattern)],
    }, async ({ page, state }) => {
      await page.getByTestId("guard-micro-mounted").waitFor()
      assert.equal(await page.getByTestId("guard-micro-mounted").innerText(), `/micro/guardmicro${route}`)
      assert.equal(state.entries, 1)
    })
  }
  await check("micro parameter grant denies undeclared detail descendants", "/micro/guardmicro/orders/123/private", {
    microApp: true, menus: [microMenu("/orders/:id")],
  }, async ({ page, state }) => {
    await page.getByText("无访问权限", { exact: true }).waitFor()
    assert.equal(state.entries, 0)
    noBusinessRequests(state)
  })
  await check("micro loading menu grants prevents entry loading", "/micro/guardmicro/public", {
    microApp: true, menus: [microMenu("/public")], loadingMenus: true,
  }, async ({ page, state }) => {
    await page.getByRole("status", { name: "正在验证访问权限", exact: true }).waitFor()
    assert.equal(state.entries, 0)
    assert.equal(state.contextRequests, 0)
    state.releaseMenus()
    await page.getByTestId("guard-micro-mounted").waitFor()
    assert.equal(state.entries, 1)
  })
  await check("micro menu errors fail closed before entry loading", "/micro/guardmicro/public", {
    microApp: true, menus: [microMenu("/public")], menusError: true,
  }, async ({ page, state }) => {
    await page.getByText("访问权限验证失败，请重试", { exact: true }).waitFor()
    assert.equal(state.entries, 0)
    assert.equal(state.contextRequests, 0)
    noBusinessRequests(state)
  })
  await check("micro live context rejects cached menu grant before entry loading", "/micro/guardmicro/public", {
    microApp: true, menus: [microMenu("/public")], microContextMenus: [],
  }, async ({ page, state }) => {
    await page.waitForURL("**/dashboard")
    await page.getByText("我的应用", { exact: true }).waitFor()
    assert(state.contextRequests > 0, "The live application context must independently validate menu access")
    assert.equal(state.entries, 0)
  })
  await check("micro live menu revocation unloads running entry", "/micro/guardmicro/public", {
    microApp: true, menus: [microMenu("/public")], microContextTtlMs: 1500,
  }, async ({ page, state }) => {
    await page.getByTestId("guard-micro-mounted").waitFor()
    const before = state.entries
    state.microContextMenus = []
    await page.getByTestId("guard-micro-mounted").waitFor({ state: "hidden" })
    await page.waitForURL("**/dashboard")
    await page.getByText("我的应用", { exact: true }).waitFor()
    assert.equal(state.entries, before)
    assert(state.contextRequests > 1)
  })
  await check("micro navigating to ungranted menu unloads without route update or new entry", "/micro/guardmicro/public", {
    microApp: true, menus: [microMenu("/public")],
  }, async ({ page, state }) => {
    await page.getByTestId("guard-micro-mounted").waitFor()
    const before = state.entries
    await page.evaluate(() => {
      window.history.pushState({}, "", "/micro/guardmicro/private")
      window.dispatchEvent(new PopStateEvent("popstate"))
    })
    await page.getByText("无访问权限", { exact: true }).waitFor()
    assert.equal(await page.getByTestId("guard-micro-mounted").count(), 0)
    assert.equal(state.entries, before)
  })
  await check("header collapses menu and topbar restores it with app rail intact", "/profile", { menus: sidebarMenus, permissions: ["system:organization"] }, async ({ page, state }) => {
    const sidebar = page.getByRole("complementary", { name: "菜单导航", exact: true })
    await sidebar.getByRole("button", { name: "组织机构", exact: true }).waitFor()
    const before = await page.locator("#app-scroll").boundingBox()
    await sidebar.getByRole("button", { name: "收起菜单", exact: true }).click()
    await sidebar.waitFor({ state: "hidden" })
    await page.getByRole("button", { name: "展开菜单", exact: true }).waitFor()
    assert.equal(await page.getByRole("complementary", { name: "应用导航", exact: true }).isVisible(), true)
    const after = await page.locator("#app-scroll").boundingBox()
    assert.equal(Math.round(after.width - before.width), 248, "Collapsed menus give their width back to content")
    await page.getByRole("button", { name: "展开菜单", exact: true }).click()
    await sidebar.waitFor()
    assert.equal(await page.evaluate(() => localStorage.getItem("platform-sidebar")), "open")
    noBusinessRequests(state)
  })
  await check("whole-menu collapse persists across reload", "/profile", { menus: sidebarMenus, permissions: ["system:organization"] }, async ({ page, state }) => {
    await page.getByRole("button", { name: "收起菜单", exact: true }).waitFor()
    await page.getByRole("button", { name: "收起菜单", exact: true }).click()
    await page.getByRole("button", { name: "展开菜单", exact: true }).waitFor()
    await page.reload({ waitUntil: "domcontentloaded" })
    await page.getByRole("button", { name: "展开菜单", exact: true }).waitFor()
    assert.equal(await page.getByRole("complementary", { name: "菜单导航", exact: true }).count(), 0)
    await page.getByRole("button", { name: "展开菜单", exact: true }).click()
    await page.getByRole("complementary", { name: "菜单导航", exact: true }).waitFor()
    noBusinessRequests(state)
  })
  await check("nested directory folds persist through whole collapse, reload and search", "/profile", { menus: sidebarMenus, permissions: ["system:organization"] }, async ({ page, state }) => {
    const sidebar = page.getByRole("complementary", { name: "菜单导航", exact: true })
    const root = sidebar.getByRole("button", { name: "组织机构", exact: true })
    const child = sidebar.getByRole("button", { name: "组织维护", exact: true })
    await child.waitFor()
    await child.click()
    await sidebar.getByRole("button", { name: "组织管理", exact: true }).waitFor({ state: "hidden" })
    await root.click()
    await child.waitFor({ state: "hidden" })
    await sidebar.getByRole("button", { name: "收起菜单", exact: true }).click()
    await page.getByRole("button", { name: "展开菜单", exact: true }).click()
    assert.equal(await root.getAttribute("aria-expanded"), "false")
    await page.reload({ waitUntil: "domcontentloaded" })
    await root.waitFor()
    assert.equal(await root.getAttribute("aria-expanded"), "false")
    await root.click()
    await child.waitFor()
    assert.equal(await child.getAttribute("aria-expanded"), "false")
    await root.click()
    const search = sidebar.getByRole("textbox", { name: "过滤菜单", exact: true })
    await search.fill("组织管理")
    await sidebar.getByRole("button", { name: "组织管理", exact: true }).waitFor()
    assert.equal(await root.getAttribute("aria-expanded"), "true")
    await search.fill("")
    assert.equal(await root.getAttribute("aria-expanded"), "false")
    await child.waitFor({ state: "hidden" })
    noBusinessRequests(state)
  })
  await check("directory preference is isolated by user tenant organization app scope", "/profile", { menus: sidebarMenus, permissions: ["system:organization"] }, async ({ page, state }) => {
    const root = page.getByRole("complementary", { name: "菜单导航", exact: true }).getByRole("button", { name: "组织机构", exact: true })
    await root.waitFor()
    await root.click()
    assert.equal(await root.getAttribute("aria-expanded"), "false")
    state.profile.organization.id = "another-guard-org"
    await page.evaluate(() => window.__permissionAuth.getState().setSession("other-menu-context"))
    await root.waitFor()
    assert.equal(await root.getAttribute("aria-expanded"), "true", "New organization uses its own fold preferences")
    state.profile.organization.id = profile.organization.id
    await page.evaluate(() => window.__permissionAuth.getState().setSession("restored-menu-context"))
    await root.waitFor()
    assert.equal(await root.getAttribute("aria-expanded"), "false", "Original context restores its own fold preferences")
    noBusinessRequests(state)
  })
  await check("mobile drawer works independently of desktop collapse", "/profile", { menus: sidebarMenus, permissions: ["system:organization"],
    viewport: { width: 390, height: 844 } }, async ({ page, state }) => {
    await page.getByRole("button", { name: "打开导航", exact: true }).waitFor()
    await page.evaluate(() => localStorage.setItem("platform-sidebar", "collapsed"))
    await page.reload({ waitUntil: "domcontentloaded" })
    await page.getByRole("button", { name: "打开导航", exact: true }).click()
    const sidebar = page.getByRole("complementary", { name: "菜单导航", exact: true })
    const root = sidebar.getByRole("button", { name: "组织机构", exact: true })
    await root.waitFor()
    assert.equal(await sidebar.getByRole("button", { name: "收起菜单", exact: true }).count(), 0)
    await root.click()
    await page.keyboard.press("Escape")
    await sidebar.waitFor({ state: "hidden" })
    await page.getByRole("button", { name: "打开导航", exact: true }).click()
    await root.waitFor()
    assert.equal(await root.getAttribute("aria-expanded"), "false")
    await page.keyboard.press("Escape")
    await sidebar.waitFor({ state: "hidden" })
    await page.setViewportSize({ width: 1440, height: 1000 })
    await page.getByRole("button", { name: "展开菜单", exact: true }).click()
    await root.waitFor()
    assert.equal(await root.getAttribute("aria-expanded"), "false", "Mobile directory preferences synchronize with desktop")
    noBusinessRequests(state)
  })
  console.log(`Route permission browser checks: ${count} passed`)
} finally {
  if (browser) await browser.close()
  if (vite && vite.exitCode === null) {
    vite.kill()
    await pause(100)
  }
}
