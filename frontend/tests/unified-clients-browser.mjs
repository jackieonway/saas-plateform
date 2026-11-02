/** Real UnifiedClientsPage, production session gate and QueryClient with an isolated HTTP authority. */
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import fs from 'node:fs/promises'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { launchBrowser } from './browser/runtime.mjs'
import { waitState } from './browser/wait.mjs'
import { fixtureUuid, identityForProfile, installSession, profileWithIdentity } from './browser/identity-fixture.mjs'

const workspace = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const reportPath = process.env.UNIFIED_CLIENTS_BROWSER_REPORT_DIR
assert(reportPath && path.isAbsolute(reportPath), 'Set explicit absolute UNIFIED_CLIENTS_BROWSER_REPORT_DIR')
const reportDirectory = path.resolve(reportPath)
for (const directory of ['src', 'tests', 'node_modules']) {
  const relative = path.relative(path.join(workspace, directory), reportDirectory)
  assert(relative.startsWith('..') || path.isAbsolute(relative), 'Browser artifacts must not overwrite source or dependencies')
}
await fs.mkdir(reportDirectory, { recursive: true })

const bindHost = '127.0.0.1'
const pause = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds))
const id = label => fixtureUuid(`unified-clients:${label}`)
const token = 'unified-clients-browser-session'
const ids = {
  actor: id('actor'), tenant: id('tenant'), organization: id('organization'), membership: id('membership'),
  system: id('system'), firstPartyApp: id('first-party-app'), partnerApp: id('partner-app'),
  portalApp: id('portal-app'), partner: id('partner'), operation: id('operation'), receipt: id('receipt'), created: id('created'),
}
const system = { id: ids.system, code: 'system', name: '系统管理', icon: 'Settings2', basePath: '/system', status: 'ENABLED',
  menuCount: 1, entryType: 'HTML', runtimeMode: 'TRUSTED', sortOrder: 0, allowedMemberTypes: ['EMPLOYEE'], accessPolicyVersion: 1 }
const firstPartyApp = { id: ids.firstPartyApp, code: 'platform-console', name: '平台控制台', icon: 'LayoutGrid', basePath: '/console', status: 'ENABLED',
  description: '平台自有应用', menuCount: 0, entryType: 'HTML', runtimeMode: 'TRUSTED', sortOrder: 1, allowedMemberTypes: ['EMPLOYEE'], accessPolicyVersion: 1 }
const partnerApp = { id: ids.partnerApp, code: 'partner-crm', name: '合作方 CRM', icon: 'Building2', basePath: '/partner-crm', status: 'ENABLED',
  description: '已批准合作方应用', menuCount: 0, entryType: 'HTML', runtimeMode: 'TRUSTED', sortOrder: 2, allowedMemberTypes: ['EMPLOYEE'], accessPolicyVersion: 1 }
const portalApp = { id: ids.portalApp, code: 'customer-portal', name: '客户门户', icon: 'Building2', basePath: '/portal', status: 'ENABLED',
  description: '平台自有客户门户', menuCount: 0, entryType: 'HTML', runtimeMode: 'TRUSTED', sortOrder: 3, allowedMemberTypes: ['CUSTOMER'], accessPolicyVersion: 1 }
const apps = [system, firstPartyApp, partnerApp, portalApp]
const profile = profileWithIdentity({
  user: { id: ids.actor, username: 'registry-reviewer', realName: '登记验收管理员', email: '', phone: '', avatar: '', gender: 'UNKNOWN', isSuperAdmin: false },
  tenant: { id: ids.tenant, name: '登记验收租户', code: 'registry-platform', isPlatform: true }, platformContext: true,
  organization: { id: ids.organization, name: '登记验收组织', path: `/${ids.organization}` }, applications: apps,
  memberships: [{ userId: ids.actor, tenantId: ids.tenant, tenantName: '登记验收租户', organizationId: ids.organization,
    organizationName: '登记验收组织', organizationPath: `/${ids.organization}`, membershipId: ids.membership,
    memberType: 'EMPLOYEE', identityVersion: '7', status: 'ENABLED', consoleSelectable: true, selectionDisabledReason: null }],
}, { membershipId: ids.membership, identityVersion: '7', accountVersion: '0' })
const proof = identityForProfile(profile)

