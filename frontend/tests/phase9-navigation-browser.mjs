/** Actual React navigation and authorization queries, with isolated HTTP fixtures. */
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { spawn } from 'node:child_process'
import { readFileSync } from 'node:fs'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { launchBrowser } from './browser/runtime.mjs'

const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const bindHost = process.env.PHASE9_BROWSER_BIND || (process.platform === 'win32' ? '::1' : '127.0.0.1')
const pause = ms => new Promise(resolve => setTimeout(resolve, ms))
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done }); return { promise, resolve } }
async function waitState(predicate, message, timeout = 12_000) {
  const deadline = Date.now() + timeout
  while (!await predicate() && Date.now() < deadline) await pause(20)
  assert(await predicate(), message)
}
const menu = (id, title, pathname, permission = null, extra = {}) => ({ id, parentId: null,
  type: 'MENU', title, path: pathname, icon: 'Boxes', permission, sortOrder: 1, visible: true, children: [], ...extra })
const system = { id: 'nav-system', code: 'system', name: '系统管理', icon: 'LayoutGrid', basePath: '/system' }
const app = (suffix) => ({ id: 'nav-app-' + suffix, code: 'nav-' + suffix, name: '导航应用 ' + suffix.toUpperCase(),
  icon: 'Boxes', basePath: '/micro/nav-' + suffix, runtimeMode: 'TRUSTED', entryType: 'HTML',
  entryUrl: '/__phase9/' + suffix + '/index.html', deploymentVersion: 'phase9', entryVersionId: 'nav-release-' + suffix })
const appA = app('a'), appB = app('b')
const appMenu = (application, suffix = '/home', title = application.name + '入口') =>
  menu(application.code + suffix, title, application.basePath + suffix, application.code + ':read')
const user = { id: 'nav-user', username: 'navigation', realName: '导航验收用户', email: 'nav@example.test',
  phone: '', gender: 'UNKNOWN', avatar: '', isSuperAdmin: true }
const profile = (apps = [system, appA, appB], org = 'org-A', platformContext = true) => ({ user,
  tenant: { id: 'nav-tenant', name: '导航租户', code: 'navigation', isPlatform: true }, platformContext,
  organization: { id: org, name: '导航组织 ' + org, path: org }, applications: apps,
  memberships: [{ userId: user.id, tenantId: 'nav-tenant', tenantName: '导航租户', organizationId: org,
    organizationName: '导航组织 ' + org, organizationPath: org }] })
const tenantRow = { id: 'nav-visible-tenant', name: '明确授权页面业务行', code: 'nav-visible', tenantType: 'COMPANY',
  status: 'ENABLED', isPlatform: false, organizationCount: 1, userCount: 1, authorizedAppCount: 1,
  createdAt: '2026-10-15T00:00:00Z', expiresAt: null }
