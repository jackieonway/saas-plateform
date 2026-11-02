/** Real Vite/React/Edge interactions with isolated HTTP fixtures; no backend credentials. */
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { mkdir } from 'node:fs/promises'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { launchBrowser } from './browser/runtime.mjs'
import { fixtureUuid, profileWithIdentity, identityForProfile, installSession, sessionRecord } from './browser/identity-fixture.mjs'

const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const screenshotDir = path.resolve(process.env.MICRO_SETUP_SCREENSHOT_DIR || path.join(workspace, 'local-micro-setup-qa'))
const bindHost = process.env.MICRO_SETUP_BROWSER_BIND || (process.platform === 'win32' ? '::1' : '127.0.0.1')
const pause = ms => new Promise(resolve => setTimeout(resolve, ms))
const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done }); return { promise, resolve } }
const ids = {
  system: '00000000-0000-7000-8000-000000000001',
  app: '00000000-0000-7000-8000-000000000002',
  tenant: '00000000-0000-7000-8000-000000000003',
  org: '00000000-0000-7000-8000-000000000004',
  role: '00000000-0000-7000-8000-000000000005',
  menu: '00000000-0000-7000-8000-000000000006',
  release: '00000000-0000-7000-8000-000000000007',
}
const system = { id: ids.system, code: 'system', name: '系统管理', icon: 'Settings2', basePath: '/system', status: 'ENABLED', menuCount: 1, entryType: 'HTML', runtimeMode: 'TRUSTED', sortOrder: 0 }
const application = { id: ids.app, code: 'crm', name: '客户管理', icon: 'Contact', basePath: '/micro/crm', status: 'ENABLED', menuCount: 1, entryType: 'HTML', runtimeMode: 'TRUSTED', entryUrl: '/sub-apps/crm/unreleased/index.html', sortOrder: 1 }
const actor = { user: fixtureUuid('micro-setup-user'), tenant: fixtureUuid('micro-setup-platform'), organization: fixtureUuid('micro-setup-root') }
const permissions = ['system:application', 'system:tenant', 'system:organization', 'system:authorization', 'system:role', 'system:menu']
const pageResult = records => ({ records, total: records.length, page: 1, size: 200 })
const menu = app => ({ id: ids.menu, parentId: null, applicationId: app.id, type: 'MENU', title: app.name + '首页', path: app.basePath, icon: 'Home', permission: '', sortOrder: 0, visible: true, linkUrl: '', children: [], authorized: false })
let browser, vite, origin, startup = ''

async function startServer() {
  const port = await new Promise((resolve, reject) => {
    const server = net.createServer().on('error', reject)
    server.listen(0, bindHost, () => { const available = server.address().port; server.close(error => error ? reject(error) : resolve(available)) })
  })
  origin = `http://${bindHost.includes(':') ? '[' + bindHost + ']' : bindHost}:${port}`
  vite = spawn(process.execPath, [path.join(workspace, 'node_modules/vite/bin/vite.js'), '--host', bindHost, '--port', String(port), '--strictPort'], {
    cwd: workspace, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
  })
  let spawnError
  vite.on('error', error => { spawnError = error })
  for (const stream of [vite.stdout, vite.stderr]) stream.on('data', chunk => { startup = (startup + chunk).slice(-6000) })
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    if (spawnError) throw spawnError
    if (vite.exitCode !== null) throw new Error('Owned Vite stopped: ' + startup)
    if (startup.includes('ready in')) return
    await pause(100)
  }
  throw new Error('Owned Vite did not become ready: ' + startup)
}