const makeClient = (index, ownership = 'THIRD_PARTY', applicationId = ids.partnerApp) => {
  const machine = index === 'machine'
  const suffix = String(index).padStart(2, '0')
  return {
    id: typeof index === 'number' ? id(`client-${suffix}`) : ids.created,
    clientId: typeof index === 'number' ? `registry-client-${suffix}` : 'registry-machine-client',
    displayName: typeof index === 'number' ? `Partner Client ${suffix}` : '新机器同步客户端',
    applicationId,
    applicationName: applicationId === ids.firstPartyApp ? firstPartyApp.name : applicationId === ids.portalApp ? portalApp.name : partnerApp.name,
    ownership,
    partnerId: ownership === 'THIRD_PARTY' ? ids.partner : null,
    environment: typeof index === 'number' ? 'production' : 'test',
    clientType: machine || index === 'confidential' || index === 1 ? 'CONFIDENTIAL' : 'PUBLIC',
    runtimeKind: machine ? 'SERVICE' : index === 'confidential' || index === 1 ? 'WEB_BFF' : 'SPA',
    subjectMode: machine ? 'MACHINE' : 'USER',
    contextMode: machine ? 'NONE' : 'ACCOUNT_ONLY',
    allowedMemberTypes: [],
    grantTypes: machine ? ['client_credentials'] : ['authorization_code'],
    tokenEndpointAuthMethod: machine || index === 'confidential' || index === 1 ? 'client_secret_basic' : 'none',
    redirectUris: machine ? [] : ['https://partner.example.test/callback'],
    postLogoutRedirectUris: [],
    logoutMode: 'SHORT_SESSION_ONLINE_CHECK', backchannelLogoutUri: null, backchannelTargetId: null,
    status: 'ENABLED',
    version: 1,
    policyVersion: 1,
    createdAt: new Date(Date.UTC(2026, 9, 1, 0, 0, typeof index === 'number' ? index : 30)).toISOString(),
    updatedAt: new Date(Date.UTC(2026, 9, 2, 0, 0, typeof index === 'number' ? index : 30)).toISOString(),
    consentGroupId: null,
    permissions: [{ resourceKey: machine ? 'sync-api' : 'userinfo', audience: machine ? 'urn:sync-api' : 'urn:identity:userinfo', scopes: machine ? ['records.read'] : ['openid', 'profile'] }],
  }
}

let browser, vite, origin, startup = ''
const scenarios = [], screenshots = []

async function startServer() {
  const port = await new Promise((resolve, reject) => {
    const server = net.createServer().on('error', reject)
    server.listen(0, bindHost, () => { const available = server.address().port; server.close(error => error ? reject(error) : resolve(available)) })
  })
  origin = `http://${bindHost}:${port}`
  vite = spawn(process.execPath, [path.join(workspace, 'node_modules/vite/bin/vite.js'), '--host', bindHost, '--port', String(port), '--strictPort'], {
    cwd: workspace, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
  })
  let failed
  vite.on('error', error => { failed = error })
  for (const stream of [vite.stdout, vite.stderr]) stream.on('data', chunk => { startup = (startup + chunk).slice(-5000) })
  await waitState(() => {
    if (failed) throw failed
    if (vite.exitCode !== null) throw new Error(`Owned Vite stopped: ${startup}`)
    return startup.includes('ready in')
  }, 'Unified clients Vite must become ready', 30_000)
}

