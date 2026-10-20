/** Real same-origin pages use the production coordinator, Web Locks and host UI.
 * HTTP and Socket.IO fixtures isolate business servers; no coordinator is faked. */
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { spawn, spawnSync } from 'node:child_process'
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import http from 'node:http'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { launchBrowser, socketServer } from './browser/runtime.mjs'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const bind = process.platform === 'win32' ? '::1' : '127.0.0.1'
const host = port => `http://${bind.includes(':') ? '[' + bind + ']' : bind}:${port}`
const key = 'platform-session-v1'
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done }); return { promise, resolve } }
const pause = ms => new Promise(resolve => setTimeout(resolve, ms))
async function wait(predicate, message, timeout = 15_000) {
  const deadline = Date.now() + timeout
  while (!await predicate() && Date.now() < deadline) await pause(20)
  assert(await predicate(), message)
}
const system = { id: 'session-system', code: 'system', name: '系统管理', icon: 'Boxes', basePath: '/system' }
const workOrders = { id: 'session-work-orders', code: 'work-orders', name: '会话验收工单', icon: 'Boxes',
  basePath: '/micro/work-orders', runtimeMode: 'IFRAME', entryType: 'HTML', entryUrl: '/__phase10-work-orders/',
  deploymentVersion: 'phase10', entryVersionId: 'session-work-orders-release' }
const menu = (id, title, pathname, permission, visible = true) => ({ id, title, path: pathname, permission,
  type: 'MENU', visible, parentId: null, icon: 'Boxes', children: [], sortOrder: 1 })
const menus = [menu('tickets', '工单列表', '/micro/work-orders', 'ticket:read'),
  menu('create', '新建工单', '/micro/work-orders/ticket-create', 'ticket:create'),
  menu('detail', '工单详情', '/micro/work-orders/tickets/:id', 'ticket:read', false)]
const permissions = ['ticket:read', 'ticket:create', 'ticket:update', 'ticket:delete', 'ticket:export']
const user = { id: 'session-user', username: 'session-test', realName: '会话验收用户', email: '', phone: '', gender: 'UNKNOWN', avatar: '', isSuperAdmin: false }
const owner = org => ({ userId: user.id, tenantId: 'session-tenant', organizationId: 'org-' + org })
const profile = org => ({ user, tenant: { id: 'session-tenant', name: '验收租户', code: 'session', isPlatform: false },
  organization: { id: 'org-' + org, name: '组织 ' + org, path: '/org-' + org + '/' }, applications: [system, workOrders],
  memberships: ['A', 'B'].map(value => ({ userId: user.id, tenantId: 'session-tenant', tenantName: '验收租户',
    organizationId: 'org-' + value, organizationName: '组织 ' + value, organizationPath: '/org-' + value + '/' })) })
let browser, vite, origin, io, socketHttp, buildOutput, sequence = 0, passed = 0
const tokenOwners = new Map()
async function start() {
  socketHttp = http.createServer((_, response) => { response.writeHead(404); response.end() })
  io = new socketServer(socketHttp, { path: '/socket.io/', transports: ['websocket', 'polling'] })
  io.on('connection', socket => socket.on('authenticate', ({ token } = {}) => {
    const entry = tokenOwners.get(token)
    if (!entry) return socket.disconnect(true)
    const row = { org: entry.org, disconnected: false, id: socket.id }
    entry.state.connections.push(row)
    socket.on('disconnect', () => { row.disconnected = true })
    socket.emit('authenticated', { contractVersion: '1.0', ...owner(entry.org), expiresAt: new Date(Date.now() + 120_000).toISOString() })
  }))
  await new Promise(resolve => socketHttp.listen(0, bind, resolve))
  const port = await new Promise(resolve => { const s = net.createServer(); s.listen(0, bind, () => { const p = s.address().port; s.close(() => resolve(p)) }) })
  origin = host(port)
  vite = spawn(process.execPath, [path.join(root, 'frontend/node_modules/vite/bin/vite.js'), '--host', bind, '--port', String(port), '--strictPort'],
    { cwd: path.join(root, 'frontend'), windowsHide: true, env: { ...process.env, NOTIFICATION_DEV_PROXY_TARGET: host(socketHttp.address().port) }, stdio: ['ignore', 'pipe', 'pipe'] })
  let output = '', error
  vite.stdout.on('data', data => { output = (output + data).slice(-8000) }); vite.stderr.on('data', data => { output = (output + data).slice(-8000) }); vite.on('error', e => { error = e })
  await wait(() => { if (error) throw error; if (vite.exitCode !== null) throw Error(output); return output.includes('ready in') }, 'Vite readiness', 30_000)
  buildOutput = path.join(mkdtempSync(path.join(os.tmpdir(), 'phase10-work-orders-')), 'build')
  const built = spawnSync(process.execPath, ['business-apps/work-orders/frontend/scripts/build.mjs', '--out-dir', buildOutput,
    '--allowed-host-origin', origin, '--version', 'phase10'], { cwd: root, encoding: 'utf8' })
  assert.equal(built.status, 0, built.stderr || built.stdout)
}

