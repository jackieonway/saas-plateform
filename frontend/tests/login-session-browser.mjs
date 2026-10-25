/** Real LoginPage, router and session gate, with isolated HTTP fixtures. */
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { mkdir } from 'node:fs/promises'
import { launchBrowser } from './browser/runtime.mjs'
import { fixtureUuid, installLegacyAuthMode, profileWithIdentity } from './browser/identity-fixture.mjs'

const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const bindHost = process.platform === 'win32' ? '::1' : '127.0.0.1'
const pause = ms => new Promise(resolve => setTimeout(resolve, ms))
async function waitState(predicate, message, timeout = 12_000) {
  const deadline = Date.now() + timeout
  while (!await predicate() && Date.now() < deadline) await pause(20)
  assert(await predicate(), message)
}
const ORG_A = fixtureUuid('login-org-A'), ORG_B = fixtureUuid('login-org-B')
const user = { id: fixtureUuid('login-user'), username: 'login-fixture', realName: '登录回归用户', email: '',
  phone: '', gender: 'UNKNOWN', avatar: '', isSuperAdmin: false }
const tenant = { id: fixtureUuid('login-tenant'), name: '登录回归租户', code: 'login', isPlatform: false }
const organization = id => ({ id, name: '登录回归组织 ' + (id === ORG_A ? 'A' : 'B'), path: `/${id}/` })
const membership = (id, memberType = 'EMPLOYEE') => ({ userId: user.id, tenantId: tenant.id, tenantName: tenant.name,
  organizationId: id, organizationName: organization(id).name, organizationPath: `/${id}/`,
  membershipId: fixtureUuid(`membership:${user.id}:${id}:${memberType}`), memberType, identityVersion: '1', status: 'ENABLED',
  consoleSelectable: memberType === 'EMPLOYEE', selectionDisabledReason: memberType === 'CUSTOMER' ? 'CLIENT_MEMBER_TYPE_NOT_ALLOWED' : null })
const memberships = () => [membership(ORG_A), membership(ORG_A, 'CUSTOMER'), membership(ORG_B)]
const system = { id: fixtureUuid('login-system'), code: 'system', name: '系统管理', icon: 'LayoutGrid',
  basePath: '/system', entryType: 'HTML', runtimeMode: 'TRUSTED' }
const profile = (id, patch = {}) => profileWithIdentity({ user, tenant, organization: organization(id), applications: [system],
  memberships: memberships().map(member => member.organizationId === id && member.memberType === 'EMPLOYEE' ? { ...member, ...patch } : member), platformContext: false }, patch)
