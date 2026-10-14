/**
 * Phase 8/9 integration: the real Vite app, trusted HTML loader, isolated iframe and releases.
 * APIs and the probe micro applications are intercepted in a new browser context.
 * No database, account credentials or running application services are required.
 * Run with Node 24; PLAYWRIGHT_MODULE and BROWSER_EXECUTABLE may override existing runtimes.
 */
import assert from "node:assert/strict"
import { spawn, spawnSync } from "node:child_process"
import { existsSync, readFileSync, mkdtempSync, cpSync, writeFileSync, rmSync } from "node:fs"
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
  icon: "Boxes", basePath: `/micro/${code}`, runtimeMode: "TRUSTED", entryType: "HTML", entryUrl: `/__phase8/${code}/`,
})), ...[
  { code: "vue-workshop", name: "真实 Vue 验收应用", folder: "vue" },
  { code: "legacy-jquery", name: "真实 jQuery 验收应用", folder: "jquery" },
].map(({ code, name, folder }) => ({ id: `application-${code}`, code, name, icon: "Boxes",
  basePath: `/micro/${code}`, runtimeMode: "TRUSTED", entryType: "HTML", entryUrl: `/sub-apps/${folder}/index.html` }))]
// Deliberately absent runtimeMode: the host must default a newly registered app to iframe.
const isolatedApplication = { id: "application-isolated", code: "isolated", name: "隔离验收应用", icon: "Boxes",
  basePath: "/micro/isolated", entryType: "HTML", entryUrl: "/__phase9/isolated/v1/",
  deploymentVersion: "v1", entryVersionId: "release-isolated-v1" }
const managementApplication = { id: 'application-managed', code: 'managed', name: '发布验收应用', icon: 'Boxes',
  basePath: '/system', runtimeMode: 'IFRAME', entryType: 'HTML', entryUrl: '/sub-apps/managed/index.html',
  description: '发布界面交互验收', sortOrder: 1, status: 'ENABLED', menuCount: 1, createdAt: new Date().toISOString() }
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
    sortOrder: 1, visible: true, children: [] },
    { id: `menu-${appCode}-home`, parentId: null, type: "MENU", title: `${label}首页`,
      path: `/micro/${appCode}`, icon: "Boxes", permission: "probe:read",
      sortOrder: 2, visible: true, children: [] }]
}

function releaseManagementFixture({ isPlatform = true, hasPermission = true } = {}) {
  const createdAt = new Date().toISOString()
  const first = { id: 'release-managed-v1', applicationId: managementApplication.id, environment: 'development', version: '1.0.0',
    entryType: 'HTML', entryUrl: '/sub-apps/managed/versions/1.0.0/index.html', healthUrl: null, resources: [],
    validationStatus: 'PASSED', validationResult: { passed: true, checkedAt: createdAt, checks: [] }, checkedAt: createdAt, active: true, createdAt }
  return { isPlatform, hasPermission, requests: [], releases: [first], preflightCount: 0, activationCount: 0,
    deployment: { applicationId: managementApplication.id, environment: 'development', activeReleaseId: first.id,
      previousReleaseId: null, version: first.version, entryType: first.entryType, entryUrl: first.entryUrl, updatedAt: createdAt } }
}

async function handleReleaseManagement(route, state, fulfill) {
  const request = route.request(), url = new URL(request.url()), value = state.management
  if (!value || !url.pathname.startsWith('/api/applications')) return false
  const body = request.postData() ? request.postDataJSON() : undefined
  value.requests.push({ method: request.method(), pathname: url.pathname, environment: url.searchParams.get('environment'), body })
  assert.equal(request.headers().authorization, 'Bearer test-session-A', 'Management UI must use the platform session.')
  if (!value.isPlatform || !value.hasPermission) {
    await fulfill(route, { code: 'FORBIDDEN', message: value.isPlatform ? '无操作权限: system:application' : '仅平台租户可执行跨租户操作' }, 403)
    return true
  }
  const prefix = '/api/applications/' + managementApplication.id
  const setValidation = (release, passed, failure = '关键脚本不存在，请修复静态部署') => {
    const checkedAt = new Date().toISOString()
    release.checkedAt = checkedAt
    release.validationStatus = passed ? 'PASSED' : 'FAILED'
    release.validationResult = { passed, checkedAt, checks: [
      { kind: 'ENTRY', url: release.entryUrl, passed: true, statusCode: 200, contentType: 'text/html', bytes: 2048, message: 'HTML 入口可用' },
      { kind: 'SCRIPT', url: '/sub-apps/managed/versions/2.0.0/app.js', passed, statusCode: passed ? 200 : 404,
        contentType: passed ? 'text/javascript' : 'text/html', bytes: passed ? 3072 : 128, message: passed ? '脚本资源验证通过' : failure },
      { kind: 'STYLE', url: '/sub-apps/managed/versions/2.0.0/styles.css', passed: true, statusCode: 200, contentType: 'text/css', bytes: 1024, message: '样式资源验证通过' },
      { kind: 'HEALTH', url: release.healthUrl, passed: true, statusCode: 200, contentType: 'application/json', bytes: 32, message: '健康检查通过' },
    ] }
  }
  const publish = (next, previous) => {
    value.deployment = { applicationId: managementApplication.id, environment: next.environment, activeReleaseId: next.id,
      previousReleaseId: previous, version: next.version, entryType: next.entryType, entryUrl: next.entryUrl, updatedAt: new Date().toISOString() }
    for (const release of value.releases) release.active = release.id === next.id
    state.applications = state.applications.map((app) => app.id === managementApplication.id ? { ...app,
      deploymentVersion: next.version, entryVersionId: next.id, entryUrl: next.entryUrl } : app)
  }
  if (url.pathname === '/api/applications' && request.method() === 'GET') await fulfill(route, [managementApplication])
  else if (url.pathname === prefix + '/releases' && request.method() === 'GET') {
    await fulfill(route, value.releases.filter((release) => release.environment === url.searchParams.get('environment')))
  } else if (url.pathname === prefix + '/releases' && request.method() === 'POST') {
    const release = { ...body, id: 'release-managed-v2', applicationId: managementApplication.id, healthUrl: body.healthUrl ?? null,
      resources: body.resources ?? [], validationStatus: 'DRAFT', validationResult: {}, checkedAt: null, active: false, createdAt: new Date().toISOString() }
    value.releases.unshift(release)
    await fulfill(route, release)
  } else if (url.pathname === prefix + '/deployments') await fulfill(route, [value.deployment])
  else if (url.pathname === prefix + '/releases/release-managed-v2/preflight') {
    const release = value.releases.find((item) => item.id === 'release-managed-v2')
    setValidation(release, ++value.preflightCount > 1)
    await fulfill(route, release)
  } else if (url.pathname === prefix + '/releases/release-managed-v2/activate') {
    const release = value.releases.find((item) => item.id === 'release-managed-v2')
    if (++value.activationCount === 1) {
      setValidation(release, false, '激活重新预检发现脚本被移除，部署保持上一版本')
      await fulfill(route, { code: 'CONFLICT', message: '重新预检失败，未激活版本' }, 409)
    } else {
      assert.equal(release.validationStatus, 'PASSED', 'UI must preflight again before retrying activation.')
      publish(release, value.deployment.activeReleaseId)
      await fulfill(route, value.deployment)
    }
  } else if (url.pathname === prefix + '/deployments/development/rollback') {
    const previous = value.releases.find((item) => item.id === value.deployment.previousReleaseId)
    assert.ok(previous && previous.validationStatus === 'PASSED')
    publish(previous, value.deployment.activeReleaseId)
    await fulfill(route, value.deployment)
  } else throw new Error('Unexpected management fixture endpoint: ' + request.method() + ' ' + url.pathname)
  return true
}