async function fixture(run, options = {}) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1100 } })
  const page = await context.newPage()
  page.setDefaultTimeout(15_000)
  const state = { apps: [structuredClone(system), structuredClone(application)], releases: [], deployments: [], urlEntries: [], grants: [], creates: [], errors: [], routeErrors: [], requests: [], accessConfigured: false, ...options }
  const profile = () => profileWithIdentity({
    user: { id: actor.user, username: 'fixture-admin', realName: '平台管理员', email: '', phone: '', avatar: '', gender: 'UNKNOWN', isSuperAdmin: true },
    tenant: { id: actor.tenant, name: '平台管理', code: 'admin-tenant', isPlatform: true }, platformContext: true,
    organization: { id: actor.organization, name: '平台管理组织', path: `/${actor.organization}/` }, applications: state.micro ? [system, state.apps[1]] : [system],
    memberships: [{ tenantId: actor.tenant, tenantName: '平台管理', organizationId: actor.organization, organizationName: '平台管理组织', organizationPath: `/${actor.organization}/` }],
  })
  page.on('pageerror', error => state.errors.push(error.message))
  const proof = identityForProfile(profile())
  await installSession(context, 'micro-setup-fixture-session', proof)
  await context.addInitScript(({ namespace, systemId }) => {
    if (window.parent !== window) return
    localStorage.setItem('platform-theme', 'light')
    localStorage.setItem(`platform-current-app-v2:${namespace}`, systemId)
  }, { namespace: sessionRecord('micro-setup-fixture-session', proof).contextKey, systemId: ids.system })
  await context.routeWebSocket(/\/socket\.io\//, socket => socket.close())
  const fulfill = (route, value, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(value) })
  await context.route(url => url.origin === origin && url.pathname.startsWith('/api/'), async route => {
    const request = route.request(), url = new URL(request.url()), method = request.method()
    try {
      state.requests.push({ method, path: url.pathname, query: url.search })
      if (url.pathname === '/api/platform/branding') return fulfill(route, { platformName: '企业工作台', platformDescription: '统一应用工作空间', logoUrl: '', loginTitle: '登录', loginDescription: '', footerText: '' })
      assert.equal(request.headers().authorization, /^\/api\/apps\//.test(url.pathname) ? 'Bearer micro-application-fixture-token' : 'Bearer micro-setup-fixture-session')
      if (url.pathname === '/api/auth/profile') return fulfill(route, profile())
      if (url.pathname === '/api/auth/permissions') return fulfill(route, { permissions, roles: [] })
      if (url.pathname === '/api/auth/menus') return fulfill(route, state.micro && url.searchParams.get('applicationId') === ids.app ? [menu(state.apps[1])] : [{ id: 'applications-menu', parentId: null, type: 'MENU', title: '应用管理', path: '/system/applications', icon: 'Boxes', permission: 'system:application', sortOrder: 0, visible: true, children: [] }])
      if (url.pathname === '/api/apps/session-token') {
        assert.equal(request.postDataJSON().appCode, 'crm')
        state.issuedTokens = (state.issuedTokens || 0) + 1
        return fulfill(route, { accessToken: 'micro-application-fixture-token', tokenType: 'Bearer', expiresIn: 120, audience: 'crm' })
      }
      if (url.pathname === '/api/apps/crm/context') {
        state.contextRequests = (state.contextRequests || 0) + 1
        const app = state.apps[1]
        return fulfill(route, { contractVersion: '1.0', contextVersion: 'micro-layout-v1', appCode: 'crm', expiresAt: new Date(Date.now() + 60_000).toISOString(),
          application: { id: app.id, name: app.name, runtimeMode: app.runtimeMode, entryType: app.entryType, entryUrl: app.entryUrl, deploymentVersion: app.deploymentVersion, entryVersionId: app.entryVersionId },
          user: { id: actor.user, username: 'fixture-admin', displayName: '平台管理员' }, tenant: { id: actor.tenant, name: '平台管理', code: 'admin-tenant' },
          organization: { id: actor.organization, name: '平台管理组织', path: `/${actor.organization}/` }, organizationScopeIds: [actor.organization], roles: [], permissions: [], dataGrants: [], menus: [menu(app)] })
      }
      if (url.pathname === '/api/apps/crm/runtime-events') return fulfill(route, { ok: true })
      if (url.pathname === '/api/notifications/unread-count') return fulfill(route, { count: 0 })
      if (url.pathname === '/api/notifications/prefs') return fulfill(route, { mutedTypes: [], dndEnabled: false, dndStart: '22:00', dndEnd: '08:00' })
      if (url.pathname === '/api/search') return fulfill(route, { users: [], tenants: [], organizations: [], roles: [] })
      if (url.pathname === '/api/applications') {
        if (method === 'GET') return fulfill(route, state.apps)
        const body = request.postDataJSON()
        state.creates.push(body)
        const created = { ...body, id: ids.app, description: body.description || '', menuCount: body.autoCreateMenu ? 1 : 0 }
        state.apps = [system, created]
        return fulfill(route, created, 201)
      }
      const metadata = /^\/api\/applications\/([^/]+)$/.exec(url.pathname)
      if (metadata && method === 'PUT') {
        const app = state.apps.find(item => item.id === metadata[1])
        assert(app)
        Object.assign(app, request.postDataJSON())
        return fulfill(route, { ok: true })
      }
      const setup = /^\/api\/applications\/([^/]+)\/setup$/.exec(url.pathname)
      if (setup) {
        if (state.setupGate) await state.setupGate.promise
        const app = state.apps.find(item => item.id === setup[1]) || application
        const environment = url.searchParams.get('environment') || 'development'
        const deployment = state.deployments.find(item => item.applicationId === app.id && item.environment === environment)
        const entry = state.urlEntries.find(item => item.applicationId === app.id && item.environment === environment)
        const target = { tenantId: url.searchParams.get('tenantId'), organizationId: url.searchParams.get('organizationId'), roleId: url.searchParams.get('roleId') }
        return fulfill(route, {
          applicationId: app.id, environment, applicationEnabled: app.status === 'ENABLED', entryType: app.entryType, runtimeMode: app.runtimeMode,
          releaseRequired: app.entryType === 'HTML' && !entry, activeVersion: entry ? null : deployment?.version || null,
          releaseValidated: app.entryType === 'HTML' && app.runtimeMode === 'TRUSTED' && Boolean(entry || deployment),
          entrySource: entry ? 'URL' : deployment ? 'RELEASE' : 'LEGACY', urlEntryConfigured: !!entry, proxyEnabled: entry?.proxyEnabled ?? false,
          menuCount: app.menuCount, entryMenuCount: app.menuCount, canConfigureAccess: state.canConfigureAccess ?? true,
          access: Object.values(target).every(Boolean) ? { ...target, tenantAuthorized: state.accessConfigured, organizationAuthorized: state.accessConfigured, roleAuthorized: state.accessConfigured, assignedUserCount: state.accessConfigured ? 1 : 0 } : null,
        })
      }
      const urlEntry = /^\/api\/applications\/([^/]+)\/url-entry$/.exec(url.pathname)
      if (urlEntry) {
        const environment = url.searchParams.get('environment') || 'development'
        const matches = item => item.applicationId === urlEntry[1] && item.environment === environment
        if (method === 'GET') return fulfill(route, state.urlEntries.find(matches) || null)
        state.urlEntries = state.urlEntries.filter(item => !matches(item))
        if (method === 'DELETE') return fulfill(route, { ok: true })
        assert.equal(method, 'PUT')
        const body = request.postDataJSON()
        const entry = { applicationId: urlEntry[1], environment, ...body, browserEntryUrl: `/micro-proxy/crm/`, updatedAt: new Date().toISOString() }
        state.urlEntries.push(entry)
        return fulfill(route, entry)
      }
      if (/\/setup\/access$/.test(url.pathname)) {
        const body = request.postDataJSON()
        state.grants.push(body)
        if (state.grantGate) await state.grantGate.promise
        state.accessConfigured = true
        return fulfill(route, { ok: true })
      }
      if (/\/releases\/upload$/.test(url.pathname)) {
        const app = state.apps.find(item => url.pathname.includes(item.id)) || application
        const release = { id: ids.release, applicationId: app.id, environment: url.searchParams.get('environment'), version: url.searchParams.get('version'), entryType: 'HTML', entryUrl: `/sub-apps/${app.code}/versions/${url.searchParams.get('version')}/index.html`, healthUrl: null, resources: [], validationStatus: 'PASSED', validationResult: null, checkedAt: new Date().toISOString(), active: false, createdAt: new Date().toISOString() }
        state.releases.push(release)
        return fulfill(route, release, 201)
      }
      if (/\/releases$/.test(url.pathname)) return fulfill(route, state.releases.filter(item => item.environment === url.searchParams.get('environment')))
      if (/\/deployments$/.test(url.pathname)) return fulfill(route, state.deployments)
      if (/\/releases\/[^/]+\/activate$/.test(url.pathname)) {
        if (state.activateGate) await state.activateGate.promise
        const release = state.releases[0]
        assert(release)
        const deployment = { applicationId: release.applicationId, environment: release.environment, version: release.version, activeReleaseId: release.id, previousReleaseId: null, entryType: 'HTML', entryUrl: release.entryUrl, updatedAt: new Date().toISOString() }
        state.deployments = [deployment]
        state.urlEntries = state.urlEntries.filter(entry => entry.applicationId !== deployment.applicationId || entry.environment !== deployment.environment)
        state.releases[0].active = true
        return fulfill(route, deployment)
      }
      if (url.pathname === '/api/tenants/options') return fulfill(route, pageResult([{ id: ids.tenant, name: '示例企业', code: 'demo', status: 'ENABLED', isPlatform: false }]))
      if (url.pathname === '/api/organizations/tree' || /\/setup\/organizations$/.test(url.pathname)) return fulfill(route, [{ id: ids.org, tenantId: ids.tenant, name: '销售团队', code: 'sales', status: 'ENABLED', parentId: null, expiresAt: null, children: [] }])
      if (url.pathname === '/api/roles') {
        if (state.roleGate) await state.roleGate.promise
        if (state.failRoles) return fulfill(route, { code: 'SERVICE_UNAVAILABLE', message: '角色候选暂不可用' }, 503)
        return fulfill(route, pageResult([{ id: ids.role, tenantId: ids.tenant, organizationId: ids.org, name: '客户经理', code: 'crm-user', status: 'ENABLED', builtIn: false, userCount: 1 }]))
      }
      if (/\/api\/tenants\/[^/]+\/authorizations$/.test(url.pathname)) {
        const app = state.apps.find(item => item.id === ids.app) || application
        return fulfill(route, { applications: [{ ...app, authorized: false }], menus: { [app.id]: app.menuCount ? [menu(app)] : [] } })
      }
      if (/\/api\/menus\/tree$/.test(url.pathname)) return fulfill(route, [menu(application)])
      throw new Error(`Unexpected API: ${method} ${url.pathname}`)
    } catch (error) {
      state.routeErrors.push(error.message)
      try { await fulfill(route, { code: 'FIXTURE_ERROR', message: error.message }, 500) } catch { /* Context may close after a test error. */ }
    }
  })
  try {
    await page.goto(origin + (state.startPath || '/system/applications'))
    if (state.micro) await page.locator('#app-scroll').waitFor()
    else await page.getByRole('heading', { name: '应用管理', exact: true }).waitFor()
    await run({ page, state })
    assert.deepEqual(state.errors, [], 'The actual page must have no browser exception.')
    assert.deepEqual(state.routeErrors, [], 'All API traffic must satisfy the isolated contract.')
  } catch (error) {
    await mkdir(screenshotDir, { recursive: true })
    await page.screenshot({ path: path.join(screenshotDir, 'micro-setup-failure.png'), fullPage: true, animations: 'disabled' })
    console.error('FIXTURE DIAGNOSTICS ' + JSON.stringify({ requests: state.requests, errors: state.errors, routeErrors: state.routeErrors, comboboxes: await page.locator('[role="combobox"]').evaluateAll(nodes => nodes.map(node => ({ text: node.textContent, label: node.getAttribute('aria-label'), labelledby: node.getAttribute('aria-labelledby'), disabled: node.disabled }))), text: (await page.locator('body').innerText()).slice(-5000) }))
    throw error
  } finally {
    state.setupGate?.resolve()
    state.grantGate?.resolve()
    state.activateGate?.resolve()
    state.roleGate?.resolve()
    await context.close()
  }
}