let browser, vite, origin, startup = '', passed = 0
async function startServer() {
  const port = await new Promise((resolve, reject) => {
    const server = net.createServer().on('error', reject)
    server.listen(0, bindHost, () => { const port = server.address().port; server.close(error => error ? reject(error) : resolve(port)) })
  })
  origin = `http://${bindHost.includes(':') ? '[' + bindHost + ']' : bindHost}:${port}`
  vite = spawn(process.execPath, [path.join(workspace, 'node_modules/vite/bin/vite.js'), '--host', bindHost,
    '--port', String(port), '--strictPort'], { cwd: workspace, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
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

async function check(returnTo, run) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1100 } })
  await installLegacyAuthMode(context)
  const page = await context.newPage()
  page.setDefaultTimeout(12_000)
  const state = { org: ORG_A, requests: [], errors: [], expirePermissions: false, memberPatch: {}, accessToken: null }
  page.on('pageerror', error => state.errors.push(error.message))
  await context.routeWebSocket(() => true, () => {})
  // Only expose actual collaborators. Keep the production React tree intact;
  // login below always starts anonymous and uses the real credential form.
  await context.route(url => url.origin === origin && url.pathname === '/src/main.tsx', async route => {
    const response = await route.fetch()
    const body = (await response.text()).replace('ReactDOM.createRoot',
      "import { authApi as loginAuthApi } from '/src/api/auth.ts';\nwindow.__loginAuth = sessionStore;\nwindow.__loginApi = loginAuthApi;\nwindow.__loginQueryClient = queryClient;\nReactDOM.createRoot")
    assert.match(body, /window\.__loginAuth/)
    await route.fulfill({ response, body })
  })
  await context.route(url => url.origin === origin && (url.pathname.startsWith('/api/') || url.pathname === '/oauth/token'), async route => {
    const request = route.request(), url = new URL(request.url())
    const record = { path: url.pathname, method: request.method() }
    state.requests.push(record)
    const reply = (body, status = 200) => { record.status = status; return route.fulfill({ status,
      contentType: 'application/json', body: JSON.stringify(body) }) }
    if (url.pathname === '/api/platform/branding') return reply({ platformName: '登录回归平台', loginTitle: '登录' })
    if (url.pathname === '/api/auth/login') {
      assert.deepEqual(request.postDataJSON(), { username: user.username, password: 'fixture-password' })
      return reply({ ticket: 'login-fixture-ticket', memberships: memberships() })
    }
    if (url.pathname === '/api/auth/context') {
      const body = request.postDataJSON()
      if (state.requests.filter(row => row.path === url.pathname).length === 1) {
        assert.equal(request.headers().authorization, 'Bearer login-fixture-ticket')
        assert.deepEqual(body, { organizationId: ORG_A, membershipId: membership(ORG_A).membershipId, userId: user.id })
      } else {
        assert.equal(body.organizationId, ORG_B)
        assert.equal(body.membershipId, profile(ORG_B, state.memberPatch).membershipId)
        assert.equal(request.headers().authorization, `Bearer ${state.accessToken}`)
      }
      state.org = body.organizationId
      state.accessToken = 'login-fixture-' + profile(state.org, state.memberPatch).membershipId
      return reply({ ...profile(state.org, state.memberPatch), accessToken: state.accessToken, refreshToken: 'login-refresh-' + state.org })
    }
    if (url.pathname === '/api/auth/profile') {
      assert.equal(request.headers().authorization, `Bearer ${state.accessToken}`, 'Profile uses the committed platform session')
      return reply(profile(state.org, state.memberPatch))
    }
    if (url.pathname === '/api/auth/permissions') {
      if (state.expirePermissions) { state.expirePermissions = false; return reply({ message: 'fixture access expired' }, 401) }
      return reply({ permissions: [], roles: [] })
    }
    if (url.pathname === '/oauth/token') {
      const body = new URLSearchParams(request.postData())
      assert.equal(body.get('grant_type'), 'refresh_token')
      state.accessToken = 'login-fixture-renewed'
      return reply({ access_token: state.accessToken, refresh_token: 'login-refresh-renewed' })
    }
    if (url.pathname === '/api/auth/menus') return reply({ menus: [{ id: fixtureUuid('login-dashboard'), parentId: null,
      type: 'MENU', title: '登录回归仪表盘', path: '/dashboard', icon: 'LayoutDashboard', sortOrder: 1, visible: true, children: [] }] })
    if (url.pathname === '/api/auth/logout') return reply({ ok: true })
    if (url.pathname === '/api/notifications/prefs') return reply({ dndEnabled: false, mutedTypes: [] })
    if (url.pathname === '/api/notifications/unread-count') return reply({ count: 0 })
    if (url.pathname === '/api/notifications/socket-context') return reply({ userId: user.id, tenantId: tenant.id,
      organizationId: state.org, sessionVersion: 0 })
    if (['/api/notifications', '/api/auth/login-history'].includes(url.pathname)) return reply({ records: [], total: 0, page: 1, size: 10 })
    if (url.pathname === '/api/auth/my-sessions') return reply({ sessions: [], summary: { active: 0, revoked: 0 } })
    if (url.pathname === '/api/monitor/overview') return reply({ code: 'FORBIDDEN', message: 'fixture monitor denied' }, 403)
    return reply({ message: 'Unconfigured fixture ' + url.pathname }, 403)
  })
  try {
    await page.goto(origin + '/login' + (returnTo ? '?return=' + encodeURIComponent(returnTo) : ''), { waitUntil: 'domcontentloaded' })
    assert.equal(await page.evaluate(() => window.__loginAuth.getState().token), null, 'Login starts without injected credentials')
    await page.locator('#username').fill(user.username)
    await page.getByPlaceholder('请输入密码', { exact: true }).fill('fixture-password')
    await page.locator('form button[type=submit]').click()
    await page.getByRole('button').filter({ hasText: tenant.name }).click()
    const employee = page.getByRole('button').filter({ hasText: organization(ORG_A).name }).filter({ hasText: '员工' })
    const customer = page.getByRole('button').filter({ hasText: organization(ORG_A).name }).filter({ hasText: '客户' })
    assert.equal(await customer.isDisabled(), true, 'A customer in the same user/organization remains visible but cannot enter the console')
    const screenshots = path.join(workspace, 'target/phase05-browser')
    await mkdir(screenshots, { recursive: true })
    await page.screenshot({ path: path.join(screenshots, 'login-identities-desktop.png'), fullPage: true })
    await page.setViewportSize({ width: 390, height: 844 })
    await page.screenshot({ path: path.join(screenshots, 'login-identities-mobile.png'), fullPage: true })
    await page.setViewportSize({ width: 1440, height: 1100 })
    await employee.click()
    await waitState(() => state.requests.some(row => row.path === '/api/auth/context' && row.status === 200), 'Credential context was accepted')
    await waitState(() => state.requests.some(row => row.path === '/api/auth/profile'), 'Accepted login mounts the protected layout and requests profile')
    await page.waitForURL(url => url.pathname + url.search + url.hash === (returnTo || '/dashboard'))
    await page.getByRole('complementary', { name: '应用导航', exact: true }).waitFor()
    assert.equal(await page.locator('#username').count(), 0, 'Router DOM leaves the login form')
    await run({ page, state })
    assert.deepEqual(state.errors, [], 'No uncaught production React errors')
    passed++
    console.log('PASS login session navigation ' + (returnTo || 'default dashboard'))
  } catch (error) {
    console.error('LOGIN_SESSION_TRACE ' + JSON.stringify({ returnTo, path: new URL(page.url()).pathname,
      loginFormVisible: await page.locator('#username').isVisible().catch(() => false),
      requests: state.requests, pageErrors: state.errors }))
    throw error
  } finally { await context.close() }
}