// Events contain authorization metadata and a token-presence flag; token values are never logged.
function probeHtml(withUpdate, entryLabel = "base") {
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
        theme: props.theme, routePath: props.routePath, appCode: props.appCode,
        deploymentVersion: props.deploymentVersion, entryVersionId: props.entryVersionId, entryLabel: ${JSON.stringify(entryLabel)} };
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
      events.push({ kind: 'unmount-start', instanceId, organizationId: current.organization.id, appCode: current.appCode,
        signalAborted: current.signal.aborted, deploymentVersion: current.deploymentVersion, entryLabel: ${JSON.stringify(entryLabel)}, at: Date.now() });
      return new Promise(function (resolve) { setTimeout(function () {
        if (node) node.replaceChildren();
        events.push({ kind: 'unmount-end', instanceId, organizationId: current.organization.id, appCode: current.appCode, at: Date.now() });
        resolve();
      }, 60); });
    } };
    ${withUpdate ? "lifecycle.update = function (props) { record('update', props); };" : ""}
    window.__MICRO_APP__ = lifecycle;
  })();</script></body></html>`
}

function isolatedHtml(entryLabel) {
  return `<!doctype html><html><head><title>Phase 9 isolated app</title></head><body>
    <main id="app"></main><script src="/__phase9/iframe-bridge.js"></script><script>(function () {
      let node, current, instanceId = crypto.randomUUID();
      let parentBlocked = false, storageBlocked = false;
      try { parent.__phase9HostMarker = 'compromised'; parent.document.body.innerHTML = 'compromised'; } catch (_) { parentBlocked = true; }
      try { parent.localStorage.getItem('platform-token'); localStorage.getItem('platform-token'); } catch (_) { storageBlocked = true; }
      window.__phase9ChildGlobal = 'child only';
      const report = (kind, props) => parent.postMessage({ kind: 'PHASE9_IFRAME_EVENT', event: {
        kind, instanceId, entryLabel: ${JSON.stringify(entryLabel)}, appCode: props.appCode,
        organizationId: props.organization.id, contextEpoch: props.contextEpoch,
        deploymentVersion: props.deploymentVersion, entryVersionId: props.entryVersionId,
        signalAborted: props.signal.aborted, tokenPresent: !!props.token,
        parentBlocked, storageBlocked, theme: props.theme, routePath: props.routePath,
        missing: ['contractVersion','contextVersion','contextEpoch','expiresAt','tokenExpiresAt','signal','token',
          'appCode','basePath','routePath','user','tenant','organization','roles','permissions','dataGrants',
          'menus','theme','onNavigate','onNotify'].filter((key) => !(key in props)),
        hasPlatformSession: 'platformSession' in props || 'clientSecret' in props
      } }, ${JSON.stringify(origin)});
      function receive(props, kind) {
        if (current) window.__phase9OldProps = current;
        current = props; window.__phase9Props = props;
        props.signal.addEventListener('abort', () => report('abort', props), { once: true });
        node.dataset.entry = ${JSON.stringify(entryLabel)};
        node.dataset.epoch = String(props.contextEpoch);
        node.dataset.org = props.organization.id;
        node.dataset.theme = props.theme;
        node.dataset.version = props.deploymentVersion || '';
        node.dataset.entryId = props.entryVersionId || '';
        node.textContent = props.organization.name + ' · ' + ${JSON.stringify(entryLabel)};
        report(kind, props);
      }
      window.__phase9Bridge = MicroIframe.connect({ appCode: 'isolated',
        allowedHostOrigins: [${JSON.stringify(origin)}], container: document.querySelector('#app'), lifecycle: {
          mount: function (container, props) {
            container.innerHTML = '<div id="isolated-state"></div><input id="isolated-input" aria-label="保留输入">';
            node = container.querySelector('#isolated-state'); receive(props, 'mount');
          }, update: function (props) { receive(props, 'update'); },
          unmount: function () { if (current) report('unmount', current); document.querySelector('#app').replaceChildren(); }
        } });
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
  const state = { version: "v1", label: "原菜单", issueCount: 0, contextCount: 0, applications,
    held: null, release: null, holdNextA: false, ...options }
  const leases = new Map()
  await context.addInitScript(() => {
    if (window.top !== window) return
    localStorage.setItem("platform-token", "test-session-A")
    localStorage.setItem("platform-theme", "light")
    window.__phase8Events = []
    window.__phase8Control = { nextId: 0 }
    window.__phase9HostMarker = 'protected-host-instance'
    window.__phase9Events = []
    window.__phase9VueEvents = []
    window.__phase9VueControl = { current: null, nextId: 0 }
    window.addEventListener('message', (event) => {
      if (event.origin === 'null' && event.source !== window && event.data?.kind === 'PHASE9_IFRAME_EVENT') {
        window.__phase9Events.push(event.data.event)
      }
    })
  })
  await context.routeWebSocket("**/*XTransformPort=3003*", (socket) => socket.close())
  await context.route("**/*XTransformPort=3003*", (route) => route.abort())
  await context.route((url) => url.origin === origin && url.pathname === "/src/main.tsx", async (route) => {
    const response = await route.fetch()
    const body = (await response.text()).replace("ReactDOM.createRoot", "window.__phase8QueryClient = queryClient;\nReactDOM.createRoot")
    assert.match(body, /window\.__phase8QueryClient/)
    await route.fulfill({ response, body })
  })
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
    const entryLabel = new URL(route.request().url()).pathname.includes('/v2/') ? 'v2' : 'v1'
    return route.fulfill({ status: 200, contentType: "text/html", body: state.externalScript
      ? externalProbeHtml(!isStatic, state.asyncScript) : probeHtml(!isStatic, entryLabel) })
  })
  await context.route("**/__phase9/**", async (route) => {
    const pathname = new URL(route.request().url()).pathname
    if (pathname.endsWith('/iframe-bridge.js')) return route.fulfill({ status: 200, contentType: 'text/javascript',
      body: readFileSync(path.join(workspace, 'examples/micro-app-sdk/iframe-bridge.js'), 'utf8') })
    if (state.holdIframeEntry && !state.iframeEntryHeld) {
      state.iframeEntryHeld = true; state.iframeEntryRelease = deferred(); await state.iframeEntryRelease.promise
    }
    return route.fulfill({ status: 200, contentType: 'text/html', body: isolatedHtml(pathname.includes('/v2/') ? 'v2' : 'v1') })
      .catch((error) => { if (!/closed|disposed|Invalid Interception|Target|cancelled/i.test(error.message)) throw error })
  })
  if (state.releaseRoot) await context.route('**/sub-apps/vue-workshop/versions/**', (route) => {
    const relative = new URL(route.request().url()).pathname.slice('/sub-apps/'.length)
    const target = path.resolve(state.releaseRoot, ...relative.split('/'))
    assert.ok(target.startsWith(path.resolve(state.releaseRoot) + path.sep), 'Version resource must stay in the test release directory.')
    if (!existsSync(target)) return route.fulfill({ status: 404, body: 'Version resource missing' })
    return route.fulfill({ status: 200, contentType: target.endsWith('.html') ? 'text/html' : 'text/javascript', body: readFileSync(target) })
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
    if (url.pathname === "/api/platform/branding") return fulfill(route, { platformName: "微应用验收平台",
      platformDescription: "隔离浏览器验收", logoUrl: "", loginTitle: "登录", loginDescription: "选择组织", footerText: "" })
    if (await handleReleaseManagement(route, state, fulfill)) return
    if (url.pathname === "/api/__phase8-delayed401") {
      state.pending401 = deferred()
      state.held401 = true
      await state.pending401.promise
      return fulfill(route, { message: "旧上下文会话被撤销", code: "UNAUTHORIZED" }, 401)
    }
    if (url.pathname === "/api/auth/profile") {
      const next = profile(group, group === "B" && state.noAppsB ? [] : state.applications)
      if (state.management) next.tenant.isPlatform = state.management.isPlatform
      return fulfill(route, next)
    }
    if (url.pathname === "/api/auth/menus") {
      if (state.management) return fulfill(route, state.management.hasPermission ? [{ id: 'menu-managed', parentId: null,
        type: 'MENU', title: '应用管理', path: '/system/applications', icon: 'Boxes', sortOrder: 1, visible: true, permission: 'system:application' }] : [])
      const app = state.applications.find((value) => value.id === url.searchParams.get("applicationId")) ?? state.applications[0]
      if (state.holdMenusApp === app.id && !state.heldMenus) {
        state.heldMenus = true
        state.menusRelease = deferred()
        await state.menusRelease.promise
      }
      return fulfill(route, [...menu(app.code, state.label), { id: `dashboard-${app.code}`, parentId: null,
        type: "MENU", title: "工作台", path: "/dashboard", icon: "Gauge", permission: "dashboard:view", sortOrder: 3, visible: true }])
    }
    if (url.pathname === "/api/auth/permissions") return fulfill(route,
      { permissions: state.management ? (state.management.hasPermission ? ['system:application'] : []) : ["probe:read", "dashboard:view"], roles: [] })
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
      return fulfill(route, { accessToken: token, expiresIn: state.tokenExpiresIn ?? 4, tokenType: "Bearer", audience: appCode })
    }
    if (/^\/api\/apps\/[^/]+\/context$/.test(url.pathname)) {
      state.contextCount++
      if (state.failAt === "context") return fulfill(route, { message: "应用授权已撤销", code: "FORBIDDEN" }, state.failStatus)
      const lease = leases.get(bearer)
      assert(lease, "Application context must receive an application token, not the platform session.")
      const currentProfile = profile(lease.group)
      const contextMode = state.contextRuntimeModeOverride || state.applications.find((app) => app.code === lease.appCode)?.runtimeMode || 'IFRAME'
      if (state.downgradeOnContext) state.applications = state.applications.map((app) => app.code === lease.appCode ? { ...app, runtimeMode: 'IFRAME' } : app)
      const response = {
        contractVersion: "1.0", contextVersion: `${lease.group}-${state.version}`,
        expiresAt: new Date(Date.now() + (state.contextTtlMs ?? 3500)).toISOString(), appCode: lease.appCode,
        application: { id: `application-${lease.appCode}`, name: "浏览器验收应用", runtimeMode: contextMode },
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
  const documentResponse = await page.goto(`${origin}${state.startPath || '/micro/probe'}`, { waitUntil: "domcontentloaded" })
  state.documentStatus = documentResponse?.status()
  state.documentType = documentResponse?.headers()["content-type"]
  return { context, page, state, errors, pending, failed, consoleErrors, scripts, close: async () => {
    state.release?.resolve()
    state.scriptRelease?.resolve()
    state.menusRelease?.resolve()
    state.pending401?.resolve()
    state.iframeEntryRelease?.resolve()
    await context.close()
  } }
}

const events = (page) => page.evaluate(() => window.__phase8Events)
const iframeEvents = (page) => page.evaluate(() => window.__phase9Events)
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
      iframeEvents: await iframeEvents(test.page),
      managementRequests: test.state.management?.requests,
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
    assert.deepEqual(menuUpdate.menuTitles, [state.label, `${state.label}首页`])
    assert.equal(menuUpdate.menuTitles.length, initial.menuTitles.length)
    await waitEvent(page, { kind: "update", tokenChanged: true })
    const beforeSwitch = await events(page)
    assert.equal(beforeSwitch.filter((entry) => entry.kind === "mount").length, 1, "All ordinary changes must use update.")
    assert.equal(beforeSwitch.filter((entry) => entry.kind === "unmount-start").length, 0)

    state.holdNextA = true
    await waitState(() => state.held, "An old application context response must be in flight before switching.")
    await page.evaluate(async () => {
      const { request } = await import("/src/lib/api.ts")
      window.__phase8Control.old401Result = request("/api/__phase8-delayed401").catch((error) => error.name)
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
    assert.equal(await page.evaluate(() => window.__phase8Control.old401Result), "AbortError",
      "A late response from the prior organization is rejected before it can handle its 401.")
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
    // Route guards now prefetch granted menus, so deliberately expire this cache
    // to keep the late-response race meaningful without waiting for a timer.
    await page.evaluate(() => window.__phase8QueryClient.invalidateQueries({
      queryKey: ["auth-menus", "application-probe-static"], refetchType: "none",
    }))
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
      assert.equal(await page.evaluate(() => localStorage.getItem("platform-token") !== null), true,
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
    await page.getByText("无访问权限", { exact: true }).waitFor()
    assert.equal(await page.locator('[data-testid="phase8-probe"]').count(), 0)
    console.log("PASS real host: unavailable application after organization switch denies before mounting")
  })
}

const waitIframeEvent = (page, criteria) => page.waitForFunction((expected) =>
  window.__phase9Events.some((entry) => Object.entries(expected).every(([key, value]) => entry[key] === value)), criteria)

async function refreshProfile(page) {
  return page.evaluate(async () => {
    const { authApi } = await import('/src/api/auth.ts')
    const { useAuthStore } = await import('/src/stores/auth.ts')
    const before = useAuthStore.getState()
    const next = await authApi.profile(before.token)
    useAuthStore.getState().setProfile(next, before.contextEpoch)
    return { before: before.contextEpoch, after: useAuthStore.getState().contextEpoch }
  })
}

async function verifyIsolatedDefault() {
  await withFixture({ applications: [...applications, isolatedApplication], startPath: '/micro/isolated' }, async ({ page, state }) => {
    await waitIframeEvent(page, { kind: 'mount', entryLabel: 'v1', organizationId: 'org-A' })
    const initial = (await iframeEvents(page)).find((event) => event.kind === 'mount')
    assert.equal(initial.parentBlocked, true)
    assert.equal(initial.storageBlocked, true)
    assert.equal(initial.hasPlatformSession, false)
    assert.equal(initial.tokenPresent, true)
    assert.deepEqual(initial.missing, [])
    assert.equal(initial.deploymentVersion, 'v1')
    assert.equal(initial.entryVersionId, 'release-isolated-v1')
    const iframe = page.locator('.micro-mount-zone iframe')
    assert.equal(await iframe.getAttribute('sandbox'), 'allow-scripts allow-forms')
    assert.equal(await page.evaluate(() => window.__phase9HostMarker), 'protected-host-instance')
    assert.equal(await page.evaluate(() => window.__phase9ChildGlobal), undefined)
    assert.equal((await events(page)).length, 0, 'Isolated scripts cannot execute the trusted host lifecycle fixture.')
    await page.evaluate(() => { window.__phase9OriginalFrame = document.querySelector('.micro-mount-zone iframe') })
    const inner = page.frameLocator('.micro-mount-zone iframe')
    await inner.locator('#isolated-input').fill('keep through renewal')
    await page.getByRole('button', { name: '切换到深色模式', exact: true }).click()
    await waitIframeEvent(page, { kind: 'update', theme: 'dark' })
    await inner.locator('#isolated-state[data-theme="dark"]').waitFor()
    state.version = 'iframe-v2'
    await page.waitForFunction(() => window.__phase9Events.some((event) => event.kind === 'update' && event.contextEpoch > window.__phase9Events[0].contextEpoch))
    assert.equal(await inner.locator('#isolated-input').inputValue(), 'keep through renewal')
    assert.equal(await page.evaluate(() => window.__phase9OriginalFrame === document.querySelector('.micro-mount-zone iframe')), true)
    const frame = page.frames().find((candidate) => candidate.url().includes('/__phase9/isolated/'))
    await frame.evaluate(() => {
      window.__phase9OldProps.onNavigate('/micro/probe')
      window.__phase9OldProps.onNotify('old iframe callback must be ignored', 'error')
      window.__phase9Props.onNavigate('/micro/probe')
    })
    await pause(100)
    assert.equal(new URL(page.url()).pathname, '/micro/isolated', 'Iframe callbacks cannot navigate to another application.')
    assert.equal(await page.getByText('old iframe callback must be ignored', { exact: true }).count(), 0)
    await frame.evaluate(() => window.__phase9Props.onNavigate('detail'))
    await page.waitForURL('**/micro/isolated/detail')
    await waitIframeEvent(page, { kind: 'update', routePath: '/micro/isolated/detail' })
    await page.evaluate(async () => {
      const { useAuthStore } = await import('/src/stores/auth.ts')
      useAuthStore.getState().beginContextSwitch()
    })
    await page.waitForFunction(() => !window.__phase9OriginalFrame.isConnected)
    await page.evaluate(async () => {
      const { useAuthStore } = await import('/src/stores/auth.ts')
      useAuthStore.getState().setToken('test-session-B')
    })
    await waitIframeEvent(page, { kind: 'mount', organizationId: 'org-B' })
    assert.equal(await page.locator('.micro-mount-zone iframe').count(), 1)
    assert.equal(await page.frameLocator('.micro-mount-zone iframe').locator('#isolated-state').getAttribute('data-org'), 'org-B')
    console.log('PASS real host: omitted runtimeMode defaults to opaque iframe, complete props, renewal state, bounded callbacks and immediate organization teardown')
  })
  await withFixture({ applications: [...applications, isolatedApplication], startPath: '/micro/isolated', holdIframeEntry: true }, async ({ page, state }) => {
    await waitState(() => state.iframeEntryHeld, 'Iframe entry must be delayed before organization replacement.')
    await page.evaluate(async () => {
      const { useAuthStore } = await import('/src/stores/auth.ts')
      useAuthStore.getState().beginContextSwitch()
      useAuthStore.getState().setToken('test-session-B')
    })
    await waitIframeEvent(page, { kind: 'mount', organizationId: 'org-B' })
    state.iframeEntryRelease.resolve(); await pause(100)
    assert(!(await iframeEvents(page)).some((event) => event.kind === 'mount' && event.organizationId === 'org-A'))
    assert.equal(await page.locator('.micro-mount-zone iframe').count(), 1)
    console.log('PASS real host: pending iframe load/handshake cancels without blocking replacement organization')
  })
  await withFixture({ applications: [...applications, { ...isolatedApplication, runtimeMode: 'TRUSTED' }], startPath: '/micro/isolated',
    contextRuntimeModeOverride: 'IFRAME', downgradeOnContext: true }, async ({ page }) => {
    await waitIframeEvent(page, { kind: 'mount', entryLabel: 'v1' })
    assert.equal(await page.evaluate(() => window.__phase9HostMarker), 'protected-host-instance')
    assert.equal(await page.evaluate(() => window.__phase9ChildGlobal), undefined)
    assert.equal(await page.locator('.micro-mount-zone iframe').count(), 1)
    assert.equal(await page.evaluate(async () => {
      const { useAuthStore } = await import('/src/stores/auth.ts')
      return useAuthStore.getState().profile.applications.find((app) => app.code === 'isolated').runtimeMode
    }), 'IFRAME')
    assert.equal((await events(page)).length, 0)
    console.log('PASS real host: stale TRUSTED profile cannot execute host scripts when latest application context requires IFRAME')
  })
  await withFixture({ applications: [...applications, { ...isolatedApplication, entryType: 'ESM' }], startPath: '/micro/isolated' }, async ({ page }) => {
    await page.getByText('隔离 iframe 应用需要 HTML 入口，请联系平台管理员配置', { exact: true }).first().waitFor()
    assert.equal(await page.locator('.micro-mount-zone iframe').count(), 0)
    assert.equal(await page.evaluate(() => window.__phase9HostMarker), 'protected-host-instance')
    assert.equal(await page.evaluate(() => window.__phase9ChildGlobal), undefined)
    assert.equal((await iframeEvents(page)).length, 0)
    console.log('PASS real host: default isolated ESM is rejected before executing any entry code')
  })
}

function buildVueReleases() {
  const temp = mkdtempSync(path.join(os.tmpdir(), 'platform-micro-host-releases-'))
  const output = path.join(temp, 'releases')
  try {
    for (const version of ['1.0.0', '1.0.1']) {
      const input = path.join(temp, 'build-' + version)
      cpSync(path.join(workspace, 'mini-services/micro-vue/public'), input, { recursive: true })
      const appPath = path.join(input, 'app.js')
      const original = readFileSync(appPath, 'utf8')
      assert.ok(original.includes('微前端子应用 · HTML 入口 + 沙箱'), 'Real Vue header source must exist before preparing new version bytes.')
      const instrument = `\n;(function () {
        var original = window.__MICRO_APP__, current, instanceId;
        var events = window.__phase9VueEvents, control = window.__phase9VueControl;
        function record(kind, props) { events.push({ kind: kind, instanceId: instanceId,
          resourceVersion: ${JSON.stringify(version)}, deploymentVersion: props.deploymentVersion,
          entryVersionId: props.entryVersionId, signalAborted: props.signal.aborted, contextEpoch: props.contextEpoch }); }
        window.__MICRO_APP__ = { name: original.name,
          mount: async function (container, props) { current = props; control.current = props; instanceId = ++control.nextId;
            await original.mount(container, props); record('mount', props); },
          update: async function (props) { current = props; control.current = props; await original.update(props); record('update', props); },
          unmount: async function () { record('unmount-start', current); await original.unmount(); record('unmount-end', current); }
        };
      })();\n`
      writeFileSync(appPath, original.replace('微前端子应用 · HTML 入口 + 沙箱', 'Vue 独立发布 ' + version) + instrument)
      const packed = spawnSync(process.execPath, [path.join(workspace, 'scripts/package-microapp.mjs'), '--app', 'vue-workshop',
        '--environment', 'development', '--version', version, '--input', input, '--output-root', output],
      { cwd: workspace, windowsHide: true, encoding: 'utf8' })
      assert.equal(packed.status, 0, 'Real independent Vue release packaging failed: ' + packed.stderr)
      const manifest = JSON.parse(readFileSync(path.join(output, 'vue-workshop/versions', version, 'release.json'), 'utf8'))
      assert.equal(manifest.entryUrl, '/sub-apps/vue-workshop/versions/' + version + '/index.html')
      assert.ok(manifest.resources.some((resource) => resource.url === 'vendor/vue.global.prod.js'))
    }
    assert.notEqual(readFileSync(path.join(output, 'vue-workshop/versions/1.0.0/app.js'), 'utf8'),
      readFileSync(path.join(output, 'vue-workshop/versions/1.0.1/app.js'), 'utf8'), 'Two packaged Vue versions must have different resource bytes.')
    return { output, cleanup: () => cleanupReleaseTemp(temp) }
  } catch (error) { cleanupReleaseTemp(temp); throw error }
}

function cleanupReleaseTemp(temp) {
  const absolute = path.resolve(temp), parent = path.resolve(os.tmpdir())
  assert.ok(absolute.startsWith(parent + path.sep) && path.basename(absolute).startsWith('platform-micro-host-releases-'), 'Only the verified test temporary directory may be removed.')
  rmSync(absolute, { recursive: true, force: true })
}

async function verifyIndependentVueRelease() {
  const releases = buildVueReleases()
  const versioned = applications.map((app) => app.code === 'vue-workshop' ? { ...app,
    entryUrl: '/sub-apps/vue-workshop/versions/1.0.0/index.html', deploymentVersion: '1.0.0', entryVersionId: 'release-vue-1.0.0' } : app)
  try {
    await withFixture({ applications: versioned, startPath: '/micro/vue-workshop', releaseRoot: releases.output,
      tokenExpiresIn: 60, contextTtlMs: 60_000 }, async ({ page, state }) => {
      const zone = page.locator('.micro-mount-zone')
      await zone.getByText('Vue 独立发布 1.0.0', { exact: true }).waitFor()
      await page.getByTestId('micro-status-bar').getByText('运行中', { exact: true }).waitFor()
      await page.evaluate(() => {
        window.__phase9HostRoot = document.querySelector('#root')
        window.__phase9VueRetired = []
        window.__phase9VueInitial = document.querySelector('.mf-vue-root')
      })
      const issued = state.issueCount
      for (const version of ['1.0.1', '1.0.0']) {
        await page.evaluate(() => window.__phase9VueRetired.push(window.__phase9VueControl.current))
        state.applications = state.applications.map((app) => app.code === 'vue-workshop' ? { ...app,
          entryUrl: '/sub-apps/vue-workshop/versions/' + version + '/index.html', deploymentVersion: version,
          entryVersionId: 'release-vue-' + version } : app)
        const epoch = await refreshProfile(page)
        assert.equal(epoch.before, epoch.after)
        await page.waitForFunction(() => window.__phase9VueRetired.at(-1).signal.aborted)
        await zone.getByText('Vue 独立发布 ' + version, { exact: true }).waitFor()
        await page.getByTestId('micro-status-bar').getByText('运行中', { exact: true }).waitFor()
        assert.equal(await page.evaluate(() => window.__phase9HostRoot === document.querySelector('#root') && window.__phase9HostMarker === 'protected-host-instance'), true)
        assert.equal(state.issueCount, issued)
        await page.evaluate(() => { const retired = window.__phase9VueRetired.at(-1)
          retired.onNavigate('/dashboard'); retired.onNotify('retired Vue release callback', 'error') })
        await pause(100)
        assert.equal(new URL(page.url()).pathname, '/micro/vue-workshop')
        assert.equal(await page.getByText('retired Vue release callback', { exact: true }).count(), 0)
        await zone.getByRole('button', { name: 'GET /api/external/menus', exact: true }).click()
        await zone.getByText('HTTP 200', { exact: false }).first().waitFor()
      }
      const history = await page.evaluate(() => window.__phase9VueEvents)
      assert.deepEqual(history.filter((event) => event.kind === 'mount').map((event) => event.resourceVersion), ['1.0.0', '1.0.1', '1.0.0'])
      assert.deepEqual(history.filter((event) => event.kind === 'mount').map((event) => event.entryVersionId), ['release-vue-1.0.0', 'release-vue-1.0.1', 'release-vue-1.0.0'])
      assert.ok(history.filter((event) => event.kind === 'unmount-start').every((event) => event.signalAborted))
      assert.equal(await page.locator('.mf-vue-root').count(), 1)
      assert.equal(await page.evaluate(() => window.__phase9VueInitial === document.querySelector('.mf-vue-root')), false)
      console.log('PASS real independent Vue: actual packaged vendor/resources 1.0.0 -> new 1.0.1 -> rollback 1.0.0, APIs work, retired signals/callbacks cancel, host document/token survive')
    })
  } finally { releases.cleanup() }
}

async function verifyDeploymentEntryReplacement() {
  const versioned = applications.map((app) => app.code === 'probe' ? { ...app, deploymentVersion: 'v1', entryVersionId: 'release-probe-v1' } : app)
  await withFixture({ applications: versioned, tokenExpiresIn: 60, contextTtlMs: 60_000 }, async ({ page, state }) => {
    await waitEvent(page, { kind: 'mount', deploymentVersion: 'v1', entryLabel: 'v1' })
    await page.evaluate(() => {
      window.__phase9HostRoot = document.querySelector('#root')
      window.__phase9EntrySignals = [window.__phase8Control.signal]
      window.__phase9InitialProbe = document.querySelector('[data-testid="phase8-probe"]')
    })
    const initialIssueCount = state.issueCount
    for (const version of ['v2', 'v1']) {
      state.applications = state.applications.map((app) => app.code === 'probe' ? { ...app,
        entryUrl: version === 'v2' ? '/__phase8/probe/v2/' : '/__phase8/probe/',
        deploymentVersion: version, entryVersionId: 'release-probe-' + version } : app)
      const epoch = await refreshProfile(page)
      assert.equal(epoch.before, epoch.after, 'Deployment activation/rollback must preserve platform authorization epoch.')
      await page.waitForFunction(() => window.__phase9EntrySignals.at(-1).aborted)
      await waitEvent(page, { kind: 'mount', deploymentVersion: version, entryLabel: version })
      await page.getByTestId('micro-status-bar').getByText('运行中', { exact: true }).waitFor()
      assert.equal(await page.evaluate(() => window.__phase9HostRoot === document.querySelector('#root') && window.__phase9HostMarker === 'protected-host-instance'), true,
        'A changed active entry replaces only the micro application; the running host document remains.')
      assert.equal(state.issueCount, initialIssueCount, 'Changing a release must retain the valid application-token pipeline.')
      await page.evaluate(() => window.__phase9EntrySignals.push(window.__phase8Control.signal))
    }
    const history = (await events(page)).filter((event) => event.appCode === 'probe')
    const mounts = history.filter((event) => event.kind === 'mount')
    assert.deepEqual(mounts.map((event) => event.deploymentVersion), ['v1', 'v2', 'v1'])
    assert.deepEqual(mounts.map((event) => event.entryVersionId), ['release-probe-v1', 'release-probe-v2', 'release-probe-v1'])
    for (const mount of mounts.slice(0, 2)) {
      const retired = history.find((event) => event.kind === 'unmount-start' && event.instanceId === mount.instanceId)
      assert.equal(retired?.signalAborted, true, 'Every retired entry props.signal is cancelled before unmount.')
      const end = history.findIndex((event) => event.kind === 'unmount-end' && event.instanceId === mount.instanceId)
      const next = history.findIndex((event) => event.kind === 'mount' && event.instanceId > mount.instanceId)
      assert.ok(end >= 0 && end < next, 'Replacement entry waits for asynchronous old lifecycle cleanup.')
    }
    assert.equal(await page.evaluate(() => window.__phase9InitialProbe === document.querySelector('[data-testid="phase8-probe"]')), false)
    console.log('PASS real host: activate v2 / rollback v1 replaces immutable entries, cancels old props, preserves host and application-token pipeline')
  })
  await withFixture({ applications: [...applications, isolatedApplication], startPath: '/micro/isolated', tokenExpiresIn: 60, contextTtlMs: 60_000 }, async ({ page, state }) => {
    await waitIframeEvent(page, { kind: 'mount', deploymentVersion: 'v1' })
    await page.evaluate(() => { window.__phase9HostRoot = document.querySelector('#root'); window.__phase9ReleaseFrames = [document.querySelector('.micro-mount-zone iframe')] })
    const issues = state.issueCount
    for (const version of ['v2', 'v1']) {
      state.applications = state.applications.map((app) => app.code === 'isolated' ? { ...app,
        entryUrl: '/__phase9/isolated/' + version + '/', deploymentVersion: version, entryVersionId: 'release-isolated-' + version } : app)
      const epoch = await refreshProfile(page)
      assert.equal(epoch.before, epoch.after)
      await page.waitForFunction(() => !window.__phase9ReleaseFrames.at(-1).isConnected)
      await page.frameLocator('.micro-mount-zone iframe').locator('#isolated-state[data-version="' + version + '"]').waitFor()
      assert.equal(await page.evaluate(() => window.__phase9HostRoot === document.querySelector('#root') && window.__phase9HostMarker === 'protected-host-instance'), true)
      assert.equal(state.issueCount, issues)
      await page.evaluate(() => window.__phase9ReleaseFrames.push(document.querySelector('.micro-mount-zone iframe')))
    }
    const mounts = (await iframeEvents(page)).filter((event) => event.kind === 'mount')
    assert.deepEqual(mounts.map((event) => event.entryLabel), ['v1', 'v2', 'v1'])
    assert.deepEqual(mounts.map((event) => event.entryVersionId), ['release-isolated-v1', 'release-isolated-v2', 'release-isolated-v1'])
    assert.equal(await page.locator('.micro-mount-zone iframe').count(), 1)
    console.log('PASS real host: isolated activate/rollback destroys retired documents and channels while the host stays running')
  })
  await verifyIndependentVueRelease()
}

async function verifyReleaseManagementUI() {
  const management = releaseManagementFixture()
  await withFixture({ management, applications: [managementApplication], startPath: '/system/applications' }, async ({ page, state }) => {
    await page.getByRole('heading', { name: '应用管理', exact: true }).waitFor()
    await page.getByRole('button', { name: '发布版本', exact: true }).click()
    const panel = page.getByRole('dialog', { name: '发布验收应用 · 发布版本', exact: true })
    await panel.waitFor()
    const deployment = panel.getByRole('region', { name: '当前环境部署', exact: true })
    await deployment.getByText('1.0.0', { exact: true }).waitFor()
    assert.equal(await deployment.getByRole('button', { name: /回滚/ }).count(), 0)
    await panel.getByRole('button', { name: '登记版本', exact: true }).click()
    const register = panel.getByRole('region', { name: '登记新版本', exact: true })
    await register.getByLabel(/版本号/).fill('2.0.0')
    await register.getByLabel(/版本入口/).fill('/sub-apps/managed/index.html')
    await register.getByRole('button', { name: '登记版本', exact: true }).click()
    await register.getByText('入口路径需包含版本号的完整目录，如 /versions/1.0.0/index.html', { exact: true }).waitFor()
    assert.equal(management.requests.filter((request) => request.method === 'POST').length, 0,
      'Invalid mutable entry address must be rejected before calling registration API.')
    await register.getByLabel(/版本入口/).fill('/sub-apps/managed/versions/2.0.0/index.html')
    await register.getByLabel('健康检查地址', { exact: true }).fill('/sub-apps/managed/health.json')
    await register.getByRole('button', { name: '添加资源', exact: true }).click()
    await register.getByLabel('资源 1 地址', { exact: true }).fill('app.js')
    await register.getByRole('button', { name: '添加资源', exact: true }).click()
    await register.getByLabel('资源 2 类型', { exact: true }).click()
    await page.getByRole('option', { name: '样式', exact: true }).click()
    await register.getByLabel('资源 2 地址', { exact: true }).fill('styles.css')
    await register.getByRole('button', { name: '登记版本', exact: true }).click()
    await page.getByText('版本已登记', { exact: true }).waitFor()
    const second = panel.locator('article').filter({ has: page.getByRole('heading', { name: '2.0.0', exact: true }) })
    await second.getByText('待预检', { exact: true }).waitFor()
    assert.equal(await second.getByRole('button', { name: '激活', exact: true }).isDisabled(), true)
    const registered = management.requests.find((request) => request.method === 'POST' && request.pathname.endsWith('/releases'))
    assert.deepEqual(registered.body, { environment: 'development', version: '2.0.0', entryType: 'HTML',
      entryUrl: '/sub-apps/managed/versions/2.0.0/index.html', healthUrl: '/sub-apps/managed/health.json',
      resources: [{ kind: 'SCRIPT', url: 'app.js' }, { kind: 'STYLE', url: 'styles.css' }] })

    await second.getByRole('button', { name: '预检', exact: true }).click()
    await second.getByText('预检失败', { exact: true }).waitFor()
    await second.getByText('关键脚本不存在，请修复静态部署', { exact: true }).waitFor()
    assert.notEqual(await second.locator('details').getAttribute('open'), null, 'Failed resource diagnostics should expand automatically.')
    assert.equal(await second.getByText('HTTP 404', { exact: true }).count(), 1)
    assert.equal(await second.getByText('text/html', { exact: true }).count(), 2)
    assert.equal(await second.getByRole('button', { name: '激活', exact: true }).isDisabled(), true)
    assert.equal(management.requests.filter((request) => request.pathname.endsWith('/activate')).length, 0)
    await second.getByRole('button', { name: '重新预检', exact: true }).click()
    await second.getByText('预检通过', { exact: true }).waitFor()
    await second.getByRole('button', { name: '激活', exact: true }).click()
    const confirm = page.getByRole('alertdialog')
    await confirm.getByRole('heading', { name: '确认激活版本', exact: true }).waitFor()
    const rejectedActivation = page.waitForResponse((response) => response.url().endsWith('/releases/release-managed-v2/activate') && response.request().method() === 'POST')
    await confirm.getByRole('button', { name: '确认激活', exact: true }).click()
    assert.equal((await rejectedActivation).status(), 409, 'Changed deployment resources use the backend activation conflict status.')
    await page.getByText('重新预检失败，未激活版本', { exact: true }).waitFor()
    await second.getByText('激活重新预检发现脚本被移除，部署保持上一版本', { exact: true }).waitFor()
    assert.equal(await second.getByRole('button', { name: '激活', exact: true }).isDisabled(), true)
    assert.equal(await deployment.getByText('1.0.0', { exact: true }).count(), 1, 'Rejected activation must preserve the previous UI deployment.')
    assert.equal(management.deployment.version, '1.0.0')
    await second.getByRole('button', { name: '重新预检', exact: true }).click()
    await second.getByText('预检通过', { exact: true }).waitFor()
    await second.getByRole('button', { name: '激活', exact: true }).click()
    await page.getByRole('alertdialog').getByRole('button', { name: '确认激活', exact: true }).click()
    await page.getByText('已激活版本 2.0.0', { exact: true }).waitFor()
    await deployment.getByText('2.0.0', { exact: true }).waitFor()
    await second.getByRole('button', { name: '已激活', exact: true }).waitFor()
    assert.equal(management.deployment.previousReleaseId, 'release-managed-v1')
    await deployment.getByRole('button', { name: '回滚至 1.0.0', exact: true }).click()
    await page.getByRole('alertdialog').getByRole('heading', { name: '确认回滚部署', exact: true }).waitFor()
    await page.getByRole('alertdialog').getByRole('button', { name: '确认回滚', exact: true }).click()
    await page.getByText('已回滚至版本 1.0.0', { exact: true }).waitFor()
    await deployment.getByText('1.0.0', { exact: true }).waitFor()
    assert.equal(management.deployment.activeReleaseId, 'release-managed-v1')
    const requests = management.requests.filter((request) => request.method === 'POST')
    assert.deepEqual(requests.map((request) => request.pathname.split('/').slice(-1)[0]),
      ['releases', 'preflight', 'preflight', 'activate', 'preflight', 'activate', 'rollback'])
    assert.ok(requests.slice(1).every((request) => request.body === undefined), 'Preflight/activate/rollback identify their targets by URL without a fabricated request payload.')
    assert.equal(requests.at(-1).pathname, '/api/applications/application-managed/deployments/development/rollback')
    assert.ok(management.requests.some((request) => request.method === 'GET' && request.pathname.endsWith('/releases') && request.environment === 'development'))
    assert.equal(state.issueCount, 0, 'Management API interactions must not issue unrelated micro application tokens.')
    console.log('PASS real release management UI: immutable registration body, DRAFT/FAILED gates, resource diagnostics, activation recheck error, PASSED activate v2 and rollback v1')
  })

  for (const access of [{ isPlatform: false, hasPermission: true }, { isPlatform: true, hasPermission: false }]) {
    const denied = releaseManagementFixture(access)
    await withFixture({ management: denied, applications: [managementApplication], startPath: '/system/applications' }, async ({ page }) => {
      const message = access.isPlatform ? '无操作权限: system:application' : '仅平台租户可执行跨租户操作'
      if (access.hasPermission) await page.getByRole('alert').getByText(message, { exact: true }).waitFor()
      else {
        await page.getByText('无访问权限', { exact: true }).waitFor()
        assert.deepEqual(denied.requests, [], 'Denied routes must not mount application management or fetch any business data.')
      }
      assert.equal(await page.getByRole('button', { name: '发布版本', exact: true }).count(), 0)
      assert.equal(denied.requests.some((request) => request.pathname.includes('/releases') || request.pathname.includes('/deployments')), false)
      assert.equal(await page.evaluate(() => localStorage.getItem('platform-token') !== null), true)
      if (!access.isPlatform) {
        await page.getByRole('button', { name: '新建微应用', exact: true }).first().click()
        const dialog = page.getByRole('dialog')
        assert.equal(await dialog.getByLabel('运行方式', { exact: true }).isDisabled(), true)
        assert.match(await dialog.getByLabel('运行方式', { exact: true }).innerText(), /默认隔离/)
      }
      console.log(`PASS real release management permission UI: ${access.isPlatform ? 'platform without system:application' : 'tenant administrator'} gets 403 and no release controls or requests`)
    })
  }
}

try {
  await startServer()
  browser = await chromium.launch({ headless: true, executablePath: browserExecutable, args: ["--no-proxy-server"] })
  const groups = { updates: verifyUpdatesAndSwitch, remount: verifySerialRemount,
    selection: verifyApplicationSelectionRace, scripts: verifyCancelledHtmlScripts,
    vendors: verifyRealVueAndJQuery, failures: verifyAuthorizationFailures,
    iframe: verifyIsolatedDefault, releases: verifyDeploymentEntryReplacement, management: verifyReleaseManagementUI }
  const selected = process.argv[2]
  assert(!selected || selected in groups, "Unknown browser scenario group.")
  for (const [name, run] of Object.entries(groups)) if (!selected || selected === name) await run()
  console.log(`Phase 8/9 browser integration passed (${selected || "all 21 scenarios"}); APIs and credentials were isolated.`)
} finally {
  await browser?.close()
  if (vite && vite.exitCode === null) {
    vite.kill()
    await Promise.race([new Promise((resolve) => vite.once("exit", resolve)), pause(5000)])
    if (vite.exitCode === null) vite.kill("SIGKILL")
  }
}