async function openCreate(page) {
  await page.getByRole('button', { name: /新建微应用|接入微应用/ }).first().click()
  await page.getByRole('heading', { name: '创建微应用', exact: true }).waitFor()
}
async function fillApplication(page, code = 'crm') {
  await page.getByLabel('应用名称', { exact: false }).fill('客户管理')
  await page.getByLabel('应用编码', { exact: false }).fill(code)
}
async function shot(page, name) {
  await mkdir(screenshotDir, { recursive: true })
  await page.locator('[data-sonner-toast]').first().waitFor({ state: 'hidden', timeout: 7000 }).catch(() => {})
  const file = path.join(screenshotDir, `micro-setup-${name}.png`)
  await page.screenshot({ path: file, fullPage: true, animations: 'disabled' })
  console.log('SCREENSHOT ' + file)
}
async function settledSheetLayout(page) {
  const layout = await page.getByRole('dialog').evaluate(async element => {
    const deadline = performance.now() + 5000
    let previous = '', stableFrames = 0, snapshot
    do {
      await new Promise(resolve => requestAnimationFrame(resolve))
      const rect = element.getBoundingClientRect(), style = getComputedStyle(element)
      const animations = element.getAnimations({ subtree: true }).filter(animation =>
        (animation.pending || animation.playState === 'running') && Number.isFinite(animation.effect?.getComputedTiming().endTime))
      snapshot = { bounds: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
        viewport: { width: window.innerWidth, height: window.innerHeight }, scrollWidth: document.documentElement.scrollWidth,
        computed: { width: style.width, maxWidth: style.maxWidth, transform: style.transform },
        animations: animations.map(animation => ({ name: animation.animationName ?? animation.transitionProperty ?? 'animation', playState: animation.playState, pending: animation.pending })) }
      const current = JSON.stringify(snapshot.bounds)
      stableFrames = animations.length === 0 && current === previous ? stableFrames + 1 : 0
      previous = current
      if (stableFrames >= 3) return { ...snapshot, stable: true }
    } while (performance.now() < deadline)
    return { ...snapshot, stable: false }
  })
  assert(layout.stable, 'The onboarding sheet must finish its finite animations and reach stable bounds: ' + JSON.stringify(layout))
  return layout
}
async function accessTarget(page) {
  await page.getByLabel('访问租户', { exact: true }).click()
  await page.getByRole('option', { name: '示例企业', exact: true }).click()
  await page.getByRole('combobox', { name: '访问组织', exact: true }).click()
  await page.getByText('销售团队', { exact: true }).click()
}