try {
  await startServer()
  browser = await launchBrowser()
  await check('/profile?login-check=1#security', async ({ page, state }) => {
    const draft = page.getByPlaceholder('请输入当前密码', { exact: true })
    await draft.fill('private-org-A-draft')
    await draft.evaluate(node => { window.__loginDraft = node })
    const before = await page.evaluate(() => {
      window.__loginQueryClient.setQueryData(['login-private-draft'], 'private-org-A-cache')
      const { sessionId, contextEpoch, sessionVersion } = window.__loginAuth.getState()
      return { sessionId, contextEpoch, sessionVersion }
    })
    state.expirePermissions = true
    await page.evaluate(() => window.__loginApi.permissions())
    const renewed = await page.evaluate(() => {
      const { sessionId, contextEpoch, sessionVersion } = window.__loginAuth.getState()
      return { sessionId, contextEpoch, sessionVersion, cached: window.__loginQueryClient.getQueryData(['login-private-draft']) }
    })
    assert.equal(renewed.sessionId, before.sessionId)
    assert.equal(renewed.contextEpoch, before.contextEpoch)
    assert(renewed.sessionVersion > before.sessionVersion)
    assert.equal(renewed.cached, 'private-org-A-cache')
    assert.equal(await draft.inputValue(), 'private-org-A-draft', 'Ordinary refresh preserves same-context form state')
    assert(await draft.evaluate(node => node === window.__loginDraft), 'Refresh keeps the same form node')
    assert.equal(state.requests.filter(row => row.path === '/oauth/token').length, 1)
    await page.evaluate(({ organizationId, membershipId }) => window.__loginApi.switchContext(organizationId, membershipId, window.__loginAuth.getState().token), membership(ORG_B))
    await waitState(async () => await page.evaluate(org => window.__loginAuth.getState().profile?.organization.id === org, ORG_B), 'New organization profile mounts')
    assert.equal(await draft.inputValue(), '', 'Organization change physically retires the prior form')
    assert(await draft.evaluate(node => node !== window.__loginDraft && !window.__loginDraft.isConnected), 'Old private form node is unmounted')
    assert.equal(await page.evaluate(() => window.__loginQueryClient.getQueryData(['login-private-draft'])), undefined, 'Organization change discards prior cache')
    await draft.fill('same-organization-old-member-draft')
    const prior = await page.evaluate(() => ({ key: window.__loginAuth.getState().contextKey, epoch: window.__loginAuth.getState().contextEpoch }))
    state.memberPatch = { membershipId: fixtureUuid('login-org-B-replacement'), identityVersion: '2' }
    await page.evaluate(({ organizationId, membershipId }) => window.__loginApi.switchContext(organizationId, membershipId, window.__loginAuth.getState().token), profile(ORG_B, state.memberPatch).memberships.find(member => member.organizationId === ORG_B))
    await waitState(async () => await page.evaluate(id => window.__loginAuth.getState().profile?.membershipId === id, state.memberPatch.membershipId), 'Same user and organization adopt the exact replacement membership')
    assert.equal(await draft.inputValue(), '', 'Same-organization member replacement retires the previous draft')
    const replaced = await page.evaluate(() => ({ key: window.__loginAuth.getState().contextKey, epoch: window.__loginAuth.getState().contextEpoch }))
    assert.notEqual(replaced.key, prior.key); assert(replaced.epoch > prior.epoch)
    assert.equal(new URL(page.url()).pathname, '/profile')
    await page.getByRole('complementary', { name: '菜单导航', exact: true }).getByRole('button').filter({ hasText: '登录回归仪表盘' }).click()
    await page.waitForURL(url => url.pathname === '/dashboard')
    await page.getByText('我的应用', { exact: true }).waitFor()
    console.log('PASS login refresh preserves draft; organization change clears form/cache and router still navigates')
  })
  await check(null, async ({ page, state }) => {
    await page.getByText('我的应用', { exact: true }).waitFor()
    await page.getByRole('button', { name: '用户菜单', exact: true }).click()
    await page.getByRole('menuitem', { name: '退出本平台', exact: true }).click()
    await page.waitForURL(url => url.pathname === '/login' && url.searchParams.get('signedOut') === '1')
    await page.locator('#username').waitFor()
    assert.equal(await page.getByRole('complementary', { name: '应用导航', exact: true }).count(), 0)
    assert.equal(await page.evaluate(() => window.__loginAuth.getState().token), null)
    assert(state.requests.some(row => row.path === '/api/auth/logout' && row.status === 200))
    console.log('PASS logout returns to the actual login form')
  })
  console.log(`Login session browser checks: ${passed} passed`)
} finally {
  if (browser) await browser.close()
  if (vite && vite.exitCode === null) { vite.kill(); await pause(100) }
}