// Observe native transport cancellation and version-only notifications. A single
// explicitly armed request may ignore transport abort to test late 200 rejection.
function observeTransport() {
  const native = window.fetch.bind(window)
  const state = window.__phase10Transport = { events: [], detachNext: false, detachRefresh: false, broadcast: [], storageEvents: 0 }
  window.addEventListener('storage', () => { state.storageEvents++ })
  if (typeof BroadcastChannel !== 'undefined') {
    const post = BroadcastChannel.prototype.postMessage
    BroadcastChannel.prototype.postMessage = function (data) { state.broadcast.push({ keys: Object.keys(data), version: data.version }); return post.call(this, data) }
  }
  window.fetch = (input, options = {}) => {
    const url = new URL(typeof input === 'string' ? input : input.url, location.href)
    if (!url.pathname.startsWith('/api/phase10/') && url.pathname !== '/oauth/token') return native(input, options)
    const signal = options.signal, detached = url.pathname === '/oauth/token' ? state.detachRefresh : state.detachNext; state.detachNext = false
    const row = { path: url.pathname, signal: Boolean(signal), aborted: Boolean(signal?.aborted), detached, completed: false }
    state.events.push(row); signal?.addEventListener('abort', () => { row.aborted = true }, { once: true })
    return native(input, detached ? { ...options, signal: undefined } : options).then(response => { row.completed = true; row.completedAfterAbort = row.aborted; return response }, error => { row.failed = true; throw error })
  }
}
async function fixture(options = {}) {
  const id = ++sequence, context = await browser.newContext({ viewport: { width: 1450, height: 1000 } })
  const tokens = Object.fromEntries(['A', 'AR', 'B', 'C'].map(group => [group, 'phase10-' + id + '-' + group]))
  const state = { requests: [], refreshes: [], connections: [], holds: [], writes: [], expiredAlways: false,
    refreshMode: 'success', appTokenTtl: 120, refreshGate: null, contextGate: null, dataGate: null, old401Gate: null, logoutGate: null, errors: [], ...options }
  for (const [group, token] of Object.entries(tokens)) tokenOwners.set(token, { state, org: group === 'AR' ? 'A' : group })
  const classify = token => Object.entries(tokens).find(([, value]) => value === token)?.[0] ?? 'none'
  context.on('page', page => { page.setDefaultTimeout(15_000); page.on('pageerror', error => state.errors.push(error.message)) })
  if (options.noLocks || options.noStorage || options.noBroadcast || options.missedEvents) await context.addInitScript(flags => {
    if (flags.noLocks) Object.defineProperty(navigator, 'locks', { value: undefined })
    if (flags.noStorage) Object.defineProperty(window, 'localStorage', { get() { throw new DOMException('Storage denied', 'SecurityError') } })
    if (flags.noBroadcast || flags.missedEvents) window.BroadcastChannel = undefined
    if (flags.missedEvents) {
      window.__phase10DropStorage = false
      const add = window.addEventListener.bind(window)
      window.addEventListener = (type, listener, ...args) => add(type, type === 'storage' ? event => { if (!window.__phase10DropStorage) listener(event) } : listener, ...args)
    }
  }, options)
  await context.addInitScript(observeTransport)
  await context.route(url => url.origin === origin && url.pathname === '/__phase10-seed', route => route.fulfill({ contentType: 'text/html', body: '<!doctype html><body>One-time fixture seed</body>' }))
  await context.route(url => url.origin === origin && url.pathname === '/src/main.tsx', async route => {
    const response = await route.fetch(), body = await response.text()
    const exposed = body.replace('ReactDOM.createRoot', `import { authStore as phase10Auth, captureSessionFence as phase10Fence } from '/src/stores/auth.ts';
      import { api as phase10Api, switchPlatformContext as phase10Switch, logoutPlatformSession as phase10Logout } from '/src/lib/api.ts';
      window.__phase10Auth=phase10Auth; window.__phase10Api=phase10Api; window.__phase10Switch=phase10Switch;
      window.__phase10Logout=phase10Logout; window.__phase10Fence=phase10Fence; window.__phase10Query=queryClient;
      ReactDOM.createRoot`)
    assert.notEqual(exposed, body); await route.fulfill({ response, body: exposed })
  })
  const fulfill = async (route, body, status = 200, headers = {}) => {
    try { await route.fulfill({ status, headers, contentType: 'application/json', body: JSON.stringify(body) }) }
    catch (error) { if (!/closed|disposed|Invalid Interception|Target|cancelled/i.test(error.message)) throw error }
  }
  await context.route(url => url.origin === origin && url.pathname.startsWith('/__phase10-work-orders/'), route => {
    const relative = new URL(route.request().url()).pathname.slice('/__phase10-work-orders/'.length) || 'index.html'
    const file = path.resolve(buildOutput, relative); assert(file.startsWith(buildOutput + path.sep))
    return route.fulfill({ status: existsSync(file) ? 200 : 404, headers: { 'Access-Control-Allow-Origin': '*' }, contentType: file.endsWith('.html') ? 'text/html' : file.endsWith('.css') ? 'text/css' : 'text/javascript', body: existsSync(file) ? readFileSync(file) : '' })
  })
  await context.route(url => url.origin === origin && (url.pathname.startsWith('/api/') || url.pathname === '/oauth/token' || url.pathname.startsWith('/work-orders-api/')), async route => {
    const request = route.request(), url = new URL(request.url()), token = request.headers().authorization?.replace(/^Bearer /, ''), group = classify(token?.replace(/^app-/, ''))
    const org = group === 'AR' ? 'A' : group
    const row = { path: url.pathname, method: request.method(), group }; state.requests.push(row)
    if (url.pathname === '/oauth/token') {
      const supplied = new URLSearchParams(request.postData()).get('refresh_token')
      state.refreshes.push({ old: supplied === 'psr-' + tokens.A, ordinal: state.refreshes.length + 1 })
      await state.refreshGate?.promise
      if (state.refreshMode === 'network') return route.abort('connectionreset')
      if (state.refreshMode === 'reject') return fulfill(route, { error: 'invalid_grant' }, 400)
      return fulfill(route, { access_token: tokens.AR, refresh_token: 'psr-' + tokens.AR, expires_in: 3600 })
    }
    if (url.pathname === '/api/auth/context') { await state.contextGate?.promise; return fulfill(route, { accessToken: tokens.B, refreshToken: 'psr-' + tokens.B }) }
    if (url.pathname === '/api/auth/logout') { await state.logoutGate?.promise; return fulfill(route, {}) }
    if (url.pathname === '/api/phase10/expired') {
      if (group === 'A' && state.old401Gate) await state.old401Gate.promise
      return fulfill(route, { ok: true }, group === 'A' || state.expiredAlways ? 401 : 200)
    }
    if (url.pathname === '/api/phase10/held') { await state.dataGate?.promise; return fulfill(route, { oldOrganization: org }) }
    if (url.pathname === '/api/platform/branding') return fulfill(route, { platformName: '会话验收平台', platformDescription: '', logoUrl: '', loginTitle: '登录', loginDescription: '', footerText: '' })
    if (url.pathname === '/api/auth/profile') return fulfill(route, profile(org))
    if (url.pathname === '/api/auth/permissions') return fulfill(route, { permissions, roles: [] })
    if (url.pathname === '/api/auth/menus') return fulfill(route, { menus: url.searchParams.get('applicationId') === workOrders.id ? menus : [] })
    if (url.pathname === '/api/oauth/session-token') return fulfill(route, { accessToken: 'app-' + token, tokenType: 'Bearer', expiresIn: state.appTokenTtl, audience: 'work-orders' })
    if (url.pathname === '/api/apps/work-orders/context') return fulfill(route, { contractVersion: '1.0', contextVersion: 'phase10-' + org,
      expiresAt: new Date(Date.now() + 120_000).toISOString(), appCode: 'work-orders',
      application: { id: workOrders.id, name: workOrders.name, runtimeMode: 'IFRAME', entryType: 'HTML', entryUrl: workOrders.entryUrl, deploymentVersion: 'phase10', entryVersionId: workOrders.entryVersionId },
      user: { id: user.id, username: user.username, displayName: user.realName }, tenant: profile(org).tenant,
      organization: profile(org).organization, roles: [], permissions,
      dataGrants: ['read', 'create', 'update', 'delete', 'export'].map(action => ({ resource: 'ticket', action, scope: 'ORG', subjectUserId: user.id, tenantId: 'session-tenant', organizationIds: ['org-' + org] })), menus })
    if (url.pathname.startsWith('/work-orders-api/')) {
      const cors = { 'Access-Control-Allow-Origin': 'null', 'Access-Control-Allow-Methods': 'GET,POST,PATCH,DELETE,OPTIONS', 'Access-Control-Allow-Headers': 'Authorization,Content-Type,Idempotency-Key,X-Request-Id' }
      if (request.method() === 'OPTIONS') return route.fulfill({ status: 204, headers: cors })
      if (request.method() === 'POST') state.writes.push({ path: url.pathname, group: classify(token?.replace(/^app-/, '')), title: request.postDataJSON()?.title })
      return fulfill(route, url.pathname.endsWith('/assignees') ? { records: [], total: 0, page: 1, size: 20 } : { records: [], total: 0, page: 1, size: 20 }, 200, cors)
    }
    if (url.pathname === '/api/notifications/prefs') return fulfill(route, { dndEnabled: false, mutedTypes: [], dndStart: null, dndEnd: null })
    if (url.pathname === '/api/notifications/unread-count') return fulfill(route, { count: 0 })
    if (url.pathname === '/api/notifications/socket-context') return fulfill(route, { ...owner(org), sessionVersion: 0 })
    if (url.pathname === '/api/auth/my-sessions') return fulfill(route, { sessions: [], summary: { active: 0, revoked: 0 } })
    if (url.pathname === '/api/search') return fulfill(route, { users: [], tenants: [], organizations: [], roles: [] })
    if (url.pathname === '/api/monitor/overview') return fulfill(route, { message: 'Forbidden' }, 403)
    return fulfill(route, { records: [], total: 0, page: 1, size: 10 })
  })
  const a = await context.newPage(); await a.goto(origin + '/__phase10-seed')
  if (!options.noStorage) await a.evaluate(({ key, token, legacy, id }) => {
    localStorage.setItem('platform-theme', 'light')
    if (legacy) { localStorage.setItem('platform-token', token); localStorage.setItem('platform-refresh-token', 'psr-' + token) }
    else localStorage.setItem(key, JSON.stringify({ schema: 1, version: 1, sessionId: 'fixture-session-' + id, status: 'active', token, refreshToken: 'psr-' + token }))
  }, { key, token: tokens.A, legacy: options.legacy, id })
  await a.goto(origin + '/dashboard', { waitUntil: 'domcontentloaded' })
  await a.waitForFunction(() => window.__phase10Auth && window.__phase10Auth.getState().sessionStatus !== 'initializing')
  const b = await context.newPage(); await b.goto(origin + '/dashboard', { waitUntil: 'domcontentloaded' })
  await b.waitForFunction(() => window.__phase10Auth && window.__phase10Auth.getState().sessionStatus !== 'initializing')
  if (!options.noLocks && !options.noStorage) await Promise.all([a, b].map(page => page.getByText('我的应用', { exact: true }).waitFor()))
  const close = async () => { for (const name of ['refreshGate', 'contextGate', 'dataGate', 'old401Gate', 'logoutGate']) state[name]?.resolve(); await context.close(); for (const token of Object.values(tokens)) tokenOwners.delete(token) }
  return { context, a, b, state, tokens, close }
}
const status = page => page.evaluate(() => { const s = __phase10Auth.getState(); return { status: s.sessionStatus, version: s.sessionVersion, epoch: s.contextEpoch, sessionId: s.sessionId, org: s.profile?.organization.id ?? null, tokenPresent: Boolean(s.token) } })
const request = page => page.evaluate(() => __phase10Api.get('/api/phase10/expired').then(() => 'ok', e => e.code || e.name))
const startRequest = async (page, kind = 'expired', detach = false) => page.evaluate(({ kind, detach }) => {
  const c = new AbortController(); window.__phase10Controller = c; __phase10Transport.detachNext = detach
  window.__phase10Pending = __phase10Api.get('/api/phase10/' + kind, { signal: c.signal }).then(value => ({ ok: true, value }), e => ({ ok: false, name: e.name, code: e.code }))
}, { kind, detach })
const pending = page => page.evaluate(() => window.__phase10Pending)
const emit = (scenario, observed) => console.log('PHASE10_SESSION_TRACE ' + JSON.stringify({ scenario, ...observed }))
async function check(name, action) {
  if (process.env.PHASE10_BROWSER_FILTER && !new RegExp(process.env.PHASE10_BROWSER_FILTER).test(name)) return
  await action(); console.log('PASS ' + name); passed++
}
async function withPair(action, options) { const f = await fixture(options); try { await action(f); assert.deepEqual(f.state.errors, [], 'The actual host pages have no uncaught errors') } finally { await f.close() } }
try {
  await start(); browser = await launchBrowser()
  const sourcePaths = ['frontend/src/lib/session-coordinator.ts', 'frontend/src/stores/auth.ts', 'frontend/src/lib/api.ts', 'frontend/src/main.tsx', 'tests/phase10-session-browser.mjs']
  console.log('PHASE10_SESSION_SOURCE ' + JSON.stringify({ browserVersion: await browser.version(), files: sourcePaths.map(file => ({ path: file, sha256: createHash('sha256').update(readFileSync(path.join(root, file))).digest('hex') })) }))
  await check('legacy migration creates one atomic record shared by two pages', () => withPair(async f => {
    const records = await Promise.all([f.a, f.b].map(page => page.evaluate(key => ({ record: JSON.parse(localStorage.getItem(key)), legacyGone: !localStorage.getItem('platform-token') && !localStorage.getItem('platform-refresh-token') }), key)))
    assert.deepEqual(records[0], records[1]); assert(records[0].legacyGone); assert.equal(records[0].record.status, 'active')
    emit('legacy-migration', { sameRecord: true, legacyGone: true, pages: await Promise.all([f.a, f.b].map(status)) })
  }, { legacy: true }))
  await check('two expired pages consume the old refresh exactly once', () => withPair(async f => {
    await f.a.evaluate(() => { history.pushState(null, '', '/micro/work-orders/ticket-create'); dispatchEvent(new PopStateEvent('popstate')) })
    const frame = f.a.frameLocator('.micro-mount-zone iframe'); await frame.getByTestId('ticket-title').fill('同组织刷新保留草稿')
    const marker = await frame.locator('body').evaluate(() => { window.__phase10FrameMarker = crypto.randomUUID(); return window.__phase10FrameMarker })
    f.state.refreshGate = deferred(); await Promise.all([startRequest(f.a), startRequest(f.b)])
    await wait(() => f.state.refreshes.length === 1, 'First rotating refresh reached server'); f.state.refreshGate.resolve()
    const outcomes = await Promise.all([pending(f.a), pending(f.b)]); assert(outcomes.every(v => v.ok)); assert.equal(f.state.refreshes.length, 1)
    const pages = await Promise.all([f.a, f.b].map(status)); assert.equal(pages[0].sessionId, pages[1].sessionId); assert.equal(pages[0].version, pages[1].version)
    await wait(() => f.state.requests.some(row => row.path === '/api/oauth/session-token' && row.group === 'AR'), 'Real scheduled app-token renewal uses the latest platform pair')
    assert.equal(await frame.getByTestId('ticket-title').inputValue(), '同组织刷新保留草稿'); assert.equal(await frame.locator('body').evaluate(() => window.__phase10FrameMarker), marker)
    const messages = (await Promise.all([f.a, f.b].map(page => page.evaluate(() => __phase10Transport.broadcast)))).flat()
    assert(messages.length > 0); assert(messages.every(message => message.keys.length === 1 && message.keys[0] === 'version' && Number.isSafeInteger(message.version)))
    emit('concurrent-refresh', { consumed: f.state.refreshes.length, outcomes, pages, sameOrganizationDraftPreserved: true, iframePreserved: true, scheduledAppRenewalUsedLatestPair: true, versionOnlyBroadcasts: messages.length })
  }, { appTokenTtl: 5 }))
  await check('single page retains refresh single-flight and one retry', () => withPair(async f => {
    f.state.refreshGate = deferred(); const first = request(f.a), second = request(f.a)
    await wait(() => f.state.refreshes.length === 1, 'Single flight started'); f.state.refreshGate.resolve(); assert.deepEqual(await Promise.all([first, second]), ['ok', 'ok']); assert.equal(f.state.refreshes.length, 1)
    f.state.expiredAlways = true; await request(f.a); assert.equal(f.state.refreshes.length, 2)
    emit('single-flight-once', { consumed: 2, apiRequests: f.state.requests.filter(row => row.path === '/api/phase10/expired').length })
  }))
  await check('late old 401 reuses winner without consuming refresh again', () => withPair(async f => {
    f.state.old401Gate = deferred(); await startRequest(f.a); await wait(() => f.state.requests.some(row => row.path === '/api/phase10/expired'), 'Old request sent')
    const gate = f.state.old401Gate; f.state.old401Gate = null; assert.equal(await request(f.b), 'ok'); gate.resolve(); assert((await pending(f.a)).ok); assert.equal(f.state.refreshes.length, 1)
    emit('late-401', { consumed: 1, pages: await Promise.all([f.a, f.b].map(status)) })
  }))
  for (const mode of ['success', 'reject']) await check(`replacement login survives old refresh ${mode}`, () => withPair(async f => {
    f.state.refreshGate = deferred(); f.state.refreshMode = mode; await f.a.evaluate(() => { __phase10Transport.detachRefresh = true })
    await startRequest(f.a); await wait(() => f.state.refreshes.length === 1, 'Refresh entered')
    assert(await f.a.evaluate(token => __phase10Auth.getState().setSession(token, 'psr-' + token), f.tokens.C))
    await wait(async () => (await status(f.a)).org === 'org-C' && (await status(f.b)).org === 'org-C', 'Both pages adopted replacement identity')
    f.state.refreshGate.resolve(); await pending(f.a)
    await wait(async () => (await f.a.evaluate(() => __phase10Transport.events)).some(event => event.path === '/oauth/token' && event.completedAfterAbort), 'Old native refresh really completed after cancellation')
    assert.equal(await f.a.evaluate(token => JSON.parse(localStorage.getItem('platform-session-v1')).token === token, f.tokens.C), true)
    emit('replacement-' + mode, { consumed: 1, pages: await Promise.all([f.a, f.b].map(status)), transport: await f.a.evaluate(() => __phase10Transport.events) })
  }))
  await check('one-page logout clears both pages and old organization views', () => withPair(async f => {
    await f.a.evaluate(() => __phase10Logout()); await wait(async () => (await status(f.b)).status === 'anonymous', 'Sibling logout observed')
    assert((await Promise.all([f.a, f.b].map(status))).every(s => !s.tokenPresent && s.org === null)); assert.equal(await f.b.getByText('我的应用', { exact: true }).count(), 0)
    emit('shared-logout', { pages: await Promise.all([f.a, f.b].map(status)) })
  }))
  await check('captured old logout cannot clear a replacement login', () => withPair(async f => {
    const fence = await f.a.evaluate(() => __phase10Fence()); await f.b.evaluate(token => __phase10Auth.getState().setSession(token, 'psr-' + token), f.tokens.C)
    assert.equal(await f.a.evaluate(old => __phase10Logout(old), fence), false); assert.equal((await status(f.b)).status, 'active')
    assert.equal(f.state.requests.filter(row => row.path === '/api/auth/logout').length, 0)
    emit('logout-fence', { serverLogoutCalls: 0, pages: await Promise.all([f.a, f.b].map(status)) })
  }))
  await check('shared organization switch aborts reads and sockets and drops actual work-order drafts', () => withPair(async f => {
    await wait(() => f.state.connections.filter(c => c.org === 'A' && !c.disconnected).length >= 2, 'Both actual sockets authenticated')
    await f.a.evaluate(() => { history.pushState(null, '', '/micro/work-orders/ticket-create'); dispatchEvent(new PopStateEvent('popstate')) })
    const frame = f.a.frameLocator('.micro-mount-zone iframe'); await frame.getByTestId('ticket-title').fill('组织 A 未提交草稿')
    f.state.dataGate = deferred(); await startRequest(f.a, 'held', true); await wait(() => f.state.requests.some(row => row.path === '/api/phase10/held'), 'Old read pending')
    f.state.contextGate = deferred(); await f.b.evaluate(() => { window.__phase10Switching = __phase10Switch('org-B') })
    await wait(async () => (await status(f.a)).status === 'switching' && (await status(f.b)).status === 'switching', 'Both pages isolate the in-flight context intent')
    assert.equal(await f.a.locator('.micro-mount-zone iframe').count(), 0); assert.notEqual(new URL(f.a.url()).pathname, '/login'); assert.notEqual(new URL(f.b.url()).pathname, '/login')
    f.state.contextGate.resolve(); await f.b.evaluate(() => __phase10Switching)
    await wait(async () => (await status(f.a)).org === 'org-B' && (await status(f.b)).org === 'org-B', 'Both pages share organization B')
    assert.notEqual(new URL(f.a.url()).pathname, '/login'); assert.notEqual(new URL(f.b.url()).pathname, '/login')
    await wait(() => f.state.connections.filter(c => c.org === 'A').every(c => c.disconnected), 'Old sockets disconnected')
    f.state.dataGate.resolve(); assert.equal((await pending(f.a)).ok, false)
    await f.a.evaluate(() => { history.pushState(null, '', '/micro/work-orders/ticket-create'); dispatchEvent(new PopStateEvent('popstate')) })
    const fresh = f.a.frameLocator('.micro-mount-zone iframe'); await fresh.getByTestId('ticket-title').waitFor(); assert.equal(await fresh.getByTestId('ticket-title').inputValue(), '')
    await fresh.getByTestId('save-ticket').click()
    assert.equal(f.state.writes.length, 0); assert.equal(await f.a.getByText('组织 A 未提交草稿').count(), 0)
    const transport = await f.a.evaluate(() => __phase10Transport.events); assert(transport.some(event => event.detached && event.aborted && event.completedAfterAbort))
    emit('shared-org-boundaries', { pages: await Promise.all([f.a, f.b].map(status)), oldSocketsDisconnected: true, workOrderDraftDiscarded: true, businessWrites: 0, transport })
  }))
  await check('organization switch and refresh serialize through the same lock', () => withPair(async f => {
    f.state.refreshGate = deferred(); await startRequest(f.a); await wait(() => f.state.refreshes.length === 1, 'Refresh owns lock')
    const switching = f.b.evaluate(() => __phase10Switch('org-B').then(() => 'ok', e => e.code || e.name)); f.state.refreshGate.resolve(); await pending(f.a); assert.equal(await switching, 'ok')
    await wait(async () => (await status(f.a)).org === 'org-B' && (await status(f.b)).org === 'org-B', 'Serialized context converges'); assert.equal(f.state.refreshes.length, 1)
    assert.notEqual(new URL(f.a.url()).pathname, '/login'); assert.notEqual(new URL(f.b.url()).pathname, '/login')
    emit('refresh-context-lock', { consumed: 1, contextCalls: f.state.requests.filter(row => row.path === '/api/auth/context').length, pages: await Promise.all([f.a, f.b].map(status)) })
  }))
  await check('closing refresh owner after consumption releases lock and quarantines unknown result', () => withPair(async f => {
    f.state.refreshGate = deferred(); await startRequest(f.a); await wait(() => f.state.refreshes.length === 1, 'Refresh consumed before owner close')
    await f.a.close(); await f.b.evaluate(() => __phase10Auth.getState().reconcileSession())
    await wait(async () => (await status(f.b)).status === 'unknown', 'Abandoned intent quarantined under the real browser lock')
    await request(f.b); assert.equal(f.state.refreshes.length, 1); await f.b.getByText(/上次会话操作的结果无法确认/).waitFor()
    const locks = await f.b.evaluate(() => navigator.locks.query()); assert.equal(locks.held.filter(lock => lock.name === 'platform-session-coordinator-v1').length, 0)
    emit('closed-owner', { ownerActuallyClosed: f.a.isClosed(), consumed: 1, sibling: await status(f.b), heldSessionLocks: 0 })
  }))
  await check('unknown network result is never retried and explicit login recovers', () => withPair(async f => {
    f.state.refreshMode = 'network'; await request(f.a); await wait(async () => (await status(f.b)).status === 'unknown', 'Unknown propagated')
    await request(f.b); assert.equal(f.state.refreshes.length, 1); await f.b.evaluate(token => __phase10Auth.getState().setSession(token, 'psr-' + token), f.tokens.C)
    await wait(async () => (await status(f.a)).org === 'org-C', 'Explicit replacement restored pages')
    emit('network-unknown', { consumed: 1, explicitLoginRecovered: true, pages: await Promise.all([f.a, f.b].map(status)) })
  }))
  await check('missing Web Locks shows verified capability refusal', () => withPair(async f => {
    for (const page of [f.a, f.b]) { assert.equal((await status(page)).status, 'unavailable'); await page.getByText(/无法安全协调共享会话/).waitFor() }
    assert.equal(f.state.errors.length, 0); assert.equal(f.state.refreshes.length, 0); assert.equal(f.state.requests.filter(row => row.path === '/api/auth/profile').length, 0)
    emit('no-web-locks', { pages: await Promise.all([f.a, f.b].map(status)), platformProfileCalls: 0, refreshes: 0 })
  }, { noLocks: true }))
  await check('denied durable storage shows verified capability refusal', () => withPair(async f => {
    for (const page of [f.a, f.b]) { assert.equal((await status(page)).status, 'unavailable'); await page.getByText(/无法安全协调共享会话/).waitFor() }
    assert.equal(f.state.errors.length, 0); assert.equal(f.state.refreshes.length, 0); assert.equal(f.state.requests.filter(row => row.path === '/api/auth/profile').length, 0); emit('no-storage', { pages: await Promise.all([f.a, f.b].map(status)), refreshes: 0 })
  }, { noStorage: true }))
  await check('BroadcastChannel absence uses real storage-event synchronization', () => withPair(async f => {
    await f.a.evaluate(token => __phase10Auth.getState().setSession(token, 'psr-' + token), f.tokens.C)
    await wait(async () => (await status(f.b)).org === 'org-C', 'Storage event adopted replacement')
    const observed = await f.b.evaluate(() => ({ available: typeof BroadcastChannel !== 'undefined', events: __phase10Transport.storageEvents })); assert.equal(observed.available, false); assert(observed.events > 0)
    emit('storage-fallback', { ...observed, pages: await Promise.all([f.a, f.b].map(status)) })
  }, { noBroadcast: true }))
  await check('focus reconciliation recovers a page that missed version notifications', () => withPair(async f => {
    await f.b.evaluate(() => { window.__phase10DropStorage = true })
    await f.a.evaluate(token => __phase10Auth.getState().setSession(token, 'psr-' + token), f.tokens.C)
    assert.notEqual((await status(f.b)).org, 'org-C'); await f.b.evaluate(() => dispatchEvent(new Event('focus')))
    await wait(async () => (await status(f.b)).org === 'org-C', 'Focus reread durable latest record')
    const messages = await f.a.evaluate(() => __phase10Transport.broadcast); assert(messages.every(message => message.keys.length === 1 && message.keys[0] === 'version'))
    emit('focus-reconciliation', { missedStorage: true, focusRecovered: true, pages: await Promise.all([f.a, f.b].map(status)) })
  }, { missedEvents: true }))
  await check('stale page handlers never submit old drafts with a newly selected organization', () => withPair(async f => {
    await f.b.evaluate(() => { window.__phase10DropStorage = true })
    f.state.contextGate = deferred()
    await f.a.evaluate(() => { window.__phase10Switching = __phase10Switch('org-B').then(() => 'ok', e => e.code || e.name) })
    await wait(() => f.state.requests.filter(row => row.path === '/api/auth/context').length === 1, 'Context intent owns the real lock')
    assert.equal((await status(f.b)).org, 'org-A', 'Sibling really missed the intent notification')
    await f.b.evaluate(() => {
      window.__phase10StaleWrite = __phase10Api.post('/api/phase10/stale-write', { title: '旧组织 A 草稿' }).then(() => 'unsafe-success', e => e.name)
      window.__phase10StaleSwitch = __phase10Switch('org-A').then(() => 'unsafe-success', e => e.name)
    })
    f.state.contextGate.resolve(); assert.equal(await f.a.evaluate(() => __phase10Switching), 'ok')
    assert.equal(await f.b.evaluate(() => __phase10StaleWrite), 'AbortError')
    assert.equal(await f.b.evaluate(() => __phase10StaleSwitch), 'AbortError')
    const writes = f.state.requests.filter(row => row.path === '/api/phase10/stale-write')
    assert.equal(writes.length, 0); assert.equal(f.state.requests.filter(row => row.path === '/api/auth/context').length, 1)
    await f.b.evaluate(() => dispatchEvent(new Event('focus')))
    await wait(async () => (await status(f.b)).org === 'org-B', 'Stale sibling subsequently adopts safe current context')
    emit('stale-handler-fence', { missedIntent: true, staleDraftRequests: 0, contextRequests: 1, pages: await Promise.all([f.a, f.b].map(status)) })
  }, { missedEvents: true }))
  console.log(`Phase 10 session browser: ${passed} check groups passed.`)
} catch (error) { console.error(error.stack || error); process.exitCode = 1 }
finally {
  await browser?.close(); await io?.close(); if (socketHttp?.listening) await new Promise(resolve => socketHttp.close(resolve))
  if (vite && vite.exitCode === null) { vite.kill(); await Promise.race([new Promise(resolve => vite.once('exit', resolve)), pause(3000)]) }
  if (buildOutput) rmSync(path.dirname(buildOutput), { recursive: true, force: true })
}