const scenarios = {
  simple_create: () => fixture(async ({ page }) => {
    await openCreate(page)
    const dialog = page.getByRole('dialog')
    assert.equal(await dialog.locator('input:visible').count(), 2)
    assert.equal(await page.getByLabel('访问路径', { exact: true }).isVisible(), false)
    assert.equal(await page.getByLabel('排序值', { exact: true }).isVisible(), false)
    await fillApplication(page)
    await dialog.getByText('/micro/crm', { exact: true }).waitFor()
    assert.doesNotMatch(await dialog.innerText(), /iframe|ESM/)
    await shot(page, 'create')
  }),
  advanced_settings: () => fixture(async ({ page }) => {
    await openCreate(page)
    await page.locator('summary').filter({ hasText: '更多设置' }).click()
    await page.getByLabel('访问路径', { exact: true }).waitFor({ state: 'visible' })
    await page.getByLabel('排序值', { exact: true }).waitFor({ state: 'visible' })
    assert.equal(await page.getByLabel('接入方式', { exact: true }).count(), 0)
    assert.equal(await page.getByLabel('加载方式', { exact: true }).count(), 0)
    assert.doesNotMatch(await page.getByRole('dialog').innerText(), /iframe|ESM/)
    assert.equal(await page.getByLabel('访问路径', { exact: true }).getAttribute('readonly'), '')
    await page.locator('summary').filter({ hasText: '更多设置' }).click()
    assert.equal(await page.getByLabel('排序值', { exact: true }).isVisible(), false)
  }),
  wizard_and_url: () => fixture(async ({ page, state }) => {
    await openCreate(page)
    await fillApplication(page)
    await page.getByRole('button', { name: '创建并继续', exact: false }).click()
    await page.getByRole('heading', { name: '配置应用入口', exact: true }).waitFor()
    await page.getByRole('navigation', { name: '应用接入步骤' }).waitFor()
    assert.equal(state.creates[0].autoCreateMenu, true)
    assert.equal(state.creates[0].runtimeMode, 'TRUSTED')
    assert.equal(await page.getByRole('dialog').count(), 1, 'Embedded release must use the one onboarding sheet.')
    await page.getByRole('button', { name: /已部署地址/ }).click()
    await page.getByLabel('HTML 入口 URL', { exact: false }).waitFor()
    assert.equal(await page.getByLabel('健康 URL（可选）', { exact: true }).isVisible(), false)
    await shot(page, 'publish-url')
  }),
  direct_url_and_environment: () => fixture(async ({ page, state }) => {
    await page.getByRole('heading', { name: '配置应用入口', exact: true }).waitFor()
    await page.getByLabel('上游入口地址', { exact: true }).fill('http://localhost:5173/')
    assert.equal(await page.getByRole('switch', { name: '通过平台同域代理', exact: true }).getAttribute('aria-checked'), 'true')
    await page.getByRole('button', { name: '保存并使用', exact: true }).click()
    await page.getByRole('button', { name: '恢复版本入口', exact: true }).waitFor()
    assert.equal(state.urlEntries.length, 1)
    assert.equal(state.urlEntries[0].environment, 'development')
    assert.equal(state.urlEntries[0].proxyEnabled, true)
    await page.getByRole('navigation', { name: '应用接入步骤' }).getByRole('button', { name: /接入检查/ }).click()
    await page.getByText('当前环境使用直接地址，页面与资源通过平台代理。', { exact: true }).waitFor()
    await page.getByLabel('接入环境', { exact: true }).click()
    await page.getByRole('option', { name: '预发环境', exact: true }).click()
    await page.getByText('填写 HTTP/HTTPS 入口即可接入，也可上传构建包或发布固定版本。', { exact: true }).waitFor()
    await page.getByRole('navigation', { name: '应用接入步骤' }).getByRole('button', { name: /应用入口/ }).click()
    await page.getByLabel('上游入口地址', { exact: true }).waitFor()
    assert.equal(await page.getByLabel('上游入口地址', { exact: true }).inputValue(), '')
    await page.getByLabel('接入环境', { exact: true }).click()
    await page.getByRole('option', { name: '开发环境', exact: true }).click()
    await page.getByRole('button', { name: '恢复版本入口', exact: true }).waitFor()
    assert.equal(await page.getByLabel('上游入口地址', { exact: true }).inputValue(), 'http://localhost:5173/')
    await shot(page, 'direct-url')
    await page.getByRole('button', { name: '恢复版本入口', exact: true }).click()
    await page.getByRole('button', { name: '恢复版本入口', exact: true }).waitFor({ state: 'hidden' })
    assert.equal(state.urlEntries.length, 0)
  }, { startPath: `/system/applications?setup=${ids.app}&step=release` }),
  legacy_reconfiguration: () => fixture(async ({ page, state }) => {
    await page.getByRole('heading', { name: '配置应用入口', exact: true }).waitFor()
    await page.getByText('等待重新配置 qiankun', { exact: true }).waitFor()
    await page.getByLabel('上游入口地址', { exact: true }).fill('https://new-app.example.com/')
    await page.getByRole('button', { name: '保存 qiankun 配置（保持停用）', exact: true }).click()
    await page.getByRole('button', { name: '保存并使用', exact: true }).waitFor()
    assert.equal(state.apps[1].entryType, 'HTML')
    assert.equal(state.apps[1].runtimeMode, 'TRUSTED')
    assert.equal(state.apps[1].status, 'DISABLED')
    assert.equal(state.urlEntries[0].entryUrl, 'https://new-app.example.com/')
    const writes = state.requests.filter(item => item.method === 'PUT')
    assert.deepEqual(writes.map(item => item.path), [`/api/applications/${ids.app}`, `/api/applications/${ids.app}/url-entry`])
  }, { startPath: `/system/applications?setup=${ids.app}&step=release`, apps: [system, { ...application, runtimeMode: 'IFRAME', status: 'DISABLED' }] }),
  overview_checks: () => fixture(async ({ page }) => {
    await page.getByRole('navigation', { name: '应用接入步骤' }).waitFor()
    await page.getByText('填写 HTTP/HTTPS 入口即可接入，也可上传构建包或发布固定版本。', { exact: true }).waitFor()
    await page.getByText('选择租户、组织和角色后，检查实际访问范围。', { exact: true }).waitFor()
    assert.equal(await page.getByText('当前范围已具备访问条件', { exact: true }).count(), 0)
    await shot(page, 'checks')
  }, { startPath: `/system/applications?setup=${ids.app}&step=overview` }),
  publish_busy_and_environment: () => fixture(async ({ page, state }) => {
    await page.getByRole('navigation', { name: '应用接入步骤' }).waitFor()
    await page.getByRole('heading', { name: '配置应用入口', exact: true }).waitFor()
    await page.getByLabel('版本号', { exact: false }).fill('1.0.0')
    await page.getByLabel('构建包', { exact: false }).setInputFiles({ name: 'crm-1.0.0.zip', mimeType: 'application/zip', buffer: Buffer.from('isolated-browser-fixture') })
    await page.getByRole('button', { name: '上传并登记', exact: true }).click()
    await page.getByRole('button', { name: '激活', exact: true }).waitFor()
    state.activateGate = deferred()
    await page.getByRole('button', { name: '激活', exact: true }).click()
    await page.getByRole('button', { name: '确认激活', exact: true }).click()
    await page.getByText('正在处理，请等待当前操作完成。', { exact: true }).waitFor()
    assert.equal(await page.getByLabel('接入环境', { exact: true }).isEnabled(), false)
    assert.equal(await page.getByRole('button', { name: '稍后继续', exact: true, includeHidden: true }).isEnabled(), false)
    for (const button of await page.getByRole('navigation', { name: '应用接入步骤', includeHidden: true }).getByRole('button', { includeHidden: true }).all()) assert.equal(await button.isEnabled(), false)
    state.activateGate.resolve()
    await page.getByRole('button', { name: '已激活', exact: true }).waitFor()
    assert.equal(state.deployments.length, 1)
    await page.getByRole('button', { name: '继续配置访问范围', exact: false }).click()
    await page.getByRole('heading', { name: '选择谁可以访问', exact: true }).waitFor()
    assert.equal(await page.getByRole('dialog').count(), 1)
  }, { startPath: `/system/applications?setup=${ids.app}&step=release` }),
  access_scope_and_busy: () => fixture(async ({ page, state }) => {
    await page.getByRole('heading', { name: '选择谁可以访问', exact: true }).waitFor()
    assert.equal(await page.getByRole('button', { name: '确认授予访问', exact: true }).isEnabled(), false)
    await accessTarget(page)
    await page.getByLabel('访问角色', { exact: true }).click()
    await page.getByRole('option', { name: '客户经理', exact: true }).click()
    await page.getByText('租户授权：待配置', { exact: true }).waitFor()
    assert.equal(await page.getByText('当前范围已具备访问条件', { exact: true }).count(), 0)
    await shot(page, 'access')
    state.grantGate = deferred()
    await page.getByRole('button', { name: '确认授予访问', exact: true }).click()
    await page.getByText('正在处理，请等待当前操作完成。', { exact: true }).waitFor()
    assert.equal(await page.getByLabel('访问租户', { exact: true }).isEnabled(), false)
    assert.equal(await page.getByRole('button', { name: '稍后继续', exact: true }).isEnabled(), false)
    assert.equal(state.grants.length, 1)
    assert.deepEqual(state.grants[0], { tenantId: ids.tenant, organizationId: ids.org, roleId: ids.role, menuIds: [ids.menu] })
    state.grantGate.resolve()
    await page.getByText('授权已保存，继续检查访问条件', { exact: true }).waitFor()
    await page.getByText('当前环境尚无可用入口，请保存直接地址或发布版本。', { exact: true }).waitFor()
    assert.equal(await page.getByText('当前范围已具备访问条件', { exact: true }).count(), 0)
    await page.getByRole('navigation', { name: '应用接入步骤' }).getByRole('button', { name: /接入检查/ }).click()
    await page.getByText('所选范围的功能授权已配置，1 名有效成员关联此角色。', { exact: true }).waitFor()
    assert.equal(await page.getByText('选择租户、组织和角色后，检查实际访问范围。', { exact: true }).count(), 0)
    await page.getByRole('navigation', { name: '应用接入步骤' }).getByRole('button', { name: /访问范围/ }).click()
    await page.getByRole('heading', { name: '选择谁可以访问', exact: true }).waitFor()
    await page.getByLabel('访问租户', { exact: true }).filter({ hasText: '示例企业' }).waitFor()
    await page.getByLabel('访问组织', { exact: true }).filter({ hasText: '销售团队' }).waitFor()
    await page.getByLabel('访问角色', { exact: true }).filter({ hasText: '客户经理' }).waitFor()
    await page.getByText('当前环境尚无可用入口，请保存直接地址或发布版本。', { exact: true }).waitFor()
    assert.equal(state.grants.length, 1, 'Step navigation must retain the target without resubmitting the grant.')
  }, { startPath: `/system/applications?setup=${ids.app}&step=access` }),
  candidate_failure_blocks_grant: () => fixture(async ({ page, state }) => {
    await page.getByRole('heading', { name: '选择谁可以访问', exact: true }).waitFor()
    await accessTarget(page)
    await page.getByText('角色候选暂不可用', { exact: true }).waitFor()
    assert.equal(await page.getByLabel('访问角色', { exact: true }).isEnabled(), false)
    assert.equal(await page.getByRole('button', { name: '确认授予访问', exact: true }).isEnabled(), false)
    assert.equal(state.grants.length, 0)
  }, { startPath: `/system/applications?setup=${ids.app}&step=access`, failRoles: true }),
  mobile_layout: () => fixture(async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 })
    await page.getByRole('navigation', { name: '应用接入步骤' }).waitFor()
    const layout = await settledSheetLayout(page), { bounds } = layout
    assert(bounds && bounds.x >= 0 && bounds.width <= 390, 'The onboarding sheet must fit the mobile viewport: ' + JSON.stringify(layout))
    await page.getByRole('button', { name: '继续配置入口', exact: true }).click()
    await page.getByRole('heading', { name: '配置应用入口', exact: true }).waitFor()
    const releaseLayout = await settledSheetLayout(page)
    const overflow = releaseLayout.scrollWidth > releaseLayout.viewport.width
    assert.equal(overflow, false, 'Mobile onboarding must not overflow horizontally: ' + JSON.stringify(releaseLayout))
    await shot(page, 'mobile')
  }, { startPath: `/system/applications?setup=${ids.app}&step=overview` }),

}

let passed = 0
try {
  const selector = process.argv[2] || 'all'
  assert(selector === 'all' || Object.hasOwn(scenarios, selector), `Unknown scenario ${selector}`)
  await startServer()
  browser = await launchBrowser()
  for (const [name, run] of Object.entries(scenarios)) {
    if (selector !== 'all' && selector !== name) continue
    await run()
    console.log('PASS micro setup browser: ' + name)
    passed++
  }
  console.log(`Micro setup browser regression: ${passed}/${selector === 'all' ? Object.keys(scenarios).length : 1} passed (isolated API fixtures; real Vite/React/browser).`)
} finally {
  await browser?.close()
  if (vite && vite.exitCode === null) {
    const exited = new Promise(resolve => vite.once('exit', resolve))
    vite.kill()
    await Promise.race([exited, pause(5000)])
    if (vite.exitCode === null) vite.kill('SIGKILL')
  }
}