let browser, vite, origin, startup = '', count = 0
async function startServer() {
  const port = await new Promise((resolve, reject) => {
    const server = net.createServer().on('error', reject)
    server.listen(0, bindHost, () => { const port = server.address().port; server.close(error => error ? reject(error) : resolve(port)) })
  })
  origin = `http://${bindHost.includes(':') ? '[' + bindHost + ']' : bindHost}:${port}`
  const args = [path.join(workspace, 'node_modules/vite/bin/vite.js'), '--host', bindHost, '--port', String(port), '--strictPort']
  if (process.env.PHASE9_VITE_CONFIG) args.push('--config', path.resolve(process.env.PHASE9_VITE_CONFIG))
  vite = spawn(process.execPath, args, { cwd: workspace, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
  let spawnError
  vite.on('error', error => { spawnError = error })
  vite.stdout.on('data', chunk => { startup = (startup + chunk).slice(-6000) })
  vite.stderr.on('data', chunk => { startup = (startup + chunk).slice(-6000) })
  await waitState(() => {
    if (spawnError) throw spawnError
    if (vite.exitCode !== null) throw new Error('Vite stopped: ' + startup)
    return startup.includes('ready in')
  }, 'Vite did not become ready', 30_000)
}

// Observe actual native requests. Only an explicitly armed old-context menu
// request detaches native abort: production still receives its real signal and
// must reject the actual late HTTP response before putting it in cache or UI.
function observeTransport(detachMenus) {
  const nativeFetch = window.fetch.bind(window)
  const state = window.__phase9Transport = { events: [], history: [] }
  for (const method of ['pushState', 'replaceState']) {
    const original = history[method].bind(history)
    history[method] = function (...args) { const result = original(...args); state.history.push({ method, path: location.pathname }); return result }
  }
  window.fetch = function (input, options = {}) {
    const url = new URL(typeof input === 'string' ? input : input.url, location.href)
    if (url.pathname !== '/api/auth/menus') return nativeFetch(input, options)
    const signal = options.signal
    const detached = detachMenus && options.headers?.Authorization === 'Bearer phase9-session-A'
    const event = { id: state.events.length + 1, appId: url.searchParams.get('applicationId'),
      signalPresent: Boolean(signal), aborted: Boolean(signal?.aborted), detached, completed: false }
    state.events.push(event)
    signal?.addEventListener('abort', () => { event.aborted = true; event.cancelledBeforeResponse = !event.completed }, { once: true })
    return nativeFetch(input, detached ? { ...options, signal: undefined } : options).then(response => {
      event.completed = true; event.status = response.status; event.completedAfterAbort = event.aborted; return response
    }, error => { event.failed = true; event.errorName = error.name; throw error })
  }
}

async function fixture(pathname, options = {}) {
  const context = await browser.newContext({ viewport: { width: 1500, height: 1100 } })
  const page = await context.newPage()
  page.setDefaultTimeout(12_000)
  const state = { profile: profile(options.apps, options.org, options.platformContext), requests: [], errors: [],
    menus: new Map([[system.id, []], [appA.id, [appMenu(appA)]], [appB.id, [appMenu(appB)]]]),
    permissions: options.permissions ?? [], menuErrors: new Set(options.menuErrors ?? []), gates: new Map(),
    entries: [], contextRequests: [], menuReplies: [] }
  for (const [id, menus] of Object.entries(options.menus ?? {})) state.menus.set(id, menus)
  for (const id of options.holdMenus ?? []) state.gates.set(id, deferred())
  const close = async () => { for (const gate of state.gates.values()) gate.resolve(); await context.close() }
  state.release = id => state.gates.get(id)?.resolve()
  page.on('pageerror', error => state.errors.push(error.message))
  await context.addInitScript(({ recent, detachMenus }) => {
    if (!localStorage.getItem("platform-session-v1")) localStorage.setItem("platform-session-v1", JSON.stringify({ schema: 1, version: 1, sessionId: crypto.randomUUID(), status: "active", token: 'phase9-session-A', refreshToken: null }))
    localStorage.setItem('platform-theme', 'light')
    localStorage.setItem('platform-current-app', 'nav-system')
    if (recent) localStorage.setItem('platform-recent-visits', JSON.stringify(recent))
  }, { recent: options.recent, detachMenus: options.detachMenus })
  await context.addInitScript(observeTransport, Boolean(options.detachMenus))
  await context.routeWebSocket(() => true, () => {})
  await context.route(url => url.origin === origin && url.pathname === '/src/main.tsx', async route => {
    const response = await route.fetch()
    const body = (await response.text()).replace('ReactDOM.createRoot',
      "import { authStore as phase9AuthStore } from '/src/stores/auth.ts';\nwindow.__phase9Auth = phase9AuthStore;\nwindow.__phase9QueryClient = queryClient;\nReactDOM.createRoot")
    assert.match(body, /window\.__phase9Auth/)
    await route.fulfill({ response, body })
  })
  const fulfill = async (route, body, status = 200) => {
    try { await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) }) }
    catch (error) { if (!/closed|disposed|Invalid Interception|Target|cancelled/i.test(error.message)) throw error }
  }
  await context.route(url => url.origin === origin && url.pathname.startsWith('/__phase9/'), async route => {
    const pathname = new URL(route.request().url()).pathname
    state.entries.push(pathname)
    await route.fulfill({ status: 200, contentType: 'text/html', body: `<!doctype html><html><body><script>
      var active; window.__MICRO_APP__ = { name: 'phase9-menu-probe',
        mount: function(container, props) { active = container; var node = document.createElement('p'); node.dataset.testid = 'phase9-mounted'; node.textContent = props.organization.id + ':' + props.routePath; container.replaceChildren(node); },
        update: function(props) { active.querySelector('[data-testid="phase9-mounted"]').textContent = props.organization.id + ':' + props.routePath; },
        unmount: function() { if(active) active.replaceChildren(); } };
      </script></body></html>` })
  })
  await context.route(url => url.origin === origin && url.pathname.startsWith('/api/'), async route => {
    const request = route.request(), url = new URL(request.url())
    const session = request.headers().authorization === 'Bearer phase9-session-B' ? 'B' : 'A'
    const record = { path: url.pathname, appId: url.searchParams.get('applicationId'), session, method: request.method() }
    state.requests.push(record)
    if (url.pathname === '/api/platform/branding') return fulfill(route, { platformName: '导航验收平台', platformDescription: '', logoUrl: '',
      loginTitle: '登录', loginDescription: '', footerText: '' })
    if (url.pathname === '/api/auth/profile') return fulfill(route, state.profile)
    if (url.pathname === '/api/auth/permissions') return fulfill(route, options.permissionError ? { message: '权限暂不可用' } :
      { permissions: state.permissions, roles: [] }, options.permissionError ? 503 : 200)
    if (url.pathname === '/api/auth/menus') {
      const id = record.appId, org = state.profile.organization.id
      const menus = structuredClone(state.menus.get(id) ?? [])
      if (session === 'A') await state.gates.get(id)?.promise
      const status = state.menuErrors.has(id) ? 503 : 200
      state.menuReplies.push({ appId: id, session, org, status, menuPaths: menus.map(node => node.path) })
      return fulfill(route, status === 200 ? { menus } : { message: '无关应用菜单暂不可用' }, status)
    }
    if (url.pathname === '/api/oauth/session-token') {
      const application = state.profile.applications.find(app => app.code === request.postDataJSON()?.appCode)
      return fulfill(route, { accessToken: 'phase9-application-' + (application?.code || ''), tokenType: 'Bearer', expiresIn: 60, audience: application?.code })
    }
    const contextMatch = /^\/api\/apps\/([^/]+)\/context$/.exec(url.pathname)
    if (contextMatch) {
      const application = state.profile.applications.find(app => app.code === contextMatch[1])
      state.contextRequests.push({ code: contextMatch[1], org: state.profile.organization.id })
      assert(application)
      return fulfill(route, { contractVersion: '1.0', contextVersion: 'phase9-' + state.profile.organization.id,
        expiresAt: new Date(Date.now() + 60_000).toISOString(), appCode: application.code,
        application: { id: application.id, name: application.name, runtimeMode: application.runtimeMode,
          entryType: application.entryType, entryUrl: application.entryUrl, deploymentVersion: application.deploymentVersion,
          entryVersionId: application.entryVersionId }, user: { id: user.id, username: user.username, displayName: user.realName },
        tenant: state.profile.tenant, organization: state.profile.organization, roles: [],
        permissions: state.permissions, dataGrants: [], menus: state.menus.get(application.id) ?? [] })
    }
    if (url.pathname === '/api/tenants') return fulfill(route, { records: [tenantRow], total: 1, page: 1, size: 10 })
    if (url.pathname === '/api/monitor/overview') return fulfill(route, { code: 'FORBIDDEN', message: '监控数据独立拒绝' }, 403)
    if (url.pathname === '/api/notifications/prefs') return fulfill(route, { dndEnabled: false, dndStart: null, dndEnd: null, mutedTypes: [] })
    if (url.pathname === '/api/notifications/unread-count') return fulfill(route, { count: 0 })
    if (url.pathname === '/api/notifications/socket-context') return fulfill(route, { userId: user.id, tenantId: state.profile.tenant.id,
      organizationId: state.profile.organization.id, sessionVersion: 0 })
    if (['/api/notifications', '/api/auth/login-history'].includes(url.pathname)) return fulfill(route, { records: [], total: 0, page: 1, size: 10 })
    if (url.pathname === '/api/auth/my-sessions') return fulfill(route, { sessions: [], summary: { active: 0, revoked: 0 } })
    if (url.pathname === '/api/search') return fulfill(route, { users: [], tenants: [], organizations: [], roles: [] })
    return fulfill(route, { message: 'Unconfigured business request ' + url.pathname }, 403)
  })
  try {
    await page.goto(origin + pathname, { waitUntil: 'domcontentloaded' })
    await page.getByRole('complementary', { name: '应用导航', exact: true }).waitFor()
    return { context, page, state, close }
  } catch (error) { await close(); throw error }
}
const rail = page => page.getByRole('complementary', { name: '应用导航', exact: true })
const sidebar = page => page.getByRole('complementary', { name: '菜单导航', exact: true })
const menuRequests = (state, id, session = 'A') => state.requests.filter(request => request.path === '/api/auth/menus' && request.appId === id && request.session === session)
const completedMenuRequests = async (page, id) => page.evaluate(applicationId => window.__phase9Transport.events.filter(event =>
  event.appId === applicationId && event.completed && !event.cancelledBeforeResponse), id)