async function fixture({ mobile = false, url = '/system/oauth/clients' } = {}) {
  const context = await browser.newContext({ viewport: mobile ? { width: 390, height: 844 } : { width: 1440, height: 1000 }, reducedMotion: 'reduce' })
  const page = await context.newPage()
  page.setDefaultTimeout(12_000)
  const state = {
    clients: [...Array.from({ length: 25 }, (_, index) => makeClient(index + 1)),
      makeClient('confidential', 'FIRST_PARTY', ids.firstPartyApp),
      { ...makeClient('portal', 'FIRST_PARTY', ids.portalApp), id: id('portal-client'), clientId: 'portal-login', displayName: '客户门户登录客户端' }],
    consentGroups: [],
    requests: [], writes: [], operationReads: 0, rotationWrites: 0, unknownOperationReads: 0,
    errors: [], unexpected: [], nextRotationUnknown: false, nextRotationNetworkFailure: false, rotationOperation: null, operationFirstReadUnknown: false, holdOperationReads: false, denyOperationReads: false, conflictNextUpdate: false, failListRequests: 0,
  }
  page.on('pageerror', error => state.errors.push(error.message))
  const session = await installSession(context, token, proof)
  await context.addInitScript(() => localStorage.setItem('platform-theme', 'light'))

  await context.route(url => url.origin === origin && url.pathname === '/src/main.tsx', async route => {
    const response = await route.fetch()
    let body = await response.text()
    const appImport = /import App from ["']\/src\/App\.tsx(?:\?[^"']*)?["']/
    assert.match(body, appImport, 'Keep the production React root, query provider and session gate')
    body = body.replace(appImport, `import UnifiedClientsUnderTest from "/src/pages/system/oauth/UnifiedClientsPage.tsx";
      import { useProfileQuery as registryProfileQuery, useMenusQuery as registryMenusQuery, usePermissionsQuery as registryPermissionsQuery } from "/src/hooks/use-auth-data.ts";
      function App() {
        const stored = sessionStore(state => state.profile);
        const epoch = sessionStore(state => state.contextEpoch);
        const query = registryProfileQuery();
        registryMenusQuery(stored ? ${JSON.stringify(ids.system)} : null);
        registryPermissionsQuery();
        React.useEffect(() => { if (query.data) sessionStore.getState().setProfile(query.data, epoch); }, [query.data, epoch]);
        return stored ? React.createElement(UnifiedClientsUnderTest) : React.createElement("main", { role: "status" }, "正在重新核验当前成员授权");
      }`)
    body = body.replace('ReactDOM.createRoot', `
      await sessionStore.getState().initializeSessionCoordination();
      sessionStore.getState().setProfile(${JSON.stringify(profile)}, sessionStore.getState().contextEpoch);
      sessionStore.getState().setCurrentApplication(${JSON.stringify(ids.system)});
      window.__unifiedClientsFixture = { queryData: () => queryClient.getQueryCache().getAll().map(query => query.state.data) };
      ReactDOM.createRoot`)
    await route.fulfill({ response, body })
  })

  const fulfill = (route, value, status = 200) => route.fulfill({ status, contentType: 'application/json', headers: { 'x-request-id': ids.receipt, 'cache-control': 'no-store' }, body: JSON.stringify(value) })
  const reject = (route, status, code, message, operationId) => fulfill(route, { code, message, ...(operationId ? { operationId } : {}) }, status)
  await context.route(url => url.origin === origin && url.pathname.startsWith('/api/'), async route => {
    const request = route.request(), parsed = new URL(request.url()), method = request.method()
    const item = { path: parsed.pathname, method, query: Object.fromEntries(parsed.searchParams), body: request.postData() ? request.postDataJSON() : undefined, operationId: request.headers()['x-identity-operation'] }
    state.requests.push(item)
    try {
      assert.equal(request.headers().authorization, `Bearer ${token}`)
      if (parsed.pathname === '/api/auth/profile') return fulfill(route, profile)
      if (parsed.pathname === '/api/auth/permissions') return fulfill(route, { permissions: ['system:oauth'], roles: [] })
      if (parsed.pathname === '/api/auth/menus') return fulfill(route, [
        { id: id('menu-clients'), parentId: null, type: 'MENU', title: 'OAuth 客户端', path: '/system/oauth/clients', permission: 'system:oauth', sortOrder: 1, visible: true, children: [] },
      ])
      if (parsed.pathname === '/api/applications' && method === 'GET') return fulfill(route, apps)
      if (parsed.pathname === '/api/identity/clients' && method === 'GET') {
        if (state.failListRequests > 0) {
          state.failListRequests--
          return reject(route, 503, 'TEMPORARILY_UNAVAILABLE', 'Temporary directory read failure')
        }
        const { applicationId, partnerId, ownership, environment, clientType, subjectMode, status } = item.query
        let records = [...state.clients].filter(client =>
          (!applicationId || client.applicationId === applicationId) && (!partnerId || client.partnerId === partnerId) &&
          (!ownership || client.ownership === ownership) && (!environment || client.environment === environment) &&
          (!clientType || client.clientType === clientType) && (!subjectMode || client.subjectMode === subjectMode) && (!status || client.status === status))
        records.sort((left, right) => left.createdAt.localeCompare(right.createdAt) || left.id.localeCompare(right.id))
        const limit = Number(item.query.limit ?? 10), offset = Number(item.query.offset ?? 0)
        return fulfill(route, { records: records.slice(offset, offset + limit), page: { limit, offset, total: records.length } })
      }
      const consentGroups = /^\/api\/identity\/applications\/([^/]+)\/consent-groups$/.exec(parsed.pathname)
      if (consentGroups && method === 'GET') {
        return fulfill(route, { groups: state.consentGroups.filter(group => group.applicationId === consentGroups[1]) })
      }
      if (parsed.pathname === '/api/identity/consent-groups' && method === 'POST') {
        state.writes.push(item)
        const created = { id: id('created-consent-group'), applicationName: apps.find(application => application.id === item.body.applicationId)?.name ?? 'Unknown application',
          ...item.body, subjectMode: 'USER', status: 'ACTIVE', version: 1 }
        state.consentGroups.push({ ...created, clients: [] })
        return fulfill(route, created, 201)
      }
      const groupMember = /^\/api\/identity\/consent-groups\/([^/]+)\/clients$/.exec(parsed.pathname)
      if (groupMember && method === 'POST') {
        state.writes.push(item)
        const group = state.consentGroups.find(value => value.id === groupMember[1])
        const client = state.clients.find(value => value.id === item.body.clientId)
        if (!group || !client || item.body.expectedGroupVersion !== group.version) return reject(route, 409, 'version_conflict', 'Consent group version changed')
        group.version++
        group.clients.push({ id: client.id, displayName: client.displayName })
        client.consentGroupId = group.id
        return fulfill(route, { groupId: group.id, clientId: client.id, version: group.version })
      }
      if (parsed.pathname === '/api/identity/clients' && method === 'POST') {
        state.writes.push(item)
        const created = { ...makeClient('machine'), ...item.body, id: ids.created, clientId: 'registry-machine-client', applicationName: partnerApp.name,
          ownership: 'THIRD_PARTY', partnerId: ids.partner, version: 1, policyVersion: 1, createdAt: new Date().toISOString(), updatedAt: new Date().toISOString(),
          permissions: item.body.permissions.map(permission => ({ ...permission, audience: `urn:${permission.resourceKey}` })) }
        state.clients.unshift(created)
        return fulfill(route, created, 201)
      }
      const client = /^\/api\/identity\/clients\/([^/]+)$/.exec(parsed.pathname)
      if (client && method === 'GET') {
        const record = state.clients.find(value => value.id === client[1])
        return record ? fulfill(route, record) : reject(route, 404, 'NOT_FOUND', 'Client not found')
      }
      if (client && method === 'PATCH') {
        state.writes.push(item)
        if (state.conflictNextUpdate) {
          state.conflictNextUpdate = false
          return reject(route, 409, 'version_conflict', 'Client version changed')
        }
        const index = state.clients.findIndex(value => value.id === client[1])
        if (index < 0) return reject(route, 404, 'NOT_FOUND', 'Client not found')
        state.clients[index] = { ...state.clients[index], ...item.body, version: state.clients[index].version + 1 }
        return fulfill(route, state.clients[index])
      }
      const projection = /^\/api\/identity\/applications\/([^/]+)\/projection$/.exec(parsed.pathname)
      if (projection && method === 'GET') return fulfill(route, {
        applicationId: projection[1], partnerId: projection[1] === ids.partnerApp ? ids.partner : null,
        ownership: [ids.firstPartyApp, ids.portalApp].includes(projection[1]) ? 'FIRST_PARTY' : 'THIRD_PARTY',
        status: 'APPROVED', displayName: apps.find(application => application.id === projection[1])?.name ?? 'Unknown application',
        purpose: 'Browser fixture', approvedScopes: ['openid', 'profile', 'records.read'], projectionVersion: 3,
      })
      const credential = /^\/api\/identity\/clients\/([^/]+)\/credentials$/.exec(parsed.pathname)
      if (credential && method === 'POST') {
        state.rotationWrites++
        state.writes.push(item)
        assert.match(item.operationId, /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/)
        state.rotationOperation = item.operationId
        if (state.nextRotationNetworkFailure) {
          state.nextRotationNetworkFailure = false
          state.operationFirstReadUnknown = true
          return route.abort('connectionreset')
        }
        if (state.nextRotationUnknown) {
          state.nextRotationUnknown = false
          state.operationFirstReadUnknown = true
          return reject(route, 503, 'OIDC_REMOTE_RESULT_UNKNOWN', '结果未知', item.operationId)
        }
        return fulfill(route, { clientId: credential[1], credentialId: id('credential'), credentialVersion: 2, clientVersion: 2,
          previousCredentialExpiresAt: null, credentialExpiresAt: '2027-01-26T00:00:00Z', clientSecret: 'fixture-secret-one-time' })
      }
      const operation = /^\/api\/identity\/client-operations\/([^/]+)$/.exec(parsed.pathname)
      if (operation && method === 'GET') {
        assert.equal(operation[1], state.rotationOperation, 'Query the operation generated before browser dispatch')
        state.operationReads++
        if (state.denyOperationReads) return reject(route, 403, 'FORBIDDEN', '原操作不属于当前管理员会话，请返回发起操作的原会话查询')
        if (state.holdOperationReads) return reject(route, 503, 'OIDC_REMOTE_RESULT_UNKNOWN', '原操作仍在处理中', operation[1])
        if (state.operationFirstReadUnknown && state.unknownOperationReads++ === 0) return reject(route, 503, 'OIDC_REMOTE_RESULT_UNKNOWN', '原操作仍在处理中', ids.operation)
        return fulfill(route, { operationId: operation[1], state: 'COMPLETED', result: {
          clientId: state.clients[0].id, credentialId: id('credential-recovered'), credentialVersion: 2, clientVersion: 2,
          previousCredentialExpiresAt: null, credentialExpiresAt: '2027-01-26T00:00:00Z', clientSecret: 'fixture-secret-one-time',
        } })
      }
      state.unexpected.push(item)
      return reject(route, 503, 'FIXTURE_UNEXPECTED', `Unexpected API ${method} ${parsed.pathname}`)
    } catch (error) {
      state.errors.push(`HTTP fixture: ${error.message}`)
      return reject(route, 503, 'FIXTURE_ASSERTION_FAILED', 'Fixture assertion failed')
    }
  })

  try {
    await page.goto(`${origin}${url}`, { waitUntil: 'domcontentloaded' })
    await page.getByRole('heading', { name: '统一客户端注册簿', exact: true }).waitFor()
    await waitState(() => state.requests.some(request => request.path === '/api/identity/clients' && request.method === 'GET'), 'Load unified client directory')
    return { context, page, state, session }
  } catch (error) { await context.close(); throw error }
}

async function runScenario(name, options, run) {
  const fixturePage = await fixture(options)
  try {
    await run(fixturePage)
    assert.deepEqual(fixturePage.state.errors, [])
    assert.deepEqual(fixturePage.state.unexpected, [])
    scenarios.push({ name, status: 'PASS' })
    console.log(`PASS unified clients: ${name}`)
  } catch (error) {
    const filename = `${name.replace(/[^a-z0-9]+/gi, '-').toLowerCase()}-failure.png`
    await fixturePage.page.screenshot({ path: path.join(reportDirectory, filename), fullPage: true, animations: 'disabled' }).catch(() => {})
    console.error('Unified clients browser diagnostics', { name, errors: fixturePage.state.errors, unexpected: fixturePage.state.unexpected,
      requests: fixturePage.state.requests.map(({ path: requestPath, method, query }) => ({ path: requestPath, method, query })),
      body: await fixturePage.page.locator('body').innerText().catch(() => '') })
    throw error
  } finally { await fixturePage.context.close() }
}

async function screenshot(page, filename) {
  await page.screenshot({ path: path.join(reportDirectory, filename), fullPage: true, animations: 'disabled' })
  screenshots.push(filename)
}
async function screenshotDialog(dialog, filename) {
  await dialog.screenshot({ path: path.join(reportDirectory, filename), animations: 'disabled' })
  screenshots.push(filename)
}