const navigate = async (page, pathname) => {
  await page.evaluate(next => { history.pushState({}, '', next); dispatchEvent(new PopStateEvent('popstate')) }, pathname)
  await page.waitForURL(url => url.pathname === pathname)
}
const openPalette = async page => { await page.keyboard.press('Control+k'); await page.getByRole('dialog', { name: '命令面板', exact: true }).waitFor() }
const emit = (scenario, data) => console.log('PHASE9_NAVIGATION_TRACE ' + JSON.stringify({ scenario, ...data }))
const recent = (pathname, title) => ({ path: pathname, title, icon: 'Boxes', at: Date.now(), count: 9 })
async function check(name, pathname, options, run) {
  if (process.env.PHASE9_BROWSER_FILTER && !new RegExp(process.env.PHASE9_BROWSER_FILTER).test(name)) return
  const f = await fixture(pathname, options)
  try {
    await run(f)
    assert.deepEqual(f.state.errors, [], 'Actual production React has no uncaught errors')
    count++; console.log('PASS ' + name)
  } catch (error) {
    console.error(JSON.stringify({ name, requests: f.state.requests, replies: f.state.menuReplies,
      entries: f.state.entries, errors: f.state.errors, body: await f.page.locator('body').innerText().catch(() => '') }))
    throw error
  } finally { await f.close() }
}
const appButton = (page, application, source) => source === 'rail'
  ? rail(page).getByRole('button', { name: '切换到应用 ' + application.name, exact: true })
  : page.locator('#app-scroll').getByRole('button', { name: new RegExp(application.name) })

try {
  await startServer(); browser = await launchBrowser()
  console.log('PHASE9_BROWSER_SOURCE ' + JSON.stringify({ browserVersion: await browser.version(),
    testSha256: createHash('sha256').update(readFileSync(fileURLToPath(import.meta.url))).digest('hex'),
    mode: process.env.PHASE9_VITE_CONFIG ? 'baseline-source-overrides-negative' : 'actual-current-source' }))
  await check('phase9 closed palette keeps unrelated application menus lazy and current query deduplicated', '/profile', {}, async ({ page, state }) => {
    await sidebar(page).getByText('该应用暂无菜单', { exact: true }).waitFor()
    await pause(180)
    assert.equal((await completedMenuRequests(page, system.id)).length, 1, 'Sidebar and topbar share one current app/epoch request; a StrictMode request cancelled before response is counted separately')
    assert.equal(menuRequests(state, appA.id).length, 0, 'A closed palette must not eagerly request unrelated app A')
    assert.equal(menuRequests(state, appB.id).length, 0, 'A closed palette must not eagerly request unrelated app B')
    emit('closed-lazy-dedup', { requests: state.requests.filter(request => request.path === '/api/auth/menus'), transport: await page.evaluate(() => window.__phase9Transport.events) })
  })
  await check('phase9 opening palette shares current menu and fetches each other app once', '/profile', {}, async ({ page, state }) => {
    await sidebar(page).getByText('该应用暂无菜单', { exact: true }).waitFor()
    await openPalette(page)
    await waitState(() => menuRequests(state, appA.id).length === 1 && menuRequests(state, appB.id).length === 1, 'Opening starts the other app queries')
    await page.getByRole('dialog').getByRole('combobox').fill('入口')
    await page.getByRole('option').filter({ hasText: appA.name + '入口' }).waitFor()
    assert.equal((await completedMenuRequests(page, system.id)).length, 1)
    await page.keyboard.press('Escape'); await openPalette(page); await pause(100)
    for (const application of [system, appA, appB]) assert.equal((await completedMenuRequests(page, application.id)).length, 1)
    emit('opened-shared-cache', { requests: state.requests.filter(request => request.path === '/api/auth/menus') })
  })
  for (const [label, sourceA, sourceB] of [['rail', 'rail', 'rail'], ['dashboard', 'dashboard', 'dashboard'], ['shared dashboard and rail', 'dashboard', 'rail']]) {
    await check('phase9 ' + label + ' A to B ignores late A navigation', '/dashboard', { holdMenus: [appA.id, appB.id] }, async ({ page, state }) => {
      await page.getByText('我的应用', { exact: true }).waitFor()
      await appButton(page, appA, sourceA).click()
      await waitState(() => menuRequests(state, appA.id).length > 0, 'A request is held')
      await appButton(page, appB, sourceB).click()
      await waitState(() => menuRequests(state, appB.id).length > 0, 'B request starts before A completes')
      state.release(appB.id)
      await page.waitForURL('**' + appB.basePath + '/home')
      await page.getByTestId('phase9-mounted').waitFor()
      state.release(appA.id)
      await waitState(() => state.menuReplies.some(reply => reply.appId === appA.id), 'Actual late A response completed')
      await pause(120)
      assert.equal(new URL(page.url()).pathname, appB.basePath + '/home')
      assert.equal(await rail(page).getByRole('button', { name: '切换到应用 ' + appB.name }).getAttribute('aria-current'), 'true')
      assert.equal(await page.getByTestId('phase9-mounted').innerText(), 'org-A:' + appB.basePath + '/home')
      assert.equal(state.entries.some(entry => entry.includes('/a/')), false)
      emit('selection-' + label.replaceAll(' ', '-'), { selected: appB.id, path: new URL(page.url()).pathname, replies: state.menuReplies, entries: state.entries })
    })
  }
  await check('phase9 A to B to A generation performs exactly one final navigation', '/profile', { holdMenus: [appA.id, appB.id] }, async ({ page, state }) => {
    await appButton(page, appA, 'rail').click()
    await waitState(() => menuRequests(state, appA.id).length > 0, 'First A is pending')
    await appButton(page, appB, 'rail').click()
    await waitState(() => menuRequests(state, appB.id).length > 0, 'B is pending')
    await appButton(page, appA, 'rail').click()
    state.release(appA.id)
    await page.waitForURL('**' + appA.basePath + '/home'); await page.getByTestId('phase9-mounted').waitFor()
    state.release(appB.id); await pause(150)
    const history = await page.evaluate(() => window.__phase9Transport.history)
    assert.equal(history.filter(entry => entry.path === appA.basePath + '/home').length, 1, 'Superseded first A callback cannot navigate again when A becomes current')
    assert.equal(new URL(page.url()).pathname, appA.basePath + '/home')
    emit('selection-A-B-A', { history, entries: state.entries })
  })
  await check('phase9 manual route navigation invalidates pending application selection', '/dashboard', { holdMenus: [appA.id] }, async ({ page, state }) => {
    await page.getByText('我的应用', { exact: true }).waitFor()
    await appButton(page, appA, 'rail').click()
    await waitState(() => menuRequests(state, appA.id).length > 0, 'A is pending')
    await page.locator('#app-scroll').getByRole('button', { name: /个人中心.*资料与登录信息/ }).click()
    await page.waitForURL('**/profile')
    state.release(appA.id)
    await waitState(() => state.menuReplies.some(reply => reply.appId === appA.id), 'Late A menu delivered')
    await pause(120)
    assert.equal(new URL(page.url()).pathname, '/profile')
    assert.deepEqual(state.entries, [])
    emit('manual-navigation-invalidates', { path: new URL(page.url()).pathname, entries: state.entries })
  })
  const tenantMenu = menu('known-tenant', '明确授权租户入口', '/system/tenants', 'system:tenant')
  await check('phase9 known authorized page and display survive unrelated menu failure', '/system/tenants', {
    permissions: ['system:tenant'], menus: { [system.id]: [tenantMenu] }, menuErrors: [appA.id],
    recent: [recent('/system/tenants', '明确授权最近访问')],
  }, async ({ page, state }) => {
    await page.getByText(tenantRow.name, { exact: true }).waitFor()
    await sidebar(page).getByRole('button', { name: tenantMenu.title, exact: true }).waitFor()
    await openPalette(page)
    await waitState(() => state.menuReplies.some(reply => reply.appId === appA.id && reply.status === 503), 'Unrelated failure actually occurred')
    await page.getByRole('dialog').getByRole('combobox').fill(tenantMenu.title)
    await page.getByRole('option').filter({ hasText: tenantMenu.title }).waitFor()
    await page.keyboard.press('Escape')
    assert.equal(await page.getByText(tenantRow.name, { exact: true }).count(), 1)
    assert.equal(await page.getByText('访问权限验证失败，请重试', { exact: true }).count(), 0)
    await navigate(page, '/dashboard')
    await page.locator('#app-scroll').getByRole('button').filter({ hasText: '/system/tenants' }).waitFor()
    emit('unrelated-error-known-grant', { failureObserved: true, businessRequests: state.requests.filter(request => request.path === '/api/tenants').length })
  })
  const rootMenu = menu('root-settings', '受限平台设置入口', '/system/settings', 'system:settings')
  await check('phase9 platform root restriction agrees across sidebar palette recent and direct route', '/dashboard', {
    platformContext: false, permissions: ['system:settings'], menus: { [system.id]: [rootMenu] },
    recent: [recent('/system/settings', '受限平台最近访问')],
  }, async ({ page, state }) => {
    await page.getByText('我的应用', { exact: true }).waitFor()
    assert.equal(await sidebar(page).getByRole('button', { name: rootMenu.title, exact: true }).count(), 0)
    assert.equal(await page.getByText('受限平台最近访问', { exact: true }).count(), 0)
    await openPalette(page); await page.getByRole('dialog').getByRole('combobox').fill(rootMenu.title); await pause(150)
    assert.equal(await page.getByRole('option').filter({ hasText: rootMenu.title }).count(), 0)
    await page.keyboard.press('Escape'); await navigate(page, '/system/settings')
    await page.getByText('无访问权限', { exact: true }).waitFor()
    assert.equal(state.requests.some(request => request.path === '/api/platform/settings'), false)
    emit('root-policy-display-direct', { platformContext: false, businessRequests: 0 })
  })
  await check('phase9 personal routes remain displayed and direct without MENU or permissions', '/dashboard', {
    permissionError: true, menuErrors: [system.id, appA.id, appB.id],
  }, async ({ page, state }) => {
    await page.getByText('我的应用', { exact: true }).waitFor()
    await openPalette(page)
    const dialog = page.getByRole('dialog')
    await dialog.getByRole('option').filter({ hasText: '仪表盘' }).waitFor()
    await page.keyboard.press('Escape')
    for (const pathname of ['/profile', '/system/notifications', '/dashboard']) {
      await navigate(page, pathname)
      if (pathname === '/dashboard') await page.getByText('我的应用', { exact: true }).waitFor()
      else await page.getByRole('heading', { name: pathname === '/profile' ? '个人中心' : '通知中心', exact: true }).waitFor()
      assert.equal(await page.getByText('无访问权限', { exact: true }).count(), 0)
    }
    assert(state.requests.some(request => request.path === '/api/monitor/overview'))
    emit('personal-policy-display-direct', { paths: ['/dashboard', '/profile', '/system/notifications'], independentMonitorDenied: true })
  })
  const unknownMenu = menu('unknown-menu', '未知权限路径入口', '/system/phase9-unknown', 'system:unknown')
  await check('phase9 unknown path remains hidden and denied even when named by MENU', '/dashboard', {
    permissions: ['system:unknown'], menus: { [system.id]: [unknownMenu] },
    recent: [recent(unknownMenu.path, '未知路径最近访问')],
  }, async ({ page, state }) => {
    await page.getByText('我的应用', { exact: true }).waitFor()
    assert.equal(await sidebar(page).getByRole('button', { name: unknownMenu.title, exact: true }).count(), 0)
    assert.equal(await page.getByText('未知路径最近访问', { exact: true }).count(), 0)
    await openPalette(page); await page.getByRole('dialog').getByRole('combobox').fill(unknownMenu.title); await pause(120)
    assert.equal(await page.getByRole('option').filter({ hasText: unknownMenu.title }).count(), 0)
    await page.keyboard.press('Escape'); await navigate(page, unknownMenu.path)
    await page.getByText('无访问权限', { exact: true }).waitFor()
    assert.equal(state.requests.some(request => request.path.includes('phase9-unknown')), false)
    emit('unknown-policy-display-direct', { path: unknownMenu.path, denied: true })
  })
  const workOrders = { ...app('work-orders'), id: 'nav-work-orders', code: 'work-orders', name: '工单创建路由验收', basePath: '/micro/work-orders' }
  const detailMenu = menu('work-orders-detail', '只读工单详情', '/micro/work-orders/tickets/:id', 'ticket:read', { visible: false })
  const createMenu = menu('work-orders-create', '明确工单创建', '/micro/work-orders/ticket-create', 'ticket:create')
  await check('phase9 detail-only permission cannot grant the nonoverlapping creation route', createMenu.path, {
    apps: [system, workOrders], permissions: ['ticket:read'], menus: { [workOrders.id]: [detailMenu] },
  }, async ({ page, state }) => {
    await page.getByText('无访问权限', { exact: true }).waitFor()
    assert.deepEqual(state.entries, []); assert.deepEqual(state.contextRequests, [])
    await navigate(page, '/micro/work-orders/tickets/existing')
    await page.getByTestId('phase9-mounted').waitFor()
    assert.equal(await page.getByTestId('phase9-mounted').innerText(), 'org-A:/micro/work-orders/tickets/existing')
    emit('detail-only-denies-creation', { deniedPath: createMenu.path, permittedDetail: '/micro/work-orders/tickets/existing', entries: state.entries })
  })
  await check('phase9 explicit creation MENU permits creation route independently of detail', createMenu.path, {
    apps: [system, workOrders], permissions: ['ticket:create'], menus: { [workOrders.id]: [createMenu] },
  }, async ({ page, state }) => {
    await page.getByTestId('phase9-mounted').waitFor()
    assert.equal(await page.getByTestId('phase9-mounted').innerText(), 'org-A:' + createMenu.path)
    await navigate(page, '/micro/work-orders/tickets/existing'); await page.getByText('无访问权限', { exact: true }).waitFor()
    assert.equal(state.entries.length, 1)
    emit('creation-MENU-independent', { allowedPath: createMenu.path, detailDenied: true })
  })
  await check('phase9 cross organization rejects actual ignored-abort late menus without old entry or data', '/profile', {
    detachMenus: true, holdMenus: [appA.id], recent: [recent(appA.basePath + '/home', '旧组织私有入口')],
  }, async ({ page, state }) => {
    await openPalette(page)
    await waitState(() => menuRequests(state, appA.id).length > 0, 'Old org request is actually pending')
    state.profile = profile([system, appA, appB], 'org-B')
    state.menus.set(appA.id, [])
    await page.evaluate(() => window.__phase9Auth.getState().setSession('phase9-session-B'))
    await waitState(async () => (await page.evaluate(() => window.__phase9Auth.getState().profile?.organization.id)) === 'org-B', 'New organization profile is authoritative')
    await openPalette(page)
    await waitState(() => menuRequests(state, appA.id, 'B').length > 0, 'New organization obtains its own menu query')
    state.release(appA.id)
    await waitState(async () => (await page.evaluate(() => window.__phase9Transport.events)).some(event => event.appId === appA.id && event.detached && event.completed), 'Detached native transport actually delivered the old response')
    const events = await page.evaluate(() => window.__phase9Transport.events)
    const old = events.filter(event => event.appId === appA.id && event.detached)
    assert(old.length > 0 && old.every(event => event.signalPresent && event.aborted && event.completedAfterAbort), 'Old epoch signal is aborted even when native transport ignores abort')
    await page.getByRole('dialog').getByRole('combobox').fill(appA.name + '入口'); await pause(120)
    assert.equal(await page.getByRole('option').filter({ hasText: appA.name + '入口' }).count(), 0)
    await page.keyboard.press('Escape'); await navigate(page, '/dashboard'); await page.getByText('我的应用', { exact: true }).waitFor()
    assert.equal(await page.getByText('旧组织私有入口', { exact: true }).count(), 0)
    assert.deepEqual(state.entries, []); assert.deepEqual(state.contextRequests, [])
    assert.equal(await page.evaluate(() => window.__phase9Auth.getState().profile.organization.id), 'org-B')
    const cached = await page.evaluate(() => window.__phase9QueryClient.getQueryCache().getAll().filter(query => query.queryKey[0] === 'auth-menus').map(query => ({ key: query.queryKey, data: query.state.data })))
    assert(!JSON.stringify(cached).includes(appA.basePath + '/home'), 'Old menu bytes never enter current query cache')
    emit('cross-org-ignored-abort', { oldNativeResponses: old, currentOrganization: 'org-B', entries: state.entries, contextRequests: state.contextRequests, cache: cached })
  })
  console.log(`Phase9 navigation browser checks: ${count} passed`)
} finally {
  if (browser) await browser.close()
  if (vite && vite.exitCode === null) { vite.kill(); await pause(100) }
}