try {
  browser = await launchBrowser()
  await startServer()

  await runScenario('desktop URL filters and server pagination restore', { url: `/system/oauth/clients?page=2&size=10&ownership=THIRD_PARTY&environment=production` }, async f => {
    assert.equal(await f.page.getByRole('tab').count(), 0, 'One directory replaces the old OAuth/OIDC tabs')
    await f.page.getByRole('button', { name: 'Partner Client 11', exact: true }).first().waitFor()
    const initial = f.state.requests.find(request => request.path === '/api/identity/clients' && request.method === 'GET')
    assert.equal(initial.query.offset, '10')
    assert.equal(initial.query.ownership, 'THIRD_PARTY')
    assert.equal(initial.query.environment, 'production')
    await f.page.getByRole('link', { name: '3', exact: true }).click()
    await f.page.getByRole('button', { name: 'Partner Client 21', exact: true }).first().waitFor()
    assert.match(f.page.url(), /(?:\?|&)page=3(?:&|$)/)
    await f.page.getByRole('button', { name: '查看 Partner Client 21 配置' }).click()
    await f.page.getByRole('dialog').getByText('应用 UUID / Partner UUID', { exact: true }).waitFor()
    await f.page.getByRole('dialog').getByRole('button', { name: 'Close' }).click().catch(async () => f.page.keyboard.press('Escape'))
    await screenshot(f.page, 'desktop-third-party-directory.png')
    await f.page.getByRole('combobox', { name: '环境' }).click()
    await f.page.getByRole('option', { name: '全部环境', exact: true }).click()
    await f.page.getByRole('combobox', { name: '归属' }).click()
    await f.page.getByRole('option', { name: '自有应用', exact: true }).click()
    await f.page.getByText('平台控制台', { exact: true }).first().waitFor()
    await f.page.getByText('客户门户', { exact: true }).first().waitFor()
    await f.page.getByText('客户门户登录客户端', { exact: true }).first().waitFor()
    assert.equal(await f.page.getByText('Partner Client 01', { exact: true }).count(), 0)
    await screenshot(f.page, 'desktop-first-party-sources.png')
  })

  await runScenario('390px editor contains keyboard focus and restores its opener on Escape', { mobile: true }, async f => {
    const opener = f.page.getByRole('button', { name: '登记客户端', exact: true })
    await opener.focus()
    await f.page.keyboard.press('Enter')
    const dialog = f.page.getByRole('dialog')
    await dialog.getByRole('heading', { name: '登记新客户端' }).waitFor()
    const count = await dialog.locator('button:not([disabled]),input:not([disabled]),select:not([disabled]),textarea:not([disabled]),[tabindex="0"]').count()
    assert(count > 2 && count < 100)
    for (const direction of ['Tab', 'Shift+Tab']) {
      for (let index = 0; index < count + 2; index++) {
        await f.page.keyboard.press(direction)
        assert(await dialog.evaluate(element => element.contains(document.activeElement)), 'Keyboard focus stays inside the modal')
      }
    }
    await f.page.keyboard.press('Escape')
    await dialog.waitFor({ state: 'hidden' })
    await waitState(() => opener.evaluate(element => element === document.activeElement), 'Escape returns focus to the client editor opener', 1500)
    assert.equal(f.state.requests.some(request => request.method === 'POST'), false, 'Closing the empty editor does not submit a write')
  })

  await runScenario('390px editor fits viewport and creates a valid machine client', { mobile: true }, async f => {
    await f.page.getByRole('button', { name: '登记客户端', exact: true }).click()
    const dialog = f.page.getByRole('dialog')
    await dialog.getByRole('heading', { name: '登记新客户端' }).waitFor()
    const bounds = await dialog.boundingBox()
    assert(bounds && bounds.x >= 0 && bounds.x + bounds.width <= 391)
    assert(await f.page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), '390px page has no horizontal viewport overflow')
    const formScroll = dialog.locator('form .overflow-y-auto').first()
    const scrollBounds = await formScroll.evaluate(element => ({ clientHeight: element.clientHeight, scrollHeight: element.scrollHeight }))
    assert(scrollBounds.scrollHeight > scrollBounds.clientHeight, 'Long forms scroll inside the dialog body')
    await formScroll.evaluate(element => { element.scrollTop = element.scrollHeight })
    assert(await dialog.getByRole('button', { name: '创建客户端', exact: true }).isVisible(), 'Fixed action bar stays available while the form scrolls')
    await formScroll.evaluate(element => { element.scrollTop = 0 })
    await screenshotDialog(dialog, 'mobile-client-editor.png')
    await dialog.getByPlaceholder('019a…').fill(ids.partnerApp)
    await dialog.getByPlaceholder('示例 CRM Web').fill('新机器同步客户端')
    await dialog.getByRole('combobox', { name: '主体模式' }).click()
    await f.page.getByRole('option', { name: '机器 MACHINE' }).click()
    await dialog.getByPlaceholder('userinfo').fill('sync-api')
    await dialog.getByPlaceholder('openid, profile').fill('records.read')
    await waitState(() => f.state.requests.some(request => request.path === `/api/identity/applications/${ids.partnerApp}/projection`), 'Read approved application projection')
    await dialog.getByRole('button', { name: '创建客户端', exact: true }).click()
    await waitState(() => f.state.writes.some(request => request.path === '/api/identity/clients' && request.method === 'POST'), 'Submit machine client')
    const payload = f.state.writes.find(request => request.path === '/api/identity/clients' && request.method === 'POST').body
    assert.equal(payload.subjectMode, 'MACHINE')
    assert.equal(payload.runtimeKind, 'SERVICE')
    assert.equal(payload.contextMode, 'NONE')
    assert.equal(payload.clientType, 'CONFIDENTIAL')
    assert.deepEqual(payload.grantTypes, ['client_credentials'])
    assert.deepEqual(payload.allowedMemberTypes, [])
    assert.deepEqual(payload.redirectUris, [])
    await f.page.getByRole('heading', { name: '统一客户端注册簿', exact: true }).waitFor()
    await screenshot(f.page, 'mobile-client-directory.png')
  })

  await runScenario('390px consent group management versions members and warns about reconfirmation', { mobile: true }, async f => {
    await f.page.getByRole('button', { name: '同意组管理', exact: true }).click()
    const dialog = f.page.getByRole('dialog')
    await dialog.getByRole('heading', { name: '用户同意组', exact: true }).waitFor()
    const bounds = await dialog.boundingBox()
    assert(bounds && bounds.x >= 0 && bounds.x + bounds.width <= 391)
    assert(await f.page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), '390px consent group view has no horizontal overflow')
    await dialog.getByPlaceholder('019a…').fill(ids.partnerApp)
    await dialog.getByPlaceholder('平台浏览器客户端').fill('浏览器同意组')
    await dialog.getByRole('combobox', { name: '同意组环境' }).click()
    await f.page.getByRole('option', { name: '生产 / production', exact: true }).click()
    await dialog.getByRole('button', { name: '建立同意组', exact: true }).click()
    await waitState(() => f.state.writes.some(request => request.path === '/api/identity/consent-groups' && request.method === 'POST'), 'Create explicit same-context consent group')
    const create = f.state.writes.find(request => request.path === '/api/identity/consent-groups' && request.method === 'POST')
    assert.equal(create.body.applicationId, ids.partnerApp)
    assert.equal(create.body.environment, 'production')
    assert.equal(create.body.contextMode, 'ACCOUNT_ONLY')
    assert.equal(create.body.purpose, 'Browser fixture')
    await dialog.getByText('浏览器同意组', { exact: true }).waitFor()
    const memberSelect = dialog.getByRole('combobox', { name: '选择加入 浏览器同意组 的客户端' })
    await memberSelect.click()
    await f.page.getByRole('option', { name: /Partner Client 01 · registry-client-01/ }).click()
    await dialog.getByRole('button', { name: '加入同意组', exact: true }).click()
    await waitState(() => f.state.writes.some(request => request.path === `/api/identity/consent-groups/${id('created-consent-group')}/clients`), 'Add a matching ungrouped USER client')
    const add = f.state.writes.find(request => request.path.endsWith('/clients') && request.method === 'POST')
    assert.equal(add.body.clientId, id('client-01'))
    assert.equal(add.body.expectedGroupVersion, 1)
    await dialog.getByText('Partner Client 01', { exact: true }).waitFor()
    await dialog.getByText('v2', { exact: true }).waitFor()
    await dialog.getByText(/加入后该组旧授权会标记为待重新确认/).waitFor()
    await screenshotDialog(dialog, 'desktop-consent-groups.png')
  })

  await runScenario('390px back-channel registration preserves its owner binding on retirement', { mobile: true }, async f => {
    const target = f.state.clients[0], reference = id('verified-delivery-target')
    await f.page.getByRole('button', { name: `编辑 ${target.displayName}` }).first().click()
    const dialog = f.page.getByRole('dialog')
    await dialog.getByRole('combobox', { name: '客户端登出合同' }).click()
    await f.page.getByRole('option', { name: '已审核 back-channel', exact: true }).click()
    await dialog.getByLabel('客户端back-channel地址').fill('https://partner.example.test/auth/backchannel-logout')
    await dialog.getByLabel('客户端back-channel目标引用').fill(reference)
    assert(await f.page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1))
    await dialog.getByLabel('客户端back-channel目标引用').scrollIntoViewIfNeeded()
    await screenshotDialog(dialog, 'phase20-backchannel-mobile.png')
    await dialog.getByRole('button', { name: '保存更改', exact: true }).click()
    await waitState(() => f.state.writes.some(row => row.method === 'PATCH'), 'Submit an existing client owner reference')
    const write = f.state.writes.find(row => row.method === 'PATCH')
    assert.equal(write.body.logoutMode, 'BACK_CHANNEL')
    assert.equal(write.body.backchannelTargetId, reference)
    await dialog.waitFor({ state: 'hidden' })
    await f.page.getByRole('button', { name: `退役 ${target.displayName}` }).first().click()
    await f.page.getByRole('button', { name: '退役客户端', exact: true }).click()
    await waitState(() => f.state.writes.filter(row => row.method === 'PATCH').length === 2, 'Retire the same bound client')
    const retired = f.state.writes.filter(row => row.method === 'PATCH')[1].body
    assert.equal(retired.status, 'RETIRED')
    assert.equal(retired.backchannelTargetId, reference)
    assert.equal(retired.backchannelLogoutUri, write.body.backchannelLogoutUri)
    assert.equal(retired.logoutMode, 'BACK_CHANNEL')
  })

  await runScenario('failed refresh retains the last successful directory', {}, async f => {
    f.state.failListRequests = 2
    await f.page.getByRole('button', { name: '刷新', exact: true }).click()
    await f.page.getByText('刷新失败，仍显示上次成功读取的配置', { exact: true }).waitFor()
    await f.page.getByRole('button', { name: 'Partner Client 01', exact: true }).first().waitFor()
    assert.equal(f.state.failListRequests, 0, 'The test exhausted one retry and received a final read failure')
  })

  await runScenario('same-frame duplicate submits issue only one versioned write', {}, async f => {
    const target = f.state.clients[0]
    await f.page.getByRole('button', { name: `编辑 ${target.displayName}` }).first().click()
    const dialog = f.page.getByRole('dialog')
    await dialog.getByPlaceholder('示例 CRM Web').fill('同帧重复提交检查')
    const form = dialog.locator('form')
    await form.evaluate(element => { element.requestSubmit(); element.requestSubmit() })
    await waitState(() => f.state.writes.filter(request => request.method === 'PATCH').length === 1, 'Accept the first write only')
    await dialog.getByRole('heading', { name: '编辑客户端登记' }).waitFor({ state: 'detached' })
    assert.equal(f.state.writes.filter(request => request.method === 'PATCH').length, 1)
  })

  await runScenario('409 version conflict requires an explicit latest-version reload', {}, async f => {
    const target = f.state.clients[0]
    f.state.conflictNextUpdate = true
    await f.page.getByRole('button', { name: `编辑 ${target.displayName}` }).first().click()
    const dialog = f.page.getByRole('dialog')
    await dialog.getByPlaceholder('示例 CRM Web').fill('未保存的并发草稿')
    await dialog.getByRole('button', { name: '保存更改', exact: true }).click()
    await dialog.getByRole('alert').getByText(/此记录已被其他管理员更新/).waitFor()
    assert.equal(await dialog.getByPlaceholder('示例 CRM Web').inputValue(), '未保存的并发草稿')
    assert.equal(await dialog.getByRole('button', { name: '保存更改', exact: true }).isDisabled(), true)
    await dialog.getByRole('button', { name: '载入最新版本', exact: true }).click()
    await waitState(async () => await dialog.getByPlaceholder('示例 CRM Web').inputValue() === target.displayName, 'Replace the draft with the current server version')
    assert.equal(await dialog.getByPlaceholder('示例 CRM Web').inputValue(), target.displayName)
    assert.equal(await dialog.getByRole('button', { name: '保存更改', exact: true }).isDisabled(), false)
    await screenshot(f.page, 'desktop-client-version-conflict.png')
  })

  for (const networkFailure of [false, true]) await runScenario(`${networkFailure ? 'lost HTTP response' : 'unknown credential result'} recovers the original operation without resubmitting`, {}, async f => {
    f.state.nextRotationUnknown = !networkFailure
    f.state.nextRotationNetworkFailure = networkFailure
    f.state.operationFirstReadUnknown = true
    const target = f.state.clients[0]
    await f.page.getByRole('button', { name: `轮换 ${target.displayName} 密钥` }).first().click()
    await f.page.getByRole('button', { name: '生成新密钥', exact: true }).click()
    await f.page.getByText('查询失败：原操作仍在处理中', { exact: false }).waitFor()
    assert.equal(f.state.rotationWrites, 1)
    assert.equal(f.state.operationReads, 1)
    await f.page.getByRole('button', { name: /查询原操作/ }).click()
    await f.page.getByText('fixture-secret-one-time', { exact: true }).waitFor()
    assert.equal(f.state.rotationWrites, 1, 'Recovery must never issue a second credential write')
    assert.equal(f.state.operationReads, 2)
    const persisted = await f.page.evaluate(() => `${location.href}\n${Object.values(localStorage).join('\n')}\n${Object.values(sessionStorage).join('\n')}\n${JSON.stringify(window.__unifiedClientsFixture.queryData())}`)
    assert.equal(persisted.includes('fixture-secret-one-time'), false, 'Secret stays out of URL, browser storage and Query cache')
    await f.page.getByRole('button', { name: '我已保存', exact: true }).click()
    await f.page.getByText('fixture-secret-one-time', { exact: true }).waitFor({ state: 'detached' })
  })

  for (const differentSession of [false, true]) await runScenario(`${differentSession ? 'different authorized session refuses old credential recovery after reload' : 'lost response then reload restores only the original pending operation'}`, {}, async f => {
    f.state.nextRotationNetworkFailure = true
    f.state.holdOperationReads = true
    const target = f.state.clients[0]
    await f.page.getByRole('button', { name: `轮换 ${target.displayName} 密钥` }).first().click()
    await f.page.getByRole('button', { name: '生成新密钥', exact: true }).click()
    await f.page.getByText('查询失败：原操作仍在处理中', { exact: false }).waitFor()
    const saved = await f.page.evaluate(() => Object.entries(sessionStorage).filter(([key]) => key.startsWith('platform-identity-operation-v1:')).map(([key, value]) => ({ key, value: JSON.parse(value) })))
    assert.equal(saved.length, 1)
    assert.deepEqual(Object.keys(saved[0].value).sort(), ['accountId', 'action', 'clientLabel', 'contextKey', 'operationId', 'schema', 'sessionId'])
    assert.equal(saved[0].value.operationId, f.state.rotationOperation)
    assert.equal(saved[0].value.action, 'rotate')
    assert.equal(saved[0].value.accountId, profile.user.id)
    assert.equal(JSON.stringify(saved).includes(token), false)
    if (differentSession) {
      f.session.sessionId = id('replacement-authorized-browser-session')
      f.state.denyOperationReads = true
    }
    const readsBeforeReload = f.state.operationReads
    await f.page.reload({ waitUntil: 'domcontentloaded' })
    await waitState(() => f.state.operationReads > readsBeforeReload, 'Mount queries the persisted original operation before allowing writes')
    assert.equal(await f.page.getByRole('button', { name: '登记客户端', exact: true }).isDisabled(), true)
    assert.equal(f.state.rotationWrites, 1)
    assert.equal(f.state.requests.filter(request => request.path.startsWith('/api/identity/client-operations/')).every(request => request.path.endsWith(f.state.rotationOperation)), true)
    if (differentSession) {
      await f.page.getByText(/原会话无法恢复时，请将操作编号交给授权管理员核查执行结果/).waitFor()
      assert.equal(await f.page.getByText('fixture-secret-one-time', { exact: true }).count(), 0)
      const retained = await f.page.evaluate(key => JSON.parse(sessionStorage.getItem(key)), saved[0].key)
      assert.deepEqual(retained, saved[0].value, 'A denied different session cannot clear or rewrite unknown recovery')
      return
    }
    await f.page.getByText('查询失败：原操作仍在处理中', { exact: false }).waitFor()
    f.state.holdOperationReads = false
    f.state.operationFirstReadUnknown = false
    await f.page.getByRole('button', { name: /查询原操作/ }).click()
    await f.page.getByText('fixture-secret-one-time', { exact: true }).waitFor()
    assert.equal(f.state.rotationWrites, 1)
    const remaining = await f.page.evaluate(() => Object.keys(sessionStorage).filter(key => key.startsWith('platform-identity-operation-v1:')))
    assert.deepEqual(remaining, [], 'Only a confirmed terminal result clears pending metadata')
    const persisted = await f.page.evaluate(() => `${Object.values(sessionStorage).join('\n')}\n${Object.values(localStorage).join('\n')}\n${JSON.stringify(window.__unifiedClientsFixture.queryData())}`)
    assert.equal(persisted.includes('fixture-secret-one-time'), false)
  })

  await fs.writeFile(path.join(reportDirectory, 'unified-clients-browser.json'), JSON.stringify({
    status: 'PASS', scope: 'Real React/Vite unified client page with production session gate and isolated HTTP fixture; no database or live backend claim',
    browser: await browser.version(), scenarios, screenshots,
  }, null, 2) + '\n')
  console.log(`Unified clients browser passed (${scenarios.length} scenarios). Report: ${reportDirectory}`)
} finally {
  await browser?.close()
  if (vite && vite.exitCode === null) {
    const exited = new Promise(resolve => vite.once('exit', resolve)); vite.kill(); await Promise.race([exited, pause(5000)])
    if (vite.exitCode === null) vite.kill('SIGKILL')
  }
}
